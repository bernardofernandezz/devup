mod catalog;
mod config;
mod doctor;
mod dotfiles;
mod plan;
mod pm;
mod theme;
mod ui;
mod update;
mod wizard;

use catalog::{CATEGORIES, Catalog, Module};
use clap::{Arg, ArgAction, CommandFactory, FromArgMatches, Parser, Subcommand};
use config::{Config, SHELLS};
use console::style;
use plan::Method;
use pm::Sys;
use serde::Serialize;
use std::{
    fs, io,
    process::{Command, ExitCode},
};
use ui::{Decision, Error};

#[derive(Parser)]
#[command(name = "devup", version)]
struct Cli {
    /// Mostra o que seria feito, sem executar nem salvar nada
    #[arg(long, global = true)]
    dry_run: bool,
    /// Aplica sem perguntar (necessário em scripts)
    #[arg(short = 'y', long, global = true)]
    yes: bool,
    /// Mostra a saída completa dos comandos
    #[arg(short, long, global = true, conflicts_with = "quiet")]
    verbose: bool,
    /// Só erros e o resumo final
    #[arg(short, long, global = true)]
    quiet: bool,
    /// Cores: auto, always ou never (respeita NO_COLOR)
    #[arg(long, global = true, value_name = "QUANDO", default_value = "auto", value_parser = ["auto", "always", "never"], hide_default_value = true)]
    color: String,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Entende a máquina, recomenda um setup, mostra o plano e aplica
    #[command(after_help = "Exemplos:\n  devup init        perguntas guiadas\n  devup init -y     aceita as recomendações sem perguntar")]
    Init,
    /// Deixa a máquina igual ao devup.toml
    #[command(after_help = "Exemplos:\n  devup apply\n  devup apply --dry-run\n  devup apply --from https://raw.githubusercontent.com/<você>/dotfiles/main/devup.toml")]
    Apply {
        /// Usa um devup.toml de outro lugar (validado antes de substituir o seu)
        #[arg(long, value_name = "ARQUIVO|URL")]
        from: Option<String>,
    },
    /// O que do devup.toml já está instalado e o que falta
    Status {
        /// Saída em JSON
        #[arg(long)]
        json: bool,
    },
    /// Adiciona apps ou perfis ao devup.toml e aplica
    #[command(after_help = "Exemplos:\n  devup add gaming obsidian\n  devup add          abre a lista para escolher")]
    Add {
        /// Apps ou perfis; sem nada, abre a lista
        #[arg(value_name = "NOME")]
        names: Vec<String>,
    },
    /// Tira apps ou perfis do devup.toml (não desinstala)
    #[command(after_help = "Exemplos:\n  devup remove vscode\n  devup remove       abre a lista do que está no devup.toml")]
    Remove {
        /// Apps ou perfis; sem nada, abre a lista
        #[arg(value_name = "NOME")]
        names: Vec<String>,
    },
    /// Apps e perfis disponíveis, e como cada app seria instalado aqui
    #[command(after_help = "Exemplos:\n  devup list\n  devup list --category games\n  devup list --profile dev --json")]
    List {
        /// Só uma categoria
        #[arg(long, value_parser = CATEGORIES, value_name = "CATEGORIA")]
        category: Option<String>,
        /// Só os apps de um perfil
        #[arg(long, value_name = "PERFIL")]
        profile: Option<String>,
        /// Saída em JSON
        #[arg(long)]
        json: bool,
    },
    /// Instala apps ou perfis sem mexer no devup.toml
    #[command(after_help = "Exemplos:\n  devup install docker btop\n  devup install      abre a lista para escolher")]
    Install {
        /// Apps ou perfis; sem nada, abre a lista
        #[arg(value_name = "NOME")]
        names: Vec<String>,
    },
    /// Usa os seus dotfiles de um repo git
    Dotfiles {
        #[command(subcommand)]
        cmd: DotfilesCmd,
    },
    /// Define o shell padrão: instala, aplica a config e troca o shell de login
    #[command(after_help = "Exemplos:\n  devup shell zsh\n  devup shell        abre a escolha")]
    Shell {
        /// zsh, fish ou bash; sem nada, abre a escolha
        #[arg(value_parser = SHELLS, value_name = "SHELL")]
        name: Option<String>,
    },
    /// Lista e troca o tema de cores
    Theme {
        #[command(subcommand)]
        cmd: ThemeCmd,
    },
    /// Diz o que está ok na máquina e o que corrigir
    Doctor {
        /// Saída em JSON
        #[arg(long)]
        json: bool,
    },
    /// Atualiza o devup para a última versão publicada
    Update,
    /// Devolve os arquivos que o devup substituiu
    #[command(after_help = "Sem ID, abre a lista de backups (ou usa o mais recente, sem terminal).")]
    Restore {
        /// Qual backup (padrão: escolher na lista)
        #[arg(value_name = "ID")]
        id: Option<String>,
    },
    /// Gera o autocomplete para o seu shell
    #[command(after_help = "Exemplos:\n  source <(devup completions zsh)\n  devup completions fish > ~/.config/fish/completions/devup.fish\n\nAs configs de shell do devup já carregam isso sozinhas.")]
    Completions {
        #[arg(value_enum, value_name = "SHELL")]
        shell: clap_complete::Shell,
    },
    #[command(name = "__update-check", hide = true)]
    UpdateCheck,
}

