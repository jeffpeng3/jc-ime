//! 引擎行程的端到端資料測試：真的起 `jc-engine`、送按鍵、收轉換結果。
//!
//! 需要 `JC_TESTDATA`（預設 `/mnt/work/jc/jc-ime-data`）；資料不存在時跳過。

use std::io::{BufReader, Write};
use std::process::{Child, Command, Stdio};

use jc_core::Key;
use jc_proto::message::{read_message, write_message};
use jc_proto::{Request, Response, DEFAULT_MAX_FRAME, PROTOCOL_VERSION};

fn spawn(args: &[&std::ffi::OsStr]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_jc-engine"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("啟動 jc-engine")
}

fn hello(child: &mut Child) -> BufReader<std::process::ChildStdout> {
    let mut input = child.stdin.take().expect("stdin");
    let mut output = BufReader::new(child.stdout.take().expect("stdout"));
    write_message(
        &mut input,
        &Request::Hello {
            protocol: PROTOCOL_VERSION,
            client: "integration".into(),
        },
        DEFAULT_MAX_FRAME,
    )
    .unwrap();
    input.flush().unwrap();
    let reply: Response = read_message(&mut output, DEFAULT_MAX_FRAME)
        .unwrap()
        .expect("要有握手回應");
    assert!(
        matches!(reply, Response::Ready { .. }),
        "握手失敗：{reply:?}"
    );
    // 後續測試直接重用這個 stdin handle。
    child.stdin = Some(input);
    output
}

fn press(child: &mut Child, output: &mut BufReader<std::process::ChildStdout>, text: &str) {
    let input = child.stdin.as_mut().expect("stdin");
    for ch in text.chars() {
        write_message(
            input,
            &Request::Key {
                key: Key::char(ch, false),
            },
            DEFAULT_MAX_FRAME,
        )
        .unwrap();
    }
    input.flush().unwrap();
    // 每個按鍵都有一個回應；先讀掉，避免下一次讀到上一個按鍵的 State。
    for _ in text.chars() {
        read_message::<_, Response>(output, DEFAULT_MAX_FRAME)
            .unwrap()
            .expect("要有按鍵回應");
    }
}

fn state(child: &mut Child, output: &mut BufReader<std::process::ChildStdout>) -> Response {
    let input = child.stdin.as_mut().expect("stdin");
    write_message(input, &Request::Snapshot, DEFAULT_MAX_FRAME).unwrap();
    input.flush().unwrap();
    read_message(output, DEFAULT_MAX_FRAME)
        .unwrap()
        .expect("要有快照回應")
}

fn shutdown(mut child: Child) {
    if let Some(input) = child.stdin.as_mut() {
        write_message(input, &Request::Shutdown, DEFAULT_MAX_FRAME).unwrap();
        input.flush().unwrap();
    }
    drop(child.stdin.take());
    let status = child.wait().expect("等待引擎結束");
    assert!(status.success(), "引擎應正常結束：{status}");
}

#[test]
fn engine_converts_zhuyin_with_data() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let root = root.into_os_string();
    // 這筆驗證注音轉出的候選清單；Mixed 模式會直接轉出文字但不列候選，
    // 所以明確用 Zh 模式執行，不要靠已改成 mixed 的預設。
    let mut child = spawn(&[
        std::ffi::OsStr::new("--data"),
        &root,
        std::ffi::OsStr::new("--mode"),
        std::ffi::OsStr::new("zh"),
    ]);
    let mut output = hello(&mut child);
    press(&mut child, &mut output, "ji3");
    let snapshot = state(&mut child, &mut output);
    let Response::State {
        preedit,
        candidates,
        ..
    } = snapshot
    else {
        panic!("應收到 State，得到 {snapshot:?}");
    };
    assert!(!preedit.is_empty(), "組字字串不該是空的");
    assert!(
        candidates.first().is_some_and(|text| !text.is_empty()),
        "首選不該是空的：{candidates:?}"
    );
    assert_eq!(candidates.first().map(String::as_str), Some("我"));

    let input = child.stdin.as_mut().expect("stdin");
    write_message(input, &Request::Commit, DEFAULT_MAX_FRAME).unwrap();
    input.flush().unwrap();
    let reply: Response = read_message(&mut output, DEFAULT_MAX_FRAME)
        .unwrap()
        .expect("要有提交回應");
    assert_eq!(
        reply,
        Response::Committed { text: "我".into() },
        "注音應轉成實際漢字"
    );
    shutdown(child);
}

