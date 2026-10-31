use crate::config::{data_dir, home};
use crate::detect::Env;
use crate::net::{self, Archive, Checksum};
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Uma linguagem que o devup instala da fonte oficial.
#[derive(Debug)]
pub struct Lang {
    pub id: &'static str,
    pub name: &'static str,
    /// Versão pedida quando ninguém diz qual.
    pub default: &'static str,
    /// Executáveis, relativos à pasta da versão, expostos em ~/.local/share/devup/bin.
    pub bins: &'static [&'static str],
    /// Precisa desta outra linguagem antes.
    pub requires: Option<&'static str>,
    pub source: &'static str,
}

pub const LANGS: [Lang; 11] = [
    Lang { id: "node", name: "Node.js", default: "lts", bins: &["bin/node", "bin/npm", "bin/npx", "bin/corepack"], requires: None, source: "nodejs.org" },
    Lang { id: "bun", name: "Bun", default: "latest", bins: &["bun"], requires: None, source: "github.com/oven-sh/bun" },
    Lang { id: "deno", name: "Deno", default: "latest", bins: &["deno"], requires: None, source: "github.com/denoland/deno" },
    Lang { id: "rust", name: "Rust", default: "stable", bins: &[], requires: None, source: "rustup.rs" },
    Lang { id: "go", name: "Go", default: "latest", bins: &["bin/go", "bin/gofmt"], requires: None, source: "go.dev" },
    Lang { id: "zig", name: "Zig", default: "latest", bins: &["zig"], requires: None, source: "ziglang.org" },
    Lang { id: "python", name: "Python (uv)", default: "3.13", bins: &[], requires: Some("uv"), source: "astral.sh (uv)" },
    Lang { id: "java", name: "Java (Temurin)", default: "lts", bins: &["bin/java", "bin/javac", "bin/jar", "bin/jshell"], requires: None, source: "adoptium.net" },
    Lang { id: "dotnet", name: ".NET", default: "lts", bins: &["dotnet"], requires: None, source: "dot.net" },
    Lang { id: "flutter", name: "Flutter", default: "stable", bins: &["bin/flutter", "bin/dart"], requires: None, source: "flutter.dev" },
    Lang { id: "kotlin", name: "Kotlin", default: "latest", bins: &["bin/kotlin", "bin/kotlinc"], requires: Some("java"), source: "github.com/JetBrains/kotlin" },
];

/// O uv é instalado como dependência do Python; não aparece na lista de linguagens.
pub const UV: Lang = Lang { id: "uv", name: "uv", default: "latest", bins: &["uv", "uvx"], requires: None, source: "github.com/astral-sh/uv" };

pub fn lang(id: &str) -> Option<&'static Lang> {
    LANGS.iter().find(|l| l.id == id).or((id == "uv").then_some(&UV))
}

/// "node@22" → ("node", "22"); "go" → ("go", "latest").
pub fn parse_spec(s: &str) -> Result<(&'static Lang, String), String> {
    let (id, version) = s.split_once('@').unwrap_or((s, ""));
    let l = lang(id).ok_or_else(|| {
        let ids = LANGS.iter().map(|l| l.id);
        format!("linguagem desconhecida: {id}{} · devup lang list mostra todas", crate::ui::did_you_mean(id, ids))
    })?;
    Ok((l, if version.is_empty() { l.default.to_string() } else { version.to_string() }))
}

/// O que baixar para instalar uma versão.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub lang: &'static str,
    pub version: String,
    pub url: String,
    pub file: String,
    pub sum: Checksum,
    pub archive: Archive,
    pub strip: usize,
}

#[derive(Clone, Copy)]
pub struct Platform {
    pub arm64: bool,
    pub musl: bool,
    pub avx2: bool,
}

impl Platform {
    pub fn from(env: &Env) -> Platform {
        Platform { arm64: env.arch == "aarch64", musl: env.musl, avx2: env.avx2 }
    }

    fn rust_triple(&self) -> String {
        format!("{}-unknown-linux-{}", if self.arm64 { "aarch64" } else { "x86_64" }, if self.musl { "musl" } else { "gnu" })
    }
}

fn no_build(lang: &str, what: &str) -> String {
    format!("{lang} não publica build oficial para {what}; use o pacote da distro (devup install {lang})")
}

/// "22" casa com "22.11.0"; "22.11" com "22.11.3"; "22.11.0" só com ela mesma.
fn matches(version: &str, spec: &str) -> bool {
    let version = version.trim_start_matches('v');
    version == spec || version.starts_with(&format!("{spec}."))
}

fn semver(v: &str) -> Vec<u64> {
    v.trim_start_matches('v').split(['.', '-', '+']).map_while(|p| p.parse().ok()).collect()
}

// ---------- resolvers (um por fonte oficial) ----------

#[derive(Deserialize)]
struct NodeRel {
    version: String,
    lts: Value,
    files: Vec<String>,
}

fn node(spec: &str, p: Platform, index: &[NodeRel], sums: &dyn Fn(&str) -> Result<String, String>) -> Result<Release, String> {
    if p.musl {
        return Err(no_build("node", "musl (Alpine)"));
    }
    let stable = index.iter().filter(|r| !r.version.contains('-'));
    let chosen = match spec {
        "lts" => stable.filter(|r| r.lts != Value::Bool(false)).next(),
        "latest" | "current" => stable.into_iter().next(),
        _ => stable.into_iter().find(|r| matches(&r.version, spec)),
    }
    .ok_or_else(|| format!("node {spec} não existe em nodejs.org"))?;
    let arch = if p.arm64 { "linux-arm64" } else { "linux-x64" };
    if !chosen.files.iter().any(|f| f == arch) {
        return Err(no_build("node", arch));
    }
    let file = format!("node-{}-{arch}.tar.gz", chosen.version);
    let sum = net::find_sum(&sums(&chosen.version)?, &file).ok_or("SHASUMS256.txt do node sem o arquivo")?;
    Ok(Release {
        lang: "node",
        version: chosen.version.trim_start_matches('v').to_string(),
        url: format!("https://nodejs.org/dist/{}/{file}", chosen.version),
        file,
        sum: Checksum::Sha256(sum),
        archive: Archive::TarGz,
        strip: 1,
    })
}

