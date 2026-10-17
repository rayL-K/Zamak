//! `zam lsp`：按 LSP 协议与一个真正的 `zam lsp` 进程对话，覆盖实时报错（含未保存内容）、
//! 补全、悬停、跳转与格式化。

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u64,
}

impl Client {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("lsp")
            .current_dir(dir)
            .env("ZAMAK_CACHE_DIR", dir.join("cache"))
            .env("ZAMAK_LANG", "en")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
            next: 0,
        }
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.input.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn receive(&mut self) -> Value {
        let mut length = 0;
        loop {
            let mut line = String::new();
            assert!(
                self.output.read_line(&mut line).unwrap() > 0,
                "server closed stdout"
            );
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = value.parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        self.output.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.receive();
            if message["id"] == id {
                return message["result"].clone();
            }
        }
    }

    /// 下一条发给 `uri` 的诊断。
    fn diagnostics(&mut self, uri: &str) -> Vec<Value> {
        loop {
            let message = self.receive();
            if message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri
            {
                return message["params"]["diagnostics"].as_array().unwrap().clone();
            }
        }
    }
}

fn uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

fn labels(result: &Value) -> Vec<String> {
    result
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["label"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn language_server_checks_completes_and_navigates() {
    let root = std::env::temp_dir().join(format!("zam-lsp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"shapes\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/geo.zm"),
        "pub struct Point:\n    x: f64\n    y: f64\n\npub fn area(w: f64, h: f64) -> f64:\n    w * h\n",
    )
    .unwrap();
    let main = root.join("src/main.zm");
    let saved = "use geo\n\nfn main():\n    println(geo.area(2.0, 3.0))\n";
    std::fs::write(&main, saved).unwrap();
    let main_uri = uri(&main);

    let mut client = Client::start(&root);
    let init = client.request(
        "initialize",
        json!({ "processId": null, "rootUri": uri(&root), "capabilities": {} }),
    );
    assert_eq!(init["capabilities"]["hoverProvider"], true);
    assert_eq!(
        init["capabilities"]["signatureHelpProvider"]["triggerCharacters"],
        json!(["(", ","])
    );
    assert_eq!(
        init["capabilities"]["completionProvider"]["triggerCharacters"],
        json!(["."])
    );
    client.notify("initialized", json!({}));

    // 未保存的内容也检查：磁盘上的文件没有错，编辑器里的有。
    let broken = "use geo\n\nfn main():\n    let x = foo(1)\n    println(geo.area(2.0, 3.0))\n";
    client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": main_uri, "languageId": "zamak", "version": 1, "text": broken } }),
    );
    let found = client.diagnostics(&main_uri);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["code"], "ZM201");
    assert_eq!(
        found[0]["range"]["start"],
        json!({ "line": 3, "character": 4 })
    );
    assert!(
        found[0]["message"]
            .as_str()
            .unwrap()
            .contains("unknown function foo"),
        "{found:?}"
    );

    // 改好之后诊断清空。
    let fixed = "use geo\nuse std/string\n\nfn main():\n    let s = \"hi\"\n    let p = geo.Point(x: 1.0, y: 2.0)\n    println(geo.area(2.0, 3.0))\n    s.\n";
    client.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": main_uri, "version": 2 }, "contentChanges": [{ "text": saved }] }),
    );
    assert_eq!(client.diagnostics(&main_uri), Vec::<Value>::new());
    client.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": main_uri, "version": 3 }, "contentChanges": [{ "text": fixed }] }),
    );
    client.diagnostics(&main_uri);

    // 补全：`geo.` 后面是模块的 pub 声明，字符串值后面是字符串方法，空白处有关键字与局部变量。
    let at = |line: u64, character: u64| json!({ "textDocument": { "uri": main_uri }, "position": { "line": line, "character": character } });
    let module = labels(&client.request("textDocument/completion", at(6, 16)));
    assert!(
        module.contains(&"area".to_owned()) && module.contains(&"Point".to_owned()),
        "{module:?}"
    );
    let methods = labels(&client.request("textDocument/completion", at(7, 6)));
    assert!(
        methods.contains(&"upper".to_owned()) && methods.contains(&"split".to_owned()),
        "{methods:?}"
    );
    let plain = labels(&client.request("textDocument/completion", at(7, 4)));
    for expected in ["fn", "while", "println", "main", "geo", "s", "p"] {
        assert!(
            plain.contains(&expected.to_owned()),
            "{expected} missing from {plain:?}"
        );
    }

    // 悬停与跳转：`geo.area` 指向 geo.zm 里的声明。
    let hover = client.request("textDocument/hover", at(6, 17));
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("pub fn area(w: f64, h: f64) -> f64"),
        "{hover}"
    );
    let target = client.request("textDocument/definition", at(6, 17));
    assert!(
        target["uri"].as_str().unwrap().ends_with("src/geo.zm"),
        "{target}"
    );
    assert_eq!(target["range"]["start"]["line"], 4);
    let local = client.request("textDocument/definition", at(7, 4));
    assert_eq!(local["range"]["start"]["line"], 4);

    // 格式化：与 zam fmt 相同。
    let messy = "fn main():\n  println(\"hi\")   \n";
    client.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": main_uri, "version": 4 }, "contentChanges": [{ "text": messy }] }),
    );
    let edits = client.request("textDocument/formatting", json!({ "textDocument": { "uri": main_uri }, "options": { "tabSize": 4, "insertSpaces": true } }));
    assert_eq!(edits[0]["newText"], "fn main():\n    println(\"hi\")\n");

    // 签名提示：`geo.area(` 刚写完左括号时给出参数列表，第二个参数时 activeParameter 是 1。
    // 第 6 行是 `    println(geo.area(2.0, 3.0))`，`geo.area(` 的左括号在第 20 列，逗号在第 24 列。
    client.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": main_uri, "version": 5 }, "contentChanges": [{ "text": fixed }] }),
    );
    client.diagnostics(&main_uri);
    let help = client.request("textDocument/signatureHelp", at(6, 21));
    assert_eq!(help["activeSignature"], 0, "{help}");
    assert!(
        help["signatures"][0]["label"]
            .as_str()
            .unwrap()
            .contains("w: f64, h: f64"),
        "{help}"
    );
    assert_eq!(help["activeParameter"], 0, "{help}");
    let second = client.request("textDocument/signatureHelp", at(6, 25));
    assert_eq!(second["activeParameter"], 1, "{second}");

    client.request("shutdown", Value::Null);
    client.notify("exit", Value::Null);
    assert!(client.child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&root);
}

