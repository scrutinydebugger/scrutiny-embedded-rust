use crate::types::{AnyType, RuntimePublishedValue};

#[cfg(feature = "protected_regions")]
use crate::types::AddressRange;

/// Callback to read a Runtime Published Value.
pub type RpvReadCallback = fn(RuntimePublishedValue, &mut AnyType) -> bool;

/// Callback to write a Runtime Published Value.
pub type RpvWriteCallback = fn(RuntimePublishedValue, &AnyType) -> bool;

/// Callback for the `UserCommand` service.
/// Arguments: subfunction, `request_data`, `response_data`, `response_data_length`.
pub type UserCommandCallback = fn(u8, &[u8], &mut [u8], &mut u16);

#[cfg(feature = "datalogging")]
/// Callback triggered when a datalogging acquisition fires.
pub type DataloggingTriggerCallback = fn();

/// Runtime configuration handed to `MainHandler` at startup.
///
/// Caller-allocated buffers are borrowed for lifetime `'buf`; all array pointers
/// (`rpvs`, address ranges) must be `'static` — no copy is made.
pub struct Config<'rxbuf, 'txbuf> {
    rx_buffer: Option<&'rxbuf mut [u8]>,
    tx_buffer: Option<&'txbuf mut [u8]>,

    #[cfg(feature = "protected_regions")]
    forbidden_ranges: Option<&'static [AddressRange]>,
    #[cfg(feature = "protected_regions")]
    readonly_ranges: Option<&'static [AddressRange]>,

    rpvs: Option<&'static [RuntimePublishedValue]>,
    rpv_read_callback: Option<RpvReadCallback>,
    rpv_write_callback: Option<RpvWriteCallback>,

    user_command_callback: Option<UserCommandCallback>,

    /// Maximum bitrate in bits/sec, enforced by the server.
    pub max_bitrate: u32,
    /// Seed for the session counter; avoids ID collision when multiple devices share one channel.
    pub session_counter_seed: u32,
    /// Display name broadcast during the discovery phase.
    pub display_name: &'static str,
    /// When `false`, all memory-write requests are rejected.
    pub memory_write_enable: bool,
}

impl<'rxbuf, 'txbuf> Config<'rxbuf, 'txbuf> {
    /// Creates a new `Config` with safe defaults (`memory_write_enable = true`, everything else unset).
    #[must_use]
    pub fn new() -> Self {
        Self {
            rx_buffer: None,
            tx_buffer: None,
            #[cfg(feature = "protected_regions")]
            forbidden_ranges: None,
            #[cfg(feature = "protected_regions")]
            readonly_ranges: None,
            rpvs: None,
            rpv_read_callback: None,
            rpv_write_callback: None,
            user_command_callback: None,
            max_bitrate: 0,
            session_counter_seed: 0,
            display_name: "",
            memory_write_enable: true,
        }
    }

    /// Resets all fields to their defaults, releasing any borrowed buffers.
    pub fn clear(&mut self) {
        self.rx_buffer = None;
        self.tx_buffer = None;
        #[cfg(feature = "protected_regions")]
        {
            self.forbidden_ranges = None;
            self.readonly_ranges = None;
        }
        self.rpvs = None;
        self.rpv_read_callback = None;
        self.rpv_write_callback = None;
        self.user_command_callback = None;
        self.max_bitrate = 0;
        self.session_counter_seed = 0;
        self.display_name = "";
        self.memory_write_enable = true;
    }

    /// Sets the communication buffers. The bigger the buffers, the more data a single request can carry.
    pub const fn set_buffers(&mut self, rx_buffer: &'rxbuf mut [u8], tx_buffer: &'txbuf mut [u8]) {
        self.rx_buffer = Some(rx_buffer);
        self.tx_buffer = Some(tx_buffer);
    }

    #[cfg(feature = "protected_regions")]
    /// Defines memory regions that Scrutiny will never read or write.
    pub const fn set_forbidden_address_range(&mut self, ranges: &'static [AddressRange]) {
        self.forbidden_ranges = Some(ranges);
    }

    #[cfg(feature = "protected_regions")]
    /// Defines memory regions that Scrutiny will never write.
    pub const fn set_readonly_address_range(&mut self, ranges: &'static [AddressRange]) {
        self.readonly_ranges = Some(ranges);
    }

