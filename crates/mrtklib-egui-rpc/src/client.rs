//! One blocking connection to `mrtk run`, one request at a time (MRTKLIB `docs/rpc/` §3, §10).

use std::fmt;
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tungstenite::client::ClientRequestBuilder;
use tungstenite::error::ProtocolError;
use tungstenite::handshake::HandshakeError;
use tungstenite::http::Uri;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::{Message, WebSocket};

use crate::types::{Accepted, EventPage, Snapshot, State, Version};

/// The WebSocket subprotocol that the server selects (§3).
const PROTOCOL: &str = "mrtk-rpc";
/// The token travels as a second offered subprotocol, `mrtk-token.<token>` (§3).
const TOKEN_ENTRY_PREFIX: &str = "mrtk-token.";
/// The only protocol version this client accepts. During 0.x any change may be
/// incompatible (§4), so the client takes exactly the version it was built for.
const SUPPORTED_PROTOCOL_VERSION: &str = "0.1";
/// WebSocket close status of a server that exits (RFC 6455 "going away"; §3).
const CLOSE_GOING_AWAY: u16 = 1001;

/// An error of the connection or of a request.
///
/// After any error except [`Error::Rpc`] the connection is not usable any more: drop it.
/// No value contains the token.
#[derive(Debug)]
pub enum Error {
    /// A network error, for example connection refused.
    Io(io::Error),
    /// The server did not answer within the timeout.
    Timeout,
    /// The server refused the WebSocket handshake with this HTTP status (not 401 or 403).
    Handshake { status: u16 },
    /// HTTP 401: the token is missing or wrong (§3).
    Unauthorized,
    /// HTTP 403: the origin is not allowed (§3).
    Forbidden,
    /// The server speaks a protocol version that this client does not accept.
    UnsupportedVersion { found: String },
    /// The server answered a request with a JSON-RPC error (§10).
    Rpc(RpcError),
    /// The server sent a close frame. See [`Error::is_server_exit`].
    Closed { code: Option<u16> },
    /// The connection ended without a close frame.
    ConnectionLost,
    /// The server broke the protocol: malformed JSON, a result that does not decode, an
    /// unexpected response, a wrong WebSocket subprotocol.
    Protocol(String),
    /// The URL, the token or the timeout passed to [`Client::connect`] is not usable.
    InvalidArgument(String),
}

