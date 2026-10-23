//! Developer tool: parse the built-in themes, print a summary, and round-trip config overrides.

use memrium_de::theme::{self, Config};

fn main() {
    for (id, _) in theme::PRESETS {
        let t = theme::preset(id).expect("preset parses");
        println!("{id:<10} {:<22} layout={:?} deco={:?} launcher={:?} glass.blur={} mode={:?} amount={}", t.name, t.layout, t.window.decoration, t.launcher.style, t.glass.blur, t.glass.mode, t.glass.amount);
    }
    // overrides: switch to windows7 and tweak two fields, then see what would be stored
    let mut cfg = Config::default();
    cfg.select("windows7");
    let mut t = cfg.resolve();
    t.glass.blur = 20.0;
    t.bars[0].height = Some(52.0);
    cfg.set_from(&t);
    println!("--- config.toml would contain ---\n{}", toml::to_string_pretty(&cfg).unwrap());
    let back = cfg.resolve();
    println!("round trip ok: {}", back == t);
}
