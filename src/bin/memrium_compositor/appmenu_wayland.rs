//! `org_kde_kwin_appmenu`: how Qt/KDE programs running natively under Wayland say where their menu is. A program makes an
//! appmenu object for a surface and gives it the bus name and path of its `com.canonical.dbusmenu`; the shell remembers
//! that per surface and reads the menu through the dbusmenu bridge when the window is in front.

use crate::state::State;
use smithay::reexports::wayland_server::{
    protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use wayland_protocols_plasma::appmenu::server::{
    org_kde_kwin_appmenu::{self, OrgKdeKwinAppmenu},
    org_kde_kwin_appmenu_manager::{self, OrgKdeKwinAppmenuManager},
};

/// The surface an appmenu object belongs to.
pub struct AppMenuData {
    surface: WlSurface,
}

/// The address of a surface's menu: (surface, bus name, object path).
pub type MenuAddress = (WlSurface, String, String);

pub fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, OrgKdeKwinAppmenuManager, ()>(1, ());
}

impl GlobalDispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn bind(_: &mut State, _: &DisplayHandle, _: &Client, resource: New<OrgKdeKwinAppmenuManager>, _: &(), init: &mut DataInit<'_, State>) {
        init.init(resource, ());
    }
}

impl Dispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn request(_: &mut State, _: &Client, _: &OrgKdeKwinAppmenuManager, request: org_kde_kwin_appmenu_manager::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, State>) {
        if let org_kde_kwin_appmenu_manager::Request::Create { id, surface } = request {
            init.init(id, AppMenuData { surface });
        }
    }
}

impl Dispatch<OrgKdeKwinAppmenu, AppMenuData> for State {
    fn request(state: &mut State, _: &Client, _: &OrgKdeKwinAppmenu, request: org_kde_kwin_appmenu::Request, data: &AppMenuData, _: &DisplayHandle, _: &mut DataInit<'_, State>) {
        if let org_kde_kwin_appmenu::Request::SetAddress { service_name, object_path } = request {
            let list = &mut state.protocols.appmenu;
            list.retain(|a| a.0 != data.surface);
            list.push((data.surface.clone(), service_name, object_path));
            state.invalidate();
        }
    }

    fn destroyed(state: &mut State, _: smithay::reexports::wayland_server::backend::ClientId, _: &OrgKdeKwinAppmenu, data: &AppMenuData) {
        state.protocols.appmenu.retain(|a| a.0 != data.surface);
        state.invalidate();
    }
}

impl State {
    /// Where the surface's menu is, when it said.
    pub fn menu_address(&self, surface: &WlSurface) -> Option<(String, String)> {
        let _ = surface.id();
        self.protocols.appmenu.iter().find(|a| &a.0 == surface).map(|a| (a.1.clone(), a.2.clone()))
    }
}
