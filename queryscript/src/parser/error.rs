use snafu::{Backtrace, Snafu};
use std::collections::BTreeMap;
use std::fmt;
pub type Result<T> = std::result::Result<T, ParserError>;
pub use crate::ast::SourceLocation as ErrorLocation;
use crate::ast::{Location, Pretty};
use crate::error::MultiError;
use colored::*;

pub trait PrettyError: ToString {
    fn location(&self) -> ErrorLocation;

    fn pretty(&self) -> String {
        format!(
            "{}{} {} {}",
            self.location().pretty(),
            ":".white().bold(),
            "error:".bright_red(),
            self.to_string()
        )
    }

    fn pretty_with_code(&self, code: &BTreeMap<String, String>) -> String {
        let pretty = self.pretty();
        let location = self.location();
        let file = location.file();
        if let Some(file) = file {
            if let Some(contents) = code.get(&file) {
                if let Some(annotated) = self.location().annotate(contents) {
                    return format!("{}\n\n{}", pretty, annotated);
                }
            }
        }
        return pretty;
    }
}

#[derive(Clone, Debug)]
pub struct FormattedError {
    pub location: ErrorLocation,
    pub text: String,
}

impl fmt::Display for FormattedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text.as_str())
    }
}

impl PrettyError for FormattedError {
    fn location(&self) -> ErrorLocation {
        self.location.clone()
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum ParserError {
    #[snafu(display("Unexpected token {:?}: {}", token, msg))]
    UnexpectedToken {
        msg: String,
        token: sqlparser::tokenizer::TokenWithLocation,
        backtrace: Option<Backtrace>,
        file: String,
    },

    #[snafu(display("Tokenizer error: {}", source))]
    TokenizerError {
        source: sqlparser::tokenizer::TokenizerError,
        backtrace: Option<Backtrace>,
        file: String,
    },

    #[snafu(display("SQL parser error: {}", source))]
    SQLParserError {
        source: sqlparser::parser::ParserError,
        backtrace: Option<Backtrace>,
        loc: ErrorLocation,
    },

    #[snafu(display("Unimplemented: {}", what))]
    Unimplemented {
        what: String,
        backtrace: Option<Backtrace>,
        loc: ErrorLocation,
    },

    #[snafu(display("Invalid syntax: {}", what))]
    InvalidSyntax {
        what: String,
        backtrace: Option<Backtrace>,
        loc: ErrorLocation,
    },

    #[snafu(display("{}", sources.first().unwrap()))]
    Multiple {
        // This is assumed to be non-empty
        //
        sources: Vec<ParserError>,
    },
}

impl ParserError {
    pub fn unimplemented(loc: ErrorLocation, what: &str) -> ParserError {
        UnimplementedSnafu { loc, what }.build()
    }

    pub fn invalid(loc: ErrorLocation, what: &str) -> ParserError {
        InvalidSyntaxSnafu { loc, what }.build()
    }
}

impl PrettyError for ParserError {
    fn location(&self) -> ErrorLocation {
        match self {
            ParserError::UnexpectedToken { file, token, .. } => {
                ErrorLocation::Single(file.clone(), token.location.clone())
            }
            ParserError::TokenizerError { file, source, .. } => ErrorLocation::Single(
                file.clone(),
                Location {
                    line: source.line,
                    column: source.col,
                },
            ),
            ParserError::SQLParserError { loc, .. } => loc.clone(),
            ParserError::Unimplemented { loc, .. } => loc.clone(),
            ParserError::InvalidSyntax { loc, .. } => loc.clone(),
            ParserError::Multiple { sources } => sources.first().unwrap().location(),
        }
    }
}

impl MultiError for ParserError {
    fn new_multi_error(errs: Vec<Self>) -> Self {
        ParserError::Multiple { sources: errs }
    }
    fn into_errors(self) -> Vec<Self> {
        match self {
            ParserError::Multiple { sources } => {
                sources.into_iter().flat_map(|e| e.into_errors()).collect()
            }
            _ => vec![self],
        }
    }
}

#[allow(unused_macros)]
macro_rules! unexpected_token {
    ($file: expr, $token: expr, $base: expr $(, $args:expr)* $(,)?) => {
        crate::parser::error::UnexpectedTokenSnafu {
            file: $file,
            msg: format!($base $(, $args)*),
            token: $token.clone(),
        }.fail()
    };
}

#[allow(unused_imports)]
pub(crate) use unexpected_token;
