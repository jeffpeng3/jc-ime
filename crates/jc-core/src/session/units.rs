//! 顯示字元 provenance。
//!
//! 候選、刪除與未來的記憶都以「一個顯示字元」為單位，不再用整段字面
//! span。每個 [`DisplayUnit`] 記住畫面上的那一個字元、產生它的原始按鍵、
//! 讀音、語言與所有權。`units.surface` 串接起來必須等於 `preedit`。
//!
//! `keys` 是正規化後的字串：注音是標準鍵盤按鍵（`ji3`），日文是羅馬字
//! （`shi`），英數與標點是字元本身。`meta` 說明這個單位是怎麼來的，讓
//! 後續規則不必再从字面猜。

use crate::composition::{CompSegment, Owner};
use crate::lang::Lang;
use crate::zh::keyboard::Keyboard;
use crate::zh::syllable::{class_of, parse_reading, Class};

use super::Session;

/// 顯示字元的來源種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayUnitMeta {
    /// 注音按鍵切出來的漢字或注音字面。
    Zh,
    /// 注音符號本身（未轉換的 ㄨ、ㄛ、ˇ）。
    Zhuyin,
    /// かな。
    Kana,
    /// 漢字（注音或日文轉換結果）。
    Han,
    /// 英數、標點與其他字面。
    #[default]
    Literal,
    /// 無法判定來源；`keys` 等於字面本身。
    Unknown,
}

/// 一個顯示字元的來源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayUnit {
    /// 畫面上的那一個字元。
    pub surface: String,
    /// 產生這個字元的正規化原始按鍵。
    pub keys: String,
    /// 讀音；字面沒有讀音時為 `None`。
    pub reading: Option<String>,
    /// 語言。
    pub lang: Lang,
    /// 所有權。
    pub owner: Owner,
    /// 來源種類。
    pub meta: DisplayUnitMeta,
}

impl DisplayUnit {
    fn new(
        surface: impl Into<String>,
        keys: impl Into<String>,
        reading: Option<String>,
        lang: Lang,
        owner: Owner,
        meta: DisplayUnitMeta,
    ) -> Self {
        Self {
            surface: surface.into(),
            keys: keys.into(),
            reading,
            lang,
            owner,
            meta,
        }
    }
}

/// 固定下來的 provenance：units 與平行的按鍵範圍。
pub(super) type UnitSnapshot = (Vec<DisplayUnit>, Vec<(usize, usize)>);

/// 建立 unit 時暫存的顯示位置與按鍵範圍。
struct Built {
    rune: usize,
    unit: DisplayUnit,
    /// `AsciiInput::typed()` 的位元組範圍；沒有對應按鍵時是 `(0, 0)`。
    typed: (usize, usize),
}

impl Session {
    /// 重新計算顯示字元 provenance。
    pub(super) fn sync_units(&mut self) {
        if let Some((units, spans)) = self.unit_override.clone() {
            let joined: String = units.iter().map(|unit| unit.surface.as_str()).collect();
            if joined == self.composed_text() {
                self.units = units;
                self.unit_spans = spans;
                return;
            }
            self.unit_override = None;
        }
        let (units, spans) = self.compute_units();
        self.units = units;
        self.unit_spans = spans;
        #[cfg(test)]
        {
            let joined: String = self
                .units
                .iter()
                .map(|unit| unit.surface.as_str())
                .collect();
            debug_assert_eq!(
                joined,
                self.composed_text(),
                "units.surface 串接必須等於 preedit"
            );
        }
    }

    /// 目前的顯示字元 provenance。
    pub fn units(&self) -> &[DisplayUnit] {
        &self.units
    }

