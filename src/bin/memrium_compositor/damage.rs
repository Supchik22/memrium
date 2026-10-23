//! Which part of the screen a frame changes. The renderer redraws only that part (the target keeps the rest of the
//! previous frame), which matters on a GPU where every full-screen pass costs milliseconds: a Dock icon growing or a
//! toggle sliding touches a few thousand pixels, not two million.

use memrium_de::renderer::{panel_box, Item, Panel};
use std::collections::HashMap;
use std::hash::Hasher;

/// One window-layer item of a frame: a frame (window chrome, a shell window) with the panels it holds, or a surface.
enum Sig {
    Frame { meta: u64, bbox: [f32; 4], panels: Vec<(u64, [f32; 4])> },
    Surface { hash: u64, bbox: [f32; 4] },
}

impl Sig {
    fn bbox(&self) -> [f32; 4] {
        match self {
            Sig::Frame { bbox, .. } | Sig::Surface { bbox, .. } => *bbox,
        }
    }
}

/// What the last frame was made of: each window-layer item (its hash and box) and each shell panel.
#[derive(Default)]
pub struct DamageTracker {
    items: Vec<Sig>,
    panels: HashMap<u64, Vec<[f32; 4]>>,
    /// a number for everything else that changes how the whole picture looks (the wallpaper, the glass adaptation...)
    global: u64,
    have: bool,
}

fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(b);
    h.finish()
}

fn union(a: &mut Option<[f32; 4]>, b: [f32; 4]) {
    *a = Some(match a {
        Some(r) => [r[0].min(b[0]), r[1].min(b[1]), r[2].max(b[2]), r[3].max(b[3])],
        None => b,
    });
}

/// A panel's reach (shadow included) cut to its frame's box.
fn clip_box(b: [f32; 4], to: [f32; 4]) -> [f32; 4] {
    [b[0].max(to[0]), b[1].max(to[1]), b[2].min(to[2]), b[3].min(to[3])]
}

impl DamageTracker {
    /// Forget the last frame: the next one is drawn whole.
    pub fn reset(&mut self) {
        self.have = false;
    }

