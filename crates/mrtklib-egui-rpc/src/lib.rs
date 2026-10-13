//! Client of the `mrtk run` JSON-RPC interface, protocol 0.1 (MRTKLIB `docs/rpc/`).
//!
//! Two layers, both blocking and free of any GUI toolkit:
//!
//! - [`Client`]: one WebSocket connection, one request at a time, typed results.
//! - [`Session`]: a worker thread that connects, synchronizes, polls events and keeps a
//!   [`Model`] that a GUI reads on every frame.
//!
//! Starting `mrtk run`, the token and the ready line belong to the process supervisor, not
//! to this crate. The token is used for the handshake only and never appears in `Debug`,
//! `Display` or errors.

mod client;
mod session;
mod types;

pub use client::{Client, Error, RpcError};
pub use session::{
    Command, Connection, Model, RequestError, RequestOutcome, Session, SessionOptions,
};
pub use types::{
    Accepted, Authentication, Covariance, CovarianceEcef, CovarianceEnu, Ecef, Event, EventData,
    EventPage, Geoid, GpsTime, LastError, Lifecycle, Llh, LogMessage, Position, Snapshot, Solution,
    SolutionStatus, State, Status, StopReason, Version,
};
