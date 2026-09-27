//! Byte-mode named-pipe client and server.
//!
//! Each connection carries exactly one newline-terminated event line and is then
//! torn down. [`EventServer::bind`] claims the name by creating the first pipe
//! instance, and every later [`EventServer::receive_one`] creates a fresh one, so
//! a stalled or hostile writer occupies one instance and nothing else.

use std::os::windows::ffi::OsStrExt;
use std::time::{Duration, Instant};

use termielle_core::MAX_EVENT_BYTES;
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, ERROR_SEM_TIMEOUT, GENERIC_WRITE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_NONE, OPEN_EXISTING,
    PIPE_ACCESS_INBOUND, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::PCWSTR;

use crate::security::{OwnedHandle, OwnerOnlySecurity};
use crate::{IpcError, from_win32, win32_code};

/// How long a pipe instance waits on a client that has connected but not
/// written, in milliseconds. Only used as `CreateNamedPipeW`'s default timeout.
const DEFAULT_PIPE_TIMEOUT_MS: u32 = 50;

/// How long a client pauses before retrying while the server is between pipe
/// instances. Short enough to be invisible against a 20 ms budget, long enough
/// not to spin a core.
const RETRY_INTERVAL: Duration = Duration::from_millis(1);

/// Stand-in deadline for a timeout so large that `Instant + timeout` would
/// overflow. Effectively "never expires", which is the honest reading of such a
/// budget and, crucially, is not a panic: the emitter's fail-open contract
/// forbids taking down the agent over a bad configuration value.
const FAR_FUTURE: Duration = Duration::from_secs(60 * 60 * 24);

/// One byte past the ceiling, so an overlong line is detected rather than
/// silently truncated at the limit.
const READ_CEILING: usize = MAX_EVENT_BYTES + 1;

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

/// Sends a single event line to a listening overlay.
///
/// The client never blocks longer than its timeout and never waits for the
/// reader to consume the bytes, so a wedged overlay cannot stall an agent.
pub struct EventClient {
    name: Vec<u16>,
    timeout: Duration,
}

impl EventClient {
    pub fn new(name: &str, timeout: Duration) -> Self {
        Self {
            name: wide(name),
            timeout,
        }
    }

    /// Delivers `line` over one connection, then closes it.
    ///
    /// Returns [`IpcError::NotFound`] when no overlay is listening, which
    /// callers treat as a fail-open success.
    pub fn send(&self, line: &[u8]) -> Result<(), IpcError> {
        // Checked before any Win32 call, so an oversized line is rejected the
        // same way whether or not an overlay happens to be running.
        if line.len() > MAX_EVENT_BYTES {
            return Err(IpcError::TooLarge);
        }

        let handle = self.open()?;

        let mut written = 0u32;
        // SAFETY: `handle` is a live pipe handle opened for writing, `line`
        // outlives the call, and `written` is a live out-parameter.
        unsafe { WriteFile(handle.raw(), Some(line), Some(&mut written), None) }
            .map_err(from_win32)?;

        if written as usize != line.len() {
            return Err(IpcError::Os(
                windows::Win32::Foundation::ERROR_WRITE_FAULT.0,
            ));
        }

        // Deliberately no `FlushFileBuffers`: it would block until the overlay
        // drained the pipe, which is exactly the agent-visible latency this
        // transport exists to avoid. Closing the handle is enough for the
        // server to see the bytes and then EOF.
        Ok(())
    }

