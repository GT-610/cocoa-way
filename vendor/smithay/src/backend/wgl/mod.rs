//! WGL (Windows OpenGL) backend for Windows
//!
//! This module provides OpenGL context management on Windows using WGL,
//! serving as a replacement for EGL on Unix systems.

mod context;
mod display;
mod ffi;

pub use context::*;
pub use display::*;
pub use ffi::get_proc_address;

use std::ffi::c_void;
use thiserror::Error;

/// WGL-related errors
#[derive(Debug, Error)]
pub enum Error {
    /// Failed to get device context
    #[error("Failed to get device context")]
    GetDCFailed,
    /// Failed to choose pixel format
    #[error("Failed to choose pixel format")]
    ChoosePixelFormatFailed,
    /// Failed to set pixel format
    #[error("Failed to set pixel format")]
    SetPixelFormatFailed,
    /// Failed to create context
    #[error("Failed to create OpenGL context")]
    ContextCreationFailed,
    /// Failed to make context current
    #[error("Failed to make context current")]
    MakeCurrentFailed,
    /// OpenGL extension not supported
    #[error("OpenGL extension not supported: {0}")]
    ExtensionNotSupported(&'static str),
    /// Library loading failed
    #[error("Failed to load OpenGL library: {0}")]
    LibraryLoadFailed(String),
}

/// Error when making a context current fails
#[derive(Debug, Error)]
#[error("Failed to make WGL context current")]
pub struct MakeCurrentError;
