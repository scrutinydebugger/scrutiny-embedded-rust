use crate::build_config::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};
use crate::config::{RpvReadCallback, RpvWriteCallback};
use crate::loop_handler::{LoopHandler, LoopType};
use crate::software_id::SOFTWARE_ID;
use crate::tools::TypeSize;
use crate::types::{AnyType, RuntimePublishedValue, VariableType};

#[cfg(feature = "protected_regions")]
use crate::types::AddressRange;

use super::definitions::{ResponseCode, CONNECT_MAGIC, DISCOVER_MAGIC, MAXIMUM_TX_BUFFER_SIZE};

/// Byte size of a native pointer as transmitted in the protocol.
const ADDR_SIZE: usize = core::mem::size_of::<*const u8>();

/// Reads a big-endian encoded address from `buf[..ADDR_SIZE]` and returns it as a `usize`.
fn read_be_addr(buf: &[u8]) -> usize {
    let mut addr = 0usize;
    for &b in &buf[..ADDR_SIZE] {
        addr = (addr << 8) | usize::from(b);
    }
    addr
}

/// Writes `addr` as a big-endian encoded address into `buf[..ADDR_SIZE]`.
fn write_be_addr(addr: usize, buf: &mut [u8]) {
    buf[..ADDR_SIZE].copy_from_slice(&addr.to_be_bytes());
}

/// Encodes `val` big-endian into `buf[..type_size]`. Silently does nothing for unsupported sizes.
fn encode_anytype_be(val: AnyType, type_size: u8, buf: &mut [u8]) {
    match type_size {
        1 => buf[0] = unsafe { val.uint8 },
        2 => buf[0..2].copy_from_slice(&unsafe { val.uint16 }.to_be_bytes()),
        4 => buf[0..4].copy_from_slice(&unsafe { val.uint32 }.to_be_bytes()),
        #[cfg(feature = "support_64bits")]
        8 => buf[0..8].copy_from_slice(&unsafe { val.uint64 }.to_be_bytes()),
        _ => {}
    }
}

/// Decodes `type_size` big-endian bytes from `buf` into an `AnyType`.
const fn decode_anytype_be(buf: &[u8], type_size: u8) -> AnyType {
    let mut v = AnyType { uint8: 0 };
    match type_size {
        1 => v.uint8 = buf[0],
        2 => v.uint16 = u16::from_be_bytes([buf[0], buf[1]]),
        4 => v.uint32 = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]),
        #[cfg(feature = "support_64bits")]
        8 => v.uint64 = u64::from_be_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]]),
        _ => {}
    }
    v
}

/// Returns the first `RuntimePublishedValue` in `rpvs` whose `id` matches, or `None`.
fn find_rpv(id: u16, rpvs: &'static [RuntimePublishedValue]) -> Option<RuntimePublishedValue> {
    rpvs.iter().find(|r| r.id == id).copied()
}

/// Returns `true` if the byte range `[addr, addr + len)` overlaps any entry in `ranges`.
#[cfg(feature = "protected_regions")]
pub(crate) fn region_overlaps(addr: usize, len: usize, ranges: &[AddressRange]) -> bool {
    if len == 0 {
        return false;
    }
    let block_end = addr.saturating_add(len - 1);
    for range in ranges {
        let range_start = range.start as usize;
        let range_end = range.end as usize;
        if addr <= range_end && range_start <= block_end {
            return true;
        }
    }
    false
}

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
        let var_type = if rpv.var_type == VariableType::Boolean {
            VariableType::Boolean8
        } else {
            rpv.var_type
        };
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
    #[cfg(feature = "datalogging")]
    let attr = if handler.datalogging_allowed() { 0x80u8 } else { 0 };
    #[cfg(not(feature = "datalogging"))]
    let attr = 0u8;
    tx_buf[2] = attr;

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
    let name_len: u8 =
        u8::try_from(name_bytes.len().min(usize::from(MAX_LOOP_NAME_LENGTH))).unwrap_or(MAX_LOOP_NAME_LENGTH);
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
    if vt == VariableType::Boolean {
        1
    } else {
        vt.type_size()
    }
}

