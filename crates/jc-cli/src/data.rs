//! 評測與互動模式共用的資料載入。
//!
//! 實際載入程式碼在 [`jc_engine::loader`]，引擎行程用同一份，避免兩邊對
//! 同一個資料包得到不同排序。這個模組只保留 CLI 專屬的路徑解析、個人化
//! 資料與錯誤型別轉換。
//!
//! 注音用 `0c-testdata/golden/zh/data` 的固定 TSV（與前身同一份），日文用
//! `packs/` 的詞典 pack 與 `ja-src/connection.tsv`。路徑都可以用環境變數
//! 覆寫，找不到時回傳帶路徑的錯誤，不 panic。

use std::path::{Path, PathBuf};

use jc_engine::loader;
use jc_engine::loader::LoadError;

use crate::cli::CliError;

pub use jc_engine::loader::{TsvWordSource, ZhResources, DEFAULT_JA_PACK, FALLBACK_JA_PACK};

/// 沒有指定 `--testdata` 時使用的 `0c-testdata` 目錄。
pub const DEFAULT_TESTDATA: &str = "/mnt/work/jc/jc-ime-data/0c-testdata";

/// 個人化學習資料檔名。
pub const PERSONAL_FILE: &str = "personal.json";

/// 把共用載入層的錯誤轉成 CLI 錯誤。
fn to_cli(err: LoadError) -> CliError {
    CliError::data(err.to_string())
}

/// 解析 `0c-testdata` 目錄。
///
/// 順序：`--testdata`、`JC_TESTDATA` 環境變數、[`DEFAULT_TESTDATA`]。
/// 目錄必須存在。
pub fn testdata_dir(explicit: Option<PathBuf>) -> Result<PathBuf, CliError> {
    if let Some(dir) = explicit {
        return check_dir(dir);
    }
    if let Some(value) = std::env::var_os("JC_TESTDATA") {
        if !value.is_empty() {
            let base = PathBuf::from(value);
            let nested = base.join("0c-testdata");
            return if nested.is_dir() {
                Ok(nested)
            } else {
                check_dir(base)
            };
        }
    }
    check_dir(PathBuf::from(DEFAULT_TESTDATA))
}

/// 由 `0c-testdata` 目錄回推資料根目錄（`packs/`、`ja-src/` 所在處）。
///
/// `--testdata` 若直接指向 `0c-testdata` 的上層，也接受。
pub fn data_root(testdata: &Path) -> PathBuf {
    if testdata
        .file_name()
        .map(|name| name == "0c-testdata")
        .unwrap_or(false)
    {
        testdata
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| testdata.to_path_buf())
    } else {
        testdata.to_path_buf()
    }
}

fn check_dir(dir: PathBuf) -> Result<PathBuf, CliError> {
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(CliError::data(format!("找不到題庫目錄：{}", dir.display())))
    }
}

/// 從使用者資料目錄載入個人化學習資料。
///
/// 檔案不存在時回傳空資料；格式錯誤直接報錯，不靜默覆蓋使用者的學習結果。
pub fn load_personal(dir: &Path) -> Result<jc_core::personal::PersonalStore, CliError> {
    let path = dir.join(PERSONAL_FILE);
    if !path.is_file() {
        return Ok(jc_core::personal::PersonalStore::new());
    }
    jc_core::personal::PersonalStore::load(&path)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", path.display())))
}

/// 把個人化學習資料寫回使用者資料目錄。
pub fn save_personal(dir: &Path, store: &jc_core::personal::PersonalStore) -> Result<(), CliError> {
    let path = dir.join(PERSONAL_FILE);
    store
        .save(&path)
        .map_err(|err| CliError::data(format!("寫入 {} 失敗：{err}", path.display())))
}

/// 載入注音固定詞典與成本設定。
pub fn load_zh_resources(data_dir: &Path) -> Result<ZhResources, CliError> {
    loader::load_zh_resources(data_dir).map_err(to_cli)
}

/// 載入 golden 的 TSV 詞典，另外接上 `lm.bin` 的 bigram。
pub fn load_zh_resources_with_lm(data_dir: &Path, lm_path: &Path) -> Result<ZhResources, CliError> {
    loader::load_zh_resources_with_lm(data_dir, lm_path).map_err(to_cli)
}

/// 載入注音二進位資料包（`lexicon.bin`、`lm.bin`、`manifest.json`）。
pub fn load_zh_bin_resources(data_dir: &Path) -> Result<ZhResources, CliError> {
    loader::load_zh_bin_resources(data_dir).map_err(to_cli)
}

/// 載入注音二進位資料包，`lm_path` 可覆寫資料包內的 `lm.bin`。
pub fn load_zh_bin_resources_with_lm(
    data_dir: &Path,
    lm_path: Option<&Path>,
) -> Result<ZhResources, CliError> {
    loader::load_zh_bin_resources_with_lm(data_dir, lm_path).map_err(to_cli)
}

/// 解析日文 pack 路徑。
pub fn resolve_ja_pack(root: &Path, explicit: Option<&Path>) -> Result<PathBuf, CliError> {
    loader::resolve_ja_pack(root, explicit).map_err(to_cli)
}

/// 載入日文解碼器，使用預設 pack（見 [`resolve_ja_pack`]）。
pub fn load_ja_decoder(root: &Path) -> Result<jc_core::ja::decode::JaDecoder, CliError> {
    loader::load_ja_decoder(root).map_err(to_cli)
}

/// 模型集合根目錄：`JC_JA_MODELS`，否則資料根目錄下的 `models/`。
pub fn ja_models_dir(root: &Path) -> PathBuf {
    loader::ja_models_dir(root)
}