impl Error {
    /// The server closed the connection with status 1001 ("going away"): the process
    /// exits, for example after `shutdown` (§3). This is not a failure.
    pub fn is_server_exit(&self) -> bool {
        matches!(
            self,
            Error::Closed {
                code: Some(CLOSE_GOING_AWAY)
            }
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "network error: {e}"),
            Error::Timeout => write!(f, "the server did not answer in time"),
            Error::Handshake { status } => {
                write!(
                    f,
                    "the server refused the connection (HTTP status {status})"
                )
            }
            Error::Unauthorized => write!(f, "the server refused the token (HTTP status 401)"),
            Error::Forbidden => write!(f, "the server refused the origin (HTTP status 403)"),
            Error::UnsupportedVersion { found } => write!(
                f,
                "the server speaks protocol {found}, this client supports {SUPPORTED_PROTOCOL_VERSION}"
            ),
            Error::Rpc(e) => e.fmt(f),
            Error::Closed {
                code: Some(CLOSE_GOING_AWAY),
            } => write!(f, "server exited"),
            Error::Closed { code: Some(code) } => {
                write!(f, "the server closed the connection (status {code})")
            }
            Error::Closed { code: None } => write!(f, "the server closed the connection"),
            Error::ConnectionLost => write!(f, "the connection was lost"),
            Error::Protocol(text) => write!(f, "protocol error: {text}"),
            Error::InvalidArgument(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// A JSON-RPC error response (§10). Branch on `reason`, never on `message`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    /// English text for display. Not part of the contract.
    pub message: String,
    /// `data.reason`: the stable name (`busy`, `alreadyRunning`, `shuttingDown`,
    /// `invalidParams`), absent for the errors that JSON-RPC itself defines.
    pub reason: Option<String>,
    /// `data.state`: with `busy`, the state that caused the rejection.
    pub state: Option<State>,
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {}", self.message, self.code)?;
        if let Some(reason) = &self.reason {
            write!(f, ", reason {reason}")?;
        }
        write!(f, ")")
    }
}

#[derive(Deserialize)]
struct WireResponse {
    #[serde(default)]
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    result: Option<Value>,
    error: Option<WireError>,
}

#[derive(Deserialize)]
struct WireError {
    code: i64,
    message: String,
    data: Option<WireErrorData>,
}

#[derive(Deserialize)]
struct WireErrorData {
    reason: Option<String>,
    state: Option<State>,
}

/// A blocking connection to `mrtk run` (or the mock). It sends one request at a time and
/// waits for its response.
pub struct Client {
    ws: WebSocket<TcpStream>,
    next_id: u64,
    timeout: Duration,
}

// The default `Debug` of the WebSocket would print buffers; keep the output small and
// free of anything that came from the handshake.
impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("next_id", &self.next_id)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Connect to `url` (`ws://HOST:PORT/rpc`, the `url` of the ready line), present the
    /// token and check the protocol version.
    ///
    /// `token` is `None` only for a server started with `--rpc-no-auth`. `timeout` bounds
    /// the TCP connection, every write, and each later request from its send to its
    /// response (frames that are not the response, such as pings, do not extend it). The
    /// protocol answers lifecycle requests when they take effect, not when the transition
    /// ends (§5). Only `ws://` is supported: protocol 0.x has no TLS (§3). The token is
    /// used for the handshake only and not kept.
    pub fn connect(
        url: &str,
        token: Option<&str>,
        timeout: Duration,
    ) -> Result<(Client, Version), Error> {
        if timeout.is_zero() {
            return Err(Error::InvalidArgument(
                "the timeout must not be zero".into(),
            ));
        }
        if let Some(token) = token {
            check_token(token)?;
        }
        let uri: Uri = url
            .parse()
            .map_err(|_| Error::InvalidArgument(format!("not a valid URL: {url}")))?;
        let stream = open_stream(&uri, timeout)?;

        let mut request = ClientRequestBuilder::new(uri).with_sub_protocol(PROTOCOL);
        if let Some(token) = token {
            request = request.with_sub_protocol(format!("{TOKEN_ENTRY_PREFIX}{token}"));
        }
        let (ws, response) =
            tungstenite::client::client(request, stream).map_err(handshake_error)?;
        // The server must select the protocol and never echo the token entry (§3). The
        // value is not put into the error: it could be the token.
        let selected = response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .map(|value| value.as_bytes());
        if selected != Some(PROTOCOL.as_bytes()) {
            return Err(Error::Protocol(format!(
                "the server did not select the {PROTOCOL} subprotocol"
            )));
        }

        let mut client = Client {
            ws,
            next_id: 0,
            timeout,
        };
        let version: Version = client.call("getVersion", None)?;
        if let Err(error) = check_protocol_version(&version.protocol_version) {
            // Leave politely; the version error is what the caller needs to see.
            let _ = client.close();
            return Err(error);
        }
        Ok((client, version))
    }

    /// `getVersion` (the result of [`Client::connect`] is the same).
    pub fn get_version(&mut self) -> Result<Version, Error> {
        self.call("getVersion", None)
    }

    /// `getStatus` (§6).
    pub fn get_status(&mut self) -> Result<Snapshot, Error> {
        self.call("getStatus", None)
    }

    /// `sync` (§7): the state together with the boundary `seq`; poll `getEvents` from there.
    pub fn sync(&mut self) -> Result<Snapshot, Error> {
        self.call("sync", None)
    }

    /// `getEvents` (§7): the events after `after`, at most `limit` (the server applies its
    /// own maximum too).
    pub fn get_events(&mut self, after: u64, limit: Option<u32>) -> Result<EventPage, Error> {
        let mut params = json!({ "after": after });
        if let Some(limit) = limit {
            params["limit"] = json!(limit);
        }
        self.call("getEvents", Some(params))
    }

