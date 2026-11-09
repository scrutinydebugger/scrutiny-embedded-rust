use scrutiny_embedded_rust::protocol::{
    CommControlSubfn, CommHandler, CommandId, RxError, TxError, DISCOVER_MAGIC, HEARTBEAT_TIMEOUT_100NS,
    RX_TIMEOUT_100NS,
};
use scrutiny_embedded_rust::timebase::Timebase;
use scrutiny_embedded_rust::tools::crc32;

/// Builds a request frame: cmd(1) + subfn(1) + len_hi(1) + len_lo(1) + data + crc32(4).
fn make_request(cmd: u8, subfn: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = Vec::new();
    frame.push(cmd);
    frame.push(subfn);
    frame.push((len >> 8) as u8);
    frame.push((len & 0xFF) as u8);
    frame.extend_from_slice(payload);
    let crc = crc32(&frame, frame.len(), 0);
    frame.push((crc >> 24) as u8);
    frame.push((crc >> 16) as u8);
    frame.push((crc >> 8) as u8);
    frame.push(crc as u8);
    frame
}

/// Builds the expected wire bytes for a response.
fn make_expected_response(cmd: u8, subfn: u8, code: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let cmd_out = cmd | 0x80;
    let mut frame = Vec::new();
    frame.push(cmd_out);
    frame.push(subfn);
    frame.push(code);
    frame.push((len >> 8) as u8);
    frame.push((len & 0xFF) as u8);
    frame.extend_from_slice(payload);
    let crc = crc32(&frame, frame.len(), 0);
    frame.push((crc >> 24) as u8);
    frame.push((crc >> 16) as u8);
    frame.push((crc >> 8) as u8);
    frame.push(crc as u8);
    frame
}

fn make_comm_buffers() -> ([u8; 128], [u8; 128]) {
    ([0u8; 128], [0u8; 128])
}

#[test]
fn test_consecutive_send() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);

    let payload = [0x11u8, 0x22, 0x33];
    let expected = make_expected_response(0x01, 0x02, 0x03, &payload);

    assert!(!comm.transmitting());

    // First send
    let resp = comm.prepare_response();
    resp.command_id = 0x01;
    resp.subfunction_id = 0x02;
    resp.response_code = 0x03;
    resp.data_length = 3;
    resp.payload()[..3].copy_from_slice(&payload);
    assert!(comm.send_response());
    assert!(comm.transmitting());

    // Second send while busy must fail
    assert!(!comm.send_response());
    assert_eq!(comm.get_tx_error(), TxError::Busy);

    let n = comm.data_to_send() as usize;
    assert_eq!(n, expected.len());

    let mut out = vec![0u8; n];
    comm.pop_data(&mut out, &tb);
    assert_eq!(out, expected);
    assert_eq!(comm.data_to_send(), 0);
    assert!(!comm.transmitting());

    // Second send after pop must succeed
    let resp = comm.prepare_response();
    resp.command_id = 0x01;
    resp.subfunction_id = 0x02;
    resp.response_code = 0x03;
    resp.data_length = 3;
    resp.payload()[..3].copy_from_slice(&payload);
    assert!(comm.send_response());
    assert!(comm.transmitting());

    let n = comm.data_to_send() as usize;
    assert_eq!(n, expected.len());
    let mut out = vec![0u8; n];
    comm.pop_data(&mut out, &tb);
    assert_eq!(out, expected);
}

#[test]
// Port of TestHeartbeatTimeoutExpire
fn test_heartbeat_timeout_expire() {
    let (mut rx, mut tx) = make_comm_buffers();
    let mut tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);

    assert!(!comm.is_connected());
    comm.connect(&tb);
    assert!(comm.is_connected());

    tb.step(HEARTBEAT_TIMEOUT_100NS - 1);
    comm.process(&tb);
    assert!(comm.is_connected());

    comm.heartbeat(10, &tb); // first challenge is always accepted
    tb.step(HEARTBEAT_TIMEOUT_100NS - 1);
    comm.process(&tb);
    assert!(comm.is_connected());

    tb.step(HEARTBEAT_TIMEOUT_100NS - 1);
    comm.heartbeat(11, &tb);
    comm.process(&tb);
    assert!(comm.is_connected());

    tb.step(HEARTBEAT_TIMEOUT_100NS - 1);
    comm.heartbeat(11, &tb); // repeated challenge — ignored
    comm.process(&tb);
    assert!(comm.is_connected());

    tb.step(1); // push past the deadline
    comm.process(&tb);
    assert!(!comm.is_connected());
}

#[test]
// Port of TestConnectDisconnectBehaviour
fn test_connect_disconnect() {
    let (mut rx, mut tx) = make_comm_buffers();
    let mut tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);

    assert!(!comm.is_connected());
    comm.connect(&tb);
    assert!(comm.is_connected());
    let first_id = comm.get_session_id();

    comm.disconnect(&tb);
    assert!(!comm.is_connected());
    comm.connect(&tb);
    assert_ne!(comm.get_session_id(), first_id);

    // A complete valid request is received after connect
    let frame = make_request(0x01, 0x01, &[]);
    assert!(!comm.request_received());
    comm.receive_data(&frame, &tb);
    comm.process(&tb);
    assert!(comm.request_received());

    // disconnect clears the pending request
    comm.disconnect(&tb);
    assert!(!comm.request_received());

    // Partial frame followed by disconnect then reconnect: the partial bytes
    // must not carry over into the new session
    comm.connect(&tb);
    let frame = make_request(0x01, 0x01, &[]);
    comm.receive_data(&frame[..frame.len() - 1], &tb);
    comm.process(&tb);
    assert!(!comm.request_received());

    comm.disconnect(&tb);
    comm.connect(&tb);
    tb.step(1); // small clock advance so the new session has a fresh timestamp
                // Supply only the last byte of the old frame — must not complete a request
    comm.receive_data(&frame[frame.len() - 1..], &tb);
    assert!(!comm.request_received());
}

