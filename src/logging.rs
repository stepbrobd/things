use std::sync::OnceLock;

use tracing::level_filters::LevelFilter;
use tracing_subscriber::{EnvFilter, Layer, prelude::*};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum LogFormat {
    #[default]
    Auto,
    Pretty,
    Simplified,
    Json,
}

/// THINGS_LOG holds a filter directive, THINGS_LOG_FORMAT one of pretty, simplified or json
pub fn init() {
    static INIT: OnceLock<()> = OnceLock::new();
    let _ = INIT.get_or_init(|| {
        let subscriber = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true);

        let log_format = match std::env::var("THINGS_LOG_FORMAT").as_deref() {
            Ok("pretty") => LogFormat::Pretty,
            Ok("simplified") => LogFormat::Simplified,
            Ok("json") => LogFormat::Json,
            _ => LogFormat::Auto,
        };
        let format = match (log_format, console::user_attended()) {
            (LogFormat::Auto, true) | (LogFormat::Pretty, _) => {
                subscriber.compact().without_time().boxed()
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

        // warnings name the objects and repairs behind a notice already printed, they wait for THINGS_LOG
        let directive = std::env::var("THINGS_LOG").unwrap_or_default();
        let filter = EnvFilter::builder()
            .with_default_directive(LevelFilter::ERROR.into())
            .parse_lossy(directive);

        tracing_subscriber::registry()
            .with(format.with_filter(filter))
            .init();
    });
}
