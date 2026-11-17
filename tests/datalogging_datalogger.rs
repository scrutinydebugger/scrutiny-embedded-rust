use scrutiny_embedded_rust::datalogging::{
    Configuration, DataLogger, DataloggerHost, LoggableItem, Operand, State,
    SupportedTriggerConditions, TriggerConfig, MAX_OPERANDS,
};
use scrutiny_embedded_rust::timebase::Timebase;
use scrutiny_embedded_rust::types::{AnyType, RuntimePublishedValue, VariableType};

static TEST_RPVS: [RuntimePublishedValue; 3] = [
    RuntimePublishedValue { id: 0x1234, var_type: VariableType::Uint32 },
    RuntimePublishedValue { id: 0x5678, var_type: VariableType::Float32 },
    RuntimePublishedValue { id: 0x1000, var_type: VariableType::Uint32 },
];

struct TestHost {
    rpv_read_callback: fn(RuntimePublishedValue, &mut AnyType) -> bool,
}

impl DataloggerHost for TestHost {
    unsafe fn read_memory(&self, dst: *mut u8, src: *const u8, count: usize) -> bool {
        core::ptr::copy_nonoverlapping(src, dst, count);
        true
    }
    fn is_rpv_read_configured(&self) -> bool { true }
    fn rpv_exists(&self, id: u16) -> bool {
        TEST_RPVS.iter().any(|r| r.id == id)
    }
    fn get_rpv(&self, id: u16) -> Option<RuntimePublishedValue> {
        TEST_RPVS.iter().find(|r| r.id == id).copied()
    }
    fn read_rpv(&self, rpv: RuntimePublishedValue, out: &mut AnyType) -> bool {
        (self.rpv_read_callback)(rpv, out)
    }
}

static RPV_1000_VAL: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static TRIGGER_CB_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn rpv_read_callback(rpv: RuntimePublishedValue, out: &mut AnyType) -> bool {
    match (rpv.id, rpv.var_type) {
        (0x1234, VariableType::Uint32) => { out.uint32 = 0xaabb_ccdd; true }
        (0x5678, VariableType::Float32) => { out.float32 = 3.141_592_6_f32; true }
        (0x1000, VariableType::Uint32) => {
            out.uint32 = RPV_1000_VAL.load(core::sync::atomic::Ordering::Relaxed);
            true
        }
        _ => false,
    }
}

