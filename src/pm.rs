use std::{env, path::Path};

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
    pub fn detect() -> Option<Pm> {
        [Pm::Pacman, Pm::Apt, Pm::Dnf, Pm::Zypper]
            .into_iter()
            .find(|p| has_bin(p.bin()))
    }

    pub fn install_cmds(self, pkgs: &[&str]) -> Vec<Vec<String>> {
        let base: &[&str] = match self {
            Pm::Pacman => &["sudo", "pacman", "-S", "--needed", "--noconfirm"],
            Pm::Apt => &["sudo", "apt-get", "install", "-y"],
            Pm::Dnf => &["sudo", "dnf", "install", "-y"],
            Pm::Zypper => &["sudo", "zypper", "--non-interactive", "install"],
        };
        let mut cmds = Vec::new();
        if self == Pm::Apt {
            cmds.push(strs(&["sudo", "apt-get", "update"]));
        }
        cmds.push(strs(base).into_iter().chain(pkgs.iter().map(|s| s.to_string())).collect());
        cmds
    }
}

pub fn has_bin(name: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|p| env::split_paths(&p).any(|dir| Path::new(&dir).join(name).is_file()))
}

pub fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}