    pub(super) fn compute_units(&self) -> (Vec<DisplayUnit>, Vec<(usize, usize)>) {
        let text = self.composed_text();
        if let Some((units, spans)) = &self.unit_override {
            let joined: String = units.iter().map(|unit| unit.surface.as_str()).collect();
            if joined == text {
                return (units.clone(), spans.clone());
            }
        }
        let expected: Vec<char> = text.chars().collect();
        if expected.is_empty() {
            return (Vec::new(), Vec::new());
        }
        // 純 ASCII 畫面就是字面本身；長 run 不必每次重跑切分規劃。
        if expected.iter().all(|ch| ch.is_ascii()) {
            let typed = self.ascii.typed();
            let same = typed == text;
            let mut byte = 0usize;
            let mut units = Vec::with_capacity(expected.len());
            let mut spans = Vec::with_capacity(expected.len());
            for ch in &expected {
                units.push(DisplayUnit {
                    surface: ch.to_string(),
                    keys: ch.to_string(),
                    reading: None,
                    lang: Lang::En,
                    owner: Owner::Auto,
                    meta: DisplayUnitMeta::Literal,
                });
                if same && text.is_char_boundary(byte) {
                    spans.push((byte, byte + ch.len_utf8()));
                } else {
                    spans.push((0, 0));
                }
                byte += ch.len_utf8();
            }
            return (units, spans);
        }
        let built = self.build_units();
        let mut units = Vec::with_capacity(expected.len());
        let mut spans = Vec::with_capacity(expected.len());
        let mut ok = built.len() == expected.len();
        if ok {
            for (index, item) in built.iter().enumerate() {
                if item.rune != index || item.unit.surface.chars().count() != 1 {
                    ok = false;
                    break;
                }
                let want: String = expected[index].to_string();
                if item.unit.surface != want {
                    ok = false;
                    break;
                }
                units.push(item.unit.clone());
                spans.push(item.typed);
            }
        }
        if !ok {
            return fallback_units(&expected);
        }
        (units, spans)
    }

    fn build_units(&self) -> Vec<Built> {
        let base = self.displayed_base().to_string();
        let base_units = self.base_units(&base);
        let literal_at = rune_offset_ascii(&base, self.literal_offset);
        let mut built = base_units;
        if !self.literal.is_empty() {
            let items = self
                .literal
                .chars()
                .enumerate()
                .map(|(offset, ch)| Built {
                    rune: literal_at + offset,
                    unit: DisplayUnit::new(
                        ch.to_string(),
                        ch.to_string(),
                        None,
                        Lang::Mixed,
                        Owner::User,
                        DisplayUnitMeta::Literal,
                    ),
                    typed: (0, 0),
                })
                .collect();
            built = insert_built(built, literal_at, items);
        }
        // 未完成的注音插在游標所在的音節格；與 `composed_text` 同一條規則。
        if !self.pending.is_empty() {
            let display = self.insert_literal(&base);
            let at = if self.in_zh_lane() && self.zh_cell() < self.syllables.len() {
                self.cell_byte(&display, self.zh_cell())
                    .map(|byte| rune_offset_ascii(&display, byte))
                    .unwrap_or(display.chars().count())
            } else {
                display.chars().count()
            };
            let items = self.pending_units();
            built = insert_built(built, at, items);
        }
        if !self.literal_tail.is_empty() {
            let at = built
                .iter()
                .map(|item| item.rune)
                .max()
                .map_or(0, |last| last + 1);
            let items = self
                .literal_tail
                .chars()
                .enumerate()
                .map(|(offset, ch)| Built {
                    rune: at + offset,
                    unit: DisplayUnit::new(
                        ch.to_string(),
                        ch.to_string(),
                        None,
                        Lang::En,
                        Owner::User,
                        DisplayUnitMeta::Literal,
                    ),
                    typed: (0, 0),
                })
                .collect();
            built = insert_built(built, at, items);
        }
        built
    }

