use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Text(String),
    Number(String),
    // 已去掉分隔符、确认有限的浮点字面量文本，如 `1.5`、`2e-3`。
    Float(String),
    Symbol(char),
    Newline,
    // 行首缩进变深 / 变浅一层。
    Indent,
    Dedent,
}

/// 报错里的记号写成源码里的样子。
fn shown(token: Option<&Token>) -> String {
    match token {
        Some(Token::Word(word)) => format!("`{word}`"),
        Some(Token::Text(text)) => format!("the string \"{text}\""),
        Some(Token::Number(number) | Token::Float(number)) => format!("the number {number}"),
        Some(Token::Symbol(c)) => format!("{c:?}"),
        Some(Token::Newline) => "the end of the line".to_owned(),
        Some(Token::Indent) => "an indented line".to_owned(),
        Some(Token::Dedent) => "the end of the block".to_owned(),
        None => "the end of the file".to_owned(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Owned,
    Shared,
    Mutable,
    Unit,
    Int,
    Float,
    Bool,
    Record(u64),
    // 结构体的调用帧借用 `&T` / `&mut T`。
    RecordRef(u64, bool),
    ArrayInt(usize),
    // 可增长数组 `[T]`（owned）与它的调用帧借用 `&[T]` / `&mut [T]`。
    List(Elem),
    ListRef(Elem, bool),
    // 字典 `{K: V}`（owned）与它的调用帧借用；键只能是 i64 或 string。
    Map(Elem, Elem),
    MapRef(Elem, Elem, bool),
    // 函数值 `fn(i64) -> bool`：编号指向签名表（`func_sig`）。`Func` 不捕获局部变量，可以存进
    // 变量、字段、返回；参数位置写的函数类型是 `FuncRef`，它可能是捕获了调用方局部变量的闭包，
    // 和借用一样只能调用或继续当实参传，不能存下来。
    Func(u64),
    FuncRef(u64),
}
impl Type {
    /// `[T]`、`{K: V}` 或它们的借用 → 背后的 owned 容器类型。
    pub fn container(self) -> Option<Type> {
        match self {
            Type::List(elem) | Type::ListRef(elem, _) => Some(Type::List(elem)),
            Type::Map(key, value) | Type::MapRef(key, value, _) => Some(Type::Map(key, value)),
            _ => None,
        }
    }
    pub fn is_reference(self) -> bool {
        matches!(
            self,
            Type::Shared
                | Type::Mutable
                | Type::ListRef(_, _)
                | Type::MapRef(_, _, _)
                | Type::RecordRef(_, _)
                | Type::FuncRef(_)
        )
    }
    /// 可被借用的值：（被借用的 owned 类型，本身已是借用时是否可写）。标量不可借用。
    pub fn borrowable(self) -> Option<(Type, Option<bool>)> {
        match self {
            Type::Owned => Some((Type::Owned, None)),
            Type::Shared => Some((Type::Owned, Some(false))),
            Type::Mutable => Some((Type::Owned, Some(true))),
            Type::Record(id) => Some((Type::Record(id), None)),
            Type::RecordRef(id, mutable) => Some((Type::Record(id), Some(mutable))),
            Type::List(elem) => Some((Type::List(elem), None)),
            Type::ListRef(elem, mutable) => Some((Type::List(elem), Some(mutable))),
            Type::Map(key, value) => Some((Type::Map(key, value), None)),
            Type::MapRef(key, value, mutable) => Some((Type::Map(key, value), Some(mutable))),
            _ => None,
        }
    }
}

/// 借用一个 owned 值得到的借用类型；标量没有借用。
pub fn borrow_of(ty: Type, mutable: bool) -> Option<Type> {
    Some(match ty {
        Type::Owned if mutable => Type::Mutable,
        Type::Owned => Type::Shared,
        Type::Record(id) => Type::RecordRef(id, mutable),
        Type::List(elem) => Type::ListRef(elem, mutable),
        Type::Map(key, value) => Type::MapRef(key, value, mutable),
        _ => return None,
    })
}

/// 全局 `abs`/`min`/`max` 与 Python 一样对整数和浮点数都能用：按第一个参数的类型选内建版本。
/// 检查器与代码生成共用，IR 里只记 `std/core:abs` 这个名字。
pub fn overload(name: &str, first: Option<Type>) -> &str {
    match (name, first) {
        ("std/core:abs", Some(Type::Float)) => "std/core:abs_f64",
        ("std/core:min", Some(Type::Float)) => "std/core:min_f64",
        ("std/core:max", Some(Type::Float)) => "std/core:max_f64",
        ("std/core:pow", Some(Type::Float)) => "std/core:pow_f64",
        ("std/core:int", Some(Type::Float)) => "std/core:i64",
        ("std/core:int", Some(ty)) if is_text(ty) => "std/string:parse_i64",
        ("std/core:float", Some(Type::Int)) => "std/core:f64",
        ("std/core:float", Some(ty)) if is_text(ty) => "std/string:parse_f64",
        _ => name,
    }
}
/// 算术与比较按 Python 的规则：整数与小数混用时整数先变成小数，`/` 永远得到小数
/// （`7 / 2` 是 3.5，整除写 `7 // 2`）。转换是内部的 `Unary('f', x)`。检查器与生成器共用。
pub fn promote(expr: &Expr, left: Type, right: Type) -> Option<Expr> {
    let Expr::Binary(op, a, b) = expr else {
        return None;
    };
    if !matches!(
        op.as_str(),
        "+" | "-" | "*" | "/" | "//" | "%" | "<" | "<=" | ">" | ">=" | "==" | "!="
    ) {
        return None;
    }
    let float = |side: &Expr| match side {
        Expr::Int(n) => Expr::Float(*n as f64),
        side => Expr::Unary('f', Box::new(side.clone())),
    };
    let (a, b) = match (left, right) {
        (Type::Int, Type::Float) => (float(a), (**b).clone()),
        (Type::Float, Type::Int) => ((**a).clone(), float(b)),
        (Type::Int, Type::Int) if op == "/" => (float(a), float(b)),
        _ => return None,
    };
    Some(Expr::Binary(op.clone(), Box::new(a), Box::new(b)))
}
/// `s[i]` 改写成 `string.char_at(&s, i)`。检查器与生成器共用。
pub fn char_at(path: &str, index: &Expr) -> Expr {
    Expr::Call(
        "std/string:char_at".into(),
        vec![Expr::Borrow(path.to_owned(), false), index.clone()],
    )
}
/// 只读借用参数自动借用：`show(name)`、`show(p.name)`、`show(xs[i])` 等于写了 `&`。
/// 只改写变量、字段与元素；`&mut` 仍要显式写。检查器与生成器共用。
pub fn auto_borrow(expr: &Expr) -> Option<Expr> {
    Some(match expr {
        Expr::Variable(name) => Expr::Borrow(name.clone(), false),
        Expr::Field(local, field) => Expr::Borrow(format!("{local}.{field}"), false),
        Expr::Index(path, index) => {
            Expr::BorrowIndex(path.clone(), index.clone(), String::new(), false)
        }
        Expr::IndexField(path, index, field) => {
            Expr::BorrowIndex(path.clone(), index.clone(), field.clone(), false)
        }
        _ => return None,
    })
}
/// `x in xs`、`x not in xs`：改写成 `xs.contains(x)`（字符串、列表、字典都有 contains）。
/// `item_text` 为真时元素是字符串，变量、字段与元素按只读借用传。检查器与生成器共用。
pub fn membership(expr: &Expr, item_text: bool) -> Option<Expr> {
    let Expr::Binary(op, item, container) = expr else {
        return None;
    };
    let negate = match op.as_str() {
        "in" => false,
        "not in" => true,
        _ => return None,
    };
    let argument = match &**item {
        Expr::Variable(name) if item_text => Expr::Borrow(name.clone(), false),
        Expr::Field(local, field) if item_text => Expr::Borrow(format!("{local}.{field}"), false),
        Expr::Index(path, index) if item_text => {
            Expr::BorrowIndex(path.clone(), index.clone(), String::new(), false)
        }
        Expr::IndexField(path, index, field) if item_text => {
            Expr::BorrowIndex(path.clone(), index.clone(), field.clone(), false)
        }
        item => item.clone(),
    };
    // `x in [1, 2, 3]`：字面量列表不必建出来，写成 `x == 1 or x == 2 or x == 3`。
    // x 要求没有副作用（变量、字段、字面量），重复求值才与求一次相同。
    if let Expr::Array(values) = &**container {
        let pure = matches!(
            &**item,
            Expr::Variable(_)
                | Expr::Field(..)
                | Expr::Int(_)
                | Expr::Float(_)
                | Expr::Bool(_)
                | Expr::Text(_)
        );
        if pure {
            let test = values
                .iter()
                .map(|value| Expr::Binary("==".into(), item.clone(), Box::new(value.clone())))
                .reduce(|a, b| Expr::Binary("||".into(), Box::new(a), Box::new(b)))
                .unwrap_or(Expr::Bool(false));
            return Some(if negate {
                Expr::Unary('!', Box::new(test))
            } else {
                test
            });
        }
    }
    let call = match &**container {
        Expr::Variable(name) => Expr::Method(name.clone(), "contains".into(), vec![argument]),
        Expr::Field(local, field) => Expr::Method(
            format!("{local}.{field}"),
            "contains".into(),
            vec![argument],
        ),
        container => Expr::Chain(
            Box::new(container.clone()),
            "contains".into(),
            vec![argument],
        ),
    };
    Some(if negate {
        Expr::Unary('!', Box::new(call))
    } else {
        call
    })
}
/// 方法调用的接收者：路径（`p`、`a.b`）、列表或字典的元素（`xs[i]`、`xs[i].f`）、调用结果。
pub enum Receiver<'a> {
    Path(&'a str, Type),
    Element(&'a str, &'a Expr, &'a str),
    Temp(&'a Expr),
}

/// 结构体或枚举里声明的方法：`Point` 里的 `fn norm(&self)` 在链接后的函数表里叫 `模块:Point.norm`。
/// 接收者不是结构体时返回 Ok(None)；是结构体但没有这个方法时报错。
pub fn record_method<'f>(
    structs: &BTreeMap<String, Struct>,
    functions: &'f BTreeMap<String, Function>,
    receiver: Type,
    method: &str,
) -> Result<Option<(String, &'f Function)>, String> {
    let (Type::Record(id) | Type::RecordRef(id, _)) = receiver else {
        return Ok(None);
    };
    let Some(owner) = structs.keys().find(|name| record_id(name) == id) else {
        return Ok(None);
    };
    // 泛型类型的实例（`Stack[Int]`）：方法声明在模板 `Stack` 里，是泛型函数，按接收者的
    // 类型实参生成副本。
    let template = instance_of(id).map(|(template, _, _)| template);
    let owner = template.as_ref().unwrap_or(owner);
    let name = format!("{owner}.{method}");
    let short = owner.rsplit(':').next().unwrap_or(owner);
    if template.is_some() && functions.contains_key(&name) {
        let name = instance(functions, &name, &[Some(Type::Record(id))])?;
        return functions
            .get(&name)
            .map(|function| Some((name.clone(), function)))
            .ok_or_else(|| format!("unknown method {short}.{method}"));
    }
    match functions.get(&name) {
        Some(function) => Ok(Some((name, function))),
        None => Err(format!(
            "unknown method {short}.{method}; declare it inside {short} as fn {method}(&self)"
        )),
    }
}

/// `p.m(a)` 改写成普通调用 `Owner.m(接收者, a)` 的实参：`&self`、`&mut self` 借用接收者，
/// `self` 把它移进去。检查器与生成器共用。
pub fn method_args(
    receiver: Receiver,
    callee: &Function,
    method: &str,
    args: &[Expr],
) -> Result<Vec<Expr>, String> {
    let Some(self_ty) = callee
        .params
        .first()
        .filter(|(name, _)| name == "self")
        .map(|(_, ty)| *ty)
    else {
        return Err(format!(
            "{method} has no self parameter; call it on the type, such as Point.{method}(...)"
        ));
    };
    let first = match (receiver, self_ty) {
        (Receiver::Path(path, place), Type::RecordRef(_, mutable)) => {
            if place.is_reference() && !path.contains('.') {
                Expr::Variable(path.into())
            } else {
                Expr::Borrow(path.into(), mutable)
            }
        }
        (Receiver::Path(path, _), _) => match path.split_once('.') {
            Some((root, rest)) => Expr::Field(root.into(), rest.into()),
            None => Expr::Variable(path.into()),
        },
        (Receiver::Element(path, index, field), Type::RecordRef(_, mutable)) => {
            Expr::BorrowIndex(path.into(), Box::new(index.clone()), field.into(), mutable)
        }
        (Receiver::Element(path, _, _), _) => {
            return Err(format!(
                "cannot move a struct out of {path}[..]: {method} takes self by value; take it out with pop() or copy(&...) first"
            ))
        }
        (Receiver::Temp(expr), _) => expr.clone(),
    };
    Ok(std::iter::once(first).chain(args.iter().cloned()).collect())
}

/// 全局 `copy(&x)`：深复制字符串、列表、字典、结构体与枚举。它对任何类型都成立，
/// 所以不在函数表里，检查器与生成器按名字特判。
pub const COPY: &str = "std/core:copy";

/// 按值复制、不需要释放的值：数字、布尔与函数值。
pub fn copied(ty: Type) -> bool {
    matches!(
        ty,
        Type::Int | Type::Float | Type::Bool | Type::Func(_) | Type::FuncRef(_)
    )
}

/// 闭包看见的外层局部变量：owned 的值变成借用，数字、布尔与函数照旧（读写的都是外层变量本身）。
/// `mutable` 是外层变量能不能写（`let mut`，或本身是 `&mut` 借用）：能写时视图是可变借用。
pub fn captured_view(ty: Type, mutable: bool) -> Result<Type, String> {
    Ok(match ty {
        Type::Owned => borrow_of(ty, mutable).expect("strings can be borrowed"),
        Type::Shared | Type::Mutable => ty,
        Type::Record(id) => Type::RecordRef(id, mutable),
        Type::List(elem) => Type::ListRef(elem, mutable),
        Type::Map(key, value) => Type::MapRef(key, value, mutable),
        Type::RecordRef(..) | Type::ListRef(..) | Type::MapRef(..) => ty,
        Type::ArrayInt(_) => {
            return Err("fixed arrays cannot be used inside an anonymous fn; use a list".into())
        }
        other => other,
    })
}

/// 表达式里用到的局部名（路径的根）。闭包据此找出它捕获了哪些外层变量。
pub fn expr_roots(expr: &Expr, out: &mut BTreeSet<String>) {
    let root = |path: &str| path.split('.').next().unwrap_or(path).to_owned();
    match expr {
        Expr::Variable(name) | Expr::Borrow(name, _) => {
            out.insert(root(name));
        }
        Expr::Field(base, _) => {
            out.insert(root(base));
        }
        Expr::Index(path, index) | Expr::IndexField(path, index, _) => {
            out.insert(root(path));
            expr_roots(index, out);
        }
        Expr::BorrowIndex(path, index, _, _) => {
            out.insert(root(path));
            expr_roots(index, out);
        }
        Expr::Method(path, _, args) | Expr::Apply(path, args) => {
            out.insert(root(path));
            args.iter().for_each(|arg| expr_roots(arg, out));
        }
        Expr::Invoke(callee, args) => {
            expr_roots(callee, out);
            args.iter().for_each(|arg| expr_roots(arg, out));
        }
        Expr::ElementMethod(path, index, _, _, args) => {
            out.insert(root(path));
            expr_roots(index, out);
            args.iter().for_each(|arg| expr_roots(arg, out));
        }
        Expr::Chain(inner, _, args) => {
            expr_roots(inner, out);
            args.iter().for_each(|arg| expr_roots(arg, out));
        }
        Expr::Call(_, args)
        | Expr::Variant(_, _, args)
        | Expr::Array(args)
        | Expr::Tuple(args)
        | Expr::Format(args) => args.iter().for_each(|arg| expr_roots(arg, out)),
        Expr::Record(_, fields) => fields.iter().for_each(|(_, v)| expr_roots(v, out)),
        Expr::MapLit(entries) => entries.iter().for_each(|(k, v)| {
            expr_roots(k, out);
            expr_roots(v, out);
        }),
        Expr::Unary(_, inner) | Expr::Try(inner) | Expr::ByteLen(inner) | Expr::CharLen(inner) => {
            expr_roots(inner, out)
        }
        Expr::Catch(inner, _, body) => {
            expr_roots(inner, out);
            block_roots(body, out);
        }
        Expr::Binary(_, a, b) | Expr::StringEqual(a, b) => {
            expr_roots(a, out);
            expr_roots(b, out);
        }
        Expr::If(c, a, b) => {
            expr_roots(c, out);
            expr_roots(a, out);
            expr_roots(b, out);
        }
        Expr::Match(scrutinee, arms) => {
            expr_roots(scrutinee, out);
            arms.iter().for_each(|(_, v)| expr_roots(v, out));
        }
        Expr::Lambda(_, _, body) => block_roots(body, out),
        Expr::Text(_) | Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::FnRef(_) => {}
    }
}

/// `for x in [a, b]:` 生成的隐藏变量：值是非空的列表字面量时返回第一项（按它定元素类型）。
pub fn loop_list<'e>(local: &str, expr: &'e Expr) -> Option<&'e Expr> {
    match expr {
        Expr::Array(items) if local.starts_with("$for") => items.first(),
        _ => None,
    }
}

/// 字符串值上的方法：`s.upper()` 就是 `string.upper(&s)`——std/string 里同名、第一个参数是
/// `&string` 的函数；`s.len()` 是字节数，`s.char_len()` 按字符计。`receiver` 是借用接收者的
/// 表达式（变量写成 `&s`，调用结果原样作临时值）。不是字符串方法时返回 None。检查器与生成器共用。
/// Python 的字符串方法名对应的 std/string 函数：`s.strip()` 是 trim，`s.startswith(p)` 是
/// starts_with，`ljust`/`rjust` 补空格。检查器、生成器与“用到哪些函数”的收集共用。
pub fn string_alias(method: &str) -> &str {
    match method {
        "strip" => "trim",
        "startswith" => "starts_with",
        "endswith" => "ends_with",
        "ljust" => "pad_right",
        "rjust" => "pad_left",
        method => method,
    }
}
pub fn string_method(
    functions: &BTreeMap<String, Function>,
    receiver: Expr,
    method: &str,
    args: &[Expr],
) -> Option<Expr> {
    match method {
        "byte_len" if args.is_empty() => return Some(Expr::ByteLen(Box::new(receiver))),
        "len" if args.is_empty() => return Some(Expr::CharLen(Box::new(receiver))),
        "char_len" if args.is_empty() => return Some(Expr::CharLen(Box::new(receiver))),
        "join" if args.len() == 1 => {
            return Some(Expr::Call(
                "std/string:join".into(),
                vec![args[0].clone(), receiver],
            ))
        }
        _ => {}
    }
    let name = format!("std/string:{}", string_alias(method));
    let function = functions.get(&name)?;
    if function.params.first().map(|(_, ty)| *ty) != Some(Type::Shared) {
        return None;
    }
    Some(Expr::Call(
        name,
        std::iter::once(receiver)
            .chain(args.iter().cloned())
            .collect(),
    ))
}

/// 字符串类型（owned 或借用）。
pub fn is_string(ty: Type) -> bool {
    matches!(ty, Type::Owned | Type::Shared | Type::Mutable)
}

/// 能接住临时值的只读借用类型：返回被借用的 owned 类型。
pub fn temporary_borrow(expected: Type) -> Option<Type> {
    match expected {
        Type::Shared
        | Type::ListRef(_, false)
        | Type::MapRef(_, _, false)
        | Type::RecordRef(_, false) => expected.borrowable().map(|(owned, _)| owned),
        _ => None,
    }
}

/// 求值得到新值的表达式（不是变量、字段、元素这些已有的位置）。
pub fn produces_value(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Call(..)
            | Expr::Try(..)
            | Expr::Chain(..)
            | Expr::Method(..)
            | Expr::ElementMethod(..)
            | Expr::Apply(..)
            | Expr::Invoke(..)
            | Expr::Binary(..)
            | Expr::If(..)
            | Expr::Match(..)
            | Expr::Record(..)
            | Expr::Variant(..)
            | Expr::Tuple(..)
            | Expr::MapLit(..)
    )
}

/// 匿名函数体里的那个表达式（打印语句打印的值，或函数体本身）；函数内部用 `fn name(...):` 定义的
/// 函数体是完整的代码块（尾部的值已经改写成 return），这时是 None。
pub fn lambda_body(body: &Block) -> Option<&Expr> {
    match body.as_slice() {
        [(Instruction::Print(expr) | Instruction::Call(expr), _)] => Some(expr),
        _ => None,
    }
}

/// 代码块里用到的局部名（路径的根），包括赋值、遍历的目标。
pub fn block_roots(block: &Block, out: &mut BTreeSet<String>) {
    let root = |path: &str| path.split('.').next().unwrap_or(path).to_owned();
    for (instruction, _) in block {
        match instruction {
            Instruction::Let(_, _, _, value)
            | Instruction::Print(value)
            | Instruction::Call(value)
            | Instruction::Fail(value)
            | Instruction::Unpack(_, _, value)
            | Instruction::Return(Some(value)) => expr_roots(value, out),
            Instruction::Return(None) | Instruction::Break | Instruction::Continue => {}
            Instruction::Assign(name, value) | Instruction::AssignField(name, _, value) => {
                out.insert(root(name));
                expr_roots(value, out);
            }
            Instruction::AssignIndex(name, index, value)
            | Instruction::AssignIndexField(name, index, _, value) => {
                out.insert(root(name));
                expr_roots(index, out);
                expr_roots(value, out);
            }
            Instruction::If(condition, yes, no) => {
                expr_roots(condition, out);
                block_roots(yes, out);
                block_roots(no, out);
            }
            Instruction::While(condition, body) => {
                expr_roots(condition, out);
                block_roots(body, out);
            }
            Instruction::For(_, start, end, body) => {
                expr_roots(start, out);
                expr_roots(end, out);
                block_roots(body, out);
            }
            Instruction::Each(_, _, path, _, body) => {
                out.insert(root(path));
                block_roots(body, out);
            }
            Instruction::Match(scrutinee, arms) => {
                expr_roots(scrutinee, out);
                for (_, body) in arms {
                    block_roots(body, out);
                }
            }
        }
    }
}

/// 匿名函数体是打印语句：没有返回值。
pub fn lambda_prints(body: &Block) -> bool {
    matches!(body[0].0, Instruction::Print(_))
}

/// 结构体字段里存的函数：`p.on_click(x)` 调用它。不是函数字段时返回 None。
pub fn field_function(
    structs: &BTreeMap<String, Struct>,
    receiver: Type,
    field: &str,
) -> Option<u64> {
    let (Type::Record(id) | Type::RecordRef(id, _)) = receiver else {
        return None;
    };
    let structure = structs.iter().find(|(name, _)| record_id(name) == id)?.1;
    match structure.fields.iter().find(|(name, _)| name == field)?.1 {
        Type::Func(func) => Some(func),
        _ => None,
    }
}

/// 结构体（含枚举）、列表与字典：有按类型生成的复制、相等与显示函数。
pub fn compound(ty: Type) -> bool {
    matches!(ty, Type::Record(_) | Type::List(_) | Type::Map(_, _))
}

/// 调用后程序就结束、不再返回的函数：代码块写到它就算走完，不用再补 `return`。
pub fn terminates(expr: &Expr) -> bool {
    matches!(expr, Expr::Call(name, _) if name == "std/env:exit")
}

/// `for` 遍历容器时各个名字的（类型, 能否直接赋值）。`&mut` 时元素可改：字符串与结构体元素是
/// `&mut` 借用，整数、浮点、布尔元素是可赋值的别名（写它就是写元素）。检查器与生成器共用。
pub fn each_bindings(
    container: Type,
    pair: bool,
    mutable: bool,
) -> Result<Vec<(Type, bool)>, String> {
    let item = |elem: Elem| match (elem, mutable) {
        (Elem::Str, true) => (Type::Mutable, false),
        (Elem::Record(id), true) => (Type::RecordRef(id, true), false),
        (Elem::Nested(id), true) => (
            borrow_of(nested(id), true).expect("containers can be borrowed"),
            false,
        ),
        (_, true) => (elem.ty(), true),
        _ => (elem.item(), false),
    };
    Ok(match (container, pair) {
        (Type::List(elem), false) => vec![item(elem)],
        (Type::List(elem), true) => vec![(Type::Int, false), item(elem)],
        (Type::Map(..), false) if mutable => {
            return Err(
                "for k in &mut m cannot change keys; write for k, v in &mut m to change the values"
                    .into(),
            )
        }
        (Type::Map(key, _), false) => vec![(key.item(), false)],
        (Type::Map(key, value), true) => vec![(key.item(), false), item(value)],
        _ => unreachable!("container type"),
    })
}

/// 容器方法的签名：(是否独占借用接收者, 参数类型, 结果类型)。检查器与代码生成共用。
pub fn method_signature(receiver: Type, method: &str) -> Option<(bool, Vec<Type>, Type)> {
    Some(match (receiver.container()?, method) {
        (_, "len") => (false, Vec::new(), Type::Int),
        (Type::List(elem), "push") => (true, vec![elem.ty()], Type::Unit),
        (Type::List(elem), "pop") => (true, Vec::new(), elem.ty()),
        (Type::List(_), "swap") => (true, vec![Type::Int, Type::Int], Type::Unit),
        (Type::List(elem), "insert") => (true, vec![Type::Int, elem.ty()], Type::Unit),
        (Type::List(elem), "remove") => (true, vec![Type::Int], elem.ty()),
        (Type::List(_), "reverse" | "clear") => (true, Vec::new(), Type::Unit),
        // `xs.sort_by(fn(a, b): a.age < b.age)`：`before(a, b)` 为真时 a 排在 b 前面，稳定排序。
        (Type::List(elem), "sort_by") => {
            (true, vec![Type::FuncRef(sort_by_type(elem))], Type::Unit)
        }
        (Type::List(elem), "extend") => (true, vec![Type::List(elem)], Type::Unit),
        // 排序、查找与切片要比较或复制元素，结构体还不行。
        (Type::List(elem), "sort") if elem.comparable() => (true, Vec::new(), Type::Unit),
        (Type::List(elem), "contains") if elem.comparable() => {
            (false, vec![elem.key_arg()], Type::Bool)
        }
        (Type::List(elem), "index_of") if elem.comparable() => {
            (false, vec![elem.key_arg()], Type::Int)
        }
        (Type::List(elem), "slice") if !matches!(elem, Elem::Func(_)) => {
            (false, vec![Type::Int, Type::Int], Type::List(elem))
        }
        (Type::List(elem @ (Elem::Int | Elem::Float)), "sum") => (false, Vec::new(), elem.ty()),
        (Type::List(elem), "min" | "max") if elem.comparable() => (false, Vec::new(), elem.ty()),
        (Type::List(elem), "sorted") if elem.comparable() => (false, Vec::new(), Type::List(elem)),
        (Type::Map(_, _), "clear") => (true, Vec::new(), Type::Unit),
        (Type::Map(_, value), "values") if value.copyable() => {
            (false, Vec::new(), Type::List(value))
        }
        (Type::Map(key, _), "contains") => (false, vec![key.key_arg()], Type::Bool),
        // 结构体不能复制，`get` 的默认值语义对它不成立。
        // `grid[i][j]` 的读写：内层元素按值读（字符串得到副本），写入时替换。
        (Type::List(elem), "$at") if elem.comparable() => (false, vec![Type::Int], elem.ty()),
        (Type::List(elem), "$put") => (true, vec![Type::Int, elem.ty()], Type::Unit),
        (Type::Map(key, value), "$at") if value.comparable() => {
            (false, vec![key.key_arg()], value.ty())
        }
        (Type::Map(key, value), "$put") => (true, vec![key.key_arg(), value.ty()], Type::Unit),
        (Type::Map(key, value), "get") if value.copyable() => {
            (false, vec![key.key_arg(), value.ty()], value.ty())
        }
        (Type::Map(key, _), "remove") => (true, vec![key.key_arg()], Type::Bool),
        (Type::Map(key, _), "keys") => (false, Vec::new(), Type::List(key)),
        _ => return None,
    })
}

/// `sort_by` 的参数类型：`fn(&T, &T) -> bool`（数字和布尔按值）。
pub fn sort_by_type(elem: Elem) -> u64 {
    func_type(vec![elem.item(), elem.item()], Type::Bool)
}

/// 方法名写错或元素类型不支持时，报错里列出能用的方法。
fn supported_methods(container: Type, method: &str) -> &'static str {
    match container {
        Type::List(Elem::Record(_))
            if matches!(
                method,
                "sort" | "sorted" | "contains" | "index_of" | "min" | "max"
            ) =>
        {
            "struct elements have no natural order; use sort_by, or a loop with == to find one"
        }
        Type::List(_) if method == "sum" => "sum needs i64 or f64 elements",
        Type::List(_)
            if matches!(
                method,
                "sort" | "sorted" | "contains" | "index_of" | "slice" | "min" | "max"
            ) =>
        {
            "sort, sorted, contains, index_of, slice, min and max need i64, f64, bool or string elements"
        }
        Type::List(_) => {
            "lists support len, push, pop, insert, remove, swap, reverse, clear, extend, sort, sorted, sort_by, contains, index_of, slice, sum, min and max"
        }
        _ if method.starts_with('$') => {
            "indexing twice, such as grid[i][j], reads i64, f64, bool and string elements; borrow deeper levels with for row in &grid"
        }
        Type::Map(_, value) if method == "values" && !value.comparable() => {
            "struct values cannot be copied; iterate the keys instead"
        }
        _ => "maps support len, contains, get, remove, keys, values and clear",
    }
}

/// `[T]` 的元素与字典的值：标量、字符串或命名结构体（还没有嵌套容器）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Elem {
    Int,
    Float,
    Bool,
    Str,
    Record(u64),
    // 内层是列表或字典（`[[i64]]`、`{string: [string]}`）：编号由内层类型的结构算出，见 `nest`。
    Nested(u64),
    // 函数值 `[fn(i64) -> i64]`：编号是函数类型（`func_sig`）。按值复制，不释放。
    Func(u64),
}

/// 嵌套容器的内层类型表：编号 → 列表或字典类型。编号只取决于类型的结构，同样的类型在任何
/// 模块、任何线程里编号都相同，所以 IR 编码与缓存键是确定的。
static NESTED: std::sync::RwLock<BTreeMap<u64, Type>> = std::sync::RwLock::new(BTreeMap::new());

/// 元组类型表：编号 → （名字, 字段类型）。元组按结构体处理，名字由字段类型拼成（如
/// `(Int, Owned)`），编号是名字的哈希，所以同样的元组类型在所有模块里是同一个类型。
type TupleTable = BTreeMap<u64, (String, Vec<Type>)>;
static TUPLES: std::sync::RwLock<TupleTable> = std::sync::RwLock::new(BTreeMap::new());
/// 字段里的结构体编号已经按模块限定的元组：链接后的模块要带上它们的结构体定义。
static LINKED_TUPLES: std::sync::RwLock<BTreeSet<u64>> = std::sync::RwLock::new(BTreeSet::new());

/// 登记一个元组类型（字段类型可能还没按模块限定）。
pub fn tuple(fields: Vec<Type>) -> (u64, String) {
    let names: Vec<String> = fields.iter().map(|ty| format!("{ty:?}")).collect();
    let name = format!("({})", names.join(", "));
    let id = record_id(&name);
    TUPLES
        .write()
        .expect("tuple table")
        .insert(id, (name.clone(), fields));
    (id, name)
}

/// 登记字段已按模块限定的元组，链接时放进结构体表（泛型模板里还含类型参数的不放）。
pub fn linked_tuple(fields: Vec<Type>) -> u64 {
    let generic = fields.iter().any(|field| mentions_param(*field));
    let (id, _) = tuple(fields);
    if !generic {
        LINKED_TUPLES.write().expect("tuple table").insert(id);
    }
    id
}

/// 元组的字段类型；不是元组时返回 None。
pub fn tuple_fields(id: u64) -> Option<Vec<Type>> {
    TUPLES
        .read()
        .expect("tuple table")
        .get(&id)
        .map(|(_, fields)| fields.clone())
}

/// 链接后的模块要带上的元组：（名字, 字段类型）。
pub fn linked_tuples() -> Vec<(String, Vec<Type>)> {
    let tuples = TUPLES.read().expect("tuple table");
    LINKED_TUPLES
        .read()
        .expect("tuple table")
        .iter()
        .filter_map(|id| tuples.get(id).cloned())
        .collect()
}

/// 已登记的可链接元组个数：检查时新出现的元组类型要重新链接后再检查一遍。
pub fn linked_tuple_count() -> usize {
    LINKED_TUPLES.read().expect("tuple table").len()
}

/// 函数类型表：编号 → （参数类型, 结果类型, 错误类型）。编号是签名文字的哈希，同样的签名在
/// 所有模块里是同一个类型。
type FuncTable = BTreeMap<u64, (Vec<Type>, Type, Option<Type>)>;
static FUNCS: std::sync::RwLock<FuncTable> = std::sync::RwLock::new(BTreeMap::new());

/// 登记一个不会失败的函数类型，返回编号。
pub fn func_type(params: Vec<Type>, result: Type) -> u64 {
    func_type_full(params, result, None)
}

/// 登记一个函数类型（`fn(&string) -> i64!string` 的 error 是 `Some(string)`），返回编号。
pub fn func_type_full(params: Vec<Type>, result: Type, error: Option<Type>) -> u64 {
    let params: Vec<Type> = params
        .into_iter()
        .map(|ty| match ty {
            Type::Func(id) => Type::FuncRef(id),
            other => other,
        })
        .collect();
    let names: Vec<String> = params.iter().map(|ty| format!("{ty:?}")).collect();
    let failure = error.map_or(String::new(), |error| format!("!{error:?}"));
    let id = record_id(&format!("fn({}) -> {result:?}{failure}", names.join(", ")));
    FUNCS
        .write()
        .expect("function types")
        .insert(id, (params, result, error));
    id
}

/// 函数类型的（参数类型, 结果类型）。
pub fn func_sig(id: u64) -> (Vec<Type>, Type) {
    let (params, result, _) = FUNCS.read().expect("function types")[&id].clone();
    (params, result)
}

