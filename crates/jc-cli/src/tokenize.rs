//! `tokenize ja`：印出 SentencePiece unigram 與最長前綴匹配的切法對照。
//!
//! 兩個切法共用同一份詞表，只有「有沒有 piece 分數」不同；`--diff` 只印
//! 兩者不同的句子，用來解釋 `rerank-fix` 留下的 tokenizer 近似誤差。

use std::io::{BufRead, Write};
use std::path::PathBuf;

use jc_model::Tokenizer;

use crate::cli::CliError;
use crate::data;

/// 預設 tokenizer 檔名，相對於資料根目錄的 `models/ja-tokenizer/`。
pub const DEFAULT_MODEL: &str = "ja-tokenizer/public_core_ja.model";

/// `tokenize ja` 的參數。
#[derive(Debug, Clone)]
pub struct TokenizeOptions {
    /// `0c-testdata` 目錄。
    pub testdata: Option<PathBuf>,
    /// tokenizer 檔；未指定時用資料根目錄的 `models/ja-tokenizer/public_core_ja.model`。
    pub vocab: Option<PathBuf>,
    /// 要切分的文字；空的時候從 stdin 逐行讀。
    pub texts: Vec<String>,
    /// 只印兩種切法不同的句子。
    pub diff: bool,
    /// 輸出 JSON。
    pub json: bool,
}

/// 執行 `tokenize ja`。
pub fn ja(options: TokenizeOptions, out: &mut dyn Write) -> Result<(), CliError> {
    let path = match &options.vocab {
        Some(path) => path.clone(),
        None => {
            let dir = data::testdata_dir(options.testdata.clone())?;
            data::data_root(&dir).join("models").join(DEFAULT_MODEL)
        }
    };
    let unigram = Tokenizer::load(&path)
        .map_err(|err| CliError::data(format!("讀取 {} 失敗：{err}", path.display())))?;
    let max_match = unigram.as_max_match();

    let texts = if options.texts.is_empty() {
        read_stdin()?
    } else {
        options.texts.clone()
    };

    if options.json {
        let rows: Vec<serde_json::Value> = texts
            .iter()
            .map(|text| row_json(&unigram, &max_match, text))
            .filter(|row| !options.diff || differs(row))
            .collect();
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&rows)
                .map_err(|err| CliError::eval(format!("輸出 JSON 失敗：{err}")))?
        )?;
        return Ok(());
    }

    let mut printed = 0;
    for text in &texts {
        let row = row_json(&unigram, &max_match, text);
        if options.diff && !differs(&row) {
            continue;
        }
        printed += 1;
        writeln!(out, "{text}")?;
        writeln!(out, "  unigram  {}", describe(&row, "unigram"))?;
        writeln!(out, "  maxmatch {}", describe(&row, "maxmatch"))?;
    }
    if options.diff {
        eprintln!("{printed} / {} 句切法不同", texts.len());
    }
    Ok(())
}

fn read_stdin() -> Result<Vec<String>, CliError> {
    let stdin = std::io::stdin();
    let mut lines = Vec::new();
    for line in stdin.lock().lines() {
        let line = line?;
        if !line.trim().is_empty() {
            lines.push(line);
        }
    }
    Ok(lines)
}

fn row_json(unigram: &Tokenizer, max_match: &Tokenizer, text: &str) -> serde_json::Value {
    serde_json::json!({
        "input": text,
        "unigram": encode_json(unigram, text),
        "maxmatch": encode_json(max_match, text),
    })
}

fn encode_json(tokenizer: &Tokenizer, text: &str) -> serde_json::Value {
    let ids = tokenizer.encode(text);
    let pieces: Vec<&str> = ids
        .iter()
        .map(|&id| tokenizer.piece(id).unwrap_or("<missing>"))
        .collect();
    serde_json::json!({"ids": ids, "pieces": pieces})
}

fn differs(row: &serde_json::Value) -> bool {
    row["unigram"]["ids"] != row["maxmatch"]["ids"]
}

fn describe(row: &serde_json::Value, key: &str) -> String {
    let ids = row[key]["ids"]
        .as_array()
        .map(|ids| {
            ids.iter()
                .map(|id| id.as_u64().unwrap_or(0).to_string())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    let pieces = row[key]["pieces"]
        .as_array()
        .map(|pieces| {
            pieces
                .iter()
                .map(|p| p.as_str().unwrap_or("?"))
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default();
    format!("[{ids}] {pieces}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_renders_ids_and_pieces() {
        let row = serde_json::json!({
            "input": "東京",
            "unigram": {"ids": [1, 2], "pieces": ["▁東", "京"]},
            "maxmatch": {"ids": [3], "pieces": ["▁東京"]},
        });
        assert!(differs(&row));
        assert_eq!(describe(&row, "unigram"), "[1,2] ▁東|京");
        assert_eq!(describe(&row, "maxmatch"), "[3] ▁東京");
        let same = serde_json::json!({
            "input": "東京",
            "unigram": {"ids": [3], "pieces": ["▁東京"]},
            "maxmatch": {"ids": [3], "pieces": ["▁東京"]},
        });
        assert!(!differs(&same));
    }

    #[test]
    fn default_model_path_is_under_the_data_root() {
        assert_eq!(DEFAULT_MODEL, "ja-tokenizer/public_core_ja.model");
    }
}
