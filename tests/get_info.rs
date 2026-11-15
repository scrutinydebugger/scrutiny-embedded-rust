use scrutiny_embedded_rust::build_config::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};
use scrutiny_embedded_rust::config::Config;
use scrutiny_embedded_rust::loop_handler::{LoopHandler, LoopType};
use scrutiny_embedded_rust::main_handler::MainHandler;
use scrutiny_embedded_rust::protocol::{
    CommandId, CommControlSubfn, GetInfoSubfn, ResponseCode, CONNECT_MAGIC,
};
use scrutiny_embedded_rust::software_id::SOFTWARE_ID;
use scrutiny_embedded_rust::tools::crc32;
use scrutiny_embedded_rust::types::{AnyType, RuntimePublishedValue, VariableType};

#[cfg(feature = "protected_regions")]
use scrutiny_embedded_rust::types::AddressRange;

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

/// Returns the response code byte from a wire response.
fn resp_code(resp: &[u8]) -> u8 {
    resp[2]
}

/// Returns the payload slice from a wire response (excludes header and CRC).
fn resp_payload(resp: &[u8]) -> &[u8] {
    let len = u16::from_be_bytes([resp[3], resp[4]]) as usize;
    &resp[5..5 + len]
}

// ── GetInfo/GetProtocolVersion ───────────────────────────────────────────────

#[test]
fn test_protocol_version() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetProtocolVersion as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp[0], CommandId::GetInfo as u8 | 0x80);
    assert_eq!(resp[1], GetInfoSubfn::GetProtocolVersion as u8);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 2);
    assert_eq!(p[0], PROTOCOL_VERSION_MAJOR);
    assert_eq!(p[1], PROTOCOL_VERSION_MINOR);
}

// ── GetInfo/GetSoftwareId ────────────────────────────────────────────────────

#[test]
fn test_software_id() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetSoftwareId as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    assert_eq!(resp_payload(&resp), &SOFTWARE_ID[..]);
}

// ── GetInfo/GetSupportedFeatures ─────────────────────────────────────────────

#[test]
fn test_supported_features_memory_write_enabled() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.memory_write_enable = true;
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetSupportedFeatures as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 1);
    // bit 7 = memory write, bit 6 = datalogging (always 0, not yet implemented),
    // bit 5 = user command, bit 4 = 64-bit support
    let mut expected: u8 = 0x80;
    #[cfg(feature = "support_64bits")]
    {
        expected |= 0x10;
    }
    assert_eq!(p[0], expected);
}

#[test]
fn test_supported_features_memory_write_disabled() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.memory_write_enable = false;
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetSupportedFeatures as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 1);
    let mut expected: u8 = 0;
    #[cfg(feature = "support_64bits")]
    {
        expected |= 0x10;
    }
    assert_eq!(p[0], expected);
}

#[test]
fn test_supported_features_with_user_command() {
    fn dummy_user_cmd(_subfn: u8, _req: &[u8], _resp: &mut [u8], _len: &mut u16) {}

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.memory_write_enable = false;
    cfg.set_user_command_callback(dummy_user_cmd);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetSupportedFeatures as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 1);
    assert_ne!(p[0] & 0x20, 0, "user command bit (0x20) must be set");
}

// ── GetInfo/GetSpecialMemoryRegionCount ───────────────────────────────────────

#[test]
fn test_special_memory_region_count_no_regions() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetSpecialMemoryRegionCount as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 2);
    assert_eq!(p[0], 0, "readonly count should be 0");
    assert_eq!(p[1], 0, "forbidden count should be 0");
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_special_memory_region_count_with_regions() {
    static READONLY: [AddressRange; 2] = [
        AddressRange { start: 0x1000 as *const u8, end: 0x1004 as *const u8 },
        AddressRange { start: 0x1001 as *const u8, end: 0x1005 as *const u8 },
    ];
    static FORBIDDEN: [AddressRange; 1] =
        [AddressRange { start: 0x1002 as *const u8, end: 0x1006 as *const u8 }];

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_readonly_address_range(&READONLY);
    cfg.set_forbidden_address_range(&FORBIDDEN);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetSpecialMemoryRegionCount as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 2);
    assert_eq!(p[0], 2, "readonly count should be 2");
    assert_eq!(p[1], 1, "forbidden count should be 1");
}

// ── GetInfo/GetSpecialMemoryLocation ─────────────────────────────────────────

