#![crate_name = "ignored"]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(missing_debug_implementations, rust_2018_idioms, rustdoc::all)]
#![allow(rustdoc::private_doc_tests)]
#![forbid(unsafe_code)]

//! # Ignored
//!
//! > **Note:** This crate _does not currently_ evaluate patterns defined in the global excludes file configured
//! > via `core.excludesFile`. Support for this may be added in the future.
//!
//! A Rust implementation of the `.gitignore` file format for quickly checking whether a path is ignored by git - without invoking the git cli.
//!
//! This crate aims for _full behavioural parity_ with git's handling of `.gitignore` files, including support for
//! all core features of the format such as negation patterns, directory-only patterns, and nested `.gitignore`
//! hierarchies.
//!
//! The full specification of the `.gitignore` format, along with details on file precedence and hierarchy, can be
//! found in the [Git documentation](https://git-scm.com/docs/gitignore#_description).
//!
//! ## Usage
//!
//! ```toml
//! [dependencies]
//! ignored = "0.0.1"
//! ```
//!
//! ### Macro
//!
//! The primary entry point to this crate is the `is_ignored!` macro, which provides a convenient way to check whether
//! a path is ignored according to the `.gitignore` rules in the current repository.
//!
//! The macro uses a global evaluator that caches discovered `.gitignore` files and their parsed glob patterns
//! for improved performance across repeated calls.
//!
//! ```rust
//! use ignored::is_ignored;
//!
//! # std::fs::create_dir("tests/fixtures/mock-project/.git");
//! let ignored = is_ignored!("tests/fixtures/mock-project/file.tmp");
//!
//! assert!(ignored);
//! ```
//!
//! ### Evaluator
//!
//! The `Evaluator` struct provides a more flexible and configurable way to evaluate paths against `.gitignore` rules.
//! It gives you explicit control over caching - for example, allowing multiple evaluators with independent caches -
//! and is useful when the global cache used by `is_ignored!` is not desirable.
//!
//! ```rust
//! use ignored::evaluator::Evaluator;
//!
//! # std::fs::create_dir("tests/fixtures/mock-project/.git");
//! let evaluator = Evaluator::default();
//! let ignored = evaluator.is_ignored("tests/fixtures/mock-project/file.tmp");
//!
//! assert!(ignored);
//! ```
//!
//! ## Comparison
//!
//! ### `ignore` crate
//!
//! The [`ignore`](https://crates.io/crates/ignore) crate is a fantastic directory iterator that supports filtering paths
//! based on `.gitignore` rules.
//!
//! Its primary focus is providing an efficient way to traverse directories while respecting ignore patterns,
//! rather than evaluating arbitrary paths in isolation. While many of its primitives can be used to evaluate
//! paths directly, it shines most in the context of directory traversal and filtering.
//!
//! Like `ignored`, the `ignore` crate also supports the full `.gitignore` specification, without the need to
//! invoke the git cli.
//!
//! However, `ignored` is specifically designed for evaluating arbitrary paths against `.gitignore` rules, with
//! behavioural parity to git, without focusing on directory traversal primitives or other features.
//!
//! If you need to check whether a specific path is ignored by git, `ignored` provides an ergonomic and performant
//! solution without invoking the git cli.
//!
//! If you're looking to traverse directories respecting `.gitignore` patterns, without invoking the git cli, then `ignore`
//! is the best fit.
//!
//! ## Contributing
//!
//! Contributions are very welcome!
//!
//! If you have suggestions, bug reports, or would like to contribute code, please open an issue or submit a pull request.
//!
//! ### Testing
//!
//! This crate includes a comprehensive test suite to ensure behavioural parity with the git cli and adherence
//! to the `.gitignore` specification.
//!
//! Run the tests with:
//!
//! ```bash
//! cargo test
//! ```

pub mod evaluator;
pub mod lexer;

#[doc(hidden)]
pub mod macros;

mod utils;
