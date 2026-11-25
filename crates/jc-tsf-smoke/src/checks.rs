//! 十三項安裝診斷的流程。
//!
//! 順序固定為：DLL 載入 → 類廠 → 介面 vtable／物件身分 → `ITfThreadMgr` →
//! `Activate` → 引擎 pipe → 註冊探測 → 安裝檔案 → machine profile 機碼 →
//! 使用者啟用（TSF API）→ 使用者語言清單 → 引擎細節 → UI 探針。前一步失敗時
//! 後面盡量改成 `[SKIP]`，不要連鎖製造假失敗。

use std::io::Write;
use std::time::{Duration, Instant};

use windows::core::{IUnknown, Interface};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IClassFactory, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Ole::CONNECT_E_ADVISELIMIT;
use windows::Win32::System::Registry::{KEY_WOW64_32KEY, KEY_WOW64_64KEY};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfClientId, ITfCompositionSink, ITfKeyEventSink, ITfKeystrokeMgr,
    ITfTextInputProcessor, ITfThreadMgr, ITfThreadMgrEventSink,
};

use crate::dll::{self, Library};
use crate::engine;
use crate::install;
use crate::product;
use crate::profile::{self, UserProfileVerdict};
use crate::registry;
use crate::report::{self, Report, Status};
use crate::tsf_profile;
use crate::ui_probe;

/// UI 探針失敗時的修復建議。
const UI_PROBE_FIX: &str = "啟動 jc-engine（候選字來自引擎），設定 JC_TSF_LOG=1 後重跑並看 %LOCALAPPDATA%\\jc-ime\\tsf.log；確認 jc-tip-register.exe user-install 已完成";

