//! Procedural 3D models. Every generator works in the piece's real size and
//! the result is snapped to exactly `width × depth × height`.

use newera_core::{Furniture, OpeningKind};

use crate::Model;
use crate::finish::{Paint, PaintedMesh, rgb, shade};
use crate::mesh::{Axis, Mesh, Rgb};

const METAL: [u8; 3] = [150, 154, 160];
use crate::mesh::GLASS;
const DARK: [u8; 3] = [40, 42, 48];
const LINEN: [u8; 3] = [245, 243, 236];

struct Ctx {
    m: PaintedMesh,
    w: f64,
    d: f64,
    h: f64,
    c: Paint,
    explicit_color: Option<Paint>,
}

impl Ctx {
    /// Keep the model's original stock palette unless the user sets a color.
    fn body_or_default(&self, color: [u8; 3]) -> Paint {
        self.explicit_color
            .unwrap_or_else(|| Paint::body(crate::mesh::rgb(color)))
    }

    /// Box by explicit bounds (cm) in the local frame.
    fn cube(&mut self, x: [f64; 2], y: [f64; 2], z: [f64; 2], color: Paint) {
        self.m.cuboid([x[0], y[0], z[0]], [x[1], y[1], z[1]], color);
    }

    fn legs(&mut self, inset: f64, top: f64, size: f64, color: Paint) {
        let (hw, hd) = (
            self.w / 2.0 - inset - size / 2.0,
            self.d / 2.0 - inset - size / 2.0,
        );
        for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            self.m
                .block([sx * hw, sz * hd], 0.0, [size, top, size], color);
        }
    }

    /// Straight leg (or post) from the floor, tapering from `size[0]` at the
    /// bottom to `size[1]` at `top`.
    fn leg(&mut self, x: f64, z: f64, top: f64, size: [f64; 2], color: Paint) {
        let square = |s: f64| [[x - s / 2.0, x + s / 2.0], [z - s / 2.0, z + s / 2.0]];
        prism(
            &mut self.m,
            Place::HOME,
            [0.0, top],
            square(size[0]),
            square(size[1]),
            color,
        );
    }

    fn handle(&mut self, x: f64, y: f64, z: f64, vertical: bool) {
        let (w, h) = if vertical { (1.5, 14.0) } else { (12.0, 1.5) };
        self.cube(
            [x - w / 2.0, x + w / 2.0],
            [y - h / 2.0, y + h / 2.0],
            [z, z + 2.0],
            rgb(METAL),
        );
    }
}

/// Horizontal rectangle `[x0, x1], [z0, z1]` in a piece's local frame.
type Span = [[f64; 2]; 2];

/// Where a sub-part (a chair of a dining set) sits: its center on the floor,
/// and whether it is turned half around so its front faces −z.
#[derive(Clone, Copy)]
struct Place {
    x: f64,
    z: f64,
    turned: bool,
}

impl Place {
    const HOME: Self = Self {
        x: 0.0,
        z: 0.0,
        turned: false,
    };

    #[allow(clippy::cast_possible_truncation)]
    fn at(self, p: [f64; 3]) -> [f32; 3] {
        let s = if self.turned { -1.0 } else { 1.0 };
        [
            (self.x + s * p[0]) as f32,
            p[1] as f32,
            (self.z + s * p[2]) as f32,
        ]
    }
}

/// Box whose bottom face is `lo` and top face is `hi`, from `y[0]` to
/// `y[1]`. Different spans taper (legs) or shear it (leaning backrests,
/// stringers); every face stays planar since its edges run along x or z.
fn prism(m: &mut PaintedMesh, place: Place, y: [f64; 2], lo: Span, hi: Span, color: Paint) {
    let ring = |s: Span, y: f64| {
        [
            [s[0][0], y, s[1][0]],
            [s[0][1], y, s[1][0]],
            [s[0][1], y, s[1][1]],
            [s[0][0], y, s[1][1]],
        ]
        .map(|p| place.at(p))
    };
    let (b, t) = (ring(lo, y[0]), ring(hi, y[1]));
    m.polygon(&[b[3], b[2], t[2], t[3]], color); // front +z
    m.polygon(&[b[1], b[0], t[0], t[1]], color); // back -z
    m.polygon(&[b[2], b[1], t[1], t[2]], color); // right +x
    m.polygon(&[b[0], b[3], t[3], t[0]], color); // left -x
    m.polygon(&[t[3], t[2], t[1], t[0]], shade(color, 0.06)); // top
    m.polygon(&[b[0], b[1], b[2], b[3]], shade(color, -0.2)); // bottom
}

/// Upright box in a [`Place`]'s frame.
fn part(m: &mut PaintedMesh, place: Place, x: [f64; 2], y: [f64; 2], z: [f64; 2], color: Paint) {
    prism(m, place, y, [x, z], [x, z], color);
}

pub(crate) fn build(model: Model, piece: &Furniture, color: Rgb) -> Mesh {
    let mut ctx = Ctx {
        m: PaintedMesh::default(),
        w: piece.width,
        d: piece.depth,
        h: piece.height,
        c: Paint::body(color),
        explicit_color: piece.color.map(|_| Paint::body(color)),
    };
    let opening = piece.opening.as_ref();
    match model {
        Model::Sofa { seats } => sofa(&mut ctx, seats),
        Model::Bed { pillows } => bed(&mut ctx, pillows),
        Model::Table { round } => table(&mut ctx, round),
        Model::Chair => chair(&mut ctx),
        Model::Stool => stool(&mut ctx),
        Model::OfficeChair => office_chair(&mut ctx),
        Model::Cabinet { doors, drawers } => cabinet(&mut ctx, doors, drawers),
        Model::Shelf { levels } => shelf(&mut ctx, levels),
        Model::TvStand => tv_stand(&mut ctx),
        Model::Tv => tv(&mut ctx),
        Model::Fridge => fridge(&mut ctx),
        Model::Stove => stove(&mut ctx),
        Model::SinkCounter => counter(&mut ctx, true),
        Model::Counter => counter(&mut ctx, false),
        Model::WallCabinet => cabinet(&mut ctx, 1, 0),
        Model::Appliance { round_door } => appliance(&mut ctx, round_door),
        Model::Microwave => microwave(&mut ctx),
        Model::Hood => hood(&mut ctx),
        Model::Cooktop => cooktop(&mut ctx),
        Model::SinkBowl => sink_bowl(&mut ctx),
        Model::Oven => oven(&mut ctx),
        Model::Toilet => toilet(&mut ctx),
        Model::Basin => basin(&mut ctx),
        Model::Shower => shower(&mut ctx),
        Model::Bathtub => bathtub(&mut ctx),
        Model::Door => door(
            &mut ctx,
            opening.map_or(1, |o| o.leaves),
            opening.is_some_and(|o| o.sliding),
        ),
        Model::Window { panes } => window(
            &mut ctx,
            panes,
            opening.is_some_and(|o| o.kind == OpeningKind::Window),
        ),
        Model::Passage => passage(&mut ctx),
        Model::Stairs => stairs(&mut ctx, crate::stair_risers(piece.height)),
        Model::Plant => plant(&mut ctx),
        Model::Tree => tree(&mut ctx),
        Model::Lamp => lamp(&mut ctx),
        Model::Car => car(&mut ctx),
        Model::Desk => desk(&mut ctx),
        Model::Crib => crib(&mut ctx),
        Model::Column { round } => {
            let c = ctx.c;
            if round {
                ctx.m.frustum(
                    [0.0, 0.0, 0.0],
                    Axis::Y,
                    ctx.h,
                    ctx.w / 2.0,
                    ctx.w / 2.0,
                    28,
                    c,
                );
            } else {
                ctx.cube(
                    [-ctx.w / 2.0, ctx.w / 2.0],
                    [0.0, ctx.h],
                    [-ctx.d / 2.0, ctx.d / 2.0],
                    c,
                );
            }
        }
        Model::Footing => footing(&mut ctx),
        Model::Railing => railing(&mut ctx),
        Model::Fence => fence(&mut ctx),
        Model::Corrugated => corrugated(&mut ctx),
        Model::GlassPanel => glass_panel(&mut ctx),
        Model::GlassRailing => glass_railing(&mut ctx),
        Model::BalconyGlazing => balcony_glazing(&mut ctx),
        Model::GarageDoor => garage_door(&mut ctx),
        Model::Pool { oval: false } => pool(&mut ctx),
        Model::Pool { oval: true } => oval_pool(&mut ctx),
        Model::Lounger => lounger(&mut ctx),
        Model::Grill => grill(&mut ctx),
        Model::SofaL => sofa_l(&mut ctx),
        Model::Bench => bench(&mut ctx),
        Model::DiningSet { chairs } => dining_set(&mut ctx, chairs),
        Model::Planter => planter(&mut ctx),
        Model::OutletTower => outlet_tower(&mut ctx),
        Model::DeskBox => desk_box(&mut ctx),
        Model::Rug | Model::Box | Model::Point(_) => {
            let c = ctx.c;
            ctx.cube(
                [-ctx.w / 2.0, ctx.w / 2.0],
                [0.0, ctx.h],
                [-ctx.d / 2.0, ctx.d / 2.0],
                c,
            );
        }
    }
    ctx.m.fit(piece.width, piece.depth, piece.height)
}

fn sofa(ctx: &mut Ctx, seats: u8) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let arm = (w * 0.1).clamp(10.0, 22.0);
    let back = (d * 0.22).clamp(12.0, 25.0);
    let feet = (h * 0.11).min(10.0);
    let seat_top = h * 0.5;
    let arm_top = h * 0.68;
    // Slim tapered feet lift the whole body off the floor.
    let (fx, fz) = (w / 2.0 - 6.0, d / 2.0 - 6.0);
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        ctx.leg(sx * fx, sz * fz, feet, [3.2, 4.8], rgb(DARK));
    }
    ctx.cube(
        [-w / 2.0 + arm, w / 2.0 - arm],
        [feet, seat_top - 12.0],
        [-d / 2.0 + back, d / 2.0 - 1.0],
        shade(c, -0.15),
    );
    // Backrest and armrests, each with a narrower cap so the top edges read
    // as rounded upholstery rather than a sharp slab.
    let cap = 4.0;
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [feet, h - cap],
        [-d / 2.0, -d / 2.0 + back],
        c,
    );
    ctx.cube(
        [-w / 2.0 + 1.5, w / 2.0 - 1.5],
        [h - cap, h],
        [-d / 2.0 + 1.5, -d / 2.0 + back - 1.5],
        c,
    );
    for x0 in [-w / 2.0, w / 2.0 - arm] {
        ctx.cube(
            [x0, x0 + arm],
            [feet, arm_top - cap],
            [-d / 2.0, d / 2.0],
            c,
        );
        ctx.cube(
            [x0 + 1.5, x0 + arm - 1.5],
            [arm_top - cap, arm_top],
            [-d / 2.0 + back - 1.0, d / 2.0 - 1.5],
            c,
        );
    }
    // Separate seat cushions with a softened top layer, and back cushions
    // leaning slightly into the backrest.
    let n = f64::from(seats.max(1));
    let inner = w - 2.0 * arm;
    let slot = inner / n;
    for i in 0..seats.max(1) {
        let x0 = -w / 2.0 + arm + f64::from(i) * slot + 0.8;
        let x1 = x0 + slot - 1.6;
        let front = d / 2.0 - 2.0;
        ctx.cube(
            [x0, x1],
            [seat_top - 12.0, seat_top - 2.0],
            [-d / 2.0 + back, front],
            shade(c, 0.08),
        );
        ctx.cube(
            [x0 + 1.2, x1 - 1.2],
            [seat_top - 2.0, seat_top],
            [-d / 2.0 + back, front - 1.2],
            shade(c, 0.08),
        );
        let z = -d / 2.0 + back;
        prism(
            &mut ctx.m,
            Place::HOME,
            [seat_top, h * 0.9],
            [[x0, x1], [z, z + 14.0]],
            [[x0 + 1.0, x1 - 1.0], [z - 3.0, z + 9.0]],
            shade(c, 0.12),
        );
    }
}

