//! The Software Settings window: Device Management, Output, Global Hotkeys,
//! OSC and General Settings, organised like OBSBOT Center's settings dialog.
//! Options Gimbly cannot honour yet are shown disabled and say so.

use crate::branding;
use crate::camera::{DeviceInfo, VideoMode};
use crate::os;
use crate::settings::*;
use crate::theme::*;
use crate::widgets::{help, place, toggle};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Event, Layout, Rect, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, ViewportCommand, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::hash::Hash;

const NOT_YET: &str = "Not implemented in Gimbly yet";
const WINDOW_SIZE: [f32; 2] = [936.0, 704.0];
const TITLE_HEIGHT: f32 = 30.0;
const NAV_WIDTH: f32 = 192.0;
const FIELD_WIDTH: f32 = 226.0;
const FIELD_HEIGHT: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    DeviceManagement,
    Output,
    GlobalHotkeys,
    Osc,
    General,
}

impl Page {
    const ALL: [Page; 5] = [
        Page::DeviceManagement,
        Page::Output,
        Page::GlobalHotkeys,
        Page::Osc,
        Page::General,
    ];

    /// Page named on the command line (`--settings=hotkeys`).
    pub fn from_arg(name: &str) -> Option<Page> {
        Some(match name {
            "device" | "devices" => Page::DeviceManagement,
            "output" => Page::Output,
            "hotkeys" => Page::GlobalHotkeys,
            "osc" => Page::Osc,
            "general" => Page::General,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Page::DeviceManagement => "Device Management",
            Page::Output => "Output",
            Page::GlobalHotkeys => "Global Hotkeys",
            Page::Osc => "OSC",
            Page::General => "General Settings",
        }
    }
}

/// What the user changed; the app reacts (restarts capture, re-registers
/// hotkeys, saves).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Resolution or preview mode: the capture must restart.
    Output,
    /// Hotkey bindings or the global on/off switch.
    Hotkeys,
    /// Anything that only needs saving.
    Saved,
}

/// What the pages need to know about the app.
pub struct View<'a> {
    pub devices: &'a [DeviceInfo],
    pub modes: &'a [VideoMode],
    pub capture_format: Option<&'a str>,
}

pub struct SettingsWindow {
    icon: egui::TextureHandle,
    pub open: bool,
    page: Page,
    /// Hotkey edits waiting for Apply.
    draft: Bindings,
    recording: Option<HotkeyAction>,
    groups_open: [bool; 2],
    starts_automatically: Option<bool>,
    notice: Option<String>,
}

impl SettingsWindow {
    pub fn new(settings: &Settings, icon: egui::TextureHandle) -> Self {
        Self {
            icon,
            open: false,
            page: Page::Output,
            draft: settings.hotkeys.clone(),
            recording: None,
            groups_open: [true, true],
            starts_automatically: None,
            notice: None,
        }
    }

    pub fn open_page(&mut self, page: Page) {
        self.open = true;
        self.page = page;
    }

