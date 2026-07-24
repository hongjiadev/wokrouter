fn main() {
    if wokrouter_desktop::run().is_err() {
        eprintln!("WokRouter desktop runtime failed.");
    }
}
