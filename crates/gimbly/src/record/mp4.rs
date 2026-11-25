//! A minimal MP4 / QuickTime writer for one Motion JPEG video track.
//!
//! Frames go straight into a 64-bit `mdat` box; the sample tables are kept
//! in memory (16 bytes per frame) and written as `moov` at the end.

use std::io::{self, Seek, SeekFrom, Write};

/// Ticks per second in the media timeline.
pub const TIMESCALE: u32 = 90_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brand {
    Mp4,
    QuickTime,
}

pub struct Writer<W: Write + Seek> {
    out: W,
    brand: Brand,
    width: u32,
    height: u32,
    mdat_at: u64,
    offsets: Vec<u64>,
    sizes: Vec<u32>,
    times: Vec<u64>,
}

impl<W: Write + Seek> Writer<W> {
    pub fn new(mut out: W, brand: Brand, width: u32, height: u32) -> io::Result<Self> {
        let mut head = Vec::new();
        boxed(&mut head, b"ftyp", |b| match brand {
            Brand::Mp4 => b.extend_from_slice(b"isom\0\0\x02\0isomiso2mp41"),
            Brand::QuickTime => b.extend_from_slice(b"qt  \0\0\x02\0qt  "),
        });
        let mdat_at = out.stream_position()? + head.len() as u64;
        head.extend_from_slice(&1u32.to_be_bytes());
        head.extend_from_slice(b"mdat");
        head.extend_from_slice(&16u64.to_be_bytes());
        out.write_all(&head)?;
        Ok(Self {
            out,
            brand,
            width,
            height,
            mdat_at,
            offsets: Vec::new(),
            sizes: Vec::new(),
            times: Vec::new(),
        })
    }

    /// Appends one frame at `ticks` (see [`TIMESCALE`]); times must not go
    /// backwards.
    pub fn frame(&mut self, jpeg: &[u8], ticks: u64) -> io::Result<()> {
        let size = u32::try_from(jpeg.len()).map_err(|_| io::Error::other("frame too large"))?;
        let ticks = ticks.max(self.times.last().copied().unwrap_or(0));
        self.offsets.push(self.out.stream_position()?);
        self.sizes.push(size);
        self.times.push(ticks);
        self.out.write_all(jpeg)
    }

    /// Writes the sample tables. `end` is the time just after the last frame.
    pub fn finish(mut self, end: u64) -> io::Result<W> {
        let mdat_end = self.out.stream_position()?;
        let durations = durations(&self.times, end);
        let total: u64 = durations.iter().map(|&d| u64::from(d)).sum();
        let mut moov = Vec::new();
        boxed(&mut moov, b"moov", |b| {
            full_box(b, b"mvhd", 1, 0, |b| {
                b.extend_from_slice(&[0; 16]); // creation and modification time
                b.extend_from_slice(&TIMESCALE.to_be_bytes());
                b.extend_from_slice(&total.to_be_bytes());
                b.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate 1.0
                b.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
                b.extend_from_slice(&[0; 10]);
                matrix(b);
                b.extend_from_slice(&[0; 24]);
                b.extend_from_slice(&2u32.to_be_bytes()); // next track ID
            });
            boxed(b, b"trak", |b| {
                full_box(b, b"tkhd", 1, 3, |b| {
                    b.extend_from_slice(&[0; 16]);
                    b.extend_from_slice(&1u32.to_be_bytes()); // track ID
                    b.extend_from_slice(&[0; 4]);
                    b.extend_from_slice(&total.to_be_bytes());
                    b.extend_from_slice(&[0; 16]); // reserved, layer, group, volume
                    matrix(b);
                    b.extend_from_slice(&(self.width << 16).to_be_bytes());
                    b.extend_from_slice(&(self.height << 16).to_be_bytes());
                });
                boxed(b, b"mdia", |b| {
                    full_box(b, b"mdhd", 1, 0, |b| {
                        b.extend_from_slice(&[0; 16]);
                        b.extend_from_slice(&TIMESCALE.to_be_bytes());
                        b.extend_from_slice(&total.to_be_bytes());
                        b.extend_from_slice(&0x55C4u16.to_be_bytes()); // "und"
                        b.extend_from_slice(&[0; 2]);
                    });
                    full_box(b, b"hdlr", 0, 0, |b| {
                        let kind: &[u8; 4] = match self.brand {
                            Brand::Mp4 => &[0; 4],
                            Brand::QuickTime => b"mhlr",
                        };
                        b.extend_from_slice(kind);
                        b.extend_from_slice(b"vide");
                        b.extend_from_slice(&[0; 12]);
                        match self.brand {
                            Brand::Mp4 => b.extend_from_slice(b"VideoHandler\0"),
                            Brand::QuickTime => b.extend_from_slice(b"\x0cVideoHandler"),
                        }
                    });
                    boxed(b, b"minf", |b| {
                        full_box(b, b"vmhd", 0, 1, |b| b.extend_from_slice(&[0; 8]));
                        boxed(b, b"dinf", |b| {
                            full_box(b, b"dref", 0, 0, |b| {
                                b.extend_from_slice(&1u32.to_be_bytes());
                                full_box(b, b"url ", 0, 1, |_| {});
                            });
                        });
                        boxed(b, b"stbl", |b| self.sample_tables(b, &durations));
                    });
                });
            });
        });
        self.out.write_all(&moov)?;
        let end_of_file = self.out.stream_position()?;
        self.out.seek(SeekFrom::Start(self.mdat_at + 8))?;
        self.out
            .write_all(&(mdat_end - self.mdat_at).to_be_bytes())?;
        self.out.seek(SeekFrom::Start(end_of_file))?;
        self.out.flush()?;
        Ok(self.out)
    }

