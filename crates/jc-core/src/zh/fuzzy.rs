//! 模糊音規則：ㄗ／ㄓ、ㄘ／ㄔ、ㄙ／ㄕ、ㄣ／ㄥ 的逐對近音展開。
//!
//! 對照前身 `core/fuzzy`。規則只在「已完成的音節」上產生替代假設：保留
//! 聲調、不改動其餘符號，且結果必須是現行音節表（[`Syllable::legal`]）內的
//! 合法音節；原音永遠是第一個假設、成本 0。本模組是純規則：不載入詞庫、
//! 不依賴模型，也不決定顯示什麼。

use crate::zh::input::{Hyp, Lattice as SyllableLattice};
use crate::zh::syllable::{class_of, Class, Syllable};

/// 一組可逐對開關的近音配對。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pair {
    /// ㄗ／ㄓ。
    ZZh,
    /// ㄘ／ㄔ。
    CCh,
    /// ㄙ／ㄕ。
    SSh,
    /// ㄣ／ㄥ。
    EnEng,
}

/// 配對的固定順序。
pub const PAIRS: [Pair; 4] = [Pair::ZZh, Pair::CCh, Pair::SSh, Pair::EnEng];

struct PairDef {
    id: &'static str,
    a: char,
    b: char,
    /// 換的是韻母（ㄣ／ㄥ）而非聲母。
    final_sound: bool,
}

const PAIR_DEFS: [PairDef; 4] = [
    PairDef {
        id: "z-zh",
        a: 'ㄗ',
        b: 'ㄓ',
        final_sound: false,
    },
    PairDef {
        id: "c-ch",
        a: 'ㄘ',
        b: 'ㄔ',
        final_sound: false,
    },
    PairDef {
        id: "s-sh",
        a: 'ㄙ',
        b: 'ㄕ',
        final_sound: false,
    },
    PairDef {
        id: "en-eng",
        a: 'ㄣ',
        b: 'ㄥ',
        final_sound: true,
    },
];

impl Pair {
    /// 設定與診斷用的穩定代號。
    pub fn id(self) -> &'static str {
        PAIR_DEFS[self as usize].id
    }

    /// 配對的兩個符號，供說明文字使用，例如「ㄗ／ㄓ」。
    pub fn symbols(self) -> String {
        let def = &PAIR_DEFS[self as usize];
        format!("{}／{}", def.a, def.b)
    }
}

/// 依固定順序列出所有配對。
pub fn pairs() -> [Pair; 4] {
    PAIRS
}

/// 由設定代號找配對。
pub fn parse_pair(id: &str) -> Option<Pair> {
    PAIRS.into_iter().find(|pair| pair.id() == id)
}

/// 一組配對的啟用方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    /// 不展開，行為與沒有此功能時相同。這也是缺少設定時的預設。
    #[default]
    Off,
    /// 只在開候選時合併近音候選；自動顯示與提交維持原音。
    Select,
    /// 除了 [`Mode::Select`] 的行為，另在近音路徑的語境優勢超過門檻時自動改組字。
    Auto,
}

impl Mode {
    /// 設定值的穩定字串。
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Select => "select",
            Mode::Auto => "auto",
        }
    }

    /// 由設定值找模式。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Mode::Off),
            "select" => Some(Mode::Select),
            "auto" => Some(Mode::Auto),
            _ => None,
        }
    }
}

/// 各配對的模式。零值全部關閉。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Config {
    modes: [Mode; 4],
}

impl Config {
    /// 建立全部關閉的設定。
    pub fn new() -> Self {
        Self::default()
    }

    /// 建立全部設為 `mode` 的設定。
    pub fn all(mode: Mode) -> Self {
        let mut config = Self::default();
        for pair in PAIRS {
            config = config.with(pair, mode);
        }
        config
    }

    /// 回傳把配對 `pair` 設為 `mode` 的新設定。
    pub fn with(mut self, pair: Pair, mode: Mode) -> Self {
        self.modes[pair as usize] = mode;
        self
    }

    /// 回傳配對 `pair` 目前的模式。
    pub fn mode(self, pair: Pair) -> Mode {
        self.modes[pair as usize]
    }

