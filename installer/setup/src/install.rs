//! What the setup does: install for the current user (no administrator
//! rights), update in place, and uninstall.
//!
//! Installed state:
//! - `%LOCALAPPDATA%\Programs\Simple Screen Grabber\Simple Screen Grabber.exe`,
//!   and `uninstall.exe` (a copy of this setup) next to it;
//! - a Start menu shortcut;
//! - the entry in Settings > Apps
//!   (`HKCU\...\Uninstall\SimpleScreenGrabber`);
//! - optionally the Run value that starts the app with Windows, the same
//!   value the app's tray menu turns on and off.

use std::path::{Path, PathBuf};

use windows::core::{w, Error, Interface, Result, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use windows::Win32::System::ApplicationInstallationAndServicing::MsiEnumRelatedProductsW;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD,
    REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{
    OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
};
use windows::Win32::UI::Shell::{
    IShellLinkW, SHGetKnownFolderPath, ShellExecuteExW, ShellLink, FOLDERID_LocalAppData,
    FOLDERID_Programs, FOLDERID_RoamingAppData, KF_FLAG_CREATE, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, PostMessageW, SW_SHOWNORMAL, WM_CLOSE,
};

pub const APP_NAME: &str = "Simple Screen Grabber";
const EXE: &str = "Simple Screen Grabber.exe";
/// The executable name of earlier versions, removed on upgrade.
const LEGACY_EXE: &str = "simple-screen-grabber.exe";
const UNINSTALLER: &str = "uninstall.exe";
const PUBLISHER: &str = "Matheus Nascimento";
const URL: &str = "https://github.com/matheusgrnascimento/simple-screen-grabber";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\SimpleScreenGrabber";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "SimpleScreenGrabber";
/// The app's hidden window, to close it before files are replaced.
const HOST_CLASS: PCWSTR = w!("SimpleScreenGrabber.Host");
/// The settings folder under %APPDATA%.
const SETTINGS_FOLDER: &str = "simple-screen-grabber";
/// The upgrade code of the earlier per-machine MSI.
const MSI_UPGRADE_CODE: PCWSTR = w!("{8DC426C3-8611-4857-A530-0935CB614078}");

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn os_error(err: std::io::Error) -> Error {
    match err.raw_os_error() {
        Some(code) => Error::from_hresult(HRESULT::from_win32(code as u32)),
        None => Error::new(HRESULT(-1), err.to_string()),
    }
}

fn known_folder(id: &windows::core::GUID) -> Result<PathBuf> {
    // SAFETY: the returned string is copied, then freed as required.
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_CREATE, None)?;
        let result = path.to_string().map(PathBuf::from);
        CoTaskMemFree(Some(path.0.cast_const().cast()));
        result.map_err(|_| Error::empty())
    }
}

/// Where the app is installed.
pub fn install_folder() -> Result<PathBuf> {
    Ok(known_folder(&FOLDERID_LocalAppData)?.join("Programs").join(APP_NAME))
}

fn shortcut_path() -> Result<PathBuf> {
    Ok(known_folder(&FOLDERID_Programs)?.join(format!("{APP_NAME}.lnk")))
}

fn reg_string(key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the buffer holds `size` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then(|| {
        let chars = (size as usize / 2).saturating_sub(1);
        String::from_utf16_lossy(&buffer[..chars])
    })
}

fn set_string(key: &str, value: &str, data: &str) -> Result<()> {
    let (key, value, data) = (wide(key), wide(value), wide(data));
    // SAFETY: null-terminated strings of the given byte size.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
        .ok()
    }
}

fn set_dword(key: &str, value: &str, data: u32) -> Result<()> {
    let (key, value) = (wide(key), wide(value));
    // SAFETY: a 4-byte value.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            REG_DWORD.0,
            Some((&data as *const u32).cast()),
            4,
        )
        .ok()
    }
}

/// The installed version, if any.
pub fn installed_version() -> Option<String> {
    reg_string(UNINSTALL_KEY, "DisplayVersion")
}

