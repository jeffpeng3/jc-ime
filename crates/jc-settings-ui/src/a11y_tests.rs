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
    assert!(keys.starts_with("按鍵：單一字母或數字、f1–f12、"), "{keys}");
    assert!(
        keys.contains("home／end／pageup／pagedown／up／down／left／right"),
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
