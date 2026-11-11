use crate::timebase::Timebase;
use crate::tools::crc32;
use crate::types::Timestamp;

use super::definitions::{
    CommControlSubfn, CommState, CommandId, Request, Response, RxError, RxFsmState, TxError, DISCOVER_MAGIC,
    HEARTBEAT_TIMEOUT_100NS, MAXIMUM_RX_BUFFER_SIZE, MAXIMUM_TX_BUFFER_SIZE, MINIMUM_RX_BUFFER_SIZE,
    MINIMUM_TX_BUFFER_SIZE, RX_TIMEOUT_100NS,
};

/// Manages the transport layer of the Scrutiny protocol.
///
/// Receives raw bytes, validates framing and CRC, and surfaces complete
/// [`Request`]s to the caller. Serialises responses back into the byte stream.
/// Maintains session lifecycle including the heartbeat watchdog.
///
/// Communication is half-duplex and works by polling with a request/response scheme.
///
/// Methods that need to read the current time take `tb: &Timebase` explicitly
/// so the caller keeps sole ownership of the timebase.
pub struct CommHandler<'rxbuf, 'txbuf> {
    active_request: Request<'rxbuf>,
    active_response: Response<'txbuf>,

    // session
    session_counter: u32,
    session_id: u32,
    session_active: bool,
    enabled: bool,
    heartbeat_timestamp: Timestamp,
    last_challenge: u16,
    first_heartbeat_received: bool,

    // state
    state: CommState,

    // RX FSM
    rx_state: RxFsmState,
    rx_error: RxError,
    pending_request: bool,
    last_rx_timestamp: Timestamp,
    /// CRC accumulated over header + payload bytes as they arrive.
    rx_crc: u32,
    /// Bytes of the current frame field assembled so far (overloaded by FSM state).
    per_state_counter: u16,

    // TX
    tx_error: TxError,
    nbytes_to_send: u16,
    nbytes_sent: u16,
}

impl<'rxbuf, 'txbuf> CommHandler<'rxbuf, 'txbuf> {
    /// Creates a new `CommHandler`.
    ///
    /// `session_counter_seed` offsets session IDs to avoid collisions when
    /// multiple Scrutiny devices share one channel.
    /// Self-disables if either buffer is outside the allowed size range.
    pub fn new(rx_buffer: &'rxbuf mut [u8], tx_buffer: &'txbuf mut [u8], session_counter_seed: u32) -> Self {
        let enabled = Self::is_valid_buffer_size(rx_buffer.len(), MINIMUM_RX_BUFFER_SIZE, MAXIMUM_RX_BUFFER_SIZE)
            && Self::is_valid_buffer_size(tx_buffer.len(), MINIMUM_TX_BUFFER_SIZE, MAXIMUM_TX_BUFFER_SIZE);

        Self {
            active_request: Request::new(rx_buffer),
            active_response: Response::new(tx_buffer),
            session_counter: session_counter_seed,
            session_id: 0,
            session_active: false,
            enabled,
            heartbeat_timestamp: 0,
            last_challenge: 0,
            first_heartbeat_received: false,
            state: CommState::Idle,
            rx_state: RxFsmState::WaitForCommand,
            rx_error: RxError::None,
            pending_request: false,
            last_rx_timestamp: 0,
            rx_crc: 0,
            per_state_counter: 0,
            tx_error: TxError::None,
            nbytes_to_send: 0,
            nbytes_sent: 0,
        }
    }

