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
dotfiles = ["https://github.com/<você>/dotfiles"]
shell = "zsh"          # zsh | fish | bash
theme = "tokyo-night"  # devup theme list
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

Perfis embutidos: `dev`, `gaming`, `essentials`, `server`, `desktop`, `full`.

## Shell

```sh
devup shell zsh   # ou fish / bash: instala, linka a config, starship + zoxide + fzf, e faz chsh
```

Os três shells carregam os mesmos aliases (`~/.config/devup/shell/aliases.sh`). As configs são links, então suas customizações vão em `~/.config/devup/shell/local.sh` (zsh/bash) ou `local.fish`.

## Desktop

```sh
devup add desktop   # fontes + config do desktop em uso
```

O perfil `desktop` detecta o ambiente (`XDG_CURRENT_DESKTOP`) e só aplica o módulo correspondente:

| Módulo | O que faz |
|---|---|
| `gnome` | gsettings: tema escuro, 6 workspaces fixos, Super+1..6, Super+Q fecha, Super+Enter terminal |
| `kde` | kwriteconfig: tema escuro, 6 áreas de trabalho, Meta+1..6, Meta+Q fecha (atalhos após novo login) |
| `hyprland` | Config completa + waybar, fuzzel, mako, hyprlock, hypridle. Pulado no Omarchy, que já cuida disso |
| `niri` | Config completa + waybar, fuzzel, mako, swaylock |
| `fonts` | JetBrainsMono Nerd Font + emoji (em qualquer desktop) |

No Hyprland, suas mudanças vão em `~/.config/hypr/local.conf`.

## Temas

```sh
devup theme list          # tokyo-night, catppuccin, gruvbox, nord + os seus
devup theme set gruvbox   # troca e recarrega o que estiver aberto
```

Um tema muda as cores de kitty, ghostty, tmux, waybar e das bordas do Hyprland, além do wallpaper (Hyprland, Niri, GNOME e KDE) e da cor de destaque do GNOME. Os arquivos gerados ficam em `~/.config/devup/current-theme/`, que as configs do devup incluem.

Para criar o seu, copie um tema de `catalog/themes/*.toml` para `~/.config/devup/themes/<nome>.toml` e troque as cores.

## Dotfiles

Módulos como `tmux`, `starship` e `ghostty` já vêm com uma config padrão, aplicada por symlink. Para usar os seus:

```sh
devup dotfiles add https://github.com/<você>/dotfiles   # clona, registra no devup.toml e aplica
devup dotfiles sync                                    # git pull em todos e reaplica
devup restore [id]                                     # desfaz: devolve os arquivos substituídos
```

O repo é clonado em `~/.local/share/devup/dotfiles/<nome>` e pode ter dois formatos:

- **stow**: cada pasta de topo é um pacote espelhado em `~` (`zsh/.zshrc` → `~/.zshrc`, `nvim/.config/nvim/init.lua` → `~/.config/nvim/init.lua`). É aplicado sempre.
- **com `devup/*.toml`**: módulos e perfis próprios, com `files.src` relativo à raiz do repo. Use com `devup add`.

Nada é apagado: o que existir no destino vai para `~/.local/state/devup/backups/<id>/`.

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
files = [{ src = "docker/daemon.json", dest = "~/.config/docker/daemon.json" }]  # src relativo à pasta do .toml
run = ["echo pronto"]                # comandos pós-instalação (devem ser idempotentes)
desktop = ["gnome", "kde"]           # só aplica nesses desktops (gnome | kde | hyprland | niri)
skip_if = "[ -f /etc/algo ]"         # se o comando der sucesso, pula o módulo
```

O catálogo embutido fica em `catalog/default.toml`, as configs padrão em `catalog/assets/` e os temas em `catalog/themes/` (paletas + `templates/`).

## Publicar uma versão

```sh
git tag v0.1.0 && git push origin v0.1.0
```

O workflow `.github/workflows/release.yml` gera os binários `devup-x86_64-linux` e `devup-aarch64-linux` e anexa ao Release.
