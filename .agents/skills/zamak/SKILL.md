---
name: zamak
description: 编写、运行、调试 Zamak（.zm）程序，或修改 Zamak 编译器本身（Rust 前端、C 后端、IR/缓存版本、VS Code 语法、测试与验收）。当任务涉及 .zm 源码、zam 命令、examples/ 示例、Zamak 语法/标准库/错误值，或 zam/ 下的编译器改动时使用。
---

# Zamak

Zamak 是静态类型、编译到原生可执行文件的小语言，仓库在 `C:\Zamak`。`zam` 就是它自己的编译器：Rust 前端（`zam/src/compiler.rs` 词法/语法/检查）→ C11 后端（`zam/src/native.rs`）→ MSVC `cl`（Windows）或 `cc`（Unix）。生成的产物不需要安装 Zamak 或 Rust。

## 命令

- `zam check [project|file]`、`zam build ...`、`zam run ...`、`zam fmt ...`、`zam demo`；无路径参数时用当前目录；源码后缀统一 `.zm`，可直接对单个源码文件操作；`run` 也能直接执行已构建的产物。
- `zam fmt` 只规范化空白：按大括号深度把每行缩进改成 4 空格的倍数、去行尾空白、结尾留一个换行、保留原换行风格（CRLF 仍是 CRLF）。它不重建 token，所以注释和字符串内容不会丢；改过的文件路径以 `FORMAT <path>` 打到 stderr，已规范的文件不重写。语法坏掉的代码先被拒绝（走 `check` 同一套解析）。不做行内空格重排与折行。
- `zam --version` / `--help`；缓存目录默认 `~/.zam/cache`，可用环境变量 `ZAMAK_CACHE_DIR` 覆盖。`check` 不需要 C 编译器。
- `zam check --json` / `zam build --json` 在 stdout 输出一行机器可读诊断：`{"command","ok","diagnostics":[{file,line,column,function,message}]}`（无法定位的字段为 `null`），失败时 stderr 仍有一行人类可读信息；其他子命令不接受 `--json`。
- 构建进度（`CACHE`/`OBJECT`/`LINK`/`OUTPUT`）与错误都写 **stderr**，stdout 只有程序输出或 `--json` 的那一行；`build` 的 `OUTPUT <path>`（Windows 为 `build/<name>-<hash>.exe`）现在也在 stderr。命令失败返回非零码。

## 语法速览（动手前先看 `examples/`）

- 函数：`fn f(a: i64, s: &string) -> i64 { ... }`；最后一个值表达式即返回值，也可写 `return`；跨模块可见要写 `pub fn`。
- 绑定：`let x = 1`、`let mut x = 1`、`let mut x: i64 = 1`。**不支持遮蔽**（同一作用域重名报错）；块内变量不能逃出块。
- 类型：`i64`、`bool`、`string`、命名结构体、`[i64; N]`（长度 1–1024，只能是局部值）。
- 结构体：`struct P { name: string` 换行 `age: i64 }`（每行一个字段，无逗号）；字面量 `P { name: "a" age: 3 }`（**字段之间没有逗号**）；字段读写 `p.name`、连续 `a.b.c`；支持整体移动、整体重新赋值、可变字段赋值；字符串字段自动释放。
- 控制：`if`/`else`/`else if`、`while`、`break`、`continue`；条件必须是 `bool`。**没有 `for`**。
- 表达式：`+ - * / %`、`== != < <= > >=`、`&& || !`、一元 `-`、括号；整数可用数字之间的单个 `_`（`1_000_000`）。溢出/除零在运行时返回错误。
- 字符串：UTF-8，转义 `\n \r \t \" \\`，插值 `"{name}"` / `"{p.name}"`（只读不移动，插值后变量仍可用），字面大括号写 `{{` / `}}`。
- 注释：`//` 与 `#`；**没有块注释**。
- 模块：`use greeting`（同目录 `greeting.zm`，只能调用其 `pub fn`）、`use std/string`、`use std/io`、`use std/fmt`；调用 `greeting.hello()`、`io.println("x")`；`println(...)` 无需导入。
- 所有权：字符串赋值/传参是**移动**，移动后再用报错；借用只能作为参数（`&s`、`&mut s`），调用返回即结束；共享借用可多个，可变借用必须独占且要求可变所有者；借用参数不能移动、返回或存进局部变量。整数/布尔是拷贝。
- 错误值：签名 `fn load(p: &string) -> i64!string`（成功 T / 失败 E）或 `-> !string`（只失败）；`fail "msg"` 产生错误并立即返回；`let v = load(&p)?` 失败时向调用方传播（当前函数签名必须带相同的 `!E`）；`load(&p) catch e { ... }` 就地处理（**不传播**，错误值只在块内可见，**块必须在所有路径 `return`**）；`fn main() -> !string` 允许，未捕获错误写 stderr 并以退出码 1 结束。当前只支持 `E = string`。
- 标准库：`string.byte_len(&s) -> i64`、`string.char_len(&s) -> i64`（按 UTF-8 码点）、`string.equal(&a, &b) -> bool`；`println` / `io.println` / `fmt.println` 打印字符串、整数或布尔值。
- **尚未实现，不要写**：泛型、闭包、枚举、`match`、`for`、浮点、动态数组、元组/映射、包管理器、调试器、REPL、文档生成、线程、unsafe/FFI、块注释、`string` 以外的错误类型、整体结构体借用、单独移出 owned 字段。

