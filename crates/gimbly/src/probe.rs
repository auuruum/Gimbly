//! `gimbly --probe`: a hardware smoke test without the UI. It opens the
//! first OBSBOT camera, or the first supported input if none is attached.
//! It reports its format and zoom range, steps the zoom and measures the
//! capture frame rate. Used to record hardware observations.
//!
//! Options: `--formats` lists the camera's native formats and exits;
//! `--mode WxH@FPS` (e.g. `3840x2160@30`) captures that mode instead of
//! stepping the zoom; `--portrait` adds the 9:16 crop. Use
//! `--virtual-camera` before `--gimbal` to exercise virtual output too.

use crate::camera::{
    self, AutoZoom, CaptureConfig, Command, Fov, GimbalCommand, SessionState, TrackingMode,
    TrackingSpeed,
};
use std::path::Path;
use std::time::{Duration, Instant};

fn parse_mode(text: &str) -> Option<(u32, u32, u32)> {
    let (size, fps) = text.split_once('@')?;
    let (w, h) = size.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?, fps.parse().ok()?))
}

/// Waits for the next preview frame, up to two seconds.
fn next_frame(session: &camera::Session) -> Option<camera::PreviewFrame> {
    let _ = session.take_frame();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        if let Some(frame) = session.take_frame() {
            return Some(frame);
        }
    }
    None
}

fn control_step(step: &str) -> Option<Command> {
    let parts: Vec<_> = step.splitn(3, ':').collect();
    let command = match parts.as_slice() {
        ["sleep"] => return Some(Command::SetSleep(true)),
        ["wake"] => return Some(Command::SetSleep(false)),
        ["reset"] => GimbalCommand::Reset,
        ["wide"] => GimbalCommand::SetFov(Fov::Wide),
        ["medium"] => GimbalCommand::SetFov(Fov::Medium),
        ["narrow"] => GimbalCommand::SetFov(Fov::Narrow),
        ["track"] => GimbalCommand::SetHumanTracking(Some(TrackingMode::Single)),
        ["group"] => GimbalCommand::SetHumanTracking(Some(TrackingMode::Group)),
        ["untrack"] => GimbalCommand::SetHumanTracking(None),
        ["autozoom", zoom] => {
            GimbalCommand::SetAutoZoom(*AutoZoom::ALL.get(zoom.parse::<usize>().ok()?)?)
        }
        ["slow"] => GimbalCommand::SetTrackingSpeed(TrackingSpeed::Slow),
        ["standard"] => GimbalCommand::SetTrackingSpeed(TrackingSpeed::Standard),
        ["fast"] => GimbalCommand::SetTrackingSpeed(TrackingSpeed::Fast),
        ["zoom", ratio] => GimbalCommand::ZoomRatio(ratio.parse().ok()?),
        ["go", id] => GimbalCommand::TriggerPreset(id.parse().ok()?),
        ["add"] => GimbalCommand::AddPreset,
        ["update", id] => GimbalCommand::UpdatePreset {
            id: id.parse().ok()?,
        },
        ["delete", id] => GimbalCommand::DeletePreset(id.parse().ok()?),
        ["rename", id, name] => GimbalCommand::RenamePreset {
            id: id.parse().ok()?,
            name: (*name).to_owned(),
        },
        _ => return None,
    };
    Some(Command::Gimbal(command))
}