    /// True while the Global Hotkeys page waits for a key press; global
    /// registrations must be paused so the keys reach the window.
    pub fn is_recording_hotkey(&self) -> bool {
        self.open && self.recording.is_some()
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        view: View<'_>,
    ) -> Vec<Change> {
        if !self.open {
            return Vec::new();
        }
        let mut changes = Vec::new();
        let mut close = false;
        let builder = egui::ViewportBuilder::default()
            .with_title("Software Settings")
            .with_app_id(crate::branding::APP_ID)
            .with_icon(branding::window_icon())
            .with_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_decorations(false);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("software-settings"),
            builder,
            |ui, _| {
                let full = ui.max_rect();
                ui.painter().rect_filled(full, CornerRadius::ZERO, CARD);
                close |= self.title_bar(ui, full);
                let body_top = full.top() + TITLE_HEIGHT;
                let nav = Rect::from_min_max(
                    pos2(full.left(), body_top),
                    pos2(full.left() + NAV_WIDTH, full.bottom()),
                );
                self.nav(ui, nav);
                let content = Rect::from_min_max(
                    pos2(nav.right() + 22.0, body_top + 14.0),
                    pos2(full.right() - 24.0, full.bottom() - 14.0),
                );
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(content)
                        .layout(Layout::top_down(Align::Min)),
                    |ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                        match self.page {
                            Page::DeviceManagement => device_page(ui, &view),
                            Page::Output => self.output_page(ui, settings, &view, &mut changes),
                            Page::GlobalHotkeys => self.hotkeys_page(ui, settings, &mut changes),
                            Page::Osc => osc_page(ui, settings),
                            Page::General => self.general_page(ui, settings, &mut changes),
                        }
                    },
                );
                ui.painter().rect_stroke(
                    full,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, WINDOW_EDGE),
                    StrokeKind::Inside,
                );
                close |= ui.ctx().input(|i| i.viewport().close_requested());
            },
        );
        if close {
            self.open = false;
            self.recording = None;
        }
        changes
    }

    /// Returns true when the close button was pressed.
    fn title_bar(&mut self, ui: &mut Ui, full: Rect) -> bool {
        let bar = Rect::from_min_size(full.min, vec2(full.width(), TITLE_HEIGHT));
        let drag = ui.interact(bar, ui.id().with("settings-title"), Sense::click_and_drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let painter = ui.painter();
        painter.rect_filled(bar, CornerRadius::ZERO, WINDOW);
        painter.image(
            self.icon.id(),
            Rect::from_center_size(pos2(bar.left() + 20.0, bar.center().y), Vec2::splat(20.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        painter.text(
            pos2(bar.left() + 34.0, bar.center().y),
            Align2::LEFT_CENTER,
            "Software Settings",
            font(15.0),
            TEXT,
        );
        let close = Rect::from_center_size(
            pos2(bar.right() - 20.0, bar.center().y),
            vec2(30.0, TITLE_HEIGHT),
        );
        let response = ui.interact(close, ui.id().with("settings-close"), Sense::click());
        if response.hovered() {
            ui.painter()
                .rect_filled(close, CornerRadius::ZERO, BUTTON_HOVER);
        }
        ui.painter().text(
            close.center(),
            Align2::CENTER_CENTER,
            icon::X,
            font(15.0),
            TEXT,
        );
        response.clicked()
    }

    fn nav(&mut self, ui: &mut Ui, nav: Rect) {
        ui.painter().rect_filled(nav, CornerRadius::ZERO, NAV);
        for (i, page) in Page::ALL.into_iter().enumerate() {
            let item = Rect::from_min_size(
                nav.min + vec2(6.0, 6.0 + i as f32 * 50.0),
                vec2(nav.width() - 12.0, 38.0),
            );
            let response = ui.interact(item, ui.id().with(("nav", i)), Sense::click());
            if self.page == page || response.hovered() {
                ui.painter()
                    .rect_filled(item, CornerRadius::same(6), NAV_SELECTED);
            }
            ui.painter().text(
                pos2(item.left() + 13.0, item.center().y),
                Align2::LEFT_CENTER,
                page.label(),
                font(15.0),
                TEXT,
            );
            if response.clicked() {
                self.page = page;
                self.recording = None;
            }
        }
    }

    // ---- Output -------------------------------------------------------------

    fn output_page(
        &mut self,
        ui: &mut Ui,
        settings: &mut Settings,
        view: &View<'_>,
        changes: &mut Vec<Change>,
    ) {
        // Offer only modes the camera reported; all of them while unknown.
        let available: Vec<Resolution> = Resolution::ALL
            .iter()
            .copied()
            .filter(|r| {
                let (w, h, fps) = r.mode();
                *r == settings.resolution
                    || view.modes.is_empty()
                    || view.modes.iter().any(|m| m.covers(w, h, fps))
            })
            .collect();
        row(
            ui,
            58.0,
            |ui| {
                ui.vertical(|ui| {
                    label(ui, "Resolution");
                    ui.add_space(6.0);
                    dim(ui, "Recording/Default Virtual Camera Resolution");
                });
            },
            |ui| {
                let labels: Vec<&str> = available.iter().map(|r| r.label()).collect();
                if let Some(i) = dropdown(
                    ui,
                    "resolution",
                    settings.resolution.label(),
                    &labels,
                    FIELD_WIDTH,
                ) && available[i] != settings.resolution
                {
                    settings.resolution = available[i];
                    changes.push(Change::Output);
                }
            },
        );
        if let Some(format) = view.capture_format {
            dim(ui, &format!("Camera format in use: {format}"));
            ui.add_space(4.0);
        }
        dim(
            ui,
            "Video apps pick up a new virtual camera resolution the next time they open the camera.",
        );
        ui.add_space(8.0);
        row(
            ui,
            48.0,
            |ui| label(ui, "Preview Mode"),
            |ui| {
                let labels: Vec<&str> = PreviewMode::ALL.iter().map(|m| m.label()).collect();
                if let Some(i) = dropdown(
                    ui,
                    "preview-mode",
                    settings.preview_mode.label(),
                    &labels,
                    FIELD_WIDTH,
                ) && PreviewMode::ALL[i] != settings.preview_mode
                {
                    settings.preview_mode = PreviewMode::ALL[i];
                    changes.push(Change::Output);
                }
            },
        );
        disabled(ui, "Recording is not implemented in Gimbly yet", |ui| {
            row(
                ui,
                48.0,
                |ui| label(ui, "Video Encoding"),
                |ui| {
                    dropdown(ui, "encoder", settings.encoder.label(), &[], FIELD_WIDTH);
                },
            );
        });
        rule(ui);
        ui.add_space(14.0);
        dim(ui, "Record");
        disabled(ui, "Recording is not implemented in Gimbly yet", |ui| {
            row(
                ui,
                46.0,
                |ui| {
                    label(ui, "Recording Bitrate");
                    help(ui, "Higher bitrates need a faster computer.");
                },
                |ui| {
                    dropdown(ui, "bitrate", settings.bitrate.label(), &[], FIELD_WIDTH);
                },
            );
            row(
                ui,
                44.0,
                |ui| label(ui, "Recording Format"),
                |ui| {
                    dropdown(
                        ui,
                        "record-format",
                        settings.record_format.label(),
                        &[],
                        FIELD_WIDTH,
                    );
                },
            );
        });
        rule(ui);
        ui.add_space(12.0);
        label(ui, "Media File Location");
        ui.add_space(10.0);
        let width = ui.available_width();
        ui.add_enabled_ui(os::SUPPORTED, |ui| {
            if path_field(ui, &settings.media_dir, width).clicked()
                && let Some(dir) = os::pick_folder(&settings.media_dir)
            {
                settings.media_dir = dir;
                changes.push(Change::Saved);
            }
        })
        .response
        .on_hover_text("Where snapshots and recordings will be saved");
        ui.add_space(24.0);
        disabled(
            ui,
            "AI Notes is an OBSBOT cloud service; Gimbly does not use it",
            |ui| {
                label(ui, "AI Notes File Cache Location");
                ui.add_space(8.0);
                dim(
                    ui,
                    "Stores recordings temporarily before upload; files will be deleted automatically after upload",
                );
                ui.add_space(10.0);
                path_field(ui, "", width);
            },
        );
    }

    // ---- Global Hotkeys -----------------------------------------------------

    fn hotkeys_page(&mut self, ui: &mut Ui, settings: &mut Settings, changes: &mut Vec<Change>) {
        self.capture_shortcut(ui);
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            label(ui, "Global Hotkeys");
            ui.add_space(14.0);
            if toggle(ui, settings.hotkeys_enabled)
                .on_hover_text("Shortcuts work even when Gimbly is in the background")
                .clicked()
            {
                settings.hotkeys_enabled = !settings.hotkeys_enabled;
                changes.push(Change::Hotkeys);
            }
        });
        if let Some(notice) = &self.notice {
            dim(ui, notice);
        }
        ui.add_space(10.0);
        let footer = 46.0;
        let list_height = ui.available_height() - footer;
        egui::ScrollArea::vertical()
            .max_height(list_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                for (g, group) in [HotkeyGroup::Universal, HotkeyGroup::TinySeries]
                    .into_iter()
                    .enumerate()
                {
                    self.hotkey_group(ui, g, group);
                }
            });
        // Footer: Revert / Clear / Apply.
        ui.add_space(10.0);
        let dirty = self.draft != settings.hotkeys;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if ui
                .add_enabled_ui(dirty, |ui| small_button(ui, "Apply", vec2(80.0, 26.0)))
                .inner
                .clicked()
            {
                settings.hotkeys = self.draft.clone();
                self.notice = None;
                changes.push(Change::Hotkeys);
            }
            if small_button(ui, "Clear", vec2(80.0, 26.0))
                .on_hover_text("Unbind every shortcut")
                .clicked()
            {
                self.draft.values_mut().for_each(|c| *c = None);
            }
            if ui
                .add_enabled_ui(dirty, |ui| small_button(ui, "Revert", vec2(80.0, 26.0)))
                .inner
                .clicked()
            {
                self.draft = settings.hotkeys.clone();
                self.notice = None;
            }
        });
    }

    fn hotkey_group(&mut self, ui: &mut Ui, index: usize, group: HotkeyGroup) {
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
        let caret = if self.groups_open[index] {
            icon::CARET_UP
        } else {
            icon::CARET_DOWN
        };
        ui.painter().text(
            pos2(rect.left() + 8.0, rect.center().y),
            Align2::CENTER_CENTER,
            caret,
            font(15.0),
            TEXT,
        );
        ui.painter().text(
            pos2(rect.left() + 28.0, rect.center().y),
            Align2::LEFT_CENTER,
            group.label(),
            font(15.0),
            TEXT,
        );
        let actions: Vec<HotkeyAction> = HotkeyAction::ALL
            .iter()
            .copied()
            .filter(|a| a.group() == group)
            .collect();
        let buttons = Rect::from_min_max(pos2(rect.right() - 80.0, rect.top()), rect.max);
        let mut header_clicked = false;
        place(
            ui,
            buttons,
            Layout::right_to_left(Align::Center),
            ("group-buttons", index),
            |ui| {
                if plain_icon(ui, icon::BASKET, "Clear this group").clicked() {
                    for a in &actions {
                        self.draft.insert(*a, None);
                    }
                    header_clicked = true;
                }
                if plain_icon(
                    ui,
                    icon::ARROWS_CLOCKWISE,
                    "Restore defaults for this group",
                )
                .clicked()
                {
                    for a in &actions {
                        self.draft.insert(*a, a.default_combo());
                    }
                    header_clicked = true;
                }
            },
        );
        if response.clicked() && !header_clicked {
            self.groups_open[index] = !self.groups_open[index];
        }
        if !self.groups_open[index] {
            return;
        }
        for action in actions {
            let combo = self.draft.get(&action).copied().flatten();
            ui.add_enabled_ui(action.implemented(), |ui| {
                let label_lines = action.label().lines().count() as f32;
                let height = if label_lines > 1.0 { 44.0 } else { 38.0 };
                row(
                    ui,
                    height,
                    |ui| {
                        ui.label(
                            egui::RichText::new(action.label())
                                .size(15.0)
                                .line_height(Some(18.0)),
                        );
                    },
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if square_button(ui, icon::X, "Clear").clicked() {
                            self.draft.insert(action, None);
                        }
                        if square_button(ui, icon::ARROWS_CLOCKWISE, "Restore default").clicked() {
                            self.draft.insert(action, action.default_combo());
                        }
                        let recording = self.recording == Some(action);
                        let text = if recording {
                            "Press a shortcut (Esc to cancel)".to_string()
                        } else {
                            combo.map(|c| c.to_string()).unwrap_or_default()
                        };
                        if key_field(ui, &text, recording).clicked() {
                            self.recording = Some(action);
                            self.notice = None;
                        }
                    },
                );
            })
            .response
            .on_disabled_hover_text(NOT_YET);
        }
    }

    /// While a key field is armed, turns the next key press into a binding.
    fn capture_shortcut(&mut self, ui: &mut Ui) {
        let Some(action) = self.recording else { return };
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, modifiers)) = pressed else {
            return;
        };
        if key == egui::Key::Escape {
            self.recording = None;
            return;
        }
        let Some(key) = map_key(key) else { return };
        let combo = KeyCombo {
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
            win: modifiers.mac_cmd,
            key,
        };
        if !combo.ctrl && !combo.alt && !combo.win {
            let notice = if cfg!(target_os = "macos") {
                "Global shortcuts need Control, Option or Command."
            } else {
                "Global shortcuts need Ctrl or Alt."
            };
            self.notice = Some(notice.into());
            return;
        }
        // One shortcut, one action: take it away from any other action.
        let taken: Vec<HotkeyAction> = self
            .draft
            .iter()
            .filter(|(a, c)| **a != action && **c == Some(combo))
            .map(|(a, _)| *a)
            .collect();
        for other in &taken {
            self.draft.insert(*other, None);
        }
        self.notice = taken.first().map(|other| {
            format!(
                "{combo} was used by \"{}\"; it now belongs to \"{}\".",
                other.label().replace('\n', " / "),
                action.label().replace('\n', " / ")
            )
        });
        self.draft.insert(action, Some(combo));
        self.recording = None;
    }

    // ---- General ------------------------------------------------------------

    fn general_page(&mut self, ui: &mut Ui, settings: &mut Settings, changes: &mut Vec<Change>) {
        ui.add_space(4.0);
        label(ui, "Software Version");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            dim(ui, concat!("v", env!("CARGO_PKG_VERSION")));
            if link(ui, "Vers. Info")
                .on_hover_text("Open the Gimbly releases page")
                .clicked()
            {
                os::open_url(concat!(env!("CARGO_PKG_REPOSITORY"), "/releases"));
            }
            link(ui, "Privacy Policy")
                .on_hover_text("Gimbly has no account, no telemetry and sends nothing anywhere.");
        });
        ui.add_space(12.0);
        disabled(ui, NOT_YET, |ui| {
            row(
                ui,
                46.0,
                |ui| label(ui, "Product Tutorial"),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    small_button(ui, "View Tutorial", vec2(110.0, 30.0));
                    small_button(ui, "Guide", vec2(110.0, 30.0));
                },
            );
        });
        row(
            ui,
            48.0,
            |ui| label(ui, "Language"),
            |ui| {
                dropdown(ui, "language", "English", &["English"], FIELD_WIDTH);
            },
        );
        rule(ui);
        row(
            ui,
            48.0,
            |ui| label(ui, "Global Hotkeys"),
            |ui| {
                if toggle(ui, settings.hotkeys_enabled).clicked() {
                    settings.hotkeys_enabled = !settings.hotkeys_enabled;
                    changes.push(Change::Hotkeys);
                }
            },
        );
        disabled(ui, NOT_YET, |ui| {
            row(
                ui,
                48.0,
                |ui| {
                    label(ui, "Remote Controller");
                    help(ui, "Support for the OBSBOT remote control");
                },
                |ui| {
                    toggle(ui, false);
                },
            );
            row(
                ui,
                48.0,
                |ui| {
                    label(ui, "Media Window Capture");
                    help(ui, "Lets screen-capture apps record Gimbly's windows");
                },
                |ui| {
                    toggle(ui, false);
                },
            );
        });
        row(
            ui,
            48.0,
            |ui| label(ui, "Close the Software"),
            |ui| {
                let labels: Vec<&str> = CloseAction::ALL.iter().map(|c| c.label()).collect();
                let current = CloseAction::ALL
                    .iter()
                    .position(|c| *c == settings.close_action)
                    .unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, current, 110.0) {
                    settings.close_action = CloseAction::ALL[i];
                    changes.push(Change::Saved);
                }
            },
        );
        let starts = *self
            .starts_automatically
            .get_or_insert_with(os::starts_automatically);
        ui.add_enabled_ui(os::SUPPORTED, |ui| {
            row(
                ui,
                48.0,
                |ui| label(ui, "Starts Automatically"),
                |ui| {
                    if toggle(ui, starts)
                        .on_hover_text("Start Gimbly when you sign in")
                        .clicked()
                    {
                        match os::set_starts_automatically(!starts) {
                            Ok(()) => self.starts_automatically = Some(!starts),
                            Err(e) => self.notice = Some(e),
                        }
                    }
                },
            );
        });
        disabled(ui, "Gimbly does not write log files yet", |ui| {
            row(
                ui,
                48.0,
                |ui| label(ui, "Software Log"),
                |ui| {
                    small_button(ui, "Open Folder", vec2(124.0, 30.0));
                },
            );
        });
        rule(ui);
        let (mode_row, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), 110.0), Sense::hover());
        ui.painter().text(
            pos2(mode_row.left(), mode_row.center().y),
            Align2::LEFT_CENTER,
            "Software Mode",
            font(15.0),
            TEXT,
        );
        let pro = Rect::from_min_size(
            pos2(mode_row.right() - 116.0, mode_row.top() + 14.0),
            vec2(102.0, 56.0),
        );
        let lite = pro.translate(vec2(-152.0, 0.0));
        mode_card(ui, pro, "Pro Mode", true, true);
        mode_card(ui, lite, "Lite Mode", false, false);
        ui.interact(lite, ui.id().with("lite-mode"), Sense::hover())
            .on_hover_text(format!("Lite Mode: {NOT_YET}"));
        if let Some(notice) = &self.notice {
            dim(ui, notice);
        }
    }
}

