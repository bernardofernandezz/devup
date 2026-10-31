use crate::catalog::Catalog;
use crate::config::{Config, SHELLS};
use crate::plan;
use crate::pm::Sys;
use crate::theme;
use crate::ui::{self, Changes, Error, Ui};
use console::style;
use std::{env, fs, process::Command};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Use {
    Dev,
    Games,
    Daily,
    Server,
}

impl Use {
    const ALL: [Use; 4] = [Use::Dev, Use::Games, Use::Daily, Use::Server];

    fn label(self) -> &'static str {
        match self {
            Use::Dev => "Programar",
            Use::Games => "Jogar",
            Use::Daily => "Apps do dia a dia",
            Use::Server => "Servidor / sem interface gráfica",
        }
    }

    fn profile(self) -> &'static str {
        match self {
            Use::Dev => "dev",
            Use::Games => "gaming",
            Use::Daily => "essentials",
            Use::Server => "server",
        }
    }
}

/// (rótulo, módulo, binário que mostra que a linguagem já é usada)
const LANGS: [(&str, &str, &str); 4] =
    [("JavaScript / Node", "nodejs", "node"), ("Python", "python", "pip3"), ("Rust", "rustup", "cargo"), ("Go", "go", "go")];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Editor {
    Both,
    Neovim,
    VsCode,
    Neither,
}

const EDITORS: [(Editor, &str); 4] = [
    (Editor::Both, "Neovim + VS Code"),
    (Editor::Neovim, "Neovim"),
    (Editor::VsCode, "VS Code"),
    (Editor::Neither, "nenhum, já tenho o meu"),
];

/// O que o devup percebeu na máquina, sem perguntar nada.
#[derive(Default, Debug)]
pub struct Signals {
    pub desktop: bool,
    /// Módulos dos perfis de uso que já estão instalados.
    pub installed: Vec<String>,
    /// Binários de linguagens e editores encontrados no PATH.
    pub bins: Vec<String>,
    /// Shell de login, só o nome (bash, zsh…).
    pub shell: Option<String>,
    pub git_identity: bool,
}

#[derive(Debug, PartialEq)]
pub struct Answers {
    pub uses: Vec<Use>,
    /// Módulos de linguagem escolhidos.
    pub langs: Vec<String>,
    pub editor: Editor,
    pub shell: Option<String>,
    pub theme: Option<String>,
}

pub struct Outcome {
    pub cfg: Config,
    /// Nome e e-mail para o git, se o usuário informou.
    pub git: Option<(String, String)>,
}

pub fn detect(cat: &Catalog, sys: &Sys) -> Signals {
    let profiles: Vec<String> = Use::ALL.iter().map(|u| u.profile().to_string()).collect();
    let installed = cat
        .expand(&profiles)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| plan::installed_pkg(m, sys) == Some(true))
        .map(|m| m.name.clone())
        .collect();
    let bins = ["node", "pip3", "cargo", "go", "code", "nvim"].into_iter().filter(|b| (sys.has)(b)).map(String::from).collect();
    let git_identity = Command::new("git")
        .args(["config", "--global", "user.name"])
        .output()
        .is_ok_and(|o| !o.stdout.trim_ascii().is_empty());
    Signals { desktop: sys.desktop.is_some(), installed, bins, shell: login_shell(sys), git_identity }
}

pub fn login_shell(sys: &Sys) -> Option<String> {
    let from_passwd = sys.user.as_ref().and_then(|u| Command::new("getent").args(["passwd", u]).output().ok()).and_then(|o| {
        let line = String::from_utf8(o.stdout).ok()?;
        line.trim().rsplit(':').next().filter(|s| !s.is_empty()).map(String::from)
    });
    let path = from_passwd.or_else(|| env::var("SHELL").ok())?;
    path.rsplit('/').next().map(String::from)
}

