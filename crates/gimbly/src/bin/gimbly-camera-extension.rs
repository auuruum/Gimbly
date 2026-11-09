#[cfg(target_os = "macos")]
#[path = "../vcam/macos_media.rs"]
mod media;

#[cfg(target_os = "macos")]
#[path = "../vcam/macos_extension.rs"]
mod extension;

fn main() {
    #[cfg(target_os = "macos")]
    if let Err(error) = extension::run() {
        eprintln!("Gimbly camera extension: {error}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Gimbly's Core Media I/O extension requires macOS");
        std::process::exit(1);
    }
}
