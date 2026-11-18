#![cfg(feature = "datalogging")]
// Test values mirror the C++ tests (3.1415926).
#![allow(clippy::approx_constant)]

use scrutiny_embedded_rust::build_config::{DATALOGGING_MAX_SIGNAL, REQUEST_MAX_PROCESS_TIME_US};
use scrutiny_embedded_rust::config::Config;
use scrutiny_embedded_rust::datalogging::{State as DlState, MAX_OPERANDS};
use scrutiny_embedded_rust::loop_handler::{FixedFreqLoopHandler, LoopHandler, VariableFreqLoopHandler};
use scrutiny_embedded_rust::main_handler::MainHandler;
use scrutiny_embedded_rust::protocol::{
    CommControlSubfn, CommandId, DataLogControlSubfn, GetInfoSubfn, ResponseCode, CONNECT_MAGIC,
};
use scrutiny_embedded_rust::tools::crc32;
use scrutiny_embedded_rust::types::{AnyType, RuntimePublishedValue, VariableType};
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(feature = "protected_regions")]
use scrutiny_embedded_rust::types::AddressRange;

const DATALOG_BUFFER_SIZE: usize = 128;

static mut SOME_VAR_OPERAND1: f32 = 0.0;
static mut SOME_VAR_LOGGED1: f32 = 0.0;

static TEST_RPVS: [RuntimePublishedValue; 1] = [RuntimePublishedValue {
    id: 0x8888,
    var_type: VariableType::Float32,
}];

fn rpv_read_callback(rpv: RuntimePublishedValue, out: &mut AnyType, _caller: Option<&'static str>) -> bool {
    if rpv.id == 0x8888 && rpv.var_type == VariableType::Float32 {
        out.float32 = 1.7f32;
        true
    } else {
        false
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

fn send_and_recv(h: &mut MainHandler<'_, '_>, req: &[u8]) -> Vec<u8> {
    h.receive_data(req);
    h.process(0);
    pop_response(h)
}

fn connect(h: &mut MainHandler<'_, '_>) {
    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Connect as u8,
        &CONNECT_MAGIC,
    );
    h.receive_data(&req);
    h.process(0);
    let n = h.data_to_send() as usize;
    let mut buf = vec![0u8; n];
    h.pop_data(&mut buf);
    h.process(0);
}

fn resp_code(resp: &[u8]) -> u8 {
    resp[2]
}

fn resp_payload(resp: &[u8]) -> &[u8] {
    let len = u16::from_be_bytes([resp[3], resp[4]]) as usize;
    &resp[5..5 + len]
}

// Wire format helpers

struct DlConfig {
    loop_id: u8,
    config_id: u16,
    decimation: u16,
    probe_location: u8,
    timeout_100ns: u32,
    trigger_condition: u8,
    hold_time_100ns: u32,
    operands: Vec<DlOperand>,
    signals: Vec<DlSignal>,
}

enum DlOperand {
    Literal(f32),
    Var {
        addr: usize,
        datatype: u8,
    },
    VarBit {
        addr: usize,
        datatype: u8,
        bitoffset: u8,
        bitsize: u8,
    },
    Rpv(u16),
}

enum DlSignal {
    Time,
    Memory { addr: usize, size: u8 },
    Rpv(u16),
}

fn encode_usize_be(val: usize) -> Vec<u8> {
    val.to_be_bytes().to_vec()
}

fn encode_configure_payload(cfg: &DlConfig) -> Vec<u8> {
    let mut buf = vec![cfg.loop_id];
    buf.extend_from_slice(&cfg.config_id.to_be_bytes());
    buf.extend_from_slice(&cfg.decimation.to_be_bytes());
    buf.push(cfg.probe_location);
    buf.extend_from_slice(&cfg.timeout_100ns.to_be_bytes());
    buf.push(cfg.trigger_condition);
    buf.extend_from_slice(&cfg.hold_time_100ns.to_be_bytes());
    buf.push(cfg.operands.len() as u8);
    for op in &cfg.operands {
        match op {
            DlOperand::Literal(val) => {
                buf.push(0); // Literal type
                buf.extend_from_slice(&val.to_be_bytes());
            }
            DlOperand::Var { addr, datatype } => {
                buf.push(1); // Var type
                buf.push(*datatype);
                buf.extend_from_slice(&encode_usize_be(*addr));
            }
            DlOperand::VarBit {
                addr,
                datatype,
                bitoffset,
                bitsize,
            } => {
                buf.push(2); // VarBit type
                buf.push(*datatype);
                buf.extend_from_slice(&encode_usize_be(*addr));
                buf.push(*bitoffset);
                buf.push(*bitsize);
            }
            DlOperand::Rpv(id) => {
                buf.push(3); // Rpv type
                buf.extend_from_slice(&id.to_be_bytes());
            }
        }
    }
    buf.push(cfg.signals.len() as u8);
    for sig in &cfg.signals {
        match sig {
            DlSignal::Memory { addr, size } => {
                buf.push(0); // Memory type
                buf.extend_from_slice(&encode_usize_be(*addr));
                buf.push(*size);
            }
            DlSignal::Rpv(id) => {
                buf.push(1); // Rpv type
                buf.extend_from_slice(&id.to_be_bytes());
            }
            DlSignal::Time => {
                buf.push(2); // Time type
            }
        }
    }
    buf
}

fn get_valid_reference_config() -> DlConfig {
    DlConfig {
        loop_id: 1,
        config_id: 0xaa55,
        decimation: 0x1234,
        probe_location: 123,
        timeout_100ns: 0x1122_3344,
        trigger_condition: 1, // Equal
        hold_time_100ns: 0xaabb_ccdd,
        operands: vec![
            DlOperand::Literal(3.141_592_6f32),
            DlOperand::Var {
                addr: core::ptr::addr_of!(SOME_VAR_OPERAND1) as usize,
                datatype: VariableType::Float32 as u8,
            },
        ],
        signals: vec![
            DlSignal::Time,
            DlSignal::Memory {
                addr: core::ptr::addr_of!(SOME_VAR_LOGGED1) as usize,
                size: 4,
            },
            DlSignal::Rpv(0x8888),
        ],
    }
}

fn get_simple_trigger_config(loop_id: u8) -> DlConfig {
    DlConfig {
        loop_id,
        config_id: 0xabcd,
        decimation: 1,
        probe_location: 0,
        timeout_100ns: 0,
        trigger_condition: 0, // AlwaysTrue
        hold_time_100ns: 1,
        operands: vec![],
        signals: vec![DlSignal::Time],
    }
}

fn send_configure(h: &mut MainHandler<'_, '_>, cfg: &DlConfig) -> Vec<u8> {
    let payload = encode_configure_payload(cfg);
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ConfigureDatalog as u8,
        &payload,
    );
    send_and_recv(h, &req)
}

fn get_status_state(h: &mut MainHandler<'_, '_>) -> u8 {
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::GetStatus as u8,
        &[],
    );
    let resp = send_and_recv(h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    resp_payload(&resp)[0]
}

