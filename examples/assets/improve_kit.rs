// Shared by examples/improve_icons.rs and improve_icons2.rs (included, not a module).
fn svg(body: &str) -> String {
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 512 512\">{body}</svg>")
}

fn rgb(h: &str) -> [f32; 3] {
    let v = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap();
    [(v >> 16 & 255) as f32 / 255.0, (v >> 8 & 255) as f32 / 255.0, (v & 255) as f32 / 255.0]
}

/// A new document on a squircle plate that fades from `top` to `bottom`.
fn new_doc(name: &str, top: &str, bottom: &str) -> IconDoc {
    let mut doc = IconDoc::new(name);
    doc.margin = 0.0;
    doc.background = Variants::new(Background {
        shape: Shape::Squircle,
        fill: FillKind::Gradient,
        top: rgb(top),
        bottom: rgb(bottom),
        glass: Glass { enabled: true, specular: 0.5, shadow_amount: 0.35, ..Glass::default() },
    });
    doc
}

/// The white plate as Apple draws it (Health): pure white at the top, light neutral grey at the bottom, a bright rim.
fn white_doc(name: &str) -> IconDoc {
    let mut d = new_doc(name, "#FDFDFD", "#E4E6E4");
    d.background.default.glass = Glass { enabled: true, specular: 1.0, shadow_amount: 0.4, ..Glass::default() };
    d
}

fn glass(specular: f32, translucency: f32, shadow: f32) -> Glass {
    Glass { enabled: true, specular, bevel: 0.0, bevel_width: 0.45, blur: 0.2, refraction: 0.5, translucency, shadow_amount: shadow, ..Glass::default() }
}

/// no glass effect at all (thin detail that must stay crisp)
fn flat() -> Glass {
    Glass { enabled: true, specular: 0.0, blur: 0.0, refraction: 0.0, translucency: 0.0, shadow_amount: 0.0, ..Glass::default() }
}

fn add(doc: &mut IconDoc, name: &str, body: &str, g: Glass, opacity: f32) {
    let id = doc.next_id;
    doc.next_id += 1;
    let props = Props { glass: g, opacity, ..Props::default() };
    doc.nodes.push(Node { id, name: name.into(), kind: Kind::Layer(Layer { svg: svg(body), props: Variants::new(props) }) });
}

/// Moves a drawing made in its own units so that its middle (`cx`, `cy`) is the plate's middle and one unit is `s` pixels of the 512 box.
fn place(cx: f32, cy: f32, s: f32) -> String {
    format!("translate(256 256) scale({s}) translate({} {})", -cx, -cy)
}

/// a circle as a path (so it can be a hole in another path)
fn disc(cx: f32, cy: f32, r: f32) -> String {
    format!("M{} {cy} a{r} {r} 0 1 0 {} 0 a{r} {r} 0 1 0 {} 0 Z", cx - r, 2.0 * r, -2.0 * r)
}

/// The dark look: the icon's own colours on a near-black plate (tinted a little when `plate` says so).
fn darken(doc: &mut IconDoc, plate: Option<([f32; 3], [f32; 3])>, set: &[(&str, &str, f32)]) {
    let (top, bottom) = plate.unwrap_or(([0.12, 0.12, 0.14], [0.02, 0.02, 0.03]));
    doc.background.set(Appearance::Dark, Background { top, bottom, ..doc.background.default }, &|_, d| *d);
    for n in &mut doc.nodes {
        if let Some((_, colour, op)) = set.iter().find(|s| s.0 == n.name) {
            let new = Props { fill: Some(rgb(colour)), opacity: *op, ..n.props().default };
            n.props_mut().set(Appearance::Dark, new, &|_, d| *d);
        }
    }
}

/// The tinted looks only recolour the picture (no glass on top that would wash a multicolour drawing out).
fn plain_tinted(doc: &mut IconDoc) {
    for n in &mut doc.nodes {
        let base = n.props().default;
        for look in [Appearance::TintedLight, Appearance::TintedDark] {
            n.props_mut().pin(look, Props { glass: flat(), ..base });
        }
    }
}
