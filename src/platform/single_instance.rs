//! 单实例：Windows 命名互斥量（当前用户会话作用域）/ Linux `fs2` 文件锁。
//!
//! [`SingleInstance::acquire`] 在成功获取锁时返回 `Some`，已有实例持有时返回
//! `None`（调用方据此打开已有实例的管理页并退出）。

use std::io;
use std::path::Path;

/// 持有的单实例锁；Drop 时释放。
pub struct SingleInstance {
    /// Windows 互斥量句柄。
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    /// Linux 锁文件（Drop 释放 `flock`）。
    #[cfg(not(windows))]
    _file: std::fs::File,
}

impl SingleInstance {
    /// 以平台默认锁路径获取单实例锁。
    pub fn acquire() -> io::Result<Option<SingleInstance>> {
        Self::acquire_at(&crate::paths::lock_path())
    }

    /// 以指定路径获取单实例锁。
    ///
    /// Windows 走命名互斥量（忽略 `path`）；Linux 对 `path` 加排他文件锁。
    #[cfg_attr(windows, allow(unused_variables))]
    pub fn acquire_at(path: &Path) -> io::Result<Option<SingleInstance>> {
        #[cfg(windows)]
        {
            windows_acquire()
        }
        #[cfg(not(windows))]
        {
            unix_acquire(path)
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
        // Unix：`_file` Drop 时自动释放文件锁。
    }
}

/// Windows 命名互斥量（会话作用域）。
#[cfg(windows)]
fn windows_acquire() -> io::Result<Option<SingleInstance>> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "WebSocket2Serial-SingleInstance"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr());
        if handle == 0 {
            return Err(io::Error::last_os_error());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(handle);
            return Ok(None);
        }
        Ok(Some(SingleInstance { handle }))
    }
}

/// Linux 排他文件锁。
#[cfg(not(windows))]
fn unix_acquire(path: &Path) -> io::Result<Option<SingleInstance>> {
    use std::io::Write;

    use fs2::FileExt;

    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)?;
    match file.try_lock_exclusive() {
        Ok(()) => {
            // 记录 PID 供诊断（截断后写入）。
            file.set_len(0)?;
            writeln!(&file, "{}", std::process::id())?;
            Ok(Some(SingleInstance { _file: file }))
        }
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(err) => Err(err),
    }
}
