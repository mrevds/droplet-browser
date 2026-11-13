mod discovery;
mod fallback;
mod rasterizer;
mod sfnt;

use std::collections::HashMap;
use std::sync::Arc;

use discovery::{system_font_set, FontSet, FontSlot};
use fallback::rasterize_fallback_glyph;

pub use fallback::{glyph_bitmap, measure_text};

pub const GLYPH_WIDTH: usize = fallback::GLYPH_WIDTH;
pub const GLYPH_HEIGHT: usize = fallback::GLYPH_HEIGHT;

const MAX_ATLAS_ENTRIES: usize = 4096;

/// Rasterized anti-aliased glyph bitmap with layout metrics.
#[derive(Debug, Clone)]
pub struct RasterGlyph {
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    /// Offset in pixels from the top of the line box (`y + bearing_y`).
    pub bearing_y: i32,
    pub advance: f32,
    /// Alpha coverage `0..=255`, length `width * height`.
    pub alpha: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphCacheKey {
    font_slot: u8,
    ch: char,
    px_tenths: u32,
    bold_synth: bool,
}

/// Fast read-only proportional text measurement using the discovered system fonts (or 8x16 fallback).
pub fn measure_text_px(text: &str, font_size: f32, bold: bool, mono: bool) -> f32 {
    system_font_set().measure_text(text, font_size, bold, mono)
}

/// Fast read-only line height calculation using discovered system fonts (or 8x16 fallback).
pub fn line_height_px(font_size: f32) -> f32 {
    system_font_set().line_height(font_size)
}

/// Offset from the top of the line box to the typographic baseline in pixels.
pub fn baseline_offset_px(font_size: f32) -> f32 {
    system_font_set().baseline_offset(font_size)
}

/// Font rasterizer engine with system TTF discovery, pure-Rust glyph rasterization,
/// 8x16 bitmap fallback, and a bounded glyph atlas cache.
pub struct FontEngine {
    fonts: Arc<FontSet>,
    atlas: HashMap<GlyphCacheKey, RasterGlyph>,
}

impl Default for FontEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FontEngine {
    /// Create a `FontEngine` backed by discovered system TrueType fonts (with 8x16 fallback).
    pub fn new() -> Self {
        Self {
            fonts: system_font_set(),
            atlas: HashMap::new(),
        }
    }

    /// Creates a fallback-only FontEngine (for testing fallback deterministically).
    pub fn fallback_only() -> Self {
        Self {
            fonts: Arc::new(FontSet::empty()),
            atlas: HashMap::new(),
        }
    }

    pub fn has_system_font(&self) -> bool {
        self.fonts.has_any()
    }

    pub fn line_height(&self, font_size: f32) -> f32 {
        self.fonts.line_height(font_size)
    }

    pub fn measure_text(&self, text: &str, font_size: f32, bold: bool, mono: bool) -> f32 {
        self.fonts.measure_text(text, font_size, bold, mono)
    }

    pub fn rasterize_glyph(
        &mut self,
        ch: char,
        font_size: f32,
        bold: bool,
        mono: bool,
    ) -> &RasterGlyph {
        let fs = if font_size.is_finite() && font_size > 0.0 {
            font_size.clamp(1.0, 512.0)
        } else {
            0.0
        };
        let px_tenths = (fs * 10.0).round() as u32;
        let slot = FontSet::slot_for(bold, mono);
        let (_, _, bold_synth) = self.fonts.resolve_for_char(ch, slot);
        let key = GlyphCacheKey {
            font_slot: slot as u8,
            ch,
            px_tenths,
            bold_synth,
        };

        if !self.atlas.contains_key(&key) {
            if self.atlas.len() >= MAX_ATLAS_ENTRIES {
                self.atlas.clear();
            }
            let glyph = if fs <= 0.0 {
                RasterGlyph {
                    width: 0,
                    height: 0,
                    bearing_x: 0,
                    bearing_y: 0,
                    advance: 0.0,
                    alpha: Vec::new(),
                }
            } else {
                self.render_uncached(ch, fs, slot)
            };
            self.atlas.insert(key, glyph);
        }

        self.atlas
            .get(&key)
            .expect("cached glyph must exist immediately after insertion")
    }

