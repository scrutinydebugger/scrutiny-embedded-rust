use scrutiny_embedded_rust::datalogging::{fetch_operand, AnyValAndTypePair, DataloggerHost, Operand};
use scrutiny_embedded_rust::types::{AnyType, RuntimePublishedValue, VariableType};

static TEST_RPVS: [RuntimePublishedValue; 2] = [
    RuntimePublishedValue { id: 0x1234, var_type: VariableType::Uint32 },
    RuntimePublishedValue { id: 0x5678, var_type: VariableType::Float32 },
];

struct TestHost;

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
        match (rpv.id, rpv.var_type) {
            (0x1234, VariableType::Uint32) => { out.uint32 = 0xaabb_ccdd; true }
            (0x5678, VariableType::Float32) => { out.float32 = 3.141_592_6_f32; true }
            _ => false,
        }
    }
}

#[test]
fn fetch_literal() {
    let host = TestHost;
    let op = Operand::Literal(0.1234_f32);
    let mut out = AnyValAndTypePair::DEFAULT;
    assert!(fetch_operand(&host, &op, &mut out));
    assert_eq!(out.valtype, VariableType::Float32);
    assert_eq!(unsafe { out.val.float32 }, 0.1234_f32);
}

#[test]
fn fetch_var() {
    let host = TestHost;
    let my_var: f32 = 3.141_592_6_f32;
    let op = Operand::Var { addr: (&raw const my_var).cast(), datatype: VariableType::Float32 };
    let mut out = AnyValAndTypePair::DEFAULT;
    assert!(fetch_operand(&host, &op, &mut out));
    assert_eq!(out.valtype, VariableType::Float32);
    assert_eq!(unsafe { out.val.float32 }, my_var);
}

#[test]
fn fetch_rpv() {
    let host = TestHost;
    let mut out = AnyValAndTypePair::DEFAULT;

    let op = Operand::Rpv { id: 0x1234 };
    assert!(fetch_operand(&host, &op, &mut out));
    assert_eq!(out.valtype, VariableType::Uint32);
    assert_eq!(unsafe { out.val.uint32 }, 0xaabb_ccdd);

    let op = Operand::Rpv { id: 0x5678 };
    assert!(fetch_operand(&host, &op, &mut out));
    assert_eq!(out.valtype, VariableType::Float32);
    assert_eq!(unsafe { out.val.float32 }, 3.141_592_6_f32);
}

#[test]
fn fetch_inexistant_rpv() {
    let host = TestHost;
    let op = Operand::Rpv { id: 0xAAAA };
    let mut out = AnyValAndTypePair::DEFAULT;
    assert!(!fetch_operand(&host, &op, &mut out));
}

#[test]
fn fetch_varbit() {
    let host = TestHost;
    // 16-bit layout: [2 padding][11-bit signed][3 padding]
    // Bit layout (little-endian): bits 3..13 hold the value.
    // We store -1000 in an 11-bit two's-complement field at bitoffset=3, bitsize=11.
    let raw_val: i16 = -1000i16;
    // Reconstruct: shift left by 3 (bitoffset), leaving room for 3 LSBs of padding.
    let packed: u16 = ((raw_val as u16) << 3) & 0xFFF8;
    let packed_bytes = packed.to_le_bytes();

    let op = Operand::VarBit {
        addr: packed_bytes.as_ptr(),
        datatype: VariableType::Sint16,
        bitoffset: 3,
        bitsize: 11,
    };
    let mut out = AnyValAndTypePair::DEFAULT;
    assert!(fetch_operand(&host, &op, &mut out));
    assert_eq!(out.valtype, VariableType::Sint16);
    assert_eq!(unsafe { out.val.sint16 }, raw_val);
}

#[test]
fn fetch_bitfield_limits() {
    let host = TestHost;
    // 4 × i32 backing store, all zero.
    let store = [0i32; 4];
    let addr: *const u8 = store.as_ptr().cast();
    let mut out = AnyValAndTypePair::DEFAULT;

    let make = |bitoffset: u8, bitsize: u8| Operand::VarBit {
        addr,
        datatype: VariableType::Sint16,
        bitoffset,
        bitsize,
    };

    // bitoffset=31, bitsize=1 → reads a single bit from a 32-bit word: valid
    assert!(fetch_operand(&host, &make(31, 1), &mut out));

    // bitoffset=32, bitsize=1 → needs a 64-bit read
    {
        let result = fetch_operand(&host, &make(32, 1), &mut out);
        #[cfg(feature = "support_64bits")]
        assert!(result);
        #[cfg(not(feature = "support_64bits"))]
        assert!(!result);
    }

    // bitoffset=0, bitsize=32 → valid 32-bit field
    assert!(fetch_operand(&host, &make(0, 32), &mut out));

    // bitoffset=0, bitsize=33 → needs 64 bits
    {
        let result = fetch_operand(&host, &make(0, 33), &mut out);
        #[cfg(feature = "support_64bits")]
        assert!(result);
        #[cfg(not(feature = "support_64bits"))]
        assert!(!result);
    }

    // bitoffset=63, bitsize=1 → needs 64-bit read
    {
        let result = fetch_operand(&host, &make(63, 1), &mut out);
        #[cfg(feature = "support_64bits")]
        assert!(result);
        #[cfg(not(feature = "support_64bits"))]
        assert!(!result);
    }

    // bitoffset=64 → always invalid (exceeds UintBiggest width)
    assert!(!fetch_operand(&host, &make(64, 1), &mut out));

    // bitoffset=0, bitsize=64 → needs 64-bit read
    {
        let result = fetch_operand(&host, &make(0, 64), &mut out);
        #[cfg(feature = "support_64bits")]
        assert!(result);
        #[cfg(not(feature = "support_64bits"))]
        assert!(!result);
    }

    // bitoffset=0, bitsize=65 → always invalid
    assert!(!fetch_operand(&host, &make(0, 65), &mut out));

    // bitsize=0 → always invalid
    assert!(!fetch_operand(&host, &make(0, 0), &mut out));
}
