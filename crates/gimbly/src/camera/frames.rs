//! One NV12 output pipeline for preview, snapshots and the virtual camera.

use super::{CaptureConfig, Command, SessionState, Worker, preview, standby};
use gimbly_vcam_shm::Nv12View;
use preview::YuvRange;
use std::time::Instant;

type Placeholder = ((usize, usize, YuvRange), Vec<u8>);

pub(super) struct Frames<'a> {
    worker: &'a Worker,
    config: CaptureConfig,
    tiny_3_lite: bool,
    virtual_camera: Option<crate::vcam::Output>,
    snapshot: bool,
    turned: Vec<u8>,
    placeholder: Option<Placeholder>,
    fps_window: (Instant, u32),
}

impl<'a> Frames<'a> {
    pub(super) fn new(worker: &'a Worker, config: CaptureConfig, tiny_3_lite: bool) -> Self {
        Self {
            worker,
            config,
            tiny_3_lite,
            virtual_camera: None,
            snapshot: false,
            turned: Vec::new(),
            placeholder: None,
            fps_window: (Instant::now(), 0),
        }
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
                Command::SetTransform(transform) => self.config.transform = transform,
                command => control(command),
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn needed(&self) -> bool {
        self.virtual_camera.is_some() || self.snapshot || self.worker.shared.frame().is_none()
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
        let replace = standby::use_placeholder(asleep, self.tiny_3_lite, full);
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
