//! `zam mcp`：给 AI 编程助手用的 MCP（Model Context Protocol）服务器，经 stdin/stdout 通信，
//! 每行一条 JSON-RPC 消息。
//!
//! 工具：
//! - `check`、`build`、`run`、`test`、`format`：调用 zam 自己的子命令，结果与命令行完全一致
//!   （check 与 build 返回 `--json` 的诊断）。另起进程运行，编译器内部出意外也不会拖垮服务器。
//! - `explain`：按错误码（如 ZM301）给出含义与改法。
//! - `docs`：一页纸语言参考（与 `zam new --template ai` 生成的相同）、某个标准库模块的函数签名，
//!   或参考里与关键词有关的段落。
//!
//! 资源（`resources/list`、`resources/read`）：一页纸语言参考（`zamak://reference`）与错误码表
//! （`zamak://diagnostics`），客户端可以把它们挂在上下文里，不用每次都调 `docs`、`explain`。

use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 最新的协议版本；客户端要求别的版本时照它说的回（我们只用到各版本共有的部分）。
const PROTOCOL: &str = "2025-06-18";
/// 工具输出的上限：太长的程序输出截断，免得塞满助手的上下文。
const LIMIT: usize = 64 * 1024;
const DIAGNOSTICS: &str = include_str!("../../book/en/src/appendix-diagnostics.md");
/// MCP 资源的 URI：语言参考与错误码表。
const REFERENCE_URI: &str = "zamak://reference";
const DIAGNOSTICS_URI: &str = "zamak://diagnostics";

pub fn run() -> crate::project::Result<()> {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            send(
                &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "parse error" } }),
            );
            continue;
        };
        if let Some(reply) = handle(&message) {
            send(&reply);
        }
    }
    Ok(())
}

fn send(message: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

/// 处理一条消息；通知（没有 id）不回复。
fn handle(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let params = &message["params"];
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": { "tools": {}, "resources": {} },
            "serverInfo": { "name": "zamak", "title": "Zamak", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Tools and resources for the Zamak programming language. Read the zamak://reference resource (or call docs) before you write Zamak code, check after every change, and explain when a diagnostic code is unclear. Paths are relative to the folder the server runs in; \".\" is the current project.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => Ok(call(
            params["name"].as_str().unwrap_or_default(),
            &params["arguments"],
        )),
        "resources/list" => Ok(json!({ "resources": resources() })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "resources/read" => {
            let uri = params["uri"].as_str().unwrap_or_default();
            match resource(uri) {
                Some(text) => Ok(json!({
                    "contents": [{ "uri": uri, "mimeType": "text/markdown", "text": text }],
                })),
                None => {
                    Err(json!({ "code": -32602, "message": format!("unknown resource {uri}") }))
                }
            }
        }
        method => Err(json!({ "code": -32601, "message": format!("unknown method {method}") })),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    })
}

fn path_schema() -> Value {
    json!({ "type": "string", "description": "A project folder (with zam.toml) or a .zm file. Defaults to \".\"." })
}

