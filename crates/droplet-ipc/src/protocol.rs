use std::io::{Read, Write};

use crate::IpcError;

pub const IPC_MAGIC: [u8; 4] = *b"DRIP";
pub const IPC_VERSION: u16 = 1;
pub const MAX_IPC_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
const FRAME_HEADER_LEN: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IpcDamage {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedAsset {
    pub url: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BrowserToRenderer {
    RenderPage {
        url: String,
        html: String,
        assets: Vec<FetchedAsset>,
        viewport_w: u32,
        viewport_h: u32,
        scroll_y: f32,
    },
    Scroll {
        scroll_y: f32,
    },
    Resize {
        viewport_w: u32,
        viewport_h: u32,
    },
    MouseMove {
        x: f32,
        y: f32,
    },
    MouseDown {
        x: f32,
        y: f32,
    },
    MouseUp {
        x: f32,
        y: f32,
    },
    JsEval {
        source: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RendererToBrowser {
    Ready {
        seccomp_applied: bool,
        user_ns_isolated: bool,
        net_ns_isolated: bool,
    },
    FrameReady {
        width: u32,
        height: u32,
        damage: IpcDamage,
        title: String,
        hover_href: Option<String>,
        layout_ms: f32,
        paint_ms: f32,
    },
    ConsoleLog {
        lines: Vec<String>,
    },
    NavigateRequest {
        url: String,
    },
    Error {
        message: String,
    },
    Ack,
}

pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), IpcError> {
    if payload.len() > MAX_IPC_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(payload.len()));
    }
    let mut header = [0u8; FRAME_HEADER_LEN];
    header[0..4].copy_from_slice(&IPC_MAGIC);
    header[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
    header[6..10].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    writer.write_all(&header)?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, IpcError> {
    let mut header = [0u8; FRAME_HEADER_LEN];
    let mut filled = 0usize;
    while filled < FRAME_HEADER_LEN {
        match reader.read(&mut header[filled..]) {
            Ok(0) => {
                return Err(if filled == 0 {
                    IpcError::Eof
                } else {
                    IpcError::Truncated
                });
            }
            Ok(n) => filled += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(IpcError::from(err)),
        }
    }

    let magic = [header[0], header[1], header[2], header[3]];
    if magic != IPC_MAGIC {
        return Err(IpcError::InvalidMagic(magic));
    }
    let version = u16::from_le_bytes([header[4], header[5]]);
    if version != IPC_VERSION {
        return Err(IpcError::UnsupportedVersion(version));
    }
    let len = u32::from_le_bytes([header[6], header[7], header[8], header[9]]) as usize;
    if len > MAX_IPC_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(len));
    }

    let mut payload = vec![0u8; len];
    if let Err(err) = reader.read_exact(&mut payload) {
        return Err(if err.kind() == std::io::ErrorKind::UnexpectedEof {
            IpcError::Truncated
        } else {
            IpcError::from(err)
        });
    }
    Ok(payload)
}

pub fn encode_browser_msg(msg: &BrowserToRenderer) -> Result<Vec<u8>, IpcError> {
    let mut out = Vec::new();
    match msg {
        BrowserToRenderer::RenderPage {
            url,
            html,
            assets,
            viewport_w,
            viewport_h,
            scroll_y,
        } => {
            out.push(1);
            write_str(&mut out, url)?;
            write_str(&mut out, html)?;
            write_u32(&mut out, assets.len())?;
            for asset in assets {
                write_str(&mut out, &asset.url)?;
                write_bytes(&mut out, &asset.bytes)?;
            }
            out.extend_from_slice(&viewport_w.to_le_bytes());
            out.extend_from_slice(&viewport_h.to_le_bytes());
            out.extend_from_slice(&scroll_y.to_le_bytes());
        }
        BrowserToRenderer::Scroll { scroll_y } => {
            out.push(2);
            out.extend_from_slice(&scroll_y.to_le_bytes());
        }
        BrowserToRenderer::Resize {
            viewport_w,
            viewport_h,
        } => {
            out.push(3);
            out.extend_from_slice(&viewport_w.to_le_bytes());
            out.extend_from_slice(&viewport_h.to_le_bytes());
        }
        BrowserToRenderer::MouseMove { x, y } => {
            out.push(4);
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
        }
        BrowserToRenderer::MouseDown { x, y } => {
            out.push(5);
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
        }
        BrowserToRenderer::MouseUp { x, y } => {
            out.push(6);
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
        }
        BrowserToRenderer::JsEval { source } => {
            out.push(7);
            write_str(&mut out, source)?;
        }
        BrowserToRenderer::Shutdown => {
            out.push(8);
        }
    }
    if out.len() > MAX_IPC_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(out.len()));
    }
    Ok(out)
}

