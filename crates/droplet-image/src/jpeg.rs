//! Pure-Rust Baseline DCT JPEG (`SOF0`) decoder.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

use crate::{validate_dimensions, DecodedImage, ImageError};

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Debug, Clone, Copy)]
struct ComponentSpec {
    id: u8,
    h_samp: u8,
    v_samp: u8,
    quant_table_id: usize,
}

#[derive(Debug, Clone)]
struct FrameHeader {
    width: u32,
    height: u32,
    components: Vec<ComponentSpec>,
    max_h: usize,
    max_v: usize,
}

#[derive(Debug, Clone, Copy)]
struct ScanComponent {
    frame_idx: usize,
    dc_table_id: usize,
    ac_table_id: usize,
}

#[derive(Debug, Clone)]
struct HuffmanTable {
    counts: [u16; 17],
    symbols: Vec<u8>,
}

impl HuffmanTable {
    fn from_dht(counts_16: &[u8], values: &[u8]) -> Result<Self, ImageError> {
        let mut counts = [0u16; 17];
        let mut total = 0usize;
        for (i, &c) in counts_16.iter().enumerate() {
            counts[i + 1] = c as u16;
            total += c as usize;
        }
        if total != values.len() || total > 256 {
            return Err(ImageError::new("invalid JPEG DHT symbol count"));
        }

        let mut left: i32 = 1;
        for &count in counts.iter().skip(1) {
            left = (left << 1) - count as i32;
            if left < 0 {
                return Err(ImageError::new("oversubscribed JPEG Huffman table"));
            }
        }

        Ok(Self {
            counts,
            symbols: values.to_vec(),
        })
    }

    fn decode(&self, reader: &mut ScanBitReader<'_>) -> Result<u8, ImageError> {
        if self.symbols.is_empty() {
            return Err(ImageError::new("empty JPEG Huffman table"));
        }
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: usize = 0;
        for &count_u16 in self.counts.iter().skip(1) {
            let bit = reader.read_bit()? as i32;
            code = (code << 1) | bit;
            let count = count_u16 as i32;
            if code - count < first {
                let sym_idx = index + (code - first) as usize;
                return self
                    .symbols
                    .get(sym_idx)
                    .copied()
                    .ok_or_else(|| ImageError::new("JPEG Huffman symbol index out of bounds"));
            }
            index += count as usize;
            first = (first + count) << 1;
        }
        Err(ImageError::new("invalid Huffman code in JPEG scan"))
    }
}

struct ScanBitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_count: u8,
}

impl<'a> ScanBitReader<'a> {
    fn new(data: &'a [u8], start_pos: usize) -> Self {
        Self {
            data,
            pos: start_pos,
            bit_buf: 0,
            bit_count: 0,
        }
    }

    fn read_unstuffed_byte(&mut self) -> Result<u8, ImageError> {
        if self.pos >= self.data.len() {
            return Err(ImageError::new("truncated JPEG entropy stream"));
        }
        let b = self.data[self.pos];
        if b != 0xFF {
            self.pos += 1;
            return Ok(b);
        }
        if self.pos + 1 >= self.data.len() {
            return Err(ImageError::new("truncated JPEG marker inside entropy stream"));
        }
        let next = self.data[self.pos + 1];
        if next == 0x00 {
            self.pos += 2;
            Ok(0xFF)
        } else {
            Err(ImageError::new(format!(
                "unexpected JPEG marker 0xFF{next:02X} inside scan data"
            )))
        }
    }

    fn ensure_bits(&mut self, n: u8) -> Result<(), ImageError> {
        while self.bit_count < n {
            let byte = self.read_unstuffed_byte()?;
            self.bit_buf = (self.bit_buf << 8) | (byte as u32);
            self.bit_count += 8;
        }
        Ok(())
    }

    fn read_bit(&mut self) -> Result<u32, ImageError> {
        self.ensure_bits(1)?;
        self.bit_count -= 1;
        Ok((self.bit_buf >> self.bit_count) & 1)
    }

    fn read_bits(&mut self, n: u8) -> Result<u32, ImageError> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure_bits(n)?;
        self.bit_count -= n;
        let mask = (1u32 << n) - 1;
        Ok((self.bit_buf >> self.bit_count) & mask)
    }

    fn consume_restart_marker(&mut self, expected_rst: u8) -> Result<(), ImageError> {
        self.bit_buf = 0;
        self.bit_count = 0;

        if self.pos >= self.data.len() || self.data[self.pos] != 0xFF {
            return Err(ImageError::new("missing JPEG restart marker"));
        }
        while self.pos < self.data.len() && self.data[self.pos] == 0xFF {
            self.pos += 1;
        }
        if self.pos >= self.data.len() {
            return Err(ImageError::new("truncated JPEG restart marker"));
        }
        let marker = self.data[self.pos];
        self.pos += 1;
        let expected_marker = 0xD0 + (expected_rst & 7);
        if marker != expected_marker {
            return Err(ImageError::new(format!(
                "invalid JPEG restart marker: expected 0xFF{expected_marker:02X}, got 0xFF{marker:02X}"
            )));
        }
        Ok(())
    }

    fn finish_scan(self) -> usize {
        self.pos
    }
}

struct IdctBasis {
    cos: [[f64; 8]; 8],
}

impl IdctBasis {
    fn new() -> Self {
        let mut cos = [[0.0f64; 8]; 8];
        for (u, row) in cos.iter_mut().enumerate() {
            let cu = if u == 0 { FRAC_1_SQRT_2 } else { 1.0 };
            for (x, cell) in row.iter_mut().enumerate() {
                *cell = cu * 0.5 * (((2 * x + 1) * u) as f64 * PI / 16.0).cos();
            }
        }
        Self { cos }
    }