#[derive(Subcommand)]
enum ThemeCmd {
    /// Lista os temas (embutidos e de ~/.config/devup/themes)
    List {
        /// Saída em JSON
        #[arg(long)]
        json: bool,
    },
    /// Troca o tema e recarrega o que estiver aberto
    #[command(after_help = "Exemplos:\n  devup theme set nord\n  devup theme set    abre a escolha com as cores")]
    Set {
        /// Nome do tema; sem nada, abre a escolha
        #[arg(value_name = "TEMA")]
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum DotfilesCmd {
    /// Clona o repo, registra no devup.toml e aplica
    Add {
        /// URL do repo git
        #[arg(value_name = "URL")]
        url: String,
    },
    /// git pull em todos os repos e reaplica
    Sync,
}

// ---------- ajuda em português ----------

fn main_help() -> String {
    let h = |s: &str| style(s).bold().underlined().to_string();
    let c = |s: &str| style(format!("{s:<18}")).cyan().to_string();
    let groups: [(&str, &[(&str, &str)]); 5] = [
        ("Começar", &[("init", "Entende a máquina, recomenda um setup, mostra o plano e aplica"), ("doctor", "Diz o que está ok e o que corrigir")]),
        ("Seu setup (devup.toml)", &[
            ("apply", "Deixa a máquina igual ao devup.toml"),
            ("add", "Adiciona apps ou perfis e aplica"),
            ("remove", "Tira apps ou perfis (não desinstala)"),
            ("status", "O que já está instalado e o que falta"),
        ]),
        ("Personalizar", &[
            ("shell", "Define o shell padrão (zsh, fish, bash)"),
            ("theme", "Lista e troca o tema de cores"),
            ("dotfiles", "Usa os seus dotfiles de um repo git"),
        ]),
        ("Catálogo", &[("list", "Apps e perfis, e como cada um seria instalado aqui"), ("install", "Instala sem mexer no devup.toml")]),
        ("Manutenção", &[
            ("restore", "Devolve arquivos que o devup substituiu"),
            ("update", "Atualiza o devup"),
            ("completions", "Autocomplete para bash, zsh e fish"),
        ]),
    ];
    let mut out = format!(
        "{} {} · seu Linux inteiro em um comando\n\n{} devup [COMANDO] [OPÇÕES]\n",
        style("devup").magenta().bold(),
        env!("CARGO_PKG_VERSION"),
        h("Uso:")
    );
    for (title, cmds) in groups {
        out += &format!("\n{}\n", h(title));
        for (name, about) in cmds {
            out += &format!("  {}{about}\n", c(name));
        }
    }
    out += &format!("\n{}\n", h("Opções"));
    for (flag, about) in [
        ("-y, --yes", "Aplica sem perguntar (necessário em scripts)"),
        ("    --dry-run", "Mostra o que seria feito, sem executar nem salvar"),
        ("-v, --verbose", "Mostra a saída completa dos comandos"),
        ("-q, --quiet", "Só erros e o resumo final"),
        ("    --color", "auto, always ou never (respeita NO_COLOR)"),
        ("-h, --help", "Ajuda (também: devup <comando> --help)"),
        ("-V, --version", "Versão"),
    ] {
        out += &format!("  {}{about}\n", c(flag));
    }
    out += &format!(
        "\n{}\n  devup                         comece por aqui\n  devup add gaming obsidian\n  devup theme set nord\n  devup apply --from <url do seu devup.toml>\n\nDocumentação: https://github.com/bernardofernandezz/devup\n",
        h("Exemplos")
    );
    out
}

/// Ajuda e cabeçalhos em português em todos os subcomandos.
fn localize(cmd: clap::Command) -> clap::Command {
    cmd.disable_help_flag(true)
        .disable_help_subcommand(true)
        .subcommand_help_heading("Comandos")
        .help_template("{about-with-newline}\n{usage-heading} {usage}\n\n{all-args}{after-help}")
        .arg(Arg::new("help").short('h').long("help").action(ArgAction::Help).help("Mostra esta ajuda"))
        .mut_args(|a| if a.is_positional() { a.help_heading("Argumentos") } else { a.help_heading("Opções") })
        .mut_subcommands(localize)
}

fn command() -> clap::Command {
    localize(Cli::command().override_help(main_help()).disable_version_flag(true).arg(
        Arg::new("version").short('V').long("version").action(ArgAction::Version).help("Mostra a versão"),
    ))
}

/// O que sobra de inglês no texto que o clap gera.
fn translate(text: &str) -> String {
    [
        ("Usage:", "Uso:"),
        ("[OPTIONS]", "[OPÇÕES]"),
        ("[COMMAND]", "[COMANDO]"),
        ("<COMMAND>", "<COMANDO>"),
        ("[possible values: ", "[valores: "),
        ("[default: ", "[padrão: "),
    ]
    .iter()
    .fold(text.to_string(), |t, (en, pt)| t.replace(en, pt))
}

fn clap_error(e: clap::Error) -> ExitCode {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    let ctx = |k| e.get(k).map(|v: &ContextValue| v.to_string()).unwrap_or_default();
    let rendered = || {
        let r = e.render();
        translate(&if console::colors_enabled() { r.ansi().to_string() } else { r.to_string() })
    };
    match e.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            print!("{}", rendered());
            return ExitCode::SUCCESS;
        }
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => eprint!("{}", rendered()),
        ErrorKind::InvalidSubcommand => {
            let bad = ctx(ContextKind::InvalidSubcommand);
            let names: Vec<String> = Cli::command().get_subcommands().filter(|c| !c.is_hide_set()).map(|c| c.get_name().to_string()).collect();
            ui::error(&format!("comando desconhecido: {bad}{}", ui::did_you_mean(&bad, names.iter().map(String::as_str))));
            ui::hint("devup --help lista os comandos");
        }
        ErrorKind::InvalidValue => {
            let arg = ctx(ContextKind::InvalidArg).trim_matches(['[', ']', '<', '>', '.']).to_string();
            let value = ctx(ContextKind::InvalidValue);
            let valid = match e.get(ContextKind::ValidValue) {
                Some(ContextValue::Strings(v)) => v.clone(),
                _ => vec![],
            };
            let suggestion = ui::did_you_mean(&value, valid.iter().map(String::as_str));
            ui::error(&format!("valor inválido para {arg}: {value}{suggestion}"));
            if !valid.is_empty() {
                ui::hint(&format!("opções: {}", valid.join(", ")));
            }
        }
        ErrorKind::UnknownArgument => {
            let suggested = match e.get(ContextKind::SuggestedArg) {
                Some(s) => format!(" · você quis dizer {}?", style(s).bold()),
                None => String::new(),
            };
            ui::error(&format!("opção desconhecida: {}{suggested}", ctx(ContextKind::InvalidArg)));
            ui::hint("devup <comando> --help lista as opções");
        }
        ErrorKind::MissingRequiredArgument => {
            ui::error(&format!("falta informar {}", ctx(ContextKind::InvalidArg)));
            ui::hint("devup <comando> --help mostra exemplos");
        }
        ErrorKind::ArgumentConflict => {
            ui::error(&format!("{} não pode ser usado junto com {}", ctx(ContextKind::InvalidArg), ctx(ContextKind::PriorArg)));
        }
        _ => eprint!("{}", rendered()),
    }
    ExitCode::from(2)
}