// Lets the helpers drive either loop type with an explicit timestep.
trait TestLoop {
    fn process_with_time(&self, timestep_100ns: u32);
}

impl TestLoop for FixedFreqLoopHandler {
    fn process_with_time(&self, timestep_100ns: u32) {
        FixedFreqLoopHandler::process_with_time(self, timestep_100ns);
    }
}

impl TestLoop for VariableFreqLoopHandler {
    fn process_with_time(&self, timestep_100ns: u32) {
        VariableFreqLoopHandler::process_with_time(self, timestep_100ns);
    }
}

// Transfer ownership from main to loop and back (Configured state achieved).
// Returns after main has received OwnershipTaken.
fn do_ownership_transfer(h: &mut MainHandler<'_, '_>, loop_handler: &impl TestLoop) {
    h.process(0); // sends TakeOwnership IPC
    loop_handler.process_with_time(0); // receives TakeOwnership, sends OwnershipTaken
    h.process(0); // receives OwnershipTaken, sets dl.owner
}

// Arm trigger via protocol, then let loop process it.
fn do_arm_trigger(h: &mut MainHandler<'_, '_>, loop_handler: &impl TestLoop) {
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ArmTrigger as u8,
        &[],
    );
    let resp = send_and_recv(h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "ArmTrigger should succeed");
    h.process(0); // sends ArmTrigger IPC
    loop_handler.process_with_time(0); // arms the trigger
    loop_handler.process_with_time(0); // sends StatusUpdate
    h.process(0); // receives StatusUpdate
}

// Run the loop until AcquisitionCompleted (uses AlwaysTrue trigger).
fn run_until_acquisition(h: &mut MainHandler<'_, '_>, loop_handler: &impl TestLoop) {
    for _ in 0..500 {
        loop_handler.process_with_time(1);
        h.process(1);
        if get_status_state(h) == DlState::AcquisitionCompleted as u8 {
            return;
        }
    }
    panic!("Acquisition did not complete within 500 iterations");
}

// Sends a DataLogControl request and drives the release-for-read handshake if a loop owns the datalogger.
fn send_and_recv_with_release(h: &mut MainHandler<'_, '_>, loop_handler: &impl TestLoop, req: &[u8]) -> Vec<u8> {
    h.receive_data(req);
    h.process(0); // Answers directly, or sends ReleaseOwnership
    if h.data_to_send() > 0 {
        return pop_response(h);
    }
    loop_handler.process_with_time(0);
    h.process(0); // Receives OwnershipReleased with data preserved, then answers
    pop_response(h)
}

fn make_handler_with_loops<'rx, 'tx>(
    rx: &'rx mut [u8],
    tx: &'tx mut [u8],
    dl_buf: &'static mut [u8],
    loops: &'static [&'static LoopHandler],
) -> MainHandler<'rx, 'tx> {
    let mut cfg = Config::new();
    cfg.set_buffers(rx, tx);
    cfg.set_loops(loops);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_callback), None);
    cfg.set_datalogger_buffer(dl_buf, None);
    MainHandler::new(cfg).unwrap()
}

// Tests

#[test]
fn test_unsupported() {
    // Handler with no dl buffer: DataLogControl returns UnsupportedFeature for all subfns
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("L", 100);
    static UNSUP_LOOPS: [&LoopHandler; 1] = [LOOP1.handler()];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&UNSUP_LOOPS);
    // Intentionally no set_datalogger_buffer
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    for subfn in 0u8..=255 {
        let req = make_request(CommandId::DataLogControl as u8, subfn, &[]);
        let resp = send_and_recv(&mut h, &req);
        assert_eq!(
            resp_code(&resp),
            ResponseCode::UnsupportedFeature as u8,
            "subfn={subfn}"
        );
        h.process(0);
    }
}

// Without a datalogging buffer, the state getter reports Idle and no error, like C++.
#[test]
fn test_getters_without_datalogging_buffer() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("L", 100);
    static NO_BUF_LOOPS: [&LoopHandler; 1] = [LOOP1.handler()];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&NO_BUF_LOOPS);
    let mut h = MainHandler::new(cfg).unwrap();

    assert_eq!(h.get_datalogger_state(), DlState::Idle);
    assert!(!h.datalogging_error());
    h.process(0);
    LOOP1.process_with_time(0);
    h.process(0);
    assert_eq!(h.get_datalogger_state(), DlState::Idle);
    assert!(!h.datalogging_error());
}

#[test]
fn test_get_setup() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("L", 100);
    static LOOPS_GET_SETUP: [&LoopHandler; 1] = [LOOP1.handler()];
    static mut DL_BUF_GET_SETUP: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 256];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_GET_SETUP) },
        &LOOPS_GET_SETUP,
    );
    connect(&mut h);

    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::GetSetup as u8,
        &[],
    );
    let resp = send_and_recv(&mut h, &req);
    h.process(0);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(
        p.len(),
        6,
        "GetSetup payload: buf_size(4) + encoding(1) + max_signal(1)"
    );
    let buf_size = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
    assert_eq!(buf_size, DATALOG_BUFFER_SIZE as u32);
    assert_eq!(p[4], 0, "Raw encoding = 0");
    assert_eq!(p[5], DATALOGGING_MAX_SIGNAL as u8);
}

#[test]
fn test_configure_valid() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_CONFIGURE_VALID: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_CONFIGURE_VALID: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_CONFIGURE_VALID) },
        &LOOPS_CONFIGURE_VALID,
    );
    connect(&mut h);

    let cfg = get_valid_reference_config();
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
}

#[test]
fn test_configure_bad_loop_id() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_BAD_LOOP: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_BAD_LOOP: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_BAD_LOOP) },
        &LOOPS_BAD_LOOP,
    );
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.loop_id = 55; // doesn't exist
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_configure_no_datalogging_support() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging(); // no datalogging
    static LOOPS_NO_DL: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_NO_DL: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_NO_DL) },
        &LOOPS_NO_DL,
    );
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.loop_id = 2; // datalogging disabled loop
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);
}

fn get_supported_features(h: &mut MainHandler<'_, '_>) -> u8 {
    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetSupportedFeatures as u8, &[]);
    let resp = send_and_recv(h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    resp_payload(&resp)[0]
}

fn get_loop_attributes(h: &mut MainHandler<'_, '_>, loop_id: u8) -> u8 {
    let req = make_request(
        CommandId::GetInfo as u8,
        GetInfoSubfn::GetLoopDefinition as u8,
        &[loop_id],
    );
    let resp = send_and_recv(h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p[0], loop_id);
    p[2]
}

