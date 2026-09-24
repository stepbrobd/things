use std::process::ExitCode;

fn main() -> ExitCode {
    match things::app::run() {
        Ok(status) => status,
        Err(err) => {
            eprintln!("{}", things::common::printable_plain(&format!("{err:#}")));
            ExitCode::FAILURE
        }
    }
}
