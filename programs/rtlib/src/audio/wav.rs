///
/// WAV File Encoding and Decoding Utilities
///
/// WAV (RIFF) file format overview (concise):
///
/// - A WAV file is a RIFF container (little-endian) that contains a sequence of
///  "chunks".
/// - The file begins with a 12-byte RIFF header:
///     Bytes 0..4   = "RIFF"
///     Bytes 4..8   = <file size - 8> (u32, little-endian)
///     Bytes 8..12  = "WAVE"
/// - After the RIFF header come a series of chunks. Each chunk has an 8-byte
///   header:
///     Bytes 0..4   = chunk ID (e.g. "fmt " or "data")
///     Bytes 4..8   = chunk size (u32, little-endian)
///   followed by 'chunk size' bytes of chunk-specific payload.
/// - The important chunks for PCM audio:
///   * "fmt " chunk (format):
///       - Minimum 16 bytes for PCM:
///           Bytes 0..2   = audio format (u16), 1 = PCM
///           Bytes 2..4   = num channels (u16)
///           Bytes 4..8   = sample rate (u32)
///           Bytes 8..12  = byte rate (u32)
///           Bytes 12..14 = block align (u16)
///           Bytes 14..16 = bits per sample (u16)
///       - The chunk may be larger than 16 bytes (extended fmt), so only the
///         first 16 bytes are required for basic playback info.
///   * "data" chunk:
///       - Contains raw interleaved sample data with layout determined by fmt
///         (channels, bits per sample).
///       - Chunk size gives number of payload bytes in the data chunk.
/// - Chunks may be in any order. Parsers should iterate chunks until "fmt " and
///   "data" are found.
/// - Chunk sizes are u32. If a chunk size is odd, a pad byte may be present
///   (not handled explicitly here).
///
/// This implementation reads the RIFF header, scans chunks for "fmt " and "data",
/// extracts channels, sample_rate and bits_per_sample (bit_depth), and records
/// the data offset/size.
///
use alloc::{vec, vec::Vec};
use crate::{Exception, ErrorCode};
use crate::fileio::*;

pub struct WaveAudio {
    pub bit_depth:      u16,
    pub byte_rate:      u32,
    pub channels:       u16,
    pub sample_count:   u32,
    pub data:           Vec<u8>,
}

impl WaveAudio {
    pub const fn new() -> Self {
        Self {
            bit_depth:      0,
            byte_rate:      0,
            channels:       0,
            sample_count:   0,
            data:           Vec::new(),
        }
    }
    pub fn from_path(path: &Path) -> Result<Self, Exception> {
        let file = File::from_path(path)?;
        // Read RIFF header (12 bytes)
        let mut riff_header = [0u8; 12];
        file.read(&mut riff_header);
        if &riff_header[0..4] != b"RIFF" || &riff_header[8..12] != b"WAVE" {
            return Err(Exception::new(ErrorCode::InvalidData,
                                    "Not a valid RIFF/WAVE file"));
        }

        let mut bit_depth = 0u16;
        let mut byte_rate = 0u32;
        let mut channels = 0u16;
        let mut data = Vec::new();
        let mut data_format = 0u16;
        let mut data_size = 0u32;

        // Iterate chunks until we find "fmt " and "data"
        loop {
            let mut chunk_hdr = [0u8; 8];
            let len = file.read(&mut chunk_hdr);
            if len < 8 {
                break;
            }
            let id = &chunk_hdr[0..4];
            let size = u32::from_le_bytes(chunk_hdr[4..8].try_into().unwrap());

            match id {
                b"fmt " => {
                    // Read fmt chunk (at least 16 bytes expected)
                    // See comment above for field offsets inside fmt chunk.
                    let mut fmt_buf = vec![0u8; size as usize];
                    file.read(&mut fmt_buf);
                    if fmt_buf.len() >= 16 {
                        data_format = u16::from_le_bytes(fmt_buf[0..2].try_into().unwrap());
                        channels = u16::from_le_bytes(fmt_buf[2..4].try_into().unwrap());
                        byte_rate = u32::from_le_bytes(fmt_buf[4..8].try_into().unwrap());
                        bit_depth = u16::from_le_bytes(fmt_buf[14..16].try_into().unwrap());
                    }
                }
                b"data" => {
                    // Record data payload offset and size.
                    // Data is raw PCM (or other codec) bytes.
                    data_size = size;
                    data = vec![0u8; data_size as usize];
                    file.read(&mut data);
                    break;
                }
                _ => {
                    // skip unknown chunk - Todo: replace with seek if supported
                    let mut skip_buf = vec![0u8; size as usize];
                    file.read(&mut skip_buf);
                }
            }
        }

        if data_size == 0 {
            return Err(Exception::new(ErrorCode::InvalidData,
                                        "No data chunk found"));
        }
        if bit_depth == 0 || channels == 0 || byte_rate == 0 {
            return Err(Exception::new(ErrorCode::InvalidData,
                                        "Incomplete fmt chunk"));
        }
        if data_format != 1 {
            return Err(Exception::new(ErrorCode::Unsupported,
                                        "Only PCM format is supported"));
        }

        let bytes_per_sample = (bit_depth as u32 + 7) / 8;
        let sample_count = (data_size as u64 / (bytes_per_sample as u64 * channels as u64))
            .try_into()
            .unwrap_or(0);

        Ok(Self {
            bit_depth,
            byte_rate,
            channels,
            sample_count,
            data,
        })
    }
}