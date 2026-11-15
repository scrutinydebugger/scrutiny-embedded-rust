use crate::build_config::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};
use crate::software_id::SOFTWARE_ID;

use super::definitions::{ResponseCode, CONNECT_MAGIC, DISCOVER_MAGIC};

const MAX_DISPLAY_NAME_LENGTH: u8 = 64;

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
