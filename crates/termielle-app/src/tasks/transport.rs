use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Sessions,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Playback,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Previous,
    Next,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub app: String,
    pub command: Command,
}

pub struct Completion {
    pub app: String,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("The displayed player is no longer available. Open the player and try again.")]
    MissingSession,
    #[error("This player does not support that media command for the current clip.")]
    Unsupported,
    #[error("The player did not accept the media command. Try controlling it in the player.")]
    Rejected,
    #[error("Windows could not control the player: {0}")]
    Windows(#[from] windows::core::Error),
}

#[derive(Default)]
struct Capabilities {
    play: bool,
    pause: bool,
    toggle: bool,
    previous: bool,
    next: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Play,
    Pause,
    Toggle,
    Previous,
    Next,
}

fn operation(command: Command, playing: bool, controls: &Capabilities) -> Option<Operation> {
    match command {
        Command::Toggle if playing && controls.pause => Some(Operation::Pause),
        Command::Toggle if !playing && controls.play => Some(Operation::Play),
        Command::Toggle if controls.toggle => Some(Operation::Toggle),
        Command::Previous if controls.previous => Some(Operation::Previous),
        Command::Next if controls.next => Some(Operation::Next),
        _ => None,
    }
}

fn find_session(app: &str) -> Result<Session, Error> {
    if app.trim().is_empty() {
        return Err(Error::MissingSession);
    }
    let manager = super::block_async(
        Sessions::RequestAsync()?,
        |op| op.Status(),
        |op| op.GetResults(),
    )?;
    if let Ok(current) = manager.GetCurrentSession() {
        if current.SourceAppUserModelId()?.to_string() == app {
            return Ok(current);
        }
    }
    let sessions = manager.GetSessions()?;
    let mut matching = None;
    for index in 0..sessions.Size()? {
        let session = sessions.GetAt(index)?;
        if session.SourceAppUserModelId()?.to_string() == app {
            if matching.is_some() {
                return Err(Error::MissingSession);
            }
            matching = Some(session);
        }
    }
    matching.ok_or(Error::MissingSession)
}

pub fn execute(request: &Request) -> Result<(), Error> {
    let _apartment =
        crate::apartment::Apartment::new(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED)?;
    let session = find_session(&request.app)?;
    let playback = session.GetPlaybackInfo()?;
    let controls = playback.Controls()?;
    let capabilities = Capabilities {
        play: controls.IsPlayEnabled()?,
        pause: controls.IsPauseEnabled()?,
        toggle: controls.IsPlayPauseToggleEnabled()?,
        previous: controls.IsPreviousEnabled()?,
        next: controls.IsNextEnabled()?,
    };
    let operation = operation(
        request.command,
        playback.PlaybackStatus()? == Playback::Playing,
        &capabilities,
    )
    .ok_or(Error::Unsupported)?;
    let pending = match operation {
        Operation::Play => session.TryPlayAsync()?,
        Operation::Pause => session.TryPauseAsync()?,
        Operation::Toggle => session.TryTogglePlayPauseAsync()?,
        Operation::Previous => session.TrySkipPreviousAsync()?,
        Operation::Next => session.TrySkipNextAsync()?,
    };
    accepted(super::block_async(
        pending,
        |op| op.Status(),
        |op| op.GetResults(),
    )?)
}

fn accepted(success: bool) -> Result<(), Error> {
    if success {
        Ok(())
    } else {
        Err(Error::Rejected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_pause_uses_the_supported_command_for_the_current_state() {
        let controls = Capabilities {
            play: true,
            pause: true,
            ..Default::default()
        };
        assert_eq!(
            operation(Command::Toggle, true, &controls),
            Some(Operation::Pause)
        );
        assert_eq!(
            operation(Command::Toggle, false, &controls),
            Some(Operation::Play)
        );
        let controls = Capabilities {
            toggle: true,
            ..Default::default()
        };
        assert_eq!(
            operation(Command::Toggle, true, &controls),
            Some(Operation::Toggle)
        );
        assert_eq!(
            operation(Command::Toggle, false, &controls),
            Some(Operation::Toggle)
        );
    }

    #[test]
    fn unsupported_commands_are_not_sent_to_another_player() {
        let controls = Capabilities::default();
        for command in [Command::Toggle, Command::Previous, Command::Next] {
            assert_eq!(operation(command, true, &controls), None);
            assert_eq!(operation(command, false, &controls), None);
        }
        assert!(matches!(find_session(""), Err(Error::MissingSession)));
    }

    #[test]
    fn skips_require_the_corresponding_capability() {
        let controls = Capabilities {
            next: true,
            ..Default::default()
        };
        assert_eq!(
            operation(Command::Next, true, &controls),
            Some(Operation::Next)
        );
        assert_eq!(operation(Command::Previous, true, &controls), None);
        let controls = Capabilities {
            previous: true,
            ..Default::default()
        };
        assert_eq!(
            operation(Command::Previous, true, &controls),
            Some(Operation::Previous)
        );
        assert_eq!(operation(Command::Next, true, &controls), None);
    }

    #[test]
    fn a_false_windows_result_is_not_reported_as_success() {
        assert!(accepted(true).is_ok());
        assert!(matches!(accepted(false), Err(Error::Rejected)));
    }
}
