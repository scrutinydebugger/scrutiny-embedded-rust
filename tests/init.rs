use scrutiny_embedded_rust::config::Config;
use scrutiny_embedded_rust::main_handler::MainHandler;
use scrutiny_embedded_rust::protocol::{CommControlSubfn, CommandId, DISCOVER_MAGIC};
use scrutiny_embedded_rust::tools::crc32;
use scrutiny_embedded_rust::types::{RuntimePublishedValue, VariableType};

fn make_request(cmd: u8, subfn: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = Vec::new();
    frame.push(cmd);
    frame.push(subfn);
    frame.push((len >> 8) as u8);
    frame.push((len & 0xFF) as u8);
    frame.extend_from_slice(payload);
    let crc = crc32(&frame, 0);
    frame.push((crc >> 24) as u8);
    frame.push((crc >> 16) as u8);
    frame.push((crc >> 8) as u8);
    frame.push(crc as u8);
    frame
}

static UNSUPPORTED_RPVS: [RuntimePublishedValue; 5] = [
    RuntimePublishedValue { id: 0x1000, var_type: VariableType::Float8 },
    RuntimePublishedValue { id: 0x1001, var_type: VariableType::Float16 },
    RuntimePublishedValue { id: 0x1002, var_type: VariableType::Cfloat8 },
    RuntimePublishedValue { id: 0x1003, var_type: VariableType::Cfloat16 },
    RuntimePublishedValue { id: 0x1004, var_type: VariableType::Cfloat32 },
];

#[cfg(feature = "support_64bits")]
static UNSUPPORTED_RPVS_64BITS: [RuntimePublishedValue; 1] =
    [RuntimePublishedValue { id: 0x1005, var_type: VariableType::Cfloat64 }];

static SUPPORTED_RPVS: [RuntimePublishedValue; 9] = [
    RuntimePublishedValue { id: 0x2000, var_type: VariableType::Boolean },
    RuntimePublishedValue { id: 0x2001, var_type: VariableType::Boolean8 },
    RuntimePublishedValue { id: 0x2002, var_type: VariableType::Sint8 },
    RuntimePublishedValue { id: 0x2003, var_type: VariableType::Sint16 },
    RuntimePublishedValue { id: 0x2004, var_type: VariableType::Sint32 },
    RuntimePublishedValue { id: 0x2005, var_type: VariableType::Uint8 },
    RuntimePublishedValue { id: 0x2006, var_type: VariableType::Uint16 },
    RuntimePublishedValue { id: 0x2007, var_type: VariableType::Uint32 },
    RuntimePublishedValue { id: 0x2008, var_type: VariableType::Float32 },
];

#[cfg(feature = "support_64bits")]
static SUPPORTED_RPVS_64BITS: [RuntimePublishedValue; 3] = [
    RuntimePublishedValue { id: 0x2009, var_type: VariableType::Sint64 },
    RuntimePublishedValue { id: 0x200A, var_type: VariableType::Uint64 },
    RuntimePublishedValue { id: 0x200B, var_type: VariableType::Float64 },
];

static MIXED_RPVS: [RuntimePublishedValue; 2] = [
    RuntimePublishedValue { id: 0x3000, var_type: VariableType::Uint32 },
    RuntimePublishedValue { id: 0x3001, var_type: VariableType::Float16 },
];

// Builds a handler with the given RPVs and returns (enabled, number of bytes answered to a Discover).
fn init_and_discover(rpvs: &'static [RuntimePublishedValue]) -> (bool, u16) {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(rpvs, None, None);
    let mut h = MainHandler::new(cfg).unwrap();
    let enabled = h.is_enabled();

    let req = make_request(CommandId::CommControl as u8, CommControlSubfn::Discover as u8, &DISCOVER_MAGIC);
    h.receive_data(&req);
    h.process(0);
    (enabled, h.data_to_send())
}

fn assert_each_rejected(rpvs: &'static [RuntimePublishedValue]) {
    for rpv in rpvs {
        let (enabled, response_size) = init_and_discover(core::slice::from_ref(rpv));
        assert!(!enabled, "RPV type {:?} should disable the handler", rpv.var_type);
        assert_eq!(response_size, 0, "RPV type {:?} should make the handler inert", rpv.var_type);
    }
}

// An RPV with an unsupported type must leave the handler disabled and inert.
#[test]
fn init_rejects_unsupported_rpv_type() {
    assert_each_rejected(&UNSUPPORTED_RPVS);
    #[cfg(feature = "support_64bits")]
    assert_each_rejected(&UNSUPPORTED_RPVS_64BITS);
}

// RPVs of every supported type keep the handler enabled and answering.
#[test]
fn init_accepts_supported_rpv_types() {
    let (enabled, response_size) = init_and_discover(&SUPPORTED_RPVS);
    assert!(enabled);
    assert!(response_size > 0);

    #[cfg(feature = "support_64bits")]
    {
        let (enabled, response_size) = init_and_discover(&SUPPORTED_RPVS_64BITS);
        assert!(enabled);
        assert!(response_size > 0);
    }
}

// A single unsupported RPV among valid ones disables the handler.
#[test]
fn init_rejects_mixed_rpv_types() {
    let (enabled, response_size) = init_and_discover(&MIXED_RPVS);
    assert!(!enabled);
    assert_eq!(response_size, 0);
}
