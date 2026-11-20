//! 注音輸入：累積按鍵的狀態機，以及交給解碼器的音節格。
//!
//! 兩層契約：
//! - [`ZhInput`] 把實體按鍵累積成音節，只負責注音前端；不是注音鍵的按鍵原樣
//!   交回呼叫端。
//! - [`Frontend`] 把使用者打的音節序列展開成音節格，帶成本的假設由其他前端
//!   （容錯、部分注音、語音）補上。

use std::sync::Arc;

use crate::key::{Key, KeyCode};
use crate::reading::Unit;
use crate::zh::keyboard::Keyboard;
use crate::zh::syllable::{class_of, Class, Composer, Syllable, TONE1};

/// 某個位置的一個讀音假設。`cost` 為負對數機率，0 表示確定。
#[derive(Debug, Clone, PartialEq)]
pub struct Hyp {
    /// 核心讀音單位；注音的一個音節就是一個單位。
    pub syl: Unit,
    /// 這個假設的成本。
    pub cost: f64,
    /// 這個假設是沒打聲調的音節展開出來的其中一個聲調。
    ///
    /// 解碼器據此裁切這些位置的弧數；全部打了聲調時沒有這種假設。
    pub untoned: bool,
}

/// 音節格：每個元素是一個位置的所有假設。
pub type Lattice = Vec<Vec<Hyp>>;

/// 把使用者輸入的音節序列展開成音節格。
pub trait Frontend {
    /// 展開音節序列。
    fn expand(&self, syls: &[Syllable]) -> Lattice;
}

/// 精確通道：使用者打什麼就是什麼。
///
/// 沒打聲調的音節展開成五個聲調，成本都是 0，由語言模型依前後文決定；
/// 其他音節只有自己。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Exact;

impl Frontend for Exact {
    fn expand(&self, syls: &[Syllable]) -> Lattice {
        syls.iter()
            .map(|s| {
                if s.untoned() {
                    s.tones()
                        .into_iter()
                        .map(|t| Hyp {
                            syl: t.reading_unit(),
                            cost: 0.0,
                            untoned: true,
                        })
                        .collect()
                } else {
                    vec![Hyp {
                        syl: s.reading_unit(),
                        cost: 0.0,
                        untoned: false,
                    }]
                }
            })
            .collect()
    }
}

/// 一次按鍵對注音前端的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZhInputEvent {
    /// 按鍵已經吃進組音狀態。
    Consumed,
    /// 音節完成（打了聲調）。
    SyllableComplete(Syllable),
    /// 使用者提交了整串讀音；內容是空白分隔的讀音鍵。
    Commit(String),
    /// 使用者取消，狀態已清空。
    Cancel,
    /// 不是注音鍵，呼叫端自己處理。
    Ignored,
}

/// 聲調閘門：決定剛按下的聲調要不要收。
///
/// 參數是按下前的暫存注音與加上聲調後的音節；回傳 `true` 表示擋下，注音
/// 留在暫存不動。預設是 [`allow_all_tones`]，行為與沒有閘門時相同。
///
/// 用 [`Arc`] 是為了讓工作階段把「讀音有沒有字」的詞典查詢包進閘門；閘門
/// 本身不可變，需要的重試狀態由實作自行處理。
pub type ToneGate = Arc<dyn Fn(&str, &Syllable) -> bool + Send + Sync>;

/// 預設的聲調閘門：全部放行。
pub fn allow_all_tones(_pending: &str, _syllable: &Syllable) -> bool {
    false
}

/// 注音輸入的累積狀態機。
///
/// 吃的是語義按鍵（[`Key`]），不是平台鍵碼。注音鍵由 [`Keyboard`] 對照，
/// 標點與其他鍵原樣交回呼叫端。
#[derive(Clone)]
pub struct ZhInput {
    keyboard: Keyboard,
    composer: Composer,
    syllables: Vec<Syllable>,
    cursor: usize,
    tone_blocked: bool,
    tone_gate: ToneGate,
}

/// 只列出組字狀態；聲調閘門是策略，不印出內容。
impl std::fmt::Debug for ZhInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZhInput")
            .field("keyboard", &self.keyboard)
            .field("composer", &self.composer)
            .field("syllables", &self.syllables)
            .field("cursor", &self.cursor)
            .field("tone_blocked", &self.tone_blocked)
            .finish()
    }
}

