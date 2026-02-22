//! Lexical analysis for `.gitignore` patterns.
//!
//! The lexer is responsible for performing lexical analysis on `.gitignore` patterns, converting
//! them into a stream of tokens that can be easily parsed by the parser.
//!
//! It is designed to produce token streams which are byte-for-byte identical to the original pattern, so
//! that any given token stream can be converted back into the original pattern.
//!
//! The token streams are independent of the `.gitignore` specification (though are inspired by it), and are
//! designed to be an intermediate representation of the original pattern which can be compiled
//! into Regular Expressions by the [`crate::evaluator::Evaluator`] for efficient matching against file
//! paths.

use std::ops::Deref;

use crate::lexer;

mod error;
mod types;

pub use error::Error;
pub use types::Result;

/// A sequence of tokens parsed from a `.gitignore` pattern by the lexer.
///
/// The tokens are designed to be as close to the original pattern as possible, so that
/// they can be easily converted back into the original pattern if needed.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TokenStream(Vec<Token>);

impl Deref for TokenStream {
    type Target = Vec<Token>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// An individual token parsed from a `.gitignore` pattern by the lexer.
#[derive(Debug, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Token {
    /// `!`
    Negation,
    /// `\x`
    ExplicitLiteral(Vec<u8>),
    /// `hello`
    ImplicitLiteral(Vec<u8>),
    /// `/`
    DirectorySeparator,
    /// `*`
    Asterisk,
    /// `**`
    DoubleAsterisk,
    /// `?`
    QuestionMark,
    /// `a-zA-Z0-9`
    Range(Vec<u8>),
    /// `# Some comment`
    Comment(Vec<u8>),
}

impl Token {
    /// Convert token back into a string representation of the original pattern.
    ///
    /// This is designed to produce a byte-for-byte identical string to the token originally
    /// in the pattern. In other words, any given token converted back into a string should
    /// be exactly the same as the original pattern that produced that token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match &self {
            Self::Negation => "!",
            Self::ExplicitLiteral(bytes) | Self::ImplicitLiteral(bytes) => {
                str::from_utf8(bytes).unwrap_or_default()
            }
            // The slash "/" is used as the directory separator.
            //
            // Notice, this isn't an operating system specific separator (i.e. `\` for
            // Windows) because all `.gitignore` patterns use `/` as the separator, which is where
            // all of these tokens are sourced from.
            Self::DirectorySeparator => "/",
            Self::Asterisk => "*",
            Self::DoubleAsterisk => "**",
            Self::QuestionMark => "?",
            Self::Range(range) => str::from_utf8(range).unwrap_or_default(),
            Self::Comment(comment) => str::from_utf8(comment).unwrap_or_default(),
        }
    }
}

