pub enum CompositorMessage {
    Maximize(bool),
    Fullscreen(bool),
    ToggleHiDpi,
    Connect(usize),
}