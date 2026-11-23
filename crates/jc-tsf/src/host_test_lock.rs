//! Windows host 測試的序列化鎖。
//!
//! `privacy` 的 `FIELD_CACHE`／`EPOCH` 是行程全域，`focus` 測試則會建立真正的訊息視窗與
//! TSF 物件；平行執行時彼此會干擾（另一個測試 `invalidate()` 讓 cache 命中失效、同時建立
//! 視窗讓 callback 建失敗）。這些測試共用同一把鎖，一次只跑一支。
//!
//! 只給測試用，且只在 Windows 編譯；Linux 的純邏輯測試不受影響。

use std::sync::{Mutex, MutexGuard};

static HOST_TEST_LOCK: Mutex<()> = Mutex::new(());

/// 取得 host 測試的序列化鎖。前一支測試 panic 留下的 poisoning 直接吞掉，
/// 不讓一個測試失敗連帶弄紅其他測試。
pub(crate) fn serial() -> MutexGuard<'static, ()> {
    HOST_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
