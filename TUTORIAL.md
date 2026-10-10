# Zamak 入门教程

这是一份给零基础读者的 Zamak 教程，按小节从零讲到能写小项目。每个小节的代码都实际编译运行过，**输出就是真实的运行结果**；示例源码在 `examples/` 下，可以直接跑。

Zamak 目前是原型：语法接近 Python 的阅读体验，编译成原生可执行文件，并且对 `string` 做所有权与借用检查。它还不适合生产使用，缺什么见最后一节。

## 第 0 节：准备环境

本机已安装时，直接确认版本：

```powershell
zam --version
```

```text
0.1.0
```

没有安装就从源码装一次（需要 Rust 工具链；Windows 上还需要 Visual Studio 的 C++ 工具，Unix 上需要 `cc`）：

```powershell
cargo install --path zam --locked
```

Zamak 的编译器就是 `zam` 本身，它生成 C 代码再调用系统 C 编译器，所以**编译出来的程序不依赖 Zamak 也不依赖 Rust**。

## 第 1 节：第一个程序

新建一个项目，然后运行它：

```powershell
zam new hello
cd hello
zam run
```

```text
NEW hello\zam.toml
NEW hello\src\main.zm
NEW hello\.gitignore
CACHE MISS d04a3b53b61b main
OBJECT MISS 85ce7172e7cb main
LINK MISS 3a1c51f75bfb
OUTPUT .\build\hello-3a1c51f75bfb5ee1b69a694147559f6e0765bfcf172cbde2e98f5ea957ced16d.exe
hello from zamak
```

`CACHE`/`OBJECT`/`LINK`/`OUTPUT` 这些进度行写到标准错误（stderr），程序自己的输出写到标准输出（stdout），所以**只需要程序输出时可以放心重定向 stdout**。

最小程序长这样，保存为 `hello.zm` 后运行 `zam run hello.zm`：

```zamak
fn main() {
    println("你好，Zamak")
}
```

```text
你好，Zamak
```

要点：

- 程序从 `fn main()` 开始执行。
- `println(...)` 打印一行，字符串用双引号。
- 函数体用大括号包起来，缩进是 4 个空格（`zam fmt` 会帮你对齐）。
- 注释用 `//`（行注释），也可以用 `#`。

## 第 2 节：变量与类型

```zamak
fn main() {
    let name = "Zamak"
    let mut count: i64 = 3
    count = count + 1
    let ok = true
    println(name)
    println(count)
    println(ok)
}
```

```text
Zamak
4
true
```

要点：

- `let` 声明变量，**默认不可重新赋值**；要改就写 `let mut`。
- 类型写冒号后面：`let mut count: i64 = 3`。也可以省略，让编译器从初值推断。
- 三种基本类型：`i64`（64 位整数）、`bool`（`true`/`false`）、`string`（字符串）。
- **没有变量遮蔽**：同一作用域里不能再用 `let` 声明一个同名变量。
- 标量是复制语义，`let copy = count` 得到独立的值。

试一试：把 `let mut count` 改成 `let count`，看看 `count = count + 1` 报什么错。

## 第 3 节：运算符

```zamak
fn main() {
    println(7 / 2)
    println(7 % 2)
    println(2 + 3 * 4)
    println(10 - 4)
    println(1_000_000)
    println(3 > 2)
    println(!true)
}
```

```text
3
1
14
6
1000000
true
false
```

要点：

- 算术：`+` `-` `*` `/` `%`；整数除法**向下取整**，`7 / 2` 得 `3`。
- 比较：`==` `!=` `<` `>` `<=` `>=`，结果是 `bool`。
- 逻辑：`!`（非）、`&&`（与）、`||`（或）。
- 数字里可以用 `_` 分组，`1_000_000` 就是一百万。
- 乘除的优先级高于加减，和数学一样；需要时加括号。

## 第 4 节：条件判断

```zamak
fn classify(n: i64) -> string {
    if n < 0 {
        return "negative"
    } else if n == 0 {
        return "zero"
    } else {
        return "positive"
    }
}

fn main() {
    println(classify(-3))
    println(classify(0))
    println(classify(9))
}
```

```text
negative
zero
positive
```

要点：

- `if` 的条件**不加括号**，条件后面直接跟代码块。
- `else if` 可以串很多个。
- 条件必须是 `bool`，Zamak 不做真假值隐式转换。
- `return` 立刻从函数返回；只要所有分支都返回了，函数就一定能返回 `string`。

## 第 5 节：循环

```zamak
fn main() {
    let mut i = 0
    while i < 5 {
        i = i + 1
        if i == 2 {
            continue
        }
        if i == 5 {
            break
        }
        println(i)
    }
}
```

```text
1
3
4
```

要点：