    fn sample_tables(&self, b: &mut Vec<u8>, durations: &[u32]) {
        full_box(b, b"stsd", 0, 0, |b| {
            b.extend_from_slice(&1u32.to_be_bytes());
            let kind = match self.brand {
                Brand::Mp4 => b"mp4v",
                Brand::QuickTime => b"jpeg",
            };
            boxed(b, kind, |b| {
                b.extend_from_slice(&[0; 6]);
                b.extend_from_slice(&1u16.to_be_bytes()); // data reference
                b.extend_from_slice(&[0; 4]);
                match self.brand {
                    Brand::Mp4 => b.extend_from_slice(&[0; 12]),
                    // QuickTime: vendor, temporal and spatial quality.
                    Brand::QuickTime => b.extend_from_slice(b"GMBY\0\0\0\0\0\0\x02\0"),
                }
                b.extend_from_slice(&(self.width as u16).to_be_bytes());
                b.extend_from_slice(&(self.height as u16).to_be_bytes());
                b.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi
                b.extend_from_slice(&0x0048_0000u32.to_be_bytes());
                b.extend_from_slice(&[0; 4]);
                b.extend_from_slice(&1u16.to_be_bytes()); // frames per sample
                let mut name = [0u8; 32];
                name[0] = 11;
                name[1..12].copy_from_slice(b"Motion JPEG");
                b.extend_from_slice(&name);
                b.extend_from_slice(&24u16.to_be_bytes()); // depth
                b.extend_from_slice(&(-1i16).to_be_bytes());
                if self.brand == Brand::Mp4 {
                    full_box(b, b"esds", 0, 0, |b| {
                        // ES descriptor > decoder config (JPEG, visual) and
                        // the predefined sync-layer config.
                        b.extend_from_slice(&[0x03, 21, 0, 1, 0]);
                        b.extend_from_slice(&[0x04, 13, 0x6C, 0x11, 0, 0, 0]);
                        b.extend_from_slice(&[0; 8]);
                        b.extend_from_slice(&[0x06, 1, 0x02]);
                    });
                }
            });
        });
        full_box(b, b"stts", 0, 0, |b| {
            let mut runs: Vec<(u32, u32)> = Vec::new();
            for &d in durations {
                match runs.last_mut() {
                    Some((count, delta)) if *delta == d => *count += 1,
                    _ => runs.push((1, d)),
                }
            }
            b.extend_from_slice(&(runs.len() as u32).to_be_bytes());
            for (count, delta) in runs {
                b.extend_from_slice(&count.to_be_bytes());
                b.extend_from_slice(&delta.to_be_bytes());
            }
        });
        full_box(b, b"stsc", 0, 0, |b| {
            // One sample per chunk.
            for v in [1u32, 1, 1, 1] {
                b.extend_from_slice(&v.to_be_bytes());
            }
        });
        full_box(b, b"stsz", 0, 0, |b| {
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&(self.sizes.len() as u32).to_be_bytes());
            for size in &self.sizes {
                b.extend_from_slice(&size.to_be_bytes());
            }
        });
        full_box(b, b"co64", 0, 0, |b| {
            b.extend_from_slice(&(self.offsets.len() as u32).to_be_bytes());
            for offset in &self.offsets {
                b.extend_from_slice(&offset.to_be_bytes());
            }
        });
    }
}

/// Sample durations from start times; the last one runs to `end`, or
/// repeats the previous duration when `end` is not later.
fn durations(times: &[u64], end: u64) -> Vec<u32> {
    let mut out: Vec<u32> = times
        .windows(2)
        .map(|w| (w[1] - w[0]).min(u64::from(u32::MAX)) as u32)
        .collect();
    if let Some(&last) = times.last() {
        let tail = match end.checked_sub(last) {
            Some(d) if d > 0 => d.min(u64::from(u32::MAX)) as u32,
            _ => out.last().copied().unwrap_or(TIMESCALE / 30),
        };
        out.push(tail);
    }
    out
}

