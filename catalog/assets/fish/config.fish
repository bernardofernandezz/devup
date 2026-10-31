# devup: fish. Não edite aqui (é um link); use ~/.config/devup/shell/local.fish
set -g fish_greeting
fish_add_path ~/.local/bin
set -gx EDITOR nvim

if status is-interactive
    type -q mise; and mise activate fish | source
    type -q zoxide; and zoxide init fish | source
    type -q fzf; and fzf --fish 2>/dev/null | source
    type -q starship; and starship init fish | source
end

for f in ~/.config/devup/shell/aliases.sh ~/.config/devup/shell/local.fish
    test -f $f; and source $f
end
