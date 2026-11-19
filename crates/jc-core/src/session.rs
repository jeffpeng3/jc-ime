//! 輸入工作階段：把注音／羅馬字輸入、分段、路由與轉換串成一次完整輸入。
//!
//! [`Session`] 是外殼唯一需要持有的狀態。每次 [`Session::press`] 都回傳完整
//! [`Snapshot`]，外殼不必自己組字；[`Session::commit`] 取走目前結果。
//!
//! 三條路徑共用同一個介面：
//! - 注音：`zh::input::ZhInput` 累積音節，`decode::Decoder` 在詞格上解碼。
//! - 日文：`ja::input::RomajiInput` 累積かな，`ja::decode::JaDecoder` 轉換。
//! - 英文與混合：混合模式由 `mixed::AsciiInput` 推斷英數按鍵，其餘交給
//!   `mixed::Router` 逐段路由。
//!
//! 沒有詞典時注音與日文仍可運作：詞格只有未知詞邊，輸出等於讀音本身，
//! 方便在沒有資料的環境跑協定與薄層行為測試。

use std::sync::Arc;

use crate::candidate::{Candidate, CandidateSource};
use crate::decode::{Decoder, Locks};
use crate::ja::decode::{JaContext, JaDecoder};
use crate::ja::input::RomajiInput;
use crate::key::{Key, KeyCode};
use crate::lang::Mode;
use crate::lattice::{InMemoryLexicon, Lattice};
use crate::mixed::{space_text, AsciiInput, AsciiPiece, Spacing};
use crate::output::{utf16_len, HansConverter, Snapshot};
use crate::score::ScoreContext;
use crate::segment::{Script, Segment};
use crate::zh::han::is_han;
use crate::zh::input::ZhInput;
use crate::zh::syllable::Syllable;

/// 這個 crate 的語意版本，供協定握手與診斷使用。
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 預設候選數。
pub const DEFAULT_MAX_CANDIDATES: usize = 5;

/// 候選視窗一頁幾項。
pub const PAGE_SIZE: usize = 10;

/// 聲調選填最多展開幾種組合；超過就維持原讀音。
const TONELESS_COMBINATIONS: usize = 256;

/// 模糊音最多展開幾種讀音組合。
const FUZZY_COMBINATIONS: usize = 16;

/// 建立 [`Session`] 所需的設定。
///
/// 資料來源以值持有：注音詞典是小型記憶體詞典，日文解碼器用 [`Arc`] 共用。
/// 沒有給詞典時，該語言仍可組字，只是轉換結果等於讀音。
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// 主要輸入模式。
    pub mode: Mode,
    /// 一次轉換保留幾個候選。
    pub max_candidates: usize,
    /// 注音詞典；`None` 時注音只做原樣輸出。
    pub zh_lexicon: Option<InMemoryLexicon>,
    /// 注音成本設定；`None` 用 [`ScoreContext::default`]。
    pub zh_ctx: Option<ScoreContext>,
    /// 日文解碼器；`None` 時日文只做原樣輸出。
    pub ja: Option<Arc<JaDecoder>>,
    /// 聲調選填：沒有聲調的音節展開成五個聲調再解碼。
    pub toneless: bool,
    /// Ctrl 標點：`Ctrl+,` 輸出全形逗號。
    pub ctrl_punct: bool,
    /// 模糊音配對；每一項是兩個可互換的注音符號。
    pub fuzzy_pairs: Vec<(char, char)>,
    /// 神經重排的固定替身設定。
    pub rerank: Option<RerankOptions>,
    /// 提交前補標點：在最後一個漢字前插入這個字元。
    pub punct_mark: Option<char>,
    /// 英文自動補字；混合模式下用 Tab 接受。
    pub english_autocomplete: bool,
    /// 送出時在漢字與 ASCII 英文字母之間補半形空格。
    pub cjk_latin_space: bool,
    /// 繁體轉簡體的轉換器；`None` 時原樣送出。
    pub simplified: Option<Arc<crate::zhconv::Converter>>,
}

/// 固定替身重排設定。
///
/// 只啟用重排路徑，不是模型本身：候選字包含 [`RerankOptions::prefer`] 任一項
/// 時，成本扣掉 `weight * prefer_score` 換算後的毫成本。
#[derive(Debug, Clone, PartialEq)]
pub struct RerankOptions {
    /// 重排權重。
    pub weight: f64,
    /// 偏好的候選字串。
    pub prefer: Vec<String>,
    /// 偏好候選的原始分數；0 代表 10。
    pub prefer_score: f64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Zh,
            max_candidates: DEFAULT_MAX_CANDIDATES,
            zh_lexicon: None,
            zh_ctx: None,
            ja: None,
            toneless: false,
            ctrl_punct: false,
            fuzzy_pairs: Vec::new(),
            rerank: None,
            punct_mark: None,
            english_autocomplete: false,
            cjk_latin_space: false,
            simplified: None,
        }
    }
}

impl SessionConfig {
    /// 用指定模式建立預設設定。
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }
}

