//! Shared branding, generated from assets/logo by the Rust build script.

pub const APP_ID: &str = "io.github.auuruum.gimbly";

use eframe::egui::{self, ColorImage, IconData, TextureHandle, TextureOptions};
use std::sync::{Arc, OnceLock};

const ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon.png"));
const LOGO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/logo.png"));
const SLEEP_NIGHT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sleep-night.png"));
const SLEEP_DUSK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sleep-dusk.png"));

/// Gimbly's own full-frame pictures for Sleep Background tiles 2 and 3.
pub fn sleep_picture(night: bool) -> Arc<IconData> {
    static PICTURES: OnceLock<[Arc<IconData>; 2]> = OnceLock::new();
    let [n, d] = PICTURES.get_or_init(|| {
        [SLEEP_NIGHT, SLEEP_DUSK].map(|png| {
            Arc::new(eframe::icon_data::from_png_bytes(png).expect("Built-in sleep picture"))
        })
    });
    if night { n.clone() } else { d.clone() }
}

pub fn window_icon() -> Arc<IconData> {
    static ICON_DATA: OnceLock<Arc<IconData>> = OnceLock::new();
    ICON_DATA
        .get_or_init(|| {
            Arc::new(eframe::icon_data::from_png_bytes(ICON).expect("Built-in app icon"))
        })
        .clone()
}

pub struct Branding {
    pub icon: TextureHandle,
    pub logo: TextureHandle,
}

impl Branding {
    pub fn new(ctx: &egui::Context) -> Self {
        let load = |name, image: &IconData| {
            ctx.load_texture(
                name,
                ColorImage::from_rgba_unmultiplied(
                    [image.width as usize, image.height as usize],
                    &image.rgba,
                ),
                TextureOptions::LINEAR,
            )
        };
        Self {
            icon: load("gimbly-icon", &window_icon()),
            logo: load(
                "gimbly-logo",
                &eframe::icon_data::from_png_bytes(LOGO).expect("Built-in logo"),
            ),
        }
    }
}