/// Por que um uso foi recomendado; vazio quando não há sinal dele.
fn hint(u: Use, sig: &Signals, cat: &Catalog) -> Option<String> {
    let installed: Vec<&str> = cat
        .expand(&[u.profile().to_string()])
        .unwrap_or_default()
        .iter()
        .filter(|m| sig.installed.contains(&m.name) && m.name != "discord") // discord não indica que a pessoa joga
        .map(|m| m.name.as_str())
        .collect();
    let langs: Vec<&str> = LANGS.iter().filter(|l| sig.bins.iter().any(|b| b == l.2)).map(|l| l.0).collect();
    match u {
        Use::Dev if installed.len() >= 2 || !langs.is_empty() => {
            let found: Vec<&str> = installed.iter().copied().chain(langs).take(3).collect();
            Some(format!("{} já por aqui", found.join(", ")))
        }
        Use::Games if !installed.is_empty() => Some(format!("{} já instalado", installed.iter().take(3).copied().collect::<Vec<_>>().join(", "))),
        Use::Daily if sig.desktop => Some("desktop detectado".into()),
        Use::Server if !sig.desktop => Some("nenhum desktop detectado".into()),
        _ => None,
    }
}

pub fn recommend(sig: &Signals, cat: &Catalog, current: Option<&Config>) -> Answers {
    if let Some(c) = current {
        let ex = |m: &str| c.exclude.iter().any(|e| e == m);
        return Answers {
            uses: Use::ALL.into_iter().filter(|u| c.profiles.iter().any(|p| p == u.profile())).collect(),
            langs: LANGS.iter().map(|l| l.1.to_string()).filter(|m| c.modules.contains(m)).collect(),
            editor: match (ex("neovim"), ex("vscode")) {
                (true, true) => Editor::Neither,
                (false, true) => Editor::Neovim,
                (true, false) => Editor::VsCode,
                (false, false) => Editor::Both,
            },
            shell: c.shell.clone(),
            theme: c.theme.clone(),
        };
    }
    let mut uses: Vec<Use> = Use::ALL.into_iter().filter(|u| hint(*u, sig, cat).is_some()).collect();
    if !uses.contains(&Use::Dev) && uses.len() < 2 {
        uses.insert(0, Use::Dev); // devup é feito para quem programa; sem sinal nenhum, é o palpite mais útil
    }
    let has = |b: &str| sig.bins.iter().any(|x| x == b);
    Answers {
        uses,
        langs: LANGS.iter().filter(|l| has(l.2)).map(|l| l.1.to_string()).collect(),
        editor: match (has("nvim"), has("code")) {
            (true, false) => Editor::Neovim,
            (false, true) => Editor::VsCode,
            _ => Editor::Both,
        },
        shell: Some(match sig.shell.as_deref() {
            Some(s) if s != "bash" && SHELLS.contains(&s) => s.to_string(),
            _ => "zsh".into(),
        }),
        theme: Some(theme::DEFAULT.into()),
    }
}

/// Monta o devup.toml a partir das respostas, preservando o que o assistente não controla.
pub fn to_config(a: &Answers, base: &Config, desktop: bool) -> Config {
    let use_profiles: Vec<&str> = Use::ALL.iter().map(|u| u.profile()).chain(["desktop"]).collect();
    let lang_mods: Vec<&str> = LANGS.iter().map(|l| l.1).collect();
    let mut cfg = Config { shell: a.shell.clone(), theme: a.theme.clone(), ..base.clone() };
    cfg.profiles.retain(|p| !use_profiles.contains(&p.as_str()));
    cfg.profiles.extend(a.uses.iter().map(|u| u.profile().to_string()));
    if desktop && a.uses.iter().any(|u| *u != Use::Server) {
        cfg.profiles.push("desktop".into());
    }
    cfg.modules.retain(|m| !lang_mods.contains(&m.as_str()));
    cfg.exclude.retain(|m| m != "neovim" && m != "vscode");
    if a.uses.contains(&Use::Dev) {
        cfg.modules.extend(a.langs.iter().cloned());
        cfg.exclude.extend(
            match a.editor {
                Editor::Both => &[][..],
                Editor::Neovim => &["vscode"],
                Editor::VsCode => &["neovim"],
                Editor::Neither => &["neovim", "vscode"],
            }
            .iter()
            .map(|s| s.to_string()),
        );
    }
    cfg
}

// ---------- interface ----------

