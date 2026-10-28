//! What a managed window is on the protocol side: an xdg toplevel, or an X11 window brought in by XWayland.
//! The rest of the compositor (frames, focus, animations) only talks to `Role`.

use smithay::{
    reexports::{wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle, Size},
    wayland::shell::xdg::ToplevelSurface,
    xwayland::X11Surface,
};

#[derive(Clone)]
pub enum Role {
    Xdg(ToplevelSurface),
    X11(X11Surface),
}

impl Role {
    /// The window's `wl_surface`. An X11 window has none until XWayland has associated one with it.
    pub fn surface(&self) -> Option<WlSurface> {
        match self {
            Role::Xdg(t) => Some(t.wl_surface().clone()),
            Role::X11(x) => x.wl_surface(),
        }
    }

    pub fn is_surface(&self, s: &WlSurface) -> bool {
        match self {
            Role::Xdg(t) => t.wl_surface() == s,
            Role::X11(x) => x.wl_surface().as_ref() == Some(s),
        }
    }

    pub fn is_x11(&self) -> bool {
        matches!(self, Role::X11(_))
    }

    pub fn x11(&self) -> Option<&X11Surface> {
        match self {
            Role::X11(x) => Some(x),
            Role::Xdg(_) => None,
        }
    }

    /// xdg toplevels must be configured once before they may map; X11 windows need nothing.
    pub fn send_initial_configure(&self) {
        if let Role::Xdg(t) = self {
            if !t.is_initial_configure_sent() {
                t.send_configure();
            }
        }
    }