fn trigger_callback() {
    TRIGGER_CB_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

fn make_host() -> TestHost {
    TestHost { rpv_read_callback }
}

fn memory_operand_f32(addr: *const f32) -> Operand {
    Operand::Var { addr: addr.cast(), datatype: VariableType::Float32 }
}

fn literal_operand(val: f32) -> Operand {
    Operand::Literal(val)
}

fn make_trigger(cond: SupportedTriggerConditions, op0: Operand, op1: Operand) -> TriggerConfig {
    TriggerConfig {
        condition: cond,
        operand_count: 2,
        operands: [op0, op1, Operand::Literal(0.0)],
        hold_time_100ns: 0,
    }
}

fn read_f32_ne(data: &[u8]) -> f32 {
    f32::from_ne_bytes(data[..4].try_into().unwrap())
}
fn read_i32_ne(data: &[u8]) -> i32 {
    i32::from_ne_bytes(data[..4].try_into().unwrap())
}
fn read_u32_be(data: &[u8]) -> u32 {
    u32::from_be_bytes(data[..4].try_into().unwrap())
}

#[allow(unused_assignments)]
#[test]
fn trigger_basics() {
    let host = make_host();
    let tb = Timebase::new();
    let mut my_var: f32 = 0.0;
    let logged_var: f32 = 0.0;
    let mut dl_buf = [0u8; 128];

    // Safety: dl_buf outlives dl.
    let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
    {
        let cfg = dl.config_mut();
        *cfg = Configuration::DEFAULT;
        cfg.items_count = 1;
        cfg.items_to_log[0] = LoggableItem::Memory {
            address: (&raw const logged_var).cast(),
            size: core::mem::size_of_val(&logged_var) as u8,
        };
        cfg.decimation = 1;
        cfg.trigger = make_trigger(
            SupportedTriggerConditions::Equal,
            memory_operand_f32(&raw const my_var),
            literal_operand(3.141_592_6_f32),
        );
        cfg.probe_location = 128;
    }
    dl.configure(&tb, 0, &host);

    // Not armed: condition met but process() must not trigger.
    my_var = 3.141_592_6_f32;
    dl.process(&tb, &host);
    assert_eq!(dl.get_state(), State::Configured);

    dl.arm_trigger();
    my_var = 0.0;
    dl.process(&tb, &host);
    assert_eq!(dl.get_state(), State::Armed); // condition not met

    my_var = 3.141_592_6_f32;
    dl.process(&tb, &host);
    assert_eq!(dl.get_state(), State::Triggered); // hold_time=0 fires immediately
}

#[allow(unused_assignments)]
#[test]
fn trigger_hold_time() {
    let host = make_host();
    let mut tb = Timebase::new();
    let mut my_var: f32 = 0.0;
    let logged_var: f32 = 0.0;
    let mut dl_buf = [0u8; 128];

    // Safety: dl_buf outlives dl.
    let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
    {
        let cfg = dl.config_mut();
        *cfg = Configuration::DEFAULT;
        cfg.items_count = 1;
        cfg.items_to_log[0] = LoggableItem::Memory {
            address: (&raw const logged_var).cast(),
            size: core::mem::size_of_val(&logged_var) as u8,
        };
        cfg.decimation = 1;
        cfg.trigger = TriggerConfig {
            condition: SupportedTriggerConditions::Equal,
            operand_count: 2,
            operands: [
                memory_operand_f32(&raw const my_var),
                literal_operand(3.141_592_6_f32),
                Operand::Literal(0.0),
            ],
            hold_time_100ns: 100,
        };
        cfg.probe_location = 128;
    }
    dl.configure(&tb, 0, &host);
    dl.arm_trigger();

    // Condition false: no trigger.
    dl.process(&tb, &host);
    assert_eq!(dl.get_state(), State::Armed);

    // Condition becomes true: hold timer starts at timestamp=0.
    my_var = 3.141_592_6_f32;
    dl.process(&tb, &host);
    assert_eq!(dl.get_state(), State::Armed); // hold not elapsed yet

    tb.step(99);
    dl.process(&tb, &host);
    assert_eq!(dl.get_state(), State::Armed); // 99 ticks < 100

    tb.step(1);
    dl.process(&tb, &host);
    assert_eq!(dl.get_state(), State::Triggered); // 100 ticks >= hold_time
}

#[test]
fn configure_rejects_varbit_with_bitsize_zero() {
    let host = make_host();
    let tb = Timebase::new();
    let dummy: u32 = 0;
    let mut dl_buf = [0u8; 128];

    // Safety: dl_buf outlives dl.
    let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
    {
        let cfg = dl.config_mut();
        *cfg = Configuration::DEFAULT;
        cfg.items_count = 1;
        cfg.items_to_log[0] = LoggableItem::Memory {
            address: (&raw const dummy).cast(),
            size: 4,
        };
        cfg.decimation = 1;
        cfg.trigger = TriggerConfig {
            condition: SupportedTriggerConditions::GreaterThan,
            operand_count: 2,
            operands: [
                Operand::VarBit {
                    addr: (&raw const dummy).cast(),
                    datatype: VariableType::Uint32,
                    bitoffset: 0,
                    bitsize: 0,
                },
                Operand::Literal(0.0),
                Operand::Literal(0.0),
            ],
            hold_time_100ns: 0,
        };
    }
    dl.configure(&tb, 0, &host);
    assert_eq!(dl.get_state(), State::Error);
    assert!(!dl.config_valid());
}

#[allow(unused_assignments)]
#[test]
fn basic_acquisition() {
    let host = make_host();
    let mut tb = Timebase::new();
    let mut my_var: f32 = 0.0;
    let mut dl_buf = [0u8; 128];

    TRIGGER_CB_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);

    // Safety: dl_buf outlives dl.
    let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), Some(trigger_callback)) };
    {
        let cfg = dl.config_mut();
        *cfg = Configuration::DEFAULT;
        cfg.items_count = 1;
        cfg.items_to_log[0] = LoggableItem::Memory {
            address: (&raw const my_var).cast(),
            size: core::mem::size_of_val(&my_var) as u8,
        };
        cfg.decimation = 2;
        cfg.trigger = TriggerConfig {
            condition: SupportedTriggerConditions::GreaterThan,
            operand_count: 2,
            operands: [
                memory_operand_f32(&raw const my_var),
                literal_operand(100.0_f32),
                Operand::Literal(0.0),
            ],
            hold_time_100ns: 100,
        };
        cfg.probe_location = 128;
    }
    dl.configure(&tb, 0, &host);

    // Not armed — data is NOT acquired even after many ticks.
    dl.process(&tb, &host);
    tb.step(100);
    dl.process(&tb, &host);
    tb.step(100);
    assert!(!dl.data_acquired());

    my_var = 200.0_f32;
    for _ in 0..100 {
        dl.process(&tb, &host);
        tb.step(100);
        my_var += 1.0;
    }
    assert!(!dl.data_acquired());
    assert_eq!(TRIGGER_CB_COUNT.load(core::sync::atomic::Ordering::Relaxed), 0);

    dl.arm_trigger();
    for _ in 0..100 {
        dl.process(&tb, &host);
        tb.step(100);
        my_var += 1.0;
    }
    assert!(dl.data_acquired());
    assert_eq!(TRIGGER_CB_COUNT.load(core::sync::atomic::Ordering::Relaxed), 1);
}

