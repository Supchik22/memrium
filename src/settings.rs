//! The Settings window, drawn by the shell (immediate mode): templates, accent, glass, wallpaper,
//! layout, window style and icons. Every control edits the live `Theme`; the caller applies and saves it.

use chebrium::spring::Spring;
use crate::files::Key;
use crate::renderer::{Align, Label, Panel, PanelDraw};
use chebrium::form::Form;
use chebrium::style::Style;
use chebrium::ui::{State, Ui};
use crate::widget::{self, BarConfig, BarShape, BarSurface, OptKind, WidgetSpec};
use crate::theme::{self, GlassMode, MenubarKind, DecoKind, Edge, IconShape, Layout, Plate, Theme, TileMode, WallKind, WallpaperSpec};

/// Size of the Settings window's client area.
pub const WIN_W: f32 = 900.0;
pub const WIN_H: f32 = 760.0;
const SIDEBAR: f32 = 196.0;

#[path = "settings_general.rs"]
mod general;
#[path = "settings_more.rs"]
mod more;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    General,
    Themes,
    Displays,
    Appearance,
    Glass,
    Panels,
    Wallpaper,
    Layout,
    Windows,
    Animations,
    Keys,
    Mouse,
    Icons,
    About,
    Notifications,
    DateTime,
    Sound,
    Network,
    Power,
    Programs,
}

const PAGES: [(Page, &str, &str); 20] = [
    (Page::General, "General", "settings"),
    (Page::Themes, "Themes", "layers"),
    (Page::Displays, "Displays", "monitor"),
    (Page::Appearance, "Appearance", "layout-grid"),
    (Page::Glass, "Glass", "sliders-horizontal"),
    (Page::Panels, "Panels", "panel-top"),
    (Page::Wallpaper, "Wallpaper", "image"),
    (Page::Layout, "Layout", "layout-panel-left"),
    (Page::Windows, "Windows", "app-window"),
    (Page::Animations, "Animations", "audio-waveform"),
    (Page::Keys, "Keyboard", "terminal"),
    (Page::Mouse, "Mouse", "move"),
    (Page::Icons, "Icons", "sticky-note"),
    (Page::About, "About", "monitor"),
    (Page::Notifications, "Notifications", "bell"),
    (Page::DateTime, "Date & Time", "clock"),
    (Page::Sound, "Sound", "volume-2"),
    (Page::Network, "Network", "wifi"),
    (Page::Power, "Power & Battery", "battery-full"),
    (Page::Programs, "Programs", "layout-panel-left"),
];

/// The colour a settings page has when the accent is several: blue for the look of things, and its own for the rest.
fn page_tone(page: Page) -> chebrium::style::Tone {
    use chebrium::style::Tone::*;
    match page {
        Page::Themes => Special,
        Page::Appearance => Primary,
        Page::Glass => Info,
        Page::Wallpaper => Success,
        Page::Icons => Warning,
        Page::Layout => Indigo,
        Page::Panels => Primary,
        Page::Windows => Primary,
        Page::Animations => Pink,
        Page::Notifications => Danger,
        Page::DateTime => Warning,
        Page::Sound => Pink,
        Page::Network => Primary,
        Page::Power => Success,
        Page::Programs => Indigo,
        Page::General => Grey,
        Page::Displays => Info,
        Page::Mouse => Grey,
        Page::Keys => Grey,
        Page::About => Grey,
    }
}

/// A wallpaper choice shown in the grid.
pub struct WallItem {
    pub name: String,
    pub spec: WallpaperSpec,
    /// thumbnail in the app-icon atlas (images)
    pub cell: Option<u32>,
    /// preview colours for live wallpapers
    pub colors: Option<([f32; 3], [f32; 3])>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsAction {
    /// some theme field changed
    ThemeChanged,
    /// switch to a built-in or user template (by id); drops per-field overrides
    Preset(String),
    Wallpaper(WallpaperSpec),
    /// the shell's language was chosen ("auto" or a code)
    SetLanguage(String),
    /// keep the current look as a template with this name
    SaveTemplate(String),
    /// take a theme file (path) into the templates
    ImportTemplate(String),
    /// set the screens up again with the display preferences
    ApplyDisplays,
    /// a command for the machine (volume, Wi-Fi, power profile)
    Sys(crate::sysctl::Cmd),
    /// show a notification, to see how they look
    TestNotification,
    Close,
}

pub struct SettingsCtx<'a> {
    pub screen: [f32; 2],
    pub theme: &'a mut Theme,
    pub presets: &'a [(String, String)],
    /// id of the template the theme is based on
    pub preset_id: &'a str,
    pub walls: &'a [WallItem],
    /// the language setting ("auto" or a code)
    pub language: &'a str,
    /// an accent colour taken from the wallpaper, once it is loaded
    pub wall_accent: Option<[f32; 3]>,
    /// everything in the config that is not the theme (shortcuts, idle times, screenshots); a changed copy is saved
    pub cfg: &'a mut crate::theme::Config,
    /// the connected monitors (empty when the shell runs inside a window)
    pub monitors: &'a [crate::display::MonitorInfo],
    /// the client area (the window can be maximised)
    pub size: [f32; 2],
    /// a few applications' icons in the app-icon atlas (cell, name), for the Icons page's preview
    pub icons: &'a [(u32, String)],
    /// the machine's state: volume, Wi-Fi, battery, power profile
    pub sys: &'a crate::sysctl::SysInfo,
    /// the cell of an installed program's icon in the app-icon atlas, by app id, once it is loaded
    pub app_icon: &'a dyn Fn(&str) -> Option<u32>,
    /// the app ids whose icons a page showed without having them: the shell loads them for the next frame
    pub want_icons: &'a mut Vec<String>,
}

pub struct Settings {
    pub open: bool,
    page: Page,
    /// pointer, drag, text editing and animations (Chebrium)
    ui: State,
    /// first visible row of the wallpaper grid
    wall_row: usize,
    /// the sidebar's search text, and how far its list is scrolled
    query: String,
    side_scroll: Spring,
    /// the sidebar's selection highlight (top edge in list coordinates, so it moves with the list) and, per page, how
    /// selected / hovered its row is (0..1)
    side_hl: Option<Spring>,
    side_t: Vec<[f32; 2]>,
    side_max: f32,
    side_drag: Option<f32>,
    /// the pointer is over the sidebar (the wheel scrolls it instead of the page)
    over_side: bool,
    /// how far the page is scrolled (it glides), and the most it can be
    scroll: Spring,
    max_scroll: f32,
    /// the scroll bar's thumb is being dragged: where in the thumb it was taken
    sb_drag: Option<f32>,
    /// Glass page: 0 = windows & menus, then one entry per glass panel
    glass_target: usize,
    panels: PanelsState,
    /// the custom accent colour editor is open
    accent_open: bool,
    /// what the last "take from the GTK theme" found
    gtk_note: String,
    anim_page: Option<Page>,
    /// Displays page: the monitor being edited
    display_sel: usize,
    /// Themes page: the name to save under and the file to import
    share: (String, String),
    /// the accent colour as hue / saturation / value while it is being edited (a grey has no hue to remember)
    accent_hsv: Option<[f32; 3]>,
    /// Programs page: the rules per program (`app-rules.toml`) and the name being typed for a new one
    rules: crate::apprules::Rules,
    rule_name: String,
    /// glass samples of the Glass page: they must be drawn in the shell layer, above the window, to refract the
    /// backdrop the page paints. (panels, labels)
    pub preview_shell: (Vec<Panel>, Vec<Label>),
}

/// What the Panels page has selected.
#[derive(Default)]
struct PanelsState {
    panel: usize,
    /// (zone, index) of the selected widget
    sel: Option<(usize, usize)>,
    /// zone the add-widget picker is open for
    add_zone: Option<usize>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { open: false, page: Page::Appearance, ui: State::default(), wall_row: 0, query: String::new(), side_scroll: Spring::new(0.0, 420.0, 1.0), side_hl: None, side_t: Vec::new(), side_max: 0.0, side_drag: None, over_side: false, scroll: Spring::new(0.0, 420.0, 1.0), max_scroll: 0.0, sb_drag: None, glass_target: 0, panels: PanelsState::default(), accent_hsv: None, accent_open: false, gtk_note: String::new(), anim_page: None, display_sel: 0, share: Default::default(), rules: crate::apprules::Rules::load(), rule_name: String::new(), preview_shell: (Vec::new(), Vec::new()) }
    }
}

impl Settings {
    pub fn visible(&self) -> bool {
        self.open
    }

    /// Index of a page by its name ("glass", "panels", ...).
    /// The pointer shape the last frame's controls asked for (a text beam over a field, a hand over a button).
    pub fn cursor_hint(&self) -> chebrium::ui::Cursor {
        self.ui.cursor
    }

    pub fn page_by_name(name: &str) -> Option<usize> {
        PAGES.iter().position(|(_, n, _)| n.eq_ignore_ascii_case(name))
    }

    /// Jump to a page by index (debug / deep links).
    pub fn set_page(&mut self, i: usize) {
        self.page = PAGES[i.min(PAGES.len() - 1)].0;
    }

    pub fn open(&mut self) {
        self.open = true;
        // the file may have been edited by hand since
        self.rules = crate::apprules::Rules::load();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.ui.pointer_up();
    }

    pub fn set_mouse(&mut self, p: [f32; 2]) {
        self.ui.pointer_move(p);
    }

    /// A press inside the content area (the compositor has already handled the window chrome).
    pub fn mouse_down(&mut self, p: [f32; 2]) {
        self.ui.pointer_down(p);
    }

    pub fn mouse_up(&mut self) {
        self.ui.pointer_up();
    }

    /// A key press while a text field is being edited. True when it was used.
    pub fn key(&mut self, k: Key, ctrl: bool, alt: bool) -> bool {
        self.ui.key(k, chebrium::input::KeyMods { ctrl, alt, shift: false })
    }

    /// Mouse wheel over the content: page through the wallpaper rows.
    pub fn scroll(&mut self, dy: f64) {
        if self.over_side {
            self.side_scroll.target = (self.side_scroll.target + dy as f32 * 2.5).clamp(0.0, self.side_max);
            return;
        }
        if self.page != Page::Wallpaper {
            self.scroll.target = (self.scroll.target + dy as f32 * 2.5).clamp(0.0, self.max_scroll);
            return;
        }
        if self.page == Page::Wallpaper {
            if dy > 0.0 {
                self.wall_row += 1;
            } else if dy < 0.0 && self.wall_row > 0 {
                self.wall_row -= 1;
            }
        }
    }

