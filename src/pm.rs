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
    Apk,
    Xbps,
}

impl Pm {
    pub fn bin(self) -> &'static str {
        match self {
            Pm::Pacman => "pacman",
            Pm::Apt => "apt-get",
            Pm::Dnf => "dnf",
            Pm::Zypper => "zypper",
            Pm::Apk => "apk",
            Pm::Xbps => "xbps-install",
        }
    }

    /// Detecta pelo binário instalado, o que cobre derivadas (Manjaro, Mint, Pop, Nobara...).
    fn detect(has: fn(&str) -> bool) -> Option<Pm> {
        [Pm::Pacman, Pm::Apt, Pm::Dnf, Pm::Zypper, Pm::Apk, Pm::Xbps]
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
    pub desktop: Option<&'static str>,
}

impl Sys {
    pub fn detect() -> Sys {
        Sys {
            pm: Pm::detect(has_bin),
            root: fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0),
            user: env::var("USER").ok(),
            has: has_bin,
            home: crate::config::home(),
            desktop: detect_desktop(),
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

    /// Comando de atualizar o índice de pacotes, se o gerenciador precisar antes de instalar.
    pub fn refresh_cmd(&self) -> Option<Vec<String>> {
        (self.pm == Some(Pm::Apt)).then(|| self.sudo(&["apt-get", "update"]))
    }

    /// Comando de instalação sem os pacotes.
    pub fn install_base(&self) -> Option<Vec<String>> {
        let base: &[&str] = match self.pm? {
            Pm::Pacman => &["pacman", "-S", "--needed", "--noconfirm"],
            // Sem perguntas do debconf (tzdata etc.), já que a saída é capturada.
            Pm::Apt => &["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y"],
            Pm::Dnf => &["dnf", "install", "-y"],
            Pm::Zypper => &["zypper", "--non-interactive", "install"],
            Pm::Apk => &["apk", "add"],
            Pm::Xbps => &["xbps-install", "-Sy"],
        };
        Some(self.sudo(base))
    }

    pub fn has_packages(&self, pkgs: &[String]) -> bool {
        let query: &[&str] = match self.pm {
            Some(Pm::Pacman) => &["pacman", "-Q"],
            Some(Pm::Apt) => &["dpkg", "-s"],
            Some(Pm::Dnf | Pm::Zypper) => &["rpm", "-q"],
            Some(Pm::Apk) => &["apk", "info", "-e"],
            // xbps-query consulta um pacote por vez.
            Some(Pm::Xbps) => return pkgs.iter().all(|p| crate::plan::succeeds(&strs(&["xbps-query", p]))),
            None => return false,
        };
        crate::plan::succeeds(&[strs(query), pkgs.to_vec()].concat())
    }
}

fn detect_desktop() -> Option<&'static str> {
    let vars = ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"];
    let seen = vars.iter().filter_map(|v| env::var(v).ok()).collect::<Vec<_>>().join(":").to_lowercase();
    crate::catalog::DESKTOPS.into_iter().find(|d| seen.contains(d))
}

pub fn has_bin(name: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|p| env::split_paths(&p).any(|dir| Path::new(&dir).join(name).is_file()))
}

pub fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_commands_per_manager() {
        let sys = |pm| Sys { pm: Some(pm), root: true, user: None, has: |_| false, home: "/h".into(), desktop: None };
        assert_eq!(sys(Pm::Apk).install_base(), Some(strs(&["apk", "add"])));
        assert_eq!(sys(Pm::Xbps).install_base(), Some(strs(&["xbps-install", "-Sy"])));
        assert!(sys(Pm::Apt).refresh_cmd().is_some() && sys(Pm::Dnf).refresh_cmd().is_none());
    }
}
