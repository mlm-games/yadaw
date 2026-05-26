use anyhow::{Result, anyhow};
use std::path::Path;

use crate::model::clip::AudioClip;

fn new_audio_clip(
    name: String,
    start_beat: f64,
    length_beats: f64,
    samples: Vec<f32>,
    sample_rate: f32,
) -> AudioClip {
    AudioClip {
        name,
        start_beat,
        length_beats,
        samples,
        sample_rate,
        ..Default::default()
    }
}

pub fn import_audio_file(path: &Path, bpm: f32) -> Result<AudioClip> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "wav" => import_wav(path, bpm),
        "mp3" | "flac" | "ogg" | "m4a" | "aac" => import_with_symphonia(path, bpm),
        _ => import_wav(path, bpm)
            .or_else(|_| import_with_symphonia(path, bpm))
            .map_err(|_| anyhow!("Unsupported audio format: {}", extension)),
    }
}

fn import_wav(path: &Path, bpm: f32) -> Result<AudioClip> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            match spec.bits_per_sample {
                8 => {
                    // 8-bit PCM is typically unsigned
                    reader
                        .samples::<i8>() // hound doesn't provide u8; i8 reads centered -128..127
                        .map(|s| s.map(|v| (v as f32) / 128.0))
                        .collect::<Result<Vec<_>, _>>()?
                }
                16 => reader
                    .samples::<i16>()
                    .map(|s| s.map(|v| (v as f32) / 32768.0))
                    .collect::<Result<Vec<_>, _>>()?,
                24 => {
                    // 24-bit PCM decoded into i32 by hound; normalize by 2^23
                    reader
                        .samples::<i32>()
                        .map(|s| s.map(|v| (v as f32) / 8_388_608.0))
                        .collect::<Result<Vec<_>, _>>()?
                }
                32 => reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| (v as f32) / 2_147_483_648.0))
                    .collect::<Result<Vec<_>, _>>()?,
                bits => return Err(anyhow!("Unsupported PCM bit depth: {}", bits)),
            }
        }
    };

    // Downmix to mono if stereo; other channel counts -> average
    let channels = spec.channels.max(1) as usize;
    let mono_samples: Vec<f32> = if channels == 1 {
        samples
    } else {
        samples
            .chunks(channels)
            .map(|ch| ch.iter().copied().sum::<f32>() / channels as f32)
            .collect()
    };

    let trimmed_samples = trim_silence_end(&mono_samples, 0.001);

    let duration_seconds = trimmed_samples.len() as f64 / spec.sample_rate as f64;
    let duration_beats = duration_seconds * (bpm as f64 / 60.0);

    Ok(new_audio_clip(
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported Audio")
            .to_string(),
        0.0,
        duration_beats,
        trimmed_samples,
        spec.sample_rate as f32,
    ))
}

fn import_with_symphonia(path: &Path, bpm: f32) -> Result<AudioClip> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::formats::TrackType;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| anyhow!("No audio tracks found"))?;
    let track_id = track.id;
    let codec_params = track
        .codec_params
        .clone()
        .ok_or_else(|| anyhow!("Track has no codec parameters"))?;
    let audio_params = codec_params
        .audio()
        .ok_or_else(|| anyhow!("Track is not an audio track"))?;

    let sample_rate = audio_params
        .sample_rate
        .ok_or_else(|| anyhow!("Unknown sample rate"))?;
    let channels = audio_params
        .channels
        .clone()
        .map(|c| c.count())
        .unwrap_or(1);

    let mut decoder = crate::audio_codecs::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())?;

    let mut all_samples = Vec::new();

    loop {
        match format.next_packet() {
            Ok(Some(packet)) => {
                if packet.track_id != track_id {
                    continue;
                }
                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        let mut packet_samples = Vec::new();
                        decoded.copy_to_vec_interleaved(&mut packet_samples);
                        all_samples.extend_from_slice(&packet_samples);
                    }
                    Err(symphonia::core::errors::Error::DecodeError(e)) => {
                        log::warn!("decode error: {e}");
                        continue;
                    }
                    Err(e) => return Err(anyhow!("Opus decode failed: {}", e)),
                }
            }
            Ok(None) => break,
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        }
    }

    let mono_samples = if channels == 2 {
        all_samples
            .chunks(2)
            .map(|chunk| (chunk[0] + chunk.get(1).copied().unwrap_or(0.0)) / 2.0)
            .collect()
    } else {
        all_samples
    };

    let trimmed_samples = trim_silence_end(&mono_samples, 0.001);

    let duration_seconds = trimmed_samples.len() as f64 / sample_rate as f64;
    let duration_beats = duration_seconds * (bpm as f64 / 60.0);

    Ok(new_audio_clip(
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported Audio")
            .to_string(),
        0.0,
        duration_beats,
        trimmed_samples,
        sample_rate as f32,
    ))
}

fn trim_silence_end(samples: &[f32], threshold: f32) -> Vec<f32> {
    let mut last_non_silent = samples.len();
    for (i, &sample) in samples.iter().enumerate().rev() {
        if sample.abs() > threshold {
            last_non_silent = i + 1;
            break;
        }
    }
    samples[..last_non_silent].to_vec()
}