#[derive(Deserialize)]
struct GoRel {
    version: String,
    stable: bool,
    files: Vec<GoFile>,
}

#[derive(Deserialize)]
struct GoFile {
    filename: String,
    os: String,
    arch: String,
    sha256: String,
    kind: String,
}

fn go(spec: &str, p: Platform, index: &[GoRel]) -> Result<Release, String> {
    let chosen = index
        .iter()
        .filter(|r| r.stable)
        .find(|r| spec == "latest" || matches(r.version.trim_start_matches("go"), spec))
        .ok_or_else(|| format!("go {spec} não existe em go.dev"))?;
    let arch = if p.arm64 { "arm64" } else { "amd64" };
    let f = chosen
        .files
        .iter()
        .find(|f| f.os == "linux" && f.arch == arch && f.kind == "archive")
        .ok_or_else(|| no_build("go", arch))?;
    Ok(Release {
        lang: "go",
        version: chosen.version.trim_start_matches("go").to_string(),
        url: format!("https://go.dev/dl/{}", f.filename),
        file: f.filename.clone(),
        sum: Checksum::Sha256(f.sha256.clone()),
        archive: Archive::TarGz,
        strip: 1,
    })
}

fn zig(spec: &str, p: Platform, index: &Value) -> Result<Release, String> {
    let obj = index.as_object().ok_or("índice do zig inválido")?;
    let mut versions: Vec<&String> = obj.keys().filter(|k| *k != "master" && !k.contains('-')).collect();
    versions.sort_by_key(|v| semver(v));
    let version = match spec {
        "latest" | "stable" => versions.last().copied(),
        _ => versions.iter().rev().find(|v| matches(v, spec)).copied(),
    }
    .ok_or_else(|| format!("zig {spec} não existe em ziglang.org"))?;
    let key = if p.arm64 { "aarch64-linux" } else { "x86_64-linux" };
    let build = &obj[version][key];
    let url = build["tarball"].as_str().ok_or_else(|| no_build("zig", key))?;
    Ok(Release {
        lang: "zig",
        version: version.clone(),
        url: url.to_string(),
        file: url.rsplit('/').next().unwrap_or("zig.tar.xz").to_string(),
        sum: Checksum::Sha256(build["shasum"].as_str().unwrap_or_default().to_string()),
        archive: Archive::TarXz,
        strip: 1,
    })
}

/// Releases no GitHub: (tag, nomes dos assets → URLs).
pub struct GhRelease {
    pub tag: String,
    pub assets: Vec<(String, String)>,
    pub body: String,
}

