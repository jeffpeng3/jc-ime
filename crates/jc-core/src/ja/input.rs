//! 日文組字層：把實體按鍵變成かな讀音。
//!
//! 這一層只產生かな，不做かな漢字変換。羅馬字與直接かな輸入都輸出
//! [`Unit`] 序列，一個單位是一個かな字元，因此小書きかな與長音符各自
//! 佔一個單位。
//!
//! 行為對照 Go 的 `core/ja/input`：羅馬字狀態機、對照表與標點表都沿用
//! 前身的規則；直接かな輸入另外支援濁點、半濁點與小書きかな。

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::key::{Key, KeyCode};
use crate::reading::Unit;

/// 日文組字層的輸入方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputMode {
    /// 羅馬字輸入。
    Romaji,
    /// 直接かな輸入。
    Kana,
}

/// 一次按鍵的結果。
///
/// `Accepted` 只代表按鍵被接受，不代表完成了かな；完成的單位放在
/// `completed`，可能為空。被拒絕的按鍵不會改變任何狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomajiEvent {
    /// 按鍵不合法，或不能接在目前前綴後面。
    Rejected,
    /// 按鍵被接受。
    Accepted {
        /// 這次按鍵完成的讀音單位。
        completed: Vec<Unit>,
    },
}

/// 直接かな輸入的按鍵結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KanaEvent {
    /// 按鍵不是かな或濁點，狀態不變。
    Rejected,
    /// 這次按鍵完成的讀音單位，至少一個。
    Completed(Vec<Unit>),
}

/// 組字區的一段顯示內容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreeditSegment {
    /// 這一段對應的讀音；未完成的前綴沒有讀音。
    pub reading: Vec<Unit>,
    /// 要顯示的文字。
    pub text: String,
    /// 這一段是否仍未完成。
    pub pending: bool,
}

/// 羅馬字對照表。
///
/// 鍵是羅馬字序列，值是對應的かな；一個值可以包含多個かな字元，例如
/// `kya` 對應 `きゃ`，會拆成兩個 [`Unit`]。表建立後不再修改，可以安全
/// 共用。
#[derive(Debug, Clone, Default)]
pub struct Table {
    entries: HashMap<String, String>,
    prefixes: HashSet<String>,
}

impl Table {
    /// 建立對照表。空鍵或空值會被忽略。
    pub fn new<I, K, V>(entries: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut table = Self::default();
        for (keys, kana) in entries {
            let keys = keys.into();
            let kana = kana.into();
            if keys.is_empty() || kana.is_empty() {
                continue;
            }
            for i in 1..keys.len() {
                if keys.is_char_boundary(i) {
                    table.prefixes.insert(keys[..i].to_string());
                }
            }
            table.entries.insert(keys, kana);
        }
        table
    }

    /// 查詢完整羅馬字序列對應的かな。
    pub fn lookup(&self, keys: &str) -> Option<&str> {
        self.entries.get(keys).map(String::as_str)
    }

    /// 回報序列是否為某個較長對照項的前綴。
    pub fn is_prefix(&self, keys: &str) -> bool {
        self.prefixes.contains(keys)
    }
}

