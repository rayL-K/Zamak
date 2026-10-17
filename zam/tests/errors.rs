mod common;

use std::{fs, process::Command};

/// 生成程序的运行时报错也要跟 ZAMAK_LANG：`zam: list index out of bounds` 与中文版一一对应。
/// C 运行时只读一次环境变量，所以这里按语言各起一次进程。
#[test]
fn runtime_panics_follow_the_language() {
    let root = std::env::temp_dir().join(format!("zam-panic-lang-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let cache = root.join("cache");
    for (name, source, en, zh) in [
        (
            "index",
            "fn main():\n    let xs: [i64] = [1, 2]\n    println(xs[5])\n",
            "zam: list index out of bounds",
            "zam: 列表下标越界",
        ),
        (
            "zero",
            "fn main():\n    let zero = 0\n    println(10 // zero)\n",
            "zam: division by zero",
            "zam: 除以零",
        ),
        (
            "key",
            "fn main():\n    let mut m: {string: i64} = {\"a\": 1}\n    println(m[\"b\"])\n",
            "zam: key not found",
            "zam: 找不到这个键",
        ),
        (
            "overflow",
            "fn main():\n    let big = 9223372036854775807\n    println(big + 1)\n",
            "zam: integer arithmetic error",
            "zam: 整数运算出错",
        ),
    ] {
        let path = root.join(format!("{name}.zm"));
        fs::write(&path, source).unwrap();
        for (lang, expected) in [("en", en), ("zh", zh)] {
            let output = Command::new(env!("CARGO_BIN_EXE_zam"))
                .arg("run")
                .arg(&path)
                .env("ZAMAK_CACHE_DIR", &cache)
                .env("ZAMAK_LANG", lang)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(1), "{name}/{lang}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains(expected),
                "{name}/{lang}: expected {expected:?} in\n{stderr}"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}

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
    // 注意 `zam run` 把缓存进度（含哈希）也打在 stdout，这里只按行比对程序自己的输出。
    assert!(!String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim() == "42"));
    // `?` 与 `fail` 的规则，以及成功值类型仍然是 T。
    for (source, because) in [
        (
            format!("{declarations}fn main():\n    let text = \"42\"\n    let value = parse(&text)\n    println(value)"),
            "fallible call without ?",
        ),
        (
            format!("{declarations}fn main():\n    let text = \"42\"\n    let value = parse(&text)?\n    println(value)"),
            "? in a function without an error type",
        ),
        (
            format!("{declarations}fn main() -> !string:\n    let text = \"42\"\n    let flag: bool = parse(&text)?\n    println(flag)"),
            "success value is still i64",
        ),
        (
            "fn double(n: i64) -> i64:\n    return n\nfn main() -> !string:\n    let value = double(1)?\n    println(value)"
                .to_string(),
            "? on a call that cannot fail",
        ),
        (
            "fn main():\n    fail \"nope\"".to_string(),
            "fail in a function without an error type",
        ),
        (
            "fn f() -> i64!i64:\n    fail 1\nfn main() -> !i64:\n    let value = f()?\n    println(value)"
                .to_string(),
            "non-string error type",
        ),
    ] {
        fs::write(&path, source).unwrap();
        assert!(!invoke("check").status.success(), "accepted {because}");
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn catch_blocks() {
    let path = std::env::temp_dir().join(format!("zam-catch-{}.zm", std::process::id()));
    let invoke = |command| {
        Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg(command)
            .arg(&path)
            .output()
            .unwrap()
    };
    let example = include_str!("../../examples/values/catch.zm");
    let declarations = example.split_once("fn main()").unwrap().0;
    // 成功：catch 不触发，值照常带出来。
    fs::write(&path, example).unwrap();
    let output = invoke("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("42\n"));
    // 失败：错误值绑定到 e，打印后从 main 返回，退出码仍是 0（错误已被处理）。
    let failure = example.replace("\"42\"", "\"\"");
    fs::write(&path, &failure).unwrap();
    let output = invoke("run");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("failed: empty input"));
    assert!(!String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim() == "42"));
    // catch 的规则。
    for (source, because) in [
        (
            format!(
                "{declarations}fn main():\n    let text = \"42\"\n    let value = parse(&text) catch e:\n        println(\"failed: {{e}}\")\n    println(value)"
            ),
            "catch block without return",
        ),
        (
            "fn double(n: i64) -> i64:\n    return n\nfn main():\n    let value = double(1) catch e:\n        println(\"nope\")\n        return\n    println(value)"
                .to_string(),
            "catch on a call that cannot fail",
        ),
        (
            format!(
                "{declarations}fn main():\n    let e = \"taken\"\n    let text = \"42\"\n    let value = parse(&text) catch e:\n        println(\"failed: {{e}}\")\n        return\n    println(value)"
            ),
            "catch binding shadows a live local",
        ),
    ] {
        fs::write(&path, source).unwrap();
        assert!(!invoke("check").status.success(), "accepted {because}");
    }
    fs::remove_file(path).unwrap();
}

/// 错误类型可以是枚举或结构体：fail 构造、catch 里 match、? 只在同一种错误之间传递，
/// main 未捕获时按 println 的格式写到 stderr。
#[test]
fn custom_error_types() {
    let dir = std::env::temp_dir().join(format!("zam-errors-custom-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let run = |name: &str, source: &str| {
        let file = dir.join(name);
        std::fs::write(&file, source).unwrap();
        std::process::Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("run")
            .arg(&file)
            .env("ZAMAK_CACHE_DIR", dir.join("cache"))
            .output()
            .unwrap()
    };
    let output = run(
        "config.zm",
        include_str!("../../examples/values/custom_errors.zm"),
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        "port 8080, 4 workers\nmissing workers\nport is not a number: eighty\nport out of range: 99999\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr)
        .replace("\r\n", "\n")
        .ends_with("\nConfigError.Missing(\"port\")\n"));
    let output = run(
        "struct.zm",
        "struct Fault:\n    code: i64\n    text: string\nfn risky(n: i64) -> i64!Fault:\n    if n > 2: fail Fault(code: n, text: \"too big\")\n    n * 10\nfn main():\n    for i in 1..5:\n        let v = risky(i) catch e:\n            println(\"{e.code}: {e.text}\")\n            break\n        println(v)\n",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        "10\n20\n3: too big\n"
    );
    for (source, message) in [
        (
            "enum E:\n    Bad\nuse std/fs\nfn f() -> !E:\n    let t = fs.read(\"x\")?\nfn main(): pass\n",
            "? cannot propagate the error of std/fs:read",
        ),
        (
            "enum E:\n    Bad\nfn f() -> !E:\n    fail \"text\"\nfn main(): pass\n",
            "mismatch",
        ),
        ("fn f() -> i64!i64: 1\nfn main(): pass\n", "unsupported error type"),
        (
            "enum E:\n    Bad\nfn f() -> !E: fail E.Bad\nfn g() -> !string:\n    f()?\nfn main(): pass\n",
            "? cannot propagate the error of",
        ),
    ] {
        let output = run("bad.zm", source);
        assert!(!output.status.success(), "accepted {source}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(message), "{source}: {stderr}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn catch_with_a_single_value_is_the_fallback() {
    // `f() catch e: value`：块只有一个表达式时，它就是出错时的值（与 Zig 的 catch 相同）。
    // 多条语句的 catch 块照旧：值要用时必须用 return、fail、break 或 continue 离开，块尾的值不算。
    let case = common::Case::new("errors", "catch-fallback");
    let output = case.run("use std/env\n\nfn parse(text: &string) -> i64!string:\n    int(text)?\n\nfn label(n: i64) -> string!string:\n    if n < 0: fail \"negative\"\n    f\"n={n}\"\n\nfn total(items: &[string]) -> i64:\n    let mut sum = 0\n    for item in items:\n        sum += parse(item) catch e: 0\n    sum\n\nfn main():\n    let home = env.var(\"ZAMAK_SURELY_UNSET\") catch e: \"unknown\"\n    println(home)\n    println(parse(\"42\") catch e: -1)\n    println(parse(\"x\") catch e: -1)\n    println(float(\"oops\") catch e: 1)\n    println(label(-3) catch e: e)\n    println(total([\"1\", \"two\", \"3\"]))\n");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        "unknown\n42\n-1\n1.0\nnegative\n4\n"
    );
    let rejected = case.run("fn parse(text: &string) -> i64!string:\n    int(text)?\n\nfn main():\n    let n = parse(\"x\") catch e:\n        println(e)\n        0\n    println(n)\n");
    assert!(!rejected.status.success());
}
