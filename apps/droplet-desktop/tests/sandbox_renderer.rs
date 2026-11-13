use std::path::Path;
use std::process::Command;

use droplet_desktop::renderer_host::RendererHost;
use droplet_ipc::FetchedAsset;

fn droplet_bin() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_droplet"))
}

#[test]
fn sandboxed_renderer_child_renders_cyrillic_dom_mutation_and_seccomp() {
    let mut host = RendererHost::spawn_with_binary(droplet_bin(), 480, 320)
        .expect("spawn sandboxed --renderer child");

    #[cfg(target_os = "linux")]
    assert!(
        host.sandbox_info().seccomp_applied,
        "expected Linux seccomp-bpf to be active in --renderer child"
    );

    let html = r#"<!DOCTYPE html>
<html>
  <head>
    <meta charset="utf-8">
    <title>Начало</title>
    <link rel="stylesheet" href="/app.css">
  </head>
  <body>
    <div id="hero">Привет, песочница!</div>
    <script>
      document.title = "Готово Привет";
      console.log("sandbox-js-ok");
    </script>
  </body>
</html>"#;

    let assets = vec![FetchedAsset {
        url: "https://example.local/app.css".to_string(),
        bytes: b"body { margin: 0; } #hero { background: #1d4ed8; color: #ffffff; width: 240px; height: 90px; }".to_vec(),
    }];

    let frame = host
        .render_page("https://example.local/index.html", html, assets, 0.0)
        .expect("render pre-fetched page in sandboxed child");

    assert_eq!(frame.width, 480);
    assert_eq!(frame.height, 320);
    assert_eq!(frame.title, "Готово Привет");
    assert!(
        frame.console_logs.iter().any(|l| l == "sandbox-js-ok"),
        "expected sandboxed JS console log, got: {:?}",
        frame.console_logs
    );

    // #1d4ed8 -> 0x001d_4ed8 in XRGB framebuffer
    let has_hero_blue = host.pixels().iter().any(|&px| (px & 0x00FF_FFFF) == 0x001D_4ED8);
    assert!(
        has_hero_blue,
        "expected rendered #hero box (#1d4ed8) in shared memory framebuffer"
    );

    let after_eval = host
        .eval_js("document.title = 'JS Eval Тест';")
        .expect("eval_js inside sandboxed child");
    assert_eq!(after_eval.title, "JS Eval Тест");

    host.shutdown().expect("clean shutdown");
}

#[test]
fn sandboxed_renderer_host_handles_child_crash_gracefully() {
    let mut host = RendererHost::spawn_with_binary(droplet_bin(), 320, 240)
        .expect("spawn sandboxed --renderer child");
    host.force_kill_child().expect("kill child");

    let err = host
        .scroll(24.0)
        .expect_err("command after child termination should return Err instead of panicking");
    assert!(!err.is_empty());
}

#[test]
fn cli_sandbox_screenshot_writes_valid_ppm() {
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("droplet-sandbox-test-{}.ppm", std::process::id()));
    let _ = std::fs::remove_file(&out_path);

    let output = Command::new(droplet_bin())
        .arg("--sandbox-screenshot")
        .arg(&out_path)
        .arg("about:droplet")
        .output()
        .expect("run droplet --sandbox-screenshot");

    assert!(
        output.status.success(),
        "droplet --sandbox-screenshot failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Ready("),
        "expected Ready(...) sandbox report in stdout, got: {stdout}"
    );
    #[cfg(target_os = "linux")]
    assert!(
        stdout.contains("seccomp_applied=true"),
        "expected seccomp_applied=true in stdout, got: {stdout}"
    );

    let bytes = std::fs::read(&out_path).expect("read generated ppm");
    assert!(bytes.starts_with(b"P6\n1024 768\n255\n"));
    let _ = std::fs::remove_file(&out_path);
}
