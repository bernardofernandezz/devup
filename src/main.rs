mod catalog;
mod config;
mod plan;
mod pm;

use catalog::{CATEGORIES, Catalog};
use clap::{Parser, Subcommand};
use config::Config;
use dialoguer::MultiSelect;
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
}

fn list(cat: &Catalog, sys: &Sys, category: Option<String>, profile: Option<String>) -> Result<(), String> {
    let mods = match &profile {
        Some(p) if cat.profiles.contains_key(p) => cat.expand(std::slice::from_ref(p))?,
        Some(p) => return Err(format!("perfil desconhecido: {p}")),
        None => cat.modules.iter().collect(),
    };
    for m in mods.iter().filter(|m| category.as_ref().is_none_or(|c| *c == m.category)) {
        let how = match plan::resolve(m, sys) {
            Method::Installed => "instalado".into(),
            Method::Native(p) => format!("{}: {p}", sys.pm.unwrap().bin()),
            Method::Aur(p) => format!("aur: {p}"),
            Method::Flatpak(f) => format!("flatpak: {f}"),
            Method::Script(_) => "script".into(),
            Method::Nothing => "config".into(),
            Method::Unavailable => "indisponível".into(),
        };
        println!("{:8} {:13} {:40} {}", m.category, m.name, how, m.description);
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

fn load_config() -> Result<Config, String> {
    Config::load()?.ok_or_else(|| "nenhum devup.toml ainda; rode `devup init`".into())
}

fn save(cfg: &Config, dry_run: bool) -> Result<(), String> {
    if dry_run {
        println!("(dry-run) devup.toml não foi salvo:\n{}", toml::to_string(cfg).map_err(|e| e.to_string())?);
    } else {
        println!("✓ salvo em {}", cfg.save()?.display());
    }
    Ok(())
}

fn apply(cat: &Catalog, sys: &Sys, cfg: &Config, dry_run: bool) -> Result<ExitCode, String> {
    Ok(plan::run(&plan::plan(&cfg.resolve(cat)?, sys), dry_run))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let sys = Sys::detect();
    if sys.pm.is_none() {
        eprintln!("⚠ Nenhum gerenciador suportado (pacman/apt/dnf/zypper); usando só flatpak e scripts");
    }

    let result = Catalog::load().and_then(|cat| match cli.cmd {
        Some(Cmd::List { category, profile }) => list(&cat, &sys, category, profile).map(|_| ExitCode::SUCCESS),
        Some(Cmd::Install { names }) => Ok(plan::run(&plan::plan(&cat.expand(&names)?, &sys), cli.dry_run)),
        Some(Cmd::Status) => status(&cat, &sys, &load_config()?).map(|_| ExitCode::SUCCESS),
        Some(Cmd::Apply { from: Some(from) }) => {
            let cfg = fetch_config(&from)?;
            cfg.resolve(&cat)?;
            save(&cfg, cli.dry_run)?;
            apply(&cat, &sys, &cfg, cli.dry_run)
        }
        Some(Cmd::Apply { from: None }) => apply(&cat, &sys, &load_config()?, cli.dry_run),
        Some(Cmd::Add { names }) => {
            let mut cfg = Config::load()?.unwrap_or_default();
            names.iter().try_for_each(|n| cfg.add(&cat, n))?;
            save(&cfg, cli.dry_run)?;
            apply(&cat, &sys, &cfg, cli.dry_run)
        }
        Some(Cmd::Remove { names }) => {
            let mut cfg = load_config()?;
            names.iter().try_for_each(|n| cfg.remove(&cat, n))?;
            save(&cfg, cli.dry_run)?;
            println!("  (nada foi desinstalado)");
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Init) | None => {
            let old = Config::load()?.unwrap_or_default();
            let mut cfg = Config::default();
            menu(&cat, &old.names())?.iter().try_for_each(|n| cfg.add(&cat, n))?;
            cfg.exclude = old.exclude;
            save(&cfg, cli.dry_run)?;
            apply(&cat, &sys, &cfg, cli.dry_run)
        }
    });
    result.unwrap_or_else(|e| {
        eprintln!("✗ {e}");
        ExitCode::FAILURE
    })
}
