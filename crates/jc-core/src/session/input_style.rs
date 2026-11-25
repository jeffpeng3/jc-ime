//! 輸入風格與鍵位：全形、標點寬度、Shift 組合、選字鍵與閒置按鍵。
//!
//! 對照上游 Go：
//! - `core/session/input_style.go`：風格按鍵（`styleKey`）、全形切換、微軟反引號
//!   前導、Shift＋字母。
//! - `core/session/shift_symbol.go`：Shift＋符號打半形。
//! - `core/session/selection_keys.go`：選字鍵、每頁項數與每排欄數。
//! - `core/session/idle_keys.go`：外殼用的閒置按鍵清單。
//! - `core/session/keypolicy.go`：注音按鍵語意表。
//!
//! 風格規則放在 [`InputStyle`]，由 [`SessionConfig::style`] 帶進工作階段；全形與
//! 反引號前導是工作階段狀態，不是存檔設定。規則版本為 0 代表沿用舊行為，
//! [`Session::style_key`] 完全不介入。

use super::*;
use crate::inputstyle::Rules;
use crate::zh::keyboard::{Keyboard, LayoutKind};
use crate::zh::syllable::{class_of, Class, TONE1};

/// 美式鍵盤要按 Shift 才打得出的符號。
///
/// 外殼送來的是按鍵實際產生的字元，核心不必另外知道 Shift 有沒有按著。
const SHIFTED_SYMBOLS: &str = "~!@#$%^&*()_+{}|:\"<>?";

/// `SpaceSelects` 舊設定已停用：空白鍵只插入空格，候選改由 ↓ 開啟。
///
/// `InputStyle::preset` 與 `jc-engine` 套用設定時都會呼叫；只記一次 warning。
pub fn warn_space_selects_ignored() {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        log::warn!("space_selects 已停用：空白鍵只插入空格，候選改由 ↓ 開啟");
    });
}

/// 一次輸入風格設定的集合。
///
/// 上游把 `inputstyle.Rules` 與幾個舊設定分開放在 `Session` 上；這裡把同一組
/// 東西收成一個值，`Rules::default()`（版本 0）代表完全不改變既有鍵位。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputStyle {
    /// 版本化的輸入風格規則。
    pub rules: Rules,
    /// 候選選字鍵；空字串或不合用的值退回預設的 `1234567890`。
    pub selection_keys: String,
    /// Shift＋符號打半形。
    pub shift_symbol_halfwidth: bool,
    /// 中英混打是否開啟；只影響閒置按鍵與風格判斷。
    pub mixed_input: bool,
    /// Emoji 捷徑；`"equals"` 時等號開表情選單。
    pub emoji_shortcut: String,
    /// 空白鍵是否只插入空白、不開候選（對照 `!SpaceSelects`）。
    pub space_inserts_space: bool,
}

impl InputStyle {
    /// 依 `crate::inputstyle` 的預設代號建立風格。
    ///
    /// 代號不存在時回傳 `None`；不認得的平台沿用 Linux 預設。
    pub fn preset(id: &str) -> Option<Self> {
        let preset = crate::inputstyle::get(id)?;
        if preset.behavior.space_selects {
            warn_space_selects_ignored();
        }
        Some(Self {
            rules: preset.rules,
            selection_keys: preset.behavior.selection_keys,
            shift_symbol_halfwidth: false,
            mixed_input: preset.behavior.mixed_input,
            emoji_shortcut: preset.behavior.emoji_shortcut,
            space_inserts_space: true,
        })
    }

    /// 風格規則是否有作用；版本 0 代表沿用舊行為。
    pub fn active(&self) -> bool {
        self.rules.version != 0
    }
}

/// 注音按鍵語意表的一個動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// 不處理，交回一般路徑。
    None,
    /// 插入字元。
    Insert,
    /// 轉換成候選（空白）。
    Convert,
    /// 送出組字結果。
    Commit,
    /// 補全。
    Complete,
    /// 取消組字。
    Cancel,
}

/// 注音 profile 的按鍵語意。
///
/// 對照上游 `ZhuyinKeyPolicy.Handle`：只描述語意，實際狀態仍由
/// [`Session::press`] 的既有路徑處理。`Convert`、`NonConvert`、假名與全半形
/// 切換鍵不在本埠的 [`KeyCode`] 內，因此對應的 `ActionNone` 分支不列出。
pub fn zhuyin_key_action(key: &Key, composing: bool, candidates: bool) -> KeyAction {
    match key.code {
        KeyCode::Space => {
            if composing || candidates {
                KeyAction::Convert
            } else {
                KeyAction::Insert
            }
        }
        KeyCode::Enter => {
            if composing || candidates {
                KeyAction::Commit
            } else {
                KeyAction::None
            }
        }
        KeyCode::Tab => {
            if composing || candidates {
                KeyAction::Complete
            } else {
                KeyAction::None
            }
        }
        KeyCode::Escape => {
            if composing || candidates {
                KeyAction::Cancel
            } else {
                KeyAction::None
            }
        }
        KeyCode::Char(ch) if single_printable(ch) => KeyAction::Insert,
        _ => KeyAction::None,
    }
}

/// 一個可列印字元。
fn single_printable(ch: char) -> bool {
    !ch.is_control()
}

/// 全形英數：空白換全形空白，`!`–`~` 移到 U+FF01–FF5E，其餘原樣。
fn full_width_ascii(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch == ' ' {
                '\u{3000}'
            } else if ('!'..='~').contains(&ch) {
                char::from_u32(ch as u32 + 0xFEE0).unwrap_or(ch)
            } else {
                ch
            }
        })
        .collect()
}

/// 全形標點的半形版本。
///
/// 全形英數區換回 ASCII，「。」換成 `.`；沒有對應半形 ASCII 的（`『』……——`）
/// 退回鍵帽字元 `key`。
fn halfwidth_punct(punct: &str, key: char) -> String {
    if punct == "。" {
        return ".".to_string();
    }
    let mut chars = punct.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        if ('\u{FF01}'..='\u{FF5E}').contains(&ch) {
            return char::from_u32(ch as u32 - 0xFEE0)
                .map(|half| half.to_string())
                .unwrap_or_else(|| key.to_string());
        }
    }
    key.to_string()
}

/// 微軟參考風格直接打半形原字元的鍵。
fn microsoft_raw_punctuation(key: char) -> bool {
    matches!(key, '\\' | '\'' | '"' | '<' | '>' | '?')
}

/// 這一版保留、尚未實作的前導鍵。
///
/// 兩者目前都視為「非表內鍵」：反引號照字面送出，`u`／`b` 照一般注音鍵輸入。
/// 未來實作時改寫 [`prefix_reserved_reason`] 與兩個 reserved 測試即可。
const PREFIX_RESERVED: &[(char, &str)] = &[
    // TODO(prefix-u): ` u 4e00 空白 → 一（Unicode 碼位）
    ('u', "unicode"),
    // TODO(prefix-b): ` b a440 空白 → 一（Big5 碼位）
    ('b', "big5"),
];

/// 保留鍵的 TODO 字串；`None` 代表不是保留鍵。
fn prefix_reserved_reason(key: char) -> Option<&'static str> {
    PREFIX_RESERVED
        .iter()
        .find(|(reserved, _)| *reserved == key)
        .map(|(_, reason)| *reason)
}

