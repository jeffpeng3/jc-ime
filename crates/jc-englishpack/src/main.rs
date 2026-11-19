//! `jc-englishpack` 命令列：建置、匯入上游匯出、驗證與查詢英文資料包。

use std::path::PathBuf;
use std::process::ExitCode;

use jc_core::mixed::{default_english, english_pack_dir, Config, English, Router};
use jc_englishpack::{compile, import_lists, load};

const USAGE: &str = "jc-englishpack（英文詞表工具）\n\n\
用法：\n  \
jc-englishpack build --source <dir> --out <dir>\n  \
jc-englishpack import-lists --source <dir> --exports <dir>\n  \
jc-englishpack verify --dir <dir>\n  \
jc-englishpack query <prefix> [--dir <dir>] [--limit 8]\n\n\
--dir 省略時用 JC_ENGLISH_PACK，再退回資料根目錄下的 english/。\n";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("jc-englishpack: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        print!("{USAGE}");
        return Ok(());
    };
    match command {
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        "build" => build(&args[1..]),
        "import-lists" => import(&args[1..]),
        "verify" => verify(&args[1..]),
        "query" => query(&args[1..]),
        other => Err(format!("不認得的命令 {other}\n\n{USAGE}")),
    }
}

fn build(args: &[String]) -> Result<(), String> {
    let (source, out) = paths(args, "source", "out")?;
    let manifest = compile::compile(&source, &out).map_err(|e| e.to_string())?;
    println!("輸出目錄：{}", out.display());
    println!("詞數：{}", manifest.words);
    println!("排序：{}", manifest.ranking);
    for source in &manifest.sources {
        println!("來源：{} {}", source.name, source.version);
        if let Some(commit) = &source.commit {
            println!("commit：{commit}");
        }
    }
    for (name, file) in &manifest.files {
        println!("{name}：{} 位元組  SHA-256 {}", file.size, file.sha256);
    }
    Ok(())
}

fn import(args: &[String]) -> Result<(), String> {
    let (source, exports) = paths(args, "source", "exports")?;
    import_lists(&source, &exports).map_err(|e| e.to_string())?;
    println!("已寫回 {}", source.join("esdb.tsv").display());
    Ok(())
}

fn verify(args: &[String]) -> Result<(), String> {
    let dir = pack_dir(args)?;
    let (dict, manifest) = load(&dir).map_err(|e| e.to_string())?;
    println!("資料包：{}", dir.display());
    println!("格式版本：{}", manifest.format);
    println!("詞數：{}", dict.len());
    println!("排序：{}", manifest.ranking);
    for (name, file) in &manifest.files {
        println!("{name}：{} 位元組  SHA-256 {}", file.size, file.sha256);
    }
    Ok(())
}

fn query(args: &[String]) -> Result<(), String> {
    let mut input = None;
    let mut dir = None;
    let mut limit = 8usize;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dir" => {
                index += 1;
                dir = Some(PathBuf::from(value(args, index, "--dir")?));
            }
            "--limit" => {
                index += 1;
                limit = value(args, index, "--limit")?
                    .parse()
                    .map_err(|_| "--limit 需要正整數".to_string())?;
            }
            other if other.starts_with("--") => {
                return Err(format!("query 不認得的參數 {other}"));
            }
            other => {
                if input.is_some() {
                    return Err("query 只接受一個輸入".to_string());
                }
                input = Some(other.to_string());
            }
        }
        index += 1;
    }
    let input = input.ok_or_else(|| "query 需要一個輸入".to_string())?;
    if limit == 0 {
        return Err("--limit 必須大於 0".to_string());
    }
    let owned = dir
        .map(English::from_pack_dir)
        .transpose()
        .map_err(|e| e.to_string())?;
    let english: &English = match &owned {
        Some(english) => english,
        None => default_english(),
    };
    match english.origin() {
        Some(path) => println!(
            "詞表：{}（{} 詞）",
            path.display(),
            english.dictionary().len()
        ),
        None => println!("詞表：內建原型（{} 詞）", english.dictionary().len()),
    }
    println!("輸入：{input}");
    let router = Router::new().with_english(english).with_config(Config {
        max_candidates: limit,
        ..Config::default()
    });
    let candidates = router.prefixes(&input);
    println!("rank  surface  cost");
    for (index, candidate) in candidates.iter().enumerate() {
        println!(
            "{:>4}  {:<8} {:>5}",
            index + 1,
            candidate.surface,
            candidate.cost
        );
    }
    if candidates.is_empty() {
        println!("（沒有候選：輸入 {input:?}）");
    }
    Ok(())
}

fn pack_dir(args: &[String]) -> Result<PathBuf, String> {
    let mut dir = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dir" => {
                index += 1;
                dir = Some(PathBuf::from(value(args, index, "--dir")?));
            }
            other => return Err(format!("不認得的參數 {other}")),
        }
        index += 1;
    }
    match dir.or_else(english_pack_dir) {
        Some(dir) => Ok(dir),
        None => Err("找不到英文資料包；請用 --dir 指定，或設 JC_ENGLISH_PACK".to_string()),
    }
}

fn paths(args: &[String], first: &str, second: &str) -> Result<(PathBuf, PathBuf), String> {
    let first_flag = format!("--{first}");
    let second_flag = format!("--{second}");
    let mut a = None;
    let mut b = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        index += 1;
        if flag == first_flag {
            a = Some(PathBuf::from(value(args, index, &first_flag)?));
        } else if flag == second_flag {
            b = Some(PathBuf::from(value(args, index, &second_flag)?));
        } else {
            return Err(format!("不認得的參數 {flag}"));
        }
        index += 1;
    }
    let a = a.ok_or_else(|| format!("需要 {first_flag}"))?;
    let b = b.ok_or_else(|| format!("需要 {second_flag}"))?;
    Ok((a, b))
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} 需要一個值"))
}
