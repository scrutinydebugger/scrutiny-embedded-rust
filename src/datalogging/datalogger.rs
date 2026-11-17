use crate::datalogging::raw_encoder::RawFormatEncoder;
use crate::datalogging::trigger::{
    active_condition_for, ActiveCondition, ConditionSharedData,
};
use crate::datalogging::types::{
    AnyValAndTypePair, Configuration, LoggableItem, Operand, SupportedTriggerConditions,
    MAX_OPERANDS,
};
use crate::timebase::Timebase;
use crate::tools::{get_var_type_type, is_supported_type, TypeSize};
use crate::types::{
    AnyType, IntBiggest, RuntimePublishedValue, Timestamp, UintBiggest, VariableType,
    VariableTypeType, BIGGEST_SINT, BIGGEST_UINT,
};

// ── Host interface ────────────────────────────────────────────────────────────

/// Interface that the `DataLogger` needs from the host application.
pub(crate) trait DataloggerHost {
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

    /// Identity of the caller (loop name) for callbacks that need to disambiguate contexts.
    /// Default is `None`, which represents a call originating from the main handler.
    fn caller_name(&self) -> Option<&'static str> { None }
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
    trigger_callback: Option<crate::config::DataloggingTriggerCallback>,
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
    pub unsafe fn new(
        buffer: *mut u8,
        buffer_len: usize,
        trigger_callback: Option<crate::config::DataloggingTriggerCallback>,
    ) -> Self {
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
    pub(crate) fn configure(
        &mut self,
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
    pub(crate) fn process(&mut self, timebase: &Timebase, host: &dyn DataloggerHost) {
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
                        cb(host.caller_name());
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
pub(crate) fn fetch_operand(
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

    // Mask computed in u64 so that bitsize == 32 cannot overflow when UintBiggest is u32.
    let mask_u64: u64 = if bitsize >= 64 {
        u64::MAX
    } else {
        (1u64 << bitsize) - 1
    };
    let mask = mask_u64 as UintBiggest;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datalogging::types::{
        AnyValAndTypePair, Configuration, LoggableItem, Operand, SupportedTriggerConditions,
        TriggerConfig, MAX_OPERANDS,
    };
    use crate::timebase::Timebase;
    use crate::types::{AnyType, RuntimePublishedValue, VariableType};

    static TEST_RPVS: [RuntimePublishedValue; 3] = [
        RuntimePublishedValue { id: 0x1234, var_type: VariableType::Uint32 },
        RuntimePublishedValue { id: 0x5678, var_type: VariableType::Float32 },
        RuntimePublishedValue { id: 0x1000, var_type: VariableType::Uint32 },
    ];

    struct TestHost {
        rpv_read_callback: fn(RuntimePublishedValue, &mut AnyType) -> bool,
    }

    impl DataloggerHost for TestHost {
        unsafe fn read_memory(&self, dst: *mut u8, src: *const u8, count: usize) -> bool {
            core::ptr::copy_nonoverlapping(src, dst, count);
            true
        }
        fn is_rpv_read_configured(&self) -> bool { true }
        fn rpv_exists(&self, id: u16) -> bool {
            TEST_RPVS.iter().any(|r| r.id == id)
        }
        fn get_rpv(&self, id: u16) -> Option<RuntimePublishedValue> {
            TEST_RPVS.iter().find(|r| r.id == id).copied()
        }
        fn read_rpv(&self, rpv: RuntimePublishedValue, out: &mut AnyType) -> bool {
            (self.rpv_read_callback)(rpv, out)
        }
    }

    static RPV_1000_VAL: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    static TRIGGER_CB_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

    fn rpv_read_callback(rpv: RuntimePublishedValue, out: &mut AnyType) -> bool {
        match (rpv.id, rpv.var_type) {
            (0x1234, VariableType::Uint32) => { out.uint32 = 0xaabb_ccdd; true }
            (0x5678, VariableType::Float32) => { out.float32 = 3.141_592_6_f32; true }
            (0x1000, VariableType::Uint32) => {
                out.uint32 = RPV_1000_VAL.load(core::sync::atomic::Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }

    fn trigger_callback(_caller: Option<&'static str>) {
        TRIGGER_CB_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }

    fn make_host() -> TestHost {
        TestHost { rpv_read_callback }
    }

    fn memory_operand_f32(addr: *const f32) -> Operand {
        Operand::Var { addr: addr.cast(), datatype: VariableType::Float32 }
    }

    fn literal_operand(val: f32) -> Operand {
        Operand::Literal(val)
    }

    fn make_trigger(cond: SupportedTriggerConditions, op0: Operand, op1: Operand) -> TriggerConfig {
        TriggerConfig {
            condition: cond,
            operand_count: 2,
            operands: [op0, op1, Operand::Literal(0.0)],
            hold_time_100ns: 0,
        }
    }

    fn read_f32_ne(data: &[u8]) -> f32 {
        f32::from_ne_bytes(data[..4].try_into().unwrap())
    }
    fn read_i32_ne(data: &[u8]) -> i32 {
        i32::from_ne_bytes(data[..4].try_into().unwrap())
    }
    fn read_u32_be(data: &[u8]) -> u32 {
        u32::from_be_bytes(data[..4].try_into().unwrap())
    }

    #[allow(unused_assignments)]
    #[test]
    fn trigger_basics() {
        let host = make_host();
        let tb = Timebase::new();
        let mut my_var: f32 = 0.0;
        let logged_var: f32 = 0.0;
        let mut dl_buf = [0u8; 128];

        // Safety: dl_buf outlives dl.
        let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
        {
            let cfg = dl.config_mut();
            *cfg = Configuration::DEFAULT;
            cfg.items_count = 1;
            cfg.items_to_log[0] = LoggableItem::Memory {
                address: (&raw const logged_var).cast(),
                size: core::mem::size_of_val(&logged_var) as u8,
            };
            cfg.decimation = 1;
            cfg.trigger = make_trigger(
                SupportedTriggerConditions::Equal,
                memory_operand_f32(&raw const my_var),
                literal_operand(3.141_592_6_f32),
            );
            cfg.probe_location = 128;
        }
        dl.configure(0, &host);

        // Not armed: condition met but process() must not trigger.
        my_var = 3.141_592_6_f32;
        dl.process(&tb, &host);
        assert_eq!(dl.get_state(), State::Configured);

        dl.arm_trigger();
        my_var = 0.0;
        dl.process(&tb, &host);
        assert_eq!(dl.get_state(), State::Armed); // condition not met

        my_var = 3.141_592_6_f32;
        dl.process(&tb, &host);
        assert_eq!(dl.get_state(), State::Triggered); // hold_time=0 fires immediately
    }

    #[allow(unused_assignments)]
    #[test]
    fn trigger_hold_time() {
        let host = make_host();
        let mut tb = Timebase::new();
        let mut my_var: f32 = 0.0;
        let logged_var: f32 = 0.0;
        let mut dl_buf = [0u8; 128];

        // Safety: dl_buf outlives dl.
        let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
        {
            let cfg = dl.config_mut();
            *cfg = Configuration::DEFAULT;
            cfg.items_count = 1;
            cfg.items_to_log[0] = LoggableItem::Memory {
                address: (&raw const logged_var).cast(),
                size: core::mem::size_of_val(&logged_var) as u8,
            };
            cfg.decimation = 1;
            cfg.trigger = TriggerConfig {
                condition: SupportedTriggerConditions::Equal,
                operand_count: 2,
                operands: [
                    memory_operand_f32(&raw const my_var),
                    literal_operand(3.141_592_6_f32),
                    Operand::Literal(0.0),
                ],
                hold_time_100ns: 100,
            };
            cfg.probe_location = 128;
        }
        dl.configure(0, &host);
        dl.arm_trigger();

        // Condition false: no trigger.
        dl.process(&tb, &host);
        assert_eq!(dl.get_state(), State::Armed);

        // Condition becomes true: hold timer starts at timestamp=0.
        my_var = 3.141_592_6_f32;
        dl.process(&tb, &host);
        assert_eq!(dl.get_state(), State::Armed); // hold not elapsed yet

        tb.step(99);
        dl.process(&tb, &host);
        assert_eq!(dl.get_state(), State::Armed); // 99 ticks < 100

        tb.step(1);
        dl.process(&tb, &host);
        assert_eq!(dl.get_state(), State::Triggered); // 100 ticks >= hold_time
    }

    #[test]
    fn configure_rejects_varbit_with_bitsize_zero() {
        let host = make_host();
        let dummy: u32 = 0;
        let mut dl_buf = [0u8; 128];

        // Safety: dl_buf outlives dl.
        let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
        {
            let cfg = dl.config_mut();
            *cfg = Configuration::DEFAULT;
            cfg.items_count = 1;
            cfg.items_to_log[0] = LoggableItem::Memory {
                address: (&raw const dummy).cast(),
                size: 4,
            };
            cfg.decimation = 1;
            cfg.trigger = TriggerConfig {
                condition: SupportedTriggerConditions::GreaterThan,
                operand_count: 2,
                operands: [
                    Operand::VarBit {
                        addr: (&raw const dummy).cast(),
                        datatype: VariableType::Uint32,
                        bitoffset: 0,
                        bitsize: 0,
                    },
                    Operand::Literal(0.0),
                    Operand::Literal(0.0),
                ],
                hold_time_100ns: 0,
            };
        }
        dl.configure(0, &host);
        assert_eq!(dl.get_state(), State::Error);
        assert!(!dl.config_valid());
    }

    #[allow(unused_assignments)]
    #[test]
    fn basic_acquisition() {
        let host = make_host();
        let mut tb = Timebase::new();
        let mut my_var: f32 = 0.0;
        let mut dl_buf = [0u8; 128];

        TRIGGER_CB_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);

        // Safety: dl_buf outlives dl.
        let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), Some(trigger_callback)) };
        {
            let cfg = dl.config_mut();
            *cfg = Configuration::DEFAULT;
            cfg.items_count = 1;
            cfg.items_to_log[0] = LoggableItem::Memory {
                address: (&raw const my_var).cast(),
                size: core::mem::size_of_val(&my_var) as u8,
            };
            cfg.decimation = 2;
            cfg.trigger = TriggerConfig {
                condition: SupportedTriggerConditions::GreaterThan,
                operand_count: 2,
                operands: [
                    memory_operand_f32(&raw const my_var),
                    literal_operand(100.0_f32),
                    Operand::Literal(0.0),
                ],
                hold_time_100ns: 100,
            };
            cfg.probe_location = 128;
        }
        dl.configure(0, &host);

        // Not armed — data is NOT acquired even after many ticks.
        dl.process(&tb, &host);
        tb.step(100);
        dl.process(&tb, &host);
        tb.step(100);
        assert!(!dl.data_acquired());

        my_var = 200.0_f32;
        for _ in 0..100 {
            dl.process(&tb, &host);
            tb.step(100);
            my_var += 1.0;
        }
        assert!(!dl.data_acquired());
        assert_eq!(TRIGGER_CB_COUNT.load(core::sync::atomic::Ordering::Relaxed), 0);

        dl.arm_trigger();
        for _ in 0..100 {
            dl.process(&tb, &host);
            tb.step(100);
            my_var += 1.0;
        }
        assert!(dl.data_acquired());
        assert_eq!(TRIGGER_CB_COUNT.load(core::sync::atomic::Ordering::Relaxed), 1);
    }

    #[allow(unused_assignments)]
    #[test]
    fn always_use_full_buffer() {
        let host = make_host();
        let mut tb = Timebase::new();
        let mut var1: f32 = 0.0;
        let mut var2: i32 = 0;
        let mut dl_buf = [0u8; 128];

        let buf_len = dl_buf.len();

        for probe_loop in 0u16..=255 {
            var1 = 0.0;
            var2 = 0;
            let probe_location = probe_loop as u8;

            // Safety: dl_buf outlives dl.
            let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
            {
                let cfg = dl.config_mut();
                *cfg = Configuration::DEFAULT;
                cfg.items_count = 4;
                cfg.items_to_log[0] = LoggableItem::Memory {
                    address: (&raw const var1).cast(),
                    size: 4,
                };
                cfg.items_to_log[1] = LoggableItem::Memory {
                    address: (&raw const var2).cast(),
                    size: 4,
                };
                cfg.items_to_log[2] = LoggableItem::Rpv { id: 0x1000 };
                cfg.items_to_log[3] = LoggableItem::Time;
                cfg.decimation = 1;
                cfg.probe_location = probe_location;
                cfg.trigger = TriggerConfig {
                    condition: SupportedTriggerConditions::AlwaysTrue,
                    operand_count: 0,
                    operands: [Operand::Literal(0.0); MAX_OPERANDS],
                    hold_time_100ns: 0,
                };
            }
            dl.configure(0, &host);
            dl.arm_trigger();

            let max_loops = buf_len * 8;
            for _ in 0..max_loops {
                dl.process(&tb, &host);
                tb.step(10);
                if dl.data_acquired() {
                    break;
                }
            }
            assert!(
                dl.data_acquired(),
                "probe_location={probe_location}: acquisition did not complete"
            );
            assert!(
                !dl.get_encoder().error(),
                "probe_location={probe_location}: encoder error"
            );

            let total = dl.get_encoder().reader_total_size();
            assert!(
                total * 10 >= buf_len * 9,
                "probe_location={probe_location}: only {total} / {buf_len} bytes used"
            );
        }
    }

    #[test]
    fn acquire_time_correctly() {
        let host = make_host();
        let mut tb = Timebase::new();
        let mut dl_buf = [0u8; 128];
        let mut out_buf = [0u8; 256];
        let max_loops = dl_buf.len() * 8;

        // Safety: dl_buf outlives dl.
        let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
        {
            let cfg = dl.config_mut();
            *cfg = Configuration::DEFAULT;
            cfg.items_count = 1;
            cfg.items_to_log[0] = LoggableItem::Time;
            cfg.decimation = 1;
            cfg.probe_location = 128;
            cfg.trigger = TriggerConfig {
                condition: SupportedTriggerConditions::AlwaysTrue,
                operand_count: 0,
                operands: [Operand::Literal(0.0); MAX_OPERANDS],
                hold_time_100ns: 0,
            };
        }
        dl.configure(0, &host);

        // Fill buffer first.
        let mut i = 0;
        while !dl.get_encoder().buffer_full() && i < max_loops {
            dl.process(&tb, &host);
            tb.step(5);
            i += 1;
        }
        dl.arm_trigger();
        dl.force_trigger();
        i = 0;
        while dl.get_state() != State::AcquisitionCompleted && i < max_loops {
            dl.process(&tb, &host);
            tb.step(5);
            i += 1;
        }
        dl.process(&tb, &host);
        assert_eq!(dl.get_state(), State::AcquisitionCompleted);

        let enc = dl.get_encoder_mut();
        enc.reset_reader();
        let mut copied = 0usize;
        while !enc.reader_finished() {
            copied += enc.reader_read_chunk(&mut out_buf[copied..]);
        }

        let entry_count = enc.get_entry_count();
        let entry_size = enc.get_entry_size();
        assert_eq!(entry_size, 4); // timestamp is 4 bytes

        for idx in 1..entry_count {
            let ts_cur = u32::from_be_bytes(out_buf[idx * 4..idx * 4 + 4].try_into().unwrap());
            let ts_prev =
                u32::from_be_bytes(out_buf[(idx - 1) * 4..(idx - 1) * 4 + 4].try_into().unwrap());
            assert_eq!(
                ts_cur.wrapping_sub(ts_prev),
                5,
                "entry #{idx}: expected Δtime=5, got {}",
                ts_cur.wrapping_sub(ts_prev)
            );
        }
    }

    #[allow(unused_assignments)]
    #[test]
    fn complex_acquisition() {
        let host = make_host();
        let mut tb = Timebase::new();
        let mut var1: f32 = 0.0;
        let mut var2: i32 = 0;
        let mut trigger_val: f32 = 0.0_f32;
        let mut dl_buf = [0u8; 128];
        let mut out_buf = [0u8; 132]; // slightly larger than dl_buf

        let buf_len = dl_buf.len();

        for probe_loop in 0u16..=255 {
            var1 = 0.0;
            var2 = 0;
            trigger_val = 0.0_f32;
            RPV_1000_VAL.store(0, core::sync::atomic::Ordering::Relaxed);
            let probe_location = probe_loop as u8;

            // Safety: dl_buf outlives dl.
            let mut dl = unsafe { DataLogger::new(dl_buf.as_mut_ptr(), dl_buf.len(), None) };
            {
                let cfg = dl.config_mut();
                *cfg = Configuration::DEFAULT;
                cfg.items_count = 4;
                cfg.items_to_log[0] = LoggableItem::Memory {
                    address: (&raw const var1).cast(),
                    size: 4,
                };
                cfg.items_to_log[1] = LoggableItem::Memory {
                    address: (&raw const var2).cast(),
                    size: 4,
                };
                cfg.items_to_log[2] = LoggableItem::Rpv { id: 0x1000 };
                cfg.items_to_log[3] = LoggableItem::Time;
                cfg.decimation = 2;
                cfg.probe_location = probe_location;
                cfg.trigger = TriggerConfig {
                    condition: SupportedTriggerConditions::Equal,
                    operand_count: 2,
                    operands: [
                        Operand::Var {
                            addr: (&raw const trigger_val).cast(),
                            datatype: VariableType::Float32,
                        },
                        Operand::Literal(100.0_f32),
                        Operand::Literal(0.0),
                    ],
                    hold_time_100ns: 50,
                };
            }
            dl.configure(probe_loop, &host);

            // Pre-arm pre-fill (100 steps of 10 ticks each).
            for _ in 0..100 {
                dl.process(&tb, &host);
                tb.step(10);
            }
            assert!(!dl.data_acquired());

            dl.arm_trigger();

            // 100 ticks pre-trigger, then set trigger_val.
            for _ in 0..100 {
                dl.process(&tb, &host);
                tb.step(10);
                var1 += 1.0_f32;
                var2 -= 1;
                RPV_1000_VAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }

            trigger_val = 100.0_f32;

            let mut var1_at_trigger = 0.0_f32;
            let mut var2_at_trigger = 0_i32;
            let mut rpv_at_trigger = 0_u32;

            for i in 0..500usize {
                dl.process(&tb, &host);
                if i == 5 {
                    var1_at_trigger = var1;
                    var2_at_trigger = var2;
                    rpv_at_trigger = RPV_1000_VAL.load(core::sync::atomic::Ordering::Relaxed);
                }
                tb.step(10);
                var1 += 1.0_f32;
                var2 -= 1;
                RPV_1000_VAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }

            assert!(
                dl.data_acquired(),
                "probe_location={probe_location}: acquisition did not complete"
            );
            assert!(
                !dl.get_encoder().error(),
                "probe_location={probe_location}: encoder error"
            );

            // Read out the acquisition data.
            let enc = dl.get_encoder_mut();
            enc.reset_reader();
            let mut copied = 0usize;
            while !enc.reader_finished() {
                assert!(!enc.reader_error());
                let n = enc.reader_read_chunk(&mut out_buf[copied..]);
                copied += n;
                if n == 0 { break; }
            }

            assert!(
                copied * 10 >= buf_len * 9,
                "probe_location={probe_location}: only {copied}/{buf_len} bytes read"
            );

            let entry_count = enc.get_entry_count();
            let entry_size = enc.get_entry_size(); // should be 4+4+4+4=16
            assert_eq!(entry_size, 16);

            // Verify that consecutive entries have the expected deltas.
            for idx in 1..entry_count {
                let base_cur = idx * entry_size;
                let base_prev = (idx - 1) * entry_size;

                let v1_cur = read_f32_ne(&out_buf[base_cur..]);
                let v1_prev = read_f32_ne(&out_buf[base_prev..]);

                let v2_cur = read_i32_ne(&out_buf[base_cur + 4..]);
                let v2_prev = read_i32_ne(&out_buf[base_prev + 4..]);

                let rpv_cur = read_u32_be(&out_buf[base_cur + 8..]);
                let rpv_prev = read_u32_be(&out_buf[base_prev + 8..]);

                let t_cur = read_u32_be(&out_buf[base_cur + 12..]);
                let t_prev = read_u32_be(&out_buf[base_prev + 12..]);

                assert_eq!(
                    v1_cur - v1_prev,
                    2.0_f32,
                    "probe_location={probe_location} entry {idx}: var1 delta"
                );
                assert_eq!(
                    v2_cur - v2_prev,
                    -2,
                    "probe_location={probe_location} entry {idx}: var2 delta"
                );
                assert_eq!(
                    rpv_cur.wrapping_sub(rpv_prev),
                    2,
                    "probe_location={probe_location} entry {idx}: rpv delta"
                );
                assert_eq!(
                    t_cur.wrapping_sub(t_prev),
                    20,
                    "probe_location={probe_location} entry {idx}: time delta"
                );
            }

            // Verify trigger position.
            let lp_after = dl.log_points_after_trigger();
            let trigger_location = entry_count.saturating_sub(lp_after);
            let expected_trigger_loc =
                ((probe_location as f64 / 255.0) * (entry_count as f64 - 1.0)).round() as usize;

            let diff = if trigger_location >= expected_trigger_loc {
                trigger_location - expected_trigger_loc
            } else {
                expected_trigger_loc - trigger_location
            };
            assert!(
                diff <= 2,
                "probe_location={probe_location}: trigger at entry {trigger_location}, expected ≈{expected_trigger_loc} (diff={diff})"
            );

            // Verify values near the trigger point.
            if trigger_location < entry_count {
                let mid_base = trigger_location * entry_size;
                let mid_v1 = read_f32_ne(&out_buf[mid_base..]);
                let mid_v2 = read_i32_ne(&out_buf[mid_base + 4..]);
                let mid_rpv = read_u32_be(&out_buf[mid_base + 8..]);

                assert!(
                    (var1_at_trigger - mid_v1).abs() <= 10.0,
                    "probe_location={probe_location}: var1 at trigger mismatch ({var1_at_trigger} vs {mid_v1})"
                );
                assert!(
                    (var2_at_trigger - mid_v2).abs() <= 10,
                    "probe_location={probe_location}: var2 at trigger mismatch ({var2_at_trigger} vs {mid_v2})"
                );
                assert!(
                    rpv_at_trigger.abs_diff(mid_rpv) <= 10,
                    "probe_location={probe_location}: rpv at trigger mismatch ({rpv_at_trigger} vs {mid_rpv})"
                );
            }
        }
    }

    // ── fetch_operand tests (from datalogging_fetch_operands.rs) ─────────────

    static FETCH_TEST_RPVS: [RuntimePublishedValue; 2] = [
        RuntimePublishedValue { id: 0x1234, var_type: VariableType::Uint32 },
        RuntimePublishedValue { id: 0x5678, var_type: VariableType::Float32 },
    ];

    struct FetchTestHost;

    impl DataloggerHost for FetchTestHost {
        unsafe fn read_memory(&self, dst: *mut u8, src: *const u8, count: usize) -> bool {
            core::ptr::copy_nonoverlapping(src, dst, count);
            true
        }
        fn is_rpv_read_configured(&self) -> bool { true }
        fn rpv_exists(&self, id: u16) -> bool {
            FETCH_TEST_RPVS.iter().any(|r| r.id == id)
        }
        fn get_rpv(&self, id: u16) -> Option<RuntimePublishedValue> {
            FETCH_TEST_RPVS.iter().find(|r| r.id == id).copied()
        }
        fn read_rpv(&self, rpv: RuntimePublishedValue, out: &mut AnyType) -> bool {
            match (rpv.id, rpv.var_type) {
                (0x1234, VariableType::Uint32) => { out.uint32 = 0xaabb_ccdd; true }
                (0x5678, VariableType::Float32) => { out.float32 = 3.141_592_6_f32; true }
                _ => false,
            }
        }
    }

    #[test]
    fn fetch_literal() {
        let host = FetchTestHost;
        let op = Operand::Literal(0.1234_f32);
        let mut out = AnyValAndTypePair::DEFAULT;
        assert!(fetch_operand(&host, &op, &mut out));
        assert_eq!(out.valtype, VariableType::Float32);
        assert_eq!(unsafe { out.val.float32 }, 0.1234_f32);
    }

    #[test]
    fn fetch_var() {
        let host = FetchTestHost;
        let my_var: f32 = 3.141_592_6_f32;
        let op = Operand::Var { addr: (&raw const my_var).cast(), datatype: VariableType::Float32 };
        let mut out = AnyValAndTypePair::DEFAULT;
        assert!(fetch_operand(&host, &op, &mut out));
        assert_eq!(out.valtype, VariableType::Float32);
        assert_eq!(unsafe { out.val.float32 }, my_var);
    }

    #[test]
    fn fetch_rpv() {
        let host = FetchTestHost;
        let mut out = AnyValAndTypePair::DEFAULT;

        let op = Operand::Rpv { id: 0x1234 };
        assert!(fetch_operand(&host, &op, &mut out));
        assert_eq!(out.valtype, VariableType::Uint32);
        assert_eq!(unsafe { out.val.uint32 }, 0xaabb_ccdd);

        let op = Operand::Rpv { id: 0x5678 };
        assert!(fetch_operand(&host, &op, &mut out));
        assert_eq!(out.valtype, VariableType::Float32);
        assert_eq!(unsafe { out.val.float32 }, 3.141_592_6_f32);
    }

    #[test]
    fn fetch_inexistant_rpv() {
        let host = FetchTestHost;
        let op = Operand::Rpv { id: 0xAAAA };
        let mut out = AnyValAndTypePair::DEFAULT;
        assert!(!fetch_operand(&host, &op, &mut out));
    }

    #[test]
    fn fetch_varbit() {
        let host = FetchTestHost;
        let raw_val: i16 = -1000i16;
        let packed: u16 = ((raw_val as u16) << 3) & 0xFFF8;
        let packed_bytes = packed.to_le_bytes();

        let op = Operand::VarBit {
            addr: packed_bytes.as_ptr(),
            datatype: VariableType::Sint16,
            bitoffset: 3,
            bitsize: 11,
        };
        let mut out = AnyValAndTypePair::DEFAULT;
        assert!(fetch_operand(&host, &op, &mut out));
        assert_eq!(out.valtype, VariableType::Sint16);
        assert_eq!(unsafe { out.val.sint16 }, raw_val);
    }

    #[test]
    fn fetch_var_boolean_behaves_as_boolean8() {
        let host = FetchTestHost;
        let v: u8 = 1;
        let op_b = Operand::Var { addr: &raw const v, datatype: VariableType::Boolean };
        let op_b8 = Operand::Var { addr: &raw const v, datatype: VariableType::Boolean8 };

        let mut out_b = AnyValAndTypePair::DEFAULT;
        let mut out_b8 = AnyValAndTypePair::DEFAULT;
        assert!(fetch_operand(&host, &op_b, &mut out_b));
        assert!(fetch_operand(&host, &op_b8, &mut out_b8));

        convert_to_compare_type(&mut out_b);
        convert_to_compare_type(&mut out_b8);
        assert_eq!(out_b.valtype, out_b8.valtype);
        assert_eq!(
            crate::tools::read_biggest_uint(&out_b.val),
            crate::tools::read_biggest_uint(&out_b8.val)
        );
    }

    #[test]
    fn fetch_varbit_boolean_datatype() {
        let host = FetchTestHost;
        let v: u8 = 0b0000_0010;
        let op = Operand::VarBit {
            addr: &raw const v,
            datatype: VariableType::Boolean,
            bitoffset: 1,
            bitsize: 1,
        };
        let mut out = AnyValAndTypePair::DEFAULT;
        assert!(fetch_operand(&host, &op, &mut out));
        assert_eq!(out.valtype, VariableType::Boolean8);
        assert_eq!(unsafe { out.val.uint8 }, 1);
    }

    #[test]
    fn convert_to_compare_type_widens() {
        use crate::types::{IntBiggest, UintBiggest, BIGGEST_SINT, BIGGEST_UINT};

        let mut pair = AnyValAndTypePair::DEFAULT;

        // Sint8 → sign-extend to BIGGEST_SINT
        pair.valtype = VariableType::Sint8;
        pair.val = AnyType { sint8: -50 };
        convert_to_compare_type(&mut pair);
        assert_eq!(pair.valtype, BIGGEST_SINT);
        assert_eq!(crate::tools::read_biggest_sint(&pair.val), -50 as IntBiggest);

        // Sint16 → sign-extend
        pair.valtype = VariableType::Sint16;
        pair.val = AnyType { sint16: -1000 };
        convert_to_compare_type(&mut pair);
        assert_eq!(pair.valtype, BIGGEST_SINT);
        assert_eq!(crate::tools::read_biggest_sint(&pair.val), -1000 as IntBiggest);

        // Sint32 → sign-extend
        pair.valtype = VariableType::Sint32;
        pair.val = AnyType { sint32: -100_000 };
        convert_to_compare_type(&mut pair);
        assert_eq!(pair.valtype, BIGGEST_SINT);
        assert_eq!(crate::tools::read_biggest_sint(&pair.val), -100_000 as IntBiggest);

        // Uint8 → zero-extend to BIGGEST_UINT
        pair.valtype = VariableType::Uint8;
        pair.val = AnyType { uint8: 50 };
        convert_to_compare_type(&mut pair);
        assert_eq!(pair.valtype, BIGGEST_UINT);
        assert_eq!(crate::tools::read_biggest_uint(&pair.val), 50 as UintBiggest);

        // Uint16 → zero-extend
        pair.valtype = VariableType::Uint16;
        pair.val = AnyType { uint16: 1000 };
        convert_to_compare_type(&mut pair);
        assert_eq!(pair.valtype, BIGGEST_UINT);
        assert_eq!(crate::tools::read_biggest_uint(&pair.val), 1000 as UintBiggest);

        // Uint32 → zero-extend
        pair.valtype = VariableType::Uint32;
        pair.val = AnyType { uint32: 100_000 };
        convert_to_compare_type(&mut pair);
        assert_eq!(pair.valtype, BIGGEST_UINT);
        assert_eq!(crate::tools::read_biggest_uint(&pair.val), 100_000 as UintBiggest);

        // Float32 → unchanged
        pair.valtype = VariableType::Float32;
        pair.val = AnyType { float32: 3.141_592_6_f32 };
        convert_to_compare_type(&mut pair);
        assert_eq!(pair.valtype, VariableType::Float32);
        assert_eq!(unsafe { pair.val.float32 }, 3.141_592_6_f32);

        #[cfg(feature = "support_64bits")]
        {
            pair.valtype = VariableType::Uint64;
            pair.val = AnyType { uint64: 0x0001_2345_6789_abcd };
            convert_to_compare_type(&mut pair);
            assert_eq!(pair.valtype, BIGGEST_UINT);
            assert_eq!(crate::tools::read_biggest_uint(&pair.val), 0x0001_2345_6789_abcd_u64);

            pair.valtype = VariableType::Sint64;
            pair.val = AnyType { sint64: -0x0001_2345_6789_abcd };
            convert_to_compare_type(&mut pair);
            assert_eq!(pair.valtype, BIGGEST_SINT);
            assert_eq!(crate::tools::read_biggest_sint(&pair.val), -0x0001_2345_6789_abcd_i64);

            // Float64 → truncated to Float32
            pair.valtype = VariableType::Float64;
            pair.val = AnyType { float64: f64::from(3.141_592_6_f32) };
            convert_to_compare_type(&mut pair);
            assert_eq!(pair.valtype, VariableType::Float32);
            assert_eq!(unsafe { pair.val.float32 }, 3.141_592_6_f32);
        }
    }

    #[test]
    fn fetch_bitfield_limits() {
        let host = FetchTestHost;
        let store = [0i32; 4];
        let addr: *const u8 = store.as_ptr().cast();
        let mut out = AnyValAndTypePair::DEFAULT;

        let make = |bitoffset: u8, bitsize: u8| Operand::VarBit {
            addr,
            datatype: VariableType::Sint16,
            bitoffset,
            bitsize,
        };

        // bitoffset=31, bitsize=1 → reads a single bit from a 32-bit word: valid
        assert!(fetch_operand(&host, &make(31, 1), &mut out));

        // bitoffset=32, bitsize=1 → needs a 64-bit read
        {
            let result = fetch_operand(&host, &make(32, 1), &mut out);
            #[cfg(feature = "support_64bits")]
            assert!(result);
            #[cfg(not(feature = "support_64bits"))]
            assert!(!result);
        }

        // bitoffset=0, bitsize=32 → valid 32-bit field
        assert!(fetch_operand(&host, &make(0, 32), &mut out));

        // bitoffset=0, bitsize=33 → needs 64 bits
        {
            let result = fetch_operand(&host, &make(0, 33), &mut out);
            #[cfg(feature = "support_64bits")]
            assert!(result);
            #[cfg(not(feature = "support_64bits"))]
            assert!(!result);
        }

        // bitoffset=63, bitsize=1 → needs 64-bit read
        {
            let result = fetch_operand(&host, &make(63, 1), &mut out);
            #[cfg(feature = "support_64bits")]
            assert!(result);
            #[cfg(not(feature = "support_64bits"))]
            assert!(!result);
        }

        // bitoffset=64 → always invalid (exceeds UintBiggest width)
        assert!(!fetch_operand(&host, &make(64, 1), &mut out));

        // bitoffset=0, bitsize=64 → needs 64-bit read
        {
            let result = fetch_operand(&host, &make(0, 64), &mut out);
            #[cfg(feature = "support_64bits")]
            assert!(result);
            #[cfg(not(feature = "support_64bits"))]
            assert!(!result);
        }

        // bitoffset=0, bitsize=65 → always invalid
        assert!(!fetch_operand(&host, &make(0, 65), &mut out));

        // bitsize=0 → always invalid
        assert!(!fetch_operand(&host, &make(0, 0), &mut out));
    }

    // Mirrors C++ TestVariableFetching.RandomFetch: values packed at unaligned offsets.
    #[test]
    fn random_fetch() {
        let host = FetchTestHost;
        let mut some_buffer = [0u8; 32];

        let f32_val: f32 = 1.2345;
        let u16_val: u16 = 0x1234;
        let b_val = true;

        let f32_offset = 0;
        let f64_offset = f32_offset + 4;
        let u16_offset = f64_offset + 8;
        let bool_offset = u16_offset + 2;

        some_buffer[f32_offset..f32_offset + 4].copy_from_slice(&f32_val.to_ne_bytes());
        some_buffer[u16_offset..u16_offset + 2].copy_from_slice(&u16_val.to_ne_bytes());
        some_buffer[bool_offset] = u8::from(b_val);

        let mut out = AnyType { uint32: 0 };

        assert!(fetch_variable(&host, &raw const some_buffer[f32_offset], VariableType::Float32, &mut out));
        assert_eq!(unsafe { out.float32 }, f32_val);

        #[cfg(feature = "support_64bits")]
        {
            let f64_val: f64 = 3.141_592_6;
            some_buffer[f64_offset..f64_offset + 8].copy_from_slice(&f64_val.to_ne_bytes());
            assert!(fetch_variable(&host, &raw const some_buffer[f64_offset], VariableType::Float64, &mut out));
            assert_eq!(unsafe { out.float64 }, f64_val);
        }

        assert!(fetch_variable(&host, &raw const some_buffer[u16_offset], VariableType::Uint16, &mut out));
        assert_eq!(unsafe { out.uint16 }, u16_val);

        assert!(fetch_variable(&host, &raw const some_buffer[bool_offset], VariableType::Boolean, &mut out));
        assert_eq!(unsafe { out.boolean }, b_val);
    }

    fn xorshift32(state: &mut u32) -> u32 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *state = x;
        x
    }

    // Fetch every supported type at pseudo-random offsets (fixed seed) and compare with what was written.
    #[test]
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    fn random_fetch_all_types_random_offsets() {
        let host = FetchTestHost;
        let mut rng: u32 = 0x1234_5678;
        let mut buffer = [0u8; 64];
        let mut out = AnyType { uint32: 0 };

        for _ in 0..1000 {
            let offset = (xorshift32(&mut rng) % 48) as usize;
            let r = xorshift32(&mut rng);
            let dst = &mut buffer[offset..];
            let addr: *const u8 = dst.as_ptr();

            dst[..1].copy_from_slice(&(r as u8).to_ne_bytes());
            assert!(fetch_variable(&host, addr, VariableType::Uint8, &mut out));
            assert_eq!(unsafe { out.uint8 }, r as u8);

            dst[..1].copy_from_slice(&(r as i8).to_ne_bytes());
            assert!(fetch_variable(&host, addr, VariableType::Sint8, &mut out));
            assert_eq!(unsafe { out.sint8 }, r as i8);

            dst[..2].copy_from_slice(&(r as u16).to_ne_bytes());
            assert!(fetch_variable(&host, addr, VariableType::Uint16, &mut out));
            assert_eq!(unsafe { out.uint16 }, r as u16);

            dst[..2].copy_from_slice(&(r as i16).to_ne_bytes());
            assert!(fetch_variable(&host, addr, VariableType::Sint16, &mut out));
            assert_eq!(unsafe { out.sint16 }, r as i16);

            dst[..4].copy_from_slice(&r.to_ne_bytes());
            assert!(fetch_variable(&host, addr, VariableType::Uint32, &mut out));
            assert_eq!(unsafe { out.uint32 }, r);

            dst[..4].copy_from_slice(&(r as i32).to_ne_bytes());
            assert!(fetch_variable(&host, addr, VariableType::Sint32, &mut out));
            assert_eq!(unsafe { out.sint32 }, r as i32);

            // Bit pattern comparison so a NaN payload is still checked.
            dst[..4].copy_from_slice(&r.to_ne_bytes());
            assert!(fetch_variable(&host, addr, VariableType::Float32, &mut out));
            assert_eq!(unsafe { out.float32 }.to_bits(), r);

            let b = (r & 1) == 1;
            dst[0] = u8::from(b);
            assert!(fetch_variable(&host, addr, VariableType::Boolean, &mut out));
            assert_eq!(unsafe { out.boolean }, b);

            #[cfg(feature = "support_64bits")]
            {
                let r64 = (u64::from(xorshift32(&mut rng)) << 32) | u64::from(r);

                dst[..8].copy_from_slice(&r64.to_ne_bytes());
                assert!(fetch_variable(&host, addr, VariableType::Uint64, &mut out));
                assert_eq!(unsafe { out.uint64 }, r64);

                dst[..8].copy_from_slice(&(r64 as i64).to_ne_bytes());
                assert!(fetch_variable(&host, addr, VariableType::Sint64, &mut out));
                assert_eq!(unsafe { out.sint64 }, r64 as i64);

                dst[..8].copy_from_slice(&r64.to_ne_bytes());
                assert!(fetch_variable(&host, addr, VariableType::Float64, &mut out));
                assert_eq!(unsafe { out.float64 }.to_bits(), r64);
            }
        }
    }
}