fn tools() -> Value {
    let read_only = json!({ "readOnlyHint": true });
    json!([
        {
            "name": "check",
            "title": "Check Zamak code",
            "description": "Check types and ownership without compiling. Returns the same JSON as `zam check --json`: ok, and diagnostics with file, line, column, function, a stable code such as ZM301, a category, the message, and usually a suggestion. Call it after every change.",
            "inputSchema": { "type": "object", "properties": { "path": path_schema() } },
            "annotations": read_only,
        },
        {
            "name": "build",
            "title": "Build a Zamak program",
            "description": "Compile to a standalone native executable. Returns the JSON of `zam build --json` and the path of the executable.",
            "inputSchema": { "type": "object", "properties": { "path": path_schema() } },
        },
        {
            "name": "run",
            "title": "Run a Zamak program",
            "description": "Build and run the program. Returns its standard output, standard error, and exit code. The program stops after 60 seconds.",
            "inputSchema": { "type": "object", "properties": {
                "path": path_schema(),
                "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments for the program." },
                "stdin": { "type": "string", "description": "Text to send to the program's standard input." },
            } },
        },
        {
            "name": "test",
            "title": "Run Zamak tests",
            "description": "Run the test_... functions, as `zam test`. Returns the test report and the exit code; failures show the assertion and the values.",
            "inputSchema": { "type": "object", "properties": {
                "path": path_schema(),
                "filter": { "type": "string", "description": "Run only the tests whose names contain this text." },
            } },
        },
        {
            "name": "format",
            "title": "Format Zamak code",
            "description": "Rewrite the project's .zm files in the standard format, as `zam fmt`. Returns the files it changed.",
            "inputSchema": { "type": "object", "properties": { "path": path_schema() } },
            "annotations": { "idempotentHint": true },
        },
        {
            "name": "explain",
            "title": "Explain a diagnostic code",
            "description": "Explain a Zamak diagnostic code, such as ZM301, and how to fix it.",
            "inputSchema": { "type": "object", "properties": {
                "code": { "type": "string", "description": "The diagnostic code, such as ZM301." },
            }, "required": ["code"] },
            "annotations": read_only,
        },
        {
            "name": "docs",
            "title": "Zamak documentation",
            "description": "Without a topic, return the one-page Zamak language reference: read it before you write Zamak code. With a standard library module such as string, fs, io, math, or time, list its functions with their types. With any other word, return the parts of the reference that mention it.",
            "inputSchema": { "type": "object", "properties": {
                "topic": { "type": "string", "description": "A standard library module or a word to look up, such as \"string\", \"match\", or \"catch\"." },
            } },
            "annotations": read_only,
        },
        {
            "name": "hover",
            "title": "Look up a type at a position",
            "description": "Given a file, a line, and a column (both counted from 1), return what the name there is: the type of a local variable as the checker inferred it, or the signature of a function, method, field, or module function. Use it to find out what a value is before you call a method on it.",
            "inputSchema": { "type": "object", "properties": {
                "path": path_schema(),
                "line": { "type": "integer", "description": "The line, counted from 1." },
                "column": { "type": "integer", "description": "The column, counted from 1." },
            }, "required": ["line", "column"] },
            "annotations": read_only,
        },
    ])
}

/// `resources/list`：挂在上下文里就能用的两份文档。
fn resources() -> Value {
    json!([
        {
            "uri": REFERENCE_URI,
            "name": "Zamak language reference",
            "title": "Zamak language reference",
            "description": "A one-page reference: syntax, types, ownership, errors, and the standard library. Read it before you write Zamak code.",
            "mimeType": "text/markdown",
        },
        {
            "uri": DIAGNOSTICS_URI,
            "name": "Zamak diagnostic codes",
            "title": "Zamak diagnostic codes",
            "description": "Every diagnostic code with its category and meaning, so a code from check --json can be explained without another call.",
            "mimeType": "text/markdown",
        },
    ])
}

/// `resources/read`：按 URI 取出文档原文；没有这个 URI 时返回 None。
fn resource(uri: &str) -> Option<String> {
    match uri {
        REFERENCE_URI => Some(crate::REFERENCE.to_owned()),
        DIAGNOSTICS_URI => Some(DIAGNOSTICS.to_owned()),
        _ => None,
    }
}

fn text(content: String, error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": content }], "isError": error })
}

