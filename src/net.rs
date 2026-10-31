use crate::config::home;
use indicatif::ProgressBar;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256, Sha512};
use std::{
    env,
    fs::{self, File},
    io::{self, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

const USER_AGENT: &str = concat!("devup/", env!("CARGO_PKG_VERSION"), " (+https://github.com/bernardofernandezz/devup)");

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .user_agent(USER_AGENT)
            .build()
            .into()
    })
}

/// Falhas de rede passageiras (DNS, conexão, timeout) merecem nova tentativa; 4xx não.
fn transient(e: &ureq::Error) -> bool {
    matches!(e, ureq::Error::Io(_) | ureq::Error::Timeout(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed)
        || matches!(e, ureq::Error::StatusCode(code) if *code >= 500)
}

const ATTEMPTS: u32 = 4;

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    get_range(url, 0)
}

/// GET com novas tentativas (1s, 2s, 4s); `from > 0` pede só o resto do arquivo.
fn get_range(url: &str, from: u64) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut attempt = 1;
    loop {
        match try_get(url, from) {
            Ok(r) => return Ok(r),
            Err((e, retry)) if retry && attempt < ATTEMPTS => {
                let _ = e;
                std::thread::sleep(Duration::from_secs(1 << (attempt - 1)));
                attempt += 1;
            }
            Err((e, _)) => return Err(e),
        }
    }
}

fn try_get(url: &str, from: u64) -> Result<ureq::http::Response<ureq::Body>, (String, bool)> {
    let mut req = agent().get(url);
    if from > 0 {
        req = req.header("Range", &format!("bytes={from}-"));
    }
    // Limite da API do GitHub sem token é baixo (60/h); com token, 5000/h.
    if url.starts_with("https://api.github.com/")
        && let Ok(token) = env::var("GITHUB_TOKEN").or_else(|_| env::var("GH_TOKEN"))
    {
        req = req.header("Authorization", &format!("Bearer {token}"));
    }
    req.call().map_err(|e| {
        let retry = transient(&e);
        let msg = match e {
            ureq::Error::StatusCode(403 | 429) if url.contains("api.github.com") => {
                "limite de consultas à API do GitHub atingido; tente em alguns minutos ou defina GITHUB_TOKEN".to_string()
            }
            ureq::Error::StatusCode(404) => format!("não encontrado: {url}"),
            other => format!("{url}: {other}"),
        };
        (msg, retry)
    })
}

pub fn get_text(url: &str) -> Result<String, String> {
    get(url)?.body_mut().with_config().limit(64 << 20).read_to_string().map_err(|e| format!("{url}: {e}"))
}

pub fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    get(url)?.body_mut().with_config().limit(64 << 20).read_json().map_err(|e| format!("{url}: {e}"))
}

// ---------- checksum ----------

#[derive(Debug, Clone, PartialEq)]
pub enum Checksum {
    Sha256(String),
    Sha512(String),
}

impl Checksum {
    pub fn hex(&self) -> &str {
        match self {
            Checksum::Sha256(h) | Checksum::Sha512(h) => h,
        }
    }

    fn compute(&self, path: &Path) -> io::Result<String> {
        fn hash<D: Digest>(mut d: D, path: &Path) -> io::Result<String> {
            let mut file = BufReader::new(File::open(path)?);
            let mut buf = vec![0; 1 << 16];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                d.update(&buf[..n]);
            }
            Ok(d.finalize().iter().map(|b| format!("{b:02x}")).collect())
        }
        match self {
            Checksum::Sha256(_) => hash(Sha256::new(), path),
            Checksum::Sha512(_) => hash(Sha512::new(), path),
        }
    }

    pub fn verify(&self, path: &Path) -> Result<(), String> {
        let got = self.compute(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if got.eq_ignore_ascii_case(self.hex()) {
            Ok(())
        } else {
            Err(format!("checksum não confere para {} (esperado {}…, veio {}…)", path.display(), &self.hex()[..12], &got[..12]))
        }
    }
}

