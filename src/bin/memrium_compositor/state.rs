//! Compositor state: windows, popups, surface textures, focus, interactive move/resize, input routing.
//! smithay handles the protocol; pixels go to our wgpu renderer.

use crate::role::Role;
use memrium_de::apps::{self, AppEntry, IconIndex, IconSlot};
use memrium_de::decor::{self, Btn};
use memrium_de::files::{self, Files, FilesCtx, FilesEvent, Key, KeyMods};
use memrium_de::fsops::{self, Fm};
use memrium_de::menu::{Menu, MenuClick, MenuItem, NavKey};
use memrium_de::theme::{Config, DecoKind, Theme};
use memrium_de::launcher::{DrawCtx, Launcher, LauncherClick};
use memrium_de::figlet::{self, Target};
use memrium_de::live;
use memrium_de::settings::{live_choices, Settings, SettingsAction, SettingsCtx, WallItem, WIN_H, WIN_W};
use memrium_de::theme::{WallKind, WallpaperSpec};
use memrium_de::wallpapers;
use memrium_de::widget::{Bar, BarClick, BarCtx, BarHit};
use memrium_de::renderer::{Item, Label, Panel, Renderer, SurfaceTex, Warp};
use memrium_de::scene::{DockIcon, DockItem, DockWin, Scene, ShellEvent};
use chebrium::spring::Spring;
use smithay::{
    backend::input::{Axis, AxisSource, ButtonState, KeyState},
    input::{
        keyboard::{FilterResult, Keycode, KeyboardHandle, Keysym, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerHandle},
        Seat, SeatState,
    },
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason, ObjectId},
            protocol::{wl_buffer::WlBuffer, wl_shm, wl_surface::WlSurface},
            DisplayHandle, Resource,
        },
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::{
            get_parent, with_states, with_surface_tree_upward, BufferAssignment, CompositorClientState, CompositorState, SubsurfaceCachedState,
            SurfaceAttributes, TraversalAction,
        },
        selection::data_device::DataDeviceState,
        shell::xdg::{decoration::XdgDecorationState, PopupSurface, SurfaceCachedState, ToplevelSurface, XdgShellState, XdgToplevelSurfaceData},
        shm::{with_buffer_contents, ShmState},
    },
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

/// resize grab margin around a window (logical px)
const EDGE: f32 = 7.0;
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;
const B_ABOUT: u32 = 400;
const B_SETTINGS: u32 = 401;
const B_FILES: u32 = 402;
const B_TERMINAL: u32 = 403;
const B_LAUNCHER: u32 = 404;
const B_QUIT: u32 = 405;
const B_CLOSE: u32 = 406;
const B_MINIMIZE: u32 = 407;
const B_ZOOM: u32 = 408;
const B_CC: u32 = 409;
const B_EDIT: u32 = 410;
const B_NEW_FILES: u32 = 411;
const B_SHELL_SETTINGS: u32 = 412;
const B_SHELL_FILES: u32 = 413;
const B_SLEEP: u32 = 414;
const B_RESTART: u32 = 415;
const B_SHUTDOWN: u32 = 416;
const B_LOGOUT: u32 = 417;
const B_LOCK: u32 = 418;
const B_LAUNCHPAD: u32 = 419;
const B_WIN_BASE: u32 = 1000;
const MIN_W: f32 = 260.0;
const MIN_H: f32 = 140.0;

pub struct ClientState {
    pub compositor: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

/// Who draws the window chrome.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Deco {
    /// no decoration protocol used: the client draws everything, we only add a shadow
    Legacy,
    /// the client asked for client-side decorations: rounded corners + shadow from us
    Client,
    /// we draw the titlebar and traffic lights
    Server,
}

/// A client surface's GPU texture.
pub struct SurfNode {
    pub surface: WlSurface,
    pub tex: Option<Rc<SurfaceTex>>,
    pub scale: i32,
    /// the client's DMA-BUF currently shown (released to the client once another one replaces it)
    pub dma_buf: Option<Held>,
    /// the pixels of a small picture (bytes, width, height, stride, opaque): a program's own cursor is read from here
    pub small: Option<(Vec<u8>, u32, u32, u32, bool)>,
    /// the size the program asked this surface to be shown at (`wp_viewport` destination), when it did: a program drawing for
    /// a fractional scale makes a buffer of 1.5 pixels per logical pixel and the destination says how many logical ones
    pub logical: Option<[f32; 2]>,
    /// the picture of an X11 window: its pixels are the device's, so it is shown at the buffer's size over the desktop's scale
    pub x11: bool,
    /// the part the program says is opaque (`wl_surface.set_opaque_region`, surface-local logical x, y, w, h): what lies
    /// beneath it does not have to be drawn
    pub opaque: Option<[f32; 4]>,
    /// for a popup that draws its own shadow: the solid parts of its picture (buffer px x0, y0, x1, y1), so our shadow and glass
    /// follow the menu itself and not the see-through margin round it. Empty when the picture is solid all over.
    pub body: Vec<[u32; 4]>,
}

/// A client's GPU buffer that we sample, and the point on its explicit-sync timeline to signal once we are done with it.
pub struct Held {
    /// None: only the timeline point is still owed (the client attached the same buffer again)
    pub buf: Option<WlBuffer>,
    pub release: Option<smithay::wayland::drm_syncobj::DrmSyncPoint>,
}

impl Held {
    /// Hand it back: the client may draw into it again.
    pub fn done(self) {
        if let Some(b) = self.buf {
            b.release();
        }
        if let Some(p) = self.release {
            if let Err(e) = p.signal() {
                eprintln!("syncobj: could not signal a release point: {e}");
            }
        }
    }
}

/// The solid rectangles of a picture (rows where pixels are at least 200/255 opaque, grouped; a narrow row such as a bubble's
/// tail is left out). Empty when one of them is the whole picture.
fn solid_parts(bytes: &[u8], w: usize, h: usize, stride: usize) -> Vec<[u32; 4]> {
    let mut parts: Vec<[u32; 4]> = Vec::new();
    let mut open = false;
    for y in 0..h {
        let row = &bytes[y * stride..y * stride + w * 4];
        let solid = |x: usize| row[x * 4 + 3] >= 200;
        let x0 = (0..w).find(|&x| solid(x));
        let x1 = (0..w).rev().find(|&x| solid(x));
        match (x0, x1) {
            (Some(a), Some(b)) if b - a >= 24 => {
                let (a, b) = (a as u32, b as u32 + 1);
                match parts.last_mut().filter(|_| open) {
                    // a row much wider than the part so far starts another one (a wide bubble over a narrower menu)
                    Some(p) if a + 8 >= p[0] && b <= p[2] + 8 => {
                        p[0] = p[0].min(a);
                        p[2] = p[2].max(b);
                        p[3] = y as u32 + 1;
                    }
                    _ => parts.push([a, y as u32, b, y as u32 + 1]),
                }
                open = true;
            }
            _ => open = false,
        }
    }
    if parts.len() == 1 && parts[0][2] - parts[0][0] + 4 >= w as u32 && parts[0][3] - parts[0][1] + 4 >= h as u32 {
        parts.clear();
    }
    parts
}

impl SurfNode {
    /// The surface's size in logical px: what the viewport says, else the buffer's size over its (whole) scale.
    pub fn logical_size(&self) -> Option<[f32; 2]> {
        let t = self.tex.as_ref()?;
        self.logical.or_else(|| {
            let sc = if self.x11 { crate::role::x_scale() } else { self.scale.max(1) as f32 };
            Some([t.size.0 as f32 / sc, t.size.1 as f32 / sc])
        })
    }
}

pub struct Win {
    pub id: u64,
    pub role: Role,
    /// an X11 override-redirect window (menu, tooltip): placed by the client, never framed or focused
    pub or: bool,
    /// where the client area was last told to be (X11 windows live in screen coordinates)
    pub x11_loc: Option<(i32, i32)>,
    pub deco: Deco,
    pub app_id: String,
    pub title: String,
    /// top-left of the window frame (geometry area + titlebar), spring-driven
    pub x: Spring,
    pub y: Spring,
    /// lift while held / settle
    pub scale: Spring,
    /// 0 = shown, 1 = minimized into the Dock (animated)
    pub min_k: Spring,
    /// 0 -> 1 when the window appears (pop-in)
    pub open_k: Spring,
    pub minimized: bool,
    /// when it was last brought to the front (see `ShellWin::stamp`)
    pub stamp: u64,
    /// the virtual desktop the window lives on
    pub ws: usize,
    /// minimized only because its desktop is not the current one
    pub ws_hidden: bool,
    /// frame rect (x, y, w, h) to return to after un-maximizing
    pub restore: Option<[f32; 4]>,
    /// set with `restore` when the window was snapped: -1 the left half, 1 the right half, 2 / 3 the top left / right corner, 4 / 5 the bottom ones (0 = maximized)
    pub tile: i8,
    /// fullscreen: no decoration, no bars; holds (the frame, and the maximize `restore`) to go back to
    pub fs: Option<([f32; 4], Option<[f32; 4]>)>,
    /// jelly while the window moves: how far its far parts lag (x, y), and the position last frame
    pub jel: [Spring; 2],
    pub jprev: [f32; 2],
    /// where the window is held while it moves (share of its width)
    pub jgrab: f32,
    /// window geometry in surface-local px: (x, y, w, h)
    pub geo: [f32; 4],
    /// decoration insets around the client area: left, top, right, bottom
    pub ins: [f32; 4],
    pub mapped: bool,
    /// the program's picture is dark along its top edge (None: not known, e.g. a GPU buffer we cannot read)
    pub content_dark: Option<bool>,
}

impl Win {
    /// The theme to draw this window's frame with: the desktop's, with light/dark turned to match the program's content.
    pub fn chrome_theme<'a>(&self, theme: &'a Theme) -> std::borrow::Cow<'a, Theme> {
        let dark = self.content_dark.filter(|d| theme.window.adapt && *d != theme.dark);
        // a maximized window with its title bar left out has a frame without one
        let bare = theme.window.hide_title_max && self.restore.is_some() && self.deco == Deco::Server;
        if dark.is_none() && !bare {
            return std::borrow::Cow::Borrowed(theme);
        }
        let mut t = theme.clone();
        if let Some(d) = dark {
            t.dark = d;
        }
        if bare {
            t.window.decoration = DecoKind::Minimal;
        }
        std::borrow::Cow::Owned(t)
    }
}

/// How bright the top edge of a picture is (0 dark .. 1 light), from a few rows of BGRA pixels below any shadow the
/// program draws around itself. `None` when those pixels are mostly see-through.
/// The biggest rectangle of an opaque region (x, y, w, h). A region with holes cut into it gives none: only its plain
/// rectangles are trusted.
fn opaque_rect(region: &smithay::wayland::compositor::RegionAttributes) -> Option<[f32; 4]> {
    use smithay::wayland::compositor::RectangleKind;
    if region.rects.iter().any(|(k, _)| matches!(k, RectangleKind::Subtract)) {
        return None;
    }
    let r = region.rects.iter().map(|(_, r)| r).filter(|r| r.size.w > 0 && r.size.h > 0).max_by_key(|r| r.size.w as i64 * r.size.h as i64)?;
    Some([r.loc.x as f32, r.loc.y as f32, r.size.w as f32, r.size.h as f32])
}

/// The buffer rows a commit's damage covers (surface damage is in logical px, so it is scaled); `None`: no damage was
/// given, which is taken as "all of it".
fn damage_rows(damage: &[smithay::wayland::compositor::Damage], scale: i32) -> Option<std::ops::Range<u32>> {
    use smithay::wayland::compositor::Damage;
    let mut band: Option<(i64, i64)> = None;
    for d in damage {
        let (y, h) = match d {
            Damage::Buffer(r) => (r.loc.y as i64, r.size.h as i64),
            // one logical row may spread over the edge of a scaled one: a row more each side
            Damage::Surface(r) => (r.loc.y as i64 * scale as i64 - 1, r.size.h as i64 * scale as i64 + 2),
        };
        let (a, b) = (y.max(0), (y + h.max(0)).max(0));
        band = Some(match band {
            Some((x0, x1)) => (x0.min(a), x1.max(b)),
            None => (a, b),
        });
    }
    band.map(|(a, b)| a.min(u32::MAX as i64) as u32..b.min(u32::MAX as i64) as u32)
}

fn top_edge_luma(bytes: &[u8], w: usize, h: usize, stride: usize, opaque: bool) -> Option<f32> {
    let row = (h / 40).clamp(2, 24);
    let (mut sum, mut n) = (0.0f32, 0u32);
    for y in row..(row + 3).min(h) {
        for k in 0..48 {
            let x = w / 10 + (w * 8 / 10) * k / 48;
            let p = y * stride + x * 4;
            let Some(px) = bytes.get(p..p + 4) else { continue };
            if !opaque && px[3] < 200 {
                continue;
            }
            sum += 0.114 * px[0] as f32 + 0.587 * px[1] as f32 + 0.299 * px[2] as f32;
            n += 1;
        }
    }
    (n >= 24).then(|| sum / n as f32 / 255.0)
}

pub struct Popup {
    pub surface: PopupSurface,
    pub grab: bool,
    /// 0 when it has just opened, 1 once it is up: it fades and grows in (the effect of the shell's own menus)
    pub k: Spring,
}

impl Popup {
    pub fn new(surface: PopupSurface) -> Popup {
        Popup { surface, grab: false, k: Spring::new(0.0, 420.0, 0.82) }
    }
}

/// Number of virtual desktops.
pub const WORKSPACES: usize = 4;

/// How far a window of desktop `ws` is from the screen while the row of desktops is at `pos` (`width` apart).
fn desk_dx(ws: usize, pos: f32, width: f32) -> f32 {
    (ws as f32 - pos) * width
}

/// The arrow keys that snap windows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapKey {
    Left,
    Right,
    Up,
    Down,
}

/// What a snap key does to a window.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SnapOp {
    Tile(i8),
    Maximize,
    Restore,
    Minimize,
    /// to the monitor beside it (-1 left, 1 right)
    Beside(i8),
}

/// Windows the shell draws itself (not Wayland clients): draggable, minimizable, listed in the Dock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sw {
    Settings,
    Files,
}

/// A closed window fading out: its last pixels and chrome, kept for a moment.
pub struct Ghost {
    panels: Vec<Panel>,
    frame: [f32; 4],
    surf: Option<(Rc<SurfaceTex>, [f32; 4], Option<([f32; 4], f32)>, bool)>,
    k: Spring,
}

pub struct ShellWin {
    pub x: Spring,
    pub y: Spring,
    /// 1 while shown, 0 hidden (closed or minimized); animates in between
    pub k: Spring,
    /// last drawn panels, replayed while the window animates away
    pub last: Vec<Panel>,
    pub minimized: bool,
    pub hover: Option<Btn>,
    /// the last transition was a minimise / restore (genie) rather than open / close
    pub genie: bool,
    /// maximised: the (x, y) to return to, and the enlarged client size
    pub restore: Option<[f32; 2]>,
    /// as for client windows (-1, 1, 2 .. 5 when snapped, 0 when maximized)
    pub tile: i8,
    pub size: Option<[f32; 2]>,
    pub jel: [Spring; 2],
    pub jprev: [f32; 2],
    pub jgrab: f32,
    /// when it was last brought to the front: one clock for shell and client windows, so they stack in one order
    pub stamp: u64,
    /// the desktop (workspace) it was opened on
    pub ws: usize,
}

impl ShellWin {
    pub fn new() -> ShellWin {
        ShellWin { x: Spring::new(0.0, 700.0, 0.8), y: Spring::new(0.0, 700.0, 0.8), k: Spring::new(0.0, 240.0, 0.74), last: Vec::new(), minimized: false, hover: None, genie: false, restore: None, tile: 0, size: None, jel: [jelly_spring(), jelly_spring()], jprev: [0.0; 2], jgrab: 0.5, stamp: 0, ws: 0 }
    }
}

/// What a context menu was opened for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuFor {
    Desktop,
    Files,
    DesktopIcons,
    /// a widget on the bar (see `State::bar_click`)
    Bar(BarHit),
    /// a panel's context menu: (bar, widget under the pointer)
    Panel(usize, Option<usize>),
    /// a Dock / taskbar item's menu (see `dock.rs`)
    Dock,
    /// the menu of a tray icon (see `tray_ui.rs`)
    Tray,
    /// the keyboard layout list
    Layout,
}

/// Files being dragged (from the file manager or the desktop).
pub struct FileDrag {
    pub paths: Vec<std::path::PathBuf>,
    pub start: [f32; 2],
    pub active: bool,
}

/// Where a picture that went to the clipboard alone is kept (so it can be dragged out of its toast); old ones are removed.
fn shot_cache_dir() -> String {
    let base = std::env::var("XDG_CACHE_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache"));
    let dir = base.join("memrium").join("screenshots");
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 86_400);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    dir.to_string_lossy().to_string()
}

