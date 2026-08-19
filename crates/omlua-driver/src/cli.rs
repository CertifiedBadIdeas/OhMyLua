use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::PathBuf;

pub const USAGE: &str = concat!(
    "usage:\n",
    "  omlua-driver emit-omir <source.rs>\n",
    "  omlua-driver build --backend lua54 <source.rs>",
);

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    EmitOmir { source: PathBuf },
    BuildLua54 { source: PathBuf },
}

#[derive(Debug, PartialEq, Eq)]
pub struct CliError(String);

pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, CliError> {
    let arguments: Vec<_> = arguments.into_iter().collect();
    let Some(mode) = arguments.first() else {
        return Err(error("missing command"));
    };

    if mode == "emit-omir" {
        return parse_emit_omir(&arguments[1..]);
    }
    if mode == "build" {
        return parse_build(&arguments[1..]);
    }
    Err(error(format!(
        "unknown command `{}`",
        mode.to_string_lossy()
    )))
}

fn parse_emit_omir(arguments: &[OsString]) -> Result<Command, CliError> {
    match arguments {
        [] => Err(error("missing Rust source path for `emit-omir`")),
        [source] => Ok(Command::EmitOmir {
            source: PathBuf::from(source),
        }),
        _ => Err(error("extra arguments after the Rust source path")),
    }
}

fn parse_build(arguments: &[OsString]) -> Result<Command, CliError> {
    let Some(flag) = arguments.first() else {
        return Err(error("missing `--backend lua54` for `build`"));
    };
    if flag != "--backend" {
        return Err(error("expected `--backend` immediately after `build`"));
    }
    let Some(backend) = arguments.get(1) else {
        return Err(error("missing value for `--backend`"));
    };
    if backend != "lua54" {
        return Err(error(format!(
            "unknown backend `{}`",
            backend.to_string_lossy()
        )));
    }
    let Some(source) = arguments.get(2) else {
        return Err(error("missing Rust source path for `build`"));
    };
    if arguments[3..]
        .iter()
        .any(|argument| argument == OsStr::new("--backend"))
    {
        return Err(error("`--backend` was specified more than once"));
    }
    if arguments.len() != 3 {
        return Err(error("extra arguments after the Rust source path"));
    }
    Ok(Command::BuildLua54 {
        source: PathBuf::from(source),
    })
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

fn error(message: impl Into<String>) -> CliError {
    CliError(format!("error: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_only_the_two_explicit_command_forms() {
        assert_eq!(
            parse(args(&["emit-omir", "source.rs"])),
            Ok(Command::EmitOmir {
                source: PathBuf::from("source.rs")
            })
        );
        assert_eq!(
            parse(args(&["build", "--backend", "lua54", "source.rs"])),
            Ok(Command::BuildLua54 {
                source: PathBuf::from("source.rs")
            })
        );
    }

    #[test]
    fn rejects_missing_values() {
        assert_eq!(
            parse(args(&[])).unwrap_err().to_string(),
            "error: missing command"
        );
        assert_eq!(
            parse(args(&["emit-omir"])).unwrap_err().to_string(),
            "error: missing Rust source path for `emit-omir`"
        );
        assert_eq!(
            parse(args(&["build"])).unwrap_err().to_string(),
            "error: missing `--backend lua54` for `build`"
        );
        assert_eq!(
            parse(args(&["build", "--backend"]))
                .unwrap_err()
                .to_string(),
            "error: missing value for `--backend`"
        );
        assert_eq!(
            parse(args(&["build", "--backend", "lua54"]))
                .unwrap_err()
                .to_string(),
            "error: missing Rust source path for `build`"
        );
    }

    #[test]
    fn rejects_unknown_duplicate_and_extra_arguments() {
        assert_eq!(
            parse(args(&["source.rs"])).unwrap_err().to_string(),
            "error: unknown command `source.rs`"
        );
        assert_eq!(
            parse(args(&["build", "--backend", "lua53", "source.rs"]))
                .unwrap_err()
                .to_string(),
            "error: unknown backend `lua53`"
        );
        assert_eq!(
            parse(args(&[
                "build",
                "--backend",
                "lua54",
                "source.rs",
                "--backend",
                "lua54",
            ]))
            .unwrap_err()
            .to_string(),
            "error: `--backend` was specified more than once"
        );
        assert_eq!(
            parse(args(&["emit-omir", "source.rs", "extra"]))
                .unwrap_err()
                .to_string(),
            "error: extra arguments after the Rust source path"
        );
    }
}
