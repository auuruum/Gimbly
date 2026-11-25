//! ALSA capture through libasound, loaded at run time so that building
//! Gimbly needs no ALSA headers. The `plughw` device converts the camera's
//! own format to 48 kHz stereo 16-bit.

use super::{Format, Sink, matches};
use libc::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};

const FORMAT: Format = Format {
    rate: 48_000,
    channels: 2,
};
const STREAM_CAPTURE: c_int = 1;
const FORMAT_S16_LE: c_int = 2;
const ACCESS_RW_INTERLEAVED: c_int = 3;
/// Frames per read (about 21 ms).
const FRAMES: usize = 1024;

type Pcm = c_void;

/// The few libasound functions Gimbly uses, with the C signatures from the
/// ALSA library documentation.
struct Alsa {
    lib: *mut c_void,
    card_next: unsafe extern "C" fn(*mut c_int) -> c_int,
    card_get_name: unsafe extern "C" fn(c_int, *mut *mut c_char) -> c_int,
    open: unsafe extern "C" fn(*mut *mut Pcm, *const c_char, c_int, c_int) -> c_int,
    set_params:
        unsafe extern "C" fn(*mut Pcm, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int,
    wait: unsafe extern "C" fn(*mut Pcm, c_int) -> c_int,
    readi: unsafe extern "C" fn(*mut Pcm, *mut c_void, c_ulong) -> c_long,
    recover: unsafe extern "C" fn(*mut Pcm, c_int, c_int) -> c_int,
    close: unsafe extern "C" fn(*mut Pcm) -> c_int,
}

impl Alsa {
    fn load() -> Option<Self> {
        // SAFETY: dlopen with a NUL-terminated name; each symbol is read as
        // the function type ALSA documents for it (see `symbol`).
        unsafe {
            let lib = libc::dlopen(c"libasound.so.2".as_ptr(), libc::RTLD_NOW);
            if lib.is_null() {
                return None;
            }
            let alsa = (|| {
                Some(Self {
                    lib,
                    card_next: symbol(lib, c"snd_card_next")?,
                    card_get_name: symbol(lib, c"snd_card_get_name")?,
                    open: symbol(lib, c"snd_pcm_open")?,
                    set_params: symbol(lib, c"snd_pcm_set_params")?,
                    wait: symbol(lib, c"snd_pcm_wait")?,
                    readi: symbol(lib, c"snd_pcm_readi")?,
                    recover: symbol(lib, c"snd_pcm_recover")?,
                    close: symbol(lib, c"snd_pcm_close")?,
                })
            })();
            if alsa.is_none() {
                libc::dlclose(lib);
            }
            alsa
        }
    }

    /// The ALSA card whose name matches the camera model.
    fn find(&self, model: &str) -> Option<c_int> {
        let mut card: c_int = -1;
        // SAFETY: ALSA fills `card` and allocates `name`, which is freed.
        unsafe {
            while (self.card_next)(&mut card) == 0 && card >= 0 {
                let mut name = std::ptr::null_mut();
                if (self.card_get_name)(card, &mut name) != 0 || name.is_null() {
                    continue;
                }
                let text = CStr::from_ptr(name).to_string_lossy().into_owned();
                libc::free(name.cast());
                if matches(&text, model) {
                    return Some(card);
                }
            }
        }
        None
    }
}

/// Looks up `name` in `lib` as a function pointer of type `F`.
///
/// # Safety
///
/// `F` must be the `extern "C"` function type of that symbol.
unsafe fn symbol<F: Copy>(lib: *mut c_void, name: &CStr) -> Option<F> {
    const { assert!(std::mem::size_of::<F>() == std::mem::size_of::<*mut c_void>()) };
    // SAFETY: `lib` is a live dlopen handle and `name` is NUL-terminated;
    // the caller guarantees the pointer has type `F`.
    unsafe {
        let pointer = libc::dlsym(lib, name.as_ptr());
        (!pointer.is_null()).then(|| std::mem::transmute_copy::<*mut c_void, F>(&pointer))
    }
}

impl Drop for Alsa {
    fn drop(&mut self) {
        // SAFETY: the handle came from dlopen and no symbol outlives `self`.
        unsafe { libc::dlclose(self.lib) };
    }
}

pub(super) fn capture(model: &str, stop: &AtomicBool, sink: &mut Sink<'_>) -> Result<(), String> {
    let alsa = Alsa::load().ok_or("libasound is not available; the recording has no sound")?;
    let card = alsa
        .find(model)
        .ok_or("The camera microphone was not found; the recording has no sound")?;
    let name = CString::new(format!("plughw:{card}")).map_err(|e| e.to_string())?;
    let mut pcm = std::ptr::null_mut();
    // SAFETY: ALSA calls on a PCM handle that stays open until `close`;
    // `readi` writes at most FRAMES frames into a buffer of that size.
    unsafe {
        let opened = (alsa.open)(&mut pcm, name.as_ptr(), STREAM_CAPTURE, 0);
        if opened < 0 || pcm.is_null() {
            return Err(format!(
                "Could not open the camera microphone (ALSA error {opened}); another app may be using it"
            ));
        }
        let result = (|| {
            let set = (alsa.set_params)(
                pcm,
                FORMAT_S16_LE,
                ACCESS_RW_INTERLEAVED,
                c_uint::from(FORMAT.channels),
                FORMAT.rate,
                1,
                500_000,
            );
            if set < 0 {
                return Err(format!(
                    "The camera microphone rejected 48 kHz stereo (ALSA error {set})"
                ));
            }
            let mut buffer = vec![0i16; FRAMES * usize::from(FORMAT.channels)];
            while !stop.load(Ordering::Relaxed) {
                let ready = (alsa.wait)(pcm, 100);
                if ready < 0 && (alsa.recover)(pcm, ready, 1) < 0 {
                    return Err(format!(
                        "The camera microphone stopped (ALSA error {ready})"
                    ));
                }
                if ready <= 0 {
                    continue;
                }
                let read = (alsa.readi)(pcm, buffer.as_mut_ptr().cast(), FRAMES as c_ulong);
                if read < 0 {
                    let code = c_int::try_from(read).unwrap_or(-libc::EIO);
                    if code != -libc::EAGAIN && (alsa.recover)(pcm, code, 1) < 0 {
                        return Err(format!("The camera microphone stopped (ALSA error {code})"));
                    }
                    continue;
                }
                let samples = (read as usize).min(FRAMES) * usize::from(FORMAT.channels);
                sink(FORMAT, &buffer[..samples])
                    .map_err(|e| format!("Could not write the sound: {e}"))?;
            }
            Ok(())
        })();
        (alsa.close)(pcm);
        result
    }
}
