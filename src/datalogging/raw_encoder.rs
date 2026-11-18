use crate::datalogging::types::{Configuration, EncodingType, LoggableItem};
use crate::types::{AnyType, Timestamp, VariableType};

/// Encodes acquisition samples into a fixed-size circular byte buffer.
/// The reader cursor state is embedded here to avoid a self-referential struct.
pub struct RawFormatEncoder {
    buffer: *mut u8,
    buffer_len: usize,
    max_entries: usize,
    next_entry_write_index: usize,
    first_valid_entry_index: usize,
    /// Number of entries written since the last `reset_write_counter()` call.
    entry_write_counter: usize,
    /// Size of one entry in bytes.
    entry_size: usize,
    full: bool,
    error: bool,
    // ── Reader state ──────────────────────────────────────────────────────────
    reader_cursor: usize,
    reader_started: bool,
    reader_finished: bool,
}

impl RawFormatEncoder {
    /// Creates a new encoder backed by the given raw buffer.
    ///
    /// # Safety
    /// `buffer` must be valid for `buffer_len` bytes for the lifetime of the encoder.
    /// Call `reset()` with a valid configuration before first use.
    pub const unsafe fn new(buffer: *mut u8, buffer_len: usize) -> Self {
        Self {
            buffer,
            buffer_len,
            max_entries: 0,
            next_entry_write_index: 0,
            first_valid_entry_index: 0,
            entry_write_counter: 0,
            entry_size: 0,
            full: false,
            error: true, // error until reset() with a valid config
            reader_cursor: 0,
            reader_started: false,
            reader_finished: false,
        }
    }

    /// Recomputes geometry from `config` and resets all counters.
    /// `host` is used to look up RPV sizes.
    pub(crate) fn reset(&mut self, config: &Configuration, host: &dyn super::datalogger::DataloggerHost) {
        self.reset_write_counter();
        self.error = false;
        self.next_entry_write_index = 0;
        self.first_valid_entry_index = 0;
        self.entry_size = 0;
        self.full = false;
        self.max_entries = 0;

        if self.buffer_len == 0 {
            self.error = true;
            return;
        }

        for i in 0..config.items_count as usize {
            if self.error {
                break;
            }
            let elem_size = match config.items_to_log[i] {
                LoggableItem::Memory { size, .. } => size as usize,
                LoggableItem::Rpv { id } => {
                    if let Some(rpv) = host.get_rpv(id) {
                        use crate::tools::TypeSize;
                        rpv.var_type.type_size() as usize
                    } else {
                        self.error = true;
                        0
                    }
                }
                LoggableItem::Time => core::mem::size_of::<Timestamp>(),
            };
            if elem_size == 0 && !self.error {
                self.error = true;
            } else {
                self.entry_size += elem_size;
            }
        }

        if let Some(n) = self.buffer_len.checked_div(self.entry_size) {
            self.max_entries = n;
        } else {
            self.error = true;
        }

        if self.max_entries == 0 {
            self.error = true;
        }

        self.reset_reader();
    }

