//! Errors the window receives: a code it translates, and an English detail (spec §5.9).

use serde::Serialize;
use tuxread_win::helper::{LaunchError, is_helper_gone};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Code {
    /// The user said no to the UAC prompt.
    Declined,
    /// The disk helper is no longer running; the disk must be opened again.
    HelperGone,
    NotFound,
    NotADirectory,
    Unsupported,
    Corrupt,
    Io,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub code: Code,
    pub message: String,
}

pub type CmdResult<T> = Result<T, CommandError>;

impl CommandError {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<tuxread_core::Error> for CommandError {
    fn from(e: tuxread_core::Error) -> Self {
        use tuxread_core::Error as E;
        let code = match &e {
            E::Io(io) => return io_error(io),
            E::NotFound => Code::NotFound,
            E::NotADirectory => Code::NotADirectory,
            E::Unsupported(_) => Code::Unsupported,
            E::Corrupt(_) => Code::Corrupt,
            E::Other(_) => Code::Other,
        };
        Self::new(code, e.to_string())
    }
}

impl From<std::io::Error> for CommandError {
    fn from(e: std::io::Error) -> Self {
        io_error(&e)
    }
}

fn io_error(e: &std::io::Error) -> CommandError {
    let code = if is_helper_gone(e) {
        Code::HelperGone
    } else if e.kind() == std::io::ErrorKind::NotFound {
        Code::NotFound
    } else {
        Code::Io
    };
    CommandError::new(code, e.to_string())
}

impl From<LaunchError> for CommandError {
    fn from(e: LaunchError) -> Self {
        let code = match e {
            LaunchError::Declined => Code::Declined,
            LaunchError::Failed(_) => Code::Other,
        };
        Self::new(code, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_keep_their_meaning_for_the_window() {
        let declined: CommandError = LaunchError::Declined.into();
        assert_eq!(declined.code, Code::Declined);
        let missing: CommandError = std::io::Error::from(std::io::ErrorKind::NotFound).into();
        assert_eq!(missing.code, Code::NotFound);
        let corrupt: CommandError = tuxread_core::Error::Corrupt("bad inode".into()).into();
        assert_eq!(corrupt.code, Code::Corrupt);
        assert!(corrupt.message.contains("bad inode"));
        let json = serde_json::to_string(&declined).unwrap();
        assert_eq!(
            json,
            r#"{"code":"declined","message":"administrator approval was declined"}"#
        );
    }
}
