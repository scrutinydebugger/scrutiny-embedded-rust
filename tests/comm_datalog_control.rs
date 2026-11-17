#![cfg(feature = "datalogging")]

use scrutiny_embedded_rust::build_config::DATALOGGING_MAX_SIGNAL;
use scrutiny_embedded_rust::config::Config;
use scrutiny_embedded_rust::datalogging::{State as DlState, MAX_OPERANDS};
use scrutiny_embedded_rust::loop_handler::LoopHandler;
use scrutiny_embedded_rust::main_handler::MainHandler;
use scrutiny_embedded_rust::protocol::{CommandId, CommControlSubfn, DataLogControlSubfn, ResponseCode, CONNECT_MAGIC};
use scrutiny_embedded_rust::tools::crc32;
use scrutiny_embedded_rust::types::{AnyType, RuntimePublishedValue, VariableType};

#[cfg(feature = "protected_regions")]
use scrutiny_embedded_rust::types::AddressRange;

const DATALOG_BUFFER_SIZE: usize = 128;

static mut SOME_VAR_OPERAND1: f32 = 0.0;
static mut SOME_VAR_LOGGED1: f32 = 0.0;

static TEST_RPVS: [RuntimePublishedValue; 1] =
    [RuntimePublishedValue { id: 0x8888, var_type: VariableType::Float32 }];

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
    let req = make_request(CommandId::CommControl as u8, CommControlSubfn::Connect as u8, &CONNECT_MAGIC);
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

// ── Wire format helpers ───────────────────────────────────────────────────────

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
    Var { addr: usize, datatype: u8 },
    VarBit { addr: usize, datatype: u8, bitoffset: u8, bitsize: u8 },
    Rpv(u16),
}

enum DlSignal {
    Time,
    Memory { addr: usize, size: u8 },
    Rpv(u16),
}

fn encode_usize_be(val: usize) -> Vec<u8> {
    let size = core::mem::size_of::<usize>();
    val.to_be_bytes()[8 - size..].to_vec()
}

fn encode_configure_payload(cfg: &DlConfig) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(cfg.loop_id);
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
            DlOperand::VarBit { addr, datatype, bitoffset, bitsize } => {
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
    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::ConfigureDatalog as u8, &payload);
    send_and_recv(h, &req)
}

fn get_status_state(h: &mut MainHandler<'_, '_>) -> u8 {
    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::GetStatus as u8, &[]);
    let resp = send_and_recv(h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    resp_payload(&resp)[0]
}

// Transfer ownership from main to loop and back (Configured state achieved).
// Returns after main has received OwnershipTaken.
fn do_ownership_transfer(h: &mut MainHandler<'_, '_>, loop_handler: &LoopHandler) {
    h.process(0); // sends TakeOwnership IPC
    loop_handler.process(0); // receives TakeOwnership, sends OwnershipTaken
    h.process(0); // receives OwnershipTaken, sets dl.owner
}

// Arm trigger via protocol, then let loop process it.
fn do_arm_trigger(h: &mut MainHandler<'_, '_>, loop_handler: &LoopHandler) {
    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::ArmTrigger as u8, &[]);
    let resp = send_and_recv(h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "ArmTrigger should succeed");
    h.process(0); // sends ArmTrigger IPC
    loop_handler.process(0); // arms the trigger
    loop_handler.process(0); // sends StatusUpdate
    h.process(0); // receives StatusUpdate
}

// Run the loop until AcquisitionCompleted (uses AlwaysTrue trigger).
fn run_until_acquisition(h: &mut MainHandler<'_, '_>, loop_handler: &LoopHandler) {
    for _ in 0..500 {
        loop_handler.process(1);
        h.process(1);
        if get_status_state(h) == DlState::AcquisitionCompleted as u8 {
            return;
        }
    }
    panic!("Acquisition did not complete within 500 iterations");
}