    /// Compare this frame with the last one. `None`: draw everything (or `full` was asked for, or nothing is known);
    /// `Some(rect)`: only `rect` (x0, y0, x1, y1) changed.
    pub fn diff(&mut self, global: u64, items: &[Item], shell: &[Panel], full: bool) -> Option<[f32; 4]> {
        // the window layer
        let mut cur_items: Vec<Sig> = Vec::with_capacity(items.len());
        let mut warp_hash = 0u64;
        let mut warp_box: Option<[f32; 4]> = None;
        for it in items {
            match it {
                Item::Warp(w) => {
                    warp_hash = 0;
                    warp_box = None;
                    if let Some(w) = w {
                        let mut v: Vec<f32> = Vec::new();
                        v.extend_from_slice(&w.rect);
                        v.push(w.t);
                        v.extend_from_slice(&w.dock);
                        v.push(w.icon_w);
                        if let Some(j) = w.jelly {
                            v.extend_from_slice(&j);
                        }
                        warp_hash = hash_bytes(bytemuck::cast_slice(&v)) | 1;
                        // a warped window reaches beyond its frame (towards the Dock, or swinging)
                        warp_box = Some([w.rect[0].min(w.dock[0] - w.icon_w) - 20.0, w.rect[1].min(w.dock[1] - w.icon_w) - 20.0, (w.rect[0] + w.rect[2]).max(w.dock[0] + w.icon_w) + 20.0, (w.rect[1] + w.rect[3]).max(w.dock[1] + w.icon_w) + 20.0]);
                    }
                }
                Item::Frame { panels, bbox, fade } => {
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    h.write_u32(fade.to_bits());
                    h.write_u64(warp_hash);
                    h.write(bytemuck::cast_slice(bbox));
                    let mut b = *bbox;
                    if let Some(w) = warp_box {
                        b = [b[0].min(w[0]), b[1].min(w[1]), b[2].max(w[2]), b[3].max(w[3])];
                    }
                    let sigs = panels.iter().map(|p| (hash_bytes(bytemuck::bytes_of(p)), panel_box(p))).collect();
                    cur_items.push(Sig::Frame { meta: h.finish(), bbox: b, panels: sigs });
                }
                Item::Layer { panels, bbox } => {
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    h.write_u8(3);
                    h.write_u64(warp_hash);
                    h.write(bytemuck::cast_slice(bbox));
                    let mut b = *bbox;
                    if let Some(w) = warp_box {
                        b = [b[0].min(w[0]), b[1].min(w[1]), b[2].max(w[2]), b[3].max(w[3])];
                    }
                    let sigs = panels.iter().map(|p| (hash_bytes(bytemuck::bytes_of(p)), panel_box(p))).collect();
                    cur_items.push(Sig::Frame { meta: h.finish(), bbox: b, panels: sigs });
                }
                // only says what need not be drawn: the picture is the same either way
                Item::Opaque(_) => {}
                Item::Surface { tex, rect, clip, frosted, alpha, veil } => {
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    h.write_usize(*tex as *const _ as usize);
                    h.write_u64(tex.generation());
                    h.write(bytemuck::cast_slice(rect));
                    if let Some((c, r)) = clip {
                        h.write(bytemuck::cast_slice(c));
                        h.write_u32(r.to_bits());
                    }
                    h.write_u8(*frosted as u8);
                    h.write_u32(alpha.to_bits());
                    h.write_u32(veil.to_bits());
                    h.write_u64(warp_hash);
                    let mut b = [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]];
                    if let Some(w) = warp_box {
                        b = [b[0].min(w[0]), b[1].min(w[1]), b[2].max(w[2]), b[3].max(w[3])];
                    }
                    cur_items.push(Sig::Surface { hash: h.finish(), bbox: b });
                }
            }
        }
        // the shell: panels by their own hash (a panel that only moved in the list is not a change)
        let mut cur_panels: HashMap<u64, Vec<[f32; 4]>> = HashMap::with_capacity(shell.len());
        for p in shell {
            cur_panels.entry(hash_bytes(bytemuck::bytes_of(p))).or_default().push(panel_box(p));
        }
        let ok = self.have && !full && global == self.global;
        let mut d: Option<[f32; 4]> = None;
        if ok {
            for i in 0..cur_items.len().max(self.items.len()) {
                match (cur_items.get(i), self.items.get(i)) {
                    (Some(Sig::Surface { hash: a, .. }), Some(Sig::Surface { hash: b, .. })) if a == b => {}
                    (Some(Sig::Frame { meta: ma, panels: pa, bbox }), Some(Sig::Frame { meta: mb, panels: pb, .. })) if ma == mb => {
                        // the same frame: only the panels that differ
                        let mut before: HashMap<u64, Vec<[f32; 4]>> = HashMap::new();
                        for (h, b) in pb {
                            before.entry(*h).or_default().push(*b);
                        }
                        let mut after: HashMap<u64, usize> = HashMap::new();
                        for (h, b) in pa {
                            let n = after.entry(*h).or_insert(0);
                            *n += 1;
                            if before.get(h).map_or(0, |v| v.len()) < *n {
                                union(&mut d, clip_box(*b, *bbox));
                            }
                        }
                        for (h, boxes) in &before {
                            let n = after.get(h).copied().unwrap_or(0);
                            for b in boxes.iter().skip(n) {
                                union(&mut d, clip_box(*b, *bbox));
                            }
                        }
                    }
                    (a, b) => {
                        if let Some(a) = a {
                            union(&mut d, a.bbox());
                        }
                        if let Some(b) = b {
                            union(&mut d, b.bbox());
                        }
                    }
                }
            }
            // panels that are new, and panels that are gone
            for (h, boxes) in &cur_panels {
                let had = self.panels.get(h).map_or(0, |v| v.len());
                for b in boxes.iter().skip(had) {
                    union(&mut d, *b);
                }
            }
            for (h, boxes) in &self.panels {
                let has = cur_panels.get(h).map_or(0, |v| v.len());
                for b in boxes.iter().skip(has) {
                    union(&mut d, *b);
                }
            }
        }
        self.items = cur_items;
        self.panels = cur_panels;
        self.global = global;
        self.have = true;
        if ok {
            // something changed (the caller saw a different hash) but nothing we can point at: do it all
            d
        } else {
            None
        }
    }
}
