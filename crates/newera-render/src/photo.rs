//! Photo renderer: a CPU path tracer over scene meshes with sunlight from
//! the home's location and time, light sources from furniture, soft
//! indirect light and translucent glass. Leaves CPU capacity for the editor.

use glam::{Vec3, Vec4};
use image::{Rgba, RgbaImage};

use crate::camera::View;
use crate::mesh::Mesh;
use crate::raster::{Images, detail};

/// A light source: a small sphere shining in every direction or as a spot
/// pointing down, or an oriented flat panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointLight {
    pub position: Vec3,
    /// Peak intensity per color channel (scene units per steradian); for
    /// panels, the radiance of their surface.
    pub intensity: Vec3,
    pub radius: f32,
    /// Spots: angle off straight down where intensity halves, radians; 0
    /// for light in every direction.
    pub half_angle: f32,
    /// Panels: half extents; their cross product is the emitting normal.
    pub panel: Option<(Vec3, Vec3)>,
    /// Geometry this close to the source (its own shade or reflector) doesn't
    /// block it, m.
    pub clearance: f32,
}

impl PointLight {
    /// Intensity toward the unit direction `dir` leaving the light.
    fn toward(&self, dir: Vec3) -> Vec3 {
        if let Some((u, v)) = self.panel {
            return self.intensity * u.cross(v).normalize_or_zero().dot(dir).max(0.0);
        }
        if self.half_angle <= 0.0 {
            return self.intensity;
        }
        let cos = -dir.y;
        if cos <= 0.0 {
            return Vec3::ZERO;
        }
        let theta = cos.min(1.0).acos();
        self.intensity * (-std::f32::consts::LN_2 * (theta / self.half_angle).powi(2)).exp()
    }

    /// How much it could light a point `d2` m² away (for picking one).
    fn weight(&self, d2: f32) -> f32 {
        let area = self
            .panel
            .map_or(1.0, |(u, v)| 4.0 * u.length() * v.length());
        luminance(self.intensity) * area / d2.max(0.04)
    }

    /// Where a camera ray meets its glowing surface, and the radiance seen.
    fn seen(&self, origin: Vec3, dir: Vec3) -> Option<(f32, Vec3)> {
        if let Some((u, v)) = self.panel {
            let normal = u.cross(v).normalize_or_zero();
            let facing = normal.dot(dir);
            if facing >= -1e-8 {
                return None;
            }
            let t = normal.dot(self.position - origin) / facing;
            let offset = origin + dir * t - self.position;
            let (a, b) = (
                offset.dot(u) / u.length_squared(),
                offset.dot(v) / v.length_squared(),
            );
            return (t > 0.0 && a.abs() <= 1.0 && b.abs() <= 1.0).then_some((t, self.intensity));
        }
        let oc = origin - self.position;
        let b = oc.dot(dir);
        let c = oc.length_squared() - self.radius * self.radius;
        let disc = b * b - c;
        if disc < 0.0 {
            return None;
        }
        let t = -b - disc.sqrt();
        (t > 0.0).then(|| {
            let area = std::f32::consts::PI * self.radius * self.radius;
            (t, self.toward(-dir) / area.max(1e-6))
        })
    }
}

/// The thread-shareable part of the options.
struct Lighting {
    bounces: u32,
    sun: Option<(Vec3, Vec3)>,
    sky: Vec3,
    lights: Vec<PointLight>,
}

/// Parameters of a photo.
pub struct PhotoOptions<'a> {
    pub width: u32,
    pub height: u32,
    pub view: View,
    /// Samples per pixel.
    pub samples: u32,
    /// Light bounces after the first hit.
    pub bounces: u32,
    /// Unit vector toward the sun and its radiance; `None` at night.
    pub sun: Option<(Vec3, Vec3)>,
    /// Radiance of the sky seen through openings.
    pub sky: Vec3,
    pub lights: Vec<PointLight>,
    pub exposure: f32,
    pub load_image: &'a dyn Fn(&str) -> Option<RgbaImage>,
}

impl std::fmt::Debug for PhotoOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoOptions")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("samples", &self.samples)
            .field("lights", &self.lights.len())
            .finish_non_exhaustive()
    }
}

/// Log-average luminance auto exposure aims for.
pub const AUTO_EXPOSURE_KEY: f32 = 0.12;