/// 微軟注音前導表的單一結果。
fn prefix_single(key: char) -> Option<&'static str> {
    Some(match key {
        ',' => "，",
        '.' => "。",
        '\'' => "、",
        ';' => "；",
        ':' => "：",
        '?' => "？",
        '!' => "！",
        '(' => "（",
        ')' => "）",
        '/' => "／",
        '\\' => "＼",
        '=' => "＝",
        '~' => "～",
        '"' => "＂",
        '@' => "＠",
        '#' => "＃",
        '$' => "＄",
        '%' => "％",
        '^' => "︿",
        '&' => "＆",
        '*' => "＊",
        '_' => "＿",
        '{' => "｛",
        '}' => "｝",
        '|' => "｜",
        '+' => "＋",
        '0' => "０",
        '1' => "１",
        '2' => "２",
        '3' => "３",
        '4' => "４",
        '5' => "５",
        '6' => "６",
        '7' => "７",
        '8' => "８",
        '9' => "９",
        _ => return None,
    })
}

/// 微軟注音前導表的多變體鍵；第一項是預設插入值。
fn prefix_variants(key: char) -> Option<&'static [&'static str]> {
    match key {
        '[' => Some(&["［", "「", "『", "【", "〔", "︹", "︻", "﹁", "﹃", "﹝"]),
        ']' => Some(&["］", "」", "』", "】", "〕", "︺", "︼", "﹂", "﹄", "﹞"]),
        '-' => Some(&["－", "…", "→", "←"]),
        '<' => Some(&["＜", "〈", "《"]),
        '>' => Some(&["＞", "〉", "》"]),
        _ => None,
    }
}

/// 給定已插入的變體，找回整組候選；逐字元候選窗用得到。
pub(super) fn prefix_variants_for_surface(surface: &str) -> Option<&'static [&'static str]> {
    ['[', ']', '-', '<', '>']
        .into_iter()
        .find_map(|key| prefix_variants(key).filter(|variants| variants.contains(&surface)))
}

/// 微軟參考風格反引號前導打出的全形標點。
fn microsoft_prefix_punctuation(key: char) -> Option<&'static str> {
    match key {
        '\\' => Some("＼"),
        '\'' => Some("、"),
        '"' => Some("＂"),
        '<' => Some("＜"),
        '>' => Some("＞"),
        '?' => Some("？"),
        _ => None,
    }
}

/// Ctrl＋標點打出的全形標點。
///
/// 對照上游 `proto.CtrlPunct`；只涵蓋本埠 `press_ctrl` 認得的鍵與反引號前導
/// 會用到的鍵。
fn ctrl_punct(key: char) -> Option<&'static str> {
    match key {
        ',' => Some("，"),
        '.' => Some("。"),
        '\'' => Some("、"),
        ';' => Some("；"),
        '?' => Some("？"),
        '!' => Some("！"),
        ':' => Some("："),
        '(' => Some("（"),
        ')' => Some("）"),
        _ => None,
    }
}

/// 沒在組字、注音模式時輸入法會處理的可見 ASCII 按鍵（0x21–0x7e，不含空白）。
///
/// 對照上游 `IdleKeys`：注音鍵、標點、反引號符號選單、等號 Emoji 選單，以及
/// 沒開混打時的聲調鍵；中英混打時照混打規則。外殼據此決定哪些鍵要先送核心。
pub fn idle_keys(keyboard: Keyboard, mixed_input: bool, emoji_shortcut: &str) -> String {
    let mixed = mixed_input && keyboard.kind() == LayoutKind::Standard;
    let equals_phonetic = keyboard.symbol('=').is_some();
    let mut out = String::new();
    for code in 0x21u32..=0x7e {
        let Some(ch) = char::from_u32(code) else {
            continue;
        };
        let symbol = keyboard.symbol(ch);
        let punct = keyboard.punct(ch).is_some();
        let keep = ch == '`'
            || punct
            || (ch == '=' && emoji_shortcut == "equals" && !equals_phonetic)
            || mixed
            || symbol.is_some_and(|sym| class_of(sym) != Class::Tone || sym != TONE1);
        if keep {
            out.push(ch);
        }
    }
    out
}

/// 全形（Shift＋空白切的字寬）時輸入法會處理的可見 ASCII 按鍵。
///
/// 對照上游 `FullWidthIdleKeys`：全形時不是注音鍵也不是標點的鍵，以及大寫字母，
/// 都先送核心；注音鍵與標點照半形規則。
pub fn full_width_idle_keys(keyboard: Keyboard, mixed_input: bool, emoji_shortcut: &str) -> String {
    let idle = idle_keys(keyboard, mixed_input, emoji_shortcut);
    let mut out = String::new();
    for code in 0x21u32..=0x7e {
        let Some(ch) = char::from_u32(code) else {
            continue;
        };
        let phonetic = keyboard.symbol(ch).is_some();
        let punct = keyboard.punct(ch).is_some();
        if ch.is_ascii_uppercase() || (!phonetic && !punct) || idle.contains(ch) {
            out.push(ch);
        }
    }
    out
}

/// 設定「Shift＋符號打半形」開啟時，會改打半形、沒在組字時交還應用程式的按鍵。
///
/// 對照上游 `HalfwidthShiftKeys`：半形就是鍵帽字元的那些；外殼從
/// [`idle_keys`] 拿掉它們，才不會吃掉之後又沒處理。
pub fn halfwidth_shift_keys(keyboard: Keyboard) -> String {
    let mut out = String::new();
    for ch in SHIFTED_SYMBOLS.chars() {
        if let Some(punct) = keyboard.punct(ch) {
            if halfwidth_punct(punct, ch) == ch.to_string() {
                out.push(ch);
            }
        }
    }
    out
}

impl Session {
    /// 目前是否在注音組字中。
    ///
    /// 混打與英文模式的「組字」由各自的路徑判斷，這裡只描述注音狀態。
    pub(super) fn composing(&self) -> bool {
        self.in_zh_lane() && (!self.zh.pending().is_empty() || !self.syllables.is_empty())
    }

    /// 候選一頁幾項。
    ///
    /// 固定 [`PAGE_SIZE`]（9），與 Windows 候選窗一致。`InputStyle` 的
    /// `candidate_page_size` 只保留給舊設定檔讀取，不再參與分頁；頁面大小
    /// 只有這一個來源。
    pub fn page_size(&self) -> usize {
        PAGE_SIZE
    }

    /// 目前使用的選字鍵。
    ///
    /// 對照上游 `selectionKeys`：每個鍵是不重複的可見 ASCII（不含大寫），
    /// 長度要符合頁面大小。舊設定可能是 10 個鍵，第一個字元起取一頁；不合用
    /// 時退回預設的 `123456789`。
    pub fn selection_keys(&self) -> String {
        let page = self.page_size();
        let configured: Vec<char> = self.style.selection_keys.chars().collect();
        // 舊版 10 鍵設定（`1234567890`）仍可讀，只取前 9 個。
        let usable_len = configured.len() == page || configured.len() == page + 1;
        if !usable_len {
            return default_selection_keys(page);
        }
        for (index, ch) in configured.iter().enumerate() {
            if *ch <= ' ' || *ch > '~' || ch.is_ascii_uppercase() {
                return default_selection_keys(page);
            }
            if configured[..index].contains(ch) {
                return default_selection_keys(page);
            }
        }
        configured.into_iter().take(page).collect()
    }

