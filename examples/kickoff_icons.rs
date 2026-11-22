//! The icons of the Plasma Kickoff menu (categories, tabs, Places, Shut Down): Papirus' compositions on the pack's plates.
//! `kickoff_icons <prefix>` writes previews (every look, and the size of the menu); `kickoff_icons <prefix> save` saves
//! the documents and writes only these icons into the themes (no full rebuild, so nothing else is redrawn or pruned).
//!
//! The names have no `-symbolic`: KDE drops that suffix before it goes on to the inherited theme. `help-about-symbolic`
//! is the exception, so that the plain `help-about` of every application's About item stays as it is.
use memrium_de::icondoc::{self, Appearance, Background, FillKind, Glass, IconDoc, Kind, Layer, Node, Props, Shape, Variants};
use memrium_de::iconrender::{self, GlassTune, Renderer};

include!("assets/improve_kit.rs");

include!("assets/menu_kit.rs");

fn icons() -> Vec<IconDoc> {
    let solid = || glass(0.85, 0.10, 0.35);
    let pane = || glass(0.6, 0.4, 0.2);
    let mut all = Vec::new();

    let mut d = new_doc("bookmarks", "#FF7BA9", "#D8306C");
    add(&mut d, "ribbon", &soft("M164 102 H348 V410 L256 338 L164 410 Z", W, 44), solid(), 0.97);
    darken(&mut d, None, &[("ribbon", "#FF5C94", 1.0)]);
    all.push(d);

    let mut d = new_doc("applications-all", "#4A97BC", "#1F5270");
    add(&mut d, "quiet tiles", &path(&(rect(270.0, 78.0, 164.0, 164.0, 44.0) + &rect(78.0, 270.0, 164.0, 164.0, 44.0)), W), pane(), 0.5);
    add(&mut d, "tiles", &path(&(rect(78.0, 78.0, 164.0, 164.0, 44.0) + &rect(270.0, 270.0, 164.0, 164.0, 44.0)), W), solid(), 0.97);
    darken(&mut d, None, &[("quiet tiles", "#2B708F", 1.0), ("tiles", "#7CCBEF", 1.0)]);
    all.push(d);

    let mut d = new_doc("applications-development", "#B5826A", "#6B4434");
    add(&mut d, "handle", &line("M222 206 L390 400", "#F3D2B8", 72), glass(0.6, 0.15, 0.3), 0.95);
    add(&mut d, "head", &soft("M100 218 L210 120 C242 96 288 92 330 120 C292 128 268 144 258 170 L270 184 L200 248 L182 228 L142 264 Z", W, 26), solid(), 0.98);
    darken(&mut d, None, &[("handle", "#B77A5B", 1.0), ("head", "#E6D8D0", 1.0)]);
    all.push(d);

    let mut d = new_doc("applications-science", "#57D9CE", "#1F9F97");
    let orbit = "M66 256 a190 78 0 1 0 380 0 a190 78 0 1 0 -380 0 Z";
    for (name, turn) in [("orbit a", 90), ("orbit b", 30), ("orbit c", 150)] {
        add(&mut d, name, &format!("<path d=\"{orbit}\" transform=\"rotate({turn} 256 256)\" fill=\"none\" stroke=\"{W}\" stroke-width=\"30\"/>"), glass(0.7, 0.1, 0.15), 0.96);
    }
    add(&mut d, "nucleus", &path(&disc(256.0, 256.0, 56.0), "#174F55"), glass(0.6, 0.0, 0.2), 1.0);
    darken(&mut d, None, &[("orbit a", "#57D9CE", 1.0), ("orbit b", "#57D9CE", 1.0), ("orbit c", "#57D9CE", 1.0), ("nucleus", "#E9FFFD", 1.0)]);
    all.push(d);

    let mut d = new_doc("applications-education", "#6C7B8F", "#323C49");
    add(&mut d, "cap", &soft("M140 262 L256 320 L372 262 V356 C310 408 202 408 140 356 Z", W, 22), pane(), 0.62);
    add(&mut d, "board", &soft("M256 102 L442 200 L256 298 L70 200 Z", W, 28), solid(), 0.98);
    add(&mut d, "cord", &line("M256 198 L104 234 V324", "#FFB02E", 24), glass(0.4, 0.0, 0.15), 1.0);
    add(&mut d, "tassel", &soft("M104 318 L76 398 H132 Z", "#FF8A1E", 20), glass(0.5, 0.0, 0.2), 1.0);
    darken(&mut d, None, &[("cap", "#5C6878", 1.0), ("board", "#9AA8BA", 1.0)]);
    all.push(d);

    let mut d = white_doc("applications-graphics");
    for (i, c) in ["#A44CD3", "#14A9F5", "#0A9E8C", "#FFC21C", "#FF8A00", "#F8463C"].iter().enumerate() {
        let a = i as f32 * std::f32::consts::TAU / 6.0 - std::f32::consts::FRAC_PI_2;
        add(&mut d, &format!("petal {i}"), &path(&disc(256.0 + 108.0 * a.cos(), 256.0 + 108.0 * a.sin(), 88.0), c), glass(0.6, 0.12, 0.15), 0.94);
    }
    add(&mut d, "middle", &path(&disc(256.0, 256.0, 40.0), W), flat(), 1.0);
    darken(&mut d, None, &[]);
    plain_tinted(&mut d);
    all.push(d);

    for name in ["applications-internet", "applications-network"] {
        let mut d = new_doc(name, "#6478EE", "#2C3BAE");
        add(&mut d, "planet", &path(&disc(256.0, 256.0, 142.0), W), solid(), 0.97);
        // the ring: an ellipse with a hole, its far side behind the planet
        add(&mut d, "ring", &line("M140 222 C28 272 30 356 174 338 C282 324 420 258 452 196 C472 150 420 138 374 154", "#49B4FF", 42), glass(0.6, 0.05, 0.2), 1.0);
        darken(&mut d, None, &[("planet", "#8C9CFF", 1.0)]);
        all.push(d);
    }

    let mut d = new_doc("applications-multimedia", "#B564D2", "#6E2A8E");
    let mut film = rect(62.0, 100.0, 338.0, 274.0, 44.0);
    for y in [134.0, 210.0, 286.0] { film += &rect(90.0, y, 42.0, 50.0, 12.0); }
    add(&mut d, "film", &path(&film, W), pane(), 0.45);
    add(&mut d, "play", &soft("M186 170 V304 L298 237 Z", W, 30), solid(), 0.98);
    add(&mut d, "stem", &line("M300 384 V258 L420 224 V350", "#FF6A2B", 36), glass(0.5, 0.0, 0.2), 1.0);
    add(&mut d, "notes", &path(&(disc(266.0, 388.0, 52.0) + &disc(388.0, 356.0, 52.0)), "#FFA424"), glass(0.6, 0.0, 0.2), 1.0);
    darken(&mut d, None, &[("film", "#A64FC4", 1.0), ("play", "#F5E3FB", 1.0)]);
    all.push(d);

    let mut d = new_doc("applications-office", "#E3A567", "#A05E2B");
    add(&mut d, "grip", &line("M186 170 V126 Q186 100 212 100 H300 Q326 100 326 126 V170", W, 40), solid(), 0.8);
    add(&mut d, "case", &path(&rect(72.0, 156.0, 368.0, 264.0, 52.0), W), pane(), 0.6);
    add(&mut d, "flap", &path(&rect(72.0, 156.0, 368.0, 152.0, 52.0), W), solid(), 0.96);
    add(&mut d, "clasp", &path(&rect(208.0, 274.0, 96.0, 66.0, 22.0), "#8A4A1C"), glass(0.4, 0.0, 0.1), 1.0);
    darken(&mut d, None, &[("grip", "#B8713A", 1.0), ("case", "#A05E2B", 1.0), ("flap", "#DE9A5A", 1.0), ("clasp", "#FFF1E2", 1.0)]);
    all.push(d);

    let mut d = new_doc("applications-other", "#66699C", "#38375F");
    add(&mut d, "pane", &path(&disc(256.0, 256.0, 206.0), W), glass(0.9, 0.4, 0.35), 0.18);
    add(&mut d, "dots", &path(&(disc(132.0, 256.0, 54.0) + &disc(256.0, 256.0, 54.0) + &disc(380.0, 256.0, 54.0)), "#62BEFF"), solid(), 1.0);
    darken(&mut d, None, &[("dots", "#2E9BFF", 1.0), ("pane", "#66699C", 0.4)]);
    all.push(d);

    let mut d = new_doc("applications-system", "#6C8C9C", "#2F4B57");
    add(&mut d, "gear", &soft(&gear(256.0, 256.0, 142.0, 192.0, 70.0, 7), W, 22), solid(), 0.97);
    darken(&mut d, None, &[("gear", "#8FB4C5", 1.0)]);
    all.push(d);

    let mut d = new_doc("applications-utilities", "#F25766", "#A51D2C");
    add(&mut d, "grip", &line("M178 180 V122 Q178 96 204 96 H308 Q334 96 334 122 V180", W, 40), solid(), 0.8);
    add(&mut d, "box", &path(&rect(70.0, 172.0, 372.0, 250.0, 48.0), W), pane(), 0.62);
    add(&mut d, "lid", &path(&rect(70.0, 172.0, 372.0, 112.0, 48.0), W), solid(), 0.97);
    add(&mut d, "latches", &path(&(rect(134.0, 238.0, 70.0, 94.0, 20.0) + &rect(308.0, 238.0, 70.0, 94.0, 20.0)), "#A51D2C"), glass(0.4, 0.0, 0.1), 1.0);
    darken(&mut d, None, &[("grip", "#C9CDD4", 1.0), ("box", "#AC2231", 1.0), ("lid", "#EC4655", 1.0), ("latches", "#FFE3E6", 1.0)]);
    all.push(d);

    let mut d = new_doc("compass", "#5CB4F4", "#2163C2");
    add(&mut d, "ring", &line(&disc(256.0, 256.0, 176.0), W, 44), solid(), 0.97);
    add(&mut d, "needle south", &soft("M350 162 L294 294 L218 218 Z", "#FF5248", 14), glass(0.6, 0.0, 0.2), 1.0);
    add(&mut d, "needle north", &soft("M162 350 L218 218 L294 294 Z", W, 14), glass(0.6, 0.0, 0.2), 1.0);
    darken(&mut d, None, &[("ring", "#4FA8F0", 1.0)]);
    all.push(d);

    for name in ["view-history", "document-open-recent"] {
        let mut d = new_doc(name, "#9887F5", "#5040C4");
        add(&mut d, "face", &path(&disc(256.0, 256.0, 192.0), W), solid(), 0.97);
        add(&mut d, "hands", &line("M256 140 V262 L338 326", "#4939B8", 46), glass(0.4, 0.0, 0.1), 1.0);
        darken(&mut d, None, &[("face", "#8C7CF0", 1.0), ("hands", "#FFFFFF", 1.0)]);
        all.push(d);
    }

    let mut d = new_doc("computer", "#8DA2B8", "#46596E");
    add(&mut d, "stand", &soft("M216 350 H296 L322 412 H190 Z", W, 22), pane(), 0.7);
    add(&mut d, "screen frame", &path(&rect(62.0, 92.0, 388.0, 276.0, 44.0), W), solid(), 0.97);
    add(&mut d, "screen", &path(&rect(96.0, 126.0, 320.0, 192.0, 20.0), "#283646"), glass(0.5, 0.0, 0.1), 1.0);
    darken(&mut d, None, &[("stand", "#6F8296", 1.0), ("screen frame", "#A9BACB", 1.0)]);
    all.push(d);

    let mut d = new_doc("drive-harddisk", "#A7B0BC", "#5C6572");
    add(&mut d, "case", &path(&rect(116.0, 64.0, 280.0, 384.0, 48.0), W), pane(), 0.6);
    add(&mut d, "platter", &path(&(disc(256.0, 206.0, 104.0) + &disc(256.0, 206.0, 36.0)), W), solid(), 0.98);
    add(&mut d, "arm", &line("M184 380 L262 318", "#3B434E", 38), glass(0.4, 0.0, 0.1), 1.0);
    darken(&mut d, None, &[("case", "#4A535F", 1.0), ("platter", "#BFC8D3", 1.0), ("arm", "#FFFFFF", 1.0)]);
    all.push(d);

    let mut d = new_doc("drive-removable-media", "#A7B0BC", "#5C6572");
    add(&mut d, "case", &path(&rect(116.0, 64.0, 280.0, 384.0, 48.0), W), pane(), 0.5);
    add(&mut d, "eject", &soft("M256 150 L338 254 H174 Z M178 336 H334 V356 H178 Z", W, 26), solid(), 0.98);
    darken(&mut d, None, &[("case", "#4A535F", 1.0), ("eject", "#C9D2DC", 1.0)]);
    all.push(d);

    let mut d = new_doc("system-run", "#5FD18A", "#208E50");
    add(&mut d, "small gear", &soft(&gear(156.0, 154.0, 62.0, 90.0, 28.0, 6), W, 14), pane(), 0.75);
    add(&mut d, "big gear", &soft(&gear(308.0, 316.0, 100.0, 138.0, 44.0, 7), W, 18), solid(), 0.97);
    darken(&mut d, None, &[("small gear", "#2F9E5C", 1.0), ("big gear", "#6BDC95", 1.0)]);
    all.push(d);

    all
}

/// Two are icons the pack may have under another name (Papirus draws both alike): without those they are left out.
fn copies() -> Vec<IconDoc> {
    [("system-shut-down", "system-shutdown"), ("system-help", "help-about-symbolic")]
        .into_iter()
        .filter(|(of, _)| icondoc::doc_path(of).exists())
        .map(|(of, name)| copy_of(of, name))
        .collect()
}

fn main() {
    let mut docs = icons();
    docs.iter_mut().for_each(own_tint);
    docs.extend(copies());
    run(docs);
}