fn bed(ctx: &mut Ctx, pillows: u8) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let head = (d * 0.04).clamp(4.0, 8.0);
    let frame_top = h * 0.45;
    let plinth = (frame_top * 0.35).min(10.0);
    // A recessed plinth leaves a shadow gap under the frame.
    ctx.cube(
        [-w / 2.0 + 8.0, w / 2.0 - 8.0],
        [0.0, plinth],
        [-d / 2.0 + head, d / 2.0 - 8.0],
        rgb([70, 52, 40]),
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [plinth, frame_top],
        [-d / 2.0, d / 2.0],
        rgb([150, 110, 80]),
    );
    // Headboard rises to the full height at the back, with an upholstered
    // panel on its face.
    let headboard = rgb([130, 95, 68]);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, h],
        [-d / 2.0, -d / 2.0 + head],
        headboard,
    );
    let mattress_top = h * 0.82;
    ctx.cube(
        [-w / 2.0 + 6.0, w / 2.0 - 6.0],
        [mattress_top - 4.0, h - 5.0],
        [-d / 2.0 + head, -d / 2.0 + head + 2.5],
        shade(headboard, 0.15),
    );
    // Folded-back edge of the duvet.
    ctx.cube(
        [-w / 2.0 + 1.0, w / 2.0 - 1.0],
        [mattress_top - 6.0, mattress_top + 5.0],
        [-d / 2.0 + d * 0.33, -d / 2.0 + d * 0.33 + 14.0],
        shade(c, 0.1),
    );
    ctx.cube(
        [-w / 2.0 + 2.0, w / 2.0 - 2.0],
        [frame_top, mattress_top],
        [-d / 2.0 + head, d / 2.0 - 2.0],
        rgb(LINEN),
    );
    // Duvet over the foot two thirds.
    ctx.cube(
        [-w / 2.0 + 1.0, w / 2.0 - 1.0],
        [mattress_top - 6.0, mattress_top + 3.0],
        [-d / 2.0 + d * 0.33, d / 2.0],
        c,
    );
    let n = pillows.max(1);
    let slot = (w - 20.0) / f64::from(n);
    for i in 0..n {
        let x0 = -w / 2.0 + 10.0 + f64::from(i) * slot + 3.0;
        ctx.cube(
            [x0, x0 + slot - 6.0],
            [mattress_top, mattress_top + 10.0],
            [-d / 2.0 + head + 6.0, -d / 2.0 + head + 42.0],
            rgb([252, 252, 250]),
        );
    }
}

fn table(ctx: &mut Ctx, round: bool) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let top = 3.0_f64.min(h * 0.08);
    if round {
        ctx.m
            .frustum([0.0, h - top, 0.0], Axis::Y, top, w / 2.0, w / 2.0, 40, c);
        // Recessed skirt, slim tapering pedestal and a low base disc.
        let skirt = 5.0_f64.min(h * 0.1);
        ctx.m.frustum(
            [0.0, h - top - skirt, 0.0],
            Axis::Y,
            skirt,
            w * 0.4,
            w * 0.4,
            32,
            shade(c, -0.1),
        );
        let r = (w * 0.05).max(2.5);
        ctx.m.frustum(
            [0.0, 2.5, 0.0],
            Axis::Y,
            h - top - skirt - 2.5,
            r,
            r * 0.7,
            16,
            shade(c, -0.2),
        );
        ctx.m.frustum(
            [0.0, 0.0, 0.0],
            Axis::Y,
            2.5,
            w * 0.25,
            w * 0.22,
            28,
            shade(c, -0.25),
        );
    } else {
        table_frame(ctx, [0.0, 0.0], [w, d], h, c);
        // Low tables (coffee tables) get a shelf between the legs.
        if h < 55.0 {
            let inset = (w.min(d) * 0.04).clamp(2.0, 5.0) + 2.0;
            ctx.cube(
                [-w / 2.0 + inset, w / 2.0 - inset],
                [h * 0.2, h * 0.2 + 2.0],
                [-d / 2.0 + inset, d / 2.0 - inset],
                shade(c, -0.05),
            );
        }
    }
}

/// Rectangular table centered at `center`: a 3 cm top over an apron and
/// four slim tapered legs.
fn table_frame(ctx: &mut Ctx, center: [f64; 2], size: [f64; 2], h: f64, c: Paint) {
    let ([cx, cz], [w, d]) = (center, size);
    let top = 3.0_f64.min(h * 0.08);
    ctx.cube(
        [cx - w / 2.0, cx + w / 2.0],
        [h - top, h],
        [cz - d / 2.0, cz + d / 2.0],
        c,
    );
    let leg = (w.min(d) * 0.06).clamp(3.5, 6.0);
    let inset = (w.min(d) * 0.04).clamp(2.0, 5.0);
    let (lx, lz) = (w / 2.0 - inset - leg / 2.0, d / 2.0 - inset - leg / 2.0);
    let wood = shade(c, -0.12);
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        ctx.leg(cx + sx * lx, cz + sz * lz, h - top, [leg * 0.7, leg], wood);
    }
    // Apron rails set just behind the outer faces of the legs.
    let apron = [h - top - 8.0_f64.min(h * 0.15), h - top];
    let (ox, oz) = (lx + leg / 2.0 - 0.6, lz + leg / 2.0 - 0.6);
    for s in [-1.0, 1.0] {
        let z = cz + s * oz;
        ctx.cube(
            [cx - lx, cx + lx],
            apron,
            [z.min(z - s * 2.0), z.max(z - s * 2.0)],
            wood,
        );
        let x = cx + s * ox;
        ctx.cube(
            [x.min(x - s * 2.0), x.max(x - s * 2.0)],
            apron,
            [cz - lz, cz + lz],
            wood,
        );
    }
}

fn chair(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let seat = 45.0_f64.min(h * 0.52);
    chair_at(&mut ctx.m, Place::HOME, [w, d, h], seat, c, shade(c, 0.12));
}

/// Contemporary dining chair filling `[w, d, h]` around `place`, front
/// towards +z: four slim legs, a seat board under a thin cushion, apron
/// rails, and a backrest panel held above the seat by rear posts that lean
/// back slightly.
fn chair_at(
    m: &mut PaintedMesh,
    place: Place,
    size: [f64; 3],
    seat: f64,
    wood: Paint,
    cushion: Paint,
) {
    let [w, d, h] = size;
    let leg = 3.2_f64.min(w * 0.08);
    let inset = 1.5;
    let lean = (d * 0.05).min(2.5);
    let legs = shade(wood, -0.15);
    let xs = [-w / 2.0 + inset + leg / 2.0, w / 2.0 - inset - leg / 2.0];
    let front = d / 2.0 - inset - leg / 2.0;
    // Center of the rear posts at height y: leaning back towards the top.
    let rear = |y: f64| -d / 2.0 + leg / 2.0 + lean * (1.0 - y / h);
    let square = |x: f64, z: f64, s: f64| [[x - s / 2.0, x + s / 2.0], [z - s / 2.0, z + s / 2.0]];
    let board = seat - 2.5;
    for x in xs {
        prism(
            m,
            place,
            [0.0, board - 2.5],
            square(x, front, leg * 0.8),
            square(x, front, leg),
            legs,
        );
        prism(
            m,
            place,
            [0.0, h],
            square(x, rear(0.0), leg),
            square(x, rear(h), leg),
            legs,
        );
    }
    // Seat board, cushion and apron rails.
    let back_edge = rear(board) - leg / 2.0;
    part(
        m,
        place,
        [-w / 2.0, w / 2.0],
        [board - 2.5, board],
        [back_edge, d / 2.0],
        wood,
    );
    part(
        m,
        place,
        [-w / 2.0 + 1.5, w / 2.0 - 1.5],
        [board, seat],
        [back_edge + 2.0, d / 2.0 - 1.5],
        cushion,
    );
    let rail = [board - 9.0, board - 2.5];
    part(
        m,
        place,
        [xs[0], xs[1]],
        rail,
        [front - 1.0, front + 1.0],
        legs,
    );
    let rz = rear(board - 6.0);
    part(m, place, [xs[0], xs[1]], rail, [rz - 1.0, rz + 1.0], legs);
    for x in xs {
        part(m, place, [x - 1.0, x + 1.0], rail, [rz, front], legs);
    }
    // Backrest panel between the posts, clear of the seat.
    let y = [seat + (h - seat) * 0.22, h - 1.5];
    let panel = |y: f64| [[xs[0], xs[1]], [rear(y) - 1.1, rear(y) + 1.1]];
    prism(m, place, y, panel(y[0]), panel(y[1]), wood);
}

fn stool(ctx: &mut Ctx) {
    let (w, h, c) = (ctx.w, ctx.h, ctx.c);
    ctx.m
        .frustum([0.0, h - 5.0, 0.0], Axis::Y, 5.0, w / 2.0, w / 2.0, 28, c);
    ctx.m
        .cylinder([0.0, 0.0, 0.0], Axis::Y, h - 5.0, 2.5, rgb(METAL));
    ctx.m.frustum(
        [0.0, 0.0, 0.0],
        Axis::Y,
        2.0,
        w / 2.0 * 0.9,
        w / 2.0 * 0.8,
        28,
        rgb(METAL),
    );
}

fn office_chair(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let seat = h * 0.45;
    for i in 0..5 {
        let a = f64::from(i) / 5.0 * std::f64::consts::TAU;
        let (x, z) = (a.cos() * w * 0.24, a.sin() * d * 0.24);
        ctx.m.block(
            [x, z],
            0.0,
            [
                w * 0.5 * a.cos().abs().max(0.15),
                4.0,
                d * 0.5 * a.sin().abs().max(0.15),
            ],
            rgb(DARK),
        );
    }
    ctx.m
        .cylinder([0.0, 4.0, 0.0], Axis::Y, seat - 10.0, 2.5, rgb(METAL));
    ctx.cube(
        [-w * 0.4, w * 0.4],
        [seat - 8.0, seat],
        [-d * 0.4, d * 0.42],
        c,
    );
    ctx.cube([-w * 0.36, w * 0.36], [seat, h], [-d * 0.46, -d * 0.36], c);
}

fn cabinet(ctx: &mut Ctx, doors: u8, drawers: u8) {
    cabinet_body(ctx, doors, drawers, None);
}

