# Zamak for VS Code

Language support for [Zamak](https://zamak.veyralux.org), the language that reads like Python and runs like C.

- **Syntax highlighting** for `.zm` files, with comment toggling, bracket pairing, and automatic indentation after a line that ends with `:`.
- **Diagnostics as you type.** The extension checks the whole project, including changes you haven't saved, and underlines each error with its code and a suggested fix.
- **Completion** for keywords, built-in functions, your declarations and local variables, functions after a module name such as `string.`, and methods and fields after a value. After `x.`, the methods and fields come from the type the checker inferred, so `let p = make()` completes `p.` with what `make` returns.
- **Parameter hints** while you type a call: after `foo(`, the parameter list appears and the argument you are writing is highlighted.
- **Rename** and **Find All References** for local variables and functions in the current file.
- **Hover** shows the signature of a function or the fields of a struct.
- **Go to definition** for functions, structs, enums, constants, and local variables, across modules.
- **Format Document** runs the same formatter as `zam fmt`.

Created by rayL-K · VeyraLux

## Requirements

Install Zamak so that `zam` is on your `PATH`, or set **zamak.path** to the full path of `zam`. The extension starts `zam lsp` in the background. Without `zam`, highlighting still works.

If the extension can't find `zam`, it offers a **Install with npm** button that runs `npm install -g zamak` in a terminal. The status bar shows the `zam` version the extension is using, and turns into a warning with an install link when `zam` is missing.

## Settings

| Setting | Default | Description |
| --- | --- | --- |
| `zamak.path` | `zam` | The `zam` command to run. |

Diagnostics follow the VS Code display language: Chinese when VS Code is in Chinese, English otherwise. Set the `ZAMAK_LANG` environment variable to `zh` or `en` to choose yourself.

If you change your Zamak installation, run **Zamak: Restart Language Server** from the Command Palette. **Zamak: Install the zam Compiler (npm)** opens a terminal running `npm install -g zamak`; **Zamak: Check for the zam Compiler Again** looks for `zam` once more and starts the server if it is now there.

## 中文说明

Zamak 语言的 VS Code 扩展：语法高亮，边写边报错（没保存的内容也检查），补全（`x.` 后面按检查器推断出的真实类型列出方法与字段），输入调用时显示参数提示，本文件内的重命名与查找引用，悬停查看签名，跳到定义，以及与 `zam fmt` 相同的格式化。需要先安装 Zamak，让 `zam` 在 `PATH` 里，或者在设置里把 `zamak.path` 设为 `zam` 的完整路径。找不到 `zam` 时扩展会给出「用 npm 安装」按钮，直接执行 `npm install -g zamak`；状态栏显示当前用的 `zam` 版本，没装时变成带安装入口的警告。VS Code 界面是中文时，报错也是中文。

## Development

Install the dependency once, then link the folder into your extensions directory:

```powershell
cd editors/vscode
npm install
# Windows
New-Item -ItemType Junction -Path "$env:USERPROFILE\.vscode\extensions\zamak-0.1.0" -Target "$PWD"
# macOS and Linux
ln -s "$PWD" ~/.vscode/extensions/zamak-0.1.0
```

To build a `.vsix` package, run:

```powershell
npx @vscode/vsce package
code --install-extension zamak-0.1.0.vsix
```

`node scripts/check-grammar.mjs` checks that the three JSON files parse, that every TextMate regular expression compiles, that the language ID, file extension, and scope name agree, and that every command registered in `extension.js` is declared in `package.json` (and the other way round). When you change keywords or operators, update `syntaxes/zamak.tmLanguage.json`, `language-configuration.json`, and the keyword list in that script.

The language server lives in the compiler (`zam/src/lsp.rs`), so new compiler releases improve the extension without a new extension release.

### Publish to the Marketplace

1. Create a publisher at <https://marketplace.visualstudio.com/manage> and replace the placeholder `"publisher": "zamak"` in `package.json` with its ID.
2. Create a personal access token in Azure DevOps with the **Marketplace → Manage** scope.
3. Update `CHANGELOG.md` and the `version` in `package.json`.
4. Run `npx @vscode/vsce publish`.

`icon.png` is `docs/icon-512.png` (from `tools/make-logo.py`) scaled to 128×128.
