use crate::error::{EllaError, EllaResult};

const FRAME_MS: usize = 20;
const PRE_ROLL_MS: usize = 120;
const POST_ROLL_MS: usize = 180;
const MIN_SPEECH_RMS: f64 = 180.0;

#[derive(Debug, Clone)]
pub struct VadOutput {
    pub samples: Vec<i16>,
    pub input_ms: f64,
    pub speech_ms: f64,
    pub speech_detected: bool,
}

/// A small energy VAD used only after the learner presses stop. It removes
/// leading/trailing room noise before batch STT; it does not turn Canary into
/// a streaming model or decide when recording should stop.
pub fn trim_to_speech(samples: &[i16], sample_rate: u32) -> EllaResult<VadOutput> {
    if sample_rate == 0 {
        return Err(EllaError::Validation(
            "The microphone reported an invalid sample rate. Reconnect it and try again.".into(),
        ));
    }
    if samples.is_empty() {
        return Ok(VadOutput {
            samples: Vec::new(),
            input_ms: 0.0,
            speech_ms: 0.0,
            speech_detected: false,
        });
    }

    let frame_len = ((sample_rate as usize * FRAME_MS) / 1_000).max(1);
    let levels = samples.chunks(frame_len).map(rms).collect::<Vec<_>>();
    let peak = levels.iter().copied().fold(0.0_f64, f64::max);
    let mut opening = levels.iter().copied().take(10).collect::<Vec<_>>();
    opening.sort_by(|left, right| left.total_cmp(right));
    let noise_floor = opening.get(opening.len() / 2).copied().unwrap_or(0.0);
    let threshold = (noise_floor * 2.5).max(MIN_SPEECH_RMS).min(peak * 0.65);

    let voiced = if peak < MIN_SPEECH_RMS {
        Vec::new()
    } else {
        levels
            .iter()
            .enumerate()
            .filter_map(|(index, level)| (*level >= threshold).then_some(index))
            .collect::<Vec<_>>()
    };
    let input_ms = samples.len() as f64 * 1_000.0 / sample_rate as f64;
    let Some(first_frame) = voiced.first().copied() else {
        return Ok(VadOutput {
            samples: samples.to_vec(),
            input_ms,
            speech_ms: input_ms,
            speech_detected: false,
        });
    };
    let last_frame = voiced.last().copied().unwrap_or(first_frame);
    let pre_frames = PRE_ROLL_MS / FRAME_MS;
    let post_frames = POST_ROLL_MS / FRAME_MS;
    let first = first_frame.saturating_sub(pre_frames) * frame_len;
    let last = ((last_frame + post_frames + 1) * frame_len).min(samples.len());
    let trimmed = samples[first..last].to_vec();
    let speech_ms = trimmed.len() as f64 * 1_000.0 / sample_rate as f64;

    Ok(VadOutput {
        samples: trimmed,
        input_ms,
        speech_ms,
        speech_detected: true,
    })
}

fn rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let energy = samples
        .iter()
        .map(|sample| {
            let value = *sample as f64;
            value * value
        })
        .sum::<f64>();
    (energy / samples.len() as f64).sqrt()
}

pub fn pcm16_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    raw_pcm_to_wav(&pcm, sample_rate, channels)
}

pub fn raw_pcm_to_wav(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let byte_rate = sample_rate * channels as u32 * 2;
    let block_align = channels * 2;
    let mut wav = Vec::with_capacity(pcm.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vad_trims_silence_but_keeps_roll_around_speech() {
        let mut samples = vec![0; 8_000];
        samples.extend(vec![4_000; 8_000]);
        samples.extend(vec![0; 8_000]);
        let output = trim_to_speech(&samples, 16_000).unwrap();
        assert!(output.speech_detected);
        assert!(output.samples.len() > 8_000);
        assert!(output.samples.len() < samples.len());
    }

    #[test]
    fn vad_does_not_claim_silence_is_speech() {
        let output = trim_to_speech(&vec![20; 16_000], 16_000).unwrap();
        assert!(!output.speech_detected);
    }

    #[test]
    fn wav_wrapper_has_expected_header_and_length() {
        let wav = pcm16_wav(&[0, 10, -10, 2], 16_000, 1);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 52);
    }
}
