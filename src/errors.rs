use std::fmt;

#[derive(Debug)]
pub struct ParseError {
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
    pub source: anyhow::Error,
}

impl ParseError {
    #[track_caller]
    pub fn new(source: impl Into<anyhow::Error>) -> Self {
        let location = std::panic::Location::caller();

        Self {
            file: location.file(),
            line: location.line(),
            column: location.column(),
            source: source.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LLSD parse error at {}:{}:{}",
            self.file, self.line, self.column
        )?;

        write!(f, ": {}", self.source)
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[macro_export]
macro_rules! parse {
    ($expr:expr) => {{
        $expr.map_err(|e| ParseError::new(e))
    }};
}
