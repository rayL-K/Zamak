mod compiler;
mod native;
mod project;

use std::path::Path;

fn execute() -> project::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() > 2 {
        return Err("usage: zam <check|build|run> [project|file]".into());
    }
    let input = Path::new(args.get(1).map(String::as_str).unwrap_or("."));
    match args.first().map(String::as_str) {
        Some("check") => {
            let project = project::Project::load(input)?;
            project.check()?;
            println!("CHECK OK {}", project.name);
        }
        Some("build") => { project::Project::load(input)?.build(&project::cache_dir())?; }
        Some("run") => {
            // 源码后缀统一为 .zm；其余已存在的文件按原生产物直接执行。
            let source = input.extension().is_some_and(|ext| ext == "zm");
            let artifact = if input.is_file() && !source { input.into() }
                else { project::Project::load(input)?.build(&project::cache_dir())? };
            project::run_artifact(&artifact)?;
        }
        Some("demo") if args.len() == 1 => demo()?,
        None | Some("--help") | Some("-h") => println!("zam {}\n  zam check [project|file]\n  zam build [project|file]\n  zam run [project|file|artifact]\n  zam demo", env!("CARGO_PKG_VERSION")),
        _ => return Err("usage: zam <check|build|run> [project|file]".into()),
    }
    Ok(())
}

fn demo() -> project::Result<()> {
    let root = std::env::temp_dir().join(format!(
        "zam-demo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let cache = root.join("cache");
    let result = (|| {
        for directory in ["a", "b"] {
            let dir = root.join(directory);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(
                dir.join("hello.zm"),
                "fn main() { println(\"hello from zamak\") }",
            )
            .map_err(|e| e.to_string())?;
            project::Project::load(&dir.join("hello.zm"))?.build(&cache)?;
        }
        let count = std::fs::read_dir(&cache)
            .map_err(|e| e.to_string())?
            .count();
        if count != 3 {
            return Err("path-independent caching failed".into());
        }
        println!("path-independent content-addressed cache verified");
        Ok(())
    })();
    std::fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
    result
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("zam: {error}");
        std::process::exit(1);
    }
}