fn boxed(out: &mut Vec<u8>, kind: &[u8; 4], body: impl FnOnce(&mut Vec<u8>)) {
    let start = out.len();
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(kind);
    body(out);
    let size = (out.len() - start) as u32;
    out[start..start + 4].copy_from_slice(&size.to_be_bytes());
}

fn full_box(
    out: &mut Vec<u8>,
    kind: &[u8; 4],
    version: u8,
    flags: u32,
    body: impl FnOnce(&mut Vec<u8>),
) {
    boxed(out, kind, |b| {
        b.push(version);
        b.extend_from_slice(&flags.to_be_bytes()[1..]);
        body(b);
    });
}

fn matrix(out: &mut Vec<u8>) {
    for v in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        out.extend_from_slice(&v.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Child boxes of a box body: (type, body).
    fn children(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 8 <= data.len() {
            let mut size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as u64;
            let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap();
            let mut head = 8;
            if size == 1 {
                size = u64::from_be_bytes(data[at + 8..at + 16].try_into().unwrap());
                head = 16;
            }
            out.push((kind, &data[at + head..at + size as usize]));
            at += size as usize;
        }
        out
    }

    fn find<'a>(data: &'a [u8], path: &[&[u8; 4]]) -> &'a [u8] {
        let (first, rest) = path.split_first().unwrap();
        let body = children(data)
            .into_iter()
            .find(|(k, _)| k == *first)
            .unwrap()
            .1;
        if rest.is_empty() {
            body
        } else {
            find(body, rest)
        }
    }

    fn be32(data: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(data[at..at + 4].try_into().unwrap())
    }

    fn write(brand: Brand) -> Vec<u8> {
        let mut writer = Writer::new(Cursor::new(Vec::new()), brand, 1280, 720).unwrap();
        writer.frame(&[0xFF, 0xD8, 1], 0).unwrap();
        writer.frame(&[0xFF, 0xD8, 2, 2], 750).unwrap();
        writer.frame(&[0xFF, 0xD8, 3], 1500).unwrap();
        writer.finish(2250).unwrap().into_inner()
    }

    #[test]
    fn durations_run_to_the_end_or_repeat() {
        assert_eq!(durations(&[0, 750, 1500], 2250), [750, 750, 750]);
        assert_eq!(durations(&[0, 750, 1700], 1700), [750, 950, 950]);
        assert_eq!(durations(&[5], 5), [3000]);
        assert!(durations(&[], 0).is_empty());
    }

    #[test]
    fn mp4_tables_point_at_every_frame() {
        let data = write(Brand::Mp4);
        let top: Vec<[u8; 4]> = children(&data).iter().map(|(k, _)| *k).collect();
        assert_eq!(top, [*b"ftyp", *b"mdat", *b"moov"]);
        assert_eq!(&find(&data, &[b"ftyp"])[..4], b"isom");
        assert_eq!(find(&data, &[b"mdat"]).len(), 10);

        let stbl = find(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
        let entry = &find(stbl, &[b"stsd"])[8..];
        assert_eq!(&entry[4..8], b"mp4v");
        assert_eq!(be32(&entry[8 + 24..], 0) >> 16, 1280);
        let esds = find(&entry[8 + 78..], &[b"esds"]);
        assert_eq!(esds[4 + 7], 0x6C, "object type: JPEG");

        assert_eq!(
            find(stbl, &[b"stts"])[4..],
            [0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 2, 238]
        );
        let stsz = find(stbl, &[b"stsz"]);
        assert_eq!((be32(stsz, 8), be32(stsz, 12), be32(stsz, 16)), (3, 3, 4));
        let co64 = find(stbl, &[b"co64"]);
        let frames: Vec<u8> = (0..3)
            .map(|i| {
                let at = u64::from_be_bytes(co64[8 + i * 8..16 + i * 8].try_into().unwrap());
                data[at as usize + 2]
            })
            .collect();
        assert_eq!(frames, [1, 2, 3]);
        let mvhd = find(&data, &[b"moov", b"mvhd"]);
        assert_eq!(be32(mvhd, 20), TIMESCALE);
        assert_eq!(u64::from_be_bytes(mvhd[24..32].try_into().unwrap()), 2250);
    }

    #[test]
    fn quicktime_uses_its_brand_and_jpeg_entry() {
        let data = write(Brand::QuickTime);
        assert_eq!(&find(&data, &[b"ftyp"])[..4], b"qt  ");
        let stbl = find(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
        let entry = &find(stbl, &[b"stsd"])[8..];
        assert_eq!(&entry[4..8], b"jpeg");
        assert_eq!(entry.len(), 86, "no esds in QuickTime");
        let hdlr = find(&data, &[b"moov", b"trak", b"mdia", b"hdlr"]);
        assert_eq!(&hdlr[4..12], b"mhlrvide");
    }
}
