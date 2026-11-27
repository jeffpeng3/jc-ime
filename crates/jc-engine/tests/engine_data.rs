//! 引擎行程的端到端資料測試：真的起 `jc-engine`、送按鍵、收轉換結果。
//!
//! 需要 `JC_TESTDATA`（預設 `/mnt/work/jc/jc-ime-data`）；資料不存在時跳過。

use std::io::{BufReader, Write};
use std::process::{Child, Command, Stdio};

use jc_core::{Key, KeyCode};
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

fn send_key(
    child: &mut Child,
    output: &mut BufReader<std::process::ChildStdout>,
    key: Key,
) -> Response {
    let input = child.stdin.as_mut().expect("stdin");
    write_message(input, &Request::Key { key }, DEFAULT_MAX_FRAME).unwrap();
    input.flush().unwrap();
    read_message::<_, Response>(output, DEFAULT_MAX_FRAME)
        .unwrap()
        .expect("要有按鍵回應")
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

/// 實機疊字回歸：`watashi` → F6 → `a` → Tab → Enter 不得把「わたし」接兩次。
#[test]
fn japanese_tab_enter_does_not_duplicate_prefix() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let settings_dir =
        std::env::temp_dir().join(format!("jc-ja-dup-prefix-{}", std::process::id()));
    std::fs::create_dir_all(&settings_dir).expect("建立暫存設定目錄");
    let settings_path = settings_dir.join("settings.json");
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
    let mut preedits = Vec::new();
    let mut send = |key: Key, output: &mut BufReader<std::process::ChildStdout>| {
        let input = child.stdin.as_mut().expect("stdin");
        write_message(input, &Request::Key { key }, DEFAULT_MAX_FRAME).unwrap();
        input.flush().unwrap();
        read_message::<_, Response>(output, DEFAULT_MAX_FRAME)
            .unwrap()
            .expect("要有按鍵回應")
    };
    for ch in "watashi".chars() {
        let reply = send(Key::char(ch, false), &mut output);
        let Response::State { preedit, .. } = reply else {
            panic!("按鍵 {ch:?} 應回 State，得到 {reply:?}");
        };
        preedits.push(preedit);
    }
    for (label, key) in [
        ("F6", Key::new(KeyCode::F(6), None, false)),
        ("a", Key::char('a', false)),
        ("Tab", Key::new(KeyCode::Tab, None, false)),
    ] {
        let reply = send(key, &mut output);
        let Response::State { preedit, .. } = reply else {
            panic!("{label} 應回 State，得到 {reply:?}");
        };
        preedits.push(preedit);
    }
    assert_eq!(
        preedits,
        [
            "w",
            "わ",
            "わt",
            "わた",
            "わたs",
            "わたsh",
            "私",
            "わたし",
            "わたしあ",
            "わたしあ"
        ],
        "每個中間 preedit 都要逐鍵前進，Tab 不能疊字"
    );
    for preedit in &preedits {
        assert!(
            !preedit.contains("わたしわたし"),
            "preedit 疊字：{preedit:?}；序列={preedits:?}"
        );
    }
    let reply = send(Key::new(KeyCode::Enter, None, false), &mut output);
    assert_eq!(
        reply,
        Response::Committed {
            text: "わたしあ".into()
        },
        "Enter 應送出一次わたしあ；preedits={preedits:?}"
    );
    shutdown(child);
    std::fs::remove_dir_all(&settings_dir).ok();
}

