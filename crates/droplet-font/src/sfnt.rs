use crate::rasterizer::{
    apply_synthetic_bold, flatten_contour, scanline_rasterize, synthetic_bold_extra_px,
    ContourPoint, Segment2D,
};
use crate::RasterGlyph;

const MAX_COMPOUND_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy)]
struct TableRange {
    offset: usize,
    length: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct TrueTypeFont {
    data: Vec<u8>,
    units_per_em: u16,
    index_to_loc_format: i16,
    num_glyphs: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    num_h_metrics: u16,
    cmap4_range: Option<TableRange>,
    cmap12_range: Option<TableRange>,
    hmtx_range: TableRange,
    loca_range: TableRange,
    glyf_range: TableRange,
}

impl TrueTypeFont {
    pub(crate) fn parse(data: Vec<u8>) -> Result<Self, &'static str> {
        if data.len() < 12 {
            return Err("TTF header truncated");
        }
        let sfnt_ver = read_u32_be(&data, 0)?;
        if sfnt_ver != 0x0001_0000 && sfnt_ver != 0x7472_7565 {
            return Err("unsupported sfnt version");
        }
        let num_tables = read_u16_be(&data, 4)? as usize;
        if num_tables == 0 || num_tables > 256 {
            return Err("invalid sfnt table count");
        }
        let dir_end = 12usize
            .checked_add(num_tables * 16)
            .ok_or("sfnt table directory overflow")?;
        if dir_end > data.len() {
            return Err("truncated sfnt table directory");
        }

        let mut head_r = None;
        let mut maxp_r = None;
        let mut cmap_r = None;
        let mut hhea_r = None;
        let mut hmtx_r = None;
        let mut loca_r = None;
        let mut glyf_r = None;

        for i in 0..num_tables {
            let rec = 12 + i * 16;
            let tag = &data[rec..rec + 4];
            let offset = read_u32_be(&data, rec + 8)? as usize;
            let length = read_u32_be(&data, rec + 12)? as usize;
            let end = offset.checked_add(length).ok_or("table offset overflow")?;
            if end > data.len() {
                return Err("table extends past end of TTF");
            }
            let range = TableRange { offset, length };
            match tag {
                b"head" => head_r = Some(range),
                b"maxp" => maxp_r = Some(range),
                b"cmap" => cmap_r = Some(range),
                b"hhea" => hhea_r = Some(range),
                b"hmtx" => hmtx_r = Some(range),
                b"loca" => loca_r = Some(range),
                b"glyf" => glyf_r = Some(range),
                _ => {}
            }
        }

        let head_r = head_r.ok_or("missing head table")?;
        let maxp_r = maxp_r.ok_or("missing maxp table")?;
        let cmap_r = cmap_r.ok_or("missing cmap table")?;
        let hhea_r = hhea_r.ok_or("missing hhea table")?;
        let hmtx_range = hmtx_r.ok_or("missing hmtx table")?;
        let loca_range = loca_r.ok_or("missing loca table")?;
        let glyf_range = glyf_r.ok_or("missing glyf table")?;

        if head_r.length < 54 {
            return Err("truncated head table");
        }
        let head = &data[head_r.offset..head_r.offset + head_r.length];
        let units_per_em = read_u16_be(head, 18)?;
        if !(16..=16384).contains(&units_per_em) {
            return Err("invalid unitsPerEm");
        }
        let index_to_loc_format = read_i16_be(head, 50)?;
        if index_to_loc_format != 0 && index_to_loc_format != 1 {
            return Err("invalid indexToLocFormat");
        }

        if maxp_r.length < 6 {
            return Err("truncated maxp table");
        }
        let maxp = &data[maxp_r.offset..maxp_r.offset + maxp_r.length];
        let num_glyphs = read_u16_be(maxp, 4)?;
        if num_glyphs == 0 {
            return Err("zero glyphs in maxp");
        }