/// An optional central opening [width, depth, bottom] leaves room for a basin.
fn cabinet_body(ctx: &mut Ctx, doors: u8, drawers: u8, opening: Option<[f64; 3]>) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let plinth = if h > 60.0 { 8.0 } else { 3.0 };
    ctx.cube(
        [-w / 2.0 + 3.0, w / 2.0 - 3.0],
        [0.0, plinth],
        [-d / 2.0, d / 2.0 - 4.0],
        shade(c, -0.35),
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [plinth, opening.map_or(h, |o| o[2])],
        [-d / 2.0, d / 2.0 - 2.0],
        c,
    );
    if let Some([bw, bd, bottom]) = opening {
        for (x, z) in [
            ([-w / 2.0, -bw / 2.0], [-d / 2.0, d / 2.0 - 2.0]),
            ([bw / 2.0, w / 2.0], [-d / 2.0, d / 2.0 - 2.0]),
            ([-bw / 2.0, bw / 2.0], [-d / 2.0, -bd / 2.0]),
            ([-bw / 2.0, bw / 2.0], [bd / 2.0, d / 2.0 - 2.0]),
        ] {
            ctx.cube(x, [bottom, h], z, c);
        }
    }
    let front = d / 2.0 - 2.0;
    let panel = shade(c, 0.05);
    if drawers > 0 {
        let rows = f64::from(drawers);
        let step = (h - plinth) / rows;
        for i in 0..drawers {
            let y0 = plinth + f64::from(i) * step + 1.0;
            ctx.cube(
                [-w / 2.0 + 1.5, w / 2.0 - 1.5],
                [y0, y0 + step - 2.0],
                [front, front + 1.5],
                panel,
            );
            ctx.handle(0.0, y0 + step / 2.0, front + 1.5, false);
        }
    } else {
        let n = f64::from(doors.max(1));
        let step = w / n;
        for i in 0..doors.max(1) {
            let x0 = -w / 2.0 + f64::from(i) * step + 1.0;
            ctx.cube(
                [x0, x0 + step - 2.0],
                [plinth + 1.0, h - 1.0],
                [front, front + 1.5],
                panel,
            );
            let hx = if i % 2 == 0 {
                x0 + step - 7.0
            } else {
                x0 + 5.0
            };
            ctx.handle(hx, plinth + (h - plinth) * 0.55, front + 1.5, true);
        }
    }
}

fn shelf(ctx: &mut Ctx, levels: u8) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let t = 2.0;
    ctx.cube([-w / 2.0, -w / 2.0 + t], [0.0, h], [-d / 2.0, d / 2.0], c);
    ctx.cube([w / 2.0 - t, w / 2.0], [0.0, h], [-d / 2.0, d / 2.0], c);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, h],
        [-d / 2.0, -d / 2.0 + 1.0],
        shade(c, -0.2),
    );
    let n = levels.max(2);
    for i in 0..=n {
        let y = f64::from(i) / f64::from(n) * (h - t);
        ctx.cube(
            [-w / 2.0 + t, w / 2.0 - t],
            [y, y + t],
            [-d / 2.0 + 1.0, d / 2.0],
            c,
        );
    }
    // A few books on the middle shelves.
    let palette = [[160, 60, 50], [50, 90, 140], [210, 180, 90], [70, 120, 80]];
    for i in 1..n.saturating_sub(1) {
        let y = f64::from(i) / f64::from(n) * (h - t) + t;
        let mut x = -w / 2.0 + t + 3.0;
        let mut k: u32 = 0;
        while x < w / 2.0 - t - 20.0 {
            let bw = 3.0 + f64::from(k % 3);
            ctx.cube(
                [x, x + bw],
                [y, y + (h / f64::from(n)) * 0.7],
                [-d / 2.0 + 3.0, d / 2.0 - 6.0],
                rgb(palette[(usize::from(i) + k as usize) % 4]),
            );
            x += bw + 0.5;
            k += 1;
        }
    }
}

/// Low media console on slim metal legs, with a row of front panels.
fn tv_stand(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let feet = (h * 0.2).min(12.0);
    let (lx, lz) = (w / 2.0 - 6.0, d / 2.0 - 5.0);
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        ctx.leg(sx * lx, sz * lz, feet, [2.0, 3.0], rgb(METAL));
    }
    let front = d / 2.0 - 2.0;
    ctx.cube([-w / 2.0, w / 2.0], [feet, h], [-d / 2.0, front], c);
    let n = (w / 60.0).round().max(2.0);
    let step = (w - 2.0) / n;
    for i in 0..crate::count(n) {
        let x0 = -w / 2.0 + 1.0 + f64::from(i) * step + 0.4;
        ctx.cube(
            [x0, x0 + step - 0.8],
            [feet + 1.5, h - 1.5],
            [front, d / 2.0 - 0.6],
            shade(c, 0.06),
        );
        // Slim finger-pull along the panel's top edge.
        let mid = x0 + step / 2.0;
        ctx.cube(
            [mid - 6.0, mid + 6.0],
            [h - 5.0, h - 4.0],
            [d / 2.0 - 0.6, d / 2.0],
            rgb(METAL),
        );
    }
}

fn tv(ctx: &mut Ctx) {
    let (w, d, h) = (ctx.w, ctx.d, ctx.h);
    let foot = (h * 0.08).min(6.0);
    ctx.cube(
        [-w * 0.15, w * 0.15],
        [0.0, 1.5],
        [-d / 2.0, d / 2.0],
        ctx.body_or_default(DARK),
    );
    ctx.cube(
        [-3.0, 3.0],
        [0.0, foot + 5.0],
        [-1.0, 1.0],
        ctx.body_or_default(DARK),
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [foot, h],
        [-d * 0.2, d * 0.2],
        ctx.body_or_default(DARK),
    );
    ctx.cube(
        [-w / 2.0 + 1.5, w / 2.0 - 1.5],
        [foot + 1.5, h - 1.5],
        [d * 0.2, d * 0.2 + 0.3],
        rgb([20, 24, 32]),
    );
}

fn fridge(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    ctx.cube([-w / 2.0, w / 2.0], [0.0, h], [-d / 2.0, d / 2.0 - 3.0], c);
    let front = d / 2.0 - 3.0;
    let split = h * 0.32;
    ctx.cube(
        [-w / 2.0 + 0.5, w / 2.0 - 0.5],
        [1.0, split - 0.5],
        [front, front + 3.0],
        shade(c, 0.06),
    );
    ctx.cube(
        [-w / 2.0 + 0.5, w / 2.0 - 0.5],
        [split + 0.5, h - 0.5],
        [front, front + 3.0],
        shade(c, 0.06),
    );
    ctx.handle(w / 2.0 - 6.0, split + 30.0, front + 3.0, true);
    ctx.handle(w / 2.0 - 6.0, split - 20.0, front + 3.0, true);
}

fn stove(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, h - 2.0],
        [-d / 2.0, d / 2.0 - 1.0],
        c,
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - 2.0, h - 1.0],
        [-d / 2.0, d / 2.0 - 1.0],
        rgb(DARK),
    );
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        ctx.m.cylinder(
            [sx * w * 0.22, h - 1.0, sz * d * 0.2],
            Axis::Y,
            1.0,
            (w * 0.09).min(7.0),
            rgb([90, 92, 96]),
        );
    }
    let front = d / 2.0 - 1.0;
    ctx.cube(
        [-w / 2.0 + 4.0, w / 2.0 - 4.0],
        [10.0, h * 0.7],
        [front, front + 1.0],
        rgb([30, 32, 36]),
    );
    ctx.handle(0.0, h * 0.74, front + 1.0, false);
}

fn counter(ctx: &mut Ctx, sink: bool) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let top = 4.0;
    ctx.cube(
        [-w / 2.0 + 1.0, w / 2.0 - 1.0],
        [0.0, 10.0],
        [-d / 2.0, d / 2.0 - 6.0],
        shade(c, -0.4),
    );
    ctx.cube(
        [-w / 2.0 + 1.0, w / 2.0 - 1.0],
        [10.0, h - top],
        [-d / 2.0, d / 2.0 - 4.0],
        c,
    );
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let doors = ((w / 60.0).round() as u8).clamp(1, 6);
    let step = (w - 2.0) / f64::from(doors);
    for i in 0..doors {
        let x0 = -w / 2.0 + 1.0 + f64::from(i) * step + 0.8;
        ctx.cube(
            [x0, x0 + step - 1.6],
            [11.0, h - top - 1.0],
            [d / 2.0 - 4.0, d / 2.0 - 2.5],
            shade(c, 0.05),
        );
        ctx.handle(x0 + step / 2.0, h - top - 8.0, d / 2.0 - 2.5, false);
    }
    let stone = rgb([70, 72, 78]);
    if sink {
        let (bw, bd) = ((w * 0.45).min(60.0), (d * 0.6).min(45.0));
        ctx.cube(
            [-w / 2.0, w / 2.0],
            [h - top, h],
            [-d / 2.0, -bd / 2.0],
            stone,
        );
        ctx.cube(
            [-w / 2.0, w / 2.0],
            [h - top, h],
            [bd / 2.0, d / 2.0],
            stone,
        );
        ctx.cube(
            [-w / 2.0, -bw / 2.0],
            [h - top, h],
            [-bd / 2.0, bd / 2.0],
            stone,
        );
        ctx.cube(
            [bw / 2.0, w / 2.0],
            [h - top, h],
            [-bd / 2.0, bd / 2.0],
            stone,
        );
        ctx.cube(
            [-bw / 2.0, bw / 2.0],
            [h - 20.0, h - top],
            [-bd / 2.0, bd / 2.0],
            rgb(METAL),
        );
        ctx.m.cylinder(
            [0.0, h - top, -bd / 2.0 - 4.0],
            Axis::Y,
            top,
            1.5,
            rgb(METAL),
        );
    } else {
        ctx.cube(
            [-w / 2.0, w / 2.0],
            [h - top, h],
            [-d / 2.0, d / 2.0],
            stone,
        );
    }
}

fn appliance(ctx: &mut Ctx, round_door: bool) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    ctx.cube([-w / 2.0, w / 2.0], [0.0, h], [-d / 2.0, d / 2.0 - 2.0], c);
    let front = d / 2.0 - 2.0;
    ctx.cube(
        [-w / 2.0 + 2.0, w / 2.0 - 2.0],
        [h - 12.0, h - 2.0],
        [front, front + 1.0],
        shade(c, -0.1),
    );
    if round_door {
        let r = (w.min(h) * 0.3).min(22.0);
        ctx.m
            .cylinder([0.0, h * 0.45, front], Axis::Z, 2.0, r, rgb(METAL));
        ctx.m.cylinder(
            [0.0, h * 0.45, front + 2.0],
            Axis::Z,
            0.5,
            r * 0.75,
            rgb(GLASS),
        );
    } else {
        ctx.cube(
            [-w / 2.0 + 1.0, w / 2.0 - 1.0],
            [2.0, h - 13.0],
            [front, front + 1.5],
            shade(c, 0.05),
        );
        ctx.handle(0.0, h - 17.0, front + 1.5, false);
    }
}