// ---------- comandos ----------

struct Opts {
    dry_run: bool,
    yes: bool,
}

#[derive(Serialize)]
struct ListRow<'a> {
    name: &'a str,
    category: &'a str,
    description: &'a str,
    /// installed | native | aur | flatpak | script | config | unavailable | skipped
    method: &'static str,
    detail: String,
}

fn list(cat: &Catalog, sys: &Sys, category: Option<String>, profile: Option<String>, json: bool) -> Result<(), Error> {
    let mods = match &profile {
        Some(p) if cat.profiles.contains_key(p) => cat.expand(std::slice::from_ref(p))?,
        Some(p) => {
            let suggestion = ui::did_you_mean(p, cat.profiles.keys().map(String::as_str));
            return Err(Error::usage(format!("perfil desconhecido: {p}{suggestion}")));
        }
        None => cat.modules.iter().collect(),
    };
    let rows: Vec<ListRow> = mods
        .iter()
        .filter(|m| category.as_ref().is_none_or(|c| *c == m.category))
        .map(|m| {
            let (method, detail) = match plan::resolve(m, sys) {
                _ if let Some(reason) = plan::skip_reason(m, sys) => ("skipped", reason),
                Method::Installed => ("installed", String::new()),
                Method::Native(p) => ("native", format!("{}: {p}", sys.pm.unwrap().bin())),
                Method::Aur(p) => ("aur", p.to_string()),
                Method::Flatpak(f) => ("flatpak", f.to_string()),
                Method::Script(_) => ("script", String::new()),
                Method::Nothing => ("config", String::new()),
                Method::Unavailable => ("unavailable", String::new()),
            };
            ListRow { name: &m.name, category: &m.category, description: &m.description, method, detail }
        })
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&rows).unwrap());
        return Ok(());
    }
    for r in &rows {
        let how = match r.method {
            "installed" => style("instalado".to_string()).green(),
            "native" => style(r.detail.clone()),
            "aur" | "flatpak" => style(format!("{}: {}", r.method, r.detail)),
            "script" => style("script oficial".to_string()),
            "config" => style("só config".to_string()),
            "unavailable" => style("indisponível aqui".to_string()).red(),
            _ => style(r.detail.clone()).dim(),
        };
        println!("{:<9}{:<14}{:<40.40} {}", style(r.category).dim(), style(r.name).bold(), how, style(r.description).dim());
    }
    if profile.is_none() && category.is_none() {
        println!("\n{}", style("Perfis").bold().underlined());
        for (name, p) in &cat.profiles {
            println!("  {:<12}{}", style(name).bold(), style(&p.description).dim());
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct StatusRow<'a> {
    name: &'a str,
    /// installed | missing | unknown | skipped
    state: &'static str,
}

fn status(cat: &Catalog, sys: &Sys, cfg: &Config, json: bool) -> Result<(), Error> {
    let rows: Vec<(StatusRow, &Module)> = cfg
        .resolve(cat)?
        .into_iter()
        .map(|m| {
            let state = match plan::installed(m, sys) {
                _ if plan::skip_reason(m, sys).is_some() => "skipped",
                Some(true) => "installed",
                Some(false) => "missing",
                None => "unknown",
            };
            (StatusRow { name: &m.name, state }, m)
        })
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&rows.iter().map(|r| &r.0).collect::<Vec<_>>()).unwrap());
        return Ok(());
    }
    for (row, m) in &rows {
        let mark = match row.state {
            "installed" => style("✓").green(),
            "missing" => style("✗").red(),
            "skipped" => style("-").dim(),
            _ => style("?").yellow(),
        };
        println!("{} {:<14}{}", mark.bold(), row.name, style(&m.description).dim());
    }
    let missing = rows.iter().filter(|r| r.0.state == "missing").count();
    if missing > 0 {
        ui::hint(&format!("{missing} faltando · devup apply instala"));
    }
    if !cfg.exclude.is_empty() {
        println!("{} {}", style("excluídos:").dim(), cfg.exclude.join(", "));
    }
    Ok(())
}

