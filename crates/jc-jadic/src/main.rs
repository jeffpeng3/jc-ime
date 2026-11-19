//! `jc-jadic` 命令列入口。

use std::process::ExitCode;

use jc_jadic::{import, ImportOptions};

const USAGE: &str = "\
用法：
  jc-jadic import --mozc <dictionary_oss 目錄> --out <輸出目錄>

選項：
  --mozc <dir>  Mozc dictionary_oss 目錄，需含 dictionary00.txt..dictionary09.txt、
                id.def 與 connection_single_column.txt
  --out <dir>   輸出目錄，寫入 ja-lexicon-mozc-v2.pack 與 .manifest.json
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("jc-jadic: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        return Err(format!("缺少子命令\n\n{USAGE}"));
    };
    match command.as_str() {
        "import" => {
            let mut mozc = None;
            let mut out = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--mozc" => {
                        mozc = Some(args.next().ok_or("--mozc 缺少路徑")?);
                    }
                    "--out" => {
                        out = Some(args.next().ok_or("--out 缺少路徑")?);
                    }
                    "-h" | "--help" => {
                        print!("{USAGE}");
                        return Ok(());
                    }
                    other => return Err(format!("未知參數 {other:?}\n\n{USAGE}")),
                }
            }
            let options = ImportOptions {
                mozc_dir: mozc.ok_or("缺少 --mozc")?.into(),
                out_dir: out.ok_or("缺少 --out")?.into(),
            };
            let report = import(&options).map_err(|error| error.to_string())?;
            println!("pack: {}", report.pack_path.display());
            println!("manifest: {}", report.manifest_path.display());
            println!("entries: {}", report.stats.entries);
            println!("surfaces: {}", report.stats.surfaces);
            println!("rows: {}", report.stats.rows);
            println!("duplicates_dropped: {}", report.stats.duplicates_dropped);
            println!("filtered_reading: {}", report.stats.filtered_reading);
            println!("pos: {}", report.pos_count);
            println!("conj_types: {}", report.conj_types);
            println!("conj_forms: {}", report.conj_forms);
            println!("context_count: {}", report.context_count);
            println!("pack_bytes: {}", report.pack_bytes);
            println!("pack_sha256: {}", report.pack_sha256);
            println!("payload_sha256: {}", report.payload_sha256);
            println!("manifest_sha256: {}", report.manifest_sha256);
            Ok(())
        }
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("未知子命令 {other:?}\n\n{USAGE}")),
    }
}
