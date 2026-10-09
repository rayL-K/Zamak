# Zamak 项目：新工作区交接说明

这份文件写在 `C:\Zamak` 根目录，目的是把工作区从中文路径迁移过来之后，新会话打开 `C:\Zamak` 就能直接接着干，不需要回看旧会话。

## 1. 给新会话的 agent：先读这三件事

1. 这份文件（上下文与现状）
2. `C:\Zamak\DESIGN.md`（语言设计原则 v0.1）
3. `C:\Zamak\NAMING.md`（命名的全部核查证据）

然后第一件事是问用户：v0 先做哪一件（见第 4 节）。

### 本机有一个必须知道的坑

会话早期所有 shell 命令都报 `spawn C:\Users\王文康\scoop\shims\pwsh.exe ENOENT`，错误信息把 shell 路径甩出来，很容易误判成「shell 缺失」。实际上 PowerShell 7.6.6 装在 `C:\Users\王文康\scoop\apps\pwsh\7.6.6\pwsh.exe`，scoop 的 shim 与 `current` 软链都正常。后来的实测表明默认工作目录已恢复正常（`Get-Location` 正确返回会话工作区），不必每次显式指定。若再遇到同样的 ENOENT，给命令显式传工作目录即可绕过。

其它偶发噪声：文件检索（glob / grep）的 ripgrep provider 偶发失败；web_fetch / read 的结果偶尔被自动审查层以 `RATE_LIMIT: 429` 拦截，重试即可。

## 2. 项目一句话

Zamak（命令行 `zam`，源文件扩展名 `.zam`）：一门为 AI 时代多 agent 并行构建而设计的编程语言。目标 = Go 级的编译速度 + Rust 级的无 GC 内存安全 + 深入底层做极致性能优化的能力。

出发点（用户原话的意思）：多个 agents 并行、多 worktree，反复 build 和 check，再强的 CPU 也经常被打满，一个个几十 G 起步的 target 一点一点吃掉磁盘空间，而今年 SSD 还那么贵。AI 写代码越来越快，构建成本越来越显眼。

## 3. 已经定下来的事

- 语言名 **Zamak**（/ˈzɑːmæk/，中文可读「扎马克」），命令行短名 **zam**（/zæm/），扩展名 **.zam**。语言名与工具名分开，照 Rust 对 cargo、Go 对 go 的先例。
- 命名第一判据是**检索唯一性**：今后的文档、示例、报错有很大一部分是模型在读，名字不唯一会让 agent 把别人的语法和标准库当成我们的。漂亮又短的现成英文词基本已被占满。
- 设计原则见 `DESIGN.md`：优先级排序（构建成本 > 内存安全 > 运行性能 > 表达力）、中心假设是「单位时间构建次数」、编译速度必须由语言规则负责而不是靠编译器优化、内存模型三个候选（A 受限所有权加借用／B ARC 加写时复制／C arena 分代，v0 只选一个）、内容寻址缓存与可复现构建、多 agent 与多 worktree 一等公民、工具链形态、明确不做清单、可测验收判据。
- 定位句：Zamak：无 GC，内存安全，构建以秒计。Rust 的纪律，Go 的手感，压铸的速度。

## 4. 还没定的事：v0 第一步

用户尚未选择，三个候选：

- **A. 先量构建成本基准**：用 Rust 在用户这台机器上把一个中等规模项目的冷构建时间、单行增量、从零 rebuild、target 体积、峰值内存全部测一遍，把 `DESIGN.md` 里的目标方向换成真实数字。这是唯一能给后面所有决策提供依据的一步，也直接回答「构建到底贵在哪」。
- **B. 先搭构建骨架**：用 Rust 写 `zam` 这个二进制，先不实现语言，只实现内容寻址缓存、跨 worktree 共享、亚秒增量，用玩具输入验证。这是这门语言真正的差异化部分。
- **C. 先写最小编译器**：词法、语法、代码生成一条龙，先让 `zam build` 编译出 hello world。最快拿到「我有一门语言了」的成就感，但此时还没有任何构建速度可看。

建议的顺序是 A → B → C。

## 5. 环境事实（2026-10-09 实测）

- 有：rustc / cargo 1.98.1（stable-x86_64-pc-windows-msvc，MSVC 链接器完整，`cargo build` 实测通过）、Python 3.14.7、Node 24.20.0、pnpm、git、scoop
- 没有：**Go**（要做 Go 对比基准得先装）、clang / cl 不在 PATH
- 机器：Intel Core Ultra 7 155H，16 核 22 线程，31.5 GB 内存，C 盘可用 677.9 GB
- 冒烟结果：hello world 的开发构建 1.9 秒，target 目录 2.9 MB。残留测试工程在 `C:\Users\王文康\Desktop\Klang\_envcheck\hello`。

## 6. 迁移清单

- `C:\Zamak\DESIGN.md` 来自 `C:\Users\王文康\Desktop\Klang\DESIGN.md`
- `C:\Zamak\NAMING.md` 来自 `C:\Users\王文康\Desktop\Klang\NAMING.md`
- `C:\Zamak\MIGRATION.md` 就是这份文件
- 旧目录 `C:\Users\王文康\Desktop\Klang` 确认无用后可以整个删掉（含 `_envcheck`）
- 新会话把工作目录设成 `C:\Zamak` 即可

## 7. 下一步的其它待办

- 注册 GitHub 组织名 `zamak-lang`（2026-10-09 核查时仍可注册）
- 商标检索与域名核查（如 zamak.dev）
- PyPI 与 npm 上的 `zamak` / `zam` 目前都是 404，可以日后预留
