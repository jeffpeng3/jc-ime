//! 顯示與提交單一來源的行為鎖。
//!
//! 每個候選套用路徑都在套用前後斷言 `display_text()` 與 `commit_text()`
//! 相同：畫面看到什麼，commit 就送什麼。選單不進組字區，確認送出時仍走
//! 同一份 `CandidateAction`。
//!
//! 選取模式中的預覽另成一組：日文文節只換目標片段；一般注音、英文補全、
//! 字面候選與選單在套用前都不預覽，等確認後才進組字區。兩邊都有測試鎖住。

use std::sync::Arc;

use jc_core::ja::decode::JaDecoder;
use jc_core::lattice::{InMemoryLexicon, LexEntry};
use jc_core::mixed::{Dictionary, English, Word};
use jc_core::session::{Session, SessionConfig};
use jc_core::{Key, KeyCode, Mode, Snapshot};

fn press(session: &mut Session, key: Key) -> Snapshot {
    session.press(key)
}

fn press_chars(session: &mut Session, text: &str) {
    for ch in text.chars() {
        press(session, Key::char(ch, false));
    }
}

fn enter(session: &mut Session) -> Snapshot {
    press(session, Key::new(KeyCode::Enter, None, false))
}

fn down(session: &mut Session) -> Snapshot {
    press(session, Key::new(KeyCode::Down, None, false))
}

/// 畫面與送出必須是同一個字串；每一條候選路徑都經過這裡。
fn assert_consistent(session: &Session, label: &str) {
    assert_eq!(
        session.display_text(),
        session.commit_text(),
        "{label}：畫面與送出不一致"
    );
}

/// 套用後仍然一致，而且真正 commit 出去的字就是剛才畫面上的字。
fn assert_commit_matches_display(session: &mut Session, label: &str) -> String {
    assert_consistent(session, label);
    let shown = session.display_text();
    let committed = session.commit();
    assert_eq!(committed, shown, "{label}：commit 送出的字和畫面不同");
    committed
}

fn select_surface(session: &mut Session, surface: &str) -> usize {
    let at = session
        .candidates()
        .iter()
        .position(|candidate| candidate.surface == surface)
        .unwrap_or_else(|| {
            let list: Vec<&str> = session
                .candidates()
                .iter()
                .map(|candidate| candidate.surface.as_str())
                .collect();
            panic!("候選 {surface} 找不到：{list:?}")
        });
    session.select(at);
    at
}

fn zh_session() -> Session {
    let mut lexicon = InMemoryLexicon::new();
    for (surface, reading, cost) in [
        ("我", "ㄨㄛˇ", 0),
        ("窩", "ㄨㄛˇ", 10),
        ("你", "ㄋㄧˇ", 0),
        ("好", "ㄏㄠˇ", 0),
    ] {
        lexicon.push(LexEntry::new(surface, reading, cost, 0));
    }
    let mut config = SessionConfig::new(Mode::Mixed);
    config.zh_lexicon = Some(Arc::new(lexicon));
    Session::new(config)
}

/// 日文文節候選；資料不存在時跳過。
fn ja_session() -> Option<Session> {
    let path = jc_testdata::path("packs/ja-lexicon-mozc-v2.pack")?;
    let decoder = JaDecoder::open_with_context(&path).expect("載入 ja pack");
    let mut config = SessionConfig::new(Mode::Mixed);
    config.ja = Some(Arc::new(decoder));
    config.ja_only = true;
    Some(Session::new(config))
}

fn english_session() -> Session {
    let words = ["hello", "help"]
        .iter()
        .map(|word| Word::new(*word, 1.0))
        .collect::<Vec<_>>();
    let mut config = SessionConfig::new(Mode::Mixed);
    config.english = Some(Arc::new(English::new(Some(Dictionary::new(words)))));
    config.english_autocomplete = true;
    Session::new(config)
}

/// 一般注音候選：套用前後，畫面與送出都是同一份組字。
#[test]
fn test_general_candidate_display_matches_commit() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    down(&mut s);
    assert_consistent(&s, "一般候選窗");
    assert!(s.snapshot().navigating, "候選窗要進入選取模式");
    let before = s.display_text();
    assert_eq!(before, "我", "開窗後先顯示目前的轉換結果");

    let at = select_surface(&mut s, "窩");
    assert_eq!(
        s.snapshot().preedit,
        before,
        "一般注音候選套用前不預覽：整段維持目前的轉換結果"
    );
    assert_consistent(&s, "一般候選窗高亮");
    let applied = s.choose_candidate(at).expect("套用候選");
    assert_eq!(applied.preedit, "窩", "套用後畫面就是候選字");
    assert_eq!(
        assert_commit_matches_display(&mut s, "一般候選套用後"),
        "窩"
    );
}

