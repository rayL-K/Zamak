# 命名记录（2026-10-09 定案）

## 结论

- 语言名：**Zamak**（读 /ˈzɑːmæk/，中文写作「扎马克」）
- 命令行工具名：**zam**（读 /zæm/，三字母、一个音节）
- 源文件扩展名：**.zm**（2026-10-09 用户最终定案，源码仅接受此后缀）
- 用户级工具目录：**~/.zam**（Windows 本机为 `C:\Users\王文康\.zam`），命令仍为 **zam**。
- 定位句：Zamak：无 GC，内存安全，构建以秒计。Rust 的纪律，Go 的手感，压铸的速度。

## 为什么不是 Zinc

Zinc 已经是一门活跃开发的编程语言，而且方向和我们要做的高度接近：

- github.com/zinc-lang 组织（2025-02-21 创建）下有 `zinc-lang/zinc`（描述就是 "zinc compiler and std library"，C 写的编译器，2026-09-12 建仓，12 star，最后推送 2026-10-08）和 `zinc-lang/zinc-design`（设计文档仓）。
- 它的方案是：保留内存安全与线程安全，去掉所有权类型和借用检查器，改用自动引用计数加写时复制；泛型默认字典传递。也就是「无 GC、内存安全」这条路上已经有人挂了同一块牌子。
- 生态里还有：crates.io 的 `zinc`、`zinc-cli` / `zinc-proto` / `zinc-daemon`（面向 AI 编码 agent 的终端多路复用器）、`zinc-core`（比特币钱包）；npm 的 `zinc`（zincio SDK）；PyPI 的 `zinc`（电商 API SDK，密钥前缀 `zn_`）。
- 结论：这个名字在搜索、包管理器、训练语料里都不唯一。今后的文档、示例、报错有很大一部分是模型在读，名字不唯一会让 agent 把别人的语法和标准库当成我们的。

## 命名判据

AI 时代给语言命名，第一判据是**检索唯一性**，其次才是好听、好记、能当动词。漂亮又短的现成英文词基本已被占满，这就是本次 12 个候选几乎全军覆没的原因。

## 撞名核查总表（2026-10-09 逐个端点实抓）

| 候选 | 是否已有同名语言 | crates.io | PyPI | npm | GitHub 组织 |
| --- | --- | --- | --- | --- | --- |
| **Zamak** | 否 | 0 | 404 空 | 404 空 | `zamak-lang` 可注册 |
| **zam** | 否 | 1 个小库（shell 历史管理，115 次下载） | 404 空 | 404 空 | `zam-lang` 可注册 |
| Spelter | 否 | 0 | 404 空 | 404 空 | 备选；词义偏软，7 字母 |
| fulgor | 否 | 1（cpplint 的 Rust 移植，1489 次下载） | 404 空 | 404 空 | 可注册；Fulgor Milano 是意大利厨电品牌 |
| kyanite | **是**（GitHub 组织简介自称 "Development of the Kyanite programming language"） | 有 | 404 空 | 有 | 组织已被占用 |
| zeta | **是**（crates.io 的 `zeta` 就是 "Compiler for the Zeta programming language"，仓库 zeta-lang/zeta） | 有 | 有 | 有 | 被占用 |
| lume | **是**（组织简介自称 "Illuminating, simplistic and expressive programming language"） | 有 | 有 | 有 | 被占用 |
| beryl | npm 上的 `beryl` 自称模板式编程语言 | 25 | 占用 | 自称语言 | `beryl-lang` 可注册 |
| teak | npm 上的 `teak` 自称 functional data language | 12（无精确同名） | 占用 | 自称语言 | 可注册 |
| aurum / auri | 否 | 21 / 733 | 占用 | 占用 | 被占用 |
| spinel | 否 | 53 | 占用 | 占用 | 可注册 |
| verve | 否 | 15（无精确同名） | 占用 | 占用 | 可注册 |
| yare | 否 | 同名库 117 万次下载 | 占用 | 占用 | 可注册 |

## 历史候选 .zam 的扩展名撞名核查（2026-10-09 实抓）

以下保留当时对旧候选后缀的查询记录，不代表当前源码后缀；当前统一使用 `.zm`。

扩展名的判据弱于语言名：真正要避免的是与**编程语言**冲突（或导致模型检索 `.zam` 时分不清谁的语法），普通工具占扩展名不算撞名。逐端点实抓结果：

| 端点 | 检查对象 | 结果 |
| --- | --- | --- |
| GitHub linguist `languages.yml`（代码托管平台语言识别的权威清单，821 门语言） | 任何语言的扩展名含 `.zam` / `.zm`，或语言名含 Zam | **0 命中**（文件已验证真实：Rust / Zig 条目在列） |
| Sourcegraph 全局代码搜索（`file:\.zam$`） | 真实世界里的 `*.zam` 文件 | **0 匹配** |
| fileinfo.com/extension/zam | 文件类型库 | 404，页面明确 "The extension .ZAM was not found"；Wayback Machine 无任何历史快照（该页从未存在过） |
| file-extension.org/extensions/zam | 文件类型库 | 页面明确 "not found" |
| file-extensions.org/zam | 文件类型库 | HTTP 404 |

当时的查询结果：所查清单、文件类型库和代码搜索中未发现 `.zam` 冲突；该结果仅为历史记录，最终源码后缀已定为 `.zm`。

附带核查（同日顺手确认，与扩展名无关）：crates.io 的 `zamak` 404 可用；npm 的 `zam` 是 "Lightweight data binding"（非语言）；PyPI 的 `zamak` 404 可用；GitHub 组织 `zamak-lang`、`zam-lang` 均 404 可注册。

## 名字本身的故事（为什么是 Zamak）

依据 Wikipedia「Zamak」条目：

1. ZAMAK 是德语四种金属名的首字母缩写：Zink（锌）加 Aluminium（铝）加 Magnesium（镁）加 Kupfer（铜）。四个词熔成一个词，对应「把多种优点熔进一门语言」。
2. 锌合金早年的致命失效叫 zinc pest（锌疫），杂质引起的晶间腐蚀会让铸件自己碎掉，等于锌的「锈」。Zamak 用 99.99% 纯锌加重流精炼治好了它。
3. 主要工艺是压铸（die casting）：同一副模具，快速、大批量、高精度出件，对应多 agent、多 worktree 反复构建。
4. 1929 年由 New Jersey Zinc 开发，早期以 MAZAK 拼写注册过商标，今天已被 ASTM、EN、JIS、GB 8738 等压铸标准当作通用术语使用；MAZAK 拼写现属日本机床公司 Yamazaki Mazak，所以只使用 zamak 拼写。

## 还没做的核查

- 商标检索（各国，含「Zamak」相关类目）与域名（如 zamak.dev / zamak-lang.org）
- GitHub 组织名 `zamak-lang` 的实际注册（2026-10-09 核查时仍可用）
- 包管理器上预留 `zamak` / `zam` 名称（PyPI 与 npm 目前皆为 404）
- 覆盖边界：通用搜索引擎（DuckDuckGo bot challenge、Brave 429、Bing 引号失效返回噪声）本次均未能有效检索，`.zam` 的结论依赖的是权威清单与代码库直查，不是全网搜索；日后若发现某小众工具用 `.zam` 作扩展名，不推翻上节结论。
