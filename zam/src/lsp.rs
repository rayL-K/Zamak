//! `zam lsp`：给编辑器用的语言服务器（Language Server Protocol，经 stdin/stdout）。
//!
//! 第一版提供：
//! - 实时报错：打开、修改、保存文件时检查整个项目（没保存的内容也算，见 `project::set_overlay`），
//!   按文件推送诊断；文字与终端一样跟随 `ZAMAK_LANG`。
//! - 补全：关键字、内置函数、本文件的声明与局部变量、`模块.` 后面的函数、`值.` 后面的方法与字段。
//! - 悬停：函数签名、结构体与枚举的定义、标准库函数的类型。
//! - 跳到定义：本文件与项目里其他模块的顶层声明，以及局部变量。
//! - 格式化：与 `zam fmt` 相同。
//!
//! 补全、悬停与跳转按文字扫描声明，不依赖完整的类型检查：代码写到一半、还不能通过解析时也能用。

use crate::{compiler, native, project, zh};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

pub fn run() -> project::Result<()> {
    let (sender, messages) = mpsc::channel::<Value>();
    // 单独的线程读 stdin：主线程一次处理完已经到达的所有消息再检查，连续打字时只检查一次。
    std::thread::spawn(move || {
        let mut input = BufReader::new(std::io::stdin().lock());
        while let Some(message) = read_message(&mut input) {
            if sender.send(message).is_err() {
                break;
            }
        }
    });
    let mut server = Server::default();
    while let Ok(message) = messages.recv() {
        let mut batch = vec![message];
        batch.extend(messages.try_iter());
        for message in batch {
            if !server.handle(&message) {
                return Ok(());
            }
        }
        if server.dirty {
            server.dirty = false;
            server.check();
        }
    }
    Ok(())
}

fn read_message(input: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let mut body = vec![0; length?];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn send(message: &Value) {
    let body = message.to_string();
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

#[derive(Default)]
struct Server {
    /// 打开的文档：URI → 文本。
    documents: BTreeMap<String, String>,
    dirty: bool,
    shutdown: bool,
}

impl Server {
    /// 处理一条消息；收到 exit 时返回 false。
    fn handle(&mut self, message: &Value) -> bool {
        let method = message["method"].as_str().unwrap_or_default();
        let params = &message["params"];
        let id = message.get("id").cloned();
        let result = match method {
            "initialize" => Some(json!({
                "capabilities": {
                    "textDocumentSync": { "openClose": true, "change": 1, "save": true },
                    "completionProvider": { "triggerCharacters": ["."] },
                    "hoverProvider": true,
                    "definitionProvider": true,
                    "signatureHelpProvider": { "triggerCharacters": ["(", ","] },
                    "renameProvider": { "prepareProvider": true },
                    "referencesProvider": true,
                    "documentFormattingProvider": true,
                },
                "serverInfo": { "name": "zam", "version": env!("CARGO_PKG_VERSION") },
            })),
            "shutdown" => {
                self.shutdown = true;
                Some(Value::Null)
            }
            "exit" => return false,
            "textDocument/didOpen" => {
                let document = &params["textDocument"];
                self.update(document["uri"].as_str(), document["text"].as_str());
                None
            }
            "textDocument/didChange" => {
                // 只声明了全量同步：最后一项就是完整的新文本。
                let text = params["contentChanges"]
                    .as_array()
                    .and_then(|changes| changes.last())
                    .and_then(|change| change["text"].as_str());
                self.update(params["textDocument"]["uri"].as_str(), text);
                None
            }
            "textDocument/didSave" => {
                self.dirty = true;
                None
            }
            "textDocument/didClose" => {
                if let Some(uri) = params["textDocument"]["uri"].as_str() {
                    self.documents.remove(uri);
                    project::set_overlay(&uri_to_path(uri), None);
                    publish(uri, Vec::new());
                }
                None
            }
            "textDocument/completion" => Some(self.request(params, completion)),
            "textDocument/hover" => Some(self.request(params, hover)),
            "textDocument/definition" => Some(self.request(params, definition)),
            "textDocument/signatureHelp" => Some(self.request(params, signature_help)),
            "textDocument/references" => Some(self.with_documents(params, references)),
            "textDocument/prepareRename" => Some(self.with_documents(params, prepare_rename)),
            "textDocument/rename" => Some(self.with_documents(params, rename)),
            "textDocument/formatting" => Some(self.formatting(params)),
            _ if id.is_some() => {
                send(
                    &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("unsupported method {method}") } }),
                );
                None
            }
            _ => None,
        };
        if let (Some(id), Some(result)) = (id, result) {
            send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        }
        true
    }

    fn update(&mut self, uri: Option<&str>, text: Option<&str>) {
        if let (Some(uri), Some(text)) = (uri, text) {
            self.documents.insert(uri.to_owned(), text.to_owned());
            project::set_overlay(&uri_to_path(uri), Some(text.to_owned()));
            self.dirty = true;
        }
    }

    fn request(&self, params: &Value, answer: fn(&Context) -> Value) -> Value {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
        let Some(text) = self.documents.get(uri) else {
            return Value::Null;
        };
        let line = params["position"]["line"].as_u64().unwrap_or(0) as usize;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as usize;
        let lines: Vec<&str> = text.lines().collect();
        let current = lines.get(line).copied().unwrap_or("");
        let column = char_index(current, character);
        let context = Context {
            uri,
            path: uri_to_path(uri),
            text,
            lines,
            line,
            column,
        };
        answer(&context)
    }

    fn with_documents(
        &self,
        params: &Value,
        answer: fn(&BTreeMap<String, String>, &Value) -> Value,
    ) -> Value {
        answer(&self.documents, params)
    }

    fn formatting(&self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
        let Some(text) = self.documents.get(uri) else {
            return Value::Null;
        };
        // 与 zam fmt 一样：语法不成立的代码不格式化。
        if compiler::parse(&compiler::expand_tabs(text)).is_err() {
            return Value::Null;
        }
        match compiler::format(text) {
            Ok(formatted) if &formatted != text => {
                let end = text.lines().count() + 1;
                json!([{ "range": range(0, 0, end, 0), "newText": formatted }])
            }
            _ => json!([]),
        }
    }

    /// 检查每个打开的文档所在的项目，把诊断按文件推送给编辑器。
    fn check(&self) {
        let mut inputs: BTreeMap<PathBuf, Vec<(&String, PathBuf)>> = BTreeMap::new();
        for uri in self.documents.keys() {
            let path = uri_to_path(uri);
            inputs
                .entry(project_input(&path))
                .or_default()
                .push((uri, canonical(&path)));
        }
        for (input, documents) in inputs {
            let entry = input.display().to_string();
            // 编译器内部的意外不该让整个服务器退出：当作这次没有结果。
            let outcome = std::panic::catch_unwind(|| {
                project::Project::load(&input).and_then(|project| project.check())
            });
            let Ok(outcome) = outcome else { continue };
            let mut found: BTreeMap<&String, Vec<Value>> = BTreeMap::new();
            if let Err(error) = outcome {
                for (_, diagnostic) in crate::diagnostics(&entry, &error) {
                    let file = canonical(Path::new(&diagnostic.file));
                    let Some((uri, _)) = documents.iter().find(|(_, path)| *path == file) else {
                        continue;
                    };
                    let text = &self.documents[*uri];
                    found
                        .entry(uri)
                        .or_default()
                        .push(lsp_diagnostic(&diagnostic, text));
                }
            }
            for (uri, _) in &documents {
                publish(uri, found.remove(uri).unwrap_or_default());
            }
        }
    }
}

