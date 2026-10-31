# devup: fish. Não edite aqui (é um link); use ~/.config/devup/shell/local.fish
set -g fish_greeting
fish_add_path ~/.local/bin
# Linguagens instaladas pelo devup (node, go, rust…)
set -q XDG_DATA_HOME; or set -l XDG_DATA_HOME ~/.local/share
test -f $XDG_DATA_HOME/devup/env.fish; and source $XDG_DATA_HOME/devup/env.fish
set -gx EDITOR nvim

if status is-interactive
    type -q mise; and mise activate fish | source
    type -q zoxide; and zoxide init fish | source
    type -q fzf; and fzf --fish 2>/dev/null | source
    type -q starship; and starship init fish | source
    type -q devup; and devup completions fish | source
end

for f in ~/.config/devup/shell/aliases.sh ~/.config/devup/shell/local.fish
    test -f $f; and source $f
end