/// `file://` address of a path, as a `text/uri-list` line wants it.
pub fn file_uri(p: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The spring of a window's jelly wobble: loose, so that it swings past and settles.
pub fn jelly_spring() -> Spring {
    Spring::new(0.0, 210.0, 0.34)
}

/// The warp that makes a window of frame `f` lag by `off`, held at `grab` (share of its width along the top).
fn jelly_warp(f: [f32; 4], off: [f32; 2], grab: f32) -> Warp {
    let reach = f[2].max(f[3]) / 2.0 + off[0].abs().max(off[1].abs()) + 8.0;
    Warp { rect: f, t: 0.0, dock: [f[0] + f[2] / 2.0, f[1] + f[3] / 2.0], icon_w: reach, jelly: Some([off[0], off[1], grab.clamp(0.0, 1.0)]) }
}

/// Where the lag target of a moving thing is: against its velocity, within limits.
fn jelly_target(v: f32) -> f32 {
    (-v * 0.05).clamp(-60.0, 60.0)
}

/// Ids the Dock gives the shell's own windows (client windows use `Win::id`).
pub const SHELL_SETTINGS: u64 = u64::MAX - 1;
pub const SHELL_FILES: u64 = u64::MAX - 2;

/// The id a window goes by in the Dock's previews and menus.
pub fn win_id(w: &Win) -> Option<u64> {
    Some(w.id)
}

pub enum Grab {
    /// `start` is where the button went down: a maximized window only comes loose after the pointer has moved a little
    ShellMove { win: Sw, off: [f32; 2], start: [f32; 2] },
    /// resizing a shell window from an edge or corner (bits as in `Resize`); `start` is the frame when it began
    ShellResize { win: Sw, edges: u8, start_mouse: [f32; 2], start: [f32; 4] },
    Move { surf: WlSurface, off: [f32; 2], start: [f32; 2] },
    /// edges bitmask: 1 left, 2 right, 4 top, 8 bottom; `start` = frame (x, y, w, h)
    Resize { surf: WlSurface, edges: u8, start_mouse: [f32; 2], start: [f32; 4] },
}

pub enum Hit {
    Button(Btn),
    Titlebar,
    Edge(u8),
    Content,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ShotReq {
    Full,
    Window,
    Region([f32; 4]),
    /// pick this area into the editor
    Edit([f32; 4]),
    /// record this area
    Record([f32; 4]),
}

/// Print held down: a short press picks an area, holding it takes the whole screen.
pub struct ShotHold {
    pub at: Instant,
    pub key: Option<u32>,
    pub fired: bool,
}

/// Region selection in progress (Shift+PrtSc): `start` is set once the button goes down.
pub struct ShotSel {
    pub start: Option<[f32; 2]>,
    /// the area is for a screen recording, not a screenshot
    pub record: bool,
}

pub struct State {
    pub dh: DisplayHandle,
    pub renderer: Renderer,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    pub seat: Seat<State>,
    pub pointer: PointerHandle<State>,
    pub keyboard: KeyboardHandle<State>,
    pub output: Output,

    /// back to front; the last window is focused
    pub wins: Vec<Win>,
    pub popups: Vec<Popup>,
    /// candidate windows of the input method
    pub ime_popups: Vec<smithay::wayland::input_method::PopupSurface>,
    /// per-program rules (`app-rules.toml`), read again when the file changes
    pub rules: memrium_de::apprules::Rules,
    pub rules_checked: Instant,
    pub surfaces: HashMap<ObjectId, SurfNode>,
    pub scene: Scene,
    pub mouse: [f32; 2],
    pub grab: Option<Grab>,
    /// integer scale advertised to clients
    pub out_scale: i32,
    /// logical output size
    pub size: [f32; 2],
    pub start: Instant,
    pub next_id: u64,

    // ---- look
    pub config: Config,
    pub theme: Theme,
    pub hover_btn: Option<(usize, Btn)>,
    pub last_title_click: Option<Instant>,

    // ---- settings window
    pub settings: Settings,
    pub swin: ShellWin,
    pub fwin: ShellWin,
    /// shell windows, back to front
    pub z: Vec<Sw>,
    /// the clock of `stamp`s: shell windows and client windows are stacked by when they were last raised
    pub stack_clock: u64,
    /// the shell window that held the focus when the keyboard was last handed over (`update` follows `focused_shell`)
    pub shell_focus_seen: Option<Sw>,
    pub files: Files,
    pub fm: Fm,
    pub menu: Menu,
    pub menu_for: MenuFor,
    /// hovered menu bar item
    pub bar_hover: Option<BarHit>,
    pub bars: Vec<Bar>,
    /// the panel being edited (widgets draggable / removable)
    pub panel_edit: Option<usize>,
    pub panel_drag: Option<crate::panels::PanelDrag>,
    pub fileicons: crate::fileicons::IconCache,
    pub desktop: memrium_de::desktop::Desktop,
    pub file_drag: Option<FileDrag>,
    /// the files a program was offered by a drag out of the shell (what `text/uri-list` is made of)
    pub file_dnd: Vec<std::path::PathBuf>,
    /// input preferences changed: the session re-applies them to the devices
    pub input_dirty: bool,
    pub presets: Vec<(String, String)>,
    pub walls: Vec<WallItem>,
    pub wall_paths: Vec<std::path::PathBuf>,
    pub walls_loaded: bool,
    pub thumb_rx: Option<Receiver<(usize, image::RgbaImage)>>,
    pub wall_rx: Option<Receiver<image::RgbaImage>>,
    /// an accent colour taken from the current wallpaper (offered in Settings)
    pub wall_accent: Option<[f32; 3]>,
    pub pending_theme: Option<Theme>,
    pub pending_actions: Vec<SettingsAction>,
    /// when the theme last changed (config is saved shortly after the last change)
    pub cfg_dirty: Option<Instant>,
    pub default_wallpaper: Option<std::path::PathBuf>,
    pub menu_light: bool,

    // ---- apps and launching
    pub socket: String,
    pub apps: Vec<AppEntry>,
    /// built on a background thread (scanning icon themes takes a moment)
    pub icons: Arc<IconSlot>,
    /// the icons menus show beside their entries (the "actions" folders), indexed off-thread
    pub action_icons: Arc<IconSlot>,
    pub icon_failed: HashSet<String>,
    /// set by the thread that rebuilt the icon indexes for a new icon theme: the loaded icons are dropped on the next frame
    pub icons_rebuilt: Arc<std::sync::atomic::AtomicBool>,
    /// when the icon theme the editor writes was last looked at, and the time its marker file had then
    pub icons_marker: (Instant, Option<std::time::SystemTime>),
    /// application icons made on other threads
    pub icon_loader: crate::icon_loader::IconLoader,
    pub pinned: Vec<String>,
    /// window previews over the Dock: the one shown, what the pointer rests on and for how long, how long it was away
    /// the cursor: what it is now, what the client under it asked for, which surface that was, when it was last
    /// worked out, a counter that changes with the picture, and the busy cursor's end and window count
    pub cursor_shape: memrium_de::cursor::Shape,
    pub client_cursor: memrium_de::cursor::Shape,
    /// the surface a program drew its own cursor on, and what it looks like now (Xwayland and older toolkits do that)
    pub client_cursor_surface: Option<WlSurface>,
    pub client_cursor_img: Option<memrium_de::cursor::ClientCursor>,
    pub cursor_focus: Option<ObjectId>,
    pub cursor_check: Instant,
    pub cursor_gen: u32,
    pub busy_until: Option<Instant>,
    pub busy_wins: usize,
    pub preview: Option<crate::dock::DockPreview>,
    pub dock_wait: (String, f32),
    /// the Dock item files are being carried over, and for how long (negative once its window was brought up)
    pub dock_drag: (String, f32),
    pub dock_linger: f32,
    /// the item whose context menu is open, with its windows
    pub dock_menu: Option<(String, Vec<DockWin>)>,
    /// the widget whose tooltip may show: (bar, widget), how long the pointer rested, and the text with the widget's rectangle
    pub tip: ((usize, usize), f32, Option<([f32; 4], String)>),
    pub launcher: Launcher,
    /// evdev codes of pressed keys we consumed (their release must not reach clients)
    pub intercepted: HashSet<u32>,
    /// ask the session backend to leave
    pub quit: bool,
    /// the session may skip rendering when the frame would be identical to the last one
    pub allow_skip: bool,
    /// the target holds the previous frame between draws (a window's swap chain does not): only what changed is redrawn
    pub persistent_target: bool,
    pub damage: crate::damage::DamageTracker,
    pub ghosts: Vec<Ghost>,
    pub shot_sel: Option<ShotSel>,
    pub shot_req: Option<ShotReq>,
    pub recording: Option<crate::screenrec::Recording>,
    /// a finished recording being closed by ffmpeg in the background
    pub rec_done: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    /// the editor open on a picked area
    pub shot_edit: Option<crate::shot_edit::ShotEdit>,
    /// Print is down: when, which key, and whether the whole-screen shot already went off
    pub shot_hold: Option<ShotHold>,
    /// a picture of one window alone is being made: the shell's panels and the desktop are left out
    pub capture_alone: bool,
    pub capturing: bool,
    /// "saved" notice (text, seconds left) and the flash level
    pub shot_notice: Option<(String, f32)>,
    pub shot_flash: f32,
    /// the snap gesture: how visible the preview of the maximised window is, whether the pointer is at the top edge, the frame a
    /// dragged window started from (it comes back there on restore), and the last pointer samples for the flick
    pub snap: f32,
    pub snap_on: bool,
    /// where the snap goes: 0 the whole work area (top edge), -1 / 1 its left / right half, 2 .. 5 a corner (see `snap_rect`)
    pub snap_side: i8,
    pub move_start: Option<[f32; 4]>,
    pub move_samples: Vec<(Instant, [f32; 2])>,
    /// set by `render` when it found nothing to draw
    pub frame_skipped: bool,
    /// frames that were really drawn (not skipped as identical); screen capture waits on this
    pub frames_drawn: u64,
    pub last_hash: u64,
    /// ask the session backend to switch to this VT
    pub pending_vt: Option<i32>,
    /// X11 support: the window manager, its protocol state and the display number apps should use
    pub xwm: Option<smithay::xwayland::X11Wm>,
    pub xwayland_shell_state: smithay::wayland::xwayland_shell::XWaylandShellState,
    pub x_display: Option<u32>,
    /// the lock screen, while the screen is locked
    pub lock: Option<crate::lock::Lock>,
    /// volume, Wi-Fi, Bluetooth, brightness, battery (read in the background) and the commands to change them
    pub sys_watcher: memrium_de::sysctl::SysWatcher,
    pub sys: memrium_de::sysctl::SysInfo,
    /// the tray: the service, the icons applications put there, the "more" arrow's state, and the menu being shown
    pub tray: crate::tray_service::TrayService,
    pub tray_items: Vec<memrium_de::tray::TrayIcon>,
    /// the ones the panel shows (rebuilt when they change, not on every look at the panel)
    pub tray_shown: Vec<memrium_de::tray::TrayIcon>,
    /// the keyboard layouts the keymap holds, by their short names (`us`, `ua`)
    pub kbd_layouts: Vec<String>,
    pub tray_retry: Instant,
    /// how far a side bar is kept from the screen's top and bottom by the bars along them (rebuilt each frame)
    pub axis_cache: std::cell::Cell<[f32; 2]>,
    pub tray_expanded: bool,
    /// the menu bar's service for programs, and the program whose menu is open
    pub appmenu: crate::appmenu_service::AppMenuService,
    /// what was last handed to other programs (see `appstyle.rs`)
    pub appstyle_last: Option<crate::appstyle::AppStyle>,
    /// the events being handled come from the desktop's icons (see `handle_desktop_events`)
    pub events_from_desktop: bool,
    pub appmenu_target: Option<crate::appmenu_service::MenuTarget>,
    /// the program menu open from a window's title bar: the window's id and the menu's name (see `title_menu.rs`)
    pub title_menu: Option<(u64, String)>,
    /// whose menu the open command search lists (see `State::menu_search`)
    pub menu_search_target: Option<crate::appmenu_service::MenuTarget>,
    /// following the GTK theme: the reading in progress, when to look next, what was found last
    pub gtk_rx: Option<std::sync::mpsc::Receiver<Option<memrium_de::gtk::GtkLook>>>,
    pub gtk_next: Instant,
    pub gtk_last: Option<memrium_de::gtk::GtkLook>,
    pub tray_menu: Option<(String, String)>,
    /// notification service and the toasts on screen
    pub notifications: crate::notify::Notifications,
    pub clipboard: crate::clipboard::Clipboard,
    /// the volume / brightness slider that a tray icon opens
    pub flyout: Option<crate::flyout::Flyout>,
    /// the power card of the battery icon, and whether sleep and screen locking were blocked by hand
    pub card: Option<crate::popup::Popup>,
    /// MEMRIUM_DEBUG_POINTER: how many raw motions were sent to clients
    pub dbg_rel: u64,
    /// MEMRIUM_DEBUG_POINTER: the lock as last seen, and the raw motions since it changed
    pub dbg_lock: (bool, u32),
    /// where a program moved the pointer to (XWarpPointer in a game, via the locked pointer's position hint); the input loop takes it
    pub warp_to: Option<[f32; 2]>,
    /// the X11 surface under the pointer and its true origin (while a button is held the pointer may be outside it)
    pub x_focus_origin: Option<(WlSurface, [f64; 2])>,
    pub manual_block: bool,
    /// start and centre (device px) of a theme-switch circle in progress
    pub reveal: Option<(Instant, [f32; 2])>,
    pub flyout_last_tick: Instant,
    /// current virtual desktop (windows of the others are put away)
    pub workspace: usize,
    /// where the desktops are shown: 1.0 is the second one in place, 0.5 half way between the first and the second
    pub ws_pos: Spring,
    /// Alt+Tab in progress: the windows as they were ordered when it began (most recently used first) and which one is chosen
    pub alt_tab: Option<(Vec<u64>, usize)>,
    /// a swipe is moving the desktops: (position it started at, how far the fingers went in desktops, speed in desktops / s)
    pub ws_drag: Option<(f32, f32, f32)>,
    /// the desktop's widgets can be moved
    /// the user manager's DISPLAY / WAYLAND_DISPLAY ... as they were before this session changed them
    pub saved_env: Vec<(String, Option<String>)>,
    /// the media player watcher, started when something wants it
    pub media: Option<crate::media::Media>,
    /// the cover in the atlas and the record's turning: angle, angular speed (rad/s)
    pub album: crate::desk_widgets::Album,
    /// the wallpaper has been loaded once (a later layout change reloads it, fitted to the monitors)
    pub wall_ready: bool,
    /// the next video frame is the first one of this wallpaper
    pub video_first: bool,
    /// the video wallpaper being played, if any
    pub video: Option<memrium_de::video::VideoWall>,
    /// listening to the sound for a wallpaper that dances to it
    pub audio_tap: Option<memrium_de::audiotap::AudioTap>,
    /// the visualizer widget's own listener (when the wallpaper is not listening already)
    pub viz_tap: Option<memrium_de::audiotap::AudioTap>,
    /// the monitor the bars are being laid out / hit-tested for (the bars repeat on every monitor)
    pub bar_mon: std::cell::Cell<usize>,
    /// the monitor of the hovered widget and of the open bar menu
    pub bar_hover_mon: usize,
    pub bar_menu_mon: usize,
    /// desktop widgets: the one being dragged (index, grab offset), the one a menu is about, where "Add" was asked
    pub desk_drag: Option<(usize, [f32; 2])>,
    /// where the dragged widget was when the drag began, the lines it is pulled onto, and the pulling the keys chose
    pub desk_drag_from: [f32; 2],
    pub desk_guides: Vec<memrium_de::deskwidgets::Guide>,
    pub desk_snap: memrium_de::deskwidgets::Snap,
    pub desk_target: Option<usize>,
    /// the desktop: (x0, y0, x1, y1) logical px; the main monitor's top-left is (0, 0)
    pub world: [f32; 4],
    /// every monitor as (x, y, w, h) logical px, and which one is the main one
    pub mons: Vec<[f32; 4]>,
    pub main_mon: usize,
    /// names, sizes and modes of the connected monitors, for the Displays page (set by the session)
    pub monitor_infos: Vec<memrium_de::display::MonitorInfo>,
    /// the Displays settings changed: the session should set the outputs up again
    pub displays_dirty: bool,
    pub widgets_panel: crate::widgets_panel::WidgetsPanel,
    /// text input (IMEs), input methods and virtual keyboards (on-screen keyboards, `wtype`)
    pub text_input_state: smithay::wayland::text_input::TextInputManagerState,
    pub input_method_state: smithay::wayland::input_method::InputMethodManagerState,
    pub virtual_keyboard_state: smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState,
    /// the event loop, once XWayland needs it (clipboard transfers)
    pub loop_handle: Option<smithay::reexports::calloop::LoopHandle<'static, State>>,
    /// when "Auto" appearance last looked at the clock
    pub auto_dark_checked: Option<Instant>,
    /// when the user last touched anything, which idle steps already ran, and whether the display is off
    pub last_input: Instant,
    pub idle_done: [bool; 3],
    pub screen_off: bool,
    pub inhibitors: Vec<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    pub idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState,
    /// the animation settings of the theme being switched to (they decide how the switch looks)
    pub theme_next_anim: memrium_de::theme::AnimStyle,
    /// pointer locking, raw motion, cursor shapes, activation
    pub protocols: crate::protocols::Protocols,
    /// a real session (not a nested window): D-Bus activated services (portals) get our display variables
    pub export_env: bool,
    /// screen-capture requests (wlr-screencopy) waiting to be served
    pub screencopies: Vec<crate::screencopy::PendingCopy>,
    pub ext: crate::ext_capture::ExtCapture,
    /// GPU buffer sharing (linux-dmabuf): the protocol state and the textures made from imported buffers
    pub dmabuf_state: smithay::wayland::dmabuf::DmabufState,
    pub dmabuf_global: Option<smithay::wayland::dmabuf::DmabufGlobal>,
    pub dmabuf_tex: HashMap<smithay::reexports::wayland_server::backend::ObjectId, Rc<SurfaceTex>>,
    /// how many imports were reported in the log so far
    pub dmabuf_logged: u32,
    /// explicit sync (linux-drm-syncobj): the NVIDIA driver hands Xwayland frames it has not finished drawing; the
    /// timeline point says when it has
    pub syncobj_state: Option<smithay::wayland::drm_syncobj::DrmSyncobjState>,
    /// buffers replaced since the last frame went to the GPU: our frames still in flight may sample them, so they go back to
    /// their clients only once that work is done (`take_retired`). Off (handed back at once) unless the backend asks.
    pub defer_release: bool,
    pub retired: Vec<Held>,
}

pub(crate) fn ellipsize(r: &Renderer, text: &str, px: f32, max_w: f32) -> String {
    if r.text_width(text, px, true) <= max_w {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while !s.is_empty() && r.text_width(&format!("{s}…"), px, true) > max_w {
        s.pop();
    }
    format!("{s}…")
}

/// All surfaces of a tree, bottom to top, with their offset from the root surface's origin.
pub fn surface_tree(root: &WlSurface) -> Vec<(WlSurface, [i32; 2])> {
    let mut out = Vec::new();
    with_surface_tree_upward(
        root,
        (0i32, 0i32),
        |_, states, loc| {
            let mut l = *loc;
            if states.role == Some("subsurface") {
                let p = states.cached_state.get::<SubsurfaceCachedState>().current().location;
                l = (l.0 + p.x, l.1 + p.y);
            }
            TraversalAction::DoChildren(l)
        },
        |surface, states, loc| {
            // the processor receives the parent's accumulated offset: add our own
            let mut l = *loc;
            if states.role == Some("subsurface") {
                let p = states.cached_state.get::<SubsurfaceCachedState>().current().location;
                l = (l.0 + p.x, l.1 + p.y);
            }
            out.push((surface.clone(), [l.0, l.1]));
        },
        |_, _, _| true,
    );
    out
}

pub(crate) fn root_of(surface: &WlSurface) -> WlSurface {
    let mut s = surface.clone();
    while let Some(p) = get_parent(&s) {
        s = p;
    }
    s
}

impl State {
    pub fn time_ms(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    // ------------------------------------------------------------ lookup helpers

    pub fn win_index(&self, root: &WlSurface) -> Option<usize> {
        self.wins.iter().position(|w| w.role.is_surface(root))
    }

    fn surf_size(&self, s: &WlSurface) -> Option<[f32; 2]> {
        self.surfaces.get(&s.id())?.logical_size()
    }

    fn popup_index(&self, s: &WlSurface) -> Option<usize> {
        self.popups.iter().position(|p| p.surface.wl_surface() == s)
    }

    /// Frame rectangle (x, y, w, h) of a window in world coordinates.
    pub fn frame_rect(w: &Win) -> [f32; 4] {
        [w.x.x, w.y.x, w.geo[2] + w.ins[0] + w.ins[2], w.geo[3] + w.ins[1] + w.ins[3]]
    }

    /// World position of the top-left of a toplevel's or popup's xdg geometry.
    fn geometry_origin(&self, surface: &WlSurface) -> Option<[f32; 2]> {
        if let Some(i) = self.win_index(surface) {
            let w = &self.wins[i];
            return Some([w.x.x + w.ins[0], w.y.x + w.ins[1]]);
        }
        let pi = self.popup_index(surface)?;
        let parent = self.popups[pi].surface.get_parent_surface()?;
        let po = self.geometry_origin(&root_of(&parent))?;
        // the popup's position is relative to its parent's geometry; subsurface parents are rare, ignore their offset
        let g = self.popups[pi].surface.with_pending_state(|s| s.geometry);
        Some([po[0] + g.loc.x as f32, po[1] + g.loc.y as f32])
    }

    pub fn geometry_origin_pub(&self, surface: &WlSurface) -> Option<[f32; 2]> {
        self.geometry_origin(&root_of(surface))
    }

    /// World position of a root surface's own (0, 0): geometry origin minus the geometry's offset inside the surface.
    fn root_origin(&self, root: &WlSurface) -> Option<[f32; 2]> {
        let go = self.geometry_origin(root)?;
        let gl = with_states(root, |st| st.cached_state.get::<SurfaceCachedState>().current().geometry).map(|g| [g.loc.x as f32, g.loc.y as f32]).unwrap_or([0.0, 0.0]);
        Some([go[0] - gl[0], go[1] - gl[1]])
    }

    // ------------------------------------------------------------ surface buffers

    /// Debug (`MEMRIUM_WHY=1`): counts the commits per program (and buffer kind), written every five seconds.
    fn why_commit(&self, surface: &WlSurface) {
        use std::cell::RefCell;
        use std::collections::HashMap;
        thread_local!(static C: RefCell<(Option<std::time::Instant>, HashMap<String, u32>)> = RefCell::new((None, HashMap::new())));
        let mut root = surface.clone();
        while let Some(p) = smithay::wayland::compositor::get_parent(&root) {
            root = p;
        }
        let app = smithay::wayland::compositor::with_states(&root, |st| {
            st.data_map.get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>().and_then(|d| d.lock().ok().and_then(|a| a.app_id.clone()))
        });
        let kind = if std::ptr::eq(&root, surface) || root == *surface { "root" } else { "sub" };
        let key = format!("{}/{}", app.unwrap_or_else(|| "?".into()), kind);
        C.with(|c| {
            let mut c = c.borrow_mut();
            let t = *c.0.get_or_insert_with(std::time::Instant::now);
            *c.1.entry(key).or_default() += 1;
            if t.elapsed().as_secs_f32() >= 5.0 {
                let mut v: Vec<_> = c.1.drain().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1));
                eprintln!("commits/5s: {:?}", v);
                c.0 = Some(std::time::Instant::now());
            }
        });
    }

    pub fn on_commit(&mut self, surface: &WlSurface) {
        if std::env::var_os("MEMRIUM_WHY").is_some() {
            self.why_commit(surface);
        }
        // 1. take the pending buffer of any surface (toplevel, subsurface, popup, cursor...)
        // the damage piles up from commit to commit until it is taken, so it is taken every time
        let (assignment, scale, damage, plain, opaque) = with_states(surface, |st| {
            let mut g = st.cached_state.get::<SurfaceAttributes>();
            let a = g.current();
            let plain = a.buffer_transform == smithay::reexports::wayland_server::protocol::wl_output::Transform::Normal;
            (a.buffer.take(), a.buffer_scale, std::mem::take(&mut a.damage), plain, a.opaque_region.as_ref().and_then(opaque_rect))
        });
        match assignment {
            Some(BufferAssignment::NewBuffer(buf)) => {
                if smithay::wayland::dmabuf::get_dmabuf(&buf).is_ok() {
                    // a GPU buffer is sampled in place and handed back when the next one arrives
                    self.attach_dmabuf(surface, &buf, scale);
                } else {
                    // the rows the program says it redrew (a viewport or a turned buffer: all of them)
                    let viewport = with_states(surface, |st| st.cached_state.get::<smithay::wayland::viewporter::ViewportCachedState>().current().size().is_some());
                    let rows = if plain && !viewport { damage_rows(&damage, scale.max(1)) } else { None };
                    self.upload(surface, &buf, scale, rows);
                    // copied out already: the client may have it back at once
                    let release = with_states(surface, |st| st.cached_state.get::<smithay::wayland::drm_syncobj::DrmSyncobjCachedState>().current().release_point.take());
                    Held { buf: Some(buf), release }.done();
                }
            }
            Some(BufferAssignment::Removed) => {
                if let Some(n) = self.surfaces.get_mut(&surface.id()) {
                    n.tex = None;
                    n.small = None;
                    if let Some(old) = n.dma_buf.take() {
                        self.retire(old);
                    }
                }
            }
            None => {}
        }

        // the viewport (a program drawing for a fractional scale): the size the surface is shown at
        let dst = with_states(surface, |st| st.cached_state.get::<smithay::wayland::viewporter::ViewportCachedState>().current().size());
        let is_x11 = self.wins.iter().any(|w| w.role.is_x11() && w.role.is_surface(surface));
        // the opaque region of an X11 window is in X pixels too
        let opaque = if is_x11 { opaque.map(|o| o.map(|v| v / crate::role::x_scale())) } else { opaque };
        if let Some(n) = self.surfaces.get_mut(&surface.id()) {
            n.x11 = is_x11;
            n.logical = dst.filter(|d| d.w > 0 && d.h > 0).map(|d| [d.w as f32, d.h as f32]);
            // a buffer with no alpha is opaque all over, whatever the region says
            n.opaque = match (n.tex.as_ref().is_some_and(|t| t.opaque), n.logical_size()) {
                (true, Some(sz)) => Some([0.0, 0.0, sz[0], sz[1]]),
                _ => opaque,
            };
        }

        // 2. role-specific bookkeeping on the root
        let root = root_of(surface);
        if let Some(i) = self.win_index(&root) {
            self.wins[i].role.send_initial_configure();
            self.refresh_geometry(i);
            let has_buf = self.surfaces.get(&root.id()).is_some_and(|n| n.tex.is_some());
            if has_buf && !self.wins[i].mapped {
                // a splash screen (one fixed size, no parent) is shown as it is, with no frame of ours
                if self.wins[i].deco == Deco::Server && self.wins[i].role.is_splash() {
                    self.wins[i].deco = Deco::Legacy;
                    self.wins[i].ins = [0.0; 4];
                }
                self.wins[i].mapped = true;
                self.wins[i].open_k.target = 1.0;
                self.output.enter(&root);
                // fullscreen asked for before the window was shown, or a window of one fixed size as large as a monitor (a game
                // in "fullscreen" that only made its window the screen's size, as CS2 on Wayland): it comes up so, not as a
                // window at the cascade
                let g = self.wins[i].geo;
                let screen_sized = self.wins[i].role.fixed_size() && self.mons.iter().any(|m| g[2] >= m[2] - 1.0 && g[3] >= m[3] - 1.0);
                if self.wins[i].fs.is_none() && (self.wins[i].role.asked_fullscreen() || screen_sized) {
                    self.fullscreen_now(i, true);
                    let w = &mut self.wins[i];
                    w.x.snap();
                    w.y.snap();
                }
                // an X11 window is asked to be focused before Xwayland has given it a surface: now it has one
                self.apply_focus();
            }
        }
    }

    fn refresh_geometry(&mut self, i: usize) {
        let Some(root) = self.wins[i].role.surface() else { return };
        let size = self.surf_size(&root).unwrap_or([0.0, 0.0]);
        let g = with_states(&root, |st| st.cached_state.get::<SurfaceCachedState>().current().geometry);
        self.wins[i].geo = match g {
            Some(g) if g.size.w > 0 && g.size.h > 0 => [g.loc.x as f32, g.loc.y as f32, g.size.w as f32, g.size.h as f32],
            _ => [0.0, 0.0, size[0], size[1]],
        };
        let (title, app_id) = self.wins[i].role.title_and_app();
        self.wins[i].title = title;
        self.wins[i].app_id = app_id;
    }

    /// Take the picture of the program's own cursor from its surface. A new picture makes the cursor be drawn again.
    pub fn read_client_cursor(&mut self) {
        use smithay::input::pointer::CursorImageSurfaceData;
        let Some(surface) = self.client_cursor_surface.clone() else { return };
        let hot = with_states(&surface, |s| s.data_map.get::<CursorImageSurfaceData>().and_then(|d| d.lock().ok().map(|a| (a.hotspot.x, a.hotspot.y)))).unwrap_or((0, 0));
        let img = self.surfaces.get(&surface.id()).and_then(|n| {
            let (bytes, w, h, stride, opaque) = n.small.as_ref()?;
            memrium_de::cursor::ClientCursor::from_bgra(bytes, *w, *h, *stride, *opaque, n.scale, hot)
        });
        if img != self.client_cursor_img {
            self.client_cursor_img = img;
            self.cursor_gen = self.cursor_gen.wrapping_add(1);
        }
        // Xwayland names the cursor's surface before it has a picture on it: the program's own cursor is shown once it has come
        if self.client_cursor_img.is_some() && self.client_cursor != memrium_de::cursor::Shape::Client {
            self.client_cursor = memrium_de::cursor::Shape::Client;
            self.update_cursor_shape();
        }
    }

    /// Copy a client's shm buffer into its texture; `rows` (buffer rows) limits the copy to what the client redrew.
    fn upload(&mut self, surface: &WlSurface, buf: &WlBuffer, scale: i32, rows: Option<std::ops::Range<u32>>) {
        // the shape of the buffer first (BGRA byte order for both ARGB/XRGB8888)
        let shape = with_buffer_contents(buf, |_, len, data| {
            if !matches!(data.format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888) {
                return None;
            }
            let (w, h, stride, off) = (data.width as usize, data.height as usize, data.stride as usize, data.offset as usize);
            if w == 0 || h == 0 || stride < w * 4 || off + stride * h > len {
                return None;
            }
            Some((w as u32, h as u32, stride as u32, data.format == wl_shm::Format::Xrgb8888))
        });
        let Ok(Some((w, h, stride, opaque))) = shape else {
            eprintln!("unsupported or invalid buffer");
            return;
        };
        // a big picture arriving through shared memory is copied by the CPU every frame (a game under Xwayland that does not
        // hand over GPU buffers): said a few times so the log shows which path a full-screen program takes
        static BIG_SHM: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if w as u64 * h as u64 >= 1_000_000 && BIG_SHM.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 4 {
            eprintln!("shm: {w}x{h} picture copied by the CPU (rows {:?}, not a dmabuf)", rows);
        }
        let node = self.surfaces.entry(surface.id()).or_insert_with(|| SurfNode { surface: surface.clone(), tex: None, scale: 1, dma_buf: None, small: None, logical: None, x11: false, opaque: None, body: Vec::new() });
        node.scale = scale.max(1);
        // a texture that came from a shared GPU buffer cannot be written to
        if let Some(old) = node.dma_buf.take() {
            node.tex = None;
            self.retire(old);
        }
        let node = self.surfaces.get_mut(&surface.id()).unwrap();
        let reuse = node.tex.as_ref().is_some_and(|t| t.size == (w, h) && t.opaque == opaque);
        if !reuse {
            node.tex = Some(Rc::new(self.renderer.create_surface_tex(w, h, opaque)));
        }
        // a new texture needs all of it; otherwise only the redrawn rows go up, straight from the pool
        let rows = rows.filter(|_| reuse).unwrap_or(0..h);
        let tex = node.tex.clone().unwrap();
        let is_popup = self.popup_index(surface).is_some();
        let renderer = &self.renderer;
        let picked = with_buffer_contents(buf, |ptr, _, data| {
            let all = unsafe { std::slice::from_raw_parts(ptr.add(data.offset as usize), stride as usize * h as usize) };
            let r0 = rows.start.min(h) as usize;
            let r1 = (rows.end.min(h) as usize).max(r0);
            renderer.upload_surface_rows(&tex, &all[r0 * stride as usize..r1 * stride as usize], stride, rows.clone());
            let small = (w <= 128 && h <= 128).then(|| all.to_vec());
            let body = if is_popup && !opaque && w <= 1600 && h <= 1600 { solid_parts(all, w as usize, h as usize, stride as usize) } else { Vec::new() };
            (small, top_edge_luma(all, w as usize, h as usize, stride as usize, opaque), body)
        });
        let Ok((small, luma, body)) = picked else { return };
        let node = self.surfaces.get_mut(&surface.id()).unwrap();
        node.body = body;
        node.small = small.map(|bytes| (bytes, w, h, stride, opaque));
        if self.client_cursor_surface.as_ref() == Some(surface) {
            self.read_client_cursor();
        }
        // the frame of a window follows its content: a dead band keeps it from flickering around the middle
        if let Some(i) = self.win_index(surface) {
            if let Some(l) = luma {
                let prev = self.wins[i].content_dark;
                self.wins[i].content_dark = match prev {
                    Some(d) if (0.40..0.60).contains(&l) => Some(d),
                    _ => Some(l < 0.5),
                };
            }
        }
    }

    /// Show a client's DMA-BUF: its imported texture is used directly, no pixels are copied.
    fn attach_dmabuf(&mut self, surface: &WlSurface, buf: &WlBuffer, scale: i32) {
        let tex = match self.dmabuf_tex.get(&buf.id()) {
            Some(t) => t.clone(),
            None => {
                // imported at creation normally; this covers buffers created without a prior check
                let Ok(d) = smithay::wayland::dmabuf::get_dmabuf(buf) else { return };
                match self.import_dmabuf(d) {
                    Ok(t) => {
                        let t = Rc::new(t);
                        self.dmabuf_tex.insert(buf.id(), t.clone());
                        t
                    }
                    Err(e) => {
                        eprintln!("dmabuf: {e}");
                        buf.release();
                        return;
                    }
                }
            }
        };
        let node = self.surfaces.entry(surface.id()).or_insert_with(|| SurfNode { surface: surface.clone(), tex: None, scale: 1, dma_buf: None, small: None, logical: None, x11: false, opaque: None, body: Vec::new() });
        node.scale = scale.max(1);
        node.tex = Some(tex.clone());
        // the point the client wants signalled when we are done with this buffer (explicit sync)
        let release = with_states(surface, |st| st.cached_state.get::<smithay::wayland::drm_syncobj::DrmSyncobjCachedState>().current().release_point.take());
        if let Some(old) = node.dma_buf.replace(Held { buf: Some(buf.clone()), release }) {
            if old.buf.as_ref() != Some(buf) {
                self.retire(old);
            } else if old.release.is_some() {
                // the same buffer again: it stays ours, only the old point is owed
                self.retire(Held { buf: None, release: old.release });
            }
        }
        // the same buffer may be redrawn by the client in place: its identity says nothing about its contents. Only
        // this surface is marked, so the rest of the screen (glass, blur) is not drawn again for it.
        tex.touch();
    }

    /// Import a client's DMA-BUF into the renderer's GPU device.
    pub fn import_dmabuf(&self, d: &smithay::backend::allocator::dmabuf::Dmabuf) -> Result<SurfaceTex, String> {
        use smithay::backend::allocator::Buffer;
        let fmt = d.format();
        let mut planes = Vec::new();
        for ((fd, offset), stride) in d.handles().zip(d.offsets()).zip(d.strides()) {
            planes.push(memrium_de::renderer::DmaPlane { fd: fd.try_clone_to_owned().map_err(|e| e.to_string())?, offset, stride });
        }
        let size = d.size();
        self.renderer.import_dmabuf(size.w as u32, size.h as u32, fmt.code as u32, u64::from(fmt.modifier), &planes)
    }

    /// The shell's cursor as a theme every program can load, and the variables that point them at it.
    pub fn export_cursor_theme(&self) {
        let prefs = &self.config.input.cursor;
        memrium_de::cursor_theme::export(prefs, self.theme.accent, memrium_de::kdecolors::active());
        for (k, v) in memrium_de::cursor_theme::env(prefs) {
            std::env::set_var(k, v);
        }
    }

    /// A buffer we no longer show: back to its client now, or once the frames that may sample it are done.
    pub fn retire(&mut self, h: Held) {
        if self.defer_release {
            self.retired.push(h);
        } else {
            h.done();
        }
    }

    /// The buffers retired so far, to be handed back when the GPU work submitted after this has finished.
    pub fn take_retired(&mut self) -> Vec<Held> {
        std::mem::take(&mut self.retired)
    }

    /// Advertise explicit sync, with the render node of the GPU we draw on (`vendor`: its PCI vendor id) to import timelines.
    pub fn init_syncobj(&mut self, vendor: u32) {
        use smithay::backend::drm::DrmDeviceFd;
        if std::env::var("MEMRIUM_SKIP").is_ok_and(|v| v.contains("syncobj")) {
            return;
        }
        let node = (128..192).map(|i| format!("renderD{i}")).find(|n| {
            std::fs::read_to_string(format!("/sys/class/drm/{n}/device/vendor")).ok().and_then(|v| u32::from_str_radix(v.trim().trim_start_matches("0x"), 16).ok()) == Some(vendor)
        });
        let Some(node) = node else {
            eprintln!("syncobj: no render node of vendor {vendor:#x}");
            return;
        };
        let file = match std::fs::OpenOptions::new().read(true).write(true).open(format!("/dev/dri/{node}")) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("syncobj: /dev/dri/{node}: {e}");
                return;
            }
        };
        let fd = DrmDeviceFd::new(smithay::utils::DeviceFd::from(std::os::fd::OwnedFd::from(file)));
        if !smithay::wayland::drm_syncobj::supports_syncobj_eventfd(&fd) {
            eprintln!("syncobj: /dev/dri/{node} cannot wait on timelines; no explicit sync");
            return;
        }
        self.syncobj_state = Some(smithay::wayland::drm_syncobj::DrmSyncobjState::new::<State>(&self.dh, fd));
        eprintln!("syncobj: explicit sync offered (timelines imported on /dev/dri/{node})");
    }

    /// Advertise linux-dmabuf when the GPU can import buffers.
    pub fn init_dmabuf(&mut self) {
        use smithay::backend::allocator::{Format, Fourcc, Modifier};
        use smithay::wayland::dmabuf::DmabufFeedbackBuilder;
        let Some(caps) = self.renderer.dmabuf_caps().cloned() else {
            eprintln!("dmabuf: not available on this GPU (clients use shared memory)");
            return;
        };
        let formats: Vec<Format> = caps.formats.iter().filter_map(|&(code, m)| Some(Format { code: Fourcc::try_from(code).ok()?, modifier: Modifier::from(m) })).collect();
        let dh = self.dh.clone();
        let global = match caps.render_dev.map(|dev| DmabufFeedbackBuilder::new(dev, formats.iter().copied()).build()) {
            Some(Ok(feedback)) => self.dmabuf_state.create_global_with_default_feedback::<State>(&dh, &feedback),
            _ => self.dmabuf_state.create_global::<State>(&dh, formats.iter().copied()),
        };
        eprintln!("dmabuf: {} format/modifier pairs advertised", formats.len());
        self.dmabuf_global = Some(global);
    }

    pub fn send_frame_callbacks(&mut self) {
        let time = self.time_ms();
        let mut roots: Vec<WlSurface> = self.wins.iter().filter_map(|w| w.role.surface()).collect();
        roots.extend(self.popups.iter().map(|p| p.surface.wl_surface().clone()));
        roots.extend(self.ime_popups.iter().map(|p| p.wl_surface().clone()));
        for r in roots {
            for (s, _) in surface_tree(&r) {
                with_states(&s, |st| {
                    for cb in st.cached_state.get::<SurfaceAttributes>().current().frame_callbacks.drain(..) {
                        cb.done(time);
                    }
                });
            }
        }
    }

    // ------------------------------------------------------------ focus / z-order

    /// Raise and focus window `idx`; returns where it ended up (override-redirect windows stay above it).
    pub fn focus_window(&mut self, idx: usize) -> usize {
        if self.wins[idx].or {
            return idx;
        }
        // a window on another desktop brings its desktop along
        if self.wins[idx].ws != self.workspace {
            let ws = self.wins[idx].ws;
            self.switch_workspace(ws);
        }
        self.wins[idx].ws_hidden = false;
        let mut w = self.wins.remove(idx);
        if w.minimized {
            w.minimized = false;
            w.min_k.target = 0.0;
        }
        w.stamp = self.next_stamp();
        let at = self.wins.iter().rposition(|o| !o.or).map(|p| p + 1).unwrap_or(0);
        self.wins.insert(at, w);
        self.apply_focus();
        at
    }

    /// A new place at the front of the stacking order (shell windows and client windows share one clock).
    pub fn next_stamp(&mut self) -> u64 {
        self.stack_clock += 1;
        self.stack_clock
    }

    /// The stamp of the frontmost client window that is shown.
    fn top_client_stamp(&self) -> u64 {
        self.wins.iter().filter(|w| w.mapped && !w.minimized && !w.or).map(|w| w.stamp).max().unwrap_or(0)
    }

    /// The stamp of the client window (or popup: always in front) that has a point of its own at `p`, if any does.
    fn client_stamp_at(&self, p: [f32; 2]) -> Option<u64> {
        let from_chrome = self.chrome_at(p).map(|(i, _)| if self.wins[i].or { u64::MAX } else { self.wins[i].stamp });
        let from_surface = self.surface_under(p).map(|(s, _)| self.win_index(&root_of(&s)).map_or(u64::MAX, |i| if self.wins[i].or { u64::MAX } else { self.wins[i].stamp }));
        from_chrome.max(from_surface)
    }

    /// Index of the frontmost window that can take the focus.
    fn top_managed(&self) -> Option<usize> {
        self.wins.iter().rposition(|w| !w.or)
    }

    pub fn apply_focus(&mut self) {
        // override-redirect windows (X11 menus) never take the focus
        let last = self.wins.iter().rposition(|w| !w.minimized && !w.or);
        // a shell window in front holds the focus: no client window is active, nor gets the keys
        let shell = self.focused_shell();
        self.shell_focus_seen = shell;
        for (i, w) in self.wins.iter().enumerate() {
            if !w.or {
                w.role.set_active(shell.is_none() && Some(i) == last);
            }
        }
        // an X11 window is focused as itself, so the X server gives it the input focus too
        let target = last.filter(|_| shell.is_none()).and_then(|i| match &self.wins[i].role {
            Role::X11(x) => x.wl_surface().is_some().then(|| crate::focus::KbFocus::X11(x.clone())),
            r => r.surface().map(crate::focus::KbFocus::Wl),
        });
        let kb = self.keyboard.clone();
        kb.set_focus(self, target, SERIAL_COUNTER.next_serial());
    }

    // ------------------------------------------------------------ window operations

    /// Area left for windows: between the bars (or the taskbar).
    pub fn work_area(&self) -> [f32; 4] {
        let (mut left, mut top, mut right, mut bottom) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        self.with_bar_ctx(|ctx| {
            for b in &self.bars {
                let [l, t, r, bt] = b.reserved(ctx);
                left = left.max(l);
                top = top.max(t);
                right = right.max(r);
                bottom = bottom.max(bt);
            }
        });
        if bottom > 0.0 {
            bottom += 6.0;
        }
        if left > 0.0 {
            left += 6.0;
        }
        if right > 0.0 {
            right += 6.0;
        }
        [left, top, (self.size[0] - left - right).max(100.0), (self.size[1] - top - bottom).max(100.0)]
    }

    pub fn toggle_maximize(&mut self, idx: usize) {
        if self.wins[idx].role.fixed_size() {
            return;
        }
        if self.wins[idx].fs.is_some() {
            self.set_fullscreen(idx, false);
            return;
        }
        let f = Self::frame_rect(&self.wins[idx]);
        let area = self.maximize_area([f[0] + f[2] / 2.0, f[1] + f[3] / 2.0]);
        let (hide, normal) = (self.theme.window.hide_title_max, decor::insets(&self.theme));
        // a window that is asked to maximize before it has any size (Kate does) must not "restore" to nothing: a frame too
        // small to be a window is replaced by a comfortable one in the middle of the area
        let comfy = [area[0] + area[2] * 0.14, area[1] + area[3] * 0.10, area[2] * 0.72, area[3] * 0.80];
        let (nx, ny) = (normal[0] + normal[2], normal[1] + normal[3]);
        let too_small = |r: &[f32; 4]| r[2] - nx < 240.0 || r[3] - ny < 160.0;
        let w = &mut self.wins[idx];
        if let Some(r) = w.restore.take() {
            w.tile = 0;
            let r = if too_small(&r) { comfy } else { r };
            // back to a window with its title bar
            if w.deco == Deco::Server {
                w.ins = normal;
            }
            let (ix, iy) = (w.ins[0] + w.ins[2], w.ins[1] + w.ins[3]);
            w.x.target = r[0];
            w.y.target = r[1];
            w.role.configure(Some((((r[2] - ix) as i32).max(1), ((r[3] - iy) as i32).max(1))), ((r[0] + w.ins[0]) as i32, (r[1] + w.ins[1]) as i32), Some(false), None);
        } else {
            let before = Self::frame_rect(w);
            w.restore = Some(if too_small(&before) { comfy } else { before });
            // the title bar is left out of a maximized window when the setting says so
            if hide && w.deco == Deco::Server {
                w.ins = [0.0; 4];
            }
            let (ix, iy) = (w.ins[0] + w.ins[2], w.ins[1] + w.ins[3]);
            w.x.target = area[0];
            w.y.target = area[1];
            w.role.configure(Some((((area[2] - ix) as i32).max(1), ((area[3] - iy) as i32).max(1))), ((area[0] + w.ins[0]) as i32, (area[1] + w.ins[1]) as i32), Some(true), None);
        }
    }

    /// The part of the work area a snapped window takes: -1 / 1 its left / right half, 2 / 3 the top left / right quarter,
    /// 4 / 5 the bottom ones, 0 all of it.
    pub fn snap_rect(&self, p: [f32; 2], side: i8) -> [f32; 4] {
        let a = self.maximize_area(p);
        let (hw, hh) = (a[2] / 2.0 - 3.0, a[3] / 2.0 - 3.0);
        let (rx, by) = (a[0] + a[2] / 2.0 + 3.0, a[1] + a[3] / 2.0 + 3.0);
        match side {
            -1 => [a[0], a[1], hw, a[3]],
            1 => [rx, a[1], hw, a[3]],
            2 => [a[0], a[1], hw, hh],
            3 => [rx, a[1], hw, hh],
            4 => [a[0], by, hw, hh],
            5 => [rx, by, hw, hh],
            _ => a,
        }
    }

    /// Snap a window to the left (-1) or right (1) half of the screen it is on. It keeps its title bar; `restore` is where it
    /// came from, so pulling it off the edge (or the maximize button) gives it back.
    pub fn tile_window(&mut self, idx: usize, side: i8) {
        let f = Self::frame_rect(&self.wins[idx]);
        self.tile_window_on(idx, side, [f[0] + f[2] / 2.0, f[1] + f[3] / 2.0]);
    }

    /// The same on the monitor that has the point `p` (a window sent to the next monitor).
    pub fn tile_window_on(&mut self, idx: usize, side: i8, p: [f32; 2]) {
        if self.wins[idx].role.fixed_size() || self.wins[idx].fs.is_some() {
            return;
        }
        let f = Self::frame_rect(&self.wins[idx]);
        let r = self.snap_rect(p, side);
        // a snapped window is dressed like a maximized one: where the theme moves the title into the bar, it does so here too
        let ins = if self.theme.window.hide_title_max { [0.0; 4] } else { decor::insets(&self.theme) };
        let w = &mut self.wins[idx];
        if w.restore.is_none() {
            w.restore = Some(f);
        }
        w.tile = side;
        if w.deco == Deco::Server {
            w.ins = ins;
        }
        let (ix, iy) = (w.ins[0] + w.ins[2], w.ins[1] + w.ins[3]);
        w.x.target = r[0];
        w.y.target = r[1];
        w.role.configure(Some((((r[2] - ix) as i32).max(1), ((r[3] - iy) as i32).max(1))), ((r[0] + w.ins[0]) as i32, (r[1] + w.ins[1]) as i32), Some(false), None);
    }

    pub fn tile_shell_window(&mut self, k: Sw, side: i8) {
        let c = self.sw_frame(k);
        let r = self.snap_rect([c[0] + c[2] / 2.0, c[1] + c[3] / 2.0], side);
        let ins = if self.theme.window.hide_title_max { [0.0; 4] } else { decor::insets(&self.theme) };
        let w = self.shell_win_mut(k);
        if w.restore.is_none() {
            w.restore = Some([w.x.target, w.y.target]);
        }
        w.tile = side;
        w.size = Some([r[2] - ins[0] - ins[2], r[3] - ins[1] - ins[3]]);
        w.x.target = r[0];
        w.y.target = r[1];
    }

    /// What an arrow key does to a window (Windows' way): `cur` is how it is now, None floating, Some(0) maximized, Some(tile) snapped.
    fn snap_op(cur: Option<i8>, key: SnapKey) -> SnapOp {
        use SnapKey::*;
        use SnapOp::*;
        match (key, cur) {
            (Left, None | Some(0)) => Tile(-1),
            (Left, Some(1)) => Restore,
            (Left, Some(-1)) => Beside(-1),
            (Left, Some(3)) => Tile(2),
            (Left, Some(5)) => Tile(4),
            (Left, Some(2 | 4)) => Beside(-1),
            (Right, None | Some(0)) => Tile(1),
            (Right, Some(-1)) => Restore,
            (Right, Some(1)) => Beside(1),
            (Right, Some(2)) => Tile(3),
            (Right, Some(4)) => Tile(5),
            (Right, Some(3 | 5)) => Beside(1),
            (Up, None) => Maximize,
            (Up, Some(0)) => Restore,
            (Up, Some(-1)) => Tile(2),
            (Up, Some(1)) => Tile(3),
            (Up, Some(4)) => Tile(-1),
            (Up, Some(5)) => Tile(1),
            (Up, Some(_)) => Maximize,
            (Down, Some(0)) => Restore,
            (Down, None) => Minimize,
            (Down, Some(-1)) => Tile(4),
            (Down, Some(1)) => Tile(5),
            (Down, Some(2)) => Tile(-1),
            (Down, Some(3)) => Tile(1),
            (Down, Some(_)) => Restore,
            // a window left in a state this does not know: back to a plain one
            (_, Some(_)) => Restore,
        }
    }

    /// The monitor next to the one with the point `p` in the direction `dir` (-1 left, 1 right): its centre.
    fn monitor_beside(&self, p: [f32; 2], dir: i8) -> Option<[f32; 2]> {
        let here = self.mons.iter().find(|m| p[0] >= m[0] && p[0] < m[0] + m[2] && p[1] >= m[1] && p[1] < m[1] + m[3])?;
        self.mons
            .iter()
            .filter(|m| if dir < 0 { m[0] + m[2] <= here[0] + 1.0 } else { m[0] >= here[0] + here[2] - 1.0 })
            .min_by(|a, b| {
                let da = if dir < 0 { here[0] - (a[0] + a[2]) } else { a[0] - (here[0] + here[2]) };
                let db = if dir < 0 { here[0] - (b[0] + b[2]) } else { b[0] - (here[0] + here[2]) };
                da.total_cmp(&db)
            })
            .map(|m| [m[0] + m[2] / 2.0, m[1] + m[3] / 2.0])
    }

    /// An arrow key with Super: snap the window in front, or take it back from where an earlier press put it.
    pub fn snap_step(&mut self, key: SnapKey) {
        use SnapOp::*;
        // a shell window in front (Files, Settings) answers to the keys too; it has no monitors to go to
        if let Some(k) = self.focused_shell() {
            let cur = self.shell_win(k).restore.map(|_| self.shell_win(k).tile);
            match Self::snap_op(cur, key) {
                Tile(side) => {
                    // from one snap to another the way back stays the original place
                    self.tile_shell_window(k, side)
                }
                Maximize => {
                    if self.shell_win(k).restore.is_none() {
                        self.toggle_shell_maximize(k);
                    } else {
                        let keep = self.shell_win(k).restore;
                        self.toggle_shell_maximize(k);
                        self.toggle_shell_maximize(k);
                        self.shell_win_mut(k).restore = keep;
                    }
                }
                Restore => {
                    if self.shell_win(k).restore.is_some() {
                        self.toggle_shell_maximize(k);
                    }
                }
                Minimize => self.shell_win_mut(k).minimized = true,
                Beside(_) => {}
            }
            return;
        }
        let Some(i) = self.wins.iter().rposition(|w| !w.minimized && !w.ws_hidden) else { return };
        if self.wins[i].role.fixed_size() {
            return;
        }
        let cur = self.wins[i].restore.map(|_| self.wins[i].tile);
        let f = Self::frame_rect(&self.wins[i]);
        let centre = [f[0] + f[2] / 2.0, f[1] + f[3] / 2.0];
        match Self::snap_op(cur, key) {
            Tile(side) => self.tile_window(i, side),
            Maximize => {
                if self.wins[i].restore.is_none() {
                    self.toggle_maximize(i);
                } else {
                    // from a snap to maximized: the way back is still where the window was before it was snapped
                    let keep = self.wins[i].restore;
                    self.toggle_maximize(i);
                    self.toggle_maximize(i);
                    self.wins[i].restore = keep;
                }
            }
            Restore => {
                if self.wins[i].restore.is_some() {
                    self.toggle_maximize(i);
                }
            }
            Minimize => self.minimize(i),
            Beside(dir) => {
                // the same place on the next monitor, mirrored (the half that touched the edge comes in at the other one)
                if let Some(p) = self.monitor_beside(centre, dir) {
                    let side = match self.wins[i].tile {
                        -1 => 1,
                        1 => -1,
                        2 => 3,
                        3 => 2,
                        4 => 5,
                        5 => 4,
                        other => other,
                    };
                    self.tile_window_on(i, side, p);
                }
            }
        }
    }

    /// Take a window over the whole monitor (bars and decoration away), or give it back its place.
    pub fn set_fullscreen(&mut self, idx: usize, on: bool) {
        // a window of one fixed size is not stretched to the screen by us (the program itself may still ask: `asked`)
        if on && self.wins[idx].role.fixed_size() {
            return;
        }
        self.fullscreen_now(idx, on);
    }

    /// Fullscreen on or off without asking whether the window can be resized: the program asked for it (a game).
    pub fn fullscreen_now(&mut self, idx: usize, on: bool) {
        if on == self.wins[idx].fs.is_some() {
            return;
        }
        let f = Self::frame_rect(&self.wins[idx]);
        let area = self.mons.iter().find(|m| { let c = [f[0] + f[2] / 2.0, f[1] + f[3] / 2.0]; c[0] >= m[0] && c[0] < m[0] + m[2] && c[1] >= m[1] && c[1] < m[1] + m[3] }).copied().unwrap_or([0.0, 0.0, self.size[0], self.size[1]]);
        let work = self.maximize_area([f[0] + f[2] / 2.0, f[1] + f[3] / 2.0]);
        let hide = self.theme.window.hide_title_max;
        let theme_ins = decor::insets(&self.theme);
        let w = &mut self.wins[idx];
        if on {
            w.fs = Some((f, w.restore));
            w.ins = [0.0; 4];
            w.x.target = area[0];
            w.y.target = area[1];
            w.role.set_fullscreen(true);
            w.role.configure(Some((area[2] as i32, area[3] as i32)), (area[0] as i32, area[1] as i32), Some(false), None);
        } else if let Some((prev, was_max)) = w.fs.take() {
            w.role.set_fullscreen(false);
            if w.deco == Deco::Server {
                w.ins = if hide && was_max.is_some() { [0.0; 4] } else { theme_ins };
            }
            let (ix, iy) = (w.ins[0] + w.ins[2], w.ins[1] + w.ins[3]);
            w.restore = was_max;
            let r = if was_max.is_some() { work } else { prev };
            w.x.target = r[0];
            w.y.target = r[1];
            w.role.configure(Some((((r[2] - ix) as i32).max(1), ((r[3] - iy) as i32).max(1))), ((r[0] + w.ins[0]) as i32, (r[1] + w.ins[1]) as i32), Some(was_max.is_some()), None);
        }
    }

    /// Is a fullscreen window the one in front? Then the bars stay out of its way.
    pub fn fullscreen_on_top(&self) -> bool {
        self.focused_shell().is_none() && self.wins.iter().rev().find(|w| w.mapped && !w.minimized && !w.or).is_some_and(|w| w.fs.is_some())
    }

    pub fn minimize(&mut self, idx: usize) {
        self.wins[idx].ws_hidden = false;
        self.wins[idx].minimized = true;
        self.wins[idx].min_k.target = 1.0;
        self.apply_focus();
    }

    /// Show the windows of desktop `n` and put the others away (they slide into the Dock like minimized ones).
    pub fn switch_workspace(&mut self, n: usize) {
        let n = n.min(WORKSPACES - 1);
        let changed = n != self.workspace;
        self.workspace = n;
        for w in self.wins.iter_mut().filter(|w| !w.or) {
            if w.ws != n {
                // put away: hit-testing and focus skip it, the slide shows it going
                if !w.minimized {
                    w.minimized = true;
                    w.ws_hidden = true;
                }
            } else if w.ws_hidden {
                w.minimized = false;
                w.ws_hidden = false;
            }
        }
        self.ws_pos.target = n as f32;
        self.apply_focus();
        if changed {
            self.shot_notice = Some((format!("Desktop {}", n + 1), 1.2));
        }
    }

    /// For the log: how many device pixels each program's buffer has per logical pixel. A program that does not draw
    /// for the desktop's scale (ratio below it) is stretched by us and looks soft.
    pub fn scale_report(&self) -> String {
        let want = self.renderer.scale();
        let mut out = format!("scales (desktop {want:.3}):");
        for w in self.wins.iter().filter(|w| w.mapped && !w.or) {
            let Some(root) = w.role.surface() else { continue };
            let Some(n) = self.surfaces.get(&root.id()) else { continue };
            let (Some(t), Some(l)) = (n.tex.as_ref(), n.logical_size()) else { continue };
            let ratio = t.size.0 as f32 / l[0].max(1.0);
            let flag = if (ratio - want).abs() > 0.02 { " SOFT" } else { "" };
            out.push_str(&format!(" [{} {}x{} px / {:.1}x{:.1} = {:.3}{flag}]", w.app_id, t.size.0, t.size.1, l[0], l[1], ratio));
        }
        out
    }

    pub fn step_workspace(&mut self, d: i32) {
        let n = self.workspace as i32 + d;
        if n < 0 || n >= WORKSPACES as i32 {
            // no desktop there: the row gives a little and comes back
            self.ws_pos.kick(-d as f32 * 2.2);
            return;
        }
        self.switch_workspace(n as usize);
    }

    /// Three fingers went down on the touchpad: the desktops follow them from now on.
    pub fn ws_swipe_begin(&mut self) {
        self.ws_drag = Some((self.ws_pos.x, 0.0, 0.0));
        self.ws_pos.v = 0.0;
        self.ws_pos.target = self.ws_pos.x;
    }

    /// The fingers moved by `dx` (touchpad units) in `dt` seconds.
    pub fn ws_swipe_move(&mut self, dx: f32, dt: f32) {
        let Some((start, moved, vel)) = self.ws_drag else { return };
        // a whole desktop takes about half a screen of finger travel; the content goes the way the fingers go
        let step = -dx / (self.size[0].max(1.0) * 0.5);
        let moved = moved + step;
        let vel = vel * 0.6 + (step / dt.max(0.001)) * 0.4;
        let want = start + moved;
        let last = (WORKSPACES - 1) as f32;
        // past the first or the last desktop it pulls back
        let shown = if want < 0.0 { want * 0.3 } else if want > last { last + (want - last) * 0.3 } else { want };
        self.ws_drag = Some((start, moved, vel));
        self.ws_pos.x = shown;
        self.ws_pos.target = shown;
        self.ws_pos.v = 0.0;
    }

    /// The fingers came up: the nearest desktop (a flick counts) takes over.
    pub fn ws_swipe_end(&mut self, cancelled: bool) {
        let Some((start, _, vel)) = self.ws_drag.take() else { return };
        let here = self.ws_pos.x;
        let mut n = if cancelled { start.round() } else { (here + vel * 0.18).round() };
        if !cancelled && n == self.workspace as f32 && (here - self.workspace as f32).abs() > 0.12 {
            // a short but deliberate pull
            n += (here - self.workspace as f32).signum();
        }
        let n = n.clamp(0.0, (WORKSPACES - 1) as f32) as usize;
        let carry = vel.clamp(-6.0, 6.0) * 0.5;
        self.switch_workspace(n);
        self.ws_pos.v = carry;
    }

    /// Send the front window to the next / previous desktop (it leaves the screen, as the desktop is not shown).
    pub fn move_window_workspace(&mut self, d: i32) {
        let Some(i) = self.wins.iter().rposition(|w| !w.minimized && !w.or && w.mapped) else { return };
        let n = (self.wins[i].ws as i32 + d).rem_euclid(WORKSPACES as i32) as usize;
        self.wins[i].ws = n;
        let cur = self.workspace;
        self.switch_workspace(cur);
        self.shot_notice = Some((format!("Moved to desktop {}", n + 1), 1.2));
    }

    /// Start a program from a shortcut.
    pub fn run_command(&mut self, cmd: &str) {
        let app = AppEntry { id: String::new(), name: cmd.to_string(), generic_name: String::new(), exec: cmd.to_string(), icon: String::new(), keywords: String::new(), terminal: false, wm_class: String::new(), categories: Vec::new() };
        if let Err(e) = apps::launch(&app, &self.socket) {
            eprintln!("could not run {cmd}: {e}");
        }
    }

    pub fn close_window(&mut self, idx: usize) {
        self.wins[idx].role.close();
    }

    fn start_resize(&mut self, idx: usize, edges: u8) {
        let w = &self.wins[idx];
        if w.restore.is_some() || edges == 0 {
            return;
        }
        let fr = Self::frame_rect(w);
        let Some(surf) = w.role.surface() else { return };
        self.grab = Some(Grab::Resize { surf, edges, start_mouse: self.mouse, start: fr });
    }

    /// A program may start a move or resize only while a button is held on its own surface; a late request from a window the
    /// pointer has left (or after the release) would drag the wrong window.
    pub fn pointer_holds(&self, root: &WlSurface) -> bool {
        self.pointer.is_grabbed() && self.pointer.current_focus().is_some_and(|f| root_of(&f) == *root)
    }

    pub fn begin_client_move(&mut self, root: &WlSurface) {
        if let Some(i) = self.win_index(root) {
            let (x, y) = (self.wins[i].x.x, self.wins[i].y.x);
            self.grab = Some(Grab::Move { surf: root.clone(), off: [self.mouse[0] - x, self.mouse[1] - y], start: self.mouse });
            self.wins[i].scale.target = 1.018;
            self.focus_window(i);
        }
    }

    pub fn begin_client_resize(&mut self, root: &WlSurface, edges: xdg_toplevel::ResizeEdge) {
        let bits = match edges {
            xdg_toplevel::ResizeEdge::Left => 1,
            xdg_toplevel::ResizeEdge::Right => 2,
            xdg_toplevel::ResizeEdge::Top => 4,
            xdg_toplevel::ResizeEdge::Bottom => 8,
            xdg_toplevel::ResizeEdge::TopLeft => 5,
            xdg_toplevel::ResizeEdge::TopRight => 6,
            xdg_toplevel::ResizeEdge::BottomLeft => 9,
            xdg_toplevel::ResizeEdge::BottomRight => 10,
            _ => 0,
        };
        if let Some(i) = self.win_index(root) {
            if self.wins[i].role.fixed_size() {
                return;
            }
            let i = self.focus_window(i);
            self.start_resize(i, bits);
        }
    }

    fn apply_resize(&mut self, surf: &WlSurface, edges: u8, start_mouse: [f32; 2], start: [f32; 4]) {
        let Some(i) = self.win_index(surf) else { return };
        let (ix, iy) = (self.wins[i].ins[0] + self.wins[i].ins[2], self.wins[i].ins[1] + self.wins[i].ins[3]);
        let (dx, dy) = (self.mouse[0] - start_mouse[0], self.mouse[1] - start_mouse[1]);
        let mut nw = start[2];
        let mut nh = start[3];
        if edges & 2 != 0 {
            nw = start[2] + dx;
        }
        if edges & 1 != 0 {
            nw = start[2] - dx;
        }
        if edges & 8 != 0 {
            nh = start[3] + dy;
        }
        if edges & 4 != 0 {
            nh = start[3] - dy;
        }
        // a window cannot be pulled out over the menu bar or off the screen
        {
            let area = self.work_area();
            if edges & 4 != 0 {
                let bottom = start[1] + start[3];
                nh = bottom - (bottom - nh).max(area[1]);
            }
            if edges & 1 != 0 {
                let right = start[0] + start[2];
                nw = right - (right - nw).max(area[0]);
            }
        }
        nw = nw.max(MIN_W + ix);
        nh = nh.max(MIN_H + iy);
        let w = &mut self.wins[i];
        // dragging the left/top edge keeps the opposite edge in place
        if edges & 1 != 0 {
            w.x.target = start[0] + (start[2] - nw);
            w.x.x = w.x.target;
        }
        if edges & 4 != 0 {
            w.y.target = start[1] + (start[3] - nh);
            w.y.x = w.y.target;
        }
        w.role.configure(Some((((nw - ix) as i32).max(1), ((nh - iy) as i32).max(1))), ((w.x.target + w.ins[0]) as i32, (w.y.target + w.ins[1]) as i32), None, Some(true));
    }

    /// Did the drag that just ended ask for the window to be snapped: the pointer was at the top edge (the whole work area) or
    /// the left / right edge (that half), or the window was flicked upwards fast? Gives the side (see `snap_side`).
    fn wants_snap(&self) -> Option<i8> {
        if self.snap_on {
            return Some(self.snap_side);
        }
        let (Some(first), Some(last)) = (self.move_samples.iter().find(|(t, _)| t.elapsed().as_millis() < 160), self.move_samples.last()) else { return None };
        let dt = last.0.duration_since(first.0).as_secs_f32();
        let up = first.1[1] - last.1[1];
        let travelled = self.move_start.map_or(0.0, |f| f[1] - self.mouse_start_y());
        (dt > 0.01 && up / dt > 1100.0 && up > 60.0 && travelled.abs() >= 0.0).then_some(0)
    }

    fn mouse_start_y(&self) -> f32 {
        self.move_samples.first().map_or(self.mouse[1], |s| s.1[1])
    }

    fn end_grab(&mut self) {
        let snap = if matches!(self.grab, Some(Grab::Move { .. }) | Some(Grab::ShellMove { .. })) { self.wants_snap() } else { None };
        let start = self.move_start.take();
        self.move_samples.clear();
        self.snap_on = false;
        match self.grab.take() {
            Some(Grab::ShellMove { win, .. }) => {
                if let (Some(side), true) = (snap, self.shell_win(win).restore.is_none()) {
                    if side == 0 { self.toggle_shell_maximize(win) } else { self.tile_shell_window(win, side) }
                    // coming back puts the window where it was picked up
                    if let (Some(f), Some(_)) = (start, self.shell_win(win).restore) {
                        self.shell_win_mut(win).restore = Some([f[0], f[1]]);
                    }
                }
            }
            Some(Grab::ShellResize { .. }) => {}
            Some(Grab::Move { surf, .. }) => {
                if let Some(i) = self.win_index(&surf) {
                    self.wins[i].scale.target = 1.0;
                    if let (Some(side), true) = (snap, self.wins[i].restore.is_none()) {
                        if side == 0 { self.toggle_maximize(i) } else { self.tile_window(i, side) }
                        if let (Some(f), true) = (start, self.wins[i].restore.is_some()) {
                            self.wins[i].restore = Some(f);
                        }
                    }
                }
            }
            Some(Grab::Resize { surf, .. }) => {
                if let Some(i) = self.win_index(&surf) {
                    let w = &self.wins[i];
                    w.role.configure(None, ((w.x.target + w.ins[0]) as i32, (w.y.target + w.ins[1]) as i32), None, Some(false));
                }
            }
            None => {}
        }
    }

    // ------------------------------------------------------------ hit testing

    fn edges_at(&self, w: &Win, p: [f32; 2]) -> u8 {
        if w.restore.is_some() {
            return 0;
        }
        Self::edges_of(Self::frame_rect(w), p)
    }

    /// Which edges of the frame `f` (x, y, w, h) the point `p` is on (1 left, 2 right, 4 top, 8 bottom), or 0.
    pub(crate) fn edges_of(f: [f32; 4], p: [f32; 2]) -> u8 {
        let (l, r, t, b) = (f[0], f[0] + f[2], f[1], f[1] + f[3]);
        if p[0] < l - EDGE || p[0] > r + EDGE || p[1] < t - EDGE || p[1] > b + EDGE {
            return 0;
        }
        let mut e = 0;
        if p[0] < l + EDGE {
            e |= 1;
        }
        if p[0] > r - EDGE {
            e |= 2;
        }
        if p[1] < t + EDGE {
            e |= 4;
        }
        if p[1] > b - EDGE {
            e |= 8;
        }
        e
    }

    /// Topmost surface under `p` (popups first, then windows): the surface and its world origin.
    pub fn surface_under(&self, p: [f32; 2]) -> Option<(WlSurface, [f32; 2])> {
        let mut roots: Vec<WlSurface> = self.popups.iter().rev().map(|pp| pp.surface.wl_surface().clone()).collect();
        for w in self.wins.iter().rev() {
            if !w.minimized && w.mapped {
                roots.extend(w.role.surface());
            }
        }
        for root in roots {
            let Some(ro) = self.root_origin(&root) else { continue };
            for (s, off) in surface_tree(&root).into_iter().rev() {
                let Some(sz) = self.surf_size(&s) else { continue };
                let o = [ro[0] + off[0] as f32, ro[1] + off[1] as f32];
                if p[0] >= o[0] && p[0] < o[0] + sz[0] && p[1] >= o[1] && p[1] < o[1] + sz[1] {
                    // respect the window shape: nothing outside the geometry rect belongs to the window
                    return Some((s, o));
                }
            }
        }
        None
    }

    /// Chrome hit-test for the topmost window under `p`.
    pub(crate) fn chrome_at(&self, p: [f32; 2]) -> Option<(usize, Hit)> {
        for (i, w) in self.wins.iter().enumerate().rev() {
            if w.minimized || !w.mapped {
                continue;
            }
            // only the active window can be resized from its edges (the front one, unless a shell window is in front)
            let active = self.focused_shell().is_none() && self.wins.iter().rposition(|o| !o.minimized && o.mapped && !o.or) == Some(i);
            let e = if w.fs.is_some() || !active || w.role.fixed_size() { 0 } else { self.edges_at(w, p) };
            let f = Self::frame_rect(w);
            let inside = p[0] >= f[0] && p[0] <= f[0] + f[2] && p[1] >= f[1] && p[1] <= f[1] + f[3];
            let in_title = w.deco == Deco::Server && w.ins[1] > 0.0 && p[1] < f[1] + w.ins[1];
            if e != 0 && !(inside && in_title && e & 4 == 0) {
                return Some((i, Hit::Edge(e)));
            }
            if !inside {
                continue;
            }
            if in_title {
                for (b, r) in decor::buttons(&self.theme, f) {
                    if p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3] {
                        return Some((i, Hit::Button(b)));
                    }
                }
                return Some((i, Hit::Titlebar));
            }
            return Some((i, Hit::Content));
        }
        None
    }

    // ------------------------------------------------------------ input from the backend

    pub fn pointer_motion(&mut self, x: f32, y: f32) {
        if (x - self.mouse[0]).abs() + (y - self.mouse[1]).abs() > 0.0 {
            self.activity();
        }
        self.mouse = [x, y];
        if self.lock.is_some() {
            return;
        }
        if self.shot_sel.is_some() {
            return;
        }
        if self.shot_edit.is_some() {
            self.shot_edit_motion();
            return;
        }
        self.panel_edit_motion();
        self.flyout_motion(x);
        self.card_motion([x, y]);
        self.desk_widget_motion([x, y]);
        self.scene.set_mouse(x, y);
        self.launcher.set_mouse([x, y]);
        self.settings.set_mouse([x, y]);
        self.files.set_mouse([x, y]);
        self.desktop.set_mouse([x, y]);
        self.menu.set_mouse([x, y]);
        let mut dragging_files = false;
        if let Some(d) = &mut self.file_drag {
            if !d.active && (d.start[0] - x).hypot(d.start[1] - y) > 6.0 {
                d.active = true;
            }
            dragging_files = d.active;
        }
        let target = if dragging_files { self.drop_target_at([x, y]) } else { None };
        // icons of the desktop carried over its empty part: show the cells they would take
        self.desktop.drag = match &self.file_drag {
            Some(d) if dragging_files && !self.key_mods().ctrl && self.shell_covers([x, y]).is_none() && target.as_deref() == Some(self.desktop.dir()) && self.desktop.owns(&d.paths) => Some((d.paths.clone(), d.start, [x, y])),
            _ => None,
        };
        self.files.drop_hover = target.clone();
        self.desktop.drop_hover = target;
        let hover = self.bar_hit([x, y]);
        self.bar_hover = hover;
        self.bar_hover_mon = self.bar_mon.get();
        if let (Some(i), true, MenuFor::Bar(cur)) = (hover, self.menu.open, self.menu_for) {
            if i != cur && i.bar == cur.bar {
                self.bar_click(i, true);
            }
        }
        self.title_menu_hover([x, y]);
        let covering = self.shell_covers([x, y]);
        for k in [Sw::Settings, Sw::Files] {
            let hover = if covering == Some(k) { decor::buttons(&self.theme, self.sw_frame(k)).into_iter().find(|(_, r)| Self::in_rect([x, y], *r)).map(|(b, _)| b) } else { None };
            self.shell_win_mut(k).hover = hover;
        }
        self.hover_btn = match self.chrome_at([x, y]) {
            Some((i, Hit::Button(b))) if !self.popup_hit([x, y]) => Some((i, b)),
            _ => None,
        };
        // dragging a window by its title: remember where it came from and whether the pointer is at the top edge (snap)
        let moving = match &self.grab {
            Some(Grab::Move { surf, .. }) => self.win_index(surf).map(|i| (Self::frame_rect(&self.wins[i]), self.wins[i].restore.is_none())),
            Some(Grab::ShellMove { win, .. }) => Some((self.sw_frame(*win), self.shell_win(*win).restore.is_none())),
            _ => None,
        };
        if let Some((frame, can_snap)) = moving {
            if self.move_start.is_none() {
                self.move_start = Some(frame);
            }
            self.move_samples.push((Instant::now(), [x, y]));
            if self.move_samples.len() > 12 {
                self.move_samples.remove(0);
            }
            let area = self.work_area_at([x, y]);
            // the top edge takes the whole work area, the left and right edges that half of it (not an edge that has another
            // monitor beyond it: the pointer is only on its way there)
            let beyond = |px: f32| self.mons.iter().any(|m| px >= m[0] && px < m[0] + m[2] && y >= m[1] && y < m[1] + m[3]);
            let (top, bottom) = (y <= area[1] + 6.0, y >= area[1] + area[3] - 4.0);
            let left = x <= area[0] + 3.0 && !beyond(area[0] - 2.0);
            let right = x >= area[0] + area[2] - 4.0 && !beyond(area[0] + area[2] + 2.0);
            // an edge near a corner snaps to that quarter
            let near_top = y <= area[1] + 56.0;
            let near_bottom = y >= area[1] + area[3] - 56.0;
            let side = if left && near_top { Some(2) } else if right && near_top { Some(3) } else if left && near_bottom { Some(4) } else if right && near_bottom { Some(5) }
                else if top { Some(0) } else if left { Some(-1) } else if right { Some(1) } else if bottom && false { None } else { None };
            self.snap_on = can_snap && side.is_some();
            if let (true, Some(s)) = (can_snap, side) {
                self.snap_side = s;
            }
        }
        match &self.grab {
            Some(Grab::ShellMove { win, off, start }) => {
                let (win, mut off, start) = (*win, *off, *start);
                if self.shell_win(win).restore.is_some() && (x - start[0]).hypot(y - start[1]) > 6.0 {
                    // dragging a maximized window's title: it restores first and the pointer keeps its place on the bar
                    let before = self.sw_frame(win);
                    self.toggle_shell_maximize(win);
                    let after = self.sw_frame(win);
                    off = [((start[0] - before[0]) / before[2].max(1.0) * after[2]).clamp(20.0, after[2] - 20.0), off[1]];
                    self.grab = Some(Grab::ShellMove { win, off, start: [f32::MAX; 2] });
                    let w = self.shell_win_mut(win);
                    w.x.x = x - off[0];
                    w.y.x = y - off[1];
                }
                let w = self.shell_win_mut(win);
                w.x.target = x - off[0];
                w.y.target = y - off[1];
            }
            Some(Grab::ShellResize { win, edges, start_mouse, start }) => {
                let (win, edges, sm, st) = (*win, *edges, *start_mouse, *start);
                self.apply_shell_resize(win, edges, sm, st);
            }
            Some(Grab::Move { surf, off, start }) => {
                let (surf, mut off, start) = (surf.clone(), *off, *start);
                if let Some(i) = self.win_index(&surf).filter(|&i| self.wins[i].restore.is_some() && (x - start[0]).hypot(y - start[1]) > 6.0) {
                    // dragging a maximized window's title: it restores first and the pointer keeps its place on the bar
                    let before = Self::frame_rect(&self.wins[i]);
                    let r = self.wins[i].restore.unwrap_or(before);
                    self.toggle_maximize(i);
                    off = [((start[0] - before[0]) / before[2].max(1.0) * r[2]).clamp(20.0, (r[2] - 20.0).max(20.0)), off[1]];
                    self.grab = Some(Grab::Move { surf: surf.clone(), off, start: [f32::MAX; 2] });
                    let w = &mut self.wins[i];
                    w.x.x = x - off[0];
                    w.y.x = y - off[1];
                }
                if let Some(i) = self.win_index(&surf) {
                    let fw = Self::frame_rect(&self.wins[i])[2].max(1.0);
                    self.wins[i].jgrab = off[0] / fw;
                    self.wins[i].x.target = x - off[0];
                    self.wins[i].y.target = y - off[1];
                }
            }
            Some(Grab::Resize { surf, edges, start_mouse, start }) => {
                let (surf, edges, sm, st) = (surf.clone(), *edges, *start_mouse, *start);
                self.apply_resize(&surf, edges, sm, st);
            }
            None => {}
        }
        // files carried out of the shell over a program's window: the program is offered them as a drag
        let offering = dragging_files && self.pointer.is_grabbed() && !self.file_dnd.is_empty();
        let over_client = !(self.launcher_modal() || self.shell_hit() || self.grab.is_some() || self.menu.open || self.shell_covers([x, y]).is_some());
        if dragging_files && !offering && over_client && !self.key_mods().ctrl && !self.pointer.is_grabbed() && self.surface_under([x, y]).is_some() {
            if let Some(d) = &self.file_drag {
                self.file_dnd = d.paths.clone();
                let serial = SERIAL_COUNTER.next_serial();
                let start = smithay::input::pointer::GrabStartData { focus: None, button: BTN_LEFT, location: (x as f64, y as f64).into() };
                let meta = smithay::wayland::selection::data_device::SourceMetadata { mime_types: vec!["text/uri-list".into()], dnd_action: smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction::Copy };
                let seat = self.seat.clone();
                let dh = self.dh.clone();
                smithay::wayland::selection::data_device::start_dnd(&dh, &seat, self, serial, Some(start), None, meta);
            }
        }
        let offering = dragging_files && self.pointer.is_grabbed() && !self.file_dnd.is_empty();
        let focus = if self.launcher_modal() || self.shell_hit() || self.grab.is_some() || self.menu.open || (dragging_files && !offering) || self.shell_covers([x, y]).is_some() { None } else { self.surface_under([x, y]) };
        let focus = focus.filter(|_| !matches!(self.chrome_at([x, y]), Some((_, Hit::Edge(_)))) || self.popup_hit([x, y]));
        let fid = focus.as_ref().map(|(s, _)| s.id());
        if fid != self.cursor_focus {
            self.cursor_focus = fid;
            self.client_cursor = memrium_de::cursor::Shape::Default;
            // the cursor surface of the window the pointer left must not turn the new one's cursor into its picture
            self.client_cursor_surface = None;
        }
        self.update_cursor_shape();
        let ptr = self.pointer.clone();
        let time = self.time_ms();
        // an X11 window counts in device pixels: the position inside it is told in those. The origin handed over stays the true
        // one and the position is stretched round it, so that `position - origin` is `(pointer - origin)` times the scale. (Moving
        // the origin instead breaks a drag: while a button is held the origin from the press is kept, and the drag then moved the
        // program's pointer by logical pixels, a fifth too slowly.)
        let xs = crate::role::x_scale() as f64;
        // without a grab the surface under the pointer is the one that is told; with one it keeps the origin it had at the press
        let grabbed = ptr.is_grabbed();
        if !grabbed {
            self.x_focus_origin = focus.as_ref().filter(|(s, _)| xs != 1.0 && self.surfaces.get(&s.id()).is_some_and(|n| n.x11)).map(|(s, o)| (s.clone(), [o[0] as f64, o[1] as f64]));
        }
        let focus = focus.map(|(s, o)| (s, Point::<f64, Logical>::from((o[0] as f64, o[1] as f64))));
        let mut location = (x as f64, y as f64);
        // the surface that gets this motion: the one under the pointer, or the one that holds a grab
        let target = if grabbed { ptr.current_focus() } else { focus.as_ref().map(|(s, _)| s.clone()) };
        if let (Some(t), Some((fs, o))) = (&target, &self.x_focus_origin) {
            if t == fs {
                location = (o[0] + (x as f64 - o[0]) * xs, o[1] + (y as f64 - o[1]) * xs);
            }
        }
        ptr.motion(
            self,
            focus,
            &MotionEvent { location: location.into(), serial: SERIAL_COUNTER.next_serial(), time },
        );
        ptr.frame(self);
        self.update_pointer_constraints();
    }

    fn popup_hit(&self, p: [f32; 2]) -> bool {
        self.surface_under(p).is_some_and(|(s, _)| self.popup_index(&root_of(&s)).is_some())
    }

    pub fn pointer_button(&mut self, button: u32, pressed: bool) {
        self.activity();
        if std::env::var_os("MEMRIUM_DEBUG_CLICK").is_some() {
            eprintln!("click {button} {pressed} at {:?}: bar_hit {:?} menu {} launcher {} card {} shell_hit {}", self.mouse, self.bar_hit(self.mouse).map(|h| (h.bar, h.widget)), self.menu.open, self.launcher_modal(), self.card_hit(self.mouse), self.shell_hit());
        }
        if self.lock.is_some() {
            return;
        }
        // a click on a notification toast is the toast's alone
        if pressed && self.toast_click(self.mouse, button) {
            return;
        }
        if pressed && button == BTN_LEFT && self.widgets_press(self.mouse) {
            return;
        }
        if pressed && button == BTN_LEFT && self.card_press(self.mouse) {
            return;
        }
        if !pressed && button == BTN_LEFT {
            self.card_release();
        }
        if button == BTN_LEFT && self.flyout.is_some() {
            if !pressed {
                self.flyout_release();
            } else if self.flyout_press(self.mouse) {
                return;
            }
        }
        if self.shot_edit.is_some() {
            self.shot_edit_button(button, pressed);
            return;
        }
        if let Some(sel) = &mut self.shot_sel {
            if button == BTN_LEFT {
                if pressed {
                    sel.start = Some(self.mouse);
                } else if let Some(a) = sel.start {
                    let b = self.mouse;
                    let r = [a[0].min(b[0]), a[1].min(b[1]), (a[0] - b[0]).abs(), (a[1] - b[1]).abs()];
                    let record = sel.record;
                    self.shot_sel = None;
                    if r[2] >= 4.0 && r[3] >= 4.0 {
                        self.shot_req = Some(if record { ShotReq::Record(r) } else { ShotReq::Edit(r) });
                    }
                }
            } else if pressed {
                self.shot_sel = None;
            }
            return;
        }
        let time = self.time_ms();
        if !pressed && button == BTN_LEFT && self.launcher_modal() {
            match self.launcher.release(self.mouse, &self.apps, self.shell_covers(self.mouse) == Some(Sw::Files)) {
                LauncherClick::App(i) => {
                    let app = self.apps[i].clone();
                    self.launch_app(&app);
                    self.launcher.close();
                }
                LauncherClick::Shortcut(i) => {
                    let app = self.apps[i].clone();
                    // a file manager window under the pointer takes it (in the folder under the pointer, else the one it
                    // shows); anywhere else it goes onto the desktop
                    let in_files = self.shell_covers(self.mouse) == Some(Sw::Files);
                    let dir = if in_files { self.files.drop_target(self.mouse).filter(|d| d.is_dir() && !d.starts_with(fsops::trash_path())) } else { Some(self.desktop.dir().to_path_buf()) };
                    match dir.map(|d| memrium_de::apps::write_shortcut(&app, &d)) {
                        Some(Ok(path)) => {
                            // the views read the folder again; on the desktop it starts out selected
                            self.fm.gen += 1;
                            if !in_files {
                                self.desktop.reload();
                                self.desktop.select_only(path);
                            }
                            self.launcher.close();
                        }
                        Some(Err(e)) => eprintln!("no launcher for {}: {e}", app.name),
                        None => eprintln!("no launcher for {}: that window shows no folder to put it in", app.name),
                    }
                }
                LauncherClick::Outside => self.launcher.close(),
                _ => {}
            }
            return;
        }
        if !pressed && button == BTN_LEFT && self.panel_edit_release() {
            return;
        }
        if pressed {
            if self.menu.open {
                match self.menu.click(self.mouse) {
                    MenuClick::Item(id) => {
                        self.menu.close();
                        self.menu_command(id);
                    }
                    MenuClick::Inside => {}
                    MenuClick::Outside => {
                        self.menu.close();
                        // clicking the same menu bar item again just closes it
                    }
                }
                return;
            }
            // Launchpad covers the whole screen: nothing under it (bars, Dock, windows) gets the click; the pointer going
            // down only picks something up, the release decides (a click, or a drop)
            if self.launcher_modal() {
                if button == BTN_LEFT {
                    self.launcher.press(self.mouse);
                }
                return;
            }
            if self.preview_active() && (button == BTN_LEFT || button == BTN_MIDDLE) {
                let used = if button == BTN_LEFT { self.preview_click() } else { self.preview_middle_click() };
                if used {
                    return;
                }
            }
            if button == BTN_RIGHT && self.panel_edit.is_none() {
                if let Some(h) = self.dock_item_at() {
                    self.open_dock_menu(&h);
                    return;
                }
            }
            if button == BTN_MIDDLE && self.panel_edit.is_none() {
                // a middle click on an item starts another window of its app
                if let Some(h) = self.dock_item_at() {
                    if let Some(app) = self.apps.iter().find(|a| a.id == h.key).cloned() {
                        self.launch_app(&app);
                        return;
                    }
                }
            }
            if (button == BTN_RIGHT || button == BTN_MIDDLE) && self.panel_edit.is_none() && self.tray_alt_click(button == BTN_RIGHT) {
                return;
            }
            if (button == BTN_RIGHT || button == BTN_MIDDLE) && self.panel_edit.is_none() && self.bar_alt_action(if button == BTN_RIGHT { "on_right" } else { "on_middle" }) {
                return;
            }
            if button == BTN_RIGHT {
                if let Some(b) = self.bar_at(self.mouse) {
                    self.open_panel_menu(b);
                    return;
                }
            }
            if button == BTN_LEFT && self.panel_edit.is_some() && self.panel_edit_press() {
                return;
            }
            if button == BTN_LEFT {
                if let Some(i) = self.bar_hit(self.mouse) {
                    self.bar_click(i, false);
                    return;
                }
                // a maximized window without a title bar is dragged and zoomed by the empty part of the bar above it
                if self.bar_title_drag() {
                    return;
                }
            }
            self.desktop.focused = false;
            if button == BTN_LEFT {
                self.scene.mouse_down();
            }
            if self.shell_hit() {
                return;
            }
            if self.launcher.visible() && self.launcher.open {
                match self.launcher.click(self.mouse) {
                    LauncherClick::App(i) => {
                        let app = self.apps[i].clone();
                        self.launch_app(&app);
                        self.launcher.close();
                    }
                    LauncherClick::Item(i) => {
                        if let Some(it) = self.launcher.item(i) {
                            self.spotlight_run(it.target);
                        }
                    }
                    LauncherClick::Power(r) => {
                        self.launcher.close();
                        self.open_power_menu([r[0], (r[1] - 140.0).max(8.0)]);
                    }
                    LauncherClick::Inside | LauncherClick::Shortcut(_) => {}
                    LauncherClick::Outside => self.launcher.close(),
                }
                return;
            }
            if (button == BTN_LEFT || button == BTN_RIGHT) && self.shell_pointer_down(button == BTN_RIGHT) {
                return;
            }
            if (button == BTN_LEFT || button == BTN_RIGHT) && self.desktop_hit(self.mouse) && self.desk_widget_press(self.mouse, button == BTN_RIGHT) {
                return;
            }
            if (button == BTN_LEFT || button == BTN_RIGHT) && self.desktop_hit(self.mouse) {
                let m = self.key_mods();
                let (consumed, ev) = self.desktop.mouse_down(self.mouse, button == BTN_RIGHT, m, &mut self.fm);
                if matches!(ev.first(), Some(FilesEvent::Menu(..))) {
                    self.menu_for = MenuFor::DesktopIcons;
                }
                self.handle_desktop_events(ev);
                if !consumed && button == BTN_RIGHT {
                    self.open_desktop_menu();
                }
                if consumed || button == BTN_LEFT {
                    return;
                }
                return;
            }
            // a click outside the open popup chain dismisses it
            if !self.popups.is_empty() && !self.popup_hit(self.mouse) {
                for p in self.popups.iter().rev().filter(|p| p.grab) {
                    p.surface.send_popup_done();
                }
                self.drop_popups(|p| p.grab);
            }
            if !self.popup_hit(self.mouse) {
                match self.chrome_at(self.mouse) {
                    Some((i, Hit::Button(Btn::Close))) => {
                        self.close_window(i);
                        return;
                    }
                    Some((i, Hit::Button(Btn::Minimize))) => {
                        self.minimize(i);
                        return;
                    }
                    Some((i, Hit::Button(Btn::Maximize))) => {
                        self.toggle_maximize(i);
                        return;
                    }
                    Some((i, Hit::Titlebar)) if self.title_menu_at(i, self.mouse).is_some() => {
                        // a name of the program's menus in the title bar: the window comes forward and the menu opens
                        let Some((name, rect)) = self.title_menu_at(i, self.mouse) else { return };
                        let last = self.focus_window(i);
                        self.open_title_menu(last, name, rect);
                        return;
                    }
                    Some((i, Hit::Titlebar)) => {
                        // a quick second click on the titlebar maximizes / restores
                        let now = Instant::now();
                        let double = self.last_title_click.is_some_and(|t| now.duration_since(t).as_millis() < 350);
                        self.last_title_click = Some(now);
                        let last = self.focus_window(i);
                        if double {
                            self.toggle_maximize(last);
                            self.last_title_click = None;
                            return;
                        }
                        let (x, y) = (self.wins[last].x.x, self.wins[last].y.x);
                        let Some(surface) = self.wins[last].role.surface() else { return };
                        self.wins[last].scale.target = 1.018;
                        self.grab = Some(Grab::Move { surf: surface, off: [self.mouse[0] - x, self.mouse[1] - y], start: self.mouse });
                        return;
                    }
                    Some((i, Hit::Edge(e))) => {
                        let i = self.focus_window(i);
                        self.start_resize(i, e);
                        return;
                    }
                    Some((i, Hit::Content)) => {
                        // also when it is already the top client window: a shell window (Files, Settings) may be in front of it
                        if Some(i) != self.top_managed() || self.focused_shell().is_some() {
                            self.focus_window(i);
                        }
                    }
                    None => {}
                }
            }
        } else {
            if button == BTN_LEFT {
                if let Some(d) = self.file_drag.take() {
                    self.toast_release(d.active);
                    if d.active {
                        // a program being offered the files takes the release: it is the drop
                        if self.pointer.is_grabbed() && !self.file_dnd.is_empty() {
                            let ptr = self.pointer.clone();
                            let time = self.time_ms();
                            ptr.button(self, &ButtonEvent { button, state: ButtonState::Released, serial: SERIAL_COUNTER.next_serial(), time });
                            ptr.frame(self);
                        }
                        self.finish_drag(d);
                        return;
                    }
                }
                self.desk_widget_release();
                self.files.mouse_up();
                self.desktop.mouse_up();
                self.settings.mouse_up();
                self.scene.mouse_up();
            }
            if self.grab.is_some() {
                self.end_grab();
                return;
            }
        }
        let ptr = self.pointer.clone();
        let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
        ptr.button(self, &ButtonEvent { button, state, serial: SERIAL_COUNTER.next_serial(), time });
        ptr.frame(self);
    }

    pub fn pointer_axis(&mut self, dx: f64, dy: f64) {
        self.activity();
        if self.lock.is_some() {
            return;
        }
        if self.launcher_modal() && !self.menu.open {
            self.launcher.scroll(dy);
            return;
        }
        match self.shell_covers(self.mouse) {
            Some(Sw::Settings) => {
                self.settings.scroll(dy);
                return;
            }
            Some(Sw::Files) => {
                self.files.scroll(dy);
                return;
            }
            None => {}
        }
        if self.menu.open {
            return;
        }
        if self.launcher.scroll(dy) {
            return;
        }
        if self.card_scroll(dy) {
            return;
        }
        if dy != 0.0 && self.panel_edit.is_none() && self.bar_alt_action(if dy < 0.0 { "scroll_up" } else { "scroll_down" }) {
            return;
        }
        let time = self.time_ms();
        let mut frame = AxisFrame::new(time).source(AxisSource::Wheel);
        if dx != 0.0 {
            frame = frame.value(Axis::Horizontal, dx);
        }
        if dy != 0.0 {
            frame = frame.value(Axis::Vertical, dy);
        }
        let ptr = self.pointer.clone();
        ptr.axis(self, frame);
        ptr.frame(self);
    }

    /// `evdev` is the Linux input event code (not the xkb keycode).
    pub fn key(&mut self, evdev: u32, pressed: bool) {
        self.activity();
        let time = self.time_ms();
        let kb = self.keyboard.clone();
        let state = if pressed { KeyState::Pressed } else { KeyState::Released };
        kb.input::<(), _>(self, Keycode::new(evdev + 8), state, SERIAL_COUNTER.next_serial(), time, move |st, mods, handle| {
            // with Ctrl/Alt/Super held a shortcut is the physical key, whatever layout is on (Ctrl+Z is Ctrl+Я in Ukrainian);
            // plain typing keeps the layout's own symbol
            let sym = match handle.raw_latin_sym_or_raw_current_sym() {
                Some(latin) if mods.ctrl || mods.alt || mods.logo => latin,
                _ => handle.modified_sym(),
            };
            if st.handle_key(evdev, pressed, mods, sym) {
                FilterResult::Intercept(())
            } else {
                FilterResult::Forward
            }
        });
    }

    /// Alt+Tab: the next window by most recent use (`back`: the previous one). Holding Alt and pressing Tab again goes further
    /// down the list; letting go of Alt keeps the choice, and the order of the others stays as it was.
    pub fn cycle_windows(&mut self, back: bool) {
        let live = |s: &State| -> Vec<u64> { s.wins.iter().rev().filter(|w| !w.minimized && w.mapped && !w.or).map(|w| w.id).collect() };
        let (order, pos) = match self.alt_tab.take() {
            // a window closed or went away meanwhile: begin again
            Some((ids, pos)) if ids.iter().all(|id| self.wins.iter().any(|w| w.id == *id && !w.minimized)) => (ids, pos),
            _ => (live(self), 0),
        };
        let n = order.len();
        if n < 2 {
            return;
        }
        let pos = (pos + if back { n - 1 } else { 1 }) % n;
        let chosen = order[pos];
        if let Some(i) = self.wins.iter().position(|w| w.id == chosen) {
            self.focus_window(i);
        }
        // the others keep their order below the chosen one (focusing it alone would shuffle them)
        let mut want: Vec<u64> = order.iter().copied().filter(|&id| id != chosen).collect();
        want.insert(0, chosen);
        want.reverse();
        let slots: Vec<usize> = (0..self.wins.len()).filter(|&i| want.contains(&self.wins[i].id)).collect();
        if slots.len() == want.len() {
            let mut pulled: Vec<Win> = slots.iter().rev().map(|&i| self.wins.remove(i)).collect();
            for (k, &slot) in slots.iter().enumerate() {
                if let Some(p) = pulled.iter().position(|w| w.id == want[k]) {
                    self.wins.insert(slot, pulled.remove(p));
                }
            }
            self.apply_focus();
        }
        self.alt_tab = Some((order, pos));
    }

    fn handle_key(&mut self, evdev: u32, pressed: bool, mods: &ModifiersState, sym: Keysym) -> bool {
        if !pressed {
            // Alt let go: the window chosen with Tab stays
            if self.alt_tab.is_some() && !mods.alt {
                self.alt_tab = None;
            }
            // Print let go before the whole-screen shot went off: it was a short press, so an area is picked
            if self.shot_hold.as_ref().is_some_and(|h| h.key == Some(evdev)) {
                if let Some(h) = self.shot_hold.take() {
                    if !h.fired {
                        self.menu.close();
                        self.shot_sel = Some(ShotSel { start: None, record: false });
                    }
                }
            }
            return self.intercepted.remove(&evdev);
        }
        let had_hold = self.shot_hold.is_some();
        let consumed = self.shortcut(mods, sym);
        if consumed {
            self.intercepted.insert(evdev);
            if !had_hold {
                if let Some(h) = self.shot_hold.as_mut().filter(|h| h.key.is_none()) {
                    h.key = Some(evdev);
                }
            }
        }
        consumed
    }

    /// The pressed combination as the shortcut table spells it (`ctrl+alt+t`).
    /// A key press with no modifiers, as the headless test hook (`MEMRIUM_TEST_INPUT`) sends it.
    pub fn test_key(&mut self, sym: Keysym) {
        self.test_chord(ModifiersState::default(), sym);
    }

    /// The same with modifiers held (`key ctrl+space` in the headless test script).
    pub fn test_chord(&mut self, mods: ModifiersState, sym: Keysym) {
        let chord = Self::chord_of(&mods, sym);
        let used = self.shortcut(&mods, sym);
        eprintln!("test key: chord {chord:?} bound {:?} used {used} launcher open {}", self.config.keys.action_for(&chord), self.launcher.open);
    }

    fn chord_of(mods: &ModifiersState, sym: Keysym) -> String {
        let mut out = String::new();
        for (on, name) in [(mods.ctrl, "ctrl+"), (mods.alt, "alt+"), (mods.shift, "shift+"), (mods.logo, "super+")] {
            if on {
                out.push_str(name);
            }
        }
        let key = match sym.key_char() {
            Some(c) if c.is_alphanumeric() => c.to_lowercase().to_string(),
            _ => sym.name().map(|n| n.trim_start_matches("XK_").trim_start_matches("KEY_").to_lowercase()).unwrap_or_default(),
        };
        out.push_str(&key);
        out
    }

    /// Run a shell action chosen by a shortcut.
    fn key_action(&mut self, action: &str) {
        let top = self.wins.iter().rposition(|w| !w.minimized);
        match action {
            "search" => self.launcher.toggle_as(Some(memrium_de::theme::LauncherKind::Figlet)),
            "menu-search" => self.menu_search(),
            "launcher" | "launcher-alt" => self.launcher.toggle(),
            "lock" => self.lock_screen(),
            "settings" => self.toggle_settings(),
            "wallpaper" => self.bar_action("wallpaper"),
            "files" => self.open_shell(Sw::Files),
            "terminal" => self.bar_command(B_TERMINAL),
            "close-window" | "close-window-2" => {
                if let Some(i) = top {
                    self.close_window(i);
                }
            }
            "minimize" => {
                if let Some(i) = top {
                    self.minimize(i);
                }
            }
            "maximize" => self.snap_step(SnapKey::Up),
            "snap-left" => self.snap_step(SnapKey::Left),
            "snap-right" => self.snap_step(SnapKey::Right),
            "snap-down" => self.snap_step(SnapKey::Down),
            "switch-window" => self.cycle_windows(false),
            "show-desktop" => self.show_desktop(),
            "widgets" => self.toggle_widgets(),
            "notifications" => self.toggle_card(crate::popup::PopKind::Notifications, None, self.size[0] - 220.0),
            "clipboard" => self.toggle_card(crate::popup::PopKind::Clipboard, None, self.size[0] / 2.0),
            // a short press picks an area, holding the key takes the whole screen (see `handle_key` and the tick)
            "screenshot" => {
                if self.shot_hold.is_none() {
                    self.shot_hold = Some(ShotHold { at: Instant::now(), key: None, fired: false });
                }
            }
            "screenshot-window" => self.shot_req = Some(ShotReq::Window),
            "screenshot-region" => {
                self.menu.close();
                self.shot_sel = Some(ShotSel { start: None, record: false });
            }
            "screen-record" => self.toggle_recording(),
            "workspace-next" => self.step_workspace(1),
            "workspace-prev" => self.step_workspace(-1),
            "move-next" => self.move_window_workspace(1),
            "move-prev" => self.move_window_workspace(-1),
            other => eprintln!("unknown shortcut action {other:?}"),
        }
    }

    /// Compositor-level keys. Returns true when the key was consumed.
    fn shortcut(&mut self, mods: &ModifiersState, sym: Keysym) -> bool {
        let (ctrl, alt, logo) = (mods.ctrl, mods.alt, mods.logo);
        // locked: every key is the password's, none reaches anything else
        if self.lock.is_some() {
            self.lock_key(mods, sym);
            return true;
        }
        // a text field of an open card (a Wi-Fi password) takes the typing
        if let Some(pop) = self.card.as_mut().filter(|p| !p.closing && p.ui.editing()) {
            if let Some(k) = Self::to_files_key(sym) {
                if pop.ui.key(k, KeyMods { ctrl, alt, shift: mods.shift }) {
                    return true;
                }
            }
        }
        let chord = Self::chord_of(mods, sym);
        let bound = self.config.keys.action_for(&chord);
        // Shift with the window-switching key goes the other way (the keyboard sends it as its own symbol)
        if alt && !ctrl && !logo && (sym == Keysym::ISO_Left_Tab || (mods.shift && sym == Keysym::Tab)) && self.config.keys.action_for("alt+tab") == Some("switch-window") {
            self.cycle_windows(true);
            return true;
        }
        if matches!(bound, Some("lock")) {
            self.lock_screen();
            return true;
        }
        if self.shot_edit.is_some() {
            self.shot_edit_key(ctrl, sym);
            return true;
        }
        if self.shot_sel.is_some() {
            if sym == Keysym::Escape {
                self.shot_sel = None;
            } else if ctrl && (sym == Keysym::a || sym == Keysym::A) {
                // the whole screen is the area
                let record = self.shot_sel.as_ref().is_some_and(|s| s.record);
                self.shot_sel = None;
                let w = self.world;
                let r = [w[0], w[1], w[2] - w[0], w[3] - w[1]];
                self.shot_req = Some(if record { ShotReq::Record(r) } else { ShotReq::Edit(r) });
            }
            return true;
        }
        if let Some(a @ ("screenshot" | "screenshot-window" | "screenshot-region" | "screen-record")) = bound {
            self.key_action(a);
            return true;
        }
        if self.menu.open {
            let nav = match sym {
                Keysym::Up => Some(NavKey::Up),
                Keysym::Down => Some(NavKey::Down),
                Keysym::Left => Some(NavKey::Left),
                Keysym::Right => Some(NavKey::Right),
                Keysym::Return | Keysym::KP_Enter => Some(NavKey::Enter),
                _ => None,
            };
            if let Some(n) = nav {
                if let Some(id) = self.menu.key_nav(n) {
                    self.menu.close();
                    self.menu_command(id);
                }
            } else {
                self.menu.close();
            }
            return true;
        }
        if self.launcher.open {
            // the key that opened it closes it
            if matches!(bound, Some("launcher" | "launcher-alt" | "search" | "menu-search")) {
                self.key_action(bound.unwrap_or_default());
                return true;
            }
            match sym {
                Keysym::Escape => self.launcher.escape(),
                Keysym::Return | Keysym::KP_Enter if self.launcher.editing() => self.launcher.end_edit(),
                Keysym::Return | Keysym::KP_Enter if self.launcher.figlet => {
                    if let Some(it) = self.launcher.accept_item() {
                        self.spotlight_run(it.target);
                    }
                }
                Keysym::Return | Keysym::KP_Enter => {
                    if let Some(i) = self.launcher.accept() {
                        let app = self.apps[i].clone();
                        self.launch_app(&app);
                    }
                    self.launcher.close();
                }
                Keysym::Left if self.launcher.grid && self.launcher.query.is_empty() => self.launcher.page_by(-1),
                Keysym::Right if self.launcher.grid && self.launcher.query.is_empty() => self.launcher.page_by(1),
                Keysym::Left if self.launcher.grid => self.launcher.move_sel(-1),
                Keysym::Right if self.launcher.grid => self.launcher.move_sel(1),
                Keysym::Up => self.launcher.move_sel(-1),
                Keysym::Down | Keysym::Tab => self.launcher.move_sel(1),
                Keysym::BackSpace => self.launcher.backspace(&self.apps),
                _ => {
                    if let (Some(c), false) = (sym.key_char(), ctrl || alt || logo) {
                        self.launcher.type_char(c, &self.apps);
                    }
                }
            }
            return true;
        }
        // a shortcut of the shell with Ctrl or Alt in it (Ctrl+Space, Alt+Space...) is not for a window of the shell to
        // take as typing: the desktop's icons have the keyboard when nothing else is open
        let global = bound.is_some() && (ctrl || alt);
        // a Settings text field being edited takes the typing
        if self.focused_shell() == Some(Sw::Settings) && !logo && !global {
            if let Some(k) = Self::to_files_key(sym) {
                if self.settings.key(k, ctrl, alt) {
                    return true;
                }
            }
        }
        if self.focused_shell() == Some(Sw::Files) && !logo && !global && !(alt && matches!(sym, Keysym::Tab | Keysym::F4)) && !(ctrl && sym == Keysym::comma) {
            if let Some(k) = Self::to_files_key(sym) {
                let m = KeyMods { ctrl, alt, shift: mods.shift };
                let (used, ev) = self.files.key(k, m, &mut self.fm);
                self.handle_files_events(ev);
                if used {
                    return true;
                }
            }
        }
        if self.desktop.focused && self.focused_shell().is_none() && !logo && !global && !(alt && matches!(sym, Keysym::Tab | Keysym::F4)) && !(ctrl && sym == Keysym::comma) {
            if let Some(k) = Self::to_files_key(sym) {
                let m = KeyMods { ctrl, alt, shift: mods.shift };
                let (used, ev) = self.desktop.key(k, m, &mut self.fm);
                self.handle_desktop_events(ev);
                if used {
                    return true;
                }
            }
        }
        // VT switching (Ctrl+Alt+F1..F12 arrive as XF86Switch_VT_n) and a way out of the session
        let raw = sym.raw();
        if (0x1008FE01..=0x1008FE0C).contains(&raw) {
            self.pending_vt = Some((raw - 0x1008FE01 + 1) as i32);
            return true;
        }
        if self.media_key(raw) {
            return true;
        }
        if sym == Keysym::BackSpace && ctrl && alt {
            self.quit = true;
            return true;
        }
        if let Some(a) = bound {
            self.key_action(a);
            return true;
        }
        if let Some(cmd) = self.config.keys.command_for(&chord).map(str::to_string) {
            self.run_command(&cmd);
            return true;
        }
        if logo && !ctrl && !alt && (sym == Keysym::f || sym == Keysym::F) {
            // Super+F: the front window goes fullscreen and back
            if let Some(i) = self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or) {
                let on = self.wins[i].fs.is_none();
                self.set_fullscreen(i, on);
                return true;
            }
        }
        match sym {
            Keysym::Escape if self.focused_shell() == Some(Sw::Settings) => self.settings.close(),
            Keysym::Escape if self.panel_edit.is_some() => self.panel_edit = None,
            Keysym::Escape if self.scene.edit_mode() => self.scene.set_edit(false),
            Keysym::Escape if self.scene.cc_open() => self.scene.set_cc_open(false),
            _ => return false,
        }
        true
    }

    // ------------------------------------------------------------ apps, Dock, launching

    pub fn launch_app(&mut self, app: &AppEntry) {
        if app.id == apps::SETTINGS_ID {
            self.open_shell(Sw::Settings);
            return;
        }
        if app.id == apps::FILES_ID {
            self.open_shell(Sw::Files);
            return;
        }
        match apps::launch(app, &self.socket) {
            Ok(()) => {
                eprintln!("launched {}", app.name);
                self.start_busy();
            }
            Err(e) => eprintln!("could not launch {}: {e}", app.name),
        }
    }

    /// The icon theme changed: index the themes again off-thread, then drop the loaded icons (see `pump_icons`).
    fn reload_icons(&mut self) {
        memrium_de::apps::set_icon_theme(&self.theme.icons.shell_theme_name(self.theme.dark));
        let (icons, actions) = (self.icons.clone(), self.action_icons.clone());
        let (mime, file_apps) = self.fileicons.slots();
        let done = self.icons_rebuilt.clone();
        std::thread::spawn(move || {
            let apps = Arc::new(IconIndex::build());
            icons.set_arc(apps.clone());
            file_apps.set_arc(apps);
            actions.set(IconIndex::build_actions());
            mime.set(IconIndex::build_mime());
            done.store(true, std::sync::atomic::Ordering::Release);
        });
    }

    /// Make sure an app's icon is in the renderer's atlas (loads at most a few per call; needs the icon index).
    pub fn ensure_icon_pub(&mut self, app_idx: usize, budget: &mut u32) {
        self.ensure_icon(app_idx, budget);
    }

    fn ensure_icon(&mut self, app_idx: usize, budget: &mut u32) {
        let app = &self.apps[app_idx];
        if *budget == 0 || self.renderer.app_icon_get(&app.id).is_some() || self.icon_failed.contains(&app.id) {
            return;
        }
        let Some(index) = self.icons.get() else { return };
        // `MEMRIUM_SYNC_ICONS=1`: the old way, on this thread (to measure what the workers save)
        if std::env::var_os("MEMRIUM_SYNC_ICONS").is_some() {
            *budget -= 1;
            match index.load(&app.icon, 128) {
                Some(img) => {
                    let id = app.id.clone();
                    self.renderer.app_icon_cell(&id, &img);
                }
                None => {
                    self.icon_failed.insert(app.id.clone());
                }
            }
            return;
        }
        // made on other threads: it shows when it is ready (see `pump_icons`)
        *budget -= 1;
        let (id, icon, tile) = (app.id.clone(), app.icon.clone(), self.renderer.icon_tile());
        self.icon_loader.request(&id, &icon, tile);
    }

    /// Put the icons the workers finished into the atlas (a few a frame: copying is quick, but not free).
    pub fn pump_icons(&mut self) {
        // the icon editor touches a marker file whenever it has written icons: index the themes again (a live preview)
        if self.icons_marker.0.elapsed() >= std::time::Duration::from_millis(700) {
            let seen = std::fs::metadata(memrium_de::iconrender::marker_path()).and_then(|m| m.modified()).ok();
            if seen.is_some() && seen != self.icons_marker.1 {
                if self.icons_marker.1.is_some() {
                    self.reload_icons();
                }
            }
            self.icons_marker = (Instant::now(), seen);
        }
        if self.icons_rebuilt.swap(false, std::sync::atomic::Ordering::AcqRel) {
            // every icon is looked up again, in the new theme
            self.renderer.clear_app_icons();
            memrium_de::theme::reset_live_icons();
            self.icon_failed.clear();
            crate::icon_loader::forget_own_packs();
            self.icon_loader = crate::icon_loader::IconLoader::start(self.icons.clone());
            self.fileicons.reset();
            self.invalidate();
        }
        // icons the bar's widgets chose from the pack (the start button, the Dock's marks)
        for name in memrium_de::widget::take_wanted_icons() {
            let id = memrium_de::widget::pack_icon_key(&name);
            if self.renderer.app_icon_get(&id).is_none() && !self.icon_failed.contains(&id) {
                let tile = self.renderer.icon_tile();
                self.icon_loader.request(&id, &name, tile);
            }
        }
        let done = self.icon_loader.take(6);
        if done.is_empty() {
            return;
        }
        for d in done {
            match d.icon {
                Some(baked) => {
                    // made for another tile style than the current one: ask again with the current
                    let inserted = self.renderer.app_icon_insert(&d.id, baked);
                    if let Some(cell) = self.renderer.app_icon_get(&d.id).filter(|_| inserted) {
                        memrium_de::theme::set_live_icon(cell, d.live);
                    }
                    if !inserted {
                        if let Some((pack, icon)) = d.id.strip_prefix("pack:").and_then(|r| r.rsplit_once(':')) {
                            let tile = self.renderer.icon_tile();
                            self.icon_loader.request_pack(&d.id, pack, icon, tile);
                        } else if let Some(name) = d.id.strip_prefix("icon:") {
                            let tile = self.renderer.icon_tile();
                            self.icon_loader.request(&d.id, name, tile);
                        } else if let Some(a) = self.apps.iter().find(|a| a.id == d.id).cloned() {
                            let tile = self.renderer.icon_tile();
                            self.icon_loader.request(&a.id, &a.icon, tile);
                        }
                    }
                }
                None => {
                    self.icon_failed.insert(d.id);
                }
            }
        }
        self.invalidate();
    }

    /// Dock/taskbar key of a window (its app's desktop id, or `win:<app_id>`).
    pub fn key_of(&self, w: &Win) -> String {
        match self.app_index_for_window(w) {
            Some(i) => self.apps[i].id.clone(),
            None => format!("win:{}", w.app_id),
        }
    }

    fn app_index_for_window(&self, w: &Win) -> Option<usize> {
        let e = apps::find_for_app_id(&self.apps, &w.app_id)?;
        self.apps.iter().position(|a| std::ptr::eq(a, e))
    }

    /// Favourite apps as indices into the app list.
    pub fn pinned_indices(&self) -> Vec<usize> {
        self.pinned.iter().filter_map(|id| self.apps.iter().position(|a| &a.id == id)).collect()
    }

    /// The windows an app has open (the Dock's previews and context menu list them).
    pub fn dock_windows(&self, key: &str) -> Vec<DockWin> {
        let mut v: Vec<DockWin> = self
            .wins
            .iter()
            .filter(|w| w.mapped && !w.or && self.key_of(w) == key)
            .filter_map(|w| Some(DockWin { id: win_id(w)?, title: if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() }, minimized: w.minimized }))
            .collect();
        if key == apps::SETTINGS_ID && self.settings.open {
            v.push(DockWin { id: SHELL_SETTINGS, title: "Settings".into(), minimized: self.swin.minimized });
        }
        if key == apps::FILES_ID && self.files.open {
            v.push(DockWin { id: SHELL_FILES, title: "Files".into(), minimized: self.fwin.minimized });
        }
        v
    }

    /// Rebuild the Dock: pinned apps first, then other running apps after a divider.
    fn sync_dock(&mut self) {
        let mut budget = 6u32;
        let mut items: Vec<DockItem> = Vec::new();
        let mut shown: HashSet<String> = HashSet::new();
        let settings_open = self.settings.open;
        let files_open = self.files.open;
        let mut running_keys: HashSet<String> = self
            .wins
            .iter()
            .filter(|w| w.mapped && !w.or)
            .map(|w| self.app_index_for_window(w).map(|i| self.apps[i].id.clone()).unwrap_or_else(|| format!("win:{}", w.app_id)))
            .collect();
        if settings_open {
            running_keys.insert(apps::SETTINGS_ID.to_string());
        }
        if files_open {
            running_keys.insert(apps::FILES_ID.to_string());
        }

        let pinned = self.pinned.clone();
        let mut entries: Vec<(String, String, Option<usize>, bool, bool)> = Vec::new(); // key, name, app index, running, separator
        for id in &pinned {
            if let Some(i) = self.apps.iter().position(|a| &a.id == id) {
                entries.push((id.clone(), self.apps[i].name.clone(), Some(i), running_keys.contains(id), false));
                shown.insert(id.clone());
            }
        }
        let mut first_extra = true;
        let mut extras: Vec<(String, String, Option<usize>)> = Vec::new();
        for w in self.wins.iter().filter(|w| w.mapped && !w.or) {
            let (key, name, idx) = match self.app_index_for_window(w) {
                Some(i) => (self.apps[i].id.clone(), self.apps[i].name.clone(), Some(i)),
                None => (format!("win:{}", w.app_id), if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() }, None),
            };
            if shown.insert(key.clone()) {
                extras.push((key, name, idx));
            }
        }
        if settings_open && !shown.contains(apps::SETTINGS_ID) {
            if let Some(i) = self.apps.iter().position(|a| a.id == apps::SETTINGS_ID) {
                shown.insert(apps::SETTINGS_ID.to_string());
                extras.push((apps::SETTINGS_ID.to_string(), self.apps[i].name.clone(), Some(i)));
            }
        }
        if files_open && !shown.contains(apps::FILES_ID) {
            if let Some(i) = self.apps.iter().position(|a| a.id == apps::FILES_ID) {
                shown.insert(apps::FILES_ID.to_string());
                extras.push((apps::FILES_ID.to_string(), self.apps[i].name.clone(), Some(i)));
            }
        }
        for (key, name, idx) in extras {
            entries.push((key, name, idx, true, first_extra));
            first_extra = false;
        }
        for (key, name, idx, running, sep) in entries {
            if let Some(i) = idx {
                self.ensure_icon(i, &mut budget);
            }
            let icon = match idx.and_then(|i| self.renderer.app_icon_get(&self.apps[i].id)) {
                Some(cell) => DockIcon::App(cell),
                None => DockIcon::Procedural(8, [0.74, 0.80, 0.94], [0.46, 0.54, 0.74]),
            };
            let windows = self.dock_windows(&key);
            let is_pinned = pinned.contains(&key);
            let badge = self.notifications.unread_for(&key, &name) as u32;
            items.push(DockItem { key, name, icon, running, separator_before: sep, windows, pinned: is_pinned, badge });
        }
        let focused = if let Some(k) = self.focused_shell() {
            Some(if k == Sw::Settings { apps::SETTINGS_ID } else { apps::FILES_ID }.to_string())
        } else {
            self.wins.iter().rev().find(|w| w.mapped && !w.minimized && !w.or).map(|w| self.key_of(w))
        };
        let title = self.bar_title();
        let mb = &self.theme.menubar;
        let light = if mb.bar && mb.opacity > 0.6 { self.theme.dark } else { self.menu_light };
        self.scene.set_menu_light(light);
        self.scene.set_menu_title(&title);
        self.sync_bar();
        for b in &mut self.bars {
            b.set_dock(&items, focused.as_deref());
        }
        self.scene.set_dock_focus(items, focused.as_deref());
    }

    /// React to Dock clicks: raise the app's window, or launch it.
    fn process_shell_events(&mut self) {
        use memrium_de::controlcenter::CcEvent;
        use memrium_de::sysctl::Cmd;
        for ev in self.scene.take_cc_events() {
            match ev {
                CcEvent::Media(cmd) => {
                    if let Some(m) = &self.media {
                        m.send(match cmd {
                            memrium_de::controlcenter::CcMediaCmd::Previous => crate::media::MediaCmd::Previous,
                            memrium_de::controlcenter::CcMediaCmd::PlayPause => crate::media::MediaCmd::PlayPause,
                            memrium_de::controlcenter::CcMediaCmd::Next => crate::media::MediaCmd::Next,
                        });
                    }
                }
                CcEvent::SetWifi(on) => self.sys_watcher.send(Cmd::SetWifi(on)),
                CcEvent::SetBluetooth(on) => self.sys_watcher.send(Cmd::SetBluetooth(on)),
                CcEvent::SetVolume(v) => self.sys_watcher.send(Cmd::SetVolume(v)),
                CcEvent::SetBrightness(v) => self.sys_watcher.send(Cmd::SetBrightness(v)),
                CcEvent::ToggleMute => self.sys_watcher.send(Cmd::ToggleMute),
                CcEvent::ToggleDark => self.bar_action("dark-mode"),
                CcEvent::Screenshot => self.shot_req = Some(ShotReq::Full),
                CcEvent::Lock => self.lock_screen(),
                CcEvent::OpenSettings => self.open_shell(Sw::Settings),
                CcEvent::Action(name) => self.bar_action(&name),
                CcEvent::AddWidget(kind) => self.desk_add_widget(&kind),
                CcEvent::Layout(layout) => {
                    self.config.cc_tiles = layout;
                    self.cfg_dirty = Some(Instant::now());
                }
            }
        }
        for ev in self.scene.take_events() {
            match ev {
                ShellEvent::DockClick(k) => self.dock_click(k),
            }
        }
    }

    /// A Dock / taskbar item was clicked: focus its window, or start the app.
    fn dock_click(&mut self, key: String) {
        {
            if key == apps::SETTINGS_ID || key == apps::FILES_ID {
                let k = if key == apps::SETTINGS_ID { Sw::Settings } else { Sw::Files };
                if self.is_open(k) && self.focused_shell() == Some(k) {
                    self.shell_win_mut(k).minimized = true;
                } else {
                    self.open_shell(k);
                }
                return;
            }
            let target = self.wins.iter().rposition(|w| {
                w.mapped && !w.or
                    && match key.strip_prefix("win:") {
                        Some(app_id) => w.app_id == app_id,
                        None => self.app_index_for_window(w).map(|i| self.apps[i].id == key).unwrap_or(false),
                    }
            });
            match target {
                Some(i) => {
                    // a click on the icon of the window that is in front puts it away (as it does for Settings and Files);
                    // otherwise it comes to the front
                    let front = self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or && w.ws == self.workspace);
                    if front == Some(i) && self.focused_shell().is_none() && !self.wins[i].minimized {
                        self.minimize(i);
                    } else {
                        self.focus_window(i);
                    }
                }
                None => {
                    if let Some(app) = self.apps.iter().find(|a| a.id == key).cloned() {
                        self.launch_app(&app);
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ settings and theme changes

    /// Ctrl+, : close Settings if it is in front, otherwise bring it up.
    pub fn toggle_settings(&mut self) {
        if self.focused_shell() == Some(Sw::Settings) {
            self.settings.close();
        } else {
            self.open_shell(Sw::Settings);
        }
    }

    pub fn is_open(&self, k: Sw) -> bool {
        match k {
            Sw::Settings => self.settings.open,
            Sw::Files => self.files.open,
        }
    }

    fn shell_win(&self, k: Sw) -> &ShellWin {
        match k {
            Sw::Settings => &self.swin,
            Sw::Files => &self.fwin,
        }
    }

    fn shell_win_mut(&mut self, k: Sw) -> &mut ShellWin {
        match k {
            Sw::Settings => &mut self.swin,
            Sw::Files => &mut self.fwin,
        }
    }

    fn client_size(&self, k: Sw) -> [f32; 2] {
        self.shell_win(k).size.unwrap_or(match k {
            Sw::Settings => [WIN_W, WIN_H],
            Sw::Files => [files::WIN_W, files::WIN_H],
        })
    }

    /// Maximise / restore a shell window (Files and Settings both lay themselves out for any size).
    pub(crate) fn toggle_shell_maximize(&mut self, k: Sw) {
        let area = self.maximize_area([self.size[0] / 2.0, self.size[1] / 2.0]);
        // the title bar is left out of a maximized window when the setting says so
        let ins = if self.theme.window.hide_title_max { [0.0; 4] } else { decor::insets(&self.theme) };
        let w = self.shell_win_mut(k);
        if let Some(r) = w.restore.take() {
            w.tile = 0;
            w.size = None;
            w.x.target = r[0];
            w.y.target = r[1];
        } else {
            w.restore = Some([w.x.target, w.y.target]);
            w.size = Some([area[2] - ins[0] - ins[2], area[3] - ins[1] - ins[3]]);
            w.x.target = area[0];
            w.y.target = area[1];
        }
    }

    /// A shell window's frame (x, y, w, h), including the theme's decoration.
    pub fn sw_frame(&self, k: Sw) -> [f32; 4] {
        let ins = self.sw_ins(k);
        let c = self.client_size(k);
        let w = self.shell_win(k);
        [w.x.x, w.y.x, c[0] + ins[0] + ins[2], c[1] + ins[1] + ins[3]]
    }

    pub(crate) fn sw_visible(&self, k: Sw) -> bool {
        self.is_open(k) && !self.shell_win(k).minimized && self.shell_win(k).ws == self.workspace
    }

    /// The shell window that has focus (it is in front of every client window).
    pub fn focused_shell(&self) -> Option<Sw> {
        let k = self.z.iter().rev().copied().find(|&k| self.sw_visible(k))?;
        (self.shell_win(k).stamp > self.top_client_stamp()).then_some(k)
    }

    fn raise_shell(&mut self, k: Sw) {
        self.z.retain(|&z| z != k);
        self.z.push(k);
        let stamp = self.next_stamp();
        self.shell_win_mut(k).stamp = stamp;
    }

    /// Open (or restore and raise) a shell window.
    pub fn open_shell(&mut self, k: Sw) {
        if !self.is_open(k) {
            let w = self.shell_win_mut(k);
            w.restore = None;
            w.size = None;
            let ins = decor::insets(&self.theme);
            let c = self.client_size(k);
            let (fw, fh) = (c[0] + ins[0] + ins[2], c[1] + ins[1] + ins[3]);
            let n = self.z.iter().filter(|&&o| o != k && self.is_open(o)).count() as f32;
            let x = ((self.size[0] - fw) / 2.0 + n * 36.0).max(8.0);
            let y = ((self.size[1] - fh) / 2.0 - 20.0 + n * 30.0).max(40.0);
            let w = self.shell_win_mut(k);
            w.genie = false;
            w.x = Spring::new(x, 700.0, 0.8);
            w.y = Spring::new(y, 700.0, 0.8);
            match k {
                Sw::Settings => {
                    self.settings.open();
                    self.load_walls();
                }
                Sw::Files => self.files.open_at(&fsops::home()),
            }
        }
        // asked for from another desktop, it comes to this one
        let ws = self.workspace;
        let w = self.shell_win_mut(k);
        w.ws = ws;
        w.minimized = false;
        self.raise_shell(k);
    }

    pub(crate) fn in_rect(p: [f32; 2], r: [f32; 4]) -> bool {
        p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
    }

    /// Topmost shell window whose frame contains `p` (ignoring client windows).
    fn shell_at(&self, p: [f32; 2]) -> Option<Sw> {
        self.z.iter().rev().copied().find(|&k| self.sw_visible(k) && Self::in_rect(p, self.sw_frame(k)))
    }

    /// The shell window under `p`, if nothing else is stacked above it there.
    pub(crate) fn shell_covers(&self, p: [f32; 2]) -> Option<Sw> {
        let k = self.shell_at(p)?;
        match self.client_stamp_at(p) {
            Some(above) if above > self.shell_win(k).stamp => None,
            _ => Some(k),
        }
    }

    fn key_mods(&self) -> KeyMods {
        let m = self.keyboard.modifier_state();
        KeyMods { ctrl: m.ctrl, alt: m.alt, shift: m.shift }
    }

    /// The shell window whose edge or corner `p` is on, with the edges (nothing is stacked above it there, and it is not maximised).
    pub(crate) fn shell_edge_hit(&self, p: [f32; 2]) -> Option<(Sw, u8)> {
        for &k in self.z.iter().rev() {
            if !self.sw_visible(k) || self.shell_win(k).restore.is_some() || self.focused_shell() != Some(k) {
                continue;
            }
            let e = Self::edges_of(self.sw_frame(k), p);
            if e == 0 {
                continue;
            }
            // a client window drawn above this one owns the point
            if self.client_stamp_at(p).is_some_and(|above| above > self.shell_win(k).stamp) {
                return None;
            }
            return Some((k, e));
        }
        None
    }

    /// Resize a shell window while an edge or corner is dragged.
    fn apply_shell_resize(&mut self, k: Sw, edges: u8, start_mouse: [f32; 2], start: [f32; 4]) {
        let ins = decor::insets(&self.theme);
        let (ix, iy) = (ins[0] + ins[2], ins[1] + ins[3]);
        let (dx, dy) = (self.mouse[0] - start_mouse[0], self.mouse[1] - start_mouse[1]);
        let (mut nw, mut nh) = (start[2], start[3]);
        if edges & 2 != 0 {
            nw = start[2] + dx;
        }
        if edges & 1 != 0 {
            nw = start[2] - dx;
        }
        if edges & 8 != 0 {
            nh = start[3] + dy;
        }
        if edges & 4 != 0 {
            nh = start[3] - dy;
        }
        // a window cannot be pulled out over the menu bar or off the screen
        {
            let area = self.work_area();
            if edges & 4 != 0 {
                let bottom = start[1] + start[3];
                nh = bottom - (bottom - nh).max(area[1]);
            }
            if edges & 1 != 0 {
                let right = start[0] + start[2];
                nw = right - (right - nw).max(area[0]);
            }
        }
        // small enough to be handy, big enough that the window's pages still lay out
        nw = nw.max(520.0 + ix);
        nh = nh.max(380.0 + iy);
        let w = self.shell_win_mut(k);
        w.size = Some([nw - ix, nh - iy]);
        // dragging the left or top edge keeps the opposite edge where it was
        if edges & 1 != 0 {
            w.x.target = start[0] + (start[2] - nw);
            w.x.x = w.x.target;
        }
        if edges & 4 != 0 {
            w.y.target = start[1] + (start[3] - nh);
            w.y.x = w.y.target;
        }
    }

    /// A press on a shell window: chrome buttons, titlebar drag, or content. True when consumed.
    fn shell_pointer_down(&mut self, right: bool) -> bool {
        let p = self.mouse;
        if !right {
            if let Some((k, e)) = self.shell_edge_hit(p) {
                self.raise_shell(k);
                self.grab = Some(Grab::ShellResize { win: k, edges: e, start_mouse: p, start: self.sw_frame(k) });
                return true;
            }
        }
        let Some(k) = self.shell_covers(p) else { return false };
        self.raise_shell(k);
        let f = self.sw_frame(k);
        let ins = self.sw_ins(k);
        let theme = self.sw_theme(k).into_owned();
        if !right {
            for (b, r) in decor::buttons(&theme, f) {
                if Self::in_rect(p, r) {
                    match b {
                        Btn::Close => match k {
                            Sw::Settings => self.settings.close(),
                            Sw::Files => self.files.close(),
                        },
                        Btn::Minimize => self.shell_win_mut(k).minimized = true,
                        Btn::Maximize => self.toggle_shell_maximize(k),
                    }
                    return true;
                }
            }
            if p[1] < f[1] + ins[1] {
                self.grab = Some(Grab::ShellMove { win: k, off: [p[0] - f[0], p[1] - f[1]], start: p });
                return true;
            }
        } else if p[1] < f[1] + ins[1] {
            return true;
        }
        match k {
            Sw::Settings => {
                if !right {
                    self.settings.mouse_down(p);
                }
            }
            Sw::Files => {
                let m = self.key_mods();
                let ev = self.files.mouse_down(p, right, m, &mut self.fm);
                self.handle_files_events(ev);
            }
        }
        true
    }

    /// Events from the desktop's icons: a menu they open sends its commands back to the desktop, not to the Files window.
    fn handle_desktop_events(&mut self, events: Vec<FilesEvent>) {
        self.events_from_desktop = true;
        self.handle_files_events(events);
        self.events_from_desktop = false;
    }

    fn handle_files_events(&mut self, events: Vec<FilesEvent>) {
        for ev in events {
            match ev {
                FilesEvent::Menu(items, at) => {
                    self.menu_for = if self.events_from_desktop { MenuFor::DesktopIcons } else { MenuFor::Files };
                    // a view that does not know where the pointer is (a name committed with Enter) asks for it with -1
                    let at = if at[0] < 0.0 { self.mouse } else { at };
                    self.menu.open_at(at, items);
                }
                FilesEvent::Open(path) => fsops::open_external(&path, &self.socket),
                FilesEvent::Own(program, args) => fsops::spawn_own(program, &args, &self.socket),
                FilesEvent::OpenFolder(path) => {
                    self.open_shell(Sw::Files);
                    self.files.goto(path, true);
                }
                FilesEvent::DragStart(paths) => {
                    self.file_drag = Some(FileDrag { paths, start: self.mouse, active: false });
                }
                FilesEvent::Terminal(dir) => self.open_terminal_in(&dir),
                FilesEvent::Prefs => {
                    self.config.files = self.files.prefs.clone();
                    self.config.desktop = self.desktop.prefs.clone();
                    self.cfg_dirty = Some(Instant::now());
                }
            }
        }
    }

    fn open_terminal_in(&mut self, dir: &std::path::Path) {
        if let Some(app) = self.apps.iter().find(|a| a.id == "memrium-term").cloned() {
            if let Err(e) = apps::launch_in(&app, &self.socket, Some(dir)) {
                eprintln!("could not open a terminal: {e}");
            }
        }
    }

    fn to_files_key(sym: Keysym) -> Option<Key> {
        Some(match sym {
            Keysym::Up => Key::Up,
            Keysym::Down => Key::Down,
            Keysym::Left => Key::Left,
            Keysym::Right => Key::Right,
            Keysym::Return | Keysym::KP_Enter => Key::Enter,
            Keysym::Escape => Key::Esc,
            Keysym::BackSpace => Key::Backspace,
            Keysym::Delete => Key::Delete,
            Keysym::F2 => Key::F2,
            Keysym::F5 => Key::F5,
            Keysym::Home => Key::Home,
            Keysym::End => Key::End,
            Keysym::Page_Up => Key::PageUp,
            Keysym::Page_Down => Key::PageDown,
            other => Key::Char(other.key_char()?),
        })
    }

    // ------------------------------------------------------------ desktop context menu

    /// Is `p` an empty bit of desktop (no window, bar, popup or shell window there)?
    fn desktop_hit(&self, p: [f32; 2]) -> bool {
        !self.shell_hit() && !self.launcher.visible() && self.shell_covers(p).is_none() && self.chrome_at(p).is_none() && self.surface_under(p).is_none()
    }

    pub fn open_desktop_menu(&mut self) {
        use files::cmd::*;
        let mut items = self.desktop.background_menu_items(&self.fm);
        items.extend([
            MenuItem::sep(),
            MenuItem::new(0, "Open").icon("folder-open").sub(vec![MenuItem::new(D_FILES, "Files").icon("folder"), MenuItem::new(D_TERMINAL, "Terminal").icon("terminal"), MenuItem::new(D_DESKTOP_DIR, "Desktop folder").icon("monitor")]),
            MenuItem::sep(),
            // one edit mode: widgets and controls are added, moved and removed in it
            MenuItem::new(D_EDIT, if self.scene.edit_mode() { "Exit Edit Mode" } else { "Enter Edit Mode" }).icon("pencil"),
            MenuItem::sep(),
            MenuItem::new(D_WALL, "Desktop and Wallpaper").icon("image").key("Super+Shift+D"),
            MenuItem::new(D_DISPLAY, "Display Configuration").icon("monitor-cog"),
        ]);
        self.menu_for = MenuFor::Desktop;
        self.menu.open_at(self.mouse, items);
    }

    // ------------------------------------------------------------ menu bar menus (macOS layout)

    /// Bars (menu bar, Dock, taskbar) are drawn from `theme.bars` in every layout.
    pub(crate) fn bar_active(&self) -> bool {
        !self.fullscreen_on_top()
    }

    /// Titles of the focused app's menus (File / Edit / View / (Go) / Window / Help).
    pub(crate) fn bar_menus(&self) -> Vec<String> {
        use memrium_de::i18n::tr;
        let app = self.app_menus();
        // a program that gives menus has its own File / Edit / View, in its own language ("Файл", "Редагувати", "Вид"),
        // so the shell's would only repeat them under other names
        let mut names: Vec<String> = Vec::new();
        if app.is_empty() {
            names.extend(["File", "Edit", "View"].map(String::from));
            if self.focused_shell() == Some(Sw::Files) {
                names.push("Go".into());
            }
        }
        let is = |title: &str, key: &str| title == key || title.to_lowercase() == tr(key).to_lowercase();
        // the front program's own menus: the same title replaces the shell's, a new one comes before Window
        let mut help = None;
        for m in app {
            if is(&m.title, "Window") {
                continue;
            }
            if is(&m.title, "Help") {
                help = Some(m.title);
            } else if !names.contains(&m.title) {
                names.push(m.title);
            }
        }
        // Window and Help stay last; a program's own Help is shown in place of the shell's (see `open_bar_menu`)
        names.push("Window".to_string());
        names.push(help.unwrap_or_else(|| "Help".to_string()));
        names
    }

    /// Run `f` with the bars' drawing context.
    /// The monitor under a desktop point (the nearest one when it is outside all of them).
    pub fn mon_at(&self, p: [f32; 2]) -> usize {
        self.mons.iter().position(|m| p[0] >= m[0] && p[0] < m[0] + m[2] && p[1] >= m[1] && p[1] < m[1] + m[3]).unwrap_or(self.main_mon)
    }

    pub fn mon_origin(&self, i: usize) -> [f32; 2] {
        self.mons.get(i).map(|m| [m[0], m[1]]).unwrap_or([0.0; 2])
    }

    /// A press on the empty part of the bar along the top while the front window is maximized with its title bar left out: a
    /// quick second press zooms (restores) the window, and dragging pulls it loose and carries it, as the title bar would.
    /// True when the press was used.
    fn bar_title_drag(&mut self) -> bool {
        if !self.window_buttons_visible() {
            return false;
        }
        // the Quick Settings pill (and anything else of the shell) on the bar is not empty bar
        if self.scene.shell_hit() {
            return false;
        }
        let Some(b) = self.bar_at(self.mouse) else { return false };
        if self.bars.get(b).map(|b| b.cfg.edge) != Some(memrium_de::theme::Edge::Top) {
            return false;
        }
        let now = Instant::now();
        let double = self.last_title_click.is_some_and(|t| now.duration_since(t).as_millis() < 350);
        self.last_title_click = if double { None } else { Some(now) };
        if let Some(k) = self.focused_shell() {
            if double {
                self.toggle_shell_maximize(k);
            } else {
                let w = self.shell_win(k);
                self.grab = Some(Grab::ShellMove { win: k, off: [self.mouse[0] - w.x.x, self.mouse[1] - w.y.x], start: self.mouse });
            }
            return true;
        }
        let Some(i) = self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or) else { return false };
        if double {
            self.toggle_maximize(i);
            return true;
        }
        let Some(surface) = self.wins[i].role.surface() else { return false };
        let (x, y) = (self.wins[i].x.x, self.wins[i].y.x);
        self.grab = Some(Grab::Move { surf: surface, off: [self.mouse[0] - x, self.mouse[1] - y], start: self.mouse });
        true
    }

    /// The window in front is maximized and its title bar is left out: the bar shows its buttons.
    fn window_buttons_visible(&self) -> bool {
        if !self.theme.window.hide_title_max {
            return false;
        }
        match self.focused_shell() {
            Some(k) => self.shell_win(k).restore.is_some(),
            None => self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or).is_some_and(|i| {
                let w = &self.wins[i];
                w.restore.is_some() && w.deco == Deco::Server && w.fs.is_none()
            }),
        }
    }

    /// Insets of a client window's decoration: none while its title bar is left out.
    pub fn client_ins(&self, w: &Win) -> [f32; 4] {
        if w.deco != Deco::Server || w.fs.is_some() {
            [0.0; 4]
        } else if self.theme.window.hide_title_max && w.restore.is_some() {
            [0.0; 4]
        } else {
            decor::insets(&self.theme)
        }
    }

    /// The same for a shell window (Settings, Files).
    fn sw_ins(&self, k: Sw) -> [f32; 4] {
        if self.theme.window.hide_title_max && self.shell_win(k).restore.is_some() {
            [0.0; 4]
        } else {
            decor::insets(&self.theme)
        }
    }

    /// The theme to draw and hit-test a shell window with: without a title bar while it is maximized and that is left out.
    fn sw_theme(&self, k: Sw) -> std::borrow::Cow<'_, Theme> {
        if self.theme.window.hide_title_max && self.shell_win(k).restore.is_some() {
            let mut t = self.theme.clone();
            t.window.decoration = DecoKind::Minimal;
            std::borrow::Cow::Owned(t)
        } else {
            std::borrow::Cow::Borrowed(&self.theme)
        }
    }

    pub(crate) fn with_bar_ctx<R>(&self, f: impl FnOnce(&BarCtx) -> R) -> R {
        let title = self.bar_title();
        let menus = if self.bar_active() { self.bar_menus() } else { Vec::new() };
        let open = match (self.menu.open, self.menu_for) {
            (true, MenuFor::Bar(h)) => Some(h),
            _ => None,
        };
        let r = &self.renderer;
        let bm = self.bar_mon.get();
        let origin = self.mon_origin(bm);
        let measure = |t: &str, s: f32, b: bool| r.text_width(t, s, b);
        // the windows in the way of an auto-hiding bar, in this monitor's own coordinates
        let wins: Vec<[f32; 4]> = self.wins.iter().filter(|w| w.mapped && !w.minimized && !w.or && w.ws == self.workspace).map(|w| { let f = Self::frame_rect(w); [f[0] - origin[0], f[1] - origin[1], f[2], f[3]] }).collect();
        let layout = self.layout_label();
        let icon = |key: &str| r.app_icon_get(key);
        let ctx = BarCtx {
            theme: &self.theme,
            screen: self.mons.get(bm).map_or(self.size, |m| [m[2], m[3]]),
            measure: &measure,
            mouse: [self.mouse[0] - origin[0], self.mouse[1] - origin[1]],
            menu_light: self.menu_light,
            title: &title,
            menus: &menus,
            hover: if bm == self.bar_hover_mon { self.bar_hover } else { None },
            open: if bm == self.bar_menu_mon { open } else { None },
            start_open: self.launcher.open,
            icon: &icon,
            workspace: self.workspace,
            windows: &wins,
            sys: &self.sys,
            vertical: false,
            axis: [0.0; 2],
            tray: &self.tray_shown,
            layout: &layout,
            notif_unread: self.notifications.unread(),
            dnd: self.config.notifications.dnd,
            ink: [0.1; 3],
            under: [0.0; 4],
            adaptive: true,
            fixed_ink: false,
            window_buttons: self.window_buttons_visible(),
        };
        // a side bar runs between the bars along the top and bottom
        let mut ctx = ctx;
        ctx.axis = self.axis_cache.get();
        f(&ctx)
    }

    /// Keep the bars in step with the theme and tell the Quick Settings where its pill is.
    fn sync_bar(&mut self) {
        if !self.bar_active() {
            // a fullscreen window has nothing of the shell over it, the Quick Settings pill included
            self.scene.set_pill(self.size[0] - 173.0, 19.0, false, false);
            return;
        }
        self.bar_mon.set(self.main_mon);
        let cfgs = self.theme.bars.clone();
        if self.bars.len() != cfgs.len() {
            self.bars = cfgs.iter().enumerate().map(|(i, c)| Bar::new(i, c)).collect();
        } else {
            for (b, c) in self.bars.iter_mut().zip(&cfgs) {
                b.sync(c);
            }
        }
        let bars = &self.bars;
        // the Quick Settings hangs from the pill widget, or from a widget that only opens it (the tray)
        let found = self.with_bar_ctx(|ctx| {
            let all: Vec<_> = bars.iter().filter_map(|b| b.cc_rect(ctx).map(|(r, role)| (r, role, b.cfg.edge))).collect();
            all.iter().find(|(_, role, _)| *role == memrium_de::widget::CcRole::Pill).or(all.first()).copied()
        });
        match found {
            Some((r, role, edge)) => self.scene.set_pill(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, edge == memrium_de::theme::Edge::Bottom, role == memrium_de::widget::CcRole::Pill),
            None => self.scene.set_pill(self.size[0] - 173.0, 19.0, false, false),
        }
    }

    /// Advance the widgets' animations (Dock magnification).
    fn update_bars(&mut self, dt: f32) {
        if !self.bar_active() {
            return;
        }
        self.bar_mon.set(self.main_mon);
        let mut bars = std::mem::take(&mut self.bars);
        // a side bar runs between the bars along the top and bottom: how much they take is looked up once a frame
        let axis = if bars.iter().any(|b| b.cfg.edge.vertical()) {
            self.with_bar_ctx(|ctx| {
                let (mut a0, mut a1) = (0.0f32, 0.0f32);
                for b in bars.iter().filter(|b| !b.cfg.edge.vertical()) {
                    let r = b.reserved(ctx);
                    a0 = a0.max(r[1]);
                    a1 = a1.max(r[3]);
                }
                [a0, a1]
            })
        } else {
            [0.0; 2]
        };
        self.axis_cache.set(axis);
        self.with_bar_ctx(|ctx| {
            for b in &mut bars {
                b.update(dt, ctx);
            }
        });
        self.bars = bars;
    }

    /// Is a full-screen launcher (Launchpad) up? It takes every click and all the pointer's attention.
    pub fn launcher_modal(&self) -> bool {
        self.launcher.grid && self.launcher.visible()
    }

    pub(crate) fn bar_hit(&self, p: [f32; 2]) -> Option<BarHit> {
        if !self.bar_active() {
            return None;
        }
        // the bars repeat on every monitor: look at the one under the pointer, in its own coordinates
        let i = self.mon_at(p);
        self.bar_mon.set(i);
        let o = self.mon_origin(i);
        self.with_bar_ctx(|ctx| self.bars.iter().find_map(|b| b.hit(ctx, [p[0] - o[0], p[1] - o[1]])))
    }

    /// The bar whose surface is under `p`, if any.
    pub(crate) fn bar_at(&self, p: [f32; 2]) -> Option<usize> {
        if !self.bar_active() {
            return None;
        }
        let i = self.mon_at(p);
        self.bar_mon.set(i);
        let o = self.mon_origin(i);
        self.with_bar_ctx(|ctx| self.bars.iter().position(|b| b.contains(ctx, [p[0] - o[0], p[1] - o[1]])))
    }

    /// Is the pointer on the Dock, a bar widget or a Quick Settings tile (so it must not reach windows)?
    fn shell_hit(&self) -> bool {
        if self.preview_hit(self.mouse) || self.scene.shell_hit() || self.toast_at(self.mouse).is_some() || self.flyout_hit(self.mouse) || self.widgets_hit(self.mouse) || self.card_hit(self.mouse) {
            return true;
        }
        if !self.bar_active() {
            return false;
        }
        let i = self.mon_at(self.mouse);
        self.bar_mon.set(i);
        let o = self.mon_origin(i);
        let local = [self.mouse[0] - o[0], self.mouse[1] - o[1]];
        self.with_bar_ctx(|ctx| self.bars.iter().any(|b| b.blocks(ctx, local) || b.hit(ctx, local).is_some()))
    }

    /// Where a minimising window should fly: the centre of its Dock icon.
    fn item_center(&self, key: &str) -> Option<[f32; 2]> {
        self.bars.iter().find_map(|b| b.item_center(key)).or_else(|| self.scene.item_center(key))
    }

    /// Name of the focused app (shown bold after the logo).
    pub fn bar_title(&self) -> String {
        match self.focused_shell() {
            Some(Sw::Settings) => "Settings".to_string(),
            Some(Sw::Files) => "Files".to_string(),
            None => match self.wins.iter().rev().find(|w| w.mapped && !w.minimized && !w.or) {
                Some(w) => match self.app_index_for_window(w) {
                    Some(i) => self.apps[i].name.clone(),
                    None if !w.title.is_empty() => w.title.clone(),
                    None => w.app_id.clone(),
                },
                None => "Desktop".to_string(),
            },
        }
    }

    /// Panels and labels of every bar: backdrops and widgets.
    fn bar_draw(&self) -> (Vec<Panel>, Vec<Label>) {
        let (mut p, mut l) = (Vec::new(), Vec::new());
        if !self.bar_active() {
            return (p, l);
        }
        for i in 0..self.mons.len() {
            self.bar_mon.set(i);
            let o = self.mon_origin(i);
            self.with_bar_ctx(|ctx| {
                for b in &self.bars {
                    let (mut bp, mut bl) = b.draw(ctx);
                    // the bars repeat on every monitor, each laid out for its own screen
                    if o != [0.0, 0.0] {
                        for q in &mut bp {
                            q.rect[0] += o[0];
                            q.rect[1] += o[1];
                        }
                        for t in &mut bl {
                            t.x += o[0];
                            t.y += o[1];
                        }
                    }
                    p.extend(bp);
                    l.extend(bl);
                }
            });
        }
        self.bar_mon.set(self.main_mon);
        (p, l)
    }

    /// A click (or, while a bar menu is open, a hover) on a widget.
    fn bar_click(&mut self, hit: BarHit, hover: bool) {
        let mut bars = std::mem::take(&mut self.bars);
        let click = self.with_bar_ctx(|ctx| bars.get_mut(hit.bar).and_then(|b| b.click(ctx, hit)));
        self.bars = bars;
        let Some(click) = click else { return };
        match click {
            BarClick::Menu { name, x } => self.open_bar_menu(hit, &name, x),
            BarClick::Action(a) if !hover => {
                self.menu.close();
                match a.as_str() {
                    "volume" => self.toggle_flyout(crate::flyout::FlyKind::Volume, hit.bar, self.mouse[0]),
                    "brightness" => self.toggle_flyout(crate::flyout::FlyKind::Brightness, hit.bar, self.mouse[0]),
                    "power" => self.toggle_card(crate::popup::PopKind::Power, Some(hit.bar), self.mouse[0]),
                    "calendar" => self.toggle_card(crate::popup::PopKind::Calendar, Some(hit.bar), self.mouse[0]),
                    "audio" => self.toggle_card(crate::popup::PopKind::Audio, Some(hit.bar), self.mouse[0]),
                    "network" => self.toggle_card(crate::popup::PopKind::Network, Some(hit.bar), self.mouse[0]),
                    "keyboard" => self.open_layout_menu(),
                    "notifications" => self.toggle_card(crate::popup::PopKind::Notifications, Some(hit.bar), self.mouse[0]),
                    "clipboard" => self.toggle_card(crate::popup::PopKind::Clipboard, Some(hit.bar), self.mouse[0]),
                    "bluetooth" => self.toggle_card(crate::popup::PopKind::Bluetooth, Some(hit.bar), self.mouse[0]),
                    _ => self.bar_action(&a),
                }
            }
            BarClick::Dock(key) if !hover => {
                self.menu.close();
                self.dock_click(key);
            }
            _ => {}
        }
    }

    /// Load the keymap the preferences describe (the Keyboard page changed the layouts or how they are switched).
    fn apply_keymap(&mut self) {
        let (layout, variant, options) = memrium_de::input::xkb_settings(&self.config.input);
        let xkb = smithay::input::keyboard::XkbConfig { layout: &layout, variant: &variant, options: (!options.is_empty()).then(|| options.clone()), ..Default::default() };
        let kb = self.keyboard.clone();
        match kb.set_xkb_config(self, xkb) {
            Ok(()) => {
                eprintln!("keyboard: layout {layout} {variant} options {options}");
                self.kbd_layouts = layout.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
            Err(e) => eprintln!("keyboard: that keymap does not load ({e:?}); keeping the old one"),
        }
    }

    /// Index of the layout in use.
    fn layout_index(&self) -> usize {
        self.keyboard.modifier_state().serialized.layout_effective as usize
    }

    /// The short name of the layout in use (`UA`); empty when the keymap has only one.
    fn layout_label(&self) -> String {
        if self.kbd_layouts.len() < 2 {
            return String::new();
        }
        let code = self.kbd_layouts.get(self.layout_index()).map(String::as_str).unwrap_or("");
        memrium_de::input::layout_short(code)
    }

    /// The list of layouts, under the bar, the one in use ticked.
    fn open_layout_menu(&mut self) {
        if self.kbd_layouts.len() < 2 {
            return;
        }
        let now = self.layout_index();
        let mut items: Vec<MenuItem> = self.kbd_layouts.iter().enumerate().map(|(i, c)| MenuItem::new(i as u32 + 1, memrium_de::input::layout_name(c)).checked(i == now)).collect();
        items.push(MenuItem::sep());
        items.push(MenuItem::new(1000, "Keyboard Settings…").icon("settings"));
        let at = self.mouse;
        self.menu_for = MenuFor::Layout;
        let height: f32 = items.iter().map(|i| if i.separator { 11.0 } else { 32.0 }).sum::<f32>() + 14.0;
        let y = if at[1] > self.size[1] / 2.0 { (at[1] - 24.0 - height).max(8.0) } else { at[1] + 18.0 };
        self.menu.open_at([at[0] - 20.0, y], items);
    }

    /// Minimise every window, or bring them back when they are all minimised already.
    fn show_desktop(&mut self) {
        let any_up = self.wins.iter().any(|w| w.mapped && !w.minimized && !w.or);
        for i in 0..self.wins.len() {
            let w = &mut self.wins[i];
            if w.mapped && !w.or {
                w.minimized = any_up;
                w.min_k.target = if any_up { 1.0 } else { 0.0 };
            }
        }
        for k in [Sw::Settings, Sw::Files] {
            if self.is_open(k) {
                self.shell_win_mut(k).minimized = any_up;
            }
        }
        self.apply_focus();
    }

    /// Named actions that bar buttons (and, later, key bindings) can run.
    pub fn bar_action(&mut self, name: &str) {
        match name {
            "win-close" => self.bar_command(B_CLOSE),
            "win-minimize" => self.bar_command(B_MINIMIZE),
            "win-zoom" => match self.focused_shell() {
                Some(k) => self.toggle_shell_maximize(k),
                None => {
                    if let Some(i) = self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or) {
                        self.toggle_maximize(i);
                    }
                }
            },
            "search" => self.launcher.toggle_as(Some(memrium_de::theme::LauncherKind::Figlet)),
            "launcher" | "start" => self.launcher.toggle(),
            // the start button: which menu, where it sits, favourites as a grid ("start:<menu>:<align>:<grid>")
            other if other.starts_with("start:") => {
                use memrium_de::theme::LauncherKind as K;
                let mut parts = other["start:".len()..].split(':');
                let kind = match parts.next() {
                    Some("plasma") => Some(K::Plasma),
                    Some("fluent") => Some(K::StartMenuWin11),
                    Some("aero") => Some(K::StartMenuWin7),
                    Some("launchpad") => Some(K::Launchpad),
                    Some("search") => Some(K::Figlet),
                    _ => None,
                };
                let align = memrium_de::launcher::MenuAlign::parse(parts.next().unwrap_or(""));
                self.launcher.open_from_start(kind, align, parts.next() == Some("1"));
            }
            "lock" => self.lock_screen(),
            "show-desktop" => self.show_desktop(),
            "widgets" => self.toggle_widgets(),
            "notifications" => self.toggle_card(crate::popup::PopKind::Notifications, None, self.size[0] - 220.0),
            "clipboard" => self.toggle_card(crate::popup::PopKind::Clipboard, None, self.size[0] / 2.0),
            "tray-more" => {
                self.tray_expanded = !self.tray_expanded;
                self.tray_refresh();
            }
            other if other.starts_with("tray:") => self.tray_click(&other[5..], 0),
            "power" => self.toggle_card(crate::popup::PopKind::Power, None, self.mouse[0]),
            "calendar" => self.toggle_card(crate::popup::PopKind::Calendar, None, self.mouse[0]),
            "audio" => self.toggle_card(crate::popup::PopKind::Audio, None, self.mouse[0]),
            "network" => self.toggle_card(crate::popup::PopKind::Network, None, self.mouse[0]),
            "bluetooth" => self.toggle_card(crate::popup::PopKind::Bluetooth, None, self.mouse[0]),
            "settings" => self.open_shell(Sw::Settings),
            "files" => self.open_shell(Sw::Files),
            "terminal" => self.bar_command(B_TERMINAL),
            "screenshot" => self.shot_req = Some(ShotReq::Full),
            "screenshot-region" => self.shot_sel = Some(ShotSel { start: None, record: false }),
            "screen-record" => self.toggle_recording(),
            "control-center" => self.bar_command(B_CC),
            "edit-controls" => self.bar_command(B_EDIT),
            "volume-up" | "volume-down" => {
                let v = self.sys.volume.unwrap_or(0.5) + if name == "volume-up" { 0.05 } else { -0.05 };
                self.sys_watcher.send(memrium_de::sysctl::Cmd::SetVolume(v.clamp(0.0, 1.0)));
            }
            "brightness-up" | "brightness-down" => {
                let v = self.sys.brightness.unwrap_or(0.5) + if name == "brightness-up" { 0.05 } else { -0.05 };
                self.sys_watcher.send(memrium_de::sysctl::Cmd::SetBrightness(v.clamp(0.02, 1.0)));
            }
            "mute" => self.sys_watcher.send(memrium_de::sysctl::Cmd::ToggleMute),
            "next-desktop" => self.step_workspace(1),
            "prev-desktop" => self.step_workspace(-1),
            "quit" => self.quit = true,
            "dark-mode" => {
                let mut t = self.theme.clone();
                t.dark = !t.dark;
                self.apply_theme(t);
            }
            "wallpaper" => {
                self.open_shell(Sw::Settings);
                self.settings.set_page(Settings::page_by_name("wallpaper").unwrap_or(0));
            }
            // `exec:program --args`: run any command (from a button or a Quick Settings tile)
            other if other.starts_with("exec:") => {
                let cmd = other[5..].trim();
                let mut c = std::process::Command::new("sh");
                c.args(["-c", cmd]).current_dir(fsops::home()).env("WAYLAND_DISPLAY", &self.socket).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                if let Err(e) = c.spawn() {
                    eprintln!("could not run {cmd:?}: {e}");
                }
            }
            other if other.starts_with("workspace:") => {
                if let Ok(n) = other[10..].parse::<usize>() {
                    self.switch_workspace(n.saturating_sub(1));
                }
            }
            other => eprintln!("unknown action {other:?}"),
        }
    }

    /// Do what a Figlet row says.
    fn spotlight_run(&mut self, target: Target) {
        if let Target::Calc(answer) = target {
            // keep Figlet open and continue from the answer
            let apps = self.apps.clone();
            self.launcher.set_query(&answer, &apps);
            return;
        }
        self.launcher.close();
        match target {
            Target::App(i) => {
                if let Some(app) = self.apps.get(i).cloned() {
                    self.launch_app(&app);
                }
            }
            Target::File(path) => {
                if path.is_dir() {
                    self.open_shell(Sw::Files);
                    self.files.goto(path, true);
                } else {
                    fsops::open_external(&path, &self.socket);
                }
            }
            Target::Action(a) => self.bar_action(a),
            Target::Web(q) => fsops::open_external(std::path::Path::new(&figlet::search_url(&q)), &self.socket),
            Target::Run(cmd) => {
                let mut c = std::process::Command::new("sh");
                c.args(["-c", &cmd]).current_dir(fsops::home()).env("WAYLAND_DISPLAY", &self.socket).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                if let Err(e) = c.spawn() {
                    eprintln!("could not run {cmd:?}: {e}");
                }
            }
            Target::MenuCmd(id) => {
                // the program is told, as if the entry had been chosen in its menu
                match self.menu_search_target.take() {
                    // a shell window's menu: its entries are the bar's own
                    None => self.bar_command(id),
                    target => {
                        self.appmenu_target = target;
                        self.app_menu_command(id);
                    }
                }
            }
            Target::Calc(_) => {}
        }
    }

    fn open_bar_menu(&mut self, hit: BarHit, name: &str, x: f32) {
        let client = self.wins.iter().any(|w| w.mapped && !w.minimized && !w.or) && self.focused_shell().is_none();
        let top_client = self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or);
        let _ = (client, top_client);
        let mut from_app = self.app_menu_items(name);
        if let Some(items) = from_app.as_mut() {
            self.resolve_menu_icons(items);
        }
        self.appmenu_target = self.menu_target();
        let Some(m) = self.bar_menu_items(name, from_app) else { return };
        self.open_bar_menu_at(hit, x, m);
    }

    /// The entries of the bar menu called `name` for the front window: its program's own when it gave `from_app`, else
    /// the shell's (`None` when there is no such menu).
    pub(crate) fn bar_menu_items(&self, name: &str, from_app: Option<Vec<MenuItem>>) -> Option<Vec<MenuItem>> {
        use files::cmd::*;
        let files = self.focused_shell() == Some(Sw::Files);
        let m: Vec<MenuItem> = match name {
            _ if from_app.is_some() => from_app.unwrap_or_default(),
            "logo" => vec![
                MenuItem::new(B_ABOUT, "About Memrium").icon("info"),
                MenuItem::sep(),
                MenuItem::new(B_SETTINGS, "Settings…").icon("settings").key("Ctrl+,"),
                MenuItem::new(B_FILES, "Files").icon("folder").key("Super+E"),
                MenuItem::new(B_TERMINAL, "Terminal").icon("terminal"),
                MenuItem::new(B_LAUNCHER, "Search…").icon("search").key("Super+S"),
                // the search and the grid of apps are two things: a theme whose launcher is the search needs only one
                MenuItem::new(B_LAUNCHPAD, if self.theme.launcher.style == memrium_de::theme::LauncherKind::Launchpad { "Launchpad" } else { "Apps" }).icon("layout-grid").key("Alt+Space").enabled(self.theme.launcher.style != memrium_de::theme::LauncherKind::Figlet),
                MenuItem::sep(),
                MenuItem::new(B_LOCK, "Lock Screen").icon("lock").key("Super+L"),
                MenuItem::new(B_SLEEP, "Sleep").icon("moon"),
                MenuItem::new(B_RESTART, "Restart…").icon("rotate-ccw"),
                MenuItem::new(B_SHUTDOWN, "Shut Down…").icon("power"),
                MenuItem::sep(),
                MenuItem::new(B_LOGOUT, "Log Out…").icon("x").key("Ctrl+Alt+Backspace"),
            ],
            "File" if files => vec![
                MenuItem::new(NEW_FOLDER, "New Folder").icon("folder-plus").key("Ctrl+Shift+N"),
                MenuItem::new(NEW_FILE, "New Text File").icon("file-plus"),
                MenuItem::new(OPEN, "Open").key("Enter").enabled(self.files.has_selection()),
                MenuItem::sep(),
                MenuItem::new(B_CLOSE, "Close Window").key("Alt+F4"),
            ],
            "File" => vec![
                MenuItem::new(B_NEW_FILES, "New Files Window").icon("folder").key("Super+E"),
                MenuItem::new(B_TERMINAL, "New Terminal").icon("terminal"),
                MenuItem::sep(),
                MenuItem::new(B_CLOSE, "Close Window").key("Alt+F4").enabled(self.has_window()),
            ],
            "Edit" => vec![
                MenuItem::new(UNDO, "Undo").icon("undo-2").key("Ctrl+Z").enabled(files && self.fm.can_undo()),
                MenuItem::sep(),
                MenuItem::new(CUT, "Cut").icon("scissors").key("Ctrl+X").enabled(files && self.files.has_selection()),
                MenuItem::new(COPY, "Copy").icon("copy").key("Ctrl+C").enabled(files && self.files.has_selection()),
                MenuItem::new(PASTE, "Paste").icon("clipboard-paste").key("Ctrl+V").enabled(files && self.fm.can_paste()),
                MenuItem::sep(),
                MenuItem::new(SELECT_ALL, "Select All").key("Ctrl+A").enabled(files),
            ],
            "View" if files => self.files.view_menu(),
            "View" => vec![
                MenuItem::new(B_CC, "Quick Settings").icon("sliders-horizontal").checked(self.scene.cc_open()),
                MenuItem::new(B_EDIT, "Edit Controls").icon("pencil"),
            ],
            "Go" => vec![
                MenuItem::new(G_BACK, "Back").icon("arrow-left").key("Alt+←").enabled(self.files.can_back()),
                MenuItem::new(G_FWD, "Forward").icon("arrow-right").key("Alt+→").enabled(self.files.can_forward()),
                MenuItem::new(G_UP, "Enclosing Folder").icon("arrow-up").key("Alt+↑"),
                MenuItem::sep(),
                MenuItem::new(G_HOME, "Home").icon("house"),
                MenuItem::new(G_DESKTOP, "Desktop").icon("monitor"),
                MenuItem::new(G_DOCS, "Documents").icon("file-text"),
                MenuItem::new(G_DOWNLOADS, "Downloads").icon("download"),
            ],
            "Window" => {
                let mut v = vec![
                    MenuItem::new(B_MINIMIZE, "Minimize").icon("minus").key("Super+M").enabled(self.has_window()),
                    MenuItem::new(B_ZOOM, "Zoom").icon("square").key("Super+↑").enabled(self.has_window() && self.focused_shell().is_none()),
                ];
                let mut list = Vec::new();
                if self.settings.open {
                    list.push(MenuItem::new(B_SHELL_SETTINGS, "Settings").icon("settings"));
                }
                if self.files.open {
                    list.push(MenuItem::new(B_SHELL_FILES, "Files").icon("folder"));
                }
                for (i, w) in self.wins.iter().enumerate().filter(|(_, w)| w.mapped && !w.or) {
                    let t = if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() };
                    list.push(MenuItem::new(B_WIN_BASE + i as u32, t).icon("app-window"));
                }
                if !list.is_empty() {
                    v.push(MenuItem::sep());
                    v.extend(list);
                }
                v
            }
            "Help" => vec![MenuItem::new(B_ABOUT, "About Memrium").icon("info"), MenuItem::new(B_SETTINGS, "Settings…").icon("settings")],
            _ => return None,
        };
        Some(m)
    }

    fn open_bar_menu_at(&mut self, hit: BarHit, x: f32, m: Vec<MenuItem>) {
        self.menu_for = MenuFor::Bar(hit);
        let bars = &self.bars;
        let (bottom, side) = self.with_bar_ctx(|ctx| bars.get(hit.bar).map(|b| { let r = b.rect(ctx); (r[1] + r[3], b.cfg.edge.vertical().then_some((r, b.cfg.edge))) })).unwrap_or((self.theme.menubar.height, None));
        // x and the bar are in the monitor's own coordinates
        let o = self.mon_origin(self.bar_mon.get());
        self.bar_menu_mon = self.bar_mon.get();
        // from a bar at the side, beside it (`x` is then the place along the bar, which runs down the screen)
        let at = match side {
            Some((r, memrium_de::theme::Edge::Left)) => [r[0] + r[2] - 2.0 + o[0], x + o[1]],
            Some((r, _)) => [r[0] - 190.0 + o[0], x + o[1]],
            None => [x + o[0], bottom - 2.0 + o[1]],
        };
        self.menu.open_at(at, m);
    }

    fn has_window(&self) -> bool {
        self.focused_shell().is_some() || self.wins.iter().any(|w| w.mapped && !w.minimized && !w.or)
    }

    /// Suspend, restart or shut down the machine through the login manager. Only a real session does it: a nested
    /// window inside another desktop must not be able to power off that desktop's computer.
    pub fn power(&mut self, what: &str) {
        if !self.export_env {
            eprintln!("power: '{what}' ignored (this is a nested window, not a session)");
            return;
        }
        if let Err(e) = std::process::Command::new("systemctl").arg(what).spawn() {
            eprintln!("power: systemctl {what} failed: {e}");
        }
    }

    /// The power menu a start menu's power button opens.
    pub fn open_power_menu(&mut self, at: [f32; 2]) {
        let items = vec![
            MenuItem::new(B_LOCK, "Lock").icon("lock"),
            MenuItem::new(B_SLEEP, "Sleep").icon("moon"),
            MenuItem::new(B_RESTART, "Restart").icon("rotate-ccw"),
            MenuItem::new(B_SHUTDOWN, "Shut down").icon("power"),
            MenuItem::sep(),
            MenuItem::new(B_LOGOUT, "Log out").icon("x"),
        ];
        // reuse the bar menus' command routing; no bar has this index, so nothing is highlighted
        self.menu_for = MenuFor::Bar(BarHit { bar: usize::MAX, widget: 0, part: 0 });
        self.menu.open_at(at, items);
    }

    fn bar_command(&mut self, id: u32) {
        if id >= crate::appmenu_service::APP_BASE {
            self.app_menu_command(id);
            return;
        }
        match id {
            B_ABOUT => {
                self.open_shell(Sw::Settings);
                self.settings.set_page(Settings::page_by_name("about").unwrap_or(0));
            }
            B_SETTINGS => self.open_shell(Sw::Settings),
            B_FILES | B_NEW_FILES => self.open_shell(Sw::Files),
            B_TERMINAL => {
                let home = fsops::home();
                self.open_terminal_in(&home);
            }
            B_LAUNCHER => self.launcher.toggle_as(Some(memrium_de::theme::LauncherKind::Figlet)),
            B_LAUNCHPAD => self.launcher.toggle(),
            B_QUIT | B_LOGOUT => self.quit = true,
            B_LOCK => self.lock_screen(),
            B_SLEEP => self.power("suspend"),
            B_RESTART => self.power("reboot"),
            B_SHUTDOWN => self.power("poweroff"),
            B_CLOSE => match self.focused_shell() {
                Some(Sw::Settings) => self.settings.close(),
                Some(Sw::Files) => self.files.close(),
                None => {
                    if let Some(i) = self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or) {
                        self.close_window(i);
                    }
                }
            },
            B_MINIMIZE => match self.focused_shell() {
                Some(k) => self.shell_win_mut(k).minimized = true,
                None => {
                    if let Some(i) = self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or) {
                        self.minimize(i);
                    }
                }
            },
            B_ZOOM => {
                if let Some(i) = self.wins.iter().rposition(|w| w.mapped && !w.minimized && !w.or) {
                    self.toggle_maximize(i);
                }
            }
            B_CC => {
                let on = !self.scene.cc_open();
                self.scene.set_cc_open(on);
            }
            B_EDIT => self.scene.set_edit(true),
            B_SHELL_SETTINGS => self.open_shell(Sw::Settings),
            B_SHELL_FILES => self.open_shell(Sw::Files),
            n if n >= B_WIN_BASE => {
                let i = (n - B_WIN_BASE) as usize;
                if i < self.wins.len() {
                    self.wins[i].minimized = false;
                    self.focus_window(i);
                }
            }
            other => {
                // Files commands chosen from the Edit / View / Go menus
                let mut ev = Vec::new();
                self.files.command(other, &mut self.fm, &mut ev);
                self.handle_files_events(ev);
            }
        }
    }

    /// Folder that files dropped at `p` would land in.
    fn drop_target_at(&self, p: [f32; 2]) -> Option<std::path::PathBuf> {
        match self.shell_covers(p) {
            Some(Sw::Files) => self.files.drop_target(p),
            Some(_) => None,
            None => (self.desktop.prefs.show && self.desktop_hit(p)).then(|| self.desktop.drop_target(p)),
        }
    }

    fn finish_drag(&mut self, d: FileDrag) {
        self.desktop.drag = None;
        self.files.drop_hover = None;
        self.desktop.drop_hover = None;
        let Some(dir) = self.drop_target_at(self.mouse) else { return };
        let copy = self.key_mods().ctrl;
        // desktop icons let go on the empty desktop are moved on the grid, not in the file system
        if !copy && self.shell_covers(self.mouse).is_none() && dir == *self.desktop.dir() && self.desktop.owns(&d.paths) && self.desktop.drop_move(&d.paths, d.start, self.mouse) {
            return;
        }
        let n = self.fm.drop_into(&d.paths, &dir, copy);
        if n > 0 {
            let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "/".into());
            self.fm.note = format!("{} item{} {} to {}", n, if n == 1 { "" } else { "s" }, if copy { "copied" } else { "moved" }, name);
        }
    }

    /// The desktop's icons as a window-layer item (under every window).
    fn desktop_item(&mut self) -> Option<Item<'static>> {
        let (wp, wl) = self.desk_widget_panels();
        if !self.desktop.prefs.show {
            if wp.is_empty() {
                return None;
            }
            let mut panels = wp;
            panels.extend(self.renderer.label_panels(&wl));
            let w = self.world;
            return Some(Item::Layer { panels, bbox: w });
        }
        let area = self.work_area();
        let time = self.time_ms() as f32 / 1000.0;
        let (mut panels, labels) = {
            let r = &self.renderer;
            let cache = &self.fileicons;
            let measure = |t: &str, s: f32, b: bool| r.text_width(t, s, b);
            let icon = |e: &fsops::Entry, thumb: bool| cache.state(r, e, thumb);
            let ctx = memrium_de::desktop::DesktopCtx { theme: &self.theme, measure: &measure, time, fm: &self.fm, icon: &icon, area };
            self.desktop.draw(&ctx)
        };
        let wanted = std::mem::take(&mut self.desktop.wanted);
        self.fileicons.request(&mut self.renderer, &wanted);
        panels.extend(self.renderer.label_panels(&labels));
        // the widgets sit above the icons; they may be anywhere on the desktop
        let has_widgets = !wp.is_empty();
        panels.extend(wp);
        panels.extend(self.renderer.label_panels(&wl));
        let w = self.world;
        let bbox = if has_widgets { w } else { [area[0], area[1], area[0] + area[2], area[1] + area[3]] };
        Some(Item::Layer { panels, bbox })
    }

    fn menu_command(&mut self, id: u32) {
        use files::cmd::*;
        match self.menu_for {
            MenuFor::Files => {
                let mut ev = Vec::new();
                self.files.command(id, &mut self.fm, &mut ev);
                self.handle_files_events(ev);
            }
            MenuFor::Bar(_) => self.bar_command(id),
            MenuFor::Panel(b, w) => self.panel_menu_command(id, b, w),
            MenuFor::Dock => self.dock_menu_command(id),
            MenuFor::Tray => self.tray_menu_command(id),
            MenuFor::Layout if id == 1000 => {
                self.settings.set_page(Settings::page_by_name("keyboard").unwrap_or(0));
                self.open_shell(Sw::Settings);
            }
            MenuFor::Layout => {
                let kb = self.keyboard.clone();
                let want = id.saturating_sub(1) as _;
                kb.with_xkb_state(self, |mut ctx| ctx.set_layout(smithay::input::keyboard::Layout(want)));
            }
            MenuFor::DesktopIcons => {
                let mut ev = Vec::new();
                self.desktop.command(id, &mut self.fm, &mut ev);
                self.handle_desktop_events(ev);
            }
            MenuFor::Desktop => {
                if self.desk_menu_command(id) {
                    return;
                }
                let desk = fsops::desktop_dir();
                match id {
                    NEW_FOLDER | NEW_FILE | PASTE | UNDO | ICON_SMALL | ICON_MEDIUM | ICON_LARGE | SORT_NAME | SORT_SIZE | SORT_DATE | SORT_KIND | HIDDEN | D_SHOW | D_ARRANGE => {
                        let mut ev = Vec::new();
                        self.desktop.command(id, &mut self.fm, &mut ev);
                        self.handle_desktop_events(ev);
                    }
                    D_EDIT => {
                        let on = !self.scene.edit_mode();
                        self.scene.set_edit(on);
                    }
                    D_WALL => {
                        self.open_shell(Sw::Settings);
                        self.settings.set_page(Settings::page_by_name("wallpaper").unwrap_or(0));
                    }
                    D_DISPLAY => {
                        self.open_shell(Sw::Settings);
                        self.settings.set_page(2);
                    }
                    D_FILES => self.open_shell(Sw::Files),
                    D_DESKTOP_DIR => {
                        self.open_shell(Sw::Files);
                        self.files.goto(desk, true);
                    }
                    D_TERMINAL => self.open_terminal_in(&desk),
                    _ => {}
                }
            }
        }
    }

    /// Wallpaper choices: live ones first, then images found on disk (thumbnails decode in the background).
    fn load_walls(&mut self) {
        if self.walls_loaded {
            return;
        }
        self.walls_loaded = true;
        self.walls = live_choices();
        for (name, path) in live::user_items() {
            self.walls.push(WallItem { name, spec: WallpaperSpec { kind: WallKind::Live, path: path.to_string_lossy().to_string() }, cell: None, colors: Some(([0.20, 0.20, 0.28], [0.55, 0.45, 0.75])) });
        }
        // videos first (they are few and chosen on purpose), then pictures
        let mut found = memrium_de::video::scan();
        found.extend(wallpapers::scan());
        let (tx, rx) = channel();
        for (name, path) in found.iter() {
            let kind = if memrium_de::video::is_video(path) { WallKind::Video } else { WallKind::Image };
            self.walls.push(WallItem { name: name.clone(), spec: WallpaperSpec { kind, path: path.to_string_lossy().to_string() }, cell: None, colors: None });
            self.wall_paths.push(path.clone());
        }
        let base = self.walls.len() - found.len();
        let paths = self.wall_paths.clone();
        std::thread::spawn(move || {
            for (i, p) in paths.iter().enumerate() {
                if let Some(t) = wallpapers::thumbnail(p) {
                    if tx.send((base + i, t)).is_err() {
                        return;
                    }
                }
            }
        });
        self.thumb_rx = Some(rx);
    }

    /// Switch to a new theme: window insets, shell layout and the saved config follow.
    pub fn apply_theme(&mut self, theme: Theme) {
        // light <-> dark spreads out from the pointer (the button that was clicked) as a circle
        let an = theme.animations.clone();
        if theme.dark != self.theme.dark && an.enabled && an.switch != memrium_de::theme::SwitchAnim::None && self.lock.is_none() && !self.capturing && std::env::var_os("MEMRIUM_NO_ANIM").is_none() {
            self.theme_next_anim = an;
            self.ui_sound(memrium_de::sound::Sound::Theme);
            self.start_reveal();
        }
        let icons_changed = theme.icons.shell_theme_name(theme.dark) != self.theme.icons.shell_theme_name(self.theme.dark);
        self.theme = theme;
        // paper is cream by day and charcoal by night
        match self.theme.glass.mode {
            memrium_de::theme::GlassMode::Paper => self.theme.glass.tint = memrium_de::theme::GlassStyle::paper_tint(self.theme.dark),
            memrium_de::theme::GlassMode::Wood => self.theme.glass.tint = memrium_de::theme::GlassStyle::wood_tint(self.theme.dark),
            _ => {}
        }
        let (hide, normal) = (self.theme.window.hide_title_max, decor::insets(&self.theme));
        for w in &mut self.wins {
            w.ins = if w.deco == Deco::Server && w.fs.is_none() && !(hide && w.restore.is_some()) { normal } else { [0.0; 4] };
        }
        self.relayout_maximized();
        self.renderer.set_icon_tile(self.theme.icons.tile_style());
        self.scene.set_theme(&self.theme);
        self.scene.set_menu_light(self.menu_light);
        self.config.set_from(&self.theme);
        self.cfg_dirty = Some(Instant::now());
        memrium_de::theme::export_appearance(&self.theme, &self.config.language);
        self.export_cursor_theme();
        if icons_changed {
            self.reload_icons();
        }
    }

    /// "Auto" appearance: dark from 19:00 to 07:00.
    fn update_auto_dark(&mut self) {
        if !self.theme.auto_dark {
            self.auto_dark_checked = None;
            return;
        }
        if self.auto_dark_checked.is_some_and(|t| t.elapsed().as_secs() < 30) {
            return;
        }
        self.auto_dark_checked = Some(Instant::now());
        let hour = unsafe {
            let now = libc::time(std::ptr::null_mut());
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&now, &mut tm);
            tm.tm_hour
        };
        let dark = !(7..19).contains(&hour);
        if dark != self.theme.dark {
            let mut t = self.theme.clone();
            t.dark = dark;
            self.apply_theme(t);
        }
    }

    /// Play an interface sound, if the theme wants them.
    pub fn ui_sound(&self, s: memrium_de::sound::Sound) {
        let a = &self.theme.animations;
        if a.sounds && std::env::var_os("MEMRIUM_NO_SOUND").is_none() {
            memrium_de::sound::play(s, a.sound_volume);
        }
    }

    /// Keep the current picture and wipe it away with a circle growing from the pointer.
    fn start_reveal(&mut self) {
        let (pw, ph) = self.renderer.pixel_size();
        let device = self.renderer.device().clone();
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reveal old frame"),
            size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.renderer.format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let (skip, notice) = (self.allow_skip, self.shot_notice.take());
        self.allow_skip = false;
        self.renderer.end_reveal();
        let enc = self.render(&view);
        self.allow_skip = skip;
        self.shot_notice = notice;
        self.renderer.queue().submit([enc.finish()]);
        let k = pw as f32 / self.renderer.logical_size()[0];
        let off = self.canvas_off();
        let c = [(self.mouse[0] + off[0]) * k, (self.mouse[1] + off[1]) * k];
        let an = &self.theme_next_anim;
        self.renderer.begin_reveal(&tex, c, an.switch == memrium_de::theme::SwitchAnim::Fade, an.rim);
        self.reveal = Some((Instant::now(), c));
        self.invalidate();
    }

    /// Grow the circle; called every tick.
    fn update_reveal(&mut self) {
        let Some((t0, c)) = self.reveal else { return };
        let (pw, ph) = self.renderer.pixel_size();
        let dur = std::env::var("MEMRIUM_REVEAL_MS").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(self.theme_next_anim.switch_ms).max(50.0) / 1000.0;
        let t = t0.elapsed().as_secs_f32() / dur;
        if t >= 1.0 {
            self.reveal = None;
            self.renderer.end_reveal();
            self.invalidate();
            return;
        }
        // reach the farthest corner
        let far = [(0.0, 0.0), (pw as f32, 0.0), (0.0, ph as f32), (pw as f32, ph as f32)].iter().map(|p: &(f32, f32)| (p.0 - c[0]).hypot(p.1 - c[1])).fold(0.0, f32::max);
        let e = 1.0 - (1.0 - t).powi(3);
        let fade = self.theme_next_anim.switch == memrium_de::theme::SwitchAnim::Fade;
        self.renderer.set_reveal_radius(if fade { t } else { e * (far + 40.0) });
    }

    /// Apply the theme's wallpaper at startup (the default image is already loaded by the renderer).
    pub fn init_wallpaper(&mut self) {
        self.wall_ready = true;
        self.renderer.set_icon_tile(self.theme.icons.tile_style());
        let spec = self.theme.wallpaper.clone();
        // the default picture is already in the renderer, but loading it again is how the accent colour is taken from it
        self.apply_wallpaper(&spec);
    }

    pub fn apply_wallpaper(&mut self, spec: &WallpaperSpec) {
        match spec.kind {
            WallKind::Image => {
                self.stop_media_wallpaper();
                let _ = self.renderer.set_live_wallpaper(None);
                let path = if spec.path.is_empty() { self.default_wallpaper.clone() } else { Some(std::path::PathBuf::from(&spec.path)) };
                match path {
                    Some(p) => {
                        let (tx, rx) = channel();
                        std::thread::spawn(move || {
                            if let Some(img) = wallpapers::load(&p, 3840) {
                                let _ = tx.send(img);
                            }
                        });
                        self.wall_rx = Some(rx);
                    }
                    None => self.renderer.set_wallpaper(None),
                }
            }
            WallKind::Live => {
                self.wall_rx = None;
                self.video = None;
                self.renderer.end_video();
                self.menu_light = matches!(spec.path.as_str(), "aurora" | "starfield" | "spectrum" | "pulse");
                self.scene.set_menu_light(self.menu_light);
                let result = live::source_for(&spec.path).and_then(|src| self.renderer.set_live_wallpaper(Some(&src)));
                if let Err(e) = result {
                    eprintln!("live wallpaper `{}` rejected: {e}", spec.path);
                    let _ = self.renderer.set_live_wallpaper(None);
                }
                // the sound is only listened to while the wallpaper wants it
                self.audio_tap = if self.renderer.live_uses_audio() { memrium_de::audiotap::AudioTap::start() } else { None };
            }
            WallKind::Video => {
                self.wall_rx = None;
                self.audio_tap = None;
                let (pw, ph) = self.renderer.pixel_size();
                let (w, h) = memrium_de::video::decode_size(pw, ph);
                match memrium_de::video::VideoWall::start(std::path::Path::new(&spec.path), w, h) {
                    Ok(v) => {
                        self.video = Some(v);
                        self.video_first = true;
                        let _ = self.renderer.set_live_wallpaper(None);
                    }
                    Err(e) => eprintln!("video wallpaper `{}`: {e}; keeping the current wallpaper", spec.path),
                }
            }
        }
    }

    /// A still or shader wallpaper takes over: the decoder and the listener stop.
    fn stop_media_wallpaper(&mut self) {
        self.video = None;
        self.audio_tap = None;
        self.renderer.end_video();
    }

    /// Per frame: hand the newest video frame and the current spectrum to the renderer, and keep the decoder paused while
    /// nobody can see the wallpaper.
    fn tick_wallpaper(&mut self) {
        if let Some(v) = &mut self.video {
            if v.is_dead() {
                eprintln!("video wallpaper `{}` stopped (is it a video ffmpeg can read?)", v.path.display());
                self.video = None;
                self.renderer.end_video();
            }
        }
        if let Some(v) = &self.video {
            v.set_paused(self.screen_off || self.fullscreen_on_top());
            if let Some(f) = v.take_frame() {
                if self.video_first {
                    // the first frame decides the accent and whether the menu bar text is light
                    self.video_first = false;
                    if let Some(img) = image::RgbaImage::from_raw(f.w, f.h, f.rgba.clone()) {
                        self.menu_light = wallpapers::is_dark_top(&img);
                        self.wall_accent = Some(wallpapers::accent_of(&img));
                        self.scene.set_menu_light(self.menu_light);
                    }
                }
                self.renderer.set_video_frame(&f.rgba, f.w, f.h);
            }
        }
        if let Some(t) = &self.audio_tap {
            let s = t.spectrum();
            self.renderer.set_audio(&s.bands, s.levels);
        }
    }

    fn process_settings(&mut self) {
        self.tick_wallpaper();
        if let Some(rx) = &self.thumb_rx {
            while let Ok((i, img)) = rx.try_recv() {
                let key = format!("wall:{}", self.wall_paths.get(i.saturating_sub(self.walls.len() - self.wall_paths.len())).map(|p| p.to_string_lossy().to_string()).unwrap_or_default());
                if let Some(cell) = self.renderer.image_cell(&key, &img) {
                    if let Some(w) = self.walls.get_mut(i) {
                        w.cell = Some(cell);
                    }
                }
            }
        }
        if let Some(rx) = &self.wall_rx {
            if let Ok(img) = rx.try_recv() {
                self.menu_light = wallpapers::is_dark_top(&img);
                self.wall_accent = Some(wallpapers::accent_of(&img));
                self.scene.set_menu_light(self.menu_light);
                let fitted = self.fit_to_monitors(img);
                self.renderer.set_wallpaper(Some(&fitted));
                self.wall_rx = None;
            }
        }
        if let Some(theme) = self.pending_theme.take() {
            self.apply_theme(theme);
        }
        for a in std::mem::take(&mut self.pending_actions) {
            match a {
                SettingsAction::ThemeChanged => {}
                SettingsAction::Close => self.settings.close(),
                SettingsAction::Sys(cmd) => self.sys_watcher.send(cmd),
                SettingsAction::TestNotification => self.test_notification(),
                SettingsAction::Preset(id) => {
                    // a clean template switch, but keep the user's wallpaper
                    let wallpaper = self.theme.wallpaper.clone();
                    self.config.select(&id);
                    let mut t = self.config.resolve();
                    t.wallpaper = if t.wallpaper.path.is_empty() { wallpaper } else { t.wallpaper };
                    // a template brings its own file manager look (Aero: Explorer)
                    if let Some(name) = t.files.clone() {
                        self.files.prefs = files::FilesPrefs::preset(&name);
                    }
                    self.apply_theme(t);
                }
                SettingsAction::SetLanguage(code) => {
                    self.config.language = code.clone();
                    memrium_de::i18n::set_language(&code);
                    memrium_de::theme::export_appearance(&self.theme, &self.config.language);
                    self.config.save();
                    self.invalidate();
                }
                SettingsAction::ApplyDisplays => {
                    self.config.save();
                    self.displays_dirty = true;
                }
                SettingsAction::SaveTemplate(name) => {
                    self.shot_notice = Some(match memrium_de::theme::save_template(&self.theme, &name) {
                        Ok(p) => {
                            self.presets = memrium_de::theme::available_themes();
                            (format!("Saved template {}", p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()), 3.0)
                        }
                        Err(e) => (format!("Could not save: {e}"), 4.0),
                    });
                }
                SettingsAction::ImportTemplate(path) => {
                    let path = if let Some(rest) = path.strip_prefix("~/") { fsops::home().join(rest) } else { std::path::PathBuf::from(&path) };
                    self.shot_notice = Some(match memrium_de::theme::import_template(&path) {
                        Ok(name) => {
                            self.presets = memrium_de::theme::available_themes();
                            (format!("Imported {name}"), 3.0)
                        }
                        Err(e) => (e, 4.0),
                    });
                }
                SettingsAction::Wallpaper(spec) => {
                    let mut t = self.theme.clone();
                    t.wallpaper = spec.clone();
                    self.apply_theme(t);
                    self.apply_wallpaper(&spec);
                }
            }
        }
        // save shortly after the last change (not on every slider tick)
        if let Some(t) = self.cfg_dirty {
            if t.elapsed().as_millis() > 800 {
                self.config.save();
                self.cfg_dirty = None;
            }
        }
    }

    /// A shell window as a window-layer item (frame + title + content), when it is shown.
    fn shell_item(&mut self, k: Sw) -> Vec<Item<'static>> {
        let logical = self.sw_visible(k);
        let kk = self.shell_win(k).k.x;
        // a window of another desktop is beside the one on show, as for client windows
        let dx = desk_dx(self.shell_win(k).ws, self.ws_pos.x, self.size[0]);
        if (!logical && kk < 0.01) || dx.abs() >= self.size[0] {
            return Vec::new();
        }
        let frame = self.sw_frame(k);
        let panels = if logical {
            let p = self.shell_content(k);
            self.shell_win_mut(k).last = p.clone();
            p
        } else {
            self.shell_win(k).last.clone()
        };
        // pop in / out; a minimised window is sucked into its Dock icon (genie)
        let c = [frame[0] + frame[2] / 2.0, frame[1] + frame[3] / 2.0];
        let minimizing = self.shell_win(k).minimized;
        let mut out = Vec::new();
        let fade;
        let xf;
        if minimizing || (logical && kk < 0.999 && self.shell_win(k).genie) {
            let dock = self.item_center(if k == Sw::Settings { apps::SETTINGS_ID } else { apps::FILES_ID }).unwrap_or([self.size[0] / 2.0, self.size[1] - 40.0]);
            out.push(Item::Warp(Some(Warp { rect: frame, t: (1.0 - kk).clamp(0.0, 1.0), dock, icon_w: 52.0, jelly: None })));
            xf = Xf { c, s: 1.0, t: [dx, 0.0], fade: 1.0 };
            fade = 1.0;
        } else {
            let o = kk.clamp(0.0, 1.08);
            xf = Xf { c, s: 0.9 + 0.1 * o, t: [dx, 0.0], fade: (kk * 1.7).clamp(0.0, 1.0) };
            fade = xf.fade;
        }
        let jel = [self.shell_win(k).jel[0].x, self.shell_win(k).jel[1].x];
        if out.is_empty() && jel[0].abs() + jel[1].abs() > 0.4 {
            out.push(Item::Warp(Some(jelly_warp(frame, jel, self.shell_win(k).jgrab))));
        }
        let panels: Vec<Panel> = if kk > 0.999 && xf.s > 0.999 && dx == 0.0 { panels } else { panels.into_iter().map(|p| xf.panel(p)).collect() };
        let bb = xf.rect([frame[0] - 90.0, frame[1] - 90.0, frame[2] + 180.0, frame[3] + 200.0]);
        out.push(Item::Frame { panels, bbox: [bb[0], bb[1], bb[0] + bb[2], bb[1] + bb[3]], fade });
        if matches!(out.first(), Some(Item::Warp(_))) {
            out.push(Item::Warp(None));
        }
        out
    }

    /// Frame, title and content panels of a shown shell window.
    fn shell_content(&mut self, k: Sw) -> Vec<Panel> {
        let frame = self.sw_frame(k);
        let ins = self.sw_ins(k);
        let frame_theme = self.sw_theme(k).into_owned();
        let focused = self.focused_shell() == Some(k);
        let hover = self.shell_win(k).hover;
        let mut panels = decor::frame_panels(&frame_theme, frame, focused, self.shell_win(k).restore.is_some(), hover);
        // the wallpaper's colour seeps into the window frame
        if let (true, Some(w)) = (self.theme.ui.wall_tint, self.wall_accent) {
            if let Some(p) = panels.first_mut().filter(|p| p.params[1] == 1.0) {
                let amount = if self.theme.dark { 0.14 } else { 0.10 };
                for q in [&mut p.tint, &mut p.color2] {
                    for i in 0..3 {
                        q[i] = q[i] * (1.0 - amount) + w[i] * amount;
                    }
                }
            }
        }
        let origin = [frame[0] + ins[0], frame[1] + ins[1]];
        match k {
            Sw::Settings => {
                let title = decor::title_labels(&frame_theme, frame, "Settings", focused);
                panels.extend(self.renderer.label_panels(&title));
                let mut theme = self.theme.clone();
                let mut cfg = self.config.clone();
                // a few of the apps in the Dock, for the Icons page's preview
                let icon_cells: Vec<(u32, String)> = self.pinned_indices().into_iter().filter_map(|i| Some((self.renderer.app_icon_get(&self.apps[i].id)?, self.apps[i].name.clone()))).take(8).collect();
                let mut want_icons = Vec::new();
                let index = self.icons.get();
                let icon_names: &[String] = index.as_deref().map_or(&[], |i| i.names());
                let (cp, cl, actions) = {
                    let renderer = &self.renderer;
                    let app_icon = |id: &str| renderer.app_icon_get(id);
                    let mut ctx = SettingsCtx { screen: self.size, theme: &mut theme, presets: &self.presets, preset_id: &self.config.theme, walls: &self.walls, language: &self.config.language, wall_accent: self.wall_accent, cfg: &mut cfg, monitors: &self.monitor_infos, size: self.client_size(Sw::Settings), icons: &icon_cells, sys: &self.sys, app_icon: &app_icon, want_icons: &mut want_icons, icon_names };
                    self.settings.draw(&mut ctx, origin)
                };
                // the icons the Programs page lacked start loading (a few a frame)
                let mut budget = 4;
                want_icons.sort();
                want_icons.dedup();
                for id in want_icons {
                    if let Some((pack, icon)) = id.strip_prefix("pack:").and_then(|r| r.rsplit_once(':')) {
                        // a sample of an icon pack, for the Icons page
                        if budget > 0 && self.renderer.app_icon_get(&id).is_none() && !self.icon_failed.contains(&id) {
                            budget -= 1;
                            let tile = self.renderer.icon_tile();
                            self.icon_loader.request_pack(&id, pack, icon, tile);
                        }
                    } else if let Some(name) = id.strip_prefix("icon:") {
                        // an icon of the pack in use, for a widget's icon picker
                        if budget > 0 && self.renderer.app_icon_get(&id).is_none() && !self.icon_failed.contains(&id) {
                            budget -= 1;
                            let tile = self.renderer.icon_tile();
                            self.icon_loader.request(&id, name, tile);
                        }
                    } else if let Some(i) = self.apps.iter().position(|a| a.id == id) {
                        self.ensure_icon(i, &mut budget);
                    }
                }
                // the pages edit a copy of the config; keep what changed
                let tidy = |k: &mut memrium_de::keys::KeyBinds| k.commands.retain(|c| !(c.keys.trim().is_empty() && c.run.trim().is_empty()));
                tidy(&mut cfg.keys);
                let mut had = self.config.keys.clone();
                tidy(&mut had);
                if cfg.screenshots != self.config.screenshots || cfg.power != self.config.power || cfg.keys != had || cfg.weather_city != self.config.weather_city || cfg.display != self.config.display || cfg.input != self.config.input || cfg.notifications != self.config.notifications || cfg.desktop != self.config.desktop {
                    self.desktop.prefs = cfg.desktop.clone();
                    self.config.desktop = cfg.desktop;
                    self.config.screenshots = cfg.screenshots;
                    self.config.power = cfg.power;
                    memrium_de::keys::set_style(&cfg.keys.style);
                    self.config.keys = cfg.keys;
                    self.config.weather_city = cfg.weather_city;
                    self.config.notifications = cfg.notifications;
                    self.config.display = cfg.display;
                    // pointer speed and the like reach the devices on the next turn of the session's loop
                    self.input_dirty = cfg.input != self.config.input;
                    let keymap_was = memrium_de::input::xkb_settings(&self.config.input);
                    self.config.input = cfg.input;
                    if memrium_de::input::xkb_settings(&self.config.input) != keymap_was {
                        self.apply_keymap();
                    }
                    self.cfg_dirty = Some(Instant::now());
                }
                panels.extend(cp);
                panels.extend(self.renderer.label_panels(&cl));
                if theme != self.theme {
                    self.pending_theme = Some(theme);
                }
                self.pending_actions.extend(actions);
            }
            Sw::Files => {
                let name = if fsops::is_computer(&self.files.cwd) { "Computer".to_string() } else { self.files.cwd.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "/".into()) };
                let title = decor::title_labels(&frame_theme, frame, &format!("Files — {name}"), focused);
                panels.extend(self.renderer.label_panels(&title));
                let time = self.time_ms() as f32 / 1000.0;
                let files_size = self.client_size(Sw::Files);
                let (cp, cl) = {
                    let r = &self.renderer;
                    let measure = |t: &str, s: f32, b: bool| r.text_width(t, s, b);
                    let cache = &self.fileicons;
                    let icon = |e: &fsops::Entry, thumb: bool| cache.state(r, e, thumb);
                    let ctx = FilesCtx { theme: &self.theme, size: files_size, measure: &measure, time, fm: &self.fm, icon: &icon };
                    self.files.draw(&ctx, origin)
                };
                let wanted = std::mem::take(&mut self.files.wanted);
                self.fileicons.request(&mut self.renderer, &wanted);
                panels.extend(cp);
                panels.extend(self.renderer.label_panels(&cl));
            }
        }
        panels
    }

    /// Keep a closing client window's last frame on screen so it can fade out.
    pub fn make_ghost(&mut self, i: usize) {
        if !self.theme.animations.on("window_close") {
            return;
        }
        let w = &self.wins[i];
        if !w.mapped || w.min_k.x > 0.3 {
            return;
        }
        let f = Self::frame_rect(w);
        let maximized = w.restore.is_some();
        let (deco, title) = (w.deco, w.title.clone());
        let Some(root) = w.role.surface() else { return };
        let origin = [f[0] + w.ins[0] - w.geo[0], f[1] + w.ins[1] - w.geo[1]];
        let ins = w.ins;
        let mut panels: Vec<Panel> = if deco == Deco::Server {
            let th = w.chrome_theme(&self.theme);
            let mut p = decor::frame_panels(&th, f, false, maximized, None);
            let (x0, x1) = decor::title_span(&th, f);
            let t = ellipsize(&self.renderer, &title, 13.0, (x1 - x0).max(40.0));
            let labels = decor::title_labels(&th, f, &t, false);
            p.extend(self.renderer.label_panels(&labels));
            p
        } else {
            let sh = self.theme.window.shadow_idle;
            let r = if deco == Deco::Client { self.theme.window.radius.min(18.0) } else { 0.0 };
            vec![Panel::solid(f[0] + f[2] / 2.0, f[1] + f[3] / 2.0, f[2], f[3], r, [0.97; 3], [0.95; 3]).flat().alpha(0.0).shadow(sh[0], sh[1], sh[2])]
        };
        let clip = match (deco, self.theme.window.decoration) {
            (Deco::Legacy, _) => None,
            (Deco::Server, _) if decor::glass_frame(&self.theme) => Some(([f[0] + ins[0], f[1] + ins[1], f[2] - ins[0] - ins[2], f[3] - ins[1] - ins[3]], 0.0)),
            (Deco::Server, _) => Some((f, if maximized && self.theme.window.max_gap < 0.5 { 0.0 } else { self.theme.window.radius })),
            (Deco::Client, _) => Some((f, self.theme.window.radius.min(18.0))),
        };
        let mut surf = None;
        if let Some(n) = self.surfaces.get_mut(&root.id()) {
            if let Some(tex) = n.tex.clone() {
                let sz = n.logical_size().unwrap_or([0.0; 2]);
                let opaque = tex.opaque;
                surf = Some((tex, [origin[0], origin[1], sz[0], sz[1]], clip, !opaque));
            }
        }
        panels.shrink_to_fit();
        // Spring::new starts with target == value, so aim it at 0 or the ghost never fades away
        let mut k = Spring::new(1.0, 260.0, 0.95);
        k.target = 0.0;
        self.ghosts.push(Ghost { panels, frame: f, surf, k });
    }

    // ------------------------------------------------------------ per-frame

    pub fn update(&mut self, dt: f32) {
        // a shell window gaining or losing the front (raised, closed, minimized) hands the keyboard over
        if self.focused_shell() != self.shell_focus_seen {
            self.apply_focus();
        }
        self.follow_gtk();
        self.sync_app_style();
        // give free memory back to the system now and then (a wallpaper or an icon scan leaves a lot behind); only after
        // something was drawn since the last time: an idle desktop allocates nothing, and the trim walks the whole heap
        static LAST_TRIM: std::sync::Mutex<Option<(Instant, u64)>> = std::sync::Mutex::new(None);
        if let Ok(mut last) = LAST_TRIM.lock() {
            let due = last.map_or(true, |(t, drawn)| t.elapsed() > std::time::Duration::from_secs(4) && drawn != self.frames_drawn);
            if due && self.start.elapsed() > std::time::Duration::from_secs(3) {
                chebrium::mem::trim();
                *last = Some((Instant::now(), self.frames_drawn));
            }
        }
        // every spring runs on the theme's clock
        let raw_dt = dt;
        let an = self.theme.animations.clone();
        let dt = an.scale_dt(dt);
        self.shot_flash = (self.shot_flash - dt * 4.0).max(0.0);
        self.snap += ((self.snap_on as u8 as f32) - self.snap) * (1.0 - (-16.0 * dt).exp());
        if self.snap < 0.003 && !self.snap_on {
            self.snap = 0.0;
        }
        if let Some(n) = &mut self.shot_notice {
            n.1 -= dt;
            if n.1 <= 0.0 {
                self.shot_notice = None;
            }
        }
        if self.shot_hold.as_ref().is_some_and(|h| !h.fired && h.at.elapsed().as_millis() >= 500) {
            if let Some(h) = self.shot_hold.as_mut() {
                h.fired = true;
            }
            self.shot_req = Some(ShotReq::Full);
        }
        if let Some(req) = self.shot_req.take() {
            self.take_screenshot(req);
        }
        self.record_tick();
        self.sync_dock();
        self.scene.set_animation(&an);
        self.scene.update(dt);
        self.update_bars(dt);
        self.update_dock_preview(dt);
        self.update_tooltip(dt);
        self.tick_cursor();
        self.process_shell_events();
        let kind = self.launcher.kind(self.theme.launcher.style);
        self.launcher.figlet = kind == memrium_de::theme::LauncherKind::Figlet;
        self.launcher.grid = kind == memrium_de::theme::LauncherKind::Launchpad;
        self.launcher.prepare(&self.apps);
        self.launcher.update(dt, &an);
        self.menu.update(dt, &an);
        self.launcher.refresh_if_indexed(&self.apps);
        for k in [Sw::Settings, Sw::Files] {
            // a window of another desktop stays "shown": it slides away with the row instead of closing
            let shown = self.is_open(k) && !self.shell_win(k).minimized;
            let w = self.shell_win_mut(k);
            if w.minimized {
                w.genie = true;
            }
            w.k.target = if shown { 1.0 } else { 0.0 };
            an.step("window_move", &mut w.x, dt);
            an.step("window_move", &mut w.y, dt);
            an.step(if shown { "window_open" } else if w.genie { "window_minimize" } else { "window_close" }, &mut w.k, dt);
            Self::jelly_step(&an, &mut w.jel, &mut w.jprev, [w.x.x, w.y.x], raw_dt);
            if !shown && w.k.x < 0.01 {
                w.last.clear();
            }
        }
        for g in &mut self.ghosts {
            g.k.update(dt);
        }
        self.ghosts.retain(|g| g.k.x > 0.02);
        self.process_settings();
        if self.ws_drag.is_none() {
            an.step("workspace_slide", &mut self.ws_pos, dt);
        }
        for w in &mut self.wins {
            an.step("window_move", &mut w.x, dt);
            an.step("window_move", &mut w.y, dt);
            an.step("window_focus", &mut w.scale, dt);
            an.step("window_minimize", &mut w.min_k, dt);
            an.step("window_open", &mut w.open_k, dt);
            Self::jelly_step(&an, &mut w.jel, &mut w.jprev, [w.x.x, w.y.x], raw_dt);
        }
        self.sync_x11_positions();
        self.update_lock(dt);
        self.sys_watcher.set_watching(self.scene.cc_open() || self.flyout.is_some() || (self.settings.open && !self.swin.minimized));
        self.sys = self.sys_watcher.info();
        self.scene.cc_set_sys(&self.sys);
        // the Quick Settings's Now Playing tile follows the media player (watched only once it has been opened)
        if self.scene.cc_open() && self.media.is_none() {
            self.media = Some(crate::media::Media::start());
        }
        if let Some(m) = &self.media {
            let i = m.get();
            self.scene.cc_set_media(&memrium_de::controlcenter::CcMedia { title: i.title, artist: i.artist, playing: i.playing, present: i.present, cover: if i.art.is_empty() { None } else { self.album.cell }, tone: self.album.color });
        }
        self.scene.cc_set_layout(&self.config.cc_tiles);
        self.scene.cc_set_record_angle(self.album.angle);
        self.update_toasts(dt);
        self.clipboard_tick();
        self.update_reveal();
        self.update_idle();
        self.update_auto_dark();
        self.update_widgets(dt);
        self.update_vinyl(dt);
        self.update_visualizer();
        self.update_flyout(dt);
        self.update_card(dt);
        self.tray_update();
        self.appmenu_update();
        self.pump_icons();
        self.serve_screencopy();
        self.sync_toplevels();
        self.serve_ext_capture();
        for p in &mut self.popups {
            p.k.target = 1.0;
            an.step("menu_open", &mut p.k, dt);
        }
        self.drop_popups(|p| !p.surface.alive());
        if self.rules_checked.elapsed().as_millis() > 700 {
            self.rules_checked = Instant::now();
            self.rules.reload_if_changed();
        }
        let gone: Vec<_> = self.surfaces.iter().filter(|(_, n)| !n.surface.is_alive()).map(|(k, _)| k.clone()).collect();
        for k in gone {
            if let Some(h) = self.surfaces.remove(&k).and_then(|n| n.dma_buf) {
                self.retire(h);
            }
        }
    }

    /// Take away the popups `gone` picks. Each leaves a ghost that shrinks and fades, so a menu does not just vanish.
    pub(crate) fn drop_popups(&mut self, gone: impl Fn(&Popup) -> bool) {
        let mut i = 0;
        while i < self.popups.len() {
            if gone(&self.popups[i]) {
                if self.theme.animations.on("menu_open") {
                    self.popup_ghost(i);
                }
                self.popups.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// The picture of popup `i` as it was, to fade away after the popup is gone.
    fn popup_ghost(&mut self, i: usize) -> Option<()> {
        let p = &self.popups[i];
        if !p.surface.alive() || p.k.x < 0.3 {
            return None;
        }
        let root = p.surface.wl_surface();
        let parent = p.surface.get_parent_surface()?;
        let go = popup_geometry_origin(&self.wins, &self.popups, &root_of(&parent), p)?;
        let gl = with_states(root, |st| st.cached_state.get::<SurfaceCachedState>().current().geometry).map(|g| [g.loc.x as f32, g.loc.y as f32]).unwrap_or([0.0, 0.0]);
        let n = self.surfaces.get(&root.id())?;
        let tex = n.tex.clone()?;
        let sz = n.logical_size()?;
        let r = [go[0] - gl[0], go[1] - gl[1], sz[0], sz[1]];
        let panels = vec![Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], 10.0, [1.0; 3], [1.0; 3]).flat().alpha(0.0).shadow(0.30, 24.0, 8.0)];
        // Spring::new starts with target == value, so aim it at 0 or the ghost never fades away
        let mut k = Spring::new(1.0, 520.0, 0.95);
        k.target = 0.0;
        let frosted = !tex.opaque;
        self.ghosts.push(Ghost { panels, frame: r, surf: Some((tex, r, Some((r, 10.0)), frosted)), k });
        Some(())
    }

    /// The jelly wobble of a moving window: its far parts lag behind its motion, then swing back.
    fn jelly_step(an: &memrium_de::theme::AnimStyle, jel: &mut [Spring; 2], prev: &mut [f32; 2], pos: [f32; 2], raw_dt: f32) {
        if !an.on("window_jelly") {
            for j in jel.iter_mut() {
                j.target = 0.0;
                j.snap();
            }
            *prev = pos;
            return;
        }
        let dt = raw_dt.max(1e-3);
        for i in 0..2 {
            let v = (pos[i] - prev[i]) / dt;
            jel[i].target = jelly_target(v);
            jel[i].update(an.dt("window_jelly", raw_dt));
        }
        *prev = pos;
    }

    /// Lay the desktop out over `placed` monitors (device px, the main one at 0, 0).
    pub fn apply_layout(&mut self, placed: &[memrium_de::display::Placed], scale: f32) {
        use memrium_de::display::bounds;
        let (x0, y0, x1, y1) = bounds(placed);
        let (cw, ch) = ((x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32);
        self.renderer.resize(cw, ch, scale);
        crate::role::set_scale_grid(scale);
        crate::role::set_x_scale(scale);
        self.set_x_dpi();
        self.send_preferred_scale_to_all();
        let k = 1.0 / scale;
        self.world = [x0 as f32 * k, y0 as f32 * k, x1 as f32 * k, y1 as f32 * k];
        self.mons = placed.iter().map(|p| [p.x as f32 * k, p.y as f32 * k, p.w as f32 * k, p.h as f32 * k]).collect();
        self.main_mon = placed.iter().position(|p| p.primary).unwrap_or(0);
        let [_, _, mw, mh] = self.mons[self.main_mon];
        self.size = [mw, mh];
        let edit = self.scene.edit_mode();
        let cc_open = self.scene.cc_open();
        self.scene = Scene::shell_only_with(mw, mh, &self.theme);
        self.scene.set_cc_open(cc_open);
        self.scene.set_edit(edit);
        self.scene.set_menu_light(self.menu_light);
        self.invalidate();
        // the wallpaper is fitted to the monitors: do it again for the new layout
        if self.wall_ready {
            let spec = self.theme.wallpaper.clone();
            self.apply_wallpaper(&spec);
        }
    }

    /// With several monitors the wallpaper is fitted to each of them (cover-fit), not stretched across all.
    fn fit_to_monitors(&self, img: image::RgbaImage) -> image::RgbaImage {
        use image::imageops::{self, FilterType};
        if self.mons.len() < 2 {
            return img;
        }
        let (cw, ch) = self.renderer.pixel_size();
        let k = cw as f32 / self.renderer.logical_size()[0];
        let mut out = image::RgbaImage::new(cw, ch);
        let (iw, ih) = img.dimensions();
        for m in &self.mons {
            let (x, y) = (((m[0] - self.world[0]) * k).round() as i64, ((m[1] - self.world[1]) * k).round() as i64);
            let (w, h) = ((m[2] * k).round().max(1.0) as u32, (m[3] * k).round().max(1.0) as u32);
            // the part of the picture that covers this monitor's shape, centred
            let s = (w as f32 / iw as f32).max(h as f32 / ih as f32);
            let (sw, sh) = (((w as f32 / s).round() as u32).clamp(1, iw), ((h as f32 / s).round() as u32).clamp(1, ih));
            let crop = imageops::crop_imm(&img, (iw - sw) / 2, (ih - sh) / 2, sw, sh).to_image();
            imageops::replace(&mut out, &imageops::resize(&crop, w, h, FilterType::Triangle), x, y);
        }
        out
    }

    /// Where the renderer's canvas starts, in desktop coordinates (so a desktop point + this = a canvas point).
    pub fn canvas_off(&self) -> [f32; 2] {
        [-self.world[0], -self.world[1]]
    }

    /// Keep a pointer position on some monitor.
    pub fn clamp_to_monitors(&self, p: [f32; 2]) -> [f32; 2] {
        let mut best = p;
        let mut best_d = f32::MAX;
        for m in &self.mons {
            let c = [p[0].clamp(m[0], m[0] + m[2] - 1.0), p[1].clamp(m[1], m[1] + m[3] - 1.0)];
            let d = (c[0] - p[0]).hypot(c[1] - p[1]);
            if d < best_d {
                best_d = d;
                best = c;
            }
        }
        best
    }

    /// What a maximized window fills on the monitor under `p`: the work area less the gap the theme leaves around it.
    pub fn maximize_area(&self, p: [f32; 2]) -> [f32; 4] {
        let a = self.work_area_at(p);
        let g = self.theme.window.max_gap.clamp(0.0, 120.0);
        [a[0] + g, a[1] + g, (a[2] - 2.0 * g).max(200.0), (a[3] - 2.0 * g).max(150.0)]
    }

    /// A theme change may have changed the gap: maximized windows take their new place.
    fn relayout_maximized(&mut self) {
        for i in 0..self.wins.len() {
            if self.wins[i].restore.is_some() && self.wins[i].fs.is_none() && self.wins[i].mapped {
                let f = Self::frame_rect(&self.wins[i]);
                let area = self.snap_rect([f[0] + f[2] / 2.0, f[1] + f[3] / 2.0], self.wins[i].tile);
                let w = &mut self.wins[i];
                let (ix, iy) = (w.ins[0] + w.ins[2], w.ins[1] + w.ins[3]);
                w.x.target = area[0];
                w.y.target = area[1];
                w.role.configure(Some((((area[2] - ix) as i32).max(1), ((area[3] - iy) as i32).max(1))), ((area[0] + w.ins[0]) as i32, (area[1] + w.ins[1]) as i32), Some(true), None);
            }
        }
        for k in [Sw::Settings, Sw::Files] {
            if self.shell_win(k).restore.is_some() {
                let area = self.snap_rect([self.size[0] / 2.0, self.size[1] / 2.0], self.shell_win(k).tile);
                let ins = self.sw_ins(k);
                let w = self.shell_win_mut(k);
                w.size = Some([area[2] - ins[0] - ins[2], area[3] - ins[1] - ins[3]]);
                w.x.target = area[0];
                w.y.target = area[1];
            }
        }
    }

    /// The area a maximised window gets on the monitor under `p`: the main one keeps its bars out of the way.
    pub fn work_area_at(&self, p: [f32; 2]) -> [f32; 4] {
        match self.mons.iter().position(|m| p[0] >= m[0] && p[0] < m[0] + m[2] && p[1] >= m[1] && p[1] < m[1] + m[3]) {
            Some(i) if i != self.main_mon => self.mons[i],
            _ => self.work_area(),
        }
    }

    pub fn resize_output(&mut self, w_px: u32, h_px: u32, scale: f32) {
        self.renderer.resize(w_px, h_px, scale);
        crate::role::set_scale_grid(scale);
        crate::role::set_x_scale(scale);
        self.set_x_dpi();
        self.send_preferred_scale_to_all();
        let [lw, lh] = self.renderer.logical_size();
        self.size = [lw, lh];
        let edit = self.scene.edit_mode();
        let cc_open = self.scene.cc_open();
        self.scene = Scene::shell_only_with(lw, lh, &self.theme);
        self.scene.set_cc_open(cc_open);
        self.scene.set_edit(edit);
        self.scene.set_menu_light(self.menu_light);
    }

    /// Draw one frame into `target`.
    pub fn render(&mut self, target: &wgpu::TextureView) -> wgpu::CommandEncoder {
        match self.render_lazy(|| Some(target.clone())) {
            Some(enc) => enc,
            // identical to the last frame: nothing to draw
            None => self.renderer.device().create_command_encoder(&Default::default()),
        }
    }

    /// Like `render`, but the target is only asked for when the frame really changed (acquiring a swapchain image
    /// costs a vblank wait). `None` = nothing to draw (the frame is identical to the last one, or no target).
    pub fn render_lazy(&mut self, target: impl FnOnce() -> Option<wgpu::TextureView>) -> Option<wgpu::CommandEncoder> {
        if self.screen_off {
            let w = self.world;
            let mut shell = vec![Panel::solid((w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0, w[2] - w[0], w[3] - w[1], 0.0, [0.0; 3], [0.0; 3]).flat()];
            let mut items: Vec<Item<'_>> = Vec::new();
            let hash = self.frame_hash(&shell, &items);
            self.frame_skipped = self.allow_skip && hash == self.last_hash;
            self.last_hash = hash;
            if self.frame_skipped {
                return None;
            }
            let target = target()?;
            self.damage.reset();
            let off = self.canvas_off();
            memrium_de::renderer::shift_scene(&mut items, &mut shell, off[0], off[1]);
            return Some(self.renderer.render(&target, &items, &shell));
        }
        if self.lock.is_some() {
            // the lock screen is the whole picture: no windows, bars or notifications behind it
            let (lock_shell, labels) = self.lock_panels();
            // every monitor is covered, not only the main one
            let w = self.world;
            let mut shell = vec![Panel::solid((w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0, w[2] - w[0], w[3] - w[1], 0.0, [0.0; 3], [0.0; 3]).flat()];
            shell.extend(lock_shell);
            shell.extend(self.renderer.label_panels(&labels));
            let mut items: Vec<Item<'_>> = Vec::new();
            let hash = self.frame_hash(&shell, &items);
            self.frame_skipped = self.allow_skip && !self.renderer.is_live() && hash == self.last_hash;
            self.last_hash = hash;
            if self.frame_skipped {
                return None;
            }
            self.frames_drawn += 1;
            let Some(target) = target() else {
                self.last_hash = 0;
                return None;
            };
            self.damage.reset();
            let off = self.canvas_off();
            memrium_de::renderer::shift_scene(&mut items, &mut shell, off[0], off[1]);
            return Some(self.renderer.render(&target, &items, &shell));
        }
        self.renderer.set_glass_adapt(self.theme.glass.adapt);
        // shell: Dock, Quick Settings, labels
        let (bar_p, bar_l) = self.bar_draw();
        let mut shell = bar_p;
        shell.extend(self.scene.shell_panels());
        let mut labels = self.scene.labels();
        labels.extend(bar_l);
        // the Glass page's samples refract the backdrop the page paints, so they live above the window
        if self.focused_shell() == Some(Sw::Settings) && self.settings.open && !self.swin.minimized && self.swin.k.x > 0.985 {
            let (pp, pl) = self.settings.preview_shell.clone();
            shell.extend(pp);
            labels.extend(pl);
        }
        if self.launcher.visible() {
            let pinned = self.pinned_indices();
            let mut budget = 8u32;
            for i in self.launcher.wanted_icons(&pinned, &self.apps) {
                self.ensure_icon(i, &mut budget);
            }
            let renderer = &self.renderer;
            // the start menu sits on the bar that holds the start button
            let (bar, anchor) = self.with_bar_ctx(|c| match self.bars.iter().find(|b| b.has("start")) {
                Some(b) => ((Some((b.cfg.edge, b.rect(c)))), b.widget_rects(c).iter().find(|(_, _, k)| *k == "start").map(|(_, r, _)| *r)),
                None => (None, None),
            });
            let ctx = DrawCtx { screen: self.size, apps: &self.apps, pinned: &pinned, theme: &self.theme, bar, anchor, icon_cell: &|a: &AppEntry| renderer.app_icon_get(&a.id) };
            let (lp, ll) = self.launcher.draw(&ctx);
            shell.extend(lp);
            labels.extend(ll);
        }
        let mut toast_labels = Vec::new();
        self.toast_panels(&mut shell, &mut toast_labels);
        self.widgets_panels(&mut shell, &mut toast_labels);
        self.flyout_panels(&mut shell, &mut toast_labels);
        self.card_panels(&mut shell, &mut toast_labels);
        labels.extend(toast_labels);
        shell.extend(self.renderer.label_panels(&labels));
        if let Some(d) = self.file_drag.as_ref().filter(|d| d.active) {
            let text = match d.paths.as_slice() {
                [one] => one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                many => format!("{} items", many.len()),
            };
            let text = if self.renderer.text_width(&text, 13.0, true) > 200.0 { format!("{}…", text.chars().take(24).collect::<String>()) } else { text };
            let w = self.renderer.text_width(&text, 13.0, true) + 46.0;
            let (cx, cy) = (self.mouse[0] + 18.0 + w / 2.0, self.mouse[1] + 22.0);
            let mut g = self.theme.glass.panel(cx, cy, w, 30.0, 15.0).tinted([0.08, 0.10, 0.16, 0.70]);
            g.extra[0] = 0.25;
            shell.push(g);
            shell.push(Panel::glyph(cx - w / 2.0 + 18.0, cy, 15.0, if d.paths.len() > 1 { "copy" } else { "file" }, [1.0; 3], 0.95));
            let gl = vec![Label::new(text, cx - w / 2.0 + 34.0, cy + 4.8, 13.0, true, [1.0; 3], 1.0, memrium_de::renderer::Align::Left)];
            shell.extend(self.renderer.label_panels(&gl));
        }
        if self.panel_edit.is_some() && !self.capturing {
            let (ep, el) = self.panel_edit_overlay();
            shell.extend(ep);
            shell.extend(self.renderer.label_panels(&el));
        }
        if self.menu.open {
            let r = &self.renderer;
            let (mp, ml) = self.menu.draw(&self.theme, self.size, &|t, s, b| r.text_width(t, s, b));
            shell.extend(mp);
            shell.extend(self.renderer.label_panels(&ml));
        }

        if !self.capturing {
            self.shot_overlay(&mut shell);
            self.shot_edit_overlay(&mut shell);
        }

        // window layer (titles are text panels inside each window's frame)
        let default_dock = [self.size[0] / 2.0, self.size[1] - 40.0];
        let docks: Vec<[f32; 2]> = self.wins.iter().map(|w| self.item_center(&self.key_of(w)).unwrap_or(default_dock)).collect();
        let focus = self.wins.iter().rposition(|w| !w.minimized);
        let mut title_panels: HashMap<usize, Vec<Panel>> = HashMap::new();
        if !self.menu.open {
            self.title_menu = None;
        }
        let title_hover = match self.chrome_at(self.mouse) {
            Some((j, Hit::Titlebar)) if !self.menu.open => Some(j),
            _ => None,
        };
        for (i, w) in self.wins.iter().enumerate() {
            if !w.mapped || w.min_k.x > 0.985 || w.deco != Deco::Server {
                continue;
            }
            let f = Self::frame_rect(w);
            // the program's menus take the left of the title bar, and the title moves over to make room
            let menus = self.title_menus(i);
            let mut tf = f;
            if let Some((_, last)) = menus.last() {
                let shift = last[0] + last[2] + 14.0 - decor::title_span(&self.theme, f).0;
                if shift > 0.0 {
                    tf = [f[0] + shift, f[1], f[2] - shift, f[3]];
                }
            }
            let (x0, x1) = decor::title_span(&self.theme, tf);
            let title = ellipsize(&self.renderer, &w.title, 13.0, (x1 - x0).max(40.0));
            let th = w.chrome_theme(&self.theme);
            let mut labels = decor::title_labels(&th, tf, &title, Some(i) == focus);
            let mut panels = Vec::new();
            if !menus.is_empty() {
                let rects: Vec<[f32; 4]> = menus.iter().map(|(_, r)| *r).collect();
                let open = self.menu.open.then(|| self.title_menu.as_ref().filter(|(id, _)| *id == w.id).and_then(|(_, n)| menus.iter().position(|(m, _)| m == n))).flatten();
                let under = if open.is_none() && title_hover == Some(i) { menus.iter().position(|(_, r)| Self::in_rect(self.mouse, *r)) } else { None };
                panels = decor::title_menu_panels(&th, &rects, open.or(under), open.is_some());
                labels.extend(decor::title_menu_labels(&th, f, &menus, Some(i) == focus));
            }
            panels.extend(self.renderer.label_panels(&labels));
            title_panels.insert(i, panels);
        }

        let mut shells = Vec::new();
        for k in self.z.clone() {
            let stamp = self.shell_win(k).stamp;
            shells.push((stamp, self.shell_item(k)));
        }
        let desktop = if self.capture_alone { None } else { self.desktop_item() };
        let preview = self.preview_layout();
        self.tooltip_panels(&mut shell);
        if let Some(l) = &preview {
            let labels = self.preview_shell(l);
            let lp = self.renderer.label_panels(&labels);
            shell.extend(lp);
        }
        let ime = self.ime_popup_origins();
        let mut items = build_items(&self.wins, &self.popups, &self.surfaces, ime, focus, &docks, &title_panels, &self.theme, self.hover_btn, shells, desktop, &self.ghosts, self.renderer.scale().max(1.0), &self.rules, (self.ws_pos.x, self.size[0]));
        if let Some(l) = &preview {
            items.extend(crate::dock::preview_items(l, &self.wins, &self.surfaces, self.theme.dark, self.theme.accent));
        }
        if self.capture_alone {
            shell.clear();
        }
        // the picked area's picture, over everything the screen shows
        if let Some(e) = &self.shot_edit {
            items.push(Item::Surface { tex: e.tex(), rect: e.rect(), clip: None, frosted: false, alpha: 1.0, veil: 1.0 });
        }
        let hash = self.frame_hash(&shell, &items);
        self.frame_skipped = self.allow_skip && !self.renderer.is_live() && hash == self.last_hash;
        self.last_hash = hash;
        if self.frame_skipped {
            return None;
        }
        self.frames_drawn += 1;
        if std::env::var_os("MEMRIUM_WHY").is_some() {
            self.why_redraw(&shell, &items);
        }
        let Some(target) = target() else {
            // nothing was drawn, so the next frame must not be skipped as a duplicate
            self.last_hash = 0;
            self.damage.reset();
            return None;
        };
        match self.theme.glass.mode {
            memrium_de::theme::GlassMode::Wood => memrium_de::renderer::retint_boards(&mut items, &mut shell, 13, self.theme.glass.tint),
            memrium_de::theme::GlassMode::Clay => memrium_de::renderer::pastelize_clay(&mut items, &mut shell, self.theme.dark),
            _ => {}
        }
        let off = self.canvas_off();
        // what changed since the last frame (the glass modes that recolour the panels afterwards redraw everything)
        let full = !(self.persistent_target && self.allow_skip) || self.renderer.is_live() || self.capturing || self.renderer.reveal_key() != 0 || std::env::var_os("MEMRIUM_NO_DAMAGE").is_some() || matches!(self.theme.glass.mode, memrium_de::theme::GlassMode::Wood | memrium_de::theme::GlassMode::Clay);
        let damage = self.damage.diff(self.renderer.epoch(), &items, &shell, full).map(|d| [d[0] + off[0], d[1] + off[1], d[2] + off[0], d[3] + off[1]]);
        if !self.allow_skip {
            // drawn into some other texture (a screenshot): the real target does not hold this frame
            self.damage.reset();
        }
        memrium_de::renderer::shift_scene(&mut items, &mut shell, off[0], off[1]);
        Some(self.renderer.render_damaged(&target, &items, &shell, damage))
    }

    /// Draw the region picker, the notice and the flash.
    fn shot_overlay(&mut self, shell: &mut Vec<Panel>) {
        // the picker spans the whole desktop; the notice stays on the main monitor
        let [wx0, wy0, wx1, wy1] = self.world;
        let (sw, sh) = (wx1 - wx0, wy1 - wy0);
        let [psw, _] = self.size;
        if let Some(sel) = &self.shot_sel {
            let dim = |x: f32, y: f32, w: f32, h: f32| Panel::solid(x + w / 2.0, y + h / 2.0, w, h, 0.0, [0.0; 3], [0.0; 3]).flat().alpha(0.38);
            match sel.start {
                None => shell.push(dim(wx0, wy0, sw, sh)),
                Some(a) => {
                    let b = self.mouse;
                    let (x0, y0, x1, y1) = (a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1]));
                    shell.push(dim(wx0, wy0, sw, y0 - wy0));
                    shell.push(dim(wx0, y1, sw, wy1 - y1));
                    shell.push(dim(wx0, y0, x0 - wx0, y1 - y0));
                    shell.push(dim(x1, y0, wx1 - x1, y1 - y0));
                    let c = self.theme.accent;
                    for (x, y, w, h) in [(x0, y0, x1 - x0, 1.5), (x0, y1 - 1.5, x1 - x0, 1.5), (x0, y0, 1.5, y1 - y0), (x1 - 1.5, y0, 1.5, y1 - y0)] {
                        shell.push(Panel::solid(x + w / 2.0, y + h / 2.0, w, h, 0.0, c, c).flat());
                    }
                    let text = format!("{} × {}", ((x1 - x0) * self.renderer.pixel_size().0 as f32 / self.renderer.logical_size()[0]).round(), ((y1 - y0) * self.renderer.pixel_size().1 as f32 / self.renderer.logical_size()[1]).round());
                    let labels = vec![Label::new(text, x0 + 6.0, (y0 - 8.0).max(16.0), 12.5, true, [1.0; 3], 1.0, memrium_de::renderer::Align::Left)];
                    shell.extend(self.renderer.label_panels(&labels));
                }
            }
        }
        if self.snap > 0.01 {
            // where the window will go if it is let go now: the work area, as a soft outline that grows into place
            let area = self.snap_rect(self.mouse, self.snap_side);
            let grow = 28.0 * (1.0 - self.snap);
            let (x, y, w, h) = (area[0] + 8.0 + grow, area[1] + 8.0 + grow, area[2] - 16.0 - 2.0 * grow, area[3] - 16.0 - 2.0 * grow);
            let c = self.theme.accent;
            // a flat veil, not glass: glass would blur the window being dragged under it
            shell.push(Panel::solid(x + w / 2.0, y + h / 2.0, w, h, self.theme.window.radius.min(24.0), c, c).flat().alpha(0.16 * self.snap));
            for (ex, ey, ew, eh) in [(x, y, w, 2.0), (x, y + h - 2.0, w, 2.0), (x, y, 2.0, h), (x + w - 2.0, y, 2.0, h)] {
                shell.push(Panel::solid(ex + ew / 2.0, ey + eh / 2.0, ew, eh, 1.0, c, c).flat().alpha(0.7 * self.snap));
            }
        }
        if self.shot_flash > 0.0 {
            shell.push(Panel::solid(wx0 + sw / 2.0, wy0 + sh / 2.0, sw, sh, 0.0, [1.0; 3], [1.0; 3]).flat().alpha(0.55 * self.shot_flash));
        }
        if let Some(rec) = &self.recording {
            let secs = rec.started.elapsed().as_secs();
            let text = format!("Recording {}:{:02}", secs / 60, secs % 60);
            let w = self.renderer.text_width(&text, 13.0, true) + 48.0;
            let (cx, cy) = (psw - w / 2.0 - 18.0, 58.0);
            let mut g = self.theme.glass.panel(cx, cy, w, 36.0, 18.0).tinted([0.08, 0.10, 0.16, 0.72]);
            g.extra[0] = 0.25;
            shell.push(g);
            // the dot blinks once a second
            let dot = if rec.started.elapsed().as_millis() % 1000 < 700 { 1.0 } else { 0.35 };
            shell.push(Panel::solid(cx - w / 2.0 + 20.0, cy, 10.0, 10.0, 5.0, [0.95, 0.2, 0.2], [0.95, 0.2, 0.2]).flat().alpha(dot));
            let labels = vec![Label::new(text, cx - w / 2.0 + 36.0, cy + 4.8, 13.0, true, [1.0; 3], 1.0, memrium_de::renderer::Align::Left)];
            shell.extend(self.renderer.label_panels(&labels));
        }
        if let Some((text, left)) = &self.shot_notice {
            let w = self.renderer.text_width(text, 13.0, true) + 48.0;
            let a = (left * 3.0).min(1.0);
            let (cx, cy) = (psw - w / 2.0 - 18.0, if self.recording.is_some() { 102.0 } else { 58.0 });
            let mut g = self.theme.glass.panel(cx, cy, w, 36.0, 18.0).tinted([0.08, 0.10, 0.16, 0.72]).alpha(a);
            g.extra[0] = 0.25;
            shell.push(g);
            shell.push(Panel::glyph(cx - w / 2.0 + 20.0, cy, 16.0, "check", [1.0; 3], a));
            let labels = vec![Label::new(text.clone(), cx - w / 2.0 + 36.0, cy + 4.8, 13.0, true, [1.0; 3], a, memrium_de::renderer::Align::Left)];
            shell.extend(self.renderer.label_panels(&labels));
        }
    }

    /// Render the screen without overlays and save it as a PNG.
    /// Draw the frame offscreen (without overlays) and read back the `logical` rectangle (the whole screen when
    /// `None`) as RGBA. Returns (width, height, pixels) in device pixels.
    /// The device-pixel box (x0, y0, x1, y1) a logical rectangle (the whole screen when `None`) is cut out of the picture with.
    pub fn crop_px(&self, logical: Option<[f32; 4]>) -> [u32; 4] {
        let (pw, ph) = self.renderer.pixel_size();
        // the rectangle is in desktop coordinates; the picture starts at the desktop's top-left corner
        let [cl_w, cl_h] = self.renderer.logical_size();
        let off = self.canvas_off();
        let logical = logical.map(|r| [r[0] + off[0], r[1] + off[1], r[2], r[3]]).unwrap_or([0.0, 0.0, cl_w, cl_h]);
        let (kx, ky) = (pw as f32 / cl_w, ph as f32 / cl_h);
        let x0 = ((logical[0] * kx).round().max(0.0) as u32).min(pw - 1);
        let y0 = ((logical[1] * ky).round().max(0.0) as u32).min(ph - 1);
        let x1 = (((logical[0] + logical[2]) * kx).round().max(0.0) as u32).clamp(x0 + 1, pw);
        let y1 = (((logical[1] + logical[3]) * ky).round().max(0.0) as u32).clamp(y0 + 1, ph);
        [x0, y0, x1, y1]
    }

    pub fn capture_rgba(&mut self, logical: Option<[f32; 4]>) -> (u32, u32, Vec<u8>) {
        let (pw, ph) = self.renderer.pixel_size();
        let format = self.renderer.format();
        let device = self.renderer.device().clone();
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let (skip, notice) = (self.allow_skip, self.shot_notice.take());
        self.allow_skip = false;
        self.capturing = true;
        let mut enc = self.render(&view);
        self.capturing = false;
        self.allow_skip = skip;
        self.shot_notice = notice;
        self.invalidate();
        let padded = (pw * 4 + 255) / 256 * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (padded * ph) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(ph) } },
            wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
        );
        self.renderer.queue().submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::Wait).unwrap();
        let [x0, y0, x1, y1] = self.crop_px(logical);
        let cw = x1 - x0;
        let ch = y1 - y0;
        let bgra = matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut rgba = Vec::with_capacity((cw * ch * 4) as usize);
        {
            let mapped = slice.get_mapped_range();
            for row in y0..y1 {
                let line = &mapped[(row * padded + x0 * 4) as usize..(row * padded + x1 * 4) as usize];
                for px in line.chunks_exact(4) {
                    if bgra {
                        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                    } else {
                        rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                    }
                }
            }
        }
        buf.unmap();
        (cw, ch, rgba)
    }

    fn take_screenshot(&mut self, req: ShotReq) {
        let (cw, ch, rgba) = match req {
            ShotReq::Edit(r) => {
                self.start_shot_edit(r);
                return;
            }
            ShotReq::Record(r) => {
                self.start_recording(r);
                return;
            }
            // a window over nothing: only its own pixels and its shadow
            ShotReq::Window => match self.shot_window_alpha() {
                Some(shot) => shot,
                None => return,
            },
            ShotReq::Full => self.capture_rgba(None),
            ShotReq::Region(r) => self.capture_rgba(Some(r)),
        };
        self.finish_shot(rgba, cw, ch, req == ShotReq::Full);
    }

    /// The record key: stops a recording, or opens the area picker for a new one.
    fn toggle_recording(&mut self) {
        if self.recording.is_some() {
            self.stop_recording();
        } else if self.rec_done.is_none() {
            self.menu.close();
            self.shot_sel = Some(ShotSel { start: None, record: true });
        }
    }

    fn start_recording(&mut self, logical: [f32; 4]) {
        let [wx0, wy0, wx1, wy1] = self.world;
        let x0 = logical[0].max(wx0);
        let y0 = logical[1].max(wy0);
        let r = [x0, y0, (logical[0] + logical[2]).min(wx1) - x0, (logical[1] + logical[3]).min(wy1) - y0];
        if r[2] < 4.0 || r[3] < 4.0 {
            return;
        }
        let prefs = &self.config.screenshots;
        let path = if prefs.target == memrium_de::shots::ShotTarget::Clipboard { std::path::PathBuf::from(shot_cache_dir()).join(format!("Recording_{}.mp4", memrium_de::shots::timestamp())) } else { prefs.video_path() };
        self.recording = Some(crate::screenrec::Recording::new(r, path, prefs.record_fps));
    }

    fn stop_recording(&mut self) {
        if let Some(rec) = self.recording.take() {
            self.rec_done = Some(rec.finish());
            self.shot_notice = Some(("Saving the recording…".into(), 30.0));
        }
    }

    /// Grab a frame when one is due; pick up the result of a finished recording.
    fn record_tick(&mut self) {
        if let Some(rx) = &self.rec_done {
            if let Ok(res) = rx.try_recv() {
                self.rec_done = None;
                match res {
                    Ok(path) => {
                        eprintln!("recording: {}", path.display());
                        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        self.shot_notice = Some((format!("Saved {name}"), 4.0));
                    }
                    Err(e) => {
                        eprintln!("recording failed: {e}");
                        self.shot_notice = Some((format!("Recording failed: {e}"), 5.0));
                    }
                }
            }
        }
        let Some(rec) = self.recording.as_ref().filter(|r| r.due()) else { return };
        let rect = rec.rect;
        let (w, h, rgba) = self.capture_rgba(Some(rect));
        if let Some(Err(e)) = self.recording.as_mut().map(|r| r.push(rgba, w, h)) {
            eprintln!("recording failed: {e}");
            self.recording = None;
            self.shot_notice = Some((format!("Recording failed: {e}"), 5.0));
        }
    }

    /// Save a finished picture and tell the user.
    pub fn finish_shot(&mut self, rgba: Vec<u8>, cw: u32, ch: u32, full: bool) {
        use memrium_de::shots::{ScreenshotPrefs, ShotTarget};
        let prefs = self.config.screenshots.clone();
        let to_clipboard = prefs.target != ShotTarget::Folder;
        let mut copied = false;
        if to_clipboard {
            match ScreenshotPrefs::png(&rgba, cw, ch) {
                Ok(png) => {
                    self.clip_set_image(png, cw, ch);
                    copied = true;
                }
                Err(e) => eprintln!("screenshot: clipboard: {e}"),
            }
        }
        // only to the clipboard: a copy is kept in the cache all the same, so the picture can be dragged out of the toast
        let clip_only = prefs.target == ShotTarget::Clipboard && copied;
        let saved = if clip_only {
            let mut cached = prefs.clone();
            cached.dir = shot_cache_dir();
            Some(cached.save(rgba, cw, ch))
        } else {
            Some(prefs.save(rgba, cw, ch))
        };
        let mut file = None;
        let notice = match &saved {
            Some(Ok(path)) => {
                eprintln!("screenshot: {}", path.display());
                file = Some(path.clone());
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                Some(if clip_only { "Screenshot copied to the clipboard".to_string() } else if copied { format!("Saved {name} and copied") } else { format!("Saved {name}") })
            }
            Some(Err(e)) => {
                eprintln!("screenshot failed: {e}");
                self.shot_notice = Some((format!("Screenshot failed: {e}"), 4.0));
                return;
            }
            None => Some("Screenshot copied to the clipboard".to_string()),
        };
        if prefs.sound && std::env::var_os("MEMRIUM_NO_SOUND").is_none() {
            memrium_de::sound::play(if full { memrium_de::sound::Sound::Shutter } else { memrium_de::sound::Sound::Click }, prefs.volume);
        }
        if prefs.flash {
            self.shot_flash = 1.0;
        }
        if prefs.notice {
            match (notice, file) {
                (Some(text), Some(path)) => self.shot_toast(text, path),
                (notice, _) => self.shot_notice = notice.map(|n| (n, 3.5)),
            }
        }
    }

    /// Forget the last frame so the next `render` draws for sure (VT switch, resize).
    pub fn invalidate(&mut self) {
        self.last_hash = 0;
        self.damage.reset();
    }

    /// Debug (`MEMRIUM_WHY=1`): says what made this frame differ from the last one, summed up every 200 frames.
    fn why_redraw(&self, shell: &[Panel], items: &[memrium_de::renderer::Item]) {
        use memrium_de::renderer::Item;
        use std::cell::RefCell;
        use std::collections::HashMap;
        use std::hash::Hasher;
        #[derive(Default)]
        struct Why {
            n: u32,
            epoch: u64,
            shell: u64,
            surf: HashMap<usize, u64>,
            count: HashMap<String, u32>,
        }
        thread_local!(static WHY: RefCell<Why> = RefCell::new(Why::default()));
        WHY.with(|w| {
            let mut w = w.borrow_mut();
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            hs.write(bytemuck::cast_slice(shell));
            let (ep, sh) = (self.renderer.epoch(), hs.finish());
            if ep != w.epoch {
                *w.count.entry("renderer epoch".into()).or_default() += 1;
            }
            if sh != w.shell {
                *w.count.entry("shell panels".into()).or_default() += 1;
            }
            let mut changed = Vec::new();
            for it in items {
                match it {
                    Item::Surface { tex, .. } => {
                        let k = *tex as *const _ as usize;
                        let g = tex.generation();
                        if w.surf.insert(k, g) != Some(g) {
                            changed.push("surface".to_string());
                        }
                    }
                    Item::Frame { .. } => changed.push("window frame".to_string()),
                    Item::Layer { .. } => changed.push("layer".to_string()),
                    _ => {}
                }
            }
            // a frame item is only counted when the whole item list's hash moved, so the surface lines are the useful ones
            for c in changed.into_iter().filter(|c| c == "surface") {
                *w.count.entry(c).or_default() += 1;
            }
            w.epoch = ep;
            w.shell = sh;
            w.n += 1;
            if w.n >= 200 {
                eprintln!("why: {} redraws: {:?}", w.n, w.count);
                w.n = 0;
                w.count.clear();
            }
        });
    }

    fn frame_hash(&self, shell: &[Panel], items: &[memrium_de::renderer::Item]) -> u64 {
        use memrium_de::renderer::Item;
        use std::hash::Hasher;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        h.write_u64(self.renderer.epoch());
        h.write_u32(self.renderer.reveal_key());
        h.write(bytemuck::cast_slice(shell));
        for it in items {
            match it {
                Item::Warp(w) => {
                    h.write_u8(2);
                    if let Some(w) = w {
                        h.write(bytemuck::cast_slice(&w.rect));
                        h.write_u32(w.t.to_bits());
                        h.write(bytemuck::cast_slice(&w.dock));
                        if let Some(j) = &w.jelly {
                            h.write(bytemuck::cast_slice(j));
                        }
                    }
                }
                Item::Frame { panels, bbox, fade } => {
                    h.write_u8(0);
                    h.write_u32(fade.to_bits());
                    h.write(bytemuck::cast_slice(panels));
                    h.write(bytemuck::cast_slice(bbox));
                }
                Item::Layer { panels, bbox } => {
                    h.write_u8(3);
                    h.write(bytemuck::cast_slice(panels));
                    h.write(bytemuck::cast_slice(bbox));
                }
                Item::Opaque(_) => {}
                Item::Surface { tex, rect, clip, frosted, alpha, veil } => {
                    h.write_u8(1);
                    h.write_u32(veil.to_bits());
                    h.write_u32(alpha.to_bits());
                    h.write_usize(*tex as *const _ as usize);
                    h.write_u64(tex.generation());
                    h.write(bytemuck::cast_slice(rect));
                    h.write_u8(*frosted as u8);
                    if let Some((r, rad)) = clip {
                        h.write(bytemuck::cast_slice(r));
                        h.write_u32(rad.to_bits());
                    }
                }
            }
        }
        h.finish().max(1)
    }
}

