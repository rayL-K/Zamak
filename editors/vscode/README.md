# Zamak for VS Code

Zamak 语言的编辑器支持：`.zm` 文件的语法高亮（关键字、类型、字符串插值、注释、数字、运算符）、行注释与括号/引号自动配对。

扩展是**纯声明式**的：只有一份 TextMate 语法和语言配置，没有 `main`、没有运行时依赖、不联网、不执行代码。同一份 `syntaxes/zamak.tmLanguage.json` 也可以直接给 Sublime Text、TextMate 或其他支持 TextMate 语法的编辑器复用。

## 本地安装（开发用）

方式一，链接到扩展目录：

```powershell
# Windows
New-Item -ItemType Junction -Path "$env:USERPROFILE\.vscode\extensions\zamak-0.1.0" -Target "C:\Zamak\editors\vscode"
# macOS / Linux
ln -s /path/to/Zamak/editors/vscode ~/.vscode/extensions/zamak-0.1.0
```

方式二，打包成 `.vsix` 再安装（需要 Node.js）：

```powershell
cd editors/vscode
npx @vscode/vsce package
code --install-extension zamak-0.1.0.vsix
```

安装后在 VS Code 里打开任意 `.zm` 文件即可；右下角语言模式应显示 `Zamak`。

## 当前不提供

- 没有语言服务器：补全、跳转、诊断、格式化都不在编辑器里做。写代码时用 `zam check <file|project>` 获取带行列的诊断。
- 没有调试适配器（DAP）和测试集成。
- 没有 `code --install` 之外的自动更新；上架前只能手工安装。

## 上架 VS Code 市场

1. 在 <https://marketplace.visualstudio.com/manage> 注册发布者，拿到 publisher id，把 `package.json` 里的 `"publisher": "zamak"` 改成自己的 id（当前值只是占位）。
2. 在 Azure DevOps 生成 Personal Access Token（scope 选 Marketplace → Manage）。
3. 补齐市场要求但仓库里还没有的文件：`LICENSE`（许可证需用户/维护者选定）、`CHANGELOG.md`。
4. 打包并发布：

```powershell
npx @vscode/vsce package          # 生成 zamak-0.1.0.vsix，可先本地安装验证
npx @vscode/vsce publish          # 需要 VSCE_PAT 环境变量或交互输入 PAT
```

5. 每次改动语法后同步 `package.json` 的 `version` 并重新发布。

## 维护

```powershell
node scripts/check-grammar.mjs
```

校验三件事：三个 JSON 都能解析、语法里每条 `match`/`begin`/`end` 正则都能编译、语言 id / 后缀 / scopeName（`source.zamak`）/仓库分组引用互相一致。改动语言关键字或符号时，同步 `syntaxes/zamak.tmLanguage.json`、`language-configuration.json` 和本校验脚本里的关键字清单。

`icon.png` 由 `ico/Zamak_256x256.ico` 缩放到 128×128 生成（VS Code 市场要求图标为 PNG，建议 128×128 及以上）。