/// Perform lexical analysis on a `.gitignore` pattern, converting it into a stream of tokens that
/// can be easily parsed by the parser.
///
/// ### Errors
///
/// Returns an error if the pattern is not valid syntactically. Otherwise, returns a stream of
/// tokens representing the original pattern.
///
/// This stream is designed to be byte-for-byte identical to the original pattern.
pub fn analyse(pattern: &str) -> lexer::Result<TokenStream> {
    let mut tokens = Vec::<Token>::new();

    let mut iter = pattern.bytes().peekable();
    while let Some(char) = iter.next() {
        // A backslash ("\") can be used to escape any character. E.g., "\*" matches a
        // literal asterisk (and "\a" matches "a", even though there is no need for escaping
        // there). As with fnmatch(3), a backslash at the end of a pattern is an invalid
        // pattern that never matches.
        if char == b'\\' {
            let Some(literal) = iter.next() else {
                // As with fnmatch(3), a backslash at the end of a pattern is an invalid pattern
                // that never matches.
                return Err(lexer::Error::InvalidPattern(pattern.into()));
            };

            tokens.push(Token::ExplicitLiteral(vec![literal]));
            continue;
        }

        if tokens.is_empty() && char == b'#' {
            // A line starting with # serves as a comment.
            tokens.push(Token::Comment(iter.collect()));

            break;
        }

        if tokens.is_empty() && char == b'!' {
            // An optional prefix "!" which negates the pattern; any matching file excluded by a previous
            // pattern will become included again. It is not possible to re-include a file if a parent
            // directory of that file is excluded. Git doesn’t list excluded directories for performance reasons,
            // so any patterns on contained files have no effect, no matter where they are defined. Put a backslash
            // ("\") in front of the first "!" for patterns that begin with a literal "!", for example, "\!important!.txt".
            tokens.push(Token::Negation);
            continue;
        }

        if char == b'/' {
            // The slash "/" is used as the directory separator. Separators may occur at the beginning, middle or
            // end of the `.gitignore` search pattern.
            tokens.push(Token::DirectorySeparator);
            continue;
        }

        if char == b'?' {
            // The character "?" matches any one character except "/".
            tokens.push(Token::QuestionMark);
            continue;
        }

        if char == b'*' {
            tokens.push(match iter.next_if(|char| *char == b'*') {
                // Two consecutive asterisks ("**") in patterns matched against full pathname may have special meaning
                Some(_) => Token::DoubleAsterisk,
                // An asterisk "*" matches anything except a slash.
                None => Token::Asterisk,
            });
            continue;
        }

        if char == b'[' {
            // The range notation, e.g. [a-zA-Z], can be used to match one of the characters in a range. See fnmatch(3) and
            // the FNM_PATHNAME flag for a more detailed description.
            let mut range = Vec::<u8>::new();
            loop {
                let Some(next) = iter.next() else {
                    // Unterminated range
                    return Err(lexer::Error::InvalidPattern(pattern.into()));
                };

                if next == b']' {
                    tokens.push(Token::Range(range));
                    break;
                }

                range.push(next);
            }

            continue;
        }

        if let Some(Token::ImplicitLiteral(chars)) = tokens.last_mut() {
            chars.push(char);
        } else {
            tokens.push(Token::ImplicitLiteral(vec![char]));
        }
    }

    Ok(TokenStream(tokens))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use crate::utils;

    use super::*;

    #[rstest]
    #[case(
        r"hello",
        Ok(TokenStream(vec![
            Token::ImplicitLiteral(vec![b'h', b'e', b'l', b'l', b'o']),
        ]),
    ))]
    #[case(
        r"hello\world",
        Ok(TokenStream(vec![
            Token::ImplicitLiteral(vec![b'h', b'e', b'l', b'l', b'o']),
            Token::ExplicitLiteral(vec![b'w']),
            Token::ImplicitLiteral(vec![b'o', b'r', b'l', b'd']),
        ]),
    ))]
    #[case(
        r"hello\",
        Err(lexer::Error::InvalidPattern(r"hello\".into()))
    )]
    #[case(
        r"!foo",
        Ok(TokenStream(vec![
            Token::Negation,
            Token::ImplicitLiteral(vec![b'f', b'o', b'o']),
        ]),
    ))]
    #[case(
        r"\!foo",
        Ok(TokenStream(vec![
            Token::ExplicitLiteral(vec![b'!']),
            Token::ImplicitLiteral(vec![b'f', b'o', b'o'])
        ]),
    ))]
    #[case(
        r"fo!o",
        Ok(TokenStream(vec![
            Token::ImplicitLiteral(vec![b'f', b'o', b'!', b'o']),
        ]),
    ))]
    #[case(
        r"!fo!o",
        Ok(TokenStream(vec![
            Token::Negation,
            Token::ImplicitLiteral(vec![b'f', b'o', b'!', b'o']),
        ]),
    ))]
    #[case(
        r"hello/world/",
        Ok(TokenStream(vec![
            Token::ImplicitLiteral(vec![b'h', b'e', b'l', b'l', b'o']),
            Token::DirectorySeparator,
            Token::ImplicitLiteral(vec![b'w', b'o', b'r', b'l', b'd']),
            Token::DirectorySeparator,
        ]),
    ))]
    #[case(
        r"hello/?/world/",
        Ok(TokenStream(vec![
            Token::ImplicitLiteral(vec![b'h', b'e', b'l', b'l', b'o']),
            Token::DirectorySeparator,
            Token::QuestionMark,
            Token::DirectorySeparator,
            Token::ImplicitLiteral(vec![b'w', b'o', b'r', b'l', b'd']),
            Token::DirectorySeparator,
        ]),
    ))]
    #[case(
        r"**",
        Ok(TokenStream(vec![
            Token::DoubleAsterisk,
        ]),
    ))]
    #[case(
        r"/foo/**/*bar",
        Ok(TokenStream(vec![
            Token::DirectorySeparator,
            Token::ImplicitLiteral(vec![b'f', b'o', b'o']),
            Token::DirectorySeparator,
            Token::DoubleAsterisk,
            Token::DirectorySeparator,
            Token::Asterisk,
            Token::ImplicitLiteral(vec![b'b', b'a', b'r']),
        ]),
    ))]
    #[case(
        r"/*****",
        Ok(TokenStream(vec![
            Token::DirectorySeparator,
            Token::DoubleAsterisk,
            Token::DoubleAsterisk,
            Token::Asterisk,
        ]),
    ))]
    #[case(
        r"hello/[a-zA-Z0-9]/world",
        Ok(TokenStream(vec![
            Token::ImplicitLiteral(vec![b'h', b'e', b'l', b'l', b'o']),
            Token::DirectorySeparator,
            Token::Range(vec![b'a', b'-', b'z', b'A', b'-', b'Z', b'0', b'-', b'9']),
            Token::DirectorySeparator,
            Token::ImplicitLiteral(vec![b'w', b'o', b'r', b'l', b'd']),
        ]),
    ))]
    #[case(
        r"hello/[a-/world",
        Err(lexer::Error::InvalidPattern(r"hello/[a-/world".into()))
    )]
    #[case(
        r"/hello/[a-zA-Z0-9]/world/",
        Ok(TokenStream(vec![
            Token::DirectorySeparator,
            Token::ImplicitLiteral(vec![b'h', b'e', b'l', b'l', b'o']),
            Token::DirectorySeparator,
            Token::Range(vec![b'a', b'-', b'z', b'A', b'-', b'Z', b'0', b'-', b'9']),
            Token::DirectorySeparator,
            Token::ImplicitLiteral(vec![b'w', b'o', b'r', b'l', b'd']),
            Token::DirectorySeparator,
        ])),
    )]
    #[case(
        r"# Hello World",
        Ok(TokenStream(vec![
            Token::Comment(vec![b' ', b'H', b'e', b'l', b'l', b'o', b' ', b'W', b'o', b'r', b'l', b'd'])
        ])),
    )]
    #[case(
        r"!deep_keep.log",
        Ok(TokenStream(vec![
            Token::Negation,
            Token::ImplicitLiteral(vec![b'd', b'e', b'e', b'p', b'_', b'k', b'e', b'e', b'p', b'.', b'l', b'o', b'g'])
        ])),
    )]
    #[case(
        r"foo ",
        Ok(TokenStream(vec![
            Token::ImplicitLiteral(vec![b'f', b'o', b'o', b' '])
        ])),
    )]
    #[case(
        r"",
        Ok(TokenStream(vec![])),
    )]
    #[case(
        r"   ",
        Ok(TokenStream(vec![
            Token::ImplicitLiteral(vec![b' ', b' ', b' '])
        ])),
    )]
    pub fn test_lexing(#[case] pattern: &str, #[case] expected_output: Result<TokenStream>) {
        let output = super::analyse(pattern);

        assert_eq!(output, expected_output);

        if let Ok(token_stream) = output {
            // Token streams should convert back into a byte-for-byte identical string
            assert_eq!(String::from(token_stream), pattern.to_string());
        }
    }

    proptest! {
        #[test]
        fn test_lexing_never_panics(
            pattern in utils::get_gitignore_pattern_fuzzing_strategy()
        ) {
            // This should never panic and should return Ok
            let output = super::analyse(&pattern);

            // Accept any output, but check it's not Err
            prop_assert!(output.is_ok(), "Failed to lex pattern: {:?}", pattern);
        }
    }
}
