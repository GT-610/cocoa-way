use serde::{Deserialize, Serialize};

pub const CONTROL_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
pub struct ControlRequest {
    pub command: String,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ControlResponse {
    pub protocol: u32,
    pub ok: bool,
    pub command: String,
    pub data: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ControlResponse {
    pub fn success(command: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            protocol: CONTROL_PROTOCOL_VERSION,
            ok: true,
            command: command.into(),
            data,
            error: None,
        }
    }

    pub fn failure(command: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            protocol: CONTROL_PROTOCOL_VERSION,
            ok: false,
            command: command.into(),
            data: serde_json::Value::Null,
            error: Some(error.into()),
        }
    }
}

fn default_limit() -> usize {
    100
}
