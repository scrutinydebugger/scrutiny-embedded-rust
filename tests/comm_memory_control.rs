use scrutiny_embedded_rust::config::Config;
use scrutiny_embedded_rust::main_handler::MainHandler;
use scrutiny_embedded_rust::protocol::{
    CommControlSubfn, CommandId, MemoryControlSubfn, ResponseCode, CONNECT_MAGIC, MINIMUM_RX_BUFFER_SIZE,
    MINIMUM_TX_BUFFER_SIZE,
};
use scrutiny_embedded_rust::tools::crc32;
use scrutiny_embedded_rust::types::{AnyType, RuntimePublishedValue, VariableType};

#[cfg(feature = "protected_regions")]
use scrutiny_embedded_rust::types::AddressRange;

// ── Wire helpers ─────────────────────────────────────────────────────────────

const ADDR_SIZE: usize = core::mem::size_of::<*const u8>();

fn make_request(cmd: u8, subfn: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut frame = Vec::new();
    frame.push(cmd);
    frame.push(subfn);
    frame.push((len >> 8) as u8);
    frame.push(len as u8);
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

// ── Address encoding helpers ─────────────────────────────────────────────────

fn encode_addr(ptr: *const u8) -> Vec<u8> {
    let addr = ptr as usize;
    let mut out = Vec::with_capacity(ADDR_SIZE);
    for i in (0..ADDR_SIZE).rev() {
        out.push((addr >> (8 * i)) as u8);
    }
    out
}

/// Builds the payload bytes for a single Read block: address + length.
fn read_block(ptr: *const u8, len: u16) -> Vec<u8> {
    let mut v = encode_addr(ptr);
    v.push((len >> 8) as u8);
    v.push(len as u8);
    v
}

/// Builds the payload bytes for a single Write block: address + length + data.
fn write_block(ptr: *const u8, data: &[u8]) -> Vec<u8> {
    let mut v = encode_addr(ptr);
    let len = data.len() as u16;
    v.push((len >> 8) as u8);
    v.push(len as u8);
    v.extend_from_slice(data);
    v
}

/// Builds the payload bytes for a single WriteMasked block: address + length + data + mask.
fn masked_write_block(ptr: *const u8, data: &[u8], mask: &[u8]) -> Vec<u8> {
    assert_eq!(data.len(), mask.len());
    let mut v = encode_addr(ptr);
    let len = data.len() as u16;
    v.push((len >> 8) as u8);
    v.push(len as u8);
    v.extend_from_slice(data);
    v.extend_from_slice(mask);
    v
}

// ── MemoryControl/Read ───────────────────────────────────────────────────────

#[test]
fn test_read_single_block() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let data = [0x11u8, 0x22, 0x33];
    let payload = read_block(data.as_ptr(), data.len() as u16);
    let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &payload);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp[0], CommandId::MemoryControl as u8 | 0x80);
    assert_eq!(resp[1], MemoryControlSubfn::Read as u8);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    let p = resp_payload(&resp);
    // Response: address(ADDR_SIZE) + length(2) + data(3)
    assert_eq!(p.len(), ADDR_SIZE + 2 + 3);
    assert_eq!(&p[..ADDR_SIZE], &encode_addr(data.as_ptr()));
    assert_eq!(u16::from_be_bytes([p[ADDR_SIZE], p[ADDR_SIZE + 1]]), 3);
    assert_eq!(&p[ADDR_SIZE + 2..], &data);
}

#[test]
fn test_read_multiple_blocks() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let buf1 = [0x11u8, 0x22, 0x33];
    let buf2 = [0xAAu8, 0xBB, 0xCC, 0xDD];
    let buf3 = [0x01u8, 0x02];

    let mut payload = Vec::new();
    payload.extend(read_block(buf1.as_ptr(), buf1.len() as u16));
    payload.extend(read_block(buf2.as_ptr(), buf2.len() as u16));
    payload.extend(read_block(buf3.as_ptr(), buf3.len() as u16));

    let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &payload);
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    let p = resp_payload(&resp);
    let bh = ADDR_SIZE + 2;
    assert_eq!(p.len(), bh * 3 + buf1.len() + buf2.len() + buf3.len());

    let mut off = 0usize;
    for (ptr, expected) in [
        (buf1.as_ptr(), buf1.as_ref()),
        (buf2.as_ptr(), buf2.as_ref()),
        (buf3.as_ptr(), buf3.as_ref()),
    ] {
        assert_eq!(&p[off..off + ADDR_SIZE], &encode_addr(ptr));
        let len = u16::from_be_bytes([p[off + ADDR_SIZE], p[off + ADDR_SIZE + 1]]) as usize;
        assert_eq!(len, expected.len());
        assert_eq!(&p[off + bh..off + bh + len], expected);
        off += bh + len;
    }
}

#[test]
fn test_read_invalid_request_bad_length() {
    // Any data length that is not a multiple of (ADDR_SIZE + 2) is invalid.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let bh = ADDR_SIZE + 2;
    for bad_len in 1..=(bh * 2 + 3) {
        if bad_len % bh == 0 {
            continue; // skip structurally valid lengths
        }
        let payload = vec![0u8; bad_len];
        let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &payload);
        let resp = send_and_recv(&mut h, &req);
        h.process(0);
        assert_eq!(
            resp_code(&resp),
            ResponseCode::InvalidRequest as u8,
            "expected InvalidRequest for data_len={bad_len}"
        );
    }
}

