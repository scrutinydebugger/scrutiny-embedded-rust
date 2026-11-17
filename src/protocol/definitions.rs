/// Magic bytes that must be present in a `CommControl/Discover` request payload.
pub const DISCOVER_MAGIC: [u8; 4] = [0x7E, 0x18, 0xFC, 0x68];

/// Magic bytes sent in every `CommControl/Connect` request and echoed in the response.
pub const CONNECT_MAGIC: [u8; 4] = [0x82, 0x90, 0x22, 0x66];

pub const MINIMUM_RX_BUFFER_SIZE: u16 = 32;
pub const MINIMUM_TX_BUFFER_SIZE: u16 = 32;
/// Leave a 16-byte margin so overflow detection works with 16-bit counters.
pub const MAXIMUM_RX_BUFFER_SIZE: u16 = 0xFFFF - 16;
pub const MAXIMUM_TX_BUFFER_SIZE: u16 = 0xFFFF - 16;

/// Inter-byte RX gap that resets the FSM, in 100 ns units.
pub const RX_TIMEOUT_100NS: u32 = crate::build_config::COMM_RX_TIMEOUT_US * 10;
/// Session heartbeat deadline, in 100 ns units.
pub const HEARTBEAT_TIMEOUT_100NS: u32 = crate::build_config::COMM_HEARTBEAT_TIMEOUT_US * 10;

/// Metadata for a fully-received and CRC-validated request from the server.
/// The raw payload bytes live in the caller-owned RX buffer.
pub struct Request {
    pub command_id: u8,
    pub subfunction_id: u8,
    pub(super) data_length: u16,
    pub(super) crc: u32,
}

impl Request {
    pub(super) const fn new() -> Self {
        Self {
            command_id: 0,
            subfunction_id: 0,
            data_length: 0,
            crc: 0,
        }
    }

    pub(super) const fn reset(&mut self) {
        self.command_id = 0;
        self.subfunction_id = 0;
        self.data_length = 0;
        self.crc = 0;
    }
}

/// Metadata for a response to be sent back to the server.
/// The payload bytes live in the caller-owned TX buffer.
pub struct Response {
    pub command_id: u8,
    pub subfunction_id: u8,
    pub response_code: u8,
    pub data_length: u16,
    pub(super) crc: u32,
}

impl Response {
    pub(super) const fn new() -> Self {
        Self {
            command_id: 0,
            subfunction_id: 0,
            response_code: 0,
            data_length: 0,
            crc: 0,
        }
    }

    pub const fn reset(&mut self) {
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

impl TryFrom<u8> for CommandId {
    type Error = ();
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            x if x == Self::GetInfo as u8 => Ok(Self::GetInfo),
            x if x == Self::CommControl as u8 => Ok(Self::CommControl),
            x if x == Self::MemoryControl as u8 => Ok(Self::MemoryControl),
            x if x == Self::UserCommand as u8 => Ok(Self::UserCommand),
            x if x == Self::DataLogControl as u8 => Ok(Self::DataLogControl),
            _ => Err(()),
        }
    }
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

impl TryFrom<u8> for CommControlSubfn {
    type Error = ();
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            x if x == Self::Discover as u8 => Ok(Self::Discover),
            x if x == Self::Heartbeat as u8 => Ok(Self::Heartbeat),
            x if x == Self::GetParams as u8 => Ok(Self::GetParams),
            x if x == Self::Connect as u8 => Ok(Self::Connect),
            x if x == Self::Disconnect as u8 => Ok(Self::Disconnect),
            _ => Err(()),
        }
    }
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

impl TryFrom<u8> for GetInfoSubfn {
    type Error = ();
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            x if x == Self::GetProtocolVersion as u8 => Ok(Self::GetProtocolVersion),
            x if x == Self::GetSoftwareId as u8 => Ok(Self::GetSoftwareId),
            x if x == Self::GetSupportedFeatures as u8 => Ok(Self::GetSupportedFeatures),
            x if x == Self::GetSpecialMemoryRegionCount as u8 => Ok(Self::GetSpecialMemoryRegionCount),
            x if x == Self::GetSpecialMemoryLocation as u8 => Ok(Self::GetSpecialMemoryLocation),
            x if x == Self::GetRuntimePublishedValuesCount as u8 => Ok(Self::GetRuntimePublishedValuesCount),
            x if x == Self::GetRuntimePublishedValuesDefinition as u8 => Ok(Self::GetRuntimePublishedValuesDefinition),
            x if x == Self::GetLoopCount as u8 => Ok(Self::GetLoopCount),
            x if x == Self::GetLoopDefinition as u8 => Ok(Self::GetLoopDefinition),
            _ => Err(()),
        }
    }
}

/// Memory region type used in `GetInfo/GetSpecialMemoryLocation` requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MemoryRegionType {
    ReadOnly = 0,
    Forbidden = 1,
}

impl TryFrom<u8> for MemoryRegionType {
    type Error = ();
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Self::ReadOnly),
            1 => Ok(Self::Forbidden),
            _ => Err(()),
        }
    }
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

impl TryFrom<u8> for MemoryControlSubfn {
    type Error = ();
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            x if x == Self::Read as u8 => Ok(Self::Read),
            x if x == Self::Write as u8 => Ok(Self::Write),
            x if x == Self::WriteMasked as u8 => Ok(Self::WriteMasked),
            x if x == Self::ReadRpv as u8 => Ok(Self::ReadRpv),
            x if x == Self::WriteRpv as u8 => Ok(Self::WriteRpv),
            _ => Err(()),
        }
    }
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

impl TryFrom<u8> for DataLogControlSubfn {
    type Error = ();
    fn try_from(val: u8) -> Result<Self, Self::Error> {
        match val {
            x if x == Self::GetSetup as u8 => Ok(Self::GetSetup),
            x if x == Self::ConfigureDatalog as u8 => Ok(Self::ConfigureDatalog),
            x if x == Self::ArmTrigger as u8 => Ok(Self::ArmTrigger),
            x if x == Self::DisarmTrigger as u8 => Ok(Self::DisarmTrigger),
            x if x == Self::GetStatus as u8 => Ok(Self::GetStatus),
            x if x == Self::GetAcquisitionMetadata as u8 => Ok(Self::GetAcquisitionMetadata),
            x if x == Self::ReadAcquisition as u8 => Ok(Self::ReadAcquisition),
            x if x == Self::ResetDatalogger as u8 => Ok(Self::ResetDatalogger),
            _ => Err(()),
        }
    }
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
