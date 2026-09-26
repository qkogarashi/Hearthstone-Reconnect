use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elev = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elev as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );

        let _ = CloseHandle(token);
        ok.is_ok() && elev.TokenIsElevated != 0
    }
}

pub fn relaunch_as_admin() {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let exe = std::env::current_exe().unwrap();
    let path: Vec<u16> = exe
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(path.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };

    if result.0 as isize > 32 {
        std::process::exit(0);
    }
}
