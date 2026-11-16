use crate::build_config::DATALOGGING_MAX_SIGNAL;
use crate::types::VariableType;

/// Maximum number of operands a trigger condition can use.
pub const MAX_OPERANDS: usize = 3;

/// Supported encoding formats for the acquisition buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EncodingType {
    Raw = 0,
}

/// A value paired with its runtime type, used during trigger evaluation.
#[derive(Clone, Copy)]
pub struct AnyValAndTypePair {
    pub val: crate::types::AnyType,
    pub valtype: VariableType,
}

impl AnyValAndTypePair {
    pub const DEFAULT: Self = Self {
        val: crate::types::AnyType { uint32: 0 },
        valtype: VariableType::Uint32,
    };
}

/// An operand in a trigger condition.
#[derive(Clone, Copy)]
pub enum Operand {
    /// A literal floating-point constant.
    Literal(f32),
    /// A variable read from a memory address with a given type.
    Var {
        addr: *const u8,
        datatype: VariableType,
    },
    /// A bitfield extracted from a variable at a memory address.
    VarBit {
        addr: *const u8,
        datatype: VariableType,
        bitoffset: u8,
        bitsize: u8,
    },
    /// A Runtime Published Value identified by its ID.
    Rpv { id: u16 },
}

impl Operand {
    const DEFAULT: Self = Operand::Literal(0.0);
}

// Safety: raw pointers in Operand are static/hardware memory addresses managed by the caller.
unsafe impl Send for Operand {}
unsafe impl Sync for Operand {}

/// Available trigger conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SupportedTriggerConditions {
    AlwaysTrue = 0,
    Equal = 1,
    NotEqual = 2,
    LessThan = 3,
    LessOrEqualThan = 4,
    GreaterThan = 5,
    GreaterOrEqualThan = 6,
    ChangeMoreThan = 7,
    IsWithin = 8,
}


/// Trigger configuration, including the condition and its operands.
#[derive(Clone, Copy)]
pub struct TriggerConfig {
    pub operands: [Operand; MAX_OPERANDS],
    /// How long (in 100 ns ticks) the condition must remain true before the trigger fires.
    pub hold_time_100ns: u32,
    pub operand_count: u8,
    pub condition: SupportedTriggerConditions,
}

impl TriggerConfig {
    pub const DEFAULT: Self = Self {
        operands: [Operand::DEFAULT; MAX_OPERANDS],
        hold_time_100ns: 0,
        operand_count: 0,
        condition: SupportedTriggerConditions::AlwaysTrue,
    };
}

/// Type of a loggable item.
#[derive(Clone, Copy)]
pub enum LoggableItem {
    /// Log raw bytes from a memory address.
    Memory { address: *const u8, size: u8 },
    /// Log a Runtime Published Value.
    Rpv { id: u16 },
    /// Log the current timebase timestamp.
    Time,
}

impl LoggableItem {
    pub const DEFAULT: Self = LoggableItem::Memory {
        address: core::ptr::null(),
        size: 0,
    };
}

// Safety: raw pointers in LoggableItem are static/hardware memory addresses managed by the caller.
unsafe impl Send for LoggableItem {}
unsafe impl Sync for LoggableItem {}

/// Full datalogging acquisition configuration.
pub struct Configuration {
    pub items_to_log: [LoggableItem; DATALOGGING_MAX_SIGNAL],
    pub trigger: TriggerConfig,
    /// If non-zero: maximum time in 100 ns ticks before the acquisition is considered complete.
    pub timeout_100ns: u32,
    /// Decimation factor. 1 = log every tick, 2 = log every other tick, etc.
    pub decimation: u16,
    pub items_count: u8,
    /// Where the trigger point sits in the acquisition window.
    /// 0 = trigger at left edge (all data after trigger), 255 = trigger at right edge.
    pub probe_location: u8,
}

impl Configuration {
    pub const DEFAULT: Self = Self {
        items_to_log: [LoggableItem::DEFAULT; DATALOGGING_MAX_SIGNAL],
        trigger: TriggerConfig::DEFAULT,
        timeout_100ns: 0,
        decimation: 1,
        items_count: 0,
        probe_location: 128,
    };
}

impl Default for Configuration {
    fn default() -> Self {
        Self::DEFAULT
    }
}
