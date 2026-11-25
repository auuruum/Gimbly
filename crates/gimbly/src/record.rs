//! High-fps recording: the camera's JPEG frames copied into a video file
//! without re-encoding (see `docs/clean-room/2026-11-25-high-fps-recording.md`).
//!
//! The capture thread hands each frame to [`Recorder::push`], which only
//! copies it into a bounded queue; a writer thread does the file I/O, so a
//! slow disk drops frames instead of stalling the video.

pub mod mkv;
pub mod mp4;

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Playback rate of Slow Motion files.
pub const SLOW_MOTION_FPS: u32 = 30;
/// Frames larger than this are not JPEG frames from a webcam.
const MAX_FRAME: usize = 16 << 20;
/// Frames waiting for the writer; about half a second at 120 fps.
const QUEUE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Mkv,
    Mp4,
    Mov,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Container::Mkv => "mkv",
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
        }
    }
}

/// What the user asked to record; the capture size and rate come from the
/// running camera mode.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub dir: PathBuf,
    /// Local time used in the file name (see [`crate::snapshot::file_name`]).
    pub time: [u16; 6],
    pub container: Container,
    pub slow_motion: bool,
}

/// Real time recorded, without the paused spans; it starts with the first
/// frame.
#[derive(Debug, Default)]
struct Clock {
    start: Option<Instant>,
    paused_at: Option<Instant>,
    paused: Duration,
}

impl Clock {
    /// Real time recorded so far.
    fn recorded(&self, now: Instant) -> Duration {
        let Some(start) = self.start else {
            return Duration::ZERO;
        };
        let now = self.paused_at.unwrap_or(now);
        now.saturating_duration_since(start)
            .saturating_sub(self.paused)
    }

    /// Whether a frame arriving `now` is recorded (not while paused).
    fn tick(&mut self, now: Instant) -> bool {
        if self.paused_at.is_some() {
            return false;
        }
        self.start.get_or_insert(now);
        true
    }

    fn set_paused(&mut self, paused: bool, now: Instant) {
        match (paused, self.paused_at) {
            (true, None) => self.paused_at = Some(now),
            (false, Some(at)) => {
                self.paused_at = None;
                // Time paused before the first frame never started counting.
                if self.start.is_some() {
                    self.paused += now.saturating_duration_since(at);
                }
            }
            _ => {}
        }
    }
}

enum Muxer {
    Mkv(mkv::Writer<BufWriter<File>>),
    Mp4(mp4::Writer<BufWriter<File>>),
}

impl Muxer {
    fn frame(&mut self, jpeg: &[u8], at: Duration) -> io::Result<()> {
        match self {
            Muxer::Mkv(w) => w.frame(jpeg, millis(at)),
            Muxer::Mp4(w) => w.frame(jpeg, ticks(at)),
        }
    }

    fn finish(self, end: Duration) -> io::Result<()> {
        match self {
            Muxer::Mkv(w) => w.finish(millis(end)).map(drop),
            Muxer::Mp4(w) => w.finish(ticks(end)).map(drop),
        }
    }
}

fn millis(at: Duration) -> u64 {
    ((at.as_nanos() + 500_000) / 1_000_000) as u64
}

fn ticks(at: Duration) -> u64 {
    ((at.as_nanos() * u128::from(mp4::TIMESCALE) + 500_000_000) / 1_000_000_000) as u64
}

pub struct Recorder {
    path: PathBuf,
    queue: Option<SyncSender<(Vec<u8>, Duration)>>,
    writer: Option<JoinHandle<io::Result<()>>>,
    clock: Clock,
    /// Playback time of one frame.
    frame: Duration,
    frames: u32,
}

impl Recorder {
    /// Creates the file and starts the writer for frames of `width` x
    /// `height` captured at `fps`.
    ///
    /// Frames are stamped at a constant rate: `fps`, or
    /// [`SLOW_MOTION_FPS`] for Slow Motion, as the official app does.
    // shortcut: a camera that delivers fewer frames than its mode promises
    // plays back too fast; stamp arrival times if that shows up.
    pub fn start(spec: &Spec, width: u32, height: u32, fps: u32) -> Result<Self, String> {
        let playback_fps = if spec.slow_motion {
            SLOW_MOTION_FPS
        } else {
            fps.max(1)
        };
        let frame = Duration::from_secs(1) / playback_fps;
        let (path, file) = create(&spec.dir, spec.time, spec.container.extension())?;
        let out = BufWriter::with_capacity(1 << 20, file);
        let muxer = match spec.container {
            Container::Mkv => {
                mkv::Writer::new(out, width, height, frame.as_nanos() as u64).map(Muxer::Mkv)
            }
            Container::Mp4 => mp4::Writer::new(out, mp4::Brand::Mp4, width, height).map(Muxer::Mp4),
            Container::Mov => {
                mp4::Writer::new(out, mp4::Brand::QuickTime, width, height).map(Muxer::Mp4)
            }
        }
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        let (queue, frames) = mpsc::sync_channel::<(Vec<u8>, Duration)>(QUEUE);
        let writer = std::thread::Builder::new()
            .name("recording-writer".into())
            .spawn(move || {
                let mut muxer = muxer;
                let mut last = None;
                for (jpeg, at) in frames {
                    muxer.frame(&jpeg, at)?;
                    last = Some(at);
                }
                muxer.finish(last.map_or(Duration::ZERO, |at| at + frame))
            })
            .map_err(|e| format!("Could not start the recording: {e}"))?;
        Ok(Self {
            path,
            queue: Some(queue),
            writer: Some(writer),
            clock: Clock::default(),
            frame,
            frames: 0,
        })
    }

