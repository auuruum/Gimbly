//! `cargo dev [args]`: build the whole workspace in release mode, then run Gimbly.
//! Extra args are passed to Gimbly. Plain `cargo run` skips the virtual camera DLL.

use std::process::{Command, ExitCode};

fn cargo(args: &[&str], extra: &[String]) -> bool {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(args)
        .args(extra)
        .status()
        .is_ok_and(|s| s.success())
}

fn main() -> ExitCode {
    let extra: Vec<String> = std::env::args().skip(1).collect();
    let ok = cargo(&["build", "--workspace", "--release"], &[])
        && cargo(&["run", "--release", "-p", "gimbly", "--"], &extra);
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
