use crate::catalog::FileLink;
use crate::config::{data_dir, state_dir};
use include_dir::{Dir, include_dir};
use std::{
    fs, io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/catalog/assets");
const MANIFEST: &str = ".devup-manifest";

pub fn assets_dir() -> PathBuf {
    data_dir().join("assets")
}

pub fn repos_dir() -> PathBuf {
    data_dir().join("dotfiles")
}

pub fn backups_dir() -> PathBuf {
    state_dir().join("backups")
}

/// Os links apontam para arquivos reais, então os assets embutidos vão para o disco antes.
pub fn extract_assets() -> io::Result<()> {
    let dir = assets_dir();
    fs::create_dir_all(&dir)?;
    ASSETS.extract(dir)
}

pub fn is_linked(src: &Path, dest: &Path) -> bool {
    fs::read_link(dest).is_ok_and(|t| t == src)
}

/// Faz `dest` apontar para `src`. O que já existir em `dest` vai para `backup`, espelhando o caminho relativo ao home.
/// Retorna se algo foi para o backup.
pub fn link(src: &Path, dest: &Path, home: &Path, backup: &Path) -> io::Result<bool> {
    if is_linked(src, dest) {
        return Ok(false);
    }
    if !src.exists() {
        return Err(io::Error::other(format!("origem não existe: {}", src.display())));
    }
    let existed = dest.symlink_metadata().is_ok();
    if existed {
        let rel = dest.strip_prefix(home).map_err(|_| io::Error::other("destino fora do HOME"))?;
        let saved = backup.join(rel);
        fs::create_dir_all(saved.parent().unwrap())?;
        fs::rename(dest, &saved)?;
        let mut manifest = fs::read_to_string(backup.join(MANIFEST)).unwrap_or_default();
        manifest.push_str(&format!("{}\n", rel.display()));
        fs::write(backup.join(MANIFEST), manifest)?;
    }
    fs::create_dir_all(dest.parent().unwrap())?;
    symlink(src, dest)?;
    Ok(existed)
}

/// Backups em ordem cronológica (o nome é o timestamp).
pub fn backups() -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(backups_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    ids.sort_by_key(|id| id.parse::<u64>().unwrap_or(0));
    ids
}

pub fn backup_entries(backup: &Path) -> Vec<String> {
    fs::read_to_string(backup.join(MANIFEST)).unwrap_or_default().lines().map(String::from).collect()
}

/// Devolve cada item do backup ao lugar original, removendo o link que o devup criou.
pub fn restore(backup: &Path, home: &Path) -> io::Result<Vec<PathBuf>> {
    let mut restored = vec![];
    for rel in backup_entries(backup) {
        let dest = home.join(&rel);
        if dest.symlink_metadata().is_ok_and(|m| m.is_symlink()) {
            fs::remove_file(&dest)?;
        }
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::rename(backup.join(&rel), &dest)?;
        restored.push(dest);
    }
    fs::remove_dir_all(backup)?;
    Ok(restored)
}

/// Repo sem `devup/`: cada pasta de topo é um pacote stow, espelhado em ~ arquivo por arquivo.
pub fn stow_files(repo: &Path) -> io::Result<Vec<FileLink>> {
    let mut files = vec![];
    for pkg in fs::read_dir(repo)? {
        let pkg = pkg?.path();
        let hidden = pkg.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if !pkg.is_dir() || hidden {
            continue;
        }
        for file in walk(&pkg)? {
            files.push(FileLink {
                src: file.strip_prefix(repo).unwrap().display().to_string(),
                dest: format!("~/{}", file.strip_prefix(&pkg).unwrap().display()),
            });
        }
    }
    files.sort_by(|a, b| a.src.cmp(&b.src));
    Ok(files)
}

fn walk(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = vec![];
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            out.extend(walk(&path)?);
        } else {
            out.push(path);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devup-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn link_backup_and_restore() {
        let root = tmp("link");
        let (home, backup) = (root.join("home"), root.join("backup"));
        let src = root.join("src.conf");
        fs::write(&src, "novo").unwrap();
        let dest = home.join(".config/app/app.conf");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(&dest, "antigo").unwrap();

        assert!(link(&src, &dest, &home, &backup).unwrap());
        assert!(is_linked(&src, &dest));
        assert_eq!(fs::read_to_string(backup.join(".config/app/app.conf")).unwrap(), "antigo");
        assert!(!link(&src, &dest, &home, &backup).unwrap()); // idempotente
        assert_eq!(backup_entries(&backup), [".config/app/app.conf"]);

        assert_eq!(restore(&backup, &home).unwrap(), [dest.clone()]);
        assert_eq!(fs::read_to_string(&dest).unwrap(), "antigo");
        assert!(!backup.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stow_layout() {
        let repo = tmp("stow");
        for f in ["nvim/.config/nvim/init.lua", "zsh/.zshrc", ".git/config"] {
            let p = repo.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "").unwrap();
        }
        let files: Vec<_> = stow_files(&repo).unwrap().into_iter().map(|f| (f.src, f.dest)).collect();
        assert_eq!(files, [
            ("nvim/.config/nvim/init.lua".to_string(), "~/.config/nvim/init.lua".to_string()),
            ("zsh/.zshrc".to_string(), "~/.zshrc".to_string()),
        ]);
        fs::remove_dir_all(repo).unwrap();
    }
}
