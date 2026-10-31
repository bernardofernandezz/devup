#!/bin/sh
# devup: GNOME estilo Omarchy: tema escuro, 6 workspaces fixos, Super+1..6, Super+Q fecha, Super+Enter abre o terminal.
set -e
g() { gsettings set "$@"; }

g org.gnome.desktop.interface color-scheme 'prefer-dark'
g org.gnome.desktop.interface monospace-font-name 'JetBrainsMono Nerd Font 11'
g org.gnome.mutter center-new-windows true
g org.gnome.mutter dynamic-workspaces false
g org.gnome.desktop.wm.preferences num-workspaces 6
g org.gnome.desktop.wm.preferences button-layout ':minimize,maximize,close'
g org.gnome.desktop.wm.keybindings close "['<Super>q']"
g org.gnome.desktop.wm.keybindings toggle-fullscreen "['<Super>f']"
for i in 1 2 3 4 5 6; do
  # Super+N abre apps do dock por padrão; libera para os workspaces.
  g org.gnome.shell.keybindings switch-to-application-$i "[]"
  g org.gnome.desktop.wm.keybindings switch-to-workspace-$i "['<Super>$i']"
  g org.gnome.desktop.wm.keybindings move-to-workspace-$i "['<Super><Shift>$i']"
done

# Super+Enter: acrescenta o atalho sem apagar os personalizados que já existem.
base=org.gnome.settings-daemon.plugins.media-keys
path=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/devup-terminal/
list=$(gsettings get $base custom-keybindings)
case "$list" in
  *devup-terminal*) ;;
  "@as []") g $base custom-keybindings "['$path']" ;;
  *) g $base custom-keybindings "${list%]}, '$path']" ;;
esac
term=$(command -v ghostty || command -v kitty || command -v kgx || command -v gnome-terminal || echo xterm)
g $base.custom-keybinding:$path name 'Terminal'
g $base.custom-keybinding:$path command "$term"
g $base.custom-keybinding:$path binding '<Super>Return'