fn microwave(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    ctx.cube([-w / 2.0, w / 2.0], [0.0, h], [-d / 2.0, d / 2.0 - 1.0], c);
    ctx.cube(
        [-w / 2.0 + 2.0, w / 2.0 - 12.0],
        [3.0, h - 3.0],
        [d / 2.0 - 1.0, d / 2.0],
        rgb([20, 22, 28]),
    );
    ctx.cube(
        [w / 2.0 - 10.0, w / 2.0 - 2.0],
        [3.0, h - 3.0],
        [d / 2.0 - 1.0, d / 2.0],
        rgb(METAL),
    );
}

/// A canopy over the cooktop, narrowing into the duct that carries the air
/// out. The canopy is what catches: it is drawn the full width of the piece,
/// because a hood narrower than the burners loses most of the front ones.
fn hood(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let canopy = (h * 0.35).clamp(6.0, 18.0);
    // The rim, wide and shallow, with a dark filter underneath.
    ctx.cube([-w / 2.0, w / 2.0], [0.0, canopy], [-d / 2.0, d / 2.0], c);
    ctx.cube(
        [-w / 2.0 + 2.0, w / 2.0 - 2.0],
        [0.0, 0.8],
        [-d / 2.0 + 2.0, d / 2.0 - 2.0],
        rgb([38, 40, 44]),
    );
    // The duct, centred, running to the top of the piece.
    let (dw, dd) = ((w * 0.3).min(30.0), (d * 0.45).min(26.0));
    if h > canopy {
        ctx.cube([-dw / 2.0, dw / 2.0], [canopy, h], [-dd / 2.0, dd / 2.0], c);
    }
}

/// A pop-up tower: its elevation is the countertop, its height what rises.
fn outlet_tower(ctx: &mut Ctx) {
    let (w, h) = (ctx.w, ctx.h);
    let flange = w / 2.0;
    let body = w * 0.36;
    // The body that hangs in the cabinet, about 28 cm under the top.
    ctx.m.cylinder(
        [0.0, -28.0, 0.0],
        Axis::Y,
        28.0,
        body * 0.95,
        rgb([90, 90, 94]),
    );
    ctx.m
        .cylinder([0.0, 0.0, 0.0], Axis::Y, 0.4, flange, rgb([170, 172, 176]));
    ctx.m
        .cylinder([0.0, 0.4, 0.0], Axis::Y, h - 1.4, body, ctx.c);
    ctx.m.cylinder(
        [0.0, h - 1.0, 0.0],
        Axis::Y,
        1.0,
        body * 1.08,
        rgb([185, 187, 190]),
    );
    // Sockets on the faces, one every 5 cm up the body.
    let mut y = 2.5;
    while y + 4.0 <= h - 1.5 || y <= 2.5 {
        ctx.cube(
            [-1.8, 1.8],
            [y, y + 4.0],
            [body - 0.2, body + 0.15],
            rgb([230, 230, 226]),
        );
        y += 5.5;
    }
}

/// A desk box: the collar flush with the top and its lid.
fn desk_box(ctx: &mut Ctx) {
    let (w, d, h) = (ctx.w, ctx.d, ctx.h);
    ctx.cube(
        [-w / 2.0 + 0.8, w / 2.0 - 0.8],
        [-8.0, 0.0],
        [-d / 2.0 + 0.8, d / 2.0 - 0.8],
        rgb([70, 70, 74]),
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, h * 0.4],
        [-d / 2.0, d / 2.0],
        ctx.c,
    );
    ctx.cube(
        [-w / 2.0 + 1.0, w / 2.0 - 1.0],
        [h * 0.4, h],
        [-d / 2.0 + 1.0, d / 2.0 - 1.0],
        shade(ctx.c, -0.12),
    );
}

fn cooktop(ctx: &mut Ctx) {
    let (w, d, h) = (ctx.w, ctx.d, ctx.h);
    // The burner box hangs under the counter; the glass shows on top.
    let glass = 0.6_f64.min(h);
    ctx.cube(
        [-w / 2.0 + 3.0, w / 2.0 - 3.0],
        [0.0, h - glass],
        [-d / 2.0 + 3.0, d / 2.0 - 3.0],
        rgb([60, 62, 66]),
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - glass, h],
        [-d / 2.0, d / 2.0],
        ctx.body_or_default([22, 22, 26]),
    );
    for (sx, sz, r) in [
        (-0.22, -0.2, 7.0),
        (0.22, -0.2, 5.5),
        (-0.22, 0.2, 5.5),
        (0.22, 0.2, 8.5),
    ] {
        ctx.m.cylinder(
            [sx * w, h, sz * d],
            Axis::Y,
            0.15,
            f64::min(r, w * 0.12),
            rgb([70, 70, 76]),
        );
    }
}

fn sink_bowl(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let wall = 0.8;
    // Rim on the counter, bowl below it.
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - 0.5, h],
        [-d / 2.0, -d / 2.0 + 2.5],
        c,
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - 0.5, h],
        [d / 2.0 - 2.5, d / 2.0],
        c,
    );
    ctx.cube(
        [-w / 2.0, -w / 2.0 + 2.5],
        [h - 0.5, h],
        [-d / 2.0, d / 2.0],
        c,
    );
    ctx.cube(
        [w / 2.0 - 2.5, w / 2.0],
        [h - 0.5, h],
        [-d / 2.0, d / 2.0],
        c,
    );
    let (iw, id) = (w / 2.0 - 2.5, d / 2.0 - 2.5);
    ctx.cube([-iw, iw], [0.0, wall], [-id, id], shade(c, -0.15));
    ctx.cube([-iw, iw], [0.0, h - 0.5], [-id, -id + wall], c);
    ctx.cube([-iw, iw], [0.0, h - 0.5], [id - wall, id], c);
    ctx.cube([-iw, -iw + wall], [0.0, h - 0.5], [-id, id], c);
    ctx.cube([iw - wall, iw], [0.0, h - 0.5], [-id, id], c);
    ctx.m
        .cylinder([0.0, wall, 0.0], Axis::Y, 0.2, 2.5, rgb([120, 122, 126]));
}

fn oven(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    ctx.cube([-w / 2.0, w / 2.0], [0.0, h], [-d / 2.0, d / 2.0 - 2.0], c);
    let front = d / 2.0 - 2.0;
    ctx.cube(
        [-w / 2.0 + 1.0, w / 2.0 - 1.0],
        [2.0, h - 10.0],
        [front, front + 2.0],
        rgb([18, 18, 22]),
    );
    ctx.cube(
        [-w / 2.0 + 1.0, w / 2.0 - 1.0],
        [h - 9.0, h - 1.0],
        [front, front + 1.5],
        rgb(METAL),
    );
    ctx.handle(0.0, h - 13.0, front + 2.0, false);
}

fn toilet(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let tank_d = (d * 0.25).clamp(12.0, 20.0);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h * 0.5, h],
        [-d / 2.0, -d / 2.0 + tank_d],
        c,
    );
    let bowl_r = (w / 2.0).min((d - tank_d) / 2.0);
    let center = -d / 2.0 + tank_d + (d - tank_d) / 2.0;
    ctx.m.frustum(
        [0.0, 0.0, center],
        Axis::Y,
        h * 0.3,
        bowl_r * 0.55,
        bowl_r * 0.75,
        24,
        c,
    );
    ctx.m.frustum(
        [0.0, h * 0.3, center],
        Axis::Y,
        h * 0.22,
        bowl_r * 0.9,
        bowl_r,
        24,
        c,
    );
    ctx.cube(
        [-w / 2.0 + 2.0, w / 2.0 - 2.0],
        [h * 0.5 - 2.0, h * 0.53],
        [-d / 2.0 + tank_d, d / 2.0],
        shade(c, 0.05),
    );
}

fn basin(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let (bw, bd) = ((w * 0.6).min(60.0), (d * 0.55).min(32.0));
    let bottom = h - (h * 0.16).min(15.0);
    ctx.h = h - 3.0;
    cabinet_body(ctx, 2, 0, Some([bw, bd, bottom - 1.0]));
    ctx.h = h;
    let porcelain = rgb([235, 237, 239]);
    // Both the worktop and carcass must be open: a bowl painted below a
    // solid top is invisible, even when its material is a different color.
    for (x, z) in [
        ([-w / 2.0, -bw / 2.0], [-d / 2.0, d / 2.0]),
        ([bw / 2.0, w / 2.0], [-d / 2.0, d / 2.0]),
        ([-bw / 2.0, bw / 2.0], [-d / 2.0, -bd / 2.0]),
        ([-bw / 2.0, bw / 2.0], [bd / 2.0, d / 2.0]),
    ] {
        ctx.cube(x, [h - 3.0, h], z, shade(c, 0.05));
    }
    let ring = |scale: f64, y: f64| {
        [
            [-bw * scale / 2.0, y, -bd * scale / 2.0],
            [bw * scale / 2.0, y, -bd * scale / 2.0],
            [bw * scale / 2.0, y, bd * scale / 2.0],
            [-bw * scale / 2.0, y, bd * scale / 2.0],
        ]
        .map(|p| Place::HOME.at(p))
    };
    let (rim, low) = (ring(1.0, h), ring(0.7, bottom));
    for i in 0..4 {
        let j = (i + 1) % 4;
        ctx.m.polygon(&[rim[j], rim[i], low[i], low[j]], porcelain);
    }
    ctx.m.polygon(&[low[3], low[2], low[1], low[0]], porcelain);
    // Drain sits on the exposed bowl floor; preserve the model's declared
    // countertop height instead of resizing the entire cabinet for a tap.
    ctx.m.cylinder(
        [0.0, bottom, 0.0],
        Axis::Y,
        0.15,
        bw.min(bd) * 0.055,
        rgb(METAL),
    );
}

fn shower(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, 5.0],
        [-d / 2.0, d / 2.0],
        rgb([240, 240, 238]),
    );
    let glass = shade(c, 0.3);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [5.0, h],
        [d / 2.0 - 1.0, d / 2.0],
        glass,
    );
    ctx.cube(
        [w / 2.0 - 1.0, w / 2.0],
        [5.0, h],
        [-d / 2.0, d / 2.0],
        glass,
    );
    ctx.cube(
        [-w / 2.0, -w / 2.0 + 1.0],
        [h - 3.0, h],
        [-d / 2.0, d / 2.0],
        rgb(METAL),
    );
    ctx.m.cylinder(
        [-w / 2.0 + 10.0, h - 25.0, -d / 2.0 + 10.0],
        Axis::Y,
        3.0,
        8.0,
        rgb(METAL),
    );
}

fn bathtub(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let rim = 8.0;
    ctx.cube([-w / 2.0, w / 2.0], [0.0, h - 15.0], [-d / 2.0, d / 2.0], c);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - 15.0, h],
        [-d / 2.0, -d / 2.0 + rim],
        c,
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - 15.0, h],
        [d / 2.0 - rim, d / 2.0],
        c,
    );
    ctx.cube(
        [-w / 2.0, -w / 2.0 + rim],
        [h - 15.0, h],
        [-d / 2.0, d / 2.0],
        c,
    );
    ctx.cube(
        [w / 2.0 - rim, w / 2.0],
        [h - 15.0, h],
        [-d / 2.0, d / 2.0],
        c,
    );
    ctx.cube(
        [-w / 2.0 + rim, w / 2.0 - rim],
        [h - 16.0, h - 15.0],
        [-d / 2.0 + rim, d / 2.0 - rim],
        rgb([180, 210, 225]),
    );
}