impl GhRelease {
    pub fn from_json(v: &Value) -> GhRelease {
        GhRelease {
            tag: v["tag_name"].as_str().unwrap_or_default().to_string(),
            assets: v["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| Some((a["name"].as_str()?.to_string(), a["browser_download_url"].as_str()?.to_string())))
                .collect(),
            body: v["body"].as_str().unwrap_or_default().to_string(),
        }
    }

    pub fn asset(&self, name: &str) -> Option<&str> {
        self.assets.iter().find(|(n, _)| n == name).map(|(_, u)| u.as_str())
    }
}

pub fn github_release(repo: &str, tag: Option<&str>) -> Result<GhRelease, String> {
    let url = match tag {
        Some(t) => format!("https://api.github.com/repos/{repo}/releases/tags/{t}"),
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    Ok(GhRelease::from_json(&net::get_json::<Value>(&url)?))
}

/// Release de um projeto no GitHub cujo asset tem um checksum ao lado (`<asset><ext>`) ou num arquivo único.
fn from_github(lang: &'static str, rel: &GhRelease, file: &str, sums: SumSource, strip: usize, get: &dyn Fn(&str) -> Result<String, String>) -> Result<Release, String> {
    let url = rel.asset(file).ok_or_else(|| format!("{lang} {} não tem o arquivo {file}", rel.tag))?;
    let sums_url = match sums {
        SumSource::Beside(ext) => rel.asset(&format!("{file}{ext}")),
        SumSource::File(name) => rel.asset(name),
    }
    .ok_or_else(|| format!("{lang} {} sem checksum publicado", rel.tag))?;
    let sum = net::find_sum(&get(sums_url)?, file).ok_or_else(|| format!("checksum de {file} não encontrado"))?;
    let version = rel.tag.trim_start_matches("bun-").trim_start_matches('v').to_string();
    Ok(Release { lang, version, url: url.to_string(), file: file.to_string(), sum: Checksum::Sha256(sum), archive: Archive::from_name(file), strip })
}

enum SumSource {
    Beside(&'static str),
    File(&'static str),
}

fn bun_file(p: Platform) -> String {
    let arch = match (p.arm64, p.avx2) {
        (true, _) => "aarch64",
        (false, true) => "x64",
        (false, false) => "x64-baseline",
    };
    format!("bun-linux-{arch}{}.zip", if p.musl { "-musl" } else { "" })
}

fn deno_file(p: Platform) -> Result<String, String> {
    if p.musl {
        return Err(no_build("deno", "musl (Alpine)"));
    }
    Ok(format!("deno-{}-unknown-linux-gnu.zip", if p.arm64 { "aarch64" } else { "x86_64" }))
}

fn uv_file(p: Platform) -> String {
    format!("uv-{}.tar.gz", p.rust_triple())
}

fn java(spec: &str, p: Platform, available: &Value, assets_for: &dyn Fn(u64) -> Result<Value, String>) -> Result<Release, String> {
    let feature = match spec {
        "lts" => available["most_recent_lts"].as_u64(),
        "latest" => available["most_recent_feature_release"].as_u64(),
        _ => spec.split('.').next().and_then(|v| v.parse().ok()),
    }
    .ok_or_else(|| format!("java {spec} inválido; use lts, latest ou o número da versão (21)"))?;
    let list = assets_for(feature)?;
    let first = list.as_array().and_then(|a| a.first()).ok_or_else(|| no_build("java", if p.arm64 { "aarch64" } else { "x64" }))?;
    let pkg = &first["binary"]["package"];
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    Ok(Release {
        lang: "java",
        version: s(&first["version"]["semver"]),
        url: s(&pkg["link"]),
        file: s(&pkg["name"]),
        sum: Checksum::Sha256(s(&pkg["checksum"])),
        archive: Archive::TarGz,
        strip: 1,
    })
}

fn java_assets_url(feature: u64, p: Platform) -> String {
    format!(
        "https://api.adoptium.net/v3/assets/latest/{feature}/hotspot?architecture={}&image_type=jdk&os={}&vendor=eclipse",
        if p.arm64 { "aarch64" } else { "x64" },
        if p.musl { "alpine-linux" } else { "linux" }
    )
}

fn dotnet(spec: &str, p: Platform, index: &Value, releases_for: &dyn Fn(&str) -> Result<Value, String>) -> Result<Release, String> {
    let channels = index["releases-index"].as_array().ok_or("índice do .NET inválido")?;
    let supported = |c: &&Value| matches!(c["support-phase"].as_str(), Some("active" | "maintenance"));
    let channel = match spec {
        "lts" => channels.iter().filter(supported).find(|c| c["release-type"] == "lts"),
        "latest" | "sts" => channels.iter().find(supported),
        _ => channels.iter().find(|c| c["channel-version"].as_str() == Some(spec)),
    }
    .ok_or_else(|| format!(".NET {spec} não existe; use lts, latest ou um canal como 10.0"))?;
    let sdk_version = channel["latest-sdk"].as_str().unwrap_or_default();
    let releases = releases_for(channel["releases.json"].as_str().unwrap_or_default())?;
    let rid = format!("linux-{}{}", if p.musl { "musl-" } else { "" }, if p.arm64 { "arm64" } else { "x64" });
    let wanted = format!("dotnet-sdk-{rid}.tar.gz");
    let file = releases["releases"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|r| r["sdks"].as_array().cloned().unwrap_or_else(|| vec![r["sdk"].clone()]))
        .filter(|sdk| sdk["version"].as_str() == Some(sdk_version))
        .flat_map(|sdk| sdk["files"].as_array().cloned().unwrap_or_default())
        .find(|f| f["name"].as_str() == Some(&wanted))
        .ok_or_else(|| no_build("dotnet", &rid))?;
    Ok(Release {
        lang: "dotnet",
        version: sdk_version.to_string(),
        url: file["url"].as_str().unwrap_or_default().to_string(),
        file: wanted,
        sum: Checksum::Sha512(file["hash"].as_str().unwrap_or_default().to_string()),
        archive: Archive::TarGz,
        strip: 0,
    })
}

fn flutter(spec: &str, p: Platform, index: &Value) -> Result<Release, String> {
    if p.arm64 || p.musl {
        return Err(no_build("flutter", if p.arm64 { "aarch64" } else { "musl (Alpine)" }));
    }
    let releases = index["releases"].as_array().ok_or("índice do Flutter inválido")?;
    let chosen = match spec {
        "stable" | "beta" | "latest" => {
            let channel = if spec == "beta" { "beta" } else { "stable" };
            let hash = &index["current_release"][channel];
            releases.iter().find(|r| &r["hash"] == hash && r["channel"] == channel)
        }
        _ => releases.iter().find(|r| r["channel"] == "stable" && r["version"].as_str().is_some_and(|v| matches(v, spec))),
    }
    .ok_or_else(|| format!("flutter {spec} não existe; use stable, beta ou uma versão como 3.47.9"))?;
    let archive = chosen["archive"].as_str().unwrap_or_default();
    Ok(Release {
        lang: "flutter",
        version: chosen["version"].as_str().unwrap_or_default().to_string(),
        url: format!("{}/{archive}", index["base_url"].as_str().unwrap_or_default()),
        file: archive.rsplit('/').next().unwrap_or(archive).to_string(),
        sum: Checksum::Sha256(chosen["sha256"].as_str().unwrap_or_default().to_string()),
        archive: Archive::from_name(archive),
        strip: 1,
    })
}

fn rust(p: Platform, sha_text: &str) -> Result<Release, String> {
    let triple = p.rust_triple();
    Ok(Release {
        lang: "rust",
        version: String::new(),
        url: format!("https://static.rust-lang.org/rustup/dist/{triple}/rustup-init"),
        file: "rustup-init".into(),
        sum: Checksum::Sha256(net::find_sum(sha_text, "rustup-init").ok_or("rustup-init.sha256 inválido")?),
        archive: Archive::Binary,
        strip: 0,
    })
}

fn github_tag(spec: &str, prefix: &str) -> Option<String> {
    (spec != "latest").then(|| format!("{prefix}{}", spec.trim_start_matches('v')))
}

/// Consulta a fonte oficial e descobre o que baixar. Python e Rust devolvem o instalador (uv / rustup-init).
pub fn resolve(id: &str, spec: &str, p: Platform) -> Result<Release, String> {
    match id {
        "node" => {
            let index: Vec<NodeRel> = net::get_json("https://nodejs.org/dist/index.json")?;
            node(spec, p, &index, &|v| net::get_text(&format!("https://nodejs.org/dist/{v}/SHASUMS256.txt")))
        }
        "go" => {
            let url = if spec == "latest" { "https://go.dev/dl/?mode=json" } else { "https://go.dev/dl/?mode=json&include=all" };
            go(spec, p, &net::get_json::<Vec<GoRel>>(url)?)
        }
        "zig" => zig(spec, p, &net::get_json("https://ziglang.org/download/index.json")?),
        "bun" => {
            let rel = github_release("oven-sh/bun", github_tag(spec, "bun-v").as_deref())?;
            from_github("bun", &rel, &bun_file(p), SumSource::File("SHASUMS256.txt"), 1, &net::get_text)
        }
        "deno" => {
            let rel = github_release("denoland/deno", github_tag(spec, "v").as_deref())?;
            from_github("deno", &rel, &deno_file(p)?, SumSource::Beside(".sha256sum"), 0, &net::get_text)
        }
        "uv" | "python" => {
            let rel = github_release("astral-sh/uv", github_tag(if id == "uv" { spec } else { "latest" }, "").as_deref())?;
            from_github("uv", &rel, &uv_file(p), SumSource::Beside(".sha256"), 1, &net::get_text)
        }
        "kotlin" => {
            let rel = github_release("JetBrains/kotlin", github_tag(spec, "v").as_deref())?;
            let version = rel.tag.trim_start_matches('v').to_string();
            from_github("kotlin", &rel, &format!("kotlin-compiler-{version}.zip"), SumSource::Beside(".sha256"), 1, &net::get_text)
        }
        "java" => {
            let available = net::get_json("https://api.adoptium.net/v3/info/available_releases")?;
            java(spec, p, &available, &|f| net::get_json(&java_assets_url(f, p)))
        }
        "dotnet" => {
            let index = net::get_json("https://builds.dotnet.microsoft.com/dotnet/release-metadata/releases-index.json")?;
            dotnet(spec, p, &index, &|url| net::get_json(url))
        }
        "flutter" => flutter(spec, p, &net::get_json("https://storage.googleapis.com/flutter_infra_release/releases/releases_linux.json")?),
        "rust" => {
            let triple = p.rust_triple();
            rust(p, &net::get_text(&format!("https://static.rust-lang.org/rustup/dist/{triple}/rustup-init.sha256"))?)
        }
        other => Err(format!("linguagem desconhecida: {other}")),
    }
}

// ---------- instalação, versão atual e PATH ----------

pub fn root() -> PathBuf {
    data_dir().join("toolchains")
}

pub fn bin_dir() -> PathBuf {
    data_dir().join("bin")
}

fn version_dir(id: &str, version: &str) -> PathBuf {
    root().join(id).join(version)
}

const DONE: &str = ".devup-ok";

/// Versões instaladas (mais nova primeiro) e a atual.
pub fn installed(id: &str) -> (Vec<String>, Option<String>) {
    let dir = root().join(id);
    let mut versions: Vec<String> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(DONE).exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    versions.sort_by_key(|v| std::cmp::Reverse(semver(v)));
    let current = fs::read_link(dir.join("current")).ok().and_then(|p| p.file_name()?.to_str().map(String::from));
    (versions, current)
}

/// Extrai uma release já baixada e verificada. Idempotente.
pub fn unpack(rel: &Release, archive: &Path) -> Result<PathBuf, String> {
    let dir = version_dir(rel.lang, &rel.version);
    if dir.join(DONE).exists() {
        return Ok(dir);
    }
    let tmp = dir.with_extension("partial");
    let _ = fs::remove_dir_all(&tmp);
    net::extract(archive, rel.archive, &tmp, rel.strip, &rel.file)?;
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&tmp, &dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(DONE), &rel.url).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn replace_symlink(target: &Path, link: &Path) -> Result<(), String> {
    let tmp = link.with_extension("devup-tmp");
    let _ = fs::remove_file(&tmp);
    symlink(target, &tmp).map_err(|e| e.to_string())?;
    fs::rename(&tmp, link).map_err(|e| format!("{}: {e}", link.display()))
}

/// Torna `version` a atual: link `current` e links dos executáveis em ~/.local/share/devup/bin.
pub fn activate(id: &str, version: &str) -> Result<(), String> {
    let l = lang(id).ok_or("linguagem desconhecida")?;
    let current = root().join(id).join("current");
    replace_symlink(&version_dir(id, version), &current)?;
    fs::create_dir_all(bin_dir()).map_err(|e| e.to_string())?;
    for bin in l.bins {
        let name = Path::new(bin).file_name().unwrap();
        replace_symlink(&current.join(bin), &bin_dir().join(name))?;
    }
    if id == "bun" {
        replace_symlink(&current.join("bun"), &bin_dir().join("bunx"))?;
    }
    write_env()
}

pub fn remove(id: &str, version: &str) -> Result<(), String> {
    let (_, current) = installed(id);
    if current.as_deref() == Some(version) {
        let l = lang(id).ok_or("linguagem desconhecida")?;
        for bin in l.bins {
            let _ = fs::remove_file(bin_dir().join(Path::new(bin).file_name().unwrap()));
        }
        let _ = fs::remove_file(root().join(id).join("current"));
    }
    fs::remove_dir_all(version_dir(id, version)).map_err(|e| e.to_string())
}

/// Roda um instalador/gerenciador oficial (rustup-init, uv) com a saída capturada.
fn run_quiet(cmd: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd).args(args).stdin(Stdio::null()).output().map_err(|e| format!("{}: {e}", cmd.display()))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        let tail: Vec<&str> = text.lines().rev().take(8).collect();
        Err(format!("{} falhou:\n    {}", cmd.display(), tail.into_iter().rev().collect::<Vec<_>>().join("\n    ")))
    }
}

/// Rust: rustup-init (já verificado) instala o toolchain pedido em ~/.cargo e ~/.rustup.
pub fn install_rust(rustup_init: &Path, spec: &str) -> Result<String, String> {
    let rustup = home().join(".cargo/bin/rustup");
    // SAFETY: só lido pelos processos filhos do rustup; nenhuma outra thread lê esta variável.
    unsafe { std::env::set_var("RUSTUP_MAX_RETRIES", "8") };
    if rustup.exists() {
        run_quiet(&rustup, &["toolchain", "install", spec, "--profile", "default"])?;
        run_quiet(&rustup, &["default", spec])?;
    } else {
        run_quiet(rustup_init, &["-y", "--no-modify-path", "--profile", "default", "--default-toolchain", spec])?;
    }
    write_env()?;
    let out = run_quiet(&home().join(".cargo/bin/rustc"), &["--version"])?;
    Ok(out.split_whitespace().nth(1).unwrap_or(spec).to_string())
}

/// Python: o uv (da devup) instala o CPython pedido e o devup expõe `python` e `python3`.
pub fn install_python(spec: &str) -> Result<String, String> {
    let uv = bin_dir().join("uv");
    let spec = if spec == "latest" { "3" } else { spec };
    run_quiet(&uv, &["python", "install", spec])?;
    let path = run_quiet(&uv, &["python", "find", spec])?.trim().to_string();
    for name in ["python", "python3"] {
        replace_symlink(Path::new(&path), &bin_dir().join(name))?;
    }
    write_env()?;
    let out = run_quiet(Path::new(&path), &["--version"])?;
    Ok(out.split_whitespace().nth(1).unwrap_or(spec).to_string())
}

pub fn env_sh() -> PathBuf {
    data_dir().join("env.sh")
}

/// Arquivos que os shells carregam: PATH e variáveis que algumas linguagens pedem.
pub fn write_env() -> Result<(), String> {
    let (bin, cargo, root) = (bin_dir(), home().join(".cargo/bin"), root());
    let sh = format!(
        "# Gerado pelo devup; não edite. Carregue com: . {env}\n\
         for d in \"{cargo}\" \"{bin}\"; do\n  case \":$PATH:\" in *\":$d:\"*) ;; *) [ -d \"$d\" ] && PATH=\"$d:$PATH\" ;; esac\ndone\nexport PATH\n\
         [ -d \"{root}/java/current\" ] && export JAVA_HOME=\"{root}/java/current\"\n\
         [ -d \"{root}/dotnet/current\" ] && export DOTNET_ROOT=\"{root}/dotnet/current\"\n",
        env = env_sh().display(),
        cargo = cargo.display(),
        bin = bin.display(),
        root = root.display(),
    );
    let fish = format!(
        "# Gerado pelo devup; não edite.\n\
         for d in {cargo} {bin}\n    test -d $d; and fish_add_path -g $d\nend\n\
         test -d {root}/java/current; and set -gx JAVA_HOME {root}/java/current\n\
         test -d {root}/dotnet/current; and set -gx DOTNET_ROOT {root}/dotnet/current\n",
        cargo = cargo.display(),
        bin = bin.display(),
        root = root.display(),
    );
    fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    fs::write(env_sh(), sh).map_err(|e| e.to_string())?;
    fs::write(data_dir().join("env.fish"), fish).map_err(|e| e.to_string())
}

/// O PATH atual já enxerga os binários do devup?
pub fn on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == bin_dir()))
}

