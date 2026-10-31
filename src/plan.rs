use crate::catalog::{FileLink, Module};
use crate::config::{SHELLS, state_dir};
use crate::dotfiles::{self, backups_dir, is_linked};
use crate::pm::{Sys, strs};
use crate::theme;
use crate::ui::{self, Level};
use console::style;
use indicatif::ProgressBar;
use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, PartialEq)]
pub enum Step {
    /// Comando avulso; `verb` + `what` formam a linha de progresso e `hint` aparece no fim se der certo.
    Cmd { verb: &'static str, what: String, cmd: Vec<String>, hint: Option<String> },
    /// Instalação em grupo; se falhar, tenta pacote por pacote para só os inexistentes falharem.
    Packages { base: Vec<String>, pkgs: Vec<String> },
    /// (origem, destino) de cada config.
    Links(Vec<(PathBuf, PathBuf)>),
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Step::Cmd { cmd, .. } => write!(f, "{}", cmd.join(" ")),
            Step::Packages { base, pkgs } => write!(f, "{} {}", base.join(" "), pkgs.join(" ")),
            Step::Links(links) => {
                let lines: Vec<String> = links.iter().map(|(src, dest)| format!("link {} -> {}", dest.display(), src.display())).collect();
                write!(f, "{}", lines.join("\n→ "))
            }
        }
    }
}

impl Step {
    fn cmd(verb: &'static str, what: impl Into<String>, cmd: Vec<String>) -> Step {
        Step::Cmd { verb, what: what.into(), cmd, hint: None }
    }

    /// Para que o sudo será usado nesta etapa, se for.
    fn sudo_reason(&self) -> Option<&'static str> {
        match self {
            Step::Packages { base, .. } if base[0] == "sudo" => Some("instalar pacotes"),
            Step::Cmd { cmd, verb, .. } if cmd[0] == "sudo" => Some(match *verb {
                "Habilitando" => "habilitar serviços",
                "Atualizando" => "atualizar o índice de pacotes",
                _ => "configurar o sistema",
            }),
            Step::Cmd { cmd, .. } if cmd[0] == "sh" && cmd[2].contains("sudo ") => Some("configurar o sistema"),
            _ => None,
        }
    }
}

