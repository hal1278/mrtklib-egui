//! One client connection: the WebSocket handshake with the token check, then requests.

use std::io;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::{HeaderValue, StatusCode};
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::{Message, WebSocket};

use crate::rpc;
use crate::state::Shared;

const PROTOCOL: &str = "mrtk-rpc";
const TOKEN_PREFIX: &str = "mrtk-token.";

/// How often a blocked read wakes up to see whether the server is ending.
const POLL: Duration = Duration::from_millis(100);

// The handshake callback's error type is fixed by tungstenite (`ErrorResponse`).
#[allow(clippy::result_large_err)]
pub(crate) fn serve(stream: TcpStream, shared: &Arc<Shared>) {
    // The listener is non-blocking; the handshake and requests use blocking reads.
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let check = |request: &Request, response: Response| check_handshake(shared, request, response);
    let Ok(mut ws) = tungstenite::accept_hdr(stream, check) else {
        return;
    };
    if ws.get_ref().set_read_timeout(Some(POLL)).is_err() {
        return;
    }
    loop {
        if shared.exit.load(Ordering::SeqCst) {
            close(&mut ws, CloseCode::Away);
            return;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let Some(reply) = rpc::handle(shared, text.as_str())
                    && ws.send(Message::text(reply)).is_err()
                {
                    return;
                }
            }
            Ok(Message::Binary(_)) => {
                close(&mut ws, CloseCode::Unsupported);
                return;
            }
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
}

/// Accept the upgrade only for `/rpc` with the `mrtk-rpc` subprotocol, an allowed origin
/// and the token, as MRTKLIB `docs/rpc/` §3 specifies.
#[allow(clippy::result_large_err)] // the error type is fixed by tungstenite's callback
fn check_handshake(
    shared: &Shared,
    request: &Request,
    mut response: Response,
) -> Result<Response, ErrorResponse> {
    if request.uri().path() != "/rpc" {
        return Err(reject(StatusCode::NOT_FOUND));
    }
    let offered: Vec<&str> = request
        .headers()
        .get_all("Sec-WebSocket-Protocol")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect();
    if !offered.contains(&PROTOCOL) {
        return Err(reject(StatusCode::BAD_REQUEST));
    }
    if let Some(origin) = request.headers().get("Origin") {
        let allowed = origin
            .to_str()
            .is_ok_and(|origin| shared.config.allowed_origins.iter().any(|a| a == origin));
        if !allowed {
            return Err(reject(StatusCode::FORBIDDEN));
        }
    }
    if let Some(token) = &shared.config.token {
        let presented = offered
            .iter()
            .any(|p| p.strip_prefix(TOKEN_PREFIX) == Some(token.as_str()));
        if !presented {
            return Err(reject(StatusCode::UNAUTHORIZED));
        }
    }
    response
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", HeaderValue::from_static(PROTOCOL));
    Ok(response)
}

fn reject(status: StatusCode) -> ErrorResponse {
    let mut response = ErrorResponse::new(None);
    *response.status_mut() = status;
    response
}

/// Send a close frame and wait briefly for the peer's reply.
fn close(ws: &mut WebSocket<TcpStream>, code: CloseCode) {
    let frame = CloseFrame {
        code,
        reason: "".into(),
    };
    if ws.close(Some(frame)).is_err() {
        return;
    }
    for _ in 0..10 {
        match ws.read() {
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
            Ok(_) => {}
        }
    }
}
