//! Out-of-process Shell verb handler, hosted by the same portable EXE.
//! Each invocation owns its selection; there is no timing-based aggregation.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, POINT};
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::{
    IExecuteCommand, IExecuteCommand_Impl, IObjectWithSelection, IObjectWithSelection_Impl,
    IShellItemArray, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, GUID, IUnknown, Interface, PCWSTR, Ref, Result, implement};

// Stable, application-owned CLSIDs. LocalServer32 works across bitness;
// no DLL is loaded into Explorer and no service is installed.
pub(crate) const FORMATS: [&str; 6] = ["full", "target", "dir", "name", "stem", "ext"];
pub(crate) fn clsid(index: usize) -> GUID {
    GUID::from_u128(0x3751ee7b_2c46_4e8a_9e65_e03a29d36810 + index as u128)
}
pub(crate) fn class_name(index: usize) -> String {
    format!("{{{:?}}}", clsid(index))
}

thread_local! {
    static STARTUP: Cell<bool> = const { Cell::new(true) };
}

fn release_server() {
    unsafe {
        // COM atomically suspends activation when the last reference goes.
        if CoReleaseServerProcess() == 0 {
            PostQuitMessage(0);
        }
    }
}

fn release_startup() {
    STARTUP.with(|startup| {
        if startup.replace(false) {
            release_server();
        }
    });
}

// Keep Shell proxies, RefCell state and server lifetime on the STA thread.
// windows-rs otherwise supplies the free-threaded marshaler by default.
#[implement(IExecuteCommand, IObjectWithSelection, Agile = false)]
struct CopyCommand {
    format: &'static str,
    selection: RefCell<Option<IShellItemArray>>,
}

impl Drop for CopyCommand {
    fn drop(&mut self) {
        // Release proxies before the last server reference.
        self.selection.get_mut().take();
        release_server();
    }
}

impl IObjectWithSelection_Impl for CopyCommand_Impl {
    fn SetSelection(&self, selection: Ref<IShellItemArray>) -> Result<()> {
        *self.selection.borrow_mut() = selection.cloned();
        Ok(())
    }

    fn GetSelection(&self, iid: *const GUID, result: *mut *mut c_void) -> Result<()> {
        if result.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *result = std::ptr::null_mut() };
        let selection = self.selection.borrow().clone().ok_or(E_FAIL)?;
        unsafe { selection.query(iid, result).ok() }
    }
}

impl IExecuteCommand_Impl for CopyCommand_Impl {
    fn SetKeyState(&self, _: u32) -> Result<()> {
        Ok(())
    }
    fn SetParameters(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn SetPosition(&self, _: &POINT) -> Result<()> {
        Ok(())
    }
    fn SetShowWindow(&self, _: i32) -> Result<()> {
        Ok(())
    }
    fn SetNoShowUI(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
    fn SetDirectory(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn Execute(&self) -> Result<()> {
        let selection = self.selection.borrow().clone().ok_or(E_FAIL)?;
        let paths = selection_paths(&selection)?;
        if crate::ctxexec::copy_selection(self.format, &paths) {
            Ok(())
        } else {
            Err(E_FAIL.into())
        }
    }
}

fn selection_paths(selection: &IShellItemArray) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    unsafe {
        let count = selection.GetCount()?;
        if count == 0 {
            return Err(E_FAIL.into());
        }
        for index in 0..count {
            let item = selection.GetItemAt(index)?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0.cast()));
            // Abort before touching the clipboard if even one item fails.
            paths.push(path.map_err(|_| windows::core::Error::from(E_FAIL))?);
        }
    }
    Ok(paths)
}

#[implement(IClassFactory, Agile = false)]
struct Factory {
    format: &'static str,
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> Result<()> {
        if result.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *result = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        unsafe {
            CoAddRefServerProcess();
        }
        let command: IUnknown = CopyCommand {
            format: self.format,
            selection: RefCell::new(None),
        }
        .into();
        release_startup();
        unsafe { command.query(iid, result).ok() }
    }
    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            unsafe {
                CoAddRefServerProcess();
            }
        } else {
            release_server();
        }
        Ok(())
    }
}

pub(crate) fn run() -> i32 {
    unsafe {
        if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
            return 1;
        }
        CoAddRefServerProcess(); // Activation has 30 seconds to arrive.
        let mut cookies = Vec::new();
        let result = (|| -> Result<()> {
            for (index, format) in FORMATS.iter().enumerate() {
                let factory: IClassFactory = Factory { format }.into();
                cookies.push(CoRegisterClassObject(
                    &clsid(index),
                    &factory,
                    CLSCTX_LOCAL_SERVER,
                    REGCLS_MULTIPLEUSE | REGCLS_SUSPENDED,
                )?);
            }
            CoResumeClassObjects()?;
            let started = std::time::Instant::now();
            loop {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    if message.message == WM_QUIT {
                        return Ok(());
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                if started.elapsed().as_secs() >= 30 {
                    release_startup();
                }
                MsgWaitForMultipleObjectsEx(None, 250, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            }
        })();
        let _ = CoSuspendClassObjects();
        for cookie in cookies {
            let _ = CoRevokeClassObject(cookie);
        }
        release_startup();
        CoUninitialize();
        i32::from(result.is_err())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Shell::{SHCreateShellItemArrayFromIDLists, SHParseDisplayName};

    #[test]
    fn shell_selection_retains_all_items_order_and_unicode() {
        // Real Shell arrays exercise the native boundary without changing
        // registration, launching Explorer or touching the clipboard.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            struct Apartment;
            impl Drop for Apartment {
                fn drop(&mut self) {
                    unsafe { CoUninitialize() };
                }
            }
            let _apartment = Apartment;
            struct Fixture(std::path::PathBuf);
            impl Drop for Fixture {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let fixture = Fixture(std::env::temp_dir().join(format!(
                    "IdleTrigger-selection-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                )));
            std::fs::create_dir(&fixture.0).unwrap();
            let file = fixture.0.join("中文 空格 % &.txt");
            let folder = fixture.0.join("folder.with.dots");
            std::fs::write(&file, b"selection fixture").unwrap();
            std::fs::create_dir(&folder).unwrap();
            let expected = [folder, file].map(|path| path.to_str().unwrap().to_owned());
            let mut pidls = Vec::new();
            for path in &expected {
                let mut pidl = std::ptr::null_mut();
                SHParseDisplayName(
                    &windows::core::HSTRING::from(path),
                    None,
                    &mut pidl,
                    0,
                    None,
                )
                .unwrap();
                pidls.push(pidl);
            }
            let pointers: Vec<_> = pidls.iter().map(|pidl| *pidl as *const _).collect();
            let selection = SHCreateShellItemArrayFromIDLists(&pointers);
            for pidl in pidls {
                CoTaskMemFree(Some(pidl.cast()));
            }
            let selection = selection.unwrap();
            // CI may expose TEMP through an 8.3 alias (RUNNER~1), while
            // Shell returns long paths. Compare canonical paths without
            // weakening item count, ordering or Unicode file identity.
            let canonical = |paths: &[String]| {
                paths
                    .iter()
                    .map(|path| std::fs::canonicalize(path).unwrap())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                canonical(&selection_paths(&selection).unwrap()),
                canonical(&expected)
            );
        }
    }
}