    /// `base`（尚未插入 literal、pending、literal_tail）的逐字 provenance。
    fn base_units(&self, base: &str) -> Vec<Built> {
        let mut out = Vec::new();
        let spans = if self.in_zh_lane() || self.in_ja_lane() {
            Vec::new()
        } else {
            self.mixed_typed_spans()
        };
        let typed = self.ascii.typed();
        let comp = self.composition.segments();
        let mut ja_unit_at = 0usize;
        for (index, segment) in comp.iter().enumerate() {
            let surface = rune_slice(base, segment.span.start, segment.span.end);
            if surface.is_empty() {
                continue;
            }
            let typed_span = spans.get(index).copied().unwrap_or((0, 0));
            let raw = typed
                .get(typed_span.0..typed_span.1)
                .filter(|_| typed_span.0 < typed_span.1)
                .unwrap_or_default();
            let raw = if raw.is_empty() && self.in_ja_lane() {
                // 讀音單位對到羅馬字：一個漢字可能吃掉多個かな（私 → わたし），
                // 不能用顯示 rune span 切 raw_units。
                let end = ja_unit_at.saturating_add(segment.units);
                self.romaji
                    .raw_units()
                    .get(ja_unit_at..end)
                    .map(|items| items.concat())
                    .unwrap_or_default()
            } else {
                raw.to_string()
            };
            if segment.lang == crate::composition::Lang::Ja {
                ja_unit_at = ja_unit_at.saturating_add(segment.units);
            }
            let units = split_segment_units(&surface, segment, &raw);
            for (offset, unit) in units.into_iter().enumerate() {
                out.push(Built {
                    rune: segment.span.start + offset,
                    unit,
                    typed: unit_span(typed_span, &surface, offset, typed),
                });
            }
        }
        // 沒有 composition 片段時（例如直接字面），逐字補齊。
        let covered: usize = out
            .iter()
            .map(|item| item.unit.surface.chars().count())
            .sum();
        if covered != base.chars().count() {
            out.clear();
            for (rune, ch) in base.chars().enumerate() {
                out.push(Built {
                    rune,
                    unit: DisplayUnit::new(
                        ch.to_string(),
                        ch.to_string(),
                        None,
                        Lang::Mixed,
                        Owner::Auto,
                        DisplayUnitMeta::Literal,
                    ),
                    typed: (0, 0),
                });
            }
        }
        // 日文 lane 沒有 `AsciiInput::typed()`；span 改成按鍵流（raw keys）
        // 的位元組範圍，刪除與候選才有正確的 provenance 座標。
        if self.in_ja_lane() {
            let mut at = 0usize;
            for item in &mut out {
                let len = item.unit.keys.len();
                item.typed = (at, at + len);
                at += len;
            }
        }
        out
    }

    /// 刪一個日文顯示字元，只移除這個 unit 的按鍵。
    ///
    /// 日文 lane 用羅馬字單位刪；混打 lane 用 `AsciiInput::typed()` 的
    /// unit span。重播若把剩下的按鍵退成待定前綴（`k`），改用畫面字面固定，
    /// 但保留原本記下的 keys（`き` 仍是 `k`）。
    pub(super) fn delete_ja_unit_visible(&mut self, rune: usize) -> bool {
        let (units, spans) = self.compute_units();
        let Some(unit) = units.get(rune) else {
            return false;
        };
        if unit.lang != Lang::Ja
            || matches!(
                unit.meta,
                DisplayUnitMeta::Literal | DisplayUnitMeta::Unknown
            )
        {
            return false;
        }
        let expected: String = self
            .composed_text()
            .chars()
            .enumerate()
            .filter(|(index, _)| *index != rune)
            .map(|(_, ch)| ch)
            .collect();
        let desired: Vec<DisplayUnit> = units
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != rune)
            .map(|(_, unit)| unit.clone())
            .collect();

        if self.in_ja_lane() {
            let Some((start, count)) = self.ja_units_at_rune(rune) else {
                return false;
            };
            if !self.romaji.delete_units(start, count) {
                return false;
            }
            self.ja_delete_surface = (!expected.is_empty()).then(|| expected.clone());
            self.ja_run_keys = self.romaji.raw_keys();
            if let Some(live) = self.live.as_mut() {
                live.invalidate_for_edit();
            }
            self.leave_empty_japanese_run();
            self.rebuild();
            if self.composed_text() != expected {
                return false;
            }
            if !units_match(&self.units, &desired) {
                let spans = key_spans(&desired);
                self.set_unit_override(desired, spans);
            }
            return true;
        }

