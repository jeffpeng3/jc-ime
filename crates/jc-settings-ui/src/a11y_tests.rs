//! Headless GPUI 驗證：快捷鍵欄位的 accessibility 值與說明文字。

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    base::Root, point, px, size, AnyWindowHandle, AppContext, Bounds, Entity, SharedString,
    TestAppContext, WindowBounds, WindowOptions,
};

use crate::state::{LaunchOptions, SettingsView};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("jc-settings-ui-a11y-{tag}-{stamp}"));
        fs::create_dir_all(&path).expect("建立暫存目錄");
        Self(path)
    }

    fn settings(&self) -> PathBuf {
        self.0.join("settings.json")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn open_settings(
    cx: &mut TestAppContext,
    dir: &TempDir,
) -> (AnyWindowHandle, Entity<SettingsView>) {
    let view = SettingsView::load(&LaunchOptions {
        data_root: dir.0.clone(),
        settings_path: dir.settings(),
    });
    open_view(cx, view)
}

fn open_view(
    cx: &mut TestAppContext,
    view: SettingsView,
) -> (AnyWindowHandle, Entity<SettingsView>) {
    let (handle, content) = cx.update(|cx| {
        let bounds = Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(1080.), px(720.)),
        };
        let (window, content) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            move |_, cx| cx.new(|_| view),
        )
        .expect("開啟設定視窗");
        (window.downcast::<Root>().expect("Base Root"), content)
    });
    (handle.into(), content)
}

fn show_input_page(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    window.render_frame(cx);
    window.within("settings-sidebar").click("0-2", cx);
    window.render_frame(cx);
}

fn assert_chord_fields(window: &gpui_kit::Window, symbol: &str, emoji: &str, tag: &str) {
    assert_eq!(
        window.find_by_label("符號選單快捷鍵").value(),
        Some(symbol),
        "{tag}：符號選單欄位"
    );
    assert_eq!(
        window.find_by_label("表情選單快捷鍵").value(),
        Some(emoji),
        "{tag}：表情選單欄位"
    );
}

fn assert_help(window: &gpui_kit::Window, key: &str, chord: &str) {
    let format_id = SharedString::from(format!("keybind-help-format-{key}"));
    let expected = format!("格式：{chord}");
    assert_eq!(
        window.find(format_id).value(),
        Some(expected.as_str()),
        "{key} 的格式說明"
    );
    let modifiers_id = SharedString::from(format!("keybind-help-modifiers-{key}"));
    assert_eq!(
        window.find(modifiers_id).value(),
        Some("可用修飾鍵：ctrl、alt、shift；至少要有一個 ctrl 或 alt"),
        "{key} 的修飾鍵說明"
    );
    let keys_id = SharedString::from(format!("keybind-help-keys-{key}"));
    let keys_snapshot = window.find(keys_id);
    let keys = keys_snapshot.value().unwrap_or_default();
    assert!(
        keys.starts_with("按鍵：單一字元（字母、數字、標點，例如 , . ; /）、f1–f12，或具名鍵"),
        "{keys}"
    );
    assert!(keys.contains("backspace"), "{keys}");
    assert!(
        keys.contains("pageup／pagedown／up／down／left／right"),
        "{keys}"
    );
}

