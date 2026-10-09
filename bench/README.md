# bench — Zamak 构建成本基准

回答 DESIGN.md 第 8 节的问题：**构建成本到底贵在哪一层**（依赖解析 / 代码生成 / 链接 / 磁盘 IO），并为 Zamak 立下靶子数字。

## 组成

- `gen.mjs` — 一份规格生成 Rust workspace 与 Go module 两套等价实现（medium-cli：10 编译单元、约 5000 行、每函数体含唯一常量防编译器折叠；`manifest.json` 记录各场景的编辑锚点）
- `measure.ps1` — 9 场景测量驱动：进程树 WorkingSet 轮询采样峰值内存、缓存隔离（CARGO_HOME/GOMODCACHE/GOCACHE 全指向 `bench/cache/`）、JSONL 落盘
- `generated/` — 生成物，不入库，随时用 `node bench/gen.mjs --lines 5000` 重建

## 跑法

```
node bench/gen.mjs --lines 5000
pwsh -c "cd bench\generated\medium-cli\go; go mod tidy"   # 首次必须
pwsh bench\measure.ps1 -Lang both -Reps 3
```

数据落 `C:\Zamak\results\raw\runs.jsonl`。

## 九个场景

1 冷构建（清依赖缓存+产物） 2 温构建（只清产物） 3 改一行（叶子 cli） 4 改一行（中间层 store） 5 改 core 私有常量 6 改 core 公开签名（含调用点） 7 连续 20 次改一行 8 4 路 worktree 并行（共享缓存） 9 check 路径（cargo check / go vet）

场景 5 的设计意图：Rust 以 crate 为粒度、私有改动仍触发下游重编；Go 导出数据未变则只重编 core——对比即「模块接口显式化收益」的实测。

## 边界

Windows/NTFS 偏差必须在报告标注，不得当成语言差距；生成代码不等于真实项目。
