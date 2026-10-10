use crate::compiler::{
    field_path, record_id, record_order, Block, Expr, Instruction, Module, Type,
};
use crate::project::Result;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

const RUNTIME: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <inttypes.h>
#include <limits.h>
static void arithmetic_error(void) { fputs("zam: integer arithmetic error\n", stderr); exit(1); }
static int64_t add(int64_t a, int64_t b) {
    if ((b > 0 && a > INT64_MAX-b) || (b < 0 && a < INT64_MIN-b)) arithmetic_error();
    return a+b;
}
static int64_t sub(int64_t a, int64_t b) {
    if ((b < 0 && a > INT64_MAX+b) || (b > 0 && a < INT64_MIN+b)) arithmetic_error();
    return a-b;
}
static int64_t mul(int64_t a, int64_t b) {
    if (a > 0 ? (b > 0 ? a > INT64_MAX/b : b < INT64_MIN/a) :
        (a < 0 && (b > 0 ? a < INT64_MIN/b : b < 0 && a < INT64_MAX/b))) arithmetic_error();
    return a*b;
}
static int64_t quotient(int64_t a, int64_t b) {
    if (!b || (a == INT64_MIN && b == -1)) arithmetic_error(); return a/b;
}
static int64_t remainder(int64_t a, int64_t b) {
    if (!b || (a == INT64_MIN && b == -1)) arithmetic_error(); return a%b;
}
static int64_t checked_index(int64_t index, size_t length) {
    if (index < 0 || (uintmax_t)index >= length) { fputs("zam: array index out of bounds\\n", stderr); exit(1); }
    return index;
}
#ifdef _WIN32
#include <io.h>
#include <fcntl.h>
#endif
static void output_mode(void) {
#ifdef _WIN32
    if (_setmode(_fileno(stdout), _O_BINARY) == -1) exit(1);
#endif
}
typedef struct { unsigned char *data; size_t len; } S;
static S own(const unsigned char *p, size_t n) {
    S s; s.data = (unsigned char*)malloc(n ? n : 1); s.len = n;
    if (!s.data) { fputs("zam: allocation failed\n", stderr); exit(1); }
    if (n) memcpy(s.data, p, n); return s;
}
static void drop(S *s) { free(s->data); s->data = NULL; s->len = 0; }
static S take(S *s) { S value = *s; s->data = NULL; s->len = 0; return value; }
static int64_t byte_len(S *s) { if ((uintmax_t)s->len > INT64_MAX) arithmetic_error(); return (int64_t)s->len; }
// ponytail: 按 UTF-8 续字节计数，不校验编码合法性；非法字节按其起始字节计一个字符。
static int64_t char_len(S *s) { int64_t n = 0; for (size_t i = 0; i < s->len; i++) { if ((s->data[i] & 0xC0) != 0x80) n++; } return n; }
static int string_equal(S *a, S *b) { return a->len == b->len && (!a->len || memcmp(a->data,b->data,a->len) == 0); }
static void print(S *s) {
    if ((s->len && fwrite(s->data, 1, s->len, stdout) != s->len) || fputc('\n', stdout) == EOF) exit(1);
}
// ponytail: 未捕获的错误只写 stderr 并以退出码 1 结束；不做栈回溯，错误值本身就是全部信息。
static void uncaught_error(S *s) {
    if (s->len && fwrite(s->data, 1, s->len, stderr) != s->len) exit(1);
    if (fputc('\n', stderr) == EOF) exit(1);
}
static S copy(S *s) { return own(s->data, s->len); }
static S cat(S a, S b) {
    size_t n = a.len + b.len;
    unsigned char *p = (unsigned char*)malloc(n ? n : 1);
    if (!p) { fputs("zam: allocation failed\n", stderr); exit(1); }
    if (a.len) memcpy(p, a.data, a.len);
    if (b.len) memcpy(p + a.len, b.data, b.len);
    drop(&a); drop(&b);
    S s; s.data = p; s.len = n; return s;
}
static S from_int(int64_t value) {
    char buffer[32];
    int n = snprintf(buffer, sizeof buffer, "%" PRId64, value);
    return own((const unsigned char*)buffer, (size_t)n);
}
static S from_bool(int value) {
    return own((const unsigned char*)(value ? "true" : "false"), value ? 4 : 5);
}
"#;