    /// Captures one sample from the host and appends it to the circular buffer.
    pub(crate) fn encode_next_entry(
        &mut self,
        config: &Configuration,
        timebase_ts: Timestamp,
        host: &dyn super::datalogger::DataloggerHost,
    ) {
        if self.error {
            return;
        }

        // Advance the read pointer if we are about to overwrite the oldest entry.
        if self.next_entry_write_index == self.first_valid_entry_index && self.full {
            self.first_valid_entry_index += 1;
            if self.first_valid_entry_index >= self.max_entries {
                self.first_valid_entry_index = 0;
            }
        }

        let mut cursor = self.next_entry_write_index * self.entry_size;
        for i in 0..config.items_count as usize {
            match config.items_to_log[i] {
                LoggableItem::Memory { address, size } => {
                    let count = size as usize;
                    // Safety: buffer is valid for buffer_len bytes (encoder invariant);
                    // cursor+count is within bounds because entry_size was validated in reset().
                    unsafe {
                        host.read_memory(self.buffer.add(cursor), address, count);
                    }
                    cursor += count;
                }
                LoggableItem::Rpv { id } => {
                    let Some(rpv) = host.get_rpv(id) else {
                        self.error = true;
                        return;
                    };
                    let mut val = AnyType { uint32: 0 };
                    if !host.read_rpv(rpv, &mut val) {
                        val = AnyType { uint32: 0 };
                    }
                    // Safety: buffer valid; remaining capacity validated in reset().
                    let dst =
                        unsafe { core::slice::from_raw_parts_mut(self.buffer.add(cursor), self.buffer_len - cursor) };
                    let written = encode_anytype_be(val, rpv.var_type, dst);
                    cursor += written;
                }
                LoggableItem::Time => {
                    let ts_bytes = timebase_ts.to_be_bytes();
                    // Safety: buffer valid; 4 bytes fit within entry boundary.
                    unsafe {
                        core::ptr::copy_nonoverlapping(ts_bytes.as_ptr(), self.buffer.add(cursor), 4);
                    }
                    cursor += core::mem::size_of::<Timestamp>();
                }
            }
        }

        self.next_entry_write_index += 1;
        if self.next_entry_write_index >= self.max_entries {
            self.full = true;
            self.next_entry_write_index = 0;
        }

        self.entry_write_counter += 1;
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    #[must_use]
    #[inline]
    pub const fn error(&self) -> bool {
        self.error
    }

    #[must_use]
    #[inline]
    pub const fn buffer_full(&self) -> bool {
        self.full
    }

    /// Number of valid entries currently held in the circular buffer.
    #[must_use]
    #[inline]
    pub const fn get_entry_count(&self) -> usize {
        if self.full {
            self.max_entries
        } else {
            self.next_entry_write_index
        }
    }

    /// Total bytes occupied by valid entries.
    #[must_use]
    #[inline]
    pub const fn get_buffer_effective_size(&self) -> usize {
        self.entry_size * self.max_entries
    }

    /// Byte offset of the oldest valid entry (start of the read window).
    #[must_use]
    #[inline]
    pub const fn get_read_cursor(&self) -> usize {
        self.first_valid_entry_index * self.entry_size
    }

    /// Byte offset one past the most recently written entry.
    #[must_use]
    #[inline]
    pub const fn get_write_cursor(&self) -> usize {
        self.next_entry_write_index * self.entry_size
    }

    /// Bytes written since the last `reset_write_counter()` call.
    #[must_use]
    #[inline]
    pub const fn get_data_write_counter(&self) -> usize {
        self.entry_write_counter * self.entry_size
    }

    /// Entry-level write count since the last `reset_write_counter()` call.
    #[must_use]
    #[inline]
    pub const fn get_entry_write_counter(&self) -> usize {
        self.entry_write_counter
    }

    #[must_use]
    #[inline]
    pub const fn get_entry_size(&self) -> usize {
        self.entry_size
    }

    #[must_use]
    #[inline]
    pub const fn get_encoding() -> EncodingType {
        EncodingType::Raw
    }

    /// Resets only the write counter (called at trigger time).
    #[inline]
    pub const fn reset_write_counter(&mut self) {
        self.entry_write_counter = 0;
    }

    /// How many more bytes must be written to reach a full buffer.
    #[must_use]
    #[inline]
    pub const fn remaining_bytes_to_full(&self) -> usize {
        if self.full {
            0
        } else {
            self.get_buffer_effective_size().saturating_sub(self.get_write_cursor())
        }
    }

    // ── Reader ────────────────────────────────────────────────────────────────

    /// Resets the reader to the start of the acquisition data.
    pub const fn reset_reader(&mut self) {
        self.reader_started = false;
        self.reader_finished = false;
        self.reader_cursor = self.get_read_cursor();
    }

    #[must_use]
    #[inline]
    pub const fn reader_finished(&self) -> bool {
        self.reader_finished
    }

    #[must_use]
    #[inline]
    pub const fn reader_error(&self) -> bool {
        self.error
    }

    /// Total bytes that the reader will produce over all calls.
    #[must_use]
    #[inline]
    pub const fn reader_total_size(&self) -> usize {
        self.get_entry_count() * self.entry_size
    }

    /// Reads the next chunk of acquisition data into `out`. Returns the number of bytes written.
    /// Handles circular-buffer wrap-around transparently.
    pub fn reader_read_chunk(&mut self, out: &mut [u8]) -> usize {
        if self.error || self.reader_finished || out.is_empty() {
            return 0;
        }

        let write_cursor = self.get_write_cursor();
        let buffer_end = self.get_buffer_effective_size();

        if self.reader_cursor == write_cursor && self.reader_started {
            self.reader_finished = true;
            return 0;
        }

        let mut out_written = 0usize;

        while out_written < out.len() {
            // Determine how far we can read in a contiguous chunk.
            let right_end = if write_cursor > self.reader_cursor {
                write_cursor
            } else {
                buffer_end
            };
            let available = right_end - self.reader_cursor;
            let want = out.len() - out_written;
            let transfer = available.min(want);

            if transfer == 0 {
                break;
            }

            // Safety: reader_cursor+transfer is within buffer_len (validated above).
            unsafe {
                core::ptr::copy_nonoverlapping(
                    self.buffer.add(self.reader_cursor),
                    out[out_written..].as_mut_ptr(),
                    transfer,
                );
            }

            self.reader_cursor += transfer;
            self.reader_started = true;
            out_written += transfer;

            if self.reader_cursor >= buffer_end {
                self.reader_cursor -= buffer_end;
            }

            if self.reader_cursor == write_cursor {
                self.reader_finished = true;
                break;
            }
        }

        out_written
    }
}

// ── Big-endian encoding helper ────────────────────────────────────────────────

/// Encodes `val` of `var_type` as big-endian bytes into `dst`.
/// Returns the number of bytes written (= `var_type.type_size()`).
fn encode_anytype_be(val: AnyType, var_type: VariableType, dst: &mut [u8]) -> usize {
    use crate::tools::TypeSize;
    // Safety: the caller guarantees var_type matches what was placed in `val`.
    unsafe {
        match var_type {
            // Booleans are copied as a raw byte: the callback may store a value that is not a valid `bool`.
            VariableType::Uint8 | VariableType::Boolean8 | VariableType::Boolean => {
                dst[0] = val.uint8;
                1
            }
            VariableType::Sint8 => {
                dst[0] = val.sint8.cast_unsigned();
                1
            }
            VariableType::Uint16 => {
                dst[..2].copy_from_slice(&val.uint16.to_be_bytes());
                2
            }
            VariableType::Sint16 => {
                dst[..2].copy_from_slice(&val.sint16.to_be_bytes());
                2
            }
            VariableType::Uint32 => {
                dst[..4].copy_from_slice(&val.uint32.to_be_bytes());
                4
            }
            VariableType::Sint32 => {
                dst[..4].copy_from_slice(&val.sint32.to_be_bytes());
                4
            }
            VariableType::Float32 => {
                dst[..4].copy_from_slice(&val.float32.to_bits().to_be_bytes());
                4
            }
            #[cfg(feature = "support_64bits")]
            VariableType::Uint64 => {
                dst[..8].copy_from_slice(&val.uint64.to_be_bytes());
                8
            }
            #[cfg(feature = "support_64bits")]
            VariableType::Sint64 => {
                dst[..8].copy_from_slice(&val.sint64.to_be_bytes());
                8
            }
            #[cfg(feature = "support_64bits")]
            VariableType::Float64 => {
                dst[..8].copy_from_slice(&val.float64.to_bits().to_be_bytes());
                8
            }
            _ => var_type.type_size() as usize, // 0 for unknown — no-op
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datalogging::datalogger::DataloggerHost;
    use crate::datalogging::types::{Configuration, LoggableItem};
    use crate::timebase::Timebase;
    use crate::types::{AnyType, RuntimePublishedValue, VariableType};

    static TEST_RPV: RuntimePublishedValue = RuntimePublishedValue {
        id: 0x1234,
        var_type: VariableType::Uint32,
    };

    struct TestHost {
        rpv_ok: bool,
    }

    impl DataloggerHost for TestHost {
        unsafe fn read_memory(&self, dst: *mut u8, src: *const u8, count: usize) -> bool {
            core::ptr::copy_nonoverlapping(src, dst, count);
            true
        }
        fn is_rpv_read_configured(&self) -> bool {
            true
        }
        fn rpv_exists(&self, id: u16) -> bool {
            id == TEST_RPV.id
        }
        fn get_rpv(&self, id: u16) -> Option<RuntimePublishedValue> {
            if id == TEST_RPV.id {
                Some(TEST_RPV)
            } else {
                None
            }
        }
        fn read_rpv(&self, _rpv: RuntimePublishedValue, out: &mut AnyType) -> bool {
            if self.rpv_ok {
                out.uint32 = 0xaabb_ccdd;
                true
            } else {
                false
            }
        }
    }

    fn make_host(rpv_ok: bool) -> TestHost {
        TestHost { rpv_ok }
    }

    #[allow(unused_assignments)]
    #[test]
    fn basic_encoding() {
        let host = make_host(true);
        let mut tb = Timebase::new();
        let mut var1: f32 = 0.0;
        let mut var2: u32 = 0;
        let mut buf = [0u8; 128];

        let mut cfg = Configuration::DEFAULT;
        cfg.items_count = 3;
        cfg.items_to_log[0] = LoggableItem::Memory {
            address: (&raw const var1).cast(),
            size: 4,
        };
        cfg.items_to_log[1] = LoggableItem::Memory {
            address: (&raw const var2).cast(),
            size: 4,
        };
        cfg.items_to_log[2] = LoggableItem::Time;

        // Safety: buf outlives enc.
        let mut enc = unsafe { RawFormatEncoder::new(buf.as_mut_ptr(), buf.len()) };
        enc.reset(&cfg, &host);
        assert!(!enc.error());

        // entry_size = 4 + 4 + 4 = 12
        assert_eq!(enc.get_entry_size(), 12);

        var1 = 1.0_f32;
        var2 = 0x1111;
        enc.encode_next_entry(&cfg, tb.get_timestamp(), &host); // ts=0
        tb.step(100);
        var1 = 2.0_f32;
        var2 = 0x2222;
        enc.encode_next_entry(&cfg, tb.get_timestamp(), &host); // ts=100
        tb.step(100);
        var1 = 3.0_f32;
        var2 = 0x3333;
        enc.encode_next_entry(&cfg, tb.get_timestamp(), &host); // ts=200

        assert_eq!(enc.get_entry_count(), 3);
        assert_eq!(enc.reader_total_size(), 36);

        let mut out = [0u8; 64];
        enc.reset_reader();
        let mut copied = 0usize;
        // Read in 10-byte chunks to exercise wrap-around handling.
        while !enc.reader_finished() {
            let n = enc.reader_read_chunk(&mut out[copied..copied + 10]);
            copied += n;
            if n == 0 {
                break;
            }
        }
        assert_eq!(copied, 36);

        // Verify each entry at byte offsets 0, 12, 24.
        for (i, (expected_f, expected_u, expected_ts)) in
            [(1.0_f32, 0x1111u32, 0u32), (2.0, 0x2222, 100), (3.0, 0x3333, 200)].iter().enumerate()
        {
            let base = i * 12;
            let got_f = f32::from_ne_bytes(out[base..base + 4].try_into().unwrap());
            let got_u = u32::from_ne_bytes(out[base + 4..base + 8].try_into().unwrap());
            let got_ts = u32::from_be_bytes(out[base + 8..base + 12].try_into().unwrap());
            assert_eq!(got_f, *expected_f, "entry {i}: var1");
            assert_eq!(got_u, *expected_u, "entry {i}: var2");
            assert_eq!(got_ts, *expected_ts, "entry {i}: timestamp");
        }
    }

    #[test]
    fn rpv_callback_failure_zeros_entry() {
        let host = make_host(false); // callback always returns false
        let mut buf = [0xFFu8; 128]; // pre-fill with 0xFF so any non-zero residue is visible

        let mut cfg = Configuration::DEFAULT;
        cfg.items_count = 1;
        cfg.items_to_log[0] = LoggableItem::Rpv { id: TEST_RPV.id };

        let mut enc = unsafe { RawFormatEncoder::new(buf.as_mut_ptr(), buf.len()) };
        enc.reset(&cfg, &host);
        assert!(!enc.error());

        enc.encode_next_entry(&cfg, 0, &host);

        // The 4-byte RPV entry must be zeroed.
        assert_eq!(&buf[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn buffer_too_small_for_single_entry() {
        let host = make_host(true);
        let var1: u32 = 0x1234_5678;
        let var2: u32 = 0x9abc_def0;

        let mut cfg = Configuration::DEFAULT;
        cfg.items_count = 2;
        cfg.items_to_log[0] = LoggableItem::Memory {
            address: (&raw const var1).cast(),
            size: 4,
        };
        cfg.items_to_log[1] = LoggableItem::Memory {
            address: (&raw const var2).cast(),
            size: 4,
        };

        // Buffer holds only 4 bytes — not enough for one 8-byte entry.
        let mut small_buf = [0u8; 4];
        // Safety: small_buf outlives enc.
        let mut enc = unsafe { RawFormatEncoder::new(small_buf.as_mut_ptr(), small_buf.len()) };
        enc.reset(&cfg, &host);
        assert!(enc.error());

        // Keep addresses live so the compiler doesn't optimize them away.
        let _ = (&raw const var1, &raw const var2);
    }
}