fn publish(uri: &str, diagnostics: Vec<Value>) {
    send(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri, "diagnostics": diagnostics },
    }));
}

fn lsp_diagnostic(diagnostic: &crate::Diagnostic, text: &str) -> Value {
    let line = diagnostic.line.unwrap_or(1).saturating_sub(1);
    let source = text.lines().nth(line).unwrap_or("");
    let chars: Vec<char> = source.chars().collect();
    let start = diagnostic
        .column
        .unwrap_or(1)
        .saturating_sub(1)
        .min(chars.len());
    // 语法错误划一个记号，其余错误从语句起点划到行尾（与终端的下划线一致）。
    let end = if diagnostic.category == "syntax" {
        let mut end = start;
        while chars
            .get(end)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            end += 1;
        }
        end.max(start + 1).min(chars.len().max(start + 1))
    } else {
        let mut end = chars.len();
        while end > start && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        end.max(start + 1)
    };
    let mut message = zh::text(&diagnostic.message);
    if let Some(suggestion) = diagnostic.suggestion {
        let colon = if zh::enabled() { "：" } else { ": " };
        message.push_str(&format!(
            "\n{}{colon}{}",
            zh::word("help"),
            zh::text(suggestion)
        ));
    }
    json!({
        "range": range(line, utf16(source, start), line, utf16(source, end)),
        "severity": 1,
        "code": diagnostic.code,
        "source": "zam",
        "message": message,
    })
}

fn range(line: usize, character: usize, end_line: usize, end_character: usize) -> Value {
    json!({
        "start": { "line": line, "character": character },
        "end": { "line": end_line, "character": end_character },
    })
}

/// 第 `chars` 个字符之前的 UTF-16 长度（LSP 默认按 UTF-16 计列）。
fn utf16(line: &str, chars: usize) -> usize {
    line.chars().take(chars).map(char::len_utf16).sum()
}

/// UTF-16 列号 → 字符下标。
fn char_index(line: &str, units: usize) -> usize {
    let mut count = 0;
    for (index, c) in line.chars().enumerate() {
        if count >= units {
            return index;
        }
        count += c.len_utf16();
    }
    line.chars().count()
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 文件所属的项目：往上找到的第一个 `zam.toml`；没有就单独检查这个文件。
fn project_input(path: &Path) -> PathBuf {
    let mut dir = path.parent();
    while let Some(current) = dir {
        if current.join("zam.toml").is_file() {
            return current.to_path_buf();
        }
        dir = current.parent();
    }
    path.to_path_buf()
}

fn uri_to_path(uri: &str) -> PathBuf {
    let raw = uri.strip_prefix("file://").unwrap_or(uri);
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&raw[index + 1..index + 3], 16) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    let path = String::from_utf8_lossy(&decoded).into_owned();
    // Windows：`/C:/dir/a.zm` → `C:/dir/a.zm`。
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => path[1..].to_owned(),
        _ => path,
    };
    PathBuf::from(path)
}

fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_owned();
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

// ---------------------------------------------------------------------------
// 补全、悬停与跳转：按文字扫描声明。

struct Context<'a> {
    uri: &'a str,
    path: PathBuf,
    text: &'a str,
    lines: Vec<&'a str>,
    line: usize,
    /// 光标所在的字符下标。
    column: usize,
}

