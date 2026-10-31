use std::{
    env, fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pm {
    Pacman,
    Apt,
    Dnf,
    Zypper,
}

impl Pm {
    pub fn bin(self) -> &'static str {
        match self {
            Pm::Pacman => "pacman",
            Pm::Apt => "apt-get",
            Pm::Dnf => "dnf",
            Pm::Zypper => "zypper",
        }
    }

    /// Detecta pelo binário instalado, o que cobre derivadas (Manjaro, Mint, Pop, Nobara...).
    fn detect(has: fn(&str) -> bool) -> Option<Pm> {
        [Pm::Pacman, Pm::Apt, Pm::Dnf, Pm::Zypper]
            .into_iter()
            .find(|p| has(p.bin()))
    }
}

/// Fatos do sistema de que o plano depende; injetáveis nos testes.
pub struct Sys {
    pub pm: Option<Pm>,
    pub root: bool,
    pub user: Option<String>,
    pub has: fn(&str) -> bool,
    pub home: PathBuf,
}

impl Sys {
    pub fn detect() -> Sys {
        Sys {
            pm: Pm::detect(has_bin),
            root: fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0),
            user: env::var("USER").ok(),
            has: has_bin,
            home: crate::config::home(),
        }
    }

    /// paru/yay recusam rodar como root.
    pub fn aur_helper(&self) -> Option<&'static str> {
        if self.pm != Some(Pm::Pacman) || self.root {
            return None;
        }
        ["paru", "yay"].into_iter().find(|h| (self.has)(h))
    }

    pub fn sudo(&self, cmd: &[&str]) -> Vec<String> {
        (!self.root).then_some("sudo").into_iter().chain(cmd.iter().copied()).map(String::from).collect()
    }

    pub fn install_cmds(&self, pkgs: &[&str]) -> Vec<Vec<String>> {
        let Some(pm) = self.pm else { return vec![] };
        let base: &[&str] = match pm {
            Pm::Pacman => &["pacman", "-S", "--needed", "--noconfirm"],
            Pm::Apt => &["apt-get", "install", "-y"],
            Pm::Dnf => &["dnf", "install", "-y"],
            Pm::Zypper => &["zypper", "--non-interactive", "install"],
        };
        let mut cmds = Vec::new();
        if pm == Pm::Apt {
            cmds.push(self.sudo(&["apt-get", "update"]));
        }
        cmds.push(self.sudo(&[base, pkgs].concat()));
        cmds
    }

    pub fn has_packages(&self, pkgs: &[String]) -> bool {
        let query: &[&str] = match self.pm {
            Some(Pm::Pacman) => &["pacman", "-Q"],
            Some(Pm::Apt) => &["dpkg", "-s"],
            Some(Pm::Dnf | Pm::Zypper) => &["rpm", "-q"],
            None => return false,
        };
        crate::plan::succeeds(&[strs(query), pkgs.to_vec()].concat())
    }
}

pub fn has_bin(name: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|p| env::split_paths(&p).any(|dir| Path::new(&dir).join(name).is_file()))
}

pub fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}
