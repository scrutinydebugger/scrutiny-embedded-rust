#[cfg(feature = "datalogging")]
use crate::types::Timediff;

#[cfg(feature = "datalogging")]
use core::cell::UnsafeCell;
#[cfg(feature = "datalogging")]
use crate::config::RpvReadCallback;
#[cfg(feature = "datalogging")]
use crate::datalogging::{DataLoggerCell, DataloggerHost, State};
#[cfg(feature = "datalogging")]
use crate::ipc::{IpcMessage, Loop2MainMsg, Loop2MainMsgKind, Main2LoopMsg, StdAtomicFlag};
#[cfg(feature = "datalogging")]
use crate::timebase::Timebase;
#[cfg(feature = "datalogging")]
use crate::types::{AnyType, RuntimePublishedValue};
#[cfg(feature = "datalogging")]
use core::ptr;

/// Loop type: fixed frequency or variable frequency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LoopType {
    /// Loop with a constant time step between calls.
    FixedFreq = 0,
    /// Loop where the time step varies between calls.
    VariableFreq = 1,
}

#[cfg(feature = "datalogging")]
/// Per-loop datalogging runtime state, accessed only from the loop task.
struct LoopDlRuntime {
    owns:           bool,
    data_acquired:  bool,
    /// Address of the `DataLoggerCell` received via `TakeOwnership` IPC; null when not owned.
    cell:           *const DataLoggerCell,
    rpvs:        Option<&'static [RuntimePublishedValue]>,
    rpv_read_cb: Option<RpvReadCallback>,
}

#[cfg(feature = "datalogging")]
impl LoopDlRuntime {
    const DEFAULT: Self = Self {
        owns:         false,
        data_acquired: false,
        cell:         ptr::null(),
        rpvs:         None,
        rpv_read_cb:  None,
    };
}

#[cfg(feature = "datalogging")]
/// Private host implementation built from `LoopDlRuntime` fields.
struct LoopHost {
    rpvs:        Option<&'static [RuntimePublishedValue]>,
    rpv_read_cb: Option<RpvReadCallback>,
    caller_name: &'static str,
}

#[cfg(feature = "datalogging")]
impl DataloggerHost for LoopHost {
    unsafe fn read_memory(&self, dst: *mut u8, src: *const u8, count: usize) -> bool {
        core::ptr::copy_nonoverlapping(src, dst, count);
        true
    }

    fn is_rpv_read_configured(&self) -> bool {
        self.rpv_read_cb.is_some() && self.rpvs.is_some_and(|s| !s.is_empty())
    }

    fn rpv_exists(&self, id: u16) -> bool {
        self.rpvs.is_some_and(|s| s.iter().any(|r| r.id == id))
    }

    fn get_rpv(&self, id: u16) -> Option<RuntimePublishedValue> {
        self.rpvs.and_then(|s| s.iter().find(|r| r.id == id).copied())
    }

    fn read_rpv(&self, rpv: RuntimePublishedValue, out: &mut AnyType) -> bool {
        self.rpv_read_cb.is_some_and(|cb| cb(rpv, out, Some(self.caller_name)))
    }

    fn caller_name(&self) -> Option<&'static str> { Some(self.caller_name) }
}

/// Describes a time domain in which scrutiny-related probing can occur.
/// Registered with `Config` so the server can discover loop properties via `GetInfo`.
pub struct LoopHandler {
    name: &'static str,
    loop_type: LoopType,
    timestep_100ns: u32,
    #[cfg(feature = "datalogging")]
    datalogging_allowed: bool,
    // Loop-private mutable state: only the loop task writes these;
    // the main handler only reads IpcMessage fields (already Sync).
    #[cfg(feature = "datalogging")]
    timebase: UnsafeCell<Timebase>,
    #[cfg(feature = "datalogging")]
    dl_runtime: UnsafeCell<LoopDlRuntime>,
    #[cfg(feature = "datalogging")]
    main2loop: IpcMessage<Main2LoopMsg, StdAtomicFlag>,
    #[cfg(feature = "datalogging")]
    loop2main: IpcMessage<Loop2MainMsg, StdAtomicFlag>,
}

// SAFETY: `timebase` and `dl_runtime` are `UnsafeCell` fields written exclusively by
// the single loop task calling `process()`. The main handler only touches `main2loop`
// and `loop2main`, which are themselves `Sync` via their atomic-flag invariant.
#[cfg(feature = "datalogging")]
unsafe impl Sync for LoopHandler {}

impl LoopHandler {
    /// Creates a fixed-frequency loop with the given name and timestep in 100 ns units.
    #[must_use]
    pub const fn fixed_freq(name: &'static str, timestep_100ns: u32) -> Self {
        Self {
            name,
            loop_type: LoopType::FixedFreq,
            timestep_100ns,
            #[cfg(feature = "datalogging")]
            datalogging_allowed: false,
            #[cfg(feature = "datalogging")]
            timebase: UnsafeCell::new(Timebase::new()),
            #[cfg(feature = "datalogging")]
            dl_runtime: UnsafeCell::new(LoopDlRuntime::DEFAULT),
            #[cfg(feature = "datalogging")]
            main2loop: IpcMessage::new_std(Main2LoopMsg::TakeOwnership { cell_addr: 0, rpvs: None, rpv_read_cb: None }),
            #[cfg(feature = "datalogging")]
            loop2main: IpcMessage::new_std(Loop2MainMsg::DEFAULT),
        }
    }