#[test]
fn test_read_empty_request() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &[]);
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_read_overflow_tx_buffer() {
    // Request more bytes than the TX buffer can hold → Overflow.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 64];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // With a 64-byte TX buf, any single-block read that needs > 64 bytes overflows.
    // 1 byte fits (ADDR_SIZE + 2 + 1), many bytes won't.
    let some_buf = [0u8; 128];
    let overflow_len = (64 - ADDR_SIZE - 2 + 1) as u16; // just one over capacity

    let payload = read_block(some_buf.as_ptr(), overflow_len);
    let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &payload);
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::Overflow as u8);

    h.process(0);

    // A request that fits should succeed.
    let fitting_len = (64 - ADDR_SIZE - 2) as u16;
    let payload = read_block(some_buf.as_ptr(), fitting_len);
    let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &payload);
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
}

#[test]
fn test_read_length_exceeds_protocol_max() {
    // A single block requesting 0xFFFF bytes is an InvalidRequest (exceeds protocol max),
    // even if it would have been Overflow anyway.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let some_buf = [0u8; 1];
    let payload = read_block(some_buf.as_ptr(), 0xFFFF);
    let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &payload);
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_read_forbidden_address() {
    // A 16-byte buffer; bytes [6..10) are forbidden.
    // A 4-byte sliding window must be rejected whenever it overlaps the forbidden region.
    let buf: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    ];
    let forbidden_start = unsafe { buf.as_ptr().add(6) };
    let forbidden_end = unsafe { buf.as_ptr().add(9) }; // inclusive end

    static FORBIDDEN: std::sync::OnceLock<[AddressRange; 1]> = std::sync::OnceLock::new();
    let ranges = FORBIDDEN.get_or_init(|| {
        [AddressRange {
            start: forbidden_start,
            end: forbidden_end,
        }]
    });

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_forbidden_address_range(ranges);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let window = 4u16;
    for i in 0..(buf.len() - window as usize) {
        let read_ptr = unsafe { buf.as_ptr().add(i) };
        let payload = read_block(read_ptr, window);
        let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &payload);
        let resp = send_and_recv(&mut h, &req);
        h.process(0);

        // Window [i, i+4) overlaps [6,10) if i < 10 AND i+4 > 6 → i in [3..9] inclusive
        let overlaps = i < 10 && i + (window as usize) > 6;
        let expected = if overlaps {
            ResponseCode::Forbidden as u8
        } else {
            ResponseCode::Ok as u8
        };
        assert_eq!(resp_code(&resp), expected, "i={i}");
    }
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_read_forbidden_containment() {
    // Request spans a range that fully contains the forbidden region → Forbidden.
    let buf = [0u8; 16];
    let forbidden_start = unsafe { buf.as_ptr().add(6) };
    let forbidden_end = unsafe { buf.as_ptr().add(9) };

    static FORBIDDEN: std::sync::OnceLock<[AddressRange; 1]> = std::sync::OnceLock::new();
    let ranges = FORBIDDEN.get_or_init(|| {
        [AddressRange {
            start: forbidden_start,
            end: forbidden_end,
        }]
    });

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_forbidden_address_range(ranges);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Read from 1 byte before forbidden through 1 byte after → 6 bytes spanning the region.
    let read_ptr = unsafe { buf.as_ptr().add(5) };
    let payload = read_block(read_ptr, 6);
    let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &payload);
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_read_readonly_address_allowed() {
    // Readonly regions must NOT block reads — only writes.
    let buf: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    ];
    let ro_start = unsafe { buf.as_ptr().add(6) };
    let ro_end = unsafe { buf.as_ptr().add(9) };

    static READONLY: std::sync::OnceLock<[AddressRange; 1]> = std::sync::OnceLock::new();
    let ranges = READONLY.get_or_init(|| {
        [AddressRange {
            start: ro_start,
            end: ro_end,
        }]
    });

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_readonly_address_range(ranges);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let window = 4u16;
    for i in 0..(buf.len() - window as usize) {
        let read_ptr = unsafe { buf.as_ptr().add(i) };
        let payload = read_block(read_ptr, window);
        let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::Read as u8, &payload);
        let resp = send_and_recv(&mut h, &req);
        h.process(0);
        assert_eq!(
            resp_code(&resp),
            ResponseCode::Ok as u8,
            "read of readonly region should succeed at i={i}"
        );
    }
}

// ── MemoryControl/Write ──────────────────────────────────────────────────────

#[test]
fn test_write_single_block() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut buf = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    let data_to_write = [0x11u8, 0x22, 0x33, 0x44];

    let payload = write_block(buf.as_mut_ptr() as *const u8, &data_to_write);
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::Write as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp[0], CommandId::MemoryControl as u8 | 0x80);
    assert_eq!(resp[1], MemoryControlSubfn::Write as u8);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    // Response echoes address + length only (no data).
    let p = resp_payload(&resp);
    assert_eq!(p.len(), ADDR_SIZE + 2);
    assert_eq!(&p[..ADDR_SIZE], &encode_addr(buf.as_ptr()));
    assert_eq!(
        u16::from_be_bytes([p[ADDR_SIZE], p[ADDR_SIZE + 1]]),
        data_to_write.len() as u16
    );

    // Verify the buffer was updated.
    assert_eq!(&buf[..4], &data_to_write);
    assert_eq!(&buf[4..], &[0x05, 0x06, 0x07, 0x08]);
}

#[test]
fn test_write_multiple_blocks() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut buf = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a];
    let data1 = [0x11u8, 0x22, 0x33];
    let data2 = [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];

    let mut payload = Vec::new();
    payload.extend(write_block(buf.as_mut_ptr() as *const u8, &data1));
    payload.extend(write_block(unsafe { buf.as_mut_ptr().add(3) } as *const u8, &data2));

    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::Write as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    let p = resp_payload(&resp);
    assert_eq!(p.len(), (ADDR_SIZE + 2) * 2);

    // buf[0..3] = data1, buf[3..9] = data2, buf[9] unchanged.
    assert_eq!(&buf[..3], &data1);
    assert_eq!(&buf[3..9], &data2);
    assert_eq!(buf[9], 0x0a);
}