/// 函数类型的错误类型；不会失败时为 None。
pub fn func_error(id: u64) -> Option<Type> {
    FUNCS.read().expect("function types")[&id].2
}

/// 把列表或字典类型登记为另一个容器的元素。
pub fn nest(container: Type) -> Elem {
    let id = record_id(&format!("nested {container:?}"));
    NESTED
        .write()
        .expect("nested type table")
        .insert(id, container);
    Elem::Nested(id)
}

/// 嵌套元素的内层类型。
pub fn nested(id: u64) -> Type {
    NESTED.read().expect("nested type table")[&id]
}

impl Elem {
    pub fn ty(self) -> Type {
        match self {
            Elem::Int => Type::Int,
            Elem::Float => Type::Float,
            Elem::Bool => Type::Bool,
            Elem::Str => Type::Owned,
            Elem::Record(id) => Type::Record(id),
            Elem::Nested(id) => nested(id),
            Elem::Func(id) => Type::Func(id),
        }
    }
    /// `for x in &items` 拿到的元素：标量按值，字符串、结构体与内层容器是只读借用。
    pub fn item(self) -> Type {
        match self {
            Elem::Str => Type::Shared,
            Elem::Record(id) => Type::RecordRef(id, false),
            Elem::Nested(id) => borrow_of(nested(id), false).expect("containers can be borrowed"),
            _ => self.ty(),
        }
    }
    /// 能比较、能复制的元素：标量与字符串。
    pub fn comparable(self) -> bool {
        matches!(self, Elem::Int | Elem::Float | Elem::Bool | Elem::Str)
    }
    /// 能整个读出来（复制一份）的元素：能比较的元素与函数值。
    pub fn copyable(self) -> bool {
        self.comparable() || matches!(self, Elem::Func(_))
    }
    /// 字典键作为参数时的类型：字符串键只读借用，整数键按值。
    pub fn key_arg(self) -> Type {
        if self == Elem::Str {
            Type::Shared
        } else {
            self.ty()
        }
    }
}

