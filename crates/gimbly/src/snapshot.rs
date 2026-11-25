//! Saving camera snapshots as PNG files in the media folder.

use crate::camera::PreviewFrame;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

/// File name for a snapshot or recording made at `time` (year, month, day,
/// hour, minute, second), e.g. `Gimbly_20261022_153012.png`.
pub fn file_name(time: [u16; 6], attempt: u32, extension: &str) -> String {
    let [y, mo, d, h, mi, s] = time;
    let suffix = if attempt == 0 {
        String::new()
    } else {
        format!("_{attempt}")
    };
    format!("Gimbly_{y:04}{mo:02}{d:02}_{h:02}{mi:02}{s:02}{suffix}.{extension}")
}

/// Writes `frame` into `dir` (created if missing) without overwriting an
/// existing file. Returns the path of the new file.
pub fn save(frame: &PreviewFrame, dir: &Path, time: [u16; 6]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let (path, file) = (0..100)
        .find_map(|attempt| {
            let path = dir.join(file_name(time, attempt, "png"));
            File::create_new(&path).ok().map(|f| (path, f))
        })
        .ok_or_else(|| format!("Could not create a snapshot file in {}", dir.display()))?;
    let result = encode(frame, BufWriter::new(file));
    if let Err(e) = &result {
        let _ = std::fs::remove_file(&path);
        return Err(format!("Could not write {}: {e}", path.display()));
    }
    Ok(path)
}

fn encode(frame: &PreviewFrame, out: impl std::io::Write) -> Result<(), png::EncodingError> {
    let [w, h] = frame.size;
    let mut encoder = png::Encoder::new(out, w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()?;
    // Preview pixels are opaque, so the premultiplied colour is the colour.
    let rgb: Vec<u8> = frame
        .pixels
        .iter()
        .flat_map(|p| [p.r(), p.g(), p.b()])
        .collect();
    writer.write_image_data(&rgb)?;
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Color32;

    #[test]
    fn names_sort_by_time_and_avoid_clashes() {
        let t = [2026, 10, 22, 9, 5, 7];
        assert_eq!(file_name(t, 0, "png"), "Gimbly_20261022_090507.png");
        assert_eq!(file_name(t, 2, "mkv"), "Gimbly_20261022_090507_2.mkv");
    }

    #[test]
    fn writes_a_readable_png_without_overwriting() {
        let dir = std::env::temp_dir().join(format!("gimbly-snapshot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let frame = PreviewFrame {
            size: [3, 2],
            pixels: vec![Color32::from_rgb(10, 20, 30); 6],
        };
        let t = [2026, 1, 2, 3, 4, 5];
        let first = save(&frame, &dir, t).unwrap();
        let second = save(&frame, &dir, t).unwrap();
        assert_ne!(first, second);

        let decoder = png::Decoder::new(std::io::BufReader::new(File::open(&first).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (3, 2));
        assert_eq!(&buf[..3], &[10, 20, 30]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