// ---- Device Management -------------------------------------------------------

fn device_page(ui: &mut Ui, view: &View<'_>) {
    row(
        ui,
        36.0,
        |ui| {
            label(ui, "Device List");
            help(
                ui,
                "Your cameras, numbered 1 to 4. Gimbly controls up to four devices.",
            );
        },
        |ui| {
            ui.add_enabled_ui(false, |ui| {
                small_button(ui, "Add Device", vec2(126.0, 26.0))
            })
            .response
            .on_disabled_hover_text("Network (NDI/RTSP) devices are not supported yet");
        },
    );
    ui.add_space(8.0);
    let area = ui.available_rect_before_wrap();
    let numbers = Rect::from_min_max(area.min, pos2(area.left() + 66.0, area.bottom()));
    let table = Rect::from_min_max(pos2(numbers.right(), area.top()), area.max);
    let painter = ui.painter();
    painter.rect_filled(table, CornerRadius::same(6), TABLE);

    let header = Rect::from_min_size(table.min, vec2(table.width(), 32.0));
    let columns = [
        ("Device", 14.0),
        ("Name", 102.0),
        ("Connection", 248.0),
        ("IP", 372.0),
    ];
    for (title, x) in columns {
        painter.text(
            pos2(header.left() + x, header.center().y),
            Align2::LEFT_CENTER,
            title,
            font(15.0),
            TEXT,
        );
    }
    painter.text(
        pos2(header.right() - 46.0, header.center().y),
        Align2::CENTER_CENTER,
        "More",
        font(15.0),
        TEXT,
    );
    painter.text(
        pos2(numbers.left() + 8.0, header.center().y),
        Align2::CENTER_CENTER,
        icon::LOCK_SIMPLE,
        font(14.0),
        TEXT,
    );
    painter.text(
        pos2(numbers.left() + 44.0, header.center().y),
        Align2::CENTER_CENTER,
        "No.",
        font(15.0),
        TEXT,
    );

    for slot in 0..4 {
        let row_rect = Rect::from_min_size(
            pos2(table.left(), header.bottom() + slot as f32 * 48.0),
            vec2(table.width(), 47.0),
        );
        painter.rect_filled(row_rect, CornerRadius::ZERO, TABLE_ROW);
        let y = row_rect.center().y;
        let device = view.devices.get(slot);
        let lock_color = if device.is_some() { TEXT } else { TEXT_FAINT };
        painter.text(
            pos2(numbers.left() + 8.0, y),
            Align2::CENTER_CENTER,
            icon::LOCK_SIMPLE_OPEN,
            font(13.0),
            lock_color,
        );
        let badge = Rect::from_center_size(pos2(numbers.left() + 44.0, y), vec2(20.0, 20.0));
        painter.rect_filled(badge, CornerRadius::same(4), ACCENT);
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            (slot + 1).to_string(),
            font(14.0),
            ON_ACCENT,
        );
        let Some(device) = device else { continue };
        let model = model_name(device);
        let chip_galley = painter.layout_no_wrap(model.clone(), font(12.0), LINK);
        let chip = Rect::from_min_size(
            pos2(row_rect.left() + 14.0, y - 13.0),
            vec2(chip_galley.size().x + 12.0, 26.0),
        );
        painter.rect_stroke(
            chip,
            CornerRadius::same(4),
            Stroke::new(1.0, LINK),
            StrokeKind::Inside,
        );
        painter.galley(
            pos2(chip.left() + 6.0, y - chip_galley.size().y / 2.0),
            chip_galley,
            LINK,
        );
        painter.text(
            pos2(row_rect.left() + 102.0, y),
            Align2::LEFT_CENTER,
            &model,
            font(15.0),
            TEXT,
        );
        painter.text(
            pos2(row_rect.left() + 248.0, y),
            Align2::LEFT_CENTER,
            icon::USB,
            font(15.0),
            TEXT,
        );
        painter.text(
            pos2(row_rect.left() + 272.0, y),
            Align2::LEFT_CENTER,
            "UVC",
            font(15.0),
            TEXT,
        );
        painter.text(
            pos2(row_rect.right() - 46.0, y),
            Align2::CENTER_CENTER,
            icon::DOTS_THREE,
            font(18.0),
            TEXT,
        );
        ui.interact(row_rect, ui.id().with(("device-row", slot)), Sense::hover())
            .on_hover_text(&device.name);
    }
    ui.allocate_rect(area, Sense::hover());
}

