//! Pure-Rust DEFLATE (RFC 1951) decompression with zlib (RFC 1950) and gzip (RFC 1952)
//! wrappers. Shared by the PNG decoder (`droplet-image`) and HTTP `Content-Encoding`
//! (`droplet-net`). Every entry point takes `max_out`: output beyond it is an error raised
//! *before* the bytes are produced, so a decompression bomb never allocates past the limit.

use std::fmt;

/// Decompression failure (corrupt or truncated stream, checksum mismatch, size limit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InflateError {
    pub message: String,
}

impl InflateError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for InflateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InflateError {}

const LIMIT_MSG: &str = "decompressed data exceeds size limit";

/// IEEE CRC-32 (gzip trailer, PNG chunks), table-driven.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

pub fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for chunk in data.chunks(5552) {
        for &b in chunk {
            s1 += b as u32;
            s2 += s1;
        }
        s1 %= MOD_ADLER;
        s2 %= MOD_ADLER;
    }
    (s2 << 16) | s1
}

struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buf: u32,
    bit_cnt: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bit_cnt: 0,
        }
    }

    fn ensure_bits(&mut self, n: u8) -> Result<(), InflateError> {
        while self.bit_cnt < n {
            if self.byte_pos >= self.data.len() {
                return Err(InflateError::new("unexpected end of deflate stream"));
            }
            self.bit_buf |= (self.data[self.byte_pos] as u32) << self.bit_cnt;
            self.byte_pos += 1;
            self.bit_cnt += 8;
        }
        Ok(())
    }

    fn read_bits(&mut self, n: u8) -> Result<u32, InflateError> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure_bits(n)?;
        let val = self.bit_buf & ((1u32 << n) - 1);
        self.bit_buf >>= n;
        self.bit_cnt -= n;
        Ok(val)
    }

    fn align_to_byte(&mut self) {
        let discard = self.bit_cnt & 7;
        self.bit_buf >>= discard;
        self.bit_cnt -= discard;
    }

    fn read_aligned_u16_le(&mut self) -> Result<u16, InflateError> {
        self.align_to_byte();
        let b0 = self.read_bits(8)? as u8;
        let b1 = self.read_bits(8)? as u8;
        Ok(u16::from_le_bytes([b0, b1]))
    }

    fn read_aligned_bytes(&mut self, len: usize, out: &mut Vec<u8>) -> Result<(), InflateError> {
        self.align_to_byte();
        // Drain any whole buffered bytes first, then copy directly from slice.
        let mut remaining = len;
        while remaining > 0 && self.bit_cnt >= 8 {
            out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf >>= 8;
            self.bit_cnt -= 8;
            remaining -= 1;
        }
        if remaining > 0 {
            let end = self
                .byte_pos
                .checked_add(remaining)
                .ok_or_else(|| InflateError::new("deflate stored block overflow"))?;
            if end > self.data.len() {
                return Err(InflateError::new("truncated deflate stored block"));
            }
            out.extend_from_slice(&self.data[self.byte_pos..end]);
            self.byte_pos = end;
        }
        Ok(())
    }

    fn finish_aligned_byte_pos(&mut self) -> usize {
        self.align_to_byte();
        let buffered_bytes = (self.bit_cnt / 8) as usize;
        self.byte_pos.saturating_sub(buffered_bytes)
    }
}

const MAX_HUFFMAN_BITS: usize = 15;

struct HuffmanTree {
    counts: [u16; MAX_HUFFMAN_BITS + 1],
    symbols: Vec<u16>,
}

