# DROP-004: System TTF/OTF Fonts, Glyph Atlas Cache, Image Decoding & `<img>` Boxes

- **Phase:** Phase 4 (Ready)
- **Status:** `ready-for-agent`
- **Blocked By:** `DROP-001` (`done`)

---

## Goal

Upgrade Droplet from the built-in 8x16 bitmap font and text-only pages to crisp anti-aliased system TrueType/OpenType (`TTF`/`OTF`) typography backed by a fast Glyph Atlas Cache, decode `PNG`, `JPEG`, `WebP`, and basic `SVG` images, and render `<img>` tags as replaced layout boxes.

---

## Files to Edit

1. **`src/font.rs`**
   - Discover Ubuntu system fonts from `/usr/share/fonts/truetype`, `/usr/share/fonts/opentype`, and `~/.local/share/fonts` (preferring fonts with both ASCII and Cyrillic coverage such as `DejaVuSans`, `Ubuntu`, `LiberationSans`, with bold variants; fall back cleanly to the built-in 8x16 bitmap font if no system TTF/OTF is present).
   - Implement `GlyphAtlas` cache keyed by `(font_id, ch, px_size_tenths, bold)` storing anti-aliased alpha coverage bitmaps and advance/bearing metrics.
   - Update `measure_text` and glyph metrics so `layout.rs` wraps text using real proportional glyph advances instead of fixed `8 * scale` width.

2. **`src/image.rs` (new module exported from `src/lib.rs`)**
   - Define `DecodedImage { pub width: u32, pub height: u32, pub rgba: Vec<u8> }`.
   - Sniff image format by magic bytes (`PNG`, `JPEG`, `WebP`, `<svg`) and decode into RGBA pixels (keep dependency footprint minimal and compile time fast).
   - Support basic static `SVG` rendering (`viewBox`, `<rect>`, `<circle>`, `<line>`, `<polygon>`, `<path>` fill/stroke).

3. **`src/net.rs`**
   - Add binary fetch helper `pub fn fetch_bytes(url: &Url) -> Result<(String, Vec<u8>), NetError>` supporting `file://` and `http://` (plus `https://` if `DROP-003` is complete, and `data:` URIs if simple).

4. **`src/layout.rs`**
   - Add replaced `<img>` box handling (`BoxType::Image` or `Replaced` variant on `LayoutBox`) using intrinsic image dimensions:
     - Respect explicit CSS/HTML `width` and `height` attributes.
     - If only `width` OR `height` is given, preserve aspect ratio `(intrinsic_w / intrinsic_h)`.
     - If image fails to load, render a compact fallback box displaying the `alt` attribute text.

5. **`src/paint.rs`**
   - Render `DisplayCommand::Text` using `GlyphAtlas` alpha blending (`blend_alpha_mask`) onto `Canvas` for crisp sub-pixel/anti-aliased text over any background color.
   - Add `DisplayCommand::Image { rect: Rect, image: Arc<DecodedImage> }` and blit/scale RGBA images cleanly (nearest/bilinear) clipped to viewport.

---

## Edge Cases to Handle

- **Dimension Bomb Protection:** Reject malformed PNG/JPEG/WebP headers claiming absurd dimensions (e.g., `width * height > 64_000_000` pixels) before allocating `Vec<u8>`.
- **Missing Cyrillic Glyphs in Chosen Font:** If the selected primary font lacks Cyrillic codepoints (`А-Я`, `а-я`, `Ё`, `ё`), fall back per-glyph to a secondary system font with Cyrillic coverage or the built-in 8x16 bitmap glyph—never draw blank tofu squares when a fallback exists.
- **Corrupt / 404 Image:** Display a bordered placeholder frame with `alt` text (or broken-image marker) without panicking or blocking page rendering.
- **Alpha Transparency:** Properly composite semi-transparent PNG/WebP/SVG pixels and anti-aliased text glyphs over arbitrary background colors (no black halos from wrong premultiplication).
- **Zero-Allocation Hot Path:** Repeated frames during scrolling must hit `GlyphAtlas` in O(1) without re-rasterizing glyphs.

---

## Acceptance Tests

1. **Font Discovery & Glyph Atlas Tests (`src/font.rs`):**
   - Atlas caches `(font_id, 'Ж', 160, false)` and returns identical bitmap + non-zero advance width on cache hit.
   - Fallback renders valid bitmap even when font path list is empty.
2. **Image Decoding & Safety Tests (`src/image.rs`):**
   - Embedded minimal `PNG`, `JPEG`, `WebP`, and `<svg>` fixtures decode to exact expected `width`, `height`, and RGBA sample pixels.
   - Corrupted bytes and oversized dimension header return `Err` cleanly without panic or OOM.
3. **Layout & Paint Tests (`src/layout.rs`, `src/paint.rs`):**
   - `<img src="..." width="100">` on a `200x100` image computes layout box `100 x 50` (aspect ratio preserved).
   - Missing image falls back to `alt` text box.
   - Semi-transparent RGBA pixel blends smoothly onto non-white canvas background.
4. **End-to-End Verification:**
   - `/home/mrevds/.cargo/bin/cargo test`
   - `/home/mrevds/.cargo/bin/cargo clippy --all-targets`
   - `/home/mrevds/.cargo/bin/cargo run -- --screenshot /tmp/droplet-p4.ppm about:droplet`
