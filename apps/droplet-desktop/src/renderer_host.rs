use std::collections::HashSet;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use droplet_dom::{html, Document, ElementData, NodeKind};
use droplet_gfx::{Canvas, Color, DamageRect};
use droplet_ipc::{
    read_renderer_msg, write_browser_msg, BrowserToRenderer, FetchedAsset, RendererToBrowser,
    ShmFrameBuffer,
};
use droplet_url::Url;

use crate::renderer_child::ENV_SHM_FD;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererSandboxInfo {
    pub seccomp_applied: bool,
    pub user_ns_isolated: bool,
    pub net_ns_isolated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteFrame {
    pub width: usize,
    pub height: usize,
    pub damage: DamageRect,
    pub title: String,
    pub hover_href: Option<String>,
    pub layout_ms: f32,
    pub paint_ms: f32,
    pub console_logs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefetchedPage {
    pub url: String,
    pub html: String,
    pub assets: Vec<FetchedAsset>,
}

pub struct RendererHost {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    shm: ShmFrameBuffer,
    width: usize,
    height: usize,
    sandbox: RendererSandboxInfo,
    closed: bool,
}

impl RendererHost {
    /// Spawns the current executable in `--renderer` sandboxed mode.
    pub fn spawn(width: usize, height: usize) -> Result<Self, String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("failed to locate droplet executable: {e}"))?;
        Self::spawn_with_binary(&exe, width, height)
    }

    /// Spawns a specified `droplet` binary path in `--renderer` sandboxed mode.
    pub fn spawn_with_binary(bin: &Path, width: usize, height: usize) -> Result<Self, String> {
        let w = width.max(1);
        let h = height.max(1);
        let shm = ShmFrameBuffer::new(w, h)
            .map_err(|e| format!("failed to allocate shared framebuffer: {e}"))?;
        let fd_token = shm
            .prepare_fd_for_child()
            .map_err(|e| format!("failed to prepare shared framebuffer FD for child: {e}"))?;

        let mut child = Command::new(bin)
            .arg("--renderer")
            .arg("--renderer-w")
            .arg(w.to_string())
            .arg("--renderer-h")
            .arg(h.to_string())
            .env(ENV_SHM_FD, fd_token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn renderer child '{}': {e}", bin.display()))?;

        let child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| "renderer child missing stdin pipe".to_string())?;
        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| "renderer child missing stdout pipe".to_string())?;

        let mut stdout = BufReader::new(child_stdout);
        let ready = match read_renderer_msg(&mut stdout) {
            Ok(Some(RendererToBrowser::Ready {
                seccomp_applied,
                user_ns_isolated,
                net_ns_isolated,
            })) => RendererSandboxInfo {
                seccomp_applied,
                user_ns_isolated,
                net_ns_isolated,
            },
            Ok(Some(RendererToBrowser::Error { message })) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("renderer failed sandbox startup: {message}"));
            }
            Ok(Some(other)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("unexpected initial message from renderer: {other:?}"));
            }
            Ok(None) => {
                let status = child.wait().ok();
                return Err(format!(
                    "renderer exited before Ready (status: {status:?})"
                ));
            }
            Err(err) => {
                let status = child.wait().ok();
                return Err(format!(
                    "failed waiting for renderer Ready: {err} (status: {status:?})"
                ));
            }
        };

        Ok(Self {
            child,
            stdin: BufWriter::new(child_stdin),
            stdout,
            shm,
            width: w,
            height: h,
            sandbox: ready,
            closed: false,
        })
    }

    pub fn sandbox_info(&self) -> RendererSandboxInfo {
        self.sandbox
    }

    pub fn child_id(&self) -> u32 {
        self.child.id()
    }

    pub fn pixels(&self) -> &[u32] {
        let count = self.width.saturating_mul(self.height);
        &self.shm.pixels()[..count]
    }

    pub fn snapshot_canvas(&self) -> Canvas {
        let mut canvas = Canvas::new(self.width, self.height, Color::rgb(255, 255, 255));
        let n = canvas.pixels.len().min(self.shm.pixels().len());
        canvas.pixels[..n].copy_from_slice(&self.shm.pixels()[..n]);
        canvas
    }

    pub fn render_page(
        &mut self,
        url: &str,
        html: &str,
        assets: Vec<FetchedAsset>,
        scroll_y: f32,
    ) -> Result<RemoteFrame, String> {
        self.send_and_wait_frame(BrowserToRenderer::RenderPage {
            url: url.to_string(),
            html: html.to_string(),
            assets,
            viewport_w: self.width as u32,
            viewport_h: self.height as u32,
            scroll_y,
        })
    }

    pub fn scroll(&mut self, scroll_y: f32) -> Result<RemoteFrame, String> {
        self.send_and_wait_frame(BrowserToRenderer::Scroll { scroll_y })
    }

    pub fn mouse_move(&mut self, x: f32, y: f32) -> Result<RemoteFrame, String> {
        self.send_and_wait_frame(BrowserToRenderer::MouseMove { x, y })
    }

    pub fn eval_js(&mut self, source: &str) -> Result<RemoteFrame, String> {
        self.send_and_wait_frame(BrowserToRenderer::JsEval {
            source: source.to_string(),
        })
    }

    /// Terminates the child process immediately (used by crash-resilience tests).
    pub fn force_kill_child(&mut self) -> Result<(), String> {
        self.child
            .kill()
            .map_err(|e| format!("failed to kill renderer child: {e}"))?;
        let _ = self.child.wait();
        Ok(())
    }

    pub fn shutdown(mut self) -> Result<(), String> {
        self.closed = true;
        let _ = write_browser_msg(&mut self.stdin, &BrowserToRenderer::Shutdown);
        loop {
            match read_renderer_msg(&mut self.stdout) {
                Ok(Some(RendererToBrowser::Ack)) | Ok(None) => break,
                Ok(Some(_)) => continue,
                Err(_) => break,
            }
        }
        let status = self
            .child
            .wait()
            .map_err(|e| format!("failed waiting for renderer shutdown: {e}"))?;
        if !status.success() {
            return Err(format!("renderer exited with non-zero status: {status}"));
        }
        Ok(())
    }

    fn send_and_wait_frame(&mut self, msg: BrowserToRenderer) -> Result<RemoteFrame, String> {
        write_browser_msg(&mut self.stdin, &msg).map_err(|err| {
            let status = self.child.try_wait().ok().flatten();
            format!("failed sending command to renderer: {err} (child status: {status:?})")
        })?;

        let mut console_logs = Vec::new();
        loop {
            match read_renderer_msg(&mut self.stdout) {
                Ok(Some(RendererToBrowser::ConsoleLog { lines })) => {
                    console_logs.extend(lines);
                }
                Ok(Some(RendererToBrowser::FrameReady {
                    width,
                    height,
                    damage,
                    title,
                    hover_href,
                    layout_ms,
                    paint_ms,
                })) => {
                    return Ok(RemoteFrame {
                        width: width as usize,
                        height: height as usize,
                        damage: DamageRect::new(
                            damage.x as usize,
                            damage.y as usize,
                            damage.width as usize,
                            damage.height as usize,
                        ),
                        title,
                        hover_href,
                        layout_ms,
                        paint_ms,
                        console_logs,
                    });
                }
                Ok(Some(RendererToBrowser::Error { message })) => {
                    return Err(format!("renderer error: {message}"));
                }
                Ok(Some(RendererToBrowser::NavigateRequest { .. }))
                | Ok(Some(RendererToBrowser::Ready { .. }))
                | Ok(Some(RendererToBrowser::Ack)) => continue,
                Ok(None) => {
                    let status = self.child.try_wait().ok().flatten();
                    return Err(format!(
                        "renderer closed IPC stream unexpectedly (child status: {status:?})"
                    ));
                }
                Err(err) => {
                    let status = self.child.try_wait().ok().flatten();
                    return Err(format!(
                        "IPC error reading renderer frame: {err} (child status: {status:?})"
                    ));
                }
            }
        }
    }
}

