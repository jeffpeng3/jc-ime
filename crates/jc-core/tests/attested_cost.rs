//! 非主要讀音的單字成本懲罰：真實 `build-zh-bigram` 資料的行為回歸。
//!
//! 「日」的 `ㄇㄧˋ` 是 CNS11643 的罕讀，詞典成本卻比 `ㄖˋ` 低；執行期
//! 要把「不是該字最低成本」的單字讀音往後排，但不能讓它消失。多字詞有
//! 自己的詞條成本，不受影響。

use std::sync::OnceLock;

use jc_core::decode::{Decoder, Locks, DEFAULT_BEAM};
use jc_core::lattice::{InMemoryLexicon, Lattice, LexEntry};
use jc_core::score::{probability_cost, BigramLm, LmScorer, ScoreContext, ZhLm};
use jc_core::zh::syllable::parse_reading;

struct Fixture {
    lexicon: InMemoryLexicon,
    lm: ZhLm,
    ctx: ScoreContext,
}

fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let dir = jc_testdata::path("build-zh-bigram")?;
            let bundle = jc_data::zh::Bundle::open(&dir).ok()?;
            let lm = ZhLm::from_lm(bundle.lm);
            let mut lexicon = InMemoryLexicon::new();
            for view in bundle.lexicon.entries() {
                let unigram = lm.uni(view.surface);
                let cost =
                    probability_cost(view.prw as f64).saturating_add(probability_cost(unigram));
                lexicon.push(LexEntry::new(
                    view.surface,
                    bundle.lexicon.reading(view.index),
                    cost,
                    0,
                ));
            }
            Some(Fixture {
                lexicon,
                lm,
                ctx: ScoreContext {
                    unknown_cost: 30_000,
                    ..ScoreContext::default()
                },
            })
        })
        .as_ref()
}

fn nbest(reading: &str, n: usize) -> Vec<String> {
    let Some(fixture) = fixture() else {
        eprintln!("缺少 build-zh-bigram，跳過非主要讀音測試");
        return Vec::new();
    };
    let units: Vec<String> = parse_reading(reading)
        .iter()
        .map(|syllable| syllable.as_str().to_string())
        .collect();
    let lattice = Lattice::build(&units, &fixture.lexicon, &fixture.ctx).expect("建立詞格");
    let scorer = LmScorer::new(&fixture.ctx, &fixture.lm);
    let mut decoder = Decoder::new(&fixture.lexicon, &fixture.ctx).with_scorer(&scorer);
    decoder.beam = DEFAULT_BEAM;
    let (hypotheses, _) = decoder.nbest(&lattice, &Locks::new(), false, n);
    hypotheses
        .into_iter()
        .map(|hypothesis| hypothesis.text)
        .collect()
}

#[test]
fn mi4_ranks_common_readings_first_but_keeps_ri_in_first_page() {
    let candidates = nbest("ㄇㄧˋ", 18);
    if candidates.is_empty() {
        return;
    }
    assert_eq!(candidates[0], "密", "候選：{candidates:?}");
    let mut first_three = candidates[..3].to_vec();
    first_three.sort();
    assert_eq!(first_three, ["密", "秘", "蜜"], "候選：{candidates:?}");

    let ri = candidates
        .iter()
        .position(|surface| surface == "日")
        .unwrap_or_else(|| panic!("日 應仍留在前 18 名：{candidates:?}"));
    assert!(ri > 0, "日 不應再是第 1 名：{candidates:?}");
    assert!(ri < 18, "日 應留在前 18 名：{candidates:?}");
}

#[test]
fn ri4_keeps_ri_first() {
    let candidates = nbest("ㄖˋ", 6);
    if candidates.is_empty() {
        return;
    }
    assert_eq!(
        candidates[0], "日",
        "最低成本的讀音不應被懲罰：{candidates:?}"
    );
}

#[test]
fn de5_keeps_de_first() {
    let candidates = nbest("ㄉㄜ˙", 6);
    if candidates.is_empty() {
        return;
    }
    assert_eq!(candidates[0], "的", "候選：{candidates:?}");
}

#[test]
fn di2_keeps_di_and_de_candidates() {
    let candidates = nbest("ㄉㄧˊ", 18);
    if candidates.is_empty() {
        return;
    }
    assert!(
        candidates[..5].iter().any(|surface| surface == "迪"),
        "候選：{candidates:?}"
    );
    assert!(
        candidates[..5].iter().any(|surface| surface == "敵"),
        "候選：{candidates:?}"
    );
    assert!(
        candidates.iter().any(|surface| surface == "的"),
        "的 仍應可選：{candidates:?}"
    );
}

#[test]
fn multi_character_words_are_not_penalized() {
    let purpose = nbest("ㄇㄨˋ ㄉㄧˋ", 6);
    if purpose.is_empty() {
        return;
    }
    assert_eq!(purpose[0], "目的", "候選：{purpose:?}");

    let secret = nbest("ㄇㄧˋ ㄇㄧˋ", 6);
    assert_eq!(secret[0], "秘密", "候選：{secret:?}");
    assert_eq!(secret[1], "祕密", "候選：{secret:?}");
}