impl HuffmanTree {
    fn from_lengths(lengths: &[u8]) -> Result<Self, InflateError> {
        let mut counts = [0u16; MAX_HUFFMAN_BITS + 1];
        for &len in lengths {
            if len as usize > MAX_HUFFMAN_BITS {
                return Err(InflateError::new("invalid Huffman code length"));
            }
            counts[len as usize] += 1;
        }

        if counts[0] as usize == lengths.len() {
            return Ok(Self {
                counts,
                symbols: Vec::new(),
            });
        }

        let mut left: i32 = 1;
        for &count in counts.iter().take(MAX_HUFFMAN_BITS + 1).skip(1) {
            left <<= 1;
            left -= count as i32;
            if left < 0 {
                return Err(InflateError::new("oversubscribed Huffman code tree"));
            }
        }

        let mut offsets = [0u16; MAX_HUFFMAN_BITS + 1];
        for bits in 1..MAX_HUFFMAN_BITS {
            offsets[bits + 1] = offsets[bits] + counts[bits];
        }

        let total_used = (offsets[MAX_HUFFMAN_BITS] + counts[MAX_HUFFMAN_BITS]) as usize;
        let mut symbols = vec![0u16; total_used];
        for (sym, &len) in lengths.iter().enumerate() {
            if len != 0 {
                let idx = offsets[len as usize] as usize;
                offsets[len as usize] += 1;
                symbols[idx] = sym as u16;
            }
        }

        Ok(Self { counts, symbols })
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, InflateError> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: usize = 0;
        for len in 1..=MAX_HUFFMAN_BITS {
            code |= reader.read_bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[index + (code - first) as usize]);
            }
            index += count as usize;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(InflateError::new("invalid Huffman code in deflate stream"))
    }
}

fn fixed_huffman_trees() -> Result<(HuffmanTree, HuffmanTree), InflateError> {
    let mut lit_lengths = [0u8; 288];
    lit_lengths[..=143].fill(8);
    lit_lengths[144..=255].fill(9);
    lit_lengths[256..=279].fill(7);
    lit_lengths[280..=287].fill(8);
    let dist_lengths = [5u8; 32];
    Ok((
        HuffmanTree::from_lengths(&lit_lengths)?,
        HuffmanTree::from_lengths(&dist_lengths)?,
    ))
}

fn decode_dynamic_trees(reader: &mut BitReader<'_>) -> Result<(HuffmanTree, HuffmanTree), InflateError> {
    const CL_ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let hlit = reader.read_bits(5)? as usize + 257;
    let hdist = reader.read_bits(5)? as usize + 1;
    let hclen = reader.read_bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(InflateError::new("invalid dynamic Huffman header counts"));
    }

    let mut cl_lengths = [0u8; 19];
    for i in 0..hclen {
        cl_lengths[CL_ORDER[i]] = reader.read_bits(3)? as u8;
    }
    let cl_tree = HuffmanTree::from_lengths(&cl_lengths)?;

    let total = hlit + hdist;
    let mut lengths = Vec::with_capacity(total);
    while lengths.len() < total {
        let sym = cl_tree.decode(reader)?;
        match sym {
            0..=15 => lengths.push(sym as u8),
            16 => {
                let prev = *lengths
                    .last()
                    .ok_or_else(|| InflateError::new("invalid repeat code in Huffman lengths"))?;
                let rep = reader.read_bits(2)? as usize + 3;
                if lengths.len() + rep > total {
                    return Err(InflateError::new("Huffman code lengths overflow"));
                }
                lengths.resize(lengths.len() + rep, prev);
            }
            17 => {
                let rep = reader.read_bits(3)? as usize + 3;
                if lengths.len() + rep > total {
                    return Err(InflateError::new("Huffman zero-repeat lengths overflow"));
                }
                lengths.resize(lengths.len() + rep, 0);
            }
            18 => {
                let rep = reader.read_bits(7)? as usize + 11;
                if lengths.len() + rep > total {
                    return Err(InflateError::new("Huffman long-zero-repeat lengths overflow"));
                }
                lengths.resize(lengths.len() + rep, 0);
            }
            _ => return Err(InflateError::new("invalid code-length Huffman symbol")),
        }
    }

    if lengths[256] == 0 {
        return Err(InflateError::new("dynamic Huffman missing EOB symbol 256"));
    }

    let lit_tree = HuffmanTree::from_lengths(&lengths[..hlit])?;
    let dist_tree = HuffmanTree::from_lengths(&lengths[hlit..])?;
    Ok((lit_tree, dist_tree))
}