/// Bibliotecas de sistema que os binários oficiais esperam e containers/instalações mínimas não têm.
/// (trecho do erro, pacote em: pacman, apt, dnf, zypper, apk, xbps)
const SYSTEM_LIBS: [(&str, [&str; 6]); 5] = [
    ("libatomic.so", ["gcc-libs", "libatomic1", "libatomic", "libatomic1", "libatomic", "libatomic"]),
    ("libstdc++.so", ["gcc-libs", "libstdc++6", "libstdc++", "libstdc++6", "libstdc++", "libstdc++"]),
    ("libgcc_s.so", ["gcc-libs", "libgcc-s1", "libgcc", "libgcc_s1", "libgcc", "libgcc"]),
    ("ICU", ["icu", "libicu-dev", "libicu", "libicu", "icu-libs", "icu"]),
    ("libz.so", ["zlib", "zlib1g", "zlib-ng-compat", "libz1", "zlib", "zlib"]),
];

/// Pacotes da distro que resolvem as bibliotecas citadas no erro.
pub fn packages_for(error: &str, pm: crate::pm::Pm) -> Vec<&'static str> {
    use crate::pm::Pm;
    let col = match pm {
        Pm::Pacman => 0,
        Pm::Apt => 1,
        Pm::Dnf => 2,
        Pm::Zypper => 3,
        Pm::Apk => 4,
        Pm::Xbps => 5,
    };
    let mut pkgs: Vec<&str> = SYSTEM_LIBS.iter().filter(|(needle, _)| error.contains(needle)).map(|(_, p)| p[col]).collect();
    pkgs.dedup();
    pkgs
}

