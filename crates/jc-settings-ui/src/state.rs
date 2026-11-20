//! 設定程式的狀態：讀取、修改、存檔。
//!
//! 所有讀寫都經過 `jc-settings`：`settings::read` 負責缺檔與壞檔，
//! `Settings::set` 負責驗證，`settings::save` 先寫暫存檔再改名。UI 只拿這裡的
//! 值，不自己拼 JSON。

use std::path::{Path, PathBuf};

use jc_settings::{read, save, FileState, ProfileAction, Settings, SettingsError};

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
            Status::Saved => Some("已儲存；引擎下次啟動才生效。".to_string()),
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

impl SettingsView {
    /// 讀取設定檔並建立狀態。
    pub fn load(options: &LaunchOptions) -> Self {
        let goos = current_goos();
        let report = read(&options.settings_path, goos);
        let load_error = report.error.as_ref().map(ToString::to_string);
        let data = DataStatus::probe(&options.data_root, &report.path, report.state);
        Self {
            data_root: options.data_root.clone(),
            settings_path: report.path.clone(),
            goos,
            defaults: Settings::default_for_platform(goos),
            settings: report.settings,
            file_state: report.state,
            load_error,
            notes: report.notes,
            status: Status::Idle,
            data,
        }
    }

    /// 現在能不能寫入設定檔。
    pub fn editable(&self) -> bool {
        self.load_error.is_none() && !self.settings.read_only()
    }

    /// 讀取一個設定鍵目前的值。
    pub fn value(&self, key: &str) -> String {
        self.settings.value(key).unwrap_or_default()
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
    pub fn apply(&mut self, key: &str, value: &str) {
        if !self.editable() {
            self.status = Status::Failed(self.blocked_reason());
            return;
        }
        let mut next = self.settings.clone();
        if let Err(err) = next.set(key, value, self.goos) {
            self.status = Status::Failed(self.explain(&err));
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
                self.status = Status::Failed(format!("存檔失敗：{err}"));
            }
        }
    }

    /// 套用開／關。
    pub fn apply_bool(&mut self, key: &str, on: bool) {
        self.apply(key, if on { "on" } else { "off" });
    }

    /// 把內建的輸入習慣複製成自訂風格，讓風格管理的項目可以修改。
    pub fn copy_profile(&mut self) {
        if !self.editable() {
            self.status = Status::Failed(self.blocked_reason());
            return;
        }
        let active = self.settings.active_profile(self.goos);
        let name = format!("{}（自訂）", active.name);
        let mut next = self.settings.clone();
        if let Err(err) = next.profile_action(ProfileAction::Copy, &active.id, &name, self.goos) {
            self.status = Status::Failed(self.explain(&err));
            return;
        }
        match save(&self.settings_path, &next) {
            Ok(()) => {
                self.settings = next;
                self.status = Status::Saved;
            }
            Err(err) => self.status = Status::Failed(format!("存檔失敗：{err}")),
        }
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
    fn test_apply_writes_readable_settings() {
        let dir = TempDir::new("apply");
        let mut view = view(&dir);
        view.apply_bool("esc-clear", false);
        assert_eq!(view.value("esc-clear"), "off");
        let report = read(&dir.0.join("settings.json"), "linux");
        assert_eq!(report.state, FileState::Ok);
        assert!(!report.settings.esc_clear, "存檔後重新讀取要一致");
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
        let mut view = view(&dir);
        let before = view.value("start-mode");
        view.apply("start-mode", "klingon");
        assert_eq!(view.value("start-mode"), before);
        assert!(!dir.0.join("settings.json").exists(), "拒絕的值不寫檔");
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