#[test]
fn test_write_disabled() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.memory_write_enable = false;
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut buf = [0xFFu8; 4];
    let payload = write_block(buf.as_mut_ptr() as *const u8, &[0x00, 0x00, 0x00, 0x00]);
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::Write as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);
    // Buffer must be untouched.
    assert_eq!(buf, [0xFFu8; 4]);
}

#[test]
fn test_write_invalid_data_length() {
    // The encoded block length claims more bytes than are present in the request.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let buf = [0u8; 10];
    // Build a block header claiming 10 bytes of data, but supply only 9.
    let mut payload = encode_addr(buf.as_ptr());
    payload.push(0x00);
    payload.push(10); // claims 10 bytes
    for _ in 0..9 {
        payload.push(0x00);
    } // only 9 bytes of data

    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::Write as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_write_invalid_request_bad_length() {
    // Various request lengths — only exact block multiples are valid.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let data_len = 3usize;
    let block_len = ADDR_SIZE + 2 + data_len;
    let two_blocks = block_len * 2;

    let some_data = [0u8; 10];

    // Build a payload for two full blocks but send various truncated/extended lengths.
    let mut full_payload = Vec::new();
    full_payload.extend(write_block(some_data.as_ptr(), &some_data[..data_len]));
    full_payload.extend(write_block(some_data.as_ptr(), &some_data[..data_len]));

    for bad_len in 0..=(two_blocks + 5) {
        if bad_len == block_len || bad_len == two_blocks {
            continue; // skip valid lengths
        }
        // Pad or truncate the payload.
        let mut payload = full_payload.clone();
        payload.resize(bad_len, 0x00);
        let req = make_request(
            CommandId::MemoryControl as u8,
            MemoryControlSubfn::Write as u8,
            &payload,
        );
        let resp = send_and_recv(&mut h, &req);
        h.process(0);
        assert_eq!(
            resp_code(&resp),
            ResponseCode::InvalidRequest as u8,
            "expected InvalidRequest for bad_len={bad_len}"
        );
    }
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_write_forbidden_address() {
    let buf: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    ];
    let forbidden_start = unsafe { buf.as_ptr().add(6) };
    let forbidden_end = unsafe { buf.as_ptr().add(9) };

    static FORBIDDEN: std::sync::OnceLock<[AddressRange; 1]> = std::sync::OnceLock::new();
    let ranges = FORBIDDEN.get_or_init(|| {
        [AddressRange {
            start: forbidden_start,
            end: forbidden_end,
        }]
    });

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_forbidden_address_range(ranges);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let write_data = [0xFFu8; 4];
    let window = write_data.len();
    let mut shadow = buf;

    for i in 0..(buf.len() - window) {
        let write_ptr = unsafe { buf.as_ptr().add(i) };
        let payload = write_block(write_ptr as *const u8, &write_data);
        let req = make_request(
            CommandId::MemoryControl as u8,
            MemoryControlSubfn::Write as u8,
            &payload,
        );
        let resp = send_and_recv(&mut h, &req);
        h.process(0);

        let overlaps = i < 10 && i + window > 6;
        let expected = if overlaps {
            ResponseCode::Forbidden as u8
        } else {
            ResponseCode::Ok as u8
        };
        assert_eq!(resp_code(&resp), expected, "i={i}");

        if !overlaps {
            // Update shadow to reflect the successful write.
            shadow[i..i + window].copy_from_slice(&write_data);
        }
    }
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_write_readonly_address() {
    let buf = [0u8; 16];
    let ro_start = unsafe { buf.as_ptr().add(6) };
    let ro_end = unsafe { buf.as_ptr().add(9) };

    static READONLY: std::sync::OnceLock<[AddressRange; 1]> = std::sync::OnceLock::new();
    let ranges = READONLY.get_or_init(|| {
        [AddressRange {
            start: ro_start,
            end: ro_end,
        }]
    });

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_readonly_address_range(ranges);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let write_data = [0xFFu8; 4];
    let window = write_data.len();

    for i in 0..(buf.len() - window) {
        let write_ptr = unsafe { buf.as_ptr().add(i) };
        let payload = write_block(write_ptr as *const u8, &write_data);
        let req = make_request(
            CommandId::MemoryControl as u8,
            MemoryControlSubfn::Write as u8,
            &payload,
        );
        let resp = send_and_recv(&mut h, &req);
        h.process(0);

        let overlaps = i < 10 && i + window > 6;
        let expected = if overlaps {
            ResponseCode::Forbidden as u8
        } else {
            ResponseCode::Ok as u8
        };
        assert_eq!(resp_code(&resp), expected, "i={i}");
    }
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_write_readonly_containment() {
    // A write spanning a region that contains the readonly range must be rejected.
    let buf = [0u8; 16];
    let ro_start = unsafe { buf.as_ptr().add(6) };
    let ro_end = unsafe { buf.as_ptr().add(9) };

    static READONLY: std::sync::OnceLock<[AddressRange; 1]> = std::sync::OnceLock::new();
    let ranges = READONLY.get_or_init(|| {
        [AddressRange {
            start: ro_start,
            end: ro_end,
        }]
    });

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_readonly_address_range(ranges);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Write from 1 byte before the readonly through 1 byte after → 6 bytes.
    let write_ptr = unsafe { buf.as_ptr().add(5) };
    let payload = write_block(write_ptr as *const u8, &[0xFFu8; 6]);
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::Write as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);
}

// ── MemoryControl/WriteMasked ────────────────────────────────────────────────

#[test]
fn test_write_masked_single_block() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Initial: 0xAA repeated.
    // Data:  [0xFF, 0xFF, 0x00, 0x00, 0x55, 0x55]
    // Mask:  [0xF0, 0xAA, 0xF0, 0xAA, 0xAF, 0x55]
    // Expected: for each byte: (cur & !mask) | (data & mask)
    //   byte 0: (0xAA & 0x0F) | (0xFF & 0xF0) = 0x0A | 0xF0 = 0xFA
    //   byte 1: (0xAA & 0x55) | (0xFF & 0xAA) = 0x00 | 0xAA = 0xAA  (wait: ~0xAA = 0x55, so 0xAA & 0x55 = 0x00; 0xFF & 0xAA = 0xAA) → 0xAA
    //   byte 2: (0xAA & 0x0F) | (0x00 & 0xF0) = 0x0A | 0x00 = 0x0A
    //   byte 3: (0xAA & 0x55) | (0x00 & 0xAA) = 0x00 | 0x00 = 0x00
    //   byte 4: (0xAA & 0x50) | (0x55 & 0xAF) = 0x00 | 0x05 = 0x05
    //   byte 5: (0xAA & 0xAA) | (0x55 & 0x55) = 0xAA | 0x55 = 0xFF
    let mut buf = [0xAAu8; 10];
    let data = [0xFFu8, 0xFF, 0x00, 0x00, 0x55, 0x55];
    let mask = [0xF0u8, 0xAA, 0xF0, 0xAA, 0xAF, 0x55];
    let expected = [0xFAu8, 0xAA, 0x0A, 0x00, 0x05, 0xFF];

    let payload = masked_write_block(buf.as_mut_ptr() as *const u8, &data, &mask);
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteMasked as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp[0], CommandId::MemoryControl as u8 | 0x80);
    assert_eq!(resp[1], MemoryControlSubfn::WriteMasked as u8);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    // Response echoes address + length only.
    let p = resp_payload(&resp);
    assert_eq!(p.len(), ADDR_SIZE + 2);
    assert_eq!(u16::from_be_bytes([p[ADDR_SIZE], p[ADDR_SIZE + 1]]), data.len() as u16);

    // Verify mask logic.
    assert_eq!(&buf[..data.len()], &expected);
    assert_eq!(&buf[data.len()..], &[0xAAu8; 4]);
}

