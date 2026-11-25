//! The camera microphone, recorded next to a High-fps video as a WAV file
//! (16-bit PCM) with the same name. The official app does the same; see
//! `docs/clean-room/2026-11-25-high-fps-recording.md`.
//!
//! Each OS module captures from the microphone whose name matches the
//! camera model and hands 16-bit interleaved samples to a sink, on a thread
//! of its own so the video never waits for audio.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod backend {
    pub(super) fn capture(
        _model: &str,
        _stop: &std::sync::atomic::AtomicBool,
        _sink: &mut super::Sink<'_>,
    ) -> Result<(), String> {
        Err("Audio capture is not supported on this system".into())
    }
}

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u16,
}

/// Takes captured samples (interleaved, 16-bit) in their format.
pub type Sink<'a> = dyn FnMut(Format, &[i16]) -> io::Result<()> + 'a;

/// Whether an audio input named `input` belongs to the OBSBOT camera
/// `model` (for example "Microphone (OBSBOT Tiny 3 Lite Audio)").
// shortcut: two cameras of the same model share a name, so the first
// matching microphone is used; match USB locations if that comes up.
pub fn matches(input: &str, model: &str) -> bool {
    let input = input.to_lowercase();
    input.contains("obsbot") && input.contains(&model.to_lowercase())
}

/// A WAV file of 16-bit PCM; the sizes are patched in at the end.
pub struct WavWriter<W: Write + Seek> {
    out: W,
    format: Format,
    data: u32,
}

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut out: W, format: Format) -> io::Result<Self> {
        let block = format.channels * 2;
        let mut head = Vec::with_capacity(44);
        head.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        head.extend_from_slice(&16u32.to_le_bytes());
        head.extend_from_slice(&1u16.to_le_bytes()); // PCM
        head.extend_from_slice(&format.channels.to_le_bytes());
        head.extend_from_slice(&format.rate.to_le_bytes());
        head.extend_from_slice(&(format.rate * u32::from(block)).to_le_bytes());
        head.extend_from_slice(&block.to_le_bytes());
        head.extend_from_slice(&16u16.to_le_bytes());
        head.extend_from_slice(b"data\0\0\0\0");
        out.write_all(&head)?;
        Ok(Self {
            out,
            format,
            data: 0,
        })
    }

    /// Appends samples in the file's format. A WAV file ends at 4 GiB;
    /// later samples are dropped.
    pub fn write(&mut self, format: Format, samples: &[i16]) -> io::Result<()> {
        if format != self.format {
            return Err(io::Error::other("the microphone changed its format"));
        }
        let room = (u32::MAX - 36 - self.data) as usize / 2;
        let mut bytes = Vec::with_capacity(samples.len().min(room) * 2);
        for sample in &samples[..samples.len().min(room)] {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.out.write_all(&bytes)?;
        self.data += bytes.len() as u32;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        let end = self.out.stream_position()?;
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(36 + self.data).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(40))?;
        self.out.write_all(&self.data.to_le_bytes())?;
        self.out.seek(SeekFrom::Start(end))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// Records the camera microphone until dropped.
pub struct Recording {
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl Recording {
    /// Opens the microphone of the camera `model` for recording into
    /// `path`. It starts paused, so that the sound can begin with the first
    /// video frame; the file is created with the first samples kept.
    pub fn start(model: &'static str, path: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(true));
        let (flag, pause) = (stop.clone(), paused.clone());
        let thread = std::thread::Builder::new()
            .name("recording-audio".into())
            .spawn(move || {
                let mut wav = None;
                let captured = backend::capture(model, &flag, &mut |format, samples| {
                    if pause.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let writer = match &mut wav {
                        Some(writer) => writer,
                        None => {
                            let file = File::create_new(&path)?;
                            wav.insert(WavWriter::new(BufWriter::new(file), format)?)
                        }
                    };
                    writer.write(format, samples)
                });
                let finished = wav.map(WavWriter::finish).transpose();
                captured?;
                finished
                    .map(drop)
                    .map_err(|e| format!("Could not write {}: {e}", path.display()))
            })
            .ok();
        Self {
            stop,
            paused,
            thread,
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Why the audio stopped early, reported once.
    pub fn take_error(&mut self) -> Option<String> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        match self.thread.take()?.join() {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(_) => Some("The audio recording failed".into()),
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn matches_only_the_camera_microphone() {
        assert!(matches(
            "Microphone (OBSBOT Tiny 3 Lite Audio)",
            "Tiny 3 Lite"
        ));
        assert!(matches("OBSBOT Tiny 3 Lite", "Tiny 3 Lite"));
        assert!(!matches("Microphone (RODE NT-USB)", "Tiny 3 Lite"));
        assert!(!matches("Microphone (OBSBOT Meet 2)", "Tiny 3 Lite"));
    }

    #[test]
    fn writes_a_pcm_wav_header_with_final_sizes() {
        let format = Format {
            rate: 48_000,
            channels: 2,
        };
        let mut wav = WavWriter::new(Cursor::new(Vec::new()), format).unwrap();
        wav.write(format, &[1, -1, 256, -256]).unwrap();
        let other = Format {
            rate: 44_100,
            channels: 2,
        };
        assert!(wav.write(other, &[0, 0]).is_err());
        let data = wav.finish().unwrap().into_inner();
        assert_eq!(data.len(), 44 + 8);
        let u32_at = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        let u16_at = |at: usize| u16::from_le_bytes(data[at..at + 2].try_into().unwrap());
        assert_eq!(&data[..4], b"RIFF");
        assert_eq!(u32_at(4), 36 + 8);
        assert_eq!(&data[8..16], b"WAVEfmt ");
        assert_eq!((u16_at(20), u16_at(22)), (1, 2), "PCM, stereo");
        assert_eq!((u32_at(24), u32_at(28)), (48_000, 192_000));
        assert_eq!((u16_at(32), u16_at(34)), (4, 16));
        assert_eq!(&data[36..40], b"data");
        assert_eq!(u32_at(40), 8);
        assert_eq!(&data[44..], [1, 0, 255, 255, 0, 1, 0, 255]);
    }
}