    /// `start` (§5). Answers when the request takes effect; follow the outcome in the
    /// status events.
    pub fn start(&mut self) -> Result<Accepted, Error> {
        self.call("start", None)
    }

    /// `stop` (§5). Succeeds without effect when already stopped.
    pub fn stop(&mut self) -> Result<Accepted, Error> {
        self.call("stop", None)
    }

    /// `restart` (§5).
    pub fn restart(&mut self) -> Result<Accepted, Error> {
        self.call("restart", None)
    }

    /// `shutdown` (§5): the process ends afterwards and closes the connection with
    /// status 1001 (see [`Error::is_server_exit`]).
    pub fn shutdown(&mut self) -> Result<Accepted, Error> {
        self.call("shutdown", None)
    }

    /// Send a normal close (status 1000) and wait for the end of the connection, at most
    /// the connect timeout. A server that drops the connection right after the close
    /// frame, instead of answering it, counts as closed.
    pub fn close(mut self) -> Result<(), Error> {
        let frame = CloseFrame {
            code: CloseCode::Normal,
            reason: "".into(),
        };
        match self.ws.close(Some(frame)) {
            Ok(()) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(e) => return Err(from_websocket(e)),
        }
        let deadline = Instant::now() + self.timeout;
        loop {
            self.limit_read(deadline)?;
            match self.ws.read() {
                Ok(_) => {} // a frame that was already on its way
                Err(
                    tungstenite::Error::ConnectionClosed
                    | tungstenite::Error::AlreadyClosed
                    | tungstenite::Error::Protocol(ProtocolError::ResetWithoutClosingHandshake),
                ) => return Ok(()),
                Err(e) => return Err(from_websocket(e)),
            }
        }
    }

    /// Let the next read wait only until `deadline`, so that frames which are not the
    /// awaited one (pings) cannot keep a request waiting longer than its timeout. A peer
    /// that trickles fragments of one message inside a single read is not covered.
    fn limit_read(&self, deadline: Instant) -> Result<(), Error> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::Timeout);
        }
        self.ws
            .get_ref()
            .set_read_timeout(Some(remaining))
            .map_err(from_io)
    }

    /// Send a request and read frames until its response arrives, at most the timeout
    /// from the send. Parameters go by name; methods without parameters omit them (§3).
    fn call<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<T, Error> {
        self.next_id += 1;
        let id = self.next_id;
        let mut request = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(params) = params {
            request["params"] = params;
        }
        let deadline = Instant::now() + self.timeout;
        self.ws
            .send(Message::text(request.to_string()))
            .map_err(from_websocket)?;
        let response = self.read_response(deadline)?;
        decode_response(response, id, method)
    }

    fn read_response(&mut self, deadline: Instant) -> Result<WireResponse, Error> {
        loop {
            self.limit_read(deadline)?;
            match self.ws.read().map_err(from_websocket)? {
                Message::Text(text) => {
                    return serde_json::from_str(text.as_str())
                        .map_err(|e| Error::Protocol(format!("malformed response: {e}")));
                }
                // tungstenite answers a ping on the next read or write.
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                Message::Binary(_) => {
                    return Err(Error::Protocol("the server sent a binary frame".into()));
                }
                Message::Close(frame) => {
                    // Answer the close frame; a failure here changes nothing for the caller.
                    let _ = self.ws.flush();
                    return Err(Error::Closed {
                        code: frame.map(|frame| u16::from(frame.code)),
                    });
                }
            }
        }
    }
}

fn decode_response<T: DeserializeOwned>(
    response: WireResponse,
    id: u64,
    method: &str,
) -> Result<T, Error> {
    if response.jsonrpc != "2.0" {
        return Err(Error::Protocol("the response is not JSON-RPC 2.0".into()));
    }
    if response.id.as_u64() != Some(id) {
        // The server answers a request it cannot parse or that is invalid with a null id.
        return Err(match response.error {
            Some(error) if response.id.is_null() => Error::Protocol(format!(
                "the server rejected the {method} request: {} (code {})",
                error.message, error.code
            )),
            _ => Error::Protocol(format!(
                "the response to {method} has an unexpected id (one request is outstanding)"
            )),
        });
    }
    if let Some(error) = response.error {
        let (reason, state) = error.data.map_or((None, None), |d| (d.reason, d.state));
        return Err(Error::Rpc(RpcError {
            code: error.code,
            message: error.message,
            reason,
            state,
        }));
    }
    let Some(result) = response.result else {
        return Err(Error::Protocol(format!(
            "the response to {method} has neither result nor error"
        )));
    };
    serde_json::from_value(result)
        .map_err(|e| Error::Protocol(format!("the result of {method} does not decode: {e}")))
}