fn call(name: &str, arguments: &Value) -> Value {
    let path = arguments["path"]
        .as_str()
        .filter(|path| !path.is_empty())
        .unwrap_or(".");
    match name {
        "check" | "build" => {
            let output = zam(&[name, "--json", path], None, Duration::from_secs(300));
            let report = output
                .stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .unwrap_or_default();
            if report.is_empty() {
                return text(format!("zam {name} failed:\n{}", output.stderr), true);
            }
            let mut answer = report.to_owned();
            if name == "build" {
                if let Some(artifact) = output
                    .stderr
                    .lines()
                    .find_map(|line| line.strip_prefix("OUTPUT "))
                {
                    answer.push_str(&format!("\nexecutable: {artifact}"));
                }
            }
            text(answer, false)
        }
        "run" => {
            let mut args = vec!["run", path];
            let extra: Vec<&str> = arguments["args"]
                .as_array()
                .map(|items| items.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if !extra.is_empty() {
                args.push("--");
                args.extend(extra);
            }
            let output = zam(&args, arguments["stdin"].as_str(), Duration::from_secs(360));
            // 构建进度（CACHE、OBJECT、LINK、OUTPUT 行）对助手没用，只留程序自己写的标准错误。
            let stderr: Vec<&str> = output
                .stderr
                .lines()
                .filter(|line| {
                    !["CACHE ", "OBJECT ", "LINK ", "OUTPUT ", "TIMING "]
                        .iter()
                        .any(|p| line.starts_with(p))
                })
                .collect();
            text(
                format!(
                    "exit code: {}\n--- stdout\n{}--- stderr\n{}",
                    output.status,
                    output.stdout,
                    stderr.join("\n")
                ),
                output.status != "0",
            )
        }
        "test" => {
            let mut args = vec!["test", path];
            if let Some(filter) = arguments["filter"].as_str().filter(|f| !f.is_empty()) {
                args.push(filter);
            }
            let output = zam(&args, None, Duration::from_secs(600));
            let failed = output.status != "0";
            let mut answer = output.stdout;
            if failed && answer.trim().is_empty() {
                answer = output.stderr;
            }
            text(format!("exit code: {}\n{answer}", output.status), failed)
        }
        "format" => {
            let output = zam(&["fmt", path], None, Duration::from_secs(120));
            if output.status != "0" {
                return text(format!("zam fmt failed:\n{}", output.stderr), true);
            }
            let changed: Vec<&str> = output
                .stderr
                .lines()
                .filter_map(|line| line.strip_prefix("FORMAT "))
                .collect();
            text(
                if changed.is_empty() {
                    "already formatted".to_owned()
                } else {
                    format!("formatted:\n{}", changed.join("\n"))
                },
                false,
            )
        }
        "explain" => explain(arguments["code"].as_str().unwrap_or_default()),
        "docs" => docs(arguments["topic"].as_str().unwrap_or_default()),
        "hover" => hover(
            path,
            arguments["line"].as_u64(),
            arguments["column"].as_u64(),
        ),
        _ => text(format!("unknown tool {name}"), true),
    }
}

struct Output {
    status: String,
    stdout: String,
    stderr: String,
}

/// 运行 zam 自己的一个子命令；超时就结束它。
fn zam(args: &[&str], stdin: Option<&str>, limit: Duration) -> Output {
    let failed = |message: String| Output {
        status: "error".into(),
        stdout: String::new(),
        stderr: message,
    };
    let Ok(exe) = std::env::current_exe() else {
        return failed("cannot find the zam executable".into());
    };
    let child = Command::new(exe)
        .args(args)
        // 诊断文字给助手看，统一用英文（与 --json 一致）。
        .env("ZAMAK_LANG", "en")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return failed(format!("cannot start zam: {error}")),
    };
    if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
        let text = text.to_owned();
        std::thread::spawn(move || {
            let _ = input.write_all(text.as_bytes());
        });
    }
    // 另开线程读输出，避免管道写满后双方互等。
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = out.read_to_end(&mut text);
        text
    });
    let error_reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = err.read_to_end(&mut text);
        text
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break status
                    .code()
                    .map_or("killed".to_owned(), |code| code.to_string())
            }
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                break format!("timed out after {} seconds", limit.as_secs());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => break format!("error: {error}"),
        }
    };
    let clip = |bytes: Vec<u8>| {
        let mut text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
        if text.len() > LIMIT {
            let mut cut = LIMIT;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str("\n… (output truncated)\n");
        }
        text
    };
    Output {
        status,
        stdout: clip(reader.join().unwrap_or_default()),
        stderr: clip(error_reader.join().unwrap_or_default()),
    }
}

fn explain(code: &str) -> Value {
    let code = code.trim().to_ascii_uppercase();
    let row = DIAGNOSTICS
        .lines()
        .find(|line| line.starts_with(&format!("| `{code}` |")));
    let Some(row) = row else {
        let known: Vec<&str> = DIAGNOSTICS
            .lines()
            .filter_map(|line| line.strip_prefix("| `")?.split('`').next())
            .collect();
        return text(
            format!(
                "unknown diagnostic code {code}; known codes: {}",
                known.join(", ")
            ),
            true,
        );
    };
    let cells: Vec<&str> = row.trim_matches('|').split('|').map(str::trim).collect();
    let (category, meaning) = (
        cells.get(1).copied().unwrap_or(""),
        cells.get(2).copied().unwrap_or(""),
    );
    let advice = match code.as_str() {
        "ZM110" => "Read the message: it names the token that's wrong. Common causes are a missing `:` at the end of an `if`, `for`, `while`, `fn`, or `match` line, indentation that isn't 4 spaces, `else if` instead of `elif`, `&&` instead of `and`, and `=` where `==` belongs.",
        "ZM200" => "Make the expression's type match the declared type. `/` always gives f64; use `//` for integer division. Convert with `i64(x)` or `f64(n)`. Annotate empty lists and maps, such as `let xs: [i64] = []`.",
        "ZM201" | "ZM203" => "Check the spelling, then the `use` line: standard library modules need `use std/string`, `use std/fs`, and so on, and other modules need `use name`. Functions from another module must be `pub fn`.",
        "ZM301" => "Assigning a string, list, map, or struct moves it. Borrow it with `&x` when you pass it to a function that only reads it, copy it with `copy(&x)`, or move it only after its last use.",
        "ZM302" | "ZM424" => "Declare the variable with `let mut` if you assign to it or pass it as `&mut`.",
        "ZM421" => "Don't change a list or map while a `for` loop or a borrowed local reads it. Collect the changes in another list and apply them after the loop, or loop over indexes with `while`.",
        "ZM422" => "Functions return owned values, not references. Return a copy or build a new value; keep `&` borrows in call arguments.",
        "ZM425" => "Write `&value` to lend a value for reading or `&mut value` to let the function change it.",
        "ZM500" => "The call can fail (its type has `!E`). Add `?` to pass the error up (the function must also declare `!E`), or handle it with `catch e: fallback`.",
        "ZM511" => "Add an arm for every variant, or end the match with `_:`; remove arms after `_`.",
        _ => "",
    };
    text(
        format!("{code} ({category}): {meaning}\n{advice}")
            .trim_end()
            .to_owned(),
        false,
    )
}