/// `textDocument/signatureHelp`：本地函数、标准库函数与结构体方法都要给出参数列表，
/// 并且按光标前的逗号标出当前参数。写完 `foo` 还没打左括号时不该有提示。
#[test]
fn signature_help_covers_local_std_and_method_calls() {
    let root = std::env::temp_dir().join(format!("zam-lsp-signature-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let source = "\
use std/io

struct Point:
    x: i64
    y: i64

    fn shifted(&self, dx: i64, dy: i64) -> Point:
        Point(x: self.x + dx, y: self.y + dy)

fn area(w: i64, h: i64) -> i64:
    w * h

fn main():
    let p = Point(x: 1, y: 2)
    let a = area(2, 3)
    let q = p.shifted(1, 1)
    let n = io.read_line(
";
    let file = root.join("main.zm");
    std::fs::write(&file, source).unwrap();
    let file_uri = uri(&file);
    let mut client = Client::start(&root);
    client.request(
        "initialize",
        json!({ "processId": null, "rootUri": uri(&root), "capabilities": {} }),
    );
    client.notify("initialized", json!({}));
    client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": file_uri, "languageId": "zamak", "version": 1, "text": source } }),
    );
    let at = |line: u64, character: u64| json!({ "textDocument": { "uri": file_uri }, "position": { "line": line, "character": character } });

    // `area(2, 3)`：左括号在第 16 列，光标在第一个参数里。
    let local = client.request("textDocument/signatureHelp", at(14, 18));
    let label = local["signatures"][0]["label"].as_str().unwrap_or("");
    assert!(label.contains("w: i64, h: i64"), "{local}");
    assert_eq!(local["activeParameter"], 0, "{local}");
    // 逗号之后是第二个参数。
    let second = client.request("textDocument/signatureHelp", at(14, 21));
    assert_eq!(second["activeParameter"], 1, "{second}");

    // 方法：`p.shifted(` 给出方法自己的参数列表。
    let method = client.request("textDocument/signatureHelp", at(15, 23));
    assert!(
        method["signatures"][0]["label"]
            .as_str()
            .unwrap_or("")
            .contains("dx: i64, dy: i64"),
        "{method}"
    );

    // 标准库：`io.read_line(` 的签名里有 &mut。
    let standard = client.request("textDocument/signatureHelp", at(16, 25));
    let label = standard["signatures"][0]["label"].as_str().unwrap_or("");
    assert!(label.contains("read_line(&mut string)"), "{standard}");
    assert_eq!(standard["activeParameter"], 0, "{standard}");

    // 光标在还没闭合的调用里（上一行的 `io.read_line(` 跨行）仍然给提示。
    // 真正不在调用里时不给提示：`fn area(` 那一行的函数头不是调用。
    let outside = client.request("textDocument/signatureHelp", at(9, 4));
    assert!(outside.is_null(), "{outside}");

    client.request("shutdown", Value::Null);
    client.notify("exit", Value::Null);
    assert!(client.child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&root);
}

/// `textDocument/references`、`prepareRename` 与 `rename`：本文件内的局部变量和函数，
/// 注释与字符串里同名的文字不算引用。
#[test]
fn references_and_rename_cover_local_names() {
    let root = std::env::temp_dir().join(format!("zam-lsp-rename-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let source = "\
fn total(n: i64) -> i64:
    n * 2

fn main():
    # total is computed twice
    let total = total(3)
    println(total)
    println(\"total\")
    println(total + 1)
";
    let file = root.join("main.zm");
    std::fs::write(&file, source).unwrap();
    let file_uri = uri(&file);
    let mut client = Client::start(&root);
    let init = client.request(
        "initialize",
        json!({ "processId": null, "rootUri": uri(&root), "capabilities": {} }),
    );
    assert_eq!(init["capabilities"]["referencesProvider"], true);
    assert_eq!(
        init["capabilities"]["renameProvider"]["prepareProvider"],
        true
    );
    client.notify("initialized", json!({}));
    client.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": file_uri, "languageId": "zamak", "version": 1, "text": source } }),
    );
    let at = |line: u64, character: u64| json!({ "textDocument": { "uri": file_uri }, "position": { "line": line, "character": character } });

    // 第 5 行 `    let total = total(3)` 里的局部变量 `total`（第 8 列起）。
    let refs = client.request(
        "textDocument/references",
        json!({ "textDocument": { "uri": file_uri }, "position": { "line": 5, "character": 9 }, "context": { "includeDeclaration": true } }),
    );
    let lines: Vec<i64> = refs
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["range"]["start"]["line"].as_i64().unwrap())
        .collect();
    // 函数声明（第 0 行）、局部变量声明（第 5 行）、局部变量在 `total(3)` 里的调用（第 5 行）、
    // 第 6 与第 8 行各一处；注释和字符串里同名的文字不算。
    assert_eq!(lines, vec![0, 5, 5, 6, 8], "{refs}");

    // 只有 `total(3)` 那处是函数调用，其余是局部变量；两者都被算上，因为都叫 total。
    let prepared = client.request("textDocument/prepareRename", at(5, 9));
    assert_eq!(prepared["placeholder"], "total", "{prepared}");
    assert_eq!(
        prepared["range"]["start"],
        json!({ "line": 5, "character": 8 })
    );

    // 改名后所有引用都换成新名字。
    let renamed = client.request(
        "textDocument/rename",
        json!({ "textDocument": { "uri": file_uri }, "position": { "line": 5, "character": 9 }, "newName": "sum" }),
    );
    let edits = renamed["changes"][&file_uri].as_array().unwrap().clone();
    assert_eq!(edits.len(), 5, "{renamed}");
    assert!(
        edits.iter().all(|edit| edit["newText"] == "sum"),
        "{renamed}"
    );

    // 保留字不能当新名字：直接拒绝，不给编辑。
    let rejected = client.request(
        "textDocument/rename",
        json!({ "textDocument": { "uri": file_uri }, "position": { "line": 5, "character": 9 }, "newName": "fn" }),
    );
    assert_eq!(rejected["changes"], json!({}), "{rejected}");

    // 空白处没有可改的名字。
    let nothing = client.request("textDocument/prepareRename", at(7, 0));
    assert!(nothing.is_null(), "{nothing}");

    client.request("shutdown", Value::Null);
    client.notify("exit", Value::Null);
    assert!(client.child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&root);
}