#[gpui_kit::test]
fn four_settings_states_show_default_chords_and_help(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let cases: [(&str, Option<&[u8]>); 4] = [
        ("missing", None),
        ("empty-object", Some(b"{}")),
        (
            "no-keybinds",
            Some(br#"{"schema":1,"space_selects":false}"#),
        ),
        (
            "empty-chords",
            Some(br#"{"keybinds":{"symbol_menu":"","emoji_palette":""}}"#),
        ),
    ];

    for (tag, content) in cases {
        let dir = TempDir::new(tag);
        if let Some(bytes) = content {
            fs::write(dir.settings(), bytes).expect("寫入設定檔");
        }
        let (handle, _) = open_settings(cx, &dir);
        cx.update_window(handle, |_, window, cx| {
            show_input_page(window, cx);
            assert_chord_fields(window, "ctrl+alt+,", "ctrl+alt+.", tag);
            assert_help(window, "keybind-symbol-menu", "ctrl+alt+,");
            assert_help(window, "keybind-emoji-palette", "ctrl+alt+.");
        })
        .expect("更新視窗");
    }
}

#[gpui_kit::test]
fn record_button_captures_chord_and_rejects_invalid_keys(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let dir = TempDir::new("record");
    let (handle, view) = open_settings(cx, &dir);

    cx.update_window(handle, |_, window, cx| {
        show_input_page(window, cx);

        let record_id = "record-keybind-keybind-symbol-menu";
        assert_eq!(
            window.find(record_id).label(),
            Some("錄製符號選單快捷鍵"),
            "快捷鍵欄位旁要有錄製按鈕"
        );
        assert_eq!(
            window.find("record-keybind-keybind-emoji-palette").label(),
            Some("錄製表情選單快捷鍵"),
            "兩個錄製按鈕的無障礙名稱要能分辨"
        );

        window.click(record_id, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("keybind-recording-keybind-symbol-menu").value(),
            Some("請按下要使用的組合鍵…"),
            "錄製中欄位顯示提示文字"
        );
        assert_eq!(
            window.find(record_id).label(),
            Some("取消錄製符號選單快捷鍵")
        );
        assert!(
            window
                .try_find("keybind-input-keybind-symbol-menu")
                .is_none(),
            "錄製中由提示文字取代輸入框"
        );

        window.press("a", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("keybind-error-keybind-symbol-menu").value(),
            Some("快捷鍵需要 Ctrl 或 Alt（不能是純按鍵）")
        );
        assert_eq!(
            window.find("keybind-recording-keybind-symbol-menu").value(),
            Some("請按下要使用的組合鍵…"),
            "無效組合後繼續等"
        );

        window.press("shift-a", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("keybind-error-keybind-symbol-menu").value(),
            Some("快捷鍵不能只用 Shift；請加上 Ctrl 或 Alt")
        );
        assert_eq!(
            window.find("keybind-recording-keybind-symbol-menu").value(),
            Some("請按下要使用的組合鍵…"),
            "Shift-only 後繼續等"
        );

        window.press("ctrl-alt-,", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find_by_label("符號選單快捷鍵").value(),
            Some("ctrl+alt+,"),
            "錄製結果寫回欄位"
        );
        assert!(
            window
                .try_find("keybind-recording-keybind-symbol-menu")
                .is_none(),
            "成功後結束錄製"
        );
        assert!(
            window
                .try_find("keybind-error-keybind-symbol-menu")
                .is_none(),
            "成功後錯誤消失"
        );
        assert_eq!(window.find(record_id).label(), Some("錄製符號選單快捷鍵"));
    })
    .expect("更新視窗");

    let value = cx.update(|cx| view.read(cx).value("keybind-symbol-menu"));
    assert_eq!(value, "ctrl+alt+,");
    let report = jc_settings::read(&dir.settings(), "linux");
    assert_eq!(report.state, jc_settings::FileState::Ok);
    assert_eq!(report.settings.keybinds.symbol_menu, "ctrl+alt+,");
}

#[gpui_kit::test]
fn escape_cancels_recording_and_keeps_original_value(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let dir = TempDir::new("record-escape");
    let (handle, _) = open_settings(cx, &dir);

    cx.update_window(handle, |_, window, cx| {
        show_input_page(window, cx);
        let record_id = "record-keybind-keybind-symbol-menu";
        window.click(record_id, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("keybind-recording-keybind-symbol-menu").value(),
            Some("請按下要使用的組合鍵…")
        );

        window.press("escape", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find_by_label("符號選單快捷鍵").value(),
            Some("ctrl+alt+,"),
            "Esc 取消後保留原值"
        );
        assert!(
            window
                .try_find("keybind-recording-keybind-symbol-menu")
                .is_none(),
            "Esc 後離開錄製"
        );
        assert_eq!(window.find(record_id).label(), Some("錄製符號選單快捷鍵"));
    })
    .expect("更新視窗");
}

#[gpui_kit::test]
fn manual_edit_after_recording_still_saves(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let dir = TempDir::new("record-manual");
    let (handle, _) = open_settings(cx, &dir);

    cx.update_window(handle, |_, window, cx| {
        show_input_page(window, cx);
        window.click("record-keybind-keybind-symbol-menu", cx);
        window.render_frame(cx);
        window.press("ctrl-alt-b", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find_by_label("符號選單快捷鍵").value(),
            Some("ctrl+alt+b")
        );

        window.click("keybind-input-keybind-symbol-menu", cx);
        window.press("ctrl-a", cx);
        window.input("ctrl+alt+x", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find_by_label("符號選單快捷鍵").value(),
            Some("ctrl+alt+x"),
            "錄製後仍可手動編輯並存檔"
        );
        assert!(window
            .try_find("keybind-error-keybind-symbol-menu")
            .is_none());
    })
    .expect("更新視窗");

    let report = jc_settings::read(&dir.settings(), "linux");
    assert_eq!(report.settings.keybinds.symbol_menu, "ctrl+alt+x");
}

#[gpui_kit::test]
fn test_switching_section_cancels_recording(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let dir = TempDir::new("record-section-switch");
    let (handle, view) = open_settings(cx, &dir);

    cx.update_window(handle, |_, window, cx| {
        show_input_page(window, cx);
        let record_id = "record-keybind-keybind-symbol-menu";
        window.click(record_id, cx);
        window.render_frame(cx);
        assert!(view.read(cx).is_recording("keybind-symbol-menu"));

        // 點側邊欄切到「日文」：焦點離開錄製按鈕，錄製要取消。
        window.within("settings-sidebar").click("0-1", cx);
        window.render_frame(cx);
        assert!(
            !view.read(cx).is_recording("keybind-symbol-menu"),
            "切分頁後不能還掛著錄製狀態"
        );
        assert!(
            window
                .try_find("keybind-recording-keybind-symbol-menu")
                .is_none(),
            "欄位不在畫面時不該顯示提示"
        );

        // 取消後按鍵不應再改值，也不該留下錯誤。
        window.press("ctrl-alt-b", cx);
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).value("keybind-symbol-menu"),
            "ctrl+alt+,",
            "切頁取消後按鍵不會偷改設定"
        );
        assert!(window
            .try_find("keybind-error-keybind-symbol-menu")
            .is_none());

        // 切回「輸入」分頁，欄位回到原值，按鈕回到「錄製」。
        window.within("settings-sidebar").click("0-2", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find_by_label("符號選單快捷鍵").value(),
            Some("ctrl+alt+,")
        );
        assert_eq!(window.find(record_id).label(), Some("錄製符號選單快捷鍵"));
    })
    .expect("更新視窗");

    let report = jc_settings::read(&dir.settings(), "linux");
    assert_eq!(report.settings.keybinds.symbol_menu, "ctrl+alt+,");
}