/// Apply a window's lift/minimize transform (scale about its centre, then a pull towards the Dock).
struct Xf {
    c: [f32; 2],
    s: f32,
    t: [f32; 2],
    /// opacity of the whole window
    fade: f32,
}

impl Xf {
    fn new(w: &Win, dock: [f32; 2]) -> Xf {
        let f = State::frame_rect(w);
        let c = [f[0] + f[2] / 2.0, f[1] + f[3] / 2.0];
        let m = w.min_k.x.clamp(0.0, 1.0);
        let o = w.open_k.x;
        let _ = (m, dock);
        Xf { c, s: w.scale.x * (0.9 + 0.1 * o.min(1.08)), t: [0.0, 0.0], fade: (o * 1.7).clamp(0.0, 1.0) }
    }

    fn rect(&self, r: [f32; 4]) -> [f32; 4] {
        let rc = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
        let nc = [self.c[0] + (rc[0] - self.c[0]) * self.s + self.t[0], self.c[1] + (rc[1] - self.c[1]) * self.s + self.t[1]];
        [nc[0] - r[2] * self.s / 2.0, nc[1] - r[3] * self.s / 2.0, r[2] * self.s, r[3] * self.s]
    }

    fn panel(&self, mut p: Panel) -> Panel {
        let nc = [self.c[0] + (p.rect[0] - self.c[0]) * self.s + self.t[0], self.c[1] + (p.rect[1] - self.c[1]) * self.s + self.t[1]];
        p.rect = [nc[0], nc[1], p.rect[2] * self.s, p.rect[3] * self.s];
        p.params[0] *= self.s;
        p.extra[1] *= self.s;
        p.extra[2] *= self.s;
        p
    }
}

