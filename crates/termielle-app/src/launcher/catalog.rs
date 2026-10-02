use super::model::App;
use crate::animation::FrameBuffer;
use std::mem::size_of;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, GetDC, ReleaseDC};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, ILGetSize, IShellItem,
    IShellItemImageFactory, KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SEE_MASK_IDLIST,
    SEE_MASK_NOASYNC, SHCreateItemFromIDList, SHELLEXECUTEINFOW, SHGetIDListFromObject,
    SHGetKnownFolderItem, SIGDN_NORMALDISPLAY, SIIGBF_ICONONLY, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{Interface, w};

pub(super) fn enumerate() -> windows::core::Result<Vec<App>> {
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        let mut apps = Vec::new();
        loop {
            let mut batch = [None];
            let mut fetched = 0;
            items.Next(&mut batch, Some(&mut fetched))?;
            let Some(item) = batch[0].take().filter(|_| fetched != 0) else {
                break;
            };
            if let Ok(app) = read_app(&item) {
                apps.push(app);
            }
        }
        apps.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then(left.target.cmp(&right.target))
        });
        apps.dedup_by(|left, right| left.target == right.target);
        Ok(apps)
    }
}

fn read_app(item: &IShellItem) -> windows::core::Result<App> {
    unsafe {
        let display_name = item.GetDisplayName(SIGDN_NORMALDISPLAY)?;
        let name = display_name.to_string();
        CoTaskMemFree(Some(display_name.0.cast()));
        let name = name?;
        let id = SHGetIDListFromObject(item)?;
        let target =
            std::slice::from_raw_parts(id.cast::<u8>(), ILGetSize(Some(id)) as usize).to_vec();
        CoTaskMemFree(Some(id.cast()));
        Ok(App {
            key: name.to_lowercase(),
            name,
            target,
            icon: None,
        })
    }
}

pub(super) fn read_icon(target: &[u8]) -> Option<FrameBuffer> {
    let item: IShellItem = unsafe { SHCreateItemFromIDList(target.as_ptr().cast()) }.ok()?;
    let factory: IShellItemImageFactory = item.cast().ok()?;
    let bitmap = unsafe { factory.GetImage(SIZE { cx: 96, cy: 96 }, SIIGBF_ICONONLY) }.ok()?;
    let result = (|| {
        let (width, height) = crate::tasks::bitmap_dims(bitmap)?;
        if width > 256 || height > 256 {
            return None;
        }
        let screen = unsafe { GetDC(None) };
        if screen.is_invalid() {
            return None;
        }
        let pixels = crate::tasks::dib_bits(screen, bitmap, width, height, 32);
        unsafe { ReleaseDC(None, screen) };
        let mut pixels = pixels?;
        let has_alpha = pixels.chunks_exact(4).any(|pixel| pixel[3] != 0);
        for pixel in pixels.chunks_exact_mut(4) {
            if !has_alpha {
                pixel[3] = 255;
            }
            for channel in 0..3 {
                pixel[channel] = (u16::from(pixel[channel]) * u16::from(pixel[3]) / 255) as u8;
            }
        }
        Some(FrameBuffer {
            width: width as u32,
            height: height as u32,
            pixels_pbgra: pixels,
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        })
    })();
    let _ = unsafe { DeleteObject(bitmap.into()) };
    result
}

pub(super) fn launch(target: &[u8]) -> windows::core::Result<()> {
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpVerb: w!("open"),
        lpIDList: target.as_ptr().cast_mut().cast(),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Opens Calculator for local launch verification"]
    fn registered_calculator_can_be_launched_without_windows_search() {
        let _apartment =
            crate::apartment::Apartment::new(windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED)
                .unwrap();
        let apps = enumerate().unwrap();
        let calculator = apps
            .iter()
            .find(|app| app.key == "calculator")
            .expect("Calculator must be installed for this manual smoke test");
        launch(&calculator.target).unwrap();
    }

    #[test]
    fn live_catalog_contains_registered_apps_and_valid_shell_targets() {
        let _apartment =
            crate::apartment::Apartment::new(windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED)
                .unwrap();
        let started = std::time::Instant::now();
        let apps = enumerate().unwrap();
        assert!(!apps.is_empty());
        assert!(apps.iter().all(|app| !app.name.trim().is_empty()
            && app.target.len() >= 2
            && app.target.ends_with(&[0, 0])));
        println!(
            "Indexed {} app names in {:?}",
            apps.len(),
            started.elapsed()
        );
        assert!(apps.iter().all(|app| app.icon.is_none()));
    }
}
