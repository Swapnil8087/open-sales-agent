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
    // -60 dBFS..-10 dBFS -> 0..1 (laptop mics are quiet; keep the meter lively)
    ((20.0 * rms.log10() + 60.0) / 50.0).clamp(0.0, 1.0)
}

pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect()
}

/// Stateful windowed-sinc resampler. Low-passes below the output Nyquist so
/// downsampling (48k -> 16k) does not alias into the voice band. Feeding chunks
/// equals feeding the whole signal.
pub struct Resampler {
    step: f64,
    cutoff: f64, // cycles per input sample
    half: usize, // kernel half-width in input samples
    pos: f64,
    pending: Vec<f32>,
}

impl Resampler {
    pub fn new(from_hz: u32, to_hz: u32) -> Self {
        let ratio = (to_hz as f64 / from_hz as f64).min(1.0);
        let cutoff = 0.5 * ratio * if ratio < 1.0 { 0.95 } else { 1.0 };
        let half = (8.0 / (2.0 * cutoff)).ceil() as usize;
        Self {
            step: from_hz as f64 / to_hz as f64,
            cutoff,
            half,
            pos: half as f64,
            pending: vec![0.0; half], // zero history so output 0 aligns with input 0
        }
    }

    fn weight(&self, x: f64) -> f64 {
        let h = self.half as f64;
        if x.abs() >= h {
            return 0.0;
        }
        let a = 2.0 * self.cutoff * x;
        let sinc = if a.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * a).sin() / (std::f64::consts::PI * a) };
        let window = 0.5 + 0.5 * (std::f64::consts::PI * x / h).cos();
        2.0 * self.cutoff * sinc * window
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.pending.extend_from_slice(input);
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 1);
        while self.pos + (self.half as f64) < self.pending.len() as f64 {
            let i0 = self.pos as usize;
            let (mut acc, mut wsum) = (0.0f64, 0.0f64);
            for k in (i0 + 1 - self.half)..=(i0 + self.half) {
                let w = self.weight(k as f64 - self.pos);
                acc += self.pending[k] as f64 * w;
                wsum += w;
            }
            out.push((acc / wsum) as f32); // unit DC gain
            self.pos += self.step;
        }
        let drop = (self.pos as usize).saturating_sub(self.half);
        self.pending.drain(..drop);
        self.pos -= drop as f64;
        out
    }
}

/// Largest absolute sample value.
pub fn peak(samples: &[i16]) -> u16 {
    samples.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0)
}

/// Scale quiet audio up to a healthy level (gain capped) so whisper hears it.
pub fn normalize(samples: &mut [i16]) {
    let p = peak(samples);
    if p == 0 {
        return;
    }
    let gain = (26_000.0 / p as f32).min(40.0);
    if gain <= 1.0 {
        return;
    }
    for v in samples.iter_mut() {
        *v = (*v as f32 * gain).round().clamp(-32767.0, 32767.0) as i16;
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
        assert!(quiet > 0.3, "quiet speech should still move the meter: {quiet}");
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
        assert!((out.len() as i64 - 1600).abs() <= 40, "len {}", out.len());
        // ignore the filter's start-up transient
        assert!(out[40..].iter().all(|v| (v - 0.5).abs() < 1e-3));
    }

    fn rms_of(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    fn tone(hz: f32, rate: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate).sin()).collect()
    }

    #[test]
    fn resampler_removes_frequencies_above_nyquist_instead_of_aliasing() {
        // 12 kHz is far above the 8 kHz Nyquist of 16 kHz output.
        let out = Resampler::new(48_000, 16_000).process(&tone(12_000.0, 48_000.0, 9600));
        assert!(rms_of(&out[100..]) < 0.05, "aliased energy {}", rms_of(&out[100..]));
    }

    #[test]
    fn resampler_keeps_speech_band_energy() {
        let out = Resampler::new(48_000, 16_000).process(&tone(1_000.0, 48_000.0, 9600));
        let r = rms_of(&out[100..]);
        assert!((r - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.05, "rms {r}");
    }

    #[test]
    fn normalize_boosts_quiet_audio_but_not_silence() {
        let mut quiet: Vec<i16> = (0..1000).map(|i| if i % 2 == 0 { 800 } else { -800 }).collect();
        normalize(&mut quiet);
        let peak = quiet.iter().map(|v| v.unsigned_abs()).max().unwrap();
        assert!(peak > 24_000 && peak <= 32_767, "peak {peak}");
        let mut silent = vec![0i16; 100];
        normalize(&mut silent);
        assert!(silent.iter().all(|v| *v == 0));
        let mut tiny = vec![3i16, -3, 3];
        normalize(&mut tiny); // gain is capped so noise is not blown up
        assert!(tiny.iter().all(|v| v.abs() <= 3 * 40));
    }

    #[test]
    fn peak_reports_largest_magnitude() {
        assert_eq!(peak(&[0, -500, 300]), 500);
        assert_eq!(peak(&[]), 0);
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
        let sig: Vec<f32> = (0..200).map(|i| (i as f32 * 0.05).sin()).collect();
        let out = Resampler::new(16_000, 16_000).process(&sig);
        assert!(out.len() >= 150);
        for (a, b) in out.iter().zip(&sig) {
            assert!((a - b).abs() < 1e-4);
        }
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
