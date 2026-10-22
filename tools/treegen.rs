//! Synthetic file-tree generator shared by `examples/gen_tree.rs`.
//! Deterministic (xorshift), std-only. Output is never meant to be committed.
use std::{fs, io::Write, path::Path};

pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo).max(1)
    }
}

pub fn preset(name: &str) -> Option<usize> {
    match name {
        "10k" => Some(10_000),
        "100k" => Some(100_000),
        "1m" | "1M" => Some(1_000_000),
        _ => None,
    }
}

/// Mostly small, long tail, a few big:
/// 90% 100B-4KB, 9% 4-64KB, 0.99% 64KB-1MB, 0.01% 1-8MB.
fn pick_size(rng: &mut Rng) -> u64 {
    match rng.range(0, 10_000) {
        0 => rng.range(1 << 20, 8 << 20),
        1..=99 => rng.range(64 << 10, 1 << 20),
        100..=999 => rng.range(4 << 10, 64 << 10),
        _ => rng.range(100, 4 << 10),
    }
}

const EXTS: [&str; 16] = [
    "txt", "rs", "png", "jpg", "dll", "json", "log", "md", "bin", "zip", "mp4", "cs", "js", "pdf",
    "toml", "tmp",
];

fn fill(seed: u64, size: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut buf = vec![0_u8; size as usize];
    for chunk in buf.chunks_mut(8) {
        let v = rng.next().to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
    buf
}

/// Creates `files` files under `root` spread over a directory tree of the given `depth`.
/// About 5% of files are exact duplicates (same seed and size) of earlier ones.
/// Returns total bytes written.
pub fn generate(root: &Path, files: usize, depth: usize, seed: u64) -> std::io::Result<u64> {
    let mut rng = Rng::new(seed);
    // fan-out so the leaf count is roughly files/40
    let leaves_target = (files / 40).max(1) as f64;
    let fanout = leaves_target
        .powf(1.0 / depth.max(1) as f64)
        .ceil()
        .max(2.0) as u64;
    let mut made_dirs = std::collections::HashSet::new();
    let mut total = 0_u64;
    let mut dup_pool: Vec<(u64, u64)> = Vec::new();
    for i in 0..files {
        let mut dir = root.to_path_buf();
        for level in 0..depth {
            dir.push(format!("d{level}_{}", rng.range(0, fanout)));
        }
        if made_dirs.insert(dir.clone()) {
            fs::create_dir_all(&dir)?;
        }
        let (content_seed, size) = if !dup_pool.is_empty() && rng.range(0, 100) < 5 {
            dup_pool[rng.range(0, dup_pool.len() as u64) as usize]
        } else {
            let pair = (rng.next(), pick_size(&mut rng));
            if dup_pool.len() < 4096 && pair.1 < (256 << 10) {
                dup_pool.push(pair);
            }
            pair
        };
        let ext = EXTS[rng.range(0, EXTS.len() as u64) as usize];
        let path = dir.join(format!("f{i}.{ext}"));
        fs::File::create(path)?.write_all(&fill(content_seed, size))?;
        total += size;
    }
    Ok(total)
}