// Buffer set but no loop allows datalogging: no datalogger is created (accepted difference D-02).
#[test]
fn test_datalogging_disabled_on_all_loops() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100).without_datalogging();
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2").without_datalogging();
    static LOOPS_ALL_DISABLED: [&LoopHandler; 2] = [LOOP1.handler(), LOOP2.handler()];
    static mut DL_BUF_ALL_DISABLED: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_ALL_DISABLED) },
        &LOOPS_ALL_DISABLED,
    );
    connect(&mut h);

    assert_eq!(
        get_supported_features(&mut h) & 0x40,
        0,
        "datalogging bit (0x40) must be clear"
    );
    assert_eq!(get_loop_attributes(&mut h, 0) & 0x80, 0);
    assert_eq!(get_loop_attributes(&mut h, 1) & 0x80, 0);

    let subfns = [
        DataLogControlSubfn::GetSetup,
        DataLogControlSubfn::ConfigureDatalog,
        DataLogControlSubfn::ArmTrigger,
        DataLogControlSubfn::DisarmTrigger,
        DataLogControlSubfn::GetStatus,
        DataLogControlSubfn::GetAcquisitionMetadata,
        DataLogControlSubfn::ReadAcquisition,
        DataLogControlSubfn::ResetDatalogger,
    ];
    for subfn in subfns {
        let resp = if subfn == DataLogControlSubfn::ConfigureDatalog {
            let mut cfg = get_valid_reference_config();
            cfg.loop_id = 0;
            send_configure(&mut h, &cfg)
        } else {
            let req = make_request(CommandId::DataLogControl as u8, subfn as u8, &[]);
            send_and_recv(&mut h, &req)
        };
        h.process(0);
        assert_eq!(
            resp_code(&resp),
            ResponseCode::UnsupportedFeature as u8,
            "subfn={}",
            subfn as u8
        );
    }

    LOOP1.process_with_time(0);
    LOOP2.process_with_time(0);
    h.process(0);
    assert_eq!(h.get_datalogger_state(), DlState::Idle);
}

// Only one loop allows datalogging: the feature is advertised, the other loop is refused.
#[test]
fn test_datalogging_disabled_on_one_loop() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2").without_datalogging();
    static LOOPS_ONE_DISABLED: [&LoopHandler; 2] = [LOOP1.handler(), LOOP2.handler()];
    static mut DL_BUF_ONE_DISABLED: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_ONE_DISABLED) },
        &LOOPS_ONE_DISABLED,
    );
    connect(&mut h);

    assert_ne!(
        get_supported_features(&mut h) & 0x40,
        0,
        "datalogging bit (0x40) must be set"
    );
    assert_ne!(get_loop_attributes(&mut h, 0) & 0x80, 0, "loop 0 allows datalogging");
    assert_eq!(
        get_loop_attributes(&mut h, 1) & 0x80,
        0,
        "loop 1 does not allow datalogging"
    );

    let mut cfg = get_valid_reference_config();
    cfg.loop_id = 1;
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);

    cfg.loop_id = 0;
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
}

#[test]
fn test_configure_item_count_overflow() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_OVERFLOW: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_OVERFLOW: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_OVERFLOW) },
        &LOOPS_OVERFLOW,
    );
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    // Add extra signals beyond the max
    while cfg.signals.len() <= DATALOGGING_MAX_SIGNAL {
        cfg.signals.push(DlSignal::Time);
    }
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Overflow as u8);
}

#[test]
fn test_configure_operand_count_overflow() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_OP_OVERFLOW: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_OP_OVERFLOW: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_OP_OVERFLOW) },
        &LOOPS_OP_OVERFLOW,
    );
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    // Add extra operands beyond MAX_OPERANDS
    while cfg.operands.len() <= MAX_OPERANDS {
        cfg.operands.push(DlOperand::Literal(1.0));
    }
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Overflow as u8);
}

#[test]
fn test_configure_operand_count_mismatch() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_OP_MISMATCH: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_OP_MISMATCH: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_OP_MISMATCH) },
        &LOOPS_OP_MISMATCH,
    );
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.trigger_condition = 1; // Equal needs 2 operands
    cfg.operands.clear(); // but we provide 0
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_bad_decimation() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_BAD_DEC: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_BAD_DEC: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_BAD_DEC) },
        &LOOPS_BAD_DEC,
    );
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.decimation = 0; // not supported
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_configure_bad_operands() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_BAD_OP: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_BAD_OP: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_BAD_OP) },
        &LOOPS_BAD_OP,
    );
    connect(&mut h);

    let bad_values: [f32; 4] = [
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::from_bits(0x7F80_0001u32), // signaling NaN
    ];

    for (i, bad_val) in bad_values.iter().enumerate() {
        let mut cfg = get_valid_reference_config();
        cfg.operands[0] = DlOperand::Literal(*bad_val);
        let resp = send_configure(&mut h, &cfg);
        h.process(0);
        assert_eq!(
            resp_code(&resp),
            ResponseCode::InvalidRequest as u8,
            "i={i} val={bad_val}"
        );
    }
}

