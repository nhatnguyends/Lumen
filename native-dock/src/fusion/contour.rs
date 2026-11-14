//! Exact circular fillets of an orthogonal rectangle union, prepared once per
//! topology. No per-pixel smooth minimum: it changes curvature and dents edges.
use super::Pane;
use std::cell::RefCell;
type Point = [f32; 2];
#[derive(Clone)]
pub(crate) struct Corner {
    pub(crate) min: Point,
    pub(crate) max: Point,
    center: Point,
    a: Point,
    b: Point,
    u: Point,
    v: Point,
    radius: f32,
    convex: bool,
}
pub struct Outline {
    rects: Vec<[f32; 4]>,
    h_lines: Vec<[f32; 3]>,
    pub(crate) v_lines: Vec<[f32; 3]>,
    pub(crate) corners: Vec<Corner>,
}
thread_local! { static CACHE: RefCell<Option<(Vec<Pane>, Outline)>> = const { RefCell::new(None) }; }
#[inline]
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
#[inline]
fn length(a: Point) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}
#[inline]
fn point(v: Point, u: Point, amount: f32) -> Point {
    [v[0] + u[0] * amount, v[1] + u[1] * amount]
}
fn coordinates(panes: &[Pane], axis: usize, tolerance: f32) -> Vec<f32> {
    let mut raw: Vec<_> = panes
        .iter()
        .flat_map(|p| [p.rect[axis], p.rect[axis + 2]])
        .collect();
    raw.sort_unstable_by(f32::total_cmp);
    let mut result = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let mut end = i + 1;
        // Group only subpixel/snap tolerances, never actual short contour edges.
        while end < raw.len() && raw[end] - raw[i] <= tolerance {
            end += 1;
        }
        result.push(raw[i..end].iter().sum::<f32>() / (end - i) as f32);
        i = end;
    }
    result
}
impl Outline {
    fn new(panes: &[Pane]) -> Self {
        let tolerance = panes.iter().map(|p| p.dpi).fold(1f32, f32::max) * 1.5;
        let xs = coordinates(panes, 0, tolerance);
        let ys = coordinates(panes, 1, tolerance);
        let nearest = |a: f32, values: &[f32]| {
            *values
                .iter()
                .min_by(|x, y| (**x - a).abs().total_cmp(&(**y - a).abs()))
                .unwrap()
        };
        let rects: Vec<_> = panes
            .iter()
            .map(|p| {
                [
                    nearest(p.rect[0], &xs),
                    nearest(p.rect[1], &ys),
                    nearest(p.rect[2], &xs),
                    nearest(p.rect[3], &ys),
                ]
            })
            .collect();
        let mut out = Self {
            rects,
            h_lines: vec![],
            v_lines: vec![],
            corners: vec![],
        };
        if xs.len() < 2 || ys.len() < 2 {
            return out;
        }
        let nx = xs.len() - 1;
        let ny = ys.len() - 1;
        let mut filled = vec![false; nx * ny];
        for y in 0..ny {
            for x in 0..nx {
                let px = (xs[x] + xs[x + 1]) * 0.5;
                let py = (ys[y] + ys[y + 1]) * 0.5;
                filled[y * nx + x] = out
                    .rects
                    .iter()
                    .any(|r| px > r[0] && px < r[2] && py > r[1] && py < r[3]);
            }
        }
        let occupied = |x: isize, y: isize| {
            x >= 0
                && y >= 0
                && (x as usize) < nx
                && (y as usize) < ny
                && filled[y as usize * nx + x as usize]
        };
        let vertex = |x: usize, y: usize| y * xs.len() + x;
        let mut edges = Vec::new();
        for y in 0..ny {
            for x in 0..nx {
                if occupied(x as isize, y as isize) {
                    if !occupied(x as isize, y as isize - 1) {
                        edges.push((vertex(x, y), vertex(x + 1, y)));
                    }
                    if !occupied(x as isize + 1, y as isize) {
                        edges.push((vertex(x + 1, y), vertex(x + 1, y + 1)));
                    }
                    if !occupied(x as isize, y as isize + 1) {
                        edges.push((vertex(x + 1, y + 1), vertex(x, y + 1)));
                    }
                    if !occupied(x as isize - 1, y as isize) {
                        edges.push((vertex(x, y + 1), vertex(x, y)));
                    }
                }
            }
        }
        let pos = |i: usize| [xs[i % xs.len()], ys[i / xs.len()]];
        let direction = |a: usize, b: usize| {
            let d = sub(pos(b), pos(a));
            if d[0] > 0. {
                0
            } else if d[1] > 0. {
                1
            } else if d[0] < 0. {
                2
            } else {
                3
            }
        };
        let radius = panes
            .iter()
            .map(|p| {
                p.radius
                    .min((p.rect[2] - p.rect[0]).min(p.rect[3] - p.rect[1]) * 0.5)
            })
            .fold(f32::INFINITY, f32::min);
        while let Some((start, mut end)) = edges.pop() {
            let mut vertices = vec![start];
            let mut previous = start;
            while end != start {
                vertices.push(end);
                let incoming = direction(previous, end);
                let candidate = edges
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.0 == end)
                    .min_by_key(|(_, e)| match (direction(e.0, e.1) + 4 - incoming) % 4 {
                        1 => 0,
                        0 => 1,
                        3 => 2,
                        _ => 3,
                    })
                    .map(|(i, _)| i);
                let Some(i) = candidate else {
                    break;
                };
                previous = end;
                end = edges.swap_remove(i).1;
            }
            if end != start {
                continue;
            }
            // Grid subdivisions and shared widget endpoints are not corners.
            let vertices: Vec<_> = (0..vertices.len())
                .filter_map(|i| {
                    let prev = vertices[(i + vertices.len() - 1) % vertices.len()];
                    let v = vertices[i];
                    let next = vertices[(i + 1) % vertices.len()];
                    (direction(prev, v) != direction(v, next)).then_some(pos(v))
                })
                .collect();
            let mut corners = Vec::new();
            for i in 0..vertices.len() {
                let v = vertices[i];
                let before = sub(v, vertices[(i + vertices.len() - 1) % vertices.len()]);
                let after = sub(vertices[(i + 1) % vertices.len()], v);
                let bl = length(before);
                let al = length(after);
                let u = [before[0] / bl, before[1] / bl];
                let w = [after[0] / al, after[1] / al];
                let r = radius.min(bl * 0.5).min(al * 0.5);
                let a = point(v, u, -r);
                let b = point(v, w, r);
                let center = point(a, w, r);
                corners.push(Corner {
                    min: [v[0].min(center[0]), v[1].min(center[1])],
                    max: [v[0].max(center[0]), v[1].max(center[1])],
                    center,
                    a,
                    b,
                    u,
                    v: w,
                    radius: r,
                    convex: u[0] * w[1] - u[1] * w[0] > 0.,
                });
            }
            for i in 0..corners.len() {
                let a = corners[i].b;
                let b = corners[(i + 1) % corners.len()].a;
                if (a[1] - b[1]).abs() <= (a[0] - b[0]).abs() {
                    out.h_lines.push([a[1], a[0].min(b[0]), a[0].max(b[0])]);
                } else {
                    out.v_lines.push([a[0], a[1].min(b[1]), a[1].max(b[1])]);
                }
            }
            out.corners.extend(corners);
        }
        out
    }
    #[inline]
    pub fn distance(&self, p: Point) -> f32 {
        let px = p[0];
        let py = p[1];
        let mut inside = false;
        for r in &self.rects {
            if px >= r[0] && px <= r[2] && py >= r[1] && py <= r[3] {
                inside = true;
                break;
            }
        }
        let mut d_sq = f32::INFINITY;
        for &[y0, x0, x1] in &self.h_lines {
            let dx = (x0 - px).max(px - x1).max(0.);
            let dy = py - y0;
            d_sq = d_sq.min(dx * dx + dy * dy);
        }
        for &[x0, y0, y1] in &self.v_lines {
            let dx = px - x0;
            let dy = (y0 - py).max(py - y1).max(0.);
            d_sq = d_sq.min(dx * dx + dy * dy);
        }
        let mut d = d_sq.sqrt();
        for c in &self.corners {
            let qx = px - c.center[0];
            let qy = py - c.center[1];
            let box_d = (qx.abs() - c.radius).max(qy.abs() - c.radius);
            if box_d > 0. && box_d >= d {
                continue;
            }
            let q_len = (qx * qx + qy * qy).sqrt();
            let on_quadrant = qx * -c.v[0] + qy * -c.v[1] >= 0. && qx * c.u[0] + qy * c.u[1] >= 0.;
            d = d.min(if on_quadrant {
                (q_len - c.radius).abs()
            } else {
                let dax = px - c.a[0];
                let day = py - c.a[1];
                let dbx = px - c.b[0];
                let dby = py - c.b[1];
                (dax * dax + day * day).min(dbx * dbx + dby * dby).sqrt()
            });
            if px >= c.min[0] && px <= c.max[0] && py >= c.min[1] && py <= c.max[1] {
                inside = if c.convex {
                    q_len <= c.radius
                } else {
                    q_len >= c.radius
                };
            }
        }
        if inside {
            -d
        } else {
            d
        }
    }
    #[inline]
    pub fn coverage_with_distance(&self, x: f32, y: f32, d: f32) -> f32 {
        if d < -0.625 {
            return 1.;
        }
        if d > 0.625 {
            return 0.;
        }
        let mut inside = 0;
        for sy in 0..8 {
            let py = y + (sy as f32 + 0.5) * 0.125 - 0.5;
            for sx in 0..8 {
                let px = x + (sx as f32 + 0.5) * 0.125 - 0.5;
                inside += (self.distance([px, py]) <= 0.) as u32;
            }
        }
        inside as f32 * (1. / 64.)
    }
    #[inline]
    pub fn rim_light(&self, x: f32, y: f32, d: f32, dpi: f32) -> f32 {
        let band = 2.5 * dpi;
        if d < -band {
            return 0.;
        }
        let nx = self.distance([x + 0.5, y]) - self.distance([x - 0.5, y]);
        let ny = self.distance([x, y + 0.5]) - self.distance([x, y - 0.5]);
        let length = (nx * nx + ny * ny).sqrt().max(0.001);
        (0.09 + 0.28 * ((-nx * 0.6 - ny * 0.8) / length).max(0.))
            * (1. + d.min(0.) / band).clamp(0., 1.)
    }
}
#[inline]
pub fn with_outline<R>(panes: &[Pane], f: impl FnOnce(&Outline) -> R) -> R {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().is_none_or(|(key, _)| key.as_slice() != panes) {
            *cache = Some((panes.to_vec(), Outline::new(panes)));
        }
        f(&cache.as_ref().unwrap().1)
    })
}
#[cfg(test)]
pub(super) fn distance(panes: &[Pane], x: f32, y: f32) -> f32 {
    with_outline(panes, |outline| outline.distance([x, y]))
}
#[cfg(test)]
pub(super) fn coverage_with_distance(panes: &[Pane], x: f32, y: f32, d: f32) -> f32 {
    with_outline(panes, |outline| outline.coverage_with_distance(x, y, d))
}