/// Roda a linguagem uma vez para ter certeza de que ela funciona nesta máquina.
pub fn smoke(id: &str) -> Result<(), String> {
    let (bin, args): (&str, &[&str]) = match id {
        "node" => ("node", &["--version"]),
        "bun" => ("bun", &["--version"]),
        "deno" => ("deno", &["--version"]),
        "go" => ("go", &["version"]),
        "zig" => ("zig", &["version"]),
        "java" => ("java", &["-version"]),
        "dotnet" => ("dotnet", &["--version"]),
        "uv" => ("uv", &["--version"]),
        _ => return Ok(()),
    };
    let out = Command::new(bin_dir().join(bin))
        .args(args)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }
}

/// Já existe alguma versão desta linguagem instalada pelo devup?
pub fn is_installed(id: &str) -> bool {
    match id {
        "rust" => home().join(".cargo/bin/rustc").exists(),
        "python" => bin_dir().join("python3").exists(),
        _ => installed(id).1.is_some(),
    }
}

/// Linguagens pedidas mais as dependências que faltam, em ondas: dependências primeiro.
pub fn waves(reqs: &[(String, String)]) -> Vec<Vec<(String, String)>> {
    let mut first: Vec<(String, String)> = vec![];
    let mut second: Vec<(String, String)> = vec![];
    for (id, spec) in reqs {
        match lang(id).and_then(|l| l.requires) {
            Some(dep) => {
                let asked = reqs.iter().any(|(i, _)| i == dep) || first.iter().any(|(i, _)| i == dep);
                if !asked && !is_installed(dep) {
                    first.push((dep.to_string(), lang(dep).unwrap().default.to_string()));
                }
                second.push((id.clone(), spec.clone()));
            }
            None => first.push((id.clone(), spec.clone())),
        }
    }
    [first, second].into_iter().filter(|w| !w.is_empty()).collect()
}

