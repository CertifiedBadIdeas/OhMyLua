#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;

#[derive(Debug)]
pub enum MirError {
    InvalidSource(PathBuf),
    NonUtf8Source(PathBuf),
    WriteMir(io::Error),
}

impl fmt::Display for MirError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSource(path) => {
                write!(formatter, "source file does not exist: {}", path.display())
            }
            Self::NonUtf8Source(path) => {
                write!(
                    formatter,
                    "source path is not valid UTF-8: {}",
                    path.display()
                )
            }
            Self::WriteMir(error) => write!(formatter, "failed to write MIR: {error}"),
        }
    }
}

impl std::error::Error for MirError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::WriteMir(error) => Some(error),
            Self::InvalidSource(_) | Self::NonUtf8Source(_) => None,
        }
    }
}

#[derive(Default)]
struct MirCallbacks {
    write_error: Option<io::Error>,
}

impl Callbacks for MirCallbacks {
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        let mut stdout = io::stdout().lock();
        if let Err(error) = rustc_middle::mir::pretty::write_mir_pretty(tcx, &mut stdout) {
            self.write_error = Some(error);
        }

        Compilation::Stop
    }
}

pub fn print_mir(source: &Path) -> Result<ExitCode, MirError> {
    if !source.is_file() {
        return Err(MirError::InvalidSource(source.to_owned()));
    }

    let source = source
        .to_str()
        .ok_or_else(|| MirError::NonUtf8Source(source.to_owned()))?;

    let arguments = vec![
        "omlua-rustc".to_owned(),
        source.to_owned(),
        "--crate-name=omlua_input".to_owned(),
        "--crate-type=bin".to_owned(),
        "--edition=2024".to_owned(),
        "--sysroot".to_owned(),
        env!("OMLUA_RUSTC_SYSROOT").to_owned(),
    ];

    let mut callbacks = MirCallbacks::default();
    let exit_code = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&arguments, &mut callbacks)
    });

    match callbacks.write_error {
        Some(error) => Err(MirError::WriteMir(error)),
        None => Ok(exit_code),
    }
}
