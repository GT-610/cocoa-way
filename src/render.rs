use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::wayland::shm::with_buffer_contents;

/// Extract pixels from a Wayland SHM buffer into a tightly-packed BGRA Vec.
///
/// Alpha is ALWAYS forced to 0xFF — we render tiles as fully opaque.
/// For XRGB8888 the X byte is undefined (often 0); for ARGB8888 many
/// compositors emit 0 for areas they haven't drawn yet.  Either way,
/// forcing opaque prevents the Metal alpha-blending from showing the
/// compositor background through the tile.
pub fn get_buffer_pixels(buffer: &WlBuffer) -> Option<(i32, i32, Vec<u8>)> {
    with_buffer_contents(buffer, |ptr, len, data| {
        let width  = data.width;
        let height = data.height;
        let stride = data.stride;
        if width == 0 || height == 0 {
            return None;
        }
        log::debug!("get_buffer_pixels: {:?}  {}x{}  stride={}", data.format, width, height, stride);
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        let mut pixels = vec![0u8; (width * height * 4) as usize];

        if stride == width * 4 {
            // Fast path: no row padding — single bulk copy then alpha fixup.
            let src_len = pixels.len().min(slice.len());
            pixels[..src_len].copy_from_slice(&slice[..src_len]);
            // Set every alpha byte (offset 3, 7, 11 …) to 0xFF.
            let mut i = 3;
            while i < pixels.len() {
                pixels[i] = 0xFF;
                i += 4;
            }
        } else {
            // Slow path: rows have extra padding bytes.
            for y in 0..height {
                let src_base = (y * stride) as usize;
                let dst_base = (y * width * 4) as usize;
                for x in 0..width as usize {
                    let s = src_base + x * 4;
                    let d = dst_base + x * 4;
                    if s + 4 <= slice.len() {
                        pixels[d]     = slice[s];
                        pixels[d + 1] = slice[s + 1];
                        pixels[d + 2] = slice[s + 2];
                        pixels[d + 3] = 0xFF;
                    }
                }
            }
        }
        Some((width, height, pixels))
    })
    .ok()
    .flatten()
}

