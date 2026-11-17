#![cfg(feature = "datalogging")]

// End-to-end acquisition with the loop running in its own thread while the main handler
// talks to the server on the test thread.

use scrutiny_embedded_rust::config::Config;
use scrutiny_embedded_rust::datalogging::State as DlState;
use scrutiny_embedded_rust::loop_handler::{FixedFreqLoopHandler, LoopHandler};
use scrutiny_embedded_rust::main_handler::MainHandler;
use scrutiny_embedded_rust::protocol::{CommControlSubfn, CommandId, DataLogControlSubfn, ResponseCode, CONNECT_MAGIC};
use scrutiny_embedded_rust::tools::crc32;
use scrutiny_embedded_rust::types::VariableType;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const DATALOG_BUFFER_SIZE: usize = 1024;
const LOOP_TIMESTEP: u32 = 100;
const CONFIG_ID: u16 = 0x2468;
const PROBE_LOCATION: u8 = 128;
const TRIGGER_MARGIN: u32 = 200;
const ENTRY_SIZE: usize = 8; // Time (u32) + counter (u32)
const WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_PERIOD: Duration = Duration::from_micros(50);

static LOOP: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", LOOP_TIMESTEP);
static LOOPS: [&LoopHandler; 1] = [LOOP.handler()];
static mut DL_BUF: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];

// Written by the loop thread only, logged and used as the first trigger operand.
static COUNTER: AtomicU32 = AtomicU32::new(0);
// Second trigger operand, written by the main thread once the trigger is armed.
static THRESHOLD: AtomicU32 = AtomicU32::new(u32::MAX);
static STOP: AtomicBool = AtomicBool::new(false);

// Stops and joins the loop thread when dropped, so the thread never outlives the MainHandler,
// even when an assertion fails.
struct LoopThread {
    handle: Option<JoinHandle<()>>,
}

impl LoopThread {
    fn start() -> Self {
        STOP.store(false, Ordering::SeqCst);
        let handle = std::thread::spawn(|| {
            while !STOP.load(Ordering::SeqCst) {
                COUNTER.fetch_add(1, Ordering::SeqCst);
                LOOP.process();
                std::thread::sleep(Duration::from_micros(20));
            }
        });
        Self { handle: Some(handle) }
    }

    fn stop(&mut self) -> bool {
        STOP.store(true, Ordering::SeqCst);
        match self.handle.take() {
            Some(handle) => handle.join().is_ok(),
            None => true,
        }
    }
}

impl Drop for LoopThread {
    fn drop(&mut self) {
        self.stop();
    }
}

