//! 設定檔熱重載與快捷鍵 chord 對照。
//!
//! 引擎每秒看一次設定檔的 mtime 與大小；變了才重讀。重讀成功會更新共用的
//! keymap／bindings／生效設定，並把 `settings_revision` +1。真正的 session
//! 留在各 worker thread，下一次請求進來時由 [`crate::Engine::sync_settings`]
//! 套用，所以不需要把非 `Send` 的 `Session` 搬過 thread。

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use jc_core::{Key, KeyCode};
use jc_settings::keybinds::Chord;
use jc_settings::settings::FileState;

use crate::config;
use crate::{lock, Registry};

/// 設定檔的變更戳記：修改時間（奈秒）與檔案大小。
pub(crate) type Stamp = (u64, u64);

/// 讀取設定檔戳記；檔案不存在時回 `None`。
pub(crate) fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    Some((modified, metadata.len()))
}

/// 把一個 `Key` 轉成正規化 chord；沒有可對應的按鍵名稱時回 `None`。
pub(crate) fn chord_of_key(key: &Key) -> Option<Chord> {
    let name = match key.code {
        KeyCode::Char(c) => {
            let ch = key.text.unwrap_or(c);
            if ch == ' ' {
                "space".to_string()
            } else if ch.is_ascii_alphabetic() {
                ch.to_ascii_lowercase().to_string()
            } else {
                ch.to_string()
            }
        }
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Escape => "escape".to_string(),
        KeyCode::Space => "space".to_string(),
        KeyCode::ArrowLeft => "left".to_string(),
        KeyCode::ArrowRight => "right".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::Tab | KeyCode::ShiftTab => "tab".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Convert => "convert".to_string(),
        KeyCode::NonConvert => "nonconvert".to_string(),
        KeyCode::F(n) => format!("f{n}"),
    };
    Some(Chord {
        ctrl: key.ctrl,
        alt: key.alt,
        shift: key.shift,
        key: name,
    })
}

/// 從檔案重讀設定並套用到共用 registry。
///
/// 壞檔或讀不到時只記 `settings_error`，不更新 revision，也不換掉目前生效的
/// 設定。成功時回新的 revision。
pub(crate) fn reload(shared: &Arc<Mutex<Registry>>) -> Result<u64, String> {
    let (path, goos, overrides) = {
        let registry = lock(shared);
        (
            registry.settings_path.clone(),
            registry.settings_goos.clone(),
            registry.settings_overrides,
        )
    };
    let Some(path) = path else {
        return Ok(lock(shared).settings_revision);
    };
    let loaded = config::load(Some(&path), None, &goos);
    for warning in &loaded.warnings {
        log::warn!("設定：{warning}");
    }
    if matches!(loaded.state, FileState::Corrupt | FileState::Unreadable) {
        let error = loaded
            .warnings
            .first()
            .cloned()
            .unwrap_or_else(|| format!("無法讀取設定檔：{}", path.display()));
        lock(shared).settings_error = Some(error.clone());
        return Err(error);
    }
    // chord 壞掉時 `config::load` 會退回預設；熱重載不能把這當成成功，
    // 否則使用者的自訂綁定會被默默換掉。保留 registry 舊值並回報錯誤。
    if let Some(err) = loaded.keybind_error {
        let error = format!("keybinds：{err}");
        lock(shared).settings_error = Some(error.clone());
        return Err(error);
    }
    let effective = config::effective(&loaded.settings, &overrides, &goos);
    let keymap = effective
        .keybinds
        .chords()
        .map_err(|err| format!("keybinds：{err}"))?;
    let revision = {
        let mut registry = lock(shared);
        registry.settings_keymap = keymap;
        registry.settings_bindings = effective.bindings.clone();
        registry.settings_effective = Some(effective);
        registry.settings_revision = registry.settings_revision.saturating_add(1);
        registry.settings_error = None;
        registry.settings_stamp = stamp(&path);
        registry.settings_revision
    };
    log::info!("重新載入設定（revision {revision}）：{}", path.display());
    Ok(revision)
}

/// 啟動每秒一次的設定檔 watcher；沒有設定檔路徑時直接返回。
pub(crate) fn spawn(shared: Arc<Mutex<Registry>>) {
    if lock(&shared).settings_path.is_none() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("jc-engine-settings".into())
        .spawn(move || {
            let mut last = lock(&shared).settings_stamp;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let (stopped, path) = {
                    let registry = lock(&shared);
                    (
                        registry.shutdown || registry.drain_done,
                        registry.settings_path.clone(),
                    )
                };
                if stopped {
                    break;
                }
                let Some(path) = path else { continue };
                let current = stamp(&path);
                if current == last {
                    continue;
                }
                // 失敗也記下這次戳記，避免同一個壞檔每秒重印 log；檔案再變才會重試。
                last = current;
                if let Err(err) = reload(&shared) {
                    log::warn!("重新載入設定失敗：{err}");
                }
            }
        });
    if let Err(err) = spawned {
        log::warn!("啟動設定 watcher 失敗：{err}");
    }
}
