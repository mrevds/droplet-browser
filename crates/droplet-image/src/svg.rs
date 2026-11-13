//! Basic static SVG rasterizer (<svg>, <g>, <rect>, <circle>, <line>).

use crate::{validate_dimensions, DecodedImage, ImageError, MAX_IMAGE_PIXELS};

pub(crate) fn looks_like_svg(bytes: &[u8]) -> bool {
    let trimmed = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(trimmed) else {
        return false;
    };
    let s = skip_xml_preamble(text);
    starts_with_ascii_ci(s, "<svg")
        && s.as_bytes()
            .get(4)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
}

fn starts_with_ascii_ci(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let n = needle.len();
    if n == 0 {
        return Some(0);
    }
    if haystack.len() < n {
        return None;
    }
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    for i in 0..=(hb.len() - n) {
        if hb[i..i + n].eq_ignore_ascii_case(nb) {
            return Some(i);
        }
    }
    None
}

fn skip_xml_preamble(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if s.starts_with("<?") {
            if let Some(end) = s.find("?>") {
                s = &s[end + 2..];
                continue;
            }
            return "";
        }
        if s.starts_with("<!--") {
            if let Some(end) = s.find("-->") {
                s = &s[end + 3..];
                continue;
            }
            return "";
        }
        if starts_with_ascii_ci(s, "<!DOCTYPE") {
            if let Some(end) = s.find('>') {
                s = &s[end + 1..];
                continue;
            }
            return "";
        }
        return s;
    }
}

#[derive(Debug, Clone)]
struct SvgPaintState {
    fill: Option<[u8; 4]>,
    stroke: Option<[u8; 4]>,
    stroke_width: f32,
    opacity: f32,
    fill_opacity: f32,
    stroke_opacity: f32,
}

impl Default for SvgPaintState {
    fn default() -> Self {
        Self {
            fill: Some([0, 0, 0, 255]),
            stroke: None,
            stroke_width: 1.0,
            opacity: 1.0,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
        }
    }
}

impl SvgPaintState {
    fn apply_attrs(&mut self, attrs: &[(String, String)]) {
        if let Some(style) = find_attr(attrs, "style") {
            for decl in style.split(';') {
                if let Some((k, v)) = decl.split_once(':') {
                    self.apply_prop(k.trim(), v.trim());
                }
            }
        }
        for (k, v) in attrs {
            if !k.eq_ignore_ascii_case("style") {
                self.apply_prop(k, v);
            }
        }
    }

    fn apply_prop(&mut self, key: &str, val: &str) {
        if key.eq_ignore_ascii_case("fill") {
            if let Some(c) = parse_svg_paint(val) {
                self.fill = c;
            }
        } else if key.eq_ignore_ascii_case("stroke") {
            if let Some(c) = parse_svg_paint(val) {
                self.stroke = c;
            }
        } else if key.eq_ignore_ascii_case("stroke-width") {
            if let Some(w) = parse_svg_length(val) {
                self.stroke_width = w.max(0.0);
            }
        } else if key.eq_ignore_ascii_case("opacity") {
            if let Ok(a) = val.trim().parse::<f32>() {
                self.opacity = a.clamp(0.0, 1.0);
            }
        } else if key.eq_ignore_ascii_case("fill-opacity") {
            if let Ok(a) = val.trim().parse::<f32>() {
                self.fill_opacity = a.clamp(0.0, 1.0);
            }
        } else if key.eq_ignore_ascii_case("stroke-opacity") {
            if let Ok(a) = val.trim().parse::<f32>() {
                self.stroke_opacity = a.clamp(0.0, 1.0);
            }
        }
    }

    fn effective_fill(&self) -> Option<[u8; 4]> {
        self.fill.map(|mut c| {
            c[3] = ((c[3] as f32) * self.opacity * self.fill_opacity)
                .round()
                .clamp(0.0, 255.0) as u8;
            c
        })
    }

