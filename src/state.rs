use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm,
    input::{pointer::CursorImageStatus, Seat, SeatHandler, SeatState},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        selection::data_device::{DataDeviceHandler, WaylandDndGrabHandler},
        selection::SelectionHandler,
        shm::{ShmHandler, ShmState},
    },
};
use smithay::wayland::shell::xdg::{XdgShellHandler, XdgShellState};
use smithay::wayland::shell::xdg::decoration::{XdgDecorationState, XdgDecorationHandler};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use crate::layout::Layout;
pub struct AppState {
    display_handle: DisplayHandle,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<AppState>,
    pub seat: Seat<Self>,
    pub data_device_state: smithay::wayland::selection::data_device::DataDeviceState,
    pub data_control_state: smithay::wayland::selection::wlr_data_control::DataControlState,
    _xdg_decoration_state: XdgDecorationState,
    _viewporter_state: smithay::wayland::viewporter::ViewporterState,
    _fractional_scale_state: smithay::wayland::fractional_scale::FractionalScaleManagerState,
    _pointer_constraints_state: smithay::wayland::pointer_constraints::PointerConstraintsState,
    _relative_pointer_state: smithay::wayland::relative_pointer::RelativePointerManagerState,
    _output_state: smithay::wayland::output::OutputManagerState,
    pub output: smithay::output::Output,
    pub toplevels: Vec<smithay::wayland::shell::xdg::ToplevelSurface>,
    pub popups: Vec<smithay::wayland::shell::xdg::PopupSurface>,
    pub layout: Layout,
    pub surface_positions: std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        (i32, i32),
    >,
    pub drag_state: Option<(
        smithay::reexports::wayland_server::backend::ObjectId,
        (f64, f64),
    )>,
    pub start_drag_request: Option<smithay::reexports::wayland_server::backend::ObjectId>,
    pub loop_signal: std::sync::mpsc::Sender<crate::messages::CompositorMessage>,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    /// Monotonic start time — used to compute frame timestamps for wl_callback::done.
    pub start_time: std::time::Instant,
    /// Frame callbacks collected during commit(); fired after swap_buffers().
    pub pending_frame_callbacks:
        Vec<smithay::reexports::wayland_server::protocol::wl_callback::WlCallback>,
    /// Set by Wayland commits or layout changes so the winit loop can avoid
    /// continuous redraws when the scene is idle.
    pub needs_redraw: bool,
    /// Total Wayland surface commits observed since startup. Used for lightweight
    /// performance diagnostics in Container Mode.
    pub commit_counter: u64,
    host_clipboard_text: Option<String>,
    pending_guest_clipboard_mime: Option<String>,
    pasteboard_change_count: isize,
    last_pasteboard_poll: std::time::Instant,
}
impl AppState {
    pub fn new(
        display_handle: &DisplayHandle,
        scale_factor: f64,
        loop_signal: std::sync::mpsc::Sender<crate::messages::CompositorMessage>,
        width: u32,
        height: u32,
    ) -> Self {
        let compositor_state = CompositorState::new::<Self>(display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(display_handle);
        let shm_state = ShmState::new::<Self>(
            display_handle,
            vec![
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Argb8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Xrgb8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Abgr8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Xbgr8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Rgba8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Rgbx8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Bgra8888,
                smithay::reexports::wayland_server::protocol::wl_shm::Format::Bgrx8888,
            ],
        );
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(display_handle, "winit-seat");
        let xkb_config = smithay::input::keyboard::XkbConfig {
            rules: "evdev",
            model: "pc105",
            layout: "us",
            variant: "",
            options: None,
        };
        seat.add_keyboard(xkb_config, 600, 50).unwrap();
        seat.add_pointer();
        let output_state = smithay::wayland::output::OutputManagerState::new_with_xdg_output::<Self>(
            display_handle,
        );
        let output = smithay::output::Output::new(
            "winit".to_string(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "Smithay".into(),
                model: "Winit".into(),
                serial_number: "0000".into(),
            },
        );
        let _global = output.create_global::<Self>(display_handle);
        let mode = smithay::output::Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        let scale_int = scale_factor.round() as i32;
        output.change_current_state(
            Some(mode),
            Some(smithay::utils::Transform::Normal),
            Some(smithay::output::Scale::Integer(scale_int)),
            Some((0, 0).into()),
        );
        output.set_preferred(mode);
        Self {
            display_handle: display_handle.clone(),
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            seat,
            data_device_state: smithay::wayland::selection::data_device::DataDeviceState::new::<Self>(
                display_handle,
            ),
            data_control_state:
                smithay::wayland::selection::wlr_data_control::DataControlState::new::<Self, _>(
                    display_handle,
                    None,
                    |_| true,
                ),
            _xdg_decoration_state: XdgDecorationState::new::<Self>(display_handle),
            _viewporter_state: smithay::wayland::viewporter::ViewporterState::new::<Self>(
                display_handle,
            ),
            _fractional_scale_state:
                smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(
                    display_handle,
                ),
            _pointer_constraints_state:
                smithay::wayland::pointer_constraints::PointerConstraintsState::new::<Self>(
                    display_handle,
                ),
            _relative_pointer_state:
                smithay::wayland::relative_pointer::RelativePointerManagerState::new::<Self>(
                    display_handle,
                ),
            _output_state: output_state,
            output,
            toplevels: Vec::new(),
            popups: Vec::new(),
            layout: {
                let (logical_width, logical_height) =
                    crate::layout::logical_size_from_physical(width, height, scale_factor);
                Layout::new(logical_width, logical_height)
            },
            surface_positions: std::collections::HashMap::new(),
            drag_state: None,
            start_drag_request: None,
            loop_signal,
            width,
            height,
            scale_factor,
            start_time: std::time::Instant::now(),
            pending_frame_callbacks: Vec::new(),
            needs_redraw: true,
            commit_counter: 0,
            host_clipboard_text: None,
            pending_guest_clipboard_mime: None,
            pasteboard_change_count: -1,
            last_pasteboard_poll: std::time::Instant::now() - std::time::Duration::from_millis(100),
        }
    }

    pub fn poll_host_clipboard(&mut self) {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_pasteboard_poll) < std::time::Duration::from_millis(100) {
            return;
        }
        self.last_pasteboard_poll = now;

        let (change_count, text) = pasteboard_snapshot();
        if change_count == self.pasteboard_change_count {
            return;
        }
        self.pasteboard_change_count = change_count;
        let Some(text) = text else {
            self.host_clipboard_text = None;
            crate::diagnostics::record_clipboard_host_change(0);
            return;
        };
        if self.host_clipboard_text.as_deref() == Some(text.as_str()) {
            return;
        }

        self.host_clipboard_text = Some(text);
        crate::diagnostics::record_clipboard_host_change(
            self.host_clipboard_text.as_ref().map_or(0, String::len),
        );
        log::info!("Clipboard: publishing changed macOS text to Wayland clients");
        smithay::wayland::selection::data_device::set_data_device_selection::<Self>(
            &self.display_handle,
            &self.seat,
            clipboard_text_mime_types(),
            (),
        );
    }