/// "12 pacotes (pacman)", "3 apps (flatpak)", "1 pacote do AUR (yay)".
fn packages_label(base: &[String], n: usize) -> String {
    let tool = base.iter().find(|t| *t != "sudo" && *t != "env" && !t.contains('=')).map_or("", String::as_str);
    let plural = |one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    match tool {
        "flatpak" => format!("{} (flatpak)", plural("app", "apps")),
        "paru" | "yay" => format!("{} ({tool})", plural("pacote do AUR", "pacotes do AUR")),
        "apt-get" => format!("{} (apt)", plural("pacote", "pacotes")),
        "xbps-install" => format!("{} (xbps)", plural("pacote", "pacotes")),
        _ => format!("{} ({tool})", plural("pacote", "pacotes")),
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
    let pkg = installed_pkg(m, sys);
    if m.files.is_empty() {
        return pkg;
    }
    let linked = m.files.iter().all(|f| {
        let (src, dest) = link_paths(m, f, sys);
        is_linked(&src, &dest)
    });
    Some(pkg.unwrap_or(true) && linked)
}

/// Só a parte de pacotes: None quando o módulo não instala nada ou não dá para saber.
pub fn installed_pkg(m: &Module, sys: &Sys) -> Option<bool> {
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

fn link_paths(m: &Module, f: &FileLink, sys: &Sys) -> (PathBuf, PathBuf) {
    (m.root.join(&f.src), sys.home.join(&f.dest[2..]))
}

pub fn succeeds(cmd: &[String]) -> bool {
    Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Instalações agrupadas (nativo, aur, flatpak), scripts, todos os links e por fim services/groups/run na ordem dos módulos.
pub fn plan(mods: &[&Module], sys: &Sys) -> Vec<Step> {
    let (mut native, mut aur, mut flatpak, mut scripts, mut links, mut post) = (vec![], vec![], vec![], vec![], vec![], vec![]);
    for m in mods {
        if skip_reason(m, sys).is_some() {
            continue;
        }
        match resolve(m, sys) {
            Method::Native(p) => native.extend(p.split_whitespace()),
            Method::Aur(p) => aur.extend(p.split_whitespace()),
            Method::Flatpak(f) => flatpak.push(f),
            Method::Script(s) => scripts.push((m.name.as_str(), s)),
            Method::Installed | Method::Nothing => {}
            Method::Unavailable => continue,
        }
        links.extend(m.files.iter().map(|f| link_paths(m, f, sys)));
        if !m.services.is_empty() && !(sys.has)("systemctl") {
            ui::warn(&format!("{}: sem systemd, serviços não serão habilitados", m.name));
        } else {
            post.extend(m.services.iter().map(|s| Step::cmd("Habilitando", format!("serviço {s}"), sys.sudo(&["systemctl", "enable", "--now", s]))));
        }
        if let (Some(user), false) = (&sys.user, sys.root) {
            post.extend(m.groups.iter().map(|g| Step::Cmd {
                verb: "Configurando",
                what: format!("grupo {g}"),
                cmd: sys.sudo(&["usermod", "-aG", g, user]),
                hint: Some(format!("Saia e entre de novo na sessão para usar o grupo {g}")),
            }));
        }
        let shell_hint = (m.category == "shell" && SHELLS.contains(&m.name.as_str())).then(|| format!("Abra um novo terminal para usar o {}", m.name));
        post.extend(m.run.iter().map(|r| Step::Cmd {
            verb: "Configurando",
            what: m.name.clone(),
            cmd: strs(&["sh", "-c", r]),
            hint: shell_hint.clone(),
        }));
    }

    let mut steps = vec![];
    if !flatpak.is_empty() && !(sys.has)("flatpak") {
        native.push("flatpak");
    }
    if let (Some(base), false) = (sys.install_base(), native.is_empty()) {
        steps.extend(sys.refresh_cmd().map(|c| Step::cmd("Atualizando", "índice de pacotes", c)));
        steps.push(Step::Packages { base, pkgs: strs(&native) });
    }
    if let (Some(helper), false) = (sys.aur_helper(), aur.is_empty()) {
        steps.push(Step::Packages { base: strs(&[helper, "-S", "--needed", "--noconfirm"]), pkgs: strs(&aur) });
    }
    if !flatpak.is_empty() {
        let remote = strs(&["flatpak", "remote-add", "--user", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"]);
        steps.push(Step::cmd("Preparando", "Flathub", remote));
        steps.push(Step::Packages { base: strs(&["flatpak", "install", "--user", "-y", "flathub"]), pkgs: strs(&flatpak) });
    }
    steps.extend(scripts.into_iter().map(|(name, s)| Step::cmd("Instalando", format!("{name} (script oficial)"), strs(&["sh", "-c", s]))));
    if !links.is_empty() {
        steps.push(Step::Links(links));
    }
    steps.extend(post);
    steps
}

// ---------- execução ----------

pub fn utc_stamp(secs: u64) -> String {
    // Data civil a partir de dias desde 1970 (algoritmo de Howard Hinnant), sem depender de crate de datas.
    let (days, rem) = ((secs / 86400) as i64, secs % 86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}_{:02}-{:02}-{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn fmt_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 { format!("{secs:.1}s") } else { format!("{}m{:02}s", d.as_secs() / 60, d.as_secs() % 60) }
}

const KEEP_LOGS: usize = 20;

/// Guarda a saída completa de cada comando de uma execução.
struct Log {
    file: Option<fs::File>,
    path: PathBuf,
}

impl Log {
    fn open() -> Log {
        let dir = state_dir().join("logs");
        let _ = fs::create_dir_all(&dir);
        rotate(&dir, KEEP_LOGS - 1);
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let path = dir.join(format!("{}.log", utc_stamp(secs)));
        Log { file: fs::File::create(&path).ok(), path }
    }

    fn write(&mut self, text: &[u8]) {
        if let Some(f) = &mut self.file {
            let _ = f.write_all(text);
        }
    }
}

/// Mantém só os `keep` logs mais recentes (o nome é a data, então a ordem alfabética é a cronológica).
fn rotate(dir: &Path, keep: usize) {
    let mut logs: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "log")).collect();
    logs.sort();
    let excess = logs.len().saturating_sub(keep);
    for old in &logs[..excess] {
        let _ = fs::remove_file(old);
    }
}

fn spinner(verb: &str, label: &str) -> Option<ProgressBar> {
    if !ui::animated() {
        return None;
    }
    let pb = ProgressBar::new_spinner();
    pb.set_style(ui::spinner_style());
    pb.set_prefix(verb.to_string());
    pb.set_message(label.to_string());
    pb.enable_steady_tick(Duration::from_millis(90));
    Some(pb)
}

/// Roda um comando mostrando só a linha de progresso; a saída vai para o log e, se falhar
/// (e `report` for true), o final dela para a tela.
fn exec(verb: &str, label: &str, cmd: &[String], log: &mut Log, report: bool) -> bool {
    log.write(format!("\n==> {label}\n$ {}\n", cmd.join(" ")).as_bytes());
    if ui::level() == Level::Verbose {
        ui::status(verb, label);
        return Command::new(&cmd[0]).args(&cmd[1..]).status().is_ok_and(|s| s.success());
    }
    let pb = spinner(verb, label);
    if pb.is_none() {
        ui::status(verb, label);
    }
    let start = Instant::now();
    let (ok, output) = match Command::new(&cmd[0]).args(&cmd[1..]).stdin(Stdio::null()).output() {
        Ok(o) => (o.status.success(), [o.stdout, o.stderr].concat()),
        Err(e) => (false, format!("não deu para executar {}: {e}", cmd[0]).into_bytes()),
    };
    log.write(&output);
    if let Some(pb) = &pb {
        pb.finish_and_clear();
    }
    if !ok && report {
        ui::error(label);
        let text = String::from_utf8_lossy(&output);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        for line in &lines[lines.len().saturating_sub(15)..] {
            eprintln!("    {}", style(line).dim());
        }
    } else if pb.is_some() {
        let took = start.elapsed();
        let took = if took.as_secs() >= 1 { format!(" {}", style(fmt_duration(took)).dim()) } else { String::new() };
        ui::status(verb, &format!("{label}{took}"));
    }
    ok
}

/// Pede a senha do sudo uma vez, antes de tudo, e mantém o sudo ativo até o fim.
fn sudo_ready(steps: &[Step]) -> bool {
    let mut reasons: Vec<&str> = steps.iter().filter_map(Step::sudo_reason).collect();
    reasons.dedup();
    if reasons.is_empty() {
        return true;
    }
    let quiet = |cmd: &mut Command| cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if !quiet(Command::new("sudo").args(["-n", "true"])) {
        if !ui::interactive() {
            ui::error(&format!("o devup precisa de sudo para {} e não há terminal para pedir a senha", reasons.join(", ")));
            return false;
        }
        ui::hint(&format!("o devup vai usar sudo para {}", reasons.join(", ")));
        if !Command::new("sudo").args(["-v", "-p", "  senha de %p para o sudo: "]).status().is_ok_and(|s| s.success()) {
            ui::error("sem a senha do sudo não dá para continuar");
            return false;
        }
    }
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(60));
            quiet(Command::new("sudo").args(["-n", "-v"]));
        }
    });
    true
}

