use std::{fs, process::Command};

#[test]
fn syntax_diagnostics_include_source_and_line() {
    let path = std::env::temp_dir().join(format!("zam-diagnostics-{}.zm", std::process::id()));
    for (source, line, column, message) in [
        (
            "# comment\nfn main() {\nprintln(@)\n}\n",
            3,
            9,
            "unexpected character",
        ),
        (
            "fn main() {\nprintln(\"bad\\q\")\n}\n",
            2,
            9,
            "unsupported string escape",
        ),
        (
            "fn main() {\nprintln(\"open",
            2,
            9,
            "unterminated string literal",
        ),
        (
            "fn main() {\nprintln(\"你好\")\nprintln(1\n}\n",
            3,
            10,
            "expected",
        ),
        ("fn main() {\nprintln(1)\n", 3, 1, "unterminated"),
        (
            "fn main() { println(\"你好\") @ }",
            1,
            27,
            "unexpected character",
        ),
        (
            "fn main() {\r\n\tprintln(@)\r\n}",
            2,
            10,
            "unexpected character",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("check")
            .arg(&path)
            .output()
            .unwrap();
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success());
        assert!(error.contains(&path.display().to_string()), "{error}");
        assert!(
            error.contains(&format!("line {line}, column {column}:")),
            "{error}"
        );
        assert!(error.contains(message), "{error}");
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn semantic_diagnostics_include_line_and_column() {
    let path = std::env::temp_dir().join(format!("zam-semantic-{}.zm", std::process::id()));
    for (source, line, column, message) in [
        (
            "fn main() {\n    let a = \"x\"\n    let b = a\n    println(a)\n}\n",
            4,
            5,
            "use after move",
        ),
        (
            "fn main() {\n    let a = \"x\"\n    if true {\n        let b = a\n        println(a)\n    }\n}\n",
            5,
            9,
            "use after move",
        ),
        (
            "fn f() -> string {}\nfn main() {}\n",
            1,
            1,
            "missing string return value",
        ),
    ] {
        fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("check")
            .arg(&path)
            .output()
            .unwrap();
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted {source}");
        assert!(
            error.contains(&format!("line {line}, column {column}:")),
            "{source}: {error}"
        );
        assert!(error.contains(message), "{source}: {error}");
    }
    fs::remove_file(path).unwrap();
}
