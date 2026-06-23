use serde::Deserialize;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use crate::runtime_paths::{build_child_path, resolve_command_path, shell_single_quote};

#[derive(Debug, Clone, Deserialize)]
pub struct ContainerSession {
    pub name: String,
    pub image: String,
    #[serde(default = "default_runtime")]
    pub runtime: String,
    pub profile: Option<String>,
    pub app: Option<String>,
    pub command: Option<String>,
    pub socket: Option<String>,
    pub container_socket: Option<String>,
    pub waypipe_path: Option<String>,
    #[serde(default)]
    pub runtime_args: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
}

#[derive(Deserialize, Default)]
struct Config {
    #[serde(default)]
    session: Vec<ContainerSession>,
}

#[derive(Clone, Copy)]
enum ContainerRuntime {
    Apple,
    Docker,
    OrbStack,
}

fn default_runtime() -> String {
    "container".into()
}

pub fn config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home)
        .join(".config/cocoa-way")
        .join("container-sessions.toml")
}

pub fn load_sessions() -> Vec<ContainerSession> {
    let path = config_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let example = r#"# cocoa-way container sessions
# Container Mode owns local Linux GUI sessions. Classic SSH/OrbStack entries
# remain in ~/.config/cocoa-way/connections.toml.

# [[session]]
# name = "Ubuntu App"
# runtime = "container"
# image = "docker.io/library/ubuntu:24.04"
# profile = "single-app"
# app = "weston-terminal"
# container_socket = "/tmp/cocoa-way/waypipe.sock"
# runtime_args = ["--rosetta"]

# [[session]]
# name = "Niri Desktop"
# runtime = "container"
# image = "localhost/cocoa-way-niri:latest"
# profile = "niri"
# command = "niri --session"
"#;
            let _ = std::fs::write(&path, example);
            log::info!("Created example container-sessions.toml at {:?}", path);
            return vec![];
        }
    };

    match toml::from_str::<Config>(&content) {
        Ok(cfg) => cfg.session,
        Err(e) => {
            log::warn!("Failed to parse container-sessions.toml: {}", e);
            vec![]
        }
    }
}

pub fn spawn_session(
    session: &ContainerSession,
    runtime_dir: &str,
    display: &str,
) -> Option<Child> {
    let child_path = build_child_path();
    let waypipe = resolve_command_path(
        "waypipe",
        session.waypipe_path.as_deref(),
        "waypipe",
        &child_path,
    )?;
    let runtime_kind = normalize_container_runtime(&session.runtime);
    let runtime_binary = resolve_command_path(
        runtime_binary_name(&session.runtime),
        None,
        runtime_binary_name(&session.runtime),
        &child_path,
    )?;
    let host_socket = session
        .socket
        .clone()
        .unwrap_or_else(|| default_container_socket(runtime_dir, &session.name));
    let container_socket = session
        .container_socket
        .clone()
        .unwrap_or_else(|| default_container_socket_path(&host_socket, runtime_kind));

    prepare_host_socket(&host_socket)?;

    let command = session_command(session);
    let server_command = build_server_command(&container_socket, &command);
    let mut cmd = Command::new(&runtime_binary);
    cmd.env("PATH", &child_path);

    match runtime_kind {
        ContainerRuntime::Apple => {
            cmd.arg("run").arg("--rm");
            for env in &session.env {
                cmd.arg("--env").arg(env);
            }
            for arg in &session.runtime_args {
                cmd.arg(arg);
            }
            cmd.arg("--publish-socket")
                .arg(format!("{}:{}", host_socket, container_socket))
                .arg(&session.image)
                .args(["sh", "-lc", &server_command]);
        }
        ContainerRuntime::Docker | ContainerRuntime::OrbStack => {
            let socket_parent = Path::new(&host_socket).parent()?;
            cmd.arg("run").arg("--rm");
            for env in &session.env {
                cmd.arg("--env").arg(env);
            }
            for arg in &session.runtime_args {
                cmd.arg(arg);
            }
            cmd.arg("-v")
                .arg(format!(
                    "{}:{}",
                    socket_parent.display(),
                    socket_parent.display()
                ))
                .arg(&session.image)
                .args(["sh", "-lc", &server_command]);
        }
    }

    let mut container_child = cmd
        .spawn()
        .map_err(|e| log::error!("Failed to start container session {}: {}", session.name, e))
        .ok()?;

    if !wait_for_socket(&host_socket, Duration::from_secs(8)) {
        let _ = container_child.kill();
        log::error!(
            "Timed out waiting for container session socket at {}",
            host_socket
        );
        return None;
    }

    spawn_waypipe_client(&waypipe, &child_path, runtime_dir, display, &host_socket)
}

fn normalize_container_runtime(runtime: &str) -> ContainerRuntime {
    match runtime {
        "docker" => ContainerRuntime::Docker,
        "orb" | "orbstack" => ContainerRuntime::OrbStack,
        _ => ContainerRuntime::Apple,
    }
}

fn runtime_binary_name(runtime: &str) -> &str {
    match runtime {
        "docker" => "docker",
        "orb" | "orbstack" => "orb",
        _ => "container",
    }
}

fn session_command(session: &ContainerSession) -> String {
    if let Some(command) = session.command.as_deref() {
        return command.into();
    }

    match session.profile.as_deref() {
        Some("niri") => "niri --session".into(),
        Some("shell") => "sh".into(),
        _ => session
            .app
            .clone()
            .unwrap_or_else(|| "weston-terminal".into()),
    }
}

fn default_container_socket(runtime_dir: &str, name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if slug.chars().last() != Some('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("container");
    }
    Path::new(runtime_dir)
        .join(format!("{}.sock", slug))
        .to_string_lossy()
        .into_owned()
}

fn default_container_socket_path(host_socket: &str, runtime: ContainerRuntime) -> String {
    match runtime {
        ContainerRuntime::Apple => "/tmp/cocoa-way/waypipe.sock".into(),
        ContainerRuntime::Docker | ContainerRuntime::OrbStack => host_socket.into(),
    }
}

fn prepare_host_socket(host_socket: &str) -> Option<()> {
    let parent = Path::new(host_socket).parent()?;
    std::fs::create_dir_all(parent)
        .map_err(|e| {
            log::error!(
                "Failed to create socket directory {}: {}",
                parent.display(),
                e
            )
        })
        .ok()?;
    let _ = std::fs::remove_file(host_socket);
    Some(())
}

fn build_server_command(container_socket: &str, command: &str) -> String {
    let container_socket = shell_single_quote(container_socket);
    let command = shell_single_quote(command);
    format!(
        "mkdir -p $(dirname {socket}) && exec waypipe --socket {socket} server sh -lc {command}",
        socket = container_socket,
        command = command,
    )
}

fn wait_for_socket(host_socket: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if Path::new(host_socket).exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn spawn_waypipe_client(
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
        .map_err(|e| log::error!("Failed to spawn waypipe client: {}", e))
        .ok()
}
