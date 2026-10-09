# 构建成本基准 v1 — Rust vs Go（同机同源码）

> 日期：2026-10-09 · 机器：Intel Core Ultra 7 155H（16C/22T，31.5 GB RAM，NVMe SSD）· Windows 11 / NTFS
> 工具链：cargo/rustc 1.98.1（msvc）· go 1.27.2 · Node 24.20（生成器）
> 生成器：`bench/gen.mjs --lines 5000`（实际 13,584 行源码，10 单元链式依赖 + serde/thiserror/clap vs cobra/viper/zap）
> 测量：`bench/measure.ps1 -Lang both -Reps 3`，隔离缓存（CARGO_HOME / GOMODCACHE / GOCACHE），峰值内存按 80ms 轮询进程树 WorkingSet。
> **偏差声明**：绝对值含 NTFS + Defender 开销，只可比比值；均为 dev profile；单机单样本，方差见原始轮次（s1 冷构建 3 轮 37.9–50.6s）。

## 结果总表（3 轮中位数）

| # | 场景 | Rust | Go | 倍率 | 峰值内存 R / G |
|---|---|---|---|---|---|
| 1 | 冷构建（清缓存+产物） | **42.9s** | 24.4s | 1.76× | 1351 / 745 MB |
| 2 | 温构建（只清产物） | 32.7s | 20.0s | 1.64× | 1214 / 788 MB |
| 3 | 改叶子模块一行 | 2.50s | 2.44s | 1.02× | 422 / 242 MB |
| 4 | 改中间模块一行 | 3.31s | 2.50s | 1.32× | 426 / 270 MB |
| 5 | 改 core 私有常量 | 4.01s | 2.47s | 1.62× | 307 / 154 MB |
| 6 | 改 core pub 签名 + 6 调用方 | **4.58s** | **1.51s** | **3.0×** | 415 / 63 MB |
| 7 | 20 次单行编辑连编 | Σ52.6s · 中位 2.42s | Σ51.5s · 中位 2.54s | ~1× | — |
| 8 | 4 路 worktree 并行 | **64.8s** | **5.0s** | **13.0×** | 948 / 817 MB |
| 9 | check 路径（cargo check / go vet） | 2.16s | 2.63s | 0.82× | 532 / 319 MB |

磁盘占用（跑完全部场景后）：Rust `target/` 334.6 MB（从 268.9 MB 一路涨上来，**每 worktree 一份、不共享**）；Go GOCACHE 145.3 MB（**全局一份、跨 worktree 复用**）。
产物体积：Rust dev exe 1.5 MB，Go exe 8.9 MB（静态链接 runtime+GC）。

## 发现

- **F1（头号发现）多 worktree 并行差 13 倍，且机制与 DESIGN.md §4/§5 的预判完全一致。** cargo 的增量指纹绑定绝对路径 + target/ 每 checkout 一份 → 4 路并行 = 4 份从零重编互相抢核（64.8s ≈ 2 × 单路 32.7s，争用导致 2 倍墙钟）。Go 的 build cache 是全局内容寻址、与路径无关 → 4 个 worktree 几乎全命中缓存，只剩 4 次链接（5.0s）。**这就是 Zamak 必须做路径无关内容寻址缓存的实证：语言侧规则相同的前提下，缓存的可共享性 alone 决定 13 倍差距。**
- **F2 接口变更是 Rust 的放大器（3.0×，且内存 6.6×）。** 改 core 一个 pub 签名，Rust 要重做下游 rlib + proc-macro 重展开（415 MB），Go 只需重编依赖方（63 MB）。字典传递 + 接口显式的收益直接可见。
- **F3 日常编辑循环（s3/s7）两种语言打平（~2.4-2.5s）。** 叶子改一行时双方都只剩「重编一个 crate/package + 链接」，链接是共同地板——佐证 DESIGN.md 目标「亚秒增量」的真实瓶颈在链接与产物布局，不在语义分析。
- **F4 冷/温构建 Rust 恒定贵 1.6-1.8×，峰值内存贵 2-3×。** 温构建 32.7s 里依赖只占 ~10s，大头是 13.5k 行 workspace 本身：proc-macro（serde/clap/thiserror derive）+ 单态化 + 泛型膨胀。
- **F5 磁盘：差距在中间产物，不在最终 exe。** 最终二进制都是 MB 级（1.5 vs 8.9 MB），「几十 G 吃磁盘」的是 target/ / 缓存目录——Rust 335 MB × 每 worktree 一份；Go 145 MB 全局一份。Zamak 的磁盘目标应锚定**中间产物总量**，不是最终二进制。
- **F6 check 路径两者相当（~2s）。** 类型检查本身不是瓶颈；瓶颈在后续代码生成与链接。

## 对 DESIGN.md §8 目标的校准

| 指标 | 基线（Rust / Go） | Zamak v0 目标 |
|---|---|---|
| 单行增量 | 2.4–3.3s / 2.4–2.5s | **< 1s** |
| 接口变更传播 | 4.6s / 1.5s | **< 1s**（接口哈希不变则零重编） |
| 4 路 worktree 并行 | 64.8s / 5.0s | **≈ 单路 relink 成本**（路径无关缓存，零争用） |
| 单项目中间产物+缓存 | 335 MB / 145 MB | **< 50 MB**（≈ Rust 的 1/7 起） |
| 构建峰值内存 | 1.35 GB / 0.75 GB | **< 200 MB** |
| 最终产物体积 | 1.5 MB / 8.9 MB | 同档即可，不追二进制极小 |

## 复现

```
node bench/gen.mjs --lines 5000
cd bench/generated/medium-cli/go && go mod tidy   # 首次
pwsh bench/measure.ps1 -Lang both -Reps 3         # ~25 分钟
# 原始逐轮数据：results/raw/runs.jsonl
```
