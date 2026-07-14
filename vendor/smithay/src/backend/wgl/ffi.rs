//! WGL FFI bindings and OpenGL function loading
//!
//! Uses libloading to load opengl32.dll and WGL functions.

use std::ffi::{c_void, CString};
use std::sync::{Mutex, OnceLock};

use libloading::Library;

/// OpenGL library handle
static GL_LIBRARY: OnceLock<Library> = OnceLock::new();

/// WGL function pointers
struct WglFunctions {
    wgl_get_proc_address: unsafe extern "system" fn(*const i8) -> *const c_void,
    wgl_create_context: unsafe extern "system" fn(isize) -> isize,
    wgl_delete_context: unsafe extern "system" fn(isize) -> i32,
    wgl_make_current: unsafe extern "system" fn(isize, isize) -> i32,
    wgl_get_current_context: unsafe extern "system" fn() -> isize,
    wgl_get_current_dc: unsafe extern "system" fn() -> isize,
}

static WGL_FUNCTIONS: OnceLock<WglFunctions> = OnceLock::new();
static INIT_LOCK: Mutex<()> = Mutex::new(());

/// Initialize the OpenGL library
pub fn init_gl_library() -> Result<(), super::Error> {
    // Use a lock to ensure thread-safe initialization
    let _guard = INIT_LOCK.lock().unwrap();

    if GL_LIBRARY.get().is_some() && WGL_FUNCTIONS.get().is_some() {
        return Ok(());
    }

    let lib = GL_LIBRARY
        .get_or_init(|| unsafe { Library::new("opengl32.dll") }.expect("Failed to load opengl32.dll"));

    WGL_FUNCTIONS.get_or_init(|| unsafe {
        WglFunctions {
            wgl_get_proc_address: *lib
                .get(b"wglGetProcAddress\0")
                .expect("Failed to load wglGetProcAddress"),
            wgl_create_context: *lib
                .get(b"wglCreateContext\0")
                .expect("Failed to load wglCreateContext"),
            wgl_delete_context: *lib
                .get(b"wglDeleteContext\0")
                .expect("Failed to load wglDeleteContext"),
            wgl_make_current: *lib
                .get(b"wglMakeCurrent\0")
                .expect("Failed to load wglMakeCurrent"),
            wgl_get_current_context: *lib
                .get(b"wglGetCurrentContext\0")
                .expect("Failed to load wglGetCurrentContext"),
            wgl_get_current_dc: *lib
                .get(b"wglGetCurrentDC\0")
                .expect("Failed to load wglGetCurrentDC"),
        }
    });

    Ok(())
}

/// Get the address of an OpenGL function by name
///
/// This is the main entry point for loading GL functions on Windows.
/// It first tries wglGetProcAddress (for extensions), then falls back
/// to GetProcAddress from opengl32.dll (for core functions).
pub fn get_proc_address(name: &str) -> *const c_void {
    // Ensure library is initialized
    if init_gl_library().is_err() {
        return std::ptr::null();
    }

    let c_name = match CString::new(name) {
        Ok(s) => s,
        Err(_) => return std::ptr::null(),
    };

    unsafe {
        // First try wglGetProcAddress (for extension functions)
        let wgl_fns = WGL_FUNCTIONS.get().unwrap();
        let ptr = (wgl_fns.wgl_get_proc_address)(c_name.as_ptr());

        if !ptr.is_null()
            && ptr != std::ptr::null::<c_void>().wrapping_add(1)
            && ptr != std::ptr::null::<c_void>().wrapping_add(2)
            && ptr != std::ptr::null::<c_void>().wrapping_add(3)
            && ptr != std::ptr::null::<c_void>().wrapping_sub(1)
        {
            return ptr;
        }

        // Fall back to GetProcAddress from the DLL (for core GL 1.1 functions)
        let lib = GL_LIBRARY.get().unwrap();
        lib.get::<*const c_void>(c_name.as_bytes_with_nul())
            .map(|sym| *sym)
            .unwrap_or(std::ptr::null())
    }
}

/// Call wglCreateContext
pub unsafe fn wgl_create_context(hdc: isize) -> isize {
    let wgl_fns = WGL_FUNCTIONS.get().expect("WGL not initialized");
    (wgl_fns.wgl_create_context)(hdc)
}

/// Call wglDeleteContext
pub unsafe fn wgl_delete_context(hglrc: isize) -> bool {
    let wgl_fns = WGL_FUNCTIONS.get().expect("WGL not initialized");
    (wgl_fns.wgl_delete_context)(hglrc) != 0
}

/// Call wglMakeCurrent
pub unsafe fn wgl_make_current(hdc: isize, hglrc: isize) -> bool {
    let wgl_fns = WGL_FUNCTIONS.get().expect("WGL not initialized");
    (wgl_fns.wgl_make_current)(hdc, hglrc) != 0
}

/// Call wglGetCurrentContext
pub unsafe fn wgl_get_current_context() -> isize {
    let wgl_fns = WGL_FUNCTIONS.get().expect("WGL not initialized");
    (wgl_fns.wgl_get_current_context)()
}

/// Call wglGetCurrentDC
pub unsafe fn wgl_get_current_dc() -> isize {
    let wgl_fns = WGL_FUNCTIONS.get().expect("WGL not initialized");
    (wgl_fns.wgl_get_current_dc)()
}

// Windows GDI32 types and functions
#[repr(C)]
#[derive(Default)]
pub struct PixelFormatDescriptor {
    pub n_size: u16,
    pub n_version: u16,
    pub dw_flags: u32,
    pub i_pixel_type: u8,
    pub c_color_bits: u8,
    pub c_red_bits: u8,
    pub c_red_shift: u8,
    pub c_green_bits: u8,
    pub c_green_shift: u8,
    pub c_blue_bits: u8,
    pub c_blue_shift: u8,
    pub c_alpha_bits: u8,
    pub c_alpha_shift: u8,
    pub c_accum_bits: u8,
    pub c_accum_red_bits: u8,
    pub c_accum_green_bits: u8,
    pub c_accum_blue_bits: u8,
    pub c_accum_alpha_bits: u8,
    pub c_depth_bits: u8,
    pub c_stencil_bits: u8,
    pub c_aux_buffers: u8,
    pub i_layer_type: u8,
    pub b_reserved: u8,
    pub dw_layer_mask: u32,
    pub dw_visible_mask: u32,
    pub dw_damage_mask: u32,
}

// Pixel format flags
pub const PFD_DRAW_TO_WINDOW: u32 = 0x00000004;
pub const PFD_SUPPORT_OPENGL: u32 = 0x00000020;
pub const PFD_DOUBLEBUFFER: u32 = 0x00000001;
pub const PFD_TYPE_RGBA: u8 = 0;
pub const PFD_MAIN_PLANE: u8 = 0;

#[link(name = "gdi32")]
extern "system" {
    pub fn ChoosePixelFormat(hdc: isize, ppfd: *const PixelFormatDescriptor) -> i32;
    pub fn SetPixelFormat(hdc: isize, format: i32, ppfd: *const PixelFormatDescriptor) -> i32;
    pub fn SwapBuffers(hdc: isize) -> i32;
}

#[link(name = "user32")]
extern "system" {
    pub fn GetDC(hwnd: isize) -> isize;
    pub fn ReleaseDC(hwnd: isize, hdc: isize) -> i32;
}
