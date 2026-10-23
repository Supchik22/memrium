//! The system tray: icons that applications put on the panel through the StatusNotifierItem protocol (a chat client's
//! unread count, a sync tool, a VPN). This file is what the panel and the compositor share: what an icon is and how
//! its picture is turned into something that can be drawn. The D-Bus side lives in the compositor.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayStatus {
    /// nothing to say: tucked away behind the "more" arrow
    Passive,
    #[default]
    Active,
    /// wants to be noticed
    Attention,
}

impl TrayStatus {
    pub fn parse(s: &str) -> TrayStatus {
        match s {
            "Passive" => TrayStatus::Passive,
            "NeedsAttention" => TrayStatus::Attention,
            _ => TrayStatus::Active,
        }
    }
}

/// A picture the application sent: RGBA, `w` x `h`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pix {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayIcon {
    /// the item's bus name and object path, `:1.42/StatusNotifierItem`
    pub id: String,
    pub title: String,
    pub tooltip: String,
    pub status: TrayStatus,
    /// a theme icon name (or an absolute path), a directory of the application's own icons, and a picture
    pub icon_name: String,
    pub icon_dir: String,
    pub pixmap: Option<Pix>,
    /// the object of the item's menu (`com.canonical.dbusmenu`), when it has one
    pub menu: String,
    /// the item has nothing but a menu: a left click opens it
    pub is_menu: bool,
    /// the icon atlas cell the compositor made for it
    pub cell: Option<u32>,
}

/// `(bus name, object path)` of an item id.
pub fn split_id(id: &str) -> (&str, &str) {
    match id.find('/') {
        Some(i) => (&id[..i], &id[i..]),
        None => (id, "/StatusNotifierItem"),
    }
}

/// What an application's registration call means: the item's id, from the name it registered and who sent it.
/// Some send a bus name (the object is then `/StatusNotifierItem`), some an object path (the bus name is the sender's).
pub fn item_id(registered: &str, sender: &str) -> String {
    if registered.starts_with('/') {
        format!("{sender}{registered}")
    } else if registered.contains('/') {
        registered.to_string()
    } else {
        format!("{registered}/StatusNotifierItem")
    }
}

/// The pictures of `IconPixmap` are ARGB, a byte per channel in network order. A tray icon is drawn small, so the
/// smallest picture that is at least 48 px wide is taken (some applications send a 512 px one, a megabyte that would be
/// copied around for nothing); failing that, the biggest there is. None when there is none that makes sense.
pub fn best_pixmap(list: Vec<(i32, i32, Vec<u8>)>) -> Option<Pix> {
    let mut ok: Vec<(i32, i32, Vec<u8>)> = list.into_iter().filter(|(w, h, d)| *w > 0 && *h > 0 && d.len() >= (*w as usize) * (*h as usize) * 4).collect();
    ok.sort_by_key(|(w, h, _)| w * h);
    let at = ok.iter().position(|(w, _, _)| *w >= 48).or(ok.len().checked_sub(1));
    at.map(|i| ok.swap_remove(i))
        .map(|(w, h, d)| {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for px in d[..(w * h * 4) as usize].chunks_exact(4) {
                rgba.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
            }
            Pix { w: w as u32, h: h as u32, rgba }
        })
}

/// A menu label of the `dbusmenu` kind: `_File` marks a mnemonic, `__` is a real underscore.
pub fn clean_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '_' {
            if it.peek() == Some(&'_') {
                out.push('_');
                it.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registrations_are_given_an_id_either_way() {
        assert_eq!(item_id("org.kde.foo", ":1.9"), "org.kde.foo/StatusNotifierItem");
        assert_eq!(item_id("/org/ayatana/NotificationItem/x", ":1.9"), ":1.9/org/ayatana/NotificationItem/x");
        assert_eq!(item_id(":1.7/StatusNotifierItem", ":1.9"), ":1.7/StatusNotifierItem");
        assert_eq!(split_id(":1.9/org/x"), (":1.9", "/org/x"));
    }

    #[test]
    fn a_picture_about_the_right_size_is_taken_and_its_channels_reordered() {
        let small = (1, 1, vec![255, 1, 2, 3]);
        let big = (2, 1, vec![128, 10, 20, 30, 255, 40, 50, 60]);
        let p = best_pixmap(vec![small, big]).unwrap();
        // none is as wide as 48: the biggest
        assert_eq!((p.w, p.h), (2, 1));
        assert_eq!(p.rgba, vec![10, 20, 30, 128, 40, 50, 60, 255]);
        assert!(best_pixmap(vec![(2, 2, vec![0; 4])]).is_none());
        // 512 is not taken when 64 will do
        let sized = |n: i32| (n, 1, vec![0u8; n as usize * 4]);
        assert_eq!(best_pixmap(vec![sized(512), sized(64), sized(16)]).unwrap().w, 64);
    }

    #[test]
    fn mnemonic_underscores_go() {
        assert_eq!(clean_label("_Quit"), "Quit");
        assert_eq!(clean_label("Snake__case"), "Snake_case");
    }
}