    /// Configures Runtime Published Values and their read/write callbacks.
    pub const fn set_published_values(
        &mut self,
        array: &'static [RuntimePublishedValue],
        rd_cb: Option<RpvReadCallback>,
        wr_cb: Option<RpvWriteCallback>,
    ) {
        self.rpvs = Some(array);
        self.rpv_read_callback = rd_cb;
        self.rpv_write_callback = wr_cb;
    }

    /// Sets the callback for `UserCommand` service requests.
    pub const fn set_user_command_callback(&mut self, callback: UserCommandCallback) {
        self.user_command_callback = Some(callback);
    }

    /// Returns `true` if both communication buffers have been set.
    #[must_use]
    pub const fn is_buffer_set(&self) -> bool {
        self.rx_buffer.is_some() && self.tx_buffer.is_some()
    }

    #[cfg(feature = "protected_regions")]
    /// Returns `true` if forbidden address ranges have been defined.
    #[must_use]
    pub const fn is_forbidden_address_range_set(&self) -> bool {
        self.forbidden_ranges.is_some()
    }

    #[cfg(feature = "protected_regions")]
    /// Returns `true` if read-only address ranges have been defined.
    #[must_use]
    pub const fn is_readonly_address_range_set(&self) -> bool {
        self.readonly_ranges.is_some()
    }

    /// Returns `true` if RPVs and a read callback are both configured.
    #[must_use]
    pub fn is_read_published_values_configured(&self) -> bool {
        self.rpv_read_callback.is_some() && self.rpvs.is_some_and(|s| !s.is_empty())
    }

    /// Returns `true` if RPVs and a write callback are both configured.
    #[must_use]
    pub fn is_write_published_values_configured(&self) -> bool {
        self.rpv_write_callback.is_some() && self.rpvs.is_some_and(|s| !s.is_empty())
    }

    /// Returns `true` if a `UserCommand` callback has been registered.
    #[must_use]
    pub const fn is_user_command_callback_set(&self) -> bool {
        self.user_command_callback.is_some()
    }

    #[cfg(feature = "protected_regions")]
    /// Returns the forbidden address ranges, or `None` if unset.
    #[must_use]
    pub const fn forbidden_ranges(&self) -> Option<&'static [AddressRange]> {
        self.forbidden_ranges
    }

    #[cfg(feature = "protected_regions")]
    /// Returns the read-only address ranges, or `None` if unset.
    #[must_use]
    pub const fn readonly_ranges(&self) -> Option<&'static [AddressRange]> {
        self.readonly_ranges
    }

    /// Returns the RPV array, or `None` if unset.
    #[must_use]
    pub const fn get_rpvs_array(&self) -> Option<&'static [RuntimePublishedValue]> {
        self.rpvs
    }

    /// Returns the number of configured RPVs.
    #[must_use]
    pub fn get_rpv_count(&self) -> u16 {
        self.rpvs.map_or(0, |s| u16::try_from(s.len()).unwrap_or(u16::MAX))
    }

    /// Returns the RPV read callback, or `None` if unset.
    #[must_use]
    pub const fn get_rpv_read_callback(&self) -> Option<RpvReadCallback> {
        self.rpv_read_callback
    }

    /// Returns the RPV write callback, or `None` if unset.
    #[must_use]
    pub const fn get_rpv_write_callback(&self) -> Option<RpvWriteCallback> {
        self.rpv_write_callback
    }

    /// Returns the `UserCommand` callback, or `None` if unset.
    #[must_use]
    pub const fn get_user_command_callback(&self) -> Option<UserCommandCallback> {
        self.user_command_callback
    }

    /// Moves the RX buffer out of the config; used by `MainHandler` during initialization.
    #[allow(dead_code)]
    pub(crate) const fn take_rx_buffer(&mut self) -> Option<&'rxbuf mut [u8]> {
        self.rx_buffer.take()
    }

    /// Moves the TX buffer out of the config; used by `MainHandler` during initialization.
    #[allow(dead_code)]
    pub(crate) const fn take_tx_buffer(&mut self) -> Option<&'txbuf mut [u8]> {
        self.tx_buffer.take()
    }
}

