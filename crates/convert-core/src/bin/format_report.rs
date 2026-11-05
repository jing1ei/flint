//! Writes `FORMATS.md` (or prints it) from the live catalog.

fn main() {
    let md = convert_core::report::markdown();
    let args: Vec<String> = std::env::args().collect();
    match args.get(1) {
        Some(path) => {
            std::fs::write(path, md).expect("could not write the report");
            println!("wrote {path}");
        }
        None => print!("{md}"),
    }
}
