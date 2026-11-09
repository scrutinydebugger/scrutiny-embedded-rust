use crate::timebase::Timebase;
use crate::tools::crc32;
use crate::types::Timestamp;

use super::definitions::{
    CommControlSubfn, CommandId, CommState, RxError, RxFsmState, TxError,
    DISCOVER_MAGIC, HEARTBEAT_TIMEOUT_100NS, MAXIMUM_RX_BUFFER_SIZE, MAXIMUM_TX_BUFFER_SIZE,
    MINIMUM_RX_BUFFER_SIZE, MINIMUM_TX_BUFFER_SIZE, RX_TIMEOUT_100NS,
};
use super::definitions::Request;

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
pub struct CommHandler<'buf> {
    rx_buffer: &'buf mut [u8],
    tx_buffer: &'buf mut [u8],

    // session
    session_counter: u32,
    session_id: u32,
    session_active: bool,
    enabled: bool,
    heartbeat_ts: Timestamp,
    last_challenge: u16,
    first_heartbeat_received: bool,

    // state
    state: CommState,

    // RX FSM
    rx_state: RxFsmState,
    rx_error: RxError,
    request_received: bool,
    last_rx_ts: Timestamp,
    rx_cmd_id: u8,
    rx_subfn_id: u8,
    /// Payload length declared in the frame header.
    rx_data_length: u16,
    /// CRC accumulated over header + payload bytes as they arrive.
    rx_crc: u32,
    /// Bytes of the current frame field assembled so far (overloaded by FSM state).
    per_state_counter: u16,
    /// CRC extracted from the last four bytes of the frame.
    rx_received_crc: u32,

    // TX
    tx_error: TxError,
    nbytes_to_send: u16,
    nbytes_sent: u16,
    tx_cmd_id: u8,
    tx_subfn_id: u8,
    tx_response_code: u8,
    tx_data_length: u16,
    tx_crc: u32,
}