    /// 多排候選每排最多幾項。
    ///
    /// 對照上游 `selectionKeyColumns`：`1234qweras` 依鍵盤三排分成 4 欄，其餘
    /// 維持 5 欄。判斷看原始設定字串；`selection_keys()` 會依頁面大小截斷，
    /// 不能拿截斷後的字串比對。
    pub fn selection_key_columns(&self) -> usize {
        if self.style.selection_keys == "1234qweras" {
            4
        } else {
            5
        }
    }

    /// 按鍵選的是這一頁的第幾項（0 起算）；不是選字鍵時回傳 `None`。
    ///
    /// 選字鍵優先；數字 1–0 不論設定都照位置選字，只顯示數字的外殼也能用。
    pub(super) fn selection_index(&self, ch: char) -> Option<usize> {
        if let Some(index) = self.selection_keys().chars().position(|key| key == ch) {
            return Some(index);
        }
        if let Some(digit) = ch.to_digit(10) {
            let index = (digit as usize + 9) % 10;
            if index < self.page_size() {
                return Some(index);
            }
        }
        None
    }

    /// 目前是否為全形。
    pub fn full_width(&self) -> bool {
        self.full_width
    }

    /// 全形英數轉換；依標點風格換引號與括號。
    ///
    /// 對照上游 `fullWidthText`：微軟風格的 `'` 是 `’`；新酷音風格的
    /// `" ' [ ]` 換成 `” ’ 〔 〕`，其餘照全形英數表。
    fn full_width_text(&self, text: &str) -> String {
        if let Some(ch) = text.chars().next() {
            if text.chars().count() == 1 {
                match self.style.rules.punctuation.as_str() {
                    "microsoft" if ch == '\'' => return "’".to_string(),
                    "chewing" => {
                        let mapped = match ch {
                            '"' => Some("”"),
                            '\'' => Some("’"),
                            '[' => Some("〔"),
                            ']' => Some("〕"),
                            _ => None,
                        };
                        if let Some(mapped) = mapped {
                            return mapped.to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        full_width_ascii(text)
    }

    /// 把字面文字插進目前組字結果；先用掉混打待定英文，維持顯示順序。
    fn insert_style_literal(&mut self, text: &str) {
        if self.config.mode == Mode::Mixed {
            self.ascii.flush();
        }
        self.push_literal(text);
    }

    /// 移除反引號前導留在組字區的標記。
    fn clear_prefix_mark(&mut self) {
        if self.literal.ends_with('`') {
            self.literal.pop();
            return;
        }
        let typed = self.ascii.typed().to_string();
        if let Some(stripped) = typed.strip_suffix('`') {
            let stripped = stripped.to_string();
            self.ascii.replay(&stripped, &self.lexicon);
            self.literal_spans.clear();
        }
    }

    /// 開始反引號前導；中文組字要先交回混打管線，符號才會落在同一條流上。
    pub(super) fn start_symbol_prefix(&mut self) {
        self.candidate_window = false;
        self.candidate_expanded = false;
        self.navigating = false;
        self.keep_candidates = false;
        self.edit = None;
        self.highlighted = 0;
        if self.in_zh_lane() {
            let _ = self.flush_zh_lane_for_mixed();
            self.ascii.seed_literal("`");
            self.mixed_cursor = Self::MIXED_CURSOR_END;
            self.rebuild();
        } else if self.in_ja_lane() {
            self.literal_tail.push('`');
        } else {
            self.ascii.seed_literal("`");
            self.mixed_cursor = Self::MIXED_CURSOR_END;
            self.rebuild();
        }
        self.symbol_prefix = true;
    }

    /// 把前導打出的全形字接進按鍵流。
    fn append_prefix_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.literal_tail.is_empty() {
            self.ascii.seed_literal(text);
            self.mixed_cursor = Self::MIXED_CURSOR_END;
            self.rebuild();
        } else {
            self.literal_tail.push_str(text);
        }
    }

    /// 前導表的多變體鍵：先插入預設值，再開候選窗列出其他變體。
    ///
    /// 游標停在插入的符號右側；此時方向鍵仍可離開，按 ← 回到符號前再 ↓
    /// 就會重新打開同一組候選。
    fn open_prefix_variants(&mut self, default: &str, variants: &'static [&'static str]) {
        self.append_prefix_text(default);
        let text = self.base_with_highlight();
        let runes = text.chars().count();
        if runes == 0 {
            return;
        }
        self.mixed_cursor = runes - 1;
        let Some(target) = self.mixed_target_at_cursor() else {
            self.mixed_cursor = runes;
            return;
        };
        let mut candidates = Vec::new();
        let mut choices = Vec::new();
        for surface in variants {
            candidates.push(Candidate::new(
                (*surface).to_string(),
                (*surface).to_string(),
                0,
                CandidateSource::Literal,
            ));
            choices.push(Some(MixedChoice {
                insert: (*surface).to_string(),
                leftover: String::new(),
                force_literal: false,
                cursor: ChoiceCursor::AfterInsert,
                syllable_edit: None,
            }));
        }
        self.candidates = candidates;
        self.mixed_choices = choices;
        self.mixed_target = Some(target);
        self.highlighted = 0;
        self.candidate_window = true;
        self.candidate_expanded = false;
        self.navigating = false;
        // 游標在符號右側。
        self.mixed_cursor = runes;
    }

    /// 前導狀態下的下一個按鍵；回傳是否已處理。
    pub(super) fn prefix_key(&mut self, key: &Key) -> bool {
        if matches!(key.code, KeyCode::Escape | KeyCode::Backspace) {
            self.symbol_prefix = false;
            self.clear_prefix_mark();
            return true;
        }
        let KeyCode::Char(ch) = key.code else {
            // 空白、Enter 與具名鍵：反引號照字面送出，按鍵照常處理。
            self.symbol_prefix = false;
            return false;
        };
        let ch = key.text.unwrap_or(ch);
        self.symbol_prefix = false;
        if ch == '`' {
            self.clear_prefix_mark();
            self.append_prefix_text("‵");
            return true;
        }
        if let Some(variants) = prefix_variants(ch) {
            self.clear_prefix_mark();
            let default = variants.first().copied().unwrap_or("");
            self.open_prefix_variants(default, variants);
            return true;
        }
        if let Some(punct) = prefix_single(ch) {
            self.clear_prefix_mark();
            self.append_prefix_text(punct);
            return true;
        }
        if prefix_reserved_reason(ch).is_some() {
            // TODO(prefix-u)／TODO(prefix-b)：保留語法尚未實作，照非表內鍵
            // 處理，反引號與按鍵都留在字面。
            return false;
        }
        // 非表內鍵：反引號留在組字區，這個鍵照常處理。
        false
    }

    /// 風格要求的立即送出：先清空組字，下一次 [`Session::commit`] 取走。
    fn commit_style_text(&mut self, text: String) {
        let text = self.finalize(text);
        if !text.is_empty() {
            self.pending_commit = Some(text);
        }
        self.clear_composition();
    }

    /// 全形時的一般字元：不是注音鍵也不是標點的可見 ASCII 換成全形。
    fn full_width_char(&mut self, ch: char) -> bool {
        if !('\u{21}'..='\u{7e}').contains(&ch) {
            return false;
        }
        let keyboard = self.zh.keyboard();
        if keyboard.symbol(ch).is_some() || keyboard.punct(ch).is_some() {
            return false;
        }
        let wide = self.full_width_text(&ch.to_string());
        if self.composing() {
            self.insert_style_literal(&wide);
        } else {
            self.commit_style_text(wide);
        }
        true
    }

    /// Shift＋符號打半形的條件。
    fn halfwidth_shift_active(&self, ch: char) -> bool {
        self.style.shift_symbol_halfwidth
            && !self.full_width
            && SHIFTED_SYMBOLS.contains(ch)
            && !(self.style.rules.punctuation == "microsoft" && microsoft_raw_punctuation(ch))
            && self.zh.keyboard().punct(ch).is_some()
    }

    /// Shift＋空白：依風格切換字寬、翻頁、插入空白或開候選。
    fn style_shift_space(&mut self) {
        match self.style.rules.shift_space.as_str() {
            "fullwidth" => self.full_width = !self.full_width,
            "space" => {
                if self.candidate_window && !self.candidates.is_empty() {
                    let size = self.page_size();
                    let pages = self.candidates.len().div_ceil(size).max(1);
                    let page = self.highlighted / size;
                    self.highlighted = ((page + 1) % pages) * size;
                } else if !self.composing() {
                    self.commit_style_text(" ".to_string());
                } else {
                    self.insert_style_literal(" ");
                }
            }
            "single" if self.composing() => {
                self.candidate_window = true;
                self.highlighted = 0;
            }
            _ => {}
        }
    }

    /// 反引號前導中的按鍵。
    ///
    /// 對照上游 `styleKey` 的 `symbolPrefix` 分支：單一字元依序查微軟前導表、
    /// Ctrl 標點表、排列標點，最後打全形；Esc／退格只取消前導，不留字元。
    fn symbol_prefix_key(&mut self, key: &Key) -> bool {
        let ch = match key.code {
            KeyCode::Char(ch) => Some(key.text.unwrap_or(ch)),
            _ => None,
        };
        let Some(ch) = ch else {
            if matches!(key.code, KeyCode::Escape | KeyCode::Backspace) {
                self.symbol_prefix = false;
                self.clear_prefix_mark();
                return true;
            }
            // 具名鍵不當成可列印字元；反引號留在組字區（送出時保留），
            // 一般路徑（Enter、方向鍵）接手。
            self.symbol_prefix = false;
            return false;
        };
        self.symbol_prefix = false;
        self.clear_prefix_mark();
        if let Some(punct) = microsoft_prefix_punctuation(ch) {
            self.insert_style_literal(punct);
            return true;
        }
        if let Some(punct) = ctrl_punct(ch) {
            self.insert_style_literal(punct);
            return true;
        }
        if let Some(punct) = self.zh.keyboard().punct(ch) {
            self.insert_style_literal(punct);
            return true;
        }
        if single_printable(ch) {
            let text = self.full_width_text(&ch.to_string());
            self.insert_style_literal(&text);
            return true;
        }
        true
    }

    /// 風格按鍵；回傳是否已處理。
    ///
    /// [`Session::press`] 在一般按鍵路徑之前呼叫。風格版本為 0 時直接回
    /// `false`，一行行為都不改。
    pub(super) fn style_key(&mut self, key: &Key) -> bool {
        if !self.style.active() {
            return false;
        }
        if key.code == KeyCode::Space && key.shift {
            self.style_shift_space();
            return true;
        }
        if self.symbol_prefix {
            return self.symbol_prefix_key(key);
        }
        // 微軟風格的半形原字元在候選窗開啟時交由候選處理。
        if self.candidate_window {
            return false;
        }
        if self.full_width && key.code == KeyCode::Space && self.in_zh_lane() {
            let wide = "\u{3000}".to_string();
            if self.composing() {
                self.insert_style_literal(&wide);
            } else {
                self.commit_style_text(wide);
            }
            return true;
        }
        // 微軟風格：反引號是標點前導，不開符號選單。
        if self.style.rules.punctuation == "microsoft" && key.code == KeyCode::Char('`') {
            self.insert_style_literal("`");
            self.symbol_prefix = true;
            return true;
        }
        let ch = match key.code {
            KeyCode::Char(ch) => key.text.unwrap_or(ch),
            _ => return false,
        };
        // 微軟風格：半形原字元。
        if self.style.rules.punctuation == "microsoft" && microsoft_raw_punctuation(ch) {
            if self.full_width {
                let wide = self.full_width_text(&ch.to_string());
                self.insert_style_literal(&wide);
                return true;
            }
            if !self.composing() {
                // 上游把閒置的半形原字元交還應用程式；本埠直接送出同一個字元。
                self.commit_style_text(ch.to_string());
                return true;
            }
            // 未完成的注音被半形原字元取消；已轉換的字保留原本順序。
            if !self.zh.pending().is_empty() {
                self.zh.reset();
                self.zh_pending_keys.clear();
                if !self.composing() {
                    self.commit_style_text(ch.to_string());
                    return true;
                }
            }
            self.insert_style_literal(&ch.to_string());
            return true;
        }
        // Shift＋符號打半形。
        if self.halfwidth_shift_active(ch) {
            let punct = self.zh.keyboard().punct(ch).unwrap_or("");
            let half = halfwidth_punct(punct, ch);
            if self.composing() {
                self.insert_style_literal(&half);
            } else {
                // 上游把「半形就是鍵帽字元」的鍵交還應用程式；本埠沒有 handled
                // 旗標，改成直接送出同一個字元，效果等於應用程式收到它。
                self.commit_style_text(half);
            }
            return true;
        }
        // Shift＋字母。
        if ch.is_ascii_uppercase() {
            match self.style.rules.shift_letter.as_str() {
                "commit" => {
                    let text = format!("{}{}", self.current_text(), ch);
                    self.commit_style_text(text);
                    return true;
                }
                "lower" => {
                    let mut text = ch.to_ascii_lowercase().to_string();
                    if self.full_width {
                        text = self.full_width_text(&text);
                    }
                    if self.composing() {
                        self.insert_style_literal(&text);
                    } else {
                        self.commit_style_text(text);
                    }
                    return true;
                }
                _ => {}
            }
        }
        if self.full_width && self.full_width_char(ch) {
            return true;
        }
        // 標準與新酷音風格的 `\` 一律打全形。
        if matches!(
            self.style.rules.punctuation.as_str(),
            "standard" | "chewing"
        ) && ch == '\\'
        {
            self.insert_style_literal("＼");
            return true;
        }
        false
    }
}

/// 預設選字鍵的前 `page` 個字元。
fn default_selection_keys(page: usize) -> String {
    "1234567890".chars().take(page).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::LexEntry;
    use crate::zh::keyboard::Keyboard;
    use crate::zh::syllable::parse_reading;

    fn lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        for (surface, reading, cost) in [
            ("我", "ㄨㄛˇ", 0),
            ("想", "ㄒㄧㄤˇ", 0),
            ("吃", "ㄔ", 0),
            ("景", "ㄐㄧㄥˇ", 0),
            // 同音詞成本要高於「景」，否則解碼器會用字面順序挑到「井」。
            ("井", "ㄐㄧㄥˇ", 10),
            ("風", "ㄈㄥ", 0),
        ] {
            lex.push(LexEntry::new(surface, reading, cost, 0));
        }
        lex
    }

    fn style_session(id: &str) -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.style = InputStyle::preset(id).expect("預設存在");
        Session::new(config)
    }

    fn plain_session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        Session::new(config)
    }