struct Generator<'a> {
    module: &'a Module,
    symbols: &'a BTreeMap<String, String>,
    locals: BTreeMap<String, (String, Type)>,
    owners: Vec<(String, Type)>,
    loops: Vec<usize>,
    code: String,
    next: usize,
    // 当前函数的签名：`?` 提前返回时要构造同样形状的结果结构体。
    result: Type,
    error: Option<Type>,
}
// 失败函数的结果结构体：ok=1 时 value 有效，ok=0 时 error 有效。
fn fallible_name(ty: Type) -> String {
    format!("F_{}", ctype(ty))
}
fn fallible_ok(ty: Type) -> String {
    format!("{}_ok", fallible_name(ty))
}
fn fallible_err(ty: Type) -> String {
    format!("{}_err", fallible_name(ty))
}
fn drop_fn(ty: Type) -> String {
    if matches!(ty, Type::Record(_)) {
        format!("drop{}", ctype(ty))
    } else {
        "drop".into()
    }
}
fn ctype(ty: Type) -> String {
    match ty {
        Type::Record(id) => return format!("R{id}"),
        Type::Owned => "S",
        Type::Shared | Type::Mutable => "S*",
        Type::Int => "int64_t",
        Type::Bool => "int",
        Type::ArrayInt(length) => return format!("int64_t[{length}]"),
        Type::Unit => "void",
    }
    .into()
}
impl Generator<'_> {
    fn ty(&self, expr: &Expr) -> Type {
        match expr {
            Expr::Record(name, _) => Type::Record(record_id(name)),
            Expr::Field(local, field) => {
                field_path(&self.module.structs, self.locals[local].1, field)
                    .expect("checked field path")
                    .1
            }
            Expr::Int(_) | Expr::ByteLen(_) | Expr::CharLen(_) => Type::Int,
            Expr::Array(values) => Type::ArrayInt(values.len()),
            Expr::Index(local, _) => match self.locals[local].1 {
                Type::ArrayInt(_) => Type::Int,
                _ => unreachable!(),
            },
            Expr::Bool(_) | Expr::StringEqual(_, _) => Type::Bool,
            Expr::Text(_) | Expr::Format(_) => Type::Owned,
            Expr::Variable(name) => self.locals[name].1,
            Expr::Borrow(_, mutable) => {
                if *mutable {
                    Type::Mutable
                } else {
                    Type::Shared
                }
            }
            Expr::Call(name, _) => self.module.functions[name].result,
            Expr::Try(inner) => self.ty(inner),
            Expr::Catch(inner, _, _) => self.ty(inner),
            Expr::Unary(op, _) => {
                if *op == '!' {
                    Type::Bool
                } else {
                    Type::Int
                }
            }
            Expr::Binary(op, _, _) => {
                if matches!(op.as_str(), "+" | "-" | "*" | "/" | "%") {
                    Type::Int
                } else {
                    Type::Bool
                }
            }
        }
    }
    fn scalar(&mut self, value: &str, ty: Type) -> String {
        let name = format!("v{}", self.next);
        self.next += 1;
        self.code
            .push_str(&format!("{} {name} = {value};\n", ctype(ty)));
        name
    }
    fn temporary(&mut self, value: &str) -> String {
        let name = format!("v{}", self.next);
        self.next += 1;
        self.code.push_str(&format!("S {name} = {value};\n"));
        self.owners.push((name.clone(), Type::Owned));
        name
    }
    fn literal(&self, text: &str) -> String {
        let bytes = text
            .as_bytes()
            .iter()
            .map(|b| format!("\\x{b:02x}"))
            .collect::<String>();
        format!("own((const unsigned char*)\"{bytes}\", {})", text.len())
    }
    fn text_of(&self, pointer: String, ty: Type) -> String {
        match ty {
            Type::Owned | Type::Shared | Type::Mutable => format!("copy({pointer})"),
            Type::Int => format!("from_int({pointer})"),
            _ => format!("from_bool({pointer})"),
        }
    }
    fn pointer(&self, name: &str) -> String {
        if let Some((root, field)) = name.split_once('.') {
            return format!("&{}", self.field(root, field));
        }
        let (local, ty) = &self.locals[name];
        if matches!(ty, Type::Owned | Type::Record(_)) {
            format!("&{local}")
        } else {
            local.clone()
        }
    }
    fn field(&self, local: &str, field: &str) -> String {
        let (indices, _) = field_path(&self.module.structs, self.locals[local].1, field)
            .expect("checked field path");
        let mut target = self.locals[local].0.clone();
        for index in indices {
            target.push_str(&format!(".m{index}"));
        }
        target
    }
    fn expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Record(name, fields) => {
                let ty = Type::Record(record_id(name));
                let local = self.scalar("{0}", ty);
                self.owners.push((local.clone(), ty));
                let declaration = &self.module.structs[name];
                for (field, expr) in fields {
                    let index = declaration
                        .fields
                        .iter()
                        .position(|(n, _)| n == field)
                        .unwrap();
                    let ty = self.ty(expr);
                    let value = self.expr(expr);
                    self.code.push_str(&format!(
                        "{local}.m{index} = {};\n",
                        if let Type::Record(_) = ty {
                            format!("take{}(&{value})", ctype(ty))
                        } else if ty == Type::Owned {
                            format!("take(&{value})")
                        } else {
                            value
                        }
                    ));
                }
                local
            }
            Expr::Field(local, field) => self.scalar(&self.field(local, field), self.ty(expr)),
            Expr::Variable(name) if matches!(self.locals[name].1, Type::Record(_)) => {
                let ty = self.locals[name].1;
                let local = self.scalar(&format!("take{}({})", ctype(ty), self.pointer(name)), ty);
                self.owners.push((local.clone(), ty));
                local
            }
            Expr::StringEqual(left, right) => {
                let pointer = |expr: &Expr| match expr {
                    Expr::Variable(name) | Expr::Borrow(name, _) => self.pointer(name),
                    _ => unreachable!("checked shared string"),
                };
                self.scalar(
                    &format!("string_equal({}, {})", pointer(left), pointer(right)),
                    Type::Bool,
                )
            }
            Expr::ByteLen(value) => {
                let pointer = match value.as_ref() {
                    Expr::Variable(name) | Expr::Borrow(name, _) => self.pointer(name),
                    _ => unreachable!("checked shared string"),
                };
                self.scalar(&format!("byte_len({pointer})"), Type::Int)
            }
            Expr::CharLen(value) => {
                let pointer = match value.as_ref() {
                    Expr::Variable(name) | Expr::Borrow(name, _) => self.pointer(name),
                    _ => unreachable!("checked shared string"),
                };
                self.scalar(&format!("char_len({pointer})"), Type::Int)
            }
            Expr::Array(values) => {
                let length = values.len();
                let name = format!("v{}", self.next);
                self.next += 1;
                self.code
                    .push_str(&format!("int64_t {name}[{length}] = {{0}};\n"));
                for (index, value) in values.iter().enumerate() {
                    let rendered = self.expr(value);
                    self.code
                        .push_str(&format!("{name}[{index}] = {rendered};\n"));
                }
                name
            }
            Expr::Index(local, index) => {
                let index = self.expr(index);
                let length = match self.locals[local].1 {
                    Type::ArrayInt(length) => length,
                    _ => unreachable!(),
                };
                self.scalar(
                    &format!("{}[checked_index({index}, {length})]", self.locals[local].0),
                    Type::Int,
                )
            }
            Expr::Text(text) => self.temporary(&self.literal(text)),
            Expr::Format(segments) => {
                let mut parts = Vec::with_capacity(segments.len());
                for segment in segments {
                    parts.push(match segment {
                        Expr::Text(text) => self.literal(text),
                        Expr::Variable(name) => {
                            let pointer = self.pointer(name);
                            self.text_of(pointer, self.ty(segment))
                        }
                        Expr::Field(local, field) => {
                            let ty = self.ty(segment);
                            let target = self.field(local, field);
                            let target = if matches!(ty, Type::Int | Type::Bool) {
                                target
                            } else {
                                format!("&{target}")
                            };
                            self.text_of(target, ty)
                        }
                        _ => unreachable!("interpolation only produces text and field paths"),
                    });
                }
                let mut parts = parts.into_iter();
                let name = self.temporary(&parts.next().expect("interpolation has segments"));
                for part in parts {
                    self.code
                        .push_str(&format!("{name} = cat({name}, {part});\n"));
                }
                name
            }
            Expr::Int(value) => self.scalar(
                &if *value == i64::MIN {
                    "INT64_MIN".into()
                } else {
                    format!("INT64_C({value})")
                },
                Type::Int,
            ),
            Expr::Bool(value) => self.scalar(if *value { "1" } else { "0" }, Type::Bool),
            Expr::Unary(op, value) => {
                let ty = self.ty(expr);
                let value = self.expr(value);
                self.scalar(
                    &if *op == '-' {
                        format!("sub(0, {value})")
                    } else {
                        format!("!{value}")
                    },
                    ty,
                )
            }
            Expr::Binary(op, left, right) => {
                let ty = self.ty(expr);
                let left = self.expr(left);
                if matches!(op.as_str(), "&&" | "||") {
                    let value = self.scalar(&left, Type::Bool);
                    self.code.push_str(&format!(
                        "if ({}{value}) {{\n",
                        if op == "||" { "!" } else { "" }
                    ));
                    let scope_start = self.owners.len();
                    let right = self.expr(right);
                    self.code.push_str(&format!("{value} = {right};\n"));
                    for (name, ty) in &self.owners[scope_start..] {
                        self.code.push_str(&format!("{}(&{name});\n", drop_fn(*ty)));
                    }
                    self.owners.truncate(scope_start);
                    self.code.push_str("}\n");
                    return value;
                }
                let right = self.expr(right);
                let function = match op.as_str() {
                    "+" => Some("add"),
                    "-" => Some("sub"),
                    "*" => Some("mul"),
                    "/" => Some("quotient"),
                    "%" => Some("remainder"),
                    _ => None,
                };
                self.scalar(
                    &if let Some(function) = function {
                        format!("{function}({left}, {right})")
                    } else {
                        format!("({left} {op} {right})")
                    },
                    ty,
                )
            }
            Expr::Variable(name) if matches!(self.locals[name].1, Type::Int | Type::Bool) => {
                let (value, ty) = self.locals[name].clone();
                self.scalar(&value, ty)
            }
            Expr::Variable(name) => self.temporary(&format!("take({})", self.pointer(name))),
            Expr::Borrow(name, _) => self.pointer(name),
            Expr::Call(name, args) => {
                let result = self.module.functions[name].result;
                let call = self.raw_call(name, args);
                self.call_value(&call, result)
            }
            Expr::Try(inner) => {
                let Expr::Call(name, args) = &**inner else {
                    unreachable!("checked ? operand")
                };
                let result = self.module.functions[name].result;
                let call = self.raw_call(name, args);
                let structure = fallible_name(result);
                let temp = format!("v{}", self.next);
                self.next += 1;
                self.code
                    .push_str(&format!("{structure} {temp} = {call};\n"));
                // 失败：先把当前作用域的值都释放掉，再构造同样形状的错误结果返回。
                self.code.push_str(&format!("if (!{temp}.ok) {{\n"));
                let outer = fallible_err(self.result);
                self.cleanup();
                self.code
                    .push_str(&format!("return {outer}(take(&{temp}.error));\n}}\n"));
                if result == Type::Unit {
                    return String::new();
                }
                self.result_value(&temp, result)
            }
            Expr::Catch(inner, binding, block) => {
                let Expr::Call(name, args) = &**inner else {
                    unreachable!("checked catch operand")
                };
                let result = self.module.functions[name].result;
                let call = self.raw_call(name, args);
                let structure = fallible_name(result);
                let temp = format!("v{}", self.next);
                self.next += 1;
                self.code
                    .push_str(&format!("{structure} {temp} = {call};\n"));
                self.code.push_str(&format!("if (!{temp}.ok) {{\n"));
                let local = format!("v{}", self.next);
                self.next += 1;
                let previous = self
                    .locals
                    .insert(binding.clone(), (local.clone(), Type::Owned));
                self.code
                    .push_str(&format!("S {local} = take(&{temp}.error);\n"));
                self.owners.push((local, Type::Owned));
                // 代码块在所有路径上都 return（检查器保证），所以出块只需要清账本。
                self.block(block, self.result);
                self.owners.pop();
                self.locals.remove(binding);
                if let Some(previous) = previous {
                    self.locals.insert(binding.clone(), previous);
                }
                self.code.push_str("}\n");
                self.result_value(&temp, result)
            }
        }
    }
    // 从结果结构体里取出成功值；Unit 没有值，返回空串。
    fn result_value(&mut self, temp: &str, result: Type) -> String {
        if result == Type::Unit {
            return String::new();
        }
        if matches!(result, Type::Record(_)) {
            let value = self.scalar(&format!("take{}(&{temp}.value)", ctype(result)), result);
            self.owners.push((value.clone(), result));
            value
        } else if result == Type::Owned {
            self.temporary(&format!("take(&{temp}.value)"))
        } else {
            self.scalar(&format!("{temp}.value"), result)
        }
    }
    fn raw_call(&mut self, name: &str, args: &[Expr]) -> String {
        let function = &self.module.functions[name];
        let types: Vec<_> = function.params.iter().map(|(_, ty)| *ty).collect();
        let mut values = Vec::new();
        for (arg, ty) in args.iter().zip(types) {
            values.push(if let Type::Record(_) = ty {
                let value = self.expr(arg);
                format!("take{}(&{value})", ctype(ty))
            } else if ty == Type::Owned {
                let value = self.expr(arg);
                format!("take(&{value})")
            } else if matches!(ty, Type::Int | Type::Bool) {
                self.expr(arg)
            } else {
                match arg {
                    Expr::Variable(local) | Expr::Borrow(local, _) => self.pointer(local),
                    _ => unreachable!(),
                }
            });
        }
        format!("{}({})", self.symbols[name], values.join(", "))
    }
    fn call_value(&mut self, call: &str, result: Type) -> String {
        if result == Type::Owned {
            self.temporary(call)
        } else if result != Type::Unit {
            let value = self.scalar(call, result);
            if matches!(result, Type::Record(_)) {
                self.owners.push((value.clone(), result));
            }
            value
        } else {
            self.code.push_str(&format!("{call};\n"));
            String::new()
        }
    }
    fn block(&mut self, body: &Block, result_type: Type) -> bool {
        let outer = self.locals.clone();
        let scope_start = self.owners.len();
        let mut returned = false;
        for (instruction, _) in body {
            match instruction {
                Instruction::If(condition, yes, no) => {
                    let value = self.expr(condition);
                    self.code.push_str(&format!("if ({value}) {{\n"));
                    let yes_returns = self.block(yes, result_type);
                    self.code.push_str("} else {\n");
                    let no_returns = self.block(no, result_type);
                    self.code.push_str("}\n");
                    returned = yes_returns && no_returns;
                }
                Instruction::While(condition, nested) => {
                    self.code.push_str("while (1) {\n");
                    let owners = self.owners.len();
                    let value = self.expr(condition);
                    self.code.push_str(&format!("int condition = {value};\n"));
                    for (name, ty) in &self.owners[owners..] {
                        self.code.push_str(&format!("{}(&{name});\n", drop_fn(*ty)));
                    }
                    self.owners.truncate(owners);
                    self.code.push_str("if (!condition) break;\n");
                    self.loops.push(self.owners.len());
                    self.block(nested, result_type);
                    self.loops.pop();
                    self.code.push_str("}\n");
                }
                Instruction::Let(name, _, _, expr) => {
                    let ty = self.ty(expr);
                    let value = self.expr(expr);
                    self.locals.insert(name.clone(), (value, ty));
                }
                Instruction::AssignIndex(local, index, expr) => {
                    let index = self.expr(index);
                    let value = self.expr(expr);
                    let length = match self.locals[local].1 {
                        Type::ArrayInt(length) => length,
                        _ => unreachable!(),
                    };
                    self.code.push_str(&format!(
                        "{}[checked_index({index}, {length})] = {value};\n",
                        self.locals[local].0
                    ));
                }
                Instruction::AssignField(local, field, expr) => {
                    let value = self.expr(expr);
                    let target = self.field(local, field);
                    let ty = self.ty(expr);
                    if matches!(ty, Type::Record(_)) {
                        self.code.push_str(&format!(
                            "{}(&{target}); {target} = take{}(&{value});\n",
                            drop_fn(ty),
                            ctype(ty)
                        ));
                    } else if ty == Type::Owned {
                        self.code
                            .push_str(&format!("drop(&{target}); {target} = take(&{value});\n"));
                    } else {
                        self.code.push_str(&format!("{target} = {value};\n"));
                    }
                }
                Instruction::Assign(name, expr) => {
                    let value = self.expr(expr);
                    if matches!(self.locals[name].1, Type::Int | Type::Bool) {
                        self.code
                            .push_str(&format!("{} = {value};\n", self.locals[name].0));
                        continue;
                    }
                    let ty = self.locals[name].1;
                    let pointer = self.pointer(name);
                    if matches!(ty, Type::Record(_)) {
                        self.code.push_str(&format!(
                            "{}({pointer}); *({pointer}) = take{}(&{value});\n",
                            drop_fn(ty),
                            ctype(ty)
                        ));
                        continue;
                    }
                    self.code.push_str(&format!(
                        "drop({pointer});\n*({pointer}) = take(&{value});\n"
                    ));
                }
                Instruction::Print(expr) => {
                    let ty = self.ty(expr);
                    if matches!(ty, Type::Int | Type::Bool) {
                        let value = self.expr(expr);
                        self.code.push_str(&if ty == Type::Int {
                            format!("if (printf(\"%\" PRId64 \"\\n\", {value}) < 0) exit(1);\n")
                        } else {
                            format!("if (puts({value} ? \"true\" : \"false\") == EOF) exit(1);\n")
                        });
                        continue;
                    }
                    let pointer = match expr {
                        Expr::Variable(name) | Expr::Borrow(name, _) => self.pointer(name),
                        Expr::Field(local, field) => format!("&{}", self.field(local, field)),
                        _ => {
                            let value = self.expr(expr);
                            format!("&{value}")
                        }
                    };
                    self.code.push_str(&format!("print({pointer});\n"));
                }
                Instruction::Call(expr) => {
                    self.expr(expr);
                }
                Instruction::Break => {
                    self.cleanup_from(*self.loops.last().expect("checked loop jump"));
                    self.code.push_str("break;\n");
                    returned = true;
                }
                Instruction::Continue => {
                    self.cleanup_from(*self.loops.last().expect("checked loop jump"));
                    self.code.push_str("continue;\n");
                    returned = true;
                }
                Instruction::Return(expr) => {
                    if let Some(expr) = expr {
                        let value = self.expr(expr);
                        self.code
                            .push_str(&if matches!(result_type, Type::Record(_)) {
                                format!(
                                    "{} result = take{}(&{value});\n",
                                    ctype(result_type),
                                    ctype(result_type)
                                )
                            } else if result_type == Type::Owned {
                                format!("S result = take(&{value});\n")
                            } else {
                                format!("{} result = {value};\n", ctype(result_type))
                            });
                    }
                    self.cleanup();
                    if self.error.is_some() {
                        // 失败函数：成功值要包进结果结构体。
                        let ok = fallible_ok(result_type);
                        self.code.push_str(&if expr.is_some() {
                            format!("return {ok}(result);\n")
                        } else {
                            format!("return {ok}();\n")
                        });
                    } else {
                        self.code.push_str(if expr.is_some() {
                            "return result;\n"
                        } else {
                            "return;\n"
                        });
                    }
                    returned = true;
                }
                Instruction::Fail(expr) => {
                    let value = self.expr(expr);
                    self.code
                        .push_str(&format!("S failure = take(&{value});\n"));
                    self.cleanup();
                    self.code
                        .push_str(&format!("return {}(failure);\n", fallible_err(self.result)));
                    returned = true;
                }
            }
        }
        if !returned {
            for (name, ty) in &self.owners[scope_start..] {
                self.code.push_str(&format!("{}(&{name});\n", drop_fn(*ty)));
            }
        }
        self.owners.truncate(scope_start);
        self.locals = outer;
        returned
    }
    fn cleanup_from(&mut self, start: usize) {
        for (name, ty) in &self.owners[start..] {
            self.code.push_str(&format!("{}(&{name});\n", drop_fn(*ty)));
        }
    }
    fn cleanup(&mut self) {
        self.cleanup_from(0);
    }
}

