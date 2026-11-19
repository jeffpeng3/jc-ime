//! 輸入習慣（profile）：完整、不可變的規則快照。
//!
//! 對照前身 `internal/settings/profiles.go`。內建目錄更新不會改動已選用的風格
//! 或使用者的複本。這裡只管資料與驗證；設定程式的說明文字與頁面不在本 crate。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use jc_core::inputstyle::{self, Preset, Rules};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::profile_rules::{feature_label, lookup_rule, rule_value_label};
use crate::settings::{on_off, SetError, Settings, SettingsError, LAYOUT_STANDARD};

/// 由輸入習慣管理的設定鍵。
pub(crate) const PROFILE_KEYS: [&str; 14] = [
    "toggle-key",
    "caps-lock-english",
    "esc-clear",
    "space-selects",
    "advance-after-select",
    "candidate-cursor",
    "candidate-layout",
    "selection-keys",
    "mixed-input",
    "english-autocomplete",
    "emoji-shortcut",
    "emoji-equals-first",
    "ctrl-punctuation",
    "toneless",
];

/// 一個完整的設定快照。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// 風格代號。
    #[serde(default)]
    pub id: String,
    /// 顯示名稱。
    #[serde(default)]
    pub name: String,
    /// 是否為內建風格。
    #[serde(default)]
    pub builtin: bool,
    /// 來源代號。
    #[serde(default)]
    pub source: String,
    /// 快照的規則版本。
    #[serde(default)]
    pub version: i32,
    /// 風格管理的設定值。
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    /// 輸入習慣規則。
    #[serde(default)]
    pub rules: Rules,
}

/// 設定檔 `input_profile` 的內容。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ProfileStore {
    /// 目前選用的風格代號。
    #[serde(default)]
    pub(crate) active: String,
    /// 已保存的風格。
    #[serde(default)]
    pub(crate) items: Vec<Profile>,
}

/// 輸入習慣的操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileAction {
    /// 選用既有風格。
    Select,
    /// 複製成自訂風格。
    Copy,
    /// 改名；只接受自訂風格。
    Rename,
    /// 刪除；只接受非使用中的自訂風格。
    Delete,
}

impl ProfileAction {
    /// 由命令列字串解析。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "select" => Some(ProfileAction::Select),
            "copy" => Some(ProfileAction::Copy),
            "rename" => Some(ProfileAction::Rename),
            "delete" => Some(ProfileAction::Delete),
            _ => None,
        }
    }
}

fn profile_error(message: impl Into<String>) -> SetError {
    SetError::Message(message.into())
}

/// 內建目錄裡有這個代號嗎。
fn reserved(id: &str) -> bool {
    inputstyle::get(id).is_some()
}

fn valid_profile_name(name: &str) -> bool {
    let count = name.chars().count();
    name == name.trim() && (1..=80).contains(&count) && !name.chars().any(char::is_control)
}

fn valid_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn managed_key(key: &str) -> bool {
    lookup_rule(key).is_some() || key == "left-shift" || PROFILE_KEYS.contains(&key)
}

fn inputstyle_default_rules() -> Rules {
    inputstyle::get("0c")
        .map(|preset| preset.rules)
        .unwrap_or_default()
}

fn builtin_profile(preset: &Preset) -> Profile {
    let behavior = &preset.behavior;
    let values = BTreeMap::from([
        ("toggle-key", behavior.toggle_key.clone()),
        ("caps-lock-english", on_off(behavior.caps_lock_english)),
        ("esc-clear", on_off(behavior.esc_clear)),
        ("space-selects", on_off(behavior.space_selects)),
        (
            "advance-after-select",
            on_off(behavior.advance_after_select),
        ),
        ("candidate-cursor", behavior.candidate_cursor.clone()),
        ("candidate-layout", behavior.candidate_layout.clone()),
        ("selection-keys", behavior.selection_keys.clone()),
        ("mixed-input", on_off(behavior.mixed_input)),
        (
            "english-autocomplete",
            on_off(behavior.english_autocomplete),
        ),
        ("emoji-shortcut", behavior.emoji_shortcut.clone()),
        ("emoji-equals-first", on_off(behavior.emoji_equals_first)),
        ("ctrl-punctuation", on_off(behavior.ctrl_punctuation)),
        ("toneless", on_off(behavior.toneless)),
    ])
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect();
    Profile {
        id: preset.id.clone(),
        name: preset.name.clone(),
        builtin: true,
        source: preset.id.clone(),
        version: inputstyle::VERSION,
        values,
        rules: preset.rules.clone(),
    }
}