/// Lê um devup.toml de um caminho ou URL; só vira o do usuário depois de validado.
fn fetch_config(from: &str) -> Result<Config, Error> {
    let text = if from.starts_with("https://") || from.starts_with("http://") {
        let out = Command::new("curl").args(["-fsSL", from]).output().map_err(|e| format!("curl: {e}"))?;
        if !out.status.success() {
            return Err(format!("não deu para baixar {from}").into());
        }
        String::from_utf8(out.stdout).map_err(|e| format!("{from}: {e}"))?
    } else {
        fs::read_to_string(from).map_err(|e| format!("{from}: {e}"))?
    };
    Ok(toml::from_str(&text).map_err(|e| format!("{from}: {e}"))?)
}

const NO_CONFIG: &str = "nenhum devup.toml ainda · rode devup init";

fn need_config(current: Option<Config>) -> Result<Config, Error> {
    current.ok_or_else(|| Error::usage(NO_CONFIG))
}

fn save(cfg: &Config, dry_run: bool) -> Result<(), Error> {
    if dry_run {
        println!("{}\n{}", style("devup.toml (não salvo, --dry-run):").dim(), toml::to_string(cfg).map_err(|e| e.to_string())?);
    } else {
        ui::status("Salvo", &Config::save(cfg)?.display().to_string());
    }
    Ok(())
}

