//! Usage: cargo fixture-gen --case <name> --out <dir>   (alias in .cargo/config.toml)
//! `cargo fixture-gen --list` prints case names. Built only with `--features fixture-gen`.
#[path = "fixturegen.rs"]
mod fixturegen;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        fixturegen::CASES.iter().for_each(|c| println!("{c}"));
        return;
    }
    let get = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
    };
    let case = get("--case").expect("--case NAME required (see --list)");
    let out = std::path::PathBuf::from(get("--out").expect("--out DIR required"));
    match fixturegen::generate(case, &out).expect("generation failed") {
        fixturegen::Outcome::Ready(m) => println!("{m:#?}"),
        fixturegen::Outcome::Skipped(why) => println!("SKIPPED: {why}"),
    }
}
