//! One NV12 output pipeline for preview, snapshots and the virtual camera.

use super::{CaptureConfig, Command, RecordingStatus, SessionState, Worker, preview, standby};
use crate::record::Recorder;
use gimbly_vcam_shm::Nv12View;
use preview::YuvRange;
use std::time::{Duration, Instant};

/// While recording, decode at most one JPEG frame per this interval for
/// the preview and virtual camera (about every second frame at 120 fps).
const PICTURE_INTERVAL: Duration = Duration::from_millis(15);

type Placeholder = ((usize, usize, YuvRange), Vec<u8>);

pub(super) struct Frames<'a> {
    worker: &'a Worker,
    config: CaptureConfig,
    standby_picture: bool,
    virtual_camera: Option<crate::vcam::Output>,
    snapshot: bool,
    turned: Vec<u8>,
    placeholder: Option<Placeholder>,
    fps_window: (Instant, u32),
    /// Width, height and rate of the camera's JPEG frames in a compressed
    /// session.
    compressed: Option<(u32, u32, u32)>,
    recorder: Option<Recorder>,
    last_picture: Option<Instant>,
}

impl<'a> Frames<'a> {
    pub(super) fn new(worker: &'a Worker, config: CaptureConfig, standby_picture: bool) -> Self {
        Self {
            worker,
            config,
            standby_picture,
            virtual_camera: None,
            snapshot: false,
            turned: Vec::new(),
            placeholder: None,
            fps_window: (Instant::now(), 0),
            compressed: None,
            recorder: None,
            last_picture: None,
        }
    }

    /// The backend delivers the camera's JPEG frames through
    /// [`Self::packet`], `width` x `height` at `fps`.
    pub(super) fn set_compressed(&mut self, width: u32, height: u32, fps: f32) {
        self.compressed = Some((width, height, fps.round().max(1.0) as u32));
    }

    fn start_recording(&mut self, spec: &crate::record::Spec) {
        let result = match self.compressed {
            Some((w, h, fps)) => Recorder::start(spec, w, h, fps),
            None => Err("This camera mode cannot be recorded".into()),
        };
        let mut status = self.worker.shared.status();
        match result {
            Ok(recorder) => {
                self.recorder = Some(recorder);
                status.recording = Some(RecordingStatus::default());
            }
            Err(error) => status.error = Some(error),
        }
        drop(status);
        (self.worker.repaint)();
    }

    /// Hands one camera JPEG frame to the recording, if one runs.
    pub(super) fn packet(&mut self, jpeg: &[u8]) {
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        let now = Instant::now();
        let pushed = recorder.push(jpeg, now);
        let current = RecordingStatus {
            elapsed: recorder.recorded(now),
            paused: recorder.paused(),
        };
        let mut status = self.worker.shared.status();
        if let Err(error) = pushed {
            self.recorder = None;
            status.recording = None;
            status.error = Some(error);
            drop(status);
            (self.worker.repaint)();
            return;
        }
        let tick = status.recording.as_ref().map(|r| r.elapsed.as_secs())
            != Some(current.elapsed.as_secs());
        status.recording = Some(current);
        drop(status);
        if tick {
            (self.worker.repaint)();
        }
    }

    /// Whether to turn this frame into a picture for the preview, a
    /// snapshot or the virtual camera. Compressed sessions decode at a
    /// reduced rate so the 120 fps recording keeps up.
    pub(super) fn picture_due(&mut self) -> bool {
        let wanted =
            self.virtual_camera.is_some() || self.snapshot || self.worker.shared.frame().is_none();
        if !wanted || self.compressed.is_none() {
            return wanted;
        }
        let now = Instant::now();
        if self
            .last_picture
            .is_some_and(|last| now.saturating_duration_since(last) < PICTURE_INTERVAL)
        {
            return false;
        }
        self.last_picture = Some(now);
        true
    }

    pub(super) fn commands(&mut self, mut control: impl FnMut(Command)) {
        for command in self.worker.commands.drain() {
            match command {
                Command::SetVirtualCamera(enabled) => {
                    if enabled == self.virtual_camera.is_some() {
                        continue;
                    }
                    let format = self.worker.shared.status().output;
                    let output = enabled
                        .then(|| crate::vcam::Output::open(self.config, format))
                        .transpose();
                    let error = match output {
                        Ok(output) => {
                            self.virtual_camera = output;
                            None
                        }
                        Err(error) => Some(format!("Virtual camera output failed: {error}")),
                    };
                    {
                        let mut status = self.worker.shared.status();
                        status.virtual_camera = false;
                        status.virtual_camera_error = error;
                    }
                    (self.worker.repaint)();
                }
                Command::Snapshot => self.snapshot = true,
                Command::Record(spec) => self.start_recording(&spec),
                Command::PauseRecording(paused) => {
                    if let Some(recorder) = &mut self.recorder {
                        recorder.set_paused(paused, Instant::now());
                        if let Some(status) = &mut self.worker.shared.status().recording {
                            status.paused = paused;
                        }
                        (self.worker.repaint)();
                    }
                }
                Command::SetTransform(transform) => self.config.transform = transform,
                command => control(command),
            }
        }
    }

