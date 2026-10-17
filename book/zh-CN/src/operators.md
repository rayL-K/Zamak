# 数字与运算符

Zamak 的运算符与 Python 一致，除法也一样。区别在于安全：整数结果超出 64 位时，Zamak 会让程序停下，而不是返回一个错误的数。

## 算术运算

```zamak
fn main():
    println(7 + 2)
    println(7 - 2)
    println(7 * 2)
    println(7 / 2)
    println(7 // 2)
    println(7 % 2)
    println(2 ** 10)
```

```text
9
5
14
3.5
3
1
1024
```

| 运算符 | 含义 |
| --- | --- |
| `+`、`-`、`*` | 加、减、乘 |
| `/` | 除法。结果总是 `f64`，所以 `7 / 2` 是 `3.5`。 |
| `//` | 整除：相除后向下取整。 |
| `%` | 取余。结果的符号与除数相同。 |
| `**` | 乘方。从右往左结合：`2 ** 3 ** 2` 是 `2 ** 9`。 |

整除和取余都向负无穷取整，与 Python 相同，所以遇到负数时两者依然一致：

```zamak
fn main():
    println(-7 // 2)
    println(-7 % 2)
    println(7.5 // 2)
```

```text
-4
1
3.0
```

## 整数与浮点数混用

一个运算里同时有 `i64` 和 `f64` 时，整数先变成 `f64`：

```zamak
fn main():
    let price = 2.5
    let count = 4
    println(price * count)
    println(count < 4.5)
```

```text
10.0
true
```

要显式转换，用 `float(n)` 或 `int(x)`。`int` 丢掉小数部分，向零取整：

```zamak
fn main():
    println(float(3))
    println(int(3.9))
    println(int(-3.9))
```

```text
3.0
3
-3
```

`f64(n)` 和 `i64(x)` 是同样转换的另一种写法。

## 浮点数怎样打印

`println` 打印能原样读回同一个 `f64` 的最短文字。浮点数总带小数点或指数，所以一眼就能和整数区分：

```zamak
use std/fmt

fn main():
    println(2.0)
    println(0.1 + 0.2)
    println(1e16)
    println(fmt.fixed(3.14159, 2))
```

```text
2.0
0.30000000000000004
1e16
3.14
```

在任何使用二进制浮点数的语言里，`0.1 + 0.2` 都不恰好等于 `0.3`。要打印固定位数的小数，用 `fmt.fixed`。

## 溢出与除以零

`i64` 最大能表示 9,223,372,036,854,775,807。结果超过这个上限时，Zamak 让程序带着错误停下：

```zamak,panics
fn largest() -> i64: 9223372036854775807

fn main():
    println("before")
    println(largest() + 1)
    println("after")
```

```text
before
zam: 整数运算出错
```

在 C 里，有符号整数溢出是未定义行为；Rust 的发布构建会回绕成一个负数。无论哪种，程序都会带着错误的值继续运行。Zamak 检查每一次整数 `+`、`-`、`*`、`//`、`%` 和 `**`。除以零同样会让程序停下，整数和浮点数都一样：

```zamak,panics
fn zero() -> i64: 0

fn main():
    println(10 // zero())
```

```text
zam: 除以零
```

这些检查的开销很小。编译器能证明其中大部分不可能出错（例如循环计数器不会超过列表长度），并把它们去掉。实测数据见[附录 B](appendix-performance.md)。

## 在一个函数里关闭溢出检查

有些代码本来就需要整数回绕，例如哈希函数和随机数生成器。给这样的函数标上 `@wrapping`：

```zamak
@wrapping
fn hash(text: &string) -> i64:
    let mut h = 1469598103934665603
    for code in text.codepoints():
        h = (h ^ code) * 1099511628211
    h

fn main():
    println(hash("zamak"))
```

```text
3838841982605952249
```

在 `@wrapping` 函数里，整数 `+`、`-`、`*` 和取负按二进制补码回绕；浮点数除以零像 C 一样得到无穷大或 NaN，而不是让程序停下。其他检查照常进行，包括整数除以零和列表下标越界。只有被标记的函数受影响，程序的其余部分仍然检查溢出。

## 比较与逻辑

比较的结果是 `bool`：`==`、`!=`、`<`、`<=`、`>`、`>=`。用单词 `and`、`or`、`not` 组合条件：

```zamak
fn main():
    let age = 30
    let member = true
    println(age >= 18 and member)
    println(not member or age > 65)
```

```text
true
false
```

`and` 和 `or` 一旦能确定结果就不再往下算，所以右边只在需要时才执行。优先级与 Python 相同：`not` 高于 `and`，`and` 高于 `or`。

Zamak 不接受 `&&`、`||` 或 `!` 作逻辑运算；写了会报错，并提示应该用哪个单词。

## 位运算

整数支持 `&`（与）、`|`（或）、`^`（异或）、`~`（取反）、`<<`（左移）和 `>>`（右移）。优先级与 Python 相同，所以 `x & 1 == 1` 表示 `(x & 1) == 1`：

```zamak
fn main():
    let flags = 0b1010
    println(flags & 0b0010)
    println(flags | 1)
    println(1 << 4)
    println(flags & 1 == 0)
```

```text
2
11
16
true
```

移位的位数必须在 0 到 63 之间。

## 数学函数

`abs`、`min`、`max` 是内置函数，不用导入。平方根、乘方、对数、三角函数和取整在 `std/math` 里：

```zamak
use std/math

fn main():
    println(abs(-5))
    println(max(2.5, 1))
    println(math.sqrt(2.0))
    println(math.round(2.5))
    println(math.pi)
```

```text
5
2.5
1.4142135623730951
3.0
3.141592653589793
```

`math.round` 把正好在中间的值向远离零的方向取整。

## 小结

- 除法与 Python 相同：`/` 得到 `f64`，`//` 和 `%` 向负无穷取整。
- `i64` 与 `f64` 混用时整数自动转换；`int` 和 `float` 显式转换。
- 整数溢出和除以零会让程序停下。`@wrapping` 在一个函数里关闭溢出检查。
- 逻辑运算用单词 `and`、`or`、`not`。
