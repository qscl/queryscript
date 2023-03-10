use snafu::{Backtrace, Snafu};
pub type Result<T> = std::result::Result<T, TypesystemError>;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum TypesystemError {
    #[snafu(display("{}", what))]
    StringError {
        what: String,
        backtrace: Option<Backtrace>,
    },

    #[snafu(display("Unimplemented: {}", what))]
    Unimplemented {
        what: String,
        backtrace: Option<Backtrace>,
    },

    #[snafu(context(false))]
    RuntimeError {
        source: Box<crate::runtime::RuntimeError>,
        backtrace: Option<Backtrace>,
    },

    #[snafu(display("Failed to parse as number: {}", val))]
    NumericParseError {
        val: String,
        backtrace: Option<Backtrace>,
    },

    #[snafu(display("Invalid timestamp precision: {}", val))]
    UnsupportedTimestampPrecisionError {
        val: u64,
        backtrace: Option<Backtrace>,
    },

    #[snafu(context(false))]
    Utf8Error {
        source: std::str::Utf8Error,
        backtrace: Option<Backtrace>,
    },

    #[snafu(context(false))]
    ArrowError {
        source: arrow::error::ArrowError,
        backtrace: Option<Backtrace>,
    },

    #[cfg(feature = "clickhouse")]
    #[snafu(context(false))]
    ClickHouseError {
        source: clickhouse_rs::errors::Error,
        backtrace: Option<Backtrace>,
    },
}

#[allow(unused_macros)]
macro_rules! ts_fail {
    ($base: expr $(, $args:expr)* $(,)?) => {
        crate::types::error::StringSnafu {
            what: format!($base $(, $args)*),
        }.fail()
    };
}

#[allow(unused_imports)]
pub(crate) use ts_fail;

#[allow(unused_macros)]
macro_rules! ts_unimplemented {
    ($base: expr $(, $args:expr)* $(,)?) => {
        crate::types::error::UnimplementedSnafu {
            what: format!($base $(, $args)*),
        }.fail()
    };
}

#[allow(unused_imports)]
pub(crate) use ts_unimplemented;
