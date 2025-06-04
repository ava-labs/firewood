use thiserror::Error;

/// Errors returned by the checker
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum CheckerError {
    /// The root node was not found
    #[error("root node not found")]
    RootNodeNotFound,

    /// IO error
    #[error("IO error")]
    IO(#[from] std::io::Error),
}