    fn render_uncached(&self, ch: char, font_size: f32, slot: FontSlot) -> RasterGlyph {
        if matches!(ch, '\n' | '\r') {
            return RasterGlyph {
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
                advance: 0.0,
                alpha: Vec::new(),
            };
        }
        let (font_opt, gid, bold_synth) = self.fonts.resolve_for_char(ch, slot);
        if let Some(font) = font_opt {
            if let Some(glyph) = font.rasterize_glyph(gid, font_size, bold_synth) {
                return glyph;
            }
        }
        rasterize_fallback_glyph(ch, font_size, bold_synth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fallback::GLYPH_FALLBACK;
    use crate::sfnt::TrueTypeFont;

    fn is_non_empty(glyph: &[u8; 16]) -> bool {
        glyph.iter().any(|&row| row != 0)
    }

    #[test]
    fn ascii_printable_non_empty_and_distinct() {
        assert!(!is_non_empty(glyph_bitmap(' ')));
        for code in 0x21u8..=0x7Eu8 {
            let ch = code as char;
            let bm = glyph_bitmap(ch);
            assert!(
                is_non_empty(bm),
                "ASCII glyph for {:?} (0x{:02X}) must not be blank",
                ch,
                code
            );
            assert_ne!(
                bm as *const [u8; 16],
                &GLYPH_FALLBACK as *const [u8; 16],
                "ASCII glyph {:?} fell back to unknown glyph",
                ch
            );
        }
    }

    #[test]
    fn cyrillic_full_alphabet_non_empty_and_distinct_from_fallback() {
        let cyrillic_upper: Vec<char> = "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ".chars().collect();
        let cyrillic_lower: Vec<char> = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя".chars().collect();
        assert_eq!(cyrillic_upper.len(), 33);
        assert_eq!(cyrillic_lower.len(), 33);

        for ch in cyrillic_upper.into_iter().chain(cyrillic_lower) {
            let bm = glyph_bitmap(ch);
            assert!(is_non_empty(bm), "Cyrillic glyph {:?} must not be blank", ch);
            assert_ne!(
                bm as *const [u8; 16],
                &GLYPH_FALLBACK as *const [u8; 16],
                "Cyrillic glyph {:?} fell back to unknown glyph",
                ch
            );
        }

        // Visually distinct pairs must have distinct bitmaps.
        assert_ne!(glyph_bitmap('Ш'), glyph_bitmap('Щ'));
        assert_ne!(glyph_bitmap('ш'), glyph_bitmap('щ'));
        assert_ne!(glyph_bitmap('Е'), glyph_bitmap('Ё'));
        assert_ne!(glyph_bitmap('е'), glyph_bitmap('ё'));
        assert_ne!(glyph_bitmap('И'), glyph_bitmap('Й'));
        assert_ne!(glyph_bitmap('и'), glyph_bitmap('й'));
        assert_ne!(glyph_bitmap('Ъ'), glyph_bitmap('Ь'));
        assert_ne!(glyph_bitmap('ъ'), glyph_bitmap('ь'));
        assert_ne!(glyph_bitmap('И'), glyph_bitmap('Н'));
    }

    #[test]
    fn symbols_and_fallback() {
        for ch in ['«', '»', '—', '–', '→', '←', '•'] {
            let bm = glyph_bitmap(ch);
            assert!(is_non_empty(bm), "Symbol {:?} must not be blank", ch);
            assert_ne!(
                bm as *const [u8; 16],
                &GLYPH_FALLBACK as *const [u8; 16],
                "Symbol {:?} fell back to unknown glyph",
                ch
            );
        }

        assert_eq!(
            glyph_bitmap('\u{2603}') as *const [u8; 16],
            &GLYPH_FALLBACK as *const [u8; 16]
        );
        assert!(is_non_empty(glyph_bitmap('\u{2603}')));
    }

    #[test]
    fn measure_text_handles_ascii_and_utf8_cyrillic() {
        assert_eq!(measure_text("", 1), 0.0);
        assert_eq!(measure_text("Hello", 1), 40.0);
        assert_eq!(measure_text("Привет", 1), 48.0);
        assert_eq!(measure_text("Привет!", 2), 7.0 * 16.0);
        assert_eq!(measure_text("ab", 0), 0.0);
    }

    #[test]
    fn system_font_engine_rasterizes_ascii_cyrillic_and_compounds() {
        let mut engine = FontEngine::new();
        assert!(
            engine.has_system_font(),
            "Ubuntu/Linux test environment must discover system TTF fonts"
        );
        assert!(engine.line_height(16.0) >= 16.0);
        assert!(line_height_px(16.0) >= 16.0);

        for ch in ['A', 'Ж', 'я', 'i', 'W', 'Ё'] {
            let g = engine.rasterize_glyph(ch, 16.0, false, false).clone();
            assert!(g.width > 0 && g.height > 0, "glyph {:?} must have non-zero size", ch);
            assert_eq!(g.alpha.len(), (g.width * g.height) as usize);
            assert!(
                g.alpha.iter().any(|&a| a > 0),
                "glyph {:?} must have non-empty alpha coverage",
                ch
            );
            assert!(
                g.alpha.iter().any(|&a| a > 0 && a < 255),
                "glyph {:?} should produce anti-aliased edge coverage",
                ch
            );
        }

        let adv_i = engine.rasterize_glyph('i', 16.0, false, false).advance;
        let adv_w = engine.rasterize_glyph('W', 16.0, false, false).advance;
        assert!(
            adv_i < adv_w,
            "proportional advance('i')={} must be < advance('W')={}",
            adv_i,
            adv_w
        );

        let mono_i = engine.rasterize_glyph('i', 16.0, false, true).advance;
        let mono_w = engine.rasterize_glyph('W', 16.0, false, true).advance;
        assert!(
            (mono_i - mono_w).abs() < 0.01,
            "monospace advances must match: {} vs {}",
            mono_i,
            mono_w
        );

        assert!(
            measure_text_px("WWW", 16.0, false, false)
                > measure_text_px("iii", 16.0, false, false)
        );
        assert!(
            (engine.measure_text("Привет!", 16.0, false, false)
                - measure_text_px("Привет!", 16.0, false, false))
            .abs()
                < 0.001
        );
    }

    #[test]
    fn bold_and_cache_hit_and_bounded_eviction() {
        let mut engine = FontEngine::new();
        let reg_sum: u64 = engine
            .rasterize_glyph('A', 20.0, false, false)
            .alpha
            .iter()
            .map(|&a| a as u64)
            .sum();
        let bold_sum: u64 = engine
            .rasterize_glyph('A', 20.0, true, false)
            .alpha
            .iter()
            .map(|&a| a as u64)
            .sum();
        assert!(bold_sum > reg_sum, "bold glyph must have greater ink coverage");

        let ptr1 = engine.rasterize_glyph('Ж', 18.0, false, false) as *const RasterGlyph;
        let len_before = engine.atlas.len();
        let ptr2 = engine.rasterize_glyph('Ж', 18.0, false, false) as *const RasterGlyph;
        assert_eq!(ptr1, ptr2, "cache hit must return exact cached RasterGlyph");
        assert_eq!(engine.atlas.len(), len_before);

        // Insert > MAX_ATLAS_ENTRIES distinct cache entries and ensure cache stays bounded.
        for size_step in 0..45u32 {
            let size = 10.0 + (size_step as f32) * 0.2;
            for code in 0x21u8..=0x7Eu8 {
                let _ = engine.rasterize_glyph(code as char, size, false, false);
            }
        }
        assert!(engine.atlas.len() <= MAX_ATLAS_ENTRIES);
    }

    #[test]
    fn fallback_only_engine_and_malformed_ttf_safe() {
        let mut fb = FontEngine::fallback_only();
        assert!(!fb.has_system_font());
        assert_eq!(fb.measure_text("Hello", 16.0, false, false), 40.0);
        assert_eq!(fb.line_height(16.0), 20.0);

        let g = fb.rasterize_glyph('Ж', 16.0, false, false).clone();
        assert_eq!(g.width, 8);
        assert_eq!(g.height, 16);
        assert_eq!(g.advance, 8.0);
        assert!(g.alpha.contains(&255));

        let g_sp = fb.rasterize_glyph(' ', 16.0, false, false).clone();
        assert_eq!(g_sp.width, 0);
        assert_eq!(g_sp.advance, 8.0);

        // Malformed TTF inputs must return Err cleanly without panicking.
        assert!(TrueTypeFont::parse(Vec::new()).is_err());
        assert!(TrueTypeFont::parse(b"OTTO\x00\x01\x00\x00\x00\x00\x00\x00".to_vec()).is_err());
        assert!(TrueTypeFont::parse(vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x50, 0x00, 0x00]).is_err());
    }
}