fn clone_missing(urls: &[String], dry_run: bool) -> Result<(), Error> {
    for url in urls {
        let dir = dotfiles::repos_dir().join(config::repo_name(url));
        if dir.exists() {
            continue;
        }
        ui::status("Clonando", url);
        if dry_run {
            continue;
        }
        let ok = Command::new("git").args(["clone", "--quiet", url]).arg(&dir).status().is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("não deu para clonar {url}").into());
        }
    }
    Ok(())
}

/// Clona os repos, valida contra o catálogo completo, mostra o plano (se `confirm`) e só então salva e executa.
fn apply(sys: &Sys, cfg: &Config, o: &Opts, persist: bool, confirm: bool) -> Result<ExitCode, Error> {
    clone_missing(&cfg.dotfiles, o.dry_run)?;
    let cat = Catalog::load(&cfg.dotfiles)?;
    let mods = cfg.resolve(&cat)?;
    if let Some(t) = &cfg.theme {
        theme::render(t)?;
    }
    if confirm {
        let summary = ui::summarize(&mods, sys, &|m| plan::installed_pkg(m, sys), &|m| ui::system_done(m, sys));
        let changes = wizard::changes(cfg, wizard::login_shell(sys).as_deref());
        match ui::confirm(&summary, &changes, o.yes, o.dry_run)? {
            Decision::Cancel => return Ok(ExitCode::SUCCESS),
            Decision::Nothing => {
                if persist {
                    save(cfg, o.dry_run)?;
                }
                return Ok(ExitCode::SUCCESS);
            }
            Decision::Apply => {}
        }
    }
    if persist {
        save(cfg, o.dry_run)?;
    }
    let code = plan::run(&plan::plan(&mods, sys), sys, o.dry_run);
    if let Some(t) = &cfg.theme
        && theme::current().as_ref() != Some(t)
    {
        theme::apply(t, true, o.dry_run)?;
    }
    Ok(code)
}

fn install(cat: &Catalog, sys: &Sys, names: &[String], o: &Opts) -> Result<ExitCode, Error> {
    let mods = cat.expand(names)?;
    let summary = ui::summarize(&mods, sys, &|m| plan::installed_pkg(m, sys), &|m| ui::system_done(m, sys));
    match ui::confirm(&summary, &ui::Changes::default(), o.yes, o.dry_run)? {
        Decision::Apply => Ok(plan::run(&plan::plan(&mods, sys), sys, o.dry_run)),
        _ => Ok(ExitCode::SUCCESS),
    }
}

