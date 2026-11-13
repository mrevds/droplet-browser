pub mod channel;
pub mod protocol;
pub mod shm;

use std::fmt;

pub use channel::{read_browser_msg, read_renderer_msg, write_browser_msg, write_renderer_msg};
pub use protocol::{
    decode_browser_msg, decode_renderer_msg, encode_browser_msg, encode_renderer_msg, read_frame,
    write_frame, BrowserToRenderer, FetchedAsset, IpcDamage, RendererToBrowser, IPC_MAGIC,
    IPC_VERSION, MAX_IPC_MESSAGE_BYTES,
};
pub use shm::{ShmFrameBuffer, MAX_SHM_PIXELS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcError {
    Eof,
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u16),
    MessageTooLarge(usize),
    Truncated,
    InvalidTag(u8),
    InvalidUtf8,
    InvalidDimensions { width: usize, height: usize },
    Shm(String),
    Io(String),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof => write!(f, "IPC stream closed (EOF)"),
            Self::InvalidMagic(magic) => write!(f, "invalid IPC frame magic: {:?}", magic),
            Self::UnsupportedVersion(version) => write!(f, "unsupported IPC version: {version}"),
            Self::MessageTooLarge(len) => {
                write!(f, "IPC message length {len} exceeds maximum {MAX_IPC_MESSAGE_BYTES}")
            }
            Self::Truncated => write!(f, "truncated IPC message or frame"),
            Self::InvalidTag(tag) => write!(f, "invalid IPC message tag: {tag}"),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 in IPC payload"),
            Self::InvalidDimensions { width, height } => {
                write!(f, "invalid SHM framebuffer dimensions: {width}x{height}")
            }
            Self::Shm(msg) => write!(f, "SHM error: {msg}"),
            Self::Io(msg) => write!(f, "IPC I/O error: {msg}"),
        }
    }
}

impl std::error::Error for IpcError {}

impl From<std::io::Error> for IpcError {
    fn from(err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::UnexpectedEof {
            Self::Eof
        } else {
            Self::Io(err.to_string())
        }
    }
}