/// 只比較組字狀態；聲調閘門是策略，不參與相等判斷。
impl PartialEq for ZhInput {
    fn eq(&self, other: &Self) -> bool {
        self.keyboard == other.keyboard
            && self.composer == other.composer
            && self.syllables == other.syllables
            && self.cursor == other.cursor
            && self.tone_blocked == other.tone_blocked
    }
}

impl Eq for ZhInput {}

impl Default for ZhInput {
    fn default() -> Self {
        Self::standard()
    }
}

impl ZhInput {
    /// 用指定的鍵盤排列建立狀態機。
    pub fn new(keyboard: Keyboard) -> Self {
        Self {
            keyboard,
            composer: Composer::new(),
            syllables: Vec::new(),
            cursor: 0,
            tone_blocked: false,
            tone_gate: Arc::new(allow_all_tones),
        }
    }

    /// 用標準（大千）排列建立狀態機。
    pub fn standard() -> Self {
        Self::new(Keyboard::standard())
    }

    /// 目前使用的鍵盤排列。
    pub fn keyboard(&self) -> Keyboard {
        self.keyboard
    }

    /// 已經完成的音節。
    pub fn syllables(&self) -> &[Syllable] {
        &self.syllables
    }

    /// 尚未完成的注音，依正規順序。
    pub fn pending(&self) -> String {
        self.composer.string()
    }

    /// 組字區的顯示內容：已完成音節的注音加上尚未完成的注音。
    pub fn preedit(&self) -> String {
        let mut out = String::new();
        for s in &self.syllables {
            out.push_str(s.as_str());
        }
        out.push_str(&self.composer.string());
        out
    }

    /// 已完成音節的讀音鍵，空白分隔。
    pub fn reading(&self) -> String {
        crate::zh::syllable::key(&self.syllables)
    }

    /// 組字區的格子數：每個完成音節一格，未完成的暫存注音算最後一格。
    pub fn cells(&self) -> usize {
        self.syllables.len() + usize::from(!self.composer.empty())
    }

    /// 組字游標的位置（格）。
    ///
    /// `0` 是最前面，最大值是 [`ZhInput::cells`]。這一輪只是給外殼與後續
    /// 移植用的位置，不影響組字結果。
    pub fn cursor(&self) -> usize {
        self.cursor.min(self.cells())
    }

    /// 把組字游標移動 `delta` 格，超出範圍時夾在邊界。
    ///
    /// 回傳游標是否真的移動。任何會改變組字的按鍵都會把游標放回結尾。
    pub fn cursor_move(&mut self, delta: isize) -> bool {
        let before = self.cursor();
        let target = (before as isize).saturating_add(delta);
        self.cursor = target.clamp(0, self.cells() as isize) as usize;
        self.cursor != before
    }

    /// 最近一次按鍵是否擋下聲調。
    ///
    /// 擋下時聲調沒有收進暫存，注音停在原處；下一個按鍵會清掉這個旗標。
    pub fn tone_blocked(&self) -> bool {
        self.tone_blocked
    }

    /// 安裝聲調閘門；傳 [`allow_all_tones`] 可以恢復全部放行。
    pub fn set_tone_gate<G>(&mut self, gate: G)
    where
        G: Fn(&str, &Syllable) -> bool + Send + Sync + 'static,
    {
        self.tone_gate = Arc::new(gate);
    }

    /// 清空狀態。
    pub fn reset(&mut self) {
        self.composer.reset();
        self.syllables.clear();
        self.cursor = 0;
        self.tone_blocked = false;
    }

    /// 處理一個按鍵。
    pub fn press(&mut self, key: Key) -> ZhInputEvent {
        self.tone_blocked = false;
        let event = match key.code {
            KeyCode::Enter => self.press_enter(),
            KeyCode::Escape => self.press_escape(),
            KeyCode::Backspace => self.press_backspace(),
            KeyCode::Space => self.press_space(),
            KeyCode::Char(ch) => self.press_char(key.text.unwrap_or(ch)),
            _ => ZhInputEvent::Ignored,
        };
        if event != ZhInputEvent::Ignored {
            self.cursor = self.cells();
        }
        event
    }