    pub fn set_active(&self, active: bool) {
        match self {
            Role::Xdg(t) => {
                t.with_pending_state(|s| {
                    if active {
                        s.states.set(xdg_toplevel::State::Activated);
                    } else {
                        s.states.unset(xdg_toplevel::State::Activated);
                    }
                });
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
            Role::X11(x) => {
                let _ = x.set_activated(active);
            }
        }
    }

    pub fn close(&self) {
        match self {
            Role::Xdg(t) => t.send_close(),
            Role::X11(x) => {
                let _ = x.close();
            }
        }
    }

    /// Ask the client for a client-area size, and tell it where that area is on screen (X11 clients live in
    /// absolute screen coordinates, so their popups and menus need to know). `maximized` / `resizing` change the
    /// matching state when given.
    pub fn configure(&self, size: Option<(i32, i32)>, loc: (i32, i32), maximized: Option<bool>, resizing: Option<bool>) {
        match self {
            Role::Xdg(t) => {
                t.with_pending_state(|s| {
                    if let Some(sz) = size {
                        s.size = Some((on_grid(sz.0), on_grid(sz.1)).into());
                    }
                    if let Some(m) = maximized {
                        if m {
                            s.states.set(xdg_toplevel::State::Maximized);
                        } else {
                            s.states.unset(xdg_toplevel::State::Maximized);
                        }
                    }
                    if let Some(r) = resizing {
                        if r {
                            s.states.set(xdg_toplevel::State::Resizing);
                        } else {
                            s.states.unset(xdg_toplevel::State::Resizing);
                        }
                    }
                });
                t.send_pending_configure();
            }
            Role::X11(x) => {
                if let Some(m) = maximized {
                    let _ = x.set_maximized(m);
                }
                let size: Size<i32, Logical> = size.map(|(w, h)| Size::from((to_x(w), to_x(h)))).unwrap_or_else(|| x.geometry().size);
                let _ = x.configure(Rectangle::new(Point::from((to_x(loc.0), to_x(loc.1))), size));
            }
        }
    }

    /// Mark the window fullscreen (or not); `configure` carries the size.
    pub fn set_fullscreen(&self, on: bool) {
        match self {
            Role::Xdg(t) => t.with_pending_state(|s| {
                if on {
                    s.states.set(xdg_toplevel::State::Fullscreen);
                } else {
                    s.states.unset(xdg_toplevel::State::Fullscreen);
                }
            }),
            Role::X11(x) => {
                let _ = x.set_fullscreen(on);
            }
        }
    }

    /// A window that asks for one size only (its smallest and its biggest are the same): it is not resized, maximised
    /// or made fullscreen.
    /// The program asked for fullscreen before its window was shown (the state waits in the configure we will send).
    pub fn asked_fullscreen(&self) -> bool {
        match self {
            Role::Xdg(t) => t.with_pending_state(|s| s.states.contains(xdg_toplevel::State::Fullscreen)),
            Role::X11(x) => x.is_fullscreen(),
        }
    }

    pub fn fixed_size(&self) -> bool {
        match self {
            Role::Xdg(t) => smithay::wayland::compositor::with_states(t.wl_surface(), |st| {
                let mut c = st.cached_state.get::<smithay::wayland::shell::xdg::SurfaceCachedState>();
                let c = c.current();
                c.min_size.w > 0 && c.min_size == c.max_size
            }),
            Role::X11(_) => false,
        }
    }

    /// A top-level window with one fixed size and no parent: a splash screen, which has no use for a title bar.
    pub fn is_splash(&self) -> bool {
        match self {
            Role::Xdg(t) => {
                if t.parent().is_some() {
                    return false;
                }
                smithay::wayland::compositor::with_states(t.wl_surface(), |st| {
                    let mut c = st.cached_state.get::<smithay::wayland::shell::xdg::SurfaceCachedState>();
                    let c = c.current();
                    c.min_size.w > 0 && c.min_size == c.max_size
                })
            }
            Role::X11(_) => false,
        }
    }

    /// (title, application id / class)
    pub fn title_and_app(&self) -> (String, String) {
        match self {
            Role::Xdg(t) => smithay::wayland::compositor::with_states(t.wl_surface(), |st| {
                let a = st.data_map.get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>().unwrap().lock().unwrap();
                (a.title.clone().unwrap_or_default(), a.app_id.clone().unwrap_or_default())
            }),
            Role::X11(x) => (x.title(), x.class()),
        }
    }
}

/// Logical sizes that make a whole number of device pixels (see `set_scale_grid`): 1 at an integer scale.
/// Device pixels per logical pixel for X11 windows, as f32 bits. X programs know nothing of fractional scales, so they get the
/// device's own pixels (one X pixel is one pixel of the screen, text sized by `Xft.dpi`) instead of being stretched and blurred.
/// 1.0 turns it off (`MEMRIUM_X11_STRETCH=1` or a desktop at 100%).
static X_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

pub fn set_x_scale(scale: f32) {
    let s = if std::env::var_os("MEMRIUM_X11_STRETCH").is_some() { 1.0 } else { scale.max(1.0) };
    X_SCALE.store(s.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

pub fn x_scale() -> f32 {
    f32::from_bits(X_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

/// Logical px -> X px.
pub fn to_x(v: i32) -> i32 {
    (v as f32 * x_scale()).round() as i32
}

/// X px -> logical px.
pub fn from_x(v: i32) -> f32 {
    v as f32 / x_scale()
}

static GRID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

/// At a fractional scale a window of an odd size has half a device pixel too many (1.5 x 1599 = 2398.5): the program rounds its
/// buffer, and the picture is stretched by a sliver, which blurs it all. Window sizes are therefore asked for in steps that make
/// whole pixels: 2 logical px at 1.5, 4 at 1.25.
pub fn set_scale_grid(scale: f32) {
    let k = (1..=8).find(|k| ((*k as f32) * scale - ((*k as f32) * scale).round()).abs() < 1e-3).unwrap_or(1);
    GRID.store(k, std::sync::atomic::Ordering::Relaxed);
}

/// A size rounded up to the grid (up: a maximized window then reaches past the screen's edge, where nothing shows, rather than
/// leaving a strip).
fn on_grid(v: i32) -> i32 {
    let k = GRID.load(std::sync::atomic::Ordering::Relaxed);
    if k <= 1 {
        v
    } else {
        (v + k - 1) / k * k
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_make_whole_device_pixels_at_a_fractional_scale() {
        set_scale_grid(1.5);
        assert_eq!((on_grid(1599), on_grid(1600), on_grid(861)), (1600, 1600, 862));
        set_scale_grid(1.25);
        assert_eq!((on_grid(1599), on_grid(1601), on_grid(1604)), (1600, 1604, 1604));
        // a whole scale, and one with no short grid, leave the sizes alone
        set_scale_grid(2.0);
        assert_eq!(on_grid(1599), 1599);
        set_scale_grid(1.0);
        assert_eq!(on_grid(777), 777);
        set_scale_grid(1.3333);
        assert_eq!(on_grid(777), 777);
        set_scale_grid(1.0);
    }
}