#[gpui_kit::test]
fn test_esc_works_after_section_switch(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let dir = TempDir::new("record-section-esc");
    let (handle, view) = open_settings(cx, &dir);

    cx.update_window(handle, |_, window, cx| {
        show_input_page(window, cx);
        window.click("record-keybind-keybind-symbol-menu", cx);
        window.render_frame(cx);
        window.within("settings-sidebar").click("0-1", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).is_recording("keybind-symbol-menu"));

        // 切頁已經取消，Esc 不該產生錯誤或改值，也不該把舊錄製叫回來。
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).is_recording("keybind-symbol-menu"));
        assert_eq!(view.read(cx).value("keybind-symbol-menu"), "ctrl+alt+,");
        assert!(window
            .try_find("keybind-error-keybind-symbol-menu")
            .is_none());
    })
    .expect("更新視窗");
}

#[gpui_kit::test]
fn tab_is_captured_while_recording_and_keeps_waiting(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let dir = TempDir::new("record-tab");
    let (handle, view) = open_settings(cx, &dir);

    cx.update_window(handle, |_, window, cx| {
        show_input_page(window, cx);
        window.click("record-keybind-keybind-symbol-menu", cx);
        window.render_frame(cx);

        window.press("tab", cx);
        window.render_frame(cx);
        assert!(
            view.read(cx).is_recording("keybind-symbol-menu"),
            "Tab 不該讓焦點跑走並取消錄製"
        );
        assert_eq!(
            window.find("record-keybind-keybind-symbol-menu").focused(),
            Some(true),
            "Tab 不該把焦點帶離錄製按鈕"
        );
        assert_eq!(
            window.find("keybind-recording-keybind-symbol-menu").value(),
            Some("請按下要使用的組合鍵…"),
            "Tab 後仍要繼續等"
        );
        assert_eq!(
            window.find("keybind-error-keybind-symbol-menu").value(),
            Some("快捷鍵需要 Ctrl 或 Alt（不能是純按鍵）")
        );

        // 連按 Tab 後仍能完成錄製。
        window.press("tab", cx);
        window.press("ctrl-alt-b", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find_by_label("符號選單快捷鍵").value(),
            Some("ctrl+alt+b")
        );
        assert!(!view.read(cx).is_recording("keybind-symbol-menu"));
    })
    .expect("更新視窗");
}

#[gpui_kit::test]
fn invalid_chord_error_is_next_to_field_and_reset_restores_default(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let dir = TempDir::new("reset");
    let (handle, view) = open_settings(cx, &dir);

    cx.update_window(handle, |_, window, cx| {
        show_input_page(window, cx);
        view.update(cx, |view, cx| {
            view.apply("keybind-symbol-menu", "a");
            cx.notify();
        });
        window.render_frame(cx);

        let error = window
            .find("keybind-error-keybind-symbol-menu")
            .value()
            .unwrap_or_default()
            .to_string();
        assert_eq!(error, "快捷鍵需要 Ctrl 或 Alt（不能是純按鍵）");

        window.click("reset-keybind-keybind-symbol-menu", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find_by_label("符號選單快捷鍵").value(),
            Some("ctrl+alt+,")
        );
        assert!(
            window
                .try_find("keybind-error-keybind-symbol-menu")
                .is_none(),
            "重設後錯誤訊息要消失"
        );
    })
    .expect("更新視窗");

    let value = cx.update(|cx| view.read(cx).value("keybind-symbol-menu"));
    assert_eq!(value, "ctrl+alt+,");
}
