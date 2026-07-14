//! Keyboard-related types for smithay's input abstraction
//! This version is a stub for platforms without xkbcommon (e.g., Windows without vcpkg).

#![allow(missing_debug_implementations)]
#![allow(dead_code)]

use crate::utils::{IsAlive, Serial};
use std::fmt;

/// Trait representing object that can receive keyboard interactions (Stub)
pub trait KeyboardTarget<D>: IsAlive + fmt::Debug + Send {}

/// Stub for KeyboardHandle
pub struct KeyboardHandle<D: ?Sized> {
    _marker: std::marker::PhantomData<fn() -> D>,
}

impl<D: ?Sized> Clone for KeyboardHandle<D> {
    fn clone(&self) -> Self {
        KeyboardHandle {
            _marker: std::marker::PhantomData,
        }
    }
}

impl<D: ?Sized> fmt::Debug for KeyboardHandle<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyboardHandle").finish()
    }
}

impl<D: ?Sized> PartialEq for KeyboardHandle<D> {
    fn eq(&self, _other: &Self) -> bool {
        true // Stub always equal
    }
}

/// Stub for KeysymHandle
#[derive(Debug)]
pub struct KeysymHandle<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
}

/// Stub for ModifiersState
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModifiersState;

/// Stub for XkbConfig
#[derive(Debug, Clone)]
pub struct XkbConfig<'a> {
    pub layout: &'a str,
    pub variant: &'a str,
    pub options: &'a str,
    pub rules: &'a str,
    pub model: &'a str,
}

impl Default for XkbConfig<'_> {
    fn default() -> Self {
        Self {
            layout: "",
            variant: "",
            options: "",
            rules: "",
            model: "",
        }
    }
}

/// Stub for LedState
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LedState;

/// Stub Error
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Keyboard not supported on this platform")]
    NotSupported,
}

impl<D> KeyboardHandle<D> {
    pub fn new(_config: XkbConfig<'_>, _delay: i32, _rate: i32) -> Result<Self, Error> {
        Err(Error::NotSupported)
    }

    pub fn set_focus(&self, _data: &mut D, _focus: Option<&mut dyn KeyboardTarget<D>>, _serial: Serial) {
        // No-op
    }

    pub fn input<T>(
        &self,
        _data: &mut D,
        _handle: &mut T,
        _keycode: u32,
        _state: crate::backend::input::KeyState,
        _serial: Serial,
        _time: u32,
    ) {
        // No-op
    }
}
