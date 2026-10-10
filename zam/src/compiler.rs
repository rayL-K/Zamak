use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Text(String),
    Number(String),
    Symbol(char),
    Newline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Owned,
    Shared,
    Mutable,
    Unit,
    Int,
    Bool,
    Record(u64),
    ArrayInt(usize),
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
    ByteLen(Box<Expr>),
    StringEqual(Box<Expr>, Box<Expr>),
    Int(i64),
    Bool(bool),
    Unary(char, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Array(Vec<Expr>),
    Index(String, Box<Expr>),
}

#[derive(Clone, Debug)]
pub enum Instruction {
    Let(String, bool, Option<Type>, Expr),
    Assign(String, Expr),
    AssignField(String, String, Expr),
    AssignIndex(String, Expr, Expr),
    Print(Expr),
    Call(Expr),
    Return(Option<Expr>),
    If(Expr, Block, Block),
    While(Expr, Block),
    Break,
    Continue,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub public: bool,
    pub params: Vec<(String, Type)>,
    pub result: Type,
    pub body: Block,
    pub position: SourcePosition,
}

#[derive(Clone, Debug)]
pub struct Struct {
    pub public: bool,
    pub fields: Vec<(String, Type)>,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub structs: BTreeMap<String, Struct>,
    pub type_names: BTreeMap<u64, String>,
    pub imports: Vec<String>,
    pub functions: BTreeMap<String, Function>,
}

type SourcePosition = (usize, usize);
// 语句序列：每条语句带上它在源码中的起始位置，供语义错误定位。
pub type Block = Vec<(Instruction, SourcePosition)>;

fn lex(source: &str) -> Result<(Vec<Token>, Vec<SourcePosition>), String> {
    let mut position = (1, 1);
    let mut chars = source
        .chars()
        .map(|c| {
            let start = position;
            if c == '\n' {
                position = (position.0 + 1, 1);
            } else {
                position.1 += 1;
            }
            (c, start)
        })
        .peekable();
    let mut tokens = Vec::new();
    let mut lines = Vec::new();
    while let Some((c, (line, column))) = chars.next() {
        let before = tokens.len();
        let result: Result<(), String> = (|| {
            match c {
                '\n' => tokens.push(Token::Newline),
                c if c.is_whitespace() => {}
                '#' => {
                    while chars.peek().is_some_and(|c| c.0 != '\n') {
                        chars.next();
                    }
                }
                '/' if chars.peek().is_some_and(|(c, _)| *c == '/') => {
                    chars.next();
                    while chars.peek().is_some_and(|c| c.0 != '\n') {
                        chars.next();
                    }
                }
                '"' => {
                    let mut text = String::new();
                    let mut closed = false;
                    while let Some((c, _)) = chars.next() {
                        match c {
                            '"' => {
                                closed = true;
                                break;
                            }
                            '\\' => text.push(match chars.next().map(|(c, _)| c) {
                                Some('n') => '\n',
                                Some('r') => '\r',
                                Some('t') => '\t',
                                Some('"') => '"',
                                Some('\\') => '\\',
                                _ => return Err("unsupported string escape".into()),
                            }),
                            '\n' | '\r' => return Err("newline in string literal".into()),
                            c => text.push(c),
                        }
                    }
                    if !closed {
                        return Err("unterminated string literal".into());
                    }
                    tokens.push(Token::Text(text));
                }
                c if c.is_ascii_alphabetic() || c == '_' => {
                    let mut word = String::from(c);
                    while chars
                        .peek()
                        .is_some_and(|(c, _)| c.is_ascii_alphanumeric() || *c == '_')
                    {
                        word.push(chars.next().unwrap().0);
                    }
                    tokens.push(Token::Word(word));
                }
                c if c.is_ascii_digit() => {
                    let mut number = String::from(c);
                    while chars
                        .peek()
                        .is_some_and(|(c, _)| c.is_ascii_digit() || *c == '_')
                    {
                        number.push(chars.next().unwrap().0);
                    }
                    if number.ends_with('_') || number.contains("__") {
                        return Err("integer separators must occur between digits".into());
                    }
                    tokens.push(Token::Number(number.replace('_', "")));
                }
                '(' | ')' | '{' | '}' | '[' | ']' | '/' | '.' | ':' | ',' | '=' | '&' | '-'
                | '>' | '<' | '+' | '*' | '%' | '!' | '|' | ';' => tokens.push(Token::Symbol(c)),
                _ => return Err(format!("unexpected character {c:?}")),
            }
            Ok(())
        })();
        result.map_err(|error| format!("line {line}, column {column}: {error}"))?;
        if tokens.len() > before {
            lines.push((line, column));
        }
    }
    lines.push(position);
    Ok((tokens, lines))
}

