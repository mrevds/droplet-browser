const RASTER_SUB_ROWS: usize = 16;

pub(crate) type Segment2D = ((f32, f32), (f32, f32));

#[derive(Clone, Copy)]
pub(crate) struct ContourPoint {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) on_curve: bool,
}

pub(crate) fn synthetic_bold_extra_px(font_size: f32) -> f32 {
    (font_size * 0.055).max(0.75)
}

pub(crate) fn flatten_contour(pts: &[ContourPoint], out: &mut Vec<Segment2D>) {
    let n = pts.len();
    if n < 2 {
        return;
    }

    let (mut cur, start_step, end_step) = if pts[0].on_curve {
        ((pts[0].x, pts[0].y), 1usize, n)
    } else if pts[n - 1].on_curve {
        ((pts[n - 1].x, pts[n - 1].y), 0usize, n - 1)
    } else {
        let mid = (
            (pts[n - 1].x + pts[0].x) * 0.5,
            (pts[n - 1].y + pts[0].y) * 0.5,
        );
        (mid, 0usize, n - 1)
    };

    let mut step = start_step;
    while step <= end_step {
        let p = pts[step % n];
        if p.on_curve {
            let next = (p.x, p.y);
            if cur != next {
                out.push((cur, next));
            }
            cur = next;
            step += 1;
        } else {
            let ctrl = (p.x, p.y);
            let next_pt = pts[(step + 1) % n];
            if next_pt.on_curve {
                let next = (next_pt.x, next_pt.y);
                flatten_quad_bezier(cur, ctrl, next, 0, out);
                cur = next;
                step += 2;
            } else {
                let mid = ((ctrl.0 + next_pt.x) * 0.5, (ctrl.1 + next_pt.y) * 0.5);
                flatten_quad_bezier(cur, ctrl, mid, 0, out);
                cur = mid;
                step += 1;
            }
        }
    }
}

fn flatten_quad_bezier(
    p0: (f32, f32),
    ctrl: (f32, f32),
    p1: (f32, f32),
    depth: u8,
    out: &mut Vec<Segment2D>,
) {
    let dev_x = (p0.0 + p1.0) * 0.5 - ctrl.0;
    let dev_y = (p0.1 + p1.1) * 0.5 - ctrl.1;
    let err_sq = dev_x * dev_x + dev_y * dev_y;
    if depth >= 8 || err_sq <= 0.125 * 0.125 {
        if p0 != p1 {
            out.push((p0, p1));
        }
        return;
    }
    let m0 = ((p0.0 + ctrl.0) * 0.5, (p0.1 + ctrl.1) * 0.5);
    let m1 = ((ctrl.0 + p1.0) * 0.5, (ctrl.1 + p1.1) * 0.5);
    let mid = ((m0.0 + m1.0) * 0.5, (m0.1 + m1.1) * 0.5);
    flatten_quad_bezier(p0, m0, mid, depth + 1, out);
    flatten_quad_bezier(mid, m1, p1, depth + 1, out);
}

pub(crate) fn scanline_rasterize(
    segments: &[Segment2D],
    x0: i32,
    y0: i32,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut local_segs: Vec<(f32, f32, f32, f32, f32, f32)> = Vec::with_capacity(segments.len());
    for &((ax, ay), (bx, by)) in segments {
        let lax = ax - x0 as f32;
        let lay = ay - y0 as f32;
        let lbx = bx - x0 as f32;
        let lby = by - y0 as f32;
        if lay != lby {
            let min_y = lay.min(lby);
            let max_y = lay.max(lby);
            local_segs.push((lax, lay, lbx, lby, min_y, max_y));
        }
    }

    let mut alpha = vec![0u8; width * height];
    let mut row_cov = vec![0.0f32; width];
    let mut row_segs: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(32);
    let mut crossings: Vec<(f32, i32)> = Vec::with_capacity(32);
    let w_f32 = width as f32;

    for row in 0..height {
        let row_top = row as f32;
        let row_bot = (row + 1) as f32;
        row_segs.clear();
        for &(ax, ay, bx, by, min_y, max_y) in &local_segs {
            if max_y > row_top && min_y < row_bot {
                row_segs.push((ax, ay, bx, by));
            }
        }
        if row_segs.is_empty() {
            continue;
        }

        row_cov.fill(0.0);
        for sub in 0..RASTER_SUB_ROWS {
            let sy = row_top + (sub as f32 + 0.5) / RASTER_SUB_ROWS as f32;
            crossings.clear();
            for &(ax, ay, bx, by) in &row_segs {
                if ay <= sy && sy < by {
                    let x = ax + (sy - ay) * (bx - ax) / (by - ay);
                    crossings.push((x, 1));
                } else if by <= sy && sy < ay {
                    let x = bx + (sy - by) * (ax - bx) / (ay - by);
                    crossings.push((x, -1));
                }
            }
            if crossings.is_empty() {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));

            let mut winding = 0i32;
            let mut span_start = 0.0f32;
            for &(x, dir) in &crossings {
                let prev_w = winding;
                winding += dir;
                if prev_w == 0 && winding != 0 {
                    span_start = x;
                } else if prev_w != 0 && winding == 0 {
                    let s = span_start.clamp(0.0, w_f32);
                    let e = x.clamp(0.0, w_f32);
                    if e > s {
                        let p_start = s.floor() as usize;
                        let p_end = (e.ceil() as usize).min(width);
                        if p_start < p_end {
                            if p_start + 1 == p_end {
                                row_cov[p_start] += e - s;
                            } else {
                                row_cov[p_start] += (p_start + 1) as f32 - s;
                                for cell in &mut row_cov[(p_start + 1)..(p_end - 1)] {
                                    *cell += 1.0;
                                }
                                row_cov[p_end - 1] += e - (p_end - 1) as f32;
                            }
                        }
                    }
                }
            }
        }

        let dst_row = &mut alpha[row * width..(row + 1) * width];
        for (px, &cov) in row_cov.iter().enumerate() {
            let v = ((cov / RASTER_SUB_ROWS as f32) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
            dst_row[px] = v;
        }
    }

    alpha
}

pub(crate) fn apply_synthetic_bold(
    alpha: &[u8],
    width: usize,
    height: usize,
    synth_px: f32,
) -> (Vec<u8>, usize) {
    let extra = (synth_px.ceil() as usize).max(1);
    let new_w = width + extra;
    let mut out = vec![0u8; new_w * height];
    let whole = synth_px.floor() as usize;
    let frac = synth_px - whole as f32;

    for y in 0..height {
        let src_row = &alpha[y * width..(y + 1) * width];
        let dst_row = &mut out[y * new_w..(y + 1) * new_w];
        for x in 0..new_w {
            let mut val = if x < width { src_row[x] } else { 0 };
            for d in 1..=whole {
                if x >= d && (x - d) < width {
                    val = val.max(src_row[x - d]);
                }
            }
            if frac > 0.0 && x > whole && (x - whole - 1) < width {
                let shifted = (src_row[x - whole - 1] as f32 * frac).round() as u8;
                val = val.max(shifted);
            }
            dst_row[x] = val;
        }
    }
    (out, new_w)
}