    /// Queues one camera frame that arrived `now`. Data that is not a JPEG
    /// frame is skipped. Fails once the writer has stopped.
    pub fn push(&mut self, jpeg: &[u8], now: Instant) -> Result<(), String> {
        if !(4..=MAX_FRAME).contains(&jpeg.len()) || jpeg[..2] != [0xFF, 0xD8] {
            return Ok(());
        }
        if !self.clock.tick(now) {
            return Ok(());
        }
        let at = self.frame * self.frames;
        self.frames = self.frames.saturating_add(1);
        let Some(queue) = &self.queue else {
            return Err("The recording has stopped".into());
        };
        match queue.try_send((jpeg.to_vec(), at)) {
            // A full queue means the disk is behind: drop this frame.
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(match self.close() {
                Err(e) => e,
                Ok(()) => "The recording stopped unexpectedly".into(),
            }),
        }
    }

    pub fn set_paused(&mut self, paused: bool, now: Instant) {
        self.clock.set_paused(paused, now);
    }

    pub fn paused(&self) -> bool {
        self.clock.paused_at.is_some()
    }

    /// Real time recorded so far, without pauses.
    pub fn recorded(&self, now: Instant) -> Duration {
        self.clock.recorded(now)
    }

    /// Writes the rest of the queue and the file index.
    fn close(&mut self) -> Result<(), String> {
        self.queue = None;
        match self.writer.take().map(JoinHandle::join) {
            None | Some(Ok(Ok(()))) => Ok(()),
            Some(Ok(Err(e))) => Err(format!("Could not write {}: {e}", self.path.display())),
            Some(Err(_)) => Err(format!("Writing {} failed", self.path.display())),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // shortcut: an error while finishing on drop is lost; the session
        // that owned the recorder is already gone and cannot report it.
        let _ = self.close();
    }
}

/// Creates a new file in `dir` without overwriting an existing one.
fn create(dir: &Path, time: [u16; 6], extension: &str) -> Result<(PathBuf, File), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    (0..100)
        .find_map(|attempt| {
            let path = dir.join(crate::snapshot::file_name(time, attempt, extension));
            File::create_new(&path).ok().map(|f| (path, f))
        })
        .ok_or_else(|| format!("Could not create a recording file in {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn recorded_time_skips_pauses() {
        let t0 = Instant::now();
        let mut clock = Clock::default();
        assert_eq!(clock.recorded(t0), Duration::ZERO);
        // A pause before the first frame does not count.
        clock.set_paused(true, t0);
        assert!(!clock.tick(t0 + MS * 5));
        clock.set_paused(false, t0 + MS * 50);
        assert!(clock.tick(t0 + MS * 100));
        assert_eq!(clock.recorded(t0 + MS * 108), MS * 8);
        clock.set_paused(true, t0 + MS * 200);
        assert!(!clock.tick(t0 + MS * 300));
        assert_eq!(clock.recorded(t0 + MS * 900), MS * 100);
        clock.set_paused(false, t0 + MS * 1200);
        assert_eq!(clock.recorded(t0 + MS * 1300), MS * 200);
    }

    #[test]
    fn frames_are_stamped_at_the_capture_or_slow_motion_rate() {
        assert_eq!(millis(Duration::from_secs(1) / 120 * 2), 17);
        assert_eq!(ticks(Duration::from_secs(1) / 120), 750);
        let dir = std::env::temp_dir().join(format!("gimbly-rate-{}", std::process::id()));
        let mut spec = Spec {
            dir: dir.clone(),
            time: [2026, 11, 25, 18, 45, 1],
            container: Container::Mp4,
            slow_motion: false,
        };
        let normal = Recorder::start(&spec, 1280, 720, 120).unwrap();
        assert_eq!(normal.frame, Duration::from_secs(1) / 120);
        spec.slow_motion = true;
        let mut slow = Recorder::start(&spec, 1280, 720, 120).unwrap();
        assert_eq!(slow.frame, Duration::from_secs(1) / SLOW_MOTION_FPS);
        let t0 = Instant::now();
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        slow.push(&jpeg, t0).unwrap();
        slow.set_paused(true, t0 + MS * 8);
        slow.push(&jpeg, t0 + MS * 16).unwrap();
        slow.set_paused(false, t0 + MS * 500);
        slow.push(&jpeg, t0 + MS * 508).unwrap();
        assert_eq!(slow.frames, 2, "the paused frame is not recorded");
        drop((normal, slow));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_jpeg_frames_and_skips_other_data() {
        let dir = std::env::temp_dir().join(format!("gimbly-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let spec = Spec {
            dir: dir.clone(),
            time: [2026, 11, 25, 18, 44, 34],
            container: Container::Mkv,
            slow_motion: false,
        };
        let mut recorder = Recorder::start(&spec, 1920, 1080, 120).unwrap();
        let second = Recorder::start(&spec, 1920, 1080, 120).unwrap();
        assert_ne!(recorder.path, second.path);
        assert_eq!(
            recorder.path.file_name().unwrap(),
            "Gimbly_20261125_184434.mkv"
        );
        let t0 = Instant::now();
        recorder.push(&[0xFF, 0xD8, 0xFF, 0xD9], t0).unwrap();
        recorder.push(&[0, 0, 0, 0], t0 + MS * 8).unwrap();
        recorder.push(&[0xFF, 0xD8], t0 + MS * 8).unwrap();
        recorder
            .push(&[0xFF, 0xD8, 0xAA, 0xD9], t0 + MS * 16)
            .unwrap();
        recorder.close().unwrap();
        let data = std::fs::read(&recorder.path).unwrap();
        let blocks = data.windows(4).filter(|w| w[..2] == [0xFF, 0xD8]).count();
        assert_eq!(blocks, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