/// Sun position for a moment and place: `(azimuth, elevation)` in degrees,
/// azimuth clockwise from north. Simplified solar ephemeris (good to a
/// fraction of a degree, plenty for daylight).
pub fn sun_position(time_ms: i64, latitude: f64, longitude: f64) -> (f64, f64) {
    #[allow(clippy::cast_precision_loss)]
    let days = time_ms as f64 / 86_400_000.0 + 2_440_587.5 - 2_451_545.0;
    let rad = f64::to_radians;
    let g = rad(357.529 + 0.985_600_28 * days);
    let q = 280.459 + 0.985_647_36 * days;
    let l = rad(q + 1.915 * g.sin() + 0.020 * (2.0 * g).sin());
    let e = rad(23.439 - 0.000_000_36 * days);
    let right_ascension = (e.cos() * l.sin()).atan2(l.cos());
    let declination = (e.sin() * l.sin()).asin();
    let sidereal = (18.697_374_558 + 24.065_709_824_419_08 * days).rem_euclid(24.0) * 15.0;
    let hour_angle = rad(sidereal + longitude) - right_ascension;
    let lat = rad(latitude);
    let elevation =
        (lat.sin() * declination.sin() + lat.cos() * declination.cos() * hour_angle.cos()).asin();
    let azimuth =
        (-hour_angle.sin()).atan2(declination.tan() * lat.cos() - lat.sin() * hour_angle.cos());
    (
        azimuth.to_degrees().rem_euclid(360.0),
        elevation.to_degrees(),
    )
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Bounds {
    const EMPTY: Self = Self {
        min: Vec3::splat(f32::MAX),
        max: Vec3::splat(f32::MIN),
    };

    fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    fn hit(&self, origin: Vec3, inv: Vec3, max_t: f32) -> bool {
        let t1 = (self.min - origin) * inv;
        let t2 = (self.max - origin) * inv;
        let near = t1.min(t2).max_element();
        let far = t1.max(t2).min_element();
        near <= far && far >= 0.0 && near <= max_t
    }
}

struct Node {
    bounds: Bounds,
    /// Leaf: first triangle; inner: index of the right child (left is next).
    start: u32,
    /// Triangles in a leaf; 0 for inner nodes.
    count: u32,
}

struct Scene<'a> {
    mesh: &'a Mesh,
    tris: Vec<[u32; 3]>,
    transparent: Vec<bool>,
    nodes: Vec<Node>,
    images: Images<'a>,
}

impl Scene<'_> {
    fn corner(&self, i: u32) -> Vec3 {
        Vec3::from(self.mesh.vertices[i as usize].position)
    }

    fn build(&mut self) {
        if self.tris.is_empty() {
            self.nodes.clear();
            return;
        }
        let mut order: Vec<u32> = (0..u32::try_from(self.tris.len()).unwrap_or(0)).collect();
        let centroids: Vec<Vec3> = self
            .tris
            .iter()
            .map(|t| (self.corner(t[0]) + self.corner(t[1]) + self.corner(t[2])) / 3.0)
            .collect();
        let bounds: Vec<Bounds> = self
            .tris
            .iter()
            .map(|t| {
                let mut b = Bounds::EMPTY;
                for &i in t {
                    b.grow(self.corner(i));
                }
                b
            })
            .collect();
        let mut nodes = Vec::with_capacity(self.tris.len() * 2 / 3 + 1);
        split(&mut nodes, &mut order, 0, &centroids, &bounds);
        self.tris = order.iter().map(|&i| self.tris[i as usize]).collect();
        self.transparent = order
            .iter()
            .map(|&i| self.transparent[i as usize])
            .collect();
        self.nodes = nodes;
    }

    /// Whether triangle `k` has a hole at barycentric `(u, v)`: a masked
    /// image whose alpha there is under the cutoff lets rays through.
    fn hole(&self, k: usize, u: f32, v: f32) -> bool {
        let tri = self.tris[k];
        let va = &self.mesh.vertices[tri[0] as usize];
        if crate::mesh::kind_cutoff(va.kind).is_none() {
            return false;
        }
        let [vb, vc] = [tri[1], tri[2]].map(|i| &self.mesh.vertices[i as usize]);
        let w = 1.0 - u - v;
        let uv = [
            va.uv[0] * w + vb.uv[0] * u + vc.uv[0] * v,
            va.uv[1] * w + vb.uv[1] * u + vc.uv[1] * v,
        ];
        self.images.cut(va.kind, uv)
    }

    /// Nearest hit: `(t, triangle, u, v)`.
    fn intersect(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<(f32, usize, f32, f32)> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = dir.recip();
        let mut best: Option<(f32, usize, f32, f32)> = None;
        let mut stack = [0u32; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let index = stack[top];
            let node = &self.nodes[index as usize];
            let limit = best.map_or(max_t, |b| b.0);
            if !node.bounds.hit(origin, inv, limit) {
                continue;
            }
            if node.count > 0 {
                for k in node.start..node.start + node.count {
                    let t = self.tris[k as usize];
                    if let Some((d, u, v)) = triangle_hit(
                        origin,
                        dir,
                        self.corner(t[0]),
                        self.corner(t[1]),
                        self.corner(t[2]),
                    ) && d > 1e-4
                        && d < best.map_or(max_t, |b| b.0)
                        && !self.hole(k as usize, u, v)
                    {
                        best = Some((d, k as usize, u, v));
                    }
                }
            } else if top + 2 <= stack.len() {
                stack[top] = node.start;
                stack[top + 1] = index + 1;
                top += 2;
            }
        }
        best
    }

    /// Light passing from `origin` along `dir` for `distance`: 0 when
    /// blocked, reduced by glass.
    fn transmittance(&self, origin: Vec3, dir: Vec3, distance: f32) -> f32 {
        let mut from = origin;
        let mut left = distance;
        let mut through = 1.0;
        for _ in 0..4 {
            let Some((t, k, _, _)) = self.intersect(from, dir, left) else {
                return through;
            };
            if !self.transparent[k] {
                return 0.0;
            }
            let alpha = self.mesh.vertices[self.tris[k][0] as usize].color[3];
            through *= 1.0 - alpha.clamp(0.0, 1.0) * 0.85;
            from += dir * (t + 1e-3);
            left -= t + 1e-3;
            if left <= 0.0 {
                return through;
            }
        }
        through
    }
}

