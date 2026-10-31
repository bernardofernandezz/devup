mod pm;

use clap::{Parser, Subcommand};
use dialoguer::MultiSelect;
use pm::{Pm, has_bin, strs};
use serde::Deserialize;
use std::process::{Command, ExitCode};

const CATALOG: &str = include_str!("../apps.toml");
const CATEGORIES: [&str; 3] = ["dev", "games", "general"];

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
    /// Lista o catálogo e como cada app seria instalado nesta máquina
    List {
        #[arg(long, value_parser = CATEGORIES)]
        category: Option<String>,
    },
    /// Instala apps pelo nome
    Install {
        #[arg(required = true)]
        names: Vec<String>,
    },
}

#[derive(Deserialize)]
struct Catalog {
    app: Vec<App>,
}

#[derive(Deserialize)]
struct App {
    name: String,
    category: String,
    description: String,
    pacman: Option<String>,
    apt: Option<String>,
    dnf: Option<String>,
    zypper: Option<String>,
    flatpak: Option<String>,
    script: Option<String>,
    bin: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Method<'a> {
    Installed,
    Native(&'a str),
    Flatpak(&'a str),
    Script(&'a str),
    Unavailable,
}

impl App {
    fn native(&self, pm: Pm) -> Option<&str> {
        match pm {
            Pm::Pacman => &self.pacman,
            Pm::Apt => &self.apt,
            Pm::Dnf => &self.dnf,
            Pm::Zypper => &self.zypper,
        }
        .as_deref()
    }

    fn resolve(&self, pm: Option<Pm>, has: &dyn Fn(&str) -> bool) -> Method<'_> {
        if self.bin.as_deref().is_some_and(has) {
            Method::Installed
        } else if let Some(p) = pm.and_then(|pm| self.native(pm)) {
            Method::Native(p)
        } else if let Some(f) = &self.flatpak {
            Method::Flatpak(f)
        } else if let Some(s) = &self.script {
            Method::Script(s)
        } else {
            Method::Unavailable
        }
    }
}

fn catalog() -> Vec<App> {
    toml::from_str::<Catalog>(CATALOG).expect("apps.toml inválido").app
}

/// Monta os comandos: todos os nativos numa chamada, todos os flatpaks noutra, scripts um a um.
fn plan(apps: &[&App], pm: Option<Pm>, has: &dyn Fn(&str) -> bool) -> Vec<Vec<String>> {
    let (mut native, mut flatpak, mut scripts) = (vec![], vec![], vec![]);
    for app in apps {
        match app.resolve(pm, has) {
            Method::Installed => println!("✓ {} já instalado", app.name),
            Method::Native(p) => native.extend(p.split_whitespace()),
            Method::Flatpak(f) => flatpak.push(f),
            Method::Script(s) => scripts.push(s),
            Method::Unavailable => eprintln!("⚠ {}: sem método de instalação para esta distro", app.name),
        }
    }

    let mut cmds = vec![];
    if !flatpak.is_empty() && !has("flatpak") {
        native.push("flatpak");
    }
    if let (Some(pm), false) = (pm, native.is_empty()) {
        cmds.extend(pm.install_cmds(&native));
    }
    if !flatpak.is_empty() {
        cmds.push(strs(&[
            "flatpak", "remote-add", "--user", "--if-not-exists", "flathub",
            "https://dl.flathub.org/repo/flathub.flatpakrepo",
        ]));
        cmds.push(strs(&["flatpak", "install", "--user", "-y", "flathub"]).into_iter().chain(flatpak.iter().map(|s| s.to_string())).collect());
    }
    cmds.extend(scripts.into_iter().map(|s| strs(&["sh", "-c", s])));
    cmds
}

fn run(cmds: &[Vec<String>], dry_run: bool) -> ExitCode {
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    let apps = catalog();
    let pm = Pm::detect();
    if pm.is_none() {
        eprintln!("⚠ Nenhum gerenciador suportado (pacman/apt/dnf/zypper); usando só flatpak e scripts");
    }

    let selected: Vec<&App> = match cli.cmd {
        Some(Cmd::List { category }) => {
            for a in apps.iter().filter(|a| category.as_ref().is_none_or(|c| *c == a.category)) {
                let how = match a.resolve(pm, &has_bin) {
                    Method::Installed => "instalado".into(),
                    Method::Native(p) => format!("{}: {p}", pm.unwrap().bin()),
                    Method::Flatpak(f) => format!("flatpak: {f}"),
                    Method::Script(_) => "script".into(),
                    Method::Unavailable => "indisponível".into(),
                };
                println!("{:8} {:13} {:40} {}", a.category, a.name, how, a.description);
            }
            return ExitCode::SUCCESS;
        }
        Some(Cmd::Install { names }) => {
            let unknown: Vec<_> = names.iter().filter(|n| !apps.iter().any(|a| &a.name == *n)).collect();
            if !unknown.is_empty() {
                eprintln!("App desconhecido: {unknown:?}. Veja `devup list`.");
                return ExitCode::FAILURE;
            }
            apps.iter().filter(|a| names.contains(&a.name)).collect()
        }
        None => {
            let labels: Vec<_> = apps.iter().map(|a| format!("[{}] {} — {}", a.category, a.name, a.description)).collect();
            let picked = MultiSelect::new()
                .with_prompt("Espaço seleciona, Enter confirma")
                .items(&labels)
                .interact();
            match picked {
                Ok(idx) => idx.into_iter().map(|i| &apps[i]).collect(),
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    run(&plan(&selected, pm, &has_bin), cli.dry_run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_valid() {
        let apps = catalog();
        for a in &apps {
            assert!(CATEGORIES.contains(&a.category.as_str()), "{}: categoria inválida", a.name);
            assert!(
                a.pacman.is_some() || a.apt.is_some() || a.dnf.is_some() || a.zypper.is_some() || a.flatpak.is_some() || a.script.is_some(),
                "{}: sem método", a.name
            );
            assert_eq!(apps.iter().filter(|b| b.name == a.name).count(), 1, "{}: duplicado", a.name);
        }
    }

    #[test]
    fn resolve_order_and_grouping() {
        let apps = catalog();
        let get = |n: &str| apps.iter().find(|a| a.name == n).unwrap();
        let none = |_: &str| false;

        assert_eq!(get("lutris").resolve(Some(Pm::Apt), &none), Method::Native("lutris"));
        assert_eq!(get("lutris").resolve(None, &none), Method::Flatpak("net.lutris.Lutris"));
        assert!(matches!(get("rustup").resolve(Some(Pm::Apt), &none), Method::Script(_)));
        assert_eq!(get("rustup").resolve(Some(Pm::Apt), &|b| b == "rustup"), Method::Installed);

        let cmds = plan(&[get("git"), get("btop"), get("spotify"), get("rustup")], Some(Pm::Dnf), &none);
        assert_eq!(cmds[0], strs(&["sudo", "dnf", "install", "-y", "git", "btop", "flatpak"]));
        assert_eq!(cmds.iter().filter(|c| c[1] == "dnf").count(), 1);
        assert!(cmds.iter().any(|c| c.last().unwrap() == "com.spotify.Client"));
        assert_eq!(cmds.last().unwrap()[0], "sh");
    }
}
