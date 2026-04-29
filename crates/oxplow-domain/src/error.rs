use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum DomainError {
    #[error("invalid value: {0}")]
    Invalid(String),
    #[error("not found")]
    NotFound,
    #[error("invariant violated: {0}")]
    Invariant(String),
}