fn door(ctx: &mut Ctx, leaves: u8, sliding: bool) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let frame = 5.0;
    let trim = if sliding {
        ctx.body_or_default([240, 238, 232])
    } else {
        rgb([240, 238, 232])
    };
    // The jambs stop under the head, so no two faces lie on each other.
    ctx.cube(
        [-w / 2.0, -w / 2.0 + frame],
        [0.0, h - frame],
        [-d / 2.0, d / 2.0],
        trim,
    );
    ctx.cube(
        [w / 2.0 - frame, w / 2.0],
        [0.0, h - frame],
        [-d / 2.0, d / 2.0],
        trim,
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - frame, h],
        [-d / 2.0, d / 2.0],
        trim,
    );
    let inner = w - 2.0 * frame;
    let n = f64::from(leaves.max(1));
    for i in 0..leaves.max(1) {
        let x0 = -w / 2.0 + frame + f64::from(i) * inner / n;
        let z = if sliding {
            (f64::from(i) - 0.5) * 3.5
        } else {
            0.0
        };
        if sliding {
            // Each leaf fills its share of the opening: a metal stile, then
            // its glass, side by side, so no two faces lie on each other.
            let stile = 4.0;
            ctx.cube(
                [x0, x0 + stile],
                [0.0, h - frame],
                [z - 1.8, z + 1.8],
                rgb(METAL),
            );
            ctx.cube(
                [x0 + stile, x0 + inner / n],
                [0.0, h - frame],
                [z - 1.5, z + 1.5],
                rgb(GLASS),
            );
        } else {
            ctx.cube(
                [x0 + 0.3, x0 + inner / n - 0.3],
                [0.5, h - frame - 0.3],
                [-2.0, 2.0],
                c,
            );
            let hx = if i == 0 {
                x0 + inner / n - 8.0
            } else {
                x0 + 8.0
            };
            ctx.handle(hx, 100.0_f64.min(h * 0.5), 2.0, false);
            ctx.cube(
                [hx - 6.0, hx + 6.0],
                [100.0_f64.min(h * 0.5) - 0.75, 100.0_f64.min(h * 0.5) + 0.75],
                [-4.0, -2.0],
                rgb(METAL),
            );
        }
    }
}

fn window(ctx: &mut Ctx, panes: u8, _is_window: bool) {
    let (w, d, h) = (ctx.w, ctx.d, ctx.h);
    let frame = 5.0;
    let white = ctx.body_or_default([245, 245, 242]);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, frame],
        [-d / 2.0, d / 2.0],
        white,
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - frame, h],
        [-d / 2.0, d / 2.0],
        white,
    );
    // The posts stand between the sill and the head, so no two faces lie
    // on each other at the corners (a path tracer turns those black).
    ctx.cube(
        [-w / 2.0, -w / 2.0 + frame],
        [frame, h - frame],
        [-d / 2.0, d / 2.0],
        white,
    );
    ctx.cube(
        [w / 2.0 - frame, w / 2.0],
        [frame, h - frame],
        [-d / 2.0, d / 2.0],
        white,
    );
    // A pane of glass between each two mullions, ending on them.
    let n = panes.max(1);
    let step = (w - 2.0 * frame) / f64::from(n);
    let mullion = 1.5;
    for i in 0..n {
        let left = -w / 2.0 + frame + f64::from(i) * step;
        let right = left + step;
        if i > 0 {
            ctx.cube(
                [left - mullion, left + mullion],
                [frame, h - frame],
                [-2.0, 2.0],
                white,
            );
        }
        ctx.cube(
            [
                if i > 0 { left + mullion } else { left },
                if i + 1 < n { right - mullion } else { right },
            ],
            [frame, h - frame],
            [-0.4, 0.4],
            rgb(GLASS),
        );
    }
}

fn passage(ctx: &mut Ctx) {
    let (w, d, h) = (ctx.w, ctx.d, ctx.h);
    let trim = ctx.body_or_default([240, 238, 232]);
    ctx.cube(
        [-w / 2.0, -w / 2.0 + 2.0],
        [0.0, h - 2.0],
        [-d / 2.0, d / 2.0],
        trim,
    );
    ctx.cube(
        [w / 2.0 - 2.0, w / 2.0],
        [0.0, h - 2.0],
        [-d / 2.0, d / 2.0],
        trim,
    );
    ctx.cube([-w / 2.0, w / 2.0], [h - 2.0, h], [-d / 2.0, d / 2.0], trim);
}

fn stairs(ctx: &mut Ctx, steps: u32) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let n = f64::from(steps.max(2));
    let rise = h / n;
    let run = d / n;
    // Climbs from the front (+z, first step at floor level) towards the back
    // (-z, the last tread is flush with the top of the piece), matching the
    // plan symbol's arrow.
    let side = (w * 0.05).clamp(3.0, 5.0);
    let stringer = shade(c, -0.25);
    // Open treads between two sloped stringers. Each stringer is a sheared
    // box running from the first tread at the floor to the last at the top.
    let band = (run * 1.5).min(d / 2.0);
    for x0 in [-w / 2.0, w / 2.0 - side] {
        prism(
            &mut ctx.m,
            Place::HOME,
            [0.0, h],
            [[x0, x0 + side], [d / 2.0 - band, d / 2.0]],
            [[x0, x0 + side], [-d / 2.0, -d / 2.0 + band]],
            stringer,
        );
    }
    let tread = 4.0_f64.min(rise * 0.4);
    for i in 0..steps.max(2) {
        let k = f64::from(i);
        let z1 = d / 2.0 - k * run;
        let top = rise * (k + 1.0);
        ctx.cube(
            [-w / 2.0 + side - 0.5, w / 2.0 - side + 0.5],
            [top - tread, top],
            [z1 - run, z1],
            if i % 2 == 0 { c } else { shade(c, 0.04) },
        );
    }
    // Handrail on the +x side, 90 cm above the nosings, as far up as the
    // piece's height allows, with a baluster every few steps.
    let rail = 90.0;
    let last = ((h - rail) / rise).floor() - 1.0;
    if last >= 2.0 {
        let metal = rgb(METAL);
        let x = [w / 2.0 - side / 2.0 - 1.5, w / 2.0 - side / 2.0 + 1.5];
        let z = |k: f64| d / 2.0 - (k + 0.5) * run;
        prism(
            &mut ctx.m,
            Place::HOME,
            [rise + rail - 2.0, rise * (last + 1.0) + rail],
            [x, [z(0.0) - 2.5, z(0.0) + 2.5]],
            [x, [z(last) - 2.5, z(last) + 2.5]],
            metal,
        );
        let mut k = 0.0;
        while k <= last {
            let top = rise * (k + 1.0);
            ctx.cube(
                [w / 2.0 - side / 2.0 - 1.0, w / 2.0 - side / 2.0 + 1.0],
                [top, top + rail - 1.0],
                [z(k) - 1.0, z(k) + 1.0],
                metal,
            );
            k += 3.0;
        }
    }
}

fn plant(ctx: &mut Ctx) {
    let (w, h, c) = (ctx.w, ctx.h, ctx.c);
    // A matte ceramic pot with a rim and dark soil.
    let pot_h = (h * 0.28).min(45.0);
    let pot = rgb([226, 221, 212]);
    ctx.m
        .frustum([0.0, 0.0, 0.0], Axis::Y, pot_h, w * 0.24, w * 0.3, 28, pot);
    ctx.m.frustum(
        [0.0, pot_h - 2.0, 0.0],
        Axis::Y,
        2.0,
        w * 0.31,
        w * 0.31,
        28,
        shade(pot, 0.05),
    );
    ctx.m.frustum(
        [0.0, pot_h - 3.0, 0.0],
        Axis::Y,
        0.5,
        w * 0.28,
        w * 0.28,
        24,
        rgb([62, 46, 34]),
    );
    // Lance-shaped leaves on a golden-angle spiral: the inner ones stand
    // up, the outer ones arch out, each a little longer, wider and a
    // different green, seen from both sides.
    let foliage = h - pot_h;
    let base = [0.0, pot_h - 1.0, 0.0];
    let count = 34u32;
    let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    for i in 0..count {
        let k = f64::from(i) / f64::from(count - 1);
        let azimuth = f64::from(i) * golden;
        let tilt = (8.0 + 52.0 * k).to_radians();
        // Deterministic jitter so no two plants of the same size differ.
        let jitter = ((f64::from(i) * 12.9898).sin() * 43_758.545).fract().abs();
        let reach = (w * 0.5 / tilt.sin().max(0.2)).min(foliage * (1.02 - 0.3 * k));
        let length = reach * (0.8 + 0.2 * jitter);
        let dir = [
            tilt.sin() * azimuth.cos(),
            tilt.cos(),
            tilt.sin() * azimuth.sin(),
        ];
        // Across the leaf: horizontal, perpendicular to where it points.
        let side = [-azimuth.sin(), 0.0, azimuth.cos()];
        #[allow(clippy::cast_possible_truncation)]
        let at = |t: f64, across: f64, droop: f64| -> [f32; 3] {
            [
                (base[0] + dir[0] * length * t + side[0] * across) as f32,
                (base[1] + dir[1] * length * t - droop) as f32,
                (base[2] + dir[2] * length * t + side[2] * across) as f32,
            ]
        };
        let half = length * (0.1 + 0.05 * jitter);
        // Outer leaves bend down toward their tips.
        let sag = length * 0.18 * k;
        let (root, left, right, tip) = (
            at(0.0, 0.0, 0.0),
            at(0.42, half, sag * 0.2),
            at(0.42, -half, sag * 0.2),
            at(1.0, 0.0, sag),
        );
        #[allow(clippy::cast_possible_truncation)]
        let green = shade(c, (jitter as f32 - 0.5) * 0.22 - 0.08 * k as f32);
        ctx.m.polygon(&[root, right, tip, left], green);
        ctx.m
            .polygon(&[root, left, tip, right], shade(green, -0.08));
    }
}

fn tree(ctx: &mut Ctx) {
    let (w, h, c) = (ctx.w, ctx.h, ctx.c);
    let trunk = h * 0.35;
    ctx.m.cylinder(
        [0.0, 0.0, 0.0],
        Axis::Y,
        trunk,
        (w * 0.05).max(8.0),
        rgb([110, 80, 55]),
    );
    ctx.m.frustum(
        [0.0, trunk * 0.8, 0.0],
        Axis::Y,
        (h - trunk * 0.8) * 0.6,
        w * 0.5,
        w * 0.3,
        18,
        c,
    );
    ctx.m.frustum(
        [0.0, trunk * 0.8 + (h - trunk * 0.8) * 0.5, 0.0],
        Axis::Y,
        (h - trunk * 0.8) * 0.5,
        w * 0.34,
        0.0,
        18,
        shade(c, 0.1),
    );
}

