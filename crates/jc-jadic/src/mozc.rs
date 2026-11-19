//! Mozc OSS 詞典來源解析。
//!
//! 只處理任務書列出的三個來源：`dictionary00.txt`..`dictionary09.txt`、
//! `id.def` 與 `connection_single_column.txt`。解析規則與 Go 參考實作
//! `internal/jadic/mozc.go` 一致，錯誤訊息一律帶檔名與行號。

use std::collections::BTreeSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::error::ImportError;

/// `id.def` 建出的 context／POS 對照表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdTable {
    /// 排序去重後的 POS 字串，索引就是 pack 的 POS id。
    pub pos: Vec<String>,
    /// 排序去重後的活用型字串。
    pub conj_types: Vec<String>,
    /// 排序去重後的活用形字串。
    pub conj_forms: Vec<String>,
    /// 每個 context id 對應的 POS 與活用索引。
    pub ids: Vec<IdInfo>,
    /// `max(id) + 1`。
    pub context_count: u16,
}

/// 單一 context id 在 pack 內需要的索引。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdInfo {
    /// POS table 索引。
    pub pos_id: u16,
    /// 活用型索引。
    pub conj_type_id: u8,
    /// 活用形索引。
    pub conj_form_id: u8,
}

/// 一筆通過格式檢查的詞典原始列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRow {
    /// 讀音，仍是來源的片假名。
    pub reading: String,
    /// 詞面。
    pub surface: String,
    /// 左 context id。
    pub left_id: u16,
    /// 右 context id。
    pub right_id: u16,
    /// MeCab 詞成本。
    pub cost: i16,
}

