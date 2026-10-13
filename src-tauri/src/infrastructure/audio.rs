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

/// Best index to split a streaming STT chunk: the start of the quietest 20 ms
/// frame inside `[search_from, search_to)`, so a cut lands between words
/// rather than through one. Falls back to `search_to` when the window is
/// degenerate.
pub fn quietest_cut_index(
    samples: &[i16],
    sample_rate: u32,
    search_from: usize,
    search_to: usize,
) -> usize {
    let search_to = search_to.min(samples.len());
    let frame_len = ((sample_rate as usize * FRAME_MS) / 1_000).max(1);
    if search_from + frame_len >= search_to {
        return search_to;
    }
    let mut best_index = search_to - frame_len;
    let mut best_rms = f64::MAX;
    let mut index = search_from;
    while index + frame_len <= search_to {
        let level = rms(&samples[index..index + frame_len]);
        if level < best_rms {
            best_rms = level;
            best_index = index;
        }
        index += frame_len;
    }
    // Cut in the middle of the quiet frame.
    best_index + frame_len / 2
}

/// Where to cut the audio heard so far when it ends in a pause, so that the
/// speech before the pause can be transcribed while the learner is quiet:
/// just after the last speech, keeping the quiet after it that the VAD keeps
/// (`POST_ROLL_MS`). None unless at least `min_speech_ms` of speech (first
/// voiced frame to last) is followed by at least `min_pause_ms` of quiet.
///
/// The noise floor is a low percentile of the frames rather than the opening
/// ones `trim_to_speech` uses, because audio cut out of a stream may open
/// mid-word.
pub fn pause_cut_index(
    samples: &[i16],
    sample_rate: u32,
    min_pause_ms: usize,
    min_speech_ms: usize,
) -> Option<usize> {
    if sample_rate == 0 || samples.is_empty() {
        return None;
    }
    let frame_len = ((sample_rate as usize * FRAME_MS) / 1_000).max(1);
    let levels = samples.chunks(frame_len).map(rms).collect::<Vec<_>>();
    let peak = levels.iter().copied().fold(0.0_f64, f64::max);
    if peak < MIN_SPEECH_RMS {
        return None;
    }
    let mut ordered = levels.clone();
    ordered.sort_by(|left, right| left.total_cmp(right));
    let noise_floor = ordered[ordered.len() / 10];
    let threshold = (noise_floor * 2.5).max(MIN_SPEECH_RMS).min(peak * 0.65);
    let first = levels.iter().position(|level| *level >= threshold)?;
    let last = levels.iter().rposition(|level| *level >= threshold)?;
    let speech_ms = (last + 1 - first) * FRAME_MS;
    let quiet_ms = (levels.len() - 1 - last) * FRAME_MS;
    if speech_ms < min_speech_ms || quiet_ms < min_pause_ms {
        return None;
    }
    Some(((last + 1 + POST_ROLL_MS / FRAME_MS) * frame_len).min(samples.len()))
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

    /// `ms` of a tone loud enough to be speech, then `quiet_ms` of room noise.
    fn said_then_quiet(ms: usize, quiet_ms: usize) -> Vec<i16> {
        let speech = (0..16 * ms).map(|index| if index % 32 < 16 { 3_000 } else { -3_000 });
        let room = (0..16 * quiet_ms).map(|index| if index % 7 == 0 { 40 } else { -40 });
        speech.chain(room).collect()
    }

    #[test]
    fn a_pause_after_enough_speech_is_cut_just_after_the_speech() {
        let samples = said_then_quiet(1_500, 700);
        let cut = pause_cut_index(&samples, 16_000, 600, 1_000).expect("a pause");
        // 1.5 s of speech and the VAD's 180 ms after it, to a 20 ms frame.
        assert_eq!(cut, 16 * (1_500 + 180));
    }

    #[test]
    fn no_cut_in_a_short_pause_after_too_little_speech_or_in_silence() {
        assert_eq!(pause_cut_index(&said_then_quiet(1_500, 400), 16_000, 600, 1_000), None, "pause too short");
        assert_eq!(pause_cut_index(&said_then_quiet(600, 900), 16_000, 600, 1_000), None, "one word");
        assert_eq!(pause_cut_index(&said_then_quiet(0, 2_000), 16_000, 600, 1_000), None, "silence");
        assert_eq!(pause_cut_index(&said_then_quiet(2_000, 0), 16_000, 600, 1_000), None, "still talking");
    }

    #[test]
    fn a_cut_after_quiet_that_opened_the_audio_still_needs_the_speech() {
        let mut samples = said_then_quiet(0, 800);
        samples.extend(said_then_quiet(1_200, 700));
        let cut = pause_cut_index(&samples, 16_000, 600, 1_000).expect("a pause");
        assert_eq!(cut, 16 * (800 + 1_200 + 180));
    }

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
