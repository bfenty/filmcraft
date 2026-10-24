//! Raw ADTS `.aac` files import as audio clips (#1291). File ▸ Import and the Media Browser
//! offer `.aac`, and FilmCraft's own AAC export writes ADTS, but `audio::opener` never matched
//! the extension, so every import failed with "unrecognised format". The decode is checked
//! against ffmpeg's, channel by channel, with per-channel tones so a swapped channel shows up.

mod common;

use common::*;

const RATE: u32 = 48_000;

/// A 1-second stereo ADTS `.aac`: left 440 Hz, right 880 Hz.
fn adts_stereo(ff: &std::path::Path) -> Option<std::path::PathBuf> {
    fixture(
        ff,
        "adts_1291_stereo.aac",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000:duration=1",
            "-filter_complex",
            "[0:a][1:a]amerge=inputs=2,afade=t=in:st=0:d=0.05[out]",
            "-map",
            "[out]",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-f",
            "adts",
        ],
    )
}

/// Largest sample difference over the first `len` samples, best over a small lag either way
/// (the two decoders may place the AAC priming differently by a few samples).
fn diff(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len()).saturating_sub(64);
    // Best over a small lag either way (the two decoders may place the AAC priming
    // differently by a few samples): each lag is scored on its own, and the most
    // forgiving lag wins — mixing lags per sample would penalize a correct alignment.
    let mut worst = f32::MAX;
    for lag in -3..=3i64 {
        let mut lag_worst = 0.0f32;
        for i in 0..len {
            let j = (i as i64 + lag).clamp(0, b.len() as i64 - 1) as usize;
            lag_worst = lag_worst.max((a[i] - b[j]).abs());
        }
        worst = worst.min(lag_worst);
    }
    worst
}

#[test]
fn adts_aac_imports_and_matches_ffmpeg() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let path = adts_stereo(&ff).expect("ffmpeg must build the ADTS fixture");
    let src = filmcraft_codecs::open_bytes("tone.aac", bytes(&path)).expect("ADTS .aac must import (#1291)");

    let info = src.info();
    let a = info.audio().expect("an audio stream");
    assert_eq!((a.channels, a.sample_rate), (2, RATE), "{a:?}");

    let want = ffmpeg_audio_f32(&ff, &path, &[]);
    let n = want.len() / 2 - 2048; // skip the priming, compare the steady state
    let ours = src.audio(0, n, RATE).unwrap().channels;
    assert_eq!(ours.len(), 2);
    for c in 0..2 {
        let theirs: Vec<f32> = want.iter().skip(c).step_by(2).copied().collect();
        let d = diff(&ours[c], &theirs);
        assert!(d < 0.05, "channel {c}: max sample difference {d} against ffmpeg");
    }
}

/// A truncated ADTS stream must not import as a short success: `split_adts` stops at the
/// damaged frame, and importing what parsed would silently drop the rest of the file (#2598).
#[test]
fn a_truncated_adts_stream_is_rejected() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let path = adts_stereo(&ff).expect("ffmpeg must build the ADTS fixture");
    let stream = std::fs::read(&path).expect("fixture bytes");
    // The whole fixture imports.
    let full = filmcraft_codecs::AudioFileSource::decode("a.aac", stream.clone().into());
    assert!(full.is_ok(), "the complete stream imports: {:?}", full.err());
    // Cut mid-frame: the framing stops at the damaged frame, so the import must fail
    // rather than report the surviving prefix as the whole file.
    let cut = &stream[..stream.len() - stream.len() / 3];
    let trunc = filmcraft_codecs::AudioFileSource::decode("a.aac", cut.to_vec().into());
    assert!(trunc.is_err(), "a truncated stream is rejected, not imported short");
}
