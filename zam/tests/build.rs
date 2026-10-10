use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Workspace(PathBuf);
impl Drop for Workspace {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("preserved failed build fixture: {}", self.0.display());
        } else {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

fn invoke(root: &Path, cache: &Path, command: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zam"))
        .args([command])
        .arg(root)
        .env("ZAMAK_CACHE_DIR", cache)
        .output()
        .unwrap()
}

fn successful(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// 构建进度（CACHE/OBJECT/LINK/OUTPUT）走 stderr，stdout 只留程序输出与 --json 结果。
fn progress(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"hello\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.zm"),
        "use greet\nuse std/io\nfn main() {\ngreet.hello()\nio.println(\"世界\")\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("src/greet.zm"),
        "pub fn hello() { println(\"hello\") }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/unused.zm"),
        "pub struct Unused { value: i64 }\nfn quiet() {}\n",
    )
    .unwrap();
}

#[test]
fn project_cache_execution_and_failures() {
    let workspace = Workspace(std::env::temp_dir().join(format!(
            "zam-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    let a = workspace.0.join("a");
    let b = workspace.0.join("b");
    let cache = workspace.0.join("cache");
    fixture(&a);
    fixture(&b);
    successful(invoke(&a, &cache, "check"));
    assert!(!cache.exists(), "check must not write cache");
    let first = progress(&invoke(&a, &cache, "build"));
    assert_eq!(first.matches("CACHE MISS").count(), 3);
    assert_eq!(first.matches("OBJECT MISS").count(), 3);
    let second = progress(&invoke(&b, &cache, "build"));
    assert_eq!(second.matches("CACHE HIT").count(), 3);
    assert_eq!(second.matches("OBJECT HIT").count(), 3);
    assert!(second.contains("LINK HIT"));
    let artifact = |text: &str| {
        PathBuf::from(
            text.lines()
                .find_map(|line| line.strip_prefix("OUTPUT "))
                .unwrap(),
        )
    };
    assert_eq!(
        fs::read(artifact(&first)).unwrap(),
        fs::read(artifact(&second)).unwrap()
    );
    let run = successful(invoke(&artifact(&second), &cache, "run"));
    assert_eq!(run, "hello\n世界\n");
    fs::write(
        b.join("src/greet.zm"),
        "pub fn hello() { println(\"changed\") }\n",
    )
    .unwrap();
    let changed = progress(&invoke(&b, &cache, "build"));
    assert_eq!(changed.matches("CACHE MISS").count(), 2);
    assert_eq!(changed.matches("CACHE HIT").count(), 1);
    assert_eq!(changed.matches("OBJECT MISS").count(), 1);
    assert_eq!(changed.matches("OBJECT HIT").count(), 2);
    assert!(changed.contains("LINK MISS"));
    assert_eq!(
        successful(invoke(&artifact(&changed), &cache, "run")),
        "changed\n世界\n"
    );

    fs::write(
        b.join("src/greet.zm"),
        "pub fn hello(x: i64) { println(x) }\n",
    )
    .unwrap();
    assert!(
        !invoke(&b, &cache, "build").status.success(),
        "stale interface must not be accepted"
    );
    fs::write(
        b.join("src/main.zm"),
        "use greet\nfn main() { greet.hello(7) }\n",
    )
    .unwrap();
    let interface = progress(&invoke(&b, &cache, "build"));
    assert_eq!(interface.matches("OBJECT MISS").count(), 2);
    assert_eq!(interface.matches("OBJECT HIT").count(), 1);
    assert_eq!(
        successful(invoke(&artifact(&interface), &cache, "run")),
        "7\n"
    );

    fs::write(
        b.join("src/unused.zm"),
        "pub struct Unused { value: bool }\nfn quiet() {}\n",
    )
    .unwrap();
    let unrelated = progress(&invoke(&b, &cache, "build"));
    assert_eq!(unrelated.matches("OBJECT HIT").count(), 3);
    assert!(
        unrelated.contains("LINK HIT"),
        "unused layout must not change native output"
    );
    fs::write(
        b.join("src/main.zm"),
        "use unused\nfn main() {\nlet p = unused.Unused { value: true }\nprintln(p.value)\n}\n",
    )
    .unwrap();
    let record = progress(&invoke(&b, &cache, "build"));
    assert_eq!(record.matches("OBJECT MISS").count(), 1);
    assert_eq!(
        successful(invoke(&artifact(&record), &cache, "run")),
        "true\n"
    );
    fs::write(
        b.join("src/unused.zm"),
        "pub struct Unused { value: i64 }\nfn quiet() {}\n",
    )
    .unwrap();
    assert!(
        !invoke(&b, &cache, "build").status.success(),
        "used field type changes must be checked"
    );
    fs::write(
        b.join("src/main.zm"),
        "use unused\nfn main() {\nlet p = unused.Unused { value: 9 }\nprintln(p.value)\n}\n",
    )
    .unwrap();
    let layout = progress(&invoke(&b, &cache, "build"));
    assert_eq!(layout.matches("OBJECT MISS").count(), 1);
    assert_eq!(layout.matches("OBJECT HIT").count(), 2);
    assert_eq!(successful(invoke(&artifact(&layout), &cache, "run")), "9\n");

    fs::write(b.join("src/unused.zm"), "pub struct Unused { value: i64 }\npub fn make() -> Unused { Unused { value: 11 } }\npub fn relay(p: Unused) -> Unused { p }\nfn quiet() {}\n").unwrap();
    fs::write(b.join("src/main.zm"), "use unused\nfn main() {\nlet p: unused.Unused = unused.relay(unused.make())\nprintln(p.value)\n}\n").unwrap();
    let passing = progress(&invoke(&b, &cache, "build"));
    assert_eq!(
        successful(invoke(&artifact(&passing), &cache, "run")),
        "11\n"
    );
    fs::write(b.join("src/unused.zm"), "struct Unused { value: i64 }\npub fn make() -> Unused { Unused { value: 11 } }\npub fn relay(p: Unused) -> Unused { p }\nfn quiet() {}\n").unwrap();
    assert!(
        !invoke(&b, &cache, "check").status.success(),
        "private named types must not be accessible via annotations"
    );

    fs::write(b.join("src/unused.zm"), "pub struct Unused { value: i64 }\npub fn make() -> Unused { Unused { value: 11 } }\nfn quiet() {}\n").unwrap();
    fs::write(b.join("src/main.zm"), "use unused\nstruct Wrapper { item: unused.Unused }\nfn main() {\nlet mut w = Wrapper { item: unused.make() }\nw.item = unused.make()\nprintln(12)\n}\n").unwrap();
    let nested = progress(&invoke(&b, &cache, "build"));
    assert_eq!(
        successful(invoke(&artifact(&nested), &cache, "run")),
        "12\n"
    );
    fs::write(b.join("src/unused.zm"), "pub struct Unused { value: bool }\npub fn make() -> Unused { Unused { value: true } }\nfn quiet() {}\n").unwrap();
    let nested_layout = progress(&invoke(&b, &cache, "build"));
    assert_eq!(nested_layout.matches("OBJECT MISS").count(), 2);
    assert_eq!(nested_layout.matches("OBJECT HIT").count(), 1);
    assert_eq!(
        successful(invoke(&artifact(&nested_layout), &cache, "run")),
        "12\n"
    );

    let concurrent_cache = workspace.0.join("concurrent-cache");
    let mut children = Vec::new();
    for index in 0..4 {
        let root = workspace.0.join(format!("parallel-{index}"));
        fixture(&root);
        children.push(
            Command::new(env!("CARGO_BIN_EXE_zam"))
                .arg("build")
                .arg(root)
                .env("ZAMAK_CACHE_DIR", &concurrent_cache)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        progress(&child.wait_with_output().unwrap());
    }
    assert_eq!(fs::read_dir(&concurrent_cache).unwrap().count(), 11);
    let artifact_cache = fs::read_dir(&concurrent_cache)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(&artifact_cache, "broken").unwrap();
    assert!(
        !invoke(&a, &concurrent_cache, "build").status.success(),
        "corrupt cache must be reported"
    );

    for source in [
        "fn main() { missing() }",
        "fn main() { println(\"unterminated) }",
        "fn main() { println(\"x\") println(\"y\") }",
        "fn main() { println(\"{x}\") }",
        "fn main() { let x = 1.5 }",
        "fn main() {}\nfn main() {}",
        "fn other() {}",
        "use missing\nfn main() {}",
        "use std/nope\nfn main() {}",
    ] {
        let path = workspace.0.join("bad.zm");
        fs::write(&path, source).unwrap();
        assert!(
            !invoke(&path, &cache, "check").status.success(),
            "accepted {source}"
        );
    }
    fs::write(a.join("src/greet.zm"), "fn hello() {}\n").unwrap();
    assert!(
        !invoke(&a, &cache, "check").status.success(),
        "private function must not be exported"
    );
    fs::write(a.join("src/greet.zm"), "use main\npub fn hello() {}\n").unwrap();
    assert!(
        !invoke(&a, &cache, "check").status.success(),
        "dependency cycles must be rejected"
    );
    fs::write(
        a.join("zam.toml"),
        "[project]\nname=\"hello\"\nentry=\"../main.zm\"\nmodules=\"src\"\n",
    )
    .unwrap();
    assert!(
        !invoke(&a, &cache, "check").status.success(),
        "manifest traversal must be rejected"
    );
    let malformed = workspace.0.join("bad.zbc");
    fs::write(
        &malformed,
        "ZAM-APP-1\nmain:main\nZAM-MODULE-1\nfn private main:main\nprint 世界\nend\n",
    )
    .unwrap();
    assert!(!invoke(&malformed, &cache, "run").status.success());
}