#[test]
fn test_write_masked_multiple_blocks() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut buf = [0x55u8; 10];

    // Block 1 at buf[0..4]
    let data1 = [0x11u8, 0x22, 0x33, 0x44];
    let mask1 = [0xFFu8, 0x00, 0xF0, 0x0F];
    // Expected: (0x55 & !m) | (d & m)
    //   0: (0x55 & 0x00) | (0x11 & 0xFF) = 0x11
    //   1: (0x55 & 0xFF) | (0x22 & 0x00) = 0x55
    //   2: (0x55 & 0x0F) | (0x33 & 0xF0) = 0x05 | 0x30 = 0x35
    //   3: (0x55 & 0xF0) | (0x44 & 0x0F) = 0x50 | 0x04 = 0x54
    let exp1 = [0x11u8, 0x55, 0x35, 0x54];

    // Block 2 at buf[6..10]
    let data2 = [0xFFu8, 0x00];
    let mask2 = [0xF0u8, 0x0F];
    // Expected:
    //   0: (0x55 & 0x0F) | (0xFF & 0xF0) = 0x05 | 0xF0 = 0xF5
    //   1: (0x55 & 0xF0) | (0x00 & 0x0F) = 0x50 | 0x00 = 0x50
    let exp2 = [0xF5u8, 0x50];

    let mut payload = Vec::new();
    payload.extend(masked_write_block(buf.as_mut_ptr() as *const u8, &data1, &mask1));
    payload.extend(masked_write_block(
        unsafe { buf.as_mut_ptr().add(6) } as *const u8,
        &data2,
        &mask2,
    ));

    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteMasked as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    assert_eq!(resp_payload(&resp).len(), (ADDR_SIZE + 2) * 2);

    assert_eq!(&buf[..4], &exp1);
    assert_eq!(buf[4], 0x55); // unchanged
    assert_eq!(buf[5], 0x55); // unchanged
    assert_eq!(&buf[6..8], &exp2);
    assert_eq!(&buf[8..], &[0x55u8; 2]); // unchanged
}

#[test]
fn test_write_masked_disabled() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.memory_write_enable = false;
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut buf = [0xFFu8; 4];
    let payload = masked_write_block(buf.as_mut_ptr() as *const u8, &[0x00; 4], &[0xFF; 4]);
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteMasked as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Forbidden as u8);
    assert_eq!(buf, [0xFFu8; 4]);
}