- Zamak **只有 `while`，没有 `for`**；遍历数组要自己数下标。
- **没有 `+=` 这类复合赋值**，写 `i = i + 1`。
- `continue` 跳过本次循环剩下的部分，`break` 直接结束循环。
- 循环条件必须是 `bool`。

## 第 6 节：函数

```zamak
fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn greet(name: string) -> string {
    return "你好，{name}！"
}

fn main() {
    println(add(2, 3))
    let who = "Zamak"
    println(greet(who))
}
```

```text
5
你好，Zamak！
```

要点：

- 参数写成 `名字: 类型`，多个参数用逗号隔开。
- `-> 类型` 是返回值类型；不写就是没有返回值。
- 函数体**最后一个表达式就是返回值**，所以 `add` 不用写 `return`；也可以用 `return` 提前返回。
- 跨模块调用需要 `pub fn`（见第 12 节）。
- 没有泛型、没有闭包、没有函数重载；本教程一律用 `while` 写循环，没有验证递归。

试一试：把 `greet` 的参数改成 `name: &string`，然后 `println(greet(&who))`。这会报 `cannot interpolate this value`——**字符串插值目前只接受拥有所有权的 `string`**，不能直接插值借用来的字符串。

## 第 7 节：字符串、转义与插值

```zamak
use std/string

fn main() {
    let name = "Zamak"
    let text = "你好，{name}！"
    println(text)
    println("第一行\n第二行")
    println("字面大括号 {{x}}")
    println(string.byte_len(&text))
    println(string.char_len(&text))
    println(string.equal(&text, &text))
}
```

```text
你好，Zamak！
第一行
第二行
字面大括号 {x}
17
9
true
```

要点：

- `{变量}` 把值插进字符串里，变量和结构体字段都行：`"{point.x}"`。
- 转义：`\n` 换行、`\r` 回车、`\t` 制表、`\"` 双引号、`\\` 反斜杠。
- 想输出真正的大括号，写 `{{` 和 `}}`。
- 字符串**不能换行**（一个字符串字面量必须写在一行里）。
- 标准库 `std/string` 提供三个函数：`string.byte_len(&s)` 按字节数、`string.char_len(&s)` 按 Unicode 字符数、`string.equal(&a, &b)` 判断内容相等。上面 `你好，Zamak！` 是 17 字节 9 个字，正好说明两者的区别。
- 中文用 UTF-8，所以按字节算长度会大于按字符算。

## 第 8 节：数组

```zamak
fn main() {
    let mut values: [i64; 3] = [10, 20, 30]
    values[1] = values[0] + 5
    let mut i = 0
    while i < 3 {
        println(values[i])
        i = i + 1
    }
}
```

```text
10
15
30
```

要点：

- 类型写成 `[i64; 3]`（元素类型 + 分号 + 长度），长度必须在 1 到 1024 之间。
- 下标从 0 开始，**运行时做边界检查**：越界会直接报错退出，不会读到别的内存。
- 数组只能作为局部变量，不能当函数参数或返回值。
- 遍历用 `while` 加下标变量。

## 第 9 节：结构体

```zamak
struct Point {
    x: i64
    y: i64
}

fn move_by(point: Point, dx: i64) -> Point {
    let mut result = point
    result.x = result.x + dx
    result
}

fn main() {
    let mut p = Point { x: 1
        y: 2
    }
    println(p.x)
    p.y = p.y + 3
    println(p.y)
    let moved = p
    println(moved.x + moved.y)
    let q = move_by(moved, 10)
    println(q.x)
}
```

```text
1
5
6
11
```

要点：

- 声明字段时**每行一个、不用逗号**（这一点和函数参数不同，参数用逗号）。
- 构造用 `Point { x: 1 y: 2 }`，字段之间同样不写逗号，可以写在一行也可以分行。
- 字段用 `.` 访问，可以直接赋值：`p.y = p.y + 3`。
- 结构体是**移动语义**：`let moved = p` 之后 `p` 就不能再用了，继续用会报 `use after move`。想让 `p` 复活，就整体重新赋值：`p = Point { ... }`。
- 嵌套结构体字段也能一路点下去：`group.leader.age = group.leader.age + 1`。

## 第 10 节：所有权与借用

```zamak
fn show(text: &string) {
    println(text)
}

fn update(text: &mut string) {
    text = "被借用修改过"
}

fn consume(text: string) {
    println(text)
}

fn main() {
    let mut message = "原始内容"
    show(&message)
    update(&mut message)
    show(&message)
    let moved = message
    consume(moved)
}
```

```text
原始内容
被借用修改过
被借用修改过
```

要点：