/// 載入日文 reranker。
pub fn load_ja_reranker(
    root: &Path,
) -> Result<std::sync::Arc<jc_core::ja::rerank::NeuralReranker>, CliError> {
    loader::load_ja_reranker(root).map_err(to_cli)
}

/// 載入日文解碼器，`pack` 覆寫預設 pack。
pub fn load_ja_decoder_with_pack(
    root: &Path,
    pack: Option<&Path>,
) -> Result<jc_core::ja::decode::JaDecoder, CliError> {
    loader::load_ja_decoder_with_pack(root, pack).map_err(to_cli)
}

/// 載入 OpenCC tw2s 字典；資料在資料根目錄的 `opencc/`。
pub fn load_zhconv(root: &Path) -> Result<jc_core::zhconv::Converter, CliError> {
    loader::load_zhconv(root).map_err(to_cli)
}

/// 模型集合根目錄。
///
/// 依序使用 `JC_MODELS`、`--testdata` 的上層 `models/`，最後退回預設
/// `0c-testdata` 的上層。目錄不存在時仍回傳路徑，由載入端報錯。
pub fn models_root(testdata: Option<&Path>) -> PathBuf {
    match testdata {
        Some(dir) => loader::models_dir(&data_root(dir)),
        None => loader::models_dir(&data_root(Path::new(DEFAULT_TESTDATA))),
    }
}

/// 載入提交時補標點的模型。
pub fn load_punctuator(
    testdata: Option<&Path>,
    mode: jc_core::Mode,
) -> Result<jc_core::output::OnnxPunctuator, CliError> {
    loader::load_punctuator(&models_root(testdata), &[mode]).map_err(to_cli)
}

/// 載入並安裝注音選字模型。
pub fn install_select_model(testdata: Option<&Path>) -> Result<(), CliError> {
    loader::install_select_model(&models_root(testdata)).map_err(to_cli)?;
    eprintln!(
        "警告：zh-select 使用 0.5／3.0 的保守參數，已預設啟用但未經 Product dev 校準；\
         需要關閉時用 --select-model off。"
    );
    Ok(())
}

/// 關閉注音選字模型。
pub fn disable_select_model() {
    loader::disable_select_model();
}

/// 選字模型的推論延遲統計。
pub fn select_model_stats() -> Option<jc_core::zh::select::LatencyStats> {
    loader::select_model_stats()
}

/// 由注音資源建立 session 設定。
pub fn zh_session_config(
    mode: jc_core::Mode,
    resources: &ZhResources,
) -> jc_core::session::SessionConfig {
    loader::zh_session_config(mode, resources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc_core::score::BigramLm;
    use jc_engine::loader::load_curated_decoder;

    #[test]
    fn probability_cost_orders_rare_words_later() {
        assert_eq!(loader::probability_cost(1.0), 0);
        assert!(loader::probability_cost(0.2) > loader::probability_cost(1.0));
        assert_eq!(loader::probability_cost(0.0), 100_000);
    }

    #[test]
    fn testdata_falls_back_to_missing_error() {
        let err = testdata_dir(Some(PathBuf::from("/no/such/jc-testdata"))).unwrap_err();
        assert!(err.to_string().contains("/no/such/jc-testdata"));
    }

    #[test]
    fn bin_resources_load_lm_and_bigrams() {
        let Some(plain) = jc_testdata::path("build-zh") else {
            eprintln!("缺少測試資料：build-zh");
            return;
        };
        let bigram = jc_testdata::require_data!("build-zh-bigram");
        let plain = load_zh_bin_resources(&plain).unwrap();
        let bigram = load_zh_bin_resources(&bigram).unwrap();
        let plain_lm = plain.lm.expect("build-zh 仍有 lm.bin");
        let bigram_lm = bigram.lm.expect("build-zh-bigram 有 lm.bin");
        assert_eq!(plain_lm.bigrams(), 0);
        assert!(bigram_lm.bigrams() > 0);
        assert!((bigram_lm.bi("我", "愛") - 0.0044).abs() < 0.001);
        assert_eq!(bigram.ctx.bigram_mu, 0.7);
    }

    #[test]
    fn tsv_resources_can_attach_bigram_lm() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let golden = root.join("0c-testdata/golden/zh/data");
        let lm = jc_testdata::require_data!("build-zh-bigram/lm.bin");
        let resources = load_zh_resources_with_lm(&golden, &lm).unwrap();
        let zh_lm = resources.lm.expect("指定 --lm 時要接上模型");
        assert!(zh_lm.bigrams() > 0);
        // unigram 改用 lm.bin 的數值：我 的機率與 golden lm.tsv 不同。
        assert!((zh_lm.uni("我") - 0.0054).abs() < 0.0002);
    }

    #[test]
    fn default_pack_converts_a_sentence() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let decoder = load_ja_decoder(&root).unwrap();
        let results = decoder.nbest(
            "わたしはほんをよむ。",
            &jc_core::ja::decode::JaContext::default(),
            1,
        );
        assert_eq!(
            results.first().map(|result| result.surface.as_str()),
            Some("私は本を読む。")
        );
    }

    #[test]
    fn curated_ja_lexicon_converts_a_sentence() {
        let Some(root) = jc_testdata::root() else {
            eprintln!("缺少測試資料：JC_TESTDATA");
            return;
        };
        let decoder = load_curated_decoder(
            &root.join("ja-src/lexicon/words.tsv"),
            &root.join("ja-src/connection.tsv"),
        )
        .unwrap();
        let results = decoder.nbest(
            "わたしはほんをよむ。",
            &jc_core::ja::decode::JaContext::default(),
            1,
        );
        assert_eq!(
            results.first().map(|result| result.surface.as_str()),
            Some("私は本を読む。")
        );
    }
}
