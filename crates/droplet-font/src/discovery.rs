use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crate::fallback::{GLYPH_HEIGHT, GLYPH_WIDTH};
use crate::rasterizer::synthetic_bold_extra_px;
use crate::sfnt::TrueTypeFont;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FontSlot {
    Regular = 0,
    Bold = 1,
    MonoRegular = 2,
    MonoBold = 3,
}

#[derive(Clone)]
pub(crate) struct FontSet {
    regular: Option<Arc<TrueTypeFont>>,
    bold: Option<(Arc<TrueTypeFont>, bool)>,
    mono_regular: Option<Arc<TrueTypeFont>>,
    mono_bold: Option<(Arc<TrueTypeFont>, bool)>,
    fallback: Vec<Arc<TrueTypeFont>>,
}

impl FontSet {
    pub(crate) fn empty() -> Self {
        Self {
            regular: None,
            bold: None,
            mono_regular: None,
            mono_bold: None,
            fallback: Vec::new(),
        }
    }

    fn discover() -> Self {
        let regular_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-R.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuSans[wdth,wght].ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
        ];
        let bold_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-B.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            r"C:\Windows\Fonts\segoeuib.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
        ];
        let mono_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuSansMono[wght].ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            r"C:\Windows\Fonts\consola.ttf",
            r"C:\Windows\Fonts\cour.ttf",
        ];
        let mono_bold_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/ubuntu/UbuntuMono-B.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
            r"C:\Windows\Fonts\consolab.ttf",
            r"C:\Windows\Fonts\courbd.ttf",
        ];
        let fallback_candidates: &[&str] = &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
        ];

        let (regular, reg_canon) = load_first_ttf(regular_candidates);
        let bold = load_bold_variant(bold_candidates, &regular, reg_canon.as_ref());

        let (mono_regular, mono_canon) = load_first_ttf(mono_candidates);
        let mono_bold = load_bold_variant(mono_bold_candidates, &mono_regular, mono_canon.as_ref());

        let mut fallback = Vec::new();
        let mut seen_paths = Vec::new();
        if let Some(p) = reg_canon {
            seen_paths.push(p);
        }
        for path in fallback_candidates {
            if let Ok(canon) = std::fs::canonicalize(path) {
                if seen_paths.contains(&canon) {
                    continue;
                }
                if let Some(font) = load_ttf_file(path) {
                    seen_paths.push(canon);
                    fallback.push(font);
                }
            }
        }

        Self {
            regular,
            bold,
            mono_regular,
            mono_bold,
            fallback,
        }
    }

    pub(crate) fn has_any(&self) -> bool {
        self.regular.is_some()
            || self.bold.is_some()
            || self.mono_regular.is_some()
            || self.mono_bold.is_some()
    }

    pub(crate) fn slot_for(bold: bool, mono: bool) -> FontSlot {
        match (bold, mono) {
            (false, false) => FontSlot::Regular,
            (true, false) => FontSlot::Bold,
            (false, true) => FontSlot::MonoRegular,
            (true, true) => FontSlot::MonoBold,
        }
    }

    fn resolve_slot(&self, slot: FontSlot) -> (Option<&Arc<TrueTypeFont>>, bool) {
        match slot {
            FontSlot::Regular => (self.regular.as_ref(), false),
            FontSlot::Bold => match &self.bold {
                Some((f, synth)) => (Some(f), *synth),
                None => (self.regular.as_ref(), true),
            },
            FontSlot::MonoRegular => (self.mono_regular.as_ref().or(self.regular.as_ref()), false),
            FontSlot::MonoBold => match &self.mono_bold {
                Some((f, synth)) => (Some(f), *synth),
                None => (self.mono_regular.as_ref().or(self.regular.as_ref()), true),
            },
        }
    }

    pub(crate) fn resolve_for_char(
        &self,
        ch: char,
        slot: FontSlot,
    ) -> (Option<&Arc<TrueTypeFont>>, u16, bool) {
        let (primary, bold_synth) = self.resolve_slot(slot);
        let wants_bold = matches!(slot, FontSlot::Bold | FontSlot::MonoBold);
        let lookup_ch = if ch == '\u{00A0}' { ' ' } else { ch };

        if let Some(font) = primary {
            let gid = font.glyph_index(lookup_ch);
            if gid != 0 {
                return (Some(font), gid, bold_synth);
            }
        }
        for fb in &self.fallback {
            let gid = fb.glyph_index(lookup_ch);
            if gid != 0 {
                return (Some(fb), gid, wants_bold);
            }
        }
        (None, 0, wants_bold)
    }

    pub(crate) fn line_height(&self, font_size: f32) -> f32 {
        if !font_size.is_finite() || font_size <= 0.0 {
            return 0.0;
        }
        let fs = font_size.min(512.0);
        if let Some(font) = self.regular.as_ref().or(self.mono_regular.as_ref()) {
            font.line_height(fs)
        } else {
            fs * 1.25
        }
    }

    pub(crate) fn baseline_offset(&self, font_size: f32) -> f32 {
        if !font_size.is_finite() || font_size <= 0.0 {
            return 0.0;
        }
        let fs = font_size.min(512.0);
        if let Some(font) = self.regular.as_ref().or(self.mono_regular.as_ref()) {
            font.baseline_from_top_f32(fs)
        } else {
            fs * 0.85
        }
    }

    pub(crate) fn measure_text(&self, text: &str, font_size: f32, bold: bool, mono: bool) -> f32 {
        if !font_size.is_finite() || font_size <= 0.0 || text.is_empty() {
            return 0.0;
        }
        let fs = font_size.min(512.0);
        let slot = Self::slot_for(bold, mono);
        let mut total = 0.0f32;
        for ch in text.chars() {
            total += self.char_advance(ch, fs, slot);
        }
        total
    }

    fn char_advance(&self, ch: char, font_size: f32, slot: FontSlot) -> f32 {
        if matches!(ch, '\n' | '\r') {
            return 0.0;
        }
        let (font_opt, gid, bold_synth) = self.resolve_for_char(ch, slot);
        if let Some(font) = font_opt {
            let base = font.advance_px(gid, font_size);
            if bold_synth {
                base + synthetic_bold_extra_px(font_size)
            } else {
                base
            }
        } else {
            let scale = (font_size / GLYPH_HEIGHT as f32).max(0.25);
            let mut adv = GLYPH_WIDTH as f32 * scale;
            if bold_synth && ch != ' ' && ch != '\u{00A0}' {
                adv += 1.0;
            }
            adv
        }
    }
}

