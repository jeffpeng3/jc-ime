//! 引擎指令與設定熱套用。
//!
//! 快捷鍵由引擎解析成 action 後呼叫 [`Session::command`]；外殼不自行決定要做
//! 什麼。設定熱重載則走 [`Session::apply_config`]，只換設定與衍生狀態，不清
//! 目前組字。

use std::sync::Arc;

use crate::session::{Session, SessionConfig};

impl Session {
    /// 執行一個具名指令；有開啟或執行時回 `true`。
    ///
    /// 未知 action 回 `false`，呼叫端據此決定是否放行。指令只影響選單狀態，
    /// 不動目前組字。
    pub fn command(&mut self, action: &str) -> bool {
        match action {
            "symbol_menu" => {
                self.open_symbol_menu();
                true
            }
            "emoji_palette" => {
                self.open_palette();
                true
            }
            _ => false,
        }
    }

    /// 目前設定的快照；熱重載用它在 worker thread 內重建下一份設定。
    ///
    /// 回傳值可能含 `Rc`，只能在建立 session 的同一條 thread 使用。
    pub fn config_snapshot(&self) -> SessionConfig {
        self.config.clone()
    }

    /// 熱套用一份新設定，保留目前組字與個人學習資料。
    ///
    /// 資料依賴（詞典、語言模型、個人 store）沿用目前 session；只換由設定檔
    /// 決定的欄位與輸入風格。日文逐字変換控制器在開關切換時才重建，避免每次
    /// 重載都把進行中的延遲轉換丟掉。
    pub fn apply_config(&mut self, config: SessionConfig) {
        let personal = self.personal.clone();
        let base_lexicon = Arc::clone(&self.base_lexicon);
        let live_was_enabled = self.live.is_some();

        let mut next = config;
        next.personal = personal.clone();
        next.zh_lexicon = Some(Arc::clone(&base_lexicon));
        self.config = next;
        self.style = self.config.style.clone();
        self.lexicon = personal.augment_lexicon(&base_lexicon);
        // 聲調閘門的快取（toneless／錯誤提示／模糊音）跟著新設定重建。
        self.install_tone_gate();

        let live_enabled = self.config.ja_live.enabled;
        if live_was_enabled && !live_enabled {
            self.live = None;
            self.live_plan = None;
        } else if !live_was_enabled && live_enabled {
            self.live = self.config.ja.as_ref().map(|decoder| {
                let mut live_config = self.config.ja_live.clone();
                live_config.neural = crate::session::neural::live_gate(&self.config.ja_neural);
                crate::ja::live::Controller::new(decoder.as_ref().clone(), live_config)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Key;

    #[test]
    fn symbol_menu_command_opens_the_menu() {
        let mut session = Session::new(SessionConfig::default());
        assert!(session.command("symbol_menu"));
        assert!(session.menu_tab().is_some());
    }

    #[test]
    fn emoji_palette_command_opens_the_palette() {
        let mut session = Session::new(SessionConfig::default());
        assert!(session.command("emoji_palette"));
        assert!(session.menu_tab().is_some());
    }

    #[test]
    fn unknown_command_is_not_handled() {
        let mut session = Session::new(SessionConfig::default());
        assert!(!session.command("no_such_action"));
        assert!(session.menu_tab().is_none());
    }

    #[test]
    fn apply_config_keeps_the_current_composition() {
        let mut session = Session::new(SessionConfig::default());
        session.press(Key::char('ㄇ', false));
        session.press(Key::char('ㄚ', false));
        assert_eq!(session.preedit(), "ㄇㄚ");

        let config = SessionConfig {
            toneless: true,
            punct_mark: Some('，'),
            ..SessionConfig::default()
        };
        session.apply_config(config);

        assert_eq!(session.preedit(), "ㄇㄚ", "熱重載不得清掉組字");
    }
}