#[test]
fn test_write_masked_invalid_data_length() {
    // A WriteMasked block with no mask bytes (only data) is invalid.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let buf = [0u8; 10];
    // Build a block claiming 4 bytes but only supplying data (no mask).
    let mut payload = encode_addr(buf.as_ptr());
    payload.push(0x00);
    payload.push(4); // length = 4
    for _ in 0..4 {
        payload.push(0x00);
    } // only data, no mask

    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteMasked as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[cfg(feature = "protected_regions")]
#[test]
fn test_write_masked_forbidden_address() {
    let buf = [0u8; 16];
    let forbidden_start = unsafe { buf.as_ptr().add(6) };
    let forbidden_end = unsafe { buf.as_ptr().add(9) };

    static FORBIDDEN: std::sync::OnceLock<[AddressRange; 1]> = std::sync::OnceLock::new();
    let ranges = FORBIDDEN.get_or_init(|| {
        [AddressRange {
            start: forbidden_start,
            end: forbidden_end,
        }]
    });

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_forbidden_address_range(ranges);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let write_data = [0xFFu8; 4];
    let mask = [0xFFu8; 4];
    let window = write_data.len();

    for i in 0..(buf.len() - window) {
        let write_ptr = unsafe { buf.as_ptr().add(i) };
        let payload = masked_write_block(write_ptr as *const u8, &write_data, &mask);
        let req = make_request(
            CommandId::MemoryControl as u8,
            MemoryControlSubfn::WriteMasked as u8,
            &payload,
        );
        let resp = send_and_recv(&mut h, &req);
        h.process(0);

        let overlaps = i < 10 && i + window > 6;
        let expected = if overlaps {
            ResponseCode::Forbidden as u8
        } else {
            ResponseCode::Ok as u8
        };
        assert_eq!(resp_code(&resp), expected, "i={i}");
    }
}

// ── MemoryControl/ReadRpv ────────────────────────────────────────────────────

fn rpv_read_cb(rpv: RuntimePublishedValue, out: &mut AnyType, _caller: Option<&'static str>) -> bool {
    match rpv.id {
        0x1122 => {
            out.uint32 = 0x12345678;
            true
        }
        0x3344 => {
            out.uint16 = 0xABCD;
            true
        }
        _ => false,
    }
}

fn rpv_write_cb_ok(rpv: RuntimePublishedValue, val: &AnyType) -> bool {
    // Store value for verification via thread-local or similar.
    // We just check the callback was called correctly by the return value.
    let _ = (rpv, val);
    true
}

fn rpv_write_cb_fail(_rpv: RuntimePublishedValue, _val: &AnyType) -> bool {
    false
}

static TEST_RPVS: [RuntimePublishedValue; 2] = [
    RuntimePublishedValue {
        id: 0x1122,
        var_type: VariableType::Uint32,
    },
    RuntimePublishedValue {
        id: 0x3344,
        var_type: VariableType::Uint16,
    },
];

#[test]
fn test_read_rpv_single() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Request: read RPV id=0x1122 (Uint32, value 0x12345678)
    let payload = [0x11u8, 0x22];
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::ReadRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp[0], CommandId::MemoryControl as u8 | 0x80);
    assert_eq!(resp[1], MemoryControlSubfn::ReadRpv as u8);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    // Response: id(2) + value(4 for Uint32)
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 6);
    assert_eq!(u16::from_be_bytes([p[0], p[1]]), 0x1122);
    assert_eq!(u32::from_be_bytes([p[2], p[3], p[4], p[5]]), 0x12345678);
}

#[test]
fn test_read_rpv_multiple() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Request both RPVs in one go.
    let payload = [0x11u8, 0x22, 0x33, 0x44];
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::ReadRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    // 0x1122: id(2) + uint32(4) = 6; 0x3344: id(2) + uint16(2) = 4; total = 10
    assert_eq!(p.len(), 10);
    assert_eq!(u16::from_be_bytes([p[0], p[1]]), 0x1122);
    assert_eq!(u32::from_be_bytes([p[2], p[3], p[4], p[5]]), 0x12345678);
    assert_eq!(u16::from_be_bytes([p[6], p[7]]), 0x3344);
    assert_eq!(u16::from_be_bytes([p[8], p[9]]), 0xABCD);
}

#[test]
fn test_read_rpv_no_callback() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    // No RPVs configured at all.
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let payload = [0x11u8, 0x22];
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::ReadRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::UnsupportedFeature as u8);
}

#[test]
fn test_read_rpv_invalid_odd_length() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Odd number of bytes → cannot be a list of 2-byte IDs.
    let payload = [0x11u8, 0x22, 0x33]; // 3 bytes
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::ReadRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_read_rpv_unknown_id() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // ID 0xDEAD is not in TEST_RPVS.
    let payload = [0xDEu8, 0xAD];
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::ReadRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_read_rpv_empty_request() {
    // An empty ReadRPV request is invalid, as in C++.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::ReadRpv as u8, &[]);
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_read_rpv_empty_rpv_list() {
    // An empty RPV list means the feature is not configured.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&[], Some(rpv_read_cb), None);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let payload = [0x11u8, 0x22];
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::ReadRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::UnsupportedFeature as u8);
}

fn rpv_read_each_type_cb(rpv: RuntimePublishedValue, out: &mut AnyType, _caller: Option<&'static str>) -> bool {
    match (rpv.id, rpv.var_type) {
        (0x9000, VariableType::Uint8) => out.uint8 = 0x99,
        (0x9001, VariableType::Uint16) => out.uint16 = 0xA5A5,
        (0x9002, VariableType::Uint32) => out.uint32 = 0x99887766,
        (0x9004, VariableType::Sint8) => out.sint8 = -64,
        (0x9005, VariableType::Sint16) => out.sint16 = -30000,
        (0x9006, VariableType::Sint32) => out.sint32 = -1000000000,
        (0x9008, VariableType::Float32) => out.float32 = 1.45,
        #[cfg(feature = "support_64bits")]
        (0x9003, VariableType::Uint64) => out.uint64 = 0xFFFF111122223333,
        #[cfg(feature = "support_64bits")]
        (0x9007, VariableType::Sint64) => out.sint64 = -90000000000,
        #[cfg(feature = "support_64bits")]
        (0x9009, VariableType::Float64) => out.float64 = -5.123,
        _ => return false,
    }
    true
}