/// 內建羅馬字對照表。
pub fn default_romaji_table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        Table::new([
            // 母音
            ("a", "あ"),
            ("i", "い"),
            ("u", "う"),
            ("e", "え"),
            ("o", "お"),
            // か行
            ("ka", "か"),
            ("ki", "き"),
            ("ku", "く"),
            ("ke", "け"),
            ("ko", "こ"),
            ("kya", "きゃ"),
            ("kyi", "きぃ"),
            ("kyu", "きゅ"),
            ("kye", "きぇ"),
            ("kyo", "きょ"),
            // が行
            ("ga", "が"),
            ("gi", "ぎ"),
            ("gu", "ぐ"),
            ("ge", "げ"),
            ("go", "ご"),
            ("gya", "ぎゃ"),
            ("gyi", "ぎぃ"),
            ("gyu", "ぎゅ"),
            ("gye", "ぎぇ"),
            ("gyo", "ぎょ"),
            // さ行
            ("sa", "さ"),
            ("si", "し"),
            ("shi", "し"),
            ("su", "す"),
            ("se", "せ"),
            ("so", "そ"),
            ("sha", "しゃ"),
            ("shu", "しゅ"),
            ("she", "しぇ"),
            ("sho", "しょ"),
            ("sya", "しゃ"),
            ("syu", "しゅ"),
            ("syo", "しょ"),
            // ざ行
            ("za", "ざ"),
            ("zi", "じ"),
            ("ji", "じ"),
            ("zu", "ず"),
            ("ze", "ぜ"),
            ("zo", "ぞ"),
            ("ja", "じゃ"),
            ("ju", "じゅ"),
            ("je", "じぇ"),
            ("jo", "じょ"),
            ("jya", "じゃ"),
            ("jyu", "じゅ"),
            ("jyo", "じょ"),
            ("zya", "じゃ"),
            ("zyu", "じゅ"),
            ("zyo", "じょ"),
            // た行
            ("ta", "た"),
            ("ti", "ち"),
            ("chi", "ち"),
            ("tsu", "つ"),
            ("tu", "つ"),
            ("te", "て"),
            ("to", "と"),
            ("cha", "ちゃ"),
            ("chu", "ちゅ"),
            ("che", "ちぇ"),
            ("cho", "ちょ"),
            ("tya", "ちゃ"),
            ("tyu", "ちゅ"),
            ("tyo", "ちょ"),
            ("cya", "ちゃ"),
            ("cyu", "ちゅ"),
            ("cyo", "ちょ"),
            // だ行
            ("da", "だ"),
            ("di", "ぢ"),
            ("du", "づ"),
            ("de", "で"),
            ("do", "ど"),
            ("dya", "ぢゃ"),
            ("dyu", "ぢゅ"),
            ("dyo", "ぢょ"),
            // な行
            ("na", "な"),
            ("ni", "に"),
            ("nu", "ぬ"),
            ("ne", "ね"),
            ("no", "の"),
            ("nya", "にゃ"),
            ("nyu", "にゅ"),
            ("nyo", "にょ"),
            // は行
            ("ha", "は"),
            ("hi", "ひ"),
            ("hu", "ふ"),
            ("fu", "ふ"),
            ("he", "へ"),
            ("ho", "ほ"),
            ("hya", "ひゃ"),
            ("hyu", "ひゅ"),
            ("hyo", "ひょ"),
            ("fa", "ふぁ"),
            ("fi", "ふぃ"),
            ("fe", "ふぇ"),
            ("fo", "ふぉ"),
            ("fya", "ふゃ"),
            ("fyu", "ふゅ"),
            ("fyo", "ふょ"),
            // ば行
            ("ba", "ば"),
            ("bi", "び"),
            ("bu", "ぶ"),
            ("be", "べ"),
            ("bo", "ぼ"),
            ("bya", "びゃ"),
            ("byu", "びゅ"),
            ("byo", "びょ"),
            // ぱ行
            ("pa", "ぱ"),
            ("pi", "ぴ"),
            ("pu", "ぷ"),
            ("pe", "ぺ"),
            ("po", "ぽ"),
            ("pya", "ぴゃ"),
            ("pyu", "ぴゅ"),
            ("pyo", "ぴょ"),
            // ま行
            ("ma", "ま"),
            ("mi", "み"),
            ("mu", "む"),
            ("me", "め"),
            ("mo", "も"),
            ("mya", "みゃ"),
            ("myu", "みゅ"),
            ("myo", "みょ"),
            // や行
            ("ya", "や"),
            ("yi", "い"),
            ("yu", "ゆ"),
            ("ye", "いぇ"),
            ("yo", "よ"),
            // ら行
            ("ra", "ら"),
            ("ri", "り"),
            ("ru", "る"),
            ("re", "れ"),
            ("ro", "ろ"),
            ("rya", "りゃ"),
            ("ryu", "りゅ"),
            ("ryo", "りょ"),
            // わ行
            ("wa", "わ"),
            ("wi", "うぃ"),
            ("wu", "う"),
            ("we", "うぇ"),
            ("wo", "を"),
            ("wyi", "ゐ"),
            ("wye", "ゑ"),
            // ゔ行
            ("va", "ゔぁ"),
            ("vi", "ゔぃ"),
            ("vu", "ゔ"),
            ("ve", "ゔぇ"),
            ("vo", "ゔぉ"),
            ("vya", "ゔゃ"),
            ("vyu", "ゔゅ"),
            ("vyo", "ゔょ"),
            // くぁ行
            ("qa", "くぁ"),
            ("qi", "くぃ"),
            ("qu", "く"),
            ("qe", "くぇ"),
            ("qo", "くぉ"),
            ("qya", "くゃ"),
            ("qyu", "くゅ"),
            ("qyo", "くょ"),
            // c で始まる別表記
            ("ca", "か"),
            ("ci", "し"),
            ("cu", "く"),
            ("ce", "せ"),
            ("co", "こ"),
            // 小書きかな（x）
            ("xa", "ぁ"),
            ("xi", "ぃ"),
            ("xu", "ぅ"),
            ("xe", "ぇ"),
            ("xo", "ぉ"),
            ("xya", "ゃ"),
            ("xyu", "ゅ"),
            ("xyo", "ょ"),
            ("xtu", "っ"),
            ("xtsu", "っ"),
            ("xwa", "ゎ"),
            ("xka", "ゕ"),
            ("xke", "ゖ"),
            // 小書きかな（l）
            ("la", "ぁ"),
            ("li", "ぃ"),
            ("lu", "ぅ"),
            ("le", "ぇ"),
            ("lo", "ぉ"),
            ("lya", "ゃ"),
            ("lyu", "ゅ"),
            ("lyo", "ょ"),
            ("ltu", "っ"),
            ("ltsu", "っ"),
            ("lwa", "ゎ"),
            ("lka", "ゕ"),
            ("lke", "ゖ"),
            // 長音符
            ("-", "ー"),
        ])
    })
}