fn make_handler_with_loops<'rx, 'tx>(
    rx: &'rx mut [u8],
    tx: &'tx mut [u8],
    dl_buf_ptr: *mut u8,
    dl_buf_len: usize,
    loops: &'static [LoopHandler],
) -> MainHandler<'rx, 'tx> {
    let mut cfg = Config::new();
    cfg.set_buffers(rx, tx);
    cfg.set_loops(loops);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_callback), None);
    unsafe { cfg.set_datalogger_buffer(dl_buf_ptr, dl_buf_len, None) };
    MainHandler::new(cfg).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_unsupported() {
    // Handler with no dl buffer — DataLogControl returns UnsupportedFeature for all subfns
    static UNSUP_LOOPS: [LoopHandler; 1] = [LoopHandler::fixed_freq("L", 100).with_datalogging()];
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
        assert_eq!(resp_code(&resp), ResponseCode::UnsupportedFeature as u8, "subfn={subfn}");
        h.process(0);
    }
}

#[test]
fn test_get_setup() {
    static LOOPS_GET_SETUP: [LoopHandler; 1] = [LoopHandler::fixed_freq("L", 100).with_datalogging()];
    static mut DL_BUF_GET_SETUP: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 256];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_GET_SETUP).cast(), DATALOG_BUFFER_SIZE, &LOOPS_GET_SETUP);
    connect(&mut h);

    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::GetSetup as u8, &[]);
    let resp = send_and_recv(&mut h, &req);
    h.process(0);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 6, "GetSetup payload: buf_size(4) + encoding(1) + max_signal(1)");
    let buf_size = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
    assert_eq!(buf_size, DATALOG_BUFFER_SIZE as u32);
    assert_eq!(p[4], 0, "Raw encoding = 0");
    assert_eq!(p[5], DATALOGGING_MAX_SIGNAL as u8);
}

#[test]
fn test_configure_valid() {
    static LOOPS_CONFIGURE_VALID: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_CONFIGURE_VALID: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_CONFIGURE_VALID).cast(), DATALOG_BUFFER_SIZE, &LOOPS_CONFIGURE_VALID);
    connect(&mut h);

    let cfg = get_valid_reference_config();
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
}

#[test]
fn test_configure_bad_loop_id() {
    static LOOPS_BAD_LOOP: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_BAD_LOOP: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_BAD_LOOP).cast(), DATALOG_BUFFER_SIZE, &LOOPS_BAD_LOOP);
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.loop_id = 55; // doesn't exist
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_configure_no_datalogging_support() {
    static LOOPS_NO_DL: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100), // no datalogging
    ];
    static mut DL_BUF_NO_DL: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_NO_DL).cast(), DATALOG_BUFFER_SIZE, &LOOPS_NO_DL);
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.loop_id = 2; // datalogging disabled loop
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);
}

#[test]
fn test_configure_item_count_overflow() {
    static LOOPS_OVERFLOW: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_OVERFLOW: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_OVERFLOW).cast(), DATALOG_BUFFER_SIZE, &LOOPS_OVERFLOW);
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
    static LOOPS_OP_OVERFLOW: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_OP_OVERFLOW: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_OP_OVERFLOW).cast(), DATALOG_BUFFER_SIZE, &LOOPS_OP_OVERFLOW);
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
    static LOOPS_OP_MISMATCH: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_OP_MISMATCH: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_OP_MISMATCH).cast(), DATALOG_BUFFER_SIZE, &LOOPS_OP_MISMATCH);
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
    static LOOPS_BAD_DEC: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_BAD_DEC: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_BAD_DEC).cast(), DATALOG_BUFFER_SIZE, &LOOPS_BAD_DEC);
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.decimation = 0; // not supported
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_configure_bad_operands() {
    static LOOPS_BAD_OP: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_BAD_OP: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_BAD_OP).cast(), DATALOG_BUFFER_SIZE, &LOOPS_BAD_OP);
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
        assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8, "i={i} val={bad_val}");
    }
}

