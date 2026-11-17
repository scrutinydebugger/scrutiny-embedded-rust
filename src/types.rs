use core::cfg_select;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VariableTypeType {
    VTTsint = 0 << 4,
    VTTuint = 1 << 4,
    VTTfloat = 2 << 4,
    VTTboolean = 3 << 4,
    VTTcfloat = 4 << 4,
    VTTundef = 0xF << 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VariableTypeSize {
    VTS8 = 0,
    VTS16 = 1,
    VTS32 = 2,
    VTS64 = 3,
    VTS128 = 4,
    VTS256 = 5,
    VTSundef = 0xF,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VariableType {
    Sint8 = (VariableTypeType::VTTsint as u8) | (VariableTypeSize::VTS8 as u8),
    Uint8 = (VariableTypeType::VTTuint as u8) | (VariableTypeSize::VTS8 as u8),
    Float8 = (VariableTypeType::VTTfloat as u8) | (VariableTypeSize::VTS8 as u8),
    Cfloat8 = (VariableTypeType::VTTcfloat as u8) | (VariableTypeSize::VTS8 as u8),
    // C++ boolean16/32/64 (0x31-0x33) are not supported because a Rust `bool` is 1 byte; `try_from` rejects them.
    Boolean8 = (VariableTypeType::VTTboolean as u8) | (VariableTypeSize::VTS8 as u8),
    Sint16 = (VariableTypeType::VTTsint as u8) | (VariableTypeSize::VTS16 as u8),
    Uint16 = (VariableTypeType::VTTuint as u8) | (VariableTypeSize::VTS16 as u8),
    Float16 = (VariableTypeType::VTTfloat as u8) | (VariableTypeSize::VTS16 as u8),
    Cfloat16 = (VariableTypeType::VTTcfloat as u8) | (VariableTypeSize::VTS16 as u8),
    Sint32 = (VariableTypeType::VTTsint as u8) | (VariableTypeSize::VTS32 as u8),
    Uint32 = (VariableTypeType::VTTuint as u8) | (VariableTypeSize::VTS32 as u8),
    Float32 = (VariableTypeType::VTTfloat as u8) | (VariableTypeSize::VTS32 as u8),
    Cfloat32 = (VariableTypeType::VTTcfloat as u8) | (VariableTypeSize::VTS32 as u8),
    /// Size-unspecified boolean. Treated as a 1-byte boolean (identical to `Boolean8`)
    /// across the lib: same `type_size`, same `convert_to_compare_type` behavior,
    /// same encoding.
    Boolean = (VariableTypeType::VTTboolean as u8) | (VariableTypeSize::VTSundef as u8),
    #[cfg(feature = "support_64bits")]
    Sint64 = (VariableTypeType::VTTsint as u8) | (VariableTypeSize::VTS64 as u8),
    #[cfg(feature = "support_64bits")]
    Uint64 = (VariableTypeType::VTTuint as u8) | (VariableTypeSize::VTS64 as u8),
    #[cfg(feature = "support_64bits")]
    Float64 = (VariableTypeType::VTTfloat as u8) | (VariableTypeSize::VTS64 as u8),
    #[cfg(feature = "support_64bits")]
    Cfloat64 = (VariableTypeType::VTTcfloat as u8) | (VariableTypeSize::VTS64 as u8),
    Unknown = 0xFF,
}

/// Union of all supported variable types. Field access is unsafe.
#[derive(Clone, Copy)]
pub union AnyType {
    pub uint8: u8,
    pub uint16: u16,
    pub uint32: u32,
    pub sint8: i8,
    pub sint16: i16,
    pub sint32: i32,
    pub float32: f32,
    pub boolean: bool,
    #[cfg(feature = "support_64bits")]
    pub sint64: i64,
    #[cfg(feature = "support_64bits")]
    pub uint64: u64,
    #[cfg(feature = "support_64bits")]
    pub float64: f64,
}

cfg_select! {
    feature = "support_64bits" => {
        pub type UintBiggest = u64;
        pub type IntBiggest = i64;
        pub type FloatBiggest = f64;
        pub const BIGGEST_UINT: VariableType = VariableType::Uint64;
        pub const BIGGEST_SINT: VariableType = VariableType::Sint64;
        pub const BIGGEST_FLOAT: VariableType = VariableType::Float64;
    },
    _ => {
        pub type UintBiggest = u32;
        pub type IntBiggest = i32;
        pub type FloatBiggest = f32;
        pub const BIGGEST_UINT: VariableType = VariableType::Uint32;
        pub const BIGGEST_SINT: VariableType = VariableType::Sint32;
        pub const BIGGEST_FLOAT: VariableType = VariableType::Float32;
    }
}

#[derive(Clone, Copy)]
pub struct RuntimePublishedValue {
    pub id: u16,
    pub var_type: VariableType,
}

#[cfg(feature = "protected_regions")]
#[derive(Clone, Copy)]
pub struct AddressRange {
    pub start: *const u8,
    pub end: *const u8,
}

// Raw pointers are not auto-Send/Sync; these represent static hardware address constants, not ownership.
#[cfg(feature = "protected_regions")]
unsafe impl Send for AddressRange {}
#[cfg(feature = "protected_regions")]
unsafe impl Sync for AddressRange {}

impl TryFrom<u8> for VariableType {
    type Error = ();
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            x if x == Self::Sint8 as u8 => Ok(Self::Sint8),
            x if x == Self::Uint8 as u8 => Ok(Self::Uint8),
            x if x == Self::Float8 as u8 => Ok(Self::Float8),
            x if x == Self::Cfloat8 as u8 => Ok(Self::Cfloat8),
            x if x == Self::Boolean8 as u8 => Ok(Self::Boolean8),
            x if x == Self::Sint16 as u8 => Ok(Self::Sint16),
            x if x == Self::Uint16 as u8 => Ok(Self::Uint16),
            x if x == Self::Float16 as u8 => Ok(Self::Float16),
            x if x == Self::Cfloat16 as u8 => Ok(Self::Cfloat16),
            x if x == Self::Sint32 as u8 => Ok(Self::Sint32),
            x if x == Self::Uint32 as u8 => Ok(Self::Uint32),
            x if x == Self::Float32 as u8 => Ok(Self::Float32),
            x if x == Self::Cfloat32 as u8 => Ok(Self::Cfloat32),
            x if x == Self::Boolean as u8 => Ok(Self::Boolean),
            #[cfg(feature = "support_64bits")]
            x if x == Self::Sint64 as u8 => Ok(Self::Sint64),
            #[cfg(feature = "support_64bits")]
            x if x == Self::Uint64 as u8 => Ok(Self::Uint64),
            #[cfg(feature = "support_64bits")]
            x if x == Self::Float64 as u8 => Ok(Self::Float64),
            #[cfg(feature = "support_64bits")]
            x if x == Self::Cfloat64 as u8 => Ok(Self::Cfloat64),
            _ => Err(()),
        }
    }
}

/// A point-in-time snapshot used as a reference for elapsed-time calculations. Units: 100 ns ticks.
pub type Timestamp = u32;

/// A duration or time delta. Units: 100 ns ticks.
pub type Timediff = u32;

#[cfg(test)]
mod tests {
    use super::*;

    // boolean16/32/64 from the C++ library are not supported and must be rejected.
    #[test]
    fn test_try_from_rejects_sized_booleans() {
        assert_eq!(VariableType::try_from(0x31), Err(()));
        assert_eq!(VariableType::try_from(0x32), Err(()));
        assert_eq!(VariableType::try_from(0x33), Err(()));
    }
}
