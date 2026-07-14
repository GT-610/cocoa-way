use wayland_client::{
    globals::GlobalListContents, protocol::wl_registry, Connection, Dispatch, QueueHandle,
};
struct App;
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            println!(
                "[Global] Interface: {}, Version: {}, Name: {}",
                interface, version, name
            );
        }
    }
}
fn main() {
    println!("Attempting to connect to Wayland server...");
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to Wayland server: {:?}", e);
            eprintln!("Ensure 'cocoa-way' is running and WAYLAND_DISPLAY is set.");
            return;
        }
    };
    println!("Connected! Initializing registry...");
    let (_globals, mut event_queue) =
        wayland_client::globals::registry_queue_init::<App>(&conn).unwrap();
    let mut app = App;
    println!("Roundtripping to fetch globals...");
    event_queue.roundtrip(&mut app).unwrap();
    println!("Success! Client connected and enumerated globals.");
}