fn lamp(ctx: &mut Ctx) {
    let (w, h, c) = (ctx.w, ctx.h, ctx.c);
    ctx.m.frustum(
        [0.0, 0.0, 0.0],
        Axis::Y,
        3.0,
        w * 0.35,
        w * 0.3,
        20,
        rgb(DARK),
    );
    ctx.m
        .cylinder([0.0, 3.0, 0.0], Axis::Y, h * 0.7, 1.2, rgb(METAL));
    ctx.m.frustum(
        [0.0, h * 0.68, 0.0],
        Axis::Y,
        h * 0.32,
        w * 0.5,
        w * 0.3,
        24,
        c,
    );
}

fn car(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    // Length runs along the width axis.
    let wheel_r = (h * 0.22).min(34.0);
    for sx in [-1.0, 1.0] {
        for sz in [-1.0, 1.0] {
            let z0 = if sz > 0.0 { d / 2.0 - 12.0 } else { -d / 2.0 };
            ctx.m.cylinder(
                [sx * w * 0.3, wheel_r, z0],
                Axis::Z,
                12.0,
                wheel_r,
                rgb(DARK),
            );
        }
    }
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [wheel_r * 0.6, h * 0.55],
        [-d / 2.0 + 8.0, d / 2.0 - 8.0],
        c,
    );
    ctx.cube(
        [-w * 0.22, w * 0.25],
        [h * 0.55, h],
        [-d / 2.0 + 16.0, d / 2.0 - 16.0],
        shade(c, 0.05),
    );
    ctx.cube(
        [-w * 0.2, w * 0.23],
        [h * 0.58, h * 0.95],
        [-d / 2.0 + 15.5, d / 2.0 - 15.5],
        rgb(GLASS),
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, 0.1],
        [-d / 2.0, d / 2.0],
        rgb(DARK),
    );
}

fn desk(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    ctx.cube([-w / 2.0, w / 2.0], [h - 3.0, h], [-d / 2.0, d / 2.0], c);
    ctx.cube(
        [-w / 2.0, -w / 2.0 + 3.0],
        [0.0, h - 3.0],
        [-d / 2.0, d / 2.0],
        shade(c, -0.1),
    );
    let drawer_w = (w * 0.35).min(45.0);
    ctx.cube(
        [w / 2.0 - drawer_w, w / 2.0],
        [0.0, h - 3.0],
        [-d / 2.0, d / 2.0 - 1.5],
        shade(c, -0.05),
    );
    for i in 0..3 {
        let y0 = 4.0 + f64::from(i) * (h - 8.0) / 3.0;
        ctx.cube(
            [w / 2.0 - drawer_w + 1.0, w / 2.0 - 1.0],
            [y0, y0 + (h - 8.0) / 3.0 - 1.5],
            [d / 2.0 - 1.5, d / 2.0],
            shade(c, 0.05),
        );
        ctx.handle(
            w / 2.0 - drawer_w / 2.0,
            y0 + (h - 8.0) / 6.0,
            d / 2.0,
            false,
        );
    }
    ctx.cube(
        [-w / 2.0 + 3.0, w / 2.0 - drawer_w],
        [h - 25.0, h - 3.0],
        [-d / 2.0, -d / 2.0 + 2.0],
        shade(c, -0.1),
    );
}

fn crib(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    ctx.legs(0.0, h, 5.0, c);
    ctx.cube(
        [-w / 2.0 + 5.0, w / 2.0 - 5.0],
        [25.0, 35.0],
        [-d / 2.0 + 5.0, d / 2.0 - 5.0],
        rgb(LINEN),
    );
    for (z0, z1) in [(-d / 2.0, -d / 2.0 + 3.0), (d / 2.0 - 3.0, d / 2.0)] {
        ctx.cube([-w / 2.0, w / 2.0], [h - 5.0, h], [z0, z1], c);
        let mut x = -w / 2.0 + 10.0;
        while x < w / 2.0 - 8.0 {
            ctx.cube([x, x + 2.0], [35.0, h - 5.0], [z0, z1], c);
            x += 9.0;
        }
    }
    for (x0, x1) in [(-w / 2.0, -w / 2.0 + 3.0), (w / 2.0 - 3.0, w / 2.0)] {
        ctx.cube([x0, x1], [h - 5.0, h], [-d / 2.0, d / 2.0], c);
        let mut z = -d / 2.0 + 10.0;
        while z < d / 2.0 - 8.0 {
            ctx.cube([x0, x1], [35.0, h - 5.0], [z, z + 2.0], c);
            z += 9.0;
        }
    }
}

fn footing(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let base = h * 0.4;
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, base],
        [-d / 2.0, d / 2.0],
        shade(c, -0.1),
    );
    let (pw, pd) = (w * 0.45, d * 0.45);
    ctx.cube([-pw / 2.0, pw / 2.0], [base, h], [-pd / 2.0, pd / 2.0], c);
}

/// A bar railing: posts every 1,20 m, a handrail and a bottom rail, and
/// vertical bars 1,9 cm thick with a 9,5 cm clear gap — under the 11 cm
/// NBR 14718 allows, and nothing to climb on.
fn railing(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let post = d.clamp(3.0, 6.0);
    let spans = (w / 120.0).ceil().max(1.0);
    for i in 0..=crate::count(spans) {
        let x = -w / 2.0 + post / 2.0 + (w - post) * f64::from(i) / spans;
        ctx.cube(
            [x - post / 2.0, x + post / 2.0],
            [0.0, h],
            [-d / 2.0, d / 2.0],
            c,
        );
    }
    let rail = post * 0.8;
    ctx.cube([-w / 2.0, w / 2.0], [h - rail, h], [-d / 2.0, d / 2.0], c);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [4.0, 4.0 + rail * 0.7],
        [-d / 2.0, d / 2.0],
        c,
    );
    let bar = 1.9;
    let pitch = bar + 9.5;
    let mut x = -w / 2.0 + post + pitch;
    while x < w / 2.0 - post - bar / 2.0 {
        ctx.cube(
            [x - bar / 2.0, x + bar / 2.0],
            [4.0 + rail * 0.7, h - rail],
            [-bar / 2.0, bar / 2.0],
            c,
        );
        x += pitch;
    }
}

/// Laminated glass between posts, under a flat metal handrail.
fn glass_railing(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let metal = rgb(METAL);
    let post = 4.0_f64.min(w / 6.0);
    let spans = (w / 110.0).ceil().max(1.0);
    for i in 0..=crate::count(spans) {
        let x = -w / 2.0 + post / 2.0 + (w - post) * f64::from(i) / spans;
        ctx.cube(
            [x - post / 2.0, x + post / 2.0],
            [0.0, h],
            [-d / 2.0, d / 2.0],
            metal,
        );
    }
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - 5.0, h],
        [-d / 2.0, d / 2.0],
        metal,
    );
    ctx.cube([-w / 2.0, w / 2.0], [0.0, 3.0], [-d / 2.0, d / 2.0], metal);
    ctx.cube(
        [-w / 2.0 + post, w / 2.0 - post],
        [3.0, h - 5.0],
        [-0.5, 0.5],
        c,
    );
}

/// Frameless glass leaves about 60 cm wide between a top and a bottom track.
fn balcony_glazing(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let metal = rgb(METAL);
    ctx.cube([-w / 2.0, w / 2.0], [0.0, 3.0], [-d / 2.0, d / 2.0], metal);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - 4.0, h],
        [-d / 2.0, d / 2.0],
        metal,
    );
    let leaves = (w / 60.0).round().max(1.0);
    let leaf = w / leaves;
    for i in 0..crate::count(leaves) {
        let x0 = -w / 2.0 + leaf * f64::from(i) + 0.2;
        ctx.cube([x0, x0 + leaf - 0.4], [3.0, h - 4.0], [-0.4, 0.4], c);
    }
}

fn fence(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let post = d.clamp(6.0, 12.0);
    let spans = (w / 200.0).ceil().max(1.0);
    for i in 0..=crate::count(spans) {
        let x = -w / 2.0 + post / 2.0 + (w - post) * f64::from(i) / spans;
        ctx.cube(
            [x - post / 2.0, x + post / 2.0],
            [0.0, h],
            [-d / 2.0, d / 2.0],
            shade(c, -0.15),
        );
    }
    let boards = (h / 15.0).floor().max(2.0);
    let pitch = h / boards;
    for i in 0..crate::count(boards) {
        let y = f64::from(i) * pitch + 1.0;
        ctx.cube(
            [-w / 2.0, w / 2.0],
            [y, y + pitch - 2.0],
            [-d / 4.0, d / 4.0],
            c,
        );
    }
}

fn corrugated(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    // Ridges every ~18 cm across the width, running along the depth.
    let waves = (w / 18.0).round().max(2.0);
    let step = w / waves;
    for i in 0..crate::count(waves) {
        let x = -w / 2.0 + f64::from(i) * step;
        let top = [x + step * 0.25, x + step * 0.75];
        ctx.cube(
            [x, x + step],
            [0.0, h * 0.35],
            [-d / 2.0, d / 2.0],
            shade(c, -0.08),
        );
        ctx.cube([top[0], top[1]], [h * 0.35, h], [-d / 2.0, d / 2.0], c);
    }
}

fn glass_panel(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let frame = 2.0_f64.min(w / 10.0);
    ctx.cube(
        [-w / 2.0 + frame, w / 2.0 - frame],
        [frame, h - frame],
        [-d / 2.0, d / 2.0],
        c,
    );
    let metal = rgb(METAL);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, frame],
        [-d / 2.0, d / 2.0],
        metal,
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [h - frame, h],
        [-d / 2.0, d / 2.0],
        metal,
    );
    ctx.cube(
        [-w / 2.0, -w / 2.0 + frame],
        [0.0, h],
        [-d / 2.0, d / 2.0],
        metal,
    );
    ctx.cube(
        [w / 2.0 - frame, w / 2.0],
        [0.0, h],
        [-d / 2.0, d / 2.0],
        metal,
    );
}

fn garage_door(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let jamb = 5.0;
    let panels = (h / 50.0).round().max(3.0);
    let step = h / panels;
    // The panels run between the jambs, ending on their inner faces.
    for i in 0..crate::count(panels) {
        let y = f64::from(i) * step;
        ctx.cube(
            [-w / 2.0 + jamb, w / 2.0 - jamb],
            [y + 0.6, y + step - 0.6],
            [-2.0, 2.0],
            if i % 2 == 0 { c } else { shade(c, -0.05) },
        );
    }
    ctx.cube(
        [-w / 2.0, -w / 2.0 + jamb],
        [0.0, h],
        [-d / 2.0, d / 2.0],
        rgb(DARK),
    );
    ctx.cube(
        [w / 2.0 - jamb, w / 2.0],
        [0.0, h],
        [-d / 2.0, d / 2.0],
        rgb(DARK),
    );
    ctx.handle(0.0, h * 0.2, 2.0, false);
}

