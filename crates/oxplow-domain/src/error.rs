use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid value: {0}")]
    Invalid(String),
    #[error("not found")]
    NotFound,
    #[error("invariant violated: {0}")]
    Invariant(String),
}