fn build_items<'a>(
    wins: &'a [Win],
    popups: &'a [Popup],
    surfaces: &'a HashMap<ObjectId, SurfNode>,
    ime: Vec<(WlSurface, [f32; 2])>,
    focus: Option<usize>,
    docks: &[[f32; 2]],
    titles: &HashMap<usize, Vec<Panel>>,
    theme: &Theme,
    hover: Option<(usize, Btn)>,
    shells: Vec<(u64, Vec<Item<'a>>)>,
    desktop: Option<Item<'a>>,
    ghosts: &'a [Ghost],
    dev: f32,
    rules: &memrium_de::apprules::Rules,
    desks: (f32, f32),
) -> Vec<Item<'a>> {
    // A program's picture is put on whole device pixels: at a place between two the filter smears every letter (a window that
    // was moved by the pointer, or maximized into a fractional area, lands there).
    let snap = |v: f32| (v * dev).round() / dev;
    let mut out = Vec::new();
    out.extend(desktop);
    // shell windows (Settings, Files) and client windows stack by when each was last raised: a shell window goes in
    // before the first client window that was raised after it
    let mut shells = shells.into_iter().peekable();
    let node_size = |s: &WlSurface| -> Option<(&'a SurfaceTex, [f32; 2], Option<[f32; 4]>)> {
        let n = surfaces.get(&s.id())?;
        let t = n.tex.as_deref()?;
        Some((t, n.logical_size()?, n.opaque))
    };
    for (i, w) in wins.iter().enumerate() {
        // a window of another desktop is beside the one on show; it is seen only while the row slides
        let dx = if w.or { 0.0 } else { desk_dx(w.ws, desks.0, desks.1) };
        if !w.mapped || w.min_k.x > 0.985 || dx.abs() >= desks.1 {
            continue;
        }
        // an X11 menu or tooltip is above every window
        let stamp = if w.or { u64::MAX } else { w.stamp };
        while shells.peek().is_some_and(|(s, _)| *s < stamp) {
            out.extend(shells.next().map(|(_, items)| items).unwrap_or_default());
        }
        let mut xf = Xf::new(w, docks.get(i).copied().unwrap_or([0.0, 0.0]));
        xf.t[0] = dx;
        let f = State::frame_rect(w);
        let genie = w.min_k.x.clamp(0.0, 1.0);
        let wobble = w.jel[0].x.abs() + w.jel[1].x.abs() > 0.4;
        if genie > 0.001 {
            out.push(Item::Warp(Some(Warp { rect: f, t: genie, dock: docks.get(i).copied().unwrap_or([0.0, 0.0]), icon_w: 52.0, jelly: None })));
        } else if wobble {
            out.push(Item::Warp(Some(jelly_warp(f, [w.jel[0].x, w.jel[1].x], w.jgrab))));
        }
        let focused = Some(i) == focus;
        let maximized = w.restore.is_some();
        let server = w.deco == Deco::Server && w.fs.is_none();
        let fs = w.fs.is_some();

        // chrome: always a shadow; the decoration itself only when we draw it (server-side)
        let mut panels: Vec<Panel> = if server {
            decor::frame_panels(&w.chrome_theme(theme), f, focused, maximized, hover.filter(|(hi, _)| *hi == i).map(|(_, b)| b))
        } else {
            let sh = if fs { [0.0; 3] } else if focused { theme.window.shadow } else { theme.window.shadow_idle };
            let r = if w.deco == Deco::Client && !fs { theme.window.radius.min(18.0) } else { 0.0 };
            vec![Panel::solid(f[0] + f[2] / 2.0, f[1] + f[3] / 2.0, f[2], f[3], r, [0.97; 3], [0.95; 3]).flat().alpha(0.0).shadow(sh[0], sh[1], sh[2])]
        };
        for p in panels.iter_mut() {
            *p = xf.panel(*p);
        }
        if let Some(t) = titles.get(&i) {
            panels.extend(t.iter().map(|p| xf.panel(*p)));
        }
        // as far as the panels reach (the shadow mostly), not further: every pixel of the box runs the frame shader
        let bb = xf.rect([f[0] - 90.0, f[1] - 90.0, f[2] + 180.0, f[3] + 180.0]);
        let bbox = panels.iter().map(memrium_de::renderer::panel_box).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]).unwrap_or([bb[0], bb[1], bb[0] + bb[2], bb[1] + bb[3]]);
        out.push(Item::Frame { panels, bbox, fade: xf.fade });

        // the client's surfaces (root + subsurfaces), clipped to the window shape
        let Some(root) = w.role.surface() else { continue };
        let root = &root;
        let origin = [f[0] + w.ins[0] - w.geo[0], f[1] + w.ins[1] - w.geo[1]];
        let client_rect = [f[0] + w.ins[0], f[1] + w.ins[1], f[2] - w.ins[0] - w.ins[2], f[3] - w.ins[1] - w.ins[3]];
        let clip = match (w.deco, theme.window.decoration) {
            _ if fs => None,
            (Deco::Legacy, _) => None,
            // Aero: the glass frame supplies the rounded outline, the client is a plain rectangle
            (Deco::Server, _) if decor::glass_frame(theme) => Some((xf.rect(client_rect), 0.0)),
            (Deco::Server, _) => Some((xf.rect(f), if maximized && theme.window.max_gap < 0.5 { 0.0 } else { theme.window.radius * xf.s })),
            (Deco::Client, _) => Some((xf.rect(f), theme.window.radius.min(18.0) * xf.s)),
        };
        for (s, off) in surface_tree(root) {
            let Some((tex, sz, opaque)) = node_size(&s) else { continue };
            let mut r = xf.rect([origin[0] + off[0] as f32, origin[1] + off[1] as f32, sz[0], sz[1]]);
            // while the window is animating (opening, jelly, genie) it moves smoothly; at rest it is on the pixel grid
            if genie <= 0.001 && !wobble && dx == 0.0 && (xf.s - 1.0).abs() < 0.002 {
                r[0] = snap(r[0]);
                r[1] = snap(r[1]);
            }
            let is_root = &s == root;
            // translucent toplevels sit on frosted glass
            // a rule for the program can thin it out (`opacity`) and frost what is behind it (`frost`)
            let rule = rules.find(&[w.app_id.as_str()]);
            let veil = if is_root { rule.and_then(|r| r.opacity).unwrap_or(1.0) } else { 1.0 };
            let frosted = is_root && rule.and_then(|r| r.frost).unwrap_or(!tex.opaque || veil < 0.999);
            out.push(Item::Surface { tex, rect: r, clip, frosted, alpha: xf.fade, veil });
            // a window at rest hides what is under its opaque part: the renderer leaves that out (not when it is thinned out)
            if let Some(o) = opaque.filter(|_| genie <= 0.001 && !wobble && (xf.s - 1.0).abs() < 0.002 && xf.fade >= 0.999 && veil >= 0.999) {
                let mut b = [r[0] + o[0], r[1] + o[1], r[0] + o[0] + o[2], r[1] + o[1] + o[3]];
                if let Some((c, rad)) = clip {
                    // the rounded corners let the frame show: only the band between them counts
                    b = [b[0].max(c[0]), b[1].max(c[1] + rad), b[2].min(c[0] + c[2]), b[3].min(c[1] + c[3] - rad)];
                }
                if b[2] > b[0] && b[3] > b[1] {
                    out.push(Item::Opaque(b));
                }
            }
        }
        if genie > 0.001 || wobble {
            out.push(Item::Warp(None));
        }
    }

    // windows that just closed shrink and fade
    for g in ghosts {
        let c = [g.frame[0] + g.frame[2] / 2.0, g.frame[1] + g.frame[3] / 2.0];
        let xf = Xf { c, s: 0.9 + 0.1 * g.k.x.clamp(0.0, 1.0), t: [0.0, 0.0], fade: (g.k.x * 1.5).clamp(0.0, 1.0) };
        let bb = xf.rect([g.frame[0] - 90.0, g.frame[1] - 90.0, g.frame[2] + 180.0, g.frame[3] + 180.0]);
        out.push(Item::Frame { panels: g.panels.iter().map(|p| xf.panel(*p)).collect(), bbox: [bb[0], bb[1], bb[0] + bb[2], bb[1] + bb[3]], fade: xf.fade });
        if let Some((tex, rect, clip, frosted)) = &g.surf {
            let clip = clip.map(|(r, rad)| (xf.rect(r), rad * xf.s));
            out.push(Item::Surface { tex, rect: xf.rect(*rect), clip, frosted: *frosted, alpha: xf.fade, veil: 1.0 });
        }
    }

    for (_, items) in shells {
        out.extend(items);
    }

    // popups on top of everything
    for p in popups {
        let root = p.surface.wl_surface();
        let Some(parent) = p.surface.get_parent_surface() else { continue };
        // popup geometry origin chain
        let go = popup_geometry_origin(wins, popups, &root_of(&parent), p);
        let Some(go) = go else { continue };
        let gl = with_states(root, |st| st.cached_state.get::<SurfaceCachedState>().current().geometry).map(|g| [g.loc.x as f32, g.loc.y as f32]).unwrap_or([0.0, 0.0]);
        let origin = [go[0] - gl[0], go[1] - gl[1]];
        let Some((_, root_sz, _)) = node_size(root) else { continue };
        // it opens from the edge that is next to what it belongs to: a little smaller, a little higher, then settles
        let k = p.k.x;
        let xf = if (k - 1.0).abs() < 0.002 {
            Xf { c: [0.0; 2], s: 1.0, t: [0.0; 2], fade: 1.0 }
        } else {
            Xf { c: [origin[0] + root_sz[0] / 2.0, origin[1]], s: 0.94 + 0.06 * k.min(1.08), t: [0.0, -5.0 * (1.0 - k.min(1.0))], fade: (k * 1.8).clamp(0.0, 1.0) }
        };
        let at_rest = xf.s == 1.0;
        for (s, off) in surface_tree(root) {
            let Some((tex, sz, _)) = node_size(&s) else { continue };
            let mut r = [origin[0] + off[0] as f32, origin[1] + off[1] as f32, sz[0], sz[1]];
            if at_rest {
                r[0] = snap(r[0]);
                r[1] = snap(r[1]);
            }
            let is_root = &s == root;
            // a popup that keeps its own shadow in a see-through margin (Telegram's menus): shadow and glass follow the solid parts
            let parts: Vec<[f32; 4]> = if is_root {
                surfaces.get(&s.id()).filter(|n| !n.body.is_empty()).and_then(|n| {
                    let t = n.tex.as_deref()?;
                    let l = n.logical_size()?;
                    let (kx, ky) = (l[0] / t.size.0 as f32, l[1] / t.size.1 as f32);
                    Some(n.body.iter().map(|b| [r[0] + b[0] as f32 * kx, r[1] + b[1] as f32 * ky, (b[2] - b[0]) as f32 * kx, (b[3] - b[1]) as f32 * ky]).collect())
                }).unwrap_or_default()
            } else {
                Vec::new()
            };
            if is_root {
                let pieces: Vec<[f32; 4]> = if parts.is_empty() { vec![r] } else { parts.clone() };
                let mut bbox = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                let panels: Vec<Panel> = pieces
                    .iter()
                    .map(|q| {
                        bbox = [bbox[0].min(q[0] - 40.0), bbox[1].min(q[1] - 40.0), bbox[2].max(q[0] + q[2] + 40.0), bbox[3].max(q[1] + q[3] + 60.0)];
                        let radius = if parts.is_empty() { 10.0 } else { (q[3] / 2.0).min(12.0) };
                        let panel = Panel::solid(q[0] + q[2] / 2.0, q[1] + q[3] / 2.0, q[2], q[3], radius, [1.0; 3], [1.0; 3]).flat().alpha(0.0).shadow(0.30, 24.0, 8.0);
                        if at_rest { panel } else { xf.panel(panel) }
                    })
                    .collect();
                out.push(Item::Frame { panels, bbox, fade: xf.fade });
            }
            let rect = if at_rest { r } else { xf.rect(r) };
            // a see-through popup sits on frosted glass, like a window does; the clip is a pixel inside a round popup, so the half-covered
            // pixels of its edge do not pick up a ring of blurred desktop
            let clip = if !is_root {
                None
            } else if parts.is_empty() {
                Some(([rect[0] + xf.s, rect[1] + xf.s, rect[2] - 2.0 * xf.s, rect[3] - 2.0 * xf.s], 17.0 * xf.s))
            } else {
                // round the glass to the bounding box of the solid parts: what lies outside is only the program's own shadow
                let b = parts.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, q| [a[0].min(q[0]), a[1].min(q[1]), a[2].max(q[0] + q[2]), a[3].max(q[1] + q[3])]);
                let b = [b[0], b[1], b[2] - b[0], b[3] - b[1]];
                let b = if at_rest { b } else { xf.rect(b) };
                Some(([b[0] + xf.s, b[1] + xf.s, b[2] - 2.0 * xf.s, b[3] - 2.0 * xf.s], 10.0 * xf.s))
            };
            out.push(Item::Surface { tex, rect, clip, frosted: is_root && !tex.opaque, alpha: xf.fade, veil: 1.0 });
        }
    }
    // the input method's candidate windows, over everything
    for (root, o) in &ime {
        for (s, off) in surface_tree(root) {
            let Some((tex, sz, _)) = node_size(&s) else { continue };
            let r = [snap(o[0] + off[0] as f32), snap(o[1] + off[1] as f32), sz[0], sz[1]];
            if &s == root {
                out.push(Item::Frame { panels: vec![Panel::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], 8.0, [1.0; 3], [1.0; 3]).flat().alpha(0.0).shadow(0.30, 18.0, 6.0)], bbox: [r[0] - 30.0, r[1] - 30.0, r[0] + r[2] + 30.0, r[1] + r[3] + 40.0], fade: 1.0 });
            }
            out.push(Item::Surface { tex, rect: r, clip: None, frosted: false, alpha: 1.0, veil: 1.0 });
        }
    }
    out
}