impl Context<'_> {
    fn before_cursor(&self) -> String {
        self.lines
            .get(self.line)
            .map(|line| line.chars().take(self.column).collect())
            .unwrap_or_default()
    }

    /// 光标处的名字，包括前面用点连着的部分：`geo.area` → (Some("geo"), "area")。
    fn word(&self) -> Option<(Option<String>, String)> {
        let chars: Vec<char> = self.lines.get(self.line)?.chars().collect();
        let ident = |c: &char| c.is_alphanumeric() || *c == '_';
        let mut start = self.column.min(chars.len());
        while start > 0 && ident(&chars[start - 1]) {
            start -= 1;
        }
        let mut end = self.column.min(chars.len());
        while end < chars.len() && ident(&chars[end]) {
            end += 1;
        }
        if start == end {
            return None;
        }
        let name: String = chars[start..end].iter().collect();
        let owner = (start >= 2 && chars[start - 1] == '.').then(|| {
            let mut from = start - 1;
            while from > 0 && ident(&chars[from - 1]) {
                from -= 1;
            }
            chars[from..start - 1].iter().collect::<String>()
        });
        Some((owner.filter(|owner| !owner.is_empty()), name))
    }
}

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "elif", "else", "while", "for", "in", "return", "break", "continue",
    "match", "struct", "enum", "use", "pub", "const", "extern", "fail", "catch", "and", "or",
    "not", "true", "false", "self", "assert",
];
const GLOBALS: &[(&str, &str)] = &[
    ("println", "fn println(value)"),
    ("print", "fn print(value)"),
    ("len", "fn len(items) -> i64"),
    ("range", "fn range(n) / range(start, end)"),
    ("abs", "fn abs(x: i64) -> i64 / abs(x: f64) -> f64"),
    ("min", "fn min(a, b)"),
    ("max", "fn max(a, b)"),
    ("copy", "fn copy(&value) -> value"),
    ("i64", "fn i64(x: f64) -> i64"),
    ("f64", "fn f64(n: i64) -> f64"),
];
const LIST_METHODS: &[&str] = &[
    "len", "push", "pop", "insert", "remove", "swap", "reverse", "clear", "extend", "sort",
    "sorted", "sort_by", "contains", "index_of", "slice", "sum", "min", "max",
];
const MAP_METHODS: &[&str] = &[
    "len", "contains", "get", "remove", "keys", "values", "clear",
];

#[derive(Clone)]
struct Declaration {
    kind: &'static str,
    name: String,
    line: usize,
    /// 声明那一行（去掉行尾的 `:`），悬停时显示。
    signature: String,
    /// 结构体的字段、枚举的变体、类型里的方法。
    members: Vec<(String, String)>,
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// 扫描顶层声明（以及结构体、枚举里的字段、变体与方法）。代码写到一半也能用。
fn declarations(text: &str) -> Vec<Declaration> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (number, line) in lines.iter().enumerate() {
        if indent(line) != 0 {
            continue;
        }
        let Some((kind, name)) = declaration_head(line) else {
            continue;
        };
        let mut members = Vec::new();
        if kind == "struct" || kind == "enum" {
            // 成员与第一个成员同一缩进；更深的是方法体。
            let mut level = None;
            for inner in lines.iter().skip(number + 1) {
                if inner.trim().is_empty() || inner.trim_start().starts_with('#') {
                    continue;
                }
                if indent(inner) == 0 {
                    break;
                }
                if *level.get_or_insert(indent(inner)) != indent(inner) {
                    continue;
                }
                let inner = inner.trim();
                if let Some((_, method)) = declaration_head(inner) {
                    members.push((method, signature(inner)));
                } else {
                    let name: String = inner
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        members.push((name, inner.to_owned()));
                    }
                }
            }
        }
        found.push(Declaration {
            kind,
            name,
            line: number,
            signature: signature(line),
            members,
        });
    }
    found
}

fn signature(line: &str) -> String {
    let line = line.trim();
    // 同一行写了函数体（`fn f(): 1`）时只保留到冒号前；类型里的 `{K: V}` 与 `fn(...)` 不算。
    let mut depth = 0;
    for (index, c) in line.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ':' if depth == 0 => return line[..index].trim_end().to_owned(),
            _ => {}
        }
    }
    line.to_owned()
}

fn declaration_head(line: &str) -> Option<(&'static str, String)> {
    let line = line.trim_start();
    let line = line.strip_prefix("pub ").unwrap_or(line);
    let line = line.strip_prefix("extern ").unwrap_or(line);
    let (kind, rest) = [
        ("fn", "fn "),
        ("struct", "struct "),
        ("enum", "enum "),
        ("const", "const "),
    ]
    .iter()
    .find_map(|(kind, prefix)| line.strip_prefix(prefix).map(|rest| (*kind, rest)))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some((kind, name))
}

/// 光标之前出现过的局部名字：参数、let、for 的循环变量、catch 的错误名。
fn locals(lines: &[&str], before: usize) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    for (number, line) in lines.iter().enumerate().take(before + 1) {
        let trimmed = line.trim_start();
        let mut push = |text: &str| {
            for name in text.split(',') {
                let name: String = name
                    .trim()
                    .trim_start_matches("mut ")
                    .trim_start_matches('(')
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() && name != "_" {
                    found.push((name, number));
                }
            }
        };
        if let Some(rest) = trimmed.strip_prefix("let ") {
            let end = rest.find(['=', ':']).unwrap_or(rest.len());
            push(&rest[..end]);
        } else if let Some(rest) = trimmed.strip_prefix("for ") {
            if let Some((names, _)) = rest.split_once(" in ") {
                push(names);
            }
        } else if let Some((_, name)) = trimmed.split_once(" catch ") {
            push(name.trim_end_matches(':'));
        }
        if declaration_head(trimmed).is_some_and(|(kind, _)| kind == "fn") {
            if let (Some(open), Some(close)) = (trimmed.find('('), trimmed.find(')')) {
                for param in trimmed[open + 1..close].split(',') {
                    push(param.split(':').next().unwrap_or(""));
                }
            }
        }
    }
    found
}

