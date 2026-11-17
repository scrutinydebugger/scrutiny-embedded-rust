use scrutiny_embedded_rust::protocol::{CommHandler, TxError};
use scrutiny_embedded_rust::timebase::Timebase;
use scrutiny_embedded_rust::tools::crc32;

const TX_BUFFER_SIZE: usize = 128;
// cmd(1) + subfn(1) + code(1) + len(2) + crc(4)
const RESPONSE_OVERHEAD: usize = 9;

/// Builds a request frame: cmd(1) + subfn(1) + len_hi(1) + len_lo(1) + data + crc32(4).
fn make_request(cmd: u8, subfn: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = vec![cmd, subfn, (len >> 8) as u8, (len & 0xFF) as u8];
    frame.extend_from_slice(payload);
    let crc = crc32(&frame, 0);
    frame.push((crc >> 24) as u8);
    frame.push((crc >> 16) as u8);
    frame.push((crc >> 8) as u8);
    frame.push(crc as u8);
    frame
}

/// Builds the expected wire bytes for a response.
fn make_expected_response(cmd: u8, subfn: u8, code: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = vec![cmd | 0x80, subfn, code, (len >> 8) as u8, (len & 0xFF) as u8];
    frame.extend_from_slice(payload);
    let crc = crc32(&frame, 0);
    frame.push((crc >> 24) as u8);
    frame.push((crc >> 16) as u8);
    frame.push((crc >> 8) as u8);
    frame.push(crc as u8);
    frame
}

fn make_comm_buffers() -> ([u8; 128], [u8; TX_BUFFER_SIZE]) {
    ([0u8; 128], [0u8; TX_BUFFER_SIZE])
}

/// Sets the response header fields used by every C++ TestTxParsing test.
fn set_response_header(comm: &mut CommHandler, data_length: u16) {
    let resp = comm.active_response_mut();
    resp.command_id = 0x81;
    resp.subfunction_id = 0x02;
    resp.response_code = 0x03;
    resp.data_length = data_length;
}

/// Queues the 3-byte response shared by the C++ read tests and returns the expected frame.
fn send_small_response(comm: &mut CommHandler, tx: &mut [u8]) -> Vec<u8> {
    let payload = [0x11u8, 0x22, 0x33];
    set_response_header(comm, 3);
    tx[..3].copy_from_slice(&payload);
    assert!(comm.send_response(tx));
    assert_eq!(comm.get_tx_error(), TxError::None);
    make_expected_response(0x81, 0x02, 0x03, &payload)
}

#[test]
// Port of TestReadAllData
fn test_read_all_data() {
    let (mut rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let expected = send_small_response(&mut comm, &mut tx);
    assert_eq!(expected.len(), 12);
    assert_eq!(&expected[..8], &[0x81, 2, 3, 0, 3, 0x11, 0x22, 0x33]);

    let mut buf = [0u8; 256];
    let n_to_read = comm.data_to_send();
    assert!(n_to_read > 0);
    assert!((n_to_read as usize) < buf.len());
    assert_eq!(n_to_read as usize, expected.len());

    let nread = comm.pop_data(&mut buf[..n_to_read as usize], &tx, &tb);
    assert_eq!(nread, n_to_read);
    assert_eq!(&buf[..expected.len()], expected.as_slice());

    // Transmission complete: the handler is ready for the next request
    assert_eq!(comm.data_to_send(), 0);
    assert!(!comm.transmitting());
    let frame = make_request(0x01, 0x01, &[]);
    comm.receive_data(&frame, &mut rx, &tb);
    assert!(comm.request_received());
}

#[test]
// Port of TestReadBytePerByte
fn test_read_byte_per_byte() {
    let (rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let expected = send_small_response(&mut comm, &mut tx);

    let mut buf = [0u8; 256];
    let n_to_read = comm.data_to_send();
    assert!(n_to_read > 0);
    assert!((n_to_read as usize) < buf.len());
    assert_eq!(n_to_read as usize, expected.len());

    for i in 0..n_to_read as usize {
        let nread = comm.pop_data(&mut buf[i..i + 1], &tx, &tb);
        assert_eq!(nread, 1);
    }

    assert_eq!(&buf[..expected.len()], expected.as_slice());
}

#[test]
// Port of TestReadByChunk
fn test_read_by_chunk() {
    let (rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let expected = send_small_response(&mut comm, &mut tx);

    let mut buf = [0u8; 256];
    let n_to_read = comm.data_to_send();
    assert_eq!(n_to_read, 12);

    let chunks = [3usize, 6, 3];
    let mut index = 0usize;
    for chunk in chunks {
        let nread = comm.pop_data(&mut buf[index..index + chunk], &tx, &tb);
        assert_eq!(nread as usize, chunk);
        index += chunk;
    }

    assert_eq!(&buf[..expected.len()], expected.as_slice());
}

#[test]
// Port of TestReadMoreThanAvailable
fn test_read_more_than_available() {
    let (rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let expected = send_small_response(&mut comm, &mut tx);

    let mut buf = [0u8; 256];
    let n_to_read = comm.data_to_send();
    let nread = comm.pop_data(&mut buf[..n_to_read as usize + 10], &tx, &tb);
    assert_eq!(nread, n_to_read);

    assert_eq!(&buf[..expected.len()], expected.as_slice());
}

#[test]
// Port of TestSendsOverflow
fn test_sends_overflow() {
    let (rx, tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    set_response_header(&mut comm, (TX_BUFFER_SIZE + 1) as u16);
    assert!(!comm.send_response(&tx));

    assert_eq!(comm.data_to_send(), 0);
    assert_eq!(comm.get_tx_error(), TxError::Overflow);

    let mut buf = [0u8; 256];
    assert_eq!(comm.pop_data(&mut buf, &tx, &tb), 0);
}

#[test]
// Port of TestSendsFullBuffer
fn test_sends_full_buffer() {
    let (rx, mut tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let datalen = TX_BUFFER_SIZE;
    set_response_header(&mut comm, datalen as u16);
    for (i, byte) in tx.iter_mut().enumerate() {
        *byte = i as u8;
    }

    assert!(comm.send_response(&tx));
    assert_eq!(comm.get_tx_error(), TxError::None);

    let n_to_read = comm.data_to_send();
    assert_eq!(n_to_read as usize, datalen + RESPONSE_OVERHEAD);

    let mut buf = [0u8; TX_BUFFER_SIZE + RESPONSE_OVERHEAD];
    let nread = comm.pop_data(&mut buf[..n_to_read as usize], &tx, &tb);
    assert_eq!(nread, n_to_read);

    assert_eq!(&buf[5..5 + datalen], &tx[..datalen]);
    let expected = make_expected_response(0x81, 0x02, 0x03, &tx);
    assert_eq!(&buf[..], expected.as_slice());
}

#[test]
// Port of TestSendsOverflowRestart
fn test_sends_overflow_restart() {
    let (rx, tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let datalen = TX_BUFFER_SIZE;
    set_response_header(&mut comm, (datalen + 1) as u16);
    assert!(!comm.send_response(&tx));
    assert_eq!(comm.data_to_send(), 0);
    assert_eq!(comm.get_tx_error(), TxError::Overflow);

    comm.active_response_mut().data_length = datalen as u16;
    assert!(comm.send_response(&tx));
    assert_eq!(comm.get_tx_error(), TxError::None);
    assert_eq!(comm.data_to_send() as usize, datalen + RESPONSE_OVERHEAD);
}
