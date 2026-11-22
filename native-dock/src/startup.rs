//! Per-user login startup for the portable native edition, without Python.
use std::{
    mem::{size_of, zeroed},
    os::windows::process::CommandExt,
    path::Path,
    process::Command,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_FILE_NOT_FOUND, HANDLE},
    Security::{
        DuplicateTokenEx, GetTokenInformation, SecurityImpersonation, TokenElevation,
        TokenPrimary, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
        TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_QUERY,
    },
    System::{
        Registry::*,
        Threading::{
            GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken,
            WaitForSingleObject, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE, STARTUPINFOW,
        },
    },
    UI::{
        Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId, SW_HIDE},
    },
};

#[link(name = "advapi32")]
extern "system" {
    fn CreateProcessWithTokenW(
        hToken: HANDLE,
        dwLogonFlags: u32,
        lpApplicationName: *const u16,
        lpCommandLine: *mut u16,
        dwCreationFlags: u32,
        lpEnvironment: *const std::ffi::c_void,
        lpCurrentDirectory: *const u16,
        lpStartupInfo: *const STARTUPINFOW,
        lpProcessInformation: *mut PROCESS_INFORMATION,
    ) -> i32;
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const TASK_NAME: &str = "Lumen";

fn exe_path(root: &Path) -> std::path::PathBuf {
    let portable = root.join("Lumen.exe");
    if portable.is_file() {
        portable
    } else {
        std::env::current_exe().unwrap_or(portable)
    }
}

pub fn set_enabled(root: &Path, enabled: bool) -> Result<(), String> {
    let command = format!("\"{}\"", exe_path(root).display());
    let path = crate::model::wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = crate::model::wide("Lumen");
    let mut key = null_mut();
    unsafe {
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        );
        if status != 0 {
            return Err(format!("Cannot open login startup settings: {status}"));
        }
        let status = if enabled {
            let value = crate::model::wide(command);
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as _,
                (value.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key, name.as_ptr())
        };
        RegCloseKey(key);
        if status != 0 && !(status == ERROR_FILE_NOT_FOUND && !enabled) {
            return Err(format!("Cannot change login startup: {status}"));
        }
    }
    Ok(())
}

/// Runs before window creation to handle elevated Task Scheduler setup or
/// waiting for a restarting parent process. Returns `true` if handled and the
/// process should exit immediately.
pub fn dispatch() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--wait-pid") {
        if let Some(pid_str) = args.get(pos + 1) {
            if let Ok(pid) = pid_str.parse::<u32>() {
                unsafe {
                    let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
                    if !h.is_null() {
                        WaitForSingleObject(h, 15_000);
                        CloseHandle(h);
                    }
                }
            }
        }
        return false;
    }
    if args.iter().any(|a| a == "--apply-fast-startup-enable") {
        let root = crate::model::root();
        let ok = apply_task_scheduler(&root, true).is_ok();
        std::process::exit(if ok { 0 } else { 1 });
    }
    if args.iter().any(|a| a == "--apply-fast-startup-disable") {
        let root = crate::model::root();
        let ok = apply_task_scheduler(&root, false).is_ok();
        std::process::exit(if ok { 0 } else { 1 });
    }
    false
}

fn apply_task_scheduler(root: &Path, enabled: bool) -> Result<(), String> {
    if enabled {
        let tr = format!("\"{}\"", exe_path(root).display());
        let out = Command::new("schtasks.exe")
            .args([
                "/Create",
                "/TN",
                TASK_NAME,
                "/TR",
                &tr,
                "/SC",
                "ONLOGON",
                "/RL",
                "LIMITED",
                "/F",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        let _ = set_enabled(root, false);
        Ok(())
    } else {
        let out = Command::new("schtasks.exe")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.to_lowercase().contains("cannot find") {
                return Err(stderr.trim().to_string());
            }
        }
        Ok(())
    }
}

pub fn set_fast_startup_elevated(enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let verb = crate::model::wide("runas");
    let file = crate::model::wide(exe);
    let param = crate::model::wide(if enabled {
        "--apply-fast-startup-enable"
    } else {
        "--apply-fast-startup-disable"
    });
    unsafe {
        let mut info: SHELLEXECUTEINFOW = zeroed();
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = param.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 {
            return Err("Administrator permission is required for Fast startup.".into());
        }
        if info.hProcess.is_null() {
            return Err("Could not monitor elevated startup helper.".into());
        }
        WaitForSingleObject(info.hProcess, 15_000);
        let mut code = 1u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if code == 0 {
            Ok(())
        } else {
            Err("Failed to update Task Scheduler startup task.".into())
        }
    }
}

pub fn restart_app() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Command::new(exe)
        .arg("--wait-pid")
        .arg(std::process::id().to_string())
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn quote_win_arg(arg: &std::ffi::OsStr) -> String {
    let s = arg.to_string_lossy();
    if !s.is_empty() && !s.bytes().any(|b| matches!(b, b' ' | b'\t' | b'"')) {
        return s.into_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for ch in s.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            for _ in 0..(backslashes * 2 + 1) {
                out.push('\\');
            }
            out.push('"');
            backslashes = 0;
        } else {
            for _ in 0..backslashes {
                out.push('\\');
            }
            backslashes = 0;
            out.push(ch);
        }
    }
    for _ in 0..(backslashes * 2) {
        out.push('\\');
    }
    out.push('"');
    out
}

