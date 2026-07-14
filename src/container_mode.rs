use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use objc2::declare_class;
use objc2::mutability::MainThreadOnly;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{ClassType, DeclaredClass, msg_send, msg_send_id, sel};
use objc2_app_kit::{
    NSAlert, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSColor, NSFont, NSModalResponseOK,
    NSOpenPanel, NSPasteboard, NSPasteboardTypeString, NSPopUpButton, NSScrollView,
    NSSecureTextField, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use crate::container_sessions::{self, ContainerSession};
use crate::messages::CompositorMessage;
use crate::runtime_paths::{build_child_path, find_command_path, shell_single_quote};

static SENDER: Mutex<Option<Sender<CompositorMessage>>> = Mutex::new(None);
static WINDOW: Mutex<Option<usize>> = Mutex::new(None);
static HANDLER: Mutex<Option<usize>> = Mutex::new(None);
static SELECTED_NAV: Mutex<usize> = Mutex::new(0);
static SELECTED_TAB: Mutex<usize> = Mutex::new(0);
static SELECTED_SESSION: Mutex<Option<usize>> = Mutex::new(None);
static ACTIVITY: Mutex<Vec<String>> = Mutex::new(Vec::new());
static SESSION_STATES: Mutex<Vec<(usize, SessionState)>> = Mutex::new(Vec::new());
static SESSION_LOGS: Mutex<Vec<(usize, Vec<String>)>> = Mutex::new(Vec::new());
static IMAGE_TASK_ACTIVE: Mutex<Option<String>> = Mutex::new(None);
static IMAGE_TASK_DETAIL: Mutex<Option<String>> = Mutex::new(None);
static PENDING_PULL_SESSION: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static LAST_STREAM_REBUILD: Mutex<Option<Instant>> = Mutex::new(None);
static LAST_PERFORMANCE_REBUILD: Mutex<Option<Instant>> = Mutex::new(None);
static LAST_RESIZE_REBUILD: Mutex<Option<Instant>> = Mutex::new(None);
static IMAGE_CREATE_ACTIONS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static IMAGE_DELETE_ACTIONS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static IMAGE_SELECT_ACTIONS: Mutex<Vec<SelectedImage>> = Mutex::new(Vec::new());
static VOLUME_DELETE_ACTIONS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static VOLUME_SELECT_ACTIONS: Mutex<Vec<SelectedVolume>> = Mutex::new(Vec::new());
static SELECTED_IMAGE: Mutex<Option<SelectedImage>> = Mutex::new(None);
static SELECTED_VOLUME: Mutex<Option<SelectedVolume>> = Mutex::new(None);
static RUNTIME_CONTAINER_ACTIONS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static RUNTIME_CONTAINER_SELECT_ACTIONS: Mutex<Vec<SelectedRuntimeContainer>> =
    Mutex::new(Vec::new());
static SELECTED_RUNTIME_CONTAINER: Mutex<Option<SelectedRuntimeContainer>> = Mutex::new(None);
static RUNTIME_CONTAINER_DETAILS: Mutex<Option<RuntimeContainerDetails>> = Mutex::new(None);
static DOCKER_CONTEXT_ACTIONS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static PERFORMANCE: Mutex<Option<PerformanceSnapshot>> = Mutex::new(None);
static ACTIVE_SESSIONS: Mutex<Vec<ActiveSessionSnapshot>> = Mutex::new(Vec::new());
static MANAGED_DISPLAYS: Mutex<Vec<ManagedDisplaySnapshot>> = Mutex::new(Vec::new());
static PENDING_MANAGED_DISPLAYS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static MANAGED_DISPLAY_LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
static MANAGED_DISPLAY_ACTIONS: Mutex<Vec<ManagedDisplaySnapshot>> = Mutex::new(Vec::new());
static RUNTIME_FPS_LABEL: Mutex<Option<usize>> = Mutex::new(None);
static APPLE_COMPATIBILITY_CACHE: Mutex<Option<(Instant, AppleContainerCompatibility)>> =
    Mutex::new(None);

const NAV_SESSIONS: usize = 0;
const NAV_IMAGES: usize = 1;
const NAV_VOLUMES: usize = 2;
const NAV_DISPLAYS: usize = 3;
const NAV_APPLE_CONTAINER: usize = 4;
const NAV_DOCKER: usize = 5;
const NAV_ORBSTACK: usize = 6;
const NAV_ACTIVITY: usize = 7;
const NAV_COMMANDS: usize = 8;

#[derive(Clone)]
struct SessionState {
    label: &'static str,
    detail: String,
}

#[derive(Clone, PartialEq, Eq)]
struct ActiveSessionSnapshot {
    index: usize,
    container_pid: Option<u32>,
    waypipe_pid: u32,
    display_slot: String,
    display_pid: Option<u32>,
}

#[derive(Clone, PartialEq, Eq)]
struct ManagedDisplaySnapshot {
    slot: String,
    runtime_dir: String,
    display: String,
    pid: u32,
}

#[derive(Clone)]
struct SelectedImage {
    runtime: String,
    runtime_key: String,
    reference: String,
    label: String,
}

#[derive(Clone)]
struct SelectedVolume {
    runtime: String,
    runtime_key: String,
    name: String,
    label: String,
}

#[derive(Clone, PartialEq, Eq)]
struct SelectedRuntimeContainer {
    runtime: String,
    name: String,
    label: String,
    running: bool,
}

#[derive(Clone)]
struct RuntimeContainerDetails {
    runtime: String,
    name: String,
    info: Vec<String>,
    logs: Vec<String>,
    stats: Vec<String>,
    error: Option<String>,
}

#[derive(Clone)]
struct PerformanceSnapshot {
    redraw_fps: f64,
    commits_per_second: f64,
    tiles: usize,
    dirty: bool,
    pending_frame_callbacks: usize,
    late_redraws_per_second: f64,
    max_redraw_wait_ms: f64,
    input_to_present_ms: Option<f64>,
}

#[derive(Clone)]
struct AppleContainerCompatibility {
    cli_version: String,
    api_version: String,
    system_status: String,
    publish_socket: bool,
    stats_json: bool,
    summary: String,
    detail: String,
}

#[derive(Default)]
struct AddSessionDefaults {
    name: String,
    runtime: String,
    display: String,
    profile: String,
    image: String,
    command: String,
    mounts: String,
    env: String,
}

fn send(msg: CompositorMessage) {
    if let Ok(g) = SENDER.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(msg);
        }
    }
}

fn remember_launch_request(index: usize) {
    let sessions = container_sessions::load_sessions();
    let message = match sessions.get(index) {
        Some(session) => {
            clear_session_logs(index);
            set_session_state(
                index,
                "Starting",
                format!(
                    "Launching {} through {} with command: {}",
                    session.name,
                    runtime_label(&session.runtime),
                    session_display_command(session)
                ),
            );
            format!(
                "Launch requested: {} via {} ({})",
                session.name,
                runtime_label(&session.runtime),
                session_display_command(session)
            )
        }
        None => format!("Launch requested for missing session #{}", index + 1),
    };
    push_activity(message);
}

fn push_activity(message: String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let mut activity = ACTIVITY.lock().unwrap();
    activity.push(format!("[{}] {}", now, message));
    if activity.len() > 50 {
        let overflow = activity.len() - 50;
        activity.drain(0..overflow);
    }
}

fn set_image_task_active(message: impl Into<String>) {
    *IMAGE_TASK_ACTIVE.lock().unwrap() = Some(message.into());
    *IMAGE_TASK_DETAIL.lock().unwrap() = None;
}

fn clear_image_task_active() {
    *IMAGE_TASK_ACTIVE.lock().unwrap() = None;
    *IMAGE_TASK_DETAIL.lock().unwrap() = None;
}

fn set_image_task_detail(message: impl Into<String>) {
    *IMAGE_TASK_DETAIL.lock().unwrap() = Some(message.into());
}

fn image_task_active() -> Option<(String, Option<String>)> {
    IMAGE_TASK_ACTIVE
        .lock()
        .unwrap()
        .clone()
        .map(|message| (message, IMAGE_TASK_DETAIL.lock().unwrap().clone()))
}

pub fn record_performance_snapshot(
    redraw_fps: f64,
    commits_per_second: f64,
    tiles: usize,
    dirty: bool,
    pending_frame_callbacks: usize,
    late_redraws_per_second: f64,
    max_redraw_wait_ms: f64,
    input_to_present_ms: Option<f64>,
) {
    *PERFORMANCE.lock().unwrap() = Some(PerformanceSnapshot {
        redraw_fps,
        commits_per_second,
        tiles,
        dirty,
        pending_frame_callbacks,
        late_redraws_per_second,
        max_redraw_wait_ms,
        input_to_present_ms,
    });
    let should_refresh = SELECTED_NAV
        .lock()
        .map(|nav| *nav == NAV_ACTIVITY)
        .unwrap_or(false);
    if should_refresh {
        unsafe {
            refresh_window_without_focus_throttled(Duration::from_secs(2));
        }
    } else {
        unsafe {
            update_runtime_fps_label();
        }
    }
}

fn performance_snapshot() -> Option<PerformanceSnapshot> {
    PERFORMANCE.lock().unwrap().clone()
}

pub(crate) fn control_performance_snapshot()
-> Option<(f64, f64, usize, bool, usize, f64, f64, Option<f64>)> {
    performance_snapshot().map(|snapshot| {
        (
            snapshot.redraw_fps,
            snapshot.commits_per_second,
            snapshot.tiles,
            snapshot.dirty,
            snapshot.pending_frame_callbacks,
            snapshot.late_redraws_per_second,
            snapshot.max_redraw_wait_ms,
            snapshot.input_to_present_ms,
        )
    })
}

pub fn record_active_container_sessions(
    sessions: Vec<(usize, Option<u32>, u32, String, Option<u32>)>,
) {
    let snapshots = sessions
        .into_iter()
        .map(
            |(index, container_pid, waypipe_pid, display_slot, display_pid)| {
                ActiveSessionSnapshot {
                    index,
                    container_pid,
                    waypipe_pid,
                    display_slot,
                    display_pid,
                }
            },
        )
        .collect::<Vec<_>>();
    let changed = {
        let mut active = ACTIVE_SESSIONS.lock().unwrap();
        if *active == snapshots {
            false
        } else {
            *active = snapshots;
            true
        }
    };
    if !changed {
        return;
    }
    unsafe {
        refresh_window_without_focus_throttled(Duration::from_millis(500));
    }
}

pub fn record_managed_display_starting(slot: &str) {
    {
        let mut pending = PENDING_MANAGED_DISPLAYS.lock().unwrap();
        if !pending.iter().any(|candidate| candidate == slot) {
            pending.push(slot.into());
        }
    }
    *MANAGED_DISPLAY_LAST_ERROR.lock().unwrap() = None;
    push_activity(format!("Creating managed display: {}", slot));
    unsafe {
        refresh_window_without_focus_throttled(Duration::from_millis(100));
    }
}

pub fn record_managed_displays(displays: Vec<(String, String, String, u32)>) {
    let snapshots = displays
        .into_iter()
        .map(|(slot, runtime_dir, display, pid)| ManagedDisplaySnapshot {
            slot,
            runtime_dir,
            display,
            pid,
        })
        .collect::<Vec<_>>();
    let changed = {
        let mut current = MANAGED_DISPLAYS.lock().unwrap();
        if *current == snapshots {
            false
        } else {
            *current = snapshots.clone();
            true
        }
    };
    if !changed {
        return;
    }
    let active_slots = snapshots
        .iter()
        .map(|display| display.slot.as_str())
        .collect::<Vec<_>>();
    PENDING_MANAGED_DISPLAYS
        .lock()
        .unwrap()
        .retain(|slot| !active_slots.contains(&slot.as_str()));
    unsafe {
        refresh_window_without_focus_throttled(Duration::from_millis(100));
    }
}

pub fn record_managed_display_failure(slot: &str, error: &str) {
    PENDING_MANAGED_DISPLAYS
        .lock()
        .unwrap()
        .retain(|candidate| candidate != slot);
    let message = format!("Managed display '{}': {}", slot, error);
    *MANAGED_DISPLAY_LAST_ERROR.lock().unwrap() = Some(message.clone());
    push_activity(message);
    unsafe {
        refresh_window_without_focus_throttled(Duration::from_millis(100));
    }
}

pub fn record_managed_display_exit(slot: &str, reason: &str) {
    PENDING_MANAGED_DISPLAYS
        .lock()
        .unwrap()
        .retain(|candidate| candidate != slot);
    push_activity(format!("Managed display '{}' closed: {}", slot, reason));
    unsafe {
        refresh_window_without_focus_throttled(Duration::from_millis(100));
    }
}

fn managed_displays_snapshot() -> Vec<ManagedDisplaySnapshot> {
    MANAGED_DISPLAYS.lock().unwrap().clone()
}

fn pending_managed_displays_snapshot() -> Vec<String> {
    PENDING_MANAGED_DISPLAYS.lock().unwrap().clone()
}

fn active_session(index: usize) -> Option<ActiveSessionSnapshot> {
    ACTIVE_SESSIONS
        .lock()
        .unwrap()
        .iter()
        .find(|session| session.index == index)
        .cloned()
}

fn active_sessions_snapshot() -> Vec<ActiveSessionSnapshot> {
    ACTIVE_SESSIONS.lock().unwrap().clone()
}

pub(crate) fn control_active_sessions() -> Vec<(usize, Option<u32>, u32, String, Option<u32>)> {
    active_sessions_snapshot()
        .into_iter()
        .map(|session| {
            (
                session.index,
                session.container_pid,
                session.waypipe_pid,
                session.display_slot,
                session.display_pid,
            )
        })
        .collect()
}

pub(crate) fn control_session_state(index: usize) -> Option<(String, String)> {
    SESSION_STATES
        .lock()
        .unwrap()
        .iter()
        .find(|(stored, _)| *stored == index)
        .map(|(_, state)| (state.label.to_string(), state.detail.clone()))
}

fn active_display_session_count(sessions: &[ContainerSession]) -> usize {
    ACTIVE_SESSIONS
        .lock()
        .unwrap()
        .iter()
        .filter(|active| active.display_slot == "default" && sessions.get(active.index).is_some())
        .count()
}

fn active_display_conflict(index: usize) -> Option<String> {
    let sessions = container_sessions::load_sessions();
    let requested = sessions.get(index)?;
    let active_sessions = ACTIVE_SESSIONS.lock().unwrap();
    let default_in_use = active_sessions
        .iter()
        .any(|active| active.index != index && active.display_slot == "default");
    let requested_target = session_display_target(requested);
    let requested_slot = match requested_target.as_str() {
        "auto" if !default_in_use => "default".to_string(),
        "auto" | "dedicated" => {
            format!("session-{}", display_slot_slug(&requested.name))
        }
        "default" => "default".to_string(),
        named => display_slot_slug(named),
    };
    active_sessions
        .iter()
        .find(|active| active.index != index && active.display_slot == requested_slot)
        .and_then(|active| {
            sessions
                .get(active.index)
                .map(|session| session.name.clone())
        })
}

unsafe fn rebuild_window_throttled(interval: Duration) {
    let now = Instant::now();
    let mut last = LAST_STREAM_REBUILD.lock().unwrap();
    let should_rebuild = last
        .map(|previous| now.duration_since(previous) >= interval)
        .unwrap_or(true);
    if should_rebuild {
        *last = Some(now);
        unsafe {
            rebuild_window();
        }
    }
}

unsafe fn refresh_window_without_focus_throttled(interval: Duration) {
    let now = Instant::now();
    let mut last = LAST_PERFORMANCE_REBUILD.lock().unwrap();
    let should_rebuild = last
        .map(|previous| now.duration_since(previous) >= interval)
        .unwrap_or(true);
    if should_rebuild {
        *last = Some(now);
        unsafe {
            refresh_window_without_focus();
        }
    }
}

unsafe fn refresh_window_for_resize(interval: Duration) {
    let now = Instant::now();
    let mut last = LAST_RESIZE_REBUILD.lock().unwrap();
    if last
        .map(|previous| now.duration_since(previous) < interval)
        .unwrap_or(false)
    {
        return;
    }
    *last = Some(now);
    unsafe {
        refresh_window_without_focus();
    }
}

fn activity_snapshot() -> Vec<String> {
    ACTIVITY.lock().unwrap().clone()
}

pub(crate) fn control_activity_snapshot(limit: usize) -> Vec<String> {
    let activity = ACTIVITY.lock().unwrap();
    let start = activity.len().saturating_sub(limit);
    activity[start..].to_vec()
}

fn push_session_log(index: usize, source: &str, line: &str) {
    let mut logs = SESSION_LOGS.lock().unwrap();
    let formatted = format!("[{}] {}", source, clean_session_log_line(line));
    if let Some((_, lines)) = logs.iter_mut().find(|(stored, _)| *stored == index) {
        lines.push(formatted);
        if lines.len() > 200 {
            let overflow = lines.len() - 200;
            lines.drain(0..overflow);
        }
    } else {
        logs.push((index, vec![formatted]));
    }
}

fn clean_session_log_line(line: &str) -> String {
    let stripped = strip_ansi_sequences(line);
    let collapsed = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    if is_niri_locale_warning(&collapsed) {
        "niri: locale1 watcher is unavailable in this container; this is non-fatal when the desktop is running.".into()
    } else {
        collapsed
    }
}

fn is_niri_locale_warning(line: &str) -> bool {
    line.contains("niri::dbus")
        && line.contains("locale1 watcher")
        && line.contains("No such file or directory")
}

fn strip_ansi_sequences(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.peek(), Some('[')) {
                chars.next();
                while let Some(next) = chars.next() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }

        if ch == '[' {
            let mut code = String::new();
            while matches!(chars.peek(), Some(next) if next.is_ascii_digit() || *next == ';') {
                if let Some(next) = chars.next() {
                    code.push(next);
                }
            }
            if !code.is_empty() && matches!(chars.peek(), Some('m')) {
                chars.next();
                continue;
            }
            output.push('[');
            output.push_str(&code);
            continue;
        }

        output.push(ch);
    }

    output
}

fn clear_session_logs(index: usize) {
    let mut logs = SESSION_LOGS.lock().unwrap();
    if let Some((_, lines)) = logs.iter_mut().find(|(stored, _)| *stored == index) {
        lines.clear();
    }
}

fn normalize_profile(profile: &str) -> Option<String> {
    let value = profile.trim().to_ascii_lowercase();
    let normalized = match value.as_str() {
        "" => return None,
        "desktop" | "niri" | "niri-desktop" => "niri",
        "app" | "single" | "single-app" => "single-app",
        "debug" | "shell" | "sh" => "shell",
        other => other,
    };
    Some(normalized.into())
}

fn session_logs(index: usize) -> Vec<String> {
    SESSION_LOGS
        .lock()
        .unwrap()
        .iter()
        .find(|(stored, _)| *stored == index)
        .map(|(_, lines)| lines.clone())
        .unwrap_or_default()
}

pub(crate) fn control_session_logs(index: usize, limit: usize) -> Vec<String> {
    let logs = session_logs(index);
    let start = logs.len().saturating_sub(limit);
    logs[start..].to_vec()
}

fn smoke_image_reference() -> &'static str {
    "localhost/cocoa-way-niri:latest"
}

fn smoke_containerfile_path() -> &'static str {
    "examples/container-images/Containerfile.niri"
}

fn smoke_build_context() -> &'static str {
    "."
}

fn default_gui_runtime_args(runtime: &str, profile: Option<&str>) -> Vec<String> {
    if matches!(runtime.trim(), "container" | "apple" | "apple-container") {
        let desktop = matches!(profile, Some("niri" | "desktop"));
        let (memory, shm) = if desktop {
            ("4G", "1G")
        } else {
            ("2G", "512M")
        };
        ["--memory", memory, "--shm-size", shm, "--cpus", "4"]
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    }
}

fn request_smoke_image_build() {
    if !allow_storage_growth("build the example image") {
        return;
    }
    push_activity(format!(
        "Example image build requested: {}",
        smoke_image_reference()
    ));
    send(CompositorMessage::BuildContainerImage {
        image: smoke_image_reference().into(),
        containerfile: smoke_containerfile_path().into(),
        context: smoke_build_context().into(),
    });
}

fn smoke_session() -> ContainerSession {
    ContainerSession {
        name: "Niri Desktop".into(),
        image: smoke_image_reference().into(),
        runtime: "container".into(),
        display: Some("auto".into()),
        profile: Some("niri".into()),
        app: None,
        command: Some("niri".into()),
        socket: None,
        container_socket: None,
        waypipe_path: None,
        waypipe_compress: None,
        waypipe_threads: None,
        runtime_args: default_gui_runtime_args("container", Some("niri")),
        mounts: Vec::new(),
        env: Vec::new(),
    }
}

fn add_or_select_smoke_session() {
    let sessions = container_sessions::load_sessions();
    if let Some(index) = sessions
        .iter()
        .position(|session| session.image == smoke_image_reference())
    {
        *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
        *SELECTED_SESSION.lock().unwrap() = Some(index);
        push_activity("Selected existing example session.".into());
        return;
    }

    let session = smoke_session();
    match container_sessions::append_session(&session) {
        Ok(()) => {
            let sessions = container_sessions::load_sessions();
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = sessions.len().checked_sub(1);
            push_activity(format!("Restored example session: {}", session.image));
        }
        Err(error) => {
            let message = format!("Failed to restore example session: {}", error);
            push_activity(message.clone());
            show_error_alert(&message);
        }
    }
}

fn set_session_state(index: usize, label: &'static str, detail: String) {
    let mut states = SESSION_STATES.lock().unwrap();
    if let Some((_, state)) = states.iter_mut().find(|(stored, _)| *stored == index) {
        state.label = label;
        state.detail = detail;
    } else {
        states.push((index, SessionState { label, detail }));
    }
}

fn session_state(index: usize) -> Option<SessionState> {
    SESSION_STATES
        .lock()
        .unwrap()
        .iter()
        .find(|(stored, _)| *stored == index)
        .map(|(_, state)| state.clone())
}

fn apple_container_gui_transport_ready() -> bool {
    true
}

fn session_has_apple_transport_block(session: &ContainerSession) -> bool {
    container_sessions::is_apple_container_session(session)
        && !apple_container_gui_transport_ready()
}

fn apple_transport_blocked_detail(session: &ContainerSession) -> String {
    format!(
        "{} uses Apple Container. Image and volume management are wired, but GUI launch needs a dedicated Apple Container transport before this profile can start.",
        session.name
    )
}

fn session_can_stop(state: Option<&SessionState>) -> bool {
    state
        .map(|state| matches!(state.label, "Starting" | "Running" | "Stopping"))
        .unwrap_or(false)
}

fn session_is_launch_busy(state: Option<&SessionState>) -> bool {
    state
        .map(|state| matches!(state.label, "Starting" | "Running" | "Stopping"))
        .unwrap_or(false)
}

pub fn record_launch_success(index: usize, report: &container_sessions::LaunchReport) {
    let sessions = container_sessions::load_sessions();
    let name = sessions
        .get(index)
        .map(|session| session.name.as_str())
        .unwrap_or("Unknown session");
    let detail = format!(
        "{} is running. Runtime: {}; container: {}; command: {}; host socket: {}; container socket: {}; waypipe pid: {}",
        name,
        report.runtime,
        report.container_name,
        report.command,
        report.host_socket,
        report.container_socket,
        report.waypipe_child.id()
    );
    set_session_state(index, "Running", detail.clone());
    push_activity(format!("Started: {}", detail));
    unsafe {
        rebuild_window();
    }
}

pub fn record_launch_already_running(index: usize) {
    let sessions = container_sessions::load_sessions();
    let detail = sessions
        .get(index)
        .map(|session| {
            format!(
                "{} is already running. Stop it before launching again.",
                session.name
            )
        })
        .unwrap_or_else(|| format!("Session #{} is already running.", index + 1));
    set_session_state(index, "Running", detail.clone());
    push_activity(format!("Launch ignored: {}", detail));
    unsafe {
        rebuild_window();
    }
}

pub fn record_launch_blocked(index: usize, detail: &str) {
    *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
    *SELECTED_SESSION.lock().unwrap() = Some(index);
    set_session_state(index, "Blocked", detail.into());
    push_activity(format!("Launch blocked: {}", detail));
    unsafe {
        rebuild_window();
    }
}

pub fn record_check_success(index: usize, report: &container_sessions::CheckReport) {
    let sessions = container_sessions::load_sessions();
    let name = sessions
        .get(index)
        .map(|session| session.name.as_str())
        .unwrap_or("Unknown session");
    let status = if report.running { "running" } else { "ready" };
    let detail = format!(
        "{} is {}. Runtime: {}; container: {}; image: {}; command: {}; waypipe: {}; runtime binary: {}",
        name,
        status,
        report.runtime,
        report.container_name,
        report.image,
        report.command,
        report.waypipe,
        report.runtime_binary
    );
    set_session_state(
        index,
        if report.running { "Running" } else { "Ready" },
        detail.clone(),
    );
    push_activity(format!("Check passed: {}", detail));
    unsafe {
        rebuild_window();
    }
}

pub fn record_check_failure(index: usize, error: &container_sessions::LaunchError) {
    let sessions = container_sessions::load_sessions();
    let name = sessions
        .get(index)
        .map(|session| session.name.as_str())
        .unwrap_or("Unknown session");
    let detail = format!("{} check failed: {}", name, error);
    *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
    *SELECTED_SESSION.lock().unwrap() = Some(index);
    let label = if error.is_container_already_running() {
        "Running"
    } else if error.is_unsupported_display() {
        "Blocked"
    } else {
        "Error"
    };
    set_session_state(index, label, detail.clone());
    push_activity(detail);
    unsafe {
        rebuild_window();
    }
}

pub fn record_launch_failure(index: usize, error: &container_sessions::LaunchError) {
    let sessions = container_sessions::load_sessions();
    let name = sessions
        .get(index)
        .map(|session| session.name.as_str())
        .unwrap_or("Unknown session");
    let detail = format!("{} failed to start: {}", name, error);
    *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
    *SELECTED_SESSION.lock().unwrap() = Some(index);
    let label = if error.is_container_already_running() {
        "Running"
    } else if error.is_unsupported_display() {
        "Blocked"
    } else {
        "Error"
    };
    set_session_state(index, label, detail.clone());
    push_activity(detail);
    unsafe {
        rebuild_window();
    }
}

pub fn record_session_log(index: usize, source: &str, line: &str) {
    push_session_log(index, source, line);
    push_activity(format!("{}: {}", source, clean_session_log_line(line)));
    unsafe {
        rebuild_window_throttled(Duration::from_millis(500));
    }
}

pub fn record_image_pull_started(runtime: &str, image: &str, configure_session: bool) {
    set_image_task_active(format!("Pulling {} image {}...", runtime, image));
    if configure_session {
        PENDING_PULL_SESSION
            .lock()
            .unwrap()
            .push((runtime.to_string(), image.to_string()));
    }
    push_activity(format!("Pull started: {} image {}", runtime, image));
    unsafe {
        rebuild_window();
    }
}

pub fn record_image_pull_log(runtime: &str, image: &str, line: &str) {
    let line = clean_session_log_line(line);
    if !line.is_empty() {
        set_image_task_detail(line.clone());
    }
    push_activity(format!("pull {} {}: {}", runtime, image, line));
    unsafe {
        rebuild_window_throttled(Duration::from_millis(500));
    }
}

pub fn record_image_pull_finished(runtime: &str, image: &str, success: bool, status: &str) {
    let configure_session = {
        let mut pending = PENDING_PULL_SESSION.lock().unwrap();
        pending
            .iter()
            .position(|(pending_runtime, pending_image)| {
                pending_runtime == runtime && pending_image == image
            })
            .map(|index| pending.remove(index))
            .is_some()
    };
    clear_image_task_active();
    let state = if success { "finished" } else { "failed" };
    push_activity(format!(
        "Pull {}: {} image {} ({})",
        state, runtime, image, status
    ));
    unsafe {
        rebuild_window();
        if success && configure_session {
            show_session_dialog_for_image(runtime, image);
        }
    }
}

pub fn record_image_load_started(path: &str) {
    set_image_task_active(format!("Loading OCI archive {}...", path));
    push_activity(format!("Image load started: {}", path));
    unsafe {
        rebuild_window();
    }
}

pub fn record_image_load_log(path: &str, line: &str) {
    push_activity(format!("load {}: {}", path, line));
    unsafe {
        rebuild_window_throttled(Duration::from_millis(500));
    }
}