    /// On systems where the camera stops delivering buffers in sleep, keep
    /// the branded output and snapshots available at the normal cadence.
    pub(super) fn idle(&mut self) {
        let (sleeping, size) = {
            let status = self.worker.shared.status();
            (status.sleeping, status.output)
        };
        if sleeping && let Some((w, h)) = size {
            self.publish_placeholder(w as usize, h as usize, YuvRange::Limited);
        }
    }

    fn publish_placeholder(&mut self, width: usize, height: usize, range: YuvRange) {
        if !self.snapshot && self.virtual_camera.is_none() {
            return;
        }
        if let Some(view) = placeholder(&mut self.placeholder, width, height, range) {
            if self.snapshot {
                *self.worker.shared.snapshot() = Some(preview::convert_full(&view, range));
                self.snapshot = false;
                (self.worker.repaint)();
            }
            let result = self
                .virtual_camera
                .as_mut()
                .map(|output| output.publish(&view, range));
            self.check_output(result);
        }
    }

    pub(super) fn received(&mut self) {
        self.fps_window.1 += 1;
        let elapsed = self.fps_window.0.elapsed().as_secs_f32();
        let mut status = self.worker.shared.status();
        if status.state == SessionState::Opening {
            status.state = SessionState::Streaming;
            (self.worker.repaint)();
        }
        if elapsed >= 1.0 {
            status.measured_fps = self.fps_window.1 as f32 / elapsed;
            self.fps_window = (Instant::now(), 0);
        }
    }

    pub(super) fn publish(&mut self, epoch: u64, full: &Nv12View<'_>, range: YuvRange) {
        let asleep = {
            let status = self.worker.shared.status();
            if status.video_epoch != epoch {
                return;
            }
            status.sleeping
        };
        let want_preview = !asleep && self.worker.shared.frame().is_none();
        if !want_preview && !self.snapshot && self.virtual_camera.is_none() {
            return;
        }
        let replace = standby::use_placeholder(asleep, self.standby_picture, full);
        let crop = self
            .config
            .portrait
            .then(|| full.crop_to_portrait())
            .flatten();
        let view = crop.as_ref().unwrap_or(full);
        let turned = (!self.config.transform.is_identity()).then(|| {
            gimbly_vcam_shm::pixel::transform_into(view, self.config.transform, &mut self.turned)
        });
        let view = turned.as_ref().unwrap_or(view);
        let sleep = replace
            .then(|| placeholder(&mut self.placeholder, view.width, view.height, range))
            .flatten();
        let view = sleep.as_ref().unwrap_or(view);
        // Transform and conversion may have overlapped a sleep transition.
        if self.worker.shared.status().video_epoch != epoch {
            return;
        }
        if self.snapshot {
            *self.worker.shared.snapshot() = Some(preview::convert_full(view, range));
            self.snapshot = false;
            (self.worker.repaint)();
        }
        if want_preview
            && self
                .worker
                .shared
                .publish_preview(epoch, preview::convert(view, range))
        {
            (self.worker.repaint)();
        }
        let result = self
            .virtual_camera
            .as_mut()
            .map(|output| output.publish(view, range));
        self.check_output(result);
    }

    fn check_output(&mut self, result: Option<Result<bool, String>>) {
        match result {
            Some(Ok(true)) => {
                let mut status = self.worker.shared.status();
                if !status.virtual_camera {
                    status.virtual_camera = true;
                    (self.worker.repaint)();
                }
            }
            Some(Err(error)) => {
                self.virtual_camera = None;
                let mut status = self.worker.shared.status();
                status.virtual_camera = false;
                status.virtual_camera_error = Some(error);
                (self.worker.repaint)();
            }
            _ => {}
        }
    }
}

fn placeholder(
    cache: &mut Option<Placeholder>,
    w: usize,
    h: usize,
    range: YuvRange,
) -> Option<Nv12View<'_>> {
    let size = (w, h, range);
    if cache.as_ref().is_none_or(|(built, _)| *built != size) {
        let icon = crate::branding::window_icon();
        let frame = preview::sleep_frame(w, h, &icon.rgba, icon.width as usize, range);
        *cache = Some((size, frame));
    }
    Nv12View::packed(&cache.as_ref()?.1, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn virtual_camera_status_requires_a_published_frame_and_clears_on_error() {
        let (_tx, commands) = super::super::queue::channel();
        let worker = Worker {
            shared: Arc::default(),
            commands,
            stop: Arc::new(AtomicBool::new(false)),
            repaint: Arc::new(|| {}),
        };
        let mut frames = Frames::new(&worker, CaptureConfig::default(), false);
        frames.check_output(Some(Ok(false)));
        assert!(!worker.shared.status().virtual_camera);
        frames.check_output(Some(Ok(true)));
        assert!(worker.shared.status().virtual_camera);
        frames.check_output(Some(Err("device closed".into())));
        let status = worker.shared.status();
        assert!(!status.virtual_camera);
        assert_eq!(
            status.virtual_camera_error.as_deref(),
            Some("device closed")
        );
    }
}
