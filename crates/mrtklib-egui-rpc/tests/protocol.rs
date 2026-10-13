//! The client against scripted servers: the failures that the mock does not produce.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mrtklib_egui_rpc::{Client, Connection, Error, Session, SessionOptions};
use serde_json::{Value, json};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::HeaderValue;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::{Message, WebSocket};

const TIMEOUT: Duration = Duration::from_secs(2);
const TOKEN: &str = "test-token_0123456789";

type Ws = WebSocket<TcpStream>;

/// A server for one connection: it accepts the upgrade, selects `selected` as the
/// subprotocol and runs `script` on the connection.
// The handshake callback's error type is fixed by tungstenite (`ErrorResponse`).
#[allow(clippy::result_large_err)]
fn fake_server(
    selected: &str,
    script: impl FnOnce(Ws) + Send + 'static,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/rpc", listener.local_addr().unwrap());
    let selected = HeaderValue::from_str(selected).unwrap();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let accept = |_: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
            response
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", selected);
            Ok(response)
        };
        script(tungstenite::accept_hdr(stream, accept).unwrap());
    });
    (url, handle)
}

fn read_request(ws: &mut Ws) -> Value {
    loop {
        match ws.read().expect("the client sends a request") {
            Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("unexpected message {other:?}"),
        }
    }
}

fn answer(ws: &mut Ws, request: &Value, result: Value) {
    let response = json!({ "jsonrpc": "2.0", "id": request["id"], "result": result });
    ws.send(Message::text(response.to_string())).unwrap();
}

/// Answer the `getVersion` that `Client::connect` sends.
fn answer_version(ws: &mut Ws, protocol_version: &str) {
    let request = read_request(ws);
    assert_eq!(request["method"], "getVersion");
    let version = json!({
        "protocolVersion": protocol_version, "mrtklibVersion": "fake", "authentication": "none"
    });
    answer(ws, &request, version);
}

/// Connect to a fake server that answers `getVersion` and then runs `script`; the client
/// then calls `getStatus`.
fn status_error(script: impl FnOnce(&mut Ws, Value) + Send + 'static) -> Error {
    let (url, server) = fake_server("mrtk-rpc", |mut ws| {
        answer_version(&mut ws, "0.1");
        let request = read_request(&mut ws);
        assert_eq!(request["method"], "getStatus");
        script(&mut ws, request);
        // Keep the connection until the client is done, so that only the script's action
        // is visible to it.
        while ws.read().is_ok() {}
    });
    let (mut client, _) = Client::connect(&url, None, TIMEOUT).unwrap();
    let error = client.get_status().unwrap_err();
    drop(client);
    server.join().unwrap();
    error
}

#[test]
fn a_ping_before_the_response_is_answered_by_the_library() {
    let (url, server) = fake_server("mrtk-rpc", |mut ws| {
        answer_version(&mut ws, "0.1");
        let request = read_request(&mut ws);
        ws.send(Message::Ping(vec![1].into())).unwrap();
        let status = json!({ "seq": 4, "status": { "lifecycle": { "state": "stopped" } } });
        answer(&mut ws, &request, status);
        while ws.read().is_ok() {}
    });
    let (mut client, _) = Client::connect(&url, None, TIMEOUT).unwrap();
    assert_eq!(client.get_status().unwrap().seq, 4);
    drop(client);
    server.join().unwrap();
}

#[test]
fn an_unsupported_protocol_version_is_refused() {
    for found in ["0.2", "1.0"] {
        let (url, server) = fake_server("mrtk-rpc", |mut ws| {
            answer_version(&mut ws, found);
            while ws.read().is_ok() {}
        });
        let error = Client::connect(&url, None, TIMEOUT).unwrap_err();
        assert!(
            matches!(&error, Error::UnsupportedVersion { found: f } if f == found),
            "{error:?}"
        );
        server.join().unwrap();
    }
}

