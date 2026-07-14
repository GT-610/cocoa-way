//! Cross-platform OpenGL function loader
//!
//! This module provides a platform-agnostic way to load OpenGL functions.
//! On Unix, it uses EGL; on Windows, it uses WGL.

use std::ffi::c_void;

/// Get the address of an OpenGL function by name
///
/// This function abstracts over platform-specific GL loading mechanisms.
#[cfg(all(unix, feature = "backend_egl"))]
pub fn get_proc_address(name: &str) -> *const c_void {
    crate::backend::egl::get_proc_address(name)
}

#[cfg(all(windows, feature = "backend_wgl"))]
pub fn get_proc_address(name: &str) -> *const c_void {
    crate::backend::wgl::get_proc_address(name)
}

// Fallback for unsupported configurations
#[cfg(not(any(all(unix, feature = "backend_egl"), all(windows, feature = "backend_wgl"))))]
pub fn get_proc_address(_name: &str) -> *const c_void {
    std::ptr::null()
}
