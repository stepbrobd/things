fn main() {
    if let Err(err) = things::app::run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