impl Drop for RendererHost {
    fn drop(&mut self) {
        if !self.closed {
            let _ = write_browser_msg(&mut self.stdin, &BrowserToRenderer::Shutdown);
            if let Ok(None) = self.child.try_wait() {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }
}

/// Fetches a page and all referenced stylesheet/image/script assets in the privileged
/// browser process so the sandboxed renderer requires zero network or filesystem I/O.
pub fn prefetch_page_payload(url_str: &str) -> Result<PrefetchedPage, String> {
    let url = Url::parse(url_str).map_err(|e| format!("Invalid URL '{url_str}': {e}"))?;
    let resp =
        droplet_net::fetch(&url).map_err(|e| format!("Failed to fetch '{}': {e}", url.as_str()))?;
    let base_url = resp.url.clone();
    let html_body = resp.body;
    let doc = html::parse(&html_body);

    let asset_refs = collect_external_asset_refs(&doc, &base_url);
    let mut assets = Vec::with_capacity(asset_refs.len());
    for resolved in asset_refs {
        if let Ok((resolved_url, bytes)) = droplet_net::fetch_uri_bytes(&resolved, &base_url) {
            assets.push(FetchedAsset {
                url: resolved_url,
                bytes,
            });
        }
    }

    Ok(PrefetchedPage {
        url: base_url.as_str().to_string(),
        html: html_body,
        assets,
    })
}

fn collect_external_asset_refs(doc: &Document, base_url: &Url) -> Vec<String> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![doc.root()];

    while let Some(curr) = stack.pop() {
        let Some(node) = doc.get(curr) else {
            continue;
        };
        if let NodeKind::Element(data) = &node.kind {
            if let Some(raw_ref) = element_asset_ref(data) {
                if !is_data_uri(raw_ref) {
                    if let Ok(resolved) = base_url.resolve(raw_ref) {
                        let key = resolved.as_str().to_string();
                        if seen.insert(key.clone()) {
                            refs.push(key);
                        }
                    }
                }
            }
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }

    refs
}

fn element_asset_ref(data: &ElementData) -> Option<&str> {
    if data.tag_name.eq_ignore_ascii_case("link") {
        let is_stylesheet = data.attr("rel").is_some_and(|rel| {
            rel.split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case("stylesheet"))
        });
        if is_stylesheet {
            let href = data.attr("href")?.trim();
            return (!href.is_empty()).then_some(href);
        }
        return None;
    }
    if data.tag_name.eq_ignore_ascii_case("img") || data.tag_name.eq_ignore_ascii_case("script") {
        let src = data.attr("src")?.trim();
        return (!src.is_empty()).then_some(src);
    }
    None
}

fn is_data_uri(uri: &str) -> bool {
    uri.trim()
        .as_bytes()
        .get(..5)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"data:"))
}
