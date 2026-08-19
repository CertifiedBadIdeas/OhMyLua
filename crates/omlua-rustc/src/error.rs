use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum CompileError {
    InvalidSource(PathBuf),
    NonUtf8Source(PathBuf),
    Lower(LowerError),
}

impl fmt::Display for CompileError {
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
            Self::Lower(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lower(error) => Some(error),
            Self::InvalidSource(_) | Self::NonUtf8Source(_) => None,
        }
    }
}

#[derive(Debug)]
pub struct LowerError {
    function: Option<String>,
    block: Option<u32>,
    detail: String,
}

impl LowerError {
    pub(crate) fn program(detail: impl Into<String>) -> Self {
        Self {
            function: None,
            block: None,
            detail: detail.into(),
        }
    }

    pub(crate) fn function(function: &str, detail: impl Into<String>) -> Self {
        Self {
            function: Some(function.to_owned()),
            block: None,
            detail: detail.into(),
        }
    }

    pub(crate) fn block(function: &str, block: u32, detail: impl Into<String>) -> Self {
        Self {
            function: Some(function.to_owned()),
            block: Some(block),
            detail: detail.into(),
        }
    }

    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for LowerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "error[OMLUA0001]: {}", self.detail)?;
        if let Some(function) = &self.function {
            write!(formatter, "\n  in function `{function}`")?;
        }
        if let Some(block) = self.block {
            write!(formatter, ", basic block bb{block}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LowerError {}
