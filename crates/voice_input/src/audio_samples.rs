pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

pub fn resample_mono(samples: &[f32], sample_rate: u32) -> Result<Vec<f32>, String> {
    if !(8_000..=192_000).contains(&sample_rate) {
        return Err(format!("Unsupported microphone sample rate: {sample_rate}"));
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err("Microphone supplied non-finite samples".into());
    }
    if sample_rate == WHISPER_SAMPLE_RATE {
        return Ok(samples.to_vec());
    }
    let output_length = samples
        .len()
        .checked_mul(WHISPER_SAMPLE_RATE as usize)
        .ok_or("Recording length overflow")?
        / sample_rate as usize;
    let cutoff = (f64::from(WHISPER_SAMPLE_RATE) / f64::from(sample_rate)).min(1.0) * 0.94;
    let mut output = Vec::with_capacity(output_length);
    // A low-pass sinc filter prevents speech above 8 kHz from folding into the
    // recognizer's input when typical 44.1/48 kHz microphones are downsampled.
    let radius = 32i64;
    for output_index in 0..output_length {
        let position =
            output_index as f64 * f64::from(sample_rate) / f64::from(WHISPER_SAMPLE_RATE);
        let center = position.floor() as i64;
        let mut weighted = 0.0;
        let mut weight_sum = 0.0;
        for source_index in (center - radius)..=(center + radius) {
            let Ok(source_index) = usize::try_from(source_index) else {
                continue;
            };
            let Some(sample) = samples.get(source_index) else {
                continue;
            };
            let distance = position - source_index as f64;
            if distance.abs() > radius as f64 {
                continue;
            }
            let argument = std::f64::consts::PI * distance * cutoff;
            let sinc = if argument.abs() < 1e-8 {
                1.0
            } else {
                argument.sin() / argument
            };
            let window = 0.5 + 0.5 * (std::f64::consts::PI * distance / radius as f64).cos();
            let weight = cutoff * sinc * window;
            weighted += f64::from(*sample) * weight;
            weight_sum += weight;
        }
        output.push(if weight_sum.abs() > 1e-8 {
            (weighted / weight_sum) as f32
        } else {
            0.0
        });
    }
    Ok(output)
}

#[derive(Default)]
pub struct Downmixer {
    sum: f64,
    pending_channels: usize,
}

impl Downmixer {
    pub fn push(&mut self, sample: f32, channels: usize) -> Option<f32> {
        if channels == 0 {
            return None;
        }
        self.sum += f64::from(sample);
        self.pending_channels += 1;
        if self.pending_channels == channels {
            let mono = (self.sum / channels as f64) as f32;
            self.sum = 0.0;
            self.pending_channels = 0;
            Some(mono)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_keeps_frames_across_callback_boundaries() {
        let mut mixer = Downmixer::default();
        assert_eq!(mixer.push(0.8, 2), None);
        assert_eq!(mixer.push(-0.8, 2), Some(0.0));
        assert_eq!(mixer.push(0.4, 2), None);
        assert_eq!(mixer.push(0.6, 2), Some(0.5));
    }

    #[test]
    fn resampling_preserves_duration_and_dc() -> Result<(), String> {
        for rate in [8_000, 16_000, 44_100, 48_000, 96_000] {
            let samples = vec![0.25; rate as usize / 10];
            let output = resample_mono(&samples, rate)?;
            assert_eq!(output.len(), 1600);
            assert!(output.iter().all(|sample| (*sample - 0.25).abs() < 1e-5));
        }
        Ok(())
    }

    #[test]
    fn downsampling_rejects_above_nyquist_energy() -> Result<(), String> {
        let samples: Vec<_> = (0..4800)
            .map(|index| (2.0 * std::f32::consts::PI * 12_000.0 * index as f32 / 48_000.0).sin())
            .collect();
        let output = resample_mono(&samples, 48_000)?;
        let middle = output
            .get(32..output.len().saturating_sub(32))
            .ok_or("Output too short")?;
        let rms =
            (middle.iter().map(|sample| sample * sample).sum::<f32>() / middle.len() as f32).sqrt();
        assert!(rms < 0.01, "aliasing RMS: {rms}");
        Ok(())
    }

    #[test]
    fn invalid_and_empty_audio() -> Result<(), String> {
        assert!(resample_mono(&[f32::NAN], 48_000).is_err());
        assert!(resample_mono(&[0.0], 0).is_err());
        assert!(resample_mono(&[], 48_000)?.is_empty());
        Ok(())
    }
}
