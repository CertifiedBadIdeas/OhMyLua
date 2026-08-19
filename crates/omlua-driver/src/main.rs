#![feature(rustc_private)]

// `rustc_private` also selects the linkage mode required by rustc dynamic libraries.
// The final executable needs it even though compiler APIs stay inside `omlua-rustc`.

use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use omlua_ir::OmProgram;

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

    match omlua_rustc::compile_to_omir(&PathBuf::from(source)) {
        Ok(omlua_rustc::CompilationResult::Program(program)) => {
            let mut stdout = io::stdout().lock();
            match write_program(&mut stdout, &program) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("error: failed to write OMIR: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Ok(omlua_rustc::CompilationResult::RustcFailed(exit_code)) => exit_code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn write_program(output: &mut dyn Write, program: &OmProgram) -> io::Result<()> {
    output.write_all(program.to_string().as_bytes())
}
