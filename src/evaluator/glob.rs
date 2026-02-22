use std::{
    fmt::{self, Debug, Display},
    path::{MAIN_SEPARATOR, Path},
};

use crate::{
    evaluator::{self, Result},
    lexer::{self, Token, TokenStream},
};

/// An individual glob pattern read from a `.gitignore` file.
///
/// The glob pattern is represented internally as a regular expression, which is used to perform the actual
/// matching against file paths.
///
/// The constructed pattern _does not_ handle negating the match itself (i.e. Glob patterns with leading
/// "!" character).
///
/// Instead, the `is_negated` field is used to indicate whether the pattern is
/// negated or not.
///
/// ## Empty Globs
///
/// Glob patterns can be empty, which is understood to mean "does not match anything".
///
/// This can occur in two cases:
/// 1. When the pattern is an empty string.
/// 2. When the pattern is a comment (i.e. starts with a "#" character).
#[derive(Debug)]
pub struct Glob {
    regex: Option<regex::bytes::Regex>,
    pattern: Option<String>,
    is_negated: bool,
}

impl Display for Glob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.pattern {
            Some(p) => write!(f, "{p}"),
            None => write!(f, "None"),
        }
    }
}

impl Glob {
    /// Create a new glob pattern from a regular expression and the original pattern string.
    ///
    /// The `is_negated` field indicates whether the pattern is negated or not (i.e. whether
    /// it starts with a "!" character). And will cause the `ignored` method to return the
    /// opposite of the match result (i.e. if the pattern is negated and it matches, then
    /// `ignored` will return `false`).
    pub(crate) fn new(regex: regex::bytes::Regex, pattern: &str, is_negated: bool) -> Self {
        Self {
            regex: Some(regex),
            pattern: Some(pattern.into()),
            is_negated,
        }
    }

    /// Create an empty glob pattern, which is understood to mean "does not match anything".
    pub(crate) const fn empty() -> Self {
        Self {
            regex: None,
            pattern: None,
            is_negated: false,
        }
    }

    /// Check if the glob pattern is empty, which is understood to mean "does not match anything".
    ///
    /// This can occur in two cases:
    /// 1. When the pattern is an empty string.
    /// 2. When the pattern is a comment (i.e. starts with a "#" character).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.regex.is_none()
    }

    /// Check if the glob pattern matches the given path.
    ///
    /// Returns `None` if the pattern is empty ([`Glob::is_empty`]), or if the pattern
    /// does not match the path given.
    ///
    /// Otherwise, returns `Some(true)` if the pattern matches and the path **is** ignored, or `Some(false)`
    /// if the pattern matches the path, but is negated (i.e. **is not** ignored).
    #[must_use]
    pub fn is_ignored(&self, path: impl AsRef<Path>) -> Option<bool> {
        let regex = self.regex.as_ref()?;

        let matched = regex.is_match(
            path.as_ref()
                .as_os_str()
                .to_str()
                .unwrap_or_default()
                .as_bytes(),
        );

        if !matched {
            log::trace!(
                "{} did not match {:?} (via regular expression: {regex})",
                path.as_ref().display(),
                &self.pattern.as_ref(),
            );

            return None;
        }

        log::debug!(
            "{} matched {:?} (via regular expression: {regex}). Is ignored: {}",
            path.as_ref().display(),
            self.pattern.as_ref(),
            !self.is_negated
        );

        Some(!self.is_negated)
    }
}

/// Convert a stream of tokens into a `Glob` that can be used to match file paths against
/// the original pattern.
///
/// Under the hood this relies on regular expressions, so the token stream is converted into an intermediate
/// representation which should always be a valid regular expression. This is then compiled into a
/// `regex::Regex` which is used to perform the actual matching against file paths.
impl TryFrom<TokenStream> for Glob {
    type Error = evaluator::Error;

