use std::{fs, process::Command};

/// `zam --help` 跟随 `ZAMAK_LANG`：两种语言都列出同样的子命令，并且每条都有一句说明
/// （以前只有一行行命令名，看不出每个命令是做什么的）。
#[test]
fn help_lists_every_command_with_a_description_in_both_languages() {
    let help = |lang: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("--help")
            .env("ZAMAK_LANG", lang)
            .output()
            .unwrap();
        assert!(output.status.success(), "{lang}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let english = help("en");
    let chinese = help("zh");
    assert!(english.starts_with("zam "), "{english}");
    assert!(chinese.starts_with("zam "), "{chinese}");
    for command in [
        "check",
        "build",
        "run",
        "test",
        "fmt",
        "new",
        "add",
        "remove",
        "update",
        "search",
        "publish",
        "lsp",
        "mcp",
        "upgrade",
        "demo",
        "--version",
    ] {
        // 每个命令一行，并且同一行上有说明（说明与命令之间至少两个空格隔开）。
        for (lang, text) in [("en", &english), ("zh", &chinese)] {
            let line = text
                .lines()
                .find(|line| line.trim_start().starts_with(&format!("zam {command}")))
                .unwrap_or_else(|| panic!("{lang}: missing {command} in\n{text}"));
            let rest = line
                .trim_start()
                .strip_prefix(&format!("zam {command}"))
                .unwrap();
            let description = rest.split("  ").map(str::trim).find(|s| !s.is_empty());
            assert!(
                description.is_some_and(|d| d.len() > 4),
                "{lang}: `zam {command}` has no description: {line:?}"
            );
        }
    }
    // 语言确实换了：英文里不该出现“只做类型与所有权检查”，中文里不该出现英文说明。
    assert!(english.contains("type-check without building"), "{english}");
    assert!(chinese.contains("只做类型与所有权检查"), "{chinese}");
    assert!(
        !chinese.contains("type-check without building"),
        "{chinese}"
    );
}

#[test]
fn invalid_arguments_fail_before_loading_or_writing_projects() {
    for args in [
        vec!["check", "--unknown"],
        vec!["build", "--unknown"],
        vec!["fmt", "--unknown"],
        vec!["run", "--unknown"],
        vec!["new", "--unknown"],
        vec!["check", ".", "extra"],
        vec!["new", "demo", "--template", "other"],
        vec!["--version", "extra"],
        vec!["--json", "check"],
        vec!["check", "--json", "--json"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("usage:"), "{args:?}: {error}");
    }
}

/// `zam fmt` 是 AI 工作循环的第三步（见 `zam new --template ai` 生成的 AI.md），
/// 所以这里锁住它的两条承重契约：只写真正变了的模块，语法不成立时一个文件都不写。
#[test]
fn fmt_rewrites_only_the_modules_whose_indentation_changed() {
    let root = std::env::temp_dir().join(format!("zam-fmt-clean-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"fmt\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    // main.zm 缩进已经正确，greeting.zm 缩进是错的。
    let clean = "use greeting\n\nfn main():\n    greeting.hello()\n";
    let messy = "pub fn hello() -> string:\n  \"hi\"\n";
    fs::write(src.join("main.zm"), clean).unwrap();
    fs::write(src.join("greeting.zm"), messy).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("fmt")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // 进度行走 stderr、stdout 留空，和 build 的 CACHE MISS/OUTPUT 行一致。
    assert!(
        output.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let progress = String::from_utf8_lossy(&output.stderr);
    assert!(
        progress.contains("FORMAT") && progress.contains("greeting.zm"),
        "{progress}"
    );
    assert!(!progress.contains("main.zm"), "{progress}");
    // 没变的文件原样不动（连 mtime 都不该碰）。
    assert_eq!(fs::read_to_string(src.join("main.zm")).unwrap(), clean);
    assert_eq!(
        fs::read_to_string(src.join("greeting.zm")).unwrap(),
        "pub fn hello() -> string:\n    \"hi\"\n"
    );

    // 幂等：已经格式化过的项目再跑一次不该报任何改动。
    let again = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("fmt")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert!(
        again.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fmt_writes_nothing_when_a_module_does_not_parse() {
    let root = std::env::temp_dir().join(format!("zam-fmt-broken-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"fmt\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    // main.zm 缩进是错的（正常情况下会被改写），greeting.zm 语法不成立。
    let messy = "fn main():\n  println(\"hi\")\n";
    fs::write(src.join("main.zm"), messy).unwrap();
    fs::write(
        src.join("greeting.zm"),
        "fn hello() -> string:\n    \"unterminated\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("fmt")
        .arg(&root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    // 整个项目先解析一遍：只要有模块读不懂，就一个文件都不动。
    assert_eq!(fs::read_to_string(src.join("main.zm")).unwrap(), messy);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fmt_accepts_a_single_file_and_rejects_json() {
    let path = std::env::temp_dir().join(format!("zam-fmt-file-{}.zm", std::process::id()));
    fs::write(&path, "fn main():\n    println(\"hi\")\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("fmt")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "fn main():\n    println(\"hi\")\n"
    );
    fs::remove_file(&path).unwrap();

    // fmt 没有结构化输出；明确拒绝比静默忽略 --json 更容易被 AI 循环发现。
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .args(["fmt", "--json", "."])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--json is only supported by check and build"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