        if hhea_r.length < 36 {
            return Err("truncated hhea table");
        }
        let hhea = &data[hhea_r.offset..hhea_r.offset + hhea_r.length];
        let ascent = read_i16_be(hhea, 4)?;
        let descent = read_i16_be(hhea, 6)?;
        let line_gap = read_i16_be(hhea, 8)?;
        let num_h_metrics = read_u16_be(hhea, 34)?;
        if num_h_metrics == 0 || (num_h_metrics as usize) * 4 > hmtx_range.length {
            return Err("invalid hmtx metrics length");
        }

        let needed_loca = if index_to_loc_format == 0 {
            (num_glyphs as usize + 1) * 2
        } else {
            (num_glyphs as usize + 1) * 4
        };
        if loca_range.length < needed_loca {
            return Err("truncated loca table");
        }

        let (cmap4_range, cmap12_range) = parse_cmap_ranges(&data, cmap_r)?;
        if cmap4_range.is_none() && cmap12_range.is_none() {
            return Err("no supported cmap format 4 or 12 subtable");
        }

        Ok(Self {
            data,
            units_per_em,
            index_to_loc_format,
            num_glyphs,
            ascent,
            descent,
            line_gap,
            num_h_metrics,
            cmap4_range,
            cmap12_range,
            hmtx_range,
            loca_range,
            glyf_range,
        })
    }

    pub(crate) fn line_height(&self, font_size: f32) -> f32 {
        let scale = font_size / self.units_per_em as f32;
        let raw = (self.ascent as i32 - self.descent as i32 + self.line_gap as i32).max(1) as f32
            * scale;
        raw.max(font_size * 1.2)
    }

    pub(crate) fn baseline_from_top_f32(&self, font_size: f32) -> f32 {
        let scale = font_size / self.units_per_em as f32;
        let ascent_px = self.ascent as f32 * scale;
        let descent_px = self.descent as f32 * scale;
        let lh = self.line_height(font_size);
        let extra = (lh - (ascent_px - descent_px)).max(0.0);
        ascent_px + extra * 0.5
    }

    pub(crate) fn baseline_from_top(&self, font_size: f32) -> i32 {
        self.baseline_from_top_f32(font_size).round() as i32
    }

    pub(crate) fn glyph_index(&self, ch: char) -> u16 {
        let cp = ch as u32;
        if let Some(r12) = self.cmap12_range {
            if let Some(gid) = lookup_cmap12(&self.data[r12.offset..r12.offset + r12.length], cp) {
                if gid < self.num_glyphs as u32 {
                    return gid as u16;
                }
            }
        }
        if cp <= 0xFFFF {
            if let Some(r4) = self.cmap4_range {
                if let Some(gid) =
                    lookup_cmap4(&self.data[r4.offset..r4.offset + r4.length], cp as u16)
                {
                    if gid < self.num_glyphs {
                        return gid;
                    }
                }
            }
        }
        0
    }

    fn advance_units(&self, gid: u16) -> u16 {
        let hmtx = &self.data[self.hmtx_range.offset..self.hmtx_range.offset + self.hmtx_range.length];
        let idx = (gid.min(self.num_h_metrics - 1)) as usize * 4;
        read_u16_be(hmtx, idx).unwrap_or(0)
    }

    pub(crate) fn advance_px(&self, gid: u16, font_size: f32) -> f32 {
        let units = self.advance_units(gid) as f32;
        units * (font_size / self.units_per_em as f32)
    }

    fn glyph_slice(&self, gid: u16) -> Option<&[u8]> {
        if gid >= self.num_glyphs {
            return None;
        }
        let loca = &self.data[self.loca_range.offset..self.loca_range.offset + self.loca_range.length];
        let (start, end) = if self.index_to_loc_format == 0 {
            let i = gid as usize * 2;
            let s = read_u16_be(loca, i).ok()? as usize * 2;
            let e = read_u16_be(loca, i + 2).ok()? as usize * 2;
            (s, e)
        } else {
            let i = gid as usize * 4;
            let s = read_u32_be(loca, i).ok()? as usize;
            let e = read_u32_be(loca, i + 4).ok()? as usize;
            (s, e)
        };
        if start >= end || end > self.glyf_range.length {
            return None;
        }
        let abs_start = self.glyf_range.offset + start;
        let abs_end = self.glyf_range.offset + end;
        Some(&self.data[abs_start..abs_end])
    }

