//! The icons of every System Settings entry the pack had none for: Papirus' compositions on the pack's squircle plates.
//! `settings_icons <prefix> [save]`, see assets/menu_kit.rs. Run `kickoff_icons save` first: some are copies of its icons.
//!
//! KDE drops `-parts` from the end of a name before it goes on to the inherited theme, so a name here also answers for
//! every `<name>-*`: never add a short name like `battery` (it would replace the tray's 170 battery states), and run
//! tools/icon_fallback_guards.py after adding names.
use memrium_de::icondoc::{self, Appearance, Background, FillKind, Glass, IconDoc, Kind, Layer, Node, Props, Shape, Variants};
use memrium_de::iconrender::{self, GlassTune, Renderer};

include!("assets/improve_kit.rs");
include!("assets/menu_kit.rs");

fn solid() -> Glass { glass(0.85, 0.10, 0.3) }
fn pane() -> Glass { glass(0.6, 0.4, 0.2) }
fn ink() -> Glass { glass(0.35, 0.0, 0.1) }

/// A white glass symbol.
fn sym(d: &mut IconDoc, name: &str, body: &str) { add(d, name, body, solid(), 0.97); }
/// A frosted white pane behind it.
fn back(d: &mut IconDoc, name: &str, body: &str, opacity: f32) { add(d, name, body, pane(), opacity); }
/// A part in a colour of its own.
fn part(d: &mut IconDoc, name: &str, body: &str) { add(d, name, body, ink(), 1.0); }

/// The dark look of a drawing: every white layer takes `colour` (the frosted ones deeper, so the depth stays).
fn dark(d: &mut IconDoc, colour: &str) {
    let c = rgb(colour);
    let mut set: Vec<(String, String)> = Vec::new();
    for n in &d.nodes {
        let Kind::Layer(l) = &n.kind else { continue };
        if !l.svg.contains("\"#FFFFFF\"") { continue; }
        let k = 0.45 + 0.55 * n.props().default.opacity;
        let v = c.map(|v| ((v * k).min(1.0) * 255.0) as u8);
        set.push((n.name.clone(), format!("#{:02X}{:02X}{:02X}", v[0], v[1], v[2])));
    }
    let set: Vec<(&str, &str, f32)> = set.iter().map(|(n, c)| (n.as_str(), c.as_str(), 1.0)).collect();
    darken(d, None, &set);
}

fn magnifier(d: &mut IconDoc) {
    sym(d, "handle", &line("M300 300 L402 402", W, 70));
    sym(d, "lens", &line(&disc(218.0, 218.0, 118.0), W, 56));
}

fn keyboard(y: f32, hole_rows: usize) -> String {
    let mut p = rect(58.0, y, 396.0, 236.0, 46.0);
    for row in 0..hole_rows {
        for k in 0..5 { p += &rect(96.0 + k as f32 * 68.0, y + 34.0 + row as f32 * 64.0, 48.0, 44.0, 12.0); }
    }
    p + &rect(136.0, y + 166.0, 240.0, 40.0, 14.0)
}

fn hand(d: &mut IconDoc) {
    add(d, "hand", &soft("M226 226 Q226 196 252 196 Q278 196 278 226 V298 L380 320 Q416 330 410 366 L396 430 H242 L166 344 Q160 310 192 316 L226 342 Z", W, 14), solid(), 1.0);
}

fn port(y: f32) -> String {
    format!("M106 {} Q106 {y} 136 {y} H376 Q406 {y} 406 {} V{} H342 V{} H170 V{} H106 Z", y + 30.0, y + 30.0, y + 206.0, y + 260.0, y + 206.0)
}

