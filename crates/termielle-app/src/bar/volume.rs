//! Master output volume, shared with the Windows sound controls.

use std::sync::{Weak, mpsc};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA,
    Endpoints::{
        IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
    },
    IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeSnapshot {
    pub level: u8,
    pub muted: bool,
}

#[windows::core::implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback {
    sender: Weak<mpsc::Sender<super::metrics::Command>>,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(
        &self,
        _notification: *mut AUDIO_VOLUME_NOTIFICATION_DATA,
    ) -> windows::core::Result<()> {
        if let Some(sender) = self.sender.upgrade() {
            let _ = sender.send(super::metrics::Command::RefreshVolume);
        }
        Ok(())
    }
}

pub(crate) struct VolumeWatcher {
    device_id: String,
    endpoint: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
}

impl Drop for VolumeWatcher {
    fn drop(&mut self) {
        let _ = unsafe { self.endpoint.UnregisterControlChangeNotify(&self.callback) };
    }
}

pub(crate) fn watch_volume(
    watcher: &mut Option<VolumeWatcher>,
    sender: Weak<mpsc::Sender<super::metrics::Command>>,
) -> windows::core::Result<()> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let id = device.GetId()?;
        let device_id = id.to_string();
        CoTaskMemFree(Some(id.0.cast()));
        let device_id = device_id?;
        if watcher
            .as_ref()
            .is_some_and(|active| active.device_id == device_id)
        {
            return Ok(());
        }
        *watcher = None;
        let endpoint = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)?;
        let callback: IAudioEndpointVolumeCallback = VolumeCallback { sender }.into();
        endpoint.RegisterControlChangeNotify(&callback)?;
        *watcher = Some(VolumeWatcher {
            device_id,
            endpoint,
            callback,
        });
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn native_callback_wakes_worker_without_retaining_service() {
        let (sender, receiver) = mpsc::channel();
        let sender = Arc::new(sender);
        let callback: IAudioEndpointVolumeCallback = VolumeCallback {
            sender: Arc::downgrade(&sender),
        }
        .into();
        unsafe { callback.OnNotify(std::ptr::null_mut()) }.unwrap();
        assert!(matches!(
            receiver.try_recv(),
            Ok(super::super::metrics::Command::RefreshVolume)
        ));
        drop(sender);
        unsafe { callback.OnNotify(std::ptr::null_mut()) }.unwrap();
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }
}
