pub mod comm_handler;
pub mod definitions;

pub use comm_handler::CommHandler;
pub use definitions::{
    CommControlSubfn, CommandId, DataLogControlSubfn, GetInfoSubfn, MemoryControlSubfn, Request, Response,
    ResponseCode, RxError, TxError, DISCOVER_MAGIC, HEARTBEAT_TIMEOUT_100NS, MAXIMUM_RX_BUFFER_SIZE,
    MAXIMUM_TX_BUFFER_SIZE, MINIMUM_RX_BUFFER_SIZE, MINIMUM_TX_BUFFER_SIZE, RX_TIMEOUT_100NS,
};
