//! Proves that a Windows process has the selected local SQLite file open.

#![cfg(windows)]

use std::{
    ffi::c_void,
    os::windows::ffi::OsStrExt,
    path::{Component, Path},
    ptr,
};

const ERROR_MORE_DATA: u32 = 234;
const FILE_ID_INFO_CLASS: i32 = 18;
const OPEN_EXISTING: u32 = 3;
const FILE_SHARE_ALL: u32 = 0x0000_0007;
const GENERIC_READ: u32 = 0x8000_0000;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const MAX_RESTART_MANAGER_PROCESSES: usize = 4096;
const MAX_RESTART_MANAGER_ATTEMPTS: usize = 4;

type Handle = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    fn ticks(self) -> u64 {
        (u64::from(self.high) << 32) | u64::from(self.low)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RmUniqueProcess {
    process_id: u32,
    process_start_time: FileTime,
}

#[repr(C)]
struct FileIdInfo {
    volume_serial_number: u64,
    file_id: [u8; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub volume_serial_number: u64,
    pub file_id: [u8; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseBindingProof {
    pub pid: u32,
    pub process_creation_time: u64,
    pub database_identity: FileIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseBindingError {
    UnsupportedPath,
    Io,
    RestartManager,
    IncompleteProcessList,
    ProcessNotUsingDatabase,
    ProcessMismatch,
    FileChanged,
}

#[link(name = "rstrtmgr")]
unsafe extern "system" {
    fn RmStartSession(session: *mut u32, flags: u32, session_key: *mut u16) -> u32;
    fn RmRegisterResources(
        session: u32,
        file_count: u32,
        files: *const *const u16,
        application_count: u32,
        applications: *const RmUniqueProcess,
        service_count: u32,
        services: *const *const u16,
    ) -> u32;
    fn RmGetList(
        session: u32,
        needed: *mut u32,
        written: *mut u32,
        processes: *mut RmUniqueProcess,
        reboot_reasons: *mut u32,
    ) -> u32;
    fn RmEndSession(session: u32) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *const c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: Handle,
    ) -> Handle;
    fn GetFileInformationByHandleEx(
        file: Handle,
        class: i32,
        information: *mut c_void,
        size: u32,
    ) -> i32;
    fn GetDriveTypeW(root_path: *const u16) -> u32;
    fn GetVolumePathNameW(file_name: *const u16, volume_path_name: *mut u16, length: u32) -> i32;
    fn OpenProcess(access: u32, inherit_handle: i32, process_id: u32) -> Handle;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

struct HandleGuard(Handle);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct RestartManagerSession(u32);

impl RestartManagerSession {
    fn start() -> Result<Self, DatabaseBindingError> {
        let mut session = 0;
        let mut key = [0u16; 33];
        let status = unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) };
        if status != 0 {
            return Err(DatabaseBindingError::RestartManager);
        }
        Ok(Self(session))
    }

    fn register_file(&self, path: &[u16]) -> Result<(), DatabaseBindingError> {
        let files = [path.as_ptr()];
        let status = unsafe {
            RmRegisterResources(self.0, 1, files.as_ptr(), 0, ptr::null(), 0, ptr::null())
        };
        if status == 0 {
            Ok(())
        } else {
            Err(DatabaseBindingError::RestartManager)
        }
    }

    fn process_list(&self) -> Result<Vec<RmUniqueProcess>, DatabaseBindingError> {
        let mut capacity = 0usize;
        for _ in 0..MAX_RESTART_MANAGER_ATTEMPTS {
            let mut needed = 0u32;
            let mut written = capacity as u32;
            let mut reboot_reasons = 0u32;
            let mut processes = vec![
                RmUniqueProcess {
                    process_id: 0,
                    process_start_time: FileTime::default(),
                };
                capacity
            ];
            let process_buffer = if processes.is_empty() {
                ptr::null_mut()
            } else {
                processes.as_mut_ptr()
            };
            let status = unsafe {
                RmGetList(
                    self.0,
                    &mut needed,
                    &mut written,
                    process_buffer,
                    &mut reboot_reasons,
                )
            };
            if status == 0 {
                if written as usize > processes.len() {
                    return Err(DatabaseBindingError::IncompleteProcessList);
                }
                processes.truncate(written as usize);
                return Ok(processes);
            }
            if status != ERROR_MORE_DATA
                || needed == 0
                || needed as usize > MAX_RESTART_MANAGER_PROCESSES
            {
                return Err(DatabaseBindingError::IncompleteProcessList);
            }
            capacity = needed as usize;
        }
        Err(DatabaseBindingError::IncompleteProcessList)
    }
}

impl Drop for RestartManagerSession {
    fn drop(&mut self) {
        unsafe {
            RmEndSession(self.0);
        }
    }
}

/// Prove that `pid` has the exact local database file open.
pub fn prove_database_binding(
    pid: u32,
    database_path: &Path,
) -> Result<DatabaseBindingProof, DatabaseBindingError> {
    if pid == 0 {
        return Err(DatabaseBindingError::ProcessMismatch);
    }

    let canonical_path = canonical_local_path(database_path)?;
    let wide_path = nul_terminated(canonical_path.as_os_str().encode_wide());
    let identity_before = file_identity(&wide_path)?;

    let session = RestartManagerSession::start()?;
    session.register_file(&wide_path)?;
    let processes = session.process_list()?;
    let mut matches = processes.iter().filter(|process| process.process_id == pid);
    let registered_process = matches
        .next()
        .ok_or(DatabaseBindingError::ProcessNotUsingDatabase)?;
    if matches.next().is_some() {
        return Err(DatabaseBindingError::ProcessMismatch);
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(DatabaseBindingError::ProcessMismatch);
    }
    let _process_guard = HandleGuard(process);
    let mut creation_time = FileTime::default();
    let mut exit_time = FileTime::default();
    let mut kernel_time = FileTime::default();
    let mut user_time = FileTime::default();
    let got_times = unsafe {
        GetProcessTimes(
            process,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
    };
    if got_times == 0 || creation_time != registered_process.process_start_time {
        return Err(DatabaseBindingError::ProcessMismatch);
    }

    let identity_after = file_identity(&wide_path)?;
    if identity_before != identity_after {
        return Err(DatabaseBindingError::FileChanged);
    }

    Ok(DatabaseBindingProof {
        pid,
        process_creation_time: creation_time.ticks(),
        database_identity: identity_after,
    })
}

fn canonical_local_path(path: &Path) -> Result<std::path::PathBuf, DatabaseBindingError> {
    if !path.is_absolute() || path.to_string_lossy().starts_with("\\\\") {
        return Err(DatabaseBindingError::UnsupportedPath);
    }
    let canonical = path.canonicalize().map_err(|_| DatabaseBindingError::Io)?;
    let mut components = canonical.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(DatabaseBindingError::UnsupportedPath);
    };
    if !matches!(prefix.kind(), std::path::Prefix::Disk(_)) {
        return Err(DatabaseBindingError::UnsupportedPath);
    }
    let mut volume_root = vec![0u16; 32_768];
    if unsafe {
        GetVolumePathNameW(
            canonical
                .as_os_str()
                .encode_wide()
                .chain([0])
                .collect::<Vec<_>>()
                .as_ptr(),
            volume_root.as_mut_ptr(),
            volume_root.len() as u32,
        )
    } == 0
    {
        return Err(DatabaseBindingError::UnsupportedPath);
    }
    let root_len = volume_root
        .iter()
        .position(|character| *character == 0)
        .ok_or(DatabaseBindingError::UnsupportedPath)?;
    volume_root.truncate(root_len + 1);
    if unsafe { GetDriveTypeW(volume_root.as_ptr()) } != 3 {
        return Err(DatabaseBindingError::UnsupportedPath);
    }
    if !canonical.is_file() {
        return Err(DatabaseBindingError::UnsupportedPath);
    }
    Ok(canonical)
}

fn file_identity(path: &[u16]) -> Result<FileIdentity, DatabaseBindingError> {
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_ALL,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle as isize == -1 {
        return Err(DatabaseBindingError::Io);
    }
    let _handle_guard = HandleGuard(handle);
    let mut information = FileIdInfo {
        volume_serial_number: 0,
        file_id: [0; 16],
    };
    let succeeded = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FILE_ID_INFO_CLASS,
            (&mut information as *mut FileIdInfo).cast(),
            std::mem::size_of::<FileIdInfo>() as u32,
        )
    };
    if succeeded == 0 {
        return Err(DatabaseBindingError::Io);
    }
    Ok(FileIdentity {
        volume_serial_number: information.volume_serial_number,
        file_id: information.file_id,
    })
}

fn nul_terminated(wide: impl Iterator<Item = u16>) -> Vec<u16> {
    wide.chain(std::iter::once(0)).collect()
}
