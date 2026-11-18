use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// Abstraction over a single atomic boolean flag.
///
/// `load` must use Acquire ordering and `store` Release ordering. Both must be safe to
/// call from concurrent contexts (ISR + main loop).
///
/// To use another flag for the loop IPC channels, implement this trait, add a
/// `const fn new(val: bool) -> Self` and point [`LoopIpcFlag`] to it.
pub trait AtomicFlag: Sync {
    /// Reads the flag with Acquire ordering.
    fn load(&self) -> bool;
    /// Writes the flag with Release ordering.
    fn store(&self, val: bool);
}

// Default implementation: core::sync::atomic::AtomicBool

/// `AtomicFlag` implementation backed by `core::sync::atomic::AtomicBool`.
/// Suitable for any target that supports the standard Rust atomic intrinsics.
pub struct StdAtomicFlag(AtomicBool);

impl StdAtomicFlag {
    /// Creates a new `StdAtomicFlag` with the given initial value.
    #[must_use]
    pub const fn new(val: bool) -> Self {
        Self(AtomicBool::new(val))
    }
}

impl AtomicFlag for StdAtomicFlag {
    fn load(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn store(&self, val: bool) {
        self.0.store(val, Ordering::Release);
    }
}

/// Flag type used by the loop IPC channels.
// Single place to change (or later make cfg-selectable) to use another platform flag.
pub type LoopIpcFlag = StdAtomicFlag;

// IpcMessage

/// Single-slot, single-producer / single-consumer message channel.
///
/// The sender calls `send()` once; the receiver calls `pop()` to consume it.
/// An atomic flag guards the slot: the sender sets it after writing the payload
/// (Release), and the receiver clears it after reading (Acquire).
///
/// `T` must be `Copy`. `F` is the platform-specific flag.
pub struct IpcMessage<T: Copy, F: AtomicFlag> {
    data: UnsafeCell<T>,
    flag: F,
}

// SAFETY: Access to `data` is serialised through `flag` with Acquire/Release ordering.
// `T: Send` ensures the value itself is safe to transfer across thread/ISR boundaries.
unsafe impl<T: Copy + Send, F: AtomicFlag> Sync for IpcMessage<T, F> {}

impl<T: Copy, F: AtomicFlag> IpcMessage<T, F> {
    /// Returns `true` if an unconsumed message is waiting in the slot.
    pub fn has_content(&self) -> bool {
        self.flag.load()
    }

    /// Writes `value` into the slot and sets the flag.
    /// If the previous message was not yet consumed, it is silently overwritten.
    pub fn send(&self, value: T) {
        // SAFETY: We write before setting the flag (Release in `flag.store`), so the
        // reader always sees a fully-written value once the flag reads `true`.
        unsafe { *self.data.get() = value };
        self.flag.store(true);
    }

    /// Consumes the message and returns it, or returns `None` if the slot is empty.
    pub fn pop(&self) -> Option<T> {
        if !self.flag.load() {
            return None;
        }
        // SAFETY: `flag.load()` returned true with Acquire ordering, so the write
        // in `send()` (Release) happened-before this read.
        let val = unsafe { *self.data.get() };
        self.flag.store(false);
        Some(val)
    }

