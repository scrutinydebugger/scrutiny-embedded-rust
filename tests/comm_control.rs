use scrutiny_embedded_rust::build_config::{
    COMM_HEARTBEAT_TIMEOUT_US, COMM_RX_TIMEOUT_US, PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR,
};
use scrutiny_embedded_rust::config::Config;
use scrutiny_embedded_rust::main_handler::MainHandler;
use scrutiny_embedded_rust::protocol::{CommControlSubfn, CommandId, ResponseCode, CONNECT_MAGIC, DISCOVER_MAGIC};
use scrutiny_embedded_rust::software_id::SOFTWARE_ID;
use scrutiny_embedded_rust::tools::crc32;

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

fn connect(h: &mut MainHandler<'_, '_>) -> u32 {
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
    h.get_session_id()
}

// ── CommControl/Discover ────────────────────────────────────────────────────

#[test]
fn test_discover_response_payload() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    let frame = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Discover as u8,
        &DISCOVER_MAGIC,
    );
    h.receive_data(&frame);
    h.process(0);

    assert!(h.data_to_send() > 0);
    let out = pop_response(&mut h);

    // Response wire format: cmd|0x80, subfn, code, len_hi, len_lo, payload…, crc(4)
    assert_eq!(out[0], CommandId::CommControl as u8 | 0x80);
    assert_eq!(out[1], CommControlSubfn::Discover as u8);
    assert_eq!(out[2], ResponseCode::Ok as u8);

    // Payload layout: major(1), minor(1), software_id(16), name_len(1), name(name_len)
    let payload_len = u16::from_be_bytes([out[3], out[4]]) as usize;
    assert!(payload_len >= 2 + SOFTWARE_ID.len() + 1);
    assert_eq!(out[5], PROTOCOL_VERSION_MAJOR);
    assert_eq!(out[6], PROTOCOL_VERSION_MINOR);
    assert_eq!(&out[7..7 + SOFTWARE_ID.len()], &SOFTWARE_ID);
    let name_len = out[7 + SOFTWARE_ID.len()] as usize;
    assert_eq!(name_len, 0); // no display name set
}

#[test]
fn test_discover_with_display_name() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.display_name = "test-device";
    let mut h = MainHandler::new(cfg).unwrap();

    let frame = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Discover as u8,
        &DISCOVER_MAGIC,
    );
    h.receive_data(&frame);
    h.process(0);

    let out = pop_response(&mut h);
    assert_eq!(out[2], ResponseCode::Ok as u8);

    let name_offset = 5 + 2 + SOFTWARE_ID.len(); // header(5) + major + minor + sw_id
    let name_len = out[name_offset] as usize;
    assert_eq!(name_len, "test-device".len());
    assert_eq!(&out[name_offset + 1..name_offset + 1 + name_len], b"test-device");
}

#[test]
fn test_discover_wrong_magic_while_disconnected_is_silently_dropped() {
    // When not connected, the comm_handler only surfaces valid Discover/Connect requests.
    // A malformed Discover is discarded without sending any response.
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Discover as u8,
        &[0u8; 4],
    );
    h.receive_data(&req);
    h.process(0);
    assert_eq!(h.data_to_send(), 0);
}

#[test]
fn test_discover_wrong_magic_while_connected_returns_invalid_request() {
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    connect(&mut h);

    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Discover as u8,
        &[0u8; 4],
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp[2], ResponseCode::InvalidRequest as u8);
}

// ── CommControl/Connect ─────────────────────────────────────────────────────

#[test]
fn test_connect() {
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    assert!(!h.is_connected());
    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Connect as u8,
        &CONNECT_MAGIC,
    );
    h.receive_data(&req);
    h.process(0);

    let n = h.data_to_send() as usize;
    assert!(n > 0);
    let mut buf = vec![0u8; n];
    h.pop_data(&mut buf);
    h.process(0);

    assert!(h.is_connected());
    assert_eq!(buf[0], CommandId::CommControl as u8 | 0x80);
    assert_eq!(buf[1], CommControlSubfn::Connect as u8);
    assert_eq!(buf[2], ResponseCode::Ok as u8);
    assert_eq!(u16::from_be_bytes([buf[3], buf[4]]), 8);
    assert_eq!(&buf[5..9], &CONNECT_MAGIC);
    let session_id = u32::from_be_bytes([buf[9], buf[10], buf[11], buf[12]]);
    assert_eq!(session_id, h.get_session_id());
}

#[test]
fn test_connect_busy_when_already_connected() {
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    connect(&mut h);
    assert!(h.is_connected());

    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Connect as u8,
        &CONNECT_MAGIC,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp[2], ResponseCode::Busy as u8);
}

// ── CommControl/Heartbeat ───────────────────────────────────────────────────