#[test]
fn test_rx_framing_valid() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);
    comm.connect(&tb);

    let payload = [0xAAu8, 0xBB, 0xCC];
    let frame = make_request(0x01, 0x02, &payload);
    comm.receive_data(&frame, &tb);

    assert!(comm.request_received());
    let req = comm.get_request().expect("request must be present");
    assert_eq!(req.command_id, 0x01);
    assert_eq!(req.subfunction_id, 0x02);
    assert_eq!(req.data(), &payload);
}

#[test]
fn test_rx_bad_crc_no_request() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);
    comm.connect(&tb);

    let mut frame = make_request(0x01, 0x01, &[]);
    *frame.last_mut().unwrap() ^= 0xFF;
    comm.receive_data(&frame, &tb);

    assert!(!comm.request_received());
}

#[test]
fn test_rx_invalid_command_bit() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);
    comm.connect(&tb);

    // Bit 7 set means it's a response frame, not a valid request
    let frame = [0x81u8, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    comm.receive_data(&frame, &tb);

    assert!(!comm.request_received());
    assert_eq!(comm.get_rx_error(), RxError::InvalidCommand);
}

#[test]
fn test_rx_timeout_resets_fsm() {
    let (mut rx, mut tx) = make_comm_buffers();
    let mut tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);
    comm.connect(&tb);

    let frame = make_request(0x01, 0x01, &[]);
    // Send the first four bytes (header only, no CRC yet)
    comm.receive_data(&frame[..4], &tb);

    // Gap longer than the inter-byte timeout
    tb.step(RX_TIMEOUT_100NS + 1);

    // The remaining bytes now arrive as a fresh start, so the joined bytes
    // do not form a valid frame
    comm.receive_data(&frame[4..], &tb);
    assert!(!comm.request_received());
}

#[test]
fn test_rx_overflow() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);
    comm.connect(&tb);

    // Payload larger than the 128-byte RX buffer
    let big_payload = vec![0u8; 200];
    let frame = make_request(0x01, 0x01, &big_payload);
    comm.receive_data(&frame, &tb);

    assert!(!comm.request_received());
    assert_eq!(comm.get_rx_error(), RxError::Overflow);
}

#[test]
fn test_rx_chunked_delivery() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);
    comm.connect(&tb);

    let payload = [1u8, 2, 3, 4, 5];
    let frame = make_request(0x01, 0x01, &payload);

    for byte in &frame {
        comm.receive_data(core::slice::from_ref(byte), &tb);
    }

    assert!(comm.request_received());
    assert_eq!(comm.get_request().unwrap().data(), &payload);
}

#[test]
fn test_discover_without_session() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);
    // No connect() — session inactive

    let frame = make_request(
        CommandId::CommControl as u8,
        CommControlSubfn::Discover as u8,
        &DISCOVER_MAGIC,
    );
    comm.receive_data(&frame, &tb);

    assert!(comm.request_received());
    let req = comm.get_request().unwrap();
    assert_eq!(req.command_id, CommandId::CommControl as u8);
    assert_eq!(req.subfunction_id, CommControlSubfn::Discover as u8);
}

#[test]
fn test_connect_request_without_session() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);

    let frame = make_request(CommandId::CommControl as u8, CommControlSubfn::Connect as u8, &[]);
    comm.receive_data(&frame, &tb);
    assert!(comm.request_received());
}

#[test]
fn test_non_discover_dropped_without_session() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);
    // No connect()

    let frame = make_request(CommandId::GetInfo as u8, 0x01, &[]);
    comm.receive_data(&frame, &tb);

    assert!(!comm.request_received());
}

#[test]
fn test_pop_data_partial_reads() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);

    let payload = [0x11u8, 0x22, 0x33];
    let expected = make_expected_response(0x01, 0x02, 0x00, &payload);

    let resp = comm.prepare_response();
    resp.command_id = 0x01;
    resp.subfunction_id = 0x02;
    resp.response_code = 0x00;
    resp.data_length = 3;
    resp.payload()[..3].copy_from_slice(&payload);
    assert!(comm.send_response());

    let mut collected = Vec::new();
    let mut tmp = [0u8; 3];
    while comm.data_to_send() > 0 {
        let n = comm.pop_data(&mut tmp, &tb);
        collected.extend_from_slice(&tmp[..n as usize]);
    }
    assert_eq!(collected, expected);
}

#[test]
fn test_disabled_handler_rejects_rx_and_tx() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&mut rx, &mut tx, 0);
    comm.disable(&tb);

    assert!(!comm.is_enabled());

    let frame = make_request(0x01, 0x01, &[]);
    comm.receive_data(&frame, &tb);
    assert_eq!(comm.get_rx_error(), RxError::Disabled);

    let resp = comm.prepare_response();
    resp.command_id = 0x01;
    resp.subfunction_id = 0x01;
    resp.response_code = 0x00;
    resp.data_length = 1;
    resp.payload()[0] = 0xFF;
    assert!(!comm.send_response());
    assert_eq!(comm.get_tx_error(), TxError::Disabled);
}
