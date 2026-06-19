use std::fmt;

/// Application-level error with a process exit code.
#[derive(Debug)]
pub enum AppError {
    /// Usage / validation of arguments or input (bad selector, parse error, bad flag combo).
    Usage(String),
    /// Result-diff verification failed (internal consistency or `git apply --check`).
    Verify(String),
    /// I/O error reading stdin or writing stdout.
    Io(String),
    /// Unexpected internal error. Reserved for future use: no current code path constructs
    /// it, but it keeps a distinct exit-code slot for invariant violations should one arise.
    Internal(String),
}

impl AppError {
    pub fn exit_code(&self) -> u8 {
        match self {
            AppError::Usage(_) => 2,
            AppError::Internal(_) | AppError::Verify(_) => 70,
            AppError::Io(_) => 74,
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Usage(m) | AppError::Verify(m) | AppError::Io(m) | AppError::Internal(m) => {
                write!(f, "{m}")
            }
        }
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_map() {
        assert_eq!(AppError::Usage("x".into()).exit_code(), 2);
        assert_eq!(AppError::Verify("x".into()).exit_code(), 70);
        assert_eq!(AppError::Internal("x".into()).exit_code(), 70);
        assert_eq!(AppError::Io("x".into()).exit_code(), 74);
    }
}