/// Decodes one raw DEFLATE stream from the start of `data`, appending to `out` (whose total
/// length may not exceed `max_out`). Returns the number of input bytes consumed, so a
/// wrapper can read the trailer that follows.
pub fn inflate_raw_into(data: &[u8], max_out: usize, out: &mut Vec<u8>) -> Result<usize, InflateError> {
    let mut reader = BitReader::new(data);
    loop {
        let bfinal = reader.read_bits(1)?;
        let btype = reader.read_bits(2)?;
        match btype {
            0 => {
                let len = reader.read_aligned_u16_le()?;
                let nlen = reader.read_aligned_u16_le()?;
                if len != !nlen {
                    return Err(InflateError::new("corrupt deflate stored block length"));
                }
                let block_len = len as usize;
                if out.len().saturating_add(block_len) > max_out {
                    return Err(InflateError::new(LIMIT_MSG));
                }
                reader.read_aligned_bytes(block_len, out)?;
            }
            1 | 2 => {
                let (lit_tree, dist_tree) = if btype == 1 {
                    fixed_huffman_trees()?
                } else {
                    decode_dynamic_trees(&mut reader)?
                };
                inflate_huffman_block(&mut reader, &lit_tree, &dist_tree, max_out, out)?;
            }
            _ => return Err(InflateError::new("reserved deflate block type BTYPE=3")),
        }
        if bfinal == 1 {
            break;
        }
    }
    Ok(reader.finish_aligned_byte_pos())
}

/// Raw DEFLATE without any wrapper (some servers send this for `Content-Encoding: deflate`).
pub fn inflate_raw(data: &[u8], max_out: usize) -> Result<Vec<u8>, InflateError> {
    let mut out = Vec::with_capacity(max_out.min(1 << 20));
    inflate_raw_into(data, max_out, &mut out)?;
    Ok(out)
}

/// Whether `data` starts with a valid zlib header (CMF/FLG check, deflate, no dictionary).
pub fn looks_like_zlib(data: &[u8]) -> bool {
    data.len() >= 2
        && (u16::from(data[0]) * 256 + u16::from(data[1])).is_multiple_of(31)
        && (data[0] & 0x0F) == 8
        && (data[0] >> 4) <= 7
        && (data[1] & 0x20) == 0
}

/// zlib stream (RFC 1950): header, DEFLATE data, big-endian Adler-32 trailer.
pub fn inflate_zlib(zlib: &[u8], max_out: usize) -> Result<Vec<u8>, InflateError> {
    if zlib.len() < 6 {
        return Err(InflateError::new("truncated zlib stream"));
    }
    if !(u16::from(zlib[0]) * 256 + u16::from(zlib[1])).is_multiple_of(31) {
        return Err(InflateError::new("invalid zlib header checksum"));
    }
    if (zlib[0] & 0x0F) != 8 || (zlib[0] >> 4) > 7 {
        return Err(InflateError::new("unsupported zlib compression parameters"));
    }
    if (zlib[1] & 0x20) != 0 {
        return Err(InflateError::new("unsupported zlib preset dictionary"));
    }

    let mut out = Vec::with_capacity(max_out.min(1 << 20));
    let end_pos = 2 + inflate_raw_into(&zlib[2..], max_out, &mut out)?;
    let Some(trailer) = zlib.get(end_pos..end_pos + 4) else {
        return Err(InflateError::new("missing zlib Adler-32 trailer"));
    };
    let expected_adler = u32::from_be_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    if adler32(&out) != expected_adler {
        return Err(InflateError::new("zlib Adler-32 checksum mismatch"));
    }
    Ok(out)
}

