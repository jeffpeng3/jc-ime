//! `jc-platform` 的公開契約測試：記錄順序、候選高亮與送出組合、假時鐘。

use jc_platform::{
    Clock, FakeScheduler, HeadlessHost, HostCall, HostError, HostEvent, InputHost, ManualClock,
    RecordingHost, RecordingLog, Scheduler, TimerId,
};

#[test]
fn recording_preserves_call_order() {
    let mut host = HeadlessHost::new();
    host.show_status("學習中").unwrap();
    host.update_preedit("ㄋㄧ", 2).unwrap();
    host.show_candidates(&["你".to_owned(), "尼".to_owned()], 0)
        .unwrap();
    host.commit("你").unwrap();
    host.clear_preedit().unwrap();

    assert_eq!(
        host.calls(),
        &[
            HostCall::Status {
                text: "學習中".to_owned()
            },
            HostCall::Preedit {
                text: "ㄋㄧ".to_owned(),
                cursor_utf16: 2
            },
            HostCall::Candidates {
                items: vec!["你".to_owned(), "尼".to_owned()],
                highlight: 0
            },
            HostCall::Commit {
                text: "你".to_owned()
            },
            HostCall::ClearPreedit,
        ]
    );
}

#[test]
fn candidate_highlight_pairs_with_commit() {
    let mut host = HeadlessHost::new();
    host.update_preedit("ㄋㄧˇ", 3).unwrap();
    host.show_candidates(&["你".to_owned(), "妳".to_owned(), "擬".to_owned()], 1)
        .unwrap();
    host.commit("妳").unwrap();

    let calls = host.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[1],
        HostCall::Candidates {
            items: vec!["你".to_owned(), "妳".to_owned(), "擬".to_owned()],
            highlight: 1,
        }
    );
    assert_eq!(
        calls[2],
        HostCall::Commit {
            text: "妳".to_owned()
        }
    );
    match &calls[1] {
        HostCall::Candidates { items, highlight } => {
            assert_eq!(items[*highlight], "妳");
        }
        other => panic!("第二筆應該顯示候選，實際是 {other:?}"),
    }
}

#[test]
fn recording_serializes_to_stable_json() {
    let mut host = HeadlessHost::new();
    host.update_preedit("ㄋㄧ", 3).unwrap();
    host.show_candidates(&["你".to_owned(), "妳".to_owned()], 1)
        .unwrap();
    host.commit("妳").unwrap();

    let json = host.to_json().unwrap();
    assert_eq!(
        json,
        r#"{"calls":[{"call":"preedit","text":"ㄋㄧ","cursor_utf16":3},{"call":"candidates","items":["你","妳"],"highlight":1},{"call":"commit","text":"妳"}]}"#
    );

    let parsed: RecordingLog = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, host.log());
    assert_eq!(parsed.calls, host.calls());
}

#[test]
fn recording_host_keeps_failed_calls_and_forwards_errors() {
    struct FailingHost;

    impl InputHost for FailingHost {
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
            Err(HostError::rejected("候選窗被系統封鎖"))
        }

        fn commit(&mut self, _text: &str) -> Result<(), HostError> {
            Err(HostError::unavailable("視窗已關閉"))
        }

        fn show_status(&mut self, _text: &str) -> Result<(), HostError> {
            Ok(())
        }
    }

    let mut host = RecordingHost::with_inner(FailingHost);
    assert_eq!(
        host.show_candidates(&["你".to_owned()], 0),
        Err(HostError::rejected("候選窗被系統封鎖"))
    );
    assert_eq!(host.commit("你"), Err(HostError::unavailable("視窗已關閉")));
    assert_eq!(
        host.calls(),
        &[
            HostCall::Candidates {
                items: vec!["你".to_owned()],
                highlight: 0,
            },
            HostCall::Commit {
                text: "你".to_owned()
            },
        ]
    );
}

#[test]
fn manual_clock_advances_forward_only() {
    let mut clock = ManualClock::new(1000);
    assert_eq!(clock.now_ms(), 1000);
    assert_eq!(clock.advance_ms(250), 1250);
    assert_eq!(clock.now_ms(), 1250);

    clock.set_ms(1200);
    assert_eq!(clock.now_ms(), 1250);
    clock.set_ms(2000);
    assert_eq!(clock.now_ms(), 2000);

    assert_eq!(clock.advance_ms(u64::MAX), u64::MAX);
}

#[test]
fn fake_scheduler_fires_due_timers_in_time_order() {
    let mut scheduler = FakeScheduler::new(100);
    let first = scheduler.schedule_after(50);
    let second = scheduler.schedule_after(20);
    let third = scheduler.schedule_after(0);
    assert_eq!(first, TimerId::new(1));
    assert_eq!(second, TimerId::new(2));
    assert_eq!(third, TimerId::new(3));
    assert_eq!(scheduler.pending_count(), 3);

    // 第三個在 100ms、第二個在 120ms 到期，依到期時間順序送出。
    assert_eq!(scheduler.advance_ms(49), vec![third, second]);
    assert_eq!(scheduler.now_ms(), 149);
    assert_eq!(scheduler.pending_count(), 1);

    // 第一個在 150ms 到期。
    assert_eq!(scheduler.advance_ms(1), vec![first]);
    assert_eq!(scheduler.now_ms(), 150);
    assert_eq!(scheduler.pending_count(), 0);
    assert!(!scheduler.is_pending(first));

    assert!(scheduler.advance_ms(1000).is_empty());
}

#[test]
fn cancelled_timer_never_fires() {
    let mut scheduler = FakeScheduler::new(0);
    let keep = scheduler.schedule_after(10);
    let drop = scheduler.schedule_after(5);
    assert_eq!(scheduler.schedule_after(20), TimerId::new(3));

    assert!(scheduler.cancel(drop));
    assert!(!scheduler.cancel(drop));
    assert!(!scheduler.cancel(TimerId::new(99)));

    assert_eq!(scheduler.advance_ms(10), vec![keep]);
    assert_eq!(scheduler.pending_count(), 1);
}

#[test]
fn host_events_round_trip_through_json() {
    let events = [
        HostEvent::FocusLost,
        HostEvent::FocusGained,
        HostEvent::CandidateClicked { index: 2 },
        HostEvent::Scroll { delta: -3 },
        HostEvent::Timer {
            id: TimerId::new(7),
        },
    ];
    for event in events {
        let json = serde_json::to_string(&event).unwrap();
        let decoded: HostEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, event);
    }
}