pub fn run(steps: &[Step], sys: &Sys, dry_run: bool) -> ExitCode {
    if dry_run {
        for step in steps {
            println!("{} {step}", style("→").dim());
        }
        return ExitCode::SUCCESS;
    }
    if steps.is_empty() {
        return ExitCode::SUCCESS;
    }
    if !sudo_ready(steps) {
        return ExitCode::FAILURE;
    }
    let start = Instant::now();
    let mut log = Log::open();
    let (mut failed, mut hints, mut pkgs_ok, mut links_ok) = (vec![], vec![], 0, 0);

    let has_links = steps.iter().any(|s| matches!(s, Step::Links(_)));
    if has_links && let Err(e) = dotfiles::extract_assets() {
        failed.push(format!("extrair configs: {e}"));
    }
    // As configs linkadas incluem arquivos do tema; sem nenhum escolhido, usa o padrão.
    if has_links && !theme::current_dir().exists() && let Err(e) = theme::apply(theme::DEFAULT, false, false) {
        failed.push(e);
    }
    let backup = backups_dir().join(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis().to_string());

    let total = steps.len();
    for (i, step) in steps.iter().enumerate() {
        let n = style(format!("[{}/{total}]", i + 1)).dim().to_string();
        match step {
            Step::Cmd { verb, what, cmd, hint } => {
                if exec(verb, &format!("{n} {what}"), cmd, &mut log, true) {
                    hints.extend(hint.clone());
                } else {
                    failed.push(what.clone());
                }
            }
            Step::Packages { base, pkgs } => {
                let label = format!("{n} {}", packages_label(base, pkgs.len()));
                // Em grupo, a falha não é mostrada: os pacotes são tentados um por um logo abaixo.
                if exec("Instalando", &label, &[base.clone(), pkgs.clone()].concat(), &mut log, pkgs.len() == 1) {
                    pkgs_ok += pkgs.len();
                } else if pkgs.len() > 1 {
                    ui::hint("algum pacote falhou; instalando um por um para isolar qual");
                    for p in pkgs {
                        if exec("Instalando", &format!("{n} {p}"), &[base.clone(), vec![p.clone()]].concat(), &mut log, true) {
                            pkgs_ok += 1;
                        } else {
                            failed.push(p.clone());
                        }
                    }
                } else {
                    failed.extend(pkgs.iter().cloned());
                }
            }
            Step::Links(links) => {
                let mut moved = 0;
                for (src, dest) in links {
                    log.write(format!("link {} -> {}\n", dest.display(), src.display()).as_bytes());
                    match dotfiles::link(src, dest, &sys.home, &backup) {
                        Ok(was_moved) => {
                            links_ok += 1;
                            moved += usize::from(was_moved);
                        }
                        Err(e) => {
                            ui::error(&format!("{}: {e}", dest.display()));
                            failed.push(dest.display().to_string());
                        }
                    }
                }
                let note = if moved > 0 { format!(" · {moved} para backup") } else { String::new() };
                ui::status("Linkando", &format!("{n} {} configs{note}", links.len()));
                if moved > 0 {
                    hints.push(format!("Os arquivos substituídos estão em {} (devup restore desfaz)", backup.display()));
                }
            }
        }
    }

    let took = fmt_duration(start.elapsed());
    let mut counts = vec![];
    if pkgs_ok > 0 {
        counts.push(format!("{pkgs_ok} pacotes"));
    }
    if links_ok > 0 {
        counts.push(format!("{links_ok} configs"));
    }
    let counts = counts.iter().map(|c| format!(" · {c}")).collect::<String>();
    let code = if failed.is_empty() {
        eprintln!("{} Pronto em {took}{counts}", style("✓").green().bold());
        ExitCode::SUCCESS
    } else {
        ui::error(&format!("Terminou em {took} com {} falha(s): {}", failed.len(), failed.join(", ")));
        hints.insert(0, format!("Log completo: {}", log.path.display()));
        ExitCode::FAILURE
    };
    hints.dedup();
    for h in &hints {
        ui::hint(h);
    }
    code
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
                Step::Cmd { cmd, .. } => cmd,
                Step::Packages { base, pkgs } => [base, pkgs].concat(),
                Step::Links(_) => panic!("link inesperado"),
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
        assert!(matches!(&steps[1], Step::Packages { base, .. } if base.contains(&"apt-get".to_string())));
        assert_eq!(steps.last().unwrap(), &Step::Links(vec![(tmux.root.join("tmux/tmux.conf"), "/h/.config/tmux/tmux.conf".into())]));
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

    #[test]
    fn steps_have_readable_labels_and_hints() {
        let cat = Catalog::builtin();
        let mods = cat.expand(&["docker".into(), "zsh".into()]).unwrap();
        let steps = plan(&mods, &sys(Some(Pm::Apt), false, |b| b == "systemctl"));
        let labels: Vec<String> = steps
            .iter()
            .map(|s| match s {
                Step::Cmd { verb, what, .. } => format!("{verb} {what}"),
                Step::Packages { base, pkgs } => packages_label(base, pkgs.len()),
                Step::Links(l) => format!("{} links", l.len()),
            })
            .collect();
        assert_eq!(labels[0], "Atualizando índice de pacotes");
        assert_eq!(labels[1], "10 pacotes (apt)"); // docker.io docker-compose + 5 cli-tools + zsh e 2 plugins
        assert!(labels.contains(&"Habilitando serviço docker".to_string()));
        assert!(steps.iter().any(|s| matches!(s, Step::Cmd { hint: Some(h), .. } if h.contains("grupo docker"))));
        assert!(steps.iter().any(|s| matches!(s, Step::Cmd { hint: Some(h), .. } if h.contains("usar o zsh"))));
        assert_eq!(steps.iter().filter_map(Step::sudo_reason).collect::<Vec<_>>()[..2], ["atualizar o índice de pacotes", "instalar pacotes"]);
        assert_eq!(packages_label(&strs(&["flatpak", "install"]), 1), "1 app (flatpak)");
    }

    #[test]
    fn log_names_sort_by_date_and_rotate() {
        assert_eq!(utc_stamp(0), "1970-01-01_00-00-00");
        assert_eq!(utc_stamp(1_793_420_000), "2026-10-31_04-13-20");
        let dir = std::env::temp_dir().join(format!("devup-logs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for d in 1..=5 {
            fs::write(dir.join(format!("2026-01-0{d}_00-00-00.log")), "").unwrap();
        }
        rotate(&dir, 2);
        let mut left: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["2026-01-04_00-00-00.log", "2026-01-05_00-00-00.log"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