pub fn record_image_load_finished(path: &str, success: bool, status: &str) {
    clear_image_task_active();
    let state = if success { "finished" } else { "failed" };
    push_activity(format!("Image load {}: {} ({})", state, path, status));
    unsafe {
        rebuild_window();
    }
}

pub fn record_image_build_started(image: &str, containerfile: &str) {
    set_image_task_active(format!("Building {} from {}...", image, containerfile));
    push_activity(format!(
        "Image build started: {} from {}",
        image, containerfile
    ));
    unsafe {
        rebuild_window();
    }
}

pub fn record_image_build_log(image: &str, line: &str) {
    push_activity(format!("build {}: {}", image, line));
    unsafe {
        rebuild_window_throttled(Duration::from_millis(500));
    }
}

pub fn record_image_build_finished(image: &str, success: bool, status: &str) {
    clear_image_task_active();
    let state = if success { "finished" } else { "failed" };
    push_activity(format!("Image build {}: {} ({})", state, image, status));
    unsafe {
        rebuild_window();
    }
}

pub fn record_storage_growth_blocked(action: &str, error: &str) {
    clear_image_task_active();
    push_activity(format!("Storage protection blocked {}: {}", action, error));
    unsafe {
        rebuild_window();
    }
}

fn allow_storage_growth(action: &str) -> bool {
    match crate::diagnostics::ensure_storage_growth_allowed() {
        Ok(_) => true,
        Err(error) => {
            record_storage_growth_blocked(action, &error);
            show_error_alert(&error);
            false
        }
    }
}

pub fn record_apple_container_system_start_started() {
    push_activity("Apple Container system start requested.".into());
    unsafe {
        rebuild_window();
    }
}

pub fn record_apple_container_system_start_log(line: &str) {
    push_activity(format!("container system start: {}", line));
    unsafe {
        rebuild_window();
    }
}

pub fn record_apple_container_system_start_finished(success: bool, status: &str) {
    let state = if success { "finished" } else { "failed" };
    push_activity(format!(
        "Apple Container system start {} ({})",
        state, status
    ));
    unsafe {
        rebuild_window();
    }
}

pub fn record_image_delete_started(runtime: &str, image: &str) {
    set_image_task_active(format!("Deleting {} image {}...", runtime, image));
    push_activity(format!("Image delete started: {} {}", runtime, image));
    unsafe {
        rebuild_window();
    }
}

pub fn record_image_delete_log(_runtime: &str, image: &str, line: &str) {
    push_activity(format!("delete image {}: {}", image, line));
    unsafe {
        rebuild_window_throttled(Duration::from_millis(500));
    }
}

pub fn record_image_delete_finished(_runtime: &str, image: &str, success: bool, status: &str) {
    clear_image_task_active();
    let state = if success { "finished" } else { "failed" };
    push_activity(format!("Image delete {}: {} ({})", state, image, status));
    unsafe {
        rebuild_window();
    }
}

pub fn record_volume_delete_started(runtime: &str, volume: &str) {
    push_activity(format!("Volume delete started: {} {}", runtime, volume));
    unsafe {
        rebuild_window();
    }
}

pub fn record_volume_delete_log(_runtime: &str, volume: &str, line: &str) {
    push_activity(format!("delete volume {}: {}", volume, line));
    unsafe {
        rebuild_window();
    }
}

pub fn record_volume_delete_finished(_runtime: &str, volume: &str, success: bool, status: &str) {
    let state = if success { "finished" } else { "failed" };
    push_activity(format!("Volume delete {}: {} ({})", state, volume, status));
    unsafe {
        rebuild_window();
    }
}

pub fn record_volume_create_started(runtime: &str, volume: &str) {
    push_activity(format!("Volume create started: {} {}", runtime, volume));
    unsafe {
        rebuild_window();
    }
}

pub fn record_volume_create_log(_runtime: &str, volume: &str, line: &str) {
    push_activity(format!("create volume {}: {}", volume, line));
    unsafe {
        rebuild_window_throttled(Duration::from_millis(500));
    }
}

pub fn record_volume_create_finished(_runtime: &str, volume: &str, success: bool, status: &str) {
    let state = if success { "finished" } else { "failed" };
    push_activity(format!("Volume create {}: {} ({})", state, volume, status));
    unsafe {
        rebuild_window();
    }
}

pub fn record_runtime_container_action_started(runtime: &str, name: &str, action: &str) {
    push_activity(format!(
        "{} container {} started: {}",
        runtime_label(runtime),
        action,
        name
    ));
    unsafe {
        rebuild_window();
    }
}

pub fn record_runtime_container_action_log(runtime: &str, name: &str, action: &str, line: &str) {
    push_activity(format!(
        "{} {} {}: {}",
        runtime_label(runtime),
        action,
        name,
        line
    ));
    unsafe {
        rebuild_window_throttled(Duration::from_millis(500));
    }
}

pub fn record_runtime_container_action_finished(
    runtime: &str,
    name: &str,
    action: &str,
    success: bool,
    status: &str,
) {
    let state = if success { "finished" } else { "failed" };
    push_activity(format!(
        "{} container {} {}: {} ({})",
        runtime_label(runtime),
        action,
        state,
        name,
        status
    ));
    let selected_matches = SELECTED_RUNTIME_CONTAINER
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|selected| selected.runtime == runtime && selected.name == name);
    if selected_matches && success && action == "delete" {
        *SELECTED_RUNTIME_CONTAINER.lock().unwrap() = None;
        *RUNTIME_CONTAINER_DETAILS.lock().unwrap() = None;
    } else if selected_matches {
        request_selected_runtime_container_details();
    }
    unsafe {
        rebuild_window();
    }
}

pub fn record_runtime_container_details_loaded(
    runtime: &str,
    name: &str,
    info: Vec<String>,
    logs: Vec<String>,
    stats: Vec<String>,
    error: Option<String>,
) {
    let selected_matches = SELECTED_RUNTIME_CONTAINER
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|selected| selected.runtime == runtime && selected.name == name);
    if !selected_matches {
        return;
    }
    let clean_lines = |lines: Vec<String>| {
        lines
            .into_iter()
            .map(|line| clean_session_log_line(&line))
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
    };
    *RUNTIME_CONTAINER_DETAILS.lock().unwrap() = Some(RuntimeContainerDetails {
        runtime: runtime.to_string(),
        name: name.to_string(),
        info: clean_lines(info),
        logs: clean_lines(logs),
        stats: clean_lines(stats),
        error,
    });
    unsafe {
        rebuild_window();
    }
}

pub fn record_runtime_container_terminal_opened(runtime: &str, name: &str) {
    push_activity(format!(
        "Opened a {} terminal for {}.",
        runtime_label(runtime),
        name
    ));
    unsafe {
        rebuild_window();
    }
}

pub fn record_runtime_container_terminal_failed(runtime: &str, name: &str, error: &str) {
    push_activity(format!(
        "Could not open a {} terminal for {}: {}",
        runtime_label(runtime),
        name,
        error
    ));
    show_error_alert(&format!("Could not open container terminal: {}", error));
    unsafe {
        rebuild_window();
    }
}

pub fn record_runtime_system_action_started(runtime: &str, action: &str) {
    push_activity(format!(
        "{} system {} started",
        runtime_label(runtime),
        action
    ));
    unsafe {
        rebuild_window();
    }
}

pub fn record_runtime_system_action_log(runtime: &str, action: &str, line: &str) {
    push_activity(format!(
        "{} system {}: {}",
        runtime_label(runtime),
        action,
        line
    ));
    unsafe {
        rebuild_window_throttled(Duration::from_millis(500));
    }
}

pub fn record_runtime_system_action_finished(
    runtime: &str,
    action: &str,
    success: bool,
    status: &str,
) {
    push_activity(format!(
        "{} system {} {} ({})",
        runtime_label(runtime),
        action,
        if success { "finished" } else { "failed" },
        status
    ));
    unsafe {
        rebuild_window();
    }
}

pub fn record_stop_success(index: usize) {
    let sessions = container_sessions::load_sessions();
    let name = sessions
        .get(index)
        .map(|session| session.name.as_str())
        .unwrap_or("Unknown session");
    let detail = format!("{} stopped.", name);
    set_session_state(index, "Stopped", detail.clone());
    push_activity(detail);
    unsafe {
        rebuild_window();
    }
}

pub fn record_stop_failure(index: usize, error: &str) {
    let sessions = container_sessions::load_sessions();
    let name = sessions
        .get(index)
        .map(|session| session.name.as_str())
        .unwrap_or("Unknown session");
    let detail = format!("{} stop failed: {}", name, error);
    set_session_state(index, "Error", detail.clone());
    push_activity(detail);
    unsafe {
        rebuild_window();
    }
}

pub fn record_terminal_opened(index: usize) {
    let sessions = container_sessions::load_sessions();
    let name = sessions
        .get(index)
        .map(|session| session.name.as_str())
        .unwrap_or("Unknown session");
    push_activity(format!("Terminal opened for {}.", name));
    unsafe {
        rebuild_window();
    }
}

pub fn record_terminal_open_failed(index: usize, error: &str) {
    let sessions = container_sessions::load_sessions();
    let name = sessions
        .get(index)
        .map(|session| session.name.as_str())
        .unwrap_or("Unknown session");
    let detail = format!("Terminal failed for {}: {}", name, error);
    push_activity(detail.clone());
    set_session_state(index, "Error", detail);
    unsafe {
        rebuild_window();
    }
}

pub fn record_process_exit(index: usize, process: &str, status: &str) {
    let sessions = container_sessions::load_sessions();
    let name = sessions
        .get(index)
        .map(|session| session.name.as_str())
        .unwrap_or("Unknown session");
    let detail = format!("{} exited because {} ended with {}.", name, process, status);
    set_session_state(index, "Exited", detail.clone());
    push_activity(detail);
    unsafe {
        rebuild_window();
    }
}

unsafe fn show_add_session_dialog() {
    unsafe {
        show_add_session_dialog_with_defaults(AddSessionDefaults::default());
    }
}

unsafe fn show_new_image_session_dialog() {
    unsafe {
        show_add_session_dialog_with_defaults(AddSessionDefaults {
            name: "Niri Desktop".into(),
            runtime: "container".into(),
            display: "auto".into(),
            profile: "niri".into(),
            image: smoke_image_reference().into(),
            command: "niri".into(),
            ..AddSessionDefaults::default()
        });
    }
}

unsafe fn show_add_session_dialog_with_defaults(defaults: AddSessionDefaults) {
    unsafe {
        show_session_dialog(defaults, None);
    }
}

unsafe fn show_edit_session_dialog(index: usize) {
    let sessions = container_sessions::load_sessions();
    let Some(session) = sessions.get(index).cloned() else {
        show_error_alert("Session no longer exists.");
        return;
    };
    unsafe {
        show_session_dialog(defaults_from_session(&session), Some((index, session)));
    }
}

unsafe fn duplicate_session_profile(index: usize) {
    let sessions = container_sessions::load_sessions();
    let Some(source) = sessions.get(index).cloned() else {
        show_error_alert("Session no longer exists.");
        return;
    };
    let mut duplicate = source.clone();
    duplicate.name = unique_duplicate_name(&sessions, &source.name);

    match container_sessions::append_session(&duplicate) {
        Ok(()) => {
            let sessions = container_sessions::load_sessions();
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = sessions.len().checked_sub(1);
            push_activity(format!("Duplicated session: {}", duplicate.name));
            unsafe {
                rebuild_window();
            }
        }
        Err(error) => {
            let message = format!("Failed to duplicate session: {}", error);
            push_activity(message.clone());
            show_error_alert(&message);
        }
    }
}

fn defaults_from_session(session: &ContainerSession) -> AddSessionDefaults {
    AddSessionDefaults {
        name: session.name.clone(),
        runtime: session.runtime.clone(),
        display: session.display.clone().unwrap_or_else(|| "auto".into()),
        profile: session.profile.clone().unwrap_or_else(|| "niri".into()),
        image: session.image.clone(),
        command: session
            .command
            .clone()
            .or_else(|| session.app.clone())
            .unwrap_or_default(),
        mounts: session.mounts.join("; "),
        env: session.env.join("; "),
    }
}

fn unique_duplicate_name(sessions: &[ContainerSession], name: &str) -> String {
    let base = format!("{} Copy", name);
    if !sessions.iter().any(|session| session.name == base) {
        return base;
    }

    for suffix in 2..100 {
        let candidate = format!("{} {}", base, suffix);
        if !sessions.iter().any(|session| session.name == candidate) {
            return candidate;
        }
    }

    format!(
        "{} {}",
        base,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    )
}