/// Only `ws://`; the host may be a name, an IPv4 address or `[IPv6]`.
fn open_stream(uri: &Uri, timeout: Duration) -> Result<TcpStream, Error> {
    if uri.scheme_str() != Some("ws") {
        return Err(Error::InvalidArgument(
            "only ws:// URLs are supported: protocol 0.x has no TLS".into(),
        ));
    }
    let host = uri
        .host()
        .ok_or_else(|| Error::InvalidArgument("the URL has no host".into()))?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let port = url_port(uri)?;
    let mut last_error = None;
    for address in (host, port).to_socket_addrs().map_err(from_io)? {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => {
                // Requests and responses are small and alternate: do not wait for the
                // delayed acknowledgement before sending.
                stream.set_nodelay(true).map_err(from_io)?;
                stream.set_read_timeout(Some(timeout)).map_err(from_io)?;
                stream.set_write_timeout(Some(timeout)).map_err(from_io)?;
                return Ok(stream);
            }
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.map_or_else(
        || Error::InvalidArgument(format!("the host {host} has no address")),
        from_io,
    ))
}

/// The port of a `ws://` URL: 80 (the RFC 6455 default) only when the URL has none. A
/// port that is present but not a number up to 65535 is an error: falling back to 80 would
/// send the token to another service.
fn url_port(uri: &Uri) -> Result<u16, Error> {
    let authority = uri.authority().map_or("", |authority| authority.as_str());
    // authority = [userinfo@]host[:port], where an IPv6 host is written in brackets.
    let host_and_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, rest)| rest);
    let after_host = uri
        .host()
        .and_then(|host| host_and_port.strip_prefix(host))
        .unwrap_or("");
    let Some(digits) = after_host.strip_prefix(':') else {
        return Ok(80);
    };
    let numeric = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    numeric
        .then(|| digits.parse().ok())
        .flatten()
        .ok_or_else(|| {
            Error::InvalidArgument("the port in the URL is not a number up to 65535".into())
        })
}

/// The token goes into a header value that holds a comma-separated list; only the
/// characters that §2 allows can be there.
fn check_token(token: &str) -> Result<(), Error> {
    let valid = !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidArgument(
            "the token must be non-empty and use only base64url characters".into(),
        ))
    }
}

/// During 0.x the client accepts exactly the version it was built for (§4).
fn check_protocol_version(found: &str) -> Result<(), Error> {
    if found == SUPPORTED_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(Error::UnsupportedVersion {
            found: found.to_string(),
        })
    }
}

fn handshake_error<S>(error: HandshakeError<S>) -> Error
where
    S: tungstenite::handshake::HandshakeRole,
{
    match error {
        // The socket has a read timeout, so a silent server ends up here.
        HandshakeError::Interrupted(_) => Error::Timeout,
        HandshakeError::Failure(error) => from_websocket(error),
    }
}

fn from_websocket(error: tungstenite::Error) -> Error {
    use tungstenite::Error as E;
    match error {
        E::Http(response) => match response.status().as_u16() {
            401 => Error::Unauthorized,
            403 => Error::Forbidden,
            status => Error::Handshake { status },
        },
        E::Io(e) => from_io(e),
        E::ConnectionClosed | E::AlreadyClosed => Error::ConnectionLost,
        E::Protocol(
            ProtocolError::ResetWithoutClosingHandshake | ProtocolError::HandshakeIncomplete,
        ) => Error::ConnectionLost,
        other => Error::Protocol(other.to_string()),
    }
}

