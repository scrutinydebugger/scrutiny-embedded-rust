use crate::types::{
    AnyType, FloatBiggest, IntBiggest, UintBiggest, VariableType, VariableTypeSize,
    VariableTypeType,
};

const CRC32_TABLE: [u32; 16] = [
    0x00000000, 0x1DB71064, 0x3B6E20C8, 0x26D930AC, 0x76DC4190, 0x6B6B51F4, 0x4DB26158, 0x5005713C,
    0xEDB88320, 0xF00F9344, 0xD6D6A3E8, 0xCB61B38C, 0x9B64C2B0, 0x86D3D2D4, 0xA00AE278, 0xBDBDF21C,
];

pub fn crc32(data: &[u8], size: usize, start_value: u32) -> u32 {
    let mut crc: u32 = !start_value;
    for i in 0..size {
        let byte = data[i];
        crc = CRC32_TABLE[((crc ^ (byte as u32)) & 0x0F) as usize] ^ (crc >> 4);
        crc = CRC32_TABLE[((crc ^ (byte >> 4) as u32) & 0x0F) as usize] ^ (crc >> 4);
    }
    !crc
}

/// Returns the byte size of a scrutiny variable type.
pub trait TypeSize {
    fn type_size(&self) -> u8;
}

impl TypeSize for VariableType {
    #[inline]
    fn type_size(&self) -> u8 {
        if *self == VariableType::Unknown {
            return 0;
        }
        if *self == VariableType::Boolean {
            return 1; // bool is always 1 byte in Rust; implementation-specific in C++
        }
        1 << ((*self as u8) & 0xF)
    }
}

impl TypeSize for VariableTypeSize {
    #[inline]
    fn type_size(&self) -> u8 {
        if *self == VariableTypeSize::VTSundef {
            return 0;
        }
        1 << ((*self as u8) & 0xF)
    }
}

pub enum Status {
    Success = 0,
    Error = 1,
}

#[inline]
pub fn get_var_type_type(vt: VariableType) -> VariableTypeType {
    unsafe { core::mem::transmute((vt as u8) & 0xF0) }
}

#[inline]
pub fn make_type(tt: VariableTypeType, ts: VariableTypeSize) -> VariableType {
    if tt == VariableTypeType::VTTundef || ts == VariableTypeSize::VTSundef {
        return VariableType::Unknown;
    }

    return unsafe { core::mem::transmute((tt as u8) | (ts as u8)) };
}