/// O hash de `file` num arquivo de checksums ("<hash>  <arquivo>" ou "<hash> *<arquivo>"),
/// ou o primeiro hash se o arquivo tiver um só.
pub fn find_sum(text: &str, file: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let named = lines.iter().find_map(|l| {
        let mut parts = l.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*').trim_start_matches("./");
        (name == file).then(|| hash.to_string())
    });
    named.or_else(|| (lines.len() == 1).then(|| lines[0].split_whitespace().next().unwrap_or_default().to_string()))
}

// ---------- download ----------

pub fn cache_dir() -> PathBuf {
    env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache")).join("devup/downloads")
}

/// Baixa para o cache (ou reaproveita o que já está lá) e confere o checksum antes de devolver o caminho.
pub fn fetch(url: &str, file: &str, sum: &Checksum, pb: &ProgressBar) -> Result<PathBuf, String> {
    let dir = cache_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dest = dir.join(format!("{}-{file}", &sum.hex()[..16]));
    if dest.exists() && sum.verify(&dest).is_ok() {
        pb.set_message("do cache");
        return Ok(dest);
    }
    let tmp = dest.with_extension("part");
    // Se a conexão cair no meio, continua de onde parou (Range) em vez de recomeçar.
    let mut attempt = 1;
    loop {
        match download_into(url, &tmp, pb) {
            Ok(()) => break,
            Err(e) if attempt < ATTEMPTS => {
                let _ = e;
                std::thread::sleep(Duration::from_secs(1 << (attempt - 1)));
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
    if let Err(e) = sum.verify(&tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

/// Baixa para `tmp`, retomando o que já existe nele se o servidor aceitar Range.
fn download_into(url: &str, tmp: &Path, pb: &ProgressBar) -> Result<(), String> {
    let have = fs::metadata(tmp).map_or(0, |m| m.len());
    let resp = get_range(url, have)?;
    let resumed = have > 0 && resp.status() == 206;
    let mut out = if resumed {
        fs::OpenOptions::new().append(true).open(tmp)
    } else {
        File::create(tmp)
    }
    .map_err(|e| format!("{}: {e}", tmp.display()))?;
    let start = if resumed { have } else { 0 };
    if let Some(len) = resp.body().content_length() {
        pb.set_length(start + len);
    }
    pb.set_position(start);
    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("download de {url} interrompido: {e}"))?;
        if n == 0 {
            return Ok(());
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        pb.inc(n as u64);
    }
}

// ---------- extração ----------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Archive {
    TarGz,
    TarXz,
    Zip,
    /// Um executável solto (rustup-init).
    Binary,
}

impl Archive {
    pub fn from_name(name: &str) -> Archive {
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Archive::TarGz
        } else if name.ends_with(".tar.xz") {
            Archive::TarXz
        } else if name.ends_with(".zip") {
            Archive::Zip
        } else {
            Archive::Binary
        }
    }
}

/// Tira os `strip` primeiros diretórios; recusa caminhos absolutos ou com "..".
fn stripped(path: &Path, strip: usize) -> Option<PathBuf> {
    let parts: Vec<_> = path.components().collect();
    if parts.iter().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return None;
    }
    let rest: PathBuf = parts.iter().filter(|c| matches!(c, Component::Normal(_))).skip(strip).collect();
    (!rest.as_os_str().is_empty()).then_some(rest)
}

fn untar<R: Read>(reader: R, dest: &Path, strip: usize) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(true);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let Some(rel) = stripped(&path, strip) else { continue };
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        entry.unpack(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(())
}

pub fn extract(file: &Path, kind: Archive, dest: &Path, strip: usize, binary_name: &str) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let open = || File::open(file).map_err(|e| format!("{}: {e}", file.display()));
    match kind {
        Archive::TarGz => untar(flate2::read::GzDecoder::new(BufReader::new(open()?)), dest, strip),
        Archive::TarXz => {
            // lzma-rs escreve num Write: descompacta para um .tar temporário, sem carregar tudo na memória.
            let tar_path = dest.with_extension("tar.tmp");
            {
                let mut out = io::BufWriter::new(File::create(&tar_path).map_err(|e| e.to_string())?);
                lzma_rs::xz_decompress(&mut BufReader::new(open()?), &mut out).map_err(|e| format!("{}: {e:?}", file.display()))?;
            }
            let result = untar(BufReader::new(File::open(&tar_path).map_err(|e| e.to_string())?), dest, strip);
            let _ = fs::remove_file(&tar_path);
            result
        }
        Archive::Zip => {
            let mut zip = zip::ZipArchive::new(open()?).map_err(|e| format!("{}: {e}", file.display()))?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
                let Some(rel) = entry.enclosed_name().and_then(|p| stripped(&p, strip)) else { continue };
                let target = dest.join(rel);
                if entry.is_dir() {
                    fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                    continue;
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let mut out = File::create(&target).map_err(|e| format!("{}: {e}", target.display()))?;
                io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
                if let Some(mode) = entry.unix_mode() {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&target, fs::Permissions::from_mode(mode & 0o7777)).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        }
        Archive::Binary => {
            use std::os::unix::fs::PermissionsExt;
            let target = dest.join(binary_name);
            fs::copy(file, &target).map_err(|e| e.to_string())?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = env::temp_dir().join(format!("devup-net-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn checksum_files_and_verification() {
        let text = "aaa  node-v1-linux-x64.tar.gz\nbbb  node-v1-linux-arm64.tar.gz\n";
        assert_eq!(find_sum(text, "node-v1-linux-arm64.tar.gz").as_deref(), Some("bbb"));
        assert_eq!(find_sum("5be1 *./rustup-init\n", "rustup-init").as_deref(), Some("5be1"));
        assert_eq!(find_sum("abc\n", "qualquer").as_deref(), Some("abc"));
        assert_eq!(find_sum(text, "outro.zip"), None);

        let d = tmp("sum");
        let f = d.join("x");
        fs::write(&f, "hello").unwrap();
        let ok = Checksum::Sha256("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".into());
        assert!(ok.verify(&f).is_ok());
        assert!(Checksum::Sha256("00".repeat(32)).verify(&f).is_err());
        let sha512 = "9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca72323c3d99ba5c11d7c7acc6e14b8c5da0c4663475c2e5c3adef46f73bcdec043";
        assert!(Checksum::Sha512(sha512.into()).verify(&f).is_ok());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn extracts_every_format_with_strip_and_no_escape() {
        let d = tmp("extract");
        // tar com pasta de topo, um executável e uma tentativa de sair do destino
        let mut builder = tar::Builder::new(Vec::new());
        let mut add = |path: &str, data: &[u8], mode: u32| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            builder.append_data(&mut h, path, data).unwrap();
        };
        add("node-v1/bin/node", b"#!/bin/sh\n", 0o755);
        add("node-v1/README", b"oi", 0o644);
        let tar_bytes = builder.into_inner().unwrap();

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar_bytes).unwrap();
        fs::write(d.join("a.tar.gz"), gz.finish().unwrap()).unwrap();
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut &tar_bytes[..], &mut xz).unwrap();
        fs::write(d.join("a.tar.xz"), xz).unwrap();
        let mut zipw = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        zipw.start_file("bun-linux-x64/bun", opts).unwrap();
        zipw.write_all(b"ELF").unwrap();
        fs::write(d.join("a.zip"), zipw.finish().unwrap().into_inner()).unwrap();

        use std::os::unix::fs::PermissionsExt;
        for (file, kind) in [("a.tar.gz", Archive::TarGz), ("a.tar.xz", Archive::TarXz)] {
            let out = d.join(format!("out-{file}"));
            extract(&d.join(file), kind, &out, 1, "").unwrap();
            assert_eq!(fs::read_to_string(out.join("README")).unwrap(), "oi");
            assert_eq!(fs::metadata(out.join("bin/node")).unwrap().permissions().mode() & 0o111, 0o111);
        }
        let out = d.join("out-zip");
        extract(&d.join("a.zip"), Archive::Zip, &out, 1, "").unwrap();
        assert_eq!(fs::read_to_string(out.join("bun")).unwrap(), "ELF");
        assert_eq!(fs::metadata(out.join("bun")).unwrap().permissions().mode() & 0o111, 0o111);

        assert_eq!(stripped(Path::new("../../etc/passwd"), 0), None);
        assert_eq!(stripped(Path::new("/etc/passwd"), 0), None);
        assert_eq!(stripped(Path::new("top"), 1), None);
        assert_eq!(Archive::from_name("x.tgz"), Archive::TarGz);
        fs::remove_dir_all(d).unwrap();
    }
}