        let (start, end) = spans.get(rune).copied().unwrap_or((0, 0));
        let typed = self.ascii.typed().to_string();
        if start >= end || end > typed.len() || !typed.is_char_boundary(start) {
            return false;
        }
        let literal_spans = self.mixed_literal_spans_after(start, end);
        let mut next = String::with_capacity(typed.len().saturating_sub(end - start));
        next.push_str(&typed[..start]);
        next.push_str(&typed[end..]);
        self.ascii.replay(&next, &self.lexicon);
        self.literal_spans = literal_spans;
        if self.composed_text() != expected {
            self.ascii.replay(&expected, &self.lexicon);
            let spans = char_spans(&expected);
            if !units_match(&self.units, &desired) {
                self.set_unit_override(desired, spans);
            }
        }
        true
    }

    fn set_unit_override(&mut self, units: Vec<DisplayUnit>, spans: Vec<(usize, usize)>) {
        self.unit_override = Some((units, spans));
    }

    fn pending_units(&self) -> Vec<Built> {
        let symbols: Vec<char> = self.pending.chars().collect();
        let keys: Vec<char> = self.zh_pending_keys.chars().collect();
        let keyboard = Keyboard::standard();
        symbols
            .into_iter()
            .enumerate()
            .map(|(offset, symbol)| {
                let key = keys
                    .get(offset)
                    .filter(|_| keys.len() == self.pending.chars().count())
                    .and_then(|key| keyboard.symbol(*key).map(|_| *key))
                    .unwrap_or(symbol);
                Built {
                    rune: offset,
                    unit: DisplayUnit::new(
                        symbol.to_string(),
                        key.to_string(),
                        Some(symbol.to_string()),
                        Lang::Zh,
                        Owner::Auto,
                        DisplayUnitMeta::Zhuyin,
                    ),
                    typed: (0, 0),
                }
            })
            .collect()
    }
}

fn units_match(current: &[DisplayUnit], desired: &[DisplayUnit]) -> bool {
    current.len() == desired.len()
        && current.iter().zip(desired).all(|(left, right)| {
            left.surface == right.surface
                && left.keys == right.keys
                && left.reading == right.reading
                && left.lang == right.lang
                && left.meta == right.meta
        })
}

fn key_spans(units: &[DisplayUnit]) -> Vec<(usize, usize)> {
    let mut at = 0usize;
    units
        .iter()
        .map(|unit| {
            let start = at;
            at += unit.keys.len();
            (start, at)
        })
        .collect()
}

fn char_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for (byte, ch) in text.char_indices() {
        spans.push((byte, byte + ch.len_utf8()));
    }
    spans
}

fn fallback_units(chars: &[char]) -> (Vec<DisplayUnit>, Vec<(usize, usize)>) {
    let units = chars
        .iter()
        .map(|ch| DisplayUnit {
            surface: ch.to_string(),
            keys: ch.to_string(),
            reading: None,
            lang: Lang::Mixed,
            owner: Owner::Auto,
            meta: DisplayUnitMeta::Unknown,
        })
        .collect();
    (units, vec![(0, 0); chars.len()])
}

fn insert_built(mut built: Vec<Built>, at: usize, items: Vec<Built>) -> Vec<Built> {
    if items.is_empty() {
        return built;
    }
    let width = items.len();
    for item in &mut built {
        if item.rune >= at {
            item.rune += width;
        }
    }
    built.extend(items.into_iter().map(|mut item| {
        item.rune += at;
        item
    }));
    built
}

fn rune_offset_ascii(text: &str, byte: usize) -> usize {
    let mut at = byte.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    text[..at].chars().count()
}