#[allow(unused_assignments)]
#[test]
fn always_use_full_buffer() {
    let host = make_host();
    let mut tb = Timebase::new();
    let mut var1: f32 = 0.0;
    let mut var2: i32 = 0;
    let mut dl_buf = [0u8; 128];

    let buf_len = dl_buf.len();

    for probe_loop in 0u16..=255 {
        var1 = 0.0;
        var2 = 0;
        let probe_location = probe_loop as u8;

        // Safety: dl_buf outlives dl.
    let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
        {
            let cfg = dl.config_mut();
            *cfg = Configuration::DEFAULT;
            cfg.items_count = 4;
            cfg.items_to_log[0] = LoggableItem::Memory {
                address: (&raw const var1).cast(),
                size: 4,
            };
            cfg.items_to_log[1] = LoggableItem::Memory {
                address: (&raw const var2).cast(),
                size: 4,
            };
            cfg.items_to_log[2] = LoggableItem::Rpv { id: 0x1000 };
            cfg.items_to_log[3] = LoggableItem::Time;
            cfg.decimation = 1;
            cfg.probe_location = probe_location;
            cfg.trigger = TriggerConfig {
                condition: SupportedTriggerConditions::AlwaysTrue,
                operand_count: 0,
                operands: [Operand::Literal(0.0); MAX_OPERANDS],
                hold_time_100ns: 0,
            };
        }
        dl.configure(&tb, 0, &host);
        dl.arm_trigger();

        let max_loops = buf_len * 8;
        for _ in 0..max_loops {
            dl.process(&tb, &host);
            tb.step(10);
            if dl.data_acquired() {
                break;
            }
        }
        assert!(
            dl.data_acquired(),
            "probe_location={probe_location}: acquisition did not complete"
        );
        assert!(
            !dl.get_encoder().error(),
            "probe_location={probe_location}: encoder error"
        );

        let total = dl.get_encoder().reader_total_size();
        assert!(
            total * 10 >= buf_len * 9,
            "probe_location={probe_location}: only {total} / {buf_len} bytes used"
        );
    }
}