    /// Draw the content for a client area whose top-left is `origin` (world coordinates).
    pub fn draw(&mut self, ctx: &mut SettingsCtx, origin: [f32; 2]) -> (Vec<Panel>, Vec<Label>, Vec<SettingsAction>) {
        let mut p = Vec::new();
        let mut l = Vec::new();
        let mut actions = Vec::new();
        if !self.open {
            self.ui.click = false;
            return (p, l, actions);
        }
        self.preview_shell = (Vec::new(), Vec::new());
        let mut shell_p: Vec<Panel> = Vec::new();
        let mut shell_l: Vec<Label> = Vec::new();
        let mut glass_prev: Option<(theme::GlassStyle, f32)> = None;
        let mut icon_prev: Option<[f32; 4]> = None;
        let (x, y) = (origin[0], origin[1]);
        let (win_w, win_h) = (ctx.size[0].max(WIN_W * 0.7), ctx.size[1].max(WIN_H * 0.7));

        let dark = ctx.theme.dark;
        let ink = if dark { [0.95, 0.96, 0.99] } else { [0.10, 0.12, 0.18] };
        let accent = ctx.theme.accent;
        // client background: frosted glass, calmer and more opaque than panels so text stays readable
        let mut bg = ctx.theme.glass.panel(x + win_w / 2.0, y + win_h / 2.0, win_w, win_h, 0.0);
        bg.fx[0] = bg.fx[0].max(18.0);
        bg.extra[0] = 0.0;
        bg.params[3] = 0.0;
        let op = ctx.theme.window.client_opacity;
        bg.tint = if dark { [0.129, 0.129, 0.129, (op - 0.06).max(0.0)] } else { [0.97, 0.98, 1.0, (op - 0.02).max(0.0)] };
        bg.color2 = [0.0; 4];
        p.push(bg);
        // the page itself is opaque; only the sidebar lets the glass show through
        let page = if dark { [0.12, 0.12, 0.12] } else { [0.97, 0.98, 1.0] };
        let (pw, ph) = (win_w - SIDEBAR, win_h);
        p.push(Panel::solid(x + SIDEBAR + pw / 2.0, y + ph / 2.0, pw, ph, 0.0, page, page).flat());

        // animations run on real time; a page that has just opened shows its controls where they belong at once
        let dt = self.ui.begin_frame();
        if self.anim_page != Some(self.page) {
            self.ui.anims.clear();
            self.anim_page = Some(self.page);
        }
        let mut painter = PanelDraw::new(&mut p, &mut l);
        let mut ui = Ui::new(&mut painter, &mut self.ui, Style::new(dark, accent).with_controls(ctx.theme.ui.controls.to_chebrium()), dt);

        // sidebar
        let (pitch, glyph) = match ctx.theme.ui.sidebar {
            theme::SidebarSize::Small => (36.0, 14.0),
            theme::SidebarSize::Medium => (40.0, 16.0),
            theme::SidebarSize::Large => (46.0, 19.0),
        };
        // search: pages whose name or settings match
        let sf = [x + 14.0, y + 12.0, SIDEBAR - 28.0, 30.0];
        let mut q = std::mem::take(&mut self.query);
        ui.text_field(sf, "settings-search", &mut q);
        if q.is_empty() && !ui.editing() {
            ui.glyph(sf[0] + 16.0, sf[1] + 15.0, 14.0, "search", ink, 0.5);
            ui.text(sf[0] + 32.0, sf[1] + 20.0, "Search", 13.0, false, ink, 0.5, Align::Left);
        }
        self.query = q;
        let needle = self.query.trim().to_lowercase();
        let shown: Vec<&(Page, &str, &str)> = PAGES
            .iter()
            .filter(|(pg, name, _)| needle.is_empty() || name.to_lowercase().contains(&needle) || crate::i18n::tr(name).to_lowercase().contains(&needle) || page_keywords(*pg).contains(&needle))
            .collect();
        let list_top = y + 52.0;
        let list_bot = y + win_h - 12.0;
        self.over_side = ui.mouse[0] >= x && ui.mouse[0] < x + SIDEBAR;
        self.side_scroll.update(dt);
        let side_mark = ui.d.mark();
        // the pages in groups (a flat list while searching)
        const CATEGORIES: [(&str, &[Page]); 5] = [
            ("LOOK", &[Page::Themes, Page::Appearance, Page::Glass, Page::Wallpaper, Page::Icons, Page::Programs]),
            ("DESKTOP", &[Page::Layout, Page::Panels, Page::Notifications, Page::DateTime]),
            ("WINDOWS", &[Page::Windows, Page::Animations]),
            ("DEVICES", &[Page::Displays, Page::Sound, Page::Network, Page::Power, Page::Mouse, Page::Keys]),
            ("SYSTEM", &[Page::General, Page::About]),
        ];
        let mut rows: Vec<(Option<&str>, Option<&(Page, &str, &str)>)> = Vec::new();
        if needle.is_empty() {
            for (cat, pages) in CATEGORIES.iter() {
                rows.push((Some(cat), None));
                for pg in pages.iter() {
                    if let Some(e) = shown.iter().find(|e| e.0 == *pg) {
                        rows.push((None, Some(*e)));
                    }
                }
            }
        } else {
            rows.extend(shown.iter().map(|e| (None, Some(*e))));
        }
        let mut cy = list_top - self.side_scroll.x;
        if self.side_t.len() != PAGES.len() {
            self.side_t = PAGES.iter().map(|(pg, ..)| [(*pg == self.page) as u8 as f32, 0.0]).collect();
        }
        let glide = 1.0 - (-14.0 * dt).exp();
        let mut hl_seen = false;
        for (head, entry) in rows {
            if let Some(h) = head {
                cy += 8.0;
                ui.section(x + 22.0, cy + 12.0, h);
                cy += 24.0;
                continue;
            }
            let Some((page, name, icon)) = entry.copied() else { continue };
            let r = [x + 14.0, cy, SIDEBAR - 28.0, pitch - 6.0];
            cy += pitch;
            let selected = page == self.page;
            let visible = r[1] >= list_top - 2.0 && r[1] + r[3] <= list_bot + 2.0;
            let idx = PAGES.iter().position(|e| e.0 == page).unwrap_or(0);
            let want = [selected as u8 as f32, (!selected && visible && ui.hit(r)) as u8 as f32];
            let t = &mut self.side_t[idx];
            for k in 0..2 {
                t[k] += (want[k] - t[k]) * glide;
                if (want[k] - t[k]).abs() < 0.003 {
                    t[k] = want[k];
                }
            }
            let [sel_t, hover_t] = *t;
            if selected {
                // one highlight that slides from the row it was on to this one
                let at = r[1] + self.side_scroll.x;
                let hl = self.side_hl.get_or_insert_with(|| Spring::new(at, 520.0, 1.0));
                hl.target = at;
                hl.update(dt);
                let top = hl.x - self.side_scroll.x;
                ui.fill([r[0], top, r[2], r[3]], 9.0, accent, 0.20);
                ui.fill([r[0], top + 8.0, 3.0, r[3] - 16.0], 1.5, accent, 1.0);
                hl_seen = true;
            }
            if hover_t > 0.0 {
                ui.fill(r, 9.0, ink, 0.07 * hover_t);
            }
            // with several accents each page has the colour of its own kind of thing
            let own = ui.tone(page_tone(page));
            let multi = ui.controls.multi;
            let tone = if multi { own.map(|c| c * 0.92) } else { [0, 1, 2].map(|c| ink[c] + (accent[c] - ink[c]) * sel_t) };
            if multi {
                // a pale plate under the coloured sign, so it reads on a dark sidebar too
                let size = glyph + 11.0;
                let plate = [r[0] + 22.0 - size / 2.0, r[1] + r[3] / 2.0 - size / 2.0, size, size];
                ui.fill([plate[0], plate[1] + 1.5, size, size], 7.0, [0.0; 3], if dark { 0.35 } else { 0.14 });
                ui.grad(plate, 7.0, [1.0, 1.0, 1.0], [0.88, 0.91, 0.96], 1.0);
            }
            ui.glyph(r[0] + 22.0, r[1] + r[3] / 2.0, glyph, icon, tone, if multi { 1.0 } else { 0.8 + 0.2 * sel_t });
            ui.text(r[0] + 44.0, r[1] + r[3] / 2.0 + 5.0, name, 13.5, selected, ink, 0.95, Align::Left);
            if ui.click && visible && ui.hit(r) && self.page != page {
                self.page = page;
                self.scroll.x = 0.0;
                self.scroll.v = 0.0;
                self.scroll.target = 0.0;
            }
        }
        if !hl_seen {
            // the selected page is not listed (a search): the highlight starts afresh when it comes back
            self.side_hl = None;
        }
        if shown.is_empty() {
            ui.text(x + SIDEBAR / 2.0, list_top + 30.0, "No matches", 12.5, false, ink, 0.5, Align::Center);
        }
        let list_bottom = ui.d.cull(side_mark, list_top - 4.0, list_bot);
        self.side_max = if shown.is_empty() { 0.0 } else { (list_bottom + self.side_scroll.x - list_bot).max(0.0) };
        if self.side_scroll.target > self.side_max {
            self.side_scroll.target = self.side_max;
        }
        if self.side_max > 1.0 {
            let track = [x + SIDEBAR - 9.0, list_top, 5.0, list_bot - list_top];
            let frac = (list_bot - list_top) / (list_bot - list_top + self.side_max);
            let th = (track[3] * frac).max(30.0);
            let ty = track[1] + (track[3] - th) * (self.side_scroll.x / self.side_max).clamp(0.0, 1.0);
            let hot = ui.hit([track[0] - 6.0, track[1], track[2] + 10.0, track[3]]);
            if ui.click && hot {
                self.side_drag = Some(if ui.hit([track[0], ty, track[2], th]) { ui.mouse[1] - ty } else { th / 2.0 });
            }
            if !ui.held {
                self.side_drag = None;
            }
            if let Some(off) = self.side_drag {
                let t = ((ui.mouse[1] - off - track[1]) / (track[3] - th).max(1.0)).clamp(0.0, 1.0);
                self.side_scroll.target = t * self.side_max;
                self.side_scroll.x = self.side_scroll.target;
                self.side_scroll.v = 0.0;
            }
            ui.fill([track[0], ty, track[2], th], 2.5, ink, if self.side_drag.is_some() { 0.5 } else if hot { 0.38 } else { 0.24 });
        }
        // divider between the sidebar and the page
        ui.fill([x + SIDEBAR, y + 20.0, 1.0, win_h - 40.0], 0.5, ink, 0.10);

        // the page keeps a readable width: a wide window has empty space on both sides of it
        let avail = win_w - SIDEBAR - 56.0;
        let cw = avail.min(780.0);
        let cx0 = x + SIDEBAR + 28.0 + (avail - cw) / 2.0;
        let scrolls = self.page != Page::Wallpaper;
        self.scroll.update(dt);
        let sy = if scrolls { self.scroll.x } else { 0.0 };
        let (view_top, view_bot) = (y, y + win_h - 4.0);
        let mark = ui.d.mark();
        // (room above the page: a maximized window starts right under the menu bar)
        const PAGE_TOP: f32 = 18.0;
        let py = y - sy + PAGE_TOP;
        match self.page {
            Page::General => general::page_general(&mut ui, ctx, cx0, py - 26.0, cw, &mut actions),
            Page::Themes => general::page_themes(&mut ui, ctx, cx0, py - 26.0, cw, &mut actions, &mut self.share),
            Page::Appearance => general::page_appearance(&mut ui, ctx, cx0, py - 26.0, cw, &mut self.accent_hsv, &mut self.accent_open, &mut self.gtk_note),
            Page::Displays => general::page_displays(&mut ui, ctx, cx0, py - 26.0, cw, &mut actions, &mut self.display_sel),
            Page::Keys => general::page_keys(&mut ui, ctx, cx0, py - 26.0, cw),
            Page::Notifications => more::page_notifications(&mut ui, ctx, cx0, py - 26.0, cw, &mut actions),
            Page::DateTime => more::page_datetime(&mut ui, ctx, cx0, py - 26.0, cw),
            Page::Sound => more::page_sound(&mut ui, ctx, cx0, py - 26.0, cw, &mut actions),
            Page::Network => more::page_network(&mut ui, ctx, cx0, py - 26.0, cw, &mut actions),
            Page::Power => more::page_power(&mut ui, ctx, cx0, py - 26.0, cw, &mut actions),
            Page::Programs => more::page_programs(&mut ui, ctx, cx0, py - 26.0, cw, &mut self.rules, &mut self.rule_name),
            Page::Mouse => general::page_mouse(&mut ui, ctx, cx0, py - 26.0, cw),
            Page::Glass => glass_prev = Some(page_glass(&mut ui, ctx, cx0, py - 26.0, cw, &mut self.glass_target)),
            Page::Panels => page_panels(&mut ui, ctx, cx0, py - 26.0, cw, &mut self.panels),
            Page::Wallpaper => {
                let rows = (ctx.walls.len() + 3) / 4;
                self.wall_row = self.wall_row.min(rows.saturating_sub(4));
                page_wallpaper(&mut ui, ctx, cx0, py - 26.0, cw, self.wall_row, &mut actions)
            }
            Page::Layout => page_layout(&mut ui, ctx, cx0, py - 26.0, cw),
            Page::Windows => page_windows(&mut ui, ctx, cx0, py - 26.0, cw),
            Page::Animations => page_animations(&mut ui, ctx, cx0, py - 26.0, cw),
            Page::Icons => icon_prev = Some(page_icons(&mut ui, ctx, cx0, py - 26.0, cw)),
            Page::About => page_about(&mut ui, cx0, py - 26.0),
        }
        // the Glass page's sample sits in the page, below the controls: its backdrop scrolls (and is cut) with the page
        // like everything else, and the glass shapes, which live in the shell layer, are cut to the page's view by hand
        let mut sample_bottom = f32::MIN;
        if let Some((style, py)) = &glass_prev {
            let from = (shell_p.len(), shell_l.len());
            glass_preview(&mut ui, [cx0, *py, cw, 250.0], style, &mut shell_p, &mut shell_l);
            sample_bottom = *py + 250.0;
            let view = [x + SIDEBAR, view_top, win_w - SIDEBAR, view_bot - view_top];
            cut_shell(&mut shell_p, &mut shell_l, from, view);
        }
        if let Some(r) = icon_prev {
            let from = (shell_p.len(), shell_l.len());
            icon_preview(&mut ui, ctx, r, &mut shell_p, &mut shell_l);
            sample_bottom = sample_bottom.max(r[1] + r[3]);
            cut_shell(&mut shell_p, &mut shell_l, from, [x + SIDEBAR, view_top, win_w - SIDEBAR, view_bot - view_top]);
        }
        if scrolls {
            let bottom = ui.d.cull(mark, view_top, view_bot).max(sample_bottom);
            // how far the page could still go: what hangs below the window now, plus a margin
            self.max_scroll = (bottom + 24.0 - view_bot).max(0.0) + self.scroll.x;
            if self.scroll.target > self.max_scroll {
                self.scroll.target = self.max_scroll;
            }
            if self.max_scroll > 1.0 {
                // the scroll bar: a thumb along the right edge of the window, dragged or clicked
                let track = [x + win_w - 14.0, view_top + 4.0, 8.0, view_bot - view_top - 8.0];
                let frac = (view_bot - view_top) / (view_bot - view_top + self.max_scroll);
                let th = (track[3] * frac).max(36.0);
                let ty = track[1] + (track[3] - th) * (self.scroll.x / self.max_scroll).clamp(0.0, 1.0);
                let thumb = [track[0], ty, track[2], th];
                let hot = ui.hit([track[0] - 6.0, track[1], track[2] + 12.0, track[3]]);
                if ui.click && hot {
                    self.sb_drag = Some(if ui.hit(thumb) { ui.mouse[1] - ty } else { th / 2.0 });
                }
                if !ui.held {
                    self.sb_drag = None;
                }
                if let Some(off) = self.sb_drag {
                    let t = ((ui.mouse[1] - off - track[1]) / (track[3] - th).max(1.0)).clamp(0.0, 1.0);
                    self.scroll.target = t * self.max_scroll;
                    self.scroll.x = self.scroll.target;
                    self.scroll.v = 0.0;
                }
                ui.fill(track, 4.0, ink, if hot { 0.08 } else { 0.0 });
                ui.fill(thumb, 4.0, ink, if self.sb_drag.is_some() { 0.55 } else if hot { 0.42 } else { 0.28 });
            }
        } else {
            self.max_scroll = 0.0;
        }
        let edit_seen = ui.edit_seen;
        let changed = ui.changed;
        if changed {
            actions.push(SettingsAction::ThemeChanged);
        }
        self.ui.end_frame(edit_seen);
        self.preview_shell = (shell_p, shell_l);
        (p, l, actions)
    }
}