fn docs(topic: &str) -> Value {
    let reference = crate::REFERENCE;
    let topic = topic.trim();
    if topic.is_empty() {
        return text(reference.to_owned(), false);
    }
    let module = topic.trim_start_matches("std/");
    let functions = crate::lsp::std_functions(module);
    if !functions.is_empty() {
        let mut answer = format!("use std/{module}\n\n");
        for (_, signature) in functions {
            answer.push_str(&signature);
            answer.push('\n');
        }
        return text(answer, false);
    }
    // 参考里提到这个词的段落（按 `## ` 小节切开）。
    let needle = topic.to_lowercase();
    let sections: Vec<&str> = split_sections(reference)
        .into_iter()
        .filter(|section| section.to_lowercase().contains(&needle))
        .collect();
    if sections.is_empty() {
        return text(format!("the reference doesn't mention \"{topic}\"; call docs without a topic for the whole reference"), false);
    }
    text(sections.join("\n\n"), false)
}

/// `hover`：给出文件、行、列（都从 1 起），返回那里的名字是什么——检查器推断出的局部变量
/// 类型，或函数、方法、字段、标准库函数的签名。与编辑器里的悬停用的是同一套查询。
fn hover(path: &str, line: Option<u64>, column: Option<u64>) -> Value {
    let (Some(line), Some(column)) = (line, column) else {
        return text(
            "hover needs line and column, both counted from 1".into(),
            true,
        );
    };
    // 单个文件直接读；给的是目录时找入口模块。
    let file = if path.ends_with(".zm") {
        std::path::PathBuf::from(path)
    } else {
        let root = std::path::Path::new(path);
        let manifest = root.join("zam.toml");
        let entry = std::fs::read_to_string(&manifest)
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.trim().strip_prefix("entry").map(str::to_owned))
                    .and_then(|rest| {
                        let rest = rest.trim().strip_prefix('=')?.trim();
                        Some(rest.trim_matches('"').to_owned())
                    })
            })
            .unwrap_or_else(|| "src/main.zm".to_owned());
        root.join(entry)
    };
    let text_of = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) => {
            return text(format!("cannot read {}: {error}", file.display()), true);
        }
    };
    let (line, column) = (line as usize, column as usize);
    if line == 0 || column == 0 {
        return text("line and column start at 1".into(), true);
    }
    // 编辑器里的悬停优先给出检查器的结论。
    if let Some((name, ty)) = crate::lsp::type_of(&text_of, line - 1, column - 1) {
        return text(format!("{name}: {ty}"), false);
    }
    // 不是局部变量：按声明找函数、方法、字段或标准库函数。
    match crate::lsp::describe_at(&text_of, line - 1, column - 1) {
        Some(answer) => text(answer, false),
        None => text(
            format!(
                "nothing declared at {}:{line}:{column}; put the column inside a name",
                file.display()
            ),
            false,
        ),
    }
}