    fn transform_8x8(&self, coeffs: &[i32; 64], out: &mut [u8; 64]) {
        let mut row_tmp = [0.0f64; 64];

        for v in 0..8 {
            let base = v * 8;
            let row = &coeffs[base..base + 8];
            if row[1..].iter().all(|&c| c == 0) {
                let dc = (row[0] as f64) * self.cos[0][0];
                row_tmp[base..base + 8].fill(dc);
            } else {
                for (x, dst) in row_tmp[base..base + 8].iter_mut().enumerate() {
                    let mut sum = 0.0f64;
                    for (u, &coeff) in row.iter().enumerate() {
                        if coeff != 0 {
                            sum += (coeff as f64) * self.cos[u][x];
                        }
                    }
                    *dst = sum;
                }
            }
        }

        for x in 0..8 {
            for y in 0..8 {
                let mut sum = 0.0f64;
                for v in 0..8 {
                    sum += row_tmp[v * 8 + x] * self.cos[v][y];
                }
                out[y * 8 + x] = (sum + 128.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    if bytes.len() < 2 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return Err(ImageError::new("invalid JPEG SOI signature"));
    }

    let mut pos = 2usize;
    let mut quant_tables: [Option<[u16; 64]>; 4] = [None; 4];
    let mut dc_tables: [Option<HuffmanTable>; 4] = [None, None, None, None];
    let mut ac_tables: [Option<HuffmanTable>; 4] = [None, None, None, None];
    let mut frame: Option<FrameHeader> = None;
    let mut restart_interval = 0usize;
    let mut decoded_image: Option<DecodedImage> = None;

    while pos < bytes.len() {
        if bytes[pos] != 0xFF {
            return Err(ImageError::new("expected JPEG marker"));
        }
        while pos < bytes.len() && bytes[pos] == 0xFF {
            pos += 1;
        }
        if pos >= bytes.len() {
            return Err(ImageError::new("truncated JPEG marker"));
        }
        let marker = bytes[pos];
        pos += 1;

        match marker {
            0x00 => return Err(ImageError::new("invalid stuffed byte 0xFF00 outside JPEG scan")),
            0x01 => continue, // TEM
            0xD9 => {
                return decoded_image
                    .ok_or_else(|| ImageError::new("JPEG EOI encountered before image scan"));
            }
            0xD8 => return Err(ImageError::new("unexpected duplicate JPEG SOI marker")),
            0xD0..=0xD7 => {
                return Err(ImageError::new(
                    "unexpected JPEG restart marker outside scan",
                ));
            }
            _ => {}
        }

        if pos + 2 > bytes.len() {
            return Err(ImageError::new("truncated JPEG marker length"));
        }
        let seg_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        if seg_len < 2 {
            return Err(ImageError::new("invalid JPEG marker segment length"));
        }
        let seg_end = pos
            .checked_add(seg_len)
            .ok_or_else(|| ImageError::new("JPEG segment length overflow"))?;
        if seg_end > bytes.len() {
            return Err(ImageError::new("truncated JPEG marker segment"));
        }
        let payload = &bytes[pos + 2..seg_end];
        pos = seg_end;

        match marker {
            // APP0..=APP15 and COM
            0xE0..=0xEF | 0xFE => {}
            // DQT: Define Quantization Table
            0xDB => parse_dqt(payload, &mut quant_tables)?,
            // DHT: Define Huffman Table
            0xC4 => parse_dht(payload, &mut dc_tables, &mut ac_tables)?,
            // DRI: Define Restart Interval
            0xDD => {
                if payload.len() != 2 {
                    return Err(ImageError::new("invalid JPEG DRI length"));
                }
                restart_interval = u16::from_be_bytes([payload[0], payload[1]]) as usize;
            }
            // SOF0: Baseline DCT
            0xC0 => {
                if frame.is_some() {
                    return Err(ImageError::new("duplicate JPEG SOF0 marker"));
                }
                frame = Some(parse_sof0(payload)?);
            }
            // SOF2: Progressive DCT
            0xC2 => {
                return Err(ImageError::new("unsupported progressive JPEG (SOF2)"));
            }
            // Arithmetic coding SOF markers and DAC
            0xC9..=0xCF => {
                return Err(ImageError::new("unsupported arithmetic-coded JPEG"));
            }
            // Other unsupported SOF markers
            0xC1 | 0xC3 | 0xC5..=0xC7 => {
                return Err(ImageError::new(format!(
                    "unsupported JPEG frame marker 0xFF{marker:02X}"
                )));
            }
            // SOS: Start of Scan
            0xDA => {
                if decoded_image.is_some() {
                    return Err(ImageError::new("multi-scan JPEG is not supported"));
                }
                let hdr = frame
                    .as_ref()
                    .ok_or_else(|| ImageError::new("JPEG SOS before SOF0"))?;
                let scan_comps = parse_sos(payload, hdr)?;
                let (img, next_pos) = decode_scan(
                    bytes,
                    pos,
                    hdr,
                    &scan_comps,
                    &quant_tables,
                    &dc_tables,
                    &ac_tables,
                    restart_interval,
                )?;
                decoded_image = Some(img);
                pos = next_pos;
            }
            other => {
                return Err(ImageError::new(format!(
                    "unsupported or invalid JPEG marker 0xFF{other:02X}"
                )));
            }
        }
    }

    Err(ImageError::new("missing JPEG EOI marker"))
}

fn parse_dqt(payload: &[u8], quant_tables: &mut [Option<[u16; 64]>; 4]) -> Result<(), ImageError> {
    if payload.is_empty() {
        return Err(ImageError::new("empty JPEG DQT marker"));
    }
    let mut off = 0usize;
    while off < payload.len() {
        let pq_tq = payload[off];
        off += 1;
        let precision = pq_tq >> 4;
        let table_id = (pq_tq & 0x0F) as usize;
        if table_id >= 4 {
            return Err(ImageError::new("invalid JPEG quantization table ID"));
        }
        let mut table = [0u16; 64];
        match precision {
            0 => {
                if off + 64 > payload.len() {
                    return Err(ImageError::new("truncated 8-bit JPEG DQT table"));
                }
                for (dst, &src) in table.iter_mut().zip(&payload[off..off + 64]) {
                    *dst = src as u16;
                }
                off += 64;
            }
            1 => {
                if off + 128 > payload.len() {
                    return Err(ImageError::new("truncated 16-bit JPEG DQT table"));
                }
                for (dst, pair) in table.iter_mut().zip(payload[off..off + 128].as_chunks::<2>().0)
                {
                    *dst = u16::from_be_bytes(*pair);
                }
                off += 128;
            }
            _ => return Err(ImageError::new("invalid JPEG DQT precision")),
        }
        quant_tables[table_id] = Some(table);
    }
    Ok(())
}

fn parse_dht(
    payload: &[u8],
    dc_tables: &mut [Option<HuffmanTable>; 4],
    ac_tables: &mut [Option<HuffmanTable>; 4],
) -> Result<(), ImageError> {
    if payload.is_empty() {
        return Err(ImageError::new("empty JPEG DHT marker"));
    }
    let mut off = 0usize;
    while off < payload.len() {
        let tc_th = payload[off];
        off += 1;
        let class = tc_th >> 4;
        let table_id = (tc_th & 0x0F) as usize;
        if class > 1 || table_id >= 4 {
            return Err(ImageError::new("invalid JPEG Huffman table header"));
        }
        if off + 16 > payload.len() {
            return Err(ImageError::new("truncated JPEG DHT code lengths"));
        }
        let counts_16 = &payload[off..off + 16];
        off += 16;
        let sym_count: usize = counts_16.iter().map(|&c| c as usize).sum();
        if off + sym_count > payload.len() {
            return Err(ImageError::new("truncated JPEG DHT symbol values"));
        }
        let values = &payload[off..off + sym_count];
        off += sym_count;

        let table = HuffmanTable::from_dht(counts_16, values)?;
        if class == 0 {
            dc_tables[table_id] = Some(table);
        } else {
            ac_tables[table_id] = Some(table);
        }
    }
    Ok(())
}

fn parse_sof0(payload: &[u8]) -> Result<FrameHeader, ImageError> {
    if payload.len() < 6 {
        return Err(ImageError::new("truncated JPEG SOF0 header"));
    }
    let precision = payload[0];
    if precision != 8 {
        return Err(ImageError::new(format!(
            "unsupported JPEG sample precision {precision}"
        )));
    }
    let height = u16::from_be_bytes([payload[1], payload[2]]) as u32;
    let width = u16::from_be_bytes([payload[3], payload[4]]) as u32;
    validate_dimensions(width, height)?;

    let num_components = payload[5] as usize;
    if num_components != 1 && num_components != 3 {
        return Err(ImageError::new(format!(
            "unsupported JPEG component count {num_components}"
        )));
    }
    if payload.len() != 6 + num_components * 3 {
        return Err(ImageError::new("invalid JPEG SOF0 payload length"));
    }

    let mut components = Vec::with_capacity(num_components);
    let mut max_h = 1usize;
    let mut max_v = 1usize;
    let mut total_mcu_blocks = 0usize;

    for i in 0..num_components {
        let base = 6 + i * 3;
        let id = payload[base];
        let samp = payload[base + 1];
        let h_samp = samp >> 4;
        let v_samp = samp & 0x0F;
        let quant_table_id = payload[base + 2] as usize;

        if h_samp == 0 || v_samp == 0 || h_samp > 4 || v_samp > 4 {
            return Err(ImageError::new("invalid JPEG component sampling factor"));
        }
        if quant_table_id >= 4 {
            return Err(ImageError::new("invalid JPEG component quantization table ID"));
        }
        if components.iter().any(|c: &ComponentSpec| c.id == id) {
            return Err(ImageError::new("duplicate JPEG component ID in SOF0"));
        }

        let (eff_h, eff_v) = if num_components == 1 {
            (1u8, 1u8)
        } else {
            (h_samp, v_samp)
        };
        max_h = max_h.max(eff_h as usize);
        max_v = max_v.max(eff_v as usize);
        total_mcu_blocks += (eff_h as usize) * (eff_v as usize);

        components.push(ComponentSpec {
            id,
            h_samp: eff_h,
            v_samp: eff_v,
            quant_table_id,
        });
    }

    if num_components == 3 {
        if total_mcu_blocks > 10 {
            return Err(ImageError::new("JPEG MCU block count exceeds baseline limit"));
        }
        for c in &components {
            if !max_h.is_multiple_of(c.h_samp as usize)
                || !max_v.is_multiple_of(c.v_samp as usize)
            {
                return Err(ImageError::new("unsupported JPEG subsampling ratio"));
            }
        }
    }

    Ok(FrameHeader {
        width,
        height,
        components,
        max_h,
        max_v,
    })
}

fn parse_sos(payload: &[u8], frame: &FrameHeader) -> Result<Vec<ScanComponent>, ImageError> {
    if payload.is_empty() {
        return Err(ImageError::new("empty JPEG SOS header"));
    }
    let ns = payload[0] as usize;
    if ns != frame.components.len() {
        return Err(ImageError::new(
            "unsupported JPEG scan component count (non-interleaved multi-scan)",
        ));
    }
    if payload.len() != 1 + ns * 2 + 3 {
        return Err(ImageError::new("invalid JPEG SOS header length"));
    }

    let mut scan_comps = Vec::with_capacity(ns);
    for j in 0..ns {
        let comp_id = payload[1 + j * 2];
        let td_ta = payload[1 + j * 2 + 1];
        let dc_table_id = (td_ta >> 4) as usize;
        let ac_table_id = (td_ta & 0x0F) as usize;
        if dc_table_id >= 4 || ac_table_id >= 4 {
            return Err(ImageError::new("invalid JPEG SOS Huffman table selector"));
        }
        let frame_idx = frame
            .components
            .iter()
            .position(|c| c.id == comp_id)
            .ok_or_else(|| ImageError::new("JPEG SOS references unknown component ID"))?;
        if scan_comps
            .iter()
            .any(|sc: &ScanComponent| sc.frame_idx == frame_idx)
        {
            return Err(ImageError::new("duplicate component in JPEG SOS"));
        }
        scan_comps.push(ScanComponent {
            frame_idx,
            dc_table_id,
            ac_table_id,
        });
    }

    let ss = payload[1 + ns * 2];
    let se = payload[1 + ns * 2 + 1];
    let ah_al = payload[1 + ns * 2 + 2];
    if ss != 0 || se != 63 || ah_al != 0 {
        return Err(ImageError::new(
            "unsupported non-baseline JPEG spectral selection or approximation",
        ));
    }

    Ok(scan_comps)
}

#[allow(clippy::too_many_arguments)]
fn decode_scan(
    bytes: &[u8],
    scan_start: usize,
    frame: &FrameHeader,
    scan_comps: &[ScanComponent],
    quant_tables: &[Option<[u16; 64]>; 4],
    dc_tables: &[Option<HuffmanTable>; 4],
    ac_tables: &[Option<HuffmanTable>; 4],
    restart_interval: usize,
) -> Result<(DecodedImage, usize), ImageError> {
    for sc in scan_comps {
        let qid = frame.components[sc.frame_idx].quant_table_id;
        if quant_tables[qid].is_none() {
            return Err(ImageError::new("missing JPEG quantization table"));
        }
        if dc_tables[sc.dc_table_id].is_none() || ac_tables[sc.ac_table_id].is_none() {
            return Err(ImageError::new("missing JPEG Huffman table"));
        }
    }

    let rgba_len = validate_dimensions(frame.width, frame.height)?;
    let mut rgba = vec![0u8; rgba_len];

    let mcu_w = frame.max_h * 8;
    let mcu_h = frame.max_v * 8;
    let mcus_x = (frame.width as usize).div_ceil(mcu_w);
    let mcus_y = (frame.height as usize).div_ceil(mcu_h);

    let basis = IdctBasis::new();
    let mut reader = ScanBitReader::new(bytes, scan_start);
    let mut dc_pred = [0i32; 3];
    let mut rst_expected: u8 = 0;

    // Max sampling factor is 4x4 => max 32x32 = 1024 samples per component per MCU.
    let mut comp_bufs = [[0u8; 1024]; 3];
    let mut coeffs = [0i32; 64];
    let mut block_px = [0u8; 64];

    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            let mcu_idx = my * mcus_x + mx;
            if restart_interval > 0 && mcu_idx > 0 && mcu_idx.is_multiple_of(restart_interval) {
                reader.consume_restart_marker(rst_expected)?;
                rst_expected = (rst_expected + 1) & 7;
                dc_pred = [0; 3];
            }

            for sc in scan_comps {
                let comp = frame.components[sc.frame_idx];
                let quant = quant_tables[comp.quant_table_id]
                    .as_ref()
                    .ok_or_else(|| ImageError::new("missing JPEG quantization table"))?;
                let dc_table = dc_tables[sc.dc_table_id]
                    .as_ref()
                    .ok_or_else(|| ImageError::new("missing JPEG DC Huffman table"))?;
                let ac_table = ac_tables[sc.ac_table_id]
                    .as_ref()
                    .ok_or_else(|| ImageError::new("missing JPEG AC Huffman table"))?;

                let h = comp.h_samp as usize;
                let v = comp.v_samp as usize;
                let comp_stride = h * 8;
                let dst_plane = &mut comp_bufs[sc.frame_idx];

                for by in 0..v {
                    for bx in 0..h {
                        decode_block(
                            &mut reader,
                            dc_table,
                            ac_table,
                            quant,
                            &mut dc_pred[sc.frame_idx],
                            &mut coeffs,
                        )?;
                        basis.transform_8x8(&coeffs, &mut block_px);

                        for py in 0..8 {
                            let dst_row = (by * 8 + py) * comp_stride + bx * 8;
                            dst_plane[dst_row..dst_row + 8]
                                .copy_from_slice(&block_px[py * 8..py * 8 + 8]);
                        }
                    }
                }
            }

            write_mcu_rgba(frame, mx, my, mcu_w, mcu_h, &comp_bufs, &mut rgba);
        }
    }

    let next_pos = reader.finish_scan();
    Ok((
        DecodedImage {
            width: frame.width,
            height: frame.height,
            rgba,
        },
        next_pos,
    ))
}

fn decode_block(
    reader: &mut ScanBitReader<'_>,
    dc_table: &HuffmanTable,
    ac_table: &HuffmanTable,
    quant: &[u16; 64],
    dc_pred: &mut i32,
    coeffs: &mut [i32; 64],
) -> Result<(), ImageError> {
    coeffs.fill(0);

    let dc_cat = dc_table.decode(reader)?;
    if dc_cat > 15 {
        return Err(ImageError::new("invalid JPEG DC Huffman category"));
    }
    let dc_diff = receive_extend(reader, dc_cat)?;
    *dc_pred = dc_pred.wrapping_add(dc_diff);
    coeffs[0] = dc_pred.wrapping_mul(quant[0] as i32);

    let mut k = 1usize;
    while k < 64 {
        let rs = ac_table.decode(reader)?;
        let rrrr = (rs >> 4) as usize;
        let ssss = rs & 0x0F;
        if ssss == 0 {
            if rrrr == 0 {
                break;
            }
            if rrrr == 15 {
                if k + 16 > 64 {
                    return Err(ImageError::new("JPEG AC ZRL run-length overflow"));
                }
                k += 16;
                continue;
            }
            return Err(ImageError::new("invalid JPEG AC Huffman symbol"));
        }

        k += rrrr;
        if k >= 64 {
            return Err(ImageError::new("JPEG AC run-length overflow"));
        }
        let val = receive_extend(reader, ssss)?;
        coeffs[ZIGZAG[k]] = val.wrapping_mul(quant[k] as i32);
        k += 1;
    }

    Ok(())
}

fn receive_extend(reader: &mut ScanBitReader<'_>, nbits: u8) -> Result<i32, ImageError> {
    if nbits == 0 {
        return Ok(0);
    }
    let v = reader.read_bits(nbits)? as i32;
    let vt = 1i32 << (nbits - 1);
    if v < vt {
        Ok(v + ((-1i32) << nbits) + 1)
    } else {
        Ok(v)
    }
}

fn write_mcu_rgba(
    frame: &FrameHeader,
    mx: usize,
    my: usize,
    mcu_w: usize,
    mcu_h: usize,
    comp_bufs: &[[u8; 1024]; 3],
    rgba: &mut [u8],
) {
    let img_w = frame.width as usize;
    let img_h = frame.height as usize;
    let x_base = mx * mcu_w;
    let y_base = my * mcu_h;
    let dx_end = (img_w - x_base).min(mcu_w);
    let dy_end = (img_h - y_base).min(mcu_h);

    if frame.components.len() == 1 {
        let y_buf = &comp_bufs[0];
        for dy in 0..dy_end {
            let row_off = ((y_base + dy) * img_w + x_base) * 4;
            let src_row = dy * 8;
            for dx in 0..dx_end {
                let g = y_buf[src_row + dx];
                let d = row_off + dx * 4;
                rgba[d..d + 4].copy_from_slice(&[g, g, g, 255]);
            }
        }
        return;
    }

    let c0 = frame.components[0];
    let c1 = frame.components[1];
    let c2 = frame.components[2];
    let s0 = (c0.h_samp as usize) * 8;
    let s1 = (c1.h_samp as usize) * 8;
    let s2 = (c2.h_samp as usize) * 8;
    let h0 = c0.h_samp as usize;
    let v0 = c0.v_samp as usize;
    let h1 = c1.h_samp as usize;
    let v1 = c1.v_samp as usize;
    let h2 = c2.h_samp as usize;
    let v2 = c2.v_samp as usize;

    for dy in 0..dy_end {
        let row_off = ((y_base + dy) * img_w + x_base) * 4;
        let y0 = (dy * v0) / frame.max_v;
        let y1 = (dy * v1) / frame.max_v;
        let y2 = (dy * v2) / frame.max_v;
        for dx in 0..dx_end {
            let x0 = (dx * h0) / frame.max_h;
            let x1 = (dx * h1) / frame.max_h;
            let x2 = (dx * h2) / frame.max_h;
            let y_val = comp_bufs[0][y0 * s0 + x0];
            let cb_val = comp_bufs[1][y1 * s1 + x1];
            let cr_val = comp_bufs[2][y2 * s2 + x2];
            let d = row_off + dx * 4;
            rgba[d..d + 4].copy_from_slice(&ycbcr_to_rgba(y_val, cb_val, cr_val));
        }
    }
}

fn ycbcr_to_rgba(y: u8, cb: u8, cr: u8) -> [u8; 4] {
    let yf = y as f32;
    let cbf = cb as f32 - 128.0;
    let crf = cr as f32 - 128.0;
    let r = (yf + 1.402 * crf).round().clamp(0.0, 255.0) as u8;
    let g = (yf - 0.344136 * cbf - 0.714136 * crf)
        .round()
        .clamp(0.0, 255.0) as u8;
    let b = (yf + 1.772 * cbf).round().clamp(0.0, 255.0) as u8;
    [r, g, b, 255]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode;

    struct JpegBitWriter {
        bytes: Vec<u8>,
        buf: u32,
        count: u8,
    }

    impl JpegBitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                buf: 0,
                count: 0,
            }
        }

        fn write_bits(&mut self, val: u32, n: u8) {
            if n == 0 {
                return;
            }
            for bit in (0..n).rev() {
                self.buf = (self.buf << 1) | ((val >> bit) & 1);
                self.count += 1;
                if self.count == 8 {
                    let byte = self.buf as u8;
                    self.bytes.push(byte);
                    if byte == 0xFF {
                        self.bytes.push(0x00);
                    }
                    self.buf = 0;
                    self.count = 0;
                }
            }
        }

        fn flush_with_ones(&mut self) {
            if self.count > 0 {
                let pad = 8 - self.count;
                self.write_bits((1u32 << pad) - 1, pad);
            }
        }
    }

    #[derive(Clone)]
    struct HuffEncEntry {
        code: u16,
        len: u8,
    }

    struct HuffSpec {
        counts: [u8; 16],
        symbols: Vec<u8>,
        enc: [Option<HuffEncEntry>; 256],
    }

    impl HuffSpec {
        fn build(sym_lengths: &[(u8, u8)]) -> Self {
            let mut sorted = sym_lengths.to_vec();
            sorted.sort_by_key(|&(sym, len)| (len, sym));

            let mut counts = [0u8; 16];
            let mut symbols = Vec::with_capacity(sorted.len());
            let mut enc = [const { None }; 256];

            let mut code: u16 = 0;
            let mut prev_len: u8 = 0;
            for (sym, len) in sorted {
                counts[(len - 1) as usize] += 1;
                symbols.push(sym);
                if prev_len > 0 && len > prev_len {
                    code <<= len - prev_len;
                }
                enc[sym as usize] = Some(HuffEncEntry { code, len });
                code += 1;
                prev_len = len;
            }

            Self {
                counts,
                symbols,
                enc,
            }
        }

        fn encode(&self, bw: &mut JpegBitWriter, sym: u8) {
            let e = self.enc[sym as usize]
                .as_ref()
                .expect("test symbol missing in Huffman table");
            bw.write_bits(e.code as u32, e.len);
        }
    }

    fn standard_dc_spec() -> HuffSpec {
        // Categories 0..=11: lengths 2..=9 (max 11 bits covers all 8-bit DC diffs).
        // 0: 2 bits (00)
        // 1: 3 bits (010), 2: 3 bits (011), 3: 3 bits (100), 4: 3 bits (101), 5: 3 bits (110)
        // 6: 4 bits, 7: 5 bits, 8: 6 bits, 9: 7 bits, 10: 8 bits, 11: 9 bits
        HuffSpec::build(&[
            (0, 2),
            (1, 3),
            (2, 3),
            (3, 3),
            (4, 3),
            (5, 3),
            (6, 4),
            (7, 5),
            (8, 6),
            (9, 7),
            (10, 8),
            (11, 9),
        ])
    }

    fn simple_ac_spec() -> HuffSpec {
        // Supports EOB (0x00), ZRL (0xF0), and all (run 0..=15, cat 1..=10) symbols.
        let mut syms = vec![(0x00, 2), (0x01, 2), (0x08, 6), (0xF0, 10)];
        for run in 0u8..=15 {
            for cat in 1u8..=10 {
                let sym = (run << 4) | cat;
                if sym != 0x01 && sym != 0x08 {
                    syms.push((sym, 10));
                }
            }
        }
        HuffSpec::build(&syms)
    }

    fn bit_category(val: i32) -> (u8, u32) {
        if val == 0 {
            return (0, 0);
        }
        let abs = val.unsigned_abs();
        let cat = (32 - abs.leading_zeros()) as u8;
        let bits = if val > 0 {
            val as u32
        } else {
            ((1i32 << cat) - 1 + val) as u32
        };
        (cat, bits)
    }

    fn encode_block(
        bw: &mut JpegBitWriter,
        zz_quantized: &[i32; 64],
        dc_pred: &mut i32,
        dc_spec: &HuffSpec,
        ac_spec: &HuffSpec,
    ) {
        let diff = zz_quantized[0] - *dc_pred;
        *dc_pred = zz_quantized[0];
        let (dc_cat, dc_bits) = bit_category(diff);
        dc_spec.encode(bw, dc_cat);
        bw.write_bits(dc_bits, dc_cat);

        let mut last_nonzero = 0usize;
        for (idx, &val) in zz_quantized.iter().enumerate().skip(1) {
            if val != 0 {
                last_nonzero = idx;
            }
        }

        let mut k = 1usize;
        while k <= last_nonzero {
            let mut zeros = 0usize;
            while k + zeros <= last_nonzero && zz_quantized[k + zeros] == 0 {
                zeros += 1;
            }
            while zeros >= 16 {
                ac_spec.encode(bw, 0xF0);
                zeros -= 16;
                k += 16;
            }
            k += zeros;
            let (cat, bits) = bit_category(zz_quantized[k]);
            let sym = ((zeros as u8) << 4) | cat;
            ac_spec.encode(bw, sym);
            bw.write_bits(bits, cat);
            k += 1;
        }
        if last_nonzero < 63 {
            ac_spec.encode(bw, 0x00);
        }
    }

    fn forward_dct_8x8(samples: &[u8; 64]) -> [i32; 64] {
        let mut natural = [0.0f64; 64];
        for v in 0..8 {
            let cv = if v == 0 { FRAC_1_SQRT_2 } else { 1.0 };
            for u in 0..8 {
                let cu = if u == 0 { FRAC_1_SQRT_2 } else { 1.0 };
                let mut sum = 0.0f64;
                for y in 0..8 {
                    let cy = (((2 * y + 1) * v) as f64 * PI / 16.0).cos();
                    for x in 0..8 {
                        let cx = (((2 * x + 1) * u) as f64 * PI / 16.0).cos();
                        sum += (samples[y * 8 + x] as f64 - 128.0) * cx * cy;
                    }
                }
                natural[v * 8 + u] = 0.25 * cu * cv * sum;
            }
        }
        let mut zz = [0i32; 64];
        for (k, &nat_idx) in ZIGZAG.iter().enumerate() {
            zz[k] = natural[nat_idx].round() as i32;
        }
        zz
    }

    fn push_marker_seg(out: &mut Vec<u8>, marker: u8, payload: &[u8]) {
        out.push(0xFF);
        out.push(marker);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(payload);
    }

    fn push_dqt_unit_8bit_multi(out: &mut Vec<u8>) {
        let mut p = vec![0u8];
        p.extend_from_slice(&[1u8; 64]);
        p.push(1u8);
        p.extend_from_slice(&[1u8; 64]);
        push_marker_seg(out, 0xDB, &p);
    }

    fn push_dht_multi(out: &mut Vec<u8>, dc: &HuffSpec, ac: &HuffSpec) {
        let mut p = Vec::new();
        for (class, id, spec) in [(0, 0, dc), (0, 1, dc), (1, 0, ac), (1, 1, ac)] {
            p.push((class << 4) | id);
            p.extend_from_slice(&spec.counts);
            p.extend_from_slice(&spec.symbols);
        }
        push_marker_seg(out, 0xC4, &p);
    }

    fn build_baseline_jpeg(
        width: u16,
        height: u16,
        components: &[(u8, u8, u8, u8)], // (id, h, v, quant_id)
        mcu_blocks: &[Vec<[i32; 64]>],   // per MCU, blocks in scan order
        restart_interval: u16,
        use_16bit_dqt: bool,
    ) -> Vec<u8> {
        let dc_spec = standard_dc_spec();
        let ac_spec = simple_ac_spec();

        let mut out = vec![0xFF, 0xD8];
        // APP0 JFIF + COM marker to exercise skipping metadata
        push_marker_seg(
            &mut out,
            0xE0,
            b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00",
        );
        push_marker_seg(&mut out, 0xFE, b"droplet-test");

        if use_16bit_dqt {
            let mut p = vec![0x10]; // Pq=1 (16-bit), Tq=0
            for _ in 0..64 {
                p.extend_from_slice(&1u16.to_be_bytes());
            }
            push_marker_seg(&mut out, 0xDB, &p);
        } else {
            push_dqt_unit_8bit_multi(&mut out);
        }

        let mut sof0 = vec![8]; // precision
        sof0.extend_from_slice(&height.to_be_bytes());
        sof0.extend_from_slice(&width.to_be_bytes());
        sof0.push(components.len() as u8);
        for &(id, h, v, qid) in components {
            sof0.push(id);
            sof0.push((h << 4) | v);
            sof0.push(qid);
        }
        push_marker_seg(&mut out, 0xC0, &sof0);

        push_dht_multi(&mut out, &dc_spec, &ac_spec);

        if restart_interval > 0 {
            push_marker_seg(&mut out, 0xDD, &restart_interval.to_be_bytes());
        }

        let mut sos = vec![components.len() as u8];
        for (idx, &(id, _, _, _)) in components.iter().enumerate() {
            sos.push(id);
            sos.push(if idx == 0 { 0x00 } else { 0x11 }); // luma: 0/0, chroma: 1/1
        }
        sos.extend_from_slice(&[0, 63, 0]);
        push_marker_seg(&mut out, 0xDA, &sos);

        let blocks_per_comp: Vec<usize> = components
            .iter()
            .map(|&(_, h, v, _)| (h as usize) * (v as usize))
            .collect();

        let mut bw = JpegBitWriter::new();
        let mut dc_pred = [0i32; 3];
        let mut rst_num = 0u8;

        for (mcu_idx, blocks) in mcu_blocks.iter().enumerate() {
            if restart_interval > 0 && mcu_idx > 0 && mcu_idx.is_multiple_of(restart_interval as usize)
            {
                bw.flush_with_ones();
                out.extend_from_slice(&bw.bytes);
                bw = JpegBitWriter::new();
                out.push(0xFF);
                out.push(0xD0 + (rst_num & 7));
                rst_num = (rst_num + 1) & 7;
                dc_pred = [0; 3];
            }

            let mut b_idx = 0usize;
            for (comp_idx, &count) in blocks_per_comp.iter().enumerate() {
                for _ in 0..count {
                    encode_block(
                        &mut bw,
                        &blocks[b_idx],
                        &mut dc_pred[comp_idx],
                        &dc_spec,
                        &ac_spec,
                    );
                    b_idx += 1;
                }
            }
        }
        bw.flush_with_ones();
        out.extend_from_slice(&bw.bytes);
        out.extend_from_slice(&[0xFF, 0xD9]); // EOI
        out
    }

    #[test]
    fn jpeg_grayscale_1_component_and_ac_dct() {
        // 5x3 grayscale pattern testing partial 8x8 block + AC coefficients + 16-bit DQT.
        let mut samples = [128u8; 64];
        for y in 0..8 {
            for x in 0..8 {
                samples[y * 8 + x] = if x < 3 { 40 } else { 220 };
            }
        }
        let zz = forward_dct_8x8(&samples);
        let jpeg = build_baseline_jpeg(5, 3, &[(1, 1, 1, 0)], &[vec![zz]], 0, true);

        let img = decode(&jpeg).expect("grayscale baseline JPEG must decode");
        assert_eq!(img.width, 5);
        assert_eq!(img.height, 3);
        assert_eq!(img.rgba.len(), 5 * 3 * 4);

        for y in 0..3 {
            for x in 0..5 {
                let idx = (y * 5 + x) * 4;
                let expected = if x < 3 { 40u8 } else { 220u8 };
                let actual = img.rgba[idx];
                assert!(
                    actual.abs_diff(expected) <= 2,
                    "pixel ({x},{y}) expected ~{expected}, got {actual}"
                );
                assert_eq!(img.rgba[idx + 1], actual);
                assert_eq!(img.rgba[idx + 2], actual);
                assert_eq!(img.rgba[idx + 3], 255);
            }
        }
    }

    #[test]
    fn jpeg_ycbcr_444_with_restart_and_byte_stuffing() {
        // Two 8x8 MCUs (16x8 image) with DRI=1 so RST0 sits between MCU 0 and MCU 1.
        // Also tests DC=-1 (all-1s bits -> triggers 0xFF 0x00 byte stuffing in entropy stream!).
        let make_dc = |val: i32| -> [i32; 64] {
            let mut b = [0i32; 64];
            b[0] = (val - 128) * 8;
            b
        };
        // MCU 0: Red-ish (Y=76, Cb=85, Cr=255), MCU 1: Blue-ish (Y=29, Cb=255, Cr=107)
        // Plus add a small block with repeated negative -1s or high values to ensure 0xFF stuffing path works.
        let mcu0 = vec![make_dc(76), make_dc(85), make_dc(255)];
        let mcu1 = vec![make_dc(29), make_dc(255), make_dc(107)];
        let jpeg = build_baseline_jpeg(
            16,
            8,
            &[(1, 1, 1, 0), (2, 1, 1, 0), (3, 1, 1, 0)],
            &[mcu0, mcu1],
            1,
            false,
        );

        let img = decode(&jpeg).expect("YCbCr 4:4:4 JPEG with DRI must decode");
        assert_eq!(img.width, 16);
        assert_eq!(img.height, 8);

        let px = |x: usize, y: usize| -> [u8; 4] {
            let i = (y * 16 + x) * 4;
            [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
        };
        // Left MCU -> Red
        assert_eq!(px(0, 0), [254, 0, 0, 255]);
        assert_eq!(px(7, 7), [254, 0, 0, 255]);
        // Right MCU -> Blue
        assert_eq!(px(8, 0), [0, 0, 254, 255]);
        assert_eq!(px(15, 7), [0, 0, 254, 255]);
    }

    #[test]
    fn jpeg_ycbcr_422_and_420_subsampling() {
        let make_dc = |val: i32| -> [i32; 64] {
            let mut b = [0i32; 64];
            b[0] = (val - 128) * 8;
            b
        };

        // 4:2:2 (16x8 MCU, cropped to 12x5): Y has 2x1 blocks (left black, right white), neutral Cb/Cr.
        let mcu_422 = vec![
            make_dc(0),   // Y block (0,0): x=0..8
            make_dc(255), // Y block (1,0): x=8..16
            make_dc(128), // Cb
            make_dc(128), // Cr
        ];
        let jpeg_422 = build_baseline_jpeg(
            12,
            5,
            &[(1, 2, 1, 0), (2, 1, 1, 0), (3, 1, 1, 0)],
            &[mcu_422],
            0,
            false,
        );
        let img_422 = decode(&jpeg_422).expect("4:2:2 JPEG must decode");
        assert_eq!(img_422.width, 12);
        assert_eq!(img_422.height, 5);
        assert_eq!(&img_422.rgba[0..4], &[0, 0, 0, 255]);
        let right_idx = (2 * 12 + 10) * 4;
        assert_eq!(&img_422.rgba[right_idx..right_idx + 4], &[255, 255, 255, 255]);

        // 4:2:0 (16x16 MCU, cropped to 12x10): Y has 2x2 blocks with distinct gray levels.
        let mcu_420 = vec![
            make_dc(0),   // Y (0,0) top-left
            make_dc(100), // Y (1,0) top-right
            make_dc(180), // Y (0,1) bottom-left
            make_dc(255), // Y (1,1) bottom-right
            make_dc(128), // Cb
            make_dc(128), // Cr
        ];
        let jpeg_420 = build_baseline_jpeg(
            12,
            10,
            &[(1, 2, 2, 0), (2, 1, 1, 1), (3, 1, 1, 1)],
            &[mcu_420],
            0,
            false,
        );
        let img_420 = decode(&jpeg_420).expect("4:2:0 JPEG must decode");
        assert_eq!(img_420.width, 12);
        assert_eq!(img_420.height, 10);
        let p420 = |x: usize, y: usize| -> [u8; 4] {
            let i = (y * 12 + x) * 4;
            [
                img_420.rgba[i],
                img_420.rgba[i + 1],
                img_420.rgba[i + 2],
                img_420.rgba[i + 3],
            ]
        };
        assert_eq!(p420(2, 2), [0, 0, 0, 255]);
        assert_eq!(p420(10, 2), [100, 100, 100, 255]);
        assert_eq!(p420(2, 9), [180, 180, 180, 255]);
        assert_eq!(p420(10, 9), [255, 255, 255, 255]);
    }

    #[test]
    fn jpeg_handles_byte_unstuffing_in_scan() {
        // DC=0 (2 bits '00') + AC[1]=255 (symbol 0x08 -> 6 bits '100000', followed by 8 bits '11111111' = 0xFF).
        let mut block = [0i32; 64];
        block[1] = 255;
        let jpeg = build_baseline_jpeg(8, 8, &[(1, 1, 1, 0)], &[vec![block]], 0, false);
        assert!(
            jpeg.windows(2).any(|w| w == [0xFF, 0x00]),
            "test stream should exercise 0xFF 0x00 stuffing"
        );
        let img = decode(&jpeg).expect("byte-stuffed JPEG must decode");
        let expected_x0 = (0.25 * FRAC_1_SQRT_2 * 255.0 * (PI / 16.0).cos() + 128.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        assert_eq!(&img.rgba[0..4], &[expected_x0, expected_x0, expected_x0, 255]);
    }

    #[test]
    fn jpeg_rejects_dimension_bomb_sof2_arithmetic_and_corrupt_streams() {
        // 1. Dimension bomb (10_000 x 10_000 > MAX_IMAGE_PIXELS)
        let mut bomb = vec![0xFF, 0xD8];
        let mut sof0_bomb = vec![8];
        sof0_bomb.extend_from_slice(&10_000u16.to_be_bytes());
        sof0_bomb.extend_from_slice(&10_000u16.to_be_bytes());
        sof0_bomb.extend_from_slice(&[1, 1, 0x11, 0]);
        push_marker_seg(&mut bomb, 0xC0, &sof0_bomb);
        bomb.extend_from_slice(&[0xFF, 0xD9]);
        let err = decode(&bomb).expect_err("dimension bomb must fail");
        assert!(err.message.contains("exceed limit"));

        // 2. Progressive JPEG (SOF2)
        let mut sof2 = vec![0xFF, 0xD8];
        push_marker_seg(&mut sof2, 0xC2, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]);
        let err = decode(&sof2).expect_err("SOF2 progressive JPEG must be rejected");
        assert!(err.message.contains("progressive"));

        // 3. Arithmetic coding (SOF9 / DAC)
        let mut sof9 = vec![0xFF, 0xD8];
        push_marker_seg(&mut sof9, 0xC9, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]);
        let err = decode(&sof9).expect_err("arithmetic JPEG must be rejected");
        assert!(err.message.contains("arithmetic"));

        let mut dac = vec![0xFF, 0xD8];
        push_marker_seg(&mut dac, 0xCC, &[0, 0]);
        assert!(decode(&dac).is_err());

        // 4. Truncated scan (cut off right before EOI / inside entropy data)
        let valid = build_baseline_jpeg(8, 8, &[(1, 1, 1, 0)], &[vec![[0; 64]]], 0, false);
        for cut in 1..valid.len() {
            assert!(
                decode(&valid[..cut]).is_err(),
                "truncated JPEG at {cut}/{} must fail cleanly",
                valid.len()
            );
        }

        // 5. Invalid marker
        let invalid_marker = [0xFF, 0xD8, 0xFF, 0xBB, 0x00, 0x02, 0xFF, 0xD9];
        assert!(decode(&invalid_marker).is_err());
    }

    #[test]
    fn jpeg_grayscale_multi_mcu_with_restart_intervals_and_wrap() {
        let make_dc = |val: i32| -> [i32; 64] {
            let mut b = [0i32; 64];
            b[0] = (val - 128) * 8;
            b
        };

        // 10 MCUs (80x8) with DRI=1 forces RST0..RST7 and wraps back to RST0 before MCU 9.
        let levels = [10i32, 35, 60, 85, 110, 135, 160, 185, 210, 240];
        let mcus: Vec<Vec<[i32; 64]>> = levels.iter().map(|&v| vec![make_dc(v)]).collect();
        let jpeg = build_baseline_jpeg(80, 8, &[(1, 1, 1, 0)], &mcus, 1, false);

        // Verify RST0..RST7 and wrapped RST0 markers are actually present in stream.
        for rst in 0u8..8 {
            let marker = [0xFF, 0xD0 + rst];
            assert!(
                jpeg.windows(2).any(|w| w == marker),
                "expected restart marker 0xFF{:02X} in stream",
                0xD0 + rst
            );
        }

        let img = decode(&jpeg).expect("multi-MCU grayscale JPEG with DRI wrap must decode");
        assert_eq!(img.width, 80);
        assert_eq!(img.height, 8);
        for (mcu_idx, &expected) in levels.iter().enumerate() {
            let x = mcu_idx * 8 + 3;
            let idx = (4 * 80 + x) * 4;
            assert_eq!(
                &img.rgba[idx..idx + 4],
                &[expected as u8, expected as u8, expected as u8, 255]
            );
        }
    }

    #[test]
    fn jpeg_ycbcr_444_without_dri_and_420_multi_mcu_with_dri() {
        let make_dc = |val: i32| -> [i32; 64] {
            let mut b = [0i32; 64];
            b[0] = (val - 128) * 8;
            b
        };

        // 4:4:4 without DRI, 15x9 (2x2 = 4 MCUs, partial right & bottom edges).
        let mcus_444 = vec![
            vec![make_dc(76), make_dc(85), make_dc(255)],   // top-left: Red
            vec![make_dc(150), make_dc(44), make_dc(21)],   // top-right: Green
            vec![make_dc(29), make_dc(255), make_dc(107)],  // bottom-left: Blue
            vec![make_dc(255), make_dc(128), make_dc(128)], // bottom-right: White
        ];
        let jpeg_444 = build_baseline_jpeg(
            15,
            9,
            &[(1, 1, 1, 0), (2, 1, 1, 1), (3, 1, 1, 1)],
            &mcus_444,
            0,
            false,
        );
        let img_444 = decode(&jpeg_444).expect("4:4:4 multi-MCU without DRI must decode");
        assert_eq!(img_444.width, 15);
        assert_eq!(img_444.height, 9);
        let p444 = |x: usize, y: usize| -> [u8; 4] {
            let i = (y * 15 + x) * 4;
            [
                img_444.rgba[i],
                img_444.rgba[i + 1],
                img_444.rgba[i + 2],
                img_444.rgba[i + 3],
            ]
        };
        assert_eq!(p444(0, 0), [254, 0, 0, 255]);
        assert_eq!(p444(14, 0), [0, 255, 1, 255]);
        assert_eq!(p444(0, 8), [0, 0, 254, 255]);
        assert_eq!(p444(14, 8), [255, 255, 255, 255]);

        // 4:2:0 with DRI=1, 20x18 (2x2 = 4 16x16 MCUs, partial right & bottom edges).
        let mcus_420 = vec![
            vec![
                make_dc(20),
                make_dc(20),
                make_dc(20),
                make_dc(20),
                make_dc(128),
                make_dc(128),
            ],
            vec![
                make_dc(90),
                make_dc(90),
                make_dc(90),
                make_dc(90),
                make_dc(128),
                make_dc(128),
            ],
            vec![
                make_dc(170),
                make_dc(170),
                make_dc(170),
                make_dc(170),
                make_dc(128),
                make_dc(128),
            ],
            vec![
                make_dc(240),
                make_dc(240),
                make_dc(240),
                make_dc(240),
                make_dc(128),
                make_dc(128),
            ],
        ];
        let jpeg_420_dri = build_baseline_jpeg(
            20,
            18,
            &[(1, 2, 2, 0), (2, 1, 1, 1), (3, 1, 1, 1)],
            &mcus_420,
            1,
            false,
        );
        let img_420 = decode(&jpeg_420_dri).expect("4:2:0 multi-MCU with DRI must decode");
        assert_eq!(img_420.width, 20);
        assert_eq!(img_420.height, 18);
        let p420 = |x: usize, y: usize| -> [u8; 4] {
            let i = (y * 20 + x) * 4;
            [
                img_420.rgba[i],
                img_420.rgba[i + 1],
                img_420.rgba[i + 2],
                img_420.rgba[i + 3],
            ]
        };
        assert_eq!(p420(5, 5), [20, 20, 20, 255]);
        assert_eq!(p420(19, 5), [90, 90, 90, 255]);
        assert_eq!(p420(5, 17), [170, 170, 170, 255]);
        assert_eq!(p420(19, 17), [240, 240, 240, 255]);

        // Truncating 4:2:0 multi-MCU DRI stream anywhere before EOI must fail cleanly.
        for cut in 1..jpeg_420_dri.len() {
            assert!(
                decode(&jpeg_420_dri[..cut]).is_err(),
                "truncated 4:2:0 DRI JPEG at {cut}/{} must fail",
                jpeg_420_dri.len()
            );
        }
    }

    #[test]
    fn jpeg_rejects_corrupt_restart_markers_headers_and_pixel_limit_boundaries() {
        let make_dc = |val: i32| -> [i32; 64] {
            let mut b = [0i32; 64];
            b[0] = (val - 128) * 8;
            b
        };

        // 1. Corrupt restart marker sequence (flip RST0 -> RST1).
        let mut bad_rst = build_baseline_jpeg(
            16,
            8,
            &[(1, 1, 1, 0)],
            &[vec![make_dc(50)], vec![make_dc(200)]],
            1,
            false,
        );
        let rst_pos = bad_rst
            .windows(2)
            .position(|w| w == [0xFF, 0xD0])
            .expect("RST0 present");
        bad_rst[rst_pos + 1] = 0xD1;
        let err = decode(&bad_rst).expect_err("wrong restart marker must fail");
        assert!(err.message.contains("restart marker"));

        // 2. Boundary dimension bombs just above MAX_IMAGE_PIXELS (64_000_000).
        for (w, h) in [(8_000u16, 8_001u16), (65_535u16, 977u16), (0u16, 8u16), (8u16, 0u16)] {
            let mut hdr = vec![0xFF, 0xD8];
            let mut sof0 = vec![8];
            sof0.extend_from_slice(&h.to_be_bytes());
            sof0.extend_from_slice(&w.to_be_bytes());
            sof0.extend_from_slice(&[1, 1, 0x11, 0]);
            push_marker_seg(&mut hdr, 0xC0, &sof0);
            hdr.extend_from_slice(&[0xFF, 0xD9]);
            assert!(
                decode(&hdr).is_err(),
                "dimensions {w}x{h} must be rejected"
            );
        }

        // 3. Oversubscribed Huffman table in DHT.
        let mut bad_dht = vec![0xFF, 0xD8];
        let mut dht_payload = vec![0x00]; // class=0, id=0
        dht_payload.push(3); // 3 codes of length 1 (max possible is 2!)
        dht_payload.extend_from_slice(&[0u8; 15]);
        dht_payload.extend_from_slice(&[0, 1, 2]);
        push_marker_seg(&mut bad_dht, 0xC4, &dht_payload);
        assert!(decode(&bad_dht).is_err());

        // 4. Excessive MCU blocks (> 10) and non-divisible subsampling ratios in SOF0.
        let mut too_many_blocks = vec![0xFF, 0xD8];
        let mut sof0_12 = vec![8, 0, 16, 0, 16, 3];
        sof0_12.extend_from_slice(&[1, 0x43, 0, 2, 0x11, 0, 3, 0x11, 0]); // 4*3 + 1 + 1 = 14 > 10
        push_marker_seg(&mut too_many_blocks, 0xC0, &sof0_12);
        assert!(decode(&too_many_blocks).is_err());

        let mut bad_ratio = vec![0xFF, 0xD8];
        let mut sof0_ratio = vec![8, 0, 16, 0, 16, 3];
        sof0_ratio.extend_from_slice(&[1, 0x31, 0, 2, 0x21, 0, 3, 0x11, 0]); // 3 not divisible by 2
        push_marker_seg(&mut bad_ratio, 0xC0, &sof0_ratio);
        assert!(decode(&bad_ratio).is_err());

        // 5. EOI before SOS.
        assert!(decode(&[0xFF, 0xD8, 0xFF, 0xD9]).is_err());
    }
}
