use std::{fs, process::Command};

#[test]
fn borrowed_byte_length() {
    let path = std::env::temp_dir().join(format!("zam-strings-{}.zm", std::process::id()));
    let invoke = |command| {
        Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg(command)
            .arg(&path)
            .output()
            .unwrap()
    };
    fs::write(&path, "use std/string\nfn size(s: &string) -> i64 { string.byte_len(s) }\nfn main() {\nlet s = \"你好\"\nprintln(string.byte_len(&s))\nprintln(size(&s))\nprintln(s)\nlet empty = \"\"\nprintln(string.byte_len(&empty))\nstring.byte_len(&s)\n}\n").unwrap();
    let output = invoke("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("6\n6\n你好\n0\n"));
    for source in [
        "fn main() { let s = \"x\"\nprintln(string.byte_len(&s)) }",
        "use std/string\nfn main() { string.byte_len() }",
        "use std/string\nfn main() { let s = \"x\"\nstring.byte_len(s) }",
        "use std/string\nfn main() { let n = 1\nstring.byte_len(&n) }",
        "use std/string\nfn main() { let mut s = \"x\"\nstring.byte_len(&mut s) }",
        "use std/string\nfn main() { let s = \"x\"\nlet moved = s\nstring.byte_len(&s) }",
        "use std/string\nfn main() { let s = \"x\"\nlet b: bool = string.byte_len(&s) }",
    ] {
        fs::write(&path, source).unwrap();
        assert!(!invoke("check").status.success(), "accepted {source}");
    }
    fs::write(&path, "use std/string\nfn same(a: &string, b: &string) -> bool { string.equal(a, b) }\nfn main() {\nlet a = \"你好\"\nlet b = \"你好\"\nlet c = \"您好\"\nlet empty = \"\"\nprintln(same(&a, &b))\nprintln(string.equal(&a, &c))\nprintln(string.equal(&a, &empty))\nprintln(string.equal(&empty, &empty))\nprintln(string.equal(&a, &a))\nprintln(a)\nstring.equal(&a, &b)\n}\n").unwrap();
    let output = invoke("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).ends_with("true\nfalse\nfalse\ntrue\ntrue\n你好\n")
    );
    for source in [
        "use std/string\nfn main() { let s = \"x\"\nstring.equal(&s) }",
        "use std/string\nfn main() { let s = \"x\"\nstring.equal(&s, s) }",
        "use std/string\nfn main() { let mut s = \"x\"\nstring.equal(&s, &mut s) }",
        "use std/string\nfn probe(s: &mut string, flag: bool) {}\nfn main() { let mut s = \"x\"\nprobe(&mut s, string.equal(&s, &s)) }",
        "use std/string\nfn main() { let s = \"x\"\nlet n: i64 = string.equal(&s, &s) }",
    ] {
        fs::write(&path, source).unwrap();
        assert!(!invoke("check").status.success(), "accepted {source}");
    }
    let declarations = "use std/string\nstruct Leaf { name: string }\nstruct Root { leaf: Leaf }\nfn change(s: &mut string) { s = \"updated\" }\nfn consume(r: Root) -> i64 { return 1 }\nfn hold(s: &string, n: i64) {}\nfn pair(a: &mut string, b: &string) {}\n";
    fs::write(&path, format!("{declarations}fn main() {{\nlet mut r = Root {{ leaf: Leaf {{ name: \"你好\" }} }}\nprintln(string.byte_len(&r.leaf.name))\nprintln(string.equal(&r.leaf.name, &r.leaf.name))\nchange(&mut r.leaf.name)\nprintln(&r.leaf.name)\nlet moved = r\nprintln(moved.leaf.name)\n}}" )).unwrap();
    let output = invoke("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("6\ntrue\nupdated\nupdated\n"));
    for body in [
        "hold(&r.leaf.name, consume(r))",
        "pair(&mut r.leaf.name, &r.leaf.name)",
        "let moved = r\nstring.byte_len(&r.leaf.name)",
        "let ref = &r.leaf.name",
        "string.byte_len(&r.leaf.missing)",
        "string.byte_len(&r.leaf)",
    ] {
        fs::write(&path, format!("{declarations}fn main() {{\nlet mut r = Root {{ leaf: Leaf {{ name: \"x\" }} }}\n{body}\n}}" )).unwrap();
        assert!(!invoke("check").status.success(), "accepted {body}");
    }
    fs::write(&path, format!("{declarations}fn main() {{\nlet r = Root {{ leaf: Leaf {{ name: \"x\" }} }}\nchange(&mut r.leaf.name)\n}}" )).unwrap();
    assert!(!invoke("check").status.success());
    fs::remove_file(path).unwrap();
}