fn split_sections(text: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = text
        .match_indices("\n## ")
        .map(|(index, _)| index + 1)
        .collect();
    starts.insert(0, 0);
    starts.push(text.len());
    starts
        .windows(2)
        .map(|pair| text[pair[0]..pair[1]].trim())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `zam new --template ai` 写进项目的 MCP 配置（Claude Code、Cursor 等读 `.mcp.json`）。
pub fn project_config() -> String {
    "{\n  \"mcpServers\": {\n    \"zamak\": {\n      \"command\": \"zam\",\n      \"args\": [\"mcp\"]\n    }\n  }\n}\n".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_serve_the_reference_and_the_code_table() {
        let listed =
            handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/list" })).unwrap();
        let uris: Vec<&str> = listed["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|resource| resource["uri"].as_str().unwrap())
            .collect();
        assert_eq!(uris, vec![REFERENCE_URI, DIAGNOSTICS_URI]);

        let reference = handle(
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/read", "params": { "uri": REFERENCE_URI } }),
        )
        .unwrap();
        let served = reference["result"]["contents"][0]["text"].as_str().unwrap();
        assert_eq!(served, crate::REFERENCE, "the reference is served verbatim");
        assert_eq!(
            reference["result"]["contents"][0]["mimeType"],
            "text/markdown"
        );

        let codes = handle(
            &json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": { "uri": DIAGNOSTICS_URI } }),
        )
        .unwrap();
        let table = codes["result"]["contents"][0]["text"].as_str().unwrap();
        for code in ["ZM110", "ZM201", "ZM301", "ZM421", "ZM500", "ZM900"] {
            assert!(table.contains(&format!("| `{code}` |")), "{code} missing");
        }

        // 没有的 URI 是错误，不是空内容。
        let unknown = handle(
            &json!({ "jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": { "uri": "zamak://nope" } }),
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
        assert!(unknown.get("result").is_none());

        // initialize 要声明支持 resources，客户端才会去列。
        let init =
            handle(&json!({ "jsonrpc": "2.0", "id": 5, "method": "initialize", "params": {} }))
                .unwrap();
        assert!(
            init["result"]["capabilities"]["resources"].is_object(),
            "{init}"
        );
    }

    #[test]
    fn explain_reads_the_diagnostics_appendix() {
        let answer = explain("zm301");
        let body = answer["content"][0]["text"].as_str().unwrap();
        assert!(
            body.starts_with("ZM301 (ownership): A value used after it was moved."),
            "{body}"
        );
        assert!(body.contains("copy(&x)"));
        assert_eq!(explain("ZM000")["isError"], true);
    }

    #[test]
    fn docs_list_modules_and_find_sections() {
        let string = docs("string");
        assert!(string["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("string.split("));
        let whole = docs("");
        assert!(whole["content"][0]["text"].as_str().unwrap().len() > 2000);
        let section = docs("catch");
        assert!(section["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("catch"));
    }

    #[test]
    fn notifications_get_no_reply_and_unknown_methods_an_error() {
        assert!(
            handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none()
        );
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": 7, "method": "nope" })).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
        let tools = handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).unwrap();
        let names: Vec<&str> = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["check", "build", "run", "test", "format", "explain", "docs", "hover"]
        );
    }

    /// `hover` 工具：给文件、行、列，返回那里的类型或签名。行、列都从 1 起。
    #[test]
    fn hover_reports_types_and_signatures() {
        let root = std::env::temp_dir().join(format!("zam-mcp-hover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = "\
struct Circle:
    radius: f64

fn make() -> Circle:
    Circle(radius: 1.0)

fn main():
    let thing = make()
    println(thing.radius)
";
        let file = root.join("main.zm");
        std::fs::write(&file, source).unwrap();
        let path = file.to_str().unwrap();

        let body = |answer: Value| answer["content"][0]["text"].as_str().unwrap().to_owned();
        // 第 8 行 `    let thing = make()`：`thing` 的类型是检查器推断出的 `Circle`。
        let local = hover(path, Some(8), Some(10));
        assert_eq!(body(local.clone()), "thing: Circle", "{local}");
        // 第 9 行 `    println(thing.radius)`：`thing` 在第 13–17 列，仍是 `Circle`。
        assert_eq!(body(hover(path, Some(9), Some(14))), "thing: Circle");
        // `make` 是函数，给出签名。
        let function = body(hover(path, Some(8), Some(18)));
        assert!(function.contains("fn make() -> Circle"), "{function}");
        // 结构体声明处给出它的定义。
        let declared = body(hover(path, Some(1), Some(9)));
        assert!(declared.contains("struct Circle"), "{declared}");
        // 少了行列、或者行列不对，都要说清楚。
        assert_eq!(hover(path, None, Some(3))["isError"], true);
        assert_eq!(hover(path, Some(0), Some(3))["isError"], true);
        let missing = body(hover(path, Some(200), Some(3)));
        assert!(missing.contains("nothing declared"), "{missing}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
