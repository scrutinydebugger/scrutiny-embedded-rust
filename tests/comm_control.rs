use scrutiny_embedded_rust::build_config::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};
use scrutiny_embedded_rust::config::Config;
use scrutiny_embedded_rust::main_handler::MainHandler;
use scrutiny_embedded_rust::protocol::{CommControlSubfn, CommandId, ResponseCode, DISCOVER_MAGIC};
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
