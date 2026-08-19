use std::fmt;

#[derive(Debug, PartialEq, Eq)]
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
        write!(formatter, "error[OMLUA0002]: {}", self.detail)?;
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