#[test]
fn test_configure_operand_bad_rpv() {
    static LOOPS_BAD_RPV: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_BAD_RPV: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_BAD_RPV).cast(), DATALOG_BUFFER_SIZE, &LOOPS_BAD_RPV);
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
    static LOOPS_FORBIDDEN_SIG: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
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
    unsafe { cfg.set_datalogger_buffer(DL_BUF_FORBIDDEN_SIG.as_mut_ptr(), DL_BUF_FORBIDDEN_SIG.len(), None) };
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
    static LOOPS_FORBIDDEN_VAR: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
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
    unsafe { cfg.set_datalogger_buffer(DL_BUF_FORBIDDEN_VAR.as_mut_ptr(), DL_BUF_FORBIDDEN_VAR.len(), None) };
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
    static LOOPS_FORBIDDEN_VARBIT: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
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
    unsafe { cfg.set_datalogger_buffer(DL_BUF_FORBIDDEN_VARBIT.as_mut_ptr(), DL_BUF_FORBIDDEN_VARBIT.len(), None) };
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
    static LOOPS_ALMOST_FORBIDDEN: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
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
    unsafe { cfg.set_datalogger_buffer(DL_BUF_ALMOST_FORBIDDEN.as_mut_ptr(), DL_BUF_ALMOST_FORBIDDEN.len(), None) };
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // VarBit at index 2, type uint16 (2 bytes), touching indices 2..4 — does NOT overlap [4..6]
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
    static LOOPS_UNK_COND: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_UNK_COND: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_UNK_COND).cast(), DATALOG_BUFFER_SIZE, &LOOPS_UNK_COND);
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.trigger_condition = 0xAA;
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_configure_loggable_bad_rpv() {
    static LOOPS_LOGGABLE_BAD_RPV: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_LOGGABLE_BAD_RPV: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_LOGGABLE_BAD_RPV).cast(), DATALOG_BUFFER_SIZE, &LOOPS_LOGGABLE_BAD_RPV);
    connect(&mut h);

    let mut cfg = get_valid_reference_config();
    cfg.signals[0] = DlSignal::Rpv(0x9999); // doesn't exist
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_owner_mechanism() {
    static LOOPS_OWNER: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_OWNER: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let tx_size = DATALOG_BUFFER_SIZE * (8 / 8) + 8 + 9;
    let mut tx = vec![0u8; tx_size];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_OWNER).cast(), DATALOG_BUFFER_SIZE, &LOOPS_OWNER);
    connect(&mut h);

    let loop0 = &LOOPS_OWNER[0];
    let loop1 = &LOOPS_OWNER[1];

    // Configure loop 0 with AlwaysTrue
    let cfg0 = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg0);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    // Transfer ownership to loop0
    do_ownership_transfer(&mut h, loop0);

    // Arm
    do_arm_trigger(&mut h, loop0);

    // Run until acquisition complete
    run_until_acquisition(&mut h, loop0);
    assert_eq!(get_status_state(&mut h), DlState::AcquisitionCompleted as u8);

    // Now re-configure to loop1 — this requires releasing ownership from loop0
    let cfg1 = get_simple_trigger_config(1);
    let payload = encode_configure_payload(&cfg1);
    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::ConfigureDatalog as u8, &payload);

    // Feed the request — it will ProcessAgain until loop0 releases
    h.receive_data(&req);
    h.process(0); // sets pending_release, returns ProcessAgain — no response yet

    // Loop0 must release first
    h.process(0); // sends ReleaseOwnership IPC
    loop0.process(0); // receives ReleaseOwnership, sends OwnershipReleased
    h.process(0); // receives OwnershipReleased (dl.owner = None), retries configure → Ok

    let resp = pop_response(&mut h);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    // Now loop1 gets TakeOwnership
    h.process(0); // sends TakeOwnership IPC to loop1
    loop1.process(0); // receives TakeOwnership, sends OwnershipTaken
    h.process(0); // receives OwnershipTaken

    // Arm loop1
    do_arm_trigger(&mut h, loop1);

    // Run acquisition on loop1
    run_until_acquisition(&mut h, loop1);
    assert_eq!(get_status_state(&mut h), DlState::AcquisitionCompleted as u8);
}

