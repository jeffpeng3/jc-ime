//! 在記錄鍵碼或接觸引擎之前檢查 TSF 欄位政策；不讀取文件文字。
//!
//! 移植自 0c `platforms/windows/tsf/src/privacy.rs`（main `8f443af`）。
//! 0c 的這支檔案不依賴 `service`／`session`，因此可以與 T4、T6 平行移植。
//! `log`、`guarded`、物件計數與插入點探測在 adapter spike 階段先於本模組內
//! 提供過渡實作；T6 接到 TSF 生命週期時改指向真正的 `crate::*`。

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{E_FAIL, E_NOTIMPL, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::CoTaskMemFree;

use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_I4, VT_UNKNOWN};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, IAccessible, ROLE_SYSTEM_CLIENT, ROLE_SYSTEM_DOCUMENT,
    ROLE_SYSTEM_TEXT,
};
use windows::Win32::UI::Controls::EM_GETPASSWORDCHAR;
// 這兩個 Oleacc 狀態在 windows crate 中共用 Controls 的具名常數。
const STATE_SYSTEM_FOCUSABLE: u32 = windows::Win32::UI::Controls::STATE_SYSTEM_FOCUSABLE.0;
const STATE_SYSTEM_UNAVAILABLE: u32 = windows::Win32::UI::Controls::STATE_SYSTEM_UNAVAILABLE.0;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled};
use windows::Win32::UI::TextServices::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowLongW, GetWindowThreadProcessId, IsChild, SendMessageW, ES_PASSWORD,
    ES_READONLY, GWL_STYLE, OBJID_CLIENT, STATE_SYSTEM_FOCUSED, STATE_SYSTEM_PROTECTED,
    STATE_SYSTEM_READONLY,
};
use windows_core::{implement, ComObject, Interface};

// T6：log／物件計數／guarded 改接真正的 crate 層（與 service、langbar、edit 共用）；
// 插入點探測改接 crate::caret。
use crate::log::{self, guarded, object_created, object_destroyed, zdebug};

/// 欄位政策。密碼、PIN、唯讀或無法判斷的欄位不送引擎；只標 IS_PRIVATE 的欄位
/// （Chrome 無痕／訪客、Firefox 私密視窗的所有欄位都是）照常組字，但整段組字以
/// private 送引擎：不學習、不補標點、不做神經重排，外殼也不讀前文。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Blocked,
    Private,
    Open,
}

/// probe 失敗時寫入診斷 log 的呼叫端上下文：哪個 callback、哪個鍵、啟用旗標。
/// 只影響紀錄內容，不影響欄位判斷；測試用 [`Default`] 帶空值。
#[derive(Clone, Copy, Debug, Default)]
pub struct ProbeContext {
    pub callback: &'static str,
    pub vk: Option<u32>,
    pub activate_flags: u32,
}

/// 欄位政策 cache：key 是 ITfContext 的 IUnknown 相容介面指標（`as_raw()` 轉 usize），
/// 所以同一個 session 裡的帳號欄與密碼欄各自分開，不會互相污染。
/// 放在 privacy module-level 而不是 `service::State`：非同步 edit session callback 可能在
/// state 借用期間或不同堆疊回來，用全域 Mutex 避免 RefCell 重入，也不必讓 callback 持有
/// TextService 參考。Deactivate／focus 離開時用 [`invalidate`]／[`forget_context`] 作廢。
static FIELD_CACHE: OnceLock<Mutex<FieldCache>> = OnceLock::new();
/// 每次 focus 離開或 Deactivate 就 +1。非同步 callback 帶著 probe 開始時的 generation 回來，
/// 比對不上就丟掉結果，避免舊欄位的結果蓋到新欄位。
static EPOCH: AtomicU64 = AtomicU64::new(0);

const CACHE_LIMIT: usize = 32;
const CACHE_TTL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
struct CachedField {
    field: Field,
    generation: u64,
    expires: Instant,
}

#[derive(Debug)]
struct FieldCache {
    entries: HashMap<usize, CachedField>,
    order: VecDeque<usize>,
}

impl FieldCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }
}

