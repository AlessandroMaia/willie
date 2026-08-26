//! JSON-RPC 2.0 envelopes, one per ndjson line.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const VERSION: &str = "2.0";

fn version() -> String {
    VERSION.to_owned()
}

/// A call that expects a matching [`Response`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    #[serde(default = "version")]
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    /// Builds a request, serialising `params` to a JSON value.
    pub fn new(
        id: u64,
        method: &str,
        params: impl Serialize,
    ) -> serde_json::Result<Self> {
        Ok(Self {
            jsonrpc: version(),
            id,
            method: method.to_owned(),
            params: serde_json::to_value(params)?,
        })
    }
}

/// Error payload carried in a failed [`Response`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    /// Stable `snake_case` code, e.g. `session_not_found`.
    pub code: String,
    /// One sentence describing what went wrong.
    pub message: String,
    /// What the caller can do about it, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl RpcError {
    #[must_use]
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            remediation: None,
        }
    }

    /// Attaches a remediation hint, replacing any existing one.
    #[must_use]
    pub fn with_remediation(mut self, hint: impl Into<String>) -> Self {
        self.remediation = Some(hint.into());
        self
    }
}

/// Reply to a [`Request`], carrying either a result or an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    #[serde(default = "version")]
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// Builds a successful response, serialising `result` to a JSON value.
    pub fn ok(id: u64, result: impl Serialize) -> serde_json::Result<Self> {
        Ok(Self {
            jsonrpc: version(),
            id,
            result: Some(serde_json::to_value(result)?),
            error: None,
        })
    }

    #[must_use]
    pub fn err(id: u64, error: RpcError) -> Self {
        Self {
            jsonrpc: version(),
            id,
            result: None,
            error: Some(error),
        }
    }

    /// Collapses the envelope into a plain `Result`.
    pub fn into_result(self) -> Result<Value, RpcError> {
        match (self.result, self.error) {
            (_, Some(err)) => Err(err),
            (Some(value), None) => Ok(value),
            (None, None) => Ok(Value::Null),
        }
    }
}

/// A message the daemon sends without being asked, e.g. a state update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    #[serde(default = "version")]
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Notification {
    #[must_use]
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: version(),
            method: method.to_owned(),
            params,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serialises_as_json_rpc_2() {
        let req = Request::new(7, "daemon.health", ()).unwrap();
        let json: Value = serde_json::to_value(&req).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 7);
        assert_eq!(json["method"], "daemon.health");
    }

    #[test]
    fn response_ok_and_err_are_mutually_exclusive() {
        let ok = Response::ok(1, 42u8).unwrap();
        assert_eq!(ok.into_result().unwrap(), serde_json::json!(42));
        let err = Response::err(1, RpcError::new("boom", "it broke"));
        assert_eq!(err.into_result().unwrap_err().code, "boom");
    }

    #[test]
    fn a_response_line_from_a_newer_daemon_still_parses() {
        let line =
            r#"{"jsonrpc":"2.0","id":3,"result":{"x":1},"extension":true}"#;
        let resp: Response = serde_json::from_str(line).unwrap();
        assert_eq!(resp.id, 3);
    }

    #[test]
    fn notifications_have_no_id() {
        let n = Notification::new("state.event", serde_json::json!({"seq": 1}));
        let json = serde_json::to_string(&n).unwrap();
        assert!(!json.contains("\"id\""));
    }

    #[test]
    fn remediation_is_omitted_when_absent() {
        let err = RpcError {
            code: "session_not_found".into(),
            message: "no such session".into(),
            remediation: None,
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(!json.contains("remediation"));
    }
}