pub fn generate(module: &Module, entry: &str, unit: &str) -> String {
    let symbols: BTreeMap<_, _> = module
        .functions
        .keys()
        .map(|name| {
            (
                name.clone(),
                format!(
                    "f{}",
                    name.as_bytes()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                ),
            )
        })
        .collect();
    let signature = |name: &str| {
        let function = &module.functions[name];
        let params = function
            .params
            .iter()
            .enumerate()
            .map(|(i, (_, ty))| format!("{} p{i}", ctype(*ty)))
            .collect::<Vec<_>>();
        format!(
            "{} {}({})",
            if function.error.is_some() {
                fallible_name(function.result)
            } else {
                ctype(function.result)
            },
            symbols[name],
            if params.is_empty() {
                "void".into()
            } else {
                params.join(", ")
            }
        )
    };
    let mut source = RUNTIME.to_owned();
    let mut records = std::collections::BTreeSet::new();
    let mut needed = std::collections::BTreeSet::new();
    fn calls(
        expr: &Expr,
        needed: &mut std::collections::BTreeSet<String>,
        records: &mut std::collections::BTreeSet<String>,
    ) {
        match expr {
            Expr::Call(name, args) => {
                needed.insert(name.clone());
                for arg in args {
                    calls(arg, needed, records);
                }
            }
            Expr::Record(name, fields) => {
                records.insert(name.clone());
                for (_, value) in fields {
                    calls(value, needed, records);
                }
            }
            Expr::Unary(_, value) | Expr::ByteLen(value) | Expr::CharLen(value) => {
                calls(value, needed, records)
            }
            Expr::Try(inner) => calls(inner, needed, records),
            Expr::Catch(inner, _, block) => {
                calls(inner, needed, records);
                body_calls(block, needed, records);
            }
            Expr::Array(values) => {
                for value in values {
                    calls(value, needed, records);
                }
            }
            Expr::Index(_, index) => calls(index, needed, records),
            Expr::Binary(_, left, right) | Expr::StringEqual(left, right) => {
                calls(left, needed, records);
                calls(right, needed, records);
            }
            _ => {}
        }
    }
    fn body_calls(
        body: &Block,
        needed: &mut std::collections::BTreeSet<String>,
        records: &mut std::collections::BTreeSet<String>,
    ) {
        for (instruction, _) in body {
            match instruction {
                Instruction::Let(_, _, _, expr)
                | Instruction::Assign(_, expr)
                | Instruction::AssignField(_, _, expr)
                | Instruction::AssignIndex(_, _, expr)
                | Instruction::Print(expr)
                | Instruction::Call(expr)
                | Instruction::Fail(expr) => calls(expr, needed, records),
                Instruction::Return(Some(expr)) => calls(expr, needed, records),
                Instruction::If(expr, yes, no) => {
                    calls(expr, needed, records);
                    body_calls(yes, needed, records);
                    body_calls(no, needed, records);
                }
                Instruction::While(expr, body) => {
                    calls(expr, needed, records);
                    body_calls(body, needed, records);
                }
                Instruction::Return(None) | Instruction::Break | Instruction::Continue => {}
            }
        }
    }
    for (name, function) in &module.functions {
        if name.split_once(':').unwrap().0 == unit {
            needed.insert(name.clone());
            body_calls(&function.body, &mut needed, &mut records);
        }
    }
    if entry.split_once(':').unwrap().0 == unit {
        needed.insert(entry.into());
    }
    for name in &needed {
        let function = &module.functions[name];
        for ty in function
            .params
            .iter()
            .map(|(_, ty)| ty)
            .chain(std::iter::once(&function.result))
        {
            if let Type::Record(id) = ty {
                records.insert(
                    module
                        .structs
                        .keys()
                        .find(|name| record_id(name) == *id)
                        .unwrap()
                        .clone(),
                );
            }
        }
    }
    for name in record_order(module, &records).expect("checked struct layouts") {
        let structure = &module.structs[&name];
        let id = record_id(&name);
        source.push_str(&format!(
            "typedef struct {{ {} }} R{id};\n",
            if structure.fields.is_empty() {
                "int empty;".into()
            } else {
                structure
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, (_, ty))| format!("{} m{i};", ctype(*ty)))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        ));
        source.push_str(&format!("static void dropR{id}(R{id} *p) {{"));
        for (i, (_, ty)) in structure.fields.iter().enumerate() {
            if matches!(ty, Type::Owned | Type::Record(_)) {
                source.push_str(&format!("{}(&p->m{i});", drop_fn(*ty)));
            }
        }
        source.push_str(&format!("memset(p,0,sizeof(*p)); }}\nstatic R{id} takeR{id}(R{id} *p) {{ R{id} v=*p; memset(p,0,sizeof(*p)); return v; }}\n"));
    }
    // 失败函数的结果结构体与构造器；调用点只读 ok 相符的那一半。
    let mut fallible: Vec<Type> = Vec::new();
    for name in &needed {
        let result = module.functions[name].result;
        if module.functions[name].error.is_some() && !fallible.contains(&result) {
            fallible.push(result);
        }
    }
    for ty in &fallible {
        let structure = fallible_name(*ty);
        source.push_str(&format!(
            "typedef struct {{ int ok; {}S error; }} {structure};\n",
            if *ty == Type::Unit {
                String::new()
            } else {
                format!("{} value; ", ctype(*ty))
            }
        ));
        if *ty == Type::Unit {
            source.push_str(&format!(
                "static {structure} {}(void) {{ {structure} r = {{0}}; r.ok = 1; return r; }}\n",
                fallible_ok(*ty)
            ));
        } else {
            source.push_str(&format!(
                "static {structure} {}({} value) {{ {structure} r = {{0}}; r.ok = 1; r.value = value; return r; }}\n",
                fallible_ok(*ty),
                ctype(*ty)
            ));
        }
        source.push_str(&format!(
            "static {structure} {}(S error) {{ {structure} r = {{0}}; r.error = error; return r; }}\n",
            fallible_err(*ty)
        ));
    }
    for name in &needed {
        source.push_str(&format!("{};\n", signature(name)));
    }
    for (name, function) in &module.functions {
        if name.split_once(':').unwrap().0 != unit {
            continue;
        }
        source.push_str(&format!("{} {{\n", signature(name)));
        let mut g = Generator {
            module,
            symbols: &symbols,
            locals: BTreeMap::new(),
            owners: Vec::new(),
            loops: Vec::new(),
            code: String::new(),
            next: 0,
            result: function.result,
            error: function.error,
        };
        for (i, (param, ty)) in function.params.iter().enumerate() {
            let local = format!("p{i}");
            if matches!(ty, Type::Owned | Type::Record(_)) {
                g.owners.push((local.clone(), *ty));
            }
            g.locals.insert(param.clone(), (local, *ty));
        }
        let returned = g.block(&function.body, function.result);
        g.cleanup();
        source.push_str(&g.code);
        if function.error.is_some() && !returned && function.result == Type::Unit {
            // 失败函数返回 Unit 时也要显式给出成功值。
            source.push_str(&format!("return {}();\n", fallible_ok(Type::Unit)));
        }
        source.push_str("}\n");
    }
    if entry.split_once(':').unwrap().0 == unit {
        if module.functions[entry].error.is_some() {
            let structure = fallible_name(module.functions[entry].result);
            source.push_str(&format!(
                "int main(void) {{ output_mode(); {structure} r = {}(); if (!r.ok) {{ uncaught_error(&r.error); return 1; }} return fflush(stdout) == EOF ? 1 : 0; }}\n",
                symbols[entry]
            ));
        } else {
            source.push_str(&format!(
                "int main(void) {{ output_mode(); {}(); return fflush(stdout) == EOF ? 1 : 0; }}\n",
                symbols[entry]
            ));
        }
    }
    source
}