pub fn decode_browser_msg(bytes: &[u8]) -> Result<BrowserToRenderer, IpcError> {
    if bytes.len() > MAX_IPC_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(bytes.len()));
    }
    let mut r = Reader::new(bytes);
    let tag = r.read_u8()?;
    let msg = match tag {
        1 => {
            let url = r.read_str()?;
            let html = r.read_str()?;
            let count = r.read_u32()? as usize;
            if count > r.remaining() / 8 {
                return Err(IpcError::Truncated);
            }
            let mut assets = Vec::with_capacity(count);
            for _ in 0..count {
                let asset_url = r.read_str()?;
                let asset_bytes = r.read_bytes()?;
                assets.push(FetchedAsset {
                    url: asset_url,
                    bytes: asset_bytes,
                });
            }
            let viewport_w = r.read_u32()?;
            let viewport_h = r.read_u32()?;
            let scroll_y = r.read_f32()?;
            BrowserToRenderer::RenderPage {
                url,
                html,
                assets,
                viewport_w,
                viewport_h,
                scroll_y,
            }
        }
        2 => BrowserToRenderer::Scroll {
            scroll_y: r.read_f32()?,
        },
        3 => BrowserToRenderer::Resize {
            viewport_w: r.read_u32()?,
            viewport_h: r.read_u32()?,
        },
        4 => BrowserToRenderer::MouseMove {
            x: r.read_f32()?,
            y: r.read_f32()?,
        },
        5 => BrowserToRenderer::MouseDown {
            x: r.read_f32()?,
            y: r.read_f32()?,
        },
        6 => BrowserToRenderer::MouseUp {
            x: r.read_f32()?,
            y: r.read_f32()?,
        },
        7 => BrowserToRenderer::JsEval {
            source: r.read_str()?,
        },
        8 => BrowserToRenderer::Shutdown,
        other => return Err(IpcError::InvalidTag(other)),
    };
    r.finish()?;
    Ok(msg)
}

pub fn encode_renderer_msg(msg: &RendererToBrowser) -> Result<Vec<u8>, IpcError> {
    let mut out = Vec::new();
    match msg {
        RendererToBrowser::Ready {
            seccomp_applied,
            user_ns_isolated,
            net_ns_isolated,
        } => {
            out.push(1);
            out.push(u8::from(*seccomp_applied));
            out.push(u8::from(*user_ns_isolated));
            out.push(u8::from(*net_ns_isolated));
        }
        RendererToBrowser::FrameReady {
            width,
            height,
            damage,
            title,
            hover_href,
            layout_ms,
            paint_ms,
        } => {
            out.push(2);
            out.extend_from_slice(&width.to_le_bytes());
            out.extend_from_slice(&height.to_le_bytes());
            out.extend_from_slice(&damage.x.to_le_bytes());
            out.extend_from_slice(&damage.y.to_le_bytes());
            out.extend_from_slice(&damage.width.to_le_bytes());
            out.extend_from_slice(&damage.height.to_le_bytes());
            write_str(&mut out, title)?;
            match hover_href {
                Some(href) => {
                    out.push(1);
                    write_str(&mut out, href)?;
                }
                None => out.push(0),
            }
            out.extend_from_slice(&layout_ms.to_le_bytes());
            out.extend_from_slice(&paint_ms.to_le_bytes());
        }
        RendererToBrowser::ConsoleLog { lines } => {
            out.push(3);
            write_u32(&mut out, lines.len())?;
            for line in lines {
                write_str(&mut out, line)?;
            }
        }
        RendererToBrowser::NavigateRequest { url } => {
            out.push(4);
            write_str(&mut out, url)?;
        }
        RendererToBrowser::Error { message } => {
            out.push(5);
            write_str(&mut out, message)?;
        }
        RendererToBrowser::Ack => {
            out.push(6);
        }
    }
    if out.len() > MAX_IPC_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(out.len()));
    }
    Ok(out)
}