fn field_cache() -> std::sync::MutexGuard<'static, FieldCache> {
    FIELD_CACHE
        .get_or_init(|| Mutex::new(FieldCache::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn epoch() -> u64 {
    EPOCH.load(Ordering::Relaxed)
}

fn context_key(ctx: &ITfContext) -> usize {
    ctx.as_raw() as usize
}

/// 命中回傳該 context 上一次 probe 的欄位；過期、generation 落後或沒有資料回 None。
fn cached_field(key: usize) -> Option<Field> {
    let current = epoch();
    let mut cache = field_cache();
    let stale = match cache.entries.get(&key) {
        Some(entry) if entry.generation == current && entry.expires > Instant::now() => {
            return Some(entry.field)
        }
        _ => true,
    };
    if stale {
        cache.entries.remove(&key);
        cache.order.retain(|k| *k != key);
    }
    None
}

/// probe 明確得到結果時寫入 cache；非同步 callback 回來時 generation 已變就丟掉。
fn cache_field(key: usize, field: Field, generation: u64) {
    if generation != epoch() {
        return;
    }
    let mut cache = field_cache();
    cache.entries.insert(
        key,
        CachedField {
            field,
            generation,
            expires: Instant::now() + CACHE_TTL,
        },
    );
    cache.order.retain(|k| *k != key);
    cache.order.push_back(key);
    while cache.entries.len() > CACHE_LIMIT {
        match cache.order.pop_front() {
            Some(old) => {
                cache.entries.remove(&old);
            }
            None => break,
        }
    }
}

/// 某個 context 的焦點離開：作廢它的 cache 與還在飛的非同步結果。
pub(crate) fn forget_context(ctx: &ITfContext) {
    EPOCH.fetch_add(1, Ordering::Relaxed);
    let key = context_key(ctx);
    let mut cache = field_cache();
    cache.entries.remove(&key);
    cache.order.retain(|k| *k != key);
}

/// 全部作廢（Deactivate）：epoch +1 讓所有還在飛的非同步結果失效，並清空 cache。
pub(crate) fn invalidate() {
    EPOCH.fetch_add(1, Ordering::Relaxed);
    let mut cache = field_cache();
    cache.entries.clear();
    cache.order.clear();
}

/// `ui_less`：本執行緒以 TF_TMAE_UIELEMENTENABLEDONLY 啟用（自行繪製候選的宿主，例如遊戲，#86）。
pub fn field(ctx: Option<&ITfContext>, client: u32, ui_less: bool) -> Field {
    field_at(ctx, client, ui_less, &ProbeContext::default())
}

/// 停用或唯讀的 context 明確 Blocked，不受 cache 影響；順手把結果寫入 cache。
fn disabled_field(ctx: &ITfContext, key: usize) -> Option<Field> {
    let reason = disabled_reason(ctx)?;
    // 程式自己停用了輸入法（例如 ImmAssociateContext(NULL)、唯讀文件）：同其他輸入法不組字。每個行程每種原因記一次（#257）。
    log::once(&format!(
        "欄位政策：程式停用了這個欄位的輸入法（{}），按鍵放行給程式 exe={}",
        reason,
        log::exe_name()
    ));
    cache_field(key, Field::Blocked, epoch());
    Some(Field::Blocked)
}

/// 同 [`field`]，另外帶入呼叫端上下文（[`ProbeContext`]），供 probe 讀不到欄位政策時寫詳細 log。
///
/// 先檢查明確停用的 context（不受 cache 影響），再查 [`FIELD_CACHE`]（key 是 ITfContext，
/// 不同欄位分開），命中就回該 context 上一次的結果。cache miss 才 probe。probe 用
/// `TF_ES_ASYNCDONTCARE`：宿主同步完成時 [`ScopeProbe`] 直接更新 cache；回 `TF_S_ASYNC` 時
/// 按鍵先放行（這裡是 cache miss，回 [`Field::Open`]），稍後 callback 更新 cache。讀不到
/// （宿主拒絕、Com 錯誤、沒有焦點文件）也回 Open，但不寫 cache，下次按鍵再試。
pub fn field_at(
    ctx: Option<&ITfContext>,
    client: u32,
    ui_less: bool,
    context: &ProbeContext,
) -> Field {
    match ctx {
        Some(ctx) => {
            let key = context_key(ctx);
            if let Some(field) = disabled_field(ctx, key) {
                return field;
            }
            cached_field(key).unwrap_or_else(|| probe(ctx, client, ui_less, context, key))
        }
        None => {
            log::once(&format!(
                "欄位政策：沒有焦點文件，按鍵放行給程式 exe={}",
                log::exe_name()
            ));
            Field::Open
        }
    }
}

/// 系統的輸入法開關熱鍵（Ctrl＋Space，#117）能不能切換中英：切換不送按鍵、不讀文件，只在確定擋下的欄位
/// （唯讀、停用、密碼）不切換。宿主正在處理按鍵、不給同步唯讀鎖時（Word、Outlook 收到熱鍵當下回
/// TS_E_SYNCHRONOUS，#164）讀不到欄位政策，照常切換，同輸入指示器按鈕。
pub fn hotkey_allowed(ctx: Option<&ITfContext>, client: u32, ui_less: bool) -> bool {
    ctx.is_some_and(|ctx| {
        field_at(Some(ctx), client, ui_less, &ProbeContext::default()) != Field::Blocked
    })
}

#[cfg(test)]
fn toggle_allowed(probed: Option<Field>) -> bool {
    probed != Some(Field::Blocked)
}

/// 用 edit session 讀 InputScope：密碼、PIN 與 IS_PRIVATE 由 [`ScopeProbe`] 判斷並更新 cache；
/// 停用與唯讀已在 [`field_at`] 的 [`disabled_field`] 先擋下。讀不到回 Open 並用 cache 銜接。
/// `key` 是這個 context 的 cache key，`generation` 是 probe 開始時的 epoch，交給
/// [`ScopeProbe`] 在非同步 callback 回來時驗過期。
fn probe(
    ctx: &ITfContext,
    client: u32,
    ui_less: bool,
    context: &ProbeContext,
    key: usize,
) -> Field {
    unsafe {
        object_created();
        let generation = epoch();
        let probe = ComObject::new(ScopeProbe {
            ctx: ctx.clone(),
            field: Cell::new(Field::Blocked),
            ui_less,
            key,
            generation,
        });
        let edit: ITfEditSession = probe.to_interface();
        match ctx.RequestEditSession(client, &edit, TF_ES_ASYNCDONTCARE | TF_ES_READ) {
            // 宿主選擇非同步：DoEditSession 稍後更新 cache。現在照「讀不到」處理，
            // 用同 context 上次 cache（field_at 已查過，這裡是 miss）或 Open 回答。
            Ok(hr) if hr == TF_S_ASYNC => Field::Open,
            // 同步完成：DoEditSession 已在 host 端跑過並寫入 cache，讀它的結果。
            Ok(hr) if hr.is_ok() => probe.field.get(),
            // 兩條讀不到路徑都是 host 拒絕鎖／Com 錯誤，不是「確認敏感」；不寫 cache，下次再試。
            Ok(hr) => {
                unreadable(hr, client, ui_less, context);
                Field::Open
            }
            Err(error) => {
                unreadable(error.code(), client, ui_less, context);
                Field::Open
            }
        }
    }
}

/// 讀不到欄位政策（宿主拒絕同步唯讀鎖，或讀 InputScope 時出錯）：按鍵放行、熱鍵照常切換（#164）。
/// 記錄完整上下文讓使用者跑一次就能看出是哪個 callback、哪個鍵被拒：
/// HRESULT、callback、vk、client、ui_less、activate_flags。
/// 預設在 tsf.log 每個行程、每種組合記一次（最多 32 則）。
fn unreadable(hr: windows_core::HRESULT, client: u32, ui_less: bool, context: &ProbeContext) {
    let detail = format!(
        "hr={:?} callback={} vk={} client={} ui_less={} activate_flags={:#x}",
        hr,
        if context.callback.is_empty() {
            "?"
        } else {
            context.callback
        },
        context
            .vk
            .map_or_else(|| "-".to_owned(), |vk| vk.to_string()),
        client,
        ui_less,
        context.activate_flags
    );
    log::once(&format!(
        "欄位政策：讀不到（{}），按鍵放行給程式、Ctrl＋Space 照常切換 exe={}",
        detail,
        log::exe_name()
    ));
    zdebug!("field policy unreadable {} exe={}", detail, log::exe_name());
}

fn context_disabled(ctx: &ITfContext) -> bool {
    disabled_reason(ctx).is_some()
}

/// context 停用或唯讀的原因（給 tsf.log）；可以組字時回 None。
fn disabled_reason(ctx: &ITfContext) -> Option<&'static str> {
    unsafe {
        match ctx.GetStatus() {
            Ok(s) if s.dwDynamicFlags & TF_SD_READONLY != 0 => return Some("唯讀"),
            Ok(_) => {}
            Err(_) => return Some("讀不到狀態"),
        }
        if let Ok(mgr) = ctx.cast::<ITfCompartmentMgr>() {
            for (guid, reason) in [
                (GUID_COMPARTMENT_KEYBOARD_DISABLED, "鍵盤停用"),
                (GUID_COMPARTMENT_EMPTYCONTEXT, "空的 context"),
            ] {
                if let Ok(compartment) = mgr.GetCompartment(&guid) {
                    if let Ok(mut value) = compartment.GetValue() {
                        let disabled = value.Anonymous.Anonymous.vt == VT_I4
                            && value.Anonymous.Anonymous.Anonymous.lVal != 0;
                        let _ = VariantClear(&mut value);
                        if disabled {
                            return Some(reason);
                        }
                    }
                }
            }
        }
        None
    }
}

pub fn blocked_in_edit(ctx: &ITfContext, ec: u32, ui_less: bool) -> bool {
    if context_disabled(ctx) {
        return true;
    }
    object_created();
    let probe = ComObject::new(ScopeProbe {
        ctx: ctx.clone(),
        field: Cell::new(Field::Blocked),
        ui_less,
        key: context_key(ctx),
        generation: epoch(),
    });
    let edit: ITfEditSession = probe.to_interface();
    unsafe { edit.DoEditSession(ec).is_err() || probe.field.get() == Field::Blocked }
}

/// 本執行緒目前的焦點視窗：必須是這個 context 的視窗（或其子視窗）、屬於本行程且可用。
fn focused_window(ctx: &ITfContext) -> Option<HWND> {
    unsafe {
        let focus = GetFocus();
        if focus.is_invalid() {
            return None;
        }
        let view = ctx.GetActiveView().and_then(|view| view.GetWnd()).ok()?;
        if view.is_invalid() || (view != focus && !IsChild(view, focus).as_bool()) {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(focus, Some(&mut pid));
        (pid == std::process::id() && IsWindowEnabled(focus).as_bool()).then_some(focus)
    }
}

/// 不讀文字，只檢查本執行緒目前焦點的原生控制項與可編輯文件。
/// Word 等文件編輯器沒有標準 EDIT 類別；InputScope 不可用時以 MSAA 的角色／狀態核對。
fn native_edit_allows(ctx: &ITfContext) -> bool {
    focused_window(ctx)
        .is_some_and(|focus| native_edit_is_plain(focus) || accessible_edit_allows(focus))
}

/// 已知沒有 InputScope、MSAA 在行程內也查不到的文件畫布：OneNote 桌面版（2016、2024、Microsoft 365 是同一套程式）
/// 的頁面 OneNote::DocumentCanvas 讀 InputScope 回 E_FAIL；跨行程看它的 MSAA 是 PANE、焦點物件是 DOCUMENT，
/// 但在 OneNote 行程裡（輸入法所在）AccessibleObjectFromWindow 回 E_FAIL，不論在不在文件鎖裡，
/// 原本的原生／MSAA 檢查都核對不到，整個頁面被當成擋下的欄位，按鍵直接以英文進入、Shift 也切不了（#163）。
/// 頁面裡沒有密碼欄（密碼保護的節由另外的對話框輸入）；context 的停用／唯讀已先檢查，
/// 這裡再確認焦點屬於本 context、本行程且可用，MSAA 查得到時也不能標成保護、唯讀或停用。
fn document_canvas_allows(ctx: &ITfContext) -> bool {
    focused_window(ctx).is_some_and(|focus| {
        window_class(focus).is_some_and(|class| document_canvas(&class))
            && !accessible_secret(focus)
    })
}

fn document_canvas(class: &str) -> bool {
    class == "OneNote::DocumentCanvas"
}

/// 缺 InputScope、也不是上面已知控制項的自訂欄位：IMM32 程式（系統相容層建立的暫時 context，TF_SS_TRANSITORY）、
/// 自行繪製候選的宿主（UI-less，例如英雄聯盟遊戲內聊天，#86），以及自己實作 TSF 文字儲存的程式——Unreal Engine 的
/// UnrealWindow（UE4、UE5 同一份 FTextStoreACP）的 RequestSupportedAttrs 回 S_OK 卻不提供任何屬性，讀 InputScope 回
/// E_FAIL，靜態旗標只有 TS_SS_NOHIDDENTEXT（WARDOGS、《二之國：交錯世界》的聊天框，#209、#210）。原本只放行前兩種，
/// 第三種整個被當成無法判斷而放行給宿主：按鍵直接以英文進入，Shift、Ctrl＋Space 也切不了中英。
/// UE 的密碼框同樣不停用輸入法、也不標 InputScope，其他輸入法（新酷音、小狼毫、PIME 只看停用／空 context）照常組字。
/// 焦點在本 context、本行程且可用，不是密碼／唯讀的標準編輯框，MSAA 也沒標成保護、唯讀或停用時，以 Private
/// 組字（不學習、不讀前文）。宿主在密碼欄等處取消輸入法關聯或改用空 context 時，context_disabled 已先擋下；
/// 標了 IS_PASSWORD 等的欄位讀得到 InputScope，不會走到這裡。
fn custom_field_allows(ctx: &ITfContext) -> bool {
    let Some(focus) = focused_window(ctx) else {
        return false;
    };
    if window_class(focus).is_some_and(|class| native_edit_class(&class)) {
        return native_edit_is_plain(focus);
    }
    !accessible_secret(focus)
}

/// Windows App SDK（WinUI 3）XAML island 的文字框：TextBox 沒設 InputScope 時讀取回 E_FAIL，
/// 焦點視窗是 island 的輸入視窗 InputSiteWindowClass，MSAA 只有沒有子物件的 CLIENT，
/// 原生／MSAA 檢查都核對不到而被擋，按鍵直接以英文進入欄位（23H2 起檔案總管的網址列與搜尋框、
/// 相片的搜尋框，#105）。XAML 的 PasswordBox 預設 InputScope 為 Password，會先由 secret_scope 擋下；
/// 唯讀、停用的文字框已由 context_disabled 擋下。這裡只再確認焦點屬於本 context、本行程且可用。
/// 宿主開了觸控鍵盤的文字預測或自動校正（TS_SS_TKB*，XAML TextBox 的預設）就是一般文字欄位，
/// 照常組字；沒開的（例如路徑或網址欄位關掉了預測）以私人方式組字（不學習、不讀前文）。
fn xaml_island_field(ctx: &ITfContext) -> Option<Field> {
    let class = focused_window(ctx).and_then(window_class)?;
    let flags = unsafe { ctx.GetStatus() }.ok()?.dwStaticFlags;
    island_field(&class, flags)
}

fn island_field(class: &str, static_flags: u32) -> Option<Field> {
    if class != "InputSiteWindowClass" {
        return None;
    }
    Some(
        if static_flags & (TS_SS_TKBPREDICTIONENABLE | TS_SS_TKBAUTOCORRECTENABLE) != 0 {
            Field::Open
        } else {
            Field::Private
        },
    )
}

/// 英文模式沒在組字時，按鍵要不要整個交給程式、不開始英文補字（#210）。
/// IMM32 程式經系統相容層（CUAS）接 TSF，每個視窗都有一個暫時 context，焦點不在文字框時也一樣：
/// Photoshop 的畫布按 B、V 是換工具，英文補字卻把字母收進組字區、跳出候選窗，快捷鍵就沒了（遊戲的 WASD 同理）。
/// IMM32 的文字框（EDIT、RichEdit、Scintilla、PuTTY 的終端機畫面）有焦點時都有系統插入點；沒有插入點就不是在打字。
///
/// 自己實作文字儲存的程式不算，它們給 context 就表示焦點在文字欄位上（非文字焦點是停用或唯讀的 context，由欄位政策處理），
/// 而且常常沒有系統插入點。.14 實測（靜態旗標／啟用旗標）：
/// - 系統相容層（immhost 的畫布與文字框）：0x4（只有 TS_SS_TRANSITORY）／0
/// - Chromium（Edge、Chrome、Electron）：0xc（TS_SS_TRANSITORY＋TS_SS_NOHIDDENTEXT）／0，ui/base/ime/win/tsf_text_store.cc
///   明寫這兩個旗標、tsf_bridge.cc 以沒有旗標的 Activate 啟用；只看暫時 context 會把這些程式的文字框全部當成畫布，
///   英文補字整個消失、Enter 直接送出訊息（#242）。
/// - Windows Terminal：自己呼叫 ActivateEx，啟用旗標不是 0（0x40000040）
///
/// 所以要暫時 context、沒有宣告 TS_SS_NOHIDDENTEXT、由相容層啟用，三者都成立才算 IMM32 程式。
/// 注音模式照舊組字（同其他注音輸入法，要用快捷鍵先切英文）。
pub fn english_passthrough(ctx: &ITfContext, activate_flags: u32) -> bool {
    imm32_without_caret(
        static_flags(ctx),
        activate_flags,
        crate::caret::thread_has_caret(),
    )
}

/// 這個 context 是不是 IMM32 程式經系統相容層（CUAS）建立的（判斷同 [`english_passthrough`]，不看插入點）。
/// 這類程式收到的提交是 WM_IME_COMPOSITION 的結果字串，排在程式自己的按鍵訊息之後處理（#260）。
pub fn imm32_context(ctx: &ITfContext, activate_flags: u32) -> bool {
    imm32_flags(static_flags(ctx), activate_flags)
}

/// 這個程式的文字欄位是哪一種、欄位政策的結果是什麼：每個行程對每種組合（靜態旗標、焦點視窗類別、結果）在 tsf.log
/// 記一次，最多 [`CONTEXT_NOTES`] 種（#257）。只有旗標與視窗類別，不含文字與鍵碼；客服從匯出的狀態摘要就看得出
/// AutoCAD 這類程式是經系統相容層（IMM32）還是自己的 TSF 文字儲存、焦點在哪種視窗、輸入法有沒有被擋下。
pub fn note_context(ctx: Option<&ITfContext>, activate_flags: u32, field: Field) {
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let Some(ctx) = ctx else {
        return;
    };
    let flags = static_flags(ctx);
    let class = window_class(unsafe { GetFocus() }).unwrap_or_default();
    if !first_context(
        &mut SEEN.lock().unwrap_or_else(|e| e.into_inner()),
        format!("{flags:#x} {class} {field:?}"),
    ) {
        return;
    }
    log::log(&format!(
        "文件：{} status={:#x} focus={} field={:?} exe={}",
        if imm32_flags(flags, activate_flags) {
            "IMM32 程式經系統相容層"
        } else {
            "TSF 文字儲存"
        },
        flags,
        class,
        field,
        log::exe_name()
    ));
}

const CONTEXT_NOTES: usize = 8;

fn first_context(seen: &mut Vec<String>, key: String) -> bool {
    if seen.len() >= CONTEXT_NOTES || seen.contains(&key) {
        return false;
    }
    seen.push(key);
    true
}

fn static_flags(ctx: &ITfContext) -> u32 {
    unsafe { ctx.GetStatus() }
        .map(|s| s.dwStaticFlags)
        .unwrap_or(0)
}

fn imm32_flags(static_flags: u32, activate_flags: u32) -> bool {
    static_flags & (TS_SS_TRANSITORY | TS_SS_NOHIDDENTEXT) == TS_SS_TRANSITORY
        && activate_flags == 0
}

fn imm32_without_caret(static_flags: u32, activate_flags: u32, caret: bool) -> bool {
    imm32_flags(static_flags, activate_flags) && !caret
}

/// 除錯紀錄用：缺 InputScope 時記下 context 狀態旗標與焦點／檢視視窗類別（不含文字）。
fn missing_scope_context(ctx: &ITfContext) -> String {
    unsafe {
        let status = ctx
            .GetStatus()
            .map(|s| format!("{:#x}/{:#x}", s.dwStaticFlags, s.dwDynamicFlags))
            .unwrap_or_else(|e| format!("{:?}", e.code()));
        let focus = GetFocus();
        let view = ctx
            .GetActiveView()
            .and_then(|view| view.GetWnd())
            .unwrap_or_default();
        format!(
            "status={} focus={} view={} same={}",
            status,
            window_class(focus).unwrap_or_default(),
            window_class(view).unwrap_or_default(),
            focused_window(ctx).is_some()
        )
    }
}

fn window_class(hwnd: HWND) -> Option<String> {
    let mut name = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut name) };
    (len > 0).then(|| String::from_utf16_lossy(&name[..len as usize]))
}

