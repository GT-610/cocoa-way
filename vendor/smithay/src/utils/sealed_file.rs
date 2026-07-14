//! Sealed files for safe sharing with clients
//!
//! Uses memfd on Linux/Android/FreeBSD, tempfile on others

use std::{ffi::CStr, fs::File};

// Platform-specific fd imports
#[cfg(unix)]
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, RawFd};

#[cfg(windows)]
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, RawHandle};

#[cfg(windows)]
type RawFd = RawHandle;
#[cfg(windows)]
type BorrowedFd<'a> = BorrowedHandle<'a>;

/// A file whose fd cannot be written by other processes
///
/// This mechanism is useful for giving clients access to large amounts of
/// information such as keymaps without them being able to write to the handle.
///
/// On Linux, Android, and FreeBSD, this uses a sealed memfd. On other platforms
/// it creates a POSIX shared memory object with `shm_open`, opens a read-only
/// copy, and unlinks it.
#[derive(Debug)]
pub struct SealedFile {
    file: File,
    size: usize,
}

impl SealedFile {
    /// Create a `[SealedFile]` with the given nul-terminated C string.
    pub fn with_content(name: &CStr, contents: &CStr) -> Result<Self, std::io::Error> {
        Self::with_data(name, contents.to_bytes_with_nul())
    }

    /// Create a `[SealedFile]` with the given binary data.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "android"))]
    pub fn with_data(name: &CStr, data: &[u8]) -> Result<Self, std::io::Error> {
        use rustix::fs::{MemfdFlags, SealFlags};
        use std::io::{Seek, Write};

        let fd = rustix::fs::memfd_create(name, MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)?;

        let mut file: File = fd.into();
        file.write_all(data)?;
        file.flush()?;

        file.seek(std::io::SeekFrom::Start(0))?;

        rustix::fs::fcntl_add_seals(
            &file,
            SealFlags::SEAL | SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE,
        )?;

        Ok(Self {
            file,
            size: data.len(),
        })
    }

    /// Create a `[SealedFile]` with the given binary data.
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "android")))]
    pub fn with_data(_name: &CStr, data: &[u8]) -> Result<Self, std::io::Error> {
        use std::io::{Seek, SeekFrom, Write};

        let mut file = tempfile::tempfile()?;
        file.write_all(data)?;
        file.flush()?;
        file.seek(SeekFrom::Start(0))?;

        Ok(Self {
            file,
            size: data.len(),
        })
    }

    /// Size of the data contained in the sealed file.
    pub fn size(&self) -> usize {
        self.size
    }
}

#[cfg(unix)]
impl AsRawFd for SealedFile {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

#[cfg(unix)]
impl AsFd for SealedFile {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

#[cfg(windows)]
impl AsRawHandle for SealedFile {
    fn as_raw_handle(&self) -> RawHandle {
        std::os::windows::io::AsRawHandle::as_raw_handle(&self.file)
    }
}

#[cfg(windows)]
impl AsHandle for SealedFile {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        std::os::windows::io::AsHandle::as_handle(&self.file)
    }
}
