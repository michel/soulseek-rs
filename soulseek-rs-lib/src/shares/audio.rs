//! The audio attributes a share advertises for a file, read from the file's
//! own headers: Soulseek codes 0 bitrate (kbps), 1 duration (seconds), 2 VBR,
//! 4 sample rate (Hz) and 5 bit depth. Searchers filter and sort on these, so
//! a file without them ranks last in every other client.

use std::path::Path;

use std::io::{Read, Seek, SeekFrom};

const BITRATE: u32 = 0;
const DURATION: u32 = 1;
const VBR: u32 = 2;
const SAMPLE_RATE: u32 = 4;
const BIT_DEPTH: u32 = 5;

/// Enough of a file's head for an MP3 frame header plus its Xing block, or a
/// FLAC STREAMINFO.
const HEAD: usize = 8 * 1024;

/// The attributes for `path`, or none when the format is not one we read or
/// the file does not parse. Only the file's head is read.
#[must_use]
pub fn probe(path: &Path) -> Vec<(u32, u32)> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some("mp3") => mp3(path),
        Some("flac") => flac(path),
        _ => None,
    }
    .unwrap_or_default()
}

fn flac(path: &Path) -> Option<Vec<(u32, u32)>> {
    let mut head = [0u8; 42];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[0..4] != b"fLaC" || head[4] & 0x7F != 0 {
        return None;
    }
    let packed = u64::from_be_bytes(head[18..26].try_into().ok()?);
    let sample_rate = (packed >> 44) as u32;
    let bits = ((packed >> 36) & 0x1F) as u32 + 1;
    let samples = packed & 0xF_FFFF_FFFF;
    if sample_rate == 0 {
        return None;
    }
    let mut attributes = Vec::new();
    if samples > 0 {
        attributes.push((DURATION, (samples / u64::from(sample_rate)) as u32));
    }
    attributes.push((SAMPLE_RATE, sample_rate));
    attributes.push((BIT_DEPTH, bits));
    Some(attributes)
}

