//! Bounded display recovery independent of the DXGI vertical-blank clock.

use windows::Win32::Foundation::LPARAM;
use windows::Win32::System::Power::POWERBROADCAST_SETTING;
use windows::core::GUID;

/// GUID_SESSION_DISPLAY_STATUS from winnt.h. Session-local display-on
/// notifications also cover lid/display wake on Modern Standby machines.
pub(super) const SESSION_DISPLAY_STATUS: GUID =
    GUID::from_u128(0x2b84c20e_ad23_4ddf_93db_05ffbd7efca5);

const RETRY_DELAYS_MS: [u32; 3] = [250, 1_000, 3_000];

#[derive(Default)]
pub(super) struct DisplayRecovery {
    next: Option<usize>,
}

impl DisplayRecovery {
    pub fn restart(&mut self) {
        self.next = Some(0);
    }

    pub fn next_delay_ms(&mut self) -> Option<u32> {
        let index = self.next?;
        let delay = RETRY_DELAYS_MS.get(index).copied();
        self.next = delay.map(|_| index + 1);
        delay
    }
}

/// # Safety
/// A nonzero `lparam` must point to the live POWERBROADCAST_SETTING supplied
/// by WM_POWERBROADCAST, including its `DataLength` bytes. Never retain it.
pub(super) unsafe fn display_is_on(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    // SAFETY: caller guarantees the Windows message payload is live.
    let setting = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
    if setting.PowerSetting != SESSION_DISPLAY_STATUS || setting.DataLength != 4 {
        return false;
    }
    // Data is a flexible-array member, not an aligned u32 field. Read only
    // after validating both the GUID and the DWORD payload length.
    unsafe { setting.Data.as_ptr().cast::<u32>().read_unaligned() == 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_is_bounded_and_coalesces_a_new_topology_change() {
        let mut recovery = DisplayRecovery::default();
        assert_eq!(recovery.next_delay_ms(), None);
        recovery.restart();
        assert_eq!(recovery.next_delay_ms(), Some(250));
        assert_eq!(recovery.next_delay_ms(), Some(1_000));
        recovery.restart();
        assert_eq!(recovery.next_delay_ms(), Some(250));
        assert_eq!(recovery.next_delay_ms(), Some(1_000));
        assert_eq!(recovery.next_delay_ms(), Some(3_000));
        assert_eq!(recovery.next_delay_ms(), None);
        assert_eq!(recovery.next_delay_ms(), None);
    }

    #[test]
    fn only_a_valid_session_display_on_payload_requests_recovery() {
        #[repr(C)]
        struct Payload {
            guid: GUID,
            length: u32,
            value: u32,
        }
        let mut payload = Payload {
            guid: SESSION_DISPLAY_STATUS,
            length: 4,
            value: 1,
        };
        let check = |payload: &Payload| {
            // SAFETY: repr(C) matches the SDK header plus its DWORD data.
            unsafe { display_is_on(LPARAM(payload as *const Payload as isize)) }
        };
        assert!(check(&payload));
        payload.value = 0;
        assert!(!check(&payload));
        payload.value = 2;
        assert!(!check(&payload));
        payload.value = 1;
        payload.length = 1;
        assert!(!check(&payload));
        payload.length = 4;
        payload.guid = GUID::zeroed();
        assert!(!check(&payload));
        assert!(!unsafe { display_is_on(LPARAM(0)) });
    }
}