/// MSAA 標示為保護（密碼）、唯讀或停用。取不到 MSAA（多數自訂視窗）時不算。
fn accessible_secret(hwnd: HWND) -> bool {
    unsafe {
        let mut raw = std::ptr::null_mut();
        if AccessibleObjectFromWindow(hwnd, OBJID_CLIENT.0 as u32, &IAccessible::IID, &mut raw)
            .is_err()
            || raw.is_null()
        {
            return false;
        }
        let accessible = IAccessible::from_raw(raw);
        let Ok(mut state) = accessible.get_accState(&VARIANT::from(0i32)) else {
            return false;
        };
        let secret = state.Anonymous.Anonymous.vt == VT_I4
            && state.Anonymous.Anonymous.Anonymous.lVal as u32
                & (STATE_SYSTEM_READONLY | STATE_SYSTEM_PROTECTED | STATE_SYSTEM_UNAVAILABLE)
                != 0;
        let _ = VariantClear(&mut state);
        secret
    }
}

fn editable_accessible(role: u32, state: u32, word_document: bool) -> bool {
    (matches!(role, ROLE_SYSTEM_DOCUMENT | ROLE_SYSTEM_TEXT)
        || (word_document && role == ROLE_SYSTEM_CLIENT))
        && state & (STATE_SYSTEM_FOCUSED | STATE_SYSTEM_FOCUSABLE)
            == STATE_SYSTEM_FOCUSED | STATE_SYSTEM_FOCUSABLE
        && state & (STATE_SYSTEM_READONLY | STATE_SYSTEM_PROTECTED | STATE_SYSTEM_UNAVAILABLE) == 0
}

