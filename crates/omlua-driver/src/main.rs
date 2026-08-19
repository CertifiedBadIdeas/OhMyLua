#![feature(rustc_private)]

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let executable = arguments.next().unwrap_or_default();
    let Some(source) = arguments.next() else {
        eprintln!("usage: {} <source.rs>", PathBuf::from(executable).display());
        return ExitCode::FAILURE;
    };

    if arguments.next().is_some() {
        eprintln!("error: expected exactly one Rust source path");
        return ExitCode::FAILURE;
    }

    match omlua_rustc::print_mir(&PathBuf::from(source)) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