    /// Feeds received bytes into the RX state machine.
    pub fn receive_data(&mut self, data: &[u8], tb: &Timebase) {
        if !self.enabled {
            self.rx_error = RxError::Disabled;
            return;
        }

        if self.state == CommState::Transmitting {
            return; // half-duplex: discard while sending
        }

        if !data.is_empty() {
            if tb.has_expired(self.last_rx_timestamp, RX_TIMEOUT_100NS) {
                self.reset_rx(tb.get_timestamp());
            }
            self.last_rx_timestamp = tb.get_timestamp();

            if self.state == CommState::Idle {
                self.state = CommState::Receiving;
            }
        }

        let mut i = 0usize;
        while i < data.len() && !self.pending_request && self.rx_state != RxFsmState::Error {
            match self.rx_state {
                RxFsmState::WaitForCommand => {
                    self.rx_crc = 0;
                    if data[i] & 0x80 != 0 {
                        self.rx_error = RxError::InvalidCommand;
                        self.rx_state = RxFsmState::Error;
                    } else {
                        self.active_request.command_id = data[i];
                        self.rx_crc = crc32(&data[i..=i], self.rx_crc);
                        self.rx_state = RxFsmState::WaitForSubfunction;
                        i += 1;
                    }
                }

                RxFsmState::WaitForSubfunction => {
                    self.active_request.subfunction_id = data[i];
                    self.rx_crc = crc32(&data[i..=i], self.rx_crc);
                    self.rx_state = RxFsmState::WaitForLength;
                    self.per_state_counter = 0;
                    i += 1;
                }

                RxFsmState::WaitForLength => {
                    if self.per_state_counter == 0 {
                        if data.len() - i >= 2 {
                            self.active_request.data_length = (u16::from(data[i]) << 8) | u16::from(data[i + 1]);
                            self.rx_crc = crc32(&data[i..i + 2], self.rx_crc);
                            self.per_state_counter = 2;
                            i += 2;
                        } else {
                            self.active_request.data_length = u16::from(data[i]) << 8;
                            self.rx_crc = crc32(&data[i..=i], self.rx_crc);
                            self.per_state_counter = 1;
                            i += 1;
                        }
                    } else {
                        self.active_request.data_length |= u16::from(data[i]);
                        self.rx_crc = crc32(&data[i..=i], self.rx_crc);
                        self.per_state_counter = 2;
                        i += 1;
                    }

                    if self.per_state_counter == 2 {
                        if self.active_request.data_length == 0 {
                            self.per_state_counter = 0;
                            self.rx_state = RxFsmState::WaitForCrc;
                        } else {
                            self.per_state_counter = 0;
                            self.rx_state = RxFsmState::WaitForData;
                        }
                    }
                }

                RxFsmState::WaitForData => {
                    let Ok(buf_len) = u16::try_from(self.active_request.buffer.len()) else {
                        self.rx_error = RxError::Overflow;
                        self.rx_state = RxFsmState::Error;
                        break;
                    };

                    if buf_len < self.active_request.data_length {
                        self.rx_error = RxError::Overflow;
                        self.rx_state = RxFsmState::Error;
                        break;
                    }

                    let Ok(available) = u16::try_from(data.len() - i) else {
                        self.rx_error = RxError::Overflow;
                        self.rx_state = RxFsmState::Error;
                        break;
                    };
                    let missing = self.active_request.data_length - self.per_state_counter;
                    let to_read = available.min(missing);
                    let to_read_usize = to_read as usize;

                    let dst = self.per_state_counter as usize;
                    self.active_request.buffer[dst..dst + to_read_usize].copy_from_slice(&data[i..i + to_read_usize]);
                    self.rx_crc = crc32(&data[i..i + to_read_usize], self.rx_crc);
                    self.per_state_counter += to_read;
                    i += to_read_usize;

                    if self.per_state_counter >= self.active_request.data_length {
                        self.per_state_counter = 0;
                        self.rx_state = RxFsmState::WaitForCrc;
                    }
                }

                RxFsmState::WaitForCrc => {
                    // CRC arrives MSB-first; shift each byte in from the right
                    self.active_request.crc = (self.active_request.crc << 8) | u32::from(data[i]);
                    self.per_state_counter += 1;
                    i += 1;

                    if self.per_state_counter >= 4 {
                        self.state = CommState::Idle;
                        if self.rx_crc == self.active_request.crc {
                            self.process_active_request();
                        } else {
                            self.reset_rx(tb.get_timestamp());
                        }
                    }
                }

                RxFsmState::WaitForProcess | RxFsmState::Error => break,
            }
        }
    }

    /// Returns `true` if a complete, CRC-validated request is waiting to be processed.
    /// Stays `true` until [`wait_next_request`](Self::wait_next_request) is called.
    #[must_use]
    pub const fn request_received(&self) -> bool {
        self.pending_request
    }

