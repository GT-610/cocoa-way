//! WGL Display handling
//!
//! Wraps a Windows device context (HDC) for OpenGL rendering.

use std::sync::Arc;

use super::ffi;
use super::Error;

/// Handle to a Windows device context for OpenGL rendering
#[derive(Debug)]
pub struct WGLDisplayHandle {
    /// The device context handle
    pub hdc: isize,
    /// The window handle (if any)
    hwnd: Option<isize>,
    /// Whether we own the DC and should release it
    owned: bool,
}

impl Drop for WGLDisplayHandle {
    fn drop(&mut self) {
        if self.owned {
            if let Some(hwnd) = self.hwnd {
                unsafe {
                    ffi::ReleaseDC(hwnd, self.hdc);
                }
            }
        }
    }
}

/// A WGL display (device context wrapper)
#[derive(Debug, Clone)]
pub struct WGLDisplay {
    handle: Arc<WGLDisplayHandle>,
}

impl WGLDisplay {
    /// Create a new WGLDisplay from a window handle
    ///
    /// # Safety
    /// The window handle must be valid for the lifetime of the display.
    pub unsafe fn from_window(hwnd: isize) -> Result<Self, Error> {
        ffi::init_gl_library()?;

        let hdc = ffi::GetDC(hwnd);
        if hdc == 0 {
            return Err(Error::GetDCFailed);
        }

        // Set up pixel format
        let pfd = ffi::PixelFormatDescriptor {
            n_size: std::mem::size_of::<ffi::PixelFormatDescriptor>() as u16,
            n_version: 1,
            dw_flags: ffi::PFD_DRAW_TO_WINDOW | ffi::PFD_SUPPORT_OPENGL | ffi::PFD_DOUBLEBUFFER,
            i_pixel_type: ffi::PFD_TYPE_RGBA,
            c_color_bits: 32,
            c_depth_bits: 24,
            c_stencil_bits: 8,
            i_layer_type: ffi::PFD_MAIN_PLANE,
            ..Default::default()
        };

        let pixel_format = ffi::ChoosePixelFormat(hdc, &pfd);
        if pixel_format == 0 {
            ffi::ReleaseDC(hwnd, hdc);
            return Err(Error::ChoosePixelFormatFailed);
        }

        if ffi::SetPixelFormat(hdc, pixel_format, &pfd) == 0 {
            ffi::ReleaseDC(hwnd, hdc);
            return Err(Error::SetPixelFormatFailed);
        }

        Ok(Self {
            handle: Arc::new(WGLDisplayHandle {
                hdc,
                hwnd: Some(hwnd),
                owned: true,
            }),
        })
    }

    /// Create from existing HDC (caller retains ownership)
    ///
    /// # Safety
    /// The HDC must be valid and have a suitable pixel format set.
    pub unsafe fn from_raw(hdc: isize) -> Result<Self, Error> {
        ffi::init_gl_library()?;

        if hdc == 0 {
            return Err(Error::GetDCFailed);
        }

        Ok(Self {
            handle: Arc::new(WGLDisplayHandle {
                hdc,
                hwnd: None,
                owned: false,
            }),
        })
    }

    /// Get the raw HDC handle
    pub fn hdc(&self) -> isize {
        self.handle.hdc
    }

    /// Swap buffers (for double buffering)
    pub fn swap_buffers(&self) -> bool {
        unsafe { ffi::SwapBuffers(self.handle.hdc) != 0 }
    }
}
