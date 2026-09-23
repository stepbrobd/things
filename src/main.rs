use std::process::ExitCode;

fn main() -> ExitCode {
    match things::app::run() {
        Ok(status) => status,
        Err(err) => {
            eprintln!("{}", things::common::printable(&format!("{err:#}")));
            ExitCode::FAILURE
        }
    }
}
