///
/// PNG image loading and decoding.
/// 
/// This is a minimal PNG loader that supports only 8-bit depth and
/// non-interlaced images.
/// It reads the PNG file, parses the header, collects IDAT chunks, and
/// decompresses the image data.
/// It does not support all PNG features (e.g. indexed color, interlacing) and
/// does not perform CRC checks or advanced error handling.
/// It is designed to be simple and self-contained, without relying on external
/// libraries.
///
/// Reference: https://www.libpng.org/pub/png/spec/1.2/PNG-Structure.html

// PNG file structure:
// - Signature: 8 bytes (fixed value)
// - Chunks: sequence of chunks until IEND
//
// Each chunk:
// - Length: 4 bytes (big-endian unsigned int) - length of chunk data
// - Type:   4 bytes (ASCII) - chunk type (e.g. IHDR, IDAT, IEND)
// - Data:   variable length (as specified by Length)
// - CRC:    4 bytes (not verified here)
//
// IHDR chunk format (the First Chunk)
//    Width:              4 bytes
//    Height:             4 bytes
//    Bit depth:          1 byte (bits per channel, e.g. 8-bit RBG = 24 b/pixel)
//    Color type:         1 byte
//    Compression method: 1 byte
//    Filter method:      1 byte
//    Interlace method:   1 byte
//
// IHDR - ColorType & BitDepth Combinations::
//   Color  Allowed    Interpretation of Each Pixel
//   Type  Bit Depths   
//   0     1,2,4,8,16  A grayscale sample.
//   2     8,16        An R,G,B triple. 
//   3     1,2,4,8     A palette index; a PLTE chunk must appear.
//   4     8,16        A grayscale sample, followed by an alpha sample.
//   6     8,16        An R,G,B triple, followed by an alpha sample.
//
// IDAT chunks contain compressed image data. Multiple IDAT chunks should be
// concatenated together before decompression.
//
// The PLTE chunk format:
// Contains from 1 to 256 palette entries, each a three-byte series of the form:
//   Red:   1 byte (0 = black, 255 = red)
//   Green: 1 byte (0 = black, 255 = green)
//   Blue:  1 byte (0 = black, 255 = blue)
// Note: The number of entries is determined from the chunk length. A chunk
// length not divisible by 3 is an error. 

use crate::{Exception, ErrorCode};
use crate::graphics::*;
use crate::fileio::*;
use crate::zlib::ZlibDecoder;
use alloc::{vec, vec::Vec};
use crate::*;

pub struct PngImage {
    pub img:    Image,
    // Some private PNG-specific fields
    comp:       u8, // Compression method (should be 0 for deflate)
    filt:       u8, // Filter method (should be 0 for standard PNG)
    // Raw/compressed PNG image data, i.e., IDAT chunks concatenated together.
    pub idat:   Vec<u8>,
    // PLTE chunk data (palette entries) for indexed PNGs.
    // Each entry is 3 bytes (R,G,B).
    pub plte:   Vec<u8>,
    // tRNS chunk data (transparency info)
    // For indexed PNGs, it contains alpha values for palette entries (1 byte per entry).
    pub trns:   Vec<u8>,
}

