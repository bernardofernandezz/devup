# devup: bash. Não edite aqui (é um link); use ~/.config/devup/shell/local.sh
[[ $- != *i* ]] && return
[ -f /etc/bashrc ] && . /etc/bashrc

HISTSIZE=50000
HISTFILESIZE=50000
HISTCONTROL=ignoreboth:erasedups
shopt -s histappend autocd

export PATH="$HOME/.local/bin:$PATH"
export EDITOR=nvim

[ -f /usr/share/bash-completion/bash_completion ] && . /usr/share/bash-completion/bash_completion

command -v mise >/dev/null && eval "$(mise activate bash)"
command -v zoxide >/dev/null && eval "$(zoxide init bash)"
command -v fzf >/dev/null && eval "$(fzf --bash 2>/dev/null)"
command -v starship >/dev/null && eval "$(starship init bash)"

for f in ~/.config/devup/shell/aliases.sh ~/.config/devup/shell/local.sh; do
  [ -f "$f" ] && . "$f"
done