// ===== MemoryControl request parsers =====

/// Iterates over `[address(ADDR_SIZE) + length(2)]` blocks in a `MemoryControl/Read` request.
struct ReadBlockParser<'a> {
    data: &'a [u8],
    offset: usize,
    done: bool,
}

impl<'a> ReadBlockParser<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            done: false,
        }
    }
}

impl Iterator for ReadBlockParser<'_> {
    type Item = Result<(usize, u16), ResponseCode>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.offset >= self.data.len() {
            return None;
        }
        if self.offset + ADDR_SIZE + 2 > self.data.len() {
            self.done = true;
            return Some(Err(ResponseCode::InvalidRequest));
        }
        let addr = read_be_addr(&self.data[self.offset..]);
        let len = u16::from_be_bytes([
            self.data[self.offset + ADDR_SIZE],
            self.data[self.offset + ADDR_SIZE + 1],
        ]);
        self.offset += ADDR_SIZE + 2;
        Some(Ok((addr, len)))
    }
}

/// A parsed block from a `MemoryControl/Write` or `MemoryControl/WriteMasked` request.
struct WriteBlock<'a> {
    addr: usize,
    len: u16,
    src: &'a [u8],
    mask: Option<&'a [u8]>,
}

/// Iterates over `[address(ADDR_SIZE) + length(2) + data(length) [+ mask(length)]]` blocks
/// in a `MemoryControl/Write` or `MemoryControl/WriteMasked` request.
struct WriteBlockParser<'a> {
    data: &'a [u8],
    offset: usize,
    masked: bool,
    done: bool,
}

impl<'a> WriteBlockParser<'a> {
    const fn new(data: &'a [u8], masked: bool) -> Self {
        Self {
            data,
            offset: 0,
            masked,
            done: false,
        }
    }
}

impl<'a> Iterator for WriteBlockParser<'a> {
    type Item = Result<WriteBlock<'a>, ResponseCode>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.offset >= self.data.len() {
            return None;
        }
        let block_header = ADDR_SIZE + 2;
        if self.offset + block_header > self.data.len() {
            self.done = true;
            return Some(Err(ResponseCode::InvalidRequest));
        }
        let addr = read_be_addr(&self.data[self.offset..]);
        let len = u16::from_be_bytes([
            self.data[self.offset + ADDR_SIZE],
            self.data[self.offset + ADDR_SIZE + 1],
        ]);
        let len_usize = usize::from(len);
        let data_start = self.offset + block_header;
        let data_end = data_start + len_usize;

        if self.masked {
            let mask_end = data_end + len_usize;
            if mask_end > self.data.len() {
                self.done = true;
                return Some(Err(ResponseCode::InvalidRequest));
            }
            let src = &self.data[data_start..data_end];
            let mask = &self.data[data_end..mask_end];
            self.offset = mask_end;
            Some(Ok(WriteBlock {
                addr,
                len,
                src,
                mask: Some(mask),
            }))
        } else {
            if data_end > self.data.len() {
                self.done = true;
                return Some(Err(ResponseCode::InvalidRequest));
            }
            let src = &self.data[data_start..data_end];
            self.offset = data_end;
            Some(Ok(WriteBlock {
                addr,
                len,
                src,
                mask: None,
            }))
        }
    }
}

// ===== MemoryControl encoders =====

