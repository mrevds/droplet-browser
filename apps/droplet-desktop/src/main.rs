use std::path::Path;
use std::process;

use droplet_dom::html;
use droplet_dom::Document;
#[cfg(feature = "sandbox")]
use droplet_desktop::{renderer_child, renderer_host};
use droplet_shell::Browser;
use droplet_url::Url;

const DEFAULT_URL: &str = "about:droplet";
const HEADLESS_VIEWPORT_WIDTH: usize = 1024;
const HEADLESS_VIEWPORT_HEIGHT: usize = 768;

fn print_usage() {
    println!(
        "Droplet — lightweight graphical web browser engine\n\n\
         USAGE:\n\
         \tdroplet [URL]\n\
         \tdroplet --screenshot <out.ppm> [URL]\n\
         \tdroplet --dump-dom [URL]\n\
         \tdroplet --dump-layout [URL]\n\
         \tdroplet --bench [--runs N] [URL]\n\
         \tdroplet --help | -h\n\n\
         GLOBAL OPTIONS:\n\
         \t--js    enable the experimental ES-subset JavaScript VM (off by default)\n\n\
         EXPERIMENTAL (build with --features sandbox):\n\
         \tdroplet --sandbox-screenshot <out.ppm> [URL]\n\
         \tdroplet --renderer [--renderer-w <W> --renderer-h <H>]\n\n\
         BUILT-IN PAGES:\n\
         \tabout:droplet, about:roadmap, about:architecture, about:test, about:blank"
    );
}

fn load_document(url_str: &str) -> Result<(Url, Document), String> {
    let url = Url::parse(url_str).map_err(|e| format!("Invalid URL '{url_str}': {e}"))?;
    let resp =
        droplet_net::fetch(&url).map_err(|e| format!("Failed to fetch '{}': {e}", url.as_str()))?;
    Ok((resp.url, html::parse(&resp.body)))
}

fn browser_for(url: &str) -> Browser {
    Browser::new(url, HEADLESS_VIEWPORT_WIDTH, HEADLESS_VIEWPORT_HEIGHT)
}

