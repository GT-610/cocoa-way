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
        shm::{BufferData, ShmHandler, ShmState},
    },
};
use smithay::wayland::shell::xdg::{XdgShellHandler, XdgShellState};
use smithay::wayland::shell::xdg::decoration::{XdgDecorationState, XdgDecorationHandler};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use crate::layout::Layout;
pub struct AppState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<AppState>,
    pub seat: Seat<Self>,
    pub data_device_state: smithay::wayland::selection::data_device::DataDeviceState,
    pub xdg_decoration_state: XdgDecorationState,
    pub viewporter_state: smithay::wayland::viewporter::ViewporterState,
    pub fractional_scale_state: smithay::wayland::fractional_scale::FractionalScaleManagerState,
    pub pointer_constraints_state: smithay::wayland::pointer_constraints::PointerConstraintsState,
    pub relative_pointer_state: smithay::wayland::relative_pointer::RelativePointerManagerState,
    pub output_state: smithay::wayland::output::OutputManagerState,
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
    pub pending_frame_callbacks: Vec<smithay::reexports::wayland_server::protocol::wl_callback::WlCallback>,
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
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            seat,
            data_device_state: smithay::wayland::selection::data_device::DataDeviceState::new::<Self>(display_handle),
            xdg_decoration_state: XdgDecorationState::new::<Self>(display_handle),
            viewporter_state: smithay::wayland::viewporter::ViewporterState::new::<Self>(display_handle),
            fractional_scale_state: smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(display_handle),
            pointer_constraints_state: smithay::wayland::pointer_constraints::PointerConstraintsState::new::<Self>(display_handle),
            relative_pointer_state: smithay::wayland::relative_pointer::RelativePointerManagerState::new::<Self>(display_handle),
            output_state,
            output,
            toplevels: Vec::new(),
            popups: Vec::new(),
            layout: Layout::new((width as f64 / scale_factor) as i32, (height as f64 / scale_factor) as i32),
            surface_positions: std::collections::HashMap::new(),
            drag_state: None,
            start_drag_request: None,
            loop_signal,
            width,
            height,
            scale_factor,
            start_time: std::time::Instant::now(),
            pending_frame_callbacks: Vec::new(),
        }
    }
    pub fn update_scale_factor(&mut self, scale: f64) {
        self.output.change_current_state(
            None, None,
            Some(smithay::output::Scale::Integer(scale.round() as i32)),
            None,
        );
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
        use smithay::wayland::compositor::{SurfaceAttributes, with_surface_tree_downward, TraversalAction};
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
        }
        let scale_int = self.scale_factor.round() as i32;
        // Tell the client the compositor window size and scale so it renders at
        // the correct HiDPI resolution.
        let logical_w = (self.width as f64 / self.scale_factor).round() as i32;
        let logical_h = (self.height as f64 / self.scale_factor).round() as i32;
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
        let logical_w = (self.width as f64 / self.scale_factor) as i32;
        let logical_h = (self.height as f64 / self.scale_factor) as i32;
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
        let logical_w = (self.width as f64 / self.scale_factor) as i32;
        let logical_h = (self.height as f64 / self.scale_factor) as i32;
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
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {
    }
}
impl SeatHandler for AppState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;
    fn seat_state(&mut self) -> &mut SeatState<AppState> {
        &mut self.seat_state
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        use smithay::input::pointer::CursorIcon;
        use objc2_app_kit::NSCursor;
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
                        CursorIcon::NotAllowed | CursorIcon::NoDrop => NSCursor::operationNotAllowedCursor(),
                        CursorIcon::EResize | CursorIcon::WResize | CursorIcon::EwResize | CursorIcon::ColResize => NSCursor::resizeLeftRightCursor(),
                        CursorIcon::NResize | CursorIcon::SResize | CursorIcon::NsResize | CursorIcon::RowResize => NSCursor::resizeUpDownCursor(),
                        CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize => NSCursor::resizeLeftRightCursor(),
                        CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize => NSCursor::resizeLeftRightCursor(),
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
    fn focus_changed(&mut self, _seat: &Seat<Self>, _focus: Option<&Self::KeyboardFocus>) {}
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
use smithay::wayland::selection::{SelectionTarget, SelectionSource};
impl SelectionHandler for AppState {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        seat: smithay::input::Seat<Self>,
    ) {
        if ty != SelectionTarget::Clipboard {
            return;
        }
        let source = match source {
            Some(s) => s,
            None => return,
        };
        let mime = if source.mime_types().iter().any(|m| m == "text/plain;charset=utf-8") {
            "text/plain;charset=utf-8".to_string()
        } else if source.mime_types().iter().any(|m| m == "text/plain") {
            "text/plain".to_string()
        } else {
            return;
        };
        let (read_fd, write_fd) = match nix_pipe() {
            Some(pair) => pair,
            None => return,
        };
        // Ask the client to write its selection data to the write end of the pipe.
        let _ = smithay::wayland::selection::data_device::request_data_device_client_selection::<AppState>(
            &seat, mime, write_fd,
        );
        std::thread::spawn(move || {
            use std::io::Read;
            use std::os::unix::io::{FromRawFd, IntoRawFd};
            let mut f = unsafe { std::fs::File::from_raw_fd(read_fd.into_raw_fd()) };
            let mut buf = String::new();
            if f.read_to_string(&mut buf).is_ok() && !buf.is_empty() {
                write_to_pasteboard(&buf);
            }
        });
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
        if !mime_type.starts_with("text/plain") {
            return;
        }
        std::thread::spawn(move || {
            use std::io::Write;
            use std::os::unix::io::{FromRawFd, IntoRawFd};
            if let Some(text) = read_from_pasteboard() {
                let mut f = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
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
    fn new_fractional_scale(&mut self, surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) {
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
    ) {}
    fn cursor_position_hint(
        &mut self,
        _surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        _pointer: &smithay::input::pointer::PointerHandle<Self>,
        _location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {}
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

fn write_to_pasteboard(text: &str) {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let ns_str = NSString::from_str(text);
        let pb_type = objc2_app_kit::NSPasteboardTypeString;
        pb.setString_forType(&ns_str, pb_type);
    }
}

fn read_from_pasteboard() -> Option<String> {
    use objc2_app_kit::NSPasteboard;
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        let pb_type = objc2_app_kit::NSPasteboardTypeString;
        pb.stringForType(pb_type).map(|s| s.to_string())
    }
}