use std::{fs, process::Command};

fn zam(args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_zam"));
    command.args(args);
    command.output().unwrap()
}

/// TUTORIAL.md 声称每段代码都实测过，这里守住这句话：把 ```zamak 围栏里的代码抽出来逐个 `zam check`。
#[test]
fn every_tutorial_snippet_checks() {
    let document = include_str!("../../TUTORIAL.md");
    let lines: Vec<&str> = document.lines().collect();
    let mut blocks: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_end() == "```zamak" {
            let mut text = String::new();
            i += 1;
            while i < lines.len() && lines[i].trim_end() != "```" {
                text.push_str(lines[i]);
                text.push('\n');
                i += 1;
            }
            blocks.push(text);
        }
        i += 1;
    }
    assert!(blocks.len() >= 14, "教程的代码块被误改：{}", blocks.len());

    let root = std::env::temp_dir().join(format!("zam-tutorial-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    // 多模块小节：`use greeting` 的入口块要和定义 `pub fn hello()` 的模块块合起来当项目检查。
    let entry = blocks
        .iter()
        .find(|block| block.contains("use greeting"))
        .expect("教程缺少多模块入口");
    let module = blocks
        .iter()
        .find(|block| block.contains("pub fn hello()"))
        .expect("教程缺少模块文件");
    let project = root.join("hello");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("zam.toml"),
        "[project]\nname = \"hello\"\nentry = \"src/main.zm\"\nmodules = \"src\"\n",
    )
    .unwrap();
    fs::write(project.join("src/main.zm"), entry).unwrap();
    fs::write(project.join("src/greeting.zm"), module).unwrap();
    let output = zam(&["check", project.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "多模块小节: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // 其余代码块都是自成一体的程序，单独检查。
    let mut checked = 0;
    for (index, block) in blocks.iter().enumerate() {
        if block.contains("use greeting") || block.contains("pub fn hello()") {
            continue;
        }
        let file = root.join(format!("snippet-{index}.zm"));
        fs::write(&file, block).unwrap();
        let output = zam(&["check", file.to_str().unwrap()]);
        assert!(
            output.status.success(),
            "代码块 {index}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        checked += 1;
    }
    assert!(checked >= 12, "被检查的教程代码块太少：{checked}");

    fs::remove_dir_all(&root).unwrap();
}