/// Resolve, baixa, verifica, extrai e ativa uma linguagem, atualizando a barra dela.
fn install_one(id: &str, spec: &str, p: Platform, pb: &indicatif::ProgressBar, dry_run: bool) -> Result<String, String> {
    let l = lang(id).ok_or("linguagem desconhecida")?;
    pb.set_style(crate::ui::spinner_style());
    pb.set_prefix("Consultando");
    pb.set_message(format!("{} em {}", l.name, l.source));
    if id == "python" {
        if dry_run {
            return Ok(format!("{spec} via uv python install"));
        }
        pb.set_prefix("Instalando");
        pb.set_message(format!("Python {spec} (uv)"));
        return install_python(spec);
    }
    if id == "rust" && home().join(".cargo/bin/rustup").exists() {
        if dry_run {
            return Ok(format!("{spec} via rustup"));
        }
        pb.set_prefix("Instalando");
        pb.set_message(format!("Rust {spec} (rustup)"));
        return install_rust(Path::new(""), spec);
    }
    let rel = resolve(id, spec, p)?;
    let label = if rel.version.is_empty() { l.name.to_string() } else { format!("{} {}", l.name, rel.version) };
    if dry_run {
        return Ok(format!("{} ← {} (sha {}…)", if rel.version.is_empty() { spec } else { &rel.version }, rel.url, &rel.sum.hex()[..12]));
    }
    if !rel.version.is_empty() && version_dir(id, &rel.version).join(DONE).exists() {
        activate(id, &rel.version)?;
        return Ok(format!("{} (já estava instalado)", rel.version));
    }
    pb.set_style(crate::ui::bar_style());
    pb.set_prefix("Baixando");
    pb.set_message(label.clone());
    let file = net::fetch(&rel.url, &rel.file, &rel.sum, pb)?;
    pb.set_style(crate::ui::spinner_style());
    pb.set_prefix("Instalando");
    pb.set_message(format!("{label} · checksum ok"));
    if id == "rust" {
        let dir = root().join("rust");
        net::extract(&file, Archive::Binary, &dir, 0, "rustup-init")?;
        pb.set_message(format!("Rust {spec} (rustup)"));
        return install_rust(&dir.join("rustup-init"), spec);
    }
    unpack(&rel, &file)?;
    activate(id, &rel.version)?;
    Ok(rel.version)
}

/// Resultado de uma linguagem: versão instalada ou erro, e o erro do teste rápido se ela não rodou.
pub struct Outcome {
    pub id: String,
    pub result: Result<String, String>,
    pub broken: Option<String>,
}

const PARALLEL: usize = 4;