/// Encodes a `MemoryControl/Read` response.
///
/// Request `data` is a sequence of `[address(ADDR_SIZE) + length(2)]` blocks.
/// For each block the matching bytes are copied from memory into the response.
///
/// # Errors
/// - `InvalidRequest` if `data` is not a non-empty multiple of `ADDR_SIZE + 2`.
/// - `Overflow` if the encoded response would exceed `tx_buf`.
/// - `Forbidden` if any block touches a forbidden address range (requires `protected_regions` feature).
///
/// # Safety
/// Reads from the raw addresses encoded in `data`. The caller guarantees those
/// addresses are valid for reading for the requested byte count.
pub unsafe fn encode_response_memory_control_read(
    data: &[u8],
    tx_buf: &mut [u8],
    #[cfg(feature = "protected_regions")] forbidden: Option<&'static [AddressRange]>,
) -> Result<u16, ResponseCode> {
    let block_header = ADDR_SIZE + 2;

    if data.is_empty() || !data.len().is_multiple_of(block_header) {
        return Err(ResponseCode::InvalidRequest);
    }

    // Pass 1: validate and compute required TX size. No memory access.
    // Two-level check: exceeding the protocol maximum → InvalidRequest;
    // fitting within the protocol but not in this device's buffer → Overflow.
    let protocol_max = usize::from(MAXIMUM_TX_BUFFER_SIZE);
    let required_tx = ReadBlockParser::new(data).try_fold(0usize, |acc, item| {
        let (_, len) = item?;
        let next = acc.checked_add(block_header + usize::from(len)).ok_or(ResponseCode::InvalidRequest)?;
        if next > protocol_max {
            return Err(ResponseCode::InvalidRequest);
        }
        Ok(next)
    })?;
    if required_tx > tx_buf.len() {
        return Err(ResponseCode::Overflow);
    }

    // Pass 2: check protected regions, then read memory into the response.
    let mut tx_cursor = 0usize;
    for item in ReadBlockParser::new(data) {
        let (addr, len) = item?;
        let len_usize = usize::from(len);

        #[cfg(feature = "protected_regions")]
        if let Some(ranges) = forbidden {
            if region_overlaps(addr, len_usize, ranges) {
                return Err(ResponseCode::Forbidden);
            }
        }

        write_be_addr(addr, &mut tx_buf[tx_cursor..]);
        tx_cursor += ADDR_SIZE;
        tx_buf[tx_cursor..tx_cursor + 2].copy_from_slice(&len.to_be_bytes());
        tx_cursor += 2;
        // Safety: caller guarantees the address range is valid for reading.
        unsafe { core::ptr::copy_nonoverlapping(addr as *const u8, tx_buf[tx_cursor..].as_mut_ptr(), len_usize) };
        tx_cursor += len_usize;
    }

    Ok(u16::try_from(tx_cursor).unwrap_or(u16::MAX))
}

