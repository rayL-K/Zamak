//! `zam mcp`：按 MCP 协议（每行一条 JSON-RPC 消息）与真正的 `zam mcp` 进程对话，
//! 覆盖初始化、工具列表、检查、运行（带参数与标准输入）、测试、格式化、解释错误码与查文档。

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u64,
}

impl Client {
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let message =
            json!({ "jsonrpc": "2.0", "id": self.next, "method": method, "params": params });
        writeln!(self.input, "{message}").unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], self.next, "{reply}");
        reply["result"].clone()
    }

    fn tool(&mut self, name: &str, arguments: Value) -> (String, bool) {
        let result = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        (
            result["content"][0]["text"].as_str().unwrap().to_owned(),
            result["isError"].as_bool().unwrap(),
        )
    }
}

#[test]
fn mcp_server_exposes_the_zamak_tools() {
    let root = std::env::temp_dir().join(format!("zam-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("bad.zm"),
        "fn main():\n    let s = \"hi\"\n    let t = s\n    println(s)\n",
    )
    .unwrap();
    std::fs::write(
        root.join("echo.zm"),
        "use std/env\nuse std/io\n\nfn main() -> !string:\n    let word = env.arg(0)?\n    let mut line = \"\"\n    let got = io.read_line(&mut line)?\n    println(f\"{word} {line} {got}\")\n",
    )
    .unwrap();
    std::fs::write(
        root.join("unit.zm"),
        "fn add(a: i64, b: i64) -> i64:\n    a + b\n\nfn test_add():\n    assert add(2, 2) == 5\n",
    )
    .unwrap();
    std::fs::write(root.join("messy.zm"), "fn main():\n  println(\"hi\")   \n").unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("mcp")
        .current_dir(&root)
        .env("ZAMAK_CACHE_DIR", root.join("cache"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut client = Client {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
        next: 0,
    };

    let init = client.request("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }));
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "zamak");
    // 语言参考与错误码表作为资源提供：客户端可以挂在上下文里，不用每次调工具。
    assert!(init["capabilities"]["resources"].is_object(), "{init}");
    let resources = client.request("resources/list", json!({}));
    let uris: Vec<&str> = resources["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["uri"].as_str().unwrap())
        .collect();
    assert_eq!(uris, vec!["zamak://reference", "zamak://diagnostics"]);
    writeln!(
        client.input,
        "{}",
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
    )
    .unwrap();
    let tools = client.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["check", "build", "run", "test", "format", "explain", "docs"]
    );

    // check：与 `zam check --json` 相同的诊断。
    let (report, error) = client.tool("check", json!({ "path": "bad.zm" }));
    assert!(!error);
    let report: Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(report["diagnostics"][0]["code"], "ZM301");

    // run：程序参数与标准输入都传得进去。
    let (output, error) = client.tool(
        "run",
        json!({ "path": "echo.zm", "args": ["hello"], "stdin": "world\n" }),
    );
    assert!(!error, "{output}");
    assert!(
        output.starts_with("exit code: 0\n--- stdout\nhello world true\n"),
        "{output}"
    );

    // test：失败的断言让结果带 isError，报告里有两边的值。
    let (report, error) = client.tool("test", json!({ "path": "unit.zm" }));
    assert!(error);
    assert!(report.contains("test_add ... FAILED"), "{report}");

    // format：改写文件并列出改了哪些。
    let (formatted, _) = client.tool("format", json!({ "path": "messy.zm" }));
    assert!(formatted.starts_with("formatted:"), "{formatted}");
    assert_eq!(
        std::fs::read_to_string(root.join("messy.zm"))
            .unwrap()
            .replace("\r\n", "\n"),
        "fn main():\n    println(\"hi\")\n"
    );

    // explain 与 docs。
    let (meaning, _) = client.tool("explain", json!({ "code": "ZM301" }));
    assert!(meaning.starts_with("ZM301 (ownership)"), "{meaning}");
    let (functions, _) = client.tool("docs", json!({ "topic": "string" }));
    assert!(functions.contains("string.split("), "{functions}");

    drop(client);
    assert!(child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(&root);
}