static EACH_TYPE_READ_RPVS: &[RuntimePublishedValue] = &[
    RuntimePublishedValue {
        id: 0x9000,
        var_type: VariableType::Uint8,
    },
    RuntimePublishedValue {
        id: 0x9004,
        var_type: VariableType::Sint8,
    },
    RuntimePublishedValue {
        id: 0x9001,
        var_type: VariableType::Uint16,
    },
    RuntimePublishedValue {
        id: 0x9002,
        var_type: VariableType::Uint32,
    },
    RuntimePublishedValue {
        id: 0x9005,
        var_type: VariableType::Sint16,
    },
    RuntimePublishedValue {
        id: 0x9006,
        var_type: VariableType::Sint32,
    },
    RuntimePublishedValue {
        id: 0x9008,
        var_type: VariableType::Float32,
    },
    #[cfg(feature = "support_64bits")]
    RuntimePublishedValue {
        id: 0x9003,
        var_type: VariableType::Uint64,
    },
    #[cfg(feature = "support_64bits")]
    RuntimePublishedValue {
        id: 0x9007,
        var_type: VariableType::Sint64,
    },
    #[cfg(feature = "support_64bits")]
    RuntimePublishedValue {
        id: 0x9009,
        var_type: VariableType::Float64,
    },
];

#[test]
fn test_read_multiple_rpv_each_type() {
    // Mirrors C++ TestReadMultipleRPVEachType: read one RPV of each type, check the encoding.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(EACH_TYPE_READ_RPVS, Some(rpv_read_each_type_cb), None);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Sorted by ID, like the C++ std::map.
    let mut expected: Vec<(u16, Vec<u8>)> = Vec::new();
    expected.push((0x9000, vec![0x99]));
    expected.push((0x9001, vec![0xA5, 0xA5]));
    expected.push((0x9002, vec![0x99, 0x88, 0x77, 0x66]));
    #[cfg(feature = "support_64bits")]
    expected.push((0x9003, vec![0xFF, 0xFF, 0x11, 0x11, 0x22, 0x22, 0x33, 0x33]));
    expected.push((0x9004, vec![0xC0])); // -64
    expected.push((0x9005, vec![0x8A, 0xD0])); // -30000
    expected.push((0x9006, vec![0xC4, 0x65, 0x36, 0x00])); // -1 000 000 000
    #[cfg(feature = "support_64bits")]
    expected.push((0x9007, vec![0xFF, 0xFF, 0xFF, 0xEB, 0x0B, 0x94, 0xFC, 0x00])); // -90 000 000 000
    expected.push((0x9008, vec![0x3F, 0xB9, 0x99, 0x9A])); // 1.45
    #[cfg(feature = "support_64bits")]
    expected.push((0x9009, vec![0xC0, 0x14, 0x7D, 0xF3, 0xB6, 0x45, 0xA1, 0xCB])); // -5.123

    let mut payload = Vec::new();
    for (id, _) in &expected {
        payload.extend_from_slice(&id.to_be_bytes());
    }
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::ReadRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    let mut expected_payload = Vec::new();
    for (id, value) in &expected {
        expected_payload.extend_from_slice(&id.to_be_bytes());
        expected_payload.extend_from_slice(value);
    }
    assert_eq!(resp_payload(&resp), expected_payload.as_slice());
}

#[test]
fn test_read_rpv_response_overflow() {
    // Mirrors C++ TestReadRPVResponseOverflow: the response does not fit the TX buffer.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let tx_size = tx.len();
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), None);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // 0x1122 is Uint32: each entry takes id(2) + value(4) = 6 bytes in the response.
    let nb_rpv = tx_size / 6 + 1;
    let mut payload = Vec::new();
    for _ in 0..nb_rpv {
        payload.extend_from_slice(&[0x11, 0x22]);
    }
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::ReadRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp[0], CommandId::MemoryControl as u8 | 0x80);
    assert_eq!(resp[1], MemoryControlSubfn::ReadRpv as u8);
    assert_eq!(resp_code(&resp), ResponseCode::Overflow as u8);
    assert!(resp_payload(&resp).is_empty());
}

// ── MemoryControl/WriteRpv ───────────────────────────────────────────────────

#[test]
fn test_write_rpv_single() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // Write RPV 0x1122 (Uint32) with value 0xDEADBEEF.
    let mut payload = vec![0x11u8, 0x22];
    payload.extend_from_slice(&0xDEADBEEFu32.to_be_bytes());

    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp[0], CommandId::MemoryControl as u8 | 0x80);
    assert_eq!(resp[1], MemoryControlSubfn::WriteRpv as u8);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);

    // Response: id(2) + type_size(1)
    let p = resp_payload(&resp);
    assert_eq!(p.len(), 3);
    assert_eq!(u16::from_be_bytes([p[0], p[1]]), 0x1122);
    assert_eq!(p[2], 4); // Uint32 is 4 bytes
}

#[test]
fn test_write_rpv_multiple() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut payload = Vec::new();
    // Write 0x1122 (Uint32)
    payload.extend_from_slice(&[0x11u8, 0x22]);
    payload.extend_from_slice(&0x11223344u32.to_be_bytes());
    // Write 0x3344 (Uint16)
    payload.extend_from_slice(&[0x33u8, 0x44]);
    payload.extend_from_slice(&0xABCDu16.to_be_bytes());

    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);

    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    let p = resp_payload(&resp);
    // Two entries of 3 bytes each.
    assert_eq!(p.len(), 6);
    assert_eq!(u16::from_be_bytes([p[0], p[1]]), 0x1122);
    assert_eq!(p[2], 4);
    assert_eq!(u16::from_be_bytes([p[3], p[4]]), 0x3344);
    assert_eq!(p[5], 2);
}

#[test]
fn test_write_rpv_no_callback() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    // No RPVs configured.
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut payload = vec![0x11u8, 0x22];
    payload.extend_from_slice(&0u32.to_be_bytes());
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::UnsupportedFeature as u8);
}

