# 配合 AI 编程助手

Zamak 在设计时就考虑到它既由人来写，也由 AI 编程助手来写。整门语言小到一页就能讲完，诊断信息稳定且便于机器读取，`zam` 还能把项目配置好，让你的 AI 助手从第一句提示起就懂 Zamak。

## 为 AI 助手创建项目

```sh
zam new myapp --template ai
```

除了常规的 `zam.toml` 和 `src/main.zm`，这条命令还会生成：

| 文件 | 由谁读取 |
| --- | --- |
| `.claude/skills/zamak/SKILL.md` | Claude Code，任务涉及 Zamak 时把它作为 Skill 载入。 |
| `AGENTS.md` | Codex、Cursor 等会读取 `AGENTS.md` 的 AI 助手。 |
| `.mcp.json` | 支持 MCP 的 AI 助手，按它启动 `zam mcp`。 |
| `README.md` | 人，里面有构建和运行项目的命令。 |

`SKILL.md` 和 `AGENTS.md` 包含完整的语言参考和推荐的工作循环。这些文字内置在 `zam` 里，所以始终与创建项目的那个版本一致。

## 工作循环

这些文件让 AI 助手小步推进，每一步都经过检查：

1. 运行 `zam check --json .`，阅读诊断信息。
2. 做最小的修改，修好报告的错误。
3. 运行 `zam fmt .`，再检查一次。
4. 检查通过后，运行 `zam build .`、`zam run .` 和 `zam test .`。

这个循环很适合 AI 助手：`zam check` 不到一秒就能完成，也不需要 C 编译器；每条诊断都准确说明要改什么：

```json
{"file":"main.zm","line":5,"column":5,"function":"main:main","code":"ZM301","category":"ownership","message":"use after move: name","suggestion":"borrow the value or move it only after the last use"}
```

- `code` 标识错误的种类，含义永远不变。
- `category` 对错误码分组，例如 `syntax`、`type`、`ownership`。
- `suggestion` 是具体的修改建议；代码用了 Python 写法时，它常常直接给出 Zamak 的写法。

## 为什么 AI 写得好 Zamak

- **一页就够**：在一项有三个模型、八个命令行任务的测试里，每个模型只读了那一页语言参考。它们完成了全部 24 个 Zamak 任务，其中 21 个一次通过。方法和数据在仓库的 [bench/ai/live](https://github.com/VeyraLux/Zamak/tree/main/bench/ai/live) 里。
- **熟悉的语法**：AI 读过大量 Python，Zamak 尽可能沿用 Python 的名字和规则。
- **错误发现得早**：类型和所有权错误在 `zam check` 阶段就出现，而不是等到运行时；整数溢出和下标越界会带着清楚的消息停下，不会产生 AI 可能察觉不到的错误结果。
- **按函数报告错误**：`zam` 每个函数报告一次类型错误，所以 AI 一次编辑就能修好好几处。

## 在已有项目里加入 Zamak 说明

要在已有项目里给 AI 助手提供 Zamak 的说明，可以用 `--template ai` 创建一个临时项目，再把 `AGENTS.md`、`.claude/skills/zamak/` 或 `.mcp.json` 复制到你的项目里。

## 用 MCP 连接 AI 助手

`zam mcp` 运行一个模型上下文协议（MCP）服务器，支持 MCP 的 AI 助手可以直接调用 Zamak，不必在终端里敲命令。用 `--template ai` 创建的项目自带启动它的 `.mcp.json`：

```json
{
  "mcpServers": {
    "zamak": {
      "command": "zam",
      "args": ["mcp"]
    }
  }
}
```

Claude Code 和 Cursor 打开项目时会读取 `.mcp.json`。其他 AI 助手可以在它的 MCP 设置里添加一个服务器，命令填 `zam`，参数填 `mcp`。服务器提供这些工具：

| 工具 | 作用 |
| --- | --- |
| `check` | 检查项目，返回与 `zam check --json` 相同的 JSON。 |
| `build` | 构建项目，返回诊断信息和可执行文件的路径。 |
| `run` | 构建并运行程序，可以传参数和标准输入，返回程序的输出和退出码。 |
| `test` | 运行测试，可以只运行名字里包含某个词的测试。 |
| `format` | 像 `zam fmt` 一样格式化项目文件。 |
| `explain` | 解释一个错误码（例如 `ZM301`）以及怎么改。 |
| `docs` | 返回语言参考、某个标准库模块的函数，或参考里与某个词有关的段落。 |
| `hover` | 给出文件、行、列（从 1 数起），返回那里的类型或签名。 |

这些工具调用的就是 `zam` 本身，所以结果和你在命令行里看到的完全一样。

服务器还提供两份[资源](https://modelcontextprotocol.io/docs/concepts/resources)：MCP 客户端可以把它们一次装进助手的上下文，不必每次都调工具。

| 资源 | 内容 |
| --- | --- |
| `zamak://reference` | 一页纸的语言参考：语法、类型、所有权、错误处理和标准库。 |
| `zamak://diagnostics` | 每个错误码的分类与含义，从 `check` 拿到的码不用再问一次就能读懂。 |

两份内容就是 `docs` 与 `explain` 工具返回的同一段文字，所以总是和编译器同步。

## 小结

- `zam new NAME --template ai` 为 Claude Code 配好 `SKILL.md`，为其他 AI 助手配好 `AGENTS.md`。
- AI 助手按 `zam check --json`、最小修改、`zam fmt`、构建的循环工作。
- `zam mcp` 把同样的工具直接交给支持 MCP 的 AI 助手，另外还有 `explain` 和 `docs`。
- 稳定的错误码、分类和修改建议，让 AI 助手很容易照着诊断去修改。