/// 選單：組字區維持原樣，確認送出時只插入高亮的一項。
#[test]
fn test_menu_display_matches_commit() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    let menu = s.open_symbol_menu();
    assert!(!menu.candidates.is_empty(), "選單要有候選");
    assert_consistent(&s, "選單開著");
    assert_eq!(s.display_text(), "我", "選單不蓋掉組字區");
    let before = s.display_text();

    // 選單也要走一次選取模式；移動高亮不會把項目塞進組字區。
    down(&mut s);
    assert!(s.snapshot().navigating, "選單 ↓ 要進入選取模式");
    assert_eq!(
        s.display_text(),
        before,
        "選單移動高亮不預覽：項目只在確認後插入"
    );
    let chosen = s.highlighted_text().expect("選單要有高亮項目").to_string();
    let expected = format!("我{chosen}");

    enter(&mut s);
    assert_consistent(&s, "選單選取後");
    assert_eq!(assert_commit_matches_display(&mut s, "選單送出"), expected);
}

/// 前導符號變體：高亮即目前字面時直接送出，畫面與送出仍一致。
#[test]
fn test_prefix_variant_display_matches_commit() {
    let mut s = Session::new(SessionConfig::new(Mode::Mixed));
    press_chars(&mut s, "`[");
    assert_consistent(&s, "前導變體開窗");
    let before = s.display_text();
    down(&mut s);
    assert!(s.snapshot().navigating, "前導變體 ↓ 要進入選取模式");
    assert_eq!(s.display_text(), before, "前導變體候選套用前不預覽");
    assert_consistent(&s, "前導變體高亮");

    let shown = enter(&mut s);
    assert_eq!(shown.preedit, "「", "第一次 Enter 只套用變體");
    assert_consistent(&s, "前導變體套用後");
    enter(&mut s);
    assert_eq!(assert_commit_matches_display(&mut s, "前導變體送出"), "「");
}

/// `zh_window_last`：只換最後一格，套用後畫面與送出一起更新。
#[test]
fn test_zh_window_last_display_matches_commit() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    down(&mut s);
    down(&mut s);
    assert_consistent(&s, "zh_window_last 開窗");
    assert!(s.snapshot().navigating, "zh_window_last 要進入選取模式");
    let before = s.display_text();
    assert_eq!(before, "我", "開窗後先顯示目前的轉換結果");

    let at = select_surface(&mut s, "窩");
    assert_eq!(
        s.snapshot().preedit,
        before,
        "zh_window_last 套用前不預覽：只換目標音節，畫面等套用後才動"
    );
    assert_consistent(&s, "zh_window_last 高亮");
    let applied = s.choose_candidate(at).expect("套用候選");
    assert_eq!(applied.preedit, "窩", "只換最後一格");
    assert_eq!(
        assert_commit_matches_display(&mut s, "zh_window_last 套用後"),
        "窩"
    );
}

/// 日文文節：候選只預覽/替換那一個文節，前後文留在畫面上。
#[test]
fn test_ja_clause_display_matches_commit() {
    let Some(mut s) = ja_session() else {
        eprintln!("缺少測試資料：packs/ja-lexicon-mozc-v2.pack");
        return;
    };
    press_chars(&mut s, "watashi");
    let before = s.display_text();
    down(&mut s);
    assert_consistent(&s, "日文文節開窗");
    assert!(s.snapshot().navigating, "日文 ↓ 要進入選取模式");

    // 選一個和目前詞面不同的假名候選，確認預覽只換這一段。
    let at = s
        .candidates()
        .iter()
        .position(|candidate| candidate.surface == "わたし")
        .or_else(|| {
            s.candidates()
                .iter()
                .position(|candidate| candidate.surface != before)
        })
        .expect("日文候選清單要有替代寫法");
    s.select(at);
    let preview = s.snapshot();
    assert_eq!(
        preview.preedit,
        s.candidates()[at].surface,
        "單一文節的預覽要和目前高亮候選一致"
    );
    assert_ne!(preview.preedit, before, "預覽要換掉目標文節");
    assert_consistent(&s, "日文文節高亮");
    let applied = s.choose_candidate(at).expect("套用日文候選");
    assert_consistent(&s, "日文文節套用後");
    assert_eq!(applied.preedit, preview.preedit, "套用後才真正取代");
    assert_commit_matches_display(&mut s, "日文文節送出");
}