fn accessible_edit_allows(hwnd: HWND) -> bool {
    unsafe {
        let mut raw = std::ptr::null_mut();
        if AccessibleObjectFromWindow(hwnd, OBJID_CLIENT.0 as u32, &IAccessible::IID, &mut raw)
            .is_err()
            || raw.is_null()
        {
            return false;
        }
        let accessible = IAccessible::from_raw(raw);
        // Word 2024 的文件 _WwG 在 MSAA 暴露 CLIENT（沒有子物件），
        // 而非 DOCUMENT／TEXT。只對這個已知文件控制項接受 CLIENT；
        // context 的唯讀／停用、焦點、密碼與 MSAA 狀態仍須全部通過。
        let mut class = [0u16; 128];
        let length = GetClassNameW(hwnd, &mut class);
        let word_document =
            length > 0 && class[..length as usize] == "_WwG".encode_utf16().collect::<Vec<_>>();
        // CHILDID_SELF=0；僅讀角色及狀態，不讀 Value／Name／文件文字。
        let child = VARIANT::from(0i32);
        let Ok(mut role) = accessible.get_accRole(&child) else {
            return false;
        };
        let mut state = match accessible.get_accState(&child) {
            Ok(state) => state,
            Err(_) => {
                let _ = VariantClear(&mut role);
                return false;
            }
        };
        let allows = role.Anonymous.Anonymous.vt == VT_I4
            && state.Anonymous.Anonymous.vt == VT_I4
            && editable_accessible(
                role.Anonymous.Anonymous.Anonymous.lVal as u32,
                state.Anonymous.Anonymous.Anonymous.lVal as u32,
                word_document,
            );
        let _ = VariantClear(&mut role);
        let _ = VariantClear(&mut state);
        allows
    }
}