    /// Returns the pending request, or `None` if no request has been received yet.
    #[must_use]
    pub const fn get_request(&self) -> Option<&Request<'_>> {
        if !self.pending_request {
            return None;
        }
        Some(&self.active_request)
    }

    /// Returns simultaneous references to the pending request and the (reset) response buffer,
    /// or `None` if no request is pending.
    ///
    /// Use this when you need to read the request and fill the response in the same scope —
    /// Rust allows the two borrows because they come from distinct fields.
    pub const fn get_req_and_response(&mut self) -> Option<(&Request<'rxbuf>, &mut Response<'txbuf>)> {
        if !self.pending_request {
            return None;
        }
        self.active_response.reset();
        Some((&self.active_request, &mut self.active_response))
    }

    /// Clears the pending request and readies the RX path for the next frame.
    pub fn wait_next_request(&mut self, tb: &Timebase) {
        self.reset_rx(tb.get_timestamp());
    }

    /// Returns a mutable reference to the response object for the caller to fill.
    /// Set [`Response::command_id`], [`Response::subfunction_id`],
    /// [`Response::response_code`], [`Response::data_length`], and write payload
    /// bytes via [`Response::payload`], then call [`send_response`](Self::send_response).
    pub const fn prepare_response(&mut self) -> &mut Response<'txbuf> {
        self.active_response.reset();
        &mut self.active_response
    }

    /// Queues the response prepared via [`prepare_response`](Self::prepare_response)
    /// for transmission.
    ///
    /// Returns `false` (and records a `tx_error`) if the handler is disabled,
    /// busy, or `data_length` exceeds the TX buffer size.
    pub fn send_response(&mut self) -> bool {
        self.tx_error = TxError::None;

        if !self.enabled {
            self.tx_error = TxError::Disabled;
            return false;
        }
        if self.state != CommState::Idle {
            self.tx_error = TxError::Busy;
            return false;
        }
        let data_len = self.active_response.data_length;
        if usize::from(data_len) > self.active_response.buffer.len() {
            self.reset_tx();
            self.tx_error = TxError::Overflow;
            return false;
        }

        let crc = Self::compute_response_crc(
            self.active_response.command_id | 0x80,
            self.active_response.subfunction_id,
            self.active_response.response_code,
            data_len,
            &self.active_response.buffer[..data_len as usize],
        );
        self.active_response.crc = crc;

        // cmd(1) + subfn(1) + code(1) + len(2) + data(data_len) + crc(4)
        self.nbytes_to_send = 1 + 1 + 1 + 2 + data_len + 4;
        self.nbytes_sent = 0;
        self.state = CommState::Transmitting;
        true
    }

    /// Copies up to `out.len()` pending TX bytes into `out`.  Returns the number
    /// of bytes written.  When all bytes have been popped the handler transitions
    /// back to `Idle` automatically and readies RX for the next frame.
    pub fn pop_data(&mut self, out: &mut [u8], tb: &Timebase) -> u16 {
        if self.state != CommState::Transmitting {
            return 0;
        }

        let remaining = self.nbytes_to_send - self.nbytes_sent;
        let to_copy = u16::try_from(out.len()).unwrap_or(0).min(remaining);
        let mut i = 0u16;

        // Header: cmd|0x80, subfn, code, len_hi, len_lo
        while self.nbytes_sent < 5 && i < to_copy {
            out[i as usize] = match self.nbytes_sent {
                0 => self.active_response.command_id | 0x80,
                1 => self.active_response.subfunction_id,
                2 => self.active_response.response_code,
                3 => (self.active_response.data_length >> 8) as u8,
                4 => (self.active_response.data_length & 0xFF) as u8,
                _ => unreachable!(),
            };
            i += 1;
            self.nbytes_sent += 1;
        }

        // Payload
        if self.nbytes_sent >= 5 && i < to_copy {
            let data_sent = self.nbytes_sent - 5;
            let data_length = self.active_response.data_length;
            if data_sent < data_length {
                let remaining_data = data_length - data_sent;
                let can_copy = (to_copy - i).min(remaining_data);
                let src = data_sent as usize;
                let dst = i as usize;
                let payload = &self.active_response.buffer[src..src + can_copy as usize];
                out[dst..dst + can_copy as usize].copy_from_slice(payload);
                i += can_copy;
                self.nbytes_sent += can_copy;
            }
        }

        // CRC (4 bytes, big-endian)
        let crc_start = self.active_response.data_length + 5;
        while i < to_copy && self.nbytes_sent < self.nbytes_to_send {
            let crc_byte = u32::from(self.nbytes_sent - crc_start);
            out[i as usize] = ((self.active_response.crc >> (24 - crc_byte * 8)) & 0xFF) as u8;
            i += 1;
            self.nbytes_sent += 1;
        }

        if self.nbytes_sent >= self.nbytes_to_send {
            self.reset_tx();
            self.wait_next_request(tb);
        }

        i
    }

    /// Returns the number of TX bytes still pending.
    #[must_use]
    pub fn data_to_send(&self) -> u16 {
        if self.state != CommState::Transmitting {
            return 0;
        }
        self.nbytes_to_send - self.nbytes_sent
    }

    /// Opens a session and returns `true`.  Returns `false` if already connected
    /// or disabled.
    pub fn connect(&mut self, tb: &Timebase) -> bool {
        if !self.enabled || self.session_active {
            return false;
        }
        self.session_id = self.session_counter;
        self.session_counter = self.session_counter.wrapping_add(1);
        self.session_active = true;
        self.first_heartbeat_received = false;
        self.heartbeat_timestamp = tb.get_timestamp();
        self.reset_rx(tb.get_timestamp());
        true
    }

    /// Closes the active session.
    pub fn disconnect(&mut self, tb: &Timebase) {
        self.session_id = 0;
        self.session_active = false;
        self.first_heartbeat_received = false;
        self.reset_rx(tb.get_timestamp());
        self.reset_tx();
    }

    /// Returns `true` if a session with the server is active.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.session_active
    }

    /// Returns the session ID assigned at the last successful [`connect`](Self::connect).
    #[must_use]
    pub const fn get_session_id(&self) -> u32 {
        self.session_id
    }

    /// Resets the heartbeat watchdog.  `challenge` must differ from the previous
    /// call; a repeated value is silently ignored and returns `false`.
    pub const fn heartbeat(&mut self, challenge: u16, tb: &Timebase) -> bool {
        if !self.session_active || !self.enabled {
            return false;
        }
        if challenge != self.last_challenge || !self.first_heartbeat_received {
            self.first_heartbeat_received = true;
            self.heartbeat_timestamp = tb.get_timestamp();
            self.last_challenge = challenge;
            return true;
        }
        false
    }

    /// Must be called regularly.  Drops the session if the heartbeat watchdog
    /// expires.
    pub fn process(&mut self, tb: &Timebase) {
        if self.session_active && tb.has_expired(self.heartbeat_timestamp, HEARTBEAT_TIMEOUT_100NS) {
            self.reset(tb);
        }
    }

    /// `true` while a response is being streamed out.
    #[must_use]
    pub fn transmitting(&self) -> bool {
        self.state == CommState::Transmitting
    }

    /// `true` while a request frame is being assembled.
    #[must_use]
    pub fn receiving(&self) -> bool {
        self.state == CommState::Receiving
    }

    /// `true` if the handler has valid buffers and has not been disabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enables the handler.
    pub const fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disables the handler and resets all state.
    pub fn disable(&mut self, tb: &Timebase) {
        self.reset(tb);
        self.enabled = false;
    }

    /// Last RX error code.
    #[must_use]
    pub const fn get_rx_error(&self) -> RxError {
        self.rx_error
    }

    /// Last TX error code.
    #[must_use]
    pub const fn get_tx_error(&self) -> TxError {
        self.tx_error
    }

    /// Size of the RX buffer in bytes.
    #[must_use]
    pub fn rx_buffer_size(&self) -> u16 {
        u16::try_from(self.active_request.buffer.len()).unwrap_or(0)
    }

    /// Size of the TX buffer in bytes.
    #[must_use]
    pub fn tx_buffer_size(&self) -> u16 {
        u16::try_from(self.active_response.buffer.len()).unwrap_or(0)
    }

    /// Resets all state including the session.
    pub fn reset(&mut self, tb: &Timebase) {
        self.state = CommState::Idle;
        self.last_challenge = 0;
        self.first_heartbeat_received = false;
        self.session_id = 0;
        self.session_active = false;
        if self.enabled {
            self.heartbeat_timestamp = tb.get_timestamp();
            self.reset_rx(tb.get_timestamp());
            self.reset_tx();
        }
    }

    fn reset_rx(&mut self, last_rx_timestamp: Timestamp) {
        self.active_request.reset();
        self.rx_state = RxFsmState::WaitForCommand;
        self.pending_request = false;
        self.rx_error = RxError::None;
        self.last_rx_timestamp = last_rx_timestamp;
        self.rx_crc = 0;
        self.per_state_counter = 0;

        if self.state == CommState::Receiving {
            self.state = CommState::Idle;
        }
    }

    fn reset_tx(&mut self) {
        self.active_response.reset();
        self.nbytes_to_send = 0;
        self.nbytes_sent = 0;
        self.tx_error = TxError::None;

        if self.state == CommState::Transmitting {
            self.state = CommState::Idle;
        }
    }

    fn process_active_request(&mut self) {
        let must_process = if self.session_active {
            true
        } else {
            self.is_discover_request() || self.is_connect_request()
        };

        if must_process {
            self.rx_state = RxFsmState::WaitForProcess;
            self.pending_request = true;
        } else {
            self.reset_rx(self.last_rx_timestamp);
        }
    }

    fn is_discover_request(&self) -> bool {
        if self.active_request.command_id != CommandId::CommControl as u8 {
            return false;
        }
        if self.active_request.subfunction_id != CommControlSubfn::Discover as u8 {
            return false;
        }
        if (self.active_request.data_length as usize) < DISCOVER_MAGIC.len() {
            return false;
        }
        self.active_request.buffer[..DISCOVER_MAGIC.len()] == DISCOVER_MAGIC
    }

    const fn is_connect_request(&self) -> bool {
        self.active_request.command_id == CommandId::CommControl as u8
            && self.active_request.subfunction_id == CommControlSubfn::Connect as u8
    }

    fn is_valid_buffer_size(len: usize, min: u16, max: u16) -> bool {
        u16::try_from(len).is_ok_and(|s| s >= min && s <= max)
    }

    fn compute_response_crc(cmd: u8, subfn: u8, code: u8, data_len: u16, payload: &[u8]) -> u32 {
        let header: [u8; 5] = [cmd, subfn, code, (data_len >> 8) as u8, (data_len & 0xFF) as u8];
        let crc = crc32(&header, 0);
        crc32(payload, crc)
    }
}
