//! System-wide keyboard shortcuts.
//!
//! On Windows a dedicated thread registers the shortcuts with
//! `RegisterHotKey` and waits for `WM_HOTKEY` messages, so they work while
//! Gimbly is minimised or unfocused. Other platforms report that global
//! shortcuts are not available yet.

use crate::settings::{HotkeyAction, KeyCombo};
use std::sync::mpsc::{Receiver, channel};

pub struct GlobalHotkeys {
    events: Receiver<HotkeyAction>,
    #[cfg(windows)]
    thread: Option<(u32, std::thread::JoinHandle<()>)>,
}

impl GlobalHotkeys {
    /// Registers `bindings`. Returns the handle and a message for each
    /// shortcut that could not be registered (usually because another app
    /// already owns it).
    pub fn start(
        bindings: Vec<(HotkeyAction, KeyCombo)>,
        notify: impl Fn() + Send + 'static,
    ) -> (Self, Vec<String>) {
        imp::start(bindings, notify)
    }

    /// Actions triggered since the last call.
    pub fn poll(&self) -> Vec<HotkeyAction> {
        self.events.try_iter().collect()
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::settings::Key;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
        UnregisterHotKey,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_HOTKEY, WM_QUIT,
    };

    /// Windows virtual-key code of `key`.
    fn virtual_key(key: Key) -> u32 {
        match key {
            Key::Letter(c) => c as u32,
            Key::Digit(d) => 0x30 + d as u32,
            Key::F(n) => 0x70 + (n as u32 - 1),
            Key::Up => 0x26,
            Key::Down => 0x28,
            Key::Left => 0x25,
            Key::Right => 0x27,
            Key::Space => 0x20,
            Key::Home => 0x24,
            Key::End => 0x23,
            Key::PageUp => 0x21,
            Key::PageDown => 0x22,
            Key::Insert => 0x2D,
            Key::Delete => 0x2E,
            Key::Plus => 0xBB,
            Key::Minus => 0xBD,
        }
    }

    fn modifiers(c: &KeyCombo) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        for (on, flag) in [
            (c.ctrl, MOD_CONTROL),
            (c.alt, MOD_ALT),
            (c.shift, MOD_SHIFT),
            (c.win, MOD_WIN),
        ] {
            if on {
                m |= flag;
            }
        }
        m
    }

    pub fn start(
        bindings: Vec<(HotkeyAction, KeyCombo)>,
        notify: impl Fn() + Send + 'static,
    ) -> (GlobalHotkeys, Vec<String>) {
        let (event_tx, events) = channel();
        let (ready_tx, ready) = channel();
        let thread = std::thread::Builder::new()
            .name("global-hotkeys".into())
            .spawn(move || {
                // SAFETY: Win32 calls on this thread's own message queue; every
                // registered id is unregistered before the thread ends.
                unsafe {
                    let mut msg = MSG::default();
                    // Create the message queue before anyone posts to it.
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                    let mut registered = Vec::new();
                    let mut failures = Vec::new();
                    for (id, (action, combo)) in bindings.iter().enumerate() {
                        let id = id as i32 + 1;
                        match RegisterHotKey(None, id, modifiers(combo), virtual_key(combo.key)) {
                            Ok(()) => registered.push(id),
                            Err(_) => failures.push(format!(
                                "{} ({}) is already used by another app",
                                combo,
                                action.label().replace('\n', " / ")
                            )),
                        }
                    }
                    let _ = ready_tx.send((GetCurrentThreadId(), failures));
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        if msg.message == WM_HOTKEY {
                            let index = msg.wParam.0.wrapping_sub(1);
                            if let Some((action, _)) = bindings.get(index) {
                                let _ = event_tx.send(*action);
                                notify();
                            }
                        }
                    }
                    for id in registered {
                        let _ = UnregisterHotKey(None, id);
                    }
                }
            });
        let (thread, failures) = match thread.ok().zip(ready.recv().ok()) {
            Some((handle, (thread_id, failures))) => (Some((thread_id, handle)), failures),
            None => (
                None,
                vec!["Could not start the global hotkey thread".into()],
            ),
        };
        (GlobalHotkeys { events, thread }, failures)
    }

    impl Drop for GlobalHotkeys {
        fn drop(&mut self) {
            if let Some((thread_id, handle)) = self.thread.take() {
                // SAFETY: posting WM_QUIT to a thread we own ends its loop.
                let _ = unsafe {
                    PostThreadMessageW(thread_id, WM_QUIT, Default::default(), Default::default())
                };
                let _ = handle.join();
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn start(
        _bindings: Vec<(HotkeyAction, KeyCombo)>,
        _notify: impl Fn() + Send + 'static,
    ) -> (GlobalHotkeys, Vec<String>) {
        let (_tx, events) = channel();
        (
            GlobalHotkeys { events },
            vec!["Global hotkeys are only implemented on Windows so far".into()],
        )
    }
}
