//! Papirus icons that get one kind of treatment, in a sheet (original and in the style): `kind_sheet <out.png> <white|dark|tile> [count]`.
//! The ones about messages and social networks come first.
use memrium_de::icondoc::{self, Appearance};
use memrium_de::iconrender::{GlassTune, Renderer};
use memrium_de::iconstyle::{self, StyleKind};
use memrium_de::{svgedit, wallpapers};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let want_kind = match args.get(2).map(String::as_str) {
        Some("dark") => StyleKind::DarkPlate,
        Some("tile") => StyleKind::Tile,
        _ => StyleKind::WhiteMark,
    };
    let count: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(24);
    let home = std::env::var("HOME").unwrap_or_default();
    let dir = std::path::PathBuf::from(home).join(".local/share/icons/Papirus/64x64/apps");
    let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().filter_map(|e| {
        let p = e.path();
        (p.extension().is_some_and(|x| x == "svg")).then(|| p.file_stem().unwrap().to_string_lossy().to_string())
    }).collect();
    names.sort();
    let done = icondoc::saved_names();
    let liked = ["signal", "whatsapp", "skype", "messenger", "element", "matrix", "zulip", "mattermost", "twitter", "facebook", "instagram", "youtube", "twitch", "reddit", "pinterest", "tumblr", "line", "wechat", "qq", "icq", "zoom", "teams", "slack", "gitter", "tox", "jami", "linphone", "mumble", "teamspeak", "tweet", "mastodon", "mail", "thunderbird", "evolution", "kmail", "chat", "message", "im", "irc", "hexchat"];
    let rank = |n: &str| liked.iter().position(|k| n.to_lowercase().contains(k)).unwrap_or(usize::MAX);
    names.sort_by_key(|n| (rank(n), n.clone()));
    let wall = wallpapers::load(&wallpapers::named("Scarlet Tree").unwrap().1, 1600).expect("wallpaper");
    let mut picked: Vec<(String, String, memrium_de::icondoc::IconDoc)> = Vec::new();
    let mut seen_svgs = std::collections::HashSet::new();
    for n in names {
        if done.contains(&n) || n.contains("symbolic") {
            continue;
        }
        let Ok(svg) = std::fs::read_to_string(dir.join(format!("{n}.svg"))) else { continue };
        // the same drawing under another name is shown once
        if !seen_svgs.insert(svg.clone()) {
            continue;
        }
        if let Some((doc, kind)) = iconstyle::favourite_style_kind(&n, &svg) {
            if kind == want_kind {
                picked.push((n, svg, doc));
            }
        }
        if picked.len() >= count {
            break;
        }
    }
    let (n, gap, cols) = (128u32, 14u32, 4u32);
    let rows = (picked.len() as u32 + cols - 1) / cols;
    let cell = n * 2 + gap;
    let (w, h) = ((cell + gap) * cols + gap, (n + gap) * rows + gap);
    let mut out = image::DynamicImage::ImageRgba8(wall).resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    let mut r = Renderer::new();
    r.tune = GlassTune::load();
    for (i, (name, svg, doc)) in picked.iter().enumerate() {
        let (cx, cy) = (gap + (i as u32 % cols) * (cell + gap), gap + (i as u32 / cols) * (n + gap));
        if let Some(img) = svgedit::rasterize(svg, n) {
            image::imageops::overlay(&mut out, &img, cx as i64, cy as i64);
        }
        let (x, y) = (cx + n, cy);
        r.backdrop = Some(Arc::new(image::imageops::crop_imm(&out, x, y, n, n).to_image()));
        image::imageops::overlay(&mut out, &r.render(doc, Appearance::Default, n), x as i64, y as i64);
        eprintln!("{:2} {name}", i + 1);
    }
    out.save(&args[1]).unwrap();
}
