use crate::catalog::{Catalog, Module};
use serde::{Deserialize, Serialize};
use std::{env, fs, io::ErrorKind, path::PathBuf};

pub fn config_dir() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// O setup desejado da máquina: `devup apply` converge para ele.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub profiles: Vec<String>,
    pub modules: Vec<String>,
    /// Módulos que não devem ser aplicados mesmo vindo de um perfil.
    pub exclude: Vec<String>,
}

impl Config {
    pub fn path() -> Result<PathBuf, String> {
        config_dir().map(|d| d.join("devup/devup.toml")).ok_or_else(|| "HOME não definido".into())
    }

    pub fn load() -> Result<Option<Config>, String> {
        let path = Self::path()?;
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let path = Self::path()?;
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        fs::create_dir_all(path.parent().unwrap())
            .and_then(|_| fs::write(&path, text))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    pub fn names(&self) -> Vec<String> {
        self.profiles.iter().chain(&self.modules).cloned().collect()
    }

    pub fn add(&mut self, cat: &Catalog, name: &str) -> Result<(), String> {
        let list = if cat.profiles.contains_key(name) {
            &mut self.profiles
        } else if cat.module(name).is_some() {
            &mut self.modules
        } else {
            return Err(format!("desconhecido: {name}. Veja `devup list`."));
        };
        if !list.iter().any(|n| n == name) {
            list.push(name.to_string());
        }
        self.exclude.retain(|n| n != name);
        Ok(())
    }

    pub fn remove(&mut self, cat: &Catalog, name: &str) -> Result<(), String> {
        let listed = self.names().iter().any(|n| n == name);
        self.profiles.retain(|n| n != name);
        self.modules.retain(|n| n != name);
        // Ainda vem de algum perfil: exclui explicitamente.
        let still_pulled = cat.module(name).is_some() && self.resolve(cat)?.iter().any(|m| m.name == name);
        if still_pulled {
            self.exclude.push(name.to_string());
        } else if !listed {
            return Err(format!("{name} não está no devup.toml"));
        }
        Ok(())
    }

    // shortcut: exclude remove o módulo mesmo se outro módulo o tiver em `requires`; avisar se isso confundir alguém.
    pub fn resolve<'a>(&self, cat: &'a Catalog) -> Result<Vec<&'a Module>, String> {
        Ok(cat.expand(&self.names())?.into_iter().filter(|m| !self.exclude.contains(&m.name)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_remove_exclude_roundtrip() {
        let cat = Catalog::builtin();
        let mut c = Config::default();
        c.add(&cat, "server").unwrap();
        c.add(&cat, "obsidian").unwrap();
        c.add(&cat, "obsidian").unwrap();
        assert!(c.add(&cat, "nope").is_err());
        assert_eq!((c.profiles.clone(), c.modules.clone()), (vec!["server".into()], vec!["obsidian".into()]));

        c.remove(&cat, "docker").unwrap(); // vem do perfil server
        assert_eq!(c.exclude, ["docker"]);
        assert!(!c.resolve(&cat).unwrap().iter().any(|m| m.name == "docker"));
        c.add(&cat, "docker").unwrap();
        assert!(c.exclude.is_empty());

        c.remove(&cat, "obsidian").unwrap();
        assert!(c.remove(&cat, "steam").is_err());

        let back: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}