fn load_ttf_file(path: &str) -> Option<Arc<TrueTypeFont>> {
    let bytes = std::fs::read(path).ok()?;
    TrueTypeFont::parse(bytes).ok().map(Arc::new)
}

fn load_first_ttf(candidates: &[&str]) -> (Option<Arc<TrueTypeFont>>, Option<PathBuf>) {
    for path in candidates {
        if let Some(font) = load_ttf_file(path) {
            let canon = std::fs::canonicalize(path).ok();
            return (Some(font), canon);
        }
    }
    (None, None)
}

fn load_bold_variant(
    bold_candidates: &[&str],
    regular: &Option<Arc<TrueTypeFont>>,
    regular_canon: Option<&PathBuf>,
) -> Option<(Arc<TrueTypeFont>, bool)> {
    let first_path = bold_candidates.first().copied();
    if let (Some(reg), Some(reg_path), Some(p)) = (regular, regular_canon, first_path) {
        if let Ok(bold_canon) = std::fs::canonicalize(p) {
            if &bold_canon == reg_path {
                // Variable-font symlink (e.g. Ubuntu-B.ttf -> Ubuntu[wdth,wght].ttf):
                // keep the matching typeface and synthesize bold weight cleanly.
                return Some((Arc::clone(reg), true));
            }
        }
    }
    for path in bold_candidates {
        if let Ok(canon) = std::fs::canonicalize(path) {
            if regular_canon == Some(&canon) {
                if let Some(reg) = regular {
                    return Some((Arc::clone(reg), true));
                }
            }
        }
        if let Some(font) = load_ttf_file(path) {
            return Some((font, false));
        }
    }
    regular.as_ref().map(|reg| (Arc::clone(reg), true))
}

static SYSTEM_FONTS: OnceLock<Arc<FontSet>> = OnceLock::new();

pub(crate) fn system_font_set() -> Arc<FontSet> {
    Arc::clone(SYSTEM_FONTS.get_or_init(|| Arc::new(FontSet::discover())))
}
