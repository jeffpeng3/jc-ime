//! 畫面：分頁、設定項目與資訊列。
//!
//! 用 GPUI Kit 的 `component::setting` 元件組頁面。開關與下拉一改就存檔；文字
//! 欄位（快捷鍵）在 Enter 或失焦時才存檔，避免打到一半的 chord 被驗證擋掉。
//! 純顯示的資訊列用 `SettingField::render`，不假裝有設定鍵。

use gpui_kit::component::{
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    label::Label,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings as SettingsPanel},
    v_flex, ActiveTheme, Disableable, Sizable, StyledExt,
};
use gpui_kit::*;

use crate::options::{self, Kind, Opt};
use crate::state::SettingsView;

/// 文字設定欄位的輸入狀態；`_subscription` 活著才會收到 InputEvent。
struct TextFieldState {
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(header(self, cx))
            .child(
                div().flex_1().min_h_0().child(
                    SettingsPanel::new("settings-panel")
                        .page(zh_page(&this, self))
                        .page(ja_page(&this, self))
                        .page(input_page(&this, self))
                        .page(data_page(&this, self)),
                ),
            )
    }
}

fn header(view: &SettingsView, cx: &App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let mut banner = v_flex().gap_1();
    banner = banner.child(
        Label::new("設定檔儲存後，引擎會在 1 秒內熱重載；快捷鍵表最多 5 秒或下次取得焦點時更新。")
            .id("banner-effect")
            .text_sm()
            .text_color(muted),
    );
    if view.read_only() {
        banner = banner.child(
            Label::new("設定檔來自較新版本，本版只讀不寫。")
                .id("banner-readonly")
                .text_sm()
                .text_color(cx.theme().danger),
        );
    }
    if let Some(error) = &view.load_error {
        banner = banner.child(
            Label::new(format!("設定檔讀取失敗，這次不會寫入：{error}"))
                .id("banner-error")
                .text_sm()
                .text_color(cx.theme().danger),
        );
    }
    for (index, note) in view.notes.iter().enumerate() {
        banner = banner.child(
            Label::new(note.clone())
                .id(("banner-note", index))
                .text_sm()
                .text_color(muted),
        );
    }
    if let Some(message) = view.status.message() {
        let color = if view.status.is_error() {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        };
        banner = banner.child(
            Label::new(message)
                .id("banner-status")
                .text_sm()
                .text_color(color),
        );
    }

    v_flex()
        .gap_2()
        .p_4()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .gap_1()
                        .child(Label::new("0c 設定").id("title").text_lg().font_semibold())
                        .child(
                            Label::new(format!("設定檔：{}", view.settings_path.display()))
                                .id("settings-path")
                                .text_sm()
                                .text_color(muted),
                        ),
                )
                .child(
                    v_flex()
                        .items_end()
                        .gap_1()
                        .child(
                            Button::new("restart-engine")
                                .label("重新啟動引擎")
                                .disabled(true),
                        )
                        .child(
                            Label::new("要等安裝器提供服務控制")
                                .id("restart-hint")
                                .text_sm()
                                .text_color(muted),
                        ),
                ),
        )
        .child(banner)
}

fn zh_page(this: &Entity<SettingsView>, view: &SettingsView) -> SettingPage {
    let fuzzy_items = options::FUZZY
        .iter()
        .map(|opt| option_item(this, view, opt));
    let model_items = options::ZH.iter().map(|opt| option_item(this, view, opt));

    SettingPage::new("注音")
        .default_open(true)
        .resettable(false)
        .description("大千注音輸入：模糊音、學習與選字模型。")
        .group(SettingGroup::new().title("模糊音").items(fuzzy_items))
        .group(SettingGroup::new().title("學習與選字").items(model_items))
}

fn ja_page(this: &Entity<SettingsView>, view: &SettingsView) -> SettingPage {
    let conversion_items = options::JA
        .iter()
        .filter(|opt| {
            matches!(
                opt.key,
                "ja-model" | "ja-neural-mode" | "ja-live-conversion" | "ja-rerank"
            )
        })
        .map(|opt| option_item(this, view, opt));
    let dictionary_items = options::JA
        .iter()
        .filter(|opt| {
            !matches!(
                opt.key,
                "ja-model" | "ja-neural-mode" | "ja-live-conversion" | "ja-rerank"
            )
        })
        .map(|opt| option_item(this, view, opt));

    SettingPage::new("日文")
        .resettable(false)
        .description("日文かな漢字変換的路徑與候選。")
        .group(SettingGroup::new().title("轉換").items(conversion_items))
        .group(
            SettingGroup::new()
                .title("候選與詞庫")
                .items(dictionary_items),
        )
}

