//! Photometry: how much light fixtures give and where it lands.
//!
//! A [`Light`] states its flux (lumens, or watts × the lamp's efficacy),
//! color temperature and distribution: a bare point, a spot with a beam
//! angle or a flat Lambertian panel. From that, [`Emitter::intensity`] gives
//! candelas in any direction and [`illuminance`] adds up lux on a surface by
//! the inverse-square cosine law, with walls in between casting shadows.
//! [`room_lighting`] reports a room at the work plane the way a lighting
//! designer checks it: average, minimum and uniformity against the usual
//! reference for the room, plus interreflected light by the split-flux method.

use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

use crate::{Furniture, FurnitureId, Home, LevelId, Light, Point2, Room, RoomId};

/// Lamp technology, for flux from electrical power.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum LampType {
    #[default]
    Led,
    Fluorescent,
    Halogen,
    Incandescent,
}

impl LampType {
    /// Typical luminous efficacy, lm/W.
    pub fn efficacy(self) -> f64 {
        match self {
            Self::Led => 100.0,
            Self::Fluorescent => 65.0,
            Self::Halogen => 16.0,
            Self::Incandescent => 12.0,
        }
    }
}

/// Flux a light gives when nothing more specific is known: Sweet Home 3D's
/// power 0.5 is a common 800 lm bulb.
const FLUX_AT_FULL_POWER: f64 = 1600.0;

impl Light {
    /// Luminous flux, lm.
    pub fn flux(&self) -> f64 {
        self.lumens
            .or_else(|| {
                self.watts
                    .map(|w| w * self.lamp.unwrap_or_default().efficacy())
            })
            .unwrap_or(self.power * FLUX_AT_FULL_POWER)
            .max(0.0)
    }

    /// Electrical power, W.
    pub fn electrical_watts(&self) -> f64 {
        self.watts
            .unwrap_or_else(|| self.flux() / self.lamp.unwrap_or_default().efficacy())
    }

    /// An LED fixture: `lumens` at `kelvin`, one source at `(x, y, z)`
    /// fractions of the piece.
    pub fn led(lumens: f64, kelvin: f64, source: (f64, f64, f64)) -> Self {
        Self {
            power: (lumens / FLUX_AT_FULL_POWER).min(1.0),
            sources: vec![crate::LightSource {
                x: source.0,
                y: source.1,
                z: source.2,
                color: kelvin_rgb8(kelvin),
                diameter: None,
            }],
            source_materials: Vec::new(),
            lumens: Some(lumens),
            watts: None,
            lamp: Some(LampType::Led),
            kelvin: Some(kelvin),
            beam: None,
            area: None,
            panel_upward: None,
        }
    }
}

/// Color of a black body at `kelvin` (1000–40000 K), linear RGB scaled so its
/// luminance is 1: flux stays the same whatever the color.
pub fn kelvin_rgb(kelvin: f64) -> [f64; 3] {
    // Tanner Helland's fit of the CIE black-body locus, in sRGB.
    let t = kelvin.clamp(1000.0, 40_000.0) / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2)
    };
    let g = if t <= 66.0 {
        99.470_802_586_1 * t.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
    };
    let linear = [r, g, b].map(|c| (c.clamp(0.0, 255.0) / 255.0).powf(2.2));
    let lum = luminance(linear).max(1e-6);
    linear.map(|c| c / lum)
}

fn kelvin_rgb8(kelvin: f64) -> [u8; 3] {
    let c = kelvin_rgb(kelvin);
    let max = c.iter().copied().fold(1e-6, f64::max);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    c.map(|v| ((v / max).powf(1.0 / 2.2) * 255.0).round() as u8)
}

fn luminance(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// How an emitter spreads its light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Distribution {
    /// The same in every direction.
    Point,
    /// Straight down, halving at `half` degrees off the axis.
    Spot { half: f64 },
    /// A horizontal Lambertian panel, `w` × `d` cm, facing down unless upward.
    Area {
        w: f64,
        d: f64,
        angle: f64,
        upward: bool,
    },
}