/// Cut what a preview added to the shell layer (from `from` on) to `view`, so it slides under the page's edges like the
/// rest of the page instead of staying whole or vanishing.
fn cut_shell(sp: &mut Vec<Panel>, sl: &mut [Label], from: (usize, usize), view: [f32; 4]) {
    let mut new = sp.split_off(from.0);
    new.retain(|p| p.rect[1] + p.rect[3] > view[1] && p.rect[1] - p.rect[3] < view[1] + view[3]);
    sp.extend(new.into_iter().map(|p| p.cut_to(view)));
    for l in &mut sl[from.1..] {
        l.clip = Some(view);
    }
}

// ---------------------------------------------------------------------------- pages

/// A small live picture of the desktop the current settings produce (shown under the sidebar on every page).
fn theme_preview(ui: &mut Ui, r: [f32; 4], t: &Theme) {
    let (x, y, w, h) = (r[0], r[1], r[2], r[3]);
    let dark = t.dark;
    if dark {
        ui.grad(r, 9.0, [0.20, 0.26, 0.44], [0.05, 0.08, 0.18], 1.0);
    } else {
        ui.grad(r, 9.0, [0.62, 0.82, 0.98], [0.28, 0.52, 0.88], 1.0);
    }
    // a window with the chosen title bar
    let (wx, wy, ww, wh) = (x + w * 0.16, y + h * 0.28, w * 0.52, h * 0.40);
    let rad = (t.window.radius * 0.18).clamp(1.0, 5.0);
    let body = if dark { [0.189, 0.189, 0.189] } else { [0.97, 0.98, 1.0] };
    let bar = if dark { [0.258, 0.258, 0.258] } else { [0.86, 0.89, 0.95] };
    ui.fill([wx, wy, ww, wh], rad, body, 0.96);
    let tb = (t.window.titlebar * 0.14).clamp(5.0, 9.0);
    match t.window.decoration {
        DecoKind::Mac => {
            ui.fill([wx, wy, ww, tb], rad, bar, 0.95);
            for (i, c) in [[1.0, 0.37, 0.34], [1.0, 0.74, 0.18], [0.16, 0.78, 0.25]].iter().enumerate() {
                ui.fill([wx + 3.0 + i as f32 * 5.0, wy + tb / 2.0 - 1.5, 3.0, 3.0], 1.5, *c, 1.0);
            }
        }
        DecoKind::Win11 => {
            ui.fill([wx, wy, ww, tb], rad, bar, 0.95);
            for i in 0..3 {
                ui.fill([wx + ww - 17.0 + i as f32 * 5.5, wy + tb / 2.0 - 1.5, 3.0, 3.0], 0.5, if dark { [0.8; 3] } else { [0.3; 3] }, 0.8);
            }
        }
        DecoKind::Gnome => {
            ui.fill([wx, wy, ww, tb], rad, bar, 0.95);
            for i in 0..3 {
                ui.fill([wx + ww - 18.0 + i as f32 * 5.5 - 5.5, wy + tb / 2.0 - 1.8, 3.6, 3.6], 1.8, if dark { [0.85; 3] } else { [0.3; 3] }, 0.7);
            }
        }
        DecoKind::Aero => {
            ui.grad([wx - 1.0, wy - 1.0, ww + 2.0, tb + 2.0], rad, [0.55, 0.75, 0.96], [0.30, 0.52, 0.82], 0.95);
            ui.fill([wx + ww - 9.0, wy, 8.0, tb - 1.0], 1.5, [0.86, 0.28, 0.22], 1.0);
        }
        DecoKind::Minimal => {}
        DecoKind::Custom => {
            use theme::{BarMaterial, ButtonColors, ButtonSide};
            let fs = &t.window.frame;
            match fs.bar {
                BarMaterial::Body => {}
                BarMaterial::Tone => ui.fill([wx, wy, ww, tb], rad, bar, 0.95),
                BarMaterial::Accent => ui.fill([wx, wy, ww, tb], rad, t.accent, 0.95),
                BarMaterial::Glass => ui.grad([wx - 1.0, wy - 1.0, ww + 2.0, tb + 2.0], rad, [0.55, 0.75, 0.96], [0.30, 0.52, 0.82], 0.95),
            }
            let step = 5.0 + fs.gap * 0.2;
            for i in 0..3 {
                let bx = if fs.side == ButtonSide::Left { wx + 3.0 + i as f32 * step } else { wx + ww - 4.0 - i as f32 * step - 3.0 };
                let c = match (fs.colors, i) {
                    (ButtonColors::Traffic, 0) => [1.0, 0.37, 0.34],
                    (ButtonColors::Traffic, 1) => [1.0, 0.74, 0.18],
                    (ButtonColors::Traffic, _) => [0.16, 0.78, 0.25],
                    (ButtonColors::Accent, _) => t.accent,
                    (ButtonColors::CloseRed, 0) => [0.86, 0.30, 0.25],
                    _ => if dark { [0.8; 3] } else { [0.3; 3] },
                };
                ui.fill([bx, wy + tb / 2.0 - 1.5, 3.0, 3.0], if fs.shape == theme::ButtonShape::Circle { 1.5 } else { 0.5 }, c, 0.9);
            }
        }
    }
    // content: a selected row in the accent colour and a toggle
    ui.fill([wx + 4.0, wy + tb + 4.0, ww * 0.55, 4.0], 2.0, t.accent, 0.9);
    ui.fill([wx + 4.0, wy + tb + 11.0, ww * 0.38, 3.0], 1.5, if dark { [0.7; 3] } else { [0.5; 3] }, 0.5);
    ui.fill([wx + ww - 14.0, wy + tb + 5.0, 9.0, 5.0], 2.5, t.accent, 1.0);

    for bar in &t.bars {
        bar_preview(ui, r, t, bar);
    }
}

/// One panel of the theme as a tiny picture: its band (or floating pill) and a mark for each widget.
fn bar_preview(ui: &mut Ui, r: [f32; 4], t: &Theme, b: &BarConfig) {
    let (x, y, w, h) = (r[0], r[1], r[2], r[3]);
    let dark = t.dark;
    let glass = if dark { [0.13, 0.13, 0.13] } else { [1.0; 3] };
    let ink = if dark { [0.92; 3] } else { [0.15, 0.18, 0.25] };
    let top = b.edge == Edge::Top;
    // width of each widget's mark, in picture pixels
    let mark_w = |kind: &str| -> f32 {
        match kind {
            "start" => 11.0,
            "search-box" => 21.0,
            "tasks" => 36.0,
            "tray" => 20.0,
            "clock" => 16.0,
            "title" => 16.0,
            "app-menu" => 28.0,
            "dock" => 0.0,
            "spacer" | "divider" => 4.0,
            _ => 7.0,
        }
    };
    let zones = [&b.left, &b.center, &b.right];
    let zone_w = |z: usize| -> f32 { zones[z].iter().map(|s| mark_w(&s.kind)).sum() };
    if b.shape == BarShape::Floating {
        // the Dock: a pill of app icons
        let n = 7.0;
        let s = (t.dock.icon_size / 64.0 * 9.0).clamp(6.0, 12.0);
        let dw = n * (s + 3.0) + 7.0;
        let py = if top { y + 4.0 } else { y + h - s - 9.0 };
        ui.fill([x + (w - dw) / 2.0, py, dw, s + 7.0], (t.dock.radius * 0.25).clamp(3.0, 9.0), glass, 0.45);
        for i in 0..7 {
            ui.fill([x + (w - dw) / 2.0 + 5.0 + i as f32 * (s + 3.0), py + 3.0, s, s], s * 0.25, [0.30 + 0.09 * i as f32, 0.62, 0.95 - 0.09 * i as f32], 0.95);
        }
        return;
    }
    // the band
    let height = b.height.unwrap_or(t.menubar.height);
    let bh = (height * 0.2).clamp(6.0, 13.0);
    let by = if top { y } else { y + h - bh };
    if b.surface == BarSurface::Glass {
        ui.fill([x, by, w, bh], b.radius * 0.2, glass, 0.82);
    } else {
        let mb = b.look.as_ref().unwrap_or(&t.menubar);
        if mb.bar {
            match mb.kind {
                MenubarKind::Glass => ui.fill([x, by, w, bh], 0.0, glass, 0.18 + mb.opacity * 0.8),
                MenubarKind::Solid => ui.fill([x, by, w, bh], 0.0, glass, mb.opacity.max(0.3)),
                MenubarKind::Fade => {
                    for i in 0..5 {
                        let f = 1.0 - i as f32 / 5.0;
                        ui.fill([x, by + i as f32 * bh * 0.5, w, bh * 0.5], 0.0, glass, (0.15 + mb.opacity * 0.7) * f * f);
                    }
                }
                MenubarKind::Clear => {}
            }
        }
    }
    // widget marks, zone by zone
    let starts = [x + 4.0, x + (w - zone_w(1)) / 2.0, x + w - 4.0 - zone_w(2)];
    let cy = by + bh / 2.0;
    for (z, specs) in zones.iter().enumerate() {
        let mut mx = starts[z];
        for sp in specs.iter() {
            let kw = mark_w(&sp.kind);
            match sp.kind.as_str() {
                "start" => {
                    if sp.str("style") == Some("orb") {
                        ui.fill([mx, cy - 4.0, 8.0, 8.0], 4.0, [0.25, 0.6, 1.0], 1.0);
                    } else {
                        ui.fill([mx, cy - 3.0, 6.0, 6.0], 1.0, t.accent, 1.0);
                    }
                }
                "search-box" => ui.fill([mx, cy - 3.0, 18.0, 6.0], 3.0, ink, 0.18),
                "tasks" => {
                    for i in 0..4 {
                        ui.fill([mx + i as f32 * 9.0, cy - 3.0, 6.0, 6.0], 1.5, [0.30 + 0.12 * i as f32, 0.6, 0.9 - 0.1 * i as f32], 0.95);
                    }
                }
                "tray" => {
                    for i in 0..3 {
                        ui.fill([mx + i as f32 * 6.0, cy - 1.5, 3.0, 3.0], 1.5, ink, 0.6);
                    }
                }
                "clock" => ui.fill([mx, cy - 1.0, 14.0, 2.0], 1.0, ink, 0.6),
                "title" | "app-menu" => ui.fill([mx, cy - 1.0, kw - 3.0, 2.0], 1.0, ink, 0.6),
                "spacer" | "divider" | "dock" => {}
                _ => ui.fill([mx + 1.0, cy - 1.5, 3.0, 3.0], 1.5, ink, 0.7),
            }
            mx += kw;
        }
    }
}