#[test]
fn engine_converts_japanese_with_data() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let root = root.into_os_string();
    let mut child = spawn(&[
        std::ffi::OsStr::new("--data"),
        &root,
        std::ffi::OsStr::new("--mode"),
        std::ffi::OsStr::new("ja"),
    ]);
    let mut output = hello(&mut child);
    press(&mut child, &mut output, "nihongo");
    let snapshot = state(&mut child, &mut output);
    let Response::State {
        preedit,
        candidates,
        ..
    } = snapshot
    else {
        panic!("應收到 State，得到 {snapshot:?}");
    };
    assert!(!preedit.is_empty(), "日文組字不該是空的");
    assert!(
        candidates.first().is_some_and(|text| !text.is_empty()),
        "日文首選不該是空的：{candidates:?}"
    );
    shutdown(child);
}

/// 日文逐鍵預覽的端到端驗證：用真實引擎與 ja pack 送 `watashi` 七個鍵，
/// 每個回應都要比上一鍵更進一步；未完成的羅馬字必須留在 preedit 尾巴。
#[test]
fn japanese_live_preview_advances_on_every_key() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let settings_dir =
        std::env::temp_dir().join(format!("jc-ja-live-preview-{}", std::process::id()));
    std::fs::create_dir_all(&settings_dir).expect("建立暫存設定目錄");
    let settings_path = settings_dir.join("settings.json");
    // 故意不寫 `ja_conversion`：舊設定檔也要預設逐字変換（live）。
    std::fs::write(
        &settings_path,
        r#"{"schema":2,"primary_language":"ja","ja_input_method":"romaji"}"#,
    )
    .expect("寫入暫存設定");

    let root = root.into_os_string();
    let settings = settings_path.clone().into_os_string();
    let mut child = spawn(&[
        std::ffi::OsStr::new("--data"),
        &root,
        std::ffi::OsStr::new("--settings"),
        &settings,
    ]);
    let mut output = hello(&mut child);
    let mut seen = Vec::new();
    for ch in "watashi".chars() {
        let input = child.stdin.as_mut().expect("stdin");
        write_message(
            input,
            &Request::Key {
                key: Key::char(ch, false),
            },
            DEFAULT_MAX_FRAME,
        )
        .unwrap();
        input.flush().unwrap();
        let reply: Response = read_message(&mut output, DEFAULT_MAX_FRAME)
            .unwrap()
            .expect("要有按鍵回應");
        let Response::State { preedit, .. } = reply else {
            panic!("按鍵 {ch:?} 應回 State，得到 {reply:?}");
        };
        assert!(!preedit.is_empty(), "按鍵 {ch:?} 的 preedit 不該是空的");
        seen.push((ch, preedit));
    }
    for pair in seen.windows(2) {
        assert_ne!(
            pair[0].1, pair[1].1,
            "按鍵 {:?} → {:?} 沒有進展：{:?}",
            pair[0].0, pair[1].0, seen
        );
    }
    assert!(
        seen.iter()
            .any(|(_, preedit)| preedit.chars().any(|ch| ch.is_ascii_lowercase())),
        "逐鍵預覽要看到尚未完成的羅馬字：{seen:?}"
    );
    assert_ne!(
        seen.last().map(|(_, preedit)| preedit.as_str()),
        Some("watashi"),
        "最後一鍵應已進入轉換，而不是原樣羅馬字：{seen:?}"
    );
    shutdown(child);
    std::fs::remove_dir_all(&settings_dir).ok();
}

#[test]
fn missing_data_directory_reports_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_jc-engine"))
        .args(["--data", "/no/such/jc-engine-data"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("啟動 jc-engine");
    assert!(!output.status.success(), "缺資料時應該啟動失敗");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("/no/such/jc-engine-data"),
        "錯誤訊息要指出路徑：{stderr}"
    );
}

#[test]
fn missing_data_flag_and_environment_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_jc-engine"))
        .env_remove("JC_DATA")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("啟動 jc-engine");
    assert!(!output.status.success(), "沒給資料來源應啟動失敗");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--data") && stderr.contains("JC_DATA"),
        "錯誤訊息要說明 --data 與 JC_DATA：{stderr}"
    );
}
