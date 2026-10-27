//! Keep plug-in diagnostic output separate from the protocol reply.
use std::{fs::File, io, os::windows::io::FromRawHandle, ptr};
use windows_sys::Win32::{
    Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle},
    System::{
        Console::{GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle},
        Threading::GetCurrentProcess,
    },
};
unsafe extern "C" {
    fn _dup2(source: i32, destination: i32) -> i32;
}
pub fn reserve() -> io::Result<File> {
    let mut copy = ptr::null_mut();
    // SAFETY: DuplicateHandle gives this module ownership of a fresh reply-pipe handle. The
    // plug-in sees only stderr through Win32 stdout and the C runtime's descriptor 1.
    unsafe {
        let process = GetCurrentProcess();
        if DuplicateHandle(process, GetStdHandle(STD_OUTPUT_HANDLE), process, &mut copy, 0, 0, DUPLICATE_SAME_ACCESS) == 0 {
            return Err(io::Error::last_os_error());
        }
        let file = File::from_raw_handle(copy);
        if SetStdHandle(STD_OUTPUT_HANDLE, GetStdHandle(STD_ERROR_HANDLE)) == 0 {
            return Err(io::Error::last_os_error());
        }
        if _dup2(2, 1) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(file)
    }
}
