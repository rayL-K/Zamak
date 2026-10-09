# Zamak 语法设计 v0.1

> 定稿日期：2026-10-09。内存模型依据 DESIGN.md §3（调用帧借用）。
> 原则：规则少到可以背下来；人类读写体验对标 Python；花括号界定块边界（可审计，不猜缩进）。

---

## 一段完整的示例

```zam
use std/io
use std/fmt

pub struct Point {
    x: f64
    y: f64
}

pub enum Shape {
    circle(r: f64)
    rect(w: f64, h: f64)
}

pub fn area(s: &Shape) -> f64 {
    match s {
        circle(r)  => 3.14159 * r * r
        rect(w, h) => w * h
    }
}

pub fn load(path: string) -> [Point]!Err {
    let mut out = []
    for line in io.read_lines(path)? {
        let (x, rest) = line.split_once(",")?
        out.push(Point{
            x: x.trim().parse(f64)?
            y: rest.trim().parse(f64)?
        })
    }
    out
}

fn main() {
    let pts = load("points.csv") catch e {
        fmt.println("load failed: {e}")
        return
    }
    fmt.println("{pts.len()} points loaded")
}
```

---

## 1. 基础语法

### 1.1 语句与块

- 换行即语句结束，不写分号。
- 块用 `{ }` 界定，缩进只是风格，不影响语义。
- `{ }` 内多个字段/语句用换行分隔，不写逗号。

```zam
let x = 1
let y = 2
let z = {
    let tmp = x + y
    tmp * tmp      // 块的最后一个表达式是块的值
}
```

### 1.2 变量

```zam
let x = 42           // 不可变，类型推断
let mut count = 0    // 可变
let n: i64 = 100     // 显式类型（pub 边界必须写）
```

- 所有变量必须初始化才能读取，编译器强制。
- 默认 move 语义：赋值 / 传参后原变量失效，除非类型实现了 `Copy`。

### 1.3 基础类型

| 类型 | 说明 |
|------|------|
| `bool` | `true` / `false` |
| `i8 i16 i32 i64 i128` | 有符号整数，溢出 panic |
| `u8 u16 u32 u64 u128` | 无符号整数，溢出 panic |
| `f32 f64` | 浮点 |
| `string` | UTF-8 字符串，owned |
| `[T]` | 动态数组（slice/vec） |
| `{K: V}` | 哈希表 |
| `(T, U, ...)` | 元组 |
| `T?` | 可选值（`some(v)` / `none`） |
| `T!E` | 结果值（ok(v) / err(e)），见 §4 |

类型转换一律显式：`x as f64`，无隐式转换。

---

## 2. 函数

```zam
// 普通函数
fn add(a: i64, b: i64) -> i64 {
    a + b
}

// pub 函数：参数类型和返回类型必须写
pub fn greet(name: string) -> string {
    "Hello, {name}!"
}

// 无返回值（省略 ->，等价于 -> ()）
fn log(msg: &string) {
    io.println(msg)
}
```

- 函数体是表达式，最后一个表达式的值就是返回值，不用写 `return`（`return` 用于提前退出）。
- 参数默认 owned（move 进来）。加 `&` 是借用，借用只活到当前调用返回。
- `pub` 函数是模块接口，类型标注是合约，不可省略。

---

## 3. 结构体与枚举

### 3.1 结构体

```zam
pub struct Point {
    x: f64
    y: f64
}

// 构造：命名字段
let p = Point{ x: 1.0; y: 2.0 }

// 字段访问
let dist = (p.x * p.x + p.y * p.y).sqrt()
```

- 字段之间用换行分隔，不写逗号。
- 结构体不能持有 `&T`（借用不能住进结构体，见 DESIGN.md §3）。

### 3.2 枚举

```zam
pub enum Shape {
    circle(r: f64)
    rect(w: f64, h: f64)
    point
}
```

- 枚举变体可以带命名字段（`circle(r: f64)`）、多字段（`rect(w, h)`），或无字段（`point`）。
- 构造和模式匹配使用**同一套语法**（构造 = 模式的镜像）：

```zam
let s = circle(r: 5.0)       // 构造

match s {
    circle(r)  => r * r * 3.14159
    rect(w, h) => w * h
    point      => 0.0
}
```