    /// 是否有任何配對啟用。
    pub fn enabled(self) -> bool {
        self.modes.iter().any(|mode| *mode != Mode::Off)
    }

    /// 是否有任何配對啟用自動修正。
    pub fn auto_enabled(self) -> bool {
        self.modes.iter().any(|mode| *mode == Mode::Auto)
    }

    /// 只保留自動修正配對的設定。
    pub fn auto_only(self) -> Self {
        let mut out = Self::default();
        for pair in PAIRS {
            if self.mode(pair) == Mode::Auto {
                out.modes[pair as usize] = Mode::Auto;
            }
        }
        out
    }

    /// 設定內容，可併入快取鍵；全部關閉時為空字串。
    pub fn key(self) -> String {
        PAIRS
            .iter()
            .filter(|pair| self.mode(**pair) != Mode::Off)
            .map(|pair| format!("{}={}", pair.id(), self.mode(*pair).as_str()))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn pair_of(self, ch: char, final_sound: bool) -> Option<(Pair, char)> {
        for pair in PAIRS {
            let def = &PAIR_DEFS[pair as usize];
            if self.mode(pair) == Mode::Off || def.final_sound != final_sound {
                continue;
            }
            if ch == def.a {
                return Some((pair, def.b));
            }
            if ch == def.b {
                return Some((pair, def.a));
            }
        }
        None
    }

    /// 回傳音節 `s` 的近音替代（不含 `s` 本身）。
    ///
    /// 只聲母、只韻母、聲母與韻母都換，各自去重並要求在現行音節表內；聲調
    /// 不變。來源音節只需結構合法：把 ㄕㄨㄥˋ（不在音節表）打成 ㄙㄨㄥˋ 的人，
    /// 正是這個功能的對象；結果則一律須是表內音節。
    pub fn alternatives(self, s: &Syllable) -> Vec<Alt> {
        if !self.enabled() || !s.valid() {
            return Vec::new();
        }
        let chars: Vec<char> = s.as_str().chars().collect();
        let initial = chars.iter().position(|ch| class_of(*ch) == Class::Initial);
        let final_sound = chars.iter().position(|ch| class_of(*ch) == Class::Final);
        let initial_pair = initial.and_then(|at| self.pair_of(chars[at], false));
        let final_pair = final_sound.and_then(|at| self.pair_of(chars[at], true));

        let mut out = Vec::new();
        let mut seen = vec![s.clone()];
        let mut add = |swap: &dyn Fn(&mut [char]), rules: Vec<Pair>| {
            let mut next = chars.clone();
            swap(&mut next);
            let alt = Syllable::from_raw(next.into_iter().collect::<String>());
            if seen.contains(&alt) || !alt.legal() {
                return;
            }
            seen.push(alt.clone());
            out.push(Alt { syl: alt, rules });
        };
        if let (Some(at), Some((pair, to))) = (initial, initial_pair) {
            add(&|chars| chars[at] = to, vec![pair]);
        }
        if let (Some(at), Some((pair, to))) = (final_sound, final_pair) {
            add(&|chars| chars[at] = to, vec![pair]);
        }
        if let (Some(i_at), Some(f_at), Some((i_pair, i_to)), Some((f_pair, f_to))) =
            (initial, final_sound, initial_pair, final_pair)
        {
            add(
                &|chars| {
                    chars[i_at] = i_to;
                    chars[f_at] = f_to;
                },
                vec![i_pair, f_pair],
            );
        }
        out
    }

    /// 把音節序列展開成有限個讀音變體，供以字串讀音建詞格的解碼器使用。
    ///
    /// 每個位置先放原音（成本 0）；沒打聲調的音節先展開五個聲調，再各自加上
    /// 近音替代。最多累計 [`MAX_SWAPS_PER_WORD`] 次換音，成本以
    /// [`SWAP_COST_TENTHS`] 的整數為單位，讓上層能和整數詞格成本相加。
    pub fn variants(self, syls: &[Syllable]) -> Vec<Variant> {
        let mut options: Vec<Vec<(Syllable, u32)>> = Vec::with_capacity(syls.len());
        for syl in syls {
            let mut row = Vec::new();
            for base in syl.tones() {
                row.push((base.clone(), 0));
                if self.enabled() {
                    for alt in self.alternatives(&base) {
                        let cost = SWAP_COST_TENTHS * alt.rules.len() as u32;
                        row.push((alt.syl, cost));
                    }
                }
            }
            let mut unique: Vec<(Syllable, u32)> = Vec::new();
            for item in row {
                match unique.iter_mut().find(|(syl, _)| *syl == item.0) {
                    Some(existing) if existing.1 > item.1 => existing.1 = item.1,
                    Some(_) => {}
                    None => unique.push(item),
                }
            }
            options.push(unique);
        }
        let mut out = Vec::new();
        let mut current = Vec::with_capacity(syls.len());
        walk_variants(&options, 0, 0, &mut current, &mut out);
        out
    }
}

/// 每次換音的通道成本，自然對數尺度。
pub const SWAP_COST: f64 = 1.2;

/// [`SWAP_COST`] 換成整數成本時的單位數（十分之一）。
pub const SWAP_COST_TENTHS: u32 = 12;

/// 單一詞（一次候選查詢）最多可累計的換音次數。
pub const MAX_SWAPS_PER_WORD: usize = 2;

/// 單詞成本上限；加一點餘量避免浮點累加誤差。
pub const MAX_WORD_COST: f64 = MAX_SWAPS_PER_WORD as f64 * SWAP_COST + 1e-9;

fn walk_variants(
    options: &[Vec<(Syllable, u32)>],
    at: usize,
    cost: u32,
    current: &mut Vec<Syllable>,
    out: &mut Vec<Variant>,
) {
    if at == options.len() {
        out.push(Variant {
            syllables: current.clone(),
            cost_tenths: cost,
        });
        return;
    }
    for (syl, extra) in &options[at] {
        let next_cost = cost + extra;
        if next_cost > SWAP_COST_TENTHS * MAX_SWAPS_PER_WORD as u32 {
            continue;
        }
        current.push(syl.clone());
        walk_variants(options, at + 1, next_cost, current, out);
        current.pop();
    }
}

/// 一個音節的近音替代。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alt {
    /// 替代音節。
    pub syl: Syllable,
    /// 用到的配對：聲母配對在前、韻母配對在後。
    pub rules: Vec<Pair>,
}

