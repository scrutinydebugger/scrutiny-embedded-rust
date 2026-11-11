use crate::build_config;
use crate::config::Config;
use crate::protocol::codec_v1_0;
use crate::protocol::definitions::{CommControlSubfn, CommandId, Response, ResponseCode};
use crate::protocol::CommHandler;
use crate::timebase::Timebase;
use crate::types::{Timediff, Timestamp};

/// Top-level Scrutiny handler. Owns the communication buffers and drives the protocol.
pub struct MainHandler<'rxbuf, 'txbuf> {
    comm: CommHandler<'rxbuf, 'txbuf>,
    timebase: Timebase,
    enabled: bool,
    processing_request: bool,
    disconnect_pending: bool,
    display_name: &'static str,
    process_again_timestamp: Timestamp,
    process_again_timestamp_taken: bool,
}

impl<'rxbuf, 'txbuf> MainHandler<'rxbuf, 'txbuf> {
    /// Consumes `config` and returns an initialised handler, or `None` if the config is invalid.
    #[must_use]
    pub fn new(mut config: Config<'rxbuf, 'txbuf>) -> Option<Self> {
        let rxbuffer = config.take_rx_buffer()?;
        let txbuffer = config.take_tx_buffer()?;
        let seed = config.session_counter_seed;
        let display_name = config.display_name;

        let comm = CommHandler::new(rxbuffer, txbuffer, seed);
        let enabled = comm.is_enabled();

        Some(Self {
            comm,
            timebase: Timebase::new(),
            enabled,
            processing_request: false,
            disconnect_pending: false,
            display_name,
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
            if let Some((req, response)) = self.comm.get_req_and_response() {
                response.command_id = req.command_id;
                response.subfunction_id = req.subfunction_id;
                let code = Self::dispatch(
                    req.command_id,
                    req.subfunction_id,
                    req.data(),
                    self.display_name,
                    response,
                );

                if code == ResponseCode::ProcessAgain {
                    // Handler isn't ready yet; retry next cycle.
                    // If the timeout expires, force a FailureToProceed response.
                    if !self.process_again_timestamp_taken {
                        self.process_again_timestamp = self.timebase.get_timestamp();
                        self.process_again_timestamp_taken = true;
                    } else if self.timebase.has_expired(
                        self.process_again_timestamp,
                        build_config::REQUEST_MAX_PROCESS_TIME_US * 10,
                    ) {
                        response.response_code = ResponseCode::FailureToProceed as u8;
                        response.data_length = 0;
                        self.comm.send_response();
                        self.processing_request = true;
                    }
                } else {
                    response.response_code = code as u8;
                    if code != ResponseCode::Ok {
                        response.data_length = 0;
                    }
                    self.processing_request = true;
                    if code != ResponseCode::NoResponseToSend {
                        self.comm.send_response();
                    }
                }
            }
        }

        self.check_finished_sending();
    }

    /// Feeds received bytes into the protocol input stream.
    pub fn receive_data(&mut self, data: &[u8]) {
        self.comm.receive_data(data, &self.timebase);
    }

    /// Copies up to `out.len()` pending TX bytes into `out`. Returns the number of bytes written.
    pub fn pop_data(&mut self, out: &mut [u8]) -> u16 {
        let n = self.comm.pop_data(out, &self.timebase);
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

    fn dispatch(
        cmd_id: u8,
        subfn_id: u8,
        data: &[u8],
        display_name: &'static str,
        response: &mut Response<'_>,
    ) -> ResponseCode {
        match CommandId::try_from(cmd_id) {
            Ok(CommandId::CommControl) => Self::process_comm_control(subfn_id, data, display_name, response),
            Ok(CommandId::GetInfo | CommandId::MemoryControl | CommandId::UserCommand | CommandId::DataLogControl)
            | Err(()) => ResponseCode::UnsupportedFeature,
        }
    }

    fn process_comm_control(
        subfn_id: u8,
        data: &[u8],
        display_name: &'static str,
        response: &mut Response<'_>,
    ) -> ResponseCode {
        match CommControlSubfn::try_from(subfn_id) {
            Ok(CommControlSubfn::Discover) => codec_v1_0::encode_discover(data, display_name, response),
            Ok(CommControlSubfn::Heartbeat | CommControlSubfn::GetParams | CommControlSubfn::Connect | CommControlSubfn::Disconnect)
            | Err(()) => ResponseCode::UnsupportedFeature,
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
