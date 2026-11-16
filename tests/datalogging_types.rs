use scrutiny_embedded_rust::datalogging::{convert_to_compare_type, AnyValAndTypePair};
use scrutiny_embedded_rust::tools::{read_biggest_sint, read_biggest_uint};
use scrutiny_embedded_rust::types::{AnyType, VariableType, BIGGEST_SINT, BIGGEST_UINT};

#[test]
fn convert_to_compare_type_sint16() {
    let mut pair = AnyValAndTypePair {
        val: AnyType { sint16: -1000 },
        valtype: VariableType::Sint16,
    };
    convert_to_compare_type(&mut pair);
    assert_eq!(pair.valtype, BIGGEST_SINT);
    assert_eq!(read_biggest_sint(&pair.val), -1000);
}

#[test]
fn convert_to_compare_type_sint32() {
    let mut pair = AnyValAndTypePair {
        val: AnyType { sint32: -100_000 },
        valtype: VariableType::Sint32,
    };
    convert_to_compare_type(&mut pair);
    assert_eq!(pair.valtype, BIGGEST_SINT);
    assert_eq!(read_biggest_sint(&pair.val), -100_000);
}

#[test]
fn convert_to_compare_type_sint8() {
    let mut pair = AnyValAndTypePair {
        val: AnyType { sint8: -50 },
        valtype: VariableType::Sint8,
    };
    convert_to_compare_type(&mut pair);
    assert_eq!(pair.valtype, BIGGEST_SINT);
    assert_eq!(read_biggest_sint(&pair.val), -50);
}

#[test]
fn convert_to_compare_type_uint8() {
    let mut pair = AnyValAndTypePair {
        val: AnyType { uint8: 50 },
        valtype: VariableType::Uint8,
    };
    convert_to_compare_type(&mut pair);
    assert_eq!(pair.valtype, BIGGEST_UINT);
    assert_eq!(read_biggest_uint(&pair.val), 50);
}

#[test]
fn convert_to_compare_type_uint16() {
    let mut pair = AnyValAndTypePair {
        val: AnyType { uint16: 1000 },
        valtype: VariableType::Uint16,
    };
    convert_to_compare_type(&mut pair);
    assert_eq!(pair.valtype, BIGGEST_UINT);
    assert_eq!(read_biggest_uint(&pair.val), 1000);
}

#[test]
fn convert_to_compare_type_uint32() {
    let mut pair = AnyValAndTypePair {
        val: AnyType { uint32: 100_000 },
        valtype: VariableType::Uint32,
    };
    convert_to_compare_type(&mut pair);
    assert_eq!(pair.valtype, BIGGEST_UINT);
    assert_eq!(read_biggest_uint(&pair.val), 100_000);
}

#[cfg(feature = "support_64bits")]
#[test]
fn convert_to_compare_type_uint64() {
    let mut pair = AnyValAndTypePair {
        val: AnyType { uint64: 0x123456789ABC },
        valtype: VariableType::Uint64,
    };
    convert_to_compare_type(&mut pair);
    assert_eq!(pair.valtype, BIGGEST_UINT);
    assert_eq!(read_biggest_uint(&pair.val), 0x123456789ABC);
}

#[cfg(feature = "support_64bits")]
#[test]
fn convert_to_compare_type_sint64() {
    let mut pair = AnyValAndTypePair {
        val: AnyType { sint64: -0x123456789ABC },
        valtype: VariableType::Sint64,
    };
    convert_to_compare_type(&mut pair);
    assert_eq!(pair.valtype, BIGGEST_SINT);
    assert_eq!(read_biggest_sint(&pair.val), -0x123456789ABC);
}

#[cfg(feature = "support_64bits")]
#[test]
fn convert_to_compare_type_float64() {
    let mut pair = AnyValAndTypePair {
        val: AnyType { float64: 3.141_592_6 },
        valtype: VariableType::Float64,
    };
    convert_to_compare_type(&mut pair);
    assert_eq!(pair.valtype, VariableType::Float32);
    assert_eq!(unsafe { pair.val.float32 }, 3.141_592_6_f64 as f32);
}