/// 一次輸入的工作階段。
///
/// 只保留組字所需的狀態；送出後立即清空，因此呼叫端每次都用快照重畫。
pub struct Session {
    config: SessionConfig,
    zh: ZhInput,
    romaji: RomajiInput,
    lexicon: InMemoryLexicon,
    ctx: ScoreContext,
    /// 注音以外的字面按鍵（標點、英文、直接輸入的注音符號）。
    literal: String,
    /// 字面按鍵是在幾個已完成音節之後打的；用來插回轉換結果中間。
    literal_anchor: usize,
    /// 字面按鍵在目前轉換結果中的位元組位置。
    literal_offset: usize,
    /// 英文與混合模式累積的原始輸入。
    raw: String,
    candidates: Vec<Candidate>,
    highlighted: usize,
    /// 候選視窗是否由 Tab／方向鍵開啟；數字鍵只在開啟時選字。
    candidate_window: bool,
    segments: Vec<Segment>,
    /// 已轉換的部分。
    base: String,
    /// 尚未轉換的尾巴（未完成的注音或羅馬字）。
    pending: String,
    /// `Enter` 先放進這裡，下一次 [`Session::commit`] 取走。
    pending_commit: Option<String>,
    /// Tab 接受的補全結果；有值時取代組字結果。
    accepted_completion: Option<String>,
    /// 尾端補全建議。
    suggestion: Option<String>,
    /// 混合模式的英數按鍵推斷。
    ascii: AsciiInput,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(SessionConfig::default())
    }
}

impl Session {
    /// 依設定建立工作階段。
    pub fn new(config: SessionConfig) -> Self {
        let ctx = config.zh_ctx.clone().unwrap_or_default();
        let lexicon = config.zh_lexicon.clone().unwrap_or_default();
        Self {
            config,
            zh: ZhInput::standard(),
            romaji: RomajiInput::new(),
            lexicon,
            ctx,
            literal: String::new(),
            literal_anchor: 0,
            literal_offset: 0,
            raw: String::new(),
            candidates: Vec::new(),
            highlighted: 0,
            candidate_window: false,
            segments: Vec::new(),
            base: String::new(),
            pending: String::new(),
            pending_commit: None,
            accepted_completion: None,
            suggestion: None,
            ascii: AsciiInput::new(),
        }
    }

    /// 目前模式。
    pub fn mode(&self) -> Mode {
        self.config.mode
    }

    /// 目前高亮的候選索引。
    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    /// 目前候選清單。
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    /// 目前組字字串。
    pub fn preedit(&self) -> String {
        self.base_with_highlight()
    }

    /// 處理一個按鍵並回傳完整快照。
    ///
    /// `Enter` 會把組字結果放進待提交區，下一次 [`Session::commit`] 取走；
    /// `Escape` 清空組字；左右方向鍵與 Tab 在候選間移動。
    pub fn press(&mut self, key: Key) -> Snapshot {
        if key.ctrl {
            self.press_ctrl(key);
        } else {
            match key.code {
                KeyCode::Enter => self.finish_composition(),
                KeyCode::Escape | KeyCode::Delete => self.clear_composition(),
                KeyCode::Backspace => self.backspace(),
                KeyCode::Space => self.space(),
                KeyCode::Tab => {
                    let accepted = self.config.mode == Mode::Mixed
                        && self.config.english_autocomplete
                        && self.ascii.accept_suggestion();
                    if !accepted && !self.accept_suggestion() {
                        self.move_highlight(1);
                    }
                }
                KeyCode::ArrowRight => self.move_highlight(1),
                KeyCode::ArrowLeft => self.move_highlight(-1),
                KeyCode::Char(ch) => self.press_char(ch),
            }
        }
        self.rebuild();
        self.snapshot()
    }

    /// 切換到指定模式；清空目前組字。
    ///
    /// 英文模式切換（`request: {"type":"toggle"}`）由此進入。
    pub fn set_mode(&mut self, mode: Mode) -> Snapshot {
        self.clear_composition();
        self.config.mode = mode;
        self.rebuild();
        self.snapshot()
    }

    /// 在中文與英文模式之間切換。
    pub fn toggle_english(&mut self) -> Snapshot {
        let mode = if self.config.mode == Mode::En {
            Mode::Zh
        } else {
            Mode::En
        };
        self.set_mode(mode)
    }

    /// 翻到候選的第 `page` 頁；頁面大小固定 10。
    ///
    /// 目前候選清單保留全部候選，翻頁只是把高亮移到該頁第一項。
    pub fn page(&mut self, page: usize) -> Snapshot {
        let index = page.saturating_mul(PAGE_SIZE);
        if index < self.candidates.len() {
            self.highlighted = index;
            self.candidate_window = true;
        }
        self.rebuild();
        self.snapshot()
    }

    /// 目前高亮的候選字串；沒有候選時回傳 `None`。
    pub fn highlighted_text(&self) -> Option<&str> {
        self.candidates
            .get(self.highlighted)
            .map(|candidate| candidate.surface.as_str())
    }

    /// 直接指定高亮的候選；超出範圍時回到 0。
    pub fn select(&mut self, index: usize) -> Snapshot {
        if index < self.candidates.len() {
            self.highlighted = index;
            self.candidate_window = true;
        } else {
            self.highlighted = 0;
        }
        self.rebuild();
        self.snapshot()
    }

    /// 送出目前的組字結果並清空狀態。
    ///
    /// 有 `Enter` 放進來的結果時直接回傳它；否則送出目前高亮的候選或組字
    /// 字串。沒有組字內容時回傳空字串。
    pub fn commit(&mut self) -> String {
        if let Some(text) = self.pending_commit.take() {
            self.clear_composition();
            return text;
        }
        if self.config.mode == Mode::Mixed {
            self.ascii.flush();
        }
        let text = self.finalize(self.current_text());
        self.clear_composition();
        text
    }

