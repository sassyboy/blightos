///
/// Audio Playback and Recording Utilities
///
use crate::{Exception};
use crate::task::Task;
use crate::fileio::File;
use core::time::Duration;

pub struct AudioFormat {
    pub channels: u16,
    pub sample_rate: u32,
    pub bit_depth: u16
}

/// Basic struct that handles the audio output device if available
pub struct Playback {
    snd: File,
    fmt: AudioFormat
}

impl Playback {
    /// Initializes the audio output device
    pub const fn new() -> Self {
        Self {
            snd: File::new(),
            fmt: AudioFormat {
                channels: 2,
                sample_rate: 48000,
                bit_depth: 16
            }
        }
    }

    pub fn set_format(&mut self, fmt: AudioFormat) {
        self.fmt = fmt;
    }

    pub fn get_format(&self) -> &AudioFormat {
        &self.fmt
    }

    pub fn duration_to_samples(&self, duration_ms: u32) -> usize {
        (self.fmt.sample_rate as usize * duration_ms as usize) / 1000
    }

    pub fn duration_to_bytes(&self, duration_ms: u32) -> usize {
        self.duration_to_samples(duration_ms) * 
                (self.fmt.bit_depth as usize / 8) *
                self.fmt.channels as usize
    }

    /// Plays audio data through the default output device.
    /// If `sync` is true, this function will block until playback is complete.
    pub fn play(&mut self, data: &[u8], sync: bool) -> Result<(), Exception> {
        if !self.snd.is_open() {
            self.snd.open("audio:/output/pcm")?;
        }
        // Write 1s worth of audio data at a time to avoid overwhelming the
        // output device. Back off for 20ms in between if the `sync` flag is set.
        let mut total_written = 0;
        let chunk_size = self.duration_to_bytes(1000);
        // println!("Starting audio playback with chunk size {}", chunk_size);
        let mut tries = 0;
        while total_written < data.len() {
            let end = usize::min(total_written + chunk_size, data.len());
            let written = self.snd.write(&data[total_written..end]);
            if written == 0 {
                // If we fail to write any data, try a few more times before
                // giving up
                tries += 1;
                if tries >= 50 {
                    break;
                }
                Task::sleep(Duration::from_millis(5));
                continue;
            } else {
                tries = 0;
            }
            // println!("Wrote {} bytes to audio output", written);
            total_written += written;
            if sync {
                // Sleep for 5ms to allow the output device to catch up
                Task::sleep(Duration::from_millis(5));
            }
        }
        Ok(())
    }

    /// Stops audio playback
    pub fn stop(&self) {
        // Code to stop audio playback
    }
}

pub mod beeper;
pub mod wav;