    /// Waits for a free instance and opens it for writing.
    ///
    /// No `WaitNamedPipeW` gate: a server blocked in `ConnectNamedPipe` — the
    /// overlay's steady state — keeps its instance "connecting", and the wait
    /// call does not consider a connecting instance available, so it would burn
    /// the whole budget on a pipe that would happily accept us. `CreateFileW`
    /// rendezvouses directly: it succeeds against both a listening instance
    /// (the server then sees `ERROR_PIPE_CONNECTED` from its side) and one
    /// waiting in `ConnectNamedPipe`. The retry loop only covers the windows
    /// where the name does not exist yet (the server creates one instance per
    /// event) or the instance is mid-flight, and it never exceeds the caller's
    /// budget.
    fn open(&self) -> Result<OwnedHandle, IpcError> {
        let now = Instant::now();
        // A `Duration` large enough to overflow the clock would panic here, and
        // an emitter that panics is exactly what the fail-open contract rules
        // out. Such a budget means "wait indefinitely", so a far-future deadline
        // is the faithful substitute.
        let deadline = now
            .checked_add(self.timeout)
            .unwrap_or_else(|| now + FAR_FUTURE);

        loop {
            let expired = deadline.saturating_duration_since(Instant::now()).is_zero();

            match self.try_open() {
                Err(IpcError::NotFound | IpcError::Busy | IpcError::Timeout) if !expired => {
                    // Yield rather than spin: the server is between instances.
                    std::thread::sleep(RETRY_INTERVAL);
                }
                outcome => return outcome,
            }
        }
    }

    /// One open attempt against the current instance, classified for the retry
    /// loop in [`EventClient::open`].
    fn try_open(&self) -> Result<OwnedHandle, IpcError> {
        // SAFETY: `self.name` is NUL-terminated and outlives the call; the
        // returned handle is immediately wrapped for RAII cleanup.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(self.name.as_ptr()),
                GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                // A named-pipe server can call `ImpersonateNamedPipeClient` and
                // act as whoever connected. Without an explicit quality of
                // service the default is SecurityImpersonation, which would let
                // a process that won the name race borrow this agent's token.
                // SECURITY_SQOS_PRESENT makes the request explicit and
                // SECURITY_IDENTIFICATION caps it: the server can learn who we
                // are, but cannot act as us.
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };

        match handle {
            Ok(handle) => Ok(OwnedHandle::new(handle)),
            // The instance we were told about was taken between the wait and
            // the open; both are the caller's "try again shortly" case.
            Err(error) => Err(match win32_code(&error) {
                code if code == ERROR_FILE_NOT_FOUND.0 => IpcError::NotFound,
                code if code == ERROR_PIPE_BUSY.0 => IpcError::Busy,
                code if code == ERROR_SEM_TIMEOUT.0 => IpcError::Timeout,
                code => IpcError::Os(code),
            }),
        }
    }
}

/// Listens for single-event connections on an owner-only pipe.
///
/// [`EventServer::bind`] claims the name, and each `receive_one` serves and
/// destroys one instance, so an abandoned connection is never reused.
pub struct EventServer {
    name: Vec<u16>,
    security: OwnerOnlySecurity,
    /// The instance created by `bind` to claim the name, held until the first
    /// `receive_one` consumes it. `None` afterwards.
    claimed: std::cell::Cell<Option<OwnedHandle>>,
}

impl EventServer {
    /// Claims `name` and prepares to listen on it.
    ///
    /// Two things must fail here rather than later. The owner-only descriptor is
    /// built first, so a failure to read the user SID surfaces at startup. Then
    /// the *first* pipe instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`,
    /// which fails if any other process already holds the name: without this,
    /// a squatter that created the name first would quietly receive every event
    /// while our own create failed later with an opaque Win32 code.
    ///
    /// Returns [`IpcError::PipeNameOwned`] when the name is already taken. The
    /// instance created here serves the first [`EventServer::receive_one`].
    pub fn bind(name: &str) -> Result<Self, IpcError> {
        let name = wide(name);
        let security = OwnerOnlySecurity::current_user()?;

        let claimed = create_instance(&name, &security, true).map_err(|error| match error {
            // `FILE_FLAG_FIRST_PIPE_INSTANCE` reports an existing name as
            // access-denied. Reclassify it so the app can say "another overlay
            // owns the pipe" instead of surfacing a bare error 5.
            IpcError::Os(code) if code == ERROR_ACCESS_DENIED.0 => IpcError::PipeNameOwned,
            other => other,
        })?;

        Ok(Self {
            name,
            security,
            claimed: std::cell::Cell::new(Some(claimed)),
        })
    }