/// "OBSBOT Tiny 3 Lite StreamCamera" -> "Tiny 3 Lite".
fn model_name(device: &DeviceInfo) -> String {
    let name = device.name.trim();
    let name = name.strip_prefix("OBSBOT ").unwrap_or(name);
    let name = name.strip_suffix(" StreamCamera").unwrap_or(name);
    name.to_string()
}

// ---- OSC ----------------------------------------------------------------------

fn osc_page(ui: &mut Ui, settings: &Settings) {
    disabled(
        ui,
        "OSC remote control is not implemented in Gimbly yet",
        |ui| {
            ui.add_space(4.0);
            label(ui, "Connect");
            ui.add_space(10.0);
            row(
                ui,
                46.0,
                |ui| label(ui, "Connection Type"),
                |ui| {
                    dropdown(
                        ui,
                        "osc-connection",
                        settings.osc_connection.label(),
                        &[],
                        FIELD_WIDTH,
                    );
                },
            );
            row(
                ui,
                46.0,
                |ui| label(ui, "Host"),
                |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(FIELD_WIDTH, FIELD_HEIGHT), Sense::hover());
                    field_face(ui, rect, &settings.osc_host, Some(icon::DOTS_THREE));
                },
            );
            row(
                ui,
                46.0,
                |ui| label(ui, "Receive Port"),
                |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(100.0, FIELD_HEIGHT), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(4), KEY_FIELD);
                    ui.painter().text(
                        pos2(rect.left() + 20.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        settings.osc_port.to_string(),
                        font(14.0),
                        TEXT,
                    );
                },
            );
            rule(ui);
            row(
                ui,
                54.0,
                |ui| label(ui, "OSC Protocol"),
                |ui| {
                    small_button(ui, "View", vec2(110.0, 28.0));
                },
            );
            rule(ui);
            row(
                ui,
                46.0,
                |ui| {
                    label(ui, "TouchOSC Example");
                    ui.add_space(12.0);
                    link(ui, "About TouchOSC");
                },
                |_| {},
            );
            for name in ["TCP sample layout", "UDP sample layout"] {
                row(
                    ui,
                    54.0,
                    |ui| label(ui, name),
                    |ui| {
                        small_button(ui, "Open Folder", vec2(124.0, 30.0));
                    },
                );
            }
        },
    );
}

