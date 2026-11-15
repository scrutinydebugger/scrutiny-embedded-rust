use crate::build_config::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};
use crate::loop_handler::{LoopHandler, LoopType};
use crate::software_id::SOFTWARE_ID;
use crate::tools::TypeSize;
use crate::types::{RuntimePublishedValue, VariableType};

use super::definitions::{ResponseCode, CONNECT_MAGIC, DISCOVER_MAGIC};

const MAX_DISPLAY_NAME_LENGTH: u8 = 64;
const MAX_LOOP_NAME_LENGTH: u8 = 32;

/// Encodes a `CommControl/Discover` response payload into `tx_buf`.
/// Returns `Ok(data_length)` on success, or `Err(ResponseCode)` on failure.
///
/// # Errors
/// Returns `Err(ResponseCode::InvalidRequest)` if `data` is not `DISCOVER_MAGIC`,
/// or `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
pub fn encode_response_comm_discover(
    data: &[u8],
    display_name: &'static str,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    if data != DISCOVER_MAGIC {
        return Err(ResponseCode::InvalidRequest);
    }

    let name_bytes = display_name.as_bytes();
    let name_len: u8 =
        u8::try_from(name_bytes.len().min(usize::from(MAX_DISPLAY_NAME_LENGTH))).unwrap_or(MAX_DISPLAY_NAME_LENGTH);
    let sw_id_len = u16::try_from(SOFTWARE_ID.len()).unwrap_or(u16::MAX);
    // Layout: [major(1), minor(1), software_id(sw_id_len), name_len(1), name(name_len)]
    let total_len: u16 = 2 + sw_id_len + 1 + u16::from(name_len);

    if tx_buf.len() < usize::from(total_len) {
        return Err(ResponseCode::Overflow);
    }

    tx_buf[0] = PROTOCOL_VERSION_MAJOR;
    tx_buf[1] = PROTOCOL_VERSION_MINOR;
    let sw_end = 2 + SOFTWARE_ID.len();
    tx_buf[2..sw_end].copy_from_slice(&SOFTWARE_ID);
    tx_buf[sw_end] = name_len;
    let name_len = usize::from(name_len);
    tx_buf[sw_end + 1..sw_end + 1 + name_len].copy_from_slice(&name_bytes[..name_len]);

    Ok(total_len)
}

/// Encodes a `CommControl/Heartbeat` response payload into `tx_buf`.
/// Returns `Ok(data_length)` on success, or `Err(ResponseCode)` on failure.
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
pub fn encode_response_comm_heartbeat(
    session_id: u32,
    challenge_response: u16,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    const DATALEN: u16 = 6;
    if tx_buf.len() < usize::from(DATALEN) {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0..4].copy_from_slice(&session_id.to_be_bytes());
    tx_buf[4..6].copy_from_slice(&challenge_response.to_be_bytes());
    Ok(DATALEN)
}

/// Encodes a `CommControl/GetParams` response payload into `tx_buf`.
/// Returns `Ok(data_length)` on success, or `Err(ResponseCode)` on failure.
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
#[allow(clippy::too_many_arguments)]
pub fn encode_response_comm_get_params(
    rx_buf_size: u16,
    tx_buf_size: u16,
    max_bitrate: u32,
    heartbeat_timeout_us: u32,
    comm_rx_timeout_us: u32,
    address_size: u8,
    char_bit: u8,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    const DATALEN: u16 = 2 + 2 + 4 + 4 + 4 + 1 + 1;
    if tx_buf.len() < usize::from(DATALEN) {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0..2].copy_from_slice(&rx_buf_size.to_be_bytes());
    tx_buf[2..4].copy_from_slice(&tx_buf_size.to_be_bytes());
    tx_buf[4..8].copy_from_slice(&max_bitrate.to_be_bytes());
    tx_buf[8..12].copy_from_slice(&heartbeat_timeout_us.to_be_bytes());
    tx_buf[12..16].copy_from_slice(&comm_rx_timeout_us.to_be_bytes());
    tx_buf[16] = address_size;
    tx_buf[17] = char_bit;
    Ok(DATALEN)
}