    /// Accepts one connection and returns the bytes it carried.
    ///
    /// The returned buffer is the line exactly as sent, including its trailing
    /// newline when the writer supplied one. Anything longer than
    /// [`MAX_EVENT_BYTES`] is rejected as [`IpcError::TooLarge`] without being
    /// returned to the caller.
    pub fn receive_one(&self) -> Result<Vec<u8>, IpcError> {
        let instance = self.next_instance()?;
        let connected = match self.accept(&instance) {
            // A closed writer can still have buffered bytes. Disconnecting
            // here discards them; drain the completed connection first.
            Err(IpcError::Os(code)) if code == ERROR_NO_DATA.0 => Ok(()),
            result => result,
        };

        let result = connected.and_then(|()| read_line(&instance));

        // SAFETY: `instance` is a live pipe handle owned by this scope. A
        // disconnect failure only means there was nothing to disconnect, so it
        // must not mask the read outcome.
        unsafe {
            let _ = DisconnectNamedPipe(instance.raw());
        }

        result
    }

    /// The instance `bind` claimed the name with, or a fresh one after that.
    fn next_instance(&self) -> Result<OwnedHandle, IpcError> {
        match self.claimed.take() {
            Some(instance) => Ok(instance),
            // Later instances must not repeat `FILE_FLAG_FIRST_PIPE_INSTANCE`
            // or they would collide with the name *we* now own.
            None => create_instance(&self.name, &self.security, false),
        }
    }

    /// Blocks until a client is attached to `instance`.
    fn accept(&self, instance: &OwnedHandle) -> Result<(), IpcError> {
        // SAFETY: `instance` is a live pipe handle owned by the caller, and a
        // null OVERLAPPED requests the synchronous form.
        match unsafe { ConnectNamedPipe(instance.raw(), None) } {
            Ok(()) => Ok(()),
            // The client won the race and connected between creation and this
            // call. The pipe is connected either way.
            Err(error) if win32_code(&error) == ERROR_PIPE_CONNECTED.0 => Ok(()),
            Err(error) => Err(from_win32(error)),
        }
    }
}

/// Creates one pipe instance, asserting ownership of the name when `first`.
fn create_instance(
    name: &[u16],
    security: &OwnerOnlySecurity,
    first: bool,
) -> Result<OwnedHandle, IpcError> {
    let mut open_mode = PIPE_ACCESS_INBOUND;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }

    let attributes = security.attributes();

    // SAFETY: `name` is NUL-terminated and outlives the call, and `attributes`
    // borrows `security`, which outlives it too, so the descriptor pointer is
    // live for the duration. The returned handle is checked and wrapped for
    // RAII cleanup.
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            0,
            MAX_EVENT_BYTES as u32,
            DEFAULT_PIPE_TIMEOUT_MS,
            Some(attributes.as_ptr()),
        )
    };

    if handle == INVALID_HANDLE_VALUE || handle.is_invalid() {
        // SAFETY: reads the calling thread's last-error, set by the failed call
        // immediately above.
        let code = unsafe { windows::Win32::Foundation::GetLastError() };
        return Err(IpcError::Os(code.0));
    }

    Ok(OwnedHandle::new(handle))
}

