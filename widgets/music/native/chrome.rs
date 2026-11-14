//! Dock material primitives and consistent media controls for all Music designs.
use super::*;
impl State {
    pub(super) unsafe fn chrome(&mut self, bg: &mut Buffer, size: [f32; 2]) {
        let [w, h] = size;
        let color = self.surface_color();
        let light = crate::accent::light(color);
        let tint = if self.config.frosted_glass {
            self.config.tint
        } else {
            100.
        };
        // The compositor owns the fused contour. Baking a private rounded
        // pane into Music leaves a visible inner corner at the join.
        let fused = crate::fusion::fused(self.hwnd);
        let pane = if fused {
            [0., 0., w, h]
        } else {
            [4., 4., w - 4., h - 4.]
        };
        let radius = if fused { 0. } else { 34. };
        // Exactly Dock's coating order/units. pane_overlay already includes
        // reflection_gradient: calling both makes the material doubly smoky.
        if self.overlay.is_none() {
            self.overlay = Canvas::pane_overlay(
                [bg.width as u32, bg.height as u32],
                self.dpi,
                tint / 100.,
                light,
            );
        }
        let c = Canvas::new(bg.dc, self.dpi);
        if self.config.design == "disc" {
            // Disc's warm printed backing is part of its graphic identity.
            c.round(pane, radius, 0xf0eeed, 255, false, false);
            c.coating(pane, radius);
            return;
        }
        if !crate::surface::managed(&self.config.palette) {
            c.round(
                pane,
                radius,
                color,
                (tint * 2.55).round() as u8,
                false,
                false,
            );
        }
        crate::surface::paint(&c, pane, &self.config.palette, tint / 100.);
        c.coating(pane, radius);
        if let Some(image) = &self.overlay {
            c.image(image, [0., 0., w, h]);
        }
        c.widget_corner_reflection_for(self.hwnd, pane, radius, light);
    }
    pub(super) unsafe fn rounded_cover(&self, c: &Canvas, url: &str, r: [f32; 4], radius: f32) {
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        let (pw, ph) = (
            (w * self.dpi).round().max(1.) as i32,
            (h * self.dpi).round().max(1.) as i32,
        );
        let identity = self.cover_image(url).map_or(0, Image::identity);
        let key = format!("{url}:{identity}:{pw}:{ph}:{radius}:{}", self.dpi);
        if !self.rounded_art.borrow().contains_key(&key) {
            if let Some(buffer) = Buffer::plain(pw, ph, self.dpi) {
                let batch = render::AlphaBatch::new(buffer.dc, self.dpi);
                let canvas = Canvas::new(buffer.dc, self.dpi);
                self.cover(&canvas, url, [0., 0., w, h], 1.);
                drop(canvas);
                drop(batch);
                let mut pixels = buffer.snapshot();
                for y in 0..ph {
                    for x in 0..pw {
                        let i = ((y * pw + x) * 4) as usize;
                        let alpha = (0.5
                            - crate::compositor::distance(
                                x as f32 + 0.5,
                                y as f32 + 0.5,
                                pw as f32,
                                ph as f32,
                                radius * self.dpi,
                                0.,
                            ))
                        .clamp(0., 1.);
                        for ch in 0..4 {
                            pixels[i + ch] = (pixels[i + ch] as f32 * alpha).round() as u8;
                        }
                        pixels.swap(i, i + 2);
                    }
                }
                if let Some(image) = Image::from_rgba(pw as u32, ph as u32, pixels) {
                    let mut cache = self.rounded_art.borrow_mut();
                    if cache.len() >= 24 {
                        cache.clear();
                    }
                    cache.insert(key.clone(), image);
                }
            }
        }
        if let Some(image) = self.rounded_art.borrow().get(&key) {
            c.image(image, r);
        }
    }
    pub(super) unsafe fn media_button(
        &mut self,
        c: &Canvas,
        r: [f32; 4],
        action: Action,
        enabled: bool,
    ) {
        if action == Action::Queue {
            return;
        }
        let (x, y) = ((r[0] + r[2]) / 2., (r[1] + r[3]) / 2.);
        let hover = self.hover.as_ref() == Some(&action);
        let down = hover && self.pressed.as_ref() == Some(&action);
        let play = action == Action::Toggle;
        let radius = (r[3] - r[1]).min(r[2] - r[0]) / 2.;
        let circle = [x - radius, y - radius, x + radius, y + radius];
        self.control_plates.push((circle, radius));
        {
            // soft_shadow consumes 0..255 opacity, not a normalized fraction.
            c.soft_shadow(
                circle,
                radius,
                if !enabled {
                    8.
                } else if down {
                    14.
                } else if hover {
                    28.
                } else {
                    19.
                },
                6.,
                if down { 1.5 } else { 3. },
            );
            let mut style = 0;
            if enabled {
                style |= lenses::ENABLED;
            }
            if hover {
                style |= lenses::HOVER;
            }
            if down {
                style |= lenses::PRESSED;
            }
            if play {
                style |= lenses::PRIMARY;
            }
            if self.light {
                style |= lenses::LIGHT;
            }
            if action == Action::Queue && self.config.queue {
                style |= lenses::ACTIVE;
            }
            self.button_lenses
                .draw(c, self.dpi, circle, self.surface_color(), style);
        }
        let ink = self.ink;
        let kind = match action {
            Action::Browse => icons::Kind::Open,
            Action::Library => icons::Kind::Library,
            Action::Queue => icons::Kind::Queue,
            Action::Toggle => {
                if self.playback.playing {
                    icons::Kind::Pause
                } else {
                    icons::Kind::Play
                }
            }
            Action::Previous => icons::Kind::Previous,
            Action::Next => icons::Kind::Next,
            _ => return,
        };
        self.media_icons.draw(
            c,
            self.dpi,
            kind,
            [x, y],
            (if play { 21.0_f32 } else { 19.0_f32 }).min(radius * 1.15),
            ink,
            if enabled { 242 } else { 70 },
        );
        if enabled {
            self.buttons.push(Button { rect: r, action });
        }
    }
}
