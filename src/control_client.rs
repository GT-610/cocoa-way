use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crate::control_protocol::{ControlRequest, ControlResponse};

pub fn default_socket_path() -> PathBuf {
    std::env::var_os("COCOA_WAY_CONTROL_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("cocoa-way")
                .join("control")
                .join("control.sock")
        })
}

pub fn send_request(socket: &Path, request: &ControlRequest) -> Result<ControlResponse, String> {
    let mut stream = UnixStream::connect(socket).map_err(|error| {
        format!(
            "cannot connect to Cocoa-Way at {}: {error}. Start Cocoa-Way first.",
            socket.display()
        )
    })?;
    let mut encoded = serde_json::to_vec(request)
        .map_err(|error| format!("failed to encode request: {error}"))?;
    encoded.push(b'\n');
    stream
        .write_all(&encoded)
        .map_err(|error| format!("failed to send request: {error}"))?;

    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| format!("failed to read response: {error}"))?;
    serde_json::from_str(&response).map_err(|error| format!("invalid server response: {error}"))
}