#[test]
fn test_configure_operand_bad_rpv() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_BAD_RPV: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_BAD_RPV: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_BAD_RPV) },
        &LOOPS_BAD_RPV,
    );
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.operands[0] = DlOperand::Rpv(0x9999); // doesn't exist
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_configure_signal_in_forbidden_region() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_FORBIDDEN_SIG: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_FORBIDDEN_SIG: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    static mut FORBIDDEN_VAR_SIG: u32 = 0;
    static FORBIDDEN_RANGES_SIG: [AddressRange; 1] = [AddressRange {
        start: core::ptr::addr_of!(FORBIDDEN_VAR_SIG) as *const u8,
        end: unsafe { (core::ptr::addr_of!(FORBIDDEN_VAR_SIG) as *const u8).add(4) },
    }];

    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&LOOPS_FORBIDDEN_SIG);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_callback), None);
    cfg.set_forbidden_address_range(&FORBIDDEN_RANGES_SIG);
    cfg.set_datalogger_buffer(unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_FORBIDDEN_SIG) }, None);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut dl_cfg = get_valid_reference_config();
    dl_cfg.signals[1] = DlSignal::Memory {
        addr: core::ptr::addr_of!(FORBIDDEN_VAR_SIG) as usize,
        size: 4,
    };
    let resp = send_configure(&mut h, &dl_cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_configure_operand_var_in_forbidden_region() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_FORBIDDEN_VAR: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_FORBIDDEN_VAR: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    static mut FORBIDDEN_VAR_OP: u32 = 0;
    static FORBIDDEN_RANGES_VAR: [AddressRange; 1] = [AddressRange {
        start: core::ptr::addr_of!(FORBIDDEN_VAR_OP) as *const u8,
        end: unsafe { (core::ptr::addr_of!(FORBIDDEN_VAR_OP) as *const u8).add(4) },
    }];

    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&LOOPS_FORBIDDEN_VAR);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_callback), None);
    cfg.set_forbidden_address_range(&FORBIDDEN_RANGES_VAR);
    cfg.set_datalogger_buffer(unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_FORBIDDEN_VAR) }, None);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut dl_cfg = get_valid_reference_config();
    dl_cfg.operands[1] = DlOperand::Var {
        addr: core::ptr::addr_of!(FORBIDDEN_VAR_OP) as usize,
        datatype: VariableType::Uint32 as u8,
    };
    let resp = send_configure(&mut h, &dl_cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_configure_operand_varbit_in_forbidden_region() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_FORBIDDEN_VARBIT: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_FORBIDDEN_VARBIT: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    static mut FORBIDDEN_VAR_VB: u32 = 0;
    static FORBIDDEN_RANGES_VARBIT: [AddressRange; 1] = [AddressRange {
        start: core::ptr::addr_of!(FORBIDDEN_VAR_VB) as *const u8,
        end: unsafe { (core::ptr::addr_of!(FORBIDDEN_VAR_VB) as *const u8).add(4) },
    }];

    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&LOOPS_FORBIDDEN_VARBIT);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_callback), None);
    cfg.set_forbidden_address_range(&FORBIDDEN_RANGES_VARBIT);
    cfg.set_datalogger_buffer(unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_FORBIDDEN_VARBIT) }, None);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut dl_cfg = get_valid_reference_config();
    dl_cfg.operands[1] = DlOperand::VarBit {
        addr: core::ptr::addr_of!(FORBIDDEN_VAR_VB) as usize,
        datatype: VariableType::Uint32 as u8,
        bitoffset: 0,
        bitsize: 8,
    };
    let resp = send_configure(&mut h, &dl_cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_configure_operand_varbit_almost_in_forbidden_region() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_ALMOST_FORBIDDEN: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_ALMOST_FORBIDDEN: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    static ALMOST_FORBIDDEN_BUFFER: [u8; 8] = [0; 8];
    // Forbid indices 4..6
    static FORBIDDEN_RANGES_ALMOST: [AddressRange; 1] = [AddressRange {
        start: unsafe { ALMOST_FORBIDDEN_BUFFER.as_ptr().add(4) },
        end: unsafe { ALMOST_FORBIDDEN_BUFFER.as_ptr().add(6) },
    }];

    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&LOOPS_ALMOST_FORBIDDEN);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_callback), None);
    cfg.set_forbidden_address_range(&FORBIDDEN_RANGES_ALMOST);
    cfg.set_datalogger_buffer(unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_ALMOST_FORBIDDEN) }, None);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // VarBit at index 2, type uint16 (2 bytes), touching indices 2..4, does not overlap [4..6]
    let mut dl_cfg = get_valid_reference_config();
    dl_cfg.operands[1] = DlOperand::VarBit {
        addr: unsafe { ALMOST_FORBIDDEN_BUFFER.as_ptr().add(2) as usize },
        datatype: VariableType::Uint16 as u8,
        bitoffset: 0,
        bitsize: 8,
    };
    let resp = send_configure(&mut h, &dl_cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
}

#[test]
fn test_configure_unknown_condition() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_UNK_COND: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_UNK_COND: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_UNK_COND) },
        &LOOPS_UNK_COND,
    );
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.trigger_condition = 0xAA;
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

// boolean16/32/64 (0x31-0x33) are not supported. A Var operand using them must be rejected.
#[test]
fn test_configure_operand_var_sized_boolean_rejected() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_VAR_SIZED_BOOL: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_VAR_SIZED_BOOL: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_VAR_SIZED_BOOL) },
        &LOOPS_VAR_SIZED_BOOL,
    );
    connect(&mut h);

    for datatype in [0x31u8, 0x32, 0x33] {
        let mut cfg = get_valid_reference_config();
        cfg.operands[1] = DlOperand::Var {
            addr: core::ptr::addr_of!(SOME_VAR_OPERAND1) as usize,
            datatype,
        };
        let resp = send_configure(&mut h, &cfg);
        h.process(0);
        assert_eq!(
            resp_code(&resp),
            ResponseCode::InvalidRequest as u8,
            "datatype={datatype:#x}"
        );
        assert_ne!(
            get_status_state(&mut h),
            DlState::Configured as u8,
            "datatype={datatype:#x}"
        );
    }
}

// boolean16/32/64 (0x31-0x33) are not supported. A VarBit operand using them must be rejected.
#[test]
fn test_configure_operand_varbit_sized_boolean_rejected() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_VARBIT_SIZED_BOOL: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_VARBIT_SIZED_BOOL: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_VARBIT_SIZED_BOOL) },
        &LOOPS_VARBIT_SIZED_BOOL,
    );
    connect(&mut h);

    for datatype in [0x31u8, 0x32, 0x33] {
        let mut cfg = get_valid_reference_config();
        cfg.operands[1] = DlOperand::VarBit {
            addr: core::ptr::addr_of!(SOME_VAR_OPERAND1) as usize,
            datatype,
            bitoffset: 0,
            bitsize: 1,
        };
        let resp = send_configure(&mut h, &cfg);
        h.process(0);
        assert_eq!(
            resp_code(&resp),
            ResponseCode::InvalidRequest as u8,
            "datatype={datatype:#x}"
        );
        assert_ne!(
            get_status_state(&mut h),
            DlState::Configured as u8,
            "datatype={datatype:#x}"
        );
    }
}

#[test]
fn test_configure_loggable_bad_rpv() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_LOGGABLE_BAD_RPV: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_LOGGABLE_BAD_RPV: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_LOGGABLE_BAD_RPV) },
        &LOOPS_LOGGABLE_BAD_RPV,
    );
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.signals[0] = DlSignal::Rpv(0x9999); // doesn't exist
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_owner_mechanism() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_OWNER: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_OWNER: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let tx_size = DATALOG_BUFFER_SIZE + 8 + 9;
    let mut tx = vec![0u8; tx_size];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_OWNER) },
        &LOOPS_OWNER,
    );
    connect(&mut h);

    let loop0 = &LOOP1;
    let loop1 = &LOOP2;

    // Configure loop 0 with AlwaysTrue
    let cfg0 = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg0);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    // Transfer ownership to loop0
    do_ownership_transfer(&mut h, loop0);
    assert_eq!(h.get_datalogger_state(), DlState::Configured);
    assert!(!h.datalogging_error());

    // Arm
    do_arm_trigger(&mut h, loop0);

    // Run until acquisition complete
    run_until_acquisition(&mut h, loop0);
    assert_eq!(get_status_state(&mut h), DlState::AcquisitionCompleted as u8);

    // Re-configure to loop1, which requires loop0 to release ownership
    let cfg1 = get_simple_trigger_config(1);
    let payload = encode_configure_payload(&cfg1);
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ConfigureDatalog as u8,
        &payload,
    );

    h.receive_data(&req);
    h.process(0); // sets pending_release, returns ProcessAgain, no response yet

    // Loop0 must release first
    h.process(0); // sends ReleaseOwnership IPC
    loop0.process_with_time(0); // receives ReleaseOwnership, sends OwnershipReleased
    h.process(0); // receives OwnershipReleased (dl.owner = None), retries configure -> Ok

    let resp = pop_response(&mut h);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    // Now loop1 gets TakeOwnership
    h.process(0); // sends TakeOwnership IPC to loop1
    loop1.process_with_time(0); // receives TakeOwnership, sends OwnershipTaken
    h.process(0); // receives OwnershipTaken

    // Arm loop1
    do_arm_trigger(&mut h, loop1);

    // Run acquisition on loop1
    run_until_acquisition(&mut h, loop1);
    assert_eq!(get_status_state(&mut h), DlState::AcquisitionCompleted as u8);
}