impl Alt {
    /// 這個替代的通道成本。
    pub fn cost(&self) -> f64 {
        SWAP_COST * self.rules.len() as f64
    }
}

/// 一組讀音變體與它的換音成本（十分之一為單位）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// 每個位置的音節。
    pub syllables: Vec<Syllable>,
    /// 換音成本；原音為 0。
    pub cost_tenths: u32,
}

/// 回傳 `matched` 相對於使用者打的 `typed` 用了哪些配對。
///
/// 兩者相同或差異不是某個配對的符號互換時回傳空清單；診斷與評測用，不看
/// 設定的模式。
pub fn rules(typed: &Syllable, matched: &Syllable) -> Vec<Pair> {
    if typed == matched || !typed.valid() || !matched.valid() {
        return Vec::new();
    }
    let typed_chars: Vec<char> = typed.as_str().chars().collect();
    let matched_chars: Vec<char> = matched.as_str().chars().collect();
    if typed_chars.len() != matched_chars.len() {
        return Vec::new();
    }
    let mut initials = Vec::new();
    let mut finals = Vec::new();
    for (index, typed_ch) in typed_chars.iter().enumerate() {
        let matched_ch = matched_chars[index];
        if *typed_ch == matched_ch {
            continue;
        }
        let class = class_of(*typed_ch);
        for pair in PAIRS {
            let def = &PAIR_DEFS[pair as usize];
            let swapped = (*typed_ch == def.a && matched_ch == def.b)
                || (*typed_ch == def.b && matched_ch == def.a);
            if swapped && def.final_sound == (class == Class::Final) {
                if def.final_sound {
                    finals.push(pair);
                } else {
                    initials.push(pair);
                }
            }
        }
    }
    initials.extend(finals);
    initials
}

/// 把輸入展開成帶成本的音節格：每個位置先放原音（成本 0），再放逐對啟用的近音。
pub struct Frontend {
    /// 使用的模糊音設定。
    pub config: Config,
}