    pub fn install_guest_clipboard(&mut self, text: String) {
        if self.host_clipboard_text.as_deref() == Some(text.as_str()) {
            return;
        }
        self.pasteboard_change_count = write_to_pasteboard(&text);
        crate::diagnostics::record_clipboard_guest_install(text.len());
        self.host_clipboard_text = Some(text);
        log::info!("Clipboard: installed Wayland text on the macOS pasteboard");
        smithay::wayland::selection::data_device::set_data_device_selection::<Self>(
            &self.display_handle,
            &self.seat,
            clipboard_text_mime_types(),
            (),
        );
    }

    /// Read a client selection only after Smithay has committed it to the seat.
    /// `SelectionHandler::new_selection` runs before that protocol state update.
    pub fn request_pending_guest_clipboard(&mut self) {
        let Some(mime) = self.pending_guest_clipboard_mime.take() else {
            return;
        };
        let (read_fd, write_fd) = match nix_pipe() {
            Some(pair) => pair,
            None => {
                crate::diagnostics::record_clipboard_failure(
                    "Unable to create a pipe for the Wayland clipboard transfer",
                );
                return;
            }
        };
        if let Err(error) =
            smithay::wayland::selection::data_device::request_data_device_client_selection::<AppState>(
                &self.seat,
                mime.clone(),
                write_fd,
            )
        {
            log::warn!("Failed to request Wayland clipboard contents as {mime}: {error}");
            crate::diagnostics::record_clipboard_failure(format!(
                "Failed to request Wayland clipboard contents as {mime}: {error}"
            ));
            return;
        }

        let loop_signal = self.loop_signal.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut file = std::fs::File::from(read_fd);
            let mut text = String::new();
            match file.read_to_string(&mut text) {
                Ok(_) if !text.is_empty() => {
                    let _ = loop_signal
                        .send(crate::messages::CompositorMessage::GuestClipboardText(text));
                }
                Ok(_) => crate::diagnostics::record_clipboard_failure(
                    "Wayland clipboard offer returned no text",
                ),
                Err(error) => crate::diagnostics::record_clipboard_failure(format!(
                    "Failed to read the Wayland clipboard offer: {error}"
                )),
            }
        });
    }
    pub fn update_scale_factor(&mut self, scale: f64) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            log::warn!("Ignoring invalid window scale factor: {}", scale);
            1.0
        };
        self.scale_factor = scale;
        self.output.change_current_state(
            None,
            None,
            Some(smithay::output::Scale::Integer(
                (scale.round() as i32).clamp(1, 8),
            )),
            None,
        );
        let (logical_width, logical_height) =
            crate::layout::logical_size_from_physical(self.width, self.height, scale);
        self.layout.set_view_size(logical_width, logical_height);
        for tile in &self.layout.tiles {
            tile.request_size();
        }
    }
}
impl smithay::wayland::output::OutputHandler for AppState {}
smithay::delegate_output!(AppState);
delegate_compositor!(AppState);
delegate_shm!(AppState);
delegate_seat!(AppState);
smithay::delegate_xdg_shell!(AppState);
impl CompositorHandler for AppState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        let client_data = client
            .get_data::<ClientState>()
            .expect("Client data missing");
        &client_data.compositor_state
    }
    fn new_surface(&mut self, _surface: &WlSurface) {
        // No-op: pre-commit hook logging removed to avoid 60fps log spam
    }
    fn commit(&mut self, surface: &WlSurface) {
        use smithay::wayland::compositor::{
            SurfaceAttributes, TraversalAction, with_surface_tree_downward,
        };
        let mut new_cbs = Vec::new();
        with_surface_tree_downward(
            surface,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |_surf, states, _| {
                let mut guard = states.cached_state.get::<SurfaceAttributes>();
                new_cbs.extend(guard.current().frame_callbacks.drain(..));
            },
            |_, _, _| true,
        );
        self.pending_frame_callbacks.extend(new_cbs);
        self.commit_counter = self.commit_counter.saturating_add(1);
        self.needs_redraw = true;
    }

    fn destroyed(&mut self, surface: &WlSurface) {
        let surface_id = surface.id();
        self.layout.remove_tile(&surface_id);
        self.toplevels
            .retain(|toplevel| toplevel.wl_surface() != surface);
        self.popups.retain(|popup| popup.wl_surface() != surface);
        self.surface_positions.remove(&surface_id);
        if self
            .drag_state
            .as_ref()
            .is_some_and(|(id, _)| id == &surface_id)
        {
            self.drag_state = None;
        }
        if self.start_drag_request.as_ref() == Some(&surface_id) {
            self.start_drag_request = None;
        }

        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard.current_focus().as_ref() == Some(surface) {
                keyboard.set_focus(self, None, smithay::utils::SERIAL_COUNTER.next_serial());
            }
        }
        self.needs_redraw = true;
        log::info!("Removed destroyed surface {surface_id:?} from the compositor layout");
    }
}
impl XdgShellHandler for AppState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }
    fn new_toplevel(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        log::info!("New XDG Toplevel Created: {:?}", surface.wl_surface().id());
        if !self.toplevels.contains(&surface) {
            self.toplevels.push(surface.clone());
            self.layout.add_tile(surface.clone());
            self.needs_redraw = true;
        }
        // Tell the client the compositor window size and scale so it renders at
        // the correct HiDPI resolution.
        let (logical_w, logical_h) =
            crate::layout::logical_size_from_physical(self.width, self.height, self.scale_factor);
        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
            state.size = Some((logical_w, logical_h).into());
        });
        // Notify the client of the compositor's fractional scale so it can
        // render at the correct resolution without needing integer rounding.
        smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
            smithay::wayland::fractional_scale::with_fractional_scale(states, |fs| {
                fs.set_preferred_scale(self.scale_factor);
            });
        });
        surface.send_configure();
    }
    fn new_popup(
        &mut self,
        surface: smithay::wayland::shell::xdg::PopupSurface,
        positioner: smithay::wayland::shell::xdg::PositionerState,
    ) {
        let geo = positioner.get_geometry();
        surface.with_pending_state(|state| {
            state.geometry = geo;
        });
        if surface.send_configure().is_err() {
            return;
        }
        self.popups.push(surface);
        self.needs_redraw = true;
    }
    fn grab(
        &mut self,
        _surface: smithay::wayland::shell::xdg::PopupSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
    }
    fn reposition_request(
        &mut self,
        _surface: smithay::wayland::shell::xdg::PopupSurface,
        _positioner: smithay::wayland::shell::xdg::PositionerState,
        _token: u32,
    ) {
    }
    fn maximize_request(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        println!("*** HIT MAXIMIZE REQUEST ***");
        log::info!("Maximize Request: {:?}", surface.wl_surface().id());
        log::info!(
            "DEBUG MAXIMIZE: self.width={}, self.height={}, self.scale_factor={}",
            self.width,
            self.height,
            self.scale_factor
        );
        let (logical_w, logical_h) =
            crate::layout::logical_size_from_physical(self.width, self.height, self.scale_factor);
        log::info!(
            "Maximizing to Logical Size: {}x{} (Physical: {}x{}, Scale: {})",
            logical_w,
            logical_h,
            self.width,
            self.height,
            self.scale_factor
        );
        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
            state.size = Some((logical_w, logical_h).into());
        });
        surface.send_configure();
        let _ = self
            .loop_signal
            .send(crate::messages::CompositorMessage::Maximize(true));
    }
    fn unmaximize_request(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        log::info!("Unmaximize Request: {:?}", surface.wl_surface().id());
        surface.with_pending_state(|state| {
             state.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
         });
        surface.send_configure();
        let _ = self
            .loop_signal
            .send(crate::messages::CompositorMessage::Maximize(false));
    }
    fn fullscreen_request(
        &mut self,
        surface: smithay::wayland::shell::xdg::ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        log::info!("Fullscreen Request: {:?}", surface.wl_surface().id());
        let (logical_w, logical_h) =
            crate::layout::logical_size_from_physical(self.width, self.height, self.scale_factor);
        log::info!("Fullscreening to Logical Size: {}x{}", logical_w, logical_h);
        surface.with_pending_state(|state| {
             state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
             state.size = Some((logical_w, logical_h).into());
         });
        surface.send_configure();
        let _ = self
            .loop_signal
            .send(crate::messages::CompositorMessage::Fullscreen(true));
    }
    fn unfullscreen_request(&mut self, surface: smithay::wayland::shell::xdg::ToplevelSurface) {
        log::info!("Unfullscreen Request: {:?}", surface.wl_surface().id());
        surface.with_pending_state(|state| {
             state.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();
        let _ = self
            .loop_signal
            .send(crate::messages::CompositorMessage::Fullscreen(false));
    }
    fn move_request(
        &mut self,
        surface: smithay::wayland::shell::xdg::ToplevelSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
        log::info!(
            "XDG Move Request received for surface {:?}",
            surface.wl_surface().id()
        );
        let id = surface.wl_surface().id();
        self.start_drag_request = Some(id);
    }
}
impl ShmHandler for AppState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
impl BufferHandler for AppState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}
impl SeatHandler for AppState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;
    fn seat_state(&mut self) -> &mut SeatState<AppState> {
        &mut self.seat_state
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        use objc2_app_kit::NSCursor;
        use smithay::input::pointer::CursorIcon;
        unsafe {
            match image {
                CursorImageStatus::Hidden => NSCursor::hide(),
                CursorImageStatus::Named(icon) => {
                    let cursor = match icon {
                        CursorIcon::Text | CursorIcon::VerticalText => NSCursor::IBeamCursor(),
                        CursorIcon::Pointer => NSCursor::pointingHandCursor(),
                        CursorIcon::Move | CursorIcon::AllScroll => NSCursor::openHandCursor(),
                        CursorIcon::Grab => NSCursor::openHandCursor(),
                        CursorIcon::Grabbing => NSCursor::closedHandCursor(),
                        CursorIcon::Crosshair => NSCursor::crosshairCursor(),
                        CursorIcon::NotAllowed | CursorIcon::NoDrop => {
                            NSCursor::operationNotAllowedCursor()
                        }
                        CursorIcon::EResize
                        | CursorIcon::WResize
                        | CursorIcon::EwResize
                        | CursorIcon::ColResize => NSCursor::resizeLeftRightCursor(),
                        CursorIcon::NResize
                        | CursorIcon::SResize
                        | CursorIcon::NsResize
                        | CursorIcon::RowResize => NSCursor::resizeUpDownCursor(),
                        CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize => {
                            NSCursor::resizeLeftRightCursor()
                        }
                        CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize => {
                            NSCursor::resizeLeftRightCursor()
                        }
                        CursorIcon::Copy => NSCursor::dragCopyCursor(),
                        CursorIcon::Alias => NSCursor::dragLinkCursor(),
                        CursorIcon::ContextMenu => NSCursor::contextualMenuCursor(),
                        CursorIcon::ZoomIn | CursorIcon::ZoomOut => NSCursor::crosshairCursor(),
                        _ => NSCursor::arrowCursor(),
                    };
                    cursor.set();
                }
                CursorImageStatus::Surface(_) => {
                    // Custom surface cursor — use arrow fallback for now
                    NSCursor::arrowCursor().set();
                }
            }
        }
    }
    fn focus_changed(&mut self, seat: &Seat<Self>, focus: Option<&Self::KeyboardFocus>) {
        let client = focus.and_then(Resource::client);
        smithay::wayland::selection::data_device::set_data_device_focus::<Self>(
            &self.display_handle,
            seat,
            client,
        );
    }
}
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}
impl smithay::reexports::wayland_server::backend::ClientData for ClientState {
    fn initialized(&self, _client_id: smithay::reexports::wayland_server::backend::ClientId) {}
    fn disconnected(
        &self,
        _client_id: smithay::reexports::wayland_server::backend::ClientId,
        _reason: smithay::reexports::wayland_server::backend::DisconnectReason,
    ) {
    }
}
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::{SelectionSource, SelectionTarget};
impl SelectionHandler for AppState {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: smithay::input::Seat<Self>,
    ) {
        if ty != SelectionTarget::Clipboard {
            return;
        }
        let source = match source {
            Some(s) => s,
            None => {
                self.pending_guest_clipboard_mime = None;
                return;
            }
        };
        let mime_types = source.mime_types();
        let Some(mime) = preferred_clipboard_text_mime(&mime_types) else {
            self.pending_guest_clipboard_mime = None;
            return;
        };
        log::info!("Clipboard: Wayland client published text as {mime}");
        crate::diagnostics::record_clipboard_guest_offer(&mime);
        self.pending_guest_clipboard_mime = Some(mime);
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: std::os::unix::io::OwnedFd,
        _seat: smithay::input::Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
        if ty != SelectionTarget::Clipboard {
            return;
        }
        if !is_clipboard_text_mime(&mime_type) {
            return;
        }
        log::info!("Clipboard: Wayland client requested macOS text as {mime_type}");
        let text = self.host_clipboard_text.clone();
        std::thread::spawn(move || {
            use std::io::Write;
            if let Some(text) = text {
                let mut f = std::fs::File::from(fd);
                let _ = f.write_all(text.as_bytes());
            }
        });
    }
}
impl DataDeviceHandler for AppState {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}
impl WaylandDndGrabHandler for AppState {}
delegate_data_device!(AppState);
impl smithay::wayland::selection::wlr_data_control::DataControlHandler for AppState {
    fn data_control_state(
        &mut self,
    ) -> &mut smithay::wayland::selection::wlr_data_control::DataControlState {
        &mut self.data_control_state
    }
}
smithay::delegate_data_control!(AppState);
use smithay::delegate_xdg_decoration;
use smithay::wayland::shell::xdg::ToplevelSurface;
impl XdgDecorationHandler for AppState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        toplevel.send_configure();
        log::info!("New decoration requested - using server-side");
    }
    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        toplevel.send_configure();
        log::info!("Decoration mode requested: {:?}", mode);
    }
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        toplevel.send_configure();
        log::info!("Decoration mode unset - defaulting to server-side");
    }
}
delegate_xdg_decoration!(AppState);
smithay::delegate_viewporter!(AppState);
impl smithay::wayland::fractional_scale::FractionalScaleHandler for AppState {
    fn new_fractional_scale(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        smithay::wayland::compositor::with_states(&surface, |states| {
            smithay::wayland::fractional_scale::with_fractional_scale(states, |fs| {
                fs.set_preferred_scale(self.scale_factor);
            });
        });
    }
}
smithay::delegate_fractional_scale!(AppState);
impl smithay::wayland::pointer_constraints::PointerConstraintsHandler for AppState {
    fn new_constraint(
        &mut self,
        _surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        _pointer: &smithay::input::pointer::PointerHandle<Self>,
    ) {
    }
    fn cursor_position_hint(
        &mut self,
        _surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        _pointer: &smithay::input::pointer::PointerHandle<Self>,
        _location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
    }
}
smithay::delegate_pointer_constraints!(AppState);
smithay::delegate_relative_pointer!(AppState);