pub fn decode_renderer_msg(bytes: &[u8]) -> Result<RendererToBrowser, IpcError> {
    if bytes.len() > MAX_IPC_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(bytes.len()));
    }
    let mut r = Reader::new(bytes);
    let tag = r.read_u8()?;
    let msg = match tag {
        1 => RendererToBrowser::Ready {
            seccomp_applied: r.read_bool()?,
            user_ns_isolated: r.read_bool()?,
            net_ns_isolated: r.read_bool()?,
        },
        2 => {
            let width = r.read_u32()?;
            let height = r.read_u32()?;
            let damage = IpcDamage {
                x: r.read_u32()?,
                y: r.read_u32()?,
                width: r.read_u32()?,
                height: r.read_u32()?,
            };
            let title = r.read_str()?;
            let has_href = r.read_bool()?;
            let hover_href = if has_href { Some(r.read_str()?) } else { None };
            let layout_ms = r.read_f32()?;
            let paint_ms = r.read_f32()?;
            RendererToBrowser::FrameReady {
                width,
                height,
                damage,
                title,
                hover_href,
                layout_ms,
                paint_ms,
            }
        }
        3 => {
            let count = r.read_u32()? as usize;
            if count > r.remaining() / 4 {
                return Err(IpcError::Truncated);
            }
            let mut lines = Vec::with_capacity(count);
            for _ in 0..count {
                lines.push(r.read_str()?);
            }
            RendererToBrowser::ConsoleLog { lines }
        }
        4 => RendererToBrowser::NavigateRequest { url: r.read_str()? },
        5 => RendererToBrowser::Error {
            message: r.read_str()?,
        },
        6 => RendererToBrowser::Ack,
        other => return Err(IpcError::InvalidTag(other)),
    };
    r.finish()?;
    Ok(msg)
}

fn write_u32(out: &mut Vec<u8>, len: usize) -> Result<(), IpcError> {
    let v = u32::try_from(len).map_err(|_| IpcError::MessageTooLarge(len))?;
    out.extend_from_slice(&v.to_le_bytes());
    Ok(())
}

fn write_bytes(out: &mut Vec<u8>, data: &[u8]) -> Result<(), IpcError> {
    if data.len() > MAX_IPC_MESSAGE_BYTES {
        return Err(IpcError::MessageTooLarge(data.len()));
    }
    write_u32(out, data.len())?;
    out.extend_from_slice(data);
    Ok(())
}

