use thiserror::Error;

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("{0}")]
    DocsNotFound(String),

    #[error("{0}")]
    Internal(String),
}
