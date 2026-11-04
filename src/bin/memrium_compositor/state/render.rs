//! Frame building and drawing: the scene is put together, hashed to see whether anything changed, diffed for damage and
//! handed to the renderer.

use super::*;

impl State {
    /// Draw one frame into `target`.
    pub fn render(&mut self, target: &wgpu::TextureView) -> wgpu::CommandEncoder {
        match self.render_lazy(|| Some(target.clone())) {
            Some(enc) => enc,
            // identical to the last frame: nothing to draw
            None => self.renderer.device().create_command_encoder(&Default::default()),
        }
    }

    /// Like `render`, but the target is only asked for when the frame really changed (acquiring a swapchain image
    /// costs a vblank wait). `None` = nothing to draw (the frame is identical to the last one, or no target).
    pub fn render_lazy(&mut self, target: impl FnOnce() -> Option<wgpu::TextureView>) -> Option<wgpu::CommandEncoder> {
        if self.screen_off {
            let w = self.world;
            let mut shell = vec![Panel::solid((w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0, w[2] - w[0], w[3] - w[1], 0.0, [0.0; 3], [0.0; 3]).flat()];
            let mut items: Vec<Item<'_>> = Vec::new();
            let hash = self.frame_hash(&shell, &items);
            self.frame_skipped = self.allow_skip && hash == self.last_hash;
            self.last_hash = hash;
            if self.frame_skipped {
                return None;
            }
            let target = target()?;
            self.damage.reset();
            let off = self.canvas_off();
            memrium_de::renderer::shift_scene(&mut items, &mut shell, off[0], off[1]);
            return Some(self.renderer.render(&target, &items, &shell));
        }
        if self.lock.is_some() {
            // the lock screen is the whole picture: no windows, bars or notifications behind it
            let (lock_shell, labels) = self.lock_panels();
            // every monitor is covered, not only the main one
            let w = self.world;
            let mut shell = vec![Panel::solid((w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0, w[2] - w[0], w[3] - w[1], 0.0, [0.0; 3], [0.0; 3]).flat()];
            shell.extend(lock_shell);
            shell.extend(self.renderer.label_panels(&labels));
            let mut items: Vec<Item<'_>> = Vec::new();
            let hash = self.frame_hash(&shell, &items);
            self.frame_skipped = self.allow_skip && !self.renderer.is_live() && hash == self.last_hash;
            self.last_hash = hash;
            if self.frame_skipped {
                return None;
            }
            self.frames_drawn += 1;
            let Some(target) = target() else {
                self.last_hash = 0;
                return None;
            };
            self.damage.reset();
            let off = self.canvas_off();
            memrium_de::renderer::shift_scene(&mut items, &mut shell, off[0], off[1]);
            return Some(self.renderer.render(&target, &items, &shell));
        }
        if self.shot_sel.is_some() && self.shot_freeze.is_some() {
            // the picker shows the frozen picture and its dimming, nothing live
            let mut shell = Vec::new();
            self.shot_overlay(&mut shell);
            let [fx0, fy0, fx1, fy1] = self.world;
            let rect = [fx0, fy0, fx1 - fx0, fy1 - fy0];
            let tex = &self.shot_freeze.as_ref().unwrap().tex;
            let mut items = vec![Item::Surface { tex, rect, clip: None, frosted: false, alpha: 1.0, veil: 1.0 }];
            let hash = self.frame_hash(&shell, &items);
            self.frame_skipped = self.allow_skip && hash == self.last_hash;
            self.last_hash = hash;
            if self.frame_skipped {
                return None;
            }
            self.frames_drawn += 1;
            let Some(target) = target() else {
                self.last_hash = 0;
                self.damage.reset();
                return None;
            };
            self.damage.reset();
            let off = self.canvas_off();
            memrium_de::renderer::shift_scene(&mut items, &mut shell, off[0], off[1]);
            return Some(self.renderer.render(&target, &items, &shell));
        }
        self.renderer.set_glass_adapt(self.theme.glass.adapt);
        // shell: Dock, Quick Settings, labels
        let (bar_p, bar_l) = self.bar_draw();
        let mut shell = bar_p;
        shell.extend(self.scene.shell_panels());
        let mut labels = self.scene.labels();
        labels.extend(bar_l);
        // the Glass page's samples refract the backdrop the page paints, so they live above the window
        if self.focused_shell() == Some(Sw::Settings) && self.settings.open && !self.swin.minimized && self.swin.k.x > 0.985 {
            let (pp, pl) = self.settings.preview_shell.clone();
            shell.extend(pp);
            labels.extend(pl);
        }
        if self.launcher.visible() {
            let pinned = self.pinned_indices();
            let mut budget = 8u32;
            for i in self.launcher.wanted_icons(&pinned, &self.apps) {
                self.ensure_icon(i, &mut budget);
            }
            let renderer = &self.renderer;
            // the start menu sits on the bar that holds the start button
            let (bar, anchor) = self.with_bar_ctx(|c| match self.bars.iter().find(|b| b.has("start")) {
                Some(b) => ((Some((b.cfg.edge, b.rect(c)))), b.widget_rects(c).iter().find(|(_, _, k)| *k == "start").map(|(_, r, _)| *r)),
                None => (None, None),
            });
            let ctx = DrawCtx { screen: self.size, apps: &self.apps, pinned: &pinned, theme: &self.theme, bar, anchor, icon_cell: &|a: &AppEntry| renderer.app_icon_get(&a.id) };
            let (lp, ll) = self.launcher.draw(&ctx);
            shell.extend(lp);
            labels.extend(ll);
        }
        let mut toast_labels = Vec::new();
        self.toast_panels(&mut shell, &mut toast_labels);
        self.widgets_panels(&mut shell, &mut toast_labels);
        self.flyout_panels(&mut shell, &mut toast_labels);
        self.card_panels(&mut shell, &mut toast_labels);
        labels.extend(toast_labels);
        shell.extend(self.renderer.label_panels(&labels));
        if let Some(d) = self.file_drag.as_ref().filter(|d| d.active) {
            let text = match d.paths.as_slice() {
                [one] => one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                many => format!("{} items", many.len()),
            };
            let text = if self.renderer.text_width(&text, 13.0, true) > 200.0 { format!("{}…", text.chars().take(24).collect::<String>()) } else { text };
            let w = self.renderer.text_width(&text, 13.0, true) + 46.0;
            let (cx, cy) = (self.mouse[0] + 18.0 + w / 2.0, self.mouse[1] + 22.0);
            let mut g = self.theme.glass.panel(cx, cy, w, 30.0, 15.0).tinted([0.08, 0.10, 0.16, 0.70]);
            g.extra[0] = 0.25;
            shell.push(g);
            shell.push(Panel::glyph(cx - w / 2.0 + 18.0, cy, 15.0, if d.paths.len() > 1 { "copy" } else { "file" }, [1.0; 3], 0.95));
            let gl = vec![Label::new(text, cx - w / 2.0 + 34.0, cy + 4.8, 13.0, true, [1.0; 3], 1.0, memrium_de::renderer::Align::Left)];
            shell.extend(self.renderer.label_panels(&gl));
        }
        if self.panel_edit.is_some() && !self.capturing {
            let (ep, el) = self.panel_edit_overlay();
            shell.extend(ep);
            shell.extend(self.renderer.label_panels(&el));
        }
        if self.menu.open {
            let r = &self.renderer;
            let (mp, ml) = self.menu.draw(&self.theme, self.size, &|t, s, b| r.text_width(t, s, b));
            shell.extend(mp);
            shell.extend(self.renderer.label_panels(&ml));
        }

        if !self.capturing {
            self.shot_overlay(&mut shell);
            self.shot_edit_overlay(&mut shell);
            if self.fps_on {
                let w = self.renderer.text_width(&self.fps_text, 13.0, true) + 24.0;
                let (cx, cy) = (self.size[0] - w / 2.0 - 12.0, 44.0);
                let mut g = self.theme.glass.panel(cx, cy, w, 26.0, 13.0).tinted([0.04, 0.05, 0.08, 0.70]);
                g.extra[0] = 0.25;
                shell.push(g);
                let gl = vec![Label::new(self.fps_text.clone(), cx - w / 2.0 + 12.0, cy + 4.6, 13.0, true, [1.0; 3], 1.0, memrium_de::renderer::Align::Left)];
                shell.extend(self.renderer.label_panels(&gl));
            }
        }

        // window layer (titles are text panels inside each window's frame)
        let default_dock = [self.size[0] / 2.0, self.size[1] - 40.0];
        let docks: Vec<[f32; 2]> = self.wins.iter().map(|w| self.item_center(&self.key_of(w)).unwrap_or(default_dock)).collect();
        let focus = self.wins.iter().rposition(|w| !w.minimized);
        let mut title_panels: HashMap<usize, Vec<Panel>> = HashMap::new();
        if !self.menu.open {
            self.title_menu = None;
        }
        let title_hover = match self.chrome_at(self.mouse) {
            Some((j, Hit::Titlebar)) if !self.menu.open => Some(j),
            _ => None,
        };
        for (i, w) in self.wins.iter().enumerate() {
            if !w.mapped || w.min_k.x > 0.985 || w.deco != Deco::Server {
                continue;
            }
            let f = Self::frame_rect(w);
            // the program's menus take the left of the title bar, and the title moves over to make room
            let menus = self.title_menus(i);
            let mut tf = f;
            if let Some((_, last)) = menus.last() {
                let shift = last[0] + last[2] + 14.0 - decor::title_span(&self.theme, f).0;
                if shift > 0.0 {
                    tf = [f[0] + shift, f[1], f[2] - shift, f[3]];
                }
            }
            let (x0, x1) = decor::title_span(&self.theme, tf);
            let title = ellipsize(&self.renderer, &w.title, 13.0, (x1 - x0).max(40.0));
            let th = w.chrome_theme(&self.theme);
            let mut labels = decor::title_labels(&th, tf, &title, Some(i) == focus);
            let mut panels = Vec::new();
            if !menus.is_empty() {
                let rects: Vec<[f32; 4]> = menus.iter().map(|(_, r)| *r).collect();
                let open = self.menu.open.then(|| self.title_menu.as_ref().filter(|(id, _)| *id == w.id).and_then(|(_, n)| menus.iter().position(|(m, _)| m == n))).flatten();
                let under = if open.is_none() && title_hover == Some(i) { menus.iter().position(|(_, r)| Self::in_rect(self.mouse, *r)) } else { None };
                panels = decor::title_menu_panels(&th, &rects, open.or(under), open.is_some());
                labels.extend(decor::title_menu_labels(&th, f, &menus, Some(i) == focus));
            }
            panels.extend(self.renderer.label_panels(&labels));
            title_panels.insert(i, panels);
        }

        let mut shells = Vec::new();
        for k in self.z.clone() {
            let stamp = self.shell_win(k).stamp;
            shells.push((stamp, self.shell_item(k)));
        }
        let desktop = if self.capture_alone { None } else { self.desktop_item() };
        let preview = self.alt_tab_layout().or_else(|| self.preview_layout());
        self.tooltip_panels(&mut shell);
        if let Some(l) = &preview {
            let labels = self.preview_shell(l);
            let lp = self.renderer.label_panels(&labels);
            shell.extend(lp);
        }
        let ime = self.ime_popup_origins();
        let mut items = build_items(&self.wins, &self.popups, &self.surfaces, ime, focus, &docks, &title_panels, &self.theme, self.hover_btn, shells, desktop, &self.ghosts, self.renderer.scale().max(1.0), &self.rules, (self.ws_pos.x, self.size[0]));
        if let Some(l) = &preview {
            items.extend(crate::dock::preview_items(l, &self.wins, &self.surfaces, self.theme.dark, self.theme.accent));
        }
        if self.capture_alone {
            shell.clear();
        }
        // a picture a program drags out of itself, at the pointer over the windows
        if let Some(n) = self.dnd_icon.as_ref().and_then(|s| self.surfaces.get(&s.id())) {
            if let (Some(tex), Some(sz)) = (n.tex.as_deref(), n.logical_size()) {
                let r = [self.mouse[0] + self.dnd_icon_off[0], self.mouse[1] + self.dnd_icon_off[1], sz[0], sz[1]];
                items.push(Item::Surface { tex, rect: r, clip: None, frosted: false, alpha: 1.0, veil: 1.0 });
            }
        }
        // the picked area's picture, over everything the screen shows
        if let Some(e) = &self.shot_edit {
            items.push(Item::Surface { tex: e.tex(), rect: e.rect(), clip: None, frosted: false, alpha: 1.0, veil: 1.0 });
        }
        let hash = self.frame_hash(&shell, &items);
        self.frame_skipped = self.allow_skip && !self.renderer.is_live() && hash == self.last_hash;
        self.last_hash = hash;
        if self.frame_skipped {
            return None;
        }
        self.frames_drawn += 1;
        if std::env::var_os("MEMRIUM_WHY").is_some() {
            self.why_redraw(&shell, &items);
        }
        let Some(target) = target() else {
            // nothing was drawn, so the next frame must not be skipped as a duplicate
            self.last_hash = 0;
            self.damage.reset();
            return None;
        };
        match self.theme.glass.mode {
            memrium_de::theme::GlassMode::Wood => memrium_de::renderer::retint_boards(&mut items, &mut shell, 13, self.theme.glass.tint),
            memrium_de::theme::GlassMode::Clay => memrium_de::renderer::pastelize_clay(&mut items, &mut shell, self.theme.dark),
            _ => {}
        }
        let off = self.canvas_off();
        // what changed since the last frame (the glass modes that recolour the panels afterwards redraw everything)
        let full = !(self.persistent_target && self.allow_skip) || self.renderer.is_live() || self.capturing || self.renderer.reveal_key() != 0 || std::env::var_os("MEMRIUM_NO_DAMAGE").is_some() || matches!(self.theme.glass.mode, memrium_de::theme::GlassMode::Wood | memrium_de::theme::GlassMode::Clay);
        let damage = self.damage.diff(self.renderer.epoch(), &items, &shell, full).map(|d| [d[0] + off[0], d[1] + off[1], d[2] + off[0], d[3] + off[1]]);
        if !self.allow_skip {
            // drawn into some other texture (a screenshot): the real target does not hold this frame
            self.damage.reset();
        }
        memrium_de::renderer::shift_scene(&mut items, &mut shell, off[0], off[1]);
        Some(self.renderer.render_damaged(&target, &items, &shell, damage))
    }

    /// Forget the last frame so the next `render` draws for sure (VT switch, resize).
    pub fn invalidate(&mut self) {
        self.last_hash = 0;
        self.damage.reset();
    }

    /// Debug (`MEMRIUM_WHY=1`): says what made this frame differ from the last one, summed up every 200 frames.
    fn why_redraw(&self, shell: &[Panel], items: &[memrium_de::renderer::Item]) {
        use memrium_de::renderer::Item;
        use std::cell::RefCell;
        use std::collections::HashMap;
        use std::hash::{BuildHasher, Hasher};
        #[derive(Default)]
        struct Why {
            n: u32,
            epoch: u64,
            shell: u64,
            surf: HashMap<usize, u64>,
            count: HashMap<String, u32>,
        }
        thread_local!(static WHY: RefCell<Why> = RefCell::new(Why::default()));
        WHY.with(|w| {
            let mut w = w.borrow_mut();
            let mut hs = foldhash::fast::FixedState::default().build_hasher();
            hs.write(bytemuck::cast_slice(shell));
            let (ep, sh) = (self.renderer.epoch(), hs.finish());
            if ep != w.epoch {
                *w.count.entry("renderer epoch".into()).or_default() += 1;
            }
            if sh != w.shell {
                *w.count.entry("shell panels".into()).or_default() += 1;
            }
            let mut changed = Vec::new();
            for it in items {
                match it {
                    Item::Surface { tex, .. } => {
                        let k = *tex as *const _ as usize;
                        let g = tex.generation();
                        if w.surf.insert(k, g) != Some(g) {
                            changed.push("surface".to_string());
                        }
                    }
                    Item::Frame { .. } => changed.push("window frame".to_string()),
                    Item::Layer { .. } => changed.push("layer".to_string()),
                    _ => {}
                }
            }
            // a frame item is only counted when the whole item list's hash moved, so the surface lines are the useful ones
            for c in changed.into_iter().filter(|c| c == "surface") {
                *w.count.entry(c).or_default() += 1;
            }
            w.epoch = ep;
            w.shell = sh;
            w.n += 1;
            if w.n >= 200 {
                eprintln!("why: {} redraws: {:?}", w.n, w.count);
                w.n = 0;
                w.count.clear();
            }
        });
    }

    fn frame_hash(&self, shell: &[Panel], items: &[memrium_de::renderer::Item]) -> u64 {
        use memrium_de::renderer::Item;
        use std::hash::{BuildHasher, Hasher};
        let mut h = foldhash::fast::FixedState::default().build_hasher();
        h.write_u64(self.renderer.epoch());
        h.write_u32(self.renderer.reveal_key());
        h.write(bytemuck::cast_slice(shell));
        for it in items {
            match it {
                Item::Warp(w) => {
                    h.write_u8(2);
                    if let Some(w) = w {
                        h.write(bytemuck::cast_slice(&w.rect));
                        h.write_u32(w.t.to_bits());
                        h.write(bytemuck::cast_slice(&w.dock));
                        if let Some(j) = &w.jelly {
                            h.write(bytemuck::cast_slice(j));
                        }
                    }
                }
                Item::Frame { panels, bbox, fade } => {
                    h.write_u8(0);
                    h.write_u32(fade.to_bits());
                    h.write(bytemuck::cast_slice(panels));
                    h.write(bytemuck::cast_slice(bbox));
                }
                Item::Layer { panels, bbox } => {
                    h.write_u8(3);
                    h.write(bytemuck::cast_slice(panels));
                    h.write(bytemuck::cast_slice(bbox));
                }
                Item::Opaque(_) => {}
                Item::Surface { tex, rect, clip, frosted, alpha, veil } => {
                    h.write_u8(1);
                    h.write_u32(veil.to_bits());
                    h.write_u32(alpha.to_bits());
                    h.write_usize(*tex as *const _ as usize);
                    h.write_u64(tex.generation());
                    h.write(bytemuck::cast_slice(rect));
                    h.write_u8(*frosted as u8);
                    if let Some((r, rad)) = clip {
                        h.write(bytemuck::cast_slice(r));
                        h.write_u32(rad.to_bits());
                    }
                }
            }
        }
        h.finish().max(1)
    }
}
