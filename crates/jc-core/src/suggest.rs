//! Tab 補全。
//!
//! 對照前身 Go `core/suggest`。補全只來自既有的個人片語與系統詞，是忠實的：
//! 不生成新內容。不理會建議時，顯示、解碼與學習與沒有此功能完全相同。
//!
//! 前身以 `decode.Explicit` 介面查詢使用者親手加入的詞；這裡用本地
//! [`ExplicitWords`] trait 隔開，避免 `suggest` 直接耦合 `vocab` 或個人資料。

use crate::decode::{Decoder, Locks, Seg};
use crate::lattice::Lattice;
use crate::reading::Unit;
use crate::zh::keyboard::Keyboard;
use crate::zh::syllable::Syllable;

/// 一個補全建議。
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// 完整的詞。
    pub word: String,
    /// 該詞在組字區的起點（音節位置）。
    pub start: usize,
    /// 尚未輸入的部分，以淡色顯示。
    pub rest: String,
    /// 尚未輸入的音節。
    pub syls: Vec<Unit>,
    /// 信心。
    pub conf: f64,
    /// 非空時為將被替換的已顯示前綴，必須清楚預覽後才接受。
    pub replace: String,
}

/// 控制何時建議。寧可少建議，不要讓使用者養成忽略它的習慣。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// 最低信心。
    pub min_conf: f64,
    /// 至少要省幾個音節。
    pub min_syllables: usize,
    /// 檢視建議的等效按鍵成本；0 停用效益門檻。
    pub attention_cost: f64,
    /// 可明確預覽並替換未鎖定的同音尾端。
    pub rewrite_prefix: bool,
    /// 延伸詞間的條件信心門檻；另受 [`Options::min_conf`] 限制。
    pub min_rewrite_conf: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            min_conf: 0.4,
            min_syllables: 1,
            attention_cost: 0.5,
            rewrite_prefix: false,
            min_rewrite_conf: 0.7,
        }
    }
}

/// 使用者親手加入的詞。
///
/// 前身的 `decode.Explicit`；由呼叫端以 `vocab` 或個人資料實作。
pub trait ExplicitWords {
    /// 回報 `word` 是否是親手加入的詞。
    fn explicit(&self, word: &str) -> bool;

    /// 是否有任何親手加入的詞；沒有時可以跳過額外查詢。
    fn has_explicit(&self) -> bool;
}

/// 依解碼結果產生建議；沒有合格的建議時回傳 `None`。
pub fn for_path(
    decoder: &Decoder<'_>,
    lat: &Lattice,
    path: &[Seg],
    options: Options,
) -> Option<Suggestion> {
    for_with_locks_and_explicit(decoder, lat, path, &Locks::new(), None, options)
}

/// 保留既有追加補全，另避開鎖定範圍。
pub fn for_with_locks(
    decoder: &Decoder<'_>,
    lat: &Lattice,
    path: &[Seg],
    locks: &Locks,
    options: Options,
) -> Option<Suggestion> {
    for_with_locks_and_explicit(decoder, lat, path, locks, None, options)
}

