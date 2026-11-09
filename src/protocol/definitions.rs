/// Magic bytes that must be present in a `CommControl/Discover` request payload.
pub const DISCOVER_MAGIC: [u8; 4] = [0x7E, 0x18, 0xFC, 0x68];

pub const MINIMUM_RX_BUFFER_SIZE: u16 = 32;
pub const MINIMUM_TX_BUFFER_SIZE: u16 = 32;
/// Leave a 16-byte margin so overflow detection works with 16-bit counters.
pub const MAXIMUM_RX_BUFFER_SIZE: u16 = 0xFFFF - 16;
pub const MAXIMUM_TX_BUFFER_SIZE: u16 = 0xFFFF - 16;

/// Inter-byte RX gap that resets the FSM, in 100 ns units.
pub const RX_TIMEOUT_100NS: u32 = crate::build_config::COMM_RX_TIMEOUT_US * 10;
/// Session heartbeat deadline, in 100 ns units.
pub const HEARTBEAT_TIMEOUT_100NS: u32 = crate::build_config::COMM_HEARTBEAT_TIMEOUT_US * 10;

/// A fully-received and CRC-validated request from the server.
/// Owns the RX buffer; valid until [`CommHandler::wait_next_request`] is called.
pub struct Request<'buf> {
    pub command_id: u8,
    pub subfunction_id: u8,
    pub(super) data_length: u16,
    pub(super) crc: u32,
    pub(super) buffer: &'buf mut [u8],
}

impl<'buf> Request<'buf> {
    pub(super) const fn new(buffer: &'buf mut [u8]) -> Self {
        Self {
            command_id: 0,
            subfunction_id: 0,
            data_length: 0,
            crc: 0,
            buffer,
        }
    }

    /// Returns the received payload bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.buffer[..self.data_length as usize]
    }

    pub(super) const fn reset(&mut self) {
        self.command_id = 0;
        self.subfunction_id = 0;
        self.data_length = 0;
        self.crc = 0;
    }
}

/// A response to be sent back to the server.
/// Owns the TX buffer; fill fields and call [`CommHandler::send_response`].
pub struct Response<'buf> {
    pub command_id: u8,
    pub subfunction_id: u8,
    pub response_code: u8,
    pub data_length: u16,
    pub(super) crc: u32,
    pub(super) buffer: &'buf mut [u8],
}

impl<'buf> Response<'buf> {
    pub(super) const fn new(buffer: &'buf mut [u8]) -> Self {
        Self {
            command_id: 0,
            subfunction_id: 0,
            response_code: 0,
            data_length: 0,
            crc: 0,
            buffer,
        }
    }

    /// Returns the full TX buffer for writing response payload bytes.
    pub const fn payload(&mut self) -> &mut [u8] {
        self.buffer
    }

    pub(super) const fn reset(&mut self) {
        self.command_id = 0;
        self.subfunction_id = 0;
        self.response_code = 0;
        self.data_length = 0;
        self.crc = 0;
    }
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