    fn effective_stroke(&self) -> Option<[u8; 4]> {
        if self.stroke_width <= 0.0 {
            return None;
        }
        self.stroke.map(|mut c| {
            c[3] = ((c[3] as f32) * self.opacity * self.stroke_opacity)
                .round()
                .clamp(0.0, 255.0) as u8;
            c
        })
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    let trimmed = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = std::str::from_utf8(trimmed)
        .map_err(|_| ImageError::new("SVG payload is not valid UTF-8"))?;
    let mut cursor = skip_xml_preamble(text);
    if !starts_with_ascii_ci(cursor, "<svg") {
        return Err(ImageError::new("expected <svg> root element"));
    }

    let (svg_tag, after_svg, root_self_closing) = parse_open_tag(cursor)?;
    if !svg_tag.name.eq_ignore_ascii_case("svg") {
        return Err(ImageError::new("expected <svg> root element"));
    }

    let vb = find_attr(&svg_tag.attrs, "viewBox")
        .and_then(parse_viewbox)
        .filter(|&(_, _, vw, vh)| vw > 0.0 && vh > 0.0);
    let parsed_w = find_attr(&svg_tag.attrs, "width").and_then(parse_svg_dim);
    let parsed_h = find_attr(&svg_tag.attrs, "height").and_then(parse_svg_dim);

    let (width_f, height_f) = match (parsed_w, parsed_h, vb) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some((_, _, vw, vh))) => (w, w * vh / vw),
        (None, Some(h), Some((_, _, vw, vh))) => (h * vw / vh, h),
        (None, None, Some((_, _, vw, vh))) => (vw, vh),
        (Some(w), None, None) => (w, 128.0),
        (None, Some(h), None) => (128.0, h),
        (None, None, None) => (128.0, 128.0),
    };

    if !width_f.is_finite() || !height_f.is_finite() || width_f <= 0.0 || height_f <= 0.0 {
        return Err(ImageError::new("invalid SVG dimensions"));
    }
    if width_f > (MAX_IMAGE_PIXELS as f32) || height_f > (MAX_IMAGE_PIXELS as f32) {
        return Err(ImageError::new("SVG dimensions exceed maximum pixel limit"));
    }
    let width = width_f.round().max(1.0) as u32;
    let height = height_f.round().max(1.0) as u32;
    let rgba_len = validate_dimensions(width, height)?;
    let mut rgba = vec![0u8; rgba_len];

    if root_self_closing {
        return Ok(DecodedImage {
            width,
            height,
            rgba,
        });
    }

    let (vb_x, vb_y, vb_w, vb_h) = match vb {
        Some((vx, vy, vw, vh)) if vw > 0.0 && vh > 0.0 => (vx, vy, vw, vh),
        _ => (0.0, 0.0, width as f32, height as f32),
    };
    let sx = width as f32 / vb_w;
    let sy = height as f32 / vb_h;
    let uniform_scale = (sx + sy) * 0.5;

    let mut root_state = SvgPaintState::default();
    root_state.apply_attrs(&svg_tag.attrs);
    let mut stack = vec![root_state];

    cursor = after_svg;
    while !cursor.is_empty() {
        let Some(lt) = cursor.find('<') else {
            break;
        };
        cursor = &cursor[lt..];
        if cursor.starts_with("<!--") {
            if let Some(end) = cursor.find("-->") {
                cursor = &cursor[end + 3..];
                continue;
            }
            break;
        }
        if cursor.starts_with("</") {
            let Some(gt) = cursor.find('>') else {
                break;
            };
            let close_name = cursor[2..gt].trim();
            if close_name.eq_ignore_ascii_case("svg") {
                break;
            }
            if close_name.eq_ignore_ascii_case("g") && stack.len() > 1 {
                stack.pop();
            }
            cursor = &cursor[gt + 1..];
            continue;
        }
        if cursor.starts_with("<?") || cursor.starts_with("<!") {
            if let Some(gt) = cursor.find('>') {
                cursor = &cursor[gt + 1..];
                continue;
            }
            break;
        }

        let (tag, next_cursor, self_closing) = parse_open_tag(cursor)?;
        cursor = next_cursor;

        if tag.name.eq_ignore_ascii_case("defs") || tag.name.eq_ignore_ascii_case("style") {
            if !self_closing {
                let close = if tag.name.eq_ignore_ascii_case("defs") {
                    "</defs>"
                } else {
                    "</style>"
                };
                if let Some(end) = find_ascii_ci(cursor, close) {
                    cursor = &cursor[end + close.len()..];
                }
            }
            continue;
        }

        let mut state = stack.last().cloned().unwrap_or_default();
        state.apply_attrs(&tag.attrs);

        if tag.name.eq_ignore_ascii_case("g") {
            if !self_closing {
                stack.push(state);
            }
            continue;
        }

        if tag.name.eq_ignore_ascii_case("rect") {
            let x = (attr_f32(&tag.attrs, "x", 0.0) - vb_x) * sx;
            let y = (attr_f32(&tag.attrs, "y", 0.0) - vb_y) * sy;
            let w = attr_f32(&tag.attrs, "width", 0.0) * sx;
            let h = attr_f32(&tag.attrs, "height", 0.0) * sy;
            if w > 0.0 && h > 0.0 {
                if let Some(fill) = state.effective_fill() {
                    raster_rect_fill(&mut rgba, width, height, [x, y, w, h], fill);
                }
                if let Some(stroke) = state.effective_stroke() {
                    let sw = state.stroke_width * uniform_scale;
                    raster_rect_stroke(&mut rgba, width, height, x, y, w, h, sw, stroke);
                }
            }
        } else if tag.name.eq_ignore_ascii_case("circle") {
            let cx = (attr_f32(&tag.attrs, "cx", 0.0) - vb_x) * sx;
            let cy = (attr_f32(&tag.attrs, "cy", 0.0) - vb_y) * sy;
            let r = attr_f32(&tag.attrs, "r", 0.0) * uniform_scale;
            if r > 0.0 {
                if let Some(fill) = state.effective_fill() {
                    raster_circle_fill(&mut rgba, width, height, cx, cy, r, fill);
                }
                if let Some(stroke) = state.effective_stroke() {
                    let sw = state.stroke_width * uniform_scale;
                    raster_circle_stroke(&mut rgba, width, height, cx, cy, r, sw, stroke);
                }
            }
        } else if tag.name.eq_ignore_ascii_case("line") {
            let x1 = (attr_f32(&tag.attrs, "x1", 0.0) - vb_x) * sx;
            let y1 = (attr_f32(&tag.attrs, "y1", 0.0) - vb_y) * sy;
            let x2 = (attr_f32(&tag.attrs, "x2", 0.0) - vb_x) * sx;
            let y2 = (attr_f32(&tag.attrs, "y2", 0.0) - vb_y) * sy;
            if let Some(stroke) = state.effective_stroke() {
                let sw = state.stroke_width * uniform_scale;
                raster_line(&mut rgba, width, height, x1, y1, x2, y2, sw, stroke);
            }
        }
    }

    Ok(DecodedImage {
        width,
        height,
        rgba,
    })
}