fn rune_slice(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn unit_span(span: (usize, usize), surface: &str, offset: usize, typed: &str) -> (usize, usize) {
    if span.0 >= span.1 || span.1 > typed.len() {
        return (0, 0);
    }
    let chars = surface.chars().count().max(1);
    // 字面一鍵一字時直接按 UTF-8 邊界切，注音字面不會切在 byte 中間。
    let mut end = span.1;
    if !typed.is_char_boundary(end) || typed[span.0..end].chars().count() != chars {
        if typed[span.0..].starts_with(surface) {
            end = span.0 + surface.len();
        } else {
            while end < typed.len() && !typed.is_char_boundary(end) {
                end += 1;
            }
        }
    }
    let slice = &typed[span.0..end];
    if slice.chars().count() == chars {
        let boundaries: Vec<usize> = slice
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(std::iter::once(slice.len()))
            .collect();
        let start = span.0 + boundaries[offset.min(chars)];
        let end = span.0 + boundaries[(offset + 1).min(chars)];
        return (start, end.max(start));
    }
    let start = span.0 + offset * (span.1 - span.0) / chars;
    let end = span.0 + (offset + 1) * (span.1 - span.0) / chars;
    (start, end.max(start))
}

fn split_segment_units(surface: &str, segment: &CompSegment, raw: &str) -> Vec<DisplayUnit> {
    let chars: Vec<char> = surface.chars().collect();
    // 直接選回注音字面時，管線可能把它記成 Literal；畫面全是注音符號
    // 仍要保留 j／i／3 的按鍵 provenance。
    if !chars.is_empty() && chars.iter().all(|ch| class_of(*ch) != Class::None) {
        return split_zh_units(&chars, segment, raw);
    }
    match segment.lang {
        crate::composition::Lang::Zh => split_zh_units(&chars, segment, raw),
        crate::composition::Lang::Ja => split_ja_units(&chars, segment, raw),
        crate::composition::Lang::Literal => chars
            .iter()
            .map(|ch| literal_unit(*ch, segment.owner))
            .collect(),
    }
}

fn literal_unit(ch: char, owner: Owner) -> DisplayUnit {
    DisplayUnit::new(
        ch.to_string(),
        ch.to_string(),
        None,
        Lang::Mixed,
        owner,
        DisplayUnitMeta::Literal,
    )
}

fn split_zh_units(chars: &[char], segment: &CompSegment, raw: &str) -> Vec<DisplayUnit> {
    let readings: Vec<&str> = segment.reading.split_whitespace().collect();
    let surface_reading: String = chars.iter().collect();
    let symbols: Vec<char> = if segment.reading.chars().count() == chars.len() {
        segment.reading.chars().collect()
    } else {
        chars.to_vec()
    };
    let keys: String = if !raw.is_empty() && raw.is_ascii() && raw.chars().count() == chars.len() {
        raw.to_string()
    } else {
        zh_keys_for(&surface_reading)
            .or_else(|| zh_keys_for(&segment.reading))
            .unwrap_or_default()
    };
    // 未轉換的注音符號：一個符號一個按鍵（`ㄨㄛˇ` → `j`、`i`、`3`）。
    if !chars.is_empty()
        && chars.iter().all(|ch| class_of(*ch) != Class::None)
        && chars.len() == symbols.len()
    {
        let key_chars: Vec<char> = keys.chars().collect();
        return chars
            .iter()
            .enumerate()
            .map(|(index, ch)| {
                let key = key_chars
                    .get(index)
                    .filter(|_| key_chars.len() == chars.len())
                    .map(|key| key.to_string())
                    .unwrap_or_else(|| ch.to_string());
                DisplayUnit::new(
                    ch.to_string(),
                    key,
                    Some(ch.to_string()),
                    Lang::Zh,
                    segment.owner,
                    DisplayUnitMeta::Zhuyin,
                )
            })
            .collect();
    }
    // 一個音節一個字（`我` → `ji3`）。
    if readings.len() == chars.len() && !readings.is_empty() {
        let key_parts = split_zh_keys(raw, &keys, chars.len());
        return chars
            .iter()
            .enumerate()
            .map(|(index, ch)| {
                let reading = readings[index];
                let keys = key_parts
                    .get(index)
                    .cloned()
                    .filter(|keys| !keys.is_empty())
                    .unwrap_or_else(|| zh_keys_for(reading).unwrap_or_else(|| ch.to_string()));
                let meta = if crate::zh::han::is_han(*ch) {
                    DisplayUnitMeta::Han
                } else {
                    DisplayUnitMeta::Zh
                };
                DisplayUnit::new(
                    ch.to_string(),
                    keys,
                    Some(reading.to_string()),
                    Lang::Zh,
                    segment.owner,
                    meta,
                )
            })
            .collect();
    }
    if chars.len() == 1 {
        let ch = chars[0];
        let reading = (!segment.reading.is_empty()).then(|| segment.reading.clone());
        let keys = if segment.reading.is_empty() {
            ch.to_string()
        } else {
            keys.clone()
        };
        return vec![DisplayUnit::new(
            ch.to_string(),
            keys,
            reading,
            Lang::Zh,
            segment.owner,
            if crate::zh::han::is_han(ch) {
                DisplayUnitMeta::Han
            } else {
                DisplayUnitMeta::Zh
            },
        )];
    }
    // 對不上單位數：保留字面，不硬猜。
    chars
        .iter()
        .map(|ch| literal_unit(*ch, segment.owner))
        .collect()
}

fn split_zh_keys(raw: &str, keys: &str, count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    if !raw.is_empty() {
        let spans = zh_key_spans(raw);
        if spans.len() == count {
            return spans
                .into_iter()
                .map(|(start, end)| raw[start..end].to_string())
                .collect();
        }
    }
    let key_chars: Vec<char> = keys.chars().collect();
    if key_chars.len() == count {
        return key_chars.into_iter().map(|ch| ch.to_string()).collect();
    }
    Vec::new()
}

fn zh_keys_for(reading: &str) -> Option<String> {
    let syllables = parse_reading(reading);
    if syllables.is_empty() {
        return None;
    }
    let keyboard = Keyboard::standard();
    let mut keys = String::new();
    for syllable in syllables {
        keys.push_str(&keyboard.keys(&syllable).ok()?);
    }
    Some(keys)
}

fn zh_key_spans(raw: &str) -> Vec<(usize, usize)> {
    let keyboard = Keyboard::standard();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut last = Class::None;
    for (index, ch) in raw.char_indices() {
        let next = index + ch.len_utf8();
        let Some(symbol) = keyboard.symbol(ch) else {
            if start < index {
                out.push((start, index));
            }
            start = next;
            last = Class::None;
            continue;
        };
        let class = class_of(symbol);
        if class == Class::Tone {
            out.push((start, next));
            start = next;
            last = Class::None;
            continue;
        }
        if class <= last && start < index {
            out.push((start, index));
            start = index;
        }
        last = class;
    }
    if start < raw.len() {
        out.push((start, raw.len()));
    }
    out
}

fn split_ja_units(chars: &[char], segment: &CompSegment, raw: &str) -> Vec<DisplayUnit> {
    let reading: Vec<char> = segment.reading.chars().collect();
    if chars.len() == 1 {
        let ch = chars[0];
        return vec![DisplayUnit::new(
            ch.to_string(),
            if raw.is_empty() {
                ch.to_string()
            } else {
                raw.to_string()
            },
            (!segment.reading.is_empty()).then(|| segment.reading.clone()),
            Lang::Ja,
            segment.owner,
            if crate::zh::han::is_han(ch) {
                DisplayUnitMeta::Han
            } else {
                DisplayUnitMeta::Kana
            },
        )];
    }
    if chars.iter().all(|ch| is_kana(*ch)) && reading.len() == chars.len() {
        let parts = split_kana_keys(raw, chars.len());
        return chars
            .iter()
            .enumerate()
            .map(|(index, ch)| {
                DisplayUnit::new(
                    ch.to_string(),
                    parts
                        .get(index)
                        .cloned()
                        .filter(|keys| !keys.is_empty())
                        .unwrap_or_else(|| ch.to_string()),
                    Some(ch.to_string()),
                    Lang::Ja,
                    segment.owner,
                    DisplayUnitMeta::Kana,
                )
            })
            .collect();
    }
    chars
        .iter()
        .map(|ch| literal_unit(*ch, segment.owner))
        .collect()
}

/// 把一段羅馬字切成かな數量相符的按鍵片段。
///
/// 例：`kyo` → `k`、`yo`；`kyou` → `k`、`yo`、`u`。一組對照表項產生多個
/// かな時，後面的小寫かな分到母音段，第一個かな只留子音。
fn split_kana_keys(raw: &str, count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    if raw.is_empty() {
        return vec![String::new(); count];
    }
    if raw.chars().count() == count {
        return raw.chars().map(|ch| ch.to_string()).collect();
    }
    let mut chunks = vowel_chunks(raw);
    while chunks.len() < count {
        let Some(index) = chunks.iter().position(|chunk| split_onset(chunk).is_some()) else {
            break;
        };
        let chunk = chunks.remove(index);
        let (head, tail) = split_onset(&chunk).expect("position 已確認可切");
        chunks.insert(index, tail);
        chunks.insert(index, head);
    }
    while chunks.len() > count {
        let tail = chunks.pop().unwrap_or_default();
        if let Some(last) = chunks.last_mut() {
            last.push_str(&tail);
        }
    }
    chunks
}

fn vowel_chunks(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in raw.chars() {
        current.push(ch);
        if is_vowel(ch) {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn is_vowel(ch: char) -> bool {
    matches!(ch, 'a' | 'i' | 'u' | 'e' | 'o')
}

fn split_onset(chunk: &str) -> Option<(String, String)> {
    let chars: Vec<char> = chunk.chars().collect();
    if chars.len() < 2 {
        return None;
    }
    // `kyo` → `k` + `yo`；`sha` → `sh` + `a`。
    if let Some(at) = chars.iter().position(|ch| *ch == 'y') {
        if at > 0 {
            let head: String = chars[..at].iter().collect();
            let tail: String = chars[at..].iter().collect();
            return Some((head, tail));
        }
    }
    let at = chars
        .iter()
        .rposition(|ch| is_vowel(*ch))
        .filter(|at| *at > 0)?;
    let head: String = chars[..at].iter().collect();
    let tail: String = chars[at..].iter().collect();
    Some((head, tail))
}

fn is_kana(ch: char) -> bool {
    matches!(ch,
        '\u{3041}'..='\u{3096}'
        | '\u{309d}'..='\u{309f}'
        | '\u{30a1}'..='\u{30fa}'
        | '\u{30fd}'..='\u{30ff}'
        | '\u{ff66}'..='\u{ff9d}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{Key, KeyCode};
    use crate::lattice::{InMemoryLexicon, LexEntry};
    use crate::session::SessionConfig;
    use crate::Mode;
    use std::sync::Arc;

    fn typed(session: &mut Session, keys: &str) {
        for ch in keys.chars() {
            let key = if ch == ' ' {
                Key::new(KeyCode::Space, None, false)
            } else {
                Key::char(ch, false)
            };
            session.press(key);
        }
    }

    fn assert_invariant(session: &Session, label: &str) {
        let joined: String = session
            .units()
            .iter()
            .map(|unit| unit.surface.as_str())
            .collect();
        assert_eq!(joined, session.preedit(), "{label}: units 串接 != preedit");
        assert_eq!(
            session.units().len(),
            session.preedit().chars().count(),
            "{label}: unit 數 != 顯示字元數"
        );
    }

    fn zh_session() -> Session {
        let mut lex = InMemoryLexicon::new();
        lex.push(LexEntry::new("我", "ㄨㄛˇ", 0, 0));
        let mut config = SessionConfig::new(Mode::Mixed);
        config.zh_lexicon = Some(Arc::new(lex));
        Session::new(config)
    }

    #[test]
    fn units_invariant_holds_for_every_lane() {
        let mut s = zh_session();
        typed(&mut s, "ji3");
        assert_invariant(&s, "注音轉換");
        assert_eq!(s.units()[0].surface, "我");
        assert_eq!(s.units()[0].keys, "ji3");

        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        typed(&mut s, "ji3");
        assert_invariant(&s, "未轉換注音");
        let keys: Vec<&str> = s.units().iter().map(|unit| unit.keys.as_str()).collect();
        assert_eq!(keys, ["j", "i", "3"]);

        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        typed(&mut s, "watashi");
        assert_invariant(&s, "日文");
        let keys: Vec<&str> = s.units().iter().map(|unit| unit.keys.as_str()).collect();
        assert_eq!(keys, ["wa", "ta", "shi"]);

        let mut s = zh_session();
        typed(&mut s, "ji3hello");
        assert_invariant(&s, "混打");
        let has_literal = s
            .units()
            .iter()
            .any(|unit| unit.meta == DisplayUnitMeta::Literal);
        assert!(has_literal, "英文字面要有 Literal unit");

        let mut s = Session::new(SessionConfig::new(Mode::Mixed));
        typed(&mut s, "abc;def");
        assert_invariant(&s, "字面");
        assert!(s
            .units()
            .iter()
            .all(|unit| unit.meta == DisplayUnitMeta::Literal));

        let mut s = zh_session();
        typed(&mut s, "ji3");
        s.press(Key::new(KeyCode::Down, None, false));
        let index = s
            .candidates()
            .iter()
            .position(|candidate| candidate.surface == "ㄨㄛˇ")
            .expect("注音字面候選");
        s.choose_candidate(index);
        assert_invariant(&s, "候選套用");
        assert_eq!(s.preedit(), "ㄨㄛˇ");
    }

    #[test]
    fn split_kana_keys_keeps_small_kana_separate() {
        assert_eq!(split_kana_keys("kyo", 2), ["k", "yo"]);
        assert_eq!(split_kana_keys("kyou", 3), ["k", "yo", "u"]);
        assert_eq!(split_kana_keys("wata", 2), ["wa", "ta"]);
    }

    #[test]
    fn zh_key_spans_split_syllables() {
        assert_eq!(zh_key_spans("ji3"), [(0, 3)]);
        assert_eq!(zh_key_spans("su3ji3"), [(0, 3), (3, 6)]);
    }
}