    fn try_from(value: TokenStream) -> Result<Self> {
        if value.is_empty() {
            // No tokens in the stream, which means there's nothing to match
            return Ok(Self::empty());
        }

        if value.len() == 1 && matches!(value.first(), Some(Token::Comment(_))) {
            // A line starting with # serves as a comment.
            return Ok(Self::empty());
        }

        let mut regex = String::new();

        let mut tokens = value.iter().peekable();

        let is_negated = tokens.next_if(|token| *token == &Token::Negation).is_some();

        let mut is_relative_to_root = false;
        let mut is_directory_only = false;

        while let Some(token) = tokens.next() {
            if *token == Token::DirectorySeparator && tokens.peek().is_some() {
                // If there is a separator at the beginning or middle (or both) of the pattern, then
                // the pattern is relative to the directory level of the particular `.gitignore` file itself.
                is_relative_to_root = true;

                if regex.is_empty() {
                    // If there's a separator at the beginning we should not include it as a literal
                    // in the regex, as the input file path will always have any leading
                    // separators stripped off before matching.
                    continue;
                }
            } else if token == &Token::DirectorySeparator && tokens.peek().is_none() {
                // If there is a separator at the end of the pattern then the
                // pattern will only match directories, otherwise the pattern can
                // match both files and directories.
                is_directory_only = true;
            }

            match token {
                Token::ExplicitLiteral(_) => regex.push_str(regex::escape(token.as_str()).as_str()),
                Token::ImplicitLiteral(_) => {
                    let literal = if tokens.peek().is_none() {
                        token
                            .as_str()
                            // Trailing spaces are ignored unless they are quoted with backslash ("\"), in
                            // which case they'd be an explicit literal.
                            .trim_end()
                    } else {
                        token.as_str()
                    };

                    // Implicit literal might still contain regular expression meta characters
                    // (namely `.`). These need to be escaped so the compiled regular expression
                    // text behaves like a literal.
                    regex.push_str(regex::escape(literal).as_str());
                }
                Token::Range(_) => {
                    // The range notation, e.g. [a-zA-Z], can be used to match one of the characters in a
                    // range.
                    regex.push('[');
                    regex.push_str(token.as_str());
                    regex.push(']');
                }
                Token::Comment(_) | Token::Negation => {
                    // Negations and comments should only be present at the start of the pattern,
                    // so if we encounter them here then the pattern is invalid (likely because of
                    // invalid parsing behavior in the crate).
                    return Err(evaluator::Error::InvalidPattern {
                        pattern: value.into(),
                        source: None,
                    });
                }
                Token::Asterisk => {
                    // An asterisk "*" matches anything except a slash
                    regex.push_str(r"[^\\/]+");
                }
                Token::DoubleAsterisk => {
                    regex.push_str(r".*");

                    if regex.is_empty()
                        && tokens
                            .next_if(|next| *next == &Token::DirectorySeparator)
                            .is_some()
                    {
                        is_relative_to_root = true;

                        // A leading "**" followed by a slash means match in all directories. For example, "**/foo" matches file or directory "foo"
                        // anywhere, the same as pattern "foo". "**/foo/bar" matches file or directory "bar" anywhere that is directly under directory "foo".
                        regex.push(MAIN_SEPARATOR);
                    }
                }
                Token::DirectorySeparator
                    if tokens
                        .next_if(|next| *next == &Token::DoubleAsterisk)
                        .is_some() =>
                {
                    // A trailing "/**" matches everything inside. For example, "abc/**" matches all files inside directory "abc", relative to
                    // the location of the `.gitignore` file, with infinite depth.
                    if tokens.peek().is_none() {
                        regex.push_str(r"[\\/].*");
                        break;
                    }

                    // A slash followed by two consecutive asterisks then a slash matches zero or more directories. For example, "a/**/b" matches
                    // "a/b", "a/x/b", "a/x/y/b" and so on.
                    if tokens
                        .next_if(|next| *next == &Token::DirectorySeparator)
                        .is_some()
                    {
                        regex.push_str(r"[\\/]([^\\/]+[\\/])*");
                    }
                }
                Token::DirectorySeparator => {
                    // Directory separators differ between operating systems, so we push a regular
                    // expression match here for either separator (matching gits behaviour), as
                    // opposed to pushing the raw string (which is the operating system specific
                    // separator).
                    regex.push_str(r"[\\/]");
                }
                Token::QuestionMark => {
                    // The character "?" matches any one character except "/"
                    regex.push_str(r"[^\\/]");
                }
            }
        }

        if is_relative_to_root {
            // If there is a separator at the beginning or middle (or both) of the pattern, then
            // the pattern is relative to the directory level of the particular `.gitignore` file itself.
            regex.insert(0, '^');
        } else {
            // When there is no separators, the pattern may match at any level (i.e.
            // directory or filename) below the `.gitignore` level.
            regex.insert_str(0, r"(?:^|[\\/])");

            if !is_directory_only {
                regex.push_str(r"(?:$|[\\/])");
            }
        }

        let regex = regex::bytes::RegexBuilder::new(regex.as_str())
            // Purposely attempt to avoid `.gitignore` glob patterns from translating
            // into very resource intensive regular expressions.
            //
            // Largely this is an arbitrary number, so can be increased. But it should
            // remain small enough to prevent any ReDoS attacks.
            .size_limit(20_000)
            //  The git man page does not call this out directly, however it can be seen that
            //  git matches on _bytes_ not characters (where non-UTF-8 chars, like "é" are a single
            //  _character_ but multiple _bytes_).
            //
            //  Getting this behaviour correct is particularly important for the `?` glob pattern.
            //
            //  This is because `?` is supposed to "match any one character except "/"" (where
            //  "character" actually means byte), and whether its bytes or characters fundamentally changes
            //  the matching bejaviour of `?` on non-UTF-8 strings.
            //
            //  Example:
            //
            //  When matching on characters, "file?.txt" matches "fileé.txt" because the two byte "é" is
            //  treated as a single character.
            //
            //  However, because of its multi-byte nature, when matching on bytes "file?.txt" DOES NOT
            //  match "fileé.txt" because "é" takes two bytes to produce the one character.
            //
            //  In order to match "fileé.txt" when Unicode is turned off the pattern would need to
            //  be "file??.txt", so that it can match bytes.
            .unicode(false)
            .build()
            .map_err(|e| evaluator::Error::InvalidRegex {
                pattern: value.clone().into(),
                regex: regex.as_str().into(),
                source: e,
            })?;

        log::trace!(
            "Converted pattern: {:?} into regex: {:?} (with negation: {})",
            String::from(value.clone()),
            regex,
            is_negated
        );

        Ok(Self::new(
            regex,
            String::from(value.clone()).as_str(),
            is_negated,
        ))
    }
}