// ---- building blocks ------------------------------------------------------------

fn label(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(15.0).color(TEXT));
}

fn dim(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(14.0).color(TEXT_DIM));
}

fn link(ui: &mut Ui, text: &str) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font(14.0), LINK);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
    let color = if response.hovered() { TEXT } else { LINK };
    ui.painter().galley(rect.min, galley, color);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Runs `add` in a disabled scope with a tooltip explaining why.
fn disabled(ui: &mut Ui, why: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_enabled_ui(false, |ui| {
        ui.vertical(add);
    })
    .response
    .on_disabled_hover_text(why);
}

/// A full-width row: `left` content left-aligned, `right` content
/// right-aligned, both vertically centred.
fn row(ui: &mut Ui, height: f32, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let salt = (rect.min.x as i32, rect.min.y as i32);
    place(
        ui,
        rect,
        Layout::left_to_right(Align::Center),
        ("row-left", salt),
        left,
    );
    place(
        ui,
        rect,
        Layout::right_to_left(Align::Center),
        ("row-right", salt),
        right,
    );
}

fn rule(ui: &mut Ui) {
    ui.add_space(10.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, RULE);
    ui.add_space(4.0);
}

fn field_face(ui: &Ui, rect: Rect, text: &str, trailing: Option<&str>) {
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(4),
        FIELD,
        Stroke::new(1.0, KEY_FIELD),
        StrokeKind::Inside,
    );
    let clip = Rect::from_min_max(rect.min, pos2(rect.right() - 34.0, rect.bottom()));
    painter.with_clip_rect(clip).text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font(14.0),
        TEXT,
    );
    if let Some(glyph) = trailing {
        let caret = Rect::from_center_size(
            pos2(rect.right() - 16.0, rect.center().y),
            Vec2::splat(22.0),
        );
        painter.rect_filled(caret, CornerRadius::same(3), CARET_BOX);
        painter.text(
            caret.center(),
            Align2::CENTER_CENTER,
            glyph,
            font(12.0),
            TEXT,
        );
    }
}