unsafe fn show_session_dialog(
    defaults: AddSessionDefaults,
    edit_target: Option<(usize, ContainerSession)>,
) {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let alert: Retained<NSAlert> = unsafe { msg_send_id![NSAlert::class(), new] };
    let is_edit = edit_target.is_some();
    unsafe {
        let _: () = msg_send![&*alert, setMessageText:
            &*NSString::from_str(if is_edit { "Edit GUI Session" } else { "Add GUI Session" })];
        let _: () = msg_send![&*alert, setInformativeText:
        &*NSString::from_str(if is_edit {
            "Update this Container Mode profile. Advanced socket and waypipe fields are preserved."
        } else {
            "Create a Container Mode profile backed by Apple Container, Docker, or OrbStack."
        })];
    }

    let view: Retained<NSView> =
        unsafe { msg_send_id![mtm.alloc::<NSView>(), initWithFrame: rect(0.0, 0.0, 420.0, 334.0)] };
    add_label(
        &view,
        "Name",
        rect(0.0, 305.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let name_field = add_text_field(
        &view,
        rect(116.0, 300.0, 304.0, 26.0),
        "Niri Desktop",
        &defaults.name,
        mtm,
    );
    add_label(
        &view,
        "Runtime",
        rect(0.0, 265.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let runtime_field = add_text_field(
        &view,
        rect(116.0, 260.0, 304.0, 26.0),
        "container",
        if defaults.runtime.is_empty() {
            "container"
        } else {
            &defaults.runtime
        },
        mtm,
    );
    add_label(
        &view,
        "Display",
        rect(0.0, 225.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let display_field = add_text_field(
        &view,
        rect(116.0, 220.0, 304.0, 26.0),
        "auto",
        if defaults.display.is_empty() {
            "auto"
        } else {
            &defaults.display
        },
        mtm,
    );
    add_label(
        &view,
        "Profile",
        rect(0.0, 185.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let profile_field = add_text_field(
        &view,
        rect(116.0, 180.0, 304.0, 26.0),
        "niri / single-app / shell",
        if defaults.profile.is_empty() {
            "niri"
        } else {
            &defaults.profile
        },
        mtm,
    );
    add_label(
        &view,
        "Image / Source",
        rect(0.0, 145.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let image_field = add_text_field(
        &view,
        rect(116.0, 140.0, 304.0, 26.0),
        smoke_image_reference(),
        &defaults.image,
        mtm,
    );
    add_label(
        &view,
        "Command",
        rect(0.0, 105.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let command_field = add_text_field(
        &view,
        rect(116.0, 100.0, 304.0, 26.0),
        "niri",
        &defaults.command,
        mtm,
    );
    add_label(
        &view,
        "Mounts",
        rect(0.0, 65.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let mounts_field = add_text_field(
        &view,
        rect(116.0, 60.0, 304.0, 26.0),
        "separate multiple mounts with ;",
        &defaults.mounts,
        mtm,
    );
    add_label(
        &view,
        "Env",
        rect(0.0, 25.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let env_field = add_text_field(
        &view,
        rect(116.0, 20.0, 304.0, 26.0),
        "WAYLAND_DEBUG=1; RUST_LOG=info",
        &defaults.env,
        mtm,
    );

    unsafe {
        let _: () = msg_send![&*alert, setAccessoryView: &*view];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str(if is_edit { "Save" } else { "Create" })];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Cancel")];
        let _: () = msg_send![&*alert, layout];
    }

    let response: isize = unsafe { msg_send![&*alert, runModal] };
    if response != 1000 {
        return;
    }

    let name = field_string(&name_field);
    let runtime = field_string(&runtime_field);
    let display = field_string(&display_field);
    let profile = field_string(&profile_field);
    let image = field_string(&image_field);
    let command = field_string(&command_field);
    let mounts = semicolon_list(&field_string(&mounts_field));
    let env = semicolon_list(&field_string(&env_field));
    if image.is_empty() {
        show_error_alert("Enter an image reference before creating the session.");
        return;
    }

    let runtime = if runtime.is_empty() {
        "container".to_string()
    } else {
        runtime
    };
    let profile = normalize_profile(&profile);
    let runtime_args = default_gui_runtime_args(&runtime, profile.as_deref());
    let mut session = ContainerSession {
        name: if name.is_empty() {
            default_session_name(&image)
        } else {
            name
        },
        image,
        runtime,
        display: Some(if display.is_empty() {
            "auto".into()
        } else {
            display
        }),
        profile,
        app: None,
        command: if command.is_empty() {
            None
        } else {
            Some(command)
        },
        socket: None,
        container_socket: None,
        waypipe_path: None,
        waypipe_compress: None,
        waypipe_threads: None,
        runtime_args,
        mounts,
        env,
    };

    let result = if let Some((index, original)) = edit_target {
        session.app = original.app;
        session.socket = original.socket;
        session.container_socket = original.container_socket;
        session.waypipe_path = original.waypipe_path;
        session.waypipe_compress = original.waypipe_compress;
        session.waypipe_threads = original.waypipe_threads;
        session.runtime_args = original.runtime_args;
        container_sessions::replace_session(index, &session).map(|_| Some(index))
    } else {
        container_sessions::append_session(&session).map(|_| None)
    };

    match result {
        Ok(selected_index) => {
            let sessions = container_sessions::load_sessions();
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() =
                selected_index.or_else(|| sessions.len().checked_sub(1));
            push_activity(format!(
                "{} session: {} ({})",
                if is_edit { "Updated" } else { "Added" },
                session.name,
                session.image
            ));
            unsafe {
                rebuild_window();
            }
        }
        Err(error) => {
            let message = format!(
                "Failed to {} session: {}",
                if is_edit { "update" } else { "add" },
                error
            );
            push_activity(message.clone());
            show_error_alert(&message);
        }
    }
}

unsafe fn show_pull_image_dialog() {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let alert: Retained<NSAlert> = unsafe { msg_send_id![NSAlert::class(), new] };
    unsafe {
        let _: () = msg_send![&*alert, setMessageText:
            &*NSString::from_str("Pull an Image")];
        let _: () = msg_send![&*alert, setInformativeText:
            &*NSString::from_str("Choose a registry and destination. Base images still need waypipe and a GUI command before they can open a Cocoa-Way session.")];
    }

    let view: Retained<NSView> =
        unsafe { msg_send_id![mtm.alloc::<NSView>(), initWithFrame: rect(0.0, 0.0, 420.0, 252.0)] };
    add_label(
        &view,
        "Destination",
        rect(0.0, 222.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let runtime_popup = add_popup(
        &view,
        rect(116.0, 216.0, 304.0, 28.0),
        &["Apple Container", "Docker-compatible Context"],
        0,
        mtm,
    );
    add_label(
        &view,
        "Registry",
        rect(0.0, 182.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let source_popup = add_popup(
        &view,
        rect(116.0, 176.0, 304.0, 28.0),
        &[
            "Docker Hub",
            "GitHub Container Registry",
            "Quay",
            "Custom OCI reference",
        ],
        0,
        mtm,
    );
    add_label(
        &view,
        "Reference",
        rect(0.0, 142.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let image_field = add_text_field(
        &view,
        rect(116.0, 136.0, 304.0, 26.0),
        "library/ubuntu:24.04",
        "library/ubuntu:24.04",
        mtm,
    );
    add_label(
        &view,
        "Platform",
        rect(0.0, 102.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let platform_popup = add_popup(
        &view,
        rect(116.0, 96.0, 304.0, 28.0),
        &["Native architecture", "Linux arm64", "Linux amd64"],
        0,
        mtm,
    );
    add_label(
        &view,
        "Connection",
        rect(0.0, 62.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let scheme_popup = add_popup(
        &view,
        rect(116.0, 56.0, 304.0, 28.0),
        &["Automatic", "HTTPS", "HTTP (insecure)"],
        0,
        mtm,
    );
    add_label(
        &view,
        "After pull",
        rect(0.0, 22.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let after_popup = add_popup(
        &view,
        rect(116.0, 16.0, 304.0, 28.0),
        &["Keep as a local image", "Configure a GUI session"],
        1,
        mtm,
    );

    unsafe {
        let _: () = msg_send![&*alert, setAccessoryView: &*view];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Pull")];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Cancel")];
        let _: () = msg_send![&*alert, layout];
    }

    let response: isize = unsafe { msg_send![&*alert, runModal] };
    if response != 1000 {
        return;
    }

    let reference = field_string(&image_field);
    if reference.is_empty() {
        show_error_alert("Image reference is required.");
        return;
    }
    let runtime = if popup_index(&runtime_popup) == 1 {
        "docker"
    } else {
        "container"
    };
    let image = normalize_registry_reference(popup_index(&source_popup), &reference);
    let platform = match popup_index(&platform_popup) {
        1 => Some("linux/arm64".into()),
        2 => Some("linux/amd64".into()),
        _ => None,
    };
    let scheme = match popup_index(&scheme_popup) {
        1 => Some("https".into()),
        2 => Some("http".into()),
        _ => None,
    };
    if !allow_storage_growth("pull an image") {
        return;
    }

    send(CompositorMessage::PullContainerImage {
        runtime: runtime.into(),
        image,
        platform,
        scheme,
        configure_session: popup_index(&after_popup) == 1,
    });
}

unsafe fn show_registry_login_dialog() {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let alert: Retained<NSAlert> = unsafe { msg_send_id![NSAlert::class(), new] };
    unsafe {
        let _: () = msg_send![&*alert, setMessageText:
            &*NSString::from_str("Registry Login")];
        let _: () = msg_send![&*alert, setInformativeText:
            &*NSString::from_str("Credentials are passed to Apple Container through standard input and are never added to process arguments or Cocoa-Way logs.")];
    }

    let view: Retained<NSView> =
        unsafe { msg_send_id![mtm.alloc::<NSView>(), initWithFrame: rect(0.0, 0.0, 420.0, 174.0)] };
    add_label(
        &view,
        "Registry",
        rect(0.0, 144.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let server_field = add_text_field(
        &view,
        rect(116.0, 138.0, 304.0, 26.0),
        "ghcr.io",
        "ghcr.io",
        mtm,
    );
    add_label(
        &view,
        "Username",
        rect(0.0, 104.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let username_field = add_text_field(
        &view,
        rect(116.0, 98.0, 304.0, 26.0),
        "account name",
        "",
        mtm,
    );
    add_label(
        &view,
        "Token / password",
        rect(0.0, 64.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let password_field = add_secure_text_field(
        &view,
        rect(116.0, 58.0, 304.0, 26.0),
        "personal access token",
        mtm,
    );
    add_label(
        &view,
        "Connection",
        rect(0.0, 24.0, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let scheme_popup = add_popup(
        &view,
        rect(116.0, 18.0, 304.0, 28.0),
        &["Automatic", "HTTPS", "HTTP (insecure)"],
        0,
        mtm,
    );

    unsafe {
        let _: () = msg_send![&*alert, setAccessoryView: &*view];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Login")];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Cancel")];
        let _: () = msg_send![&*alert, layout];
    }
    let response: isize = unsafe { msg_send![&*alert, runModal] };
    if response != 1000 {
        return;
    }

    let server = field_string(&server_field);
    let username = field_string(&username_field);
    let password = field_string(&password_field);
    if server.is_empty() || username.is_empty() || password.is_empty() {
        show_error_alert("Registry, username, and token/password are required.");
        return;
    }
    let scheme = match popup_index(&scheme_popup) {
        1 => Some("https".into()),
        2 => Some("http".into()),
        _ => None,
    };
    send(CompositorMessage::LoginContainerRegistry {
        server,
        username,
        password,
        scheme,
    });
}

unsafe fn show_load_image_dialog() {
    if !allow_storage_growth("load an OCI archive") {
        return;
    }
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let panel = unsafe { NSOpenPanel::openPanel(mtm) };
    unsafe {
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        let _: () = msg_send![&*panel, setTitle:
            &*NSString::from_str("Load OCI Image Archive")];
        let _: () = msg_send![&*panel, setMessage:
            &*NSString::from_str("Choose an OCI-compatible image tar archive to load into Apple Container.")];
    }

    let response = unsafe { panel.runModal() };
    if response != NSModalResponseOK {
        return;
    }

    let Some(url) = (unsafe { panel.URL() }) else {
        show_error_alert("No archive was selected.");
        return;
    };
    let Some(path) = (unsafe { url.path() }) else {
        show_error_alert("Selected archive does not have a local filesystem path.");
        return;
    };

    send(CompositorMessage::LoadContainerImage {
        path: path.to_string(),
    });
}

unsafe fn show_delete_image_dialog() {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let alert: Retained<NSAlert> = unsafe { msg_send_id![NSAlert::class(), new] };
    unsafe {
        let _: () = msg_send![&*alert, setMessageText:
            &*NSString::from_str("Delete Image")];
        let _: () = msg_send![&*alert, setInformativeText:
            &*NSString::from_str("Delete a local image from Apple Container or Docker. Running containers are not stopped by this action.")];
    }

    let view: Retained<NSView> =
        unsafe { msg_send_id![mtm.alloc::<NSView>(), initWithFrame: rect(0.0, 0.0, 380.0, 94.0)] };
    add_label(
        &view,
        "Runtime",
        rect(0.0, 65.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let runtime_field = add_text_field(
        &view,
        rect(116.0, 60.0, 264.0, 26.0),
        "container",
        "container",
        mtm,
    );
    add_label(
        &view,
        "Image",
        rect(0.0, 25.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let image_field = add_text_field(
        &view,
        rect(116.0, 20.0, 264.0, 26.0),
        smoke_image_reference(),
        "",
        mtm,
    );

    unsafe {
        let _: () = msg_send![&*alert, setAccessoryView: &*view];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Delete")];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Cancel")];
        let _: () = msg_send![&*alert, layout];
    }

    let response: isize = unsafe { msg_send![&*alert, runModal] };
    if response != 1000 {
        return;
    }

    let runtime = field_string(&runtime_field);
    let image = field_string(&image_field);
    if image.is_empty() {
        show_error_alert("Image is required.");
        return;
    }

    send(CompositorMessage::DeleteContainerImage {
        runtime: if runtime.is_empty() {
            "container".into()
        } else {
            runtime
        },
        image,
    });
}

unsafe fn show_create_volume_dialog() {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let alert: Retained<NSAlert> = unsafe { msg_send_id![NSAlert::class(), new] };
    unsafe {
        let _: () = msg_send![&*alert, setMessageText:
            &*NSString::from_str("Create Volume")];
        let _: () = msg_send![&*alert, setInformativeText:
            &*NSString::from_str("Create a named volume in Apple Container or the active Docker-compatible context.")];
    }

    let view: Retained<NSView> =
        unsafe { msg_send_id![mtm.alloc::<NSView>(), initWithFrame: rect(0.0, 0.0, 380.0, 94.0)] };
    add_label(
        &view,
        "Runtime",
        rect(0.0, 65.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let runtime_field = add_text_field(
        &view,
        rect(116.0, 60.0, 264.0, 26.0),
        "container",
        "container",
        mtm,
    );
    add_label(
        &view,
        "Volume name",
        rect(0.0, 25.0, 120.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let volume_field = add_text_field(
        &view,
        rect(116.0, 20.0, 264.0, 26.0),
        "cocoa-way-data",
        "",
        mtm,
    );

    unsafe {
        let _: () = msg_send![&*alert, setAccessoryView: &*view];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Create")];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Cancel")];
        let _: () = msg_send![&*alert, layout];
    }

    let response: isize = unsafe { msg_send![&*alert, runModal] };
    if response != 1000 {
        return;
    }

    let runtime = field_string(&runtime_field);
    let volume = field_string(&volume_field);
    if volume.is_empty() {
        show_error_alert("Volume name is required.");
        return;
    }
    send(CompositorMessage::CreateContainerVolume {
        runtime: if runtime.is_empty() {
            "container".into()
        } else {
            runtime
        },
        volume,
    });
}

unsafe fn delete_container_session(index: usize) {
    let sessions = container_sessions::load_sessions();
    let Some(session) = sessions.get(index) else {
        show_error_alert("Session no longer exists.");
        return;
    };
    if !confirm_delete_session(&session.name) {
        return;
    }

    match container_sessions::remove_session(index) {
        Ok(()) => {
            *SELECTED_SESSION.lock().unwrap() = None;
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            push_activity(format!("Deleted session: {}", session.name));
            unsafe {
                rebuild_window();
            }
        }
        Err(error) => {
            let message = format!("Failed to delete session: {}", error);
            push_activity(message.clone());
            show_error_alert(&message);
        }
    }
}

fn confirm_delete_session(name: &str) -> bool {
    unsafe {
        let alert: Retained<NSAlert> = msg_send_id![NSAlert::class(), new];
        let _: () = msg_send![&*alert, setMessageText:
            &*NSString::from_str("Delete GUI Session?")];
        let message = format!(
            "This removes '{}' from container-sessions.toml. It will not stop a running container.",
            name
        );
        let _: () = msg_send![&*alert, setInformativeText:
            &*NSString::from_str(&message)];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Delete")];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Cancel")];
        let response: isize = msg_send![&*alert, runModal];
        response == 1000
    }
}

fn confirm_delete_resource(kind: &str, runtime: &str, name: &str) -> bool {
    unsafe {
        let alert: Retained<NSAlert> = msg_send_id![NSAlert::class(), new];
        let title = format!("Delete {}?", kind);
        let _: () = msg_send![&*alert, setMessageText:
            &*NSString::from_str(&title)];
        let message = format!(
            "This deletes '{}' from {}. Running containers that use it may fail.",
            name,
            runtime_label(runtime)
        );
        let _: () = msg_send![&*alert, setInformativeText:
            &*NSString::from_str(&message)];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Delete")];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Cancel")];
        let response: isize = msg_send![&*alert, runModal];
        response == 1000
    }
}

fn confirm_close_managed_display(slot: &str) -> bool {
    unsafe {
        let alert: Retained<NSAlert> = msg_send_id![NSAlert::class(), new];
        let _: () = msg_send![&*alert, setMessageText:
            &*NSString::from_str("Close Managed Display?")];
        let message = format!(
            "Closing '{}' disconnects GUI clients attached through copied environment variables, including clients Cocoa-Way cannot track.",
            slot
        );
        let _: () = msg_send![&*alert, setInformativeText:
            &*NSString::from_str(&message)];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Close Display")];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("Cancel")];
        let response: isize = msg_send![&*alert, runModal];
        response == 1000
    }
}

fn default_session_name(image: &str) -> String {
    if image.to_ascii_lowercase().contains("niri") {
        return "Niri Desktop".into();
    }

    image
        .rsplit('/')
        .next()
        .and_then(|tail| tail.split(':').next())
        .filter(|value| !value.is_empty())
        .unwrap_or("GUI Session")
        .replace('-', " ")
}

fn session_defaults_for_image(runtime: &str, image: &str) -> AddSessionDefaults {
    let is_niri_image = image.to_ascii_lowercase().contains("niri");
    AddSessionDefaults {
        name: default_session_name(image),
        runtime: runtime.into(),
        display: "auto".into(),
        profile: if is_niri_image {
            "niri".into()
        } else {
            "single-app".into()
        },
        image: image.into(),
        command: if is_niri_image {
            "niri".into()
        } else {
            String::new()
        },
        ..AddSessionDefaults::default()
    }
}

unsafe fn show_session_dialog_for_image(runtime: &str, image: &str) {
    unsafe {
        show_add_session_dialog_with_defaults(session_defaults_for_image(runtime, image));
    }
}

fn normalize_registry_reference(source: isize, reference: &str) -> String {
    let reference = reference.trim().trim_start_matches("docker://");
    let first = reference.split('/').next().unwrap_or_default();
    let has_registry = reference.contains('/')
        && (first.contains('.') || first.contains(':') || first == "localhost");
    if has_registry || source == 3 {
        return reference.to_string();
    }

    match source {
        1 => format!("ghcr.io/{}", reference.trim_start_matches('/')),
        2 => format!("quay.io/{}", reference.trim_start_matches('/')),
        _ if reference.contains('/') => {
            format!("docker.io/{}", reference.trim_start_matches('/'))
        }
        _ => format!("docker.io/library/{}", reference),
    }
}

fn field_string(field: &NSTextField) -> String {
    let value: Retained<NSString> = unsafe { msg_send_id![field, stringValue] };
    value.to_string().trim().to_string()
}

fn popup_index(popup: &NSPopUpButton) -> isize {
    unsafe { msg_send![popup, indexOfSelectedItem] }
}

fn semicolon_list(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn show_error_alert(message: &str) {
    unsafe {
        let alert: Retained<NSAlert> = msg_send_id![NSAlert::class(), new];
        let _: () = msg_send![&*alert, setMessageText:
            &*NSString::from_str("Container Mode")];
        let _: () = msg_send![&*alert, setInformativeText:
            &*NSString::from_str(message)];
        let _: Retained<NSObject> = msg_send_id![&*alert, addButtonWithTitle:
            &*NSString::from_str("OK")];
        let _: isize = msg_send![&*alert, runModal];
    }
}

fn remember_stop_request(index: usize) {
    let sessions = container_sessions::load_sessions();
    let message = match sessions.get(index) {
        Some(session) => {
            set_session_state(index, "Stopping", format!("Stopping {}.", session.name));
            format!("Stop requested: {}", session.name)
        }
        None => format!("Stop requested for missing session #{}", index + 1),
    };
    push_activity(message);
}

declare_class!(
    pub struct ContainerModeHandler;

    unsafe impl ClassType for ContainerModeHandler {
        type Super = NSObject;
        type Mutability = MainThreadOnly;
        const NAME: &'static str = "CocoaWayContainerModeHandler";
    }

    impl DeclaredClass for ContainerModeHandler {
        type Ivars = ();
    }

    unsafe impl ContainerModeHandler {
        #[method(windowDidResize:)]
        fn window_did_resize(&self, _notification: &AnyObject) {
            unsafe {
                refresh_window_for_resize(Duration::from_millis(16));
            }
        }

        #[method(windowDidEndLiveResize:)]
        fn window_did_end_live_resize(&self, _notification: &AnyObject) {
            *LAST_RESIZE_REBUILD.lock().unwrap() = None;
            unsafe {
                refresh_window_without_focus();
            }
        }

        #[method(launchContainerSession:)]
        fn launch_container_session(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let index = tag.max(0) as usize;
            if let Some(conflict) = active_display_conflict(index) {
                let message = format!(
                    "Default display is already used by '{}'. Stop that session before launching another one.",
                    conflict
                );
                set_session_state(index, "Blocked", message.clone());
                push_activity(format!("Launch blocked: {}", message));
                *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
                *SELECTED_SESSION.lock().unwrap() = Some(index);
                show_error_alert(&message);
                unsafe { rebuild_window(); }
                return;
            }
            remember_launch_request(index);
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = Some(index);
            send(CompositorMessage::StartContainerSession(index));
            unsafe { rebuild_window(); }
        }

        #[method(checkContainerSession:)]
        fn check_container_session(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let index = tag.max(0) as usize;
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = Some(index);
            set_session_state(index, "Checking", "Running launch preflight checks.".into());
            push_activity(format!("Check requested for session #{}", index + 1));
            send(CompositorMessage::CheckContainerSession(index));
            unsafe { rebuild_window(); }
        }

        #[method(stopContainerSession:)]
        fn stop_container_session(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let index = tag.max(0) as usize;
            if !session_can_stop(session_state(index).as_ref()) {
                let name = container_sessions::load_sessions()
                    .get(index)
                    .map(|session| session.name.clone())
                    .unwrap_or_else(|| format!("session #{}", index + 1));
                push_activity(format!("Stop ignored: {} is not running.", name));
                *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
                *SELECTED_SESSION.lock().unwrap() = Some(index);
                unsafe { rebuild_window(); }
                return;
            }
            remember_stop_request(index);
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = Some(index);
            send(CompositorMessage::StopContainerSession(index));
            unsafe { rebuild_window(); }
        }

        #[method(openContainerTerminal:)]
        fn open_container_terminal(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let index = tag.max(0) as usize;
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = Some(index);
            *SELECTED_TAB.lock().unwrap() = 2;
            push_activity(format!("Terminal requested for session #{}", index + 1));
            send(CompositorMessage::OpenContainerTerminal(index));
            unsafe { rebuild_window(); }
        }

        #[method(reloadContainerMode:)]
        fn reload_container_mode(&self, _sender: &AnyObject) {
            request_selected_runtime_container_details();
            unsafe { rebuild_window(); }
        }

        #[method(createManagedDisplay:)]
        fn create_managed_display(&self, _sender: &AnyObject) {
            send(CompositorMessage::CreateManagedDisplay);
        }

        #[method(copyManagedDisplayEnvironment:)]
        fn copy_managed_display_environment(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let display = MANAGED_DISPLAY_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some(display) = display else {
                show_error_alert("Managed display no longer exists.");
                return;
            };
            let command = format!(
                "export XDG_RUNTIME_DIR={} WAYLAND_DISPLAY={}",
                shell_single_quote(&display.runtime_dir),
                shell_single_quote(&display.display)
            );
            unsafe {
                let pasteboard = NSPasteboard::generalPasteboard();
                pasteboard.clearContents();
                pasteboard.setString_forType(
                    &NSString::from_str(&command),
                    NSPasteboardTypeString,
                );
            }
            push_activity(format!("Copied environment for managed display: {}", display.slot));
        }

        #[method(copyManagedDisplayCommand:)]
        fn copy_managed_display_command(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let display = MANAGED_DISPLAY_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some(display) = display else {
                show_error_alert("Managed display no longer exists.");
                return;
            };
            let command = format!(
                "./run_waypipe.sh --display {}",
                shell_single_quote(&display.slot)
            );
            unsafe {
                let pasteboard = NSPasteboard::generalPasteboard();
                pasteboard.clearContents();
                pasteboard.setString_forType(
                    &NSString::from_str(&command),
                    NSPasteboardTypeString,
                );
            }
            push_activity(format!(
                "Copied run_waypipe.sh command for managed display: {}",
                display.slot
            ));
        }

        #[method(closeManagedDisplay:)]
        fn close_managed_display(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let display = MANAGED_DISPLAY_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some(display) = display else {
                show_error_alert("Managed display no longer exists.");
                return;
            };
            if confirm_close_managed_display(&display.slot) {
                send(CompositorMessage::CloseManagedDisplay(display.slot));
            }
        }

        #[method(clearContainerActivity:)]
        fn clear_container_activity(&self, _sender: &AnyObject) {
            ACTIVITY.lock().unwrap().clear();
            unsafe { rebuild_window(); }
        }

        #[method(openContainerConfig:)]
        fn open_container_config(&self, _sender: &AnyObject) {
            let path = container_sessions::config_path();
            let _ = container_sessions::load_sessions();
            let _ = std::process::Command::new("open")
                .arg("-R")
                .arg(path)
                .spawn();
        }

        #[method(addContainerSession:)]
        fn add_container_session(&self, _sender: &AnyObject) {
            unsafe { show_add_session_dialog(); }
        }

        #[method(editContainerSession:)]
        fn edit_container_session(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            unsafe { show_edit_session_dialog(tag.max(0) as usize); }
        }

        #[method(duplicateContainerSession:)]
        fn duplicate_container_session(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            unsafe { duplicate_session_profile(tag.max(0) as usize); }
        }

        #[method(newImageContainerSession:)]
        fn new_image_container_session(&self, _sender: &AnyObject) {
            unsafe { show_new_image_session_dialog(); }
        }

        #[method(restoreSmokeContainerSession:)]
        fn restore_smoke_container_session(&self, _sender: &AnyObject) {
            add_or_select_smoke_session();
            unsafe { rebuild_window(); }
        }

        #[method(pullContainerImage:)]
        fn pull_container_image(&self, _sender: &AnyObject) {
            unsafe { show_pull_image_dialog(); }
        }

        #[method(loginContainerRegistry:)]
        fn login_container_registry(&self, _sender: &AnyObject) {
            unsafe { show_registry_login_dialog(); }
        }

        #[method(loadContainerImage:)]
        fn load_container_image(&self, _sender: &AnyObject) {
            unsafe { show_load_image_dialog(); }
        }

        #[method(buildSmokeContainerImage:)]
        fn build_smoke_container_image(&self, _sender: &AnyObject) {
            request_smoke_image_build();
            unsafe { rebuild_window(); }
        }

        #[method(buildSmokeContainerSessionImage:)]
        fn build_smoke_container_session_image(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = Some(tag.max(0) as usize);
            request_smoke_image_build();
            unsafe { rebuild_window(); }
        }

        #[method(startAppleContainerSystem:)]
        fn start_apple_container_system(&self, _sender: &AnyObject) {
            send(CompositorMessage::StartAppleContainerSystem);
            unsafe { rebuild_window(); }
        }

        #[method(stopAppleContainerSystem:)]
        fn stop_apple_container_system(&self, _sender: &AnyObject) {
            send(CompositorMessage::RuntimeSystemAction {
                runtime: "apple".into(),
                action: "stop".into(),
            });
        }

        #[method(pullContainerSessionImage:)]
        fn pull_container_session_image(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let index = tag.max(0) as usize;
            let sessions = container_sessions::load_sessions();
            let Some(session) = sessions.get(index) else {
                show_error_alert("Session no longer exists.");
                return;
            };
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = Some(index);
            push_activity(format!(
                "Pull requested for missing image: {}",
                session.image
            ));
            if !allow_storage_growth("pull the missing session image") {
                return;
            }
            send(CompositorMessage::PullContainerImage {
                runtime: session.runtime.clone(),
                image: session.image.clone(),
                platform: None,
                scheme: None,
                configure_session: false,
            });
            unsafe { rebuild_window(); }
        }

        #[method(loadContainerSessionImage:)]
        fn load_container_session_image(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = Some(tag.max(0) as usize);
            unsafe { show_load_image_dialog(); }
        }

        #[method(copySmokeImageBuildCommand:)]
        fn copy_smoke_image_build_command(&self, _sender: &AnyObject) {
            unsafe {
                let pasteboard = NSPasteboard::generalPasteboard();
                pasteboard.clearContents();
                pasteboard.setString_forType(
            &NSString::from_str(&smoke_image_build_command()),
            NSPasteboardTypeString,
        );
    }
    push_activity("Copied example image build command.".into());
            unsafe { rebuild_window(); }
        }

        #[method(deleteContainerImage:)]
        fn delete_container_image(&self, _sender: &AnyObject) {
            unsafe { show_delete_image_dialog(); }
        }

        #[method(createContainerVolume:)]
        fn create_container_volume(&self, _sender: &AnyObject) {
            unsafe { show_create_volume_dialog(); }
        }

        #[method(createContainerSessionFromImage:)]
        fn create_container_session_from_image(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let action = IMAGE_CREATE_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some((runtime, image)) = action else {
                show_error_alert("Image action no longer exists. Press Reload and try again.");
                return;
            };
            unsafe {
                show_session_dialog_for_image(&runtime, &image);
            }
        }

        #[method(deleteLocalContainerImage:)]
        fn delete_local_container_image(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let action = IMAGE_DELETE_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some((runtime, image)) = action else {
                show_error_alert("Image action no longer exists. Press Reload and try again.");
                return;
            };
            if !confirm_delete_resource("Image", &runtime, &image) {
                return;
            }
            send(CompositorMessage::DeleteContainerImage { runtime, image });
        }

        #[method(deleteLocalContainerVolume:)]
        fn delete_local_container_volume(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let action = VOLUME_DELETE_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some((runtime, volume)) = action else {
                show_error_alert("Volume action no longer exists. Press Reload and try again.");
                return;
            };
            if !confirm_delete_resource("Volume", &runtime, &volume) {
                return;
            }
            send(CompositorMessage::DeleteContainerVolume { runtime, volume });
        }

        #[method(stopRuntimeContainer:)]
        fn stop_runtime_container(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let action = RUNTIME_CONTAINER_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some((runtime, name)) = action else {
                show_error_alert("Container action no longer exists. Press Reload and try again.");
                return;
            };
            send(CompositorMessage::StopRuntimeContainer { runtime, name });
        }

        #[method(startRuntimeContainer:)]
        fn start_runtime_container(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let action = RUNTIME_CONTAINER_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some((runtime, name)) = action else {
                show_error_alert("Container action no longer exists. Press Reload and try again.");
                return;
            };
            send(CompositorMessage::StartRuntimeContainer { runtime, name });
        }

        #[method(deleteRuntimeContainer:)]
        fn delete_runtime_container(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let action = RUNTIME_CONTAINER_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some((runtime, name)) = action else {
                show_error_alert("Container action no longer exists. Press Reload and try again.");
                return;
            };
            if !confirm_delete_resource("Container", &runtime, &name) {
                return;
            }
            send(CompositorMessage::DeleteRuntimeContainer { runtime, name });
        }

        #[method(restartRuntimeContainer:)]
        fn restart_runtime_container(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let action = RUNTIME_CONTAINER_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some((runtime, name)) = action else {
                show_error_alert("Container action no longer exists. Press Reload and try again.");
                return;
            };
            send(CompositorMessage::RestartRuntimeContainer { runtime, name });
        }

        #[method(openRuntimeContainerTerminal:)]
        fn open_runtime_container_terminal(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let action = RUNTIME_CONTAINER_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some((runtime, name)) = action else {
                show_error_alert("Container action no longer exists. Press Reload and try again.");
                return;
            };
            send(CompositorMessage::OpenRuntimeContainerTerminal { runtime, name });
        }

        #[method(selectRuntimeContainer:)]
        fn select_runtime_container(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let selected = RUNTIME_CONTAINER_SELECT_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some(selected) = selected else {
                show_error_alert("Container selection no longer exists. Press Reload and try again.");
                return;
            };
            *SELECTED_NAV.lock().unwrap() = runtime_nav(&selected.runtime);
            *SELECTED_RUNTIME_CONTAINER.lock().unwrap() = Some(selected.clone());
            *RUNTIME_CONTAINER_DETAILS.lock().unwrap() = None;
            send(CompositorMessage::RefreshRuntimeContainerDetails {
                runtime: selected.runtime,
                name: selected.name,
            });
            unsafe { rebuild_window(); }
        }

        #[method(refreshRuntimeContainerDetails:)]
        fn refresh_runtime_container_details(&self, _sender: &AnyObject) {
            request_selected_runtime_container_details();
            unsafe { rebuild_window(); }
        }

        #[method(startOrbStack:)]
        fn start_orbstack(&self, _sender: &AnyObject) {
            send(CompositorMessage::RuntimeSystemAction {
                runtime: "orbstack".into(),
                action: "start".into(),
            });
        }

        #[method(stopOrbStack:)]
        fn stop_orbstack(&self, _sender: &AnyObject) {
            send(CompositorMessage::RuntimeSystemAction {
                runtime: "orbstack".into(),
                action: "stop".into(),
            });
        }

        #[method(useDockerContext:)]
        fn use_docker_context(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let context = DOCKER_CONTEXT_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            let Some(name) = context else {
                show_error_alert(
                    "Docker context action no longer exists. Press Reload and try again.",
                );
                return;
            };
            send(CompositorMessage::UseDockerContext { name });
        }

        #[method(selectContainerImage:)]
        fn select_container_image(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let image = IMAGE_SELECT_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            if let Some(image) = image {
                *SELECTED_NAV.lock().unwrap() = NAV_IMAGES;
                *SELECTED_SESSION.lock().unwrap() = None;
                *SELECTED_IMAGE.lock().unwrap() = Some(image);
                unsafe { rebuild_window(); }
            }
        }

        #[method(selectContainerVolume:)]
        fn select_container_volume(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let volume = VOLUME_SELECT_ACTIONS
                .lock()
                .unwrap()
                .get(tag.max(0) as usize)
                .cloned();
            if let Some(volume) = volume {
                *SELECTED_NAV.lock().unwrap() = NAV_VOLUMES;
                *SELECTED_SESSION.lock().unwrap() = None;
                *SELECTED_VOLUME.lock().unwrap() = Some(volume);
                unsafe { rebuild_window(); }
            }
        }

        #[method(copyContainerCommand:)]
        fn copy_container_command(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let commands = command_items();
            let Some(command) = commands.get(tag.max(0) as usize) else {
                show_error_alert("Command no longer exists.");
                return;
            };
            unsafe {
                let pasteboard = NSPasteboard::generalPasteboard();
                pasteboard.clearContents();
                pasteboard.setString_forType(&NSString::from_str(command), NSPasteboardTypeString);
            }
            push_activity(format!("Copied command: {}", command));
            unsafe { rebuild_window(); }
        }

        #[method(openAppleContainerDataRoot:)]
        fn open_apple_container_data_root(&self, _sender: &AnyObject) {
            let path = apple_container_data_root();
            let _ = std::process::Command::new("open").arg(path).spawn();
            push_activity("Opened Apple Container data root.".into());
            unsafe { rebuild_window(); }
        }

        #[method(openOrbStackApp:)]
        fn open_orbstack_app(&self, _sender: &AnyObject) {
            match std::process::Command::new("open")
                .args(["-a", "OrbStack"])
                .spawn()
            {
                Ok(_) => push_activity("Opened OrbStack.".into()),
                Err(error) => show_error_alert(&format!("Could not open OrbStack: {}", error)),
            }
            unsafe { rebuild_window(); }
        }

        #[method(deleteContainerSession:)]
        fn delete_container_session(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let index = tag.max(0) as usize;
            unsafe { delete_container_session(index); }
        }

        #[method(selectContainerNav:)]
        fn select_container_nav(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let selected = tag.max(0) as usize;
            *SELECTED_NAV.lock().unwrap() = selected;
            if selected != NAV_SESSIONS {
                *SELECTED_SESSION.lock().unwrap() = None;
            }
            if selected != NAV_IMAGES {
                *SELECTED_IMAGE.lock().unwrap() = None;
            }
            if selected != NAV_VOLUMES {
                *SELECTED_VOLUME.lock().unwrap() = None;
            }
            let keep_runtime = SELECTED_RUNTIME_CONTAINER
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|container| runtime_nav(&container.runtime) == selected);
            if !keep_runtime {
                *SELECTED_RUNTIME_CONTAINER.lock().unwrap() = None;
                *RUNTIME_CONTAINER_DETAILS.lock().unwrap() = None;
            }
            unsafe { rebuild_window(); }
        }

        #[method(selectContainerTab:)]
        fn select_container_tab(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            *SELECTED_TAB.lock().unwrap() = tag.max(0) as usize;
            unsafe { rebuild_window(); }
        }

        #[method(selectContainerSession:)]
        fn select_container_session(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            *SELECTED_NAV.lock().unwrap() = NAV_SESSIONS;
            *SELECTED_SESSION.lock().unwrap() = Some(tag.max(0) as usize);
            *SELECTED_IMAGE.lock().unwrap() = None;
            *SELECTED_VOLUME.lock().unwrap() = None;
            *SELECTED_RUNTIME_CONTAINER.lock().unwrap() = None;
            *RUNTIME_CONTAINER_DETAILS.lock().unwrap() = None;
            unsafe { rebuild_window(); }
        }
    }
);

pub fn show(sender: Sender<CompositorMessage>, mtm: MainThreadMarker) {
    *SENDER.lock().unwrap() = Some(sender);

    unsafe {
        ensure_handler();
        let window = ensure_window(mtm);
        install_content(window, mtm);
        window.center();
        window.makeKeyAndOrderFront(None);
    }
}

unsafe fn rebuild_window() {
    let Some(window_ptr) = *WINDOW.lock().unwrap() else {
        return;
    };
    // Container Mode actions are only wired from AppKit controls on the main thread.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };

    let window = unsafe { &*(window_ptr as *mut NSWindow) };
    unsafe {
        install_content(window, mtm);
    }
    window.makeKeyAndOrderFront(None);
}

unsafe fn refresh_window_without_focus() {
    let Some(window_ptr) = *WINDOW.lock().unwrap() else {
        return;
    };
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let window = unsafe { &*(window_ptr as *mut NSWindow) };
    unsafe {
        install_content(window, mtm);
    }
}

unsafe fn ensure_handler() -> *mut AnyObject {
    if let Some(ptr) = *HANDLER.lock().unwrap() {
        return ptr as *mut AnyObject;
    }

    let handler: Retained<ContainerModeHandler> =
        unsafe { msg_send_id![ContainerModeHandler::class(), new] };
    let ptr = Retained::into_raw(handler) as *mut AnyObject;
    *HANDLER.lock().unwrap() = Some(ptr as usize);
    ptr
}

unsafe fn ensure_window(mtm: MainThreadMarker) -> &'static NSWindow {
    if let Some(ptr) = *WINDOW.lock().unwrap() {
        return unsafe { &*(ptr as *mut NSWindow) };
    }

    let frame = rect(160.0, 160.0, 1180.0, 760.0);
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable;
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc::<NSWindow>(),
            frame,
            style,
            NSBackingStoreType::NSBackingStoreBuffered,
            false,
        )
    };
    window.setTitle(&NSString::from_str("Cocoa-Way Container Mode"));
    unsafe {
        window.setContentMinSize(NSSize {
            width: 1040.0,
            height: 620.0,
        });
    }
    let handler = unsafe { ensure_handler() };
    let _: () = unsafe { msg_send![&*window, setDelegate: handler] };
    let ptr = Retained::into_raw(window);
    *WINDOW.lock().unwrap() = Some(ptr as usize);
    unsafe { &*ptr }
}

unsafe fn install_content(window: &NSWindow, mtm: MainThreadMarker) {
    let (width, height) = content_size(window);
    let root: Retained<NSView> = unsafe {
        msg_send_id![mtm.alloc::<NSView>(), initWithFrame: rect(0.0, 0.0, width, height)]
    };
    let load_report = container_sessions::load_sessions_report();
    let sessions = load_report.sessions;
    let config_error = load_report.error;
    let handler = unsafe { ensure_handler() };
    let selected_nav = *SELECTED_NAV.lock().unwrap();
    let selected_tab = *SELECTED_TAB.lock().unwrap();
    let selected_session = selected_session_index(sessions.len(), selected_nav);

    let sidebar_w = 230.0;
    let available_w = (width - sidebar_w).max(760.0);
    let mut list_w = if matches!(selected_nav, NAV_IMAGES | NAV_VOLUMES | NAV_DISPLAYS) {
        (available_w * 0.46).clamp(440.0, 560.0)
    } else {
        (available_w * 0.39).clamp(380.0, 470.0)
    };
    let min_detail_w = 420.0;
    if width - sidebar_w - list_w < min_detail_w {
        list_w = (width - sidebar_w - min_detail_w).max(360.0);
    }
    let toolbar_h = 64.0;
    let detail_x = sidebar_w + list_w;
    let detail_w = width - detail_x;

    add_resource_sidebar(
        &root,
        sessions.len(),
        height,
        sidebar_w,
        selected_nav,
        handler,
        mtm,
    );
    add_separator(&root, rect(sidebar_w, 0.0, 1.0, height), mtm);

    add_list_toolbar(
        &root,
        sidebar_w,
        height - toolbar_h,
        list_w,
        toolbar_h,
        selected_nav,
        handler,
        mtm,
    );
    add_separator(&root, rect(sidebar_w, height - toolbar_h, list_w, 1.0), mtm);
    add_separator(&root, rect(detail_x, 0.0, 1.0, height), mtm);

    let scroll_frame = rect(sidebar_w + 1.0, 0.0, list_w - 1.0, height - toolbar_h);
    let scroll = unsafe { NSScrollView::initWithFrame(mtm.alloc::<NSScrollView>(), scroll_frame) };
    unsafe {
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
    }

    let row_height = 142.0;
    let content_w = list_w - 16.0;
    let min_page_height = match selected_nav {
        NAV_APPLE_CONTAINER => 1660.0,
        NAV_ORBSTACK => 1320.0,
        NAV_DOCKER => 1240.0,
        NAV_IMAGES => 1800.0,
        NAV_VOLUMES => 1400.0,
        NAV_DISPLAYS => {
            let managed_rows =
                managed_displays_snapshot().len() + pending_managed_displays_snapshot().len();
            1180.0 + managed_rows as f64 * 126.0
        }
        NAV_ACTIVITY | NAV_COMMANDS => 1080.0,
        _ => height - toolbar_h,
    };
    let content_height = (sessions.len().max(1) as f64 * row_height + 18.0).max(min_page_height);
    let content: Retained<NSView> = unsafe {
        msg_send_id![mtm.alloc::<NSView>(), initWithFrame: rect(0.0, 0.0, content_w, content_height)]
    };

    if selected_nav == NAV_IMAGES {
        add_images_list(&content, content_w, content_height, handler, mtm);
    } else if selected_nav == NAV_VOLUMES {
        add_volumes_list(&content, content_w, content_height, handler, mtm);
    } else if selected_nav == NAV_DISPLAYS {
        add_displays_list(&content, content_w, content_height, &sessions, handler, mtm);
    } else if matches!(
        selected_nav,
        NAV_APPLE_CONTAINER | NAV_DOCKER | NAV_ORBSTACK
    ) {
        add_runtime_list(
            &content,
            content_w,
            content_height,
            selected_nav,
            handler,
            mtm,
        );
    } else if selected_nav == NAV_ACTIVITY {
        add_activity_list(&content, content_w, content_height, mtm);
    } else if selected_nav == NAV_COMMANDS {
        add_commands_list(&content, content_w, content_height, handler, mtm);
    } else if selected_nav != NAV_SESSIONS {
        add_placeholder_list(
            &content,
            content_w,
            content_height,
            nav_title(selected_nav),
            mtm,
        );
    } else if let Some(error) = config_error.as_deref() {
        add_config_error_list(&content, content_w, content_height, error, handler, mtm);
    } else if sessions.is_empty() {
        add_session_empty_list(&content, content_w, content_height, handler, mtm);
    } else {
        for (index, session) in sessions.iter().enumerate() {
            let y = content_height - ((index + 1) as f64 * row_height);
            unsafe {
                add_session_row(
                    &content,
                    session,
                    index,
                    selected_session == Some(index),
                    session_state(index),
                    y,
                    content_w,
                    handler,
                    mtm,
                );
            }
        }
    }

    unsafe {
        scroll.setDocumentView(Some(&content));
        let clip_view: Retained<AnyObject> = msg_send_id![&*scroll, contentView];
        let top_y = (content_height - scroll_frame.size.height).max(0.0);
        let _: () = msg_send![&*clip_view, scrollToPoint: NSPoint { x: 0.0, y: top_y }];
        let _: () = msg_send![&*scroll, reflectScrolledClipView: &*clip_view];
    }
    unsafe {
        root.addSubview(&scroll);
    }
    add_detail_panel(
        &root,
        detail_x,
        0.0,
        detail_w,
        height,
        selected_tab,
        selected_nav,
        selected_session.and_then(|index| sessions.get(index).map(|session| (index, session))),
        handler,
        mtm,
    );
    window.setContentView(Some(&root));
}

fn selected_session_index(session_count: usize, selected_nav: usize) -> Option<usize> {
    if selected_nav != NAV_SESSIONS {
        return None;
    }

    let mut selected = SELECTED_SESSION.lock().unwrap();
    match *selected {
        Some(index) if index < session_count => Some(index),
        _ => {
            *selected = None;
            None
        }
    }
}

fn content_size(window: &NSWindow) -> (f64, f64) {
    if let Some(content) = window.contentView() {
        let frame = content.frame();
        return (frame.size.width, frame.size.height);
    }
    (1180.0, 760.0)
}

fn add_resource_sidebar(
    parent: &NSView,
    session_count: usize,
    height: f64,
    width: f64,
    selected_nav: usize,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let compact = height < 700.0;
    let nav_height = if compact { 34.0 } else { 44.0 };
    let session_y = if compact {
        height - 132.0
    } else {
        height - 144.0
    };
    let nav_step = if compact { 38.0 } else { 48.0 };
    let runtime_heading_y = if compact {
        height - 286.0
    } else {
        height - 340.0
    };
    let runtime_first_y = if compact {
        height - 324.0
    } else {
        height - 388.0
    };
    let general_heading_y = if compact {
        height - 440.0
    } else {
        height - 536.0
    };
    let general_first_y = if compact {
        height - 478.0
    } else {
        height - 584.0
    };
    add_label(
        parent,
        "Cocoa-Way",
        rect(18.0, height - 52.0, width - 36.0, 24.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Container GUI",
        rect(18.0, height - 96.0, width - 36.0, 18.0),
        mtm,
        TextStyle::Section,
    );
    add_nav_item(
        parent,
        NAV_SESSIONS,
        "GUI Sessions",
        &format!("{} profiles", session_count),
        selected_nav == NAV_SESSIONS,
        rect(10.0, session_y, width - 20.0, nav_height),
        handler,
        mtm,
    );
    add_nav_item(
        parent,
        NAV_IMAGES,
        "Images",
        "GUI-ready images",
        selected_nav == NAV_IMAGES,
        rect(10.0, session_y - nav_step, width - 20.0, nav_height),
        handler,
        mtm,
    );
    add_nav_item(
        parent,
        NAV_VOLUMES,
        "Volumes",
        "shared data",
        selected_nav == NAV_VOLUMES,
        rect(10.0, session_y - nav_step * 2.0, width - 20.0, nav_height),
        handler,
        mtm,
    );
    add_nav_item(
        parent,
        NAV_DISPLAYS,
        "Displays",
        "window slots",
        selected_nav == NAV_DISPLAYS,
        rect(10.0, session_y - nav_step * 3.0, width - 20.0, nav_height),
        handler,
        mtm,
    );

    add_label(
        parent,
        "Runtime",
        rect(18.0, runtime_heading_y, width - 36.0, 18.0),
        mtm,
        TextStyle::Section,
    );
    add_nav_item(
        parent,
        NAV_APPLE_CONTAINER,
        "Apple Container",
        "first-class target",
        selected_nav == NAV_APPLE_CONTAINER,
        rect(10.0, runtime_first_y, width - 20.0, nav_height),
        handler,
        mtm,
    );
    add_nav_item(
        parent,
        NAV_DOCKER,
        "Docker",
        "compatibility",
        selected_nav == NAV_DOCKER,
        rect(10.0, runtime_first_y - nav_step, width - 20.0, nav_height),
        handler,
        mtm,
    );
    add_nav_item(
        parent,
        NAV_ORBSTACK,
        "OrbStack",
        "classic bridge",
        selected_nav == NAV_ORBSTACK,
        rect(
            10.0,
            runtime_first_y - nav_step * 2.0,
            width - 20.0,
            nav_height,
        ),
        handler,
        mtm,
    );

    add_label(
        parent,
        "General",
        rect(18.0, general_heading_y, width - 36.0, 18.0),
        mtm,
        TextStyle::Section,
    );
    let activity_count = activity_snapshot().len();
    add_nav_item(
        parent,
        NAV_ACTIVITY,
        "Activity",
        &format!("{} events", activity_count),
        selected_nav == NAV_ACTIVITY,
        rect(10.0, general_first_y, width - 20.0, nav_height),
        handler,
        mtm,
    );
    add_nav_item(
        parent,
        NAV_COMMANDS,
        "Commands",
        "launch helpers",
        selected_nav == NAV_COMMANDS,
        rect(10.0, general_first_y - nav_step, width - 20.0, nav_height),
        handler,
        mtm,
    );
    add_separator(parent, rect(0.0, 70.0, width, 1.0), mtm);
    add_label(
        parent,
        "Open source\nLocal control plane",
        rect(18.0, 22.0, width - 36.0, 38.0),
        mtm,
        TextStyle::Caption,
    );
}

fn add_nav_item(
    parent: &NSView,
    index: usize,
    title: &str,
    subtitle: &str,
    active: bool,
    frame: NSRect,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    if active {
        add_card(parent, frame, mtm);
        add_runtime_accent(
            parent,
            index,
            rect(
                frame.origin.x,
                frame.origin.y + 8.0,
                3.0,
                frame.size.height - 16.0,
            ),
            mtm,
        );
    }
    if frame.size.height < 40.0 {
        add_label(
            parent,
            title,
            rect(
                frame.origin.x + 16.0,
                frame.origin.y + 7.0,
                frame.size.width - 32.0,
                20.0,
            ),
            mtm,
            TextStyle::Heading,
        );
    } else {
        add_label(
            parent,
            title,
            rect(
                frame.origin.x + 16.0,
                frame.origin.y + 22.0,
                frame.size.width - 32.0,
                20.0,
            ),
            mtm,
            TextStyle::Heading,
        );
        add_label(
            parent,
            subtitle,
            rect(
                frame.origin.x + 16.0,
                frame.origin.y + 7.0,
                frame.size.width - 32.0,
                16.0,
            ),
            mtm,
            TextStyle::Caption,
        );
    }
    add_hit_button(
        parent,
        frame,
        index,
        handler,
        sel!(selectContainerNav:),
        mtm,
    );
}

fn add_list_toolbar(
    parent: &NSView,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    selected_nav: usize,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_label(
        parent,
        nav_title(selected_nav),
        rect(x + 18.0, y + 18.0, width - 160.0, 30.0),
        mtm,
        TextStyle::Title,
    );
    let reload = add_button(
        parent,
        "Reload",
        rect(x + width - 132.0, y + 17.0, 72.0, 30.0),
        handler,
        sel!(reloadContainerMode:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*reload, setToolTip:
            &*NSString::from_str("Reload container-sessions.toml")];
    }
    if selected_nav == NAV_ACTIVITY {
        let clear = add_button(
            parent,
            "Clear",
            rect(x + width - 214.0, y + 17.0, 74.0, 30.0),
            handler,
            sel!(clearContainerActivity:),
            mtm,
        );
        unsafe {
            let _: () = msg_send![&*clear, setToolTip:
                &*NSString::from_str("Clear Container Mode activity messages")];
        }
    }
    if selected_nav == NAV_SESSIONS {
        let open = add_button(
            parent,
            "+",
            rect(x + width - 44.0, y + 17.0, 34.0, 30.0),
            handler,
            sel!(addContainerSession:),
            mtm,
        );
        unsafe {
            let _: () = msg_send![&*open, setToolTip:
                &*NSString::from_str("Add a Container Mode GUI session")];
        }
    }
    let _ = height;
}

fn add_session_empty_list(
    parent: &NSView,
    width: f64,
    content_height: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let center_y = (content_height * 0.52).max(250.0);
    add_label(
        parent,
        "No GUI Sessions",
        rect(34.0, center_y, width - 68.0, 34.0),
        mtm,
        TextStyle::Title,
    );
    add_label(
        parent,
        "Restore the bundled Niri desktop profile or create a custom container GUI session.",
        rect(34.0, center_y - 42.0, width - 68.0, 42.0),
        mtm,
        TextStyle::Body,
    );
    add_label(
        parent,
        "Example\nruntime = \"container\"\nimage = \"localhost/cocoa-way-niri:latest\"\nprofile = \"niri\"\ncommand = \"niri\"",
        rect(34.0, center_y - 142.0, width - 68.0, 90.0),
        mtm,
        TextStyle::Mono,
    );
    let restore = add_button(
        parent,
        "Restore Example",
        rect(34.0, center_y - 190.0, 128.0, 30.0),
        handler,
        sel!(restoreSmokeContainerSession:),
        mtm,
    );
    let add = add_button(
        parent,
        "Custom...",
        rect(174.0, center_y - 190.0, 96.0, 30.0),
        handler,
        sel!(addContainerSession:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*restore, setToolTip:
            &*NSString::from_str("Restore the bundled example GUI session")];
        let _: () = msg_send![&*add, setToolTip:
            &*NSString::from_str("Create a custom Container Mode GUI session")];
    }
}

fn add_config_error_list(
    parent: &NSView,
    width: f64,
    content_height: f64,
    error: &str,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let center_y = (content_height * 0.52).max(250.0);
    add_label(
        parent,
        "Config Error",
        rect(34.0, center_y, width - 68.0, 34.0),
        mtm,
        TextStyle::Title,
    );
    add_label(
        parent,
        "Container Mode could not parse container-sessions.toml. Fix the file and press Reload.",
        rect(34.0, center_y - 42.0, width - 68.0, 42.0),
        mtm,
        TextStyle::Body,
    );
    add_label(
        parent,
        error,
        rect(34.0, center_y - 128.0, width - 68.0, 72.0),
        mtm,
        TextStyle::Mono,
    );
    let open = add_button(
        parent,
        "Open Config",
        rect(34.0, center_y - 176.0, 116.0, 30.0),
        handler,
        sel!(openContainerConfig:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*open, setToolTip:
            &*NSString::from_str("Reveal container-sessions.toml in Finder")];
    }
}

fn add_activity_list(parent: &NSView, width: f64, content_height: f64, mtm: MainThreadMarker) {
    let activity = activity_snapshot();
    let mut y = content_height - 58.0;
    add_label(
        parent,
        "Performance",
        rect(34.0, y, width - 68.0, 24.0),
        mtm,
        TextStyle::Title,
    );
    y -= 44.0;
    add_card(parent, rect(24.0, y - 174.0, width - 48.0, 204.0), mtm);
    if let Some(snapshot) = performance_snapshot() {
        add_label(
            parent,
            &format!(
                "Render {:.1} fps  |  commits {:.1}/s  |  late {:.1}/s",
                snapshot.redraw_fps, snapshot.commits_per_second, snapshot.late_redraws_per_second
            ),
            rect(38.0, y + 2.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Body,
        );
        add_label(
            parent,
            &format!(
                "Max redraw wait {:.1} ms  |  host input -> present {}",
                snapshot.max_redraw_wait_ms,
                snapshot
                    .input_to_present_ms
                    .map(|value| format!("{value:.1} ms"))
                    .unwrap_or_else(|| "waiting".into()),
            ),
            rect(38.0, y - 26.0, width - 76.0, 18.0),
            mtm,
            TextStyle::Caption,
        );
        add_label(
            parent,
            &format!(
                "Scene {} tile(s)  |  dirty {}  |  callbacks {}",
                snapshot.tiles,
                if snapshot.dirty { "yes" } else { "no" },
                snapshot.pending_frame_callbacks
            ),
            rect(38.0, y - 50.0, width - 76.0, 18.0),
            mtm,
            TextStyle::Caption,
        );
    } else {
        add_label(
            parent,
            "No performance sample yet. Launch a GUI session or wait for the next render tick.",
            rect(38.0, y - 8.0, width - 76.0, 36.0),
            mtm,
            TextStyle::Caption,
        );
    }
    let resources = crate::diagnostics::resource_snapshot();
    let resource_line = if resources.available {
        format!(
            "Apple containers {}  |  CPU {}  |  memory {:.2} / {:.2} GiB",
            resources.containers.len(),
            resources
                .total_cpu_percent
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| "sampling".into()),
            crate::diagnostics::bytes_to_gib(resources.total_memory_usage_bytes),
            crate::diagnostics::bytes_to_gib(resources.total_memory_limit_bytes),
        )
    } else {
        format!(
            "Apple container resources: {}",
            resources.error.as_deref().unwrap_or("unavailable")
        )
    };
    add_label(
        parent,
        &resource_line,
        rect(38.0, y - 78.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let clipboard = crate::diagnostics::clipboard_snapshot();
    add_label(
        parent,
        &format!(
            "Clipboard {}  |  H->G {}  |  G->H {}  |  errors {}",
            clipboard.last_direction.as_deref().unwrap_or("waiting"),
            clipboard.host_to_guest_events,
            clipboard.guest_to_host_events,
            clipboard.failures,
        ),
        rect(38.0, y - 102.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    add_label(
        parent,
        &format!(
            "Disk {}",
            resources
                .disk_available_bytes
                .map(|bytes| format!("{:.1} GiB free", crate::diagnostics::bytes_to_gib(bytes)))
                .unwrap_or_else(|| "unknown".into())
        ),
        rect(38.0, y - 126.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    y -= 236.0;

    if activity.is_empty() {
        add_label(
            parent,
            "No recent activity",
            rect(34.0, y, width - 68.0, 24.0),
            mtm,
            TextStyle::Title,
        );
        return;
    }

    add_label(
        parent,
        "Recent Activity",
        rect(34.0, y, width - 68.0, 24.0),
        mtm,
        TextStyle::Title,
    );
    y -= 42.0;
    for line in activity.iter().rev().take(12) {
        add_card(parent, rect(24.0, y - 10.0, width - 48.0, 42.0), mtm);
        add_label(
            parent,
            line,
            rect(38.0, y + 2.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Caption,
        );
        y -= 52.0;
    }
}

fn add_images_list(
    parent: &NSView,
    width: f64,
    content_height: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let inventories = image_inventories();
    let registry_summary = apple_registry_login_summary(&build_child_path());
    IMAGE_CREATE_ACTIONS.lock().unwrap().clear();
    IMAGE_DELETE_ACTIONS.lock().unwrap().clear();
    IMAGE_SELECT_ACTIONS.lock().unwrap().clear();
    let selected_image = SELECTED_IMAGE.lock().unwrap().clone();
    let mut y = content_height - 58.0;
    add_label(
        parent,
        "Local Images",
        rect(34.0, y, width - 68.0, 24.0),
        mtm,
        TextStyle::Title,
    );
    y -= 42.0;

    add_card(parent, rect(24.0, y - 140.0, width - 48.0, 170.0), mtm);
    add_label(
        parent,
        "Sources & images",
        rect(38.0, y + 8.0, width - 76.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Pull from Docker Hub, GHCR, Quay, or any OCI registry.",
        rect(38.0, y - 20.0, width - 76.0, 32.0),
        mtm,
        TextStyle::Caption,
    );
    add_label(
        parent,
        &registry_summary,
        rect(38.0, y - 46.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let button_area = width - 76.0;
    let half = ((button_area - 10.0) / 2.0).max(80.0);
    let third = ((button_area - 20.0) / 3.0).max(64.0);
    let pull = add_button(
        parent,
        "Pull from Source...",
        rect(38.0, y - 78.0, half, 28.0),
        handler,
        sel!(pullContainerImage:),
        mtm,
    );
    let login = add_button(
        parent,
        "Registry Login...",
        rect(48.0 + half, y - 78.0, half, 28.0),
        handler,
        sel!(loginContainerRegistry:),
        mtm,
    );
    let add_session = add_button(
        parent,
        "New Session...",
        rect(38.0, y - 114.0, third, 28.0),
        handler,
        sel!(newImageContainerSession:),
        mtm,
    );
    let build = add_button(
        parent,
        "Build Example",
        rect(48.0 + third, y - 114.0, third, 28.0),
        handler,
        sel!(buildSmokeContainerImage:),
        mtm,
    );
    let load = add_button(
        parent,
        "Load OCI",
        rect(58.0 + third * 2.0, y - 114.0, third, 28.0),
        handler,
        sel!(loadContainerImage:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*add_session, setToolTip:
            &*NSString::from_str("Create a GUI session with sensible image defaults")];
        let _: () = msg_send![&*build, setToolTip:
            &*NSString::from_str("Build the bundled example image with Apple Container")];
        let _: () = msg_send![&*pull, setToolTip:
            &*NSString::from_str("Choose a registry, platform, destination, and post-pull action")];
        let _: () = msg_send![&*login, setToolTip:
            &*NSString::from_str("Log in to a private OCI registry through Apple Container")];
        let _: () = msg_send![&*load, setToolTip:
            &*NSString::from_str("Load an OCI image tar archive into Apple Container")];
    }
    y -= 202.0;

    for inventory in inventories {
        add_label(
            parent,
            inventory.runtime,
            rect(34.0, y, width - 68.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        y -= 38.0;

        for row in inventory.rows.iter().take(10) {
            if row.reference.is_none() {
                add_label(
                    parent,
                    &row.label,
                    rect(38.0, y + 4.0, width - 76.0, 18.0),
                    mtm,
                    TextStyle::Caption,
                );
                y -= 40.0;
                continue;
            }
            let selected = selected_image.as_ref().is_some_and(|selected| {
                selected.reference == row.reference.clone().unwrap_or_default()
            });
            add_card(parent, rect(24.0, y - 42.0, width - 48.0, 76.0), mtm);
            if selected {
                add_separator(parent, rect(24.0, y - 42.0, 4.0, 76.0), mtm);
            }
            add_label(
                parent,
                &row.label,
                rect(38.0, y + 12.0, width - 76.0, 18.0),
                mtm,
                if row.reference.is_some() {
                    TextStyle::Mono
                } else {
                    TextStyle::Caption
                },
            );
            if let Some(reference) = row.reference.as_ref() {
                let select_index = {
                    let mut actions = IMAGE_SELECT_ACTIONS.lock().unwrap();
                    let action_index = actions.len();
                    actions.push(SelectedImage {
                        runtime: inventory.runtime.to_string(),
                        runtime_key: inventory.runtime_key.to_string(),
                        reference: reference.clone(),
                        label: row.label.clone(),
                    });
                    action_index
                };
                add_hit_button(
                    parent,
                    rect(24.0, y - 42.0, width - 48.0, 76.0),
                    select_index,
                    handler,
                    sel!(selectContainerImage:),
                    mtm,
                );
                let create_index = {
                    let mut actions = IMAGE_CREATE_ACTIONS.lock().unwrap();
                    let action_index = actions.len();
                    actions.push((inventory.runtime_key.to_string(), reference.clone()));
                    action_index
                };
                let create = add_button(
                    parent,
                    "Add Session",
                    rect(38.0, y - 24.0, 112.0, 28.0),
                    handler,
                    sel!(createContainerSessionFromImage:),
                    mtm,
                );
                unsafe {
                    let _: () = msg_send![&*create, setTag: create_index as isize];
                    let _: () = msg_send![&*create, setToolTip:
                        &*NSString::from_str("Create a GUI session from this image")];
                }
                let delete_index = {
                    let mut actions = IMAGE_DELETE_ACTIONS.lock().unwrap();
                    let action_index = actions.len();
                    actions.push((inventory.runtime_key.to_string(), reference.clone()));
                    action_index
                };
                let delete = add_button(
                    parent,
                    "Delete",
                    rect(160.0, y - 24.0, 78.0, 28.0),
                    handler,
                    sel!(deleteLocalContainerImage:),
                    mtm,
                );
                unsafe {
                    let _: () = msg_send![&*delete, setTag: delete_index as isize];
                    let _: () = msg_send![&*delete, setToolTip:
                        &*NSString::from_str("Delete this local image after confirmation")];
                }
            }
            y -= 86.0;
        }
        y -= 18.0;
    }

    add_label(
        parent,
        "Maintenance",
        rect(34.0, y, width - 68.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    y -= 38.0;
    add_card(parent, rect(24.0, y - 42.0, width - 48.0, 76.0), mtm);
    add_label(
        parent,
        "Delete a local image",
        rect(38.0, y + 12.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Body,
    );
    add_label(
        parent,
        "Destructive action. It is separate from the normal Add Session path.",
        rect(38.0, y - 10.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let delete = add_button(
        parent,
        "Delete Image...",
        rect(38.0, y - 42.0, 120.0, 28.0),
        handler,
        sel!(deleteContainerImage:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*delete, setToolTip:
            &*NSString::from_str("Delete a local image by runtime and reference")];
    }
}

fn add_volumes_list(
    parent: &NSView,
    width: f64,
    content_height: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let inventories = volume_inventories();
    VOLUME_DELETE_ACTIONS.lock().unwrap().clear();
    VOLUME_SELECT_ACTIONS.lock().unwrap().clear();
    let selected_volume = SELECTED_VOLUME.lock().unwrap().clone();
    let mut y = content_height - 58.0;
    add_label(
        parent,
        "Local Volumes",
        rect(34.0, y, width - 68.0, 24.0),
        mtm,
        TextStyle::Title,
    );
    y -= 42.0;

    add_card(parent, rect(24.0, y - 58.0, width - 48.0, 92.0), mtm);
    add_label(
        parent,
        "Volume actions",
        rect(38.0, y + 10.0, width - 76.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Create persistent storage in Apple Container or the active Docker context.",
        rect(38.0, y - 16.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let create = add_button(
        parent,
        "Create Volume...",
        rect(38.0, y - 52.0, 126.0, 28.0),
        handler,
        sel!(createContainerVolume:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*create, setToolTip:
            &*NSString::from_str("Create a named volume in Apple Container or Docker")];
    }
    y -= 126.0;

    for inventory in inventories {
        add_label(
            parent,
            inventory.runtime,
            rect(34.0, y, width - 68.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        y -= 38.0;

        for row in inventory.rows.iter().take(10) {
            if row.name.is_none() {
                add_label(
                    parent,
                    &row.label,
                    rect(38.0, y + 4.0, width - 76.0, 18.0),
                    mtm,
                    TextStyle::Caption,
                );
                y -= 40.0;
                continue;
            }
            let selected = selected_volume
                .as_ref()
                .is_some_and(|selected| selected.name == row.name.clone().unwrap_or_default());
            add_card(parent, rect(24.0, y - 42.0, width - 48.0, 76.0), mtm);
            if selected {
                add_separator(parent, rect(24.0, y - 42.0, 4.0, 76.0), mtm);
            }
            add_label(
                parent,
                &row.label,
                rect(38.0, y + 12.0, width - 76.0, 18.0),
                mtm,
                if row.name.is_some() {
                    TextStyle::Mono
                } else {
                    TextStyle::Caption
                },
            );
            if let Some(name) = row.name.as_ref() {
                let select_index = {
                    let mut actions = VOLUME_SELECT_ACTIONS.lock().unwrap();
                    let action_index = actions.len();
                    actions.push(SelectedVolume {
                        runtime: inventory.runtime.to_string(),
                        runtime_key: inventory.runtime_key.to_string(),
                        name: name.clone(),
                        label: row.label.clone(),
                    });
                    action_index
                };
                add_hit_button(
                    parent,
                    rect(24.0, y - 42.0, width - 48.0, 76.0),
                    select_index,
                    handler,
                    sel!(selectContainerVolume:),
                    mtm,
                );
                let action_index = {
                    let mut actions = VOLUME_DELETE_ACTIONS.lock().unwrap();
                    let action_index = actions.len();
                    actions.push((inventory.runtime_key.to_string(), name.clone()));
                    action_index
                };
                let delete = add_button(
                    parent,
                    "Delete",
                    rect(38.0, y - 24.0, 76.0, 28.0),
                    handler,
                    sel!(deleteLocalContainerVolume:),
                    mtm,
                );
                unsafe {
                    let _: () = msg_send![&*delete, setTag: action_index as isize];
                    let _: () = msg_send![&*delete, setToolTip:
                        &*NSString::from_str("Delete this local volume from the selected runtime")];
                }
            }
            y -= 86.0;
        }
        y -= 18.0;
    }
}

fn add_displays_list(
    parent: &NSView,
    width: f64,
    content_height: f64,
    sessions: &[ContainerSession],
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    MANAGED_DISPLAY_ACTIONS.lock().unwrap().clear();
    let managed_displays = managed_displays_snapshot();
    let pending_displays = pending_managed_displays_snapshot();
    let mut y = content_height - 58.0;
    add_label(
        parent,
        "Managed Displays",
        rect(34.0, y, width - 210.0, 24.0),
        mtm,
        TextStyle::Title,
    );
    add_button(
        parent,
        "Create Display",
        rect(width - 154.0, y - 4.0, 120.0, 30.0),
        handler,
        sel!(createManagedDisplay:),
        mtm,
    );
    y -= 36.0;
    add_label(
        parent,
        "Create a persistent Cocoa-Way window, target it with run_waypipe.sh --display, or assign its slot name to a GUI Session.",
        rect(34.0, y - 30.0, width - 68.0, 44.0),
        mtm,
        TextStyle::Caption,
    );
    y -= 58.0;

    for slot in &pending_displays {
        add_card(parent, rect(24.0, y - 42.0, width - 48.0, 70.0), mtm);
        add_label(
            parent,
            slot,
            rect(38.0, y + 2.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        add_label(
            parent,
            "Starting an independent Wayland display window...",
            rect(38.0, y - 24.0, width - 76.0, 18.0),
            mtm,
            TextStyle::Caption,
        );
        y -= 80.0;
    }

    if managed_displays.is_empty() && pending_displays.is_empty() {
        add_card(parent, rect(24.0, y - 58.0, width - 48.0, 86.0), mtm);
        add_label(
            parent,
            "No managed displays",
            rect(38.0, y + 2.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        add_label(
            parent,
            "The default display still works. Create one when external and GUI-managed connections need explicit allocation.",
            rect(38.0, y - 40.0, width - 76.0, 38.0),
            mtm,
            TextStyle::Caption,
        );
        y -= 98.0;
    }

    for display in &managed_displays {
        let action_index = {
            let mut actions = MANAGED_DISPLAY_ACTIONS.lock().unwrap();
            let index = actions.len();
            actions.push(display.clone());
            index
        };
        add_card(parent, rect(24.0, y - 90.0, width - 48.0, 118.0), mtm);
        add_label(
            parent,
            &display.slot,
            rect(38.0, y + 2.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        add_label(
            parent,
            &format!("{} · display pid {}", display.display, display.pid),
            rect(38.0, y - 22.0, width - 76.0, 18.0),
            mtm,
            TextStyle::Caption,
        );
        add_label(
            parent,
            &short_text(
                &display.runtime_dir,
                chars_for_width(width - 76.0, TextStyle::Mono),
            ),
            rect(38.0, y - 46.0, width - 76.0, 18.0),
            mtm,
            TextStyle::Mono,
        );
        let copy_command = add_button(
            parent,
            "Copy Command",
            rect(38.0, y - 82.0, 112.0, 28.0),
            handler,
            sel!(copyManagedDisplayCommand:),
            mtm,
        );
        let copy_environment = add_button(
            parent,
            "Copy Env",
            rect(160.0, y - 82.0, 92.0, 28.0),
            handler,
            sel!(copyManagedDisplayEnvironment:),
            mtm,
        );
        let close = add_button(
            parent,
            "Close",
            rect(262.0, y - 82.0, 78.0, 28.0),
            handler,
            sel!(closeManagedDisplay:),
            mtm,
        );
        unsafe {
            let _: () = msg_send![&*copy_command, setTag: action_index as isize];
            let _: () = msg_send![&*copy_environment, setTag: action_index as isize];
            let _: () = msg_send![&*close, setTag: action_index as isize];
            let _: () = msg_send![&*copy_command, setToolTip:
                &*NSString::from_str("Copy a run_waypipe.sh command prefix for this display")];
            let _: () = msg_send![&*copy_environment, setToolTip:
                &*NSString::from_str("Copy XDG_RUNTIME_DIR and WAYLAND_DISPLAY exports")];
        }
        y -= 128.0;
    }

    if let Some(error) = MANAGED_DISPLAY_LAST_ERROR.lock().unwrap().as_deref() {
        add_label(
            parent,
            &short_text(error, chars_for_width(width - 68.0, TextStyle::Caption)),
            rect(34.0, y - 20.0, width - 68.0, 34.0),
            mtm,
            TextStyle::Caption,
        );
        y -= 44.0;
    }

    y -= 14.0;
    add_label(
        parent,
        "Built-in Display Slots",
        rect(34.0, y, width - 68.0, 24.0),
        mtm,
        TextStyle::Title,
    );
    y -= 42.0;

    add_card(parent, rect(24.0, y - 118.0, width - 48.0, 148.0), mtm);
    add_label(
        parent,
        "Default Display",
        rect(38.0, y + 8.0, width - 76.0, 22.0),
        mtm,
        TextStyle::Heading,
    );
    let display_body_width = width - 76.0;
    add_label(
        parent,
        "Current Cocoa-Way Wayland socket and Metal window.",
        rect(38.0, y - 20.0, display_body_width, 32.0),
        mtm,
        TextStyle::Body,
    );
    let assigned = sessions
        .iter()
        .filter(|session| {
            matches!(
                resolved_session_display_target(session),
                "automatic" | "default"
            )
        })
        .count();
    let active_assigned = active_display_session_count(sessions);
    add_label(
        parent,
        &format!(
            "{} profile{} eligible for the default display",
            assigned,
            if assigned == 1 { "" } else { "s" }
        ),
        rect(38.0, y - 48.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    add_label(
        parent,
        &format!(
            "{} active GUI session{} currently using this display",
            active_assigned,
            if active_assigned == 1 { "" } else { "s" }
        ),
        rect(38.0, y - 70.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    add_label(
        parent,
        "Auto uses this window while it is free, then creates a dedicated display for additional sessions.",
        rect(38.0, y - 114.0, display_body_width, 32.0),
        mtm,
        TextStyle::Caption,
    );
    y -= 172.0;

    let dedicated = active_sessions_snapshot()
        .into_iter()
        .filter(|active| active.display_slot != "default")
        .collect::<Vec<_>>();
    if !dedicated.is_empty() {
        add_label(
            parent,
            "Active Dedicated Displays",
            rect(34.0, y, width - 68.0, 22.0),
            mtm,
            TextStyle::Heading,
        );
        y -= 38.0;
        for active in dedicated {
            let managed_display = managed_displays
                .iter()
                .find(|display| display.slot == active.display_slot);
            let session_name = sessions
                .get(active.index)
                .map(|session| session.name.as_str())
                .unwrap_or("Unknown session");
            add_card(parent, rect(24.0, y - 36.0, width - 48.0, 68.0), mtm);
            add_label(
                parent,
                session_name,
                rect(38.0, y + 8.0, width - 206.0, 20.0),
                mtm,
                TextStyle::Heading,
            );
            add_label(
                parent,
                &format!(
                    "{} · display pid {} · waypipe pid {}",
                    active.display_slot,
                    active
                        .display_pid
                        .or_else(|| managed_display.map(|display| display.pid))
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "starting".into()),
                    active.waypipe_pid
                ),
                rect(38.0, y - 16.0, width - 206.0, 18.0),
                mtm,
                TextStyle::Caption,
            );
            let stop = add_button(
                parent,
                "Stop",
                rect(width - 124.0, y - 22.0, 76.0, 28.0),
                handler,
                sel!(stopContainerSession:),
                mtm,
            );
            unsafe {
                let _: () = msg_send![&*stop, setTag: active.index as isize];
                let _: () = msg_send![&*stop, setToolTip:
                &*NSString::from_str(if managed_display.is_some() {
                    "Stop this session and keep its managed display available"
                } else {
                    "Stop this session and close its dedicated display"
                })];
            }
            y -= 78.0;
        }
        y -= 18.0;
    }

    add_label(
        parent,
        "Assigned Sessions",
        rect(34.0, y, width - 68.0, 22.0),
        mtm,
        TextStyle::Heading,
    );
    y -= 38.0;
    if sessions.is_empty() {
        add_label(
            parent,
            "No profiles yet. Create a GUI Session and keep Display set to auto.",
            rect(38.0, y, width - 76.0, 22.0),
            mtm,
            TextStyle::Caption,
        );
        return;
    }
    for (index, session) in sessions.iter().enumerate().take(12) {
        let active = active_session(index);
        let text_width = if active.is_some() {
            width - 168.0
        } else {
            width - 76.0
        };
        add_card(parent, rect(24.0, y - 36.0, width - 48.0, 68.0), mtm);
        add_label(
            parent,
            &session.name,
            rect(38.0, y + 8.0, text_width, 20.0),
            mtm,
            TextStyle::Heading,
        );
        add_label(
            parent,
            &format!(
                "{} · {}{}",
                session_display_summary(session),
                session_display_command(session),
                active
                    .as_ref()
                    .map(|active| format!(" · pid {}", active.waypipe_pid))
                    .unwrap_or_default()
            ),
            rect(38.0, y - 16.0, text_width, 18.0),
            mtm,
            TextStyle::Caption,
        );
        if active.is_some() {
            let stop = add_button(
                parent,
                "Stop",
                rect(width - 124.0, y - 22.0, 76.0, 28.0),
                handler,
                sel!(stopContainerSession:),
                mtm,
            );
            unsafe {
                let _: () = msg_send![&*stop, setTag: index as isize];
                let _: () = msg_send![&*stop, setToolTip:
                    &*NSString::from_str("Stop this active session and release its display")];
            }
        }
        y -= 78.0;
    }
}

fn add_runtime_list(
    parent: &NSView,
    width: f64,
    content_height: f64,
    selected_nav: usize,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    RUNTIME_CONTAINER_ACTIONS.lock().unwrap().clear();
    RUNTIME_CONTAINER_SELECT_ACTIONS.lock().unwrap().clear();
    DOCKER_CONTEXT_ACTIONS.lock().unwrap().clear();
    let runtime = match selected_nav {
        NAV_APPLE_CONTAINER => RuntimeInfoTarget {
            title: "Apple Container",
            command: "container",
            checks: vec![
                RuntimeCheck::new("Version", &["--version"]),
                RuntimeCheck::new("System", &["system", "status"]),
                RuntimeCheck::new("Images", &["image", "list"]),
            ],
        },
        NAV_DOCKER => RuntimeInfoTarget {
            title: "Docker",
            command: "docker",
            checks: vec![
                RuntimeCheck::new("Version", &["--version"]),
                RuntimeCheck::new(
                    "Images",
                    &["image", "ls", "--format", "{{.Repository}}:{{.Tag}}"],
                ),
            ],
        },
        _ => RuntimeInfoTarget {
            title: "OrbStack",
            command: "orbctl",
            checks: vec![
                RuntimeCheck::new("Status", &["status"]),
                RuntimeCheck::new("Version", &["version"]),
            ],
        },
    };

    let mut y = content_height - 58.0;
    add_label(
        parent,
        runtime.title,
        rect(34.0, y, width - 180.0, 24.0),
        mtm,
        TextStyle::Title,
    );
    add_runtime_accent(parent, selected_nav, rect(24.0, y + 2.0, 4.0, 24.0), mtm);
    if selected_nav == NAV_APPLE_CONTAINER {
        let open = add_button(
            parent,
            "Open Data Root",
            rect(width - 148.0, y - 4.0, 124.0, 28.0),
            handler,
            sel!(openAppleContainerDataRoot:),
            mtm,
        );
        unsafe {
            let _: () = msg_send![&*open, setToolTip:
                &*NSString::from_str("Open Apple Container's local data directory in Finder")];
        }
    }
    y -= 42.0;

    let child_path = build_child_path();
    let Some(command_path) = find_command_path(runtime.command, &child_path) else {
        let is_orbstack = selected_nav == NAV_ORBSTACK;
        add_card(
            parent,
            rect(
                24.0,
                y - if is_orbstack { 112.0 } else { 78.0 },
                width - 48.0,
                if is_orbstack { 142.0 } else { 108.0 },
            ),
            mtm,
        );
        add_label(
            parent,
            "Missing",
            rect(38.0, y + 8.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        let missing_detail = if is_orbstack {
            "OrbStack's CLI was not found. Open OrbStack once or use the Docker page with an OrbStack context."
                .to_string()
        } else {
            format!("Command `{}` was not found in PATH.", runtime.command)
        };
        add_label(
            parent,
            &missing_detail,
            rect(38.0, y - 18.0, width - 76.0, 36.0),
            mtm,
            TextStyle::Body,
        );
        if is_orbstack {
            let open = add_button(
                parent,
                "Open OrbStack",
                rect(38.0, y - 76.0, 112.0, 28.0),
                handler,
                sel!(openOrbStackApp:),
                mtm,
            );
            unsafe {
                let _: () = msg_send![&*open, setToolTip:
                    &*NSString::from_str("Open OrbStack so its CLI and Docker endpoint become available")];
            }
        }
        return;
    };

    add_card(parent, rect(24.0, y - 58.0, width - 48.0, 88.0), mtm);
    add_label(
        parent,
        "Command",
        rect(38.0, y + 8.0, width - 76.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        &command_path.display().to_string(),
        rect(38.0, y - 18.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Mono,
    );
    y -= 104.0;

    if selected_nav == NAV_APPLE_CONTAINER {
        add_apple_container_system_controls(parent, width, y, handler, mtm);
        y -= 156.0;

        let compatibility = apple_container_compatibility(&command_path, &child_path);
        add_card(parent, rect(24.0, y - 160.0, width - 48.0, 190.0), mtm);
        add_label(
            parent,
            "Compatibility",
            rect(38.0, y + 8.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        add_label(
            parent,
            &compatibility.summary,
            rect(38.0, y - 20.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Body,
        );
        add_label(
            parent,
            &format!(
                "CLI {} · API {} · system {}",
                compatibility.cli_version, compatibility.api_version, compatibility.system_status
            ),
            rect(38.0, y - 48.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Mono,
        );
        add_label(
            parent,
            &format!(
                "Published sockets: {} · resource JSON: {}",
                yes_no(compatibility.publish_socket),
                yes_no(compatibility.stats_json)
            ),
            rect(38.0, y - 76.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Caption,
        );
        add_label(
            parent,
            &compatibility.detail,
            rect(38.0, y - 142.0, width - 76.0, 54.0),
            mtm,
            TextStyle::Caption,
        );
        y -= 206.0;

        let publish_socket_ready = compatibility.publish_socket;

        add_card(parent, rect(24.0, y - 150.0, width - 48.0, 180.0), mtm);
        add_label(
            parent,
            "GUI Transport",
            rect(38.0, y + 8.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        add_label(
            parent,
            if publish_socket_ready {
                "Transport V2 ready"
            } else {
                "Compatibility relay"
            },
            rect(38.0, y - 28.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Body,
        );
        add_label(
            parent,
            if publish_socket_ready {
                "Waypipe data uses Apple Container's published Unix socket. The stdio relay remains an automatic fallback."
            } else {
                "This Apple Container CLI has no --publish-socket support, so GUI launch uses the stdio compatibility relay."
            },
            rect(38.0, y - 112.0, width - 76.0, 70.0),
            mtm,
            TextStyle::Caption,
        );
        y -= 196.0;

        add_apple_container_inventory(parent, width, y, &command_path, &child_path, handler, mtm);
        y -= 340.0;
    } else if selected_nav == NAV_DOCKER {
        add_docker_context_inventory(parent, width, y, &child_path, handler, mtm);
        y -= 206.0;

        add_docker_container_inventory(parent, width, y, &child_path, handler, mtm);
        y -= 318.0;
    } else if selected_nav == NAV_ORBSTACK {
        let running = orbstack_is_running(&command_path, &child_path);
        add_orbstack_machine_inventory(
            parent,
            width,
            y,
            &command_path,
            &child_path,
            running,
            handler,
            mtm,
        );
        y -= 262.0;

        add_orbstack_docker_inventory(parent, width, y, &child_path, running, handler, mtm);
        y -= 316.0;
    }

    for check in runtime.checks {
        add_card(parent, rect(24.0, y - 88.0, width - 48.0, 118.0), mtm);
        add_label(
            parent,
            check.label,
            rect(38.0, y + 8.0, width - 76.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        let lines = command_preview_lines(&command_path, &child_path, check.args);
        let mut line_y = y - 18.0;
        for line in lines.iter().take(4) {
            add_label(
                parent,
                line,
                rect(38.0, line_y, width - 76.0, 18.0),
                mtm,
                TextStyle::Mono,
            );
            line_y -= 20.0;
        }
        y -= 134.0;
    }
}

fn add_apple_container_system_controls(
    parent: &NSView,
    width: f64,
    y: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_card(parent, rect(24.0, y - 110.0, width - 48.0, 140.0), mtm);
    add_label(
        parent,
        "System Controls",
        rect(38.0, y + 8.0, width - 76.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Thin wrappers around Apple Container CLI commands.",
        rect(38.0, y - 24.0, width - 76.0, 24.0),
        mtm,
        TextStyle::Caption,
    );
    let start = add_button(
        parent,
        "Start System",
        rect(38.0, y - 58.0, 104.0, 28.0),
        handler,
        sel!(startAppleContainerSystem:),
        mtm,
    );
    let stop = add_button(
        parent,
        "Stop System",
        rect(152.0, y - 58.0, 104.0, 28.0),
        handler,
        sel!(stopAppleContainerSystem:),
        mtm,
    );
    let status = add_button(
        parent,
        "Copy Status",
        rect(38.0, y - 92.0, 104.0, 28.0),
        handler,
        sel!(copyContainerCommand:),
        mtm,
    );
    let open = add_button(
        parent,
        "Open Data",
        rect(152.0, y - 92.0, 104.0, 28.0),
        handler,
        sel!(openAppleContainerDataRoot:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*start, setToolTip:
            &*NSString::from_str("Run `container system start` asynchronously")];
        let _: () = msg_send![&*stop, setToolTip:
            &*NSString::from_str("Run `container system stop` asynchronously")];
        let _: () = msg_send![&*status, setTag: 0isize];
        let _: () = msg_send![&*status, setToolTip:
            &*NSString::from_str("Copy `container system status` to the clipboard")];
        let _: () = msg_send![&*open, setToolTip:
            &*NSString::from_str("Open Apple Container's local data root in Finder")];
    }
}

fn add_apple_container_inventory(
    parent: &NSView,
    width: f64,
    y: f64,
    command_path: &std::path::Path,
    child_path: &str,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_card(parent, rect(24.0, y - 294.0, width - 48.0, 324.0), mtm);
    add_label(
        parent,
        "Containers",
        rect(38.0, y + 8.0, width - 76.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Start, stop, or delete local instances without leaving Cocoa-Way.",
        rect(38.0, y - 40.0, width - 76.0, 30.0),
        mtm,
        TextStyle::Caption,
    );

    let rows = apple_container_rows(command_path, child_path);
    let mut row_y = y - 82.0;
    for row in rows.iter().take(4) {
        add_runtime_container_row(parent, width, row_y, "apple", row, handler, mtm);
        row_y -= if row.name.is_some() { 54.0 } else { 24.0 };
    }
}

fn apple_container_rows(
    command_path: &std::path::Path,
    child_path: &str,
) -> Vec<RuntimeContainerRow> {
    match run_ui_command(
        command_path,
        child_path,
        &["list", "--all"],
        Duration::from_secs(2),
    ) {
        Ok(output) if output.status.success() => {
            let mut rows = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.trim().is_empty())
                .skip(1)
                .map(parse_apple_container_row)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                rows.push(RuntimeContainerRow {
                    name: None,
                    label: "No local Apple containers".into(),
                    running: false,
                });
            }
            rows
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            vec![RuntimeContainerRow {
                name: None,
                label: stderr
                    .lines()
                    .next()
                    .filter(|line| !line.trim().is_empty())
                    .unwrap_or("container list failed")
                    .to_string(),
                running: false,
            }]
        }
        Err(error) => vec![RuntimeContainerRow {
            name: None,
            label: format!("container list failed: {}", error),
            running: false,
        }],
    }
}

fn parse_apple_container_row(line: &str) -> RuntimeContainerRow {
    let parts = line.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 5 {
        return RuntimeContainerRow {
            name: None,
            label: short_text(line, 96),
            running: false,
        };
    }

    let id = parts[0];
    let image = parts[1];
    let state = parts.get(4).copied().unwrap_or("unknown");
    if id == "buildkit" {
        return RuntimeContainerRow {
            name: None,
            label: format!("BuildKit helper  {}", state),
            running: state.eq_ignore_ascii_case("running"),
        };
    }

    RuntimeContainerRow {
        name: Some(id.to_string()),
        label: format!(
            "{}  {}  {}",
            short_text(id, 28),
            state,
            short_text(image, 38)
        ),
        running: state.eq_ignore_ascii_case("running"),
    }
}

fn add_docker_context_inventory(
    parent: &NSView,
    width: f64,
    y: f64,
    child_path: &str,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_card(parent, rect(24.0, y - 158.0, width - 48.0, 188.0), mtm);
    add_label(
        parent,
        "Docker Context",
        rect(38.0, y + 8.0, width - 76.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Active Docker-compatible endpoint; OrbStack appears when its context is selected.",
        rect(38.0, y - 38.0, width - 76.0, 32.0),
        mtm,
        TextStyle::Caption,
    );

    let rows = docker_context_rows(child_path);
    let mut line_y = y - 76.0;
    for row in rows.iter().take(3) {
        let button_width = if row.name.is_some() && !row.current {
            58.0
        } else {
            0.0
        };
        add_label(
            parent,
            &row.label,
            rect(38.0, line_y, width - 84.0 - button_width, 22.0),
            mtm,
            TextStyle::Mono,
        );
        if let Some(name) = row.name.as_ref().filter(|_| !row.current) {
            let action_index = {
                let mut actions = DOCKER_CONTEXT_ACTIONS.lock().unwrap();
                let index = actions.len();
                actions.push(name.clone());
                index
            };
            let use_button = add_button(
                parent,
                "Use",
                rect(width - 94.0, line_y - 4.0, 52.0, 24.0),
                handler,
                sel!(useDockerContext:),
                mtm,
            );
            unsafe {
                let _: () = msg_send![&*use_button, setTag: action_index as isize];
                let _: () = msg_send![&*use_button, setToolTip:
                    &*NSString::from_str("Make this the active Docker context")];
            }
        }
        line_y -= 32.0;
    }
}

fn add_docker_container_inventory(
    parent: &NSView,
    width: f64,
    y: f64,
    child_path: &str,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_card(parent, rect(24.0, y - 262.0, width - 48.0, 292.0), mtm);
    add_label(
        parent,
        "Containers",
        rect(38.0, y + 8.0, width - 76.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Stop running containers or remove stopped containers below.",
        rect(38.0, y - 28.0, width - 76.0, 18.0),
        mtm,
        TextStyle::Caption,
    );

    let rows = docker_container_rows(child_path);
    let mut row_y = y - 70.0;
    for row in rows.iter().take(4) {
        add_runtime_container_row(parent, width, row_y, "docker", row, handler, mtm);
        row_y -= if row.name.is_some() { 54.0 } else { 24.0 };
    }
}

fn add_orbstack_machine_inventory(
    parent: &NSView,
    width: f64,
    y: f64,
    command_path: &std::path::Path,
    child_path: &str,
    running: bool,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_card(parent, rect(24.0, y - 214.0, width - 48.0, 244.0), mtm);
    add_label(
        parent,
        "Machines",
        rect(38.0, y + 8.0, width - 76.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "OrbStack machines are separate from Docker containers and shown read-only here.",
        rect(38.0, y - 38.0, width - 76.0, 32.0),
        mtm,
        TextStyle::Caption,
    );

    let start = add_button(
        parent,
        "Start OrbStack",
        rect(38.0, y - 76.0, 112.0, 28.0),
        handler,
        sel!(startOrbStack:),
        mtm,
    );
    let stop = add_button(
        parent,
        "Stop OrbStack",
        rect(160.0, y - 76.0, 112.0, 28.0),
        handler,
        sel!(stopOrbStack:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*start, setToolTip:
            &*NSString::from_str("Run `orbctl start` asynchronously")];
        let _: () = msg_send![&*stop, setToolTip:
            &*NSString::from_str("Run `orbctl stop` asynchronously")];
        let _: () = msg_send![&*start, setEnabled: !running];
        let _: () = msg_send![&*stop, setEnabled: running];
    }

    let lines = orbstack_machine_lines(command_path, child_path, running);
    let mut line_y = y - 116.0;
    for line in lines.iter().take(5) {
        add_label(
            parent,
            line,
            rect(38.0, line_y, width - 76.0, 18.0),
            mtm,
            TextStyle::Mono,
        );
        line_y -= 22.0;
    }
}

fn add_orbstack_docker_inventory(
    parent: &NSView,
    width: f64,
    y: f64,
    child_path: &str,
    running: bool,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_card(parent, rect(24.0, y - 266.0, width - 48.0, 296.0), mtm);
    add_label(
        parent,
        "Docker-compatible Containers",
        rect(38.0, y + 8.0, width - 76.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        if running {
            "Uses Docker CLI when the OrbStack context is active."
        } else {
            "OrbStack is stopped. Docker inventory is paused so Cocoa-Way does not wake it."
        },
        rect(38.0, y - 38.0, width - 76.0, 32.0),
        mtm,
        TextStyle::Caption,
    );

    if !running {
        add_label(
            parent,
            "Start OrbStack to inspect its Docker-compatible containers.",
            rect(38.0, y - 82.0, width - 76.0, 36.0),
            mtm,
            TextStyle::Body,
        );
        return;
    }

    let lines = docker_context_lines(child_path);
    let mut line_y = y - 76.0;
    for line in lines.iter().take(2) {
        add_label(
            parent,
            line,
            rect(38.0, line_y, width - 76.0, 18.0),
            mtm,
            TextStyle::Mono,
        );
        line_y -= 22.0;
    }
    let rows = docker_container_rows(child_path);
    line_y -= 12.0;
    for row in rows.iter().take(3) {
        add_runtime_container_row(parent, width, line_y, "orbstack", row, handler, mtm);
        line_y -= if row.name.is_some() { 54.0 } else { 24.0 };
    }
}

fn add_runtime_container_row(
    parent: &NSView,
    width: f64,
    y: f64,
    runtime: &str,
    row: &RuntimeContainerRow,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let Some(name) = row.name.as_ref() else {
        add_label(
            parent,
            &short_text(&row.label, chars_for_width(width - 76.0, TextStyle::Mono)),
            rect(38.0, y, width - 76.0, 18.0),
            mtm,
            TextStyle::Mono,
        );
        return;
    };

    let selected = {
        let mut selected = SELECTED_RUNTIME_CONTAINER.lock().unwrap();
        if let Some(selected) = selected
            .as_mut()
            .filter(|selected| selected.runtime == runtime && selected.name == name.as_str())
        {
            selected.label = row.label.clone();
            selected.running = row.running;
            true
        } else {
            false
        }
    };
    add_card(parent, rect(38.0, y - 36.0, width - 76.0, 46.0), mtm);
    if selected {
        add_runtime_accent(
            parent,
            runtime_nav(runtime),
            rect(38.0, y - 36.0, 4.0, 46.0),
            mtm,
        );
    }
    let text_width = width - 246.0;
    add_label(
        parent,
        &short_text(&row.label, chars_for_width(text_width, TextStyle::Mono)),
        rect(52.0, y - 6.0, text_width, 18.0),
        mtm,
        TextStyle::Mono,
    );

    let select_index = {
        let mut actions = RUNTIME_CONTAINER_SELECT_ACTIONS.lock().unwrap();
        let index = actions.len();
        actions.push(SelectedRuntimeContainer {
            runtime: runtime.to_string(),
            name: name.clone(),
            label: row.label.clone(),
            running: row.running,
        });
        index
    };
    add_hit_button(
        parent,
        rect(38.0, y - 36.0, (width - 236.0).max(96.0), 46.0),
        select_index,
        handler,
        sel!(selectRuntimeContainer:),
        mtm,
    );

    let action_index = push_runtime_container_action(runtime, name);
    let primary = add_button(
        parent,
        if row.running { "Stop" } else { "Start" },
        rect(width - 184.0, y - 12.0, 68.0, 24.0),
        handler,
        if row.running {
            sel!(stopRuntimeContainer:)
        } else {
            sel!(startRuntimeContainer:)
        },
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*primary, setTag: action_index as isize];
        let tooltip = if row.running {
            "Stop this Docker-compatible container"
        } else {
            "Start this Docker-compatible container"
        };
        let _: () = msg_send![&*primary, setToolTip: &*NSString::from_str(tooltip)];
    }

    let action_index = push_runtime_container_action(runtime, name);
    let delete = add_button(
        parent,
        "Delete",
        rect(width - 106.0, y - 12.0, 72.0, 24.0),
        handler,
        sel!(deleteRuntimeContainer:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*delete, setTag: action_index as isize];
        let _: () = msg_send![&*delete, setToolTip:
            &*NSString::from_str("Delete this Docker-compatible container after confirmation")];
    }
}

fn push_runtime_container_action(runtime: &str, name: &str) -> usize {
    let mut actions = RUNTIME_CONTAINER_ACTIONS.lock().unwrap();
    let index = actions.len();
    actions.push((runtime.to_string(), name.to_string()));
    index
}

fn docker_context_lines(child_path: &str) -> Vec<String> {
    docker_context_rows(child_path)
        .into_iter()
        .map(|row| row.label)
        .collect()
}

struct DockerContextRow {
    name: Option<String>,
    label: String,
    current: bool,
}

fn docker_context_rows(child_path: &str) -> Vec<DockerContextRow> {
    let Some(path) = find_command_path("docker", child_path) else {
        return vec![DockerContextRow {
            name: None,
            label: "docker command not found".into(),
            current: false,
        }];
    };

    match run_ui_command(
        &path,
        child_path,
        &[
            "context",
            "ls",
            "--format",
            "{{.Name}}\t{{.Current}}\t{{.DockerEndpoint}}\t{{.Description}}",
        ],
        Duration::from_secs(2),
    ) {
        Ok(output) if output.status.success() => {
            let mut lines = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.trim().is_empty())
                .filter_map(parse_docker_context_line)
                .collect::<Vec<_>>();
            if lines.is_empty() {
                lines.push(DockerContextRow {
                    name: None,
                    label: "No Docker contexts found".into(),
                    current: false,
                });
            }
            lines
        }
        Ok(output) => vec![DockerContextRow {
            name: None,
            label: first_stderr_line(&output, "Docker context list failed"),
            current: false,
        }],
        Err(error) => vec![DockerContextRow {
            name: None,
            label: format!("Docker context list failed: {}", error),
            current: false,
        }],
    }
}

fn parse_docker_context_line(line: &str) -> Option<DockerContextRow> {
    let parts = line.split('\t').map(str::trim).collect::<Vec<_>>();
    let name = parts.first().copied().filter(|name| !name.is_empty())?;
    let current = parts.get(1).is_some_and(|value| *value == "true");
    let endpoint = parts.get(2).copied().unwrap_or_default();
    let description = parts.get(3).copied().unwrap_or_default();
    let detail = if description.is_empty() {
        endpoint
    } else {
        description
    };
    Some(DockerContextRow {
        name: Some(name.to_string()),
        label: format!(
            "{} {}  {}",
            if current { "*" } else { " " },
            name,
            short_text(detail, 52)
        ),
        current,
    })
}

struct RuntimeContainerRow {
    name: Option<String>,
    label: String,
    running: bool,
}

fn docker_container_rows(child_path: &str) -> Vec<RuntimeContainerRow> {
    let Some(path) = find_command_path("docker", child_path) else {
        return vec![RuntimeContainerRow {
            name: None,
            label: "docker command not found".into(),
            running: false,
        }];
    };

    match run_ui_command(
        &path,
        child_path,
        &[
            "ps",
            "-a",
            "--format",
            "{{.Names}}\t{{.State}}\t{{.Status}}\t{{.Image}}",
        ],
        Duration::from_secs(2),
    ) {
        Ok(output) if output.status.success() => {
            let mut rows = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(format_docker_container_row)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                rows.push(RuntimeContainerRow {
                    name: None,
                    label: "No Docker containers to stop or delete".into(),
                    running: false,
                });
            }
            rows
        }
        Ok(output) => vec![RuntimeContainerRow {
            name: None,
            label: first_stderr_line(&output, "Docker container list failed"),
            running: false,
        }],
        Err(error) => vec![RuntimeContainerRow {
            name: None,
            label: format!("Docker container list failed: {}", error),
            running: false,
        }],
    }
}

fn orbstack_is_running(command_path: &std::path::Path, child_path: &str) -> bool {
    run_ui_command(
        command_path,
        child_path,
        &["status"],
        Duration::from_secs(2),
    )
    .ok()
    .filter(|output| output.status.success())
    .is_some_and(|output| {
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .eq_ignore_ascii_case("running")
    })
}

fn orbstack_machine_lines(
    command_path: &std::path::Path,
    child_path: &str,
    running: bool,
) -> Vec<String> {
    let status = command_preview_lines(command_path, child_path, &["status"])
        .into_iter()
        .next()
        .unwrap_or_else(|| "Status unknown".into());
    let mut lines = vec![format!("Status: {}", short_text(&status, 70))];

    if !running {
        lines.push("Inventory paused while OrbStack is stopped".into());
        return lines;
    }

    match run_ui_command(command_path, child_path, &["list"], Duration::from_secs(2)) {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let machine_lines = stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| short_text(line, 92));
            lines.extend(machine_lines);
        }
        Ok(output) => lines.push(first_stderr_line(&output, "Machine list failed")),
        Err(error) => lines.push(format!("Machine list failed: {}", error)),
    }
    if lines.len() == 1 {
        lines.push("No machines listed".into());
    }
    lines
}

fn format_docker_container_row(line: &str) -> RuntimeContainerRow {
    let parts = line.split('\t').collect::<Vec<_>>();
    if parts.len() < 4 {
        return RuntimeContainerRow {
            name: None,
            label: short_text(line, 96),
            running: false,
        };
    }
    let name = parts[0].trim().to_string();
    let state = parts[1].trim();
    let status = parts[2].trim();
    let image = parts[3].trim();
    RuntimeContainerRow {
        name: Some(name.clone()),
        label: format!(
            "{}  {}  {}",
            short_text(&name, 26),
            short_text(status, 32),
            short_text(image, 30)
        ),
        running: matches!(state, "running" | "restarting" | "paused"),
    }
}

struct RuntimeInfoTarget {
    title: &'static str,
    command: &'static str,
    checks: Vec<RuntimeCheck>,
}

struct RuntimeCheck {
    label: &'static str,
    args: &'static [&'static str],
}

impl RuntimeCheck {
    fn new(label: &'static str, args: &'static [&'static str]) -> Self {
        Self { label, args }
    }
}

fn run_ui_command(
    command: &std::path::Path,
    child_path: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, String> {
    let mut child = Command::new(command)
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
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Command timed out after {}ms", timeout.as_millis()));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn apple_container_compatibility(
    command: &std::path::Path,
    child_path: &str,
) -> AppleContainerCompatibility {
    if let Some((checked_at, compatibility)) = APPLE_COMPATIBILITY_CACHE
        .lock()
        .unwrap()
        .as_ref()
        .filter(|(checked_at, _)| checked_at.elapsed() < Duration::from_secs(5))
    {
        let _ = checked_at;
        return compatibility.clone();
    }

    let version_output =
        run_ui_command(command, child_path, &["--version"], Duration::from_secs(1))
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
    let cli_version = extract_version(&version_output).unwrap_or_else(|| "unknown".into());

    let status_json = run_ui_command(
        command,
        child_path,
        &["system", "status", "--format", "json"],
        Duration::from_secs(2),
    )
    .ok()
    .filter(|output| output.status.success())
    .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok());
    let system_status = status_json
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unavailable")
        .to_string();
    let api_version = status_json
        .as_ref()
        .and_then(|value| value.get("apiServerVersion"))
        .and_then(serde_json::Value::as_str)
        .and_then(extract_version)
        .unwrap_or_else(|| "unknown".into());

    let publish_socket = container_sessions::apple_publish_socket_supported(command, child_path);
    let stats_help = run_ui_command(
        command,
        child_path,
        &["stats", "--help"],
        Duration::from_secs(1),
    )
    .ok()
    .filter(|output| output.status.success())
    .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
    .unwrap_or_default();
    let stats_json = stats_help.contains("--format") && stats_help.contains("--no-stream");

    let versions_match =
        cli_version == "unknown" || api_version == "unknown" || cli_version == api_version;
    let at_least_1_1 = version_at_least(&cli_version, (1, 1, 0));
    let summary = if !versions_match {
        "Client/API version mismatch".into()
    } else if !publish_socket {
        "Legacy transport fallback required".into()
    } else if system_status != "running" {
        "Installed; management service is not running".into()
    } else {
        "Compatible with Cocoa-Way".into()
    };
    let detail = if !versions_match {
        "The CLI and API server differ. Restart or reinstall Apple Container before launching GUI sessions."
            .into()
    } else if !publish_socket {
        "Cocoa-Way can use its compatibility relay, but Transport V2 requires `container run --publish-socket`."
            .into()
    } else if at_least_1_1 {
        "Apple Container 1.1+ includes the non-root Unix-socket permission fix used by Transport V2."
            .into()
    } else {
        "Apple Container 1.0 is supported. Version 1.1+ is recommended for non-root published sockets."
            .into()
    };

    let compatibility = AppleContainerCompatibility {
        cli_version,
        api_version,
        system_status,
        publish_socket,
        stats_json,
        summary,
        detail,
    };
    *APPLE_COMPATIBILITY_CACHE.lock().unwrap() = Some((Instant::now(), compatibility.clone()));
    compatibility
}

fn extract_version(text: &str) -> Option<String> {
    text.split(|character: char| character.is_whitespace() || character == '(')
        .map(|token| {
            token.trim_matches(|character: char| !character.is_ascii_digit() && character != '.')
        })
        .find(|token| {
            let parts = token.split('.').collect::<Vec<_>>();
            parts.len() >= 3
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
}

fn version_at_least(version: &str, minimum: (u64, u64, u64)) -> bool {
    let mut parts = version
        .split('.')
        .filter_map(|part| part.parse::<u64>().ok());
    let current = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    current >= minimum
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn command_preview_lines(
    command: &std::path::Path,
    child_path: &str,
    args: &[&str],
) -> Vec<String> {
    match run_ui_command(command, child_path, args, Duration::from_secs(2)) {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let lines = stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .take(8)
                .map(|line| line.to_string())
                .collect::<Vec<_>>();
            if lines.is_empty() {
                vec!["OK".into()]
            } else {
                lines
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            vec![
                stderr
                    .lines()
                    .next()
                    .filter(|line| !line.trim().is_empty())
                    .unwrap_or("Command failed")
                    .to_string(),
            ]
        }
        Err(error) => vec![format!("Command failed: {}", error)],
    }
}

fn resource_preview_lines(
    runtime_key: &str,
    resource: &str,
    action: &str,
    name: &str,
) -> Vec<String> {
    let child_path = build_child_path();
    let command_name = match runtime_key {
        "container" | "apple" | "apple container" => "container",
        "docker" => "docker",
        _ => return vec!["Unsupported runtime for inspect preview".into()],
    };
    let Some(command_path) = find_command_path(command_name, &child_path) else {
        return vec![format!("Command `{}` was not found in PATH.", command_name)];
    };
    let args = [resource, action, name];
    command_preview_lines(&command_path, &child_path, &args)
        .into_iter()
        .map(|line| short_text(&line, 96))
        .collect()
}

fn command_items() -> Vec<String> {
    let config_path = container_sessions::config_path();
    vec![
        format!("container system status"),
        format!("container image list"),
        smoke_image_build_command(),
        format!("container image pull docker.io/library/alpine:3.20"),
        format!("container image load --input /tmp/cocoa-way-niri.tar"),
        format!("docker image ls"),
        format!(
            "docker buildx build -f examples/container-images/Containerfile.niri --output type=oci,dest=/tmp/cocoa-way-niri.tar ."
        ),
        format!(
            "open '{}'",
            apple_container_data_root().replace('\'', "'\\''")
        ),
        format!("open -R {}", config_path.display()),
    ]
}

fn add_commands_list(
    parent: &NSView,
    width: f64,
    content_height: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let commands = command_items();
    let mut y = content_height - 58.0;
    add_label(
        parent,
        "Useful Commands",
        rect(34.0, y, width - 68.0, 24.0),
        mtm,
        TextStyle::Title,
    );
    y -= 42.0;
    for (index, command) in commands.iter().enumerate() {
        add_card(parent, rect(24.0, y - 18.0, width - 48.0, 48.0), mtm);
        add_label(
            parent,
            command,
            rect(38.0, y, width - 142.0, 18.0),
            mtm,
            TextStyle::Mono,
        );
        let copy = add_button(
            parent,
            "Copy",
            rect(width - 86.0, y - 6.0, 62.0, 28.0),
            handler,
            sel!(copyContainerCommand:),
            mtm,
        );
        unsafe {
            let _: () = msg_send![&*copy, setTag: index as isize];
            let _: () = msg_send![&*copy, setToolTip:
                &*NSString::from_str("Copy this command to the clipboard")];
        }
        y -= 62.0;
    }
}

struct ImageInventory {
    runtime: &'static str,
    runtime_key: &'static str,
    rows: Vec<ImageRow>,
}

struct ImageRow {
    label: String,
    reference: Option<String>,
}

struct VolumeInventory {
    runtime: &'static str,
    runtime_key: &'static str,
    rows: Vec<VolumeRow>,
}

struct VolumeRow {
    label: String,
    name: Option<String>,
}

fn image_inventories() -> Vec<ImageInventory> {
    if let Some((task, detail)) = image_task_active() {
        let mut rows = vec![ImageRow::message(task)];
        if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
            rows.push(ImageRow::message(detail));
        }
        return vec![ImageInventory {
            runtime: "Apple Container",
            runtime_key: "container",
            rows,
        }];
    }

    let child_path = build_child_path();
    vec![
        ImageInventory {
            runtime: "Apple Container",
            runtime_key: "container",
            rows: apple_container_image_rows(&child_path),
        },
        ImageInventory {
            runtime: "Docker-compatible Context",
            runtime_key: "docker",
            rows: docker_image_rows(&child_path),
        },
    ]
}

fn apple_registry_login_summary(child_path: &str) -> String {
    let Some(path) = find_command_path("container", child_path) else {
        return "Registry login unavailable: Apple Container is not installed.".into();
    };
    let output = Command::new(path)
        .env("PATH", child_path)
        .args(["registry", "list", "--quiet"])
        .output();
    let Ok(output) = output else {
        return "Registry login status unavailable.".into();
    };
    if !output.status.success() {
        return "Registry login status unavailable.".into();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let registries = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>();
    if registries.is_empty() {
        "Public pulls ready; no private registry login saved.".into()
    } else {
        format!("Signed in: {}", registries.join(", "))
    }
}

fn volume_inventories() -> Vec<VolumeInventory> {
    let child_path = build_child_path();
    vec![
        VolumeInventory {
            runtime: "Apple Container",
            runtime_key: "container",
            rows: apple_container_volume_rows(&child_path),
        },
        VolumeInventory {
            runtime: "Docker-compatible Context",
            runtime_key: "docker",
            rows: docker_volume_rows(&child_path),
        },
    ]
}

fn apple_container_image_rows(child_path: &str) -> Vec<ImageRow> {
    let Some(path) = find_command_path("container", child_path) else {
        return vec![ImageRow::message("container not installed")];
    };

    match run_ui_command(
        &path,
        child_path,
        &["image", "list"],
        Duration::from_secs(2),
    ) {
        Ok(output) if output.status.success() => {
            let rows = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(parse_apple_container_image_line)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                vec![ImageRow::message("No local images found")]
            } else {
                rows
            }
        }
        Ok(output) => vec![ImageRow::message(first_stderr_line(
            &output,
            "Image list failed",
        ))],
        Err(error) => vec![ImageRow::message(format!("Image list failed: {}", error))],
    }
}

fn apple_container_volume_rows(child_path: &str) -> Vec<VolumeRow> {
    let Some(path) = find_command_path("container", child_path) else {
        return vec![VolumeRow::message("container not installed")];
    };

    match run_ui_command(
        &path,
        child_path,
        &["volume", "list"],
        Duration::from_secs(2),
    ) {
        Ok(output) if output.status.success() => {
            let rows = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(parse_volume_line)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                vec![VolumeRow::message("No local volumes found")]
            } else {
                rows
            }
        }
        Ok(output) => vec![VolumeRow::message(first_stderr_line(
            &output,
            "Volume list failed",
        ))],
        Err(error) => vec![VolumeRow::message(format!("Volume list failed: {}", error))],
    }
}

fn docker_image_rows(child_path: &str) -> Vec<ImageRow> {
    let Some(path) = find_command_path("docker", child_path) else {
        return vec![ImageRow::message("docker not installed")];
    };
    match run_ui_command(
        &path,
        child_path,
        &[
            "image",
            "ls",
            "--format",
            "{{.Repository}}:{{.Tag}}\t{{.ID}}\t{{.Size}}",
        ],
        Duration::from_secs(2),
    ) {
        Ok(output) if output.status.success() => {
            let rows = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(parse_docker_image_line)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                vec![ImageRow::message("No Docker-compatible images found")]
            } else {
                rows
            }
        }
        Ok(output) => vec![ImageRow::message(first_stderr_line(
            &output,
            "Docker image list failed",
        ))],
        Err(error) => vec![ImageRow::message(format!(
            "Docker image list failed: {}",
            error
        ))],
    }
}

fn docker_volume_rows(child_path: &str) -> Vec<VolumeRow> {
    let Some(path) = find_command_path("docker", child_path) else {
        return vec![VolumeRow::message("docker not installed")];
    };
    match run_ui_command(
        &path,
        child_path,
        &["volume", "ls", "--format", "{{.Name}}\t{{.Driver}}"],
        Duration::from_secs(2),
    ) {
        Ok(output) if output.status.success() => {
            let rows = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(parse_volume_line)
                .collect::<Vec<_>>();
            if rows.is_empty() {
                vec![VolumeRow::message("No Docker-compatible volumes found")]
            } else {
                rows
            }
        }
        Ok(output) => vec![VolumeRow::message(first_stderr_line(
            &output,
            "Docker volume list failed",
        ))],
        Err(error) => vec![VolumeRow::message(format!(
            "Docker volume list failed: {}",
            error
        ))],
    }
}

fn parse_volume_line(line: &str) -> Option<VolumeRow> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let name = trimmed.split_whitespace().next()?;
    if name.eq_ignore_ascii_case("name") {
        return None;
    }
    Some(VolumeRow {
        label: trimmed.to_string(),
        name: Some(name.to_string()),
    })
}

fn parse_apple_container_image_line(line: &str) -> Option<ImageRow> {
    let columns = line.split_whitespace().collect::<Vec<_>>();
    if columns.is_empty() || columns[0].eq_ignore_ascii_case("name") {
        return None;
    }

    let name = columns[0];
    let tag = columns.get(1).copied().unwrap_or_default();
    let digest = columns.get(2).copied().unwrap_or_default();
    let reference = if tag.is_empty() || tag == "<none>" {
        name.to_string()
    } else {
        format!("{}:{}", name, tag)
    };
    let label = if digest.is_empty() {
        reference.clone()
    } else {
        format!("{}    {}", reference, digest)
    };

    Some(ImageRow {
        label,
        reference: Some(reference),
    })
}

fn parse_docker_image_line(line: &str) -> Option<ImageRow> {
    let columns = line.split('\t').map(str::trim).collect::<Vec<_>>();
    let tagged_reference = columns.first().copied().filter(|value| !value.is_empty())?;
    let id = columns.get(1).copied().unwrap_or_default();
    let size = columns.get(2).copied().unwrap_or_default();
    let reference = if tagged_reference.starts_with("<none>:") {
        id.to_string()
    } else {
        tagged_reference.to_string()
    };
    if reference.is_empty() {
        return None;
    }
    let metadata = [id, size]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("    ");
    Some(ImageRow {
        label: if metadata.is_empty() {
            reference.clone()
        } else {
            format!("{}    {}", reference, metadata)
        },
        reference: Some(reference),
    })
}

impl ImageRow {
    fn message(message: impl Into<String>) -> Self {
        Self {
            label: message.into(),
            reference: None,
        }
    }
}

impl VolumeRow {
    fn message(message: impl Into<String>) -> Self {
        Self {
            label: message.into(),
            name: None,
        }
    }
}

fn first_stderr_line(output: &Output, fallback: &str) -> String {
    let line = String::from_utf8_lossy(&output.stderr)
        .lines()
        .next()
        .filter(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .to_string();
    if line.contains("Cannot connect to the Docker daemon") {
        "Docker daemon is offline. Start Docker or OrbStack, then Reload.".into()
    } else {
        short_text(&line, 82)
    }
}

fn add_detail_panel(
    parent: &NSView,
    x: f64,
    _y: f64,
    width: f64,
    height: f64,
    selected_tab: usize,
    selected_nav: usize,
    selected_session: Option<(usize, &ContainerSession)>,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let compact_toolbar = width < 560.0;
    let open_w = if compact_toolbar { 64.0 } else { 112.0 };
    let open_x = x + width - open_w - 24.0;
    add_button(
        parent,
        if compact_toolbar {
            "Config"
        } else {
            "Open Config"
        },
        rect(open_x, height - 47.0, open_w, 30.0),
        handler,
        sel!(openContainerConfig:),
        mtm,
    );
    if selected_nav == NAV_SESSIONS {
        let tab_x = if compact_toolbar { x + 50.0 } else { x + 70.0 };
        let max_tab_w = (open_x - tab_x - 18.0).max(190.0);
        let tab_w = max_tab_w.min(360.0);
        add_button(
            parent,
            "+",
            rect(
                if compact_toolbar { x + 12.0 } else { x + 22.0 },
                height - 47.0,
                if compact_toolbar { 30.0 } else { 34.0 },
                30.0,
            ),
            handler,
            sel!(addContainerSession:),
            mtm,
        );
        add_tab_bar(
            parent,
            tab_x,
            height - 50.0,
            tab_w,
            selected_tab,
            handler,
            mtm,
        );
    } else {
        add_label(
            parent,
            nav_title(selected_nav),
            rect(x + 28.0, height - 44.0, open_x - x - 52.0, 26.0),
            mtm,
            TextStyle::Title,
        );
    }
    add_separator(parent, rect(x, height - 64.0, width, 1.0), mtm);

    let compact_summary = width < 560.0;
    let summary_height = if compact_summary { 108.0 } else { 72.0 };
    let scroll_height = (height - 64.0 - summary_height).max(240.0);
    let scroll = unsafe {
        NSScrollView::initWithFrame(
            mtm.alloc::<NSScrollView>(),
            rect(x, summary_height, width, scroll_height),
        )
    };
    unsafe {
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
    }
    let selected_runtime = SELECTED_RUNTIME_CONTAINER
        .lock()
        .unwrap()
        .clone()
        .filter(|container| runtime_nav(&container.runtime) == selected_nav);
    let document_width = (width - 14.0).max(320.0);
    let document_height = if selected_session.is_some() {
        820.0_f64.max(scroll_height)
    } else if selected_runtime.is_some() {
        860.0_f64.max(scroll_height)
    } else if matches!(selected_nav, NAV_IMAGES | NAV_VOLUMES) {
        700.0_f64.max(scroll_height)
    } else {
        scroll_height
    };
    let document: Retained<NSView> = unsafe {
        msg_send_id![
            mtm.alloc::<NSView>(),
            initWithFrame: rect(0.0, 0.0, document_width, document_height)
        ]
    };

    if let Some((index, session)) = selected_session {
        add_session_detail(
            &document,
            index,
            session,
            session_state(index).as_ref(),
            0.0,
            0.0,
            document_width,
            document_height,
            selected_tab,
            handler,
            mtm,
        );
    } else if selected_nav == NAV_SESSIONS {
        let content_x = 42.0;
        let content_y = document_height * 0.52;
        add_label(
            &document,
            "No Selection",
            rect(content_x, content_y, document_width - 84.0, 42.0),
            mtm,
            TextStyle::Hero,
        );
        add_label(
            &document,
            detail_empty_message(selected_tab),
            rect(content_x, content_y - 40.0, document_width - 84.0, 40.0),
            mtm,
            TextStyle::Body,
        );
    } else if selected_nav == NAV_IMAGES {
        if let Some(image) = SELECTED_IMAGE.lock().unwrap().clone() {
            add_image_detail(
                &document,
                &image,
                0.0,
                0.0,
                document_width,
                document_height,
                handler,
                mtm,
            );
        } else {
            add_section_detail(
                &document,
                nav_title(selected_nav),
                0.0,
                0.0,
                document_width,
                document_height,
                mtm,
            );
        }
    } else if selected_nav == NAV_VOLUMES {
        if let Some(volume) = SELECTED_VOLUME.lock().unwrap().clone() {
            add_volume_detail(
                &document,
                &volume,
                0.0,
                0.0,
                document_width,
                document_height,
                handler,
                mtm,
            );
        } else {
            add_section_detail(
                &document,
                nav_title(selected_nav),
                0.0,
                0.0,
                document_width,
                document_height,
                mtm,
            );
        }
    } else if let Some(container) = selected_runtime {
        add_runtime_container_detail(
            &document,
            &container,
            0.0,
            0.0,
            document_width,
            document_height,
            handler,
            mtm,
        );
    } else {
        add_section_detail(
            &document,
            nav_title(selected_nav),
            0.0,
            0.0,
            document_width,
            document_height,
            mtm,
        );
    }
    unsafe {
        scroll.setDocumentView(Some(&document));
        let clip_view: Retained<AnyObject> = msg_send_id![&*scroll, contentView];
        let top_y = (document_height - scroll_height).max(0.0);
        let _: () = msg_send![&*clip_view, scrollToPoint: NSPoint { x: 0.0, y: top_y }];
        let _: () = msg_send![&*scroll, reflectScrolledClipView: &*clip_view];
        parent.addSubview(&scroll);
    }
    add_separator(parent, rect(x, summary_height, width, 1.0), mtm);
    add_runtime_summary(parent, x + 28.0, 4.0, width - 56.0, compact_summary, mtm);
}

fn add_session_detail(
    parent: &NSView,
    index: usize,
    session: &ContainerSession,
    state: Option<&SessionState>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    selected_tab: usize,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let content_x = x + 42.0;
    let header_y = y + height - 124.0;
    add_label(
        parent,
        &session.name,
        rect(content_x, header_y, width - 84.0, 34.0),
        mtm,
        TextStyle::Title,
    );
    add_label(
        parent,
        &format!(
            "{} session backed by {}",
            runtime_label(&session.runtime),
            session.image
        ),
        rect(content_x, header_y - 28.0, width - 84.0, 20.0),
        mtm,
        TextStyle::Body,
    );
    let actions_y = header_y - 72.0;
    let compact_actions = width < 520.0;
    let secondary_y = actions_y - 40.0;
    let active = active_session(index);
    let missing_image = state.is_some_and(is_missing_image_state);
    let transport_blocked = session_has_apple_transport_block(session);
    let stop_enabled = active.is_some() || session_can_stop(state);
    let launch_busy = active.is_some() || session_is_launch_busy(state);
    let check = add_button(
        parent,
        "Check",
        rect(content_x, actions_y, 82.0, 30.0),
        handler,
        sel!(checkContainerSession:),
        mtm,
    );
    let primary_label = if launch_busy {
        state.map(|state| state.label).unwrap_or("Running")
    } else if transport_blocked {
        "Blocked"
    } else if missing_image {
        if is_smoke_image_reference(&session.image) {
            "Build"
        } else if is_local_image_reference(&session.image) {
            "Load OCI"
        } else {
            "Pull"
        }
    } else {
        "Launch"
    };
    let primary_selector = if launch_busy || transport_blocked {
        sel!(checkContainerSession:)
    } else if missing_image {
        if is_smoke_image_reference(&session.image) {
            sel!(buildSmokeContainerSessionImage:)
        } else if is_local_image_reference(&session.image) {
            sel!(loadContainerSessionImage:)
        } else {
            sel!(pullContainerSessionImage:)
        }
    } else {
        sel!(launchContainerSession:)
    };
    let primary_tooltip = if launch_busy {
        "This session is already active. Stop it before launching again."
    } else if transport_blocked {
        "Apple Container GUI relay is currently unavailable"
    } else if missing_image {
        if is_smoke_image_reference(&session.image) {
            "Build the bundled example image with Apple Container before launching"
        } else if is_local_image_reference(&session.image) {
            "Load an OCI archive into Apple Container before launching"
        } else {
            "Pull the missing image before launching"
        }
    } else {
        "Launch this GUI session"
    };
    let primary = add_button(
        parent,
        primary_label,
        rect(content_x + 92.0, actions_y, 88.0, 30.0),
        handler,
        primary_selector,
        mtm,
    );
    let stop = add_button(
        parent,
        "Stop",
        rect(content_x + 190.0, actions_y, 78.0, 30.0),
        handler,
        sel!(stopContainerSession:),
        mtm,
    );
    let delete = add_button(
        parent,
        "Delete Profile",
        if compact_actions {
            rect(content_x + 196.0, secondary_y, 106.0, 28.0)
        } else {
            rect(content_x + 278.0, actions_y, 112.0, 30.0)
        },
        handler,
        sel!(deleteContainerSession:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*check, setTag: index as isize];
        let _: () = msg_send![&*check, setToolTip:
            &*NSString::from_str("Run preflight checks without launching")];
        let _: () = msg_send![&*primary, setTag: index as isize];
        let _: () = msg_send![&*primary, setToolTip:
            &*NSString::from_str(primary_tooltip)];
        if launch_busy || transport_blocked {
            let _: () = msg_send![&*primary, setEnabled: false];
        }
        let _: () = msg_send![&*stop, setTag: index as isize];
        let _: () = msg_send![&*stop, setToolTip:
            &*NSString::from_str("Stop the tracked or named container session")];
        if !stop_enabled {
            let _: () = msg_send![&*stop, setEnabled: false];
        }
        let _: () = msg_send![&*delete, setTag: index as isize];
        let _: () = msg_send![&*delete, setToolTip:
            &*NSString::from_str("Remove this GUI profile from container-sessions.toml; images remain installed")];
    }

    let edit = add_button(
        parent,
        "Edit Profile",
        rect(
            content_x,
            secondary_y,
            if compact_actions { 92.0 } else { 104.0 },
            28.0,
        ),
        handler,
        sel!(editContainerSession:),
        mtm,
    );
    let duplicate = add_button(
        parent,
        "Duplicate",
        rect(
            content_x + if compact_actions { 102.0 } else { 114.0 },
            secondary_y,
            if compact_actions { 84.0 } else { 96.0 },
            28.0,
        ),
        handler,
        sel!(duplicateContainerSession:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*edit, setTag: index as isize];
        let _: () = msg_send![&*edit, setToolTip:
            &*NSString::from_str("Edit this profile. Running containers are not changed until next launch.")];
        let _: () = msg_send![&*duplicate, setTag: index as isize];
        let _: () = msg_send![&*duplicate, setToolTip:
            &*NSString::from_str("Create a copy of this profile for another image, command, or display target")];
    }

    let panel_top = secondary_y - 24.0;
    match selected_tab {
        1 => add_session_logs(
            parent,
            index,
            session,
            state,
            content_x,
            panel_top - 292.0,
            width - 84.0,
            mtm,
        ),
        2 => add_session_terminal(
            parent,
            index,
            session,
            content_x,
            panel_top - 226.0,
            width - 84.0,
            handler,
            mtm,
        ),
        3 => add_session_files(
            parent,
            session,
            content_x,
            panel_top - 178.0,
            width - 84.0,
            mtm,
        ),
        _ => add_session_info(
            parent,
            index,
            session,
            state,
            content_x,
            panel_top - 334.0,
            width - 84.0,
            handler,
            mtm,
        ),
    }
}

fn add_session_info(
    parent: &NSView,
    index: usize,
    session: &ContainerSession,
    state: Option<&SessionState>,
    x: f64,
    y: f64,
    width: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_detail_card(parent, x, y, width, 334.0, mtm);
    let derived_transport_blocked = session_has_apple_transport_block(session);
    let active = active_session(index);
    let mut rows = vec![
        (
            "Status".to_string(),
            active
                .as_ref()
                .map(|_| "Running".to_string())
                .or_else(|| state.map(|state| state.label.to_string()))
                .unwrap_or_else(|| {
                    if derived_transport_blocked {
                        "Blocked".into()
                    } else {
                        "Not launched".into()
                    }
                }),
        ),
        (
            "Status detail".to_string(),
            active
                .as_ref()
                .map(|active| {
                    let container_pid = active
                        .container_pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "Apple Container".into());
                    format!(
                        "tracked: container {}; waypipe {}",
                        container_pid, active.waypipe_pid
                    )
                })
                .or_else(|| state.map(|state| compact_detail(&state.detail)))
                .unwrap_or_else(|| {
                    if derived_transport_blocked {
                        compact_detail(&apple_transport_blocked_detail(session))
                    } else {
                        "Run Check to validate before launch.".into()
                    }
                }),
        ),
        (
            "Runtime".to_string(),
            runtime_label(&session.runtime).to_string(),
        ),
        ("Display".to_string(), session_display_summary(session)),
        ("Image".to_string(), session.image.clone()),
        (
            "Container name".to_string(),
            container_sessions::container_name(session),
        ),
        (
            "Profile".to_string(),
            session
                .profile
                .as_deref()
                .unwrap_or("single-app")
                .to_string(),
        ),
        ("Command".to_string(), session_display_command(session)),
        ("Waypipe".to_string(), session_waypipe_summary(session)),
        (
            "Display use".to_string(),
            display_occupancy_summary(index, session),
        ),
    ];
    if let Some(socket) = session.socket.as_deref().filter(|value| !value.is_empty()) {
        rows.push(("Host socket".to_string(), socket.to_string()));
    }
    if let Some(socket) = session
        .container_socket
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        rows.push(("Container socket".to_string(), socket.to_string()));
    }
    if !session.runtime_args.is_empty() {
        rows.push((
            "Runtime args".to_string(),
            list_or_empty(&session.runtime_args),
        ));
    }
    if !session.mounts.is_empty() {
        rows.push(("Mounts".to_string(), list_or_empty(&session.mounts)));
    }
    if !session.env.is_empty() {
        rows.push(("Environment".to_string(), list_or_empty(&session.env)));
    }

    let mut row_y = y + 290.0;
    for (key, value) in rows {
        add_detail_row(parent, &key, &value, x + 22.0, row_y, width - 44.0, mtm);
        row_y -= 26.0;
    }

    if state.is_some_and(is_missing_image_state) {
        add_missing_image_card(parent, index, session, x, y - 98.0, width, handler, mtm);
    } else if state.is_some_and(is_apple_container_stopped_state) {
        add_apple_container_stopped_card(parent, x, y - 98.0, width, handler, mtm);
    } else if derived_transport_blocked {
        add_apple_container_transport_card(parent, x, y - 112.0, width, mtm);
    } else {
        add_display_note_card(parent, index, session, x, y - 98.0, width, mtm);
    }
}

fn is_missing_image_state(state: &SessionState) -> bool {
    state.detail.contains("not available locally")
}

fn is_apple_container_stopped_state(state: &SessionState) -> bool {
    state.detail.contains("container system start")
        || state.detail.contains("reports running")
        || state.detail.contains("not running")
}

fn is_local_image_reference(image: &str) -> bool {
    image.starts_with("localhost/") || image.starts_with("localhost:") || !image.contains('/')
}

fn is_smoke_image_reference(image: &str) -> bool {
    image.contains("cocoa-way-niri") || image.contains("cocoa-way-smoke")
}

fn smoke_image_build_command() -> String {
    format!(
        "container build -f {} -t {} {}",
        smoke_containerfile_path(),
        smoke_image_reference(),
        smoke_build_context()
    )
}

fn add_missing_image_card(
    parent: &NSView,
    index: usize,
    session: &ContainerSession,
    x: f64,
    y: f64,
    width: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let local_image = is_local_image_reference(&session.image);
    add_detail_card(parent, x, y, width, 96.0, mtm);
    add_label(
        parent,
        if local_image {
            "Local image missing"
        } else {
            "Missing image"
        },
        rect(x + 22.0, y + 60.0, width - 44.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    let message = if local_image {
        format!(
            "'{}' is a local tag. Build/export OCI, then load it.",
            short_text(&session.image, 48)
        )
    } else {
        format!(
            "'{}' is not in Apple Container yet.",
            short_text(&session.image, 54)
        )
    };
    add_label(
        parent,
        &message,
        rect(x + 22.0, y + 36.0, width - 44.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let mut button_x = x + 22.0;
    if local_image && is_smoke_image_reference(&session.image) {
        let build = add_button(
            parent,
            "Build",
            rect(button_x, y + 6.0, 78.0, 28.0),
            handler,
            sel!(buildSmokeContainerSessionImage:),
            mtm,
        );
        unsafe {
            let _: () = msg_send![&*build, setTag: index as isize];
            let _: () = msg_send![&*build, setToolTip:
                &*NSString::from_str("Build the bundled example image with Apple Container")];
        }
        button_x += 88.0;
    } else if !local_image {
        let pull = add_button(
            parent,
            "Pull Image",
            rect(button_x, y + 6.0, 96.0, 28.0),
            handler,
            sel!(pullContainerSessionImage:),
            mtm,
        );
        unsafe {
            let _: () = msg_send![&*pull, setTag: index as isize];
            let _: () = msg_send![&*pull, setToolTip:
                &*NSString::from_str("Pull the session image with Apple Container")];
        }
        button_x += 106.0;
    }
    let load = add_button(
        parent,
        "Load OCI",
        rect(button_x, y + 6.0, 88.0, 28.0),
        handler,
        sel!(loadContainerSessionImage:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*load, setTag: index as isize];
        let _: () = msg_send![&*load, setToolTip:
            &*NSString::from_str("Load an OCI archive into Apple Container")];
    }
    if local_image && is_smoke_image_reference(&session.image) {
        let build = add_button(
            parent,
            "Copy Build Cmd",
            rect(button_x + 98.0, y + 6.0, 128.0, 28.0),
            handler,
            sel!(copySmokeImageBuildCommand:),
            mtm,
        );
        unsafe {
            let _: () = msg_send![&*build, setToolTip:
                &*NSString::from_str("Copy an Apple Container build command for the bundled example image")];
        }
    }
}

fn add_apple_container_stopped_card(
    parent: &NSView,
    x: f64,
    y: f64,
    width: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_detail_card(parent, x, y, width, 96.0, mtm);
    add_label(
        parent,
        "Apple Container is stopped",
        rect(x + 22.0, y + 60.0, width - 44.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Start the Apple Container system, then run Check again.",
        rect(x + 22.0, y + 36.0, width - 44.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let start = add_button(
        parent,
        "Start System",
        rect(x + 22.0, y + 6.0, 112.0, 28.0),
        handler,
        sel!(startAppleContainerSystem:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*start, setToolTip:
            &*NSString::from_str("Run `container system start`")];
    }
}

fn add_display_note_card(
    parent: &NSView,
    index: usize,
    session: &ContainerSession,
    x: f64,
    y: f64,
    width: f64,
    mtm: MainThreadMarker,
) {
    let requested = session_display_target(session);
    let active = active_session(index);
    let (title, detail, behavior) = if let Some(active) = active {
        if active.display_slot == "default" {
            (
                "Default display".to_string(),
                "This session is using the current Cocoa-Way display window.".to_string(),
                "Stop it to release the default display for another auto session.".to_string(),
            )
        } else {
            (
                format!("Dedicated display: {}", active.display_slot),
                "This session owns an independent Metal window and Wayland socket.".to_string(),
                "Stopping the session also closes and cleans up its display worker.".to_string(),
            )
        }
    } else if requested == "auto" {
        (
            "Automatic display".to_string(),
            "Auto uses the default Cocoa-Way window when it is available.".to_string(),
            "If it is occupied, Cocoa-Way creates a dedicated display automatically.".to_string(),
        )
    } else if requested == "default" {
        (
            "Default display".to_string(),
            "This profile always targets the current Cocoa-Way display window.".to_string(),
            "Launch is blocked while another session owns the default display.".to_string(),
        )
    } else {
        (
            format!("Dedicated display: {}", requested),
            "This profile launches in an independent Cocoa-Way display window.".to_string(),
            "The named display is recreated on launch and cleaned up on stop.".to_string(),
        )
    };
    add_detail_card(parent, x, y, width, 96.0, mtm);
    add_label(
        parent,
        &title,
        rect(x + 22.0, y + 60.0, width - 44.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        &detail,
        rect(x + 22.0, y + 36.0, width - 44.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    add_label(
        parent,
        &behavior,
        rect(x + 22.0, y + 18.0, width - 44.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
}

fn compact_detail(value: &str) -> String {
    const MAX: usize = 120;
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX {
        normalized
    } else {
        let mut truncated = normalized.chars().take(MAX - 3).collect::<String>();
        truncated.push_str("...");
        truncated
    }
}

fn add_session_logs(
    parent: &NSView,
    index: usize,
    session: &ContainerSession,
    state: Option<&SessionState>,
    x: f64,
    y: f64,
    width: f64,
    mtm: MainThreadMarker,
) {
    add_detail_card(parent, x, y, width, 292.0, mtm);
    add_label(
        parent,
        "Launch Logs",
        rect(x + 22.0, y + 244.0, width - 44.0, 24.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Live stdout/stderr from the container runtime and waypipe client is captured for this app session.",
        rect(x + 22.0, y + 212.0, width - 44.0, 30.0),
        mtm,
        TextStyle::Body,
    );
    let logs = session_logs(index);
    if logs.is_empty() {
        let fallback = state
            .map(|state| state.detail.clone())
            .unwrap_or_else(|| format!("No process output captured for '{}'.", session.name));
        add_label(
            parent,
            &fallback,
            rect(x + 22.0, y + 170.0, width - 44.0, 26.0),
            mtm,
            TextStyle::Caption,
        );
        return;
    }

    let max_chars = ((width - 44.0) / 10.0).floor().clamp(48.0, 120.0) as usize;
    let visible_lines = wrapped_log_lines(&logs, max_chars, 8);
    let mut row_y = y + 176.0;
    for line in visible_lines {
        add_label(
            parent,
            &line,
            rect(x + 22.0, row_y, width - 44.0, 18.0),
            mtm,
            TextStyle::Mono,
        );
        row_y -= 22.0;
    }
}

fn add_session_terminal(
    parent: &NSView,
    index: usize,
    session: &ContainerSession,
    x: f64,
    y: f64,
    width: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_detail_card(parent, x, y, width, 226.0, mtm);
    add_label(
        parent,
        "Terminal Bridge",
        rect(x + 22.0, y + 178.0, width - 44.0, 24.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Open a macOS Terminal shell inside the running GUI container. Launch the session first, then attach here.",
        rect(x + 22.0, y + 142.0, width - 44.0, 34.0),
        mtm,
        TextStyle::Body,
    );
    add_label(
        parent,
        &format!(
            "Target runtime: {}    container: {}",
            runtime_label(&session.runtime),
            container_sessions::container_name(session)
        ),
        rect(x + 22.0, y + 104.0, width - 44.0, 24.0),
        mtm,
        TextStyle::Caption,
    );
    add_label(
        parent,
        &container_sessions::terminal_command(session),
        rect(x + 22.0, y + 70.0, width - 44.0, 20.0),
        mtm,
        TextStyle::Mono,
    );
    let button = add_button(
        parent,
        "Open Shell",
        rect(x + 22.0, y + 24.0, 116.0, 30.0),
        handler,
        sel!(openContainerTerminal:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*button, setTag: index as isize];
        let _: () = msg_send![&*button, setToolTip:
            &*NSString::from_str("Open macOS Terminal and attach to this running GUI container")];
    }
}

fn add_session_files(
    parent: &NSView,
    session: &ContainerSession,
    x: f64,
    y: f64,
    width: f64,
    mtm: MainThreadMarker,
) {
    add_detail_card(parent, x, y, width, 178.0, mtm);
    add_label(
        parent,
        "Shared Files",
        rect(x + 22.0, y + 130.0, width - 44.0, 24.0),
        mtm,
        TextStyle::Heading,
    );
    if session.mounts.is_empty() {
        add_label(
            parent,
            "Declare mounts in container-sessions.toml to share project folders with this GUI session.",
            rect(x + 22.0, y + 94.0, width - 44.0, 34.0),
            mtm,
            TextStyle::Body,
        );
        add_label(
            parent,
            &format!("No file mounts are declared for '{}'.", session.name),
            rect(x + 22.0, y + 54.0, width - 44.0, 26.0),
            mtm,
            TextStyle::Caption,
        );
    } else {
        add_label(
            parent,
            "Mounted folders are passed to the container runtime at launch.",
            rect(x + 22.0, y + 98.0, width - 44.0, 28.0),
            mtm,
            TextStyle::Body,
        );
        let mut row_y = y + 64.0;
        for mount in session.mounts.iter().take(3) {
            add_label(
                parent,
                mount,
                rect(x + 22.0, row_y, width - 44.0, 18.0),
                mtm,
                TextStyle::Mono,
            );
            row_y -= 22.0;
        }
    }
}

fn add_apple_container_transport_card(
    parent: &NSView,
    x: f64,
    y: f64,
    width: f64,
    mtm: MainThreadMarker,
) {
    add_detail_card(parent, x, y, width, 116.0, mtm);
    add_label(
        parent,
        "GUI relay unavailable",
        rect(x + 22.0, y + 78.0, width - 44.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        "Apple Container GUI launch needs Transport V2 or the stdio compatibility relay.",
        rect(x + 22.0, y + 42.0, width - 44.0, 34.0),
        mtm,
        TextStyle::Caption,
    );
    add_label(
        parent,
        "Run Check, then inspect Logs if Launch cannot start the relay.",
        rect(x + 22.0, y + 18.0, width - 44.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
}

fn add_image_detail(
    parent: &NSView,
    image: &SelectedImage,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let content_x = x + 42.0;
    let header_y = y + height - 124.0;
    add_label(
        parent,
        &short_text(&image.reference, 52),
        rect(content_x, header_y, width - 84.0, 34.0),
        mtm,
        TextStyle::Title,
    );
    add_label(
        parent,
        &format!("{} image in the local runtime store", image.runtime),
        rect(content_x, header_y - 28.0, width - 84.0, 20.0),
        mtm,
        TextStyle::Body,
    );

    let create_index = {
        let mut actions = IMAGE_CREATE_ACTIONS.lock().unwrap();
        let action_index = actions.len();
        actions.push((image.runtime_key.clone(), image.reference.clone()));
        action_index
    };
    let delete_index = {
        let mut actions = IMAGE_DELETE_ACTIONS.lock().unwrap();
        let action_index = actions.len();
        actions.push((image.runtime_key.clone(), image.reference.clone()));
        action_index
    };
    let create = add_button(
        parent,
        "Add Session",
        rect(content_x, header_y - 72.0, 112.0, 30.0),
        handler,
        sel!(createContainerSessionFromImage:),
        mtm,
    );
    let delete = add_button(
        parent,
        "Delete",
        rect(content_x + 124.0, header_y - 72.0, 84.0, 30.0),
        handler,
        sel!(deleteLocalContainerImage:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*create, setTag: create_index as isize];
        let _: () = msg_send![&*create, setToolTip:
            &*NSString::from_str("Create a GUI session from this image")];
        let _: () = msg_send![&*delete, setTag: delete_index as isize];
        let _: () = msg_send![&*delete, setToolTip:
            &*NSString::from_str("Delete this local image after confirmation")];
    }

    let card_y = header_y - 246.0;
    add_detail_card(parent, content_x, card_y, width - 84.0, 144.0, mtm);
    add_detail_row(
        parent,
        "Runtime",
        &image.runtime,
        content_x + 22.0,
        card_y + 104.0,
        width - 128.0,
        mtm,
    );
    add_detail_row(
        parent,
        "Reference",
        &image.reference,
        content_x + 22.0,
        card_y + 78.0,
        width - 128.0,
        mtm,
    );
    add_detail_row(
        parent,
        "Source row",
        &image.label,
        content_x + 22.0,
        card_y + 52.0,
        width - 128.0,
        mtm,
    );

    let inspect_y = card_y - 190.0;
    add_detail_card(parent, content_x, inspect_y, width - 84.0, 158.0, mtm);
    add_label(
        parent,
        "Inspect Preview",
        rect(content_x + 22.0, inspect_y + 118.0, width - 128.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    let lines = resource_preview_lines(&image.runtime_key, "image", "inspect", &image.reference);
    let mut line_y = inspect_y + 90.0;
    for line in lines.iter().take(4) {
        add_label(
            parent,
            line,
            rect(content_x + 22.0, line_y, width - 128.0, 18.0),
            mtm,
            TextStyle::Mono,
        );
        line_y -= 22.0;
    }
}

fn add_volume_detail(
    parent: &NSView,
    volume: &SelectedVolume,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let content_x = x + 42.0;
    let header_y = y + height - 124.0;
    add_label(
        parent,
        &short_text(&volume.name, 52),
        rect(content_x, header_y, width - 84.0, 34.0),
        mtm,
        TextStyle::Title,
    );
    add_label(
        parent,
        &format!("{} local volume", volume.runtime),
        rect(content_x, header_y - 28.0, width - 84.0, 20.0),
        mtm,
        TextStyle::Body,
    );

    let delete_index = {
        let mut actions = VOLUME_DELETE_ACTIONS.lock().unwrap();
        let action_index = actions.len();
        actions.push((volume.runtime_key.clone(), volume.name.clone()));
        action_index
    };
    let delete = add_button(
        parent,
        "Delete Volume",
        rect(content_x, header_y - 72.0, 124.0, 30.0),
        handler,
        sel!(deleteLocalContainerVolume:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*delete, setTag: delete_index as isize];
        let _: () = msg_send![&*delete, setToolTip:
            &*NSString::from_str("Delete this volume after confirmation")];
    }

    let card_y = header_y - 220.0;
    add_detail_card(parent, content_x, card_y, width - 84.0, 118.0, mtm);
    add_detail_row(
        parent,
        "Runtime",
        &volume.runtime,
        content_x + 22.0,
        card_y + 78.0,
        width - 128.0,
        mtm,
    );
    add_detail_row(
        parent,
        "Name",
        &volume.name,
        content_x + 22.0,
        card_y + 52.0,
        width - 128.0,
        mtm,
    );
    add_detail_row(
        parent,
        "Source row",
        &volume.label,
        content_x + 22.0,
        card_y + 26.0,
        width - 128.0,
        mtm,
    );

    let inspect_y = card_y - 190.0;
    add_detail_card(parent, content_x, inspect_y, width - 84.0, 158.0, mtm);
    add_label(
        parent,
        "Inspect Preview",
        rect(content_x + 22.0, inspect_y + 118.0, width - 128.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    let lines = resource_preview_lines(&volume.runtime_key, "volume", "inspect", &volume.name);
    let mut line_y = inspect_y + 90.0;
    for line in lines.iter().take(4) {
        add_label(
            parent,
            line,
            rect(content_x + 22.0, line_y, width - 128.0, 18.0),
            mtm,
            TextStyle::Mono,
        );
        line_y -= 22.0;
    }
}

fn add_runtime_container_detail(
    parent: &NSView,
    container: &SelectedRuntimeContainer,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let content_x = x + 42.0;
    let content_width = width - 84.0;
    let header_y = y + height - 124.0;
    add_label(
        parent,
        &short_text(
            &container.name,
            chars_for_width(content_width, TextStyle::Title),
        ),
        rect(content_x, header_y, content_width, 34.0),
        mtm,
        TextStyle::Title,
    );
    add_label(
        parent,
        &format!(
            "{} container · {}",
            runtime_label(&container.runtime),
            if container.running {
                "Running"
            } else {
                "Stopped"
            }
        ),
        rect(content_x, header_y - 28.0, content_width, 20.0),
        mtm,
        TextStyle::Body,
    );

    let action_index = push_runtime_container_action(&container.runtime, &container.name);
    let primary = add_button(
        parent,
        if container.running { "Stop" } else { "Start" },
        rect(content_x, header_y - 72.0, 88.0, 30.0),
        handler,
        if container.running {
            sel!(stopRuntimeContainer:)
        } else {
            sel!(startRuntimeContainer:)
        },
        mtm,
    );
    let restart = add_button(
        parent,
        "Restart",
        rect(content_x + 98.0, header_y - 72.0, 88.0, 30.0),
        handler,
        sel!(restartRuntimeContainer:),
        mtm,
    );
    let terminal = add_button(
        parent,
        "Terminal",
        rect(content_x + 196.0, header_y - 72.0, 96.0, 30.0),
        handler,
        sel!(openRuntimeContainerTerminal:),
        mtm,
    );
    let refresh = add_button(
        parent,
        "Refresh",
        rect(content_x, header_y - 110.0, 88.0, 30.0),
        handler,
        sel!(refreshRuntimeContainerDetails:),
        mtm,
    );
    let delete = add_button(
        parent,
        "Delete",
        rect(content_x + 98.0, header_y - 110.0, 88.0, 30.0),
        handler,
        sel!(deleteRuntimeContainer:),
        mtm,
    );
    unsafe {
        for button in [&primary, &restart, &terminal, &delete] {
            let _: () = msg_send![&**button, setTag: action_index as isize];
        }
        let _: () = msg_send![&*primary, setToolTip:
            &*NSString::from_str(if container.running { "Stop this container" } else { "Start this container" })];
        let _: () = msg_send![&*restart, setToolTip:
            &*NSString::from_str("Restart this container and refresh its details")];
        let _: () = msg_send![&*terminal, setToolTip:
            &*NSString::from_str("Open an interactive shell in macOS Terminal")];
        let _: () = msg_send![&*refresh, setToolTip:
            &*NSString::from_str("Reload inspect, resource, and recent log output")];
        let _: () = msg_send![&*delete, setToolTip:
            &*NSString::from_str("Delete this container after confirmation")];
        let _: () = msg_send![&*restart, setEnabled: container.running];
        let _: () = msg_send![&*terminal, setEnabled: container.running];
    }

    let details = RUNTIME_CONTAINER_DETAILS
        .lock()
        .unwrap()
        .clone()
        .filter(|details| details.runtime == container.runtime && details.name == container.name);
    let mut info = details
        .as_ref()
        .map(|details| details.info.clone())
        .unwrap_or_else(|| vec!["Loading runtime details...".into()]);
    if let Some(error) = details.as_ref().and_then(|details| details.error.as_ref()) {
        info.push(format!("Warning: {}", error));
    }
    if info.is_empty() {
        info.push(container.label.clone());
    }
    let stats = details
        .as_ref()
        .map(|details| details.stats.clone())
        .unwrap_or_else(|| vec!["Waiting for a one-shot resource sample...".into()]);
    let logs = details
        .as_ref()
        .map(|details| details.logs.clone())
        .unwrap_or_else(|| vec!["Waiting for recent container logs...".into()]);

    let info_y = header_y - 286.0;
    add_runtime_output_card(
        parent,
        "Info",
        &info,
        content_x,
        info_y,
        content_width,
        140.0,
        4,
        mtm,
    );
    let stats_y = info_y - 136.0;
    add_runtime_output_card(
        parent,
        "Resources",
        &stats,
        content_x,
        stats_y,
        content_width,
        112.0,
        3,
        mtm,
    );
    let logs_y = stats_y - 250.0;
    add_runtime_output_card(
        parent,
        "Recent Logs",
        &logs,
        content_x,
        logs_y,
        content_width,
        226.0,
        8,
        mtm,
    );
}

#[allow(clippy::too_many_arguments)]
fn add_runtime_output_card(
    parent: &NSView,
    title: &str,
    lines: &[String],
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    max_rows: usize,
    mtm: MainThreadMarker,
) {
    add_detail_card(parent, x, y, width, height, mtm);
    add_label(
        parent,
        title,
        rect(x + 22.0, y + height - 38.0, width - 44.0, 20.0),
        mtm,
        TextStyle::Heading,
    );
    let max_chars = chars_for_width(width - 44.0, TextStyle::Mono);
    let visible = wrapped_log_lines(lines, max_chars, max_rows);
    let mut line_y = y + height - 66.0;
    for line in visible {
        add_label(
            parent,
            &line,
            rect(x + 22.0, line_y, width - 44.0, 18.0),
            mtm,
            TextStyle::Mono,
        );
        line_y -= 22.0;
    }
}

fn add_section_detail(
    parent: &NSView,
    title: &str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    mtm: MainThreadMarker,
) {
    let detail = match title {
        "Images" => "Create sessions, pull/load images, and keep cleanup separate.",
        "Volumes" => "Inspect local volumes. Deletes ask for confirmation.",
        "Displays" => {
            "Create managed display windows for scripts and explicit session assignment, or let auto use the default window and allocate dedicated displays."
        }
        "Activity" => "Recent Container Mode actions and runtime output are shown on the left.",
        "Commands" => {
            "Copy launch helper commands from the left when you need to debug outside the GUI."
        }
        "Docker" => {
            "Use the left pane to inspect Docker-compatible containers and stop/delete visible entries."
        }
        "OrbStack" => {
            "Use the left pane to inspect OrbStack state and manage Docker-compatible containers."
        }
        _ => "Runtime status and diagnostics are shown on the left.",
    };
    let content_x = x + 42.0;
    let content_y = y + height * 0.52;
    add_label(
        parent,
        title,
        rect(content_x, content_y, width - 84.0, 42.0),
        mtm,
        TextStyle::Hero,
    );
    add_label(
        parent,
        detail,
        rect(content_x, content_y - 48.0, width - 84.0, 42.0),
        mtm,
        TextStyle::Body,
    );
}

fn add_detail_card(
    parent: &NSView,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    mtm: MainThreadMarker,
) {
    add_card(parent, rect(x, y, width, height), mtm);
}

fn add_detail_row(
    parent: &NSView,
    key: &str,
    value: &str,
    x: f64,
    y: f64,
    width: f64,
    mtm: MainThreadMarker,
) {
    add_label(
        parent,
        key,
        rect(x, y, 112.0, 18.0),
        mtm,
        TextStyle::Caption,
    );
    let value_width = (width - 124.0).max(80.0);
    let value = short_text(value, chars_for_width(value_width, TextStyle::Body));
    add_label(
        parent,
        &value,
        rect(x + 124.0, y - 1.0, value_width, 20.0),
        mtm,
        TextStyle::Body,
    );
}

fn add_tab_bar(
    parent: &NSView,
    x: f64,
    y: f64,
    width: f64,
    selected_tab: usize,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    add_card(parent, rect(x, y, width, 36.0), mtm);
    let tab_w = width / 4.0;
    add_card(
        parent,
        rect(
            x + selected_tab as f64 * tab_w + 4.0,
            y + 4.0,
            tab_w - 8.0,
            28.0,
        ),
        mtm,
    );
    for (index, title) in ["Info", "Logs", "Terminal", "Files"].iter().enumerate() {
        let label_width = tab_w - 16.0;
        add_label(
            parent,
            title,
            rect(x + index as f64 * tab_w + 8.0, y + 9.0, label_width, 18.0),
            mtm,
            if index == selected_tab {
                TextStyle::Heading
            } else {
                TextStyle::Body
            },
        );
        add_hit_button(
            parent,
            rect(x + index as f64 * tab_w, y, tab_w, 36.0),
            index,
            handler,
            sel!(selectContainerTab:),
            mtm,
        );
    }
}

fn add_runtime_summary(
    parent: &NSView,
    x: f64,
    y: f64,
    width: f64,
    compact: bool,
    mtm: MainThreadMarker,
) {
    let diagnostics = runtime_diagnostics(&[]);
    *RUNTIME_FPS_LABEL.lock().unwrap() = None;
    let columns = if compact { 3 } else { diagnostics.len().max(1) };
    let item_w = width / columns as f64;
    for (i, diagnostic) in diagnostics.iter().enumerate() {
        let column = if compact { i % columns } else { i };
        let row_y = if compact && i < columns { y + 48.0 } else { y };
        let item_x = x + column as f64 * item_w;
        add_label(
            parent,
            diagnostic.name,
            rect(item_x, row_y + 30.0, item_w - 14.0, 16.0),
            mtm,
            TextStyle::Caption,
        );
        let value_label = add_label(
            parent,
            &short_text(
                &diagnostic.state,
                chars_for_width(item_w - 14.0, TextStyle::Heading),
            ),
            rect(item_x, row_y + 10.0, item_w - 14.0, 20.0),
            mtm,
            TextStyle::Heading,
        );
        if diagnostic.name == "FPS" {
            *RUNTIME_FPS_LABEL.lock().unwrap() =
                Some((&*value_label as *const NSTextField) as usize);
        }
    }
}

unsafe fn update_runtime_fps_label() {
    let Some(label_ptr) = *RUNTIME_FPS_LABEL.lock().unwrap() else {
        return;
    };
    let label = unsafe { &*(label_ptr as *const NSTextField) };
    let state = performance_diagnostic().state;
    let _: () = unsafe { msg_send![label, setStringValue: &*NSString::from_str(&state)] };
}

fn nav_title(index: usize) -> &'static str {
    match index {
        NAV_IMAGES => "Images",
        NAV_VOLUMES => "Volumes",
        NAV_DISPLAYS => "Displays",
        NAV_APPLE_CONTAINER => "Apple Container",
        NAV_DOCKER => "Docker",
        NAV_ORBSTACK => "OrbStack",
        NAV_ACTIVITY => "Activity",
        NAV_COMMANDS => "Commands",
        _ => "GUI Sessions",
    }
}

fn detail_empty_message(index: usize) -> &'static str {
    match index {
        1 => "Select a GUI session to inspect its launch logs and waypipe output.",
        2 => "Select a GUI session to open an interactive terminal bridge.",
        3 => "Select a GUI session to inspect files exported from the container.",
        _ => "Select a GUI session to inspect launch details, runtime, image, and command.",
    }
}

fn add_placeholder_list(
    parent: &NSView,
    width: f64,
    content_height: f64,
    title: &str,
    mtm: MainThreadMarker,
) {
    let center_y = (content_height * 0.54).max(250.0);
    add_label(
        parent,
        &format!("No {}", title),
        rect(34.0, center_y, width - 68.0, 34.0),
        mtm,
        TextStyle::Title,
    );
    add_label(
        parent,
        "No local resources are available for this section.",
        rect(34.0, center_y - 44.0, width - 68.0, 54.0),
        mtm,
        TextStyle::Body,
    );
}

struct RuntimeDiagnostic {
    name: &'static str,
    state: String,
}

fn runtime_diagnostics(_sessions: &[ContainerSession]) -> Vec<RuntimeDiagnostic> {
    let child_path = build_child_path();
    vec![
        command_diagnostic("waypipe", "waypipe", &child_path),
        apple_container_diagnostic(&child_path),
        apple_gui_transport_diagnostic(&child_path),
        performance_diagnostic(),
        disk_diagnostic(&child_path),
    ]
}

fn performance_diagnostic() -> RuntimeDiagnostic {
    let state = performance_snapshot()
        .map(|snapshot| format!("{:.1} fps", snapshot.redraw_fps))
        .unwrap_or_else(|| "Waiting".into());
    RuntimeDiagnostic { name: "FPS", state }
}

fn apple_container_diagnostic(child_path: &str) -> RuntimeDiagnostic {
    let Some(path) = find_command_path("container", child_path) else {
        return RuntimeDiagnostic {
            name: "Apple Mgmt",
            state: "Missing".into(),
        };
    };

    let _ = path;
    let state = if crate::diagnostics::resource_snapshot().available {
        "Running"
    } else {
        "Installed"
    };

    RuntimeDiagnostic {
        name: "Apple Mgmt",
        state: state.into(),
    }
}

fn apple_gui_transport_diagnostic(child_path: &str) -> RuntimeDiagnostic {
    let Some(container) = find_command_path("container", child_path) else {
        return RuntimeDiagnostic {
            name: "GUI Transport",
            state: "Missing".into(),
        };
    };

    RuntimeDiagnostic {
        name: "GUI Transport",
        state: if container_sessions::apple_publish_socket_supported(&container, child_path) {
            "V2 Ready".into()
        } else {
            "Fallback".into()
        },
    }
}

fn command_diagnostic(label: &'static str, command: &str, child_path: &str) -> RuntimeDiagnostic {
    match find_command_path(command, child_path) {
        Some(_path) => RuntimeDiagnostic {
            name: label,
            state: "Ready".into(),
        },
        None => RuntimeDiagnostic {
            name: label,
            state: "Missing".into(),
        },
    }
}

fn disk_diagnostic(child_path: &str) -> RuntimeDiagnostic {
    let _ = child_path;
    let state = crate::diagnostics::available_disk_bytes()
        .map(|bytes| format_disk_state(bytes / 1024))
        .unwrap_or_else(|| "Unknown".into());
    RuntimeDiagnostic {
        name: "Disk",
        state,
    }
}

fn apple_container_data_root() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users".into());
    format!("{}/Library/Application Support/com.apple.container", home)
}

fn format_disk_state(available_kib: u64) -> String {
    let available_gib = available_kib as f64 / 1024.0 / 1024.0;
    if available_gib < 8.0 {
        format!("Low {:.1}G free", available_gib)
    } else {
        format!("{:.1}G free", available_gib)
    }
}

unsafe fn add_session_row(
    parent: &NSView,
    session: &ContainerSession,
    index: usize,
    active: bool,
    state: Option<SessionState>,
    y: f64,
    width: f64,
    handler: *mut AnyObject,
    mtm: MainThreadMarker,
) {
    let card_w = (width - 24.0).max(260.0);
    let process_active = active_session(index).is_some();
    let state_label = state
        .as_ref()
        .map(|state| state.label)
        .unwrap_or(if process_active {
            "Running"
        } else if session_has_apple_transport_block(session) {
            "Blocked"
        } else {
            "Not launched"
        });
    let missing_image = state.as_ref().is_some_and(is_missing_image_state);
    let transport_blocked = session_has_apple_transport_block(session);
    let launch_busy = process_active || session_is_launch_busy(state.as_ref());
    add_card(parent, rect(12.0, y + 10.0, card_w, 122.0), mtm);
    if active {
        add_separator(parent, rect(12.0, y + 10.0, 4.0, 122.0), mtm);
    }
    add_label(
        parent,
        &session.name,
        rect(32.0, y + 96.0, card_w - 138.0, 24.0),
        mtm,
        TextStyle::Heading,
    );
    add_label(
        parent,
        &format!("{} · {}", state_label, runtime_label(&session.runtime)),
        rect(32.0, y + 70.0, card_w - 138.0, 20.0),
        mtm,
        TextStyle::Body,
    );
    add_label(
        parent,
        &short_image_label(&session.image, 42),
        rect(32.0, y + 46.0, card_w - 138.0, 20.0),
        mtm,
        TextStyle::Body,
    );
    add_label(
        parent,
        &format!(
            "{} · display {}",
            session_display_command(session),
            session_display_summary(session)
        ),
        rect(32.0, y + 22.0, card_w - 138.0, 20.0),
        mtm,
        TextStyle::Caption,
    );
    add_hit_button(
        parent,
        rect(12.0, y + 10.0, card_w - 108.0, 122.0),
        index,
        handler,
        sel!(selectContainerSession:),
        mtm,
    );

    let primary_label = if launch_busy {
        state.as_ref().map(|state| state.label).unwrap_or("Running")
    } else if transport_blocked {
        "Blocked"
    } else if missing_image {
        if is_smoke_image_reference(&session.image) {
            "Build"
        } else if is_local_image_reference(&session.image) {
            "Load OCI"
        } else {
            "Pull"
        }
    } else {
        "Launch"
    };
    let primary_selector = if launch_busy || transport_blocked {
        sel!(checkContainerSession:)
    } else if missing_image {
        if is_smoke_image_reference(&session.image) {
            sel!(buildSmokeContainerSessionImage:)
        } else if is_local_image_reference(&session.image) {
            sel!(loadContainerSessionImage:)
        } else {
            sel!(pullContainerSessionImage:)
        }
    } else {
        sel!(launchContainerSession:)
    };
    let primary_tooltip = if launch_busy {
        "This session is already active. Stop it before launching again."
    } else if transport_blocked {
        "Apple Container GUI relay is currently unavailable"
    } else if missing_image {
        if is_smoke_image_reference(&session.image) {
            "Build the bundled example image with Apple Container before launching"
        } else if is_local_image_reference(&session.image) {
            "Load an OCI archive into Apple Container before launching"
        } else {
            "Pull the missing image before launching"
        }
    } else {
        "Start this session through Cocoa-Way's compositor event loop"
    };
    let primary = add_button(
        parent,
        primary_label,
        rect(card_w - 94.0, y + 78.0, 82.0, 30.0),
        handler,
        primary_selector,
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*primary, setTag: index as isize];
        let _: () = msg_send![&*primary, setToolTip:
            &*NSString::from_str(primary_tooltip)];
        if launch_busy || transport_blocked {
            let _: () = msg_send![&*primary, setEnabled: false];
        }
    }

    let check = add_button(
        parent,
        "Check",
        rect(card_w - 94.0, y + 42.0, 82.0, 30.0),
        handler,
        sel!(checkContainerSession:),
        mtm,
    );
    unsafe {
        let _: () = msg_send![&*check, setTag: index as isize];
        let _: () = msg_send![&*check, setToolTip:
            &*NSString::from_str("Run preflight checks without launching the session")];
    }

    if process_active || matches!(state_label, "Running" | "Stopping") {
        let stop = add_button(
            parent,
            "Stop",
            rect(card_w - 94.0, y + 6.0, 82.0, 30.0),
            handler,
            sel!(stopContainerSession:),
            mtm,
        );
        unsafe {
            let _: () = msg_send![&*stop, setTag: index as isize];
            let _: () = msg_send![&*stop, setToolTip:
                &*NSString::from_str("Stop the tracked container and waypipe processes")];
        }
    }
}

fn session_display_command(session: &ContainerSession) -> String {
    if let Some(command) = session.command.as_deref() {
        return command.into();
    }

    match session.profile.as_deref() {
        Some("niri") => "niri".into(),
        Some("shell") => "sh".into(),
        _ => session
            .app
            .clone()
            .unwrap_or_else(|| "weston-terminal".into()),
    }
}

fn session_display_target(session: &ContainerSession) -> String {
    session
        .display
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("auto")
        .to_string()
}

fn display_slot_slug(value: &str) -> String {
    let slug = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "display".into()
    } else {
        slug
    }
}

fn resolved_session_display_target(session: &ContainerSession) -> &'static str {
    match session_display_target(session).as_str() {
        "auto" => "automatic",
        "default" => "default",
        _ => "dedicated",
    }
}

fn session_display_summary(session: &ContainerSession) -> String {
    let requested = session_display_target(session);
    match requested.as_str() {
        "auto" | "default" | "dedicated" => requested,
        _ => format!("{} -> dedicated", requested),
    }
}

fn display_occupancy_summary(index: usize, session: &ContainerSession) -> String {
    match active_session(index) {
        Some(active) => {
            let worker = active
                .display_pid
                .map(|pid| format!("; display pid {}", pid))
                .unwrap_or_default();
            format!(
                "using {} display; waypipe pid {}{}",
                active.display_slot, active.waypipe_pid, worker
            )
        }
        None => format!(
            "requested {} display; not running",
            session_display_target(session)
        ),
    }
}

fn session_waypipe_summary(session: &ContainerSession) -> String {
    let compress = session
        .waypipe_compress
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if container_sessions::is_apple_container_session(session) {
                "none"
            } else {
                "lz4"
            }
        });
    let threads = session
        .waypipe_threads
        .map(|value| value.to_string())
        .unwrap_or_else(|| "auto".into());
    format!("compress {}; threads {}", compress, threads)
}

fn short_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.into();
    }
    let mut result = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    result.push_str("...");
    result
}

fn chars_for_width(width: f64, style: TextStyle) -> usize {
    let average_char_width = match style {
        TextStyle::Hero => 15.0,
        TextStyle::Title => 12.0,
        TextStyle::Heading => 8.5,
        TextStyle::Section | TextStyle::Caption => 6.5,
        TextStyle::Body => 7.2,
        TextStyle::Mono => 7.8,
    };
    ((width.max(32.0) / average_char_width).floor() as usize).max(8)
}

fn wrapped_log_lines(logs: &[String], max_chars: usize, max_rows: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in logs {
        let mut remaining = line.as_str();
        let mut first = true;
        while !remaining.is_empty() {
            let prefix = if first { "" } else { "  " };
            let limit = max_chars.saturating_sub(prefix.chars().count()).max(16);
            let (chunk, rest) = split_at_char_count(remaining, limit);
            rows.push(format!("{}{}", prefix, chunk));
            remaining = rest.trim_start();
            first = false;
        }
    }

    if rows.len() > max_rows {
        rows[rows.len() - max_rows..].to_vec()
    } else {
        rows
    }
}

fn split_at_char_count(value: &str, max_chars: usize) -> (&str, &str) {
    if value.chars().count() <= max_chars {
        return (value, "");
    }
    let split = value
        .char_indices()
        .nth(max_chars)
        .map(|(index, _)| index)
        .unwrap_or(value.len());
    value.split_at(split)
}

fn short_image_label(image: &str, max_chars: usize) -> String {
    short_text(image, max_chars)
}

fn runtime_label(runtime: &str) -> &'static str {
    match runtime.trim().to_ascii_lowercase().as_str() {
        "docker" => "Docker",
        "orb" | "orbstack" => "OrbStack",
        _ => "Apple Container",
    }
}

fn runtime_nav(runtime: &str) -> usize {
    match runtime.trim().to_ascii_lowercase().as_str() {
        "docker" => NAV_DOCKER,
        "orb" | "orbstack" => NAV_ORBSTACK,
        _ => NAV_APPLE_CONTAINER,
    }
}

fn request_selected_runtime_container_details() {
    let selected = SELECTED_RUNTIME_CONTAINER.lock().unwrap().clone();
    let Some(selected) = selected else {
        return;
    };
    *RUNTIME_CONTAINER_DETAILS.lock().unwrap() = None;
    send(CompositorMessage::RefreshRuntimeContainerDetails {
        runtime: selected.runtime,
        name: selected.name,
    });
}

fn list_or_empty(values: &[String]) -> String {
    if values.is_empty() {
        "none".into()
    } else {
        values.join(", ")
    }
}

#[derive(Clone, Copy)]
enum TextStyle {
    Hero,
    Title,
    Heading,
    Section,
    Body,
    Caption,
    Mono,
}

fn add_label(
    parent: &NSView,
    text: &str,
    frame: NSRect,
    mtm: MainThreadMarker,
    style: TextStyle,
) -> Retained<NSTextField> {
    unsafe {
        let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        label.setFrame(frame);
        label.setSelectable(false);
        let font = match style {
            TextStyle::Hero => NSFont::boldSystemFontOfSize(28.0),
            TextStyle::Title => NSFont::boldSystemFontOfSize(22.0),
            TextStyle::Heading => NSFont::boldSystemFontOfSize(15.0),
            TextStyle::Section => NSFont::boldSystemFontOfSize(12.0),
            TextStyle::Body => NSFont::systemFontOfSize(13.0),
            TextStyle::Caption => NSFont::systemFontOfSize(11.0),
            TextStyle::Mono => NSFont::userFixedPitchFontOfSize(13.0)
                .unwrap_or_else(|| NSFont::systemFontOfSize(13.0)),
        };
        let _: () = msg_send![&*label, setFont: &*font];
        // Headings must stay on one line; treating every tall label as multiline
        // lets AppKit wrap titles into neighbouring fixed-layout rows.
        let multiline_style = matches!(
            style,
            TextStyle::Body | TextStyle::Caption | TextStyle::Mono
        );
        let multiline = text.contains('\n')
            || (multiline_style && frame.size.height >= line_height_for_style(style) * 1.6);
        let line_break_mode: isize = if multiline { 0 } else { 4 };
        let line_height = line_height_for_style(style);
        let max_lines: isize = if multiline {
            (frame.size.height / line_height).floor().max(1.0) as isize
        } else {
            1
        };
        let _: () = msg_send![&*label, setUsesSingleLineMode: !multiline];
        let _: () = msg_send![&*label, setPreferredMaxLayoutWidth: frame.size.width];
        let _: () = msg_send![&*label, setLineBreakMode: line_break_mode];
        let _: () = msg_send![&*label, setMaximumNumberOfLines: max_lines];
        let cell: Option<Retained<AnyObject>> = msg_send_id![&*label, cell];
        if let Some(cell) = cell {
            let _: () = msg_send![&*cell, setWraps: multiline];
            let _: () = msg_send![&*cell, setScrollable: false];
        }
        let visible_chars =
            chars_for_width(frame.size.width, style).saturating_mul(max_lines.max(1) as usize);
        if text.chars().count() > visible_chars {
            let _: () = msg_send![&*label, setToolTip: &*NSString::from_str(text)];
        }
        parent.addSubview(&label);
        label
    }
}

fn line_height_for_style(style: TextStyle) -> f64 {
    match style {
        TextStyle::Hero => 34.0,
        TextStyle::Title => 27.0,
        TextStyle::Heading => 20.0,
        TextStyle::Section => 17.0,
        TextStyle::Body | TextStyle::Mono => 18.0,
        TextStyle::Caption => 15.0,
    }
}

fn add_button(
    parent: &NSView,
    title: &str,
    frame: NSRect,
    handler: *mut AnyObject,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    unsafe {
        let button = NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(&*handler),
            Some(action),
            mtm,
        );
        button.setFrame(frame);
        parent.addSubview(&button);
        button
    }
}

fn add_text_field(
    parent: &NSView,
    frame: NSRect,
    placeholder: &str,
    value: &str,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    unsafe {
        let field: Retained<NSTextField> =
            msg_send_id![mtm.alloc::<NSTextField>(), initWithFrame: frame];
        let _: () = msg_send![&*field, setPlaceholderString:
            &*NSString::from_str(placeholder)];
        if !value.is_empty() {
            let _: () = msg_send![&*field, setStringValue:
                &*NSString::from_str(value)];
        }
        parent.addSubview(&field);
        field
    }
}

fn add_secure_text_field(
    parent: &NSView,
    frame: NSRect,
    placeholder: &str,
    mtm: MainThreadMarker,
) -> Retained<NSSecureTextField> {
    unsafe {
        let field: Retained<NSSecureTextField> =
            msg_send_id![mtm.alloc::<NSSecureTextField>(), initWithFrame: frame];
        let _: () = msg_send![&*field, setPlaceholderString:
            &*NSString::from_str(placeholder)];
        parent.addSubview(&field);
        field
    }
}

fn add_popup(
    parent: &NSView,
    frame: NSRect,
    items: &[&str],
    selected: usize,
    mtm: MainThreadMarker,
) -> Retained<NSPopUpButton> {
    unsafe {
        let popup: Retained<NSPopUpButton> = msg_send_id![
            mtm.alloc::<NSPopUpButton>(),
            initWithFrame: frame,
            pullsDown: false
        ];
        for item in items {
            let _: () = msg_send![&*popup, addItemWithTitle: &*NSString::from_str(item)];
        }
        let _: () = msg_send![&*popup, selectItemAtIndex: selected as isize];
        parent.addSubview(&popup);
        popup
    }
}

fn add_hit_button(
    parent: &NSView,
    frame: NSRect,
    tag: usize,
    handler: *mut AnyObject,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) {
    unsafe {
        let button = NSButton::buttonWithTitle_target_action(
            &NSString::from_str(""),
            Some(&*handler),
            Some(action),
            mtm,
        );
        button.setFrame(frame);
        button.setBordered(false);
        button.setTransparent(true);
        let _: () = msg_send![&*button, setTag: tag as isize];
        parent.addSubview(&button);
    }
}

fn add_card(parent: &NSView, frame: NSRect, mtm: MainThreadMarker) {
    unsafe {
        let card = NSBox::initWithFrame(mtm.alloc::<NSBox>(), frame);
        card.setBoxType(NSBoxType::NSBoxCustom);
        card.setTitle(&NSString::from_str(""));
        card.setTransparent(false);
        card.setCornerRadius(10.0);
        card.setBorderWidth(0.5);
        let fill = NSColor::controlBackgroundColor().colorWithAlphaComponent(0.62);
        let border = NSColor::separatorColor().colorWithAlphaComponent(0.55);
        card.setFillColor(&fill);
        card.setBorderColor(&border);
        parent.addSubview(&card);
    }
}

fn add_runtime_accent(parent: &NSView, nav: usize, frame: NSRect, mtm: MainThreadMarker) {
    unsafe {
        let accent = NSBox::initWithFrame(mtm.alloc::<NSBox>(), frame);
        accent.setBoxType(NSBoxType::NSBoxCustom);
        accent.setTitle(&NSString::from_str(""));
        accent.setTransparent(false);
        accent.setCornerRadius(frame.size.width / 2.0);
        accent.setBorderWidth(0.0);
        let color = match nav {
            NAV_APPLE_CONTAINER => NSColor::systemBlueColor(),
            NAV_DOCKER => NSColor::systemTealColor(),
            NAV_ORBSTACK => NSColor::systemOrangeColor(),
            _ => NSColor::controlAccentColor(),
        };
        accent.setFillColor(&color.colorWithAlphaComponent(0.9));
        parent.addSubview(&accent);
    }
}

fn add_separator(parent: &NSView, frame: NSRect, mtm: MainThreadMarker) {
    unsafe {
        let sep = NSBox::initWithFrame(mtm.alloc::<NSBox>(), frame);
        sep.setBoxType(NSBoxType::NSBoxSeparator);
        parent.addSubview(&sep);
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect {
        origin: NSPoint { x, y },
        size: NSSize { width, height },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_sources_build_canonical_image_references() {
        assert_eq!(
            normalize_registry_reference(0, "ubuntu:24.04"),
            "docker.io/library/ubuntu:24.04"
        );
        assert_eq!(
            normalize_registry_reference(0, "library/alpine:3.20"),
            "docker.io/library/alpine:3.20"
        );
        assert_eq!(
            normalize_registry_reference(1, "owner/desktop:latest"),
            "ghcr.io/owner/desktop:latest"
        );
        assert_eq!(
            normalize_registry_reference(2, "owner/desktop:latest"),
            "quay.io/owner/desktop:latest"
        );
    }

    #[test]
    fn explicit_registry_reference_is_not_prefixed_twice() {
        assert_eq!(
            normalize_registry_reference(0, "registry.example.com/team/gui:v1"),
            "registry.example.com/team/gui:v1"
        );
        assert_eq!(
            normalize_registry_reference(3, "localhost/gui:latest"),
            "localhost/gui:latest"
        );
    }

    #[test]
    fn generic_images_do_not_assume_a_terminal_command() {
        let defaults = session_defaults_for_image("container", "docker.io/library/ubuntu:24.04");
        assert_eq!(defaults.runtime, "container");
        assert_eq!(defaults.profile, "single-app");
        assert!(defaults.command.is_empty());

        let niri = session_defaults_for_image("container", "example/cocoa-way-niri:latest");
        assert_eq!(niri.profile, "niri");
        assert_eq!(niri.command, "niri");
    }

    #[test]
    fn clean_session_log_line_strips_ansi_sequences() {
        assert_eq!(
            clean_session_log_line(
                "\u{1b}[2m2026-06-26\u{1b}[0m \u{1b}[33mWARN\u{1b}[0m [2mniri[0m"
            ),
            "2026-06-26 WARN niri"
        );
    }

    #[test]
    fn clean_session_log_line_marks_niri_locale_warning_non_fatal() {
        let line = "\u{1b}[2m2026-06-26T12:39:42Z\u{1b}[0m \u{1b}[33mWARN\u{1b}[0m \u{1b}[2mniri::dbus\u{1b}[0m: error starting locale1 watcher: I/O error: No such file or directory (os error 2)";
        assert_eq!(
            clean_session_log_line(line),
            "niri: locale1 watcher is unavailable in this container; this is non-fatal when the desktop is running."
        );
    }

    #[test]
    fn apple_container_row_keeps_lifecycle_fields() {
        let row = "cocoa-way-niri-desktop localhost/cocoa-way-niri:latest linux arm64 running 192.168.64.66/24 4 1024 MB 2026-06-25T01:33:46Z";
        let parsed = parse_apple_container_row(row);
        assert_eq!(parsed.name.as_deref(), Some("cocoa-way-niri-desktop"));
        assert!(parsed.running);
        assert!(parsed.label.contains("localhost/cocoa-way-niri:latest"));
    }

    #[test]
    fn apple_container_row_protects_buildkit_helper() {
        let row = "buildkit builder:latest linux arm64 stopped - 2 2048 MB 2026-07-13T05:13:37Z";
        let parsed = parse_apple_container_row(row);
        assert_eq!(parsed.name, None);
        assert!(!parsed.running);
        assert!(parsed.label.contains("BuildKit helper"));
    }

    #[test]
    fn format_docker_container_row_keeps_actions_and_summary() {
        let running = format_docker_container_row("web\trunning\tUp 2 minutes\tnginx:latest");
        assert_eq!(running.name.as_deref(), Some("web"));
        assert!(running.running);
        assert!(running.label.contains("nginx:latest"));

        let stopped =
            format_docker_container_row("worker\texited\tExited (0) 1 hour ago\talpine:latest");
        assert_eq!(stopped.name.as_deref(), Some("worker"));
        assert!(!stopped.running);
    }

    #[test]
    fn wrapped_log_lines_respects_row_limit() {
        let logs = vec!["abcdefghijklmnopqrstuvwxyz".to_string()];
        let rows = wrapped_log_lines(&logs, 16, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], "abcdefghijklmnop");
        assert_eq!(rows[1], "  qrstuvwxyz");
    }

    #[test]
    fn desktop_sessions_receive_larger_apple_container_limits() {
        assert_eq!(
            default_gui_runtime_args("container", Some("niri")),
            ["--memory", "4G", "--shm-size", "1G", "--cpus", "4"]
        );
    }

    #[test]
    fn single_apps_keep_moderate_apple_container_limits() {
        assert_eq!(
            default_gui_runtime_args("container", Some("single-app")),
            ["--memory", "2G", "--shm-size", "512M", "--cpus", "4"]
        );
        assert!(default_gui_runtime_args("docker", Some("niri")).is_empty());
    }

    #[test]
    fn docker_image_inventory_preserves_reference_and_metadata() {
        let row = parse_docker_image_line("alpine:3.20\tdeadbeef\t8MB").unwrap();
        assert_eq!(row.reference.as_deref(), Some("alpine:3.20"));
        assert!(row.label.contains("deadbeef"));
        assert!(row.label.contains("8MB"));
    }

    #[test]
    fn dangling_docker_image_uses_id_for_actions() {
        let row = parse_docker_image_line("<none>:<none>\tcafebabe\t12MB").unwrap();
        assert_eq!(row.reference.as_deref(), Some("cafebabe"));
    }

    #[test]
    fn docker_volume_inventory_keeps_name_and_driver() {
        let row = parse_volume_line("project-cache\tlocal").unwrap();
        assert_eq!(row.name.as_deref(), Some("project-cache"));
        assert!(row.label.contains("local"));
    }

    #[test]
    fn docker_context_inventory_marks_current_context() {
        let row = parse_docker_context_line(
            "orbstack\ttrue\tunix:///Users/test/.orbstack/run/docker.sock\tOrbStack",
        )
        .unwrap();
        assert_eq!(row.name.as_deref(), Some("orbstack"));
        assert!(row.current);
        assert!(row.label.starts_with("* orbstack"));
    }

    #[test]
    fn apple_container_versions_are_extracted_from_cli_and_api_text() {
        assert_eq!(
            extract_version("container CLI version 1.1.0 (build: release)"),
            Some("1.1.0".into())
        );
        assert_eq!(
            extract_version("container-apiserver version 1.0.0 (build: release)"),
            Some("1.0.0".into())
        );
    }

    #[test]
    fn apple_container_version_comparison_handles_minor_updates() {
        assert!(version_at_least("1.1.0", (1, 1, 0)));
        assert!(version_at_least("2.0.0", (1, 1, 0)));
        assert!(!version_at_least("1.0.9", (1, 1, 0)));
        assert!(!version_at_least("unknown", (1, 0, 0)));
    }
}