/// 真實資料的游標回歸：`ji3`、`ji394su3`、日文 `watashi` 的左移、右移與
/// Backspace 都要落在 rune 邊界，而且不能把游標推到看不見的位置。
#[test]
fn cursor_arrows_and_backspace_on_real_data() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let root = root.into_os_string();
    let mut child = spawn(&[
        std::ffi::OsStr::new("--data"),
        &root,
        std::ffi::OsStr::new("--mode"),
        std::ffi::OsStr::new("mixed"),
    ]);
    let mut output = hello(&mut child);
    for phrase in ["ji3", "ji394su3", "watashi"] {
        for ch in phrase.chars() {
            let reply = send_key(&mut child, &mut output, Key::char(ch, false));
            assert!(
                matches!(reply, Response::State { .. }),
                "{phrase:?} 的按鍵 {ch:?} 應回 State，得到 {reply:?}"
            );
        }
        let snapshot = state(&mut child, &mut output);
        let Response::State {
            preedit: typed,
            cursor_rune: typed_cursor,
            ..
        } = snapshot
        else {
            panic!("{phrase:?} 應收到 State，得到 {snapshot:?}");
        };
        let typed_runes = typed.chars().count();
        assert!(!typed.is_empty(), "{phrase:?} 組字不該是空的");
        assert!(
            typed_cursor <= typed_runes,
            "{phrase:?} 游標 {typed_cursor} 超出 rune 數 {typed_runes}：{typed:?}"
        );

        let left = send_key(
            &mut child,
            &mut output,
            Key::new(KeyCode::ArrowLeft, None, false),
        );
        let Response::State {
            preedit: left_text,
            cursor_rune: left_cursor,
            ..
        } = left
        else {
            panic!("{phrase:?} 左移應回 State");
        };
        assert_eq!(left_text, typed, "{phrase:?} 左移不改組字");
        assert_eq!(
            left_cursor,
            typed_cursor.saturating_sub(1),
            "{phrase:?} 左移一格"
        );

        let right = send_key(
            &mut child,
            &mut output,
            Key::new(KeyCode::ArrowRight, None, false),
        );
        let Response::State {
            preedit: right_text,
            cursor_rune: right_cursor,
            ..
        } = right
        else {
            panic!("{phrase:?} 右移應回 State");
        };
        assert_eq!(right_text, typed, "{phrase:?} 右移不改組字");
        assert_eq!(
            right_cursor,
            (left_cursor + 1).min(typed_runes),
            "{phrase:?} 右移回到原位移"
        );

        let back = send_key(
            &mut child,
            &mut output,
            Key::new(KeyCode::Backspace, None, false),
        );
        let Response::State {
            preedit: back_text,
            cursor_rune: back_cursor,
            ..
        } = back
        else {
            panic!("{phrase:?} Backspace 應回 State");
        };
        // 結尾 Backspace 要刪掉一個音節並停在新的結尾；日文 lane 目前是
        // no-op，維持原字串與游標。這裡斷言精確位置，不用「不超過長度」。
        let (want_text, want_cursor) = match phrase {
            "ji3" => (String::new(), 0),
            "ji394su3" => ("我愛".to_string(), 2),
            "watashi" => (right_text.clone(), right_cursor),
            _ => unreachable!("未定義的測試字串"),
        };
        assert_eq!(back_text, want_text, "{phrase:?} Backspace 後組字");
        assert_eq!(back_cursor, want_cursor, "{phrase:?} Backspace 後游標");
        let _ = send_key(
            &mut child,
            &mut output,
            Key::new(KeyCode::Escape, None, false),
        );
    }
    shutdown(child);
}

/// 多音節結尾 Backspace 後游標要停在新的結尾，而不是退回舊的音節格。
///
/// 這是本次修正的行為：base 在 `ji394su3` 按 Backspace 後游標是 1，
/// 新實作是 2；接著按 Delete 應該是 no-op，不會再刪掉「愛」。
#[test]
fn backspace_at_end_then_delete_stays() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let root = root.into_os_string();
    let mut child = spawn(&[
        std::ffi::OsStr::new("--data"),
        &root,
        std::ffi::OsStr::new("--mode"),
        std::ffi::OsStr::new("mixed"),
    ]);
    let mut output = hello(&mut child);
    for (phrase, want_text, want_cursor) in [("ji394su3", "我愛", 2), ("ji3su3", "我", 1)] {
        for ch in phrase.chars() {
            let _ = send_key(&mut child, &mut output, Key::char(ch, false));
        }
        let back = send_key(
            &mut child,
            &mut output,
            Key::new(KeyCode::Backspace, None, false),
        );
        let Response::State {
            preedit,
            cursor_rune,
            ..
        } = back
        else {
            panic!("{phrase:?} Backspace 應回 State");
        };
        assert_eq!(preedit, want_text, "{phrase:?} Backspace 後組字");
        assert_eq!(cursor_rune, want_cursor, "{phrase:?} Backspace 後游標");

        let delete = send_key(
            &mut child,
            &mut output,
            Key::new(KeyCode::Delete, None, false),
        );
        let Response::State {
            preedit: after_delete,
            cursor_rune: after_cursor,
            ..
        } = delete
        else {
            panic!("{phrase:?} Delete 應回 State");
        };
        assert_eq!(after_delete, want_text, "{phrase:?} 結尾 Delete 應 no-op");
        assert_eq!(after_cursor, want_cursor, "{phrase:?} 結尾 Delete 不改游標");
        let _ = send_key(
            &mut child,
            &mut output,
            Key::new(KeyCode::Escape, None, false),
        );
    }
    shutdown(child);
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