/// gzip data (RFC 1952), including several concatenated members. Each member's CRC-32 and
/// ISIZE are verified; NUL padding after the last member is tolerated, other trailing bytes
/// are an error.
pub fn gunzip(data: &[u8], max_out: usize) -> Result<Vec<u8>, InflateError> {
    const FHCRC: u8 = 0x02;
    const FEXTRA: u8 = 0x04;
    const FNAME: u8 = 0x08;
    const FCOMMENT: u8 = 0x10;
    const RESERVED: u8 = 0xE0;

    let mut out = Vec::with_capacity(max_out.min(1 << 20));
    let mut pos = 0usize;
    let mut members = 0usize;
    loop {
        let rest = &data[pos..];
        if members > 0 && rest.iter().all(|&b| b == 0) {
            break;
        }
        if rest.len() < 18 {
            return Err(InflateError::new("truncated gzip member"));
        }
        if rest[0] != 0x1F || rest[1] != 0x8B {
            return Err(InflateError::new("not gzip data (bad magic bytes)"));
        }
        if rest[2] != 8 {
            return Err(InflateError::new("unsupported gzip compression method"));
        }
        let flags = rest[3];
        if flags & RESERVED != 0 {
            return Err(InflateError::new("reserved gzip header flags set"));
        }
        let mut cur = 10usize;
        let take = |cur: &mut usize, n: usize| -> Result<(), InflateError> {
            *cur = cur
                .checked_add(n)
                .filter(|&end| end <= rest.len())
                .ok_or_else(|| InflateError::new("truncated gzip header"))?;
            Ok(())
        };
        if flags & FEXTRA != 0 {
            let xlen_bytes = rest
                .get(cur..cur + 2)
                .ok_or_else(|| InflateError::new("truncated gzip header"))?;
            let xlen = usize::from(u16::from_le_bytes([xlen_bytes[0], xlen_bytes[1]]));
            take(&mut cur, 2 + xlen)?;
        }
        for flag in [FNAME, FCOMMENT] {
            if flags & flag != 0 {
                let nul = rest[cur..]
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or_else(|| InflateError::new("truncated gzip header"))?;
                take(&mut cur, nul + 1)?;
            }
        }
        if flags & FHCRC != 0 {
            take(&mut cur, 2)?;
        }

        let member_start = out.len();
        let used = inflate_raw_into(&rest[cur..], max_out, &mut out)?;
        let trailer_at = cur + used;
        let Some(trailer) = rest.get(trailer_at..trailer_at + 8) else {
            return Err(InflateError::new("missing gzip CRC-32/ISIZE trailer"));
        };
        let crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
        let isize = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
        let member = &out[member_start..];
        if crc32(member) != crc {
            return Err(InflateError::new("gzip CRC-32 mismatch"));
        }
        if member.len() as u32 != isize {
            return Err(InflateError::new("gzip ISIZE mismatch"));
        }
        members += 1;
        pos += trailer_at + 8;
        if pos == data.len() {
            break;
        }
    }
    Ok(out)
}

