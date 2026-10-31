use crate::catalog::{Catalog, Module};
use serde::{Deserialize, Serialize};
use std::{env, fs, io::ErrorKind, path::PathBuf};

pub fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).expect("HOME não definido")
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home().join(fallback))
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("devup")
}

pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("devup")
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("devup")
}

pub const SHELLS: [&str; 3] = ["zsh", "fish", "bash"];

/// O setup desejado da máquina: `devup apply` converge para ele.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub profiles: Vec<String>,
    pub modules: Vec<String>,
    /// Módulos que não devem ser aplicados mesmo vindo de um perfil.
    pub exclude: Vec<String>,
    /// Repos git de dotfiles, clonados em ~/.local/share/devup/dotfiles.
    pub dotfiles: Vec<String>,
    /// Shell padrão do usuário: um de SHELLS.
    pub shell: Option<String>,
    /// Tema: embutido ou ~/.config/devup/themes/<nome>.toml.
    pub theme: Option<String>,
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("devup.toml")
    }

    pub fn load() -> Result<Option<Config>, String> {
        let path = Self::path();
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let path = Self::path();
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
        // Repos no formato stow viram um módulo com o nome do repo, sempre aplicado.
        let stow = self.dotfiles.iter().map(|u| repo_name(u)).filter(|n| cat.module(n).is_some());
        if let Some(sh) = self.shell.as_ref().filter(|sh| !SHELLS.contains(&sh.as_str())) {
            return Err(format!("shell inválido: {sh} (use {})", SHELLS.join(", ")));
        }
        let names: Vec<String> = self.names().into_iter().chain(self.shell.clone()).chain(stow).collect();
        Ok(cat.expand(&names)?.into_iter().filter(|m| !self.exclude.contains(&m.name)).collect())
    }
}

pub fn repo_name(url: &str) -> String {
    let url = url.trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    url.rsplit(['/', ':']).next().unwrap_or(url).to_string()
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

    #[test]
    fn repo_names() {
        assert_eq!(repo_name("https://github.com/me/dotfiles.git"), "dotfiles");
        assert_eq!(repo_name("git@github.com:me/dots"), "dots");
        assert_eq!(repo_name("https://github.com/me/cfg/"), "cfg");
    }

    #[test]
    fn shell_pulls_its_dependencies() {
        let cat = Catalog::builtin();
        let mut c = Config { shell: Some("zsh".into()), ..Default::default() };
        let names: Vec<_> = c.resolve(&cat).unwrap().iter().map(|m| m.name.clone()).collect();
        assert_eq!(names, ["cli-tools", "starship", "shell-common", "zsh"]);
        c.shell = Some("tcsh".into());
        assert!(c.resolve(&cat).is_err());
    }
}
