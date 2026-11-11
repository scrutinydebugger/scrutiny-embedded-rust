use crate::config::Config;
use crate::protocol::codec_v1_0;
use crate::protocol::definitions::{CommControlSubfn, CommandId, Response, ResponseCode};
use crate::protocol::CommHandler;
use crate::timebase::Timebase;
use crate::types::Timediff;

/// Top-level Scrutiny handler. Owns the communication buffers and drives the protocol.
pub struct MainHandler<'rxbuf, 'txbuf> {
    comm: CommHandler<'rxbuf, 'txbuf>,
    timebase: Timebase,
    enabled: bool,
    processing_request: bool,
    disconnect_pending: bool,
    display_name: &'static str,
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
        })
    }

    /// Must be called as fast as possible. `timestep_100ns` is the elapsed time since the last call.
    pub fn process(&mut self, timestep_100ns: Timediff) {
        if !self.enabled {
            self.processing_request = false;
            self.disconnect_pending = false;
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
        if cmd_id == CommandId::CommControl as u8 {
            Self::process_comm_control(subfn_id, data, display_name, response)
        } else {
            ResponseCode::UnsupportedFeature
        }
    }

    fn process_comm_control(
        subfn_id: u8,
        data: &[u8],
        display_name: &'static str,
        response: &mut Response<'_>,
    ) -> ResponseCode {
        if subfn_id == CommControlSubfn::Discover as u8 {
            codec_v1_0::encode_discover(data, display_name, response)
        } else {
            ResponseCode::UnsupportedFeature
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
