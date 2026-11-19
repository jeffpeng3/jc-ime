//! 日文轉換的成本拆解診斷工具。
//!
//! 對一段かな讀音印出：
//!
//! 1. 這次載入的 pack 絕對路徑、SHA-256 與能力；
//! 2. lattice 上每個詞弧的 surface／reading／left_id／right_id／pos／cost；
//! 3. 所有完整路徑的詞成本總和、連接成本總和與最終成本；
//! 4. 解碼器實際回傳的 N-best，用來對照第 3 步。
//!
//! 用法：
//!
//! ```text
//! cargo run -p jc-cli --release --bin ja-cost -- わたしわ
//! cargo run -p jc-cli --release --bin ja-cost -- --pack <pack> わたしわ
//! ```
//!
//! 成本全部是 Mozc 原始整數。解碼器內部除以 1000（[`COST_SCALE`]）再乘回去，
//! 對整數成本是恆等轉換，因此本工具直接印原始整數。

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use jc_core::ja::decode::{
    JaBuilder, JaContext, JaDecoder, JaDictionary, JaNode, DEFAULT_UNKNOWN_WORD_COST,
};
use jc_core::ja::lexicon::{JaConnectionCost, JaLexicon};

/// 解碼器沒有 context 矩陣時的詞性連接成本。
///
/// `JaDecoder::new` 的 `conn` 是空白的 [`jc_core::ja::lexicon::SingleColumnCost`]，
/// 每一組詞性都退回 `DEFAULT_CONNECTION_COST`（1000）。
const POS_FALLBACK_CONNECTION: i32 = 1_000;

/// 命令列參數。
struct Args {
    /// 讀音。
    reading: String,
    /// 覆寫 pack 路徑。
    pack: Option<PathBuf>,
    /// `0c-testdata` 目錄。
    testdata: Option<PathBuf>,
    /// 最多印幾條路徑。
    limit: usize,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ja-cost: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(std::env::args().skip(1).collect())?;
    let testdata = jc_cli::data::testdata_dir(args.testdata.clone()).map_err(|e| e.to_string())?;
    let root = jc_cli::data::data_root(&testdata);
    let pack = jc_engine::loader::resolve_ja_pack(&root, args.pack.as_deref())
        .map_err(|e| e.to_string())?;
    let pack = std::fs::canonicalize(&pack).map_err(|e| format!("{}：{e}", pack.display()))?;

    let lex = Arc::new(JaLexicon::open(&pack).map_err(|e| e.to_string())?);
    let raw_pack = lex.pack();
    let caps = raw_pack.capabilities();
    println!("pack: {}", pack.display());
    println!("file_sha256: {}", raw_pack.file_sha256());
    println!("payload_sha256: {}", raw_pack.payload_sha256());
    println!(
        "bytes: {}  entries: {}  format: v{}  context_ids: {}  matrix: {}  context_count: {}",
        raw_pack.bytes(),
        raw_pack.entry_count(),
        raw_pack.format_version(),
        caps.context_ids,
        caps.connection_matrix,
        raw_pack.context_count()
    );

    let units: Vec<char> = args.reading.chars().collect();
    if units.is_empty() {
        return Err("讀音是空的".to_string());
    }
    println!("reading: {} ({} units)", args.reading, units.len());
    println!(
        "decoder: max_word_units=32 unknown_word_cost={} prefer_hiragana=false lm_weight=0 reranker=off",
        DEFAULT_UNKNOWN_WORD_COST
    );

    let dict = JaDictionary::new(Arc::clone(&lex));
    let matrix = lex.connection_cost();
    let lat = JaBuilder {
        source: Some(&dict),
        ..JaBuilder::default()
    }
    .build(&units);

    println!();
    println!("== lattice ==");
    println!(
        "{:>5} {:>3} {:>3}  {:<10} {:<10} {:<8} {:>5} {:>5} {:>7}  {:<28} unknown",
        "start", "end", "len", "surface", "reading", "pos_class", "lid", "rid", "cost", "pos"
    );
    for start in 0..units.len() {
        for node in lat.nodes(start) {
            println!(
                "{:>5} {:>3} {:>3}  {:<10} {:<10} {:<8} {:>5} {:>5} {:>7}  {:<28} {}",
                node.start,
                node.end,
                node.units(),
                node.entry.surface,
                node.entry.reading,
                format!("{:?}", node.entry.pos_class()),
                opt_id(node.left_id),
                opt_id(node.right_id),
                node.entry.cost,
                node.entry.pos,
                node.unknown
            );
        }
    }