/// 羅馬字組字狀態機。
///
/// 未完成的按鍵保留在 pending，例如 `k`；下一個按鍵完成かな後才放進
/// 已完成的單位。單獨的 `n` 會停在 pending，等到邊界、[`flush`] 或下一個
/// 按鍵才成為 `ん`。
///
/// [`flush`]: RomajiInput::flush
#[derive(Debug, Clone)]
pub struct RomajiInput {
    table: Table,
    units: Vec<Unit>,
    pending: String,
    /// 剛打完 `nn` 並已輸出 `ん`；第二個 `n` 仍可和下一個母音或 `y`
    /// 組成な行かな。
    carry_n: bool,
}

impl RomajiInput {
    /// 建立使用內建對照表的羅馬字組字器。
    pub fn new() -> Self {
        Self::with_table(default_romaji_table().clone())
    }

    /// 建立使用自訂對照表的羅馬字組字器。
    pub fn with_table(table: Table) -> Self {
        Self {
            table,
            units: Vec::new(),
            pending: String::new(),
            carry_n: false,
        }
    }

    /// 目前的輸入方式。
    pub fn mode(&self) -> InputMode {
        InputMode::Romaji
    }

    /// 處理一個按鍵。
    ///
    /// 只接受單一可列印 ASCII 字元；具名鍵請呼叫 [`backspace`] 或
    /// [`cancel`]。
    ///
    /// [`backspace`]: RomajiInput::backspace
    /// [`cancel`]: RomajiInput::cancel
    pub fn press(&mut self, key: Key) -> RomajiEvent {
        let KeyCode::Char(ch) = key.code else {
            return RomajiEvent::Rejected;
        };
        if !('!'..='~').contains(&ch) {
            return RomajiEvent::Rejected;
        }
        match self.push(ch.to_ascii_lowercase()) {
            PushResult::Rejected => RomajiEvent::Rejected,
            PushResult::Accepted(completed) => RomajiEvent::Accepted { completed },
        }
    }

    /// 先移除未完成的按鍵；沒有未完成按鍵時移除最後一個已完成的かな。
    ///
    /// 兩者都沒有時回傳 `false`。
    pub fn backspace(&mut self) -> bool {
        if self.carry_n {
            self.carry_n = false;
            self.units.pop();
            return true;
        }
        if !self.pending.is_empty() {
            self.pending.pop();
            return true;
        }
        if !self.units.is_empty() {
            self.units.pop();
            return true;
        }
        false
    }

    /// 清空所有組字狀態。
    pub fn reset(&mut self) {
        self.units.clear();
        self.pending.clear();
        self.carry_n = false;
    }

    /// 清空整段組字，對應 Escape；有東西被清掉時回傳 `true`。
    pub fn cancel(&mut self) -> bool {
        if self.units.is_empty() && self.pending.is_empty() && !self.carry_n {
            return false;
        }
        self.reset();
        true
    }

    /// 結束目前輸入，把可以獨立的未完成按鍵變成かな。
    ///
    /// 目前只有單獨的 `n` 會變成 `ん`；其他未完成前綴保持原狀並回傳空
    /// 序列。
    pub fn flush(&mut self) -> Vec<Unit> {
        if self.pending == "n" {
            self.pending.clear();
            return self.emit(Unit::new("ん"));
        }
        Vec::new()
    }

    /// 已完成的讀音單位。
    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// 尚未完成的按鍵，例如 `k`；沒有時為空字串。
    pub fn pending(&self) -> &str {
        &self.pending
    }

    /// 組字區的顯示分段：已完成的かな一段，未完成按鍵一段。
    pub fn segments(&self) -> Vec<PreeditSegment> {
        segments(&self.units, &self.pending)
    }

    /// 組字區目前顯示的所有文字，包含未完成按鍵。
    pub fn preedit_text(&self) -> String {
        preedit_text(&self.units, &self.pending)
    }

    /// 游標位置，單位是顯示字元數。
    ///
    /// 組字層只在尾端插入與刪除，所以游標固定在組字內容之後。
    pub fn cursor(&self) -> usize {
        self.preedit_text().chars().count()
    }

