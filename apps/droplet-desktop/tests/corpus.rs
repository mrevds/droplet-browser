//! Layout golden tests over `tests/corpus/*.html`.
//!
//! Each page is rendered with `DROPLET_FONTS=builtin` (machine-independent metrics) via
//! `droplet --dump-layout` and compared to `tests/corpus/expected/<name>.layout`.
//! A diff means layout changed: inspect it, then re-bless with
//! `DROPLET_BLESS=1 cargo test -p droplet-desktop --test corpus`.
//!
//! Goldens detect *change*, not correctness. Whether a page renders *correctly* is tracked
//! in `tests/corpus/manifest.txt` (`ok` / `wrong`), reviewed by a human from a screenshot.

use std::path::{Path, PathBuf};
use std::process::Command;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn pages() -> Vec<PathBuf> {
    let mut pages: Vec<PathBuf> = std::fs::read_dir(corpus_dir())
        .expect("tests/corpus exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "html"))
        .collect();
    pages.sort();
    pages
}

fn stem(page: &Path) -> String {
    page.file_stem().unwrap().to_string_lossy().into_owned()
}

fn dump_layout(page: &Path) -> String {
    let abs = page.canonicalize().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_droplet"))
        .env("DROPLET_FONTS", "builtin")
        .arg("--dump-layout")
        .arg(format!("file://{}", abs.display()))
        .output()
        .expect("run droplet --dump-layout");
    assert!(
        out.status.success(),
        "--dump-layout failed for {}: {}",
        page.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8 layout dump")
}

#[test]
fn corpus_layout_matches_goldens() {
    let bless = std::env::var_os("DROPLET_BLESS").is_some();
    let expected_dir = corpus_dir().join("expected");
    let mut failures = Vec::new();

    for page in pages() {
        let name = stem(&page);
        let actual = dump_layout(&page);
        let golden = expected_dir.join(format!("{name}.layout"));
        if bless {
            std::fs::write(&golden, &actual).unwrap();
            continue;
        }
        match std::fs::read_to_string(&golden) {
            Ok(expected) if expected == actual => {}
            Ok(expected) => {
                let line = expected
                    .lines()
                    .zip(actual.lines())
                    .position(|(a, b)| a != b)
                    .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
                failures.push(format!("{name}: layout differs from golden at line {}", line + 1));
            }
            Err(_) => failures.push(format!("{name}: missing golden {}", golden.display())),
        }
    }

    assert!(
        failures.is_empty(),
        "corpus layout changed (re-bless with DROPLET_BLESS=1 after reviewing):\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_corpus_page_has_a_reviewed_status() {
    let manifest = std::fs::read_to_string(corpus_dir().join("manifest.txt")).expect("manifest.txt");
    let mut missing = Vec::new();
    for page in pages() {
        let name = stem(&page);
        let entry = manifest
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .find(|l| l.split_whitespace().next() == Some(name.as_str()));
        match entry.and_then(|l| l.split_whitespace().nth(1)) {
            Some("ok") | Some("wrong") => {}
            _ => missing.push(name),
        }
    }
    assert!(missing.is_empty(), "pages without ok/wrong status in manifest.txt: {missing:?}");
}