impl Default for Config<'_, '_> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::VariableType;

    #[test]
    fn new_has_safe_defaults() {
        let cfg = Config::new();
        assert!(!cfg.is_buffer_set());
        assert!(!cfg.is_read_published_values_configured());
        assert!(!cfg.is_write_published_values_configured());
        assert!(!cfg.is_user_command_callback_set());
        assert_eq!(cfg.max_bitrate, 0);
        assert_eq!(cfg.session_counter_seed, 0);
        assert_eq!(cfg.display_name, "");
        assert!(cfg.memory_write_enable);
    }

    #[test]
    fn set_buffers_marks_buffer_set() {
        let mut rx = [0u8; 64];
        let mut tx = [0u8; 64];
        let mut cfg = Config::new();
        assert!(!cfg.is_buffer_set());
        cfg.set_buffers(&mut rx, &mut tx);
        assert!(cfg.is_buffer_set());
    }

    #[test]
    fn clear_releases_buffers_and_resets_fields() {
        let mut rx = [0u8; 64];
        let mut tx = [0u8; 64];
        let mut cfg = Config::new();
        cfg.set_buffers(&mut rx, &mut tx);
        cfg.max_bitrate = 9600;
        cfg.memory_write_enable = false;
        cfg.clear();
        assert!(!cfg.is_buffer_set());
        assert_eq!(cfg.max_bitrate, 0);
        assert!(cfg.memory_write_enable);
    }

    #[test]
    fn rpv_configured_requires_non_empty_array_and_callback() {
        static RPVS: [RuntimePublishedValue; 1] = [RuntimePublishedValue {
            id: 1,
            var_type: VariableType::Uint32,
        }];
        fn read_cb(_rpv: RuntimePublishedValue, _out: &mut AnyType) -> bool {
            true
        }
        fn write_cb(_rpv: RuntimePublishedValue, _in: &AnyType) -> bool {
            true
        }

        let mut cfg = Config::new();
        assert!(!cfg.is_read_published_values_configured());
        assert!(!cfg.is_write_published_values_configured());

        cfg.set_published_values(&RPVS, Some(read_cb), Some(write_cb));
        assert!(cfg.is_read_published_values_configured());
        assert!(cfg.is_write_published_values_configured());
        assert_eq!(cfg.get_rpv_count(), 1);
    }

    #[test]
    fn rpv_read_only_or_write_only() {
        static RPVS: [RuntimePublishedValue; 1] = [RuntimePublishedValue {
            id: 2,
            var_type: VariableType::Sint32,
        }];
        fn read_cb(_rpv: RuntimePublishedValue, _out: &mut AnyType) -> bool {
            false
        }

        let mut cfg = Config::new();
        cfg.set_published_values(&RPVS, Some(read_cb), None);
        assert!(cfg.is_read_published_values_configured());
        assert!(!cfg.is_write_published_values_configured());
    }

    #[test]
    fn user_command_callback() {
        fn my_cb(_subfn: u8, _req: &[u8], _resp: &mut [u8], _len: &mut u16) {}
        let mut cfg = Config::new();
        assert!(!cfg.is_user_command_callback_set());
        cfg.set_user_command_callback(my_cb);
        assert!(cfg.is_user_command_callback_set());
        assert!(cfg.get_user_command_callback().is_some());
    }

    #[test]
    fn take_buffers_empties_the_fields() {
        let mut rx = [0u8; 64];
        let mut tx = [0u8; 64];
        let mut cfg = Config::new();
        cfg.set_buffers(&mut rx, &mut tx);
        assert!(cfg.take_rx_buffer().is_some());
        assert!(cfg.take_tx_buffer().is_some());
        assert!(!cfg.is_buffer_set());
    }

    #[cfg(feature = "protected_regions")]
    #[test]
    fn protected_regions() {
        static FORBIDDEN: [AddressRange; 1] = [AddressRange {
            start: 0x1000 as *const u8,
            end: 0x2000 as *const u8,
        }];
        static READONLY: [AddressRange; 1] = [AddressRange {
            start: 0x3000 as *const u8,
            end: 0x4000 as *const u8,
        }];

        let mut cfg = Config::new();
        assert!(!cfg.is_forbidden_address_range_set());
        assert!(!cfg.is_readonly_address_range_set());

        cfg.set_forbidden_address_range(&FORBIDDEN);
        cfg.set_readonly_address_range(&READONLY);

        assert!(cfg.is_forbidden_address_range_set());
        assert!(cfg.is_readonly_address_range_set());
        assert_eq!(cfg.forbidden_ranges().unwrap().len(), 1);
        assert_eq!(cfg.readonly_ranges().unwrap().len(), 1);
    }
}