    fn push(&mut self, key: char) -> PushResult {
        if self.carry_n {
            if !self.known_start(key) {
                return PushResult::Rejected;
            }
            self.carry_n = false;
            if is_vowel_or_y(key) {
                self.pending.push('n');
                return self.push_generic(key);
            }
            if key == 'n' {
                self.pending.push('n');
                return PushResult::Accepted(Vec::new());
            }
            if key == '\'' {
                return PushResult::Accepted(Vec::new());
            }
            return self.push(key);
        }
        if self.pending == "n" {
            return self.push_after_n(key);
        }
        self.push_generic(key)
    }

    /// 處理前一個按鍵是單獨 `n` 的情況。
    ///
    /// `na`、`ni`、`nya` 走一般對照表；`n` 後面接子音、符號或長音符時
    /// 先完成 `ん`。
    fn push_after_n(&mut self, key: char) -> PushResult {
        if key == 'n' {
            self.pending.clear();
            self.carry_n = true;
            return PushResult::Accepted(self.emit(Unit::new("ん")));
        }
        if key == '\'' {
            self.pending.clear();
            return PushResult::Accepted(self.emit(Unit::new("ん")));
        }
        if is_vowel_or_y(key) {
            return self.push_generic(key);
        }
        if self.known_start(key) {
            self.pending.clear();
            return self.emit_then(Unit::new("ん"), key);
        }
        PushResult::Rejected
    }

    fn push_generic(&mut self, key: char) -> PushResult {
        let candidate = format!("{}{}", self.pending, key);

        if let Some(kana) = self.table.lookup(&candidate).map(str::to_string) {
            self.pending.clear();
            let units = split_units(&kana);
            self.units.extend(units.iter().cloned());
            return PushResult::Accepted(units);
        }

        if self.table.is_prefix(&candidate) {
            self.pending = candidate;
            return PushResult::Accepted(Vec::new());
        }

        // 促音：相同子音連續出現時先完成 `っ`，第二個子音留在 pending，
        // 讓後面的母音或拗音組成下一個かな。`n` 由上面的專用規則處理。
        let chars: Vec<char> = candidate.chars().collect();
        if chars.len() == 2 && chars[0] == chars[1] && can_sokuon(chars[0]) {
            self.pending.clear();
            self.pending.push(chars[1]);
            return PushResult::Accepted(self.emit(Unit::new("っ")));
        }

        PushResult::Rejected
    }

    fn emit(&mut self, unit: Unit) -> Vec<Unit> {
        self.units.push(unit.clone());
        vec![unit]
    }

    fn emit_then(&mut self, unit: Unit, key: char) -> PushResult {
        let mut done = self.emit(unit);
        match self.push(key) {
            PushResult::Rejected => PushResult::Rejected,
            PushResult::Accepted(rest) => {
                done.extend(rest);
                PushResult::Accepted(done)
            }
        }
    }

    fn known_start(&self, key: char) -> bool {
        if key == 'n' || key == '\'' {
            return true;
        }
        let key = key.to_string();
        self.table.lookup(&key).is_some() || self.table.is_prefix(&key)
    }
}

impl Default for RomajiInput {
    fn default() -> Self {
        Self::new()
    }
}

/// 直接かな組字狀態機。
///
/// 每次接受一個かな字元就完成一個讀音單位。片假名會轉成平假名；濁點與
/// 半濁點會和上一個單位組合成濁音或半濁音。小書きかな是一個獨立單位，
/// 和前一個かな各自保留。
#[derive(Debug, Clone, Default)]
pub struct KanaInput {
    units: Vec<Unit>,
}

impl KanaInput {
    /// 建立直接かな組字器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 目前的輸入方式。
    pub fn mode(&self) -> InputMode {
        InputMode::Kana
    }

    /// 處理一個按鍵。
    pub fn press(&mut self, key: Key) -> KanaEvent {
        let KeyCode::Char(ch) = key.code else {
            return KanaEvent::Rejected;
        };
        match self.plan(ch) {
            Some(KanaAction::Push(unit)) => {
                self.units.push(unit.clone());
                KanaEvent::Completed(vec![unit])
            }
            Some(KanaAction::ReplaceLast(text)) => {
                let last = Unit::new(text);
                if let Some(slot) = self.units.last_mut() {
                    *slot = last.clone();
                }
                KanaEvent::Completed(vec![last])
            }
            None => KanaEvent::Rejected,
        }
    }

    /// 移除最後一個已完成的かな；沒有內容時回傳 `false`。
    pub fn backspace(&mut self) -> bool {
        self.units.pop().is_some()
    }

    /// 清空所有組字狀態。
    pub fn reset(&mut self) {
        self.units.clear();
    }

    /// 清空整段組字，對應 Escape；有東西被清掉時回傳 `true`。
    pub fn cancel(&mut self) -> bool {
        if self.units.is_empty() {
            return false;
        }
        self.units.clear();
        true
    }

    /// 直接かな輸入沒有未完成的前綴，一律回傳空序列。
    pub fn flush(&mut self) -> Vec<Unit> {
        Vec::new()
    }