#[cfg(feature = "protected_regions")]
#[test]
fn test_special_memory_region_location() {
    static READONLY: [AddressRange; 2] = [
        AddressRange { start: 0x1000 as *const u8, end: 0x1004 as *const u8 },
        AddressRange { start: 0x1001 as *const u8, end: 0x1005 as *const u8 },
    ];
    static FORBIDDEN: [AddressRange; 1] =
        [AddressRange { start: 0x1002 as *const u8, end: 0x1006 as *const u8 }];

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_readonly_address_range(&READONLY);
    cfg.set_forbidden_address_range(&FORBIDDEN);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let addr_size = core::mem::size_of::<usize>();

    // (region_type, index, expected_start, expected_end)
    let cases: [(u8, u8, usize, usize); 3] = [
        (0, 0, 0x1000, 0x1004), // readonly[0]
        (0, 1, 0x1001, 0x1005), // readonly[1]
        (1, 0, 0x1002, 0x1006), // forbidden[0]
    ];

    for (region_type, region_index, expected_start, expected_end) in &cases {
        let req = make_request(
            CommandId::GetInfo as u8,
            GetInfoSubfn::GetSpecialMemoryLocation as u8,
            &[*region_type, *region_index],
        );
        let resp = send_and_recv(&mut h, &req);
        h.process(0);

        assert_eq!(
            resp_code(&resp),
            ResponseCode::Ok as u8,
            "type={region_type} index={region_index}"
        );
        let p = resp_payload(&resp);
        assert_eq!(p.len(), 2 + 2 * addr_size);
        assert_eq!(p[0], *region_type);
        assert_eq!(p[1], *region_index);

        let start_bytes = expected_start.to_be_bytes();
        let end_bytes = expected_end.to_be_bytes();
        assert_eq!(&p[2..2 + addr_size], &start_bytes[..], "type={region_type} index={region_index} start mismatch");
        assert_eq!(&p[2 + addr_size..2 + 2 * addr_size], &end_bytes[..], "type={region_type} index={region_index} end mismatch");
    }
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_special_memory_region_location_wrong_index() {
    static READONLY: [AddressRange; 2] = [
        AddressRange { start: 0x1000 as *const u8, end: 0x1004 as *const u8 },
        AddressRange { start: 0x1001 as *const u8, end: 0x1005 as *const u8 },
    ];
    static FORBIDDEN: [AddressRange; 1] =
        [AddressRange { start: 0x1002 as *const u8, end: 0x1006 as *const u8 }];

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_readonly_address_range(&READONLY);
    cfg.set_forbidden_address_range(&FORBIDDEN);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Index 2 is out of bounds for readonly (only indices 0 and 1 exist).
    let req = make_request(
        CommandId::GetInfo as u8,
        GetInfoSubfn::GetSpecialMemoryLocation as u8,
        &[0, 2],
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

// ── GetInfo/GetRuntimePublishedValuesCount ────────────────────────────────────

fn rpv_read_cb(_rpv: RuntimePublishedValue, _out: &mut AnyType) -> bool {
    true
}
fn rpv_write_cb(_rpv: RuntimePublishedValue, _in: &AnyType) -> bool {
    true
}

#[test]
fn test_rpv_count() {
    static RPVS: [RuntimePublishedValue; 3] = [
        RuntimePublishedValue { id: 0x1122, var_type: VariableType::Uint32 },
        RuntimePublishedValue { id: 0x3344, var_type: VariableType::Float32 },
        RuntimePublishedValue { id: 0x5566, var_type: VariableType::Uint16 },
    ];

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&RPVS, Some(rpv_read_cb), Some(rpv_write_cb));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(
        CommandId::GetInfo as u8,
        GetInfoSubfn::GetRuntimePublishedValuesCount as u8,
        &[],
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 2);
    assert_eq!(u16::from_be_bytes([p[0], p[1]]), 3);
}

// ── GetInfo/GetRuntimePublishedValuesDefinition ───────────────────────────────

#[test]
fn test_rpv_definition() {
    static RPVS: [RuntimePublishedValue; 4] = [
        RuntimePublishedValue { id: 0x1122, var_type: VariableType::Uint32 },
        RuntimePublishedValue { id: 0x3344, var_type: VariableType::Float32 },
        RuntimePublishedValue { id: 0x5566, var_type: VariableType::Uint16 },
        // Boolean is mapped to Boolean8 by the protocol encoder.
        RuntimePublishedValue { id: 0x7788, var_type: VariableType::Boolean },
    ];

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&RPVS, Some(rpv_read_cb), Some(rpv_write_cb));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Request: start_index=1, count=3 — reads rpvs[1..4]
    let req = make_request(
        CommandId::GetInfo as u8,
        GetInfoSubfn::GetRuntimePublishedValuesDefinition as u8,
        &[0x00, 0x01, 0x00, 0x03],
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 3 * 3, "3 entries × 3 bytes each");

    // rpvs[1]: id=0x3344, type=Float32
    assert_eq!(p[0], 0x33);
    assert_eq!(p[1], 0x44);
    assert_eq!(p[2], VariableType::Float32 as u8);

    // rpvs[2]: id=0x5566, type=Uint16
    assert_eq!(p[3], 0x55);
    assert_eq!(p[4], 0x66);
    assert_eq!(p[5], VariableType::Uint16 as u8);

    // rpvs[3]: id=0x7788, Boolean → Boolean8 (protocol maps it to the platform boolean size)
    assert_eq!(p[6], 0x77);
    assert_eq!(p[7], 0x88);
    assert_eq!(p[8], VariableType::Boolean8 as u8);
}

#[test]
fn test_rpv_definition_overflow() {
    static RPVS: [RuntimePublishedValue; 3] = [
        RuntimePublishedValue { id: 0x1122, var_type: VariableType::Uint32 },
        RuntimePublishedValue { id: 0x3344, var_type: VariableType::Float32 },
        RuntimePublishedValue { id: 0x5566, var_type: VariableType::Uint16 },
    ];

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&RPVS, Some(rpv_read_cb), Some(rpv_write_cb));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // start=1, count=3: would read indices 1,2,3 but only 0,1,2 exist → overflow
    let req = make_request(
        CommandId::GetInfo as u8,
        GetInfoSubfn::GetRuntimePublishedValuesDefinition as u8,
        &[0x00, 0x01, 0x00, 0x03],
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

// ── GetInfo/GetLoopCount ──────────────────────────────────────────────────────

static TEST_LOOPS: [LoopHandler; 3] = [
    LoopHandler::fixed_freq("Loop1", 0x12345678),
    LoopHandler::variable_freq("Loop2"),
    LoopHandler::fixed_freq("Loop3", 100),
];

#[test]
fn test_loop_count() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&TEST_LOOPS);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetLoopCount as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0], 3);
}

