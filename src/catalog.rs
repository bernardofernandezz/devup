use crate::config::{config_dir, repo_name};
use crate::dotfiles::{assets_dir, repos_dir, stow_files};
use crate::pm::Pm;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

const DEFAULT: &str = include_str!("../catalog/default.toml");
pub const DESKTOPS: [&str; 4] = ["gnome", "kde", "hyprland", "niri"];
pub const CATEGORIES: [&str; 8] = ["dev", "games", "general", "server", "shell", "desktop", "theme", "dotfiles"];

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(default, rename = "module")]
    pub modules: Vec<Module>,
    #[serde(default, rename = "profile")]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub name: String,
    pub category: String,
    pub description: String,
    #[serde(default)]
    pub requires: Vec<String>,
    /// Só aplica nestes desktops (vazio = todos).
    #[serde(default)]
    pub desktop: Vec<String>,
    /// Comando sh; se der sucesso, o módulo é pulado.
    pub skip_if: Option<String>,
    pub pacman: Option<String>,
    pub apt: Option<String>,
    pub dnf: Option<String>,
    pub zypper: Option<String>,
    pub apk: Option<String>,
    pub xbps: Option<String>,
    pub aur: Option<String>,
    pub flatpak: Option<String>,
    pub script: Option<String>,
    pub bin: Option<String>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub run: Vec<String>,
    #[serde(default)]
    pub files: Vec<FileLink>,
    /// Pasta base dos `files.src`: assets embutidos, pasta do .toml do usuário ou repo de dotfiles.
    #[serde(skip)]
    pub root: PathBuf,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FileLink {
    pub src: String,
    /// Sempre dentro do home: `~/...`
    pub dest: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub modules: Vec<String>,
}

impl Module {
    pub fn native(&self, pm: Pm) -> Option<&str> {
        match pm {
            Pm::Pacman => &self.pacman,
            Pm::Apt => &self.apt,
            Pm::Dnf => &self.dnf,
            Pm::Zypper => &self.zypper,
            Pm::Apk => &self.apk,
            Pm::Xbps => &self.xbps,
        }
        .as_deref()
    }

    pub fn has_install(&self) -> bool {
        [&self.pacman, &self.apt, &self.dnf, &self.zypper, &self.apk, &self.xbps, &self.aur, &self.flatpak, &self.script]
            .iter()
            .any(|f| f.is_some())
    }
}

impl Catalog {
    /// Camadas, a de baixo sobrescreve pelo nome: embutido, ~/.config/devup/modules/*.toml,
    /// e cada repo de dotfiles já clonado (`devup/*.toml`, ou um módulo stow com o nome do repo).
    pub fn load(repos: &[String]) -> Result<Catalog, String> {
        let mut cat = parse(DEFAULT, "catálogo embutido", &assets_dir())?;
        let modules_dir = config_dir().join("modules");
        for path in toml_files(&modules_dir) {
            cat.merge(parse_file(&path, &modules_dir)?);
        }
        for url in repos {
            let (name, repo) = (repo_name(url), repos_dir().join(repo_name(url)));
            if !repo.exists() {
                continue;
            }
            if repo.join("devup").is_dir() {
                for path in toml_files(&repo.join("devup")) {
                    cat.merge(parse_file(&path, &repo)?);
                }
            } else {
                let files = stow_files(&repo).map_err(|e| format!("{}: {e}", repo.display()))?;
                let description = format!("dotfiles de {url}");
                let module = Module { name, category: "dotfiles".into(), description, files, root: repo, ..Default::default() };
                cat.merge(Catalog { modules: vec![module], ..Default::default() });
            }
        }
        cat.validate()?;
        Ok(cat)
    }

    #[cfg(test)]
    pub fn builtin() -> Catalog {
        parse(DEFAULT, "default", &assets_dir()).unwrap()
    }