const MPEG1_BITRATES: [u32; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
const MPEG2_BITRATES: [u32; 15] =
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
const SAMPLE_RATES: [[u32; 3]; 3] = [
    [11_025, 12_000, 8_000],
    [22_050, 24_000, 16_000],
    [44_100, 48_000, 32_000],
];

/// One Layer III frame header, decoded.
struct Frame {
    bitrate_kbps: u32,
    sample_rate: u32,
    samples: u32,
    /// Offset of the Xing/Info block relative to the header.
    xing_at: usize,
}

fn frame(h: &[u8]) -> Option<Frame> {
    if h.len() < 4 || h[0] != 0xFF || h[1] & 0xE0 != 0xE0 {
        return None;
    }
    let version = (h[1] >> 3) & 3; // 0 = 2.5, 2 = 2, 3 = 1
    let layer = (h[1] >> 1) & 3; // 1 = Layer III
    let bitrate_index = usize::from(h[2] >> 4);
    let rate_index = usize::from((h[2] >> 2) & 3);
    if version == 1
        || layer != 1
        || bitrate_index == 0
        || bitrate_index == 15
        || rate_index == 3
    {
        return None;
    }
    let mpeg1 = version == 3;
    let mono = (h[3] >> 6) & 3 == 3;
    let rates = SAMPLE_RATES[if version == 0 {
        0
    } else if mpeg1 {
        2
    } else {
        1
    }];
    Some(Frame {
        bitrate_kbps: if mpeg1 {
            MPEG1_BITRATES
        } else {
            MPEG2_BITRATES
        }[bitrate_index],
        sample_rate: rates[rate_index],
        samples: if mpeg1 { 1152 } else { 576 },
        xing_at: 4 + match (mpeg1, mono) {
            (true, false) => 32,
            (true, true) | (false, false) => 17,
            (false, true) => 9,
        },
    })
}

fn mp3(path: &Path) -> Option<Vec<(u32, u32)>> {
    let mut file = std::fs::File::open(path).ok()?;
    let total = file.metadata().ok()?.len();
    let mut head = vec![0u8; HEAD];
    let read = file.read(&mut head).ok()?;
    head.truncate(read);
    // An ID3v2 tag in front: its size is four syncsafe bytes, plus a footer.
    let mut audio_at = 0usize;
    if head.len() >= 10 && &head[0..3] == b"ID3" {
        let size = head[6..10]
            .iter()
            .fold(0usize, |acc, &b| (acc << 7) | usize::from(b & 0x7F));
        audio_at = 10 + size + if head[5] & 0x10 != 0 { 10 } else { 0 };
        if audio_at + 4 > head.len() {
            file.seek(SeekFrom::Start(audio_at as u64)).ok()?;
            head = vec![0u8; HEAD];
            let read = file.read(&mut head).ok()?;
            head.truncate(read);
            audio_at = 0;
        }
    }
    let start = (audio_at..head.len().saturating_sub(4))
        .find(|&i| frame(&head[i..]).is_some())?;
    let frame = frame(&head[start..])?;
    let audio_bytes = total.saturating_sub((start + audio_at) as u64);

    let xing = start + frame.xing_at;
    let tag = head.get(xing..xing + 4);
    let (duration, bitrate, vbr) = if tag == Some(b"Xing")
        || tag == Some(b"Info")
    {
        let flags =
            u32::from_be_bytes(head.get(xing + 4..xing + 8)?.try_into().ok()?);
        let mut at = xing + 8;
        let mut frames = None;
        if flags & 1 != 0 {
            frames = Some(u32::from_be_bytes(
                head.get(at..at + 4)?.try_into().ok()?,
            ));
            at += 4;
        }
        let bytes = if flags & 2 != 0 {
            u64::from(u32::from_be_bytes(
                head.get(at..at + 4)?.try_into().ok()?,
            ))
        } else {
            audio_bytes
        };
        let seconds = f64::from(frames?) * f64::from(frame.samples)
            / f64::from(frame.sample_rate);
        let kbps = if seconds > 0.0 {
            (bytes as f64 * 8.0 / seconds / 1000.0) as u32
        } else {
            frame.bitrate_kbps
        };
        (seconds as u32, kbps, u32::from(tag == Some(b"Xing")))
    } else {
        let seconds = audio_bytes * 8 / (u64::from(frame.bitrate_kbps) * 1000);
        (seconds as u32, frame.bitrate_kbps, 0)
    };
    Some(vec![(BITRATE, bitrate), (DURATION, duration), (VBR, vbr)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One MPEG-1 Layer III frame header: 128 kbps, 44.1 kHz, stereo.
    const CBR_128: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];
    const CBR_128_FRAME_LEN: usize = 417;

    fn cbr_mp3(frames: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..frames {
            out.extend_from_slice(&CBR_128);
            out.resize(out.len() + CBR_128_FRAME_LEN - 4, 0);
        }
        out
    }

    fn write(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("soulseek-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_constant_bitrate_mp3_reports_bitrate_duration_and_no_vbr() {
        // 100 frames of 1152 samples at 44.1 kHz is 2.61 s.
        let path = write("cbr.mp3", &cbr_mp3(100));
        assert_eq!(probe(&path), [(0, 128), (1, 2), (2, 0)]);
    }

    #[test]
    fn an_id3v2_tag_in_front_of_the_frames_is_skipped() {
        let mut bytes = b"ID3\x04\x00\x00\x00\x00\x02\x00".to_vec();
        bytes.resize(bytes.len() + 256, 0); // 0x0200 syncsafe = 256 bytes
        bytes.extend_from_slice(&cbr_mp3(100));
        let path = write("tagged.mp3", &bytes);
        assert_eq!(probe(&path), [(0, 128), (1, 2), (2, 0)]);
    }

    #[test]
    fn a_variable_bitrate_mp3_uses_its_xing_header() {
        // A Xing header in the first frame's data: 200 frames, 104_250 bytes
        // in the stream, so 5.22 s at an average of 160 kbps.
        let mut bytes = cbr_mp3(1);
        let xing_at = 4 + 32; // MPEG-1 stereo side info
        bytes[xing_at..xing_at + 4].copy_from_slice(b"Xing");
        bytes[xing_at + 4..xing_at + 8].copy_from_slice(&3u32.to_be_bytes());
        bytes[xing_at + 8..xing_at + 12].copy_from_slice(&200u32.to_be_bytes());
        bytes[xing_at + 12..xing_at + 16]
            .copy_from_slice(&104_250u32.to_be_bytes());
        let path = write("vbr.mp3", &bytes);
        assert_eq!(probe(&path), [(0, 159), (1, 5), (2, 1)]);
    }

    #[test]
    fn a_flac_reports_duration_sample_rate_and_bit_depth() {
        // STREAMINFO: 44.1 kHz, 2 channels, 16 bits, 441_000 samples (10 s).
        let mut bytes = b"fLaC".to_vec();
        bytes.extend_from_slice(&[0x80, 0x00, 0x00, 0x22]);
        bytes.extend_from_slice(&[0x10, 0x00, 0x10, 0x00]); // block sizes
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // frame sizes
        let packed: u64 =
            (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36) | 0x0006_BAA8;
        bytes.extend_from_slice(&packed.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 16]); // md5
        let path = write("lossless.flac", &bytes);
        assert_eq!(probe(&path), [(1, 10), (4, 44_100), (5, 16)]);
    }

    #[test]
    fn anything_else_or_a_truncated_file_has_no_attributes() {
        assert!(probe(&write("notes.txt", b"hello")).is_empty());
        assert!(probe(&write("short.mp3", b"ID3")).is_empty());
        assert!(probe(&write("short.flac", b"fLaC\x80\x00")).is_empty());
        assert!(probe(Path::new("/nonexistent/x.mp3")).is_empty());
    }
}