fn pool(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let coping = 30.0_f64.min(w / 6.0).min(d / 6.0);
    let stone = rgb([225, 220, 205]);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, h],
        [-d / 2.0, -d / 2.0 + coping],
        stone,
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, h],
        [d / 2.0 - coping, d / 2.0],
        stone,
    );
    ctx.cube(
        [-w / 2.0, -w / 2.0 + coping],
        [0.0, h],
        [-d / 2.0 + coping, d / 2.0 - coping],
        stone,
    );
    ctx.cube(
        [w / 2.0 - coping, w / 2.0],
        [0.0, h],
        [-d / 2.0 + coping, d / 2.0 - coping],
        stone,
    );
    ctx.cube(
        [-w / 2.0 + coping, w / 2.0 - coping],
        [0.0, h * 0.7],
        [-d / 2.0 + coping, d / 2.0 - coping],
        c,
    );
}

fn oval_pool(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let coping = 30.0_f64.min(w / 6.0).min(d / 6.0);
    let stone = rgb([225, 220, 205]);
    let n: u16 = 48;
    #[allow(clippy::cast_possible_truncation)]
    let ring = |rx: f64, rz: f64, y: f64| -> Vec<[f32; 3]> {
        (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * f64::from(i) / f64::from(n);
                [(rx * a.cos()) as f32, y as f32, (rz * a.sin()) as f32]
            })
            .collect()
    };
    let (outer_top, inner_top) = (
        ring(w / 2.0, d / 2.0, h),
        ring(w / 2.0 - coping, d / 2.0 - coping, h),
    );
    let outer_bottom = ring(w / 2.0, d / 2.0, 0.0);
    let water = ring(w / 2.0 - coping, d / 2.0 - coping, h * 0.7);
    let k = usize::from(n);
    for i in 0..k {
        let j = (i + 1) % k;
        // Coping on top, facing up; its outer side facing out; the inner lip.
        ctx.m.polygon(
            &[outer_top[i], inner_top[i], inner_top[j], outer_top[j]],
            stone,
        );
        ctx.m.polygon(
            &[outer_bottom[i], outer_top[i], outer_top[j], outer_bottom[j]],
            stone,
        );
        ctx.m.polygon(
            &[inner_top[i], water[i], water[j], inner_top[j]],
            shade(stone, -0.1),
        );
    }
    #[allow(clippy::cast_possible_truncation)]
    let center = [0.0, (h * 0.7) as f32, 0.0];
    for i in 0..k {
        let j = (i + 1) % k;
        ctx.m.polygon(&[center, water[j], water[i]], c);
    }
}

fn lounger(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let metal = rgb(METAL);
    // Frame on four short legs, lifted 10–15 cm off the deck.
    let clear = (h * 0.16).clamp(10.0, 15.0);
    let rail = 3.5;
    let deck = clear + 4.0;
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        ctx.leg(
            sx * (w / 2.0 - rail / 2.0),
            sz * (d / 2.0 - 6.0),
            clear,
            [3.0, 3.5],
            metal,
        );
    }
    for x0 in [-w / 2.0, w / 2.0 - rail] {
        ctx.cube([x0, x0 + rail], [clear, deck], [-d / 2.0 + 2.0, d / 2.0], c);
    }
    for z in [-d / 2.0 + 4.0, d / 2.0 - 4.0] {
        ctx.cube(
            [-w / 2.0 + rail, w / 2.0 - rail],
            [clear + 0.5, deck - 0.5],
            [z - 1.5, z + 1.5],
            shade(c, -0.1),
        );
    }
    // Seat cushion, then the backrest reclining over the first third.
    let hinge = -d / 2.0 + d * 0.3;
    let x = [-w / 2.0 + 1.0, w / 2.0 - 1.0];
    ctx.cube(
        x,
        [deck, deck + 8.0],
        [hinge, d / 2.0 - 1.5],
        shade(c, 0.04),
    );
    let t = (d * 0.07).clamp(8.0, 14.0);
    prism(
        &mut ctx.m,
        Place::HOME,
        [deck, h],
        [x, [hinge - t, hinge]],
        [x, [-d / 2.0, -d / 2.0 + t]],
        shade(c, 0.1),
    );
    // Two struts prop the backrest from the side rails.
    let zs = -d / 2.0 + d * 0.1;
    let under = |z: f64| deck + (h - deck) * (hinge - t - z) / (hinge - t + d / 2.0);
    for sx in [-1.0, 1.0] {
        let xs = sx * (w / 2.0 - rail / 2.0);
        ctx.cube(
            [xs - 1.2, xs + 1.2],
            [deck, under(zs)],
            [zs - 1.2, zs + 1.2],
            metal,
        );
    }
}

fn grill(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let counter = h * 0.4;
    ctx.cube([-w / 2.0, w / 2.0], [0.0, counter], [-d / 2.0, d / 2.0], c);
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [counter, counter + 4.0],
        [-d / 2.0, d / 2.0],
        rgb([60, 60, 62]),
    );
    // Firebox opening and chimney hood.
    ctx.cube(
        [-w * 0.35, w * 0.35],
        [counter + 4.0, counter + 40.0],
        [-d / 2.0, -d / 2.0 + 10.0],
        c,
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [counter + 40.0, counter + 70.0],
        [-d / 2.0, d / 2.0],
        shade(c, -0.1),
    );
    ctx.cube(
        [-w * 0.2, w * 0.2],
        [counter + 70.0, h],
        [-d * 0.3, d * 0.2],
        c,
    );
}

fn sofa_l(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let seat_d = (d * 0.55).min(95.0);
    let back = 22.0_f64.min(seat_d * 0.25);
    let arm = 18.0_f64.min(w * 0.08);
    let seat = h * 0.5;
    let chaise = (w * 0.3).max(70.0).min(w * 0.5);
    // Main seat along the back.
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, seat],
        [-d / 2.0, -d / 2.0 + seat_d],
        shade(c, -0.1),
    );
    ctx.cube(
        [-w / 2.0, w / 2.0],
        [0.0, h],
        [-d / 2.0, -d / 2.0 + back],
        c,
    );
    ctx.cube(
        [-w / 2.0, -w / 2.0 + arm],
        [0.0, h * 0.68],
        [-d / 2.0, -d / 2.0 + seat_d],
        c,
    );
    // Chaise running forward on the right.
    ctx.cube(
        [w / 2.0 - chaise, w / 2.0],
        [0.0, seat],
        [-d / 2.0 + seat_d, d / 2.0],
        shade(c, -0.1),
    );
    ctx.cube(
        [w / 2.0 - arm, w / 2.0],
        [0.0, h * 0.68],
        [-d / 2.0, d / 2.0],
        c,
    );
}

fn bench(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    ctx.cube([-w / 2.0, w / 2.0], [h - 5.0, h], [-d / 2.0, d / 2.0], c);
    for x in [-w / 2.0 + 8.0, w / 2.0 - 14.0] {
        ctx.cube(
            [x, x + 6.0],
            [0.0, h - 5.0],
            [-d / 2.0 + 4.0, d / 2.0 - 4.0],
            shade(c, -0.2),
        );
    }
}

fn dining_set(ctx: &mut Ctx, chairs: u8) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let chair = 45.0_f64.min(d * 0.3);
    // The table spans the full width: chairs only sit along the long sides,
    // and the old `w - 2·0.6·chair` top was stretched to `w` by `fit_to`
    // anyway, so this keeps the rendered footprint without distorting it.
    let (tw, td) = (w, d - 2.0 * chair);
    let top = 75.0_f64.min(h * 0.85);
    table_frame(ctx, [0.0, 0.0], [tw, td], top, c);
    let per_side = u32::from(chairs.max(2)) / 2;
    let seat = 45.0_f64.min(top * 0.6);
    let fabric = rgb([150, 140, 125]);
    // Every chair is tucked the same few cm under the table edge, back out.
    let tuck = 6.0_f64.min(td / 4.0);
    let cd = chair + tuck;
    let slot = tw / f64::from(per_side);
    let cw = chair.min(slot * 0.8);
    for side in [-1.0, 1.0] {
        for i in 0..per_side {
            let place = Place {
                x: -tw / 2.0 + slot * (f64::from(i) + 0.5),
                z: side * (d / 2.0 - cd / 2.0),
                // Back towards the outside: the +z row faces −z.
                turned: side > 0.0,
            };
            chair_at(&mut ctx.m, place, [cw, cd, h], seat, c, fabric);
        }
    }
}

fn planter(ctx: &mut Ctx) {
    let (w, d, h, c) = (ctx.w, ctx.d, ctx.h, ctx.c);
    let soil = h * 0.55;
    ctx.cube([-w / 2.0, w / 2.0], [0.0, soil], [-d / 2.0, d / 2.0], c);
    let green = rgb([70, 140, 80]);
    let clumps = (w / 30.0).round().max(1.0);
    for i in 0..crate::count(clumps) {
        let x = -w / 2.0 + w * (f64::from(i) + 0.5) / clumps;
        ctx.m.frustum(
            [x, soil, 0.0],
            Axis::Y,
            h - soil,
            (w / clumps / 2.0).min(d / 2.0),
            2.0,
            10,
            shade(green, if i % 2 == 0 { 0.0 } else { 0.1 }),
        );
    }
}

/// Places a 2D point of a solid's ring at sweep position 0 or 1.
type Lift<'a> = Box<dyn Fn([f64; 2], f64) -> [f32; 3] + 'a>;

/// A polygon swept into a closed solid: plan outlines go up by the height,
/// profiles run along the depth. Points are centered on the piece.
pub(crate) fn solid(shape: &newera_core::SolidShape, piece: &Furniture, color: Rgb) -> Mesh {
    use crate::mesh::shade;
    use newera_core::{Point2, SolidShape};
    let mut m = Mesh::default();
    // `ring` in its own 2D plane, `lift(p, t)` puts point p at sweep t ∈ {0, 1}.
    let (ring, lift): (Vec<[f64; 2]>, Lift<'_>) = match shape {
        SolidShape::Outline(points) => (
            points.clone(),
            #[allow(clippy::cast_possible_truncation)]
            Box::new(move |p, t| [p[0] as f32, (t * piece.height) as f32, p[1] as f32]),
        ),
        SolidShape::Profile(points) => (
            points.clone(),
            #[allow(clippy::cast_possible_truncation)]
            Box::new(move |p, t| [p[0] as f32, p[1] as f32, ((t - 0.5) * piece.depth) as f32]),
        ),
    };
    if ring.len() < 3 {
        return m;
    }
    let as_points: Vec<Point2> = ring.iter().map(|p| Point2::new(p[0], p[1])).collect();
    // Counter-clockwise in its plane so caps and sides face out.
    let ccw = newera_core::signed_area(&as_points) > 0.0;
    let flip_outline = matches!(shape, SolidShape::Outline(_));
    let tris = newera_core::triangulate(&as_points);
    let cap = |m: &mut Mesh, t: f64, up: bool| {
        for [a, b, c] in &tris {
            let (a, b, c) = (lift(ring[*a], t), lift(ring[*b], t), lift(ring[*c], t));
            // Outlines map plan y to z, which mirrors their winding.
            if up == (ccw != flip_outline) {
                m.polygon(&[a, b, c], color);
            } else {
                m.polygon(&[a, c, b], color);
            }
        }
    };
    cap(&mut m, 0.0, false);
    cap(&mut m, 1.0, true);
    let n = ring.len();
    for i in 0..n {
        let (p, q) = (ring[i], ring[(i + 1) % n]);
        let quad = [lift(p, 0.0), lift(q, 0.0), lift(q, 1.0), lift(p, 1.0)];
        if ccw == flip_outline {
            m.polygon(&[quad[3], quad[2], quad[1], quad[0]], shade(color, -0.05));
        } else {
            m.polygon(&quad, shade(color, -0.05));
        }
    }
    m.fit_to(piece.width, piece.depth, piece.height);
    m
}