/// Builds nodes depth-first: a node is followed by its left subtree; inner
/// nodes store the index of their right child in `start`.
fn split(
    nodes: &mut Vec<Node>,
    order: &mut [u32],
    offset: u32,
    centroids: &[Vec3],
    bounds: &[Bounds],
) {
    let mut b = Bounds::EMPTY;
    let mut c = Bounds::EMPTY;
    for &i in order.iter() {
        b.grow(bounds[i as usize].min);
        b.grow(bounds[i as usize].max);
        c.grow(centroids[i as usize]);
    }
    let index = nodes.len();
    nodes.push(Node {
        bounds: b,
        start: offset,
        count: u32::try_from(order.len()).unwrap_or(0),
    });
    if order.len() <= 4 {
        return;
    }
    let extent = c.max - c.min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    if extent[axis] < 1e-6 {
        return;
    }
    let mid = order.len() / 2;
    order.select_nth_unstable_by(mid, |a, b| {
        centroids[*a as usize][axis].total_cmp(&centroids[*b as usize][axis])
    });
    let (left, right) = order.split_at_mut(mid);
    nodes[index].count = 0;
    split(nodes, left, offset, centroids, bounds);
    nodes[index].start = u32::try_from(nodes.len()).unwrap_or(0);
    split(
        nodes,
        right,
        offset + u32::try_from(mid).unwrap_or(0),
        centroids,
        bounds,
    );
}

fn triangle_hit(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, f32, f32)> {
    let (e1, e2) = (b - a, c - a);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some((e2.dot(q) * inv, u, v))
}

/// Small fast random numbers (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        // The top 24 bits fit an f32 exactly.
        let bits = u32::try_from(self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40).unwrap_or(0);
        #[allow(clippy::cast_precision_loss)]
        let unit = bits as f32 / 16_777_216.0;
        unit
    }
}

fn to_linear(c: Vec3) -> Vec3 {
    c.powf(2.2)
}

/// Cosine-weighted direction around `n`.
fn hemisphere(n: Vec3, rng: &mut Rng) -> Vec3 {
    let (r1, r2) = (rng.next(), rng.next());
    let phi = std::f32::consts::TAU * r1;
    let r = r2.sqrt();
    let (x, y, z) = (r * phi.cos(), r * phi.sin(), (1.0 - r2).sqrt());
    let helper = if n.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
    let t = helper.cross(n).normalize();
    let b = n.cross(t);
    (t * x + b * y + n * z).normalize()
}

/// Transparent surfaces at most this opaque are clear glass (window glass is
/// 0.35): rays always cross them.
pub const CLEAR_GLASS_ALPHA: f32 = 0.5;

/// A tent-distributed offset in -1..1 from a uniform number.
fn tent(r: f32) -> f32 {
    let r = 2.0 * r;
    if r < 1.0 {
        r.sqrt() - 1.0
    } else {
        1.0 - (2.0 - r).sqrt()
    }
}

fn aces(x: Vec3) -> Vec3 {
    let (a, b, c, d, e) = (2.51, 0.03, 2.43, 0.59, 0.14);
    ((x * (x * a + b)) / (x * (x * c + d) + e)).clamp(Vec3::ZERO, Vec3::ONE)
}

