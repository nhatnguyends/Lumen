//! Vendored Lucide vectors; native images are owned by the widget, never TLS.
use super::{
    model::Kind,
    render::{Canvas, Image},
};
use std::{cell::RefCell, collections::HashMap};
#[derive(Default)]
pub(super) struct Icons(RefCell<HashMap<(Kind, u32, u32), Image>>, RefCell<HashMap<(u32,u32),Image>>);
impl Icons {
    pub unsafe fn pencil(&self,c:&Canvas,dpi:f32,r:[f32;4],color:u32){
        let px=(14.*dpi*2.).ceil() as u32;let mut cache=self.1.borrow_mut();
        if !cache.contains_key(&(color,px)) {
            let svg=format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#{:02x}{:02x}{:02x}" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><path d="m16 3 5 5M4 15 16 3a3.54 3.54 0 0 1 5 5L9 20l-6 1z"/></svg>"##,color&255,(color>>8)&255,(color>>16)&255);
            if let Ok(tree)=resvg::usvg::Tree::from_str(&svg,&resvg::usvg::Options::default()){if let Some(mut pix)=resvg::tiny_skia::Pixmap::new(px,px){resvg::render(&tree,resvg::tiny_skia::Transform::from_scale(px as f32/24.,px as f32/24.),&mut pix.as_mut());if let Some(im)=Image::from_rgba(px,px,pix.data().to_vec()){if cache.len()>12{cache.clear();}cache.insert((color,px),im);}}}
        }
        if let Some(im)=cache.get(&(color,px)){c.image(im,r);}
    }
    pub fn clear(&self) {
        self.0.borrow_mut().clear();self.1.borrow_mut().clear();
    }
    pub unsafe fn draw(
        &self,
        dpi: f32,
        c: &Canvas,
        kind: Kind,
        x: f32,
        y: f32,
        size: f32,
        color: u32,
        available: bool,
    ) {
        let center = [x + size / 2., y + size / 2.];
        let alpha = if available { 245 } else { 105 };
        let px = (size * dpi * 2.).ceil().max(1.) as u32;
        let key = (kind, color, px);
        let mut cache = self.0.borrow_mut();
        if !cache.contains_key(&key) {
            if let Some(pixels) = raster(kind, color, px) {
                if let Some(image) = Image::from_rgba(px, px, pixels) {
                    if cache.len() >= 96 {
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
        Kind::Display => include_str!("../assets/monitor.svg"),
        Kind::Hdr => include_str!("../assets/sun.svg"),
        Kind::Audio => include_str!("../assets/headphones.svg"),
        Kind::Volume => include_str!("../assets/volume-2.svg"),
        Kind::Mute => include_str!("../assets/volume-x.svg"),
        Kind::Microphone | Kind::MicVolume => include_str!("../assets/mic.svg"),
        Kind::MicMute => include_str!("../assets/mic-off.svg"),
    }
    .replace("stroke-width=\"2\"", "stroke-width=\"2.25\"");
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
    fn lucide_controls_use_antialiased_vectors_at_multiple_dpi() {
        for kind in [
            Kind::Display,
            Kind::Hdr,
            Kind::Audio,
            Kind::Volume,
            Kind::Mute,
            Kind::Microphone,
            Kind::MicVolume,
            Kind::MicMute,
        ] {
            for px in [24, 30, 48, 96] {
                let p = raster(kind, 0xffffff, px).unwrap();
                assert_eq!(p[3], 0);
                assert!(p.chunks_exact(4).any(|p| p[3] > 200));
                assert!(p.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
            }
        }
    }
}
