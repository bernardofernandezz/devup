use crate::config::{data_dir, home};
use serde::Serialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

/// O que dá para saber da máquina lendo /proc, /sys e o ambiente, sem pedir nada.
#[derive(Serialize, Debug, Default)]
pub struct Env {
    pub arch: String,
    pub musl: bool,
    pub avx2: bool,
    pub cpus: usize,
    pub mem_gb: u64,
    pub distro: Option<String>,
    pub session: Option<String>,
    pub wsl: bool,
    pub container: bool,
    pub laptop: bool,
    pub gpus: Vec<&'static str>,
    pub nvidia_driver: bool,
}

/// Raiz do sistema de arquivos; os testes apontam para um diretório falso.
pub fn detect() -> Env {
    detect_in(Path::new("/"))
}

fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path.trim_start_matches('/'))).unwrap_or_default()
}

pub fn detect_in(root: &Path) -> Env {
    let cpuinfo = read(root, "/proc/cpuinfo");
    let meminfo = read(root, "/proc/meminfo");
    let mem_kb: u64 = meminfo
        .lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0);
    let os_release = read(root, "/etc/os-release");
    let distro = os_release.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string());
    let mut gpus: Vec<&'static str> = fs::read_dir(root.join("sys/class/drm"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read_to_string(e.path().join("device/vendor")).ok())
        .filter_map(|v| match v.trim() {
            "0x10de" => Some("NVIDIA"),
            "0x1002" => Some("AMD"),
            "0x8086" => Some("Intel"),
            _ => None,
        })
        .collect();
    gpus.sort();
    gpus.dedup();
    let laptop = fs::read_dir(root.join("sys/class/power_supply"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("BAT"));
    let musl = fs::read_dir(root.join("lib"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("ld-musl"));
    Env {
        arch: env::consts::ARCH.to_string(),
        musl,
        avx2: cpuinfo.lines().any(|l| l.starts_with("flags") && l.split_whitespace().any(|f| f == "avx2")),
        cpus: thread::available_parallelism().map_or(1, |n| n.get()),
        mem_gb: (mem_kb as f64 / 1024.0 / 1024.0).round() as u64,
        distro,
        session: env::var("XDG_SESSION_TYPE").ok().filter(|s| !s.is_empty()),
        wsl: read(root, "/proc/version").to_lowercase().contains("microsoft"),
        container: root.join(".dockerenv").exists() || root.join("run/.containerenv").exists(),
        laptop,
        nvidia_driver: root.join("proc/driver/nvidia/version").exists(),
        gpus,
    }
}

impl Env {
    /// "x86_64 · NVIDIA · 16 núcleos · 32 GB · Wayland · notebook"
    pub fn summary(&self) -> String {
        let mut parts = vec![self.arch.clone()];
        parts.extend(self.gpus.iter().map(|g| g.to_string()));
        parts.push(format!("{} núcleos", self.cpus));
        if self.mem_gb > 0 {
            parts.push(format!("{} GB", self.mem_gb));
        }
        if let Some(s) = &self.session {
            parts.push(match s.as_str() {
                "wayland" => "Wayland".into(),
                "x11" => "X11".into(),
                other => other.into(),
            });
        }
        if self.laptop {
            parts.push("notebook".into());
        }
        if self.wsl {
            parts.push("WSL".into());
        }
        if self.container {
            parts.push("container".into());
        }
        if self.musl {
            parts.push("musl".into());
        }
        parts.join(" · ")
    }
}

/// Uma linguagem que já está no PATH.
#[derive(Serialize, Debug, Clone)]
pub struct Tool {
    pub lang: &'static str,
    pub version: String,
    /// devup | mise | nvm | asdf | rustup | sdkman | distro | outro
    pub source: &'static str,
    pub path: PathBuf,
}

/// De onde veio um binário, pelo caminho real dele.
pub fn source_of(path: &Path) -> &'static str {
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let p = real.to_string_lossy();
    let ours = data_dir();
    if real.starts_with(&ours) || path.starts_with(&ours) {
        "devup"
    } else if p.contains("/mise/") {
        "mise"
    } else if p.contains("/.nvm/") {
        "nvm"
    } else if p.contains("/.asdf/") {
        "asdf"
    } else if p.contains("/.sdkman/") {
        "sdkman"
    } else if real.starts_with(home().join(".cargo")) || real.starts_with(home().join(".rustup")) {
        "rustup"
    } else if p.starts_with("/usr/") || p.starts_with("/bin/") {
        "distro"
    } else {
        "outro"
    }
}

