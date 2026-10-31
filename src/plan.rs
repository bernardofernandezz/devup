use crate::catalog::Module;
use crate::pm::{Sys, strs};
use std::process::{Command, ExitCode, Stdio};

#[derive(Debug, PartialEq)]
pub enum Method<'a> {
    Installed,
    Native(&'a str),
    Aur(&'a str),
    Flatpak(&'a str),
    Script(&'a str),
    /// Módulo só de configuração (services/groups/run).
    Nothing,
    Unavailable,
}

pub fn resolve<'a>(m: &'a Module, sys: &Sys) -> Method<'a> {
    if m.bin.as_deref().is_some_and(sys.has) {
        Method::Installed
    } else if let Some(p) = sys.pm.and_then(|pm| m.native(pm)) {
        Method::Native(p)
    } else if let (Some(a), Some(_)) = (&m.aur, sys.aur_helper()) {
        Method::Aur(a)
    } else if let Some(f) = &m.flatpak {
        Method::Flatpak(f)
    } else if let Some(s) = &m.script {
        Method::Script(s)
    } else if m.has_install() {
        Method::Unavailable
    } else {
        Method::Nothing
    }
}

/// Consulta o sistema; None quando não dá para saber (script sem `bin`, módulo só de config).
pub fn installed(m: &Module, sys: &Sys) -> Option<bool> {
    let pkgs = |p: &str| p.split_whitespace().map(String::from).collect::<Vec<_>>();
    match resolve(m, sys) {
        Method::Installed => Some(true),
        Method::Native(p) => Some(sys.has_packages(&pkgs(p))),
        Method::Aur(p) => Some(succeeds(&[strs(&["pacman", "-Q"]), pkgs(p)].concat())),
        Method::Flatpak(f) => Some(succeeds(&strs(&["flatpak", "info", "--user", f]))),
        Method::Script(_) if m.bin.is_some() => Some(false),
        _ => None,
    }
}

pub fn succeeds(cmd: &[String]) -> bool {
    Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Instalações agrupadas (nativo, aur, flatpak), depois scripts, depois services/groups/run na ordem dos módulos.
pub fn plan(mods: &[&Module], sys: &Sys) -> Vec<Vec<String>> {
    let (mut native, mut aur, mut flatpak, mut scripts, mut post) = (vec![], vec![], vec![], vec![], vec![]);
    for m in mods {
        match resolve(m, sys) {
            Method::Installed => println!("✓ {} já instalado", m.name),
            Method::Native(p) => native.extend(p.split_whitespace()),
            Method::Aur(p) => aur.extend(p.split_whitespace()),
            Method::Flatpak(f) => flatpak.push(f),
            Method::Script(s) => scripts.push(s),
            Method::Nothing => {}
            Method::Unavailable => {
                eprintln!("⚠ {}: sem método de instalação para esta distro", m.name);
                continue;
            }
        }
        if !m.services.is_empty() && !(sys.has)("systemctl") {
            eprintln!("⚠ {}: sem systemctl, serviços não habilitados", m.name);
        } else {
            post.extend(m.services.iter().map(|s| sys.sudo(&["systemctl", "enable", "--now", s])));
        }
        if let (Some(user), false) = (&sys.user, sys.root) {
            post.extend(m.groups.iter().map(|g| sys.sudo(&["usermod", "-aG", g, user])));
        }
        post.extend(m.run.iter().map(|r| strs(&["sh", "-c", r])));
    }

    let mut cmds = vec![];
    if !flatpak.is_empty() && !(sys.has)("flatpak") {
        native.push("flatpak");
    }
    if !native.is_empty() {
        cmds.extend(sys.install_cmds(&native));
    }
    if let (Some(helper), false) = (sys.aur_helper(), aur.is_empty()) {
        cmds.push(strs(&[&[helper, "-S", "--needed", "--noconfirm"], &aur[..]].concat()));
    }
    if !flatpak.is_empty() {
        cmds.push(strs(&[
            "flatpak", "remote-add", "--user", "--if-not-exists", "flathub",
            "https://dl.flathub.org/repo/flathub.flatpakrepo",
        ]));
        cmds.push(strs(&[&["flatpak", "install", "--user", "-y", "flathub"], &flatpak[..]].concat()));
    }
    cmds.extend(scripts.into_iter().map(|s| strs(&["sh", "-c", s])));
    cmds.extend(post);
    cmds
}

pub fn run(cmds: &[Vec<String>], dry_run: bool) -> ExitCode {
    let mut failed = vec![];
    for cmd in cmds {
        println!("→ {}", cmd.join(" "));
        if dry_run {
            continue;
        }
        let ok = Command::new(&cmd[0]).args(&cmd[1..]).status().is_ok_and(|s| s.success());
        if !ok {
            failed.push(cmd.join(" "));
        }
    }
    if failed.is_empty() {
        return ExitCode::SUCCESS;
    }
    eprintln!("\n✗ Falharam:");
    failed.iter().for_each(|c| eprintln!("  {c}"));
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::pm::Pm;

    fn sys(pm: Option<Pm>, root: bool, has: fn(&str) -> bool) -> Sys {
        Sys { pm, root, user: Some("me".into()), has }
    }

    #[test]
    fn resolve_order() {
        let cat = Catalog::builtin();
        let get = |n: &str| cat.module(n).unwrap();
        let none = sys(Some(Pm::Apt), false, |_| false);

        assert_eq!(resolve(get("lutris"), &none), Method::Native("lutris"));
        assert_eq!(resolve(get("lutris"), &sys(None, false, |_| false)), Method::Flatpak("net.lutris.Lutris"));
        assert!(matches!(resolve(get("rustup"), &none), Method::Script(_)));
        assert_eq!(resolve(get("rustup"), &sys(Some(Pm::Apt), false, |b| b == "rustup")), Method::Installed);
        assert_eq!(resolve(get("vscode"), &sys(Some(Pm::Pacman), false, |b| b == "paru")), Method::Aur("visual-studio-code-bin"));
        assert_eq!(resolve(get("vscode"), &sys(Some(Pm::Pacman), true, |b| b == "paru")), Method::Flatpak("com.visualstudio.code"));
    }

    #[test]
    fn plan_groups_and_post_steps() {
        let cat = Catalog::builtin();
        let mods = cat.expand(&["lazygit".into(), "btop".into(), "spotify".into(), "rustup".into(), "docker".into()]).unwrap();
        let cmds = plan(&mods, &sys(Some(Pm::Dnf), false, |b| b == "systemctl"));

        assert_eq!(cmds[0], strs(&["sudo", "dnf", "install", "-y", "git", "btop", "moby-engine", "docker-compose", "flatpak"]));
        assert_eq!(cmds.iter().filter(|c| c.contains(&"dnf".to_string())).count(), 1);
        assert!(cmds.iter().any(|c| c.last().unwrap() == "com.spotify.Client"));
        assert_eq!(cmds[cmds.len() - 3][0], "sh");
        assert_eq!(cmds[cmds.len() - 2], strs(&["sudo", "systemctl", "enable", "--now", "docker"]));
        assert_eq!(cmds[cmds.len() - 1], strs(&["sudo", "usermod", "-aG", "docker", "me"]));

        // lazygit não tem pacote no dnf: avisa e segue
        assert!(!cmds.concat().contains(&"lazygit".to_string()));

        let root = plan(&mods, &sys(Some(Pm::Dnf), true, |b| b == "systemctl"));
        assert_eq!(root[0][0], "dnf");
        assert!(!root.iter().any(|c| c[0] == "usermod"));
    }
}