fn strings<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn describe(c: &Config) -> String {
    let profiles = if c.profiles.is_empty() { "sem perfis".into() } else { c.profiles.join(" + ") };
    [Some(profiles), c.shell.clone(), c.theme.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

fn distro() -> Option<String> {
    let text = fs::read_to_string("/etc/os-release").ok()?;
    let line = text.lines().find(|l| l.starts_with("PRETTY_NAME="))?;
    Some(line["PRETTY_NAME=".len()..].trim_matches('"').to_string())
}

fn header(sys: &Sys, sig: &Signals) {
    let parts = [
        distro(),
        sys.pm.map(|p| p.bin().to_string()),
        Some(sys.desktop.unwrap_or("sem desktop").to_string()),
        sig.shell.as_ref().map(|s| format!("shell {s}")),
    ];
    let line = parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
    println!("\n{}  {}\n", style("devup").bold().magenta(), style(line).dim());
}

fn ask_uses(ui: &Ui, a: &mut Answers, sig: &Signals, cat: &Catalog) -> Result<(), Error> {
    let items: Vec<String> = Use::ALL
        .iter()
        .map(|u| match hint(*u, sig, cat) {
            Some(h) => format!("{:<34}{}", u.label(), style(h).dim()),
            None => u.label().to_string(),
        })
        .collect();
    let defaults: Vec<bool> = Use::ALL.iter().map(|u| a.uses.contains(u)).collect();
    let picked = ui.multi("Para que você usa este computador? (espaço marca)", &items, &|i| Use::ALL[i].label().into(), &defaults)?;
    a.uses = picked.into_iter().map(|i| Use::ALL[i]).collect();
    Ok(())
}

fn ask_dev(ui: &Ui, a: &mut Answers, sig: &Signals) -> Result<(), Error> {
    if !a.uses.contains(&Use::Dev) {
        return Ok(());
    }
    let items: Vec<String> = LANGS
        .iter()
        .map(|l| match sig.bins.iter().any(|b| b == l.2) {
            true => format!("{:<22}{}", l.0, style(format!("{} encontrado", l.2)).dim()),
            false => l.0.to_string(),
        })
        .collect();
    let defaults: Vec<bool> = LANGS.iter().map(|l| a.langs.iter().any(|m| m == l.1)).collect();
    a.langs = ui.multi("Quais linguagens?", &items, &|i| LANGS[i].0.into(), &defaults)?.into_iter().map(|i| LANGS[i].1.to_string()).collect();

    let items: Vec<String> = EDITORS.iter().map(|e| e.1.to_string()).collect();
    let default = EDITORS.iter().position(|e| e.0 == a.editor).unwrap_or(0);
    a.editor = EDITORS[ui.select("Editor?", &items, &|i| EDITORS[i].1.into(), default)?].0;
    Ok(())
}

pub fn ask_shell(ui: &Ui, a: &mut Answers, sig: &Signals) -> Result<(), Error> {
    let about = |s: &str| match s {
        "zsh" => "autosuggestions, syntax highlighting, starship",
        "fish" => "sugestões e cores sem configurar nada",
        _ => "o clássico, com completion e starship",
    };
    let mut items: Vec<String> = SHELLS
        .iter()
        .map(|s| {
            let mut tags = vec![];
            if sig.shell.as_deref() == Some(s) {
                tags.push("atual");
            }
            if a.shell.as_deref() == Some(s) {
                tags.push("recomendado");
            }
            let tags = if tags.is_empty() { String::new() } else { format!("  ({})", tags.join(", ")) };
            format!("{:<6}{}{}", s, style(about(s)).dim(), style(tags).green())
        })
        .collect();
    items.push("não mexer no shell".into());
    let default = a.shell.as_ref().and_then(|s| SHELLS.iter().position(|x| x == s)).unwrap_or(SHELLS.len());
    let i = ui.select("Qual shell?", &items, &|i| SHELLS.get(i).unwrap_or(&"não mexer").to_string(), default)?;
    a.shell = SHELLS.get(i).map(|s| s.to_string());
    Ok(())
}

pub fn swatch(name: &str) -> String {
    let Ok(p) = theme::palette(name) else { return String::new() };
    ["background", "accent", "color1", "color2", "color3", "color4", "color5", "color6"]
        .iter()
        .filter_map(|k| p.get(&format!("{k}_raw")))
        .filter_map(|hex| {
            let n = u32::from_str_radix(hex, 16).ok()?;
            Some(format!("\x1b[38;2;{};{};{}m██\x1b[0m", n >> 16, (n >> 8) & 0xff, n & 0xff))
        })
        .collect()
}

pub fn ask_theme(ui: &Ui, a: &mut Answers) -> Result<(), Error> {
    let names = theme::names();
    let mut items: Vec<String> = names.iter().map(|n| format!("{}  {n}", swatch(n))).collect();
    items.push("não mexer no tema".into());
    let default = a.theme.as_ref().and_then(|t| names.iter().position(|n| n == t)).unwrap_or(names.len());
    let i = ui.select("Qual tema?", &items, &|i| names.get(i).cloned().unwrap_or("não mexer".into()), default)?;
    a.theme = names.get(i).cloned();
    Ok(())
}

fn ask_git(ui: &Ui, a: &Answers, sig: &Signals) -> Result<Option<(String, String)>, Error> {
    if !a.uses.contains(&Use::Dev) || sig.git_identity {
        return Ok(None);
    }
    let name = ui.input("Seu nome para os commits do git (Enter pula)")?;
    if name.is_empty() {
        return Ok(None);
    }
    let email = ui.input("E-mail do git")?;
    Ok((!email.is_empty()).then_some((name, email)))
}

/// Lista completa, já marcada; o que sair de um perfil vira `exclude`, o que entrar a mais vira `modules`.
fn adjust(ui: &Ui, cat: &Catalog, cfg: Config) -> Result<Config, Error> {
    let chosen: Vec<String> = cfg.resolve(cat)?.iter().map(|m| m.name.clone()).collect();
    let items: Vec<String> = cat
        .modules
        .iter()
        .map(|m| format!("{:<13}{:<9}{}", m.name, style(&m.category).dim(), style(&m.description).dim()))
        .collect();
    let defaults: Vec<bool> = cat.modules.iter().map(|m| chosen.contains(&m.name)).collect();
    let picked: Vec<String> =
        ui.multi("Ajuste os apps", &items, &|i| cat.modules[i].name.clone(), &defaults)?.into_iter().map(|i| cat.modules[i].name.clone()).collect();
    let from_profiles: Vec<String> = Config { modules: vec![], exclude: vec![], ..cfg.clone() }
        .resolve(cat)?
        .iter()
        .map(|m| m.name.clone())
        .collect();
    Ok(Config {
        modules: picked.iter().filter(|n| !from_profiles.contains(n)).cloned().collect(),
        exclude: from_profiles.iter().filter(|n| !picked.contains(n)).cloned().collect(),
        ..cfg
    })
}

/// Linhas de shell e tema do plano, comparando com o que a máquina usa hoje.
pub fn changes(cfg: &Config, login_shell: Option<&str>) -> Changes {
    let shell = match (login_shell, cfg.shell.as_deref()) {
        (_, None) => None,
        (Some(old), Some(new)) if old != new => Some(format!("{old} → {new}")),
        (None, Some(new)) => Some(new.to_string()),
        (Some(_), Some(_)) => None,
    };
    let theme = cfg.theme.clone().filter(|t| theme::current().as_ref() != Some(t));
    Changes { shell, theme }
}

/// O assistente do `devup init`. Retorna None se o usuário cancelar.
pub fn run(cat: &Catalog, sys: &Sys, current: Option<Config>, yes: bool) -> Result<Option<Outcome>, Error> {
    if !yes && !ui::interactive() {
        return Err(Error::usage("o init é interativo; use `devup init -y` para aceitar as recomendações, ou `devup apply`"));
    }
    eprintln!("{}", style("Analisando sua máquina…").dim());
    let sig = detect(cat, sys);
    header(sys, &sig);
    let base = current.clone().unwrap_or_default();
    let mut ans = recommend(&sig, cat, current.as_ref());
    let ui = Ui::new();
    let mut git = None;

    let mut cfg = match (&current, yes) {
        (Some(c), true) => c.clone(),
        (None, true) => to_config(&ans, &base, sig.desktop),
        (current, false) => {
            let step = match current {
                Some(c) => {
                    let items = strings(["Aplicar de novo", "Mudar o uso do computador", "Trocar o shell", "Trocar o tema", "Começar do zero"]);
                    ui.select(&format!("Você já tem um setup: {}. O que quer fazer?", describe(c)), &items, &|i| items[i].clone(), 0)?
                }
                None => 4,
            };
            match step {
                0 => base.clone(),
                1 => {
                    ask_uses(&ui, &mut ans, &sig, cat)?;
                    ask_dev(&ui, &mut ans, &sig)?;
                    to_config(&ans, &base, sig.desktop)
                }
                2 => {
                    ask_shell(&ui, &mut ans, &sig)?;
                    Config { shell: ans.shell.clone(), ..base.clone() }
                }
                3 => {
                    ask_theme(&ui, &mut ans)?;
                    Config { theme: ans.theme.clone(), ..base.clone() }
                }
                _ => {
                    // Do zero: recomendações da máquina, mantendo só os repos de dotfiles.
                    ans = recommend(&sig, cat, None);
                    ask_uses(&ui, &mut ans, &sig, cat)?;
                    ask_dev(&ui, &mut ans, &sig)?;
                    ask_shell(&ui, &mut ans, &sig)?;
                    ask_theme(&ui, &mut ans)?;
                    git = ask_git(&ui, &ans, &sig)?;
                    to_config(&ans, &Config { dotfiles: base.dotfiles.clone(), ..Default::default() }, sig.desktop)
                }
            }
        }
    };

    loop {
        let mods = cfg.resolve(cat)?;
        let summary = ui::summarize(&mods, sys, &|m| plan::installed_pkg(m, sys), &|m| ui::system_done(m, sys));
        ui::print_summary(&summary, &changes(&cfg, sig.shell.as_deref()));
        if yes {
            return Ok(Some(Outcome { cfg, git }));
        }
        let items = strings(["Aplicar", "Ajustar apps", "Cancelar"]);
        match ui.select("Tudo certo?", &items, &|i| items[i].clone(), 0)? {
            0 => return Ok(Some(Outcome { cfg, git })),
            1 => cfg = adjust(&ui, cat, cfg)?,
            _ => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pm::Pm;

    fn sig(desktop: bool, installed: &[&str], bins: &[&str], shell: &str) -> Signals {
        Signals {
            desktop,
            installed: installed.iter().map(|s| s.to_string()).collect(),
            bins: bins.iter().map(|s| s.to_string()).collect(),
            shell: Some(shell.into()),
            git_identity: false,
        }
    }

    #[test]
    fn recommends_from_what_the_machine_has() {
        let cat = Catalog::builtin();
        let r = recommend(&sig(true, &[], &[], "bash"), &cat, None);
        assert_eq!(r.uses, [Use::Dev, Use::Daily]);
        assert_eq!(r.shell.as_deref(), Some("zsh"));

        let r = recommend(&sig(true, &["steam", "lutris"], &["cargo", "code"], "fish"), &cat, None);
        assert_eq!(r.uses, [Use::Dev, Use::Games, Use::Daily]);
        assert_eq!(r.langs, ["rustup"]);
        assert_eq!(r.editor, Editor::VsCode);
        assert_eq!(r.shell.as_deref(), Some("fish"));

        let r = recommend(&sig(false, &["git", "docker"], &[], "bash"), &cat, None);
        assert_eq!(r.uses, [Use::Dev, Use::Server]);
    }

    #[test]
    fn recommends_from_existing_config() {
        let cat = Catalog::builtin();
        let c = Config { profiles: vec!["gaming".into()], modules: vec!["go".into()], exclude: vec!["vscode".into()], shell: Some("fish".into()), ..Default::default() };
        let r = recommend(&sig(true, &[], &[], "bash"), &cat, Some(&c));
        assert_eq!((r.uses, r.langs, r.editor, r.shell.as_deref()), (vec![Use::Games], vec!["go".to_string()], Editor::Neovim, Some("fish")));
    }

    #[test]
    fn answers_become_config_keeping_custom_entries() {
        let base = Config {
            profiles: vec!["meu-setup".into(), "gaming".into()],
            modules: vec!["obsidian".into(), "nodejs".into()],
            dotfiles: vec!["https://x/dots".into()],
            ..Default::default()
        };
        let a = Answers {
            uses: vec![Use::Dev, Use::Daily],
            langs: vec!["python".into()],
            editor: Editor::Neovim,
            shell: Some("zsh".into()),
            theme: Some("nord".into()),
        };
        let c = to_config(&a, &base, true);
        assert_eq!(c.profiles, ["meu-setup", "dev", "essentials", "desktop"]);
        assert_eq!(c.modules, ["obsidian", "python"]);
        assert_eq!(c.exclude, ["vscode"]);
        assert_eq!((c.shell.as_deref(), c.theme.as_deref(), c.dotfiles.len()), (Some("zsh"), Some("nord"), 1));
        Catalog::builtin().module("python").unwrap();

        let server_only = Answers { uses: vec![Use::Server], ..a };
        let c = to_config(&server_only, &Config::default(), true);
        assert_eq!(c.profiles, ["server"]);
        assert!(c.exclude.is_empty() && c.modules.is_empty());
    }
}