    pub(crate) fn rasterize_glyph(
        &self,
        gid: u16,
        font_size: f32,
        bold_synth: bool,
    ) -> Option<RasterGlyph> {
        let scale = font_size / self.units_per_em as f32;
        let mut advance = self.advance_px(gid, font_size);
        let synth_px = if bold_synth {
            synthetic_bold_extra_px(font_size)
        } else {
            0.0
        };
        if bold_synth {
            advance += synth_px;
        }

        let mut segments = Vec::new();
        self.collect_glyph_segments(gid, Transform2D::IDENTITY, scale, 0, &mut segments);
        if segments.is_empty() {
            return Some(RasterGlyph {
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
                advance,
                alpha: Vec::new(),
            });
        }

        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for &((ax, ay), (bx, by)) in &segments {
            min_x = min_x.min(ax).min(bx);
            min_y = min_y.min(ay).min(by);
            max_x = max_x.max(ax).max(bx);
            max_y = max_y.max(ay).max(by);
        }
        if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
            return None;
        }

        let x0 = min_x.floor() as i32;
        let y0 = min_y.floor() as i32;
        let x1 = max_x.ceil() as i32;
        let y1 = max_y.ceil() as i32;

        let base_w = (x1 - x0).clamp(0, 1024) as usize;
        let h = (y1 - y0).clamp(0, 1024) as usize;
        if base_w == 0 || h == 0 {
            return Some(RasterGlyph {
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
                advance,
                alpha: Vec::new(),
            });
        }

        let mut alpha = scanline_rasterize(&segments, x0, y0, base_w, h);
        let mut final_w = base_w;
        if bold_synth {
            let (dilated, new_w) = apply_synthetic_bold(&alpha, base_w, h, synth_px);
            alpha = dilated;
            final_w = new_w;
        }

