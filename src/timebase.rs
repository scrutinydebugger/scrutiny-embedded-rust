use crate::types::{Timediff, Timestamp};

/// Minimalist monotonic time tracker driven by explicit `step()` calls.
/// All time values are in 100 ns increments. Arithmetic wraps on `u32` overflow.
pub struct Timebase {
    time_100ns: Timestamp,
}

impl Timebase {
    /// Creates a new timebase initialised to zero.
    #[must_use]
    pub const fn new() -> Self {
        Self { time_100ns: 0 }
    }

    /// Advances the clock by `timestep_100ns` ticks.
    pub const fn step(&mut self, timestep_100ns: Timediff) {
        self.time_100ns = self.time_100ns.wrapping_add(timestep_100ns);
    }

    /// Returns the current time as an opaque snapshot for use with [`elapsed_since`] and [`has_expired`].
    #[must_use]
    pub const fn get_timestamp(&self) -> Timestamp {
        self.time_100ns
    }

    /// Returns the number of 100 ns ticks elapsed since `timestamp`.
    #[must_use]
    pub const fn elapsed_since(&self, timestamp: Timestamp) -> Timediff {
        self.time_100ns.wrapping_sub(timestamp)
    }

    /// Returns the number of whole microseconds elapsed since `timestamp`.
    #[must_use]
    pub const fn elapsed_us_since(&self, timestamp: Timestamp) -> Timediff {
        self.elapsed_since(timestamp) / 10
    }

    /// Returns `true` if at least `timeout_100ns` ticks have elapsed since `timestamp`.
    #[must_use]
    pub const fn has_expired(&self, timestamp: Timestamp, timeout_100ns: Timediff) -> bool {
        self.elapsed_since(timestamp) >= timeout_100ns
    }

    /// Resets the internal counter to `val` (pass `0` for a full reset).
    pub const fn reset(&mut self, val: Timestamp) {
        self.time_100ns = val;
    }
}

impl Default for Timebase {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_timeouts() {
        let mut tb = Timebase::new();

        let ts = tb.get_timestamp();
        tb.step(100);
        assert!(tb.has_expired(ts, 99));
        assert!(tb.has_expired(ts, 100));
        assert!(!tb.has_expired(ts, 101));

        let ts = tb.get_timestamp();
        tb.step(100);
        assert!(tb.has_expired(ts, 99));
        assert!(tb.has_expired(ts, 100));
        assert!(!tb.has_expired(ts, 101));

        tb.reset(0);
        let ts = tb.get_timestamp();
        assert_eq!(ts, 0);

        tb.step(0x7FFF_FFFF);
        assert!(tb.has_expired(ts, 0x7FFF_FFFE));
        assert!(tb.has_expired(ts, 0x7FFF_FFFF));
        assert!(!tb.has_expired(ts, 0x8000_0000));

        // Wrapping: start near u32::MAX, step past it
        tb.reset(0);
        tb.step(0xFFFF_FFFF);
        let ts = tb.get_timestamp();
        tb.step(2);
        assert!(tb.has_expired(ts, 1));
        assert!(tb.has_expired(ts, 2));
        assert!(!tb.has_expired(ts, 3));
    }

    #[test]
    fn test_elapsed_us_since() {
        let mut tb = Timebase::new();
        let ts = tb.get_timestamp();
        tb.step(100); // 100 * 100 ns = 10 us
        assert_eq!(tb.elapsed_us_since(ts), 10);
        tb.step(5); // total 105 * 100 ns = 10 us (floored)
        assert_eq!(tb.elapsed_us_since(ts), 10);
        tb.step(5); // total 110 * 100 ns = 11 us
        assert_eq!(tb.elapsed_us_since(ts), 11);
    }
}
