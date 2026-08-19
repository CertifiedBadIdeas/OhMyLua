#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

mod error;
mod lower;

use std::path::Path;
use std::process::ExitCode;

pub use error::{CompileError, LowerError};
use omlua_ir::OmProgram;
use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;

pub enum CompilationResult {
    Program(OmProgram),
    RustcFailed(ExitCode),
}

#[derive(Default)]
struct OmirCallbacks {
    result: Option<Result<OmProgram, LowerError>>,
}

impl Callbacks for OmirCallbacks {
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        self.result = Some(lower::lower_program(tcx));
        Compilation::Stop
    }
}

pub fn compile_to_omir(source: &Path) -> Result<CompilationResult, CompileError> {
    if !source.is_file() {
        return Err(CompileError::InvalidSource(source.to_owned()));
    }

    let source = source
        .to_str()
        .ok_or_else(|| CompileError::NonUtf8Source(source.to_owned()))?;

    let arguments = vec![
        "omlua-rustc".to_owned(),
        source.to_owned(),
        "--crate-name=omlua_input".to_owned(),
        "--crate-type=bin".to_owned(),
        "--edition=2024".to_owned(),
        "--sysroot".to_owned(),
        env!("OMLUA_RUSTC_SYSROOT").to_owned(),
    ];

    let mut callbacks = OmirCallbacks::default();
    let exit_code = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&arguments, &mut callbacks)
    });

    match callbacks.result {
        Some(Ok(program)) => Ok(CompilationResult::Program(program)),
        Some(Err(error)) => Err(CompileError::Lower(error)),
        None => Ok(CompilationResult::RustcFailed(exit_code)),
    }
}
