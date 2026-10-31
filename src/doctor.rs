use crate::catalog::Catalog;
use crate::config::{Config, home, repo_name};
use crate::dotfiles::{backups, is_linked, repos_dir};
use crate::detect::Env;
use crate::pm::{Pm, Sys};
use crate::plan::succeeds;
use crate::{pm::strs, theme};
use console::style;
use serde::Serialize;
use std::{env, process::ExitCode};

#[derive(Serialize)]
pub struct Check {
    /// ok | warn | error
    pub level: &'static str,
    pub message: String,
}

/// Diagnóstico da máquina: ✓ ok, - aviso (o devup contorna), ✗ problema que impede algo.
pub fn run(sys: &Sys, cat: &Catalog, cfg: Option<&Config>, env: &Env, json: bool) -> ExitCode {
    let checks = checks(sys, cat, cfg, env);
    if json {
        println!("{}", serde_json::to_string_pretty(&checks).unwrap());
    } else {
        for c in &checks {
            let mark = match c.level {
                "ok" => style("✓").green(),
                "warn" => style("-").yellow(),
                _ => style("✗").red(),
            };
            println!("{} {}", mark.bold(), c.message);
        }
    }
    if checks.iter().any(|c| c.level == "error") { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn checks(sys: &Sys, cat: &Catalog, cfg: Option<&Config>, env: &Env) -> Vec<Check> {
    let mut out = vec![];
    let mut line = |ok: Option<bool>, message: String| {
        let level = match ok {
            Some(true) => "ok",
            None => "warn",
            Some(false) => "error",
        };
        out.push(Check { level, message });
    };
    let has = sys.has;

    match sys.pm {
        Some(pm) => line(Some(true), format!("gerenciador de pacotes: {}", pm.bin())),
        None => line(Some(false), "nenhum gerenciador suportado (pacman/apt/dnf/zypper/apk/xbps)".into()),
    }
    match sys.desktop {
        Some(d) => line(Some(true), format!("desktop: {d}")),
        None => line(None, "desktop não detectado: módulos de desktop serão pulados".into()),
    }
    match (sys.root, has("sudo")) {
        (true, _) => line(Some(true), "rodando como root".into()),
        (false, true) => line(Some(true), "sudo disponível".into()),
        (false, false) => line(Some(false), "sem sudo: pacotes nativos não podem ser instalados".into()),
    }
    if sys.pm == Some(Pm::Pacman) {
        match sys.aur_helper() {
            Some(h) => line(Some(true), format!("AUR via {h}")),
            None => line(None, "sem paru/yay: módulos do AUR usam flatpak".into()),
        }
    }
    if !has("flatpak") {
        line(None, "flatpak não instalado (o devup instala quando precisar)".into());
    } else if succeeds(&strs(&["sh", "-c", "flatpak remotes --user | grep -q flathub"])) {
        line(Some(true), "flatpak + flathub".into());
    } else {
        line(None, "flathub (usuário) ainda não configurado (o devup adiciona quando precisar)".into());
    }
    if env.gpus.contains(&"NVIDIA") && !env.nvidia_driver && !env.wsl {
        let how = match sys.pm {
            Some(Pm::Pacman) => "pacman -S nvidia-open (ou nvidia-open-dkms)",
            Some(Pm::Apt) => "sudo ubuntu-drivers install (Ubuntu) ou o pacote nvidia-driver (Debian)",
            Some(Pm::Dnf) => "os drivers do RPM Fusion (akmod-nvidia)",
            _ => "o driver proprietário da sua distro",
        };
        line(None, format!("placa NVIDIA sem o driver proprietário carregado; para jogos e CUDA, instale {how}"));
    }
    if env.wsl {
        line(None, "WSL detectado: módulos de desktop e serviços são pulados".into());
    }
    if !has("systemctl") {
        line(None, "sem systemd: `services` dos módulos serão ignorados".into());
    }
    for bin in ["git", "curl"] {
        if !has(bin) {
            line(Some(false), format!("{bin} não encontrado (usado por dotfiles e scripts de instalação)"));
        }
    }
    let local_bin = home().join(".local/bin");
    if !env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d == local_bin)) {
        line(Some(false), "~/.local/bin fora do PATH (scripts como starship e mise instalam lá)".into());
    }

    let Some(cfg) = cfg else {
        line(None, format!("sem {} (rode `devup init`)", Config::path().display()));
        return out;
    };
    let mods = match cfg.resolve(cat) {
        Ok(mods) => {
            line(Some(true), format!("devup.toml válido: {} módulos", mods.len()));
            mods
        }
        Err(e) => {
            line(Some(false), format!("devup.toml: {e}"));
            return out;
        }
    };
    for url in &cfg.dotfiles {
        let cloned = repos_dir().join(repo_name(url)).exists();
        line(Some(cloned), format!("dotfiles {url}{}", if cloned { "" } else { " não clonado (rode `devup apply`)" }));
    }
    for m in &mods {
        for f in &m.files {
            let (src, dest) = (m.root.join(&f.src), sys.home.join(&f.dest[2..]));
            if dest.is_symlink() && !dest.exists() {
                line(Some(false), format!("link quebrado: {} (rode `devup apply`)", f.dest));
            } else if dest.exists() && !is_linked(&src, &dest) {
                line(None, format!("{} não é do devup (`devup apply` substitui com backup)", f.dest));
            }
        }
    }
    for (id, spec) in &cfg.toolchains {
        if !crate::toolchains::is_installed(id) {
            line(Some(false), format!("linguagem {id} ({spec}) no devup.toml, mas não instalada · devup apply"));
        }
    }
    if !cfg.toolchains.is_empty() && !crate::toolchains::on_path() {
        line(Some(false), format!("as linguagens do devup não estão no PATH deste shell · . {}", crate::toolchains::env_sh().display()));
    }
    let theme_name = cfg.theme.as_deref().unwrap_or(theme::DEFAULT);
    match theme::current_dir().exists() {
        true => line(Some(true), format!("tema: {theme_name}")),
        false => line(None, format!("tema {theme_name} ainda não aplicado (rode `devup apply`)")),
    }
    let n = backups().len();
    if n > 0 {
        line(None, format!("{n} backup(s) de arquivos substituídos (`devup restore`)"));
    }
    out
}
