use std::{fs, path::Path, process::Command};

fn format(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("fmt")
        .arg(root)
        .output()
        .unwrap()
}

#[test]
fn fmt_reindents_a_project_and_refuses_broken_source() {
    let root = std::env::temp_dir().join(format!("zam-fmt-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("zam.toml"),
        "[project]\nname = \"hello\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    let entry = root.join("src/main.zm");
    let ugly = "fn main() {\n //保留注释\n let x =1\n\tif true {\n println(x)\n }\n}\n\n\n";
    fs::write(&entry, ugly).unwrap();

    let output = format(&root);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("FORMAT "), "{stderr}");
    let formatted = fs::read_to_string(&entry).unwrap();
    assert_eq!(
        formatted,
        "fn main() {\n    //保留注释\n    let x =1\n    if true {\n        println(x)\n    }\n}\n"
    );

    // 已经格式化过：不再改写，也不再报 FORMAT。
    let again = format(&root);
    assert!(again.status.success());
    assert!(!String::from_utf8_lossy(&again.stderr).contains("FORMAT"));
    assert_eq!(fs::read_to_string(&entry).unwrap(), formatted);

    // 语法坏掉的文件原样保留，命令失败。
    let broken = "fn main() {\nprintln(@)\n}\n";
    fs::write(&entry, broken).unwrap();
    let failed = format(&root);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("unexpected character"));
    assert_eq!(fs::read_to_string(&entry).unwrap(), broken);

    fs::remove_dir_all(&root).unwrap();
}