fn input_page(this: &Entity<SettingsView>, view: &SettingsView) -> SettingPage {
    let items: Vec<SettingItem> = options::INPUT
        .iter()
        .map(|opt| option_item(this, view, opt))
        .collect();

    SettingPage::new("輸入")
        .resettable(false)
        .description("中英切換、啟動模式與空白鍵行為；第一次開啟即可修改。")
        .group(SettingGroup::new().title("鍵位與模式").items(items))
}

fn data_page(this: &Entity<SettingsView>, view: &SettingsView) -> SettingPage {
    let data = &view.data;
    let state_label = match data.settings_state {
        jc_settings::FileState::Ok => "可讀取".to_string(),
        jc_settings::FileState::Missing => "第一次開啟時建立；目前尚未建立".to_string(),
        jc_settings::FileState::Corrupt => "無法解析；本程式不會覆寫".to_string(),
        jc_settings::FileState::Unreadable => "讀不到；本程式不會寫入".to_string(),
    };

    SettingPage::new("資料")
        .resettable(false)
        .description("資料根目錄、manifest 版本與模型檔狀態。")
        .group(
            SettingGroup::new()
                .title("位置")
                .item(info_item(
                    "資料根目錄",
                    "引擎的 --data；模型、詞典與設定檔都在這下面。",
                    data.root.display().to_string(),
                    false,
                ))
                .item(info_item(
                    "設定檔",
                    "設定程式讀寫的檔案；與 jc-engine 的 --settings 相同。",
                    format!("{}（{state_label}）", data.settings_path.display()),
                    data.settings_state == jc_settings::FileState::Corrupt
                        || data.settings_state == jc_settings::FileState::Unreadable,
                ))
                .item(rescan_item(this)),
        )
        .group(
            SettingGroup::new()
                .title("Manifest")
                .item(info_item(
                    "模型集合 manifest",
                    "models/manifest.json 的 schema；模型角色由它決定。",
                    data.model_manifest
                        .clone()
                        .unwrap_or_else(|| "找不到 models/manifest.json".to_string()),
                    data.model_manifest.is_none(),
                ))
                .item(info_item(
                    "注音資料包 manifest",
                    "build-zh/manifest.json 的格式與詞條數。",
                    data.zh_manifest
                        .clone()
                        .unwrap_or_else(|| "找不到 build-zh/manifest.json".to_string()),
                    data.zh_manifest.is_none(),
                )),
        )
        .group(SettingGroup::new().title("模型").item(model_item(
            "Jinen tokenizer",
            "Jinen 的 tokenizer；可從 JC_JINEN_TOKENIZER 或資料 manifest 覆寫。",
            &data.jinen_tokenizer,
            "需下載",
        )))
}

fn option_item(this: &Entity<SettingsView>, view: &SettingsView, opt: &Opt) -> SettingItem {
    let disabled = !view.editable();

    match &opt.kind {
        Kind::Switch => {
            let read_view = this.clone();
            let write_view = this.clone();
            let key = opt.key;
            SettingItem::new(
                opt.title,
                SettingField::switch(
                    move |cx| read_view.read(cx).value(key) == "on",
                    move |on, cx| {
                        write_view.update(cx, |view, cx| {
                            view.apply_bool(key, on);
                            cx.notify();
                        });
                    },
                ),
            )
            .description(options::description(opt, &view.defaults))
            .disabled(disabled)
        }
        Kind::Choice(choices) => {
            let read_view = this.clone();
            let write_view = this.clone();
            let key = opt.key;
            let choices: Vec<(SharedString, SharedString)> = choices
                .iter()
                .map(|choice| (choice.value.into(), choice.label.into()))
                .collect();
            SettingItem::new(
                opt.title,
                SettingField::dropdown(
                    choices,
                    move |cx| SharedString::from(read_view.read(cx).value(key)),
                    move |value: SharedString, cx| {
                        write_view.update(cx, |view, cx| {
                            view.apply(key, &value);
                            cx.notify();
                        });
                    },
                ),
            )
            .description(options::description(opt, &view.defaults))
            .disabled(disabled)
        }
        Kind::Text => keybind_item(this, view, opt, disabled),
    }
}