fn new_profile_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(nanos.to_le_bytes());
    hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("custom-{}", &digest[..24])
}

/// 嚴格解析 `input_profile`：有不認得的成員就視為較新格式。
fn parse_store(raw: &Value) -> Option<ProfileStore> {
    let object = raw.as_object()?;
    if !only_keys(object, &["active", "items"]) {
        return None;
    }
    let items = object.get("items")?.as_array()?;
    for item in items {
        let profile = item.as_object()?;
        if !only_keys(
            profile,
            &[
                "id", "name", "builtin", "source", "version", "values", "rules",
            ],
        ) {
            return None;
        }
        let values = profile.get("values")?.as_object()?;
        if values.values().any(|value| !value.is_string()) {
            return None;
        }
        let rules = profile.get("rules")?.as_object()?;
        if !only_keys(
            rules,
            &[
                "caps_letter",
                "candidate_navigation",
                "shift_delete",
                "up",
                "candidate_tab",
                "candidate_page_size",
                "version",
                "punctuation",
                "shift_space",
                "tab",
                "candidate_space",
                "candidate_enter",
                "shift_letter",
            ],
        ) {
            return None;
        }
    }
    serde_json::from_value(raw.clone()).ok()
}

fn only_keys(object: &serde_json::Map<String, Value>, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}