/// One source of light in the home, ready for photometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Emitter {
    /// Top-level owner used to associate reports with the selectable assembly.
    pub piece: FurnitureId,
    /// Visible leaf fixture that physically emits this light.
    pub source: FurnitureId,
    pub level: Option<LevelId>,
    /// Plan position and height above the ground floor, cm.
    pub position: [f64; 3],
    /// Luminous flux, lm.
    pub flux: f64,
    /// Linear color with luminance 1.
    pub color: [f64; 3],
    pub distribution: Distribution,
    /// Electrical power, W.
    pub watts: f64,
}

/// Relative intensity of a spot `theta` radians off its axis.
fn spot_profile(theta: f64, half: f64) -> f64 {
    if theta >= PI / 2.0 {
        return 0.0;
    }
    (-std::f64::consts::LN_2 * (theta / half.to_radians().max(1e-3)).powi(2)).exp()
}

/// Flux of a spot with a peak intensity of 1 cd.
fn spot_flux_per_candela(half: f64) -> f64 {
    let steps = 512;
    let dt = PI / 2.0 / f64::from(steps);
    (0..steps)
        .map(|i| {
            let t = (f64::from(i) + 0.5) * dt;
            spot_profile(t, half) * t.sin() * dt
        })
        .sum::<f64>()
        * 2.0
        * PI
}

impl Emitter {
    /// Candelas toward the unit direction `dir` (x, y plan, z up).
    pub fn intensity(&self, dir: [f64; 3]) -> f64 {
        match self.distribution {
            Distribution::Point => self.flux / (4.0 * PI),
            Distribution::Spot { half } => {
                let theta = (-dir[2]).clamp(-1.0, 1.0).acos();
                self.flux / spot_flux_per_candela(half) * spot_profile(theta, half)
            }
            Distribution::Area { upward, .. } => {
                (self.flux / PI) * (if upward { dir[2] } else { -dir[2] }).max(0.0)
            }
        }
    }

    /// Peak intensity, cd, along the emitting axis.
    pub fn peak(&self) -> f64 {
        self.intensity([
            0.0,
            0.0,
            if matches!(self.distribution, Distribution::Area { upward: true, .. }) {
                1.0
            } else {
                -1.0
            },
        ])
    }

    /// Luminance of its emitting surface, cd/m², for panels.
    pub fn panel_luminance(&self) -> Option<f64> {
        match self.distribution {
            Distribution::Area { w, d, .. } => {
                Some(self.flux / (PI * (w * d / 10_000.0).max(1e-6)))
            }
            _ => None,
        }
    }
}

/// Every emitting piece of the home. `preset` gives the light of pieces that
/// have none stored (catalog fixtures placed before they had one).
pub fn emitters(home: &Home, preset: &dyn Fn(&Furniture) -> Option<Light>) -> Vec<Emitter> {
    let mut out = Vec::new();
    for top in &home.furniture {
        for piece in top.visible_leaves() {
            let Some(light) = piece.light.clone().or_else(|| preset(piece)) else {
                continue;
            };
            let flux = light.flux();
            if flux <= 0.0 {
                continue;
            }
            let floor = home.elevation_of(piece.level.or(top.level));
            let directional = light.beam.is_some() || light.area.is_some();
            let default_source = [crate::LightSource {
                x: 0.5,
                y: 0.5,
                z: if light.panel_upward == Some(true) {
                    1.0
                } else if directional {
                    0.0
                } else {
                    0.5
                },
                color: [255, 255, 255],
                diameter: None,
            }];
            let sources = if light.sources.is_empty() {
                &default_source[..]
            } else {
                &light.sources[..]
            };
            #[allow(clippy::cast_precision_loss)]
            let share = sources.len() as f64;
            for source in sources {
                let at = piece.to_plan((
                    (source.x - 0.5) * piece.width,
                    (source.y - 0.5) * piece.depth,
                ));
                let z = floor + piece.elevation + source.z.clamp(0.0, 1.0) * piece.height;
                let color = light.kelvin.map_or_else(
                    || {
                        let c = source.color.map(|v| (f64::from(v) / 255.0).powf(2.2));
                        let lum = luminance(c);
                        if lum < 1e-4 {
                            [1.0; 3]
                        } else {
                            c.map(|v| v / lum)
                        }
                    },
                    kelvin_rgb,
                );
                let distribution = match (light.area, light.beam) {
                    (Some([w, d]), _) => Distribution::Area {
                        w,
                        d,
                        angle: piece.angle,
                        upward: light.panel_upward.unwrap_or(false),
                    },
                    (None, Some(beam)) => Distribution::Spot {
                        half: (beam / 2.0).clamp(1.0, 89.0),
                    },
                    (None, None) => Distribution::Point,
                };
                out.push(Emitter {
                    piece: top.id,
                    source: piece.id,
                    level: home.resolve_level(piece.level.or(top.level)),
                    position: [at.x, at.y, z],
                    flux: flux / share,
                    color,
                    distribution,
                    watts: light.electrical_watts() / share,
                });
            }
        }
    }
    out
}

