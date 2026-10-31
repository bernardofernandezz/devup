# devup

Configure seu Linux no estilo [Omarchy](https://omarchy.org), em qualquer distro: escolha apps de dev, jogos e uso geral e o `devup` instala cada um pelo melhor método disponível (pacote nativo → flatpak → script).

Suporta distros com `pacman`, `apt`, `dnf` ou `zypper` (Arch, Debian/Ubuntu, Fedora, openSUSE e derivadas). Nas demais, usa flatpak e scripts.

## Instalação

```sh
curl -fsSL https://raw.githubusercontent.com/bernardofernandezz/devup/main/install.sh | sh
```

Baixa o binário do último Release para `~/.local/bin` (mude com `DEVUP_BIN_DIR=/outro/dir`).

Ou da fonte: `cargo install --git https://github.com/bernardofernandezz/devup`

## Uso

```sh
devup                          # menu interativo (espaço marca, Enter instala)
devup list [--category games]  # catálogo e como cada app seria instalado nesta máquina
devup install docker steam     # instala direto
devup --dry-run install ...    # só mostra os comandos
```

## Adicionar apps

Edite `apps.toml` e recompile. Exemplo:

```toml
[[app]]
name = "docker"
category = "dev"                # dev | games | general
description = "Containers"
pacman = "docker"               # pacotes nativos (separados por espaço)
apt = "docker.io"
dnf = "moby-engine"
zypper = "docker"
flatpak = "..."                 # fallback
script = "curl ... | sh"        # último fallback
bin = "docker"                  # se já estiver no PATH, pula
```

## Publicar uma versão

```sh
git tag v0.1.0 && git push origin v0.1.0
```

O workflow `.github/workflows/release.yml` gera os binários `devup-x86_64-linux` e `devup-aarch64-linux` e anexa ao Release.