/// Whether the app starts with Windows now (to keep the choice on update).
pub fn autostart_enabled() -> bool {
    reg_string(RUN_KEY, RUN_VALUE).is_some()
}

fn set_autostart(exe: Option<&Path>) -> Result<()> {
    match exe {
        Some(exe) => set_string(RUN_KEY, RUN_VALUE, &format!("\"{}\" --autostart", exe.display())),
        None => {
            let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
            // SAFETY: deletes one value; a missing value is fine.
            let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(value.as_ptr())) };
            Ok(())
        }
    }
}

/// Asks the running app to exit and waits for it (up to 5 s).
pub fn close_running_app() {
    // SAFETY: plain window and process queries; the handle is closed.
    unsafe {
        let Ok(hwnd) = FindWindowW(HOST_CLASS, PCWSTR::null()) else {
            return;
        };
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok();
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        if let Some(process) = process {
            WaitForSingleObject(process, 5000);
            let _ = CloseHandle(process);
        }
    }
}

/// Removes the earlier per-machine MSI install, if present. Windows asks for
/// administrator approval for it.
fn remove_old_msi() -> Result<()> {
    let mut code = [0u16; 39];
    // SAFETY: the buffer holds a product code (38 characters and a null).
    let found = unsafe { MsiEnumRelatedProductsW(MSI_UPGRADE_CODE, None, 0, PWSTR(code.as_mut_ptr())) };
    if found != 0 {
        return Ok(());
    }
    let product = String::from_utf16_lossy(&code[..38]);
    let parameters = wide(&format!("/x {product} /passive /norestart"));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("open"),
        lpFile: w!("msiexec.exe"),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: the strings outlive the call; the process handle is closed.
    unsafe {
        ShellExecuteExW(&mut info)?;
        if !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, INFINITE);
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(())
}

fn create_shortcut(target: &Path, folder: &Path) -> Result<()> {
    let path = shortcut_path()?;
    let (target_w, folder_w, path_w) = (
        wide(&target.to_string_lossy()),
        wide(&folder.to_string_lossy()),
        wide(&path.to_string_lossy()),
    );
    let description = wide("Select an area of the screen, annotate it, save or copy it");
    // SAFETY: COM calls with null-terminated strings that outlive them.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(target_w.as_ptr()))?;
        link.SetWorkingDirectory(PCWSTR(folder_w.as_ptr()))?;
        link.SetDescription(PCWSTR(description.as_ptr()))?;
        link.SetIconLocation(PCWSTR(target_w.as_ptr()), 0)?;
        link.cast::<IPersistFile>()?.Save(PCWSTR(path_w.as_ptr()), true)
    }
}

/// Writes `data` as `path` through a temporary file, replacing it.
fn write_replacing(path: &Path, data: &[u8]) -> Result<()> {
    let temp = path.with_extension("new");
    std::fs::write(&temp, data).map_err(os_error)?;
    let (from, to) = (wide(&temp.to_string_lossy()), wide(&path.to_string_lossy()));
    // SAFETY: null-terminated paths.
    let moved = unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved.is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err(Error::new(
            HRESULT(-1),
            "Simple Screen Grabber is still running. Exit it from the tray icon and try again.",
        ));
    }
    Ok(())
}

/// The choices of an install.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub autostart: bool,
}

