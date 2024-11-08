use jsonptr::assign::AssignError;
use jsonptr::index::ParseIndexError;
use jsonptr::resolve::ResolveError;
pub use thiserror::Error;

#[derive(Debug, Error)]
pub enum PatchError {
    #[error("index out of bounds at {0}")]
    OutOfBounds(usize),

    #[error("unexpected type at {0}")]
    UnexpectedType(String),

    #[error("the target path does not exist: {0}")]
    TargetDoesNotExist(String),

    #[error("json_patch error: {0}")]
    JsonPatchError(#[from] json_patch::PatchError),

    #[error("json path resolve error: {0}")]
    ResolveError(#[from] ResolveError),

    #[error("json path assign error: {0}")]
    AssignError(#[from] AssignError),

    #[error("index parse error: {0}")]
    ParseIndexError(#[from] ParseIndexError),
}
