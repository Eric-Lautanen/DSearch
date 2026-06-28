use crate::proto::msg_type::{MsgType, MAX_PAYLOAD_SIZE, PROTOCOL_VERSION};
use serde::Deserialize;
use std::io;

const HEADER_SIZE: usize = 6; // 1 + 1 + 4

#[derive(Debug)]
pub struct Frame {
    pub version: u8,
    pub msg_type: MsgType,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(msg_type: MsgType, payload: Vec<u8>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            msg_type,
            payload,
        }
    }

    /// Encode frame into bytes: [version u8][msg_type u8][length u32 BE][payload]
    pub fn encode(&self) -> Vec<u8> {
        let len = self.payload.len() as u32;
        let mut buf = Vec::with_capacity(HEADER_SIZE + self.payload.len());
        buf.push(self.version);
        buf.push(self.msg_type.as_u8());
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Decode payload as a typed message.
    pub fn decode_payload<T: for<'de> Deserialize<'de>>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.payload)
    }
}

#[derive(Debug)]
pub enum FrameError {
    PayloadTooLarge { got: u32, max: u32 },
    UnknownMsgType(u8),
    Io(io::Error),
}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        FrameError::Io(e)
    }
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::PayloadTooLarge { got, max } => {
                write!(f, "payload too large: {} bytes (max {})", got, max)
            }
            FrameError::UnknownMsgType(t) => write!(f, "unknown message type: 0x{:02X}", t),
            FrameError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for FrameError {}

/// Async read a frame from a QUIC receive stream.
pub async fn read_frame(recv: &mut quinn::RecvStream) -> Result<Option<Frame>, FrameError> {
    let mut header = [0u8; HEADER_SIZE];
    match recv.read_exact(&mut header).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(_)) => return Ok(None),
        Err(quinn::ReadExactError::ReadError(e)) => {
            return Err(FrameError::Io(io::Error::other(e.to_string())));
        }
    }

    let version = header[0];
    let msg_byte = header[1];
    let length = u32::from_be_bytes([header[2], header[3], header[4], header[5]]);

    if length > MAX_PAYLOAD_SIZE {
        return Err(FrameError::PayloadTooLarge {
            got: length,
            max: MAX_PAYLOAD_SIZE,
        });
    }

    let msg_type = MsgType::from_u8(msg_byte).ok_or(FrameError::UnknownMsgType(msg_byte))?;

    let mut payload = vec![0u8; length as usize];
    if length > 0 {
        match recv.read_exact(&mut payload).await {
            Ok(()) => {}
            Err(quinn::ReadExactError::FinishedEarly(_)) => return Ok(None),
            Err(quinn::ReadExactError::ReadError(e)) => {
                return Err(FrameError::Io(io::Error::other(e.to_string())));
            }
        }
    }

    Ok(Some(Frame {
        version,
        msg_type,
        payload,
    }))
}

/// Async write a frame to a QUIC send stream.
pub async fn write_frame(
    send: &mut quinn::SendStream,
    frame: &Frame,
) -> Result<(), quinn::WriteError> {
    send.write_all(&frame.encode()).await
}