#[test]
fn test_arm_trigger_not_configured() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOPS_ARM_NOT_CFG: [&LoopHandler; 1] = [LOOP1.handler()];
    static mut DL_BUF_ARM_NOT_CFG: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_ARM_NOT_CFG) },
        &LOOPS_ARM_NOT_CFG,
    );
    connect(&mut h);

    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ArmTrigger as u8,
        &[],
    );
    let resp = send_and_recv(&mut h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_arm_disarm_trigger_ok() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_ARM_DISARM: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_ARM_DISARM: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_ARM_DISARM) },
        &LOOPS_ARM_DISARM,
    );
    connect(&mut h);

    let loop0 = &LOOP1;

    // Configure with the reference config (Equal condition, won't auto-trigger)
    let cfg = get_valid_reference_config();
    let mut cfg_loop0 = cfg;
    cfg_loop0.loop_id = 0;
    let resp = send_configure(&mut h, &cfg_loop0);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);

    // Arm trigger
    let arm_req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ArmTrigger as u8,
        &[],
    );
    let arm_resp = send_and_recv(&mut h, &arm_req);
    h.process(0);
    assert_eq!(resp_code(&arm_resp), ResponseCode::Ok as u8);
    h.process(0); // sends ArmTrigger IPC
    loop0.process_with_time(0); // arms the trigger
    loop0.process_with_time(0); // sends StatusUpdate
    h.process(0); // receives StatusUpdate
    assert_eq!(get_status_state(&mut h), DlState::Armed as u8);

    // Disarm trigger
    let disarm_req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::DisarmTrigger as u8,
        &[],
    );
    let disarm_resp = send_and_recv(&mut h, &disarm_req);
    h.process(0);
    assert_eq!(resp_code(&disarm_resp), ResponseCode::Ok as u8);
    h.process(0); // sends DisarmTrigger IPC
    loop0.process_with_time(0); // disarms the trigger
    loop0.process_with_time(0); // sends StatusUpdate
    h.process(0); // receives StatusUpdate
    assert_eq!(get_status_state(&mut h), DlState::Configured as u8);
}

#[test]
fn test_get_status() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_GET_STATUS: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_GET_STATUS: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    let tx_size = DATALOG_BUFFER_SIZE + 8 + 9;
    let mut tx = vec![0u8; tx_size];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_GET_STATUS) },
        &LOOPS_GET_STATUS,
    );
    connect(&mut h);

    let loop0 = &LOOP1;

    // Initial state: Idle
    assert_eq!(
        get_status_state(&mut h),
        DlState::Idle as u8,
        "initial state should be Idle"
    );

    // Configure loop 0 with AlwaysTrue trigger
    let cfg = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);

    // After ownership transfer: loop sends StatusUpdate when it processes
    loop0.process_with_time(0); // send status
    h.process(0); // receive status
    assert_eq!(
        get_status_state(&mut h),
        DlState::Configured as u8,
        "should be Configured"
    );

    // Arm via protocol
    do_arm_trigger(&mut h, loop0);
    assert_eq!(get_status_state(&mut h), DlState::Armed as u8, "should be Armed");

    // Run until acquisition complete
    run_until_acquisition(&mut h, loop0);
    assert_eq!(
        get_status_state(&mut h),
        DlState::AcquisitionCompleted as u8,
        "should be AcquisitionCompleted"
    );
}

#[test]
fn test_get_acquisition_metadata() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_META: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_META: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    let tx_size = DATALOG_BUFFER_SIZE + 8 + 9;
    let mut tx = vec![0u8; tx_size];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_META) },
        &LOOPS_META,
    );
    connect(&mut h);

    let loop0 = &LOOP1;

    // Configure
    let cfg = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);

    // Before acquisition: expect FailureToProceed
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::GetAcquisitionMetadata as u8,
        &[],
    );
    let resp = send_and_recv(&mut h, &req);
    h.process(0);
    assert_eq!(
        resp_code(&resp),
        ResponseCode::FailureToProceed as u8,
        "no acquisition yet"
    );

    // Arm and run to completion
    do_arm_trigger(&mut h, loop0);
    run_until_acquisition(&mut h, loop0);

    // After acquisition: loop still owns; main must release-for-read first.
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::GetAcquisitionMetadata as u8,
        &[],
    );
    let resp = send_and_recv_with_release(&mut h, loop0, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "acquisition complete");
    let p = resp_payload(&resp);
    // acquisition_id(2) + config_id(2) + entry_count(4) + data_size(4) + after_trigger(4)
    assert_eq!(p.len(), 16, "metadata payload should be 16 bytes");
    let config_id = u16::from_be_bytes([p[2], p[3]]);
    assert_eq!(config_id, 0xabcd, "config_id should match");
}

#[test]
fn test_read_acquisition_no_data_available() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOPS_READ_NO_DATA: [&LoopHandler; 1] = [LOOP1.handler()];
    static mut DL_BUF_READ_NO_DATA: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_READ_NO_DATA) },
        &LOOPS_READ_NO_DATA,
    );
    connect(&mut h);

    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ReadAcquisition as u8,
        &[],
    );
    let resp = send_and_recv(&mut h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_read_acquisition_one_transfer() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_READ_ONE: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_READ_ONE: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    // TX must be large enough to hold the entire buffer in one frame
    let tx_size = DATALOG_BUFFER_SIZE + 8 + 9 + 64;
    let mut tx = vec![0u8; tx_size];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_READ_ONE) },
        &LOOPS_READ_ONE,
    );
    connect(&mut h);

    let loop0 = &LOOP1;

    let cfg = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);
    do_arm_trigger(&mut h, loop0);
    run_until_acquisition(&mut h, loop0);

    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ReadAcquisition as u8,
        &[],
    );
    let out = send_and_recv_with_release(&mut h, loop0, &req);
    h.process(0);

    assert_eq!(resp_code(&out), ResponseCode::Ok as u8);
    // Response payload: finished(1) + rolling_counter(1) + acquisition_id(2) + data + crc(4)
    let payload_len = u16::from_be_bytes([out[3], out[4]]) as usize;
    assert!(payload_len >= 8, "payload too short");

    let finished = out[5];
    let rolling_counter = out[6];
    assert_eq!(finished, 1, "should be finished in one transfer");
    assert_eq!(rolling_counter, 0, "first rolling counter");

    // Verify the data CRC (last 4 bytes of payload before frame CRC)
    // Frame layout: hdr(5) + payload(payload_len) + frame_crc(4)
    let data_len = payload_len - 4 - 4; // subtract header(4 bytes in payload) and crc(4)
    let data = &out[9..9 + data_len];
    let expected_crc = crc32(data, 0);
    let stored_crc = u32::from_be_bytes([
        out[9 + data_len],
        out[9 + data_len + 1],
        out[9 + data_len + 2],
        out[9 + data_len + 3],
    ]);
    assert_eq!(stored_crc, expected_crc, "data CRC mismatch");
}