/// `--gimbal DIR STEP ...` sends the same commands as the panel and saves
/// a frame before and after each step. `PITCH:YAW:MS` moves the joystick.
fn gimbal(session: &camera::Session, args: &[String], virtual_camera: bool) -> i32 {
    let Some((dir, turns)) = args.split_first() else {
        eprintln!("usage: --gimbal DIR STEP ...");
        return 2;
    };
    let save = |step: usize| {
        let status = session.status();
        let frame = if status.sleeping {
            session.send(Command::Snapshot);
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut snapshot = session.take_snapshot();
            while snapshot.is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
                snapshot = session.take_snapshot();
            }
            snapshot
        } else {
            next_frame(session)
        };
        let saved = frame
            .ok_or("no frame".to_string())
            .and_then(|f| crate::snapshot::save(&f, Path::new(dir), [0, 0, 0, 0, 0, step as u16]));
        let status = session.status();
        println!(
            "step {step}: {saved:?}, vendor control {}, sleeping {}, tracking {:?} {:?}, auto zoom {:?}, speed {:?}, zoom {:?}, FOV {:?}, presets {:?}, error {:?}",
            status.vendor_control,
            status.sleeping,
            status.human_tracking,
            status.tracking_mode,
            status.auto_zoom,
            status.tracking_speed,
            status.zoom_ratio,
            status.fov,
            status.presets,
            status.error
        );
        saved.is_ok()
            && status.error.is_none()
            && status.vendor_control
            && (!virtual_camera || status.virtual_camera)
    };
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let status = session.status();
        if status.state == SessionState::Streaming
            && status.vendor_control
            && (!virtual_camera || status.virtual_camera)
        {
            break;
        }
        if status.state == SessionState::Failed || Instant::now() >= deadline {
            eprintln!(
                "Camera did not become ready for gimbal testing: {:?}",
                status.error
            );
            return 1;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !save(0) {
        return 1;
    }
    for (step, turn) in turns.iter().enumerate() {
        println!("{turn}");
        if let Some(command) = control_step(turn) {
            session.send(command);
        } else {
            let parts: Vec<_> = turn.split(':').collect();
            let [pitch, yaw, ms] = parts.as_slice() else {
                eprintln!("bad step {turn:?}");
                return 2;
            };
            let (Ok(pitch), Ok(yaw), Ok(ms)) =
                (pitch.parse::<f32>(), yaw.parse::<f32>(), ms.parse::<u64>())
            else {
                eprintln!("bad turn {turn:?}");
                return 2;
            };
            if !pitch.is_finite() || !yaw.is_finite() || ms > 60_000 {
                eprintln!("bad turn {turn:?}");
                return 2;
            }
            session.send(Command::Gimbal(GimbalCommand::Drag(true)));
            let until = Instant::now() + Duration::from_millis(ms);
            while Instant::now() < until {
                session.send(Command::Gimbal(GimbalCommand::Speed { pitch, yaw }));
                std::thread::sleep(crate::joystick::REPEAT_EVERY);
            }
            session.send(Command::Gimbal(GimbalCommand::Speed {
                pitch: 0.0,
                yaw: 0.0,
            }));
            session.send(Command::Gimbal(GimbalCommand::Drag(false)));
        }
        std::thread::sleep(Duration::from_millis(3500));
        if !save(step + 1) {
            return 1;
        }
    }
    0
}

pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let mode = args
        .iter()
        .position(|a| a == "--mode")
        .and_then(|i| args.get(i + 1))
        .map(|m| parse_mode(m).ok_or(m.clone()));
    let mode = match mode {
        Some(Err(m)) => {
            eprintln!("bad --mode {m:?}, expected e.g. 1920x1080@60");
            return 2;
        }
        Some(Ok(m)) => Some(m),
        None => None,
    };
    let mut config = CaptureConfig {
        portrait: args.iter().any(|a| a == "--portrait"),
        ..CaptureConfig::default()
    };
    if let Some((width, height, fps)) = mode {
        (config.width, config.height, config.fps) = (width, height, fps);
    }
    let devices = match camera::list_devices() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("enumeration failed: {e}");
            return 1;
        }
    };
    for d in &devices {
        println!(
            "device: {} vid={:04x?} pid={:04x?} obsbot={}",
            d.name,
            d.usb_vendor_id,
            d.usb_product_id,
            d.is_obsbot()
        );
    }
    let Some(device) = devices
        .iter()
        .find(|d| d.is_obsbot())
        .or_else(|| devices.first())
        .cloned()
    else {
        eprintln!("no supported capture device found");
        return 1;
    };
    if args.iter().any(|a| a == "--controls") {
        let script = args
            .iter()
            .skip_while(|a| *a != "--controls")
            .skip(1)
            .cloned()
            .collect();
        return match camera::probe_controls(&device, script) {
            Ok(lines) => {
                lines.iter().for_each(|l| println!("{l}"));
                0
            }
            Err(e) => {
                eprintln!("control query failed: {e}");
                1
            }
        };
    }
    if args.iter().any(|a| a == "--formats") {
        return match camera::list_modes(&device) {
            Ok(modes) => {
                for m in modes {
                    println!(
                        "{} {}x{} @ {:.2} fps (range {:.2}-{:.2}){}",
                        m.codec,
                        m.width,
                        m.height,
                        m.fps,
                        m.min_fps,
                        m.max_fps,
                        if m.supported { "" } else { " [not supported]" }
                    );
                }
                0
            }
            Err(e) => {
                eprintln!("format query failed: {e}");
                1
            }
        };
    }
    println!("requested: {config:?}");
    let virtual_camera = args.iter().any(|a| a == "--virtual-camera");
    if virtual_camera && let Err(error) = crate::vcam::register() {
        eprintln!("virtual camera unavailable: {error}");
        return 1;
    }
    let session = camera::Session::start(device, config, virtual_camera, || {});
    if let Some(i) = args.iter().position(|a| a == "--gimbal") {
        return gimbal(
            &session,
            args.get(i + 1..).unwrap_or_default(),
            virtual_camera,
        );
    }
    let deadline = Instant::now() + Duration::from_secs(if mode.is_some() { 8 } else { 15 });
    let mut frames = 0u32;
    // With an explicit mode we only measure; the zoom plan stays empty.
    let mut plan: Option<Vec<i32>> = mode.map(|_| Vec::new());
    let mut last_step = Instant::now();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        if let Some(f) = session.take_frame() {
            if frames == 0 {
                println!("first preview frame: {}x{}", f.size[0], f.size[1]);
            }
            frames += 1;
        }
        let status = session.status();
        if status.state == SessionState::Failed {
            eprintln!("session failed: {:?}", status.error);
            return 1;
        }
        if plan.is_none() && frames > 0 {
            println!("format: {:?}", status.format);
            plan = Some(match status.zoom_range {
                Some(r) => {
                    println!("zoom range: {r:?}, current: {:?}", status.zoom);
                    vec![r.min, r.snap((r.min + r.max) / 2), r.max, r.default]
                }
                None => {
                    if status.vendor_control {
                        println!(
                            "vendor zoom: {:?}; use --gimbal DIR zoom:RATIO to exercise it",
                            status.zoom_ratio
                        );
                    } else {
                        println!("no zoom control reported");
                    }
                    Vec::new()
                }
            });
            last_step = Instant::now();
        }
        if let Some(steps) = plan.as_mut().filter(|p| !p.is_empty())
            && last_step.elapsed() > Duration::from_millis(2500)
        {
            let target = steps.remove(0);
            session.send(Command::SetZoom(target));
            let sent = Instant::now();
            // Report how the camera's own reading converges on the target.
            let mut trace = Vec::new();
            while sent.elapsed() < Duration::from_millis(2000) {
                std::thread::sleep(Duration::from_millis(250));
                trace.push(session.status().zoom.unwrap_or(-1));
                let _ = session.take_frame();
            }
            println!("zoom -> {target}: readings every 250 ms {trace:?}");
            last_step = Instant::now();
        }
    }
    let status = session.status();
    println!(
        "format: {:?}, output: {:?}, preview frames taken: {frames}, measured capture fps: {:.1}",
        status.format, status.output, status.measured_fps
    );
    if frames == 0 || (virtual_camera && !status.virtual_camera) {
        eprintln!(
            "no usable output: {:?}",
            status.virtual_camera_error.or(status.error)
        );
        return 1;
    }
    0
}
