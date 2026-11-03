use super::path::{PathError, normalize};
use super::{FileEntry, FileKind, FilesError};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Plain files under a root directory. Symlinks are never followed or listed.
pub struct PublicFiles {
    root: PathBuf,
}

fn modified(md: &fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl PublicFiles {
    pub fn open(root: &Path) -> Result<Self, FilesError> {
        fs::create_dir_all(root)?;
        Ok(Self { root: root.into() })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Joins a normalized path to the root, refusing any existing symlink component.
    fn resolve(&self, norm: &str) -> Result<PathBuf, FilesError> {
        let mut p = self.root.clone();
        for c in norm.split('/').filter(|c| !c.is_empty()) {
            p.push(c);
            match fs::symlink_metadata(&p) {
                Ok(md) if md.file_type().is_symlink() => return Err(PathError::Invalid.into()),
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(p)
    }

    fn target(&self, path: &str) -> Result<(String, PathBuf), FilesError> {
        let norm = normalize(path)?;
        if norm.is_empty() {
            return Err(FilesError::Invalid);
        }
        let p = self.resolve(&norm)?;
        Ok((norm, p))
    }

    pub fn list(&self) -> Result<Vec<FileEntry>, FilesError> {
        let mut out = vec![];
        walk(&self.root, "", &mut out)?;
        super::sort_entries(&mut out);
        Ok(out)
    }

    pub fn write(&self, path: &str, data: &[u8]) -> Result<FileEntry, FilesError> {
        let (norm, p) = self.target(path)?;
        if p.is_dir() {
            return Err(FilesError::IsDir);
        }
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(not_dir)?;
        }
        fs::write(&p, data).map_err(not_dir)?;
        let md = fs::metadata(&p)?;
        Ok(FileEntry {
            path: norm,
            kind: FileKind::File,
            size: md.len(),
            modified: modified(&md),
        })
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>, FilesError> {
        let (_, p) = self.target(path)?;
        match fs::metadata(&p) {
            Ok(md) if md.is_dir() => Err(FilesError::IsDir),
            Ok(_) => Ok(fs::read(&p)?),
            Err(e) => Err(not_found(e)),
        }
    }

    pub fn mkdir(&self, path: &str) -> Result<(), FilesError> {
        let (_, p) = self.target(path)?;
        if p.is_file() {
            return Err(FilesError::Exists);
        }
        fs::create_dir_all(&p).map_err(not_dir)?;
        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), FilesError> {
        let (nf, pf) = self.target(from)?;
        let (nt, pt) = self.target(to)?;
        if nt == nf || nt.starts_with(&format!("{nf}/")) {
            return Err(FilesError::Invalid);
        }
        if fs::symlink_metadata(&pf).is_err() {
            return Err(FilesError::NotFound);
        }
        if fs::symlink_metadata(&pt).is_ok() {
            return Err(FilesError::Exists);
        }
        if let Some(parent) = pt.parent() {
            fs::create_dir_all(parent).map_err(not_dir)?;
        }
        fs::rename(&pf, &pt)?;
        Ok(())
    }

    pub fn delete(&self, path: &str) -> Result<(), FilesError> {
        let (_, p) = self.target(path)?;
        let md = fs::symlink_metadata(&p).map_err(not_found)?;
        if md.is_dir() {
            fs::remove_dir(&p).map_err(|e| {
                if e.kind() == std::io::ErrorKind::DirectoryNotEmpty {
                    FilesError::NotEmpty
                } else {
                    e.into()
                }
            })
        } else {
            Ok(fs::remove_file(&p)?)
        }
    }
}

fn not_found(e: std::io::Error) -> FilesError {
    if e.kind() == std::io::ErrorKind::NotFound {
        FilesError::NotFound
    } else {
        e.into()
    }
}

/// Creating something below a regular file fails with NotADirectory/AlreadyExists.
fn not_dir(e: std::io::Error) -> FilesError {
    match e.kind() {
        std::io::ErrorKind::NotADirectory | std::io::ErrorKind::AlreadyExists => FilesError::NotDir,
        _ => e.into(),
    }
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<FileEntry>) -> Result<(), FilesError> {
    for e in fs::read_dir(dir)? {
        let e = e?;
        let md = e.metadata()?;
        if md.file_type().is_symlink() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if md.is_dir() {
            out.push(FileEntry {
                path: path.clone(),
                kind: FileKind::Dir,
                size: 0,
                modified: modified(&md),
            });
            walk(&e.path(), &path, out)?;
        } else if md.is_file() {
            out.push(FileEntry {
                path,
                kind: FileKind::File,
                size: md.len(),
                modified: modified(&md),
            });
        }
    }
    Ok(())
}
