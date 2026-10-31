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

O seu setup fica em `~/.config/devup/devup.toml`:

```toml
profiles = ["dev", "gaming"]
modules = ["obsidian"]
exclude = ["vscode"]   # não aplica, mesmo vindo de um perfil
```

```sh
devup                          # = devup init: menu com perfis e módulos, salva o devup.toml e aplica
devup apply                    # converge a máquina para o devup.toml
devup apply --from <arquivo|url>  # usa o devup.toml de outra máquina/repo (validado antes de salvar)
devup status                   # ✓ instalado, ✗ falta, ? não dá pra saber
devup add gaming obsidian      # adiciona ao devup.toml e aplica
devup remove docker            # tira do devup.toml (não desinstala)
```

Nova máquina, mesmo setup:

```sh
curl -fsSL https://raw.githubusercontent.com/bernardofernandezz/devup/main/install.sh | sh
devup apply --from https://raw.githubusercontent.com/<você>/dotfiles/main/devup.toml
```

Comandos avulsos, sem mexer no devup.toml:

```sh
devup list [--category games]  # módulos, como cada um seria instalado aqui, e perfis
devup list --profile dev       # módulos de um perfil
devup install dev docker       # instala perfis e/ou módulos (dependências entram sozinhas)
devup --dry-run install ...    # só mostra os comandos
```

Perfis embutidos: `dev`, `gaming`, `essentials`, `server`, `full`.

## Seus módulos e perfis

Crie arquivos em `~/.config/devup/modules/*.toml`. Um módulo com o mesmo nome de um embutido o substitui.

```toml
[profile.meu-setup]
description = "Meu setup"
include = ["dev"]                    # outros perfis
modules = ["docker", "obsidian"]

[[module]]
name = "docker"
category = "dev"                     # dev | games | general | server | shell | desktop | theme | dotfiles
description = "Containers"
requires = ["git"]                   # instalados antes, automaticamente
pacman = "docker docker-compose"     # pacotes nativos (separados por espaço)
apt = "docker.io"
dnf = "moby-engine"
zypper = "docker"
aur = "..."                          # usado se houver paru/yay
flatpak = "..."                      # fallback
script = "curl ... | sh"             # último fallback
bin = "docker"                       # se já estiver no PATH, pula a instalação
services = ["docker"]                # systemctl enable --now
groups = ["docker"]                  # usermod -aG <grupo> $USER
run = ["echo pronto"]                # comandos pós-instalação (devem ser idempotentes)
```

O catálogo embutido fica em `catalog/default.toml`.

## Publicar uma versão

```sh
git tag v0.1.0 && git push origin v0.1.0
```

O workflow `.github/workflows/release.yml` gera os binários `devup-x86_64-linux` e `devup-aarch64-linux` e anexa ao Release.