#[test]
fn a_server_that_does_not_select_the_protocol_is_refused_without_leaking_the_token() {
    // The server echoes the token entry, which the protocol forbids.
    let (url, server) = fake_server(
        &format!("mrtk-token.{TOKEN}"),
        |mut ws| {
            while ws.read().is_ok() {}
        },
    );
    let error = Client::connect(&url, Some(TOKEN), TIMEOUT).unwrap_err();
    assert!(matches!(error, Error::Protocol(_)), "{error:?}");
    let text = format!("{error:?} {error}");
    assert!(!text.contains(TOKEN), "{text}");
    server.join().unwrap();
}

#[test]
fn a_response_with_another_id_is_a_protocol_error() {
    let error = status_error(|ws, request| {
        let other = json!({ "id": request["id"].as_u64().unwrap() + 1 });
        answer(ws, &other, json!({ "seq": 1 }));
    });
    assert!(matches!(error, Error::Protocol(_)), "{error:?}");
}

#[test]
fn malformed_json_is_a_protocol_error() {
    let error = status_error(|ws, _| ws.send(Message::text("{")).unwrap());
    assert!(matches!(error, Error::Protocol(_)), "{error:?}");
}

#[test]
fn a_result_that_does_not_decode_is_a_protocol_error() {
    let error = status_error(|ws, request| answer(ws, &request, json!({ "seq": 1 })));
    let Error::Protocol(text) = error else {
        panic!("expected a protocol error");
    };
    assert!(text.contains("getStatus"), "{text}");
}

#[test]
fn an_error_response_without_an_id_is_a_protocol_error() {
    let error = status_error(|ws, _| {
        let response = json!({
            "jsonrpc": "2.0", "id": null,
            "error": { "code": -32600, "message": "Invalid Request" }
        });
        ws.send(Message::text(response.to_string())).unwrap();
    });
    let Error::Protocol(text) = error else {
        panic!("expected a protocol error");
    };
    assert!(text.contains("-32600"), "{text}");
}

#[test]
fn a_close_frame_is_reported_with_its_status() {
    for (code, expect_exit) in [(CloseCode::Away, true), (CloseCode::Policy, false)] {
        let error = status_error(move |ws, _| {
            let frame = CloseFrame {
                code,
                reason: "".into(),
            };
            ws.close(Some(frame)).unwrap();
        });
        assert!(
            matches!(error, Error::Closed { code: Some(_) }),
            "{error:?}"
        );
        assert_eq!(error.is_server_exit(), expect_exit);
    }
    let error = status_error(|ws, _| ws.close(None).unwrap());
    assert!(matches!(error, Error::Closed { code: None }), "{error:?}");
}

#[test]
fn a_connection_dropped_without_a_close_frame_is_lost() {
    let (url, server) = fake_server("mrtk-rpc", |mut ws| {
        answer_version(&mut ws, "0.1");
        let _ = read_request(&mut ws);
        // Dropping the connection ends it without a close frame.
    });
    let (mut client, _) = Client::connect(&url, None, TIMEOUT).unwrap();
    let error = client.get_status().unwrap_err();
    assert!(matches!(error, Error::ConnectionLost), "{error:?}");
    server.join().unwrap();
}

#[test]
fn a_silent_server_times_out() {
    let (url, server) = fake_server("mrtk-rpc", |mut ws| {
        answer_version(&mut ws, "0.1");
        let _ = read_request(&mut ws);
        thread::sleep(Duration::from_millis(600));
    });
    let (mut client, _) = Client::connect(&url, None, Duration::from_millis(150)).unwrap();
    let error = client.get_status().unwrap_err();
    assert!(matches!(error, Error::Timeout), "{error:?}");
    drop(client);
    server.join().unwrap();
}

/// A server that accepts the TCP connection and runs `script` on the raw stream.
fn raw_server(script: impl FnOnce(TcpStream) + Send + 'static) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/rpc", listener.local_addr().unwrap());
    let handle = thread::spawn(move || script(listener.accept().unwrap().0));
    (url, handle)
}

