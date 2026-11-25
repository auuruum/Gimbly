//! A minimal Matroska writer for one Motion JPEG video track.
//!
//! Layout: EBML header, then a Segment with a SeekHead (pointing at Cues),
//! Info, Tracks, one Cluster per second and Cues at the end. Sizes that are
//! only known later are written as 8-byte placeholders and patched.

use std::io::{self, Seek, SeekFrom, Write};

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TRACKS: u32 = 0x1654_AE6B;
const CLUSTER: u32 = 0x1F43_B675;
const CUES: u32 = 0x1C53_BB6B;
const VOID: u32 = 0xEC;
const DURATION: u32 = 0x4489;

/// Bytes reserved after the Segment header for the SeekHead.
const SEEK_HEAD_SPACE: usize = 40;
/// Start a new cluster after this many milliseconds.
const CLUSTER_MS: u64 = 1000;

pub struct Writer<W: Write + Seek> {
    out: W,
    /// Offset of the first byte inside the Segment.
    segment_data: u64,
    duration_at: u64,
    cluster: Option<(u64, u64)>,
    cues: Vec<(u64, u64)>,
    last_ms: u64,
}

impl<W: Write + Seek> Writer<W> {
    pub fn new(mut out: W, width: u32, height: u32, frame_ns: u64) -> io::Result<Self> {
        let mut head = Vec::new();
        element(&mut head, EBML, |b| {
            uint(b, 0x4286, 1);
            uint(b, 0x42F7, 1);
            uint(b, 0x42F2, 4);
            uint(b, 0x42F3, 8);
            string(b, 0x4282, "matroska");
            uint(b, 0x4287, 4);
            uint(b, 0x4285, 2);
        });
        id(&mut head, SEGMENT);
        let segment_size_at = head.len() as u64;
        head.extend_from_slice(&UNKNOWN_SIZE);
        let segment_data = head.len() as u64;
        void(&mut head, SEEK_HEAD_SPACE);
        let mut info = Vec::new();
        uint(&mut info, 0x2AD7B1, 1_000_000);
        string(&mut info, 0x4D80, "Gimbly");
        string(&mut info, 0x5741, "Gimbly");
        id(&mut info, DURATION);
        size(&mut info, 8);
        let duration_in_info = info.len();
        info.extend_from_slice(&0f64.to_be_bytes());
        id(&mut head, INFO);
        size(&mut head, info.len() as u64);
        let duration_at = (head.len() + duration_in_info) as u64;
        head.extend_from_slice(&info);
        element(&mut head, TRACKS, |b| {
            element(b, 0xAE, |b| {
                uint(b, 0xD7, 1);
                uint(b, 0x73C5, 1);
                uint(b, 0x83, 1);
                string(b, 0x86, "V_MJPEG");
                uint(b, 0x9C, 0);
                uint(b, 0x23E383, frame_ns);
                element(b, 0xE0, |b| {
                    uint(b, 0xB0, u64::from(width));
                    uint(b, 0xBA, u64::from(height));
                });
            });
        });
        let base = out.stream_position()?;
        out.write_all(&head)?;
        debug_assert_eq!(segment_size_at, segment_data - 8);
        Ok(Self {
            out,
            segment_data: base + segment_data,
            duration_at: base + duration_at,
            cluster: None,
            cues: Vec::new(),
            last_ms: 0,
        })
    }

    /// Appends one frame at `ms` milliseconds; times must not go backwards.
    pub fn frame(&mut self, jpeg: &[u8], ms: u64) -> io::Result<()> {
        let ms = ms.max(self.last_ms);
        self.last_ms = ms;
        let start = match self.cluster {
            Some((_, start)) if ms - start < CLUSTER_MS => start,
            _ => self.open_cluster(ms)?,
        };
        let mut head = Vec::with_capacity(16);
        id(&mut head, 0xA3);
        size(&mut head, jpeg.len() as u64 + 4);
        head.push(0x81); // track 1
        head.extend_from_slice(&((ms - start) as i16).to_be_bytes());
        head.push(0x80); // keyframe
        self.out.write_all(&head)?;
        self.out.write_all(jpeg)
    }

