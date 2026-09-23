fn main() {
    if let Err(err) = things::app::run() {
        eprintln!("{}", things::common::printable(&format!("{err:#}")));
        std::process::exit(1);
    }
}
