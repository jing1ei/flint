//! Native shell operations: paths are data, never cmd.exe syntax.
use std::path::Path;

pub fn open(path: &Path, reveal: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::{
            Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName, ShellExecuteW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };
    let path = dunce::simplified(path);
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if wide[..wide.len() - 1].contains(&0) {
        return Err("Invalid path.".into());
    }
    // Called on a blocking worker. Balance COM initialization and release the shell allocation.
    unsafe {
        let initialized = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        if initialized < 0 {
            return Err(format!("Could not initialize Windows shell ({initialized:#x})."));
        }
        let result = if reveal {
            let mut item = std::ptr::null_mut();
            let parsed = SHParseDisplayName(
                wide.as_ptr(),
                std::ptr::null_mut(),
                &mut item,
                0,
                std::ptr::null_mut(),
            );
            let opened = if parsed >= 0 {
                SHOpenFolderAndSelectItems(item, 0, std::ptr::null(), 0)
            } else {
                parsed
            };
            if !item.is_null() {
                CoTaskMemFree(item.cast());
            }
            if opened >= 0 {
                Ok(())
            } else {
                Err(format!("Could not reveal the path ({opened:#x})."))
            }
        } else {
            let code = ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            ) as isize;
            if code > 32 {
                Ok(())
            } else {
                Err(format!("Windows could not open the path (error {code})."))
            }
        };
        CoUninitialize();
        result
    }
}