#[test]
fn test_write_rpv_unknown_id() {
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // ID 0xDEAD is not registered — parsing cannot continue (unknown size).
    let mut payload = vec![0xDEu8, 0xAD];
    payload.extend_from_slice(&0u32.to_be_bytes()); // some value
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_write_rpv_truncated_value() {
    // The value bytes for a known RPV are truncated → InvalidRequest.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    // 0x1122 is Uint32 (4 bytes) but we only send 2 bytes of value.
    let payload = [0x11u8, 0x22, 0xDE, 0xAD]; // only 2 value bytes instead of 4
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_write_rpv_callback_failure() {
    // When the write callback returns false → FailureToProceed.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_fail));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut payload = vec![0x11u8, 0x22];
    payload.extend_from_slice(&0u32.to_be_bytes());
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::FailureToProceed as u8);
}

#[test]
fn test_write_rpv_empty_request() {
    // An empty WriteRPV request is invalid.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&TEST_RPVS, Some(rpv_read_cb), Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::MemoryControl as u8, MemoryControlSubfn::WriteRpv as u8, &[]);
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::InvalidRequest as u8);
}

#[test]
fn test_write_rpv_empty_rpv_list() {
    // An empty RPV list means the feature is not configured.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&[], None, Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut payload = vec![0x11u8, 0x22];
    payload.extend_from_slice(&0u32.to_be_bytes());
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::UnsupportedFeature as u8);
}

#[derive(Default)]
struct AllTypeResult {
    some_u8: u8,
    some_s8: i8,
    some_u16: u16,
    some_s16: i16,
    some_u32: u32,
    some_s32: i32,
    some_f32: f32,
    #[cfg(feature = "support_64bits")]
    some_u64: u64,
    #[cfg(feature = "support_64bits")]
    some_s64: i64,
    #[cfg(feature = "support_64bits")]
    some_f64: f64,
}

// Only test_write_rpv_all_types writes here; the Mutex keeps the access safe.
static ALL_TYPE_WRITE_RESULT: std::sync::Mutex<Option<AllTypeResult>> = std::sync::Mutex::new(None);

fn rpv_write_all_types_cb(rpv: RuntimePublishedValue, val: &AnyType) -> bool {
    let mut guard = ALL_TYPE_WRITE_RESULT.lock().unwrap();
    let dest = guard.get_or_insert_with(AllTypeResult::default);
    unsafe {
        match (rpv.id, rpv.var_type) {
            (0x1000, VariableType::Uint8) => dest.some_u8 = val.uint8,
            (0x1001, VariableType::Uint16) => dest.some_u16 = val.uint16,
            (0x1002, VariableType::Uint32) => dest.some_u32 = val.uint32,
            (0x1004, VariableType::Sint8) => dest.some_s8 = val.sint8,
            (0x1005, VariableType::Sint16) => dest.some_s16 = val.sint16,
            (0x1006, VariableType::Sint32) => dest.some_s32 = val.sint32,
            (0x1008, VariableType::Float32) => dest.some_f32 = val.float32,
            #[cfg(feature = "support_64bits")]
            (0x1003, VariableType::Uint64) => dest.some_u64 = val.uint64,
            #[cfg(feature = "support_64bits")]
            (0x1007, VariableType::Sint64) => dest.some_s64 = val.sint64,
            #[cfg(feature = "support_64bits")]
            (0x1009, VariableType::Float64) => dest.some_f64 = val.float64,
            _ => return false,
        }
    }
    true
}

static ALL_TYPE_WRITE_RPVS: &[RuntimePublishedValue] = &[
    RuntimePublishedValue {
        id: 0x1000,
        var_type: VariableType::Uint8,
    },
    RuntimePublishedValue {
        id: 0x1004,
        var_type: VariableType::Sint8,
    },
    RuntimePublishedValue {
        id: 0x1001,
        var_type: VariableType::Uint16,
    },
    RuntimePublishedValue {
        id: 0x1002,
        var_type: VariableType::Uint32,
    },
    RuntimePublishedValue {
        id: 0x1005,
        var_type: VariableType::Sint16,
    },
    RuntimePublishedValue {
        id: 0x1006,
        var_type: VariableType::Sint32,
    },
    RuntimePublishedValue {
        id: 0x1008,
        var_type: VariableType::Float32,
    },
    #[cfg(feature = "support_64bits")]
    RuntimePublishedValue {
        id: 0x1003,
        var_type: VariableType::Uint64,
    },
    #[cfg(feature = "support_64bits")]
    RuntimePublishedValue {
        id: 0x1007,
        var_type: VariableType::Sint64,
    },
    #[cfg(feature = "support_64bits")]
    RuntimePublishedValue {
        id: 0x1009,
        var_type: VariableType::Float64,
    },
];

static WRITE_U16_RPV: [RuntimePublishedValue; 1] = [RuntimePublishedValue {
    id: 0x1001,
    var_type: VariableType::Uint16,
}];

