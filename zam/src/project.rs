use crate::compiler::{self, Block, Expr, Instruction, Module};
use crate::native;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub type Result<T> = std::result::Result<T, String>;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);
const VERSION: &str = concat!("zam-", env!("CARGO_PKG_VERSION"), "-native-c-18");

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
}

pub struct Project {
    pub name: String,
    pub root: PathBuf,
    entry: String,
    modules: BTreeMap<String, Input>,
}

fn relative(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("project paths must be nonempty relative paths without '..'".into());
    }
    Ok(path)
}

fn is_source(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "zm")
}

fn scan(dir: &Path, base: &Path, modules: &mut BTreeMap<String, Input>) -> Result<()> {
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
            scan(&path, base, modules)?;
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
            let source =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let module =
                compiler::parse(&source).map_err(|e| format!("{}: {e}", path.display()))?;
            if modules
                .insert(id.clone(), Input { source, module })
                .is_some()
            {
                return Err(format!("duplicate module {id}"));
            }
        }
    }
    Ok(())
}

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Project {
    pub fn load(input: &Path) -> Result<Self> {
        if is_source(input) {
            let source = fs::read_to_string(input).map_err(|e| e.to_string())?;
            let module =
                compiler::parse(&source).map_err(|e| format!("{}: {e}", input.display()))?;
            return Ok(Self {
                name: "main".into(),
                root: input.parent().unwrap_or(Path::new(".")).into(),
                entry: "main".into(),
                modules: BTreeMap::from([("main".into(), Input { source, module })]),
            });
        }
        let manifest = if input.is_dir() {
            input.join("zam.toml")
        } else {
            input.into()
        };
        let text =
            fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let config: toml::Table = text.parse().map_err(|e| format!("invalid zam.toml: {e}"))?;
        if config.len() != 1 {
            return Err("zam.toml accepts only [project]".into());
        }
        let table = config
            .get("project")
            .and_then(toml::Value::as_table)
            .ok_or("missing [project]")?;
        if table
            .keys()
            .any(|key| !matches!(key.as_str(), "name" | "entry" | "modules"))
        {
            return Err("unknown project setting".into());
        }
        let setting = |key: &str| -> Result<&str> {
            table
                .get(key)
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("missing string project.{key}"))
        };
        let name = setting("name")?.to_owned();
        if !identifier(&name) {
            return Err("project.name must be an identifier".into());
        }
        let entry_path = relative(setting("entry")?)?;
        let module_path = relative(setting("modules")?)?;
        let entry = entry_path
            .strip_prefix(&module_path)
            .map_err(|_| "entry must be inside modules")?;
        if !is_source(entry) {
            return Err("entry must be a .zm file".into());
        }
        let entry = entry
            .with_extension("")
            .to_str()
            .ok_or("entry must be UTF-8")?
            .replace('\\', "/");
        let root = manifest.parent().unwrap_or(Path::new(".")).to_owned();
        let base = root.join(module_path);
        let mut modules = BTreeMap::new();
        scan(&base, &base, &mut modules)?;
        if !modules.contains_key(&entry) {
            return Err(format!("missing entry module {entry}"));
        }
        Ok(Self {
            name,
            root,
            entry,
            modules,
        })
    }

    fn resolve(&self, module: &str, call: &str) -> Result<String> {
        let (target, name) = if let Some((alias, name)) = call.split_once('.') {
            let matches: Vec<_> = self.modules[module]
                .module
                .imports
                .iter()
                .filter(|path| path.rsplit('/').next() == Some(alias))
                .collect();
            if matches.len() != 1 {
                return Err(format!("{module}: ambiguous or missing import for {call}"));
            }
            (matches[0].as_str(), name)
        } else {
            (module, call)
        };
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

    pub fn check(&self) -> Result<()> {
        self.modules[&self.entry]
            .module
            .functions
            .get("main")
            .ok_or("entry module requires fn main()")?;
        for (id, input) in &self.modules {
            for import in &input.module.imports {
                if !matches!(import.as_str(), "std/io" | "std/fmt" | "std/string")
                    && !self.modules.contains_key(import)
                {
                    return Err(format!("{id}: missing module {import}"));
                }
            }
        }
        let mut keys = BTreeMap::new();
        for id in self.modules.keys() {
            self.key(id, &mut keys, &mut BTreeSet::new())?;
        }
        compiler::check(&self.linked()?, &format!("{}:main", self.entry))
    }

    fn linked(&self) -> Result<Module> {
        let mut linked = Module {
            structs: BTreeMap::new(),
            type_names: BTreeMap::new(),
            imports: Vec::new(),
            functions: BTreeMap::new(),
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
                for (_, ty) in &mut function.params {
                    self.resolve_type(id, ty)?;
                }
                self.resolve_type(id, &mut function.result)?;
                self.resolve_body(id, &mut function.body)?;
                linked.functions.insert(format!("{id}:{name}"), function);
            }
        }
        Ok(linked)
    }

    fn resolve_type(&self, module: &str, ty: &mut compiler::Type) -> Result<()> {
        if let compiler::Type::Record(id) = ty {
            let name = self.modules[module]
                .module
                .type_names
                .get(id)
                .ok_or("unknown type name")?;
            let mut constructor = Expr::Record(name.clone(), Vec::new());
            self.resolve_expr(module, &mut constructor)?;
            if let Expr::Record(name, _) = constructor {
                *ty = compiler::Type::Record(compiler::record_id(&name));
            }
        }
        Ok(())
    }

    fn resolve_body(&self, module: &str, body: &mut Block) -> Result<()> {
        for (instruction, _) in body {
            if let Instruction::Let(_, _, Some(ty), _) = instruction {
                self.resolve_type(module, ty)?;
            }
            let expr = match instruction {
                Instruction::Let(_, _, _, expr)
                | Instruction::Assign(_, expr)
                | Instruction::AssignField(_, _, expr)
                | Instruction::AssignIndex(_, _, expr)
                | Instruction::Print(expr)
                | Instruction::Call(expr)
                | Instruction::Fail(expr) => Some(expr),
                Instruction::Return(expr) => expr.as_mut(),
                Instruction::If(condition, yes, no) => {
                    self.resolve_body(module, yes)?;
                    self.resolve_body(module, no)?;
                    Some(condition)
                }
                Instruction::While(condition, nested) => {
                    self.resolve_body(module, nested)?;
                    Some(condition)
                }
                Instruction::Break | Instruction::Continue => None,
            };
            if let Some(expr) = expr {
                self.resolve_expr(module, expr)?;
            }
        }
        Ok(())
    }

    fn resolve_expr(&self, module: &str, expr: &mut Expr) -> Result<()> {
        match expr {
            Expr::Record(name, fields) => {
                let (target, short) = if let Some((alias, short)) = name.split_once('.') {
                    let imports: Vec<_> = self.modules[module]
                        .module
                        .imports
                        .iter()
                        .filter(|p| p.rsplit('/').next() == Some(alias))
                        .collect();
                    if imports.len() != 1 {
                        return Err("missing or ambiguous struct import".into());
                    }
                    (imports[0].as_str(), short)
                } else {
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
                    self.resolve_expr(module, value)?;
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
                        self.resolve_expr(module, arg)?;
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
                *name = self.resolve(module, name)?;
                for arg in args {
                    self.resolve_expr(module, arg)?;
                }
            }
            Expr::Unary(_, value)
            | Expr::ByteLen(value)
            | Expr::CharLen(value)
            | Expr::Try(value) => self.resolve_expr(module, value)?,
            Expr::Catch(inner, _, block) => {
                self.resolve_expr(module, inner)?;
                self.resolve_body(module, block)?;
            }
            Expr::Array(values) => {
                for value in values {
                    self.resolve_expr(module, value)?;
                }
            }
            Expr::Index(_, index) => self.resolve_expr(module, index)?,
            Expr::Binary(_, left, right) | Expr::StringEqual(left, right) => {
                self.resolve_expr(module, left)?;
                self.resolve_expr(module, right)?;
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
        self.check()?;
        let mut keys = BTreeMap::new();
        for id in self.modules.keys() {
            self.key(id, &mut keys, &mut BTreeSet::new())?;
        }
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
            println!(
                "CACHE {} {:.12} {id}",
                if hit { "HIT" } else { "MISS" },
                keys[id]
            );
        }
        let (tool, identity) = native::tool()?;
        let mut objects = Vec::new();
        let mut object_keys = Vec::new();
        for id in self.modules.keys() {
            let source = native::generate(&linked, &format!("{}:main", self.entry), id);
            let key = hash(&[
                VERSION.as_bytes(),
                source.as_bytes(),
                identity.as_bytes(),
                native::OPTIONS.as_bytes(),
                env::consts::OS.as_bytes(),
                env::consts::ARCH.as_bytes(),
            ]);
            let (bytes, hit) = native_cached(cache, &key, "obj", |scratch| {
                native::compile(&tool, scratch, &source)
            })?;
            println!(
                "OBJECT {} {:.12} {id}",
                if hit { "HIT" } else { "MISS" },
                key
            );
            objects.push(bytes);
            object_keys.push(key);
        }
        let app_key = hash(&[
            VERSION.as_bytes(),
            object_keys.join("\n").as_bytes(),
            identity.as_bytes(),
            native::OPTIONS.as_bytes(),
        ]);
        let (bytes, hit) = native_cached(cache, &app_key, "exe", |scratch| {
            native::link(&tool, scratch, &objects)
        })?;
        println!("LINK {} {:.12}", if hit { "HIT" } else { "MISS" }, app_key);
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        let artifact = self
            .root
            .join("build")
            .join(format!("{}-{app_key}{suffix}", self.name));
        atomic_write(&artifact, &bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        println!("OUTPUT {}", artifact.display());
        Ok(artifact)
    }
}

fn native_cached(
    cache: &Path,
    key: &str,
    extension: &str,
    build: impl FnOnce(&Path) -> Result<Vec<u8>>,
) -> Result<(Vec<u8>, bool)> {
    let path = cache.join(format!("{key}.{extension}"));
    let checksum = cache.join(format!("{key}.{extension}.sha256"));
    match fs::read(&path) {
        Ok(bytes) => {
            let expected = fs::read_to_string(&checksum)
                .map_err(|e| format!("missing native cache checksum: {e}"))?;
            if hash(&[&bytes]) != expected {
                return Err(format!("corrupt cache entry {}", path.display()));
            }
            Ok((bytes, true))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
        Err(e) => Err(e.to_string()),
    }
}

pub fn run_artifact(path: &Path) -> Result<()> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let status = std::process::Command::new(path)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("program failed: {status}"))
    }
}