/// Whether a wall of the storey stands between `a` and `b` (cm, z above the
/// ground floor).
fn blocked(home: &Home, level: Option<LevelId>, a: [f64; 3], b: [f64; 3]) -> bool {
    let floor = home.elevation_of(level);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    home.walls
        .iter()
        .filter(|w| home.resolve_level(w.level) == level && !w.is_arc())
        .any(|w| {
            let (ex, ey) = (w.end.x - w.start.x, w.end.y - w.start.y);
            let den = dx * ey - dy * ex;
            if den.abs() < 1e-9 {
                return false;
            }
            let (sx, sy) = (w.start.x - a[0], w.start.y - a[1]);
            // Along the ray (s) and along the wall (t).
            let s = (sx * ey - sy * ex) / den;
            let t = (sx * dy - sy * dx) / den;
            let ray = dx.hypot(dy).max(1e-6);
            // Pieces mounted on the wall itself don't shadow themselves.
            let margin = (w.thickness / 2.0 + 1.0) / ray;
            if !(margin..=1.0 - margin).contains(&s) || !(0.0..=1.0).contains(&t) {
                return false;
            }
            let z = a[2] + (b[2] - a[2]) * s - floor;
            let top = w.height + (w.height_at_end.unwrap_or(w.height) - w.height) * t;
            z < top
        })
}

/// Direct illuminance at `p` (cm) on a surface facing `normal`, lux.
pub fn illuminance(home: &Home, emitters: &[Emitter], p: [f64; 3], normal: [f64; 3]) -> f64 {
    let mut total = 0.0;
    for e in emitters {
        // Panels count as a few points so nearby surfaces see their size.
        let points: Vec<([f64; 3], f64)> = match e.distribution {
            Distribution::Area { w, d, angle, .. } => {
                let (sin, cos) = angle.to_radians().sin_cos();
                let n = 3;
                let mut v = Vec::with_capacity(9);
                for i in 0..n {
                    for j in 0..n {
                        let u = (f64::from(i) + 0.5) / f64::from(n) - 0.5;
                        let k = (f64::from(j) + 0.5) / f64::from(n) - 0.5;
                        let (lx, ly) = (u * w, k * d);
                        v.push((
                            [
                                e.position[0] + lx * cos - ly * sin,
                                e.position[1] + lx * sin + ly * cos,
                                e.position[2],
                            ],
                            1.0 / 9.0,
                        ));
                    }
                }
                v
            }
            _ => vec![(e.position, 1.0)],
        };
        for (at, share) in points {
            let to = [at[0] - p[0], at[1] - p[1], at[2] - p[2]];
            let d2 = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).max(1.0);
            let d = d2.sqrt();
            let l = [to[0] / d, to[1] / d, to[2] / d];
            let cos = l[0] * normal[0] + l[1] * normal[1] + l[2] * normal[2];
            if cos <= 0.0 {
                continue;
            }
            let from_light = [-l[0], -l[1], -l[2]];
            let candela = e.intensity(from_light) * share;
            if candela <= 0.0 || blocked(home, e.level, at, p) {
                continue;
            }
            // cd / m² → lux; distances are in cm.
            total += candela * cos / (d2 / 10_000.0);
        }
    }
    total
}