fn nix_pipe() -> Option<(std::os::unix::io::OwnedFd, std::os::unix::io::OwnedFd)> {
    use std::os::unix::io::FromRawFd;
    let mut fds = [0i32; 2];
    let ret = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if ret != 0 {
        return None;
    }
    let read = unsafe { std::os::unix::io::OwnedFd::from_raw_fd(fds[0]) };
    let write = unsafe { std::os::unix::io::OwnedFd::from_raw_fd(fds[1]) };
    Some((read, write))
}

fn clipboard_text_mime_types() -> Vec<String> {
    [
        "text/plain;charset=utf-8",
        "text/plain",
        "UTF8_STRING",
        "TEXT",
        "STRING",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn is_clipboard_text_mime(mime: &str) -> bool {
    let normalized = mime.trim().to_ascii_lowercase();
    normalized == "utf8_string"
        || normalized == "text"
        || normalized == "string"
        || normalized == "text/plain"
        || normalized.starts_with("text/plain;")
}

fn preferred_clipboard_text_mime(mime_types: &[String]) -> Option<String> {
    const PRIORITIES: [&str; 5] = [
        "text/plain;charset=utf-8",
        "text/plain",
        "utf8_string",
        "text",
        "string",
    ];
    PRIORITIES
        .iter()
        .find_map(|preferred| {
            mime_types
                .iter()
                .find(|mime| mime.trim().eq_ignore_ascii_case(preferred))
                .cloned()
        })
        .or_else(|| {
            mime_types
                .iter()
                .find(|mime| is_clipboard_text_mime(mime))
                .cloned()
        })
}

#[cfg(test)]
mod clipboard_tests {
    use super::{is_clipboard_text_mime, preferred_clipboard_text_mime};

    #[test]
    fn chooses_the_exact_mime_offered_by_the_client() {
        let offered = vec![
            "image/png".to_string(),
            "text/plain;charset=UTF-8".to_string(),
            "UTF8_STRING".to_string(),
        ];
        assert_eq!(
            preferred_clipboard_text_mime(&offered).as_deref(),
            Some("text/plain;charset=UTF-8")
        );
    }

    #[test]
    fn accepts_wayland_and_xwayland_text_aliases() {
        for mime in [
            "text/plain",
            "text/plain;charset=utf-8",
            "UTF8_STRING",
            "TEXT",
            "STRING",
        ] {
            assert!(is_clipboard_text_mime(mime), "rejected {mime}");
        }
        assert!(!is_clipboard_text_mime("image/png"));
    }
}

fn write_to_pasteboard(text: &str) -> isize {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let ns_str = NSString::from_str(text);
        let pb_type = objc2_app_kit::NSPasteboardTypeString;
        pb.setString_forType(&ns_str, pb_type);
        pb.changeCount()
    }
}

fn pasteboard_snapshot() -> (isize, Option<String>) {
    use objc2_app_kit::NSPasteboard;
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        let pb_type = objc2_app_kit::NSPasteboardTypeString;
        (
            pb.changeCount(),
            pb.stringForType(pb_type).map(|s| s.to_string()),
        )
    }
}