/// 真實資料的長串：游標移到中間的顯示字元再開候選，預覽只換那一個字元，
/// 前後文與其他片段都留在畫面上；套用後才真正取代。
#[test]
fn test_ja_long_clause_preview_only_replaces_target() {
    let Some(mut s) = ja_session() else {
        eprintln!("缺少測試資料：packs/ja-lexicon-mozc-v2.pack");
        return;
    };
    press_chars(&mut s, "watashihanihongowotukatteimasu");
    let before = s.display_text();
    assert_eq!(before, "私は日本語を使っています", "真實資料的長串");

    let target = "使";
    let target_start = before.find(target).expect("長串要有目標文節");
    let target_rune = before[..target_start].chars().count();
    let tail_runes = before.chars().count() - target_rune;
    for _ in 0..tail_runes {
        press(&mut s, Key::new(KeyCode::ArrowLeft, None, false));
    }
    assert_eq!(
        s.snapshot().cursor_rune,
        target_rune,
        "游標要停在目標文節開頭"
    );

    down(&mut s);
    let opened = s.snapshot();
    assert!(opened.navigating, "目標文節的 ↓ 要進入選取模式");
    assert_eq!(opened.preedit, before, "開窗時顯示目前的詞面");

    let at = s
        .candidates()
        .iter()
        .position(|candidate| candidate.surface != target)
        .expect("目標文節要有替代候選");
    s.select(at);
    let surface = s.candidates()[at].surface.clone();
    let preview = s.snapshot();
    let want = format!(
        "{}{}{}",
        &before[..target_start],
        surface,
        &before[target_start + target.len()..]
    );
    assert_eq!(preview.preedit, want, "預覽只換目標字元");
    assert!(preview.preedit.starts_with("私は日本語を"), "前文不變");
    assert!(preview.preedit.ends_with("っています"), "後文不變");
    assert_eq!(s.display_text(), want, "組字區與高亮候選一致");

    let applied = s.choose_candidate(at).expect("套用長串字元候選");
    assert_eq!(applied.preedit, want, "套用後才真正取代");
    assert_commit_matches_display(&mut s, "長串文節送出");
}

/// 英文補全候選：還沒套用前，移動高亮不會提前把候選塞進組字區。
#[test]
fn test_english_completion_candidate_has_no_preview() {
    let mut s = english_session();
    press_chars(&mut s, "hell");
    assert_eq!(s.display_text(), "hell", "補全前只顯示已輸入的字母");
    assert_eq!(s.snapshot().suggestion.as_deref(), Some("o"));

    // 整段英文補全只由 Tab 開啟；↓ 只對游標字母。
    press(&mut s, Key::new(KeyCode::Tab, None, false));
    assert!(s.snapshot().navigating, "英文候選 Tab 要進入選取模式");
    let at = select_surface(&mut s, "hello");
    assert_eq!(s.snapshot().preedit, "hell", "英文補全候選套用前不預覽");
    assert_consistent(&s, "英文補全候選高亮");

    let applied = s.choose_candidate(at).expect("套用英文補全");
    assert_eq!(applied.preedit, "hello", "套用後才補完");
    assert_eq!(
        assert_commit_matches_display(&mut s, "英文補全候選送出"),
        "hello"
    );
}

/// 英文補全：Tab 開候選窗，選了補完的字後畫面與送出都是它。
#[test]
fn test_english_completion_display_matches_commit() {
    let mut s = english_session();
    press_chars(&mut s, "hell");
    let opened = press(&mut s, Key::new(KeyCode::Tab, None, false));
    assert_eq!(opened.preedit, "hell", "Tab 只開窗，不直接套用");
    assert!(opened.navigating, "Tab 要進入候選選取");
    let at = select_surface(&mut s, "hello");
    let applied = s.choose_candidate(at).expect("套用英文補全");
    assert_eq!(applied.preedit, "hello");
    assert_consistent(&s, "英文補全");
    assert_eq!(
        assert_commit_matches_display(&mut s, "英文補全送出"),
        "hello"
    );
}

/// 字面候選：選回原始按鍵時，畫面與送出都是原始字面。
#[test]
fn test_literal_candidate_display_matches_commit() {
    let mut s = zh_session();
    press_chars(&mut s, "ji3");
    down(&mut s);
    let before = s.display_text();
    assert_eq!(before, "我", "開窗後先顯示目前的轉換結果");
    let at = select_surface(&mut s, "ji3");
    assert_eq!(
        s.snapshot().preedit,
        before,
        "字面候選套用前不預覽：原始按鍵只在選取後取代"
    );
    assert_consistent(&s, "字面候選高亮");
    let applied = s.choose_candidate(at).expect("套用字面候選");
    assert_eq!(applied.preedit, "ji3", "選回原始按鍵要顯示原始字面");
    assert_eq!(assert_commit_matches_display(&mut s, "字面候選送出"), "ji3");
}