fn from_io(error: io::Error) -> Error {
    use io::ErrorKind as K;
    match error.kind() {
        K::WouldBlock | K::TimedOut => Error::Timeout,
        K::UnexpectedEof | K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe => {
            Error::ConnectionLost
        }
        _ => Error::Io(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_protocol_version_is_accepted() {
        assert!(check_protocol_version("0.1").is_ok());
        for found in ["0.2", "0.0", "0.10", "1.0", "1.1", "0.1.0", ""] {
            assert!(
                matches!(
                    check_protocol_version(found),
                    Err(Error::UnsupportedVersion { found: ref f }) if f == found
                ),
                "{found}"
            );
        }
    }

    #[test]
    fn tokens_are_limited_to_base64url_characters() {
        assert!(check_token("Abc-_019").is_ok());
        for token in ["", "a b", "a,b", "a.b", "a\r\nb", "ä"] {
            assert!(check_token(token).is_err(), "{token:?}");
        }
    }

    #[test]
    fn only_close_status_1001_is_a_server_exit() {
        assert!(Error::Closed { code: Some(1001) }.is_server_exit());
        assert!(!Error::Closed { code: Some(1000) }.is_server_exit());
        assert!(!Error::Closed { code: None }.is_server_exit());
        assert!(!Error::ConnectionLost.is_server_exit());
    }

    #[test]
    fn rpc_errors_keep_reason_and_state() {
        let response: WireResponse = serde_json::from_value(json!({
            "jsonrpc": "2.0", "id": 3,
            "error": { "code": 1001, "message": "Busy",
                       "data": { "reason": "busy", "state": "starting" } }
        }))
        .unwrap();
        let Err(Error::Rpc(error)) = decode_response::<Accepted>(response, 3, "start") else {
            panic!("expected an RPC error");
        };
        assert_eq!(error.code, 1001);
        assert_eq!(error.reason.as_deref(), Some("busy"));
        assert_eq!(error.state, Some(State::Starting));
        assert_eq!(error.to_string(), "Busy (code 1001, reason busy)");
    }

    #[test]
    fn a_response_for_another_request_is_a_protocol_error() {
        let response: WireResponse =
            serde_json::from_value(json!({ "jsonrpc": "2.0", "id": 2, "result": { "seq": 1 } }))
                .unwrap();
        assert!(matches!(
            decode_response::<Accepted>(response, 3, "start"),
            Err(Error::Protocol(_))
        ));
    }

    #[test]
    fn only_an_absent_port_defaults_to_80() {
        let port = |url: &str| url_port(&url.parse().unwrap());
        assert_eq!(port("ws://host/rpc").unwrap(), 80);
        assert_eq!(port("ws://[::1]/rpc").unwrap(), 80);
        assert_eq!(port("ws://host:8080/rpc").unwrap(), 8080);
        assert_eq!(port("ws://[::1]:43215/rpc").unwrap(), 43215);
        assert_eq!(port("ws://user:pw@host:81/rpc").unwrap(), 81);
        assert_eq!(port("ws://host:65535/rpc").unwrap(), 65535);
        for url in [
            "ws://host:65536/rpc",
            "ws://host:99999/rpc",
            "ws://[::1]:99999/rpc",
            "ws://host:/rpc",
        ] {
            // The URL parser accepts these (`port_u16()` is `None` for them); they must not
            // become port 80.
            assert!(matches!(port(url), Err(Error::InvalidArgument(_))), "{url}");
        }
    }

    #[test]
    fn invalid_arguments_are_refused_before_connecting() {
        let timeout = Duration::from_secs(1);
        for (url, token, timeout) in [
            ("wss://127.0.0.1:1/rpc", None, timeout),
            ("ws://127.0.0.1:99999/rpc", None, timeout),
            ("not a url", None, timeout),
            ("ws://127.0.0.1:1/rpc", Some("bad token"), timeout),
            ("ws://127.0.0.1:1/rpc", None, Duration::ZERO),
        ] {
            assert!(
                matches!(
                    Client::connect(url, token, timeout),
                    Err(Error::InvalidArgument(_))
                ),
                "{url}"
            );
        }
    }
}