/// Geometry origin of popup `p`, given its already-resolved parent chain (free function so it can run while the renderer is borrowed).
fn popup_geometry_origin(wins: &[Win], popups: &[Popup], parent_root: &WlSurface, p: &Popup) -> Option<[f32; 2]> {
    let base = if let Some(w) = wins.iter().find(|w| w.role.is_surface(parent_root)) {
        [w.x.x + w.ins[0], w.y.x + w.ins[1]]
    } else {
        let pp = popups.iter().find(|q| q.surface.wl_surface() == parent_root)?;
        let up = pp.surface.get_parent_surface()?;
        popup_geometry_origin(wins, popups, &root_of(&up), pp)?
    };
    let g: Rectangle<i32, Logical> = p.surface.with_pending_state(|s| s.geometry);
    Some([base[0] + g.loc.x as f32, base[1] + g.loc.y as f32])
}

#[cfg(test)]
mod resize_tests {
    use super::State;

    // a 400 x 300 frame at (100, 100): the zone is EDGE px either side of each edge
    const F: [f32; 4] = [100.0, 100.0, 400.0, 300.0];

    #[test]
    fn edges_and_corners_are_found_just_inside_and_just_outside() {
        assert_eq!(State::edges_of(F, [102.0, 250.0]), 1, "left, inside");
        assert_eq!(State::edges_of(F, [96.0, 250.0]), 1, "left, outside");
        assert_eq!(State::edges_of(F, [498.0, 250.0]), 2, "right");
        assert_eq!(State::edges_of(F, [300.0, 103.0]), 4, "top");
        assert_eq!(State::edges_of(F, [300.0, 398.0]), 8, "bottom");
        assert_eq!(State::edges_of(F, [101.0, 101.0]), 5, "top left corner");
        assert_eq!(State::edges_of(F, [499.0, 399.0]), 10, "bottom right corner");
    }