fn cmd_line(command: &mut Command, line: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(line);
    }
    #[cfg(not(windows))]
    command.arg(line);
}

pub fn tool() -> Result<(String, String)> {
    if cfg!(windows) {
        let root = std::env::var_os("ProgramFiles(x86)").ok_or("missing ProgramFiles(x86)")?;
        let output =
            Command::new(Path::new(&root).join("Microsoft Visual Studio/Installer/vswhere.exe"))
                .args([
                    "-latest",
                    "-products",
                    "*",
                    "-requires",
                    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                    "-property",
                    "installationPath",
                ])
                .output()
                .map_err(|e| format!("MSVC discovery failed: {e}"))?;
        let installation = String::from_utf8(output.stdout)
            .map_err(|e| e.to_string())?
            .trim()
            .to_owned();
        let setup = Path::new(&installation).join("VC/Auxiliary/Build/vcvars64.bat");
        if !output.status.success() || !setup.is_file() {
            return Err("install MSVC C++ build tools for native builds".into());
        }
        let setup = setup.to_string_lossy().into_owned();
        if setup.contains(['"', '%', '\r', '\n']) {
            return Err("unsupported MSVC setup path".into());
        }
        let tools = std::fs::read_to_string(
            Path::new(&installation)
                .join("VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt"),
        )
        .map_err(|e| format!("MSVC default version unavailable: {e}"))?;
        let tools = tools.trim();
        if tools.is_empty() || !tools.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Err("invalid MSVC default version".into());
        }
        let compiler = Path::new(&installation)
            .join("VC/Tools/MSVC")
            .join(tools)
            .join("bin/Hostx64/x64/cl.exe");
        let version = Command::new(compiler)
            .arg("/Bv")
            .output()
            .map_err(|e| format!("MSVC version query failed: {e}"))?;
        let identity = format!(
            "{}{}",
            String::from_utf8_lossy(&version.stdout),
            String::from_utf8_lossy(&version.stderr)
        );
        // /Bv without an input reports versions then exits 2 (D8003).
        if !matches!(version.status.code(), Some(0 | 2))
            || !identity.contains("cl.exe:")
            || !identity.contains("c2.dll:")
        {
            return Err(format!("MSVC version query failed:\n{identity}"));
        }
        Ok((setup, identity))
    } else {
        let compiler = std::env::var("CC").unwrap_or_else(|_| "cc".into());
        let output = Command::new(&compiler)
            .arg("--version")
            .output()
            .map_err(|e| format!("C compiler unavailable: {e}"))?;
        if !output.status.success() {
            return Err("C compiler version query failed".into());
        }
        Ok((
            compiler,
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ))
    }
}