/// 保留既有追加補全，沒有合格建議時才找同音替換。
///
/// 僅檢查解碼分段的邊界，最多 [`Decoder::max_len`] 音節（自訂詞可以更長），
/// 不跨手選詞。
pub fn for_with_locks_and_explicit(
    decoder: &Decoder<'_>,
    lat: &Lattice,
    path: &[Seg],
    locks: &Locks,
    explicit: Option<&dyn ExplicitWords>,
    options: Options,
) -> Option<Suggestion> {
    if path.is_empty() {
        return None;
    }
    if let Some(suggestion) = explicit_append(decoder, lat, path, locks, explicit, options) {
        return Some(suggestion);
    }
    let mut best = append_only(decoder, lat, path, options);
    if best
        .as_ref()
        .is_some_and(|suggestion| overlaps(locks, suggestion.start, lat.len()))
    {
        best = None;
    }
    if let Some(cross) = cross_append(decoder, lat, path, locks, options) {
        if best
            .as_ref()
            .is_none_or(|current| cross.conf > current.conf)
        {
            best = Some(cross);
        }
    }
    if best.is_some() {
        return best;
    }
    if !options.rewrite_prefix {
        return None;
    }
    let mut winner: Option<Suggestion> = None;
    for (index, segment) in path.iter().enumerate() {
        let typed = lat.len().saturating_sub(segment.start);
        if typed < 2 || typed >= decoder.max_len() || overlaps(locks, segment.start, lat.len()) {
            continue;
        }
        let left = if index > 0 {
            path[index - 1].word.as_str()
        } else {
            ""
        };
        let (completion, found) = decoder.complete_homophones(lat, segment.start, left);
        if !found || completion.conf < options.min_rewrite_conf {
            continue;
        }
        let displayed: String = path[index..].iter().map(|seg| seg.word.as_str()).collect();
        let surface_prefix: String = completion.entry.surface.chars().take(typed).collect();
        if surface_prefix == displayed {
            continue;
        }
        if let Some(mut suggestion) = completion_of(&completion, segment.start, typed, options) {
            if winner
                .as_ref()
                .is_none_or(|current| suggestion.conf > current.conf)
            {
                suggestion.replace = displayed;
                winner = Some(suggestion);
            }
        }
    }
    winner
}

/// 起點與長度是否碰到任何鎖定。
fn overlaps(locks: &Locks, start: usize, end: usize) -> bool {
    locks
        .iter()
        .any(|(at, lock)| *at < end && start < at + lock.len)
}

/// 找跨過幾個解碼分段的自訂詞補全。
///
/// 「新阿姆」解成「新｜阿姆」時，尾端那一段只看得到「阿姆斯特丹」，使用者
/// 加的「新阿姆斯特朗…砲」卻是從「新」開始。從最長的範圍找起，只接受畫面
/// 上的字不變、往後延伸的自訂詞；最後一段本身由 [`append_only`] 處理。
fn explicit_append(
    decoder: &Decoder<'_>,
    lat: &Lattice,
    path: &[Seg],
    locks: &Locks,
    explicit: Option<&dyn ExplicitWords>,
    options: Options,
) -> Option<Suggestion> {
    let explicit = explicit?;
    if !explicit.has_explicit() {
        return None;
    }
    for index in 0..path.len() {
        let start = path[index].start;
        let typed = lat.len().saturating_sub(start);
        if typed < 2 || overlaps(locks, start, lat.len()) {
            continue;
        }
        let displayed: String = path[index..].iter().map(|seg| seg.word.as_str()).collect();
        let mut merged: Vec<Seg> = path[..index].to_vec();
        merged.push(Seg::partial(displayed, start, lat.len()));
        let (completion, found) = decoder.complete(lat, &merged);
        if !found
            || completion.entry.units() <= typed
            || !explicit.explicit(&completion.entry.surface)
        {
            continue;
        }
        if let Some(suggestion) = completion_of(&completion, start, typed, options) {
            return Some(suggestion);
        }
    }
    None
}

/// 找跨過幾個解碼分段的系統詞補全。
///
/// 解碼器切開這段，表示它認為切開的讀法比較可能，所以不能只在延伸詞之間
/// 比。信心另外乘上 [`Decoder::complete_across`] 的比例：延伸詞群 ÷
/// （延伸詞群＋目前路徑），比例要過半。
fn cross_append(
    decoder: &Decoder<'_>,
    lat: &Lattice,
    path: &[Seg],
    locks: &Locks,
    options: Options,
) -> Option<Suggestion> {
    let mut best: Option<Suggestion> = None;
    if path.len() < 2 {
        return None;
    }
    for index in 0..path.len() - 1 {
        let start = path[index].start;
        let typed = lat.len().saturating_sub(start);
        if typed < 2 || typed >= decoder.max_len() || overlaps(locks, start, lat.len()) {
            continue;
        }
        let (mut completion, ratio, found) = decoder.complete_across(lat, path, index);
        if !found || completion.entry.units() <= typed || ratio <= 0.5 {
            continue;
        }
        completion.conf *= ratio;
        if let Some(suggestion) = completion_of(&completion, start, typed, options) {
            if best
                .as_ref()
                .is_none_or(|current| suggestion.conf > current.conf)
            {
                best = Some(suggestion);
            }
        }
    }
    best
}