/// Reads until a newline, EOF, or one byte past the protocol ceiling.
fn read_line(instance: &OwnedHandle) -> Result<Vec<u8>, IpcError> {
    let mut line: Vec<u8> = Vec::with_capacity(READ_CEILING);
    let mut chunk = [0u8; 256];

    loop {
        let mut read = 0u32;
        // SAFETY: `instance` is a live pipe handle opened for reading, `chunk`
        // is a live buffer for the duration of the call, and `read` is a live
        // out-parameter.
        let outcome = unsafe { ReadFile(instance.raw(), Some(&mut chunk), Some(&mut read), None) };

        match outcome {
            Ok(()) => {}
            // The writer closed its end; treat it as EOF, matching a zero-byte
            // read on a pipe that shut down cleanly.
            Err(error) if win32_code(&error) == ERROR_BROKEN_PIPE.0 => break,
            Err(error) => return Err(from_win32(error)),
        }

        if read == 0 {
            break;
        }

        let received = &chunk[..read as usize];
        let boundary = received
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);

        // Bytes after the newline belong to no message on a one-event-per-
        // connection pipe, so they are dropped rather than merged into it.
        line.extend_from_slice(&received[..boundary.unwrap_or(received.len())]);

        if line.len() > MAX_EVENT_BYTES {
            return Err(IpcError::TooLarge);
        }

        if boundary.is_some() {
            break;
        }
    }

    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app binds on the main thread, so a SID or name-ownership failure is
    /// reported at startup, then moves the server onto the pipe thread. That
    /// move is only possible if `EventServer` is `Send`; the raw pointers inside
    /// the descriptor and the pipe handle make it a hand-written guarantee
    /// rather than an inferred one, so it is pinned here.
    #[test]
    fn the_server_can_be_moved_to_a_pipe_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<EventServer>();

        let name = format!(r"\\.\pipe\termielle-send-{}", std::process::id());
        let server = EventServer::bind(&name).expect("bind");
        // A compile-time bound alone would be satisfied by a type nobody ever
        // moves; actually moving one across a thread boundary is the behavior
        // the app depends on.
        std::thread::spawn(move || drop(server))
            .join()
            .expect("server moved to another thread");
    }

    /// Proves the client's quality-of-service actually caps what a server can
    /// do with its identity.
    ///
    /// A named-pipe server can call `ImpersonateNamedPipeClient` and then run as
    /// whoever connected. With no explicit QoS the default is
    /// SecurityImpersonation, which would let a process that squatted the name
    /// act as the agent. This test plays the hostile server: it impersonates a
    /// real `EventClient` and reads back the level it was granted, so a
    /// regression that drops the flags fails here rather than in the field.
    #[test]
    fn a_server_can_only_identify_the_client_never_act_as_it() {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Security::{
            GetTokenInformation, SECURITY_IMPERSONATION_LEVEL, SecurityIdentification, TOKEN_QUERY,
            TokenImpersonationLevel,
        };
        use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
        use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};

        let name = format!(r"\\.\pipe\termielle-impersonate-{}", std::process::id());
        let server = EventServer::bind(&name).expect("bind");

        let client_name = name.clone();
        let client = std::thread::spawn(move || {
            EventClient::new(&client_name, Duration::from_secs(5)).send(b"{}\n")
        });

        let instance = server.next_instance().expect("instance");
        server.accept(&instance).expect("accept");
        // Read first: the impersonation context is only guaranteed to be
        // available once the server has consumed the client's request.
        let line = read_line(&instance).expect("read");
        assert_eq!(line, b"{}\n");

        // SAFETY: `instance` is a live, connected pipe instance owned by this
        // scope. On success the calling thread carries an impersonation token
        // until the `RevertToSelf` below, which every path reaches.
        unsafe { ImpersonateNamedPipeClient(instance.raw()) }.expect("impersonate");

        let mut token = HANDLE::default();
        // SAFETY: `GetCurrentThread` returns a pseudo-handle needing no cleanup,
        // and `token` is a live out-parameter receiving the impersonation token
        // installed immediately above.
        let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) };
        let token = OwnedHandle::new(token);

        let mut level = SECURITY_IMPERSONATION_LEVEL::default();
        let mut needed = 0u32;
        // SAFETY: `token` is the live impersonation token just opened, and
        // `level` is a live out-parameter exactly as long as the
        // `TokenImpersonationLevel` class returns.
        let queried = unsafe {
            GetTokenInformation(
                token.raw(),
                TokenImpersonationLevel,
                Some((&raw mut level).cast()),
                size_of::<SECURITY_IMPERSONATION_LEVEL>() as u32,
                &mut needed,
            )
        };

        // SAFETY: drops the impersonation token installed above, restoring the
        // thread's own identity. Run before any assertion so a failure cannot
        // leave the test thread impersonating.
        unsafe { windows::Win32::Security::RevertToSelf() }.expect("revert");
        drop(token);

        // SAFETY: `instance` is live and owned here; a disconnect failure only
        // means there was nothing attached.
        unsafe {
            let _ = DisconnectNamedPipe(instance.raw());
        }
        client.join().expect("client thread").expect("send");

        opened.expect("open thread token");
        queried.expect("query impersonation level");
        assert_eq!(
            level, SecurityIdentification,
            "a server must not be able to act as the client; \
             got level {} (2 = SecurityImpersonation)",
            level.0
        );
    }

    /// Proves the owner-only descriptor actually reaches the kernel object,
    /// rather than being built correctly and then silently dropped.
    ///
    /// The exact access mask is not asserted: the kernel expands the SDDL's
    /// generic `GA` into object-specific rights, so the stored value is a
    /// mapping detail. What matters for the isolation boundary is that the DACL
    /// is protected (no inherited entries can widen it) and grants to exactly
    /// one trustee, this user.
    #[test]
    fn the_pipe_instance_carries_an_owner_only_dacl() {
        use windows::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
        };
        use windows::Win32::Security::{
            ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR,
        };
        use windows::core::PWSTR;

        let name = format!(r"\\.\pipe\termielle-dacl-{}", std::process::id());
        let server = EventServer::bind(&name).expect("bind");
        // The instance `bind` claimed the name with: the very handle a client
        // would connect to, so its DACL is the one that matters.
        let instance = server.next_instance().expect("claimed instance");

        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `instance` is a live kernel handle, and both out-parameters
        // are live. On success the callee allocates a descriptor that owns the
        // ACL memory; it is freed through `LocalFree` below.
        let status = unsafe {
            GetSecurityInfo(
                instance.raw(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                Some(&mut descriptor),
            )
        };
        assert_eq!(status.0, 0, "GetSecurityInfo failed");
        assert!(
            !dacl.is_null(),
            "pipe has a null DACL: access is unrestricted"
        );

        // SAFETY: `dacl` points into the descriptor allocated above, which is
        // still alive, and `GetSecurityInfo` guarantees a well-formed ACL.
        let ace_count = unsafe { (*dacl).AceCount };

        let mut rendered = PWSTR::null();
        // SAFETY: `descriptor` is the live allocation from `GetSecurityInfo`,
        // and `rendered` is a live out-parameter receiving local memory.
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                windows::Win32::Security::Authorization::SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                &mut rendered,
                None,
            )
        }
        .expect("render descriptor");
        // SAFETY: `rendered` is the NUL-terminated string just produced, still
        // owned by this scope.
        let sddl = unsafe { rendered.to_string() }.expect("descriptor is UTF-16");

        let (_sid_buffer, sid) = crate::security::current_user_sid().expect("current user SID");

        assert_eq!(ace_count, 1, "expected exactly one ACE, got {sddl}");
        assert!(
            sddl.contains("D:P"),
            "DACL is not protected, so inherited entries could widen it: {sddl}"
        );

        let mut ace_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `dacl` is the live ACL from `GetSecurityInfo` above, and
        // `ace_ptr` is a live out-parameter receiving a pointer into it.
        unsafe { windows::Win32::Security::GetAce(dacl, 0, &mut ace_ptr) }.expect("GetAce failed");

        // SAFETY: `ace_ptr` names an `ACCESS_ALLOWED_ACE` inside `dacl`, which
        // is still alive, and `sid` points into `sid_buffer`, also alive.
        let ace = unsafe { &*(ace_ptr as *const windows::Win32::Security::ACCESS_ALLOWED_ACE) };
        // `SidStart` is the ACE's inline SID; its `u32` field models the
        // flexible-array member at that offset.
        let ace_sid =
            windows::Win32::Security::PSID((&ace.SidStart as *const u32) as *mut core::ffi::c_void);
        assert!(
            unsafe { windows::Win32::Security::EqualSid(ace_sid, sid) }.is_ok(),
            "DACL does not name the current user: {sddl}"
        );
        // SAFETY: both allocations came from Win32 and are released once, here.
        unsafe {
            let _ = windows::Win32::Foundation::LocalFree(Some(
                windows::Win32::Foundation::HLOCAL(rendered.0.cast()),
            ));
            let _ = windows::Win32::Foundation::LocalFree(Some(
                windows::Win32::Foundation::HLOCAL(descriptor.0),
            ));
        }
    }
}