fn mini_preview(ui: &mut Ui, r: [f32; 4], preset: &str) {
    // sky background
    ui.grad(r, 8.0, [0.62, 0.82, 0.98], [0.30, 0.55, 0.88], 1.0);
    let (x, y, w, h) = (r[0], r[1], r[2], r[3]);
    // a little window
    ui.fill([x + w * 0.20, y + h * 0.22, w * 0.46, h * 0.40], 3.0, [1.0; 3], 0.92);
    ui.fill([x + w * 0.20, y + h * 0.22, w * 0.46, h * 0.08], 3.0, [0.8, 0.84, 0.92], 0.9);
    match preset {
        "windows11" => {
            ui.fill([x, y + h - 10.0, w, 10.0], 0.0, [0.96, 0.97, 0.99], 0.92);
            for i in 0..5 {
                ui.fill([x + w * 0.5 - 28.0 + i as f32 * 12.0, y + h - 8.0, 7.0, 6.0], 2.0, if i == 0 { [0.0, 0.47, 0.84] } else { [0.4, 0.45, 0.55] }, 0.9);
            }
        }
        "unity" => {
            // the panel on top and the Launcher down the left side
            ui.fill([x, y, w, 7.0], 0.0, [0.17, 0.04, 0.12], 0.95);
            ui.fill([x, y + 7.0, 13.0, h - 7.0], 0.0, [0.20, 0.05, 0.15], 0.85);
            for i in 0..4 {
                ui.fill([x + 3.0, y + 11.0 + i as f32 * 10.0, 7.0, 7.0], 2.0, [0.91, 0.33 + 0.1 * i as f32, 0.13 + 0.15 * i as f32], 0.95);
            }
        }
        "windows7" => {
            ui.grad([x, y + h - 11.0, w, 11.0], 0.0, [0.45, 0.65, 0.92], [0.18, 0.36, 0.65], 0.95);
            ui.fill([x + 2.0, y + h - 10.0, 9.0, 9.0], 4.5, [0.25, 0.6, 1.0], 1.0);
            for i in 0..4 {
                ui.fill([x + 16.0 + i as f32 * 11.0, y + h - 9.0, 8.0, 7.0], 1.5, [1.0; 3], 0.55);
            }
        }
        _ => {
            ui.fill([x, y, w, 6.0], 0.0, [1.0; 3], 0.30);
            ui.fill([x + w * 0.5 - 30.0, y + h - 15.0, 60.0, 11.0], 5.5, [1.0; 3], 0.5);
            for i in 0..5 {
                ui.fill([x + w * 0.5 - 24.0 + i as f32 * 10.0, y + h - 14.0, 8.0, 8.0], 2.5, [0.3 + 0.1 * i as f32, 0.6, 0.95 - 0.1 * i as f32], 0.95);
            }
        }
    }
}

/// Pick any accent colour: hue, saturation and brightness sliders whose tracks show the result, a field for a hex code,
/// and (when the wallpaper has been looked at) a button that takes the colour from it.
fn accent_custom(ui: &mut Ui, ctx: &mut SettingsCtx, hsv: &mut Option<[f32; 3]>, x: f32, y: f32, cw: f32) {
    use theme::{from_hex, hsv_to_rgb, rgb_to_hsv, to_hex};
    // keep the hue of a grey, and follow the colour when a swatch or the wallpaper changed it from outside
    let mut cur = match *hsv {
        Some(h) if {
            let rgb = hsv_to_rgb(h[0], h[1], h[2]);
            (0..3).all(|i| (rgb[i] - ctx.theme.accent[i]).abs() < 0.004)
        } => h,
        _ => rgb_to_hsv(ctx.theme.accent),
    };
    ui.row(x, y, "Custom colour");
    // the colour now, and its hex code
    let now = hsv_to_rgb(cur[0], cur[1], cur[2]);
    ui.fill([x + 150.0, y - 13.0, 26.0, 26.0], 13.0, now, 1.0);
    let mut hex = to_hex(now);
    if ui.text_field([x + 186.0, y - 13.0, 104.0, 26.0], "accent-hex", &mut hex) {
        if let Some(c) = from_hex(&hex) {
            ctx.theme.accent = c;
            cur = rgb_to_hsv(c);
        }
    }
    if let Some(w) = ctx.wall_accent {
        let r = [x + cw - 190.0, y - 14.0, 190.0, 28.0];
        ui.fill([r[0] + 6.0, r[1] + 5.0, 18.0, 18.0], 9.0, w, 1.0);
        if ui.button([r[0] + 30.0, r[1], r[2] - 30.0, r[3]], "From wallpaper", false) {
            ctx.theme.accent = w;
            cur = rgb_to_hsv(w);
            ui.changed = true;
        }
    }
    let track = |yy: f32| [x + 150.0, yy - 6.0, cw - 150.0 - 8.0, 12.0];
    let before = cur;
    ui.row(x, y + 40.0, "Hue");
    ui.gradient_slider(track(y + 40.0), &mut cur[0], &|t| hsv_to_rgb(t, 1.0, 1.0));
    ui.row(x, y + 74.0, "Saturation");
    let (h0, v0) = (cur[0], cur[2]);
    ui.gradient_slider(track(y + 74.0), &mut cur[1], &|t| hsv_to_rgb(h0, t, v0.max(0.5)));
    ui.row(x, y + 108.0, "Brightness");
    let (h1, s1) = (cur[0], cur[1]);
    ui.gradient_slider(track(y + 108.0), &mut cur[2], &|t| hsv_to_rgb(h1, s1, t));
    if cur != before {
        ctx.theme.accent = hsv_to_rgb(cur[0], cur[1], cur[2]);
    }
    *hsv = Some(cur);
}

fn page_glass(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, target: &mut usize) -> (theme::GlassStyle, f32) {
    ui.text(x, y + 52.0, "Glass", 22.0, true, ui.ink, 0.97, Align::Left);
    // panels drawn as glass (taskbars) can have glass of their own, separate from windows and menus
    let glass_bars: Vec<usize> = ctx.theme.bars.iter().enumerate().filter(|(_, b)| b.surface == BarSurface::Glass).map(|(i, _)| i).collect();
    let mut dy = 0.0;
    if !glass_bars.is_empty() {
        let mut names = vec!["Windows & menus".to_string()];
        names.extend(glass_bars.iter().map(|&i| ctx.theme.bars[i].name.clone()));
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        ui.row(x, y + 86.0, "Edit");
        *target = (*target).min(names.len() - 1);
        ui.segmented([x + 150.0, y + 80.0, cw - 150.0, 30.0], &refs, target);
        dy = 44.0;
    } else {
        *target = 0;
    }
    // where the sample goes below the controls
    let below = |g: &theme::GlassStyle, base: f32| match g.mode {
        GlassMode::Flat => base + 190.0,
        GlassMode::Paper | GlassMode::Wood | GlassMode::Clay => base + 310.0,
        GlassMode::Glass => base + 230.0,
        GlassMode::Custom => base + 620.0,
    };
    if *target >= 1 {
        let bar = &mut ctx.theme.bars[glass_bars[*target - 1]];
        let mut same = bar.glass.is_none();
        ui.row(x, y + 86.0 + dy, "Same as windows");
        ui.toggle(x + cw - 44.0, y + 82.0 + dy, &mut same);
        if same != bar.glass.is_none() {
            bar.glass = if same { None } else { Some(ctx.theme.glass.clone()) };
        }
        dy += 40.0;
        match bar.glass.as_mut() {
            Some(g) => {
                glass_editor(ui, g, x, y + dy, cw);
                (g.clone(), below(g, y + dy))
            }
            None => {
                ui.text(x, y + 86.0 + dy, "This panel uses the glass set for windows and menus.", 12.5, false, ui.ink, 0.6, Align::Left);
                (ctx.theme.glass.clone(), y + dy + 120.0)
            }
        }
    } else {
        glass_editor(ui, &mut ctx.theme.glass, x, y + dy, cw);
        (ctx.theme.glass.clone(), below(&ctx.theme.glass, y + dy))
    }
}

/// A live sample of glass: a colourful backdrop painted into the window, and glass shapes in the style above it.
/// The glass has to refract the backdrop, so it goes into `sp` / `sl` (the shell layer, drawn over the window).
fn glass_preview(ui: &mut Ui, r: [f32; 4], g: &theme::GlassStyle, sp: &mut Vec<Panel>, sl: &mut Vec<Label>) {
    let s = (r[2] / 400.0).min(r[3] / 220.0);
    let (ox, oy) = (r[0] + (r[2] - 400.0 * s) / 2.0, r[1] + (r[3] - 220.0 * s) / 2.0);
    let rc = |x: f32, y: f32, w: f32, h: f32| [ox + x * s, oy + y * s, w * s, h * s];
    let big = s >= 0.7;
    // backdrop
    ui.grad(rc(0.0, 0.0, 400.0, 220.0), 16.0 * s, [0.16, 0.30, 0.80], [0.52, 0.22, 0.66], 1.0);
    for (cx, cy, d, c) in [(62.0, 56.0, 84.0, [1.0, 0.42, 0.58]), (336.0, 52.0, 64.0, [1.0, 0.86, 0.30]), (205.0, 150.0, 96.0, [0.20, 0.86, 0.96]), (358.0, 158.0, 62.0, [0.40, 0.90, 0.50]), (150.0, 36.0, 34.0, [1.0, 1.0, 1.0])] {
        ui.fill(rc(cx - d / 2.0, cy - d / 2.0, d, d), d * s / 2.0, c, 0.95);
    }
    for i in 0..9 {
        ui.fill(rc(0.0, 20.0 + i as f32 * 22.0, 400.0, 1.6), 0.8 * s, [1.0; 3], 0.35);
    }
    if big {
        ui.text(ox + 200.0 * s, oy + 212.0 * s, "Jelly Glass 0123 — Aa Bb Cc", 17.0 * s, true, [1.0; 3], 0.95, Align::Center);
    }
    // glass on top
    let dark_tile = [0.50, 0.55, 0.66, 0.26];
    let tile = |x: f32, y: f32, w: f32, h: f32, t: [f32; 4]| {
        let c = rc(x, y, w, h);
        g.panel(c[0] + c[2] / 2.0, c[1] + c[3] / 2.0, c[2], c[3], c[3] / 2.0).tinted(t).adaptive()
    };
    // a Wi-Fi style pill with its round badge
    sp.push(tile(26.0, 26.0, 196.0, 62.0, dark_tile));
    let b = rc(34.0, 33.0, 48.0, 48.0);
    sp.push(g.panel(b[0] + b[2] / 2.0, b[1] + b[3] / 2.0, b[2], b[3], b[2] / 2.0).tinted([0.30, 0.35, 0.46, 0.40]).shadow(0.0, 0.0, 0.0).adaptive());
    sp.push(Panel::glyph(b[0] + b[2] / 2.0, b[1] + b[3] / 2.0, 22.0 * s, "wifi-off", [1.0; 3], 0.95));
    if big {
        sl.push(Label::new("Wi-Fi", ox + 98.0 * s, oy + 54.0 * s, 13.0 * s, true, [1.0; 3], 1.0, Align::Left));
        sl.push(Label::new("Off", ox + 98.0 * s, oy + 72.0 * s, 11.0 * s, false, [1.0; 3], 0.7, Align::Left));
    }
    // a round button
    let c = rc(262.0, 22.0, 70.0, 70.0);
    sp.push(g.panel(c[0] + c[2] / 2.0, c[1] + c[3] / 2.0, c[2], c[3], c[2] / 2.0).tinted(dark_tile).adaptive());
    sp.push(Panel::glyph(c[0] + c[2] / 2.0, c[1] + c[3] / 2.0, 26.0 * s, "moon", [1.0; 3], 0.95));
    // a slider card
    sp.push(tile(26.0, 112.0, 348.0, 66.0, dark_tile));
    let tr = rc(70.0, 150.0, 270.0, 5.0);
    sp.push(Panel::solid(tr[0] + tr[2] / 2.0, tr[1] + tr[3] / 2.0, tr[2], tr[3], tr[3] / 2.0, [1.0; 3], [1.0; 3]).alpha(0.28));
    sp.push(Panel::solid(tr[0] + tr[2] * 0.3, tr[1] + tr[3] / 2.0, tr[2] * 0.6, tr[3], tr[3] / 2.0, [1.0; 3], [1.0; 3]).alpha(0.95));
    sp.push(Panel::glyph(rc(48.0, 152.0, 0.0, 0.0)[0], rc(48.0, 152.0, 0.0, 0.0)[1], 15.0 * s, "sun", [1.0; 3], 0.9));
    if big {
        sl.push(Label::new("Display", ox + 52.0 * s, oy + 132.0 * s, 12.5 * s, true, [1.0; 3], 1.0, Align::Left));
    }
}

