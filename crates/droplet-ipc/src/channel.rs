use std::io::{Read, Write};

use crate::protocol::{
    decode_browser_msg, decode_renderer_msg, encode_browser_msg, encode_renderer_msg, read_frame,
    write_frame, BrowserToRenderer, RendererToBrowser,
};
use crate::IpcError;

pub fn write_browser_msg<W: Write>(
    writer: &mut W,
    msg: &BrowserToRenderer,
) -> Result<(), IpcError> {
    let payload = encode_browser_msg(msg)?;
    write_frame(writer, &payload)
}

pub fn read_browser_msg<R: Read>(reader: &mut R) -> Result<Option<BrowserToRenderer>, IpcError> {
    match read_frame(reader) {
        Ok(payload) => decode_browser_msg(&payload).map(Some),
        Err(IpcError::Eof) => Ok(None),
        Err(err) => Err(err),
    }
}

pub fn write_renderer_msg<W: Write>(
    writer: &mut W,
    msg: &RendererToBrowser,
) -> Result<(), IpcError> {
    let payload = encode_renderer_msg(msg)?;
    write_frame(writer, &payload)
}

pub fn read_renderer_msg<R: Read>(reader: &mut R) -> Result<Option<RendererToBrowser>, IpcError> {
    match read_frame(reader) {
        Ok(payload) => decode_renderer_msg(&payload).map(Some),
        Err(IpcError::Eof) => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::IpcDamage;
    use std::io::Cursor;

    #[test]
    fn channel_helpers_stream_multiple_messages_and_clean_eof() {
        let mut pipe = Vec::new();
        let req1 = BrowserToRenderer::JsEval {
            source: "1 + 2".to_string(),
        };
        let req2 = BrowserToRenderer::Shutdown;
        write_browser_msg(&mut pipe, &req1).expect("write 1");
        write_browser_msg(&mut pipe, &req2).expect("write 2");

        let mut reader = Cursor::new(pipe);
        assert_eq!(read_browser_msg(&mut reader).unwrap(), Some(req1));
        assert_eq!(read_browser_msg(&mut reader).unwrap(), Some(req2));
        assert_eq!(read_browser_msg(&mut reader).unwrap(), None);

        let mut rpipe = Vec::new();
        let res1 = RendererToBrowser::Ready {
            seccomp_applied: true,
            user_ns_isolated: true,
            net_ns_isolated: true,
        };
        let res2 = RendererToBrowser::FrameReady {
            width: 640,
            height: 480,
            damage: IpcDamage {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            title: "Droplet".to_string(),
            hover_href: None,
            layout_ms: 0.3,
            paint_ms: 0.7,
        };
        write_renderer_msg(&mut rpipe, &res1).expect("write r1");
        write_renderer_msg(&mut rpipe, &res2).expect("write r2");

        let mut rreader = Cursor::new(rpipe);
        assert_eq!(read_renderer_msg(&mut rreader).unwrap(), Some(res1));
        assert_eq!(read_renderer_msg(&mut rreader).unwrap(), Some(res2));
        assert_eq!(read_renderer_msg(&mut rreader).unwrap(), None);
    }
}
