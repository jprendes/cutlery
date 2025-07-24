use core::mem::{size_of, zeroed};
use core::ptr::{null, null_mut};
use core::slice;
use std::io::{Error, Result};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use phnt::ffi::{
    NtCreateUserProcess, NtQueryInformationProcess, NtSetInformationObject, OBJ_INHERIT,
    OBJ_PROTECT_CLOSE, OBJECT_HANDLE_FLAG_INFORMATION, OBJECT_INFORMATION_CLASS,
    PROCESS_CREATE_FLAGS_INHERIT_HANDLES, PROCESS_HANDLE_SNAPSHOT_INFORMATION,
    PROCESS_HANDLE_TABLE_ENTRY_INFO, PROCESSINFOCLASS, PS_CREATE_INFO, ULONG,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, HANDLE, NTSTATUS, STATUS_BUFFER_OVERFLOW,
    STATUS_BUFFER_TOO_SMALL, STATUS_INFO_LENGTH_MISMATCH, STATUS_PROCESS_CLONED, STATUS_SUCCESS,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, FreeConsole};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessId, INFINITE, PROCESS_ALL_ACCESS,
    THREAD_ALL_ACCESS, TerminateProcess, WaitForSingleObject,
};

use super::{Child, Fork};

pub(super) type OwnedFileDescriptor = OwnedHandle;

// Based on https://github.com/huntandhackett/process-cloning/blob/master/1.NtCreateUserProcess/main.c

pub(super) fn fork() -> Result<Fork> {
    let mut process_handle = HANDLE::default();
    let mut thread_handle = HANDLE::default();
    let mut create_info: PS_CREATE_INFO = unsafe { zeroed() };
    create_info.Size = size_of::<PS_CREATE_INFO>() as u64;

    let handles = snapshot_all_handles().unwrap();

    for handle in &handles {
        make_inheritable(handle, true);
    }

    let status = unsafe {
        NtCreateUserProcess(
            &mut process_handle.0 as *mut _,
            &mut thread_handle.0 as *mut _,
            PROCESS_ALL_ACCESS.0,
            THREAD_ALL_ACCESS.0,
            null(),
            null(),
            PROCESS_CREATE_FLAGS_INHERIT_HANDLES,
            0,
            null_mut(),
            &mut create_info as *mut _,
            null_mut(),
        )
    };

    let status = NTSTATUS(status);

    for handle in &handles {
        make_inheritable(handle, false);
    }

    if status == STATUS_PROCESS_CLONED {
        let _ = unsafe { FreeConsole() };
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };

        Ok(Fork::Child)
    } else {
        if let Err(err) = status.ok() {
            println!("Failed to clone the current process: {err:?}");
            return Err(err.into());
        }

        let _ = unsafe { CloseHandle(thread_handle) };

        let pid = unsafe { GetProcessId(process_handle) };
        let descriptor = unsafe { OwnedHandle::from_raw_handle(process_handle.0) };
        let status = None;

        Ok(Fork::Parent(Child {
            pid,
            descriptor,
            status,
        }))
    }
}

pub(super) fn wait(child: &Child) -> Result<i32> {
    let event = unsafe { WaitForSingleObject(HANDLE(child.descriptor.as_raw_handle()), INFINITE) };

    if event != WAIT_OBJECT_0 {
        return Err(Error::last_os_error());
    }

    let mut code = 0u32;
    unsafe {
        GetExitCodeProcess(
            HANDLE(child.descriptor.as_raw_handle()),
            &mut code as *mut _,
        )
    }?;

    Ok(code as _)
}

pub(super) fn try_wait(child: &Child) -> Result<Option<i32>> {
    let event = unsafe { WaitForSingleObject(HANDLE(child.descriptor.as_raw_handle()), 0) };

    match event {
        WAIT_OBJECT_0 => {}
        WAIT_TIMEOUT => return Ok(None),
        _ => return Err(Error::last_os_error()),
    }

    let mut code = 0u32;
    unsafe {
        GetExitCodeProcess(
            HANDLE(child.descriptor.as_raw_handle()),
            &mut code as *mut _,
        )
    }?;

    Ok(Some(code as _))
}

pub(super) fn kill(child: &Child) -> Result<()> {
    let result = unsafe { TerminateProcess(HANDLE(child.descriptor.as_raw_handle()), 1) };
    if let Err(err) = result {
        // TerminateProcess returns ERROR_ACCESS_DENIED if the process has already been
        // terminated (by us, or for any other reason). So check if the process was actually
        // terminated, and if so, do not return an error.
        if err.code() != ERROR_ACCESS_DENIED.to_hresult() || try_wait(child).is_err() {
            return Err(err.into());
        }
    }
    Ok(())
}

fn snapshot_all_handles() -> Result<Vec<PROCESS_HANDLE_TABLE_ENTRY_INFO>> {
    let mut buffer = vec![0u8; 0x800]; // 2kiB to start with

    loop {
        let mut size = buffer.len() as ULONG;
        let status = unsafe {
            NtQueryInformationProcess(
                GetCurrentProcess().0,
                PROCESSINFOCLASS::ProcessHandleInformation,
                buffer.as_mut_ptr() as _,
                size,
                &mut size as *mut _,
            )
        };
        match NTSTATUS(status) {
            STATUS_INFO_LENGTH_MISMATCH | STATUS_BUFFER_TOO_SMALL | STATUS_BUFFER_OVERFLOW => {
                buffer.resize(size as _, 0);
            }
            STATUS_SUCCESS => break,
            status => {
                status.ok()?;
            }
        }
    }

    let buffer = buffer.as_ptr() as *const PROCESS_HANDLE_SNAPSHOT_INFORMATION;
    let snapshot = unsafe { buffer.as_ref().unwrap() };
    let handles =
        unsafe { slice::from_raw_parts(snapshot.Handles.as_ptr(), snapshot.NumberOfHandles as _) };
    let handles = handles.to_vec();

    Ok(handles)
}

fn make_inheritable(handle: &PROCESS_HANDLE_TABLE_ENTRY_INFO, force_inherit: bool) {
    let attrs = handle.HandleAttributes | if force_inherit { OBJ_INHERIT } else { 0 };

    let mut flags = OBJECT_HANDLE_FLAG_INFORMATION {
        Inherit: (attrs & OBJ_INHERIT) as _,
        ProtectFromClose: (attrs & OBJ_PROTECT_CLOSE) as _,
    };

    unsafe {
        NtSetInformationObject(
            handle.HandleValue,
            OBJECT_INFORMATION_CLASS::ObjectHandleFlagInformation,
            &mut flags as *mut _ as *mut _,
            size_of::<OBJECT_HANDLE_FLAG_INFORMATION>() as _,
        )
    };
}