    // 同一讀音的不同詞條，用來確認候選有沒有被過濾掉。
    for (start, reading) in [(0usize, "わたし"), (2usize, "わ")] {
        if start >= units.len() {
            continue;
        }
        let want: Vec<char> = reading.chars().collect();
        if units.get(start..start + want.len()) != Some(&want[..]) {
            continue;
        }
        println!();
        println!("== 讀音 {:?} 的全部詞條（pack 原始資料） ==", reading);
        println!(
            "{:>10} {:>5} {:>5} {:>7}  {:<28}",
            "surface", "lid", "rid", "cost", "pos"
        );
        for entry in raw_pack.entries() {
            if entry.reading.as_ref() != reading {
                continue;
            }
            println!(
                "{:>10} {:>5} {:>5} {:>7}  {:<28}",
                entry.surface,
                opt_id(entry.left_id()),
                opt_id(entry.right_id()),
                entry.cost,
                raw_pack.pos_str(entry.pos_id).unwrap_or("?")
            );
        }
    }

    let paths = enumerate(&lat, units.len());
    println!();
    println!("== 全部路徑（共 {} 條，依最終成本排序） ==", paths.len());
    println!(
        "{:>4}  {:<12} {:>7} {:>8} {:>9} {:>9} {:>10}",
        "#", "surface", "words", "nodes", "conn", "total", "total_pos"
    );
    let mut ranked: Vec<PathCost> = paths
        .iter()
        .map(|path| cost_path(path, &matrix, caps.connection_matrix))
        .collect();
    ranked.sort_by(|a, b| {
        a.total
            .cmp(&b.total)
            .then_with(|| a.surface.cmp(&b.surface))
    });
    for (index, pc) in ranked.iter().enumerate() {
        if index >= args.limit {
            println!("... 其餘 {} 條省略", ranked.len() - args.limit);
            break;
        }
        println!(
            "{:>4}  {:<12} {:>7} {:>8} {:>9} {:>9} {:>10}",
            index + 1,
            pc.surface,
            pc.words,
            pc.nodes,
            pc.conn,
            pc.total,
            pc.total_pos()
        );
    }

    println!();
    println!("== 前 3 條路徑的成本拆解 ==");
    for pc in ranked.iter().take(3) {
        println!("{}：{}", pc.surface, pc.total);
        for (label, value) in &pc.word_terms {
            println!("  詞成本  {label} = {value}");
        }
        println!("  詞成本小計 = {}", pc.words);
        for (label, value) in &pc.conn_terms {
            println!("  連接成本 {label} = {value}");
        }
        println!("  連接成本小計 = {}", pc.conn);
        println!("  合計 = {} + {} = {}", pc.words, pc.conn, pc.total);
        println!();
    }

    println!(
        "== 對照：前 3 條路徑若改用固定詞性連接（每段 {}） ==",
        POS_FALLBACK_CONNECTION
    );
    println!(
        "{:<12} {:>7} {:>9} {:>9} {:>10}",
        "surface", "words", "conn_pos", "total_pos", "matrix_total"
    );
    for pc in ranked.iter().take(3) {
        println!(
            "{:<12} {:>7} {:>9} {:>9} {:>10}",
            pc.surface,
            pc.words,
            pc.conn_pos(),
            pc.total_pos(),
            pc.total
        );
    }

    println!();
    println!("== decoder 實際輸出（JaDecoder::open_with_context） ==");
    let decoder = JaDecoder::open_with_context(&pack).map_err(|e| e.to_string())?;
    let results = decoder.results(&units, &JaContext::default(), 8);
    println!("{:<5} {:<12} {:>9}  clauses", "rank", "surface", "cost");
    for (index, result) in results.iter().enumerate() {
        let clauses: Vec<String> = result
            .clauses
            .iter()
            .map(|clause| format!("{}={}", clause.surface, clause.cost))
            .collect();
        println!(
            "{:<5} {:<12} {:>9}  {}",
            index + 1,
            result.surface(),
            result.conversion().cost,
            clauses.join(" + ")
        );
    }

    Ok(())
}

/// 印出 cost 明細用的結構。
struct PathCost {
    surface: String,
    words: i32,
    conn: i32,
    total: i32,
    nodes: usize,
    /// 每個詞的 `(說明, 詞成本)`。
    word_terms: Vec<(String, i32)>,
    /// 每段連接的 `(說明, 連接成本)`，含 BOS 與 EOS。
    conn_terms: Vec<(String, i32)>,
}

impl PathCost {
    /// 沒有 context 矩陣時，每段連接都是固定詞性成本。
    fn conn_pos(&self) -> i32 {
        POS_FALLBACK_CONNECTION * (self.nodes as i32 + 1)
    }

