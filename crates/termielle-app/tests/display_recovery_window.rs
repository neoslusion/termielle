//! Synthetic messages to isolated windows only. Never sleeps the host,
//! changes monitor topology, reserves desktop space, or touches the live bar.
use termielle_app::animation::FrameBuffer;
use termielle_app::window::{OverlayWindow, WindowEvent};
use termielle_core::{AppConfig, IslandLayout};
use windows::Win32::Foundation::{LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PostMessageW, SPI_SETWORKAREA, SendMessageW,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_POWERBROADCAST, WM_SETTINGCHANGE,
};

fn expect(window: &mut OverlayWindow, expected: WindowEvent) {
    window.set_timer(Some(2_000)).unwrap();
    loop {
        match window.next_event().unwrap() {
            Some(event) if event == expected => return,
            Some(WindowEvent::Timer) => panic!("timed out waiting for {expected:?}"),
            Some(event) => panic!("unexpected event {event:?}, wanted {expected:?}"),
            None => {}
        }
    }
}

#[test]
fn posted_resume_broadcasts_reach_the_real_message_pump() {
    std::thread::spawn(|| {
        let mut window = OverlayWindow::create(&AppConfig::default(), true).unwrap();
        for reason in [PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND] {
            unsafe {
                PostMessageW(
                    Some(window.hwnd()),
                    WM_POWERBROADCAST,
                    WPARAM(reason as usize),
                    LPARAM(0),
                )
            }
            .unwrap();
            expect(&mut window, WindowEvent::Resumed);
        }
        window.destroy();
    })
    .join()
    .unwrap();
}

#[test]
fn recovery_timer_wakes_without_an_animation_clock_or_dxgi_output() {
    std::thread::spawn(|| {
        let mut window = OverlayWindow::create(&AppConfig::default(), true).unwrap();
        window.schedule_display_recovery(true).unwrap();
        expect(&mut window, WindowEvent::DisplayRecovery);
        window.destroy();
    })
    .join()
    .unwrap();
}

#[test]
fn anchored_bar_defers_dpi_repositioning_and_distinguishes_appbar_work_area_changes() {
    std::thread::spawn(|| {
        let config = AppConfig::default();
        let mut window = OverlayWindow::create(&config, true).unwrap();
        window.set_island(true);
        window.set_bar(true);
        let before = window.position().0;
        let suggested = RECT {
            left: 500,
            top: 80,
            right: 501,
            bottom: 81,
        };
        // Pointer-bearing system messages require synchronous dispatch.
        unsafe {
            SendMessageW(
                window.hwnd(),
                WM_DPICHANGED,
                Some(WPARAM(96 | (96 << 16))),
                Some(LPARAM(&suggested as *const RECT as isize)),
            );
        }
        expect(&mut window, WindowEvent::DisplayChanged);
        assert_eq!(
            before,
            window.position().0,
            "GUI owner must anchor/resize the bar, not the callback"
        );
        unsafe {
            SendMessageW(
                window.hwnd(),
                WM_SETTINGCHANGE,
                Some(WPARAM(SPI_SETWORKAREA.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        expect(&mut window, WindowEvent::WorkAreaChanged);
        window.destroy();
    })
    .join()
    .unwrap();
}

#[test]
fn classic_display_callback_survives_a_surface_wider_than_the_monitor() {
    std::thread::spawn(|| {
        let mut config = AppConfig::default();
        config.island.layout = IslandLayout::Classic;
        let mut window = OverlayWindow::create(&config, true).unwrap();
        let width = window.monitor_width() + 640;
        let frame = FrameBuffer {
            width,
            height: 36,
            pixels_pbgra: [0, 0, 255, 255].repeat(width as usize * 36),
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        };
        window.present(&frame).unwrap();
        unsafe { PostMessageW(Some(window.hwnd()), WM_DISPLAYCHANGE, WPARAM(32), LPARAM(0)) }
            .unwrap();
        expect(&mut window, WindowEvent::DisplayChanged);
        assert_eq!(window.position().0.width(), width as i32);
        window.destroy();
    })
    .join()
    .unwrap();
}
