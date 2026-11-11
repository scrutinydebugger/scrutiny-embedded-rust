use crate::build_config::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};
use crate::software_id::SOFTWARE_ID;

use super::definitions::{Response, ResponseCode, DISCOVER_MAGIC};

const MAX_DISPLAY_NAME_LENGTH: u8 = 64;

/// Encodes a `CommControl/Discover` response into `response`.
pub fn encode_discover(data: &[u8], display_name: &'static str, response: &mut Response<'_>) -> ResponseCode {
    if data != DISCOVER_MAGIC {
        return ResponseCode::InvalidRequest;
    }

    let name_bytes = display_name.as_bytes();
    let name_len: u8 =
        u8::try_from(name_bytes.len().min(usize::from(MAX_DISPLAY_NAME_LENGTH))).unwrap_or(MAX_DISPLAY_NAME_LENGTH);
    let sw_id_len = u16::try_from(SOFTWARE_ID.len()).unwrap_or(u16::MAX);
    // Layout: [major(1), minor(1), software_id(sw_id_len), name_len(1), name(name_len)]
    let total_len: u16 = 2 + sw_id_len + 1 + u16::from(name_len);

    let payload = response.payload();
    if payload.len() < usize::from(total_len) {
        return ResponseCode::Overflow;
    }

    payload[0] = PROTOCOL_VERSION_MAJOR;
    payload[1] = PROTOCOL_VERSION_MINOR;
    let sw_end = 2 + SOFTWARE_ID.len();
    payload[2..sw_end].copy_from_slice(&SOFTWARE_ID);
    payload[sw_end] = name_len;
    let name_len = usize::from(name_len);
    payload[sw_end + 1..sw_end + 1 + name_len].copy_from_slice(&name_bytes[..name_len]);

    response.data_length = total_len;
    ResponseCode::Ok
}
