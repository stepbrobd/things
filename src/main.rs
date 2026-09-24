use std::process::ExitCode;

fn main() -> ExitCode {
    match things::app::run() {
        Ok(status) => status,
        Err(err) => {
            things::common::eprint_line(&things::common::printable_plain(&format!("{err:#}")));
            ExitCode::FAILURE
        }
    }
}