#[test]
fn test_read_acquisition_multiple_transfers() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_READ_MULTI: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_READ_MULTI: [u8; 1000] = [0; 1000];
    const DL_BUF_LEN: usize = 1000;
    const TX_LEN: usize = 64;
    let mut rx = [0u8; 512];
    // Small TX buffer to force multiple transfers
    let mut tx = [0u8; TX_LEN];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_READ_MULTI) },
        &LOOPS_READ_MULTI,
    );
    connect(&mut h);

    let loop0 = &LOOP1;

    let cfg = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);
    do_arm_trigger(&mut h, loop0);

    // Run until acquisition complete (need more iterations for bigger buffer)
    for _ in 0..5000 {
        loop0.process_with_time(1);
        h.process(1);
        if get_status_state(&mut h) == DlState::AcquisitionCompleted as u8 {
            break;
        }
    }
    assert_eq!(
        get_status_state(&mut h),
        DlState::AcquisitionCompleted as u8,
        "acquisition should complete"
    );

    // Read in multiple chunks
    let max_iterations = (DL_BUF_LEN + 4) / (TX_LEN - 9 - 4) + 10;
    let mut all_data: Vec<u8> = Vec::new();
    let mut finished = false;
    let mut last_crc = 0u32;

    for i in 0..max_iterations {
        let req = make_request(
            CommandId::DataLogControl as u8,
            DataLogControlSubfn::ReadAcquisition as u8,
            &[],
        );
        // First iteration releases ownership for read; later iterations run directly (owner is None).
        let resp = if i == 0 {
            send_and_recv_with_release(&mut h, loop0, &req)
        } else {
            send_and_recv(&mut h, &req)
        };
        h.process(0);
        assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "iteration {i}");

        let payload_len = u16::from_be_bytes([resp[3], resp[4]]) as usize;
        assert!(payload_len >= 4, "iteration {i}: payload too short");

        let chunk_finished = resp[5] != 0;
        let rolling_counter = resp[6];
        assert_eq!(
            rolling_counter,
            (i % 256) as u8,
            "rolling counter mismatch at iteration {i}"
        );

        if chunk_finished {
            // Last chunk: data + crc(4)
            let data_len = payload_len - 4 - 4;
            all_data.extend_from_slice(&resp[9..9 + data_len]);
            last_crc = u32::from_be_bytes([
                resp[9 + data_len],
                resp[9 + data_len + 1],
                resp[9 + data_len + 2],
                resp[9 + data_len + 3],
            ]);
            finished = true;
            break;
        } else {
            let data_len = payload_len - 4;
            all_data.extend_from_slice(&resp[9..9 + data_len]);
        }
    }

    assert!(finished, "should have finished reading");
    let expected_crc = crc32(&all_data, 0);
    assert_eq!(last_crc, expected_crc, "final CRC mismatch");
    assert!(!all_data.is_empty(), "should have read some data");
}

#[test]
fn test_reset_datalogger() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOP2: VariableFreqLoopHandler = VariableFreqLoopHandler::new("Loop2");
    static LOOP3: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop3", 100).without_datalogging();
    static LOOPS_RESET: [&LoopHandler; 3] = [LOOP1.handler(), LOOP2.handler(), LOOP3.handler()];
    static mut DL_BUF_RESET: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_RESET) },
        &LOOPS_RESET,
    );
    connect(&mut h);

    let loop0 = &LOOP1;

    // Initial state: Idle
    assert_eq!(get_status_state(&mut h), DlState::Idle as u8);

    // Configure and transfer ownership
    let cfg = get_valid_reference_config();
    let mut cfg_loop0 = cfg;
    cfg_loop0.loop_id = 0;
    let resp = send_configure(&mut h, &cfg_loop0);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);

    // Send StatusUpdate to get Configured state visible
    loop0.process_with_time(0); // send status
    h.process(0); // receive status
    assert_eq!(get_status_state(&mut h), DlState::Configured as u8);

    // Send Reset
    let reset_req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ResetDatalogger as u8,
        &[],
    );
    h.receive_data(&reset_req);
    h.process(0); // sets pending_release, ProcessAgain

    // Loop must release before reset proceeds
    h.process(0); // sends ReleaseOwnership IPC
    loop0.process_with_time(0); // receives ReleaseOwnership, sends OwnershipReleased
    h.process(1); // receives OwnershipReleased, retries Reset -> Ok
    h.process(0); // allow sending

    let resp = pop_response(&mut h);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "Reset should succeed");

    // After reset: Idle
    assert_eq!(
        get_status_state(&mut h),
        DlState::Idle as u8,
        "should be Idle after reset"
    );
}

// A request that keeps returning ProcessAgain (loop never releases the datalogger) is answered
// with FailureToProceed once REQUEST_MAX_PROCESS_TIME_US has elapsed, then the handler recovers.
#[test]
fn test_process_again_timeout_failure_to_proceed() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOPS_PA_TIMEOUT: [&LoopHandler; 1] = [LOOP1.handler()];
    static mut DL_BUF_PA_TIMEOUT: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_PA_TIMEOUT) },
        &LOOPS_PA_TIMEOUT,
    );
    connect(&mut h);

    let resp = send_configure(&mut h, &get_simple_trigger_config(0));
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    do_ownership_transfer(&mut h, &LOOP1);
    assert!(h.datalogging_ownership_taken());

    // The loop is not processed anymore, so it never releases the datalogger.
    let timeout_100ns = REQUEST_MAX_PROCESS_TIME_US * 10;
    let reset_req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ResetDatalogger as u8,
        &[],
    );
    h.receive_data(&reset_req);
    h.process(0); // First ProcessAgain, timestamp taken
    assert_eq!(h.data_to_send(), 0);

    let step = timeout_100ns / 10;
    for _ in 0..9 {
        h.process(step);
        assert_eq!(h.data_to_send(), 0);
    }
    h.process(step - 1);
    assert_eq!(h.data_to_send(), 0, "no response expected just before the timeout");

    h.process(1);
    assert!(h.data_to_send() > 0, "response expected once the timeout is reached");
    let resp = pop_response(&mut h);
    h.process(0);
    assert_eq!(resp[0], CommandId::DataLogControl as u8 | 0x80);
    assert_eq!(resp[1], DataLogControlSubfn::ResetDatalogger as u8);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
    assert!(resp_payload(&resp).is_empty());

    // The handler accepts a new request afterward.
    assert_eq!(get_status_state(&mut h), DlState::Configured as u8);
}

