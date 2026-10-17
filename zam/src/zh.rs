//! 中文报错（D8）。
//!
//! 编译器内部的报错都是英文句子（`project::Result<T> = Result<T, String>`），JSON 诊断与测试都依赖
//! 这些原文，所以不改它们，只在终端显示时翻译：按下面的句式表逐条匹配，`%` 处是名字、类型等
//! 原样保留的部分（它们本身若也是表里的句子，比如 “the end of the line”，会一并翻译）。
//! 整句对不上时按 “; ” 拆开分别翻译；仍然对不上的部分保留英文，不会丢信息。
//!
//! 语言：`ZAMAK_LANG=zh`（或 zh-CN、zh_CN）用中文，`ZAMAK_LANG=en` 用英文；不设时跟随系统语言
//! （Unix 看 LC_ALL、LC_MESSAGES、LANG，Windows 看用户界面语言）。`--json` 的输出始终是英文，
//! 给编辑器与 AI 工具一个稳定的格式。

use std::sync::OnceLock;

/// 当前是否用中文显示报错。
pub fn enabled() -> bool {
    static ZH: OnceLock<bool> = OnceLock::new();
    *ZH.get_or_init(|| match std::env::var("ZAMAK_LANG") {
        Ok(lang) if !lang.trim().is_empty() => chinese(&lang),
        _ => system_chinese(),
    })
}

fn chinese(lang: &str) -> bool {
    lang.trim().to_ascii_lowercase().starts_with("zh")
}

fn system_chinese() -> bool {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                return chinese(&value);
            }
        }
    }
    windows_chinese()
}

#[cfg(windows)]
fn windows_chinese() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    // 语言 ID 的低 10 位是主语言，0x04 是中文（简体与繁体都算）。
    unsafe { GetUserDefaultUILanguage() & 0x3ff == 0x04 }
}

#[cfg(not(windows))]
fn windows_chinese() -> bool {
    false
}

/// 终端上的固定用语。
pub fn word(en: &'static str) -> &'static str {
    if !enabled() {
        return en;
    }
    match en {
        "error" => "错误",
        "help" => "提示",
        _ => en,
    }
}

/// `zam: N errors`。
pub fn error_count(count: usize) -> String {
    if enabled() {
        format!("zam：共 {count} 个错误")
    } else {
        format!("zam: {count} errors")
    }
}

/// 翻译一条报错或建议；不是中文模式时原样返回。
pub fn text(en: &str) -> String {
    if !enabled() {
        return en.to_string();
    }
    translate(en)
}

/// `zam --help`：每个子命令一行说明，跟随 `ZAMAK_LANG`。末尾的换行由调用方负责。
pub fn help(version: &str) -> String {
    // （命令与参数, 英文说明, 中文说明）
    const COMMANDS: &[(&str, &str, &str)] = &[
        (
            "check [project|file]",
            "type-check without building",
            "只做类型与所有权检查，不构建",
        ),
        (
            "build [project|file]",
            "compile into a native executable",
            "编译成原生可执行文件",
        ),
        (
            "run [project|file] [-- args...]",
            "build and run; arguments after -- go to the program",
            "构建并运行；-- 之后的参数交给程序",
        ),
        (
            "test [project|file] [filter]",
            "run the test_ functions",
            "运行所有 test_ 开头的测试函数",
        ),
        (
            "fmt [project|file]",
            "reindent the sources in place",
            "就地整理缩进",
        ),
        (
            "new <directory> [--template ai]",
            "create a project skeleton",
            "新建项目骨架",
        ),
        (
            "add <name> [version|git-url|directory] [rev]",
            "add a dependency to zam.toml",
            "给 zam.toml 添加依赖",
        ),
        ("remove <name>", "remove a dependency", "删除一个依赖"),
        (
            "update [project]",
            "re-resolve the git dependencies",
            "重新解析 git 依赖",
        ),
        (
            "search [word]",
            "search the package index",
            "在包索引里搜索",
        ),
        (
            "publish [project]",
            "publish a version to the package index",
            "把当前版本发布到包索引",
        ),
        (
            "lsp",
            "run the language server on stdio",
            "在 stdio 上启动语言服务器",
        ),
        (
            "mcp",
            "run the MCP server for AI coding agents",
            "启动给 AI 编程助手用的 MCP 服务器",
        ),
        (
            "upgrade",
            "update an npm-installed zam",
            "更新用 npm 安装的 zam",
        ),
        (
            "demo",
            "write and run a small sample project",
            "生成并运行一个示例项目",
        ),
        ("--version", "print the version", "显示版本号"),
    ];
    let chinese = enabled();
    let mut out = format!("zam {version}\n");
    for (usage, en, zh) in COMMANDS {
        let line = if chinese { zh } else { en };
        out.push_str(&format!("  zam {usage:<48}{line}\n"));
    }
    if chinese {
        out.push_str(
            "\n环境变量：ZAMAK_LANG=zh|en 切换界面语言，ZAMAK_JOBS 控制并行编译数，ZAMAK_CACHE_DIR 指定缓存目录。\n",
        );
    } else {
        out.push_str(
            "\nEnvironment: ZAMAK_LANG=zh|en picks the language, ZAMAK_JOBS controls parallel compiles, ZAMAK_CACHE_DIR sets the cache directory.\n",
        );
    }
    out
}

fn translate(en: &str) -> String {
    if let Some(zh) = whole(en) {
        return zh;
    }
    if en.contains("; ") {
        // 句子里带着补充说明（“…; 改法”）：分开翻译。但 `[T; N]` 这类类型写法里的分号不是
        // 句子分隔符，照抄会让中文里冒出 `[T；N]`；写成方括号里且有 `;` 时整句不拆。
        if en.split_once('[').is_some_and(|(_, rest)| {
            rest.split_once(']')
                .is_some_and(|(inner, _)| inner.contains(';'))
        }) {
            return en.to_string();
        }
        return en.split("; ").map(translate).collect::<Vec<_>>().join("；");
    }
    // 位置或模块前缀（“src/main.zm: …”“geo: …”）：保留前缀，翻译后半句。
    if let Some((head, tail)) = en.split_once(": ") {
        if !head.contains(' ') {
            if let Some(zh) = whole(tail) {
                return format!("{head}：{zh}");
            }
        }
    }
    en.to_string()
}

/// 整句匹配句式表；有多条能匹配时取原文字面部分最长的那条（最具体）。
fn whole(en: &str) -> Option<String> {
    let mut best: Option<(usize, &str, Vec<&str>)> = None;
    for (pattern, zh) in TABLE {
        if let Some(captures) = capture(pattern, en) {
            let literal = pattern.len() - captures.len();
            if best.as_ref().is_none_or(|(len, ..)| literal > *len) {
                best = Some((literal, zh, captures));
            }
        }
    }
    let (_, zh, captures) = best?;
    let mut out = zh.to_string();
    for (index, value) in captures.iter().enumerate() {
        // 占位处若本身也是表里的句子（如 “the end of the line”），一并翻译。
        let shown = translate(value);
        out = out.replace(&format!("{{{index}}}"), &shown);
    }
    Some(tidy(&out))
}

/// 占位两边为英文名字留的空格，换进来的是中文时去掉（“却是 文件结尾” → “却是文件结尾”）。
fn tidy(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let cjk =
        |c: char| ('\u{3000}'..='\u{9FFF}').contains(&c) || ('\u{FF00}'..='\u{FFEF}').contains(&c);
    chars
        .iter()
        .enumerate()
        .filter(|&(i, &c)| {
            !(c == ' ' && i > 0 && i + 1 < chars.len() && cjk(chars[i - 1]) && cjk(chars[i + 1]))
        })
        .map(|(_, &c)| c)
        .collect()
}

/// `pattern` 里的 `%` 匹配任意非空文字；字面部分按从左到右第一次出现对齐。
fn capture<'a>(pattern: &str, text: &'a str) -> Option<Vec<&'a str>> {
    let parts: Vec<&str> = pattern.split('%').collect();
    let mut rest = text.strip_prefix(parts[0])?;
    let mut captures = Vec::new();
    for (index, part) in parts.iter().enumerate().skip(1) {
        let last = index == parts.len() - 1;
        let at = if last {
            if part.is_empty() {
                rest.len()
            } else {
                rest.strip_suffix(part).map(str::len)?
            }
        } else {
            // 占位至少一个字符。
            rest.get(1..)?.find(part)? + 1
        };
        if at == 0 {
            return None;
        }
        if rest[..at].contains("; ") {
            return None;
        }
        captures.push(&rest[..at]);
        rest = &rest[at + part.len()..];
    }
    (parts.len() > 1 || rest.is_empty()).then_some(captures)
}