/// Peak resident set size of this process in KiB (Linux only).
fn peak_rss_kib() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
        line.split_whitespace().nth(1)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn run_bench(args: &[String]) -> Result<String, String> {
    let mut runs = 20usize;
    let mut url = DEFAULT_URL;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--runs" => {
                let value = args.get(i + 1).ok_or("--runs requires a number")?;
                runs = value
                    .parse()
                    .map_err(|_| format!("invalid --runs value '{value}'"))?;
                i += 2;
            }
            other if other.starts_with('-') => return Err(format!("unknown bench option '{other}'")),
            other => {
                url = other;
                i += 1;
            }
        }
    }
    let parsed = Url::parse(url).map_err(|e| format!("Invalid URL '{url}': {e}"))?;
    let report = droplet_shell::bench::bench_url(
        &parsed,
        runs,
        HEADLESS_VIEWPORT_WIDTH,
        HEADLESS_VIEWPORT_HEIGHT,
    )?;
    let rss = peak_rss_kib().map_or_else(|| "n/a".to_string(), |kib| kib.to_string());
    Ok(format!("{} peak_rss_kib={rss}", report.to_line()))
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(pos) = args.iter().position(|a| a == "--js") {
        args.remove(pos);
        droplet_shell::set_js_default(true);
    }

    if args.is_empty() {
        run_gui(DEFAULT_URL);
        return;
    }

    match args[0].as_str() {
        "--help" | "-h" => {
            print_usage();
        }
        "--dump-dom" => {
            if args.len() > 2 {
                eprintln!("Error: unexpected extra arguments for --dump-dom");
                print_usage();
                process::exit(2);
            }
            let url = args.get(1).map(String::as_str).unwrap_or(DEFAULT_URL);
            match load_document(url) {
                Ok((_, doc)) => print!("{}", doc.dump_tree()),
                Err(err) => {
                    eprintln!("Error: {err}");
                    process::exit(1);
                }
            }
        }
        "--dump-layout" => {
            if args.len() > 2 {
                eprintln!("Error: unexpected extra arguments for --dump-layout");
                print_usage();
                process::exit(2);
            }
            let url = args.get(1).map(String::as_str).unwrap_or(DEFAULT_URL);
            print!("{}", browser_for(url).active_layout_dump());
        }
        "--bench" => match run_bench(&args[1..]) {
            Ok(line) => println!("{line}"),
            Err(err) => {
                eprintln!("Error: {err}");
                process::exit(1);
            }
        },
        "--screenshot" => {
            if args.len() < 2 || args.len() > 3 {
                eprintln!("Error: --screenshot requires <out.ppm> and optional [URL]");
                print_usage();
                process::exit(2);
            }
            let out_path = Path::new(&args[1]);
            let url = args.get(2).map(String::as_str).unwrap_or(DEFAULT_URL);
            let mut browser = browser_for(url);
            let canvas = browser.render();
            if let Err(err) = canvas.write_ppm(out_path) {
                eprintln!(
                    "Error: failed to write screenshot '{}': {err}",
                    out_path.display()
                );
                process::exit(1);
            }
            println!(
                "Saved {}x{} screenshot to {}",
                canvas.width,
                canvas.height,
                out_path.display()
            );
        }
        #[cfg(feature = "sandbox")]
        "--sandbox-screenshot" => {
            if args.len() < 2 || args.len() > 3 {
                eprintln!("Error: --sandbox-screenshot requires <out.ppm> and optional [URL]");
                print_usage();
                process::exit(2);
            }
            let out_path = Path::new(&args[1]);
            let url = args.get(2).map(String::as_str).unwrap_or(DEFAULT_URL);
            match run_sandbox_screenshot(out_path, url) {
                Ok((sandbox, frame)) => {
                    println!(
                        "Ready(seccomp_applied={}, user_ns_isolated={}, net_ns_isolated={})",
                        sandbox.seccomp_applied,
                        sandbox.user_ns_isolated,
                        sandbox.net_ns_isolated
                    );
                    println!(
                        "Saved {}x{} sandboxed screenshot to {} (title={:?}, layout_ms={:.2}, paint_ms={:.2})",
                        frame.width,
                        frame.height,
                        out_path.display(),
                        frame.title,
                        frame.layout_ms,
                        frame.paint_ms
                    );
                }
                Err(err) => {
                    eprintln!("Error: {err}");
                    process::exit(1);
                }
            }
        }
        #[cfg(feature = "sandbox")]
        "--renderer" => {
            let (w, h) = match renderer_child::parse_renderer_dimensions(&args[1..]) {
                Ok(dims) => dims,
                Err(err) => {
                    eprintln!("Error: {err}");
                    process::exit(2);
                }
            };
            if let Err(err) = renderer_child::run_renderer_child(w, h) {
                eprintln!("Renderer child error: {err}");
                process::exit(1);
            }
        }
        flag if flag.starts_with('-') => {
            eprintln!("Error: unknown option '{flag}'");
            print_usage();
            process::exit(2);
        }
        url => {
            if args.len() > 1 {
                eprintln!("Error: unexpected extra arguments");
                print_usage();
                process::exit(2);
            }
            run_gui(url);
        }
    }
}

#[cfg(feature = "sandbox")]
fn run_sandbox_screenshot(
    out_path: &Path,
    url: &str,
) -> Result<(renderer_host::RendererSandboxInfo, renderer_host::RemoteFrame), String> {
    let prefetched = renderer_host::prefetch_page_payload(url)?;
    let mut host =
        renderer_host::RendererHost::spawn(HEADLESS_VIEWPORT_WIDTH, HEADLESS_VIEWPORT_HEIGHT)?;
    let sandbox = host.sandbox_info();
    let frame = host.render_page(&prefetched.url, &prefetched.html, prefetched.assets, 0.0)?;
    let canvas = host.snapshot_canvas();
    canvas.write_ppm(out_path).map_err(|err| {
        format!(
            "failed to write sandboxed screenshot '{}': {err}",
            out_path.display()
        )
    })?;
    host.shutdown()?;
    Ok((sandbox, frame))
}

fn run_gui(url: &str) {
    // The window opens at once; the page loads in the background (DROP-110).
    let browser = Browser::new_deferred(url, 1140, 780);
    if let Err(err) = droplet_platform::run(browser) {
        eprintln!("Could not start graphical window: {err}");
        eprintln!("Tip: if no display server is attached, you can run Droplet headlessly using:");
        eprintln!("  droplet --screenshot out.ppm {url}");
        eprintln!("  droplet --dump-dom {url}");
        eprintln!("  droplet --dump-layout {url}");
        process::exit(1);
    }
}
