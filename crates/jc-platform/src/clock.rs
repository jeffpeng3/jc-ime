//! 時間來源與計時器排程。
//!
//! 核心不直接讀系統時鐘：需要時間的地方一律注入 [`Clock`] 或
//! [`Scheduler`]，測試才能確定性地推進時間。

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// 時間來源，單位為毫秒。
///
/// 核心只要求單調不倒退，不要求與 wall clock 對齊；實作可以從開機或
/// 行程啟動開始算。
pub trait Clock {
    /// 目前時間（毫秒）。
    fn now_ms(&self) -> u64;
}

/// 系統時鐘，回傳 Unix epoch 以來的毫秒數。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
    }
}

/// 測試用時鐘，只能透過 [`ManualClock::advance_ms`] 前進。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ManualClock {
    now_ms: u64,
}

impl ManualClock {
    /// 從指定的毫秒數開始。
    pub fn new(now_ms: u64) -> Self {
        Self { now_ms }
    }

    /// 直接把時間設成 `now_ms`；只允許往前，避免測試依賴時間倒退。
    ///
    /// 傳入比目前小的值會被忽略。
    pub fn set_ms(&mut self, now_ms: u64) {
        if now_ms > self.now_ms {
            self.now_ms = now_ms;
        }
    }

    /// 前進 `ms` 毫秒，回傳新的時間。
    pub fn advance_ms(&mut self, ms: u64) -> u64 {
        self.now_ms = self.now_ms.saturating_add(ms);
        self.now_ms
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now_ms
    }
}

/// 計時器識別碼，由排程器發出，在同一個排程器內唯一。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TimerId(u64);

impl TimerId {
    /// 用原始數值建立識別碼；wrapper 從事件還原時使用。
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// 原始數值。
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// 計時器排程。
///
/// 核心排定計時器後不會自己觸發；wrapper 在到期時把
/// [`crate::HostEvent::Timer`] 送回核心。計時器不要求精準，提前或延後
/// 幾毫秒都可以，但同一個 `id` 只能送一次。
pub trait Scheduler: Clock {
    /// 排定 `delay_ms` 之後觸發的計時器，回傳它的識別碼。
    ///
    /// `delay_ms` 為 `0` 代表下一個排程回合，不保證同步觸發。
    fn schedule_after(&mut self, delay_ms: u64) -> TimerId;

    /// 取消尚未觸發的計時器；已經觸發或不存在時回傳 `false`。
    fn cancel(&mut self, id: TimerId) -> bool;
}

/// 測試用排程器：時間只在 [`FakeScheduler::advance_ms`] 時前進。
///
/// 到期的計時器不會自動送出事件，而是由 `advance_ms` 依到期時間回傳，
/// 測試再自行決定要不要當成 [`crate::HostEvent::Timer`] 餵回核心。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeScheduler {
    now_ms: u64,
    next_id: u64,
    pending: BTreeMap<(u64, u64), TimerId>,
}

impl FakeScheduler {
    /// 從指定的毫秒數開始，尚未有任何計時器。
    pub fn new(start_ms: u64) -> Self {
        Self {
            now_ms: start_ms,
            next_id: 1,
            pending: BTreeMap::new(),
        }
    }

    /// 前進 `ms` 毫秒，回傳這段期間到期的計時器。
    ///
    /// 回傳順序是先比到期時間，再比建立順序；同一個計時器只會回傳一次。
    pub fn advance_ms(&mut self, ms: u64) -> Vec<TimerId> {
        self.now_ms = self.now_ms.saturating_add(ms);
        let due: Vec<(u64, u64)> = self
            .pending
            .range(..=(self.now_ms, u64::MAX))
            .map(|(key, _)| *key)
            .collect();
        let mut fired = Vec::with_capacity(due.len());
        for key in due {
            if let Some(id) = self.pending.remove(&key) {
                fired.push(id);
            }
        }
        fired
    }

    /// 尚未到期的計時器數量。
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// 指定的計時器是否還在等待。
    pub fn is_pending(&self, id: TimerId) -> bool {
        self.pending.values().any(|pending| *pending == id)
    }
}

impl Clock for FakeScheduler {
    fn now_ms(&self) -> u64 {
        self.now_ms
    }
}

impl Scheduler for FakeScheduler {
    fn schedule_after(&mut self, delay_ms: u64) -> TimerId {
        let id = TimerId::new(self.next_id);
        self.next_id += 1;
        let due = self.now_ms.saturating_add(delay_ms);
        self.pending.insert((due, id.get()), id);
        id
    }

    fn cancel(&mut self, id: TimerId) -> bool {
        if let Some(key) = self
            .pending
            .iter()
            .find(|(_, pending)| **pending == id)
            .map(|(key, _)| *key)
        {
            self.pending.remove(&key);
            true
        } else {
            false
        }
    }
}