/// Encodes a `MemoryControl/Write` response and applies the writes.
///
/// Request `data` is a sequence of `[address(ADDR_SIZE) + length(2) + data(length)]` blocks.
/// Each block is written to the target address. The response echoes `[address + length]` per block.
///
/// # Errors
/// - `Forbidden` if `memory_write_enable` is `false`, or a block touches a forbidden/readonly region.
/// - `InvalidRequest` if `data` does not parse into a valid sequence of blocks.
/// - `Overflow` if the echo response exceeds `tx_buf`.
///
/// # Safety
/// Writes to the raw addresses encoded in `data`. The caller guarantees those
/// addresses are valid for writing for the requested byte count.
pub unsafe fn encode_response_memory_control_write(
    data: &[u8],
    tx_buf: &mut [u8],
    memory_write_enable: bool,
    #[cfg(feature = "protected_regions")] forbidden: Option<&'static [AddressRange]>,
    #[cfg(feature = "protected_regions")] readonly: Option<&'static [AddressRange]>,
) -> Result<u16, ResponseCode> {
    if !memory_write_enable {
        return Err(ResponseCode::Forbidden);
    }
    if data.is_empty() {
        return Err(ResponseCode::InvalidRequest);
    }

    let block_header = ADDR_SIZE + 2;

    // Pass 1: validate structure and compute required TX size. No memory access.
    let required_tx = WriteBlockParser::new(data, false).try_fold(0usize, |acc, item| {
        item?;
        acc.checked_add(block_header).ok_or(ResponseCode::Overflow)
    })?;
    if required_tx > tx_buf.len() {
        return Err(ResponseCode::Overflow);
    }

    // Pass 2: check protected regions, perform writes, encode response.
    let mut tx_cursor = 0usize;
    for item in WriteBlockParser::new(data, false) {
        let block = item?;
        let len_usize = usize::from(block.len);

        #[cfg(feature = "protected_regions")]
        {
            if let Some(ranges) = forbidden {
                if region_overlaps(block.addr, len_usize, ranges) {
                    return Err(ResponseCode::Forbidden);
                }
            }
            if let Some(ranges) = readonly {
                if region_overlaps(block.addr, len_usize, ranges) {
                    return Err(ResponseCode::Forbidden);
                }
            }
        }

        // Safety: caller guarantees the address range is valid for writing.
        unsafe { core::ptr::copy_nonoverlapping(block.src.as_ptr(), block.addr as *mut u8, len_usize) };

        write_be_addr(block.addr, &mut tx_buf[tx_cursor..]);
        tx_cursor += ADDR_SIZE;
        tx_buf[tx_cursor..tx_cursor + 2].copy_from_slice(&block.len.to_be_bytes());
        tx_cursor += 2;
    }

    Ok(u16::try_from(tx_cursor).unwrap_or(u16::MAX))
}

/// Encodes a `MemoryControl/WriteMasked` response and applies masked writes.
///
/// Request `data` is `[address(ADDR_SIZE) + length(2) + data(length) + mask(length)]` per block.
/// Each destination byte is updated as: `dst = (dst & !mask) | (src & mask)`.
///
/// # Errors
/// Same as `encode_response_memory_control_write`.
///
/// # Safety
/// Same as `encode_response_memory_control_write`.
pub unsafe fn encode_response_memory_control_write_masked(
    data: &[u8],
    tx_buf: &mut [u8],
    memory_write_enable: bool,
    #[cfg(feature = "protected_regions")] forbidden: Option<&'static [AddressRange]>,
    #[cfg(feature = "protected_regions")] readonly: Option<&'static [AddressRange]>,
) -> Result<u16, ResponseCode> {
    if !memory_write_enable {
        return Err(ResponseCode::Forbidden);
    }
    if data.is_empty() {
        return Err(ResponseCode::InvalidRequest);
    }

    let block_header = ADDR_SIZE + 2;

    // Pass 1: validate structure and compute required TX size. No memory access.
    let required_tx = WriteBlockParser::new(data, true).try_fold(0usize, |acc, item| {
        item?;
        acc.checked_add(block_header).ok_or(ResponseCode::Overflow)
    })?;
    if required_tx > tx_buf.len() {
        return Err(ResponseCode::Overflow);
    }

    // Pass 2: check protected regions, perform masked writes, encode response.
    let mut tx_cursor = 0usize;
    for item in WriteBlockParser::new(data, true) {
        let block = item?;
        let mask = block.mask.unwrap_or(&[]); // always Some for masked=true; unwrap_or for safety

        #[cfg(feature = "protected_regions")]
        {
            let len_usize = usize::from(block.len);
            if let Some(ranges) = forbidden {
                if region_overlaps(block.addr, len_usize, ranges) {
                    return Err(ResponseCode::Forbidden);
                }
            }
            if let Some(ranges) = readonly {
                if region_overlaps(block.addr, len_usize, ranges) {
                    return Err(ResponseCode::Forbidden);
                }
            }
        }

        // Safety: caller guarantees the address range is valid for reading and writing.
        unsafe {
            let dst = block.addr as *mut u8;
            for (i, (&src_byte, &mask_byte)) in block.src.iter().zip(mask.iter()).enumerate() {
                let ptr = dst.add(i);
                ptr.write((ptr.read() & !mask_byte) | (src_byte & mask_byte));
            }
        }

        write_be_addr(block.addr, &mut tx_buf[tx_cursor..]);
        tx_cursor += ADDR_SIZE;
        tx_buf[tx_cursor..tx_cursor + 2].copy_from_slice(&block.len.to_be_bytes());
        tx_cursor += 2;
    }

    Ok(u16::try_from(tx_cursor).unwrap_or(u16::MAX))
}

