//! JSON-RPC 2.0 requests: parsing, dispatch and error objects (MRTKLIB `docs/rpc/` §3, §10).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::state::{self, CursorAhead, LifecycleError, Shared};

pub(crate) const PROTOCOL_VERSION: &str = "0.1";

struct RpcError {
    code: i64,
    message: &'static str,
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: &'static str) -> Self {
        Self {
            code,
            message,
            data: None,
        }
    }

    fn invalid_params() -> Self {
        Self {
            code: -32602,
            message: "Invalid params",
            data: Some(json!({ "reason": "invalidParams" })),
        }
    }
}

impl From<LifecycleError> for RpcError {
    fn from(error: LifecycleError) -> Self {
        match error {
            LifecycleError::Busy(state) => Self {
                code: 1001,
                message: "Busy",
                data: Some(json!({ "reason": "busy", "state": state })),
            },
            LifecycleError::AlreadyRunning => Self {
                code: 1002,
                message: "Already running",
                data: Some(json!({ "reason": "alreadyRunning" })),
            },
            LifecycleError::ShuttingDown => Self {
                code: 1003,
                message: "Shutting down",
                data: Some(json!({ "reason": "shuttingDown" })),
            },
        }
    }
}

/// Handle one text frame. Returns the response, or `None` for a notification.
pub(crate) fn handle(shared: &Arc<Shared>, text: &str) -> Option<String> {
    let request: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            return Some(error_response(
                &Value::Null,
                RpcError::new(-32700, "Parse error"),
            ));
        }
    };
    let Some(object) = request.as_object() else {
        // Batches (arrays) are not supported in protocol 0.x.
        return Some(error_response(
            &Value::Null,
            RpcError::new(-32600, "Invalid Request"),
        ));
    };
    let id = object.get("id").cloned();
    let method = object.get("method").and_then(Value::as_str);
    let (Some(method), Some("2.0")) = (method, object.get("jsonrpc").and_then(Value::as_str))
    else {
        return Some(error_response(
            &id.unwrap_or(Value::Null),
            RpcError::new(-32600, "Invalid Request"),
        ));
    };
    // Requests without an id are notifications; they are not executed.
    let id = id?;
    let params = match object.get("params") {
        None => Map::new(),
        Some(Value::Object(params)) => params.clone(),
        Some(_) => return Some(error_response(&id, RpcError::invalid_params())),
    };
    let reply = match dispatch(shared, method, &params) {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
        Err(error) => error_response(&id, error),
    };
    Some(reply)
}

fn dispatch(
    shared: &Arc<Shared>,
    method: &str,
    params: &Map<String, Value>,
) -> Result<Value, RpcError> {
    let no_params = || {
        if params.is_empty() {
            Ok(())
        } else {
            Err(RpcError::invalid_params())
        }
    };
    match method {
        "getVersion" => {
            no_params()?;
            let authentication = if shared.config.token.is_some() {
                "token"
            } else {
                "none"
            };
            Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "mrtklibVersion": "mock",
                "authentication": authentication,
            }))
        }
        "getStatus" | "sync" => {
            no_params()?;
            Ok(shared.status())
        }
        "getEvents" => {
            let (after, limit) = events_params(params)?;
            shared
                .events(after, limit)
                .map_err(|CursorAhead| RpcError::invalid_params())
        }
        "start" => Ok(accepted(state::start(shared, overwrite_params(params)?)?)),
        "stop" => {
            no_params()?;
            Ok(accepted(state::stop(shared)?))
        }
        "restart" => Ok(accepted(state::restart(shared, overwrite_params(params)?)?)),
        "shutdown" => {
            no_params()?;
            Ok(accepted(state::shutdown(shared)))
        }
        _ => Err(RpcError::new(-32601, "Method not found")),
    }
}

fn overwrite_params(params: &Map<String, Value>) -> Result<Vec<String>, RpcError> {
    if params.keys().any(|key| key != "overwrite") {
        return Err(RpcError::invalid_params());
    }
    match params.get("overwrite") {
        None => Ok(Vec::new()),
        Some(Value::Array(paths)) => paths
            .iter()
            .map(|path| {
                path.as_str()
                    .filter(|path| !path.contains('\0'))
                    .map(str::to_owned)
                    .ok_or_else(RpcError::invalid_params)
            })
            .collect(),
        Some(_) => Err(RpcError::invalid_params()),
    }
}

fn events_params(params: &Map<String, Value>) -> Result<(u64, Option<u64>), RpcError> {
    if params.keys().any(|key| key != "after" && key != "limit") {
        return Err(RpcError::invalid_params());
    }
    let integer = |value: &Value| {
        value
            .as_f64()
            .filter(|number| number.is_finite() && number.fract() == 0.0)
    };
    let after = params
        .get("after")
        .and_then(integer)
        .filter(|number| *number >= 0.0 && *number < 9_007_199_254_740_992.0)
        .ok_or_else(RpcError::invalid_params)? as u64;
    let limit = match params.get("limit") {
        None => None,
        Some(value) => Some(
            integer(value)
                .filter(|number| *number >= 1.0)
                .ok_or_else(RpcError::invalid_params)?
                .min(1000.0) as u64,
        ),
    };
    Ok((after, limit))
}

fn accepted(seq: u64) -> Value {
    json!({ "seq": seq })
}

fn error_response(id: &Value, error: RpcError) -> String {
    let mut object = json!({ "code": error.code, "message": error.message });
    if let Some(data) = error.data {
        object["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": object }).to_string()
}
