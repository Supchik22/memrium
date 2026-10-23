//! `org_kde_kwin_blur`: Qt/KDE programs ask for the window behind them to be blurred (a translucent terminal, an editor
//! with a see-through background) and, without this global, hide the option. The renderer already frosts the backdrop of
//! every translucent window, so the requests are accepted and need nothing more.

use crate::state::State;
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New};
use wayland_protocols_plasma::blur::server::{
    org_kde_kwin_blur::{self, OrgKdeKwinBlur},
    org_kde_kwin_blur_manager::{self, OrgKdeKwinBlurManager},
};

pub fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, OrgKdeKwinBlurManager, ()>(1, ());
}

impl GlobalDispatch<OrgKdeKwinBlurManager, ()> for State {
    fn bind(_: &mut State, _: &DisplayHandle, _: &Client, resource: New<OrgKdeKwinBlurManager>, _: &(), init: &mut DataInit<'_, State>) {
        init.init(resource, ());
    }
}

impl Dispatch<OrgKdeKwinBlurManager, ()> for State {
    fn request(_: &mut State, _: &Client, _: &OrgKdeKwinBlurManager, request: org_kde_kwin_blur_manager::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, State>) {
        if let org_kde_kwin_blur_manager::Request::Create { id, .. } = request {
            init.init(id, ());
        }
    }
}

impl Dispatch<OrgKdeKwinBlur, ()> for State {
    fn request(_: &mut State, _: &Client, _: &OrgKdeKwinBlur, _: org_kde_kwin_blur::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, State>) {}
}