/// `use` 行：别名（路径的最后一段）→ 路径。
fn imports(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("use "))
        .map(|path| {
            let path = path.trim();
            (
                path.rsplit('/').next().unwrap_or(path).to_owned(),
                path.to_owned(),
            )
        })
        .collect()
}

/// 标准库模块的函数：(名字, 签名)。
pub(crate) fn std_functions(module: &str) -> Vec<(String, String)> {
    let prefix = format!("std/{module}:");
    native::builtin_functions()
        .filter_map(|(name, function)| {
            let short = name.strip_prefix(&prefix)?.to_owned();
            let params: Vec<String> = function.params.iter().map(|(_, ty)| show(*ty)).collect();
            let mut text = format!("{module}.{short}({})", params.join(", "));
            if function.result != compiler::Type::Unit {
                text.push_str(&format!(" -> {}", show(function.result)));
            }
            if function.error.is_some() {
                text.push_str(if function.result == compiler::Type::Unit {
                    " -> !string"
                } else {
                    "!string"
                });
            }
            Some((short, text))
        })
        .collect()
}

fn show(ty: compiler::Type) -> String {
    use compiler::{Elem, Type};
    let elem = |elem: Elem| match elem {
        Elem::Int => "i64",
        Elem::Float => "f64",
        Elem::Bool => "bool",
        Elem::Str => "string",
        _ => "…",
    };
    match ty {
        Type::Owned => "string".into(),
        Type::Shared => "&string".into(),
        Type::Mutable => "&mut string".into(),
        Type::Unit => "()".into(),
        Type::Int => "i64".into(),
        Type::Float => "f64".into(),
        Type::Bool => "bool".into(),
        Type::List(e) => format!("[{}]", elem(e)),
        Type::ListRef(e, mutable) => format!("&{}[{}]", if mutable { "mut " } else { "" }, elem(e)),
        Type::Map(k, v) => format!("{{{}: {}}}", elem(k), elem(v)),
        Type::MapRef(k, v, mutable) => {
            format!(
                "&{}{{{}: {}}}",
                if mutable { "mut " } else { "" },
                elem(k),
                elem(v)
            )
        }
        _ => "…".into(),
    }
}

const STD_MODULES: &[&str] = &[
    "io", "fmt", "string", "fs", "env", "math", "time", "random", "path",
];

/// 项目里另一个模块的源码（打开的文档优先），按 `use` 的路径找：`modules 目录/路径.zm`。
fn module_source(context: &Context, path: &str) -> Option<(PathBuf, String)> {
    let root = project_input(&context.path);
    let base = if root.is_dir() {
        let manifest = std::fs::read_to_string(root.join("zam.toml")).ok()?;
        let config: toml::Value = toml::from_str(&manifest).ok()?;
        let modules = config.get("project")?.get("modules")?.as_str()?.to_owned();
        root.join(modules)
    } else {
        context.path.parent()?.to_path_buf()
    };
    let file = base.join(format!("{path}.zm"));
    let text = project::read_overlay(&file).or_else(|| std::fs::read_to_string(&file).ok())?;
    Some((file, text))
}

fn item(label: &str, kind: u32, detail: &str) -> Value {
    json!({ "label": label, "kind": kind, "detail": detail })
}

// LSP 的 CompletionItemKind。
const METHOD: u32 = 2;
const FUNCTION: u32 = 3;
const FIELD: u32 = 5;
const VARIABLE: u32 = 6;
const MODULE: u32 = 9;
const KEYWORD: u32 = 14;
const STRUCT: u32 = 22;
const ENUM_MEMBER: u32 = 20;
const CONSTANT: u32 = 21;

fn kind_of(declaration: &Declaration) -> u32 {
    match declaration.kind {
        "fn" => FUNCTION,
        "const" => CONSTANT,
        _ => STRUCT,
    }
}

