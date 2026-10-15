//! Scenarios against the mock. Every result, event and error datum is validated against
//! the pinned contract (`contract/openrpc.json`), so the mock cannot teach the GUI a
//! message shape that the real server will not send.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

use mrtklib_egui_mock::{Config, Running, start};
use serde_json::{Value, json};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const TOKEN: &str = "test-token_0123456789";

struct Contract {
    doc: Value,
    validators: RefCell<HashMap<String, jsonschema::Validator>>,
}

impl Contract {
    fn load() -> Self {
        let doc = serde_json::from_str(include_str!("../contract/openrpc.json")).expect("contract");
        Self {
            doc,
            validators: RefCell::new(HashMap::new()),
        }
    }

    /// Validate `value` against a schema of the contract, e.g. `#/components/schemas/Event`.
    fn check(&self, schema: &str, value: &Value) {
        let mut validators = self.validators.borrow_mut();
        let validator = validators.entry(schema.to_string()).or_insert_with(|| {
            // The OpenRPC document is the root, so that its `#/components/...` references resolve.
            let mut root = self.doc.clone();
            root["$ref"] = json!(schema);
            jsonschema::options()
                .with_draft(jsonschema::Draft::Draft7)
                .build(&root)
                .expect("schema")
        });
        let errors: Vec<String> = validator
            .iter_errors(value)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{schema}: {errors:?}\n{value}");
    }

    fn result_schema(&self, method: &str) -> String {
        let methods = self.doc["methods"].as_array().expect("methods");
        let method = methods
            .iter()
            .find(|m| m["name"] == method)
            .expect("method in contract");
        method["result"]["schema"]["$ref"]
            .as_str()
            .expect("result $ref")
            .to_string()
    }
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

fn connect(url: &str, protocols: &str, origin: Option<&str>) -> Result<Socket, tungstenite::Error> {
    let mut request = url.into_client_request()?;
    let headers = request.headers_mut();
    headers.insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(protocols).unwrap(),
    );
    if let Some(origin) = origin {
        headers.insert("Origin", HeaderValue::from_str(origin).unwrap());
    }
    tungstenite::connect(request).map(|(ws, _)| ws)
}

fn http_status(result: Result<Socket, tungstenite::Error>) -> u16 {
    match result {
        Err(tungstenite::Error::Http(response)) => response.status().as_u16(),
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => 101,
    }
}

struct Client {
    ws: Socket,
    id: u64,
    contract: Contract,
    cursor: u64,
    events: Vec<Value>,
}

impl Client {
    fn new(running: &Running) -> Self {
        let ws = connect(
            running.url(),
            &format!("mrtk-rpc, mrtk-token.{TOKEN}"),
            None,
        )
        .expect("connect");
        let mut client = Self {
            ws,
            id: 0,
            contract: Contract::load(),
            cursor: 0,
            events: Vec::new(),
        };
        let snapshot = client.call("sync", json!({})).expect("sync");
        client.cursor = snapshot["seq"].as_u64().unwrap();
        client
    }

    fn send_text(&mut self, text: &str) {
        self.ws.send(Message::text(text)).expect("send");
    }

    fn read_json(&mut self) -> Value {
        loop {
            match self.ws.read().expect("read") {
                Message::Text(text) => return serde_json::from_str(text.as_str()).expect("json"),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("unexpected message {other:?}"),
            }
        }
    }