fn append_only(
    decoder: &Decoder<'_>,
    lat: &Lattice,
    path: &[Seg],
    options: Options,
) -> Option<Suggestion> {
    let (completion, found) = decoder.complete(lat, path);
    if !found {
        return None;
    }
    let last = path.last()?;
    let typed = last.end.saturating_sub(last.start);
    completion_of(&completion, last.start, typed, options)
}

fn completion_of(
    completion: &crate::decode::Completion,
    start: usize,
    typed: usize,
    options: Options,
) -> Option<Suggestion> {
    let saved = completion.entry.units().saturating_sub(typed);
    if saved < options.min_syllables || completion.conf < options.min_conf {
        return None;
    }
    // 每個符號一鍵，另有聲調／一聲空白；Tab 本身也要一鍵。標準與倚天 41 鍵
    // 都是一鍵一符號，鍵數相同。這是操作效益的代理量，不宣稱等同真實輸入時間。
    let keyboard = Keyboard::standard();
    let mut keys = 0usize;
    for raw in completion.entry.reading.split_whitespace().skip(typed) {
        let syllable = Syllable::from_raw(raw.to_string());
        keys += keyboard
            .keys(&syllable)
            .map(|sequence| sequence.chars().count())
            .unwrap_or(0);
    }
    if options.attention_cost > 0.0
        && completion.conf * (keys as f64 - 1.0) <= options.attention_cost
    {
        return None;
    }
    let rest: String = completion.entry.surface.chars().skip(typed).collect();
    let syls: Vec<Unit> = completion
        .entry
        .reading
        .split_whitespace()
        .skip(typed)
        .map(Unit::new)
        .collect();
    Some(Suggestion {
        word: completion.entry.surface.clone(),
        start,
        rest,
        syls,
        conf: completion.conf,
        replace: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lattice::{InMemoryLexicon, Lattice, LexEntry};
    use crate::score::{ConnectionMatrix, ScoreContext};

    fn entry(surface: &str, reading: &str, cost: i32) -> LexEntry {
        LexEntry::new(surface, reading, cost, 1)
    }

    fn context() -> ScoreContext {
        ScoreContext {
            length_bonus: 0,
            unknown_cost: 100,
            connection: ConnectionMatrix::with_fallback(0),
            ..Default::default()
        }
    }

    /// 同樣信心，省一鍵與省多鍵的建議應有不同效益；不用真實時間單位假裝量過
    /// 使用者速度。
    #[test]
    fn test_benefit_depends_on_remaining_typing() {
        let cases: &[(&str, &str, bool)] = &[
            ("麻吃", "ㄇㄚˊ ㄔ", false),
            ("麻婆豆腐", "ㄇㄚˊ ㄆㄛˊ ㄉㄡˋ ㄈㄨˇ", true),
        ];
        for (word, reading, want) in cases {
            let mut lex = InMemoryLexicon::new();
            lex.push(entry("麻", "ㄇㄚˊ", 0));
            lex.push(entry(word, reading, 0));
            // 同讀音的競爭詞讓補全信心約為一半，才測得到效益門檻而不是排序。
            lex.push(entry("麻煩", "ㄇㄚˊ ㄈㄢˊ", 0));
            let ctx = context();
            let lat = Lattice::build(&["ㄇㄚˊ"], &lex, &ctx).expect("建立詞格");
            let d = crate::decode::Decoder::new(&lex, &ctx);
            let path = vec![Seg::new("麻", 0, 1)];
            let mut options = Options {
                attention_cost: 0.6,
                ..Options::default()
            };
            assert_eq!(
                for_path(&d, &lat, &path, options).is_some(),
                *want,
                "{word}：有無建議"
            );
            options.attention_cost = 0.0;
            assert!(
                for_path(&d, &lat, &path, options).is_some(),
                "關閉效益門檻後原候選應可顯示"
            );
        }
    }

    #[test]
    fn test_script_is_used_in_suggestion_shape() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("沒", "ㄇㄟˊ", 0));
        lex.push(entry("沒有", "ㄇㄟˊ ㄧㄡˇ", 0));
        let ctx = context();
        let lat = Lattice::build(&["ㄇㄟˊ"], &lex, &ctx).expect("建立詞格");
        let d = crate::decode::Decoder::new(&lex, &ctx);
        let path = vec![Seg::new("沒", 0, 1)];
        let suggestion = for_path(
            &d,
            &lat,
            &path,
            Options {
                attention_cost: 0.0,
                ..Options::default()
            },
        )
        .expect("應有建議");
        assert_eq!(suggestion.word, "沒有");
        assert_eq!(suggestion.start, 0);
        assert_eq!(suggestion.rest, "有");
        assert_eq!(suggestion.syls, vec![Unit::new("ㄧㄡˇ")], "剩下的音節");
        assert!(suggestion.conf > 0.4);
    }

    /// 本地 trait 讓呼叫端注入自訂詞，不必依賴 `vocab`。
    struct Custom(&'static [&'static str]);

    impl ExplicitWords for Custom {
        fn explicit(&self, word: &str) -> bool {
            self.0.contains(&word)
        }

        fn has_explicit(&self) -> bool {
            !self.0.is_empty()
        }
    }

    #[test]
    fn test_explicit_words_extend_across_segments() {
        let mut lex = InMemoryLexicon::new();
        lex.push(entry("新", "ㄒㄧㄣ", 0));
        lex.push(entry("阿姆", "ㄚ ㄇㄨˇ", 0));
        lex.push(entry(
            "新阿姆斯特朗回旋加速噴氣式阿姆斯特朗砲",
            "ㄒㄧㄣ ㄚ ㄇㄨˇ ㄙ ㄊㄜˋ ㄌㄤˇ ㄏㄨㄟˊ ㄒㄩㄢˊ ㄐㄧㄚ ㄙㄨˋ ㄆㄣ ㄑㄧˋ ㄕˋ ㄚ ㄇㄨˇ ㄙ ㄊㄜˋ ㄌㄤˇ ㄆㄠˋ",
            0,
        ));
        let ctx = context();
        let lat = Lattice::build(&["ㄒㄧㄣ", "ㄚ", "ㄇㄨˇ"], &lex, &ctx).expect("建立詞格");
        let d = crate::decode::Decoder::new(&lex, &ctx);
        let path = vec![Seg::new("新", 0, 1), Seg::new("阿姆", 1, 3)];
        let custom = Custom(&["新阿姆斯特朗回旋加速噴氣式阿姆斯特朗砲"]);
        let suggestion = for_with_locks_and_explicit(
            &d,
            &lat,
            &path,
            &Locks::new(),
            Some(&custom),
            Options {
                attention_cost: 0.0,
                ..Options::default()
            },
        )
        .expect("自訂詞應跨分段延伸");
        assert_eq!(suggestion.start, 0);
        assert_eq!(suggestion.word, "新阿姆斯特朗回旋加速噴氣式阿姆斯特朗砲");
        assert_eq!(suggestion.syls.len(), 16, "剩下 19 個音節中的 16 個");
    }
}
