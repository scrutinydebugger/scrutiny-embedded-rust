/// Magic bytes that must be present in a `CommControl/Discover` request payload.
pub const DISCOVER_MAGIC: [u8; 4] = [0x7E, 0x18, 0xFC, 0x68];

pub const MINIMUM_RX_BUFFER_SIZE: u16 = 32;
pub const MINIMUM_TX_BUFFER_SIZE: u16 = 32;
/// Leave a 16-byte margin so overflow detection works with 16-bit counters.
pub const MAXIMUM_RX_BUFFER_SIZE: u16 = 0xFFFF - 16;
pub const MAXIMUM_TX_BUFFER_SIZE: u16 = 0xFFFF - 16;

/// Inter-byte RX gap that resets the FSM (50 ms in 100 ns units).
pub const RX_TIMEOUT_100NS: u32 = 50_000 * 10;
/// Session heartbeat deadline (5 s in 100 ns units).
pub const HEARTBEAT_TIMEOUT_100NS: u32 = 5_000_000 * 10;

/// A fully-received and CRC-validated request from the server.
/// `data` is a slice into the `CommHandler`'s RX buffer and is valid until
/// [`CommHandler::wait_next_request`] is called.
pub struct Request<'buf> {
    pub command_id: u8,
    pub subfunction_id: u8,
    pub data: &'buf [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CommandId {
    GetInfo = 0x01,
    CommControl = 0x02,
    MemoryControl = 0x03,
    UserCommand = 0x04,
    DataLogControl = 0x05,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ResponseCode {
    Ok = 0,
    InvalidRequest = 1,
    UnsupportedFeature = 2,
    Overflow = 3,
    Busy = 4,
    FailureToProceed = 5,
    Forbidden = 6,
    NoResponseToSend = 0xFE,
    ProcessAgain = 0xFF,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxError {
    None,
    Overflow,
    Disabled,
    InvalidCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxError {
    None,
    Overflow,
    Busy,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CommControlSubfn {
    Discover = 1,
    Heartbeat = 2,
    GetParams = 3,
    Connect = 4,
    Disconnect = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GetInfoSubfn {
    GetProtocolVersion = 1,
    GetSoftwareId = 2,
    GetSupportedFeatures = 3,
    GetSpecialMemoryRegionCount = 4,
    GetSpecialMemoryLocation = 5,
    GetRuntimePublishedValuesCount = 6,
    GetRuntimePublishedValuesDefinition = 7,
    GetLoopCount = 8,
    GetLoopDefinition = 9,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MemoryControlSubfn {
    Read = 1,
    Write = 2,
    WriteMasked = 3,
    ReadRpv = 4,
    WriteRpv = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DataLogControlSubfn {
    GetSetup = 1,
    ConfigureDatalog = 2,
    ArmTrigger = 3,
    DisarmTrigger = 4,
    GetStatus = 5,
    GetAcquisitionMetadata = 6,
    ReadAcquisition = 7,
    ResetDatalogger = 8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommState {
    Idle,
    Receiving,
    Transmitting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RxFsmState {
    WaitForCommand,
    WaitForSubfunction,
    WaitForLength,
    WaitForData,
    WaitForCrc,
    WaitForProcess,
    Error,
}