fn inside(points: &[Point2], p: Point2) -> bool {
    let mut odd = false;
    let mut j = points.len().wrapping_sub(1);
    for (i, a) in points.iter().enumerate() {
        let b = points[j];
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            odd = !odd;
        }
        j = i;
    }
    odd
}

/// Recommended average illuminance on the work plane for a room, lux, and
/// what it's for. NBR ISO/CIE 8995-1 covers workplaces only; homes take the
/// residential table of the NBR 5413:1992 it replaced (middle values), and
/// 8995-1 where 5413 has none (office, laundry, dining).
pub fn recommended_lux(name: &str) -> (f64, &'static str) {
    let reference = lux_for(crate::vocabulary::mentions(name));
    (reference.lux, reference.what)
}

/// The illuminance a room should reach, what for, and the standard that
/// says so — each value cites its own table, not both of them at once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LuxReference {
    pub lux: f64,
    pub what: &'static str,
    /// Registry code of the table the value comes from; `None` where no
    /// standard gives one and the value is practice.
    pub source: Option<&'static str>,
}

/// The same, for what a room is rather than what it is called.
#[must_use]
pub fn lux_for(says: crate::vocabulary::Mentions) -> LuxReference {
    use crate::vocabulary::Mention as M;
    let (lux, what, source) = if says.any(&[M::Office, M::Study]) {
        (500.0, "leitura e trabalho", Some("nbr8995"))
    } else if says.any(&[M::Kitchen, M::Gourmet]) {
        (
            150.0,
            "cozinha (bancada, fogão e pia: 300 lx localizado)",
            Some("nbr5413"),
        )
    } else if says.has(M::Laundry) {
        (
            300.0,
            "lavanderia (valor de lavanderia da 8995-1)",
            Some("nbr8995"),
        )
    } else if says.any(&[M::Bathroom, M::Lavatory]) {
        (
            150.0,
            "banheiro (espelho: 300 lx localizado)",
            Some("nbr5413"),
        )
    } else if says.has(M::Dining) {
        (
            200.0,
            "jantar (valor de refeitório da 8995-1)",
            Some("nbr8995"),
        )
    } else if says.has(M::Bedroom) {
        (
            150.0,
            "dormitório (espelho, penteadeira e cama: 300 lx localizado)",
            Some("nbr5413"),
        )
    } else if says.any(&[M::Living, M::Tv]) {
        (150.0, "estar", Some("nbr5413"))
    } else if says.any(&[M::Corridor, M::Stairs, M::Entrance]) {
        (100.0, "circulação", Some("nbr5413"))
    } else if says.has(M::Garage) {
        (100.0, "garagem", Some("nbr5413"))
    } else if says.any(&[M::Balcony, M::Outdoor]) {
        (30.0, "área externa (prática; sem norma)", None)
    } else {
        (150.0, "uso geral residencial", Some("nbr5413"))
    };
    LuxReference { lux, what, source }
}

/// Surface reflectances used for interreflected light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reflectance {
    pub ceiling: f64,
    pub walls: f64,
    pub floor: f64,
}

impl Default for Reflectance {
    fn default() -> Self {
        Self {
            ceiling: 0.8,
            walls: 0.6,
            floor: 0.3,
        }
    }
}

/// A room's lighting at the work plane.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoomLighting {
    pub room: RoomId,
    pub name: String,
    pub area_m2: f64,
    /// Average, minimum and maximum lux (direct plus interreflected).
    pub average: f64,
    pub min: f64,
    pub max: f64,
    /// Minimum over average.
    pub uniformity: f64,
    /// Interreflected part of the average, lux.
    pub indirect: f64,
    pub target: f64,
    pub target_use: &'static str,
    /// The table `target` comes from, if any — see [`LuxReference`].
    pub target_source: Option<&'static str>,
    /// Emitters inside the room.
    pub fixtures: usize,
    pub lumens: f64,
    pub watts: f64,
    pub watts_per_m2: f64,
    pub points: usize,
}

/// Band along the walls left out of a room's grid, cm.
const BORDER: f64 = 30.0;