## 常见诊断（照文本改代码）

- `fallible call f must be handled with ?` → 调用处漏了 `?` 或 `catch`。
- `? cannot propagate the error of f from this function` → 当前函数签名缺少 `!E`，或错误类型不同。
- `fail requires a function that declares an error type` → 在非 `!E` 函数里用了 `fail`。
- `catch block for f must return a value` → catch 代码块要在所有路径上 `return`。
- `only string error types are supported for now` → 错误类型只能是 `string`。
- 移动后使用、借用冲突/逃逸、不可变赋值、重复/缺失字段等都在 `zam check` 阶段报错。
- 诊断格式：`zam: <文件>: line N, column M: <模块>:<函数>: <信息>`；词法/语法错误带行列（列从 1 开始、按 Unicode 字符计数），语义错误取语句起点。

## 修改 Zamak 编译器本身

1. 验收（工作目录 `zam/`）：`cargo test --locked`（9 个集成套件，冷跑约 6 分钟）、`cargo clippy --all-targets --locked -- -D warnings`、`cargo fmt --check`；改完源码要重新 `cargo install --path zam --locked` 才能用新的 `zam`。
2. 新增语言构造时按惯例升版本标记：`zam/src/compiler.rs` 的 encode 头 `ZAM-IR-N` 与 `zam/src/project.rs` 的 `VERSION`（`native-c-M`）。两者都参与缓存键，只作失效标记——缓存命中时按字节全等校验，没有 decode，旧产物只会报错不会静默错译；同时更新 `README.md` 里的 IR 号。
3. C 后端源码按 UTF-8 写出，MSVC `cl` 默认按本地代码页（936）读取，会吞掉中文注释后的下一行 → 编译命令行必须保留 `/utf-8`；改动编译命令行要同步 `zam/src/native.rs` 的 `OPTIONS`（`-vN`）。
4. 示例放 `examples/<主题>/*.zm`（多模块项目为 `<主题>/src/main.zm` + `zam.toml`），并在 `zam/tests/<主题>.rs` 里用 `include_str!` 引用示例做正例与反例，真正运行产物并断言 stdout/stderr/退出码。
5. 文档边界：`README.md` 会入库；`SYNTAX.md`、`ROADMAP.md`、`DESIGN.md`、`NAMING.md`、`MIGRATION.md` 被 `.gitignore` 忽略，只在本地维护。二者都要同步（README 如实标注缺口，ROADMAP 记进度）。
6. 提交：分支 `main`，信息用 `<type>:中文描述`（feat/fix/test/docs/chore），按主题分批，必要时 `git push origin main`。
7. 编辑器/AI 生态：`editors/vscode/` 是纯 TextMate 语法扩展（零运行时依赖），改动关键字、符号或类型时要同步 `editors/vscode/syntaxes/zamak.tmLanguage.json` 与 `language-configuration.json`。

## 参考资料

`README.md`（能力与缺口）、`SYNTAX.md`（语法细节）、`examples/`（可运行示例）、`ROADMAP.md`（当前进度的权威记录）、`zam/src/compiler.rs`（前端）、`zam/src/native.rs`（C 后端）、`zam/src/project.rs`（构建与缓存）。