/// Convert a stream of tokens back into the original pattern that produced those tokens.
///
/// This is designed to produce a byte-for-byte identical string to the original pattern that produced the
/// token stream. In other words, any given token stream converted back into a string should be exactly the
/// same as the original pattern that produced that token stream.
impl From<TokenStream> for String {
    fn from(value: TokenStream) -> Self {
        let mut pattern = Self::new();

        for token in value.iter() {
            match token {
                Token::ExplicitLiteral(_) => {
                    pattern.push('\\');
                    pattern.push_str(token.as_str());
                }
                Token::Range(_) => {
                    pattern.push('[');
                    pattern.push_str(token.as_str());
                    pattern.push(']');
                }
                Token::Comment(_) => {
                    pattern.push('#');
                    pattern.push_str(token.as_str());
                }
                _ => pattern.push_str(token.as_str()),
            }
        }

        pattern
    }
}

impl TryFrom<&str> for Glob {
    type Error = evaluator::Error;

    fn try_from(value: &str) -> evaluator::Result<Self> {
        if value.is_empty() {
            return Ok(Self::empty());
        }

        let tokens = lexer::analyse(value).map_err(|e| evaluator::Error::InvalidPattern {
            pattern: value.into(),
            source: Some(e),
        })?;

        Self::try_from(tokens)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use crate::utils;

    #[rstest]
    #[case(r"")]
    #[case(r"# This is a comment")]
    pub fn test_empty_globs(#[case] pattern: &str) {
        let output = super::Glob::try_from(pattern)
            .expect("Should never fail to build glob from empty or comment pattern");

        assert!(output.is_empty());
    }

    proptest! {
        #[test]
        fn test_building_never_panics(
            pattern in utils::get_gitignore_pattern_fuzzing_strategy()
        ) {
            let output = super::Glob::try_from(pattern.as_str());

            prop_assert!(output.is_ok(), "Failed to build glob from pattern: {:?}", pattern);
        }
    }
}
