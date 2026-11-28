//! 注音／日文候選在每一次變換後不得退化成只剩自己。
//!
//! 用真實資料（`JC_TESTDATA`）把選字、原始輸入、Tab 補全、游標移動、換音節、
//! 刪除、片段切換、日文轉換與 Escape 後重開都跑一遍；每次變換後關窗再按
//! `↓`，斷言候選仍有目前值、原始輸入與推薦。資料不存在時跳過。

use jc_core::key::Key;
use jc_core::session::Session;
use jc_core::KeyCode;
use jc_core::Mode;
use jc_core::Snapshot;

fn key(code: KeyCode) -> Key {
    Key::new(code, None, false)
}

fn down() -> Key {
    key(KeyCode::Down)
}

fn escape() -> Key {
    key(KeyCode::Escape)
}

fn chars(text: &str) -> Vec<Key> {
    text.chars().map(|ch| Key::char(ch, false)).collect()
}

fn base(data: &jc_engine::loader::EngineData) -> Session {
    let mut config = data.session_config(Mode::Mixed);
    config.max_candidates = 200;
    config.english_autocomplete = true;
    Session::new(config)
}

fn press_all(session: &mut Session, keys: &[Key]) {
    for &key in keys {
        session.press(key);
    }
}

/// 選一個字面；找不到就 panic 並印出目前清單。
fn choose(session: &mut Session, surface: &str) -> Snapshot {
    let list: Vec<String> = session
        .candidates()
        .iter()
        .map(|candidate| candidate.surface.clone())
        .collect();
    let index = list
        .iter()
        .position(|item| item == surface)
        .unwrap_or_else(|| panic!("找不到候選 {surface:?}：{list:?}"));
    session.choose_candidate(index).expect("套用候選")
}

/// 關掉還開著的候選窗（如果有），再按一次 `↓` 重新開啟。
fn reopen(session: &mut Session) -> Snapshot {
    if session.snapshot().navigating {
        session.press(escape());
    }
    session.press(down())
}

type Build = Box<dyn Fn(&jc_engine::loader::EngineData) -> Session + 'static>;
type Case = (
    &'static str,
    Build,
    &'static str,
    Option<&'static str>,
    usize,
);

fn surfaces(snap: &Snapshot) -> Vec<&str> {
    snap.candidates
        .iter()
        .map(|candidate| candidate.surface.as_str())
        .collect()
}

/// 變換後重開：長度與前兩項都要符合。
fn assert_reopened(
    session: &mut Session,
    label: &str,
    first: &str,
    second: Option<&str>,
    min: usize,
) {
    let display = session.snapshot();
    // 窗還開著時，顯示中的清單本身就要完整；關窗後的顯示清單可以只有
    // 目前鎖定的那一個字（例如剛選完字），真正的檢查是重開後。
    if display.navigating {
        assert!(
            display.candidates.len() >= min,
            "{label}：開窗中的清單只剩 {} 項 {:?}",
            display.candidates.len(),
            surfaces(&display)
        );
    }
    let snap = reopen(session);
    let list = surfaces(&snap);
    assert!(
        list.len() >= min,
        "{label}：重開後只剩 {list:?}（顯示清單 {} 項，preedit {:?}）",
        display.candidates.len(),
        display.preedit
    );
    assert_eq!(
        snap.candidates[0].surface, first,
        "{label}：第一項：{list:?}（preedit {:?}）",
        snap.preedit
    );
    if let Some(second) = second {
        assert_eq!(
            snap.candidates[1].surface, second,
            "{label}：第二項：{list:?}（preedit {:?}）",
            snap.preedit
        );
    }
}