/// Dropdown in the settings style. Returns the index picked this frame.
fn dropdown(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    current: &str,
    options: &[&str],
    width: f32,
) -> Option<usize> {
    let (rect, response) = ui.allocate_exact_size(vec2(width, FIELD_HEIGHT), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    field_face(ui, rect, current, Some(icon::CARET_DOWN));
    let mut chosen = None;
    if options.is_empty() {
        return None;
    }
    egui::Popup::from_toggle_button_response(&response)
        .id(ui.id().with(("dropdown", id)))
        .width(width)
        .gap(2.0)
        .frame(
            egui::Frame::new()
                .fill(FIELD)
                .stroke(Stroke::new(1.0, KEY_FIELD))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(0, 4)),
        )
        .show(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for (i, option) in options.iter().enumerate() {
                let (item, r) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
                if r.hovered() {
                    ui.painter()
                        .rect_filled(item, CornerRadius::ZERO, BUTTON_HOVER);
                }
                let color = if *option == current {
                    ACCENT_HOVER
                } else {
                    TEXT
                };
                ui.painter().text(
                    pos2(item.left() + 9.0, item.center().y),
                    Align2::LEFT_CENTER,
                    *option,
                    font(14.0),
                    color,
                );
                if r.clicked() {
                    chosen = Some(i);
                }
            }
        });
    chosen
}

