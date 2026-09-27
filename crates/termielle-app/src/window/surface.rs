//! Reusable, UI-thread-owned upload surface for UpdateLayeredWindow.

use super::WindowError;
use std::{ffi::c_void, marker::PhantomData, rc::Rc};
use windows::Win32::Graphics::Gdi::*;

pub(super) struct Surface {
    pub dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut c_void,
    pub width: u32,
    pub height: u32,
    _thread: PhantomData<Rc<()>>,
}

impl Surface {
    pub fn new(width: u32, height: u32) -> Result<Self, WindowError> {
        let width = width
            .checked_next_power_of_two()
            .filter(|w| *w <= i32::MAX as u32)
            .ok_or(WindowError::FrameBuffer)?;
        let height = height
            .checked_next_power_of_two()
            .filter(|h| *h <= i32::MAX as u32)
            .ok_or(WindowError::FrameBuffer)?;
        (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|n| *n <= isize::MAX as usize)
            .ok_or(WindowError::FrameBuffer)?;
        // A memory DC with a 32-bit DIB needs no borrowed screen DC.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return Err(windows::core::Error::from_thread().into());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = match unsafe {
            CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(bitmap) => bitmap,
            Err(error) => {
                unsafe {
                    let _ = DeleteDC(dc);
                }
                return Err(error.into());
            }
        };
        let previous = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
        if bits.is_null() || previous.is_invalid() {
            unsafe {
                let _ = DeleteDC(dc);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            return Err(WindowError::FrameBuffer);
        }
        Ok(Self {
            dc,
            bitmap,
            previous,
            bits,
            width,
            height,
            _thread: PhantomData,
        })
    }

    pub fn pixels(&mut self) -> &mut [u8] {
        // The selected DIB owns this allocation until Drop; &mut self rules
        // out simultaneous Rust access and the UI thread serializes presents.
        unsafe {
            std::slice::from_raw_parts_mut(
                self.bits.cast(),
                self.width as usize * self.height as usize * 4,
            )
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}