/// Encodes a `CommControl/Connect` response payload into `tx_buf`.
/// Payload: `CONNECT_MAGIC`(4) + `session_id`(4) = 8 bytes.
/// Returns `Ok(data_length)` on success, or `Err(ResponseCode)` on failure.
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
pub fn encode_response_comm_connect(session_id: u32, tx_buf: &mut [u8]) -> Result<u16, ResponseCode> {
    const DATALEN: u16 = 4 + 4;
    if tx_buf.len() < usize::from(DATALEN) {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0..4].copy_from_slice(&CONNECT_MAGIC);
    tx_buf[4..8].copy_from_slice(&session_id.to_be_bytes());
    Ok(DATALEN)
}

// ===== GetInfo encoders =====

/// Encodes a `GetInfo/GetProtocolVersion` response: major(1) + minor(1).
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
pub fn encode_response_get_info_protocol_version(tx_buf: &mut [u8]) -> Result<u16, ResponseCode> {
    const DATALEN: u16 = 2;
    if tx_buf.len() < usize::from(DATALEN) {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0] = PROTOCOL_VERSION_MAJOR;
    tx_buf[1] = PROTOCOL_VERSION_MINOR;
    Ok(DATALEN)
}

/// Encodes a `GetInfo/GetSoftwareId` response: the 16-byte firmware identifier.
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
pub fn encode_response_get_info_software_id(tx_buf: &mut [u8]) -> Result<u16, ResponseCode> {
    let datalen = SOFTWARE_ID.len();
    if tx_buf.len() < datalen {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[..datalen].copy_from_slice(&SOFTWARE_ID);
    Ok(u16::try_from(datalen).unwrap_or(u16::MAX))
}

/// Encodes a `GetInfo/GetSupportedFeatures` response: 1-byte bitfield.
/// Bit 7 = memory write, bit 6 = datalogging, bit 5 = user command, bit 4 = 64-bit support.
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
#[allow(clippy::fn_params_excessive_bools)]
pub const fn encode_response_get_info_supported_features(
    memory_write: bool,
    datalogging: bool,
    user_command: bool,
    support_64bits: bool,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    if tx_buf.is_empty() {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0] = (if memory_write { 0x80u8 } else { 0 })
        | (if datalogging { 0x40 } else { 0 })
        | (if user_command { 0x20 } else { 0 })
        | (if support_64bits { 0x10 } else { 0 });
    Ok(1)
}

/// Encodes a `GetInfo/GetSpecialMemoryRegionCount` response: 2 bytes (readonly count, forbidden count).
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
pub const fn encode_response_get_info_special_memory_region_count(
    readonly_count: u8,
    forbidden_count: u8,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    if tx_buf.len() < 2 {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0] = readonly_count;
    tx_buf[1] = forbidden_count;
    Ok(2)
}

/// Encodes a `GetInfo/GetSpecialMemoryLocation` response.
/// Wire layout: type(1) + index(1) + start address(ptr size) + end address(ptr size).
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
#[cfg(feature = "protected_regions")]
pub fn encode_response_get_info_special_memory_region_location(
    region_type: u8,
    region_index: u8,
    start: *const u8,
    end: *const u8,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    let addr_size = core::mem::size_of::<*const u8>();
    let datalen = 1 + 1 + addr_size + addr_size;
    if tx_buf.len() < datalen {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0] = region_type;
    tx_buf[1] = region_index;
    // Safety: casting a raw pointer to usize is defined — it gives the numeric address.
    let start_bytes = (start as usize).to_be_bytes();
    let end_bytes = (end as usize).to_be_bytes();
    tx_buf[2..2 + addr_size].copy_from_slice(&start_bytes);
    tx_buf[2 + addr_size..2 + 2 * addr_size].copy_from_slice(&end_bytes);
    Ok(u16::try_from(datalen).unwrap_or(u16::MAX))
}

/// Encodes a `GetInfo/GetRuntimePublishedValuesCount` response: count(2).
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
pub fn encode_response_get_info_rpv_count(count: u16, tx_buf: &mut [u8]) -> Result<u16, ResponseCode> {
    if tx_buf.len() < 2 {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0..2].copy_from_slice(&count.to_be_bytes());
    Ok(2)
}

/// Encodes a `GetInfo/GetRuntimePublishedValuesDefinition` response.
///
/// Writes `count` entries starting at `start_index`, each as id(2) + type(1).
/// The slice must be pre-validated so that `start_index + count <= rpvs.len()`.
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small to hold all entries.
pub fn encode_response_get_info_rpv_definitions(
    rpvs: &[RuntimePublishedValue],
    start_index: u16,
    count: u16,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    let start = usize::from(start_index);
    let end = start + usize::from(count);
    let mut cursor = 0usize;
    for rpv in &rpvs[start..end] {
        if cursor + 3 > tx_buf.len() {
            return Err(ResponseCode::Overflow);
        }
        // In Rust, bool is always 1 byte — map Boolean to Boolean8 for the protocol.
        let var_type = if rpv.var_type == VariableType::Boolean { VariableType::Boolean8 } else { rpv.var_type };
        tx_buf[cursor..cursor + 2].copy_from_slice(&rpv.id.to_be_bytes());
        tx_buf[cursor + 2] = var_type as u8;
        cursor += 3;
    }
    Ok(u16::try_from(cursor).unwrap_or(u16::MAX))
}

/// Encodes a `GetInfo/GetLoopCount` response: count(1).
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
pub const fn encode_response_get_info_loop_count(count: u8, tx_buf: &mut [u8]) -> Result<u16, ResponseCode> {
    if tx_buf.is_empty() {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0] = count;
    Ok(1)
}

/// Encodes a `GetInfo/GetLoopDefinition` response for the given loop handler.
///
/// Wire layout: loop\_id(1) + loop\_type(1) + attributes(1) + \[timestep\_100ns(4) if fixed\] + name\_len(1) + name.
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
pub fn encode_response_get_info_loop_definition(
    loop_id: u8,
    handler: &LoopHandler,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    // Minimum fixed header: loop_id + loop_type + attributes + name_len = 4 bytes
    if tx_buf.len() < 4 {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0] = loop_id;
    tx_buf[1] = handler.loop_type() as u8;
    tx_buf[2] = 0; // attributes: bit 7 = datalogging supported (always 0 until datalogging is implemented)

    let mut cursor = 3usize;
    match handler.loop_type() {
        LoopType::FixedFreq => {
            if cursor + 4 > tx_buf.len() {
                return Err(ResponseCode::Overflow);
            }
            tx_buf[cursor..cursor + 4].copy_from_slice(&handler.get_timestep_100ns().to_be_bytes());
            cursor += 4;
        }
        LoopType::VariableFreq => {}
    }

    let name_bytes = handler.get_name().as_bytes();
    // name_len is capped at MAX_LOOP_NAME_LENGTH (32), so it always fits in u8.
    let name_len: u8 = u8::try_from(name_bytes.len().min(usize::from(MAX_LOOP_NAME_LENGTH))).unwrap_or(MAX_LOOP_NAME_LENGTH);
    let name_len_usize = usize::from(name_len);
    if cursor + 1 + name_len_usize > tx_buf.len() {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[cursor] = name_len;
    cursor += 1;
    tx_buf[cursor..cursor + name_len_usize].copy_from_slice(&name_bytes[..name_len_usize]);
    cursor += name_len_usize;

    Ok(u16::try_from(cursor).unwrap_or(u16::MAX))
}

/// Returns the byte size of a `VariableType` as reported by the protocol (Boolean → 1).
#[must_use]
pub fn protocol_type_size(vt: VariableType) -> u8 {
    if vt == VariableType::Boolean { 1 } else { vt.type_size() }
}
