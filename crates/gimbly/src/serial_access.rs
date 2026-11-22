//! Linux: the camera's serial port carries the tracking box, and most
//! distributions let only a group such as uucp or dialout open it. This
//! dialog offers to install Gimbly's udev rule
//! (`packaging/linux/70-gimbly.rules`) with polkit, or copies the same
//! steps as a terminal command. Only a button click asks for approval, and
//! the work runs off the UI thread.

use crate::camera::SerialDenied;
use crate::vcam::setup::system_tool;
use eframe::egui::{self, Context};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};

const RULE: &str = include_str!("../../../packaging/linux/70-gimbly.rules");
const RULE_PATH: &str = "/etc/udev/rules.d/70-gimbly.rules";

/// The rule's lines without comments, for the terminal command.
fn rule_lines() -> impl Iterator<Item = &'static str> {
    RULE.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// One line to paste in a terminal: writes the rule and applies it.
fn terminal_command() -> String {
    let lines: Vec<String> = rule_lines().map(|line| format!("'{line}'")).collect();
    format!(
        "printf '%s\\n' {} | sudo tee {RULE_PATH} >/dev/null && sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=tty",
        lines.join(" ")
    )
}

/// The root script run through pkexec; the rule arrives on stdin, so no
/// user-controlled text is part of the command.
fn install_script(udevadm: &str) -> String {
    format!(
        "set -e\n\
         umask 022\n\
         mkdir -p /etc/udev/rules.d\n\
         cat > {RULE_PATH}.new\n\
         mv -f {RULE_PATH}.new {RULE_PATH}\n\
         {udevadm} control --reload\n\
         {udevadm} trigger --subsystem-match=tty\n"
    )
}

fn can_install() -> bool {
    ["pkexec", "sh", "udevadm"]
        .iter()
        .all(|tool| system_tool(tool).is_some())
}

fn install() -> Result<(), String> {
    let (Some(pkexec), Some(sh), Some(udevadm)) = (
        system_tool("pkexec"),
        system_tool("sh"),
        system_tool("udevadm"),
    ) else {
        return Err(
            "Administrator approval is unavailable. Copy the terminal command instead.".into(),
        );
    };
    let child = Command::new(pkexec)
        .arg("--disable-internal-agent")
        .arg(sh)
        .arg("-c")
        .arg(install_script(&udevadm.display().to_string()))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            return Err(format!(
                "Could not open the system authentication prompt. {error}"
            ));
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        // A cancelled prompt closes the pipe; the exit code says why.
        let _ = stdin.write_all(RULE.as_bytes());
    }
    match child.wait_with_output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) if output.status.code() == Some(126) => {
            Err("Cancelled. You can try again when ready.".into())
        }
        Ok(output) if output.status.code() == Some(127) => Err(
            "Administrator approval was unavailable. Use your desktop's authentication agent or copy the terminal command."
                .into(),
        ),
        Ok(output) => {
            let detail: String = String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(220)
                .collect();
            Err(format!("Could not install the udev rule. {detail}"))
        }
        Err(error) => Err(format!("Could not install the udev rule. {error}")),
    }
}

#[derive(Default)]
pub struct SerialAccess {
    /// The port the user chose "Not now" for in this run.
    dismissed: Option<String>,
    pending: Option<Receiver<Result<(), String>>>,
    message: Option<String>,
    installed: bool,
}

impl SerialAccess {
    /// Shows the dialog while `denied` is set. It closes by itself once the
    /// camera thread opens the port.
    pub fn show(&mut self, ctx: &Context, denied: Option<&SerialDenied>) {
        self.poll();
        let Some(denied) = denied else {
            self.message = None;
            self.installed = false;
            return;
        };
        if self.dismissed.as_deref() == Some(denied.path.as_str()) {
            return;
        }
        let busy = self.pending.is_some();
        let installable = can_install();
        let mut allow = false;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("linux-serial-access")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading("Show the tracking box");
            ui.add_space(6.0);
            let group = denied.group.as_deref().unwrap_or("such as uucp or dialout");
            ui.label(format!(
                "The camera sends the box around the tracked person or object on its serial \
                 port {}. Only the group {group} may open it, so the preview shows no box.",
                denied.path
            ));
            ui.add_space(4.0);
            ui.label(if installable {
                "Gimbly can install a udev rule that lets you open this camera's port, after you approve the system prompt."
            } else {
                "Run the terminal command below to install a udev rule that lets you open this camera's port."
            });
            ui.add_space(6.0);
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Waiting for approval…");
                });
            }
            if let Some(message) = &self.message {
                ui.label(message);
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                if installable && !self.installed {
                    allow = ui
                        .add_enabled(!busy, egui::Button::new("Allow access…"))
                        .clicked();
                }
                if ui.button("Copy terminal command").clicked() {
                    ctx.copy_text(terminal_command());
                    self.message = Some(
                        "Copied. Run it in a terminal; the box appears a few seconds later."
                            .into(),
                    );
                }
                let label = if self.installed { "Close" } else { "Not now" };
                close = ui.add_enabled(!busy, egui::Button::new(label)).clicked();
            });
        });
        if allow {
            self.start(ctx);
        } else if close || (!busy && modal.should_close()) {
            self.dismissed = Some(denied.path.clone());
        }
    }

    fn start(&mut self, ctx: &Context) {
        let (tx, rx) = mpsc::sync_channel(1);
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("serial-access-setup".into())
            .spawn(move || {
                let _ = tx.send(install());
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.pending = Some(rx);
                self.message = None;
            }
            Err(error) => self.message = Some(format!("Could not start setup. {error}")),
        }
    }

    fn poll(&mut self) {
        let Some(pending) = &self.pending else { return };
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Setup stopped unexpectedly.".into()),
        };
        self.pending = None;
        match result {
            Ok(()) => {
                self.installed = true;
                self.message = Some(
                    "Access granted. The box appears within a few seconds. If it does not, \
                     unplug the camera and plug it back in."
                        .into(),
                );
            }
            Err(message) => self.message = Some(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_targets_the_camera_serial_ports() {
        let lines: Vec<_> = rule_lines().collect();
        assert_eq!(lines.len(), 1);
        let rule = lines.first().copied().unwrap_or_default();
        assert!(rule.contains(r#"SUBSYSTEM=="tty""#));
        assert!(rule.contains(r#"ATTRS{idVendor}=="3564""#));
        assert!(rule.contains(r#"TAG+="uaccess""#));
    }

    #[test]
    fn terminal_command_quotes_the_rule_safely() {
        // The rule goes inside single quotes, so it must not contain one.
        assert!(rule_lines().all(|line| !line.contains('\'')));
        let command = terminal_command();
        assert!(command.starts_with("printf '%s\\n' 'SUBSYSTEM=="));
        assert!(command.contains(&format!("sudo tee {RULE_PATH} >/dev/null")));
        assert!(command.ends_with("sudo udevadm trigger --subsystem-match=tty"));
        assert!(!command.contains('#'));
        assert!(!command.contains('\n'));
    }

    #[test]
    fn install_script_writes_the_rule_atomically_and_applies_it() {
        let script = install_script("/usr/bin/udevadm");
        assert!(script.starts_with("set -e\n"));
        let write = script.find(&format!("cat > {RULE_PATH}.new"));
        let rename = script.find(&format!("mv -f {RULE_PATH}.new {RULE_PATH}"));
        let reload = script.find("/usr/bin/udevadm control --reload");
        let trigger = script.find("/usr/bin/udevadm trigger --subsystem-match=tty");
        assert!(write < rename && rename < reload && reload < trigger);
        assert!(write.is_some());
    }
}