/// Lighting of `room` at `plane` cm above its floor, over a grid kept
/// [`BORDER`] off its walls (small rooms use all of it).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn room_lighting(
    home: &Home,
    emitters: &[Emitter],
    room: &Room,
    plane: f64,
    reflectance: Reflectance,
) -> RoomLighting {
    let level = home.resolve_level(room.level);
    let floor = home.elevation_of(room.level);
    let height = level
        .and_then(|id| home.level(id))
        .map_or(home.wall_height, |l| l.height)
        .max(1.0);
    let area = crate::polygon_area(&room.points);
    let perimeter: f64 = room
        .points
        .iter()
        .zip(room.points.iter().cycle().skip(1))
        .map(|(a, b)| a.distance(*b))
        .sum();
    let own: Vec<&Emitter> = emitters
        .iter()
        .filter(|e| {
            e.level == level && inside(&room.points, Point2::new(e.position[0], e.position[1]))
        })
        .collect();
    let same_level: Vec<Emitter> = emitters
        .iter()
        .filter(|e| e.level == level)
        .cloned()
        .collect();
    // Grid over the bounding box, kept inside the outline.
    let (lo, hi) = room.points.iter().fold(
        (
            Point2::new(f64::MAX, f64::MAX),
            Point2::new(f64::MIN, f64::MIN),
        ),
        |(lo, hi), p| {
            (
                Point2::new(lo.x.min(p.x), lo.y.min(p.y)),
                Point2::new(hi.x.max(p.x), hi.y.max(p.y)),
            )
        },
    );
    let step = (area.sqrt() / 14.0).clamp(20.0, 60.0);
    let mut values = Vec::new();
    let nx = ((hi.x - lo.x) / step).ceil().max(1.0) as usize;
    let ny = ((hi.y - lo.y) / step).ceil().max(1.0) as usize;
    for i in 0..nx {
        for j in 0..ny {
            let p = Point2::new(
                lo.x + (hi.x - lo.x) * (i as f64 + 0.5) / nx as f64,
                lo.y + (hi.y - lo.y) * (j as f64 + 0.5) / ny as f64,
            );
            // Like task areas in lighting standards, leave out a band along the walls.
            let edge = edge_distance(&room.points, p);
            if inside(&room.points, p) && (edge >= BORDER || area < 40_000.0) {
                values.push(illuminance(
                    home,
                    &same_level,
                    [p.x, p.y, floor + plane],
                    [0.0, 0.0, 1.0],
                ));
            }
        }
    }
    let lumens: f64 = own.iter().map(|e| e.flux).sum();
    let watts: f64 = own.iter().map(|e| e.watts).sum();
    // Split-flux interreflection: light bouncing around the room's surfaces.
    let (a_m2, wall_m2) = (area / 10_000.0, perimeter * height / 10_000.0);
    let surfaces = 2.0 * a_m2 + wall_m2;
    let rho = if surfaces > 0.0 {
        (a_m2 * (reflectance.ceiling + reflectance.floor) + wall_m2 * reflectance.walls) / surfaces
    } else {
        0.5
    };
    let indirect = if surfaces > 0.0 {
        lumens * rho / (surfaces * (1.0 - rho).max(0.05))
    } else {
        0.0
    };
    let count = values.len().max(1) as f64;
    let direct_avg = values.iter().sum::<f64>() / count;
    let min = values.iter().copied().fold(f64::MAX, f64::min).min(1e12);
    let max = values.iter().copied().fold(0.0, f64::max);
    let average = direct_avg + indirect;
    let reference = lux_for(room.mentions());
    RoomLighting {
        room: room.id,
        name: room.name.clone(),
        area_m2: a_m2,
        average,
        min: if values.is_empty() {
            0.0
        } else {
            min + indirect
        },
        max: max + indirect,
        uniformity: if average > 0.0 && !values.is_empty() {
            (min + indirect) / average
        } else {
            0.0
        },
        indirect,
        target: reference.lux,
        target_use: reference.what,
        target_source: reference.source,
        fixtures: own.len(),
        lumens,
        watts,
        watts_per_m2: if a_m2 > 0.0 { watts / a_m2 } else { 0.0 },
        points: values.len(),
    }
}