#[test]
fn test_heartbeat() {
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    let session_id = connect(&mut h);

    for challenge in 0u16..4 {
        let mut hb_data = [0u8; 6];
        hb_data[0..4].copy_from_slice(&session_id.to_be_bytes());
        hb_data[4..6].copy_from_slice(&challenge.to_be_bytes());

        let req = make_request(
            CommandId::CommControl as u8,
            CommControlSubfn::Heartbeat as u8,
            &hb_data,
        );
        h.receive_data(&req);
        // Step less than the heartbeat timeout to verify the session stays alive.
        h.process(COMM_HEARTBEAT_TIMEOUT_US / 2);

        assert!(h.is_connected(), "session dropped at challenge={challenge}");

        let resp = pop_response(&mut h);
        assert_eq!(resp[0], CommandId::CommControl as u8 | 0x80);
        assert_eq!(resp[1], CommControlSubfn::Heartbeat as u8);
        assert_eq!(resp[2], ResponseCode::Ok as u8);
        assert_eq!(u16::from_be_bytes([resp[3], resp[4]]), 6);
        let resp_session_id = u32::from_be_bytes([resp[5], resp[6], resp[7], resp[8]]);
        let resp_challenge = u16::from_be_bytes([resp[9], resp[10]]);
        assert_eq!(resp_session_id, session_id);
        assert_eq!(resp_challenge, !challenge);

        h.process(0);
    }
}

#[test]
fn test_heartbeat_wrong_session_id() {
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    let session_id = connect(&mut h);
    let bad_id = session_id.wrapping_add(1);
    let mut hb_data = [0u8; 6];
    hb_data[0..4].copy_from_slice(&bad_id.to_be_bytes());
    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Heartbeat as u8,
        &hb_data,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp[2], ResponseCode::InvalidRequest as u8);
}

// ── CommControl/GetParams ───────────────────────────────────────────────────

#[test]
fn test_get_params() {
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.max_bitrate = 500_000;
    let mut h = MainHandler::new(cfg).unwrap();

    connect(&mut h);

    let req = make_request(CommandId::CommControl as u8, CommControlSubfn::GetParams as u8, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp[0], CommandId::CommControl as u8 | 0x80);
    assert_eq!(resp[1], CommControlSubfn::GetParams as u8);
    assert_eq!(resp[2], ResponseCode::Ok as u8);
    assert_eq!(u16::from_be_bytes([resp[3], resp[4]]), 18);
    // Payload at resp[5]:
    // [0..2]  rx_buf_size
    // [2..4]  tx_buf_size
    // [4..8]  max_bitrate
    // [8..12] heartbeat_timeout_us
    // [12..16] comm_rx_timeout_us
    // [16]    address_size
    // [17]    char_bit
    assert_eq!(u16::from_be_bytes([resp[5], resp[6]]), 64);
    assert_eq!(u16::from_be_bytes([resp[7], resp[8]]), 64);
    assert_eq!(u32::from_be_bytes([resp[9], resp[10], resp[11], resp[12]]), 500_000);
    assert_eq!(
        u32::from_be_bytes([resp[13], resp[14], resp[15], resp[16]]),
        COMM_HEARTBEAT_TIMEOUT_US
    );
    assert_eq!(
        u32::from_be_bytes([resp[17], resp[18], resp[19], resp[20]]),
        COMM_RX_TIMEOUT_US
    );
    assert_eq!(
        resp[21],
        u8::try_from(core::mem::size_of::<*const u8>()).unwrap_or(u8::MAX)
    );
    assert_eq!(resp[22], 8u8);
}

// ── CommControl/Disconnect ──────────────────────────────────────────────────

#[test]
fn test_disconnect() {
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    let session_id = connect(&mut h);
    assert!(h.is_connected());

    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Disconnect as u8,
        &session_id.to_be_bytes(),
    );
    h.receive_data(&req);
    h.process(0);
    let resp = pop_response(&mut h);
    h.process(0); // disconnect is applied after the response is fully sent

    assert_eq!(resp[0], CommandId::CommControl as u8 | 0x80);
    assert_eq!(resp[1], CommControlSubfn::Disconnect as u8);
    assert_eq!(resp[2], ResponseCode::Ok as u8);
    assert_eq!(u16::from_be_bytes([resp[3], resp[4]]), 0);
    assert!(!h.is_connected());
}

#[test]
fn test_disconnect_wrong_session_id() {
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    let session_id = connect(&mut h);
    let bad_id = session_id.wrapping_add(1);

    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Disconnect as u8,
        &bad_id.to_be_bytes(),
    );
    let resp = send_and_recv(&mut h, &req);
    h.process(0);

    assert_eq!(resp[2], ResponseCode::InvalidRequest as u8);
    assert!(h.is_connected());
}

#[test]
fn test_connect_wrong_magic() {
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();

    let bad_magic = [0x00u8, 0x00, 0x00, 0x00];
    let req = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Connect as u8,
        &bad_magic,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp[2], ResponseCode::InvalidRequest as u8);
    assert!(!h.is_connected());
}
