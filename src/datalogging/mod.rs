pub mod datalogger;
pub mod raw_encoder;
pub mod trigger;
pub mod types;

pub use datalogger::{convert_to_compare_type, fetch_operand, DataLogger, DataloggerHost, State};
pub use raw_encoder::RawFormatEncoder;
pub use trigger::{active_condition_for, ActiveCondition, ConditionSharedData};
pub use types::{
    AnyValAndTypePair, Configuration, EncodingType, LoggableItem, Operand,
    SupportedTriggerConditions, TriggerConfig, MAX_OPERANDS,
};

// ── DataLoggerCell ────────────────────────────────────────────────────────────

use core::cell::UnsafeCell;

/// Shared, `Sync` wrapper around `DataLogger` for cross-task ownership transfer.
///
/// Access is guarded by the IPC ownership protocol: only the holder of the
/// ownership token (enforced by `Main2LoopMsg`/`Loop2MainMsg`) may call `get()`.
pub struct DataLoggerCell<'buf>(UnsafeCell<DataLogger<'buf>>);

// SAFETY: `DataLogger` is only accessed while the caller holds the IPC ownership
// token. The protocol ensures mutual exclusion between the main task and the loop task.
unsafe impl Sync for DataLoggerCell<'_> {}

impl<'buf> DataLoggerCell<'buf> {
    /// Creates a new cell backed by `buf`. Pass `trigger_callback` to be notified on trigger.
    pub fn new(buf: &'buf mut [u8], trigger_callback: Option<fn()>) -> Self {
        Self(UnsafeCell::new(DataLogger::new(buf, trigger_callback)))
    }

    /// Returns the total byte capacity of the acquisition buffer.
    /// Safe to call without ownership — `buffer_size` is set once at construction.
    pub fn buffer_size(&self) -> usize {
        // SAFETY: buffer_size is a plain usize written once at construction and never changed.
        unsafe { (*self.0.get()).buffer_size() }
    }

    /// Returns a mutable reference to the inner `DataLogger`.
    ///
    /// # Safety
    /// Caller must hold the IPC ownership token (no other task may be calling methods
    /// on the same cell concurrently).
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn get(&self) -> &mut DataLogger<'buf> {
        &mut *self.0.get()
    }
}
