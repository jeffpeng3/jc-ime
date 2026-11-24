//! 真實資料包的整合測試。
//!
//! 資料不存在時用 `jc_testdata::require_data!` 跳過，不讓測試失敗。

use super::*;

#[test]
fn real_bigram_pack_matches_manifest() {
    let dir = jc_testdata::require_data!("build-zh-bigram");
    let bundle = Bundle::open(&dir).unwrap();
    assert_eq!(bundle.lexicon.len(), 271_170);
    assert_eq!(bundle.lexicon.longest_word(), 16);
    assert_eq!(bundle.lm.words(), 238_021);
    assert_eq!(bundle.lm.bigrams(), 7_028_326);
    assert_eq!(bundle.manifest.count("entries"), Some(271_170));
    assert_eq!(bundle.manifest.count("words"), Some(238_021));
    assert_eq!(bundle.manifest.count("bigrams"), Some(7_028_326));
}

#[test]
fn real_lexicon_answers_exact_and_prefix() {
    let dir = jc_testdata::require_data!("build-zh");
    let lexicon = Lexicon::open(dir.join("lexicon.bin")).unwrap();

    let exact = lexicon.exact("ㄨㄛˇ");
    assert!(exact.iter().any(|entry| entry.surface == "我"));
    assert!(exact.iter().all(|entry| !entry.surface.is_empty()));

    let prefixed = lexicon.prefixed("ㄨㄛˇ");
    assert!(prefixed.len() > exact.len());
    assert!(lexicon.has_prefix("ㄨㄛˇ"));
    assert!(!lexicon.has_prefix("ㄅㄨˋ ㄨㄛˇ"));

    let wo = lexicon.lookup("我");
    assert_eq!(wo.len(), 2, "我 有 ㄨㄛˇ 與 ㄜˇ 兩個讀音");
    assert!(wo
        .iter()
        .any(|entry| lexicon.reading(entry.index) == "ㄨㄛˇ"));
}

#[test]
fn real_lm_unigram_matches_known_words() {
    let dir = jc_testdata::require_data!("build-zh-bigram");
    let lm = Lm::open(dir.join("lm.bin")).unwrap();
    assert!((lm.uni("我") - 0.005_417_9).abs() < 1e-6);
    assert!(lm.uni("愛") > 0.0);
    assert_eq!(lm.uni("這個詞不存在於詞表"), UNSEEN);
}

#[test]
fn real_bigram_pack_exposes_bigrams() {
    let dir = jc_testdata::require_data!("build-zh-bigram");
    let bundle = Bundle::open(&dir).unwrap();
    assert_eq!(bundle.lm.words(), 238_021);
    assert_eq!(bundle.lm.bigrams(), 7_028_326);
    assert!(bundle.lm.bi("我", "愛") > 0.0);
    assert_eq!(bundle.lm.bi("我", "這個詞不存在"), 0.0);
    assert!(bundle.lm.has_left("我"));
    assert!(!bundle.lm.has_left("這個詞不存在"));
}

#[test]
fn unigram_only_pack_is_rejected() {
    let dir = jc_testdata::require_data!("build-zh");
    let err = Lm::open(dir.join("lm.bin"))
        .err()
        .expect("unigram-only 應被拒絕");
    assert!(
        err.to_string().contains("bigram"),
        "unigram-only lm.bin 要回報必須有 bigram：{err}"
    );
    let err = Bundle::open(&dir).err().expect("unigram-only 應被拒絕");
    assert!(
        err.to_string().contains("bigram"),
        "Bundle::open 也要拒絕 unigram-only 資料包：{err}"
    );
}
