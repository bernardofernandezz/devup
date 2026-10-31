# Changelog

Todas as mudanças relevantes do devup. Formato baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/), versões em [SemVer](https://semver.org/lang/pt-BR/).

## [0.1.0] - 2026-10-31

Primeira versão: um comando para deixar qualquer distro Linux pronta para trabalhar e jogar.

### Instalação

```sh
curl -fsSL https://raw.githubusercontent.com/bernardofernandezz/devup/main/install.sh | sh
devup init
```

Binários estáticos para `x86_64` e `aarch64`, que rodam em qualquer distro. O instalador confere o `sha256sums.txt`.

### Destaques

- **Setup declarativo**: `~/.config/devup/devup.toml` descreve a máquina e `devup apply` converge para ele. `devup apply --from <url>` replica o setup em outro computador.
- **Catálogo com 37 módulos e 6 perfis**: `dev`, `gaming`, `essentials`, `server`, `desktop` e `full`.
- **Qualquer distro**: `pacman`, `apt`, `dnf`, `zypper`, `apk` e `xbps`, com fallback para AUR (paru/yay), flatpak e scripts. Se um pacote não existir na sua versão da distro, só ele falha.
- **Dotfiles**: configs padrão para tmux, starship, kitty e ghostty, ou o seu repo git (`devup dotfiles add`). Tudo por symlink, com backup e `devup restore`.
- **Shell**: `devup shell zsh|fish|bash` instala e configura com starship, zoxide, fzf e aliases comuns.
- **Desktop**: GNOME, KDE Plasma, Hyprland e Niri, detectados automaticamente.
- **Temas**: `devup theme set` com tokyo-night, catppuccin, gruvbox e nord. Muda terminal, tmux, waybar, bordas e wallpaper, e recarrega o que estiver aberto.
- **Manutenção**: `devup doctor`, `devup status` e `devup update`.

### Testado

- `cargo test` (17 testes).
- `--dry-run` em containers de Arch, Ubuntu 24.04, Fedora 42, openSUSE Tumbleweed, Alpine 3.22 e Void.
- Nomes de pacotes conferidos nos repositórios reais dessas distros.

### Limitações conhecidas

- GNOME, KDE e Niri ainda não foram testados em sessões reais. Os atalhos do KDE valem após um novo login.
- Em Alpine e Void (sem systemd), os `services` dos módulos não são habilitados.
- A detecção de desktop usa `XDG_CURRENT_DESKTOP`: via SSH ou TTY, os módulos de desktop são pulados.

[0.1.0]: https://github.com/bernardofernandezz/devup/releases/tag/v0.1.0
