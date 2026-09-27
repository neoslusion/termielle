//! Master output volume, shared with the Windows sound controls.

use windows::Win32::Media::Audio::{
    Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeSnapshot {
    pub level: u8,
    pub muted: bool,
}

struct Apartment(bool);
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn with_endpoint<T>(
    f: impl FnOnce(&IAudioEndpointVolume) -> windows::core::Result<T>,
) -> windows::core::Result<T> {
    // RPC_E_CHANGED_MODE means the caller already owns a usable apartment.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let _apartment = Apartment(hr.is_ok());
    if hr.is_err() && hr != windows::Win32::Foundation::RPC_E_CHANGED_MODE {
        hr.ok()?;
    }
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let endpoint = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)?;
        f(&endpoint)
    }
}

pub fn query_volume() -> VolumeSnapshot {
    try_query_volume().unwrap_or(VolumeSnapshot {
        level: 0,
        muted: true,
    })
}

pub fn try_query_volume() -> windows::core::Result<VolumeSnapshot> {
    with_endpoint(|endpoint| unsafe {
        Ok(VolumeSnapshot {
            level: (endpoint.GetMasterVolumeLevelScalar()? * 100.0).round() as u8,
            muted: endpoint.GetMute()?.as_bool(),
        })
    })
}

pub fn toggle_mute() -> windows::core::Result<()> {
    with_endpoint(|endpoint| unsafe {
        endpoint.SetMute(!endpoint.GetMute()?.as_bool(), std::ptr::null())
    })
}

pub fn set_volume(level: u8) -> windows::core::Result<()> {
    with_endpoint(|endpoint| unsafe {
        endpoint.SetMasterVolumeLevelScalar(f32::from(level.min(100)) / 100.0, std::ptr::null())?;
        if level > 0 {
            endpoint.SetMute(false, std::ptr::null())?;
        }
        Ok(())
    })
}
