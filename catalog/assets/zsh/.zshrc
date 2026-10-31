# devup: zsh. Não edite aqui (é um link); use ~/.config/devup/shell/local.sh
HISTFILE=~/.zsh_history
HISTSIZE=50000
SAVEHIST=50000
setopt share_history hist_ignore_all_dups autocd
bindkey -e
autoload -Uz compinit && compinit
zstyle ':completion:*' matcher-list 'm:{a-z}={A-Z}'
zstyle ':completion:*' menu select

export PATH="$HOME/.local/bin:$PATH"
export EDITOR=nvim

# Caminhos mudam entre distros (Arch usa /usr/share/zsh/plugins).
for p in zsh-autosuggestions zsh-syntax-highlighting; do
  for dir in /usr/share/zsh/plugins/$p /usr/share/$p; do
    [ -f "$dir/$p.zsh" ] && source "$dir/$p.zsh" && break
  done
done

command -v mise >/dev/null && eval "$(mise activate zsh)"
command -v zoxide >/dev/null && eval "$(zoxide init zsh)"
command -v fzf >/dev/null && source <(fzf --zsh 2>/dev/null)
command -v starship >/dev/null && eval "$(starship init zsh)"
command -v devup >/dev/null && source <(devup completions zsh)

for f in ~/.config/devup/shell/aliases.sh ~/.config/devup/shell/local.sh; do
  [ -f "$f" ] && source "$f"
done
