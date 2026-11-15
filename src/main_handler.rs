use crate::build_config;
use crate::config::Config;
use crate::protocol::codec_v1_0;
use crate::protocol::definitions::{CommControlSubfn, CommandId, ResponseCode, CONNECT_MAGIC};
use crate::protocol::CommHandler;
use crate::timebase::Timebase;
use crate::types::{Timediff, Timestamp};

/// Top-level Scrutiny handler. Owns the communication buffers and drives the protocol.
pub struct MainHandler<'rxbuf, 'txbuf> {
    comm: CommHandler,
    rx_buf: &'rxbuf mut [u8],
    tx_buf: &'txbuf mut [u8],
    timebase: Timebase,
    enabled: bool,
    processing_request: bool,
    disconnect_pending: bool,
    display_name: &'static str,
    max_bitrate: u32,
    process_again_timestamp: Timestamp,
    process_again_timestamp_taken: bool,
}

impl<'rxbuf, 'txbuf> MainHandler<'rxbuf, 'txbuf> {
    /// Consumes `config` and returns an initialised handler, or `None` if the config is invalid.
    #[must_use]
    pub fn new(mut config: Config<'rxbuf, 'txbuf>) -> Option<Self> {
        let rx_buf = config.take_rx_buffer()?;
        let tx_buf = config.take_tx_buffer()?;
        let seed = config.session_counter_seed;
        let display_name = config.display_name;
        let max_bitrate = config.max_bitrate;

        let comm = CommHandler::new(rx_buf, tx_buf, seed);
        let enabled = comm.is_enabled();

        Some(Self {
            comm,
            rx_buf,
            tx_buf,
            timebase: Timebase::new(),
            enabled,
            processing_request: false,
            disconnect_pending: false,
            display_name,
            max_bitrate,
            process_again_timestamp: 0,
            process_again_timestamp_taken: false,
        })
    }

    /// Must be called as fast as possible. `timestep_100ns` is the elapsed time since the last call.
    pub fn process(&mut self, timestep_100ns: Timediff) {
        if !self.enabled {
            self.processing_request = false;
            self.disconnect_pending = false;
            self.process_again_timestamp_taken = false;
            self.comm.reset(&self.timebase);
            return;
        }

        self.timebase.step(timestep_100ns);
        self.comm.process(&self.timebase);

        if self.comm.request_received() && !self.processing_request {
            let Some((cmd_id, subfn_id, data_len)) = self.comm.get_request() else {
                self.check_finished_sending();
                return;
            };

            let data_len_usize = usize::from(data_len);
            if data_len_usize > self.rx_buf.len() {
                // CommHandler guarantees data_length <= rx_buf.len(), so this is unreachable
                // in normal operation. Bail silently rather than panic.
                self.enabled = false;
                return;
            }

            // Reset so dispatch always starts from a clean response; prevents stale
            // data_length from a previous ProcessAgain cycle leaking into the response.
            self.comm.active_response_mut().reset();
            let data = &self.rx_buf[..data_len_usize];

            let code = Self::dispatch(
                &mut self.comm,
                &mut *self.tx_buf,
                self.display_name,
                self.max_bitrate,
                &self.timebase,
                &mut self.disconnect_pending,
                cmd_id,
                subfn_id,
                data,
            );

            if code == ResponseCode::ProcessAgain {
                if !self.process_again_timestamp_taken {
                    self.process_again_timestamp = self.timebase.get_timestamp();
                    self.process_again_timestamp_taken = true;
                } else if self.timebase.has_expired(
                    self.process_again_timestamp,
                    build_config::REQUEST_MAX_PROCESS_TIME_US * 10,
                ) {
                    {
                        let resp = self.comm.active_response_mut();
                        resp.command_id = cmd_id;
                        resp.subfunction_id = subfn_id;
                        resp.response_code = ResponseCode::FailureToProceed as u8;
                        resp.data_length = 0;
                    }
                    self.comm.send_response(&*self.tx_buf);
                    self.processing_request = true;
                }
            } else {
                {
                    let resp = self.comm.active_response_mut();
                    resp.command_id = cmd_id;
                    resp.subfunction_id = subfn_id;
                    resp.response_code = code as u8;
                    if code != ResponseCode::Ok {
                        resp.data_length = 0;
                    }
                }
                self.processing_request = true;
                if code != ResponseCode::NoResponseToSend {
                    self.comm.send_response(&*self.tx_buf);
                }
            }
        }

        self.check_finished_sending();
    }

    /// Feeds received bytes into the protocol input stream.
    pub fn receive_data(&mut self, data: &[u8]) {
        self.comm.receive_data(data, &mut *self.rx_buf, &self.timebase);
    }

    /// Copies up to `out.len()` pending TX bytes into `out`. Returns the number of bytes written.
    pub fn pop_data(&mut self, out: &mut [u8]) -> u16 {
        let n = self.comm.pop_data(out, &*self.tx_buf, &self.timebase);
        self.check_finished_sending();
        n
    }