    #[test]
    fn the_middle_and_far_away_are_not_edges() {
        assert_eq!(State::edges_of(F, [300.0, 250.0]), 0);
        assert_eq!(State::edges_of(F, [50.0, 250.0]), 0);
        assert_eq!(State::edges_of(F, [300.0, 460.0]), 0);
    }
}

#[cfg(test)]
mod solid_parts_tests {
    use super::solid_parts;

    fn fill(buf: &mut [u8], w: usize, r: [usize; 4]) {
        for y in r[1]..r[3] {
            for x in r[0]..r[2] {
                buf[(y * w + x) * 4 + 3] = 255;
            }
        }
    }

    #[test]
    fn bubble_over_menu() {
        let (w, h) = (200, 300);
        let mut b = vec![0u8; w * h * 4];
        // faint shadow everywhere, a wide bubble, a gap, a narrower menu
        for px in b.chunks_mut(4) {
            px[3] = 30;
        }
        fill(&mut b, w, [10, 10, 190, 46]);
        fill(&mut b, w, [35, 60, 165, 280]);
        let p = solid_parts(&b, w, h, w * 4);
        assert_eq!(p, vec![[10, 10, 190, 46], [35, 60, 165, 280]]);
    }

    #[test]
    fn solid_all_over_is_empty() {
        let (w, h) = (100, 100);
        let mut b = vec![0u8; w * h * 4];
        fill(&mut b, w, [0, 0, 100, 100]);
        assert!(solid_parts(&b, w, h, w * 4).is_empty());
    }
}