/// Lista para escolher apps e perfis quando o comando veio sem nomes.
fn pick_names(cat: &Catalog, prompt: &str, hide: &[String]) -> Result<Vec<String>, Error> {
    let ui = ui::need_terminal("os nomes, por exemplo: devup add docker")?;
    let mut names = vec![];
    let mut items = vec![];
    for (name, p) in cat.profiles.iter().filter(|(n, _)| !hide.contains(n)) {
        names.push(name.clone());
        items.push(format!("{} {:<12}{}", style("◆").magenta(), name, style(&p.description).dim()));
    }
    for m in cat.modules.iter().filter(|m| !hide.contains(&m.name)) {
        names.push(m.name.clone());
        items.push(format!("  {:<12}{:<9}{}", m.name, style(&m.category).dim(), style(&m.description).dim()));
    }
    let picked = ui.multi(prompt, &items, &|i| names[i].clone(), &vec![false; items.len()])?;
    Ok(picked.into_iter().map(|i| names[i].clone()).collect())
}

fn restore(sys: &Sys, id: Option<String>, dry_run: bool) -> Result<(), Error> {
    let ids = dotfiles::backups();
    if ids.is_empty() {
        return Err("nenhum backup ainda".into());
    }
    let id = match id {
        Some(id) if ids.contains(&id) => id,
        Some(id) => return Err(Error::usage(format!("backup {id} não existe{}", ui::did_you_mean(&id, ids.iter().map(String::as_str))))),
        None if ui::interactive() && ids.len() > 1 => {
            let newest_first: Vec<&String> = ids.iter().rev().collect();
            let labels: Vec<String> = newest_first
                .iter()
                .map(|id| {
                    let when = id.parse::<u64>().map_or(id.to_string(), |ms| plan::utc_stamp(ms / 1000).replacen('_', " ", 1));
                    let files = dotfiles::backup_entries(&dotfiles::backups_dir().join(id));
                    format!("{when} UTC  {}", style(format!("{} arquivo(s): {}", files.len(), files.join(", "))).dim())
                })
                .collect();
            let i = ui::Ui::new().select("Qual backup restaurar?", &labels, &|i| newest_first[i].clone(), 0)?;
            newest_first[i].clone()
        }
        None => ids.last().unwrap().clone(),
    };
    let dir = dotfiles::backups_dir().join(&id);
    if dry_run {
        dotfiles::backup_entries(&dir).iter().for_each(|e| println!("{} restaurar ~/{e}", style("→").dim()));
        return Ok(());
    }
    for path in dotfiles::restore(&dir, &sys.home).map_err(|e| e.to_string())? {
        ui::status("Restaurado", &path.display().to_string());
    }
    Ok(())
}

fn init(cat: &Catalog, sys: &Sys, current: Option<Config>, o: &Opts) -> Result<ExitCode, Error> {
    let Some(out) = wizard::run(cat, sys, current, o.yes)? else {
        eprintln!("Cancelado. Nada foi alterado.");
        return Ok(ExitCode::SUCCESS);
    };
    // O assistente já mostrou o plano e confirmou.
    let code = apply(sys, &out.cfg, o, true, false)?;
    if let Some((name, email)) = out.git {
        for (key, value) in [("user.name", name), ("user.email", email)] {
            ui::status("Configurando", &format!("git {key}"));
            if !o.dry_run && !Command::new("git").args(["config", "--global", key, &value]).status().is_ok_and(|s| s.success()) {
                ui::warn(&format!("não deu para configurar {key} do git"));
            }
        }
    }
    Ok(code)
}