cfg_select! {
    feature = "support_64bits" => {
        #[inline]
        pub fn read_biggest_uint(v: &AnyType) -> UintBiggest {
            unsafe { v.uint64 }
        }

        #[inline]
        pub fn read_biggest_sint(v: &AnyType) -> IntBiggest{
            unsafe { v.sint64 }
        }

        #[inline]
        pub fn read_biggest_float(v: &AnyType) -> FloatBiggest{
            unsafe { v.float64 }
        }

        #[inline]
        pub fn set_biggest_sint(v: &mut AnyType, val: IntBiggest){
            v.sint64 = val;
        }

        #[inline]
        pub fn set_biggest_uint(v: &mut AnyType, val: UintBiggest){
            v.uint64 = val;
        }

        #[inline]
        pub fn set_biggest_float(v: &mut AnyType, val: FloatBiggest){
            v.float64 = val;
        }

    },
    _ => {

        #[inline]
        pub fn read_biggest_uint(v: &AnyType) -> UIntBiggest {
            unsafe { v.uint32 }
        }

        #[inline]
        pub fn read_biggest_sint(v: &AnyType) -> IntBiggest{
            unsafe { v.sint32 }
        }

        #[inline]
        pub fn read_biggest_float(v: &AnyType) -> FloatBiggest{
            unsafe { v.float32 }
        }

        #[inline]
        pub fn set_biggest_sint(v: &mut AnyType, val: IntBiggest){
            v.sint32 = val;
        }

        #[inline]
        pub fn set_biggest_uint(v: &mut AnyType, val: UintBiggest){
            v.uint32 = val;
        }

        #[inline]
        pub fn set_biggest_float(v: &mut AnyType, val: FloatBiggest){
            v.float32 = val;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32() {
        let data: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(crc32(&data, 10, 0), 622_876_539u32);
    }

    #[test]
    fn test_crc32_incremental() {
        // Splitting the input and chaining must yield the same result as a single pass.
        let data: &[u8] = &[1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let crc = crc32(&data[..5], 5, 0);
        let crc = crc32(&data[5..], 5, crc);
        assert_eq!(crc, 622_876_539u32);
    }

    #[test]
    fn test_get_type_size_from_vt() {
        assert_eq!(VariableType::Unknown.type_size(), 0);

        assert_eq!(VariableType::Boolean.type_size(), 1);
        assert_eq!(VariableType::Boolean8.type_size(), 1);

        assert_eq!(VariableType::Sint8.type_size(), 1);
        assert_eq!(VariableType::Uint8.type_size(), 1);
        assert_eq!(VariableType::Float8.type_size(), 1);
        assert_eq!(VariableType::Cfloat8.type_size(), 1);

        assert_eq!(VariableType::Sint16.type_size(), 2);
        assert_eq!(VariableType::Uint16.type_size(), 2);
        assert_eq!(VariableType::Float16.type_size(), 2);
        assert_eq!(VariableType::Cfloat16.type_size(), 2);

        assert_eq!(VariableType::Sint32.type_size(), 4);
        assert_eq!(VariableType::Uint32.type_size(), 4);
        assert_eq!(VariableType::Float32.type_size(), 4);
        assert_eq!(VariableType::Cfloat32.type_size(), 4);

        #[cfg(feature = "support_64bits")]
        {
            assert_eq!(VariableType::Sint64.type_size(), 8);
            assert_eq!(VariableType::Uint64.type_size(), 8);
            assert_eq!(VariableType::Float64.type_size(), 8);
            assert_eq!(VariableType::Cfloat64.type_size(), 8);
        }
    }

    #[test]
    fn test_get_type_size_from_vts() {
        assert_eq!(VariableTypeSize::VTSundef.type_size(), 0);
        assert_eq!(VariableTypeSize::VTS8.type_size(), 1);
        assert_eq!(VariableTypeSize::VTS16.type_size(), 2);
        assert_eq!(VariableTypeSize::VTS32.type_size(), 4);
        assert_eq!(VariableTypeSize::VTS64.type_size(), 8);
        assert_eq!(VariableTypeSize::VTS128.type_size(), 16);
    }

    #[test]
    fn test_make_type() {
        assert_eq!(
            make_type(VariableTypeType::VTTsint, VariableTypeSize::VTS8),
            VariableType::Sint8
        );
        assert_eq!(
            make_type(VariableTypeType::VTTuint, VariableTypeSize::VTS8),
            VariableType::Uint8
        );
        assert_eq!(
            make_type(VariableTypeType::VTTfloat, VariableTypeSize::VTS8),
            VariableType::Float8
        );
        assert_eq!(
            make_type(VariableTypeType::VTTcfloat, VariableTypeSize::VTS8),
            VariableType::Cfloat8
        );
        assert_eq!(
            make_type(VariableTypeType::VTTboolean, VariableTypeSize::VTS8),
            VariableType::Boolean8
        );

        assert_eq!(
            make_type(VariableTypeType::VTTsint, VariableTypeSize::VTS16),
            VariableType::Sint16
        );
        assert_eq!(
            make_type(VariableTypeType::VTTuint, VariableTypeSize::VTS16),
            VariableType::Uint16
        );
        assert_eq!(
            make_type(VariableTypeType::VTTfloat, VariableTypeSize::VTS16),
            VariableType::Float16
        );
        assert_eq!(
            make_type(VariableTypeType::VTTcfloat, VariableTypeSize::VTS16),
            VariableType::Cfloat16
        );

        assert_eq!(
            make_type(VariableTypeType::VTTsint, VariableTypeSize::VTS32),
            VariableType::Sint32
        );
        assert_eq!(
            make_type(VariableTypeType::VTTuint, VariableTypeSize::VTS32),
            VariableType::Uint32
        );
        assert_eq!(
            make_type(VariableTypeType::VTTfloat, VariableTypeSize::VTS32),
            VariableType::Float32
        );
        assert_eq!(
            make_type(VariableTypeType::VTTcfloat, VariableTypeSize::VTS32),
            VariableType::Cfloat32
        );

        #[cfg(feature = "support_64bits")]
        {
            assert_eq!(
                make_type(VariableTypeType::VTTsint, VariableTypeSize::VTS64),
                VariableType::Sint64
            );
            assert_eq!(
                make_type(VariableTypeType::VTTuint, VariableTypeSize::VTS64),
                VariableType::Uint64
            );
            assert_eq!(
                make_type(VariableTypeType::VTTfloat, VariableTypeSize::VTS64),
                VariableType::Float64
            );
            assert_eq!(
                make_type(VariableTypeType::VTTcfloat, VariableTypeSize::VTS64),
                VariableType::Cfloat64
            );
        }
    }
}
