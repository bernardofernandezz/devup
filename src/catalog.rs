use crate::config::config_dir;
use crate::pm::Pm;
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::PathBuf};

const DEFAULT: &str = include_str!("../catalog/default.toml");
pub const CATEGORIES: [&str; 8] = ["dev", "games", "general", "server", "shell", "desktop", "theme", "dotfiles"];

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(default, rename = "module")]
    pub modules: Vec<Module>,
    #[serde(default, rename = "profile")]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub name: String,
    pub category: String,
    pub description: String,
    #[serde(default)]
    pub requires: Vec<String>,
    pub pacman: Option<String>,
    pub apt: Option<String>,
    pub dnf: Option<String>,
    pub zypper: Option<String>,
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
        }
        .as_deref()
    }

    pub fn has_install(&self) -> bool {
        [&self.pacman, &self.apt, &self.dnf, &self.zypper, &self.aur, &self.flatpak, &self.script]
            .iter()
            .any(|f| f.is_some())
    }
}

impl Catalog {
    /// Catálogo embutido + ~/.config/devup/modules/*.toml (mesmo nome sobrescreve).
    pub fn load() -> Result<Catalog, String> {
        let mut cat = parse(DEFAULT, "catálogo embutido")?;
        for path in user_files() {
            let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            cat.merge(parse(&text, &path.display().to_string())?);
        }
        cat.validate()?;
        Ok(cat)
    }

    #[cfg(test)]
    pub fn builtin() -> Catalog {
        parse(DEFAULT, "default").unwrap()
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
            return Err(format!("desconhecido: {name}. Veja `devup list`."));
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
            if !m.has_install() && m.services.is_empty() && m.groups.is_empty() && m.run.is_empty() {
                return Err(format!("{}: módulo sem nada a fazer", m.name));
            }
        }
        // Expandir tudo pega referências desconhecidas e ciclos.
        let all: Vec<String> = self.modules.iter().map(|m| m.name.clone()).chain(self.profiles.keys().cloned()).collect();
        self.expand(&all).map(|_| ())
    }
}

fn parse(text: &str, origin: &str) -> Result<Catalog, String> {
    toml::from_str(text).map_err(|e| format!("{origin}: {e}"))
}

fn user_files() -> Vec<PathBuf> {
    let Some(config) = config_dir() else { return vec![] };
    let mut files: Vec<_> = fs::read_dir(config.join("devup/modules"))
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
        parse(text, "test").unwrap()
    }

    fn names(mods: Vec<&Module>) -> Vec<&str> {
        mods.iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn default_catalog_is_valid() {
        parse(DEFAULT, "default").unwrap().validate().unwrap();
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
        let mut c = parse(DEFAULT, "default").unwrap();
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
}
