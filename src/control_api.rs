use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::container_mode;
use crate::container_sessions::{self, ContainerSession};
use crate::control_protocol::{ControlRequest, ControlResponse};
use crate::diagnostics;
use crate::messages::CompositorMessage;
use crate::runtime_paths::{build_child_path, find_command_path};

const CONTROL_SOCKET_ENV: &str = "COCOA_WAY_CONTROL_SOCKET";
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

pub fn socket_path() -> PathBuf {
    std::env::var_os(CONTROL_SOCKET_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("cocoa-way")
                .join("control")
                .join("control.sock")
        })
}

pub fn start(sender: Sender<CompositorMessage>) -> Result<PathBuf, String> {
    let path = socket_path();
    start_at(path, sender)
}

fn start_at(path: PathBuf, sender: Sender<CompositorMessage>) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "control socket path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create control socket directory: {error}"))?;
    let metadata = std::fs::metadata(parent)
        .map_err(|error| format!("failed to inspect control socket directory: {error}"))?;
    if metadata.uid() == unsafe { libc::geteuid() } {
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("failed to secure control socket directory: {error}"))?;
    }
    if path.exists() && UnixStream::connect(&path).is_ok() {
        return Err(format!(
            "another Cocoa-Way control server is active at {}",
            path.display()
        ));
    }
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("failed to replace stale control socket: {error}"))?;
    }
    let listener = UnixListener::bind(&path)
        .map_err(|error| format!("failed to bind control socket: {error}"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("failed to secure control socket: {error}"))?;

    let thread_path = path.clone();
    std::thread::Builder::new()
        .name("cocoa-way-control".into())
        .spawn(move || {
            for connection in listener.incoming() {
                match connection {
                    Ok(stream) => {
                        let connection_sender = sender.clone();
                        let connection_path = thread_path.clone();
                        let _ = std::thread::Builder::new()
                            .name("cocoa-way-control-request".into())
                            .spawn(move || {
                                handle_connection(stream, &connection_sender, &connection_path)
                            });
                    }
                    Err(error) => log::warn!("Control socket accept failed: {error}"),
                }
            }
        })
        .map_err(|error| format!("failed to start control socket thread: {error}"))?;
    Ok(path)
}

pub fn remove_socket(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn handle_connection(mut stream: UnixStream, sender: &Sender<CompositorMessage>, path: &Path) {
    let request = {
        let mut line = String::new();
        let mut reader = BufReader::new((&stream).take(MAX_REQUEST_BYTES));
        match reader.read_line(&mut line) {
            Ok(0) => Err("empty control request".to_string()),
            Ok(_) => serde_json::from_str::<ControlRequest>(&line)
                .map_err(|error| format!("invalid control request: {error}")),
            Err(error) => Err(format!("failed to read control request: {error}")),
        }
    };
    let response = match request {
        Ok(request) => dispatch(request, sender, path),
        Err(error) => ControlResponse::failure("invalid", error),
    };
    if let Ok(mut encoded) = serde_json::to_vec(&response) {
        encoded.push(b'\n');
        let _ = stream.write_all(&encoded);
    }
}

fn dispatch(
    request: ControlRequest,
    sender: &Sender<CompositorMessage>,
    path: &Path,
) -> ControlResponse {
    let command = request.command.trim().to_ascii_lowercase();
    match command.as_str() {
        "status" => ControlResponse::success(&command, status_snapshot(path)),
        "sessions" => ControlResponse::success(&command, sessions_snapshot()),
        "displays" => ControlResponse::success(&command, displays_snapshot()),
        "images" => ControlResponse::success(&command, images_snapshot()),
        "logs" => match resolve_session(request.session.as_deref()) {
            Ok((index, session)) => ControlResponse::success(
                &command,
                json!({
                    "index": index,
                    "name": session.name,
                    "lines": container_mode::control_session_logs(index, request.limit.clamp(1, 1000)),
                }),
            ),
            Err(error) => ControlResponse::failure(&command, error),
        },
        "launch" | "start" | "stop" | "check" => {
            queue_session_command(&command, request.session.as_deref(), sender)
        }
        _ => ControlResponse::failure(
            &command,
            "unsupported command; use status, sessions, displays, images, logs, check, launch, or stop",
        ),
    }
}

fn queue_session_command(
    command: &str,
    selector: Option<&str>,
    sender: &Sender<CompositorMessage>,
) -> ControlResponse {
    let (index, session) = match resolve_session(selector) {
        Ok(resolved) => resolved,
        Err(error) => return ControlResponse::failure(command, error),
    };
    let message = match command {
        "launch" | "start" => CompositorMessage::StartContainerSession(index),
        "stop" => CompositorMessage::StopContainerSession(index),
        "check" => CompositorMessage::CheckContainerSession(index),
        _ => unreachable!(),
    };
    match sender.send(message) {
        Ok(()) => ControlResponse::success(
            command,
            json!({
                "accepted": true,
                "index": index,
                "name": session.name,
                "note": "The command was queued on Cocoa-Way's compositor event loop.",
            }),
        ),
        Err(error) => {
            ControlResponse::failure(command, format!("event loop is unavailable: {error}"))
        }
    }
}

fn resolve_session(selector: Option<&str>) -> Result<(usize, ContainerSession), String> {
    let selector = selector
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "a session index or exact name is required".to_string())?;
    let sessions = container_sessions::load_sessions();
    if let Some((index, session)) = sessions
        .iter()
        .enumerate()
        .find(|(_, session)| session.name.eq_ignore_ascii_case(selector))
    {
        return Ok((index, session.clone()));
    }
    if let Ok(index) = selector.parse::<usize>()
        && let Some(session) = sessions.get(index)
    {
        return Ok((index, session.clone()));
    }
    Err(format!("session '{selector}' was not found"))
}