fn path_field(ui: &mut Ui, path: &str, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, FIELD_HEIGHT), Sense::click());
    field_face(ui, rect, path, Some(icon::CARET_DOWN));
    response
}

fn small_button(ui: &mut Ui, text: &str, size: Vec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if ui.is_enabled() && response.hovered() {
        BUTTON_HOVER
    } else {
        SMALL_BUTTON
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        fill,
        Stroke::new(1.0, SMALL_BUTTON_STROKE),
        StrokeKind::Inside,
    );
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, text, font(15.0), TEXT);
    response
}

fn square_button(ui: &mut Ui, glyph: &str, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let fill = if ui.is_enabled() && response.hovered() {
        BUTTON_HOVER
    } else {
        SMALL_BUTTON
    };
    ui.painter().rect_filled(rect, CornerRadius::same(3), fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        font(14.0),
        TEXT_DIM,
    );
    response.on_hover_text(tip)
}

fn plain_icon(ui: &mut Ui, glyph: &str, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let color = if response.hovered() {
        TEXT
    } else {
        Color32::from_gray(0xD8)
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        font(17.0),
        color,
    );
    response.on_hover_text(tip)
}

fn key_field(ui: &mut Ui, text: &str, recording: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(310.0, 28.0), Sense::click());
    let stroke = if recording {
        Stroke::new(1.0, ACCENT)
    } else {
        Stroke::NONE
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(3),
        KEY_FIELD,
        stroke,
        StrokeKind::Inside,
    );
    let color = if recording { TEXT_DIM } else { TEXT };
    ui.painter().text(
        pos2(rect.left() + 19.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font(14.0),
        color,
    );
    response.on_hover_text("Click, then press the new shortcut")
}

