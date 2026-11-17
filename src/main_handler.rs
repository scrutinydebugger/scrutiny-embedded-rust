use crate::build_config;
use crate::config::{Config, RpvReadCallback, RpvWriteCallback};
#[cfg(feature = "datalogging")]
use crate::loop_handler::LoopHandler;
use crate::protocol::codec_v1_0;
use crate::protocol::definitions::{
    CommControlSubfn, CommandId, GetInfoSubfn, MemoryControlSubfn, ResponseCode, CONNECT_MAGIC,
};
#[cfg(feature = "protected_regions")]
use crate::protocol::definitions::MemoryRegionType;
use crate::protocol::CommHandler;
use crate::timebase::Timebase;
use crate::types::{AnyType, RuntimePublishedValue, Timediff, Timestamp};

#[cfg(feature = "protected_regions")]
use crate::types::AddressRange;

#[cfg(feature = "datalogging")]
use crate::datalogging::{DataLoggerCell, DataloggerHost, State as DlState};
#[cfg(feature = "datalogging")]
use crate::ipc::{Loop2MainMsgKind, Main2LoopMsg};
#[cfg(feature = "datalogging")]
use crate::protocol::codec_v1_0::{
    decode_datalogging_configure_request, encode_response_datalogging_get_acquisition_metadata,
    encode_response_datalogging_get_setup, encode_response_datalogging_get_status,
    encode_response_datalogging_read_acquisition,
};
#[cfg(feature = "datalogging")]
use crate::protocol::definitions::DataLogControlSubfn;

// ── Datalogging error type ────────────────────────────────────────────────────

#[cfg(feature = "datalogging")]
/// Error state tracked by the main handler's datalogging subsystem.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DataloggingError {
    None,
    UnexpectedClaim,
    UnexpectedRelease,
}

// ── Datalogging runtime state (main-handler side) ─────────────────────────────

#[cfg(feature = "datalogging")]
struct DataloggingState {
    /// Index into the loops slice of the loop that currently owns the datalogger.
    owner: Option<u8>,
    /// Loop index that should take ownership next (triggers a `TakeOwnership` IPC message).
    pending_new_owner: Option<u8>,
    request_release: bool,
    pending_release: bool,
    request_arm: bool,
    request_disarm: bool,
    reading_in_progress: bool,
    read_rolling_counter: u8,
    read_crc: u32,
    /// Cached state updated via IPC `StatusUpdate` / `DataAcquired` messages.
    cached_state: DlState,
    cached_bytes_to_complete: usize,
    cached_write_counter: usize,
    error: DataloggingError,
}

#[cfg(feature = "datalogging")]
impl DataloggingState {
    const DEFAULT: Self = Self {
        owner: None,
        pending_new_owner: None,
        request_release: false,
        pending_release: false,
        request_arm: false,
        request_disarm: false,
        reading_in_progress: false,
        read_rolling_counter: 0,
        read_crc: 0,
        cached_state: DlState::Idle,
        cached_bytes_to_complete: 0,
        cached_write_counter: 0,
        error: DataloggingError::None,
    };
}

// ── DataloggerHost implementation (inline host for configure / validate) ──────

#[cfg(feature = "datalogging")]
struct RpvHost {
    rpvs: Option<&'static [RuntimePublishedValue]>,
    rpv_read_callback: Option<RpvReadCallback>,
}

#[cfg(feature = "datalogging")]
impl DataloggerHost for RpvHost {
    unsafe fn read_memory(&self, dst: *mut u8, src: *const u8, count: usize) -> bool {
        // Safety: caller guarantees src range is valid.
        core::ptr::copy_nonoverlapping(src, dst, count);
        true
    }

    fn is_rpv_read_configured(&self) -> bool {
        self.rpv_read_callback.is_some() && self.rpvs.is_some_and(|s| !s.is_empty())
    }

