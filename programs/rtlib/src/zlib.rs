use alloc::vec;
use alloc::vec::*;
use crate::{Exception, ErrorCode};

pub struct ZlibDecoder<'a> {
    input: &'a [u8],    // input zlib-compressed data
    pos: usize,         // position in input byte slice
    bitbuf: u32,        // bit buffer for reading bits from input
    bitcnt: u8,
    finished: bool,
}

impl<'a> ZlibDecoder<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        ZlibDecoder {
            input,
            pos: 0,
            bitbuf: 0,
            bitcnt: 0,
            finished: false,
        }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, Exception> {
        // Simple implementation: decompress everything into a temp buffer then serve requested portion.
        // This is fine for the uses in this file.
        let mut tmp = Vec::new();
        self.decompress_all(&mut tmp)?;
        // consume any remaining input future calls shouldn't produce anything new
        self.finished = true;
        let n = core::cmp::min(buf.len(), tmp.len());
        buf[..n].copy_from_slice(&tmp[..n]);
        Ok(n)
    }

    // Convenience used by the caller in this file.
    pub fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<(), Exception> {
        self.decompress_all(out)
    }

    // --- Bit-level helpers ---
    fn read_byte(&mut self) -> Result<u8, Exception> {
        if self.pos >= self.input.len() {
            Err(Exception::new(ErrorCode::UnexpectedEoF, ""))
        } else {
            let b = self.input[self.pos];
            self.pos += 1;
            Ok(b)
        }
    }

    // Ensure we have at least n bits in bitbuf, reading more bytes if needed.
    fn ensure_bits(&mut self, n: u8) -> Result<(), Exception> {
        while self.bitcnt < n {
            let b = self.read_byte()?;
            self.bitbuf |= (b as u32) << self.bitcnt;
            self.bitcnt += 8;
        }
        Ok(())
    }

    fn read_bits(&mut self, n: u8) -> Result<u32, Exception> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure_bits(n)?;
        let v = self.bitbuf & ((1u32 << n) - 1);
        self.bitbuf >>= n;
        self.bitcnt -= n;
        Ok(v)
    }

    fn byte_align(&mut self) {
        if self.bitcnt % 8 != 0 {
            let drop = self.bitcnt % 8;
            self.bitbuf >>= drop;
            self.bitcnt -= drop;
        }
    }

    // --- Huffman table construction & decoding ---
    // Build a fast lookup table for canonical Huffman codes.
    // lengths: slice of code lengths per symbol (0 = unused).
    // returns (table_symbols, table_lens, table_bits) where 
    // table_bits is the lookup table bits (power-of-two size = 1<<table_bits)
    fn build_huffman_table(lengths: &[u8]) -> Result<(Vec<i32>, Vec<u8>, usize), Exception> {
        // find max code length
        let mut max_len = 0usize;
        for &l in lengths {
            if l as usize > max_len {
                max_len = l as usize;
            }
        }
        if max_len == 0 {
            // empty huffman table
            return Err(Exception::new(ErrorCode::InvalidFormat, 
                                    "Empty Huffman table"));
        }
        if max_len > 15 {
            // huffman max bit length > 15 is not allowed in DEFLATE
            return Err(Exception::new(ErrorCode::InvalidFormat,
                                    "Huffman code length exceeds limit of 15"));
        }
        // bl_count[len] = number of codes of length len
        let mut bl_count = vec![0usize; max_len + 1];
        for &l in lengths {
            if l as usize > 0 {
                bl_count[l as usize] += 1;
            }
        }
        // compute next_code values
        let mut code = 0usize;
        let mut next_code = vec![0usize; max_len + 1];
        for bits in 1..=max_len {
            code = (code + bl_count[bits - 1]) << 1;
            next_code[bits] = code;
        }
        // assign codes to symbols and store reversed codes for LSB-first bitstream
        let table_bits = max_len; // we will build table sized 1<<max_len
        let table_size = 1usize << table_bits;
        let mut table_symbol = vec![-1i32; table_size];
        let mut table_len = vec![0u8; table_size];
        for (sym, &len) in lengths.iter().enumerate() {
            let len = len as usize;
            if len != 0 {
                let code = next_code[len];
                next_code[len] += 1;
                let rev = reverse_bits(code as u32, len as u32) as usize;
                // fill all entries that share the same prefix (pad with all combinations of remaining bits)
                let repeat = 1usize << (table_bits - len);
                let base = rev;
                for i in 0..repeat {
                    let idx = base | (i << len);
                    table_symbol[idx] = sym as i32;
                    table_len[idx] = len as u8;
                }
            }
        }
        Ok((table_symbol, table_len, table_bits))
    }

    fn decode_symbol(&mut self, table_symbol: &[i32], table_len: &[u8],
                    table_bits: usize) -> Result<i32, Exception> {
        // ensure we have at least table_bits bits
        let want = table_bits as u8;
        self.ensure_bits(want)?;
        let idx = (self.bitbuf & ((1u32 << want) - 1)) as usize;
        let sym = table_symbol[idx];
        if sym < 0 {
            // This shouldn't happen if table is well-formed
            Err(Exception::new(ErrorCode::InvalidFormat, "Invalid Huffman code"))
        } else {
            let len = table_len[idx];
            // drop len bits
            self.bitbuf >>= len;
            self.bitcnt -= len;
            Ok(sym)
        }
    }

    // --- Main decompression (zlib wrapper + DEFLATE) ---
    fn decompress_all(&mut self, out: &mut Vec<u8>) -> Result<(), Exception> {
        if self.finished {
            return Ok(());
        }
        // Parse zlib header (2 bytes)
        let cmf = self.read_byte()?;
        let flg = self.read_byte()?;
        // check compression method = 8 (deflate) and FCHECK
        if (cmf & 0x0F) != 8 {
            return Err(Exception::new(ErrorCode::NotSupported, 
                                    "zlib compression method not supported"));
        }
        let combined = ((cmf as u16) << 8) | (flg as u16);
        if combined % 31 != 0 {
            return Err(Exception::new(ErrorCode::InvalidFormat,
                                            "Invalid zlib header checksum"));
        }
        // Now DEFLATE stream
        loop {
            let final_bit = self.read_bits(1)?;
            let btype = self.read_bits(2)? as u8;
            match btype {
                0 => {
                    // No compression (stored)
                    self.byte_align();
                    // read LEN and NLEN (little-endian)
                    let lo = self.read_byte()? as u16;
                    let hi = self.read_byte()? as u16;
                    let len = hi << 8 | lo;
                    let nlo = self.read_byte()? as u16;
                    let nhi = self.read_byte()? as u16;
                    let nlen = nhi << 8 | nlo;
                    if len != (!nlen) {
                        // invalid uncompressed LEN/NLEN
                        return Err(Exception::new(ErrorCode::InvalidFormat, 
                                            "Invalid uncompressed LEN/NLEN"));
                    }
                    // copy len bytes
                    for _ in 0..len {
                        let b = self.read_byte()?;
                        out.push(b);
                    }
                }
                1 => {
                    // Fixed Huffman codes
                    // Build fixed literal/length lengths
                    let mut litlen_lengths = vec![0u8; 288];
                    for i in 0..=287 {
                        if i <= 143 {
                            litlen_lengths[i] = 8;
                        } else if i <= 255 {
                            litlen_lengths[i] = 9;
                        } else if i <= 279 {
                            litlen_lengths[i] = 7;
                        } else {
                            litlen_lengths[i] = 8;
                        }
                    }
                    let dist_lengths = vec![5u8; 32];
                    let (ll_table_sym, ll_table_len, ll_bits) = Self::build_huffman_table(&litlen_lengths)?;
                    let (d_table_sym, d_table_len, d_bits) = Self::build_huffman_table(&dist_lengths)?;
                    self.inflate_using_tables(&ll_table_sym, &ll_table_len, ll_bits, &d_table_sym, &d_table_len, d_bits, out)?;
                }
                2 => {
                    // Dynamic Huffman codes
                    let hlit = self.read_bits(5)? as usize + 257;
                    let hdist = self.read_bits(5)? as usize + 1;
                    let hclen = self.read_bits(4)? as usize + 4;
                    // read code length code lengths (19 codes) in specific order
                    let cl_order = [16usize,17,18,0,8,7,9,6,10,5,11,4,12,3,13,2,14,1,15];
                    let mut cl_lengths = vec![0u8; 19];
                    for i in 0..hclen {
                        let v = self.read_bits(3)? as u8;
                        cl_lengths[cl_order[i]] = v;
                    }
                    // build code length huffman
                    let (cl_table_sym, cl_table_len, cl_bits) =
                                        Self::build_huffman_table(&cl_lengths)?;
                    // read literal/length and distance code lengths
                    let mut lengths = Vec::with_capacity(hlit + hdist);
                    let mut i = 0usize;
                    while i < hlit + hdist {
                        // decode a symbol from cl table
                        let sym = {
                            // ensure bits for cl_bits
                            self.ensure_bits(cl_bits as u8)?;
                            let idx = (self.bitbuf & ((1u32 << cl_bits) - 1)) as usize;
                            let s = cl_table_sym[idx];
                            if s < 0 {
                                // invalid code length
                                return Err(
                                    Exception::new(ErrorCode::InvalidFormat,
                                                    "Invalid code length"));
                            }
                            let l = cl_table_len[idx];
                            self.bitbuf >>= l;
                            self.bitcnt -= l;
                            s as usize
                        };
                        match sym {
                            0..=15 => {
                                lengths.push(sym as u8);
                                i += 1;
                            }
                            16 => {
                                // copy previous 3-6 times, extra 2 bits
                                if lengths.is_empty() {
                                    return Err(
                                        Exception::new(ErrorCode::InvalidFormat,
                                            "No previous length to repeat"));
                                }
                                let extra = self.read_bits(2)? as usize;
                                let repeat = 3 + extra;
                                let prev = *lengths.last().unwrap();
                                for _ in 0..repeat {
                                    lengths.push(prev);
                                    i += 1;
                                }
                            }
                            17 => {
                                // repeat zero 3-10 times, extra 3 bits
                                let extra = self.read_bits(3)? as usize;
                                let repeat = 3 + extra;
                                for _ in 0..repeat {
                                    lengths.push(0);
                                    i += 1;
                                }
                            }
                            18 => {
                                // repeat zero 11-138 times, extra 7 bits
                                let extra = self.read_bits(7)? as usize;
                                let repeat = 11 + extra;
                                for _ in 0..repeat {
                                    lengths.push(0);
                                    i += 1;
                                }
                            }
                            _ => {
                                // invalid code length symbol
                                return Err(Exception::new(
                                                ErrorCode::InvalidFormat,
                                                "Invalid code length"));
                            }
                        }
                    }
                    if lengths.len() != hlit + hdist {
                        // bad lengths
                        return Err(Exception::new(ErrorCode::InvalidFormat,
                                            "Bad lengths"));
                    }
                    let litlen_lengths = &lengths[0..hlit];
                    let dist_lengths = &lengths[hlit..];
                    let (ll_table_sym, ll_table_len, ll_bits) = Self::build_huffman_table(litlen_lengths)?;
                    let (d_table_sym, d_table_len, d_bits) = Self::build_huffman_table(dist_lengths)?;
                    self.inflate_using_tables(&ll_table_sym, &ll_table_len, ll_bits, &d_table_sym, &d_table_len, d_bits, out)?;
                }
                _ => {
                    // reserved BTYPE
                    return Err(Exception::new(ErrorCode::InvalidFormat,
                                            "Invalid DEFLATE block type"));
                }
            }
            if final_bit != 0 {
                break;
            }
        }
        // After DEFLATE blocks, there may be an Adler32 (4 bytes) checksum. We'll ignore/consume it if present.
        // If there are at least 4 bytes left, consume them.
        // Note: PNG zlib stream contains an Adler32; we don't validate it here.
        if self.pos + 4 <= self.input.len() {
            self.pos += 4;
        }
        self.finished = true;
        Ok(())
    }

    fn inflate_using_tables(&mut self, ll_table_sym: &[i32],ll_table_len: &[u8],
        ll_bits: usize, d_table_sym: &[i32], d_table_len: &[u8], d_bits: usize,
        out: &mut Vec<u8>) -> Result<(), Exception> {
        // length base and extra bits for codes 257..285
        const LEN_BASE: [usize; 29] = [
            3,4,5,6,7,8,9,10,11,13,15,17,19,23,27,31,35,43,51,59,67,83,99,115,
            131,163,195,227,258
        ];
        const LEN_EXTRA: [u8; 29] = [
            0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4,5,5,5,5,0
        ];
        const DIST_BASE: [usize; 30] = [
            1,2,3,4,5,7,9,13,17,25,33,49,65,97,129,193,257,385,513,769,1025,
            1537,2049,3073,4097,6145,8193,12289,16385,24577
        ];
        const DIST_EXTRA: [u8; 30] = [
            0,0,0,0,1,1,2,2,3,3,4,4,5,5,6,6,7,7,8,8,9,9,10,10,11,11,12,12,13,13
        ];

        loop {
            // decode next literal/length symbol
            let lit_sym = self.decode_symbol(ll_table_sym, ll_table_len, ll_bits)?;
            if lit_sym < 256 {
                out.push(lit_sym as u8);
            } else if lit_sym == 256 {
                // end of block
                break;
            } else if lit_sym >= 257 && lit_sym <= 285 {
                let idx = (lit_sym as usize) - 257;
                if idx >= LEN_BASE.len() {
                    // invalid length symbol
                    return Err(Exception::new(ErrorCode::InvalidFormat,
                                                    "Invalid length symbol"));
                }
                let base = LEN_BASE[idx];
                let extra = LEN_EXTRA[idx];
                let extra_bits = if extra == 0 { 0 } else { self.read_bits(extra)? as usize };
                let length = base + extra_bits;
                // decode distance
                let dist_sym = self.decode_symbol(d_table_sym, d_table_len, d_bits)?;
                if dist_sym < 0 {
                    // invalid distance symbol
                    return Err(Exception::new(ErrorCode::InvalidFormat,
                                            "Invalid distance symbol"));
                }
                let dist_idx = dist_sym as usize;
                if dist_idx >= DIST_BASE.len() {
                    // invalid distance index
                    return Err(Exception::new(ErrorCode::InvalidFormat,
                                                    "Invalid distance index"));
                }
                let db = DIST_BASE[dist_idx];
                let de = DIST_EXTRA[dist_idx];
                let extrad = if de == 0 { 0 } else { self.read_bits(de)? as usize };
                let distance = db + extrad;
                if distance == 0 || distance > out.len() {
                    // invalid distance 
                    return Err(Exception::new(ErrorCode::InvalidFormat,
                                                    "Invalid distance"));
                }
                // copy length bytes from distance back
                let start = out.len() - distance;
                for i in 0..length {
                    let b = out[start + i % distance];
                    out.push(b);
                }
            } else {
                // invalid literal/length symbol
                return Err(Exception::new(ErrorCode::InvalidFormat,
                                            "Invalid literal/length symbol"));
            }
        }
        Ok(())
    }
}


// helper: reverse lowest 'bits' bits of value
fn reverse_bits(mut v: u32, bits: u32) -> u32 {
    let mut r = 0u32;
    for _ in 0..bits {
        r = (r << 1) | (v & 1);
        v >>= 1;
    }
    r
}