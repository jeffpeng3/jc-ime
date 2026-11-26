//! 設定程式的狀態：讀取、修改、存檔。
//!
//! 所有讀寫都經過 `jc-settings`：`settings::read` 負責缺檔與壞檔，
//! `Settings::set` 負責驗證，`settings::save` 先寫暫存檔再改名。UI 只拿這裡的
//! 值，不自己拼 JSON。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jc_settings::{
    read, save, Chord, ChordError, FileState, ProfileAction, Settings, SettingsError,
};

use crate::data::DataStatus;

/// 啟動參數。
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// 資料根目錄。
    pub data_root: PathBuf,
    /// 設定檔路徑。
    pub settings_path: PathBuf,
}

/// 上一次操作的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// 還沒操作。
    Idle,
    /// 已寫入設定檔。
    Saved,
    /// 操作失敗；內容是給人讀的原因。
    Failed(String),
}

impl Status {
    /// 橫幅文字。
    pub fn message(&self) -> Option<String> {
        match self {
            Status::Idle => None,
            Status::Saved => Some("已儲存；引擎會在 1 秒內熱重載，快捷鍵表最多 5 秒。".to_string()),
            Status::Failed(reason) => Some(reason.clone()),
        }
    }

    /// 是否為錯誤。
    pub fn is_error(&self) -> bool {
        matches!(self, Status::Failed(_))
    }
}

/// 設定程式的資料狀態。
pub struct SettingsView {
    /// 資料根目錄。
    pub data_root: PathBuf,
    /// 設定檔路徑。
    pub settings_path: PathBuf,
    /// 平台代號，交給 `jc-settings` 決定預設值。
    pub goos: &'static str,
    /// 平台預設值，只用來標「預設值」。
    pub defaults: Settings,
    /// 目前生效的設定。
    pub settings: Settings,
    /// 設定檔讀取狀態。
    pub file_state: FileState,
    /// 讀取時的錯誤；有值時不寫檔。
    pub load_error: Option<String>,
    /// 讀取時退回預設或保留原文的欄位說明。
    pub notes: Vec<String>,
    /// 上一次操作的結果。
    pub status: Status,
    /// 各設定鍵最近一次的錯誤；顯示在欄位旁，不只留在狀態列。
    pub field_errors: BTreeMap<String, String>,
    /// 正在錄製的快捷鍵欄位；有值時下一個按鍵會被這個欄位吃掉。
    pub recording: Option<String>,
    /// 目前已完成的 render 序號；用來偵測錄製欄位是否還在畫面上。
    pub render_seq: u64,
    /// 最後一次看到錄製欄位的 render 序號。
    pub recording_seen_seq: u64,
    /// 資料根目錄的檢查結果。
    pub data: DataStatus,
}

/// 由執行平台決定 `goos`。
pub fn current_goos() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// 把內建風格轉成自訂快照，讓所有輸入習慣項目都能直接修改。
fn make_editable(settings: &mut Settings, goos: &str) -> Result<(), SettingsError> {
    let active = settings.active_profile(goos);
    if !active.builtin {
        return Ok(());
    }
    let name = format!("{}（自訂）", active.name);
    settings.profile_action(ProfileAction::Copy, &active.id, &name, goos)
}

impl SettingsView {
    /// 讀取設定檔並建立狀態。
    ///
    /// 第一次啟動時直接把預設值存成可改的檔案，不再讓使用者先「複製」。
    pub fn load(options: &LaunchOptions) -> Self {
        let goos = current_goos();
        let mut report = read(&options.settings_path, goos);
        let mut notes = report.notes;
        let mut file_state = report.state;
        if file_state == FileState::Missing
            && report.error.is_none()
            && !report.settings.read_only()
        {
            match make_editable(&mut report.settings, goos) {
                Ok(()) => match save(&report.path, &report.settings) {
                    Ok(()) => {
                        file_state = FileState::Ok;
                        notes.push("第一次開啟已建立預設設定檔，所有項目可直接修改。".to_string());
                    }
                    Err(err) => {
                        notes.push(format!("無法建立預設設定檔：{err}；修改設定時會再試一次。"))
                    }
                },
                Err(err) => notes.push(format!("無法建立可修改的輸入習慣：{err}")),
            }
        }
        let load_error = report.error.as_ref().map(ToString::to_string);
        let data = DataStatus::probe(&options.data_root, &report.path, file_state);
        Self {
            data_root: options.data_root.clone(),
            settings_path: report.path.clone(),
            goos,
            defaults: Settings::default_for_platform(goos),
            settings: report.settings,
            file_state,
            load_error,
            notes,
            status: Status::Idle,
            field_errors: BTreeMap::new(),
            recording: None,
            render_seq: 0,
            recording_seen_seq: 0,
            data,
        }
    }