    fn merge(&mut self, other: Catalog) {
        for m in other.modules {
            match self.modules.iter_mut().find(|x| x.name == m.name) {
                Some(slot) => *slot = m,
                None => self.modules.push(m),
            }
        }
        self.profiles.extend(other.profiles);
    }

    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.name == name)
    }

    /// Expande nomes de módulos/perfis numa lista sem repetição, com dependências antes de quem as usa.
    pub fn expand(&self, names: &[String]) -> Result<Vec<&Module>, String> {
        let mut out = vec![];
        for n in names {
            self.visit(n, &mut out, &mut vec![])?;
        }
        Ok(out)
    }

    fn visit<'a>(&'a self, name: &str, out: &mut Vec<&'a Module>, stack: &mut Vec<String>) -> Result<(), String> {
        if stack.iter().any(|s| s == name) {
            return Err(format!("ciclo: {} -> {name}", stack.join(" -> ")));
        }
        let children: Vec<&String> = if let Some(m) = self.module(name) {
            if out.iter().any(|o| o.name == name) {
                return Ok(());
            }
            m.requires.iter().collect()
        } else if let Some(p) = self.profiles.get(name) {
            p.include.iter().chain(&p.modules).collect()
        } else {
            let known = self.modules.iter().map(|m| m.name.as_str()).chain(self.profiles.keys().map(String::as_str));
            return Err(format!("app ou perfil desconhecido: {name}{} · devup list mostra todos", crate::ui::did_you_mean(name, known)));
        };
        stack.push(name.to_string());
        for c in children {
            self.visit(c, out, stack)?;
        }
        stack.pop();
        if let Some(m) = self.module(name) {
            out.push(m);
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        for m in &self.modules {
            if !CATEGORIES.contains(&m.category.as_str()) {
                return Err(format!("{}: categoria inválida '{}'", m.name, m.category));
            }
            if self.profiles.contains_key(&m.name) {
                return Err(format!("{}: nome usado por módulo e perfil", m.name));
            }
            if !m.has_install() && m.services.is_empty() && m.groups.is_empty() && m.run.is_empty() && m.files.is_empty() {
                return Err(format!("{}: módulo sem nada a fazer", m.name));
            }
            if let Some(d) = m.desktop.iter().find(|d| !DESKTOPS.contains(&d.as_str())) {
                return Err(format!("{}: desktop inválido '{d}' (use {})", m.name, DESKTOPS.join(", ")));
            }
            for f in &m.files {
                let escapes = Path::new(&f.src).components().any(|c| !matches!(c, Component::Normal(_)));
                if escapes || !f.dest.starts_with("~/") || f.dest.contains("/../") {
                    return Err(format!("{}: arquivo inválido {} -> {} (src relativo, dest em ~/)", m.name, f.src, f.dest));
                }
            }
        }
        // Expandir tudo pega referências desconhecidas e ciclos.
        let all: Vec<String> = self.modules.iter().map(|m| m.name.clone()).chain(self.profiles.keys().cloned()).collect();
        self.expand(&all).map(|_| ())
    }
}

fn parse(text: &str, origin: &str, root: &Path) -> Result<Catalog, String> {
    let mut cat: Catalog = toml::from_str(text).map_err(|e| format!("{origin}: {e}"))?;
    cat.modules.iter_mut().for_each(|m| m.root = root.to_path_buf());
    Ok(cat)
}

fn parse_file(path: &Path, root: &Path) -> Result<Catalog, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text, &path.display().to_string(), root)
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(text: &str) -> Catalog {
        parse(text, "test", Path::new("/r")).unwrap()
    }

    fn names(mods: Vec<&Module>) -> Vec<&str> {
        mods.iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn default_catalog_is_valid() {
        Catalog::builtin().validate().unwrap();
    }

    #[test]
    fn expand_profiles_requires_and_dedup() {
        let c = cat(r#"
            [profile.a]
            modules = ["y", "x"]
            [profile.b]
            include = ["a"]
            modules = ["x"]
            [[module]]
            name = "x"
            category = "dev"
            description = ""
            run = ["true"]
            [[module]]
            name = "y"
            category = "dev"
            description = ""
            requires = ["x"]
            run = ["true"]
        "#);
        assert_eq!(names(c.expand(&["b".into()]).unwrap()), ["x", "y"]);
        assert!(c.expand(&["nope".into()]).is_err());
    }

    #[test]
    fn detects_cycles() {
        let c = cat(r#"
            [[module]]
            name = "x"
            category = "dev"
            description = ""
            requires = ["y"]
            run = ["true"]
            [[module]]
            name = "y"
            category = "dev"
            description = ""
            requires = ["x"]
            run = ["true"]
        "#);
        assert!(c.validate().unwrap_err().contains("ciclo"));
    }

    #[test]
    fn user_layer_overrides_by_name() {
        let mut c = Catalog::builtin();
        let before = c.modules.len();
        c.merge(cat(r#"
            [[module]]
            name = "git"
            category = "dev"
            description = "meu git"
            run = ["true"]
            [[module]]
            name = "novo"
            category = "dev"
            description = ""
            run = ["true"]
        "#));
        assert_eq!(c.module("git").unwrap().description, "meu git");
        assert_eq!(c.modules.len(), before + 1);
        c.validate().unwrap();
    }

    #[test]
    fn rejects_files_outside_root_or_home() {
        for (src, dest) in [("../x", "~/.x"), ("/etc/passwd", "~/.x"), ("a", "/etc/x"), ("a", "~/../x")] {
            let c = cat(&format!(r#"
                [[module]]
                name = "x"
                category = "dotfiles"
                description = ""
                files = [{{ src = "{src}", dest = "{dest}" }}]
            "#));
            assert!(c.validate().is_err(), "{src} -> {dest}");
        }
    }
}
