//! Bounded command mailboxes. Replace pending slider and joystick updates
//! before they reach a worker, without moving them past discrete commands.

use super::{Command, GimbalCommand};
use std::collections::VecDeque;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CAPACITY: usize = 32;

#[derive(Default)]
struct Pending {
    commands: Mutex<VecDeque<Command>>,
}

pub(super) struct Sender {
    pending: Arc<Pending>,
    wake: mpsc::SyncSender<()>,
}

pub(super) struct Receiver {
    pending: Arc<Pending>,
    wake: mpsc::Receiver<()>,
}

pub(super) fn channel() -> (Sender, Receiver) {
    let pending = Arc::new(Pending::default());
    let (tx, rx) = mpsc::sync_channel(1);
    (
        Sender {
            pending: pending.clone(),
            wake: tx,
        },
        Receiver { pending, wake: rx },
    )
}

fn continuous(command: &Command) -> Option<u8> {
    match command {
        Command::Gimbal(GimbalCommand::Speed { .. }) => Some(0),
        Command::Gimbal(GimbalCommand::ZoomRatio(_)) => Some(1),
        Command::SetZoom(_) => Some(2),
        Command::SetTransform(_) => Some(3),
        Command::Gimbal(GimbalCommand::SetFocusPosition(_)) => Some(4),
        Command::Gimbal(GimbalCommand::SetShutter(_)) => Some(5),
        Command::Gimbal(GimbalCommand::SetIso(_)) => Some(6),
        Command::Gimbal(GimbalCommand::SetCompensation(_)) => Some(7),
        Command::Gimbal(GimbalCommand::SetIsoLimits(..)) => Some(8),
        Command::Gimbal(GimbalCommand::SetExposureCurve(_)) => Some(9),
        Command::Gimbal(GimbalCommand::SetWhiteBalance(_)) => Some(10),
        Command::Gimbal(GimbalCommand::SetImage(_)) => Some(11),
        _ => None,
    }
}

impl Sender {
    pub(super) fn send(&self, command: Command) -> bool {
        {
            let mut queue = self
                .pending
                .commands
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let older = continuous(&command).and_then(|kind| {
                queue
                    .iter_mut()
                    .rev()
                    .take_while(|c| continuous(c).is_some())
                    .find(|c| continuous(c) == Some(kind))
            });
            if let Some(older) = older {
                *older = command;
            } else {
                if queue.len() >= CAPACITY {
                    // Reserve one extra slot for releasing the joystick.
                    // The worker sends zero on release even if it was dropped.
                    if !matches!(command, Command::Gimbal(GimbalCommand::Drag(false))) {
                        return false;
                    }
                    if queue.len() > CAPACITY {
                        return true;
                    }
                }
                queue.push_back(command);
            }
        }
        !matches!(
            self.wake.try_send(()),
            Err(mpsc::TrySendError::Disconnected(()))
        )
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.pending
            .commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

impl Receiver {
    pub(super) fn drain(&self) -> VecDeque<Command> {
        let mut queue = self
            .pending
            .commands
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *queue)
    }

    pub(super) fn wait(&self, timeout: Duration) -> Result<VecDeque<Command>, RecvTimeoutError> {
        match self.wake.recv_timeout(timeout) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => Ok(self.drain()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_updates_stay_bounded_and_release_survives_saturation() {
        let (sender, receiver) = channel();
        for speed in 1..100 {
            assert!(sender.send(Command::Gimbal(GimbalCommand::Speed {
                pitch: 0.0,
                yaw: speed as f32,
            })));
        }
        assert_eq!(receiver.drain().len(), 1);
        for _ in 0..CAPACITY {
            assert!(sender.send(Command::Snapshot));
        }
        assert!(!sender.send(Command::Snapshot));
        assert!(sender.send(Command::Gimbal(GimbalCommand::Drag(false))));
        let pending = receiver.drain();
        assert_eq!(pending.len(), CAPACITY + 1);
        assert_eq!(
            pending.back(),
            Some(&Command::Gimbal(GimbalCommand::Drag(false)))
        );
    }

    #[test]
    fn focus_slider_keeps_only_the_newest_position_in_order() {
        let (sender, receiver) = channel();
        let focus = |p| Command::Gimbal(GimbalCommand::SetFocusPosition(p));
        sender.send(Command::Gimbal(GimbalCommand::SetAutoFocus(false)));
        for position in 0..=100 {
            assert!(sender.send(focus(position)));
        }
        sender.send(Command::Gimbal(GimbalCommand::SetAutoFocus(true)));
        sender.send(focus(7));
        let pending: Vec<_> = receiver.drain().into_iter().collect();
        assert_eq!(
            pending,
            [
                Command::Gimbal(GimbalCommand::SetAutoFocus(false)),
                focus(100),
                Command::Gimbal(GimbalCommand::SetAutoFocus(true)),
                focus(7),
            ]
        );
    }
}