    /// Call a method; check the result or the error data against the contract.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.id += 1;
        let request =
            json!({ "jsonrpc": "2.0", "id": self.id, "method": method, "params": params });
        self.send_text(&request.to_string());
        let response = self.read_json();
        assert_eq!(response["id"], self.id);
        if let Some(result) = response.get("result") {
            self.contract
                .check(&self.contract.result_schema(method), result);
            return Ok(result.clone());
        }
        let error = response["error"].clone();
        if let Some(data) = error.get("data") {
            self.contract.check("#/components/schemas/ErrorData", data);
        }
        Err(error)
    }

    /// Poll events until `done` holds for the events received so far.
    fn poll_until(&mut self, done: impl Fn(&[Value]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(&self.events) {
            assert!(
                Instant::now() < deadline,
                "timed out; events: {:#?}",
                self.events
            );
            let page = self
                .call("getEvents", json!({ "after": self.cursor }))
                .expect("getEvents");
            assert_eq!(page["lost"], false, "events lost: {page}");
            self.events
                .extend(page["events"].as_array().unwrap().iter().cloned());
            self.cursor = page["next"].as_u64().unwrap();
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn fast() -> Config {
    Config {
        token: Some(TOKEN.to_string()),
        transition: Duration::from_millis(100),
        solution_interval: Duration::from_millis(20),
        ..Config::default()
    }
}

fn states(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["kind"] == "status")
        .map(|e| e["data"]["state"].as_str().unwrap().to_string())
        .collect()
}

fn solutions(events: &[Value]) -> usize {
    events.iter().filter(|e| e["kind"] == "solution").count()
}

fn last_state(events: &[Value]) -> Option<String> {
    states(events).last().cloned()
}

fn error_reason(error: &Value) -> &str {
    error["data"]["reason"].as_str().unwrap_or("")
}

#[test]
fn handshake_checks_path_protocol_origin_and_token() {
    let running = start(Config {
        allowed_origins: vec!["http://localhost:8080".into()],
        ..fast()
    })
    .unwrap();
    let url = running.url();
    let good = format!("mrtk-rpc, mrtk-token.{TOKEN}");
    assert_eq!(
        http_status(connect(&url.replace("/rpc", "/x"), &good, None)),
        404
    );
    assert_eq!(
        http_status(connect(url, &format!("mrtk-token.{TOKEN}"), None)),
        400
    );
    assert_eq!(http_status(connect(url, "mrtk-rpc", None)), 401);
    assert_eq!(
        http_status(connect(url, "mrtk-rpc, mrtk-token.wrong", None)),
        401
    );
    assert_eq!(
        http_status(connect(url, &good, Some("http://evil.example"))),
        403
    );
    assert_eq!(
        http_status(connect(url, &good, Some("http://localhost:8080"))),
        101
    );
    assert_eq!(http_status(connect(url, &good, None)), 101);
}

#[test]
fn start_publishes_solutions_and_stop_ends_the_run() {
    let running = start(fast()).unwrap();
    let mut client = Client::new(&running);
    let version = client.call("getVersion", json!({})).unwrap();
    assert_eq!(version["protocolVersion"], "0.1");
    assert_eq!(version["authentication"], "token");

    let status = client.call("getStatus", json!({})).unwrap();
    assert_eq!(status["status"]["lifecycle"]["state"], "stopped");

    client.call("start", json!({})).unwrap();
    client.poll_until(|events| solutions(events) >= 12);
    let status = client.call("getStatus", json!({})).unwrap();
    assert_eq!(status["status"]["mode"], "kinematic");
    assert!(status["status"]["solution"].is_object());

    client.call("stop", json!({})).unwrap();
    client.poll_until(|events| last_state(events).as_deref() == Some("stopped"));
    assert_eq!(
        states(&client.events),
        ["starting", "running", "stopping", "stopped"]
    );
    let stopped = client
        .events
        .iter()
        .rev()
        .find(|e| e["kind"] == "status")
        .unwrap();
    assert_eq!(stopped["data"]["stopReason"], "request");
    let statuses: Vec<&str> = client
        .events
        .iter()
        .filter(|e| e["kind"] == "solution")
        .map(|e| e["data"]["status"].as_str().unwrap())
        .collect();
    assert!(
        statuses.contains(&"single") && statuses.contains(&"float") && statuses.contains(&"fix")
    );

    // Stopping a stopped server succeeds without effect.
    let before = client.call("getStatus", json!({})).unwrap()["seq"].clone();
    assert_eq!(client.call("stop", json!({})).unwrap()["seq"], before);
}

#[test]
fn lifecycle_requests_are_refused_while_busy_or_running() {
    let running = start(Config {
        transition: Duration::from_millis(300),
        ..fast()
    })
    .unwrap();
    let mut client = Client::new(&running);
    client.call("start", json!({})).unwrap();
    let busy = client.call("start", json!({})).unwrap_err();
    assert_eq!(error_reason(&busy), "busy");
    assert_eq!(busy["data"]["state"], "starting");
    client.poll_until(|events| last_state(events).as_deref() == Some("running"));
    let already = client.call("start", json!({})).unwrap_err();
    assert_eq!(already["code"], 1002);
    assert_eq!(error_reason(&already), "alreadyRunning");
}

#[test]
fn restart_passes_through_stopped_and_stays_busy() {
    let running = start(fast()).unwrap();
    let mut client = Client::new(&running);
    client.call("start", json!({})).unwrap();
    client.poll_until(|events| last_state(events).as_deref() == Some("running"));
    client.call("restart", json!({})).unwrap();
    let busy = client.call("start", json!({})).unwrap_err();
    assert_eq!(error_reason(&busy), "busy");
    client.poll_until(|events| states(events).len() >= 6);
    assert_eq!(
        states(&client.events),
        [
            "starting", "running", "stopping", "stopped", "starting", "running"
        ]
    );
}

#[test]
fn failed_start_reports_last_error() {
    let running = start(Config {
        fail_start: true,
        ..fast()
    })
    .unwrap();
    let mut client = Client::new(&running);
    client.call("start", json!({})).unwrap();
    client.poll_until(|events| last_state(events).as_deref() == Some("stopped"));
    let lifecycle = &client.call("getStatus", json!({})).unwrap()["status"]["lifecycle"];
    assert_eq!(lifecycle["stopReason"], "error");
    assert_eq!(lifecycle["lastError"]["reason"], "startFailed");
}

#[test]
fn input_end_stops_the_run_by_itself() {
    let running = start(Config {
        input_end_after: Some(3),
        ..fast()
    })
    .unwrap();
    let mut client = Client::new(&running);
    client.call("start", json!({})).unwrap();
    client.poll_until(|events| last_state(events).as_deref() == Some("stopped"));
    assert_eq!(solutions(&client.events), 3);
    let stopped = client
        .events
        .iter()
        .rev()
        .find(|e| e["kind"] == "status")
        .unwrap();
    assert_eq!(stopped["data"]["stopReason"], "inputEnd");
}

#[test]
fn pages_and_lost_events() {
    let running = start(Config {
        event_capacity: 5,
        solution_interval: Duration::from_millis(10),
        ..fast()
    })
    .unwrap();
    let mut client = Client::new(&running);
    client.call("start", json!({})).unwrap();
    thread::sleep(Duration::from_millis(400));
    let page = client.call("getEvents", json!({ "after": 0 })).unwrap();
    assert_eq!(page["lost"], true);
    assert!(page["oldestSeq"].as_u64().unwrap() > 1);

    let snapshot = client.call("sync", json!({})).unwrap();
    let seq = snapshot["seq"].as_u64().unwrap();
    thread::sleep(Duration::from_millis(40));
    let page = client
        .call("getEvents", json!({ "after": seq, "limit": 1 }))
        .unwrap();
    assert_eq!(page["events"].as_array().unwrap().len(), 1);
    assert_eq!(page["more"], true);
}

#[test]
fn protocol_errors() {
    let running = start(fast()).unwrap();
    let mut client = Client::new(&running);
    client.send_text("not json");
    let parse = client.read_json();
    assert_eq!(parse["error"]["code"], -32700);
    assert_eq!(parse["id"], Value::Null);

    client.send_text("[]");
    assert_eq!(client.read_json()["error"]["code"], -32600);

    // A notification is not answered; the next response belongs to the next request.
    client.send_text(r#"{"jsonrpc":"2.0","method":"start"}"#);
    let status = client.call("getStatus", json!({})).unwrap();
    assert_eq!(status["status"]["lifecycle"]["state"], "stopped");

    assert_eq!(
        client.call("noSuchMethod", json!({})).unwrap_err()["code"],
        -32601
    );
    for params in [
        json!({}),
        json!({ "after": 1_000_000 }),
        json!({ "after": 0, "limit": 0 }),
        json!({ "after": 0, "x": 1 }),
    ] {
        let error = client.call("getEvents", params).unwrap_err();
        assert_eq!(error["code"], -32602);
        assert_eq!(error_reason(&error), "invalidParams");
    }
    assert_eq!(
        client.call("getStatus", json!({ "x": 1 })).unwrap_err()["code"],
        -32602
    );
}

#[test]
fn binary_frames_close_with_unsupported_data() {
    let running = start(fast()).unwrap();
    let mut client = Client::new(&running);
    client.ws.send(Message::binary(vec![1, 2, 3])).unwrap();
    match client.ws.read().unwrap() {
        Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Unsupported),
        other => panic!("expected a close frame, got {other:?}"),
    }
}

#[test]
fn shutdown_during_start_runs_after_the_transition() {
    let running = start(Config {
        transition: Duration::from_millis(200),
        ..fast()
    })
    .unwrap();
    let mut client = Client::new(&running);
    client.call("start", json!({})).unwrap();
    client.call("shutdown", json!({})).unwrap();
    let refused = client.call("stop", json!({})).unwrap_err();
    assert_eq!(error_reason(&refused), "shuttingDown");
    client.poll_until(|events| last_state(events).as_deref() == Some("stopped"));
    assert_eq!(
        states(&client.events),
        ["starting", "running", "stopping", "stopped"]
    );
    loop {
        match client.ws.read() {
            Ok(Message::Close(Some(frame))) => {
                assert_eq!(frame.code, CloseCode::Away);
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("expected a close frame, got {e}"),
        }
    }
    running.wait();
}

#[test]
fn overwrite_consent_is_exact_distinct_and_belongs_to_each_request() {
    let running = start(Config {
        existing_outputs: vec!["b.pos".into(), "a.pos".into(), "b.pos".into()],
        ..fast()
    })
    .unwrap();
    let mut client = Client::new(&running);
    client
        .call("start", json!({"overwrite": ["./b.pos"]}))
        .unwrap();
    client.poll_until(|events| last_state(events).as_deref() == Some("stopped"));
    let status = client.call("getStatus", json!({})).unwrap();
    assert_eq!(
        status["status"]["lifecycle"]["lastError"]["paths"],
        json!(["b.pos", "a.pos"])
    );
    client.events.clear();
    client
        .call("start", json!({"overwrite": ["b.pos"]}))
        .unwrap();
    client.poll_until(|events| last_state(events).as_deref() == Some("stopped"));
    let status = client.call("getStatus", json!({})).unwrap();
    assert_eq!(
        status["status"]["lifecycle"]["lastError"]["paths"],
        json!(["a.pos"])
    );
    client.events.clear();
    client
        .call("start", json!({"overwrite": ["b.pos", "a.pos"]}))
        .unwrap();
    client.poll_until(|events| last_state(events).as_deref() == Some("running"));
    client.events.clear();
    client.call("restart", json!({})).unwrap();
    client.poll_until(|events| states(events).len() >= 4);
    let status = client.call("getStatus", json!({})).unwrap();
    assert_eq!(
        status["status"]["lifecycle"]["lastError"]["paths"],
        json!(["b.pos", "a.pos"])
    );
}

#[test]
fn invalid_overwrite_and_unknown_parameters_are_rejected() {
    let running = start(fast()).unwrap();
    let mut client = Client::new(&running);
    for method in ["start", "restart"] {
        for overwrite in [
            json!(null),
            json!(1),
            json!(true),
            json!("path"),
            json!({}),
            json!([1]),
            json!([false]),
            json!([null]),
            json!(["a\u{0}b"]),
        ] {
            let error = client
                .call(method, json!({"overwrite": overwrite}))
                .unwrap_err();
            assert_eq!(error_reason(&error), "invalidParams");
        }
    }
    for method in [
        "getVersion",
        "getStatus",
        "sync",
        "getEvents",
        "start",
        "stop",
        "restart",
        "shutdown",
    ] {
        let error = client.call(method, json!({"unknown": 1})).unwrap_err();
        assert_eq!(error_reason(&error), "invalidParams");
    }
}

#[test]
fn event_limit_is_cut_to_the_server_maximum() {
    let running = start(Config {
        page_limit: 2,
        ..fast()
    })
    .unwrap();
    let mut client = Client::new(&running);
    client.call("start", json!({})).unwrap();
    client.poll_until(|events| solutions(events) >= 2);
    let page = client
        .call("getEvents", json!({"after": 0.0, "limit": 1e99}))
        .unwrap();
    assert_eq!(page["events"].as_array().unwrap().len(), 2);
    assert_eq!(page["more"], true);
}
