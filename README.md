# Memrium

> This repository is the **shell** (compositor, bars, dock, launcher, Settings) and the library its programs share
> (`memrium_de`). The whole desktop, with the toolkit and the programs, is assembled in
> [memrium-desktop](https://github.com/Supchik22/memrium-desktop), which is the place to start.
>
> Components: [chebrium](https://github.com/Supchik22/chebrium) (UI toolkit),
> [memrium-secrets](https://github.com/Supchik22/memrium-secrets), [chebrium-qtstyle](https://github.com/Supchik22/chebrium-qtstyle),
> and the programs [calc](https://github.com/Supchik22/memrium-calc), [files](https://github.com/Supchik22/memrium-files),
> [view](https://github.com/Supchik22/memrium-view), [player](https://github.com/Supchik22/memrium-player),
> [term](https://github.com/Supchik22/memrium-term), [paint](https://github.com/Supchik22/memrium-paint),
> [snake](https://github.com/Supchik22/memrium-snake), [monitor](https://github.com/Supchik22/memrium-monitor),
> [archive](https://github.com/Supchik22/memrium-archive), [icons](https://github.com/Supchik22/memrium-icons),
> [convert](https://github.com/Supchik22/memrium-convert).
> The sections below still describe the desktop as one project; paths such as `src/bin/memrium_calc` now live in those repositories.


A configurable Wayland desktop with a GPU "jelly glass" renderer. It ships four looks, **macOS
Jelly Glass**, **GNOME**, **Fluent** and **Aero**, and everything about them (layout, glass, windows,
icons, wallpapers) is data you can change live from the Settings window. See `tools/idle-cpu.sh` for the design.

## Build

```
nix-shell --run 'cargo build --release'
```

The workspace has two crates: **`chebrium`**, the UI/UX toolkit shared by every program (below), and
**`memrium-de`**, the desktop itself (compositor, shell, apps) built on top of it.

Binaries in `target/release/`:

| binary | what it is |
|---|---|
| `memrium_compositor` | the desktop: compositor + shell (nested window by default, `--session` for a real session) |
| `memrium_term` | the terminal emulator (Wayland client, drawn to match the theme) |
| `memrium_files` | a file manager on Chebrium (places, list with sorting, rename, trash, copy / cut / paste, undo; `memrium_files [dir]`) |
| `memrium_view` | an image viewer (`memrium_view [file | folder]`): PNG, JPEG, WebP, GIF, BMP, TIFF, ICO and SVG, neighbours decoded in advance, wheel zoom towards the pointer, drag to pan, rotate, info, Trash |
| `memrium_player` | a media player on libmpv (`memrium_player <file | url> …`): every format mpv plays, subtitles, audio tracks, speed, playlist, fullscreen |
| `memrium_calc` | a pocket calculator on Chebrium (keyboard and mouse, animated keypad) |
| `memrium_icons` | the icon composer: layers and groups with Liquid Glass, blend modes, drag-and-drop layers, Default / Clear / Tinted looks in light and dark, saved as `.memicon`, built into an icon theme (`memrium_icons [icon-name]`) |
| `memrium_paint` | a Paint-style bitmap editor (ribbon, palette, shapes, fill, zoom, undo; `memrium_paint [file.png]`) |
| `memrium_preview` | shader demo (`--frames` writes PNGs) |
| `memrium_testclient`, `memrium_appscan`, `memrium_themecheck` | developer tools |

### Viewer and player keys

`memrium_view`: arrows / Space / Page keys next and previous, `+` `-` zoom, `0` fit, `1` actual size, `r` / `R` rotate,
`f` fullscreen, `i` information, `Delete` moves to the Trash, double click toggles fit and 100 %, `Esc` / `q` quit.

`memrium_player`: Space / `k` pause, Left / Right seek 5 s (Shift 1 s, Ctrl 60 s), Up / Down or the wheel volume, `m` mute,
`f` fullscreen, `n` / `p` next and previous, `s` subtitles, `a` audio track, `l` playlist, `i` information, `[` `]`
speed (Backspace resets it), `.` `,` single frames, `r` repeat, `c` screenshot to `~/Pictures`, click pauses and double
click toggles fullscreen. The player needs libmpv (it is loaded when it starts; `shell.nix` provides it, or set
`MEMRIUM_LIBMPV`). mpv's software renderer is single-threaded, so the player renders at a lower resolution when a frame
takes longer than the video allows and scales it up smoothly; a GPU output would be the next step for 4K.

Pictures (PNG, JPEG, WebP, GIF, BMP, TIFF, ICO, SVG) and audio / video files opened from the shell's Files, from
`memrium_files`, from desktop icons or from Figlet go to `memrium_view` and `memrium_player` (`fsops::open_external`, which
looks for them next to the running program, then in `PATH`); everything else, and these too when they are missing, goes to
`xdg-open`.

## Programs follow the shell

Whenever its look changes the shell writes `~/.config/memrium/appearance.toml` (light or dark, accent, window shape,
glass, language); Chebrium programs read it and follow live (see `chebrium/README.md`). Programs can ask for the shell's
header, draw their own in the same style, or have none.

## Chebrium: the shared UI library

`chebrium/` is what makes programs here pretty, animated and *very light*. It needs no GPU and no toolkit runtime:
a window is a shared-memory buffer painted by a small anti-aliased software canvas, and a program that is not moving
draws nothing.

| module | what it gives |
|---|---|
| `spring`, `anim` | the springy motion used everywhere; both report when they are at rest so redraws can stop |
| `canvas`, `soft` | software painter (rounded rects, gradients, lines, text, icons) over an ARGB buffer |
| `lucide`, `text` | the bundled Lucide icons and the Inter typeface |
| `draw` | the `Draw` trait: what a widget needs from a painter (software here, GPU glass in the desktop) |
| `ui`, `form` | immediate-mode widgets (button, switch, slider, segmented control, text field, settings rows) |
| `record` | the frame store behind `app`: widgets are recorded, an unchanged frame is skipped, a change repaints only its box |
| `picture` | decoded pictures and video frames drawn scaled and cropped (bilinear, with mips so shrinking stays clean) |
| `input`, `style` | keys the widgets understand, ink and accent colours |
| `app` | the window runtime for a program: one `App` impl and `app::run`; redraws only on input and while animating |

More in [`chebrium/README.md`](chebrium/README.md).

Settings and the other shell windows are built from the same widgets; `memrium-de` only supplies the GPU painter
(`renderer::PanelDraw`). See the widget kit rendered without any window:

```
cargo run -p chebrium --example gallery -- out_dir     # gallery-dark.png, gallery-light.png
```

## Try it inside your current desktop

```
./target/release/memrium_compositor                       # opens a window
./target/release/memrium_compositor --exec memrium_term    # ... and starts a terminal in it
MEMRIUM_THEME=windows7 ./target/release/memrium_compositor # try a template without saving anything
```

Useful debug flags: `--settings [page]`, `--launcher "query"`, `--edit`, `--live aurora`,
`--shot out.png --after-ms 4000` (save a screenshot of the compositor's own frame and exit).

## On NixOS (build it and try it)

The repository is a Nix package, a flake and a NixOS module (`nix/`, `default.nix`, `flake.nix`). Rust libraries are
fetched from `Cargo.lock`, so the first build needs the network and takes a while (about 12 minutes).

```
nix-build                      # ./result/bin/memrium_compositor, with every library and tool wrapped in
./result/bin/memrium_compositor    # inside a running Wayland desktop it opens as a window: nothing is installed

nix run .                      # the same through the flake (nix --extra-experimental-features 'nix-command flakes' ...)
```

To get it on the login screen, with the flake in `/etc/nixos/flake.nix`:

```nix
inputs.memrium.url = "path:/path/to/this/checkout";   # or a git URL
# in the system's modules:
imports = [ inputs.memrium.nixosModules.default ];
programs.memrium.enable = true;
```

then `sudo nixos-rebuild switch` and pick "Memrium" in the session list. Without flakes, add
`services.displayManager.sessionPackages = [ (pkgs.callPackage /path/to/this/checkout/nix/package.nix { }) ];` instead.
The Qt style plugin (`qtstyle/`, CMake) is not part of the package yet; Qt programs then keep their own style.
Settings live in `~/.config/memrium`.

## Run as a real session (from a TTY)

Log in on a text console (Ctrl+Alt+F3) and run:

```
/path/to/memrium-desktop/memrium/session/start-tty.sh
```

`start-tty.sh` enters `nix-shell` (the libraries come from `shell.nix`) and runs `session/memrium-session`; the output
is also saved to `~/memrium-session.log`. On non-Nix systems run `session/memrium-session` directly.

It starts a session D-Bus if needed and the compositor with `--session` (DRM/KMS output, libinput input,
libseat for device access, so it needs logind or seatd and must not run inside another graphical session).
`Ctrl+Alt+F1..F12` switches VT, `Ctrl+Alt+Backspace` ends the session. `~/.config/memrium/autostart`
lists commands to start (one per line). `MEMRIUM_SCALE=1.5` forces a UI scale.

For a display manager, install `session/memrium-de.desktop` into `/usr/share/wayland-sessions/` (on NixOS add a
package that provides it to `services.displayManager.sessionPackages`) and put `memrium-session` on the PATH.

> The session backend compiles and follows the standard libseat/DRM/libinput flow, but it has only been
> exercised for build correctness, not on real hardware. Expect to debug the first TTY run.

## Settings and themes

`Ctrl+,` or the **Settings** app in the Dock/start menu opens the Settings window: template cards,
dark mode, accent colour, glass sliders, wallpaper, layout (menu bar + Dock, or taskbar), window style, icon
shape/plate/padding/tint. Changes apply live and are saved to `~/.config/memrium/config.toml`:

```toml
theme = "windows11"          # macos | windows11 | windows7 | a file in ~/.config/memrium/themes/

[override.glass]             # any theme field, merged over the chosen template
blur = 20.0

[override.taskbar]
position = "top"
align = "left"

[override.icons]
shape = "circle"
plate = "glass"
[override.icons.overrides]
"org.kde.dolphin" = "~/icons/folder.svg"
```

Whole themes are TOML files (see `themes/*.toml` for every field).

## Wallpapers

Images (PNG/JPEG/WebP) from `~/Pictures/Wallpapers`, `~/Pictures` and the system wallpaper packs, plus **live
shaders**. A live wallpaper is a WGSL file defining one function; drop it into `~/.config/memrium/wallpapers/`:

```wgsl
fn live_wall(uv: vec2<f32>, t: f32, aspect: f32) -> vec3<f32> {
    return vec3(0.5 + 0.5 * sin(t + uv.x * 6.0), uv.y, 0.6);
}
```

A shader that fails to compile is rejected with a message on stderr and the current wallpaper stays.

## Shortcuts

| keys | action |
|---|---|
| `Ctrl+Space` or `Super+Space` | launcher (Figlet / start menu, per theme) |
| `Ctrl+,` | Settings |
| `Alt+Tab` | previous window |
| `Alt+F4` or `Super+Q` | close window; `Super+M` minimize; `Super+Up` maximize |
| `Esc` | close launcher / Settings / Edit Controls |
| `Super+F` | fullscreen the front window (clients that ask for fullscreen get it too; bars hide) |
| `Super+L` / `Super+W` / `Super+D` | lock / widgets panel / show desktop |
| `Ctrl+Alt+←/→` | previous / next virtual desktop (`+Shift` moves the window along) |

Every shortcut above can be changed on the **Keyboard** page of Settings (or in `[keys.binds]` in `config.toml`),
and the same page binds shortcuts to programs (`[[keys.commands]]`). `Ctrl+Space` is the default launcher key; the
older `Super+Space` stays free for the keyboard-layout switch.

## Several monitors

In a real session every connected monitor is used. The laptop panel is the main display (menu bar, Dock, panels
and notifications live there); the others extend the desktop to its left (default) or right, and windows can be
dragged across all of them. Plug and unplug is picked up within a few seconds. The **Displays** page of Settings
chooses the main display, the side, the order, the resolution and refresh rate of each monitor, switches monitors
off and sets the interface size; press *Apply* to use the changes (`[display]` in `config.toml`).
Monitors on different graphics cards work too, because every frame is drawn once and copied to each screen.
`MEMRIUM_FAKE_LEFT=700` gives a nested window an imaginary monitor on the left, for trying the layout.

## Desktop widgets

Right-click the desktop → **Add widget**: clock, **glass clock** (Jelly Glass digits, resizable from its menu),
calendar, system, disk, weather, **now playing** (any MPRIS player, with play / next / previous) and a **vinyl
record** that turns while music plays, with the album cover as its label. Drag them anywhere (also onto another
monitor); right-click a widget to resize or remove it. The lock screen's clock is glass too.

## Trying things without a window

`memrium_compositor --headless --shot out.png --after-ms 3000` draws the shell offscreen (no desktop needed); the
`--settings`, `--lock`, `--exec` and `MEMRIUM_TEST_*` switches work with it. `MEMRIUM_HEADLESS_SIZE=1600x900`.

## Also in Settings

- **Appearance**: Light / Dark / Auto (dark from 19:00 to 07:00), accent colour (presets or any colour), highlight
  colour, sidebar size, wallpaper tinting, scroll bar behaviour.
- **Animations**: speed, the light/dark switch effect (circle from the pointer, fade, none) and interface sounds.
- **Themes**: templates, saving the current look as a template, importing a theme file.
- **General**: language, auto-lock / screen off / suspend, screenshot options, weather city for the widgets panel.

## Status

Works: windows (resize, minimize, maximize, popups, subsurfaces), real app launching, Dock/taskbar,
three templates, Settings, live wallpapers, icon styling, terminal, X11 apps (XWayland), GPU-buffer clients
(linux-dmabuf), screen capture (wlr-screencopy), pointer locking for games. Not yet: notifications, lock screen,
real Wi-Fi/volume/brightness controls (the panels are visual), video wallpapers, multi-monitor, input methods.

### X11 apps, GPU buffers, screen sharing

- **XWayland** starts with the compositor (`MEMRIUM_NO_XWAYLAND=1` turns it off). X11 windows are framed and animated
  like any other; menus and tooltips (override-redirect windows) are placed by the program and never take focus.
  Programs started from the shell get `DISPLAY` automatically. The clipboard is not bridged to X11 yet.
- **linux-dmabuf**: the wgpu device is created with the Vulkan DMA-BUF import extensions, so browsers, players and
  Xwayland's glamor path hand over GPU buffers without a copy (`MEMRIUM_NO_DMABUF=1` falls back to shared memory).
  Only single-plane 8-bit RGBA layouts are advertised; compressed (CCS) modifiers are left out on purpose.
- **Screen capture**: `wlr-screencopy` is served (`memrium_grab out.png` is a small client for it). For screen sharing in
  browsers and OBS install `xdg-desktop-portal-wlr`, copy `session/memrium-portals.conf` to
  `~/.config/xdg-desktop-portal/`, and start the session with `session/memrium-session`, which exports the display
  variables to D-Bus. This chain needs a real session with PipeWire; only the screencopy part is tested in a nested window.
- **Games**: relative pointer motion and pointer locking are implemented (real sessions; in a nested window the host
  keeps the pointer).

### Icon tiles

Settings → Icons → *Plate behind icons* → **Tile** sets every application icon (Papirus or any other theme) on an
opaque tile in the theme's icon shape, like macOS. *Tile colour*: Auto (light tile for coloured icons, dark for
mostly-white ones), Light, Dark or Vivid (the icon's own colour). Only application icons get tiles; file icons and
thumbnails are left alone.

### Memrium Icons

`memrium_icons` is an icon composer in the manner of Apple's Icon Composer. An icon of your applications (a Papirus one) opens as a
stack of **layers and groups** on a **plate**: one layer for each shape of its SVG, grouped as the drawing groups them. The left
panel lists them (eye = visible, fold the groups; **drag a layer** to reorder it, onto the middle of another layer to group the two, onto a group to
move it inside; Forward / Backward / Group / Ungroup / Delete / Duplicate; the *Icons* tab picks another icon); click a layer on the icon to select it (it is outlined, with handles at the corners of its box): **drag it to move it, drag a corner handle to scale it**, nudge it with the arrow keys (Shift for bigger steps).

The right panel is the inspector of the selection:
- **Color**: one fill colour for the layer (or its original colours), opacity, blend mode (Normal, Multiply, Screen, Overlay,
  Darken, Lighten, Plus). The colour picker takes a typed or pasted hex code (Ctrl+C / Ctrl+V), and an eyedropper that takes a colour
  from the icon.
- **Liquid Glass**, for each layer or group (individually or combined) and for the plate: specular edge light, frosting of what is
  behind, refraction, translucency, and a neutral or chromatic shadow.
- **Composition**: visibility, shift and scale. The plate has a shape (circle, squircle, rounded), a solid or gradient fill and
  glass of its own; *Border* leaves room for shadows.

The looks above the icon are the ones Apple's systems make of an icon: **Default**, **Clear** (frosted grey glass) and **Tinted** (one
colour), each for a **Light** or a **Dark** theme (the usual look in a dark theme is *Default + Dark*). **The looks follow each other, so one icon is enough**: Dark follows Default; Clear (dark) follows Clear (light), which follows
Default; Tinted the same. A look keeps only what you changed in it (an opacity, a colour, a position) and gets the rest from its
parent, so a later change in Default still reaches every look that did not set that field itself. Clear and Tinted start as grey or
tinted glass made of the default look. *Inherit all* in the inspector drops a look's own changes. Undo / redo (Ctrl+Z, Ctrl+Y) cover every change. *All looks* under the icon shows all six side by side (light above, dark below), drawn
again live as you edit; click one to edit that look.

Documents are saved as you work in the `.memicon` format (`~/.local/share/memrium/icons/<icon>.memicon`): a small binary container
of our own (magic `MEMICON3`; version 1 and 2 files are still read, deflated payload, CRC-32; see `src/icondoc.rs`). *Build icon theme* draws every saved document into
`~/.local/share/icons/Memrium-Icons` (PNGs of 48 to 256 px, default look; everything else is inherited from Papirus). To use it,
pick **Memrium-Icons** in Settings → Icons and set the shape and plate there to *None*. Drawing is `src/iconrender.rs`, importing
SVG shapes is `src/svgedit.rs`.

## Panels and widgets

The menu bar and the Dock are both *panels* (`src/widget.rs`) holding *widgets*. A panel is a full-width `strip` or
a `floating` pill on the top or bottom edge; its background is `glass`, `solid`, `fade` (blur that fades out
downwards, no hard edge) or `clear`. **Right-click a panel** to change all of this, add / remove / move widgets or
enter *Edit Panel*, where widgets can be dragged around, removed with their red badge, or added with the toolbar.
Everything is saved to `config.toml` and can also be written by hand:

```toml
[[override.bars]]
name  = "menubar"
edge  = "top"
shape = "strip"
left  = ["logo", "title", "app-menu"]
right = ["battery", "wifi", { kind = "button", icon = "camera", action = "screenshot" }, "control-center", "clock"]

[[override.bars]]
name   = "dock"
edge   = "bottom"
shape  = "floating"
center = ["dock", "divider", { kind = "button", icon = "terminal", action = "terminal" }]
```

Widgets: `logo`, `title`, `app-menu`, `clock {format, format2}`, `battery`, `wifi`, `bluetooth`, `volume`, `search`,
`button {icon, action}`, `text {text, bold}`, `spacer {width}`, `divider`, `control-center`, `dock`, and the
taskbar pieces in `src/taskwidgets.rs`: `start {style}`, `search-box`, `tasks`, `tray`, `show-desktop`.
Actions: `launcher` (`start`), `settings`, `files`, `terminal`, `screenshot`, `screenshot-region`, `control-center`,
`edit-controls`, `dark-mode`, `wallpaper`, `show-desktop`, `quit`.

A taskbar is just a panel with `surface = "glass"` (its own `glass`, `radius`, `inset`). **Settings → Panels** edits
panels and, for the selected widget, every option it has; **Settings → Glass** edits the glass of the theme and of
each glass panel.

**Quick Settings tiles are data.** The dropdown is a four-column grid whose tiles come from `[[cc_tiles]]` in
`config.toml` (empty = the built-in layout, see `src/cctiles.rs`). Each tile has a `kind`, a place (`col`, `row`) and
options:

```toml
[[cc_tiles]]
kind = "media"            # wifi | bluetooth | media | slider | pill | toggle
col = 2
row = 0
look = "half-record"      # player | record | half-record

[[cc_tiles]]
kind = "toggle"           # a round button: icon + action
col = 0
row = 3
icon = "terminal"
action = "terminal"       # any bar action (settings, files, lock, dark-mode, show-desktop, ...) or "exec:program --arg"
```

`slider` takes `target = "display" | "sound"`; `pill` takes `icon`, `title`, `subtitle`, `action` and
`style = "dark" | "active"`. Open the Quick Settings and press *Edit controls* to change it without touching the file:
tap a control in the gallery to add it, drag a tile to move it, tap the **−** on its corner to remove it, tap a Now
Playing tile to step through its looks (the record is one GPU panel, kind 15 in `glass.wgsl`) or a slider to switch
between Display and Sound. Adding a kind: a `KindInfo` in `cctiles.rs`, a branch in `ControlCenter::build_tile`.
The Windows quick-settings panel still has its own fixed layout.

Adding a widget: implement `Widget`, register its name in `widget::create`, list it in `widget::CATALOG`, and describe
its options once in `widget::options` (the Settings page, the defaults the widget reads and the tests all use that
one table).

### Animations

Settings → Animations lists every animation in groups (Windows, Menus, Shell), each with its own switch and speed;
`[override.animations.effects.<id>] on = false, speed = 1.5` in `config.toml`. An animation that is switched off does
not play: what it moves is put at its end state at once (`AnimStyle::step`). Windows wobble like jelly while they are
moved (`window_jelly`; a warp in `glass.wgsl`, `genie.x < 0`).

### Dock, taskbar and cursor

- The **Dock** widget is configurable per widget (Settings → Panels): icon size, gap, magnification amount and reach,
  running mark (dot / line / glow / none), name tooltips, hover **window previews** (a thumbnail, the title and a close
  button for each window of the app; the previews can be switched off per widget), only running apps, bounce. The
  taskbar's **Task buttons** have the same previews option.
- Right click on a Dock or taskbar item: windows of the app, Open New Window, Pin / Unpin (kept in
  `~/.config/memrium/dock`), Move to Desktop, Minimize, Maximize, Fullscreen, Close, Edit Dock. Middle click starts
  another window.
- Every interactive widget also takes `on_right`, `on_middle`, `scroll_up`, `scroll_down` (an action name such as
  `volume-up`, or `exec:command`) and a `tooltip`. The **Command output** widget shows the first line of a shell
  command, refreshed every few seconds.
- The **Mouse** page of Settings sets the cursor's size and colour, pointer speed, acceleration, scrolling and
  tapping. The shell draws the cursor itself (`cursor.rs`): clients that use `cursor-shape-v1` (Chebrium programs do)
  get the shell's arrow, text beam, hand, resize arrows and so on; the shell also shows resize arrows over window
  edges and a busy cursor while a program starts. Clients that draw their own cursor picture (Xwayland) keep the arrow.
- Launcher styles: Figlet, Launchpad (full-screen paged grid, scroll or ←/→ for pages), Start (Fluent), Start (Aero).

## Cost

The shell draws only when the picture changed: each frame is hashed and an identical one is skipped (no swapchain
image, no GPU work), and the loop sleeps ~30 ms between checks instead of spinning. Springs come to rest exactly
(`spring.rs`) so animations really end. Measured with the nested compositor on an idle desktop: from ~99 % of a
core to ~1.7 %. Check it yourself with the idle-CPU script described in `tools/idle-cpu.sh`, or `--bench N` for the
cost of drawing a frame.

`cargo test --lib` covers the widget catalogue and option schema, every template's panels, the glass modes, config
round trips and the spring.

### Frame cost

`tools/bench-drag.sh [compositor arguments]` drags a window across the desktop (headless) and prints the GPU time of the
window pass and the shell pass, the CPU time of building a frame and the frame times. `MEMRIUM_SKIP=layer` switches the
cache of the desktop's icon layer off, `frame`, `surf`, `mip` or `shell` leave that kind of work out. On the development
machine (Intel UHD, 1600 x 900, a Files window moved over 40 desktop icons) the icon cache brought the window pass from
8.9 ms to 6.7 ms and the frame from 16.9 ms to 14.3 ms, with a picture identical pixel for pixel.
