//! Usage: cargo run --release --example gen_tree -- --out DIR (--preset 10k|100k|1m | --files N) [--depth D] [--seed S]
#[path = "../tools/treegen.rs"]
mod treegen;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
    };
    let out = get("--out").expect("--out DIR required");
    let files = match (get("--preset"), get("--files")) {
        (Some(p), _) => treegen::preset(p).expect("preset must be 10k, 100k or 1m"),
        (None, Some(n)) => n.parse().expect("--files N"),
        _ => panic!("give --preset or --files"),
    };
    let depth = get("--depth").map(|d| d.parse().unwrap()).unwrap_or(5);
    let seed = get("--seed").map(|d| d.parse().unwrap()).unwrap_or(42);
    let started = std::time::Instant::now();
    let bytes = treegen::generate(std::path::Path::new(out), files, depth, seed).unwrap();
    println!(
        "generated {files} files, {:.1} MiB, depth {depth}, in {:.1}s -> {out}",
        bytes as f64 / 1048576.0,
        started.elapsed().as_secs_f64()
    );
}
