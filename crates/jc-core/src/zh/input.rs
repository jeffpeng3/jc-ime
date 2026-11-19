//! 注音輸入：累積按鍵的狀態機，以及交給解碼器的音節格。
//!
//! 兩層契約：
//! - [`ZhInput`] 把實體按鍵累積成音節，只負責注音前端；不是注音鍵的按鍵原樣
//!   交回呼叫端。
//! - [`Frontend`] 把使用者打的音節序列展開成音節格，帶成本的假設由其他前端
//!   （容錯、部分注音、語音）補上。

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

/// 注音輸入的累積狀態機。
///
/// 吃的是語義按鍵（[`Key`]），不是平台鍵碼。注音鍵由 [`Keyboard`] 對照，
/// 標點與其他鍵原樣交回呼叫端。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZhInput {
    keyboard: Keyboard,
    composer: Composer,
    syllables: Vec<Syllable>,
}

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

    /// 清空狀態。
    pub fn reset(&mut self) {
        self.composer.reset();
        self.syllables.clear();
    }

    /// 處理一個按鍵。
    pub fn press(&mut self, key: Key) -> ZhInputEvent {
        match key.code {
            KeyCode::Enter => self.press_enter(),
            KeyCode::Escape => self.press_escape(),
            KeyCode::Backspace => self.press_backspace(),
            KeyCode::Space => self.press_space(),
            KeyCode::Char(ch) => self.press_char(key.text.unwrap_or(ch)),
            _ => ZhInputEvent::Ignored,
        }
    }

    fn press_char(&mut self, ch: char) -> ZhInputEvent {
        let Some(sym) = self.keyboard.symbol(ch) else {
            return ZhInputEvent::Ignored;
        };
        if class_of(sym) == Class::Tone {
            return match self.composer.add(sym) {
                Some(syl) => {
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
}