impl PngImage {
    /// Create an image descriptor from explicit dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {  
            img: Image {
                format:     ImageFormat::Png,
                width,
                height,
                bit_depth:  8,
                color_type: ColorType::RGBA,
                interlaced: false,
            },
            comp: 0,
            filt: 0,
            idat: Vec::new(),
            plte: Vec::new(),
            trns: Vec::new(),
        }
    }

    /// Number of pixels.
    pub fn pixel_count(&self) -> usize {
        (self.img.width as usize).saturating_mul(self.img.height as usize)
    }

    /// Required byte size for an RGBA8 buffer, returning None on overflow.
    pub fn required_buffer_size(&self) -> Option<usize> {
        self.pixel_count().checked_mul(4)
    }

    // Loads a PNG file from the given path, parses its header, and returns a
    // PngImage with the info filled in.
    // This does not decode the image data or validate the IDAT chunks.
    pub fn from_path(path: &Path) -> Result<Self, Exception> {
        let mut f = File::from_path(path, File::MODE_READ)?;

        // Parse PNG signature and chunks, extract IHDR and collect IDAT
        let mut sig = [0u8; 8];
        f.read(&mut sig)?;
        if sig != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
            return Err(Exception::new(ErrorCode::InvalidFormat,
                                            "invalid PNG signature"));
        }

        // Extract IHDR and IDAT chunks
        let mut png = Self::new(0, 0);
        let mut valid_ihdr = false;
        loop {
            // Read the chunk's length
            let mut len_b = [0u8; 4];
            if f.read(&mut len_b).unwrap_or(0) != 4 {
                return Err(Exception::new(ErrorCode::IOError,
                                "IO Error while reading chunk length"));
            }
            let len = u32::from_be_bytes(len_b) as usize;
            // Read the chunk's type
            let mut typ = [0u8; 4];
            if f.read(&mut typ).unwrap_or(0) != 4 {
                return Err(Exception::new(ErrorCode::IOError,
                                "IO Error while reading chunk type"));
            }
            let typ_s = &typ;
            // Read chunk's data
            let mut data = vec![0u8; len];
            if len > 0 {
                let ret = f.read(&mut data).unwrap_or(0);
                if ret != len {
                    println!("IO Error while reading chunk data: {} != {}", ret, len);
                    return Err(Exception::new(ErrorCode::IOError,
                                "IO Error while reading chunk data"));
                }
            }
            // Read CRC (skip verification)
            let mut crc = [0u8; 4];
            if f.read(&mut crc).unwrap_or(0) != 4 {
                return Err(Exception::new(ErrorCode::IOError,
                                "IO Error while reading chunk CRC"));
            }
            // Decode the chunk based on its type
            match typ_s {
            b"IHDR" => {
                if len != 13 {
                    return Err(Exception::new(ErrorCode::InvalidFormat, 
                                                        "invalid IHDR length"));
                }
                png.img.width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                png.img.height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                png.img.bit_depth  = data[8];
                png.comp = data[10];
                png.filt = data[11];
                png.img.interlaced = data[12] != 0;
                png.img.color_type = match data[9] {
                    0 => ColorType::Grayscale,
                    2 => ColorType::RGB,
                    3 => ColorType::Indexed,
                    4 => ColorType::GrayscaleAlpha,
                    6 => ColorType::RGBA,
                    _ => return Err(Exception::new(ErrorCode::NotSupported, 
                                                    "unsupported color type")),
                };
                // Support verification
                if png.comp != 0 || png.filt != 0 {
                    return Err(Exception::new(ErrorCode::NotSupported, 
                                    "unsupported compression/filter method"));
                }
                if png.img.width == 0 || png.img.height == 0 {
                    return Err(Exception::new(ErrorCode::InvalidFormat, 
                                                "invalid image dimensions"));
                }
                if png.img.bit_depth != 8 {
                    return Err(Exception::new(ErrorCode::NotSupported,
                                            "only 8-bit PNGs are supported"));
                }
                valid_ihdr = true;
            },
            b"IDAT" => {
                // Collect IDAT data for later decompression
                png.idat.extend_from_slice(&data);
            },
            b"IEND" => {
                break;
            },
            b"PLTE" => {
                png.plte = data;
                if png.plte.len() % 3 != 0 {
                    return Err(Exception::new(ErrorCode::InvalidFormat,
                                            "invalid PLTE length"));
                }
            },
            b"tRNS" => {
                png.trns = data;
            },
            _ => {
                // ignore other chunks
            }
            }
        } // End of the chunk-reading loop

        if !valid_ihdr {
            return Err(Exception::new(ErrorCode::InvalidFormat, "Invalid IHDR"));
        }
        png.idat.shrink_to_fit();
        Ok(png)
    }

    // Load the PNG into a user-provided buffer in RGBA8 format.
    // The buffer must be exactly width * height
    pub fn decode(&mut self) -> Result<Vec<RGBA>, Exception> {
        // Validate the PNG format
        if self.img.interlaced {
            return Err(Exception::new(ErrorCode::NotSupported,
                                            "interlaced PNGs not supported"));
        }
        
        // Validate output buffer size (check for overflow and exact size)
        let num_pixels = (self.img.width as usize) * (self.img.height as usize);
        let mut out_buf = vec![(0u8, 0u8, 0u8, 0u8); num_pixels];

        // Allocate a buffer large enough for the decoded frame
        // Note: Each scanline has 1 filter byte + (width * bpp)
        let bpp = self.img.color_type.bytes_per_pixel();
        
        let frame_buf_size = (self.img.height as usize)
                .saturating_mul(1 + (self.img.width as usize).saturating_mul(bpp));
        let mut decoded = vec![0u8; frame_buf_size];
        let frame_size = self.next_frame(&mut decoded)?;
        let data = &decoded[..frame_size];

        match self.img.color_type {
        ColorType::RGBA => {
            // Already RGBA8: copy directly (sizes must match)
            if data.len() != num_pixels * 4 {
                return Err(Exception::new(ErrorCode::InvalidFormat, 
                                                    "unexpected data size"));
            }
            for (i, pixel) in out_buf.iter_mut().enumerate() {
                let base = i * 4;
                pixel.0 = data[base];
                pixel.1 = data[base + 1];
                pixel.2 = data[base + 2];
                pixel.3 = data[base + 3];
            }
        }
        ColorType::RGB => {
            // Expand RGB -> RGBA (alpha = 255)
            if data.len() != num_pixels * 3 {
                return Err(Exception::new(ErrorCode::InvalidFormat, 
                                                    "unexpected RGB size"));
            }
            for (i, pixel) in out_buf.iter_mut().enumerate() {
                let base = i * 3;
                pixel.0 = data[base];
                pixel.1 = data[base + 1];
                pixel.2 = data[base + 2];
                pixel.3 = 0xFF;
            }
        }
        ColorType::Grayscale => {
            // Gray -> RGBA (replicate gray into RGB, alpha=255)
            if data.len() != num_pixels {
                return Err(Exception::new(ErrorCode::InvalidFormat, 
                                                "unexpected grayscale size"));
            }
            for (i, pixel) in out_buf.iter_mut().enumerate() {
                let g = data[i];
                pixel.0 = g;
                pixel.1 = g;
                pixel.2 = g;
                pixel.3 = 0xFF;
            }
        }
        ColorType::GrayscaleAlpha => {
            // Gray+Alpha -> RGBA (gray -> R,G,B ; keep A)
            if data.len() != num_pixels * 2 {
                return Err(Exception::new(ErrorCode::InvalidFormat, 
                                                "unexpected gray+alpha size"));
            }
            for (i, pixel) in out_buf.iter_mut().enumerate() {
                let g = data[i * 2];
                let a = data[i * 2 + 1];
                pixel.0 = g;
                pixel.1 = g;
                pixel.2 = g;
                pixel.3 = a;
            }
        }
        ColorType::Indexed => {
            // Map palette indices -> RGBA using the PLTE chunk.
            if self.plte.is_empty() {
                return Err(Exception::new(ErrorCode::InvalidFormat, 
                                            "missing PLTE for indexed PNG"));
            }
            // Each palette entry is 3 bytes (R,G,B)
            let entries = self.plte.len() / 3;
            if data.len() != num_pixels {
                return Err(Exception::new(ErrorCode::InvalidFormat, 
                                            "unexpected indexed image size"));
            }
            for (i, pixel) in out_buf.iter_mut().enumerate() {
                let idx = data[i] as usize;
                if idx >= entries {
                    return Err(Exception::new(ErrorCode::InvalidFormat,
                                                "palette index out of range"));
                }
                let p = idx * 3;
                pixel.0 = self.plte[p];
                pixel.1 = self.plte[p + 1];
                pixel.2 = self.plte[p + 2];
                pixel.3 = self.trns.get(idx).copied().unwrap_or(0xFF);
            }
        }
        }
        Ok(out_buf)
    }

    // Decompress IDAT data and reconstruct scanlines into the supplied buffer.
    // Writes raw pixel data (no filter bytes) into out and returns the frame's
    // actual size in bytes.
    // The caller is responsible for interpreting the pixel data according to
    // the color type and bit depth.
    fn next_frame(&mut self, out: &mut [u8]) -> Result<usize, Exception> {
        // Decompress zlib stream
        let mut z = ZlibDecoder::new(&self.idat[..]);
        let mut decompressed = Vec::new();
        z.read_to_end(&mut decompressed)?;

        let width = self.img.width as usize;
        let height = self.img.height as usize;
        let bpp = self.img.color_type.bytes_per_pixel();
        let row_bytes = width * bpp;
        let expected_len = height.checked_mul(1 + row_bytes)
                .ok_or_else(|| Exception::new(ErrorCode::InvalidFormat,
                                                        "image too large"))?;
        if decompressed.len() < expected_len {
            return Err(Exception::new(ErrorCode::InvalidFormat,
                                                "decompressed data too short"));
        }

        let out_len = height.checked_mul(row_bytes)
                .ok_or_else(|| Exception::new(ErrorCode::InvalidFormat,
                                                        "image too large"))?;
        if out.len() < out_len {
            return Err(Exception::new(ErrorCode::InvalidFormat,
                                                    "output buffer too small"));
        }

        // Process each scanline: first byte is filter type
        let mut prev_row = vec![0u8; row_bytes];
        let mut recon_row = vec![0u8; row_bytes];
        let mut src_off = 0usize;
        let mut dst_off = 0usize;
        for _row in 0..height {
            let filter = decompressed[src_off];
            src_off += 1;
            let scan = &decompressed[src_off..src_off + row_bytes];
            src_off += row_bytes;

            match filter {
                0 => {
                    // None
                    recon_row.copy_from_slice(scan);
                }
                1 => {
                    // Sub
                    for i in 0..row_bytes {
                        let left = if i >= bpp { recon_row[i - bpp] } else { 0 };
                        recon_row[i] = scan[i].wrapping_add(left);
                    }
                }
                2 => {
                    // Up
                    for i in 0..row_bytes {
                        recon_row[i] = scan[i].wrapping_add(prev_row[i]);
                    }
                }
                3 => {
                    // Average
                    for i in 0..row_bytes {
                        let left = if i >= bpp { recon_row[i - bpp] } else { 0 };
                        let up = prev_row[i];
                        let val = (left as u16 + up as u16) / 2;
                        recon_row[i] = scan[i].wrapping_add(val as u8);
                    }
                }
                4 => {
                    // Paeth predictor: See the PNG specification
                    for i in 0..row_bytes {
                        let a = 
                            if i >= bpp { recon_row[i - bpp] as i32} else { 0 };
                        let b = prev_row[i] as i32;
                        let c =
                            if i >= bpp { prev_row[i - bpp] as i32} else { 0 };
                        
                        let mut p = a + b - c;
                        let pa = (p - a).abs();
                        let pb = (p - b).abs();
                        let pc = (p - c).abs();
                        if pa <= pb && pa <= pc {
                            p = a;
                        } else if pb <= pc {
                            p = b;
                        } else {
                            p = c;
                        }

                        recon_row[i] = scan[i].wrapping_add(p as u8);
                    }
                }
                _ => {
                    return Err(Exception::new(ErrorCode::NotSupported,
                                                    "unsupported filter type"));
                }
            }

            // copy recon_row to output and swap prev_row
            out[dst_off..dst_off + row_bytes].copy_from_slice(&recon_row);
            prev_row.copy_from_slice(&recon_row);
            dst_off += row_bytes;
        }
        Ok(out_len)
    }
}