- `string` 是拥有所有权的值。传参会**移动**它，传走以后原变量失效——这就是 `consume(text: string)` 的用法。
- 只想读，就借出去：`&message` 传给 `text: &string`。
- 想改，就借出去一个可变的：`&mut message` 传给 `text: &mut string`（原变量必须是 `let mut`）。
- 借用只在这次调用期间有效，所以 `show` 和 `update` 之后 `message` 还能继续用。
- 检查发生在编译期（`zam check` 就能发现），报错信息形如 `line 4, column 1: main:main: use after move: a`。
- 目前只有 `string` 和结构体受这套规则约束，`i64`/`bool` 是复制语义。

## 第 11 节：错误处理

函数可以在返回类型后面加 `!错误类型` 表示「可能失败」；当前只支持 `E = string`。

```zamak
use std/string

fn parse(text: &string) -> i64!string {
    let empty = ""
    if string.equal(text, &empty) {
        fail "empty input"
    }
    return 42
}

fn load(text: &string) -> i64!string {
    let value = parse(text)?
    return value
}

fn main() -> !string {
    let text = "42"
    let value = load(&text)?
    println(value)
}
```

```text
42
```

三件事：

- `i64!string`：成功返回 `i64`，失败返回 `string` 错误信息。
- `fail "empty input"`：产生一个错误值（只能在声明了错误的函数里用）。
- `parse(text)?`：**把错误直接往上抛**；如果 `parse` 失败，`load` 立刻带着这个错误返回。用了 `?` 的函数自己必须也是可失败的，且错误类型要一致。

`main` 也可以写成 `-> !string`，表示「顶层可能失败」。没人接住的错误会打到 stderr 并以退出码 1 结束。把上面的 `"42"` 改成 `""` 就能看到失败过程：

```text
empty input
zam: program failed: exit code: 1
```

只想就地处理、不往上抛，用 `catch`：

```zamak
use std/string

fn parse(text: &string) -> i64!string {
    let empty = ""
    if string.equal(text, &empty) {
        fail "empty input"
    }
    return 42
}

fn main() {
    let text = "42"
    let value = parse(&text) catch e {
        println("解析失败：{e}")
        return
    }
    println(value)
}
```

成功时输出 `42`；把 `text` 改成 `""` 就输出：

```text
解析失败：empty input
```

要点：

- `catch e { ... }` 里的 `e` 是错误值，只在块内可见。
- 块内**必须 `return`**（或者给出一个与 `let value` 类型匹配的值），不能什么都不返回。
- 目前错误类型只能是 `string`，也只有一个 `?`，没有泛型错误、没有错误堆栈。

## 第 12 节：多模块项目

单个 `.zm` 文件可以直接 `zam run`，但项目通常会有多个模块，用 `zam.toml` 描述：

```toml
[project]
name = "hello"
entry = "src/main.zm"
modules = "src"
```

```text
hello/
├── zam.toml
└── src/
    ├── main.zm
    └── greeting.zm
```

`src/main.zm`：

```zamak
use greeting
use std/io

fn main() {
    greeting.hello()
    io.println("你好，Zamak")
}
```

`src/greeting.zm`：

```zamak
pub fn hello() {
    println("hello from zamak")
}
```

在项目根目录执行 `zam run`（也可以 `zam run examples/hello`）得到：

```text
hello from zamak
你好，Zamak
```

要点：

- `entry` 是入口文件，`modules` 是模块所在目录。
- **模块名就是文件名**：`greeting.zm` 用 `use greeting` 引入，调用写成 `greeting.hello()`。
- 跨模块可见的函数、结构体要加 `pub`。
- 跨模块用类型要写模块前缀：`leader: model.Person`。
- 文件后缀统一是 `.zm`；入口必须是 `.zm`（否则报 `entry must be a .zm file`）。
- 不想写配置文件时，`zam check`/`build`/`run`/`fmt` 也接受单个 `.zm` 文件。

## 第 13 节：命令行工具

```text
zam 0.1.0
  zam check [project|file]
  zam build [project|file]
  zam fmt [project|file]
  zam run [project|file|artifact]
  zam new <directory>
  zam check --json [project|file]
  zam build --json [project|file]
  zam demo
  zam --version
```

- `zam check`：只做词法、语法、类型与所有权检查，不生成机器码，所以**不需要 C 编译器**，适合编辑器里快速反馈。成功打印 `CHECK OK main`。
- `zam build`：生成可执行文件，路径以 `OUTPUT <path>` 写到 stderr，可以直接运行。
- `zam run`：先构建再运行，程序输出就是它的 stdout。
- `zam fmt`：就地整理缩进（4 个空格）、去行尾空白、结尾留一个换行，保留原来的 CRLF/LF。只改空白、不重建 token，所以注释和字符串原样保留；语法有错的代码会被拒绝而不会被改写。改写过的文件用 `FORMAT <path>` 报出来。
- `zam new <directory>`：生成最小项目骨架（第 1 节）。目录名必须是合法标识符，已存在的目录会被拒绝，不会覆盖任何文件。
- `zam demo`：跑一遍内置的双目录缓存演示，展示第二次构建全命中。
- `zam --json`（只给 `check`/`build` 用）：结果写成一行 JSON，方便编辑器和 CI 读取：