fn icons() -> Vec<IconDoc> {
    let mut all: Vec<IconDoc> = Vec::new();
    let mut put = |name: &str, top: &str, bottom: &str, accent: &str, draw: &dyn Fn(&mut IconDoc)| {
        let mut d = if top == W { white_doc(name) } else { new_doc(name, top, bottom) };
        draw(&mut d);
        dark(&mut d, accent);
        if top == W { plain_tinted(&mut d); }
        own_tint(&mut d);
        all.push(d);
    };

    // --- the pages of the first screen
    put("preferences-desktop", W, W, W, &|d| {
        for (i, (x, y)) in [(138.0f32, 180.0f32), (256.0, 330.0), (374.0, 240.0)].into_iter().enumerate() {
            part(d, &format!("track {i}"), &line(&format!("M{x} 106 V406"), "#B9C2CE", 40));
            part(d, &format!("level {i}"), &line(&format!("M{x} {y} V406"), "#3584E4", 40));
            add(d, &format!("knob {i}"), &path(&disc(x, y, 56.0), "#1B5FC4"), glass(0.9, 0.1, 0.35), 1.0);
        }
    });

    // --- input and output
    for name in ["preferences-desktop-mouse", "preferences-desktop-peripherals"] {
        put(name, "#767C86", "#373C44", "#AEB6C2", &|d| {
            sym(d, "body", &path(&format!("{} {}", rect(136.0, 62.0, 240.0, 388.0, 118.0), rect(234.0, 116.0, 44.0, 96.0, 22.0)), W));
        });
    }
    put("preferences-desktop-touchpad", "#767C86", "#373C44", "#AEB6C2", &|d| {
        back(d, "pad", &path(&rect(72.0, 92.0, 368.0, 328.0, 52.0), W), 0.55);
        sym(d, "buttons", &path(&(rect(98.0, 308.0, 150.0, 86.0, 24.0) + &rect(264.0, 308.0, 150.0, 86.0, 24.0)), W));
        sym(d, "finger", &path(&disc(256.0, 200.0, 52.0), W));
    });
    put("preferences-desktop-keyboard", "#7B8796", "#39424E", "#AEBBCB", &|d| {
        sym(d, "keys", &path(&keyboard(138.0, 2), W));
    });
    put("preferences-desktop-keyboard-shortcut", "#F7B04C", "#D66A14", "#F59A32", &|d| {
        back(d, "key behind", &path(&rect(66.0, 72.0, 244.0, 244.0, 56.0), W), 0.55);
        sym(d, "key", &path(&rect(196.0, 196.0, 250.0, 250.0, 56.0), W));
        part(d, "arrow", &line("M262 372 L374 268 M290 264 H378 V350", "#C4600E", 38));
    });
    put("input-keyboard-virtual", "#4DB6E8", "#1766B8", "#4DA6E8", &|d| {
        sym(d, "keys", &path(&keyboard(82.0, 2), W));
        sym(d, "chevron", &line("M188 372 L256 430 L324 372", W, 44));
    });
    for name in ["preferences-desktop-touchscreen", "preferences-desktop-gestures-touch"] {
        put(name, "#3FA0F4", "#1560C4", "#4BA3F5", &|d| {
            back(d, "screen", &path(&rect(60.0, 92.0, 392.0, 276.0, 48.0), W), 0.36);
            if name.ends_with("gestures-touch") {
                part(d, "swipe", &line("M112 206 Q250 78 390 190 M326 190 H394 V124", "#9BE7FF", 30));
            } else {
                part(d, "touch", &line(&disc(252.0, 210.0, 74.0), "#9BE7FF", 30));
            }
            hand(d);
        });
    }
    put("preferences-desktop-tablet", "#6C737D", "#31363E", "#9CA6B3", &|d| {
        back(d, "tablet", &path(&rect(56.0, 98.0, 380.0, 280.0, 46.0), W), 0.6);
        part(d, "badge", &path(&disc(362.0, 354.0, 106.0), "#23A566"));
        add(d, "pen", &soft("M308 408 L320 358 L392 286 L430 324 L358 396 Z", "#FFFFFF", 14), solid(), 1.0);
    });
    put("preferences-desktop-sound", "#5F646C", "#2A2D32", "#8B939E", &|d| {
        back(d, "box", &path(&rect(112.0, 52.0, 288.0, 408.0, 56.0), W), 0.6);
        part(d, "tweeter", &path(&disc(256.0, 126.0, 30.0), "#2C2F34"));
        add(d, "cone", &path(&disc(256.0, 296.0, 118.0), "#FFC21C"), solid(), 1.0);
        part(d, "middle", &path(&disc(256.0, 296.0, 48.0), "#35383E"));
    });
    put("preferences-desktop-display", "#69727E", "#2B3038", "#AAB6C4", &|d| {
        back(d, "stand", &soft("M216 350 H296 L322 414 H190 Z", W, 22), 0.7);
        sym(d, "frame", &path(&rect(56.0, 86.0, 400.0, 284.0, 46.0), W));
        part(d, "screen", &path(&rect(90.0, 120.0, 332.0, 204.0, 20.0), "#1E232A"));
    });
    put("preferences-desktop-display-randr", "#4DA6F0", "#1C5FB8", "#58A9F2", &|d| {
        back(d, "second", &path(&rect(212.0, 84.0, 244.0, 186.0, 38.0), W), 0.55);
        sym(d, "first", &path(&(rect(56.0, 176.0, 300.0, 214.0, 42.0) + &rect(88.0, 208.0, 236.0, 150.0, 18.0)), W));
        sym(d, "foot", &line("M148 430 H264", W, 34));
    });
    put("redshift-status-on", "#FF7A4E", "#C9301C", "#FF6545", &|d| {
        sym(d, "bulb", &soft("M210 334 C210 286 136 266 136 184 C136 112 190 70 256 70 C322 70 376 112 376 184 C376 266 302 286 302 334 Z", W, 20));
        part(d, "base", &line("M216 386 H296 M232 432 H280", "#5B2218", 36));
    });
    put("preferences-desktop-accessibility", "#6474DE", "#2C379C", "#7C8BEE", &|d| {
        sym(d, "head", &path(&disc(256.0, 110.0, 54.0), W));
        sym(d, "body", &line("M96 200 H416 M256 204 V290 M256 290 L190 428 M256 290 L322 428", W, 58));
    });
    put("preferences-desktop-gestures-screenedges", "#3FA0F4", "#1560C4", "#4BA3F5", &|d| {
        back(d, "screen", &path(&rect(56.0, 100.0, 400.0, 312.0, 24.0), W), 0.4);
        sym(d, "corners", &soft("M68 112 H214 A146 146 0 0 1 68 258 Z M444 400 H298 A146 146 0 0 1 444 254 Z", W, 18));
    });

    // --- devices and network
    put("preferences-devices-printer", "#707780", "#34393F", "#A4ADB8", &|d| {
        back(d, "paper in", &path(&rect(142.0, 58.0, 228.0, 150.0, 26.0), W), 0.6);
        sym(d, "body", &path(&rect(58.0, 170.0, 396.0, 190.0, 44.0), W));
        part(d, "lamp", &path(&disc(116.0, 226.0, 22.0), "#52D726"));
        part(d, "slot", &path(&rect(132.0, 276.0, 248.0, 176.0, 22.0), "#3B4047"));
        part(d, "lines", &line("M178 336 H334 M178 394 H286", "#E8EBEE", 26));
    });
    put("preferences-system-disks", "#58C7C0", "#1D7F86", "#52BDB8", &|d| {
        back(d, "camera", &path(&rect(118.0, 70.0, 276.0, 196.0, 46.0), W), 0.55);
        part(d, "lens", &path(&(disc(256.0, 166.0, 62.0) + &disc(256.0, 166.0, 26.0)), "#12585E"));
        sym(d, "drive", &path(&rect(62.0, 280.0, 388.0, 154.0, 42.0), W));
        part(d, "lamp", &path(&disc(380.0, 357.0, 24.0), "#1D9A5B"));
        part(d, "slot", &line("M124 357 H270", "#8DA5A8", 26));
    });
    put("preferences-system-network-proxy", "#93A9C0", "#4A5E76", "#93A9C0", &|d| {
        back(d, "port", &path(&port(72.0), W), 0.6);
        sym(d, "arrows", &line("M118 396 H394 M338 342 L394 396 L338 450", W, 40));
        part(d, "pins", &line("M184 124 V174 M232 124 V174 M280 124 V174 M328 124 V174", "#F4BE3C", 24));
    });
    put("preferences-system-network-remote", "#8E7BEA", "#4A35B4", "#9A88F2", &|d| {
        back(d, "screen", &path(&rect(56.0, 82.0, 400.0, 290.0, 46.0), W), 0.5);
        back(d, "foot", &line("M180 424 H300", W, 36), 0.7);
        add(d, "pointer", &soft("M242 180 L408 330 L330 336 L374 422 L332 442 L290 354 L242 406 Z", W, 20), solid(), 1.0);
    });
    put("network-connect", "#36BDFA", "#0877CC", "#38B4F5", &|d| {
        sym(d, "globe", &path(&disc(256.0, 256.0, 196.0), W));
        part(d, "land", &soft("M110 172 L172 108 L236 98 L226 148 L270 182 L222 236 L168 228 L136 264 L92 250 Z M180 300 L246 292 L300 340 L262 384 L256 428 L222 426 L202 368 Z M334 118 L400 168 L428 232 L370 250 L334 208 L350 172 Z", "#118BDC", 18));
    });

    // --- appearance
    put("preferences-desktop-theme-global", "#42A0F2", "#1560C4", "#4BA3F5", &|d| {
        back(d, "desktop", &path(&rect(56.0, 86.0, 400.0, 340.0, 44.0), W), 0.36);
        sym(d, "icons", &path(&(rect(90.0, 124.0, 58.0, 58.0, 16.0) + &rect(90.0, 204.0, 58.0, 58.0, 16.0)), W));
        sym(d, "window", &path(&rect(196.0, 176.0, 218.0, 156.0, 28.0), W));
        part(d, "panel", &path(&rect(84.0, 356.0, 344.0, 44.0, 18.0), "#17365C"));
    });
    put("preferences-desktop-plasma-theme", "#42A0F2", "#1560C4", "#4BA3F5", &|d| {
        back(d, "desktop", &path(&rect(56.0, 86.0, 400.0, 340.0, 44.0), W), 0.36);
        part(d, "panel", &path(&rect(76.0, 296.0, 360.0, 110.0, 28.0), "#17365C"));
        sym(d, "tasks", &path(&(rect(102.0, 322.0, 144.0, 58.0, 18.0) + &rect(266.0, 322.0, 144.0, 58.0, 18.0)), W));
    });
    put("preferences-desktop-theme-applications", "#8A97A8", "#465264", "#A9B7C9", &|d| {
        sym(d, "window", &path(&rect(56.0, 82.0, 400.0, 348.0, 48.0), W));
        part(d, "checks", &path(&(disc(134.0, 164.0, 30.0) + &rect(202.0, 134.0, 60.0, 60.0, 14.0)), "#3B82E6"));
        part(d, "track", &line("M124 320 H388", "#9AA6B5", 40));
        part(d, "knob", &path(&disc(328.0, 320.0, 56.0), "#3B82E6"));
    });
    put("preferences-desktop-theme-windowdecorations", "#78909C", "#3D525C", "#9DB6C2", &|d| {
        back(d, "window", &path(&rect(56.0, 72.0, 400.0, 368.0, 46.0), W), 0.55);
        sym(d, "title", &path(&rect(56.0, 72.0, 400.0, 136.0, 46.0), W));
        for (i, c) in ["#FF5F57", "#FEBC2E", "#28C840"].iter().enumerate() {
            part(d, &format!("button {i}"), &path(&disc(130.0 + i as f32 * 92.0, 140.0, 34.0), c));
        }
    });
    put("preferences-desktop-cursors", "#A9AFB8", "#5C636D", "#C3CAD3", &|d| {
        sym(d, "pointer", &soft("M150 76 L398 298 L282 308 L346 418 L286 450 L228 338 L150 416 Z", W, 26));
    });
    put("preferences-desktop-font", "#B4B4B8", "#68686E", "#C6C6CC", &|d| {
        sym(d, "letter", &line("M140 150 C180 70 300 138 408 94 M300 136 C256 250 240 400 144 414 C86 420 88 350 134 350 M188 270 H344", W, 48));
    });
    put("preferences-desktop-font-installer", "#B4B4B8", "#68686E", "#C6C6CC", &|d| {
        sym(d, "letter", &line("M110 126 C150 60 240 116 338 80 M246 118 C208 214 196 340 116 354 C70 360 70 300 108 300 M154 228 H282", W, 44));
        part(d, "badge", &path(&disc(370.0, 370.0, 98.0), "#2A7BE0"));
        add(d, "arrow", &line("M370 316 V418 M328 378 L370 422 L412 378", W, 30), flat(), 1.0);
    });
    put("preferences-desktop-wallpaper", "#35C8F8", "#087FC4", "#35B8EE", &|d| {
        back(d, "picture", &path(&rect(54.0, 86.0, 404.0, 340.0, 48.0), W), 0.4);
        part(d, "sun", &path(&disc(172.0, 182.0, 58.0), "#FFE85C"));
        part(d, "hills", &soft("M78 356 C150 250 206 270 250 330 C296 250 366 236 434 310 V384 Q434 404 414 404 H98 Q78 404 78 384 Z", "#175A78", 8));
    });
    put("preferences-desktop-animations", "#A76AF0", "#5526B6", "#A675F2", &|d| {
        back(d, "first", &path(&disc(134.0, 256.0, 74.0), W), 0.3);
        back(d, "second", &path(&disc(226.0, 256.0, 96.0), W), 0.55);
        sym(d, "last", &path(&disc(338.0, 256.0, 116.0), W));
    });
    put("emblem-music-symbolic", "#FF8A5C", "#E0395C", "#FF7664", &|d| {
        sym(d, "stems", &line("M198 372 V128 L394 92 V330", W, 44));
        sym(d, "beam", &soft("M198 128 L394 92 V148 L198 184 Z", W, 20));
        sym(d, "notes", &path(&(disc(152.0, 378.0, 70.0) + &disc(348.0, 342.0, 70.0)), W));
    });

    // --- apps and windows
    put("preferences-desktop-default-applications", "#66CC6C", "#2A8E38", "#58BC60", &|d| {
        sym(d, "star", &soft("M256 70 L314 192 L446 208 L348 300 L374 432 L256 366 L138 432 L164 300 L66 208 L198 192 Z", W, 34));
        part(d, "letter", &line("M196 342 L256 178 L316 342 M222 294 H290", "#237C30", 36));
    });
    put("preferences-desktop-effects", "#FFCF4A", "#EE8A10", "#FFBE32", &|d| {
        sym(d, "star", &soft("M256 70 L314 192 L446 208 L348 300 L374 432 L256 366 L138 432 L164 300 L66 208 L198 192 Z", W, 34));
    });
    put("preferences-desktop-filetype-association", "#708C99", "#324B57", "#8FB0BF", &|d| {
        back(d, "page", &soft("M124 80 H286 L366 160 V420 H124 Z", W, 34), 0.55);
        sym(d, "gear", &soft(&gear(294.0, 308.0, 94.0, 132.0, 42.0, 7), W, 16));
    });
    put("preferences-desktop-notification-bell", "#FFD23A", "#EE9606", "#FFC21C", &|d| {
        part(d, "clapper", &path(&disc(256.0, 402.0, 50.0), "#E8590C"));
        sym(d, "bell", &soft("M96 378 Q150 340 150 220 Q150 130 222 114 A36 36 0 0 1 290 114 Q362 130 362 220 Q362 340 416 378 Z", W, 26));
    });
    put("preferences-desktop-activities", "#3C5C80", "#16283E", "#4C6C92", &|d| {
        back(d, "quiet", &path(&rect(62.0, 272.0, 388.0, 166.0, 46.0), W), 0.3);
        add(d, "current", &path(&rect(62.0, 74.0, 388.0, 166.0, 46.0), "#2D9BF5"), solid(), 1.0);
    });
    put("preferences-desktop-virtual", "#3E74E0", "#173C9E", "#4F82E8", &|d| {
        back(d, "others", &path(&(rect(66.0, 80.0, 180.0, 166.0, 38.0) + &rect(266.0, 80.0, 180.0, 166.0, 38.0) + &rect(266.0, 266.0, 180.0, 166.0, 38.0)), W), 0.4);
        sym(d, "current", &path(&rect(66.0, 266.0, 180.0, 166.0, 38.0), W));
    });

    // --- workspace, search
    for name in ["preferences-desktop-search", "preferences-desktop-baloo"] {
        put(name, "#27A8FF", "#0A64D0", "#2A9CF5", &|d| magnifier(d));
    }
    for name in ["plasma-search", "krunner"] {
        put(name, "#F7BE50", "#D6821A", "#F0A83A", &|d| magnifier(d));
    }

    // --- security, people, language, system
    put("preferences-security", "#45CE92", "#12805A", "#3CBF86", &|d| {
        sym(d, "shield", &soft("M256 66 L418 128 V240 C418 338 352 414 256 448 C160 414 94 338 94 240 V128 Z", W, 26));
        part(d, "tick", &line("M182 256 L236 310 L336 200", "#12805A", 46));
    });
    put("preferences-desktop-user-password", "#FFC94A", "#E08A12", "#FFC040", &|d| {
        back(d, "shackle", &line("M164 232 V174 C164 56 348 56 348 174 V232", W, 48), 0.75);
        sym(d, "body", &path(&rect(98.0, 214.0, 316.0, 232.0, 54.0), W));
        part(d, "keyhole", &soft("M256 298 L272 372 H240 Z M228 298 A28 28 0 1 0 284 298 A28 28 0 1 0 228 298 Z", "#B96A08", 10).replace("evenodd", "nonzero"));
    });
    put("preferences-desktop-user", "#16B3A3", "#006F66", "#1AA898", &|d| {
        sym(d, "head", &path(&disc(256.0, 162.0, 88.0), W));
        sym(d, "body", &soft("M112 418 C104 304 180 280 256 280 C332 280 408 304 400 418 Z", W, 26));
    });
    put("preferences-desktop-feedback", "#63D84A", "#239A2C", "#5CCB42", &|d| {
        sym(d, "bubble", &soft("M256 78 C376 78 446 146 446 240 C446 336 366 400 256 400 C224 400 194 396 166 386 L90 432 L108 344 C80 316 66 282 66 240 C66 146 136 78 256 78 Z", W, 14));
        part(d, "lines", &line("M166 184 H346 M166 244 H346 M166 304 H300", "#2C9E34", 30));
    });
    put("preferences-desktop-locale", "#5A96F8", "#2254C8", "#6A9EF8", &|d| {
        back(d, "paper", &path(&rect(220.0, 176.0, 236.0, 270.0, 40.0), W), 0.5);
        part(d, "sign", &line("M274 260 H414 M344 226 V258 M386 262 C376 326 330 372 278 398 M300 264 C312 326 358 372 410 398", "#183F9C", 26));
        sym(d, "card", &path(&rect(56.0, 66.0, 244.0, 276.0, 40.0), W));
        part(d, "letter", &line("M122 268 L178 130 L234 268 M144 228 H212", "#2D66DC", 34));
    });
    put("lighttable", "#7183E4", "#29337E", "#8494F0", &|d| {
        back(d, "night", &path(&disc(256.0, 256.0, 192.0), W), 0.3);
        sym(d, "day", &path("M120.2 391.8 A192 192 0 0 1 391.8 120.2 Z", W));
    });
    put("tools-check-spelling", "#4CC3B4", "#16786E", "#4CC3B4", &|d| {
        sym(d, "a", &line("M204 170 V288 M204 206 C170 154 86 162 86 230 C86 298 170 304 204 254", W, 42));
        sym(d, "b", &line("M284 92 V288 M284 206 C318 154 422 162 422 230 C422 298 318 304 284 254", W, 42));
        part(d, "wave", &line("M88 390 L132 354 L176 390 L220 354 L264 390 L308 354 L352 390 L396 354 L428 380", "#FFE066", 30));
    });
    put("preferences-system-login", "#74CC4A", "#36882A", "#6CC044", &|d| {
        back(d, "fins", &soft("M208 220 L116 228 L80 308 L176 290 Z M292 304 L284 396 L204 432 L222 336 Z", W, 18), 0.65);
        sym(d, "rocket", &soft("M426 86 C326 76 222 150 164 296 L216 348 C362 290 436 186 426 86 Z", W, 22));
        part(d, "window", &path(&disc(322.0, 190.0, 40.0), "#36882A"));
        part(d, "flame", &soft("M142 346 C100 360 82 402 78 434 C110 430 152 412 166 370 Z", "#FFB52E", 14));
    });
    put("ktip", "#58B4FF", "#1C72D6", "#4CA6F5", &|d| {
        sym(d, "dot", &path(&disc(256.0, 120.0, 50.0), W));
        sym(d, "stem", &path(&rect(212.0, 200.0, 88.0, 240.0, 36.0), W));
    });
    put("xorg", "#686C72", "#2A2C30", "#A0A6AE", &|d| {
        sym(d, "cross", &line("M122 122 L390 390 M390 122 L122 390", W, 84));
    });

    for (of, name) in [
        ("applications-games", "preferences-desktop-gaming"),
        ("applications-graphics", "preferences-desktop-color"),
        ("applications-multimedia", "preferences-desktop-multimedia"),
        ("applications-internet", "preferences-web-browser-shortcuts"),
    ] {
        // (before kickoff_icons has saved, the preview simply goes without these)
        if icondoc::doc_path(of).exists() { all.push(copy_of(of, name)); }
    }
    all
}

fn main() {
    run(icons());
}