#[derive(Clone, Debug)]
pub enum Expr {
    Record(String, Vec<(String, Expr)>),
    Field(String, String),
    Text(String),
    Format(Vec<Expr>),
    Variable(String),
    Borrow(String, bool),
    Call(String, Vec<Expr>),
    Try(Box<Expr>),
    Catch(Box<Expr>, String, Block),
    ByteLen(Box<Expr>),
    CharLen(Box<Expr>),
    StringEqual(Box<Expr>, Box<Expr>),
    Int(i64),
    Float(f64),
    Bool(bool),
    Unary(char, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Array(Vec<Expr>),
    Index(String, Box<Expr>),
    // `xs[i].a.b`：读取结构体元素的字段路径。
    IndexField(String, Box<Expr>, String),
    // `&xs[i]`、`&mut xs[i].name`：借用元素或元素的字段（字段路径可为空），只能作为调用参数。
    BorrowIndex(String, Box<Expr>, String, bool),
    // `xs.push(v)`：接收者是局部变量或字段路径，由模块解析把非导入别名的 `a.b(...)` 改写而来。
    Method(String, String, Vec<Expr>),
    // `xs[i].tags.push(v)`：接收者是结构体元素里的容器字段（容器路径, 下标, 字段路径, 方法, 参数）。
    ElementMethod(String, Box<Expr>, String, String, Vec<Expr>),
    // 元组字面量 `(a, b)`：类型由各项的类型决定。
    Tuple(Vec<Expr>),
    // `f().len()`、`xs.slice(0, 2).len()`：在调用结果（临时值）上调用列表或字典的方法，
    // 临时值在语句结束时释放。
    Chain(Box<Expr>, String, Vec<Expr>),
    // `{k: v, ...}` 字典字面量；空字典需要类型标注。
    MapLit(Vec<(Expr, Expr)>),
    // `fn(x: i64) -> i64: x * 2`：匿名函数，函数体是一个表达式；参数类型与返回类型可以省略，
    // 按期望的函数类型补上。用到外层局部变量时是闭包，只能直接当实参传。
    // 函数体存成只有一条语句的代码块：`println(x)` 是打印语句，其他是一个表达式。
    Lambda(Vec<(String, Option<Type>)>, Option<Type>, Block),
    // 把具名函数当值用：`double`、`util.double`（模块解析后是限定名）。
    FnRef(String),
    // 调用函数值：`f(x)`，`f` 是函数类型的局部变量或参数。
    Apply(String, Vec<Expr>),
    // 调用求值得到的函数：`steps[0](x)`、`commands["run"](x)`。先求被调用的值，再求实参。
    Invoke(Box<Expr>, Vec<Expr>),
    // `if c { a } else { b }` 作为值：每个分支是单个表达式，必须有 else。
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    // `Shape.Circle(1.0)`、`Shape.Empty`：构造枚举变体（模块解析后是限定的枚举名）。
    Variant(String, String, Vec<Expr>),
    // `match s { Circle(r) => ... }` 作为值：每个分支是单个表达式。
    Match(Box<Expr>, Vec<(Pattern, Expr)>),
}

/// match 分支的模式：变体名（`None` 是 `_`）和按位置绑定的载荷名（`_` 表示不绑定）。
pub type Pattern = (Option<String>, Vec<String>);

#[derive(Clone, Debug)]
pub enum Instruction {
    Let(String, bool, Option<Type>, Expr),
    Assign(String, Expr),
    AssignField(String, String, Expr),
    AssignIndex(String, Expr, Expr),
    // `xs[i].a.b = v`：写结构体元素的字段。
    AssignIndexField(String, Expr, String, Expr),
    Print(Expr),
    Call(Expr),
    Fail(Expr),
    Return(Option<Expr>),
    If(Expr, Block, Block),
    While(Expr, Block),
    // `for name in start..end`：区间左闭右开，起止只求值一次，循环变量是不可变的 i64。
    For(String, Expr, Expr, Block),
    // `for x in &items` / `for i, x in &items` / `for k, v in &m`，`&mut` 时第二个（或唯一的）
    // 名字可以改元素：（第一个名字, 第二个名字, 容器路径, 是否 &mut, 循环体）。
    // 循环期间容器被冻结（不能增删或移走），字符串与结构体元素以借用绑定。
    Each(String, Option<String>, String, bool, Block),
    // `match s { ... }` 语句：分支是代码块或单条语句；匹配期间被匹配的位置冻结。
    Match(Expr, Vec<(Pattern, Block)>),
    // `let (a, b) = expr` / `let mut (a, b) = expr`：把元组的各项移进新变量，`_` 丢弃该项。
    Unpack(Vec<String>, bool, Expr),
    Break,
    Continue,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub public: bool,
    pub params: Vec<(String, Type)>,
    pub result: Type,
    pub error: Option<Type>,
    pub body: Block,
    pub position: SourcePosition,
    // 标准库函数：只有签名，函数体由原生后端按名字提供；检查器不检查它的空函数体。
    pub builtin: bool,
    // 泛型函数的类型参数（`fn largest[T](...)`）。泛型函数本身不检查也不生成，
    // 调用时按实参类型生成专门的副本（`largest[Int]`）。
    pub generics: Vec<String>,
    // `extern fn sqrt(x: f64) -> f64`：没有函数体，调用同名的 C 函数。
    pub external: bool,
    // `@wrapping`：整数 + - * 与取负按二进制补码回绕，浮点 / // % 按 IEEE 754（不检查除以零）。
    pub wrapping: bool,
}

/// `extern fn` 能用的参数与返回类型：数值、布尔、只读字符串参数（C 侧是 `const char *`）、
/// 返回字符串（从 C 的 `const char *` 复制一份）。
pub fn extern_signature(name: &str, params: &[(String, Type)], result: Type) -> Result<(), String> {
    for (param, ty) in params {
        if !matches!(ty, Type::Int | Type::Float | Type::Bool | Type::Shared) {
            return Err(format!(
                "unsupported type for parameter {param} of extern fn {name}; extern functions take i64, f64, bool or &string, so pass other data through a small C wrapper"
            ));
        }
    }
    if !matches!(
        result,
        Type::Unit | Type::Int | Type::Float | Type::Bool | Type::Owned
    ) {
        return Err(format!(
            "unsupported return type for extern fn {name}; extern functions return nothing, i64, f64, bool or string"
        ));
    }
    Ok(())
}

/// 泛型函数的类型参数在类型里写成一个特殊的结构体编号。
pub fn type_param(name: &str) -> u64 {
    let id = record_id(&format!("$param:{name}"));
    PARAMS.write().expect("type parameters").insert(id);
    id
}
static PARAMS: std::sync::RwLock<BTreeSet<u64>> = std::sync::RwLock::new(BTreeSet::new());

/// 类型里还有没换成具体类型的类型参数（泛型模板里的类型）。
pub fn mentions_param(ty: Type) -> bool {
    let params = PARAMS.read().expect("type parameters");
    fn walk(ty: Type, params: &BTreeSet<u64>) -> bool {
        match ty {
            Type::Record(id) | Type::RecordRef(id, _) => {
                params.contains(&id)
                    || tuple_fields(id)
                        .is_some_and(|fields| fields.into_iter().any(|f| walk(f, params)))
                    || instance_of(id)
                        .is_some_and(|(_, args, _)| args.into_iter().any(|a| walk(a, params)))
            }
            Type::List(elem) | Type::ListRef(elem, _) => walk(elem.ty(), params),
            Type::Map(_, value) | Type::MapRef(_, value, _) => walk(value.ty(), params),
            Type::Func(id) | Type::FuncRef(id) => {
                let (args, result) = func_sig(id);
                args.into_iter().chain([result]).any(|ty| walk(ty, params))
            }
            _ => false,
        }
    }
    walk(ty, &params)
}

/// 已生成的泛型函数副本：名字 → 函数。链接时放进函数表。
static INSTANCES: std::sync::RwLock<BTreeMap<String, Function>> =
    std::sync::RwLock::new(BTreeMap::new());

pub fn instances() -> Vec<(String, Function)> {
    INSTANCES
        .read()
        .expect("instance table")
        .iter()
        .map(|(name, function)| (name.clone(), function.clone()))
        .collect()
}

pub fn instance_count() -> usize {
    INSTANCES.read().expect("instance table").len()
}

/// 列表元素或字典值对应的 Elem。
pub fn elem_from(ty: Type) -> Option<Elem> {
    Some(match ty {
        Type::Int => Elem::Int,
        Type::Float => Elem::Float,
        Type::Bool => Elem::Bool,
        Type::Owned => Elem::Str,
        Type::Record(id) => Elem::Record(id),
        Type::List(_) | Type::Map(_, _) => nest(ty),
        Type::Func(id) => Elem::Func(id),
        _ => return None,
    })
}

/// 按实参类型推断类型参数：在形参类型里找到类型参数出现的位置，对上实参类型的同一位置。
fn unify(pattern: Type, actual: Type, params: &BTreeSet<u64>, map: &mut BTreeMap<u64, Type>) {
    match (pattern, actual) {
        (Type::Record(id), _) if params.contains(&id) => {
            map.entry(id).or_insert(actual);
        }
        (Type::List(p), Type::List(a)) => unify_elem(p, a, params, map),
        (Type::Func(p) | Type::FuncRef(p), Type::Func(a) | Type::FuncRef(a)) => {
            let ((p_args, p_result), (a_args, a_result)) = (func_sig(p), func_sig(a));
            for (p, a) in p_args.into_iter().zip(a_args).chain([(p_result, a_result)]) {
                // `fn(&T)` 对上 `fn(&string)`：按借用的目标类型推断。
                let p = p.borrowable().map_or(p, |(owned, _)| owned);
                let a = a.borrowable().map_or(a, |(owned, _)| owned);
                unify(p, a, params, map);
            }
        }
        (Type::Map(_, p), Type::Map(_, a)) => unify_elem(p, a, params, map),
        (Type::Record(p), Type::Record(a)) | (Type::RecordRef(p, _), Type::RecordRef(a, _)) => {
            if let (Some(p), Some(a)) = (tuple_fields(p), tuple_fields(a)) {
                for (p, a) in p.into_iter().zip(a) {
                    unify(p, a, params, map);
                }
            }
            // 同一个泛型类型的两个实例：`Option[T]` 对上 `Option[Int]`，按位置推断类型实参。
            if let (Some((p_owner, p_args, _)), Some((a_owner, a_args, _))) =
                (instance_of(p), instance_of(a))
            {
                if p_owner == a_owner {
                    for (p, a) in p_args.into_iter().zip(a_args) {
                        unify(p, a, params, map);
                    }
                }
            }
        }
        _ => {}
    }
}

fn unify_elem(pattern: Elem, actual: Elem, params: &BTreeSet<u64>, map: &mut BTreeMap<u64, Type>) {
    match (pattern, actual) {
        (Elem::Nested(p), Elem::Nested(a)) => unify(nested(p), nested(a), params, map),
        _ => unify(pattern.ty(), actual.ty(), params, map),
    }
}

/// 把类型参数换成具体类型。`&T` 遇到 i64、f64、bool 时按值传；`&mut T` 不能是这三种。
fn substitute(ty: Type, map: &BTreeMap<u64, Type>) -> Result<Type, String> {
    Ok(match ty {
        Type::Record(id) if map.contains_key(&id) => map[&id],
        Type::RecordRef(id, mutable) if map.contains_key(&id) => match map[&id] {
            concrete @ (Type::Int | Type::Float | Type::Bool | Type::Func(_)) if !mutable => {
                concrete
            }
            concrete => borrow_of(concrete, mutable).ok_or_else(|| {
                format!("type mismatch: &mut T cannot be {concrete:?}; numbers and booleans are passed by value")
            })?,
        },
        Type::Func(id) | Type::FuncRef(id) => {
            let (args, result) = func_sig(id);
            let args = args
                .into_iter()
                .map(|arg| substitute(arg, map))
                .collect::<Result<Vec<_>, _>>()?;
            let error = func_error(id).map(|error| substitute(error, map)).transpose()?;
            let id = func_type_full(args, substitute(result, map)?, error);
            if matches!(ty, Type::Func(_)) {
                Type::Func(id)
            } else {
                Type::FuncRef(id)
            }
        }
        Type::Record(id) => Type::Record(substituted_record(id, map)?),
        Type::RecordRef(id, mutable) => Type::RecordRef(substituted_record(id, map)?, mutable),
        Type::List(elem) => Type::List(substitute_elem(elem, map)?),
        Type::ListRef(elem, mutable) => Type::ListRef(substitute_elem(elem, map)?, mutable),
        Type::Map(key, value) => Type::Map(key, substitute_elem(value, map)?),
        Type::MapRef(key, value, mutable) => {
            Type::MapRef(key, substitute_elem(value, map)?, mutable)
        }
        other => other,
    })
}

/// 元组与泛型实例里的类型参数也要换掉；普通结构体原样返回。
fn substituted_record(id: u64, map: &BTreeMap<u64, Type>) -> Result<u64, String> {
    if let Some(fields) = tuple_fields(id) {
        return substituted_tuple(fields, map);
    }
    if let Some((template, args, _)) = instance_of(id) {
        let args = args
            .into_iter()
            .map(|arg| substitute(arg, map))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(struct_instance(&template, args));
    }
    Ok(id)
}

fn substituted_tuple(fields: Vec<Type>, map: &BTreeMap<u64, Type>) -> Result<u64, String> {
    let fields = fields
        .into_iter()
        .map(|field| substitute(field, map))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(linked_tuple(fields))
}

fn substitute_elem(elem: Elem, map: &BTreeMap<u64, Type>) -> Result<Elem, String> {
    match elem {
        Elem::Record(_) | Elem::Nested(_) => {
            let ty = substitute(elem.ty(), map)?;
            elem_from(ty).ok_or_else(|| format!("unsupported element type {ty:?}"))
        }
        other => Ok(other),
    }
}

/// 函数体里写着类型的地方（`let x: T = ...`，包括 if/循环/match/catch 里的）换成具体类型。
fn substitute_block(block: &mut Block, map: &BTreeMap<u64, Type>) -> Result<(), String> {
    for (instruction, _) in block {
        match instruction {
            Instruction::Let(_, _, annotation, value) => {
                if let Some(ty) = annotation {
                    *ty = substitute(*ty, map)?;
                }
                substitute_catch(value, map)?;
            }
            Instruction::If(_, yes, no) => {
                substitute_block(yes, map)?;
                substitute_block(no, map)?;
            }
            Instruction::While(_, body)
            | Instruction::For(_, _, _, body)
            | Instruction::Each(_, _, _, _, body) => substitute_block(body, map)?,
            Instruction::Match(_, arms) => {
                for (_, body) in arms {
                    substitute_block(body, map)?;
                }
            }
            Instruction::Call(value)
            | Instruction::Assign(_, value)
            | Instruction::Return(Some(value))
            | Instruction::Unpack(_, _, value) => substitute_catch(value, map)?,
            _ => {}
        }
    }
    Ok(())
}

fn substitute_catch(expr: &mut Expr, map: &BTreeMap<u64, Type>) -> Result<(), String> {
    match expr {
        Expr::Catch(_, _, block) => substitute_block(block, map)?,
        Expr::Lambda(params, result, block) => {
            for (_, ty) in params.iter_mut() {
                if let Some(ty) = ty {
                    *ty = substitute(*ty, map)?;
                }
            }
            if let Some(result) = result {
                *result = substitute(*result, map)?;
            }
            substitute_block(block, map)?;
        }
        Expr::Call(_, args) | Expr::Apply(_, args) | Expr::Method(_, _, args) => {
            for arg in args {
                substitute_catch(arg, map)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// 泛型函数当值用（`let f: fn(i64) -> i64 = identity`）：按期望的函数类型的参数推断类型参数，
/// 返回专门副本的名字。检查器与生成器共用。
pub fn generic_value(
    functions: &BTreeMap<String, Function>,
    name: &str,
    expected: u64,
) -> Result<String, String> {
    let actuals: Vec<Option<Type>> = func_sig(expected)
        .0
        .into_iter()
        .map(|ty| Some(ty.borrowable().map_or(ty, |(owned, _)| owned)))
        .collect();
    instance(functions, name, &actuals)
}

/// 调用泛型函数：按实参类型（已去掉借用）推断类型参数，返回专门副本的名字，没生成过就生成。
/// 不是泛型函数时原样返回名字。
pub fn instance(
    functions: &BTreeMap<String, Function>,
    name: &str,
    actuals: &[Option<Type>],
) -> Result<String, String> {
    instance_with(functions, name, actuals, &|_, _| None)
}

/// 同 `instance`；`late(i, 参数类型)` 给出第 i 个实参（没写参数类型的匿名函数）在这些参数类型下
/// 的函数类型：先用其他实参推断出的类型参数补全它的参数，再用它的返回类型推断剩下的。
pub fn instance_with(
    functions: &BTreeMap<String, Function>,
    name: &str,
    actuals: &[Option<Type>],
    late: &dyn Fn(usize, Vec<Type>) -> Option<Type>,
) -> Result<String, String> {
    let Some(template) = functions.get(name).filter(|f| !f.generics.is_empty()) else {
        return Ok(name.to_owned());
    };
    let params: BTreeSet<u64> = template.generics.iter().map(|g| type_param(g)).collect();
    let mut map = BTreeMap::new();
    for ((_, pattern), actual) in template.params.iter().zip(actuals) {
        let pattern = pattern.borrowable().map_or(*pattern, |(owned, _)| owned);
        if let Some(actual) = actual {
            unify(pattern, *actual, &params, &mut map);
        }
    }
    for (index, ((_, pattern), actual)) in template.params.iter().zip(actuals).enumerate() {
        let (Type::Func(id) | Type::FuncRef(id), None) = (pattern, actual) else {
            continue;
        };
        let known: Result<Vec<Type>, String> = func_sig(*id)
            .0
            .into_iter()
            .map(|arg| substitute(arg, &map))
            .collect();
        let Ok(known) = known else { continue };
        if known.iter().any(|arg| mentions_param(*arg)) {
            continue;
        }
        if let Some(actual) = late(index, known) {
            unify(*pattern, actual, &params, &mut map);
        }
    }
    let mut types = Vec::new();
    for generic in &template.generics {
        let ty = map.get(&type_param(generic)).ok_or_else(|| {
            let short = name.split_once(':').map_or(name, |(_, n)| n);
            format!("cannot infer {generic} for {short}: {generic} must appear in a parameter type")
        })?;
        if matches!(ty, Type::ArrayInt(_)) {
            return Err(format!(
                "type mismatch: {generic} cannot be a fixed array; use a list, such as let xs: [i64] = [1, 2]"
            ));
        }
        if matches!(ty, Type::FuncRef(_)) {
            return Err(format!(
                "type mismatch: {generic} cannot be a function parameter that may capture local variables; give the parameter a function type instead"
            ));
        }
        types.push(format!("{ty:?}"));
    }
    let instance = format!("{name}[{}]", types.join(", "));
    if INSTANCES
        .read()
        .expect("instance table")
        .contains_key(&instance)
    {
        return Ok(instance);
    }
    let mut function = template.clone();
    function.generics.clear();
    for (_, ty) in &mut function.params {
        *ty = substitute(*ty, &map)?;
    }
    function.result = substitute(function.result, &map)?;
    if let Some(error) = &mut function.error {
        *error = substitute(*error, &map)?;
    }
    substitute_block(&mut function.body, &map)?;
    INSTANCES
        .write()
        .expect("instance table")
        .insert(instance.clone(), function);
    Ok(instance)
}

#[derive(Clone, Debug)]
pub struct Struct {
    pub public: bool,
    pub fields: Vec<(String, Type)>,
    // 枚举：每个变体的（名字, 第一个载荷字段的下标, 载荷字段数）。字段 0 是隐藏的标签 `$tag`，
    // 各变体的载荷平铺在后面，名字形如 `Circle$r`；没选中的变体字段保持全零，释放时一并释放也安全。
    pub variants: Vec<(String, usize, usize)>,
    // 泛型结构体或枚举的类型参数（`struct Pair[A, B]`、`enum Option[T]`）。模板本身不生成，
    // 用到的具体类型（`Option[Int]`）按结构生成实例。
    pub generics: Vec<String>,
}

/// 泛型结构体与枚举的实例：编号 → （模板名, 类型实参, 实例名）。实例名如 `main:Option[Int]`，
/// 编号是它的哈希，所以同样的实例在所有模块里是同一个类型。
type StructInstances = BTreeMap<u64, (String, Vec<Type>, String)>;
static STRUCT_INSTANCES: std::sync::RwLock<StructInstances> =
    std::sync::RwLock::new(BTreeMap::new());

/// 登记一个泛型结构体实例，返回它的编号。
pub fn struct_instance(template: &str, args: Vec<Type>) -> u64 {
    let names: Vec<String> = args.iter().map(|ty| format!("{ty:?}")).collect();
    let name = format!("{template}[{}]", names.join(", "));
    let id = record_id(&name);
    STRUCT_INSTANCES
        .write()
        .expect("struct instances")
        .insert(id, (template.to_owned(), args, name));
    id
}

/// 编号对应的泛型实例：（模板名, 类型实参, 实例名）。
pub fn instance_of(id: u64) -> Option<(String, Vec<Type>, String)> {
    STRUCT_INSTANCES
        .read()
        .expect("struct instances")
        .get(&id)
        .cloned()
}

/// 要链接的实例：类型实参里还有类型参数的（泛型模板内部用到的 `Option[T]`）不生成。
pub fn struct_instances() -> Vec<(String, Vec<Type>, String)> {
    let all: Vec<_> = STRUCT_INSTANCES
        .read()
        .expect("struct instances")
        .values()
        .cloned()
        .collect();
    all.into_iter()
        .filter(|(_, args, _)| !args.iter().any(|arg| mentions_param(*arg)))
        .collect()
}

pub fn struct_instance_count() -> usize {
    STRUCT_INSTANCES.read().expect("struct instances").len()
}

/// 解析时遇到的 `Option[i64]`：编号 → （模板名, 类型实参），链接时换成实例。
type Applications = BTreeMap<u64, (String, Vec<Type>)>;
static APPLICATIONS: std::sync::RwLock<Applications> = std::sync::RwLock::new(BTreeMap::new());

pub fn application(id: u64) -> Option<(String, Vec<Type>)> {
    APPLICATIONS
        .read()
        .expect("type applications")
        .get(&id)
        .cloned()
}

/// 把类型参数换成具体类型（链接实例的字段时用）。
pub fn substitute_fields(
    generics: &[String],
    args: &[Type],
    fields: &[(String, Type)],
) -> Result<Vec<(String, Type)>, String> {
    let map: BTreeMap<u64, Type> = generics
        .iter()
        .map(|g| type_param(g))
        .zip(args.iter().copied())
        .collect();
    fields
        .iter()
        .map(|(name, ty)| Ok((name.clone(), substitute(*ty, &map)?)))
        .collect()
}

/// 构造结构体或枚举的值时实际的类型：普通结构体就是它自己；泛型模板看期望类型（是它的实例
/// 就用期望类型），否则按字段或载荷的类型推断类型实参。`actual` 给出子表达式的类型。
pub fn constructed(
    structs: &BTreeMap<String, Struct>,
    expr: &Expr,
    expected: Option<Type>,
    actual: impl Fn(&Expr) -> Option<Type>,
) -> Result<u64, String> {
    let (Expr::Record(name, _) | Expr::Variant(name, _, _)) = expr else {
        unreachable!("constructor expression")
    };
    let Some(template) = structs.get(name).filter(|s| !s.generics.is_empty()) else {
        return Ok(record_id(name));
    };
    if let Some(Type::Record(id)) = expected {
        if instance_of(id).is_some_and(|(owner, _, _)| owner == *name) {
            return Ok(id);
        }
    }
    let pairs: Vec<(Type, &Expr)> = match expr {
        Expr::Record(_, fields) => fields
            .iter()
            .filter_map(|(field, value)| {
                let pattern = template.fields.iter().find(|(n, _)| n == field)?.1;
                Some((pattern, value))
            })
            .collect(),
        Expr::Variant(_, variant, args) => {
            let Some((_, first, _)) = template.variants.iter().find(|(v, _, _)| v == variant)
            else {
                return Ok(record_id(name));
            };
            template.fields[*first..]
                .iter()
                .zip(args)
                .map(|((_, pattern), arg)| (*pattern, arg))
                .collect()
        }
        _ => unreachable!(),
    };
    let params: BTreeSet<u64> = template.generics.iter().map(|g| type_param(g)).collect();
    let mut map = BTreeMap::new();
    for (pattern, value) in pairs {
        // `[1.5, 2.5]` 本身会被当成定长数组；填进泛型位置时是列表，按第一项推断元素类型。
        let ty = match (pattern, value) {
            (_, Expr::Array(items)) => items
                .first()
                .and_then(&actual)
                .and_then(elem_from)
                .map(Type::List),
            _ => actual(value),
        };
        if let Some(ty) = ty {
            unify(pattern, ty, &params, &mut map);
        }
    }
    let short = name.rsplit(':').next().unwrap_or(name);
    let mut args = Vec::new();
    for generic in &template.generics {
        let ty = *map.get(&type_param(generic)).ok_or_else(|| {
            format!("cannot infer {generic} for {short}; annotate the type, such as let x: {short}[i64] = ...")
        })?;
        if ty.is_reference() || matches!(ty, Type::ArrayInt(_) | Type::Unit) {
            return Err(format!(
                "type mismatch: {generic} of {short} must be an owned type such as i64, string or a list"
            ));
        }
        args.push(ty);
    }
    Ok(struct_instance(name, args))
}

/// 内置的 `Option[T]`：`Some(value: T)` 或 `None`，所有模块都能直接用（模块自己声明了
/// `Option` 时用自己的）。链接后叫 `std/core:Option`。
pub const OPTION: &str = "std/core:Option";
pub fn option() -> Struct {
    Struct {
        public: true,
        fields: vec![
            ("$tag".into(), Type::Int),
            ("Some$value".into(), Type::Record(type_param("T"))),
        ],
        variants: vec![("Some".into(), 1, 1), ("None".into(), 2, 0)],
        generics: vec!["T".into()],
    }
}

/// 顶层常量：值在解析时算好，是整数、浮点、布尔或字符串字面量；用到的地方直接换成这个字面量。
#[derive(Clone, Debug)]
pub struct Constant {
    pub public: bool,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub structs: BTreeMap<String, Struct>,
    pub type_names: BTreeMap<u64, String>,
    pub imports: Vec<String>,
    pub functions: BTreeMap<String, Function>,
    pub constants: BTreeMap<String, Constant>,
}

/// 常量名全大写（可含数字与下划线），与结构体的大驼峰名区分。
pub fn constant_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// 在编译期算出常量表达式：只能用字面量、前面声明过的常量和运算符。整数溢出、除零、
/// 移位量越界在这里直接报错，与运行时的规则一致。
fn fold(expr: &Expr, constants: &BTreeMap<String, Constant>) -> Result<Expr, String> {
    let fail = || "constants can only use literals, earlier constants and operators".to_owned();
    let overflow = || "integer arithmetic error in constant".to_owned();
    Ok(match expr {
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Text(_) => expr.clone(),
        Expr::Variable(name) => constants
            .get(name)
            .map(|constant| constant.value.clone())
            .ok_or_else(|| format!("unknown constant {name}; {}", fail()))?,
        Expr::Unary(op, value) => match (op, fold(value, constants)?) {
            ('-', Expr::Int(n)) => Expr::Int(n.checked_neg().ok_or_else(overflow)?),
            ('-', Expr::Float(x)) => Expr::Float(-x),
            ('~', Expr::Int(n)) => Expr::Int(!n),
            ('!', Expr::Bool(b)) => Expr::Bool(!b),
            ('f', Expr::Int(n)) => Expr::Float(n as f64),
            _ => return Err("type mismatch in constant".into()),
        },
        Expr::Binary(op, left, right) => {
            let (a, b) = (fold(left, constants)?, fold(right, constants)?);
            let compare = |order: std::cmp::Ordering| {
                use std::cmp::Ordering::*;
                Expr::Bool(match op.as_str() {
                    "==" => order == Equal,
                    "!=" => order != Equal,
                    "<" => order == Less,
                    "<=" => order != Greater,
                    ">" => order == Greater,
                    _ => order != Less,
                })
            };
            let comparison = matches!(op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=");
            match (a, b) {
                (Expr::Int(x), Expr::Int(y)) if comparison => compare(x.cmp(&y)),
                (Expr::Int(x), Expr::Int(y)) if op == "/" => {
                    if y == 0 {
                        return Err("division by zero in a constant".into());
                    }
                    Expr::Float(x as f64 / y as f64)
                }
                (Expr::Int(x), Expr::Int(y)) => Expr::Int(
                    match op.as_str() {
                        "+" => x.checked_add(y),
                        "-" => x.checked_sub(y),
                        "*" => x.checked_mul(y),
                        "/" => x.checked_div(y),
                        "//" => x
                            .checked_div_euclid(y)
                            .map(|_| x.div_euclid(y))
                            .and_then(|_| {
                                let q = x.checked_div(y)?;
                                Some(if x % y != 0 && (x < 0) != (y < 0) {
                                    q - 1
                                } else {
                                    q
                                })
                            }),
                        "%" if y == -1 => Some(0),
                        "%" => x.checked_rem(y).map(|r| {
                            if r != 0 && (r < 0) != (y < 0) {
                                r + y
                            } else {
                                r
                            }
                        }),
                        "&" => Some(x & y),
                        "|" => Some(x | y),
                        "^" => Some(x ^ y),
                        "<<" => (0..64).contains(&y).then(|| ((x as u64) << y) as i64),
                        ">>" => (0..64).contains(&y).then(|| x >> y),
                        _ => return Err("type mismatch in constant".into()),
                    }
                    .ok_or_else(overflow)?,
                ),
                (Expr::Float(x), Expr::Float(y)) if comparison => {
                    // NaN 与任何数比较都不成立，只有 != 为真。
                    match x.partial_cmp(&y) {
                        Some(order) => compare(order),
                        None => Expr::Bool(op == "!="),
                    }
                }
                (Expr::Float(x), Expr::Float(y)) => Expr::Float(match op.as_str() {
                    "+" => x + y,
                    "-" => x - y,
                    "*" => x * y,
                    "/" | "//" | "%" if y == 0.0 => {
                        return Err("division by zero in a constant".into())
                    }
                    "/" => x / y,
                    "//" => (x / y).floor(),
                    "%" => {
                        let r = x % y;
                        if r != 0.0 && (r < 0.0) != (y < 0.0) {
                            r + y
                        } else {
                            r
                        }
                    }
                    _ => return Err("type mismatch in constant".into()),
                }),
                (Expr::Bool(x), Expr::Bool(y)) => Expr::Bool(match op.as_str() {
                    "&&" => x && y,
                    "||" => x || y,
                    "==" => x == y,
                    "!=" => x != y,
                    _ => return Err("type mismatch in constant".into()),
                }),
                (Expr::Text(x), Expr::Text(y)) if comparison => compare(x.cmp(&y)),
                (Expr::Text(x), Expr::Text(y)) if op == "+" => Expr::Text(x + &y),
                // 整数与小数混用：整数变成小数（与 Python 相同）。
                (Expr::Int(x), Expr::Float(y)) => {
                    return fold(
                        &Expr::Binary(
                            op.clone(),
                            Box::new(Expr::Float(x as f64)),
                            Box::new(Expr::Float(y)),
                        ),
                        constants,
                    )
                }
                (Expr::Float(x), Expr::Int(y)) => {
                    return fold(
                        &Expr::Binary(
                            op.clone(),
                            Box::new(Expr::Float(x)),
                            Box::new(Expr::Float(y as f64)),
                        ),
                        constants,
                    )
                }
                _ => return Err("type mismatch in constant".into()),
            }
        }
        _ => return Err(fail()),
    })
}

/// 常量字面量的类型。
fn literal_type(expr: &Expr) -> Type {
    match expr {
        Expr::Int(_) => Type::Int,
        Expr::Float(_) => Type::Float,
        Expr::Bool(_) => Type::Bool,
        _ => Type::Owned,
    }
}

type SourcePosition = (usize, usize);
// 语句序列：每条语句带上它在源码中的起始位置，供语义错误定位。
pub type Block = Vec<(Instruction, SourcePosition)>;

/// 词法分析。括号外的每个逻辑行以 `Newline` 结束；行首缩进比上一层深时产生 `Indent`，
/// 变浅时按缩进栈逐层产生 `Dedent`。括号里的换行和缩进不算，空行与只有注释的行跳过。
/// 返回记号、每个记号的起始位置，以及整个文件是否几乎没有缩进（缺缩进时据此提示复制丢了缩进）。
fn lex(source: &str) -> Result<(Vec<Token>, Vec<SourcePosition>, bool), String> {
    let physical: Vec<&str> = source.split('\n').collect();
    let count = physical.len();
    let mut tokens = Vec::new();
    let mut lines = Vec::new();
    let mut indents = vec![0usize];
    // 还没闭合的括号：（括号字符, 位置）。括号层数就是它的长度。
    let mut open: Vec<(char, SourcePosition)> = Vec::new();
    let mut pending = false;
    let (mut code, mut indented) = (0usize, 0usize);
    for (index, raw) in physical.iter().enumerate() {
        let line = index + 1;
        let chars: Vec<char> = raw.trim_end_matches('\r').chars().collect();
        let last = index + 1 == count;
        let mut i = 0;
        if open.is_empty() {
            let start = chars
                .iter()
                .position(|c| !c.is_whitespace())
                .unwrap_or(chars.len());
            let rest = &chars[start..];
            if rest.is_empty() || rest[0] == '#' {
                continue;
            }
            if let Some(tab) = chars[..start].iter().position(|c| *c == '\t') {
                return Err(format!(
                    "line {line}, column {}: indentation contains a tab; run `zam fmt` to replace tabs with spaces",
                    tab + 1
                ));
            }
            code += 1;
            indented += usize::from(start > 0);
            if start > *indents.last().unwrap() {
                indents.push(start);
                tokens.push(Token::Indent);
                lines.push((line, start + 1));
            } else {
                while start < *indents.last().unwrap() {
                    indents.pop();
                    tokens.push(Token::Dedent);
                    lines.push((line, start + 1));
                }
                if start != *indents.last().unwrap() {
                    return Err(format!(
                        "line {line}, column {}: unindent does not match any outer indentation level",
                        start + 1
                    ));
                }
            }
            i = start;
        }
        while i < chars.len() {
            let c = chars[i];
            let column = i + 1;
            let error = |message: &str| format!("line {line}, column {column}: {message}");
            let at = |j: usize| chars.get(j).copied();
            let token = match c {
                c if c.is_whitespace() => {
                    i += 1;
                    continue;
                }
                '#' => break,
                '/' if at(i + 1) == Some('/') && chars[..i].iter().all(|c| c.is_whitespace()) => {
                    return Err(error(
                        "comments start with #; // is floor division, as in 7 // 2",
                    ));
                }
                '"' => {
                    let mut text = String::new();
                    let mut j = i + 1;
                    loop {
                        match at(j) {
                            None => {
                                return Err(error(if last {
                                    "unterminated string literal"
                                } else {
                                    "newline in string literal"
                                }))
                            }
                            Some('"') => break,
                            Some('\\') => {
                                text.push(match at(j + 1) {
                                    Some('n') => '\n',
                                    Some('r') => '\r',
                                    Some('t') => '\t',
                                    Some('"') => '"',
                                    Some('\\') => '\\',
                                    _ => return Err(error("unsupported string escape")),
                                });
                                j += 2;
                            }
                            Some(c) => {
                                text.push(c);
                                j += 1;
                            }
                        }
                    }
                    i = j + 1;
                    Token::Text(text)
                }
                // 字符字面量 `'a'`：值是该字符的 Unicode 码点（i64），ASCII 字符等于 byte_at 读到的字节。
                '\'' => {
                    let single = "character literal must contain exactly one character";
                    let (value, width) = match at(i + 1) {
                        Some('\\') => (
                            match at(i + 2) {
                                Some('n') => '\n',
                                Some('r') => '\r',
                                Some('t') => '\t',
                                Some('0') => '\0',
                                Some('\\') => '\\',
                                Some('\'') => '\'',
                                Some('"') => '"',
                                _ => return Err(error("unsupported character escape")),
                            },
                            2,
                        ),
                        Some('\'') | None => return Err(error(single)),
                        Some(c) => (c, 1),
                    };
                    if at(i + 1 + width) != Some('\'') {
                        return Err(error(single));
                    }
                    i += width + 2;
                    Token::Number(u32::from(value).to_string())
                }
                c if c.is_ascii_alphabetic() || c == '_' => {
                    let mut j = i;
                    while at(j).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                        j += 1;
                    }
                    let word = chars[i..j].iter().collect();
                    i = j;
                    Token::Word(word)
                }
                // `0xFF`、`0b1010`、`0o17`：与 Python 一样的进制前缀，数字之间可以有单个 `_`。
                // 记号里存十进制文本，后面按普通整数处理（超出 i64 照常报溢出）。
                '0' if matches!(at(i + 1), Some('x' | 'X' | 'b' | 'B' | 'o' | 'O')) => {
                    let radix = match at(i + 1) {
                        Some('x' | 'X') => 16,
                        Some('b' | 'B') => 2,
                        _ => 8,
                    };
                    let mut j = i + 2;
                    while at(j).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                        j += 1;
                    }
                    let text: String = chars[i + 2..j].iter().collect();
                    if text.starts_with('_') || text.ends_with('_') || text.contains("__") {
                        return Err(error("integer separators must occur between digits"));
                    }
                    let digits = text.replace('_', "");
                    let value = u128::from_str_radix(&digits, radix).map_err(|_| {
                        error(&format!(
                            "invalid base-{radix} integer literal {}",
                            chars[i..j].iter().collect::<String>()
                        ))
                    })?;
                    i = j;
                    Token::Number(value.to_string())
                }
                c if c.is_ascii_digit() => {
                    let mut j = i;
                    let digits = |j: &mut usize| {
                        while at(*j).is_some_and(|c| c.is_ascii_digit() || c == '_') {
                            *j += 1;
                        }
                    };
                    digits(&mut j);
                    let integer: String = chars[i..j].iter().collect();
                    if integer.ends_with('_') || integer.contains("__") {
                        return Err(error("integer separators must occur between digits"));
                    }
                    let mut float = false;
                    // `1.5` 是浮点数；`0..n` 的点后面不是数字，整数照旧，点交给区间。
                    if at(j) == Some('.') && at(j + 1).is_some_and(|c| c.is_ascii_digit()) {
                        float = true;
                        j += 1;
                        digits(&mut j);
                    }
                    if matches!(at(j), Some('e' | 'E')) {
                        float = true;
                        j += 1;
                        if matches!(at(j), Some('+' | '-')) {
                            j += 1;
                        }
                        if !at(j).is_some_and(|c| c.is_ascii_digit()) {
                            return Err(error(
                                "expected digits in the f64 exponent, such as 1e9 or 2.5e-3",
                            ));
                        }
                        while at(j).is_some_and(|c| c.is_ascii_digit()) {
                            j += 1;
                        }
                    }
                    let number: String = chars[i..j].iter().collect();
                    i = j;
                    if !float {
                        Token::Number(number.replace('_', ""))
                    } else {
                        let bytes = number.as_bytes();
                        if (0..bytes.len()).any(|k| {
                            bytes[k] == b'_'
                                && !(bytes[k - 1].is_ascii_digit()
                                    && bytes.get(k + 1).is_some_and(u8::is_ascii_digit))
                        }) {
                            return Err(error("digit separators must occur between digits"));
                        }
                        let text = number.replace('_', "");
                        if !text.parse::<f64>().is_ok_and(f64::is_finite) {
                            return Err(error("f64 literal is out of range"));
                        }
                        Token::Float(text)
                    }
                }
                '(' | ')' | '{' | '}' | '[' | ']' | '/' | '.' | ':' | ',' | '=' | '&' | '-'
                | '>' | '<' | '+' | '*' | '%' | '!' | '|' | ';' | '?' | '^' | '~' | '@' => {
                    match c {
                        '(' | '[' | '{' => open.push((c, (line, column))),
                        ')' | ']' | '}' => {
                            open.pop();
                        }
                        _ => {}
                    }
                    i += 1;
                    Token::Symbol(c)
                }
                _ => return Err(error(&format!("unexpected character {c:?}"))),
            };
            tokens.push(token);
            lines.push((line, column));
            pending = true;
        }
        if open.is_empty() && pending {
            tokens.push(Token::Newline);
            lines.push((line, chars.len() + 1));
            pending = false;
        }
    }
    if let Some((c, (line, column))) = open.last() {
        return Err(format!(
            "line {line}, column {column}: '{c}' was never closed"
        ));
    }
    let end = (
        count,
        physical[count - 1].trim_end_matches('\r').chars().count() + 1,
    );
    if pending {
        tokens.push(Token::Newline);
        lines.push(end);
    }
    while indents.len() > 1 {
        indents.pop();
        tokens.push(Token::Dedent);
        lines.push(end);
    }
    lines.push(end);
    Ok((tokens, lines, code >= 2 && indented * 10 <= code))
}

/// 行首空白里的 Tab 展开成空格（补到下一个 4 的倍数）；行内其他位置的 Tab 不动。
pub fn expand_tabs(source: &str) -> String {
    source
        .split('\n')
        .map(|line| {
            let mut out = String::new();
            let mut column = 0;
            let mut chars = line.chars().peekable();
            while let Some(&c) = chars.peek() {
                match c {
                    ' ' => column += 1,
                    '\t' => {
                        let width = 4 - column % 4;
                        out.push_str(&" ".repeat(width));
                        column += width;
                        chars.next();
                        continue;
                    }
                    _ => break,
                }
                out.push(c);
                chars.next();
            }
            out.extend(chars);
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 一个逻辑行在格式化时需要的信息。
struct Logical {
    first: usize,
    last: usize,
    depth: usize,
    head: String,
    // 末尾是 `:` 且下一行缩进进去：后面跟着一个缩进块。
    opens: bool,
}

/// 格式化：缩进统一成 4 个空格，括号里的续行按括号层数缩进，注释按上下文的层级缩进；
/// 多行形式的 if/for/while/match 前后各留一个空行（块里第一条、前一行是注释、块结束处除外），
/// 连续空行压成一个，顶层声明之间留一个空行。注释和字符串原样保留，换行风格不变。
pub fn format(source: &str) -> Result<String, String> {
    if source.trim().is_empty() {
        return Ok(String::new());
    }
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let text = expand_tabs(&source.replace("\r\n", "\n"));
    parse(&text)?;
    let (tokens, positions, _) = lex(&text)?;
    let physical: Vec<&str> = text.split('\n').collect();
    // 逻辑行：从 Indent/Dedent 之后的第一个记号到 Newline。
    let mut logical: Vec<Logical> = Vec::new();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Indent => depth += 1,
            Token::Dedent => depth -= 1,
            Token::Newline => {
                let first = start.take().expect("logical line has tokens");
                logical.push(Logical {
                    first: positions[first].0 - 1,
                    last: positions[i].0 - 1,
                    depth,
                    // `pub const` 的开头按 `const` 算，连续的常量之间不加空行。
                    head: match (&tokens[first], tokens.get(first + 1)) {
                        (Token::Word(word), Some(Token::Word(next))) if word == "pub" => {
                            next.clone()
                        }
                        (Token::Word(word), _) => word.clone(),
                        (Token::Symbol('@'), _) => "@".into(),
                        _ => String::new(),
                    },
                    opens: tokens.get(i + 1) == Some(&Token::Indent),
                });
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    // 每个物理行属于哪个逻辑行，以及续行行首的括号层数（行首是闭括号时少一层）。
    let mut owner: Vec<Option<usize>> = vec![None; physical.len()];
    let mut nesting = vec![0usize; physical.len()];
    let mut by_line: Vec<Vec<&Token>> = vec![Vec::new(); physical.len()];
    for (i, token) in tokens.iter().enumerate() {
        if !matches!(token, Token::Indent | Token::Dedent | Token::Newline) {
            by_line[positions[i].0 - 1].push(token);
        }
    }
    for (index, line) in logical.iter().enumerate() {
        let mut bracket = 0usize;
        for number in line.first..=line.last {
            owner[number] = Some(index);
            let closes = by_line[number]
                .first()
                .is_some_and(|t| matches!(t, Token::Symbol(')' | ']' | '}')));
            nesting[number] = bracket.saturating_sub(usize::from(closes));
            for token in &by_line[number] {
                match token {
                    Token::Symbol('(' | '[' | '{') => bracket += 1,
                    Token::Symbol(')' | ']' | '}') => bracket = bracket.saturating_sub(1),
                    _ => {}
                }
            }
        }
    }
    #[derive(Clone, PartialEq)]
    enum Entry {
        Blank,
        Comment(usize, String),
        // 物理行文字、缩进层级、所属逻辑行、是否是逻辑行的第一行。
        Code(usize, String, usize, bool),
    }
    let mut entries: Vec<Entry> = Vec::new();
    // 每层缩进在原文里的列，用来给注释找层级。
    let mut columns: Vec<usize> = vec![0];
    let mut previous: Option<usize> = None;
    for (number, raw) in physical.iter().enumerate() {
        let trimmed = raw.trim();
        if let Some(index) = owner[number] {
            let line = &logical[index];
            if number == line.first {
                let column = raw.len() - raw.trim_start().len();
                columns.truncate(line.depth);
                columns.push(column);
                previous = Some(index);
                entries.push(Entry::Code(line.depth, trimmed.to_owned(), index, true));
            } else if trimmed.is_empty() {
                entries.push(Entry::Blank);
            } else {
                entries.push(Entry::Code(
                    line.depth + nesting[number],
                    trimmed.to_owned(),
                    index,
                    false,
                ));
            }
            continue;
        }
        if trimmed.is_empty() {
            entries.push(Entry::Blank);
            continue;
        }
        // 注释行：紧跟块头时属于块；否则取原文列不超过它的最深一层，夹在前后两行的层级之间。
        let next = (number..physical.len())
            .find_map(|n| owner[n])
            .map(|i| logical[i].depth);
        let before = previous.map_or(0, |i| logical[i].depth);
        let after = next.unwrap_or(0);
        let column = raw.len() - raw.trim_start().len();
        let level = if after > before {
            after
        } else {
            (after..=before)
                .rev()
                .find(|&k| columns.get(k).is_some_and(|&c| c <= column))
                .unwrap_or(after)
        };
        entries.push(Entry::Comment(level, trimmed.to_owned()));
    }
    // 语句的范围：if 连同它的 elif/else，一直到下一条同层或更浅的语句之前。
    let end_of = |index: usize| {
        let depth = logical[index].depth;
        let chain = logical[index].head == "if";
        let mut end = index;
        for (k, line) in logical.iter().enumerate().skip(index + 1) {
            if line.depth < depth
                || (line.depth == depth
                    && !(chain && matches!(line.head.as_str(), "elif" | "else")))
            {
                break;
            }
            end = k;
        }
        end
    };
    let mut before_line: BTreeSet<usize> = BTreeSet::new();
    // 语句结束的物理行 → 语句的层级；空行加在之后第一个不比它深的行前面（块内的尾注释留在块里）。
    let mut after_line: BTreeMap<usize, usize> = BTreeMap::new();
    for (index, line) in logical.iter().enumerate() {
        if !matches!(line.head.as_str(), "if" | "for" | "while" | "match") {
            continue;
        }
        let end = end_of(index);
        if !(index..=end).any(|k| logical[k].opens) {
            continue;
        }
        let first_in_block = index == 0 || logical[index - 1].depth < line.depth;
        if !first_in_block {
            before_line.insert(line.first);
        }
        let next = logical.get(end + 1);
        if next.is_some_and(|next| next.depth == line.depth) {
            after_line.insert(logical[end].last, line.depth);
        }
    }
    // 顶层声明之间（连续的 use、const、extern fn 除外），空行放在声明前面紧挨着的注释之前。
    let top: Vec<usize> = (0..logical.len())
        .filter(|&i| logical[i].depth == 0)
        .collect();
    for pair in top.windows(2) {
        let (a, b) = (&logical[pair[0]], &logical[pair[1]]);
        if a.head == b.head && matches!(a.head.as_str(), "use" | "const" | "extern") {
            continue;
        }
        // 装饰器紧贴着它下面的 fn。
        if a.head == "@" {
            continue;
        }
        before_line.insert(b.first);
    }
    let mut out: Vec<Entry> = Vec::new();
    let mut pending: Option<usize> = None;
    for (number, entry) in entries.into_iter().enumerate() {
        if let Some(depth) = pending {
            match &entry {
                Entry::Blank => pending = None,
                Entry::Comment(level, _) if *level > depth => {}
                _ => {
                    if out.last() != Some(&Entry::Blank) {
                        out.push(Entry::Blank);
                    }
                    pending = None;
                }
            }
        }
        let wants_blank = before_line.contains(&number);
        if wants_blank {
            // 往回跳过紧挨着的注释，空行放在注释前面；前一行已经是注释时不加（顶层除外）。
            let top_level = owner[number].is_some_and(|i| logical[i].depth == 0);
            let mut at = out.len();
            if top_level {
                while at > 0 && matches!(out[at - 1], Entry::Comment(..)) {
                    at -= 1;
                }
            }
            if at > 0 && matches!(out[at - 1], Entry::Code(..)) {
                out.insert(at, Entry::Blank);
            }
        }
        if entry == Entry::Blank && (out.is_empty() || out.last() == Some(&Entry::Blank)) {
            continue;
        }
        out.push(entry);
        if let Some(&depth) = after_line.get(&number) {
            pending = Some(depth);
        }
    }
    while out.last() == Some(&Entry::Blank) {
        out.pop();
    }
    let mut formatted = String::new();
    for entry in &out {
        match entry {
            Entry::Blank => {}
            Entry::Comment(level, text) | Entry::Code(level, text, _, _) => {
                formatted.push_str(&"    ".repeat(*level));
                formatted.push_str(text);
            }
        }
        formatted.push_str(newline);
    }
    Ok(formatted)
}

struct Parser {
    tokens: Vec<Token>,
    lines: Vec<SourcePosition>,
    position: usize,
    type_names: BTreeMap<u64, String>,
    // 当前函数的上下文；`catch` 的代码块出现在表达式位置，需要它来解析语句。
    imports: Vec<String>,
    result: Type,
    // 块末尾的调用被暂当作隐式返回值的语句位置；函数体解析完后由 `untail` 按尾部位置修正。
    implicit: BTreeSet<SourcePosition>,
    // 写了 `return` 关键字的语句位置。其他 Return 来自块尾的裸表达式：catch 块里它是回退值。
    explicit: BTreeSet<SourcePosition>,
    // 整个文件几乎没有缩进：缺缩进块的报错里提示可能是复制时丢了缩进。
    flat: bool,
    // 要插在当前语句前面的语句：对调用结果做字面量 match 时，先把值存进隐藏变量。
    pending: Vec<Instruction>,
    temps: usize,
    // 正在解析的泛型函数的类型参数：这些名字是类型参数，不是结构体。
    type_params: Vec<String>,
    // 正在解析的泛型结构体或枚举的类型参数：方法的 self 是 `Owner[T]`。
    owner_generics: Vec<String>,
    // 源码各行：`assert` 失败时把条件原文写进报错。
    text: Vec<String>,
}

/// match 的一个分支：枚举变体、字面量（`1 | 2`、`"a"`、`true`）或 `_`。
enum Arm {
    Variant(Pattern),
    Values(Vec<Expr>),
    Wildcard,
}
impl Arm {
    fn pattern(self) -> Pattern {
        match self {
            Arm::Variant(pattern) => pattern,
            _ => (None, Vec::new()),
        }
    }
}

/// 没有调用的表达式：重复求值不会产生额外副作用（复合赋值的下标用它判断）。
fn pure(expr: &Expr) -> bool {
    match expr {
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::Bool(_)
        | Expr::Text(_)
        | Expr::Variable(_)
        | Expr::Borrow(_, _)
        | Expr::Field(_, _) => true,
        Expr::Format(segments) => segments.iter().all(pure),
        Expr::Unary(_, inner) | Expr::Index(_, inner) | Expr::IndexField(_, inner, _) => {
            pure(inner)
        }
        Expr::Binary(_, left, right) => pure(left) && pure(right),
        _ => false,
    }
}

fn is_text(ty: Type) -> bool {
    matches!(ty, Type::Owned | Type::Shared | Type::Mutable)
}

/// 算术运算的结果类型：左操作数是 f64 就是 f64，否则按 i64 检查。
/// 位运算：只用于 i64，结果也是 i64。
pub fn bitwise(op: &str) -> bool {
    matches!(op, "&" | "|" | "^" | "<<" | ">>")
}

pub fn numeric(ty: Type) -> Type {
    if ty == Type::Float {
        Type::Float
    } else {
        Type::Int
    }
}

/// 只有函数尾部的调用才是隐式返回值：函数体的最后一条，以及尾部 if/else 分支的最后一条。
/// 循环体与非尾部分支里的末尾调用只求值、不返回（否则 `while c { f() }` 会在第一轮就返回）。
/// `catch` 代码块必须返回，保持原样。
/// `call() catch e: value` 的回退值：catch 块只有一个表达式语句时就是它。检查器与生成器共用。
pub fn catch_fallback(block: &Block) -> Option<&Expr> {
    match block.as_slice() {
        [(Instruction::Call(value), _)] => Some(value),
        _ => None,
    }
}
/// 回退值的类型能不能当作调用的成功值：同类型，整数当小数，借用的字符串会复制成 owned。
pub fn fallback_fits(found: Type, success: Type) -> bool {
    found == success
        || (found == Type::Int && success == Type::Float)
        || (success == Type::Owned && is_string(found))
}
fn untail(body: &mut Block, tail: bool, implicit: &BTreeSet<SourcePosition>) {
    let last = body.len().saturating_sub(1);
    for (index, (instruction, position)) in body.iter_mut().enumerate() {
        let here = tail && index == last;
        match instruction {
            Instruction::Return(Some(expr)) if !here && implicit.contains(position) => {
                *instruction = Instruction::Call(expr.clone());
            }
            Instruction::If(_, yes, no) => {
                untail(yes, here, implicit);
                untail(no, here, implicit);
            }
            Instruction::Match(_, arms) => {
                for (_, arm) in arms {
                    untail(arm, here, implicit);
                }
            }
            Instruction::While(_, nested)
            | Instruction::For(_, _, _, nested)
            | Instruction::Each(_, _, _, _, nested) => untail(nested, false, implicit),
            _ => {}
        }
    }
}
impl Parser {
    fn at(&self) -> SourcePosition {
        self.lines[self.position.min(self.lines.len() - 1)]
    }
    /// 从第 `start` 个记号到当前位置之前的源码原文（跨行时只取第一行）。
    fn snippet(&self, start: usize) -> String {
        let (line, column) = self.lines[start];
        let Some(text) = self.text.get(line.wrapping_sub(1)) else {
            return String::new();
        };
        let chars: Vec<char> = text.chars().collect();
        let from = column.saturating_sub(1).min(chars.len());
        let to = match self.lines.get(self.position) {
            Some((end_line, end_column)) if *end_line == line => {
                end_column.saturating_sub(1).clamp(from, chars.len())
            }
            _ => chars.len(),
        };
        chars[from..to].iter().collect::<String>().trim().to_owned()
    }
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }
    fn take(&mut self) -> Option<Token> {
        let token = self.peek().cloned();
        self.position += usize::from(token.is_some());
        token
    }
    fn symbol(&mut self, c: char) -> bool {
        if self.peek() == Some(&Token::Symbol(c)) {
            self.take();
            true
        } else {
            false
        }
    }
    fn keyword(&mut self, word: &str) -> bool {
        if self.peek() == Some(&Token::Word(word.into())) {
            self.take();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.symbol(c) {
            Ok(())
        } else {
            Err(format!("expected {c:?}, found {}", shown(self.peek())))
        }
    }
    fn word(&mut self) -> Result<String, String> {
        match self.take() {
            Some(Token::Word(word)) if matches!(word.as_str(), "and" | "or" | "not" | "assert") => {
                Err(format!(
                    "expected identifier, found the reserved word {word}"
                ))
            }
            Some(Token::Word(word)) => Ok(word),
            token => Err(format!(
                "expected identifier, found {}",
                shown(token.as_ref())
            )),
        }
    }
    fn newlines(&mut self) {
        while self.peek() == Some(&Token::Newline) {
            self.take();
        }
    }
    /// 语句之间：换行；以缩进块结尾的语句（上一个记号是 Dedent）后面不再有换行。
    fn separator(&mut self) -> Result<(), String> {
        match self.peek() {
            Some(Token::Newline) => {
                self.take();
                Ok(())
            }
            None | Some(Token::Dedent) => Ok(()),
            _ if self.ended_block() => Ok(()),
            token => Err(format!(
                "expected newline between statements, found {}",
                shown(token)
            )),
        }
    }
    /// 刚读完一个缩进块：后面的记号属于下一条语句，不能再接着当运算符读。
    fn ended_block(&self) -> bool {
        self.position > 0 && self.tokens[self.position - 1] == Token::Dedent
    }
    /// 块头末尾的冒号。
    fn colon(&mut self, header: &str) -> Result<(), String> {
        if self.symbol(':') {
            return Ok(());
        }
        if self.peek() == Some(&Token::Symbol('{')) {
            return Err(format!(
                "expected ':' at the end of the {header} line; blocks are indented, not wrapped in braces"
            ));
        }
        Err(format!(
            "expected ':' at the end of the {header} line, found {}",
            shown(self.peek())
        ))
    }
    /// 冒号之后换行、缩进进去的一块。
    fn indent(&mut self, header: &str) -> Result<(), String> {
        if self.peek() == Some(&Token::Indent) {
            self.take();
            return Ok(());
        }
        let mut message = format!("expected an indented block after the {header} line");
        if self.flat {
            message.push_str("; the indentation may have been lost when the code was copied");
        }
        Err(message)
    }
    /// 冒号之后：同一行写一条语句（`pass` 表示空），或者换行缩进写一块。
    fn suite(&mut self, imports: &[String], result: Type, header: &str) -> Result<Block, String> {
        self.colon(header)?;
        let mut body = Vec::new();
        if self.peek() != Some(&Token::Newline) {
            let position = self.at();
            if let Some(instruction) = self.statement(imports, result, Some(true))? {
                body.extend(self.pending.drain(..).map(|hoisted| (hoisted, position)));
                body.push((instruction, position));
            }
            return Ok(body);
        }
        self.take();
        self.indent(header)?;
        while !matches!(self.peek(), Some(Token::Dedent) | None) {
            let position = self.at();
            if let Some(instruction) = self.statement(imports, result, None)? {
                body.extend(self.pending.drain(..).map(|hoisted| (hoisted, position)));
                body.push((instruction, position));
            }
            self.separator()?;
        }
        self.take();
        Ok(body)
    }
    /// 值位置的块头之后：同一行一个值，或换行缩进的一行值。
    fn value_suite(&mut self, header: &str) -> Result<Expr, String> {
        self.colon(header)?;
        if self.peek() != Some(&Token::Newline) {
            return self.expr();
        }
        self.take();
        self.indent(header)?;
        let value = self.expr();
        let single = value.is_ok()
            && self.peek() == Some(&Token::Newline)
            && self.tokens.get(self.position + 1) == Some(&Token::Dedent);
        if !single {
            return Err(if header == "match arm" {
                "expected a single value: a match expression arm holds a single value; compute it with let before the match, or write the match as a statement".into()
            } else {
                "expected a single value: an if expression branch holds a single value; compute it with let before the if".into()
            });
        }
        self.take();
        self.take();
        value
    }
    fn symbol_dedent(&mut self) -> bool {
        if self.peek() == Some(&Token::Dedent) {
            self.take();
            true
        } else {
            false
        }
    }
    /// 单行分支后面的换行：下一行是 elif / else 时跳过它，接着读同一条 if。
    fn continues_with(&mut self, words: &[&str]) {
        if self.peek() == Some(&Token::Newline)
            && matches!(self.tokens.get(self.position + 1), Some(Token::Word(w)) if words.contains(&w.as_str()))
        {
            self.take();
        }
    }
    /// 类型名：`i64`、`Point` 或跨模块的 `model.Point`。
    fn type_name(&mut self) -> Result<String, String> {
        let mut name = self.word()?;
        while self.symbol('.') {
            name.push('.');
            name.push_str(&self.word()?);
        }
        Ok(name)
    }
    /// 首字母大写的类型名是结构体：记下名字，返回它的 id。
    fn record(&mut self, name: &str) -> Option<u64> {
        if self.type_params.iter().any(|param| param == name) {
            let id = type_param(name);
            self.type_names.insert(id, format!("$param:{name}"));
            return Some(id);
        }
        if !name
            .rsplit('.')
            .next()
            .is_some_and(|n| n.starts_with(char::is_uppercase))
        {
            return None;
        }
        let id = record_id(name);
        self.type_names.insert(id, name.to_owned());
        Some(id)
    }
    /// 列表元素与字典值：标量、字符串或结构体。
    /// 元素类型本身是列表或字典：`[[i64]]` 里的 `[i64]`、`{string: [string]}` 里的 `[string]`。
    fn inner_container(&mut self) -> Result<Option<Elem>, String> {
        // 函数类型的元素：`[fn(i64) -> i64]`、`{string: fn()}`。
        if self.peek() == Some(&Token::Word("fn".into())) {
            let Type::Func(id) = self.ty()? else {
                unreachable!("fn starts a function type")
            };
            return Ok(Some(Elem::Func(id)));
        }
        if !matches!(self.peek(), Some(Token::Symbol('[' | '{' | '('))) {
            return Ok(None);
        }
        let inner = self.ty()?;
        if let Type::Record(id) = inner {
            return Ok(Some(Elem::Record(id)));
        }
        if !matches!(inner, Type::List(_) | Type::Map(_, _)) {
            return Err(
                "unsupported element type: fixed arrays cannot be stored in lists or maps".into(),
            );
        }
        Ok(Some(nest(inner)))
    }
    /// 类型名后面的 `[T, U]`：泛型结构体或枚举的类型实参，如 `Option[i64]`、`Pair[string, i64]`。
    fn applied(&mut self, name: &str) -> Result<Option<u64>, String> {
        if self.peek() != Some(&Token::Symbol('['))
            || !name
                .rsplit('.')
                .next()
                .is_some_and(|n| n.starts_with(char::is_uppercase))
        {
            return Ok(None);
        }
        self.take();
        let mut args = Vec::new();
        loop {
            let arg = self.ty()?;
            if arg.is_reference() || matches!(arg, Type::Unit | Type::ArrayInt(_)) {
                return Err("type arguments are owned types, such as Option[string]".into());
            }
            args.push(arg);
            if self.symbol(']') {
                break;
            }
            self.expect(',')?;
        }
        Ok(Some(self.apply(name, args)))
    }
    /// 登记 `Name[args]`（名字还没按模块限定），链接时换成泛型实例。
    fn apply(&mut self, name: &str, args: Vec<Type>) -> u64 {
        let names: Vec<String> = args.iter().map(|ty| format!("{ty:?}")).collect();
        let text = format!("{name}[{}]", names.join(", "));
        let id = record_id(&format!("apply {text}"));
        self.type_names.insert(id, text);
        APPLICATIONS
            .write()
            .expect("type applications")
            .insert(id, (name.to_owned(), args));
        id
    }
    fn elem(&mut self, name: &str) -> Option<Elem> {
        Some(match name {
            "i64" => Elem::Int,
            "f64" => Elem::Float,
            "bool" => Elem::Bool,
            "string" => Elem::Str,
            _ => Elem::Record(self.record(name)?),
        })
    }
    fn ty(&mut self) -> Result<Type, String> {
        // 函数类型 `fn(i64, &string) -> bool`；没有 `->` 时没有返回值。
        if self.keyword("fn") {
            self.expect('(')?;
            let mut params = Vec::new();
            while !self.symbol(')') {
                let param = self.ty()?;
                if matches!(param, Type::ArrayInt(_)) {
                    return Err("arrays are only supported as local values".into());
                }
                params.push(param);
                if self.symbol(')') {
                    break;
                }
                self.expect(',')?;
            }
            let result = if self.symbol('-') {
                self.expect('>')?;
                // `-> !E`：没有成功值，只会失败。
                let result = if self.peek() == Some(&Token::Symbol('!')) {
                    Type::Unit
                } else {
                    self.ty()?
                };
                if result.is_reference() || matches!(result, Type::ArrayInt(_)) {
                    return Err("function types return owned values".into());
                }
                result
            } else {
                Type::Unit
            };
            let error = if self.symbol('!') {
                Some(self.ty()?)
            } else {
                None
            };
            return Ok(Type::Func(func_type_full(params, result, error)));
        }
        let ty = if self.symbol('&') {
            if self.keyword("mut") {
                Type::Mutable
            } else {
                Type::Shared
            }
        } else {
            Type::Owned
        };
        // 元组类型 `(i64, string)`。
        if self.symbol('(') {
            let mut fields = Vec::new();
            loop {
                let field = self.ty()?;
                if field.is_reference() || matches!(field, Type::ArrayInt(_)) {
                    return Err("tuples hold owned values, such as (string, i64)".into());
                }
                fields.push(field);
                if self.symbol(')') {
                    break;
                }
                self.expect(',')?;
            }
            if fields.len() < 2 {
                return Err("a tuple type lists at least two types, such as (i64, string)".into());
            }
            let (id, name) = tuple(fields);
            self.type_names.insert(id, name);
            return Ok(match ty {
                Type::Owned => Type::Record(id),
                Type::Shared => Type::RecordRef(id, false),
                _ => Type::RecordRef(id, true),
            });
        }
        if self.peek() == Some(&Token::Symbol('{')) {
            self.take();
            let key = match self.word()?.as_str() {
                "i64" => Elem::Int,
                "string" => Elem::Str,
                _ => return Err("unsupported map key type; expected i64 or string keys".into()),
            };
            self.expect(':')?;
            let value = match self.inner_container()? {
                Some(value) => value,
                None => {
                    let name = self.type_name()?;
                    match self.applied(&name)? {
                        Some(id) => Elem::Record(id),
                        None => self.elem(&name).ok_or(
                            "unsupported map value type; expected i64, f64, bool, string, a struct, a list or a map",
                        )?,
                    }
                }
            };
            self.expect('}')?;
            return Ok(match ty {
                Type::Owned => Type::Map(key, value),
                Type::Shared => Type::MapRef(key, value, false),
                _ => Type::MapRef(key, value, true),
            });
        }
        if self.peek() == Some(&Token::Symbol('[')) {
            self.take();
            if let Some(elem) = self.inner_container()? {
                self.expect(']')?;
                return Ok(match ty {
                    Type::Owned => Type::List(elem),
                    Type::Shared => Type::ListRef(elem, false),
                    _ => Type::ListRef(elem, true),
                });
            }
            let element = self.type_name()?;
            if !self.symbol(';') {
                let elem = match self.applied(&element)? {
                    Some(id) => Elem::Record(id),
                    None => self.elem(&element).ok_or(
                        "unsupported list element type; expected [i64], [f64], [bool], [string] or a struct such as [Point]",
                    )?,
                };
                self.expect(']')?;
                return Ok(match ty {
                    Type::Owned => Type::List(elem),
                    Type::Shared => Type::ListRef(elem, false),
                    _ => Type::ListRef(elem, true),
                });
            }
            if element != "i64" {
                return Err("only [i64; N] arrays are supported".into());
            }
            if ty != Type::Owned {
                return Err("arrays are only supported as local values".into());
            }
            let length = match self.take() {
                Some(Token::Number(number)) => {
                    number.parse().map_err(|_| "invalid array length")?
                }
                _ => return Err("array length must be an integer".into()),
            };
            if length == 0 || length > 1024 {
                return Err("array length must be between 1 and 1024".into());
            }
            self.expect(']')?;
            return Ok(Type::ArrayInt(length));
        }
        let name = self.type_name()?;
        if let Some(id) = self.applied(&name)? {
            return Ok(match ty {
                Type::Owned => Type::Record(id),
                _ => Type::RecordRef(id, ty == Type::Mutable),
            });
        }
        match name.as_str() {
            "string" => Ok(ty),
            "i64" if ty == Type::Owned => Ok(Type::Int),
            "f64" if ty == Type::Owned => Ok(Type::Float),
            "bool" if ty == Type::Owned => Ok(Type::Bool),
            _ => match (self.record(&name), ty) {
                (Some(id), Type::Owned) => Ok(Type::Record(id)),
                (Some(id), _) => Ok(Type::RecordRef(id, ty == Type::Mutable)),
                (None, _) => {
                    Err("unsupported type; expected string, i64, f64, bool or a struct".into())
                }
            },
        }
    }
    /// 点号后面是字段名：标识符，或元组的项号 `t.0`。
    fn member_follows(&self) -> bool {
        self.peek() == Some(&Token::Symbol('.'))
            && matches!(
                self.tokens.get(self.position + 1),
                Some(Token::Word(_) | Token::Number(_))
            )
    }
    fn member(&mut self) -> Result<String, String> {
        if let Some(Token::Number(number)) = self.peek().cloned() {
            self.take();
            return Ok(number);
        }
        self.word()
    }
    /// 元素后面的字段路径 `.a.b`；没有字段时为空串。`..` 不是字段访问。
    fn field_path(&mut self) -> Result<String, String> {
        let mut path = String::new();
        while self.member_follows() {
            self.take();
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(&self.member()?);
        }
        Ok(path)
    }
    fn expr(&mut self) -> Result<Expr, String> {
        let value = self.binary(0)?;
        // Python 的条件表达式 `a if c else b`，等于 `if c: a else: b`。
        if self.peek() == Some(&Token::Word("if".into())) && !self.ended_block() {
            self.take();
            let condition = self.binary(0)?;
            if !self.keyword("else") {
                return Err(format!(
                    "expected `else` in a if c else b, found {}",
                    shown(self.peek())
                ));
            }
            let otherwise = self.expr()?;
            return Ok(Expr::If(
                Box::new(condition),
                Box::new(value),
                Box::new(otherwise),
            ));
        }
        Ok(value)
    }
    /// 当前位置是不是复合赋值 `op=`：返回运算符和要跳过的记号数。
    fn compound(&self) -> Option<(String, usize)> {
        let at = |offset: usize| match self.tokens.get(self.position + offset) {
            Some(Token::Symbol(c)) => Some(*c),
            _ => None,
        };
        match (at(0)?, at(1), at(2)) {
            ('/', Some('/'), Some('=')) => Some(("//".to_owned(), 3)),
            (c @ ('+' | '-' | '*' | '/' | '%' | '&' | '|' | '^'), Some('='), _) => {
                Some((c.to_string(), 2))
            }
            (c @ ('<' | '>'), Some(d), Some('=')) if c == d => Some((format!("{c}{c}"), 3)),
            _ => None,
        }
    }
    /// 优先级与 Python 相同，从低到高：`or`、`and`、`not`、比较、`|`、`^`、`&`、移位、加减、乘除。
    /// `not` 比比较运算松：`not a == b` 是 `not (a == b)`，所以它不能出现在比较或算术运算的右边。
    fn binary(&mut self, minimum: u8) -> Result<Expr, String> {
        let mut left = if self.peek() == Some(&Token::Word("not".into())) {
            if minimum > 3 {
                return Err(
                    "expected an operand: `not` cannot follow a comparison or arithmetic operator; wrap it in parentheses, such as a == (not b)"
                        .into(),
                );
            }
            self.take();
            Expr::Unary('!', Box::new(self.binary(3)?))
        } else {
            self.atom()?
        };
        loop {
            if self.ended_block() {
                break;
            }
            // `x += 1`、`x <<= 2` 里的运算符是复合赋值，不是二元运算。
            if self.compound().is_some() {
                break;
            }
            let next = self.tokens.get(self.position + 1);
            let (op, priority, width) = match self.peek() {
                Some(Token::Word(word)) if word == "and" => ("&&".to_owned(), 2, 1),
                Some(Token::Word(word)) if word == "or" => ("||".to_owned(), 1, 1),
                Some(Token::Symbol(c @ ('&' | '|'))) if next == Some(&Token::Symbol(*c)) => {
                    let word = if *c == '&' { "and" } else { "or" };
                    return Err(format!("expected `{word}` instead of `{c}{c}`"));
                }
                Some(Token::Symbol('|')) => ("|".to_owned(), 4, 1),
                Some(Token::Symbol('^')) => ("^".to_owned(), 5, 1),
                Some(Token::Symbol('&')) => ("&".to_owned(), 6, 1),
                Some(Token::Symbol(c @ ('<' | '>'))) if next == Some(&Token::Symbol(*c)) => {
                    (format!("{c}{c}"), 7, 2)
                }
                Some(Token::Symbol(c @ ('=' | '!' | '<' | '>'))) => {
                    let equal = self.tokens.get(self.position + 1) == Some(&Token::Symbol('='));
                    if matches!(c, '=' | '!') && !equal {
                        break;
                    }
                    (
                        format!("{c}{}", if equal { "=" } else { "" }),
                        3,
                        if equal { 2 } else { 1 },
                    )
                }
                Some(Token::Word(word)) if word == "in" => ("in".to_owned(), 3, 1),
                Some(Token::Word(word))
                    if word == "not" && next == Some(&Token::Word("in".into())) =>
                {
                    ("not in".to_owned(), 3, 2)
                }
                Some(Token::Symbol(c @ ('+' | '-'))) => (c.to_string(), 8, 1),
                Some(Token::Symbol('*')) if next == Some(&Token::Symbol('*')) => {
                    ("**".to_owned(), 10, 2)
                }
                Some(Token::Symbol('/')) if next == Some(&Token::Symbol('/')) => {
                    ("//".to_owned(), 9, 2)
                }
                Some(Token::Symbol(c @ ('*' | '/' | '%'))) => (c.to_string(), 9, 1),
                _ => break,
            };
            if priority < minimum {
                break;
            }
            self.position += width;
            if op == "**" {
                // 右结合：`2 ** 3 ** 2` 是 `2 ** (3 ** 2)`；整数与浮点按第一个操作数选内建版本。
                let right = self.binary(priority)?;
                left = Expr::Call("$pow".into(), vec![left, right]);
                continue;
            }
            let right = self.binary(priority + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    /// 单目运算的操作数后面紧跟 `**` 时，先算乘方（右边照常右结合）。
    fn power_tail(&mut self, base: Expr) -> Result<Expr, String> {
        if self.peek() == Some(&Token::Symbol('*'))
            && self.tokens.get(self.position + 1) == Some(&Token::Symbol('*'))
        {
            self.position += 2;
            let exponent = self.binary(10)?;
            return Ok(Expr::Call("$pow".into(), vec![base, exponent]));
        }
        Ok(base)
    }
    fn atom(&mut self) -> Result<Expr, String> {
        // `fn(x: i64) -> i64: x * 2`、`fn(a, b): a < b`：匿名函数，冒号后是一个表达式。
        if self.keyword("fn") {
            self.expect('(')?;
            let mut params = Vec::new();
            while !self.symbol(')') {
                let name = self.word()?;
                let ty = if self.symbol(':') {
                    let ty = self.ty()?;
                    if matches!(ty, Type::ArrayInt(_)) {
                        return Err("arrays are only supported as local values".into());
                    }
                    Some(match ty {
                        Type::Func(id) => Type::FuncRef(id),
                        ty => ty,
                    })
                } else {
                    None
                };
                if params.iter().any(|(other, _)| *other == name) {
                    return Err(format!("duplicate parameter {name}"));
                }
                params.push((name, ty));
                if self.symbol(')') {
                    break;
                }
                self.expect(',')?;
            }
            let result = if self.symbol('-') {
                self.expect('>')?;
                let result = self.ty()?;
                if result.is_reference() || matches!(result, Type::ArrayInt(_)) {
                    return Err("function types return owned values".into());
                }
                Some(result)
            } else {
                None
            };
            if !self.symbol(':') {
                return Err(
                    "expected ':' and an expression after the parameters of fn(...)".into(),
                );
            }
            if matches!(self.peek(), Some(Token::Newline | Token::Indent)) {
                return Err("an anonymous fn holds one expression on the same line; declare a named fn for longer code".into());
            }
            let position = self.at();
            // `fn(s): fail "..."`：函数体是一条 fail（函数类型要带错误类型）。
            if self.keyword("fail") {
                let error = self.expr()?;
                return Ok(Expr::Lambda(
                    params,
                    result,
                    vec![(Instruction::Fail(error), position)],
                ));
            }
            let body = self.expr()?;
            let statement = match body {
                Expr::Call(name, mut args)
                    if args.len() == 1
                        && matches!(name.as_str(), "println" | "io.println" | "fmt.println") =>
                {
                    Instruction::Print(args.remove(0))
                }
                body => Instruction::Call(body),
            };
            return Ok(Expr::Lambda(params, result, vec![(statement, position)]));
        }
        if self.symbol('(') {
            let expr = self.expr()?;
            // `(a, b)` 是元组；`(a)` 只是括号。
            if self.symbol(',') {
                let mut items = vec![expr];
                while !self.symbol(')') {
                    items.push(self.expr()?);
                    if self.symbol(')') {
                        break;
                    }
                    self.expect(',')?;
                }
                return Ok(Expr::Tuple(items));
            }
            self.expect(')')?;
            return Ok(expr);
        }
        // 与 Python 相同，`**` 比单目 `-`、`~` 结合得紧：`-2 ** 2` 是 `-(2 ** 2)`。
        if self.symbol('-') {
            let power_follows = |p: &Self| {
                p.tokens.get(p.position + 1) == Some(&Token::Symbol('*'))
                    && p.tokens.get(p.position + 2) == Some(&Token::Symbol('*'))
            };
            if let Some(Token::Number(number)) = self.peek().cloned() {
                if !power_follows(self) {
                    self.take();
                    return Ok(Expr::Int(
                        format!("-{number}")
                            .parse()
                            .map_err(|_| "i64 literal overflow")?,
                    ));
                }
            }
            if let Some(Token::Float(text)) = self.peek().cloned() {
                if !power_follows(self) {
                    self.take();
                    return Ok(Expr::Float(-text.parse::<f64>().expect("lexed float")));
                }
            }
            let operand = self.atom()?;
            let operand = self.power_tail(operand)?;
            return Ok(Expr::Unary('-', Box::new(operand)));
        }
        if self.symbol('~') {
            let operand = self.atom()?;
            let operand = self.power_tail(operand)?;
            return Ok(Expr::Unary('~', Box::new(operand)));
        }
        if self.peek() == Some(&Token::Symbol('!')) {
            return Err(
                "expected `not` instead of `!`; `!` only marks error types such as i64!string"
                    .into(),
            );
        }
        if let Some(Token::Number(number)) = self.peek().cloned() {
            self.take();
            return Ok(Expr::Int(
                number.parse().map_err(|_| "i64 literal overflow")?,
            ));
        }
        if let Some(Token::Float(text)) = self.peek().cloned() {
            self.take();
            return Ok(Expr::Float(text.parse().expect("lexed float")));
        }
        if self.keyword("if") {
            return self.if_value();
        }
        if self.keyword("match") {
            return self.match_value();
        }
        // Python 的 `True`、`False` 也可以。
        if self.keyword("true") || self.keyword("True") {
            return Ok(Expr::Bool(true));
        }
        if self.keyword("false") || self.keyword("False") {
            return Ok(Expr::Bool(false));
        }
        // Python 的 f 串：Zamak 的字符串本来就能插值，`f"{n}"` 与 `"{n}"` 相同。
        if self.peek() == Some(&Token::Word("f".into()))
            && matches!(self.tokens.get(self.position + 1), Some(Token::Text(_)))
        {
            self.take();
        }
        if let Some(Token::Text(text)) = self.peek().cloned() {
            self.take();
            let value = self.literal(&text)?;
            // `"a,b".split(",")`、`", ".join(words)`：字面量上调用字符串方法。
            if self.member_follows() {
                self.take();
                let method = self.member()?;
                if !self.symbol('(') {
                    return Err(format!(
                        "expected ( after .{method}; a string has methods such as \"a,b\".split(\",\")"
                    ));
                }
                let args = self.arguments()?;
                return self.after_call(Expr::Chain(Box::new(value), method, args));
            }
            return Ok(value);
        }
        if self.symbol('{') {
            // 字典字面量：`{k: v, k2: v2}`，条目之间用逗号，可以分行写。
            let mut entries = Vec::new();
            self.newlines();
            while !self.symbol('}') {
                let key = self.expr()?;
                self.expect(':')?;
                let value = self.expr()?;
                entries.push((key, value));
                self.newlines();
                if self.symbol('}') {
                    break;
                }
                self.expect(',')?;
                self.newlines();
            }
            return Ok(Expr::MapLit(entries));
        }
        if self.symbol('[') {
            let mut values = Vec::new();
            // 与 Python 相同，最后一项后面可以多写一个逗号：`[a, b,]`。
            while !self.symbol(']') {
                values.push(self.expr()?);
                if self.symbol(']') {
                    break;
                }
                self.expect(',')?;
            }
            return Ok(Expr::Array(values));
        }
        if self.symbol('&') {
            let mutable = self.keyword("mut");
            let start = self.position;
            let mut name = self.word()?;
            while self.symbol('.') {
                name.push('.');
                name.push_str(&self.word()?);
            }
            // `&env.arg(0)?`、`&make()`：借调用结果就是把临时值传过去，与不写 & 相同。
            if self.peek() == Some(&Token::Symbol('(')) {
                if mutable {
                    return Err(
                        "cannot mutably borrow a call result; store it with let mut first".into(),
                    );
                }
                self.position = start;
                return self.atom();
            }
            if self.symbol('[') {
                let index = self.expr()?;
                self.expect(']')?;
                let field = self.field_path()?;
                if self.peek() == Some(&Token::Symbol('(')) {
                    return Err(
                        "expected a field after &xs[..]; references cannot point at call results"
                            .into(),
                    );
                }
                return Ok(Expr::BorrowIndex(name, Box::new(index), field, mutable));
            }
            return Ok(Expr::Borrow(name, mutable));
        }
        let mut name = self.word()?;
        // 只有点号后面跟标识符才是字段路径；`a..b` 里的 `..` 留给 for 区间。
        while self.member_follows() {
            self.take();
            name.push('.');
            name.push_str(&self.member()?);
        }
        if !self.symbol('(') {
            if self.symbol('[') {
                // Python 的切片 `xs[a:b]`、`xs[:b]`、`s[a:]`：等于 `xs.slice(a, b)`，复制出一段。
                let from = if self.peek() == Some(&Token::Symbol(':')) {
                    None
                } else {
                    Some(self.expr()?)
                };
                if self.symbol(':') {
                    let to = if self.peek() == Some(&Token::Symbol(']')) {
                        None
                    } else {
                        Some(self.expr()?)
                    };
                    self.expect(']')?;
                    let bounds = vec![
                        from.unwrap_or(Expr::Int(0)),
                        to.unwrap_or(Expr::Int(i64::MAX)),
                    ];
                    return self.after_call(Expr::Method(name, "slice".into(), bounds));
                }
                let index = from.expect("index expression");
                self.expect(']')?;
                // `grid[i][j]`：嵌套容器的第二层下标，读写由内层容器的 `$at`/`$put` 完成。
                if self.symbol('[') {
                    let inner = self.expr()?;
                    self.expect(']')?;
                    if matches!(self.peek(), Some(Token::Symbol('[' | '.'))) {
                        return Err("index at most two levels deep, such as grid[i][j]; borrow a row with for row in &grid".into());
                    }
                    return Ok(Expr::ElementMethod(
                        name,
                        Box::new(index),
                        String::new(),
                        "$at".into(),
                        vec![inner],
                    ));
                }
                let field = self.field_path()?;
                if !field.is_empty() && self.symbol('(') {
                    let (field, method) = field.rsplit_once('.').unwrap_or(("", &field));
                    let args = self.arguments()?;
                    return Ok(Expr::ElementMethod(
                        name,
                        Box::new(index),
                        field.to_owned(),
                        method.to_owned(),
                        args,
                    ));
                }
                if !field.is_empty() {
                    return Ok(Expr::IndexField(name, Box::new(index), field));
                }
                let element = Expr::Index(name, Box::new(index));
                // `steps[0](x)`：元素是函数时直接调用。
                if self.symbol('(') {
                    let args = self.arguments()?;
                    return self.after_call(Expr::Invoke(Box::new(element), args));
                }
                return Ok(element);
            }
            if let Some((local, field)) = name.split_once('.') {
                return Ok(Expr::Field(local.into(), field.into()));
            }
            return Ok(Expr::Variable(name));
        }
        // 首字母大写的名字后面跟括号：`Point(x: 1, y: 2)` 是结构体字面量，
        // `Shape.Circle(1.0)` 是枚举变体（按位置传值）。
        if name
            .rsplit('.')
            .next()
            .is_some_and(|n| n.starts_with(char::is_uppercase))
        {
            let named = matches!(self.peek(), Some(Token::Word(_)))
                && self.tokens.get(self.position + 1) == Some(&Token::Symbol(':'));
            if named {
                let mut fields = Vec::new();
                loop {
                    let field = self.word()?;
                    self.expect(':')?;
                    fields.push((field, self.expr()?));
                    if self.symbol(')') {
                        break;
                    }
                    self.expect(',')?;
                    if self.symbol(')') {
                        break;
                    }
                }
                return Ok(Expr::Record(name, fields));
            }
            let args = self.arguments()?;
            return match name.rsplit_once('.') {
                Some((enumeration, variant)) => {
                    Ok(Expr::Variant(enumeration.into(), variant.into(), args))
                }
                None if args.is_empty() => Ok(Expr::Record(name, Vec::new())),
                None => Err(format!(
                    "expected field names in the struct literal, such as {name}(x: 1, y: 2)"
                )),
            };
        }
        let args = self.arguments()?;
        self.after_call(Expr::Call(name, args))
    }
    /// 调用后面可以接 `?`、`catch e:`，或在结果上接着调用方法。
    fn after_call(&mut self, mut call: Expr) -> Result<Expr, String> {
        // `?` 只作用于调用；其他位置的 `?` 不是合法语法。
        if self.symbol('?') {
            call = Expr::Try(Box::new(call));
        } else if self.keyword("catch") {
            // `call catch e: ...` 就地处理错误，不传播。
            let binding = self.word()?;
            let imports = self.imports.clone();
            let mut block = self.suite(&imports, self.result, "catch")?;
            // 块尾的裸表达式不是函数的隐式返回：`f() catch e: 0` 里的 0 是出错时的值。
            if let Some((Instruction::Return(Some(value)), position)) = block.last() {
                if !self.explicit.contains(position) {
                    let value = value.clone();
                    let last = block.len() - 1;
                    block[last].0 = Instruction::Call(value);
                }
            }
            return Ok(Expr::Catch(Box::new(call), binding, block));
        }
        // 调用结果上接着调用方法：`string.split(&s, ",").len()`。
        while self.peek() == Some(&Token::Symbol('.'))
            && matches!(self.tokens.get(self.position + 1), Some(Token::Word(_)))
            && self.tokens.get(self.position + 2) == Some(&Token::Symbol('('))
        {
            self.take();
            let method = self.word()?;
            self.expect('(')?;
            let args = self.arguments()?;
            call = Expr::Chain(Box::new(call), method, args);
        }
        Ok(call)
    }
    /// `fn` 之后的函数声明：名字、参数、返回类型与函数体。`owner` 是方法所在的结构体或枚举，
    /// 这时第一个参数可以写 `self`（移入）、`&self` 或 `&mut self`。
    fn function(
        &mut self,
        public: bool,
        start: SourcePosition,
        imports: &[String],
        owner: Option<&str>,
    ) -> Result<(String, Function), String> {
        let name = self.word()?;
        // 首字母大写的名字留给结构体与枚举：`Point(x: 1)` 是结构体字面量，不是调用。
        if name.starts_with(char::is_uppercase) {
            return Err(format!(
                "function names must start with a lowercase letter: {name}; uppercase names are for structs and enums"
            ));
        }
        // `i64(x)`、`f64(n)` 是内建的数值转换。
        if matches!(name.as_str(), "i64" | "f64") {
            return Err(format!("reserved type name {name}"));
        }
        // `fn largest[T](xs: &[T]) -> T`：类型参数首字母大写，函数体里也能用。
        // 泛型结构体或枚举里的方法还带上外层的类型参数（`Stack[T]` 的 `push` 也是泛型函数）。
        let mut generics = self.type_params.clone();
        for generic in self.generics()? {
            if generics.contains(&generic) {
                return Err(format!("duplicate type parameter {generic}"));
            }
            generics.push(generic);
        }
        let outer = std::mem::replace(&mut self.type_params, generics.clone());
        let parsed = self.function_rest(public, start, imports, owner, name, generics);
        self.type_params = outer;
        parsed
    }
    /// 函数名（与类型参数）之后：参数表、返回类型与函数体。
    fn function_rest(
        &mut self,
        public: bool,
        start: SourcePosition,
        imports: &[String],
        owner: Option<&str>,
        name: String,
        generics: Vec<String>,
    ) -> Result<(String, Function), String> {
        let (params, result, error) = self.signature(owner)?;
        self.imports = imports.to_vec();
        self.result = result;
        let mut body = self.suite(imports, result, "fn")?;
        untail(&mut body, true, &self.implicit);
        Ok((
            name,
            Function {
                public,
                params,
                result,
                error,
                body,
                position: start,
                builtin: false,
                generics,
                external: false,
                wrapping: false,
            },
        ))
    }
    /// `extern fn name(...) -> T`：只有签名，没有函数体。
    fn external(
        &mut self,
        public: bool,
        start: SourcePosition,
    ) -> Result<(String, Function), String> {
        if !self.keyword("fn") {
            return Err("expected fn after extern, such as extern fn sqrt(x: f64) -> f64".into());
        }
        let name = self.word()?;
        if name.starts_with(char::is_uppercase) {
            return Err(format!(
                "function names must start with a lowercase letter: {name}; wrap the C function in a small C function with a lowercase name"
            ));
        }
        if self.peek() == Some(&Token::Symbol('[')) {
            return Err(format!("extern fn {name} cannot have type parameters"));
        }
        let (params, result, error) = self.signature(None)?;
        if error.is_some() {
            return Err(format!(
                "extern fn {name} cannot declare an error type; return a status value and check it in Zamak"
            ));
        }
        extern_signature(&name, &params, result)?;
        if self.symbol(':') {
            return Err(format!(
                "extern fn {name} has no body; its code is the C function {name}"
            ));
        }
        Ok((
            name,
            Function {
                public,
                params,
                result,
                error: None,
                body: Vec::new(),
                position: start,
                builtin: false,
                generics: Vec::new(),
                external: true,
                wrapping: false,
            },
        ))
    }
    /// 参数列表与返回类型（`(a: i64) -> T!E`）。
    #[allow(clippy::type_complexity)]
    fn signature(
        &mut self,
        owner: Option<&str>,
    ) -> Result<(Vec<(String, Type)>, Type, Option<Type>), String> {
        self.expect('(')?;
        let mut params = Vec::new();
        if !self.symbol(')') {
            loop {
                let receiver = match owner {
                    Some(owner) if params.is_empty() => self.self_param(owner)?,
                    _ => None,
                };
                if let Some(ty) = receiver {
                    params.push(("self".to_owned(), ty));
                } else {
                    let param = self.word()?;
                    if param == "self" {
                        return Err("self can only be the first parameter of a method declared inside a struct or enum".into());
                    }
                    self.expect(':')?;
                    let ty = match self.ty()? {
                        // 参数位置的函数类型可以接收闭包，按借用对待。
                        Type::Func(id) => Type::FuncRef(id),
                        ty => ty,
                    };
                    if matches!(ty, Type::ArrayInt(_)) {
                        return Err("arrays are only supported as local values".into());
                    }
                    params.push((param, ty));
                }
                if self.symbol(')') {
                    break;
                }
                self.expect(',')?;
            }
        }
        let (result, error) = if self.symbol('-') {
            self.expect('>')?;
            // `-> !E` 表示没有成功值、只有错误值。
            if self.symbol('!') {
                let error = self.ty()?;
                (Type::Unit, Some(error))
            } else {
                let ty = self.ty()?;
                if ty.is_reference() {
                    return Err("borrow escape: functions cannot return references".into());
                }
                if matches!(ty, Type::ArrayInt(_)) {
                    return Err("arrays are only supported as local values".into());
                }
                let error = if self.symbol('!') {
                    Some(self.ty()?)
                } else {
                    None
                };
                (ty, error)
            }
        } else {
            (Type::Unit, None)
        };
        Ok((params, result, error))
    }
    /// 名字后面的 `[T, U]`：类型参数列表，没有时为空。
    fn generics(&mut self) -> Result<Vec<String>, String> {
        let mut generics = Vec::new();
        if self.symbol('[') {
            loop {
                let generic = self.word()?;
                if !generic.starts_with(char::is_uppercase) {
                    return Err(format!(
                        "type parameter names start with an uppercase letter, such as T; found {generic}"
                    ));
                }
                if generics.contains(&generic) {
                    return Err(format!("duplicate type parameter {generic}"));
                }
                generics.push(generic);
                if self.symbol(']') {
                    break;
                }
                self.expect(',')?;
            }
        }
        Ok(generics)
    }
    /// 方法的第一个参数 `self` / `&self` / `&mut self`；不是时退回原位置。
    fn self_param(&mut self, owner: &str) -> Result<Option<Type>, String> {
        let at = self.position;
        let borrowed = self.symbol('&');
        let mutable = borrowed && self.keyword("mut");
        if self.keyword("self") {
            // 泛型结构体的方法：self 是 `Stack[T]`。
            let id = if self.owner_generics.is_empty() {
                self.record(owner)
                    .expect("struct and enum names are uppercase")
            } else {
                let args = self
                    .owner_generics
                    .clone()
                    .iter()
                    .map(|g| Type::Record(self.record(g).expect("type parameter")))
                    .collect();
                self.apply(owner, args)
            };
            return Ok(Some(if borrowed {
                Type::RecordRef(id, mutable)
            } else {
                Type::Record(id)
            }));
        }
        self.position = at;
        Ok(None)
    }
    /// 结构体或枚举块里的下一行是方法声明。
    fn method_start(&self) -> bool {
        let word = |offset: usize, text: &str| {
            self.tokens.get(self.position + offset) == Some(&Token::Word(text.into()))
        };
        word(0, "fn") || (word(0, "pub") && word(1, "fn"))
    }
    /// 结构体或枚举里的方法：在函数表里叫 `Owner.name`。
    fn method(&mut self, owner: &str, imports: &[String]) -> Result<(String, Function), String> {
        let start = self.at();
        let public = self.keyword("pub");
        self.keyword("fn");
        let (name, function) = self.function(public, start, imports, Some(owner))?;
        Ok((format!("{owner}.{name}"), function))
    }
    /// 左括号之后的实参列表，读到右括号为止。
    fn arguments(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        while !self.symbol(')') {
            args.push(self.expr()?);
            if self.symbol(')') {
                break;
            }
            self.expect(',')?;
        }
        Ok(args)
    }
    /// `if` 出现在表达式位置：`if c: a elif d: b else: e`，分支各是一个值。
    fn if_value(&mut self) -> Result<Expr, String> {
        let condition = self.condition("if")?;
        let yes = self.value_suite("if")?;
        self.continues_with(&["elif", "else"]);
        let no = if self.keyword("elif") {
            self.if_value()?
        } else if self.keyword("else") {
            if self.peek() == Some(&Token::Word("if".into())) {
                return Err("expected `elif` instead of `else if`".into());
            }
            self.value_suite("else")?
        } else {
            return Err("expected else: an if expression needs an else branch".into());
        };
        Ok(Expr::If(Box::new(condition), Box::new(yes), Box::new(no)))
    }
    /// 插值里的 `{...}`：变量或字段路径照旧；其他内容按表达式解析（不能含字符串字面量）。
    fn literal(&mut self, text: &str) -> Result<Expr, String> {
        let Expr::Format(segments) = literal(text)? else {
            return literal(text);
        };
        let mut parsed = Vec::with_capacity(segments.len());
        for segment in segments {
            parsed.push(match segment {
                Expr::Call(source, _) => {
                    let (tokens, lines, _) = lex(&source)
                        .map_err(|error| format!("in interpolation {{{source}}}: {error}"))?;
                    let count = tokens
                        .iter()
                        .position(|token| *token == Token::Newline)
                        .unwrap_or(tokens.len());
                    let mut inner = Parser {
                        tokens,
                        lines,
                        position: 0,
                        type_names: BTreeMap::new(),
                        imports: self.imports.clone(),
                        result: self.result,
                        implicit: BTreeSet::new(),
                        explicit: BTreeSet::new(),
                        flat: false,
                        pending: Vec::new(),
                        temps: 0,
                        type_params: self.type_params.clone(),
                        owner_generics: Vec::new(),
                        text: Vec::new(),
                    };
                    let expr = inner
                        .expr()
                        .map_err(|error| format!("in interpolation {{{source}}}: {error}"))?;
                    if inner.position != count {
                        return Err(format!(
                            "unsupported interpolation {{{source}}}: expected one expression"
                        ));
                    }
                    self.type_names.extend(inner.type_names);
                    expr
                }
                other => other,
            });
        }
        Ok(Expr::Format(parsed))
    }
    /// `if`/`while` 的条件。新手常把比较写成 `if score = 60`，在这里直接指出要用 `==`。
    fn condition(&mut self, keyword: &str) -> Result<Expr, String> {
        let condition = self.expr()?;
        if self.peek() == Some(&Token::Symbol('=')) {
            return Err(format!(
                "expected `==` to compare in the {keyword} condition; a single `=` assigns a value"
            ));
        }
        Ok(condition)
    }
    fn if_statement(&mut self, imports: &[String], result: Type) -> Result<Instruction, String> {
        let condition = self.condition("if")?;
        let yes = self.suite(imports, result, "if")?;
        self.continues_with(&["elif", "else"]);
        let position = self.at();
        let no = if self.keyword("elif") {
            vec![(self.if_statement(imports, result)?, position)]
        } else if self.keyword("else") {
            if self.peek() == Some(&Token::Word("if".into())) {
                return Err("expected `elif` instead of `else if`".into());
            }
            self.suite(imports, result, "else")?
        } else {
            Vec::new()
        };
        Ok(Instruction::If(condition, yes, no))
    }
    /// 后面（跳过换行）就是块的结尾：当前语句是代码块的最后一条。
    fn at_end(&self) -> bool {
        matches!(
            self.tokens[self.position..]
                .iter()
                .find(|token| **token != Token::Newline),
            None | Some(Token::Dedent)
        )
    }
    /// 一条语句；`pass` 没有指令。`last` 指定它是不是所在代码块的最后一条（写在块头同一行的
    /// 语句总是）；为 `None` 时看后面是不是块的结尾。
    fn statement(
        &mut self,
        imports: &[String],
        result: Type,
        last: Option<bool>,
    ) -> Result<Option<Instruction>, String> {
        let position = self.at();
        if self.keyword("pass") {
            return Ok(None);
        }
        match self.peek() {
            Some(Token::Indent) => {
                return Err(
                    "unexpected indent; indent only the lines inside a block that ends with ':'"
                        .into(),
                )
            }
            Some(Token::Word(word)) if word == "elif" || word == "else" => {
                return Err(format!(
                    "unexpected {word}: it must follow an if block at the same indentation"
                ))
            }
            _ => {}
        }
        Ok(Some(if self.keyword("if") {
            self.if_statement(imports, result)?
        } else if self.keyword("match") {
            self.match_statement(imports, result)?
        } else if self.keyword("while") {
            let condition = self.condition("while")?;
            let nested = self.suite(imports, result, "while")?;
            Instruction::While(condition, nested)
        } else if self.keyword("for") {
            let local = self.word()?;
            let second = if self.symbol(',') {
                Some(self.word()?)
            } else {
                None
            };
            if !self.keyword("in") {
                return Err("expected `in` after the for loop variable".into());
            }
            // `for x in xs:` 与 `for x in &xs:` 相同：只借用，不移走（Python 的写法）。
            let bare = {
                let mut at = self.position;
                let mut path = matches!(self.tokens.get(at), Some(Token::Word(_)));
                at += 1;
                while path && self.tokens.get(at) == Some(&Token::Symbol('.')) {
                    path = matches!(self.tokens.get(at + 1), Some(Token::Word(_)));
                    at += 2;
                }
                path && self.tokens.get(at) == Some(&Token::Symbol(':'))
            };
            if bare || self.symbol('&') {
                let mutable = !bare && self.keyword("mut");
                let mut path = self.word()?;
                while self.symbol('.') {
                    path.push('.');
                    path.push_str(&self.word()?);
                }
                let nested = self.suite(imports, result, "for")?;
                Instruction::Each(local, second, path, mutable, nested)
            } else {
                let start = self.expr()?;
                // Python 的 `for i in range(n)`、`range(a, b)`：就是区间循环。
                if let Expr::Call(name, args) = &start {
                    if name == "range" && self.peek() == Some(&Token::Symbol(':')) {
                        let (from, to) = match args.as_slice() {
                            [to] => (Expr::Int(0), to.clone()),
                            [from, to] => (from.clone(), to.clone()),
                            _ => {
                                return Err(
                                    "range takes one or two arguments, such as range(n) or range(a, b); for a step, use while"
                                        .into(),
                                )
                            }
                        };
                        if second.is_some() {
                            return Err("a range has one loop variable: for i in range(n)".into());
                        }
                        let nested = self.suite(imports, result, "for")?;
                        return Ok(Some(Instruction::For(local, from, to, nested)));
                    }
                }
                // `for name in fs.list(dir)?:`：遍历一个值（调用结果、字面量）。先把它存进隐藏变量，
                // 循环借用那个变量；值在循环前求一次。
                if self.peek() == Some(&Token::Symbol(':')) {
                    let hidden = format!("$for{}", self.temps);
                    self.temps += 1;
                    let nested = self.suite(imports, result, "for")?;
                    // 循环体先解析完再登记，免得被循环体里的第一条语句拿走。
                    self.pending
                        .push(Instruction::Let(hidden.clone(), false, None, start));
                    return Ok(Some(Instruction::Each(
                        local, second, hidden, false, nested,
                    )));
                }
                if second.is_some() {
                    return Err(
                        "two loop variables need a list or map: for i, x in items or for k, v in m"
                            .into(),
                    );
                }
                if !(self.symbol('.') && self.symbol('.')) {
                    return Err("expected `..` in for range: for name in start..end, or iterate a list with for x in items".into());
                }
                let end = self.expr()?;
                let nested = self.suite(imports, result, "for")?;
                Instruction::For(local, start, end, nested)
            }
        } else if self.peek() == Some(&Token::Word("fn".into()))
            && matches!(self.tokens.get(self.position + 1), Some(Token::Word(_)))
        {
            // 函数里定义的函数：可以读写外层的局部变量（闭包），只在这个函数里用。
            self.take();
            let outer_result = self.result;
            let outer_generics = self.type_params.len();
            let (name, function) = self.function(false, position, imports, None)?;
            self.result = outer_result;
            if function.generics.len() > outer_generics {
                return Err(format!(
                    "a fn declared inside a function cannot have type parameters; declare {name} at the top level"
                ));
            }
            // 会失败的函数：完整的函数类型（带错误类型）记在 let 的标注上，函数体按它检查。
            let annotation = function.error.map(|error| {
                Type::Func(func_type_full(
                    function.params.iter().map(|(_, ty)| *ty).collect(),
                    function.result,
                    Some(error),
                ))
            });
            let params = function
                .params
                .into_iter()
                .map(|(param, ty)| (param, Some(ty)))
                .collect();
            Instruction::Let(
                name,
                false,
                annotation,
                Expr::Lambda(params, Some(function.result), function.body),
            )
        } else if self.keyword("break") {
            Instruction::Break
        } else if self.keyword("continue") {
            Instruction::Continue
        } else if self.keyword("let") {
            let mutable = self.keyword("mut");
            if self.symbol('(') {
                let mut names = Vec::new();
                loop {
                    names.push(self.word()?);
                    if self.symbol(')') {
                        break;
                    }
                    self.expect(',')?;
                }
                self.expect('=')?;
                return Ok(Some(Instruction::Unpack(names, mutable, self.expr()?)));
            }
            let local = self.word()?;
            let annotation = if self.symbol(':') {
                Some(self.ty()?)
            } else {
                None
            };
            if annotation.is_some_and(Type::is_reference) {
                return Err(format!(
                    "borrow escape: a local cannot be declared with a reference type; borrow without a type, such as let {local} = &value"
                ));
            }
            self.expect('=')?;
            Instruction::Let(local, mutable, annotation, self.expr()?)
        } else if self.keyword("return") {
            let expr = if matches!(
                self.peek(),
                None | Some(Token::Newline | Token::Dedent | Token::Symbol(')' | ']' | ','))
            ) {
                None
            } else {
                Some(self.expr()?)
            };
            self.explicit.insert(position);
            Instruction::Return(expr)
        } else if self.keyword("fail") {
            // `fail e` 在声明了错误类型的函数里返回错误值。
            Instruction::Fail(self.expr()?)
        } else if self.keyword("assert") {
            // `assert 条件` / `assert 条件, 说明`：展开成 `if not 条件: 报错并退出`。报错带条件原文；
            // `a == b` 两边是变量、字段、元素或字面量时再显示两边的值（只在失败时求第二次）。
            let (line, _) = self.lines[self.position - 1];
            let start = self.position;
            let condition = self.expr()?;
            let text = self.snippet(start);
            let mut parts = vec![Expr::Text(format!(":{line}: assertion failed: {text}"))];
            if self.symbol(',') {
                parts.push(Expr::Text(": ".into()));
                parts.push(self.expr()?);
            }
            if let Expr::Binary(op, left, right) = &condition {
                let plain = |expr: &Expr| {
                    matches!(
                        expr,
                        Expr::Variable(_)
                            | Expr::Field(_, _)
                            | Expr::Index(_, _)
                            | Expr::IndexField(_, _, _)
                            | Expr::Int(_)
                            | Expr::Float(_)
                            | Expr::Bool(_)
                            | Expr::Text(_)
                    )
                };
                if matches!(op.as_str(), "==" | "!=") && plain(left) && plain(right) {
                    parts.push(Expr::Text("\n  left:  ".into()));
                    parts.push((**left).clone());
                    parts.push(Expr::Text("\n  right: ".into()));
                    parts.push((**right).clone());
                }
            }
            let at = self.lines[start];
            Instruction::If(
                Expr::Unary('!', Box::new(condition)),
                vec![(
                    Instruction::Call(Expr::Call("$assert".into(), vec![Expr::Format(parts)])),
                    at,
                )],
                Vec::new(),
            )
        } else {
            let expr = self.expr()?;
            if let Some((op, width)) = self.compound() {
                {
                    // `x op= e` 等价于 `x = x op e`；下标里不能有调用，保证只求值一次的语义。
                    self.position += width;
                    let value = self.expr()?;
                    let combined =
                        Expr::Binary(op.clone(), Box::new(expr.clone()), Box::new(value));
                    return Ok(Some(match expr {
                            Expr::Variable(name) => Instruction::Assign(name, combined),
                            Expr::Field(local, field) => {
                                Instruction::AssignField(local, field, combined)
                            }
                            Expr::Index(local, index) if pure(&index) => {
                                Instruction::AssignIndex(local, *index, combined)
                            }
                            Expr::IndexField(local, index, field) if pure(&index) => {
                                Instruction::AssignIndexField(local, *index, field, combined)
                            }
                            Expr::ElementMethod(local, index, _, method, mut args)
                                if method == "$at" && pure(&index) && args.iter().all(pure) =>
                            {
                                args.push(combined);
                                Instruction::Call(Expr::ElementMethod(
                                    local,
                                    index,
                                    String::new(),
                                    "$put".into(),
                                    args,
                                ))
                            }
                            Expr::Index(_, _) | Expr::IndexField(_, _, _) => return Err(
                                "unsupported compound assignment index: it must not call functions; bind it with let first"
                                    .into(),
                            ),
                            _ => {
                                return Err(format!(
                                    "expected a variable, field or index before {op}="
                                ))
                            }
                    }));
                }
            }
            if let Expr::Field(local, field) = &expr {
                if self.symbol('=') {
                    Instruction::AssignField(local.clone(), field.clone(), self.expr()?)
                } else {
                    Instruction::Return(Some(expr))
                }
            } else if let Expr::Index(local, index) = &expr {
                if self.symbol('=') {
                    Instruction::AssignIndex(local.clone(), (**index).clone(), self.expr()?)
                } else {
                    Instruction::Return(Some(expr))
                }
            } else if let (Expr::ElementMethod(local, index, _, method, args), true) = (
                &expr,
                self.peek() == Some(&Token::Symbol('='))
                    && self.tokens.get(self.position + 1) != Some(&Token::Symbol('=')),
            ) {
                if method != "$at" {
                    return Err("expected newline between statements, found Symbol('=')".into());
                }
                self.take();
                let mut args = args.clone();
                args.push(self.expr()?);
                Instruction::Call(Expr::ElementMethod(
                    local.clone(),
                    index.clone(),
                    String::new(),
                    "$put".into(),
                    args,
                ))
            } else if let Expr::IndexField(local, index, field) = &expr {
                if self.symbol('=') {
                    Instruction::AssignIndexField(
                        local.clone(),
                        (**index).clone(),
                        field.clone(),
                        self.expr()?,
                    )
                } else {
                    Instruction::Return(Some(expr))
                }
            } else if let Expr::Variable(name) = &expr {
                if self.symbol('=') {
                    Instruction::Assign(name.clone(), self.expr()?)
                } else {
                    Instruction::Return(Some(expr))
                }
            } else if let Expr::Call(callee, args) = &expr {
                if matches!(callee.as_str(), "println" | "io.println" | "fmt.println") {
                    if let Some(namespace) = callee.strip_suffix(".println") {
                        if !imports.contains(&format!("std/{namespace}")) {
                            return Err(format!("{callee} requires use std/{namespace}"));
                        }
                    }
                    if args.len() != 1 {
                        return Err("println requires one string argument".into());
                    }
                    Instruction::Print(args[0].clone())
                } else if result != Type::Unit && last.unwrap_or_else(|| self.at_end()) {
                    self.implicit.insert(position);
                    Instruction::Return(Some(expr))
                } else {
                    Instruction::Call(expr)
                }
            } else if matches!(
                expr,
                Expr::Try(_)
                    | Expr::Catch(_, _, _)
                    | Expr::ElementMethod(_, _, _, _, _)
                    | Expr::Chain(_, _, _)
                    | Expr::Invoke(_, _)
            ) {
                // `write(&p)?` 这类语句：不是函数的最后一个值表达式，就只求值、不返回。
                if result != Type::Unit && last.unwrap_or_else(|| self.at_end()) {
                    self.implicit.insert(position);
                    Instruction::Return(Some(expr))
                } else {
                    Instruction::Call(expr)
                }
            } else {
                Instruction::Return(Some(expr))
            }
        }))
    }
    /// `match` 的分支模式：`Circle(r)`、`Rect(w, _)`、`Shape.Empty`、字面量 `1 | 2`、`"a"`、`true`
    /// 或 `_`，后接 `:`。
    fn pattern(&mut self) -> Result<Arm, String> {
        let literal = matches!(
            self.peek(),
            Some(Token::Number(_) | Token::Float(_) | Token::Text(_) | Token::Symbol('-'))
        ) || matches!(self.peek(), Some(Token::Word(word)) if word == "true" || word == "false");
        let pattern = if self.keyword("_") {
            Arm::Wildcard
        } else if literal {
            let mut values = Vec::new();
            loop {
                let value = self.atom()?;
                if !matches!(
                    value,
                    Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Text(_)
                ) {
                    return Err(
                        "match patterns are literals such as 1, -2, 'a', \"text\" or true, without interpolation"
                            .into(),
                    );
                }
                values.push(value);
                if !self.symbol('|') {
                    break;
                }
            }
            Arm::Values(values)
        } else {
            let name = self.type_name()?;
            let variant = name.rsplit('.').next().unwrap_or(&name).to_owned();
            if !variant.starts_with(char::is_uppercase) {
                return Err(format!(
                    "expected an enum variant such as Circle(r), or _, in a match arm; found {name}"
                ));
            }
            let mut bindings = Vec::new();
            if self.symbol('(') && !self.symbol(')') {
                loop {
                    bindings.push(self.word()?);
                    if self.symbol(')') {
                        break;
                    }
                    self.expect(',')?;
                }
            }
            if self.peek() == Some(&Token::Symbol('|')) {
                return Err(
                    "only literal patterns can be combined with |; write one arm per variant"
                        .into(),
                );
            }
            Arm::Variant((Some(variant), bindings))
        };
        if self.peek() == Some(&Token::Symbol('='))
            && self.tokens.get(self.position + 1) == Some(&Token::Symbol('>'))
        {
            return Err(
                "expected ':' after the match pattern; arms are written Circle(r): value, not =>"
                    .into(),
            );
        }
        Ok(pattern)
    }
    /// match 的分支在冒号后面换行缩进，每行一个分支。
    fn arms(&mut self) -> Result<(), String> {
        self.colon("match")?;
        if self.peek() != Some(&Token::Newline) {
            return Err("expected the match arms on the following lines, indented".into());
        }
        self.take();
        self.indent("match")
    }
    /// 语句形式的 match：分支是同一行的一条语句或缩进的一块。
    fn match_statement(&mut self, imports: &[String], result: Type) -> Result<Instruction, String> {
        let position = self.at();
        let scrutinee = self.expr()?;
        self.arms()?;
        let mut arms = Vec::new();
        while !matches!(self.peek(), Some(Token::Dedent) | None) {
            let pattern = self.pattern()?;
            let body = self.suite(imports, result, "match arm")?;
            arms.push((pattern, body));
            self.separator()?;
        }
        self.take();
        if !literal_arms(&arms)? {
            let arms = arms
                .into_iter()
                .map(|(arm, body)| (arm.pattern(), body))
                .collect();
            return Ok(Instruction::Match(scrutinee, arms));
        }
        // 字面量 match 展开成 if/elif/else：每个分支的条件是 `值 == 字面量`，`|` 连成 or。
        let subject = self.subject(scrutinee);
        let mut tail: Block = Vec::new();
        for (arm, body) in arms.into_iter().rev() {
            tail = match arm {
                Arm::Values(values) => vec![(
                    Instruction::If(literal_test(&subject, values), body, tail),
                    position,
                )],
                _ => body,
            };
        }
        Ok(match tail.pop() {
            Some((instruction, _)) if tail.is_empty() => instruction,
            _ => unreachable!("literal match has a value arm first"),
        })
    }
    /// 表达式形式的 match：`let v = match s:` 下面每行一个 `Pattern: 值`。
    fn match_value(&mut self) -> Result<Expr, String> {
        let scrutinee = self.expr()?;
        self.arms()?;
        let mut arms = Vec::new();
        while !matches!(self.peek(), Some(Token::Dedent) | None) {
            let pattern = self.pattern()?;
            arms.push((pattern, self.value_suite("match arm")?));
            self.separator()?;
        }
        self.take();
        if !literal_arms(&arms)? {
            let arms = arms
                .into_iter()
                .map(|(arm, value)| (arm.pattern(), value))
                .collect();
            return Ok(Expr::Match(Box::new(scrutinee), arms));
        }
        // 值要有着落：最后是 `_`，或者布尔值的 true、false 都列出了。
        let wildcard = matches!(arms.last(), Some((Arm::Wildcard, _)));
        let mut booleans = BTreeSet::new();
        for (arm, _) in &arms {
            if let Arm::Values(values) = arm {
                for value in values {
                    if let Expr::Bool(b) = value {
                        booleans.insert(*b);
                    }
                }
            }
        }
        if !wildcard && booleans.len() < 2 {
            return Err("a match expression needs a _ arm for the values no pattern lists".into());
        }
        let subject = self.subject(scrutinee);
        let mut arms = arms.into_iter().rev();
        let mut value = arms.next().expect("match has arms").1;
        for (arm, branch) in arms {
            let Arm::Values(values) = arm else {
                unreachable!("_ is the last arm")
            };
            value = Expr::If(
                Box::new(literal_test(&subject, values)),
                Box::new(branch),
                Box::new(value),
            );
        }
        Ok(value)
    }
    /// 字面量 match 反复比较的值：变量、字段、元素直接用；调用等其他表达式先存进隐藏变量
    /// （插在当前语句前面求值一次）。
    fn subject(&mut self, scrutinee: Expr) -> Expr {
        if pure(&scrutinee) && !matches!(scrutinee, Expr::Borrow(_, _)) {
            return scrutinee;
        }
        let name = format!("$match{}", self.temps);
        self.temps += 1;
        self.pending
            .push(Instruction::Let(name.clone(), false, None, scrutinee));
        Expr::Variable(name)
    }
}

/// 把结构体或枚举里声明的方法放进函数表。
fn add_methods(module: &mut Module, methods: Vec<(String, Function)>) -> Result<(), String> {
    for (name, function) in methods {
        if module.functions.insert(name.clone(), function).is_some() {
            return Err(format!("duplicate method {name}"));
        }
    }
    Ok(())
}

/// 分支是不是字面量模式（要展开成 if 链）。不能和枚举变体混用；`_` 只能在最后；字面量不能重复。
fn literal_arms<T>(arms: &[(Arm, T)]) -> Result<bool, String> {
    let literal = arms.iter().any(|(arm, _)| matches!(arm, Arm::Values(_)));
    let mut seen = BTreeSet::new();
    for (index, (arm, _)) in arms.iter().enumerate() {
        match arm {
            Arm::Variant(_) if literal => {
                return Err("this match mixes enum variants and literal patterns".into())
            }
            Arm::Wildcard if index + 1 < arms.len() => {
                return Err("unreachable match arm after _".into())
            }
            Arm::Values(values) => {
                for value in values {
                    if !seen.insert(format!("{value:?}")) {
                        return Err(format!("duplicate match arm {value:?}"));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(literal)
}

/// 字面量分支的条件：`subject == v1 or subject == v2 ...`。
fn literal_test(subject: &Expr, values: Vec<Expr>) -> Expr {
    values
        .into_iter()
        .map(|value| Expr::Binary("==".into(), Box::new(subject.clone()), Box::new(value)))
        .reduce(|a, b| Expr::Binary("||".into(), Box::new(a), Box::new(b)))
        .expect("a literal arm lists at least one value")
}

// 字符串插值：`{name}` 读取变量或字段，`{{`/`}}` 输出字面大括号。
// ponytail: 花括号内只接受变量与字段路径；需要任意表达式时把插值下沉到代码生成。
fn literal(text: &str) -> Result<Expr, String> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut changed = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
                changed = true;
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
                changed = true;
            }
            '{' => {
                changed = true;
                if !literal.is_empty() {
                    segments.push(Expr::Text(std::mem::take(&mut literal)));
                }
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') => return Err("nested { in string interpolation".into()),
                        Some(c) => inner.push(c),
                        None => return Err("unterminated { in string literal".into()),
                    }
                }
                let path = inner.trim();
                if path.is_empty() {
                    return Err("empty {} in string literal; write {{}} for literal braces".into());
                }
                let simple = path.split('.').all(|part| {
                    part.starts_with(|c: char| c.is_alphabetic() || c == '_')
                        && part.chars().all(|c| c.is_alphanumeric() || c == '_')
                });
                segments.push(if !simple {
                    // 由 Parser::literal 按表达式解析。
                    Expr::Call(path.into(), Vec::new())
                } else {
                    match path.split_once('.') {
                        Some((root, field)) => Expr::Field(root.into(), field.into()),
                        None => Expr::Variable(path.into()),
                    }
                });
            }
            '}' => return Err("unmatched } in string literal".into()),
            c => literal.push(c),
        }
    }
    if !changed {
        return Ok(Expr::Text(text.into()));
    }
    if !literal.is_empty() {
        segments.push(Expr::Text(literal));
    }
    Ok(Expr::Format(segments))
}

pub fn parse(source: &str) -> Result<Module, String> {
    let (tokens, lines, flat) = lex(source)?;
    let mut p = Parser {
        tokens,
        lines,
        position: 0,
        type_names: BTreeMap::new(),
        imports: Vec::new(),
        result: Type::Unit,
        implicit: BTreeSet::new(),
        explicit: BTreeSet::new(),
        flat,
        pending: Vec::new(),
        temps: 0,
        type_params: Vec::new(),
        owner_generics: Vec::new(),
        text: source.lines().map(str::to_owned).collect(),
    };
    let result = (|| {
        let mut module = Module {
            structs: BTreeMap::new(),
            type_names: BTreeMap::new(),
            imports: Vec::new(),
            functions: BTreeMap::new(),
            constants: BTreeMap::new(),
        };
        p.newlines();
        while p.peek().is_some() {
            if p.peek() == Some(&Token::Indent) {
                return Err(
                    "unexpected indent; top-level declarations start at the beginning of the line"
                        .into(),
                );
            }
            if p.keyword("use") {
                let mut path = p.word()?;
                while p.symbol('/') {
                    path.push('/');
                    path.push_str(&p.word()?);
                }
                if module.imports.contains(&path) {
                    return Err(format!("duplicate import {path}"));
                }
                module.imports.push(path);
                p.separator()?;
                continue;
            }
            // `@wrapping` 写在 fn 的上一行：这个函数里的算术局部关闭溢出检查。
            let mut wrapping = false;
            while p.symbol('@') {
                let name = p.word()?;
                if name != "wrapping" {
                    return Err(format!(
                        "unknown decorator @{name}; the only decorator is @wrapping"
                    ));
                }
                wrapping = true;
                p.separator()?;
            }
            let public = p.keyword("pub");
            let start = p.at();
            if wrapping && !matches!(p.peek(), Some(Token::Word(word)) if word == "fn") {
                return Err("@wrapping applies only to a fn declaration".into());
            }
            // `const LIMIT = 100`、`const NAME: string = "x"`：解析时就算出值。
            if p.keyword("const") {
                let name = p.word()?;
                if !constant_name(&name) {
                    return Err(format!(
                        "constant names are written in UPPER_CASE, such as {}",
                        name.to_uppercase()
                    ));
                }
                let annotation = if p.symbol(':') { Some(p.ty()?) } else { None };
                p.expect('=')?;
                let value = fold(&p.expr()?, &module.constants)?;
                if let Some(ty) = annotation {
                    let found = literal_type(&value);
                    if ty != found {
                        return Err(match (ty, found) {
                            (Type::Float, Type::Int) => format!(
                                "type mismatch: constant {name} is declared f64 but its value is i64; write it as 1.0"
                            ),
                            _ => format!("type mismatch: constant {name} does not have the declared type"),
                        });
                    }
                }
                if module.structs.contains_key(&name)
                    || module
                        .constants
                        .insert(name.clone(), Constant { public, value })
                        .is_some()
                {
                    return Err(format!("duplicate declaration {name}"));
                }
                p.separator()?;
                continue;
            }
            if p.keyword("struct") {
                let name = p.word()?;
                if matches!(name.as_str(), "string" | "i64" | "f64" | "bool") {
                    return Err(format!("reserved type name {name}"));
                }
                if !name.starts_with(char::is_uppercase) {
                    return Err("struct names must start with an uppercase letter".into());
                }
                // `struct Pair[A, B]:`：字段和方法里可以用 A、B。
                let generics = p.generics()?;
                p.type_params = generics.clone();
                p.owner_generics = generics.clone();
                p.colon("struct")?;
                let mut fields = Vec::new();
                let mut names = BTreeSet::new();
                let mut methods = Vec::new();
                let block = p.peek() == Some(&Token::Newline);
                if block {
                    p.take();
                    p.indent("struct")?;
                }
                loop {
                    if block && matches!(p.peek(), Some(Token::Dedent) | None) {
                        p.take();
                        break;
                    }
                    if p.keyword("pass") {
                        if !block {
                            break;
                        }
                        p.separator()?;
                        continue;
                    }
                    if block && p.method_start() {
                        methods.push(p.method(&name, &module.imports.clone())?);
                        p.separator()?;
                        continue;
                    }
                    let field = p.word()?;
                    if !names.insert(field.clone()) {
                        return Err(format!("duplicate field {name}.{field}"));
                    }
                    p.expect(':')?;
                    let ty = p.ty()?;
                    if ty.is_reference() {
                        return Err("borrow escape: structs cannot store references".into());
                    }
                    if matches!(ty, Type::ArrayInt(_)) {
                        return Err("arrays are only supported as local values".into());
                    }
                    fields.push((field, ty));
                    if !block {
                        break;
                    }
                    p.separator()?;
                }
                if module.functions.contains_key(&name)
                    || module.constants.contains_key(&name)
                    || module
                        .structs
                        .insert(
                            name.clone(),
                            Struct {
                                public,
                                fields,
                                variants: Vec::new(),
                                generics,
                            },
                        )
                        .is_some()
                {
                    return Err(format!("duplicate declaration {name}"));
                }
                add_methods(&mut module, methods)?;
                p.type_params.clear();
                p.owner_generics.clear();
                p.separator()?;
                continue;
            }
            if p.keyword("enum") {
                let name = p.word()?;
                if matches!(name.as_str(), "string" | "i64" | "f64" | "bool") {
                    return Err(format!("reserved type name {name}"));
                }
                if !name.starts_with(char::is_uppercase) {
                    return Err("enum names must start with an uppercase letter".into());
                }
                let generics = p.generics()?;
                p.type_params = generics.clone();
                p.owner_generics = generics.clone();
                p.colon("enum")?;
                if p.keyword("pass") {
                    return Err(format!("expected at least one variant in enum {name}"));
                }
                if p.peek() != Some(&Token::Newline) {
                    return Err(format!(
                        "expected the variants of enum {name} on the following lines, indented"
                    ));
                }
                p.take();
                p.indent("enum")?;
                let mut fields = vec![("$tag".to_owned(), Type::Int)];
                let mut variants: Vec<(String, usize, usize)> = Vec::new();
                let mut methods = Vec::new();
                while !p.symbol_dedent() {
                    if p.peek().is_none() {
                        break;
                    }
                    if p.method_start() {
                        methods.push(p.method(&name, &module.imports.clone())?);
                        p.separator()?;
                        continue;
                    }
                    let variant = p.word()?;
                    if !variant.starts_with(char::is_uppercase) {
                        return Err("enum variant names must start with an uppercase letter".into());
                    }
                    if variants.iter().any(|(other, _, _)| *other == variant) {
                        return Err(format!("duplicate variant {name}.{variant}"));
                    }
                    let first = fields.len();
                    if p.symbol('(') && !p.symbol(')') {
                        let mut names = BTreeSet::new();
                        loop {
                            let field = p.word()?;
                            if !names.insert(field.clone()) {
                                return Err(format!("duplicate field {variant}.{field}"));
                            }
                            p.expect(':')?;
                            let ty = p.ty()?;
                            if ty.is_reference() {
                                return Err("borrow escape: enums cannot store references".into());
                            }
                            if matches!(ty, Type::ArrayInt(_)) {
                                return Err("arrays are only supported as local values".into());
                            }
                            fields.push((format!("{variant}${field}"), ty));
                            if p.symbol(')') {
                                break;
                            }
                            p.expect(',')?;
                        }
                    }
                    variants.push((variant, first, fields.len() - first));
                    p.separator()?;
                }
                if variants.is_empty() {
                    return Err(format!("expected at least one variant in enum {name}"));
                }
                if module.functions.contains_key(&name)
                    || module.constants.contains_key(&name)
                    || module
                        .structs
                        .insert(
                            name.clone(),
                            Struct {
                                public,
                                fields,
                                variants,
                                generics,
                            },
                        )
                        .is_some()
                {
                    return Err(format!("duplicate declaration {name}"));
                }
                add_methods(&mut module, methods)?;
                p.type_params.clear();
                p.owner_generics.clear();
                p.separator()?;
                continue;
            }
            let (name, function) = if p.keyword("extern") {
                p.external(public, start)?
            } else if p.keyword("fn") {
                let imports = module.imports.clone();
                let (name, mut function) = p.function(public, start, &imports, None)?;
                function.wrapping = wrapping;
                (name, function)
            } else {
                return Err(
                    "expected use, const, struct, enum, fn or extern fn declaration".into(),
                );
            };
            if module.structs.contains_key(&name) {
                return Err(format!("duplicate declaration {name}"));
            }
            if module.functions.insert(name.clone(), function).is_some() {
                return Err(format!("duplicate function {name}"));
            }
            p.separator()?;
        }
        module.type_names = std::mem::take(&mut p.type_names);
        Ok(module)
    })();
    result.map_err(|error: String| {
        let (line, column) = p.at();
        format!("line {line}, column {column}: {error}")
    })
}

// Stable, path-free checked IR for module cache validation.
pub fn encode(module: &Module) -> Vec<u8> {
    let declarations: Vec<_> = module
        .structs
        .iter()
        .map(|(name, structure)| {
            (
                name,
                structure.public,
                &structure.fields,
                &structure.variants,
                &structure.generics,
            )
        })
        .collect();
    let constants: Vec<_> = module
        .constants
        .iter()
        .map(|(name, constant)| (name, constant.public, &constant.value))
        .collect();
    format!(
        "ZAM-IR-34\n{declarations:?}\n{constants:?}\n{:?}\n{:?}\n{:?}\n",
        module.type_names, module.imports, module.functions
    )
    .into_bytes()
}

#[derive(Clone)]
struct Binding {
    ty: Type,
    mutable: bool,
    live: bool,
}
pub fn record_id(name: &str) -> u64 {
    u64::from_str_radix(&crate::project::hash(&[name.as_bytes()])[..16], 16).unwrap()
}

pub fn field_path(
    structs: &BTreeMap<String, Struct>,
    mut ty: Type,
    path: &str,
) -> Result<(Vec<usize>, Type), String> {
    // 借用的结构体按它指向的结构体取字段。
    if let Type::RecordRef(id, _) = ty {
        ty = Type::Record(id);
    }
    let mut indices = Vec::new();
    for field in path.split('.') {
        let structure = structs
            .iter()
            .find(|(name, _)| ty == Type::Record(record_id(name)))
            .ok_or("field access requires a struct")?
            .1;
        let (index, (_, field_ty)) = structure
            .fields
            .iter()
            .enumerate()
            .find(|(_, (name, _))| name == field)
            .ok_or_else(|| format!("unknown field {field}"))?;
        indices.push(index);
        ty = *field_ty;
    }
    Ok((indices, ty))
}

struct Checker<'a> {
    structs: &'a BTreeMap<String, Struct>,
    functions: &'a BTreeMap<String, Function>,
    locals: BTreeMap<String, Binding>,
    loops: Vec<(BTreeMap<String, Binding>, Expr)>,
    position: SourcePosition,
    // 当前函数的错误类型（`-> T!E` 的 E）；`?`/`fail` 依赖它。
    error: Option<Type>,
    // 当前函数的成功类型；`catch` 的代码块按它检查 return。
    result: Type,
    // 正被 `for x in &items` 遍历的根绑定：循环体内不能修改、移走或独占借用它们。
    frozen: Vec<String>,
    // 当前函数所在的模块：调用别的模块的方法要求它是 pub。
    module: &'a str,
    // 正在检查单独成句的 `f() catch e:`：调用的值没人用，catch 块处理完可以接着往下走。
    discarded: bool,
    // 检查期间被写过的根（赋值、可变借用、改容器都经过 `thawed`）：闭包据此分出哪些捕获要可变借用。
    written: std::cell::RefCell<BTreeSet<String>>,
    // 绑定到名字上的闭包（`fn helper(...):`、`let f = fn(...): ...`）：它用到的外层变量与是否写它们。
    // 每次调用或转传闭包时，这些变量在那条语句里被借用。
    closures: BTreeMap<String, Vec<(String, bool)>>,
    // 正在检查泛型函数的副本：`copy(v)` 对 i64、f64、bool 也成立（T 可能是任何类型）。
    generic: bool,
    // 块内声明的每个局部量的类型（`block` 退出时会从 `locals` 删掉它们，这里留一份）。
    // 编辑器（`local_type`）靠它拿到光标处变量的类型。
    declared: BTreeMap<String, Type>,
}
impl<'a> Checker<'a> {
    fn thawed(&self, root: &str) -> Result<(), String> {
        self.written.borrow_mut().insert(root.to_owned());
        if self.frozen.iter().any(|name| name == root) {
            Err(format!(
                "borrow conflict: {root} is borrowed by an enclosing for loop or a borrowed local such as let x = &{root}[i]"
            ))
        } else {
            Ok(())
        }
    }
    /// 局部变量或字段路径的根名与类型；根必须仍然有效。
    fn place(&self, path: &str) -> Result<(String, Type), String> {
        let (root, field) = path
            .split_once('.')
            .map_or((path, None), |(root, field)| (root, Some(field)));
        let binding = self.binding(root)?;
        let ty = match field {
            Some(field) => field_path(self.structs, binding.ty, field)?.1,
            None => binding.ty,
        };
        Ok((root.to_owned(), ty))
    }
    /// 把路径当容器借用一次：检查可变性、冻结与已有借用，返回背后的 owned 容器类型
    /// （`[T]` 或 `{K: V}`）。不记录借用，调用方按需要把根写进 `loans`。
    fn container(
        &self,
        path: &str,
        mutable: bool,
        loans: &BTreeMap<String, Type>,
    ) -> Result<Type, String> {
        let (root, ty) = self.place(path)?;
        let (Some(base), Some((_, by_ref))) = (ty.container(), ty.borrowable()) else {
            return Err(format!("type mismatch: {path} is not a list or map"));
        };
        if mutable {
            if !by_ref.unwrap_or(self.writable(&root)) {
                return Err(format!("cannot mutably borrow immutable value {path}"));
            }
            self.thawed(&root)?;
        }
        if loans
            .get(&root)
            .is_some_and(|old| *old == Type::Mutable || mutable)
        {
            return Err(format!("borrow conflict: {path}"));
        }
        Ok(base)
    }
    /// 容器下标的（键类型, 元素类型）：列表用 i64 下标，字典用键。
    fn subscript(container: Type) -> (Type, Elem) {
        match container {
            Type::List(elem) => (Type::Int, elem),
            Type::Map(key, value) => (key.key_arg(), value),
            _ => unreachable!("container type"),
        }
    }
    /// `ty` 是枚举或枚举的借用时，返回（限定名, 声明）。
    fn enumeration(&self, ty: Type) -> Option<(&'a String, &'a Struct)> {
        let (Type::Record(id) | Type::RecordRef(id, _)) = ty else {
            return None;
        };
        self.structs
            .iter()
            .find(|(name, structure)| record_id(name) == id && !structure.variants.is_empty())
    }
    /// match 被匹配值的类型，不求值（位置按位置取类型）。
    fn scrutinee_ty(&self, expr: &Expr) -> Result<Type, String> {
        match expr {
            Expr::Variable(name) => Ok(self.place(name)?.1),
            Expr::Field(local, field) => Ok(self.place(&format!("{local}.{field}"))?.1),
            Expr::Index(path, _) if self.place(path)?.1.container().is_some() => {
                Ok(self.element(path, "")?.1)
            }
            Expr::IndexField(path, _, field) => Ok(self.element(path, field)?.1),
            _ => self.ty(expr),
        }
    }
    /// 检查 match 的被匹配值：变量、字段与元素只借用（返回匹配期间要冻结的根绑定），
    /// 其他表达式求值成临时值。
    fn scrutinee(
        &mut self,
        expr: &Expr,
        loans: &mut BTreeMap<String, Type>,
    ) -> Result<(u64, Option<String>), String> {
        let ty = self.scrutinee_ty(expr)?;
        let Some((name, _)) = self.enumeration(ty) else {
            return Err("type mismatch: match requires an enum value, or literal patterns such as 1, \"a\" or true".into());
        };
        let id = record_id(name);
        let root = match expr {
            Expr::Variable(name) | Expr::Field(name, _) => {
                Some(name.split('.').next().unwrap_or(name).to_owned())
            }
            Expr::Index(path, index) | Expr::IndexField(path, index, _)
                if self.place(path)?.1.container().is_some() =>
            {
                let (key, _) = self.element(path, "")?;
                self.container(path, false, loans)?;
                self.expr(index, key, loans)?;
                Some(path.split('.').next().unwrap_or(path).to_owned())
            }
            _ => {
                self.expr(expr, ty, loans)?;
                None
            }
        };
        if let Some(root) = &root {
            if loans.get(root) == Some(&Type::Mutable) {
                return Err(format!("borrow conflict: {root}"));
            }
        }
        Ok((id, root))
    }
    /// 一个分支绑定的变量：标量载荷按值，其他载荷是只读借用。
    fn arm_bindings(&self, id: u64, pattern: &Pattern) -> Result<Vec<(String, Type)>, String> {
        let (name, declaration) = self.enumeration(Type::Record(id)).expect("checked enum");
        let Some(variant) = &pattern.0 else {
            return Ok(Vec::new());
        };
        let short = name.rsplit(':').next().unwrap_or(name);
        let (_, first, count) = declaration
            .variants
            .iter()
            .find(|(other, _, _)| other == variant)
            .ok_or_else(|| format!("unknown variant {variant} of {short}"))?;
        if pattern.1.len() != *count {
            return Err(format!(
                "type mismatch: {short}.{variant} has {count} field(s), but the pattern binds {}",
                pattern.1.len()
            ));
        }
        let mut bindings: Vec<(String, Type)> = Vec::new();
        for (binding, (_, ty)) in pattern.1.iter().zip(&declaration.fields[*first..]) {
            if binding == "_" {
                continue;
            }
            if self.locals.contains_key(binding) || bindings.iter().any(|(b, _)| b == binding) {
                return Err(format!("duplicate binding {binding}"));
            }
            let ty = if copied(*ty) {
                *ty
            } else {
                borrow_of(*ty, false).expect("enum payloads are owned values")
            };
            bindings.push((binding.clone(), ty));
        }
        Ok(bindings)
    }
    /// 分支要覆盖所有变体（或有 `_`），不能重复，`_` 之后不能再有分支。
    fn arms_cover<'p>(
        &self,
        id: u64,
        patterns: impl Iterator<Item = &'p Pattern>,
    ) -> Result<(), String> {
        let (name, declaration) = self.enumeration(Type::Record(id)).expect("checked enum");
        let short = name.rsplit(':').next().unwrap_or(name);
        let mut seen = BTreeSet::new();
        let mut wildcard = false;
        for (variant, _) in patterns {
            if wildcard {
                return Err("unreachable match arm after _".into());
            }
            match variant {
                None => wildcard = true,
                Some(variant) => {
                    if !declaration.variants.iter().any(|(v, _, _)| v == variant) {
                        return Err(format!("unknown variant {variant} of {short}"));
                    }
                    if !seen.insert(variant) {
                        return Err(format!("duplicate match arm {variant}"));
                    }
                }
            }
        }
        let missing: Vec<&str> = declaration
            .variants
            .iter()
            .filter(|(v, _, _)| !seen.contains(v))
            .map(|(v, _, _)| v.as_str())
            .collect();
        if !wildcard && !missing.is_empty() {
            return Err(format!(
                "match on {short} does not cover {}; add an arm for each or a _ arm",
                missing.join(", ")
            ));
        }
        Ok(())
    }
    /// 根绑定能否经由它写：`let mut` 局部，或 `&mut T` 结构体借用。
    fn writable(&self, root: &str) -> bool {
        self.locals
            .get(root)
            .is_some_and(|b| b.mutable || matches!(b.ty, Type::RecordRef(_, true)))
    }
    /// `path[..]` 的元素，或元素里 `field` 路径上的值：（键类型, 目标类型）。
    fn element(&self, path: &str, field: &str) -> Result<(Type, Type), String> {
        let Some(container) = self.place(path)?.1.container() else {
            return Err(format!("type mismatch: {path} is not a list or map"));
        };
        let (key, elem) = Self::subscript(container);
        let target = if field.is_empty() {
            elem.ty()
        } else {
            field_path(self.structs, elem.ty(), field)?.1
        };
        Ok((key, target))
    }
    /// `path[..].field.method(...)` 的（下标类型, 是否独占借用接收者, 参数类型, 结果类型）。
    fn element_method(
        &self,
        path: &str,
        field: &str,
        method: &str,
    ) -> Result<(Type, bool, Vec<Type>, Type), String> {
        let (key, target) = self.element(path, field)?;
        let receiver = format!("{path}[..].{field}");
        if method.starts_with('$') && target.container().is_none() {
            return Err(format!(
                "type mismatch: {path}[..] is not a list or map, so it cannot be indexed again"
            ));
        }
        let Some(container) = target.container() else {
            return Err(format!(
                "type mismatch: {receiver} has no method {method}; methods are only available on lists and maps"
            ));
        };
        let (mutable, params, result) = method_signature(target, method).ok_or_else(|| {
            let supported = supported_methods(container, method);
            format!("unknown function {receiver}.{method}: {supported}")
        })?;
        Ok((key, mutable, params, result))
    }
    /// 被调函数的实际名字：`abs`/`min`/`max` 按第一个参数的类型选版本。
    /// 被调函数的实际名字：`abs`/`min`/`max` 按第一个参数的类型选版本，泛型函数按实参类型
    /// 选（并生成）专门的副本。推断失败时返回原名，由调用处报错。
    fn callee(&self, name: &str, args: &[Expr]) -> String {
        // `2 ** 0.5`：乘方只要有一边是浮点数就按浮点算，与 Python 相同。
        let float_power = name == "std/core:pow"
            && args
                .iter()
                .any(|arg| self.ty(arg).ok() == Some(Type::Float));
        let first = if float_power {
            Some(Type::Float)
        } else {
            args.first().and_then(|arg| self.ty(arg).ok())
        };
        let name = overload(name, first);
        self.generic_instance(name, args)
            .unwrap_or_else(|_| name.to_owned())
    }
    /// 泛型函数推断不出类型参数时的报错。
    fn generic_error(&self, name: &str, args: &[Expr]) -> Option<String> {
        self.generic_instance(name, args).err()
    }
    fn generic_instance(&self, name: &str, args: &[Expr]) -> Result<String, String> {
        let actuals: Vec<Option<Type>> = args.iter().map(|arg| self.value_type(arg).ok()).collect();
        // 没写参数类型的匿名函数：参数类型由其他实参推断出的类型参数决定，返回类型由函数体决定。
        let late = |index: usize, params: Vec<Type>| {
            let Expr::Lambda(names, result, body) = &args[index] else {
                return None;
            };
            let (id, _) = self
                .fork()
                .lambda(names, *result, body, Some((params, None, None)))
                .ok()?;
            Some(Type::FuncRef(id))
        };
        instance_with(self.functions, name, &actuals, &late)
    }
    /// 调用的目标函数与实参：普通调用原样返回；结构体的方法改写成 `Owner.m(接收者, ...)`。
    /// 列表、字典的方法与不是调用的表达式返回 None。
    fn call_target(&self, expr: &Expr) -> Result<Option<(String, Vec<Expr>)>, String> {
        let (receiver, ty, method, args) = match expr {
            Expr::Call(name, args) => return Ok(Some((self.callee(name, args), args.clone()))),
            Expr::Method(path, method, args) => {
                let ty = self.place(path)?.1;
                (Receiver::Path(path, ty), ty, method, args)
            }
            Expr::ElementMethod(path, index, field, method, args) => (
                Receiver::Element(path, index, field),
                self.element(path, field)?.1,
                method,
                args,
            ),
            Expr::Chain(receiver, method, args) => {
                (Receiver::Temp(receiver), self.ty(receiver)?, method, args)
            }
            _ => return Ok(None),
        };
        let Some((name, callee)) = record_method(self.structs, self.functions, ty, method)? else {
            return Ok(None);
        };
        if !callee.public && name.split(':').next() != Some(self.module) {
            let short = name.split_once(':').map_or(name.as_str(), |(_, n)| n);
            return Err(format!("private method {short}; declare it with pub fn"));
        }
        Ok(Some((
            name.clone(),
            method_args(receiver, callee, method, args)?,
        )))
    }
    /// 结构体方法调用（不是列表、字典的方法）。
    fn record_call(&self, expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Method(..) | Expr::ElementMethod(..) | Expr::Chain(..)
        ) && !matches!(self.call_target(expr), Ok(None))
    }
    /// 调用结果上的方法不存在：列出能用的方法。
    fn chain_error(&self, ty: Type, method: &str) -> String {
        match ty.container() {
            Some(container) => format!(
                "unknown function .{method}: {}",
                supported_methods(container, method)
            ),
            None => format!(
                "type mismatch: this value has no method {method}; methods are only available on lists and maps"
            ),
        }
    }
    /// 只读使用一个值时它的 owned 类型：`&x`、`&xs[i]` 与借用参数都换成被借用的类型。
    fn value_type(&self, expr: &Expr) -> Result<Type, String> {
        let ty = match expr {
            Expr::Borrow(path, _) => self.place(path)?.1,
            Expr::BorrowIndex(path, _, field, _) => self.element(path, field)?.1,
            _ => self.ty(expr)?,
        };
        Ok(ty.borrowable().map_or(ty, |(owned, _)| owned))
    }
    /// 只读使用一个值（比较、打印、复制）：变量、字段、元素与借用都不移动；
    /// 其他表达式（调用、字面量）求值成临时值，语句结束时释放。
    fn read(
        &mut self,
        expr: &Expr,
        ty: Type,
        loans: &mut BTreeMap<String, Type>,
    ) -> Result<(), String> {
        let shared = borrow_of(ty, false).unwrap_or(ty);
        match expr {
            Expr::Variable(name) => self.binding(name).map(|_| ()),
            Expr::Field(_, _) => self.ty(expr).map(|_| ()),
            Expr::Borrow(_, false) | Expr::BorrowIndex(_, _, _, false) => {
                self.expr(expr, shared, &mut loans.clone())
            }
            Expr::Index(local, index) if self.place(local)?.1.container().is_some() => self.expr(
                &Expr::BorrowIndex(local.clone(), index.clone(), String::new(), false),
                shared,
                &mut loans.clone(),
            ),
            Expr::IndexField(local, index, field) => self.expr(
                &Expr::BorrowIndex(local.clone(), index.clone(), field.clone(), false),
                shared,
                &mut loans.clone(),
            ),
            _ => self.expr(expr, ty, loans),
        }
    }
    fn method(&self, path: &str, method: &str) -> Result<(bool, Vec<Type>, Type), String> {
        let root = path.split('.').next().unwrap_or(path);
        if !self.locals.contains_key(root) {
            return Err(format!("ambiguous or missing import for {path}.{method}"));
        }
        let ty = self.place(path)?.1;
        let Some(container) = ty.container() else {
            let message = "methods are only available on lists and maps";
            return Err(format!(
                "type mismatch: {path} has no method {method}; {message}"
            ));
        };
        method_signature(ty, method).ok_or_else(|| {
            if method == "get" && matches!(container, Type::Map(_, Elem::Record(_))) {
                return format!(
                    "unknown function {path}.get: struct values cannot be copied; check {path}.contains(k), then read {path}[k].field or borrow &{path}[k]"
                );
            }
            if method == "get" && matches!(container, Type::Map(_, Elem::Nested(_))) {
                return format!(
                    "unknown function {path}.get: list and map values cannot be copied by get; check {path}.contains(k), then borrow &{path}[k] or call {path}[k].len()"
                );
            }
            let supported = supported_methods(container, method);
            format!("unknown function {path}.{method}: {supported}")
        })
    }
    fn binding(&self, name: &str) -> Result<&Binding, String> {
        let binding = self
            .locals
            .get(name)
            .ok_or_else(|| format!("unknown variable {name}"))?;
        if !binding.live {
            return Err(format!("use after move: {name}"));
        }
        Ok(binding)
    }
    /// 构造表达式的类型编号与（实例化后的）声明：泛型结构体与枚举按期望类型或字段推断类型实参。
    fn constructor(
        &self,
        expr: &Expr,
        expected: Option<Type>,
    ) -> Result<(u64, &'a Struct), String> {
        let (Expr::Record(name, _) | Expr::Variant(name, _, _)) = expr else {
            unreachable!("constructor expression")
        };
        self.structs
            .get(name)
            .ok_or_else(|| format!("unknown struct {name}"))?;
        let id = constructed(self.structs, expr, expected, |e| self.ty(e).ok())?;
        let declaration = self
            .structs
            .iter()
            .find(|(name, _)| record_id(name) == id)
            .ok_or_else(|| format!("unknown struct {name}"))?
            .1;
        Ok((id, declaration))
    }
    /// 带着当前局部变量的副本：求类型时要在里面试着检查（匿名函数、match 分支）。
    fn fork(&self) -> Checker<'a> {
        Checker {
            structs: self.structs,
            functions: self.functions,
            locals: self.locals.clone(),
            loops: Vec::new(),
            position: self.position,
            error: self.error,
            result: self.result,
            frozen: self.frozen.clone(),
            module: self.module,
            generic: self.generic,
            declared: Default::default(),
            discarded: false,
            written: Default::default(),
            closures: self.closures.clone(),
        }
    }
    /// 具名函数当值用时的类型。泛型函数、会失败的函数暂时不能当值。
    fn fn_value(&self, name: &str) -> Result<Type, String> {
        let function = self
            .functions
            .get(name)
            .ok_or_else(|| format!("unknown function {name}"))?;
        let short = name.rsplit(':').next().unwrap_or(name);
        if !function.generics.is_empty() {
            return Err(format!(
                "generic function {short} needs a known function type to be used as a value; annotate it, such as let f: fn(i64) -> i64 = {short}"
            ));
        }
        let params = function.params.iter().map(|(_, ty)| *ty).collect();
        Ok(Type::Func(func_type_full(
            params,
            function.result,
            function.error,
        )))
    }
    /// 函数值的签名：`f` 是函数类型的局部变量或参数，`p.on_click` 是结构体里的函数字段。
    fn apply_sig(&self, path: &str) -> Result<(Vec<Type>, Type), String> {
        match self.place(path)?.1 {
            Type::Func(id) | Type::FuncRef(id) => Ok(func_sig(id)),
            _ => Err(format!("type mismatch: {path} is not a function")),
        }
    }
    /// `p.on_click(x)`：调用结构体里存的函数（没有同名方法时）。
    fn field_call(&self, expr: &Expr) -> Option<(String, Vec<Expr>)> {
        let Expr::Method(path, method, args) = expr else {
            return None;
        };
        let receiver = self.place(path).ok()?.1;
        field_function(self.structs, receiver, method)?;
        Some((format!("{path}.{method}"), args.clone()))
    }
    /// 匿名函数：参数类型取标注或 `hint`，函数体在只读看见外层局部变量的环境里检查。
    /// 返回函数类型的编号与捕获的外层变量。
    fn lambda(
        &mut self,
        params: &[(String, Option<Type>)],
        result: Option<Type>,
        block: &Block,
        hint: Option<(Vec<Type>, Option<Type>, Option<Type>)>,
    ) -> Result<(u64, Vec<(String, bool)>), String> {
        let body = lambda_body(block);
        let prints = lambda_prints(block);
        let failure = hint.as_ref().and_then(|(_, _, error)| *error);
        if let Some((types, _, _)) = &hint {
            if types.len() != params.len() {
                return Err(format!(
                    "argument count mismatch: the function takes {} parameter(s), but this fn has {}",
                    types.len(),
                    params.len()
                ));
            }
        }
        let mut types = Vec::new();
        for (index, (name, annotated)) in params.iter().enumerate() {
            let wanted = hint.as_ref().map(|(types, _, _)| types[index]);
            let ty = match (*annotated, wanted) {
                (Some(ty), Some(wanted)) if ty != wanted => {
                    return Err(format!(
                        "type mismatch: parameter {name} of this fn has a different type than the expected function type"
                    ))
                }
                (Some(ty), _) | (None, Some(ty)) => ty,
                (None, None) => {
                    return Err(format!(
                        "cannot infer the type of parameter {name}; write it, such as fn({name}: i64): ..."
                    ))
                }
            };
            if self.locals.contains_key(name) {
                return Err(format!(
                    "{name} is already declared; give the fn parameter another name"
                ));
            }
            types.push(ty);
        }
        let mut roots = BTreeSet::new();
        block_roots(block, &mut roots);
        let captures: Vec<String> = roots
            .into_iter()
            .filter(|name| self.locals.contains_key(name) && !params.iter().any(|(p, _)| p == name))
            .collect();
        let mut inner = BTreeMap::new();
        for name in &captures {
            let binding = self.binding(name)?;
            let writable = binding.mutable
                || binding
                    .ty
                    .borrowable()
                    .is_some_and(|(_, by_ref)| by_ref == Some(true));
            inner.insert(
                name.clone(),
                Binding {
                    ty: captured_view(binding.ty, writable)?,
                    mutable: binding.mutable,
                    live: true,
                },
            );
        }
        for ((name, _), ty) in params.iter().zip(&types) {
            inner.insert(
                name.clone(),
                Binding {
                    ty: *ty,
                    mutable: false,
                    live: true,
                },
            );
        }
        let outer = std::mem::replace(&mut self.locals, inner);
        let error = std::mem::replace(&mut self.error, failure);
        let loops = std::mem::take(&mut self.loops);
        let frozen = std::mem::take(&mut self.frozen);
        let outer_result = self.result;
        let closures = std::mem::take(&mut self.closures);
        let written = self.written.take();
        let checked = (|| {
            let wanted = hint.as_ref().and_then(|(_, result, _)| *result);
            let result = match (result, wanted) {
                (Some(ty), Some(wanted)) if ty != wanted => {
                    return Err("type mismatch: this fn returns a different type than the expected function type".to_owned())
                }
                (Some(ty), _) | (None, Some(ty)) => ty,
                (None, None) if prints => Type::Unit,
                (None, None) => match body {
                    Some(body) => self.ty(body)?,
                    None => return Err("write the return type of this fn, such as -> i64".to_owned()),
                },
            };
            if result.is_reference() {
                return Err("borrow escape: an anonymous fn returns owned values; copy the value, such as copy(&x)".to_owned());
            }
            let Some(body) = body else {
                // 函数里定义的函数：完整的代码块，return 从它自己返回。
                self.result = result;
                let returned = self.block(block, result)?;
                if result != Type::Unit && !returned {
                    return Err(
                        "missing return: this fn returns a value, but the end of its body has none; end every path with a value or return"
                            .to_owned(),
                    );
                }
                return Ok(func_type_full(types, result, failure));
            };
            if prints || result == Type::Unit {
                if prints && result != Type::Unit {
                    return Err(
                        "type mismatch: println has no value, but the function type returns one"
                            .to_owned(),
                    );
                }
                self.block(block, Type::Unit)?;
            } else {
                self.expr(body, result, &mut BTreeMap::new())?;
            }
            Ok(func_type_full(types, result, failure))
        })();
        self.locals = outer;
        self.error = error;
        self.loops = loops;
        self.frozen = frozen;
        self.result = outer_result;
        let inner_closures = std::mem::replace(&mut self.closures, closures);
        let wrote = self.written.replace(written);
        self.written.borrow_mut().extend(wrote.iter().cloned());
        let id = checked?;
        // 函数体里调用的闭包用到的变量，也算这个函数用到的。
        let mut result: Vec<(String, bool)> = captures
            .iter()
            .map(|name| (name.clone(), wrote.contains(name)))
            .collect();
        for name in &captures {
            for (inner, writes) in self.closures.get(name).cloned().unwrap_or_default() {
                match result.iter_mut().find(|(other, _)| *other == inner) {
                    Some(entry) => entry.1 |= writes,
                    None => result.push((inner, writes)),
                }
            }
        }
        drop(inner_closures);
        Ok((id, result))
    }
    /// 调用或转传一个闭包：它用到的变量必须还有效，在这条语句里被借用（写的要可变借用）。
    fn capture_loans(
        &self,
        user: &str,
        captures: &[(String, bool)],
        loans: &mut BTreeMap<String, Type>,
    ) -> Result<(), String> {
        for (name, writes) in captures {
            let binding = self
                .locals
                .get(name)
                .ok_or_else(|| format!("unknown variable {name}"))?;
            if !binding.live {
                return Err(format!("use after move: {name} (the fn {user} uses it)"));
            }
            if *writes {
                self.thawed(name)?;
            }
            match loans.get(name) {
                Some(Type::Mutable) => {
                    return Err(format!("borrow conflict: {name} (the fn {user} uses it)"))
                }
                Some(_) if *writes => {
                    return Err(format!(
                        "borrow conflict: {name} (the fn {user} changes it)"
                    ))
                }
                _ => {}
            }
            if *writes {
                loans.insert(name.clone(), Type::Mutable);
            } else {
                loans.entry(name.clone()).or_insert(Type::Shared);
            }
        }
        Ok(())
    }
    /// `s.upper()`、`string.trim(&s).upper()`：字符串上的方法改写成 std/string 的调用。接收者不是
    /// 字符串时返回 None；是字符串但没有这个方法时报错。
    fn string_call(&self, expr: &Expr) -> Result<Option<Expr>, String> {
        // `s[i]`：第 i 个字符（与 Python 相同，负数从末尾数）。
        if let Expr::Index(path, index) = expr {
            if self.place(path).is_ok_and(|(_, ty)| is_string(ty)) {
                return Ok(Some(char_at(path, index)));
            }
            return Ok(None);
        }
        if let Expr::Binary(op, item, other) = expr {
            if op != "in" && op != "not in" {
                return Ok(match (self.ty(item), self.ty(other)) {
                    (Ok(left), Ok(right)) => promote(expr, left, right),
                    _ => None,
                });
            }
            return Ok(membership(expr, self.ty(item).is_ok_and(is_text)));
        }
        let (receiver, method, args) = match expr {
            Expr::Method(path, method, args) => {
                if !self.place(path).is_ok_and(|(_, ty)| is_string(ty)) {
                    return Ok(None);
                }
                (Expr::Borrow(path.clone(), false), method, args)
            }
            Expr::Chain(inner, method, args) => {
                if !self.ty(inner).is_ok_and(is_string) {
                    return Ok(None);
                }
                ((**inner).clone(), method, args)
            }
            // `parts[0].strip()`、`rows[i].name.upper()`：字符串元素或字段借给 std/string 函数。
            Expr::ElementMethod(path, index, field, method, args) => {
                if !self.element(path, field).is_ok_and(|(_, ty)| is_string(ty)) {
                    return Ok(None);
                }
                (
                    Expr::BorrowIndex(path.clone(), index.clone(), field.clone(), false),
                    method,
                    args,
                )
            }
            _ => return Ok(None),
        };
        string_method(self.functions, receiver, method, args)
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "unknown method .{method} for a string; string methods are the std/string functions, such as s.len(), s.upper(), s.split(\",\") or s.contains(\"x\")"
                )
            })
    }
    fn ty(&self, expr: &Expr) -> Result<Type, String> {
        if let Some(call) = self.string_call(expr)? {
            return self.ty(&call);
        }
        match expr {
            Expr::FnRef(name) => self.fn_value(name),
            Expr::Invoke(callee, _) => match self.value_type(callee)? {
                Type::Func(id) | Type::FuncRef(id) => Ok(func_sig(id).1),
                _ => Err("type mismatch: only a function value can be called".into()),
            },
            Expr::Lambda(params, result, body) => {
                let (id, _) = self.fork().lambda(params, *result, body, None)?;
                Ok(Type::Func(id))
            }
            Expr::Apply(path, _) => Ok(self.apply_sig(path)?.1),
            Expr::Method(..) if self.field_call(expr).is_some() => {
                let (path, _) = self.field_call(expr).expect("field call");
                Ok(self.apply_sig(&path)?.1)
            }
            _ if self.record_call(expr) => {
                let (name, _) = self.call_target(expr)?.expect("record method");
                Ok(self.functions[&name].result)
            }
            Expr::Record(_, _) | Expr::Variant(_, _, _) => {
                Ok(Type::Record(self.constructor(expr, None)?.0))
            }
            // 分支的值可能用到绑定：在带上第一个分支绑定的副本里求类型。
            Expr::Match(scrutinee, arms) => {
                let ty = self.scrutinee_ty(scrutinee)?;
                let Some((name, _)) = self.enumeration(ty) else {
                    return Err("type mismatch: match requires an enum value, or literal patterns such as 1, \"a\" or true".into());
                };
                let (pattern, value) = arms.first().ok_or("a match needs at least one arm")?;
                let mut locals = self.locals.clone();
                for (binding, ty) in self.arm_bindings(record_id(name), pattern)? {
                    locals.insert(
                        binding,
                        Binding {
                            ty,
                            mutable: false,
                            live: true,
                        },
                    );
                }
                Checker {
                    structs: self.structs,
                    functions: self.functions,
                    locals,
                    loops: Vec::new(),
                    position: self.position,
                    error: self.error,
                    result: self.result,
                    frozen: Vec::new(),
                    module: self.module,
                    generic: self.generic,
                    declared: Default::default(),
            discarded: false,
            written: Default::default(),
            closures: BTreeMap::new(),
                }
                .ty(value)
            }
            Expr::Field(local, field) => {
                field_path(self.structs, self.binding(local)?.ty, field).map(|(_, ty)| ty)
            }
            Expr::Array(values) if values.is_empty() => Err(
                "type mismatch: an empty list literal needs a type annotation, such as let xs: [i64] = []"
                    .into(),
            ),
            Expr::Array(values) => Ok(Type::ArrayInt(values.len())),
            Expr::Index(local, _) => match self.place(local)?.1 {
                Type::ArrayInt(_) => Ok(Type::Int),
                Type::List(elem) | Type::ListRef(elem, _) => Ok(elem.ty()),
                Type::Map(_, value) | Type::MapRef(_, value, _) => Ok(value.ty()),
                ty if is_text(ty) => Err(
                    "index requires an array, list or map; take one character of a string with s.slice(i, i + 1)"
                        .into(),
                ),
                _ => Err("index requires an array, list or map".into()),
            },
            Expr::MapLit(entries) => {
                let Some((key, value)) = entries.first() else {
                    return Err("type mismatch: an empty map literal needs a type annotation, such as let m: {string: i64} = {}".into());
                };
                let elem = |ty| match ty {
                    Type::Int => Some(Elem::Int),
                    Type::Float => Some(Elem::Float),
                    Type::Bool => Some(Elem::Bool),
                    Type::Owned | Type::Shared | Type::Mutable => Some(Elem::Str),
                    Type::Record(id) => Some(Elem::Record(id)),
                    Type::List(_) | Type::Map(_, _) => Some(nest(ty)),
                    _ => None,
                };
                match (elem(self.ty(key)?), elem(self.ty(value)?)) {
                    (Some(key @ (Elem::Int | Elem::Str)), Some(value)) => Ok(Type::Map(key, value)),
                    _ => Err("unsupported map literal; keys are i64 or string, values are i64, f64, bool or string".into()),
                }
            }
            Expr::IndexField(path, _, field) => Ok(self.element(path, field)?.1),
            Expr::BorrowIndex(path, _, field, mutable) => {
                borrow_of(self.element(path, field)?.1, *mutable)
                    .ok_or_else(|| "scalar references are not supported".into())
            }
            Expr::Method(path, method, _) => self.method(path, method).map(|(_, _, ty)| ty),
            Expr::Tuple(items) => {
                let mut fields = Vec::new();
                for item in items {
                    let ty = self.ty(item)?;
                    if ty.is_reference() || matches!(ty, Type::Unit | Type::ArrayInt(_)) {
                        return Err("tuples hold owned values; copy borrowed values first, such as string.copy(name)".into());
                    }
                    fields.push(ty);
                }
                Ok(Type::Record(linked_tuple(fields)))
            }
            Expr::Chain(receiver, method, _) => {
                let ty = self.ty(receiver)?;
                method_signature(ty, method)
                    .map(|(_, _, result)| result)
                    .ok_or_else(|| self.chain_error(ty, method))
            }
            Expr::ElementMethod(path, _, field, method, _) => self
                .element_method(path, field, method)
                .map(|(_, _, _, ty)| ty),
            Expr::Text(_) => Ok(Type::Owned),
            Expr::Format(_) => Ok(Type::Owned),
            Expr::Int(_) | Expr::ByteLen(_) | Expr::CharLen(_) => Ok(Type::Int),
            Expr::Float(_) => Ok(Type::Float),
            Expr::Bool(_) | Expr::StringEqual(_, _) => Ok(Type::Bool),
            Expr::Variable(name) => Ok(self.binding(name)?.ty),
            Expr::Borrow(_, mutable) => Ok(if *mutable {
                Type::Mutable
            } else {
                Type::Shared
            }),
            Expr::Call(name, args) if name == COPY => match args.as_slice() {
                [arg] => self.value_type(arg),
                _ => Err("argument count mismatch for copy".into()),
            },
            Expr::Call(name, args) => self
                .functions
                .get(&self.callee(name, args))
                .map(|f| f.result)
                .ok_or_else(|| format!("unknown function {name}")),
            // `?` 的值就是被调函数的成功值。
            Expr::Try(inner) => self.ty(inner),
            // `catch` 的成功路径同样拿到被调函数的成功值。
            Expr::Catch(inner, _, _) => self.ty(inner),
            Expr::Unary('!', _) => Ok(Type::Bool),
            Expr::Unary('~', _) => Ok(Type::Int),
            Expr::Unary('f', _) => Ok(Type::Float),
            Expr::Unary(_, value) => Ok(numeric(self.ty(value)?)),
            Expr::Binary(op, left, _) if op == "+" && is_text(self.ty(left)?) => Ok(Type::Owned),
            Expr::Binary(op, _, _) if bitwise(op) => Ok(Type::Int),
            Expr::Binary(op, left, _) => Ok(if matches!(op.as_str(), "+" | "-" | "*" | "/" | "//" | "%") {
                numeric(self.ty(left)?)
            } else {
                Type::Bool
            }),
            Expr::If(_, yes, _) => self.ty(yes),
        }
    }
    /// 二元运算两边一个是 i64、一个是 f64 时的报错：没有隐式转换。
    fn mixed(&self, op: &str, left: &Expr, right: &Expr) -> Option<String> {
        let types = (self.ty(left).ok()?, self.ty(right).ok()?);
        if !matches!(types, (Type::Int, Type::Float) | (Type::Float, Type::Int)) {
            return None;
        }
        let literal = |expr: &Expr| match expr {
            Expr::Int(value) => Some(*value),
            _ => None,
        };
        Some(match literal(left).or(literal(right)) {
            Some(value) => format!(
                "type mismatch: {op} mixes i64 and f64; write the integer {value} as {value}.0"
            ),
            None => format!(
                "type mismatch: {op} mixes i64 and f64; convert one side with f64(n) or i64(x)"
            ),
        })
    }
    /// 字符串运算（`+`、比较）与插值读取的操作数：变量、字段与元素只读，临时值移入。
    fn text_operand(
        &mut self,
        expr: &Expr,
        loans: &mut BTreeMap<String, Type>,
    ) -> Result<(), String> {
        match expr {
            Expr::Variable(name) if is_text(self.binding(name)?.ty) => Ok(()),
            Expr::Field(_, _) if self.ty(expr)? == Type::Owned => Ok(()),
            Expr::Borrow(_, _) => self.expr(expr, Type::Shared, &mut loans.clone()),
            Expr::Index(local, index) if self.ty(expr)? == Type::Owned && self.place(local)?.1.container().is_some() => self.expr(
                &Expr::BorrowIndex(local.clone(), index.clone(), String::new(), false),
                Type::Shared,
                &mut loans.clone(),
            ),
            Expr::IndexField(local, index, field) if self.ty(expr)? == Type::Owned => self.expr(
                &Expr::BorrowIndex(local.clone(), index.clone(), field.clone(), false),
                Type::Shared,
                &mut loans.clone(),
            ),
            _ if self.ty(expr)? == Type::Owned => self.expr(expr, Type::Owned, loans),
            _ => Err(
                "type mismatch: expected a string; to join other values, interpolate them as in \"{s}{n}\""
                    .into(),
            ),
        }
    }
    fn expr(
        &mut self,
        expr: &Expr,
        expected: Type,
        loans: &mut BTreeMap<String, Type>,
    ) -> Result<(), String> {
        if let Some(call) = self.string_call(expr)? {
            return self.expr(&call, expected, loans);
        }
        // 期望 f64 的位置给整数（`let x: f64 = 1`、`area(2)`）：与 Python 一样当小数用。
        // C 侧这些位置都是赋值、带原型的参数、return 或字段初始化，int64_t 自动转成 double。
        if expected == Type::Float && matches!(self.ty(expr), Ok(Type::Int)) {
            return self.expr(expr, Type::Int, loans);
        }
        // 只读借用参数：变量、字段与元素自动借用（`show(name)` 等于 `show(&name)`）。
        if temporary_borrow(expected).is_some() {
            if let Some(borrow) = auto_borrow(expr) {
                return self.expr(&borrow, expected, loans);
            }
        }
        // 求出来的值（调用结果、字符串拼接、if/match 的值）借给只读借用参数：先成为临时值，
        // 语句结束时释放，例如 `string.parse_i64(string.trim(&line))`、`total(&xs.slice(0, 2))`。
        if let Some(owned) = temporary_borrow(expected) {
            if produces_value(expr) && self.value_type(expr).is_ok_and(|ty| ty == owned) {
                return self.expr(expr, owned, loans);
            }
            // 列表字面量借给 `&[T]`：`", ".join(["a", "b"])`、`total([1, 2])`。
            if matches!(expr, Expr::Array(_)) && matches!(owned, Type::List(_)) {
                return self.expr(expr, owned, loans);
            }
        }
        match expr {
            Expr::FnRef(name) => {
                let generic = self
                    .functions
                    .get(name)
                    .is_some_and(|function| !function.generics.is_empty());
                let name = &match expected {
                    Type::Func(id) | Type::FuncRef(id) if generic => {
                        generic_value(self.functions, name, id)?
                    }
                    _ => name.clone(),
                };
                let Type::Func(found) = self.fn_value(name)? else {
                    unreachable!("function value")
                };
                let short = name.rsplit(':').next().unwrap_or(name);
                match expected {
                    Type::Func(id) | Type::FuncRef(id) if id == found => Ok(()),
                    Type::Func(id) | Type::FuncRef(id)
                        if func_error(found).is_some() && func_error(id).is_none() =>
                    {
                        Err(format!(
                            "type mismatch: {short} can fail, but the expected function type has no !E; add the error type, such as fn(&string) -> i64!string"
                        ))
                    }
                    _ => Err(format!(
                        "type mismatch: {short} has a different function type"
                    )),
                }
            }
            Expr::Variable(name) if matches!(expected, Type::Func(_) | Type::FuncRef(_)) => {
                if let Some(captures) = self.closures.get(name).cloned() {
                    self.capture_loans(name, &captures, loans)?;
                }
                match (self.binding(name)?.ty, expected) {
                    (Type::Func(a), Type::Func(b) | Type::FuncRef(b))
                    | (Type::FuncRef(a), Type::FuncRef(b))
                        if a == b =>
                    {
                        Ok(())
                    }
                    (Type::FuncRef(_), Type::Func(_)) if self.closures.contains_key(name) => {
                        Err(format!(
                            "borrow escape: {name} uses local variables of this function, so it can only be called or passed as an argument, not stored or returned"
                        ))
                    }
                    (Type::FuncRef(_), Type::Func(_)) => Err(format!(
                        "borrow escape: {name} may capture local variables of its caller; it can only be called or passed as an argument"
                    )),
                    _ => Err(format!("type mismatch: {name} has a different function type")),
                }
            }
            Expr::Lambda(params, result, body) => {
                let hint = match expected {
                    Type::Func(id) | Type::FuncRef(id) => {
                        let (types, result) = func_sig(id);
                        Some((types, Some(result), func_error(id)))
                    }
                    _ => {
                        return Err(
                            "type mismatch: an anonymous fn is a function value, but another type is expected"
                                .into(),
                        )
                    }
                };
                let (id, captures) = self.lambda(params, *result, body, hint)?;
                if !matches!(expected, Type::Func(want) | Type::FuncRef(want) if want == id) {
                    return Err("type mismatch: this fn has a different function type".into());
                }
                if matches!(expected, Type::Func(_)) && !captures.is_empty() {
                    return Err(format!(
                        "borrow escape: this fn uses the local variable {}, so it can only be passed directly as an argument or bound with let",
                        captures[0].0
                    ));
                }
                self.capture_loans("this fn", &captures, loans)
            }
            Expr::Apply(path, args) => self.apply(path, args, expected, loans),
            Expr::Invoke(callee, args) => {
                let ty = self.value_type(callee)?;
                let (Type::Func(id) | Type::FuncRef(id)) = ty else {
                    return Err("type mismatch: only a function value can be called".into());
                };
                if func_error(id).is_some() {
                    return Err(
                        "fallible call this function value must be handled with ? or catch".into(),
                    );
                }
                let (params, result) = func_sig(id);
                // 被调用的值先求出来（函数值按值复制），实参在同一个调用帧里检查。
                let mut frame = loans.clone();
                self.expr(callee, ty, &mut frame)?;
                let callee = Function {
                    public: true,
                    params: params.into_iter().map(|ty| (String::new(), ty)).collect(),
                    result,
                    error: None,
                    body: Vec::new(),
                    position: self.position,
                    builtin: false,
                    generics: Vec::new(),
                    external: false,
                    wrapping: false,
                };
                self.call("this function value", &callee, args, expected, &mut frame)
            }
            Expr::Method(..) if self.field_call(expr).is_some() => {
                let (path, args) = self.field_call(expr).expect("field call");
                self.apply(&path, &args, expected, loans)
            }
            _ if self.record_call(expr) => {
                let (name, args) = self.call_target(expr)?.expect("record method");
                let callee = self.functions[&name].clone();
                if callee.error.is_some() {
                    return Err(format!("fallible call {name} must be handled with ?"));
                }
                self.call(&name, &callee, &args, expected, loans)
            }
            Expr::Record(name, _)
                if self
                    .structs
                    .get(name)
                    .is_some_and(|s| !s.variants.is_empty()) =>
            {
                let short = name.rsplit(':').next().unwrap_or(name);
                Err(format!(
                    "type mismatch: {short} is an enum; construct a variant with {short}.Variant(...)"
                ))
            }
            Expr::Variant(name, variant, args) => {
                let short = name.rsplit(':').next().unwrap_or(name);
                if self
                    .structs
                    .get(name)
                    .ok_or_else(|| format!("unknown struct {name}"))?
                    .variants
                    .is_empty()
                {
                    return Err(format!("type mismatch: {short} is a struct, not an enum"));
                }
                let (id, declaration) = self.constructor(expr, Some(expected))?;
                if expected != Type::Record(id) {
                    return Err(format!("type mismatch: {short}.{variant} is a {short}"));
                }
                let (_, first, count) = declaration
                    .variants
                    .iter()
                    .find(|(other, _, _)| other == variant)
                    .ok_or_else(|| format!("unknown variant {variant} of {short}"))?;
                if args.len() != *count {
                    return Err(format!(
                        "argument count mismatch for {short}.{variant}: it takes {count} value(s)"
                    ));
                }
                let fields = declaration.fields[*first..first + count].to_vec();
                for (arg, (_, ty)) in args.iter().zip(&fields) {
                    self.expr(arg, *ty, loans)?;
                }
                Ok(())
            }
            Expr::Match(scrutinee, arms) => {
                if expected.is_reference() || matches!(expected, Type::ArrayInt(_) | Type::Unit) {
                    return Err(
                        "type mismatch: a match expression produces an owned value, not a borrow or fixed array"
                            .into(),
                    );
                }
                let (id, root) = self.scrutinee(scrutinee, loans)?;
                self.arms_cover(id, arms.iter().map(|(pattern, _)| pattern))?;
                let before = self.locals.clone();
                let mut states = Vec::new();
                for (pattern, value) in arms {
                    self.locals = before.clone();
                    let bindings = self.arm_bindings(id, pattern)?;
                    for (name, ty) in &bindings {
                        self.locals.insert(
                            name.clone(),
                            Binding {
                                ty: *ty,
                                mutable: false,
                                live: true,
                            },
                        );
                    }
                    self.frozen.extend(root.clone());
                    self.expr(value, expected, &mut loans.clone())?;
                    if root.is_some() {
                        self.frozen.pop();
                    }
                    for (name, _) in &bindings {
                        self.locals.remove(name);
                    }
                    states.push(self.locals.clone());
                }
                self.locals = before;
                for (name, binding) in &mut self.locals {
                    binding.live &= states.iter().all(|state| state[name].live);
                }
                Ok(())
            }
            Expr::Record(_, fields) => {
                let (id, declaration) = self.constructor(expr, Some(expected))?;
                if expected != Type::Record(id) {
                    return Err("struct type mismatch".into());
                }
                let declaration = declaration.fields.clone();
                let mut seen = BTreeSet::new();
                for (field, value) in fields {
                    if !seen.insert(field) {
                        return Err(format!("duplicate initializer {field}"));
                    }
                    let ty = declaration
                        .iter()
                        .find(|(n, _)| n == field)
                        .ok_or_else(|| format!("unknown field {field}"))?
                        .1;
                    self.expr(value, ty, loans)?;
                }
                if seen.len() != declaration.len() {
                    return Err("missing struct field".into());
                }
                Ok(())
            }
            Expr::MapLit(entries) => {
                let Type::Map(key, value) = expected else {
                    return Err("type mismatch: unexpected map literal".into());
                };
                // 字面量里的键和值都移入字典。
                for (k, v) in entries {
                    self.expr(k, key.ty(), loans)?;
                    self.expr(v, value.ty(), loans)?;
                }
                Ok(())
            }
            Expr::Array(values) if matches!(expected, Type::List(_)) => {
                let Type::List(elem) = expected else {
                    unreachable!()
                };
                for value in values {
                    self.expr(value, elem.ty(), loans)?;
                }
                Ok(())
            }
            Expr::Array(values) => {
                if !matches!(expected, Type::ArrayInt(n) if n == values.len()) {
                    return Err("array length or element type mismatch".into());
                }
                for value in values {
                    self.expr(value, Type::Int, loans)?;
                }
                Ok(())
            }
            Expr::Index(local, index) if self.place(local)?.1.container().is_some() => {
                let (key, elem) = Self::subscript(self.container(local, false, loans)?);
                if elem == Elem::Str {
                    return Err(format!(
                        "cannot move a string out of {local}[..]; borrow it with &{local}[..] or copy it with string.copy(&{local}[..])"
                    ));
                }
                if let Elem::Record(_) = elem {
                    return Err(format!(
                        "cannot move a struct out of {local}[..]; borrow it with &{local}[..] or read a field with {local}[..].field"
                    ));
                }
                if let Elem::Nested(_) = elem {
                    return Err(format!(
                        "cannot move a list or map out of {local}[..]; borrow it with &{local}[..], call methods such as {local}[..].len(), or read an element with {local}[i][j]"
                    ));
                }
                if elem.ty() != expected {
                    return Err("type mismatch: element type differs".into());
                }
                self.expr(index, key, loans)
            }
            Expr::IndexField(path, index, field) => {
                let (key, target) = self.element(path, field)?;
                if target != expected {
                    return Err("field type mismatch".into());
                }
                if !matches!(target, Type::Int | Type::Float | Type::Bool) {
                    return Err(format!(
                        "owned field moves are not supported; borrow it with &{path}[..].{field}"
                    ));
                }
                self.container(path, false, loans)?;
                self.expr(index, key, loans)
            }
            Expr::Index(local, index) => {
                if expected != Type::Int {
                    return Err("type mismatch: array indexing returns i64".into());
                }
                let length = match self.binding(local)?.ty {
                    Type::ArrayInt(length) => length,
                    _ => return Err("index requires an array or list".into()),
                };
                if let Expr::Int(value) = index.as_ref() {
                    // 与 Python 相同，负下标从末尾数：-1 是最后一个。
                    if (*value as i128) < -(length as i128) || (*value as i128) >= length as i128 {
                        return Err("array index out of bounds".into());
                    }
                }
                self.expr(index, Type::Int, loans)
            }
            Expr::Field(_, _) => {
                if self.ty(expr)? != expected {
                    return Err("field type mismatch".into());
                }
                if matches!(
                    expected,
                    Type::Owned | Type::Record(_) | Type::List(_) | Type::Map(_, _)
                ) {
                    return Err("owned field moves are not supported; move the whole struct".into());
                }
                Ok(())
            }
            Expr::Variable(_name) if matches!(expected, Type::ArrayInt(_)) => {
                Err("array values cannot be moved or copied yet".into())
            }
            Expr::Variable(name) if matches!(expected, Type::Record(_)) => {
                let ty = self.binding(name)?.ty;
                if ty.is_reference() {
                    return Err(format!("cannot move borrowed value {name}"));
                }
                if ty != expected {
                    return Err("struct type mismatch".into());
                }
                if loans.contains_key(name) {
                    return Err("borrow conflict".into());
                }
                self.thawed(name)?;
                self.locals.get_mut(name).unwrap().live = false;
                Ok(())
            }
            Expr::Variable(name) if matches!(expected, Type::List(_) | Type::Map(_, _)) => {
                let ty = self.binding(name)?.ty;
                if ty.is_reference() {
                    return Err(format!("cannot move borrowed value {name}"));
                }
                if ty != expected {
                    return Err("type mismatch: container element types differ".into());
                }
                if loans.contains_key(name) {
                    return Err(format!(
                        "borrow conflict: cannot move {name} during a borrow"
                    ));
                }
                self.thawed(name)?;
                self.locals.get_mut(name).unwrap().live = false;
                Ok(())
            }
            Expr::StringEqual(left, right) => {
                if expected != Type::Bool {
                    return Err("equal returns bool".into());
                }
                let mut frame = loans.clone();
                self.expr(left, Type::Shared, &mut frame)?;
                self.expr(right, Type::Shared, &mut frame)
            }
            Expr::ByteLen(value) => {
                if expected != Type::Int {
                    return Err("byte_len returns i64".into());
                }
                self.expr(value, Type::Shared, &mut loans.clone())
            }
            Expr::CharLen(value) => {
                if expected != Type::Int {
                    return Err("char_len returns i64".into());
                }
                self.expr(value, Type::Shared, &mut loans.clone())
            }
            Expr::Format(segments) if expected == Type::Owned => {
                for segment in segments {
                    // 结构体、列表与字典按 println 的格式插入，同样只读。
                    let value = self.value_type(segment)?;
                    if compound(value) {
                        self.read(segment, value, loans)?;
                        continue;
                    }
                    // 插值只读取变量、字段与元素，不移动；其他表达式求值后插入。
                    let ty = self.ty(segment)?;
                    if !matches!(
                        ty,
                        Type::Owned
                            | Type::Shared
                            | Type::Mutable
                            | Type::Int
                            | Type::Float
                            | Type::Bool
                    ) {
                        return Err("cannot interpolate this value".into());
                    }
                    match segment {
                        Expr::Text(_) | Expr::Variable(_) | Expr::Field(_, _) => {}
                        _ if is_text(ty) => self.text_operand(segment, loans)?,
                        _ => self.expr(segment, ty, &mut loans.clone())?,
                    }
                }
                Ok(())
            }
            Expr::If(condition, yes, no) => {
                if expected.is_reference() || matches!(expected, Type::ArrayInt(_) | Type::Unit) {
                    return Err(
                        "type mismatch: an if expression produces an owned value, not a borrow or fixed array"
                            .into(),
                    );
                }
                self.expr(condition, Type::Bool, loans)?;
                let before = self.locals.clone();
                self.expr(yes, expected, &mut loans.clone())?;
                let after_yes = std::mem::replace(&mut self.locals, before);
                self.expr(no, expected, &mut loans.clone())?;
                for (name, binding) in &mut self.locals {
                    binding.live &= after_yes.get(name).is_none_or(|other| other.live);
                }
                Ok(())
            }
            Expr::Text(_) if expected == Type::Owned => Ok(()),
            // 字面量与插值可以直接传给 `&string` 参数：借用一个只活到语句结束的临时字符串。
            Expr::Text(_) | Expr::Format(_) if expected == Type::Shared => {
                self.expr(expr, Type::Owned, loans)
            }
            Expr::Int(_) if expected == Type::Int => Ok(()),
            Expr::Float(_) if expected == Type::Float => Ok(()),
            Expr::Bool(_) if expected == Type::Bool => Ok(()),
            Expr::Int(value) if expected == Type::Float => Err(format!(
                "type mismatch: expected f64, found the integer {value}; write {value}.0"
            )),
            Expr::Float(_) if expected == Type::Int => {
                Err("type mismatch: expected i64, found an f64 literal; convert with i64(x)".into())
            }
            Expr::Variable(name) if matches!(expected, Type::Int | Type::Float | Type::Bool) => {
                match (self.binding(name)?.ty, expected) {
                    (found, expected) if found == expected => Ok(()),
                    (Type::Int, Type::Float) => Err(format!(
                        "type mismatch: {name} is i64 but f64 is expected; convert with f64({name})"
                    )),
                    (Type::Float, Type::Int) => Err(format!(
                        "type mismatch: {name} is f64 but i64 is expected; convert with i64({name})"
                    )),
                    _ => Err("type mismatch".into()),
                }
            }
            Expr::Unary(op, value) => {
                let ty = match op {
                    '!' => Type::Bool,
                    '~' => Type::Int,
                    'f' => {
                        if expected != Type::Float {
                            return Err("type mismatch".into());
                        }
                        return self.expr(value, Type::Int, loans);
                    }
                    _ => numeric(self.ty(value)?),
                };
                if *op == '~' && self.ty(value)? != Type::Int {
                    return Err("type mismatch: ~ requires an i64 operand".into());
                }
                if expected != ty {
                    return Err("type mismatch".into());
                }
                self.expr(value, ty, loans)
            }
            Expr::Binary(op, left, right) => {
                if matches!(op.as_str(), "<" | "<=" | ">" | ">=")
                    && compound(self.value_type(left)?)
                {
                    return Err(format!(
                        "type mismatch: {op} compares i64, f64, bool and string values; structs, enums, lists and maps only support == and !="
                    ));
                }
                // 结构体、枚举、列表、字典的 == / !=：逐项比较，两边都只读，不移动。一边自己看不出
                // 类型时（`x == Option.None`）按另一边的类型构造。
                let shared = match (self.value_type(left), self.value_type(right)) {
                    (Ok(ty), other) | (other, Ok(ty)) if compound(ty) => {
                        if other.as_ref().is_ok_and(|other| *other != ty) {
                            return Err(format!(
                                "type mismatch: {op} compares two values of the same type"
                            ));
                        }
                        Some(ty)
                    }
                    _ => None,
                };
                if let (true, Some(ty)) = (matches!(op.as_str(), "==" | "!="), shared) {
                    if expected != Type::Bool {
                        return Err("type mismatch".into());
                    }
                    self.read(left, ty, loans)?;
                    return self.read(right, ty, loans);
                }
                if bitwise(op) {
                    if self.ty(left)? != Type::Int || self.ty(right)? != Type::Int {
                        return Err(format!(
                            "type mismatch: {op} requires i64 operands; combine conditions with and, or, not"
                        ));
                    }
                    if expected != Type::Int {
                        return Err("type mismatch".into());
                    }
                    self.expr(left, Type::Int, loans)?;
                    return self.expr(right, Type::Int, loans);
                }
                if let Some(error) = self.mixed(op, left, right) {
                    return Err(error);
                }
                let ty = self.ty(expr)?;
                if expected != ty {
                    return Err(match (ty, expected) {
                        (Type::Int, Type::Float) => "type mismatch: this i64 expression is used as f64; convert it with f64(...)".into(),
                        (Type::Float, Type::Int) if op == "/" => "type mismatch: / always gives f64 (as in Python); use // for integer division".into(),
                        (Type::Float, Type::Int) => "type mismatch: this f64 expression is used as i64; convert it with i64(...)".into(),
                        _ => "type mismatch".into(),
                    });
                }
                if matches!(op.as_str(), "&&" | "||") {
                    self.expr(left, Type::Bool, loans)?;
                    let before = self.locals.clone();
                    self.expr(right, Type::Bool, &mut loans.clone())?;
                    for (name, binding) in &mut self.locals {
                        binding.live &= before[name].live;
                    }
                    return Ok(());
                }
                let comparison = matches!(op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=");
                if (op == "+" || comparison) && is_text(self.ty(left)?) {
                    // 字符串拼接与比较：两边都只读，不移动变量。
                    self.text_operand(left, loans)?;
                    return self.text_operand(right, loans);
                }
                let operand = if matches!(op.as_str(), "==" | "!=") {
                    self.ty(left)?
                } else {
                    numeric(self.ty(left)?)
                };
                if !matches!(operand, Type::Int | Type::Float | Type::Bool) {
                    return Err("comparison requires i64, f64, bool or string; structs, enums, lists and maps only support == and !=".into());
                }
                self.expr(left, operand, loans)?;
                self.expr(right, operand, loans)
            }
            Expr::Variable(name) if expected == Type::Owned => {
                // 借用的字符串（参数、for 循环变量、借用局部）：复制一份，不移动。
                if matches!(self.binding(name)?.ty, Type::Shared | Type::Mutable) {
                    return Ok(());
                }
                if self.binding(name)?.ty != Type::Owned {
                    return Err(format!("cannot move borrowed value {name}"));
                }
                if loans.contains_key(name) {
                    return Err(format!(
                        "borrow conflict: cannot move {name} during a borrow"
                    ));
                }
                self.thawed(name)?;
                self.locals.get_mut(name).unwrap().live = false;
                Ok(())
            }
            Expr::Variable(name) | Expr::Borrow(name, _) if expected.is_reference() => {
                // 借用的目标：`&string` 借字符串，`&[T]`、`&{K: V}` 借同类型的容器。
                let (want, Some(want_mutable)) = expected.borrowable().expect("reference type")
                else {
                    unreachable!("reference types are borrows")
                };
                let (root, ty) = self.place(name)?;
                // 位置本身已是借用（参数、for 循环变量）时，记下它是否允许修改。
                let Some((base, by_ref)) = ty.borrowable() else {
                    return Err("scalar references are not supported".into());
                };
                if base != want {
                    return Err(format!("borrow type mismatch for {name}"));
                }
                let requested = match expr {
                    Expr::Borrow(_, mutable) => *mutable,
                    _ if by_ref.is_some() => want_mutable,
                    _ => {
                        return Err(format!(
                            "expected explicit borrow of {name}; write &mut {name} to let the function change it"
                        ))
                    }
                };
                if requested != want_mutable {
                    return Err(format!("borrow type mismatch for {name}"));
                }
                if requested && !by_ref.unwrap_or(self.writable(&root)) {
                    return Err(format!("cannot mutably borrow immutable value {name}"));
                }
                if requested {
                    self.thawed(&root)?;
                }
                if loans
                    .get(&root)
                    .is_some_and(|old| *old == Type::Mutable || requested)
                {
                    return Err(format!("borrow conflict: {name}"));
                }
                // ponytail: loans cover the root; split field loans if disjoint mutation is needed.
                loans.insert(
                    root,
                    if requested {
                        Type::Mutable
                    } else {
                        Type::Shared
                    },
                );
                Ok(())
            }
            Expr::BorrowIndex(path, index, field, mutable) => {
                if !expected.is_reference() {
                    return Err("borrow escape: references only belong in call arguments".into());
                }
                let (key, target) = self.element(path, field)?;
                let Some(borrowed) = borrow_of(target, *mutable) else {
                    return Err("scalar references are not supported".into());
                };
                if borrowed != expected {
                    return Err(format!("borrow type mismatch for {path}[..]"));
                }
                self.container(path, *mutable, loans)?;
                self.expr(index, key, loans)?;
                let root = path.split('.').next().unwrap_or(path).to_owned();
                loans.insert(
                    root,
                    if *mutable {
                        Type::Mutable
                    } else {
                        Type::Shared
                    },
                );
                Ok(())
            }
            Expr::Method(path, method, args) => {
                let (mutable, params, result) = self.method(path, method)?;
                if result != expected {
                    return Err(format!("return type mismatch for {path}.{method}"));
                }
                if params.len() != args.len() {
                    return Err(format!("argument count mismatch for {path}.{method}"));
                }
                // 参数先求值（与 C 侧一致），再借用接收者。
                let mut frame = loans.clone();
                for (arg, ty) in args.iter().zip(&params) {
                    self.expr(arg, *ty, &mut frame)?;
                }
                // 闭包在整个调用期间都读它捕获的变量：方法要改的容器不能被它读。
                let root = path.split('.').next().unwrap_or(path);
                if mutable
                    && args.iter().any(|arg| {
                        let mut roots = BTreeSet::new();
                        if let Expr::Lambda(_, _, body) = arg {
                            block_roots(body, &mut roots);
                        }
                        roots.contains(root)
                    })
                {
                    return Err(format!(
                        "borrow conflict: the fn reads {root} while .{method} changes it"
                    ));
                }
                self.container(path, mutable, loans)?;
                Ok(())
            }
            // 调用结果（临时结构体）借给 `&T` 参数：只活到语句结束，比如 `make().norm()`。
            Expr::Call(_, _) | Expr::Try(_) | Expr::Chain(_, _, _)
                if matches!(expected, Type::RecordRef(_, _)) =>
            {
                let owned = expected.borrowable().expect("reference").0;
                self.expr(expr, owned, loans)
            }
            Expr::Tuple(items) => {
                let fields = match expected {
                    Type::Record(id) => tuple_fields(id),
                    _ => None,
                }
                .ok_or("type mismatch: a tuple is not expected here")?;
                if fields.len() != items.len() {
                    return Err(format!(
                        "type mismatch: expected a tuple of {} items, found {}",
                        fields.len(),
                        items.len()
                    ));
                }
                for (item, field) in items.iter().zip(fields) {
                    self.expr(item, field, loans)?;
                }
                Ok(())
            }
            Expr::Chain(receiver, method, args) => {
                // 先求值调用结果（临时值，语句结束时释放），再求实参，然后调用方法。
                let ty = self.ty(receiver)?;
                let (_, params, result) =
                    method_signature(ty, method).ok_or_else(|| self.chain_error(ty, method))?;
                if !matches!(ty, Type::List(_) | Type::Map(_, _)) {
                    return Err(self.chain_error(ty, method));
                }
                if result != expected {
                    return Err(format!("return type mismatch for .{method}"));
                }
                if params.len() != args.len() {
                    return Err(format!("argument count mismatch for .{method}"));
                }
                self.expr(receiver, ty, loans)?;
                let mut frame = loans.clone();
                for (arg, ty) in args.iter().zip(&params) {
                    self.expr(arg, *ty, &mut frame)?;
                }
                Ok(())
            }
            Expr::ElementMethod(path, index, field, method, args) => {
                let (key, mutable, params, result) = self.element_method(path, field, method)?;
                if result != expected {
                    return Err(format!(
                        "return type mismatch for {path}[..].{field}.{method}"
                    ));
                }
                if params.len() != args.len() {
                    return Err(format!(
                        "argument count mismatch for {path}[..].{field}.{method}"
                    ));
                }
                // 参数、下标依次求值，最后借用接收者所在的容器。
                let mut frame = loans.clone();
                for (arg, ty) in args.iter().zip(&params) {
                    self.expr(arg, *ty, &mut frame)?;
                }
                self.expr(index, key, loans)?;
                self.container(path, mutable, loans)?;
                Ok(())
            }
            Expr::Call(name, args) if name == COPY => {
                let [arg] = args.as_slice() else {
                    return Err(
                        "argument count mismatch for copy: copy takes one borrowed value, such as copy(&x)"
                            .into(),
                    );
                };
                let ty = self.value_type(arg)?;
                match ty {
                    Type::Owned | Type::Record(_) | Type::List(_) | Type::Map(_, _) => {}
                    Type::Int | Type::Float | Type::Bool if self.generic => {
                        if expected != ty {
                            return Err("type mismatch: copy returns a value of the borrowed type".into());
                        }
                        let value = match arg {
                            Expr::Borrow(path, _) => match path.split_once('.') {
                                Some((root, field)) => Expr::Field(root.into(), field.into()),
                                None => Expr::Variable(path.clone()),
                            },
                            other => other.clone(),
                        };
                        return self.expr(&value, ty, loans);
                    }
                    Type::Int | Type::Float | Type::Bool => {
                        return Err("type mismatch: copy is for strings, lists, maps, structs and enums; i64, f64 and bool values are already copied by assignment, such as let b = a".into())
                    }
                    _ => {
                        return Err(
                            "type mismatch: copy is for strings, lists, maps, structs and enums"
                                .into(),
                        )
                    }
                }
                let borrowed = matches!(
                    arg,
                    Expr::Borrow(_, false) | Expr::BorrowIndex(_, _, _, false)
                ) || self.ty(arg)?.is_reference();
                if !borrowed {
                    return Err(
                        "expected explicit borrow: write copy(&x), copy(&xs[i]) or copy(&p.field)"
                            .into(),
                    );
                }
                if expected != ty {
                    return Err("type mismatch: copy returns a value of the borrowed type".into());
                }
                self.read(arg, ty, loans)
            }
            Expr::Call(name, args) => {
                let name = &self.callee(name, args);
                if let Some(error) = self.generic_error(name, args) {
                    return Err(error);
                }
                let callee = self
                    .functions
                    .get(name)
                    .ok_or_else(|| format!("unknown function {name}"))?
                    .clone();
                // 会失败的调用必须先被 `?` 或 `catch` 处理，错误值不会静默丢弃。
                if callee.error.is_some() {
                    return Err(format!("fallible call {name} must be handled with ?"));
                }
                self.call(name, &callee, args, expected, loans)
            }
            Expr::Try(inner) => {
                if let Some((label, error, _)) = self.value_call(inner, expected, loans)? {
                    if Some(error) != self.error {
                        return Err(format!(
                            "? cannot propagate the error of {label} from this function"
                        ));
                    }
                    return Ok(());
                }
                let Some((name, args)) = self.call_target(inner)? else {
                    return Err("? requires a function call".into());
                };
                let (name, args) = (&name, &args);
                let callee = self
                    .functions
                    .get(name)
                    .ok_or_else(|| format!("unknown function {name}"))?
                    .clone();
                let error = callee
                    .error
                    .ok_or_else(|| format!("? requires a fallible call, but {name} cannot fail"))?;
                if Some(error) != self.error {
                    return Err(format!(
                        "? cannot propagate the error of {name} from this function"
                    ));
                }
                self.call(name, &callee, args, expected, loans)
            }
            Expr::Catch(inner, binding, block) => {
                let discarded = std::mem::take(&mut self.discarded);
                let (name, error, success) = match self.value_call(inner, expected, loans)? {
                    Some(found) => found,
                    None => {
                        let Some((name, args)) = self.call_target(inner)? else {
                            return Err("catch requires a function call".into());
                        };
                        let callee = self
                            .functions
                            .get(&name)
                            .ok_or_else(|| format!("unknown function {name}"))?
                            .clone();
                        let error = callee.error.ok_or_else(|| {
                            format!("catch requires a fallible call, but {name} cannot fail")
                        })?;
                        self.call(&name, &callee, &args, expected, loans)?;
                        (name, error, callee.result)
                    }
                };
                let name = &name;
                // 错误值只在 catch 的代码块里可见。
                if self.locals.contains_key(binding) {
                    return Err(format!("duplicate binding {binding}"));
                }
                self.locals.insert(
                    binding.clone(),
                    Binding {
                        ty: error,
                        mutable: false,
                        live: true,
                    },
                );
                // `f() catch e: default`：块只有一个表达式、类型合乎成功值时，它就是出错时的值。
                if let Some(value) = catch_fallback(block) {
                    if success != Type::Unit
                        && self.ty(value).is_ok_and(|ty| fallback_fits(ty, success))
                    {
                        let checked = self.expr(value, success, loans);
                        self.locals.remove(binding);
                        return checked;
                    }
                }
                let returned = self.block(block, self.result)?;
                self.locals.remove(binding);
                // 在循环里用 break / continue 收尾也算走完：控制流离开了这次调用所在的语句。
                let jumped = matches!(
                    block.last(),
                    Some((Instruction::Break | Instruction::Continue, _))
                );
                // 没有成功值的调用（`fs.remove(p) catch e:`）或单独成句、值没人用的调用：处理完错误
                // 可以接着往下走，块里移走的变量之后照旧算移走。值要用时块必须离开，否则表达式没有值。
                if !returned && !jumped && success != Type::Unit && !discarded {
                    return Err(format!("catch block for {name} must return a value"));
                }
                Ok(())
            }
            Expr::Borrow(_, _) => {
                Err("borrow escape: references only belong in call arguments".into())
            }
            _ => Err("string type mismatch".into()),
        }
    }
    /// 函数值（局部变量、参数、结构体字段）的错误类型。
    fn value_error(&self, path: &str) -> Result<Option<Type>, String> {
        match self.place(path)?.1 {
            Type::Func(id) | Type::FuncRef(id) => Ok(func_error(id)),
            _ => Ok(None),
        }
    }
    /// `?`、`catch` 包着的是对函数值的调用（`f(x)?`、`steps[0](x) catch e:`）：检查调用，返回
    /// （显示用的名字, 错误类型, 结果类型）。不是函数值调用时返回 None，按普通调用处理。
    fn value_call(
        &mut self,
        inner: &Expr,
        expected: Type,
        loans: &mut BTreeMap<String, Type>,
    ) -> Result<Option<(String, Type, Type)>, String> {
        let (label, id, args) = match inner {
            Expr::Apply(path, args) => match self.place(path)?.1 {
                Type::Func(id) | Type::FuncRef(id) => (path.clone(), id, args),
                _ => return Ok(None),
            },
            Expr::Method(..) if self.field_call(inner).is_some() => {
                let (path, _) = self.field_call(inner).expect("field call");
                let Expr::Method(_, _, args) = inner else {
                    unreachable!("field call")
                };
                match self.place(&path)?.1 {
                    Type::Func(id) | Type::FuncRef(id) => (path, id, args),
                    _ => return Ok(None),
                }
            }
            Expr::Invoke(callee, args) => match self.value_type(callee)? {
                Type::Func(id) | Type::FuncRef(id) => {
                    self.expr(callee, Type::Func(id), loans)?;
                    ("this function value".to_owned(), id, args)
                }
                _ => return Err("type mismatch: only a function value can be called".into()),
            },
            _ => return Ok(None),
        };
        let Some(error) = func_error(id) else {
            return Err(format!("{label} cannot fail, so it needs no ? or catch"));
        };
        if let Some(captures) = self.closures.get(&label).cloned() {
            self.capture_loans(&label, &captures, loans)?;
        }
        let (params, result) = func_sig(id);
        let callee = Function {
            public: true,
            params: params.into_iter().map(|ty| (String::new(), ty)).collect(),
            result,
            error: Some(error),
            body: Vec::new(),
            position: self.position,
            builtin: false,
            generics: Vec::new(),
            external: false,
            wrapping: false,
        };
        self.call(&label, &callee, args, expected, loans)?;
        Ok(Some((label, error, result)))
    }
    /// 调用函数值：参数与返回值按它的函数类型检查。
    fn apply(
        &mut self,
        path: &str,
        args: &[Expr],
        expected: Type,
        loans: &mut BTreeMap<String, Type>,
    ) -> Result<(), String> {
        let (params, result) = self.apply_sig(path)?;
        if self.value_error(path)?.is_some() {
            return Err(format!(
                "fallible call {path} must be handled with ? or catch"
            ));
        }
        if let Some(captures) = self.closures.get(path).cloned() {
            self.capture_loans(path, &captures, loans)?;
        }
        let callee = Function {
            public: true,
            params: params.into_iter().map(|ty| (String::new(), ty)).collect(),
            result,
            error: None,
            body: Vec::new(),
            position: self.position,
            builtin: false,
            generics: Vec::new(),
            external: false,
            wrapping: false,
        };
        self.call(path, &callee, args, expected, loans)
    }
    fn call(
        &mut self,
        name: &str,
        callee: &Function,
        args: &[Expr],
        expected: Type,
        loans: &mut BTreeMap<String, Type>,
    ) -> Result<(), String> {
        if callee.result != expected {
            return Err(format!("return type mismatch for {name}"));
        }
        if callee.params.len() != args.len() {
            return Err(format!("argument count mismatch for {name}"));
        }
        // Nested calls inherit outer loans; their own loans end when they return.
        let mut frame = loans.clone();
        for (arg, (_, ty)) in args.iter().zip(&callee.params) {
            self.expr(arg, *ty, &mut frame)?;
        }
        Ok(())
    }
    /// `let x = &place` 的类型：被借用的值的只读借用（数字与布尔不能借用）。
    fn local_borrow(&self, expr: &Expr) -> Result<Type, String> {
        let owned = match expr {
            Expr::Borrow(path, _) => self.place(path)?.1,
            Expr::BorrowIndex(path, _, field, _) => self.element(path, field)?.1,
            _ => unreachable!("borrow expression"),
        };
        let owned = owned.borrowable().map_or(owned, |(owned, _)| owned);
        borrow_of(owned, false).ok_or_else(|| {
            "scalar references are not supported; copy the number instead, such as let n = xs[i]"
                .to_owned()
        })
    }
    fn block(&mut self, body: &Block, result: Type) -> Result<bool, String> {
        let outer_names: BTreeSet<String> = self.locals.keys().cloned().collect();
        // 代码块里 `let x = &...` 冻结的根到块尾解冻。
        let outer_frozen = self.frozen.len();
        let mut returned = false;
        let mut terminated = false;
        for (instruction, position) in body {
            self.position = *position;
            if returned || terminated {
                return Err("unreachable statement after return".into());
            }
            let mut loans = BTreeMap::new();
            match instruction {
                Instruction::If(condition, yes, no) => {
                    self.expr(condition, Type::Bool, &mut loans)?;
                    let before = self.locals.clone();
                    let yes_returns = self.block(yes, result)?;
                    let yes_state = self.locals.clone();
                    self.locals = before;
                    let no_returns = self.block(no, result)?;
                    for (name, binding) in &mut self.locals {
                        binding.live = if yes_returns {
                            binding.live
                        } else if no_returns {
                            yes_state[name].live
                        } else {
                            binding.live && yes_state[name].live
                        };
                    }
                    returned = yes_returns && no_returns;
                }
                Instruction::While(condition, nested) => {
                    let before = self.locals.clone();
                    self.expr(condition, Type::Bool, &mut loans)?;
                    self.loops.push((before.clone(), condition.clone()));
                    let returns = self.block(nested, result)?;
                    self.loops.pop();
                    if !returns {
                        self.expr(condition, Type::Bool, &mut BTreeMap::new())?;
                        for (name, binding) in &before {
                            if binding.live && !self.locals[name].live {
                                return Err(format!(
                                    "loop moves value without reinitialization: {name}"
                                ));
                            }
                        }
                    }
                    self.locals = before;
                    self.expr(condition, Type::Bool, &mut BTreeMap::new())?;
                    returned = false;
                }
                Instruction::Each(local, second, path, mutable, nested) => {
                    // 列表遍历元素（两个名字时先是下标），字典遍历键（两个名字时还有值），都按插入顺序。
                    // `for ch in s:`：逐个字符（与 Python 相同），按字符串列表的规则绑定；字符串被冻结。
                    let string = self.place(path).is_ok_and(|(_, ty)| is_string(ty));
                    if string && *mutable {
                        return Err(
                            "a string cannot be changed through a for loop; build a new string instead"
                                .into(),
                        );
                    }
                    if string {
                        self.expr(
                            &Expr::Borrow(path.clone(), false),
                            Type::Shared,
                            &mut loans.clone(),
                        )?;
                    }
                    let container = if string {
                        Ok(Type::List(Elem::Str))
                    } else {
                        self.container(path, *mutable, &loans)
                    }
                    .map_err(|error| {
                        if path.starts_with("$for") {
                            "type mismatch: for x in VALUE needs a list or map; to count, write a range such as for i in 0..n".to_owned()
                        } else if error.ends_with("is not a list or map") {
                            format!("{error}; to count, write for i in 0..{path}")
                        } else {
                            error
                        }
                    })?;
                    let names = each_bindings(container, second.is_some(), *mutable)?;
                    let bound: Vec<&String> = std::iter::once(local).chain(second).collect();
                    for (name, (ty, assignable)) in bound.iter().zip(names) {
                        if self.locals.contains_key(*name) {
                            return Err(format!("duplicate binding {name}"));
                        }
                        self.locals.insert(
                            (*name).clone(),
                            Binding {
                                ty,
                                mutable: assignable,
                                live: true,
                            },
                        );
                    }
                    let before = self.locals.clone();
                    let root = path.split('.').next().unwrap_or(path).to_owned();
                    self.loops.push((before.clone(), Expr::Bool(true)));
                    self.frozen.push(root);
                    let returns = self.block(nested, result)?;
                    self.frozen.pop();
                    self.loops.pop();
                    if !returns {
                        for (name, binding) in &before {
                            if binding.live && !self.locals[name].live {
                                return Err(format!(
                                    "loop moves value without reinitialization: {name}"
                                ));
                            }
                        }
                    }
                    self.locals = before;
                    for name in bound {
                        self.locals.remove(name);
                    }
                    returned = false;
                }
                Instruction::Match(scrutinee, arms) => {
                    let (id, root) = self.scrutinee(scrutinee, &mut loans)?;
                    self.arms_cover(id, arms.iter().map(|(pattern, _)| pattern))?;
                    let before = self.locals.clone();
                    let mut states = Vec::new();
                    for (pattern, body) in arms {
                        self.locals = before.clone();
                        let bindings = self.arm_bindings(id, pattern)?;
                        for (name, ty) in &bindings {
                            self.locals.insert(
                                name.clone(),
                                Binding {
                                    ty: *ty,
                                    mutable: false,
                                    live: true,
                                },
                            );
                        }
                        self.frozen.extend(root.clone());
                        let returns = self.block(body, result)?;
                        if root.is_some() {
                            self.frozen.pop();
                        }
                        for (name, _) in &bindings {
                            self.locals.remove(name);
                        }
                        states.push((returns, self.locals.clone()));
                    }
                    self.locals = before;
                    // 没有返回的分支各自的所有权状态取交集。
                    for (name, binding) in &mut self.locals {
                        binding.live = states
                            .iter()
                            .filter(|(returns, _)| !returns)
                            .all(|(_, state)| state[name].live);
                    }
                    returned = states.iter().all(|(returns, _)| *returns);
                }
                Instruction::For(local, start, end, nested) => {
                    self.expr(start, Type::Int, &mut loans)?;
                    self.expr(end, Type::Int, &mut loans)?;
                    if self.locals.contains_key(local) {
                        return Err(format!("duplicate binding {local}"));
                    }
                    self.locals.insert(
                        local.clone(),
                        Binding {
                            ty: Type::Int,
                            mutable: false,
                            live: true,
                        },
                    );
                    let before = self.locals.clone();
                    // 上界在进入循环前已求值，continue 不需要再检查任何条件。
                    self.loops.push((before.clone(), Expr::Bool(true)));
                    let returns = self.block(nested, result)?;
                    self.loops.pop();
                    if !returns {
                        for (name, binding) in &before {
                            if binding.live && !self.locals[name].live {
                                return Err(format!(
                                    "loop moves value without reinitialization: {name}"
                                ));
                            }
                        }
                    }
                    self.locals = before;
                    self.locals.remove(local);
                    returned = false;
                }
                Instruction::Break | Instruction::Continue => {
                    let (before, condition) = self
                        .loops
                        .last()
                        .cloned()
                        .ok_or("loop jump outside while")?;
                    // ponytail: preserve loop-entry owners even on break; merge exit states if move-and-break is needed.
                    for (name, binding) in &before {
                        if binding.live && !self.locals[name].live {
                            return Err(format!(
                                "loop jump moves value without reinitialization: {name}"
                            ));
                        }
                    }
                    if matches!(instruction, Instruction::Continue) {
                        self.expr(&condition, Type::Bool, &mut BTreeMap::new())?;
                    }
                    terminated = true;
                }
                Instruction::Let(local, mutable, annotation, expr) => {
                    if self.locals.contains_key(local) {
                        return Err(format!("duplicate binding {local}"));
                    }
                    self.closures.remove(local);
                    // `let entry = &entries[i]`、`let name = &p.name`：只读借用存进局部变量。从这里到代码块
                    // 结束，被借用的根冻结（不能改、不能移走，与 for 遍历相同）；借用不能返回或存进别处。
                    if let (
                        None,
                        Expr::Borrow(path, false) | Expr::BorrowIndex(path, _, _, false),
                    ) = (annotation, expr)
                    {
                        if *mutable {
                            return Err(format!(
                                "a borrowed local cannot be mut; write let {local} = &..."
                            ));
                        }
                        let ty = self.local_borrow(expr)?;
                        self.expr(expr, ty, &mut loans)?;
                        let root = path.split('.').next().unwrap_or(path).to_owned();
                        self.frozen.push(root);
                        self.locals.insert(
                            local.clone(),
                            Binding {
                                ty,
                                mutable: false,
                                live: true,
                            },
                        );
                        continue;
                    }
                    // 用到外层变量的匿名函数（含函数里定义的 fn）绑定到名字上：不借用，等到调用或
                    // 转传它时，再在那条语句里借用它用到的变量。它不能存起来或返回，活不过那些变量。
                    if let Expr::Lambda(params, result, body) = expr {
                        let mut roots = BTreeSet::new();
                        block_roots(body, &mut roots);
                        if roots.contains(local) {
                            return Err(format!(
                                "{local} cannot call itself: a fn declared inside a function is not recursive yet; declare {local} at the top level"
                            ));
                        }
                        let hint = match annotation {
                            Some(Type::Func(id) | Type::FuncRef(id)) => {
                                let (types, result) = func_sig(*id);
                                Some((types, Some(result), func_error(*id)))
                            }
                            Some(_) => {
                                return Err(
                                    "type mismatch: an anonymous fn is a function value, but another type is expected"
                                        .into(),
                                )
                            }
                            None => None,
                        };
                        let (id, captures) = self.lambda(params, *result, body, hint)?;
                        if annotation.is_some_and(|ty| !matches!(ty, Type::Func(want) | Type::FuncRef(want) if want == id)) {
                            return Err("type mismatch: this fn has a different function type".into());
                        }
                        if !captures.is_empty() {
                            if *mutable {
                                return Err(format!(
                                    "{local} uses local variables, so it cannot be declared mut; bind another fn with a new name"
                                ));
                            }
                            self.locals.insert(
                                local.clone(),
                                Binding {
                                    ty: Type::FuncRef(id),
                                    mutable: false,
                                    live: true,
                                },
                            );
                            self.closures.insert(local.clone(), captures);
                            continue;
                        }
                    }
                    let ty = match (annotation, loop_list(local, expr)) {
                        (Some(ty), _) => *ty,
                        // `for n in [3, 1, 2]:` 的隐藏变量是列表，元素类型看第一项。
                        (None, Some(first)) => Type::List(
                            elem_from(self.ty(first)?)
                                .ok_or("unsupported list element type in for loop")?,
                        ),
                        (None, None) => self.ty(expr)?,
                    };
                    if ty.is_reference() || ty == Type::Unit {
                        return Err("borrow escape: locals require an owned value".into());
                    }
                    self.expr(expr, ty, &mut loans)?;
                    self.locals.insert(
                        local.clone(),
                        Binding {
                            ty,
                            mutable: *mutable,
                            live: true,
                        },
                    );
                }
                Instruction::AssignIndex(local, index, expr)
                    if self.place(local)?.1.container().is_some() =>
                {
                    let container = self.place(local)?.1.container().expect("guarded");
                    let (key, elem) = Self::subscript(container);
                    self.expr(index, key, &mut loans)?;
                    self.container(local, true, &loans)?;
                    self.expr(expr, elem.ty(), &mut loans)?;
                    self.place(local)?;
                }
                Instruction::AssignIndexField(local, index, field, expr) => {
                    let (key, target) = self.element(local, field)?;
                    self.expr(index, key, &mut loans)?;
                    self.container(local, true, &loans)?;
                    self.expr(expr, target, &mut loans)?;
                    self.place(local)?;
                }
                Instruction::AssignIndex(local, index, expr) => {
                    if !self.binding(local)?.mutable {
                        return Err(format!("cannot assign immutable value {local}"));
                    }
                    if !matches!(self.binding(local)?.ty, Type::ArrayInt(_)) {
                        return Err("index requires i64 array".into());
                    }
                    self.expr(
                        &Expr::Index(local.clone(), Box::new(index.clone())),
                        Type::Int,
                        &mut loans,
                    )?;
                    self.expr(expr, Type::Int, &mut loans)?;
                    self.binding(local)?;
                }
                Instruction::AssignField(local, field, expr) => {
                    self.binding(local)?;
                    if !self.writable(local) {
                        return Err(format!(
                            "cannot assign immutable value {local}{}",
                            if self.locals[local].ty.is_reference() {
                                "; it is a shared borrow, declare the parameter as &mut and pass &mut"
                            } else {
                                ""
                            }
                        ));
                    }
                    self.thawed(local)?;
                    let ty = self.ty(&Expr::Field(local.clone(), field.clone()))?;
                    self.expr(expr, ty, &mut loans)?;
                    // Replacing a field cannot reinitialize a moved whole struct.
                    self.binding(local)?;
                }
                Instruction::Assign(local, expr) => {
                    let binding = self
                        .locals
                        .get(local)
                        .ok_or_else(|| format!("unknown variable {local}"))?;
                    let by_ref = binding.ty.is_reference();
                    if !(binding.mutable
                        || binding.ty.borrowable().is_some_and(|b| b.1 == Some(true)))
                    {
                        return Err(format!("cannot assign immutable value {local}"));
                    }
                    // 通过 `&mut` 参数赋值时写的是被借用的值。
                    let ty = if by_ref {
                        binding.ty.borrowable().expect("reference type").0
                    } else {
                        binding.ty
                    };
                    self.thawed(local)?;
                    if matches!(ty, Type::ArrayInt(_)) {
                        return Err("array values cannot be moved or copied yet".into());
                    }
                    self.expr(expr, ty, &mut loans)?;
                    self.locals.get_mut(local).unwrap().live = true;
                }
                Instruction::Print(expr) if self.value_type(expr).is_ok_and(compound) => {
                    let value = self.value_type(expr)?;
                    self.read(expr, value, &mut loans)?;
                }
                Instruction::Print(expr) => {
                    let ty = self.ty(expr)?;
                    if !matches!(
                        ty,
                        Type::Owned
                            | Type::Shared
                            | Type::Mutable
                            | Type::Int
                            | Type::Float
                            | Type::Bool
                    ) {
                        if matches!(ty, Type::Func(_) | Type::FuncRef(_)) {
                            return Err("type mismatch: function values cannot be printed; call the function and print its result".into());
                        }
                        return Err("cannot print void or a fixed array".into());
                    }
                    if let Expr::Variable(local) = expr {
                        self.binding(local)?;
                    } else if let Expr::Field(_, _) = expr {
                        self.ty(expr)?;
                    } else if matches!(expr, Expr::Index(_, _) | Expr::IndexField(_, _, _))
                        && ty == Type::Owned
                        && self.string_call(expr)?.is_none()
                    {
                        self.text_operand(expr, &mut loans)?;
                    } else if let Expr::BorrowIndex(_, _, _, _) = expr {
                        self.expr(expr, ty, &mut loans)?;
                    } else if let Expr::Borrow(local, mutable) = expr {
                        self.expr(
                            expr,
                            if *mutable {
                                Type::Mutable
                            } else {
                                Type::Shared
                            },
                            &mut loans,
                        )?;
                        self.binding(local.split('.').next().unwrap())?;
                    } else {
                        let ty = self.ty(expr)?;
                        self.expr(expr, ty, &mut loans)?;
                    }
                }
                Instruction::Call(expr) => {
                    let ty = if matches!(expr, Expr::Call(callee, _) if callee == COPY) {
                        self.ty(expr)?
                    } else if let Expr::Call(callee, args) = expr {
                        self.functions
                            .get(&self.callee(callee, args))
                            .ok_or_else(|| format!("unknown function {callee}"))?
                            .result
                    } else if matches!(
                        expr,
                        Expr::ByteLen(_)
                            | Expr::CharLen(_)
                            | Expr::StringEqual(_, _)
                            | Expr::Try(_)
                            | Expr::Catch(_, _, _)
                            | Expr::Method(_, _, _)
                            | Expr::ElementMethod(_, _, _, _, _)
                            | Expr::Chain(_, _, _)
                            | Expr::Apply(_, _)
                            | Expr::Invoke(_, _)
                    ) {
                        self.ty(expr)?
                    } else {
                        // 单独一行的值什么也不做；最常见的是多行 catch 块最后写了回退值。
                        return Err("expected function call: this line is only a value, so it does nothing; a catch block gives a fallback value only when the value is the whole block, such as f() catch e: 0".into());
                    };
                    self.discarded = matches!(expr, Expr::Catch(..));
                    let checked = self.expr(expr, ty, &mut loans);
                    self.discarded = false;
                    checked?;
                    if terminates(expr) {
                        returned = true;
                    }
                }
                Instruction::Unpack(names, mutable, expr) => {
                    let ty = self.ty(expr)?;
                    let fields = match ty {
                        Type::Record(id) => tuple_fields(id),
                        _ => None,
                    }
                    .ok_or("type mismatch: let (a, b) = ... needs a tuple value")?;
                    if fields.len() != names.len() {
                        return Err(format!(
                            "type mismatch: the tuple has {} items, but the pattern names {}",
                            fields.len(),
                            names.len()
                        ));
                    }
                    self.expr(expr, ty, &mut loans)?;
                    for (name, field) in names.iter().zip(fields) {
                        if name == "_" {
                            continue;
                        }
                        if self.locals.contains_key(name) {
                            return Err(format!("duplicate binding {name}"));
                        }
                        self.locals.insert(
                            name.clone(),
                            Binding {
                                ty: field,
                                mutable: *mutable,
                                live: true,
                            },
                        );
                    }
                }
                Instruction::Fail(expr) => {
                    let error = self
                        .error
                        .ok_or("fail requires a function that declares an error type")?;
                    self.expr(expr, error, &mut loans)?;
                    returned = true;
                }
                Instruction::Return(expr) => {
                    if let Some(expr) = expr {
                        if result == Type::Unit {
                            return Err("void function cannot return a value".into());
                        }
                        self.expr(expr, result, &mut loans)?;
                    } else if result != Type::Unit {
                        return Err("missing string return value".into());
                    }
                    returned = true;
                }
            }
        }
        // 块内声明的变量在退出时从 `locals` 删掉（它们不能逃出块），但编辑器要按位置查它们
        // 的类型，所以在删之前留一份快照。
        let declared: Vec<(String, Type)> = self
            .locals
            .iter()
            .filter(|(name, _)| !outer_names.contains(*name))
            .map(|(name, binding)| (name.clone(), binding.ty))
            .collect();
        for (name, ty) in declared {
            self.declared.insert(name, ty);
        }
        let outer: BTreeSet<_> = outer_names;
        self.locals.retain(|name, _| outer.contains(name));
        self.frozen.truncate(outer_frozen);
        Ok(returned)
    }
}

pub fn record_order(module: &Module, roots: &BTreeSet<String>) -> Result<Vec<String>, String> {
    fn visit(
        module: &Module,
        name: &str,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if done.contains(name) {
            return Ok(());
        }
        if !active.insert(name.into()) {
            return Err(format!("recursive struct layout: {name}"));
        }
        for (_, ty) in &module.structs[name].fields {
            // 还没链接进来的泛型实例下一轮检查再看。
            if let Type::Record(id) = ty {
                if let Some(child) = module.structs.keys().find(|name| record_id(name) == *id) {
                    visit(module, child, active, done, order)?;
                }
            }
        }
        active.remove(name);
        done.insert(name.into());
        order.push(name.into());
        Ok(())
    }
    let mut order = Vec::new();
    let mut active = BTreeSet::new();
    let mut done = BTreeSet::new();
    for name in roots {
        visit(module, name, &mut active, &mut done, &mut order)?;
    }
    Ok(order)
}

/// 多条诊断放进同一个错误字符串时用的分隔符（ASCII 记录分隔符），由命令行拆开逐条输出。
pub const DIAGNOSTIC_SEPARATOR: char = '\u{1e}';

/// 逐个函数独立检查：每个出错的函数报它的第一条错误，互不影响；全部检查完再一起返回。
pub fn check(module: &Module, entry: &str) -> Result<(), String> {
    // 泛型模板的字段里还有类型参数，不排布局；用到的实例各自排。
    let concrete = module
        .structs
        .iter()
        .filter(|(_, s)| s.generics.is_empty())
        .map(|(name, _)| name.clone())
        .collect();
    record_order(module, &concrete)?;
    // `zam test` 不需要 main：入口传空串。
    if !entry.is_empty() {
        let main = module.functions.get(entry).ok_or("missing main function")?;
        if !main.params.is_empty() || main.result != Type::Unit {
            return Err("main must have no parameters or return value".into());
        }
    }
    let mut errors = Vec::new();
    for (name, function) in &module.functions {
        if function.builtin || function.external || !function.generics.is_empty() {
            continue;
        }
        let mut checker = Checker {
            structs: &module.structs,
            functions: &module.functions,
            locals: BTreeMap::new(),
            loops: Vec::new(),
            position: function.position,
            error: function.error,
            result: function.result,
            frozen: Vec::new(),
            module: name.split(':').next().unwrap_or(name),
            generic: name.ends_with(']'),
            declared: Default::default(),
            discarded: false,
            written: Default::default(),
            closures: BTreeMap::new(),
        };
        let result = (|| {
            // 错误类型是字符串、结构体或枚举；C 侧结果结构体按（成功类型, 错误类型）区分。
            if function
                .error
                .is_some_and(|ty| !matches!(ty, Type::Owned | Type::Record(_)))
            {
                return Err(
                    "unsupported error type; errors are a string, a struct or an enum, such as -> i64!ParseError"
                        .into(),
                );
            }
            let mut names = BTreeSet::new();
            for (param, ty) in &function.params {
                if !names.insert(param.clone()) {
                    return Err(format!("duplicate binding {param}"));
                }
                checker.locals.insert(
                    param.clone(),
                    Binding {
                        ty: *ty,
                        mutable: false,
                        live: true,
                    },
                );
            }
            let returned = checker.block(&function.body, function.result)?;
            checker.position = function.position;
            if function.result != Type::Unit && !returned {
                return Err("missing string return value".into());
            }
            Ok(())
        })();
        if let Err(error) = result {
            let (line, column) = checker.position;
            errors.push(format!("line {line}, column {column}: {name}: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(&DIAGNOSTIC_SEPARATOR.to_string()))
    }
}

/// 编辑器用：推断光标处的变量在所在函数里的静态类型，返回源码写法的类型名。
///
/// 走的是和检查器同一套 `locals` 绑定，所以 `let p = Point(...)` 之后 `p` 就是 `Point`，
/// 不需要按字面猜。只要光标之前的代码能检查通过（正常写着的文件都满足），拿到的就是真实
/// 类型；检查不通过（代码写到一半、有错）时返回 None，调用方退回文字推断。
///
/// `name` 由编辑器从光标左侧取出；它不在 `locals` 里（字段名、模块名、函数名）也返回 None，
/// 那些不是“变量 `x.` 后面该列出什么”的问题。
pub fn local_type(module: &Module, line: usize, name: &str) -> Option<String> {
    // 包含这一行的函数：从它的声明行开始，到下一个函数声明前结束。
    let mut starts: Vec<(usize, &String)> = module
        .functions
        .iter()
        .filter(|(_, f)| !f.builtin && !f.external && f.generics.is_empty())
        .map(|(function, f)| (f.position.0, function))
        .collect();
    starts.sort_unstable();
    let index = starts.iter().rposition(|(start, _)| *start <= line)?;
    let (_, function_name) = starts[index];
    let end = starts.get(index + 1).map(|(next, _)| *next);
    if end.is_some_and(|end| line >= end) {
        return None;
    }
    let function = &module.functions[function_name];
    let mut checker = Checker {
        structs: &module.structs,
        functions: &module.functions,
        locals: BTreeMap::new(),
        loops: Vec::new(),
        position: function.position,
        error: function.error,
        result: function.result,
        frozen: Vec::new(),
        module: function_name.split(':').next().unwrap_or(function_name),
        generic: false,
        declared: BTreeMap::new(),
        discarded: false,
        written: Default::default(),
        closures: BTreeMap::new(),
    };
    for (param, ty) in &function.params {
        checker.locals.insert(
            param.clone(),
            Binding {
                ty: *ty,
                mutable: false,
                live: true,
            },
        );
    }
    // 用整个函数体检查一遍；`block` 退出时会从 `locals` 删掉块内声明的变量，但它们已经
    // 记进 `declared`，所以照样查得到。中途报错也没关系，前面的绑定仍然有效。
    let _ = checker.block(&function.body, function.result);
    let ty = *checker
        .locals
        .get(name)
        .map(|binding| &binding.ty)
        .or_else(|| checker.declared.get(name))?;
    Some(type_text(ty, &module.structs))
}

/// 把类型写成源码里的样子（`&mut Point`、`[string]`、`{string: i64}`），供编辑器显示。
pub fn type_text(ty: Type, structs: &BTreeMap<String, Struct>) -> String {
    match ty {
        Type::Owned => "string".to_owned(),
        Type::Shared => "&string".to_owned(),
        Type::Mutable => "&mut string".to_owned(),
        Type::Unit => "()".to_owned(),
        Type::Int => "i64".to_owned(),
        Type::Float => "f64".to_owned(),
        Type::Bool => "bool".to_owned(),
        Type::Record(id) => record_text(id, structs),
        Type::RecordRef(id, mutable) => format!(
            "&{}{}",
            if mutable { "mut " } else { "" },
            record_text(id, structs)
        ),
        Type::ArrayInt(n) => format!("[i64; {n}]"),
        Type::List(elem) => format!("[{}]", elem_text(elem, structs)),
        Type::ListRef(elem, mutable) => format!(
            "&{}[{}]",
            if mutable { "mut " } else { "" },
            elem_text(elem, structs)
        ),
        Type::Map(key, value) => format!(
            "{{{}: {}}}",
            elem_text(key, structs),
            elem_text(value, structs)
        ),
        Type::MapRef(key, value, mutable) => format!(
            "&{}{{{}: {}}}",
            if mutable { "mut " } else { "" },
            elem_text(key, structs),
            elem_text(value, structs)
        ),
        Type::Func(id) | Type::FuncRef(id) => func_text(id, structs),
    }
}

fn elem_text(elem: Elem, structs: &BTreeMap<String, Struct>) -> String {
    match elem {
        Elem::Int => "i64".to_owned(),
        Elem::Float => "f64".to_owned(),
        Elem::Bool => "bool".to_owned(),
        Elem::Str => "string".to_owned(),
        Elem::Record(id) => record_text(id, structs),
        Elem::Nested(id) => type_text(nested(id), structs),
        Elem::Func(id) => func_text(id, structs),
    }
}

/// 结构体 id → 显示用的名字：在结构体表里找编号对得上的那个（表里的键是 `模块:名字`）。
fn record_text(id: u64, structs: &BTreeMap<String, Struct>) -> String {
    structs
        .keys()
        .find(|name| record_id(name) == id)
        .cloned()
        .unwrap_or_else(|| format!("#{id}"))
}

fn func_text(id: u64, structs: &BTreeMap<String, Struct>) -> String {
    let (params, result) = func_sig(id);
    let params = params
        .iter()
        .map(|ty| type_text(*ty, structs))
        .collect::<Vec<_>>()
        .join(", ");
    format!("fn({params}) -> {}", type_text(result, structs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_reindents_to_four_spaces_and_keeps_comments_and_text() {
        let source = "fn main():\n  #块里的注释\n  let x =1\n  if true:\n      println(x)  #行尾注释\n      #井号注释\n  let y = {\n  \"a\": 1,\n      }\n";
        assert_eq!(
            format(source).unwrap(),
            "fn main():\n    #块里的注释\n    let x =1\n\n    if true:\n        println(x)  #行尾注释\n        #井号注释\n\n    let y = {\n        \"a\": 1,\n    }\n"
        );
    }

    #[test]
    fn format_expands_tabs_and_keeps_line_endings() {
        let source = "fn main():\r\n\tlet text = \"你好\"\r\n\tif true:\r\n\t\tprintln(&text)\r\n";
        let once = format(source).unwrap();
        assert_eq!(
            once,
            "fn main():\r\n    let text = \"你好\"\r\n\r\n    if true:\r\n        println(&text)\r\n"
        );
        assert_eq!(format(&once).unwrap(), once);
        let unix = format(&source.replace("\r\n", "\n")).unwrap();
        assert!(!unix.contains('\r'));
        assert_eq!(format(&unix).unwrap(), unix);
    }

    #[test]
    fn format_spaces_multi_line_statements_and_top_level_declarations() {
        let source = "use std/io\nuse std/string\nstruct P:\n    x: i64\n# 说明\nfn f(n: i64) -> i64:\n    if n < 1: return n\n    let a = 1\n    while a < n:\n        a = a + 1\n    # 结束\n    for i in 0..n:\n        println(i)\n    if a > 1:\n        println(1)\n    elif a > 0:\n        println(2)\n    else:\n        println(3)\n\n\n    a\nfn main():\n    match f(3):\n        _: pass\n";
        assert_eq!(
            format(source).unwrap(),
            "use std/io\nuse std/string\n\nstruct P:\n    x: i64\n\n# 说明\nfn f(n: i64) -> i64:\n    if n < 1: return n\n    let a = 1\n\n    while a < n:\n        a = a + 1\n\n    # 结束\n    for i in 0..n:\n        println(i)\n\n    if a > 1:\n        println(1)\n    elif a > 0:\n        println(2)\n    else:\n        println(3)\n\n    a\n\nfn main():\n    match f(3):\n        _: pass\n"
        );
        let once = format(source).unwrap();
        assert_eq!(format(&once).unwrap(), once);
    }

    #[test]
    fn format_keeps_decorators_next_to_their_function() {
        assert_eq!(
            format("fn f() -> i64: 1\n@wrapping\nfn g(n: i64) -> i64: n * 3\n").unwrap(),
            "fn f() -> i64: 1\n\n@wrapping\nfn g(n: i64) -> i64: n * 3\n"
        );
    }

    /// AST 的调试输出去掉源码位置 `(行, 列)`，只比较结构。
    fn shape(module: &Module) -> String {
        let text = format!(
            "{:?}{:?}{:?}{:?}",
            module.structs, module.type_names, module.imports, module.functions
        );
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            // 位置是 `(数字, 数字)`；AST 里别的元组都带字符串或名字，不会这样匹配。
            if chars[i] == '(' {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 && chars.get(j) == Some(&',') && chars.get(j + 1) == Some(&' ') {
                    let mut k = j + 2;
                    while k < chars.len() && chars[k].is_ascii_digit() {
                        k += 1;
                    }
                    if k > j + 2 && chars.get(k) == Some(&')') {
                        out.push_str("(_)");
                        i = k + 1;
                        continue;
                    }
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    /// 缩进决定代码块，所以缩进丢了的代码不能悄悄变成另一个程序：把示例与文档代码块的
    /// 行首空白全部删掉之后，要么报错，要么解析出完全相同的 AST。
    #[test]
    fn stripping_indentation_never_changes_meaning() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut sources: Vec<(String, String)> = Vec::new();
        fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n != "build") {
                        walk(&path, out);
                    }
                } else if path.extension().is_some_and(|e| e == "zm") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    out.push((path.display().to_string(), text));
                }
            }
        }
        walk(&root.join("examples"), &mut sources);
        walk(&root.join("bench/ai/live/solutions"), &mut sources);
        for (doc, bare) in [
            ("README.md", false),
            ("README.zh-CN.md", false),
            (".agents/skills/zamak/SKILL.md", false),
            ("bench/ai/live/zamak-reference.md", true),
        ] {
            let text = std::fs::read_to_string(root.join(doc)).unwrap();
            let mut block: Option<String> = None;
            for (number, line) in text.lines().enumerate() {
                let fence = line.trim_end();
                match &mut block {
                    None if fence == "```zamak"
                        || fence == "```zam"
                        || (bare && fence == "```") =>
                    {
                        block = Some(String::new());
                    }
                    Some(body) if fence == "```" => {
                        sources.push((format!("{doc}:{number}"), std::mem::take(body)));
                        block = None;
                    }
                    Some(body) => {
                        body.push_str(line);
                        body.push('\n');
                    }
                    None => {}
                }
            }
        }
        let mut checked = 0;
        for (name, source) in &sources {
            let Ok(module) = parse(source) else { continue };
            checked += 1;
            let flat: String = source
                .lines()
                .map(|line| line.trim_start().to_owned() + "\n")
                .collect();
            if let Ok(stripped) = parse(&flat) {
                assert_eq!(shape(&stripped), shape(&module), "{name}");
            }
        }
        assert!(checked >= 60, "too few sources checked: {checked}");
    }

    #[test]
    fn format_drops_trailing_blank_lines_and_reports_broken_source() {
        assert_eq!(
            format("fn main(): pass\n\n\n").unwrap(),
            "fn main(): pass\n"
        );
        assert!(format("fn main():\nprintln(\"open\n").is_err());
        assert!(format("fn main():\nprintln(1)\n").is_err());
    }
}
