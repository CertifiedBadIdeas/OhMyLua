#![feature(rustc_private)]

// `rustc_private` also selects the linkage mode required by rustc dynamic libraries.
// The final executable needs it even though compiler APIs stay inside `omlua-rustc`.

mod artifact;
mod cli;

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use artifact::LuaArtifact;
use cli::Command;
use omlua_ir::OmProgram;
use omlua_lua_backend::{LuaBackendProfile, lower_program};

fn main() -> ExitCode {
    let command = match cli::parse(env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}\n{}", cli::USAGE);
            return ExitCode::FAILURE;
        }
    };

    match command {
        Command::EmitOmir { source } => emit_omir(&source),
        Command::BuildLua54 { source } => build_lua54(&source),
    }
}

fn emit_omir(source: &Path) -> ExitCode {
    match compile(source) {
        Ok(program) => {
            let mut stdout = io::stdout().lock();
            match write_program(&mut stdout, &program) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("error: failed to write OMIR: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(status) => status,
    }
}

fn build_lua54(source: &Path) -> ExitCode {
    let project_directory = match env::current_dir() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("error: failed to determine the project directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let artifact = match LuaArtifact::prepare(&project_directory) {
        Ok(artifact) => artifact,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let program = match compile(source) {
        Ok(program) => program,
        Err(status) => return status,
    };
    let profile = LuaBackendProfile::lua54();
    let lir = match lower_program(&program, &profile) {
        Ok(lir) => lir,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let lua = match omlua_codegen::emit_lua54(&lir, &profile) {
        Ok(lua) => lua,
        Err(error) => {
            eprintln!("internal compiler {error}");
            return ExitCode::FAILURE;
        }
    };
    match artifact.commit(&lua) {
        Ok(path) => {
            println!("{}", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn compile(source: &Path) -> Result<OmProgram, ExitCode> {
    match omlua_rustc::compile_to_omir(&PathBuf::from(source)) {
        Ok(omlua_rustc::CompilationResult::Program(program)) => Ok(program),
        Ok(omlua_rustc::CompilationResult::RustcFailed(exit_code)) => Err(exit_code),
        Err(error) => {
            eprintln!("{error}");
            Err(ExitCode::FAILURE)
        }
    }
}

fn write_program(output: &mut dyn Write, program: &OmProgram) -> io::Result<()> {
    output.write_all(program.to_string().as_bytes())
}