        let baseline_px = self.baseline_from_top(font_size);
        Some(RasterGlyph {
            width: final_w as u32,
            height: h as u32,
            bearing_x: x0,
            bearing_y: baseline_px + y0,
            advance,
            alpha,
        })
    }

    fn collect_glyph_segments(
        &self,
        gid: u16,
        xform: Transform2D,
        scale: f32,
        depth: usize,
        out: &mut Vec<Segment2D>,
    ) {
        if depth >= MAX_COMPOUND_DEPTH {
            return;
        }
        let Some(g) = self.glyph_slice(gid) else {
            return;
        };
        if g.len() < 10 {
            return;
        }
        let Ok(num_contours) = read_i16_be(g, 0) else {
            return;
        };

        if num_contours >= 0 {
            decode_simple_glyph(g, num_contours as usize, xform, scale, out);
        } else {
            self.decode_compound_glyph(g, xform, scale, depth, out);
        }
    }

    fn decode_compound_glyph(
        &self,
        g: &[u8],
        parent_xform: Transform2D,
        scale: f32,
        depth: usize,
        out: &mut Vec<Segment2D>,
    ) {
        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const ARGS_ARE_XY_VALUES: u16 = 0x0002;
        const WE_HAVE_A_SCALE: u16 = 0x0008;
        const MORE_COMPONENTS: u16 = 0x0020;
        const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
        const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

        let mut pos = 10usize;
        loop {
            let Ok(flags) = read_u16_be(g, pos) else {
                return;
            };
            let Ok(sub_gid) = read_u16_be(g, pos + 2) else {
                return;
            };
            pos += 4;

            let (dx, dy) = if (flags & ARG_1_AND_2_ARE_WORDS) != 0 {
                let Ok(a1) = read_i16_be(g, pos) else {
                    return;
                };
                let Ok(a2) = read_i16_be(g, pos + 2) else {
                    return;
                };
                pos += 4;
                if (flags & ARGS_ARE_XY_VALUES) != 0 {
                    (a1 as f32, a2 as f32)
                } else {
                    (0.0, 0.0)
                }
            } else {
                if pos + 2 > g.len() {
                    return;
                }
                let a1 = g[pos] as i8;
                let a2 = g[pos + 1] as i8;
                pos += 2;
                if (flags & ARGS_ARE_XY_VALUES) != 0 {
                    (a1 as f32, a2 as f32)
                } else {
                    (0.0, 0.0)
                }
            };

            let mut comp = Transform2D {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                tx: dx,
                ty: dy,
            };

            if (flags & WE_HAVE_A_SCALE) != 0 {
                let Ok(s) = read_f2dot14(g, pos) else {
                    return;
                };
                pos += 2;
                comp.a = s;
                comp.d = s;
            } else if (flags & WE_HAVE_AN_X_AND_Y_SCALE) != 0 {
                let Ok(sx) = read_f2dot14(g, pos) else {
                    return;
                };
                let Ok(sy) = read_f2dot14(g, pos + 2) else {
                    return;
                };
                pos += 4;
                comp.a = sx;
                comp.d = sy;
            } else if (flags & WE_HAVE_A_TWO_BY_TWO) != 0 {
                let Ok(a) = read_f2dot14(g, pos) else {
                    return;
                };
                let Ok(b) = read_f2dot14(g, pos + 2) else {
                    return;
                };
                let Ok(c) = read_f2dot14(g, pos + 4) else {
                    return;
                };
                let Ok(d) = read_f2dot14(g, pos + 6) else {
                    return;
                };
                pos += 8;
                comp.a = a;
                comp.b = b;
                comp.c = c;
                comp.d = d;
            }

            let combined = comp.then(&parent_xform);
            self.collect_glyph_segments(sub_gid, combined, scale, depth + 1, out);

            if (flags & MORE_COMPONENTS) == 0 {
                break;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Transform2D {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
}

impl Transform2D {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    fn then(&self, parent: &Self) -> Self {
        Self {
            a: parent.a * self.a + parent.c * self.b,
            b: parent.b * self.a + parent.d * self.b,
            c: parent.a * self.c + parent.c * self.d,
            d: parent.b * self.c + parent.d * self.d,
            tx: parent.a * self.tx + parent.c * self.ty + parent.tx,
            ty: parent.b * self.tx + parent.d * self.ty + parent.ty,
        }
    }
}

fn decode_simple_glyph(
    g: &[u8],
    num_contours: usize,
    xform: Transform2D,
    scale: f32,
    out: &mut Vec<Segment2D>,
) {
    if num_contours == 0 {
        return;
    }
    let mut pos = 10usize;
    let Some(end_pts_end) = pos.checked_add(num_contours * 2) else {
        return;
    };
    if end_pts_end + 2 > g.len() {
        return;
    }

    let mut end_pts = Vec::with_capacity(num_contours);
    let mut prev_end = -1i32;
    for i in 0..num_contours {
        let Ok(ep) = read_u16_be(g, pos + i * 2) else {
            return;
        };
        if (ep as i32) < prev_end {
            return;
        }
        prev_end = ep as i32;
        end_pts.push(ep as usize);
    }
    pos = end_pts_end;

    let num_points = match end_pts.last() {
        Some(&last) if last < 65535 => last + 1,
        _ => return,
    };

    let Ok(instr_len) = read_u16_be(g, pos) else {
        return;
    };
    pos += 2;
    pos = match pos.checked_add(instr_len as usize) {
        Some(p) if p <= g.len() => p,
        _ => return,
    };

    let mut flags = Vec::with_capacity(num_points);
    while flags.len() < num_points {
        if pos >= g.len() {
            return;
        }
        let flag = g[pos];
        pos += 1;
        flags.push(flag);
        if (flag & 0x08) != 0 {
            if pos >= g.len() {
                return;
            }
            let repeat = g[pos] as usize;
            pos += 1;
            if flags.len() + repeat > num_points {
                return;
            }
            flags.resize(flags.len() + repeat, flag);
        }
    }

    let mut xs = vec![0i32; num_points];
    let mut cur_x = 0i32;
    for i in 0..num_points {
        let f = flags[i];
        let short_x = (f & 0x02) != 0;
        let same_or_pos = (f & 0x10) != 0;
        if short_x {
            if pos >= g.len() {
                return;
            }
            let d = g[pos] as i32;
            pos += 1;
            cur_x += if same_or_pos { d } else { -d };
        } else if !same_or_pos {
            let Ok(d) = read_i16_be(g, pos) else {
                return;
            };
            pos += 2;
            cur_x += d as i32;
        }
        xs[i] = cur_x;
    }

    let mut ys = vec![0i32; num_points];
    let mut cur_y = 0i32;
    for i in 0..num_points {
        let f = flags[i];
        let short_y = (f & 0x04) != 0;
        let same_or_pos = (f & 0x20) != 0;
        if short_y {
            if pos >= g.len() {
                return;
            }
            let d = g[pos] as i32;
            pos += 1;
            cur_y += if same_or_pos { d } else { -d };
        } else if !same_or_pos {
            let Ok(d) = read_i16_be(g, pos) else {
                return;
            };
            pos += 2;
            cur_y += d as i32;
        }
        ys[i] = cur_y;
    }

    let mut start = 0usize;
    for &end_idx in &end_pts {
        if end_idx >= num_points || end_idx < start {
            return;
        }
        let contour: Vec<ContourPoint> = (start..=end_idx)
            .map(|i| {
                let (fx, fy) = xform.apply(xs[i] as f32, ys[i] as f32);
                ContourPoint {
                    x: fx * scale,
                    y: -fy * scale,
                    on_curve: (flags[i] & 0x01) != 0,
                }
            })
            .collect();
        flatten_contour(&contour, out);
        start = end_idx + 1;
    }
}

fn parse_cmap_ranges(
    data: &[u8],
    cmap_r: TableRange,
) -> Result<(Option<TableRange>, Option<TableRange>), &'static str> {
    if cmap_r.length < 4 {
        return Err("truncated cmap table");
    }
    let cmap = &data[cmap_r.offset..cmap_r.offset + cmap_r.length];
    let num_tables = read_u16_be(cmap, 2)? as usize;
    if 4 + num_tables * 8 > cmap.len() {
        return Err("truncated cmap encoding records");
    }

    let mut best4: Option<(u8, TableRange)> = None;
    let mut best12: Option<(u8, TableRange)> = None;

    for i in 0..num_tables {
        let rec = 4 + i * 8;
        let platform = read_u16_be(cmap, rec)?;
        let encoding = read_u16_be(cmap, rec + 2)?;
        let sub_rel = read_u32_be(cmap, rec + 4)? as usize;
        if sub_rel + 4 > cmap.len() {
            continue;
        }
        let abs_off = cmap_r.offset + sub_rel;
        let sub_avail = cmap.len() - sub_rel;
        let fmt = read_u16_be(cmap, sub_rel)?;

        match fmt {
            4 => {
                let sub_len = read_u16_be(cmap, sub_rel + 2)? as usize;
                if sub_len < 16 || sub_len > sub_avail {
                    continue;
                }
                let prio = match (platform, encoding) {
                    (3, 1) => 3,
                    (0, 3) => 2,
                    (0, _) => 1,
                    _ => 0,
                };
                if best4.is_none_or(|(p, _)| prio > p) {
                    best4 = Some((
                        prio,
                        TableRange {
                            offset: abs_off,
                            length: sub_len,
                        },
                    ));
                }
            }
            12 => {
                if sub_avail < 16 {
                    continue;
                }
                let sub_len = read_u32_be(cmap, sub_rel + 4)? as usize;
                if sub_len < 16 || sub_len > sub_avail {
                    continue;
                }
                let prio = match (platform, encoding) {
                    (3, 10) => 3,
                    (0, 4) => 2,
                    (0, _) => 1,
                    _ => 0,
                };
                if best12.is_none_or(|(p, _)| prio > p) {
                    best12 = Some((
                        prio,
                        TableRange {
                            offset: abs_off,
                            length: sub_len,
                        },
                    ));
                }
            }
            _ => {}
        }
    }

    Ok((best4.map(|(_, r)| r), best12.map(|(_, r)| r)))
}

fn lookup_cmap4(sub: &[u8], cp: u16) -> Option<u16> {
    if sub.len() < 16 {
        return None;
    }
    let seg_count_x2 = read_u16_be(sub, 6).ok()? as usize;
    if seg_count_x2 < 2 || !seg_count_x2.is_multiple_of(2) {
        return None;
    }
    let seg_count = seg_count_x2 / 2;
    let end_codes_off = 14usize;
    let start_codes_off = end_codes_off + seg_count * 2 + 2;
    let id_delta_off = start_codes_off + seg_count * 2;
    let id_range_off = id_delta_off + seg_count * 2;
    if id_range_off + seg_count * 2 > sub.len() {
        return None;
    }

    let mut lo = 0usize;
    let mut hi = seg_count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let end_code = read_u16_be(sub, end_codes_off + mid * 2).ok()?;
        if end_code < cp {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo >= seg_count {
        return Some(0);
    }

    let start_code = read_u16_be(sub, start_codes_off + lo * 2).ok()?;
    if cp < start_code {
        return Some(0);
    }

    let id_delta = read_i16_be(sub, id_delta_off + lo * 2).ok()?;
    let range_off_pos = id_range_off + lo * 2;
    let id_range_val = read_u16_be(sub, range_off_pos).ok()?;

    if id_range_val == 0 {
        Some((cp as i32 + id_delta as i32) as u16)
    } else {
        let glyph_addr = range_off_pos
            .checked_add(id_range_val as usize)?
            .checked_add((cp - start_code) as usize * 2)?;
        let raw_gid = read_u16_be(sub, glyph_addr).ok()?;
        if raw_gid == 0 {
            Some(0)
        } else {
            Some((raw_gid as i32 + id_delta as i32) as u16)
        }
    }
}

fn lookup_cmap12(sub: &[u8], cp: u32) -> Option<u32> {
    if sub.len() < 16 {
        return None;
    }
    let num_groups = read_u32_be(sub, 12).ok()? as usize;
    if 16usize.checked_add(num_groups * 12)? > sub.len() {
        return None;
    }

    let mut lo = 0usize;
    let mut hi = num_groups;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let end_cp = read_u32_be(sub, 16 + mid * 12 + 4).ok()?;
        if end_cp < cp {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo >= num_groups {
        return Some(0);
    }

    let off = 16 + lo * 12;
    let start_cp = read_u32_be(sub, off).ok()?;
    let end_cp = read_u32_be(sub, off + 4).ok()?;
    let start_gid = read_u32_be(sub, off + 8).ok()?;
    if cp >= start_cp && cp <= end_cp {
        Some(start_gid.wrapping_add(cp - start_cp))
    } else {
        Some(0)
    }
}

fn read_u16_be(data: &[u8], off: usize) -> Result<u16, &'static str> {
    let end = off.checked_add(2).ok_or("u16 read overflow")?;
    if end > data.len() {
        return Err("truncated u16");
    }
    Ok(u16::from_be_bytes([data[off], data[off + 1]]))
}

fn read_i16_be(data: &[u8], off: usize) -> Result<i16, &'static str> {
    Ok(read_u16_be(data, off)? as i16)
}

fn read_u32_be(data: &[u8], off: usize) -> Result<u32, &'static str> {
    let end = off.checked_add(4).ok_or("u32 read overflow")?;
    if end > data.len() {
        return Err("truncated u32");
    }
    Ok(u32::from_be_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ]))
}

fn read_f2dot14(data: &[u8], off: usize) -> Result<f32, &'static str> {
    Ok(read_i16_be(data, off)? as f32 / 16384.0)
}