#[test]
fn test_arm_trigger_not_configured() {
    static LOOPS_ARM_NOT_CFG: [LoopHandler; 1] = [LoopHandler::fixed_freq("Loop1", 100).with_datalogging()];
    static mut DL_BUF_ARM_NOT_CFG: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_ARM_NOT_CFG).cast(), DATALOG_BUFFER_SIZE, &LOOPS_ARM_NOT_CFG);
    connect(&mut h);

    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::ArmTrigger as u8, &[]);
    let resp = send_and_recv(&mut h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_arm_disarm_trigger_ok() {
    static LOOPS_ARM_DISARM: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_ARM_DISARM: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_ARM_DISARM).cast(), DATALOG_BUFFER_SIZE, &LOOPS_ARM_DISARM);
    connect(&mut h);

    let loop0 = &LOOPS_ARM_DISARM[0];

    // Configure with the reference config (Equal condition, won't auto-trigger)
    let cfg = get_valid_reference_config();
    let mut cfg_loop0 = cfg;
    cfg_loop0.loop_id = 0;
    let resp = send_configure(&mut h, &cfg_loop0);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);

    // Arm trigger
    let arm_req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::ArmTrigger as u8, &[]);
    let arm_resp = send_and_recv(&mut h, &arm_req);
    h.process(0);
    assert_eq!(resp_code(&arm_resp), ResponseCode::Ok as u8);
    h.process(0); // sends ArmTrigger IPC
    loop0.process(0); // arms the trigger
    loop0.process(0); // sends StatusUpdate
    h.process(0); // receives StatusUpdate
    assert_eq!(get_status_state(&mut h), DlState::Armed as u8);

    // Disarm trigger
    let disarm_req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::DisarmTrigger as u8, &[]);
    let disarm_resp = send_and_recv(&mut h, &disarm_req);
    h.process(0);
    assert_eq!(resp_code(&disarm_resp), ResponseCode::Ok as u8);
    h.process(0); // sends DisarmTrigger IPC
    loop0.process(0); // disarms the trigger
    loop0.process(0); // sends StatusUpdate
    h.process(0); // receives StatusUpdate
    assert_eq!(get_status_state(&mut h), DlState::Configured as u8);
}

#[test]
fn test_get_status() {
    static LOOPS_GET_STATUS: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_GET_STATUS: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    let tx_size = DATALOG_BUFFER_SIZE * (8 / 8) + 8 + 9;
    let mut tx = vec![0u8; tx_size];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_GET_STATUS).cast(), DATALOG_BUFFER_SIZE, &LOOPS_GET_STATUS);
    connect(&mut h);

    let loop0 = &LOOPS_GET_STATUS[0];

    // Initial state: Idle
    assert_eq!(get_status_state(&mut h), DlState::Idle as u8, "initial state should be Idle");

    // Configure loop 0 with AlwaysTrue trigger
    let cfg = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);

    // After ownership transfer: loop sends StatusUpdate when it processes
    loop0.process(0); // send status
    h.process(0); // receive status
    assert_eq!(get_status_state(&mut h), DlState::Configured as u8, "should be Configured");

    // Arm via protocol
    do_arm_trigger(&mut h, loop0);
    assert_eq!(get_status_state(&mut h), DlState::Armed as u8, "should be Armed");

    // Run until acquisition complete
    run_until_acquisition(&mut h, loop0);
    assert_eq!(get_status_state(&mut h), DlState::AcquisitionCompleted as u8, "should be AcquisitionCompleted");
}

