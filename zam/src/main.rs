mod compiler;
mod lsp;
mod mcp;
mod native;
mod package;
mod project;
mod registry;
mod zh;

use std::path::Path;

fn execute() -> project::Result<()> {
    // `zam run <path> -- a b` 把 `--` 之后的参数原样交给程序，不参与 zam 自己的参数解析。
    let mut raw_args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let program_args = match raw_args.iter().position(|arg| arg == "--") {
        Some(index) => {
            let rest = raw_args.split_off(index + 1);
            raw_args.pop();
            Some(rest)
        }
        None => None,
    };
    let raw_args: Vec<String> = raw_args
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if program_args.is_some() && raw_args.first().map(String::as_str) != Some("run") {
        return Err("usage: only zam run accepts program arguments after --".into());
    }
    let json_count = raw_args
        .iter()
        .filter(|arg| arg.as_str() == "--json")
        .count();
    let json = json_count == 1;
    let args: Vec<String> = raw_args
        .iter()
        .filter(|arg| arg.as_str() != "--json")
        .cloned()
        .collect();
    if json_count > 1 {
        return Err("usage: --json may be specified once".into());
    }
    let valid_shape = match args.first().map(String::as_str) {
        Some("check" | "build" | "fmt" | "run") => matches!(args.len(), 1..=2),
        Some("test") => matches!(args.len(), 1..=3),
        Some("update") => matches!(args.len(), 1..=2),
        Some("add") => matches!(args.len(), 2..=4),
        Some("remove") => args.len() == 2,
        Some("search" | "publish") => matches!(args.len(), 1..=2),
        Some("new") => {
            args.len() == 2
                || (args.len() == 4 && args[2] == "--template" && args[3] == "ai")
                || (args.len() == 3 && args[2] == "--ai")
        }
        Some("demo" | "lsp" | "mcp" | "upgrade" | "--version" | "-V" | "--help" | "-h") => {
            args.len() == 1
        }
        None => args.is_empty(),
        _ => false,
    };
    let option_as_path = args.get(1).is_some_and(|arg| arg.starts_with('-'));
    let missing_command = raw_args.first().is_some_and(|arg| arg == "--json");
    if !valid_shape || option_as_path || missing_command {
        return Err(
            "usage: zam <check|build|run|test|fmt|new|add|remove|update|search|publish> [project|file] [--template ai]"
                .into(),
        );
    }
    let input = Path::new(args.get(1).map(String::as_str).unwrap_or("."));
    if json {
        // ponytail: 只有 check/build 会产生诊断；其余命令直接拒绝，避免静默忽略。
        let outcome = match args.first().map(String::as_str) {
            Some("check") => project::Project::load(input).and_then(|project| project.check()),
            Some("build") => project::Project::load(input)
                .and_then(|project| project.build(&project::cache_dir()).map(|_| ())),
            _ => return Err("--json is only supported by check and build".into()),
        };
        println!(
            "{}",
            report(
                args.first().map(String::as_str).unwrap(),
                &input.display().to_string(),
                &outcome
            )
        );
        // 失败时 main 仍会把人类可读的一行写到 stderr，stdout 保持是合法 JSON。
        return outcome;
    }
    match args.first().map(String::as_str) {
        Some("check") => {
            let project = project::Project::load(input)?;
            project.check()?;
            println!("CHECK OK {}", project.name);
        }
        Some("build") => { project::Project::load(input)?.build(&project::cache_dir())?; }
        Some("fmt") => format_sources(input)?,
        // `zam add <名字>`（索引里的最新版本）、`zam add <名字> <版本范围>`、
        // `zam add <名字> <git 地址|本地目录> [rev]`：在当前目录的 zam.toml 里加一个依赖并拉取。
        Some("add") => {
            let name = &args[1];
            let store = project::cache_dir().join("packages");
            let source = match args.get(2) {
                None => package::Source::Registry(registry::Requirement::parse(&format!(
                    "^{}",
                    package::latest(&store, name)?
                ))?),
                Some(location) if args.len() == 3 && requirement_like(location) => {
                    package::Source::Registry(registry::Requirement::parse(location)?)
                }
                Some(location) if Path::new(location).is_dir() => {
                    if args.len() == 4 {
                        return Err("a path dependency takes no rev".into());
                    }
                    package::Source::Path(location.into())
                }
                Some(location) => package::Source::Git {
                    url: location.clone(),
                    rev: args.get(3).cloned().unwrap_or_else(|| "HEAD".into()),
                },
            };
            change_manifest(|manifest| package::add(manifest, name, &source))?;
            match &source {
                package::Source::Registry(requirement) => eprintln!("ADD {name} {requirement}"),
                _ => eprintln!("ADD {name}"),
            }
        }
        Some("remove") => {
            change_manifest(|manifest| package::remove(manifest, &args[1]))?;
            eprintln!("REMOVE {}", args[1]);
        }
        Some("search") => search(args.get(1).map(String::as_str).unwrap_or(""))?,
        Some("publish") => publish(input)?,
        Some("update") => {
            project::Project::load_updating(input)?;
            let lock = project::Project::manifest_path(input).with_file_name("zam.lock");
            eprintln!("UPDATE {}", lock.display());
        }
        // `zam test [project|file] [名字里的一段]`：运行 test_ 开头的函数，有失败时退出码非零。
        Some("test") => {
            let project = project::Project::load(input)?;
            let failed = project.test(&project::cache_dir(), args.get(2).map(String::as_str))?;
            if failed > 0 {
                return Err(format!("{failed} test(s) failed"));
            }
        }
        Some("new") if args.len() == 2 => new_project(input, false)?,
        Some("new") if args.len() == 4 && args[2] == "--template" && args[3] == "ai" => {
            new_project(input, true)?
        }
        Some("new") if args.len() == 3 && args[2] == "--ai" => new_project(input, true)?,
        Some("new") => return Err("usage: zam new <directory> [--template ai]".into()),
        Some("run") => {
            // 源码后缀统一为 .zm；其余已存在的文件按原生产物直接执行。
            let source = input.extension().is_some_and(|ext| ext == "zm");
            let artifact = if input.is_file() && !source { input.into() }
                else { project::Project::load(input)?.build(&project::cache_dir())? };
            project::run_artifact(&artifact, &program_args.unwrap_or_default())?;
        }
        Some("demo") if args.len() == 1 => demo()?,
        Some("lsp") => lsp::run()?,
        Some("mcp") => mcp::run()?,
        // 用 npm、Bun 装的 zam 由 npm 包里的启动器处理 `zam upgrade`（编译器不在运行时才能替换它）；
        // 走到这里说明是安装脚本或源码装的。
        Some("upgrade") => println!("zam {}: to get updates with one command, install Zamak with npm:\n  npm install -g zamak\nthen run `zam upgrade`. If you installed it with the install script, run the script again.", env!("CARGO_PKG_VERSION")),
        Some("--version") | Some("-V") => println!("{}", env!("CARGO_PKG_VERSION")),
        None | Some("--help") | Some("-h") => print!("{}", zh::help(env!("CARGO_PKG_VERSION"))),
        _ => return Err("usage: zam <check|build|run|test|fmt|new|add|remove|update|search|publish> [project|file]".into()),
    }
    Ok(())
}

