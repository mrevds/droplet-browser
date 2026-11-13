# Rendering corpus

Small hand-written pages, one feature area each. The top comment of every page (`EXPECT:`)
says what a correct rendering looks like.

- `expected/*.layout` — layout goldens (`droplet --dump-layout` with `DROPLET_FONTS=builtin`,
  so metrics are identical on every machine). They detect **change**, not correctness.
- `manifest.txt` — human-reviewed status (`ok` / `wrong`) and the phase that owns the fix.
  This is the honest scoreboard: a phase is done when its pages are `ok`.

```bash
# check goldens
/home/mrevds/.cargo/bin/cargo test -p droplet-desktop --test corpus
# after an intended layout change: review screenshots, then re-bless
DROPLET_BLESS=1 /home/mrevds/.cargo/bin/cargo test -p droplet-desktop --test corpus
# look at one page
/home/mrevds/.cargo/bin/cargo run --release -- --screenshot /tmp/p.ppm file://$PWD/tests/corpus/04-table.html
```

Real-site smoke + performance numbers: `tools/bench-sites.sh` (network, not run in CI).