struct XmlTag {
    name: String,
    attrs: Vec<(String, String)>,
}

fn parse_open_tag(s: &str) -> Result<(XmlTag, &str, bool), ImageError> {
    if !s.starts_with('<') {
        return Err(ImageError::new("malformed SVG tag"));
    }
    let bytes = s.as_bytes();
    let mut i = 1usize;
    let name_start = i;
    while i < bytes.len()
        && !bytes[i].is_ascii_whitespace()
        && bytes[i] != b'>'
        && bytes[i] != b'/'
    {
        i += 1;
    }
    let name = s[name_start..i].to_string();
    if name.is_empty() {
        return Err(ImageError::new("empty SVG tag name"));
    }

    let mut attrs = Vec::new();
    let mut self_closing = false;

    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(ImageError::new("unterminated SVG tag"));
        }
        if bytes[i] == b'>' {
            i += 1;
            break;
        }
        if bytes[i] == b'/' {
            self_closing = true;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'>' {
                return Err(ImageError::new("malformed self-closing SVG tag"));
            }
            i += 1;
            break;
        }

        let k_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let key = s[k_start..i].to_string();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let v_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(ImageError::new("unterminated SVG attribute string"));
                }
                val = s[v_start..i].to_string();
                i += 1;
            } else {
                let v_start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && bytes[i] != b'>'
                    && bytes[i] != b'/'
                {
                    i += 1;
                }
                val = s[v_start..i].to_string();
            }
        }
        if !key.is_empty() {
            attrs.push((key, val));
        }
    }

    Ok((XmlTag { name, attrs }, &s[i..], self_closing))
}

