use crate::catalog::{FileLink, Module};
use crate::dotfiles::{self, backups_dir, is_linked};
use crate::pm::{Sys, strs};
use crate::theme;
use std::{
    fmt,
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, PartialEq)]
pub enum Step {
    Cmd(Vec<String>),
    /// Instalação em grupo; se falhar, tenta pacote por pacote para só os inexistentes falharem.
    Packages { base: Vec<String>, pkgs: Vec<String> },
    Link { src: PathBuf, dest: PathBuf },
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Step::Cmd(c) => write!(f, "{}", c.join(" ")),
            Step::Packages { base, pkgs } => write!(f, "{} {}", base.join(" "), pkgs.join(" ")),
            Step::Link { src, dest } => write!(f, "link {} -> {}", dest.display(), src.display()),
        }
    }
}

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

/// Por que o módulo não se aplica nesta máquina, se for o caso.
pub fn skip_reason(m: &Module, sys: &Sys) -> Option<String> {
    if !m.desktop.is_empty() && !sys.desktop.is_some_and(|d| m.desktop.iter().any(|x| x == d)) {
        return Some(format!("só para {}", m.desktop.join("/")));
    }
    let cond = m.skip_if.as_ref()?;
    succeeds(&strs(&["sh", "-c", cond])).then(|| format!("pulado: {cond}"))
}

/// Consulta o sistema; None quando não dá para saber (script sem `bin`, módulo só de config).
pub fn installed(m: &Module, sys: &Sys) -> Option<bool> {
    let pkgs = |p: &str| p.split_whitespace().map(String::from).collect::<Vec<_>>();
    let pkg = match resolve(m, sys) {
        Method::Installed => Some(true),
        Method::Native(p) => Some(sys.has_packages(&pkgs(p))),
        Method::Aur(p) => Some(succeeds(&[strs(&["pacman", "-Q"]), pkgs(p)].concat())),
        Method::Flatpak(f) => Some(succeeds(&strs(&["flatpak", "info", "--user", f]))),
        Method::Script(_) if m.bin.is_some() => Some(false),
        _ => None,
    };
    if m.files.is_empty() {
        return pkg;
    }
    let linked = m.files.iter().all(|f| matches!(link_step(m, f, sys), Step::Link { src, dest } if is_linked(&src, &dest)));
    Some(pkg.unwrap_or(true) && linked)
}

fn link_step(m: &Module, f: &FileLink, sys: &Sys) -> Step {
    Step::Link { src: m.root.join(&f.src), dest: sys.home.join(&f.dest[2..]) }
}