#[allow(clippy::too_many_lines)]
/// What the camera ray first sees, to guide denoising.
#[derive(Clone, Copy, Default)]
struct Surface {
    albedo: Vec3,
    normal: Vec3,
    depth: f32,
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

fn radiance(
    scene: &Scene<'_>,
    options: &Lighting,
    mut origin: Vec3,
    mut dir: Vec3,
    rng: &mut Rng,
    first: &mut Option<Surface>,
) -> Vec3 {
    let mut throughput = Vec3::ONE;
    let mut total = Vec3::ZERO;
    let mut depth = 0;
    let mut pass_through = 0;
    loop {
        let hit = scene.intersect(origin, dir, 1e4);
        // Lamps seen straight from the camera glow; a lens or diffuser just
        // in front of the source counts as the source.
        if depth == 0 {
            let nearest = options
                .lights
                .iter()
                .filter_map(|l| l.seen(origin, dir))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((t_light, glow)) = nearest
                && hit.is_none_or(|h| t_light <= h.0 + 0.02)
            {
                if first.is_none() {
                    *first = Some(Surface {
                        albedo: Vec3::ONE,
                        normal: -dir,
                        depth: t_light,
                    });
                }
                return total + throughput * glow;
            }
        }
        let Some((t, k, u, v)) = hit else {
            let sun_disc = options.sun.map_or(Vec3::ZERO, |(sun_dir, sun)| {
                if depth == 0 && dir.dot(sun_dir) > 0.9995 {
                    sun * 0.02
                } else {
                    Vec3::ZERO
                }
            });
            if first.is_none() {
                *first = Some(Surface {
                    albedo: Vec3::ONE,
                    normal: -dir,
                    depth: 1e4,
                });
            }
            return total + throughput * (options.sky + sun_disc);
        };
        let tri = scene.tris[k];
        let w = 1.0 - u - v;
        let [va, vb, vc] = tri.map(|i| &scene.mesh.vertices[i as usize]);
        let point = origin + dir * t;
        let mut normal =
            (Vec3::from(va.normal) * w + Vec3::from(vb.normal) * u + Vec3::from(vc.normal) * v)
                .normalize_or_zero();
        if normal == Vec3::ZERO {
            normal = (Vec3::from(vb.position) - Vec3::from(va.position))
                .cross(Vec3::from(vc.position) - Vec3::from(va.position))
                .normalize_or_zero();
        }
        if normal.dot(dir) > 0.0 {
            normal = -normal;
        }
        let color = Vec4::from(va.color) * w + Vec4::from(vb.color) * u + Vec4::from(vc.color) * v;
        let uv = [
            va.uv[0] * w + vb.uv[0] * u + vc.uv[0] * v,
            va.uv[1] * w + vb.uv[1] * u + vc.uv[1] * v,
        ];
        // Like Sweet Home 3D photos: material colors are linear albedo,
        // only image textures (and patterns standing in for them) are sRGB.
        let base = (color.truncate() * to_linear(detail(va.kind, uv, 0.002, &scene.images)))
            .min(Vec3::splat(0.95));

        // Clear glass lets light through on every sample, reflecting some
        // sky (Schlick, F0 = 0.04): picking between surface and
        // see-through at random left windows full of white dots.
        let alpha = color.w.clamp(0.0, 1.0);
        if scene.transparent[k] && pass_through < 6 && alpha <= CLEAR_GLASS_ALPHA {
            let fresnel = 0.04 + 0.96 * (1.0 - normal.dot(-dir).abs()).powi(5);
            // Only where the mirror image is open sky: glass inside a room
            // reflects the room, which is dim next to the sky.
            let mirror = dir - normal * 2.0 * dir.dot(normal);
            if scene
                .intersect(point + mirror * 1e-3, mirror, 1e4)
                .is_none()
            {
                total += throughput * options.sky * fresnel;
            }
            origin = point + dir * 1e-3;
            throughput *= Vec3::ONE.lerp(base, 0.15 + alpha * 0.3) * (1.0 - fresnel);
            pass_through += 1;
            continue;
        }
        // Frosted or tinted panels: seen as a surface part of the time.
        if scene.transparent[k] && pass_through < 6 && rng.next() > alpha * 0.6 {
            origin = point + dir * 1e-3;
            throughput *= Vec3::ONE.lerp(base, 0.15);
            pass_through += 1;
            continue;
        }

        if first.is_none() {
            *first = Some(Surface {
                albedo: base,
                normal,
                depth: (point - origin).length() + t * 0.0,
            });
        }
        let at = point + normal * 1e-3;
        // Sunlight.
        if let Some((sun_dir, sun)) = options.sun {
            let cos = normal.dot(sun_dir);
            if cos > 0.0 {
                let visible = scene.transmittance(at, sun_dir, 1e4);
                total += throughput * base * sun * (cos * visible / std::f32::consts::PI);
            }
        }
        // One lamp, picked in proportion to how much it could light this
        // point (intensity over squared distance, facing it).
        let weights: Vec<f32> = options
            .lights
            .iter()
            .map(|l| {
                let to = l.position - at;
                if normal.dot(to) <= 0.0 && l.panel.is_none() {
                    0.0
                } else {
                    l.weight(to.length_squared())
                }
            })
            .collect();
        let weight_sum: f32 = weights.iter().sum();
        if weight_sum > 0.0 {
            let mut target = rng.next() * weight_sum;
            let mut pick = weights.len() - 1;
            for (i, w) in weights.iter().enumerate() {
                if target < *w {
                    pick = i;
                    break;
                }
                target -= w;
            }
            let probability = weights[pick] / weight_sum;
            let light = options.lights[pick];
            // A point on the panel, or on the bulb.
            let (source, area) = match light.panel {
                Some((u, v)) => (
                    light.position + u * (rng.next() * 2.0 - 1.0) + v * (rng.next() * 2.0 - 1.0),
                    4.0 * u.length() * v.length(),
                ),
                None => (
                    light.position + hemisphere(Vec3::Y, rng) * light.radius,
                    1.0,
                ),
            };
            let to_light = source - at;
            let distance = to_light.length();
            let l = to_light / distance.max(1e-4);
            let cos = normal.dot(l);
            if cos > 0.0 {
                let reach = (distance - light.radius.max(light.clearance)).max(0.0);
                let visible = scene.transmittance(at, l, reach);
                total += throughput
                    * base
                    * light.toward(-l)
                    * (cos * area * visible
                        / probability.max(1e-6)
                        / (distance * distance).max(0.04)
                        / std::f32::consts::PI);
            }
        }

        if depth >= options.bounces {
            return total;
        }
        throughput *= base;
        if depth >= 2 {
            let survive = throughput.max_element().clamp(0.05, 0.95);
            if rng.next() > survive {
                return total;
            }
            throughput /= survive;
        }
        origin = at;
        dir = hemisphere(normal, rng);
        depth += 1;
    }
}

/// Renders a photo of the mesh.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn render_photo(mesh: &Mesh, options: &PhotoOptions<'_>) -> RgbaImage {
    newera_core::progress::step("Preparando cena", 0, 0);
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let mut transparent = Vec::new();
    for (list, clear) in [(&mesh.indices, false), (&mesh.transparent, true)] {
        for t in list.as_chunks::<3>().0 {
            tris.push(*t);
            transparent.push(clear);
        }
    }
    let mut scene = Scene {
        mesh,
        tris,
        transparent,
        nodes: Vec::new(),
        images: Images {
            loaded: mesh
                .images
                .iter()
                .map(|p| (options.load_image)(p))
                .collect(),
            _source: std::marker::PhantomData,
        },
    };
    scene.build();
    if newera_core::progress::cancelled() {
        return RgbaImage::new(1, 1);
    }

