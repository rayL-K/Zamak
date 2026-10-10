use std::{fs, process::Command};

fn zam(args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_zam"));
    command.args(args);
    command.output().unwrap()
}

#[test]
fn new_scaffolds_a_runnable_project_and_refuses_existing() {
    let root = std::env::temp_dir().join(format!("zam-new-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let project = root.join("hello");
    let project = project.to_str().unwrap();

    let output = zam(&["new", project]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(output.stdout.is_empty(), "stdout 只留给程序输出");
    assert!(stderr.contains("NEW "), "{stderr}");
    assert_eq!(
        fs::read_to_string(root.join("hello/zam.toml")).unwrap(),
        "[project]\nname = \"hello\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("hello/src/main.zm")).unwrap(),
        "fn main() {\n    println(\"hello from zamak\")\n}\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("hello/.gitignore")).unwrap(),
        "build/\n"
    );

    // 骨架能直接检查，而且本来就是格式化过的。
    assert!(zam(&["check", project]).status.success());
    let formatted = zam(&["fmt", project]);
    assert!(formatted.status.success());
    assert!(!String::from_utf8_lossy(&formatted.stderr).contains("FORMAT"));

    // 目录已存在就拒绝，绝不覆盖里面的文件。
    fs::write(root.join("hello/src/main.zm"), "fn main() {}\n").unwrap();
    let again = zam(&["new", project]);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("already exists"));
    assert_eq!(
        fs::read_to_string(root.join("hello/src/main.zm")).unwrap(),
        "fn main() {}\n"
    );

    // 项目名必须是标识符（zam.toml 的 name 会写进配置）。
    let bad = root.join("my-app");
    let rejected = zam(&["new", bad.to_str().unwrap()]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("identifier"));
    assert!(!bad.exists());

    fs::remove_dir_all(&root).unwrap();
}