fn make_request(cmd: u8, subfn: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = vec![cmd, subfn, (len >> 8) as u8, (len & 0xff) as u8];
    frame.extend_from_slice(payload);
    let c = crc32(&frame, 0);
    frame.extend_from_slice(&[(c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8]);
    frame
}

fn pop_response(h: &mut MainHandler<'_, '_>) -> Vec<u8> {
    let n = h.data_to_send() as usize;
    let mut buf = vec![0u8; n];
    h.pop_data(&mut buf);
    buf
}

// Sends a request and processes until the response is ready. A ProcessAgain answer (release of the
// datalogger by the loop thread) is retried by the next process() call. Bounded by WAIT_TIMEOUT.
fn send_and_wait(h: &mut MainHandler<'_, '_>, req: &[u8]) -> Vec<u8> {
    h.receive_data(req);
    let start = Instant::now();
    loop {
        h.process(0);
        if h.data_to_send() > 0 {
            break;
        }
        assert!(
            start.elapsed() < WAIT_TIMEOUT,
            "No response to command {:#04x}/{:#04x}",
            req[0],
            req[1]
        );
        std::thread::sleep(POLL_PERIOD);
    }
    let resp = pop_response(h);
    h.process(0);
    assert_eq!(resp[0], req[0] | 0x80);
    assert_eq!(resp[1], req[1]);
    resp
}

fn send_datalog_request(h: &mut MainHandler<'_, '_>, subfn: DataLogControlSubfn, payload: &[u8]) -> Vec<u8> {
    let req = make_request(CommandId::DataLogControl as u8, subfn as u8, payload);
    send_and_wait(h, &req)
}

fn connect(h: &mut MainHandler<'_, '_>) {
    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Connect as u8,
        &CONNECT_MAGIC,
    );
    let resp = send_and_wait(h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
}

fn resp_code(resp: &[u8]) -> u8 {
    resp[2]
}

fn resp_payload(resp: &[u8]) -> &[u8] {
    let len = u16::from_be_bytes([resp[3], resp[4]]) as usize;
    &resp[5..5 + len]
}

fn encode_usize_be(val: usize) -> Vec<u8> {
    let size = core::mem::size_of::<usize>();
    val.to_be_bytes()[8 - size..].to_vec()
}

// Trigger when COUNTER > THRESHOLD on loop 0, log the loop time and COUNTER.
fn encode_threshold_configure_payload() -> Vec<u8> {
    let mut buf = vec![
        0, // loop_id
    ];
    buf.extend_from_slice(&CONFIG_ID.to_be_bytes());
    buf.extend_from_slice(&1u16.to_be_bytes()); // decimation
    buf.push(PROBE_LOCATION);
    buf.extend_from_slice(&0u32.to_be_bytes()); // timeout
    buf.push(5); // GreaterThan
    buf.extend_from_slice(&0u32.to_be_bytes()); // hold time
    buf.push(2); // operand count
    for addr in [COUNTER.as_ptr() as usize, THRESHOLD.as_ptr() as usize] {
        buf.push(1); // Var operand
        buf.push(VariableType::Uint32 as u8);
        buf.extend_from_slice(&encode_usize_be(addr));
    }
    buf.push(2); // signal count
    buf.push(2); // Time signal
    buf.push(0); // Memory signal
    buf.extend_from_slice(&encode_usize_be(COUNTER.as_ptr() as usize));
    buf.push(4);
    buf
}

fn get_status_state(h: &mut MainHandler<'_, '_>) -> u8 {
    let resp = send_datalog_request(h, DataLogControlSubfn::GetStatus, &[]);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    resp_payload(&resp)[0]
}

fn wait_for_state(h: &mut MainHandler<'_, '_>, state: DlState) {
    let start = Instant::now();
    loop {
        if get_status_state(h) == state as u8 {
            return;
        }
        assert!(
            start.elapsed() < WAIT_TIMEOUT,
            "Datalogger did not reach state {state:?}"
        );
        std::thread::sleep(POLL_PERIOD);
    }
}

fn wait_for_ownership_taken(h: &mut MainHandler<'_, '_>) {
    let start = Instant::now();
    loop {
        h.process(0);
        if h.datalogging_ownership_taken() {
            return;
        }
        assert!(
            start.elapsed() < WAIT_TIMEOUT,
            "Loop did not take ownership of the datalogger"
        );
        std::thread::sleep(POLL_PERIOD);
    }
}

// Reads every ReadAcquisition chunk. Checks the rolling counter, the acquisition ID and the final CRC.
fn read_all_acquisition(h: &mut MainHandler<'_, '_>, acquisition_id: u16) -> Vec<u8> {
    let mut all_data: Vec<u8> = Vec::new();
    for i in 0..200usize {
        let resp = send_datalog_request(h, DataLogControlSubfn::ReadAcquisition, &[]);
        assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "chunk {i}");
        let p = resp_payload(&resp);
        assert!(p.len() >= 4, "chunk {i}: payload too short");
        assert_eq!(p[1], (i % 256) as u8, "rolling counter mismatch at chunk {i}");
        assert_eq!(
            u16::from_be_bytes([p[2], p[3]]),
            acquisition_id,
            "acquisition ID mismatch at chunk {i}"
        );
        let finished = p[0] != 0;
        if finished {
            assert!(p.len() >= 8, "chunk {i}: last payload too short");
            let data_end = p.len() - 4;
            all_data.extend_from_slice(&p[4..data_end]);
            let stored_crc = u32::from_be_bytes([p[data_end], p[data_end + 1], p[data_end + 2], p[data_end + 3]]);
            assert_eq!(stored_crc, crc32(&all_data, 0), "final CRC mismatch");
            return all_data;
        }
        all_data.extend_from_slice(&p[4..]);
    }
    panic!("ReadAcquisition did not finish within 200 chunks");
}

// Returns (time, counter) for each entry. Time is big endian, the counter is a raw memory copy.
fn decode_entries(data: &[u8]) -> Vec<(u32, u32)> {
    assert_eq!(data.len() % ENTRY_SIZE, 0, "data must hold whole entries");
    let mut entries = Vec::new();
    for start in (0..data.len()).step_by(ENTRY_SIZE) {
        let entry = &data[start..start + ENTRY_SIZE];
        let time = u32::from_be_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let counter = u32::from_ne_bytes([entry[4], entry[5], entry[6], entry[7]]);
        entries.push((time, counter));
    }
    entries
}

