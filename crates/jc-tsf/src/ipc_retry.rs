//! 單次 I/O 逾時與整體期限的重試政策。
//!
//! Windows 的 overlapped I/O 在 `ipc.rs`；這裡只放不依賴 Win32 的決策：
//! 單次逾時只代表這一小段時間沒有進展，對方還活著就繼續在同一個請求上等，
//! 整體期限用完才放棄。Linux 也能直接測這幾條。

use std::time::{Duration, Instant};

/// 一次嘗試的結果。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step<T> {
    /// 完成，附帶完成的位元組數或值。
    Done(T),
    /// 單次等待逾時，對方可能還活著。
    Timeout,
    /// 無法重試的錯誤。
    Failed,
}

/// 下一次單次等待的時間：不超過 `io_timeout`，也不超過整體期限。
pub(crate) fn next_wait(deadline: Instant, io_timeout: Duration) -> Option<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return None;
    }
    Some(remaining.min(io_timeout).max(Duration::from_millis(1)))
}

/// 在整體期限內重試 `attempt`，直到完成、失敗或期限用完。
///
/// 單次逾時時先問 `peer_alive`：對方還活著才繼續，關掉就立刻放棄。
pub(crate) fn run<T>(
    deadline: Instant,
    io_timeout: Duration,
    peer_alive: &dyn Fn() -> bool,
    mut attempt: impl FnMut(Duration) -> Step<T>,
) -> Option<T> {
    loop {
        let wait = next_wait(deadline, io_timeout)?;
        match attempt(wait) {
            Step::Done(value) => return Some(value),
            Step::Timeout => {
                if !peer_alive() {
                    return None;
                }
            }
            Step::Failed => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn deadline_in(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    #[test]
    fn timeout_with_a_live_peer_retries_until_it_succeeds() {
        let calls = Cell::new(0);
        let result = run(deadline_in(50), Duration::from_millis(1), &|| true, |_| {
            let n = calls.get() + 1;
            calls.set(n);
            if n < 3 {
                Step::Timeout
            } else {
                Step::Done(n)
            }
        });
        assert_eq!(result, Some(3), "總預算內要重試到成功");
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn timeout_with_a_closed_peer_stops_immediately() {
        let calls = Cell::new(0);
        let result: Option<u32> = run(deadline_in(50), Duration::from_millis(1), &|| false, |_| {
            calls.set(calls.get() + 1);
            Step::Timeout
        });
        assert_eq!(result, None);
        assert_eq!(calls.get(), 1, "對方關閉後不再重試");
    }

    #[test]
    fn overall_deadline_stops_the_retry_loop() {
        let start = Instant::now();
        let result: Option<u32> = run(
            start + Duration::from_millis(5),
            Duration::from_millis(1),
            &|| true,
            |_| {
                std::thread::sleep(Duration::from_millis(1));
                Step::Timeout
            },
        );
        assert_eq!(result, None);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn hard_error_stops_immediately() {
        let calls = Cell::new(0);
        let result: Option<u32> = run(deadline_in(50), Duration::from_millis(1), &|| true, |_| {
            calls.set(calls.get() + 1);
            Step::Failed
        });
        assert_eq!(result, None);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn wait_never_exceeds_the_single_io_timeout_or_the_deadline() {
        let io = Duration::from_millis(1000);
        assert_eq!(next_wait(deadline_in(10_000), io), Some(io));
        assert!(next_wait(deadline_in(3), io).is_some_and(|wait| wait <= io));
        assert_eq!(
            next_wait(Instant::now() - Duration::from_millis(1), io),
            None
        );
    }
}
