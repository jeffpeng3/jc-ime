//! 有時間與來源的使用證據（對照 `core/personal/evidence.go`）。

use serde::{Deserialize, Serialize};

use super::{DAY_SECONDS, UNSEEN};

const SHORT_HALF_LIFE: i64 = 2 * DAY_SECONDS;
const LONG_HALF_LIFE: i64 = 30 * DAY_SECONDS;
const SHORT_CAP: f64 = 8.0;
const LONG_CAP: f64 = 16.0;

/// 同時保存近期重複與跨日習慣。長期證據每個 UTC 日至多增加 1。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageEvidence {
    /// 近期證據。
    pub short: f64,
    /// 長期證據。
    pub long: f64,
    /// 最近更新時間。
    pub updated: i64,
    /// 最近計入的 UTC 日。
    pub day: i64,
    /// 當日已增加的長期權重。
    pub day_weight: f64,
}

/// 指數衰減。
pub(crate) fn decay(value: f64, elapsed: i64, half_life: i64) -> f64 {
    value * 2f64.powf(-(elapsed.max(0) as f64) / half_life as f64)
}

impl UsageEvidence {
    /// 依經過時間衰減，並在跨日時清掉當日額度。
    pub fn at(&self, now: i64) -> Self {
        let mut out = *self;
        let elapsed = (now - out.updated).max(0);
        out.short = decay(out.short, elapsed, SHORT_HALF_LIFE);
        out.long = decay(out.long, elapsed, LONG_HALF_LIFE);
        out.updated = out.updated.max(now);
        let day = now / DAY_SECONDS;
        if day > out.day {
            out.day = day;
            out.day_weight = 0.0;
        }
        out
    }

    /// 加入一筆證據。
    pub fn add(&self, now: i64, weight: f64) -> Self {
        let mut out = self.at(now);
        let weight = weight.max(0.0);
        out.short = (out.short + weight).min(SHORT_CAP);
        let long_weight = weight.min((1.0 - out.day_weight).max(0.0));
        out.long = (out.long + long_weight).min(LONG_CAP);
        out.day_weight += long_weight;
        out
    }

    /// 衰減後的短期＋長期證據。
    pub fn strength(&self, now: i64) -> f64 {
        let at = self.at(now);
        at.short + at.long
    }
}

/// 系統未登錄詞的保底值；保留給 prune 的相容比較。
pub(crate) const _UNSEEN: f64 = UNSEEN;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_halves_after_half_life() {
        let e = UsageEvidence {
            short: 2.0,
            updated: 0,
            ..UsageEvidence::default()
        };
        assert!((e.at(SHORT_HALF_LIFE).short - 1.0).abs() < 1e-9);
    }

    #[test]
    fn daily_long_cap_is_one() {
        let now = 10 * DAY_SECONDS;
        let e = UsageEvidence::default().add(now, 0.75).add(now, 0.75);
        assert!((e.long - 1.0).abs() < 1e-9);
        assert!((e.short - 1.5).abs() < 1e-9);
    }
}