// One full server sequence: Configure, Arm, GetStatus until complete, GetAcquisitionMetadata,
// ReadAcquisition until finished, ResetDatalogger.
fn run_acquisition_cycle(h: &mut MainHandler<'_, '_>, cycle: u32) {
    THRESHOLD.store(u32::MAX, Ordering::SeqCst);

    let resp = send_datalog_request(
        h,
        DataLogControlSubfn::ConfigureDatalog,
        &encode_threshold_configure_payload(),
    );
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "cycle {cycle}: Configure");
    wait_for_ownership_taken(h);

    let resp = send_datalog_request(h, DataLogControlSubfn::ArmTrigger, &[]);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "cycle {cycle}: ArmTrigger");
    wait_for_state(h, DlState::Armed);

    // The trigger is armed, the counter crosses the threshold TRIGGER_MARGIN loop iterations from now.
    let threshold = COUNTER.load(Ordering::SeqCst) + TRIGGER_MARGIN;
    THRESHOLD.store(threshold, Ordering::SeqCst);
    wait_for_state(h, DlState::AcquisitionCompleted);

    let resp = send_datalog_request(h, DataLogControlSubfn::GetAcquisitionMetadata, &[]);
    assert_eq!(
        resp_code(&resp),
        ResponseCode::Ok as u8,
        "cycle {cycle}: GetAcquisitionMetadata"
    );
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 16, "cycle {cycle}: metadata length");
    let acquisition_id = u16::from_be_bytes([p[0], p[1]]);
    let config_id = u16::from_be_bytes([p[2], p[3]]);
    let entry_count = u32::from_be_bytes([p[4], p[5], p[6], p[7]]) as usize;
    let data_size = u32::from_be_bytes([p[8], p[9], p[10], p[11]]) as usize;
    let points_after_trigger = u32::from_be_bytes([p[12], p[13], p[14], p[15]]) as usize;
    assert_eq!(config_id, CONFIG_ID, "cycle {cycle}");
    assert!(entry_count > 0, "cycle {cycle}: no entry");
    assert_eq!(data_size, entry_count * ENTRY_SIZE, "cycle {cycle}");
    assert!(
        points_after_trigger < entry_count,
        "cycle {cycle}: trigger outside of the acquisition"
    );
    assert!(
        !h.datalogging_ownership_taken(),
        "cycle {cycle}: the loop should have released the datalogger"
    );

    let data = read_all_acquisition(h, acquisition_id);
    assert_eq!(data.len(), data_size, "cycle {cycle}");
    let entries = decode_entries(&data);
    assert_eq!(entries.len(), entry_count, "cycle {cycle}");

    // One entry per loop iteration: the counter and the time advance together.
    for i in 1..entries.len() {
        let (prev_time, prev_counter) = entries[i - 1];
        let (time, counter) = entries[i];
        assert!(
            counter > prev_counter,
            "cycle {cycle} entry {i}: counter not strictly increasing"
        );
        assert_eq!(counter - prev_counter, 1, "cycle {cycle} entry {i}: counter step");
        assert_eq!(
            time.wrapping_sub(prev_time),
            LOOP_TIMESTEP,
            "cycle {cycle} entry {i}: time step"
        );
    }

    // The trigger entry is the last one before the points logged after the trigger.
    let trigger_index = entry_count - points_after_trigger - 1;
    assert_eq!(
        entries[trigger_index].1,
        threshold + 1,
        "cycle {cycle}: counter at the trigger point"
    );
    if trigger_index > 0 {
        assert_eq!(
            entries[trigger_index - 1].1,
            threshold,
            "cycle {cycle}: counter before the trigger point"
        );
    }

    let resp = send_datalog_request(h, DataLogControlSubfn::ResetDatalogger, &[]);
    assert_eq!(
        resp_code(&resp),
        ResponseCode::Ok as u8,
        "cycle {cycle}: ResetDatalogger"
    );
    assert_eq!(get_status_state(h), DlState::Idle as u8, "cycle {cycle}");
    assert!(!h.datalogging_data_available(), "cycle {cycle}");
}

// The loop runs in its own thread: the datalogger ownership goes back and forth between the two
// time domains through the IPC, for several acquisitions in a row.
#[test]
fn test_threaded_acquisition() {
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128]; // Forces several ReadAcquisition chunks
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&LOOPS);
    cfg.set_datalogger_buffer(unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF) }, None);
    let Some(mut h) = MainHandler::new(cfg) else {
        panic!("MainHandler creation failed");
    };
    connect(&mut h);

    // Declared after the MainHandler so it is dropped (thread stopped) before it.
    let mut loop_thread = LoopThread::start();

    for cycle in 0..3 {
        run_acquisition_cycle(&mut h, cycle);
    }

    assert!(loop_thread.stop(), "loop thread panicked");
}
