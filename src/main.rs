mod catalog;
mod config;
mod dotfiles;
mod plan;
mod pm;
mod theme;

use catalog::{CATEGORIES, Catalog};
use clap::{Parser, Subcommand};
use config::{Config, SHELLS};
use dialoguer::{MultiSelect, Select};
use plan::Method;
use pm::Sys;
use std::{
    fs,
    process::{Command, ExitCode},
};

#[derive(Parser)]
#[command(version, about = "Configura seu Linux estilo Omarchy, em qualquer distro")]
struct Cli {
    /// Só mostra os comandos, sem executar
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Escolhe perfis e módulos num menu, salva no devup.toml e aplica
    Init,
    /// Converge a máquina para o devup.toml
    Apply {
        /// Usa um devup.toml de outro lugar (caminho ou URL) como o seu
        #[arg(long)]
        from: Option<String>,
    },
    /// Mostra o que do devup.toml já está instalado e o que falta
    Status,
    /// Adiciona módulos ou perfis ao devup.toml e aplica
    Add {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Tira módulos ou perfis do devup.toml (não desinstala)
    Remove {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Lista módulos e perfis, e como cada módulo seria instalado nesta máquina
    List {
        #[arg(long, value_parser = CATEGORIES)]
        category: Option<String>,
        /// Mostra só os módulos de um perfil
        #[arg(long)]
        profile: Option<String>,
    },
    /// Instala módulos ou perfis pelo nome, sem mexer no devup.toml
    Install {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Seus dotfiles num repo git
    Dotfiles {
        #[command(subcommand)]
        cmd: DotfilesCmd,
    },
    /// Define o shell padrão (instala, aplica a config e faz chsh)
    Shell {
        #[arg(value_parser = SHELLS)]
        name: String,
    },
    /// Temas de cores para terminal, tmux, waybar, Hyprland e wallpaper
    Theme {
        #[command(subcommand)]
        cmd: ThemeCmd,
    },
    /// Devolve os arquivos que o devup substituiu (o backup mais recente, ou o informado)
    Restore { id: Option<String> },
}

#[derive(Subcommand)]
enum ThemeCmd {
    /// Lista os temas (embutidos e de ~/.config/devup/themes)
    List,
    /// Troca o tema, salva no devup.toml e recarrega os apps abertos
    Set { name: String },
}

#[derive(Subcommand)]
enum DotfilesCmd {
    /// Clona o repo, registra no devup.toml e aplica
    Add { url: String },
    /// git pull em todos os repos e reaplica
    Sync,
}

fn list(cat: &Catalog, sys: &Sys, category: Option<String>, profile: Option<String>) -> Result<(), String> {
    let mods = match &profile {
        Some(p) if cat.profiles.contains_key(p) => cat.expand(std::slice::from_ref(p))?,
        Some(p) => return Err(format!("perfil desconhecido: {p}")),
        None => cat.modules.iter().collect(),
    };
    for m in mods.iter().filter(|m| category.as_ref().is_none_or(|c| *c == m.category)) {
        let how = match plan::resolve(m, sys) {
            _ if let Some(reason) = plan::skip_reason(m, sys) => reason,
            Method::Installed => "instalado".into(),
            Method::Native(p) => format!("{}: {p}", sys.pm.unwrap().bin()),
            Method::Aur(p) => format!("aur: {p}"),
            Method::Flatpak(f) => format!("flatpak: {f}"),
            Method::Script(_) => "script".into(),
            Method::Nothing => "config".into(),
            Method::Unavailable => "indisponível".into(),
        };
        println!("{:8} {:13} {:40.40} {}", m.category, m.name, how, m.description);
    }
    if profile.is_none() && category.is_none() {
        println!("\nPerfis:");
        for (name, p) in &cat.profiles {
            println!("  {name:12} {}", p.description);
        }
    }
    Ok(())
}

fn menu(cat: &Catalog, selected: &[String]) -> Result<Vec<String>, String> {
    let names: Vec<&String> = cat.profiles.keys().chain(cat.modules.iter().map(|m| &m.name)).collect();
    let defaults: Vec<bool> = names.iter().map(|n| selected.contains(n)).collect();
    let labels: Vec<String> = cat
        .profiles
        .iter()
        .map(|(n, p)| format!("◆ perfil {n} — {}", p.description))
        .chain(cat.modules.iter().map(|m| format!("[{}] {} — {}", m.category, m.name, m.description)))
        .collect();
    let picked = MultiSelect::new()
        .with_prompt("Espaço seleciona, Enter confirma")
        .items(&labels)
        .defaults(&defaults)
        .interact()
        .map_err(|e| e.to_string())?;
    Ok(picked.into_iter().map(|i| names[i].clone()).collect())
}

fn status(cat: &Catalog, sys: &Sys, cfg: &Config) -> Result<(), String> {
    for m in cfg.resolve(cat)? {
        let mark = match plan::installed(m, sys) {
            _ if plan::skip_reason(m, sys).is_some() => "-",
            Some(true) => "✓",
            Some(false) => "✗",
            None => "?",
        };
        println!("{mark} {:13} {}", m.name, m.description);
    }
    if !cfg.exclude.is_empty() {
        println!("excluídos: {}", cfg.exclude.join(", "));
    }
    Ok(())
}

/// Lê um devup.toml de um caminho ou URL; só vira o do usuário depois de validado.
fn fetch_config(from: &str) -> Result<Config, String> {
    let text = if from.starts_with("https://") || from.starts_with("http://") {
        let out = Command::new("curl").args(["-fsSL", from]).output().map_err(|e| format!("curl: {e}"))?;
        if !out.status.success() {
            return Err(format!("falha ao baixar {from}"));
        }
        String::from_utf8(out.stdout).map_err(|e| format!("{from}: {e}"))?
    } else {
        fs::read_to_string(from).map_err(|e| format!("{from}: {e}"))?
    };
    toml::from_str(&text).map_err(|e| format!("{from}: {e}"))
}

const NO_CONFIG: &str = "nenhum devup.toml ainda; rode `devup init`";

fn save(cfg: &Config, dry_run: bool) -> Result<(), String> {
    if dry_run {
        println!("(dry-run) devup.toml não foi salvo:\n{}", toml::to_string(cfg).map_err(|e| e.to_string())?);
    } else {
        println!("✓ salvo em {}", cfg.save()?.display());
    }
    Ok(())
}

fn clone_missing(urls: &[String], dry_run: bool) -> Result<(), String> {
    for url in urls {
        let dir = dotfiles::repos_dir().join(config::repo_name(url));
        if dir.exists() {
            continue;
        }
        println!("→ git clone {url} {}", dir.display());
        if !dry_run && !Command::new("git").args(["clone", url]).arg(&dir).status().is_ok_and(|s| s.success()) {
            return Err(format!("falha ao clonar {url}"));
        }
    }
    Ok(())
}

/// Clona os repos, valida o config contra o catálogo completo e só então salva (se `persist`) e executa.
fn apply(sys: &Sys, cfg: &Config, dry_run: bool, persist: bool) -> Result<ExitCode, String> {
    clone_missing(&cfg.dotfiles, dry_run)?;
    let cat = Catalog::load(&cfg.dotfiles)?;
    let mods = cfg.resolve(&cat)?;
    if let Some(t) = &cfg.theme {
        theme::render(t)?;
    }
    if persist {
        save(cfg, dry_run)?;
    }
    let code = plan::run(&plan::plan(&mods, sys), sys, dry_run);
    if let Some(t) = &cfg.theme {
        theme::apply(t, true, dry_run)?;
    }
    Ok(code)
}

fn restore(sys: &Sys, id: Option<String>, dry_run: bool) -> Result<(), String> {
    let ids = dotfiles::backups();
    let id = id.or_else(|| ids.last().cloned()).ok_or("nenhum backup")?;
    let dir = dotfiles::backups_dir().join(&id);
    if !ids.contains(&id) {
        return Err(format!("backup {id} não existe; disponíveis: {}", ids.join(", ")));
    }
    if dry_run {
        dotfiles::backup_entries(&dir).iter().for_each(|e| println!("→ restaurar ~/{e}"));
        return Ok(());
    }
    for path in dotfiles::restore(&dir, &sys.home).map_err(|e| e.to_string())? {
        println!("✓ {}", path.display());
    }
    Ok(())
}

fn pick_shell(current: Option<&str>) -> Result<Option<String>, String> {
    let items: Vec<&str> = ["não mexer no shell"].into_iter().chain(SHELLS).collect();
    let default = current.and_then(|c| items.iter().position(|i| *i == c)).unwrap_or(0);
    let i = Select::new()
        .with_prompt("Shell padrão")
        .items(&items)
        .default(default)
        .interact()
        .map_err(|e| e.to_string())?;
    Ok((i > 0).then(|| items[i].to_string()))
}

fn pick_theme(current: Option<&str>) -> Result<Option<String>, String> {
    let names = theme::names();
    let items: Vec<&str> = ["não mexer no tema"].into_iter().chain(names.iter().map(String::as_str)).collect();
    let default = current.and_then(|c| items.iter().position(|i| *i == c)).unwrap_or(0);
    let i = Select::new()
        .with_prompt("Tema")
        .items(&items)
        .default(default)
        .interact()
        .map_err(|e| e.to_string())?;
    Ok((i > 0).then(|| items[i].to_string()))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let sys = Sys::detect();
    if sys.pm.is_none() {
        eprintln!("⚠ Nenhum gerenciador suportado (pacman/apt/dnf/zypper); usando só flatpak e scripts");
    }

    let result = (|| {
        let current = Config::load()?;
        let cat = Catalog::load(current.as_ref().map_or(&[][..], |c| &c.dotfiles))?;
        let dry_run = cli.dry_run;
        match cli.cmd {
            Some(Cmd::List { category, profile }) => list(&cat, &sys, category, profile).map(|_| ExitCode::SUCCESS),
            Some(Cmd::Install { names }) => Ok(plan::run(&plan::plan(&cat.expand(&names)?, &sys), &sys, dry_run)),
            Some(Cmd::Status) => status(&cat, &sys, &current.ok_or(NO_CONFIG)?).map(|_| ExitCode::SUCCESS),
            Some(Cmd::Apply { from: Some(from) }) => apply(&sys, &fetch_config(&from)?, dry_run, true),
            Some(Cmd::Apply { from: None }) => apply(&sys, &current.ok_or(NO_CONFIG)?, dry_run, false),
            Some(Cmd::Add { names }) => {
                let mut cfg = current.unwrap_or_default();
                names.iter().try_for_each(|n| cfg.add(&cat, n))?;
                apply(&sys, &cfg, dry_run, true)
            }
            Some(Cmd::Remove { names }) => {
                let mut cfg = current.ok_or(NO_CONFIG)?;
                names.iter().try_for_each(|n| cfg.remove(&cat, n))?;
                save(&cfg, dry_run)?;
                println!("  (nada foi desinstalado)");
                Ok(ExitCode::SUCCESS)
            }
            Some(Cmd::Dotfiles { cmd: DotfilesCmd::Add { url } }) => {
                let mut cfg = current.unwrap_or_default();
                if !cfg.dotfiles.contains(&url) {
                    cfg.dotfiles.push(url);
                }
                apply(&sys, &cfg, dry_run, true)
            }
            Some(Cmd::Dotfiles { cmd: DotfilesCmd::Sync }) => {
                let cfg = current.ok_or(NO_CONFIG)?;
                for url in &cfg.dotfiles {
                    let dir = dotfiles::repos_dir().join(config::repo_name(url));
                    println!("→ git -C {} pull --ff-only", dir.display());
                    if !dry_run && !Command::new("git").arg("-C").arg(&dir).args(["pull", "--ff-only"]).status().is_ok_and(|s| s.success()) {
                        return Err(format!("falha ao atualizar {url}"));
                    }
                }
                apply(&sys, &cfg, dry_run, false)
            }
            Some(Cmd::Shell { name }) => {
                let cfg = Config { shell: Some(name), ..current.unwrap_or_default() };
                apply(&sys, &cfg, dry_run, true)
            }
            Some(Cmd::Theme { cmd: ThemeCmd::List }) => {
                let current = current.and_then(|c| c.theme);
                for name in theme::names() {
                    let mark = if current.as_ref() == Some(&name) { "●" } else { " " };
                    println!("{mark} {name}");
                }
                Ok(ExitCode::SUCCESS)
            }
            Some(Cmd::Theme { cmd: ThemeCmd::Set { name } }) => {
                theme::render(&name)?;
                let cfg = Config { theme: Some(name.clone()), ..current.unwrap_or_default() };
                save(&cfg, dry_run)?;
                theme::apply(&name, true, dry_run)?;
                Ok(ExitCode::SUCCESS)
            }
            Some(Cmd::Restore { id }) => restore(&sys, id, dry_run).map(|_| ExitCode::SUCCESS),
            Some(Cmd::Init) | None => {
                let old = current.unwrap_or_default();
                let mut cfg = Config::default();
                menu(&cat, &old.names())?.iter().try_for_each(|n| cfg.add(&cat, n))?;
                cfg.exclude = old.exclude;
                cfg.dotfiles = old.dotfiles;
                cfg.shell = pick_shell(old.shell.as_deref())?;
                cfg.theme = pick_theme(old.theme.as_deref())?;
                apply(&sys, &cfg, dry_run, true)
            }
        }
    })();
    result.unwrap_or_else(|e| {
        eprintln!("✗ {e}");
        ExitCode::FAILURE
    })
}