fn glass_editor(ui: &mut Ui, g: &mut theme::GlassStyle, x: f32, y: f32, cw: f32) {
    let mut f = Form::new(x, y + 86.0, cw, 54.0);
    let mut mode = g.mode;
    if f.pick(ui, "Mode", &mut mode, &[(GlassMode::Flat, "Flat"), (GlassMode::Glass, "Glass"), (GlassMode::Clay, "Clay"), (GlassMode::Paper, "Paper"), (GlassMode::Wood, "Wood"), (GlassMode::Custom, "Custom")]) {
        g.set_mode(mode);
    }
    match g.mode {
        GlassMode::Flat => ui.text(x, y + 150.0, "Frosted panels with no refraction, edge light or gloss.", 12.5, false, ui.ink, 0.6, Align::Left),
        GlassMode::Glass => {
            let mut a = g.amount;
            f.slider(ui, "Glassiness", &mut a, 0.0, 1.0, "");
            if a != g.amount {
                g.amount = a;
                g.apply_mode();
            }
            ui.text(x, y + 190.0, "0 is flat, 1 is the full Jelly Glass look.", 12.5, false, ui.ink, 0.6, Align::Left);
        }
        GlassMode::Clay => {
            f.slider(ui, "Puffiness", &mut g.bezel, 6.0, 60.0, " px");
            f.slider(ui, "Texture", &mut g.dispersion, 0.0, 1.0, "");
            f.slider(ui, "Highlight", &mut g.rim, 0.0, 1.0, "");
            ui.text(x, y + 276.0, "Soft, opaque pastel clay: a puffy rim, a fingertip texture and a deep shadow.", 12.5, false, ui.ink, 0.6, Align::Left);
        }
        GlassMode::Paper => {
            f.slider(ui, "Torn edges", &mut g.blur, 0.0, 14.0, " px");
            f.slider(ui, "Grain", &mut g.dispersion, 0.0, 1.0, "");
            f.slider(ui, "Creases", &mut g.rim, 0.0, 1.0, "");
            ui.text(x, y + 276.0, "Sheets of paper: torn edges, fibres, creases and a soft shadow.", 12.5, false, ui.ink, 0.6, Align::Left);
        }
        GlassMode::Wood => {
            f.slider(ui, "Rings", &mut g.blur, 0.3, 3.0, "");
            f.slider(ui, "Grain", &mut g.dispersion, 0.0, 1.0, "");
            f.slider(ui, "Varnish", &mut g.gloss, 0.0, 1.0, "");
            ui.text(x, y + 276.0, "Wooden boards: grain, rings, knots and a varnished bevel.", 12.5, false, ui.ink, 0.6, Align::Left);
        }
        GlassMode::Custom => glass_custom(ui, g, x, y + 118.0, cw),
    }
}

fn glass_custom(ui: &mut Ui, g: &mut theme::GlassStyle, x: f32, y: f32, cw: f32) {
    let mut f = Form::new(x, y + 18.0, cw, 29.0);
    ui.section(x, y, "LIGHT & REFRACTION");
    f.slider(ui, "Blur", &mut g.blur, 0.0, 40.0, "");
    f.slider(ui, "Refraction", &mut g.refraction, 0.0, 200.0, "");
    f.slider(ui, "Edge width", &mut g.bezel, 0.0, 90.0, "");
    f.slider(ui, "Colour split", &mut g.dispersion, 0.0, 0.5, "");
    f.slider(ui, "Edge light", &mut g.rim, 0.0, 3.0, "");
    f.slider(ui, "Edge sharpness", &mut g.rim_sharp, 0.0, 1.0, "");
    f.slider(ui, "Gloss", &mut g.gloss, 0.0, 1.0, "");
    f.slider(ui, "Saturation", &mut g.saturation, 0.0, 2.5, "");
    ui.section(x, f.y + 8.0, "TINT & SHADE");
    f.skip(26.0);
    f.slider(ui, "Tint strength", &mut g.tint[3], 0.0, 0.9, "");
    // (the three colour sliders are in their own colours when the accent is several)
    use chebrium::style::Tone;
    ui.tinted(Tone::Danger, |ui| f.slider(ui, "Tint red", &mut g.tint[0], 0.0, 1.0, ""));
    ui.tinted(Tone::Success, |ui| f.slider(ui, "Tint green", &mut g.tint[1], 0.0, 1.0, ""));
    ui.tinted(Tone::Primary, |ui| f.slider(ui, "Tint blue", &mut g.tint[2], 0.0, 1.0, ""));
    f.slider(ui, "Adaptive shade", &mut g.adapt, 0.0, 1.0, "");
    f.slider(ui, "Border opacity", &mut g.border[3], 0.0, 1.0, "");
    f.slider(ui, "Shadow opacity", &mut g.shadow[0], 0.0, 0.8, "");
}

fn widget_label(kind: &str) -> &str {
    widget::CATALOG.iter().find(|(k, _)| *k == kind).map(|(_, l)| *l).unwrap_or(kind)
}