    let (w, h) = (
        options.width.max(1) as usize,
        options.height.max(1) as usize,
    );
    let aspect = w as f32 / h as f32;
    let forward = (options.view.target - options.view.eye).normalize();
    let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(forward);
    let tan = (options.view.fov_y / 2.0).tan();
    let samples = options.samples.max(1);

    let lighting = Lighting {
        bounces: options.bounces,
        sun: options.sun,
        sky: options.sky,
        lights: options.lights.clone(),
    };
    let (eye, exposure) = (options.view.eye, options.exposure);
    // Where the final exposure will land, from a coarse grid of rays: samples
    // are averaged after squeezing their brightness at that exposure (and
    // expanded back), so one bright sky or lamp sample can't take over a
    // pixel on an edge and bring the staircase back.
    let squeeze = {
        let mut rng = Rng(0x5EED_1234_ABCD_0001);
        let (gw, gh) = (24usize, 18usize);
        let mut log_sum = 0.0f32;
        for gy in 0..gh {
            if newera_core::progress::cancelled() {
                return RgbaImage::new(1, 1);
            }
            for gx in 0..gw {
                let sx = ((gx as f32 + 0.5) / gw as f32 * 2.0 - 1.0) * tan * aspect;
                let sy = (1.0 - (gy as f32 + 0.5) / gh as f32 * 2.0) * tan;
                let dir = (forward + right * sx + up * sy).normalize();
                let value = radiance(&scene, &lighting, eye, dir, &mut rng, &mut None);
                let value = if value.is_finite() {
                    value.min(Vec3::splat(20.0))
                } else {
                    Vec3::ZERO
                };
                log_sum += (luminance(value) + 1e-4).ln();
            }
        }
        let auto = (AUTO_EXPOSURE_KEY / (log_sum / (gw * gh) as f32).exp()).clamp(0.3, 4000.0);
        exposure * auto
    };
    let mut pixels = vec![(Vec3::ZERO, Surface::default()); w * h];
    // Browsers (wasm32) have no threads: work on the calling one there.
    let threads = if cfg!(target_arch = "wasm32") {
        1
    } else {
        render_threads()
    };
    let rows_per = h.div_ceil(threads);
    let watcher = newera_core::progress::listener();
    newera_core::progress::step("Renderizando imagem", 0, h as u64);
    let completed = std::sync::atomic::AtomicU64::new(0);
    let work = |chunk_index: usize, chunk: &mut [(Vec3, Surface)]| {
        {
            let (scene, lighting) = (&scene, &lighting);
            {
                for (i, pixel) in chunk.iter_mut().enumerate() {
                    if watcher.as_ref().is_some_and(|p| p.cancelled()) {
                        return;
                    }
                    let (x, y) = (i % w, chunk_index * rows_per + i / w);
                    let mut rng =
                        Rng(((y as u64) << 32 | x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
                    let mut sum = Vec3::ZERO;
                    let mut surface = Surface::default();
                    for _ in 0..samples {
                        if watcher.as_ref().is_some_and(|p| p.cancelled()) {
                            return;
                        }
                        // Tent filter over neighboring pixels: smooth edges.
                        let (jx, jy) = (0.5 + tent(rng.next()), 0.5 + tent(rng.next()));
                        let sx = ((x as f32 + jx) / w as f32 * 2.0 - 1.0) * tan * aspect;
                        let sy = (1.0 - (y as f32 + jy) / h as f32 * 2.0) * tan;
                        let dir = (forward + right * sx + up * sy).normalize();
                        let mut first = None;
                        let value = radiance(scene, lighting, eye, dir, &mut rng, &mut first);
                        // Clamp fireflies from rare paths; a broken sample adds nothing.
                        if value.is_finite() {
                            let value = value.min(Vec3::splat(20.0));
                            sum += value / (1.0 + luminance(value) * squeeze);
                        }
                        let f = first.unwrap_or_default();
                        surface.albedo += f.albedo;
                        surface.normal += f.normal;
                        surface.depth += f.depth;
                    }
                    let n = samples as f32;
                    let squeezed = sum / n;
                    *pixel = (
                        squeezed / (1.0 - luminance(squeezed) * squeeze).max(0.05),
                        Surface {
                            albedo: surface.albedo / n,
                            normal: surface.normal.normalize_or_zero(),
                            depth: surface.depth / n,
                        },
                    );
                    if (i + 1) % w == 0 {
                        let done = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        if let Some(p) = &watcher {
                            p.step("Renderizando imagem", done, h as u64);
                        }
                    }
                }
            }
        }
    };
    if threads == 1 {
        work(0, &mut pixels);
    } else {
        #[allow(
            clippy::disallowed_methods,
            reason = "this branch needs more than one thread, and on wasm32 `threads` is 1 (the `cfg!` above)"
        )]
        std::thread::scope(|s| {
            for (chunk_index, chunk) in pixels.chunks_mut(rows_per * w).enumerate() {
                let work = &work;
                s.spawn(move || work(chunk_index, chunk));
            }
        });
    }