fn find_attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn attr_f32(attrs: &[(String, String)], name: &str, default: f32) -> f32 {
    find_attr(attrs, name)
        .and_then(parse_svg_length)
        .unwrap_or(default)
}

fn parse_svg_length(s: &str) -> Option<f32> {
    let t = s.trim();
    let t = t.strip_suffix("px").unwrap_or(t).trim();
    t.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn parse_svg_dim(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.ends_with('%') {
        return None;
    }
    parse_svg_length(t)
}

fn parse_viewbox(s: &str) -> Option<(f32, f32, f32, f32)> {
    let nums: Vec<f32> = s
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    if nums.len() == 4 && nums.iter().all(|v| v.is_finite()) {
        Some((nums[0], nums[1], nums[2], nums[3]))
    } else {
        None
    }
}

fn parse_svg_paint(s: &str) -> Option<Option<[u8; 4]>> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    if t.eq_ignore_ascii_case("transparent") {
        return Some(Some([0, 0, 0, 0]));
    }
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex_color(hex).map(Some);
    }
    if starts_with_ascii_ci(t, "rgb(") || starts_with_ascii_ci(t, "rgba(") {
        return parse_rgb_func(t).map(Some);
    }
    named_svg_color(t).map(|rgb| Some([rgb[0], rgb[1], rgb[2], 255]))
}

fn parse_hex_color(hex: &str) -> Option<[u8; 4]> {
    let b = hex.as_bytes();
    let nibble = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let byte2 = |hi: u8, lo: u8| -> Option<u8> { Some((nibble(hi)? << 4) | nibble(lo)?) };
    match b.len() {
        3 => {
            let r = nibble(b[0])? * 17;
            let g = nibble(b[1])? * 17;
            let b_ch = nibble(b[2])? * 17;
            Some([r, g, b_ch, 255])
        }
        4 => {
            let r = nibble(b[0])? * 17;
            let g = nibble(b[1])? * 17;
            let b_ch = nibble(b[2])? * 17;
            let a = nibble(b[3])? * 17;
            Some([r, g, b_ch, a])
        }
        6 => Some([
            byte2(b[0], b[1])?,
            byte2(b[2], b[3])?,
            byte2(b[4], b[5])?,
            255,
        ]),
        8 => Some([
            byte2(b[0], b[1])?,
            byte2(b[2], b[3])?,
            byte2(b[4], b[5])?,
            byte2(b[6], b[7])?,
        ]),
        _ => None,
    }
}