#[test]
fn test_get_acquisition_metadata() {
    static LOOPS_META: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_META: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    let tx_size = DATALOG_BUFFER_SIZE * (8 / 8) + 8 + 9;
    let mut tx = vec![0u8; tx_size];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_META).cast(), DATALOG_BUFFER_SIZE, &LOOPS_META);
    connect(&mut h);

    let loop0 = &LOOPS_META[0];

    // Configure
    let cfg = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);

    // Before acquisition: expect FailureToProceed
    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::GetAcquisitionMetadata as u8, &[]);
    let resp = send_and_recv(&mut h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8, "no acquisition yet");

    // Arm and run to completion
    do_arm_trigger(&mut h, loop0);
    run_until_acquisition(&mut h, loop0);

    // After acquisition: expect Ok with metadata
    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::GetAcquisitionMetadata as u8, &[]);
    let resp = send_and_recv(&mut h, &req);
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
    static LOOPS_READ_NO_DATA: [LoopHandler; 1] = [LoopHandler::fixed_freq("Loop1", 100).with_datalogging()];
    static mut DL_BUF_READ_NO_DATA: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_READ_NO_DATA).cast(), DATALOG_BUFFER_SIZE, &LOOPS_READ_NO_DATA);
    connect(&mut h);

    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::ReadAcquisition as u8, &[]);
    let resp = send_and_recv(&mut h, &req);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_read_acquisition_one_transfer() {
    static LOOPS_READ_ONE: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_READ_ONE: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 512];
    // TX must be large enough to hold the entire buffer in one frame
    let tx_size = DATALOG_BUFFER_SIZE * (8 / 8) + 8 + 9 + 64;
    let mut tx = vec![0u8; tx_size];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_READ_ONE).cast(), DATALOG_BUFFER_SIZE, &LOOPS_READ_ONE);
    connect(&mut h);

    let loop0 = &LOOPS_READ_ONE[0];

    let cfg = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);
    do_arm_trigger(&mut h, loop0);
    run_until_acquisition(&mut h, loop0);

    let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::ReadAcquisition as u8, &[]);
    h.receive_data(&req);
    h.process(0);
    let n = h.data_to_send() as usize;
    assert!(n > 0, "should have data to send");
    let mut out = vec![0u8; n];
    h.pop_data(&mut out);
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
    static LOOPS_READ_MULTI: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_READ_MULTI: [u8; 1000] = [0; 1000];
    const DL_BUF_LEN: usize = 1000;
    const TX_LEN: usize = 64;
    let mut rx = [0u8; 512];
    // Small TX buffer to force multiple transfers
    let mut tx = [0u8; TX_LEN];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_READ_MULTI).cast(), 1000, &LOOPS_READ_MULTI);
    connect(&mut h);

    let loop0 = &LOOPS_READ_MULTI[0];

    let cfg = get_simple_trigger_config(0);
    let resp = send_configure(&mut h, &cfg);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    do_ownership_transfer(&mut h, loop0);
    do_arm_trigger(&mut h, loop0);

    // Run until acquisition complete (need more iterations for bigger buffer)
    for _ in 0..5000 {
        loop0.process(1);
        h.process(1);
        if get_status_state(&mut h) == DlState::AcquisitionCompleted as u8 {
            break;
        }
    }
    assert_eq!(get_status_state(&mut h), DlState::AcquisitionCompleted as u8, "acquisition should complete");

    // Read in multiple chunks
    let max_iterations = (DL_BUF_LEN + 4) / (TX_LEN - 9 - 4) + 10;
    let mut all_data: Vec<u8> = Vec::new();
    let mut finished = false;
    let mut last_crc = 0u32;

    for i in 0..max_iterations {
        let req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::ReadAcquisition as u8, &[]);
        let resp = send_and_recv(&mut h, &req);
        h.process(0);
        assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "iteration {i}");

        let payload_len = u16::from_be_bytes([resp[3], resp[4]]) as usize;
        assert!(payload_len >= 4, "iteration {i}: payload too short");

        let chunk_finished = resp[5] != 0;
        let rolling_counter = resp[6];
        assert_eq!(rolling_counter, (i % 256) as u8, "rolling counter mismatch at iteration {i}");

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
    static LOOPS_RESET: [LoopHandler; 3] = [
        LoopHandler::fixed_freq("Loop1", 100).with_datalogging(),
        LoopHandler::variable_freq("Loop2").with_datalogging(),
        LoopHandler::fixed_freq("Loop3", 100),
    ];
    static mut DL_BUF_RESET: [u8; DATALOG_BUFFER_SIZE] = [0; DATALOG_BUFFER_SIZE];
    let mut rx = [0u8; 256];
    let mut tx = [0u8; 128];
    let mut h = make_handler_with_loops(&mut rx, &mut tx, core::ptr::addr_of_mut!(DL_BUF_RESET).cast(), DATALOG_BUFFER_SIZE, &LOOPS_RESET);
    connect(&mut h);

    let loop0 = &LOOPS_RESET[0];

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
    loop0.process(0); // send status
    h.process(0); // receive status
    assert_eq!(get_status_state(&mut h), DlState::Configured as u8);

    // Send Reset
    let reset_req = make_request(CommandId::DataLogControl as u8, DataLogControlSubfn::ResetDatalogger as u8, &[]);
    h.receive_data(&reset_req);
    h.process(0); // sets pending_release, ProcessAgain

    // Loop must release before reset proceeds
    h.process(0); // sends ReleaseOwnership IPC
    loop0.process(0); // receives ReleaseOwnership, sends OwnershipReleased
    h.process(1); // receives OwnershipReleased, retries Reset → Ok
    h.process(0); // allow sending

    let resp = pop_response(&mut h);
    h.process(0);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8, "Reset should succeed");

    // After reset: Idle
    assert_eq!(get_status_state(&mut h), DlState::Idle as u8, "should be Idle after reset");
}