/// Encodes a `MemoryControl/ReadRpv` response.
///
/// Request `data` is a sequence of 2-byte RPV IDs. For each ID the read callback is called
/// and the value is written as `id(2) + value(type_size)` in the response.
///
/// # Errors
/// - `UnsupportedFeature` if the RPV list is missing or empty, or no read callback is configured.
/// - `InvalidRequest` if `data` is empty or its length is odd.
/// - `FailureToProceed` if an RPV ID is not found or the read callback returns `false`.
/// - `Overflow` if the response exceeds `tx_buf`.
pub fn encode_response_memory_control_read_rpv(
    data: &[u8],
    rpvs: Option<&'static [RuntimePublishedValue]>,
    read_cb: Option<RpvReadCallback>,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    let Some(rpvs) = rpvs else {
        return Err(ResponseCode::UnsupportedFeature);
    };
    if rpvs.is_empty() {
        return Err(ResponseCode::UnsupportedFeature);
    }
    let Some(read_cb) = read_cb else {
        return Err(ResponseCode::UnsupportedFeature);
    };

    if data.is_empty() {
        return Err(ResponseCode::InvalidRequest);
    }
    if !data.len().is_multiple_of(2) {
        return Err(ResponseCode::InvalidRequest);
    }

    let mut tx_cursor = 0usize;
    let mut offset = 0usize;
    while offset < data.len() {
        let id = u16::from_be_bytes([data[offset], data[offset + 1]]);
        offset += 2;

        let Some(rpv) = find_rpv(id, rpvs) else {
            return Err(ResponseCode::FailureToProceed);
        };

        let type_size = protocol_type_size(rpv.var_type);
        if type_size == 0 {
            return Err(ResponseCode::FailureToProceed);
        }

        if tx_cursor + 2 + usize::from(type_size) > tx_buf.len() {
            return Err(ResponseCode::Overflow);
        }

        let mut val = AnyType { uint8: 0 };
        if !read_cb(rpv, &mut val, None) {
            return Err(ResponseCode::FailureToProceed);
        }

        tx_buf[tx_cursor..tx_cursor + 2].copy_from_slice(&id.to_be_bytes());
        tx_cursor += 2;
        encode_anytype_be(val, type_size, &mut tx_buf[tx_cursor..]);
        tx_cursor += usize::from(type_size);
    }

    Ok(u16::try_from(tx_cursor).unwrap_or(u16::MAX))
}