fn parse_rgb_func(s: &str) -> Option<[u8; 4]> {
    let inner = s.split_once('(')?.1.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let r = parts[0].parse::<f32>().ok()?.round().clamp(0.0, 255.0) as u8;
    let g = parts[1].parse::<f32>().ok()?.round().clamp(0.0, 255.0) as u8;
    let b = parts[2].parse::<f32>().ok()?.round().clamp(0.0, 255.0) as u8;
    let a = if parts.len() == 4 {
        (parts[3].parse::<f32>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        255
    };
    Some([r, g, b, a])
}

fn named_svg_color(name: &str) -> Option<[u8; 3]> {
    let lower = name.to_ascii_lowercase();
    let rgb = match lower.as_str() {
        "black" | "currentcolor" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "lime" => [0, 255, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "cyan" | "aqua" => [0, 255, 255],
        "magenta" | "fuchsia" => [255, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "silver" => [192, 192, 192],
        "lightgray" | "lightgrey" => [211, 211, 211],
        "darkgray" | "darkgrey" => [169, 169, 169],
        "maroon" => [128, 0, 0],
        "darkred" => [139, 0, 0],
        "navy" => [0, 0, 128],
        "darkblue" => [0, 0, 139],
        "darkgreen" => [0, 100, 0],
        "olive" => [128, 128, 0],
        "purple" => [128, 0, 128],
        "rebeccapurple" => [102, 51, 153],
        "teal" => [0, 128, 128],
        "orange" => [255, 165, 0],
        "pink" => [255, 192, 203],
        "brown" => [165, 42, 42],
        "gold" => [255, 215, 0],
        "indigo" => [75, 0, 130],
        "violet" => [238, 130, 238],
        "coral" => [255, 127, 80],
        "crimson" => [220, 20, 60],
        "lightblue" => [173, 216, 230],
        "skyblue" => [135, 206, 235],
        _ => return None,
    };
    Some(rgb)
}

fn blend_rgba(dst: &mut [u8], src: [u8; 4], coverage: f32) {
    let sa = (src[3] as f32 / 255.0) * coverage.clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    if sa >= 0.9999 {
        dst[0] = src[0];
        dst[1] = src[1];
        dst[2] = src[2];
        dst[3] = 255;
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    let inv_sa = 1.0 - sa;
    for c in 0..3 {
        let out_c = (src[c] as f32 * sa + dst[c] as f32 * da * inv_sa) / out_a;
        dst[c] = out_c.round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn raster_rect_fill(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    [x, y, w, h]: [f32; 4],
    color: [u8; 4],
) {
    let x0 = (x - 1.0).floor().max(0.0) as u32;
    let y0 = (y - 1.0).floor().max(0.0) as u32;
    let x1 = ((x + w + 1.0).ceil().max(0.0) as u32).min(img_w);
    let y1 = ((y + h + 1.0).ceil().max(0.0) as u32).min(img_h);
    for py in y0..y1 {
        let cy = py as f32 + 0.5;
        let cy_cov = (cy - y + 0.5).min(y + h - cy + 0.5).clamp(0.0, 1.0);
        if cy_cov <= 0.0 {
            continue;
        }
        for px in x0..x1 {
            let cx = px as f32 + 0.5;
            let cx_cov = (cx - x + 0.5).min(x + w - cx + 0.5).clamp(0.0, 1.0);
            let cov = cx_cov * cy_cov;
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_rect_stroke(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    stroke_w: f32,
    color: [u8; 4],
) {
    let half = stroke_w * 0.5;
    let cx_box = x + w * 0.5;
    let cy_box = y + h * 0.5;
    let hw = w * 0.5;
    let hh = h * 0.5;

    let x0 = (x - half - 1.0).floor().max(0.0) as u32;
    let y0 = (y - half - 1.0).floor().max(0.0) as u32;
    let x1 = ((x + w + half + 1.0).ceil().max(0.0) as u32).min(img_w);
    let y1 = ((y + h + half + 1.0).ceil().max(0.0) as u32).min(img_h);

    for py in y0..y1 {
        let dy = (py as f32 + 0.5 - cy_box).abs() - hh;
        for px in x0..x1 {
            let dx = (px as f32 + 0.5 - cx_box).abs() - hw;
            let outside = dx.max(0.0).hypot(dy.max(0.0));
            let inside = dx.max(dy).min(0.0);
            let sd_rect = outside + inside;
            let cov = (half - sd_rect.abs() + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

fn raster_circle_fill(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    cx: f32,
    cy: f32,
    r: f32,
    color: [u8; 4],
) {
    let x0 = (cx - r - 1.0).floor().max(0.0) as u32;
    let y0 = (cy - r - 1.0).floor().max(0.0) as u32;
    let x1 = ((cx + r + 1.0).ceil().max(0.0) as u32).min(img_w);
    let y1 = ((cy + r + 1.0).ceil().max(0.0) as u32).min(img_h);

    for py in y0..y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..x1 {
            let dx = px as f32 + 0.5 - cx;
            let dist = dx.hypot(dy);
            let cov = (r - dist + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_circle_stroke(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    cx: f32,
    cy: f32,
    r: f32,
    stroke_w: f32,
    color: [u8; 4],
) {
    let half = stroke_w * 0.5;
    let outer = r + half + 1.0;
    let x0 = (cx - outer).floor().max(0.0) as u32;
    let y0 = (cy - outer).floor().max(0.0) as u32;
    let x1 = ((cx + outer).ceil().max(0.0) as u32).min(img_w);
    let y1 = ((cy + outer).ceil().max(0.0) as u32).min(img_h);

    for py in y0..y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..x1 {
            let dx = px as f32 + 0.5 - cx;
            let dist = (dx.hypot(dy) - r).abs();
            let cov = (half - dist + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_line(
    rgba: &mut [u8],
    img_w: u32,
    img_h: u32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    stroke_w: f32,
    color: [u8; 4],
) {
    let half = stroke_w * 0.5;
    let pad = half + 1.0;
    let min_x = (x1.min(x2) - pad).floor().max(0.0) as u32;
    let min_y = (y1.min(y2) - pad).floor().max(0.0) as u32;
    let max_x = ((x1.max(x2) + pad).ceil().max(0.0) as u32).min(img_w);
    let max_y = ((y1.max(y2) + pad).ceil().max(0.0) as u32).min(img_h);

    let vx = x2 - x1;
    let vy = y2 - y1;
    let len_sq = vx * vx + vy * vy;

    for py in min_y..max_y {
        let cy = py as f32 + 0.5;
        for px in min_x..max_x {
            let cx = px as f32 + 0.5;
            let t = if len_sq > 1e-6 {
                (((cx - x1) * vx + (cy - y1) * vy) / len_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let proj_x = x1 + t * vx;
            let proj_y = y1 + t * vy;
            let dist = (cx - proj_x).hypot(cy - proj_y);
            let cov = (half - dist + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                let idx = ((py * img_w + px) * 4) as usize;
                blend_rgba(&mut rgba[idx..idx + 4], color, cov);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::decode;

    #[test]
    fn svg_renders_rect_circle_and_line() {
        let svg = br##"<?xml version="1.0" encoding="UTF-8"?>
        <svg width="20" height="20" xmlns="http://www.w3.org/2000/svg">
            <rect x="2" y="2" width="6" height="6" fill="#ff0000"/>
            <circle cx="14" cy="14" r="4" fill="blue"/>
            <line x1="0" y1="18" x2="20" y2="18" stroke="lime" stroke-width="2"/>
        </svg>"##;
        let img = decode(svg).expect("SVG must decode");
        assert_eq!(img.width, 20);
        assert_eq!(img.height, 20);

        let px = |x: u32, y: u32| -> &[u8] {
            let i = ((y * img.width + x) * 4) as usize;
            &img.rgba[i..i + 4]
        };
        assert_eq!(px(0, 0), [0, 0, 0, 0]);
        assert_eq!(px(4, 4), [255, 0, 0, 255]);
        assert_eq!(px(14, 14), [0, 0, 255, 255]);
        assert_eq!(px(5, 18), [0, 255, 0, 255]);
    }
}
