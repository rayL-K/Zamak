# 列表、字典与元组

Zamak 有三种存放多个值的方式：列表按顺序保存值，字典按键查找值，元组把固定数量、类型可以不同的值组合在一起。

## 列表

列表 `[T]` 按顺序存放任意多个 `T` 类型的值。空列表需要类型标注：

```zamak
fn main():
    let mut scores: [i64] = []
    scores.push(72)
    scores.push(95)
    scores.push(88)
    println(scores)
    println(len(scores))
    println(scores[0])
    println(scores[-1])
```

```text
[72, 95, 88]
3
72
88
```

下标从 0 开始，负数下标从末尾数起。越过列表范围的下标会让程序带着错误停下，而不是读到随机的内存：

```zamak,panics
fn main():
    let xs: [i64] = [1, 2, 3]
    println(xs[3])
```

```text
zam: 列表下标越界
```

### 修改列表

```zamak
fn main():
    let mut xs: [i64] = [5, 3, 8]
    xs[0] = 6
    xs.insert(1, 4)
    println(xs)
    let last = xs.pop()
    let first = xs.remove(0)
    println(f"{first} {last} {xs}")
    xs.extend([1, 2])
    xs.reverse()
    println(xs)
    xs.clear()
    println(len(xs))
```

```text
[6, 4, 3, 8]
6 8 [4, 3]
[2, 1, 3, 4]
0
```

| 方法 | 作用 |
| --- | --- |
| `xs.push(v)` | 在末尾添加 `v`。 |
| `xs.insert(i, v)` | 在位置 `i` 插入 `v`。 |
| `xs.pop()` | 移除并返回最后一个元素。 |
| `xs.remove(i)` | 移除并返回位置 `i` 的元素。 |
| `xs.extend(ys)` | 把 `ys` 全部移到末尾。 |
| `xs.reverse()`、`xs.clear()`、`xs.swap(i, j)` | 就地反转、清空、交换。 |

### 查找、排序与统计

下面这些适用于数字、布尔值和字符串列表：

```zamak
fn main():
    let xs: [i64] = [4, 1, 3, 1]
    println(3 in xs)
    println(xs.index_of(1))
    println(sorted(xs))
    println(f"min {min(xs)}, max {max(xs)}, sum {sum(xs)}")
    println(xs[1:3])
    let mut names: [string] = ["Linus", "Ada", "Grace"]
    names.sort()
    println(names)
```

```text
true
1
[1, 1, 3, 4]
min 1, max 4, sum 9
[1, 3]
["Ada", "Grace", "Linus"]
```

`sorted(xs)` 返回排好序的副本，`xs.sort()` 就地排序，两者都是稳定排序。切片 `xs[a:b]` 复制列表的一部分，与 Python 相同。

要给任意列表排序（包括结构体列表），就给 `sort_by` 一个函数，`a` 应该排在前面时返回 `true`：

```zamak
struct City:
    name: string
    people: i64

fn main():
    let mut cities: [City] = [
        City(name: "Lyon", people: 520000),
        City(name: "Paris", people: 2100000),
        City(name: "Nice", people: 340000),
    ]
    cities.sort_by(fn(a, b): a.people > b.people)
    for c in cities:
        println(c.name)
```

```text
Paris
Lyon
Nice
```

### 在循环里修改元素

`for x in &mut xs` 让循环可以修改每个元素。给 `x` 赋值就是写回元素：

```zamak
fn main():
    let mut prices: [f64] = [10, 20, 30]
    for p in &mut prices:
        p = p * 0.9
    println(prices)
```

```text
[9.0, 18.0, 27.0]
```

### 列表的列表

元素也可以是列表或字典。`grid[i][j]` 读写内层的元素：

```zamak
fn main():
    let mut grid: [[i64]] = [[0, 0, 0], [0, 0, 0]]
    grid[1][2] = 7
    grid[0].push(1)
    for row in grid:
        println(row)
```

```text
[0, 0, 0, 1]
[0, 0, 7]
```

## 字典

字典 `{K: V}` 按键保存值。键是 `i64` 或 `string`，字典会记住键插入的顺序：

```zamak
fn main():
    let mut ages = {"Ada": 36, "Grace": 85}
    ages["Linus"] = 21
    ages["Ada"] = 37
    println(ages)
    println(ages["Grace"])
    println(ages.get("Guido", 0))
    println("Ada" in ages)
    println(ages.remove("Grace"))
    println(ages.keys())
    println(len(ages))
```

```text
{"Ada": 37, "Grace": 85, "Linus": 21}
85
0
true
true
["Ada", "Linus"]
2
```

- `m[k] = v` 插入或替换。
- `m[k]` 读取，键不存在时程序会停下。`m.get(k, default)` 在键不存在时返回 `default`。
- `k in m` 和 `m.contains(k)` 判断键是否存在，`m.remove(k)` 返回是否删掉了一个键。
- `m.keys()` 和 `m.values()` 按插入顺序返回列表。

用 `for k in m` 遍历键，用 `for k, v in m` 同时遍历键和值。

### 计数

计数是字典最常见的用途。Zamak 会把这个惯用写法编译成一次查找：

```zamak
fn main():
    let mut counts: {string: i64} = {}
    for ch in "mississippi":
        counts[ch] = counts.get(ch, 0) + 1
    println(counts)
```

```text
{"m": 1, "i": 4, "s": 4, "p": 2}
```

### 分组

字典的值可以是列表：

```zamak
fn main():
    let words: [string] = ["apple", "avocado", "banana", "blueberry", "cherry"]
    let mut by_letter: {string: [string]} = {}
    for w in words:
        let letter = w[0]
        if letter not in by_letter:
            by_letter[letter] = []
        by_letter[letter].push(w)
    println(by_letter)
```

```text
{"a": ["apple", "avocado"], "b": ["banana", "blueberry"], "c": ["cherry"]}
```

## 元组

元组把固定数量的值组合在一起。类型写作 `(i64, string)`，用 `.0`、`.1` 等读取各项：

```zamak
fn min_max(xs: &[i64]) -> (i64, i64):
    (min(xs), max(xs))

fn main():
    let pair = (3, "three")
    println(pair.0)
    println(pair.1)
    let (low, high) = min_max([4, 9, 1])
    println(f"{low}..{high}")
    let mut scores: [(string, i64)] = []
    scores.push(("Ada", 90))
    println(scores)
```

```text
3
three
1..9
[("Ada", 90)]
```

元组适合临时组合几个值，例如多个返回值。这组值有自己的含义时，用结构体给它起个名字。

## 小结

- `[T]` 是可增长的列表。下标从 0 开始，负数从末尾数，越界会让程序停下。
- `{K: V}` 是字典，键是 `i64` 或 `string`，保持插入顺序。
- `(A, B)` 是元组，用 `let (a, b) = ...` 拆开。
- `for x in &mut xs` 就地修改元素。
