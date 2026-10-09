// zam v0.0.1 — build skeleton: content-addressed cache, path-independent.
// The "compilation" is a stub. This binary proves the cache thesis:
// same content from two different directories hits one cache entry.
// Benchmark showed Rust's path-bound fingerprints cost 13× on 4-way worktree;
// this design avoids that by keying on content hash alone.

use sha2::{Sha256, Digest};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

fn cache_dir() -> PathBuf {
    let mut p = env::var("ZAMAK_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = env::var("USERPROFILE")
                .or_else(|_| env::var("HOME"))
                .unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".zamak").join("cache")
        });
    fs::create_dir_all(&p).ok();
    p
}

fn content_hash(src: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(src.as_bytes());
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ponytail: stub compiler — real codegen comes later. Just return an uppercased copy
// so the cache output is observably different from the input.
fn compile(src: &str) -> Vec<u8> {
    src.to_uppercase().into_bytes()
}

fn build(file: &str) -> String {
    let src = fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("zam: cannot read {file}: {e}");
        std::process::exit(1);
    });
    let hash = content_hash(&src);
    let cache = cache_dir();
    let cached = cache.join(format!("{hash}.out"));

    let start = Instant::now();
    let label = if cached.exists() {
        "CACHE HIT"
    } else {
        let out = compile(&src);
        fs::write(&cached, &out).unwrap_or_else(|e| {
            eprintln!("zam: cache write failed: {e}");
            std::process::exit(1);
        });
        "CACHE MISS"
    };
    format!("{label} {hash:.12} {file} {:.1}ms", start.elapsed().as_secs_f64() * 1000.0)
}

fn demo() {
    // Two files with identical content in two different directories.
    // Path-independent cache → second build must be a hit.
    let tmp = env::temp_dir().join("zam_demo");
    let dir_a = tmp.join("worktree_a");
    let dir_b = tmp.join("worktree_b");
    fs::create_dir_all(&dir_a).ok();
    fs::create_dir_all(&dir_b).ok();

    let src = "fn main() { println(\"hello from zamak\") }";
    let file_a = dir_a.join("hello.zam");
    let file_b = dir_b.join("hello.zam");
    fs::write(&file_a, src).unwrap();
    fs::write(&file_b, src).unwrap();

    // Clear any stale cache for this exact content.
    let hash = content_hash(src);
    let cached = cache_dir().join(format!("{hash}.out"));
    fs::remove_file(&cached).ok();

    let r1 = build(file_a.to_str().unwrap());
    let r2 = build(file_b.to_str().unwrap());

    println!("{r1}");
    println!("{r2}");

    assert!(r1.starts_with("CACHE MISS"), "first build should miss");
    assert!(r2.starts_with("CACHE HIT"), "second build from different dir must hit — path independence failed");

    // Different content → must miss.
    let src2 = "fn main() { println(\"different\") }";
    let file_c = dir_a.join("other.zam");
    fs::write(&file_c, src2).unwrap();
    let r3 = build(file_c.to_str().unwrap());
    println!("{r3}");
    assert!(r3.starts_with("CACHE MISS"), "different content must miss");

    println!("\n✓ path-independent content-addressed cache verified");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("demo") => demo(),
        Some("build") => {
            if let Some(file) = args.get(2) {
                println!("{}", build(file));
            } else {
                eprintln!("usage: zam build <file>");
                std::process::exit(1);
            }
        }
        _ => {
            println!("zam 0.0.1 — content-addressed build cache skeleton");
            println!("  zam demo         verify path-independent caching");
            println!("  zam build <file> hash file, check cache, stub-compile if miss");
        }
    }
}
