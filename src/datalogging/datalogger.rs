use crate::datalogging::raw_encoder::RawFormatEncoder;
use crate::datalogging::trigger::{
    active_condition_for, ActiveCondition, ConditionSharedData,
};
use crate::datalogging::types::{
    AnyValAndTypePair, Configuration, LoggableItem, Operand, SupportedTriggerConditions,
    MAX_OPERANDS,
};
use crate::timebase::Timebase;
use crate::tools::{get_var_type_type, TypeSize};
use crate::types::{
    AnyType, IntBiggest, RuntimePublishedValue, Timestamp, UintBiggest, VariableType,
    VariableTypeType, BIGGEST_SINT, BIGGEST_UINT,
};

// ── Host interface ────────────────────────────────────────────────────────────

/// Interface that the `DataLogger` needs from the host application.
///
/// Wire this up to `MainHandler` once the rest of the library is in place.
/// For tests, implement it directly with simple in-process stubs.
pub trait DataloggerHost {
    /// Copies `count` bytes from the arbitrary address `src` into `dst`.
    ///
    /// # Safety
    /// `src..src+count` must be a valid, readable memory region.
    unsafe fn read_memory(&self, dst: *mut u8, src: *const u8, count: usize) -> bool;

    /// Returns `true` if RPV reading is configured (array + read callback present).
    fn is_rpv_read_configured(&self) -> bool;

    /// Returns `true` if an RPV with the given `id` exists.
    fn rpv_exists(&self, id: u16) -> bool;

    /// Returns the RPV descriptor for `id`, or `None` if not found.
    fn get_rpv(&self, id: u16) -> Option<RuntimePublishedValue>;

    /// Calls the RPV read callback, filling `out` with the RPV's current value.
    fn read_rpv(&self, rpv: RuntimePublishedValue, out: &mut AnyType) -> bool;
}

// ── DataLogger state machine ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Configured,
    Armed,
    Triggered,
    AcquisitionCompleted,
    Error,
}

struct TriggerState {
    active_condition: ActiveCondition,
    condition_data: ConditionSharedData,
    rising_edge_timestamp: Timestamp,
    previous_val: bool,
}

/// Oscilloscope-style data logger.
///
/// Create one per time domain. Call `configure()` when a new acquisition
/// is requested, `arm_trigger()` to start watching for the trigger, and
/// `process()` on every loop tick.
pub struct DataLogger {
    config: Configuration,
    encoder: RawFormatEncoder,
    buffer_size: usize,
    trigger_callback: Option<fn()>,
    trigger_timestamp: Timestamp,
    trigger_cursor_location: usize,
    remaining_data_to_write: usize,
    log_points_after_trigger: usize,
    decimation_counter: u16,
    acquisition_id: u16,
    config_id: u16,
    manual_trigger: bool,
    config_valid: bool,
    state: State,
    trigger: TriggerState,
    // Scratch space for operand data during check_trigger (keeps it off the hot path stack).
    ops_data: [AnyValAndTypePair; MAX_OPERANDS],
}

impl DataLogger {
    /// Creates a new `DataLogger` backed by the given raw buffer.
    ///
    /// # Safety
    /// `buffer` must be valid for `buffer_len` bytes for the lifetime of this `DataLogger`.
    #[must_use]
    pub unsafe fn new(buffer: *mut u8, buffer_len: usize, trigger_callback: Option<fn()>) -> Self {
        let mut dl = Self {
            config: Configuration::DEFAULT,
            // Safety: forwarded from caller.
            encoder: unsafe { RawFormatEncoder::new(buffer, buffer_len) },
            buffer_size: buffer_len,
            trigger_callback,
            trigger_timestamp: 0,
            trigger_cursor_location: 0,
            remaining_data_to_write: 0,
            log_points_after_trigger: 0,
            decimation_counter: 0,
            acquisition_id: 0,
            config_id: 0,
            manual_trigger: false,
            config_valid: false,
            state: State::Idle,
            trigger: TriggerState {
                active_condition: active_condition_for(SupportedTriggerConditions::AlwaysTrue),
                condition_data: ConditionSharedData::DEFAULT,
                rising_edge_timestamp: 0,
                previous_val: false,
            },
            ops_data: [AnyValAndTypePair::DEFAULT; MAX_OPERANDS],
        };
        dl.reset();
        dl
    }

