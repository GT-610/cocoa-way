#![forbid(unsafe_op_in_unsafe_fn)]

use std::{path::PathBuf, sync::Arc};

// Use platform-compatible fd types
#[cfg(unix)]
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

#[cfg(windows)]
use crate::compat::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle, RawHandle};

/// Ref-counted file descriptor of an open device node
#[derive(Debug, Clone)]
pub struct DeviceFd(Arc<OwnedFd>);

impl PartialEq for DeviceFd {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_raw_fd() == other.as_raw_fd()
    }
}

impl AsFd for DeviceFd {
    #[inline]
    fn as_fd(&self) -> BorrowedFd<'_> {
        #[cfg(unix)]
        {
            self.0.as_fd()
        }
        #[cfg(windows)]
        {
            AsHandle::as_handle(&*self.0)
        }
    }
}

impl AsRawFd for DeviceFd {
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        #[cfg(unix)]
        {
            self.0.as_raw_fd()
        }
        #[cfg(windows)]
        {
            AsRawHandle::as_raw_handle(&*self.0)
        }
    }
}

impl FromRawFd for DeviceFd {
    /// SAFETY:
    /// Make sure that `fd` is a valid value!
    #[inline]
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        #[cfg(unix)]
        {
            DeviceFd(Arc::new(unsafe { OwnedFd::from_raw_fd(fd) }))
        }
        #[cfg(windows)]
        {
            DeviceFd(Arc::new(unsafe { FromRawHandle::from_raw_handle(fd) }))
        }
    }
}

impl From<OwnedFd> for DeviceFd {
    #[inline]
    fn from(fd: OwnedFd) -> Self {
        DeviceFd(Arc::new(fd))
    }
}

impl TryInto<OwnedFd> for DeviceFd {
    type Error = DeviceFd;

    #[inline]
    fn try_into(self) -> Result<OwnedFd, Self::Error> {
        Arc::try_unwrap(self.0).map_err(DeviceFd)
    }
}

/// Trait representing open devices that *may* return a `Path`
pub trait DevPath {
    /// Returns the path of the open device if possible
    fn dev_path(&self) -> Option<PathBuf>;
}

#[cfg(unix)]
impl<A: AsFd> DevPath for A {
    fn dev_path(&self) -> Option<PathBuf> {
        use std::fs;
        use std::os::unix::io::AsRawFd;
        fs::read_link(format!("/proc/self/fd/{:?}", self.as_fd().as_raw_fd())).ok()
    }
}

#[cfg(windows)]
impl<A: AsFd> DevPath for A {
    fn dev_path(&self) -> Option<PathBuf> {
        // Windows doesn't have /proc/self/fd, return None
        None
    }
}