fn segmented(ui: &mut Ui, labels: &[&str], selected: usize, item_width: f32) -> Option<usize> {
    let size = vec2(item_width * labels.len() as f32 + 4.0, FIELD_HEIGHT + 2.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        FIELD,
        Stroke::new(1.0, KEY_FIELD),
        StrokeKind::Inside,
    );
    let mut clicked = None;
    for (i, text) in labels.iter().enumerate() {
        let item = Rect::from_min_size(
            rect.min + vec2(2.0 + i as f32 * item_width, 2.0),
            vec2(item_width, FIELD_HEIGHT - 2.0),
        );
        let response = ui.interact(item, ui.id().with(("segment", i)), Sense::click());
        if i == selected {
            ui.painter()
                .rect_filled(item, CornerRadius::same(3), ACCENT);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(item, CornerRadius::same(3), BUTTON_HOVER);
        }
        ui.painter().text(
            item.center(),
            Align2::CENTER_CENTER,
            *text,
            font(15.0),
            if i == selected { ON_ACCENT } else { TEXT },
        );
        if response.clicked() && i != selected {
            clicked = Some(i);
        }
    }
    clicked
}

/// Schematic thumbnail of a software layout (drawn, not an image) with its
/// title underneath.
fn mode_card(ui: &Ui, rect: Rect, title: &str, pro: bool, selected: bool) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), NAV_SELECTED);
    if pro {
        // Control panel next to a preview.
        let panel = Rect::from_min_size(rect.min + vec2(30.0, 6.0), vec2(42.0, 44.0));
        painter.rect_filled(panel, CornerRadius::same(2), WINDOW);
        for i in 0..4 {
            let dot =
                Rect::from_min_size(panel.min + vec2(4.0 + i as f32 * 9.0, 5.0), vec2(6.0, 6.0));
            painter.rect_filled(
                dot,
                CornerRadius::same(1),
                if i == 0 { ACCENT } else { BUTTON_HOVER },
            );
        }
        painter.rect_filled(
            Rect::from_min_size(panel.min + vec2(4.0, 16.0), vec2(34.0, 24.0)),
            CornerRadius::same(1),
            CARD,
        );
    } else {
        let preview = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(64.0, 44.0));
        painter.rect_filled(preview, CornerRadius::same(2), Color32::from_gray(0x45));
        painter.rect_filled(
            Rect::from_min_size(rect.min + vec2(74.0, 6.0), vec2(22.0, 44.0)),
            CornerRadius::same(2),
            WINDOW,
        );
    }
    if selected {
        painter.rect_stroke(
            rect.expand(3.0),
            CornerRadius::same(4),
            Stroke::new(2.0, ACCENT),
            StrokeKind::Outside,
        );
    }
    let color = if selected { TEXT } else { TEXT_FAINT };
    painter.text(
        pos2(rect.center().x, rect.bottom() + 20.0),
        Align2::CENTER_CENTER,
        title,
        font(15.0),
        color,
    );
}

/// Maps a key from egui to the keys global shortcuts support.
fn map_key(key: egui::Key) -> Option<Key> {
    use egui::Key as E;
    let name = key.name();
    if name.len() == 1 {
        let c = name.chars().next()?;
        if c.is_ascii_uppercase() {
            return Some(Key::Letter(c));
        }
        if c.is_ascii_digit() {
            return Some(Key::Digit(c as u8 - b'0'));
        }
    }
    Some(match key {
        E::ArrowUp => Key::Up,
        E::ArrowDown => Key::Down,
        E::ArrowLeft => Key::Left,
        E::ArrowRight => Key::Right,
        E::Space => Key::Space,
        E::Home => Key::Home,
        E::End => Key::End,
        E::PageUp => Key::PageUp,
        E::PageDown => Key::PageDown,
        E::Insert => Key::Insert,
        E::Delete => Key::Delete,
        E::Equals | E::Plus => Key::Plus,
        E::Minus => Key::Minus,
        _ => {
            let n: u8 = name.strip_prefix('F')?.parse().ok()?;
            return (1..=24).contains(&n).then_some(Key::F(n));
        }
    })
}
