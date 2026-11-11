//! Exact circular fillets of an orthogonal rectangle union, prepared once per
//! topology. No per-pixel smooth minimum: it changes curvature and dents edges.
use super::Pane;
use std::cell::RefCell;
type Point = [f32; 2];
#[derive(Clone)]
struct Corner {
    vertex: Point,
    center: Point,
    a: Point,
    b: Point,
    u: Point,
    v: Point,
    radius: f32,
    convex: bool,
}
struct Outline {
    rects: Vec<[f32; 4]>,
    lines: Vec<(Point, Point)>,
    corners: Vec<Corner>,
}
thread_local! { static CACHE: RefCell<Option<(Vec<Pane>, Outline)>> = const { RefCell::new(None) }; }
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: Point, b: Point) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn length(a: Point) -> f32 {
    a[0].hypot(a[1])
}
fn point(v: Point, u: Point, amount: f32) -> Point {
    [v[0] + u[0] * amount, v[1] + u[1] * amount]
}
fn segment(p: Point, a: Point, b: Point) -> f32 {
    let d = sub(b, a);
    let n = dot(d, d);
    let t = if n > 0. {
        (dot(sub(p, a), d) / n).clamp(0., 1.)
    } else {
        0.
    };
    length(sub(p, point(a, d, t)))
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
            lines: vec![],
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
                    vertex: v,
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
                out.lines
                    .push((corners[i].b, corners[(i + 1) % corners.len()].a));
            }
            out.corners.extend(corners);
        }
        out
    }
    fn distance(&self, p: Point) -> f32 {
        let mut inside = self
            .rects
            .iter()
            .any(|r| p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]);
        let mut d = f32::INFINITY;
        for &(a, b) in &self.lines {
            d = d.min(segment(p, a, b));
        }
        for c in &self.corners {
            let q = sub(p, c.center);
            let on_quadrant = dot(q, [-c.v[0], -c.v[1]]) >= 0. && dot(q, c.u) >= 0.;
            d = d.min(if on_quadrant {
                (length(q) - c.radius).abs()
            } else {
                length(sub(p, c.a)).min(length(sub(p, c.b)))
            });
            if (0..2).all(|i| {
                p[i] >= c.vertex[i].min(c.center[i]) && p[i] <= c.vertex[i].max(c.center[i])
            }) {
                inside = if c.convex {
                    length(q) <= c.radius
                } else {
                    length(q) >= c.radius
                };
            }
        }
        if inside {
            -d
        } else {
            d
        }
    }
}
pub(super) fn distance(panes: &[Pane], x: f32, y: f32) -> f32 {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().is_none_or(|(key, _)| key != panes) {
            *cache = Some((panes.to_vec(), Outline::new(panes)));
        }
        cache.as_ref().unwrap().1.distance([x, y])
    })
}
