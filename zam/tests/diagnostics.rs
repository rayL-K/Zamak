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

#[test]
fn json_diagnostics_are_one_line_and_structured() {
    let path = std::env::temp_dir().join(format!("zam-json-{}.zm", std::process::id()));
    let escaped = path.display().to_string().replace('\\', "\\\\");
    fs::write(&path, "fn main() {\nprintln(@)\n}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["check", "--json"])
        .arg(&path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(!output.status.success());
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    for field in [
        "\"command\":\"check\"",
        "\"ok\":false",
        &format!("\"file\":\"{escaped}\""),
        "\"line\":2",
        "\"column\":9",
        "\"function\":null",
        "\"message\":\"unexpected character '@'\"",
    ] {
        assert!(stdout.contains(field), "{field} missing from {stdout}");
    }
    // 人类可读的那一行仍然在 stderr，stdout 保持是合法 JSON。
    assert!(String::from_utf8_lossy(&output.stderr).contains(&path.display().to_string()));

    fs::write(&path, "fn main() {\nprintln(1)\n}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["check", "--json"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "{\"command\":\"check\",\"ok\":true,\"diagnostics\":[]}"
    );

    // 不支持 --json 的命令必须报错，而不是静默忽略。
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["run", "--json"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--json is only supported"));
    fs::remove_file(path).unwrap();
}

#[test]
fn build_json_keeps_progress_off_stdout() {
    let root = std::env::temp_dir().join(format!("zam-json-build-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("main.zm");
    fs::write(&path, "fn main() {\nprintln(1)\n}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["build", "--json"])
        .arg(&path)
        .env("ZAMAK_CACHE_DIR", root.join("cache"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        stdout.trim(),
        "{\"command\":\"build\",\"ok\":true,\"diagnostics\":[]}"
    );
    // CACHE/OBJECT/LINK/OUTPUT 进度挪到了 stderr，stdout 才可能是合法 JSON。
    assert!(String::from_utf8_lossy(&output.stderr).contains("CACHE MISS"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("OUTPUT "));
    fs::remove_dir_all(root).unwrap();
}