    /// 現在能不能寫入設定檔。
    pub fn editable(&self) -> bool {
        self.load_error.is_none() && !self.settings.read_only()
    }

    /// 讀取一個設定鍵目前的值。
    ///
    /// 空字串代表設定檔沒有可用值（例如舊檔的 `keybinds` 成員是空字串），
    /// 此時一律回到平台預設值。畫面上的每個欄位都走這裡，所以缺值不會只
    /// 影響單一欄位；`Settings::default_for_platform` 是唯一的預設來源。
    pub fn value(&self, key: &str) -> String {
        let value = self.settings.value(key).unwrap_or_default();
        if value.is_empty() {
            self.defaults.value(key).unwrap_or_default()
        } else {
            value
        }
    }

    /// 這個鍵是否由目前選用的輸入習慣固定。
    pub fn locked(&self, key: &str) -> bool {
        self.settings.profile_locked(key, self.goos)
    }

    /// 設定檔是否來自較新版本。
    pub fn read_only(&self) -> bool {
        self.settings.read_only()
    }

    /// 套用一個值並立刻存檔；失敗時保留原值。
    ///
    /// 舊檔若還用著內建風格，第一次修改時自動轉成自訂快照，不需要先複製。
    pub fn apply(&mut self, key: &str, value: &str) {
        self.field_errors.remove(key);
        if !self.editable() {
            let reason = self.blocked_reason();
            self.fail(key, reason);
            return;
        }
        let mut next = self.settings.clone();
        if self.locked(key) {
            if let Err(err) = make_editable(&mut next, self.goos) {
                self.fail(key, self.explain(&err));
                return;
            }
        }
        if let Err(err) = next.set(key, value, self.goos) {
            self.fail(key, self.explain(&err));
            return;
        }
        match save(&self.settings_path, &next) {
            Ok(()) => {
                self.settings = next;
                self.status = Status::Saved;
                self.load_error = None;
                self.notes.clear();
                self.file_state = FileState::Ok;
            }
            Err(err) => {
                self.fail(key, format!("存檔失敗：{err}"));
            }
        }
    }

    /// 回到某個設定鍵的平台預設值，存檔後交給既有熱重載。
    pub fn reset_to_default(&mut self, key: &str) {
        let value = self.defaults.value(key).unwrap_or_default();
        self.apply(key, &value);
    }

    /// 開始錄製某個快捷鍵欄位；先清掉舊的欄位錯誤。
    pub fn begin_recording(&mut self, key: &str) {
        self.recording = Some(key.to_string());
        self.recording_seen_seq = self.render_seq;
        self.field_errors.remove(key);
    }

    /// 記下這一幀仍看得到錄製欄位；分頁切走後就不會再更新。
    pub fn mark_recording_rendered(&mut self, key: &str) {
        if self.is_recording(key) {
            self.recording_seen_seq = self.render_seq;
        }
    }

    /// 是否正在錄製指定欄位。
    pub fn is_recording(&self, key: &str) -> bool {
        self.recording.as_deref() == Some(key)
    }

    /// 取消錄製；原值不動，Esc 與再按一次「取消錄製」都走這裡。
    pub fn cancel_recording(&mut self) {
        if let Some(key) = self.recording.take() {
            self.field_errors.remove(&key);
        }
    }

