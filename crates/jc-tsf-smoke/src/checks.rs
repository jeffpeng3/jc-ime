//! 七項檢查的流程。
//!
//! 順序固定為：DLL 載入 → 類廠 → 介面 vtable／物件身分 → `ITfThreadMgr` →
//! `Activate` → 引擎 pipe → 註冊探測。前一步失敗時，後面依賴它的項目改印
//! `[SKIP]`，避免一個原因噴出一串失敗。

use std::time::Duration;

use windows::core::{IUnknown, Interface};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IClassFactory, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_ThreadMgr, ITfCompositionSink, ITfKeyEventSink, ITfKeystrokeMgr,
    ITfTextInputProcessor, ITfThreadMgr, ITfThreadMgrEventSink,
};

use crate::dll::{self, Library};
use crate::engine;
use crate::product;
use crate::registry;
use crate::report::{self, Report};

/// 執行全部檢查，回傳行程結束碼。
pub fn run(args: Vec<String>) -> std::process::ExitCode {
    let dll_arg = match parse_args(&args) {
        Cli::Run(dll) => dll,
        Cli::Help => return std::process::ExitCode::SUCCESS,
        Cli::Error => return std::process::ExitCode::from(2),
    };

    println!("jc-tsf 免管理員冒煙測試（{}）", product::DISPLAY_NAME);
    println!("不需要管理員；不會呼叫 ITfInputProcessorProfiles::Register。");
    println!();

    let mut report = Report::default();

    // TSF 的 COM 物件是 apartment-threaded；先初始化再建立任何物件。
    // SAFETY: 這條執行緒還沒有初始化 COM。
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    report.info(format!(
        "CoInitializeEx(APARTMENTTHREADED) = {}",
        report::hex(com.0 as u32)
    ));
    let com_ready = com.is_ok();

    let path = dll::resolve_path(dll_arg.as_deref());
    report.info(format!("DLL：{}", path.display()));
    report.info(format!(
        "CLSID {}；profile {}；LANGID {}",
        product::registry_key(&product::CLSID_JC_TSF),
        product::registry_key(&product::GUID_PROFILE_JC_TSF),
        report::hex(product::LANGID_ZH_TW as u32)
    ));

    // ── 1. DLL 載入與 DllCanUnloadNow ────────────────────────────────
    let mut library: Option<Library> = None;
    match Library::load(&path) {
        Ok(loaded) => {
            match loaded.can_unload_now() {
                Ok(hresult) => report.pass(
                    "1. LoadLibraryW + DllCanUnloadNow",
                    format!(
                        "{}；DllCanUnloadNow = {}",
                        path.display(),
                        report::hex(hresult.0 as u32)
                    ),
                ),
                Err(error) => report.fail(
                    "1. LoadLibraryW + DllCanUnloadNow",
                    format!("{} 已載入，但 {error}", path.display()),
                ),
            }
            library = Some(loaded);
        }
        Err(error) => report.fail("1. LoadLibraryW + DllCanUnloadNow", error.to_string()),
    }

    // ── 2. DllGetClassObject → IClassFactory ─────────────────────────
    let mut factory: Option<IClassFactory> = None;
    match library.as_ref() {
        Some(loaded) => match loaded.get_class_object(&product::CLSID_JC_TSF) {
            Ok(class_factory) => {
                report.pass(
                    "2. DllGetClassObject(CLSID_JC_TSF, IClassFactory)",
                    format!("S_OK（{}）", report::hex(0)),
                );
                factory = Some(class_factory);
            }
            Err(error) => report.fail(
                "2. DllGetClassObject(CLSID_JC_TSF, IClassFactory)",
                error.to_string(),
            ),
        },
        None => report.skip(
            "2. DllGetClassObject(CLSID_JC_TSF, IClassFactory)",
            "DLL 未載入",
        ),
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
                            report.pass("3. CreateInstance + QI（vtable 與物件身分）", detail);
                        }
                        Some(false) => {
                            detail.push_str(
                                "；ITfKeyEventSink 不是 ITfTextInputProcessor 同一個物件（msctf 不接受獨立 sink）",
                            );
                            report.fail("3. CreateInstance + QI（vtable 與物件身分）", detail);
                        }
                        None => {
                            detail.push_str("；無法比對物件身分（QI IUnknown 失敗）");
                            report.fail("3. CreateInstance + QI（vtable 與物件身分）", detail);
                        }
                    }
                }
                Err(error) => report.fail(
                    "3. CreateInstance + QI（vtable 與物件身分）",
                    format!("CreateInstance 回 {}", report::hex(error.code().0 as u32)),
                ),
            }
        }
        None => report.skip(
            "3. CreateInstance + QI（vtable 與物件身分）",
            "沒有 IClassFactory",
        ),
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
                "4. CoCreateInstance(CLSID_TF_ThreadMgr)",
                format!("S_OK（{}）", report::hex(0)),
            );
            thread_mgr = Some(manager);
        }
        Err(error) => report.fail(
            "4. CoCreateInstance(CLSID_TF_ThreadMgr)",
            format!("HRESULT {}", report::hex(error.code().0 as u32)),
        ),
    }

    // ── 5. Activate ─────────────────────────────────────────────────
    match (text_service.as_ref(), thread_mgr.as_ref()) {
        (Some(service), Some(manager)) => {
            // SAFETY: manager 剛由本執行緒建立；sink 的存活期由 service 管理。
            match unsafe { manager.Activate() } {
                Ok(client_id) => {
                    let mut activated = false;
                    match unsafe { service.Activate(manager, client_id) } {
                        Ok(()) => {
                            report.pass(
                                "5. Activate",
                                format!(
                                    "ITfThreadMgr::Activate → client_id={client_id}；ITfTextInputProcessor::Activate = S_OK（{}）",
                                    report::hex(0)
                                ),
                            );
                            activated = true;
                        }
                        Err(error) => report.fail(
                            "5. Activate",
                            format!(
                                "ITfTextInputProcessor::Activate 回 HRESULT {}",
                                report::hex(error.code().0 as u32)
                            ),
                        ),
                    }

                    if activated {
                        report_key_chain(&report, manager);
                    }

                    // 收乾淨：Deactivate 會停掉 worker 並關掉引擎 pipe。引擎一次只
                    // 服務一條連線，不先放掉的話檢查 6 會連不進去。
                    // SAFETY: 上面才 Activate 過；重複 Deactivate 由 jc-tsf 自己容錯。
                    let _ = unsafe { service.Deactivate() };
                    let _ = unsafe { manager.Deactivate() };
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(error) => report.fail(
                    "5. Activate",
                    format!(
                        "ITfThreadMgr::Activate 回 HRESULT {}",
                        report::hex(error.code().0 as u32)
                    ),
                ),
            }
        }
        _ => report.skip("5. Activate", "缺少 ITfTextInputProcessor 或 ITfThreadMgr"),
    }

    // ── 6. 引擎 pipe ────────────────────────────────────────────────
    match engine::probe() {
        Ok(detail) => report.pass("6. 引擎 pipe", detail),
        Err(reason) => report.skip(
            "6. 引擎 pipe",
            format!("{reason}；引擎未啟動或未安裝，只警告不判定失敗"),
        ),
    }

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

    // ── 7. 註冊探測（只回報） ────────────────────────────────────────
    let clsid_text = product::registry_key(&product::CLSID_JC_TSF);
    match registry::probe(&clsid_text) {
        Ok(found) if found.found => report.pass(
            "7. 註冊探測",
            format!(
                r"HKLM\{} 下有 {}（共 {} 個 TIP）",
                registry::TIP_KEY,
                clsid_text,
                found.total
            ),
        ),
        Ok(found) => report.skip(
            "7. 註冊探測",
            format!(
                r"HKLM\{} 下有 {} 個 TIP，沒有 {}（尚未安裝，不判定失敗）",
                registry::TIP_KEY,
                found.total,
                clsid_text
            ),
        ),
        Err(error) => report.skip("7. 註冊探測", format!("{error}（不判定失敗）")),
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

    println!();
    println!("結果：{}", report.summary());
    if com_ready {
        // SAFETY: 上面的 CoInitializeEx 成功，配對呼叫一次。
        unsafe { CoUninitialize() };
    }

    if report.failed() == 0 {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
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

/// 命令列解析結果。
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    /// 執行檢查；內容是 `--dll` 或位置參數給的路徑。
    Run(Option<String>),
    /// 印用法後以 0 結束。
    Help,
    /// 參數錯誤，以 2 結束。
    Error,
}

/// 解析參數。
fn parse_args(args: &[String]) -> Cli {
    let mut dll = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => {
                print_usage();
                return Cli::Help;
            }
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
    Cli::Run(dll)
}

/// 印出用法。
fn print_usage() {
    println!("用法：jc-tsf-smoke.exe [--dll <path>]");
    println!();
    println!("  --dll <path>   指定 jc-tsf.dll。預設依序找 JC_TSF_DLL、執行檔同目錄、");
    println!("                 LoadLibraryW 的標準搜尋路徑");
    println!("  --help         印出這份說明");
    println!();
    println!("環境變數：");
    println!("  JC_TSF_DLL     同 --dll");
    println!("  JC_IME_PIPE    覆寫引擎 pipe 名稱（預設讀 HKCU\\Software\\jc-ime\\EnginePipe）");
    println!();
    println!("結束碼：0 全部通過；1 有 [FAIL]；2 參數錯誤。");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_dll_flag_and_positional() {
        let flag = vec!["--dll".to_string(), r"C:\a\jc-tsf.dll".to_string()];
        assert_eq!(
            parse_args(&flag),
            Cli::Run(Some(r"C:\a\jc-tsf.dll".to_string()))
        );

        let positional = vec![r"C:\b\jc-tsf.dll".to_string()];
        assert_eq!(
            parse_args(&positional),
            Cli::Run(Some(r"C:\b\jc-tsf.dll".to_string()))
        );

        assert_eq!(parse_args(&[]), Cli::Run(None));
    }

    #[test]
    fn parse_args_rejects_unknown_flags() {
        let args = vec!["--nope".to_string()];
        assert_eq!(parse_args(&args), Cli::Error);
    }
}