    /// Puts the datalogger back to its startup state.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.trigger.previous_val = false;
        self.trigger.rising_edge_timestamp = 0;
        self.trigger.active_condition = active_condition_for(SupportedTriggerConditions::AlwaysTrue);
        self.trigger.condition_data = ConditionSharedData::DEFAULT;
        self.trigger_cursor_location = 0;
        self.trigger_timestamp = 0;
        self.remaining_data_to_write = 0;
        self.config_valid = false;
        self.manual_trigger = false;
        self.decimation_counter = 0;
        self.log_points_after_trigger = 0;
    }

    /// Applies the current configuration (set via `config_mut()`) and transitions to `Configured`.
    /// Call once per acquisition request.
    pub fn configure(
        &mut self,
        timebase: &Timebase,
        config_id: u16,
        host: &dyn DataloggerHost,
    ) {
        self.reset();
        self.config_valid = true;
        self.config_id = config_id;

        // Validate items_count.
        if self.config.items_count == 0
            || self.config.items_count as usize > crate::build_config::DATALOGGING_MAX_SIGNAL
        {
            self.config_valid = false;
        }

        // Validate operand_count.
        if self.config.trigger.operand_count as usize > MAX_OPERANDS {
            self.config_valid = false;
        }

        self.trigger.active_condition = active_condition_for(self.config.trigger.condition);

        // Operand count must match what the condition expects.
        if self.config.trigger.operand_count != self.trigger.active_condition.operand_count {
            self.config_valid = false;
        }

        // Decimation must be non-zero.
        if self.config.decimation == 0 {
            self.config_valid = false;
        }

        // Validate individual operands.
        if self.config_valid {
            for i in 0..self.config.trigger.operand_count as usize {
                match self.config.trigger.operands[i] {
                    Operand::Literal(val) => {
                        if !val.is_finite() {
                            self.config_valid = false;
                        }
                    }
                    Operand::Rpv { id } => {
                        if !host.is_rpv_read_configured() || !host.rpv_exists(id) {
                            self.config_valid = false;
                        }
                    }
                    Operand::Var { datatype, .. } => {
                        if !is_supported_type(datatype) {
                            self.config_valid = false;
                        }
                    }
                    Operand::VarBit {
                        datatype,
                        bitoffset,
                        bitsize,
                        ..
                    } => {
                        #[allow(clippy::cast_possible_truncation)]
                        let max_bits =
                            (core::mem::size_of::<UintBiggest>() * 8) as u8;
                        if bitoffset >= max_bits || bitsize > max_bits || bitsize == 0 {
                            self.config_valid = false;
                        }
                        if !is_supported_type(datatype) {
                            self.config_valid = false;
                        }
                        if u16::from(bitoffset) + u16::from(bitsize)
                            > u16::from(datatype.type_size()) * 8
                        {
                            self.config_valid = false;
                        }
                    }
                }
                if !self.config_valid {
                    break;
                }
            }
        }

        // Validate loggable items.
        if self.config_valid {
            for i in 0..self.config.items_count as usize {
                match self.config.items_to_log[i] {
                    LoggableItem::Rpv { id } => {
                        if !host.is_rpv_read_configured() || !host.rpv_exists(id) {
                            self.config_valid = false;
                        }
                    }
                    LoggableItem::Memory { .. } | LoggableItem::Time => {}
                }
                if !self.config_valid {
                    break;
                }
            }
        }

        if self.config_valid {
            if let Some(reset_fn) = self.trigger.active_condition.reset_fn {
                reset_fn(&mut self.trigger.condition_data);
            }
            self.encoder.reset(&self.config, host);
            self.state = State::Configured;
        } else {
            self.state = State::Error;
        }

        // Suppress unused `timebase` warning — it may be used for future extensions
        // and mirrors the C++ API signature.
        let _ = timebase;
    }

    /// Arms the trigger so the datalogger actively watches for the trigger condition.
    pub const fn arm_trigger(&mut self) {
        if matches!(
            self.state,
            State::Configured | State::AcquisitionCompleted | State::Triggered
        ) {
            self.state = State::Armed;
        }
    }

    /// Disarms the trigger; the datalogger keeps logging pre-trigger data.
    pub const fn disarm_trigger(&mut self) {
        if matches!(
            self.state,
            State::Armed | State::AcquisitionCompleted | State::Triggered
        ) {
            self.state = State::Configured;
        }
    }

    /// Forces an immediate trigger (only while Armed).
    pub fn force_trigger(&mut self) {
        if self.state == State::Armed {
            self.manual_trigger = true;
        }
    }

    /// Must be called as fast as possible (every loop tick).
    pub fn process(&mut self, timebase: &Timebase, host: &dyn DataloggerHost) {
        match self.state {
            State::Idle | State::Error | State::AcquisitionCompleted => {}
            State::Configured | State::Armed | State::Triggered => {
                if self.encoder.error() {
                    self.state = State::Error;
                    return;
                }

                self.process_acquisition(timebase, host);

                if self.state == State::Armed && self.check_trigger(timebase, host) {
                    if let Some(cb) = self.trigger_callback {
                        cb();
                    }
                    self.stamp_trigger_point(timebase);
                    self.state = State::Triggered;
                }

                if self.state == State::Triggered && self.acquisition_completed(timebase) {
                    self.acquisition_id = self.acquisition_id.wrapping_add(1);
                    self.log_points_after_trigger = self.encoder.get_entry_write_counter();
                    self.state = State::AcquisitionCompleted;
                }
            }
        }
    }

    /// Checks whether the trigger condition is currently satisfied (hold time included).
    /// Only meaningful when armed.
    pub(crate) fn check_trigger(&mut self, timebase: &Timebase, host: &dyn DataloggerHost) -> bool {
        if self.state != State::Armed {
            return false;
        }

        let nb_operands = self.trigger.active_condition.operand_count as usize;
        if nb_operands > MAX_OPERANDS {
            return false;
        }

        if self.manual_trigger {
            self.manual_trigger = false;
            self.trigger.previous_val = true;
            return true;
        }

        for i in 0..nb_operands {
            if !fetch_operand(
                host,
                &self.config.trigger.operands[i],
                &mut self.ops_data[i],
            ) {
                return false;
            }
            convert_to_compare_type(&mut self.ops_data[i]);
        }

        let condition_result = (self.trigger.active_condition.eval_fn)(
            &mut self.trigger.condition_data,
            &self.ops_data[..nb_operands],
        );

        if condition_result {
            if !self.trigger.previous_val {
                self.trigger.rising_edge_timestamp = timebase.get_timestamp();
            }
            if timebase.has_expired(
                self.trigger.rising_edge_timestamp,
                self.config.trigger.hold_time_100ns,
            ) {
                self.trigger.previous_val = condition_result;
                return true;
            }
        }

        self.trigger.previous_val = condition_result;
        false
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    /// Total byte capacity of the acquisition buffer.
    #[must_use]
    #[inline]
    pub const fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    #[must_use]
    #[inline]
    pub fn data_acquired(&self) -> bool {
        self.state == State::AcquisitionCompleted
    }

    #[must_use]
    #[inline]
    pub const fn get_state(&self) -> State {
        self.state
    }

    #[must_use]
    #[inline]
    pub fn armed(&self) -> bool {
        self.state == State::Armed
    }

    #[must_use]
    #[inline]
    pub fn in_error(&self) -> bool {
        self.state == State::Error
    }

    #[must_use]
    #[inline]
    pub const fn config_valid(&self) -> bool {
        self.config_valid
    }

    #[must_use]
    #[inline]
    pub const fn get_acquisition_id(&self) -> u16 {
        self.acquisition_id
    }

    #[must_use]
    #[inline]
    pub const fn get_config_id(&self) -> u16 {
        self.config_id
    }

    /// Number of entries logged after the trigger point (available after `AcquisitionCompleted`).
    #[must_use]
    #[inline]
    pub const fn log_points_after_trigger(&self) -> usize {
        self.log_points_after_trigger
    }

    /// Bytes yet to be written after trigger before the acquisition completes.
    #[must_use]
    #[inline]
    pub fn get_bytes_to_acquire_from_trigger_to_completion(&self) -> usize {
        if self.state == State::Triggered {
            self.remaining_data_to_write
        } else {
            0
        }
    }

    /// Bytes written since the trigger event.
    #[must_use]
    #[inline]
    pub fn data_counter_since_trigger(&self) -> usize {
        if self.state == State::Triggered {
            self.encoder.get_data_write_counter()
        } else {
            0
        }
    }

    /// Mutable access to the acquisition configuration.
    #[inline]
    pub const fn config_mut(&mut self) -> &mut Configuration {
        &mut self.config
    }

    /// Immutable access to the acquisition configuration.
    #[must_use]
    #[inline]
    pub const fn config_ref(&self) -> &Configuration {
        &self.config
    }

    /// Mutable access to the encoder (which also embeds the reader state).
    #[inline]
    pub const fn get_encoder_mut(&mut self) -> &mut RawFormatEncoder {
        &mut self.encoder
    }

    /// Immutable access to the encoder.
    #[must_use]
    #[inline]
    pub const fn get_encoder(&self) -> &RawFormatEncoder {
        &self.encoder
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn process_acquisition(&mut self, timebase: &Timebase, host: &dyn DataloggerHost) {
        self.decimation_counter += 1;
        if self.decimation_counter >= self.config.decimation {
            self.encoder
                .encode_next_entry(&self.config, timebase.get_timestamp(), host);
            self.decimation_counter = 0;
        }
    }

    fn stamp_trigger_point(&mut self, timebase: &Timebase) {
        self.trigger_cursor_location = self.encoder.get_write_cursor();
        self.trigger_timestamp = timebase.get_timestamp();
        self.encoder.reset_write_counter();

        // How much data to record after the trigger, based on probe_location:
        //   probe_location=0   → write ≈ buffer_size after trigger (trigger near start)
        //   probe_location=255 → write ≈ 0 after trigger (trigger near end)
        let multiplier = u64::from(255u8 - self.config.probe_location);
        let remaining = ((self.buffer_size as u64 * multiplier) >> 8) as usize;

        self.remaining_data_to_write = if self.encoder.buffer_full() {
            remaining
        } else {
            remaining.max(self.encoder.remaining_bytes_to_full())
        };

        if self.remaining_data_to_write > self.buffer_size {
            self.remaining_data_to_write = self.buffer_size;
        }
    }

    fn acquisition_completed(&self, timebase: &Timebase) -> bool {
        if self.state == State::AcquisitionCompleted {
            return true;
        }

        if self.state == State::Triggered {
            if self.config.timeout_100ns > 0
                && timebase.has_expired(self.trigger_timestamp, self.config.timeout_100ns)
            {
                return true;
            }
            if self.encoder.get_data_write_counter() >= self.remaining_data_to_write {
                return true;
            }
        }

        false
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

/// Reads one operand and stores value + type in `out`.
/// Returns `false` on failure (bad operand type, RPV not found, etc.).
pub fn fetch_operand(  // pub: used by integration tests
    host: &dyn DataloggerHost,
    operand: &Operand,
    out: &mut AnyValAndTypePair,
) -> bool {
    match *operand {
        Operand::Literal(val) => {
            out.val = AnyType { float32: val };
            out.valtype = VariableType::Float32;
            true
        }
        Operand::Rpv { id } => {
            let Some(rpv) = host.get_rpv(id) else {
                out.val = AnyType { uint32: 0 };
                out.valtype = VariableType::Float32;
                return false;
            };
            let success = host.read_rpv(rpv, &mut out.val);
            out.valtype = rpv.var_type;
            if !success {
                out.val = AnyType { uint32: 0 };
                out.valtype = VariableType::Float32;
            }
            success
        }
        Operand::Var { addr, datatype } => {
            let success = fetch_variable(host, addr, datatype, &mut out.val);
            out.valtype = datatype;
            if !success {
                out.val = AnyType { uint32: 0 };
                out.valtype = VariableType::Float32;
            }
            success
        }
        Operand::VarBit {
            addr,
            datatype,
            bitoffset,
            bitsize,
        } => {
            let tt = get_var_type_type(datatype);
            let success = fetch_variable_bitfield(host, addr, tt, bitoffset, bitsize, out);
            if !success {
                out.val = AnyType { uint32: 0 };
                out.valtype = VariableType::Float32;
            }
            success
        }
    }
}

/// Reads a typed variable from `addr` using the host's `read_memory`.
pub(crate) fn fetch_variable(
    host: &dyn DataloggerHost,
    addr: *const u8,
    vartype: VariableType,
    out: &mut AnyType,
) -> bool {
    let size = vartype.type_size() as usize;
    if size == 0 || size > core::mem::size_of::<AnyType>() {
        return false;
    }
    // Safety: caller ensures addr is valid; we only write `size` bytes into `out`.
    unsafe { host.read_memory(core::ptr::from_mut::<AnyType>(out).cast::<u8>(), addr, size) }
}

/// Reads a bit-field from memory and stores the result in `out`.
/// `var_tt` determines whether the bitfield is interpreted as sint, uint, or bool.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn fetch_variable_bitfield(
    host: &dyn DataloggerHost,
    addr: *const u8,
    var_tt: VariableTypeType,
    bitoffset: u8,
    bitsize: u8,
    out: &mut AnyValAndTypePair,
) -> bool {
    if bitsize == 0 {
        return false;
    }
    if !matches!(
        var_tt,
        VariableTypeType::VTTsint | VariableTypeType::VTTuint | VariableTypeType::VTTboolean
    ) {
        return false;
    }

    // Determine the smallest integer type needed to read the full bitfield.
    let fetch_required_bytes = ((u16::from(bitoffset) + u16::from(bitsize) - 1) / 8 + 1) as u8;
    let fetch_size = required_type_size_bytes(fetch_required_bytes);
    let output_required_bytes = ((u16::from(bitsize) - 1) / 8 + 1) as u8;
    let output_size = required_type_size_bytes(output_required_bytes);

    // We always read as an unsigned integer then shift/mask.
    let fetch_type = crate::tools::make_type(VariableTypeType::VTTuint, fetch_size);
    let output_type = crate::tools::make_type(var_tt, output_size);

    if fetch_type == VariableType::Unknown || output_type == VariableType::Unknown {
        return false;
    }

    let mut raw = AnyType { uint32: 0 };
    if !fetch_variable(host, addr, fetch_type, &mut raw) {
        return false;
    }

    // Shift right by bitoffset, then mask to bitsize bits.
    // All operations done in the largest unsigned type.
    let shifted: UintBiggest = unsafe {
        match fetch_type {
            VariableType::Uint8 => u64::from(raw.uint8) >> bitoffset,
            VariableType::Uint16 => u64::from(raw.uint16) >> bitoffset,
            VariableType::Uint32 => u64::from(raw.uint32) >> bitoffset,
            #[cfg(feature = "support_64bits")]
            VariableType::Uint64 => raw.uint64 >> bitoffset,
            _ => return false,
        }
    } as UintBiggest;

    let mask: UintBiggest = if bitsize >= 64 {
        UintBiggest::MAX
    } else {
        (1u64 << bitsize) as UintBiggest - 1
    };
    let masked = shifted & mask;

    // Write the masked value into `out.val` with the correct output type, then sign-extend if sint.
    // Safety: we only write bytes appropriate for `output_type`.
    match output_type {
        VariableType::Uint8 | VariableType::Boolean8 => {
            out.val = AnyType { uint8: masked as u8 };
            out.valtype = output_type;
        }
        VariableType::Uint16 => {
            out.val = AnyType { uint16: masked as u16 };
            out.valtype = output_type;
        }
        VariableType::Uint32 => {
            out.val = AnyType { uint32: masked as u32 };
            out.valtype = output_type;
        }
        VariableType::Sint8 => {
            let v = sign_extend(u32::from(masked as u8), u32::from(bitsize)) as i8;
            out.val = AnyType { sint8: v };
            out.valtype = output_type;
        }
        VariableType::Sint16 => {
            let v = sign_extend(u32::from(masked as u16), u32::from(bitsize)) as i16;
            out.val = AnyType { sint16: v };
            out.valtype = output_type;
        }
        VariableType::Sint32 => {
            let v = sign_extend(masked as u32, u32::from(bitsize));
            out.val = AnyType { sint32: v };
            out.valtype = output_type;
        }
        #[cfg(feature = "support_64bits")]
        VariableType::Uint64 => {
            out.val = AnyType { uint64: masked };
            out.valtype = output_type;
        }
        #[cfg(feature = "support_64bits")]
        VariableType::Sint64 => {
            let v = sign_extend_64(masked, u32::from(bitsize));
            out.val = AnyType { sint64: v };
            out.valtype = output_type;
        }
        _ => return false,
    }
    true
}

/// Widens `val` in-place so that after the call `valtype` is one of
/// `Float32`, `BIGGEST_UINT`, or `BIGGEST_SINT` and the matching union field
/// holds a correctly zero/sign-extended value.
pub fn convert_to_compare_type(pair: &mut AnyValAndTypePair) {
    // Safety: each branch only reads the field that is valid for `valtype`.
    unsafe {
        match pair.valtype {
            #[cfg(feature = "support_64bits")]
            VariableType::Float64 => {
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                let f32val = pair.val.float64 as f32;
                pair.val.float32 = f32val;
                pair.valtype = VariableType::Float32;
            }
            VariableType::Boolean | VariableType::Boolean8 => {
                let b = UintBiggest::from(pair.val.boolean);
                crate::tools::set_biggest_uint(&mut pair.val, b);
                pair.valtype = BIGGEST_UINT;
            }
            VariableType::Sint8 => {
                let v = IntBiggest::from(pair.val.sint8);
                crate::tools::set_biggest_sint(&mut pair.val, v);
                pair.valtype = BIGGEST_SINT;
            }
            VariableType::Sint16 => {
                let v = IntBiggest::from(pair.val.sint16);
                crate::tools::set_biggest_sint(&mut pair.val, v);
                pair.valtype = BIGGEST_SINT;
            }
            VariableType::Sint32 => {
                let v = IntBiggest::from(pair.val.sint32);
                crate::tools::set_biggest_sint(&mut pair.val, v);
                pair.valtype = BIGGEST_SINT;
            }
            #[cfg(feature = "support_64bits")]
            VariableType::Sint64 => {
                // Already the biggest sint; just normalise the type tag.
                pair.valtype = BIGGEST_SINT;
            }
            VariableType::Uint8 => {
                let v = UintBiggest::from(pair.val.uint8);
                crate::tools::set_biggest_uint(&mut pair.val, v);
                pair.valtype = BIGGEST_UINT;
            }
            VariableType::Uint16 => {
                let v = UintBiggest::from(pair.val.uint16);
                crate::tools::set_biggest_uint(&mut pair.val, v);
                pair.valtype = BIGGEST_UINT;
            }
            VariableType::Uint32 => {
                let v = UintBiggest::from(pair.val.uint32);
                crate::tools::set_biggest_uint(&mut pair.val, v);
                pair.valtype = BIGGEST_UINT;
            }
            #[cfg(feature = "support_64bits")]
            VariableType::Uint64 => {
                pair.valtype = BIGGEST_UINT;
            }
            // Float32, Unknown, and other unsupported types: leave unchanged.
            _ => {}
        }
    }
}

// ── Private free helpers ──────────────────────────────────────────────────────

/// Returns `true` for types the datalogger can safely read and compare.
pub(crate) const fn is_supported_type(vt: VariableType) -> bool {
    matches!(
        vt,
        VariableType::Boolean
            | VariableType::Boolean8
            | VariableType::Sint8
            | VariableType::Sint16
            | VariableType::Sint32
            | VariableType::Uint8
            | VariableType::Uint16
            | VariableType::Uint32
            | VariableType::Float32
    ) || {
        cfg_if_64bits_supported(vt)
    }
}

#[inline]
const fn cfg_if_64bits_supported(vt: VariableType) -> bool {
    #[cfg(feature = "support_64bits")]
    {
        matches!(vt, VariableType::Sint64 | VariableType::Uint64 | VariableType::Float64)
    }
    #[cfg(not(feature = "support_64bits"))]
    {
        let _ = vt;
        false
    }
}

/// Returns the smallest `VariableTypeSize` variant whose byte count is ≥ `size_bytes`.
const fn required_type_size_bytes(size_bytes: u8) -> crate::types::VariableTypeSize {
    use crate::types::VariableTypeSize;
    match size_bytes {
        0 => VariableTypeSize::VTSundef,
        1 => VariableTypeSize::VTS8,
        2 => VariableTypeSize::VTS16,
        3..=4 => VariableTypeSize::VTS32,
        #[cfg(feature = "support_64bits")]
        5..=8 => VariableTypeSize::VTS64,
        _ => VariableTypeSize::VTSundef,
    }
}

/// Sign-extends a `bitsize`-bit two's-complement integer stored in a `u32`.
#[allow(clippy::cast_possible_wrap)]
const fn sign_extend(val: u32, bitsize: u32) -> i32 {
    if bitsize == 0 || bitsize >= 32 {
        return val as i32;
    }
    let shift = 32 - bitsize;
    ((val << shift) as i32) >> shift
}

#[cfg(feature = "support_64bits")]
#[allow(clippy::cast_possible_wrap)]
const fn sign_extend_64(val: u64, bitsize: u32) -> i64 {
    if bitsize == 0 || bitsize >= 64 {
        return val as i64;
    }
    let shift = 64 - bitsize;
    ((val << shift) as i64) >> shift
}