impl crate::zh::input::Frontend for Frontend {
    fn expand(&self, syls: &[Syllable]) -> SyllableLattice {
        self.config
            .widen(&crate::zh::input::Exact.expand(syls), syls, 0, syls.len())
    }
}

impl Config {
    /// 回傳 `lat` 的複本，其中 `[from, to)` 各位置補上近音替代。
    ///
    /// 已存在的假設（例如先前鎖定的近音讀音）不重複加入，也不被較高的成本
    /// 取代。`lat` 本身不被修改。
    pub fn widen(
        self,
        lat: &SyllableLattice,
        syls: &[Syllable],
        from: usize,
        to: usize,
    ) -> SyllableLattice {
        let mut out = lat.clone();
        let start = from;
        let end = to.min(out.len()).min(syls.len());
        for index in start..end {
            let mut row = out[index].clone();
            for tone in syls[index].tones() {
                for alt in self.alternatives(&tone) {
                    if !row.iter().any(|hyp| hyp.syl.as_str() == alt.syl.as_str()) {
                        row.push(Hyp {
                            syl: alt.syl.reading_unit(),
                            cost: alt.cost(),
                            untoned: false,
                        });
                    }
                }
            }
            out[index] = row;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zh::input::Frontend as _;
    use crate::zh::syllable::parse_reading;

    fn all() -> Config {
        Config::all(Mode::Select)
    }

    fn alts(config: Config, s: &str) -> Vec<String> {
        config
            .alternatives(&Syllable::from_raw(s))
            .into_iter()
            .map(|alt| alt.syl.as_str().to_string())
            .collect()
    }

    #[test]
    fn test_alternatives_per_pair() {
        let cases: &[(Pair, &str, &[&str])] = &[
            (Pair::ZZh, "ㄗ", &["ㄓ"]),
            (Pair::ZZh, "ㄓˋ", &["ㄗˋ"]),
            (Pair::ZZh, "ㄗㄞˋ", &["ㄓㄞˋ"]),
            (Pair::CCh, "ㄘ", &["ㄔ"]),
            (Pair::CCh, "ㄔㄞˊ", &["ㄘㄞˊ"]),
            (Pair::SSh, "ㄙㄥ", &["ㄕㄥ"]),
            (Pair::SSh, "ㄙㄢ", &["ㄕㄢ"]),
            (Pair::SSh, "ㄕㄢ", &["ㄙㄢ"]),
            (Pair::EnEng, "ㄆㄣˊ", &["ㄆㄥˊ"]),
            (Pair::EnEng, "ㄧㄥ", &["ㄧㄣ"]),
            (Pair::EnEng, "ㄅㄥˇ", &["ㄅㄣˇ"]),
            (Pair::EnEng, "ㄩㄣˊ", &["ㄩㄥˊ"]),
            (Pair::EnEng, "ㄧㄣ", &["ㄧㄥ"]),
            (Pair::EnEng, "ㄕㄨㄣ", &[]),
            (Pair::SSh, "ㄕㄨㄥˋ", &["ㄙㄨㄥˋ"]),
            (Pair::EnEng, "ㄕㄨㄥˋ", &["ㄕㄨㄣˋ"]),
            (Pair::EnEng, "ㄙㄨㄣ", &["ㄙㄨㄥ"]),
            (Pair::ZZh, "ㄅㄚ", &[]),
            (Pair::ZZh, "ㄗㄠˇ˙", &[]),
        ];
        for (pair, input, want) in cases {
            let got = alts(Config::default().with(*pair, Mode::Select), input);
            let got: Vec<&str> = got.iter().map(String::as_str).collect();
            assert_eq!(got, *want, "{input} 只開 {}：{got:?}", pair.id());
        }
    }

    #[test]
    fn test_alternatives_both_sides() {
        let config = all();
        assert_eq!(
            alts(config, "ㄙㄣ"),
            vec!["ㄕㄣ", "ㄙㄥ", "ㄕㄥ"],
            "替代順序應為聲母、韻母、兩者"
        );
        let got = config.alternatives(&Syllable::from_raw("ㄙㄣ"));
        let last = got.last().unwrap();
        assert_eq!(last.syl.as_str(), "ㄕㄥ");
        assert_eq!(last.rules, vec![Pair::SSh, Pair::EnEng]);
        assert_eq!(last.cost(), 2.0 * SWAP_COST);
        assert_eq!(got[0].cost(), SWAP_COST);
    }

    #[test]
    fn test_only_enabled_pairs_expand() {
        let config = Config::default().with(Pair::ZZh, Mode::Select);
        for input in ["ㄘ", "ㄙㄢ", "ㄆㄣˊ", "ㄧㄥ"] {
            assert!(alts(config, input).is_empty(), "只開 z-zh 不該展開 {input}");
        }
        assert!(config.enabled());
        assert!(!Config::default().enabled());
        assert_eq!(config.key(), "z-zh=select");
        assert_eq!(Config::default().key(), "");
    }

    #[test]
    fn test_alternatives_across_inventory() {
        let config = all();
        let bases = crate::zh::pinyin::inventory();
        assert!(bases.len() >= 400, "音節表只有 {} 個基底", bases.len());
        let mut total = 0;
        for base in &bases {
            for tone in 1..=5 {
                let syl = base.with_tone(tone);
                assert!(Config::default().alternatives(&syl).is_empty());
                let mut seen: Vec<Syllable> = Vec::new();
                for alt in config.alternatives(&syl) {
                    total += 1;
                    assert_ne!(alt.syl, syl, "替代不得含原音");
                    assert!(alt.syl.legal(), "{syl} → {} 不是合法音節", alt.syl);
                    assert_eq!(alt.syl.tone(), syl.tone(), "{syl} → {} 聲調改變", alt.syl);
                    assert!(!seen.contains(&alt.syl), "{syl} → {} 重複", alt.syl);
                    assert!(
                        (1..=2).contains(&alt.rules.len()),
                        "{syl} → {} 規則數 {}",
                        alt.syl,
                        alt.rules.len()
                    );
                    seen.push(alt.syl.clone());
                }
            }
        }
        assert!(total > 0, "音節表沒有產生任何替代");
    }

    #[test]
    fn test_alternatives_from_any_valid_syllable() {
        let config = all();
        let initials: Vec<&str> = vec![
            "", "ㄅ", "ㄆ", "ㄇ", "ㄈ", "ㄉ", "ㄊ", "ㄋ", "ㄌ", "ㄍ", "ㄎ", "ㄏ", "ㄐ", "ㄑ", "ㄒ",
            "ㄓ", "ㄔ", "ㄕ", "ㄖ", "ㄗ", "ㄘ", "ㄙ",
        ];
        let medials = ["", "ㄧ", "ㄨ", "ㄩ"];
        let finals = [
            "", "ㄚ", "ㄛ", "ㄜ", "ㄝ", "ㄞ", "ㄟ", "ㄠ", "ㄡ", "ㄢ", "ㄣ", "ㄤ", "ㄥ", "ㄦ",
        ];
        let mut total = 0;
        let mut illegal_sources = 0;
        for initial in &initials {
            for medial in &medials {
                for final_sound in &finals {
                    let base = Syllable::from_raw(format!("{initial}{medial}{final_sound}"));
                    if !base.valid() {
                        continue;
                    }
                    for tone in 1..=5 {
                        let syl = base.with_tone(tone);
                        if !syl.legal() {
                            illegal_sources += 1;
                        }
                        for alt in config.alternatives(&syl) {
                            total += 1;
                            assert_ne!(alt.syl, syl);
                            assert!(alt.syl.legal());
                            assert_eq!(alt.syl.tone(), syl.tone());
                        }
                    }
                }
            }
        }
        assert!(total > 0 && illegal_sources > 0, "窮舉沒有涵蓋預期情形");
    }

    #[test]
    fn test_branch_distribution_matches_inventory() {
        let config = all();
        let mut single = std::collections::BTreeMap::new();
        let mut both = std::collections::BTreeMap::new();
        for base in crate::zh::pinyin::inventory() {
            let mut one = 1;
            let mut full = 1;
            for alt in config.alternatives(&base) {
                full += 1;
                if alt.rules.len() == 1 {
                    one += 1;
                }
            }
            *single.entry(one).or_insert(0) += 1;
            *both.entry(full).or_insert(0) += 1;
        }
        assert_eq!(crate::zh::pinyin::inventory().len(), 419);
        assert_eq!(single.get(&1), Some(&254));
        assert_eq!(single.get(&2), Some(&144));
        assert_eq!(single.get(&3), Some(&21));
        assert_eq!(both.get(&4), Some(&20));
        assert_eq!(both.get(&1), Some(&254));
        assert!(both.keys().all(|branches| *branches <= 4));
    }

    #[test]
    fn test_widen_keeps_original_first_and_existing_hypotheses() {
        let syls = parse_reading("ㄗ ㄕˋ ㄅㄚ");
        let exact = crate::zh::input::Exact.expand(&syls);
        let config = Config::default()
            .with(Pair::ZZh, Mode::Select)
            .with(Pair::SSh, Mode::Select);
        let lat = config.widen(&exact, &syls, 0, 2);
        assert_eq!(lat[0][0].syl.as_str(), "ㄗ");
        assert_eq!(lat[1][0].syl.as_str(), "ㄕˋ");
        assert_eq!(lat[0].len(), 2);
        assert_eq!(lat[0][1].syl.as_str(), "ㄓ");
        assert_eq!(lat[0][1].cost, SWAP_COST);
        assert_eq!(lat[2].len(), 1, "範圍外的位置不展開");
        assert_eq!(exact[0].len(), 1, "Widen 不得修改傳入的音節格");

        let mut existing = exact;
        existing[0].push(Hyp {
            syl: crate::reading::Unit::new("ㄓ"),
            cost: 9.0,
            untoned: false,
        });
        let again = config.widen(&existing, &syls, 0, 1);
        assert_eq!(again[0].len(), 2);
        assert_eq!(again[0][1].cost, 9.0);
    }

    #[test]
    fn test_frontend_expands_toned_reading() {
        let frontend = Frontend { config: all() };
        let lat = frontend.expand(&parse_reading("ㄧㄣ ㄍㄞ"));
        assert_eq!(lat.len(), 2);
        assert_eq!(lat[0].len(), 2);
        assert_eq!(lat[0][1].syl.as_str(), "ㄧㄥ");
        assert_eq!(lat[1].len(), 1);
    }

    #[test]
    fn test_parse_pair_and_mode() {
        for pair in pairs() {
            assert_eq!(parse_pair(pair.id()), Some(pair));
            assert!(!pair.symbols().is_empty());
        }
        assert_eq!(parse_pair("f-h"), None);
        for (value, want) in [
            ("off", Mode::Off),
            ("select", Mode::Select),
            ("auto", Mode::Auto),
        ] {
            assert_eq!(Mode::parse(value), Some(want));
            assert_eq!(want.as_str(), value);
        }
        assert_eq!(Mode::parse("learn"), None);
    }

    #[test]
    fn test_auto_threshold() {
        let policy = AutoPolicy {
            margin: 6.0,
            short: 3.0,
            per_extra: 1.0,
            neural_extra: 0.0,
        };
        assert_eq!(policy.threshold(1, 1), 9.0);
        assert_eq!(policy.threshold(2, 1), 6.0);
        assert_eq!(policy.threshold(2, 2), 7.0);
        assert_eq!(policy.threshold(4, 3), 8.0);
        assert_eq!(policy.threshold(1, 2), 10.0);
        let default = AutoPolicy::default();
        assert!(default.margin > 0.0 && default.short >= 0.0 && default.per_extra >= 0.0);
    }

    #[test]
    fn test_auto_config() {
        let config = Config::default()
            .with(Pair::ZZh, Mode::Auto)
            .with(Pair::CCh, Mode::Select);
        assert!(config.enabled());
        assert!(config.auto_enabled());
        let only = config.auto_only();
        assert_eq!(only.mode(Pair::ZZh), Mode::Auto);
        assert_eq!(only.mode(Pair::CCh), Mode::Off);
        assert!(!Config::default()
            .with(Pair::CCh, Mode::Select)
            .auto_enabled());
        assert!(!Config::default().auto_enabled());
        assert_eq!(config.key(), "z-zh=auto,c-ch=select");
    }

    #[test]
    fn test_rules() {
        let ids = |rules: Vec<Pair>| -> Vec<String> {
            rules
                .into_iter()
                .map(|pair| pair.id().to_string())
                .collect()
        };
        let cases: &[(&str, &str, &[&str])] = &[
            ("ㄗ", "ㄓ", &["z-zh"]),
            ("ㄔ", "ㄘ", &["c-ch"]),
            ("ㄙㄣ", "ㄕㄥ", &["s-sh", "en-eng"]),
            ("ㄧㄣ", "ㄧㄥ", &["en-eng"]),
            ("ㄅㄚˋ", "ㄅㄚˋ", &[]),
            ("ㄅㄚˋ", "ㄆㄚˋ", &[]),
            ("ㄕㄨㄥˋ", "ㄙㄨㄥˋ", &["s-sh"]),
        ];
        for (typed, matched, want) in cases {
            let got = ids(rules(
                &Syllable::from_raw(*typed),
                &Syllable::from_raw(*matched),
            ));
            let got: Vec<&str> = got.iter().map(String::as_str).collect();
            assert_eq!(got, *want, "rules({typed}, {matched})");
        }
        let config = all();
        for input in ["ㄗㄣ", "ㄕㄥˋ", "ㄘㄥˊ", "ㄧㄣ"] {
            let typed = Syllable::from_raw(input);
            for alt in config.alternatives(&typed) {
                assert_eq!(rules(&typed, &alt.syl), alt.rules, "{input}→{}", alt.syl);
            }
        }
    }

    #[test]
    fn test_variants_keep_original_first_and_cap_swaps() {
        let config = all();
        let syls = parse_reading("ㄗ ㄕˋ");
        let variants = config.variants(&syls);
        assert_eq!(variants[0].syllables, syls);
        assert_eq!(variants[0].cost_tenths, 0);
        assert!(variants
            .iter()
            .any(|variant| variant.syllables[0].as_str() == "ㄓ"));
        assert!(variants
            .iter()
            .all(|variant| variant.cost_tenths <= SWAP_COST_TENTHS * 2));
        assert!(variants
            .iter()
            .filter(|variant| variant.syllables != syls)
            .all(|variant| variant.cost_tenths > 0));
    }

    #[test]
    fn test_variants_expand_untoned_syllables() {
        let config = Config::default();
        let variants = config.variants(&parse_reading("ㄔㄤ?"));
        assert_eq!(variants.len(), 5, "沒打聲調的音節應展開五個聲調");
        assert_eq!(variants[0].syllables[0].as_str(), "ㄔㄤ");
        assert_eq!(variants[4].syllables[0].as_str(), "ㄔㄤ˙");
    }
}

/// 自動修正的門檻（自然對數尺度）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoPolicy {
    /// 基本門檻。
    pub margin: f64,
    /// 修改範圍只有一個音節時額外加的門檻。
    pub short: f64,
    /// 同一處每多換一個音節額外加的門檻。
    pub per_extra: f64,
    /// 顯示字已被神經重排改過時額外加的門檻。
    pub neural_extra: f64,
}

impl Default for AutoPolicy {
    fn default() -> Self {
        Self {
            margin: 6.0,
            short: 3.0,
            per_extra: 1.0,
            neural_extra: 0.0,
        }
    }
}

impl AutoPolicy {
    /// 修改範圍長 `span` 個音節、其中 `swaps` 個音節換了音時的門檻。
    pub fn threshold(&self, span: usize, swaps: usize) -> f64 {
        let mut threshold = self.margin;
        if span <= 1 {
            threshold += self.short;
        }
        if swaps > 1 {
            threshold += self.per_extra * (swaps as f64 - 1.0);
        }
        threshold
    }

    /// 同 [`AutoPolicy::threshold`]，`neural` 為真時加上額外門檻。
    pub fn threshold_for(&self, span: usize, swaps: usize, neural: bool) -> f64 {
        let threshold = self.threshold(span, swaps);
        if neural {
            threshold + self.neural_extra
        } else {
            threshold
        }
    }
}