fn inflate_huffman_block(
    reader: &mut BitReader<'_>,
    lit_tree: &HuffmanTree,
    dist_tree: &HuffmanTree,
    max_out: usize,
    out: &mut Vec<u8>,
) -> Result<(), InflateError> {
    const LEN_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const LEN_EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DIST_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DIST_EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];

    loop {
        let sym = lit_tree.decode(reader)?;
        if sym < 256 {
            if out.len() >= max_out {
                return Err(InflateError::new(LIMIT_MSG));
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let len_idx = (sym - 257) as usize;
            if len_idx >= LEN_BASE.len() {
                return Err(InflateError::new("invalid deflate length symbol"));
            }
            let length =
                LEN_BASE[len_idx] as usize + reader.read_bits(LEN_EXTRA[len_idx])? as usize;

            let dist_sym = dist_tree.decode(reader)? as usize;
            if dist_sym >= DIST_BASE.len() {
                return Err(InflateError::new("invalid deflate distance symbol"));
            }
            let dist =
                DIST_BASE[dist_sym] as usize + reader.read_bits(DIST_EXTRA[dist_sym])? as usize;
            if dist == 0 || dist > out.len() {
                return Err(InflateError::new("deflate back-reference out of bounds"));
            }
            if out.len().saturating_add(length) > max_out {
                return Err(InflateError::new(LIMIT_MSG));
            }
            for _ in 0..length {
                let b = out[out.len() - dist];
                out.push(b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflate_fixed_huffman_stream() {
        // Compress "HelloHello" with Fixed Huffman (BTYPE=01):
        // 'H'=72, 'e'=101, 'l'=108, 'l'=108, 'o'=111, then length=5 (sym 259, code 0000011 -> 7 bits),
        // distance=5 (sym 4, 5-bit code 00100 + 1 extra bit 0), then EOB=256 (7 bits 0000000).
        struct BitWriter {
            bytes: Vec<u8>,
            buf: u32,
            cnt: u8,
        }
        impl BitWriter {
            fn write_bits(&mut self, val: u32, n: u8) {
                self.buf |= val << self.cnt;
                self.cnt += n;
                while self.cnt >= 8 {
                    self.bytes.push((self.buf & 0xFF) as u8);
                    self.buf >>= 8;
                    self.cnt -= 8;
                }
            }
            fn write_huff_msb(&mut self, code: u32, n: u8) {
                for bit in (0..n).rev() {
                    self.write_bits((code >> bit) & 1, 1);
                }
            }
            fn finish(mut self) -> Vec<u8> {
                if self.cnt > 0 {
                    self.bytes.push((self.buf & 0xFF) as u8);
                }
                self.bytes
            }
        }

        let mut bw = BitWriter {
            bytes: vec![0x78, 0x9C],
            buf: 0,
            cnt: 0,
        };
        bw.write_bits(1, 1); // BFINAL=1
        bw.write_bits(1, 2); // BTYPE=01 (Fixed Huffman)
        for &ch in b"Hello" {
            // Fixed Huffman: 0..=143 use 8-bit codes starting at 0b00110000 (48)
            bw.write_huff_msb(48 + ch as u32, 8);
        }
        // Length 5 -> symbol 259 (256..=279 use 7-bit codes starting at 0)
        bw.write_huff_msb(259 - 256, 7);
        // Distance 5 -> symbol 4 (base=5, 1 extra bit = 0)
        bw.write_huff_msb(4, 5);
        bw.write_bits(0, 1);
        // EOB 256
        bw.write_huff_msb(0, 7);
        let mut z = bw.finish();
        z.extend_from_slice(&adler32(b"HelloHello").to_be_bytes());

        let out = inflate_zlib(&z, 64).expect("fixed huffman stream should inflate");
        assert_eq!(&out, b"HelloHello");
    }

    #[test]
    fn inflate_dynamic_huffman_stream() {
        let expected: Vec<u8> = (0..400)
            .map(|i| ((i * 13 + (i >> 3) * 7) % 17) as u8)
            .collect();
        let dyn_zlib: [u8; 67] = [
            120, 218, 237, 206, 201, 13, 0, 32, 12, 3, 193, 112, 132, 59, 129, 254, 171, 69, 194,
            46, 131, 207, 252, 188, 178, 88, 215, 224, 163, 192, 184, 225, 172, 48, 29, 184, 26,
            204, 2, 173, 67, 13, 208, 7, 51, 145, 153, 201, 76, 98, 102, 49, 147, 153, 49, 102,
            148, 25, 151, 255, 227, 77, 47, 201, 23, 12, 139,
        ];
        assert_eq!((dyn_zlib[2] >> 1) & 3, 2);
        let out = inflate_zlib(&dyn_zlib, expected.len()).expect("dynamic huffman must inflate");
        assert_eq!(out, expected);
    }

    // Vectors produced by CPython's zlib/gzip (reference implementation).
    const GZ_EMPTY: &[u8] = &[31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    const GZ_ASCII: &[u8] = &[31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 203, 72, 205, 201, 201, 87, 200, 64, 34, 83, 138, 242, 11, 114, 82, 75, 0, 177, 0, 94, 225, 25, 0, 0, 0];
    const GZ_CYRILLIC: &[u8] = &[31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 237, 204, 193, 9, 128, 48, 16, 68, 209, 86, 226, 93, 172, 113, 147, 28, 115, 178, 2, 91, 80, 33, 36, 130, 88, 195, 159, 142, 140, 69, 120, 219, 227, 48, 159, 199, 38, 163, 115, 82, 149, 230, 192, 77, 151, 77, 129, 85, 69, 81, 73, 166, 194, 21, 56, 70, 180, 43, 211, 70, 102, 99, 82, 105, 35, 86, 86, 228, 249, 190, 37, 224, 144, 67, 14, 57, 228, 208, 223, 208, 11, 74, 216, 182, 55, 180, 5, 0, 0];
    const GZ_TWO_MEMBERS: &[u8] = &[31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 75, 203, 44, 42, 46, 177, 6, 0, 16, 179, 148, 150, 6, 0, 0, 0, 31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 187, 176, 233, 98, 211, 133, 125, 23, 27, 46, 236, 187, 176, 19, 0, 50, 97, 60, 28, 12, 0, 0, 0];
    const GZ_WITH_FNAME: &[u8] = &[31, 139, 8, 8, 0, 0, 0, 0, 2, 255, 112, 97, 103, 101, 46, 104, 116, 109, 108, 0, 179, 41, 176, 203, 75, 204, 77, 77, 177, 209, 47, 176, 3, 0, 89, 231, 86, 211, 12, 0, 0, 0];
    const ZLIB_HTML: &[u8] = &[120, 156, 179, 201, 40, 201, 205, 177, 187, 176, 255, 98, 195, 133, 29, 23, 54, 93, 216, 122, 177, 201, 70, 31, 44, 4, 0, 177, 120, 13, 80];
    const RAW_HTML: &[u8] = &[179, 201, 40, 201, 205, 177, 43, 74, 44, 87, 72, 73, 77, 203, 73, 44, 73, 85, 184, 176, 255, 98, 195, 133, 29, 23, 54, 93, 216, 122, 177, 201, 70, 31, 44, 13, 0];

    const LIMIT: usize = 16 * 1024 * 1024;

    #[test]
    fn crc32_matches_reference_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32("Привет".as_bytes()), 0xa8eab199); // CPython zlib.crc32
    }

    #[test]
    fn gunzip_reference_vectors() {
        assert_eq!(gunzip(GZ_EMPTY, LIMIT).unwrap(), b"");
        assert_eq!(gunzip(GZ_ASCII, LIMIT).unwrap(), b"hello hello hello droplet");
        let ru = "Привет, мир! Быстрый браузер без мусора. ".repeat(20);
        assert_eq!(gunzip(GZ_CYRILLIC, LIMIT).unwrap(), ru.as_bytes());
        assert!(GZ_CYRILLIC.len() < ru.len() / 5, "vector should really be compressed");
        assert_eq!(gunzip(GZ_TWO_MEMBERS, LIMIT).unwrap(), "first;второй".as_bytes());
        assert_eq!(gunzip(GZ_WITH_FNAME, LIMIT).unwrap(), b"<p>named</p>");
    }

    #[test]
    fn gunzip_tolerates_nul_padding_but_not_garbage() {
        let mut padded = GZ_ASCII.to_vec();
        padded.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(gunzip(&padded, LIMIT).unwrap(), b"hello hello hello droplet");
        let mut garbage = GZ_ASCII.to_vec();
        garbage.extend_from_slice(b"junk-after-member-xxxxxxx");
        assert!(gunzip(&garbage, LIMIT).is_err());
    }

    #[test]
    fn gunzip_detects_corruption() {
        let n = GZ_ASCII.len();
        let mut bad_crc = GZ_ASCII.to_vec();
        bad_crc[n - 8] ^= 0xFF;
        assert_eq!(gunzip(&bad_crc, LIMIT).unwrap_err().message, "gzip CRC-32 mismatch");
        let mut bad_size = GZ_ASCII.to_vec();
        bad_size[n - 1] ^= 0x01;
        assert_eq!(gunzip(&bad_size, LIMIT).unwrap_err().message, "gzip ISIZE mismatch");
        assert!(gunzip(&GZ_ASCII[..n - 3], LIMIT).is_err(), "truncated trailer");
        assert!(gunzip(&GZ_ASCII[..12], LIMIT).is_err(), "truncated body");
        assert!(gunzip(b"<html>not gzip</html>", LIMIT).is_err());
        let mut reserved = GZ_ASCII.to_vec();
        reserved[3] |= 0x80;
        assert!(gunzip(&reserved, LIMIT).is_err());
    }

    #[test]
    fn zlib_and_raw_deflate_vectors() {
        assert!(looks_like_zlib(ZLIB_HTML));
        assert_eq!(inflate_zlib(ZLIB_HTML, LIMIT).unwrap(), "<html>привет</html>".as_bytes());
        assert!(!looks_like_zlib(RAW_HTML));
        assert_eq!(inflate_raw(RAW_HTML, LIMIT).unwrap(), "<html>raw deflate привет</html>".as_bytes());
    }

    /// Fixed-Huffman stream: one literal 0, then `matches` copies of length 258 / distance 1.
    /// 13 bits of input per 258 bytes of output — a classic decompression bomb.
    fn zero_bomb(matches: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        let (mut buf, mut cnt) = (0u64, 0u32);
        let mut put = |val: u64, n: u32, bytes: &mut Vec<u8>| {
            buf |= val << cnt;
            cnt += n;
            while cnt >= 8 {
                bytes.push(buf as u8);
                buf >>= 8;
                cnt -= 8;
            }
        };
        let rev = |code: u64, n: u32| (0..n).fold(0u64, |acc, i| acc | (((code >> i) & 1) << (n - 1 - i)));
        put(1, 1, &mut bytes); // BFINAL
        put(1, 2, &mut bytes); // BTYPE=01 fixed
        put(rev(0x30, 8), 8, &mut bytes); // literal 0
        for _ in 0..matches {
            put(rev(0xC5, 8), 8, &mut bytes); // length symbol 285 = 258
            put(0, 5, &mut bytes); // distance symbol 0 = 1
        }
        put(0, 7, &mut bytes); // EOB
        if cnt > 0 {
            bytes.push(buf as u8);
        }
        bytes
    }

    #[test]
    fn decompression_bomb_stops_at_the_limit() {
        let bomb = zero_bomb(81_300); // would expand to ~21 MB
        assert!(bomb.len() < 200_000);
        // Small bomb that fits: checks the generator itself is a valid stream.
        let ok = inflate_raw(&zero_bomb(10), 4096).unwrap();
        assert_eq!(ok.len(), 1 + 10 * 258);
        assert!(ok.iter().all(|&b| b == 0));

        let err = inflate_raw(&bomb, LIMIT).unwrap_err();
        assert_eq!(err.message, LIMIT_MSG);
        let mut gz = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0xFF];
        gz.extend_from_slice(&bomb);
        gz.extend_from_slice(&[0; 8]);
        assert_eq!(gunzip(&gz, LIMIT).unwrap_err().message, LIMIT_MSG);
    }
}