---

## 4. 错误处理

Zamak 的错误是值，没有异常。

```zam
// 函数签名：返回 [Point] 或 Err
pub fn load(path: string) -> [Point]!Err { ... }

// ? 运算符：遇到错误立即返回给调用方
let lines = io.read_lines(path)?

// catch 就地处理
let data = load("x.csv") catch e {
    fmt.println("failed: {e}")
    return
}
```

- `T!E` 表示"成功返回 T，失败返回 E"。
- `?` 在当前函数签名允许传播错误时使用（函数返回类型必须是 `!E`）。
- `catch` 用于就地处理，不传播。

---

## 5. 控制流

```zam
// if / elif / else
if x > 0 {
    "positive"
} elif x < 0 {
    "negative"
} else {
    "zero"
}

// if 是表达式
let label = if x > 0 { "pos" } else { "non-pos" }

// for-in
for item in collection {
    fmt.println("{item}")
}

// for 带索引
for i, item in collection {
    fmt.println("{i}: {item}")
}

// while
while condition {
    ...
}

// match 是表达式
let desc = match shape {
    circle(r) => "circle r={r}"
    rect(w, h) => "rect {w}x{h}"
}
```

---

## 6. 内存：所有权与借用

三条规则，一次记住：

1. **值默认 move**：赋值或传参后原变量失效。
2. **`&T` 是借用**：只活到当前函数返回，不能存进结构体。
3. **要长期持有就 own**：clone 一份，或让结构体持有 owned value。

```zam
fn area(s: &Shape) -> f64 { ... }  // 借用，s 在调用返回后还给调用方

struct Renderer {
    shape: Shape    // owned，Renderer 自己负责 shape 的生命周期
}

// 错误：把借来的存进结构体
struct Bad { ref: &Shape }   // 编译器拒绝
```

需要底层控制时用 `unsafe { }` 块，内部可以操作裸指针和内存布局。

---

## 7. 泛型

```zam
// 泛型函数：默认字典分发（不展开，编译快）
fn first(xs: &[T]) -> T? {
    if xs.len() == 0 { none } else { some(xs[0]) }
}

// 泛型结构体
struct Pair(T, U) {
    first: T
    second: U
}
```

- 泛型参数写在函数名或结构体名后的括号里。
- 默认不单态化：所有具体类型共享同一份机器码，通过函数表间接调用（运行时有微小开销，编译极快）。
- 编译器可基于 profile 数据自动对热点调用点做特化，无需手动标注。

---

## 8. 闭包

```zam
// 短闭包
let double = |x| x * 2

// 多行闭包
let process = |x: i64| -> i64 {
    let y = x * x
    y + 1
}

// 传给高阶函数
let results = items.map(|x| x * 2)
```

- 闭包默认**非逃逸**：可以捕获 `&T`，但不能被存进结构体或跨线程传递。
- 如果闭包需要逃逸（存进字段、返回给调用方），只能捕获 owned value。

---

## 9. 模块与导入

```zam
use std/io
use std/fmt
use mylib/parser { Ast, parse }   // 只引入指定名字

pub fn my_func() { ... }          // pub = 对外可见
fn helper() { ... }               // 无 pub = 模块私有
```

- 一个文件就是一个模块，路径即模块名。
- `pub` 标注的函数和类型构成模块接口，编译器只读接口，不读实现体。

---

## 10. 字符串插值与注释

```zam
let name = "Zamak"
let msg = "Hello, {name}! Version {version}"   // 花括号内是任意表达式

# 这是注释（单行）
```

---

## 关键字总表（17 个）

`use` `pub` `fn` `let` `mut` `struct` `enum` `match` `if` `elif` `else`
`for` `while` `return` `in` `catch` `unsafe`

---

## 已推迟到后续版本

- **Trait / 接口系统**：泛型约束、UFCS（统一函数调用语法）依赖 trait，待内存模型原型验证后设计。
- **并发原语**：`async/await` 与调用帧借用的交互需要专门设计（跨挂起点的借用语义）。
- **类型级编程**：明确不做（见 DESIGN.md §7）。
- **包管理与版本锁**：`zam.toml` 格式，v0 最小化。
