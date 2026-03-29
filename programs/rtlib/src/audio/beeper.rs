///
/// Waveform Generator
///
/// 
use core::f32::consts::PI;
use alloc::vec::Vec;
use crate::sin;
use crate::*;

/// Maps musical notes to their corresponding frequencies in Hz at Octave 0.
/// To get the higher octaves, multiply the base frequency by 2 for each octave
/// increase.
pub enum Note {
    C,
    Cs,
    D,
    Ds,
    E,
    F,
    Fs,
    G,
    Gs,
    A,
    As,
    B
}

pub enum Waveform {
    Sine,
    Square,
    Triangle,
    Sawtooth
}

pub struct WaveformGenerator {
    sample_rate:   u32,
    bit_depth:     u16,
    channels:      u16,
}

impl WaveformGenerator {
    pub const fn new() -> Self {
        Self { sample_rate: 48000, bit_depth: 16, channels: 2 }
    }

    pub fn note_to_frequency(&self, note: Note, octave: u8) -> f32 {
        let base_freq: f32;
        match note {
            Note::C   => base_freq = 16.35,
            Note::Cs  => base_freq = 17.32,
            Note::D   => base_freq = 18.35,
            Note::Ds  => base_freq = 19.45,
            Note::E   => base_freq = 20.61,
            Note::F   => base_freq = 21.83,
            Note::Fs  => base_freq = 23.12,
            Note::G   => base_freq = 24.50,
            Note::Gs  => base_freq = 25.96,
            Note::A   => base_freq = 27.50,
            Note::As  => base_freq = 29.14,
            Note::B   => base_freq = 30.87
        }
        let coeff = 1 << octave; // 2^octave
        base_freq * coeff as f32
    }

    pub fn generate(&self, note: Note, octave: u8, duration_ms: u32,
                                            waveform: Waveform) -> Vec<u8> {
        let frequency = self.note_to_frequency(note, octave) as f64;
        let period = 1.0 / frequency; // Period of the note in seconds
        let rate = self.sample_rate as f32;
        let total_samples = (rate * (duration_ms as f32 / 1000.0)) as usize;
        let mut pcm = Vec::with_capacity(total_samples * self.channels as usize
                                            * (self.bit_depth as usize / 8));
        const MAX_AMP: f64 = 32000.0; // Max amplitude for 16-bit audio
        let tstep = 1.0 / rate as f64; // Time between samples in seconds
        let mut t: f64 = 0.0; // Current time in seconds
        for _i in 0..total_samples {
            let sample_value = match waveform {
                Waveform::Sine => 
                    sin((2.0f64 * PI as f64 * t / period) as f64),
                Waveform::Square => 
                    if t % period < period / 2.0 {
                        1.0 
                    } else {
                        -1.0
                    },
                Waveform::Triangle => 
                    if t % period < period / 2.0 {
                        -4.0 * t / period + 1.0
                    } else {
                        4.0 * t / period - 1.0
                    },
                Waveform::Sawtooth => 
                    2.0* t / period - 1.0,
            };

            // Multiple the sample_value by the amplitude and write it twice
            // for stereo output. Convert the float sample value to an integer
            // based on the bit depth.
            let int_sample = ((sample_value + 1.0) / 2.0 * MAX_AMP) as u16;
            pcm.extend_from_slice(&int_sample.to_le_bytes());
            pcm.extend_from_slice(&int_sample.to_le_bytes());
            t += tstep;
        }
        pcm
    }
}