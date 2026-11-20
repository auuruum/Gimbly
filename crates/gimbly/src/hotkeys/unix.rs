//! System-wide shortcuts use the compositor portal on Wayland and native
//! desktop registration on X11 and macOS.

use super::*;
use crate::settings::Key;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::collections::HashMap;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Mutex, Once};

pub(super) enum Platform {
    Native {
        _manager: GlobalHotKeyManager,
    },
    #[cfg(target_os = "linux")]
    Portal {
        _close: tokio::sync::oneshot::Sender<()>,
    },
}

struct Listener {
    actions: HashMap<u32, HotkeyAction>,
    events: SyncSender<HotkeyAction>,
    notify: Box<dyn Fn() + Send>,
}

static LISTENER: Mutex<Option<Listener>> = Mutex::new(None);
static INSTALL_HANDLER: Once = Once::new();

impl Drop for Platform {
    fn drop(&mut self) {
        if matches!(self, Self::Native { .. }) {
            *LISTENER.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
}

pub(super) fn start(
    bindings: Vec<(HotkeyAction, KeyCombo)>,
    #[cfg(target_os = "linux")] wayland: bool,
    notify: impl Fn() + Send + 'static,
) -> (GlobalHotkeys, Vec<String>) {
    let (tx, events) = sync_channel(32);
    let (error_tx, errors) = channel();
    let mut result = GlobalHotkeys {
        events,
        errors,
        _platform: None,
    };
    if bindings.is_empty() {
        return (result, Vec::new());
    }

    #[cfg(target_os = "linux")]
    if wayland {
        let (close, closed) = tokio::sync::oneshot::channel();
        match std::thread::Builder::new()
            .name("shortcut-portal".into())
            .spawn(move || {
                let run = || -> Result<(), String> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| e.to_string())?;
                    runtime.block_on(portal(bindings, tx, closed, &notify))
                };
                if let Err(error) = run() {
                    let message = format!(
                        "Wayland global shortcuts unavailable: {error}. See the Linux desktop setup in CONTRIBUTING.md."
                    );
                    eprintln!("{message}");
                    let _ = error_tx.send(message);
                    notify();
                }
            }) {
            Ok(_) => {
                result._platform = Some(Platform::Portal { _close: close });
                return (result, Vec::new());
            }
            Err(e) => {
                return (
                    result,
                    vec![format!("Could not start global shortcuts: {e}")],
                );
            }
        }
    }

    #[cfg(target_os = "macos")]
    drop(error_tx);
    INSTALL_HANDLER.call_once(|| {
        GlobalHotKeyEvent::set_event_handler(Some(|event: GlobalHotKeyEvent| {
            if event.state != HotKeyState::Pressed {
                return;
            }
            let listener = LISTENER.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(listener) = &*listener
                && let Some(action) = listener.actions.get(&event.id)
                && listener.events.try_send(*action).is_ok()
            {
                (listener.notify)();
            }
        }));
    });
    let manager = match GlobalHotKeyManager::new() {
        Ok(manager) => manager,
        Err(e) => {
            return (
                result,
                vec![format!("Could not start global shortcuts: {e}")],
            );
        }
    };
    let mut actions = HashMap::new();
    let mut failures = Vec::new();
    for (action, combo) in bindings {
        let key = match combo.key {
            Key::PageUp => "PageUp".into(),
            Key::PageDown => "PageDown".into(),
            Key::Insert => "Insert".into(),
            Key::Delete => "Delete".into(),
            key => key.to_string(),
        };
        let mut parts = Vec::new();
        for (enabled, name) in [
            (combo.ctrl, "Ctrl"),
            (combo.alt, "Alt"),
            (combo.shift, "Shift"),
            (combo.win, "Super"),
        ] {
            if enabled {
                parts.push(name);
            }
        }
        parts.push(&key);
        let hotkey = parts
            .join("+")
            .parse::<HotKey>()
            .map_err(|e| e.to_string())
            .and_then(|key| {
                manager
                    .register(key)
                    .map(|()| key)
                    .map_err(|e| e.to_string())
            });
        match hotkey {
            Ok(key) => {
                actions.insert(key.id(), action);
            }
            Err(e) => failures.push(format!(
                "{combo} ({}): {e}",
                action.label().replace('\n', " / ")
            )),
        }
    }
    *LISTENER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Listener {
        actions,
        events: tx,
        notify: Box::new(notify),
    });
    result._platform = Some(Platform::Native { _manager: manager });
    (result, failures)
}