    /// 收到錄製中的按鍵組合。
    ///
    /// `candidate` 是 UI 從 `KeyDownEvent` 拼出的 chord 字串，或轉換時的錯誤。
    /// 不合法時只更新欄位錯誤、維持錄製狀態繼續等；合法時走與手動編輯相同的
    /// `apply`／存檔／熱重載路徑，成功才結束錄製。
    pub fn record_chord(&mut self, key: &str, candidate: Result<String, ChordError>) {
        match candidate {
            Ok(text) => match Chord::parse(&text) {
                Ok(chord) => {
                    self.apply(key, &chord.as_str());
                    if !self.field_errors.contains_key(key) {
                        self.recording = None;
                    }
                }
                Err(err) => self.record_error(key, err),
            },
            Err(err) => self.record_error(key, err),
        }
    }

    fn record_error(&mut self, key: &str, err: ChordError) {
        self.field_errors.insert(key.to_string(), err.to_string());
    }

    /// 某個設定鍵最近一次的錯誤；文字欄位會顯示在輸入框下方。
    pub fn field_error(&self, key: &str) -> Option<&str> {
        self.field_errors.get(key).map(String::as_str)
    }

    fn fail(&mut self, key: &str, reason: String) {
        self.field_errors.insert(key.to_string(), reason.clone());
        self.status = Status::Failed(reason);
    }

    /// 套用開／關。
    pub fn apply_bool(&mut self, key: &str, on: bool) {
        self.apply(key, if on { "on" } else { "off" });
    }

    /// 重新檢查資料根目錄。
    pub fn rescan(&mut self) {
        self.data = DataStatus::probe(&self.data_root, &self.settings_path, self.file_state);
    }

    fn blocked_reason(&self) -> String {
        if let Some(error) = &self.load_error {
            format!("設定檔讀取失敗，這次不會寫入：{error}")
        } else {
            SettingsError::ReadOnly.to_string()
        }
    }

    fn explain(&self, err: &SettingsError) -> String {
        match err {
            SettingsError::Set(inner) => inner.to_string(),
            other => other.to_string(),
        }
    }
}

