use crate::lexer;

#[doc(hidden)]
pub type Result<T> = std::result::Result<T, lexer::Error>;