#[test]
fn test_loop_count_no_loops() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetLoopCount as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0], 0);
}

// ── GetInfo/GetLoopDefinition ─────────────────────────────────────────────────

#[test]
fn test_loop_definition_fixed_freq() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&TEST_LOOPS);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetLoopDefinition as u8, &[0]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    // Wire: loop_id(1) + type(1) + attributes(1) + timestep_100ns(4) + name_len(1) + name
    assert_eq!(p[0], 0, "loop ID");
    assert_eq!(p[1], LoopType::FixedFreq as u8, "loop type");
    assert_eq!(p[2], 0, "attributes (datalogging not implemented)");
    let timestep = u32::from_be_bytes([p[3], p[4], p[5], p[6]]);
    assert_eq!(timestep, 0x12345678u32, "timestep_100ns");
    let name_len = p[7] as usize;
    assert_eq!(name_len, 5, "name length");
    assert_eq!(&p[8..8 + name_len], b"Loop1");
}

#[test]
fn test_loop_definition_variable_freq() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&TEST_LOOPS);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetLoopDefinition as u8, &[1]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    // Wire: loop_id(1) + type(1) + attributes(1) + name_len(1) + name  (no timestep for variable freq)
    assert_eq!(p[0], 1, "loop ID");
    assert_eq!(p[1], LoopType::VariableFreq as u8, "loop type");
    assert_eq!(p[2], 0, "attributes");
    let name_len = p[3] as usize;
    assert_eq!(name_len, 5, "name length");
    assert_eq!(&p[4..4 + name_len], b"Loop2");
}

#[test]
fn test_loop_definition_fixed_freq_datalogging_disabled() {
    // Loop3 has datalogging disabled in C++. In Rust datalogging is not implemented yet,
    // so the attributes byte is always 0 regardless.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&TEST_LOOPS);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetLoopDefinition as u8, &[2]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    assert_eq!(p[0], 2, "loop ID");
    assert_eq!(p[1], LoopType::FixedFreq as u8, "loop type");
    assert_eq!(p[2], 0, "attributes (datalogging disabled)");
    let timestep = u32::from_be_bytes([p[3], p[4], p[5], p[6]]);
    assert_eq!(timestep, 100u32, "timestep_100ns");
    let name_len = p[7] as usize;
    assert_eq!(name_len, 5, "name length");
    assert_eq!(&p[8..8 + name_len], b"Loop3");
}

#[test]
fn test_loop_definition_oob_id() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_loops(&TEST_LOOPS);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // ID=3 does not exist (loops are 0, 1, 2).
    let req = make_request(CommandId::GetInfo as u8, GetInfoSubfn::GetLoopDefinition as u8, &[3]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}
