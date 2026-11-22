// Shared by examples/kickoff_icons.rs and examples/settings_icons.rs (included, not a module; needs improve_kit.rs).
const W: &str = "#FFFFFF";

fn path(d: &str, fill: &str) -> String {
    format!("<path fill-rule=\"evenodd\" d=\"{d}\" fill=\"{fill}\"/>")
}

/// One path with round corners (a stroke of its own colour).
fn soft(d: &str, fill: &str, w: u32) -> String {
    format!("<path fill-rule=\"evenodd\" d=\"{d}\" fill=\"{fill}\" stroke=\"{fill}\" stroke-width=\"{w}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>")
}

fn line(d: &str, colour: &str, w: u32) -> String {
    format!("<path d=\"{d}\" fill=\"none\" stroke=\"{colour}\" stroke-width=\"{w}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>")
}

fn rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> String {
    format!("M{} {y} h{} a{r} {r} 0 0 1 {r} {r} v{} a{r} {r} 0 0 1 {} {r} h{} a{r} {r} 0 0 1 {} {} v{} a{r} {r} 0 0 1 {r} {} Z", x + r, w - 2.0 * r, h - 2.0 * r, -r, -(w - 2.0 * r), -r, -r, -(h - 2.0 * r), -r)
}

fn gear(cx: f32, cy: f32, root: f32, tip: f32, hole: f32, teeth: usize) -> String {
    let step = std::f32::consts::TAU / teeth as f32;
    let mut d = String::new();
    for i in 0..teeth {
        for (k, (at, r)) in [(-0.34, root), (-0.18, tip), (0.18, tip), (0.34, root)].into_iter().enumerate() {
            let a = (i as f32 + at) * step - std::f32::consts::FRAC_PI_2;
            d += &format!("{}{:.1} {:.1} ", if i == 0 && k == 0 { "M" } else { "L" }, cx + r * a.cos(), cy + r * a.sin());
        }
    }
    d + "Z " + &disc(cx, cy, hole)
}

fn copy_of(of: &str, name: &str) -> IconDoc {
    let mut d = icondoc::load(&icondoc::doc_path(of)).unwrap_or_else(|e| panic!("{of}: {e}"));
    d.name = name.into();
    d
}

/// The tinted-by-icon look takes the plate's colour (the symbols are white, so the colour found from them would be a grey).
fn own_tint(d: &mut IconDoc) {
    let b = d.background.default;
    let white = b.top.iter().all(|v| *v > 0.9);
    if d.icon_tint.is_none() && b.fill == FillKind::Gradient && !white {
        let c = [0, 1, 2].map(|k| (b.top[k] + b.bottom[k]) / 2.0);
        let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        // (a grey plate stays a middle grey: brightened it would leave the white symbol on white)
        let k = if hi - lo < 0.2 { 0.55 } else { 0.86 } / hi.max(0.3);
        d.icon_tint = Some(c.map(|v| (v * k).min(1.0)));
    }
}

/// `<prefix>` writes the previews (every look, and the size of the menus); `<prefix> save` saves the documents and writes
/// only these icons into the themes (no full rebuild, so nothing else is redrawn or pruned).
fn run(docs: Vec<IconDoc>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let prefix = args.first().expect("<preview prefix> [save]").clone();
    let tune = GlassTune::load();
    if args.get(1).is_some_and(|a| a == "save") {
        let mut failed = false;
        for d in &docs {
            match icondoc::save_generated(d) {
                Ok(true) => match iconrender::build_one(d, tune, "Papirus", false) {
                    Ok(()) => println!("written {}", d.name),
                    Err(e) => { eprintln!("FAILED {}: {e}", d.name); failed = true; }
                },
                Ok(false) => println!("left alone (edited by hand) {}", d.name),
                Err(e) => { eprintln!("FAILED {}: {e}", d.name); failed = true; }
            }
        }
        iconrender::touch_marker();
        if failed { std::process::exit(1); }
        return;
    }
    let mut r = Renderer::new();
    r.tune = tune;
    let (s, g) = (100u32, 6u32);
    for (page, part) in docs.chunks(13).enumerate() {
        let mut out = image::RgbaImage::from_pixel(7 * (s + g) + g, part.len() as u32 * (s + g) + g, image::Rgba([84, 92, 106, 255]));
        for (y, d) in part.iter().enumerate() {
            for x in 0..7usize {
                let (look, suffix) = iconrender::THEME_LOOKS[x];
                r.tint_override = None;
                if suffix == iconrender::ICON_TINT_SUFFIX { r.tint_override = Some(r.icon_colour(d)); }
                let img = r.render(d, look, s);
                image::imageops::overlay(&mut out, &img, (g + x as u32 * (s + g)) as i64, (g + y as u32 * (s + g)) as i64);
            }
        }
        r.tint_override = None;
        out.save(format!("{prefix}-looks-{page}.png")).unwrap();
    }
    // as the menus show them: the 48 px picture brought down to 22 px, on a light and on a dark panel
    let (cols, cell) = (12u32, 32u32);
    let rows = (docs.len() as u32).div_ceil(cols);
    let mut small = image::RgbaImage::from_fn(cols * cell, rows * cell * 2, |_, y| if y < rows * cell { image::Rgba([250, 250, 252, 255]) } else { image::Rgba([32, 35, 38, 255]) });
    for (k, d) in docs.iter().enumerate() {
        let img = image::imageops::resize(&r.render(d, Appearance::Default, 48), 22, 22, image::imageops::FilterType::Triangle);
        for half in 0..2 {
            image::imageops::overlay(&mut small, &img, ((k as u32 % cols) * cell + 5) as i64, ((k as u32 / cols + half * rows) * cell + 5) as i64);
        }
    }
    let (w, h) = small.dimensions();
    image::imageops::resize(&small, w * 3, h * 3, image::imageops::FilterType::CatmullRom).save(format!("{prefix}-small.png")).unwrap();
    println!("{} icons", docs.len());
}
