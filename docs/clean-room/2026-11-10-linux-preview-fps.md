# Linux MJPEG preview throughput

## Source and method

2026-11-10. Measured Gimbly's JPEG decoder, NV12 packing, and preview RGB
conversion in a temporary Rust harness. FFmpeg's `testsrc2` generated one
4:2:0 JPEG per resolution at quality setting `-q:v 4`. Each timing is the
mean of 16 iterations after three warm-up frames, with buffers reused.
The harness used the decoder and pixel code from the workspace and the
optimization levels from the corresponding Cargo build.

## Setup

CachyOS, Linux 7.3.3-1-cachyos x86_64, Ryzen 7 7700X, Rust 1.99.1.
Synthetic input. Camera model, firmware, and OBSBOT Center version are
inapplicable to this measurement.

## Observed

| CPU work per frame | Original development profile | Optimized development profile |
| --- | ---: | ---: |
| 1080p MJPEG to preview RGB | 203.9 ms | 5.6 ms |
| 4K MJPEG to preview RGB | 798.6 ms | 22.5 ms |
| 1080p NV12 mirror | 23.7 ms | 1.1 ms |
| 4K NV12 mirror | 93.6 ms | 4.7 ms |

The original 1080p processing time allowed about 4.9 frames per second
before texture upload or display. The mirror measurements used a separate
NV12 buffer and measured only the transform.

## Inferred

The default development build could reproduce the reported 5 FPS through
CPU work alone. Optimizing Gimbly, the shared pixel crate, and the MJPEG
decoder gives those steps time within a 1080p60 or 4K30 frame interval on
the synthetic input. [Cargo package profile overrides](https://doc.rust-lang.org/cargo/reference/profiles.html#overrides)

## Unknown

The physical camera and the native Wayland and X11 preview frame rates
still need verification. Windows and macOS runtime checks are pending.

## Test

Run `cargo run -p gimbly -- --probe --mode 1920x1080@30` with the camera
available and compare the number of preview frames received over the
eight-second probe. Then run `cargo run -p gimbly`, open Video Preview, and
check motion on native Wayland and native X11. Repeat at 1080p60 and 4K30
if the camera advertises those modes, and try mirroring the preview.
