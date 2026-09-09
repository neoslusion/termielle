//! Audio volume level query and toggle.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use windows::Win32::Media::Audio::{waveOutGetVolume, waveOutSetVolume};

static MUTED: AtomicBool = AtomicBool::new(false);
static PREV_VOL: AtomicU8 = AtomicU8::new(75);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeSnapshot {
    pub level: u8, // 0-100
    pub muted: bool,
}

pub fn query_volume() -> VolumeSnapshot {
    let mut vol_dword = 0u32;
    // MMSYSERR_NOERROR is 0
    let res = unsafe { waveOutGetVolume(None, &raw mut vol_dword) };
    if res != 0 {
        return VolumeSnapshot {
            level: PREV_VOL.load(Ordering::Relaxed),
            muted: MUTED.load(Ordering::Relaxed),
        };
    }

    let left = (vol_dword & 0xffff) as f32;
    let level = ((left / 65535.0) * 100.0).round() as u8;
    let muted = MUTED.load(Ordering::Relaxed) || level == 0;

    VolumeSnapshot { level, muted }
}

pub fn toggle_mute() {
    let current_muted = MUTED.load(Ordering::Relaxed);
    if current_muted {
        let prev = PREV_VOL.load(Ordering::Relaxed).max(10);
        set_volume(prev);
        MUTED.store(false, Ordering::Relaxed);
    } else {
        let current = query_volume().level;
        PREV_VOL.store(current, Ordering::Relaxed);
        set_volume(0);
        MUTED.store(true, Ordering::Relaxed);
    }
}

pub fn set_volume(level: u8) {
    let level = level.min(100);
    let channel_val = ((level as f32 / 100.0) * 65535.0).round() as u32;
    let dword = channel_val | (channel_val << 16);
    let _ = unsafe { waveOutSetVolume(None, dword) };
    if level > 0 {
        MUTED.store(false, Ordering::Relaxed);
        PREV_VOL.store(level, Ordering::Relaxed);
    }
}
