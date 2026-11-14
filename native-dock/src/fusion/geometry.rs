//! Geometry only: coordinates are physical screen pixels; no HWND or input API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pane {
    pub id: usize,
    pub rect: [f32; 4],
    pub radius: f32,
    pub dpi: f32,
    pub topmost: bool,
}
impl Pane {
    pub fn distance(&self, x: f32, y: f32) -> f32 {
        let [l, t, r, b] = self.rect;
        crate::compositor::distance(
            x - l,
            y - t,
            r - l,
            b - t,
            self.radius.min((r - l).min(b - t) * 0.5),
            0.,
        )
    }
}
fn overlap(a: f32, b: f32, c: f32, d: f32) -> f32 {
    b.min(d) - a.max(c)
}
/// Broad, C2-continuous optical transition, with identical material at the seam.
/// Distances are physical pixels; the transition itself is stable in DIP.
pub fn material_weight(normal: f32, tangent: f32, dpi: f32) -> f32 {
    let fade = |distance: f32, extent: f32| {
        let t = (1. - distance / (extent * dpi)).clamp(0., 1.);
        t * t * t * (t * (t * 6. - 15.) + 10.)
    };
    fade(normal, 140.) * fade(tangent, 80.)
}
pub use contour::with_outline;
/// Integrate edge coverage in physical pixels, shared by the member and bridge
/// surfaces. Integrating the same circular contour prevents a jagged rim at
/// fractional DPI and at the transition between member and bridge.
#[cfg(test)]
pub fn coverage(panes: &[Pane], x: f32, y: f32) -> f32 {
    if covered_core(panes, x, y) {
        return 1.;
    }
    let d = distance(panes, x, y);
    coverage_with_distance(panes, x, y, d)
}
/// Conservative rectangular strips and rounded cores inside members, away from
/// snapping tolerance. Their union is fully covered without evaluating the outline.
#[inline]
pub fn covered_core(panes: &[Pane], x: f32, y: f32) -> bool {
    covered_core_within(panes, x, y, 0.)
}
#[inline]
pub fn covered_core_within(panes: &[Pane], x: f32, y: f32, min_margin_dip: f32) -> bool {
    panes.iter().any(|p| {
        let [l, t, r, b] = p.rect;
        let margin = min_margin_dip.max(4.) * p.dpi;
        if x < l + margin || x >= r - margin || y < t + margin || y >= b - margin {
            return false;
        }
        let inset = p.radius + margin;
        (x >= l + inset && x < r - inset)
            || (y >= t + inset && y < b - inset)
            || p.distance(x, y) <= -margin
    })
}
#[cfg(test)]
#[inline]
pub fn coverage_with_distance(panes: &[Pane], x: f32, y: f32, d: f32) -> f32 {
    contour::coverage_with_distance(panes, x, y, d)
}
/// Accept the small overlap of translucent HWND margins, but not a visible gap.
pub fn touching_edges(end: f32, start: f32, scale: f32) -> bool {
    let overlap = end - start;
    overlap >= -1.5 * scale && overlap <= 8. * scale
}
pub fn adjacent(a: &Pane, b: &Pane) -> bool {
    if a.id == b.id || a.topmost != b.topmost {
        return false;
    }
    let scale = a.dpi.max(b.dpi);
    let [al, at, ar, ab] = a.rect;
    let [bl, bt, br, bb] = b.rect;
    ((touching_edges(ar, bl, scale) || touching_edges(br, al, scale))
        && overlap(at, ab, bt, bb) >= 48. * scale)
        || ((touching_edges(ab, bt, scale) || touching_edges(bb, at, scale))
            && overlap(al, ar, bl, br) >= 48. * scale)
}
/// Partition overlapping HWND margins. Each translucent background is painted
/// once, including at T junctions; otherwise two 50% panes make a 75% stripe.
pub fn owner(panes: &[Pane], x: f32, y: f32) -> Option<usize> {
    panes
        .iter()
        .filter(|p| {
            let [l, t, r, b] = p.rect;
            let m = 4. * p.dpi;
            x >= l - m && x < r + m && y >= t - m && y < b + m
        })
        .min_by(|a, b| {
            let d = |p: &Pane| {
                let [l, t, r, b] = p.rect;
                (l - x).max(x - r).max(t - y).max(y - b)
            };
            d(a).total_cmp(&d(b)).then(a.id.cmp(&b.id))
        })
        .map(|p| p.id)
}
pub fn components(panes: &[Pane]) -> Vec<Vec<Pane>> {
    let mut out = Vec::new();
    let mut seen = vec![false; panes.len()];
    for start in 0..panes.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut indexes = vec![start];
        let mut at = 0;
        while at < indexes.len() {
            for i in 0..panes.len() {
                if !seen[i] && adjacent(&panes[indexes[at]], &panes[i]) {
                    seen[i] = true;
                    indexes.push(i);
                }
            }
            at += 1;
        }
        out.push(indexes.into_iter().map(|i| panes[i]).collect());
    }
    out
}
#[path = "contour.rs"]
mod contour;
/// Distance to one rounded outline of the rectangle union. Collinear boundary
/// segments merge before rounding, so aligned joins have no corner at all.
#[cfg(test)]
pub fn distance(panes: &[Pane], x: f32, y: f32) -> f32 {
    contour::distance(panes, x, y)
}
/// Move just the dragged pane. Never moves another member of a fused component.
/// Resize only the grabbed edges (left/top/right/bottom). Opposite anchors and
/// minimum content dimensions remain invariant, including corner drags.
pub fn snap_resize(pane: &Pane, others: &[Pane], edges: [bool; 4], min: [f32; 2]) -> [f32; 4] {
    let mut rect = pane.rect;
    let threshold = 12. * pane.dpi;
    for side in 0..4 {
        if !edges[side] {
            continue;
        }
        let axis = side % 2;
        let tangent = 1 - axis;
        let mut best = threshold + 1.;
        let mut target = rect[side];
        for other in others {
            if other.id == pane.id || other.topmost != pane.topmost {
                continue;
            }
            if overlap(
                rect[tangent],
                rect[tangent + 2],
                other.rect[tangent],
                other.rect[tangent + 2],
            ) < -threshold
            {
                continue;
            }
            for value in [other.rect[axis], other.rect[axis + 2]] {
                let delta = (value - rect[side]).abs();
                let size = if side < 2 {
                    rect[side + 2] - value
                } else {
                    value - rect[side - 2]
                };
                if delta <= threshold && delta < best && size >= min[axis] {
                    best = delta;
                    target = value;
                }
            }
        }
        rect[side] = target;
    }
    rect
}
/// Move just the dragged pane. Never moves another member of a fused component.
pub fn snap_move(moving: &Pane, others: &[Pane]) -> [f32; 2] {
    let threshold = 12. * moving.dpi;
    let [l, t, r, b] = moving.rect;
    let mut best: Option<(f32, [f32; 2])> = None;
    for p in others {
        if p.id == moving.id || p.topmost != moving.topmost {
            continue;
        }
        let [pl, pt, pr, pb] = p.rect;
        let scale = moving.dpi.max(p.dpi);
        let mut try_delta = |delta: [f32; 2]| {
            let candidate = Pane {
                rect: [l + delta[0], t + delta[1], r + delta[0], b + delta[1]],
                ..*moving
            };
            if !adjacent(&candidate, p) {
                return;
            }
            let score = delta[0].abs() + delta[1].abs();
            if best.as_ref().is_none_or(|v| score < v.0) {
                best = Some((score, delta));
            }
        };
        if overlap(t, b, pt, pb) >= 48. * scale {
            for dx in [pl - r, pr - l] {
                if dx.abs() <= threshold {
                    let dy = [pt - t, pb - b]
                        .into_iter()
                        .filter(|v| v.abs() <= threshold)
                        .min_by(|a, b| a.abs().total_cmp(&b.abs()))
                        .unwrap_or(0.);
                    try_delta([dx, dy]);
                }
            }
        }
        if overlap(l, r, pl, pr) >= 48. * scale {
            for dy in [pt - b, pb - t] {
                if dy.abs() <= threshold {
                    let dx = [pl - l, pr - r]
                        .into_iter()
                        .filter(|v| v.abs() <= threshold)
                        .min_by(|a, b| a.abs().total_cmp(&b.abs()))
                        .unwrap_or(0.);
                    try_delta([dx, dy]);
                }
            }
        }
    }
    best.map(|v| v.1).unwrap_or([0., 0.])
}