pub fn succeeds(cmd: &[String]) -> bool {
    Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Instalações agrupadas (nativo, aur, flatpak), depois scripts, depois files/services/groups/run na ordem dos módulos.
pub fn plan(mods: &[&Module], sys: &Sys) -> Vec<Step> {
    let (mut native, mut aur, mut flatpak, mut scripts, mut post) = (vec![], vec![], vec![], vec![], vec![]);
    for m in mods {
        if let Some(reason) = skip_reason(m, sys) {
            eprintln!("⚠ {}: {reason}", m.name);
            continue;
        }
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
        post.extend(m.files.iter().map(|f| link_step(m, f, sys)));
        if !m.services.is_empty() && !(sys.has)("systemctl") {
            eprintln!("⚠ {}: sem systemctl, serviços não habilitados", m.name);
        } else {
            post.extend(m.services.iter().map(|s| Step::Cmd(sys.sudo(&["systemctl", "enable", "--now", s]))));
        }
        if let (Some(user), false) = (&sys.user, sys.root) {
            post.extend(m.groups.iter().map(|g| Step::Cmd(sys.sudo(&["usermod", "-aG", g, user]))));
        }
        post.extend(m.run.iter().map(|r| Step::Cmd(strs(&["sh", "-c", r]))));
    }

    let mut steps = vec![];
    if !flatpak.is_empty() && !(sys.has)("flatpak") {
        native.push("flatpak");
    }
    if let (Some(base), false) = (sys.install_base(), native.is_empty()) {
        steps.extend(sys.refresh_cmd().map(Step::Cmd));
        steps.push(Step::Packages { base, pkgs: strs(&native) });
    }
    if let (Some(helper), false) = (sys.aur_helper(), aur.is_empty()) {
        steps.push(Step::Packages { base: strs(&[helper, "-S", "--needed", "--noconfirm"]), pkgs: strs(&aur) });
    }
    if !flatpak.is_empty() {
        steps.push(Step::Cmd(strs(&[
            "flatpak", "remote-add", "--user", "--if-not-exists", "flathub",
            "https://dl.flathub.org/repo/flathub.flatpakrepo",
        ])));
        steps.push(Step::Packages { base: strs(&["flatpak", "install", "--user", "-y", "flathub"]), pkgs: strs(&flatpak) });
    }
    steps.extend(scripts.into_iter().map(|s| Step::Cmd(strs(&["sh", "-c", s]))));
    steps.extend(post);
    steps
}

fn exec(cmd: &[String]) -> bool {
    Command::new(&cmd[0]).args(&cmd[1..]).status().is_ok_and(|s| s.success())
}

pub fn run(steps: &[Step], sys: &Sys, dry_run: bool) -> ExitCode {
    let mut failed = vec![];
    let has_links = steps.iter().any(|s| matches!(s, Step::Link { .. }));
    if has_links && !dry_run && let Err(e) = dotfiles::extract_assets() {
        failed.push(format!("extrair assets: {e}"));
    }
    // As configs linkadas incluem arquivos do tema; sem nenhum escolhido, usa o padrão.
    if has_links && !dry_run && !theme::current_dir().exists() && let Err(e) = theme::apply(theme::DEFAULT, false, false) {
        failed.push(e);
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    let backup = backups_dir().join(now.to_string());
    let mut backed_up = false;
    for step in steps {
        println!("→ {step}");
        if dry_run {
            continue;
        }
        let ok = match step {
            Step::Cmd(cmd) => exec(cmd),
            Step::Packages { base, pkgs } => {
                let all = exec(&[base.clone(), pkgs.clone()].concat());
                if !all && pkgs.len() > 1 {
                    println!("  falhou em grupo; tentando um por um");
                    for p in pkgs {
                        let cmd = [base.clone(), vec![p.clone()]].concat();
                        if !exec(&cmd) {
                            failed.push(cmd.join(" "));
                        }
                    }
                }
                // Com mais de um pacote, as falhas individuais já foram registradas acima.
                all || pkgs.len() > 1
            }
            Step::Link { src, dest } => dotfiles::link(src, dest, &sys.home, &backup)
                .inspect(|moved| backed_up |= moved)
                .inspect_err(|e| eprintln!("  {e}"))
                .is_ok(),
        };
        if !ok {
            failed.push(step.to_string());
        }
    }
    if backed_up {
        println!("\nArquivos substituídos foram para {} (desfaça com `devup restore`)", backup.display());
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
        Sys { pm, root, user: Some("me".into()), has, home: "/h".into(), desktop: None }
    }

    fn as_cmds(steps: Vec<Step>) -> Vec<Vec<String>> {
        steps
            .into_iter()
            .map(|s| match s {
                Step::Cmd(c) => c,
                Step::Packages { base, pkgs } => [base, pkgs].concat(),
                Step::Link { .. } => panic!("link inesperado"),
            })
            .collect()
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
        let cmds = as_cmds(plan(&mods, &sys(Some(Pm::Dnf), false, |b| b == "systemctl")));

        assert_eq!(cmds[0], strs(&["sudo", "dnf", "install", "-y", "git", "btop", "moby-engine", "docker-compose", "flatpak"]));
        assert_eq!(cmds.iter().filter(|c| c.contains(&"dnf".to_string())).count(), 1);
        assert!(cmds.iter().any(|c| c.last().unwrap() == "com.spotify.Client"));
        assert_eq!(cmds[cmds.len() - 3][0], "sh");
        assert_eq!(cmds[cmds.len() - 2], strs(&["sudo", "systemctl", "enable", "--now", "docker"]));
        assert_eq!(cmds[cmds.len() - 1], strs(&["sudo", "usermod", "-aG", "docker", "me"]));

        // lazygit não tem pacote no dnf: avisa e segue
        assert!(!cmds.concat().contains(&"lazygit".to_string()));

        let root = as_cmds(plan(&mods, &sys(Some(Pm::Dnf), true, |b| b == "systemctl")));
        assert_eq!(root[0][0], "dnf");
        assert!(!root.iter().any(|c| c[0] == "usermod"));
    }

    #[test]
    fn links_come_after_install_and_resolve_paths() {
        let cat = Catalog::builtin();
        let tmux = cat.module("tmux").unwrap();
        let steps = plan(&[tmux], &sys(Some(Pm::Apt), false, |_| false));
        assert!(matches!(&steps[1], Step::Packages { base, .. } if base[1] == "apt-get"));
        assert_eq!(steps.last().unwrap(), &Step::Link {
            src: tmux.root.join("tmux/tmux.conf"),
            dest: "/h/.config/tmux/tmux.conf".into(),
        });
    }

    #[test]
    fn desktop_modules_only_apply_on_their_desktop() {
        let cat = Catalog::builtin();
        let gnome = cat.module("gnome").unwrap();
        let mut s = sys(Some(Pm::Apt), false, |_| false);
        assert!(plan(&[gnome], &s).is_empty());
        s.desktop = Some("gnome");
        assert!(plan(&[gnome], &s).iter().any(|st| st.to_string().ends_with("gnome.sh")));
        s.desktop = Some("kde");
        assert!(plan(&[gnome], &s).is_empty());
    }
}