/// 快捷鍵欄位：輸入框、重設按鈕、chord 語法說明與欄位錯誤都在同一區塊。
fn keybind_item(
    this: &Entity<SettingsView>,
    view: &SettingsView,
    opt: &Opt,
    disabled: bool,
) -> SettingItem {
    let read_view = this.clone();
    let write_view = this.clone();
    let reset_view = this.clone();
    let key = opt.key;
    let title = opt.title;
    let help = options::chord_help(opt, &view.defaults);

    // 快捷鍵要整段打完才能驗證，所以不像開關／下拉那樣每個 Change 都存檔；
    // 只有在 Enter 或失焦時才交給 Settings::set，錯誤會留在欄位下方與狀態列。
    SettingItem::render(move |options, window, cx| {
        let value = SharedString::from(read_view.read(cx).value(key));
        let error = read_view.read(cx).field_error(key).map(ToOwned::to_owned);
        let muted = cx.theme().muted_foreground;
        let state = window.use_keyed_state(
            SharedString::from(format!(
                "text-state-{}-{}-{}",
                options.page_ix(),
                options.group_ix(),
                options.item_ix()
            )),
            cx,
            {
                let write_view = write_view.clone();
                let value = value.clone();
                move |window, cx| {
                    let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
                    let _subscription = cx.subscribe(&input, {
                        move |_, input, event: &InputEvent, cx| {
                            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                                let value = input.read(cx).value();
                                write_view.update(cx, |view, cx| {
                                    view.apply(key, &value);
                                    cx.notify();
                                });
                            }
                        }
                    });
                    TextFieldState {
                        input,
                        _subscription,
                    }
                }
            },
        );

        // 外部改動（或存檔失敗後離開欄位）時把顯示值拉回設定值；
        // 正在輸入時不要覆蓋使用者打到一半的內容。
        state.update(cx, |state, cx| {
            let editing = state.input.read(cx).focus_handle(cx).is_focused(window);
            if !editing && state.input.read(cx).value() != value {
                state.input.update(cx, |input, cx| {
                    input.set_value(value.clone(), window, cx);
                });
            }
        });

        let reset_view = reset_view.clone();
        let state = state.read(cx);
        let input = Input::new(&state.input)
            .id(SharedString::from(format!("keybind-input-{key}")))
            .aria_label(title)
            .disabled(options.is_disabled())
            .with_size(options.size())
            .w_full()
            .into_any_element();
        let reset = Button::new(SharedString::from(format!("reset-keybind-{key}")))
            .label("重設為預設")
            .small()
            .disabled(options.is_disabled())
            .on_click(move |_, _, cx| {
                reset_view.update(cx, |view, cx| {
                    view.reset_to_default(key);
                    cx.notify();
                });
            })
            .into_any_element();

        let mut item = v_flex()
            .w_full()
            .gap_2()
            .child(Label::new(title).text_sm())
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(input))
                    .child(reset),
            )
            .child(
                Label::new(help.format.clone())
                    .id(SharedString::from(format!("keybind-help-format-{key}")))
                    .text_sm()
                    .text_color(muted),
            )
            .child(
                Label::new(help.modifiers)
                    .id(SharedString::from(format!("keybind-help-modifiers-{key}")))
                    .text_sm()
                    .text_color(muted),
            )
            .child(
                Label::new(help.keys)
                    .id(SharedString::from(format!("keybind-help-keys-{key}")))
                    .text_sm()
                    .text_color(muted),
            );
        if let Some(error) = error {
            item = item.child(
                Label::new(error)
                    .id(SharedString::from(format!("keybind-error-{key}")))
                    .text_sm()
                    .text_color(cx.theme().danger),
            );
        }
        item.into_any_element()
    })
    .keywords([opt.title, "快捷鍵", "chord"])
    .disabled(disabled)
}

fn info_item(
    title: &'static str,
    description: impl Into<SharedString>,
    value: String,
    bad: bool,
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::render(move |_, _, cx| {
            Label::new(value.clone())
                .id(title)
                .text_sm()
                .text_color(if bad {
                    cx.theme().danger
                } else {
                    cx.theme().foreground
                })
        }),
    )
    .description(description.into())
}

/// 顯示模型檔狀態：有檔案就寫「已安裝」，缺檔才用警告色顯示 `missing`。
///
/// 這裡只列仍在使用或隨 MSI 散布的角色；已移除的角色不在模型清單，
/// 不會被誤報成「缺少」。
fn model_item(
    title: &'static str,
    description: &'static str,
    path: &Option<std::path::PathBuf>,
    missing: &'static str,
) -> SettingItem {
    match path {
        Some(_) => info_item(title, description, "已安裝".to_string(), false),
        None => info_item(title, description, missing.to_string(), true),
    }
}

fn rescan_item(this: &Entity<SettingsView>) -> SettingItem {
    let view = this.clone();
    SettingItem::render(move |_, _, _cx| {
        Button::new("rescan-data")
            .label("重新檢查")
            .on_click({
                let view = view.clone();
                move |_, _, cx| {
                    view.update(cx, |view, cx| {
                        view.rescan();
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    })
    .keywords(["資料", "模型", "manifest"])
    .description("重新讀取資料根目錄與模型檔。")
}
