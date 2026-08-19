use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub struct CodegenError {
    detail: String,
}

impl CodegenError {
    pub(crate) fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for CodegenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "error[OMLUA0003]: {}", self.detail)
    }
}

impl std::error::Error for CodegenError {}