#[cfg(test)]
mod solid_tests {
    use newera_core::{Furniture, SolidShape};

    use crate::mesh::Rgb;

    // First surface seen from above, including opaque faces that would hide
    // the bowl. Checking only for porcelain vertices missed the original bug.
    fn top_hit(mesh: &crate::Mesh, x: f32, z: f32) -> (f32, Rgb, f32) {
        mesh.indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|tri| {
                let [a, b, c] = [0, 1, 2].map(|i| mesh.positions[tri[i] as usize]);
                let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
                if det.abs() < 1e-6 {
                    return None;
                }
                let u = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
                let v = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
                let t = 1.0 - u - v;
                if u < -1e-5 || v < -1e-5 || t < -1e-5 {
                    return None;
                }
                Some((
                    u * a[1] + v * b[1] + t * c[1],
                    mesh.colors[tri[0] as usize],
                    mesh.normals[tri[0] as usize][1],
                ))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .expect("surface under sample")
    }

    #[test]
    fn basin_has_an_exposed_recess_and_preserves_countertop_height_and_cabinet_color() {
        for width in [60.0, 100.0, 120.0] {
            let piece = Furniture {
                width,
                depth: 48.0,
                height: 85.0,
                ..Furniture::default()
            };
            let brown = crate::mesh::rgb([110, 70, 45]);
            let mesh = super::build(crate::Model::Basin, &piece, brown);
            for (x, z) in [(5.0, 0.0), (-5.0, 0.0), (0.0, 10.0), (0.0, -10.0)] {
                let (height, color, normal_y) = top_hit(&mesh, x, z);
                assert!(
                    height < 83.0 && height > 65.0,
                    "occluded/shallow bowl at {x},{z}: {height}"
                );
                assert!(
                    color
                        .iter()
                        .zip(crate::mesh::rgb([235, 237, 239]))
                        .all(|(a, b)| (a - b).abs() < 1e-6)
                );
                assert!(normal_y > 0.1, "bowl faces must face the opening");
            }
            #[allow(clippy::cast_possible_truncation)]
            let (height, _, _) = top_hit(&mesh, (width * 0.45) as f32, 0.0);
            assert!((height - 85.0).abs() < 1e-4);
            assert!(mesh.colors.contains(&brown));
            assert!(mesh.positions.iter().all(|p| p[1] <= 85.0 && p[1] >= 0.0));
        }
    }

    fn faces_out(mesh: &crate::Mesh) {
        #[allow(clippy::cast_precision_loss)]
        let n = mesh.positions.len() as f32;
        let center = mesh.positions.iter().fold([0.0f32; 3], |c, p| {
            [c[0] + p[0] / n, c[1] + p[1] / n, c[2] + p[2] / n]
        });
        for tri in mesh.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| mesh.positions[tri[k] as usize]);
            let normal = mesh.normals[tri[0] as usize];
            let mid = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let out = [mid[0] - center[0], mid[1] - center[1], mid[2] - center[2]];
            let dot = out[0] * normal[0] + out[1] * normal[1] + out[2] * normal[2];
            assert!(dot > -1e-3, "face points inward: {tri:?}");
        }
    }

    #[test]
    fn outlines_and_profiles_are_closed_outward_solids() {
        let grey: Rgb = [0.5; 3];
        for points in [
            vec![
                [-100.0, -50.0],
                [100.0, -50.0],
                [100.0, 50.0],
                [-100.0, 50.0],
            ],
            vec![
                [-100.0, -50.0],
                [-100.0, 50.0],
                [100.0, 50.0],
                [100.0, -50.0],
            ],
        ] {
            let piece = Furniture {
                width: 200.0,
                depth: 100.0,
                height: 20.0,
                ..Furniture::default()
            };
            let mesh = super::solid(&SolidShape::Outline(points), &piece, grey);
            assert_eq!(mesh.indices.len() / 3, 2 + 2 + 8);
            faces_out(&mesh);
        }
        // A 600 cm wide, 675 cm high gable swept 20 cm.
        for points in [
            vec![[-300.0, 0.0], [300.0, 0.0], [0.0, 675.0]],
            vec![[-300.0, 0.0], [0.0, 675.0], [300.0, 0.0]],
        ] {
            let piece = Furniture {
                width: 600.0,
                depth: 20.0,
                height: 675.0,
                ..Furniture::default()
            };
            let mesh = super::solid(&SolidShape::Profile(points), &piece, grey);
            faces_out(&mesh);
            let top = mesh.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
            assert!((top - 675.0).abs() < 1e-3);
        }
    }
}

#[cfg(test)]
mod frame_tests {
    use newera_core::{Furniture, Opening, OpeningKind};

    type Triangle = [[f32; 3]; 3];

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    /// Twice the signed area of a polygon.
    fn area2(poly: &[[f32; 2]]) -> f32 {
        (0..poly.len())
            .map(|i| {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum()
    }

    /// The part of `poly` inside the counter-clockwise triangle `clip`.
    fn clipped(poly: Vec<[f32; 2]>, clip: [[f32; 2]; 3]) -> Vec<[f32; 2]> {
        let mut out = poly;
        for i in 0..3 {
            let (a, b) = (clip[i], clip[(i + 1) % 3]);
            let side = |p: [f32; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
            let input = std::mem::take(&mut out);
            for j in 0..input.len() {
                let (p, q) = (input[j], input[(j + 1) % input.len()]);
                let (sp, sq) = (side(p), side(q));
                if sp >= 0.0 {
                    out.push(p);
                }
                if (sp >= 0.0) != (sq >= 0.0) {
                    let t = sp / (sp - sq);
                    out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
                }
            }
            if out.is_empty() {
                break;
            }
        }
        out
    }

    /// Pairs of triangles lying in one plane, facing the same way, whose
    /// areas overlap by more than 0.001 cm²: a path tracer that starts a ray
    /// on one finds the other at once, and the surface comes out black.
    fn overlapping_faces(mesh: &crate::Mesh) -> Vec<(Triangle, Triangle)> {
        let tris: Vec<(Triangle, [f32; 3])> = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|t| {
                let v = t.map(|i| mesh.positions[i as usize]);
                let n = cross(sub(v[1], v[0]), sub(v[2], v[0]));
                let len = dot(n, n).sqrt();
                (len > 1e-6).then(|| (v, n.map(|c| c / len)))
            })
            .collect();
        let mut found = Vec::new();
        for (i, (a, n)) in tris.iter().enumerate() {
            // Onto the plane of the two axes the normal leans on least.
            let drop = (0..3)
                .max_by(|&x, &y| n[x].abs().total_cmp(&n[y].abs()))
                .expect("three axes");
            let (u, v) = ((drop + 1) % 3, (drop + 2) % 3);
            let flat = |t: &Triangle| {
                let f = t.map(|p| [p[u], p[v]]);
                if area2(&f) < 0.0 {
                    [f[0], f[2], f[1]]
                } else {
                    f
                }
            };
            for (b, m) in &tris[i + 1..] {
                if dot(*n, *m) < 0.999 || dot(sub(b[0], a[0]), *n).abs() > 1e-3 {
                    continue;
                }
                let shared = clipped(flat(a).to_vec(), flat(b));
                if shared.len() >= 3 && area2(&shared).abs() / 2.0 > 1e-3 {
                    found.push((*a, *b));
                }
            }
        }
        found
    }

    fn opening(kind: OpeningKind, leaves: u8, sliding: bool) -> Opening {
        Opening {
            kind,
            leaves,
            sliding,
            ..Opening::default()
        }
    }

    #[test]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "unit normals and plane offsets of whole centimetres"
    )]
    fn overlapping_faces_are_found_exactly() {
        let white = crate::mesh::rgb([245, 245, 242]);
        let mut two = crate::Mesh::default();
        two.cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 1.0], white);
        two.cuboid([9.9, 0.0, 0.0], [20.0, 10.0, 1.0], white);
        let mut planes: Vec<[i32; 4]> = overlapping_faces(&two)
            .iter()
            .map(|(a, _)| {
                let n = cross(sub(a[1], a[0]), sub(a[2], a[0]));
                let n = n.map(|c| if c.abs() < 1e-6 { 0 } else { c.signum() as i32 });
                [n[0], n[1], n[2], dot(n.map(|c| c as f32), a[0]) as i32]
            })
            .collect();
        planes.sort_unstable();
        planes.dedup();
        assert_eq!(
            planes,
            [[0, -1, 0, 0], [0, 0, -1, 0], [0, 0, 1, 1], [0, 1, 0, 10]],
            "bottom, back, front and top overlap; the touching ends do not"
        );
        let mut touching = crate::Mesh::default();
        touching.cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 1.0], white);
        touching.cuboid([10.0, 0.0, 0.0], [20.0, 10.0, 1.0], white);
        assert_eq!(overlapping_faces(&touching), []);
    }

    #[test]
    fn opening_frames_have_no_faces_lying_on_each_other() {
        let white = crate::mesh::rgb([245, 245, 242]);
        let cases = [
            (
                crate::Model::Window { panes: 1 },
                Some(opening(OpeningKind::Window, 1, false)),
            ),
            (
                crate::Model::Window { panes: 2 },
                Some(opening(OpeningKind::Window, 1, false)),
            ),
            (
                crate::Model::Window { panes: 3 },
                Some(opening(OpeningKind::Window, 1, false)),
            ),
            (
                crate::Model::Door,
                Some(opening(OpeningKind::Door, 1, false)),
            ),
            (
                crate::Model::Door,
                Some(opening(OpeningKind::Door, 2, false)),
            ),
            (
                crate::Model::Door,
                Some(opening(OpeningKind::Door, 1, true)),
            ),
            (
                crate::Model::Door,
                Some(opening(OpeningKind::Door, 2, true)),
            ),
            (
                crate::Model::Window { panes: 4 },
                Some(opening(OpeningKind::Window, 1, false)),
            ),
            (crate::Model::Passage, None),
        ];
        let sized = cases
            .into_iter()
            .map(|(model, opening)| (model, opening, [150.0, 210.0]))
            .chain([[150.0, 210.0], [80.0, 120.0], [240.0, 250.0]].map(|size| {
                (
                    crate::Model::GarageDoor,
                    Some(opening(OpeningKind::Door, 1, false)),
                    size,
                )
            }));
        for (model, opening, [width, height]) in sized {
            let piece = Furniture {
                width,
                depth: 15.0,
                height,
                opening: opening.clone(),
                ..Furniture::default()
            };
            let mesh = super::build(model, &piece, white);
            assert_eq!(
                overlapping_faces(&mesh),
                [],
                "{model:?} {width}x{height} {opening:?}"
            );
        }
    }
}