    /// Creates a variable-frequency loop with the given name.
    #[must_use]
    pub const fn variable_freq(name: &'static str) -> Self {
        Self {
            name,
            loop_type: LoopType::VariableFreq,
            timestep_100ns: 0,
            #[cfg(feature = "datalogging")]
            datalogging_allowed: false,
            #[cfg(feature = "datalogging")]
            timebase: UnsafeCell::new(Timebase::new()),
            #[cfg(feature = "datalogging")]
            dl_runtime: UnsafeCell::new(LoopDlRuntime::DEFAULT),
            #[cfg(feature = "datalogging")]
            main2loop: IpcMessage::new_std(Main2LoopMsg::TakeOwnership { cell_addr: 0, rpvs: None, rpv_read_cb: None }),
            #[cfg(feature = "datalogging")]
            loop2main: IpcMessage::new_std(Loop2MainMsg::DEFAULT),
        }
    }

    /// Enables datalogging for this loop.
    #[cfg(feature = "datalogging")]
    #[must_use]
    pub const fn with_datalogging(mut self) -> Self {
        self.datalogging_allowed = true;
        self
    }

    /// Returns the loop name.
    #[must_use]
    pub const fn get_name(&self) -> &'static str {
        self.name
    }

    /// Returns the loop type.
    #[must_use]
    pub const fn loop_type(&self) -> LoopType {
        self.loop_type
    }

    /// Returns the fixed timestep in 100 ns units; 0 for variable-frequency loops.
    #[must_use]
    pub const fn get_timestep_100ns(&self) -> u32 {
        self.timestep_100ns
    }

    /// Returns `true` if datalogging is enabled for this loop.
    #[cfg(feature = "datalogging")]
    #[must_use]
    pub const fn datalogging_allowed(&self) -> bool {
        self.datalogging_allowed
    }

    /// IPC channel from main handler to this loop.
    #[cfg(feature = "datalogging")]
    pub(crate) const fn main2loop(&self) -> &IpcMessage<Main2LoopMsg, StdAtomicFlag> {
        &self.main2loop
    }

    /// IPC channel from this loop to the main handler.
    #[cfg(feature = "datalogging")]
    pub(crate) const fn loop2main(&self) -> &IpcMessage<Loop2MainMsg, StdAtomicFlag> {
        &self.loop2main
    }

    /// Must be called on every loop tick. Advances the timebase, handles IPC messages,
    /// and drives the datalogger state machine.
    #[cfg(feature = "datalogging")]
    pub fn process(&self, timestep: Timediff) {
        // SAFETY: only this loop task writes `timebase` and `dl_runtime`.
        let timebase = unsafe { &mut *self.timebase.get() };
        timebase.step(timestep);

        if !self.datalogging_allowed { return; }

        let runtime = unsafe { &mut *self.dl_runtime.get() };

        // Handle inbound IPC message if the outbound slot is free.
        if self.main2loop.has_content() && !self.loop2main.has_content() {
            if let Some(msg) = self.main2loop.pop() {
                match msg {
                    Main2LoopMsg::TakeOwnership { cell_addr, rpvs, rpv_read_cb } => {
                        runtime.cell = cell_addr as *const DataLoggerCell;
                        runtime.owns = true;
                        runtime.data_acquired = false;
                        runtime.rpvs = rpvs;
                        runtime.rpv_read_cb = rpv_read_cb;
                        self.loop2main.send(Loop2MainMsg {
                            kind: Loop2MainMsgKind::OwnershipTaken,
                            ..Loop2MainMsg::DEFAULT
                        });
                    }
                    Main2LoopMsg::ArmTrigger => {
                        if runtime.owns && !runtime.cell.is_null() {
                            // SAFETY: we own the datalogger per IPC protocol.
                            unsafe { (*runtime.cell).get() }.arm_trigger();
                        }
                    }
                    Main2LoopMsg::DisarmTrigger => {
                        if runtime.owns && !runtime.cell.is_null() {
                            // SAFETY: we own the datalogger per IPC protocol.
                            unsafe { (*runtime.cell).get() }.disarm_trigger();
                        }
                    }
                    Main2LoopMsg::ReleaseOwnership => {
                        if runtime.owns {
                            runtime.owns = false;
                            runtime.cell = ptr::null();
                            runtime.rpvs = None;
                            runtime.rpv_read_cb = None;
                            self.loop2main.send(Loop2MainMsg {
                                kind: Loop2MainMsgKind::OwnershipReleased,
                                ..Loop2MainMsg::DEFAULT
                            });
                        }
                    }
                }
            }
        }

        if runtime.owns && !runtime.cell.is_null() {
            // SAFETY: we own the datalogger per IPC protocol.
            let dl = unsafe { (*runtime.cell).get() };
            let host = LoopHost {
                rpvs: runtime.rpvs,
                rpv_read_cb: runtime.rpv_read_cb,
                caller_name: self.name,
            };
            dl.process(timebase, &host);

            if !self.loop2main.has_content() {
                if dl.data_acquired() && !runtime.data_acquired {
                    self.loop2main.send(Loop2MainMsg {
                        kind: Loop2MainMsgKind::DataAcquired,
                        ..Loop2MainMsg::DEFAULT
                    });
                    runtime.data_acquired = true;
                } else {
                    let state = dl.get_state();
                    let (bytes_to_complete, write_counter) = if state == State::Triggered {
                        (dl.get_bytes_to_acquire_from_trigger_to_completion(), dl.data_counter_since_trigger())
                    } else {
                        (0, 0)
                    };
                    self.loop2main.send(Loop2MainMsg {
                        kind: Loop2MainMsgKind::StatusUpdate,
                        state,
                        bytes_to_complete,
                        write_counter,
                    });
                }
            }
        }
    }
}
