use std::{env::VarError, io::IsTerminal, sync::OnceLock};

use tracing::level_filters::LevelFilter;
use tracing_subscriber::{EnvFilter, Layer, prelude::*};

use crate::common::eprint_line;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum LogFormat {
    #[default]
    Auto,
    Pretty,
    Simplified,
    Json,
}

/// THINGS_LOG holds a filter directive
///
/// THINGS_LOG_FORMAT holds one of pretty, simplified or json
/// a value that cannot apply is reported and ignored
pub fn init() {
    static INIT: OnceLock<()> = OnceLock::new();
    let _ = INIT.get_or_init(|| {
        let subscriber = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true);

        let log_format = match std::env::var_os("THINGS_LOG_FORMAT") {
            None => LogFormat::Auto,
            Some(value) => match value.to_str() {
                Some("") => LogFormat::Auto,
                Some("pretty") => LogFormat::Pretty,
                Some("simplified") => LogFormat::Simplified,
                Some("json") => LogFormat::Json,
                _ => {
                    eprint_line(&format!(
                        "THINGS_LOG_FORMAT is ignored: {value:?} is not pretty, simplified or json"
                    ));
                    LogFormat::Auto
                }
            },
        };
        let terminal = std::io::stderr().is_terminal();
        let color = terminal && !crate::common::no_color_requested();
        let format = match (log_format, terminal) {
            (LogFormat::Auto, true) | (LogFormat::Pretty, _) => {
                subscriber.compact().without_time().with_ansi(color).boxed()
            }
            (LogFormat::Auto, false) | (LogFormat::Simplified, _) => {
                subscriber.with_ansi(false).boxed()
            }
            (LogFormat::Json, _) => subscriber
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(true)
                .with_file(true)
                .with_line_number(true)
                .boxed(),
        };

        // warnings name the objects and repairs behind a notice already printed
        // they wait for THINGS_LOG
        let directive = match std::env::var("THINGS_LOG") {
            Ok(directive) => directive,
            Err(VarError::NotPresent) => String::new(),
            Err(VarError::NotUnicode(value)) => {
                eprint_line(&format!(
                    "THINGS_LOG is ignored: {value:?} is not valid UTF-8"
                ));
                String::new()
            }
        };
        let builder = EnvFilter::builder().with_default_directive(LevelFilter::ERROR.into());
        // a directive that does not parse is reported through eprint_line
        // the library's own report would panic on a closed stderr
        let filter = builder.parse(&directive).unwrap_or_else(|error| {
            eprint_line(&format!("THINGS_LOG is ignored: {error}"));
            builder.parse_lossy("")
        });

        tracing_subscriber::registry()
            .with(format.with_filter(filter))
            .init();
    });
}
