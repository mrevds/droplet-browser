//! RFC 1950 zlib and RFC 1951 DEFLATE decompression.

use crate::ImageError;

pub(crate) fn adler32(data: &[u8]) -> u32 {
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

    fn ensure_bits(&mut self, n: u8) -> Result<(), ImageError> {
        while self.bit_cnt < n {
            if self.byte_pos >= self.data.len() {
                return Err(ImageError::new("unexpected end of deflate stream"));
            }
            self.bit_buf |= (self.data[self.byte_pos] as u32) << self.bit_cnt;
            self.byte_pos += 1;
            self.bit_cnt += 8;
        }
        Ok(())
    }

    fn read_bits(&mut self, n: u8) -> Result<u32, ImageError> {
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

    fn read_aligned_u16_le(&mut self) -> Result<u16, ImageError> {
        self.align_to_byte();
        let b0 = self.read_bits(8)? as u8;
        let b1 = self.read_bits(8)? as u8;
        Ok(u16::from_le_bytes([b0, b1]))
    }

    fn read_aligned_bytes(&mut self, len: usize, out: &mut Vec<u8>) -> Result<(), ImageError> {
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
                .ok_or_else(|| ImageError::new("deflate stored block overflow"))?;
            if end > self.data.len() {
                return Err(ImageError::new("truncated deflate stored block"));
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
    fn from_lengths(lengths: &[u8]) -> Result<Self, ImageError> {
        let mut counts = [0u16; MAX_HUFFMAN_BITS + 1];
        for &len in lengths {
            if len as usize > MAX_HUFFMAN_BITS {
                return Err(ImageError::new("invalid Huffman code length"));
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
                return Err(ImageError::new("oversubscribed Huffman code tree"));
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

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, ImageError> {
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
        Err(ImageError::new("invalid Huffman code in deflate stream"))
    }
}

fn fixed_huffman_trees() -> Result<(HuffmanTree, HuffmanTree), ImageError> {
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

fn decode_dynamic_trees(reader: &mut BitReader<'_>) -> Result<(HuffmanTree, HuffmanTree), ImageError> {
    const CL_ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let hlit = reader.read_bits(5)? as usize + 257;
    let hdist = reader.read_bits(5)? as usize + 1;
    let hclen = reader.read_bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(ImageError::new("invalid dynamic Huffman header counts"));
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
                    .ok_or_else(|| ImageError::new("invalid repeat code in Huffman lengths"))?;
                let rep = reader.read_bits(2)? as usize + 3;
                if lengths.len() + rep > total {
                    return Err(ImageError::new("Huffman code lengths overflow"));
                }
                lengths.resize(lengths.len() + rep, prev);
            }
            17 => {
                let rep = reader.read_bits(3)? as usize + 3;
                if lengths.len() + rep > total {
                    return Err(ImageError::new("Huffman zero-repeat lengths overflow"));
                }
                lengths.resize(lengths.len() + rep, 0);
            }
            18 => {
                let rep = reader.read_bits(7)? as usize + 11;
                if lengths.len() + rep > total {
                    return Err(ImageError::new("Huffman long-zero-repeat lengths overflow"));
                }
                lengths.resize(lengths.len() + rep, 0);
            }
            _ => return Err(ImageError::new("invalid code-length Huffman symbol")),
        }
    }

    if lengths[256] == 0 {
        return Err(ImageError::new("dynamic Huffman missing EOB symbol 256"));
    }

    let lit_tree = HuffmanTree::from_lengths(&lengths[..hlit])?;
    let dist_tree = HuffmanTree::from_lengths(&lengths[hlit..])?;
    Ok((lit_tree, dist_tree))
}

pub(crate) fn inflate_zlib(zlib: &[u8], max_out: usize) -> Result<Vec<u8>, ImageError> {
    if zlib.len() < 6 {
        return Err(ImageError::new("truncated zlib stream"));
    }
    let cmf = zlib[0];
    let flg = zlib[1];
    if !((cmf as u16) * 256 + (flg as u16)).is_multiple_of(31) {
        return Err(ImageError::new("invalid zlib header checksum"));
    }
    if (cmf & 0x0F) != 8 || (cmf >> 4) > 7 {
        return Err(ImageError::new("unsupported zlib compression parameters"));
    }
    if (flg & 0x20) != 0 {
        return Err(ImageError::new("unsupported zlib preset dictionary"));
    }

    let mut reader = BitReader::new(&zlib[2..]);
    let mut out = Vec::with_capacity(max_out.min(1 << 20));

    loop {
        let bfinal = reader.read_bits(1)?;
        let btype = reader.read_bits(2)?;
        match btype {
            0 => {
                let len = reader.read_aligned_u16_le()?;
                let nlen = reader.read_aligned_u16_le()?;
                if len != !nlen {
                    return Err(ImageError::new("corrupt deflate stored block length"));
                }
                let block_len = len as usize;
                if out.len().saturating_add(block_len) > max_out {
                    return Err(ImageError::new("decompressed PNG data exceeds expected size"));
                }
                reader.read_aligned_bytes(block_len, &mut out)?;
            }
            1 | 2 => {
                let (lit_tree, dist_tree) = if btype == 1 {
                    fixed_huffman_trees()?
                } else {
                    decode_dynamic_trees(&mut reader)?
                };
                inflate_huffman_block(&mut reader, &lit_tree, &dist_tree, max_out, &mut out)?;
            }
            _ => return Err(ImageError::new("reserved deflate block type BTYPE=3")),
        }
        if bfinal == 1 {
            break;
        }
    }

    let end_pos = 2 + reader.finish_aligned_byte_pos();
    if end_pos + 4 > zlib.len() {
        return Err(ImageError::new("missing zlib Adler-32 trailer"));
    }
    let expected_adler = u32::from_be_bytes([
        zlib[end_pos],
        zlib[end_pos + 1],
        zlib[end_pos + 2],
        zlib[end_pos + 3],
    ]);
    let actual_adler = adler32(&out);
    if actual_adler != expected_adler {
        return Err(ImageError::new("zlib Adler-32 checksum mismatch"));
    }

    Ok(out)
}

fn inflate_huffman_block(
    reader: &mut BitReader<'_>,
    lit_tree: &HuffmanTree,
    dist_tree: &HuffmanTree,
    max_out: usize,
    out: &mut Vec<u8>,
) -> Result<(), ImageError> {
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
                return Err(ImageError::new("decompressed PNG data exceeds expected size"));
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let len_idx = (sym - 257) as usize;
            if len_idx >= LEN_BASE.len() {
                return Err(ImageError::new("invalid deflate length symbol"));
            }
            let length =
                LEN_BASE[len_idx] as usize + reader.read_bits(LEN_EXTRA[len_idx])? as usize;

            let dist_sym = dist_tree.decode(reader)? as usize;
            if dist_sym >= DIST_BASE.len() {
                return Err(ImageError::new("invalid deflate distance symbol"));
            }
            let dist =
                DIST_BASE[dist_sym] as usize + reader.read_bits(DIST_EXTRA[dist_sym])? as usize;
            if dist == 0 || dist > out.len() {
                return Err(ImageError::new("deflate back-reference out of bounds"));
            }
            if out.len().saturating_add(length) > max_out {
                return Err(ImageError::new("decompressed PNG data exceeds expected size"));
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
}
