mod catalog;
mod plan;
mod pm;

use catalog::{CATEGORIES, Catalog};
use clap::{Parser, Subcommand};
use dialoguer::MultiSelect;
use plan::Method;
use pm::Sys;
use std::process::ExitCode;

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
    /// Lista módulos e perfis, e como cada módulo seria instalado nesta máquina
    List {
        #[arg(long, value_parser = CATEGORIES)]
        category: Option<String>,
        /// Mostra só os módulos de um perfil
        #[arg(long)]
        profile: Option<String>,
    },
    /// Instala módulos ou perfis pelo nome (dependências entram automaticamente)
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

fn menu(cat: &Catalog) -> Result<Vec<String>, String> {
    let names: Vec<&String> = cat.profiles.keys().chain(cat.modules.iter().map(|m| &m.name)).collect();
    let labels: Vec<String> = cat
        .profiles
        .iter()
        .map(|(n, p)| format!("◆ perfil {n} — {}", p.description))
        .chain(cat.modules.iter().map(|m| format!("[{}] {} — {}", m.category, m.name, m.description)))
        .collect();
    let picked = MultiSelect::new()
        .with_prompt("Espaço seleciona, Enter confirma")
        .items(&labels)
        .interact()
        .map_err(|e| e.to_string())?;
    Ok(picked.into_iter().map(|i| names[i].clone()).collect())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let sys = Sys::detect();
    if sys.pm.is_none() {
        eprintln!("⚠ Nenhum gerenciador suportado (pacman/apt/dnf/zypper); usando só flatpak e scripts");
    }

    let result = Catalog::load().and_then(|cat| {
        let names = match cli.cmd {
            Some(Cmd::List { category, profile }) => return list(&cat, &sys, category, profile).map(|_| ExitCode::SUCCESS),
            Some(Cmd::Install { names }) => names,
            None => menu(&cat)?,
        };
        let mods = cat.expand(&names)?;
        Ok(plan::run(&plan::plan(&mods, &sys), cli.dry_run))
    });
    result.unwrap_or_else(|e| {
        eprintln!("✗ {e}");
        ExitCode::FAILURE
    })
}
