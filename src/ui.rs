use crate::catalog::Module;
use crate::dotfiles::is_linked;
use crate::plan::{self, Method};
use crate::pm::{Sys, strs};
use console::style;
use dialoguer::{Input, MultiSelect, Select, theme::ColorfulTheme};
use std::{
    env,
    io::{self, IsTerminal},
    sync::OnceLock,
};

// ---------- erros com código de saída ----------

/// Erro que sabe com que código o processo deve sair: 1 falha, 2 uso incorreto, 130 cancelado.
#[derive(Debug)]
pub struct Error {
    pub code: u8,
    pub msg: String,
}

impl Error {
    pub fn usage(msg: impl Into<String>) -> Error {
        Error { code: 2, msg: msg.into() }
    }

    pub fn cancelled() -> Error {
        Error { code: 130, msg: String::new() }
    }
}

impl From<String> for Error {
    fn from(msg: String) -> Error {
        Error { code: 1, msg }
    }
}

impl From<&str> for Error {
    fn from(msg: &str) -> Error {
        Error { code: 1, msg: msg.into() }
    }
}

// ---------- nível de saída e cores ----------

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub enum Level {
    Quiet,
    Normal,
    Verbose,
}

static LEVEL: OnceLock<Level> = OnceLock::new();

pub fn set_level(level: Level) {
    let _ = LEVEL.set(level);
}

pub fn level() -> Level {
    *LEVEL.get().unwrap_or(&Level::Normal)
}

/// `auto` respeita NO_COLOR e se a saída é um terminal; `always` e `never` forçam.
pub fn set_color(choice: &str) {
    let on = match choice {
        "always" => true,
        "never" => false,
        _ => {
            if env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
                false
            } else {
                return;
            }
        }
    };
    console::set_colors_enabled(on);
    console::set_colors_enabled_stderr(on);
}

/// Dá para perguntar algo ao usuário?
pub fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

/// Linha de progresso no estilo do cargo: verbo verde alinhado à direita. Vai para o stderr.
pub fn status(verb: &str, msg: &str) {
    if level() >= Level::Normal {
        eprintln!("{} {msg}", style(format!("{verb:>12}")).green().bold());
    }
}

pub fn warn(msg: &str) {
    if level() >= Level::Normal {
        eprintln!("{} {msg}", style(format!("{:>12}", "aviso")).yellow().bold());
    }
}

pub fn error(msg: &str) {
    eprintln!("{} {msg}", style("✗").red().bold());
}

pub fn hint(msg: &str) {
    if level() >= Level::Normal {
        eprintln!("{} {msg}", style("→").cyan().bold());
    }
}

// ---------- sugestões ----------

/// Distância de edição em que trocar duas letras vizinhas conta como 1 (zhs → zsh).
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// O candidato mais parecido, se for parecido o bastante para valer a sugestão.
pub fn suggest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let input = input.to_lowercase();
    candidates
        .into_iter()
        .map(|c| {
            let d = if input.len() >= 3 && c.starts_with(&input) { 0 } else { distance(&input, c) };
            (d, c)
        })
        .filter(|(d, c)| *d <= 2 && *d < input.len().max(c.len()) / 2 + 1)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// " · você quis dizer X?" ou vazio.
pub fn did_you_mean<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> String {
    suggest(input, candidates).map(|s| format!(" · você quis dizer {}?", style(s).bold())).unwrap_or_default()
}

// ---------- prompts ----------

pub struct Ui {
    theme: ColorfulTheme,
}

fn prompt_error(e: dialoguer::Error) -> Error {
    let dialoguer::Error::IO(io) = e;
    if io.kind() == io::ErrorKind::Interrupted {
        let _ = console::Term::stderr().show_cursor();
        Error::cancelled()
    } else {
        io.to_string().into()
    }
}

fn report(prompt: &str, answer: &str) {
    let prompt = prompt.split(" (").next().unwrap_or(prompt);
    eprintln!("{} {} {} {}", style("✔").green(), style(prompt).bold(), style("·").dim(), style(answer).green());
}

impl Ui {
    pub fn new() -> Ui {
        Ui { theme: ColorfulTheme::default() }
    }