#[test]
fn acquire_time_correctly() {
    let host = make_host();
    let mut tb = Timebase::new();
    let mut dl_buf = [0u8; 128];
    let mut out_buf = [0u8; 256];
    let max_loops = dl_buf.len() * 8;

    // Safety: dl_buf outlives dl.
    let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
    {
        let cfg = dl.config_mut();
        *cfg = Configuration::DEFAULT;
        cfg.items_count = 1;
        cfg.items_to_log[0] = LoggableItem::Time;
        cfg.decimation = 1;
        cfg.probe_location = 128;
        cfg.trigger = TriggerConfig {
            condition: SupportedTriggerConditions::AlwaysTrue,
            operand_count: 0,
            operands: [Operand::Literal(0.0); MAX_OPERANDS],
            hold_time_100ns: 0,
        };
    }
    dl.configure(&tb, 0, &host);

    // Fill buffer first.
    let mut i = 0;
    while !dl.get_encoder().buffer_full() && i < max_loops {
        dl.process(&tb, &host);
        tb.step(5);
        i += 1;
    }
    dl.arm_trigger();
    dl.force_trigger();
    i = 0;
    while dl.get_state() != State::AcquisitionCompleted && i < max_loops {
        dl.process(&tb, &host);
        tb.step(5);
        i += 1;
    }
    dl.process(&tb, &host);
    assert_eq!(dl.get_state(), State::AcquisitionCompleted);

    let enc = dl.get_encoder_mut();
    enc.reset_reader();
    let mut copied = 0usize;
    while !enc.reader_finished() {
        copied += enc.reader_read_chunk(&mut out_buf[copied..]);
    }

    let entry_count = enc.get_entry_count();
    let entry_size = enc.get_entry_size();
    assert_eq!(entry_size, 4); // timestamp is 4 bytes

    for idx in 1..entry_count {
        let ts_cur = u32::from_be_bytes(out_buf[idx * 4..idx * 4 + 4].try_into().unwrap());
        let ts_prev =
            u32::from_be_bytes(out_buf[(idx - 1) * 4..(idx - 1) * 4 + 4].try_into().unwrap());
        assert_eq!(
            ts_cur.wrapping_sub(ts_prev),
            5,
            "entry #{idx}: expected Δtime=5, got {}",
            ts_cur.wrapping_sub(ts_prev)
        );
    }
}

