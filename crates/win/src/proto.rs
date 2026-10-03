//! The disk helper's pipe protocol (spec §5.7): little-endian, length-prefixed frames,
//! written by hand. A frame is `u32 body length` followed by the body; the first body byte
//! says what it is.
//!
//! - Requests: `1 Open { disk: u32 }`, `2 Read { offset: u64, len: u32 }`.
//! - Replies: `1 Opened { size: u64, sector: u32 }`, `2 Data(bytes)`,
//!   `3 Failed { code: u32, message: UTF-8 }`.

use std::io::{self, Read, Write};

use crate::align::MAX_IO;

/// Longest error message sent, in bytes.
const MAX_MESSAGE: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Opens `\\.\PhysicalDrive<disk>`; the first request on every connection.
    Open { disk: u32 },
    /// Sector-aligned read of at most `MAX_IO` bytes.
    Read { offset: u64, len: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Opened {
        size: u64,
        sector: u32,
    },
    Data(Vec<u8>),
    /// A Win32 error code (0 if there is none) and its message.
    Failed {
        code: u32,
        message: String,
    },
}

impl Request {
    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        let mut body = Vec::with_capacity(13);
        match *self {
            Request::Open { disk } => {
                body.push(1);
                body.extend_from_slice(&disk.to_le_bytes());
            }
            Request::Read { offset, len } => {
                body.push(2);
                body.extend_from_slice(&offset.to_le_bytes());
                body.extend_from_slice(&len.to_le_bytes());
            }
        }
        write_frame(w, &body)
    }

    /// `Ok(None)` when the other side closed the connection between frames.
    pub fn read_from(r: &mut impl Read) -> io::Result<Option<Self>> {
        let Some(body) = read_frame(r, 64)? else {
            return Ok(None);
        };
        let mut rest = body.as_slice();
        let request = match take::<1>(&mut rest)? {
            [1] => Request::Open {
                disk: u32::from_le_bytes(take(&mut rest)?),
            },
            [2] => Request::Read {
                offset: u64::from_le_bytes(take(&mut rest)?),
                len: u32::from_le_bytes(take(&mut rest)?),
            },
            [op] => return Err(invalid(format!("unknown request {op}"))),
        };
        finished(rest)?;
        Ok(Some(request))
    }
}

impl Reply {
    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        let mut body = Vec::new();
        match self {
            Reply::Opened { size, sector } => {
                body.push(1);
                body.extend_from_slice(&size.to_le_bytes());
                body.extend_from_slice(&sector.to_le_bytes());
            }
            Reply::Data(data) => {
                body.reserve(1 + data.len());
                body.push(2);
                body.extend_from_slice(data);
            }
            Reply::Failed { code, message } => {
                body.push(3);
                body.extend_from_slice(&code.to_le_bytes());
                let mut end = message.len().min(MAX_MESSAGE);
                while !message.is_char_boundary(end) {
                    end -= 1;
                }
                body.extend_from_slice(message.get(..end).unwrap_or_default().as_bytes());
            }
        }
        write_frame(w, &body)
    }

    pub fn read_from(r: &mut impl Read) -> io::Result<Self> {
        let Some(body) = read_frame(r, 1 + MAX_IO)? else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the disk helper closed the connection",
            ));
        };
        let mut rest = body.as_slice();
        let reply = match take::<1>(&mut rest)? {
            [1] => {
                let size = u64::from_le_bytes(take(&mut rest)?);
                let sector = u32::from_le_bytes(take(&mut rest)?);
                finished(rest)?;
                Reply::Opened { size, sector }
            }
            [2] => Reply::Data(rest.to_vec()),
            [3] => {
                let code = u32::from_le_bytes(take(&mut rest)?);
                if rest.len() > MAX_MESSAGE {
                    return Err(invalid("error message too long".into()));
                }
                let message = String::from_utf8_lossy(rest).into_owned();
                Reply::Failed { code, message }
            }
            [tag] => return Err(invalid(format!("unknown reply {tag}"))),
        };
        Ok(reply)
    }
}

fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len()).map_err(|_| invalid("frame too large".into()))?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(body);
    w.write_all(&frame)?;
    w.flush()
}

/// `Ok(None)` at a clean end of stream (or a closed pipe) before a frame starts.
fn read_frame(r: &mut impl Read, max: usize) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > max {
        return Err(invalid(format!("frame of {len} bytes")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

fn take<const N: usize>(rest: &mut &[u8]) -> io::Result<[u8; N]> {
    let (head, tail) = rest
        .split_first_chunk::<N>()
        .ok_or_else(|| invalid("frame too short".into()))?;
    *rest = tail;
    Ok(*head)
}

fn finished(rest: &[u8]) -> io::Result<()> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(invalid("frame too long".into()))
    }
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire_request(request: Request) -> Vec<u8> {
        let mut wire = Vec::new();
        request.write_to(&mut wire).unwrap();
        wire
    }

    #[test]
    fn requests_and_replies_round_trip() {
        for request in [
            Request::Open { disk: 7 },
            Request::Read {
                offset: 1 << 40,
                len: 4096,
            },
        ] {
            let wire = wire_request(request);
            assert_eq!(
                Request::read_from(&mut wire.as_slice()).unwrap(),
                Some(request)
            );
        }
        for reply in [
            Reply::Opened {
                size: 1 << 41,
                sector: 4096,
            },
            Reply::Data(vec![0xAB; 1000]),
            Reply::Failed {
                code: 5,
                message: "Access is denied.".into(),
            },
        ] {
            let mut wire = Vec::new();
            reply.write_to(&mut wire).unwrap();
            assert_eq!(Reply::read_from(&mut wire.as_slice()).unwrap(), reply);
        }
    }

    #[test]
    fn the_exact_bytes_of_a_read_request() {
        let wire = wire_request(Request::Read {
            offset: 512,
            len: 4096,
        });
        assert_eq!(wire, [13, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0, 0, 0, 0, 16, 0, 0]);
    }

    #[test]
    fn a_closed_connection_between_frames_is_not_an_error() {
        assert_eq!(Request::read_from(&mut [].as_slice()).unwrap(), None);
    }

    #[test]
    fn malformed_frames_are_refused_without_allocating_their_claimed_size() {
        let cases: [&[u8]; 5] = [
            &[0xFF, 0xFF, 0xFF, 0xFF],       // 4 GiB frame
            &[0, 0, 0, 0],                   // empty frame
            &[1, 0, 0, 0, 9],                // unknown request
            &[3, 0, 0, 0, 1, 0, 0],          // Open cut short
            &[6, 0, 0, 0, 1, 0, 0, 0, 0, 0], // Open with a trailing byte
        ];
        for wire in cases {
            let err = Request::read_from(&mut &wire[..]).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{wire:?}");
        }
        let huge_reply = (2 + MAX_IO as u32).to_le_bytes();
        assert!(Reply::read_from(&mut huge_reply.as_slice()).is_err());
    }

    #[test]
    fn long_error_messages_are_cut_on_a_character_boundary() {
        let message = "ğ".repeat(MAX_MESSAGE); // 2 bytes per character
        let mut wire = Vec::new();
        Reply::Failed { code: 1, message }
            .write_to(&mut wire)
            .unwrap();
        let Reply::Failed { message, .. } = Reply::read_from(&mut wire.as_slice()).unwrap() else {
            panic!("not a Failed reply");
        };
        assert_eq!(message.len(), MAX_MESSAGE);
    }
}
