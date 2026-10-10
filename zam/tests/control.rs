use std::{fs, process::Command};

#[test]
fn control_flow_and_path_ownership() {
    let root = std::env::temp_dir().join(format!("zam-control-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let file = root.join("main.zm");
    let invoke = |command: &str| {
        Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg(command)
            .arg(&file)
            .env("ZAMAK_CACHE_DIR", root.join("cache"))
            .output()
            .unwrap()
    };
    fs::write(&file, include_str!("../../examples/control/main.zm")).unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout)
        .ends_with("3628800\n6\nbranch\nfirst\nnext\nnext\nnext\n"));
    for (source, error) in [
        ("fn main() {\nlet a = \"x\"\nif true { let b = a }\nprintln(a)\n}", "use after move"),
        ("fn main() {\nlet a = \"x\"\nwhile true { let b = a }\n}", "loop moves"),
        ("fn main() {\nif true { let x = 1 }\nprintln(x)\n}", "unknown variable"),
        ("fn main() { if 1 {} }", "type mismatch"),
        ("fn eat(s: string) -> bool { return true }\nfn main() {\nlet a = \"x\"\nlet flag = false && eat(a)\nprintln(a)\n}", "use after move"),
        ("fn main() { println(1 && true) }", "type mismatch"),
        ("fn main() { if true {} println(1) }", "newline"),
        ("fn f() -> i64 { if true { return 1 } }\nfn main() {}", "missing"),
        ("fn main() {\nif true { return } else { return }\nprintln(1)\n}", "unreachable"),
        ("fn eat(s: string) -> bool { return true }\nfn main() {\nlet a = \"x\"\nwhile eat(a) {}\n}", "use after move"),
    ] {
        fs::write(&file, source).unwrap(); let result = invoke("check");
        assert!(!result.status.success(), "accepted {source}");
        assert!(String::from_utf8_lossy(&result.stderr).contains(error), "{}", String::from_utf8_lossy(&result.stderr));
    }
    fs::write(&file, "fn f(flag: bool) {\nlet a = \"live\"\nif flag {\nlet b = a\nreturn\n}\nprintln(a)\n}\nfn main() {\nf(false)\nf(true)\nif true { let x = 1 } else { let x = 2 }\nlet x = 3\nprintln(x)\n}").unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).ends_with("live\n3\n"));
    fs::write(&file, "fn probe() -> bool {\nprintln(99)\nreturn true\n}\nfn main() {\nprintln(false && (1 / 0 == 0))\nprintln(true || (1 / 0 == 0))\nprintln(true && probe())\nprintln(false || probe())\nprintln(true || false && false)\n}\n").unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout)
        .ends_with("false\ntrue\n99\ntrue\n99\ntrue\ntrue\n"));
    fs::write(&file, "fn classify(n: i64) -> string {\nif n < 0 { return \"negative\" } else if n == 0 { return \"zero\" } else if n == 1 { return \"one\" } else { return \"many\" }\n}\nfn show(n: i64) {\nlet s = \"live\"\nif n < 0 { let moved = s\nreturn } else if n == 0 { let moved = s\nreturn }\nprintln(s)\n}\nfn main() {\nprintln(classify(-1))\nprintln(classify(0))\nprintln(classify(1))\nprintln(classify(2))\nshow(-1)\nshow(0)\nshow(2)\n}\n").unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).ends_with("negative\nzero\none\nmany\nlive\n"));
    for (source, error) in [
        (
            "fn main() {\nlet s = \"x\"\nif false {} else if true { let moved = s }\nprintln(s)\n}",
            "use after move",
        ),
        (
            "fn f() -> i64 { if true { return 1 } else if false { return 2 } }\nfn main() {}",
            "missing",
        ),
        ("fn main() { if true {} else if 1 {} }", "type mismatch"),
        (
            "fn main() { if true {} else if false {} println(1) }",
            "newline",
        ),
    ] {
        fs::write(&file, source).unwrap();
        let result = invoke("check");
        assert!(!result.status.success(), "accepted {source}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(error),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    fs::write(&file, "fn main() {\nlet mut n = 0\nwhile n < 5 {\nn = n + 1\nif n == 2 { continue }\nif n == 4 { break }\nprintln(n)\n}\n}\n").unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).ends_with("1\n3\n"));
    fs::write(&file, "fn main() {\nlet mut i = 0\nlet mut outer = \"outer\"\nwhile i < 3 {\nlet inner = \"inner\"\ni = i + 1\nif i == 1 { continue }\nprintln(inner)\nif i == 2 { break }\n}\nprintln(outer)\n}\n").unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).ends_with("inner\nouter\n"));
    fs::write(&file, include_str!("../../examples/control/jumps.zm")).unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).ends_with("2\n3\n1\n2\n2\n2\n3\n3\n"));
    for source in [
        "fn main() { break }",
        "fn main() { continue }",
        "fn main() { let x = \"x\" while true { let y = x break println(y) } }",
        "fn main() { while true { continue println(1) } }",
    ] {
        fs::write(&file, source).unwrap();
        assert!(!invoke("check").status.success(), "accepted {source}");
    }
    fs::remove_dir_all(root).unwrap();
}