struct Parser {
    tokens: Vec<Token>,
    lines: Vec<SourcePosition>,
    position: usize,
    type_names: BTreeMap<u64, String>,
}
impl Parser {
    fn at(&self) -> SourcePosition {
        self.lines[self.position.min(self.lines.len() - 1)]
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
            Err(format!("expected {c:?}, found {:?}", self.peek()))
        }
    }
    fn word(&mut self) -> Result<String, String> {
        match self.take() {
            Some(Token::Word(word)) => Ok(word),
            token => Err(format!("expected identifier, found {token:?}")),
        }
    }
    fn newlines(&mut self) {
        while self.peek() == Some(&Token::Newline) {
            self.take();
        }
    }
    fn separator(&mut self) -> Result<(), String> {
        match self.peek() {
            None | Some(Token::Newline) | Some(Token::Symbol('}')) => {
                self.newlines();
                Ok(())
            }
            token => Err(format!(
                "expected newline between statements, found {token:?}"
            )),
        }
    }
    fn ty(&mut self) -> Result<Type, String> {
        let ty = if self.symbol('&') {
            if self.keyword("mut") {
                Type::Mutable
            } else {
                Type::Shared
            }
        } else {
            Type::Owned
        };
        if ty == Type::Owned && self.peek() == Some(&Token::Symbol('[')) {
            self.take();
            if !self.keyword("i64") || !self.symbol(';') {
                return Err("only [i64; N] arrays are supported".into());
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
        let mut name = self.word()?;
        while self.symbol('.') {
            name.push('.');
            name.push_str(&self.word()?);
        }
        match name.as_str() {
            "string" => Ok(ty),
            "i64" if ty == Type::Owned => Ok(Type::Int),
            "bool" if ty == Type::Owned => Ok(Type::Bool),
            _ if ty == Type::Owned
                && name
                    .rsplit('.')
                    .next()
                    .is_some_and(|n| n.starts_with(char::is_uppercase)) =>
            {
                let id = record_id(&name);
                self.type_names.insert(id, name);
                Ok(Type::Record(id))
            }
            _ => Err("unsupported type; expected string, i64, bool or owned struct".into()),
        }
    }
    fn expr(&mut self) -> Result<Expr, String> {
        self.binary(0)
    }
    fn binary(&mut self, minimum: u8) -> Result<Expr, String> {
        let mut left = self.atom()?;
        loop {
            let (op, priority, width) = match self.peek() {
                Some(Token::Symbol(c @ ('&' | '|')))
                    if self.tokens.get(self.position + 1) == Some(&Token::Symbol(*c)) =>
                {
                    (format!("{c}{c}"), if *c == '|' { 1 } else { 2 }, 2)
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
                Some(Token::Symbol(c @ ('+' | '-'))) => (c.to_string(), 4, 1),
                Some(Token::Symbol(c @ ('*' | '/' | '%'))) => (c.to_string(), 5, 1),
                _ => break,
            };
            if priority < minimum {
                break;
            }
            self.position += width;
            let right = self.binary(priority + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn atom(&mut self) -> Result<Expr, String> {
        if self.symbol('(') {
            let expr = self.expr()?;
            self.expect(')')?;
            return Ok(expr);
        }
        if self.symbol('-') {
            if let Some(Token::Number(number)) = self.peek().cloned() {
                self.take();
                return Ok(Expr::Int(
                    format!("-{number}")
                        .parse()
                        .map_err(|_| "i64 literal overflow")?,
                ));
            }
            return Ok(Expr::Unary('-', Box::new(self.atom()?)));
        }
        if self.symbol('!') {
            return Ok(Expr::Unary('!', Box::new(self.atom()?)));
        }
        if let Some(Token::Number(number)) = self.peek().cloned() {
            self.take();
            return Ok(Expr::Int(
                number.parse().map_err(|_| "i64 literal overflow")?,
            ));
        }
        if self.keyword("true") {
            return Ok(Expr::Bool(true));
        }
        if self.keyword("false") {
            return Ok(Expr::Bool(false));
        }
        if let Some(Token::Text(text)) = self.peek().cloned() {
            self.take();
            return literal(&text);
        }
        if self.symbol('[') {
            let mut values = Vec::new();
            if !self.symbol(']') {
                loop {
                    values.push(self.expr()?);
                    if self.symbol(']') {
                        break;
                    }
                    self.expect(',')?;
                }
            }
            return Ok(Expr::Array(values));
        }
        if self.symbol('&') {
            let mutable = self.keyword("mut");
            let mut name = self.word()?;
            while self.symbol('.') {
                name.push('.');
                name.push_str(&self.word()?);
            }
            return Ok(Expr::Borrow(name, mutable));
        }
        let mut name = self.word()?;
        while self.symbol('.') {
            name.push('.');
            name.push_str(&self.word()?);
        }
        if name
            .rsplit('.')
            .next()
            .is_some_and(|n| n.starts_with(char::is_uppercase))
            && self.symbol('{')
        {
            self.newlines();
            let mut fields = Vec::new();
            while !self.symbol('}') {
                let field = self.word()?;
                self.expect(':')?;
                fields.push((field, self.expr()?));
                self.separator()?;
            }
            return Ok(Expr::Record(name, fields));
        }
        if !self.symbol('(') {
            if self.symbol('[') {
                let index = self.expr()?;
                self.expect(']')?;
                return Ok(Expr::Index(name, Box::new(index)));
            }
            if let Some((local, field)) = name.split_once('.') {
                return Ok(Expr::Field(local.into(), field.into()));
            }
            return Ok(Expr::Variable(name));
        }
        let mut args = Vec::new();
        if !self.symbol(')') {
            loop {
                args.push(self.expr()?);
                if self.symbol(')') {
                    break;
                }
                self.expect(',')?;
            }
        }
        Ok(Expr::Call(name, args))
    }
    fn if_statement(&mut self, imports: &[String], result: Type) -> Result<Instruction, String> {
        let condition = self.expr()?;
        let yes = self.block(imports, result)?;
        let boundary = self.position;
        self.newlines();
        let no = if self.keyword("else") {
            let position = self.at();
            if self.keyword("if") {
                vec![(self.if_statement(imports, result)?, position)]
            } else {
                self.block(imports, result)?
            }
        } else {
            self.position = boundary;
            Vec::new()
        };
        Ok(Instruction::If(condition, yes, no))
    }

    fn block(&mut self, imports: &[String], result: Type) -> Result<Block, String> {
        self.expect('{')?;
        self.newlines();
        let mut body = Vec::new();
        while self.peek() != Some(&Token::Symbol('}')) {
            if self.peek().is_none() {
                return Err("unterminated function block".into());
            }
            let position = self.at();
            let instruction = if self.keyword("if") {
                body.push((self.if_statement(imports, result)?, position));
                self.separator()?;
                continue;
            } else if self.keyword("while") {
                let condition = self.expr()?;
                let nested = self.block(imports, result)?;
                Instruction::While(condition, nested)
            } else if self.keyword("break") {
                Instruction::Break
            } else if self.keyword("continue") {
                Instruction::Continue
            } else if self.keyword("let") {
                let mutable = self.keyword("mut");
                let local = self.word()?;
                let annotation = if self.symbol(':') {
                    Some(self.ty()?)
                } else {
                    None
                };
                if annotation.is_some_and(|ty| matches!(ty, Type::Shared | Type::Mutable)) {
                    return Err("borrow escape: references cannot be stored in locals".into());
                }
                self.expect('=')?;
                Instruction::Let(local, mutable, annotation, self.expr()?)
            } else if self.keyword("return") {
                let expr = if matches!(
                    self.peek(),
                    None | Some(Token::Newline) | Some(Token::Symbol('}'))
                ) {
                    None
                } else {
                    Some(self.expr()?)
                };
                Instruction::Return(expr)
            } else {
                let expr = self.expr()?;
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
                    } else if result != Type::Unit
                        && self.tokens[self.position..]
                            .iter()
                            .find(|token| **token != Token::Newline)
                            == Some(&Token::Symbol('}'))
                    {
                        Instruction::Return(Some(expr))
                    } else {
                        Instruction::Call(expr)
                    }
                } else {
                    Instruction::Return(Some(expr))
                }
            };
            body.push((instruction, position));
            self.separator()?;
        }
        self.take();
        Ok(body)
    }
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
                let valid = !path.is_empty()
                    && path.split('.').all(|part| {
                        part.starts_with(|c: char| c.is_alphabetic() || c == '_')
                            && part.chars().all(|c| c.is_alphanumeric() || c == '_')
                    });
                if !valid {
                    return Err(format!(
                        "unsupported interpolation {{{inner}}}: expected a variable or field path"
                    ));
                }
                segments.push(match path.split_once('.') {
                    Some((root, field)) => Expr::Field(root.into(), field.into()),
                    None => Expr::Variable(path.into()),
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
    let (tokens, lines) = lex(source)?;
    let mut p = Parser {
        tokens,
        lines,
        position: 0,
        type_names: BTreeMap::new(),
    };
    let result = (|| {
        let mut module = Module {
            structs: BTreeMap::new(),
            type_names: BTreeMap::new(),
            imports: Vec::new(),
            functions: BTreeMap::new(),
        };
        p.newlines();
        while p.peek().is_some() {
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
            let public = p.keyword("pub");
            let start = p.at();
            if p.keyword("struct") {
                let name = p.word()?;
                if matches!(name.as_str(), "string" | "i64" | "bool") {
                    return Err(format!("reserved type name {name}"));
                }
                if !name.starts_with(char::is_uppercase) {
                    return Err("struct names must start with an uppercase letter".into());
                }
                p.expect('{')?;
                p.newlines();
                let mut fields = Vec::new();
                let mut names = BTreeSet::new();
                while !p.symbol('}') {
                    let field = p.word()?;
                    if !names.insert(field.clone()) {
                        return Err(format!("duplicate field {name}.{field}"));
                    }
                    p.expect(':')?;
                    let ty = p.ty()?;
                    if matches!(ty, Type::Shared | Type::Mutable) {
                        return Err("borrow escape: structs cannot store references".into());
                    }
                    if matches!(ty, Type::ArrayInt(_)) {
                        return Err("arrays are only supported as local values".into());
                    }
                    fields.push((field, ty));
                    p.separator()?;
                }
                if module.functions.contains_key(&name)
                    || module
                        .structs
                        .insert(name.clone(), Struct { public, fields })
                        .is_some()
                {
                    return Err(format!("duplicate declaration {name}"));
                }
                p.separator()?;
                continue;
            }
            if !p.keyword("fn") {
                return Err("expected use, struct or fn declaration".into());
            }
            let name = p.word()?;
            if module.structs.contains_key(&name) {
                return Err(format!("duplicate declaration {name}"));
            }
            p.expect('(')?;
            let mut params = Vec::new();
            if !p.symbol(')') {
                loop {
                    let param = p.word()?;
                    p.expect(':')?;
                    let ty = p.ty()?;
                    if matches!(ty, Type::ArrayInt(_)) {
                        return Err("arrays are only supported as local values".into());
                    }
                    params.push((param, ty));
                    if p.symbol(')') {
                        break;
                    }
                    p.expect(',')?;
                }
            }
            let result = if p.symbol('-') {
                p.expect('>')?;
                let ty = p.ty()?;
                if matches!(ty, Type::Shared | Type::Mutable) {
                    return Err("borrow escape: functions cannot return references".into());
                }
                if matches!(ty, Type::ArrayInt(_)) {
                    return Err("arrays are only supported as local values".into());
                }
                ty
            } else {
                Type::Unit
            };
            let body = p.block(&module.imports, result)?;
            if module
                .functions
                .insert(
                    name.clone(),
                    Function {
                        public,
                        params,
                        result,
                        body,
                        position: start,
                    },
                )
                .is_some()
            {
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
        .map(|(name, structure)| (name, structure.public, &structure.fields))
        .collect();
    format!(
        "ZAM-IR-9\n{declarations:?}\n{:?}\n{:?}\n{:?}\n",
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
}
impl Checker<'_> {
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
    fn ty(&self, expr: &Expr) -> Result<Type, String> {
        match expr {
            Expr::Record(name, _) => {
                self.structs
                    .get(name)
                    .ok_or_else(|| format!("unknown struct {name}"))?;
                Ok(Type::Record(record_id(name)))
            }
            Expr::Field(local, field) => {
                field_path(self.structs, self.binding(local)?.ty, field).map(|(_, ty)| ty)
            }
            Expr::Array(values) => Ok(Type::ArrayInt(values.len())),
            Expr::Index(local, _) => match self.binding(local)?.ty {
                Type::ArrayInt(_) => Ok(Type::Int),
                _ => Err("index requires i64 array".into()),
            },
            Expr::Text(_) => Ok(Type::Owned),
            Expr::Format(_) => Ok(Type::Owned),
            Expr::Int(_) | Expr::ByteLen(_) => Ok(Type::Int),
            Expr::Bool(_) | Expr::StringEqual(_, _) => Ok(Type::Bool),
            Expr::Variable(name) => Ok(self.binding(name)?.ty),
            Expr::Borrow(_, mutable) => Ok(if *mutable {
                Type::Mutable
            } else {
                Type::Shared
            }),
            Expr::Call(name, _) => self
                .functions
                .get(name)
                .map(|f| f.result)
                .ok_or_else(|| format!("unknown function {name}")),
            Expr::Unary(op, _) => Ok(if *op == '!' { Type::Bool } else { Type::Int }),
            Expr::Binary(op, _, _) => Ok(if matches!(op.as_str(), "+" | "-" | "*" | "/" | "%") {
                Type::Int
            } else {
                Type::Bool
            }),
        }
    }
    fn expr(
        &mut self,
        expr: &Expr,
        expected: Type,
        loans: &mut BTreeMap<String, Type>,
    ) -> Result<(), String> {
        match expr {
            Expr::Record(name, fields) => {
                if expected != Type::Record(record_id(name)) {
                    return Err("struct type mismatch".into());
                }
                let declaration = self
                    .structs
                    .get(name)
                    .ok_or("unknown struct")?
                    .fields
                    .clone();
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
            Expr::Array(values) => {
                if !matches!(expected, Type::ArrayInt(n) if n == values.len()) {
                    return Err("array length or element type mismatch".into());
                }
                for value in values {
                    self.expr(value, Type::Int, loans)?;
                }
                Ok(())
            }
            Expr::Index(local, index) => {
                if expected != Type::Int {
                    return Err("type mismatch: array indexing returns i64".into());
                }
                let length = match self.binding(local)?.ty {
                    Type::ArrayInt(length) => length,
                    _ => return Err("index requires i64 array".into()),
                };
                if let Expr::Int(value) = index.as_ref() {
                    if *value < 0 || (*value as u128) >= length as u128 {
                        return Err("array index out of bounds".into());
                    }
                }
                self.expr(index, Type::Int, loans)
            }
            Expr::Field(_, _) => {
                if self.ty(expr)? != expected {
                    return Err("field type mismatch".into());
                }
                if matches!(expected, Type::Owned | Type::Record(_)) {
                    return Err("owned field moves are not supported; move the whole struct".into());
                }
                Ok(())
            }
            Expr::Variable(_name) if matches!(expected, Type::ArrayInt(_)) => {
                Err("array values cannot be moved or copied yet".into())
            }
            Expr::Variable(name) if matches!(expected, Type::Record(_)) => {
                if self.binding(name)?.ty != expected {
                    return Err("struct type mismatch".into());
                }
                if loans.contains_key(name) {
                    return Err("borrow conflict".into());
                }
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
            Expr::Format(segments) if expected == Type::Owned => {
                for segment in segments {
                    // 插值只读取变量，不移动；未支持的类型在检查阶段拒绝。
                    if !matches!(self.ty(segment)?, Type::Owned | Type::Int | Type::Bool) {
                        return Err("cannot interpolate this value".into());
                    }
                }
                Ok(())
            }
            Expr::Text(_) if expected == Type::Owned => Ok(()),
            Expr::Int(_) if expected == Type::Int => Ok(()),
            Expr::Bool(_) if expected == Type::Bool => Ok(()),
            Expr::Variable(name) if matches!(expected, Type::Int | Type::Bool) => {
                if self.binding(name)?.ty == expected {
                    Ok(())
                } else {
                    Err("type mismatch".into())
                }
            }
            Expr::Unary(op, value) => {
                let ty = if *op == '!' { Type::Bool } else { Type::Int };
                if expected != ty {
                    return Err("type mismatch".into());
                }
                self.expr(value, ty, loans)
            }
            Expr::Binary(op, left, right) => {
                let ty = self.ty(expr)?;
                if expected != ty {
                    return Err("type mismatch".into());
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
                let operand = if matches!(op.as_str(), "==" | "!=") {
                    self.ty(left)?
                } else {
                    Type::Int
                };
                if !matches!(operand, Type::Int | Type::Bool) {
                    return Err("comparison requires i64 or bool".into());
                }
                self.expr(left, operand, loans)?;
                self.expr(right, operand, loans)
            }
            Expr::Variable(name) if expected == Type::Owned => {
                if self.binding(name)?.ty != Type::Owned {
                    return Err(format!("cannot move borrowed value {name}"));
                }
                if loans.contains_key(name) {
                    return Err(format!(
                        "borrow conflict: cannot move {name} during a borrow"
                    ));
                }
                self.locals.get_mut(name).unwrap().live = false;
                Ok(())
            }
            Expr::Variable(name) | Expr::Borrow(name, _)
                if matches!(expected, Type::Shared | Type::Mutable) =>
            {
                let (root, field) = name
                    .split_once('.')
                    .map_or((name.as_str(), None), |(root, field)| (root, Some(field)));
                let binding = self.binding(root)?;
                let ty = if let Some(field) = field {
                    field_path(self.structs, binding.ty, field)?.1
                } else {
                    binding.ty
                };
                if matches!(ty, Type::Int | Type::Bool | Type::Record(_)) {
                    return Err("scalar references are not supported".into());
                }
                let requested = match expr {
                    Expr::Borrow(_, true) => Type::Mutable,
                    Expr::Borrow(_, false) => Type::Shared,
                    _ if ty != Type::Owned => expected,
                    _ => return Err(format!("expected explicit borrow of {name}")),
                };
                if expected != requested {
                    return Err(format!("borrow type mismatch for {name}"));
                }
                if requested == Type::Mutable
                    && !(binding.ty == Type::Mutable || (ty == Type::Owned && binding.mutable))
                {
                    return Err(format!("cannot mutably borrow immutable value {name}"));
                }
                if loans
                    .get(root)
                    .is_some_and(|old| *old == Type::Mutable || requested == Type::Mutable)
                {
                    return Err(format!("borrow conflict: {name}"));
                }
                // ponytail: loans cover the root; split field loans if disjoint mutation is needed.
                loans.insert(root.into(), requested);
                Ok(())
            }
            Expr::Call(name, args) => {
                let callee = self
                    .functions
                    .get(name)
                    .ok_or_else(|| format!("unknown function {name}"))?
                    .clone();
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
            Expr::Borrow(_, _) => {
                Err("borrow escape: references only belong in call arguments".into())
            }
            _ => Err("string type mismatch".into()),
        }
    }
    fn block(&mut self, body: &Block, result: Type) -> Result<bool, String> {
        let outer_names = self.locals.keys().cloned().collect();
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
                    let ty = annotation.unwrap_or(self.ty(expr)?);
                    if matches!(ty, Type::Shared | Type::Mutable | Type::Unit) {
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
                    if !self.binding(local)?.mutable {
                        return Err(format!("cannot assign immutable value {local}"));
                    }
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
                    if !(binding.mutable || binding.ty == Type::Mutable) {
                        return Err(format!("cannot assign immutable value {local}"));
                    }
                    let ty = if binding.ty == Type::Mutable {
                        Type::Owned
                    } else {
                        binding.ty
                    };
                    if matches!(ty, Type::ArrayInt(_)) {
                        return Err("array values cannot be moved or copied yet".into());
                    }
                    self.expr(expr, ty, &mut loans)?;
                    self.locals.get_mut(local).unwrap().live = true;
                }
                Instruction::Print(expr) => {
                    if matches!(
                        self.ty(expr)?,
                        Type::Unit | Type::Record(_) | Type::ArrayInt(_)
                    ) {
                        return Err("cannot print void, struct or array".into());
                    }
                    if let Expr::Variable(local) = expr {
                        self.binding(local)?;
                    } else if let Expr::Field(_, _) = expr {
                        self.ty(expr)?;
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
                        if matches!(ty, Type::Unit | Type::Record(_) | Type::ArrayInt(_)) {
                            return Err("cannot print void, struct or array".into());
                        }
                        self.expr(expr, ty, &mut loans)?;
                    }
                }
                Instruction::Call(expr) => {
                    let ty = if let Expr::Call(callee, _) = expr {
                        self.functions
                            .get(callee)
                            .ok_or_else(|| format!("unknown function {callee}"))?
                            .result
                    } else if matches!(expr, Expr::ByteLen(_) | Expr::StringEqual(_, _)) {
                        self.ty(expr)?
                    } else {
                        return Err("expected function call".into());
                    };
                    self.expr(expr, ty, &mut loans)?;
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
        let outer: BTreeSet<_> = outer_names;
        self.locals.retain(|name, _| outer.contains(name));
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
            if let Type::Record(id) = ty {
                let child = module
                    .structs
                    .keys()
                    .find(|name| record_id(name) == *id)
                    .ok_or("unknown field type")?;
                visit(module, child, active, done, order)?;
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

pub fn check(module: &Module, entry: &str) -> Result<(), String> {
    record_order(module, &module.structs.keys().cloned().collect())?;
    let main = module.functions.get(entry).ok_or("missing main function")?;
    if !main.params.is_empty() || main.result != Type::Unit {
        return Err("main must have no parameters or return value".into());
    }
    for (name, function) in &module.functions {
        let mut checker = Checker {
            structs: &module.structs,
            functions: &module.functions,
            locals: BTreeMap::new(),
            loops: Vec::new(),
            position: function.position,
        };
        let result = (|| {
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
        result.map_err(|error: String| {
            let (line, column) = checker.position;
            format!("line {line}, column {column}: {name}: {error}")
        })?;
    }
    Ok(())
}
