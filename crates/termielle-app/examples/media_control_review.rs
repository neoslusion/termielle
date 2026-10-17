use std::time::Duration;
use termielle_app::tasks::{MediaInfo, current_media, transport};

fn same_clip(before: &MediaInfo, after: &MediaInfo) -> bool {
    before.app == after.app && before.title == after.title && before.artist == after.artist
}

fn wait_for(before: &MediaInfo, playing: bool) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(100));
        if let Some(after) = current_media() {
            if !same_clip(before, &after) {
                return Err("The media source changed during the review".into());
            }
            if after.playing == playing {
                return Ok(());
            }
        }
    }
    Err("The player did not publish the expected playback state".into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let before =
        current_media().ok_or("No playing or paused Windows media session is available")?;
    println!("Source: {}; playing: {}", before.app, before.playing);
    if !std::env::args().any(|arg| arg == "--exercise") {
        return Ok(());
    }
    let request = transport::Request {
        app: before.app.clone(),
        command: transport::Command::Toggle,
    };
    transport::execute(&request)?;
    let changed = wait_for(&before, !before.playing);
    let current = current_media().ok_or("The media session disappeared after the command")?;
    if !same_clip(&before, &current) {
        return Err("The media source changed; refusing to control a different clip".into());
    }
    if current.playing != before.playing {
        transport::execute(&request)?;
        wait_for(&before, before.playing)?;
    }
    changed?;
    println!("Verified play/pause on the same clip; initial playing state restored.");
    Ok(())
}
