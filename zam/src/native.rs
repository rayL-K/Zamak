use crate::compiler::{field_path, record_id, record_order, Expr, Instruction, Module, Type};
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
static int string_equal(S *a, S *b) { return a->len == b->len && (!a->len || memcmp(a->data,b->data,a->len) == 0); }
static void print(S *s) {
    if ((s->len && fwrite(s->data, 1, s->len, stdout) != s->len) || fputc('\n', stdout) == EOF) exit(1);
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
            Expr::Int(_) | Expr::ByteLen(_) => Type::Int,
            Expr::Array(values) => Type::ArrayInt(values.len()),
            Expr::Index(local, _) => match self.locals[local].1 {
                Type::ArrayInt(_) => Type::Int,
                _ => unreachable!(),
            },
            Expr::Bool(_) | Expr::StringEqual(_, _) => Type::Bool,
            Expr::Text(_) => Type::Owned,
            Expr::Variable(name) => self.locals[name].1,
            Expr::Borrow(_, mutable) => {
                if *mutable {
                    Type::Mutable
                } else {
                    Type::Shared
                }
            }
            Expr::Call(name, _) => self.module.functions[name].result,
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
            Expr::Text(text) => {
                let bytes = text
                    .as_bytes()
                    .iter()
                    .map(|b| format!("\\x{b:02x}"))
                    .collect::<String>();
                self.temporary(&format!(
                    "own((const unsigned char*)\"{bytes}\", {})",
                    text.len()
                ))
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
                let function = &self.module.functions[name];
                let types: Vec<_> = function.params.iter().map(|(_, ty)| *ty).collect();
                let result = function.result;
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
                let call = format!("{}({})", self.symbols[name], values.join(", "));
                if result == Type::Owned {
                    self.temporary(&call)
                } else if result != Type::Unit {
                    let value = self.scalar(&call, result);
                    if matches!(result, Type::Record(_)) {
                        self.owners.push((value.clone(), result));
                    }
                    value
                } else {
                    self.code.push_str(&format!("{call};\n"));
                    String::new()
                }
            }
        }
    }
    fn block(&mut self, body: &[Instruction], result_type: Type) -> bool {
        let outer = self.locals.clone();
        let scope_start = self.owners.len();
        let mut returned = false;
        for instruction in body {
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
                    self.code.push_str(if expr.is_some() {
                        "return result;\n"
                    } else {
                        "return;\n"
                    });
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
            ctype(function.result),
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
            Expr::Unary(_, value) | Expr::ByteLen(value) => calls(value, needed, records),
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
        body: &[Instruction],
        needed: &mut std::collections::BTreeSet<String>,
        records: &mut std::collections::BTreeSet<String>,
    ) {
        for instruction in body {
            match instruction {
                Instruction::Let(_, _, _, expr)
                | Instruction::Assign(_, expr)
                | Instruction::AssignField(_, _, expr)
                | Instruction::AssignIndex(_, _, expr)
                | Instruction::Print(expr)
                | Instruction::Call(expr) => calls(expr, needed, records),
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
        };
        for (i, (param, ty)) in function.params.iter().enumerate() {
            let local = format!("p{i}");
            if matches!(ty, Type::Owned | Type::Record(_)) {
                g.owners.push((local.clone(), *ty));
            }
            g.locals.insert(param.clone(), (local, *ty));
        }
        g.block(&function.body, function.result);
        g.cleanup();
        source.push_str(&g.code);
        source.push_str("}\n");
    }
    if entry.split_once(':').unwrap().0 == unit {
        source.push_str(&format!(
            "int main(void) {{ output_mode(); {}(); return fflush(stdout) == EOF ? 1 : 0; }}\n",
            symbols[entry]
        ));
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

pub const OPTIONS: &str = "c11-O2-MT-Brepro-pathmap-objects-v2";
pub fn compile(tool: &str, dir: &Path, source: &str) -> Result<Vec<u8>> {
    std::fs::write(dir.join("program.c"), source).map_err(|e| e.to_string())?;
    let output = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.current_dir(dir).args(["/d", "/s", "/c"]);
        let mapped = std::fs::canonicalize(dir).map_err(|e| e.to_string())?.to_string_lossy().trim_start_matches(r"\\?\").to_owned();
        if mapped.contains(['"', '%', '\r', '\n']) { return Err("unsupported native cache path".into()); }
        cmd_line(&mut command, &format!("\"call \"{tool}\" >nul && cl /nologo /O2 /MT /Brepro /c /d2pathmap:\"{mapped}=Z:\" program.c /Fo:program.obj\""));
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
