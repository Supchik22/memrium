//! `gtk_shell1`: GTK's private protocol. A GTK program running natively under Wayland says through it where its menus
//! are on the session bus (`org.gtk.Menus` at the menu bar's path, `org.gtk.Actions` at the application's and the window's
//! path). The shell remembers that per surface; `gtkmenu.rs` reads the menu when the window is in front.

use crate::state::State;
use smithay::reexports::wayland_server::{
    self, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

mod gen {
    use smithay::reexports::wayland_server;
    use wayland_server::protocol::*;
    pub mod __interfaces {
        use smithay::reexports::wayland_server::{backend as wayland_backend, protocol::__interfaces::*};
        wayland_scanner::generate_interfaces!("protocols/gtk-shell.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_server_code!("protocols/gtk-shell.xml");
}
use gen::{
    gtk_shell1::{self, GtkShell1},
    gtk_surface1::{self, GtkSurface1},
};

/// Where one surface's menus are on the bus.
#[derive(Clone, Debug, PartialEq)]
pub struct GtkAddress {
    pub surface: WlSurface,
    /// the program's unique bus name
    pub service: String,
    /// `org.gtk.Menus` of the menu bar
    pub menubar: String,
    /// `org.gtk.Actions` of the window (`win.` actions) and of the application (`app.` actions)
    pub window: Option<String>,
    pub app: Option<String>,
}

pub struct GtkSurfaceData {
    surface: WlSurface,
}

pub fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, GtkShell1, ()>(5, ());
}

/// Bit of `gtk_shell1.capabilities`: the shell shows the menu bar itself, so programs keep it out of their windows.
const GLOBAL_MENU_BAR: u32 = 2;

impl GlobalDispatch<GtkShell1, ()> for State {
    fn bind(_: &mut State, _: &DisplayHandle, _: &Client, resource: New<GtkShell1>, _: &(), init: &mut DataInit<'_, State>) {
        let shell = init.init(resource, ());
        shell.capabilities(GLOBAL_MENU_BAR);
    }
}

impl Dispatch<GtkShell1, ()> for State {
    fn request(_: &mut State, _: &Client, _: &GtkShell1, request: gtk_shell1::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, State>) {
        if let gtk_shell1::Request::GetGtkSurface { gtk_surface, surface } = request {
            init.init(gtk_surface, GtkSurfaceData { surface });
        }
    }
}

impl Dispatch<GtkSurface1, GtkSurfaceData> for State {
    fn request(state: &mut State, _: &Client, _: &GtkSurface1, request: gtk_surface1::Request, data: &GtkSurfaceData, _: &DisplayHandle, _: &mut DataInit<'_, State>) {
        if let gtk_surface1::Request::SetDbusProperties { menubar_path, window_object_path, application_object_path, unique_bus_name, .. } = request {
            let list = &mut state.protocols.gtk_menus;
            list.retain(|a| a.surface != data.surface);
            if let (Some(service), Some(menubar)) = (unique_bus_name, menubar_path) {
                list.push(GtkAddress { surface: data.surface.clone(), service, menubar, window: window_object_path, app: application_object_path });
            }
            state.invalidate();
        }
    }

    fn destroyed(state: &mut State, _: wayland_server::backend::ClientId, _: &GtkSurface1, data: &GtkSurfaceData) {
        state.protocols.gtk_menus.retain(|a| a.surface != data.surface);
        state.invalidate();
    }
}

impl State {
    /// Where the surface's GTK menus are, when its program said.
    pub fn gtk_address(&self, surface: &WlSurface) -> Option<GtkAddress> {
        let _ = surface.id();
        self.protocols.gtk_menus.iter().find(|a| &a.surface == surface).cloned()
    }
}
