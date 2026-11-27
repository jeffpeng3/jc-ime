//! 真實資料的顯示與提交一致性：三條長串都要做到「畫面看到什麼就送出什麼」。
//!
//! 需要 `JC_TESTDATA`（預設 `/mnt/work/jc/jc-ime-data`）；資料不存在時跳過。

use std::io::{BufReader, Write};
use std::process::{Child, Command, Stdio};

use jc_core::Key;
use jc_proto::message::{read_message, write_message};
use jc_proto::{Request, Response, DEFAULT_MAX_FRAME, PROTOCOL_VERSION};

fn spawn(mode: &str, root: &std::ffi::OsStr) -> Child {
    Command::new(env!("CARGO_BIN_EXE_jc-engine"))
        .arg("--data")
        .arg(root)
        .arg("--mode")
        .arg(mode)
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
            client: "display-commit".into(),
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
    child.stdin = Some(input);
    output
}

fn type_keys(child: &mut Child, output: &mut BufReader<std::process::ChildStdout>, text: &str) {
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
    for _ in text.chars() {
        read_message::<_, Response>(output, DEFAULT_MAX_FRAME)
            .unwrap()
            .expect("要有按鍵回應");
    }
}

fn snapshot(child: &mut Child, output: &mut BufReader<std::process::ChildStdout>) -> String {
    let input = child.stdin.as_mut().expect("stdin");
    write_message(input, &Request::Snapshot, DEFAULT_MAX_FRAME).unwrap();
    input.flush().unwrap();
    let reply: Response = read_message(output, DEFAULT_MAX_FRAME)
        .unwrap()
        .expect("要有快照回應");
    let Response::State { preedit, .. } = reply else {
        panic!("應收到 State，得到 {reply:?}");
    };
    preedit
}

fn commit(child: &mut Child, output: &mut BufReader<std::process::ChildStdout>) -> String {
    let input = child.stdin.as_mut().expect("stdin");
    write_message(input, &Request::Commit, DEFAULT_MAX_FRAME).unwrap();
    input.flush().unwrap();
    let reply: Response = read_message(output, DEFAULT_MAX_FRAME)
        .unwrap()
        .expect("要有提交回應");
    let Response::Committed { text } = reply else {
        panic!("應收到 Committed，得到 {reply:?}");
    };
    text
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

/// 三條長串：注音、混打英數字面、日文羅馬字。畫面與送出必須逐字相同。
#[test]
fn long_real_input_display_matches_commit() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let root = root.into_os_string();
    for (mode, keys) in [
        ("zh", "ji394su3ji394su3su3cl3"),
        ("mixed", "helloworldthisislongliteral"),
        ("ja", "watashihanihongowotukatteimasu"),
    ] {
        let mut child = spawn(mode, &root);
        let mut output = hello(&mut child);
        type_keys(&mut child, &mut output, keys);
        let shown = snapshot(&mut child, &mut output);
        assert!(!shown.is_empty(), "{mode} {keys:?} 的畫面不該是空的");
        let committed = commit(&mut child, &mut output);
        assert_eq!(
            committed, shown,
            "{mode} {keys:?}：畫面與送出不一致；commit={committed:?}"
        );
        shutdown(child);
    }
}
