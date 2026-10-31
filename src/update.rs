use crate::config::state_dir;
use crate::ui::{self, Error};
use console::style;
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const INSTALL_URL: &str = "https://raw.githubusercontent.com/bernardofernandezz/devup/main/install.sh";
const LATEST_API: &str = "https://api.github.com/repos/bernardofernandezz/devup/releases/latest";
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

/// Roda o install.sh de novo, no mesmo diretório do binário atual.
pub fn update(dry_run: bool) -> Result<ExitCode, Error> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("binário sem diretório")?;
    ui::status("Atualizando", &format!("devup em {}", dir.display()));
    if dry_run {
        println!("{} curl -fsSL {INSTALL_URL} | DEVUP_BIN_DIR={} sh", style("→").dim(), dir.display());
        return Ok(ExitCode::SUCCESS);
    }
    let ok = Command::new("sh")
        .args(["-c", "curl -fsSL \"$0\" | sh", INSTALL_URL])
        .env("DEVUP_BIN_DIR", dir)
        .status()
        .is_ok_and(|s| s.success());
    if ok { Ok(ExitCode::SUCCESS) } else { Err("falha ao atualizar".into()) }
}

/// Consulta a última versão e grava no cache. Roda em segundo plano (`devup __update-check`).
pub fn check() {
    let Ok(out) = Command::new("curl").args(["-fsSL", "--max-time", "5", LATEST_API]).output() else { return };
    let latest = serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .ok()
        .and_then(|v| v.get("tag_name")?.as_str().map(String::from));
    let cache = Cache { checked: now(), latest };
    let _ = fs::create_dir_all(state_dir());
    let _ = fs::write(cache_path(), serde_json::to_string(&cache).unwrap_or_default());
}

/// Avisa se há versão nova (a partir do cache) e renova o cache uma vez por dia, sem bloquear.
pub fn notice() {
    if env::var_os("DEVUP_NO_UPDATE_CHECK").is_some() || !ui::interactive() || ui::level() == ui::Level::Quiet {
        return;
    }
    let cache: Cache = fs::read_to_string(cache_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if let Some(latest) = &cache.latest
        && newer(latest, env!("CARGO_PKG_VERSION"))
    {
        eprintln!(
            "\n{} nova versão do devup: {} → {} · rode {}",
            style("↑").cyan().bold(),
            env!("CARGO_PKG_VERSION"),
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
}