/// 解析 `id.def`。
///
/// 每列是「ID<空白>POS1,POS2,POS3,POS4,活用型,活用形,...」。POS 只取前四欄，
/// 活用型與活用形分別取第五、第六欄；缺欄以 `*` 補齊。
pub fn parse_id_def<R: BufRead>(reader: R, path: &Path) -> Result<IdTable, ImportError> {
    struct Row {
        id: u32,
        pos: String,
        conj_type: String,
        conj_form: String,
    }

    let mut rows: Vec<Row> = Vec::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut max_id: Option<u32> = None;
    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = line.map_err(|source| ImportError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(split) = line.find([' ', '\t']) else {
            return Err(ImportError::parse(path, line_number, "需要 ID 與詞性 CSV"));
        };
        let (id_text, csv) = line.split_at(split);
        let csv = csv.trim();
        if csv.is_empty() {
            return Err(ImportError::parse(path, line_number, "需要 ID 與詞性 CSV"));
        }
        let id: u32 = id_text
            .trim()
            .parse()
            .map_err(|_| ImportError::parse(path, line_number, format!("ID {id_text:?} 不合法")))?;
        if !seen.insert(id) {
            return Err(ImportError::BadId {
                line: line_number,
                id,
                context_count: max_id.unwrap_or(0) + 1,
            });
        }
        max_id = Some(max_id.map_or(id, |m| m.max(id)));

        let fields: Vec<&str> = csv.split(',').map(str::trim).collect();
        let mut field = |i: usize| -> String {
            fields
                .get(i)
                .map(|s| (*s).to_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "*".to_owned())
        };
        let pos = (0..4).map(&mut field).collect::<Vec<_>>().join(",");
        rows.push(Row {
            id,
            pos,
            conj_type: field(4),
            conj_form: field(5),
        });
    }

    if rows.is_empty() {
        return Err(ImportError::parse(path, 0, "沒有任何詞性"));
    }
    let max_id = max_id.unwrap_or(0);
    if max_id > u16::MAX as u32 {
        return Err(ImportError::CountOverflow {
            field: "context_count",
            value: max_id as usize + 1,
            limit: u16::MAX as u64,
        });
    }
    let context_count = (max_id + 1) as u16;
    if context_count as u32 != rows.len() as u32 {
        let missing = (0..context_count as u32)
            .find(|id| !seen.contains(id))
            .unwrap_or(0);
        return Err(ImportError::BadId {
            line: rows.len().max(1),
            id: missing,
            context_count: context_count as u32,
        });
    }

    let pos: Vec<String> = rows
        .iter()
        .map(|r| r.pos.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let conj_types: Vec<String> = rows
        .iter()
        .map(|r| r.conj_type.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let conj_forms: Vec<String> = rows
        .iter()
        .map(|r| r.conj_form.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    if pos.len() > u16::MAX as usize {
        return Err(ImportError::CountOverflow {
            field: "pos_count",
            value: pos.len(),
            limit: u16::MAX as u64,
        });
    }
    for (field, count) in [
        ("conj_type_count", conj_types.len()),
        ("conj_form_count", conj_forms.len()),
    ] {
        if count > u8::MAX as usize {
            return Err(ImportError::CountOverflow {
                field,
                value: count,
                limit: u8::MAX as u64,
            });
        }
    }

    let pos_id =
        |s: &str| -> u16 { pos.iter().position(|p| p == s).expect("POS 已收集") as u16 };
    let conj_type_id = |s: &str| -> u8 {
        conj_types
            .iter()
            .position(|p| p == s)
            .expect("活用型已收集") as u8
    };
    let conj_form_id = |s: &str| -> u8 {
        conj_forms
            .iter()
            .position(|p| p == s)
            .expect("活用形已收集") as u8
    };

    let mut ids = vec![
        IdInfo {
            pos_id: 0,
            conj_type_id: 0,
            conj_form_id: 0,
        };
        context_count as usize
    ];
    for row in &rows {
        ids[row.id as usize] = IdInfo {
            pos_id: pos_id(&row.pos),
            conj_type_id: conj_type_id(&row.conj_type),
            conj_form_id: conj_form_id(&row.conj_form),
        };
    }

    Ok(IdTable {
        pos,
        conj_types,
        conj_forms,
        ids,
        context_count,
    })
}

/// 串流解析一個 `dictionaryNN.txt`，每列呼叫一次 `sink`。
pub fn parse_dictionary<R: BufRead>(
    reader: R,
    path: &Path,
    context_count: u16,
    mut sink: impl FnMut(RawRow),
) -> Result<(), ImportError> {
    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = line.map_err(|source| ImportError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 5 {
            return Err(ImportError::parse(
                path,
                line_number,
                "需要讀音、左 ID、右 ID、成本、詞面五欄",
            ));
        }
        let parse_int = |index: usize, name: &str| -> Result<i64, ImportError> {
            fields[index].trim().parse::<i64>().map_err(|_| {
                ImportError::parse(
                    path,
                    line_number,
                    format!("{name} {:?} 不合法", fields[index]),
                )
            })
        };
        let left = parse_int(1, "左 ID")?;
        let right = parse_int(2, "右 ID")?;
        let cost = parse_int(3, "詞成本")?;
        if left < 0 || right < 0 {
            return Err(ImportError::parse(
                path,
                line_number,
                format!("左 ID {left}、右 ID {right} 不可為負"),
            ));
        }
        if left >= context_count as i64 || right >= context_count as i64 {
            return Err(ImportError::ContextIdRange {
                path: path.to_path_buf(),
                line: line_number,
                left: left as u32,
                right: right as u32,
                context_count: context_count as u32,
            });
        }
        if !(-32767..=32767).contains(&cost) {
            return Err(ImportError::CostRange {
                path: path.to_path_buf(),
                line: line_number,
                cost,
            });
        }
        let surface = fields[4];
        if surface.is_empty() {
            return Err(ImportError::parse(path, line_number, "詞面不可為空"));
        }
        sink(RawRow {
            reading: fields[0].to_owned(),
            surface: surface.to_owned(),
            left_id: left as u16,
            right_id: right as u16,
            cost: cost as i16,
        });
    }
    Ok(())
}

/// 解析 `connection_single_column.txt`。
///
/// 第一列是矩陣維度，之後是 row-major 的成本值。回傳長度必須是
/// `context_count²`。
pub fn parse_connection<R: BufRead>(
    reader: R,
    path: &Path,
    context_count: u16,
) -> Result<Vec<i16>, ImportError> {
    let mut lines = reader.lines().enumerate();
    let mut size: Option<u32> = None;
    let mut costs: Vec<i16> = Vec::new();
    while let Some((index, line)) = lines.next() {
        let line_number = index + 1;
        let line = line.map_err(|source| ImportError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let text = line.trim_end_matches('\r').trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        if size.is_none() {
            let n: u32 = text.parse().map_err(|_| {
                ImportError::parse(path, line_number, format!("矩陣大小 {text:?} 不合法"))
            })?;
            if n != context_count as u32 {
                return Err(ImportError::MatrixSize {
                    actual: n,
                    expected: context_count as u32,
                });
            }
            size = Some(n);
            costs.reserve_exact(n as usize * n as usize);
            continue;
        }
        let value: i64 = text.parse().map_err(|_| {
            ImportError::parse(path, line_number, format!("連接成本 {text:?} 不合法"))
        })?;
        if !(-32767..=32767).contains(&value) {
            return Err(ImportError::ConnectionRange {
                path: path.to_path_buf(),
                line: line_number,
                value,
            });
        }
        costs.push(value as i16);
    }

    let Some(n) = size else {
        return Err(ImportError::parse(path, 0, "是空檔，缺少矩陣大小"));
    };
    let expected = n as usize * n as usize;
    if costs.len() != expected {
        return Err(ImportError::parse(
            path,
            costs.len() + 1,
            format!("連接成本有 {} 筆，預期 {expected} 筆", costs.len()),
        ));
    }
    Ok(costs)
}

/// 回傳 `dictionary00.txt`..`dictionary09.txt` 的完整路徑。
pub fn dictionary_files(dir: &Path) -> Result<Vec<PathBuf>, ImportError> {
    let mut out = Vec::with_capacity(10);
    for i in 0..10 {
        let path = dir.join(format!("dictionary{i:02}.txt"));
        if !path.is_file() {
            return Err(ImportError::Io {
                path: path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "缺少 Mozc 詞典檔"),
            });
        }
        out.push(path);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::PathBuf;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    #[test]
    fn id_def_builds_sorted_tables_and_indexes() {
        let text = "\
0 BOS/EOS,*,*,*,*,*,*
1 名詞,一般,*,*,*,*,*
2 動詞,自立,*,*,五段・カ行,基本形,*
";
        let table = parse_id_def(Cursor::new(text), &p("id.def")).expect("解析 id.def");
        assert_eq!(table.context_count, 3);
        assert_eq!(
            table.pos,
            vec![
                "BOS/EOS,*,*,*".to_owned(),
                "動詞,自立,*,*".to_owned(),
                "名詞,一般,*,*".to_owned()
            ]
        );
        assert_eq!(
            table.conj_types,
            vec!["*".to_owned(), "五段・カ行".to_owned()]
        );
        assert_eq!(table.conj_forms, vec!["*".to_owned(), "基本形".to_owned()]);
        assert_eq!(
            table.ids[2],
            IdInfo {
                pos_id: 1,
                conj_type_id: 1,
                conj_form_id: 1
            }
        );
    }

    #[test]
    fn id_def_error_carries_file_and_line() {
        let text = "0 BOS/EOS,*,*,*,*,*,*\nbroken\n";
        let err = parse_id_def(Cursor::new(text), &p("id.def")).expect_err("應失敗");
        assert_eq!(err.to_string(), "id.def 第 2 行：需要 ID 與詞性 CSV");
    }

    #[test]
    fn dictionary_parses_five_fields_and_ignores_extra() {
        let text = "\
あい\t10\t20\t1234\t愛
かき\t10\t20\t2000\t柿\tSPELLING_CORRECTION
# 註解
";
        let mut rows = Vec::new();
        parse_dictionary(Cursor::new(text), &p("dictionary00.txt"), 2672, |row| {
            rows.push(row)
        })
        .expect("解析詞典");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].reading, "あい");
        assert_eq!(rows[0].surface, "愛");
        assert_eq!(rows[0].left_id, 10);
        assert_eq!(rows[0].right_id, 20);
        assert_eq!(rows[0].cost, 1234);
        assert_eq!(rows[1].surface, "柿");
    }

    #[test]
    fn dictionary_error_carries_file_and_line() {
        let text = "あい\t10\t20\t1234\t愛\nあい\t10\t20\n";
        let mut rows = Vec::new();
        let err = parse_dictionary(Cursor::new(text), &p("dictionary00.txt"), 2672, |row| {
            rows.push(row)
        })
        .expect_err("應失敗");
        assert_eq!(
            err.to_string(),
            "dictionary00.txt 第 2 行：需要讀音、左 ID、右 ID、成本、詞面五欄"
        );
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn dictionary_rejects_out_of_range_cost() {
        let text = "あい\t1\t1\t40000\t愛\n";
        let err = parse_dictionary(Cursor::new(text), &p("dictionary00.txt"), 2672, |_| {})
            .expect_err("應失敗");
        assert!(matches!(err, ImportError::CostRange { cost: 40000, .. }));
        assert!(err.to_string().contains("dictionary00.txt 第 1 行"));
    }

    #[test]
    fn connection_reads_row_major_and_matches_size() {
        let text = "3\n0\n1\n2\n3\n4\n5\n6\n7\n8\n";
        let costs = parse_connection(Cursor::new(text), &p("connection.txt"), 3).expect("解析矩陣");
        assert_eq!(costs, (0..9).collect::<Vec<i16>>());
        assert_eq!(costs[1 * 3 + 2], 5);
    }

    #[test]
    fn connection_rejects_wrong_size_and_range() {
        let err =
            parse_connection(Cursor::new("4\n"), &p("connection.txt"), 3).expect_err("維度不符");
        assert!(matches!(
            err,
            ImportError::MatrixSize {
                actual: 4,
                expected: 3
            }
        ));

        let err = parse_connection(
            Cursor::new("3\n0\n0\n0\n0\n0\n0\n0\n0\n40000\n"),
            &p("connection.txt"),
            3,
        )
        .expect_err("值域不符");
        assert!(matches!(
            err,
            ImportError::ConnectionRange { value: 40000, .. }
        ));
    }
}
