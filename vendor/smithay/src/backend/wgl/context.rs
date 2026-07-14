//! WGL Context handling
//!
//! Manages OpenGL rendering contexts on Windows.

use std::sync::Arc;

use super::display::WGLDisplay;
use super::ffi;
use super::{Error, MakeCurrentError};

/// Handle to a WGL rendering context
#[derive(Debug)]
struct WGLContextHandle {
    /// The rendering context handle
    hglrc: isize,
    /// The associated display
    display: WGLDisplay,
}

impl Drop for WGLContextHandle {
    fn drop(&mut self) {
        unsafe {
            // Make sure we're not current before deleting
            if ffi::wgl_get_current_context() == self.hglrc {
                ffi::wgl_make_current(0, 0);
            }
            ffi::wgl_delete_context(self.hglrc);
        }
    }
}

/// A WGL OpenGL rendering context
#[derive(Debug, Clone)]
pub struct WGLContext {
    handle: Arc<WGLContextHandle>,
}

impl WGLContext {
    /// Create a new WGL context for the given display
    pub fn new(display: &WGLDisplay) -> Result<Self, Error> {
        let hglrc = unsafe { ffi::wgl_create_context(display.hdc()) };
        if hglrc == 0 {
            return Err(Error::ContextCreationFailed);
        }

        Ok(Self {
            handle: Arc::new(WGLContextHandle {
                hglrc,
                display: display.clone(),
            }),
        })
    }

    /// Make this context current
    pub fn make_current(&self) -> Result<(), MakeCurrentError> {
        let success = unsafe { ffi::wgl_make_current(self.handle.display.hdc(), self.handle.hglrc) };

        if success {
            Ok(())
        } else {
            Err(MakeCurrentError)
        }
    }

    /// Check if this context is current
    pub fn is_current(&self) -> bool {
        unsafe { ffi::wgl_get_current_context() == self.handle.hglrc }
    }

    /// Unbind the current context
    pub fn unbind() -> Result<(), MakeCurrentError> {
        let success = unsafe { ffi::wgl_make_current(0, 0) };
        if success {
            Ok(())
        } else {
            Err(MakeCurrentError)
        }
    }

    /// Get the associated display
    pub fn display(&self) -> &WGLDisplay {
        &self.handle.display
    }

    /// Swap buffers for this context's display
    pub fn swap_buffers(&self) -> bool {
        self.handle.display.swap_buffers()
    }

    /// Get the raw HGLRC handle
    pub fn hglrc(&self) -> isize {
        self.handle.hglrc
    }
}