    /// 已完成的讀音單位。
    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// 直接かな輸入沒有未完成按鍵，一律回傳空字串。
    pub fn pending(&self) -> &str {
        ""
    }

    /// 組字區的顯示分段。
    pub fn segments(&self) -> Vec<PreeditSegment> {
        segments(&self.units, "")
    }

    /// 組字區目前顯示的所有文字。
    pub fn preedit_text(&self) -> String {
        preedit_text(&self.units, "")
    }

    /// 游標位置，單位是顯示字元數。
    pub fn cursor(&self) -> usize {
        self.preedit_text().chars().count()
    }

    fn plan(&self, ch: char) -> Option<KanaAction> {
        if is_dakuten(ch) {
            let last = self.units.last()?.as_str();
            return dakuten_on(last).map(KanaAction::ReplaceLast);
        }
        if is_handakuten(ch) {
            let last = self.units.last()?.as_str();
            return handakuten_on(last).map(KanaAction::ReplaceLast);
        }
        normalize_kana(ch).map(|kana| KanaAction::Push(Unit::new(kana.to_string())))
    }
}

/// 可設定的日文標點與記号鍵對照表。
///
/// 組字層的讀音單位只放かな，因此標點不進入讀音序列。外殼可以在送出
/// 按鍵前先查這張表。
#[derive(Debug, Clone, Default)]
pub struct Symbols {
    table: HashMap<String, String>,
}

impl Symbols {
    /// 建立自訂對照表。空鍵或空值會被忽略。
    pub fn new<I, K, V>(entries: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut table = Self::default();
        for (key, text) in entries {
            let key = key.into();
            let text = text.into();
            if key.is_empty() || text.is_empty() {
                continue;
            }
            table.table.insert(key, text);
        }
        table
    }

    /// 查詢按鍵對應的標點或記号。
    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.table.get(key).map(String::as_str)
    }

    /// 沒有對照的輸入原樣回傳。
    pub fn insert(&self, text: &str) -> String {
        self.lookup(text).unwrap_or(text).to_string()
    }
}

/// 內建日文標點對照表。
pub fn default_symbols() -> &'static Symbols {
    static SYMBOLS: OnceLock<Symbols> = OnceLock::new();
    SYMBOLS.get_or_init(|| {
        Symbols::new([
            (".", "。"),
            (",", "、"),
            ("?", "？"),
            ("!", "！"),
            ("[", "「"),
            ("]", "」"),
            ("{", "『"),
            ("}", "』"),
            ("(", "（"),
            (")", "）"),
            ("/", "・"),
            ("\\", "￥"),
            ("~", "〜"),
            (":", "："),
            (";", "；"),
            ("<", "＜"),
            (">", "＞"),
            ("@", "＠"),
            ("#", "＃"),
            ("$", "＄"),
            ("%", "％"),
            ("&", "＆"),
            ("*", "＊"),
            ("+", "＋"),
            ("=", "＝"),
            ("_", "＿"),
            ("|", "｜"),
            ("`", "｀"),
            ("^", "＾"),
        ])
    })
}

enum PushResult {
    Rejected,
    Accepted(Vec<Unit>),
}

enum KanaAction {
    Push(Unit),
    ReplaceLast(String),
}

fn segments(units: &[Unit], pending: &str) -> Vec<PreeditSegment> {
    let mut out = Vec::new();
    if !units.is_empty() {
        out.push(PreeditSegment {
            reading: units.to_vec(),
            text: units.iter().map(Unit::as_str).collect(),
            pending: false,
        });
    }
    if !pending.is_empty() {
        out.push(PreeditSegment {
            reading: Vec::new(),
            text: pending.to_string(),
            pending: true,
        });
    }
    out
}

fn preedit_text(units: &[Unit], pending: &str) -> String {
    let mut text: String = units.iter().map(Unit::as_str).collect();
    text.push_str(pending);
    text
}

fn split_units(kana: &str) -> Vec<Unit> {
    kana.chars().map(|r| Unit::new(r.to_string())).collect()
}

fn is_vowel_or_y(key: char) -> bool {
    matches!(key, 'a' | 'i' | 'u' | 'e' | 'o' | 'y')
}

/// 重複這個子音時是否形成促音。
///
/// `n` 由專用規則處理，`y`、`w`、`x`、`l` 不使用促音寫法。
fn can_sokuon(c: char) -> bool {
    matches!(
        c,
        'b' | 'c'
            | 'd'
            | 'f'
            | 'g'
            | 'h'
            | 'j'
            | 'k'
            | 'm'
            | 'p'
            | 'q'
            | 'r'
            | 's'
            | 't'
            | 'v'
            | 'z'
    )
}