fn status_snapshot(path: &Path) -> Value {
    let active = active_sessions_json();
    let performance = container_mode::control_performance_snapshot().map(
        |(
            redraw_fps,
            commits_per_second,
            tiles,
            dirty,
            pending_frame_callbacks,
            late_redraws_per_second,
            max_redraw_wait_ms,
            input_to_present_ms,
        )| {
            json!({
                "redraw_fps": redraw_fps,
                "commits_per_second": commits_per_second,
                "tiles": tiles,
                "dirty": dirty,
                "pending_frame_callbacks": pending_frame_callbacks,
                "late_redraws_per_second": late_redraws_per_second,
                "max_redraw_wait_ms": max_redraw_wait_ms,
                "host_input_to_present_ms": input_to_present_ms,
            })
        },
    );
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "control_socket": path,
        "configured_sessions": container_sessions::load_sessions().len(),
        "active_sessions": active,
        "performance": performance,
        "resources": diagnostics::resource_snapshot(),
        "clipboard": diagnostics::clipboard_snapshot(),
        "activity": container_mode::control_activity_snapshot(10),
    })
}

fn sessions_snapshot() -> Value {
    let active = container_mode::control_active_sessions();
    Value::Array(
        container_sessions::load_sessions()
            .into_iter()
            .enumerate()
            .map(|(index, session)| {
                let tracked = active.iter().find(|active| active.0 == index);
                let state = container_mode::control_session_state(index);
                json!({
                    "index": index,
                    "name": session.name,
                    "runtime": session.runtime,
                    "image": session.image,
                    "command": session.command,
                    "profile": session.profile,
                    "display": session.display.as_deref().unwrap_or("auto"),
                    "state": state.as_ref().map(|state| state.0.as_str()).unwrap_or(if tracked.is_some() { "Running" } else { "Idle" }),
                    "state_detail": state.map(|state| state.1),
                    "active": tracked.map(|active| json!({
                        "container_pid": active.1,
                        "waypipe_pid": active.2,
                        "display_slot": active.3,
                        "display_pid": active.4,
                    })),
                })
            })
            .collect(),
    )
}

fn displays_snapshot() -> Value {
    json!({
        "default": {
            "kind": "embedded",
            "description": "The compositor window owned by the main Cocoa-Way process."
        },
        "active": active_sessions_json(),
    })
}

fn active_sessions_json() -> Value {
    Value::Array(
        container_mode::control_active_sessions()
            .into_iter()
            .map(
                |(index, container_pid, waypipe_pid, display_slot, display_pid)| {
                    json!({
                        "index": index,
                        "container_pid": container_pid,
                        "waypipe_pid": waypipe_pid,
                        "display_slot": display_slot,
                        "display_pid": display_pid,
                    })
                },
            )
            .collect(),
    )
}

fn images_snapshot() -> Value {
    let child_path = build_child_path();
    json!({
        "apple_container": command_snapshot(
            "container",
            &["image", "list"],
            &child_path,
        ),
        "docker": command_snapshot(
            "docker",
            &["image", "ls", "--format", "{{json .}}"],
            &child_path,
        ),
    })
}

fn command_snapshot(command: &str, args: &[&str], child_path: &str) -> Value {
    let Some(path) = find_command_path(command, child_path) else {
        return json!({ "available": false, "error": format!("{command} command not found") });
    };
    match run_command(&path, args, child_path, Duration::from_secs(3)) {
        Ok(output) => json!({
            "available": true,
            "success": output.status.success(),
            "stdout": String::from_utf8_lossy(&output.stdout).lines().collect::<Vec<_>>(),
            "stderr": String::from_utf8_lossy(&output.stderr).lines().collect::<Vec<_>>(),
        }),
        Err(error) => json!({ "available": true, "success": false, "error": error }),
    }
}

fn run_command(
    path: &Path,
    args: &[&str],
    child_path: &str,
    timeout: Duration,
) -> Result<Output, String> {
    let mut child = Command::new(path)
        .env("PATH", child_path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().map_err(|error| error.to_string()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("command timed out after {}ms", timeout.as_millis()));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn control_socket_is_private_to_the_runtime_directory() {
        let path = socket_path();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("control.sock")
        );
        assert!(path.parent().is_some());
    }

    #[test]
    fn unsupported_control_command_returns_structured_failure() {
        let (sender, _receiver) = std::sync::mpsc::channel();
        let response = dispatch(
            ControlRequest {
                command: "delete".into(),
                session: None,
                limit: 10,
            },
            &sender,
            Path::new("/tmp/control.sock"),
        );
        assert!(!response.ok);
        assert!(response.error.unwrap().contains("unsupported command"));
    }

    #[test]
    fn unix_socket_returns_a_json_status_response() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("control.sock");
        let (sender, _receiver) = std::sync::mpsc::channel();
        start_at(socket.clone(), sender).unwrap();

        let mut stream = UnixStream::connect(&socket).unwrap();
        stream
            .write_all(b"{\"command\":\"status\",\"limit\":10}\n")
            .unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        let response: ControlResponse = serde_json::from_str(&response).unwrap();
        assert!(response.ok);
        assert_eq!(response.command, "status");
        assert_eq!(
            response.data["control_socket"],
            socket.to_string_lossy().as_ref()
        );
    }
}
