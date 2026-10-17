# Changelog

## Unreleased

- One-click install: when `zam` isn't on `PATH`, the extension offers **Install with npm** (runs `npm install -g zamak` in a terminal) and **Zamak: Check for the zam Compiler Again** to pick it up afterwards.
- The status bar shows the `zam` version in use, and turns into a warning with an install entry point when `zam` is missing.
- Parameter hints while typing a call, and **Rename** / **Find All References** for names in the current file.
- Completion after `x.` uses the type the checker inferred instead of guessing from the name.

## 0.1.0

- Syntax highlighting for `.zm` files, with comment toggling, bracket pairing, and automatic indentation after a line that ends with `:`.
- Diagnostics as you type, including unsaved changes, from `zam lsp`.
- Completion, hover, go to definition, and Format Document.
- The `zamak.path` setting and the **Zamak: Restart Language Server** command.
