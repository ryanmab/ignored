use crate::evaluator;

#[doc(hidden)]
pub type Result<T> = std::result::Result<T, evaluator::Error>;