#[test]
fn test_write_rpv_all_types() {
    // Mirrors C++ TestWriteAllTypes: write one RPV of each type, check the decoded values.
    *ALL_TYPE_WRITE_RESULT.lock().unwrap() = Some(AllTypeResult::default());

    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(ALL_TYPE_WRITE_RPVS, None, Some(rpv_write_all_types_cb));
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let mut entries: Vec<(u16, Vec<u8>)> = Vec::new();
    entries.push((0x1000, vec![0x55]));
    entries.push((0x1004, vec![0xC0]));
    entries.push((0x1001, vec![0xAB, 0xCD]));
    entries.push((0x1002, vec![0x12, 0x34, 0x56, 0x78]));
    #[cfg(feature = "support_64bits")]
    entries.push((0x1003, vec![0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10]));
    entries.push((0x1005, vec![0x8A, 0xD0]));
    entries.push((0x1006, vec![0xC4, 0x65, 0x36, 0x00]));
    #[cfg(feature = "support_64bits")]
    entries.push((0x1007, vec![0xFF, 0xFF, 0xFF, 0xEB, 0x0B, 0x94, 0xFC, 0x00]));
    entries.push((0x1008, vec![0x3F, 0xB9, 0x99, 0x9A]));
    #[cfg(feature = "support_64bits")]
    entries.push((0x1009, vec![0xC0, 0x14, 0x7D, 0xF3, 0xB6, 0x45, 0xA1, 0xCB]));

    let mut payload = Vec::new();
    let mut expected_payload = Vec::new();
    for (id, value) in &entries {
        payload.extend_from_slice(&id.to_be_bytes());
        payload.extend_from_slice(value);
        expected_payload.extend_from_slice(&id.to_be_bytes());
        expected_payload.push(value.len() as u8);
    }

    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    assert_eq!(resp_payload(&resp), expected_payload.as_slice());

    let guard = ALL_TYPE_WRITE_RESULT.lock().unwrap();
    let result = guard.as_ref().unwrap();
    assert_eq!(result.some_u8, 0x55);
    assert_eq!(result.some_s8, -64);
    assert_eq!(result.some_u16, 0xABCD);
    assert_eq!(result.some_u32, 0x12345678);
    assert_eq!(result.some_s16, -30000);
    assert_eq!(result.some_s32, -1000000000);
    assert_eq!(result.some_f32, 1.45f32);
    #[cfg(feature = "support_64bits")]
    {
        assert_eq!(result.some_u64, 0xFEDCBA9876543210);
        assert_eq!(result.some_s64, -90000000000);
        assert_eq!(result.some_f64, -5.123f64);
    }
}

#[test]
fn test_write_rpv_response_overflow() {
    // Mirrors C++ TestWriteRPVResponseOverflow: the confirmation list does not fit the TX buffer.
    const BUFFERSIZE_MIN_MOD3: usize = (MINIMUM_TX_BUFFER_SIZE as usize).div_ceil(3) * 3;
    const NB_WRITE: usize = BUFFERSIZE_MIN_MOD3 / 3 + 1;
    const RX_DATA_SIZE: usize = NB_WRITE * 4; // id(2) + uint16(2)
    const TX_DATA_SIZE: usize = (NB_WRITE - 1) * 3; // id(2) + len(1), one entry short
    const _: () = assert!(RX_DATA_SIZE >= MINIMUM_RX_BUFFER_SIZE as usize);
    const _: () = assert!(TX_DATA_SIZE >= MINIMUM_TX_BUFFER_SIZE as usize);

    let mut rx = [0u8; RX_DATA_SIZE];
    let mut tx = [0u8; TX_DATA_SIZE];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&WRITE_U16_RPV, None, Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    assert!(h.is_enabled());
    connect(&mut h);

    let mut payload = Vec::new();
    for _ in 0..NB_WRITE {
        payload.extend_from_slice(&[0x10, 0x01, 0xAA, 0xBB]);
    }
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp.len(), 9);
    assert_eq!(resp[0], CommandId::MemoryControl as u8 | 0x80);
    assert_eq!(resp[1], MemoryControlSubfn::WriteRpv as u8);
    assert_eq!(resp_code(&resp), ResponseCode::Overflow as u8);
}

#[test]
fn test_write_rpv_response_full_no_overflow() {
    // Mirrors C++ TestWriteRPVResponseFullNoOverflow: the confirmation list exactly fills the TX buffer.
    const BUFFERSIZE_MIN_MOD3: usize = (MINIMUM_TX_BUFFER_SIZE as usize).div_ceil(3) * 3;
    const NB_WRITE: usize = BUFFERSIZE_MIN_MOD3 / 3;
    const RX_DATA_SIZE: usize = NB_WRITE * 4; // id(2) + uint16(2)
    const TX_DATA_SIZE: usize = NB_WRITE * 3; // id(2) + len(1)
    const _: () = assert!(RX_DATA_SIZE >= MINIMUM_RX_BUFFER_SIZE as usize);
    const _: () = assert!(TX_DATA_SIZE >= MINIMUM_TX_BUFFER_SIZE as usize);

    let mut rx = [0u8; RX_DATA_SIZE];
    let mut tx = [0u8; TX_DATA_SIZE];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    cfg.set_published_values(&WRITE_U16_RPV, None, Some(rpv_write_cb_ok));
    let mut h = MainHandler::new(cfg).unwrap();
    assert!(h.is_enabled());
    connect(&mut h);

    let mut payload = Vec::new();
    let mut expected_payload = Vec::new();
    for _ in 0..NB_WRITE {
        payload.extend_from_slice(&[0x10, 0x01, 0xAA, 0xBB]);
        expected_payload.extend_from_slice(&[0x10, 0x01, 2]);
    }
    let req = make_request(
        CommandId::MemoryControl as u8,
        MemoryControlSubfn::WriteRpv as u8,
        &payload,
    );
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp.len(), TX_DATA_SIZE + 9);
    assert_eq!(resp_code(&resp), ResponseCode::Ok as u8);
    assert_eq!(resp_payload(&resp), expected_payload.as_slice());
}

// ── Subfn routing ────────────────────────────────────────────────────────────

#[test]
fn test_unsupported_subfn() {
    // Subfunction 0x99 is not defined for MemoryControl.
    let mut rx = [0u8; 128];
    let mut tx = [0u8; 128];
    let mut cfg = Config::new();
    cfg.set_buffers(&mut rx, &mut tx);
    let mut h = MainHandler::new(cfg).unwrap();
    connect(&mut h);

    let req = make_request(CommandId::MemoryControl as u8, 0x99, &[]);
    let resp = send_and_recv(&mut h, &req);
    assert_eq!(resp_code(&resp), ResponseCode::UnsupportedFeature as u8);
}