unsafe fn is_elevated() -> bool {
    let mut token: HANDLE = null_mut();
    if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 || token.is_null() {
        return false;
    }
    let mut elev: TOKEN_ELEVATION = zeroed();
    let mut len = 0u32;
    let ok = GetTokenInformation(
        token,
        TokenElevation,
        &mut elev as *mut _ as _,
        size_of::<TOKEN_ELEVATION>() as u32,
        &mut len,
    );
    CloseHandle(token);
    ok != 0 && elev.TokenIsElevated != 0
}

pub fn de_elevate_if_needed() -> bool {
    if std::env::args().any(|a| matches!(a.as_str(), "--no-de-elevate" | "--quit" | "--smoke-test"))
    {
        return false;
    }
    unsafe {
        if !is_elevated() {
            return false;
        }
        let mut shell_hwnd = GetShellWindow();
        for _ in 0..19 {
            if !shell_hwnd.is_null() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
            shell_hwnd = GetShellWindow();
        }
        if shell_hwnd.is_null() {
            crate::logger::warn("app", "de-elevation skipped: shell window unavailable");
            return false;
        }
        let mut shell_pid = 0u32;
        GetWindowThreadProcessId(shell_hwnd, &mut shell_pid);
        if shell_pid == 0 {
            crate::logger::warn(
                "app",
                format!(
                    "de-elevation failed (err={}); continuing elevated",
                    GetLastError()
                ),
            );
            return false;
        }
        let shell_proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, shell_pid);
        if shell_proc.is_null() {
            crate::logger::warn(
                "app",
                format!(
                    "de-elevation failed (err={}); continuing elevated",
                    GetLastError()
                ),
            );
            return false;
        }
        let mut shell_token: HANDLE = null_mut();
        if OpenProcessToken(shell_proc, TOKEN_DUPLICATE, &mut shell_token) == 0
            || shell_token.is_null()
        {
            let err = GetLastError();
            CloseHandle(shell_proc);
            crate::logger::warn(
                "app",
                format!("de-elevation failed (err={err}); continuing elevated"),
            );
            return false;
        }
        let mut primary_token: HANDLE = null_mut();
        let dup_ok = DuplicateTokenEx(
            shell_token,
            TOKEN_ASSIGN_PRIMARY
                | TOKEN_DUPLICATE
                | TOKEN_QUERY
                | TOKEN_ADJUST_DEFAULT
                | TOKEN_ADJUST_SESSIONID,
            null(),
            SecurityImpersonation,
            TokenPrimary,
            &mut primary_token,
        );
        if dup_ok == 0 || primary_token.is_null() {
            let err = GetLastError();
            CloseHandle(shell_token);
            CloseHandle(shell_proc);
            crate::logger::warn(
                "app",
                format!("de-elevation failed (err={err}); continuing elevated"),
            );
            return false;
        }

        let Ok(exe) = std::env::current_exe() else {
            CloseHandle(primary_token);
            CloseHandle(shell_token);
            CloseHandle(shell_proc);
            return false;
        };
        let mut cmd = quote_win_arg(exe.as_os_str());
        for arg in std::env::args_os().skip(1) {
            cmd.push(' ');
            cmd.push_str(&quote_win_arg(&arg));
        }
        cmd.push_str(" --no-de-elevate");
        let exe_w = crate::model::wide(exe);
        let mut cmd_w = crate::model::wide(cmd);
        let mut si: STARTUPINFOW = zeroed();
        si.cb = size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = zeroed();
        let created = CreateProcessWithTokenW(
            primary_token,
            0,
            exe_w.as_ptr(),
            cmd_w.as_mut_ptr(),
            0,
            null(),
            null(),
            &si,
            &mut pi,
        );
        let err = if created == 0 { GetLastError() } else { 0 };
        if !pi.hThread.is_null() {
            CloseHandle(pi.hThread);
        }
        if !pi.hProcess.is_null() {
            CloseHandle(pi.hProcess);
        }
        CloseHandle(primary_token);
        CloseHandle(shell_token);
        CloseHandle(shell_proc);
        if created != 0 {
            crate::logger::info(
                "app",
                format!(
                    "elevated launch de-elevated to shell token (child_pid={})",
                    pi.dwProcessId
                ),
            );
            true
        } else {
            crate::logger::warn(
                "app",
                format!("de-elevation failed (err={err}); continuing elevated"),
            );
            false
        }
    }
}
