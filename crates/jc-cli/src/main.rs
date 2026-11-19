//! `jc-cli` 的進入點；實際工作都在 [`jc_cli`] 的模組裡。

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = jc_cli::run(&args, &mut std::io::stdout()) {
        eprintln!("jc-cli: {err}");
        std::process::exit(1);
    }
}
