use octo_core::files::path::{PathError, normalize};

#[test]
fn normalizes() {
    assert_eq!(normalize("a/b.txt").unwrap(), "a/b.txt");
    assert_eq!(normalize("a//b/./c/").unwrap(), "a/b/c");
    assert_eq!(normalize("").unwrap(), "");
    assert_eq!(normalize(".").unwrap(), "");
    assert_eq!(normalize("./x").unwrap(), "x");
    assert_eq!(normalize("папка/файл.txt").unwrap(), "папка/файл.txt");
}

#[test]
fn rejects_traversal() {
    for p in [
        "..",
        "../x",
        "a/../b",
        "a/..",
        "a/b/../../..",
        "./../x",
        "..//x",
        "a/../../etc/passwd",
    ] {
        assert_eq!(normalize(p), Err(PathError::Traversal), "{p}");
    }
}

#[test]
fn rejects_absolute() {
    for p in [
        "/",
        "/etc/passwd",
        "//x",
        "C:\\x",
        "c:/x",
        "\\\\server\\share",
    ] {
        assert!(normalize(p).is_err(), "{p}");
    }
    assert_eq!(normalize("/etc"), Err(PathError::Absolute));
}

#[test]
fn rejects_backslash_nul_control_and_long() {
    for p in ["a\\..\\b", "a\\b", "a\0b", "a\nb", "a/\u{7f}"] {
        assert_eq!(normalize(p), Err(PathError::Invalid), "{p:?}");
    }
    assert_eq!(normalize(&"x".repeat(256)), Err(PathError::Invalid));
    assert!(normalize(&"x".repeat(255)).is_ok());
}

#[test]
fn dotdot_lookalikes_are_fine() {
    assert_eq!(normalize("..a/b..").unwrap(), "..a/b..");
    assert_eq!(normalize("a/...").unwrap(), "a/...");
}

#[test]
fn percent_encoded_is_literal() {
    // decoding is the HTTP layer's job; a literal "%2e%2e" is just a name
    assert_eq!(normalize("%2e%2e/x").unwrap(), "%2e%2e/x");
}

#[test]
fn rejects_colon_and_windows_reserved_names() {
    for p in [
        "ab:c",
        "dir/file:stream",
        "CON",
        "con",
        "a/NUL",
        "nul.txt",
        "x/Aux.tar.gz",
        "COM1",
        "lpt9.log",
        "prn",
    ] {
        assert_eq!(normalize(p), Err(PathError::Invalid), "{p}");
    }
    for p in [
        "console",
        "com0",
        "com10",
        "nullable.txt",
        "a/communal",
        "lpt",
    ] {
        assert!(normalize(p).is_ok(), "{p}");
    }
}
