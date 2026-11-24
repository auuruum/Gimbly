//! Linux setup runs off the UI and capture threads. Only an explicit button
//! click can ask polkit to load the module or add a dedicated loopback node.

use super::FRIENDLY_NAME;
use super::linux::{NodeError, loopback};
use eframe::egui::{self, Context};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

const DOCS: &str = "https://github.com/v4l2loopback/v4l2loopback#load-the-module";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Load,
    Add,
}

impl Method {
    fn tool(self) -> &'static str {
        match self {
            Self::Load => "modprobe",
            Self::Add => "v4l2loopback-ctl",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Load => &[
                "v4l2loopback",
                "devices=1",
                "exclusive_caps=1",
                concat!("card_label=", "Gimbly Virtual Camera"),
            ],
            Self::Add => &["add", "-x", "1", "-n", FRIENDLY_NAME],
        }
    }

    fn terminal_command(self) -> &'static str {
        match self {
            Self::Load => {
                "sudo modprobe v4l2loopback devices=1 exclusive_caps=1 'card_label=Gimbly Virtual Camera'"
            }
            Self::Add => "sudo v4l2loopback-ctl add -x 1 -n 'Gimbly Virtual Camera'",
        }
    }
}

struct Problem {
    message: String,
    method: Option<Method>,
    can_create: bool,
}

impl Problem {
    fn blocked(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            method: None,
            can_create: false,
        }
    }
}

type Check = Result<(), Problem>;

pub(crate) fn system_tool(name: &str) -> Option<PathBuf> {
    // Never use a program found in an arbitrary PATH for privileged setup.
    [
        "/usr/bin",
        "/usr/sbin",
        "/bin",
        "/sbin",
        "/run/wrappers/bin",
        "/run/current-system/sw/bin",
    ]
    .iter()
    .map(|dir| Path::new(dir).join(name))
    .find(|path| path.is_file())
}

fn inspect() -> Check {
    match loopback() {
        Ok(_) => return Ok(()),
        Err(NodeError::Unavailable(error)) => return Err(Problem::blocked(error)),
        Err(NodeError::Missing) => {}
    }
    let loaded = Path::new("/sys/module/v4l2loopback").is_dir();
    let method = if loaded { Method::Add } else { Method::Load };
    let installed = loaded
        || system_tool("modinfo").is_some_and(|tool| {
            Command::new(tool)
                .args(["-n", "v4l2loopback"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        });
    if !installed {
        return Err(Problem::blocked(
            "Install v4l2loopback for your running Linux kernel, then select Check again. If you just updated the kernel, reboot first.",
        ));
    }
    let helper = system_tool(method.tool()).is_some();
    let polkit = system_tool("pkexec").is_some();
    let message = match (method, helper, polkit) {
        (Method::Add, false, _) => {
            "The driver is loaded. Install your distribution's v4l2loopback utilities to add a dedicated Gimbly camera."
        }
        (_, _, false) | (Method::Load, false, _) => {
            "Create the Gimbly camera from a terminal using the command below, then select Check again."
        }
        (Method::Load, true, true) => {
            "The Linux driver is installed. Gimbly can load it and create the camera after you approve the system prompt."
        }
        (Method::Add, true, true) => {
            "Gimbly can add its own camera to the loaded Linux driver after you approve the system prompt."
        }
    };
    Err(Problem {
        message: message.into(),
        method: Some(method),
        can_create: helper && polkit,
    })
}

fn create(method: Method) -> Check {
    let mut problem = match inspect() {
        Ok(()) => return Ok(()),
        Err(problem) if problem.method == Some(method) && problem.can_create => problem,
        Err(problem) => return Err(problem),
    };
    let (Some(pkexec), Some(tool)) = (system_tool("pkexec"), system_tool(method.tool())) else {
        return inspect();
    };
    let output = Command::new(pkexec)
        .arg("--disable-internal-agent")
        .arg(tool)
        .args(method.args())
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            // udev may still be creating the video node and applying its ACL.
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match loopback() {
                    Ok(_) => return Ok(()),
                    Err(error) if Instant::now() >= deadline => {
                        problem.message = format!("The setup command completed. {error}");
                        return Err(problem);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        }
        Ok(output) if output.status.code() == Some(126) => {
            problem.message = "Setup was cancelled. You can try again when ready.".into();
        }
        Ok(output) if output.status.code() == Some(127) => {
            problem.message = "Administrator approval was unavailable. Use your desktop's authentication agent or copy the terminal command.".into();
        }
        Ok(output) => {
            let detail = String::from_utf8_lossy(&output.stderr);
            let detail: String = detail
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(220)
                .collect();
            problem.message = format!("Could not create the virtual camera. {detail}");
        }
        Err(error) => {
            problem.message = format!("Could not open the system authentication prompt. {error}")
        }
    }
    Err(problem)
}

#[derive(Default)]
pub struct Setup {
    pending: Option<Receiver<Check>>,
    problem: Option<Problem>,
}

pub enum Outcome {
    Ready,
    NeedsSetup,
}

impl Setup {
    pub fn check(&mut self, ctx: &Context) {
        self.start(ctx, inspect);
    }

    fn start(&mut self, ctx: &Context, work: impl FnOnce() -> Check + Send + 'static) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("virtual-camera-setup".into())
            .spawn(move || {
                let _ = tx.send(work());
                ctx.request_repaint();
            }) {
            Ok(_) => self.pending = Some(rx),
            Err(error) => {
                self.problem = Some(Problem::blocked(format!("Could not start setup. {error}")))
            }
        }
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn poll(&mut self) -> Option<Outcome> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(Problem::blocked("Setup stopped unexpectedly.")),
        };
        self.pending = None;
        self.problem = result.err();
        Some(if self.problem.is_none() {
            Outcome::Ready
        } else {
            Outcome::NeedsSetup
        })
    }

    pub fn show(&mut self, ctx: &Context) {
        let Some(problem) = &self.problem else { return };
        let mut install = None;
        let mut check = false;
        let mut close = false;
        let busy = self.pending();
        let modal = egui::Modal::new(egui::Id::new("linux-virtual-camera")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.heading("Set up Virtual Camera");
            ui.add_space(6.0);
            ui.label(&problem.message);
            ui.add_space(6.0);
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Waiting for Linux setup…");
                });
            }
            if let Some(method) = problem.method {
                if problem.can_create
                    && ui
                        .add_enabled(!busy, egui::Button::new("Create virtual camera…"))
                        .clicked()
                {
                    install = Some(method);
                }
                if ui.small_button("Copy terminal command").clicked() {
                    ctx.copy_text(method.terminal_command().into());
                }
            }
            ui.hyperlink_to("Linux driver setup", DOCS);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                check = ui
                    .add_enabled(!busy, egui::Button::new("Check again"))
                    .clicked();
                close = ui.add_enabled(!busy, egui::Button::new("Close")).clicked();
            });
        });
        if let Some(method) = install {
            self.start(ctx, move || create(method));
        } else if check {
            self.check(ctx);
        } else if close || (!busy && modal.should_close()) {
            self.problem = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_targets_one_named_device_and_does_not_replace_other_devices() {
        assert_eq!(
            Method::Load.args(),
            [
                "v4l2loopback",
                "devices=1",
                "exclusive_caps=1",
                "card_label=Gimbly Virtual Camera"
            ]
        );
        assert_eq!(Method::Add.args(), ["add", "-x", "1", "-n", FRIENDLY_NAME]);
    }
}
