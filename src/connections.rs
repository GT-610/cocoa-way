use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use crate::runtime_paths::{build_child_path, resolve_command_path};

#[derive(Debug, Clone, Deserialize)]
pub struct Connection {
    pub name: String,
    #[serde(rename = "type", default = "default_type")]
    pub conn_type: String, // "ssh" or "local"
    pub host: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity: Option<String>,
    pub socket: Option<String>, // for conn_type = "local"
    pub app: Option<String>,    // program to launch on remote
    pub password: Option<String>,
    pub waypipe_path: Option<String>,
}

fn default_type() -> String {
    "ssh".into()
}

#[derive(Deserialize, Default)]
struct Config {
    pub waypipe_path: Option<String>,
    #[serde(default)]
    connection: Vec<Connection>,
}

/// Load connections from ~/.config/cocoa-way/connections.toml.
/// Creates an example file if none exists.
pub fn load_connections() -> Vec<Connection> {
    let home = std::env::var("HOME").unwrap_or_default();
    let config_dir = std::path::PathBuf::from(&home).join(".config/cocoa-way");
    let path = config_dir.join("connections.toml");

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            // Write an example config so the user knows the format
            let _ = std::fs::create_dir_all(&config_dir);
            let example = r#"# cocoa-way connections
# Each [[connection]] block defines a remote machine to connect to via waypipe.
# Optional: set this if waypipe is installed somewhere cocoa-way cannot discover.
# waypipe_path = "/opt/homebrew/bin/waypipe"

# --- Local VM example (Unix socket) ---
# [[connection]]
# name = "Linux VM"
# type = "local"
# socket = "/tmp/waypipe-vm.sock"
# app = "weston-terminal"

# --- Remote SSH example ---
# [[connection]]
# name = "Home Server"
# type = "ssh"
# host = "192.168.1.100"
# user = "jiaxi"
# app = "weston-terminal"
# port = 22
# identity = "~/.ssh/id_rsa"
"#;
            let _ = std::fs::write(&path, example);
            log::info!("Created example connections.toml at {:?}", path);
            return vec![];
        }
    };

    match toml::from_str::<Config>(&content) {
        Ok(mut cfg) => {
            for conn in &mut cfg.connection {
                if conn.waypipe_path.is_none() {
                    conn.waypipe_path = cfg.waypipe_path.clone();
                }
            }
            cfg.connection
        }
        Err(e) => {
            log::warn!("Failed to parse connections.toml: {}", e);
            vec![]
        }
    }
}

/// Spawn a waypipe connection for the given connection config.
/// Returns the child process handle so the caller can track/kill it.
pub fn spawn_waypipe(conn: &Connection, runtime_dir: &str, display: &str) -> Option<Child> {
    let app = conn.app.as_deref().unwrap_or("weston-terminal");
    let waypipe = resolve_waypipe_path(conn.waypipe_path.as_deref())?;
    let child_path = build_child_path();

    match conn.conn_type.as_str() {
        "local" => {
            // Local VM reachable through a Unix socket (e.g. OrbStack / QEMU)
            let socket = conn.socket.as_deref()?;
            spawn_local_waypipe_client(&waypipe, &child_path, runtime_dir, display, socket)
        }
        _ => {
            // SSH connection
            let host = conn.host.as_deref()?;
            let user = conn.user.as_deref().unwrap_or("root");
            let target = format!("{}@{}", user, host);

            let mut args: Vec<String> = vec![
                "--compress".into(),
                "lz4".into(),
                "ssh".into(),
                "-o".into(),
                "StreamLocalBindUnlink=yes".into(),
            ];
            if let Some(port) = conn.port {
                args.extend(["-p".into(), port.to_string()]);
            }
            if let Some(identity) = &conn.identity {
                args.extend(["-i".into(), identity.clone()]);
            }
            args.push(target);
            args.push(app.into());

            let mut cmd = Command::new(&waypipe);
            cmd.env("PATH", &child_path)
                .env("XDG_RUNTIME_DIR", runtime_dir)
                .env("WAYLAND_DISPLAY", display)
                .args(&args);

            if let Some(pw) = &conn.password {
                spawn_with_askpass(&mut cmd, pw)
            } else {
                cmd.spawn()
                    .map_err(|e| log::error!("Failed to spawn waypipe (ssh): {}", e))
                    .ok()
            }
        }
    }
}

fn resolve_waypipe_path(configured: Option<&str>) -> Option<PathBuf> {
    let child_path = build_child_path();
    resolve_command_path("waypipe", configured, "waypipe", &child_path)
}

fn spawn_local_waypipe_client(
    waypipe: &Path,
    child_path: &str,
    runtime_dir: &str,
    display: &str,
    socket: &str,
) -> Option<Child> {
    Command::new(waypipe)
        .env("PATH", child_path)
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .env("WAYLAND_DISPLAY", display)
        .args(["--socket", socket, "client"])
        .spawn()
        .map_err(|e| log::error!("Failed to spawn waypipe (local): {}", e))
        .ok()
}

/// Spawn a command with SSH_ASKPASS set to a temporary script that returns the password.
/// The script is deleted after 30 s — long enough for the SSH handshake to complete.
fn spawn_with_askpass(cmd: &mut Command, password: &str) -> Option<Child> {
    use std::os::unix::fs::PermissionsExt;

    let tmp_path =
        std::env::temp_dir().join(format!("cocoa-way-askpass-{}.sh", std::process::id()));

    // Shell-escape the password for use inside a single-quoted string.
    let escaped = password.replace('\'', "'\\''");
    let script = format!("#!/bin/sh\nprintf '%s' '{}'\n", escaped);

    std::fs::write(&tmp_path, &script)
        .map_err(|e| log::error!("askpass: write failed: {}", e))
        .ok()?;
    std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| log::error!("askpass: chmod failed: {}", e))
        .ok()?;

    // SSH_ASKPASS_REQUIRE=force tells OpenSSH to call the helper even without a tty.
    cmd.env("SSH_ASKPASS", &tmp_path)
        .env("SSH_ASKPASS_REQUIRE", "force");

    let child = cmd
        .spawn()
        .map_err(|e| log::error!("Failed to spawn waypipe (password): {}", e))
        .ok();

    // Delete the temp script after 30 s — the SSH handshake is done by then.
    let cleanup = tmp_path;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(30));
        let _ = std::fs::remove_file(&cleanup);
    });

    child
}