    fn rpv_exists(&self, id: u16) -> bool {
        self.rpvs.is_some_and(|s| s.iter().any(|r| r.id == id))
    }

    fn get_rpv(&self, id: u16) -> Option<RuntimePublishedValue> {
        self.rpvs.and_then(|s| s.iter().find(|r| r.id == id).copied())
    }

    fn read_rpv(&self, rpv: RuntimePublishedValue, out: &mut AnyType) -> bool {
        self.rpv_read_callback.is_some_and(|cb| cb(rpv, out))
    }
}

// ── MainHandler ───────────────────────────────────────────────────────────────

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
    memory_write_enable: bool,
    user_command_configured: bool,
    rpvs: Option<&'static [RuntimePublishedValue]>,
    rpv_read_callback: Option<RpvReadCallback>,
    rpv_write_callback: Option<RpvWriteCallback>,
    loops: Option<&'static [LoopHandler]>,
    #[cfg(feature = "protected_regions")]
    forbidden_ranges: Option<&'static [AddressRange]>,
    #[cfg(feature = "protected_regions")]
    readonly_ranges: Option<&'static [AddressRange]>,
    process_again_timestamp: Timestamp,
    process_again_timestamp_taken: bool,
    #[cfg(feature = "datalogging")]
    datalogger_cell: Option<DataLoggerCell>,
    #[cfg(feature = "datalogging")]
    dl: DataloggingState,
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
        let memory_write_enable = config.memory_write_enable;
        let user_command_configured = config.get_user_command_callback().is_some();
        let rpvs = config.get_rpvs_array();
        let rpv_read_callback = config.get_rpv_read_callback();
        let rpv_write_callback = config.get_rpv_write_callback();
        let loops = config.get_loops();
        #[cfg(feature = "protected_regions")]
        let forbidden_ranges = config.forbidden_ranges();
        #[cfg(feature = "protected_regions")]
        let readonly_ranges = config.readonly_ranges();
        #[cfg(feature = "datalogging")]
        let datalogger_cell = {
            let has_dl_loop = loops.is_some_and(|l| l.iter().any(LoopHandler::datalogging_allowed));
            if let Some((buf, len, cb)) = config.take_datalogger_buf() {
                if has_dl_loop {
                    // Safety: caller guarantees buf is valid for len bytes and outlives this handler.
                    Some(unsafe { DataLoggerCell::new(buf, len, cb) })
                } else {
                    None
                }
            } else {
                None
            }
        };

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
            memory_write_enable,
            user_command_configured,
            rpvs,
            rpv_read_callback,
            rpv_write_callback,
            loops,
            #[cfg(feature = "protected_regions")]
            forbidden_ranges,
            #[cfg(feature = "protected_regions")]
            readonly_ranges,
            process_again_timestamp: 0,
            process_again_timestamp_taken: false,
            #[cfg(feature = "datalogging")]
            datalogger_cell,
            #[cfg(feature = "datalogging")]
            dl: DataloggingState::DEFAULT,
        })
    }

    /// Must be called as fast as possible. `timestep_100ns` is the elapsed time since the last call.
    pub fn process(&mut self, timestep_100ns: Timediff) {
        if !self.enabled {
            self.processing_request = false;
            self.disconnect_pending = false;
            self.process_again_timestamp_taken = false;
            self.comm.reset(&self.timebase);
            #[cfg(feature = "datalogging")]
            if let Some(ref cell) = self.datalogger_cell {
                // Safety: no loop owns the datalogger when handler is disabled.
                unsafe { cell.get() }.reset();
            }
            return;
        }

        self.timebase.step(timestep_100ns);
        self.comm.process(&self.timebase);

        self.process_loops();
        #[cfg(feature = "datalogging")]
        self.process_datalogging_logic();

        if self.comm.request_received() && !self.processing_request {
            let Some((cmd_id, subfn_id, data_len)) = self.comm.get_request() else {
                self.check_finished_sending();
                return;
            };

            let data_len_usize = usize::from(data_len);
            if data_len_usize > self.rx_buf.len() {
                self.enabled = false;
                return;
            }

            self.comm.active_response_mut().reset();

            let code = self.dispatch(cmd_id, subfn_id, data_len_usize);

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

        self.process_loops();
        #[cfg(feature = "datalogging")]
        self.process_datalogging_logic();

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

    // ── Loop / datalogging IPC pumps ──────────────────────────────────────────

    fn process_loops(&mut self) {
        let Some(loops) = self.loops else { return };
        for (i, loop_handler) in loops.iter().enumerate() {
            #[cfg(feature = "datalogging")]
            if let Some(msg) = loop_handler.loop2main().pop() {
                self.handle_loop2main_msg(u8::try_from(i).unwrap_or(u8::MAX), msg);
            }
            #[cfg(not(feature = "datalogging"))]
            let _ = loop_handler;
        }
    }

    #[cfg(feature = "datalogging")]
    fn handle_loop2main_msg(&mut self, loop_idx: u8, msg: crate::ipc::Loop2MainMsg) {
        match msg.kind {
            Loop2MainMsgKind::OwnershipTaken => {
                if self.dl.owner.is_some() {
                    self.dl.error = DataloggingError::UnexpectedClaim;
                }
                self.dl.owner = Some(loop_idx);
            }
            Loop2MainMsgKind::OwnershipReleased => {
                if self.dl.owner != Some(loop_idx) {
                    self.dl.error = DataloggingError::UnexpectedRelease;
                }
                self.dl.owner = None;
                self.dl.pending_release = false;
                if let Some(ref cell) = self.datalogger_cell {
                    // Safety: no loop owns the datalogger after release.
                    unsafe { cell.get() }.reset();
                }
            }
            Loop2MainMsgKind::DataAcquired => {
                self.dl.cached_state = DlState::AcquisitionCompleted;
            }
            Loop2MainMsgKind::StatusUpdate => {
                self.dl.cached_state = msg.state;
                self.dl.cached_bytes_to_complete = msg.bytes_to_complete;
                self.dl.cached_write_counter = msg.write_counter;
                if msg.state != DlState::AcquisitionCompleted {
                    self.dl.reading_in_progress = false;
                }
            }
        }
    }

    #[cfg(feature = "datalogging")]
    fn process_datalogging_logic(&mut self) {
        if self.dl.error != DataloggingError::None {
            return;
        }

        let Some(loops) = self.loops else { return };

        if self.dl.owner.is_none() {
            // When no loop owns the datalogger we can read state directly.
            if let Some(ref cell) = self.datalogger_cell {
                // Safety: no loop owns the datalogger.
                self.dl.cached_state = unsafe { cell.get() }.get_state();
            }

            if let Some(new_owner_idx) = self.dl.pending_new_owner {
                let loop_handler = &loops[new_owner_idx as usize];
                if !loop_handler.main2loop().has_content() {
                    if let Some(ref cell) = self.datalogger_cell {
                        let cell_addr = core::ptr::from_ref::<DataLoggerCell>(cell) as usize;
                        loop_handler.main2loop().send(Main2LoopMsg::TakeOwnership {
                            cell_addr,
                            rpvs:        self.rpvs,
                            rpv_read_cb: self.rpv_read_callback,
                        });
                        self.dl.pending_new_owner = None;
                    }
                }
            }

            // Can't arm/disarm without an owner.
            self.dl.request_arm = false;
            self.dl.request_disarm = false;
        } else if let Some(owner_idx) = self.dl.owner {
            let loop_handler = &loops[owner_idx as usize];
            if !loop_handler.main2loop().has_content() {
                if self.dl.request_release {
                    loop_handler.main2loop().send(Main2LoopMsg::ReleaseOwnership);
                    self.dl.request_release = false;
                    self.dl.pending_release = true;
                } else if self.dl.request_arm {
                    loop_handler.main2loop().send(Main2LoopMsg::ArmTrigger);
                    self.dl.request_arm = false;
                } else if self.dl.request_disarm {
                    loop_handler.main2loop().send(Main2LoopMsg::DisarmTrigger);
                    self.dl.request_disarm = false;
                }
            }
        }
    }

    fn dispatch(&mut self, cmd_id: u8, subfn_id: u8, data_len: usize) -> ResponseCode {
        let data = &self.rx_buf[..data_len];
        #[cfg(feature = "datalogging")]
        let dl_enabled = self.datalogger_cell.is_some();
        #[cfg(not(feature = "datalogging"))]
        let dl_enabled = false;
        match CommandId::try_from(cmd_id) {
            Ok(CommandId::GetInfo) => Self::process_get_info(
                &mut self.comm,
                &mut *self.tx_buf,
                self.memory_write_enable,
                self.user_command_configured,
                self.rpvs,
                self.loops,
                #[cfg(feature = "protected_regions")]
                self.forbidden_ranges,
                #[cfg(feature = "protected_regions")]
                self.readonly_ranges,
                dl_enabled,
                subfn_id,
                data,
            ),
            Ok(CommandId::CommControl) => Self::process_comm_control(
                &mut self.comm,
                &mut *self.tx_buf,
                self.display_name,
                self.max_bitrate,
                &self.timebase,
                &mut self.disconnect_pending,
                subfn_id,
                data,
            ),
            Ok(CommandId::MemoryControl) => Self::process_memory_control(
                &mut self.comm,
                &mut *self.tx_buf,
                self.memory_write_enable,
                self.rpvs,
                self.rpv_read_callback,
                self.rpv_write_callback,
                #[cfg(feature = "protected_regions")]
                self.forbidden_ranges,
                #[cfg(feature = "protected_regions")]
                self.readonly_ranges,
                subfn_id,
                data,
            ),
            Ok(CommandId::UserCommand) | Err(()) => ResponseCode::UnsupportedFeature,
            #[cfg(not(feature = "datalogging"))]
            Ok(CommandId::DataLogControl) => ResponseCode::UnsupportedFeature,
            #[cfg(feature = "datalogging")]
            Ok(CommandId::DataLogControl) => {
                let Some(cell) = self.datalogger_cell.as_ref() else {
                    return ResponseCode::UnsupportedFeature;
                };
                Self::process_datalog_control(
                    &mut self.comm,
                    &mut *self.tx_buf,
                    self.loops,
                    self.rpvs,
                    self.rpv_read_callback,
                    cell,
                    &mut self.dl,
                    #[cfg(feature = "protected_regions")]
                    self.forbidden_ranges,
                    subfn_id,
                    data,
                )
            }
        }
    }

    // ===== GetInfo =====

    #[allow(clippy::too_many_arguments)]
    fn process_get_info(
        comm: &mut CommHandler,
        tx_buf: &mut [u8],
        memory_write_enable: bool,
        user_command_configured: bool,
        rpvs: Option<&'static [RuntimePublishedValue]>,
        loops: Option<&'static [LoopHandler]>,
        #[cfg(feature = "protected_regions")] forbidden_ranges: Option<&'static [AddressRange]>,
        #[cfg(feature = "protected_regions")] readonly_ranges: Option<&'static [AddressRange]>,
        datalogging_enabled: bool,
        subfn_id: u8,
        data: &[u8],
    ) -> ResponseCode {
        match GetInfoSubfn::try_from(subfn_id) {
            Ok(GetInfoSubfn::GetProtocolVersion) => {
                match codec_v1_0::encode_response_get_info_protocol_version(tx_buf) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(GetInfoSubfn::GetSoftwareId) => match codec_v1_0::encode_response_get_info_software_id(tx_buf) {
                Ok(len) => {
                    comm.active_response_mut().data_length = len;
                    ResponseCode::Ok
                }
                Err(code) => code,
            },

            Ok(GetInfoSubfn::GetSupportedFeatures) => {
                #[cfg(feature = "support_64bits")]
                let support_64bits = true;
                #[cfg(not(feature = "support_64bits"))]
                let support_64bits = false;

                #[cfg(feature = "datalogging")]
                let dl = datalogging_enabled;
                #[cfg(not(feature = "datalogging"))]
                let dl = false;

                match codec_v1_0::encode_response_get_info_supported_features(
                    memory_write_enable,
                    dl,
                    user_command_configured,
                    support_64bits,
                    tx_buf,
                ) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(GetInfoSubfn::GetSpecialMemoryRegionCount) => {
                #[cfg(feature = "protected_regions")]
                let (readonly_count, forbidden_count) = (
                    readonly_ranges.map_or(0u8, |arr| u8::try_from(arr.len()).unwrap_or(u8::MAX)),
                    forbidden_ranges.map_or(0u8, |arr| u8::try_from(arr.len()).unwrap_or(u8::MAX)),
                );
                #[cfg(not(feature = "protected_regions"))]
                let (readonly_count, forbidden_count) = (0u8, 0u8);

                match codec_v1_0::encode_response_get_info_special_memory_region_count(
                    readonly_count,
                    forbidden_count,
                    tx_buf,
                ) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(GetInfoSubfn::GetSpecialMemoryLocation) => {
                #[cfg(feature = "protected_regions")]
                {
                    if data.len() != 2 {
                        return ResponseCode::InvalidRequest;
                    }
                    let region_type_byte = data[0];
                    let region_index = data[1];

                    let Ok(region_type) = MemoryRegionType::try_from(region_type_byte) else {
                        return ResponseCode::InvalidRequest;
                    };

                    let (start, end) = match region_type {
                        MemoryRegionType::ReadOnly => {
                            let Some(ranges) = readonly_ranges else {
                                return ResponseCode::FailureToProceed;
                            };
                            let idx = usize::from(region_index);
                            if idx >= ranges.len() {
                                return ResponseCode::FailureToProceed;
                            }
                            (ranges[idx].start, ranges[idx].end)
                        }
                        MemoryRegionType::Forbidden => {
                            let Some(ranges) = forbidden_ranges else {
                                return ResponseCode::FailureToProceed;
                            };
                            let idx = usize::from(region_index);
                            if idx >= ranges.len() {
                                return ResponseCode::FailureToProceed;
                            }
                            (ranges[idx].start, ranges[idx].end)
                        }
                    };

                    match codec_v1_0::encode_response_get_info_special_memory_region_location(
                        region_type_byte,
                        region_index,
                        start,
                        end,
                        tx_buf,
                    ) {
                        Ok(len) => {
                            comm.active_response_mut().data_length = len;
                            ResponseCode::Ok
                        }
                        Err(code) => code,
                    }
                }
                #[cfg(not(feature = "protected_regions"))]
                ResponseCode::UnsupportedFeature
            }

            Ok(GetInfoSubfn::GetRuntimePublishedValuesCount) => {
                let count = rpvs.map_or(0u16, |s| u16::try_from(s.len()).unwrap_or(u16::MAX));
                match codec_v1_0::encode_response_get_info_rpv_count(count, tx_buf) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(GetInfoSubfn::GetRuntimePublishedValuesDefinition) => {
                if data.len() != 4 {
                    return ResponseCode::InvalidRequest;
                }
                let start_index = u16::from_be_bytes([data[0], data[1]]);
                let count = u16::from_be_bytes([data[2], data[3]]);

                let Some(rpv_slice) = rpvs else {
                    return ResponseCode::FailureToProceed;
                };
                let rpv_count = u16::try_from(rpv_slice.len()).unwrap_or(u16::MAX);

                if start_index >= rpv_count {
                    return ResponseCode::FailureToProceed;
                }
                if start_index.saturating_add(count) > rpv_count {
                    return ResponseCode::FailureToProceed;
                }

                match codec_v1_0::encode_response_get_info_rpv_definitions(rpv_slice, start_index, count, tx_buf) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(GetInfoSubfn::GetLoopCount) => {
                let count = loops.map_or(0u8, |s| u8::try_from(s.len()).unwrap_or(u8::MAX));
                match codec_v1_0::encode_response_get_info_loop_count(count, tx_buf) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(GetInfoSubfn::GetLoopDefinition) => {
                if data.len() != 1 {
                    return ResponseCode::InvalidRequest;
                }
                let loop_id = data[0];

                let Some(loop_slice) = loops else {
                    return ResponseCode::FailureToProceed;
                };
                if usize::from(loop_id) >= loop_slice.len() {
                    return ResponseCode::FailureToProceed;
                }

                match codec_v1_0::encode_response_get_info_loop_definition(
                    loop_id,
                    &loop_slice[usize::from(loop_id)],
                    tx_buf,
                ) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Err(()) => ResponseCode::UnsupportedFeature,
        }
    }

    // ===== CommControl =====

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
                    8,
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

    // ===== MemoryControl =====

    #[allow(clippy::too_many_arguments)]
    fn process_memory_control(
        comm: &mut CommHandler,
        tx_buf: &mut [u8],
        memory_write_enable: bool,
        rpvs: Option<&'static [RuntimePublishedValue]>,
        rpv_read_callback: Option<RpvReadCallback>,
        rpv_write_callback: Option<RpvWriteCallback>,
        #[cfg(feature = "protected_regions")] forbidden_ranges: Option<&'static [crate::types::AddressRange]>,
        #[cfg(feature = "protected_regions")] readonly_ranges: Option<&'static [crate::types::AddressRange]>,
        subfn_id: u8,
        data: &[u8],
    ) -> ResponseCode {
        match MemoryControlSubfn::try_from(subfn_id) {
            Ok(MemoryControlSubfn::Read) => {
                let result = unsafe {
                    codec_v1_0::encode_response_memory_control_read(
                        data,
                        tx_buf,
                        #[cfg(feature = "protected_regions")]
                        forbidden_ranges,
                    )
                };
                match result {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(MemoryControlSubfn::Write) => {
                let result = unsafe {
                    codec_v1_0::encode_response_memory_control_write(
                        data,
                        tx_buf,
                        memory_write_enable,
                        #[cfg(feature = "protected_regions")]
                        forbidden_ranges,
                        #[cfg(feature = "protected_regions")]
                        readonly_ranges,
                    )
                };
                match result {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(MemoryControlSubfn::WriteMasked) => {
                let result = unsafe {
                    codec_v1_0::encode_response_memory_control_write_masked(
                        data,
                        tx_buf,
                        memory_write_enable,
                        #[cfg(feature = "protected_regions")]
                        forbidden_ranges,
                        #[cfg(feature = "protected_regions")]
                        readonly_ranges,
                    )
                };
                match result {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(MemoryControlSubfn::ReadRpv) => {
                match codec_v1_0::encode_response_memory_control_read_rpv(data, rpvs, rpv_read_callback, tx_buf) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Ok(MemoryControlSubfn::WriteRpv) => {
                match codec_v1_0::encode_response_memory_control_write_rpv(data, rpvs, rpv_write_callback, tx_buf) {
                    Ok(len) => {
                        comm.active_response_mut().data_length = len;
                        ResponseCode::Ok
                    }
                    Err(code) => code,
                }
            }

            Err(()) => ResponseCode::UnsupportedFeature,
        }
    }

    // ===== DataLogControl =====

    #[cfg(feature = "datalogging")]
    #[allow(clippy::too_many_arguments)]
    fn process_datalog_control(
        comm: &mut CommHandler,
        tx_buf: &mut [u8],
        loops: Option<&'static [LoopHandler]>,
        rpvs: Option<&'static [RuntimePublishedValue]>,
        rpv_read_callback: Option<RpvReadCallback>,
        cell: &DataLoggerCell,
        dl: &mut DataloggingState,
        #[cfg(feature = "protected_regions")] forbidden_ranges: Option<&'static [AddressRange]>,
        subfn_id: u8,
        data: &[u8],
    ) -> ResponseCode {
        let host = RpvHost { rpvs, rpv_read_callback };

        let code = match DataLogControlSubfn::try_from(subfn_id) {
            Err(()) => return ResponseCode::UnsupportedFeature,

            Ok(DataLogControlSubfn::GetSetup) => {
                let buf_size = u32::try_from(cell.buffer_size()).unwrap_or(u32::MAX);
                let encoding = crate::datalogging::RawFormatEncoder::get_encoding();
                let max_signals = u8::try_from(crate::build_config::DATALOGGING_MAX_SIGNAL).unwrap_or(u8::MAX);
                match encode_response_datalogging_get_setup(buf_size, encoding, max_signals, tx_buf) {
                    Ok(len) => { comm.active_response_mut().data_length = len; ResponseCode::Ok }
                    Err(e) => e,
                }
            }

            Ok(DataLogControlSubfn::ConfigureDatalog) => {
                dl.reading_in_progress = false;

                if dl.owner.is_some() {
                    if !dl.pending_release {
                        dl.request_release = true;
                    }
                    return ResponseCode::ProcessAgain;
                }

                // Safety: no loop owns the datalogger right now.
                let dl_obj = unsafe { cell.get() };

                let parse_result = decode_datalogging_configure_request(data, dl_obj.config_mut());
                let (loop_id, config_id) = match parse_result {
                    Ok(ids) => ids,
                    Err(e) => { dl_obj.reset(); return e; }
                };

                let Some(loop_slice) = loops else { dl_obj.reset(); return ResponseCode::FailureToProceed; };
                if loop_id as usize >= loop_slice.len() { dl_obj.reset(); return ResponseCode::FailureToProceed; }
                if !loop_slice[loop_id as usize].datalogging_allowed() { dl_obj.reset(); return ResponseCode::Forbidden; }

                // Validate forbidden regions for Var/VarBit operands and Memory items.
                #[cfg(feature = "protected_regions")]
                if let Some(forbidden) = forbidden_ranges {
                    use crate::datalogging::types::{Operand, LoggableItem};
                    use crate::tools::TypeSize;
                    let config = dl_obj.config_ref();
                    let mut region_err = false;
                    'outer: for i in 0..config.trigger.operand_count as usize {
                        let touches = match config.trigger.operands[i] {
                            Operand::Var { addr, datatype } | Operand::VarBit { addr, datatype, .. } =>
                                codec_v1_0::region_overlaps(addr as usize, datatype.type_size() as usize, forbidden),
                            _ => false,
                        };
                        if touches { region_err = true; break 'outer; }
                    }
                    if !region_err {
                        for i in 0..config.items_count as usize {
                            if let LoggableItem::Memory { address, size } = config.items_to_log[i] {
                                if codec_v1_0::region_overlaps(address as usize, size as usize, forbidden) {
                                    region_err = true;
                                    break;
                                }
                            }
                        }
                    }
                    if region_err { dl_obj.reset(); return ResponseCode::Forbidden; }
                }

                dl_obj.configure(&Timebase::new(), config_id, &host);

                if dl_obj.config_valid() {
                    dl.pending_new_owner = Some(loop_id);
                    ResponseCode::Ok
                } else {
                    dl_obj.reset();
                    ResponseCode::InvalidRequest
                }
            }

            Ok(DataLogControlSubfn::ArmTrigger) => {
                if dl.owner.is_none() || dl.pending_release {
                    return ResponseCode::FailureToProceed;
                }
                dl.request_arm = true;
                ResponseCode::Ok
            }

            Ok(DataLogControlSubfn::DisarmTrigger) => {
                if dl.owner.is_none() || dl.pending_release {
                    return ResponseCode::FailureToProceed;
                }
                dl.request_disarm = true;
                ResponseCode::Ok
            }

            Ok(DataLogControlSubfn::GetStatus) => {
                let state = dl.cached_state as u8;
                let bytes = u32::try_from(dl.cached_bytes_to_complete).unwrap_or(u32::MAX);
                let counter = u32::try_from(dl.cached_write_counter).unwrap_or(u32::MAX);
                match encode_response_datalogging_get_status(state, bytes, counter, tx_buf) {
                    Ok(len) => { comm.active_response_mut().data_length = len; ResponseCode::Ok }
                    Err(e) => e,
                }
            }

            Ok(DataLogControlSubfn::GetAcquisitionMetadata) => {
                if dl.owner.is_none() || dl.cached_state != DlState::AcquisitionCompleted {
                    return ResponseCode::FailureToProceed;
                }
                // Safety: owner is Some and state is AcquisitionCompleted — loop is idle, safe to read.
                let dl_obj = unsafe { cell.get() };
                let acq_id = dl_obj.get_acquisition_id();
                let config_id = dl_obj.get_config_id();
                let entry_count = u32::try_from(dl_obj.get_encoder().get_entry_count()).unwrap_or(u32::MAX);
                let data_size = u32::try_from(dl_obj.get_encoder().reader_total_size()).unwrap_or(u32::MAX);
                let after_trigger = u32::try_from(dl_obj.log_points_after_trigger()).unwrap_or(u32::MAX);
                match encode_response_datalogging_get_acquisition_metadata(
                    acq_id, config_id, entry_count, data_size, after_trigger, tx_buf,
                ) {
                    Ok(len) => { comm.active_response_mut().data_length = len; ResponseCode::Ok }
                    Err(e) => e,
                }
            }

            Ok(DataLogControlSubfn::ReadAcquisition) => {
                if dl.owner.is_none() {
                    return ResponseCode::FailureToProceed;
                }
                if dl.cached_state != DlState::AcquisitionCompleted {
                    dl.reading_in_progress = false;
                    return ResponseCode::FailureToProceed;
                }
                // Safety: AcquisitionCompleted and we own the slot — loop is idle.
                let dl_obj = unsafe { cell.get() };
                if !dl.reading_in_progress {
                    dl_obj.get_encoder_mut().reset_reader();
                    dl.reading_in_progress = true;
                    dl.read_rolling_counter = 0;
                    dl.read_crc = 0;
                }
                let acq_id = dl_obj.get_acquisition_id();
                match encode_response_datalogging_read_acquisition(
                    dl_obj.get_encoder_mut(),
                    dl.read_rolling_counter,
                    acq_id,
                    &mut dl.read_crc,
                    tx_buf,
                ) {
                    Ok((len, finished)) => {
                        comm.active_response_mut().data_length = len;
                        dl.read_rolling_counter = dl.read_rolling_counter.wrapping_add(1);
                        if finished {
                            dl.reading_in_progress = false;
                        }
                        ResponseCode::Ok
                    }
                    Err(e) => {
                        dl.reading_in_progress = false;
                        e
                    }
                }
            }

            Ok(DataLogControlSubfn::ResetDatalogger) => {
                if dl.owner.is_some() {
                    if !dl.pending_release {
                        dl.request_release = true;
                    }
                    return ResponseCode::ProcessAgain;
                }
                // Safety: no loop owns the datalogger.
                unsafe { cell.get() }.reset();
                ResponseCode::Ok
            }
        };

        // On configure failure (not ProcessAgain), the datalogger has already been reset above.
        let _ = host; // suppress unused warning
        code
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
