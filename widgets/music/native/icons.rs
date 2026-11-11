//! Vendored Lucide vectors; native images are owned by the widget, never TLS.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    Open,
    Queue,
    Play,
    Pause,
    Previous,
    Next,
    Library,
    Home,
}
#[derive(Default)]
pub(super) struct Icons(RefCell<HashMap<(Kind, u32, u32), Image>>);
impl Icons {
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }
    pub unsafe fn draw(
        &self,
        c: &Canvas,
        dpi: f32,
        kind: Kind,
        center: [f32; 2],
        size: f32,
        color: u32,
        alpha: u8,
    ) {
        let px = (size * dpi * 2.).ceil().max(1.) as u32;
        let key = (kind, color, px);
        let mut cache = self.0.borrow_mut();
        if !cache.contains_key(&key) {
            if let Some(pixels) = raster(kind, color, px) {
                if let Some(image) = Image::from_rgba(px, px, pixels) {
                    if cache.len() >= 24 {
                        cache.clear();
                    }
                    cache.insert(key, image);
                }
            }
        }
        if let Some(image) = cache.get(&key) {
            c.image_alpha(
                image,
                [
                    center[0] - size / 2.,
                    center[1] - size / 2.,
                    center[0] + size / 2.,
                    center[1] + size / 2.,
                ],
                alpha as f32 / 255.,
            );
        }
    }
}
fn raster(kind: Kind, color: u32, px: u32) -> Option<Vec<u8>> {
    let svg = match kind {
        Kind::Open => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/music/external-link.svg"
        )),
        Kind::Queue => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/music/list-music.svg"
        )),
        Kind::Play => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/music/play.svg"
        )),
        Kind::Pause => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/music/pause.svg"
        )),
        Kind::Previous => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/music/skip-back.svg"
        )),
        Kind::Next => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/music/skip-forward.svg"
        )),
        Kind::Library => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/music/library.svg"
        )),
        Kind::Home => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/music/house.svg"
        )),
    };
    let svg = svg.replace(
        "currentColor",
        &format!(
            "#{:02x}{:02x}{:02x}",
            color & 255,
            (color >> 8) & 255,
            (color >> 16) & 255
        ),
    );
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixels = resvg::tiny_skia::Pixmap::new(px, px)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(px as f32 / 24., px as f32 / 24.),
        &mut pixels.as_mut(),
    );
    Some(pixels.data().to_vec())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lucide_media_icons_have_antialiased_strokes_and_clear_corners() {
        for kind in [
            Kind::Open,
            Kind::Queue,
            Kind::Play,
            Kind::Pause,
            Kind::Previous,
            Kind::Next,
            Kind::Library,
            Kind::Home,
        ] {
            let p = raster(kind, 0xffffff, 64).unwrap();
            assert_eq!(p[3], 0);
            assert!(p.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
            assert!(p.chunks_exact(4).any(|p| p[3] > 200));
        }
    }
}
