#!/bin/sh
# devup: KDE Plasma: tema escuro, 6 áreas de trabalho, Meta+1..6, Meta+Q fecha. Atalhos valem após novo login.
set -e
k=$(command -v kwriteconfig6 || command -v kwriteconfig5) || { echo "kwriteconfig não encontrado" >&2; exit 1; }

plasma-apply-colorscheme BreezeDark || true
$k --file kdeglobals --group General --key fixed 'JetBrainsMono Nerd Font,11,-1,5,400,0,0,0,0,0,0,0,0,0,0,1'
$k --file kwinrc --group Desktops --key Number 6
$k --file kwinrc --group Desktops --key Rows 1
$k --file kglobalshortcutsrc --group kwin --key 'Window Close' 'Meta+Q,Alt+F4,Close Window'
for i in 1 2 3 4 5 6; do
  $k --file kglobalshortcutsrc --group kwin --key "Switch to Desktop $i" "Meta+$i,Ctrl+F$i,Switch to Desktop $i"
done
# Meta+N abre itens da barra de tarefas por padrão; libera para as áreas de trabalho.
for i in 1 2 3 4 5 6 7 8 9; do
  $k --file kglobalshortcutsrc --group plasmashell --key "activate task manager entry $i" "none,Meta+$i,Activate Task Manager Entry $i"
done
qdbus6 org.kde.KWin /KWin reconfigure 2>/dev/null || qdbus org.kde.KWin /KWin reconfigure 2>/dev/null || true
