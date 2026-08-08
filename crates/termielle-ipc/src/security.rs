//! Owner-only security descriptor for the event pipe.
//!
//! The descriptor is built from the current process token's user SID, so only
//! the account that started the overlay can open the pipe. This is the isolation
//! boundary the design relies on: no network exposure, no other local user, and
//! no elevated-to-unelevated crossing.

use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{PCWSTR, PWSTR};

use crate::{IpcError, from_win32, win32_code};

/// A `HANDLE` that is closed exactly once, when it goes out of scope.
pub(crate) struct OwnedHandle(HANDLE);

impl OwnedHandle {
    pub(crate) fn new(handle: HANDLE) -> Self {
        Self(handle)
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: `self.0` is a live kernel handle this type exclusively
            // owns and has not closed, and `Drop` runs at most once.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

// SAFETY: a Win32 `HANDLE` names a kernel object in the *process* handle table,
// not a thread-local resource, and `CloseHandle` may be called from any thread.
// `OwnedHandle` is neither `Clone` nor `Copy` and never hands out an owning
// copy, so exactly one owner exists and it is the one being moved. Only `Send`
// is asserted; the type is left `!Sync`, matching the single-threaded use of a
// synchronous pipe instance.
unsafe impl Send for OwnedHandle {}

/// A `LocalAlloc`-backed pointer released with `LocalFree` on drop.
///
/// `ConvertSidToStringSidW` and `ConvertStringSecurityDescriptorToSecurityDescriptorW`
/// both hand back local memory the caller must free; this makes that automatic
/// on every path, including early returns.
struct LocalBuffer(*mut core::ffi::c_void);

impl Drop for LocalBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` came from a Win32 `Local*` allocation this type
            // exclusively owns, and `Drop` runs at most once.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0)));
            }
        }
    }
}

/// An owner-only security descriptor plus the `SECURITY_ATTRIBUTES` that points
/// at it.
///
/// The descriptor allocation is owned here, and [`OwnerOnlySecurity::attributes`]
/// hands out only a [`BorrowedSecurityAttributes`], which carries a lifetime tied
/// to `&self`. That is what keeps the raw pointer inside the attributes from
/// outliving the allocation it points into.
pub(crate) struct OwnerOnlySecurity {
    descriptor: LocalBuffer,
}

// SAFETY: `OwnerOnlySecurity` owns exactly one `LocalAlloc` block, reached only
// through the `LocalBuffer` field. The pointer is never handed out as an owning
// copy and never aliased: `attributes()` only lends it inside a value borrowing
// `&self`, so no second owner can exist on any thread. `LocalFree` is documented
// as callable from any thread for memory from the process heap, so moving the
// value (and therefore its eventual `Drop`) to another thread is sound. The type
// is deliberately not `Sync`-by-hand: only `Send` is asserted here, which is all
// the app needs to bind on the main thread and move the server to a pipe thread.
unsafe impl Send for OwnerOnlySecurity {}

/// `SECURITY_ATTRIBUTES` that cannot outlive the descriptor they point into.
///
/// `SECURITY_ATTRIBUTES` is `Copy` and holds a bare `*mut c_void`, so returning
/// one by value would let a caller keep it after the descriptor was freed. The
/// `PhantomData<&'a OwnerOnlySecurity>` is what makes that a borrow-checker
/// error instead of a use-after-free.
pub(crate) struct BorrowedSecurityAttributes<'a> {
    raw: SECURITY_ATTRIBUTES,
    _owner: PhantomData<&'a OwnerOnlySecurity>,
}

impl BorrowedSecurityAttributes<'_> {
    /// A pointer valid for `'a`, suitable for a Win32 `lpSecurityAttributes`.
    pub(crate) fn as_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        &raw const self.raw
    }
}