#[cfg(target_os = "linux")]
async fn portal(
    bindings: Vec<(HotkeyAction, KeyCombo)>,
    events: SyncSender<HotkeyAction>,
    mut closed: tokio::sync::oneshot::Receiver<()>,
    notify: &dyn Fn(),
) -> Result<(), String> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;

    let connect = async {
        let connection = ashpd::zbus::Connection::session().await?;
        let app_id = crate::branding::APP_ID.parse::<ashpd::AppID>()?;
        // Launchers may already have associated this process with its app ID.
        // Older portals also lack the Registry interface.
        let registration =
            ashpd::register_host_app_with_connection(connection.clone(), app_id).await;
        let portal = GlobalShortcuts::with_connection(connection).await?;
        let session = portal
            .create_session(Default::default())
            .await
            .map_err(|error| registration.err().unwrap_or(error))?;
        Ok::<_, ashpd::Error>((portal, session))
    };
    let (portal, session) = tokio::select! {
        _ = &mut closed => return Ok(()),
        result = connect => result.map_err(|e| e.to_string())?,
    };
    let result = async {
        let mut actions = HashMap::new();
        let shortcuts: Vec<_> = bindings
            .into_iter()
            .map(|(action, combo)| {
                let id = format!("{action:?}");
                actions.insert(id.clone(), action);
                NewShortcut::new(id, action.label().replace('\n', " / "))
                    .preferred_trigger(portal_trigger(combo).as_str())
            })
            .collect();
        let signals = portal
            .receive_activated()
            .await
            .map_err(|e| e.to_string())?;
        let response = portal
            .bind_shortcuts(&session, &shortcuts, None, Default::default())
            .await
            .map_err(|e| e.to_string())?
            .response()
            .map_err(|e| e.to_string())?;
        if response.shortcuts().is_empty() {
            return Err("The desktop did not bind any shortcuts".into());
        }
        let mut signals = std::pin::pin!(signals);
        while let Some(signal) = signals.next().await {
            if let Some(action) = actions.get(signal.shortcut_id())
                && events.try_send(*action).is_ok()
            {
                notify();
            }
        }
        Err("The desktop shortcut session ended".into())
    };
    let result = tokio::select! {
        _ = &mut closed => Ok(()),
        result = result => result,
    };
    let _ = session.close().await;
    result
}

#[cfg(target_os = "linux")]
fn portal_trigger(combo: KeyCombo) -> String {
    let key = match combo.key {
        Key::Letter(c) => c.to_ascii_lowercase().to_string(),
        Key::Space => "space".into(),
        Key::Plus => "equal".into(),
        Key::Minus => "minus".into(),
        Key::PageUp => "Page_Up".into(),
        Key::PageDown => "Page_Down".into(),
        Key::Insert => "Insert".into(),
        Key::Delete => "Delete".into(),
        key => key.to_string(),
    };
    let mut parts = Vec::new();
    for (enabled, name) in [
        (combo.ctrl, "CTRL"),
        (combo.alt, "ALT"),
        (combo.shift, "SHIFT"),
        (combo.win, "LOGO"),
    ] {
        if enabled {
            parts.push(name);
        }
    }
    parts.push(&key);
    parts.join("+")
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn uses_xdg_keysyms_instead_of_display_labels() {
        assert_eq!(portal_trigger(KeyCombo::alt(Key::Letter('R'))), "ALT+r");
        assert_eq!(
            portal_trigger(KeyCombo::alt(Key::PageDown)),
            "ALT+Page_Down"
        );
        assert_eq!(portal_trigger(KeyCombo::alt(Key::Plus)), "ALT+equal");
    }
}
