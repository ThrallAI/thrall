# Thrall on the AUR

`thrall-bin` installs the desktop app, the engine (`thrall`) and its launcher entry from the
release's `.deb`, so `yay -S thrall-bin` works on Arch and Omarchy.

## Omarchy

- Open Thrall from the app launcher (Super+Space), or bind a key in
  `~/.config/hypr/bindings.conf`, for example:
  `bindd = SUPER, T, Thrall, exec, thrall-desktop`
  (pick a key Omarchy doesn't already use).
- The tray icon shows in Waybar's tray.
- "Start when I log in" (Settings or the tray) writes `~/.config/autostart/thrall.desktop`. If your
  session doesn't start XDG autostart entries, add `exec-once = thrall-desktop --hidden` to
  `~/.config/hypr/autostart.conf` instead.
- A blank window on an NVIDIA card: start it with `WEBKIT_DISABLE_DMABUF_RENDERER=1 thrall-desktop`.

These are being checked on a real Omarchy install for the first release; corrections welcome.