/// Fixtures of `fixture_lm` needed for `target` lux over `area_m2` by the
/// lumen method, with a utilization factor of 0.6 and maintenance of 0.8.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn fixtures_needed(target: f64, area_m2: f64, fixture_lm: f64) -> usize {
    if fixture_lm <= 0.0 {
        return 0;
    }
    ((target * area_m2 / (fixture_lm * 0.6 * 0.8))
        .ceil()
        .max(1.0)) as usize
}

/// `count` evenly spaced points covering a room (a grid matching its
/// proportions, spacing s with s/2 to the edges), kept inside the outline
/// and at least `clear` cm off every edge of it — half a fixture's diagonal
/// keeps the whole fixture in the room, even at the inner corner of an L.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn grid_positions(points: &[Point2], count: usize, clear: f64) -> Vec<Point2> {
    if points.len() < 3 || count == 0 {
        return Vec::new();
    }
    let (lo, hi) = points.iter().fold(
        (
            Point2::new(f64::MAX, f64::MAX),
            Point2::new(f64::MIN, f64::MIN),
        ),
        |(lo, hi), p| {
            (
                Point2::new(lo.x.min(p.x), lo.y.min(p.y)),
                Point2::new(hi.x.max(p.x), hi.y.max(p.y)),
            )
        },
    );
    let (w, d) = ((hi.x - lo.x).max(1.0), (hi.y - lo.y).max(1.0));
    // Grow the grid until enough of it falls inside (L-shaped rooms).
    let mut want = count;
    let mut best: Vec<Point2> = Vec::new();
    for _ in 0..8 {
        let cols = ((want as f64 * w / d).sqrt().round() as usize).max(1);
        let rows = want.div_ceil(cols).max(1);
        let grid: Vec<Point2> = (0..rows)
            .flat_map(|r| {
                (0..cols).map(move |c| {
                    Point2::new(
                        lo.x + w * (c as f64 + 0.5) / cols as f64,
                        lo.y + d * (r as f64 + 0.5) / rows as f64,
                    )
                })
            })
            .filter(|p| inside(points, *p) && edge_distance(points, *p) >= clear)
            .collect();
        if grid.len() >= count {
            return grid;
        }
        want += count - grid.len();
        if grid.len() > best.len() {
            best = grid;
        }
    }
    // Fewer than asked, where they fit; or the middle, if even that keeps
    // the fixture in; else nowhere does.
    if !best.is_empty() {
        return best;
    }
    crate::polygon_centroid(points)
        .filter(|c| inside(points, *c) && edge_distance(points, *c) >= clear)
        .into_iter()
        .collect()
}