fn native_edit_is_plain(hwnd: HWND) -> bool {
    unsafe {
        let Some(class) = window_class(hwnd) else {
            return false;
        };
        if !native_edit_class(&class) || !IsWindowEnabled(hwnd).as_bool() {
            return false;
        }
        GetWindowLongW(hwnd, GWL_STYLE) & (ES_PASSWORD | ES_READONLY) == 0
            && SendMessageW(hwnd, EM_GETPASSWORDCHAR, Some(WPARAM(0)), Some(LPARAM(0))).0 == 0
    }
}

fn native_edit_class(class: &str) -> bool {
    [
        "Edit",
        "RichEdit",
        "RichEdit20A",
        "RichEdit20W",
        "RICHEDIT50W",
        "RichEditD2DPT",
    ]
    .iter()
    .any(|known| class.eq_ignore_ascii_case(known))
}

fn secret_scope(scope: InputScope) -> bool {
    [
        IS_PASSWORD,
        IS_NUMERIC_PASSWORD,
        IS_NUMERIC_PIN,
        IS_ALPHANUMERIC_PIN,
        IS_ALPHANUMERIC_PIN_SET,
    ]
    .contains(&scope)
}

/// 密碼／PIN 優先於 IS_PRIVATE：無痕視窗裡的密碼欄同時帶兩者，仍要繞過。
fn scope_field(scopes: &[InputScope]) -> Field {
    if scopes.iter().any(|s| secret_scope(*s)) {
        Field::Blocked
    } else if scopes.contains(&IS_PRIVATE) {
        Field::Private
    } else {
        Field::Open
    }
}