pub const OPTIONS: &str = "c11-O2-MT-Brepro-pathmap-objects-v3";
pub fn compile(tool: &str, dir: &Path, source: &str) -> Result<Vec<u8>> {
    std::fs::write(dir.join("program.c"), source).map_err(|e| e.to_string())?;
    let output = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.current_dir(dir).args(["/d", "/s", "/c"]);
        let mapped = std::fs::canonicalize(dir).map_err(|e| e.to_string())?.to_string_lossy().trim_start_matches(r"\\?\").to_owned();
        if mapped.contains(['"', '%', '\r', '\n']) { return Err("unsupported native cache path".into()); }
        // ponytail: 源码按 UTF-8 写出，cl 默认按本地代码页(如 936)读取会吞掉中文注释后的下一行；/utf-8 固定两端编码。
        cmd_line(&mut command, &format!("\"call \"{tool}\" >nul && cl /nologo /O2 /MT /Brepro /utf-8 /c /d2pathmap:\"{mapped}=Z:\" program.c /Fo:program.obj\""));
        command.output()
    } else {
        Command::new(tool).current_dir(dir).args(["-std=c11", "-O2", "-c", "program.c", "-o", "program.obj"]).output()
    }.map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "native compilation failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    std::fs::read(dir.join("program.obj")).map_err(|e| e.to_string())
}

pub fn link(tool: &str, dir: &Path, objects: &[Vec<u8>]) -> Result<Vec<u8>> {
    let mut names = Vec::new();
    for (index, bytes) in objects.iter().enumerate() {
        let name = format!("module{index}.obj");
        std::fs::write(dir.join(&name), bytes).map_err(|e| e.to_string())?;
        names.push(name);
    }
    let output = if cfg!(windows) {
        let mut command = Command::new("cmd"); command.current_dir(dir).args(["/d", "/s", "/c"]);
        cmd_line(&mut command, &format!("\"call \"{tool}\" >nul && link /nologo /Brepro /INCREMENTAL:NO /OUT:program.exe {}\"", names.join(" ")));
        command.output()
    } else {
        Command::new(tool).current_dir(dir).args(&names).args(["-o", "program.exe"]).output()
    }.map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "native linking failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    std::fs::read(dir.join("program.exe")).map_err(|e| e.to_string())
}