    /// Escolha única; `short` é o que fica na tela depois de responder.
    pub fn select(&self, prompt: &str, items: &[String], short: &dyn Fn(usize) -> String, default: usize) -> Result<usize, Error> {
        let i = Select::with_theme(&self.theme)
            .with_prompt(prompt)
            .items(items)
            .default(default)
            .report(false)
            .interact()
            .map_err(prompt_error)?;
        report(prompt, &short(i));
        Ok(i)
    }

    pub fn multi(&self, prompt: &str, items: &[String], short: &dyn Fn(usize) -> String, defaults: &[bool]) -> Result<Vec<usize>, Error> {
        let picked = MultiSelect::with_theme(&self.theme)
            .with_prompt(prompt)
            .items(items)
            .defaults(defaults)
            .report(false)
            .interact()
            .map_err(prompt_error)?;
        let answer = match picked.len() {
            0 => "nada".to_string(),
            n if n > 6 => format!("{n} selecionados"),
            _ => picked.iter().map(|i| short(*i)).collect::<Vec<_>>().join(", "),
        };
        report(prompt, &answer);
        Ok(picked)
    }

    pub fn input(&self, prompt: &str) -> Result<String, Error> {
        Input::<String>::with_theme(&self.theme)
            .with_prompt(prompt)
            .allow_empty(true)
            .interact_text()
            .map(|s| s.trim().to_string())
            .map_err(prompt_error)
    }
}

/// Garante que dá para perguntar; senão explica qual argumento passar.
pub fn need_terminal(what: &str) -> Result<Ui, Error> {
    if interactive() { Ok(Ui::new()) } else { Err(Error::usage(format!("sem terminal para perguntar; informe {what}"))) }
}

// ---------- plano ----------

#[derive(Default, Debug)]
pub struct Summary {
    pub install: Vec<String>,
    pub have: Vec<String>,
    pub configure: Vec<String>,
    pub skip: Vec<String>,
    /// Links que ainda não apontam para o devup.
    pub links: usize,
    /// Destinos (`~/...`) que já existem e vão para o backup.
    pub backups: Vec<String>,
    /// Serviços a habilitar e grupos a entrar que ainda não estão feitos.
    pub system: Vec<String>,
}

impl Summary {
    /// Há algo a fazer além dos comandos `run` (que são idempotentes)?
    pub fn pending(&self) -> bool {
        !self.install.is_empty() || self.links > 0 || !self.system.is_empty()
    }
}