#[implement(ITfEditSession)]
struct ScopeProbe {
    ctx: ITfContext,
    field: Cell<Field>,
    ui_less: bool,
    /// 這個 context 在 FIELD_CACHE 的 key；非同步 callback 回來時更新它的結果。
    key: usize,
    /// probe 開始時的 epoch；callback 回來時若已變（focus 換掉／Deactivate）就丟掉結果。
    generation: u64,
}
impl Drop for ScopeProbe {
    fn drop(&mut self) {
        object_destroyed();
    }
}
impl ITfEditSession_Impl for ScopeProbe_Impl {
    fn DoEditSession(&self, ec: u32) -> windows_core::Result<()> {
        // DLL 正在卸載時不要把結果寫回全域 cache；一般情況 probe 已 object_created，計數不為 0。
        if log::LIVE_OBJECTS.load(Ordering::Relaxed) == 0 {
            return Ok(());
        }
        let result = guarded("InputScope", || unsafe {
            // 沒有 scope 屬性的傳統文字框可正常輸入；無法解讀的屬性則阻擋引擎處理。
            let Ok(prop) = self.ctx.GetAppProperty(&GUID_PROP_INPUTSCOPE) else {
                return Ok(Field::Open);
            };
            let mut selected = [TF_SELECTION::default()];
            let mut fetched = 0;
            self.ctx
                .GetSelection(ec, TF_DEFAULT_SELECTION, &mut selected, &mut fetched)?;
            let range = std::mem::ManuallyDrop::take(&mut selected[0].range);
            let Some(range) = range else {
                log::once(&format!(
                    "欄位政策：宿主沒有回報選取範圍，無法判斷，按鍵放行給程式 exe={}",
                    log::exe_name()
                ));
                // 讀不到選取範圍不是確認敏感，放行。
                return Ok(Field::Open);
            };
            let mut value = match prop.GetValue(ec, &range) {
                Ok(value) => value,
                // 部分 RichEdit（含 Windows 11 記事本）沒有 InputScope，回傳 E_FAIL。
                // 僅在焦點確實屬於本 context 且密碼／唯讀狀態可核對時退回原生／MSAA 檢查。
                Err(error) if error.code() == E_FAIL || error.code() == E_NOTIMPL => {
                    let field =
                        if native_edit_allows(&self.ctx) || document_canvas_allows(&self.ctx) {
                            Field::Open
                        } else if let Some(field) = xaml_island_field(&self.ctx) {
                            field
                        } else if custom_field_allows(&self.ctx) {
                            Field::Private
                        } else {
                            Field::Blocked
                        };
                    let detail = format!(
                        "scope={:?} ui_less={} {} field={:?}",
                        error.code(),
                        self.ui_less,
                        missing_scope_context(&self.ctx),
                        field
                    );
                    // 降級成私人組字或放行給宿主時，預設在 tsf.log 記一次（每個行程同樣的情況只記一次，#209）；
                    // 只有欄位類別與旗標，不含文字與鍵碼。
                    if field != Field::Open {
                        log::once(&format!(
                            "欄位政策：讀不到 InputScope，{} exe={} {}",
                            if field == Field::Private {
                                "以私人方式組字"
                            } else {
                                "無法判斷，按鍵放行給程式"
                            },
                            log::exe_name(),
                            detail
                        ));
                    }
                    zdebug!("field policy missing {}", detail);
                    return Ok(field);
                }
                Err(error) => return Err(error),
            };
            let result = (|| -> windows_core::Result<Field> {
                if value.Anonymous.Anonymous.vt != VT_UNKNOWN {
                    return Ok(Field::Open);
                }
                let Some(unknown) = value.Anonymous.Anonymous.Anonymous.punkVal.as_ref() else {
                    return Ok(Field::Open);
                };
                let scope: ITfInputScope = unknown.cast()?;
                let mut scopes = std::ptr::null_mut();
                let mut count = 0;
                scope.GetInputScopes(&mut scopes, &mut count)?;
                let field = if scopes.is_null() {
                    // 宿主回報有 scope 數量卻沒給陣列（count != 0）是 API 回 inconsistent，
                    // 讀不到內容，不是確認敏感；與其他「讀不到」路徑一樣放行。
                    Field::Open
                } else {
                    scope_field(std::slice::from_raw_parts(scopes, count as usize))
                };
                CoTaskMemFree(Some(scopes.cast()));
                if field == Field::Blocked {
                    log::once(&format!(
                        "欄位政策：密碼或 PIN 欄位，按鍵直接交給程式 exe={}",
                        log::exe_name()
                    ));
                }
                Ok(field)
            })();
            let _ = VariantClear(&mut value);
            result
        });
        let field = result?;
        // 同步或非同步都在這裡更新 cache；generation 不符代表 focus 已換或已 Deactivate，丟掉。
        self.field.set(field);
        cache_field(self.key, field, self.generation);
        Ok(())
    }
}