/// Encodes a `MemoryControl/WriteRpv` response and calls the write callback for each RPV.
///
/// Request `data` is a sequence of `id(2) + value(type_size)` entries.
/// The response echoes `id(2) + type_size(1)` per entry.
///
/// # Errors
/// - `UnsupportedFeature` if the RPV list is missing or empty, or no write callback is configured.
/// - `InvalidRequest` if `data` is empty, does not parse into valid entries, or an RPV ID is not found.
/// - `FailureToProceed` if the write callback returns `false`.
/// - `Overflow` if the response exceeds `tx_buf`.
pub fn encode_response_memory_control_write_rpv(
    data: &[u8],
    rpvs: Option<&'static [RuntimePublishedValue]>,
    write_cb: Option<RpvWriteCallback>,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    let Some(rpvs) = rpvs else {
        return Err(ResponseCode::UnsupportedFeature);
    };
    if rpvs.is_empty() {
        return Err(ResponseCode::UnsupportedFeature);
    }
    let Some(write_cb) = write_cb else {
        return Err(ResponseCode::UnsupportedFeature);
    };

    if data.is_empty() {
        return Err(ResponseCode::InvalidRequest);
    }

    let mut tx_cursor = 0usize;
    let mut offset = 0usize;
    while offset < data.len() {
        if offset + 2 > data.len() {
            return Err(ResponseCode::InvalidRequest);
        }
        let id = u16::from_be_bytes([data[offset], data[offset + 1]]);
        offset += 2;

        let Some(rpv) = find_rpv(id, rpvs) else {
            // Can't determine how many bytes follow — parsing is impossible.
            return Err(ResponseCode::InvalidRequest);
        };

        let type_size = protocol_type_size(rpv.var_type);
        if type_size == 0 {
            return Err(ResponseCode::InvalidRequest);
        }
        if offset + usize::from(type_size) > data.len() {
            return Err(ResponseCode::InvalidRequest);
        }

        let val = decode_anytype_be(&data[offset..], type_size);
        offset += usize::from(type_size);

        if tx_cursor + 3 > tx_buf.len() {
            return Err(ResponseCode::Overflow);
        }

        if !write_cb(rpv, &val) {
            return Err(ResponseCode::FailureToProceed);
        }

        tx_buf[tx_cursor..tx_cursor + 2].copy_from_slice(&id.to_be_bytes());
        tx_buf[tx_cursor + 2] = type_size;
        tx_cursor += 3;
    }

    Ok(u16::try_from(tx_cursor).unwrap_or(u16::MAX))
}

// ===== DataLogControl encoders / decoders =====

#[cfg(feature = "datalogging")]
use crate::datalogging::{
    types::{Configuration, LoggableItem, Operand},
    EncodingType, RawFormatEncoder,
};
#[cfg(feature = "datalogging")]
use crate::tools::crc32;

/// Encodes a `DataLogControl/GetSetup` response: buffer\_size(4) + encoding(1) + max\_signal\_count(1).
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
#[cfg(feature = "datalogging")]
pub fn encode_response_datalogging_get_setup(
    buffer_size: u32,
    encoding: EncodingType,
    max_signal_count: u8,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    const DATALEN: usize = 4 + 1 + 1;
    if tx_buf.len() < DATALEN {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0..4].copy_from_slice(&buffer_size.to_be_bytes());
    tx_buf[4] = encoding as u8;
    tx_buf[5] = max_signal_count;
    #[allow(clippy::cast_possible_truncation)]
    Ok(DATALEN as u16)
}

/// Encodes a `DataLogControl/GetStatus` response: state(1) + bytes\_to\_complete(4) + write\_counter(4).
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
#[cfg(feature = "datalogging")]
pub fn encode_response_datalogging_get_status(
    state: u8,
    bytes_to_complete: u32,
    write_counter: u32,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    const DATALEN: usize = 1 + 4 + 4;
    if tx_buf.len() < DATALEN {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0] = state;
    tx_buf[1..5].copy_from_slice(&bytes_to_complete.to_be_bytes());
    tx_buf[5..9].copy_from_slice(&write_counter.to_be_bytes());
    #[allow(clippy::cast_possible_truncation)]
    Ok(DATALEN as u16)
}

