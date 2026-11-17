use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use scrutiny_embedded_rust::ipc::{IpcMessage, StdAtomicFlag};

const MAX_ITERATIONS: u32 = 10_000_000;
const TIMEOUT: Duration = Duration::from_secs(5);

// Several words so that a torn read is detected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Payload {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

fn make_payload(value: u32) -> Payload {
    Payload { a: value, b: !value, c: value.wrapping_mul(3), d: value ^ 0xA5A5_A5A5 }
}

struct SideResult {
    error_at_iter: Option<u32>,
    sent_count: u32,
}

// Pops from `rx` and sends to `tx` until `exit` is set. Returns the first out-of-order value seen.
fn exchange(
    rx: &IpcMessage<Payload, StdAtomicFlag>,
    tx: &IpcMessage<Payload, StdAtomicFlag>,
    exit: &AtomicBool,
    is_main: bool,
) -> SideResult {
    let start = Instant::now();
    let mut expected_value: u32 = 0;
    let mut my_value: u32 = 0;
    let mut error_at_iter = None;

    while !exit.load(Ordering::Relaxed) {
        if let Some(received) = rx.pop() {
            if received != make_payload(expected_value) {
                error_at_iter = Some(my_value);
                exit.store(true, Ordering::Relaxed);
            }
            expected_value += 1;
        }

        if !tx.has_content() {
            tx.send(make_payload(my_value));
            my_value += 1;
        }

        if is_main && (my_value >= MAX_ITERATIONS || start.elapsed() > TIMEOUT) {
            exit.store(true, Ordering::Relaxed);
        }
    }

    SideResult { error_at_iter, sent_count: my_value }
}

// Mirrors C++ TestIPC.CheckWithThread: two threads exchange sequential values in both directions.
#[test]
fn ipc_check_with_thread() {
    let msg_to_thread = IpcMessage::new_std(make_payload(0));
    let msg_from_thread = IpcMessage::new_std(make_payload(0));
    let exit = AtomicBool::new(false);

    let (main_result, thread_result) = std::thread::scope(|s| {
        let handle = s.spawn(|| exchange(&msg_to_thread, &msg_from_thread, &exit, false));
        let main_result = exchange(&msg_from_thread, &msg_to_thread, &exit, true);
        let thread_result = handle.join();
        (main_result, thread_result)
    });

    let Ok(thread_result) = thread_result else {
        panic!("IPC thread panicked");
    };
    assert_eq!(main_result.error_at_iter, None, "Error in main thread");
    assert_eq!(thread_result.error_at_iter, None, "Error in worker thread");
    assert!(main_result.sent_count >= 1000);
    assert!(thread_result.sent_count >= 1000);
}