    fn type_syllable(session: &mut Session, reading: &str) {
        let keyboard = Keyboard::standard();
        for syllable in parse_reading(reading) {
            let keys = keyboard.keys(&syllable).expect("音節有按鍵");
            for ch in keys.chars() {
                session.press(Key::char(ch, false));
            }
        }
    }

    fn type_keys(session: &mut Session, keys: &str) {
        for ch in keys.chars() {
            session.press(Key::char(ch, false));
        }
    }

    fn enter(session: &mut Session) -> String {
        session.press(Key::new(KeyCode::Enter, None, false));
        session.commit()
    }

    fn many_session(selection_keys: &str) -> Session {
        let mut lex = InMemoryLexicon::new();
        for (index, surface) in "是事市試式世室士視示適勢誓釋".chars().enumerate() {
            lex.push(LexEntry::new(
                surface.to_string(),
                "ㄕˋ",
                index as i32 * 10,
                0,
            ));
        }
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lex));
        config.max_candidates = 20;
        config.style = InputStyle {
            selection_keys: selection_keys.to_string(),
            ..InputStyle::default()
        };
        Session::new(config)
    }

    /// 對照 `TestOriginalStylePreservesLegacySequences`：0c 風格只處理明確差異，
    /// 一般組字序列與零規則逐鍵相同。
    #[test]
    fn test_original_style_preserves_legacy_sequences() {
        for keys in ["ji3< ", "wu/3", "`=", "ru/3"] {
            let mut styled = style_session("0c");
            let mut zero = plain_session();
            for ch in keys.chars() {
                let key = Key::char(ch, false);
                let a = styled.press(key);
                let b = zero.press(key);
                assert_eq!(a.preedit, b.preedit, "0c 在 {ch:?} 改變了組字");
                assert_eq!(
                    a.candidates.len(),
                    b.candidates.len(),
                    "0c 在 {ch:?} 改變候選"
                );
            }
        }
    }

    /// 全形是工作階段狀態，混合組字中的英數字也吃全形。
    ///
    /// 英文模式已移到外殼；核心只剩混打，因此等價測試改成在組字中打入
    /// 全形英數字，確認它接在同一段組字後面。
    #[test]
    fn test_microsoft_width_is_state_and_includes_english() {
        let mut s = style_session("microsoft");
        s.press(Key::new(KeyCode::Space, None, true));
        assert!(s.full_width(), "Shift＋空白切全形");
        type_syllable(&mut s, "ㄨㄛˇ");
        for (key, want) in [('@', "我＠"), ('#', "我＠＃")] {
            let snap = s.press(Key::char(key, false));
            assert_eq!(snap.preedit, want, "{key:?}");
        }
        s.press(Key::new(KeyCode::Space, None, false));
        assert_eq!(enter(&mut s), "我＠＃\u{3000}", "全形空白也留在組字裡");
        s.press(Key::new(KeyCode::Space, None, true));
        assert!(!s.full_width(), "再按一次切回半形");
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::Space, None, true));
        assert_eq!(snap.preedit, "我", "切字寬不動組字");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我");
    }

    /// 對照 `TestMicrosoftPrefixAndInlineUp`：反引號前導，下一個標點打全形。
    #[test]
    fn test_microsoft_prefix_inserts_fullwidth_punctuation() {
        let mut s = style_session("microsoft");
        let snap = s.press(Key::char('`', false));
        assert_eq!(snap.preedit, "`", "前導本身顯示在組字區");
        let snap = s.press(Key::char(',', false));
        assert_eq!(snap.preedit, "，");
        assert_eq!(enter(&mut s), "，");
    }

    /// 對照 `TestMicrosoftPrefixCancellationPreservesComposition`：Esc 只取消前導，
    /// Enter 保留畫面中的反引號。
    #[test]
    fn test_microsoft_prefix_cancellation_preserves_composition() {
        let mut s = style_session("microsoft");
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::char('`', false));
        let snap = s.press(Key::new(KeyCode::Escape, None, false));
        assert_eq!(snap.preedit, "我", "Esc 只取消前導");

        s.press(Key::char('`', false));
        assert_eq!(enter(&mut s), "我`", "Enter 保留反引號");

        // 前導只作用一次：送出行為後 `,` 回到字面標點。
        let snap = s.press(Key::char(',', false));
        assert_eq!(snap.preedit, ",", "前導用完就消失");
    }

    /// 對照 `TestMicrosoftObservedRawAndPrefixPunctuation` 的前導表。
    #[test]
    fn test_microsoft_observed_prefix_punctuation() {
        for (key, prefix) in [
            ('\\', "＼"),
            ('\'', "、"),
            ('"', "＂"),
            ('<', "＜"),
            ('>', "＞"),
            ('?', "？"),
        ] {
            let mut s = style_session("microsoft");
            s.press(Key::char('`', false));
            let snap = s.press(Key::char(key, false));
            assert_eq!(snap.preedit, prefix, "{key:?} 的前導");
        }
    }

    /// 對照 `TestMicrosoftRawPunctuationInComposition`：半形原字元取消未完成的
    /// 注音；已完成的字照舊留在組字區。
    #[test]
    fn test_microsoft_raw_punctuation_in_composition() {
        for raw in ['\\', '\'', '<'] {
            let mut s = style_session("microsoft");
            type_keys(&mut s, "su");
            let snap = s.press(Key::char(raw, false));
            assert_eq!(snap.preedit, "", "未完成注音被 {raw:?} 取消");
            assert_eq!(s.commit(), raw.to_string(), "取消後直接送出 {raw:?}");

            type_syllable(&mut s, "ㄨㄛˇ");
            let snap = s.press(Key::char(raw, false));
            assert_eq!(snap.preedit, format!("我{raw}"), "完成的字留在組字區");
            let snap = s.press(Key::new(KeyCode::Escape, None, false));
            assert_eq!(snap.preedit, "", "Esc 清掉組字");
        }
    }

    /// 對照 `TestStyleLiteralPreservesMixedRunAndShiftCommit` 的 Shift 送出。
    #[test]
    fn test_shift_letter_commit_appends_to_composition() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(std::sync::Arc::new(lexicon()));
        config.style = InputStyle::preset("mcbopomofo").expect("預設存在");
        let mut s = Session::new(config);
        type_keys(&mut s, "hello");
        assert_eq!(s.snapshot().preedit, "hello", "混打的待定英文");
        let snap = s.press(Key::char('A', false));
        assert_eq!(snap.preedit, "", "送出後不留組字");
        assert_eq!(s.commit(), "helloA");
    }

    /// 對照 `TestMcBopomofoCycleAndShiftSpace` 的空白與全形標點。
    #[test]
    fn test_mcbopomofo_space_and_backslash() {
        let mut s = style_session("mcbopomofo");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let snap = s.press(Key::new(KeyCode::Space, None, true));
        assert_eq!(snap.preedit, "景 ");
        assert!(!s.full_width(), "mcbopomofo 的 Shift＋空白不切字寬");

        let snap = s.press(Key::char('\\', false));
        assert_eq!(snap.preedit, "景 ＼");
        assert_eq!(enter(&mut s), "景 ＼");
    }

    /// 對照 `TestChewingWidthAndHorizontalNavigation` 的全形表。
    ///
    /// 英文模式已移除；等價測試在混合組字中打同一個鍵，確認新酷音的全形
    /// 對照表仍套用到字面。
    #[test]
    fn test_chewing_english_width_table() {
        let mut s = style_session("chewing");
        s.press(Key::new(KeyCode::Space, None, true));
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::char('@', false));
        assert_eq!(snap.preedit, "我＠", "新酷音的全形表");
        assert_eq!(enter(&mut s), "我＠");
    }

    /// 一般標點與 Shift 符號照字面；「Shift 符號打半形」不再改變結果。
    #[test]
    fn test_shift_symbol_halfwidth_off_keeps_fullwidth() {
        let mut s = plain_session();
        let snap = s.press(Key::char('<', false));
        assert_eq!(snap.preedit, "<");
    }

    /// 對照 `TestShiftSymbolHalfwidthTypesTheKey`：開啟後 Shift 符號打半形，
    /// 組字中插在目前結果後面。
    #[test]
    fn test_shift_symbol_halfwidth_types_the_key() {
        for (key, half) in [
            ('<', ","),
            ('>', "."),
            ('?', "?"),
            (':', ":"),
            ('"', ";"),
            ('!', "!"),
            ('~', "~"),
            ('{', "{"),
            ('}', "}"),
            ('(', "("),
            (')', ")"),
            ('&', "&"),
            ('^', "^"),
            ('_', "_"),
        ] {
            let mut s = style_session("0c");
            s.style.shift_symbol_halfwidth = true;
            let snap = s.press(Key::char(key, false));
            assert_eq!(snap.preedit, "", "沒在組字時 {key:?} 直接送出");
            assert_eq!(s.commit(), half, "沒在組字時 {key:?}");

            type_syllable(&mut s, "ㄨㄛˇ");
            let snap = s.press(Key::char(key, false));
            assert_eq!(snap.preedit, format!("我{half}"), "組字中 {key:?}");
            type_syllable(&mut s, "ㄒㄧㄤˇ");
            assert_eq!(enter(&mut s), format!("我{half}想"), "{key:?} 之後繼續打字");
        }
    }

    /// 未按 Shift 的 `[ ] ' \` 也照字面；全形改由反引號前導打。
    #[test]
    fn test_shift_symbol_halfwidth_unshifted_keys_stay_fullwidth() {
        for key in ['[', ']', '\'', '\\'] {
            let mut s = style_session("0c");
            s.style.shift_symbol_halfwidth = true;
            let snap = s.press(Key::char(key, false));
            assert_eq!(snap.preedit, key.to_string(), "{key:?} 照字面");
        }
    }

    /// 微軟風格的半形原字元鍵不受「Shift 符號打半形」影響。
    #[test]
    fn test_shift_symbol_halfwidth_keeps_microsoft_raw_keys() {
        for key in ['<', '>', '"', '?', '\\', '\''] {
            let mut off = style_session("microsoft");
            let mut on = style_session("microsoft");
            on.style.shift_symbol_halfwidth = true;
            for s in [&mut off, &mut on] {
                type_syllable(s, "ㄨㄛˇ");
                s.press(Key::char(key, false));
            }
            assert_eq!(off.snapshot().preedit, on.snapshot().preedit, "{key:?}");
        }
    }

    /// 對照 `TestShiftSymbolHalfwidthIdleKeysMatchSession` 的靜態清單部分。
    #[test]
    fn test_halfwidth_shift_keys_lists_keys_that_are_already_halfwidth() {
        let listed = halfwidth_shift_keys(Keyboard::standard());
        for key in SHIFTED_SYMBOLS.chars() {
            let half = Keyboard::standard()
                .punct(key)
                .map(|punct| halfwidth_punct(punct, key));
            assert_eq!(
                listed.contains(key),
                half.as_deref() == Some(key.to_string().as_str()),
                "{key:?} 的清單判斷"
            );
        }
        assert!(
            !listed.contains(
                ['[', ']', '\'', ',', '.', ';', '\\', '@', '#', '<', '>', '"'].as_slice()
            ),
            "未按 Shift 或本來就半形的鍵不算：{listed:?}"
        );
    }

    /// 對照 `TestSelectionKeysDefaultLabelsDigits`：不合用的設定退回 1–9。
    #[test]
    fn test_selection_keys_default_to_digits() {
        for keys in [
            "",
            "1234567890",
            "abc",
            "1234qwera",
            "1123qweras",
            "1234QWERAS",
            "ㄅ234qweras",
        ] {
            let mut s = many_session(keys);
            type_syllable(&mut s, "ㄕˋ");
            s.press(Key::new(KeyCode::Down, None, false));
            let candidates = s.candidates().to_vec();
            s.press(Key::char('9', false));
            assert_eq!(
                s.commit(),
                candidates[8].surface,
                "SelectionKeys={keys:?} 的 9"
            );
        }
    }

    /// 對照 `TestSelectionKeysLettersSelectByPosition`：字母按位置選字。
    ///
    /// 頁面固定 9 項，10 鍵設定只取前 9 個；第 10 個鍵不再選字。
    #[test]
    fn test_selection_keys_letters_select_by_position() {
        for (keys, pressed, index) in [
            ("1234qweras", 'q', 4),
            ("1234qweras", 'w', 5),
            ("1234qweras", '1', 0),
            ("asdfghjkl;", 'a', 0),
            ("asdfghjkl;", 'l', 8),
            ("1234qweras", '5', 4),
            ("1234567890", '9', 8),
            ("asdfghjkl;", '1', 0),
        ] {
            let mut s = many_session(keys);
            type_syllable(&mut s, "ㄕˋ");
            s.press(Key::new(KeyCode::Down, None, false));
            let candidates = s.candidates().to_vec();
            s.press(Key::char(pressed, false));
            assert_eq!(
                s.commit(),
                candidates[index].surface,
                "{keys} 按 {pressed:?} 選第 {} 項",
                index + 1
            );
        }
    }

    /// 對照 `TestSelectionKeysOnlyApplyWhileCandidatesOpen`：候選沒開時字母是注音鍵。
    #[test]
    fn test_selection_keys_only_apply_while_candidates_open() {
        let mut s = many_session("asdfghjkl;");
        let snap = s.press(Key::char('a', false));
        assert_eq!(snap.preedit, "ㄇ");
        let snap = s.press(Key::char(';', false));
        assert_eq!(snap.preedit, "ㄇㄤ");
    }

    /// 對照 `TestSelectionKeysInEmojiPalette`：選單也用同一組選字鍵。
    #[test]
    fn test_selection_keys_in_emoji_palette() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.style = InputStyle {
            selection_keys: "1234qweras".to_string(),
            ..InputStyle::default()
        };
        let mut s = Session::new(config);
        let snap = s.open_emoji("emoji");
        let want = snap.candidates[4].surface.clone();
        s.press(Key::char('q', false));
        assert_eq!(s.commit(), want);
    }

    /// 頁面大小固定 9，與風格設定無關；`page(1)` 跳到第 9 項。
    #[test]
    fn test_candidate_page_size_can_be_nine() {
        let mut s = many_session("");
        assert_eq!(s.page_size(), 9, "預設風格也是 9");
        s.style = InputStyle::preset("mcbopomofo").expect("預設存在");
        assert_eq!(s.page_size(), 9, "風格設定不再改變頁面大小");
        type_syllable(&mut s, "ㄕˋ");
        s.page(1);
        assert_eq!(s.highlighted(), 9);
    }

    /// 選字鍵每排欄數。
    #[test]
    fn test_selection_key_columns() {
        let mut s = many_session("1234qweras");
        assert_eq!(s.selection_key_columns(), 4);
        s.style.selection_keys = "1234567890".to_string();
        assert_eq!(s.selection_key_columns(), 5);
    }

    /// 對照 `TestSelectionKeysDefaultLabelsDigits` 的 `page_size` 邊界：9 項頁面
    /// 上 `0` 不是選字鍵。
    #[test]
    fn test_page_size_nine_digit_zero_not_a_selection_key() {
        let mut s = many_session("");
        s.style = InputStyle::preset("mcbopomofo").expect("預設存在");
        type_syllable(&mut s, "ㄕˋ");
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::char('0', false));
        assert_eq!(snap.preedit, "ㄢ是", "0 回到注音鍵，離開選取模式");
    }

    /// 閒置按鍵：注音鍵、標點與反引號都在清單內；@ 不在。
    #[test]
    fn test_idle_keys_lists_phonetic_and_punctuation() {
        let idle = idle_keys(Keyboard::standard(), false, "equals");
        for key in ['1', 'a', ' ', '<', '`', '=', '3'] {
            if key == ' ' {
                assert!(!idle.contains(key), "空白不在清單");
                continue;
            }
            assert!(idle.contains(key), "{key:?} 應在閒置清單");
        }
        assert!(!idle.contains('@'), "@ 交還應用程式");
        assert!(!idle.contains('A'), "Shift＋字母交還應用程式");
    }

    /// 全形閒置按鍵多收不是注音鍵也不是標點的字元與大寫字母。
    #[test]
    fn test_full_width_idle_keys_adds_plain_ascii() {
        let keyboard = Keyboard::standard();
        let idle = idle_keys(keyboard, false, "equals");
        let wide = full_width_idle_keys(keyboard, false, "equals");
        for key in ['@', '#', 'A', '1', '`'] {
            assert!(wide.contains(key), "{key:?} 全形時先送核心");
        }
        for key in idle.chars() {
            assert!(wide.contains(key), "{key:?} 半形清單是子集");
        }
        assert!(!wide.contains(' '), "空白不在清單");
    }

    /// 混打開啟時標準排列的可見 ASCII 全都在閒置清單。
    #[test]
    fn test_idle_keys_mixed_includes_every_visible_ascii() {
        let idle = idle_keys(Keyboard::standard(), true, "off");
        for code in 0x21u32..=0x7e {
            let ch = char::from_u32(code).expect("ASCII");
            assert!(idle.contains(ch), "混打時 {ch:?} 應由核心處理");
        }
        let without_equals = idle_keys(Keyboard::standard(), false, "off");
        assert!(!without_equals.contains('='), "關掉捷徑時 = 交還應用程式");
    }

    /// 對照 `ZhuyinKeyPolicy.Handle` 的語意表。
    #[test]
    fn test_zhuyin_key_policy() {
        let space = Key::new(KeyCode::Space, None, false);
        assert_eq!(zhuyin_key_action(&space, false, false), KeyAction::Insert);
        assert_eq!(zhuyin_key_action(&space, true, false), KeyAction::Convert);
        assert_eq!(zhuyin_key_action(&space, false, true), KeyAction::Convert);

        let enter = Key::new(KeyCode::Enter, None, false);
        assert_eq!(zhuyin_key_action(&enter, false, false), KeyAction::None);
        assert_eq!(zhuyin_key_action(&enter, true, false), KeyAction::Commit);

        let tab = Key::new(KeyCode::Tab, None, false);
        assert_eq!(zhuyin_key_action(&tab, true, false), KeyAction::Complete);

        let escape = Key::new(KeyCode::Escape, None, false);
        assert_eq!(zhuyin_key_action(&escape, false, true), KeyAction::Cancel);

        let letter = Key::char('a', false);
        assert_eq!(zhuyin_key_action(&letter, true, false), KeyAction::Insert);

        let backspace = Key::new(KeyCode::Backspace, None, false);
        assert_eq!(zhuyin_key_action(&backspace, true, false), KeyAction::None);
    }

    /// 全形英數對照表。
    #[test]
    fn test_full_width_ascii_table() {
        assert_eq!(full_width_ascii("a1 "), "ａ１\u{3000}");
        assert_eq!(full_width_ascii("中"), "中");
    }

    /// 半形標點對照表。
    #[test]
    fn test_halfwidth_punct_table() {
        assert_eq!(halfwidth_punct("，", '<'), ",");
        assert_eq!(halfwidth_punct("。", '>'), ".");
        assert_eq!(halfwidth_punct("？", '?'), "?");
        assert_eq!(halfwidth_punct("『", '{'), "{");
        assert_eq!(halfwidth_punct("……", '^'), "^");
    }

    /// 一般標點與 Shift 符號照字面插入，不轉全形、不開選單。
    #[test]
    fn test_mixed_punctuation_is_literal() {
        for key in ",.';/\\\"!?:()[]{}<>=~".chars() {
            let mut s = plain_session();
            let snap = s.press(Key::char(key, false));
            assert_eq!(snap.preedit, key.to_string(), "{key:?} 照字面");
            assert!(!snap.navigating, "{key:?} 不開候選");
        }
        for key in "~!@#$%^&*()_+{}|:\"<>?".chars() {
            let mut s = plain_session();
            let snap = s.press(Key::char(key, false));
            assert_eq!(snap.preedit, key.to_string(), "Shift 符號 {key:?}");
        }
    }

    /// 反引號前導的完整單一表。
    #[test]
    fn test_backtick_prefix_single_table() {
        for (key, want) in [
            (',', "，"),
            ('.', "。"),
            ('\'', "、"),
            (';', "；"),
            (':', "："),
            ('?', "？"),
            ('!', "！"),
            ('(', "（"),
            (')', "）"),
            ('/', "／"),
            ('\\', "＼"),
            ('=', "＝"),
            ('~', "～"),
            ('"', "＂"),
            ('@', "＠"),
            ('#', "＃"),
            ('$', "＄"),
            ('%', "％"),
            ('^', "︿"),
            ('&', "＆"),
            ('*', "＊"),
            ('_', "＿"),
            ('{', "｛"),
            ('}', "｝"),
            ('|', "｜"),
            ('+', "＋"),
            ('0', "０"),
            ('5', "５"),
        ] {
            let mut s = plain_session();
            s.press(Key::char('`', false));
            let snap = s.press(Key::char(key, false));
            assert_eq!(snap.preedit, want, "`{key}");
        }
    }

    /// 多變體鍵先插入預設值，游標停在符號右側；候選窗列出其他變體。
    #[test]
    fn test_backtick_prefix_multivariant_default_and_switch() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        let snap = s.press(Key::char('[', false));
        assert_eq!(snap.preedit, "［", "先插入預設值");
        assert_eq!(snap.cursor_rune, 1, "游標在符號右側");
        assert!(!snap.navigating, "先顯示候選，不強制進入選取模式");
        let surfaces: Vec<&str> = snap
            .candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert_eq!(
            surfaces,
            ["［", "「", "『", "【", "〔", "︹", "︻", "﹁", "﹃", "﹝"],
            "[ 的變體要完整對齊微軟注音"
        );

        // ← 回到符號前，再按兩下 ↓ 選第二個變體。
        let left = s.press(Key::new(KeyCode::ArrowLeft, None, false));
        assert_eq!(left.cursor_rune, 0);
        s.press(Key::new(KeyCode::Down, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "「", "↓ 換到下一個變體再 Enter");
        assert!(!snap.navigating);
    }

    /// `]` 的變體清單也完整對齊微軟注音。
    #[test]
    fn test_backtick_prefix_variants_for_right_bracket() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        let snap = s.press(Key::char(']', false));
        let surfaces: Vec<&str> = snap
            .candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert_eq!(
            surfaces,
            ["］", "」", "』", "】", "〕", "︺", "︼", "﹂", "﹄", "﹞"]
        );
    }

    /// 選過第二個變體後，游標回去再 ↓ 仍能重新打開整組候選。
    #[test]
    fn test_backtick_prefix_reopens_variants_after_switch() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        s.press(Key::char('[', false));
        // 選「」：← 回到符號前，↓ 開窗，再一次 ↓ 到第二項。
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "「");
        assert_eq!(snap.cursor_rune, 1, "選完游標在符號右側");

        // ← 回去再 ↓：整組 `[` 變體都要重新出現，目前變體排最前面。
        s.press(Key::new(KeyCode::ArrowLeft, None, false));
        let snap = s.press(Key::new(KeyCode::Down, None, false));
        let surfaces: Vec<&str> = snap
            .candidates
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert_eq!(
            surfaces,
            ["「", "［", "『", "【", "〔", "︹", "︻", "﹁", "﹃", "﹝"],
            "重開時目前變體排最前面，其餘照原順序"
        );
    }

    /// 連按兩下反引號是 ‵。
    #[test]
    fn test_double_backtick_is_reversed_prime() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        let snap = s.press(Key::char('`', false));
        assert_eq!(snap.preedit, "‵");
    }

    /// 非表內鍵：反引號照字面留下，按鍵照常處理。
    #[test]
    fn test_backtick_prefix_non_table_falls_through() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        let snap = s.press(Key::char('q', false));
        assert_eq!(snap.preedit, "`q", "反引號與按鍵都留在字面");
    }

    /// 連續兩段前導符號：組字與提交都要保留兩段。
    #[test]
    fn test_two_prefix_symbols_commit_both() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        s.press(Key::char(';', false));
        s.press(Key::char('`', false));
        let snap = s.press(Key::char('[', false));
        assert_eq!(snap.preedit, "；［");
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "；［");
        assert_eq!(s.commit(), "；［");
    }

    /// 連續三段前導符號；包含多變體鍵。
    #[test]
    fn test_three_prefix_symbols_commit_all() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        s.press(Key::char(';', false));
        s.press(Key::char('`', false));
        s.press(Key::char('[', false));
        s.press(Key::char('`', false));
        let snap = s.press(Key::char(']', false));
        assert_eq!(snap.preedit, "；［］");
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "；［］");
        assert_eq!(s.commit(), "；［］");
    }

    /// 注音音節之後的連續前導符號：套用變體時不能蓋掉整段字面。
    #[test]
    fn test_prefix_symbol_then_zh_syllable() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        s.press(Key::char(';', false));
        type_syllable(&mut s, "ㄨㄛˇ");
        assert_eq!(s.preedit(), "；我", "前導符號與注音字都在");
        // 注音片段會切開字面；再累積兩段前導符號，才會落進同一個字面 run。
        s.press(Key::char('`', false));
        s.press(Key::char(';', false));
        s.press(Key::char('`', false));
        let snap = s.press(Key::char('[', false));
        assert_eq!(snap.preedit, "；我；［", "注音字與兩段前導符號都在");
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "；我；「", "只把游標所在的符號換成變體");
        assert_eq!(s.commit(), "；我；「");
    }

    /// 英文之後的連續前導符號：套用變體時不能蓋掉整段字面。
    #[test]
    fn test_prefix_symbol_then_english() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        s.press(Key::char(';', false));
        type_keys(&mut s, "hello");
        assert_eq!(s.preedit(), "；hello", "前導符號與英文都在");
        // 英文片段會切開字面；再累積兩段前導符號，才會落進同一個字面 run。
        s.press(Key::char('`', false));
        s.press(Key::char(';', false));
        s.press(Key::char('`', false));
        let snap = s.press(Key::char('[', false));
        assert_eq!(snap.preedit, "；hello；［", "英文與兩段前導符號都在");
        s.press(Key::new(KeyCode::Down, None, false));
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(snap.preedit, "；hello；「", "只把游標所在的符號換成變體");
        assert_eq!(s.commit(), "；hello；「");
    }

    /// Esc 取消前導，不留字。
    #[test]
    fn test_backtick_prefix_escape_cancels() {
        let mut s = plain_session();
        s.press(Key::char('`', false));
        let snap = s.press(Key::new(KeyCode::Escape, None, false));
        assert_eq!(snap.preedit, "");
    }

    /// `u` 是保留鍵；這一版反引號照字面、`u` 照一般注音鍵（TODO(prefix-u)）。
    #[test]
    fn prefix_u_is_reserved_and_falls_through_for_now() {
        assert_eq!(prefix_reserved_reason('u'), Some("unicode"));
        let mut s = plain_session();
        s.press(Key::char('`', false));
        let snap = s.press(Key::char('u', false));
        assert!(
            snap.preedit.starts_with('`'),
            "反引號照字面：{:?}",
            snap.preedit
        );
        assert_ne!(snap.preedit, "`u 一", "尚未實作 Unicode 前導");
    }

    /// `b` 是保留鍵；這一版反引號照字面、`b` 照一般注音鍵（TODO(prefix-b)）。
    #[test]
    fn prefix_b_is_reserved_and_falls_through_for_now() {
        assert_eq!(prefix_reserved_reason('b'), Some("big5"));
        let mut s = plain_session();
        s.press(Key::char('`', false));
        let snap = s.press(Key::char('b', false));
        assert!(
            snap.preedit.starts_with('`'),
            "反引號照字面：{:?}",
            snap.preedit
        );
        assert_ne!(snap.preedit, "`b 一", "尚未實作 Big5 前導");
    }
}