/// Serviço habilitado e grupo do usuário; consultas só de leitura.
pub fn system_done(m: &Module, sys: &Sys) -> Vec<String> {
    let mut todo = vec![];
    if (sys.has)("systemctl") {
        for s in &m.services {
            if !plan::succeeds(&strs(&["systemctl", "is-enabled", "--quiet", s])) {
                todo.push(format!("serviço {s}"));
            }
        }
    }
    if let (Some(user), false) = (&sys.user, sys.root) {
        let groups = std::process::Command::new("id").args(["-nG", user]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        for g in &m.groups {
            if !groups.split_whitespace().any(|x| x == g) {
                todo.push(format!("grupo {g}"));
            }
        }
    }
    todo
}

pub fn summarize(
    mods: &[&Module],
    sys: &Sys,
    is_installed: &dyn Fn(&Module) -> Option<bool>,
    system_todo: &dyn Fn(&Module) -> Vec<String>,
) -> Summary {
    let mut s = Summary::default();
    for m in mods {
        if plan::skip_reason(m, sys).is_some() || plan::resolve(m, sys) == Method::Unavailable {
            s.skip.push(m.name.clone());
            continue;
        }
        let list = match is_installed(m) {
            Some(true) => &mut s.have,
            _ if m.has_install() => &mut s.install,
            _ => &mut s.configure,
        };
        list.push(m.name.clone());
        for f in &m.files {
            let (src, dest) = (m.root.join(&f.src), sys.home.join(&f.dest[2..]));
            if is_linked(&src, &dest) {
                continue;
            }
            s.links += 1;
            if dest.symlink_metadata().is_ok() {
                s.backups.push(f.dest.clone());
            }
        }
        s.system.extend(system_todo(m));
    }
    s
}

/// O que muda além dos apps; `None` omite a linha.
#[derive(Default)]
pub struct Changes {
    pub shell: Option<String>,
    pub theme: Option<String>,
}

fn row(label: &str, names: &[String], note: &str) {
    if names.is_empty() {
        return;
    }
    let shown: Vec<&str> = names.iter().take(6).map(String::as_str).collect();
    let more = if names.len() > 6 { format!(" +{}", names.len() - 6) } else { String::new() };
    println!("  {:<11}{:>3}  {}{}{}", style(label).bold(), names.len(), shown.join(", "), more, style(note).dim());
}

pub fn print_summary(s: &Summary, changes: &Changes) {
    println!("\n{}", style("Plano").bold().underlined());
    row("instalar", &s.install, "");
    row("já tem", &s.have, "");
    row("configurar", &s.configure, "");
    row("sistema", &s.system, "");
    row("pular", &s.skip, "  (não se aplicam aqui)");
    if s.links > 0 {
        let backups = if s.backups.is_empty() {
            String::new()
        } else {
            format!(" · {} para backup: {}", s.backups.len(), s.backups.join(", "))
        };
        println!("  {:<11}{:>3}  arquivos linkados{}", style("configs").bold(), s.links, backups);
    }
    if let Some(shell) = &changes.shell {
        println!("  {:<11}     {shell}", style("shell").bold());
    }
    if let Some(theme) = &changes.theme {
        println!("  {:<11}     {theme}", style("tema").bold());
    }
    println!();
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Apply,
    /// Tudo já está como pedido.
    Nothing,
    Cancel,
}

/// Mostra o plano e pergunta uma vez, como o `terraform apply`.
pub fn confirm(s: &Summary, changes: &Changes, yes: bool, dry_run: bool) -> Result<Decision, Error> {
    if !s.pending() && changes.shell.is_none() && changes.theme.is_none() {
        eprintln!("{} Nada a fazer: tudo já está como pedido.", style("✓").green().bold());
        return Ok(Decision::Nothing);
    }
    print_summary(s, changes);
    if dry_run || yes {
        return Ok(Decision::Apply);
    }
    if !interactive() {
        return Err(Error::usage("sem terminal para confirmar; rode de novo com -y para aplicar sem perguntar"));
    }
    let items = vec!["Aplicar".to_string(), "Cancelar".to_string()];
    if Ui::new().select("Aplicar?", &items, &|i| items[i].clone(), 0)? == 0 {
        Ok(Decision::Apply)
    } else {
        eprintln!("Cancelado. Nada foi alterado.");
        Ok(Decision::Cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_close_names_only() {
        let names = ["docker", "neovim", "nord", "nodejs", "steam"];
        assert_eq!(suggest("dokcer", names), Some("docker"));
        assert_eq!(suggest("nrod", names), Some("nord"));
        assert_eq!(suggest("neo", names), Some("neovim"));
        assert_eq!(suggest("Steam", names), Some("steam"));
        assert_eq!(suggest("zhs", ["zsh", "fish", "bash"]), Some("zsh"));
        assert_eq!(suggest("xyzzyq", names), None);
        assert_eq!(suggest("a", names), None);
    }

    #[test]
    fn summary_splits_and_predicts_backups() {
        use crate::catalog::Catalog;
        use crate::pm::Pm;
        use std::fs;
        let home = env::temp_dir().join(format!("devup-summary-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(".config/devup/shell")).unwrap();
        fs::write(home.join(".config/devup/shell/aliases.sh"), "meu").unwrap();
        let sys = Sys { pm: Some(Pm::Apt), root: false, user: None, has: |_| false, home: home.clone(), desktop: None };

        let cat = Catalog::builtin();
        let mods: Vec<&Module> = ["btop", "lutris", "gnome", "shell-common", "docker"].iter().map(|n| cat.module(n).unwrap()).collect();
        let s = summarize(&mods, &sys, &|m| Some(m.name == "btop"), &|m| if m.name == "docker" { vec!["serviço docker".into()] } else { vec![] });
        assert_eq!((s.have, s.install, s.skip, s.configure), (vec!["btop".to_string()], vec!["lutris".to_string(), "docker".to_string()], vec!["gnome".to_string()], vec!["shell-common".to_string()]));
        assert_eq!((s.links, s.backups, s.system.len()), (1, vec!["~/.config/devup/shell/aliases.sh".to_string()], 1));
        assert!(Summary::default().pending() == false);
        fs::remove_dir_all(home).unwrap();
    }
}
