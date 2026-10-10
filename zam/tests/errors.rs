use std::{fs, process::Command};

#[test]
fn error_values() {
    let path = std::env::temp_dir().join(format!("zam-errors-{}.zm", std::process::id()));
    let invoke = |command| {
        Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg(command)
            .arg(&path)
            .output()
            .unwrap()
    };
    let example = include_str!("../../examples/values/errors.zm");
    let (declarations, main) = example.split_once("fn main()").unwrap();
    let success = format!("{declarations}fn main(){main}");
    // 失败版：把入口的文本换成空串，让错误穿过两层 `?` 到 main。
    let failure = success.replace("\"42\"", "\"\"");
    // 成功：`?` 把值一路带出来，main 正常退出。
    fs::write(&path, &success).unwrap();
    let output = invoke("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("42\n"));
    // 失败：未捕获的错误写 stderr 并以 1 退出。
    fs::write(&path, &failure).unwrap();
    let output = invoke("run");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("empty input"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("42"));
    // `?` 与 `fail` 的规则，以及成功值类型仍然是 T。
    for (source, because) in [
        (
            format!("{declarations}fn main() {{\nlet text = \"42\"\nlet value = parse(&text)\nprintln(value)\n}}"),
            "fallible call without ?",
        ),
        (
            format!("{declarations}fn main() {{\nlet text = \"42\"\nlet value = parse(&text)?\nprintln(value)\n}}"),
            "? in a function without an error type",
        ),
        (
            format!("{declarations}fn main() -> !string {{\nlet text = \"42\"\nlet flag: bool = parse(&text)?\nprintln(flag)\n}}"),
            "success value is still i64",
        ),
        (
            "fn double(n: i64) -> i64 {\nreturn n\n}\nfn main() -> !string {\nlet value = double(1)?\nprintln(value)\n}"
                .to_string(),
            "? on a call that cannot fail",
        ),
        (
            "fn main() {\nfail \"nope\"\n}".to_string(),
            "fail in a function without an error type",
        ),
        (
            "fn f() -> i64!i64 {\nfail 1\n}\nfn main() -> !i64 {\nlet value = f()?\nprintln(value)\n}"
                .to_string(),
            "non-string error type",
        ),
    ] {
        fs::write(&path, source).unwrap();
        assert!(!invoke("check").status.success(), "accepted {because}");
    }
    fs::remove_file(path).unwrap();
}
