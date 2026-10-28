# Add the Memrium session to the login screen on NixOS: import this file from configuration.nix
#   imports = [ /home/mykhailo/Data/oldwindows/projects/rust/liquid-de/session/nixos-session.nix ];
# then `sudo nixos-rebuild switch`. The session appears in SDDM's session list next to Plasma.
{ pkgs, ... }:
let
  root = "/home/mykhailo/Data/oldwindows/projects/rust/liquid-de";
  # The file dialog backend (memrium_portal) from the cargo build of this checkout. On NixOS xdg-desktop-portal reads .portal
  # files only from NIX_XDG_DESKTOP_PORTAL_DIR (the system profile), so ~/.local/share/xdg-desktop-portal is ignored.
  # libwayland-client and libxkbcommon are loaded at run time, and a service started by D-Bus does not get the session's
  # LD_LIBRARY_PATH (shell.nix): without them the portal cannot list windows and the chooser dialog never opens
  libs = pkgs.lib.makeLibraryPath (with pkgs; [ wayland libxkbcommon libGL vulkan-loader libinput pipewire ]);
  memriumPortal = pkgs.runCommand "memrium-portal" { } ''
    mkdir -p $out/share/xdg-desktop-portal/portals $out/share/dbus-1/services
    cat > $out/share/xdg-desktop-portal/portals/memrium.portal <<PORTAL
    [portal]
    DBusName=org.freedesktop.impl.portal.desktop.memrium
    Interfaces=org.freedesktop.impl.portal.FileChooser;org.freedesktop.impl.portal.ScreenCast
    UseIn=Memrium
    PORTAL
    cat > $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.memrium.service <<SERVICE
    [D-BUS Service]
    Name=org.freedesktop.impl.portal.desktop.memrium
    Exec=${pkgs.coreutils}/bin/env LD_LIBRARY_PATH=${libs} ${root}/target/release/memrium_portal
    SERVICE
  '';
in
{
  services.displayManager.sessionPackages = [
    ((pkgs.writeTextDir "share/wayland-sessions/memrium-de.desktop" ''
      [Desktop Entry]
      Name=Memrium
      Comment=Configurable Wayland desktop
      Exec=${root}/session/start-dm.sh
      Type=Application
      DesktopNames=Memrium
    '').overrideAttrs (_: { passthru.providedSessions = [ "memrium-de" ]; }))
  ];

  # Screen sharing (memrium_portal: the screen or one window, through PipeWire; screenshots stay with xdg-desktop-portal-wlr)
  # and the Memrium file dialogs
  services.pipewire.enable = true;
  xdg.portal = {
    enable = true;
    wlr.enable = true;
    extraPortals = [ pkgs.xdg-desktop-portal-gtk memriumPortal ];
    config.memrium = {
      default = [ "gtk" ];
      "org.freedesktop.impl.portal.ScreenCast" = [ "memrium" ];
      "org.freedesktop.impl.portal.Screenshot" = [ "wlr" ];
      "org.freedesktop.impl.portal.FileChooser" = [ "memrium" ];
    };
  };
  services.dbus.packages = [ memriumPortal ];
}