fn page_panels(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, st: &mut PanelsState) {
    ui.text(x, y + 52.0, "Panels", 22.0, true, ui.ink, 0.97, Align::Left);
    let menubar = ctx.theme.menubar.clone();
    let accent = ui.accent;
    let bars = &mut ctx.theme.bars;
    st.panel = st.panel.min(bars.len().saturating_sub(1));

    // while the add-widget picker is open the page underneath ignores the pointer
    let (click, held) = (ui.click, ui.held);
    if st.add_zone.is_some() {
        ui.click = false;
        ui.held = false;
    }

    // which panel
    ui.row(x, y + 86.0, "Panel");
    let names: Vec<String> = bars.iter().map(|b| b.name.clone()).collect();
    if !names.is_empty() {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut sel = st.panel;
        if ui.segmented([x + 150.0, y + 80.0, cw - 150.0 - 150.0, 30.0], &refs, &mut sel) {
            st.panel = sel;
            st.sel = None;
        }
    }
    if ui.button([x + cw - 140.0, y + 80.0, 66.0, 30.0], "+ New", false) {
        bars.push(BarConfig { name: format!("panel {}", bars.len() + 1), edge: Edge::Bottom, surface: BarSurface::Glass, height: Some(44.0), center: vec![WidgetSpec::new("tasks")], right: vec![WidgetSpec::new("clock")], ..Default::default() });
        st.panel = bars.len() - 1;
        st.sel = None;
        ui.changed = true;
        return;
    }
    if !bars.is_empty() && ui.button([x + cw - 68.0, y + 80.0, 68.0, 30.0], "Delete", false) {
        bars.remove(st.panel);
        st.sel = None;
        ui.changed = true;
        return;
    }
    let Some(bar) = bars.get_mut(st.panel) else {
        ui.text(x, y + 140.0, "No panels. Press + New to add one.", 13.0, false, ui.ink, 0.6, Align::Left);
        return;
    };
    ui.row(x, y + 126.0, "Name");
    let mut name = bar.name.clone();
    if ui.text_field([x + 66.0, y + 120.0, cw / 2.0 - 76.0, 26.0], &format!("bar-name-{}", st.panel), &mut name) && !name.trim().is_empty() {
        bar.name = name;
    }
    let y = y + 36.0;

    // the panel itself, in two columns
    let colw = cw / 2.0 - 10.0;
    let x2 = x + cw / 2.0 + 10.0;
    let sl = |cx: f32, yy: f32| [cx + 66.0, yy - 9.0, colw - 66.0 - 44.0, 18.0];
    let strip = bar.shape == BarShape::Strip;
    ui.row(x, y + 126.0, "Edge");
    let mut sel = match bar.edge {
        Edge::Top => 0,
        Edge::Bottom => 1,
        Edge::Left => 2,
        Edge::Right => 3,
    };
    if ui.segmented([x + 66.0, y + 120.0, colw - 66.0, 28.0], &["Top", "Bottom", "Left", "Right"], &mut sel) {
        bar.edge = [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right][sel];
    }
    ui.row(x2, y + 126.0, "Shape");
    let mut sel = (bar.shape == BarShape::Floating) as usize;
    if ui.segmented([x2 + 66.0, y + 120.0, colw - 66.0, 28.0], &["Strip", "Floating"], &mut sel) {
        bar.shape = if sel == 0 { BarShape::Strip } else { BarShape::Floating };
    }
    if strip {
        ui.row(x, y + 162.0, "Look");
        let mut sel = (bar.surface == BarSurface::Glass) as usize;
        if ui.segmented([x + 66.0, y + 156.0, colw - 66.0, 28.0], &["Band", "Glass"], &mut sel) {
            bar.surface = if sel == 0 { BarSurface::Look } else { BarSurface::Glass };
        }
        ui.row(x2, y + 162.0, "Height");
        let mut h = bar.height.unwrap_or(menubar.height);
        let old = h;
        ui.slider(sl(x2, y + 162.0), &mut h, 20.0, 90.0, "");
        if h != old {
            bar.height = Some(h.round());
        }
        ui.row(x, y + 198.0, "Inset");
        ui.slider(sl(x, y + 198.0), &mut bar.inset, 0.0, 40.0, "");
        if bar.surface == BarSurface::Glass {
            ui.row(x2, y + 198.0, "Radius");
            ui.slider(sl(x2, y + 198.0), &mut bar.radius, 0.0, 40.0, "");
        } else {
            ui.row(x2, y + 198.0, "Opacity");
            let look = bar.look.get_or_insert_with(|| menubar.clone());
            ui.slider(sl(x2, y + 198.0), &mut look.opacity, 0.0, 1.0, "");
        }
    } else {
        ui.row(x, y + 162.0, "Margin");
        ui.slider(sl(x, y + 162.0), &mut bar.margin, 0.0, 60.0, "");
    }

    ui.row(x, y + 224.0, "Auto-hide");
    let mut sel = (bar.autohide != widget::AutoHide::Off) as usize;
    if ui.segmented([x + 66.0, y + 212.0, colw - 66.0, 24.0], &["Off", "At the edge"], &mut sel) {
        bar.autohide = if sel == 0 { widget::AutoHide::Off } else { widget::AutoHide::Always };
    }

    // widgets, by zone
    ui.section(x, y + 244.0, "WIDGETS");
    for z in 0..3 {
        let yy = y + 252.0 + z as f32 * 36.0;
        ui.text(x, yy + 19.0, ["Left", "Centre", "Right"][z], 12.0, false, ui.ink, 0.55, Align::Left);
        let labels: Vec<String> = bar.zone_mut(z).iter().map(|w| widget_label(&w.kind).to_string()).collect();
        let est: Vec<f32> = labels.iter().map(|l| l.chars().count() as f32 * 6.6 + 22.0).collect();
        let total: f32 = est.iter().map(|w| w + 6.0).sum::<f32>() + 34.0;
        let k = ((cw - 64.0) / total).min(1.0);
        let mut cx = x + 64.0;
        for (i, (lab, w)) in labels.iter().zip(&est).enumerate() {
            let r = [cx, yy, w * k, 28.0];
            let selected = st.sel == Some((z, i));
            if selected {
                ui.fill(r, 8.0, accent, 0.30);
            } else {
                ui.fill(r, 8.0, ui.ink, if ui.hit(r) { 0.16 } else { 0.08 });
            }
            ui.text(r[0] + r[2] / 2.0, r[1] + 18.5, lab, 12.0 * k.max(0.85), selected, ui.ink, 0.95, Align::Center);
            if ui.click && ui.hit(r) {
                st.sel = Some((z, i));
            }
            cx += r[2] + 6.0;
        }
        let r = [cx, yy, 28.0, 28.0];
        ui.fill(r, 8.0, ui.ink, if ui.hit(r) { 0.16 } else { 0.08 });
        ui.glyph(r[0] + 14.0, r[1] + 14.0, 14.0, "plus", ui.ink, 0.9);
        if ui.click && ui.hit(r) {
            st.add_zone = Some(z);
        }
    }

    // the selected widget: move / remove, then its own options
    let mut sel = st.sel.filter(|&(z, i)| i < bar.zone_mut(z).len());
    st.sel = sel;
    match sel.take() {
        None => ui.text(x, y + 394.0, "Select a widget above to move or configure it.", 12.5, false, ui.ink, 0.55, Align::Left),
        Some((z, i)) => {
            let n = bar.zone_mut(z).len();
            let ty = y + 366.0;
            if ui.button([x, ty, 36.0, 28.0], "←", false) && i > 0 {
                bar.zone_mut(z).swap(i, i - 1);
                st.sel = Some((z, i - 1));
            }
            if ui.button([x + 42.0, ty, 36.0, 28.0], "→", false) && i + 1 < n {
                bar.zone_mut(z).swap(i, i + 1);
                st.sel = Some((z, i + 1));
            }
            if ui.button([x + 84.0, ty, 110.0, 28.0], "Next zone", false) {
                let w = bar.zone_mut(z).remove(i);
                let nz = (z + 1) % 3;
                bar.zone_mut(nz).push(w);
                st.sel = Some((nz, bar.zone_mut(nz).len() - 1));
            }
            if ui.button([x + 200.0, ty, 80.0, 28.0], "Remove", false) {
                bar.zone_mut(z).remove(i);
                st.sel = None;
                ui.changed = true;
            }
            if let Some((z, i)) = st.sel.or(Some((z, i))).filter(|&(z, i)| i < bar.zone_mut(z).len()) {
                let spec = &mut bar.zone_mut(z)[i];
                let opts = widget::options(&spec.kind);
                ui.section(x, y + 418.0, &format!("{} OPTIONS", widget_label(&spec.kind).to_uppercase()));
                if opts.is_empty() {
                    ui.text(x, y + 446.0, "This widget has nothing to configure.", 12.5, false, ui.ink, 0.55, Align::Left);
                }
                let mut f = Form::new(x, y + 440.0, cw, 30.0);
                for o in &opts {
                    match o.kind {
                        OptKind::Num(lo, hi, _) => {
                            let old = spec.opt_num(o);
                            let mut v = old;
                            f.slider(ui, o.label, &mut v, lo, hi, "");
                            if v != old {
                                spec.set_num(o.key, v);
                            }
                        }
                        OptKind::Bool(_) => {
                            let old = spec.opt_bool(o);
                            let mut b = old;
                            f.toggle(ui, o.label, &mut b);
                            if b != old {
                                spec.set_bool(o.key, b);
                            }
                        }
                        OptKind::Text(_) => {
                            let mut t = spec.opt_str(o);
                            let key = format!("w{}:{}:{}:{}", st.panel, z, i, o.key);
                            let hint = (spec.kind == "clock").then(|| widget::format_time(&if t.is_empty() { "%a %e %b %l:%M %P".to_string() } else { t.clone() }));
                            if f.text(ui, o.label, &key, &mut t, hint.as_deref()) {
                                spec.set_str(o.key, &t);
                            }
                        }
                        OptKind::Choice(choices, _) => {
                            let cur = spec.opt_choice(o);
                            let step = f.stepper(ui, o.label, choices[cur].1);
                            if step != 0 {
                                spec.set_str(o.key, choices[(cur as i32 + step).rem_euclid(choices.len() as i32) as usize].0);
                            }
                        }
                    }
                }
            }
        }
    }

    // the add-widget picker
    if let Some(z) = st.add_zone {
        ui.click = click;
        ui.held = held;
        let r = [x - 10.0, y + 244.0, cw + 20.0, 252.0];
        let bg = if ui.dark { [0.139, 0.139, 0.139] } else { [0.98, 0.99, 1.0] };
        ui.fill([r[0] - 1.0, r[1] - 1.0, r[2] + 2.0, r[3] + 2.0], 13.0, ui.ink, 0.18);
        ui.fill(r, 12.0, bg, 0.98);
        ui.text(r[0] + 16.0, r[1] + 26.0, &format!("Add a widget to the {} zone", ["left", "centre", "right"][z]), 13.5, true, ui.ink, 0.95, Align::Left);
        let (cols, cell_w, cell_h) = (4usize, (r[2] - 32.0) / 4.0, 34.0);
        let mut chosen = None;
        for (n, (kind, label)) in widget::CATALOG.iter().enumerate() {
            let cell = [r[0] + 16.0 + (n % cols) as f32 * cell_w, r[1] + 44.0 + (n / cols) as f32 * (cell_h + 4.0), cell_w - 6.0, cell_h];
            ui.fill(cell, 8.0, ui.ink, if ui.hit(cell) { 0.16 } else { 0.07 });
            ui.text(cell[0] + cell[2] / 2.0, cell[1] + 21.0, label, 12.0, false, ui.ink, 0.95, Align::Center);
            if ui.click && ui.hit(cell) {
                chosen = Some(*kind);
            }
        }
        if let Some(kind) = chosen {
            bar.zone_mut(z).push(WidgetSpec::new(kind));
            st.sel = Some((z, bar.zone_mut(z).len() - 1));
            st.add_zone = None;
            ui.changed = true;
        } else if ui.button([r[0] + r[2] - 90.0, r[1] + r[3] - 40.0, 74.0, 28.0], "Cancel", false) || (ui.click && !ui.hit(r)) {
            st.add_zone = None;
        }
    }
}

fn page_wallpaper(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32, first_row: usize, actions: &mut Vec<SettingsAction>) {
    ui.text(x, y + 52.0, "Wallpaper", 22.0, true, ui.ink, 0.97, Align::Left);
    ui.section(x, y + 86.0, "CHOOSE ONE");
    let cols = 4.0;
    let gap = 14.0;
    let tw = (cw - gap * (cols - 1.0)) / cols;
    let th = tw * 0.62;
    for (i, item) in ctx.walls.iter().enumerate() {
        let row_idx = i / 4;
        if row_idx < first_row || row_idx >= first_row + 4 {
            continue;
        }
        let (col, row) = ((i % 4) as f32, (row_idx - first_row) as f32);
        let r = [x + col * (tw + gap), y + 100.0 + row * (th + 34.0), tw, th];
        let current = ctx.theme.wallpaper == item.spec;
        let hov = ui.hit(r);
        if current {
            ui.fill([r[0] - 3.0, r[1] - 3.0, r[2] + 6.0, r[3] + 6.0], 10.0, ui.accent, 1.0);
        }
        match (item.cell, item.colors) {
            (Some(cell), _) => {
                ui.d.image(r, 8.0, cell);
            }
            (None, Some((a, b))) => {
                ui.grad(r, 8.0, a, b, 1.0);
                ui.fill([r[0] + 8.0, r[1] + 8.0, 34.0, 16.0], 8.0, [0.0; 3], 0.45);
                ui.text(r[0] + 25.0, r[1] + 20.0, "LIVE", 9.5, true, [1.0; 3], 0.95, Align::Center);
            }
            _ => ui.fill(r, 8.0, ui.ink, 0.12),
        }
        if hov && !current {
            ui.fill(r, 8.0, [1.0; 3], 0.12);
        }
        let name: String = if item.name.chars().count() > 20 { item.name.chars().take(19).collect::<String>() + "…" } else { item.name.clone() };
        ui.text(r[0] + r[2] / 2.0, r[1] + r[3] + 16.0, &name, 11.5, current, ui.ink, 0.9, Align::Center);
        if ui.click && hov {
            actions.push(SettingsAction::Wallpaper(item.spec.clone()));
        }
    }
    if ctx.walls.len() > 16 {
        ui.text(x + cw, y + 86.0, &format!("scroll for more ({} total)", ctx.walls.len()), 11.0, false, ui.ink, 0.5, Align::Right);
    }
    if ctx.walls.is_empty() {
        ui.text(x, y + 130.0, "No wallpapers found. Put images in ~/Pictures/Wallpapers.", 13.0, false, ui.ink, 0.6, Align::Left);
    }
}

fn page_layout(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32) {
    ui.text(x, y + 52.0, "Layout", 22.0, true, ui.ink, 0.97, Align::Left);
    let mut f = Form::new(x, y + 92.0, cw, 46.0);
    let mut layout = ctx.theme.layout;
    if f.pick(ui, "Shell layout", &mut layout, &[(Layout::Macos, "Menu bar + Dock"), (Layout::Taskbar, "Taskbar")]) {
        ctx.theme.layout = layout;
        // each layout brings its own panels
        ctx.theme.bars = if layout == Layout::Macos { widget::default_bars() } else { theme::preset("windows11").map(|t| t.bars).unwrap_or_default() };
    }
    f.pick(
        ui,
        "Launcher",
        &mut ctx.theme.launcher.style,
        &[(theme::LauncherKind::Figlet, "Figlet"), (theme::LauncherKind::Launchpad, "Launchpad"), (theme::LauncherKind::StartMenuWin11, "Start (Fluent)"), (theme::LauncherKind::StartMenuWin7, "Start (Aero)")],
    );
    f.toggle(ui, "Icons on the desktop", &mut ctx.cfg.desktop.show);
    f.slider(ui, "Desktop icon size", &mut ctx.cfg.desktop.icon_size, 32.0, 128.0, " px");
    ctx.cfg.desktop.icon_size = ctx.cfg.desktop.icon_size.round();
    if layout == Layout::Taskbar {
        f.note(ui, "The taskbar is built from panels and widgets.", false);
        f.note(ui, "Position, start button, search box, tray, clock and icon sizes are on the Panels page.", true);
        return;
    }
    let (dock, mb) = (&mut ctx.theme.dock, &mut ctx.theme.menubar);
    f.slider(ui, "Dock icon size", &mut dock.icon_size, 40.0, 88.0, " px");
    f.toggle(ui, "Magnify on hover", &mut dock.magnify);
    f.pitch = 42.0;
    f.toggle(ui, "Menu bar", &mut mb.bar);
    f.pick(ui, "Bar style", &mut mb.kind, &[(MenubarKind::Glass, "Glass"), (MenubarKind::Solid, "Solid"), (MenubarKind::Fade, "Fade"), (MenubarKind::Clear, "None")]);
    f.slider(ui, "Opacity", &mut mb.opacity, 0.0, 1.0, "");
    f.slider(ui, "Blur", &mut mb.blur, 0.0, 30.0, "");
    f.slider(ui, "Height", &mut mb.height, 34.0, 44.0, " px");
    f.toggle(ui, "Bottom border line", &mut mb.border);
    f.toggle(ui, "Logo and app name", &mut mb.show_title);
}

