//! Microphone capture (cpal) plus the pure signal helpers around it.

use std::path::Path;

pub const SAMPLE_RATE: u32 = 16_000;

/// Average interleaved channels down to mono.
pub fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Map RMS of a chunk to a 0..1 meter value suitable for the visualizer.
pub fn level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    if rms <= 0.0 {
        return 0.0;
    }
    // -50 dBFS..0 dBFS -> 0..1
    ((20.0 * rms.log10() + 50.0) / 50.0).clamp(0.0, 1.0)
}

pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect()
}

/// Stateful linear resampler; feeding chunks equals feeding the whole signal.
pub struct Resampler {
    step: f64,
    pos: f64,
    pending: Vec<f32>,
}

impl Resampler {
    pub fn new(from_hz: u32, to_hz: u32) -> Self {
        Self { step: from_hz as f64 / to_hz as f64, pos: 0.0, pending: Vec::new() }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.pending.extend_from_slice(input);
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 1);
        while self.pos + 1.0 < self.pending.len() as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(self.pending[i] * (1.0 - frac) + self.pending[i + 1] * frac);
            self.pos += self.step;
        }
        let drop = (self.pos as usize).min(self.pending.len());
        self.pending.drain(..drop);
        self.pos -= drop as f64;
        out
    }
}

pub fn write_wav(path: &Path, samples: &[i16]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for s in samples {
        w.write_sample(*s).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix_to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(downmix_to_mono(&[0.3, 0.4], 1), vec![0.3, 0.4]);
        assert!(downmix_to_mono(&[], 2).is_empty());
    }

    #[test]
    fn level_is_bounded_and_monotonic() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0.0; 100]), 0.0);
        let quiet = level(&[0.01; 100]);
        let loud = level(&[0.3; 100]);
        assert!(quiet > 0.0 && quiet < loud);
        assert!(level(&[5.0; 100]) <= 1.0);
    }

    #[test]
    fn i16_conversion_clamps() {
        assert_eq!(f32_to_i16(&[0.0, 1.0, -1.0, 2.0, -2.0]), vec![0, 32767, -32767, 32767, -32767]);
    }

    #[test]
    fn resampler_48k_to_16k_length_and_dc() {
        let mut r = Resampler::new(48_000, 16_000);
        let out = r.process(&vec![0.5; 4800]);
        assert!((out.len() as i64 - 1600).abs() <= 1, "len {}", out.len());
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn resampler_chunked_equals_whole() {
        let sig: Vec<f32> = (0..9000).map(|i| ((i as f32) * 0.01).sin()).collect();
        let whole = Resampler::new(44_100, 16_000).process(&sig);
        let mut r = Resampler::new(44_100, 16_000);
        let mut chunked = Vec::new();
        for c in sig.chunks(777) {
            chunked.extend(r.process(c));
        }
        assert_eq!(whole.len(), chunked.len());
        for (a, b) in whole.iter().zip(&chunked) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn resampler_same_rate_is_passthrough_ish() {
        let sig = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let out = Resampler::new(16_000, 16_000).process(&sig);
        assert_eq!(&out[..], &sig[..out.len()]);
        assert!(out.len() >= 4);
    }

    #[test]
    fn wav_roundtrip_is_16k_mono() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.wav");
        let data: Vec<i16> = (0..1600).map(|i| (i % 100) as i16).collect();
        write_wav(&p, &data).unwrap();
        let mut rd = hound::WavReader::open(&p).unwrap();
        let spec = rd.spec();
        assert_eq!((spec.channels, spec.sample_rate, spec.bits_per_sample), (1, 16_000, 16));
        let back: Vec<i16> = rd.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(back, data);
    }
}