    if watcher.as_ref().is_some_and(|p| p.cancelled()) {
        return RgbaImage::new(1, 1);
    }
    newera_core::progress::step("Finalizando imagem", 0, 0);
    let mut denoised = denoise(&pixels, w, h);
    white_balance(&mut denoised, WHITE_BALANCE);
    // Auto exposure: bring the scene's average brightness to a middle tone
    // (calibrated against Sweet Home 3D photos of the same cameras).
    let log_mean = denoised
        .iter()
        .map(|c| (luminance(*c) + 1e-4).ln())
        .sum::<f32>()
        / denoised.len().max(1) as f32;
    let auto = (AUTO_EXPOSURE_KEY / log_mean.exp()).clamp(0.3, 4000.0);
    let mut image = RgbaImage::new(w as u32, h as u32);
    for (i, value) in denoised.iter().enumerate() {
        let mapped = aces(*value * exposure * auto).powf(1.0 / 2.2) * 255.0;
        image.put_pixel(
            (i % w) as u32,
            (i / w) as u32,
            Rgba([mapped.x as u8, mapped.y as u8, mapped.z as u8, 255]),
        );
    }
    image
}

/// Worker threads for a photo: `NEWERA_RENDER_THREADS` when set, otherwise
/// half the cores. Every core flat out for minutes has tripped a desktop's
/// power protection, and the machine stays usable while it renders.
/// The threads a photo may use: half the cores, at most four, fewer when
/// `NEWERA_RENDER_THREADS` says so.
#[must_use]
pub fn thread_budget() -> usize {
    let budget =
        (std::thread::available_parallelism().map_or(2, std::num::NonZero::get) / 2).clamp(1, 4);
    std::env::var("NEWERA_RENDER_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(budget)
        .clamp(1, budget)
}

thread_local! {
    static THREAD_CAP: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Runs `work` with the photos it renders on this thread using at most
/// `threads` — never more than [`thread_budget`], whatever is asked.
pub fn with_thread_cap<R>(threads: usize, work: impl FnOnce() -> R) -> R {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            THREAD_CAP.with(|cap| cap.set(self.0));
        }
    }
    let _restore = Restore(THREAD_CAP.with(|cap| cap.replace(Some(threads.max(1)))));
    work()
}

fn render_threads() -> usize {
    let budget = thread_budget();
    THREAD_CAP
        .with(std::cell::Cell::get)
        .map_or(budget, |cap| cap.clamp(1, budget))
}

/// How far photos are pulled toward neutral (0 none, 1 full gray world):
/// halfway keeps warm lamps warm but stops daylight rooms with 3000 K
/// downlights on from turning orange, as a camera's auto white balance does.
pub const WHITE_BALANCE: f32 = 0.5;

/// Gray-world white balance: channel gains that make the scene's average
/// color neutral, raised to `strength` and keeping luminance. Very bright
/// pixels (sky through windows, lamp glow) don't vote.
#[allow(clippy::cast_precision_loss)]
fn white_balance(pixels: &mut [Vec3], strength: f32) {
    let mean_lum = pixels.iter().map(|c| luminance(*c)).sum::<f32>() / pixels.len().max(1) as f32;
    let cap = mean_lum * 4.0;
    let (sum, count) = pixels
        .iter()
        .filter(|c| c.is_finite() && luminance(**c) <= cap)
        .fold((Vec3::ZERO, 0usize), |(s, n), c| (s + *c, n + 1));
    if count == 0 || strength <= 0.0 {
        return;
    }
    let average = sum / count as f32;
    let lum = luminance(average);
    if lum <= 1e-8 || average.min_element() <= 1e-8 {
        return;
    }
    let raw = Vec3::splat(lum) / average;
    let gains = Vec3::new(
        raw.x.powf(strength),
        raw.y.powf(strength),
        raw.z.powf(strength),
    )
    .clamp(Vec3::splat(0.5), Vec3::splat(2.0));
    // Same brightness after the gains.
    let gains = gains * (lum / luminance(average * gains).max(1e-8));
    for c in pixels.iter_mut() {
        *c *= gains;
    }
}