impl Settings {
    /// 風格管理的設定值快照。
    pub(crate) fn profile_values(&self) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        for key in PROFILE_KEYS {
            if let Some(value) = self.value(key) {
                values.insert(key.to_string(), value);
            }
        }
        values.insert(
            "caps-lock-english".to_string(),
            on_off(self.caps_lock_english),
        );
        values
    }

    /// 設定檔中的風格清單；沒有或讀不懂時用「我的原有設定」。
    pub(crate) fn profile_store(&self) -> ProfileStore {
        if let Some(raw) = self.profiles.as_ref() {
            if let Ok(store) = serde_json::from_value::<ProfileStore>(raw.clone()) {
                return store;
            }
        }
        ProfileStore {
            active: "legacy".to_string(),
            items: vec![Profile {
                id: "legacy".to_string(),
                name: "我的原有設定".to_string(),
                builtin: false,
                source: "legacy".to_string(),
                version: inputstyle::VERSION,
                values: self.profile_values(),
                rules: inputstyle_default_rules(),
            }],
        }
    }

    pub(crate) fn put_profile_store(&mut self, store: ProfileStore) {
        self.profiles = serde_json::to_value(store).ok();
    }

    /// 目前選用的風格。
    pub fn active_profile(&self, goos: &str) -> Profile {
        let store = self.profile_store();
        if let Some(profile) = store.items.iter().find(|p| p.id == store.active) {
            return profile.clone();
        }
        match inputstyle::get_for_platform("0c", goos) {
            Some(preset) => builtin_profile(&preset),
            None => Profile::default(),
        }
    }

    /// 目前風格實際生效的規則。
    pub fn profile_rules(&self, goos: &str) -> Rules {
        if self.profile_invalid {
            return inputstyle_default_rules();
        }
        self.active_profile(goos).rules
    }

    /// 目前風格的指紋；改名不會改變它。
    pub fn profile_fingerprint(&self, goos: &str) -> String {
        let profile = self.active_profile(goos);
        let payload = serde_json::json!({
            "ID": profile.id,
            "Values": self.profile_values(),
            "Rules": profile.rules,
        });
        let bytes = serde_json::to_vec(&payload).unwrap_or_default();
        format!("{:x}", Sha256::digest(bytes))
    }

    /// 這一項是否由選用的風格固定。
    pub fn profile_locked(&self, key: &str, goos: &str) -> bool {
        managed_key(key) && self.active_profile(goos).builtin
    }

    pub(crate) fn sync_profile(&mut self) {
        if self.profiles.is_none() {
            let store = self.profile_store();
            self.put_profile_store(store);
            return;
        }
        let mut store = self.profile_store();
        for profile in &mut store.items {
            if profile.id == store.active && !profile.builtin {
                profile.values = self.profile_values();
            }
        }
        self.put_profile_store(store);
    }

    pub(crate) fn apply_profile(
        &mut self,
        profile: &Profile,
        goos: &str,
    ) -> Result<(), SettingsError> {
        profile
            .rules
            .validate()
            .map_err(|err| profile_error(err.to_string()))?;
        let mut next = self.clone();
        next.profiles = None;
        for key in PROFILE_KEYS {
            let Some(value) = profile.values.get(key) else {
                return Err(profile_error(format!("風格缺少設定：{key}")).into());
            };
            if key == "caps-lock-english" {
                if value != "on" && value != "off" {
                    return Err(profile_error("Caps Lock 設定無效").into());
                }
                next.caps_lock_english = value == "on";
                next.forget("caps_lock_english");
                continue;
            }
            next.apply_set(key, value, goos)?;
        }
        next.profiles = self.profiles.clone();
        *self = next;
        Ok(())
    }

    /// 這個風格能不能套用在目前的鍵盤排列上。
    pub fn profile_availability(&self, profile: &Profile) -> Result<(), String> {
        if self.keyboard_layout != LAYOUT_STANDARD && profile.rules != inputstyle_default_rules() {
            return Err("此風格目前支援標準注音排列；請先在「鍵盤與符號」選擇標準排列".to_string());
        }
        Ok(())
    }

    /// 列出內建目錄與已保存的自訂風格。
    pub fn profiles(&self, goos: &str) -> Vec<Profile> {
        let store = self.profile_store();
        let mut out = Vec::new();
        for preset in inputstyle::presets_for_platform(goos) {
            let mut profile = builtin_profile(&preset);
            if let Some(saved) = store.items.iter().find(|saved| saved.id == profile.id) {
                profile = saved.clone();
            }
            out.push(profile);
        }
        out.extend(store.items.iter().filter(|p| !p.builtin).cloned());
        out
    }

    /// 選用、複製、改名或刪除輸入習慣。
    ///
    /// 失敗時不改動目前的設定。
    pub fn profile_action(
        &mut self,
        action: ProfileAction,
        id: &str,
        name: &str,
        goos: &str,
    ) -> Result<(), SettingsError> {
        if self.read_only {
            return Err(SettingsError::ReadOnly);
        }
        let mut next = self.clone();
        let mut store = next.profile_store();
        let idx = store.items.iter().position(|p| p.id == id);
        let mut profile = match idx {
            Some(index) => store.items[index].clone(),
            None => match inputstyle::get_for_platform(id, goos) {
                Some(preset) => builtin_profile(&preset),
                None => return Err(profile_error("找不到指定的輸入習慣").into()),
            },
        };
        let mut append = false;
        match action {
            ProfileAction::Select | ProfileAction::Copy => {
                next.profile_availability(&profile).map_err(profile_error)?;
                if action == ProfileAction::Copy {
                    let name = name.trim();
                    if !valid_profile_name(name) {
                        return Err(profile_error("名稱須為 1～80 字，不能包含控制字元").into());
                    }
                    if store.items.len() >= 64 {
                        return Err(profile_error("自訂風格已達上限，請先刪除不使用的風格").into());
                    }
                    profile.id = new_profile_id();
                    profile.name = name.to_string();
                    profile.builtin = false;
                    append = true;
                }
                next.apply_profile(&profile, goos)?;
                if append {
                    store.items.push(profile.clone());
                }
                store.active = profile.id.clone();
            }
            ProfileAction::Rename => {
                if profile.builtin {
                    return Err(profile_error("內建風格不能改名；請先複製成自訂").into());
                }
                let name = name.trim();
                if !valid_profile_name(name) {
                    return Err(profile_error("名稱須為 1～80 字，不能包含控制字元").into());
                }
                if let Some(index) = idx {
                    store.items[index].name = name.to_string();
                }
            }
            ProfileAction::Delete => {
                if profile.builtin {
                    return Err(profile_error("內建風格不能刪除").into());
                }
                if store.active == id {
                    return Err(profile_error("請先切換至其他風格，再刪除目前的風格").into());
                }
                if let Some(index) = idx {
                    store.items.remove(index);
                }
            }
        }
        next.put_profile_store(store);
        next.validate_profile_combination(goos)?;
        *self = next;
        Ok(())
    }

    /// 讀取 `input_profile`：不認得的格式只讀，但仍可用扁平欄位讀取。
    pub(crate) fn load_profiles(&mut self, raw: Value) {
        self.profiles = Some(raw.clone());
        self.profile_invalid = true;
        let Some(store) = parse_store(&raw) else {
            self.read_only = true;
            return;
        };
        if store.active.is_empty() || store.items.is_empty() || store.items.len() > 64 {
            self.read_only = true;
            return;
        }
        let mut seen = BTreeSet::new();
        let mut found = false;
        for profile in &store.items {
            if !valid_profile_id(&profile.id)
                || !seen.insert(profile.id.clone())
                || !valid_profile_name(&profile.name)
                || profile.version != inputstyle::VERSION
                || profile.rules.version != inputstyle::VERSION
                || profile.rules.validate().is_err()
            {
                self.read_only = true;
                return;
            }
            if reserved(&profile.id) != profile.builtin
                || (!reserved(&profile.id)
                    && profile.id != "legacy"
                    && !profile.id.starts_with("custom-"))
            {
                self.read_only = true;
                return;
            }
            for key in profile.values.keys() {
                if !managed_key(key) || key == "left-shift" {
                    self.read_only = true;
                    return;
                }
            }
            if profile.builtin && !reserved(&profile.id) {
                self.read_only = true;
                return;
            }
            let mut check = self.clone();
            if check.apply_profile(profile, "linux").is_err() {
                self.read_only = true;
                return;
            }
            if profile.id == store.active {
                found = true;
            }
        }
        if !found {
            self.read_only = true;
            return;
        }
        self.profile_invalid = false;
        let active = self.active_profile("linux");
        if self.apply_profile(&active, "linux").is_err() {
            self.read_only = true;
            self.profile_invalid = true;
        }
        self.put_profile_store(store);
        if self.validate_profile_combination("linux").is_err() {
            self.read_only = true;
            self.profile_invalid = true;
        }
    }

    /// 目前設定組合是否可用；衝突時回傳給人讀的原因。
    pub(crate) fn validate_profile_combination(&self, goos: &str) -> Result<(), SettingsError> {
        let profile = self.active_profile(goos);
        self.profile_availability(&profile).map_err(profile_error)?;
        let rules = self.profile_rules(goos);
        for conflict in style_conflicts() {
            if (conflict.when)(&rules) && feature_on(self, conflict.feature) {
                let value = lookup_rule(conflict.rule)
                    .map(|spec| (spec.get)(&rules))
                    .unwrap_or_default();
                return Err(profile_error(format!(
                    "「{}」與「{}：{}」不能同時使用；請先關閉其中一項",
                    feature_label(conflict.feature),
                    conflict.rule,
                    rule_value_label(conflict.rule, &value)
                ))
                .into());
            }
        }
        Ok(())
    }
}

