//! `gimbly --probe`: a hardware smoke test without the UI. It opens the
//! first OBSBOT camera, reports its format and zoom range, steps the zoom and
//! measures the capture frame rate. Used to record hardware observations.
//!
//! Options: `--formats` lists the camera's native formats and exits;
//! `--mode WxH@FPS` (e.g. `3840x2160@30`) captures that mode instead of
//! stepping the zoom; `--portrait` adds the 9:16 crop.

use crate::camera::{self, CaptureConfig, Command, GimbalCommand, SessionState};
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

/// `--gimbal DIR PITCH:YAW:MS ...`: turns the gimbal through the same
/// commands the joystick sends and saves a preview frame to DIR before the
/// first turn and after each one.
fn gimbal(session: &camera::Session, args: &[String]) -> i32 {
    let Some((dir, turns)) = args.split_first() else {
        eprintln!("usage: --gimbal DIR PITCH:YAW:MS ...");
        return 2;
    };
    let save = |step: usize| {
        let saved = next_frame(session)
            .ok_or("no frame".to_string())
            .and_then(|f| crate::snapshot::save(&f, Path::new(dir), [0, 0, 0, 0, 0, step as u16]));
        let status = session.status();
        println!(
            "step {step}: {saved:?}, vendor control {}, error {:?}",
            status.vendor_control, status.error
        );
    };
    std::thread::sleep(Duration::from_secs(2));
    save(0);
    for (step, turn) in turns.iter().enumerate() {
        let parts: Vec<f32> = turn.split(':').filter_map(|p| p.parse().ok()).collect();
        let [pitch, yaw, ms] = parts[..] else {
            eprintln!("bad turn {turn:?}");
            return 2;
        };
        session.send(Command::Gimbal(GimbalCommand::Drag(true)));
        let until = Instant::now() + Duration::from_secs_f32(ms / 1000.0);
        while Instant::now() < until {
            session.send(Command::Gimbal(GimbalCommand::Speed { pitch, yaw }));
            std::thread::sleep(crate::joystick::REPEAT_EVERY);
        }
        session.send(Command::Gimbal(GimbalCommand::Speed {
            pitch: 0.0,
            yaw: 0.0,
        }));
        session.send(Command::Gimbal(GimbalCommand::Drag(false)));
        std::thread::sleep(Duration::from_millis(1500));
        save(step + 1);
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
    let Some(device) = devices.into_iter().find(|d| d.is_obsbot()) else {
        eprintln!("no OBSBOT camera found");
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
    let session = camera::Session::start(device, config, false, || {});
    if let Some(i) = args.iter().position(|a| a == "--gimbal") {
        return gimbal(&session, args.get(i + 1..).unwrap_or_default());
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
                    println!("no zoom control reported");
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
    0
}