    /// Discards any pending message by clearing the flag.
    // Loop IPC channels only exist with datalogging.
    #[cfg(any(feature = "datalogging", test))]
    pub(crate) fn clear(&self) {
        self.flag.store(false);
    }
}

impl<T: Copy> IpcMessage<T, StdAtomicFlag> {
    /// Creates a new `IpcMessage` backed by `StdAtomicFlag`.
    #[must_use]
    pub const fn new_std(init: T) -> Self {
        Self {
            data: UnsafeCell::new(init),
            flag: StdAtomicFlag::new(false),
        }
    }
}

// Loop IPC channels only exist with datalogging.
#[cfg(feature = "datalogging")]
impl<T: Copy> IpcMessage<T, LoopIpcFlag> {
    /// Creates a new `IpcMessage` backed by `LoopIpcFlag`, for the loop IPC channels.
    #[must_use]
    pub(crate) const fn new_loop(init: T) -> Self {
        Self {
            data: UnsafeCell::new(init),
            flag: LoopIpcFlag::new(false),
        }
    }
}

// Datalogging IPC message types

#[cfg(feature = "datalogging")]
/// Messages the main handler sends to a loop handler.
#[derive(Clone, Copy)]
pub enum Main2LoopMsg {
    /// `cell_addr` is the `*const DataLoggerCell` cast to `usize` to stay `Send`.
    TakeOwnership {
        cell_addr: usize,
        rpvs: Option<&'static [crate::types::RuntimePublishedValue]>,
        rpv_read_cb: Option<crate::config::RpvReadCallback>,
    },
    ReleaseOwnership,
    ArmTrigger,
    DisarmTrigger,
}

#[cfg(feature = "datalogging")]
/// Messages a loop handler sends back to the main handler.
#[derive(Clone, Copy, Debug)]
pub struct Loop2MainMsg {
    pub kind: Loop2MainMsgKind,
    pub state: crate::datalogging::State,
    pub bytes_to_complete: usize,
    pub write_counter: usize,
}

#[cfg(feature = "datalogging")]
impl Loop2MainMsg {
    pub(crate) const DEFAULT: Self = Self {
        kind: Loop2MainMsgKind::StatusUpdate,
        state: crate::datalogging::State::Idle,
        bytes_to_complete: 0,
        write_counter: 0,
    };
}

#[cfg(feature = "datalogging")]
/// Discriminant for `Loop2MainMsg`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Loop2MainMsgKind {
    OwnershipTaken,
    OwnershipReleased,
    DataAcquired,
    StatusUpdate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_slot_returns_none() {
        let msg: IpcMessage<u32, StdAtomicFlag> = IpcMessage::new_std(0);
        assert!(!msg.has_content());
        assert_eq!(msg.pop(), None);
    }

    #[test]
    fn send_then_pop_returns_value() {
        let msg: IpcMessage<u32, StdAtomicFlag> = IpcMessage::new_std(0);
        msg.send(42);
        assert!(msg.has_content());
        assert_eq!(msg.pop(), Some(42));
        assert!(!msg.has_content());
        assert_eq!(msg.pop(), None);
    }

    #[test]
    fn overwrite_before_pop() {
        let msg: IpcMessage<u32, StdAtomicFlag> = IpcMessage::new_std(0);
        msg.send(1);
        msg.send(2);
        assert_eq!(msg.pop(), Some(2));
        assert_eq!(msg.pop(), None);
    }

    // clear() discards a pending message.
    #[test]
    fn clear_discards_pending_message() {
        let msg: IpcMessage<u32, StdAtomicFlag> = IpcMessage::new_std(0);
        msg.send(5);
        msg.clear();
        assert!(!msg.has_content());
        assert_eq!(msg.pop(), None);
    }

    #[test]
    fn works_with_composite_type() {
        #[derive(Clone, Copy, PartialEq, Debug)]
        struct Point {
            x: i16,
            y: i16,
        }
        let msg: IpcMessage<Point, StdAtomicFlag> = IpcMessage::new_std(Point { x: 0, y: 0 });
        msg.send(Point { x: 3, y: -7 });
        assert_eq!(msg.pop(), Some(Point { x: 3, y: -7 }));
    }

    #[test]
    fn custom_atomic_flag_impl() {
        // Verify the trait is usable with a custom implementation.
        use core::sync::atomic::{AtomicBool, Ordering};
        struct MyFlag(AtomicBool);
        impl AtomicFlag for MyFlag {
            fn load(&self) -> bool {
                self.0.load(Ordering::Acquire)
            }
            fn store(&self, val: bool) {
                self.0.store(val, Ordering::Release);
            }
        }
        impl MyFlag {
            const fn new() -> Self {
                Self(AtomicBool::new(false))
            }
        }

        let msg = IpcMessage {
            data: UnsafeCell::new(99u8),
            flag: MyFlag::new(),
        };
        assert_eq!(msg.pop(), None);
        msg.send(7u8);
        assert_eq!(msg.pop(), Some(7u8));
    }
}
