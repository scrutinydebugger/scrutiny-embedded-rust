use std::sync::Mutex;

use scrutiny_embedded_rust::config::Config;
use scrutiny_embedded_rust::main_handler::MainHandler;
use scrutiny_embedded_rust::protocol::{CommControlSubfn, CommandId, ResponseCode, CONNECT_MAGIC};
use scrutiny_embedded_rust::tools::crc32;

const TX_BUFFER_SIZE: usize = 128;

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

// Inputs seen by `record_callback`: subfunction, request data, response buffer length.
struct CallbackData {
    subfunction: u8,
    request_data: Vec<u8>,
    response_max_len: usize,
}

static CALLBACK1_DATA: Mutex<Option<CallbackData>> = Mutex::new(None);

fn record_callback(subfn: u8, request: &[u8], response: &mut [u8], response_len: &mut u16) {
    let data = CallbackData {
        subfunction: subfn,
        request_data: request.to_vec(),
        response_max_len: response.len(),
    };
    *CALLBACK1_DATA.lock().unwrap() = Some(data);

    response[0] = 0x11;
    response[1] = 0x22;
    response[2] = 0x33;
    response[3] = 0x44;
    *response_len = 4;
}

fn overflow_callback(_subfn: u8, _request: &[u8], response: &mut [u8], response_len: &mut u16) {
    *response_len = response.len() as u16 + 1;
}

// The callback receives the subfunction, the request payload and the full TX buffer; its output is sent back.
#[test]
fn test_command_called() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; TX_BUFFER_SIZE];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_user_command_callback(record_callback);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::UserCommand as u8, 0xAA, &[0x12, 0x34, 0x56]);
    let resp = send_and_recv(&mut h, &req);

    {
        let guard = CALLBACK1_DATA.lock().unwrap();
        let data = guard.as_ref().expect("callback not called");
        assert_eq!(data.subfunction, 0xAA);
        assert_eq!(data.request_data.len(), 3);
        assert_eq!(data.request_data, vec![0x12, 0x34, 0x56]);
        assert_eq!(data.response_max_len, TX_BUFFER_SIZE);
    }

    let mut expected = vec![0x84, 0xAA, 0x00, 0x00, 0x04, 0x11, 0x22, 0x33, 0x44];
    let crc = crc32(&expected, 0);
    expected.extend_from_slice(&crc.to_be_bytes());
    assert_eq!(resp, expected);
}

// A response length larger than the response buffer gives Overflow with no payload.
#[test]
fn test_response_overflow() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; TX_BUFFER_SIZE];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_user_command_callback(overflow_callback);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::UserCommand as u8, 0, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp.len(), 9);
    assert_eq!(resp[0], CommandId::UserCommand as u8 | 0x80);
    assert_eq!(resp[1], 0);
    assert_eq!(resp[2], ResponseCode::Overflow as u8);
    assert_eq!(u16::from_be_bytes([resp[3], resp[4]]), 0);
}

// Without a callback, UserCommand is reported as unsupported.
#[test]
fn test_no_callback() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; TX_BUFFER_SIZE];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::UserCommand as u8, 0, &[]);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp.len(), 9);
    assert_eq!(resp[0], CommandId::UserCommand as u8 | 0x80);
    assert_eq!(resp[1], 0);
    assert_eq!(resp[2], ResponseCode::UnsupportedFeature as u8);
    assert_eq!(u16::from_be_bytes([resp[3], resp[4]]), 0);
}
