use crate::state::AudioClip;
use anyhow::{anyhow, Result};
use std::path::Path;

fn new_audio_clip(
    name: String,
    start_beat: f64,
    length_beats: f64,
    samples: Vec<f32>,
    sample_rate: f32,
) -> crate::state::AudioClip {
    crate::state::AudioClip {
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
        "mp3" | "flac" | "ogg" => import_with_symphonia(path, bpm),
        _ => Err(anyhow!("Unsupported audio format: {}", extension)),
    }
}

fn import_wav(path: &Path, bpm: f32) -> Result<AudioClip> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    // Convert to mono f32
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            let max_val = (1 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| s as f32 / max_val))
                .collect::<Result<Vec<_>, _>>()?
        }
    };

    // Convert to mono if stereo
    let mono_samples = if spec.channels == 2 {
        samples
            .chunks(2)
            .map(|chunk| (chunk[0] + chunk.get(1).copied().unwrap_or(0.0)) / 2.0)
            .collect()
    } else {
        samples
    };

    // Trim silence from the end
    let trimmed_samples = trim_silence_end(&mono_samples, 0.001); // -60dB threshold

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
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    // Open the file
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // Probe the format
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();
    let probed =
        symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

    let mut format = probed.format;

    // Get the default track
    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("No audio tracks found"))?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| anyhow!("Unknown sample rate"))?;

    let channels = codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut decoder =
        symphonia::default::get_codecs().make(&codec_params, &DecoderOptions::default())?;

    let mut all_samples = Vec::new();
    let mut sample_buf = None;

    // Decode all packets
    loop {
        match format.next_packet() {
            Ok(packet) => {
                if packet.track_id() != track_id {
                    continue;
                }

                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        if sample_buf.is_none() {
                            let spec = *decoded.spec();
                            sample_buf =
                                Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
                        }

                        if let Some(buf) = &mut sample_buf {
                            buf.copy_interleaved_ref(decoded);
                            all_samples.extend_from_slice(buf.samples());
                        }
                    }
                    Err(symphonia::core::errors::Error::DecodeError(_)) => {
                        // Skip decode errors (can happen at end of some files)
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                // (Normal) end of file
                break;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                // End of stream
                break;
            }
            Err(e) => return Err(e.into()),
        }
    }

    // Convert to mono
    let mono_samples = if channels == 2 {
        all_samples
            .chunks(2)
            .map(|chunk| (chunk[0] + chunk.get(1).copied().unwrap_or(0.0)) / 2.0)
            .collect()
    } else {
        all_samples
    };

    // Trim silence from the end
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

// Helper function to trim silence from the end
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
