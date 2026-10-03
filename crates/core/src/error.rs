use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    NotFound,
    NotADirectory,
    /// The data is valid but uses something TuxRead cannot read (yet).
    Unsupported(String),
    /// The on-disk data is inconsistent.
    Corrupt(String),
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::NotFound => f.write_str("not found"),
            Error::NotADirectory => f.write_str("not a directory"),
            Error::Unsupported(what) => write!(f, "not supported: {what}"),
            Error::Corrupt(what) => write!(f, "corrupt data: {what}"),
            Error::Other(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