    /// 只用詞成本加固定詞性連接的總成本。
    fn total_pos(&self) -> i32 {
        self.words + self.conn_pos()
    }
}

/// 依位置展開所有完整路徑。
fn enumerate(lat: &jc_core::ja::decode::Lattice, len: usize) -> Vec<Vec<&JaNode>> {
    fn walk<'a>(
        lat: &'a jc_core::ja::decode::Lattice,
        len: usize,
        pos: usize,
        cur: &mut Vec<&'a JaNode>,
        out: &mut Vec<Vec<&'a JaNode>>,
    ) {
        if pos == len {
            out.push(cur.clone());
            return;
        }
        for node in lat.nodes(pos) {
            cur.push(node);
            walk(lat, len, node.end, cur, out);
            cur.pop();
        }
    }
    let mut out = Vec::new();
    let mut cur = Vec::new();
    walk(lat, len, 0, &mut cur, &mut out);
    out
}

/// 計算一條路徑的詞成本、連接成本與總成本。
///
/// 連接成本的判定與 [`JaDecoder`] 相同：左詞有 right ID 且右詞有 left ID 時
/// 查 context 矩陣，否則退回固定詞性成本。
fn cost_path(path: &[&JaNode], matrix: &JaConnectionCost, has_matrix: bool) -> PathCost {
    let mut words = 0i32;
    let mut conn = 0i32;
    let mut word_terms: Vec<(String, i32)> = Vec::new();
    let mut conn_terms: Vec<(String, i32)> = Vec::new();
    let mut surface = String::new();
    let mut left_rid: Option<u16> = has_matrix.then_some(0);

    for node in path {
        words = words.saturating_add(node.entry.cost);
        surface.push_str(&node.entry.surface);
        word_terms.push((
            format!(
                "{}[{},{:?},lid={},rid={}]",
                node.entry.surface,
                node.entry.pos,
                node.entry.pos_class(),
                opt_id(node.left_id),
                opt_id(node.right_id)
            ),
            node.entry.cost,
        ));
        let right_lid = node.left_id;
        let label = match left_rid {
            Some(rid) => format!("{rid} -> {}", opt_id(right_lid)),
            None => format!("? -> {}", opt_id(right_lid)),
        };
        let (value, how) = match (left_rid, right_lid) {
            (Some(rid), Some(lid)) => match matrix.cost(rid, lid) {
                Some(cost) => (cost, format!("matrix {label}")),
                None => (POS_FALLBACK_CONNECTION, format!("fallback {label}")),
            },
            _ => (
                POS_FALLBACK_CONNECTION,
                format!("fallback {label}（缺 context ID）"),
            ),
        };
        conn = conn.saturating_add(value);
        conn_terms.push((how, value));
        left_rid = node.right_id;
    }

    let (final_value, final_how) = match left_rid {
        Some(rid) if has_matrix => match matrix.cost(rid, 0) {
            Some(cost) => (cost, format!("matrix {rid} -> 0 (EOS)")),
            None => (
                POS_FALLBACK_CONNECTION,
                format!("fallback {rid} -> 0 (EOS)"),
            ),
        },
        _ => (POS_FALLBACK_CONNECTION, "fallback ? -> 0 (EOS)".to_string()),
    };
    conn = conn.saturating_add(final_value);
    conn_terms.push((final_how, final_value));

    PathCost {
        surface,
        words,
        conn,
        total: words.saturating_add(conn),
        nodes: path.len(),
        word_terms,
        conn_terms,
    }
}

fn opt_id(id: Option<u16>) -> String {
    match id {
        Some(value) => value.to_string(),
        None => "-".to_string(),
    }
}

/// 解析命令列參數。
fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut reading = None;
    let mut pack = None;
    let mut testdata = None;
    let mut limit = 40usize;
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--pack" => {
                index += 1;
                pack = Some(PathBuf::from(argv.get(index).ok_or("--pack 需要路徑")?));
            }
            "--testdata" => {
                index += 1;
                testdata = Some(PathBuf::from(argv.get(index).ok_or("--testdata 需要路徑")?));
            }
            "--limit" => {
                index += 1;
                limit = argv
                    .get(index)
                    .ok_or("--limit 需要數字")?
                    .parse()
                    .map_err(|_| "--limit 需要正整數".to_string())?;
            }
            other if other.starts_with("--") => {
                return Err(format!("不認得的參數 {other}"));
            }
            other => {
                if reading.is_some() {
                    return Err("只接受一個讀音".to_string());
                }
                reading = Some(other.to_string());
            }
        }
        index += 1;
    }
    Ok(Args {
        reading: reading.ok_or("缺少讀音；用法：ja-cost <reading>")?,
        pack,
        testdata,
        limit,
    })
}