fn run(cli: Cli, sys: &Sys) -> Result<ExitCode, Error> {
    let o = Opts { dry_run: cli.dry_run, yes: cli.yes };
    let current = Config::load()?;
    let cat = Catalog::load(current.as_ref().map_or(&[][..], |c| &c.dotfiles))?;
    match cli.cmd {
        None | Some(Cmd::Init) => init(&cat, sys, current, &o),
        Some(Cmd::List { category, profile, json }) => list(&cat, sys, category, profile, json).map(|_| ExitCode::SUCCESS),
        Some(Cmd::Status { json }) => status(&cat, sys, &need_config(current)?, json).map(|_| ExitCode::SUCCESS),
        Some(Cmd::Doctor { json }) => Ok(doctor::run(sys, &cat, current.as_ref(), json)),
        Some(Cmd::Apply { from: Some(from) }) => apply(sys, &fetch_config(&from)?, &o, true, true),
        Some(Cmd::Apply { from: None }) => apply(sys, &need_config(current)?, &o, false, true),
        Some(Cmd::Install { mut names }) => {
            if names.is_empty() {
                names = pick_names(&cat, "O que instalar? (espaço marca)", &[])?;
            }
            if names.is_empty() {
                return Ok(ExitCode::SUCCESS);
            }
            install(&cat, sys, &names, &o)
        }
        Some(Cmd::Add { mut names }) => {
            let mut cfg = current.unwrap_or_default();
            if names.is_empty() {
                names = pick_names(&cat, "O que adicionar? (espaço marca)", &cfg.names())?;
            }
            if names.is_empty() {
                return Ok(ExitCode::SUCCESS);
            }
            names.iter().try_for_each(|n| cfg.add(&cat, n))?;
            apply(sys, &cfg, &o, true, true)
        }
        Some(Cmd::Remove { mut names }) => {
            let mut cfg = need_config(current)?;
            if names.is_empty() {
                let listed = cfg.names();
                if listed.is_empty() {
                    return Err("o devup.toml não tem apps nem perfis para tirar".into());
                }
                let ui = ui::need_terminal("os nomes, por exemplo: devup remove vscode")?;
                let picked = ui.multi("O que tirar do devup.toml?", &listed, &|i| listed[i].clone(), &vec![false; listed.len()])?;
                names = picked.into_iter().map(|i| listed[i].clone()).collect();
            }
            names.iter().try_for_each(|n| cfg.remove(&cat, n))?;
            save(&cfg, o.dry_run)?;
            ui::hint("nada foi desinstalado; só não será mais aplicado");
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Dotfiles { cmd: DotfilesCmd::Add { url } }) => {
            let mut cfg = current.unwrap_or_default();
            if !cfg.dotfiles.contains(&url) {
                cfg.dotfiles.push(url);
            }
            apply(sys, &cfg, &o, true, true)
        }
        Some(Cmd::Dotfiles { cmd: DotfilesCmd::Sync }) => {
            let cfg = need_config(current)?;
            for url in &cfg.dotfiles {
                let dir = dotfiles::repos_dir().join(config::repo_name(url));
                ui::status("Atualizando", url);
                let ok = o.dry_run || Command::new("git").arg("-C").arg(&dir).args(["pull", "--ff-only", "--quiet"]).status().is_ok_and(|s| s.success());
                if !ok {
                    return Err(format!("não deu para atualizar {url}").into());
                }
            }
            apply(sys, &cfg, &o, false, true)
        }
        Some(Cmd::Shell { name }) => {
            let cfg = current.unwrap_or_default();
            let name = match name {
                Some(n) => n,
                None => {
                    let ui = ui::need_terminal("o shell, por exemplo: devup shell zsh")?;
                    let sig = wizard::Signals { shell: wizard::login_shell(sys), ..Default::default() };
                    let mut a = wizard::recommend(&sig, &cat, None);
                    if cfg.shell.is_some() {
                        a.shell = cfg.shell.clone();
                    }
                    wizard::ask_shell(&ui, &mut a, &sig)?;
                    match a.shell {
                        Some(s) => s,
                        None => return Ok(ExitCode::SUCCESS),
                    }
                }
            };
            apply(sys, &Config { shell: Some(name), ..cfg }, &o, true, true)
        }
        Some(Cmd::Theme { cmd: ThemeCmd::List { json } }) => {
            let applied = theme::current();
            let names = theme::names();
            if json {
                let rows: Vec<_> = names.iter().map(|n| serde_json::json!({ "name": n, "current": applied.as_ref() == Some(n) })).collect();
                println!("{}", serde_json::to_string_pretty(&rows).unwrap());
            } else {
                for n in &names {
                    let mark = if applied.as_ref() == Some(n) { style("●").green() } else { style(" ") };
                    println!("{} {}  {n}", mark, wizard::swatch(n));
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Theme { cmd: ThemeCmd::Set { name } }) => {
            let cfg = current.unwrap_or_default();
            let name = match name {
                Some(n) => n,
                None => {
                    let ui = ui::need_terminal("o tema, por exemplo: devup theme set nord")?;
                    let mut a = wizard::recommend(&wizard::Signals::default(), &cat, Some(&cfg));
                    a.theme = theme::current().or(a.theme);
                    wizard::ask_theme(&ui, &mut a)?;
                    match a.theme {
                        Some(t) => t,
                        None => return Ok(ExitCode::SUCCESS),
                    }
                }
            };
            theme::render(&name)?;
            save(&Config { theme: Some(name.clone()), ..cfg }, o.dry_run)?;
            theme::apply(&name, true, o.dry_run)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Update) => update::update(o.dry_run),
        Some(Cmd::Restore { id }) => restore(sys, id, o.dry_run).map(|_| ExitCode::SUCCESS),
        Some(Cmd::Completions { shell }) => {
            clap_complete::generate(shell, &mut command(), "devup", &mut io::stdout());
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::UpdateCheck) => {
            update::check();
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn main() -> ExitCode {
    // Saída cortada (`devup list | head`) encerra em silêncio, em vez de panic.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let _ = ctrlc::set_handler(|| {
        let _ = console::Term::stderr().show_cursor();
        eprintln!();
        std::process::exit(130);
    });

    let matches = match command().try_get_matches() {
        Ok(m) => m,
        Err(e) => return clap_error(e),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => return clap_error(e),
    };
    ui::set_color(&cli.color);
    ui::set_level(match (cli.quiet, cli.verbose) {
        (true, _) => ui::Level::Quiet,
        (_, true) => ui::Level::Verbose,
        _ => ui::Level::Normal,
    });
    let json = matches!(
        &cli.cmd,
        Some(Cmd::List { json: true, .. } | Cmd::Status { json: true } | Cmd::Doctor { json: true } | Cmd::Theme { cmd: ThemeCmd::List { json: true } })
    );
    let notify = !json && !matches!(&cli.cmd, Some(Cmd::Update | Cmd::Completions { .. } | Cmd::UpdateCheck));

    let sys = Sys::detect();
    if sys.pm.is_none() && !json {
        ui::warn("nenhum gerenciador de pacotes suportado (pacman/apt/dnf/zypper/apk/xbps); só flatpak e scripts");
    }
    let code = match run(cli, &sys) {
        Ok(code) => code,
        Err(e) if e.code == 130 => {
            eprintln!("Cancelado.");
            return ExitCode::from(130);
        }
        Err(e) => {
            ui::error(&e.msg);
            ExitCode::from(e.code)
        }
    };
    if notify {
        update::notice();
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_help_lists_every_command() {
        let help = console::strip_ansi_codes(&main_help()).to_string();
        for sub in Cli::command().get_subcommands().filter(|c| !c.is_hide_set()) {
            assert!(help.contains(&format!("  {}", sub.get_name())), "ajuda sem {}", sub.get_name());
        }
        command().debug_assert();
    }

    #[test]
    fn clap_errors_are_translated() {
        assert_eq!(translate("Usage: devup add [OPTIONS] [NOME]..."), "Uso: devup add [OPÇÕES] [NOME]...");
        let err = command().try_get_matches_from(["devup", "instal"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
        let err = command().try_get_matches_from(["devup", "shell", "zhs"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
        let help = command().try_get_matches_from(["devup", "add", "--help"]).unwrap_err();
        let text = translate(&help.render().to_string());
        assert!(text.contains("Uso:") && text.contains("Argumentos:") && text.contains("Opções:") && text.contains("Exemplos:"), "{text}");
        assert!(!text.contains("Usage") && !text.contains("Print help"), "{text}");
    }
}
