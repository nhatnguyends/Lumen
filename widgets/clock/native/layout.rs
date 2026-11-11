//! One proportional vector layout for every size, without theme breakpoints.
pub(super) fn frame(w: f32, h: f32) -> (f32, f32, f32) {
    // Preserve the original 340-DIP artwork. At very small heights reserve
    // header space above the first content row (62), without rearranging it.
    let scale = ((w - 16.) / 340.)
        .min((h - 16.) / 340.)
        .min((h - 52.) / 278.);
    let x = (w - 340. * scale) / 2.;
    let y = ((h - 340. * scale) / 2.).max(44. - 62. * scale);
    (x, y, scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proportional_faces_keep_the_same_geometry_through_old_breakpoints() {
        for size in [
            120., 128., 159., 160., 161., 200., 239., 240., 279., 280., 281., 380.,
        ] {
            let (x, y, s) = frame(size, size);
            assert!(s > 0.);
            assert!(x + 26. * s >= 8.);
            assert!(x + 314. * s <= size - 8.);
            assert!(y + 62. * s >= 44. - 0.001);
            assert!(y + 333. * s <= size - 8. + 0.001);
            let (nx, ny, ns) = frame(size + 0.01, size + 0.01);
            assert!((nx - x).abs() < 0.02 && (ny - y).abs() < 0.02 && (ns - s).abs() < 0.001);
        }
        for (w, h) in [(120., 320.), (320., 120.), (480., 300.)] {
            let (x, y, s) = frame(w, h);
            assert!(x >= 0. && y + 62. * s >= 44. - 0.001);
            assert!(y + 333. * s <= h - 8. + 0.001);
        }
        assert_eq!(frame(380., 380.), (8., 8., 364. / 340.));
    }
}
