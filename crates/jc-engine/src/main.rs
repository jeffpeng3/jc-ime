//! 引擎行程：在 Linux 開發期用 stdio／unix socket，Windows 用 named pipe。
//! 這是唯一的長期執行核心，wrapper 只透過 `jc-proto` 與它溝通。

fn main() {
    println!(
        "jc-engine {} (protocol {})",
        env!("CARGO_PKG_VERSION"),
        jc_proto::PROTOCOL_VERSION
    );
}
