//! jc-ui 升級交接：收到 WM_CLOSE 時不立刻結束；候選窗還顯示著、或剛收到呈現訊息時
//! 先記下，定時再看，閒下來才結束。移植自 0c 的 retire.rs（純 std，無 Win32）。
use std::time::Duration;

/// 最後一則呈現訊息之後要安靜多久才結束。
pub const RETIRE_QUIET: Duration = Duration::from_millis(1500);
/// 延後結束時多久再檢查一次。
pub const RETIRE_POLL_MS: u32 = 500;

/// 收到結束要求時可不可以現在結束。
pub fn ready_to_exit(visible: bool, since_last_present: Option<Duration>) -> bool {
    !visible && since_last_present.is_none_or(|d| d >= RETIRE_QUIET)
}