impl<'buf> CommHandler<'buf> {
    /// Creates a new `CommHandler`.
    ///
    /// `session_counter_seed` offsets session IDs to avoid collisions when
    /// multiple Scrutiny devices share one channel.
    /// Self-disables if either buffer is outside the allowed size range.
    pub fn new(
        rx_buffer: &'buf mut [u8],
        tx_buffer: &'buf mut [u8],
        session_counter_seed: u32,
    ) -> Self {
        let rx_size = rx_buffer.len() as u16;
        let tx_size = tx_buffer.len() as u16;
        let enabled = rx_size >= MINIMUM_RX_BUFFER_SIZE
            && rx_size <= MAXIMUM_RX_BUFFER_SIZE
            && tx_size >= MINIMUM_TX_BUFFER_SIZE
            && tx_size <= MAXIMUM_TX_BUFFER_SIZE;

        Self {
            rx_buffer,
            tx_buffer,
            session_counter: session_counter_seed,
            session_id: 0,
            session_active: false,
            enabled,
            heartbeat_ts: 0,
            last_challenge: 0,
            first_heartbeat_received: false,
            state: CommState::Idle,
            rx_state: RxFsmState::WaitForCommand,
            rx_error: RxError::None,
            request_received: false,
            last_rx_ts: 0,
            rx_cmd_id: 0,
            rx_subfn_id: 0,
            rx_data_length: 0,
            rx_crc: 0,
            per_state_counter: 0,
            rx_received_crc: 0,
            tx_error: TxError::None,
            nbytes_to_send: 0,
            nbytes_sent: 0,
            tx_cmd_id: 0,
            tx_subfn_id: 0,
            tx_response_code: 0,
            tx_data_length: 0,
            tx_crc: 0,
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
            if tb.has_expired(self.last_rx_ts, RX_TIMEOUT_100NS) {
                self.reset_rx(tb.get_timestamp());
                self.state = CommState::Idle;
            }
            self.last_rx_ts = tb.get_timestamp();

            if self.state == CommState::Idle {
                self.state = CommState::Receiving;
            }
        }

        let mut i = 0usize;
        while i < data.len() && !self.request_received && self.rx_state != RxFsmState::Error {
            match self.rx_state {
                RxFsmState::WaitForCommand => {
                    self.rx_crc = 0;
                    if data[i] & 0x80 != 0 {
                        self.rx_error = RxError::InvalidCommand;
                        self.rx_state = RxFsmState::Error;
                    } else {
                        self.rx_cmd_id = data[i];
                        self.rx_crc = crc32(&data[i..=i], 1, self.rx_crc);
                        self.rx_state = RxFsmState::WaitForSubfunction;
                        i += 1;
                    }
                }

                RxFsmState::WaitForSubfunction => {
                    self.rx_subfn_id = data[i];
                    self.rx_crc = crc32(&data[i..=i], 1, self.rx_crc);
                    self.rx_state = RxFsmState::WaitForLength;
                    self.per_state_counter = 0;
                    i += 1;
                }

                RxFsmState::WaitForLength => {
                    if self.per_state_counter == 0 {
                        if data.len() - i >= 2 {
                            self.rx_data_length =
                                (u16::from(data[i]) << 8) | u16::from(data[i + 1]);
                            self.rx_crc = crc32(&data[i..i + 2], 2, self.rx_crc);
                            self.per_state_counter = 2;
                            i += 2;
                        } else {
                            self.rx_data_length = u16::from(data[i]) << 8;
                            self.rx_crc = crc32(&data[i..=i], 1, self.rx_crc);
                            self.per_state_counter = 1;
                            i += 1;
                        }
                    } else {
                        self.rx_data_length |= u16::from(data[i]);
                        self.rx_crc = crc32(&data[i..=i], 1, self.rx_crc);
                        self.per_state_counter = 2;
                        i += 1;
                    }

                    if self.per_state_counter == 2 {
                        if self.rx_data_length == 0 {
                            self.per_state_counter = 0;
                            self.rx_state = RxFsmState::WaitForCrc;
                        } else {
                            self.per_state_counter = 0;
                            self.rx_state = RxFsmState::WaitForData;
                        }
                    }
                }

                RxFsmState::WaitForData => {
                    if u16::try_from(self.rx_buffer.len()).unwrap_or(u16::MAX)
                        < self.rx_data_length
                    {
                        self.rx_error = RxError::Overflow;
                        self.rx_state = RxFsmState::Error;
                        break;
                    }

                    let available = data.len() - i;
                    let missing = (self.rx_data_length - self.per_state_counter) as usize;
                    let to_read = available.min(missing);

                    let dst = self.per_state_counter as usize;
                    self.rx_buffer[dst..dst + to_read].copy_from_slice(&data[i..i + to_read]);
                    self.rx_crc = crc32(&data[i..i + to_read], to_read, self.rx_crc);
                    self.per_state_counter += to_read as u16;
                    i += to_read;

                    if self.per_state_counter >= self.rx_data_length {
                        self.per_state_counter = 0;
                        self.rx_state = RxFsmState::WaitForCrc;
                    }
                }

                RxFsmState::WaitForCrc => {
                    // CRC arrives MSB-first; shift each byte in from the right
                    self.rx_received_crc = (self.rx_received_crc << 8) | u32::from(data[i]);
                    self.per_state_counter += 1;
                    i += 1;

                    if self.per_state_counter >= 4 {
                        self.state = CommState::Idle;
                        if self.rx_crc == self.rx_received_crc {
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
    pub fn request_received(&self) -> bool {
        self.request_received
    }

    /// Returns the pending request, or `None` if no request has been received yet.
    pub fn get_request(&self) -> Option<Request<'_>> {
        if !self.request_received {
            return None;
        }
        Some(Request {
            command_id: self.rx_cmd_id,
            subfunction_id: self.rx_subfn_id,
            data: &self.rx_buffer[..self.rx_data_length as usize],
        })
    }

    /// Clears the pending request and readies the RX path for the next frame.
    pub fn wait_next_request(&mut self, tb: &Timebase) {
        self.reset_rx(tb.get_timestamp());
    }

    /// Returns a mutable slice of the TX buffer for the caller to fill with
    /// response payload bytes.  Call [`send_response`](Self::send_response) with
    /// the matching header fields once the slice has been populated.
    pub fn prepare_response(&mut self) -> &mut [u8] {
        self.tx_buffer
    }

    /// Queues a response for transmission.
    ///
    /// The payload must already be written into the buffer returned by
    /// [`prepare_response`](Self::prepare_response).
    /// Returns `false` (and records a `tx_error`) if the handler is disabled,
    /// busy, or `data_len` exceeds the TX buffer size.
    pub fn send_response(
        &mut self,
        command_id: u8,
        subfunction_id: u8,
        response_code: u8,
        data_len: u16,
    ) -> bool {
        self.tx_error = TxError::None;

        if !self.enabled {
            self.tx_error = TxError::Disabled;
            return false;
        }
        if self.state != CommState::Idle {
            self.tx_error = TxError::Busy;
            return false;
        }
        if usize::from(data_len) > self.tx_buffer.len() {
            self.reset_tx();
            self.tx_error = TxError::Overflow;
            return false;
        }

        self.tx_cmd_id = command_id | 0x80;
        self.tx_subfn_id = subfunction_id;
        self.tx_response_code = response_code;
        self.tx_data_length = data_len;
        self.tx_crc = Self::compute_response_crc(
            self.tx_cmd_id,
            self.tx_subfn_id,
            self.tx_response_code,
            self.tx_data_length,
            &self.tx_buffer[..data_len as usize],
        );

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
        let to_copy = out.len().min(remaining as usize) as u16;
        let mut i = 0u16;

        // Header: cmd, subfn, code, len_hi, len_lo
        while self.nbytes_sent < 5 && i < to_copy {
            out[i as usize] = match self.nbytes_sent {
                0 => self.tx_cmd_id,
                1 => self.tx_subfn_id,
                2 => self.tx_response_code,
                3 => (self.tx_data_length >> 8) as u8,
                4 => (self.tx_data_length & 0xFF) as u8,
                _ => unreachable!(),
            };
            i += 1;
            self.nbytes_sent += 1;
        }

        // Payload
        if self.nbytes_sent >= 5 && i < to_copy {
            let data_sent = self.nbytes_sent - 5;
            if data_sent < self.tx_data_length {
                let remaining_data = self.tx_data_length - data_sent;
                let can_copy = (to_copy - i).min(remaining_data);
                let src = data_sent as usize;
                out[i as usize..(i + can_copy) as usize]
                    .copy_from_slice(&self.tx_buffer[src..src + can_copy as usize]);
                i += can_copy;
                self.nbytes_sent += can_copy;
            }
        }

        // CRC (4 bytes, big-endian)
        let crc_start = self.tx_data_length + 5;
        while i < to_copy && self.nbytes_sent < self.nbytes_to_send {
            let crc_byte = self.nbytes_sent - crc_start;
            out[i as usize] = ((self.tx_crc >> (24 - crc_byte * 8)) & 0xFF) as u8;
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
        self.heartbeat_ts = tb.get_timestamp();
        self.reset_rx(tb.get_timestamp());
        true
    }

    /// Closes the active session.
    pub fn disconnect(&mut self) {
        self.session_id = 0;
        self.session_active = false;
        self.first_heartbeat_received = false;
        self.reset_rx(0);
        self.reset_tx();
    }

    /// Returns `true` if a session with the server is active.
    pub fn is_connected(&self) -> bool {
        self.session_active
    }

    /// Returns the session ID assigned at the last successful [`connect`](Self::connect).
    pub fn get_session_id(&self) -> u32 {
        self.session_id
    }

    /// Resets the heartbeat watchdog.  `challenge` must differ from the previous
    /// call; a repeated value is silently ignored and returns `false`.
    pub fn heartbeat(&mut self, challenge: u16, tb: &Timebase) -> bool {
        if !self.session_active || !self.enabled {
            return false;
        }
        if challenge != self.last_challenge || !self.first_heartbeat_received {
            self.first_heartbeat_received = true;
            self.heartbeat_ts = tb.get_timestamp();
            self.last_challenge = challenge;
            return true;
        }
        false
    }

    /// Must be called regularly.  Drops the session if the heartbeat watchdog
    /// expires.
    pub fn process(&mut self, tb: &Timebase) {
        if self.session_active && tb.has_expired(self.heartbeat_ts, HEARTBEAT_TIMEOUT_100NS) {
            self.reset(tb);
        }
    }

    /// `true` while a response is being streamed out.
    pub fn transmitting(&self) -> bool {
        self.state == CommState::Transmitting
    }

    /// `true` while a request frame is being assembled.
    pub fn receiving(&self) -> bool {
        self.state == CommState::Receiving
    }

    /// `true` if the handler has valid buffers and has not been disabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enables the handler.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disables the handler and resets all state.
    pub fn disable(&mut self, tb: &Timebase) {
        self.reset(tb);
        self.enabled = false;
    }

    /// Last RX error code.
    pub fn get_rx_error(&self) -> RxError {
        self.rx_error
    }

    /// Last TX error code.
    pub fn get_tx_error(&self) -> TxError {
        self.tx_error
    }

    /// Size of the RX buffer in bytes.
    pub fn rx_buffer_size(&self) -> u16 {
        self.rx_buffer.len() as u16
    }

    /// Size of the TX buffer in bytes.
    pub fn tx_buffer_size(&self) -> u16 {
        self.tx_buffer.len() as u16
    }

    /// Resets all state including the session.
    pub fn reset(&mut self, tb: &Timebase) {
        self.state = CommState::Idle;
        self.last_challenge = 0;
        self.first_heartbeat_received = false;
        self.session_id = 0;
        self.session_active = false;
        if self.enabled {
            self.heartbeat_ts = tb.get_timestamp();
            self.reset_rx(tb.get_timestamp());
            self.reset_tx();
        }
    }

    fn reset_rx(&mut self, last_rx_ts: Timestamp) {
        self.rx_cmd_id = 0;
        self.rx_subfn_id = 0;
        self.rx_data_length = 0;
        self.rx_received_crc = 0;
        self.rx_state = RxFsmState::WaitForCommand;
        self.request_received = false;
        self.rx_error = RxError::None;
        self.last_rx_ts = last_rx_ts;
        self.rx_crc = 0;
        self.per_state_counter = 0;

        if self.state == CommState::Receiving {
            self.state = CommState::Idle;
        }
    }

    fn reset_tx(&mut self) {
        self.tx_data_length = 0;
        self.nbytes_to_send = 0;
        self.nbytes_sent = 0;
        self.tx_error = TxError::None;

        if self.state == CommState::Transmitting {
            self.state = CommState::Idle;
        }
    }

    fn process_active_request(&mut self) {
        let must_process = if !self.session_active {
            self.is_discover_request() || self.is_connect_request()
        } else {
            true
        };

        if must_process {
            self.rx_state = RxFsmState::WaitForProcess;
            self.request_received = true;
        } else {
            self.reset_rx(self.last_rx_ts);
        }
    }

    fn is_discover_request(&self) -> bool {
        if self.rx_cmd_id != CommandId::CommControl as u8 {
            return false;
        }
        if self.rx_subfn_id != CommControlSubfn::Discover as u8 {
            return false;
        }
        if (self.rx_data_length as usize) < DISCOVER_MAGIC.len() {
            return false;
        }
        self.rx_buffer[..DISCOVER_MAGIC.len()] == DISCOVER_MAGIC
    }

    fn is_connect_request(&self) -> bool {
        self.rx_cmd_id == CommandId::CommControl as u8
            && self.rx_subfn_id == CommControlSubfn::Connect as u8
    }

    fn compute_response_crc(
        cmd: u8,
        subfn: u8,
        code: u8,
        data_len: u16,
        payload: &[u8],
    ) -> u32 {
        let header: [u8; 5] = [
            cmd,
            subfn,
            code,
            (data_len >> 8) as u8,
            (data_len & 0xFF) as u8,
        ];
        let crc = crc32(&header, 5, 0);
        crc32(payload, payload.len(), crc)
    }
}