/// 設定檔路徑：給了 `--settings` 就用它，否則用資料根目錄的 `settings.json`。
pub fn settings_path_for(data_root: &Path, settings: Option<PathBuf>) -> PathBuf {
    settings.unwrap_or_else(|| jc_settings::paths::settings_path(data_root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("jc-settings-ui-{tag}-{stamp}"));
            fs::create_dir_all(&path).expect("建立暫存目錄");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn view(dir: &TempDir) -> SettingsView {
        SettingsView::load(&LaunchOptions {
            data_root: dir.0.clone(),
            settings_path: dir.0.join("settings.json"),
        })
    }

    #[test]
    fn test_first_launch_creates_editable_default_file() {
        let dir = TempDir::new("first-launch");
        let path = dir.0.join("settings.json");
        let view = view(&dir);
        assert_eq!(view.file_state, FileState::Ok);
        assert!(path.exists(), "第一次啟動就建立設定檔");
        assert!(view.editable());
        for key in ["toggle-key", "start-mode", "esc-clear", "space-selects"] {
            assert!(!view.locked(key), "{key} 第一次開啟就該可改");
        }
        let report = read(&path, "linux");
        assert_eq!(report.state, FileState::Ok);
        assert!(
            !report.settings.active_profile("linux").builtin,
            "預設檔改用自訂風格，不再需要按複製"
        );
    }

    /// UI 顯示的每個欄位都要等於平台預設值。
    fn assert_matches_defaults(view: &SettingsView) {
        let defaults = Settings::default_for_platform(view.goos);
        for opt in crate::options::FUZZY
            .iter()
            .chain(crate::options::ZH.iter())
            .chain(crate::options::JA.iter())
            .chain(crate::options::INPUT.iter())
        {
            assert_eq!(
                view.value(opt.key),
                defaults.value(opt.key).unwrap_or_default(),
                "{} 的顯示值不是預設值",
                opt.key
            );
            assert!(!view.value(opt.key).is_empty(), "{} 顯示空白", opt.key);
        }
    }

    #[test]
    fn test_missing_file_shows_platform_defaults() {
        let dir = TempDir::new("missing-defaults");
        let path = dir.0.join("settings.json");
        let view = view(&dir);
        assert_matches_defaults(&view);
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+alt+,");
        assert_eq!(view.value("keybind-emoji-palette"), "ctrl+alt+.");

        // 第一次儲存寫出完整預設，不是只寫改過的欄位。
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("讀回預設檔")).expect("解析預設檔");
        let members = json.as_object().expect("設定檔是物件");
        for key in ["selection", "space_selects", "keybinds", "input_profile"] {
            assert!(members.contains_key(key), "第一次儲存缺少 {key}");
        }
        assert!(members.len() >= 40, "第一次儲存只寫了部分欄位");
        assert_eq!(json["keybinds"]["symbol_menu"], "ctrl+alt+,");
        assert_eq!(json["keybinds"]["emoji_palette"], "ctrl+alt+.");
    }

    #[test]
    fn test_empty_object_file_shows_platform_defaults() {
        let dir = TempDir::new("empty-object-defaults");
        fs::write(dir.0.join("settings.json"), b"{}").expect("寫入空物件");
        let view = view(&dir);
        assert_matches_defaults(&view);
    }

    #[test]
    fn test_file_without_keybinds_shows_default_chords() {
        let dir = TempDir::new("no-keybinds-defaults");
        fs::write(
            dir.0.join("settings.json"),
            br#"{"schema":1,"space_selects":false}"#,
        )
        .expect("寫入缺 keybinds 的設定");
        let view = view(&dir);
        assert_matches_defaults(&view);
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+alt+,");
        assert_eq!(view.value("keybind-emoji-palette"), "ctrl+alt+.");
    }

    #[test]
    fn test_empty_keybind_values_show_default_chords() {
        let dir = TempDir::new("empty-keybinds-defaults");
        fs::write(
            dir.0.join("settings.json"),
            br#"{"keybinds":{"symbol_menu":"","emoji_palette":""}}"#,
        )
        .expect("寫入空 chord 的設定");
        let view = view(&dir);
        assert_matches_defaults(&view);
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+alt+,");
        assert_eq!(view.value("keybind-emoji-palette"), "ctrl+alt+.");

        // 空 chord 補回預設後，欄位要能直接改成新的合法 chord。
        let mut view = view;
        view.apply("keybind-symbol-menu", "ctrl+shift+s");
        assert!(matches!(view.status, Status::Saved), "空值修好後應可儲存");
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+shift+s");
        let report = read(&dir.0.join("settings.json"), "linux");
        assert_eq!(report.settings.keybinds.symbol_menu, "ctrl+shift+s");
        assert_eq!(report.settings.keybinds.emoji_palette, "ctrl+alt+.");
    }

    #[test]
    fn test_invalid_chord_sets_field_error_and_reset_clears_it() {
        let dir = TempDir::new("field-error");
        let mut view = view(&dir);

        view.apply("keybind-symbol-menu", "a");
        assert!(matches!(view.status, Status::Failed(_)));
        assert_eq!(
            view.field_error("keybind-symbol-menu"),
            Some("快捷鍵需要 Ctrl 或 Alt（不能是純按鍵）")
        );
        assert_eq!(
            view.value("keybind-symbol-menu"),
            "ctrl+alt+,",
            "不合法 chord 不覆寫目前值"
        );

        view.apply("keybind-symbol-menu", "ctrl+shift+s");
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+shift+s");
        assert!(
            view.field_error("keybind-symbol-menu").is_none(),
            "成功存檔後欄位錯誤要消失"
        );

        view.reset_to_default("keybind-symbol-menu");
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+alt+,");
        assert!(matches!(view.status, Status::Saved));
        let report = read(&dir.0.join("settings.json"), "linux");
        assert_eq!(report.settings.keybinds.symbol_menu, "ctrl+alt+,");
    }

    #[test]
    fn test_record_chord_normalizes_and_saves() {
        let dir = TempDir::new("record-save");
        let mut view = view(&dir);
        view.begin_recording("keybind-symbol-menu");
        assert!(view.is_recording("keybind-symbol-menu"));

        view.record_chord("keybind-symbol-menu", Ok("alt+ctrl+shift+s".to_string()));

        assert!(!view.is_recording("keybind-symbol-menu"), "成功後結束錄製");
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+alt+shift+s");
        assert!(matches!(view.status, Status::Saved));
        assert!(view.field_error("keybind-symbol-menu").is_none());
        let report = read(&dir.0.join("settings.json"), "linux");
        assert_eq!(
            report.settings.keybinds.symbol_menu, "ctrl+alt+shift+s",
            "錄製結果要走同一條存檔路徑"
        );
    }

    #[test]
    fn test_record_without_modifier_keeps_waiting_and_does_not_save() {
        let dir = TempDir::new("record-no-modifier");
        let path = dir.0.join("settings.json");
        let mut view = view(&dir);
        let before = fs::read(&path).expect("第一次啟動已建立設定檔");
        let original = view.value("keybind-symbol-menu");

        view.begin_recording("keybind-symbol-menu");
        view.record_chord("keybind-symbol-menu", Ok("a".to_string()));

        assert!(view.is_recording("keybind-symbol-menu"), "錯誤後繼續等");
        assert_eq!(
            view.field_error("keybind-symbol-menu"),
            Some("快捷鍵需要 Ctrl 或 Alt（不能是純按鍵）")
        );
        assert_eq!(view.value("keybind-symbol-menu"), original);
        assert_eq!(fs::read(&path).expect("讀回設定檔"), before, "不存檔");

        view.record_chord("keybind-symbol-menu", Ok("shift+a".to_string()));
        assert!(view.is_recording("keybind-symbol-menu"));
        assert_eq!(
            view.field_error("keybind-symbol-menu"),
            Some("快捷鍵不能只用 Shift；請加上 Ctrl 或 Alt")
        );

        // 同一次錄製可以接著按合法組合。
        view.record_chord("keybind-symbol-menu", Ok("ctrl+alt+b".to_string()));
        assert!(!view.is_recording("keybind-symbol-menu"));
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+alt+b");
    }

    #[test]
    fn test_record_duplicate_chord_stays_in_recording() {
        let dir = TempDir::new("record-duplicate");
        let mut view = view(&dir);
        view.begin_recording("keybind-symbol-menu");
        view.record_chord("keybind-symbol-menu", Ok("ctrl+alt+.".to_string()));

        assert!(view.is_recording("keybind-symbol-menu"), "重複綁定要繼續等");
        assert_eq!(
            view.field_error("keybind-symbol-menu"),
            Some("同一個快捷鍵 ctrl+alt+. 綁定了多個動作")
        );
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+alt+,");
    }

    #[test]
    fn test_cancel_recording_keeps_original_value() {
        let dir = TempDir::new("record-cancel");
        let path = dir.0.join("settings.json");
        let mut view = view(&dir);
        let before = fs::read(&path).expect("第一次啟動已建立設定檔");
        let original = view.value("keybind-symbol-menu");

        view.begin_recording("keybind-symbol-menu");
        view.record_chord("keybind-symbol-menu", Ok("a".to_string()));
        assert!(view.field_error("keybind-symbol-menu").is_some());
        view.cancel_recording();

        assert!(!view.is_recording("keybind-symbol-menu"));
        assert!(view.field_error("keybind-symbol-menu").is_none());
        assert_eq!(view.value("keybind-symbol-menu"), original, "原值保留");
        assert_eq!(fs::read(&path).expect("讀回設定檔"), before, "取消不寫檔");
    }

    #[test]
    fn test_manual_edit_after_recording_still_saves() {
        let dir = TempDir::new("record-then-manual");
        let mut view = view(&dir);
        view.begin_recording("keybind-symbol-menu");
        view.record_chord("keybind-symbol-menu", Ok("ctrl+alt+b".to_string()));
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+alt+b");

        view.apply("keybind-symbol-menu", "ctrl+shift+s");
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+shift+s");
        let report = read(&dir.0.join("settings.json"), "linux");
        assert_eq!(report.settings.keybinds.symbol_menu, "ctrl+shift+s");
    }

    #[test]
    fn test_record_rejects_platform_modifier_candidate() {
        let dir = TempDir::new("record-platform");
        let mut view = view(&dir);
        view.begin_recording("keybind-symbol-menu");
        view.record_chord("keybind-symbol-menu", Err(ChordError::NoModifier));
        assert!(view.is_recording("keybind-symbol-menu"));
        assert_eq!(
            view.field_error("keybind-symbol-menu"),
            Some("快捷鍵需要 Ctrl 或 Alt（不能是純按鍵）")
        );
        assert_eq!(view.value("keybind-symbol-menu"), "ctrl+alt+,");
    }

    #[test]
    fn test_apply_writes_readable_settings() {
        let dir = TempDir::new("apply");
        let mut view = view(&dir);
        assert_eq!(view.value("esc-clear"), "on");
        view.apply_bool("esc-clear", false);
        assert!(
            matches!(view.status, Status::Saved),
            "第一次開啟就能直接改 esc-clear"
        );
        assert_eq!(view.value("esc-clear"), "off");
        assert!(!view.locked("esc-clear"));
        let report = read(&dir.0.join("settings.json"), "linux");
        assert_eq!(report.state, FileState::Ok);
        assert!(!report.settings.esc_clear, "存檔後重新讀取要一致");
    }

    #[test]
    fn test_locked_builtin_profile_is_customized_on_first_edit() {
        let dir = TempDir::new("locked");
        let path = dir.0.join("settings.json");
        let mut builtin = Settings::default_for_platform("linux");
        builtin
            .profile_action(ProfileAction::Select, "0c", "", "linux")
            .expect("選用內建風格");
        save(&path, &builtin).expect("寫入內建風格設定");
        let mut view = view(&dir);
        assert!(view.locked("esc-clear"), "內建風格原本鎖定");

        view.apply_bool("esc-clear", false);

        assert_eq!(view.value("esc-clear"), "off");
        assert!(!view.locked("esc-clear"), "第一次修改時自動轉成自訂風格");
        let report = read(&path, "linux");
        assert!(!report.settings.esc_clear);
        assert!(!report.settings.active_profile("linux").builtin);
    }

    #[test]
    fn test_apply_choice_persists_value() {
        let dir = TempDir::new("choice");
        let mut view = view(&dir);
        view.apply("start-mode", "english");
        assert_eq!(view.value("start-mode"), "english");
        let report = read(&dir.0.join("settings.json"), "linux");
        assert_eq!(report.settings.start_mode, "english");
    }

    #[test]
    fn test_invalid_value_keeps_settings() {
        let dir = TempDir::new("invalid");
        let path = dir.0.join("settings.json");
        let mut view = view(&dir);
        let before = view.value("start-mode");
        let bytes = fs::read(&path).expect("第一次啟動已建立設定檔");
        view.apply("start-mode", "klingon");
        assert_eq!(view.value("start-mode"), before);
        assert_eq!(
            fs::read(&path).expect("讀回設定檔"),
            bytes,
            "拒絕的值不寫檔"
        );
        assert!(matches!(view.status, Status::Failed(_)));
    }

    #[test]
    fn test_corrupt_file_is_not_overwritten() {
        let dir = TempDir::new("corrupt");
        fs::write(dir.0.join("settings.json"), b"{not json").expect("寫入壞檔");
        let mut view = view(&dir);
        assert!(!view.editable());
        view.apply_bool("esc-clear", false);
        let bytes = fs::read(dir.0.join("settings.json")).expect("讀回壞檔");
        assert_eq!(bytes, b"{not json", "壞檔不可被覆寫");
    }

    #[test]
    fn test_newer_schema_is_read_only() {
        let dir = TempDir::new("newer");
        fs::write(dir.0.join("settings.json"), br#"{"schema":99}"#).expect("寫入新檔");
        let view = view(&dir);
        assert!(view.read_only());
        assert!(!view.editable());
    }
}