// A request returning ProcessAgain completes with OK when the loop releases the datalogger
// before REQUEST_MAX_PROCESS_TIME_US has elapsed.
#[test]
fn test_process_again_completes_before_timeout() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOPS_PA_OK: [&LoopHandler; 1] = [LOOP1.handler()];
    static mut DL_BUF_PA_OK: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_PA_OK) },
        &LOOPS_PA_OK,
    );
    connect(&mut h);

    let resp = send_configure(&mut h, &get_simple_trigger_config(0));
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    do_ownership_transfer(&mut h, &LOOP1);
    assert!(h.datalogging_ownership_taken());

    let timeout_100ns = REQUEST_MAX_PROCESS_TIME_US * 10;
    let step = timeout_100ns / 10;
    let reset_req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ResetDatalogger as u8,
        &[],
    );
    h.receive_data(&reset_req);
    h.process(0); // First ProcessAgain, timestamp taken

    let mut elapsed = 0u32;
    while h.data_to_send() == 0 {
        assert!(elapsed < timeout_100ns, "request did not complete before the timeout");
        LOOP1.process_with_time(step);
        h.process(step);
        elapsed += step;
    }
    let resp = pop_response(&mut h);
    h.process(0);
    assert!(elapsed < timeout_100ns);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    assert!(!h.datalogging_ownership_taken());
    assert_eq!(get_status_state(&mut h), DlState::Idle as u8);
}

// Read-with-release flow (D-03)

// AlwaysTrue trigger on loop 0 that logs the loop time and a u32 counter.
fn get_counter_trigger_config(counter: &AtomicU32) -> DlConfig {
    DlConfig {
        loop_id: 0,
        config_id: 0x1357,
        decimation: 1,
        probe_location: 0,
        timeout_100ns: 0,
        trigger_condition: 0, // AlwaysTrue
        hold_time_100ns: 1,
        operands: vec![],
        signals: vec![
            DlSignal::Time,
            DlSignal::Memory {
                addr: counter.as_ptr() as usize,
                size: 4,
            },
        ],
    }
}

// Like run_until_acquisition, but increments the counter before each loop step of 1 time unit,
// so the counter and the loop timestamp advance together.
fn run_until_acquisition_with_counter(h: &mut MainHandler<'_, '_>, loop_handler: &impl TestLoop, counter: &AtomicU32) {
    for _ in 0..500 {
        counter.fetch_add(1, Ordering::Relaxed);
        loop_handler.process_with_time(1);
        h.process(1);
        if get_status_state(h) == DlState::AcquisitionCompleted as u8 {
            return;
        }
    }
    panic!("Acquisition did not complete within 500 iterations");
}

// Configure with the counter config, give ownership to the loop, arm and run until the acquisition completes.
fn setup_completed_counter_acquisition(h: &mut MainHandler<'_, '_>, loop_handler: &impl TestLoop, counter: &AtomicU32) {
    let resp = send_configure(h, &get_counter_trigger_config(counter));
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    do_ownership_transfer(h, loop_handler);
    assert!(h.datalogging_ownership_taken());
    do_arm_trigger(h, loop_handler);
    run_until_acquisition_with_counter(h, loop_handler, counter);
    assert!(h.datalogging_ownership_taken());
}

// Reads every ReadAcquisition chunk while no loop owns the datalogger. Checks the rolling counter,
// the acquisition ID and the final CRC. Returns the concatenated data.
fn read_all_acquisition(h: &mut MainHandler<'_, '_>, acquisition_id: u16) -> Vec<u8> {
    let mut all_data: Vec<u8> = Vec::new();
    for i in 0..200usize {
        let req = make_request(
            CommandId::DataLogControl as u8,
            DataLogControlSubfn::ReadAcquisition as u8,
            &[],
        );
        let resp = send_and_recv(h, &req);
        h.process(0);
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

// Each entry is the loop time (u32, big endian) followed by the counter (u32, native endian).
// Both advance together, so their difference is the same in every entry.
fn check_counter_entries(data: &[u8]) {
    assert!(!data.is_empty());
    assert_eq!(data.len() % 8, 0, "data must hold whole entries");
    let mut expected_offset: Option<u32> = None;
    let mut first_ts: Option<u32> = None;
    let mut last_ts = 0u32;
    for (i, entry) in data.chunks_exact(8).enumerate() {
        let ts = u32::from_be_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let val = u32::from_ne_bytes([entry[4], entry[5], entry[6], entry[7]]);
        let offset = val.wrapping_sub(ts);
        match expected_offset {
            None => expected_offset = Some(offset),
            Some(o) => assert_eq!(offset, o, "entry {i}: counter does not match the logged time"),
        }
        if first_ts.is_none() {
            first_ts = Some(ts);
        } else {
            assert!(ts >= last_ts, "entry {i}: time going backward");
        }
        last_ts = ts;
    }
    assert!(first_ts.is_some_and(|t| last_ts > t), "logged time should advance");
}

// Sends GetAcquisitionMetadata while the loop owns a completed acquisition and asserts each step of the
// release-for-read flow, then reads the data. Returns the metadata payload.
fn do_explicit_read_release_flow(h: &mut MainHandler<'_, '_>, loop_handler: &impl TestLoop) -> Vec<u8> {
    assert!(h.datalogging_ownership_taken());
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::GetAcquisitionMetadata as u8,
        &[],
    );
    h.receive_data(&req);
    h.process(0); // ProcessAgain, ReleaseOwnership sent to the loop
    assert_eq!(
        h.data_to_send(),
        0,
        "no response before the loop releases the datalogger"
    );
    assert!(h.datalogging_ownership_taken(), "loop still owns the datalogger");

    loop_handler.process_with_time(0); // Releases the datalogger
    h.process(0); // Receives OwnershipReleased, then answers the request
    let resp = pop_response(h);
    h.process(0);
    assert_eq!(resp[0], CommandId::DataLogControl as u8 | 0x80);
    assert_eq!(resp[1], DataLogControlSubfn::GetAcquisitionMetadata as u8);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    assert!(
        !h.datalogging_ownership_taken(),
        "loop should have released the datalogger"
    );

    let metadata = resp_payload(&resp).to_vec();
    assert_eq!(metadata.len(), 16);
    let acquisition_id = u16::from_be_bytes([metadata[0], metadata[1]]);
    let config_id = u16::from_be_bytes([metadata[2], metadata[3]]);
    let entry_count = u32::from_be_bytes([metadata[4], metadata[5], metadata[6], metadata[7]]) as usize;
    let data_size = u32::from_be_bytes([metadata[8], metadata[9], metadata[10], metadata[11]]) as usize;
    assert_eq!(config_id, 0x1357);
    assert!(entry_count > 0, "the release must not reset the acquisition");
    assert_eq!(data_size, entry_count * 8);

    let data = read_all_acquisition(h, acquisition_id);
    assert_eq!(data.len(), data_size);
    check_counter_entries(&data);

    assert_eq!(get_status_state(h), DlState::AcquisitionCompleted as u8);
    assert!(h.datalogging_data_available());
    assert!(!h.datalogging_ownership_taken());
    metadata
}

// GetAcquisitionMetadata is held until the owning loop releases the datalogger; the data is preserved
// by that release and can be read in several chunks.
#[test]
fn test_read_release_flow_explicit() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOPS_RR_EXPLICIT: [&LoopHandler; 1] = [LOOP1.handler()];
    static mut DL_BUF_RR_EXPLICIT: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    static COUNTER_RR_EXPLICIT: AtomicU32 = AtomicU32::new(1000);
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 64]; // Forces several ReadAcquisition chunks
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_RR_EXPLICIT) },
        &LOOPS_RR_EXPLICIT,
    );
    connect(&mut h);

    setup_completed_counter_acquisition(&mut h, &LOOP1, &COUNTER_RR_EXPLICIT);
    do_explicit_read_release_flow(&mut h, &LOOP1);

    // A second read gives the same data, nothing was consumed or reset.
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::GetAcquisitionMetadata as u8,
        &[],
    );
    let resp = send_and_recv(&mut h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let acquisition_id = u16::from_be_bytes([resp_payload(&resp)[0], resp_payload(&resp)[1]]);
    check_counter_entries(&read_all_acquisition(&mut h, acquisition_id));
}

