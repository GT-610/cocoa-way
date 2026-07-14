//! Platform compatibility layer for cross-platform support
//!
//! This module provides abstractions over platform-specific APIs to enable
//! smithay to compile and run on Windows.

#[cfg(unix)]
/// File-descriptor types and traits used by Unix platform code.
pub mod fd {
    pub use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
}

#[cfg(windows)]
pub mod fd {
    use std::os::windows::io::{
        AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, IntoRawHandle, OwnedHandle, RawHandle,
    };

    /// Windows equivalent of RawFd
    pub type RawFd = RawHandle;

    /// Windows equivalent of OwnedFd
    pub type OwnedFd = OwnedHandle;

    /// Windows equivalent of BorrowedFd
    pub type BorrowedFd<'a> = BorrowedHandle<'a>;

    /// Trait for types that have a raw fd/handle
    pub trait AsRawFd {
        fn as_raw_fd(&self) -> RawFd;
    }

    impl<T: AsRawHandle> AsRawFd for T {
        fn as_raw_fd(&self) -> RawFd {
            self.as_raw_handle()
        }
    }

    /// Trait for types that can borrow their fd/handle
    pub trait AsFd {
        fn as_fd(&self) -> BorrowedFd<'_>;
    }

    impl<T: AsHandle> AsFd for T {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.as_handle()
        }
    }

    /// Trait for creating from raw fd/handle
    pub trait FromRawFd {
        unsafe fn from_raw_fd(fd: RawFd) -> Self;
    }

    impl<T: FromRawHandle> FromRawFd for T {
        unsafe fn from_raw_fd(fd: RawFd) -> Self {
            Self::from_raw_handle(fd)
        }
    }

    /// Trait for converting into raw fd/handle
    pub trait IntoRawFd {
        fn into_raw_fd(self) -> RawFd;
    }

    impl<T: IntoRawHandle> IntoRawFd for T {
        fn into_raw_fd(self) -> RawFd {
            self.into_raw_handle()
        }
    }
}

pub use fd::*;

/// Cross-platform time utilities
#[cfg(unix)]
pub mod time {
    pub use rustix::time::{clock_gettime, ClockId, Timespec};
}

#[cfg(windows)]
pub mod time {
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    /// Clock ID for Windows (simplified)
    #[derive(Debug, Clone, Copy)]
    pub enum ClockId {
        Monotonic,
        Realtime,
    }

    /// Timespec for Windows
    #[derive(Debug, Clone, Copy)]
    pub struct Timespec {
        pub tv_sec: i64,
        pub tv_nsec: i64,
    }

    impl Timespec {
        pub fn new(sec: i64, nsec: i64) -> Self {
            Self {
                tv_sec: sec,
                tv_nsec: nsec,
            }
        }
    }

    /// Get current time for the given clock
    pub fn clock_gettime(clock: ClockId) -> Timespec {
        match clock {
            ClockId::Monotonic => {
                // Use Instant for monotonic time
                static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
                let start = START.get_or_init(Instant::now);
                let elapsed = start.elapsed();
                Timespec {
                    tv_sec: elapsed.as_secs() as i64,
                    tv_nsec: elapsed.subsec_nanos() as i64,
                }
            }
            ClockId::Realtime => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO);
                Timespec {
                    tv_sec: now.as_secs() as i64,
                    tv_nsec: now.subsec_nanos() as i64,
                }
            }
        }
    }
}

/// Cross-platform memory mapping (stub for Windows)
#[cfg(windows)]
pub mod mman {
    use std::ptr;

    pub const PROT_READ: i32 = 1;
    pub const PROT_WRITE: i32 = 2;
    pub const MAP_SHARED: i32 = 1;

    /// Memory-mapped region (Windows stub)
    pub struct MmapRegion {
        ptr: *mut u8,
        len: usize,
    }

    impl MmapRegion {
        pub fn new(_fd: super::RawFd, _len: usize, _prot: i32, _flags: i32) -> std::io::Result<Self> {
            // TODO: Implement Windows memory mapping with CreateFileMapping + MapViewOfFile
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Memory mapping not yet implemented on Windows",
            ))
        }

        pub fn as_ptr(&self) -> *const u8 {
            self.ptr
        }

        pub fn as_mut_ptr(&mut self) -> *mut u8 {
            self.ptr
        }

        pub fn len(&self) -> usize {
            self.len
        }
    }
}
