use crate::config::state_dir;
use crate::net::{self, Checksum};
use crate::toolchains::github_release;
use crate::ui::{self, Error};
use console::style;
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const REPO: &str = "bernardofernandezz/devup";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const DAY: u64 = 24 * 60 * 60;

#[derive(Serialize, Deserialize, Default)]
struct Cache {
    checked: u64,
    latest: Option<String>,
}

fn cache_path() -> PathBuf {
    state_dir().join("update.json")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

fn newer(latest: &str, current: &str) -> bool {
    matches!((version(latest), version(current)), (Some(l), Some(c)) if l > c)
}

fn asset_name() -> String {
    format!("devup-{}-linux", env::consts::ARCH)
}

/// Caminho do binário em uso e do backup para rollback.
fn paths() -> Result<(PathBuf, PathBuf), Error> {
    let exe = env::current_exe().and_then(fs::canonicalize).map_err(|e| e.to_string())?;
    let old = exe.with_file_name(format!("{}.old", exe.file_name().unwrap().to_string_lossy()));
    Ok((exe, old))
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".devup-write-test");
    let ok = fs::write(&probe, "").is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

/// Troca `exe` por `new` de forma atômica, guardando o atual em `old`.
fn swap(exe: &Path, new: &Path, old: &Path) -> Result<(), String> {
    let _ = fs::remove_file(old);
    fs::hard_link(exe, old).or_else(|_| fs::copy(exe, old).map(|_| ())).map_err(|e| format!("backup de {}: {e}", exe.display()))?;
    fs::rename(new, exe).map_err(|e| format!("troca de {}: {e}", exe.display()))
}

fn release_notes(body: &str) -> String {
    body.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with("```") && !l.starts_with("curl ") && !l.starts_with("devup init"))
        .take(18)
        .map(|l| {
            let l = l.replace("**", "");
            if let Some(h) = l.strip_prefix("### ") { format!("  {}", style(h).bold()) } else { format!("  {}", style(l).dim()) }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn update(dry_run: bool, check_only: bool, rollback: bool) -> Result<ExitCode, Error> {
    let (exe, old) = paths()?;
    let dir = exe.parent().ok_or("binário sem diretório")?.to_path_buf();
    if !dry_run && !writable(&dir) {
        return Err(format!("sem permissão para escrever em {} · rode: sudo devup update{}", dir.display(), if rollback { " --rollback" } else { "" }).into());
    }

    if rollback {
        if !old.exists() {
            return Err("não há versão anterior guardada para voltar".into());
        }
        let out = Command::new(&old).arg("--version").output().map_err(|e| e.to_string())?;
        let previous = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if dry_run {
            ui::status("Voltaria", &format!("para {previous}"));
            return Ok(ExitCode::SUCCESS);
        }
        let tmp = exe.with_extension("rollback");
        fs::rename(&old, &tmp).map_err(|e| e.to_string())?;
        swap(&exe, &tmp, &old)?;
        ui::status("Restaurado", &format!("{previous} (a {VERSION} ficou guardada; devup update --rollback volta de novo)"));
        return Ok(ExitCode::SUCCESS);
    }

    let pb = indicatif::ProgressBar::new_spinner();
    if ui::animated() {
        pb.set_style(ui::spinner_style());
        pb.set_prefix("Consultando");
        pb.set_message("últimas versões do devup");
        pb.enable_steady_tick(std::time::Duration::from_millis(90));
    } else {
        pb.set_draw_target(indicatif::ProgressDrawTarget::hidden());
    }
    let rel = github_release(REPO, None)?;
    pb.finish_and_clear();
    let latest = rel.tag.trim_start_matches('v').to_string();
    if !newer(&latest, VERSION) {
        eprintln!("{} Você já tem a versão mais recente ({VERSION}).", style("✓").green().bold());
        return Ok(ExitCode::SUCCESS);
    }
    eprintln!("{} devup {} → {}\n", style("↑").cyan().bold(), VERSION, style(&latest).green().bold());
    eprintln!("{}\n", release_notes(&rel.body));
    if check_only || dry_run {
        ui::hint("devup update instala");
        return Ok(ExitCode::SUCCESS);
    }

    let name = asset_name();
    let url = rel.asset(&name).ok_or_else(|| format!("o release {} não tem {name}", rel.tag))?;
    let sums = net::get_text(rel.asset("sha256sums.txt").ok_or("release sem sha256sums.txt")?)?;
    let sum = Checksum::Sha256(net::find_sum(&sums, &name).ok_or("sha256sums.txt sem o binário")?);

    let pb = indicatif::ProgressBar::new(0);
    if ui::animated() {
        pb.set_style(ui::bar_style());
        pb.set_prefix("Baixando");
        pb.set_message(format!("devup {latest}"));
    } else {
        pb.set_draw_target(indicatif::ProgressDrawTarget::hidden());
        ui::status("Baixando", &format!("devup {latest}"));
    }
    let downloaded = net::fetch(url, &name, &sum, &pb)?;
    pb.finish_and_clear();

    let new = dir.join(".devup.new");
    fs::copy(&downloaded, &new).map_err(|e| e.to_string())?;
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&new, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    // Só troca se o binário novo roda nesta máquina.
    let runs = Command::new(&new).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if !runs {
        let _ = fs::remove_file(&new);
        return Err("o binário novo não rodou nesta máquina; nada foi trocado".into());
    }
    swap(&exe, &new, &old)?;
    ui::splash(&format!("atualizado para {latest} · checksum conferido"));
    eprintln!("{} devup {latest} instalado em {}", style("✓").green().bold(), exe.display());
    ui::hint("devup update --rollback volta para a versão anterior");
    let _ = fs::write(cache_path(), serde_json::to_string(&Cache { checked: now(), latest: Some(rel.tag) }).unwrap_or_default());
    Ok(ExitCode::SUCCESS)
}

/// Consulta a última versão e grava no cache. Roda em segundo plano (`devup __update-check`).
pub fn check() {
    let latest = github_release(REPO, None).ok().map(|r| r.tag);
    let _ = fs::create_dir_all(state_dir());
    let _ = fs::write(cache_path(), serde_json::to_string(&Cache { checked: now(), latest }).unwrap_or_default());
}

/// Avisa se há versão nova (a partir do cache) e renova o cache uma vez por dia, sem bloquear.
pub fn notice() {
    if env::var_os("DEVUP_NO_UPDATE_CHECK").is_some() || !ui::interactive() || ui::level() == ui::Level::Quiet {
        return;
    }
    let cache: Cache = fs::read_to_string(cache_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if let Some(latest) = &cache.latest
        && newer(latest, VERSION)
    {
        eprintln!(
            "\n{} nova versão do devup: {VERSION} → {} · rode {}",
            style("↑").cyan().bold(),
            style(latest.trim_start_matches('v')).green().bold(),
            style("devup update").bold()
        );
    }
    if now().saturating_sub(cache.checked) > DAY
        && let Ok(exe) = env::current_exe()
    {
        // Marca antes, para várias execuções seguidas não dispararem várias consultas.
        let _ = fs::create_dir_all(state_dir());
        let _ = fs::write(cache_path(), serde_json::to_string(&Cache { checked: now(), ..cache }).unwrap_or_default());
        let _ = Command::new(exe).arg("__update-check").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("v0.3.1", "0.3.0"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("v0.3.0", "0.3.0"));
        assert!(!newer("v0.2.9", "0.3.0"));
        assert!(!newer("lixo", "0.3.0"));
    }

    #[test]
    fn swap_keeps_a_backup_and_rollback_restores_it() {
        let d = env::temp_dir().join(format!("devup-swap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let (exe, new, old) = (d.join("devup"), d.join(".devup.new"), d.join("devup.old"));
        fs::write(&exe, "v1").unwrap();
        fs::write(&new, "v2").unwrap();
        swap(&exe, &new, &old).unwrap();
        assert_eq!((fs::read_to_string(&exe).unwrap(), fs::read_to_string(&old).unwrap()), ("v2".into(), "v1".into()));
        assert!(!new.exists());
        // rollback = trocar de novo usando o backup
        let tmp = exe.with_extension("rollback");
        fs::rename(&old, &tmp).unwrap();
        swap(&exe, &tmp, &old).unwrap();
        assert_eq!((fs::read_to_string(&exe).unwrap(), fs::read_to_string(&old).unwrap()), ("v1".into(), "v2".into()));
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn notes_are_trimmed_for_the_terminal() {
        let notes = release_notes("Primeira linha.\n\n### Destaques\n\n- **Novo**: coisa\n```sh\ncurl x | sh\n```\n");
        let plain = console::strip_ansi_codes(&notes).to_string();
        assert!(plain.contains("Destaques") && plain.contains("- Novo: coisa") && !plain.contains("curl"));
    }
}
