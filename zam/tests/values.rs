use std::{fs, process::Command};

#[test]
fn scalar_values_and_checked_arithmetic() {
    let root = std::env::temp_dir().join(format!("zam-values-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let file = root.join("main.zm");
    let invoke = |command: &str| {
        Command::new(env!("CARGO_BIN_EXE_zam"))
            .arg(command)
            .arg(&file)
            .env("ZAMAK_CACHE_DIR", root.join("cache"))
            .output()
            .unwrap()
    };
    fs::write(&file, "fn twice(x: i64) -> i64 { x * 2 }\nfn yes(x: bool) -> bool { !x }\nfn main() {\nlet mut n: i64 = 20\nlet copy = n\nn = twice(n) + 2\nprintln(n)\nprintln(copy)\nprintln(n == 42)\nprintln(yes(false))\nprintln(-9223372036854775808)\nprintln(7 % 3)\n}\n").unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout)
        .ends_with("42\n20\ntrue\ntrue\n-9223372036854775808\n1\n"));
    fs::write(&file, "fn main() {\nprintln(1_000_000)\nprintln(-9_223_372_036_854_775_808)\nprintln(9_223_372_036_854_775_807)\nprintln(1_2 + 3_4)\n}\n").unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout)
        .ends_with("1000000\n-9223372036854775808\n9223372036854775807\n46\n"));
    for source in [
        "fn main() { println(1_) }",
        "fn main() { println(1__2) }",
        "fn main() { println(_1) }",
        "fn main() { println(9_223_372_036_854_775_808) }",
        "fn main() { println(-9_223_372_036_854_775_809) }",
        "fn main() { let x: bool = 1 }",
        "fn main() { println(true + 1) }",
        "fn main() { println(9223372036854775808) }",
        "fn main() { let x: i64 = \"text\" }",
        "fn main() { println(1 == false) }",
        "fn f(x: &string) {}\nfn main() {\nlet n = 1\nf(&n)\n}",
    ] {
        fs::write(&file, source).unwrap();
        assert!(!invoke("check").status.success(), "accepted {source}");
    }
    for expression in [
        "9223372036854775807 + 1",
        "-9223372036854775808 - 1",
        "9223372036854775807 * 2",
        "-9223372036854775808 * -1",
        "1 / 0",
        "-9223372036854775808 / -1",
        "-(-9223372036854775808)",
    ] {
        fs::write(&file, format!("fn main() {{ println({expression}) }}")).unwrap();
        let result = invoke("run");
        assert!(!result.status.success(), "accepted {expression}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("integer arithmetic error"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    fs::write(&file, include_str!("../../examples/values/arrays.zm")).unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).ends_with("10\n15\n30\n"));
    for source in [
        "fn main() { let a: [i64; 2] = [1] }",
        "fn main() { let a: [i64; 0] = [] }",
        "fn main() { let a: [i64; 1025] = [1] }",
        "fn main() { let a: [i64; 2] = [1, true] }",
        "fn main() { let a = [1, 2]\nprintln(a[2]) }",
        "fn main() { let a = [1, 2]\nprintln(a[-1]) }",
        "fn main() { let a = [1, 2]\na[0] = 3 }",
        "fn take(a: [i64; 2]) {}\nfn main() {}",
        "fn make() -> [i64; 2] { return [1, 2] }\nfn main() {}",
        "struct Boxed { values: [i64; 2] }\nfn main() {}",
        "fn main() { let a = [1, 2]\nprintln(a) }",
        "fn main() { let mut a = [1, 2]\na = [3, 4] }",
    ] {
        fs::write(&file, source).unwrap();
        assert!(!invoke("check").status.success(), "accepted {source}");
    }
    fs::write(
        &file,
        "fn main() { let a = [1, 2]\nlet i = 2\nprintln(a[i]) }",
    )
    .unwrap();
    let result = invoke("run");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("array index out of bounds"));
    fs::remove_dir_all(root).unwrap();
}