#[test]
fn candidate_lists_survive_every_transform() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let data = jc_engine::loader::load_engine_data(&root).unwrap();

    // (名稱, 建構到「變換完成、還沒重新開窗」的按鍵／選字, [0], [1], 最少項數)
    let mut cases: Vec<Case> = Vec::new();

    let mut add = |name: &'static str,
                   action: Build,
                   first: &'static str,
                   second: Option<&'static str>,
                   min: usize| {
        cases.push((name, action, first, second, min));
    };

    add(
        "zh 選目前值",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "我");
            s
        }),
        "我",
        Some("ji3"),
        5,
    );
    add(
        "zh 選推薦",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "婐");
            s
        }),
        "婐",
        Some("ji3"),
        5,
    );
    add(
        "zh 選原始輸入後只對聲調鍵",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "ji3");
            s
        }),
        "3",
        Some("３"),
        2,
    );
    add(
        "zh 選注音字面後只對聲調符號",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "ㄨㄛˇ");
            s
        }),
        "ˇ",
        Some("3"),
        2,
    );
    add(
        "zh 把聲調拆成按鍵",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "ㄨㄛˇ");
            press_all(&mut s, &[down()]);
            choose(&mut s, "3");
            s
        }),
        "3",
        Some("３"),
        2,
    );
    add(
        "zh 選無聲調按鍵後只對聲調鍵",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "ji");
            s
        }),
        "3",
        Some("３"),
        2,
    );
    add(
        "zh 選かな拆解後只對聲調鍵",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "じ3");
            s
        }),
        "3",
        Some("３"),
        2,
    );
    add(
        "zh 數字鍵選推薦",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down(), Key::char('3', false)]);
            s
        }),
        "婐",
        Some("ji3"),
        5,
    );
    add(
        "zh 字面刪聲調後只對韻母",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "ㄨㄛˇ");
            press_all(&mut s, &[key(KeyCode::Backspace)]);
            s
        }),
        "ㄛ",
        Some("i"),
        2,
    );
    add(
        "zh Enter 套用高亮",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down(), down(), down(), key(KeyCode::Enter)]);
            s
        }),
        "婐",
        Some("ji3"),
        5,
    );
    add(
        "zh Tab 在候選窗內",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down(), key(KeyCode::Tab)]);
            s
        }),
        "我",
        Some("ji3"),
        5,
    );
    add(
        "zh Tab 補全",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[key(KeyCode::Tab)]);
            s
        }),
        "我",
        Some("ji3"),
        5,
    );
    add(
        "zh 游標進字面中間只對右側符號",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "ㄨㄛˇ");
            press_all(&mut s, &[key(KeyCode::ArrowLeft), key(KeyCode::ArrowLeft)]);
            s
        }),
        "ㄛˇ",
        Some("i3"),
        2,
    );
    add(
        "zh 下一個音節",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3su3"));
            s
        }),
        "你",
        Some("su3"),
        5,
    );
    add(
        "zh Backspace",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3su3"));
            press_all(&mut s, &[key(KeyCode::Backspace)]);
            s
        }),
        "我",
        Some("ji3"),
        5,
    );
    add(
        "zh Delete",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji394su3"));
            press_all(
                &mut s,
                &[
                    key(KeyCode::ArrowLeft),
                    key(KeyCode::ArrowLeft),
                    key(KeyCode::ArrowLeft),
                    key(KeyCode::Delete),
                ],
            );
            s
        }),
        "愛",
        Some("94"),
        5,
    );
    add(
        "zh 片段切換",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji394su3"));
            press_all(&mut s, &[key(KeyCode::ArrowLeft), key(KeyCode::ArrowLeft)]);
            s
        }),
        "愛",
        Some("94"),
        2,
    );
    add(
        "zh Escape 後重開",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down(), escape()]);
            s
        }),
        "我",
        Some("ji3"),
        5,
    );
    add(
        "zh 學習後重開",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("ji3"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "婐");
            // 第一次重開（學習生效），再關窗重開確認排序固定。
            press_all(&mut s, &[down(), escape()]);
            s
        }),
        "婐",
        Some("ji3"),
        5,
    );
    add(
        "en Tab 開候選窗",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("hellp"));
            press_all(&mut s, &[key(KeyCode::Tab)]);
            s
        }),
        "hellp",
        Some("hello"),
        2,
    );
    add(
        "ja 轉換",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("watashi"));
            s
        }),
        "私",
        Some("watashi"),
        5,
    );
    add(
        "ja 選かな後只對下一個字元",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("watashi"));
            press_all(&mut s, &[down()]);
            choose(&mut s, "わたし");
            s
        }),
        "た",
        Some("ta"),
        5,
    );
    add(
        "ja Tab 換候選",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("watashi"));
            press_all(&mut s, &[down(), key(KeyCode::Tab), escape()]);
            s
        }),
        "私",
        Some("watashi"),
        5,
    );
    add(
        "ja Backspace 刪可見字元",
        Box::new(|data| {
            let mut s = base(data);
            press_all(&mut s, &chars("watashi"));
            press_all(&mut s, &[key(KeyCode::F(6))]);
            press_all(&mut s, &[key(KeyCode::Backspace)]);
            s
        }),
        "た",
        Some("ta"),
        5,
    );

    for (name, build, first, second, min) in cases {
        let mut session = build(&data);
        assert_reopened(&mut session, name, first, second, min);
    }

    // 候選窗開著時走到尾端再回頭，清單不能被換掉。
    let mut s = base(&data);
    press_all(&mut s, &chars("ji3"));
    let opened = s.press(down());
    let first = surfaces(&opened);
    for _ in 0..20 {
        s.press(down());
    }
    let last_snap = s.snapshot();
    let last = surfaces(&last_snap);
    assert_eq!(first, last, "走到尾端不能改清單");
    for _ in 0..20 {
        s.press(key(KeyCode::Up));
    }
    assert_eq!(s.snapshot().highlighted, 0, "回頭回到第一項");
    assert_eq!(surfaces(&s.snapshot()), first, "回頭清單不變");
}

/// zh Tab 補全：接受整詞補全後，開著的候選清單仍要有目前值、原始輸入與推薦。
#[test]
fn zh_tab_completion_keeps_full_candidate_list() {
    let Some(root) = jc_testdata::root() else {
        eprintln!("缺少測試資料：JC_TESTDATA");
        return;
    };
    let data = jc_engine::loader::load_engine_data(&root).unwrap();
    let mut s = base(&data);
    press_all(&mut s, &chars("ji3"));
    let snap = s.press(key(KeyCode::Tab));
    assert_eq!(snap.preedit, "我們", "Tab 接受整詞補全");
    assert!(
        snap.candidates.len() >= 5,
        "補全後清單不能只剩一項：{:?}",
        surfaces(&snap)
    );
    assert_eq!(
        snap.candidates[0].surface,
        "我",
        "第一項是目前值：{:?}",
        surfaces(&snap)
    );
    assert!(
        surfaces(&snap).contains(&"ㄨㄛˇ"),
        "補全後仍要有注音字面：{:?}",
        surfaces(&snap)
    );
    // 新 target 模型：完成後的顯示字元各自是 unit；游標在結尾時 ↓
    // 只對最後一個字元 們，不再回頭列整個補全詞。
    let down = s.press(down());
    assert_eq!(
        down.candidates[0].surface,
        "們",
        "↓ 只對最後一個顯示字元：{:?}",
        surfaces(&down)
    );
    let target = s.candidate_target().expect("有 target");
    assert_eq!((target.0, target.1), (1, 1), "target 是最後一個字元");
}