impl OwnerOnlySecurity {
    /// Builds `D:P(A;;GA;;;<current user SID>)`: a protected DACL granting
    /// generic-all to this user alone, with no inherited entries.
    pub(crate) fn current_user() -> Result<Self, IpcError> {
        let sid = current_user_sid_string()?;
        let sddl = format!("D:P(A;;GA;;;{sid})");
        let wide: Vec<u16> = std::ffi::OsStr::new(&sddl)
            .encode_wide()
            .chain(Some(0))
            .collect();

        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `wide` is NUL-terminated and outlives the call, and
        // `descriptor` is a live out-parameter. On success the callee stores a
        // `LocalAlloc` pointer that `LocalBuffer` then owns and frees.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(from_win32)?;

        Ok(Self {
            descriptor: LocalBuffer(descriptor.0),
        })
    }

    /// Attributes referring to the descriptor owned by `self`.
    ///
    /// The returned value borrows `self`, so the descriptor pointer it carries
    /// cannot be observed after the allocation is freed: returning it from a
    /// function that owns the `OwnerOnlySecurity` is a compile error.
    pub(crate) fn attributes(&self) -> BorrowedSecurityAttributes<'_> {
        BorrowedSecurityAttributes {
            raw: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: self.descriptor.0,
                bInheritHandle: false.into(),
            },
            _owner: PhantomData,
        }
    }
}

/// Reads this process's user SID, kept alive by the returned buffer.
///
/// The SID points into the `TOKEN_USER` buffer the query filled, so the pair
/// must stay together: dropping the buffer invalidates the SID.
pub(crate) fn current_user_sid() -> Result<(Vec<u8>, PSID), IpcError> {
    let mut token = HANDLE::default();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no
    // cleanup, and `token` is a live out-parameter.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(from_win32)?;
    let token = OwnedHandle::new(token);

    let mut needed = 0u32;
    // SAFETY: passing a null buffer with length 0 is the documented way to ask
    // for the required size; `needed` is a live out-parameter. The call is
    // expected to fail with ERROR_INSUFFICIENT_BUFFER.
    let probe = unsafe { GetTokenInformation(token.raw(), TokenUser, None, 0, &mut needed) };
    match probe {
        Err(error) if win32_code(&error) == ERROR_INSUFFICIENT_BUFFER.0 => {}
        Err(error) => return Err(from_win32(error)),
        // A zero-length TOKEN_USER is impossible; treat an unexpected success
        // as a malformed token rather than reading from a buffer we never got.
        Ok(()) => return Err(IpcError::Os(ERROR_INSUFFICIENT_BUFFER.0)),
    }

    let mut buffer = vec![0u8; needed as usize];
    // SAFETY: `buffer` is `needed` bytes long, matching the size the probe
    // asked for, and stays alive across the call.
    unsafe {
        GetTokenInformation(
            token.raw(),
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    }
    .map_err(from_win32)?;

    if buffer.len() < size_of::<TOKEN_USER>() {
        return Err(IpcError::Os(ERROR_INSUFFICIENT_BUFFER.0));
    }

    // SAFETY: the buffer was filled by `GetTokenInformation(TokenUser)`, so it
    // begins with a `TOKEN_USER`, and the length check above confirms it is
    // large enough to read one. The SID it points at lives inside `buffer`.
    let sid = unsafe { buffer.as_ptr().cast::<TOKEN_USER>().read_unaligned() }
        .User
        .Sid;

    Ok((buffer, sid))
}

/// Reads this process's user SID and renders it in string form.
pub(crate) fn current_user_sid_string() -> Result<String, IpcError> {
    let (_buffer, sid) = current_user_sid()?;

    let mut text = PWSTR::null();
    // SAFETY: `sid` points into `_buffer`, which is still alive, and `text` is
    // a live out-parameter that receives a `LocalAlloc` string.
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(from_win32)?;
    let owned = LocalBuffer(text.0.cast());

    // SAFETY: `text` is a NUL-terminated string the previous call produced, and
    // `owned` keeps it alive until after this read.
    let rendered = unsafe { text.to_string() };
    drop(owned);

    rendered.map_err(|_| IpcError::Os(windows::Win32::Foundation::ERROR_INVALID_DATA.0))
}
