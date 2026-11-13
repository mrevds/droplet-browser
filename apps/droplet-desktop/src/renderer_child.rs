use std::io::{BufReader, BufWriter};

use droplet_ipc::{
    read_browser_msg, write_renderer_msg, BrowserToRenderer, IpcDamage, RendererToBrowser,
    ShmFrameBuffer,
};
use droplet_sandbox::{apply_renderer_sandbox, SandboxProfile};
use droplet_shell::{HeadlessRenderer, RenderPayload};

pub const ENV_SHM_FD: &str = "DROPLET_IPC_SHM_FD";
const MAX_RENDERER_DIM: usize = 8192;

pub fn parse_renderer_dimensions(args: &[String]) -> Result<(usize, usize), String> {
    let mut width = 1024usize;
    let mut height = 768usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--renderer-w" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| "--renderer-w requires a positive integer".to_string())?;
                width = val
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --renderer-w value: {val}"))?;
                i += 2;
            }
            "--renderer-h" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| "--renderer-h requires a positive integer".to_string())?;
                height = val
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --renderer-h value: {val}"))?;
                i += 2;
            }
            other => {
                return Err(format!("unknown --renderer argument: {other}"));
            }
        }
    }
    if width == 0 || height == 0 || width > MAX_RENDERER_DIM || height > MAX_RENDERER_DIM {
        return Err(format!(
            "renderer dimensions out of bounds: {width}x{height}"
        ));
    }
    Ok((width, height))
}