#[test]
fn a_refused_handshake_maps_the_http_status() {
    for (status, expected) in [
        (401, "Unauthorized"),
        (403, "Forbidden"),
        (400, "Handshake"),
        (500, "Handshake"),
    ] {
        let (url, server) = raw_server(move |mut stream| {
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 {status} Refused\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let error = Client::connect(&url, None, TIMEOUT).unwrap_err();
        match (&error, expected) {
            (Error::Unauthorized, "Unauthorized") | (Error::Forbidden, "Forbidden") => {}
            (Error::Handshake { status: s }, "Handshake") if *s == status => {}
            _ => panic!("status {status}: {error:?}"),
        }
        server.join().unwrap();
    }
}

#[test]
fn a_server_that_never_answers_the_handshake_times_out() {
    let (url, server) = raw_server(|stream| {
        thread::sleep(Duration::from_millis(600));
        drop(stream);
    });
    let error = Client::connect(&url, None, Duration::from_millis(150)).unwrap_err();
    assert!(matches!(error, Error::Timeout), "{error:?}");
    server.join().unwrap();
}

#[test]
fn a_refused_connection_is_an_io_error() {
    // A port that was free a moment ago and has no listener now.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let error = Client::connect(&format!("ws://127.0.0.1:{port}/rpc"), None, TIMEOUT).unwrap_err();
    assert!(matches!(error, Error::Io(_)), "{error:?}");
}

/// Answer requests with `respond(method, params)` until the client goes away.
fn serve(ws: &mut Ws, mut respond: impl FnMut(&str, &Value) -> Value) {
    loop {
        let request: Value = match ws.read() {
            Ok(Message::Text(text)) => serde_json::from_str(text.as_str()).unwrap(),
            Ok(_) => continue,
            Err(_) => return,
        };
        let result = respond(request["method"].as_str().unwrap(), &request["params"]);
        answer(ws, &request, result);
    }
}

/// A `getStatus` result for a running server.
fn running_status(seq: u64, mode: &str) -> Value {
    json!({
        "seq": seq,
        "status": {
            "lifecycle": { "state": "running" },
            "mode": mode,
            "solution": {
                "time": { "week": 1, "tow": 1.0 }, "leapSeconds": 18, "status": "none"
            }
        }
    })
}

#[test]
fn a_session_takes_only_the_mode_from_a_status_response_and_discards_a_stale_one() {
    let (url, server) = fake_server("mrtk-rpc", |mut ws| {
        answer_version(&mut ws, "0.1");
        let mut status_calls = 0;
        serve(&mut ws, |method, params| match method {
            "sync" => json!({ "seq": 0, "status": { "lifecycle": { "state": "stopped" } } }),
            // The running event, then an event of a kind that the client does not know.
            "getEvents" if params["after"] == 0 => json!({
                "events": [
                    { "seq": 5, "kind": "status", "data": { "state": "running" } },
                    { "seq": 6, "kind": "addedLater", "data": [1, 2] }
                ],
                "next": 6, "more": false, "lost": false
            }),
            "getEvents" => {
                json!({ "events": [], "next": params["after"], "more": false, "lost": false })
            }
            "getStatus" => {
                status_calls += 1;
                // The first response is older than the running event (seq 5): stale.
                if status_calls == 1 {
                    running_status(4, "stale")
                } else {
                    running_status(6, "kinematic")
                }
            }
            other => panic!("unexpected method {other}"),
        });
    });
    let options = SessionOptions {
        poll_interval: Duration::from_millis(20),
        timeout: TIMEOUT,
        ..SessionOptions::default()
    };
    let session = Session::spawn(url, None, options, || {});
    let deadline = std::time::Instant::now() + TIMEOUT;
    let mut modes = Vec::new();
    let model = loop {
        let model = session.model();
        modes.push(model.mode.clone());
        if model.mode.is_some() {
            break model;
        }
        assert!(std::time::Instant::now() < deadline, "no mode: {model:#?}");
        thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(model.mode.as_deref(), Some("kinematic"));
    assert!(!modes.iter().any(|m| m.as_deref() == Some("stale")));
    // The solution of the response is not applied; solutions come from the events.
    assert!(model.solution.is_none());
    // The unknown event was skipped, and the session goes on.
    assert_eq!(model.seq, 6);
    assert!(matches!(model.connection, Connection::Connected { .. }));
    drop(session);
    server.join().unwrap();
}
