//! Behaviour that must be identical for the public (plain) and private (encrypted) store.
use octo_core::crypto::Key;
use octo_core::files::private::PrivateFiles;
use octo_core::files::public::PublicFiles;
use octo_core::files::{FileKind, FilesError};

macro_rules! common_tests {
    ($modname:ident, $make:expr) => {
        mod $modname {
            use super::*;

            fn store() -> (tempfile::TempDir, impl Fn() -> Store) {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path().join("root");
                let mk: fn(&std::path::Path) -> Store = $make;
                (dir, move || mk(&root))
            }

            #[test]
            fn write_read_roundtrip_creates_parents() {
                let (_d, open) = store();
                let s = open();
                let e = s.write("a/b/c.txt", b"hello").unwrap();
                assert_eq!(
                    (e.path.as_str(), e.kind, e.size),
                    ("a/b/c.txt", FileKind::File, 5)
                );
                assert_eq!(s.read("a/b/c.txt").unwrap(), b"hello");
                let list = s.list().unwrap();
                let paths: Vec<_> = list.iter().map(|e| (e.path.as_str(), e.kind)).collect();
                assert_eq!(
                    paths,
                    [
                        ("a", FileKind::Dir),
                        ("a/b", FileKind::Dir),
                        ("a/b/c.txt", FileKind::File)
                    ]
                );
            }

            #[test]
            fn overwrite_replaces_content() {
                let (_d, open) = store();
                let s = open();
                s.write("f.txt", b"one").unwrap();
                s.write("f.txt", b"three").unwrap();
                assert_eq!(s.read("f.txt").unwrap(), b"three");
                assert_eq!(s.list().unwrap().len(), 1);
            }

            #[test]
            fn persists_across_reopen() {
                let (_d, open) = store();
                open().write("x/y.txt", b"data").unwrap();
                assert_eq!(open().read("x/y.txt").unwrap(), b"data");
            }

            #[test]
            fn missing_and_type_errors() {
                let (_d, open) = store();
                let s = open();
                assert!(matches!(s.read("nope"), Err(FilesError::NotFound)));
                s.mkdir("d").unwrap();
                s.write("f", b"1").unwrap();
                assert!(matches!(s.read("d"), Err(FilesError::IsDir)));
                assert!(matches!(s.write("d", b"x"), Err(FilesError::IsDir)));
                let r = s.write("f/child", b"x");
                assert!(matches!(r, Err(FilesError::NotDir)), "{r:?}");
                assert!(matches!(s.mkdir("f"), Err(FilesError::Exists)));
                assert!(s.mkdir("d").is_ok(), "mkdir is idempotent for dirs");
            }

            #[test]
            fn mkdir_nested() {
                let (_d, open) = store();
                let s = open();
                s.mkdir("p/q/r").unwrap();
                let l = s.list().unwrap();
                assert_eq!(l.len(), 3);
                assert!(l.iter().all(|e| e.kind == FileKind::Dir));
            }

            #[test]
            fn rename_file_and_dir_tree() {
                let (_d, open) = store();
                let s = open();
                s.write("d/a.txt", b"A").unwrap();
                s.write("d/sub/b.txt", b"B").unwrap();
                s.rename("d/a.txt", "d/z.txt").unwrap();
                assert_eq!(s.read("d/z.txt").unwrap(), b"A");
                s.rename("d", "e/moved").unwrap();
                assert_eq!(s.read("e/moved/sub/b.txt").unwrap(), b"B");
                assert_eq!(s.read("e/moved/z.txt").unwrap(), b"A");
                assert!(matches!(s.read("d/z.txt"), Err(FilesError::NotFound)));
                let paths: Vec<_> = s.list().unwrap().into_iter().map(|e| e.path).collect();
                assert!(!paths.iter().any(|p| p == "d" || p.starts_with("d/")));
            }

            #[test]
            fn rename_errors() {
                let (_d, open) = store();
                let s = open();
                s.write("a", b"1").unwrap();
                s.write("b", b"2").unwrap();
                s.mkdir("dir").unwrap();
                assert!(matches!(s.rename("a", "b"), Err(FilesError::Exists)));
                assert!(matches!(s.rename("zz", "y"), Err(FilesError::NotFound)));
                assert!(matches!(
                    s.rename("dir", "dir/inner"),
                    Err(FilesError::Invalid)
                ));
                assert!(matches!(s.rename("", "x"), Err(FilesError::Invalid)));
                assert!(matches!(s.rename("a", ""), Err(FilesError::Invalid)));
                assert!(matches!(s.rename("a", "../x"), Err(FilesError::Path(_))));
                assert_eq!(s.read("a").unwrap(), b"1");
                assert_eq!(s.read("b").unwrap(), b"2");
            }

            #[test]
            fn delete_file_empty_dir_but_not_full_dir() {
                let (_d, open) = store();
                let s = open();
                s.write("d/f", b"1").unwrap();
                assert!(matches!(s.delete("d"), Err(FilesError::NotEmpty)));
                s.delete("d/f").unwrap();
                s.delete("d").unwrap();
                assert!(s.list().unwrap().is_empty());
                assert!(matches!(s.delete("d"), Err(FilesError::NotFound)));
                assert!(matches!(s.delete(""), Err(FilesError::Invalid)));
            }

            #[test]
            fn traversal_is_rejected_everywhere() {
                let (d, open) = store();
                let s = open();
                std::fs::write(d.path().join("outside.txt"), b"x").unwrap();
                for p in ["../outside.txt", "a/../../outside.txt", "/etc/passwd", ".."] {
                    assert!(matches!(s.read(p), Err(FilesError::Path(_))), "{p}");
                    assert!(
                        matches!(s.write(p, b"evil"), Err(FilesError::Path(_))),
                        "{p}"
                    );
                    assert!(matches!(s.mkdir(p), Err(FilesError::Path(_))), "{p}");
                    assert!(matches!(s.delete(p), Err(FilesError::Path(_))), "{p}");
                    assert!(matches!(s.rename(p, "ok"), Err(FilesError::Path(_))), "{p}");
                }
                assert_eq!(std::fs::read(d.path().join("outside.txt")).unwrap(), b"x");
            }

            #[test]
            fn unicode_and_spaces_in_names() {
                let (_d, open) = store();
                let s = open();
                s.write("Мои заметки/файл 1.txt", "привет".as_bytes())
                    .unwrap();
                assert_eq!(
                    s.read("Мои заметки/файл 1.txt").unwrap(),
                    "привет".as_bytes()
                );
            }
        }
    };
}

enum Store {
    Public(PublicFiles),
    Private(PrivateFiles),
}

macro_rules! delegate {
    ($($name:ident($($arg:ident: $t:ty),*) -> $ret:ty;)*) => {
        impl Store {
            $(fn $name(&self, $($arg: $t),*) -> $ret {
                match self { Store::Public(s) => s.$name($($arg),*), Store::Private(s) => s.$name($($arg),*) }
            })*
        }
    };
}

delegate! {
    list() -> Result<Vec<octo_core::files::FileEntry>, FilesError>;
    write(path: &str, data: &[u8]) -> Result<octo_core::files::FileEntry, FilesError>;
    read(path: &str) -> Result<Vec<u8>, FilesError>;
    mkdir(path: &str) -> Result<(), FilesError>;
    rename(from: &str, to: &str) -> Result<(), FilesError>;
    delete(path: &str) -> Result<(), FilesError>;
}

common_tests!(public, |root| Store::Public(
    PublicFiles::open(root).unwrap()
));
common_tests!(private, |root| Store::Private(
    PrivateFiles::open(root, Key::from_bytes([3u8; 32])).unwrap()
));