#[cfg(test)]
mod tests {
    #[test]
    fn coverage_fast_path_matches_exact_contour_at_fractional_scales() {
        for scale in [1., 1.25, 1.5, 2.] {
            let panes = [
                pane(1, [0., 0., 160. * scale, 180. * scale]),
                pane(2, [0.8 * scale, 180.5 * scale, 300. * scale, 360. * scale]),
            ]
            .map(|p| Pane {
                radius: 34. * scale,
                dpi: scale,
                ..p
            });
            for y in (0..(360. * scale) as i32).step_by(3) {
                for x in (0..(300. * scale) as i32).step_by(3) {
                    let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
                    assert_eq!(
                        coverage(&panes, x, y),
                        coverage_with_distance(&panes, x, y, distance(&panes, x, y))
                    );
                }
            }
        }
    }
    use super::*;
    #[test]
    fn saved_clock_music_overlap_is_a_direct_material_neighbor() {
        let a = Pane {
            id: 1,
            rect: [1406., 763., 2041., 1356.],
            radius: 59.5,
            dpi: 1.75,
            topmost: false,
        };
        let b = Pane {
            id: 2,
            rect: [2033., 763., 3076., 1356.],
            ..a
        };
        assert!(adjacent(&a, &b));
        assert!(adjacent(&b, &a));
        assert!(touching_edges(a.rect[2], b.rect[0], 1.75));
        let gap = Pane {
            rect: [2048., 763., 3091., 1356.],
            ..b
        };
        assert!(!adjacent(&a, &gap), "do not fuse across visible gaps");
    }
    fn pane(id: usize, r: [f32; 4]) -> Pane {
        Pane {
            id,
            rect: r,
            radius: 34.,
            dpi: 1.,
            topmost: false,
        }
    }
    #[test]
    fn aligned_outer_edges_have_no_neck_at_any_rotation_or_dpi() {
        for dpi in [1., 1.25, 1.75, 2.] {
            for rotation in 0..4 {
                let rotate = |mut x: f32, mut y: f32| {
                    for _ in 0..rotation {
                        (x, y) = (-y, x);
                    }
                    [x * dpi, y * dpi]
                };
                let make = |id, r: [f32; 4]| {
                    let a = rotate(r[0], r[1]);
                    let b = rotate(r[2], r[3]);
                    Pane {
                        rect: [
                            a[0].min(b[0]),
                            a[1].min(b[1]),
                            a[0].max(b[0]),
                            a[1].max(b[1]),
                        ],
                        radius: 34. * dpi,
                        dpi,
                        ..pane(id, r)
                    }
                };
                let ps = [
                    make(1, [0., 0., 300., 200.]),
                    make(2, [0., 200., 700., 500.]),
                ];
                for y in 160..240 {
                    let p = rotate(0., y as f32);
                    assert!(distance(&ps, p[0], p[1]).abs() < 0.001);
                    let inside = rotate(1., y as f32);
                    let outside = rotate(-1., y as f32);
                    assert!(distance(&ps, inside[0], inside[1]) < 0.);
                    assert!(distance(&ps, outside[0], outside[1]) > 0.);
                }
            }
        }
    }
    #[test]
    fn convex_and_concave_corners_use_the_same_exact_circle() {
        for dpi in [1., 1.25, 1.75, 2.] {
            let mut ps = [
                pane(1, [0., 0., 300., 200.]),
                pane(2, [0., 200., 700., 500.]),
            ];
            for p in &mut ps {
                p.rect = p.rect.map(|v| v * dpi);
                p.radius *= dpi;
                p.dpi = dpi;
            }
            for i in 1..90 {
                let theta = (i as f32).to_radians();
                let convex = [
                    (34. - 34. * theta.cos()) * dpi,
                    (34. - 34. * theta.sin()) * dpi,
                ];
                let concave = [
                    (334. - 34. * theta.cos()) * dpi,
                    (166. + 34. * theta.sin()) * dpi,
                ];
                assert!(distance(&ps, convex[0], convex[1]).abs() < 0.002);
                assert!(distance(&ps, concave[0], concave[1]).abs() < 0.002);
            }
        }
    }
    #[test]
    fn full_aligned_join_is_one_rounded_rectangle_and_internal_alignment_is_straight() {
        let ps = [
            pane(1, [0., 0., 400., 200.]),
            pane(2, [0., 200., 400., 500.]),
        ];
        let whole = pane(3, [0., 0., 400., 500.]);
        for y in (-10..510).step_by(3) {
            for x in (-10..410).step_by(3) {
                assert!(
                    (distance(&ps, x as f32, y as f32) - whole.distance(x as f32, y as f32)).abs()
                        < 0.002
                );
            }
        }
        let ps = [
            pane(1, [0., 0., 300., 200.]),
            pane(2, [0., 200., 700., 500.]),
            pane(3, [-300., 500., 100., 800.]),
        ];
        for y in 160..240 {
            assert!(distance(&ps, 0., y as f32).abs() < 0.002);
        }
    }
    #[test]
    fn near_aligned_edges_share_a_single_straight_boundary() {
        let ps = [
            pane(1, [0., 0., 300., 200.]),
            pane(2, [0.8, 200.5, 700., 500.]),
        ];
        for y in 160..240 {
            assert!(distance(&ps, 0.4, y as f32).abs() < 0.002);
        }
    }
    #[test]
    fn resize_snaps_only_grabbed_edges_preserves_anchors_and_minimums() {
        let other = pane(2, [400., 0., 700., 300.]);
        let a = pane(1, [0., 0., 392., 292.]);
        assert_eq!(
            snap_resize(&a, &[other], [false, false, true, true], [240., 180.]),
            [0., 0., 400., 300.]
        );
        let a = pane(1, [408., 5., 900., 500.]);
        assert_eq!(
            snap_resize(&a, &[other], [true, true, false, false], [240., 180.]),
            [400., 0., 900., 500.]
        );
        let a = pane(1, [165., 0., 403., 300.]);
        assert_eq!(
            snap_resize(&a, &[other], [false, false, true, false], [238., 180.]),
            a.rect
        );
        let mut other = other;
        other.topmost = true;
        assert_eq!(
            snap_resize(&a, &[other], [false, false, true, false], [200., 180.]),
            a.rect
        );
    }
    #[test]
    fn resize_can_match_width_of_a_widget_above_at_fractional_dpi() {
        for dpi in [1., 1.25, 1.75, 2.] {
            let mut a = pane(1, [-700., 308., -208., 600.]);
            let mut b = pane(2, [-700., 0., -200., 300.]);
            for p in [&mut a, &mut b] {
                p.rect = p.rect.map(|v| v * dpi);
                p.dpi = dpi;
                p.radius *= dpi;
            }
            let r = snap_resize(
                &a,
                &[b],
                [false, false, true, false],
                [240. * dpi, 180. * dpi],
            );
            assert_eq!(r, [-700. * dpi, 308. * dpi, -200. * dpi, 600. * dpi]);
        }
    }
    #[test]
    fn blend_is_broad_symmetric_and_has_soft_endpoints() {
        for dpi in [1., 1.75, 2.] {
            assert_eq!(material_weight(0., 0., dpi), 1.);
            assert!(material_weight(56. * dpi, 0., dpi) > 0.6);
            assert!(material_weight(0., 40. * dpi, dpi) > 0.49);
            assert_eq!(material_weight(140. * dpi, 0., dpi), 0.);
            assert!(material_weight(139. * dpi, 0., dpi) < 0.0001);
        }
    }
    #[test]
    fn concave_edge_coverage_has_fractional_pixels() {
        let ps = [
            pane(1, [0., 0., 300., 200.]),
            pane(2, [0., 200., 700., 500.]),
        ];
        let mut fractional = 0;
        for y in 170..202 {
            for x in 296..335 {
                let c = coverage(&ps, x as f32 + 0.5, y as f32 + 0.5);
                fractional += (c > 0. && c < 1.) as usize;
            }
        }
        assert!(fractional > 12);
    }
    #[test]
    fn translucent_seams_have_one_owner_even_at_t_junctions() {
        let panes = [
            pane(1, [0., 0., 400., 300.]),
            pane(2, [400., 0., 700., 300.]),
            pane(3, [100., -200., 350., 0.]),
        ];
        for x in 396..404 {
            for y in [100., 150., 200.] {
                let x = x as f32 + 0.5;
                let owned = owner(&panes, x, y).unwrap();
                for alpha in [0u32, 64, 128, 255] {
                    let mut combined = 0u32;
                    for p in &panes {
                        let a = if owned == p.id { alpha } else { 0 };
                        combined = a + (combined * (255 - a) + 127) / 255;
                    }
                    assert_eq!(combined, alpha);
                }
            }
        }
        for x in 100..350 {
            assert!(owner(&panes, x as f32 + 0.5, 0.).is_some());
        }
    }
    #[test]
    fn mixed_dpi_snaps_only_when_the_same_pair_can_fuse() {
        let mut high = pane(1, [0., 0., 600., 800.]);
        high.dpi = 2.;
        let low = pane(2, [609., 740., 909., 1040.]);
        assert_eq!(snap_move(&low, &[high]), [0., 0.]);
        let mut low = pane(2, [609., 680., 909., 1040.]);
        let d = snap_move(&low, &[high]);
        assert_eq!(d, [-9., 0.]);
        low.rect[0] += d[0];
        low.rect[2] += d[0];
        assert!(adjacent(&high, &low));
    }
    #[test]
    fn chains_and_t_junctions_form_one_component_but_corners_do_not() {
        let a = pane(1, [0., 0., 400., 300.]);
        let b = pane(2, [400., 0., 700., 300.]);
        let c = pane(3, [100., -200., 350., 0.]);
        let d = pane(4, [700., 300., 900., 500.]);
        let cs = components(&[a, b, c, d]);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].len(), 3);
    }
    #[test]
    fn snapping_preserves_size_and_only_moves_the_dragged_pane() {
        let a = pane(1, [0., 0., 400., 300.]);
        let mut b = pane(2, [409., 5., 709., 305.]);
        let old = b.rect;
        let d = snap_move(&b, &[a]);
        assert_eq!(d, [-9., -5.]);
        b.rect = [old[0] + d[0], old[1] + d[1], old[2] + d[0], old[3] + d[1]];
        assert!(adjacent(&a, &b));
        assert_eq!(b.rect[2] - b.rect[0], 300.);
        assert_eq!(a.rect, [0., 0., 400., 300.]);
    }
    #[test]
    fn pulling_away_detaches_and_mixed_layers_do_not_snap() {
        let a = pane(1, [0., 0., 400., 300.]);
        let mut b = pane(2, [420., 0., 720., 300.]);
        assert_eq!(snap_move(&b, &[a]), [0., 0.]);
        assert!(!adjacent(&a, &b));
        b.rect = [400., 0., 700., 300.];
        b.topmost = true;
        assert!(!adjacent(&a, &b));
        assert_eq!(snap_move(&b, &[a]), [0., 0.]);
    }
    #[test]
    fn union_removes_internal_seam_and_rounds_the_concave_junction() {
        let a = pane(1, [0., 200., 700., 500.]);
        let b = pane(2, [0., 0., 300., 200.]);
        assert!(distance(&[a, b], 150., 200.) < -8.);
        assert!(distance(&[a, b], 303., 197.) < a.distance(303., 197.).min(b.distance(303., 197.)));
        assert_eq!(distance(&[a, b], 250., 100.), distance(&[b, a], 250., 100.));
        assert!(distance(&[a, b], 750., 100.) > 0.);
    }
    #[test]
    fn resizing_one_member_changes_topology_without_moving_others() {
        let a = pane(1, [0., 0., 400., 300.]);
        let b = pane(2, [400., 0., 700., 300.]);
        assert_eq!(components(&[a, b]).len(), 1);
        let resized = Pane {
            rect: [410., 0., 700., 300.],
            ..b
        };
        assert_eq!(components(&[a, resized]).len(), 2);
        assert_eq!(a.rect, [0., 0., 400., 300.]);
    }
    #[test]
    fn negative_monitor_coordinates_and_dpi_use_physical_geometry() {
        let mut a = pane(1, [-1200., -300., -600., 150.]);
        a.dpi = 1.5;
        let mut b = pane(2, [-584., -300., -134., 150.]);
        b.dpi = 1.5;
        assert_eq!(snap_move(&b, &[a]), [-16., 0.]);
    }
}
