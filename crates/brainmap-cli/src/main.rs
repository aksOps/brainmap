fn main() {
    if let Err(err) = brainmap_cli::run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
