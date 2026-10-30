fn main() {
    if let Err(error) = husk_gpui::run() {
        eprintln!("Husk: {error}");
        std::process::exit(1);
    }
}
