//! Isolated hidden native controls; no real profile writes or shell actions.
use termielle_app::{
    preferences::{PreferencesWindow, model::Request},
    window::OverlayWindow,
};
use termielle_core::AppConfig;
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        GetClientRect, GetDlgItem, GetWindowRect, IsWindowVisible, SendMessageW, WM_CLOSE,
        WM_COMMAND,
    },
};
#[test]
fn preferences_are_native_hidden_controls_and_requests_are_not_file_writes() {
    std::thread::spawn(|| {
        let config = AppConfig::default();
        let owner = OverlayWindow::create(&config, true).unwrap();
        let preferences =
            PreferencesWindow::create(owner.hwnd(), owner.wake_handle(), &config.island).unwrap();
        assert!(!unsafe { IsWindowVisible(preferences.hwnd()) }.as_bool());
        for id in [
            101, 102, 103, 104, 105, 106, 107, 108, 111, 112, 120, 130, 131, 132, 133, 155,
        ] {
            assert!(
                !unsafe { GetDlgItem(Some(preferences.hwnd()), id) }
                    .unwrap()
                    .0
                    .is_null()
            );
        }
        let mut text = [0u16; 128];
        let layout = unsafe { GetDlgItem(Some(preferences.hwnd()), 102) }.unwrap();
        let length =
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(layout, &mut text) };
        assert_eq!(
            String::from_utf16_lossy(&text[..length as usize]),
            "Bar",
            "native combo selection must be populated even if hidden WM_PRINT omits it"
        );
        unsafe {
            SendMessageW(
                preferences.hwnd(),
                WM_COMMAND,
                Some(WPARAM(130)),
                Some(LPARAM(0)),
            )
        };
        match preferences.take_request().unwrap() {
            Request::Preview(draft) => assert_eq!(*draft, config.island),
            _ => panic!("preview must be separate from apply"),
        }
        unsafe {
            SendMessageW(
                preferences.hwnd(),
                WM_COMMAND,
                Some(WPARAM(131)),
                Some(LPARAM(0)),
            )
        };
        match preferences.take_request().unwrap() {
            Request::Apply { baseline, draft } => {
                assert_eq!(*baseline, config.island);
                assert_eq!(*draft, config.island);
            }
            _ => panic!("apply request expected"),
        }
        unsafe { SendMessageW(preferences.hwnd(), WM_CLOSE, None, None) };
        assert!(matches!(preferences.take_request(), Some(Request::Revert)));
        assert!(!unsafe { IsWindowVisible(preferences.hwnd()) }.as_bool());
        // Native button only requests Off; the GUI owner must confirm before
        // changing the per-user preference. This test never writes that flag.
        unsafe {
            SendMessageW(
                preferences.hwnd(),
                WM_COMMAND,
                Some(WPARAM(155)),
                Some(LPARAM(0)),
            )
        };
        assert!(matches!(preferences.take_request(), Some(Request::TurnOff)));
    })
    .join()
    .unwrap();
}
#[test]
fn escape_routes_to_revert_and_footer_fits_the_native_client() {
    std::thread::spawn(|| {
        let owner = OverlayWindow::create(&AppConfig::default(), true).unwrap();
        let preferences = PreferencesWindow::create(
            owner.hwnd(),
            owner.wake_handle(),
            &AppConfig::default().island,
        )
        .unwrap();
        unsafe {
            SendMessageW(
                preferences.hwnd(),
                WM_COMMAND,
                Some(WPARAM(2)),
                Some(LPARAM(0)),
            )
        };
        assert!(matches!(preferences.take_request(), Some(Request::Revert)));
        let mut client = windows::Win32::Foundation::RECT::default();
        unsafe { GetClientRect(preferences.hwnd(), &mut client) }.unwrap();
        let mut parent = windows::Win32::Foundation::RECT::default();
        unsafe { GetWindowRect(preferences.hwnd(), &mut parent) }.unwrap();
        let mut footer = windows::Win32::Foundation::RECT::default();
        unsafe {
            GetWindowRect(
                GetDlgItem(Some(preferences.hwnd()), 133).unwrap(),
                &mut footer,
            )
        }
        .unwrap();
        let dpi =
            unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(preferences.hwnd()) } as f32 / 96.0;
        assert!(
            client.bottom >= (444.0 * dpi).round() as i32,
            "footer must remain visible"
        );
        assert!(footer.bottom <= parent.bottom);
    })
    .join()
    .unwrap();
}
