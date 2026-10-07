//! Pointer-width-neutral Win32 LongPtr aliases. windows-rs maps these to
//! Long on x86 with i32 signatures, unlike the pointer-sized x64 signatures.
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{GET_CLASS_LONG_INDEX, WINDOW_LONG_PTR_INDEX},
};

pub(crate) unsafe fn window_long_ptr(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> isize {
    #[cfg(target_pointer_width = "64")]
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, index)
    }
    #[cfg(target_pointer_width = "32")]
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongW(hwnd, index) as isize
    }
}

pub(crate) unsafe fn set_window_long_ptr(
    hwnd: HWND,
    index: WINDOW_LONG_PTR_INDEX,
    value: isize,
) -> isize {
    #[cfg(target_pointer_width = "64")]
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(hwnd, index, value)
    }
    #[cfg(target_pointer_width = "32")]
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SetWindowLongW(hwnd, index, value as i32) as isize
    }
}

pub(crate) unsafe fn class_long_ptr(hwnd: HWND, index: GET_CLASS_LONG_INDEX) -> usize {
    #[cfg(target_pointer_width = "64")]
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetClassLongPtrW(hwnd, index)
    }
    #[cfg(target_pointer_width = "32")]
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetClassLongW(hwnd, index) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{Win32::UI::WindowsAndMessaging::*, core::w};
    #[test]
    fn owned_hidden_userdata_preserves_pointer_bits_and_previous_value() {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        for value in [isize::MIN + 17, isize::MAX - 31, 0] {
            let previous = unsafe { window_long_ptr(hwnd, GWLP_USERDATA) };
            assert_eq!(
                unsafe { set_window_long_ptr(hwnd, GWLP_USERDATA, value) },
                previous
            );
            assert_eq!(unsafe { window_long_ptr(hwnd, GWLP_USERDATA) }, value);
        }
        assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
        unsafe { DestroyWindow(hwnd) }.unwrap();
    }
}
