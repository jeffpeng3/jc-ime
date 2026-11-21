//! 畫面：分頁、設定項目與資訊列。
//!
//! 用 GPUI Kit 的 `component::setting` 元件組頁面。可編輯項目一律走
//! [`crate::state::SettingsView::apply`]，變更即存檔；純顯示的資訊列用
//! `SettingField::render`，不假裝有設定鍵。

use gpui_kit::component::{
    button::Button,
    h_flex,
    label::Label,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings as SettingsPanel},
    v_flex, ActiveTheme, Disableable, StyledExt,
};
use gpui_kit::*;

use crate::options::{self, Kind, Opt};
use crate::state::SettingsView;

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
        Label::new("引擎下次啟動才生效；設定檔儲存後不會即時套用到執行中的引擎。")
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
    let model_items = options::ZH
        .iter()
        .map(|opt| option_item(this, view, opt))
        .chain(std::iter::once(model_status_item(view)));

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
    let active = view.settings.active_profile(view.goos);
    let mut items: Vec<SettingItem> = options::INPUT
        .iter()
        .map(|opt| option_item(this, view, opt))
        .collect();
    if active.builtin && view.editable() {
        items.push(profile_copy_item(this, &active.name));
    }
    let footer = if active.builtin {
        format!(
            "目前使用內建輸入習慣「{}」。標示為固定的項目要複製成自訂風格後才能修改。",
            active.name
        )
    } else {
        format!(
            "目前使用自訂輸入習慣「{}」，所有項目都可以修改。",
            active.name
        )
    };

    SettingPage::new("輸入")
        .resettable(false)
        .description("中英切換、啟動模式與空白鍵行為。")
        .group(
            SettingGroup::new()
                .title("鍵位與模式")
                .items(items)
                .footer(move |_, cx| {
                    Label::new(footer.clone())
                        .id("profile-footer")
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                }),
        )
}

fn data_page(this: &Entity<SettingsView>, view: &SettingsView) -> SettingPage {
    let data = &view.data;
    let state_label = match data.settings_state {
        jc_settings::FileState::Ok => "可讀取".to_string(),
        jc_settings::FileState::Missing => "還沒有設定檔，第一次修改時建立".to_string(),
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
        .group(
            SettingGroup::new()
                .title("模型")
                .item(model_item(
                    "注音選字模型",
                    "zh-select 權重；沒有時選字退回統計排序。",
                    &data.zh_select,
                    "需下載",
                ))
                .item(model_item(
                    "日文神經模型（Jinen）",
                    "日文轉換的主要神經模型。",
                    &data.jinen,
                    "需下載",
                ))
                .item(model_item(
                    "Jinen tokenizer",
                    "Jinen 的 tokenizer；與 GGUF 一起隨 MSI 散布。",
                    &data.jinen_tokenizer,
                    "需下載",
                ))
                .item(model_item(
                    "日文詞典 pack",
                    "Mozc 或 public-core 詞典；引擎啟動時載入。",
                    &data.ja_pack,
                    "需下載",
                )),
        )
}

fn option_item(this: &Entity<SettingsView>, view: &SettingsView, opt: &Opt) -> SettingItem {
    let locked = view.locked(opt.key);
    let mut description = options::description(opt, &view.defaults);
    if locked {
        let name = view.settings.active_profile(view.goos).name;
        description.push_str(&format!("　此項由輸入習慣「{name}」固定。"));
    }
    let disabled = !view.editable() || locked;

    let item = match &opt.kind {
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
        }
    };
    item.description(description).disabled(disabled)
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

fn model_status_item(view: &SettingsView) -> SettingItem {
    let ready = view.data.zh_select_ready();
    info_item(
        "選字模型狀態",
        "神經模型權重放在 models/zh-select；缺檔時引擎只記警告並用統計排序。",
        if ready {
            "已安裝".to_string()
        } else {
            "需下載".to_string()
        },
        !ready,
    )
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

fn profile_copy_item(this: &Entity<SettingsView>, name: &str) -> SettingItem {
    let view = this.clone();
    let label = format!("把「{name}」複製成自訂輸入習慣");
    let description = "複製後這 14 項鍵位規則由你管理；原本的內建風格保留在目錄裡。".to_string();
    SettingItem::render(move |_, _, _cx| {
        Button::new("copy-profile")
            .label(label.clone())
            .on_click({
                let view = view.clone();
                move |_, _, cx| {
                    view.update(cx, |view, cx| {
                        view.copy_profile();
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    })
    .description(description)
    .keywords(["輸入習慣", "profile", "自訂"])
}
