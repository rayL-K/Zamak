use std::fs;
use std::process::Command;

#[test]
fn struct_declarations_and_rejections() {
    let path = std::env::temp_dir().join(format!("zam-structs-{}.zm", std::process::id()));
    for (source, accepted) in [
        (
            "pub struct Person {\nname: string\nage: i64\nactive: bool\n}\nfn main() {}",
            true,
        ),
        ("struct Empty {}\nfn main() {}", true),
        (
            "struct Person {\nname: string\nname: bool\n}\nfn main() {}",
            false,
        ),
        ("struct Person {\nname: &string\n}\nfn main() {}", false),
        ("struct Person {\nname: &mut string\n}\nfn main() {}", false),
        ("struct Person {}\nstruct Person {}\nfn main() {}", false),
        ("struct Person {}\nfn Person() {}\nfn main() {}", false),
        ("fn Person() {}\nstruct Person {}\nfn main() {}", false),
        ("struct string {}\nfn main() {}", false),
        ("struct Person {\nchild: Person\n}\nfn main() {}", false),
        ("struct Person {}\nfn main() { let p = Person() }", false),
    ] {
        fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("check")
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            accepted,
            "{source}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let declaration = "struct Person {\nname: string\nage: i64\n}\n";
    for body in [
        "let p = Person { name: \"a\" }",
        "let p = Person {\nname: \"a\"\nage: true\n}",
        "let p = Person {\nname: \"a\"\nage: 1\nage: 2\n}",
        "let p = Person {\nname: \"a\"\nage: 1\n}\nlet q = p\nprintln(p.age)",
        "let p = Person {\nname: \"a\"\nage: 1\n}\nlet s = p.name",
    ] {
        fs::write(&path, format!("{declaration}fn main() {{\n{body}\n}}")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("check")
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {body}");
    }
    fs::write(&path, format!("{declaration}fn main() {{\nlet mut p = Person {{\nname: \"first\"\nage: 41\n}}\nlet q = p\nprintln(q.age + 1)\np = Person {{\nname: \"second\"\nage: 7\n}}\nprintln(p.age)\n}}" )).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("42\n7\n"));
    for body in [
        "let p = Person {\nname: \"a\"\nage: 1\n}\np.age = 2",
        "let mut p = Person {\nname: \"a\"\nage: 1\n}\np.age = true",
        "let mut p = Person {\nname: \"a\"\nage: 1\n}\np.missing = 2",
        "let mut p = Person {\nname: \"a\"\nage: 1\n}\nlet q = p\np.name = \"b\"",
        "let mut p = Person {\nname: \"a\"\nage: 1\n}\nlet s = \"b\"\np.name = s\nprintln(s)",
        "let mut p = Person {\nname: \"a\"\nage: 1\n}\np.name = p.name",
    ] {
        fs::write(&path, format!("{declaration}fn main() {{\n{body}\n}}")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("check")
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {body}");
    }
    fs::write(&path, format!("{declaration}fn replacement() -> string {{ return \"updated\" }}\nfn main() {{\nlet mut p = Person {{\nname: \"first\"\nage: 0\n}}\nprintln(p.name)\nprintln(p.name)\nwhile p.age < 3 {{\np.name = replacement()\np.age = p.age + 1\n}}\nlet s = \"moved\"\np.name = s\nprintln(p.name)\nprintln(p.age)\nlet q = p\nprintln(q.name)\n}}" )).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("first\nfirst\nmoved\n3\nmoved\n"));
    for source in [
        "fn take(p: Missing) {}\nfn main() {}",
        "struct Person {}\nfn take(p: &Person) {}\nfn main() {}",
        "struct Person {}\nfn make() -> Person { return 1 }\nfn main() {}",
        "struct Person {}\nfn take(p: Person) {}\nfn main() {\nlet p = Person {}\ntake(p)\ntake(p)\n}",
        "struct Person {}\nstruct Other {}\nfn take(p: Person) {}\nfn main() { take(Other {}) }",
        "struct Person {}\nfn main() { let p: Missing = Person {} }",
    ] {
        fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam")).arg("check").arg(&path).output().unwrap();
        assert!(!output.status.success(), "accepted {source}");
    }
    fs::write(&path, "struct Person { name: string }\nfn make() -> Person { Person { name: \"returned\" } }\nfn relay(p: Person) -> Person { return p }\nfn consume(p: Person) { println(p.name) }\nfn main() {\nlet p: Person = relay(make())\nconsume(p)\nrelay(make())\n}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("returned\n"));
    for source in [
        "struct A { b: B }\nstruct B { a: A }\nfn main() {}",
        "struct A { b: Missing }\nfn main() {}",
        "struct B {}\nstruct A { b: B }\nfn main() {\nlet a = A { b: B {} }\nlet b = a.b\n}",
        "struct B {}\nstruct A { b: B }\nfn main() {\nlet a = A { b: B {} }\nprintln(a.b)\n}",
        "struct B {}\nstruct A { b: B }\nfn main() {\nlet a = A { b: B {} }\nprintln(a)\n}",
        "struct B {}\nstruct A { b: B }\nfn main() {\nlet mut a = A { b: B {} }\na.b = a.b\n}",
    ] {
        fs::write(&path, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("check")
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {source}");
    }
    fs::write(&path, "struct Leaf { name: string }\nstruct Branch { leaf: Leaf }\nstruct Root { branch: Branch }\nfn leaf() -> Leaf { Leaf { name: \"nested\" } }\nfn root() -> Root { Root { branch: Branch { leaf: leaf() } } }\nfn consume(r: Root) { println(8) }\nfn main() {\nlet mut r = root()\nr.branch = Branch { leaf: leaf() }\nr.branch.leaf = leaf()\nr.branch.leaf.name = \"deep\"\nprintln(r.branch.leaf.name)\nlet q = r\nconsume(q)\nr = root()\nroot()\n}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("deep\n8\n"));
    let nested = "struct Leaf {\nname: string\nage: i64\n}\nstruct Root { leaf: Leaf }\n";
    for body in [
        "let r = Root { leaf: Leaf {\nname: \"a\"\nage: 1\n} }\nr.leaf.age = 2",
        "let mut r = Root { leaf: Leaf {\nname: \"a\"\nage: 1\n} }\nr.leaf.age = true",
        "let r = Root { leaf: Leaf {\nname: \"a\"\nage: 1\n} }\nprintln(r.leaf.age.bad)",
        "let r = Root { leaf: Leaf {\nname: \"a\"\nage: 1\n} }\nlet q = r\nprintln(r.leaf.name)",
        "let r = Root { leaf: Leaf {\nname: \"a\"\nage: 1\n} }\nlet s = r.leaf.name",
        "let r = Root { leaf: Leaf {\nname: \"a\"\nage: 1\n} }\nprintln(r.leaf.missing)",
    ] {
        fs::write(&path, format!("{nested}fn main() {{\n{body}\n}}")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg("check")
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {body}");
    }
    fs::write(&path, format!("{nested}fn main() {{\nlet mut r = Root {{ leaf: Leaf {{\nname: \"first\"\nage: 1\n}} }}\nprintln(r.leaf.name)\nr.leaf.name = \"second\"\nr.leaf.age = r.leaf.age + 1\nprintln(r.leaf.age)\nprintln(r.leaf.name)\nlet q = r\nprintln(q.leaf.name)\n}}" )).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("first\n2\nsecond\nsecond\n"));
    let legacy = path.with_extension("zam");
    fs::copy(&path, &legacy).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zam"))
        .arg("check")
        .arg(&legacy)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "legacy source extension must be rejected"
    );
    fs::remove_file(legacy).unwrap();
    fs::remove_file(path).unwrap();
}
