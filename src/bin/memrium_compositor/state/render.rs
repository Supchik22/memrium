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
        if self.live_shot.is_some() {
            // Live Text: the frozen screen, its lines lit, nothing live
            let mut shell = Vec::new();
            self.live_text_panels(&mut shell);
            let [fx0, fy0, fx1, fy1] = self.world;
            let rect = [fx0, fy0, fx1 - fx0, fy1 - fy0];
            let tex = &self.live_shot.as_ref().unwrap().tex;
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
        self.renderer.set_corners(self.theme.ui.corners);
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
        self.gesture_panels(&mut shell);
        self.voice_panels(&mut shell);
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
        // (worked out before the items borrow the windows)
        let intro = if self.capturing { None } else { self.intro_rems() };
        let wall_view = if self.capturing { None } else { Some(self.wall_view_now()) };
        let ime = self.ime_popup_origins();
        let ov_cards = if self.ov.x > 0.001 { self.ov_layout() } else { Vec::new() };
        self.ov_labels(&ov_cards, &mut shell);
        self.ov_strip_panels(&mut shell);
        let mut items = build_items(&self.wins, &self.popups, &self.surfaces, ime, focus, &docks, &title_panels, &self.theme, self.hover_btn, shells, desktop, &self.ghosts, self.renderer.scale().max(1.0), &self.rules, (self.ws_pos.x, self.size[0]), (self.ov.x.max(0.0), &ov_cards, self.ov_hover));
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
        // the opening of the session (the desktop slides in) and the wallpaper's view (parallax, the opening's zoom)
        if let Some(r) = intro {
            intro_shift(&mut shell, &mut items, r, self.size);
        }
        if let Some((off, zoom)) = wall_view {
            if self.renderer.set_wall_view(off, zoom) {
                self.last_hash = 0;
            }
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

    /// How far the opening of the session still has to go for a part that starts `delay` s in and takes `dur` s: 1 = not
    /// yet moving, 0 = in place (a slight overshoot on the way, as a spring). None: no opening (played, or turned off).
    fn intro_left(&mut self, delay: f32, dur: f32) -> Option<f32> {
        if self.intro_done || !self.theme.animations.on("session_start") {
            return None;
        }
        // it waits (everything still out of place) until the wallpaper is read, so it does not play over the default one
        if self.intro_t0.is_none() && self.wall_rx.is_some() && self.start.elapsed().as_secs_f32() < 4.0 {
            return Some(1.0);
        }
        let t0 = *self.intro_t0.get_or_insert_with(Instant::now);
        let t = t0.elapsed().as_secs_f32() / self.theme.animations.speed.clamp(0.2, 4.0);
        // (the longest part: the wallpaper's zoom)
        if t > 1.6 {
            self.intro_done = true;
            self.intro_t0 = None;
            return None;
        }
        let u = ((t - delay) / dur).clamp(0.0, 1.0);
        // ease out with a little overshoot: 1 + c3 (u-1)^3 + c1 (u-1)^2
        let (c1, c3) = (1.15f32, 2.15f32);
        let e = 1.0 + c3 * (u - 1.0).powi(3) + c1 * (u - 1.0).powi(2);
        Some(1.0 - e)
    }

    /// How far the bars, the bottom and the middle still have to go in the opening (see `intro_shift`).
    fn intro_rems(&mut self) -> Option<[f32; 3]> {
        let (Some(top), Some(bottom), Some(mid)) = (self.intro_left(0.10, 0.70), self.intro_left(0.22, 0.75), self.intro_left(0.34, 0.80)) else { return None };
        Some([top, bottom, mid])
    }

    /// Where the wallpaper is moved to and how much it is enlarged (shares of the picture): it lies behind all the
    /// desktops and moves a little with each (more for a layered one, whose near layers move more than the far ones),
    /// follows the pointer when asked, and comes from a little closer at the opening.
    fn wall_view_now(&mut self) -> ([f32; 2], f32) {
        let layered = self.renderer.wall_layers() > 1;
        let mut off = [0.0f32; 2];
        let mut margin = 0.0f32;
        if self.theme.animations.on("wallpaper_desktops") {
            let span = if layered { 0.06 } else { 0.03 };
            let k = span / (WORKSPACES - 1).max(1) as f32;
            off[0] -= (self.ws_pos.x - (WORKSPACES - 1) as f32 / 2.0) * k;
            margin += span / 2.0;
        }
        if self.theme.animations.on("wallpaper_pointer") {
            let amp = if layered { 0.022 } else { 0.012 };
            let m = [(self.mouse[0] / self.size[0].max(1.0)).clamp(0.0, 1.0), (self.mouse[1] / self.size[1].max(1.0)).clamp(0.0, 1.0)];
            // eased after the pointer (each frame drawn goes a part of the way)
            for i in 0..2 {
                self.wall_ptr[i] += (m[i] - self.wall_ptr[i]) * 0.18;
                if (m[i] - self.wall_ptr[i]).abs() < 0.0005 {
                    self.wall_ptr[i] = m[i];
                }
                off[i] -= (self.wall_ptr[i] - 0.5) * 2.0 * amp;
            }
            margin += amp;
        }
        let zoom_left = self.intro_left(0.0, 1.5).unwrap_or(0.0).max(0.0);
        (off, 1.0 + 2.0 * margin + 0.10 * zoom_left)
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

/// The opening: the bars come down from above, what is at the bottom (the Dock) comes up, the sides come in from the
/// sides, and what is in the middle (desktop icons, widgets) gathers from further out, one after the other. `r` = how far
/// the top, the bottom and the middle still have to go (1 = all the way).
fn intro_shift(shell: &mut [Panel], items: &mut [Item<'_>], r: [f32; 3], size: [f32; 2]) {
    let [top, bottom, mid] = r;
    let [w, h] = size;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let shift = |p: &mut Panel| {
        let (x, y) = (p.rect[0], p.rect[1]);
        if y < h * 0.16 {
            p.rect[1] -= top * 90.0;
        } else if y > h * 0.84 {
            p.rect[1] += bottom * 140.0;
        } else if x < w * 0.12 {
            p.rect[0] -= mid * 120.0;
        } else if x > w * 0.88 {
            p.rect[0] += mid * 120.0;
        } else {
            // from a spread-out place towards its own
            p.rect[0] += (x - cx) * mid * 0.45;
            p.rect[1] += (y - cy) * mid * 0.45;
        }
    };
    for p in shell.iter_mut() {
        shift(p);
    }
    for it in items.iter_mut() {
        if let Item::Layer { panels, .. } = it {
            for p in panels.iter_mut() {
                shift(p);
            }
        }
    }
}