/// Instala várias linguagens, até 4 ao mesmo tempo, com uma linha animada por linguagem.
pub fn ensure(reqs: &[(String, String)], p: Platform, dry_run: bool) -> Vec<Outcome> {
    use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget};
    use std::sync::{Arc, Mutex};
    let mp = MultiProgress::new();
    if !crate::ui::animated() {
        mp.set_draw_target(ProgressDrawTarget::hidden());
    }
    let mut results = vec![];
    for wave in waves(reqs) {
        // Fila: cada linguagem já aparece "na fila" e os workers pegam a próxima.
        let queue: Arc<Mutex<Vec<(String, String, ProgressBar)>>> = Arc::new(Mutex::new(
            wave.into_iter()
                .rev()
                .map(|(id, spec)| {
                    let pb = mp.add(ProgressBar::new_spinner());
                    pb.set_style(crate::ui::spinner_style());
                    pb.set_prefix("Na fila");
                    pb.set_message(lang(&id).map_or(id.clone(), |l| l.name.to_string()));
                    pb.enable_steady_tick(std::time::Duration::from_millis(90));
                    (id, spec, pb)
                })
                .collect(),
        ));
        let workers: Vec<_> = (0..PARALLEL)
            .map(|_| {
                let queue = Arc::clone(&queue);
                let mp = mp.clone();
                std::thread::spawn(move || {
                    let mut done = vec![];
                    loop {
                        let Some((id, spec, pb)) = queue.lock().unwrap().pop() else { break };
                        let start = std::time::Instant::now();
                        let result = install_one(&id, &spec, p, &pb, dry_run);
                        pb.finish_and_clear();
                        let broken = if dry_run || result.is_err() { None } else { smoke(&id).err() };
                        let name = lang(&id).map_or(id.as_str(), |l| l.name);
                        let took = console::style(format!("{:.1}s", start.elapsed().as_secs_f32())).dim();
                        // Imprime com as barras pausadas, para não sobrar pedaço de animação na tela.
                        mp.suspend(|| match (&result, &broken) {
                            (Ok(v), None) => crate::ui::status(if dry_run { "Instalaria" } else { "Pronto" }, &format!("{name} {v} {took}")),
                            (Ok(v), Some(_)) => crate::ui::warn(&format!("{name} {v} instalado, mas ainda não roda (vou ver o que falta)")),
                            (Err(e), _) => crate::ui::error(&format!("{name}: {e}")),
                        });
                        done.push(Outcome { id, result, broken });
                    }
                    done
                })
            })
            .collect();
        for w in workers {
            results.extend(w.join().unwrap_or_default());
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    const X64: Platform = Platform { arm64: false, musl: false, avx2: true };
    const ARM: Platform = Platform { arm64: true, musl: false, avx2: false };
    const MUSL: Platform = Platform { arm64: false, musl: true, avx2: false };

    #[test]
    fn specs() {
        let (l, v) = parse_spec("node@22").unwrap();
        assert_eq!((l.id, v.as_str()), ("node", "22"));
        assert_eq!(parse_spec("go").unwrap().1, "latest");
        assert!(parse_spec("noed").unwrap_err().contains("node"));
        assert!(matches("v22.11.0", "22") && matches("go1.23.4", "go1.23") && !matches("22.11.0", "2"));
    }

    #[test]
    fn node_picks_lts_latest_and_exact_ignoring_prereleases() {
        let index: Vec<NodeRel> = serde_json::from_str(
            r#"[{"version":"v27.0.0-alpha.0","lts":false,"files":["linux-x64"]},
                {"version":"v26.14.0","lts":false,"files":["linux-x64","linux-arm64"]},
                {"version":"v24.11.0","lts":"Krypton","files":["linux-x64","linux-arm64"]},
                {"version":"v22.20.0","lts":"Jod","files":["linux-x64"]}]"#,
        )
        .unwrap();
        let sums = |v: &str| Ok(format!("abc  node-{v}-linux-x64.tar.gz\ndef  node-{v}-linux-arm64.tar.gz\n"));
        assert_eq!(node("latest", X64, &index, &sums).unwrap().version, "26.14.0");
        let lts = node("lts", ARM, &index, &sums).unwrap();
        assert_eq!((lts.version.as_str(), lts.sum.hex()), ("24.11.0", "def"));
        assert_eq!(lts.url, "https://nodejs.org/dist/v24.11.0/node-v24.11.0-linux-arm64.tar.gz");
        assert_eq!(node("22", X64, &index, &sums).unwrap().version, "22.20.0");
        assert!(node("22", ARM, &index, &sums).is_err());
        assert!(node("lts", MUSL, &index, &sums).unwrap_err().contains("musl"));
    }

    #[test]
    fn go_zig_java_dotnet_flutter_resolve_from_official_metadata() {
        let goidx: Vec<GoRel> = serde_json::from_str(
            r#"[{"version":"go1.27.3","stable":true,"files":[{"filename":"go1.27.3.linux-amd64.tar.gz","os":"linux","arch":"amd64","sha256":"aa","kind":"archive"},{"filename":"go1.27.3.src.tar.gz","os":"","arch":"","sha256":"bb","kind":"source"}]},
                {"version":"go1.26.9","stable":true,"files":[{"filename":"go1.26.9.linux-arm64.tar.gz","os":"linux","arch":"arm64","sha256":"cc","kind":"archive"}]}]"#,
        )
        .unwrap();
        assert_eq!(go("latest", X64, &goidx).unwrap().url, "https://go.dev/dl/go1.27.3.linux-amd64.tar.gz");
        assert_eq!(go("1.26", ARM, &goidx).unwrap().sum.hex(), "cc");

        let zigidx: Value = serde_json::from_str(
            r#"{"master":{},"0.9.1":{"x86_64-linux":{"tarball":"https://z/0.9.1.tar.xz","shasum":"old"}},
                "0.17.0":{"x86_64-linux":{"tarball":"https://ziglang.org/download/0.17.0/zig-x86_64-linux-0.17.0.tar.xz","shasum":"new"}}}"#,
        )
        .unwrap();
        let z = zig("latest", X64, &zigidx).unwrap();
        assert_eq!((z.version.as_str(), z.file.as_str(), z.archive), ("0.17.0", "zig-x86_64-linux-0.17.0.tar.xz", Archive::TarXz));

        let available: Value = serde_json::from_str(r#"{"most_recent_lts":25,"most_recent_feature_release":27}"#).unwrap();
        let assets = |f: u64| -> Result<Value, String> {
            Ok(serde_json::from_str(&format!(r#"[{{"version":{{"semver":"{f}.0.1+9"}},"binary":{{"package":{{"link":"https://github.com/adoptium/x.tar.gz","name":"OpenJDK{f}.tar.gz","checksum":"j{f}"}}}}}}]"#)).unwrap())
        };
        assert_eq!(java("lts", X64, &available, &assets).unwrap().sum.hex(), "j25");
        assert_eq!(java("21", X64, &available, &assets).unwrap().version, "21.0.1+9");
        assert!(java_assets_url(21, MUSL).contains("os=alpine-linux"));

        let idx: Value = serde_json::from_str(
            r#"{"releases-index":[{"channel-version":"11.0","support-phase":"go-live","release-type":"lts","latest-sdk":"11.0.100-rc.3","releases.json":"u11"},
                {"channel-version":"10.0","support-phase":"active","release-type":"lts","latest-sdk":"10.0.403","releases.json":"u10"},
                {"channel-version":"9.0","support-phase":"maintenance","release-type":"sts","latest-sdk":"9.0.320","releases.json":"u9"}]}"#,
        )
        .unwrap();
        let releases = |u: &str| -> Result<Value, String> {
            let v = if u == "u10" { "10.0.403" } else { "9.0.320" };
            Ok(serde_json::from_str(&format!(r#"{{"releases":[{{"sdks":[{{"version":"{v}","files":[{{"name":"dotnet-sdk-linux-x64.tar.gz","url":"https://builds.dotnet.microsoft.com/{v}.tar.gz","hash":"h{v}"}},{{"name":"dotnet-sdk-linux-musl-x64.tar.gz","url":"m","hash":"m"}}]}}]}}]}}"#)).unwrap())
        };
        let d = dotnet("lts", X64, &idx, &releases).unwrap();
        assert_eq!((d.version.as_str(), d.sum.clone()), ("10.0.403", Checksum::Sha512("h10.0.403".into())));
        assert_eq!(dotnet("9.0", MUSL, &idx, &releases).unwrap().url, "m");
        assert!(dotnet("lts", ARM, &idx, &releases).is_err());

        let fl: Value = serde_json::from_str(
            r#"{"base_url":"https://storage.googleapis.com/flutter_infra_release/releases","current_release":{"stable":"s1","beta":"b1"},
                "releases":[{"hash":"b1","channel":"beta","version":"3.48.0-1.pre","archive":"beta/linux/f-beta.tar.xz","sha256":"bb"},
                {"hash":"s1","channel":"stable","version":"3.47.9","archive":"stable/linux/flutter_linux_3.47.9-stable.tar.xz","sha256":"ss"}]}"#,
        )
        .unwrap();
        let f = flutter("stable", X64, &fl).unwrap();
        assert_eq!((f.version.as_str(), f.file.as_str()), ("3.47.9", "flutter_linux_3.47.9-stable.tar.xz"));
        assert_eq!(flutter("beta", X64, &fl).unwrap().sum.hex(), "bb");
        assert!(flutter("stable", ARM, &fl).is_err());
    }

    #[test]
    fn github_projects_pick_the_right_asset_and_checksum() {
        let rel = GhRelease::from_json(
            &serde_json::from_str(
                r#"{"tag_name":"bun-v1.4.4","body":"notas","assets":[
                    {"name":"bun-linux-x64.zip","browser_download_url":"https://g/x64.zip"},
                    {"name":"bun-linux-x64-baseline.zip","browser_download_url":"https://g/base.zip"},
                    {"name":"SHASUMS256.txt","browser_download_url":"https://g/sums"}]}"#,
            )
            .unwrap(),
        );
        let get = |_: &str| Ok("11  bun-linux-x64.zip\n22  bun-linux-x64-baseline.zip\n".to_string());
        let r = from_github("bun", &rel, &bun_file(X64), SumSource::File("SHASUMS256.txt"), 1, &get).unwrap();
        assert_eq!((r.version.as_str(), r.url.as_str(), r.sum.hex()), ("1.4.4", "https://g/x64.zip", "11"));
        let no_avx2 = Platform { avx2: false, ..X64 };
        assert_eq!(from_github("bun", &rel, &bun_file(no_avx2), SumSource::File("SHASUMS256.txt"), 1, &get).unwrap().sum.hex(), "22");
        assert_eq!(bun_file(MUSL), "bun-linux-x64-baseline-musl.zip");
        assert!(deno_file(MUSL).is_err());
        assert_eq!(uv_file(ARM), "uv-aarch64-unknown-linux-gnu.tar.gz");
        assert_eq!(github_tag("1.2.3", "bun-v").as_deref(), Some("bun-v1.2.3"));
        assert_eq!(rust(MUSL, "ff *./rustup-init").unwrap().url, "https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-musl/rustup-init");
    }

    #[test]
    fn dependencies_install_first() {
        let w = waves(&[("kotlin".into(), "latest".into()), ("node".into(), "lts".into()), ("java".into(), "21".into())]);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0], [("node".to_string(), "lts".to_string()), ("java".to_string(), "21".to_string())]);
        assert_eq!(w[1], [("kotlin".to_string(), "latest".to_string())]);
    }

    #[test]
    fn maps_missing_libraries_to_distro_packages() {
        use crate::pm::Pm;
        let node_err = "node: error while loading shared libraries: libatomic.so.1: cannot open shared object file";
        assert_eq!(packages_for(node_err, Pm::Apt), ["libatomic1"]);
        assert_eq!(packages_for(node_err, Pm::Dnf), ["libatomic"]);
        let bun_err = "Error loading shared library libstdc++.so.6: No such file\nError relocating: libgcc_s.so.1";
        assert_eq!(packages_for(bun_err, Pm::Apk), ["libstdc++", "libgcc"]);
        assert_eq!(packages_for("Process terminated. Couldn't find a valid ICU package installed on the system.", Pm::Apt), ["libicu-dev"]);
        assert!(packages_for("tudo certo", Pm::Apt).is_empty());
    }
}