    fn press_char(&mut self, ch: char) -> ZhInputEvent {
        let Some(sym) = self.keyboard.symbol(ch) else {
            return ZhInputEvent::Ignored;
        };
        if class_of(sym) == Class::Tone {
            // 先在副本上完成音節，讓閘門看得到結果；擋下時暫存完全不動。
            let mut next = self.composer.clone();
            return match next.add(sym) {
                Some(syl) => {
                    if (self.tone_gate)(&self.composer.string(), &syl) {
                        self.tone_blocked = true;
                        return ZhInputEvent::Consumed;
                    }
                    self.composer = next;
                    self.syllables.push(syl.clone());
                    ZhInputEvent::SyllableComplete(syl)
                }
                None => ZhInputEvent::Ignored,
            };
        }
        self.composer.add(sym);
        ZhInputEvent::Consumed
    }

    fn press_space(&mut self) -> ZhInputEvent {
        if !self.composer.empty() {
            return match self.composer.add(TONE1) {
                Some(syl) => {
                    self.syllables.push(syl.clone());
                    ZhInputEvent::SyllableComplete(syl)
                }
                None => ZhInputEvent::Ignored,
            };
        }
        self.commit()
    }

    fn press_enter(&mut self) -> ZhInputEvent {
        if !self.composer.empty() {
            if let Some(syl) = self.composer.add(TONE1) {
                self.syllables.push(syl);
            }
        }
        self.commit()
    }

    fn press_escape(&mut self) -> ZhInputEvent {
        if self.composer.empty() && self.syllables.is_empty() {
            return ZhInputEvent::Ignored;
        }
        self.reset();
        ZhInputEvent::Cancel
    }

    fn press_backspace(&mut self) -> ZhInputEvent {
        if self.composer.backspace() {
            return ZhInputEvent::Consumed;
        }
        if self.syllables.pop().is_some() {
            return ZhInputEvent::Consumed;
        }
        ZhInputEvent::Cancel
    }