fn page_windows(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32) {
    use theme::{BarMaterial, BodyTone, ButtonColors, ButtonOrder, ButtonShape, ButtonSide, FrameStyle, GlyphShow};
    ui.text(x, y + 52.0, "Windows", 22.0, true, ui.ink, 0.97, Align::Left);
    let custom = ctx.theme.window.decoration == DecoKind::Custom;
    let mut f = Form::new(x, y + 90.0, cw, if custom { 30.0 } else { 46.0 });
    let w = &mut ctx.theme.window;
    let before = w.decoration;
    f.pick(ui, "Title bar style", &mut w.decoration, &[(DecoKind::Mac, "Mac"), (DecoKind::Win11, "Win 11"), (DecoKind::Aero, "Aero"), (DecoKind::Minimal, "None"), (DecoKind::Custom, "Custom")]);
    // choosing Custom starts from the frame that was in use, so nothing jumps
    if before != w.decoration && w.decoration == DecoKind::Custom && before != DecoKind::Custom {
        w.frame = FrameStyle::from_kind(before);
    }
    if w.decoration == DecoKind::Custom {
        // ---- the constructor
        ui.row(f.x, f.y, "Start from");
        for (i, (kind, name)) in [(DecoKind::Mac, "Mac"), (DecoKind::Win11, "Windows"), (DecoKind::Gnome, "GNOME"), (DecoKind::Aero, "Aero")].iter().enumerate() {
            if ui.button([f.x + 150.0 + i as f32 * 96.0, f.y - 12.0, 88.0, 24.0], name, false) {
                w.frame = FrameStyle::from_kind(*kind);
            }
        }
        f.y += f.pitch + 6.0;
        ui.section(x, f.y - 6.0, "BUTTONS");
        f.y += 14.0;
        let fs = &mut w.frame;
        f.pick(ui, "Side", &mut fs.side, &[(ButtonSide::Left, "Left"), (ButtonSide::Right, "Right")]);
        f.pick(ui, "Order from the edge", &mut fs.order, &[(ButtonOrder::CloseMinMax, "x – +"), (ButtonOrder::CloseMaxMin, "x + –"), (ButtonOrder::MinMaxClose, "– + x"), (ButtonOrder::MinCloseMax, "– x +")]);
        f.pick(ui, "Shape", &mut fs.shape, &[(ButtonShape::Circle, "Circle"), (ButtonShape::Rounded, "Rounded"), (ButtonShape::Square, "Square"), (ButtonShape::Flat, "Flat"), (ButtonShape::Bar, "Bar")]);
        f.pick(ui, "Colours", &mut fs.colors, &[(ButtonColors::Traffic, "Traffic"), (ButtonColors::Accent, "Accent"), (ButtonColors::Mono, "Mono"), (ButtonColors::CloseRed, "Red close")]);
        f.pick(ui, "Icons", &mut fs.glyphs, &[(GlyphShow::Hover, "On hover"), (GlyphShow::Always, "Always"), (GlyphShow::Never, "Never")]);
        f.slider(ui, "Size", &mut fs.button, 10.0, 52.0, " px");
        f.slider(ui, "Gap", &mut fs.gap, 0.0, 24.0, " px");
        ui.section(x, f.y - 6.0, "BAR AND FRAME");
        f.y += 14.0;
        f.pick(ui, "Bar", &mut fs.bar, &[(BarMaterial::Body, "As window"), (BarMaterial::Tone, "Tinted"), (BarMaterial::Accent, "Accent"), (BarMaterial::Glass, "Glass")]);
        f.pick(ui, "Window colour", &mut fs.body, &[(BodyTone::Auto, "Follow theme"), (BodyTone::Light, "Light"), (BodyTone::Dark, "Dark")]);
        f.slider(ui, "Frame thickness", &mut fs.border, 0.0, 14.0, " px");
        f.slider(ui, "Title size", &mut fs.title_size, 10.0, 18.0, " px");
        f.toggle(ui, "Bold title", &mut fs.title_bold);
        f.toggle(ui, "Hairline outline", &mut fs.outline);
        f.pick(ui, "Title position", &mut w.title_align, &[(theme::TitleAlign::Left, "Left"), (theme::TitleAlign::Center, "Centre")]);
        ui.section(x, f.y - 6.0, "SHAPE");
        f.y += 14.0;
    }
    f.slider(ui, "Corner radius", &mut w.radius, 0.0, 32.0, " px");
    f.slider(ui, "Title bar height", &mut w.titlebar, 24.0, 52.0, " px");
    f.slider(ui, "Gap around maximized", &mut w.max_gap, 0.0, 60.0, " px");
    f.toggle(ui, "Hide the title bar when maximized", &mut w.hide_title_max);
    f.slider(ui, "Content opacity", &mut w.client_opacity, 0.3, 1.0, "");
    let mut sh = w.shadow[0];
    f.slider(ui, "Shadow strength", &mut sh, 0.0, 0.8, "");
    w.shadow[0] = sh;
    w.shadow_idle[0] = sh * 0.52;
    f.skip(if custom { 4.0 } else { 24.0 });
    f.note(ui, "Windows already open keep their frame until the next theme change.", true);
}

fn effect_row(ui: &mut Ui, an: &mut theme::AnimStyle, x: f32, y: f32, id: &str, label: &str) {
    let mut e = an.effects.get(id).copied().unwrap_or_else(|| theme::default_effect(id));
    let before = e;
    ui.text(x, y + 4.5, label, 12.5, false, ui.ink, if an.enabled { 0.92 } else { 0.45 }, Align::Left);
    ui.toggle(x + 150.0, y - 12.0, &mut e.on);
    if e.on {
        ui.slider([x + 210.0, y - 10.0, 66.0, 20.0], &mut e.speed, 0.25, 3.0, "x");
    } else {
        ui.text(x + 210.0, y + 4.5, "off", 12.0, false, ui.ink, 0.4, Align::Left);
    }
    if e != before {
        if e == theme::default_effect(id) {
            an.effects.remove(id);
        } else {
            an.effects.insert(id.to_string(), e);
        }
    }
}

/// Words (lower case) that find a page in the sidebar search besides its name.
fn page_keywords(page: Page) -> &'static str {
    match page {
        Page::General => "language shortcuts screenshots power sleep lock idle weather city sidebar scroll",
        Page::Themes => "template preset import export share save theme gnome fluent aero jelly",
        Page::Displays => "monitor screen resolution scale refresh rate hdr arrangement",
        Page::Appearance => "light dark mode accent colour color highlight wallpaper tint",
        Page::Glass => "blur refraction rim dispersion tint saturation bezel material paper wood clay",
        Page::Panels => "bar dock taskbar widget menu bar tray clock start",
        Page::Wallpaper => "background picture image live shader",
        Page::Layout => "launcher dock size magnify menu bar style figlet launchpad start menu",
        Page::Windows => "title bar buttons frame border corner radius shadow opacity traffic lights decoration custom",
        Page::Animations => "motion speed jelly genie minimize open close effects sounds volume transition",
        Page::Keys => "keyboard shortcut keys layout run program",
        Page::Mouse => "cursor pointer speed acceleration scroll natural tap touchpad left-handed size",
        Page::Icons => "icon shape plate tile padding theme",
        Page::About => "version info credits",
        Page::Notifications => "notification toast do not disturb dnd corner position time",
        Page::DateTime => "clock time date 24 hour seconds calendar week monday sunday format",
        Page::Sound => "volume mute speaker output sound effects audio",
        Page::Network => "wifi wi-fi wireless bluetooth internet connection",
        Page::Power => "battery power profile saver performance sleep suspend lock screen off idle health charge",
        Page::Programs => "programs apps rules per program opacity frost blur dark light font popup menu tooltip density slide highlight qt gtk kde other"
    }
}

fn page_animations(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32) {
    ui.text(x, y + 52.0, "Animations", 22.0, true, ui.ink, 0.97, Align::Left);
    let a = &mut ctx.theme.animations;
    // ---- all of them
    let gy = y + 76.0;
    ui.fill([x, gy, cw, 54.0], 12.0, ui.ink, 0.05);
    ui.row(x + 16.0, gy + 14.0, "Animations");
    ui.toggle(x + 120.0, gy + 15.0, &mut a.enabled);
    ui.row(x + 220.0, gy + 14.0, "Speed of all");
    ui.slider([x + 330.0, gy + 17.0, 200.0, 20.0], &mut a.speed, 0.25, 3.0, "x");
    // ---- each one: groups in two columns
    let colw = (cw - 28.0) / 2.0;
    let top = gy + 80.0;
    let mut col_y = [top, top];
    let mut group = "";
    let mut col = 0;
    for (g, id, label) in theme::EFFECTS {
        if *g != group {
            group = g;
            // Windows and Menus on the left, the Shell on the right
            col = if *g == "Shell" { 1 } else { 0 };
            if col_y[col] > top {
                col_y[col] += 20.0;
            }
            ui.section(x + col as f32 * (colw + 28.0), col_y[col], &g.to_uppercase());
            col_y[col] += 22.0;
        }
        effect_row(ui, a, x + col as f32 * (colw + 28.0) + 4.0, col_y[col] + 10.0, id, label);
        col_y[col] += 32.0;
    }
    // ---- the light / dark switch and the sounds, under the Shell column
    let rx = x + colw + 28.0;
    let mut ry = col_y[1] + 10.0;
    ui.section(rx, ry, "LIGHT / DARK SWITCH");
    ry += 26.0;
    let mut sel = match a.switch {
        theme::SwitchAnim::Circle => 0,
        theme::SwitchAnim::Fade => 1,
        theme::SwitchAnim::None => 2,
    };
    if ui.segmented([rx + 4.0, ry - 10.0, colw - 8.0, 28.0], &["Circle", "Fade", "None"], &mut sel) {
        a.switch = [theme::SwitchAnim::Circle, theme::SwitchAnim::Fade, theme::SwitchAnim::None][sel];
    }
    ry += 46.0;
    ui.row(rx + 4.0, ry - 4.0, "Duration");
    ui.slider([rx + 110.0, ry - 4.0, colw - 200.0, 20.0], &mut a.switch_ms, 150.0, 2500.0, " ms");
    ry += 34.0;
    ui.row(rx + 4.0, ry - 4.0, "Glowing edge");
    ui.toggle(rx + 150.0, ry - 8.0, &mut a.rim);
    ry += 40.0;
    ui.section(rx, ry, "SOUNDS");
    ry += 26.0;
    ui.row(rx + 4.0, ry - 4.0, "Interface sounds");
    ui.toggle(rx + 150.0, ry - 8.0, &mut a.sounds);
    ry += 34.0;
    ui.row(rx + 4.0, ry - 4.0, "Volume");
    ui.slider([rx + 110.0, ry - 4.0, colw - 180.0, 20.0], &mut a.sound_volume, 0.0, 1.5, "");
    let ny = col_y[0].max(ry) + 34.0;
    ui.text(x, ny, "A switched-off animation does not play at all: what it moves jumps to where it ends.", 12.0, false, ui.ink, 0.55, Align::Left);
}

/// Whether the "Ours" icon kind is offered on the icons page.
const SHOW_OWN_ICONS: bool = false;