/// 執行全部檢查，回傳行程結束碼。
pub fn run(args: Vec<String>) -> std::process::ExitCode {
    // 純診斷：記錄起點，供 Deactivate／probe 的卡點時間戳使用。
    let start = Instant::now();

    let options = match parse_args(&args) {
        Cli::Run(options) => options,
        Cli::Help => return std::process::ExitCode::SUCCESS,
        Cli::Error => return std::process::ExitCode::from(2),
    };

    let mut report = Report::new(options.json);
    match options.only {
        Some(CheckFilter::UiProbe) => {
            report.heading(format!("jc-ime UI 探針（{}）", product::DISPLAY_NAME));
            report.heading(
                "需要已安裝並啟用的 jc 注音與互動桌面；只把按鍵送到本行程建立的 EDIT，\
                 不改系統或使用者的預設輸入法。",
            );
        }
        None => {
            report.heading(format!("jc-ime 安裝診斷（{}）", product::DISPLAY_NAME));
            report.heading(
                "不需要管理員；只讀機碼與 TSF API。唯一的寫入是在本行程 ActivateProfile，不改系統或使用者的預設輸入法。",
            );
        }
    }
    report.heading("");

    // TSF 的 COM 物件是 apartment-threaded；先初始化再建立任何物件。
    // SAFETY: 這條執行緒還沒有初始化 COM。
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    report.info(format!(
        "CoInitializeEx(APARTMENTTHREADED) = {}",
        report::hex(com.0 as u32)
    ));
    let com_ready = com.is_ok();

    // --check ui-probe／--ui-probe：只跑檢查 13，不碰 DLL 載入與其他 12 項。
    if options.only == Some(CheckFilter::UiProbe) {
        if com_ready {
            check_ui_probe(&mut report);
        } else {
            report.skip("ui.probe", "UI 探針", "COM 沒有初始化，無法建立 TSF 物件");
        }
        if com_ready {
            // SAFETY: 上面的 CoInitializeEx 成功，配對呼叫一次。
            unsafe { CoUninitialize() };
        }
        report.finish();
        return exit_code(&report);
    }

    let clsid_text = product::registry_key(&product::CLSID_JC_TSF);
    let profile_text = product::registry_key(&product::GUID_PROFILE_JC_TSF);

    let path = dll::resolve_path(options.dll.as_deref());
    report.info(format!("DLL：{}", path.display()));
    report.info(format!(
        "CLSID {clsid_text}；profile {profile_text}；LANGID {}",
        report::hex(product::LANGID_ZH_TW as u32)
    ));

    // ── 1. DLL 載入與 DllCanUnloadNow ────────────────────────────────
    let mut library: Option<Library> = None;
    match Library::load(&path) {
        Ok(loaded) => {
            match loaded.can_unload_now() {
                Ok(hresult) => report.pass(
                    "dll.load",
                    "DLL 載入",
                    format!(
                        "{}；DllCanUnloadNow = {}",
                        path.display(),
                        report::hex(hresult.0 as u32)
                    ),
                ),
                Err(error) => report.fail(
                    "dll.load",
                    "DLL 載入",
                    format!("{} 已載入，但 {error}", path.display()),
                    "確認 DLL 是完整的 release 建置（匯出 DllCanUnloadNow），或重新安裝",
                ),
            }
            library = Some(loaded);
        }
        Err(error) => report.fail(
            "dll.load",
            "DLL 載入",
            error.to_string(),
            "確認 jc-tsf.dll 與它的依賴（VC runtime）在同一個目錄；用 --dll 指定正確路徑",
        ),
    }

    // ── 2. DllGetClassObject → IClassFactory ─────────────────────────
    let mut factory: Option<IClassFactory> = None;
    match library.as_ref() {
        Some(loaded) => match loaded.get_class_object(&product::CLSID_JC_TSF) {
            Ok(class_factory) => {
                report.pass(
                    "dll.class_factory",
                    "類廠",
                    format!(
                        "DllGetClassObject(CLSID_JC_TSF, IClassFactory) = S_OK（{}）",
                        report::hex(0)
                    ),
                );
                factory = Some(class_factory);
            }
            Err(error) => report.fail(
                "dll.class_factory",
                "類廠",
                error.to_string(),
                "DLL 與 jc-tsf-smoke 的 CLSID 不一致或 DLL 版本錯誤；重新建置／安裝",
            ),
        },
        None => report.skip("dll.class_factory", "類廠", "DLL 未載入"),
    }

    // ── 3. CreateInstance 之後的 QI 與物件身分 ───────────────────────
    let mut text_service: Option<ITfTextInputProcessor> = None;
    match factory.as_ref() {
        Some(class_factory) => {
            // SAFETY: 不支援 aggregation，外部 IUnknown 傳 None。
            match unsafe { class_factory.CreateInstance::<_, IUnknown>(None::<&IUnknown>) } {
                Ok(unknown) => {
                    let mut cells = Vec::new();
                    text_service =
                        qi::<ITfTextInputProcessor>(&unknown, "ITfTextInputProcessor", &mut cells);
                    let key_sink = qi::<ITfKeyEventSink>(&unknown, "ITfKeyEventSink", &mut cells);
                    let _ =
                        qi::<ITfThreadMgrEventSink>(&unknown, "ITfThreadMgrEventSink", &mut cells);
                    let _ = qi::<ITfCompositionSink>(&unknown, "ITfCompositionSink", &mut cells);

                    // msctf 的硬性要求：ITfKeyEventSink 必須是 ITfTextInputProcessor
                    // 的同一個 COM 物件。QI 到 IUnknown 後比指標，才不會被不同的
                    // vtable 指標誤導。
                    let identity = match (text_service.as_ref(), key_sink.as_ref()) {
                        (Some(text_service), Some(key_sink)) => {
                            match (text_service.cast::<IUnknown>(), key_sink.cast::<IUnknown>()) {
                                (Ok(one), Ok(other)) => {
                                    Some(Interface::as_raw(&one) == Interface::as_raw(&other))
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };

                    let mut detail = cells.join("、");
                    match identity {
                        Some(true) => {
                            detail.push_str(
                                "；ITfKeyEventSink 與 ITfTextInputProcessor 是同一個物件",
                            );
                            report.pass("dll.vtable", "vtable 與物件身分", detail);
                        }
                        Some(false) => {
                            detail.push_str(
                                "；ITfKeyEventSink 不是 ITfTextInputProcessor 同一個物件（msctf 不接受獨立 sink）",
                            );
                            report.fail(
                                "dll.vtable",
                                "vtable 與物件身分",
                                detail,
                                "把 ITfKeyEventSink 實作在同一個物件上（jc-tsf 的 Control）",
                            );
                        }
                        None => {
                            detail.push_str("；無法比對物件身分（QI IUnknown 失敗）");
                            report.fail(
                                "dll.vtable",
                                "vtable 與物件身分",
                                detail,
                                "檢查 DllGetClassObject 回傳的物件與 QueryInterface 實作",
                            );
                        }
                    }
                }
                Err(error) => report.fail(
                    "dll.vtable",
                    "vtable 與物件身分",
                    format!("CreateInstance 回 {}", report::hex(error.code().0 as u32)),
                    "檢查 IClassFactory::CreateInstance 的實作與 DLL 版本",
                ),
            }
        }
        None => report.skip("dll.vtable", "vtable 與物件身分", "沒有 IClassFactory"),
    }

    // ── 4. msctf 的 ITfThreadMgr ─────────────────────────────────────
    let mut thread_mgr: Option<ITfThreadMgr> = None;
    // SAFETY: 標準的 in-proc COM 建立，不聚合。
    match unsafe {
        CoCreateInstance::<_, ITfThreadMgr>(
            &CLSID_TF_ThreadMgr,
            None::<&IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
    } {
        Ok(manager) => {
            report.pass(
                "tsf.thread_mgr",
                "ITfThreadMgr",
                format!(
                    "CoCreateInstance(CLSID_TF_ThreadMgr) = S_OK（{}）",
                    report::hex(0)
                ),
            );
            thread_mgr = Some(manager);
        }
        Err(error) => report.fail(
            "tsf.thread_mgr",
            "ITfThreadMgr",
            format!(
                "CoCreateInstance(CLSID_TF_ThreadMgr) 回 HRESULT {}",
                report::hex(error.code().0 as u32)
            ),
            "msctf.dll 不在或被系統精簡；這不是 jc-tsf 的問題，先修復 Windows TSF",
        ),
    }

    // ── 5. Activate ─────────────────────────────────────────────────
    match (text_service.as_ref(), thread_mgr.as_ref()) {
        (Some(service), Some(manager)) => {
            // SAFETY: manager 剛由本執行緒建立；sink 的存活期由 service 管理。
            match unsafe { manager.Activate() } {
                Ok(application_client_id) => {
                    report.info(format!(
                        "ITfThreadMgr::Activate → 應用程式 client_id={application_client_id}（不是 TIP 的 client_id）"
                    ));

                    // 「已為使用者啟用」不等於「這個執行緒正在用它」：msctf 要等到
                    // ActivateProfile 才會載入 TIP，並用真正的 TIP client id 呼叫
                    // Activate。少了這一步，GetClientId 建出的 id 背後沒有 TIP，
                    // AdviseKeyEventSink 就回 E_INVALIDARG（這就是先前的
                    // 0x80070057）。TF_IPPMF_FORPROCESS 只影響本行程。
                    let profile_activated = match tsf_profile::activate_ours() {
                        Ok(()) => {
                            report.info(
                                "ITfInputProcessorProfileMgr::ActivateProfile(TF_IPPMF_FORPROCESS) = S_OK；                                 msctf 已在本行程載入 jc-tsf",
                            );
                            true
                        }
                        Err(error) => {
                            report.fail(
                                "tsf.activate",
                                "Activate",
                                format!(
                                    "{error}；msctf 沒有載入 jc-tsf，Activate 無法驗證。\
                                     只註冊 profile 不夠，它還不是這個執行緒的作用中輸入法"
                                ),
                                "以管理員執行 jc-tip-register machine-register，再以目前使用者執行 user-install",
                            );
                            false
                        }
                    };

                    // 應用程式與文字服務的 TfClientId 來源不同：前者從
                    // ITfThreadMgr::Activate 拿到，後者由 TSF 在
                    // ITfTextInputProcessor::Activate 傳進來。把前者傳給 TIP 會讓
                    // msctf 的 AdviseKeyEventSink 回 E_INVALIDARG，因為它不是 TIP
                    // 的 client id。這裡自己建的 TIP 物件用 ITfClientId::GetClientId
                    // 取得與 CLSID 綁定的 id；上一行 ActivateProfile 之後 msctf 已經
                    // 用同一個 id 激活過 TIP，這時會回 CONNECT_E_ADVISELIMIT。
                    let mut activated = false;
                    if profile_activated {
                        let tip_client_id =
                            manager.cast::<ITfClientId>().and_then(|client_id| unsafe {
                                client_id.GetClientId(&product::CLSID_JC_TSF)
                            });
                        match tip_client_id {
                        Ok(tip_client_id) => {
                            report.info(format!(
                                "ITfClientId::GetClientId(CLSID_JC_TSF) → TIP client_id={tip_client_id}"
                            ));
                            match unsafe { service.Activate(manager, tip_client_id) } {
                                Ok(()) => {
                                    report.pass(
                                        "tsf.activate",
                                        "Activate",
                                        format!(
                                            "ITfThreadMgr::Activate → 應用程式 client_id={application_client_id}；\
                                             ITfClientId::GetClientId → TIP client_id={tip_client_id}；\
                                             ITfTextInputProcessor::Activate = S_OK（{}）",
                                            report::hex(0)
                                        ),
                                    );
                                    activated = true;
                                }
                                Err(error) => match classify_tip_activate(error.code().0) {
                                    TipActivate::AlreadyActivated => {
                                        report.pass(
                                            "tsf.activate",
                                            "Activate",
                                            format!(
                                                "TIP client_id={tip_client_id} 已有 key sink（HRESULT {}）：\
                                                 msctf 在 ActivateProfile 時已用真正的 TIP client id 激活本 TIP，\
                                                 手動建立的物件不需再激活一次",
                                                report::hex(CONNECT_E_ADVISELIMIT.0 as u32)
                                            ),
                                        );
                                        activated = true;
                                    }
                                    TipActivate::Failed(_) | TipActivate::Activated => report.fail(
                                        "tsf.activate",
                                        "Activate",
                                        format!(
                                            "ITfTextInputProcessor::Activate 回 HRESULT {}；詳細步驟見 TSF 日誌：\
                                             設定環境變數 JC_TSF_LOG=1 後重跑，會追加到 %LOCALAPPDATA%\\jc-ime\\tsf.log\
                                             （沒設定時只走 OutputDebugStringW，用 DebugView 才看得到）",
                                            report::hex(error.code().0 as u32)
                                        ),
                                        "設定 JC_TSF_LOG=1 重跑取得 tsf.log；確認 Activate 收到的是 TIP client id（不是應用程式 id）",
                                    ),
                                },
                            }
                        }
                        Err(error) => report.fail(
                            "tsf.activate",
                            "Activate",
                            format!(
                                "取得 TIP client_id 失敗：HRESULT {}",
                                report::hex(error.code().0 as u32)
                            ),
                            "確認 ITfThreadMgr 支援 ITfClientId；這是 msctf 版本問題，先修復 Windows TSF",
                        ),
                    }
                    }

                    if activated {
                        report_key_chain(&report, manager);
                    }

                    // Activate 不再連 pipe，檢查 6 的探測與 TIP 無關；Deactivate
                    // 仍會非同步放掉 worker，保留短等待讓可能殘留的連線收乾淨。
                    // 純診斷：以下時間戳每印一行就 flush，確保行程卡住時最後一行
                    // 不會留在緩衝區，一眼就能看出卡在哪一步。
                    // SAFETY: 上面才 Activate 過；重複 Deactivate 由 jc-tsf 自己容錯。
                    eprintln!(
                        "[smoke {:.3}s] before service.Deactivate()",
                        start.elapsed().as_secs_f64()
                    );
                    let _ = std::io::stderr().flush();
                    let _ = unsafe { service.Deactivate() };
                    eprintln!(
                        "[smoke {:.3}s] after service.Deactivate()",
                        start.elapsed().as_secs_f64()
                    );
                    let _ = std::io::stderr().flush();
                    eprintln!(
                        "[smoke {:.3}s] before manager.Deactivate()",
                        start.elapsed().as_secs_f64()
                    );
                    let _ = std::io::stderr().flush();
                    let _ = unsafe { manager.Deactivate() };
                    eprintln!(
                        "[smoke {:.3}s] after manager.Deactivate()",
                        start.elapsed().as_secs_f64()
                    );
                    let _ = std::io::stderr().flush();
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(error) => report.fail(
                    "tsf.activate",
                    "Activate",
                    format!(
                        "ITfThreadMgr::Activate 回 HRESULT {}",
                        report::hex(error.code().0 as u32)
                    ),
                    "TSF 可能被同執行緒的其他程式佔用；關掉其他使用 TSF 的程式再重跑",
                ),
            }
        }
        _ => report.skip(
            "tsf.activate",
            "Activate",
            "缺少 ITfTextInputProcessor 或 ITfThreadMgr",
        ),
    }

    // ── 6. 引擎 pipe ────────────────────────────────────────────────
    eprintln!(
        "[smoke {:.3}s] before engine::probe()",
        start.elapsed().as_secs_f64()
    );
    let _ = std::io::stderr().flush();
    let engine_probe = engine::probe();
    match &engine_probe {
        engine::Probe::Connected(_) => {
            report.pass("engine.pipe", "引擎 pipe", engine_probe.pipe_detail())
        }
        engine::Probe::Unavailable { .. } => report.skip(
            "engine.pipe",
            "引擎 pipe",
            format!(
                "{}；{}；引擎未啟動或未安裝，只警告不判定失敗",
                engine_probe.pipe_detail(),
                autostart_hint()
            ),
        ),
    }
    eprintln!(
        "[smoke {:.3}s] after engine::probe()",
        start.elapsed().as_secs_f64()
    );
    let _ = std::io::stderr().flush();

    if let Some(test_key) = library.as_ref().and_then(|loaded| loaded.test_key()) {
        // 讓引擎把上一條連線收乾淨，再直驅 DLL 的測試匯出。
        std::thread::sleep(Duration::from_millis(100));
        let code = test_key(0x4A);
        let text = match code {
            0 => "0（按鍵已送達引擎並取得回應）".to_string(),
            1 => "1（這個鍵不歸輸入法處理）".to_string(),
            2 => "2（連不上引擎 pipe）".to_string(),
            3 => "3（引擎往返失敗）".to_string(),
            other => format!("{other}（未定義的回應碼）"),
        };
        report.info(format!(
            "jc_test_key(VK_J = 0x4A) = {text}；僅供參考，不影響結果"
        ));
    } else {
        report.info("jc-tsf.dll 沒有 jc_test_key 匯出（release 建置正常，需要時開 test-export）");
    }

    // ── 7. 註冊探測（兩個機碼檢視） ──────────────────────────────────
    //
    // 對應手動驗證的 `reg query ... /reg:64` 與 `/reg:32`：兩個檢視都要看到同一個
    // CLSID，而且各指到對應架構的 DLL。只看得到一邊就代表有一個架構的程式打不出
    // 字，所以算失敗；完全沒安裝才算略過。
    match registry::probe(&clsid_text) {
        Ok(probe) => match probe.verdict(&clsid_text) {
            registry::Verdict::Installed { x64_dll, x86_dll } => report.pass(
                "registry.tip",
                "註冊探測",
                format!(
                    "兩個檢視都有 {clsid_text}：64 位元 → {x64_dll}（{} 個 TIP）、\
                     32 位元 → {x86_dll}（{} 個 TIP）",
                    probe.x64.tip_count, probe.x86.tip_count
                ),
            ),
            registry::Verdict::NotInstalled => report.skip(
                "registry.tip",
                "註冊探測",
                format!("64 位元與 32 位元檢視都沒有 {clsid_text}（尚未安裝，不判定失敗）"),
            ),
            registry::Verdict::Broken(message) => report.fail(
                "registry.tip",
                "註冊探測",
                message,
                "CLSID 由 MSI 寫，先修復安裝（msiexec /fa <msi>）；再以管理員執行 jc-tip-register machine-register 補 profile 與分類",
            ),
        },
        Err(error) => report.skip(
            "registry.tip",
            "註冊探測",
            format!("{error}（不判定失敗）"),
        ),
    }

    // ── 8. 安裝檔案 ─────────────────────────────────────────────────
    check_install_files(&mut report, &clsid_text);

    // ── 9. machine profile 機碼 ─────────────────────────────────────
    check_machine_profile(&mut report, &clsid_text, &profile_text);

    // ── 10. 使用者啟用（TSF API） ───────────────────────────────────
    check_user_profile(&mut report, com_ready, &clsid_text);

    // ── 11. 使用者語言清單 ──────────────────────────────────────────
    check_language_list(&mut report, &clsid_text, &profile_text);

    // ── 12. 引擎細節 ────────────────────────────────────────────────
    match engine_probe.engine_detail() {
        Ok(detail) if !engine_probe.draining() => {
            if engine_probe.has_bigram() != Some(true) {
                report.fail(
                    "engine.detail",
                    "引擎細節",
                    format!("{detail}；health 沒有回報 has_bigram=true"),
                    "確認 %ProgramData%\\jc-ime\\data\\build-zh-bigram\\lm.bin 存在且與 manifest 相符；重新啟動引擎",
                );
            } else {
                match engine_probe.english_words() {
                    Some(words) if words > 1_000 => {
                        report.pass("engine.detail", "引擎細節", detail);
                    }
                    Some(words) => report.fail(
                        "engine.detail",
                        "引擎細節",
                        format!("{detail}；英文只有 {words} 詞，應是內建原型"),
                        "確認 %ProgramData%\\jc-ime\\data\\english\\words.bin 存在且與 manifest 相符；重新啟動引擎",
                    ),
                    None => report.warn(
                        "engine.detail",
                        "引擎細節",
                        format!("{detail}；health 沒有回報 english_words，無法確認英文資料包"),
                    ),
                }
            }
        }
        Ok(detail) => report.warn(
            "engine.detail",
            "引擎細節",
            format!("{detail}；引擎正在交接，重跑或重新啟動引擎後再確認"),
        ),
        Err(detail) => report.skip(
            "engine.detail",
            "引擎細節",
            format!("{detail}；引擎未啟動時無法取得版本與健康狀態"),
        ),
    }

    // 慢請求只做觀測：WARN 不影響結束碼，但 JSON 與人類輸出都帶得出 op／耗時。
    if let Some((op, ms, input_len)) = engine_probe.last_slow() {
        report.warn_slow(
            "engine.slow",
            "慢請求",
            format!("最近一次慢請求 op={op}、耗時 {ms}ms、輸入長度 {input_len} 位元組"),
            &op,
            ms,
            input_len,
        );
    }

    // ── 13. UI 探針 ─────────────────────────────────────────────────
    if com_ready {
        check_ui_probe(&mut report);
    } else {
        report.skip("ui.probe", "UI 探針", "COM 沒有初始化，無法建立 TSF 物件");
    }

    // 放掉所有 COM 參照後再看一次：S_OK 代表沒有漏掉的引用。
    drop(text_service);
    drop(thread_mgr);
    drop(factory);
    if let Some(loaded) = &library {
        if let Ok(hresult) = loaded.can_unload_now() {
            report.info(format!(
                "釋放物件後 DllCanUnloadNow = {}（S_OK 代表沒有 refcount 洩漏）",
                report::hex(hresult.0 as u32)
            ));
        }
    }
    drop(library);

    if com_ready {
        // SAFETY: 上面的 CoInitializeEx 成功，配對呼叫一次。
        unsafe { CoUninitialize() };
    }

    report.finish();
    exit_code(&report)
}

/// 把報表轉成行程結束碼：有 `[FAIL]` 才是 1。
fn exit_code(report: &Report) -> std::process::ExitCode {
    if report.failed() == 0 {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

/// 檢查 13：真的開視窗、送注音、確認候選窗出現與消失。
///
/// 沒有互動桌面（`Headless`）、TIP 沒接到按鍵（`Inconclusive`）或本產品沒
/// 啟用（`Unavailable`）都記成 `[SKIP]`，不讓環境問題把 CI 判成產品失敗。
fn check_ui_probe(report: &mut Report) {
    let outcome = ui_probe::run();
    match outcome.verdict {
        ui_probe::Verdict::Headless
        | ui_probe::Verdict::Inconclusive
        | ui_probe::Verdict::Unavailable => {
            report.ui_probe(Status::Skip, outcome.detail, None, outcome.cells)
        }
        ui_probe::Verdict::Pass => {
            report.ui_probe(Status::Pass, outcome.detail, None, outcome.cells)
        }
        ui_probe::Verdict::Fail => report.ui_probe(
            Status::Fail,
            outcome.detail,
            Some(UI_PROBE_FIX.to_string()),
            outcome.cells,
        ),
    }
}

/// 檢查 8：從 `InprocServer32` 推出安裝目錄，確認四個必要檔案並輸出 SHA-256。
fn check_install_files(report: &mut Report, clsid_text: &str) {
    let inproc = match registry::read_install_inproc(clsid_text) {
        Ok(Some(path)) => path,
        Ok(None) => {
            report.skip(
                "install.files",
                "安裝檔案",
                "找不到 InprocServer32（尚未安裝）",
            );
            return;
        }
        Err(error) => {
            report.skip(
                "install.files",
                "安裝檔案",
                format!("{error}（不判定失敗）"),
            );
            return;
        }
    };

    let Some(probe) = install::probe(&inproc) else {
        report.skip(
            "install.files",
            "安裝檔案",
            format!("InprocServer32 的路徑無法推出安裝目錄：{inproc}"),
        );
        return;
    };

    let mut missing = Vec::new();
    let mut details = Vec::new();
    let mut without_version = 0usize;
    for file in &probe.files {
        if !file.present {
            missing.push(file.path.display().to_string());
        } else {
            if file.missing_version() {
                without_version += 1;
            }
            details.push(file.detail());
        }
    }
    if !probe.manifest.present {
        missing.push(probe.manifest.path.display().to_string());
    } else {
        details.push(probe.manifest.sha_detail());
    }
    if !probe.english.present {
        missing.push(probe.english.path.display().to_string());
    } else {
        details.push(probe.english.sha_detail());
    }

    if !missing.is_empty() {
        report.fail(
            "install.files",
            "安裝檔案",
            format!(
                "{} 缺少：{}；已存在：{}",
                probe.dir.display(),
                missing.join("、"),
                if details.is_empty() {
                    "（無）".to_string()
                } else {
                    details.join("；")
                }
            ),
            "修復安裝（msiexec /fa <msi>）；資料檔遺失時確認 %ProgramData%\\jc-ime\\data 沒有被手動刪除",
        );
        return;
    }

    let detail = format!("安裝目錄 {}；{}", probe.dir.display(), details.join("；"));
    if without_version > 0 {
        report.warn(
            "install.files",
            "安裝檔案",
            format!("{detail}；{without_version} 個 PE 檔沒有版本資源（不影響載入）"),
        );
    } else {
        report.pass("install.files", "安裝檔案", detail);
    }
}

/// 檢查 9：`HKLM\...\LanguageProfile\0x00000404\{PROFILE}` 的 64／32 位元檢視。
fn check_machine_profile(report: &mut Report, clsid_text: &str, profile_text: &str) {
    let key_path = profile::machine_profile_key(clsid_text, product::LANGID_ZH_TW, profile_text);
    let views = [("64 位元", KEY_WOW64_64KEY), ("32 位元", KEY_WOW64_32KEY)];

    let mut problems = Vec::new();
    let mut read_errors = Vec::new();
    let mut notes = Vec::new();
    let mut details = Vec::new();
    let mut present = 0usize;
    let mut missing = 0usize;

    for (label, view) in views {
        let canonical =
            registry::read_machine_profile(clsid_text, product::LANGID_ZH_TW, profile_text, view);
        let (profile, legacy) = match canonical {
            Ok(Some(profile)) => (Some(profile), false),
            Ok(None) => match registry::read_legacy_machine_profile(
                clsid_text,
                product::LANGID_ZH_TW,
                profile_text,
                view,
            ) {
                Ok(Some(profile)) => (Some(profile), true),
                Ok(None) => (None, false),
                Err(error) => {
                    read_errors.push(format!("{label}檢視讀取失敗：{error}"));
                    continue;
                }
            },
            Err(error) => {
                read_errors.push(format!("{label}檢視讀取失敗：{error}"));
                continue;
            }
        };

        let Some(profile) = profile else {
            missing += 1;
            problems.push(format!("{label}檢視的 {key_path} 不存在"));
            continue;
        };

        present += 1;
        if legacy {
            notes.push(format!(
                "{label}檢視只有非標準的 0x0404 機碼；msctf 讀的是 0x00000404"
            ));
        }

        let description = profile.description.clone().unwrap_or_default();
        if description.is_empty() {
            problems.push(format!("{label}檢視的 Description 是空的"));
        }
        match profile.enable {
            Some(enable) => {
                details.push(format!(
                    "{label}：Enable={enable}、Description={description}"
                ));
            }
            None => {
                details.push(format!("{label}：Enable 不存在、Description={description}"));
                notes.push(format!("{label}檢視沒有 Enable 值"));
            }
        }
        if profile.icon_file.is_none() {
            notes.push(format!("{label}檢視沒有 IconFile（正常，安裝器不帶圖示）"));
        }
    }

    // 兩個檢視都讀不到、而且都是「不存在」：尚未安裝，不是失敗。
    if present == 0 && missing == 2 && read_errors.is_empty() {
        report.skip(
            "profile.machine",
            "machine profile 機碼",
            format!("兩個檢視都沒有 {key_path}（尚未安裝）"),
        );
        return;
    }

    if !problems.is_empty() || !read_errors.is_empty() {
        let note = if notes.is_empty() {
            String::new()
        } else {
            format!("；{}", notes.join("；"))
        };
        let details = if details.is_empty() {
            String::new()
        } else {
            format!("；{}", details.join("；"))
        };
        report.fail(
            "profile.machine",
            "machine profile 機碼",
            format!(
                "{}{}{details}{note}",
                problems.join("；"),
                if read_errors.is_empty() {
                    String::new()
                } else {
                    format!("；{}", read_errors.join("；"))
                }
            ),
            "以管理員執行 jc-tip-register machine-register；若機碼完全不存在，先修復安裝（msiexec /fa <msi>）",
        );
        return;
    }

    let detail = format!("{key_path}：{}", details.join("；"));
    if notes.is_empty() {
        report.pass("profile.machine", "machine profile 機碼", detail);
    } else {
        report.warn(
            "profile.machine",
            "machine profile 機碼",
            format!("{detail}；{}", notes.join("；")),
        );
    }
}

/// 引擎沒在跑時，補一句「登入後會不會自己起來」。
///
/// 少了這句，doctor 全綠但打不出字時沒人知道是 Run 機碼沒寫或沒生效。
fn autostart_hint() -> String {
    match registry::read_engine_autostart() {
        Some(command) => format!("自動啟動機碼存在：{command}"),
        None => {
            "自動啟動機碼不存在（HKLM\\..\\Run\\jc-ime-engine）；登入後不會自己起來".to_string()
        }
    }
}

/// 檢查 10：用 TSF API 問目前使用者是否已啟用、profile 是否被列舉。
fn check_user_profile(report: &mut Report, com_ready: bool, clsid_text: &str) {
    if !com_ready {
        report.skip(
            "profile.user",
            "使用者啟用",
            "COM 沒有初始化，無法呼叫 TSF API",
        );
        return;
    }

    let state = match tsf_profile::query() {
        Ok(state) => state,
        Err(error) => {
            report.skip(
                "profile.user",
                "使用者啟用",
                format!("{error}（無法查詢 TSF API）"),
            );
            return;
        }
    };

    let detail = state.detail();
    match state.verdict() {
        Some(UserProfileVerdict::Ready) => report.pass("profile.user", "使用者啟用", detail),
        Some(UserProfileVerdict::Disabled) => report.fail(
            "profile.user",
            "使用者啟用",
            detail,
            format!(
                "以目前使用者執行 \"{}\" user-install",
                register_exe(clsid_text)
            ),
        ),
        Some(UserProfileVerdict::NotEnumerated) => report.fail(
            "profile.user",
            "使用者啟用",
            detail,
            "以管理員執行 jc-tip-register machine-register（profile 沒有被 EnumLanguageProfiles 列出）",
        ),
        None => report.skip(
            "profile.user",
            "使用者啟用",
            format!("{detail}；TSF API 沒有回傳完整結果"),
        ),
    }
}

/// 檢查 11：`HKCU\Control Panel\International\User Profile` 的語言清單。
fn check_language_list(report: &mut Report, clsid_text: &str, profile_text: &str) {
    let list = match registry::read_user_language_list() {
        Ok(list) => list,
        Err(error) => {
            report.skip(
                "profile.language_list",
                "語言清單",
                format!("{error}（不判定失敗）"),
            );
            return;
        }
    };

    let has_language = list
        .languages
        .iter()
        .any(|entry| profile::language_matches(entry, profile::LANGUAGE_ZH_HANT_TW));
    let expected_tip = profile::input_method_tip(clsid_text, profile_text, product::LANGID_ZH_TW);
    let has_tip = list
        .input_method_tips
        .iter()
        .any(|entry| profile::tip_matches(entry, clsid_text, profile_text, product::LANGID_ZH_TW));

    let detail = format!(
        "Languages {} 項（{}）；zh-Hant-TW\\InputMethodTips {} 項（{}）",
        list.languages.len(),
        if has_language {
            "含 zh-Hant-TW"
        } else {
            "缺 zh-Hant-TW"
        },
        list.input_method_tips.len(),
        if has_tip {
            format!("含 {expected_tip}")
        } else {
            "缺本產品".to_string()
        }
    );

    if has_language && has_tip {
        report.pass("profile.language_list", "語言清單", detail);
    } else {
        report.fail(
            "profile.language_list",
            "語言清單",
            detail,
            format!(
                "以目前使用者執行 \"{}\" user-install",
                register_exe(clsid_text)
            ),
        );
    }
}

/// `jc-tip-register.exe` 的完整路徑；取不到安裝目錄時用預設安裝路徑。
fn register_exe(clsid_text: &str) -> String {
    let dir = registry::read_install_inproc(clsid_text)
        .ok()
        .flatten()
        .and_then(|path| profile::install_dir(&path))
        .map(|dir| dir.display().to_string())
        .unwrap_or_else(|| r"C:\Program Files\jc-ime".to_string());
    format!(r"{dir}\jc-tip-register.exe")
}

/// `ITfTextInputProcessor::Activate` 的判定結果。
#[derive(Debug, PartialEq, Eq)]
enum TipActivate {
    /// 這次成功註冊了 sink。
    Activated,
    /// 同一個 TIP client id 已經有 sink：msctf 自己激活過，視為成功。
    AlreadyActivated,
    /// 真的失敗；保留原始 HRESULT 供報表顯示。
    Failed(i32),
}

/// 把 `Activate` 的 HRESULT 轉成判定。
///
/// `CONNECT_E_ADVISELIMIT` 代表 tid 已經有 key sink。對手動建立的 TIP 物件
/// 而言，這正是「msctf 在 `ITfThreadMgr::Activate` 時已激活同一個 TIP」的證據，
/// 不能當成失敗；其餘錯誤（`E_INVALIDARG` 等）都要照實回報。
fn classify_tip_activate(hresult: i32) -> TipActivate {
    match hresult {
        0 => TipActivate::Activated,
        code if code == CONNECT_E_ADVISELIMIT.0 => TipActivate::AlreadyActivated,
        code => TipActivate::Failed(code),
    }
}

/// 用 `ITfKeystrokeMgr::TestKeyDown` 確認按鍵鏈接得上（只印資訊）。
///
/// 引擎沒連線時 `OnTestKeyDown` 一律放行，所以 `consumed=false` 是正常結果；
/// 這裡不判定成敗。
fn report_key_chain(report: &Report, manager: &ITfThreadMgr) {
    match manager.cast::<ITfKeystrokeMgr>() {
        Ok(keystroke) => {
            // SAFETY: manager 已 Activate；虛擬鍵 U 與任意 lParam 都不會碰記憶體。
            match unsafe { keystroke.TestKeyDown(WPARAM(0x55), LPARAM(1)) } {
                Ok(consumed) => report.info(format!(
                    "ITfKeystrokeMgr::TestKeyDown('u') consumed={}（引擎未連線時應為 false）",
                    consumed.as_bool()
                )),
                Err(error) => report.info(format!(
                    "ITfKeystrokeMgr::TestKeyDown 回 HRESULT {}",
                    report::hex(error.code().0 as u32)
                )),
            }
        }
        Err(error) => report.info(format!(
            "ITfThreadMgr::cast::<ITfKeystrokeMgr> 回 HRESULT {}",
            report::hex(error.code().0 as u32)
        )),
    }
}

/// QI 一個介面，並把結果寫進 `cells`；成功才回 `Some`。
fn qi<T: Interface>(unknown: &IUnknown, name: &str, cells: &mut Vec<String>) -> Option<T> {
    match unknown.cast::<T>() {
        Ok(value) => {
            cells.push(format!("{name}=S_OK"));
            Some(value)
        }
        Err(error) => {
            cells.push(format!("{name}={}", report::hex(error.code().0 as u32)));
            None
        }
    }
}

/// 只跑哪一項檢查。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckFilter {
    /// 檢查 13：UI 探針。
    UiProbe,
}

/// 命令列選項。
#[derive(Debug, PartialEq, Eq)]
struct Options {
    /// `--dll` 或位置參數給的路徑。
    dll: Option<String>,
    /// stdout 只輸出 JSON。
    json: bool,
    /// 只跑指定的檢查；`None` 代表 13 項全跑。
    only: Option<CheckFilter>,
}

/// 命令列解析結果。
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    /// 執行檢查。
    Run(Options),
    /// 印用法後以 0 結束。
    Help,
    /// 參數錯誤，以 2 結束。
    Error,
}

/// 解析參數。
fn parse_args(args: &[String]) -> Cli {
    let mut dll = None;
    let mut json = false;
    let mut only = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => {
                print_usage();
                return Cli::Help;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            "--ui-probe" => {
                only = Some(CheckFilter::UiProbe);
                index += 1;
            }
            "--check" => match args.get(index + 1) {
                Some(name) => match check_filter(name) {
                    Some(filter) => {
                        only = Some(filter);
                        index += 2;
                    }
                    None => {
                        eprintln!("--check 只支援 ui-probe（也接受 13 與 ui.probe）：{name}");
                        return Cli::Error;
                    }
                },
                None => {
                    eprintln!("--check 需要一個檢查 id");
                    return Cli::Error;
                }
            },
            "--dll" => match args.get(index + 1) {
                Some(path) => {
                    dll = Some(path.clone());
                    index += 2;
                }
                None => {
                    eprintln!("--dll 需要一個路徑");
                    return Cli::Error;
                }
            },
            other if !other.starts_with('-') => {
                dll = Some(other.to_string());
                index += 1;
            }
            other => {
                eprintln!("未知參數：{other}");
                print_usage();
                return Cli::Error;
            }
        }
    }
    Cli::Run(Options { dll, json, only })
}

/// 把 `--check` 的值轉成檢查；目前只開放 UI 探針。
fn check_filter(name: &str) -> Option<CheckFilter> {
    match name {
        "ui-probe" | "ui.probe" | "13" => Some(CheckFilter::UiProbe),
        _ => None,
    }
}

/// 印出用法。
fn print_usage() {
    println!("用法：jc-tsf-smoke.exe [--json] [--dll <path>] [--check ui-probe | --ui-probe]");
    println!();
    println!("  --json          stdout 只輸出一個 jc-ime-doctor/1 JSON 物件；診斷訊息走 stderr");
    println!("  --dll <path>   指定 jc-tsf.dll。預設依序找 JC_TSF_DLL、執行檔同目錄、");
    println!("                 LoadLibraryW 的標準搜尋路徑");
    println!("  --check <id>   只跑指定檢查；目前支援 ui-probe（也接受 13、ui.probe）");
    println!("  --ui-probe     同 --check ui-probe：開視窗、打注音、驗候選窗出現與消失");
    println!("  --help         印出這份說明");
    println!();
    println!("環境變數：");
    println!("  JC_TSF_DLL     同 --dll");
    println!("  JC_IME_PIPE    覆寫引擎 pipe 名稱（預設讀 HKCU\\Software\\jc-ime\\EnginePipe）");
    println!();
    println!("結束碼：0 全部通過或只有警告；1 有 [FAIL]；2 參數錯誤。");
    println!("UI 探針在沒有互動桌面時回 inconclusive（[SKIP]），不算失敗。");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_dll_flag_and_positional() {
        let flag = vec!["--dll".to_string(), r"C:\a\jc-tsf.dll".to_string()];
        assert_eq!(
            parse_args(&flag),
            Cli::Run(Options {
                dll: Some(r"C:\a\jc-tsf.dll".to_string()),
                json: false,
                only: None,
            })
        );

        let positional = vec![r"C:\b\jc-tsf.dll".to_string()];
        assert_eq!(
            parse_args(&positional),
            Cli::Run(Options {
                dll: Some(r"C:\b\jc-tsf.dll".to_string()),
                json: false,
                only: None,
            })
        );

        assert_eq!(
            parse_args(&[]),
            Cli::Run(Options {
                dll: None,
                json: false,
                only: None,
            })
        );
    }

    #[test]
    fn parse_args_accepts_json_in_any_position() {
        let args = vec![
            "--json".to_string(),
            "--dll".to_string(),
            r"C:\a\jc-tsf.dll".to_string(),
        ];
        assert_eq!(
            parse_args(&args),
            Cli::Run(Options {
                dll: Some(r"C:\a\jc-tsf.dll".to_string()),
                json: true,
                only: None,
            })
        );
    }

    #[test]
    fn parse_args_accepts_ui_probe_shortcuts() {
        for args in [
            vec!["--check".to_string(), "ui-probe".to_string()],
            vec!["--check".to_string(), "ui.probe".to_string()],
            vec!["--check".to_string(), "13".to_string()],
            vec!["--ui-probe".to_string()],
        ] {
            assert_eq!(
                parse_args(&args),
                Cli::Run(Options {
                    dll: None,
                    json: false,
                    only: Some(CheckFilter::UiProbe),
                }),
                "{args:?}"
            );
        }

        let json = vec![
            "--ui-probe".to_string(),
            "--json".to_string(),
            "--dll".to_string(),
            r"C:\a\jc-tsf.dll".to_string(),
        ];
        assert_eq!(
            parse_args(&json),
            Cli::Run(Options {
                dll: Some(r"C:\a\jc-tsf.dll".to_string()),
                json: true,
                only: Some(CheckFilter::UiProbe),
            })
        );
    }

    #[test]
    fn parse_args_rejects_unknown_check() {
        let args = vec!["--check".to_string(), "profile.user".to_string()];
        assert_eq!(parse_args(&args), Cli::Error);
        let missing = vec!["--check".to_string()];
        assert_eq!(parse_args(&missing), Cli::Error);
    }

    #[test]
    fn parse_args_rejects_unknown_flags() {
        let args = vec!["--nope".to_string()];
        assert_eq!(parse_args(&args), Cli::Error);
    }

    #[test]
    fn tip_activate_accepts_repeated_activation() {
        // S_OK：第一次激活，或解除註冊後重新激活。
        assert_eq!(classify_tip_activate(0), TipActivate::Activated);
        // 已安裝的 TIP 會被 msctf 先激活；第二次手動激活拿到 ADVISELIMIT，
        // 這是重複執行也該通過的路徑。
        assert_eq!(
            classify_tip_activate(CONNECT_E_ADVISELIMIT.0),
            TipActivate::AlreadyActivated
        );
        // 之前把應用程式 client id 當成 TIP id 時的真機錯誤，必須判定失敗。
        let e_invalidarg = 0x8007_0057u32 as i32;
        assert_eq!(
            classify_tip_activate(e_invalidarg),
            TipActivate::Failed(e_invalidarg)
        );
    }

    #[test]
    fn register_exe_falls_back_to_default_install_dir() {
        let text = register_exe("{00000000-0000-0000-0000-000000000000}");
        assert!(text.ends_with("jc-tip-register.exe"), "{text}");
    }
}
