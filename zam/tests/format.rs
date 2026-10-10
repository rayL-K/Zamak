use std::{fs, process::Command};

#[test]
fn string_interpolation_reads_without_moving() {
    let path = std::env::temp_dir().join(format!("zam-format-{}.zm", std::process::id()));
    let invoke = |command| {
        Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg(command)
            .arg(&path)
            .output()
            .unwrap()
    };
    fs::write(&path, include_str!("../../examples/values/format.zm")).unwrap();
    let output = invoke("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout)
        .ends_with("你好，Zamak！x=7 true 原点\n字面大括号 {x} 与 {}\nZamak\nZamak\nZamak\n"));
    for source in [
        "fn main() { println(\"{1}\") }",
        "fn main() { let n = 1\nprintln(\"{n.y}\") }",
        "fn main() { println(\"a={name}\") }",
        "fn main() { println(\"{name\") }",
        "fn main() { println(\"a}b\") }",
        "fn main() { println(\"{a{b}}\") }",
        "use std/fmt\nfn main() { let s = \"x\"\nlet t = s\nfmt.println(\"{s}\") }",
        "struct P { x: i64 }\nfn main() { let p = P { x: 1 }\nprintln(\"{p}\") }",
        "fn main() { let s = \"x\"\nlet n: i64 = \"v={s}\" }",
        "use std/fmt\nfn main() { let s = \"x\"\nfmt.println(\"{&s}\") }",
    ] {
        fs::write(&path, source).unwrap();
        assert!(!invoke("check").status.success(), "accepted {source}");
    }
    fs::remove_file(path).unwrap();
}
