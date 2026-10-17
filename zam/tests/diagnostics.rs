use std::{fs, process::Command};

#[test]
fn syntax_diagnostics_include_source_and_line() {
    let path = std::env::temp_dir().join(format!("zam-diagnostics-{}.zm", std::process::id()));
    for (source, line, column, message) in [
        (
            "# comment\nfn main():\n    println(`)\n",
            3,
            13,
            "unexpected character",
        ),
        (
            "fn main():\n    println(\"bad\\q\")\n",
            2,
            13,
            "unsupported string escape",
        ),
        (
            "fn main():\n    println(\"open",
            2,
            13,
            "unterminated string literal",
        ),
        (
            "fn main():\n    println(\"你好\")\n    println(1\n",
            3,
            12,
            "'(' was never closed",
        ),
        (
            "fn main():\n",
            2,
            1,
            "expected an indented block after the fn line",
        ),
        (
            "fn main(): println(\"你好\") `",
            1,
            26,
            "unexpected character",
        ),
        (
            "fn main():\r\n\tprintln(`)\r\n",
            2,
            1,
            "indentation contains a tab; run `zam fmt`",
        ),
        (
            "fn main():\n    let x = 1\n      println(x)\n",
            3,
            7,
            "unexpected indent",
        ),
        (
            "fn main():\n    if true:\n        println(1)\n  println(2)\n",
            4,
            3,
            "unindent does not match any outer indentation level",
        ),
        (
            "fn main():\nlet x = 1\nif x > 0:\nprintln(x)\n",
            2,
            1,
            "the indentation may have been lost when the code was copied",
        ),
        (
            "fn main() {\n    println(1)\n}\n",
            1,
            11,
            "blocks are indented, not wrapped in braces",
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
        // 人类可读格式：`error[ZMxxx]: 信息`，下一行 `--> 文件:行:列`，有源码行时带下划线。
        assert!(error.starts_with("error[ZM1"), "{error}");
        assert!(
            error.contains(&format!(
                "--> {}:{line}:{column}
",
                path.display()
            )),
            "{error}"
        );
        assert!(error.contains(message), "{error}");
        if source.lines().nth(line - 1).is_some() {
            assert!(error.contains(" | ") && error.contains('^'), "{error}");
        }
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn semantic_diagnostics_include_line_and_column() {
    let path = std::env::temp_dir().join(format!("zam-semantic-{}.zm", std::process::id()));
    for (source, line, column, message) in [
        (
            "fn main():\n    let a = \"x\"\n    let b = a\n    println(a)\n",
            4,
            5,
            "use after move",
        ),
        (
            "fn main():\n    let a = \"x\"\n    if true:\n        let b = a\n        println(a)\n",
            5,
            9,
            "use after move",
        ),
        (
            "fn f() -> string: pass\nfn main(): pass\n",
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
            error.contains(&format!(
                "{}:{line}:{column}
",
                path.display()
            )),
            "{source}: {error}"
        );
        let text = source.lines().nth(line - 1).unwrap();
        assert!(
            error.contains(&format!(
                "{line} | {text}
"
            )),
            "{source}: {error}"
        );
        assert!(error.contains(message), "{source}: {error}");
    }
    fs::remove_file(path).unwrap();
}

/// 类型检查逐个函数进行：每个出错的函数各报一条，人类可读输出与 JSON 都按位置排序。
#[test]
fn every_failing_function_reports_its_first_error() {
    let path = std::env::temp_dir().join(format!("zam-multi-{}.zm", std::process::id()));
    fs::write(
        &path,
        "fn later():\n    let x: i64 = \"a\"\nfn main():\n    let a = \"x\"\n    let b = a\n    println(a)\n    println(a)\nfn fine(): pass\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("check")
        .arg(&path)
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    let first = error
        .find(
            ":2:5
",
        )
        .expect(&error);
    let second = error
        .find(
            ":6:5
",
        )
        .expect(&error);
    assert!(first < second, "{error}");
    assert!(
        !error.contains(":7:5"),
        "only the first error per function: {error}"
    );
    assert!(
        error.contains(
            "    println(a)
  |     ^^^^^^^^^^
"
        ),
        "{error}"
    );
    assert!(error.trim_end().ends_with("zam: 2 errors"), "{error}");
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["check", "--json"])
        .arg(&path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let codes: Vec<_> = stdout
        .match_indices("\"code\":\"")
        .map(|(at, _)| &stdout[at + 8..at + 13])
        .collect();
    assert_eq!(codes, ["ZM200", "ZM301"], "{stdout}");
    assert!(stdout.find("\"line\":2,").unwrap() < stdout.find("\"line\":6,").unwrap());
    fs::remove_file(path).unwrap();
}

#[test]
fn json_diagnostics_are_one_line_and_structured() {
    let path = std::env::temp_dir().join(format!("zam-json-{}.zm", std::process::id()));
    let escaped = path.display().to_string().replace('\\', "\\\\");
    fs::write(&path, "fn main():\n    println(`)\n").unwrap();
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
        "\"column\":13",
        "\"function\":null",
        "\"code\":\"ZM100\"",
        "\"category\":\"syntax\"",
        "\"message\":\"unexpected character '`'\"",
        "\"suggestion\":\"remove or replace the unsupported character\"",
    ] {
        assert!(stdout.contains(field), "{field} missing from {stdout}");
    }
    // 人类可读的那一行仍然在 stderr，stdout 保持是合法 JSON。
    assert!(String::from_utf8_lossy(&output.stderr).contains(&path.display().to_string()));

    fs::write(&path, "fn main():\n    println(1)\n").unwrap();
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

    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["check", "--json", "--json"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("may be specified once"));
    fs::remove_file(path).unwrap();
}

/// 每段都是真实源码：必须被 `check` 拒绝，并归到表里那一个诊断码。
/// 只断言单元测试里造出来的字符串不够——`classify` 匹配的是前端真实文案。
/// 这张表同时是码表契约的底稿：`documented_codes_are_all_covered_by_real_sources`
/// 要求它与 `docs/AI_WORKFLOW.md` 列出的码逐个对应。
const REAL_SOURCE_CODES: &[(&str, &str, &str)] = &[
    (
        "fn main():\n    let x: i64 = \"s\"\n",
        "ZM200",
        "type",
    ),
    ("fn main():\n    println(nope)\n", "ZM201", "name"),
    ("fn main():\n    nope()\n", "ZM201", "name"),
    (
        "struct Point:\n    x: i64\n    y: i64\nfn main():\n    let p = Point(\n        x: 1,\n        y: 2,\n    )\n    println(p.z)\n",
        "ZM202",
        "name",
    ),
    (
        "fn main():\n    let a = \"x\"\n    let b = a\n    println(a)\n",
        "ZM301",
        "ownership",
    ),
    (
        "fn main():\n    let x = 1\n    x = 2\n",
        "ZM302",
        "ownership",
    ),
    (
        "fn f(a: &string, b: string) -> i64:\n    return 1\nfn main():\n    let s = \"x\"\n    f(&s, s)\n",
        "ZM421",
        "ownership",
    ),
    (
        "fn main():\n    let s = \"x\"\n    let r: &string = &s\n",
        "ZM422",
        "ownership",
    ),
    (
        "fn f(a: &mut string) -> i64:\n    return 1\nfn main():\n    let mut s = \"x\"\n    f(&s)\n",
        "ZM423",
        "ownership",
    ),
    (
        "fn f(a: &mut string) -> i64:\n    return 1\nfn main():\n    let s = \"x\"\n    f(&mut s)\n",
        "ZM424",
        "ownership",
    ),
    (
        "fn f(a: &mut string) -> i64:\n    return 1\nfn main():\n    let s = \"x\"\n    f(s)\n",
        "ZM425",
        "ownership",
    ),
    (
        "fn f() -> string: pass\nfn main(): pass\n",
        "ZM510",
        "control_flow",
    ),
    (
        "enum Light:\n    Red\n    Green\nfn main():\n    let l = Light.Red\n    match l:\n        Red: println(1)\n",
        "ZM511",
        "control_flow",
    ),
    (
        "enum Light:\n    Red\nfn main():\n    let l = Light.Blue\n",
        "ZM201",
        "name",
    ),
    (
        "fn main():\n    let m: {bool: i64} = {}\n",
        "ZM900",
        "unsupported",
    ),
    (
        "fn main():\n    println(\"a\\qb\")\n",
        "ZM101",
        "syntax",
    ),
    (
        "fn main():\n    let x = 9223372036854775808\n    println(x)\n",
        "ZM600",
        "arithmetic",
    ),
    (
        "fn main():\n    let a = [1, 2, 3]\n    println(a[5])\n",
        "ZM406",
        "bounds",
    ),
    (
        "fn load(p: &string) -> i64!string:\n    return 1\nfn main():\n    let v = load(\"a\")\n    println(v)\n",
        "ZM500",
        "error_handling",
    ),
    (
        "fn f() -> i64!i64:\n    return 1\nfn main():\n    println(1)\n",
        "ZM509",
        "error_handling",
    ),
    (
        "fn main():\n    println(\"hi\");\n",
        "ZM110",
        "syntax",
    ),
    (
        "fn foo() -> i64:\n    return 1\n\nfn foo() -> i64:\n    return 2\n\nfn main():\n    println(foo())\n",
        "ZM204",
        "name",
    ),
    ("fn main():\n    println(`)\n", "ZM100", "syntax"),
    (
        "fn main():\n    let point = shapes.Point(x: 1)\n",
        "ZM203",
        "name",
    ),
    (
        "fn main():\n    let s = \"hi\"\n    println(string.byte_len(&s))\n",
        "ZM203",
        "name",
    ),
];

#[test]
fn json_codes_classify_real_sources() {
    let path = std::env::temp_dir().join(format!("zam-classify-{}.zm", std::process::id()));
    for &(source, code, category) in REAL_SOURCE_CODES {
        fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .args(["check", "--json"])
            .arg(&path)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(!output.status.success(), "accepted {source}");
        assert_eq!(stdout.lines().count(), 1, "{source}: {stdout}");
        for field in [
            format!("\"code\":\"{code}\""),
            format!("\"category\":\"{category}\""),
            "\"suggestion\":\"".to_string(),
        ] {
            assert!(
                stdout.contains(&field),
                "{source}: {field} missing from {stdout}"
            );
        }
        assert!(
            !stdout.contains("\"suggestion\":null"),
            "{source}: missing suggestion in {stdout}"
        );
    }
    fs::remove_file(path).unwrap();
}

/// `docs/AI_WORKFLOW.md` 的码表是给 AI 工具看的对外契约：列出的每个码都要有真实源码
/// 断言，反过来也不许断言了却没写进文档的码。任一侧单独更新，AI 循环就会照过期契约走。
#[test]
fn documented_codes_are_all_covered_by_real_sources() {
    // 兜底分类没有稳定的可复现源文；文档本身要求 AI 保留完整 message 并提交最小复现。
    const WITHOUT_REPRODUCIBLE_SOURCE: &[&str] = &["ZM999"];
    let mut documented: Vec<&str> = include_str!("../../docs/AI_WORKFLOW.md")
        .lines()
        .filter_map(|line| line.strip_prefix("- `"))
        .filter_map(|rest| rest.split('`').next())
        .filter(|code| !WITHOUT_REPRODUCIBLE_SOURCE.contains(code))
        .collect();
    // 同一个码可以有多段源码（ZM201 就分了变量与函数两种），所以比的是去重后的集合。
    let mut asserted: Vec<&str> = REAL_SOURCE_CODES.iter().map(|(_, code, _)| *code).collect();
    documented.sort_unstable();
    documented.dedup();
    asserted.sort_unstable();
    asserted.dedup();
    let only_documented: Vec<&str> = documented
        .iter()
        .copied()
        .filter(|code| !asserted.contains(code))
        .collect();
    let only_asserted: Vec<&str> = asserted
        .iter()
        .copied()
        .filter(|code| !documented.contains(code))
        .collect();
    assert!(
        only_documented.is_empty() && only_asserted.is_empty(),
        "文档码表与真实源码断言不一致：只有文档 {only_documented:?}，只有断言 {only_asserted:?}"
    );
}

#[test]
fn build_json_keeps_progress_off_stdout() {
    let root = std::env::temp_dir().join(format!("zam-json-build-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("main.zm");
    fs::write(&path, "fn main():\n    println(1)\n").unwrap();
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

/// 模块解析期的错误（例如缺少 `use std/string`）过去只有 message，`file`/`line`/`column`
/// 全为 null，AI 修复循环只能靠正则抠模块名。位置取自 `Block` 里已有的指令位置，粒度与
/// 类型检查器一致（指向语句起始）；项目模式下 `file` 也从项目目录收紧到出错的那个模块。
#[test]
fn module_resolution_errors_include_location() {
    let source = "fn main():\n    let s = \"hi\"\n    println(string.byte_len(&s))\n";
    let root = std::env::temp_dir().join(format!("zam-module-location-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("main.zm");
    let module = root.join("src").join("main.zm");
    fs::write(&file, source).unwrap();
    fs::write(&module, source).unwrap();
    fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"probe\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();

    for (input, reported) in [(&file, &file), (&root, &module)] {
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .args(["check", "--json"])
            .arg(input)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(!output.status.success(), "{stdout}");
        assert_eq!(stdout.lines().count(), 1, "{stdout}");
        let escaped = reported.display().to_string().replace('\\', "\\\\");
        for field in [
            format!("\"file\":\"{escaped}\""),
            "\"line\":3".to_string(),
            "\"column\":5".to_string(),
            "\"code\":\"ZM203\"".to_string(),
            "\"category\":\"name\"".to_string(),
        ] {
            assert!(stdout.contains(&field), "{field} missing from {stdout}");
        }
    }
    fs::remove_dir_all(root).unwrap();
}

/// 直接检查单个 `.zm` 文件时只有一个模块，报错里 `<模块>:` 前缀指的还是这个文件本身，读起来是噪音。
/// 单文件去掉它，项目里保留（项目有多个模块，前缀才说明出错的是哪一个）。
#[test]
fn single_file_diagnostics_omit_the_module_prefix() {
    let root = std::env::temp_dir().join(format!("zam-single-prefix-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("main.zm");
    fs::write(&file, "fn main():\n    nope()\n").unwrap();

    for (lang, expected) in [("en", "unknown function nope"), ("zh", "未知的函数 nope")] {
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .args(["check", "--json"])
            .arg(&file)
            .env("ZAMAK_LANG", lang)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(!output.status.success(), "{stdout}");
        // --json 始终是英文原文，中文只在终端渲染时经 zh::text() 翻译。
        assert!(stdout.contains("unknown function nope"), "{stdout}");
        assert!(!stdout.contains("main: unknown function nope"), "{stdout}");
        let rendered = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("check")
            .arg(&file)
            .env("ZAMAK_LANG", lang)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&rendered.stderr).into_owned();
        assert!(text.contains(expected), "{text}");
        assert!(!text.contains("main："), "{text}");
        assert!(!text.contains("main: "), "{text}");
    }

    // 项目里保留模块名：它指出出错的是哪个模块。
    fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"probe\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    fs::write(root.join("src").join("main.zm"), "fn main():\n    nope()\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["check", "--json"])
        .arg(&root)
        .env("ZAMAK_LANG", "en")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(stdout.contains("main: unknown function nope"), "{stdout}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ambiguous_imports_are_classified_as_name_diagnostics() {
    let root = std::env::temp_dir().join(format!("zam-ambiguous-import-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"probe\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src").join("main.zm"),
        "use one/foo\nuse two/foo\n\nfn main():\n    println(foo.hello())\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("src").join("one")).unwrap();
    fs::create_dir_all(root.join("src").join("two")).unwrap();
    fs::write(
        root.join("src").join("one").join("foo.zm"),
        "pub fn hello() -> i64:\n    return 1\n",
    )
    .unwrap();
    fs::write(
        root.join("src").join("two").join("foo.zm"),
        "pub fn hello() -> i64:\n    return 2\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["check", "--json"])
        .arg(&root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success(), "{stdout}");
    assert!(stdout.contains("\"code\":\"ZM203\""), "{stdout}");
    assert!(stdout.contains("\"category\":\"name\""), "{stdout}");
    assert!(
        stdout.contains("check the matching `use` import"),
        "{stdout}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catch_resolution_errors_keep_inner_location() {
    let root = std::env::temp_dir().join(format!("zam-catch-location-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"probe\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    let file = root.join("main.zm");
    let module = root.join("src").join("main.zm");
    for (handler, line, column) in [
        ("println(missing.call())\n        return\n", 5, 9),
        (
            "let inner = load() catch nested:\n            println(missing.call())\n            return\n        return\n",
            6,
            13,
        ),
    ] {
        let source = format!(
            "fn load() -> i64!string:\n    fail \"nope\"\nfn main():\n    let value = load() catch e:\n        {handler}\n    println(value)\n"
        );
        fs::write(&file, &source).unwrap();
        fs::write(&module, &source).unwrap();
        for (input, reported, prefix) in [(&file, &file, ""), (&root, &module, "main: ")] {
            let output = Command::new(env!("CARGO_BIN_EXE_zam"))
                .args(["check", "--json"])
                .arg(input)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(!output.status.success(), "{stdout}");
            assert_eq!(stdout.lines().count(), 1, "{stdout}");
            let escaped = reported.display().to_string().replace('\\', "\\\\");
            for field in [
                format!("\"file\":\"{escaped}\""),
                format!("\"line\":{line},"),
                format!("\"column\":{column},"),
                "\"code\":\"ZM203\"".to_string(),
                format!(
                    "\"message\":\"{prefix}ambiguous or missing import for missing.call\""
                ),
            ] {
                assert!(stdout.contains(&field), "{field} missing from {stdout}");
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

/// 类型检查期的错误由 `compiler::check` 产生，message 里带的是链接后的 `模块:函数`、不含
/// 文件路径；项目模式下 `main.rs` 因此曾把 `file` 退化成项目目录。这里断言它补回了真正
/// 出错模块的路径，与解析期错误保持一致。
#[test]
fn project_check_errors_report_module_path() {
    let root = std::env::temp_dir().join(format!("zam-check-location-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"probe\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src").join("main.zm"),
        "use greeting\n\nfn main():\n    println(greeting.hello())\n",
    )
    .unwrap();
    let module = root.join("src").join("greeting.zm");
    fs::write(&module, "pub fn hello() -> string:\n    return 1\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["check", "--json"])
        .arg(&root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(!output.status.success(), "{stdout}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let escaped = module.display().to_string().replace('\\', "\\\\");
    for field in [
        format!("\"file\":\"{escaped}\""),
        "\"line\":2".to_string(),
        "\"column\":5".to_string(),
        "\"function\":\"greeting:hello\"".to_string(),
        "\"code\":\"ZM200\"".to_string(),
        "\"category\":\"type\"".to_string(),
    ] {
        assert!(stdout.contains(&field), "{field} missing from {stdout}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn python_habits_get_the_zamak_spelling() {
    // 常见的 Python 写法：报错末尾给出 Zamak 的写法，记号按源码样子显示。
    let path = std::env::temp_dir().join(format!("zam-python-{}.zm", std::process::id()));
    for (line, hint) in [
        ("xs.append(3)", "use .push(v)"),
        ("let t = s + n", "interpolate them"),
    ] {
        let source = format!(
            "fn main():\n    let mut xs: [i64] = [1, 2]\n    let s = \"abc\"\n    let n = 7\n    {line}\n"
        );
        fs::write(&path, &source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("check")
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted {line}");
        assert!(stderr.contains(hint), "{line}: {stderr}");
        assert!(!stderr.contains("Some("), "{line}: {stderr}");
    }
    let _ = fs::remove_file(path);
}
