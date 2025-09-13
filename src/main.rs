mod entry;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    entry::run_app()
}
