use crate::compiler::{self, Block, Expr, Instruction, Module};
use crate::native;
use crate::package;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub type Result<T> = std::result::Result<T, String>;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);
const VERSION: &str = concat!("zam-", env!("CARGO_PKG_VERSION"), "-native-c-59");

pub fn hash(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn cache_dir() -> PathBuf {
    env::var_os("ZAMAK_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                env::var_os("USERPROFILE")
                    .or_else(|| env::var_os("HOME"))
                    .unwrap_or_else(|| ".".into()),
            )
            .join(".zam/cache")
        })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("output has no parent directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = parent.join(format!(
        ".zam-{}-{}.tmp",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        match fs::hard_link(&temp, path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = fs::read(path).map_err(|e| e.to_string())?;
                if existing == bytes {
                    Ok(())
                } else {
                    Err(format!("corrupt cache entry {}", path.display()))
                }
            }
            Err(e) => Err(e.to_string()),
        }
    })();
    let cleanup = fs::remove_file(&temp);
    if result.is_ok() && temp.exists() {
        cleanup.map_err(|e| e.to_string())?;
    }
    result
}

struct Input {
    source: String,
    module: Module,
    // `zam fmt` 要落盘；模块 id 与文件名不一定同名（单文件入口恒为 main）。
    path: PathBuf,
    // 来自哪个依赖包；项目自己的模块是 None。
    package: Option<String>,
}

pub struct Project {
    pub name: String,
    pub root: PathBuf,
    entry: String,
    // 直接检查单个 .zm 文件（不是项目）：报错里只有一个模块，函数名前的 `main:` 前缀是多余的。
    single: bool,
    modules: BTreeMap<String, Input>,
    // 项目与各个依赖包的 `[native]`：（包的根目录, 设置）。
    native: Vec<(PathBuf, package::Native)>,
    // 库：`zam.toml` 没写 entry，没有 main，只能检查和测试。`entry` 是它的第一个模块，测试的
    // 入口生成在那里。
    pub library: bool,
    pub version: Option<crate::registry::Version>,
    pub description: Option<String>,
}

/// 编辑器里还没保存的内容（`zam lsp`）：规范化路径 → 文本。加载项目时优先读这里，
/// 所以边打字边检查不用先存盘。命令行用不到它，始终为空。
static OVERLAY: std::sync::RwLock<BTreeMap<PathBuf, String>> =
    std::sync::RwLock::new(BTreeMap::new());

fn overlay_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 设置或清除（`None`）某个文件的未保存内容。
pub fn set_overlay(path: &Path, text: Option<String>) {
    let mut overlay = OVERLAY.write().unwrap();
    match text {
        Some(text) => overlay.insert(overlay_key(path), text),
        None => overlay.remove(&overlay_key(path)),
    };
}

/// 某个文件的未保存内容（没有就是 None）。
pub fn read_overlay(path: &Path) -> Option<String> {
    OVERLAY.read().unwrap().get(&overlay_key(path)).cloned()
}

fn read_source(path: &Path) -> std::io::Result<String> {
    {
        let overlay = OVERLAY.read().unwrap();
        if !overlay.is_empty() {
            if let Some(text) = overlay.get(&overlay_key(path)) {
                return Ok(text.clone());
            }
        }
    }
    fs::read_to_string(path)
}

fn is_source(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "zm")
}

/// 单文件项目只有一个模块，报错里 `line L, column C: <模块>:<函数>: <信息>` 的 `<模块>` 是多余
/// 的（文件名已经指向它），去掉后读作 `line L, column C: <信息>`。
/// 消息正文里的 `模块:函数`（例如参数为 `main:f`）保留：那些是名字，不是位置标记。
fn strip_function_module(error: &str) -> String {
    let Some((position, rest)) = error.split_once(": ") else {
        return error.to_owned();
    };
    let Some((head, message)) = rest.split_once(": ") else {
        return error.to_owned();
    };
    match head.split_once(':') {
        // `main:main: ...`：模块与函数名都去掉。
        Some((module, name)) if is_identifier(module, true) && is_identifier(name, false) => {
            format!("{position}: {message}")
        }
        // `main: borrow conflict: ...`：错误本身已经没有函数名，只去掉模块名。
        None if is_identifier(head, true) => format!("{position}: {message}"),
        _ => error.to_owned(),
    }
}

/// 诊断里的 `<模块>:<函数>` 前缀只用标识符字符（模块路径可带 `/`）。
fn is_identifier(text: &str, slash: bool) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || (slash && c == '/'))
}

type Parse = fn(&str) -> std::result::Result<compiler::Module, String>;

fn scan(
    dir: &Path,
    base: &Path,
    modules: &mut BTreeMap<String, Input>,
    parse: Parse,
) -> Result<()> {
    for item in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let item = item.map_err(|e| e.to_string())?;
        let kind = item.file_type().map_err(|e| e.to_string())?;
        let path = item.path();
        if kind.is_symlink() {
            return Err(format!(
                "symlink source is not supported: {}",
                path.display()
            ));
        }
        if kind.is_dir() {
            scan(&path, base, modules, parse)?;
        } else if is_source(&path) {
            let id = path
                .strip_prefix(base)
                .unwrap()
                .with_extension("")
                .to_str()
                .ok_or("module path must be UTF-8")?
                .replace('\\', "/");
            if !id.split('/').all(identifier) {
                return Err(format!("invalid module path {id}"));
            }
            let source = read_source(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let module = parse(&source).map_err(|e| format!("{}: {e}", path.display()))?;
            if modules
                .insert(
                    id.clone(),
                    Input {
                        source,
                        module,
                        path,
                        package: None,
                    },
                )
                .is_some()
            {
                return Err(format!("duplicate module {id}"));
            }
        }
    }
    Ok(())
}

/// 把依赖包的模块挂到包名下：`src/geo.zm` 是 `包名/geo`，与包同名的 `src/包名.zm` 就是 `包名`。
/// 包内部的 `use geo` 改写成限定后的名字；指向别的包或标准库的导入不变。
fn load_package(
    name: &str,
    package: &package::Package,
    modules: &mut BTreeMap<String, Input>,
    parse: Parse,
) -> Result<()> {
    let base = package.dir.join(&package.manifest.modules);
    let mut own = BTreeMap::new();
    scan(&base, &base, &mut own, parse)?;
    let qualified = |id: &str| {
        if id == name {
            name.to_owned()
        } else {
            format!("{name}/{id}")
        }
    };
    let local: BTreeSet<String> = own.keys().cloned().collect();
    for (id, mut input) in own {
        for import in &mut input.module.imports {
            if local.contains(import.as_str()) {
                *import = qualified(import);
            }
        }
        input.package = Some(name.to_owned());
        let id = qualified(&id);
        if modules.insert(id.clone(), input).is_some() {
            return Err(format!("duplicate module {id}"));
        }
    }
    Ok(())
}