    /// Returns the number of TX bytes currently waiting to be popped.
    #[must_use]
    pub fn data_to_send(&self) -> u16 {
        self.comm.data_to_send()
    }

    /// Returns `true` if the handler was successfully initialised and is operational.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Returns `true` if a session with the server is currently active.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.comm.is_connected()
    }

    /// Returns the session ID of the current session (zero when disconnected).
    #[must_use]
    pub const fn get_session_id(&self) -> u32 {
        self.comm.get_session_id()
    }

    fn check_finished_sending(&mut self) {
        if self.processing_request && !self.comm.transmitting() {
            self.comm.wait_next_request(&self.timebase);
            self.processing_request = false;
            self.process_again_timestamp_taken = false;

            if self.disconnect_pending {
                self.comm.disconnect(&self.timebase);
                self.disconnect_pending = false;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        comm: &mut CommHandler,
        tx_buf: &mut [u8],
        display_name: &'static str,
        max_bitrate: u32,
        timebase: &Timebase,
        disconnect_pending: &mut bool,
        cmd_id: u8,
        subfn_id: u8,
        data: &[u8],
    ) -> ResponseCode {
        match CommandId::try_from(cmd_id) {
            Ok(CommandId::CommControl) => Self::process_comm_control(
                comm,
                tx_buf,
                display_name,
                max_bitrate,
                timebase,
                disconnect_pending,
                subfn_id,
                data,
            ),
            Ok(CommandId::GetInfo | CommandId::MemoryControl | CommandId::UserCommand | CommandId::DataLogControl)
            | Err(()) => ResponseCode::UnsupportedFeature,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process_comm_control(
        comm: &mut CommHandler,
        tx_buf: &mut [u8],
        display_name: &'static str,
        max_bitrate: u32,
        timebase: &Timebase,
        disconnect_pending: &mut bool,
        subfn_id: u8,
        data: &[u8],
    ) -> ResponseCode {
        match CommControlSubfn::try_from(subfn_id) {
            Ok(CommControlSubfn::Discover) => {
                match codec_v1_0::encode_response_comm_discover(data, display_name, tx_buf) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(CommControlSubfn::Heartbeat) => {
                // Request: session_id(4) + challenge(2) = 6 bytes exactly.
                if data.len() != 6 {
                    return ResponseCode::InvalidRequest;
                }
                let session_id = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                let challenge = u16::from_be_bytes([data[4], data[5]]);

                if session_id != comm.get_session_id() {
                    return ResponseCode::InvalidRequest;
                }
                if !comm.heartbeat(challenge, timebase) {
                    return ResponseCode::InvalidRequest;
                }
                let cur_session_id = comm.get_session_id();
                match codec_v1_0::encode_response_comm_heartbeat(cur_session_id, !challenge, tx_buf) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(CommControlSubfn::GetParams) => {
                let rx_size = comm.rx_buffer_size();
                let tx_size = comm.tx_buffer_size();
                let addr_size = u8::try_from(core::mem::size_of::<*const u8>()).unwrap_or(u8::MAX);
                match codec_v1_0::encode_response_comm_get_params(
                    rx_size,
                    tx_size,
                    max_bitrate,
                    build_config::COMM_HEARTBEAT_TIMEOUT_US,
                    build_config::COMM_RX_TIMEOUT_US,
                    addr_size,
                    8, // Rust does not support 16 bits bytes
                    tx_buf,
                ) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(CommControlSubfn::Connect) => {
                if data.len() != CONNECT_MAGIC.len() || data != CONNECT_MAGIC {
                    return ResponseCode::InvalidRequest;
                }
                if comm.is_connected() {
                    return ResponseCode::Busy;
                }
                if !comm.connect(timebase) {
                    return ResponseCode::FailureToProceed;
                }
                let session_id = comm.get_session_id();
                match codec_v1_0::encode_response_comm_connect(session_id, tx_buf) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(CommControlSubfn::Disconnect) => {
                // Request: session_id(4) = 4 bytes exactly.
                if data.len() != 4 {
                    return ResponseCode::InvalidRequest;
                }
                let session_id = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                if comm.is_connected() {
                    if comm.get_session_id() != session_id {
                        return ResponseCode::InvalidRequest;
                    }
                    *disconnect_pending = true;
                }
                ResponseCode::Ok
            }

            Err(()) => ResponseCode::UnsupportedFeature,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_fails_without_buffers() {
        let cfg = Config::new();
        assert!(MainHandler::new(cfg).is_none());
    }

    #[test]
    fn init_succeeds_with_valid_buffers() {
        let mut rx = [0u8; 64];
        let mut tx = [0u8; 64];
        let mut cfg = Config::new();
        cfg.set_buffers(&mut rx, &mut tx);
        let handler = MainHandler::new(cfg).unwrap();
        assert!(handler.is_enabled());
    }
}
