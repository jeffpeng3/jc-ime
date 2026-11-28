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

fn send_key(
    child: &mut Child,
    output: &mut BufReader<std::process::ChildStdout>,
    key: Key,
) -> Response {
    let input = child.stdin.as_mut().expect("stdin");
    write_message(input, &Request::Key { key }, DEFAULT_MAX_FRAME).unwrap();
    input.flush().unwrap();
    read_message(output, DEFAULT_MAX_FRAME)
        .unwrap()
        .expect("要有按鍵回應")
}

fn idle(child: &mut Child, output: &mut BufReader<std::process::ChildStdout>) -> Response {
    let input = child.stdin.as_mut().expect("stdin");
    write_message(input, &Request::Idle { session: 0 }, DEFAULT_MAX_FRAME).unwrap();
    input.flush().unwrap();
    read_message(output, DEFAULT_MAX_FRAME)
        .unwrap()
        .expect("要有停頓回應")
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

/// 真實資料的長串預覽：游標停在目標文節，移動候選時只有那一段變，
/// 前後文留在畫面上；Commit 送出的也是預覽看到的字。
#[test]
fn long_japanese_clause_preview_only_replaces_target() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let root = root.into_os_string();
    let mut child = spawn("ja", &root);
    let mut output = hello(&mut child);
    type_keys(&mut child, &mut output, "watashihanihongowotukatteimasu");
    let Response::State {
        preedit: before, ..
    } = idle(&mut child, &mut output)
    else {
        panic!("Idle 應回 State");
    };
    assert_eq!(before, "私は日本語を使っています", "真實資料的長串");

    let target = "使";
    let target_byte = before.find(target).expect("長串要有目標字元");
    let target_rune = before[..target_byte].chars().count();
    let tail_runes = before.chars().count() - target_rune;
    for _ in 0..tail_runes {
        send_key(
            &mut child,
            &mut output,
            Key::new(jc_core::KeyCode::ArrowLeft, None, false),
        );
    }

    let opened = send_key(
        &mut child,
        &mut output,
        Key::new(jc_core::KeyCode::Down, None, false),
    );
    let Response::State {
        preedit,
        navigating,
        ..
    } = opened
    else {
        panic!("第一次 ↓ 應回 State");
    };
    assert!(navigating, "目標文節的 ↓ 要進入選取模式");
    assert_eq!(preedit, before, "開窗時顯示目前的詞面");

    let stepped = send_key(
        &mut child,
        &mut output,
        Key::new(jc_core::KeyCode::Down, None, false),
    );
    let Response::State {
        preedit,
        candidates,
        highlighted,
        ..
    } = stepped
    else {
        panic!("第二次 ↓ 應回 State");
    };
    let surface = candidates.get(highlighted).expect("要有高亮候選").clone();
    assert_ne!(surface, target, "高亮要離開目前詞面");
    let want = format!(
        "{}{}{}",
        &before[..target_byte],
        surface,
        &before[target_byte + target.len()..]
    );
    assert_eq!(preedit, want, "預覽只換目標字元");
    assert!(preedit.starts_with("私は日本語を"), "前文不變");
    assert!(preedit.ends_with("っています"), "後文不變");

    let committed = commit(&mut child, &mut output);
    assert_eq!(committed, preedit, "送出的是畫面上的預覽");
    shutdown(child);
}
