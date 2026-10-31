# Changelog

Todas as mudanças relevantes do devup. Formato baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/), versões em [SemVer](https://semver.org/lang/pt-BR/).

## [0.3.0] - 2026-10-31

Experiência de uso no nível de CLIs como gh, cargo, uv, brew e terraform.

### Saída e execução

- **Progresso no estilo cargo**: uma linha por etapa com contador (`Instalando [2/5] 12 pacotes (pacman)`), spinner e tempo. A saída do pacman, apt e afins vai para um log, não para a tela.
- **Falhas legíveis**: só o final da saída de quem falhou aparece, junto com o caminho do log completo.
- **Resumo final**: `✓ Pronto em 1m12s · 15 pacotes · 5 configs`, mais o próximo passo quando houver, como "abra um novo terminal para usar o zsh" ou "saia e entre de novo para usar o grupo docker".
- **Logs** de cada execução em `~/.local/state/devup/logs/` (os 20 mais recentes).
- **sudo uma vez, no começo**: o devup diz para que vai usar o sudo, pede a senha antes de começar e mantém o sudo ativo até o fim.
- **`-v` / `-q`**: saída completa dos comandos ou só erros e resumo.

### Plano antes de agir

- `apply`, `add`, `install`, `shell` e `dotfiles` mostram o plano (instalar, já tem, pular, configs e backups, serviços e grupos, shell e tema) e pedem uma confirmação, como o `terraform apply`.
- Se a máquina já está igual ao pedido, o comando diz "Nada a fazer" e não pergunta nada.
- `-y` aplica sem perguntar. Sem terminal, `-y` é obrigatório (exit 2).

### Descoberta e erros

- **Ajuda inteira em português**, com comandos agrupados, exemplos em cada subcomando e argumentos documentados.
- **"Você quis dizer?"** para comandos, opções, apps, perfis, temas, shells e backups (`dokcer` → `docker`, `zhs` → `zsh`).
- **Argumento faltando abre um seletor**: `devup add`, `install`, `remove`, `theme set`, `shell` e `restore`.
- **Autocomplete**: `devup completions bash|zsh|fish`. As configs de shell do devup já carregam o autocomplete.

### Automação e robustez

- `--json` em `list`, `status`, `doctor` e `theme list`.
- Códigos de saída: 0 ok, 1 falha, 2 uso incorreto, 130 cancelado.
- `--color auto|always|never` e respeito a `NO_COLOR`.
- `devup list | head` não quebra mais, e o Ctrl+C sempre devolve o cursor.
- Aviso de versão nova, verificado no máximo uma vez por dia em segundo plano (desligue com `DEVUP_NO_UPDATE_CHECK=1`).
- A troca de shell usa `usermod` com o sudo já autorizado, em vez de um `chsh` que pedia senha no meio.

## [0.2.0] - 2026-10-31

O `devup init` agora entende a sua máquina em vez de mostrar uma lista com 43 opções.

### Init inteligente

- **Detecta antes de perguntar**: distro, desktop, shell atual, apps e linguagens que você já usa e se o git já tem nome configurado.
- **Poucas perguntas, já respondidas**: para que você usa o PC (programar, jogar, dia a dia, servidor), linguagens, editor, shell e tema. Cada uma vem marcada com a recomendação e o motivo, por exemplo "cargo encontrado" ou "desktop detectado".
- **Plano antes de agir**: o que vai instalar, o que você já tem, o que é pulado nesta máquina, quais configs vão para backup e a troca de shell e tema. Uma confirmação: Aplicar, Ajustar apps ou Cancelar.
- **Já tem um setup?** O init pergunta o que você quer mudar (uso, shell ou tema) e mantém o resto, inclusive o que você adicionou à mão.
- **Prévia dos temas** com as cores reais no terminal.
- **Identidade do git**: se faltar, o init pergunta nome e e-mail.
- **`devup init --yes`** aceita as recomendações sem perguntar, para scripts e máquinas novas.

### Catálogo

- Novos módulos `python` e `go`, conferidos nos repositórios das 6 distros.
- `nodejs` é pulado se o `node` já estiver no PATH (via mise, por exemplo), e `rustup` não é instalado por cima de um Rust instalado de outro jeito.

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

[0.3.0]: https://github.com/bernardofernandezz/devup/releases/tag/v0.3.0
[0.2.0]: https://github.com/bernardofernandezz/devup/releases/tag/v0.2.0
[0.1.0]: https://github.com/bernardofernandezz/devup/releases/tag/v0.1.0