/// 一條操作規則跟擴充功能真正衝突的組合。
struct StyleConflict {
    rule: &'static str,
    feature: &'static str,
    when: fn(&Rules) -> bool,
}

fn style_conflicts() -> Vec<StyleConflict> {
    vec![
        StyleConflict {
            rule: "style-shift-space",
            feature: "mixed-input",
            when: |rules| rules.shift_space == "fullwidth",
        },
        StyleConflict {
            rule: "style-tab",
            feature: "mixed-input",
            when: |rules| rules.tab == "segment",
        },
        StyleConflict {
            rule: "style-shift-letter",
            feature: "mixed-input",
            when: |rules| rules.shift_letter == "commit",
        },
        StyleConflict {
            rule: "style-shift-letter",
            feature: "mixed-input",
            when: |rules| rules.shift_letter == "lower",
        },
        StyleConflict {
            rule: "style-punctuation",
            feature: "toneless",
            when: |rules| rules.punctuation != "0c",
        },
    ]
}

fn feature_on(settings: &Settings, key: &str) -> bool {
    match key {
        "mixed-input" => settings.mixed_input,
        "english-autocomplete" => settings.english_autocomplete,
        "toneless" => settings.toneless,
        _ => false,
    }
}

/// 全新安裝的設定：選用內建「靈犀注音」，只留這一個風格。
pub(crate) fn fresh_settings(goos: &str) -> Settings {
    let mut settings = Settings::default_for_platform(goos);
    settings
        .profile_action(ProfileAction::Select, "0c", "", goos)
        .expect("內建輸入習慣必須合法");
    let mut store = settings.profile_store();
    if !store.items.is_empty() {
        store.items.remove(0);
    }
    settings.put_profile_store(store);
    settings
}