    /// 清空所有狀態，包含尚未取走的提交結果。
    pub fn reset(&mut self) {
        self.pending_commit = None;
        self.clear_composition();
    }

    /// 目前的完整快照。
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            preedit: self.base_with_highlight(),
            cursor_utf16: self.cursor_utf16(),
            segments: self.segments.clone(),
            candidates: self.candidates.clone(),
            highlighted: self.highlighted,
        }
    }

    /// 游標位置，以 UTF-16 code unit 計算。
    pub fn cursor_utf16(&self) -> usize {
        utf16_len(&self.preedit_string())
    }

    fn max_candidates(&self) -> usize {
        self.config.max_candidates.max(1)
    }

    /// Ctrl 組合鍵：目前只處理啟用後的 `Ctrl+,`。
    fn press_ctrl(&mut self, key: Key) {
        if !self.config.ctrl_punct {
            return;
        }
        if key.code == KeyCode::Char(',') {
            self.push_literal("，");
        }
    }

    /// 把字面文字插進組字結果；記錄插入位置。
    fn push_literal(&mut self, text: &str) {
        if self.literal.is_empty() {
            self.literal_anchor = self.zh.syllables().len();
        }
        self.literal.push_str(text);
    }

    /// 接受目前的補全建議；沒有建議時回傳 `false`。
    fn accept_suggestion(&mut self) -> bool {
        let Some(text) = self.suggestion.take() else {
            return false;
        };
        self.accepted_completion = Some(text);
        self.candidate_window = false;
        true
    }

    fn press_char(&mut self, ch: char) {
        self.accepted_completion = None;
        self.suggestion = None;
        let window = self.candidate_window;
        if window && self.select_by_digit(ch) {
            self.candidate_window = false;
            return;
        }
        self.candidate_window = false;
        if !window {
            self.highlighted = 0;
        }
        match self.config.mode {
            Mode::Zh => match self.zh.press(Key::char(ch, false)) {
                crate::zh::input::ZhInputEvent::Consumed
                | crate::zh::input::ZhInputEvent::SyllableComplete(_) => {}
                crate::zh::input::ZhInputEvent::Commit(reading) => {
                    // 空白鍵在沒有未完成注音時提交；改用轉換後的結果。
                    if let Some(text) = self.convert_reading(&reading) {
                        self.pending_commit = Some(text);
                        self.zh.reset();
                    }
                }
                crate::zh::input::ZhInputEvent::Cancel => self.zh.reset(),
                crate::zh::input::ZhInputEvent::Ignored => {
                    let punct = self.zh.keyboard().punct(ch);
                    let text = punct.map(str::to_string).unwrap_or_else(|| ch.to_string());
                    self.push_literal(&text);
                }
            },
            Mode::Ja => {
                self.romaji.press(Key::char(ch, false));
            }
            Mode::En => self.raw.push(ch),
            Mode::Mixed => {
                if !self.ascii.tone(ch, &self.lexicon) {
                    self.ascii.press(ch);
                }
            }
        }
    }

    /// 數字鍵在候選視窗開啟時選字；成功時把該候選放進待提交區。
    fn select_by_digit(&mut self, ch: char) -> bool {
        if self.candidates.is_empty()
            || !self.candidate_window
            || (self.config.mode == Mode::Zh && !self.zh.pending().is_empty())
        {
            return false;
        }
        let index = match ch {
            '1'..='9' => ch as usize - '1' as usize,
            '0' => 9,
            _ => return false,
        };
        let Some(candidate) = self.candidates.get(index).cloned() else {
            return false;
        };
        self.pending_commit = Some(self.finalize(self.insert_literal(&candidate.surface)));
        self.clear_composition();
        true
    }

    fn space(&mut self) {
        match self.config.mode {
            Mode::Zh if !self.zh.pending().is_empty() => {
                self.zh.press(Key::new(KeyCode::Space, None, false));
            }
            _ if self.candidate_window && !self.candidates.is_empty() => {
                let text = self.finalize(self.current_text());
                self.pending_commit = Some(text);
                self.clear_composition();
            }
            Mode::En => self.raw.push(' '),
            Mode::Mixed => self.ascii.space(&self.lexicon),
            _ => {}
        }
    }

    fn backspace(&mut self) {
        self.candidate_window = false;
        match self.config.mode {
            Mode::Zh => {
                if self.zh.pending().is_empty()
                    && self.zh.syllables().is_empty()
                    && !self.literal.is_empty()
                {
                    self.literal.pop();
                } else {
                    self.zh.press(Key::new(KeyCode::Backspace, None, false));
                }
            }
            Mode::Ja => {
                self.romaji.backspace();
            }
            Mode::En => {
                self.raw.pop();
            }
            Mode::Mixed => {
                self.ascii.backspace();
            }
        }
    }

    fn finish_composition(&mut self) {
        if self.config.mode == Mode::Mixed {
            self.ascii.flush();
        }
        let text = match self.config.mode {
            Mode::Mixed => self.mixed_surface(),
            _ => self.current_text(),
        };
        let text = self.finalize(text);
        if !text.is_empty() {
            self.pending_commit = Some(text);
        }
        self.clear_composition();
    }

    /// 送出前套用簡體轉換與中英自動空格。
    fn finalize(&self, text: String) -> String {
        let text = self.puncted(text);
        let text = match &self.config.simplified {
            Some(converter) => converter.to_simplified(&text),
            None => text,
        };
        if self.config.cjk_latin_space {
            space_text(&text, Spacing::Always)
        } else {
            text
        }
    }

    /// 依設定在最後一個漢字前插入標點。
    fn puncted(&self, text: String) -> String {
        let Some(mark) = self.config.punct_mark else {
            return text;
        };
        let Some((index, _)) = text.char_indices().rev().find(|(_, ch)| is_han(*ch)) else {
            return text;
        };
        if index == 0 {
            return text;
        }
        let mut out = String::with_capacity(text.len() + mark.len_utf8());
        out.push_str(&text[..index]);
        out.push(mark);
        out.push_str(&text[index..]);
        out
    }

    fn move_highlight(&mut self, delta: isize) {
        let len = self.candidates.len();
        if len == 0 {
            return;
        }
        self.candidate_window = true;
        if len == 1 {
            return;
        }
        let len = len as isize;
        let mut index = (self.highlighted as isize + delta) % len;
        if index < 0 {
            index += len;
        }
        self.highlighted = index as usize;
    }

    fn clear_composition(&mut self) {
        self.zh.reset();
        self.romaji.reset();
        self.ascii.reset();
        self.literal.clear();
        self.literal_anchor = 0;
        self.literal_offset = 0;
        self.raw.clear();
        self.candidates.clear();
        self.highlighted = 0;
        self.candidate_window = false;
        self.segments.clear();
        self.base.clear();
        self.pending.clear();
        self.accepted_completion = None;
        self.suggestion = None;
    }

    /// 目前要提交的文字：接受的補全優先，其次高亮候選，最後轉換結果。
    fn current_text(&self) -> String {
        if self.config.mode == Mode::Mixed {
            return self.mixed_surface();
        }
        if let Some(text) = &self.accepted_completion {
            return format!("{}{}", self.insert_literal(text), self.pending);
        }
        if let Some(candidate) = self.candidates.get(self.highlighted) {
            return format!(
                "{}{}",
                self.insert_literal(&candidate.surface),
                self.pending
            );
        }
        self.preedit_string()
    }

    /// 高亮候選或轉換結果加上尚未轉換的尾巴。
    fn preedit_string(&self) -> String {
        format!("{}{}", self.insert_literal(&self.base), self.pending)
    }

    /// 顯示用字串：高亮候選反映在組字區。
    fn base_with_highlight(&self) -> String {
        let base = match (
            &self.accepted_completion,
            self.candidates.get(self.highlighted),
        ) {
            (Some(text), _) => text.as_str(),
            (None, Some(candidate)) => candidate.surface.as_str(),
            (None, None) => self.base.as_str(),
        };
        format!("{}{}", self.insert_literal(base), self.pending)
    }

    /// 把字面按鍵插回轉換結果的對應位置。
    fn insert_literal(&self, base: &str) -> String {
        if self.literal.is_empty() {
            return base.to_string();
        }
        let mut offset = self.literal_offset.min(base.len());
        while offset > 0 && !base.is_char_boundary(offset) {
            offset -= 1;
        }
        let mut out = String::with_capacity(base.len() + self.literal.len());
        out.push_str(&base[..offset]);
        out.push_str(&self.literal);
        out.push_str(&base[offset..]);
        out
    }

    fn rebuild(&mut self) {
        self.base.clear();
        self.pending.clear();
        self.candidates.clear();
        self.segments.clear();
        self.literal_offset = 0;
        match self.config.mode {
            Mode::Zh => self.rebuild_zh(),
            Mode::Ja => self.rebuild_ja(),
            Mode::En => self.rebuild_en(),
            Mode::Mixed => self.rebuild_mixed(),
        }
        if self.highlighted >= self.candidates.len() {
            self.highlighted = 0;
        }
    }

    fn rebuild_zh(&mut self) {
        self.pending = self.zh.pending();
        let syllables = self.zh.syllables().to_vec();
        if syllables.is_empty() {
            return;
        }
        let readings = self.zh_readings(&syllables);
        let units: Vec<&str> = readings.iter().map(String::as_str).collect();
        let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) else {
            self.base = units.join("");
            return;
        };
        let decoder = Decoder::new(&self.lexicon, &self.ctx);
        let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
        let conversion = decoder.conversion(&lattice, &segs, Mode::Zh);
        self.base = conversion.surface;
        self.segments = Self::segments_of(&segs, Mode::Zh, Script::Bopomofo);
        self.literal_offset = if self.literal_anchor == 0 {
            0
        } else {
            self.segments
                .get(self.literal_anchor - 1)
                .map(|segment| segment.end)
                .unwrap_or(self.base.len())
        };
        let reading = readings.join(" ");
        let literal = readings.join("");
        let (mut hypotheses, _) =
            decoder.nbest(&lattice, &Locks::new(), false, self.max_candidates());
        if let Some(options) = &self.config.rerank {
            Self::apply_rerank(&mut hypotheses, options);
            if let Some(best) = hypotheses.first() {
                self.base = best.text.clone();
                self.segments = Self::segments_of(&best.segments, Mode::Zh, Script::Bopomofo);
            }
        }
        let mut candidates: Vec<Candidate> = hypotheses
            .into_iter()
            .map(|hypothesis| {
                let source = if hypothesis.text == literal {
                    CandidateSource::Literal
                } else {
                    CandidateSource::Lexicon
                };
                Candidate::new(hypothesis.text, reading.clone(), hypothesis.cost, source)
            })
            .collect();
        let near = self.fuzzy_candidates(&readings);
        if !near.is_empty() {
            let at = candidates
                .iter()
                .position(|candidate| candidate.surface == literal)
                .unwrap_or(candidates.len());
            for (offset, candidate) in near.into_iter().enumerate() {
                let index = (at + offset).min(candidates.len());
                candidates.insert(index, candidate);
            }
            let mut seen = Vec::new();
            candidates.retain(|candidate| {
                if seen.contains(&candidate.surface) {
                    false
                } else {
                    seen.push(candidate.surface.clone());
                    true
                }
            });
        }
        candidates.truncate(self.max_candidates());
        self.candidates = candidates;
        let (completion, _) = decoder.complete(&lattice, &segs);
        self.suggestion = if completion.found() {
            Some(completion.entry.surface)
        } else {
            None
        };
    }

    /// 目前音節要交給詞格的讀音。
    ///
    /// 聲調選填開啟時，沒有聲調的音節展開成五個聲調，從組合中挑成本最低的
    /// 一條；組合數超過上限（長句）時維持原樣，避免指數爆炸。
    fn zh_readings(&self, syllables: &[Syllable]) -> Vec<String> {
        let exact: Vec<String> = syllables
            .iter()
            .map(|syllable| syllable.as_str().to_string())
            .collect();
        if !self.config.toneless {
            return exact;
        }
        let rows: Vec<Vec<String>> = syllables
            .iter()
            .map(|syllable| {
                if syllable.tone() != 1 {
                    return vec![syllable.as_str().to_string()];
                }
                (1..=5)
                    .map(|tone| syllable.with_tone(tone).as_str().to_string())
                    .collect()
            })
            .collect();
        let combinations: usize = rows.iter().map(Vec::len).product();
        if combinations == 0 || combinations > TONELESS_COMBINATIONS {
            return exact;
        }
        let mut best: Option<(i32, Vec<String>)> = None;
        let mut index = vec![0usize; rows.len()];
        loop {
            let readings: Vec<String> = rows
                .iter()
                .zip(index.iter())
                .map(|(row, &i)| row[i].clone())
                .collect();
            let units: Vec<&str> = readings.iter().map(String::as_str).collect();
            if let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) {
                let decoder = Decoder::new(&self.lexicon, &self.ctx);
                let (_, cost) = decoder.best(&lattice, &Locks::new(), false);
                if best.as_ref().is_none_or(|(best_cost, _)| cost < *best_cost) {
                    best = Some((cost, readings));
                }
            }
            let mut at = index.len();
            loop {
                if at == 0 {
                    return best.map(|(_, readings)| readings).unwrap_or(exact);
                }
                at -= 1;
                index[at] += 1;
                if index[at] < rows[at].len() {
                    break;
                }
                index[at] = 0;
            }
        }
    }

    /// 依固定替身設定重排：偏好字串命中時扣掉換算後的成本。
    fn apply_rerank(hypotheses: &mut [crate::decode::Hypothesis], options: &RerankOptions) {
        let score = if options.prefer_score > 0.0 {
            options.prefer_score
        } else {
            10.0
        };
        let bonus = (options.weight * score * 1000.0).round() as i32;
        hypotheses.sort_by_key(|hypothesis| {
            let preferred = options
                .prefer
                .iter()
                .any(|text| hypothesis.text.contains(text));
            if preferred {
                hypothesis.cost.saturating_sub(bonus)
            } else {
                hypothesis.cost
            }
        });
    }

    /// 模糊音候選：把已完成的音節換成近音讀音，取候選補進清單。
    fn fuzzy_candidates(&self, readings: &[String]) -> Vec<Candidate> {
        if self.config.fuzzy_pairs.is_empty() {
            return Vec::new();
        }
        let mut variants: Vec<Vec<String>> = vec![Vec::new()];
        for reading in readings {
            let mut alternatives = vec![reading.clone()];
            for &(left, right) in &self.config.fuzzy_pairs {
                for (from, to) in [(left, right), (right, left)] {
                    if reading.contains(from) {
                        let swapped: String = reading
                            .chars()
                            .map(|ch| if ch == from { to } else { ch })
                            .collect();
                        if !alternatives.contains(&swapped) {
                            alternatives.push(swapped);
                        }
                    }
                }
            }
            let mut next = Vec::new();
            for prefix in &variants {
                for alternative in &alternatives {
                    let mut variant = prefix.clone();
                    variant.push(alternative.clone());
                    next.push(variant);
                }
            }
            variants = next;
            if variants.len() > FUZZY_COMBINATIONS {
                variants.truncate(FUZZY_COMBINATIONS);
            }
        }
        let original = readings.join(" ");
        let mut out: Vec<Candidate> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for variant in variants {
            if variant.as_slice() == readings {
                continue;
            }
            let units: Vec<&str> = variant.iter().map(String::as_str).collect();
            let Ok(lattice) = Lattice::build(&units, &self.lexicon, &self.ctx) else {
                continue;
            };
            let decoder = Decoder::new(&self.lexicon, &self.ctx);
            let (hypotheses, _) =
                decoder.nbest(&lattice, &Locks::new(), false, self.max_candidates());
            for hypothesis in hypotheses {
                if hypothesis.text == variant.join("") || seen.contains(&hypothesis.text) {
                    continue;
                }
                seen.push(hypothesis.text.clone());
                out.push(Candidate::new(
                    hypothesis.text,
                    original.clone(),
                    hypothesis.cost,
                    CandidateSource::Lexicon,
                ));
            }
        }
        out.sort_by_key(|candidate| candidate.cost);
        out
    }

    fn rebuild_ja(&mut self) {
        self.pending = self.romaji.pending().to_string();
        let reading: String = self.romaji.units().iter().map(|u| u.as_str()).collect();
        if reading.is_empty() {
            return;
        }
        let Some(decoder) = &self.config.ja else {
            self.base = reading;
            return;
        };
        let results = decoder.nbest(&reading, &JaContext::default(), self.max_candidates());
        let Some(best) = results.first() else {
            self.base = reading;
            return;
        };
        self.base = best.surface.clone();
        self.segments = best.segments.clone();
        self.candidates = results
            .into_iter()
            .map(|result| {
                Candidate::new(
                    result.surface,
                    reading.clone(),
                    result.cost,
                    CandidateSource::Lexicon,
                )
            })
            .collect();
    }

    fn rebuild_en(&mut self) {
        self.base = self.raw.clone();
        if !self.raw.is_empty() {
            self.candidates.push(Candidate::new(
                self.raw.clone(),
                self.raw.clone(),
                0,
                CandidateSource::Literal,
            ));
        }
    }

    fn rebuild_mixed(&mut self) {
        if self.ascii.is_empty() {
            return;
        }
        self.base = self.mixed_surface();
        let mut at = 0usize;
        self.segments = self
            .ascii
            .pieces()
            .iter()
            .map(|piece| {
                let (text, mode, script) = match piece {
                    AsciiPiece::Zh(syllable) => {
                        (self.decode_syllable(syllable), Mode::Zh, Script::Bopomofo)
                    }
                    AsciiPiece::Literal(text) => (text.clone(), Mode::En, Script::Latin),
                };
                let start = at;
                at += text.len();
                Segment::new(start, at, mode, script)
            })
            .collect();
        if !self.ascii.pending().is_empty() {
            let pending = self.ascii.pending();
            let start = at;
            at += pending.len();
            self.segments
                .push(Segment::new(start, at, Mode::Zh, Script::Bopomofo));
        }
    }

    /// 已完成的注音音節轉成字面。
    fn decode_syllable(&self, syllable: &Syllable) -> String {
        let reading = syllable.as_str();
        let units = [reading];
        match Lattice::build(&units, &self.lexicon, &self.ctx) {
            Ok(lattice) => {
                let decoder = Decoder::new(&self.lexicon, &self.ctx);
                let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
                decoder.conversion(&lattice, &segs, Mode::Zh).surface
            }
            Err(_) => reading.to_string(),
        }
    }

    /// 混合模式的完整字面：注音片段解碼，英數照字面，待定按鍵照顯示。
    fn mixed_surface(&self) -> String {
        let mut out = String::new();
        let mut run: Vec<String> = Vec::new();
        let flush = |run: &mut Vec<String>, out: &mut String| {
            if run.is_empty() {
                return;
            }
            let units: Vec<&str> = run.iter().map(String::as_str).collect();
            match Lattice::build(&units, &self.lexicon, &self.ctx) {
                Ok(lattice) => {
                    let decoder = Decoder::new(&self.lexicon, &self.ctx);
                    let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
                    out.push_str(&decoder.conversion(&lattice, &segs, Mode::Zh).surface);
                }
                Err(_) => {
                    for unit in run.iter() {
                        out.push_str(unit);
                    }
                }
            }
            run.clear();
        };
        for piece in self.ascii.pieces() {
            match piece {
                AsciiPiece::Zh(syllable) => run.push(syllable.as_str().to_string()),
                AsciiPiece::Literal(text) => {
                    flush(&mut run, &mut out);
                    out.push_str(text);
                }
            }
        }
        flush(&mut run, &mut out);
        out.push_str(&self.ascii.display_pending());
        out
    }

    /// 把解碼片段轉成輸出字串上的位元組範圍。
    fn segments_of(segs: &[crate::decode::Seg], mode: Mode, script: Script) -> Vec<Segment> {
        let mut at = 0usize;
        segs.iter()
            .map(|seg| {
                let start = at;
                at += seg.word.len();
                Segment::new(start, at, mode, script)
            })
            .collect()
    }

    /// 把一整串讀音直接轉成最佳字面；給空白鍵提交與外部驅動使用。
    fn convert_reading(&self, reading: &str) -> Option<String> {
        let syllables = crate::zh::syllable::parse_reading(reading);
        if syllables.is_empty() {
            return None;
        }
        let units: Vec<&str> = syllables.iter().map(|s| s.as_str()).collect();
        let lattice = Lattice::build(&units, &self.lexicon, &self.ctx).ok()?;
        let decoder = Decoder::new(&self.lexicon, &self.ctx);
        let (segs, _) = decoder.best(&lattice, &Locks::new(), false);
        Some(decoder.conversion(&lattice, &segs, Mode::Zh).surface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::LexEntry;

    fn lexicon() -> InMemoryLexicon {
        let mut lex = InMemoryLexicon::new();
        for (surface, reading) in [
            ("我", "ㄨㄛˇ"),
            ("想", "ㄒㄧㄤˇ"),
            ("吃", "ㄔ"),
            ("風", "ㄈㄥ"),
            ("景", "ㄐㄧㄥˇ"),
            ("風景", "ㄈㄥ ㄐㄧㄥˇ"),
            ("麻婆豆腐", "ㄇㄚˊ ㄆㄛˊ ㄉㄡˋ ㄈㄨˇ"),
            ("城市", "ㄔㄥˊ ㄕˋ"),
            ("程式", "ㄔㄥˊ ㄕˋ"),
            ("宗", "ㄗㄨㄥ"),
            ("中", "ㄓㄨㄥ"),
            ("麻", "ㄇㄚˊ"),
            ("煩", "ㄈㄢˊ"),
            ("窩", "ㄨㄛ"),
            ("龍", "ㄌㄨㄥˊ"),
            ("風", "ㄈㄥ"),
            ("學", "ㄒㄩㄝˊ"),
            ("國", "ㄍㄨㄛˊ"),
        ] {
            lex.push(LexEntry::new(surface, reading, 0, 0));
        }
        lex
    }

    fn session() -> Session {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        Session::new(config)
    }

    fn type_syllable(session: &mut Session, reading: &str) {
        let keyboard = crate::zh::keyboard::Keyboard::standard();
        for syllable in crate::zh::syllable::parse_reading(reading) {
            let keys = keyboard.keys(&syllable).expect("音節應該有對應按鍵");
            for ch in keys.chars() {
                session.press(Key::char(ch, false));
            }
        }
    }

    #[test]
    fn test_commit_exactly_once() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let snap = s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風景");
        assert_eq!(snap.preedit, "");
        assert!(snap.candidates.is_empty());
        // 已無組字時再按 Enter 不得重複提交。
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "");
    }

    #[test]
    fn test_session_segments_follow_zhuyin_path_and_cursor() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        let snap = s.press(Key::char('t', false)); // ㄔ
        assert_eq!(snap.preedit, "我想吃");
        assert_eq!(snap.cursor_utf16, 3);
        assert_eq!(snap.segments.len(), 3);
        let words: Vec<&str> = ["我", "想", "吃"].to_vec();
        for (segment, word) in snap.segments.iter().zip(words) {
            assert_eq!(&snap.preedit[segment.start..segment.end], word);
            assert_eq!(segment.mode, Mode::Zh);
        }
    }

    #[test]
    fn test_long_word_stays_merged() {
        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let snap = s.snapshot();
        assert_eq!(snap.preedit, "風景");
        assert_eq!(snap.segments.len(), 1);
    }

    #[test]
    fn test_backspace_removes_last_syllable() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        s.press(Key::new(KeyCode::Backspace, None, false));
        assert_eq!(s.snapshot().preedit, "我");
    }

    #[test]
    fn test_escape_clears_composition() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        let snap = s.press(Key::new(KeyCode::Escape, None, false));
        assert_eq!(snap.preedit, "");
        assert_eq!(s.commit(), "");
    }

    #[test]
    fn test_candidates_are_ranked_and_tab_moves_highlight() {
        let mut s = session();
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let snap = s.snapshot();
        assert!(!snap.candidates.is_empty());
        let first = snap.candidates[0].surface.clone();
        assert_eq!(snap.preedit, first);
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.highlighted(), 1 % snap.candidates.len().max(1));
    }

    #[test]
    fn test_digit_selects_candidate() {
        let mut s = session();
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        let candidates = s.candidates().to_vec();
        s.press(Key::new(KeyCode::ArrowRight, None, false));
        s.press(Key::char('2', false));
        assert_eq!(s.commit(), candidates[1].surface);
    }

    #[test]
    fn test_without_lexicon_commits_reading() {
        let mut s = Session::new(SessionConfig::new(Mode::Zh));
        let keyboard = crate::zh::keyboard::Keyboard::standard();
        for ch in keyboard
            .keys(&crate::zh::syllable::Syllable::parse("ㄇㄚ").unwrap())
            .unwrap()
            .chars()
        {
            s.press(Key::char(ch, false));
        }
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "ㄇㄚ");
    }

    #[test]
    fn test_ctrl_comma_inserts_fullwidth_comma() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.ctrl_punct = true;
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::Char(','), Some(','), false).with_ctrl());
        type_syllable(&mut s, "ㄒㄧㄤˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我，想");
    }

    #[test]
    fn test_punctuator_inserts_mark_before_last_han() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.punct_mark = Some('，');
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄈㄥ");
        type_syllable(&mut s, "ㄐㄧㄥˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風，景");
    }

    #[test]
    fn test_rerank_prefers_configured_candidate() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.rerank = Some(RerankOptions {
            weight: 1.0,
            prefer: vec!["城市".to_string()],
            prefer_score: 10.0,
        });
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄔㄥˊ");
        type_syllable(&mut s, "ㄕˋ");
        assert_eq!(s.candidates()[0].surface, "城市");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "城市");
    }

    #[test]
    fn test_toneless_readings_expand_to_cheapest_tones() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.toneless = true;
        let mut s = Session::new(config);
        for keys in ["ji", "vu;"] {
            for ch in keys.chars() {
                s.press(Key::char(ch, false));
            }
            s.press(Key::new(KeyCode::Space, None, false));
        }
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我想");
    }

    #[test]
    fn test_fuzzy_candidates_appear_before_literal() {
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.fuzzy_pairs = vec![('ㄗ', 'ㄓ')];
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄗㄨㄥ");
        let surfaces: Vec<&str> = s
            .candidates()
            .iter()
            .map(|candidate| candidate.surface.as_str())
            .collect();
        assert_eq!(surfaces, ["宗", "中", "ㄗㄨㄥ"]);
    }

    #[test]
    fn test_page_moves_highlight_to_page_start() {
        let mut lexicon = InMemoryLexicon::new();
        for (index, surface) in ["甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬", "癸"]
            .iter()
            .chain(["子", "丑"].iter())
            .enumerate()
        {
            lexicon.push(LexEntry::new(*surface, "ㄐㄧ", index as i32, 0));
        }
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon);
        config.max_candidates = 20;
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄐㄧ");
        s.page(1);
        assert_eq!(s.highlighted_text(), Some("子"));
    }

    fn mixed_session() -> Session {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(lexicon());
        Session::new(config)
    }

    fn type_keys(session: &mut Session, keys: &str) {
        for ch in keys.chars() {
            session.press(Key::char(ch, false));
        }
    }

    /// 對照 `TestAutomaticMixedChineseEnglishChinese`：英數鍵在注音組字中
    /// 由聲調鍵或空白決定是中文還是英文。
    #[test]
    fn test_automatic_mixed_chinese_english_chinese() {
        for (keys, want) in [
            ("ji3hellot ", "我hello吃"),
            ("helloji3", "hello我"),
            ("t ", "吃"),
            ("a86z06", "麻煩"),
            ("ji3hello worldji3", "我hello world我"),
        ] {
            let mut s = mixed_session();
            type_keys(&mut s, keys);
            let snap = s.press(Key::new(KeyCode::Enter, None, false));
            assert_eq!(s.commit(), want, "{keys:?}");
            assert_eq!(snap.preedit, "");
        }
    }

    /// 對照 `TestMixedCompleteEnglishWordKeepsSpace`：完整英文詞接空白保留
    /// 空白，組得成注音的字尾讓給中文。
    #[test]
    fn test_mixed_complete_english_word_keeps_space() {
        for (keys, want) in [
            ("hello ", "hello "),
            ("helloji ", "hello窩"),
            ("ji3hello ji3", "我hello 我"),
        ] {
            let mut s = mixed_session();
            type_keys(&mut s, keys);
            assert_eq!(s.snapshot().preedit, want, "{keys:?}");
            s.press(Key::new(KeyCode::Enter, None, false));
            assert_eq!(s.commit(), want, "{keys:?}");
        }
    }

    #[test]
    fn test_mixed_english_suggestion_and_completion() {
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(lexicon());
        config.english_autocomplete = true;
        let mut s = Session::new(config);
        type_keys(&mut s, "ji3hellp");
        assert_eq!(s.snapshot().preedit, "我hellp");
        s.press(Key::new(KeyCode::Tab, None, false));
        assert_eq!(s.snapshot().preedit, "我hello");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "我hello");
    }

    /// 對照 `TestCJKLatinSpaceInMixedInput`：組字區照打的顯示，送出時才補空格。
    #[test]
    fn test_cjk_latin_space_in_mixed_input() {
        for (keys, off, on) in [
            ("ji3hellot ", "我hello吃", "我 hello 吃"),
            ("ji3hello su3", "我hello 我", "我 hello 我"),
        ] {
            for space in [false, true] {
                let mut config = SessionConfig::new(Mode::Mixed);
                config.zh_lexicon = Some(lexicon());
                config.cjk_latin_space = space;
                let mut s = Session::new(config);
                type_keys(&mut s, keys);
                assert_eq!(s.snapshot().preedit, off, "{keys:?}");
                s.press(Key::new(KeyCode::Enter, None, false));
                assert_eq!(s.commit(), if space { on } else { off }, "{keys:?}");
            }
        }
    }

    #[test]
    fn test_simplified_output_converts_commit_only() {
        let path = jc_testdata::require_data!("opencc");
        let converter = crate::zhconv::tw2s_from_dir(&path).expect("讀取 OpenCC 字典");
        let mut config = SessionConfig::new(Mode::Zh);
        config.zh_lexicon = Some(lexicon());
        config.simplified = Some(Arc::new(converter));
        let mut s = Session::new(config);
        type_syllable(&mut s, "ㄌㄨㄥˊ");
        assert_eq!(s.snapshot().preedit, "龍");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "龙");

        let mut s = session();
        type_syllable(&mut s, "ㄈㄥ");
        s.press(Key::new(KeyCode::Enter, None, false));
        assert_eq!(s.commit(), "風");
    }

    #[test]
    fn test_reset_clears_pending_commit() {
        let mut s = session();
        type_syllable(&mut s, "ㄨㄛˇ");
        s.press(Key::new(KeyCode::Enter, None, false));
        s.reset();
        assert_eq!(s.commit(), "");
        assert_eq!(s.snapshot().preedit, "");
    }
}