/// Encodes a `DataLogControl/GetAcquisitionMetadata` response.
/// Wire layout: acq\_id(2) + config\_id(2) + entry\_count(4) + data\_size(4) + points\_after\_trigger(4).
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` is too small.
#[cfg(feature = "datalogging")]
#[allow(clippy::too_many_arguments)]
pub fn encode_response_datalogging_get_acquisition_metadata(
    acq_id: u16,
    config_id: u16,
    entry_count: u32,
    data_size: u32,
    points_after_trigger: u32,
    tx_buf: &mut [u8],
) -> Result<u16, ResponseCode> {
    const DATALEN: usize = 2 + 2 + 4 + 4 + 4;
    if tx_buf.len() < DATALEN {
        return Err(ResponseCode::Overflow);
    }
    tx_buf[0..2].copy_from_slice(&acq_id.to_be_bytes());
    tx_buf[2..4].copy_from_slice(&config_id.to_be_bytes());
    tx_buf[4..8].copy_from_slice(&entry_count.to_be_bytes());
    tx_buf[8..12].copy_from_slice(&data_size.to_be_bytes());
    tx_buf[12..16].copy_from_slice(&points_after_trigger.to_be_bytes());
    #[allow(clippy::cast_possible_truncation)]
    Ok(DATALEN as u16)
}

/// Encodes one chunk of a `DataLogControl/ReadAcquisition` response.
///
/// Wire layout: finished(1) + rolling\_counter(1) + acq\_id(2) + data\[..\] \[+ crc(4) when finished\].
/// Returns `(data_length, finished)`.
///
/// # Errors
/// Returns `Err(ResponseCode::Overflow)` if `tx_buf` has fewer than 8 bytes.
#[cfg(feature = "datalogging")]
pub fn encode_response_datalogging_read_acquisition(
    encoder: &mut RawFormatEncoder,
    rolling_counter: u8,
    acq_id: u16,
    crc: &mut u32,
    tx_buf: &mut [u8],
) -> Result<(u16, bool), ResponseCode> {
    const HEADER: usize = 4;
    const CRC_SIZE: usize = 4;
    if tx_buf.len() < HEADER + CRC_SIZE {
        return Err(ResponseCode::Overflow);
    }

    let available = tx_buf.len() - HEADER;
    let nread = encoder.reader_read_chunk(&mut tx_buf[HEADER..HEADER + available]);
    *crc = crc32(&tx_buf[HEADER..HEADER + nread], *crc);

    let finished = if encoder.reader_finished() && nread + HEADER + CRC_SIZE <= tx_buf.len() {
        tx_buf[HEADER + nread..HEADER + nread + CRC_SIZE].copy_from_slice(&crc.to_be_bytes());
        true
    } else {
        false
    };

    tx_buf[0] = u8::from(finished);
    tx_buf[1] = rolling_counter;
    tx_buf[2..4].copy_from_slice(&acq_id.to_be_bytes());

    let total = if finished {
        HEADER + nread + CRC_SIZE
    } else {
        HEADER + nread
    };
    Ok((u16::try_from(total).unwrap_or(u16::MAX), finished))
}

/// Decodes a `DataLogControl/Configure` request payload into `config`.
/// Returns `(loop_id, config_id)` on success.
///
/// # Errors
/// Returns `Err(ResponseCode)` if the payload is malformed or overflows limits.
#[cfg(feature = "datalogging")]
pub(crate) fn decode_datalogging_configure_request(
    data: &[u8],
    config: &mut Configuration,
) -> Result<(u8, u16), ResponseCode> {
    if data.len() < 16 {
        return Err(ResponseCode::InvalidRequest);
    }

    let loop_id = data[0];
    let config_id = u16::from_be_bytes([data[1], data[2]]);

    config.decimation = u16::from_be_bytes([data[3], data[4]]);
    config.probe_location = data[5];
    config.timeout_100ns = u32::from_be_bytes([data[6], data[7], data[8], data[9]]);
    config.trigger.condition = match crate::datalogging::types::SupportedTriggerConditions::try_from(data[10]) {
        Ok(c) => c,
        Err(()) => return Err(ResponseCode::InvalidRequest),
    };
    config.trigger.hold_time_100ns = u32::from_be_bytes([data[11], data[12], data[13], data[14]]);
    config.trigger.operand_count = data[15];

    if config.trigger.operand_count as usize > crate::datalogging::MAX_OPERANDS {
        return Err(ResponseCode::Overflow);
    }

    let mut cursor = 16usize;

    for i in 0..config.trigger.operand_count as usize {
        if data.len() < cursor + 1 {
            return Err(ResponseCode::InvalidRequest);
        }
        let optype = data[cursor];
        cursor += 1;

        config.trigger.operands[i] = match optype {
            0 => {
                // Literal: f32 big-endian
                if data.len() < cursor + 4 {
                    return Err(ResponseCode::InvalidRequest);
                }
                let bits = u32::from_be_bytes([data[cursor], data[cursor + 1], data[cursor + 2], data[cursor + 3]]);
                cursor += 4;
                Operand::Literal(f32::from_bits(bits))
            }
            1 => {
                // Var: type(1) + addr(ADDR_SIZE)
                if data.len() < cursor + 1 + ADDR_SIZE {
                    return Err(ResponseCode::InvalidRequest);
                }
                let Ok(datatype) = crate::types::VariableType::try_from(data[cursor]) else {
                    return Err(ResponseCode::InvalidRequest);
                };
                cursor += 1;
                let addr = read_be_addr(&data[cursor..]) as *const u8;
                cursor += ADDR_SIZE;
                Operand::Var { addr, datatype }
            }
            2 => {
                // VarBit: type(1) + addr(ADDR_SIZE) + bitoffset(1) + bitsize(1)
                if data.len() < cursor + 3 + ADDR_SIZE {
                    return Err(ResponseCode::InvalidRequest);
                }
                let Ok(datatype) = crate::types::VariableType::try_from(data[cursor]) else {
                    return Err(ResponseCode::InvalidRequest);
                };
                cursor += 1;
                let addr = read_be_addr(&data[cursor..]) as *const u8;
                cursor += ADDR_SIZE;
                let bitoffset = data[cursor];
                cursor += 1;
                let bitsize = data[cursor];
                cursor += 1;
                Operand::VarBit {
                    addr,
                    datatype,
                    bitoffset,
                    bitsize,
                }
            }
            3 => {
                // Rpv: id(2)
                if data.len() < cursor + 2 {
                    return Err(ResponseCode::InvalidRequest);
                }
                let id = u16::from_be_bytes([data[cursor], data[cursor + 1]]);
                cursor += 2;
                Operand::Rpv { id }
            }
            _ => return Err(ResponseCode::InvalidRequest),
        };
    }

    if data.len() < cursor + 1 {
        return Err(ResponseCode::InvalidRequest);
    }
    config.items_count = data[cursor];
    cursor += 1;

    if config.items_count as usize > crate::build_config::DATALOGGING_MAX_SIGNAL {
        return Err(ResponseCode::Overflow);
    }

    for i in 0..config.items_count as usize {
        if data.len() < cursor + 1 {
            return Err(ResponseCode::InvalidRequest);
        }
        let item_type = data[cursor];
        cursor += 1;

        config.items_to_log[i] = match item_type {
            0 => {
                // Memory: addr(ADDR_SIZE) + size(1)
                if data.len() < cursor + ADDR_SIZE + 1 {
                    return Err(ResponseCode::InvalidRequest);
                }
                let address = read_be_addr(&data[cursor..]) as *const u8;
                cursor += ADDR_SIZE;
                let size = data[cursor];
                cursor += 1;
                LoggableItem::Memory { address, size }
            }
            1 => {
                // Rpv: id(2)
                if data.len() < cursor + 2 {
                    return Err(ResponseCode::InvalidRequest);
                }
                let id = u16::from_be_bytes([data[cursor], data[cursor + 1]]);
                cursor += 2;
                LoggableItem::Rpv { id }
            }
            2 => LoggableItem::Time,
            _ => return Err(ResponseCode::InvalidRequest),
        };
    }

    if cursor != data.len() {
        return Err(ResponseCode::InvalidRequest);
    }

    Ok((loop_id, config_id))
}