```text
{"command":"check","ok":false,"diagnostics":[{"file":".tmp-tut\\13_broken.zm","line":2,"column":1,"function":null,"message":"unterminated function block"}]}
```

- 环境变量 `ZAMAK_CACHE_DIR` 可以改缓存目录。
- 出错时退出码非 0，错误写 stderr。

## 第 14 节：常见错误与解决办法

诊断的格式是：

```text
<文件>: line <行>, column <列>: <模块>:<函数>: <信息>
```

词法和语法错误没有「模块:函数」这一段。列号从 1 开始、按字符计，制表符算一个字符。

| 报错信息 | 原因 | 怎么改 |
| --- | --- | --- |
| `unexpected character '@'` | 出现了不属于 Zamak 的字符 | 删掉或换掉该字符 |
| `expected '{', found Some(Word("string"))` | 语法没写对，比如类型写错位置 | 对照前面的小节检查该行结构 |
| `unterminated function block` | 大括号没配对 | 补上 `}` |
| `use after move: a` | 变量已经被移动给别人了 | 用 `&a` 借用，或重新赋值 |
| `cannot interpolate this value` | 字符串插值用了 `&string` | 把参数类型改成 `string` |
| `borrow escape: functions cannot return references` | 函数想返回 `&string`/`&mut string` | 返回拥有所有权的 `string` |
| `arrays are only supported as local values` | 把数组当参数或返回值 | 改成多个标量参数，或在函数内建数组 |
| `fallible call main:f must be handled with ?` | 漏写 `?` | 加 `?`，或用 `catch` 就地处理 |
| `? cannot propagate the error of main:f from this function` | 当前函数没声明错误类型，或错误类型不一致 | 函数签名写 `-> T!string` |
| `fail requires a function that declares an error type` | 在不可失败的函数里用了 `fail` | 给函数加 `!string` |
| `catch block for f must return a value` | `catch` 块里没有 `return` | 加 `return` |
| `only string error types are supported for now` | 用了 `!i64` 之类的错误类型 | 改成 `!string` |
| `entry must be a .zm file` | `zam.toml` 的入口后缀不对 | 改成 `.zm` |
| `zam.toml: 系统找不到指定的文件` | 目录里没有 `zam.toml` | 用 `zam new` 建项目，或直接传单个 `.zm` 文件 |
| `project name must be an identifier` | `zam new` 的目录名不是合法标识符 | 换成字母/数字/下划线 |
| `<dir>: already exists` | 目标目录已存在 | 换个目录名，或先删掉 |
| `zam: program failed: exit code: 1` | 程序里的错误没人接住 | 用 `?` 传到 `main`，或用 `catch` 处理 |

## 第 15 节：还没实现的功能

写代码前先知道边界，省得白试：

- 没有 `for`、没有 `switch`/`match`、没有 `enum`、没有泛型、没有闭包、没有 trait/接口。
- 没有包管理器、没有 REPL、没有文档生成、没有格式化以外的重构工具。
- 标准库只有 `std/string` 的 `byte_len`/`char_len`/`equal`，`std/io` 与 `std/fmt` 的 `println`。
- 错误类型只能是 `string`，没有错误堆栈。
- 数组只能做局部变量，不能当参数或返回值；没有切片、没有动态数组。
- 所有权检查只覆盖 `string` 和结构体；没有线程、没有 `unsafe`、没有 FFI。
- 编辑器支持只有一份 VS Code 语法高亮扩展（`editors/vscode/`），没有语言服务器、补全和调试器。
- 安装/发行流程（预编译产物、包管理器）尚未提供。

## 附：示例与测试索引

想直接读代码，`examples/` 下都是能跑的：

| 示例 | 内容 |
| --- | --- |
| `examples/hello/` | 多模块项目、`use`、`io.println` |
| `examples/ownership/` | `&string`、`&mut string`、移动 |
| `examples/structs/` | 结构体声明、字段赋值、嵌套、跨模块类型 |
| `examples/control/` | 条件、`while`、`break`/`continue`、函数 |
| `examples/values/` | 变量、运算符、字符串、数组、格式化、错误值、`catch` |

每个特性都有对应的集成测试（`zam/tests/`），例如 `errors.rs` 覆盖 `T!string`/`fail`/`?`/`catch` 的成功与失败路径，`source_format.rs` 覆盖 `zam fmt` 的行为。想确认某个写法到底行不行，最快的办法是写个小文件跑 `zam check`。