/// Installs or updates the app. `progress` gets a fraction and a status.
pub fn install(payload: &[u8], version: &str, options: Options, progress: &mut dyn FnMut(f32, &str)) -> Result<PathBuf> {
    if payload.is_empty() {
        return Err(Error::new(HRESULT(-1), "This setup was built without the app."));
    }
    progress(0.1, "Closing Simple Screen Grabber");
    close_running_app();
    progress(0.2, "Removing the earlier version");
    remove_old_msi()?;
    progress(0.4, "Copying files");
    let folder = install_folder()?;
    std::fs::create_dir_all(&folder).map_err(os_error)?;
    let exe = folder.join(EXE);
    write_replacing(&exe, payload)?;
    let _ = std::fs::remove_file(folder.join(LEGACY_EXE));
    let me = std::env::current_exe().map_err(os_error)?;
    let uninstaller = folder.join(UNINSTALLER);
    if me != uninstaller {
        let bytes = std::fs::read(&me).map_err(os_error)?;
        write_replacing(&uninstaller, &bytes)?;
    }
    progress(0.7, "Adding the shortcut");
    create_shortcut(&exe, &folder)?;
    progress(0.85, "Registering the app");
    let uninstall = format!("\"{}\" --uninstall", uninstaller.display());
    let size_kb = [&exe, &uninstaller]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len() / 1024)
        .sum::<u64>() as u32;
    for (value, data) in [
        ("DisplayName", APP_NAME.to_string()),
        ("DisplayVersion", version.to_string()),
        ("Publisher", PUBLISHER.to_string()),
        ("DisplayIcon", format!("{},0", exe.display())),
        ("InstallLocation", folder.display().to_string()),
        ("UninstallString", uninstall.clone()),
        ("QuietUninstallString", format!("{uninstall} --silent")),
        ("URLInfoAbout", URL.to_string()),
    ] {
        set_string(UNINSTALL_KEY, value, &data)?;
    }
    set_dword(UNINSTALL_KEY, "NoModify", 1)?;
    set_dword(UNINSTALL_KEY, "NoRepair", 1)?;
    set_dword(UNINSTALL_KEY, "EstimatedSize", size_kb)?;
    set_autostart(options.autostart.then_some(exe.as_path()))?;
    progress(1.0, "Done");
    Ok(exe)
}

/// Starts the installed app, as the user who runs the setup.
pub fn launch(exe: &Path) {
    let _ = std::process::Command::new(exe).current_dir(exe.parent().unwrap_or(Path::new("."))).spawn();
}

/// Removes the app. Screenshots always stay; the settings only stay if
/// `keep_settings`.
pub fn uninstall(keep_settings: bool, progress: &mut dyn FnMut(f32, &str)) -> Result<()> {
    progress(0.2, "Closing Simple Screen Grabber");
    close_running_app();
    progress(0.5, "Removing files");
    set_autostart(None)?;
    if let Ok(shortcut) = shortcut_path() {
        let _ = std::fs::remove_file(shortcut);
    }
    let folder = install_folder()?;
    if folder.exists() {
        std::fs::remove_dir_all(&folder).map_err(os_error)?;
    }
    let key = wide(UNINSTALL_KEY);
    // SAFETY: deletes this app's own key.
    let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr())) };
    if !keep_settings {
        if let Ok(appdata) = known_folder(&FOLDERID_RoamingAppData) {
            let _ = std::fs::remove_dir_all(appdata.join(SETTINGS_FOLDER));
        }
    }
    progress(1.0, "Done");
    Ok(())
}

/// The uninstaller lives in the folder it removes: it runs from a copy in
/// the temporary folder instead. Returns true when this process should exit
/// because the copy took over.
pub fn relaunch_from_temp(args: &[String]) -> bool {
    let Ok(me) = std::env::current_exe() else {
        return false;
    };
    let Ok(folder) = install_folder() else {
        return false;
    };
    if !me.starts_with(&folder) {
        return false;
    }
    let copy = std::env::temp_dir().join(format!("SimpleScreenGrabber-uninstall-{}.exe", std::process::id()));
    if std::fs::copy(&me, &copy).is_err() {
        return false;
    }
    std::process::Command::new(&copy).args(args).spawn().is_ok()
}

/// Deletes this temporary uninstaller copy once it exits.
pub fn delete_self_later() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    if !me.starts_with(std::env::temp_dir()) {
        return;
    }
    let _ = std::process::Command::new("cmd.exe")
        .args(["/c", "ping -n 3 127.0.0.1 >nul & del /f /q"])
        .arg(&me)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

/// An HWND the install thread may post to.
pub struct PostTarget(pub HWND);
// SAFETY: PostMessageW accepts a window of another thread.
unsafe impl Send for PostTarget {}
