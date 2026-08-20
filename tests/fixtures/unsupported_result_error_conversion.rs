struct SourceError {
    code: i32,
}

struct TargetError {
    code: i32,
}

impl From<SourceError> for TargetError {
    fn from(error: SourceError) -> Self {
        Self { code: error.code }
    }
}

fn source() -> Result<i32, SourceError> {
    Err(SourceError { code: 7 })
}

fn convert() -> Result<i32, TargetError> {
    let value = source()?;
    Ok(value)
}

fn main() {
    let _ = convert();
}