/// Edge-aware à-trous filter on lighting (radiance divided by albedo), so
/// textures stay sharp while noise in light and shadow is smoothed.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]
fn denoise(pixels: &[(Vec3, Surface)], w: usize, h: usize) -> Vec<Vec3> {
    let albedo: Vec<Vec3> = pixels
        .iter()
        .map(|(_, s)| s.albedo.max(Vec3::splat(0.03)))
        .collect();
    let mut light: Vec<Vec3> = pixels
        .iter()
        .zip(&albedo)
        .map(|((c, _), a)| *c / *a)
        .collect();
    let kernel = [1.0 / 16.0, 0.25, 3.0 / 8.0, 0.25, 1.0 / 16.0];
    for pass in 0..5 {
        let step = 1i64 << pass;
        let source = light.clone();
        for y in 0..h {
            for x in 0..w {
                let center = y * w + x;
                let (c, s) = (source[center], pixels[center].1);
                let cl = luminance(c);
                let (mut sum, mut weight) = (Vec3::ZERO, 0.0f32);
                for (j, ky) in kernel.iter().enumerate() {
                    let yy = y as i64 + (j as i64 - 2) * step;
                    if yy < 0 || yy >= h as i64 {
                        continue;
                    }
                    for (i, kx) in kernel.iter().enumerate() {
                        let xx = x as i64 + (i as i64 - 2) * step;
                        if xx < 0 || xx >= w as i64 {
                            continue;
                        }
                        let index = yy as usize * w + xx as usize;
                        let (q, qs) = (source[index], pixels[index].1);
                        let normal = s.normal.dot(qs.normal).max(0.0).powi(32);
                        let depth = (-(s.depth - qs.depth).abs()
                            / (0.05 * s.depth.max(0.5) * step as f32))
                            .exp();
                        // Symmetric in the pair, so a pixel whose few samples all
                        // missed the light still borrows from brighter neighbors.
                        let ql = luminance(q);
                        let lum = (-(cl - ql).abs() / (0.6 * (cl.max(ql) + 0.05))).exp();
                        let wgt = kx * ky * normal * depth * lum;
                        sum += q * wgt;
                        weight += wgt;
                    }
                }
                light[center] = if weight > 1e-6 { sum / weight } else { c };
            }
        }
    }
    light.iter().zip(&albedo).map(|(l, a)| *l * *a).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_mesh_renders_sky_without_an_invalid_bvh_child() {
        let home = newera_core::Home::default();
        let image = render_photo(
            &Mesh::default(),
            &PhotoOptions {
                width: 8,
                height: 6,
                samples: 1,
                bounces: 1,
                view: View::aerial(&home, 30.0, 40.0),
                sun: None,
                sky: Vec3::ONE,
                lights: vec![],
                exposure: 1.0,
                load_image: &|_| None,
            },
        );
        assert_eq!(image.dimensions(), (8, 6));
        assert!(
            image
                .pixels()
                .all(|p| p.0[3] == 255 && p.0[..3].iter().any(|v| *v > 0))
        );
    }

    #[test]
    fn cancellation_is_observed_before_finishing_a_long_row() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        };
        #[derive(Default)]
        struct CancelDuringSamples {
            rendering: AtomicBool,
            checks: AtomicU64,
            rows: AtomicU64,
        }
        impl newera_core::progress::Watcher for CancelDuringSamples {
            fn step(&self, phase: &str, done: u64, _: u64) {
                if phase == "Renderizando imagem" {
                    self.rendering.store(true, Ordering::SeqCst);
                    self.rows.store(done, Ordering::SeqCst);
                }
            }
            fn cancelled(&self) -> bool {
                self.rendering.load(Ordering::SeqCst)
                    && self.checks.fetch_add(1, Ordering::SeqCst) >= 8
            }
        }
        let state = Arc::new(CancelDuringSamples::default());
        let watcher: Arc<dyn newera_core::progress::Watcher> = state.clone();
        let home = newera_core::Home::default();
        let image = newera_core::progress::watched(&watcher, || {
            render_photo(
                &Mesh::default(),
                &PhotoOptions {
                    width: 128,
                    height: 1,
                    samples: 192,
                    bounces: 1,
                    view: View::aerial(&home, 30.0, 40.0),
                    sun: None,
                    sky: Vec3::ONE,
                    lights: vec![],
                    exposure: 1.0,
                    load_image: &|_| None,
                },
            )
        });
        assert_eq!(image.dimensions(), (1, 1));
        assert_eq!(
            state.rows.load(Ordering::SeqCst),
            0,
            "cancellation must interrupt samples, not wait for the full row"
        );
    }

    #[test]
    fn panels_emit_and_are_visible_only_on_their_luminous_side() {
        for sign in [-1.0, 1.0] {
            let light = PointLight {
                position: Vec3::ZERO,
                intensity: Vec3::ONE,
                radius: 0.0,
                half_angle: 0.0,
                panel: Some((Vec3::X, Vec3::Z * sign)),
                clearance: 0.01,
            };
            let normal = -Vec3::Y * sign;
            assert!(light.toward(normal).distance(Vec3::ONE) < 1e-6);
            assert!(light.toward(-normal).length() < 1e-6);
            assert!(light.seen(normal * 2.0, -normal).is_some());
            assert!(light.seen(-normal * 2.0, normal).is_none());
            assert!(light.seen(normal * 2.0 + Vec3::X * 3.0, -normal).is_none());
        }
    }

    #[test]
    fn white_balance_pulls_an_orange_room_halfway_to_neutral() {
        let orange = Vec3::new(0.9, 0.55, 0.3);
        let mut pixels = vec![orange; 64];
        // A blown-out window doesn't count.
        pixels[0] = Vec3::splat(50.0);
        white_balance(&mut pixels, 0.5);
        let c = pixels[10];
        let ratio = |v: Vec3| v.x / v.z;
        assert!(ratio(c) < ratio(orange) && ratio(c) > 1.2, "{c:?}");
        assert!((luminance(c) - luminance(orange)).abs() < 1e-3, "{c:?}");
        let mut full = vec![orange; 8];
        white_balance(&mut full, 1.0);
        // Gains are capped at 2×, so blue lands a hair short.
        assert!((full[0].x - full[0].z).abs() < 0.02, "{:?}", full[0]);
    }

    #[test]
    fn noon_sun_is_high_and_midnight_below_the_horizon() {
        // 2026-06-21 12:00 local solar time at the equator's longitude 0.
        let noon = 1_782_043_200_000;
        let (_, elevation) = sun_position(noon, 0.0, 0.0);
        assert!(elevation > 60.0, "{elevation}");
        let (_, night) = sun_position(noon + 12 * 3_600_000, 0.0, 0.0);
        assert!(night < 0.0, "{night}");
        // São Paulo, morning: sun to the east (azimuth between 45° and 135°).
        let morning = 1_789_214_400_000 - 3 * 3_600_000; // 09:00 UTC ≈ 06:00 local
        let (azimuth, _) = sun_position(morning + 3 * 3_600_000, -23.5, -46.6);
        assert!((45.0..135.0).contains(&azimuth), "{azimuth}");
    }

    #[test]
    fn triangles_are_hit_from_either_side() {
        let (a, b, c) = (Vec3::ZERO, Vec3::X, Vec3::Y);
        let hit = triangle_hit(Vec3::new(0.2, 0.2, 1.0), -Vec3::Z, a, b, c).unwrap();
        assert!((hit.0 - 1.0).abs() < 1e-5);
        assert!(triangle_hit(Vec3::new(0.2, 0.2, -1.0), Vec3::Z, a, b, c).is_some());
        assert!(triangle_hit(Vec3::new(2.0, 2.0, 1.0), -Vec3::Z, a, b, c).is_none());
    }

    #[test]
    fn a_ray_passes_through_the_holes_of_a_masked_image_and_stops_at_its_strands() {
        // A 2 × 2 sheet at z = 0 whose image is clear on its left half.
        let kind = crate::mesh::masked(crate::mesh::IMAGE_BASE, 0.5);
        let corner = |x: f32, y: f32| crate::mesh::Vertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            color: [1.0; 4],
            uv: [x / 2.0, y / 2.0],
            kind,
        };
        let mesh = Mesh {
            vertices: vec![
                corner(0.0, 0.0),
                corner(2.0, 0.0),
                corner(2.0, 2.0),
                corner(0.0, 2.0),
            ],
            ..Mesh::default()
        };
        let mut image = RgbaImage::from_pixel(2, 1, Rgba([200, 180, 120, 255]));
        image.put_pixel(0, 0, Rgba([200, 180, 120, 0]));
        let mut scene = Scene {
            mesh: &mesh,
            tris: vec![[0, 1, 2], [0, 2, 3]],
            transparent: vec![false; 2],
            nodes: Vec::new(),
            images: Images {
                loaded: vec![Some(image)],
                _source: std::marker::PhantomData,
            },
        };
        scene.build();
        let down = Vec3::new(0.0, 0.0, -1.0);
        assert!(
            scene
                .intersect(Vec3::new(0.5, 1.0, 5.0), down, 100.0)
                .is_none()
        );
        assert!(
            scene
                .intersect(Vec3::new(1.5, 1.0, 5.0), down, 100.0)
                .is_some()
        );
        // Light too: sun through the holes, shade under the strands.
        assert!((scene.transmittance(Vec3::new(0.5, 1.0, -1.0), -down, 10.0) - 1.0).abs() < 1e-6);
        assert!(scene.transmittance(Vec3::new(1.5, 1.0, -1.0), -down, 10.0) < 1e-6);
    }

    #[test]
    fn a_photo_uses_the_threads_it_is_given_and_never_more_than_the_budget() {
        let budget = thread_budget();
        assert!((1..=4).contains(&budget));
        assert_eq!(with_thread_cap(1, render_threads), 1);
        assert_eq!(with_thread_cap(64, render_threads), budget);
        assert_eq!(render_threads(), budget, "the cap ends with its call");
    }
}