fn completion(context: &Context) -> Value {
    let before = context.before_cursor();
    let trimmed = before.trim_start();
    let mut items = Vec::new();
    // `use std/...`：模块路径。
    if let Some(partial) = trimmed.strip_prefix("use ") {
        if !partial.contains(' ') {
            for module in STD_MODULES {
                items.push(item(&format!("std/{module}"), MODULE, "standard library"));
            }
            return json!(items);
        }
    }
    // `名字.`：模块函数、类型的变体与方法、值的方法与字段。
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let owner = before
        .trim_end_matches(ident)
        .strip_suffix('.')
        .map(|head| &head[head.trim_end_matches(ident).len()..])
        .unwrap_or_default();
    if !owner.is_empty() {
        return json!(member_items(context, owner));
    }
    for keyword in KEYWORDS {
        items.push(item(keyword, KEYWORD, "keyword"));
    }
    for (name, detail) in GLOBALS {
        items.push(item(name, FUNCTION, detail));
    }
    for declaration in declarations(context.text) {
        items.push(item(
            &declaration.name,
            kind_of(&declaration),
            &declaration.signature,
        ));
    }
    for (alias, path) in imports(context.text) {
        items.push(item(&alias, MODULE, &format!("use {path}")));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (name, _) in locals(&context.lines, context.line).into_iter().rev() {
        if seen.insert(name.clone()) {
            items.push(item(&name, VARIABLE, "local"));
        }
    }
    json!(items)
}

fn member_items(context: &Context, owner: &str) -> Vec<Value> {
    let mut items = Vec::new();
    // 导入的模块：标准库函数或项目模块的 pub 声明。
    if let Some(path) = imports(context.text).get(owner) {
        if let Some(module) = path.strip_prefix("std/") {
            for (name, signature) in std_functions(module) {
                items.push(item(&name, FUNCTION, &signature));
            }
            if module == "math" {
                for constant in ["pi", "e", "inf"] {
                    items.push(item(constant, CONSTANT, "f64"));
                }
            }
        } else if let Some((_, text)) = module_source(context, path) {
            for declaration in declarations(&text) {
                if text
                    .lines()
                    .nth(declaration.line)
                    .is_some_and(|line| line.starts_with("pub "))
                {
                    items.push(item(
                        &declaration.name,
                        kind_of(&declaration),
                        &declaration.signature,
                    ));
                }
            }
        }
        return items;
    }
    let declarations = declarations(context.text);
    // 类型名：枚举变体与关联函数。
    if let Some(declaration) = declarations
        .iter()
        .find(|d| d.name == owner && d.kind != "fn")
    {
        for (name, detail) in &declaration.members {
            let kind = if detail.starts_with("fn ") || detail.starts_with("pub fn ") {
                METHOD
            } else {
                ENUM_MEMBER
            };
            items.push(item(name, kind, detail));
        }
        return items;
    }
    // 值：按声明的类型给出方法与字段；看不出类型时全部列出。
    let kind = value_type(context, owner);
    let string = || {
        std_functions("string")
            .into_iter()
            .map(|(name, signature)| item(&name, METHOD, &signature))
            .collect::<Vec<_>>()
    };
    match kind.as_deref() {
        Some("string") => items.extend(string()),
        Some("list") => items.extend(LIST_METHODS.iter().map(|m| item(m, METHOD, "list method"))),
        Some("map") => items.extend(MAP_METHODS.iter().map(|m| item(m, METHOD, "map method"))),
        Some(name) => {
            if let Some(declaration) = declarations.iter().find(|d| d.name == name) {
                for (member, detail) in &declaration.members {
                    let method = detail.starts_with("fn ") || detail.starts_with("pub fn ");
                    items.push(item(member, if method { METHOD } else { FIELD }, detail));
                }
            }
        }
        None => {
            items.extend(LIST_METHODS.iter().map(|m| item(m, METHOD, "list method")));
            items.extend(
                MAP_METHODS
                    .iter()
                    .filter(|m| !LIST_METHODS.contains(m))
                    .map(|m| item(m, METHOD, "map method")),
            );
            items.extend(string());
        }
    }
    items
}

/// 从声明里看出变量的类型：`string`、`list`、`map` 或结构体名。
fn value_type(context: &Context, name: &str) -> Option<String> {
    let classify = |ty: &str| {
        let ty = ty
            .trim()
            .trim_start_matches('&')
            .trim_start_matches("mut ")
            .trim();
        if ty == "string" {
            Some("string".to_owned())
        } else if ty.starts_with('[') {
            Some("list".to_owned())
        } else if ty.starts_with('{') {
            Some("map".to_owned())
        } else {
            let head: String = ty
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            (!head.is_empty() && head.starts_with(char::is_uppercase)).then_some(head)
        }
    };
    for line in context.lines.iter().take(context.line + 1).rev() {
        let trimmed = line.trim_start();
        // 参数：`name: T`
        if declaration_head(trimmed).is_some_and(|(kind, _)| kind == "fn") {
            if let (Some(open), Some(close)) = (trimmed.find('('), trimmed.rfind(')')) {
                for param in split_top(&trimmed[open + 1..close]) {
                    if let Some((param, ty)) = param.split_once(':') {
                        if param.trim().trim_start_matches("mut ") == name {
                            return classify(ty);
                        }
                    }
                }
            }
        }
        let Some(rest) = trimmed.strip_prefix("let ") else {
            continue;
        };
        let rest = rest.strip_prefix("mut ").unwrap_or(rest);
        let Some(after) = rest.strip_prefix(name) else {
            continue;
        };
        let after = after.trim_start();
        if let Some(ty) = after.strip_prefix(':') {
            return classify(ty.split('=').next().unwrap_or(ty));
        }
        if let Some(value) = after.strip_prefix('=') {
            let value = value.trim();
            return if value.starts_with('"') || value.starts_with("f\"") {
                Some("string".into())
            } else if value.starts_with('[') {
                Some("list".into())
            } else if value.starts_with('{') {
                Some("map".into())
            } else {
                classify(value)
            };
        }
    }
    None
}

/// 按顶层逗号拆参数（类型里的 `{K: V}`、`fn(a, b)` 不拆）。
fn split_top(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (index, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn markdown(code: &str) -> Value {
    json!({ "contents": { "kind": "markdown", "value": format!("```zamak\n{code}\n```") } })
}

fn hover(context: &Context) -> Value {
    let Some((owner, name)) = context.word() else {
        return Value::Null;
    };
    if let Some(owner) = owner {
        if let Some(path) = imports(context.text).get(&owner) {
            if let Some(module) = path.strip_prefix("std/") {
                if let Some((_, signature)) =
                    std_functions(module).into_iter().find(|(n, _)| *n == name)
                {
                    return markdown(&signature);
                }
            } else if let Some((_, text)) = module_source(context, path) {
                if let Some(declaration) = declarations(&text).into_iter().find(|d| d.name == name)
                {
                    return markdown(&describe(&declaration));
                }
            }
            return Value::Null;
        }
        for declaration in declarations(context.text) {
            if declaration.name == owner {
                if let Some((_, detail)) = declaration.members.iter().find(|(n, _)| *n == name) {
                    return markdown(detail);
                }
            }
        }
        return Value::Null;
    }
    if let Some(declaration) = declarations(context.text)
        .into_iter()
        .find(|d| d.name == name)
    {
        return markdown(&describe(&declaration));
    }
    if let Some((_, detail)) = GLOBALS.iter().find(|(n, _)| *n == name) {
        return markdown(detail);
    }
    if let Some(path) = imports(context.text).get(&name) {
        return markdown(&format!("use {path}"));
    }
    Value::Null
}

fn describe(declaration: &Declaration) -> String {
    if declaration.members.is_empty() || declaration.kind == "fn" {
        return declaration.signature.clone();
    }
    let mut text = format!("{}:", declaration.signature);
    for (_, member) in &declaration.members {
        text.push_str(&format!("\n    {member}"));
    }
    text
}

/// 光标处的调用：从光标往前找到还没闭合的 `(`，取出被调用的名字与前面已经写了几个参数。
/// 返回（被调用的名字, 当前是第几个参数）。
fn open_call(context: &Context) -> Option<(String, usize)> {
    // 只看光标之前的文本；跨行调用也要能找到，所以把前面的行拼起来。
    let mut before = String::new();
    for (index, line) in context.lines.iter().enumerate() {
        if index < context.line {
            before.push_str(line);
            before.push('\n');
        } else {
            before.extend(line.chars().take(context.column));
            break;
        }
    }
    let chars: Vec<char> = before.chars().collect();
    // 从后往前找未配对的 `(`；顺便数这一层里已经出现了几个逗号。
    let mut depth = 0i32;
    let mut commas = 0usize;
    for position in (0..chars.len()).rev() {
        match chars[position] {
            ')' | ']' | '}' => depth += 1,
            // 遇到没配对的闭括号就说明光标不在这次调用里。
            '(' | '[' | '{' if depth > 0 => depth -= 1,
            '(' => {
                // 找到调用，往左取名字（可以带 `模块.` 前缀）。
                let mut start = position;
                while start > 0
                    && (chars[start - 1].is_alphanumeric()
                        || chars[start - 1] == '_'
                        || chars[start - 1] == '.')
                {
                    start -= 1;
                }
                let name: String = chars[start..position].iter().collect();
                if name.is_empty()
                    || !name
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_alphabetic() || c == '_')
                {
                    return None;
                }
                // 括号里的逗号就是已经写好的参数个数（跳过嵌套的括号）。
                let mut inner = 0i32;
                for c in &chars[position + 1..] {
                    match c {
                        '(' | '[' | '{' => inner += 1,
                        ')' | ']' | '}' => inner -= 1,
                        ',' if inner == 0 => commas += 1,
                        _ => {}
                    }
                }
                return Some((name, commas));
            }
            _ => {}
        }
    }
    None
}

/// 把签名拆成（前缀, 每个参数, 返回类型）：`fn f(a: i64, b: &string) -> bool` →
/// （`fn f(`, [`a: i64`, `b: &string`], `) -> bool`）。
fn signature_parts(signature: &str) -> Option<(String, Vec<String>, String)> {
    let open = signature.find('(')?;
    let mut depth = 0i32;
    let mut close = None;
    for (offset, c) in signature[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let inner = &signature[open + 1..close];
    // 参数按顶层逗号切开（`{K: V}`、`fn(...)`、元组里的逗号不算）。
    let mut params = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (index, c) in inner.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                params.push(inner[start..index].trim().to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = inner[start..].trim();
    if !tail.is_empty() {
        params.push(tail.to_owned());
    }
    Some((
        signature[..=open].to_owned(),
        params,
        signature[close..].to_owned(),
    ))
}

/// 本地或标准库函数的签名：`模块.名` 优先查标准库，否则查本文档与 `use` 进来的模块。
fn declared_signature(context: &Context, name: &str) -> Option<String> {
    let (owner, short) = match name.split_once('.') {
        Some((owner, short)) => (Some(owner), short),
        None => (None, name),
    };
    if let Some(owner) = owner {
        // 结构体方法：`p.norm()` 里的 `norm` 属于接收者类型，名字里没有类型信息，按方法名找。
        for declaration in declarations(context.text) {
            if declaration.name == owner {
                if let Some((_, member)) = declaration.members.iter().find(|(n, _)| n == short) {
                    return Some(member.clone());
                }
            }
        }
        if let Some(path) = imports(context.text).get(owner) {
            if let Some(module) = path.strip_prefix("std/") {
                return std_functions(module)
                    .into_iter()
                    .find(|(n, _)| n == short)
                    .map(|(_, signature)| signature);
            }
            if let Some((_, text)) = module_source(context, path) {
                return declarations(&text)
                    .into_iter()
                    .find(|d| d.name == short)
                    .map(|d| d.signature);
            }
        }
        // 方法调用的接收者不是模块名：按方法名在本文档的类型里找。
        for declaration in declarations(context.text) {
            if let Some((_, member)) = declaration.members.iter().find(|(n, _)| n == short) {
                if declaration.kind != "fn" {
                    return Some(member.clone());
                }
            }
        }
        return None;
    }
    if let Some(declaration) = declarations(context.text)
        .into_iter()
        .find(|d| d.name == short && d.kind == "fn")
    {
        return Some(declaration.signature);
    }
    if let Some((_, signature)) = GLOBALS.iter().find(|(n, _)| *n == short) {
        return Some((*signature).to_owned());
    }
    // 打开的标准库模块里找（`use std/io` 之后 `io.read_line(...)` 走上面的分支，
    // 这里是没写前缀时直接叫名字的情况）。
    for path in imports(context.text).values() {
        if let Some(module) = path.strip_prefix("std/") {
            if let Some((_, signature)) =
                std_functions(module).into_iter().find(|(n, _)| n == short)
            {
                return Some(signature);
            }
        }
    }
    None
}

/// `textDocument/signatureHelp`：输入 `foo(` 时显示参数列表，并标出当前是第几个参数。
fn signature_help(context: &Context) -> Value {
    let Some((name, active)) = open_call(context) else {
        return Value::Null;
    };
    let Some(declaration) = declared_signature(context, &name) else {
        return Value::Null;
    };
    let Some((prefix, params, tail)) = signature_parts(&declaration) else {
        return Value::Null;
    };
    // 签名里的 `; 说明` 之类的补充不算参数，只保留 `fn ...(...) -> T` 这一段。
    let label = format!("{prefix}{}{tail}", params.join(", "));
    let mut parameters = Vec::new();
    let mut offset = prefix.chars().count();
    for parameter in &params {
        let length = parameter.chars().count();
        parameters.push(json!({
            "label": [offset, offset + length],
            "documentation": json!({ "kind": "markdown", "value": parameter }),
        }));
        offset += length + ", ".len();
    }
    // 当前参数：写多了就停在最后一个，参数列表为空时给 None。
    let active_parameter = (!params.is_empty()).then(|| active.min(params.len() - 1));
    json!({
        "signatures": [{
            "label": label,
            "documentation": json!({ "kind": "markdown", "value": format!("```zamak\n{declaration}\n```") }),
            "parameters": parameters,
        }],
        "activeSignature": 0,
        "activeParameter": active_parameter,
    })
}

/// 第 `line` 行的第 `start` 个字符（0 起）处、横跨 `length` 个字符的范围（列号按 UTF-16）。
fn span(line: usize, source: &str, start: usize, length: usize) -> Value {
    let begin = utf16(source, start);
    let end = utf16(source, start + length);
    range(line, begin, line, end)
}

/// 光标所在的标识符：返回（名字, 起始字符下标, 字符数）。
fn identifier_at(context: &Context) -> Option<(String, usize, usize)> {
    let chars: Vec<char> = context.lines.get(context.line)?.chars().collect();
    let ident = |c: &char| c.is_alphanumeric() || *c == '_';
    let mut start = context.column.min(chars.len());
    while start > 0 && ident(&chars[start - 1]) {
        start -= 1;
    }
    let mut end = context.column.min(chars.len());
    while end < chars.len() && ident(&chars[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some((chars[start..end].iter().collect(), start, end - start))
}

/// 本文件里 `name` 的每次出现：按标识符边界匹配，跳过注释（`#` 之后）与字符串字面量里的文字。
fn occurrences(text: &str, name: &str) -> Vec<(usize, usize, usize)> {
    let wanted: Vec<char> = name.chars().collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut found = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut in_string = false;
        let mut escaped = false;
        let mut index = 0;
        while index < chars.len() {
            let c = chars[index];
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
                index += 1;
                continue;
            }
            if c == '"' {
                in_string = true;
                index += 1;
                continue;
            }
            // `#` 之后是注释，剩下这一行都不用看。
            if c == '#' && (index == 0 || chars[index - 1] != '\\') {
                break;
            }
            let boundary = (index == 0 || !ident(chars[index - 1]))
                && chars.get(index + wanted.len()).is_none_or(|c| !ident(*c));
            if boundary && chars[index..].starts_with(&wanted) {
                found.push((number, index, wanted.len()));
                index += wanted.len();
                continue;
            }
            index += 1;
        }
    }
    found
}

/// 光标位置的上下文（供需要文档映射的处理函数共用）。
fn context_at<'a>(text: &'a str, params: &Value) -> Option<Context<'a>> {
    let line = params["position"]["line"].as_u64().unwrap_or(0) as usize;
    let character = params["position"]["character"].as_u64().unwrap_or(0) as usize;
    let lines: Vec<&str> = text.lines().collect();
    let current = lines.get(line).copied().unwrap_or("");
    Some(Context {
        uri: "",
        path: PathBuf::new(),
        text,
        lines,
        line,
        column: char_index(current, character),
    })
}

/// `textDocument/references`：本文件里这个名字出现在哪些地方。`includeDeclaration` 为假时
/// 去掉声明那一行。跨模块的引用先不做（那要按 `use` 去解析别的模块）。
fn references(documents: &BTreeMap<String, String>, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let Some(text) = documents.get(&uri) else {
        return Value::Null;
    };
    let Some(context) = context_at(text, params) else {
        return json!([]);
    };
    let Some((name, _, _)) = identifier_at(&context) else {
        return json!([]);
    };
    let include_declaration = params["context"]["includeDeclaration"]
        .as_bool()
        .unwrap_or(true);
    let declaration = declarations(text)
        .into_iter()
        .find(|d| d.name == name)
        .map(|d| d.line);
    json!(occurrences(text, &name)
        .into_iter()
        .filter(|(number, _, _)| include_declaration || Some(*number) != declaration)
        .map(|(number, start, length)| {
            json!({ "uri": uri, "range": span(number, context.lines[number], start, length) })
        })
        .collect::<Vec<_>>())
}

/// `textDocument/prepareRename`：报告会被一起改掉的范围，编辑器据此弹出输入框；
/// 光标不在标识符上时返回 null，编辑器就不会弹出。
fn prepare_rename(documents: &BTreeMap<String, String>, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
    let Some(text) = documents.get(uri) else {
        return Value::Null;
    };
    let Some(context) = context_at(text, params) else {
        return Value::Null;
    };
    let Some((name, start, length)) = identifier_at(&context) else {
        return Value::Null;
    };
    json!({
        "range": span(context.line, context.lines[context.line], start, length),
        "placeholder": name,
    })
}

/// `textDocument/rename`：把本文件里这个名字的所有出现改成新名字。跨模块重命名留到以后。
fn rename(documents: &BTreeMap<String, String>, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let Some(text) = documents.get(&uri) else {
        return Value::Null;
    };
    let Some(new_name) = params["newName"].as_str() else {
        return Value::Null;
    };
    // 新名字必须是本语言认可的标识符（不以数字开头、不是保留字），否则会直接编不过。
    let valid = !new_name.is_empty()
        && new_name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && new_name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&new_name);
    if !valid {
        return json!({ "changes": {} });
    }
    let Some(context) = context_at(text, params) else {
        return json!({ "changes": {} });
    };
    let Some((name, _, _)) = identifier_at(&context) else {
        return json!({ "changes": {} });
    };
    let edits: Vec<Value> = occurrences(text, &name)
        .into_iter()
        .map(|(number, start, length)| {
            json!({
                "range": span(number, context.lines[number], start, length),
                "newText": new_name,
            })
        })
        .collect();
    let mut changes = serde_json::Map::new();
    changes.insert(uri, Value::Array(edits));
    json!({ "changes": changes })
}

fn location(uri: &str, line: usize, text: &str, name: &str) -> Value {
    let source = text.lines().nth(line).unwrap_or("");
    let start = source
        .find(name)
        .map(|byte| source[..byte].chars().count())
        .unwrap_or(0);
    let start = utf16(source, start);
    json!({ "uri": uri, "range": range(line, start, line, start + name.encode_utf16().count()) })
}

fn definition(context: &Context) -> Value {
    let Some((owner, name)) = context.word() else {
        return Value::Null;
    };
    if let Some(owner) = owner {
        if let Some(path) = imports(context.text).get(&owner) {
            if let Some((file, text)) = module_source(context, path) {
                if let Some(declaration) = declarations(&text).into_iter().find(|d| d.name == name)
                {
                    return location(&path_to_uri(&file), declaration.line, &text, &name);
                }
            }
        }
        return Value::Null;
    }
    // 局部变量：光标之前最近的一次声明。
    if let Some((_, line)) = locals(&context.lines, context.line)
        .into_iter()
        .rev()
        .find(|(local, line)| *local == name && *line != context.line)
    {
        return location(context.uri, line, context.text, &name);
    }
    if let Some(declaration) = declarations(context.text)
        .into_iter()
        .find(|d| d.name == name)
    {
        return location(context.uri, declaration.line, context.text, &name);
    }
    if let Some(path) = imports(context.text).get(&name) {
        if let Some((file, _)) = module_source(context, path) {
            return json!({ "uri": path_to_uri(&file), "range": range(0, 0, 0, 0) });
        }
    }
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_round_trip_through_paths() {
        assert_eq!(
            uri_to_path("file:///e%3A/My%20Code/a.zm"),
            PathBuf::from("e:/My Code/a.zm")
        );
        assert_eq!(
            uri_to_path("file:///home/me/a.zm"),
            PathBuf::from("/home/me/a.zm")
        );
        assert_eq!(
            path_to_uri(Path::new("/home/me/my code.zm")),
            "file:///home/me/my%20code.zm"
        );
        assert_eq!(
            path_to_uri(Path::new(r"C:\dir\a.zm")),
            "file:///C:/dir/a.zm"
        );
    }

    #[test]
    fn utf16_columns_count_wide_characters() {
        assert_eq!(utf16("名字 x", 3), 3);
        assert_eq!(utf16("😀x", 1), 2);
        assert_eq!(char_index("😀x", 2), 1);
    }

    #[test]
    fn declarations_and_locals_are_found_in_broken_code() {
        let text = "use std/string\n\npub struct Point:\n    x: f64\n    y: f64\n    fn norm(&self) -> f64: 0.0\n\nfn area(w: f64, h: f64) -> f64:\n    let mut total = w * h\n    for i, v in &xs:\n        total = \n";
        let found = declarations(text);
        assert_eq!(
            found.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["Point", "area"]
        );
        assert_eq!(found[0].members.len(), 3);
        assert_eq!(found[1].signature, "fn area(w: f64, h: f64) -> f64");
        let lines: Vec<&str> = text.lines().collect();
        let names: Vec<String> = locals(&lines, 10).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["w", "h", "total", "i", "v"].map(String::from));
    }
}