/// 把直接かな輸入正規化成平假名。
///
/// 片假名轉成對應平假名，長音符與踊り字保持原樣。舊假名 `ゐ`、`ゑ`、
/// `を` 也保持原樣，與前身一致。
fn normalize_kana(r: char) -> Option<char> {
    match r {
        'ぁ'..='ゖ' => Some(r),
        'ー' | 'ゝ' | 'ゞ' => Some(r),
        'ァ'..='ヶ' => char::from_u32(r as u32 - 0x60).or(Some(r)),
        'ヽ' => Some('ゝ'),
        'ヾ' => Some('ゞ'),
        _ => None,
    }
}

fn is_dakuten(ch: char) -> bool {
    matches!(ch, '\u{3099}' | '\u{309B}')
}

fn is_handakuten(ch: char) -> bool {
    matches!(ch, '\u{309A}' | '\u{309C}')
}

/// 對一個かな套用濁點。
fn dakuten_on(kana: &str) -> Option<String> {
    let voiced = match kana {
        "か" => 'が',
        "き" => 'ぎ',
        "く" => 'ぐ',
        "け" => 'げ',
        "こ" => 'ご',
        "さ" => 'ざ',
        "し" => 'じ',
        "す" => 'ず',
        "せ" => 'ぜ',
        "そ" => 'ぞ',
        "た" => 'だ',
        "ち" => 'ぢ',
        "つ" => 'づ',
        "て" => 'で',
        "と" => 'ど',
        "は" => 'ば',
        "ひ" => 'び',
        "ふ" => 'ぶ',
        "へ" => 'べ',
        "ほ" => 'ぼ',
        "う" => 'ゔ',
        _ => return None,
    };
    Some(voiced.to_string())
}