    fn open_cluster(&mut self, ms: u64) -> io::Result<u64> {
        self.close_cluster()?;
        let at = self.out.stream_position()?;
        let mut head = Vec::new();
        id(&mut head, CLUSTER);
        head.extend_from_slice(&UNKNOWN_SIZE);
        uint(&mut head, 0xE7, ms);
        self.out.write_all(&head)?;
        self.cluster = Some((at, ms));
        self.cues.push((ms, at - self.segment_data));
        Ok(ms)
    }

    fn close_cluster(&mut self) -> io::Result<()> {
        if let Some((at, _)) = self.cluster.take() {
            let end = self.out.stream_position()?;
            self.patch(at + 4, &sized(end - at - 12))?;
            self.out.seek(SeekFrom::Start(end))?;
        }
        Ok(())
    }

    fn patch(&mut self, at: u64, bytes: &[u8]) -> io::Result<()> {
        self.out.seek(SeekFrom::Start(at))?;
        self.out.write_all(bytes)
    }

    /// Writes the cues and the final sizes. `duration_ms` is the time just
    /// after the last frame.
    pub fn finish(mut self, duration_ms: u64) -> io::Result<W> {
        self.close_cluster()?;
        let cues_at = self.out.stream_position()?;
        let mut cues = Vec::new();
        element(&mut cues, CUES, |b| {
            for &(ms, position) in &self.cues {
                element(b, 0xBB, |b| {
                    uint(b, 0xB3, ms);
                    element(b, 0xB7, |b| {
                        uint(b, 0xF7, 1);
                        uint(b, 0xF1, position);
                    });
                });
            }
        });
        self.out.write_all(&cues)?;
        let end = self.out.stream_position()?;

        let mut seek_head = Vec::new();
        element(&mut seek_head, SEEK_HEAD, |b| {
            element(b, SEEK, |b| {
                id_element(b, SEEK_ID, CUES);
                uint(b, SEEK_POSITION, cues_at - self.segment_data);
            });
        });
        let rest = SEEK_HEAD_SPACE - seek_head.len();
        void(&mut seek_head, rest);
        let segment_data = self.segment_data;
        self.patch(segment_data, &seek_head)?;
        self.patch(segment_data - 8, &sized(end - segment_data))?;
        let duration = duration_ms.max(self.last_ms) as f64;
        let duration_at = self.duration_at;
        self.patch(duration_at, &duration.to_be_bytes())?;
        self.out.seek(SeekFrom::Start(end))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// "Size unknown" in eight bytes, replaced once the size is known.
const UNKNOWN_SIZE: [u8; 8] = [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

/// A size in the fixed eight-byte form used for patched placeholders.
fn sized(value: u64) -> [u8; 8] {
    let mut bytes = (value & 0x00FF_FFFF_FFFF_FFFF).to_be_bytes();
    bytes[0] = 0x01;
    bytes
}

fn id(out: &mut Vec<u8>, id: u32) {
    let bytes = id.to_be_bytes();
    let skip = bytes.iter().position(|&b| b != 0).unwrap_or(3);
    out.extend_from_slice(&bytes[skip..]);
}

fn size(out: &mut Vec<u8>, value: u64) {
    // Shortest variable-length integer; all-ones values are reserved.
    let len = (1..=8u32)
        .find(|&n| value < (1u64 << (7 * n)) - 1)
        .unwrap_or(8);
    let marked = value | (1u64 << (7 * len));
    out.extend_from_slice(&marked.to_be_bytes()[8 - len as usize..]);
}

fn element(out: &mut Vec<u8>, element_id: u32, body: impl FnOnce(&mut Vec<u8>)) {
    let mut inner = Vec::new();
    body(&mut inner);
    id(out, element_id);
    size(out, inner.len() as u64);
    out.extend_from_slice(&inner);
}

fn uint(out: &mut Vec<u8>, element_id: u32, value: u64) {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    element(out, element_id, |b| b.extend_from_slice(&bytes[skip..]));
}

fn id_element(out: &mut Vec<u8>, element_id: u32, value: u32) {
    element(out, element_id, |b| id(b, value));
}

fn string(out: &mut Vec<u8>, element_id: u32, value: &str) {
    element(out, element_id, |b| b.extend_from_slice(value.as_bytes()));
}

/// A Void element taking exactly `total` bytes (at least two).
fn void(out: &mut Vec<u8>, total: usize) {
    id(out, VOID);
    let body = total.saturating_sub(2);
    size(out, body as u64);
    out.resize(out.len() + body, 0);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    /// Reads one element header: (id, size, header length).
    pub(crate) fn header(data: &[u8]) -> Option<(u32, u64, usize)> {
        let first = *data.first()?;
        let id_len = first.leading_zeros() as usize + 1;
        let id = data
            .get(..id_len)?
            .iter()
            .fold(0u32, |v, &b| v << 8 | u32::from(b));
        let rest = data.get(id_len..)?;
        let size_len = rest.first()?.leading_zeros() as usize + 1;
        let mut value = u64::from(rest[0]) & (0xFF >> size_len);
        for &b in rest.get(1..size_len)? {
            value = value << 8 | u64::from(b);
        }
        Some((id, value, id_len + size_len))
    }

    fn children(data: &[u8]) -> Vec<(u32, &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while let Some((id, len, head)) = header(&data[at..]) {
            let body = &data[at + head..at + head + len as usize];
            out.push((id, body));
            at += head + len as usize;
        }
        out
    }

    #[test]
    fn variable_sizes_use_the_shortest_form() {
        let encode = |v| {
            let mut b = Vec::new();
            size(&mut b, v);
            b
        };
        assert_eq!(encode(5), [0x85]);
        assert_eq!(encode(126), [0xFE]);
        assert_eq!(encode(127), [0x40, 0x7F]);
        assert_eq!(sized(300), [1, 0, 0, 0, 0, 0, 1, 44]);
    }

    #[test]
    fn writes_frames_with_times_sizes_and_cues() {
        let mut writer = Writer::new(Cursor::new(Vec::new()), 1920, 1080, 8_333_333).unwrap();
        let frames: [(&[u8], u64); 4] = [
            (&[0xFF, 0xD8, 1], 0),
            (&[0xFF, 0xD8, 2, 2], 8),
            (&[0xFF, 0xD8, 3], 1500),
            (&[0xFF, 0xD8, 4], 1490), // late frame: clamped
        ];
        for (jpeg, ms) in frames {
            writer.frame(jpeg, ms).unwrap();
        }
        let data = writer.finish(1508).unwrap().into_inner();

        let top = children(&data);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, EBML);
        assert_eq!(top[1].0, SEGMENT);
        let segment = children(top[1].1);
        let ids: Vec<u32> = segment.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [SEEK_HEAD, VOID, INFO, TRACKS, CLUSTER, CLUSTER, CUES]);

        let info = children(segment[2].1);
        let duration = info.iter().find(|(id, _)| *id == DURATION).unwrap().1;
        assert_eq!(f64::from_be_bytes(duration.try_into().unwrap()), 1508.0);

        let mut blocks = Vec::new();
        for (_, cluster) in segment.iter().filter(|(id, _)| *id == CLUSTER) {
            let parts = children(cluster);
            assert_eq!(parts[0].0, 0xE7);
            let base = parts[0].1.iter().fold(0u64, |v, &b| v << 8 | u64::from(b));
            for (id, block) in &parts[1..] {
                assert_eq!(*id, 0xA3);
                let rel = i16::from_be_bytes([block[1], block[2]]) as u64;
                blocks.push((base + rel, block[4..].to_vec()));
            }
        }
        let expected: Vec<(u64, Vec<u8>)> = vec![
            (0, vec![0xFF, 0xD8, 1]),
            (8, vec![0xFF, 0xD8, 2, 2]),
            (1500, vec![0xFF, 0xD8, 3]),
            (1500, vec![0xFF, 0xD8, 4]),
        ];
        assert_eq!(blocks, expected);

        // The SeekHead and the cues point at real elements.
        let segment_start = data.len() - top[1].1.len();
        let seek = children(children(segment[0].1)[0].1);
        let cues_position = seek[1].1.iter().fold(0usize, |v, &b| v << 8 | b as usize);
        assert_eq!(
            header(&data[segment_start + cues_position..]).unwrap().0,
            CUES
        );
        for (_, point) in children(segment[6].1) {
            let positions = children(children(point)[1].1);
            let at = positions[1]
                .1
                .iter()
                .fold(0usize, |v, &b| v << 8 | b as usize);
            assert_eq!(header(&data[segment_start + at..]).unwrap().0, CLUSTER);
        }
    }
}