fn write_str(out: &mut Vec<u8>, s: &str) -> Result<(), IpcError> {
    write_bytes(out, s.as_bytes())
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], IpcError> {
        if n > self.remaining() {
            return Err(IpcError::Truncated);
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, IpcError> {
        Ok(self.take(1)?[0])
    }

    fn read_bool(&mut self) -> Result<bool, IpcError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(IpcError::InvalidTag(other)),
        }
    }

    fn read_u32(&mut self) -> Result<u32, IpcError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_f32(&mut self) -> Result<f32, IpcError> {
        let bytes = self.take(4)?;
        Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_bytes(&mut self) -> Result<Vec<u8>, IpcError> {
        let len = self.read_u32()? as usize;
        if len > MAX_IPC_MESSAGE_BYTES {
            return Err(IpcError::MessageTooLarge(len));
        }
        Ok(self.take(len)?.to_vec())
    }

    fn read_str(&mut self) -> Result<String, IpcError> {
        let len = self.read_u32()? as usize;
        if len > MAX_IPC_MESSAGE_BYTES {
            return Err(IpcError::MessageTooLarge(len));
        }
        let slice = self.take(len)?;
        let s = std::str::from_utf8(slice).map_err(|_| IpcError::InvalidUtf8)?;
        Ok(s.to_string())
    }

    fn finish(&self) -> Result<(), IpcError> {
        if self.pos == self.data.len() {
            Ok(())
        } else {
            Err(IpcError::Truncated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn browser_messages_round_trip_with_cyrillic_and_assets() {
        let msgs = vec![
            BrowserToRenderer::RenderPage {
                url: "https://example.org/тест".to_string(),
                html: "<h1>Привет, мир!</h1>".to_string(),
                assets: vec![
                    FetchedAsset {
                        url: "https://example.org/style.css".to_string(),
                        bytes: b"body { color: #112233; }".to_vec(),
                    },
                    FetchedAsset {
                        url: "https://example.org/app.js".to_string(),
                        bytes: "console.log('Привет');".as_bytes().to_vec(),
                    },
                ],
                viewport_w: 1280,
                viewport_h: 720,
                scroll_y: 42.5,
            },
            BrowserToRenderer::Scroll { scroll_y: 120.25 },
            BrowserToRenderer::Resize {
                viewport_w: 1920,
                viewport_h: 1080,
            },
            BrowserToRenderer::MouseMove { x: 10.5, y: 20.5 },
            BrowserToRenderer::MouseDown { x: 100.0, y: 200.0 },
            BrowserToRenderer::MouseUp { x: 100.0, y: 200.0 },
            BrowserToRenderer::JsEval {
                source: "document.title = 'Заголовок'".to_string(),
            },
            BrowserToRenderer::Shutdown,
        ];

        for msg in &msgs {
            let encoded = encode_browser_msg(msg).expect("encode");
            let mut buf = Vec::new();
            write_frame(&mut buf, &encoded).expect("write_frame");
            let mut cursor = Cursor::new(buf);
            let frame = read_frame(&mut cursor).expect("read_frame");
            let decoded = decode_browser_msg(&frame).expect("decode");
            assert_eq!(&decoded, msg);
            assert_eq!(read_frame(&mut cursor).unwrap_err(), IpcError::Eof);
        }
    }

    #[test]
    fn renderer_messages_round_trip_with_cyrillic() {
        let msgs = vec![
            RendererToBrowser::Ready {
                seccomp_applied: true,
                user_ns_isolated: false,
                net_ns_isolated: true,
            },
            RendererToBrowser::FrameReady {
                width: 1280,
                height: 720,
                damage: IpcDamage {
                    x: 10,
                    y: 20,
                    width: 300,
                    height: 400,
                },
                title: "Заголовок страницы".to_string(),
                hover_href: Some("https://example.org/ссылка".to_string()),
                layout_ms: 0.45,
                paint_ms: 1.12,
            },
            RendererToBrowser::FrameReady {
                width: 800,
                height: 600,
                damage: IpcDamage::default(),
                title: String::new(),
                hover_href: None,
                layout_ms: 0.1,
                paint_ms: 0.2,
            },
            RendererToBrowser::ConsoleLog {
                lines: vec!["лог 1".to_string(), "лог 2".to_string()],
            },
            RendererToBrowser::NavigateRequest {
                url: "about:droplet".to_string(),
            },
            RendererToBrowser::Error {
                message: "Ошибка скрипта".to_string(),
            },
            RendererToBrowser::Ack,
        ];

        for msg in &msgs {
            let encoded = encode_renderer_msg(msg).expect("encode");
            let mut buf = Vec::new();
            write_frame(&mut buf, &encoded).expect("write_frame");
            let mut cursor = Cursor::new(buf);
            let frame = read_frame(&mut cursor).expect("read_frame");
            let decoded = decode_renderer_msg(&frame).expect("decode");
            assert_eq!(&decoded, msg);
        }
    }

    #[test]
    fn rejects_corrupt_magic_version_oversized_and_truncated_frames() {
        // Corrupt magic
        let bad_magic = [b'X', b'Y', b'Z', b'W', 1, 0, 0, 0, 0, 0];
        assert_eq!(
            read_frame(&mut Cursor::new(&bad_magic)).unwrap_err(),
            IpcError::InvalidMagic(*b"XYZW")
        );

        // Unsupported version
        let mut bad_ver = Vec::new();
        bad_ver.extend_from_slice(&IPC_MAGIC);
        bad_ver.extend_from_slice(&99u16.to_le_bytes());
        bad_ver.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            read_frame(&mut Cursor::new(&bad_ver)).unwrap_err(),
            IpcError::UnsupportedVersion(99)
        );

        // Oversized frame length in header
        let mut too_big = Vec::new();
        too_big.extend_from_slice(&IPC_MAGIC);
        too_big.extend_from_slice(&IPC_VERSION.to_le_bytes());
        too_big.extend_from_slice(&((MAX_IPC_MESSAGE_BYTES as u32) + 1).to_le_bytes());
        assert_eq!(
            read_frame(&mut Cursor::new(&too_big)).unwrap_err(),
            IpcError::MessageTooLarge(MAX_IPC_MESSAGE_BYTES + 1)
        );

        // Oversized write_frame
        let big_payload = vec![0u8; MAX_IPC_MESSAGE_BYTES + 1];
        assert_eq!(
            write_frame(&mut Vec::new(), &big_payload).unwrap_err(),
            IpcError::MessageTooLarge(MAX_IPC_MESSAGE_BYTES + 1)
        );

        // Truncated header
        assert_eq!(
            read_frame(&mut Cursor::new(b"DRI")).unwrap_err(),
            IpcError::Truncated
        );

        // Truncated payload
        let mut trunc_payload = Vec::new();
        trunc_payload.extend_from_slice(&IPC_MAGIC);
        trunc_payload.extend_from_slice(&IPC_VERSION.to_le_bytes());
        trunc_payload.extend_from_slice(&10u32.to_le_bytes());
        trunc_payload.extend_from_slice(&[1, 2, 3]);
        assert_eq!(
            read_frame(&mut Cursor::new(&trunc_payload)).unwrap_err(),
            IpcError::Truncated
        );

        // Invalid UTF-8 & invalid tag & huge string length in decode
        let mut invalid_utf8 = vec![7u8];
        invalid_utf8.extend_from_slice(&2u32.to_le_bytes());
        invalid_utf8.extend_from_slice(&[0xFF, 0xFE]);
        assert_eq!(
            decode_browser_msg(&invalid_utf8).unwrap_err(),
            IpcError::InvalidUtf8
        );

        assert_eq!(
            decode_browser_msg(&[250]).unwrap_err(),
            IpcError::InvalidTag(250)
        );
        assert_eq!(
            decode_renderer_msg(&[6, 0]).unwrap_err(),
            IpcError::Truncated
        );
    }
}
