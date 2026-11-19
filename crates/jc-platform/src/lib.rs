//! 平台抽象：核心與 wrapper 的共同契約。
//!
//! 核心只透過這裡的型別與外界互動，作業系統整合（視窗、輸入法上下文、
//! 訊息迴圈）全部由 wrapper 實作。這個 crate 不引入任何 OS 型別，Windows
//! TSF 與 Fcitx5 wrapper 共用同一份介面。
//!
//! - [`InputHost`]：核心推給 wrapper 的畫面更新與送出。
//! - [`HostEvent`]：wrapper 回送給核心的使用者事件。
//! - [`HeadlessHost`]：不接 UI、只記錄呼叫的宿主，供測試與無畫面流程使用。
//! - [`Clock`]／[`Scheduler`]：時間來源與計時器，測試用 [`ManualClock`]、
//!   [`FakeScheduler`] 推進。
//!
//! 完整契約見 `docs/port/jc-platform.md`。

mod clock;
mod headless;
mod host;

pub use clock::{Clock, FakeScheduler, ManualClock, Scheduler, SystemClock, TimerId};
pub use headless::{HeadlessHost, NullHost, RecordingHost, RecordingLog};
pub use host::{HostCall, HostError, HostEvent, InputHost};

#[cfg(test)]
mod tests {
    use super::*;

    struct Host;

    impl InputHost for Host {
        fn update_preedit(&mut self, _text: &str, _cursor_utf16: usize) -> Result<(), HostError> {
            Ok(())
        }

        fn clear_preedit(&mut self) -> Result<(), HostError> {
            Ok(())
        }

        fn show_candidates(
            &mut self,
            _items: &[String],
            _highlight: usize,
        ) -> Result<(), HostError> {
            Ok(())
        }

        fn commit(&mut self, _text: &str) -> Result<(), HostError> {
            Ok(())
        }

        fn show_status(&mut self, _text: &str) -> Result<(), HostError> {
            Ok(())
        }
    }

    #[test]
    fn a_host_can_be_constructed() {
        let _ = Host;
    }
}