// After a read, no loop owns the datalogger, so ArmTrigger and DisarmTrigger fail until the next Configure.
#[test]
fn test_arm_after_read_fails() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOPS_RR_ARM: [&LoopHandler; 1] = [LOOP1.handler()];
    static mut DL_BUF_RR_ARM: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    static COUNTER_RR_ARM: AtomicU32 = AtomicU32::new(1000);
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 64];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_RR_ARM) },
        &LOOPS_RR_ARM,
    );
    connect(&mut h);

    setup_completed_counter_acquisition(&mut h, &LOOP1, &COUNTER_RR_ARM);
    do_explicit_read_release_flow(&mut h, &LOOP1);

    let arm_req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ArmTrigger as u8,
        &[],
    );
    let resp = send_and_recv(&mut h, &arm_req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);

    let disarm_req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::DisarmTrigger as u8,
        &[],
    );
    let resp = send_and_recv(&mut h, &disarm_req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);

    // The loop is not given the datalogger back and the acquisition is still available.
    LOOP1.process_with_time(1);
    h.process(0);
    assert!(!h.datalogging_ownership_taken());
    assert_eq!(get_status_state(&mut h), DlState::AcquisitionCompleted as u8);
}

// Mirrors the server poller (datalogging_poller.py): Configure, Arm, GetStatus until complete,
// GetAcquisitionMetadata, ReadAcquisition until finished, ResetDatalogger, then a new Configure and Arm.
#[test]
fn test_full_server_sequence() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOPS_SERVER_SEQ: [&LoopHandler; 1] = [LOOP1.handler()];
    static mut DL_BUF_SERVER_SEQ: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    static COUNTER_SERVER_SEQ: AtomicU32 = AtomicU32::new(1000);
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 64];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_SERVER_SEQ) },
        &LOOPS_SERVER_SEQ,
    );
    connect(&mut h);

    for cycle in 0..2 {
        setup_completed_counter_acquisition(&mut h, &LOOP1, &COUNTER_SERVER_SEQ);

        let req = make_request(
            CommandId::DataLogControl as u8,
            DataLogControlSubfn::GetAcquisitionMetadata as u8,
            &[],
        );
        let resp = send_and_recv_with_release(&mut h, &LOOP1, &req);
        h.process(0);
        assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "cycle {cycle}");
        let acquisition_id = u16::from_be_bytes([resp_payload(&resp)[0], resp_payload(&resp)[1]]);
        let data_size = u32::from_be_bytes([
            resp_payload(&resp)[8],
            resp_payload(&resp)[9],
            resp_payload(&resp)[10],
            resp_payload(&resp)[11],
        ]) as usize;

        let data = read_all_acquisition(&mut h, acquisition_id);
        assert_eq!(data.len(), data_size, "cycle {cycle}");
        check_counter_entries(&data);

        let reset_req = make_request(
            CommandId::DataLogControl as u8,
            DataLogControlSubfn::ResetDatalogger as u8,
            &[],
        );
        let resp = send_and_recv(&mut h, &reset_req);
        h.process(0);
        assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "cycle {cycle}");
        assert_eq!(get_status_state(&mut h), DlState::Idle as u8, "cycle {cycle}");
        assert!(!h.datalogging_data_available());
    }
}

// A release requested by Configure uses the default reset_on_release = true, even after a release-for-read.
// The Configure is left to time out so the reset done by the release itself is observable.
#[test]
fn test_release_without_read_resets() {
    static LOOP1: FixedFreqLoopHandler = FixedFreqLoopHandler::new("Loop1", 100);
    static LOOPS_RR_RESET: [&LoopHandler; 1] = [LOOP1.handler()];
    static mut DL_BUF_RR_RESET: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    static COUNTER_RR_RESET: AtomicU32 = AtomicU32::new(1000);
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 64];
    let mut h = make_handler_with_loops(
        &mut rx,
        &mut tx,
        unsafe { &mut *core::ptr::addr_of_mut!(DL_BUF_RR_RESET) },
        &LOOPS_RR_RESET,
    );
    connect(&mut h);

    // First cycle uses the release-for-read path (reset_on_release = false).
    setup_completed_counter_acquisition(&mut h, &LOOP1, &COUNTER_RR_RESET);
    do_explicit_read_release_flow(&mut h, &LOOP1);

    // Second cycle: Configure while the loop owns a completed acquisition.
    setup_completed_counter_acquisition(&mut h, &LOOP1, &COUNTER_RR_RESET);
    assert!(h.datalogging_data_available());

    let payload = encode_configure_payload(&get_counter_trigger_config(&COUNTER_RR_RESET));
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::ConfigureDatalog as u8,
        &payload,
    );
    h.receive_data(&req);
    h.process(0); // ProcessAgain, ReleaseOwnership sent to the loop
    assert_eq!(h.data_to_send(), 0);

    // The loop does not run, so the Configure times out.
    let step = REQUEST_MAX_PROCESS_TIME_US; // A tenth of the timeout, in 100ns units
    for _ in 0..20 {
        if h.data_to_send() > 0 {
            break;
        }
        h.process(step);
    }
    assert!(h.data_to_send() > 0, "Configure should time out");
    let resp = pop_response(&mut h);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
    assert!(h.datalogging_ownership_taken());

    // The release arrives late; it must reset the datalogger.
    LOOP1.process_with_time(0);
    h.process(0);
    assert!(!h.datalogging_ownership_taken());
    assert_eq!(
        get_status_state(&mut h),
        DlState::Idle as u8,
        "release without read must reset the datalogger"
    );
    assert!(!h.datalogging_data_available());
    let req = make_request(
        CommandId::DataLogControl as u8,
        DataLogControlSubfn::GetAcquisitionMetadata as u8,
        &[],
    );
    let resp = send_and_recv(&mut h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);

    // Reconfigure: the datalogger goes back to the loop and a new acquisition can be read.
    setup_completed_counter_acquisition(&mut h, &LOOP1, &COUNTER_RR_RESET);
    assert_eq!(h.get_datalogger_state(), DlState::AcquisitionCompleted);
    do_explicit_read_release_flow(&mut h, &LOOP1);
}
