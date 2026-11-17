use scrutiny_embedded_rust::protocol::{CommHandler, RxError, MAXIMUM_RX_BUFFER_SIZE, RX_TIMEOUT_100NS};
use scrutiny_embedded_rust::timebase::Timebase;
use scrutiny_embedded_rust::tools::crc32;

const RX_BUFFER_SIZE: usize = 128;

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

/// Builds a frame whose length field is `datalen` but carries `RX_BUFFER_SIZE` data bytes, like the C++ buffer tests.
fn make_buffer_sized_request(datalen: u16) -> Vec<u8> {
    let mut frame = vec![1u8, 2, (datalen >> 8) as u8, (datalen & 0xFF) as u8];
    frame.resize(4 + RX_BUFFER_SIZE, 0);
    let crc = crc32(&frame, 0);
    frame.push((crc >> 24) as u8);
    frame.push((crc >> 16) as u8);
    frame.push((crc >> 8) as u8);
    frame.push(crc as u8);
    frame
}

fn make_comm_buffers() -> ([u8; RX_BUFFER_SIZE], [u8; 128]) {
    ([0u8; RX_BUFFER_SIZE], [0u8; 128])
}

#[test]
// Port of TestRx_ZeroLen_AllInOne
fn test_rx_zero_len_all_in_one() {
    let (mut rx, tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let frame = make_request(1, 2, &[]);
    comm.receive_data(&frame, &mut rx, &tb);

    assert!(comm.request_received());
    assert_eq!(comm.get_request(), Some((1, 2, 0)));
    assert_eq!(comm.get_rx_error(), RxError::None);
}

#[test]
// Port of TestRx_ZeroLen_BytePerByte
fn test_rx_zero_len_byte_per_byte() {
    let (mut rx, tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let frame = make_request(1, 2, &[]);
    for i in 0..frame.len() {
        comm.receive_data(&frame[i..=i], &mut rx, &tb);
    }

    assert!(comm.request_received());
    assert_eq!(comm.get_request(), Some((1, 2, 0)));
    assert_eq!(comm.get_rx_error(), RxError::None);
}

#[test]
// Port of TestRx_NonZeroLen_SplitInLen: the 2-byte length field arrives in two calls
fn test_rx_non_zero_len_split_in_len() {
    let (mut rx, tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let frame = make_request(1, 2, &[0x11, 0x22, 0x33]);
    assert_eq!(frame.len(), 11);
    comm.receive_data(&frame[..3], &mut rx, &tb);
    comm.receive_data(&frame[3..], &mut rx, &tb);

    assert!(comm.request_received());
    assert_eq!(comm.get_request(), Some((1, 2, 3)));
    assert_eq!(&rx[..3], &[0x11, 0x22, 0x33]);
    assert_eq!(comm.get_rx_error(), RxError::None);
}

#[test]
// Port of TestRx_Discover_2_parts
fn test_rx_discover_2_parts() {
    let (mut rx, tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let frame = make_request(1, 2, &[0x7e, 0x18, 0xfc, 0x68]);
    for i in 0..frame.len() {
        comm.receive_data(&frame[..i], &mut rx, &tb);
        comm.process(&tb);

        comm.receive_data(&frame[i..], &mut rx, &tb);

        assert!(comm.request_received(), "i={i}");
        assert_eq!(comm.get_request(), Some((1, 2, 4)), "i={i}");
        assert_eq!(&rx[..4], &[0x7e, 0x18, 0xfc, 0x68], "i={i}");
        assert_eq!(comm.get_rx_error(), RxError::None, "i={i}");
    }
}

#[test]
// Port of TestRx_UseAllBuffer: a payload as large as the RX buffer is accepted
fn test_rx_use_all_buffer() {
    assert!(RX_BUFFER_SIZE < MAXIMUM_RX_BUFFER_SIZE as usize);
    let (mut rx, tx) = make_comm_buffers();
    let tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let frame = make_buffer_sized_request(RX_BUFFER_SIZE as u16);
    comm.receive_data(&frame, &mut rx, &tb);

    assert!(comm.request_received());
}

#[test]
// Port of TestRx_OverflowRestoreAfterDelay
fn test_rx_overflow_restore_after_delay() {
    assert!(RX_BUFFER_SIZE < MAXIMUM_RX_BUFFER_SIZE as usize);
    let (mut rx, tx) = make_comm_buffers();
    let mut tb = Timebase::new();
    let mut comm = CommHandler::new(&rx, &tx, 0);
    comm.connect(&tb);

    let frame = make_buffer_sized_request(RX_BUFFER_SIZE as u16 + 1);
    comm.receive_data(&frame, &mut rx, &tb);

    assert!(!comm.request_received());
    assert_eq!(comm.get_rx_error(), RxError::Overflow);

    tb.step(RX_TIMEOUT_100NS);

    let frame2 = make_buffer_sized_request(RX_BUFFER_SIZE as u16);
    comm.receive_data(&frame2, &mut rx, &tb);

    assert!(comm.request_received());
}