#[allow(unused_assignments)]
#[test]
fn complex_acquisition() {
    let host = make_host();
    let mut tb = Timebase::new();
    let mut var1: f32 = 0.0;
    let mut var2: i32 = 0;
    let mut trigger_val: f32 = 0.0_f32;
    let mut dl_buf = [0u8; 128];
    let mut out_buf = [0u8; 132]; // slightly larger than dl_buf

    let buf_len = dl_buf.len();

    for probe_loop in 0u16..=255 {
        var1 = 0.0;
        var2 = 0;
        trigger_val = 0.0_f32;
        RPV_1000_VAL.store(0, core::sync::atomic::Ordering::Relaxed);
        let probe_location = probe_loop as u8;

        // Safety: dl_buf outlives dl.
    let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
        {
            let cfg = dl.config_mut();
            *cfg = Configuration::DEFAULT;
            cfg.items_count = 4;
            cfg.items_to_log[0] = LoggableItem::Memory {
                address: (&raw const var1).cast(),
                size: 4,
            };
            cfg.items_to_log[1] = LoggableItem::Memory {
                address: (&raw const var2).cast(),
                size: 4,
            };
            cfg.items_to_log[2] = LoggableItem::Rpv { id: 0x1000 };
            cfg.items_to_log[3] = LoggableItem::Time;
            cfg.decimation = 2;
            cfg.probe_location = probe_location;
            cfg.trigger = TriggerConfig {
                condition: SupportedTriggerConditions::Equal,
                operand_count: 2,
                operands: [
                    Operand::Var {
                        addr: (&raw const trigger_val).cast(),
                        datatype: VariableType::Float32,
                    },
                    Operand::Literal(100.0_f32),
                    Operand::Literal(0.0),
                ],
                hold_time_100ns: 50,
            };
        }
        dl.configure(&tb, probe_loop, &host);

        // Pre-arm pre-fill (100 steps of 10 ticks each).
        for _ in 0..100 {
            dl.process(&tb, &host);
            tb.step(10);
        }
        assert!(!dl.data_acquired());

        dl.arm_trigger();

        // 100 ticks pre-trigger, then set trigger_val.
        for _ in 0..100 {
            dl.process(&tb, &host);
            tb.step(10);
            var1 += 1.0_f32;
            var2 -= 1;
            RPV_1000_VAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }

        trigger_val = 100.0_f32;

        let mut var1_at_trigger = 0.0_f32;
        let mut var2_at_trigger = 0_i32;
        let mut rpv_at_trigger = 0_u32;

        for i in 0..500usize {
            dl.process(&tb, &host);
            if i == 5 {
                var1_at_trigger = var1;
                var2_at_trigger = var2;
                rpv_at_trigger = RPV_1000_VAL.load(core::sync::atomic::Ordering::Relaxed);
            }
            tb.step(10);
            var1 += 1.0_f32;
            var2 -= 1;
            RPV_1000_VAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }

        assert!(
            dl.data_acquired(),
            "probe_location={probe_location}: acquisition did not complete"
        );
        assert!(
            !dl.get_encoder().error(),
            "probe_location={probe_location}: encoder error"
        );

        // Read out the acquisition data.
        let enc = dl.get_encoder_mut();
        enc.reset_reader();
        let mut copied = 0usize;
        while !enc.reader_finished() {
            assert!(!enc.reader_error());
            let n = enc.reader_read_chunk(&mut out_buf[copied..]);
            copied += n;
            if n == 0 { break; }
        }

        assert!(
            copied * 10 >= buf_len * 9,
            "probe_location={probe_location}: only {copied}/{buf_len} bytes read"
        );

        let entry_count = enc.get_entry_count();
        let entry_size = enc.get_entry_size(); // should be 4+4+4+4=16
        assert_eq!(entry_size, 16);

        // Verify that consecutive entries have the expected deltas.
        // Memory items: f32 and i32 are stored native-endian.
        // RPV items: big-endian u32.
        // Time items: big-endian u32.
        for idx in 1..entry_count {
            let base_cur = idx * entry_size;
            let base_prev = (idx - 1) * entry_size;

            let v1_cur = read_f32_ne(&out_buf[base_cur..]);
            let v1_prev = read_f32_ne(&out_buf[base_prev..]);

            let v2_cur = read_i32_ne(&out_buf[base_cur + 4..]);
            let v2_prev = read_i32_ne(&out_buf[base_prev + 4..]);

            let rpv_cur = read_u32_be(&out_buf[base_cur + 8..]);
            let rpv_prev = read_u32_be(&out_buf[base_prev + 8..]);

            let t_cur = read_u32_be(&out_buf[base_cur + 12..]);
            let t_prev = read_u32_be(&out_buf[base_prev + 12..]);

            assert_eq!(
                v1_cur - v1_prev,
                2.0_f32,
                "probe_location={probe_location} entry {idx}: var1 delta"
            );
            assert_eq!(
                v2_cur - v2_prev,
                -2,
                "probe_location={probe_location} entry {idx}: var2 delta"
            );
            assert_eq!(
                rpv_cur.wrapping_sub(rpv_prev),
                2,
                "probe_location={probe_location} entry {idx}: rpv delta"
            );
            assert_eq!(
                t_cur.wrapping_sub(t_prev),
                20,
                "probe_location={probe_location} entry {idx}: time delta"
            );
        }

        // Verify trigger position.
        let lp_after = dl.log_points_after_trigger();
        let trigger_location = entry_count.saturating_sub(lp_after);
        let expected_trigger_loc =
            ((probe_location as f64 / 255.0) * (entry_count as f64 - 1.0)).round() as usize;

        let diff = if trigger_location >= expected_trigger_loc {
            trigger_location - expected_trigger_loc
        } else {
            expected_trigger_loc - trigger_location
        };
        assert!(
            diff <= 2,
            "probe_location={probe_location}: trigger at entry {trigger_location}, expected ≈{expected_trigger_loc} (diff={diff})"
        );

        // Verify values near the trigger point.
        // When probe_location=255, trigger_location == entry_count (no post-trigger data),
        // so there is no entry at that index to inspect — skip in that case.
        if trigger_location < entry_count {
            let mid_base = trigger_location * entry_size;
            let mid_v1 = read_f32_ne(&out_buf[mid_base..]);
            let mid_v2 = read_i32_ne(&out_buf[mid_base + 4..]);
            let mid_rpv = read_u32_be(&out_buf[mid_base + 8..]);

            assert!(
                (var1_at_trigger - mid_v1).abs() <= 10.0,
                "probe_location={probe_location}: var1 at trigger mismatch ({var1_at_trigger} vs {mid_v1})"
            );
            assert!(
                (var2_at_trigger - mid_v2).abs() <= 10,
                "probe_location={probe_location}: var2 at trigger mismatch ({var2_at_trigger} vs {mid_v2})"
            );
            assert!(
                rpv_at_trigger.abs_diff(mid_rpv) <= 10,
                "probe_location={probe_location}: rpv at trigger mismatch ({rpv_at_trigger} vs {mid_rpv})"
            );
        }
    }
}
