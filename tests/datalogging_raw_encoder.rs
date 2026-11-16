use scrutiny_embedded_rust::datalogging::{DataloggerHost, RawFormatEncoder};
use scrutiny_embedded_rust::datalogging::{Configuration, LoggableItem};
use scrutiny_embedded_rust::timebase::Timebase;
use scrutiny_embedded_rust::types::{AnyType, RuntimePublishedValue, VariableType};

static TEST_RPV: RuntimePublishedValue =
    RuntimePublishedValue { id: 0x1234, var_type: VariableType::Uint32 };

struct TestHost {
    rpv_ok: bool,
}

impl DataloggerHost for TestHost {
    unsafe fn read_memory(&self, dst: *mut u8, src: *const u8, count: usize) -> bool {
        core::ptr::copy_nonoverlapping(src, dst, count);
        true
    }
    fn is_rpv_read_configured(&self) -> bool { true }
    fn rpv_exists(&self, id: u16) -> bool { id == TEST_RPV.id }
    fn get_rpv(&self, id: u16) -> Option<RuntimePublishedValue> {
        if id == TEST_RPV.id { Some(TEST_RPV) } else { None }
    }
    fn read_rpv(&self, _rpv: RuntimePublishedValue, out: &mut AnyType) -> bool {
        if self.rpv_ok {
            out.uint32 = 0xaabb_ccdd;
            true
        } else {
            false
        }
    }
}

fn make_host(rpv_ok: bool) -> TestHost { TestHost { rpv_ok } }

#[allow(unused_assignments)]
#[test]
fn basic_encoding() {
    let host = make_host(true);
    let mut tb = Timebase::new();
    let mut var1: f32 = 0.0;
    let mut var2: u32 = 0;
    let mut buf = [0u8; 128];

    let mut cfg = Configuration::DEFAULT;
    cfg.items_count = 3;
    cfg.items_to_log[0] = LoggableItem::Memory {
        address: (&raw const var1).cast(),
        size: 4,
    };
    cfg.items_to_log[1] = LoggableItem::Memory {
        address: (&raw const var2).cast(),
        size: 4,
    };
    cfg.items_to_log[2] = LoggableItem::Time;

    let mut enc = RawFormatEncoder::new(&mut buf);
    enc.reset(&cfg, &host);
    assert!(!enc.error());

    // entry_size = 4 + 4 + 4 = 12
    assert_eq!(enc.get_entry_size(), 12);

    var1 = 1.0_f32;
    var2 = 0x1111;
    enc.encode_next_entry(&cfg, tb.get_timestamp(), &host); // ts=0
    tb.step(100);
    var1 = 2.0_f32;
    var2 = 0x2222;
    enc.encode_next_entry(&cfg, tb.get_timestamp(), &host); // ts=100
    tb.step(100);
    var1 = 3.0_f32;
    var2 = 0x3333;
    enc.encode_next_entry(&cfg, tb.get_timestamp(), &host); // ts=200

    assert_eq!(enc.get_entry_count(), 3);
    assert_eq!(enc.reader_total_size(), 36);

    let mut out = [0u8; 64];
    enc.reset_reader();
    let mut copied = 0usize;
    // Read in 10-byte chunks to exercise wrap-around handling.
    while !enc.reader_finished() {
        let n = enc.reader_read_chunk(&mut out[copied..copied + 10]);
        copied += n;
        if n == 0 { break; }
    }
    assert_eq!(copied, 36);

    // Verify each entry at byte offsets 0, 12, 24.
    for (i, (expected_f, expected_u, expected_ts)) in
        [(1.0_f32, 0x1111u32, 0u32), (2.0, 0x2222, 100), (3.0, 0x3333, 200)]
            .iter()
            .enumerate()
    {
        let base = i * 12;
        let got_f = f32::from_ne_bytes(out[base..base + 4].try_into().unwrap());
        let got_u = u32::from_ne_bytes(out[base + 4..base + 8].try_into().unwrap());
        let got_ts = u32::from_be_bytes(out[base + 8..base + 12].try_into().unwrap());
        assert_eq!(got_f, *expected_f, "entry {i}: var1");
        assert_eq!(got_u, *expected_u, "entry {i}: var2");
        assert_eq!(got_ts, *expected_ts, "entry {i}: timestamp");
    }
}

#[test]
fn rpv_callback_failure_zeros_entry() {
    let host = make_host(false); // callback always returns false
    let mut buf = [0xFFu8; 128]; // pre-fill with 0xFF so any non-zero residue is visible

    let mut cfg = Configuration::DEFAULT;
    cfg.items_count = 1;
    cfg.items_to_log[0] = LoggableItem::Rpv { id: TEST_RPV.id };

    let mut enc = RawFormatEncoder::new(&mut buf);
    enc.reset(&cfg, &host);
    assert!(!enc.error());

    enc.encode_next_entry(&cfg, 0, &host);

    // The 4-byte RPV entry must be zeroed.
    assert_eq!(&buf[..4], &[0, 0, 0, 0]);
}

#[test]
fn buffer_too_small_for_single_entry() {
    let host = make_host(true);
    let var1: u32 = 0x1234_5678;
    let var2: u32 = 0x9abc_def0;

    let mut cfg = Configuration::DEFAULT;
    cfg.items_count = 2;
    cfg.items_to_log[0] = LoggableItem::Memory { address: (&raw const var1).cast(), size: 4 };
    cfg.items_to_log[1] = LoggableItem::Memory { address: (&raw const var2).cast(), size: 4 };

    // Buffer holds only 4 bytes — not enough for one 8-byte entry.
    let mut small_buf = [0u8; 4];
    let mut enc = RawFormatEncoder::new(&mut small_buf);
    enc.reset(&cfg, &host);
    assert!(enc.error());

    // Keep addresses live so the compiler doesn't optimize them away.
    let _ = (&raw const var1, &raw const var2);
}
