#!/usr/bin/env bash
# Real-site smoke test + per-stage numbers. Needs network; not part of `cargo test`.
#   tools/bench-sites.sh            # default site list
#   tools/bench-sites.sh URL...     # custom list
# Prints one `droplet --bench` line per URL plus a JS-off screenshot path under $OUT.
set -u
CARGO=${CARGO:-/home/mrevds/.cargo/bin/cargo}
OUT=${OUT:-/tmp/droplet-bench}
RUNS=${RUNS:-10}
mkdir -p "$OUT"
"$CARGO" build --release -q -p droplet-desktop || exit 1
# .cargo/config.toml puts build output in /tmp/droplet-target unless CARGO_TARGET_DIR overrides it.
BIN=${CARGO_TARGET_DIR:-/tmp/droplet-target}/release/droplet

if [ "$#" -gt 0 ]; then SITES=("$@"); else SITES=(
  about:droplet
  about:test
  https://example.com
  https://news.ycombinator.com
  "https://en.wikipedia.org/wiki/Rust_(programming_language)"
  https://lite.duckduckgo.com/lite/
  https://www.rust-lang.org
); fi

i=0
for url in "${SITES[@]}"; do
  i=$((i + 1))
  if line=$(timeout 60 "$BIN" --bench --runs "$RUNS" "$url" 2>&1); then
    echo "$line"
  else
    echo "url=$url FAILED: $line"
  fi
  timeout 60 "$BIN" --screenshot "$OUT/site-$i.ppm" "$url" >/dev/null 2>&1 \
    && echo "  screenshot: $OUT/site-$i.ppm"
done
