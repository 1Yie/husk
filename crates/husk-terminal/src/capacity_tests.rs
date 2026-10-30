use super::{lock, CapacityWait, Session, TerminalState, MAX_TERMINALS_PER_WINDOW};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

fn full_window() -> TerminalState {
    let state = TerminalState::default();
    for index in 0..MAX_TERMINALS_PER_WINDOW {
        let session = Arc::new(Session::new("window".into()));
        // Model reserved slots without spawning any process. Mark them reaped
        // so cleanup never waits for a fake child.
        session.finish();
        lock(&state.registry)
            .sessions
            .insert(format!("held-{index}"), session);
    }
    state
}

#[test]
fn no_wait_stays_immediate_and_bounded_wait_does_not_overbook() {
    let state = full_window();
    assert!(state
        .reserve("window", None)
        .err()
        .unwrap()
        .contains("maximum 8"));
    let cancelled = AtomicBool::new(false);
    let start = Instant::now();
    let result = state.reserve(
        "window",
        Some(CapacityWait {
            cancelled: &cancelled,
            timeout: Duration::from_millis(20),
        }),
    );
    assert!(result.err().unwrap().contains("maximum 8"));
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(
        lock(&state.registry).sessions.len(),
        MAX_TERMINALS_PER_WINDOW
    );
    let other = state.reserve("other-window", None).unwrap();
    other.session.finish();
    assert_eq!(other.session.owner, "other-window");
    state.shutdown();
}

#[test]
fn capacity_release_wakes_a_waiter_without_relaxing_the_limit() {
    let state = full_window();
    let waiting = state.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let cancelled = AtomicBool::new(false);
        entered_tx.send(()).unwrap();
        let reservation = waiting.reserve(
            "window",
            Some(CapacityWait {
                cancelled: &cancelled,
                timeout: Duration::from_secs(5),
            }),
        );
        assert!(result_tx.send(reservation).is_ok());
    });
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(result_rx.recv_timeout(Duration::from_millis(20)).is_err());
    lock(&state.registry).sessions.remove("held-0");
    state.capacity_changed.notify_all();
    let reservation = result_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    assert_eq!(reservation.session.owner, "window");
    assert_eq!(
        lock(&state.registry).sessions.len(),
        MAX_TERMINALS_PER_WINDOW
    );
    reservation.session.finish();
    state.shutdown();
}

#[test]
fn cancellation_interrupts_capacity_wait_and_prevents_a_late_reservation() {
    let state = full_window();
    let waiting = state.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancelled.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        let result = waiting.reserve(
            "window",
            Some(CapacityWait {
                cancelled: &worker_cancel,
                timeout: Duration::from_secs(5),
            }),
        );
        assert!(result_tx.send(result).is_ok());
    });
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(result_rx.recv_timeout(Duration::from_millis(20)).is_err());
    cancelled.store(true, Ordering::Release);
    assert!(result_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .err()
        .unwrap()
        .contains("cancelled"));
    worker.join().unwrap();
    assert_eq!(
        lock(&state.registry).sessions.len(),
        MAX_TERMINALS_PER_WINDOW
    );
    // Even an immediately available slot must not revive a cancelled request.
    assert!(state
        .reserve(
            "other-window",
            Some(CapacityWait {
                cancelled: &cancelled,
                timeout: Duration::from_secs(1)
            }),
        )
        .err()
        .unwrap()
        .contains("cancelled"));
    state.shutdown();
}

#[test]
fn retiring_an_owner_or_shutting_down_wakes_waiters_without_spawning() {
    for shutdown in [false, true] {
        let state = full_window();
        let waiting = state.clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let cancelled = AtomicBool::new(false);
            entered_tx.send(()).unwrap();
            assert!(result_tx
                .send(waiting.reserve(
                    "window",
                    Some(CapacityWait {
                        cancelled: &cancelled,
                        timeout: Duration::from_secs(5)
                    }),
                ))
                .is_ok());
        });
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(result_rx.recv_timeout(Duration::from_millis(20)).is_err());
        if shutdown {
            state.shutdown();
        } else {
            state.destroy_owner("window");
        }
        let result = result_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(result.err().unwrap().contains("closing"));
        worker.join().unwrap();
        state.shutdown();
    }
}