pub fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Project {
    pub fn load(input: &Path) -> Result<Self> {
        Self::load_with(input, compiler::parse, Some(false))
    }

    /// `zam update`：忽略 `zam.lock`，把每个 git 依赖的 rev 重新解析成最新的提交。
    pub fn load_updating(input: &Path) -> Result<Self> {
        Self::load_with(input, compiler::parse, Some(true))
    }

    /// `zam fmt` 用：行首 Tab 先展开再解析，Tab 缩进的文件也能被格式化。只整理项目自己的模块，
    /// 不读依赖。
    pub fn load_for_format(input: &Path) -> Result<Self> {
        Self::load_with(
            input,
            |source| compiler::parse(&compiler::expand_tabs(source)),
            None,
        )
    }

    /// 清单文件的路径：目录下的 `zam.toml`，或者直接给出的文件。
    pub fn manifest_path(input: &Path) -> PathBuf {
        if input.is_dir() {
            input.join("zam.toml")
        } else {
            input.into()
        }
    }

    /// `dependencies` 为 None 时不解析依赖（格式化），Some(update) 时解析，update 表示忽略锁文件。
    fn load_with(input: &Path, parse: Parse, dependencies: Option<bool>) -> Result<Self> {
        if is_source(input) {
            let source = read_source(input).map_err(|e| e.to_string())?;
            let module = parse(&source).map_err(|e| format!("{}: {e}", input.display()))?;
            return Ok(Self {
                name: "main".into(),
                root: input.parent().unwrap_or(Path::new(".")).into(),
                entry: "main".into(),
                single: true,
                modules: BTreeMap::from([(
                    "main".into(),
                    Input {
                        source,
                        module,
                        path: input.to_path_buf(),
                        package: None,
                    },
                )]),
                native: Vec::new(),
                library: false,
                version: None,
                description: None,
            });
        }
        let manifest = Self::manifest_path(input);
        let config = package::manifest(&manifest, true)?;
        let name = config.name.clone();
        let module_path = config.modules.clone();
        let entry = match &config.entry {
            Some(entry_path) => {
                let entry = entry_path
                    .strip_prefix(&module_path)
                    .map_err(|_| "entry must be inside modules")?;
                if !is_source(entry) {
                    return Err("entry must be a .zm file".into());
                }
                Some(
                    entry
                        .with_extension("")
                        .to_str()
                        .ok_or("entry must be UTF-8")?
                        .replace('\\', "/"),
                )
            }
            None => None,
        };
        let root = manifest.parent().unwrap_or(Path::new(".")).to_owned();
        let base = root.join(module_path);
        let mut modules = BTreeMap::new();
        scan(&base, &base, &mut modules, parse)?;
        let library = entry.is_none();
        let entry = match entry {
            Some(entry) if modules.contains_key(&entry) => entry,
            Some(entry) => return Err(format!("missing entry module {entry}")),
            // 库：按名字排第一的模块（与包同名的根模块优先）。
            None => {
                if modules.contains_key(&name) {
                    name.clone()
                } else {
                    modules
                        .keys()
                        .next()
                        .cloned()
                        .ok_or_else(|| format!("{} has no .zm files", base.display()))?
                }
            }
        };
        let mut native = vec![(root.clone(), config.native.clone())];
        if let Some(update) = dependencies {
            let packages = package::resolve(
                &root,
                &config.dependencies,
                &cache_dir().join("packages"),
                update,
            )?;
            for (dependency, package) in &packages {
                if let Some(id) = modules
                    .keys()
                    .find(|id| *id == dependency || id.starts_with(&format!("{dependency}/")))
                {
                    return Err(format!(
                        "module {id} has the same name as the dependency {dependency}; rename one of them"
                    ));
                }
                load_package(dependency, package, &mut modules, parse)?;
                native.push((package.dir.clone(), package.manifest.native.clone()));
            }
        }
        Ok(Self {
            name,
            root,
            entry,
            single: false,
            modules,
            native,
            library,
            version: config.version,
            description: config.description.clone(),
        })
    }

    fn resolve(&self, module: &str, call: &str) -> Result<String> {
        // `Point.origin()`：本模块结构体或枚举里声明的函数，函数表里就叫 `Point.origin`。
        let owner = call
            .split_once('.')
            .is_some_and(|(alias, _)| self.modules[module].module.structs.contains_key(alias));
        let (target, name) = if owner {
            (module, call)
        } else if let Some((alias, name)) = call.split_once('.') {
            let matches: Vec<_> = self.modules[module]
                .module
                .imports
                .iter()
                .filter(|path| path.rsplit('/').next() == Some(alias))
                .collect();
            // 标准库模块忘了导入：直接说要加哪一行。
            if matches.is_empty()
                && matches!(
                    alias,
                    "io" | "fmt" | "string" | "fs" | "env" | "math" | "time" | "random" | "path"
                )
            {
                return Err(format!(
                    "{module}: {call} requires use std/{alias}; add `use std/{alias}` at the top of the file"
                ));
            }
            if matches.len() != 1 {
                return Err(format!("{module}: ambiguous or missing import for {call}"));
            }
            (matches[0].as_str(), name)
        } else {
            (module, call)
        };
        if target.starts_with("std/") {
            let qualified = format!("{target}:{name}");
            return if native::builtin(&qualified).is_some() {
                Ok(qualified)
            } else {
                Err(format!("{module}: unknown function {call}"))
            };
        }
        let function = self
            .modules
            .get(target)
            .and_then(|input| input.module.functions.get(name))
            .ok_or_else(|| format!("{module}: unknown function {call}"))?;
        if target != module && !function.public {
            return Err(format!("{module}: {call} is private"));
        }
        Ok(format!("{target}:{name}"))
    }

    /// 模块在磁盘上的真实路径，供 `zam fmt` 就地重写。
    pub fn module_paths(&self) -> Vec<PathBuf> {
        self.modules
            .values()
            .filter(|input| input.package.is_none())
            .map(|input| input.path.clone())
            .collect()
    }

    pub fn check(&self) -> Result<()> {
        self.check_with(self.library)
    }

    /// 测试函数：各模块里名字以 `test_` 开头的函数（`模块:test_x`），按名字排序。
    pub fn tests(&self) -> Result<Vec<String>> {
        let mut tests = Vec::new();
        for (id, input) in &self.modules {
            // 依赖包里的测试由包自己运行。
            if input.package.is_some() {
                continue;
            }
            for (name, function) in &input.module.functions {
                if !name.starts_with("test_") {
                    continue;
                }
                if !function.params.is_empty()
                    || function.result != compiler::Type::Unit
                    || !function.generics.is_empty()
                {
                    return Err(self.located(
                        id,
                        function.position,
                        format!(
                            "test function {name} must take no parameters and return nothing, such as fn {name}():"
                        ),
                    ));
                }
                tests.push(format!("{id}:{name}"));
            }
        }
        Ok(tests)
    }

    /// 测试时不要求 `fn main()`。
    fn check_with(&self, testing: bool) -> Result<()> {
        if !testing {
            self.modules[&self.entry]
                .module
                .functions
                .get("main")
                .ok_or("entry module requires fn main()")?;
        }
        for (id, input) in &self.modules {
            for import in &input.module.imports {
                if !matches!(
                    import.as_str(),
                    "std/io"
                        | "std/fmt"
                        | "std/string"
                        | "std/fs"
                        | "std/env"
                        | "std/math"
                        | "std/time"
                        | "std/random"
                        | "std/path"
                ) && !self.modules.contains_key(import)
                {
                    return Err(format!("{id}: missing module {import}"));
                }
            }
        }
        let mut keys = BTreeMap::new();
        for id in self.modules.keys() {
            self.key(id, &mut keys, &mut BTreeSet::new())?;
        }
        // 检查时可能推断出新的元组类型（`let t = (1, "a")`）：重新链接后再检查，直到不再出现新的。
        let entry = if testing {
            String::new()
        } else {
            format!("{}:main", self.entry)
        };
        // 泛型函数的新副本同理：副本里再调用泛型函数时会生成更多副本，直到不再增加。
        let result = loop {
            let counts = || {
                (
                    compiler::linked_tuple_count(),
                    compiler::instance_count(),
                    compiler::struct_instance_count(),
                )
            };
            let known = counts();
            let result = compiler::check(&self.linked()?, &entry);
            if counts() == known {
                break result;
            }
        };
        result.map_err(|errors| {
            errors
                .split(compiler::DIAGNOSTIC_SEPARATOR)
                .map(|error| self.located_function(error.to_owned()))
                .collect::<Vec<_>>()
                .join(&compiler::DIAGNOSTIC_SEPARATOR.to_string())
        })
    }

    fn linked(&self) -> Result<Module> {
        let mut linked = Module {
            structs: BTreeMap::new(),
            type_names: BTreeMap::new(),
            imports: Vec::new(),
            functions: BTreeMap::new(),
            constants: BTreeMap::new(),
        };
        for (id, input) in &self.modules {
            for (name, structure) in &input.module.structs {
                let mut structure = structure.clone();
                for (_, ty) in &mut structure.fields {
                    self.resolve_type(id, ty)?;
                }
                linked.structs.insert(format!("{id}:{name}"), structure);
            }
            for (name, function) in &input.module.functions {
                let mut function = function.clone();
                let position = function.position;
                for (_, ty) in &mut function.params {
                    self.resolve_type(id, ty)
                        .map_err(|error| self.located(id, position, error))?;
                }
                self.resolve_type(id, &mut function.result)
                    .map_err(|error| self.located(id, position, error))?;
                if let Some(error) = &mut function.error {
                    self.resolve_type(id, error)
                        .map_err(|error| self.located(id, position, error))?;
                }
                let mut locals: BTreeSet<String> = function
                    .params
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect();
                declared(&function.body, &mut locals);
                // 不支持遮蔽：局部名不能和本模块的常量同名。
                if let Some(name) = locals
                    .iter()
                    .find(|name| input.module.constants.contains_key(name.as_str()))
                {
                    return Err(self.located(
                        id,
                        position,
                        format!("{name} is already a constant; choose another name for the local"),
                    ));
                }
                self.resolve_body(id, &locals, &mut function.body)?;
                linked.functions.insert(format!("{id}:{name}"), function);
            }
        }
        linked.functions.extend(native::builtin_functions());
        // 泛型函数按类型生成的副本（检查时生成，名字形如 `模块:largest[Int]`）。
        linked.functions.extend(compiler::instances());
        // 元组没有声明，按结构登记过的都放进来；字段名是项号 `0`、`1`……
        for (name, fields) in compiler::linked_tuples() {
            linked.structs.insert(
                name,
                compiler::Struct {
                    public: true,
                    fields: fields
                        .into_iter()
                        .enumerate()
                        .map(|(i, ty)| (i.to_string(), ty))
                        .collect(),
                    variants: Vec::new(),
                    generics: Vec::new(),
                },
            );
        }
        linked
            .structs
            .insert(compiler::OPTION.into(), compiler::option());
        // 泛型结构体与枚举按用到的类型实参生成实例（`Option[Int]`），字段里的类型参数换成实参。
        // 实例的字段可能又用到新的实例，检查循环会再链接一遍。
        if compiler::struct_instance_count() > 10_000 {
            return Err(
                "too many generic type instances; a generic type may expand without end".into(),
            );
        }
        for (template, args, name) in compiler::struct_instances() {
            let Some(declaration) = linked.structs.get(&template) else {
                continue;
            };
            let short = template.rsplit(':').next().unwrap_or(&template);
            if declaration.generics.len() != args.len() {
                return Err(format!(
                    "type argument count mismatch for {short}: it takes {}",
                    declaration.generics.len()
                ));
            }
            let mut instance = declaration.clone();
            instance.fields =
                compiler::substitute_fields(&declaration.generics, &args, &declaration.fields)?;
            instance.generics.clear();
            linked.structs.insert(name, instance);
        }
        Ok(linked)
    }

    /// 把类型里的结构体 id（结构体本身、它的借用、列表元素或字典值，包括嵌套容器里的）
    /// 换成按模块限定的 id。
    fn resolve_type(&self, module: &str, ty: &mut compiler::Type) -> Result<()> {
        use compiler::Type;
        match ty {
            Type::Record(id) | Type::RecordRef(id, _) => self.resolve_record(module, id),
            Type::List(elem) | Type::ListRef(elem, _) => self.resolve_elem(module, elem),
            Type::Map(_, value) | Type::MapRef(_, value, _) => self.resolve_elem(module, value),
            // 函数类型：参数与结果限定后重新登记，得到新的编号。
            Type::Func(id) | Type::FuncRef(id) => {
                let (mut params, mut result) = compiler::func_sig(*id);
                for param in &mut params {
                    self.resolve_type(module, param)?;
                }
                self.resolve_type(module, &mut result)?;
                let mut error = compiler::func_error(*id);
                if let Some(error) = &mut error {
                    self.resolve_type(module, error)?;
                }
                *id = compiler::func_type_full(params, result, error);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn resolve_elem(&self, module: &str, elem: &mut compiler::Elem) -> Result<()> {
        use compiler::Elem;
        match elem {
            Elem::Record(id) => self.resolve_record(module, id),
            // 内层类型改写后重新登记，得到新的编号。
            Elem::Nested(id) => {
                let mut inner = compiler::nested(*id);
                self.resolve_type(module, &mut inner)?;
                *elem = compiler::nest(inner);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn resolve_record(&self, module: &str, id: &mut u64) -> Result<()> {
        let name = self.modules[module]
            .module
            .type_names
            .get(id)
            .ok_or("unknown type name")?;
        // 泛型函数的类型参数：调用时才换成具体类型。
        if name.starts_with("$param:") {
            return Ok(());
        }
        // `Option[i64]`：模板名按模块限定，类型实参也限定，换成实例的编号。
        if let Some((template, mut args)) = compiler::application(*id) {
            let mut constructor = Expr::Record(template.clone(), Vec::new());
            self.resolve_expr(module, &BTreeSet::new(), &mut constructor)?;
            let Expr::Record(qualified, _) = constructor else {
                unreachable!("record constructor")
            };
            let count = self.declared_generics(&qualified);
            if count != Some(args.len()) {
                let short = template.rsplit('.').next().unwrap_or(&template);
                return Err(match count {
                    Some(0) | None => format!("{short} is not generic; write it without [...]"),
                    Some(count) => {
                        format!("type argument count mismatch for {short}: it takes {count}")
                    }
                });
            }
            for arg in &mut args {
                self.resolve_type(module, arg)?;
            }
            *id = compiler::struct_instance(&qualified, args);
            return Ok(());
        }
        // 元组：各项类型限定后重新登记，同样的结构在所有模块里是同一个类型。
        if name.starts_with('(') {
            let mut fields = compiler::tuple_fields(*id).ok_or("unknown tuple type")?;
            for field in &mut fields {
                self.resolve_type(module, field)?;
            }
            *id = compiler::linked_tuple(fields);
            return Ok(());
        }
        let mut constructor = Expr::Record(name.clone(), Vec::new());
        self.resolve_expr(module, &BTreeSet::new(), &mut constructor)?;
        if let Expr::Record(qualified, _) = constructor {
            if let Some(count @ 1..) = self.declared_generics(&qualified) {
                let short = name.rsplit('.').next().unwrap_or(name);
                let params = vec!["i64"; count].join(", ");
                return Err(format!(
                    "{short} takes {count} type argument(s); write it as {short}[{params}]"
                ));
            }
            *id = compiler::record_id(&qualified);
        }
        Ok(())
    }

    /// 限定名对应的结构体或枚举声明了几个类型参数；找不到时为 None。
    fn declared_generics(&self, qualified: &str) -> Option<usize> {
        if qualified == compiler::OPTION {
            return Some(1);
        }
        let (module, name) = qualified.rsplit_once(':')?;
        self.modules
            .get(module)?
            .module
            .structs
            .get(name)
            .map(|s| s.generics.len())
    }

    /// 模块解析期错误此前不带位置，AI 只能从 message 里抠模块名。指令级位置已经在
    /// `Block` 里，这里直接复用；粒度与类型检查器一致（都指向语句起始），不假装更精确。
    fn located(&self, module: &str, position: (usize, usize), error: String) -> String {
        // catch 内部的语句已定位，外层表达式不能覆盖更精确的位置。
        if self
            .modules
            .get(module)
            .is_some_and(|input| error.starts_with(&format!("{}: line ", input.path.display())))
        {
            return error;
        }
        // 单文件项目只有一个模块：报错里 `<模块>:` 前缀是多余的（文件名已经指出位置）。
        let error = if self.single {
            match error.strip_prefix(&format!("{module}: ")) {
                Some(rest) => rest.to_owned(),
                None => error,
            }
        } else {
            error
        };
        self.with_module_path(
            module,
            format!("line {}, column {}: {error}", position.0, position.1),
        )
    }

    /// `compiler::check` 的错误自带 `line L, column C`，但函数名是链接后的 `模块:函数`、
    /// 不含文件路径；项目模式下 `main.rs` 因此只能把 `file` 退化成项目目录。这里按前缀
    /// 里的模块补回真实路径，使诊断契约里的 `file` 与解析期错误保持一致。
    fn located_function(&self, error: String) -> String {
        let Some((position, rest)) = error.split_once(": ") else {
            return error;
        };
        if !position.starts_with("line ") {
            return error;
        }
        let Some((module, _)) = rest.split_once(':') else {
            return error;
        };
        let module = module.to_string();
        let error = if self.single {
            strip_function_module(&error)
        } else {
            error
        };
        self.with_module_path(&module, error)
    }

    fn with_module_path(&self, module: &str, error: String) -> String {
        match self.modules.get(module) {
            Some(input) => format!("{}: {error}", input.path.display()),
            None => error,
        }
    }

    fn resolve_body(
        &self,
        module: &str,
        locals: &BTreeSet<String>,
        body: &mut Block,
    ) -> Result<()> {
        for (instruction, position) in body {
            let position = *position;
            // `print(x)` 与 Python 一样打印一行（同 println）；模块自己定义了 print 时用自己的。
            if let Instruction::Call(Expr::Call(name, args)) = instruction {
                if name == "print"
                    && args.len() <= 1
                    && !self.modules[module].module.functions.contains_key("print")
                {
                    let value = args.pop().unwrap_or_else(|| Expr::Text(String::new()));
                    *instruction = Instruction::Print(value);
                }
            }
            if let Instruction::Let(_, _, Some(ty), _) = instruction {
                self.resolve_type(module, ty)
                    .map_err(|error| self.located(module, position, error))?;
            }
            if let Instruction::Assign(name, _)
            | Instruction::AssignField(name, _, _)
            | Instruction::AssignIndex(name, _, _)
            | Instruction::AssignIndexField(name, _, _, _) = instruction
            {
                if !locals.contains(name.as_str())
                    && self.modules[module]
                        .module
                        .constants
                        .contains_key(name.as_str())
                {
                    return Err(self.located(
                        module,
                        position,
                        format!("cannot assign to the constant {name}"),
                    ));
                }
            }
            let expr = match instruction {
                Instruction::Let(_, _, _, expr)
                | Instruction::Unpack(_, _, expr)
                | Instruction::Assign(_, expr)
                | Instruction::AssignField(_, _, expr)
                | Instruction::Print(expr)
                | Instruction::Call(expr)
                | Instruction::Fail(expr) => Some(expr),
                Instruction::Return(expr) => expr.as_mut(),
                Instruction::AssignIndex(_, index, expr)
                | Instruction::AssignIndexField(_, index, _, expr) => {
                    self.resolve_expr(module, locals, index)
                        .map_err(|error| self.located(module, position, error))?;
                    Some(expr)
                }
                Instruction::If(condition, yes, no) => {
                    self.resolve_body(module, locals, yes)?;
                    self.resolve_body(module, locals, no)?;
                    Some(condition)
                }
                Instruction::While(condition, nested) => {
                    self.resolve_body(module, locals, nested)?;
                    Some(condition)
                }
                Instruction::For(_, start, end, nested) => {
                    self.resolve_expr(module, locals, start)
                        .map_err(|error| self.located(module, position, error))?;
                    self.resolve_body(module, locals, nested)?;
                    Some(end)
                }
                Instruction::Each(_, _, _, _, nested) => {
                    self.resolve_body(module, locals, nested)?;
                    None
                }
                Instruction::Match(scrutinee, arms) => {
                    for (_, body) in arms {
                        self.resolve_body(module, locals, body)?;
                    }
                    Some(scrutinee)
                }
                Instruction::Break | Instruction::Continue => None,
            };
            if let Some(expr) = expr {
                self.resolve_expr(module, locals, expr)
                    .map_err(|error| self.located(module, position, error))?;
            }
        }
        Ok(())
    }

    /// 常量的用法换成它的字面量：本模块的 `LIMIT`、导入模块的 `config.LIMIT`，以及字符串常量的
    /// 借用 `&NAME`（`&string` 参数本来就接受字符串字面量）。不是常量时返回 None。
    fn constant_use(
        &self,
        module: &str,
        locals: &BTreeSet<String>,
        expr: &Expr,
    ) -> Result<Option<Expr>> {
        let (path, borrowed) = match expr {
            Expr::Variable(name) => (name.clone(), false),
            Expr::Borrow(name, false) => (name.clone(), true),
            Expr::Field(root, field) => (format!("{root}.{field}"), false),
            _ => return Ok(None),
        };
        let (target, name) = match path.split_once('.') {
            None => (module.to_owned(), path.as_str()),
            Some((alias, name)) if !name.contains('.') && !locals.contains(alias) => {
                let imports: Vec<_> = self.modules[module]
                    .module
                    .imports
                    .iter()
                    .filter(|p| p.rsplit('/').next() == Some(alias))
                    .collect();
                match imports.as_slice() {
                    [target] if self.modules.contains_key(target.as_str()) => {
                        ((*target).clone(), name)
                    }
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        if target == module && locals.contains(name) {
            return Ok(None);
        }
        let Some(constant) = self.modules[&target].module.constants.get(name) else {
            return Ok(None);
        };
        if target != module && !constant.public {
            return Err(format!(
                "private constant {path}; declare it with pub const"
            ));
        }
        if borrowed && !matches!(constant.value, Expr::Text(_)) {
            return Err(format!(
                "cannot borrow the constant {path}; only string constants can be passed as &string"
            ));
        }
        Ok(Some(constant.value.clone()))
    }

    fn resolve_expr(&self, module: &str, locals: &BTreeSet<String>, expr: &mut Expr) -> Result<()> {
        if let Some(value) = self.constant_use(module, locals, expr)? {
            *expr = value;
            return Ok(());
        }
        match expr {
            // `f(x)`：f 是局部的函数值。
            Expr::Call(name, args) if !name.contains('.') && locals.contains(name.as_str()) => {
                let mut args = std::mem::take(args);
                for arg in &mut args {
                    self.resolve_expr(module, locals, arg)?;
                }
                *expr = Expr::Apply(name.clone(), args);
            }
            // 不是局部名、也不是常量的 `double`：本模块的函数当值用。
            Expr::Variable(name)
                if !locals.contains(name.as_str())
                    && self.modules[module]
                        .module
                        .functions
                        .contains_key(name.as_str()) =>
            {
                *expr = Expr::FnRef(self.resolve(module, name)?);
            }
            // `util.double`：导入模块的函数当值用。
            Expr::Field(root, field)
                if !locals.contains(root.as_str())
                    && !field.contains('.')
                    && field.starts_with(|c: char| c.is_lowercase())
                    && self.modules[module]
                        .module
                        .imports
                        .iter()
                        .any(|path| path.rsplit('/').next() == Some(root.as_str()))
                    && self.resolve(module, &format!("{root}.{field}")).is_ok() =>
            {
                *expr = Expr::FnRef(self.resolve(module, &format!("{root}.{field}"))?);
            }
            Expr::Lambda(params, result, body) => {
                for (_, ty) in params.iter_mut() {
                    if let Some(ty) = ty {
                        self.resolve_type(module, ty)?;
                    }
                }
                if let Some(result) = result {
                    self.resolve_type(module, result)?;
                }
                self.resolve_body(module, locals, body)?;
            }
            Expr::Apply(_, args) => {
                for arg in args {
                    self.resolve_expr(module, locals, arg)?;
                }
            }
            Expr::Invoke(callee, args) => {
                self.resolve_expr(module, locals, callee)?;
                for arg in args {
                    self.resolve_expr(module, locals, arg)?;
                }
            }
            Expr::Record(name, fields) => {
                let (target, short) = if let Some((alias, short)) = name.split_once('.') {
                    let imports: Vec<_> = self.modules[module]
                        .module
                        .imports
                        .iter()
                        .filter(|p| p.rsplit('/').next() == Some(alias))
                        .collect();
                    if imports.len() != 1 {
                        if self.modules[module]
                            .module
                            .structs
                            .get(alias)
                            .is_some_and(|s| !s.variants.is_empty())
                        {
                            return Err(format!(
                                "type mismatch: {alias}.{short} is an enum variant; pass its values by position, such as {alias}.{short}(1.0)"
                            ));
                        }
                        return Err("missing or ambiguous struct import".into());
                    }
                    (imports[0].as_str(), short)
                } else {
                    // 模块自己没有声明 `Option` 时用内置的。
                    if name == "Option"
                        && !self.modules[module].module.structs.contains_key("Option")
                    {
                        *name = compiler::OPTION.into();
                        for (_, value) in fields {
                            self.resolve_expr(module, locals, value)?;
                        }
                        return Ok(());
                    }
                    (module, name.as_str())
                };
                let declaration = self
                    .modules
                    .get(target)
                    .and_then(|m| m.module.structs.get(short))
                    .ok_or("unknown struct")?;
                if target != module && !declaration.public {
                    return Err("private struct".into());
                }
                *name = format!("{target}:{short}");
                for (_, value) in fields {
                    self.resolve_expr(module, locals, value)?;
                }
            }
            // `math.pi` 这类常量：别名指向 std/math 且不是局部变量时，直接换成字面量。
            Expr::Field(root, name)
                if !locals.contains(root.as_str())
                    && self.modules[module]
                        .module
                        .imports
                        .iter()
                        .any(|path| path == "std/math" && root == "math") =>
            {
                *expr = Expr::Float(match name.as_str() {
                    "pi" => std::f64::consts::PI,
                    "e" => std::f64::consts::E,
                    "inf" => f64::INFINITY,
                    _ => {
                        return Err(format!(
                            "unknown variable math.{name}; std/math has the constants pi, e and inf"
                        ))
                    }
                });
            }
            // `Shape.Empty`、`model.Shape.Empty`：没有载荷的枚举变体，写法和字段访问一样。
            Expr::Field(root, field)
                if !locals.contains(root.as_str())
                    && field
                        .rsplit('.')
                        .next()
                        .is_some_and(|v| v.starts_with(char::is_uppercase)) =>
            {
                let full = format!("{root}.{field}");
                let (path, variant) = full.rsplit_once('.').expect("field path");
                let mut constructor = Expr::Record(path.to_owned(), Vec::new());
                if self.resolve_expr(module, locals, &mut constructor).is_ok() {
                    if let Expr::Record(qualified, _) = constructor {
                        *expr = Expr::Variant(qualified, variant.to_owned(), Vec::new());
                    }
                }
            }
            // `f64(n)`、`i64(x)`：数值转换，不需要导入。
            Expr::Call(name, args) if matches!(name.as_str(), "f64" | "i64") => {
                *name = format!("std/core:{name}");
                for arg in args {
                    self.resolve_expr(module, locals, arg)?;
                }
            }
            // `assert` 展开出的报错调用：报错前面补上模块文件路径。
            Expr::Call(name, args) if name == "$assert" => {
                *name = "std/core:assert_failed".into();
                if let Some(Expr::Format(parts)) = args.first_mut() {
                    let path = self.modules[module].path.display().to_string();
                    parts.insert(0, Expr::Text(path));
                }
                for arg in args {
                    self.resolve_expr(module, locals, arg)?;
                }
            }
            // Python 的内置函数：`len(x)` 是 `x.len()`，`str(x)` 是 `"{x}"`，`sum(xs)`、`sorted(xs)`、
            // 只给一个列表的 `min(xs)`/`max(xs)` 是列表方法。本模块自己定义了同名函数时用自己的。
            Expr::Call(name, args)
                if args.len() == 1
                    && matches!(
                        name.as_str(),
                        "len" | "str" | "sum" | "sorted" | "min" | "max"
                    )
                    && self.resolve(module, name).is_err() =>
            {
                let method = name.clone();
                let mut arg = args.remove(0);
                self.resolve_expr(module, locals, &mut arg)?;
                *expr = match (method.as_str(), arg) {
                    ("str", arg) => Expr::Format(vec![arg]),
                    (_, Expr::Variable(local)) if locals.contains(&local) => {
                        Expr::Method(local, method, Vec::new())
                    }
                    (_, Expr::Field(local, field)) if locals.contains(&local) => {
                        Expr::Method(format!("{local}.{field}"), method, Vec::new())
                    }
                    (_, arg) => Expr::Chain(Box::new(arg), method, Vec::new()),
                };
            }
            // `abs`/`min`/`max`/`copy`/`int`/`float`：全局内建函数；本模块自己定义了同名函数时用自己的。
            Expr::Call(name, args)
                if matches!(
                    name.as_str(),
                    "abs" | "min" | "max" | "copy" | "int" | "float" | "$pow"
                ) && self.resolve(module, name).is_err() =>
            {
                *name = format!("std/core:{}", name.trim_start_matches('$'));
                for arg in args.iter_mut() {
                    self.resolve_expr(module, locals, arg)?;
                    // `copy(p)` 与 `copy(&p)` 相同：局部变量、字段与元素按只读借用传。
                    if name.as_str() == crate::compiler::COPY {
                        let local = match arg {
                            Expr::Variable(root)
                            | Expr::Field(root, _)
                            | Expr::Index(root, _)
                            | Expr::IndexField(root, _, _) => {
                                locals.contains(root.split('.').next().unwrap_or(root))
                            }
                            _ => false,
                        };
                        if let Some(borrow) = crate::compiler::auto_borrow(arg).filter(|_| local) {
                            *arg = borrow;
                        }
                    }
                }
            }
            Expr::Call(name, args) => {
                if matches!(
                    name.as_str(),
                    "string.byte_len" | "string.char_len" | "string.equal"
                ) && self.modules[module]
                    .module
                    .imports
                    .iter()
                    .any(|p| p == "std/string")
                {
                    if self.modules[module]
                        .module
                        .imports
                        .iter()
                        .filter(|p| p.rsplit('/').next() == Some("string"))
                        .count()
                        != 1
                    {
                        return Err("ambiguous string import".into());
                    }
                    let equal = name == "string.equal";
                    let count = if equal { 2 } else { 1 };
                    if args.len() != count {
                        return Err(format!(
                            "{name} requires {count} borrowed string argument(s)"
                        ));
                    }
                    for arg in args.iter_mut() {
                        self.resolve_expr(module, locals, arg)?;
                    }
                    let left = Box::new(args.remove(0));
                    *expr = if equal {
                        Expr::StringEqual(left, Box::new(args.remove(0)))
                    } else if name == "string.byte_len" {
                        Expr::ByteLen(left)
                    } else {
                        Expr::CharLen(left)
                    };
                    return Ok(());
                }
                // `a.b(...)`：`a` 是本函数声明过的局部名（且不是导入别名）就是方法调用，交给检查器
                // 按接收者类型处理；否则是模块调用，拼错的模块名照旧报缺少导入。
                if let Some((receiver, method)) = name.rsplit_once('.') {
                    let root = receiver.split('.').next().unwrap_or(receiver);
                    let imported = self.modules[module]
                        .module
                        .imports
                        .iter()
                        .any(|path| path.rsplit('/').next() == Some(root));
                    // 局部变量与没导入的标准库同名（`let fs = ...`）时是方法调用；导入了同名模块才算模块调用。
                    if locals.contains(root) && (receiver.contains('.') || !imported) {
                        let mut args = std::mem::take(args);
                        for arg in &mut args {
                            self.resolve_expr(module, locals, arg)?;
                        }
                        *expr = Expr::Method(receiver.to_owned(), method.to_owned(), args);
                        return Ok(());
                    }
                }
                *name = self.resolve(module, name)?;
                for arg in args {
                    self.resolve_expr(module, locals, arg)?;
                }
            }
            Expr::Method(_, _, args) => {
                for arg in args {
                    self.resolve_expr(module, locals, arg)?;
                }
            }
            Expr::ElementMethod(_, index, _, _, args) | Expr::Chain(index, _, args) => {
                self.resolve_expr(module, locals, index)?;
                for arg in args {
                    self.resolve_expr(module, locals, arg)?;
                }
            }
            Expr::BorrowIndex(_, index, _, _) | Expr::IndexField(_, index, _) => {
                self.resolve_expr(module, locals, index)?
            }
            Expr::MapLit(entries) => {
                for (key, value) in entries {
                    self.resolve_expr(module, locals, key)?;
                    self.resolve_expr(module, locals, value)?;
                }
            }
            Expr::Format(segments) => {
                for segment in segments {
                    self.resolve_expr(module, locals, segment)?;
                }
            }
            Expr::If(condition, yes, no) => {
                self.resolve_expr(module, locals, condition)?;
                self.resolve_expr(module, locals, yes)?;
                self.resolve_expr(module, locals, no)?;
            }
            // 枚举名按结构体名解析成限定名。
            Expr::Variant(name, variant, args) => {
                let mut constructor = Expr::Record(name.clone(), Vec::new());
                match self.resolve_expr(module, locals, &mut constructor) {
                    Ok(()) => {
                        if let Expr::Record(qualified, _) = constructor {
                            *name = qualified;
                        }
                    }
                    // `model.Empty()`：没有字段的跨模块结构体，写法和无载荷的变体一样。
                    Err(error) => {
                        let mut record = Expr::Record(format!("{name}.{variant}"), Vec::new());
                        if !args.is_empty()
                            || self.resolve_expr(module, locals, &mut record).is_err()
                        {
                            return Err(error);
                        }
                        *expr = record;
                        return Ok(());
                    }
                }
                for arg in args {
                    self.resolve_expr(module, locals, arg)?;
                }
            }
            Expr::Match(scrutinee, arms) => {
                self.resolve_expr(module, locals, scrutinee)?;
                for (_, value) in arms {
                    self.resolve_expr(module, locals, value)?;
                }
            }
            Expr::Unary(_, value)
            | Expr::ByteLen(value)
            | Expr::CharLen(value)
            | Expr::Try(value) => self.resolve_expr(module, locals, value)?,
            Expr::Catch(inner, _, block) => {
                self.resolve_expr(module, locals, inner)?;
                self.resolve_body(module, locals, block)?;
            }
            Expr::Array(values) | Expr::Tuple(values) => {
                for value in values {
                    self.resolve_expr(module, locals, value)?;
                }
            }
            Expr::Index(_, index) => self.resolve_expr(module, locals, index)?,
            Expr::Binary(_, left, right) | Expr::StringEqual(left, right) => {
                self.resolve_expr(module, locals, left)?;
                self.resolve_expr(module, locals, right)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn key(
        &self,
        id: &str,
        keys: &mut BTreeMap<String, String>,
        visiting: &mut BTreeSet<String>,
    ) -> Result<String> {
        if let Some(key) = keys.get(id) {
            return Ok(key.clone());
        }
        if !visiting.insert(id.into()) {
            return Err(format!("cyclic module dependency at {id}"));
        }
        let input = &self.modules[id];
        let mut parts = vec![
            VERSION.as_bytes().to_vec(),
            env::consts::OS.as_bytes().to_vec(),
            env::consts::ARCH.as_bytes().to_vec(),
            input.source.as_bytes().to_vec(),
        ];
        for dependency in &input.module.imports {
            parts.push(dependency.as_bytes().to_vec());
            if !dependency.starts_with("std/") {
                parts.push(self.key(dependency, keys, visiting)?.into_bytes());
            }
        }
        let key = hash(&parts.iter().map(Vec::as_slice).collect::<Vec<_>>());
        visiting.remove(id);
        keys.insert(id.into(), key.clone());
        Ok(key)
    }

    pub fn build(&self, cache: &Path) -> Result<PathBuf> {
        self.build_with(cache, &[])
    }

    /// `zam test`：编出测试程序，每个测试单独起一个进程运行（一个测试崩溃不影响其他），
    /// 打印每个测试的结果与失败输出。返回失败的个数。
    pub fn test(&self, cache: &Path, filter: Option<&str>) -> Result<usize> {
        let tests = self.tests()?;
        let label = |test: &str| {
            let (module, name) = test.split_once(':').expect("qualified test name");
            if module == self.entry {
                name.to_owned()
            } else {
                format!("{}.{name}", module.replace('/', "."))
            }
        };
        let selected: Vec<&String> = tests
            .iter()
            .filter(|test| filter.is_none_or(|filter| label(test).contains(filter)))
            .collect();
        if selected.is_empty() {
            println!("no tests found; name test functions test_..., such as fn test_add():");
            return Ok(0);
        }
        let artifact = self.build_with(cache, &tests)?;
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results = std::sync::Mutex::new(vec![None; selected.len()]);
        std::thread::scope(|scope| {
            for _ in 0..jobs().min(selected.len()) {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(test) = selected.get(index) else {
                        break;
                    };
                    let result = run_test(&artifact, test);
                    results.lock().expect("test results")[index] = Some(result);
                });
            }
        });
        let results = results.into_inner().expect("test results");
        let mut failures = Vec::new();
        for (test, result) in selected.iter().zip(results) {
            let (passed, output) = result.expect("every test ran");
            println!(
                "test {} ... {}",
                label(test),
                if passed { "ok" } else { "FAILED" }
            );
            if !passed {
                failures.push((label(test), output));
            }
        }
        for (name, output) in &failures {
            println!("\n---- {name} ----\n{}", output.trim_end());
        }
        println!(
            "\ntest result: {}. {} passed; {} failed",
            if failures.is_empty() { "ok" } else { "FAILED" },
            selected.len() - failures.len(),
            failures.len()
        );
        Ok(failures.len())
    }

    fn build_with(&self, cache: &Path, tests: &[String]) -> Result<PathBuf> {
        if self.library && tests.is_empty() {
            return Err(format!(
                "{} is a library: zam.toml has no entry, so there is no program to build or run; use zam check or zam test, or add entry = \"src/main.zm\" under [project]",
                self.name
            ));
        }
        let build_started = Instant::now();
        let check_started = Instant::now();
        self.check_with(!tests.is_empty())?;
        let check_ms = check_started.elapsed().as_secs_f64() * 1000.0;
        let key_started = Instant::now();
        let mut keys = BTreeMap::new();
        for id in self.modules.keys() {
            self.key(id, &mut keys, &mut BTreeSet::new())?;
        }
        let key_ms = key_started.elapsed().as_secs_f64() * 1000.0;
        let linked = self.linked()?;
        for (id, input) in &self.modules {
            let bytes = compiler::encode(&input.module);
            let path = cache.join(format!("{}.zmo", keys[id]));
            let hit = match fs::read(&path) {
                Ok(existing) => {
                    if existing != bytes {
                        return Err(format!("corrupt cache entry {}", path.display()));
                    }
                    true
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    atomic_write(&path, &bytes)?;
                    false
                }
                Err(e) => return Err(e.to_string()),
            };
            // ponytail: 进度走 stderr，stdout 留给程序输出与 --json 的机器可读结果。
            eprintln!(
                "CACHE {} {:.12} {id}",
                if hit { "HIT" } else { "MISS" },
                keys[id]
            );
        }
        let object_prepare_started = Instant::now();
        let (tool, identity) = native::tool()?;
        let entry = format!("{}:main", self.entry);
        // ponytail: 两段式。先把所有模块并行「生成 C + 查缓存」（每模块 3–8ms，忽略不计），
        // 再把 MISS 交给有界工作队列编译。已命中的模块不占编译位，空闲线程立即
        // 领取下一个 MISS，不再等待整批中最慢的模块。
        // 每个 MISS 仍然在自己的 `cl` 里编 `program.c`，对象字节不变。
        // 并发度按内存封顶（每个编译器约 30–60 MB，DESIGN §8 要求构建峰值 < 200 MB），
        // 需要调就设 ZAMAK_JOBS。
        let jobs = jobs();
        let mut slots: Vec<Option<(String, Vec<u8>)>> = Vec::with_capacity(self.modules.len());
        let mut misses: Vec<(usize, String, String)> = Vec::new();
        let mut object_ms = 0.0;
        for chunk in self.modules.keys().collect::<Vec<_>>().chunks(jobs) {
            let object_started = Instant::now();
            let results = std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|id| {
                        let (linked, entry, identity) = (&linked, &entry, &identity);
                        scope.spawn(move || -> Result<(String, String, Option<Vec<u8>>)> {
                            let source = native::generate_with(linked, entry, id, tests);
                            // 调试生成的 C：`ZAMAK_DUMP_C=<目录>` 时把每个模块的 C 源码写出来。
                            if let Some(dir) = env::var_os("ZAMAK_DUMP_C") {
                                let name = id.replace(['/', ':'], "_");
                                let _ = fs::create_dir_all(&dir);
                                let _ =
                                    fs::write(Path::new(&dir).join(format!("{name}.c")), &source);
                            }
                            let key = hash(&[
                                VERSION.as_bytes(),
                                source.as_bytes(),
                                identity.as_bytes(),
                                native::OPTIONS.as_bytes(),
                                env::consts::OS.as_bytes(),
                                env::consts::ARCH.as_bytes(),
                            ]);
                            let bytes = cached(cache, &key, "obj")?;
                            eprintln!(
                                "OBJECT {} {:.12} {id}",
                                if bytes.is_some() { "HIT" } else { "MISS" },
                                key
                            );
                            Ok((key, source, bytes))
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err("worker panicked".into()))
                    })
                    .collect::<Vec<Result<(String, String, Option<Vec<u8>>)>>>()
            });
            object_ms += object_started.elapsed().as_secs_f64() * 1000.0;
            for result in results {
                let (key, source, bytes) = result?;
                let slot = slots.len();
                match bytes {
                    Some(bytes) => slots.push(Some((key, bytes))),
                    None => {
                        slots.push(None);
                        misses.push((slot, key, source));
                    }
                }
            }
        }
        // 完成一个模块就领取下一个，避免每批等待最慢的编译进程。
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..jobs.min(misses.len()))
                .map(|_| {
                    let (misses, next, tool) = (&misses, &next, &tool);
                    scope.spawn(move || -> Result<Vec<(usize, String, Vec<u8>)>> {
                        let mut completed = Vec::new();
                        loop {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some((slot, key, source)) = misses.get(index) else {
                                break;
                            };
                            let (bytes, _) = native_cached(cache, key, "obj", |scratch| {
                                native::compile(tool, scratch, source)
                            })?;
                            completed.push((*slot, key.clone(), bytes));
                        }
                        Ok(completed)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|_| Err("worker panicked".into()))
                })
                .collect::<Vec<_>>()
        });
        for result in results {
            for (slot, key, bytes) in result? {
                slots[slot] = Some((key, bytes));
            }
        }
        let object_prepare_ms = object_prepare_started.elapsed().as_secs_f64() * 1000.0;
        let mut objects = Vec::with_capacity(slots.len());
        let mut object_keys = Vec::with_capacity(slots.len());
        for slot in slots {
            let (key, bytes) = slot.ok_or("native cache entry was not produced")?;
            object_keys.push(key);
            objects.push(bytes);
        }
        // `[native]` 的 C 源码：各自编成目标文件，按源码与头文件的内容缓存。
        let mut libraries = Vec::new();
        let mut library_paths = Vec::new();
        for (root, settings) in &self.native {
            // 编译在缓存的临时目录里进行，路径都换成绝对路径。
            let absolute = |path: PathBuf| std::path::absolute(&path).unwrap_or(path);
            let includes: Vec<PathBuf> = settings
                .include
                .iter()
                .map(|dir| absolute(root.join(dir)))
                .collect();
            let headers = header_digest(&includes)?;
            for source in &settings.sources {
                let path = root.join(source);
                let text = fs::read(&path)
                    .map_err(|e| format!("native source {}: {e}", path.display()))?;
                let mut dirs = vec![absolute(path.parent().unwrap_or(root).to_owned())];
                dirs.extend(includes.iter().cloned());
                let key = hash(&[
                    VERSION.as_bytes(),
                    b"native source",
                    &text,
                    headers.as_bytes(),
                    identity.as_bytes(),
                    native::OPTIONS.as_bytes(),
                    env::consts::OS.as_bytes(),
                    env::consts::ARCH.as_bytes(),
                ]);
                let (bytes, hit) = native_cached(cache, &key, "obj", |scratch| {
                    native::compile_with(&tool, scratch, &String::from_utf8_lossy(&text), &dirs)
                })?;
                eprintln!(
                    "OBJECT {} {:.12} {}",
                    if hit { "HIT" } else { "MISS" },
                    key,
                    source.display()
                );
                object_keys.push(key);
                objects.push(bytes);
            }
            libraries.extend(settings.libraries.iter().cloned());
            library_paths.extend(
                settings
                    .library_paths
                    .iter()
                    .map(|dir| absolute(root.join(dir))),
            );
        }
        let link_settings = format!("{libraries:?}\n{library_paths:?}");
        let app_key = hash(&[
            VERSION.as_bytes(),
            object_keys.join("\n").as_bytes(),
            identity.as_bytes(),
            native::OPTIONS.as_bytes(),
            link_settings.as_bytes(),
        ]);
        let link_started = Instant::now();
        let (bytes, hit) = native_cached(cache, &app_key, "exe", |scratch| {
            native::link(&tool, scratch, &objects, &libraries, &library_paths)
        })?;
        let link_ms = link_started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("LINK {} {:.12}", if hit { "HIT" } else { "MISS" }, app_key);
        eprintln!(
            "TIMING check_ms={check_ms:.3} key_ms={key_ms:.3} object_ms={object_ms:.3} object_prepare_ms={object_prepare_ms:.3} link_ms={link_ms:.3} total_ms={:.3}",
            build_started.elapsed().as_secs_f64() * 1000.0
        );
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        let artifact = self.root.join("build").join(format!(
            "{}-{}{app_key}{suffix}",
            self.name,
            if tests.is_empty() { "" } else { "test-" }
        ));
        atomic_write(&artifact, &bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        eprintln!("OUTPUT {}", artifact.display());
        Ok(artifact)
    }
}

/// 运行一个测试：（是否通过, 合并后的 stdout 与 stderr）。超过 60 秒算失败并结束进程。
fn run_test(artifact: &Path, test: &str) -> (bool, String) {
    use std::io::Read;
    let child = std::process::Command::new(artifact)
        .arg(test)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return (false, format!("cannot start the test program: {error}")),
    };
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stdout.read_to_end(&mut text);
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.read_to_end(&mut text);
        text
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed().as_secs() >= 60 => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
            Err(_) => break None,
        }
    };
    let mut output = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    output.push_str(&String::from_utf8_lossy(&err.join().unwrap_or_default()));
    match status {
        Some(status) if status.success() => (true, output),
        Some(status) => {
            output.push_str(&format!("(exit status {})", status.code().unwrap_or(-1)));
            (false, output)
        }
        None => {
            output.push_str("(timed out after 60 seconds)");
            (false, output)
        }
    }
}

/// 头文件目录里所有 `.h` 文件（递归）的内容摘要：头文件改了，用到它的 C 源码要重新编译。
fn header_digest(dirs: &[PathBuf]) -> Result<String> {
    fn walk(dir: &Path, base: &Path, parts: &mut Vec<Vec<u8>>) -> Result<()> {
        let Ok(items) = fs::read_dir(dir) else {
            return Ok(());
        };
        let mut items: Vec<_> = items.filter_map(|item| item.ok()).collect();
        items.sort_by_key(|item| item.file_name());
        for item in items {
            let path = item.path();
            if path.is_dir() {
                walk(&path, base, parts)?;
            } else if path.extension().is_some_and(|ext| ext == "h") {
                let name = path.strip_prefix(base).unwrap_or(&path);
                parts.push(name.to_string_lossy().replace('\\', "/").into_bytes());
                parts.push(fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?);
            }
        }
        Ok(())
    }
    let mut parts = Vec::new();
    for dir in dirs {
        walk(dir, dir, &mut parts)?;
    }
    Ok(hash(&parts.iter().map(Vec::as_slice).collect::<Vec<_>>()))
}

/// 并行编译的并发度：默认取可用核数，内存封顶 8；`ZAMAK_JOBS=1` 可关掉并行。
fn jobs() -> usize {
    env::var("ZAMAK_JOBS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(1, std::num::NonZeroUsize::get)
                .min(8)
        })
}

/// 只查缓存，不编译：命中要校验 sha256，字节对不上就是坏条目，缺失则返回 `None`。
fn cached(cache: &Path, key: &str, extension: &str) -> Result<Option<Vec<u8>>> {
    let path = cache.join(format!("{key}.{extension}"));
    match fs::read(&path) {
        Ok(bytes) => {
            let checksum = cache.join(format!("{key}.{extension}.sha256"));
            let expected = fs::read_to_string(&checksum)
                .map_err(|e| format!("missing native cache checksum: {e}"))?;
            if hash(&[&bytes]) != expected {
                return Err(format!("corrupt cache entry {}", path.display()));
            }
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn native_cached(
    cache: &Path,
    key: &str,
    extension: &str,
    build: impl FnOnce(&Path) -> Result<Vec<u8>>,
) -> Result<(Vec<u8>, bool)> {
    if let Some(bytes) = cached(cache, key, extension)? {
        return Ok((bytes, true));
    }
    let checksum = cache.join(format!("{key}.{extension}.sha256"));
    let path = cache.join(format!("{key}.{extension}"));
    let scratch = cache.join(format!(
        ".native-{}-{}",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let result = build(&scratch);
    let cleanup = fs::remove_dir_all(&scratch);
    let bytes = result?;
    cleanup.map_err(|e| e.to_string())?;
    atomic_write(&checksum, hash(&[&bytes]).as_bytes())?;
    atomic_write(&path, &bytes)?;
    Ok((bytes, false))
}

/// 函数里声明过的所有局部名（`let`、循环变量、`catch` 绑定），不论作用域；方法调用的
/// 接收者只能是其中之一，作用域与有效性由检查器负责。
fn declared(body: &Block, names: &mut BTreeSet<String>) {
    fn expr(value: &Expr, names: &mut BTreeSet<String>) {
        match value {
            Expr::Lambda(params, _, body) => {
                names.extend(params.iter().map(|(name, _)| name.clone()));
                declared(body, names);
            }
            Expr::Apply(_, values) => values.iter().for_each(|value| expr(value, names)),
            Expr::Invoke(callee, values) => {
                expr(callee, names);
                values.iter().for_each(|value| expr(value, names));
            }
            Expr::Catch(inner, binding, block) => {
                names.insert(binding.clone());
                declared(block, names);
                expr(inner, names);
            }
            Expr::Call(_, values)
            | Expr::Method(_, _, values)
            | Expr::Array(values)
            | Expr::Tuple(values)
            | Expr::Format(values) => values.iter().for_each(|value| expr(value, names)),
            Expr::Record(_, fields) => fields.iter().for_each(|(_, value)| expr(value, names)),
            Expr::ElementMethod(_, index, _, _, values) | Expr::Chain(index, _, values) => {
                expr(index, names);
                values.iter().for_each(|value| expr(value, names));
            }
            Expr::MapLit(entries) => entries.iter().for_each(|(key, value)| {
                expr(key, names);
                expr(value, names);
            }),
            Expr::If(condition, yes, no) => {
                expr(condition, names);
                expr(yes, names);
                expr(no, names);
            }
            Expr::Variant(_, _, values) => values.iter().for_each(|value| expr(value, names)),
            Expr::Match(scrutinee, arms) => {
                expr(scrutinee, names);
                for ((_, bindings), value) in arms {
                    names.extend(bindings.iter().cloned());
                    expr(value, names);
                }
            }
            Expr::Unary(_, inner)
            | Expr::Try(inner)
            | Expr::ByteLen(inner)
            | Expr::CharLen(inner)
            | Expr::Index(_, inner)
            | Expr::IndexField(_, inner, _)
            | Expr::BorrowIndex(_, inner, _, _) => expr(inner, names),
            Expr::Binary(_, left, right) | Expr::StringEqual(left, right) => {
                expr(left, names);
                expr(right, names);
            }
            _ => {}
        }
    }
    for (instruction, _) in body {
        match instruction {
            Instruction::Let(name, _, _, value) => {
                names.insert(name.clone());
                expr(value, names);
            }
            Instruction::Unpack(bound, _, value) => {
                names.extend(bound.iter().cloned());
                expr(value, names);
            }
            Instruction::Assign(_, value)
            | Instruction::AssignField(_, _, value)
            | Instruction::Print(value)
            | Instruction::Call(value)
            | Instruction::Fail(value)
            | Instruction::Return(Some(value)) => expr(value, names),
            Instruction::AssignIndex(_, index, value)
            | Instruction::AssignIndexField(_, index, _, value) => {
                expr(index, names);
                expr(value, names);
            }
            Instruction::If(condition, yes, no) => {
                expr(condition, names);
                declared(yes, names);
                declared(no, names);
            }
            Instruction::While(condition, nested) => {
                expr(condition, names);
                declared(nested, names);
            }
            Instruction::For(name, start, end, nested) => {
                names.insert(name.clone());
                expr(start, names);
                expr(end, names);
                declared(nested, names);
            }
            Instruction::Each(name, second, _, _, nested) => {
                names.insert(name.clone());
                names.extend(second.iter().cloned());
                declared(nested, names);
            }
            Instruction::Match(scrutinee, arms) => {
                expr(scrutinee, names);
                for ((_, bindings), body) in arms {
                    names.extend(bindings.iter().cloned());
                    declared(body, names);
                }
            }
            Instruction::Return(None) | Instruction::Break | Instruction::Continue => {}
        }
    }
}

pub fn run_artifact(path: &Path, args: &[std::ffi::OsString]) -> Result<()> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let status = std::process::Command::new(path)
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        return Ok(());
    }
    // 与 `cargo run`、`python prog.py` 一样，原样带回程序自己的退出码，不再加一行提示。
    match status.code() {
        Some(code) => std::process::exit(code),
        None => Err(format!("program failed: {status}")),
    }
}