fn which(bin: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|p| env::split_paths(&p).map(|d| d.join(bin)).find(|f| f.is_file()))
}

/// Primeira versão no estilo x.y.z que aparece na saída.
fn first_version(text: &str) -> Option<String> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|t| t.matches('.').count() >= 1 && t.split('.').all(|p| !p.is_empty()))
        .map(String::from)
}

/// (linguagem, binário, argumentos de versão)
const PROBES: [(&str, &str, &[&str]); 11] = [
    ("node", "node", &["--version"]),
    ("bun", "bun", &["--version"]),
    ("deno", "deno", &["--version"]),
    ("rust", "rustc", &["--version"]),
    ("go", "go", &["version"]),
    ("zig", "zig", &["version"]),
    ("python", "python3", &["--version"]),
    ("java", "java", &["-version"]),
    ("dotnet", "dotnet", &["--version"]),
    ("flutter", "flutter", &["--version", "--suppress-analytics"]),
    ("kotlin", "kotlinc", &["-version"]),
];

/// Linguagens no PATH, com versão e origem. As consultas rodam em paralelo.
pub fn tools() -> Vec<Tool> {
    let handles: Vec<_> = PROBES
        .iter()
        .filter_map(|(lang, bin, args)| {
            let path = which(bin)?;
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            Some(thread::spawn(move || {
                let out = Command::new(&path).args(&args).stdin(Stdio::null()).output().ok()?;
                let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                Some(Tool { lang, version: first_version(&text)?, source: source_of(&path), path })
            }))
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hardware_from_a_fake_root() {
        let root = env::temp_dir().join(format!("devup-detect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let w = |p: &str, text: &str| {
            let f = root.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, text).unwrap();
        };
        w("proc/cpuinfo", "processor : 0\nflags : fpu sse avx avx2\n");
        w("proc/meminfo", "MemTotal:       32768000 kB\n");
        w("proc/version", "Linux version 6.6 (microsoft-standard-WSL2)");
        w("etc/os-release", "NAME=Ubuntu\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n");
        w("sys/class/drm/card0/device/vendor", "0x10de\n");
        w("sys/class/drm/card1/device/vendor", "0x8086\n");
        w("sys/class/power_supply/BAT0/status", "Charging");
        w("lib/ld-musl-x86_64.so.1", "");
        let e = detect_in(&root);
        assert!(e.avx2 && e.wsl && e.laptop && e.musl && !e.nvidia_driver);
        assert_eq!((e.mem_gb, e.distro.as_deref(), e.gpus.clone()), (31, Some("Ubuntu 24.04 LTS"), vec!["Intel", "NVIDIA"]));
        assert!(e.summary().contains("NVIDIA") && e.summary().contains("notebook"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_versions_and_sources() {
        assert_eq!(first_version("v22.11.0\n").as_deref(), Some("22.11.0"));
        assert_eq!(first_version("go version go1.27.3 linux/amd64").as_deref(), Some("1.27.3"));
        assert_eq!(first_version("openjdk version \"21.0.13\" 2025-10-21").as_deref(), Some("21.0.13"));
        assert_eq!(first_version("rustc 1.99.1 (5f94df478 2026-08-27)").as_deref(), Some("1.99.1"));
        assert_eq!(source_of(Path::new("/home/x/.local/share/mise/installs/node/22/bin/node")), "mise");
        assert_eq!(source_of(Path::new("/home/x/.nvm/versions/node/v20/bin/node")), "nvm");
        assert_eq!(source_of(Path::new("/usr/bin/python3")), "distro");
    }
}
