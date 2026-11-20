//! Rasterize our SVG branding at build time; no SVG renderer ships in the app.

#[path = "build/windows_resource.rs"]
mod windows_resource;

use resvg::{tiny_skia, usvg};
use std::{env, error::Error, fs, path::PathBuf};
use windows_resource::Icon;

fn main() -> Result<(), Box<dyn Error>> {
    let assets =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../assets/logo");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let load = |name| -> Result<usvg::Tree, Box<dyn Error>> {
        let path = assets.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        Ok(usvg::Tree::from_data(
            &fs::read(path)?,
            &usvg::Options::default(),
        )?)
    };
    let icon = load("gimbly-icon.svg")?;
    let logo = load("gimbly-logo-dark.svg")?;
    render(&icon, 256)?.save_png(out.join("icon.png"))?;
    render(&logo, 640)?.save_png(out.join("logo.png"))?;

    // Windows needs an executable resource as well as the runtime window icon.
    // We write it ourselves so building needs no resource compiler.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icons = [16, 20, 24, 32, 48, 64, 128, 256]
            .into_iter()
            .map(|size| {
                Ok(Icon {
                    size,
                    png: render(&icon, size)?.encode_png()?,
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        let arch = env::var("CARGO_CFG_TARGET_ARCH")?;
        let extension = if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            "obj"
        } else {
            "o"
        };
        let path = out.join(format!("icon-resource.{extension}"));
        fs::write(&path, windows_resource::icon_object(&arch, &icons)?)?;
        println!("cargo:rustc-link-arg-bins={}", path.display());
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // The standalone development executable also needs a TCC usage
        // description when it asks AVFoundation for camera access.
        let plist =
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing manifest directory")?)
                .join("../../packaging/macos/Info.plist");
        println!("cargo:rerun-if-changed={}", plist.display());
        println!(
            "cargo:rustc-link-arg-bin=gimbly=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
    Ok(())
}

fn render(tree: &usvg::Tree, width: u32) -> Result<tiny_skia::Pixmap, Box<dyn Error>> {
    let scale = width as f32 / tree.size().width();
    let height = (tree.size().height() * scale).round() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("Invalid SVG size")?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}