/// Distance from `p` to the nearest edge of the outline, cm.
fn edge_distance(points: &[Point2], p: Point2) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| p.distance_to_segment(*a, *b))
        .fold(f64::MAX, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Room, RoomId, Wall, WallId};

    fn piece(id: u64, at: (f64, f64), elevation: f64, light: Light) -> Furniture {
        Furniture {
            id: FurnitureId(id),
            catalog: "downlight".into(),
            position: Point2::new(at.0, at.1),
            elevation,
            width: 10.0,
            depth: 10.0,
            height: 5.0,
            light: Some(light),
            ..Furniture::default()
        }
    }

    #[test]
    fn a_bare_bulb_follows_the_inverse_square_law() {
        let mut home = Home::default();
        // 1257 lm in all directions is 100 cd.
        let mut light = Light::led(400.0 * PI, 3000.0, (0.5, 0.5, 0.5));
        light.watts = None;
        home.furniture.push(piece(1, (0.0, 0.0), 297.5, light));
        let e = emitters(&home, &|_| None);
        assert_eq!(e.len(), 1);
        assert!((e[0].peak() - 100.0).abs() < 1e-6);
        // 2 m straight below: 100 / 2² = 25 lx; 1 m: 100 lx.
        let at = |z: f64| illuminance(&home, &e, [0.0, 0.0, z], [0.0, 0.0, 1.0]);
        assert!((at(100.0) - 25.0).abs() < 1e-6, "{}", at(100.0));
        assert!((at(200.0) - 100.0).abs() < 1e-6);
        // 45° off: cos³ falloff on a horizontal plane.
        let off = illuminance(&home, &e, [200.0, 0.0, 100.0], [0.0, 0.0, 1.0]);
        assert!(
            (off - 25.0 * 45f64.to_radians().cos().powi(3)).abs() < 1e-6,
            "{off}"
        );
        // Facing away: nothing.
        assert!(illuminance(&home, &e, [0.0, 0.0, 100.0], [0.0, 0.0, -1.0]).abs() < 1e-12);
    }

    #[test]
    fn upward_panels_light_the_ceiling_and_preserve_flux() {
        let mut light = Light::led(1000.0, 3000.0, (0.5, 0.5, 1.0));
        light.area = Some([100.0, 1.0]);
        light.panel_upward = Some(true);
        let json = serde_json::to_string(&light).unwrap();
        let restored: Light = serde_json::from_str(&json).unwrap();
        let mut home = Home::default();
        home.furniture.push(piece(1, (0.0, 0.0), 250.0, restored));
        let lights = emitters(&home, &|_| None);
        let e = &lights[0];
        assert!((e.intensity([0.0, 0.0, 1.0]) - 1000.0 / PI).abs() < 1e-6);
        assert!(e.intensity([0.0, 0.0, -1.0]).abs() < 1e-6);
        assert!((e.peak() - 1000.0 / PI).abs() < 1e-6);
        let z = e.position[2];
        assert!(illuminance(&home, &lights, [0.0, 0.0, z + 50.0], [0.0, 0.0, -1.0]) > 0.0);
        assert!(illuminance(&home, &lights, [0.0, 0.0, z - 50.0], [0.0, 0.0, 1.0]).abs() < 1e-6);
        let old = json.replace(",\"panel_upward\":true", "");
        assert_eq!(
            serde_json::from_str::<Light>(&old).unwrap().panel_upward,
            None
        );
    }

    #[test]
    fn spots_keep_their_flux_and_halve_at_the_beam_edge() {
        let mut light = Light::led(600.0, 3000.0, (0.5, 0.5, 0.0));
        light.beam = Some(60.0);
        let mut home = Home::default();
        home.furniture.push(piece(1, (0.0, 0.0), 250.0, light));
        let e = &emitters(&home, &|_| None)[0];
        // Integrated back over the sphere, the intensity gives the flux.
        let steps = 400;
        let dt = PI / f64::from(steps);
        let flux: f64 = (0..steps)
            .map(|i| {
                let t = (f64::from(i) + 0.5) * dt;
                e.intensity([t.sin(), 0.0, -t.cos()]) * t.sin() * dt * 2.0 * PI
            })
            .sum();
        assert!((flux - 600.0).abs() < 3.0, "{flux}");
        let edge = e.intensity([30f64.to_radians().sin(), 0.0, -30f64.to_radians().cos()]);
        assert!((edge / e.peak() - 0.5).abs() < 1e-6);
        // Much more light under a spot than under a bulb of the same flux.
        assert!(e.peak() > 600.0 / (4.0 * PI) * 4.0);
        // Watts from lumens by efficacy.
        assert!((e.watts - 6.0).abs() < 1e-9);
    }

    #[test]
    fn a_fill_keeps_every_fixture_inside_an_l_shaped_room() {
        // An L whose inner edge (x = 228) runs 3 cm from a column of the grid.
        let l = [
            (0.0, 0.0),
            (600.0, 0.0),
            (600.0, 50.0),
            (228.0, 50.0),
            (228.0, 400.0),
            (0.0, 400.0),
        ]
        .map(|(x, y)| Point2::new(x, y));
        let hugging = |spots: &[Point2]| spots.iter().any(|p| edge_distance(&l, *p) < 10.0);
        assert!(
            hugging(&grid_positions(&l, 4, 0.0)),
            "the case this is about"
        );
        for count in 1..=12 {
            let spots = grid_positions(&l, count, 30.0);
            assert_ne!(spots, [] as [crate::geometry::Point2; 0]);
            for p in &spots {
                assert!(inside(&l, *p), "{count}: {p:?} outside");
                assert!(edge_distance(&l, *p) >= 30.0, "{count}: {p:?} on the edge");
            }
        }
        // A U whose middle is the courtyard: the spots that fit are kept even
        // when there are fewer than asked and the middle fits nothing.
        let u = [
            (0.0, 0.0),
            (600.0, 0.0),
            (600.0, 400.0),
            (420.0, 400.0),
            (420.0, 80.0),
            (180.0, 80.0),
            (180.0, 400.0),
            (0.0, 400.0),
        ]
        .map(|(x, y)| Point2::new(x, y));
        let middle = crate::polygon_centroid(&u).unwrap();
        assert!(!inside(&u, middle) || edge_distance(&u, middle) < 60.0);
        let spots = grid_positions(&u, 40, 60.0);
        assert!(!spots.is_empty() && spots.len() < 40, "{spots:?}");
        assert!(
            spots
                .iter()
                .all(|p| inside(&u, *p) && edge_distance(&u, *p) >= 60.0)
        );
        // A room too small for the fixture has no spot for it, not a wrong one.
        let closet =
            [(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)].map(|(x, y)| Point2::new(x, y));
        assert_eq!(
            grid_positions(&closet, 1, 30.0),
            [] as [crate::geometry::Point2; 0]
        );
        assert_eq!(grid_positions(&closet, 1, 10.0).len(), 1);
    }

    #[test]
    fn walls_cast_shadows_and_rooms_are_rated() {
        let mut home = Home::default();
        home.wall_height = 260.0;
        let mut wall = Wall::new(
            WallId(1),
            Point2::new(400.0, 0.0),
            Point2::new(400.0, 400.0),
        );
        wall.height = 260.0;
        wall.thickness = 15.0;
        home.walls.push(wall);
        let mut panel = Light::led(3600.0, 4000.0, (0.5, 0.5, 0.0));
        panel.area = Some([60.0, 60.0]);
        home.furniture.push(piece(1, (200.0, 200.0), 255.0, panel));
        let e = emitters(&home, &|_| None);
        let here = illuminance(&home, &e, [300.0, 200.0, 75.0], [0.0, 0.0, 1.0]);
        let behind = illuminance(&home, &e, [500.0, 200.0, 75.0], [0.0, 0.0, 1.0]);
        assert!(here > 100.0 && behind.abs() < 1e-9, "{here} {behind}");
        let room = Room::new(
            RoomId(2),
            "Cozinha",
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(400.0, 0.0),
                Point2::new(400.0, 400.0),
                Point2::new(0.0, 400.0),
            ],
        );
        let report = room_lighting(&home, &e, &room, 75.0, Reflectance::default());
        // NBR 5413 residential kitchen: 150 lx general (300 at the counter).
        assert!((report.target - 150.0).abs() < 1e-9);
        assert_eq!(report.fixtures, 1);
        assert!(report.average > report.min && report.min > 0.0);
        assert!(report.uniformity > 0.0 && report.uniformity < 1.0);
        assert!((report.watts_per_m2 - 36.0 / 16.0).abs() < 1e-9);
        // The lumen method asks for more panels to reach 300 lx over 16 m².
        let n = fixtures_needed(300.0, report.area_m2, 3600.0);
        assert_eq!(n, 3);
        let spots = grid_positions(&room.points, 4, 0.0);
        assert_eq!(spots.len(), 4);
        assert!(
            spots
                .iter()
                .any(|p| (p.x - 100.0).abs() < 1e-9 && (p.y - 100.0).abs() < 1e-9)
        );
    }

    #[test]
    fn warm_light_is_orange_and_cool_light_is_blue() {
        let warm = kelvin_rgb(2700.0);
        let cool = kelvin_rgb(6500.0);
        assert!(warm[0] > warm[2] && cool[2] > warm[2]);
        assert!((luminance(warm) - 1.0).abs() < 1e-9);
    }
}
