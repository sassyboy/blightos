///
/// Audio Playback and Recording Utilities
///
use crate::{Exception};
use crate::task::Task;
use crate::fileio::*;
use crate::time::TimeStampCounter;
use core::time::Duration;
use crate::*;

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

    pub fn samples_to_bytes(&self, samples: usize) -> usize {
        samples * (self.fmt.bit_depth as usize / 8) * self.fmt.channels as usize
    }

    pub fn bytes_to_samples(&self, bytes: usize) -> usize {
        bytes / ((self.fmt.bit_depth as usize / 8) * self.fmt.channels as usize)
    }

    pub fn duration_to_samples(&self, duration_ms: u32) -> usize {
        (self.fmt.sample_rate as usize * duration_ms as usize) / 1000
    }

    pub fn samples_to_duration(&self, samples: usize) -> u32 {
        ((samples * 1000) as u32) / self.fmt.sample_rate
    }

    pub fn duration_to_bytes(&self, duration_ms: u32) -> usize {
        self.duration_to_samples(duration_ms) * 
                (self.fmt.bit_depth as usize / 8) *
                self.fmt.channels as usize
    }

    pub fn bytes_to_duration(&self, bytes: usize) -> u32 {
        let samples = self.bytes_to_samples(bytes);
        self.samples_to_duration(samples)
    }
    /// Plays audio data through the default output device.
    /// If `sync` is true, this function will block until playback is complete.
    pub fn play(&mut self, data: &[u8], sync: bool) -> Result<(), Exception> {
        const CHUNK_MS: u32 = 1000;
        if !self.snd.is_open() {
            self.snd.open(&Path::from("audio:/output/pcm"),
                            File::MODE_STREAM | File::MODE_WRITE)?;
        }
        
        let total_time = self.bytes_to_duration(data.len());
        // println!("Playing audio data of length {} bytes ({}ms).",
        //         data.len(), total_time);
        let mut total_written = 0;
        // Write CHUNK_MS milliseconds worth of audio data at a time to avoid
        // overwhelming the output device and allow stopping playback if needed
        let chunk_size = self.duration_to_bytes(CHUNK_MS);
        // println!("Starting audio playback with chunk size {}", chunk_size);
        let mut tries = 0;
        let mut tsc = TimeStampCounter::new();
        let start_time = tsc.current_as_nanos() / 1_000_000;
        while total_written < data.len() {
            let end = usize::min(total_written + chunk_size, data.len());
            let wrrt = self.snd.write(&data[total_written..end]);
            let Ok(written) = wrrt else {
                // If we fail to write any data, try a few more times before
                // giving up
                let err = wrrt.err().unwrap();
                tries += 1;
                if tries >= 50 {
                    break;
                }
                println!("AudioWrite failed due to error: {:?}", err);
                Task::sleep(Duration::from_millis(10));
                continue;

            };
            if written == 0 {
                // Buffer full. Back off and try again
                let backoff_ms = CHUNK_MS as u64 / 2;
                Task::sleep(Duration::from_millis(backoff_ms));
                continue;
            } else {
                tries = 0;
            }
            total_written += written;
            if sync {
                let backoff_ms = self.bytes_to_duration(written) / 100 * 80;
                Task::sleep(Duration::from_millis(backoff_ms as u64));
            }
        }
        let end_time = tsc.current_as_nanos() / 1_000_000;
        let duration = (end_time - start_time) as u32;
        if sync && duration < total_time {
            // Sleep for the remaining time to ensure sync playback
            Task::sleep(Duration::from_millis(duration as u64));
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