//! Bounded, local-only message pipe I/O. In-flight OVERLAPPED storage is
//! retained until cancellation completes; callers never replay sent commands.
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, LocalFree, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_GROUPS, TOKEN_QUERY,
    TokenLogonSid,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject,
};
use windows::core::{Error, PCWSTR, PWSTR, Result};

pub const TIMEOUT_MS: u32 = 2000;
const BUFFER_SIZE: usize = 4096;
pub struct Pipe(HANDLE);
impl Drop for Pipe {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
struct Allocation(HLOCAL);
impl Drop for Allocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(Some(self.0));
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn security() -> Result<Allocation> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let token = Pipe(token);
        let mut size = 0;
        let _ = GetTokenInformation(token.0, TokenLogonSid, None, 0, &mut size);
        if !(std::mem::size_of::<TOKEN_GROUPS>() as u32..=65536).contains(&size) {
            return Err(Error::from_thread());
        }
        let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            token.0,
            TokenLogonSid,
            Some(buffer.as_mut_ptr().cast()),
            size,
            &mut size,
        )?;
        let groups = &*buffer.as_ptr().cast::<TOKEN_GROUPS>();
        if groups.GroupCount == 0 {
            return Err(Error::from_thread());
        }
        let mut sid = PWSTR::null();
        ConvertSidToStringSidW(groups.Groups[0].Sid, &mut sid)?;
        let _sid_memory = Allocation(HLOCAL(sid.0.cast()));
        let sddl = wide(&format!(
            "D:(A;;GA;;;{})(A;;GA;;;SY)(A;;GA;;;BA)",
            sid.to_string()?
        ));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut descriptor,
            None,
        )?;
        Ok(Allocation(HLOCAL(descriptor.0)))
    }
}

impl Pipe {
    pub fn listen(name: &str) -> Result<Self> {
        let descriptor = security()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0.0,
            bInheritHandle: false.into(),
        };
        let name = wide(name);
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                BUFFER_SIZE as u32,
                BUFFER_SIZE as u32,
                TIMEOUT_MS,
                Some(&attributes),
            )
        };
        if handle.is_invalid() {
            Err(Error::from_thread())
        } else {
            Ok(Self(handle))
        }
    }

    pub fn open(name: &str) -> Result<Self> {
        let name = wide(name);
        unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
            .map(Self)
        }
    }

    fn io(
        &self,
        timeout: u32,
        start: impl FnOnce(*mut OVERLAPPED, &mut u32) -> Result<()>,
    ) -> Result<u32> {
        let event = Pipe(unsafe { CreateEventW(None, true, false, None)? });
        let mut operation = OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        };
        let mut transferred = 0;
        match start(&mut operation, &mut transferred) {
            Ok(()) => return Ok(transferred),
            Err(error) if error.code() == ERROR_PIPE_CONNECTED.to_hresult() => return Ok(0),
            Err(error) if error.code() == ERROR_IO_PENDING.to_hresult() => {}
            Err(error) => return Err(error),
        }
        unsafe {
            if WaitForSingleObject(event.0, timeout) != WAIT_OBJECT_0 {
                let _ = CancelIoEx(self.0, Some(&operation));
                let _ = GetOverlappedResult(self.0, &operation, &mut transferred, true);
                return Err(Error::from_hresult(
                    windows::Win32::Foundation::ERROR_TIMEOUT.to_hresult(),
                ));
            }
            GetOverlappedResult(self.0, &operation, &mut transferred, false)?;
        }
        Ok(transferred)
    }

    pub fn connect(&self) -> Result<()> {
        self.io(TIMEOUT_MS, |operation, _| unsafe {
            ConnectNamedPipe(self.0, Some(operation))
        })
        .map(|_| ())
    }
    pub fn read(&self) -> Result<String> {
        let mut buffer = [0u8; BUFFER_SIZE];
        let count = self.io(TIMEOUT_MS, |operation, transferred| unsafe {
            ReadFile(
                self.0,
                Some(&mut buffer),
                Some(transferred),
                Some(operation),
            )
        })?;
        String::from_utf8(buffer[..count as usize].to_vec())
            .map(|s| s.trim().to_owned())
            .map_err(|_| {
                Error::from_hresult(windows::Win32::Foundation::ERROR_INVALID_DATA.to_hresult())
            })
    }
    pub fn write(&self, text: &str) -> Result<()> {
        let bytes = format!("{text}\r\n");
        if bytes.len() > BUFFER_SIZE {
            return Err(Error::from_hresult(
                windows::Win32::Foundation::ERROR_BUFFER_OVERFLOW.to_hresult(),
            ));
        }
        let count = self.io(TIMEOUT_MS, |operation, transferred| unsafe {
            WriteFile(
                self.0,
                Some(bytes.as_bytes()),
                Some(transferred),
                Some(operation),
            )
        })?;
        if count as usize == bytes.len() {
            Ok(())
        } else {
            Err(Error::from_hresult(
                windows::Win32::Foundation::ERROR_WRITE_FAULT.to_hresult(),
            ))
        }
    }
    pub fn finish(&self) {
        // Wait for the peer to consume the reply and close. Disconnecting
        // immediately after WriteFile can discard a buffered response.
        let _ = self.read();
        self.disconnect();
    }
    pub fn disconnect(&self) {
        let _ = unsafe { DisconnectNamedPipe(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preconnected_client_delayed_reply_read_and_cancelled_io() {
        let name = format!(
            r"\\.\pipe\IdleTrigger-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let server_name = name.clone();
        let (ready, wait_ready) = std::sync::mpsc::channel();
        let (connected, wait_connected) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let pipe = Pipe::listen(&server_name).unwrap();
            assert!(Pipe::listen(&server_name).is_err());
            ready.send(()).unwrap();
            wait_connected.recv().unwrap();
            pipe.connect().unwrap(); // ERROR_PIPE_CONNECTED is a success.
            let mut byte = [0u8; 1];
            let start = std::time::Instant::now();
            assert!(
                pipe.io(50, |operation, transferred| unsafe {
                    ReadFile(pipe.0, Some(&mut byte), Some(transferred), Some(operation))
                })
                .is_err()
            );
            assert!(start.elapsed() < std::time::Duration::from_secs(1));
            ready.send(()).unwrap();
            // The cancelled operation cannot consume the next request.
            assert_eq!(pipe.read().unwrap(), "toggle");
            pipe.write("ok once").unwrap();
            pipe.finish();
            // Reuse the original server handle, retaining first-instance ownership.
            ready.send(()).unwrap();
            pipe.connect().unwrap();
            assert_eq!(pipe.read().unwrap(), "second");
            pipe.write("ok twice").unwrap();
            pipe.finish();
        });
        wait_ready.recv().unwrap();
        let client = Pipe::open(&name).unwrap();
        connected.send(()).unwrap();
        wait_ready.recv().unwrap();
        client.write("toggle").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert_eq!(client.read().unwrap(), "ok once");
        drop(client);
        wait_ready.recv().unwrap();
        let client = Pipe::open(&name).unwrap();
        client.write("second").unwrap();
        assert_eq!(client.read().unwrap(), "ok twice");
        drop(client);
        server.join().unwrap();
    }
}