/// 句式表：英文原文（`%` 为原样保留的部分；原文里没有 %，花括号又常出现在报错里）→ 中文（`{0}`、`{1}`… 依次对应）。
/// 写法：主动、简短；代码、类型名与命令保留原样，中英文之间留空格。
static TABLE: &[(&str, &str)] = &[
    // ---- 词法与语法 ----
    ("unexpected character %", "无法识别的字符 {0}"),
    ("unexpected character %: comments start with #", "无法识别的字符 {0}：注释以 # 开头"),
    ("unsupported string escape", "不支持的字符串转义"),
    ("unsupported character escape", "不支持的字符转义"),
    ("character literal must contain exactly one character", "字符字面量里只能有一个字符"),
    ("digit separators must occur between digits", "数字分隔符只能写在两个数字之间"),
    ("integer separators must occur between digits", "整数分隔符只能写在两个数字之间"),
    ("expected digits in the f64 exponent, such as 1e9 or 2.5e-3", "f64 的指数部分缺少数字，例如 1e9 或 2.5e-3"),
    ("invalid base-% integer literal %", "{1} 不是合法的 {0} 进制整数"),
    ("i64 literal overflow", "整数字面量超出 i64 的范围"),
    ("f64 literal is out of range", "f64 字面量超出范围"),
    ("unterminated string literal", "字符串没有结束引号"),
    ("unterminated character literal", "字符字面量没有结束引号"),
    ("unterminated { in string literal", "字符串里的 { 没有配对的 }"),
    ("unmatched } in string literal", "字符串里的 } 没有配对的 {"),
    ("nested { in string interpolation", "字符串插值里不能再嵌套 {"),
    ("empty {} in string literal", "字符串里有空的 {}"),
    ("empty {} in string literal; write {} for literal braces", "字符串里有空的 {}；要输出大括号本身，请写 {}"),
    ("byte_len returns i64", "byte_len 返回 i64"),
    ("char_len returns i64", "char_len 返回 i64"),
    ("equal returns bool", "equal 返回 bool"),
    ("array index out of bounds", "数组下标越界"),
    ("write {{}} for literal braces", "要输出大括号本身，写 {{}}"),
    ("unsupported interpolation {%}: expected one expression", "不支持的插值 {{0}}：花括号里只能有一个表达式"),
    ("in interpolation {%}: %", "插值 {{0}} 中：{1}"),
    ("cannot interpolate this value", "这个值不能插进字符串"),
    ("'%' was never closed", "'{0}' 没有闭合"),
    ("indentation contains a tab", "缩进里有制表符"),
    ("run `zam fmt` to replace tabs with spaces", "运行 `zam fmt` 把制表符换成空格"),
    ("unindent does not match any outer indentation level", "取消缩进后的位置与外层任何一级缩进都对不上"),
    ("unexpected indent", "多余的缩进"),
    ("indent only the lines inside a block that ends with ':'", "只有以 ':' 结尾的那一行下面的代码块才缩进"),
    ("top-level declarations start at the beginning of the line", "顶层声明从行首开始写"),
    ("expected an indented block after the % line", "{0} 这一行后面需要一个缩进的代码块"),
    ("expected ':' at the end of the % line, found %", "{0} 这一行末尾应该是 ':'，却是 {1}"),
    ("expected ':' at the end of the % line", "{0} 这一行末尾应该是 ':'"),
    ("blocks are indented, not wrapped in braces", "代码块靠缩进表示，不用大括号"),
    ("expected ':' after the match pattern", "match 的模式后面应该是 ':'"),
    ("arms are written Circle(r): value, not =>", "分支写成 Circle(r): 值，不用 =>"),
    ("expected ':' and an expression after the parameters of fn(...)", "fn(...) 的参数后面应该是 ':' 和一个表达式"),
    ("expected `..` in for range: for name in start..end, or iterate a list with for x in items", "for 的区间缺少 `..`：写成 for name in start..end，或用 for x in items 遍历列表"),
    ("expected `==` to compare in the % condition", "{0} 的条件里比较要用 `==`"),
    ("a single `=` assigns a value", "单个 `=` 是赋值"),
    ("expected `elif` instead of `else if`", "应该写 `elif`，不是 `else if`"),
    ("expected `else` in a if c else b, found %", "a if c else b 里缺少 `else`，却是 {0}"),
    ("expected `in` after the for loop variable", "for 的循环变量后面应该是 `in`"),
    ("expected `not` instead of `!`", "应该写 `not`，不是 `!`"),
    ("`!` only marks error types such as i64!string", "`!` 只用来标记错误类型，例如 i64!string"),
    ("expected `%` instead of `%`", "应该写 `{0}`，不是 `{1}`"),
    ("expected ( after .%", ".{0} 后面应该是 ("),
    (r#"a string has methods such as "a,b".split(",")"#, r#"字符串有这样的方法："a,b".split(",")"#),
    ("expected a field after &xs[..]", "&xs[..] 后面应该跟一个字段"),
    ("references cannot point at call results", "借用不能指向函数调用的结果"),
    ("expected a single value: a match expression arm holds a single value", "这里只能是一个值：match 表达式的每个分支只能是一个值"),
    ("compute it with let before the match, or write the match as a statement", "先在 match 前用 let 算好，或把 match 写成语句"),
    ("expected a single value: an if expression branch holds a single value", "这里只能是一个值：if 表达式的每个分支只能是一个值"),
    ("compute it with let before the if", "先在 if 前用 let 算好"),
    ("expected a variable, field or index before %=", "{0}= 前面应该是变量、字段或下标"),
    ("expected an enum variant such as Circle(r), or _, in a match arm", "match 的分支应该是枚举变体（例如 Circle(r)）或 _"),
    ("found %", "却是 {0}"),
    ("expected an operand: `not` cannot follow a comparison or arithmetic operator", "缺少操作数：`not` 不能直接跟在比较或算术运算符后面"),
    ("wrap it in parentheses, such as a == (not b)", "加上括号，例如 a == (not b)"),
    ("expected at least one variant in enum %", "枚举 {0} 至少要有一个变体"),
    ("expected else: an if expression needs an else branch", "缺少 else：if 表达式必须有 else 分支"),
    ("expected field names in the struct literal, such as %(x: 1, y: 2)", "结构体字面量要写字段名，例如 {0}(x: 1, y: 2)"),
    ("expected fn after extern, such as extern fn sqrt(x: f64) -> f64", "extern 后面应该是 fn，例如 extern fn sqrt(x: f64) -> f64"),
    ("expected function call", "这里应该是一个函数调用"),
    ("expected function call: this line is only a value, so it does nothing", "这里应该是一个函数调用：这一行只有一个值，什么也不做"),
    ("a catch block gives a fallback value only when the value is the whole block, such as f() catch e: 0", "只有当 catch 块里只有这个值时，它才是回退值，例如 f() catch e: 0"),
    ("use the value in a let or a call, or make the value the only line of the catch block", "把这个值用在 let 或函数调用里，或者让它成为 catch 块里唯一的一行"),
    ("finish the loop or the borrow first: collect the changes in another list, then apply them after the loop", "先结束循环或借用：把要做的修改记在另一个列表里，循环结束后再改"),
    ("expected identifier, found the reserved word %", "这里应该是名字，却是保留字 {0}"),
    ("expected identifier, found %", "这里应该是名字，却是 {0}"),
    ("expected newline between statements, found %", "两条语句之间应该换行，却是 {0}"),
    ("expected the match arms on the following lines, indented", "match 的分支应该写在下面几行，并且缩进"),
    ("expected the variants of enum % on the following lines, indented", "枚举 {0} 的变体应该写在下面几行，并且缩进"),
    ("expected use, const, struct, enum, fn or extern fn declaration", "顶层只能写 use、const、struct、enum、fn 或 extern fn 声明"),
    ("expected %, found %", "这里应该是 {0}，却是 {1}"),
    ("unexpected %: it must follow an if block at the same indentation", "多余的 {0}：它必须紧跟在同一缩进的 if 代码块后面"),
    ("the end of the block", "代码块结尾"),
    ("the end of the file", "文件结尾"),
    ("the end of the line", "行尾"),
    ("an indented line", "一行缩进的代码"),
    ("the number %", "数字 {0}"),
    (r#"the string "%""#, r#"字符串 "{0}""#),
    ("Symbol('%')", "符号 '{0}'"),
    ("Name(\"%\")", "名字 {0}"),
    ("an anonymous fn holds one expression on the same line", "匿名函数只能在同一行写一个表达式"),
    ("declare a named fn for longer code", "代码较长时请声明一个具名函数"),
    ("unknown decorator @%", "未知的装饰器 @{0}"),
    ("the only decorator is @wrapping", "目前只有 @wrapping 这一个装饰器"),
    ("match patterns are literals such as 1, -2, 'a', \"text\" or true, without interpolation", "match 的模式只能是字面量，例如 1、-2、'a'、\"text\" 或 true，不能插值"),
    ("only literal patterns can be combined with |", "只有字面量模式能用 | 合并"),
    ("write one arm per variant", "每个变体写一个分支"),
    ("this match mixes enum variants and literal patterns", "这个 match 把枚举变体和字面量模式混在一起了"),
    ("a match needs at least one arm", "match 至少要有一个分支"),
    ("a literal arm lists at least one value", "字面量分支至少要列出一个值"),
    ("a match expression needs a _ arm for the values no pattern lists", "match 表达式要有一个 _ 分支，处理其他分支没有列出的值"),

    // ---- 名字与声明 ----
    ("unknown variable %", "未知的变量 {0}"),
    ("unknown function %", "未知的函数 {0}"),
    ("unknown function %: %", "未知的函数 {0}：{1}"),
    ("unknown struct %", "未知的结构体 {0}"),
    ("unknown struct", "未知的结构体"),
    ("unknown field %", "未知的字段 {0}"),
    ("unknown variant % of %", "{1} 没有变体 {0}"),
    ("unknown method %", "未知的方法 {0}"),
    ("declare it inside % as fn %(&self)", "在 {0} 里声明它：fn {1}(&self)"),
    ("unknown method .% for a string", "字符串没有方法 .{0}"),
    ("string methods are the std/string functions, such as s.len(), s.upper(), s.split(\",\") or s.contains(\"x\")", "字符串的方法就是 std/string 里的函数，例如 s.len()、s.upper()、s.split(\",\")、s.contains(\"x\")"),
    ("unknown constant %", "未知的常量 {0}"),
    ("unknown type name", "未知的类型名"),
    ("unknown tuple type", "未知的元组类型"),
    ("reserved type name %", "{0} 是保留的类型名"),
    ("unknown variable math.%", "未知的变量 math.{0}"),
    ("std/math has the constants pi, e and inf", "std/math 只有 pi、e、inf 这几个常量"),
    ("ambiguous or missing import for %", "{0} 的导入缺失或有歧义"),
    ("missing or ambiguous struct import", "结构体的导入缺失或有歧义"),
    ("ambiguous string import", "string 的导入有歧义"),
    ("% requires use std/%", "{0} 需要导入 std/{1}"),
    ("add `use std/%` at the top of the file", "在文件开头加上 `use std/{0}`"),
    ("% is private", "{0} 是私有的"),
    ("private method %", "{0} 是私有方法"),
    ("declare it with pub fn", "用 pub fn 声明它"),
    ("private constant %", "{0} 是私有常量"),
    ("declare it with pub const", "用 pub const 声明它"),
    ("private struct", "私有结构体"),
    ("duplicate binding %", "{0} 重复绑定"),
    ("duplicate declaration %", "{0} 重复声明"),
    ("duplicate field %", "字段 {0} 重复"),
    ("duplicate function %", "函数 {0} 重复定义"),
    ("duplicate import %", "{0} 重复导入"),
    ("duplicate initializer %", "字段 {0} 赋值了两次"),
    ("duplicate match arm %", "match 分支 {0} 重复"),
    ("duplicate method %", "方法 {0} 重复定义"),
    ("duplicate parameter %", "参数 {0} 重复"),
    ("duplicate type parameter %", "类型参数 {0} 重复"),
    ("duplicate variant %", "变体 {0} 重复"),
    ("duplicate module %", "模块 {0} 重复"),
    ("% is already declared", "{0} 已经声明过了"),
    ("give the fn parameter another name", "给 fn 的参数换个名字"),
    ("% is already a constant", "{0} 已经是常量了"),
    ("choose another name for the local", "给局部变量换个名字"),
    ("struct names must start with an uppercase letter", "结构体名必须以大写字母开头"),
    ("enum names must start with an uppercase letter", "枚举名必须以大写字母开头"),
    ("enum variant names must start with an uppercase letter", "枚举变体名必须以大写字母开头"),
    ("struct and enum names are uppercase", "结构体和枚举名以大写字母开头"),
    ("function names must start with a lowercase letter: %", "函数名必须以小写字母开头：{0}"),
    ("uppercase names are for structs and enums", "大写开头的名字留给结构体和枚举"),
    ("wrap the C function in a small C function with a lowercase name", "用一个小写名字的小 C 函数把它包一层"),
    ("type parameter names start with an uppercase letter, such as T", "类型参数名以大写字母开头，例如 T"),
    ("constant names are written in UPPER_CASE, such as %", "常量名全部大写，例如 {0}"),
    ("constants can only use literals, earlier constants and operators", "常量只能用字面量、前面声明过的常量和运算符"),
    ("cannot assign to the constant %", "不能给常量 {0} 赋值"),
    ("cannot borrow the constant %", "不能借用常量 {0}"),
    ("only string constants can be passed as &string", "只有字符串常量能当作 &string 传"),
    ("self can only be the first parameter of a method declared inside a struct or enum", "self 只能是 struct 或 enum 里声明的方法的第一个参数"),
    ("% has no self parameter", "{0} 没有 self 参数"),
    ("call it on the type, such as Point.%(...)", "通过类型调用它，例如 Point.{0}(...)"),
    ("missing main function", "缺少 main 函数"),
    ("entry module requires fn main()", "入口模块需要 fn main()"),
    ("main must have no parameters or return value", "main 不能有参数，也不能有返回值"),
    ("write the return type of this fn, such as -> i64", "写出这个 fn 的返回类型，例如 -> i64"),
    ("a fn declared inside a function cannot have type parameters", "函数里声明的 fn 不能有类型参数"),
    ("declare % at the top level", "把 {0} 声明在模块顶层"),
    ("% cannot call itself: a fn declared inside a function is not recursive yet", "{0} 不能调用自己：函数里声明的 fn 暂时不能递归"),
    ("% uses local variables, so it cannot be declared mut", "{0} 用到了局部变量，所以不能声明成 mut"),
    ("bind another fn with a new name", "用新名字绑定另一个 fn"),

    // ---- 类型 ----
    ("type mismatch", "类型不匹配"),
    ("type mismatch: %", "类型不匹配：{0}"),
    ("type mismatch in constant", "常量的类型不匹配"),
    ("string type mismatch", "字符串类型不匹配"),
    ("struct type mismatch", "结构体类型不匹配"),
    ("field type mismatch", "字段类型不匹配"),
    ("array length or element type mismatch", "数组的长度或元素类型不匹配"),
    ("return type mismatch for %", "{0} 的返回类型不匹配"),
    ("borrow type mismatch for %", "{0} 的借用类型不匹配"),
    ("argument count mismatch for %", "{0} 的参数个数不对"),
    ("argument count mismatch for %: it takes % value(s)", "{0} 的参数个数不对：它需要 {1} 个值"),
    ("argument count mismatch for copy: copy takes one borrowed value, such as copy(&x)", "copy 的参数个数不对：copy 只接受一个借用，例如 copy(&x)"),
    ("argument count mismatch: the function takes % parameter(s), but this fn has %", "参数个数不对：函数类型有 {0} 个参数，这个 fn 有 {1} 个"),
    ("type argument count mismatch for %: it takes %", "{0} 的类型实参个数不对：它需要 {1} 个"),
    ("% takes % type argument(s)", "{0} 需要 {1} 个类型实参"),
    ("write it as %[%]", "写成 {0}[{1}]"),
    ("% is not generic", "{0} 不是泛型"),
    ("write it without [...]", "去掉 [...]"),
    ("cannot infer % for %: % must appear in a parameter type", "推断不出 {1} 的 {0}：{2} 必须出现在某个参数的类型里"),
    ("cannot infer % for %", "推断不出 {1} 的 {0}"),
    ("annotate the type, such as let x: %[i64] = ...", "标注类型，例如 let x: {0}[i64] = ..."),
    ("cannot infer the type of parameter %", "推断不出参数 {0} 的类型"),
    ("write it, such as fn(%: i64): ...", "把类型写出来，例如 fn({0}: i64): ..."),
    ("generic function % needs a known function type to be used as a value", "泛型函数 {0} 要当作值使用，需要已知的函数类型"),
    ("annotate it, such as let f: fn(i64) -> i64 = %", "标注类型，例如 let f: fn(i64) -> i64 = {0}"),
    ("too many generic type instances", "泛型类型的实例太多"),
    ("a generic type may expand without end", "某个泛型类型可能在无限展开"),
    ("type arguments are owned types, such as Option[string]", "类型实参必须是有所有权的类型，例如 Option[string]"),
    ("/ always gives f64 (as in Python)", "/ 的结果总是 f64（与 Python 一样）"),
    ("use // for integer division", "整数除法用 //"),
    ("% is f64 but i64 is expected", "{0} 是 f64，这里需要 i64"),
    ("% is i64 but f64 is expected", "{0} 是 i64，这里需要 f64"),
    ("convert with i64(%)", "用 i64({0}) 转换"),
    ("convert with f64(%)", "用 f64({0}) 转换"),
    ("this f64 expression is used as i64", "这个 f64 表达式被当作 i64 使用"),
    ("this i64 expression is used as f64", "这个 i64 表达式被当作 f64 使用"),
    ("convert it with i64(...)", "用 i64(...) 转换"),
    ("convert it with f64(...)", "用 f64(...) 转换"),
    ("expected f64, found the integer %", "这里需要 f64，却是整数 {0}"),
    ("write %.0", "写成 {0}.0"),
    ("expected i64, found an f64 literal", "这里需要 i64，却是 f64 字面量"),
    ("convert with i64(x)", "用 i64(x) 转换"),
    ("% mixes i64 and f64", "{0} 两边一个是 i64、一个是 f64"),
    ("convert one side with f64(n) or i64(x)", "用 f64(n) 或 i64(x) 转换其中一边"),
    ("write the integer % as %.0", "把整数 {0} 写成 {1}.0"),
    ("% requires i64 operands", "{0} 的两边必须是 i64"),
    ("combine conditions with and, or, not", "组合条件用 and、or、not"),
    ("~ requires an i64 operand", "~ 只能用于 i64"),
    ("% compares two values of the same type", "{0} 只能比较同一类型的两个值"),
    ("% compares i64, f64, bool and string values", "{0} 只能比较 i64、f64、bool 和 string"),
    ("structs, enums, lists and maps only support == and !=", "结构体、枚举、列表和字典只支持 == 和 !="),
    ("comparison requires i64, f64, bool or string", "比较的两边必须是 i64、f64、bool 或 string"),
    ("expected a string", "这里需要字符串"),
    ("to join other values, interpolate them as in \"{s}{n}\"", "要拼接其他值，用插值写法，例如 \"{s}{n}\""),
    ("constant % does not have the declared type", "常量 {0} 的值与声明的类型不符"),
    ("constant % is declared f64 but its value is i64", "常量 {0} 声明为 f64，值却是 i64"),
    ("write it as 1.0", "写成 1.0 这样"),
    ("container element types differ", "容器的元素类型不一致"),
    ("element type differs", "元素类型不一致"),
    ("an empty list literal needs a type annotation, such as let xs: [i64] = []", "空列表需要标注类型，例如 let xs: [i64] = []"),
    ("an empty map literal needs a type annotation, such as let m: {string: i64} = {}", "空字典需要标注类型，例如 let m: {string: i64} = {}"),
    ("unexpected map literal", "这里不该是字典字面量"),
    ("a tuple is not expected here", "这里不该是元组"),
    ("expected a tuple of % items, found %", "这里需要 {0} 项的元组，却是 {1}"),
    ("let (a, b) = ... needs a tuple value", "let (a, b) = ... 的右边必须是元组"),
    ("the tuple has % items, but the pattern names %", "元组有 {0} 项，模式却写了 {1} 个名字"),
    ("a tuple type lists at least two types, such as (i64, string)", "元组类型至少要有两项，例如 (i64, string)"),
    ("tuples hold owned values, such as (string, i64)", "元组只能装有所有权的值，例如 (string, i64)"),
    ("tuples hold owned values", "元组只能装有所有权的值"),
    ("copy borrowed values first, such as string.copy(name)", "先复制借来的值，例如 string.copy(name)"),
    ("a match expression produces an owned value, not a borrow or fixed array", "match 表达式产生的是有所有权的值，不能是借用或定长数组"),
    ("an if expression produces an owned value, not a borrow or fixed array", "if 表达式产生的是有所有权的值，不能是借用或定长数组"),
    ("match requires an enum value, or literal patterns such as 1, \"a\" or true", "match 需要枚举值，或者 1、\"a\"、true 这样的字面量模式"),
    ("match on % does not cover %", "对 {0} 的 match 没有覆盖 {1}"),
    ("add an arm for each or a _ arm", "给每一个都加上分支，或者加一个 _ 分支"),
    ("unreachable match arm after _", "_ 后面的分支永远不会执行"),
    ("% is a struct, not an enum", "{0} 是结构体，不是枚举"),
    ("% is an enum", "{0} 是枚举"),
    ("construct a variant with %.Variant(...)", "用 {0}.Variant(...) 构造一个变体"),
    ("% has % field(s), but the pattern binds %", "{0} 有 {1} 个字段，模式却绑定了 {2} 个"),
    ("%.% is a %", "{0}.{1} 属于 {2}"),
    ("% is an enum variant", "{0} 是枚举变体"),
    ("pass its values by position, such as %(1.0)", "按位置传值，例如 {0}(1.0)"),
    ("% of % must be an owned type such as i64, string or a list", "{1} 的 {0} 必须是有所有权的类型，例如 i64、string 或列表"),
    ("% cannot be a fixed array", "{0} 不能是定长数组"),
    ("use a list, such as let xs: [i64] = [1, 2]", "改用列表，例如 let xs: [i64] = [1, 2]"),
    ("% cannot be a function parameter that may capture local variables", "{0} 不能是可能捕获局部变量的函数参数"),
    ("give the parameter a function type instead", "改为给参数写一个函数类型"),
    ("&mut T cannot be %", "&mut T 不能是 {0}"),
    ("numbers and booleans are passed by value", "数字和布尔值按值传递"),
    ("copy is for strings, lists, maps, structs and enums", "copy 只用于字符串、列表、字典、结构体和枚举"),
    ("i64, f64 and bool values are already copied by assignment, such as let b = a", "i64、f64 和 bool 赋值时就已经复制了，例如 let b = a"),
    ("copy returns a value of the borrowed type", "copy 返回的是被借用的那种类型的值"),
    ("for x in VALUE needs a list or map", "for x in 值 需要列表或字典"),
    ("to count, write a range such as for i in 0..n", "要计数，写一个区间，例如 for i in 0..n"),
    ("to count, write for i in 0..%", "要计数，写 for i in 0..{0}"),
    ("two loop variables need a list or map: for i, x in items or for k, v in m", "两个循环变量需要列表或字典：for i, x in items 或 for k, v in m"),
    ("a range has one loop variable: for i in range(n)", "区间只有一个循环变量：for i in range(n)"),
    ("range takes one or two arguments, such as range(n) or range(a, b)", "range 接受一个或两个参数，例如 range(n) 或 range(a, b)"),
    ("for a step, use while", "需要步长时用 while"),
    ("function values cannot be printed", "函数值不能打印"),
    ("call the function and print its result", "调用函数，打印它的结果"),
    ("only a function value can be called", "只有函数值才能调用"),
    ("% is not a function", "{0} 不是函数"),
    ("% is not a list or map", "{0} 不是列表或字典"),
    ("%[..] is not a list or map, so it cannot be indexed again", "{0}[..] 不是列表或字典，不能再取下标"),
    ("% has no method %", "{0} 没有方法 {1}"),
    ("this value has no method %", "这个值没有方法 {0}"),
    ("methods are only available on lists and maps", "只有列表和字典有方法"),
    ("parameter % of this fn has a different type than the expected function type", "这个 fn 的参数 {0} 与期望的函数类型不一致"),
    ("this fn returns a different type than the expected function type", "这个 fn 的返回类型与期望的函数类型不一致"),
    ("this fn has a different function type", "这个 fn 的函数类型不一致"),
    ("% has a different function type", "{0} 的函数类型不一致"),
    ("println has no value, but the function type returns one", "println 没有值，但函数类型需要返回值"),
    ("an anonymous fn is a function value, but another type is expected", "匿名函数是函数值，但这里需要别的类型"),
    ("% can fail, but the expected function type has no !E", "{0} 可能失败，但期望的函数类型没有 !E"),
    ("add the error type, such as fn(&string) -> i64!string", "加上错误类型，例如 fn(&string) -> i64!string"),
    ("array indexing returns i64", "数组下标取出的是 i64"),
    ("index requires an array or list", "只有数组或列表能取下标"),
    ("index requires an array, list or map", "只有数组、列表或字典能取下标"),
    ("take one character of a string with s.slice(i, i + 1)", "取字符串里的一个字符用 s.slice(i, i + 1)"),
    ("index requires i64 array", "这里需要 i64 数组"),
    ("index at most two levels deep, such as grid[i][j]", "下标最多两层，例如 grid[i][j]"),
    ("borrow a row with for row in &grid", "用 for row in &grid 借出一行"),
    ("indexing twice, such as grid[i][j], reads i64, f64, bool and string elements", "两层下标（例如 grid[i][j]）只能读 i64、f64、bool 和 string 元素"),
    ("borrow deeper levels with for row in &grid", "更深的层用 for row in &grid 借出来"),
    ("field access requires a struct", "只有结构体能访问字段"),
    ("missing struct field", "缺少结构体字段"),
    ("recursive struct layout: %", "结构体 {0} 直接包含了自己"),
    ("array length must be an integer", "数组长度必须是整数"),
    ("array length must be between 1 and 1024", "数组长度必须在 1 到 1024 之间"),
    ("invalid array length", "数组长度不合法"),
    ("only [i64; N] arrays are supported", "只支持 [i64; N] 数组"),
    ("arrays are only supported as local values", "数组只能作为局部变量"),
    ("array values cannot be moved or copied yet", "数组暂时不能移动或复制"),
    ("fixed arrays cannot be used inside an anonymous fn", "匿名函数里不能用定长数组"),
    ("use a list", "改用列表"),
    ("cannot print void or a fixed array", "不能打印空值或定长数组"),
    ("println requires one string argument", "println 需要一个字符串参数"),
    ("void function cannot return a value", "没有返回类型的函数不能返回值"),
    ("missing return: this fn returns a value, but the end of its body has none", "缺少返回值：这个 fn 要返回值，但函数体末尾没有"),
    ("end every path with a value or return", "每条路径都要以一个值或 return 结尾"),
    ("missing string return value", "缺少字符串返回值"),
    ("unreachable statement after return", "return 后面的语句永远不会执行"),
    ("lists support len, push, pop, insert, remove, swap, reverse, clear, extend, sort, sorted, sort_by, contains, index_of, slice, sum, min and max", "列表支持 len、push、pop、insert、remove、swap、reverse、clear、extend、sort、sorted、sort_by、contains、index_of、slice、sum、min、max"),
    ("maps support len, contains, get, remove, keys, values and clear", "字典支持 len、contains、get、remove、keys、values、clear"),
    ("sort, sorted, contains, index_of, slice, min and max need i64, f64, bool or string elements", "sort、sorted、contains、index_of、slice、min、max 需要 i64、f64、bool 或 string 元素"),
    ("sum needs i64 or f64 elements", "sum 需要 i64 或 f64 元素"),
    ("struct elements have no natural order", "结构体元素没有天然的顺序"),
    ("use sort_by, or a loop with == to find one", "用 sort_by 排序，或者写个循环用 == 查找"),
    ("struct values cannot be copied", "结构体值不能复制"),
    ("iterate the keys instead", "改为遍历键"),
    ("list and map values cannot be copied by get", "get 不能复制列表或字典的值"),
    ("check %.contains(k), then borrow &%[k] or call %[k].len()", "先检查 {0}.contains(k)，再借用 &{1}[k] 或调用 {2}[k].len()"),
    ("check %.contains(k), then read %[k].field or borrow &%[k]", "先检查 {0}.contains(k)，再读 {1}[k].field 或借用 &{2}[k]"),
    ("unsupported error type", "不支持的错误类型"),
    ("errors are a string, a struct or an enum, such as -> i64!ParseError", "错误类型只能是字符串、结构体或枚举，例如 -> i64!ParseError"),
    ("unsupported element type %", "不支持的元素类型 {0}"),
    ("unsupported element type: fixed arrays cannot be stored in lists or maps", "不支持的元素类型：定长数组不能放进列表或字典"),
    ("unsupported list element type in for loop", "for 循环不支持这种列表元素类型"),
    ("unsupported list element type", "不支持的列表元素类型"),
    ("expected [i64], [f64], [bool], [string] or a struct such as [Point]", "应该是 [i64]、[f64]、[bool]、[string] 或结构体列表，例如 [Point]"),
    ("unsupported map key type", "不支持的字典键类型"),
    ("expected i64 or string keys", "键只能是 i64 或 string"),
    ("unsupported map literal", "不支持的字典字面量"),
    ("keys are i64 or string, values are i64, f64, bool or string", "键是 i64 或 string，值是 i64、f64、bool 或 string"),
    ("unsupported map value type", "不支持的字典值类型"),
    ("expected i64, f64, bool, string, a struct, a list or a map", "应该是 i64、f64、bool、string、结构体、列表或字典"),
    ("unsupported type", "不支持的类型"),
    ("expected string, i64, f64, bool or a struct", "应该是 string、i64、f64、bool 或结构体"),
    ("unsupported compound assignment index: it must not call functions", "不支持这种复合赋值的下标：下标里不能调用函数"),
    ("bind it with let first", "先用 let 绑定它"),
    ("scalar references are not supported", "不支持对数字或布尔值的借用"),
    ("copy the number instead, such as let n = xs[i]", "直接复制数字，例如 let n = xs[i]"),
    ("reference types are borrows", "引用类型就是借用"),
    ("function types return owned values", "函数类型返回有所有权的值"),
    ("enum payloads are owned values", "枚举的载荷必须是有所有权的值"),
    ("integer arithmetic error in constant", "常量里的整数运算出错"),
    ("division by zero in a constant", "常量里除以零"),
    ("% cannot fail, so it needs no ? or catch", "{0} 不会失败，不需要 ? 或 catch"),

    // ---- 所有权与借用 ----
    ("use after move: %", "使用已移走的值：{0}"),
    ("use after move: % (the fn % uses it)", "使用已移走的值：{0}（fn {1} 用到了它）"),
    ("loop moves value without reinitialization: %", "循环移走了 {0}，却没有重新赋值"),
    ("loop jump moves value without reinitialization: %", "跳转前移走了 {0}，却没有重新赋值"),
    ("loop jump outside while", "break 或 continue 不在循环里"),
    ("cannot move borrowed value %", "{0} 正被借用，不能移走"),
    ("cannot assign immutable value %", "{0} 不可变，不能赋值"),
    ("cannot mutably borrow immutable value %", "{0} 不可变，不能用 &mut 借用"),
    ("cannot mutably borrow a call result", "不能用 &mut 借用函数调用的结果"),
    ("store it with let mut first", "先用 let mut 存下来"),
    ("a borrowed local cannot be mut", "借来的局部变量不能是 mut"),
    ("write let % = &...", "写成 let {0} = &..."),
    ("a string cannot be changed through a for loop", "不能在 for 循环里修改字符串"),
    ("build a new string instead", "改为构造一个新字符串"),
    ("for k in &mut m cannot change keys", "for k in &mut m 不能修改键"),
    ("write for k, v in &mut m to change the values", "要修改值，写 for k, v in &mut m"),
    ("borrow conflict", "借用冲突"),
    ("borrow conflict: %", "借用冲突：{0}"),
    ("borrow conflict: cannot move % during a borrow", "借用冲突：{0} 正被借用，不能移走"),
    ("borrow conflict: the fn reads % while .% changes it", "借用冲突：fn 读取 {0} 的同时 .{1} 在修改它"),
    ("borrow conflict: % (the fn % changes it)", "借用冲突：{0}（fn {1} 会修改它）"),
    ("borrow conflict: % (the fn % uses it)", "借用冲突：{0}（fn {1} 用到了它）"),
    ("borrow conflict: % is borrowed by an enclosing for loop or a borrowed local such as let x = &%[i]", "借用冲突：{0} 正被外层 for 循环或借来的局部变量（例如 let x = &{1}[i]）借用"),
    ("borrow escape: enums cannot store references", "借用逃逸：枚举不能存放借用"),
    ("borrow escape: structs cannot store references", "借用逃逸：结构体不能存放借用"),
    ("borrow escape: functions cannot return references", "借用逃逸：函数不能返回借用"),
    ("borrow escape: locals require an owned value", "借用逃逸：局部变量需要有所有权的值"),
    ("borrow escape: references only belong in call arguments", "借用逃逸：借用只能出现在调用参数里"),
    ("borrow escape: a local cannot be declared with a reference type", "借用逃逸：局部变量不能声明成引用类型"),
    ("borrow without a type, such as let % = &value", "借用时不写类型，例如 let {0} = &value"),
    ("borrow escape: an anonymous fn returns owned values", "借用逃逸：匿名函数返回的必须是有所有权的值"),
    ("copy the value, such as copy(&x)", "复制这个值，例如 copy(&x)"),
    ("borrow escape: this fn uses the local variable %, so it can only be passed directly as an argument or bound with let", "借用逃逸：这个 fn 用到了局部变量 {0}，只能直接当参数传或用 let 绑定"),
    ("borrow escape: % may capture local variables of its caller", "借用逃逸：{0} 可能捕获了调用方的局部变量"),
    ("it can only be called or passed as an argument", "它只能被调用或当参数传"),
    ("borrow escape: % uses local variables of this function, so it can only be called or passed as an argument, not stored or returned", "借用逃逸：{0} 用到了本函数的局部变量，只能调用或当参数传，不能存起来或返回"),
    ("expected explicit borrow of %", "{0} 需要显式借用"),
    ("write &mut % to let the function change it", "写 &mut {0}，让函数可以修改它"),
    ("expected explicit borrow: write copy(&x), copy(&xs[i]) or copy(&p.field)", "需要显式借用：写 copy(&x)、copy(&xs[i]) 或 copy(&p.field)"),
    ("cannot move a list or map out of %[..]", "不能把列表或字典从 {0}[..] 里移出来"),
    ("borrow it with &%[..], call methods such as %[..].len(), or read an element with %[i][j]", "用 &{0}[..] 借用，调用 {1}[..].len() 这样的方法，或用 {2}[i][j] 读元素"),
    ("cannot move a string out of %[..]", "不能把字符串从 {0}[..] 里移出来"),
    ("borrow it with &%[..] or copy it with string.copy(&%[..])", "用 &{0}[..] 借用，或用 string.copy(&{1}[..]) 复制"),
    ("cannot move a struct out of %[..]", "不能把结构体从 {0}[..] 里移出来"),
    ("borrow it with &%[..] or read a field with %[..].field", "用 &{0}[..] 借用，或用 {1}[..].field 读字段"),
    ("cannot move a struct out of %[..]: % takes self by value", "不能把结构体从 {0}[..] 里移出来：{1} 按值接收 self"),
    ("take it out with pop() or copy(&...) first", "先用 pop() 或 copy(&...) 取出来"),
    ("owned field moves are not supported", "不支持移走单个字段"),
    ("borrow it with &%[..].%", "用 &{0}[..].{1} 借用"),
    ("move the whole struct", "移走整个结构体"),

    // ---- 错误处理 ----
    ("fallible call % must be handled with ?", "{0} 可能失败，要用 ? 处理"),
    ("fallible call % must be handled with ? or catch", "{0} 可能失败，要用 ? 或 catch 处理"),
    ("fallible call this function value must be handled with ? or catch", "这个函数值可能失败，要用 ? 或 catch 处理"),
    ("fail requires a function that declares an error type", "只有声明了错误类型的函数才能 fail"),
    ("catch requires a fallible call, but % cannot fail", "catch 只用于可能失败的调用，但 {0} 不会失败"),
    ("catch requires a function call", "catch 前面必须是函数调用"),
    ("catch block for % must return a value", "{0} 的 catch 块必须返回一个值"),
    ("cannot propagate %", "不能传播 {0}"),
    ("cannot propagate the error of %", "不能把 {0} 的错误往外传"),

    // ---- extern fn ----
    ("extern fn % cannot declare an error type", "extern fn {0} 不能声明错误类型"),
    ("return a status value and check it in Zamak", "返回一个状态值，在 Zamak 里检查它"),
    ("extern fn % cannot have type parameters", "extern fn {0} 不能有类型参数"),
    ("extern fn % has no body", "extern fn {0} 没有函数体"),
    ("its code is the C function %", "它的实现就是 C 函数 {0}"),
    ("unsupported return type for extern fn %", "extern fn {0} 的返回类型不受支持"),
    ("extern functions return nothing, i64, f64, bool or string", "extern 函数只能不返回，或返回 i64、f64、bool、string"),
    ("unsupported type for parameter % of extern fn %", "extern fn {1} 的参数 {0} 类型不受支持"),
    ("extern functions take i64, f64, bool or &string, so pass other data through a small C wrapper", "extern 函数只接受 i64、f64、bool 和 &string，其他数据请经一个小 C 函数转一下"),

    // ---- 项目、模块与测试 ----
    ("missing module %", "找不到模块 {0}"),
    ("missing entry module %", "找不到入口模块 {0}"),
    ("cyclic module dependency at %", "模块 {0} 处出现循环依赖"),
    ("invalid module path %", "模块路径 {0} 不合法"),
    ("module % has the same name as the dependency %", "模块 {0} 与依赖 {1} 同名"),
    ("rename one of them", "把其中一个改名"),
    ("entry must be a .zm file", "入口必须是 .zm 文件"),
    ("entry must be inside modules", "入口必须在 modules 目录里"),
    ("entry must be UTF-8", "入口路径必须是 UTF-8"),
    ("module path must be UTF-8", "模块路径必须是 UTF-8"),
    ("% has no .zm files", "{0} 里没有 .zm 文件"),
    ("% is a library: zam.toml has no entry, so there is no program to build or run", "{0} 是一个库：zam.toml 里没有 entry，所以没有可以构建或运行的程序"),
    ("use zam check or zam test, or add entry = \"src/main.zm\" under [project]", "用 zam check 或 zam test，或者在 [project] 下加上 entry = \"src/main.zm\""),
    ("no tests found", "没有找到测试"),
    ("name test functions test_..., such as fn test_add():", "测试函数以 test_ 开头，例如 fn test_add():"),
    ("test function % must take no parameters and return nothing, such as fn %():", "测试函数 {0} 不能有参数，也不能有返回值，例如 fn {1}():"),
    ("% test(s) failed", "{0} 个测试失败"),
    ("cannot start the test program: %", "无法启动测试程序：{0}"),
    ("program failed: %", "程序运行失败：{0}"),
    ("symlink source is not supported: %", "不支持符号链接的源码：{0}"),
    ("% requires % borrowed string argument(s)", "{0} 需要 {1} 个借用的字符串参数"),
    ("%: already exists", "{0}：已经存在"),
    ("project name must be an identifier", "项目名必须是合法的标识符"),
    ("project name must be UTF-8", "项目名必须是 UTF-8"),
    ("cannot start zam: %", "无法启动 zam：{0}"),
    ("worker panicked", "构建线程意外退出"),
    ("output has no parent directory", "输出路径没有上级目录"),
    ("path-independent caching failed", "与路径无关的缓存失败"),
    ("native cache entry was not produced", "没有生成原生缓存条目"),
    ("missing native cache checksum: %", "原生缓存缺少校验和：{0}"),

    // ---- zam.toml、依赖与包索引 ----
    ("%: invalid zam.toml: %", "{0}：zam.toml 写法不对：{1}"),
    ("%: invalid zam.lock: %", "{0}：zam.lock 写法不对：{1}"),
    ("missing [project]", "缺少 [project]"),
    ("missing string project.entry", "缺少字符串 project.entry"),
    ("missing string project.modules", "缺少字符串 project.modules"),
    ("missing string project.name", "缺少字符串 project.name"),
    ("project.name must be an identifier", "project.name 必须是合法的标识符"),
    ("project.% must be a string", "project.{0} 必须是字符串"),
    ("unknown project setting", "未知的项目设置"),
    ("project paths must be nonempty relative paths without '..'", "项目路径必须是非空的相对路径，并且不含 '..'"),
    ("zam.toml accepts only [project], [dependencies] and [native]", "zam.toml 只接受 [project]、[dependencies] 和 [native]"),
    ("unknown native setting %", "未知的 native 设置 {0}"),
    ("[native] accepts sources, include, libraries and library_paths", "[native] 只接受 sources、include、libraries 和 library_paths"),
    ("native.% must be a list of strings", "native.{0} 必须是字符串列表"),
    ("native source % must be a .c file", "native 源文件 {0} 必须是 .c 文件"),
    ("invalid library name %", "库名 {0} 不合法"),
    ("write the bare name, such as \"sqlite3\" for libsqlite3 or sqlite3.lib", "只写库的名字，例如 libsqlite3 或 sqlite3.lib 写成 \"sqlite3\""),
    ("invalid dependency name %", "依赖名 {0} 不合法"),
    ("dependency % accepts only git, rev, path and version", "依赖 {0} 只接受 git、rev、path 和 version"),
    ("dependency % needs exactly one of version, git (with an optional rev) or path", "依赖 {0} 要在 version、git（可加 rev）和 path 中选且只选一种"),
    ("dependencies.% must be a string", "dependencies.{0} 必须是字符串"),
    ("dependency % already exists in zam.toml", "zam.toml 里已经有依赖 {0} 了"),
    ("dependency % is not in zam.toml", "zam.toml 里没有依赖 {0}"),
    ("dependency % is required from two different sources", "依赖 {0} 被要求从两个不同的来源获取"),
    ("use the same version, git and rev, or path everywhere", "各处使用相同的 version、git 与 rev，或相同的 path"),
    ("dependency % points to a package named %", "依赖 {0} 指向的包名叫 {1}"),
    ("use the same name", "两边用同一个名字"),
    ("dependency %: %", "依赖 {0}：{1}"),
    ("a version without version", "某个版本条目缺少 version"),
    ("add the public repository as the origin remote before publishing", "发布前先把公开仓库添加为 origin"),
    ("publish a package from a git repository; run git init and commit it first", "包要发布自 git 仓库：先执行 git init 并提交"),
    ("add version = \"%\" under [project] in zam.toml before publishing", "发布前在 zam.toml 的 [project] 下加上 version = \"{0}\""),
    ("dependency versions did not settle", "依赖版本无法确定下来"),
    ("pin the packages that keep changing with \"=1.2.3\"", "用 \"=1.2.3\" 把一直在变的包固定下来"),
    ("dependency %: directory % not found (a path dependency inside a git package must stay inside that repository)", "依赖 {0}：找不到目录 {1}（git 包里的路径依赖必须在同一个仓库里）"),
    ("dependency %: % has no zam.toml at %", "依赖 {0}：{1} 在 {2} 没有 zam.toml"),
    ("locked entries carry a version or a rev", "锁定的条目要带 version 或 rev"),
    ("zam.lock: invalid commit % for %", "zam.lock：{1} 的提交 {0} 不合法"),
    ("%: package entry without %", "{0}：包条目缺少 {1}"),
    ("the package index lists % %, but its zam.toml at that commit says version %", "包索引里写的是 {0} {1}，但那个提交里的 zam.toml 写的版本是 {2}"),
    ("report it to the package author", "请告诉包的作者"),
    ("package index entry %: %", "包索引条目 {0}：{1}"),
    ("unknown key %", "未知的键 {0}"),
    ("unknown version key %", "未知的版本键 {0}"),
    ("version % is listed twice", "版本 {0} 列了两次"),
    ("version % needs a 40-character commit", "版本 {0} 需要 40 个字符的提交号"),
    ("yanked must be true or false", "yanked 只能是 true 或 false"),
    ("missing repository", "缺少 repository"),
    ("name must be %", "name 必须是 {0}"),
    ("each [[versions]] must be a table", "每个 [[versions]] 都必须是表"),
    ("% (yanked)", "{0}（已撤回）"),
    ("corrupt cache entry %", "缓存条目 {0} 已损坏"),
    ("native source %: %", "native 源文件 {0}：{1}"),
    ("non-UTF-8 path", "路径不是 UTF-8"),
    ("a path dependency takes no rev", "路径依赖不需要 rev"),
    ("git dependencies need git on PATH: %", "git 依赖需要 PATH 里有 git：{0}"),
    ("git % failed: %", "git {0} 失败：{1}"),
    ("cannot find rev % in %", "在 {1} 里找不到 rev {0}"),
    ("cannot find the line for % under [dependencies]", "在 [dependencies] 下找不到 {0} 那一行"),
    ("remove it from zam.toml by hand", "请手动从 zam.toml 里删掉它"),
    ("package % has no versions to install yet", "包 {0} 还没有可以安装的版本"),
    ("package % is not in the package index %", "包索引 {1} 里没有包 {0}"),
    ("look for it with zam search, or add it from git with zam add % <url>", "用 zam search 搜索，或用 zam add {0} <url> 从 git 添加"),
    ("no version of % satisfies every requirement: %", "{0} 没有能满足所有要求的版本：{1}"),
    ("available versions: %", "可用的版本：{0}"),
    ("cannot read the package index %: %", "无法读取包索引 {0}：{1}"),
    ("invalid version %", "版本号 {0} 不合法"),
    ("write three numbers such as 1.2.0", "写成三个数字，例如 1.2.0"),
    ("invalid version requirement %", "版本要求 {0} 不合法"),
    ("% requires % %", "{0} 要求 {1} {2}"),
    ("zam.toml must be at the root of its git repository to publish it", "zam.toml 必须在 git 仓库的根目录才能发布"),
    ("tag the version first: git tag %", "先给这个版本打标签：git tag {0}"),
    ("push the tag first: git push origin %", "先推送标签：git push origin {0}"),

    // ---- 命令行 ----
    ("usage: zam new <directory> [--template ai]", "用法：zam new <目录> [--template ai]"),
    ("usage: only zam run accepts program arguments after --", "用法：只有 zam run 能在 -- 后面接程序参数"),
    ("usage: --json may be specified once", "用法：--json 只能写一次"),
    ("--json is only supported by check and build", "只有 check 和 build 支持 --json"),
    ("usage: zam <check|build|run|test|fmt|new|add|remove|update|search|publish> [project|file]", "用法：zam <check|build|run|test|fmt|new|add|remove|update|search|publish> [项目|文件]"),

    // ---- C 工具链 ----
    ("install MSVC C++ build tools for native builds", "构建原生程序需要安装 MSVC C++ 生成工具"),
    ("C compiler unavailable: %", "找不到 C 编译器：{0}"),
    ("MSVC % not found", "找不到 MSVC 的 {0}"),
    ("C compiler version query failed", "查询 C 编译器版本失败"),
    ("MSVC discovery failed: %", "查找 MSVC 失败：{0}"),
    ("MSVC setup failed: %", "MSVC 环境准备失败：{0}"),
    ("MSVC setup failed:\n%", "MSVC 环境准备失败：\n{0}"),
    ("MSVC setup produced no INCLUDE", "MSVC 环境准备后没有 INCLUDE 路径"),
    ("MSVC setup unavailable: %", "MSVC 环境不可用：{0}"),
    ("MSVC version query failed: %", "查询 MSVC 版本失败：{0}"),
    ("MSVC version query failed:\n%", "查询 MSVC 版本失败：\n{0}"),
    ("MSVC default version unavailable: %", "无法确定 MSVC 的默认版本：{0}"),
    ("invalid MSVC default version", "MSVC 的默认版本不合法"),
    ("missing ProgramFiles(x86)", "缺少环境变量 ProgramFiles(x86)"),
    ("unsupported MSVC setup path", "MSVC 的环境准备路径不受支持"),
    ("unsupported native cache path", "原生缓存路径不受支持"),

    // ---- main.rs 里补充的 Python 习惯提示 ----
    ("read a line with io.read_line()? and use std/io", "用 io.read_line()? 读一行，并导入 std/io"),
    ("an optional value is written Option.None, or use a default value", "可选值写成 Option.None，或者用一个默认值"),
    ("methods take the value as a normal parameter, such as fn area(p: &Point)", "方法把值当作普通参数接收，例如 fn area(p: &Point)"),
    ("use .push(v) to add to the end", "用 .push(v) 加到末尾"),

    // ---- 建议（help） ----
    ("index a list, array or map", "对列表、数组或字典取下标"),
    ("slice a string instead", "字符串请改用切片"),
    ("remove or replace the unsupported character", "删掉或替换这个不支持的字符"),
    ("use a supported escape such as `\\n`, `\\r`, `\\t`, `\\\"`, or `\\\\`", "使用支持的转义，例如 `\\n`、`\\r`、`\\t`、`\\\"` 或 `\\\\`"),
    ("borrow the value or move it only after the last use", "借用这个值，或在最后一次使用之后再移走"),
    ("keep references in call arguments", "借用只放在调用参数里"),
    ("store or return an owned value", "存放或返回有所有权的值"),
    ("match the borrow type to the parameter declaration", "让借用类型与参数声明一致"),
    ("declare the owner with `let mut` before taking `&mut`", "先用 `let mut` 声明，再取 `&mut`"),
    ("pass `&value` for a shared borrow or `&mut value` for an exclusive borrow", "共享借用传 `&value`，独占借用传 `&mut value`"),
    ("avoid overlapping moves or exclusive borrows in the same call", "同一次调用里不要同时移走和独占借用同一个值"),
    ("return a string on every path or change the function result type", "每条路径都返回字符串，或修改函数的返回类型"),
    ("add an arm for each missing variant or a final `_:` arm, and remove arms after `_`", "给每个缺少的变体加一个分支，或在最后加 `_:` 分支，并删掉 `_` 之后的分支"),
    ("handle the call with `?` or a `catch` block", "用 `?` 或 `catch` 块处理这个调用"),
    ("use a string, a struct or an enum as the error type, such as `T!string` or `T!ParseError`", "错误类型用字符串、结构体或枚举，例如 `T!string` 或 `T!ParseError`"),
    ("use an f64 literal no larger than 1.7976931348623157e308", "f64 字面量不能大于 1.7976931348623157e308"),
    ("use a literal that fits in `i64` (max 9223372036854775807)", "使用 `i64` 范围内的字面量（最大 9223372036854775807）"),
    ("check the input range before arithmetic or division", "运算或除法之前先检查输入的范围"),
    ("ensure the index is within the declared array length", "确保下标不超过数组声明的长度"),
    ("fixed arrays are written [T; N] with a length from 1 to 1024 and declared as local values", "定长数组写成 [T; N]，长度 1–1024，并且只能声明为局部值"),
    ("fixed arrays are written [T; N] with a length from 1 to 1024 and declared as local values, and [T] is a growable list", "定长数组写成 [T; N]、长度 1–1024 且只能声明为局部值；需要可增长的列表请用 [T]"),
    ("use [T] for a growable list", "需要可增长的列表请用 [T]"),
    ("declare tests as fn test_name(): or fn test_name() -> !string: with no parameters", "测试写成 fn test_name(): 或 fn test_name() -> !string:，不带参数"),
    ("match the function type: the same parameter count and types, and no !E", "与函数类型保持一致：参数个数和类型相同，并且没有 !E"),
    ("rename the parameter", "给参数改个名字"),
    ("Zamak does not allow shadowing", "Zamak 不允许遮蔽同名变量"),
    ("keep fn(...): on one line, or declare a named fn and pass its name", "把 fn(...): 写在一行里，或者声明一个具名 fn 再传它的名字"),
    ("write the type arguments in an annotation, such as let x: Option[i64] = Option.None", "在标注里写出类型实参，例如 let x: Option[i64] = Option.None"),
    ("match the expression type to the declared type", "让表达式的类型与声明的类型一致"),
    ("check the field name against the struct declaration", "对照结构体的声明检查字段名"),
    ("check the name, declaration, and module import", "检查名字、声明和模块导入"),
    ("check the matching `use` import", "检查对应的 `use` 导入"),
    ("add it if missing or disambiguate modules with the same alias", "缺了就补上，同一别名指向多个模块时请区分开"),
    ("remove or rename the duplicate declaration", "删掉或改名重复的声明"),
    ("declare the fn at the top level of the module", "把 fn 声明在模块顶层"),
    ("a fn inside a function cannot be generic, fail, call itself or be reassigned yet", "函数里的 fn 暂时不能是泛型、不能失败、不能调用自己、不能重新赋值"),
    ("remove the ? or catch", "去掉 ? 或 catch"),
    ("only calls whose function type has !E can fail", "只有函数类型带 !E 的调用才会失败"),
    ("end every path of the fn body with a value or a return statement", "fn 函数体的每条路径都以一个值或 return 语句结尾"),
    ("declare the binding with `let mut` before assigning it", "赋值前先用 `let mut` 声明"),
    ("extern fn takes i64, f64, bool or &string and returns nothing, i64, f64, bool or string", "extern fn 只接受 i64、f64、bool 或 &string，只能不返回或返回 i64、f64、bool、string"),
    ("convert other data in a small C function listed in [native] sources", "其他数据请在 [native] sources 里写个小 C 函数转换"),
    ("write extern fn name(params) -> T on one line, with no body, type parameters or error type", "extern fn name(params) -> T 写在一行里，不要函数体、类型参数或错误类型"),
    ("use a construct listed as supported in `zam --help` or the tutorial", "改用 `zam --help` 或教程里列出的写法"),
    ("end the line with `:` instead of `{`, indent the block by 4 spaces, and delete the closing `}`", "行尾用 `:` 代替 `{`，代码块缩进 4 个空格，删掉结尾的 `}`"),
    ("run `zam fmt` to replace the tabs with spaces", "运行 `zam fmt` 把制表符换成空格"),
    ("restore the indentation: indent every block 4 spaces under the line that ends with `:`", "恢复缩进：每个代码块比以 `:` 结尾的那一行多缩进 4 个空格"),
    ("indent the lines of a block 4 spaces deeper than the line ending with `:`, and line up lines of the same block", "代码块比以 `:` 结尾的那一行多缩进 4 个空格，同一代码块的各行对齐"),
    ("write `elif condition:` instead of `else if condition:`", "写 `elif condition:`，不是 `else if condition:`"),
    ("logical operators are words: write `a and b`, `a or b`, `not a`", "逻辑运算符用单词：`a and b`、`a or b`、`not a`"),
    ("add the matching closing bracket", "补上配对的右括号"),
    ("compare with `==`, for example `score == 60`", "比较用 `==`，例如 `score == 60`"),
    ("a single `=` only assigns", "单个 `=` 只用来赋值"),
    ("repair the reported token or delimiter, then run `zam check` again", "修好标出的记号或括号，再运行一次 `zam check`"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_follow_the_pattern() {
        assert_eq!(
            capture("unknown variable %", "unknown variable x"),
            Some(vec!["x"])
        );
        assert_eq!(capture("a % b %", "a 1 b 2"), Some(vec!["1", "2"]));
        assert_eq!(
            capture("missing main function", "missing main function"),
            Some(vec![])
        );
        assert_eq!(
            capture("missing main function", "missing main function!"),
            None
        );
        assert_eq!(capture("unknown variable %", "unknown variable "), None);
    }

    #[test]
    fn messages_and_their_parts_translate() {
        assert_eq!(translate("use after move: name"), "使用已移走的值：name");
        // 最具体的句式优先。
        assert_eq!(
            translate("use after move: s (the fn show uses it)"),
            "使用已移走的值：s（fn show 用到了它）"
        );
        // 占位里的句子也翻译。
        assert_eq!(
            translate("expected ':' at the end of the if line, found the end of the file"),
            "if 这一行末尾应该是 ':'，却是文件结尾"
        );
        // 整句没有，就按 “; ” 拆开。
        assert_eq!(
            translate("type mismatch: x is f64 but i64 is expected; convert with i64(x)"),
            "类型不匹配：x 是 f64，这里需要 i64；用 i64(x) 转换"
        );
        // 不认识的部分保留英文。
        assert_eq!(
            translate("something new; use .push(v) to add to the end"),
            "something new；用 .push(v) 加到末尾"
        );
        // 模块前缀保留。
        assert_eq!(
            translate("geo: unknown function area"),
            "geo：未知的函数 area"
        );
        // 类型写法里方括号中的分号不是句子分隔符，不能变成全角分号。
        assert_eq!(
            translate(
                "fixed arrays are written [T; N] with a length from 1 to 1024 and declared as local values, and [T] is a growable list"
            ),
            "定长数组写成 [T; N]、长度 1–1024 且只能声明为局部值；需要可增长的列表请用 [T]"
        );
        assert_eq!(
            translate("only [i64; N] arrays are supported"),
            "只支持 [i64; N] 数组"
        );
    }

    #[test]
    fn table_has_no_duplicate_patterns() {
        let mut seen = std::collections::HashSet::new();
        let duplicates: Vec<_> = TABLE
            .iter()
            .filter(|(en, _)| !seen.insert(*en))
            .map(|(en, _)| *en)
            .collect();
        assert!(duplicates.is_empty(), "{duplicates:?}");
    }

    #[test]
    fn placeholders_line_up() {
        for (en, zh) in TABLE {
            let count = en.matches('%').count();
            for index in 0..count {
                assert!(
                    zh.contains(&format!("{{{index}}}")),
                    "{en} → {zh}: missing {{{index}}}"
                );
            }
            assert!(
                !zh.contains(&format!("{{{count}}}")),
                "{en} → {zh}: extra {{{count}}}"
            );
        }
    }

    /// 只在本地跑的覆盖率体检：把编译器里所有报错句子抽出来，逐句过一遍 `translate`，
    /// 列出还没翻译的。句式表靠手工维护，漏掉一条中文用户就会看到英文句子。
    ///
    /// 抽法：编译器与项目层的报错都是 `format!` 出来的英文句子，参数是名字/类型/数量，
    /// 句式表里用 `%` 占位。这里把 `{名字}`、`{:?}` 之类的插值换成 `x`，再按句子比较；
    /// 表里已有的句式用 `%` 通配匹配，所以只要句子骨架在表里就算覆盖。
    ///
    /// 内部断言用的句子（`unreachable`、`expected T`、`debug_assert` 之类）不算用户可见的
    /// 报错，列在 `INTERNAL` 里跳过；确实只在内部用的句子也在这里说明理由。
    #[test]
    fn every_error_sentence_is_translated() {
        let mut missing = Vec::new();
        for source in [
            include_str!("compiler.rs"),
            include_str!("project.rs"),
            include_str!("package.rs"),
            include_str!("registry.rs"),
            include_str!("native.rs"),
            include_str!("main.rs"),
        ] {
            for sentence in error_sentences(source) {
                if INTERNAL.iter().any(|(text, _)| *text == sentence) {
                    continue;
                }
                if !is_translated(&sentence) {
                    missing.push(sentence);
                }
            }
        }
        missing.sort_unstable();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "{} 条报错句子还没有中文（见 zh.rs 的 TABLE）：\n{}",
            missing.len(),
            missing.join("\n")
        );
    }

    /// 一句英文报错的骨架是否已由 TABLE 覆盖（不要求整句对得上，按句子里每段分别看）。
    fn is_translated(sentence: &str) -> bool {
        // 先整句匹配；`[i64; N]` 这类类型名里的分号不是句子分隔符，整句对不上时才拆。
        whole(sentence).is_some()
            || sentence.split("; ").all(|part| {
                whole(part).is_some()
                    // `translate` 还会剥掉 `模块: ` 前缀后翻译后半句。
                    || part.split_once(": ").is_some_and(|(head, tail)| {
                        !head.contains(' ') && whole(tail).is_some()
                    })
                    // 只有名字/类型/数字的片段（`x`、`%d`、`i64`）没什么可翻的。
                    || part.split_whitespace().count() <= 1
            })
    }

    /// 从 Rust 源码里抽出用户能看到的报错句子：只取返回错误的位置（`Err(...)`、`ok_or*`、
    /// `map_err`、`fail`）上的字符串字面量。C 运行时字符串、帮助文字、进度输出、测试代码都
    /// 不在范围里。
    fn error_sentences(source: &str) -> Vec<String> {
        const SITES: &[&str] = &["Err(", "ok_or", "map_err", "fail ", "push(format!"];
        let mut found = Vec::new();
        let mut in_tests = false;
        let lines: Vec<&str> = source.lines().collect();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            let trimmed = line.trim_start();
            // `#[cfg(test)] mod tests` 之后的都是测试代码，里面的字符串不是用户能看到的。
            if trimmed.starts_with("mod tests") || trimmed.contains("mod tests {") {
                in_tests = true;
            }
            index += 1;
            if in_tests || trimmed.starts_with("//") {
                continue;
            }
            // `format!(` 跨行时，把后面独占一行的字面量拼回同一句。
            let mut text = line.to_owned();
            let site = SITES.iter().any(|site| line.contains(site));
            if site {
                let mut next = index;
                while next < lines.len() {
                    let candidate = lines[next].trim();
                    let literal = candidate.strip_suffix(')').unwrap_or(candidate);
                    if let Some(inner) = literal
                        .strip_prefix('"')
                        .and_then(|rest| rest.strip_suffix('"'))
                    {
                        text.push_str(inner);
                        index = next + 1;
                        next += 1;
                        if candidate.ends_with(')') {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            } else {
                continue;
            }
            for literal in string_literals(&text) {
                let skeleton = placeholders(&literal);
                if !looks_like_error_sentence(&skeleton) {
                    continue;
                }
                found.push(skeleton);
            }
        }
        found
    }

    /// 像一句给用户看的英文报错：以字母开头、至少两个词，并且不是 C 代码片段或注释行。
    fn looks_like_error_sentence(skeleton: &str) -> bool {
        let words: Vec<&str> = skeleton.split_whitespace().collect();
        if words.len() < 2 {
            return false;
        }
        if !words[0]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            return false;
        }
        // C 片段（`x *cx;`、`x cx;`）、声明片段（`byte_len returns i64` 这类没有空格的类型名
        // 其实是真实报错，保留）与行首注释都不算。
        const NOT_A_SENTENCE: &[&str] = &[
            "x *cx;",
            "x cx;",
            "x: x",
            "line x, column x: x: x",
            "or true",
            "match arm",
            "nested x",
            "unterminated x",
            "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
        ];
        !NOT_A_SENTENCE.contains(&skeleton)
    }

    /// 一行里的所有 `"..."` 字面量（`\n` 换成空格，`\"` 还原成引号）。
    fn string_literals(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = line;
        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            // 找结尾的引号，跳过被转义的 `\"`。
            let mut end = None;
            let mut escaped = false;
            for (index, c) in after.char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                match c {
                    '\\' => escaped = true,
                    '"' => {
                        end = Some(index);
                        break;
                    }
                    _ => {}
                }
            }
            let Some(end) = end else { break };
            out.push(after[..end].replace("\\n", " ").replace("\\\"", "\""));
            rest = &after[end + 1..];
        }
        out
    }

    /// `line {line}, column {column}: foo {a:?}` → `line x, column x: foo x`。
    /// 源码里写的是转义后的 `{{}}`，展开成 `{}` 后保留不动——它是报错里原样显示的花括号
    /// （例如 “empty {} in string literal”），不是插值。
    fn placeholders(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '{' {
                if chars.peek() == Some(&'{') {
                    // 源码里的 `{{` 是转义，显示出来就是一个 `{`；紧跟的 `}}` 同理。
                    chars.next();
                    if chars.peek() == Some(&'}') {
                        chars.next();
                    }
                    out.push('{');
                    continue;
                }
                let mut inner = String::new();
                let mut closed = false;
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    inner.push(next);
                }
                // `{名字}`、`{:?}` 是 Rust 插值，换成 `x`；`{}` 是报错里原样显示的花括号
                // （“empty {} in string literal”），保留成 `{}`。
                if closed && !inner.is_empty() {
                    out.push('x');
                } else if closed {
                    out.push_str("{}");
                } else {
                    out.push('{');
                    out.push_str(&inner);
                }
            } else if c == '}' && chars.peek() == Some(&'}') {
                chars.next();
                out.push('}');
            } else {
                out.push(c);
            }
        }
        out
    }

    /// 内部断言或调试用、不会显示给用户的句子。
    static INTERNAL: &[(&str, &str)] = &[
        ("not reached", "match 分支的内部兜底"),
        ("expected an array or list", "生成器的内部断言"),
        (
            "invalid base",
            "折叠常量时的内部断言（用户看到的是带部分的版本）",
        ),
    ];
}