/// Returns the rectangle where the live preview goes (drawn in the shell layer, see `icon_preview`).
fn page_icons(ui: &mut Ui, ctx: &mut SettingsCtx, x: f32, y: f32, cw: f32) -> [f32; 4] {
    ui.text(x, y + 52.0, "Icons", 22.0, true, ui.ink, 0.97, Align::Left);
    let mut f = Form::new(x, y + 92.0, cw, 46.0);
    let accent = ctx.theme.accent;
    let icons = &mut ctx.theme.icons;
    // two kinds of icons: the simple ones of a pack on the shell's plate, and ours (made in Memrium Icons)
    ui.section(x, f.y - 6.0, "ICONS");
    f.skip(14.0);
    let own_ready = crate::iconrender::theme_dir().join("index.theme").is_file();
    let is_own = icons.is_own();
    let half = (cw - 10.0) / 2.0;
    for (i, (glyph, title, sub)) in [("layers", "Simple, with a plate", "A pack's icons on the shell's plate"), ("sparkles", "Ours", "Made in Memrium Icons, each with its own plate")].iter().enumerate() {
        // "Ours" is hidden for now
        if i == 1 && !SHOW_OWN_ICONS {
            continue;
        }
        let r = [x + i as f32 * (half + 10.0), f.y - 15.0, half, 64.0];
        let selected = is_own == (i == 1);
        let usable = i == 0 || own_ready;
        let hov = ui.hit(r) && usable;
        if hov {
            ui.want_cursor(chebrium::ui::Cursor::Pointer);
        }
        ui.fill(r, 12.0, ui.ink, if hov { 0.10 } else { 0.05 });
        if selected {
            ui.fill([r[0] - 1.5, r[1] - 1.5, r[2] + 3.0, r[3] + 3.0], 13.0, accent, 0.9);
            ui.fill(r, 12.0, if ui.dark { [0.181, 0.181, 0.181] } else { [0.93, 0.95, 1.0] }, 1.0);
            ui.fill(r, 12.0, accent, 0.14);
        }
        let a = if usable { 1.0 } else { 0.4 };
        ui.glyph(r[0] + 26.0, r[1] + 32.0, 22.0, glyph, ui.ink, 0.8 * a);
        ui.text_raw(r[0] + 50.0, r[1] + 28.0, title, 13.5, true, ui.ink, 0.95 * a, Align::Left);
        let sub = if usable { sub } else { "Build the icons in Memrium Icons first" };
        ui.text_raw(r[0] + 50.0, r[1] + 47.0, sub, 11.0, false, ui.ink, 0.6 * a, Align::Left);
        if ui.click && hov && !selected {
            icons.use_own(i == 1);
            ui.changed = true;
        }
    }
    f.skip(74.0);
    // which look of our icons the shell shows
    if icons.is_own() {
        let cols = 4usize;
        let gap = 6.0;
        let w = (cw - gap * (cols as f32 - 1.0)) / cols as f32;
        for (i, (key, name, _)) in crate::theme::OWN_LOOKS.iter().enumerate() {
            let r = [x + (i % cols) as f32 * (w + gap), f.y - 12.0 + (i / cols) as f32 * 34.0, w, 28.0];
            if ui.button(r, name, icons.own_look == *key) && icons.own_look != *key {
                icons.own_look = key.to_string();
                ui.changed = true;
            }
        }
        f.skip(((crate::theme::OWN_LOOKS.len() + cols - 1) / cols) as f32 * 34.0 + 10.0);
    }
    f.pick(ui, "Shape", &mut icons.shape, &[(IconShape::None, "Original"), (IconShape::Squircle, "Squircle"), (IconShape::Circle, "Circle"), (IconShape::Rounded, "Rounded")]);
    if f.pick(ui, "Plate behind icons", &mut icons.plate, &[(Plate::None, "None"), (Plate::Glass, "Glass"), (Plate::Solid, "Solid"), (Plate::Tile, "Tile")]) && icons.plate == Plate::Tile && icons.padding < 0.04 {
        icons.padding = 0.16;
    }
    if icons.plate == Plate::Tile {
        f.pick(ui, "Tile colour", &mut icons.tile, &[(TileMode::Auto, "Auto"), (TileMode::Light, "Light"), (TileMode::Dark, "Dark"), (TileMode::Vivid, "Vivid")]);
    }
    f.slider(ui, "Inner padding", &mut icons.padding, 0.0, 0.4, "");
    // the icon pack app icons come from: a card per installed pack, with a few of its icons
    ui.section(x, f.y, "ICON PACK");
    f.skip(14.0);
    static PACKS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    // (the themes of our icons are chosen above, not among the packs)
    let packs = PACKS.get_or_init(|| crate::apps::installed_icon_themes().into_iter().filter(|n| !n.starts_with(crate::theme::OWN_ICON_THEME)).collect());
    const SAMPLE: [&str; 4] = ["system-file-manager", "internet-web-browser", "utilities-terminal", "preferences-system"];
    let (cols, gap, card_h) = (3usize, 10.0, 88.0);
    let card_w = (cw - gap * (cols as f32 - 1.0)) / cols as f32;
    let auto = icons.theme.is_empty() || icons.theme == "auto";
    // "Auto" first: the desktop's own pack
    let mut cards: Vec<(&str, &str)> = vec![("auto", "Follow the desktop")];
    cards.extend(packs.iter().map(|p| (p.as_str(), p.as_str())));
    for (i, (value, name)) in cards.iter().enumerate() {
        let r = [x + (i % cols) as f32 * (card_w + gap), f.y + (i / cols) as f32 * (card_h + gap) - 15.0, card_w, card_h];
        let selected = if *value == "auto" { auto } else { icons.theme == *value };
        let hov = ui.hit(r);
        if hov {
            ui.want_cursor(chebrium::ui::Cursor::Pointer);
        }
        ui.fill(r, 12.0, ui.ink, if hov { 0.10 } else { 0.05 });
        if selected {
            ui.fill([r[0] - 1.5, r[1] - 1.5, r[2] + 3.0, r[3] + 3.0], 13.0, accent, 0.9);
            ui.fill(r, 12.0, if ui.dark { [0.181, 0.181, 0.181] } else { [0.93, 0.95, 1.0] }, 1.0);
            ui.fill(r, 12.0, accent, 0.14);
        }
        if *value == "auto" {
            ui.glyph(r[0] + r[2] / 2.0, r[1] + 32.0, 24.0, "monitor", ui.ink, 0.7);
        } else {
            let sw = 34.0;
            let start = r[0] + (r[2] - SAMPLE.len() as f32 * sw) / 2.0;
            for (k, icon) in SAMPLE.iter().enumerate() {
                let id = format!("pack:{value}:{icon}");
                let cell = [start + k as f32 * sw + 2.0, r[1] + 14.0, 30.0, 30.0];
                match (ctx.app_icon)(&id) {
                    Some(c) => ui.d.image(cell, 7.0, c),
                    None => {
                        ui.fill(cell, 7.0, ui.ink, 0.07);
                        ctx.want_icons.push(id);
                    }
                }
            }
        }
        ui.text_raw(r[0] + r[2] / 2.0, r[1] + r[3] - 14.0, name, 12.0, selected, ui.ink, if selected { 1.0 } else { 0.8 }, Align::Center);
        if ui.click && hov && !selected {
            icons.theme = value.to_string();
            ui.changed = true;
        }
    }
    f.skip(((cards.len() + cols - 1) / cols) as f32 * (card_h + gap) + 6.0);
    let mut mono = icons.tint.is_some();
    let before = mono;
    f.toggle(ui, "One-colour icons", &mut mono);
    if mono != before {
        icons.tint = if mono { Some(accent) } else { None };
    }
    f.skip(14.0);
    ui.section(x, f.y, "PREVIEW");
    f.skip(20.0);
    let rect = [x, f.y, cw, 318.0];
    f.skip(rect[3] + 10.0);
    f.note(ui, "The preview updates live. App icons come from the icon theme above;", true);
    f.note(ui, "set per-app replacements under [override.icons.overrides] in config.toml.", true);
    rect
}

/// The icon preview: your apps in the current style, growing under the pointer, in several sizes, and in a Dock, a
/// notification and a list, as they will be used.
fn icon_preview(ui: &mut Ui, ctx: &SettingsCtx, r: [f32; 4], sp: &mut Vec<Panel>, sl: &mut Vec<Label>) {
    use crate::renderer::Panel as P;
    let theme: &Theme = ctx.theme;
    let ink = ui.ink;
    let apps = ctx.icons;
    if apps.is_empty() {
        sl.push(Label::new("No application icons loaded yet", r[0] + r[2] / 2.0, r[1] + 60.0, 13.0, false, ink, 0.55, Align::Center));
        return;
    }
    // a card behind everything, so the icons are seen on a plain ground
    let ground = if theme.dark { [0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0] };
    sp.push(P::solid(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2], r[3], 16.0, ground, ground).flat().alpha(if theme.dark { 0.22 } else { 0.45 }));
    // ---- the apps, in a row; the one under the pointer grows
    let n = apps.len().min(8);
    let slot = (r[2] - 40.0) / n as f32;
    let row_y = r[1] + 56.0;
    for (i, (cell, name)) in apps.iter().take(n).enumerate() {
        let cx = r[0] + 20.0 + slot * (i as f32 + 0.5);
        let hot = (ui.mouse[0] - cx).abs() < slot / 2.0 && (ui.mouse[1] - row_y).abs() < 52.0;
        let k = ui.ease(9100 + i as u64, if hot { 1.28 } else { 1.0 }, 16.0);
        let size = 52.0 * k;
        crate::theme::push_app_icon(theme, sp, cx, row_y - (k - 1.0) * 26.0, size, *cell, (0.22, 10.0, 4.0));
        let short: String = if name.chars().count() > 12 { name.chars().take(11).collect::<String>() + "…" } else { name.clone() };
        sl.push(Label::new(short, cx, row_y + 46.0, 11.5, false, ink, if hot { 0.95 } else { 0.6 }, Align::Center));
    }
    // ---- one icon in every size
    let (cell, _) = &apps[0];
    sl.push(Label::new("Sizes", r[0] + 20.0, r[1] + 118.0, 11.5, true, ink, 0.5, Align::Left));
    let mut x = r[0] + 40.0;
    for size in [16.0, 24.0, 32.0, 48.0, 64.0] {
        crate::theme::push_app_icon(theme, sp, x + size / 2.0, r[1] + 168.0 - size / 2.0 + 24.0, size, *cell, (0.16, 6.0, 2.0));
        sl.push(Label::new(format!("{size:.0}"), x + size / 2.0, r[1] + 206.0, 10.5, false, ink, 0.5, Align::Center));
        x += size + 26.0;
    }
    // ---- in use: a Dock, a notification, a list row
    let gy = r[1] + 238.0;
    let dock_w = 5.0 * 44.0 + 24.0;
    let dock_x = r[0] + 20.0;
    let mut dock = theme.glass.panel(dock_x + dock_w / 2.0, gy + 24.0, dock_w, 58.0, theme.dock.radius.min(22.0)).tinted([0.5, 0.55, 0.65, 0.20]);
    dock.fx[0] = dock.fx[0].max(14.0);
    sp.push(dock);
    for i in 0..5 {
        let (c, _) = &apps[i % apps.len()];
        crate::theme::push_app_icon(theme, sp, dock_x + 22.0 + i as f32 * 44.0 + 2.0, gy + 24.0, 38.0, *c, (0.2, 8.0, 3.0));
    }
    let tx = r[0] + 20.0 + dock_w + 28.0;
    let tw = r[2] - (tx - r[0]) - 20.0;
    let mut toast = theme.glass.panel(tx + tw / 2.0, gy + 24.0, tw.max(160.0), 58.0, 16.0).tinted(if theme.dark { [0.1, 0.1, 0.1, 0.7] } else { [0.96, 0.97, 1.0, 0.7] });
    toast.fx[0] = toast.fx[0].max(14.0);
    sp.push(toast);
    crate::theme::push_app_icon(theme, sp, tx + 30.0, gy + 24.0, 34.0, apps[apps.len() / 2].0, (0.16, 6.0, 2.0));
    sl.push(Label::new(apps[apps.len() / 2].1.clone(), tx + 56.0, gy + 20.0, 12.5, true, ink, 0.95, Align::Left));
    sl.push(Label::new("Your icons look like this in notifications", tx + 56.0, gy + 38.0, 11.0, false, ink, 0.6, Align::Left));
    let _ = ground;
}

fn page_about(ui: &mut Ui, x: f32, y: f32) {
    ui.text(x, y + 52.0, "About", 22.0, true, ui.ink, 0.97, Align::Left);
    let lines = [
        "Memrium: a configurable Wayland desktop.",
        "Glass renderer: wgpu. Protocol: smithay. Icons: Lucide.",
        "",
        "Themes are TOML files. Choose a template, then change anything:",
        "   Jelly Glass  |  Fluent  |  Aero",
        "",
        "Config:   ~/.config/memrium/config.toml",
        "Themes:   ~/.config/memrium/themes/*.toml",
        "Shortcuts: Super+S  search       Alt+Tab  switch window",
        "           Super+Q  close           Super+M  minimize",
    ];
    for (i, t) in lines.iter().enumerate() {
        ui.text(x, y + 100.0 + i as f32 * 24.0, t, 13.0, false, ui.ink, 0.8, Align::Left);
    }
    let _ = theme::config_dir;
}

/// Wallpaper kinds that exist (for the compositor's scanning code).
pub fn live_choices() -> Vec<WallItem> {
    // pictures the program draws itself come first, then the animated ones
    let drawn = crate::wallpapers::BUILTIN.iter().map(|(name, id, a, b)| WallItem { name: name.to_string(), spec: WallpaperSpec { kind: WallKind::Image, path: id.to_string() }, cell: None, colors: Some((*a, *b)) });
    drawn.chain([
        ("Aurora", "aurora", [0.05, 0.10, 0.30], [0.10, 0.80, 0.60]),
        ("Liquid Gradient", "liquid-gradient", [0.45, 0.25, 0.95], [0.95, 0.45, 0.60]),
        ("Ocean Waves", "ocean", [0.02, 0.18, 0.40], [0.20, 0.65, 0.85]),
        ("Starfield", "starfield", [0.01, 0.02, 0.08], [0.20, 0.25, 0.55]),
    ]
    .iter()
    .map(|(name, id, a, b)| WallItem { name: name.to_string(), spec: WallpaperSpec { kind: WallKind::Live, path: id.to_string() }, cell: None, colors: Some((*a, *b)) }))
    .collect()
}
