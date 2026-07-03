use anyhow::{Result, anyhow};
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::Path;

use crate::model::clip::AudioClip;

fn new_audio_clip(
    name: String,
    start_beat: f64,
    length_beats: f64,
    samples: Vec<f32>,
    sample_rate: f32,
    source_hash: Option<u64>,
) -> AudioClip {
    AudioClip {
        name,
        start_beat,
        length_beats,
        samples,
        sample_rate,
        source_hash,
        ..Default::default()
    }
}

fn hash_source_bytes(data: &[u8]) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

pub fn import_audio_file(path: &Path, bpm: f32) -> Result<AudioClip> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let data = std::fs::read(path)?;

    import_audio_data(
        &path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported Audio"),
        &data,
        &extension,
        bpm,
    )
}

pub fn import_audio_data(
    name: &str,
    data: &[u8],
    extension: &str,
    bpm: f32,
) -> Result<AudioClip> {
    let source_hash = Some(hash_source_bytes(data));
    let ext = extension.to_lowercase();

    match ext.as_str() {
        "wav" => decode_wav_bytes(data, name, bpm, source_hash),
        "mp3" | "flac" | "ogg" | "m4a" | "aac" => {
            decode_with_symphonia_bytes(data, name, &ext, bpm, source_hash)
        }
        _ => decode_wav_bytes(data, name, bpm, source_hash)
            .or_else(|_| decode_with_symphonia_bytes(data, name, &ext, bpm, source_hash))
            .map_err(|_| anyhow!("Unsupported audio format: {}", extension)),
    }
}

fn decode_wav_bytes(
    data: &[u8],
    name: &str,
    bpm: f32,
    source_hash: Option<u64>,
) -> Result<AudioClip> {
    let mut reader = hound::WavReader::new(Cursor::new(data))?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => match spec.bits_per_sample {
            8 => reader.samples::<i8>()
                .map(|s| s.map(|v| (v as f32) / 128.0))
                .collect::<Result<Vec<_>, _>>()?,
            16 => reader.samples::<i16>()
                .map(|s| s.map(|v| (v as f32) / 32768.0))
                .collect::<Result<Vec<_>, _>>()?,
            24 => reader.samples::<i32>()
                .map(|s| s.map(|v| (v as f32) / 8_388_608.0))
                .collect::<Result<Vec<_>, _>>()?,
            32 => reader.samples::<i32>()
                .map(|s| s.map(|v| (v as f32) / 2_147_483_648.0))
                .collect::<Result<Vec<_>, _>>()?,
            bits => return Err(anyhow!("Unsupported PCM bit depth: {}", bits)),
        }
    };

    let channels = spec.channels.max(1) as usize;
    let mono_samples: Vec<f32> = if channels == 1 {
        samples
    } else {
        samples.chunks(channels)
            .map(|ch| ch.iter().copied().sum::<f32>() / channels as f32)
            .collect()
    };

    let trimmed_samples = trim_silence_end(&mono_samples, 0.001);
    let duration_seconds = trimmed_samples.len() as f64 / spec.sample_rate as f64;
    let duration_beats = duration_seconds * (bpm as f64 / 60.0);

    Ok(new_audio_clip(
        name.to_string(),
        0.0,
        duration_beats,
        trimmed_samples,
        spec.sample_rate as f32,
        source_hash,
    ))
}

fn decode_with_symphonia_bytes(
    data: &[u8],
    name: &str,
    extension: &str,
    bpm: f32,
    source_hash: Option<u64>,
) -> Result<AudioClip> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::formats::TrackType;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let mss = MediaSourceStream::new(Box::new(Cursor::new(data)), Default::default());

    let mut hint = Hint::new();
    hint.with_extension(extension);

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
                if packet.track_id != track_id { continue; }
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
                    Err(e) => return Err(anyhow!("Decode failed: {}", e)),
                }
            }
            Ok(None) => break,
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        }
    }

    let mono_samples = if channels == 2 {
        all_samples.chunks(2)
            .map(|chunk| (chunk[0] + chunk.get(1).copied().unwrap_or(0.0)) / 2.0)
            .collect()
    } else {
        all_samples
    };

    let trimmed_samples = trim_silence_end(&mono_samples, 0.001);
    let duration_seconds = trimmed_samples.len() as f64 / sample_rate as f64;
    let duration_beats = duration_seconds * (bpm as f64 / 60.0);

    Ok(new_audio_clip(
        name.to_string(),
        0.0,
        duration_beats,
        trimmed_samples,
        sample_rate as f32,
        source_hash,
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