/// 對一個かな套用半濁點。
fn handakuten_on(kana: &str) -> Option<String> {
    let semi = match kana {
        "は" => 'ぱ',
        "ひ" => 'ぴ',
        "ふ" => 'ぷ',
        "へ" => 'ぺ',
        "ほ" => 'ぽ',
        _ => return None,
    };
    Some(semi.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_key(ch: char) -> Key {
        Key::char(ch, false)
    }

    fn press_str(input: &mut RomajiInput, keys: &str) -> Vec<Unit> {
        let mut completed = Vec::new();
        for ch in keys.chars() {
            match input.press(char_key(ch)) {
                RomajiEvent::Accepted { completed: done } => completed.extend(done),
                RomajiEvent::Rejected => panic!("Insert({ch:?}) rejected after {completed:?}"),
            }
        }
        completed
    }

    fn units_of(seq: &[&str]) -> Vec<Unit> {
        seq.iter().map(|s| Unit::new(*s)).collect()
    }

    #[test]
    fn test_romaji_composer_table() {
        let path = jc_testdata::require_data!("0c-testdata/ja/input/romaji.tsv");
        let text = std::fs::read_to_string(&path).expect("讀取 romaji.tsv");
        let mut cases = 0;
        for (line_no, line) in text.lines().enumerate() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(
                fields.len(),
                3,
                "{}:{}: 欄位數 {}，預期 3",
                path.display(),
                line_no + 1,
                fields.len()
            );
            let keys = fields[0];
            let want_units: Vec<Unit> = if fields[1] == "-" {
                Vec::new()
            } else {
                fields[1]
                    .split(' ')
                    .flat_map(|chunk| chunk.chars())
                    .map(|r| Unit::new(r.to_string()))
                    .collect()
            };
            let want_pending = if fields[2] == "-" { "" } else { fields[2] };

            let mut input = RomajiInput::new();
            let completed = press_str(&mut input, keys);

            assert_eq!(input.units(), want_units.as_slice(), "案例 {keys:?} 的單位");
            assert_eq!(input.pending(), want_pending, "案例 {keys:?} 的前綴");
            assert_eq!(completed, want_units, "案例 {keys:?} 的 Completed 加總不符");

            let mut want_display: String = want_units.iter().map(Unit::as_str).collect();
            want_display.push_str(want_pending);
            assert_eq!(input.preedit_text(), want_display, "案例 {keys:?} 的顯示");
            assert_eq!(
                input.cursor(),
                want_display.chars().count(),
                "案例 {keys:?} 的游標"
            );

            let segments = input.segments();
            let mut display = String::new();
            for segment in &segments {
                display.push_str(&segment.text);
                let want_pending_flag = segment.text == want_pending && !want_pending.is_empty();
                assert_eq!(
                    segment.pending, want_pending_flag,
                    "案例 {keys:?} 的片段 {segment:?} pending 旗標"
                );
                if !segment.pending {
                    assert_eq!(segment.reading, want_units, "案例 {keys:?} 的片段讀音");
                }
            }
            assert_eq!(display, want_display, "案例 {keys:?} 的片段組合");
            cases += 1;
        }
        assert!(cases >= 200, "案例數 {cases}，預期至少 200");
    }

    #[test]
    fn test_backspace() {
        let mut input = RomajiInput::new();
        assert!(!input.backspace(), "空組字器的 Backspace 應回傳 false");

        assert!(matches!(
            input.press(char_key('k')),
            RomajiEvent::Accepted { .. }
        ));
        assert!(input.backspace(), "Backspace 未移除未完成按鍵");
        assert_eq!(input.pending(), "");
        assert!(input.units().is_empty());

        press_str(&mut input, "kak");
        assert_eq!(input.units(), units_of(&["か"]).as_slice());
        assert_eq!(input.pending(), "k");
        assert!(input.backspace());
        assert_eq!(input.pending(), "");
        assert_eq!(input.units(), units_of(&["か"]).as_slice());
        assert!(input.backspace());
        assert!(input.units().is_empty());
        assert!(!input.backspace());

        // nn 會先完成 `ん`，Backspace 要能刪掉這個かな。
        let mut input = RomajiInput::new();
        press_str(&mut input, "nn");
        assert_eq!(input.units(), units_of(&["ん"]).as_slice());
        assert!(input.backspace());
        assert!(input.units().is_empty());
        assert_eq!(input.pending(), "");
    }

    #[test]
    fn test_flush() {
        struct Case {
            keys: &'static str,
            flushed: &'static [&'static str],
            units: &'static [&'static str],
            pending: &'static str,
        }
        let cases = [
            Case {
                keys: "n",
                flushed: &["ん"],
                units: &["ん"],
                pending: "",
            },
            Case {
                keys: "nn",
                flushed: &[],
                units: &["ん"],
                pending: "",
            },
            Case {
                keys: "nnn",
                flushed: &["ん"],
                units: &["ん", "ん"],
                pending: "",
            },
            Case {
                keys: "k",
                flushed: &[],
                units: &[],
                pending: "k",
            },
            Case {
                keys: "",
                flushed: &[],
                units: &[],
                pending: "",
            },
        ];
        for case in cases {
            let mut input = RomajiInput::new();
            press_str(&mut input, case.keys);
            let flushed = input.flush();
            assert_eq!(flushed, units_of(case.flushed), "{}: Flush", case.keys);
            assert_eq!(input.units(), units_of(case.units), "{}: units", case.keys);
            assert_eq!(input.pending(), case.pending, "{}: pending", case.keys);
        }
    }

    #[test]
    fn test_cancel() {
        let mut input = RomajiInput::new();
        assert!(!input.cancel(), "空組字器的 Cancel 應回傳 false");
        press_str(&mut input, "kan");
        assert!(input.cancel(), "Cancel 未清空組字");
        assert_eq!(input.pending(), "");
        assert!(input.units().is_empty());
        assert_eq!(input.preedit_text(), "");
        assert_eq!(input.cursor(), 0);
        assert!(!input.cancel(), "第二次 Cancel 應回傳 false");
    }

    #[test]
    fn test_invalid_combinations() {
        let mut input = RomajiInput::new();
        assert!(matches!(
            input.press(char_key('k')),
            RomajiEvent::Accepted { .. }
        ));
        assert_eq!(input.press(char_key('z')), RomajiEvent::Rejected);
        assert_eq!(input.pending(), "k");
        assert!(input.units().is_empty());
        assert_eq!(
            input.press(char_key('a')),
            RomajiEvent::Accepted {
                completed: units_of(&["か"])
            }
        );

        for key in ['@', ' ', 'あ'] {
            let mut input = RomajiInput::new();
            assert_eq!(
                input.press(char_key(key)),
                RomajiEvent::Rejected,
                "羅馬字模式接受了 {key:?}"
            );
        }

        let mut input = RomajiInput::new();
        let _ = input.press(char_key('n'));
        let _ = input.press(char_key('@'));
        assert_eq!(input.pending(), "n", "n 後接非法鍵改變了 pending");

        let mut input = RomajiInput::new();
        press_str(&mut input, "KA");
        assert_eq!(input.units(), units_of(&["か"]).as_slice());
    }

    #[test]
    fn test_kana_composer() {
        let mut input = KanaInput::new();
        assert_eq!(input.mode(), InputMode::Kana);

        let cases = [
            ("き", &["き"][..]),
            ("ゃ", &["ゃ"][..]),
            ("カ", &["か"][..]),
            ("ヴ", &["ゔ"][..]),
            ("ー", &["ー"][..]),
            ("きゃ", &["き", "ゃ"][..]),
        ];
        let mut want = Vec::new();
        for (key, units) in cases {
            let mut got = Vec::new();
            for ch in key.chars() {
                match input.press(char_key(ch)) {
                    KanaEvent::Completed(done) => got.extend(done),
                    KanaEvent::Rejected => panic!("Insert({ch:?}) rejected"),
                }
                assert_eq!(input.pending(), "");
            }
            assert_eq!(got, units_of(units), "Insert({key:?})");
            want.extend(units_of(units));
        }
        assert_eq!(input.units(), want.as_slice());

        let before = input.units().to_vec();
        for key in ["a", "b", "c", "", "漢", "ｱ"] {
            for ch in key.chars() {
                assert_eq!(input.press(char_key(ch)), KanaEvent::Rejected);
            }
            assert_eq!(input.units(), before.as_slice(), "拒絕的輸入改動了單位");
        }
        assert_eq!(input.press(char_key(' ')), KanaEvent::Rejected);

        assert!(input.backspace());
        assert_ne!(input.units(), before.as_slice());
    }

    #[test]
    fn test_kana_composer_dakuten_and_small_kana() {
        let cases = [
            ('か', '\u{309B}', &["が"][..]),
            ('か', '\u{3099}', &["が"][..]),
            ('は', '\u{309C}', &["ぱ"][..]),
            ('は', '\u{309A}', &["ぱ"][..]),
            ('う', '\u{309B}', &["ゔ"][..]),
            ('カ', '\u{309B}', &["が"][..]),
            ('き', 'ゃ', &["き", "ゃ"][..]),
        ];
        for (base, mark, want) in cases {
            let mut input = KanaInput::new();
            assert!(matches!(
                input.press(char_key(base)),
                KanaEvent::Completed(_)
            ));
            assert_eq!(
                input.press(char_key(mark)),
                KanaEvent::Completed(units_of(want)),
                "{base:?} + {mark:?}"
            );
            assert_eq!(input.units(), units_of(want), "{base:?} + {mark:?}");
        }

        // 不能加濁點的かな維持原狀，按鍵被拒絕。
        let mut input = KanaInput::new();
        let _ = input.press(char_key('が'));
        assert_eq!(input.press(char_key('\u{309B}')), KanaEvent::Rejected);
        assert_eq!(input.units(), units_of(&["が"]).as_slice());

        // 沒有上一個かな時，濁點本身不是かな。
        let mut input = KanaInput::new();
        assert_eq!(input.press(char_key('\u{309B}')), KanaEvent::Rejected);
        assert!(input.units().is_empty());
    }

    #[test]
    fn test_kana_normalization_preserves_old_kana() {
        let mut input = KanaInput::new();
        for (key, want) in [
            ('ゐ', "ゐ"),
            ('ゑ', "ゑ"),
            ('を', "を"),
            ('ヽ', "ゝ"),
            ('ヾ', "ゞ"),
        ] {
            let done = match input.press(char_key(key)) {
                KanaEvent::Completed(done) => done,
                KanaEvent::Rejected => panic!("Insert({key:?}) rejected"),
            };
            assert_eq!(done, units_of(&[want]), "Insert({key:?})");
        }
    }

    #[test]
    fn test_preedit_display() {
        let mut input = RomajiInput::new();
        assert!(input.segments().is_empty());
        assert_eq!(input.preedit_text(), "");
        assert_eq!(input.cursor(), 0);

        let _ = input.press(char_key('k'));
        assert_eq!(input.preedit_text(), "k");
        assert_eq!(input.cursor(), 1);
        assert_eq!(
            input.segments(),
            vec![PreeditSegment {
                reading: Vec::new(),
                text: "k".to_string(),
                pending: true,
            }]
        );

        let _ = input.press(char_key('a'));
        assert_eq!(input.preedit_text(), "か");
        assert_eq!(input.cursor(), 1);
        assert_eq!(input.segments().len(), 1);
        assert_eq!(input.segments()[0].text, "か");
        assert_eq!(input.segments()[0].reading, units_of(&["か"]));
        assert!(!input.segments()[0].pending);

        let _ = input.press(char_key('k'));
        assert_eq!(input.preedit_text(), "かk");
        assert_eq!(input.cursor(), 2);
        assert_eq!(input.segments().len(), 2);
        assert!(input.segments()[1].pending);
    }

    #[test]
    fn test_custom_romaji_table() {
        let table = Table::new([("zj", "ず"), ("ka", "か")]);
        let mut input = RomajiInput::with_table(table);
        press_str(&mut input, "zj");
        assert_eq!(input.units(), units_of(&["ず"]).as_slice());
        assert_eq!(input.press(char_key('a')), RomajiEvent::Rejected);
    }

    #[test]
    fn test_symbols() {
        let symbols = default_symbols();
        for (key, want) in [
            (".", "。"),
            (",", "、"),
            ("?", "？"),
            ("!", "！"),
            ("[", "「"),
            ("]", "」"),
            ("/", "・"),
        ] {
            assert_eq!(symbols.lookup(key), Some(want));
            assert_eq!(symbols.insert(key), want);
        }
        assert_eq!(symbols.insert("kana"), "kana");

        let custom = Symbols::new([("x", "※")]);
        assert_eq!(custom.lookup("x"), Some("※"));
        assert_eq!(custom.insert("."), ".");
    }
}
