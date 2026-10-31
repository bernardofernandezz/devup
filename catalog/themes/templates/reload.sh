#!/bin/sh
# devup: aplica o tema {{name}} no que estiver rodando. Tudo aqui é opcional e silencioso.
wall="$HOME/.config/devup/current-theme/wallpaper.svg"
case "${XDG_CURRENT_DESKTOP:-}" in
  *GNOME*)
    gsettings set org.gnome.desktop.interface accent-color '{{gnome_accent}}' 2>/dev/null
    gsettings set org.gnome.desktop.background picture-uri "file://$wall"
    gsettings set org.gnome.desktop.background picture-uri-dark "file://$wall"
    ;;
  *KDE*) plasma-apply-wallpaperimage "$wall" >/dev/null 2>&1 ;;
esac
pgrep -x Hyprland >/dev/null && hyprctl reload >/dev/null
pkill -USR1 -x kitty
pkill -USR2 -x ghostty
pkill -USR2 -x waybar
tmux source-file ~/.config/tmux/tmux.conf 2>/dev/null
# Só troca o wallpaper do swaybg se a config do Hyprland/Niri for a do devup (não mexe no Omarchy, por exemplo).
if grep -qs "^# devup\|^// devup" ~/.config/hypr/hyprland.conf ~/.config/niri/config.kdl && pgrep -x swaybg >/dev/null; then
  pkill -x swaybg
  setsid swaybg -m fill -i "$wall" >/dev/null 2>&1 &
fi
exit 0
