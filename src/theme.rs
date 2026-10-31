use crate::config::config_dir;
use include_dir::{Dir, include_dir};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

static THEMES: Dir = include_dir!("$CARGO_MANIFEST_DIR/catalog/themes");
pub const DEFAULT: &str = "tokyo-night";

/// Arquivos gerados que as configs do devup incluem.
pub fn current_dir() -> PathBuf {
    config_dir().join("current-theme")
}

fn user_dir() -> PathBuf {
    config_dir().join("themes")
}

/// O tema aplicado agora, se houver.
pub fn current() -> Option<String> {
    fs::read_to_string(current_dir().join(".name")).ok().map(|s| s.trim().to_string())
}

/// Temas embutidos + ~/.config/devup/themes/*.toml.
pub fn names() -> Vec<String> {
    let builtin = THEMES.files().map(|f| f.path().to_path_buf());
    let user = fs::read_dir(user_dir()).into_iter().flatten().flatten().map(|e| e.path());
    let mut names: Vec<String> = builtin
        .chain(user)
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter_map(|p| Some(p.file_stem()?.to_str()?.to_string()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Cores do tema. Cada `#rrggbb` também vira `<nome>_raw` (sem `#`), para formatos como `rgb(...)` do Hyprland.
pub fn palette(name: &str) -> Result<BTreeMap<String, String>, String> {
    let user = fs::read_to_string(user_dir().join(format!("{name}.toml"))).ok();
    let builtin = THEMES.get_file(format!("{name}.toml")).and_then(|f| f.contents_utf8()).map(String::from);
    let text = user.or(builtin).ok_or_else(|| {
        let names = names();
        format!("tema desconhecido: {name}{} · devup theme list mostra todos", crate::ui::did_you_mean(name, names.iter().map(String::as_str)))
    })?;
    let table: toml::Table = toml::from_str(&text).map_err(|e| format!("tema {name}: {e}"))?;
    let mut vars = BTreeMap::from([("name".to_string(), name.to_string())]);
    for (k, v) in table {
        let v = v.as_str().ok_or_else(|| format!("tema {name}: {k} precisa ser texto"))?.to_string();
        if let Some(raw) = v.strip_prefix('#') {
            vars.insert(format!("{k}_raw"), raw.to_string());
        }
        vars.insert(k, v);
    }
    Ok(vars)
}

/// Preenche cada template com a paleta; erro se faltar alguma cor.
pub fn render(name: &str) -> Result<Vec<(String, String)>, String> {
    let vars = palette(name)?;
    let templates = THEMES.get_dir("templates").expect("catalog/themes/templates");
    templates
        .files()
        .map(|f| {
            let mut text = f.contents_utf8().unwrap().to_string();
            for (k, v) in &vars {
                text = text.replace(&format!("{{{{{k}}}}}"), v);
            }
            let file = f.path().file_name().unwrap().to_string_lossy().to_string();
            match text.find("{{") {
                Some(i) => Err(format!("tema {name}: {file} usa {} que a paleta não define", &text[i..text[i..].find("}}").map_or(text.len(), |j| i + j + 2)])),
                None => Ok((file, text)),
            }
        })
        .collect()
}

/// Escreve o tema em ~/.config/devup/current-theme; com `reload`, avisa os apps abertos.
pub fn apply(name: &str, reload: bool, dry_run: bool) -> Result<(), String> {
    let files = render(name)?;
    let dir = current_dir();
    crate::ui::status("Aplicando", &format!("tema {name}"));
    if dry_run {
        return Ok(());
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (file, text) in files {
        fs::write(dir.join(&file), text).map_err(|e| format!("{file}: {e}"))?;
    }
    fs::write(dir.join(".name"), name).map_err(|e| e.to_string())?;
    if reload {
        let _ = Command::new("sh").arg(dir.join("reload.sh")).status();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_theme_fills_every_template() {
        let names: Vec<_> = THEMES.files().filter_map(|f| f.path().file_stem()?.to_str()).collect();
        assert!(names.contains(&DEFAULT));
        for name in names {
            let files = render(name).unwrap();
            let hypr = &files.iter().find(|(f, _)| f == "hyprland.conf").unwrap().1;
            assert!(hypr.contains("rgb(") && !hypr.contains("rgb(#"), "{name}");
        }
        assert!(render("nope").is_err());
    }
}