    fn commit(&mut self) -> ZhInputEvent {
        if self.syllables.is_empty() {
            return ZhInputEvent::Ignored;
        }
        let key = self.reading();
        self.syllables.clear();
        ZhInputEvent::Commit(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading;
    use crate::zh::syllable::UNTONED_MARK;

    fn press_all(input: &mut ZhInput, keys: &str) -> Vec<ZhInputEvent> {
        keys.chars()
            .map(|ch| input.press(Key::char(ch, false)))
            .collect()
    }

    #[test]
    fn test_hyp_uses_reading_unit() {
        let lat: Lattice = vec![
            vec![Hyp {
                syl: Unit::new("き"),
                cost: 0.0,
                untoned: false,
            }],
            vec![Hyp {
                syl: Unit::new("ょ"),
                cost: 0.0,
                untoned: false,
            }],
            vec![Hyp {
                syl: Unit::new("う"),
                cost: 0.0,
                untoned: false,
            }],
        ];
        let seq: Vec<Unit> = lat.iter().map(|hs| hs[0].syl.clone()).collect();
        assert_eq!(reading::key(&seq), "き ょ う");
    }

    #[test]
    fn test_exact_produces_reading_units() {
        let syls = crate::zh::syllable::parse_reading("ㄓㄨˋ ㄧ");
        let lat = Exact.expand(&syls);
        assert_eq!(lat.len(), 2);
        assert_eq!(lat[0].len(), 1);
        assert_eq!(lat[1].len(), 1);
        assert_eq!(
            reading::key(&[lat[0][0].syl.clone(), lat[1][0].syl.clone()]),
            "ㄓㄨˋ ㄧ"
        );
        assert_eq!(lat[0][0].syl.as_str(), "ㄓㄨˋ");
        assert_eq!(lat[0][0].cost, 0.0);
        assert!(!lat[0][0].untoned);
    }

    #[test]
    fn exact_expands_untoned_syllables_to_five_tones() {
        let syl = Syllable::from_raw(format!("ㄋㄧ{UNTONED_MARK}"));
        let lat = Exact.expand(&[syl]);
        let got: Vec<&str> = lat[0].iter().map(|h| h.syl.as_str()).collect();
        assert_eq!(got, ["ㄋㄧ", "ㄋㄧˊ", "ㄋㄧˇ", "ㄋㄧˋ", "ㄋㄧ˙"]);
        assert!(lat[0].iter().all(|h| h.untoned && h.cost == 0.0));
    }

    #[test]
    fn press_tones_complete_a_syllable() {
        let mut input = ZhInput::standard();
        assert_eq!(
            press_all(&mut input, "ru/"),
            vec![
                ZhInputEvent::Consumed,
                ZhInputEvent::Consumed,
                ZhInputEvent::Consumed
            ]
        );
        assert_eq!(input.pending(), "ㄐㄧㄥ");
        assert_eq!(
            input.press(Key::char('3', false)),
            ZhInputEvent::SyllableComplete(Syllable::from_raw("ㄐㄧㄥˇ"))
        );
        assert_eq!(input.pending(), "");
        assert_eq!(input.reading(), "ㄐㄧㄥˇ");
        assert_eq!(input.preedit(), "ㄐㄧㄥˇ");
    }

    #[test]
    fn press_supports_empty_initial_and_er() {
        let mut input = ZhInput::standard();
        // ㄧ + 二聲。
        assert_eq!(input.press(Key::char('u', false)), ZhInputEvent::Consumed);
        assert_eq!(
            input.press(Key::char('6', false)),
            ZhInputEvent::SyllableComplete(Syllable::from_raw("ㄧˊ"))
        );
        // ㄦ + 四聲。
        input.press(Key::char('-', false));
        assert_eq!(
            input.press(Key::char('4', false)),
            ZhInputEvent::SyllableComplete(Syllable::from_raw("ㄦˋ"))
        );
        assert_eq!(input.reading(), "ㄧˊ ㄦˋ");
    }

    #[test]
    fn press_supports_neutral_tone() {
        let mut input = ZhInput::standard();
        press_all(&mut input, "ak");
        assert_eq!(
            input.press(Key::char('7', false)),
            ZhInputEvent::SyllableComplete(Syllable::from_raw("ㄇㄜ˙"))
        );
    }

    #[test]
    fn space_completes_then_commits() {
        let mut input = ZhInput::standard();
        press_all(&mut input, "5j");
        assert_eq!(
            input.press(Key::new(KeyCode::Space, None, false)),
            ZhInputEvent::SyllableComplete(Syllable::from_raw("ㄓㄨ"))
        );
        assert_eq!(
            input.press(Key::new(KeyCode::Space, None, false)),
            ZhInputEvent::Commit("ㄓㄨ".to_string())
        );
        assert_eq!(
            input.press(Key::new(KeyCode::Space, None, false)),
            ZhInputEvent::Ignored
        );
    }

    #[test]
    fn enter_flushes_pending_then_commits() {
        let mut input = ZhInput::standard();
        press_all(&mut input, "5j/4u");
        assert_eq!(
            input.press(Key::new(KeyCode::Enter, None, false)),
            ZhInputEvent::Commit("ㄓㄨㄥˋ ㄧ".to_string())
        );
        assert_eq!(input.reading(), "");
    }

    #[test]
    fn escape_cancels_everything() {
        let mut input = ZhInput::standard();
        press_all(&mut input, "5j");
        assert_eq!(
            input.press(Key::new(KeyCode::Escape, None, false)),
            ZhInputEvent::Cancel
        );
        assert_eq!(input.preedit(), "");
        assert_eq!(
            input.press(Key::new(KeyCode::Escape, None, false)),
            ZhInputEvent::Ignored
        );
    }

    #[test]
    fn backspace_follows_display_order_then_pops_syllables() {
        let mut input = ZhInput::standard();
        press_all(&mut input, "j05");
        assert_eq!(input.pending(), "ㄓㄨㄢ");
        assert_eq!(
            input.press(Key::new(KeyCode::Backspace, None, false)),
            ZhInputEvent::Consumed
        );
        assert_eq!(input.pending(), "ㄓㄨ");
        assert_eq!(
            input.press(Key::new(KeyCode::Backspace, None, false)),
            ZhInputEvent::Consumed
        );
        assert_eq!(input.pending(), "ㄓ");
        assert_eq!(
            input.press(Key::new(KeyCode::Backspace, None, false)),
            ZhInputEvent::Consumed
        );
        assert_eq!(input.pending(), "");

        press_all(&mut input, "y3");
        assert_eq!(input.reading(), "ㄗˇ");
        assert_eq!(
            input.press(Key::new(KeyCode::Backspace, None, false)),
            ZhInputEvent::Consumed
        );
        assert_eq!(input.reading(), "");
        assert_eq!(
            input.press(Key::new(KeyCode::Backspace, None, false)),
            ZhInputEvent::Cancel
        );
    }

    #[test]
    fn non_zhuyin_keys_are_ignored() {
        let mut input = ZhInput::standard();
        assert_eq!(
            input.press(Key::char('<', true)),
            ZhInputEvent::Ignored,
            "標點由呼叫端處理"
        );
        assert_eq!(
            input.press(Key::new(KeyCode::Tab, None, false)),
            ZhInputEvent::Ignored
        );
        assert_eq!(
            input.press(Key::new(KeyCode::ArrowLeft, None, false)),
            ZhInputEvent::Ignored
        );
        assert_eq!(input.preedit(), "");
    }

    #[test]
    fn tone_without_phonetic_is_ignored() {
        let mut input = ZhInput::standard();
        assert_eq!(
            input.press(Key::char('3', false)),
            ZhInputEvent::Ignored,
            "空組音器不接受聲調"
        );
    }

    #[test]
    fn cursor_move_starts_at_end_and_clamps() {
        let mut input = ZhInput::standard();
        assert_eq!(input.cells(), 0);
        assert_eq!(input.cursor(), 0);
        assert!(!input.cursor_move(-1), "空格數沒有可移動的游標");

        press_all(&mut input, "5j");
        assert_eq!(input.cells(), 1, "未完成的 ㄓㄨ 算一格");
        assert_eq!(input.cursor(), 1);
        assert!(input.cursor_move(-1));
        assert_eq!(input.cursor(), 0);
        assert!(!input.cursor_move(-1), "已在最前面");
        assert!(input.cursor_move(3));
        assert_eq!(input.cursor(), 1, "往後夾在結尾");

        assert_eq!(
            input.press(Key::char('3', false)),
            ZhInputEvent::SyllableComplete(Syllable::from_raw("ㄓㄨˇ"))
        );
        assert_eq!(input.cells(), 1);
        assert_eq!(input.cursor(), 1, "按鍵後游標回到結尾");
    }

    #[test]
    fn cursor_counts_completed_syllables_and_pending() {
        let mut input = ZhInput::standard();
        press_all(&mut input, "5j3u");
        assert_eq!(input.cells(), 2, "一個完成音節加一個暫存音節");
        assert_eq!(input.cursor(), 2);
        input.press(Key::new(KeyCode::Escape, None, false));
        assert_eq!(input.cells(), 0);
        assert_eq!(input.cursor(), 0);
    }

    fn block_third_tone(pending: &str, syl: &Syllable) -> bool {
        pending == "ㄓㄨ" && syl.as_str() == "ㄓㄨˇ"
    }

    #[test]
    fn tone_gate_can_block_a_tone() {
        let mut input = ZhInput::standard();
        input.set_tone_gate(block_third_tone);
        press_all(&mut input, "5j");
        assert!(!input.tone_blocked());

        assert_eq!(input.press(Key::char('3', false)), ZhInputEvent::Consumed);
        assert!(input.tone_blocked(), "聲調被擋下");
        assert_eq!(input.pending(), "ㄓㄨ", "擋下後注音留在暫存");
        assert_eq!(input.reading(), "");
        assert_eq!(input.preedit(), "ㄓㄨ");

        assert_eq!(input.press(Key::char('u', false)), ZhInputEvent::Consumed);
        assert!(!input.tone_blocked(), "下一個按鍵清掉旗標");
    }

    #[test]
    fn tone_gate_lets_other_tones_through() {
        let mut input = ZhInput::standard();
        input.set_tone_gate(block_third_tone);
        press_all(&mut input, "5j");
        assert_eq!(
            input.press(Key::char('4', false)),
            ZhInputEvent::SyllableComplete(Syllable::from_raw("ㄓㄨˋ"))
        );
        assert!(!input.tone_blocked());
    }

    #[test]
    fn default_tone_gate_allows_everything() {
        let mut input = ZhInput::standard();
        assert_eq!(
            input.press(Key::char('3', false)),
            ZhInputEvent::Ignored,
            "沒有暫存注音時，預設閘門不放行也不會標記"
        );
        assert!(!input.tone_blocked());

        input.set_tone_gate(allow_all_tones);
        press_all(&mut input, "5j");
        assert_eq!(
            input.press(Key::char('3', false)),
            ZhInputEvent::SyllableComplete(Syllable::from_raw("ㄓㄨˇ"))
        );
        assert!(!input.tone_blocked());
    }
}