pub fn run_renderer_child(initial_w: usize, initial_h: usize) -> Result<(), String> {
    let token = std::env::var(ENV_SHM_FD)
        .map_err(|_| format!("missing {ENV_SHM_FD} environment variable"))?;
    let mut shm = ShmFrameBuffer::from_env_token(&token, initial_w, initial_h)
        .map_err(|e| format!("failed to attach shared framebuffer: {e}"))?;

    // Warm font engine and renderer state before seccomp locks down `openat`.
    let mut renderer = HeadlessRenderer::new(initial_w, initial_h);

    let mut stdout = BufWriter::new(std::io::stdout().lock());
    let report = match apply_renderer_sandbox(SandboxProfile::StrictRenderer) {
        Ok(report) => report,
        Err(err) => {
            let _ = write_renderer_msg(
                &mut stdout,
                &RendererToBrowser::Error {
                    message: format!("failed to apply renderer sandbox: {err}"),
                },
            );
            return Err(format!("failed to apply renderer sandbox: {err}"));
        }
    };

    write_renderer_msg(
        &mut stdout,
        &RendererToBrowser::Ready {
            seccomp_applied: report.seccomp_applied,
            user_ns_isolated: report.user_ns_isolated,
            net_ns_isolated: report.net_ns_isolated,
        },
    )
    .map_err(|e| format!("failed to write Ready message: {e}"))?;

    let mut stdin = BufReader::new(std::io::stdin().lock());
    let mut current_w = initial_w;
    let mut current_h = initial_h;
    let mut sent_logs = 0usize;

    loop {
        let msg = match read_browser_msg(&mut stdin) {
            Ok(Some(msg)) => msg,
            Ok(None) => break,
            Err(err) => {
                let _ = write_renderer_msg(
                    &mut stdout,
                    &RendererToBrowser::Error {
                        message: format!("IPC decode error: {err}"),
                    },
                );
                return Err(format!("IPC read error: {err}"));
            }
        };

        match msg {
            BrowserToRenderer::RenderPage {
                url,
                html,
                assets,
                viewport_w,
                viewport_h,
                scroll_y,
            } => {
                let w = (viewport_w as usize).clamp(1, MAX_RENDERER_DIM);
                let h = (viewport_h as usize).clamp(1, MAX_RENDERER_DIM);
                let needed = w.saturating_mul(h);
                if needed > shm.pixels().len() {
                    write_renderer_msg(
                        &mut stdout,
                        &RendererToBrowser::Error {
                            message: format!(
                                "requested viewport {w}x{h} exceeds shared buffer capacity"
                            ),
                        },
                    )
                    .map_err(|e| format!("IPC write error: {e}"))?;
                    continue;
                }
                current_w = w;
                current_h = h;
                sent_logs = 0;
                let asset_pairs: Vec<(String, Vec<u8>)> =
                    assets.into_iter().map(|a| (a.url, a.bytes)).collect();
                renderer.load_payload(RenderPayload {
                    url: &url,
                    html: &html,
                    assets: &asset_pairs,
                    viewport_width: w,
                    viewport_height: h,
                    scroll_y,
                });
                paint_and_emit(
                    &mut renderer,
                    &mut shm,
                    current_w,
                    current_h,
                    &mut sent_logs,
                    &mut stdout,
                )?;
            }
            BrowserToRenderer::Scroll { scroll_y } => {
                renderer.scroll(scroll_y);
                paint_and_emit(
                    &mut renderer,
                    &mut shm,
                    current_w,
                    current_h,
                    &mut sent_logs,
                    &mut stdout,
                )?;
            }
            BrowserToRenderer::Resize {
                viewport_w,
                viewport_h,
            } => {
                let w = (viewport_w as usize).clamp(1, MAX_RENDERER_DIM);
                let h = (viewport_h as usize).clamp(1, MAX_RENDERER_DIM);
                let needed = w.saturating_mul(h);
                if needed > shm.pixels().len() {
                    write_renderer_msg(
                        &mut stdout,
                        &RendererToBrowser::Error {
                            message: format!(
                                "resize {w}x{h} exceeds shared buffer capacity"
                            ),
                        },
                    )
                    .map_err(|e| format!("IPC write error: {e}"))?;
                    continue;
                }
                current_w = w;
                current_h = h;
                renderer.resize(w, h);
                paint_and_emit(
                    &mut renderer,
                    &mut shm,
                    current_w,
                    current_h,
                    &mut sent_logs,
                    &mut stdout,
                )?;
            }
            BrowserToRenderer::MouseMove { x, y } => {
                renderer.mouse_move(x, y);
                paint_and_emit(
                    &mut renderer,
                    &mut shm,
                    current_w,
                    current_h,
                    &mut sent_logs,
                    &mut stdout,
                )?;
            }
            BrowserToRenderer::MouseDown { x, y } => {
                renderer.mouse_down(x, y);
                paint_and_emit(
                    &mut renderer,
                    &mut shm,
                    current_w,
                    current_h,
                    &mut sent_logs,
                    &mut stdout,
                )?;
            }
            BrowserToRenderer::MouseUp { x, y } => {
                renderer.mouse_up(x, y);
                paint_and_emit(
                    &mut renderer,
                    &mut shm,
                    current_w,
                    current_h,
                    &mut sent_logs,
                    &mut stdout,
                )?;
            }
            BrowserToRenderer::JsEval { source } => match renderer.eval_js(&source) {
                Ok(_) => {
                    paint_and_emit(
                        &mut renderer,
                        &mut shm,
                        current_w,
                        current_h,
                        &mut sent_logs,
                        &mut stdout,
                    )?;
                }
                Err(message) => {
                    write_renderer_msg(&mut stdout, &RendererToBrowser::Error { message })
                        .map_err(|e| format!("IPC write error: {e}"))?;
                }
            },
            BrowserToRenderer::Shutdown => {
                let _ = write_renderer_msg(&mut stdout, &RendererToBrowser::Ack);
                break;
            }
        }
    }

    Ok(())
}

fn paint_and_emit<W: std::io::Write>(
    renderer: &mut HeadlessRenderer,
    shm: &mut ShmFrameBuffer,
    width: usize,
    height: usize,
    sent_logs: &mut usize,
    stdout: &mut W,
) -> Result<(), String> {
    let pixel_count = width.saturating_mul(height);
    let info = renderer.paint_into_slice(&mut shm.pixels_mut()[..pixel_count], width, height);

    if info.console_logs.len() > *sent_logs {
        let delta = info.console_logs[*sent_logs..].to_vec();
        *sent_logs = info.console_logs.len();
        write_renderer_msg(stdout, &RendererToBrowser::ConsoleLog { lines: delta })
            .map_err(|e| format!("IPC write error: {e}"))?;
    }

    write_renderer_msg(
        stdout,
        &RendererToBrowser::FrameReady {
            width: width as u32,
            height: height as u32,
            damage: IpcDamage {
                x: info.damage.x as u32,
                y: info.damage.y as u32,
                width: info.damage.width as u32,
                height: info.damage.height as u32,
            },
            title: info.title,
            hover_href: info.hover_href,
            layout_ms: info.layout_ms,
            paint_ms: info.paint_ms,
        },
    )
    .map_err(|e| format!("IPC write error: {e}"))
}