#[cfg(test)]
#[path = "privacy_host_tests.rs"]
pub(crate) mod host_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_notes_once_per_kind_up_to_the_limit() {
        let mut seen = Vec::new();
        assert!(first_context(&mut seen, "0x4 Edit Private".into()));
        assert!(!first_context(&mut seen, "0x4 Edit Private".into()));
        assert!(first_context(&mut seen, "0x4 Edit Open".into()));
        for i in 0..CONTEXT_NOTES {
            first_context(&mut seen, i.to_string());
        }
        assert_eq!(seen.len(), CONTEXT_NOTES);
        assert!(!first_context(
            &mut seen,
            "0xc Chrome_RenderWidgetHostHWND Open".into()
        ));
    }

    #[test]
    fn missing_scope_document_fallback_requires_editable_focused_nonprivate_content() {
        let focused = STATE_SYSTEM_FOCUSED | STATE_SYSTEM_FOCUSABLE;
        assert!(editable_accessible(ROLE_SYSTEM_DOCUMENT, focused, false));
        assert!(editable_accessible(ROLE_SYSTEM_TEXT, focused, false));
        assert!(editable_accessible(ROLE_SYSTEM_CLIENT, focused, true));
        for flag in [
            STATE_SYSTEM_READONLY,
            STATE_SYSTEM_PROTECTED,
            STATE_SYSTEM_UNAVAILABLE,
        ] {
            assert!(!editable_accessible(
                ROLE_SYSTEM_DOCUMENT,
                focused | flag,
                false
            ));
            assert!(!editable_accessible(
                ROLE_SYSTEM_TEXT,
                focused | flag,
                false
            ));
            assert!(!editable_accessible(
                ROLE_SYSTEM_CLIENT,
                focused | flag,
                true
            ));
        }
        assert!(!editable_accessible(
            ROLE_SYSTEM_DOCUMENT,
            STATE_SYSTEM_FOCUSABLE,
            false
        ));
        assert!(!editable_accessible(
            windows::Win32::UI::Accessibility::ROLE_SYSTEM_CLIENT,
            focused,
            false
        ));
    }

    #[test]
    fn english_passthrough_only_for_imm32_contexts_without_caret() {
        // #210：IMM32 程式沒有插入點（Photoshop 的畫布）時英文模式按鍵整個交給程式；有插入點（EDIT、PuTTY）照常補字。
        // 系統相容層的 context 實測只有 TS_SS_TRANSITORY。
        assert!(imm32_without_caret(TS_SS_TRANSITORY, 0, false));
        assert!(!imm32_without_caret(TS_SS_TRANSITORY, 0, true));
        // #242：Chromium（Edge、Chrome、Electron）實測 0xc、啟用旗標 0、沒有系統插入點，文字框要照常補字。
        assert_eq!(TS_SS_TRANSITORY | TS_SS_NOHIDDENTEXT, 0xc);
        assert!(!imm32_without_caret(0xc, 0, false));
        // Windows Terminal：自己接 TSF、沒有系統插入點，但啟用旗標不是 0（.14 實測 0x40000040）。
        assert!(!imm32_without_caret(TS_SS_TRANSITORY, 0x4000_0040, false));
        assert!(!imm32_without_caret(0xc, 0x4000_0040, false));
        // 其他自己實作文字儲存的程式不是暫時 context。
        assert!(!imm32_without_caret(TS_SS_NOHIDDENTEXT, 0, false));
        assert!(!imm32_without_caret(0, 0, false));
        // #260：有插入點的 IMM32 文字框（EDIT）也是系統相容層，英文補字照常但單字的送出方式不同。
        assert!(imm32_flags(TS_SS_TRANSITORY, 0));
        assert!(!imm32_flags(0xc, 0));
        assert!(!imm32_flags(TS_SS_TRANSITORY, 0x4000_0040));
        assert!(!imm32_flags(TS_SS_NOHIDDENTEXT, 0));
    }

    #[test]
    fn onenote_page_canvas_is_a_document() {
        // #163：OneNote 桌面版的頁面（標題與內文）是這個視窗類別；大小寫不同、OneNote 的其他視窗不算。
        assert!(document_canvas("OneNote::DocumentCanvas"));
        for class in [
            "onenote::documentcanvas",
            "Framework::CFrame",
            "OneNote::CFirstRunDialog",
            "NetUIHWND",
            "_WwG",
            "RichEdit20W",
            "",
        ] {
            assert!(!document_canvas(class), "{class}");
        }
    }

    #[test]
    fn hotkey_toggles_unless_the_field_is_known_blocked() {
        // #164：讀不到欄位政策（宿主不給同步鎖）照常切換；確定擋下的欄位不切換。
        assert!(toggle_allowed(None));
        assert!(toggle_allowed(Some(Field::Open)));
        assert!(toggle_allowed(Some(Field::Private)));
        assert!(!toggle_allowed(Some(Field::Blocked)));
    }

    #[test]
    fn native_fallback_rejects_password_readonly_disabled_and_unknown_controls() {
        use windows::Win32::UI::Controls::EM_SETPASSWORDCHAR;
        use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
        use windows::Win32::UI::WindowsAndMessaging::*;
        use windows_core::w;
        unsafe {
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("Edit"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                100,
                20,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            assert!(native_edit_is_plain(edit));
            SendMessageW(
                edit,
                EM_SETPASSWORDCHAR,
                Some(WPARAM('*' as usize)),
                Some(LPARAM(0)),
            );
            assert!(!native_edit_is_plain(edit));
            SendMessageW(edit, EM_SETPASSWORDCHAR, Some(WPARAM(0)), Some(LPARAM(0)));
            let style = GetWindowLongW(edit, GWL_STYLE);
            SetWindowLongW(edit, GWL_STYLE, style | ES_READONLY);
            assert!(!native_edit_is_plain(edit));
            SetWindowLongW(edit, GWL_STYLE, style);
            let _ = EnableWindow(edit, false);
            assert!(!native_edit_is_plain(edit));
            let _ = DestroyWindow(edit);
        }
        assert!(!native_edit_class("Chrome_RenderWidgetHostHWND"));
        assert!(!native_edit_class("Windows.UI.Core.CoreWindow"));
        assert!(!native_edit_class("RichEditPrivateCustom"));
    }

    #[test]
    fn missing_scope_fallback_accepts_only_winui_island_input() {
        // #105：WinUI 3 文字框的焦點視窗；其他 XAML 外框或自訂視窗仍須通過原有檢查。
        // 相片搜尋框實測的靜態旗標是 0x32（REGIONS＋觸控鍵盤預測＋自動校正）。
        assert_eq!(
            island_field("InputSiteWindowClass", 0x32),
            Some(Field::Open)
        );
        assert_eq!(
            island_field("InputSiteWindowClass", TS_SS_TKBPREDICTIONENABLE),
            Some(Field::Open)
        );
        assert_eq!(
            island_field("InputSiteWindowClass", TS_SS_TKBAUTOCORRECTENABLE),
            Some(Field::Open)
        );
        // 宿主關掉預測與校正：仍可組字，但不學習、不讀前文。
        assert_eq!(
            island_field("InputSiteWindowClass", TS_SS_REGIONS),
            Some(Field::Private)
        );
        assert_eq!(
            island_field("InputSiteWindowClass", 0),
            Some(Field::Private)
        );
        for class in [
            "Microsoft.UI.Content.DesktopChildSiteBridge",
            "WinUIDesktopWin32WindowClass",
            "Windows.UI.Core.CoreWindow",
            "Chrome_RenderWidgetHostHWND",
            "inputsitewindowclass",
            "Edit",
        ] {
            assert_eq!(island_field(class, 0x32), None, "{class}");
        }
    }

    #[test]
    fn secret_scopes_bypass_engine() {
        for s in [
            IS_PASSWORD,
            IS_NUMERIC_PASSWORD,
            IS_NUMERIC_PIN,
            IS_ALPHANUMERIC_PIN,
            IS_ALPHANUMERIC_PIN_SET,
        ] {
            assert_eq!(scope_field(&[s]), Field::Blocked);
            // 無痕視窗的密碼欄同時帶 IS_PRIVATE，仍要繞過。
            assert_eq!(scope_field(&[IS_PRIVATE, s]), Field::Blocked);
        }
        for s in [IS_DEFAULT, IS_TEXT, IS_EMAIL_SMTPEMAILADDRESS, IS_NUMBER] {
            assert_eq!(scope_field(&[s]), Field::Open);
        }
        assert_eq!(scope_field(&[]), Field::Open);
    }

    #[test]
    fn private_scope_composes_without_learning() {
        // Chrome 無痕／訪客與 Firefox 私密視窗把每個欄位標成 IS_PRIVATE（#68、#70）。
        assert_eq!(scope_field(&[IS_PRIVATE]), Field::Private);
        assert_eq!(scope_field(&[IS_DEFAULT, IS_PRIVATE]), Field::Private);
        assert_eq!(scope_field(&[IS_PRIVATE, IS_URL]), Field::Private);
    }
}