/// `zam add` 的第二个参数是版本范围（`^1.2`、`1.0`、`>=1, <2`、`*`）而不是地址或目录。
fn requirement_like(text: &str) -> bool {
    !Path::new(text).exists()
        && text
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || "^~=<>*".contains(c))
}

/// 改当前目录的 zam.toml，再重新加载项目（拉取依赖、更新 zam.lock）。失败时把 zam.toml 与
/// zam.lock 恢复原样，不留下坏掉的依赖。
fn change_manifest(edit: impl FnOnce(&Path) -> project::Result<()>) -> project::Result<()> {
    let manifest = project::Project::manifest_path(Path::new("."));
    let original = std::fs::read(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let lock = manifest.with_file_name("zam.lock");
    let original_lock = std::fs::read(&lock).ok();
    let result = edit(&manifest).and_then(|()| project::Project::load(Path::new(".")).map(|_| ()));
    if result.is_err() {
        let _ = std::fs::write(&manifest, original);
        match original_lock {
            Some(bytes) => {
                let _ = std::fs::write(&lock, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&lock);
            }
        }
    }
    result
}

/// `zam search [词]`：列出索引里名字或介绍含这个词的包。
fn search(word: &str) -> project::Result<()> {
    let mut index = registry::Index::new(&project::cache_dir().join("packages"));
    let word = word.to_lowercase();
    let mut found = 0;
    for name in index.names()? {
        let entry = index.entry(&name)?;
        if !name.contains(&word) && !entry.description.to_lowercase().contains(&word) {
            continue;
        }
        let latest = entry
            .best(|_| true)
            .map_or("-".to_owned(), |release| release.version.to_string());
        println!("{name} {latest}  {}", entry.description);
        found += 1;
    }
    if found == 0 {
        eprintln!("no packages match {word:?} in {}", index.location);
    }
    Ok(())
}

/// `zam publish [项目]`：检查包能用、版本已打标签并推送，然后生成索引条目。索引是本地目录时
/// 直接写进去；否则把条目打到 stdout，由作者向索引仓库提 pull request。
fn publish(input: &Path) -> project::Result<()> {
    let project = project::Project::load(input)?;
    project.check()?;
    let name = &project.name;
    let version = project
        .version
        .ok_or("add version = \"0.1.0\" under [project] in zam.toml before publishing")?;
    let manifest = package::manifest(&project::Project::manifest_path(input), true)?;
    if let Some(dependency) = manifest
        .dependencies
        .iter()
        .find(|(_, source)| matches!(source, package::Source::Path(_)))
        .map(|(name, _)| name)
    {
        return Err(format!(
            "published packages cannot have path dependencies; publish {dependency} too and depend on its version"
        ));
    }
    let root = std::fs::canonicalize(&project.root).map_err(|e| e.to_string())?;
    let top = package::git(&["rev-parse", "--show-toplevel"], Some(&root))
        .map_err(|_| "publish a package from a git repository; run git init and commit it first")?;
    if std::fs::canonicalize(&top).ok().as_deref() != Some(root.as_path()) {
        return Err("zam.toml must be at the root of its git repository to publish it".into());
    }
    // 包自己的 zam.lock 不影响依赖它的项目（它们按自己的锁文件选版本），不算未提交的改动。
    let changes: Vec<String> = package::git(&["status", "--porcelain"], Some(&root))?
        .lines()
        .filter(|line| !line.ends_with("zam.lock"))
        .map(str::to_owned)
        .collect();
    if !changes.is_empty() {
        return Err(format!(
            "commit your changes before publishing; uncommitted: {}",
            changes[..changes.len().min(5)].join(", ")
        ));
    }
    let tag = format!("v{version}");
    let commit = package::git(
        &["rev-parse", "--verify", &format!("{tag}^{{commit}}")],
        Some(&root),
    )
    .map_err(|_| format!("tag the version first: git tag {tag} && git push origin {tag}"))?;
    let remote = package::git(&["remote", "get-url", "origin"], Some(&root))
        .map_err(|_| "add the public repository as the origin remote before publishing")?;
    let repository = public_url(&remote);
    let pushed = package::git(
        &[
            "ls-remote",
            "origin",
            &format!("refs/tags/{tag}"),
            &format!("refs/tags/{tag}^{{}}"),
        ],
        Some(&root),
    )
    .unwrap_or_default();
    if !pushed.lines().any(|line| line.starts_with(&commit)) {
        return Err(format!("push the tag first: git push origin {tag}"));
    }
    let mut index = registry::Index::new(&project::cache_dir().join("packages"));
    let existing = index.find(name)?;
    let quote = |text: &str| toml::Value::from(text).to_string();
    let release = format!("\n[[versions]]\nversion = \"{version}\"\ncommit = \"{commit}\"\n");
    let text = match &existing {
        Some(entry) => {
            if entry.repository != repository {
                return Err(format!(
                    "the name {name} is already taken by {} in the package index; choose another name",
                    entry.repository
                ));
            }
            if entry.versions.iter().any(|old| old.version == version) {
                return Err(format!(
                    "{name} {version} is already published; raise version in zam.toml and tag it"
                ));
            }
            index.raw(name)?.unwrap_or_default().trim_end().to_owned() + "\n" + &release
        }
        None => format!(
            "name = {}\ndescription = {}\nrepository = {}\n{release}",
            quote(name),
            quote(project.description.as_deref().unwrap_or("")),
            quote(&repository),
        ),
    };
    if project.description.as_deref().unwrap_or("").is_empty() {
        eprintln!("WARNING add description = \"...\" under [project] so people can find {name} with zam search");
    }
    if let Some(dir) = index.local() {
        let path = dir.join("packages").join(format!("{name}.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("PUBLISH {} {version} -> {}", name, path.display());
    } else {
        print!("{text}");
        eprintln!(
            "PUBLISH {name} {version}: open a pull request to {} that {} packages/{name}.toml with the text above",
            index.location,
            if existing.is_some() { "replaces" } else { "adds" }
        );
    }
    Ok(())
}

/// 远程地址换成别人能克隆的 https 地址：`git@github.com:a/b.git` → `https://github.com/a/b.git`。
fn public_url(remote: &str) -> String {
    match remote
        .strip_prefix("git@")
        .and_then(|rest| rest.split_once(':'))
    {
        Some((host, path)) => format!("https://{host}/{path}"),
        None => remote.trim_end_matches('/').to_owned(),
    }
}

/// 生成一个最小可运行的项目骨架。目标目录已存在就拒绝，不覆盖任何用户文件。
/// 一页纸语言参考：AI 实测里交给模型的同一份文本，也是 `zam new --template ai` 生成的 Skill 的正文。
const REFERENCE: &str = include_str!("../../bench/ai/live/zamak-reference.md");

/// Skill 的 description：告诉助手什么时候加载它。
const SKILL_DESCRIPTION: &str = "Write, check, fix, and run Zamak code (.zm files, zam.toml projects). Use it whenever you read or change Zamak source, read zam diagnostics, or run zam commands.";

/// 给 AI 编程助手的说明：工作循环在前，语言参考在后。
fn agent_guide() -> String {
    format!(
        "# Working on this Zamak project\n\n\
         Zamak is a statically typed language that compiles to native executables. Read the language reference below before you write code, and follow it closely: it describes the language as `zam {}` implements it.\n\n\
         ## Loop\n\n\
         1. Run `zam check --json .`. Each diagnostic has `file`, `line`, `column`, `function`, a stable `code` such as `ZM301`, a `category`, and usually a `suggestion`.\n\
         2. Make the smallest change that fixes the reported error. Syntax errors are reported one at a time; type and ownership errors are reported once per function, so you can fix several functions in one pass.\n\
         3. Run `zam fmt .`, then check again.\n\
         4. When the check passes, run `zam build .` and `zam run .`. If the project has tests (`fn test_name():` functions that use `assert condition`), run `zam test .`.\n\
         5. To add a library, run `zam add <name>` (from the package index) or `zam add <name> <git-url> [rev]`, then `use <name>`.\n\n\
         Don't guess at features the reference doesn't describe; if a construct is missing, write it with the features that exist.\n\n\
         {}",
        env!("CARGO_PKG_VERSION"),
        REFERENCE.replacen("# Zamak language reference", "## Language reference", 1)
            .replace("\n## ", "\n### ")
            .replacen("### Language reference", "## Language reference", 1)
    )
}

fn new_project(directory: &Path, ai_template: bool) -> project::Result<()> {
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("project name must be UTF-8")?;
    if !project::identifier(name) {
        return Err("project name must be an identifier".into());
    }
    if directory.exists() {
        return Err(format!("{}: already exists", directory.display()));
    }
    let mut files = vec![
        (
            directory.join("zam.toml"),
            format!("[project]\nname = \"{name}\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n"),
        ),
        (
            directory.join("src/main.zm"),
            "fn main():\n    println(\"hello from zamak\")\n".to_string(),
        ),
        (directory.join(".gitignore"), "build/\n".to_string()),
    ];
    if ai_template {
        // 给 AI 编程助手的说明：Claude Code 读 Skill，Codex、Cursor、Copilot 等读 AGENTS.md。
        // 两份内容相同，都由编进 zam 的一页纸语言参考生成，参考改了新项目就跟着更新。
        let guide = agent_guide();
        files.push((
            directory.join(".claude/skills/zamak/SKILL.md"),
            format!("---\nname: zamak\ndescription: {SKILL_DESCRIPTION}\n---\n\n{guide}"),
        ));
        files.push((directory.join("AGENTS.md"), guide));
        // MCP：Claude Code、Cursor 等读 `.mcp.json`，启动 `zam mcp`，助手就能直接调用检查、运行、查文档。
        files.push((directory.join(".mcp.json"), mcp::project_config()));
        files.push((
            directory.join("README.md"),
            format!("# {name}\n\nA Zamak project. Your AI coding agent reads `AGENTS.md` (or the Claude Code skill in `.claude/skills/zamak`) for the language reference and the check-fix loop, and `.mcp.json` starts `zam mcp` so it can call the Zamak tools directly.\n\n```sh\nzam check --json .\nzam fmt .\nzam build .\nzam run .\nzam test .\n```\n"),
        ));
    }
    for (path, text) in files {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("NEW {}", path.display());
    }
    Ok(())
}

/// 就地重排缩进；只写真正变了的文件，改动过的路径打到 stderr（与构建进度一致）。
fn format_sources(input: &Path) -> project::Result<()> {
    // 先解析一遍：语法就不成立的代码不格式化，免得把错误改得更难看。
    let project = project::Project::load_for_format(input)?;
    for path in project.module_paths() {
        let source =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let formatted =
            compiler::format(&source).map_err(|e| format!("{}: {e}", path.display()))?;
        if formatted != source {
            std::fs::write(&path, &formatted).map_err(|e| format!("{}: {e}", path.display()))?;
            eprintln!("FORMAT {}", path.display());
        }
    }
    Ok(())
}

///把一次命令的结果渲染成一行 JSON，供编辑器等工具读取。
fn report(command: &str, entry: &str, outcome: &project::Result<()>) -> String {
    let items = match outcome {
        Ok(()) => String::new(),
        Err(error) => diagnostics(entry, error)
            .iter()
            .map(|(_, diagnostic)| item(diagnostic))
            .collect::<Vec<_>>()
            .join(","),
    };
    format!(
        "{{\"command\":\"{command}\",\"ok\":{},\"diagnostics\":[{items}]}}",
        outcome.is_ok()
    )
}

struct Diagnostic {
    file: String,
    line: Option<usize>,
    column: Option<usize>,
    function: Option<String>,
    code: &'static str,
    category: &'static str,
    message: String,
    suggestion: Option<&'static str>,
}

/// ponytail: 诊断目前是拼好的文本（`project::Result<T> = Result<T, String>`），这里按既有格式
/// 反解成字段；等更多消费者（LSP、CI 注解）出现时，再改成贯穿编译器的结构化 Diagnostic。
fn diagnostic(entry: &str, error: &str) -> Diagnostic {
    let (file, rest) = match error.split_once(": line ") {
        Some((head, tail)) => (head.to_string(), format!("line {tail}")),
        None => (entry.to_string(), error.to_string()),
    };
    let (position, rest) = match rest.split_once(": ") {
        Some((head, tail)) if head.starts_with("line ") => (head.to_string(), tail.to_string()),
        _ => (String::new(), rest),
    };
    let (line, column) = match position.split_once(", ") {
        Some((line, column)) => (
            line.strip_prefix("line ")
                .and_then(|n| n.trim().parse().ok()),
            column
                .strip_prefix("column ")
                .and_then(|n| n.trim().parse().ok()),
        ),
        None => (None, None),
    };
    let (function, message) = match rest.split_once(": ") {
        Some((head, tail)) => {
            let mut parts = head.split(':');
            match (parts.next(), parts.next(), parts.next()) {
                (Some(module), Some(name), None)
                    if identifier(module, true) && identifier(name, false) =>
                {
                    (Some(format!("{module}:{name}")), tail.to_string())
                }
                _ => (None, rest.clone()),
            }
        }
        None => (None, rest.clone()),
    };
    let message = with_python_hint(message);
    // 模块解析期的错误在整条信息里只带 `模块:`（没有函数名），上面的 `模块:函数:` 分支认不出来，
    // `message` 会留着这个前缀；classify 按句子开头匹配，先去掉它再分。
    let body = match message.split_once(": ") {
        Some((head, tail)) if identifier(head, true) => tail.to_owned(),
        _ => message.clone(),
    };
    let (code, category, suggestion) = classify(&body);
    Diagnostic {
        file,
        line,
        column,
        function,
        code,
        category,
        message,
        suggestion,
    }
}

/// 把 Python 的内置名写成未知函数/变量时，在报错末尾补上 Zamak 的写法。
fn with_python_hint(message: String) -> String {
    let name = ["unknown function ", "unknown variable "]
        .iter()
        .find_map(|prefix| message.rsplit_once(prefix).map(|(_, name)| name));
    let hint = match name {
        Some("input") => "read a line with io.read_line()? and use std/io",
        Some("None") => "an optional value is written Option.None, or use a default value",
        Some("self") => "methods take the value as a normal parameter, such as fn area(p: &Point)",
        _ if message.contains(".append:") => "use .push(v) to add to the end",
        _ => return message,
    };
    format!("{message}; {hint}")
}

fn classify(message: &str) -> (&'static str, &'static str, Option<&'static str>) {
    if message.contains("index requires an array, list or map") {
        return (
            "ZM200",
            "type",
            Some("index a list, array or map; slice a string instead"),
        );
    }
    if message.contains("unexpected character") || message.contains("comments start with #") {
        (
            "ZM100",
            "syntax",
            Some("remove or replace the unsupported character"),
        )
    } else if message.contains("unsupported string escape")
        || message.contains("unsupported character escape")
    {
        (
            "ZM101",
            "syntax",
            Some("use a supported escape such as `\\n`, `\\r`, `\\t`, `\\\"`, or `\\\\`"),
        )
    } else if message.contains("use after move") {
        (
            "ZM301",
            "ownership",
            Some("borrow the value or move it only after the last use"),
        )
    } else if message.contains("borrow escape") {
        (
            "ZM422",
            "ownership",
            Some("keep references in call arguments; store or return an owned value"),
        )
    } else if message.contains("borrow type mismatch") {
        (
            "ZM423",
            "ownership",
            Some("match the borrow type to the parameter declaration"),
        )
    } else if message.contains("cannot mutably borrow immutable") {
        (
            "ZM424",
            "ownership",
            Some("declare the owner with `let mut` before taking `&mut`"),
        )
    } else if message.contains("expected explicit borrow") {
        (
            "ZM425",
            "ownership",
            Some("pass `&value` for a shared borrow or `&mut value` for an exclusive borrow"),
        )
    } else if message.contains("borrowed by an enclosing for loop") {
        (
            "ZM421",
            "ownership",
            Some("finish the loop or the borrow first: collect the changes in another list, then apply them after the loop"),
        )
    } else if message.contains("borrow") || message.contains("borrowed") {
        (
            "ZM421",
            "ownership",
            Some("avoid overlapping moves or exclusive borrows in the same call"),
        )
    } else if message.contains("missing string return value") {
        (
            "ZM510",
            "control_flow",
            Some("return a string on every path or change the function result type"),
        )
    } else if message.contains("does not cover") || message.contains("unreachable match arm") {
        (
            "ZM511",
            "control_flow",
            Some("add an arm for each missing variant or a final `_:` arm, and remove arms after `_`"),
        )
    } else if message.contains("fallible call") || message.contains("propagate the error") {
        (
            "ZM500",
            "error_handling",
            Some("handle the call with `?` or a `catch` block"),
        )
    } else if message.contains("unsupported error type") {
        (
            "ZM509",
            "error_handling",
            Some("use a string, a struct or an enum as the error type, such as `T!string` or `T!ParseError`"),
        )
    } else if message.contains("f64 literal is out of range") {
        (
            "ZM600",
            "arithmetic",
            Some("use an f64 literal no larger than 1.7976931348623157e308"),
        )
    } else if message.contains("literal overflow") {
        (
            "ZM600",
            "arithmetic",
            Some("use a literal that fits in `i64` (max 9223372036854775807)"),
        )
    } else if message.contains("overflow") || message.contains("division by zero") {
        (
            "ZM600",
            "arithmetic",
            Some("check the input range before arithmetic or division"),
        )
    } else if message.contains("array index") {
        (
            "ZM406",
            "bounds",
            Some("ensure the index is within the declared array length"),
        )
    } else if message.contains("arrays are only supported as local values")
        || message.contains("only [i64; N] arrays are supported")
        || message.contains("array length must be")
        || message.contains("invalid array length")
    {
        (
            "ZM200",
            "type",
            Some(
                "fixed arrays are written [T; N] with a length from 1 to 1024 and declared as local values, and [T] is a growable list",
            ),
        )
    } else if message.starts_with("test function ") || message.contains(": test function ") {
        (
            "ZM200",
            "type",
            Some(
                "declare tests as fn test_name(): or fn test_name() -> !string: with no parameters",
            ),
        )
    } else if message.contains("cannot be used as a value")
        || message.contains("function values cannot")
        || message.contains("argument count mismatch")
    {
        (
            "ZM200",
            "type",
            Some("match the function type: the same parameter count and types, and no !E"),
        )
    } else if message.contains("is already declared") {
        (
            "ZM204",
            "name",
            Some("rename the parameter; Zamak does not allow shadowing"),
        )
    } else if message.contains("anonymous fn holds one expression") {
        (
            "ZM110",
            "syntax",
            Some("keep fn(...): on one line, or declare a named fn and pass its name"),
        )
    } else if message.contains("cannot infer ")
        || message.contains("type argument")
        || message.contains("is not generic")
    {
        (
            "ZM200",
            "type",
            Some("write the type arguments in an annotation, such as let x: Option[i64] = Option.None"),
        )
    } else if message.contains("type mismatch") {
        (
            "ZM200",
            "type",
            Some("match the expression type to the declared type"),
        )
    } else if message.contains("unknown field ") {
        (
            "ZM202",
            "name",
            Some("check the field name against the struct declaration"),
        )
    } else if message.contains("unknown variable ")
        || message.contains("unknown function ")
        || message.contains("unknown struct")
        || message.contains("unknown type name")
        || message.contains("reserved type name")
        || message.contains("unknown variant ")
        || message.contains("unknown method .")
    {
        (
            "ZM201",
            "name",
            Some("check the name, declaration, and module import"),
        )
    } else if message.contains("ambiguous or missing import for ")
        || message.contains("missing or ambiguous struct import")
        || message.contains("ambiguous string import")
        || message.contains(" requires use std/")
    {
        (
            "ZM203",
            "name",
            Some("check the matching `use` import; add it if missing or disambiguate modules with the same alias"),
        )
    } else if message.starts_with("duplicate ") {
        (
            "ZM204",
            "name",
            Some("remove or rename the duplicate declaration"),
        )
    } else if message.contains("declared inside a function")
        || message.contains("cannot call itself")
        || message.contains("cannot be declared mut")
    {
        (
            "ZM200",
            "type",
            Some("declare the fn at the top level of the module; a fn inside a function cannot be generic, fail, call itself or be reassigned yet"),
        )
    } else if message.contains("needs no ? or catch") {
        (
            "ZM500",
            "error_handling",
            Some("remove the ? or catch; only calls whose function type has !E can fail"),
        )
    } else if message.starts_with("missing return") {
        (
            "ZM200",
            "type",
            Some("end every path of the fn body with a value or a return statement"),
        )
    } else if message.starts_with("cannot assign immutable value ")
        || message.contains("cannot assign to the constant ")
    {
        (
            "ZM302",
            "ownership",
            Some("declare the binding with `let mut` before assigning it"),
        )
    } else if message.contains("unsupported") && message.contains("extern fn") {
        (
            "ZM900",
            "unsupported",
            Some("extern fn takes i64, f64, bool or &string and returns nothing, i64, f64, bool or string; convert other data in a small C function listed in [native] sources"),
        )
    } else if message.starts_with("extern fn ") || message.contains("expected fn after extern") {
        (
            "ZM110",
            "syntax",
            Some("write extern fn name(params) -> T on one line, with no body, type parameters or error type"),
        )
    } else if message.contains("unsupported") {
        (
            "ZM900",
            "unsupported",
            Some("use a construct listed as supported in `zam --help` or the tutorial"),
        )
    } else if message.contains("blocks are indented, not wrapped in braces") {
        (
            "ZM110",
            "syntax",
            Some("end the line with `:` instead of `{`, indent the block by 4 spaces, and delete the closing `}`"),
        )
    } else if message.contains("indentation contains a tab") {
        (
            "ZM110",
            "syntax",
            Some("run `zam fmt` to replace the tabs with spaces"),
        )
    } else if message.contains("indentation may have been lost") {
        (
            "ZM110",
            "syntax",
            Some("restore the indentation: indent every block 4 spaces under the line that ends with `:`"),
        )
    } else if message.contains("indent") {
        (
            "ZM110",
            "syntax",
            Some("indent the lines of a block 4 spaces deeper than the line ending with `:`, and line up lines of the same block"),
        )
    } else if message.contains("instead of `else if`") {
        (
            "ZM110",
            "syntax",
            Some("write `elif condition:` instead of `else if condition:`"),
        )
    } else if ["instead of `&&`", "instead of `||`", "instead of `!`"]
        .iter()
        .any(|text| message.contains(text))
    {
        (
            "ZM110",
            "syntax",
            Some("logical operators are words: write `a and b`, `a or b`, `not a`"),
        )
    } else if message.contains("was never closed") {
        ("ZM110", "syntax", Some("add the matching closing bracket"))
    } else if message.contains("this line is only a value") {
        (
            "ZM110",
            "syntax",
            Some("use the value in a let or a call, or make the value the only line of the catch block"),
        )
    } else if message.contains("a single `=` assigns") {
        (
            "ZM110",
            "syntax",
            Some("compare with `==`, for example `score == 60`; a single `=` only assigns"),
        )
    } else if message.starts_with("expected")
        || message.contains("unterminated")
        || message.contains("character literal")
    {
        (
            "ZM110",
            "syntax",
            Some("repair the reported token or delimiter, then run `zam check` again"),
        )
    } else {
        ("ZM999", "general", None)
    }
}

/// 诊断里的 `<模块>:<函数>` 前缀只用标识符字符，磁盘路径与普通句子不会误判。
fn identifier(text: &str, slash: bool) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || (slash && c == '/'))
}

fn item(diagnostic: &Diagnostic) -> String {
    format!(
        "{{\"file\":{},\"line\":{},\"column\":{},\"function\":{},\"code\":{},\"category\":{},\"message\":{},\"suggestion\":{}}}",
        text(Some(&diagnostic.file)),
        number(diagnostic.line),
        number(diagnostic.column),
        text(diagnostic.function.as_deref()),
        text(Some(diagnostic.code)),
        text(Some(diagnostic.category)),
        text(Some(&diagnostic.message)),
        text(diagnostic.suggestion),
    )
}

fn number(value: Option<usize>) -> String {
    value.map_or_else(|| "null".to_string(), |value| value.to_string())
}

fn text(value: Option<&str>) -> String {
    value.map_or_else(
        || "null".to_string(),
        |value| format!("\"{}\"", escape(value)),
    )
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn demo() -> project::Result<()> {
    let root = std::env::temp_dir().join(format!(
        "zam-demo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let cache = root.join("cache");
    let result = (|| {
        for directory in ["a", "b"] {
            let dir = root.join(directory);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(
                dir.join("hello.zm"),
                "fn main(): println(\"hello from zamak\")\n",
            )
            .map_err(|e| e.to_string())?;
            project::Project::load(&dir.join("hello.zm"))?.build(&cache)?;
        }
        let count = std::fs::read_dir(&cache)
            .map_err(|e| e.to_string())?
            .count();
        if count != 3 {
            return Err("path-independent caching failed".into());
        }
        println!("path-independent content-addressed cache verified");
        Ok(())
    })();
    std::fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
    result
}

fn main() {
    if let Err(error) = execute() {
        // 带行列的诊断渲染成「错误码 + 位置 + 源码行 + 下划线 + 建议」；其余错误保持一行。
        // 没有位置的错误不渲染源码，兜底文件名用不到。
        let diagnostics = diagnostics(".", &error);
        let count = diagnostics.len();
        for (index, (raw, diagnostic)) in diagnostics.iter().enumerate() {
            if index > 0 {
                eprintln!();
            }
            match render(diagnostic) {
                Some(text) => eprintln!("{text}"),
                None => eprintln!("zam: {}", zh::text(raw)),
            }
        }
        if count > 1 {
            eprintln!("\n{}", zh::error_count(count));
        }
        std::process::exit(1);
    }
}

/// 拆开一次失败里的多条诊断，按文件、行、列排序。
fn diagnostics(entry: &str, error: &str) -> Vec<(String, Diagnostic)> {
    let mut items: Vec<_> = error
        .split(compiler::DIAGNOSTIC_SEPARATOR)
        .map(|raw| (raw.to_owned(), diagnostic(entry, raw)))
        .collect();
    items.sort_by(|(_, a), (_, b)| (&a.file, a.line, a.column).cmp(&(&b.file, b.line, b.column)));
    items
}

/// 终端里的显示宽度：制表符按 4 格，东亚宽字符与 emoji 按 2 格。
fn width(c: char) -> usize {
    match c {
        '\t' => 4,
        '\u{1100}'..='\u{115F}'
        | '\u{2E80}'..='\u{A4CF}'
        | '\u{AC00}'..='\u{D7A3}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF00}'..='\u{FF60}'
        | '\u{FFE0}'..='\u{FFE6}'
        | '\u{1F300}'..='\u{1FAFF}'
        | '\u{20000}'..='\u{3FFFD}' => 2,
        _ => 1,
    }
}

/// 源码行（制表符展开）和对应的下划线行。`column` 从 1 开始按字符计数；`token` 为真时只划
/// 出该位置的一个记号（词法/语法错误），否则划到语句所在行的末尾（语义错误指向语句起点）。
fn underline(line: &str, column: usize, token: bool) -> (String, String) {
    let chars: Vec<char> = line.chars().collect();
    let start = column.saturating_sub(1).min(chars.len());
    let end = if token {
        let mut end = start;
        match chars.get(start) {
            Some(c) if c.is_alphanumeric() || *c == '_' => {
                while chars
                    .get(end)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                {
                    end += 1;
                }
            }
            Some('"') => {
                end += 1;
                while chars.get(end).is_some_and(|c| *c != '"') {
                    end += 1;
                }
                end = (end + 1).min(chars.len());
            }
            Some(_) => end += 1,
            None => {}
        }
        end
    } else {
        let mut end = chars.len();
        while end > start && (chars[end - 1].is_whitespace() || chars[end - 1] == '{') {
            end -= 1;
        }
        end
    };
    let shown = line.replace('\t', "    ");
    let pad: usize = chars[..start].iter().map(|c| width(*c)).sum();
    let span: usize = chars[start..end].iter().map(|c| width(*c)).sum();
    (
        shown,
        format!("{}{}", " ".repeat(pad), "^".repeat(span.max(1))),
    )
}

fn render(diagnostic: &Diagnostic) -> Option<String> {
    let (line, column) = (diagnostic.line?, diagnostic.column?);
    // 左侧栏按行号宽度对齐，`-->`、`|`、`=` 落在同一列。
    let gutter = " ".repeat(line.to_string().len());
    // 终端显示按语言翻译（见 zh.rs）；--json 的输出不经过这里，始终是英文。
    let colon = if zh::enabled() { "：" } else { ": " };
    let mut out = format!(
        "{}[{}]{colon}{}\n{gutter}--> {}:{line}:{column}",
        zh::word("error"),
        diagnostic.code,
        zh::text(&diagnostic.message),
        diagnostic.file
    );
    let source = std::fs::read_to_string(&diagnostic.file).ok();
    if let Some(text) = source
        .as_deref()
        .and_then(|text| text.lines().nth(line - 1))
    {
        let (shown, marks) = underline(text, column, diagnostic.category == "syntax");
        out.push_str(&format!(
            "\n{gutter} |\n{line} | {shown}\n{gutter} | {marks}"
        ));
    }
    if let Some(suggestion) = diagnostic.suggestion {
        out.push_str(&format!(
            "\n{gutter} = {}{colon}{}",
            zh::word("help"),
            zh::text(suggestion)
        ));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn underline_marks_tokens_or_statements_by_display_width() {
        // 记号：标识符、字符串字面量、单个符号；中文按 2 格对齐，制表符展开成 4 格。
        assert_eq!(
            underline("let 名字 = @x", 10, true),
            ("let 名字 = @x".into(), "           ^".into())
        );
        assert_eq!(
            underline("\tprintln(\"未闭合", 10, true),
            ("    println(\"未闭合".into(), "            ^^^^^^^".into())
        );
        assert_eq!(underline("x", 5, true).1, " ^");
        // 语句：划到行尾，去掉行尾空白与开块的 `{`。
        assert_eq!(
            underline("    while xs.len() > 0 {  ", 5, false).1,
            "    ^^^^^^^^^^^^^^^^^^"
        );
    }

    #[test]
    fn diagnostic_categories_distinguish_repair_actions() {
        for (message, code) in [
            (
                "borrow escape: references cannot be stored in locals",
                "ZM422",
            ),
            ("borrow type mismatch for text", "ZM423"),
            ("cannot mutably borrow immutable value text", "ZM424"),
            ("expected explicit borrow of text", "ZM425"),
            ("borrow conflict: text", "ZM421"),
            ("string type mismatch", "ZM200"),
            ("unknown variable text", "ZM201"),
            ("entry: unknown function text", "ZM201"),
            ("unknown struct Point", "ZM201"),
            ("unknown type name Thing", "ZM201"),
            ("unknown field z", "ZM202"),
            (
                "main: ambiguous or missing import for string.byte_len",
                "ZM203",
            ),
            ("string.byte_len requires use std/string", "ZM203"),
            ("missing or ambiguous struct import", "ZM203"),
            ("ambiguous string import", "ZM203"),
            ("duplicate function foo", "ZM204"),
            ("reserved type name Thing", "ZM201"),
            ("cannot assign immutable value count", "ZM302"),
            ("unsupported type f64", "ZM900"),
            ("i64 literal overflow", "ZM600"),
            ("f64 literal is out of range", "ZM600"),
            (
                "type mismatch: + mixes i64 and f64; convert one side with f64(n) or i64(x)",
                "ZM200",
            ),
        ] {
            let d = diagnostic("entry.zm", message);
            assert_eq!(d.code, code, "{message}");
            assert!(d.suggestion.is_some());
            assert_eq!(d.message, message);
        }
    }

    #[test]
    fn literal_overflow_suggests_a_smaller_literal() {
        let d = diagnostic("entry.zm", "i64 literal overflow");
        assert_eq!(d.code, "ZM600");
        assert_eq!(d.category, "arithmetic");
        assert_eq!(
            d.suggestion,
            Some("use a literal that fits in `i64` (max 9223372036854775807)")
        );
    }

    fn parse(entry: &str, error: &str) -> Diagnostic {
        diagnostic(entry, error)
    }

    #[test]
    fn diagnostics_parse_every_shape() {
        let with_file = parse(
            "entry.zm",
            r"C:\a\main.zm: line 3, column 9: unexpected character",
        );
        assert_eq!(with_file.file, r"C:\a\main.zm");
        assert_eq!((with_file.line, with_file.column), (Some(3), Some(9)));
        assert_eq!(with_file.function, None);
        assert_eq!(with_file.message, "unexpected character");
        assert_eq!(with_file.code, "ZM100");
        assert_eq!(with_file.category, "syntax");

        // 语义诊断由 project::check 传 <模块>:main 名字，消息里还可能出现别的冒号。
        let semantic = parse(
            "entry.zm",
            "line 6, column 5: main:main: fallible call main:f must be handled with ?",
        );
        assert_eq!(semantic.file, "entry.zm");
        assert_eq!(semantic.function.as_deref(), Some("main:main"));
        assert_eq!(
            semantic.message,
            "fallible call main:f must be handled with ?"
        );

        // 与位置无关的错误保留整段原文，位置为空。
        let plain = parse("entry.zm", "missing entry module main");
        assert_eq!(
            (plain.line, plain.column, plain.function),
            (None, None, None)
        );
        assert_eq!(plain.message, "missing entry module main");

        // 路径里的冒号不能当成 <模块>:<函数> 前缀。
        let config = parse("entry.zm", r"C:\p\zam.toml: missing string project.name");
        assert_eq!(config.file, "entry.zm");
        assert_eq!(config.function, None);
        assert_eq!(
            config.message,
            r"C:\p\zam.toml: missing string project.name"
        );
    }

    #[test]
    fn report_is_one_line_json() {
        let ok: project::Result<()> = Ok(());
        assert_eq!(
            report("build", "entry.zm", &ok),
            r#"{"command":"build","ok":true,"diagnostics":[]}"#
        );

        let failed: project::Result<()> =
            Err(r#"C:\a\main.zm: line 1, column 2: bad "quote""#.into());
        let line = report("check", "entry.zm", &failed);
        assert_eq!(line.lines().count(), 1);
        assert_eq!(
            line,
            r#"{"command":"check","ok":false,"diagnostics":[{"file":"C:\\a\\main.zm","line":1,"column":2,"function":null,"code":"ZM999","category":"general","message":"bad \"quote\"","suggestion":null}]}"#
        );
    }
}
