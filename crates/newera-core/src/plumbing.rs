//! The plumbing project as an installer reads it: the water and sewer points
//! each fixture needs, where the supply comes from and where the sewer goes,
//! and what a branch takes to build.
//!
//! A point is a piece of the plumbing project and is what its catalog entry
//! says (`cold-water`, `sewer`, `floor-drain`…), never what it is named after.
//! A fixture is the piece it serves: a toilet, a basin, a sink. NBR 5626 and
//! NBR 8160 are paid standards we hold only in part, so the rules check that
//! a thing exists and is reachable, and the figures they use — branch
//! diameters, slopes — are the ones every installer's table repeats, said as
//! such.

use serde::Serialize;

use crate::electrical::{Finding, Material, Severity, inside, metres, reaches};
use crate::elements::Room;
use crate::furniture::Furniture;
use crate::geometry::Point2;
use crate::home::Home;
use crate::ids::{FurnitureId, RoomId};
use crate::routing::{Route, Terminal, Via};
use crate::say;
use crate::style::Polyline;

/// What a pipe run carries.
pub const PIPE_KEY: &str = "plumb:pipe";
/// The run a laid-out pipe belongs to, so laying it again replaces it.
pub const RUN_KEY: &str = "plumb:run";
/// The run's real length, drops included, cm.
pub const RUN_CM_KEY: &str = "plumb:run_cm";
/// The points a laid-out run serves, ids joined by commas: reached by it
/// whatever the distance from the wall's axis it runs in.
pub const ENDS_KEY: &str = "plumb:ends";

/// Whether a line of the plan serves a point: it lists it, or passes by it.
pub fn serves_point(line: &Polyline, id: crate::ids::FurnitureId, at: Point2) -> bool {
    line.properties
        .get(ENDS_KEY)
        .is_some_and(|ends| ends.split(',').any(|e| e == id.to_string()))
        || reaches(line, at)
}

/// What a plumbing point is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum PointKind {
    Cold,
    Hot,
    Sewer,
    Drain,
    Valve,
    GreaseTrap,
    InspectionBox,
    Vent,
    /// A rainwater drain: goes to the rainwater system, never the sewer.
    RainDrain,
    WaterMeter,
    Gas,
}

impl PointKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Cold => "Água fria",
            Self::Hot => "Água quente",
            Self::Sewer => "Esgoto",
            Self::Drain => "Ralo",
            Self::Valve => "Registro",
            Self::GreaseTrap => "Caixa de gordura",
            Self::InspectionBox => "Caixa de inspeção",
            Self::Vent => "Ventilação",
            Self::RainDrain => "Ralo pluvial",
            Self::WaterMeter => "Hidrômetro",
            Self::Gas => "Gás",
        }
    }

    fn of(catalog: &str) -> Option<Self> {
        Some(match catalog {
            "cold-water" => Self::Cold,
            "hot-water" => Self::Hot,
            "sewer" => Self::Sewer,
            "floor-drain" | "floor-drain-100" | "floor-drain-75" | "trap-drain-small"
            | "dry-drain" | "linear-drain" | "linear-drain-trap" => Self::Drain,
            "rain-drain" => Self::RainDrain,
            "valve" => Self::Valve,
            "grease-trap" => Self::GreaseTrap,
            "inspection-box" => Self::InspectionBox,
            "vent-pipe" => Self::Vent,
            "water-meter" => Self::WaterMeter,
            "gas-point" => Self::Gas,
            _ => return None,
        })
    }
}

/// A point of the project, located.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Point {
    pub id: FurnitureId,
    pub kind: PointKind,
    pub name: String,
    pub room: Option<RoomId>,
    pub at: Point2,
    /// Height of its centre above the floor, cm.
    pub z: f64,
}

/// A piece that uses water.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Fixture {
    Toilet,
    Basin,
    KitchenSink,
    Shower,
    Bathtub,
    Washer,
    LaundrySink,
    Dishwasher,
    Bidet,
}

impl Fixture {
    pub fn name(self) -> &'static str {
        match self {
            Self::Toilet => "vaso sanitário",
            Self::Basin => "lavatório",
            Self::KitchenSink => "pia de cozinha",
            Self::Shower => "chuveiro",
            Self::Bathtub => "banheira",
            Self::Washer => "máquina de lavar",
            Self::LaundrySink => "tanque",
            Self::Dishwasher => "lava-louças",
            Self::Bidet => "bidê",
        }
    }

    /// Its name as a word of a sentence, translated with it.
    #[must_use]
    pub fn said(self) -> crate::text::Text {
        crate::text::Text::from(self.name())
    }

    /// Its name with its article, to start a sentence with: "a pia de
    /// cozinha", never "o pia de cozinha".
    #[must_use]
    pub fn the(self) -> crate::text::Text {
        crate::text::Text::from(match self {
            Self::Toilet => "o vaso sanitário",
            Self::Basin => "o lavatório",
            Self::KitchenSink => "a pia de cozinha",
            Self::Shower => "o chuveiro",
            Self::Bathtub => "a banheira",
            Self::Washer => "a máquina de lavar",
            Self::LaundrySink => "o tanque",
            Self::Dishwasher => "a lava-louças",
            Self::Bidet => "o bidê",
        })
    }

    /// Whether it takes hot water where the home has it.
    fn takes_hot(self) -> bool {
        matches!(
            self,
            Self::Basin | Self::KitchenSink | Self::Shower | Self::Bathtub | Self::Bidet
        )
    }

    /// Whether a floor drain (a trap box) may take its waste instead of a
    /// sewer point of its own: basins, bidets, tubs and showers of the same
    /// unit (NBR 8160 4.2.2.3), and a laundry sink, whose 40 mm branch fits
    /// the box's inlets. A washing machine's 50 mm branch does not: it takes
    /// its own point with a trap (4.2.2.6, 5.1.1.1 b).
    fn drains_to_floor(self) -> bool {
        matches!(
            self,
            Self::Basin | Self::Shower | Self::Bathtub | Self::LaundrySink | Self::Bidet
        )
    }

    /// Hunter's contribution units, NBR 8160 table 3.
    pub fn uhc(self) -> u32 {
        match self {
            Self::Toilet => 6,
            Self::KitchenSink | Self::Washer | Self::LaundrySink => 3,
            Self::Shower | Self::Bathtub | Self::Dishwasher => 2,
            Self::Basin | Self::Bidet => 1,
        }
    }

    /// The smallest discharge branch, mm, NBR 8160 table 3: 100 for a
    /// toilet, 50 for a kitchen sink or a machine, 40 for the rest.
    pub fn sewer_mm(self) -> u32 {
        match self {
            Self::Toilet => 100,
            Self::KitchenSink | Self::Washer | Self::Dishwasher => 50,
            Self::Basin | Self::Shower | Self::Bathtub | Self::LaundrySink | Self::Bidet => 40,
        }
    }

    /// What a catalog item is made as, whatever it is called.
    fn by_catalog(catalog: &str) -> Option<Self> {
        match catalog {
            "toilet" => Some(Self::Toilet),
            "basin-cabinet" => Some(Self::Basin),
            "sink-counter" => Some(Self::KitchenSink),
            "shower" | "shower-glass" => Some(Self::Shower),
            "bathtub" => Some(Self::Bathtub),
            "washer" => Some(Self::Washer),
            "laundry-sink" => Some(Self::LaundrySink),
            "dishwasher" => Some(Self::Dishwasher),
            _ => None,
        }
    }

    fn of(piece: &Furniture) -> Option<Self> {
        if piece.discipline.is_some() {
            return None;
        }
        let by_catalog = Self::by_catalog(&piece.catalog);
        if by_catalog.is_some() {
            return by_catalog;
        }
        // By name only a piece of a fixture's size whose name starts with it:
        // "Pia centralizada", "10 — Vaso suíte" — never "Gavetão pia —
        // moldura", a part of the cabinet under it.
        if piece.width.min(piece.depth) < 25.0 {
            return None;
        }
        let folded = crate::annotations::fold(&piece.name);
        let name = folded.trim_start_matches(|c: char| {
            c.is_ascii_digit() || c.is_whitespace() || matches!(c, '-' | '.' | ':' | '—' | '–')
        });
        let starts = |words: &[&str]| words.iter().any(|w| name.starts_with(w));
        if starts(&["vaso", "bacia sanitaria"]) {
            Some(Self::Toilet)
        } else if starts(&["lavatorio", "cuba"]) {
            Some(Self::Basin)
        } else if starts(&["pia"]) {
            Some(Self::KitchenSink)
        } else if starts(&["chuveiro", "ducha", "box"]) {
            Some(Self::Shower)
        } else if starts(&["banheira"]) {
            Some(Self::Bathtub)
        } else if starts(&["lava-loucas", "lava loucas", "lava louca"]) {
            Some(Self::Dishwasher)
        } else if starts(&["maquina de lavar", "lava e seca", "lava-e-seca", "lavadora"]) {
            Some(Self::Washer)
        } else if starts(&["bide"]) {
            Some(Self::Bidet)
        } else if starts(&["tanque"]) {
            Some(Self::LaundrySink)
        } else {
            None
        }
    }
}

/// Whether a piece heats water: a heater, a boiler, by catalog or name.
pub fn is_heater(piece: &Furniture) -> bool {
    let name = crate::annotations::fold(&piece.name);
    piece.catalog.contains("heater")
        || ["aquecedor", "boiler", "caldeira"]
            .iter()
            .any(|w| name.contains(w))
}

/// What a pipe carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Pipe {
    Cold,
    Hot,
    Sewer,
    /// A vent branch: from the traps up to a vent stack.
    Vent,
}

impl Pipe {
    pub const ALL: [Self; 4] = [Self::Cold, Self::Hot, Self::Sewer, Self::Vent];

    pub fn key(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Hot => "hot",
            Self::Sewer => "sewer",
            Self::Vent => "vent",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == raw.trim())
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Cold => "Água fria",
            Self::Hot => "Água quente",
            Self::Sewer => "Esgoto",
            Self::Vent => "Ventilação",
        }
    }

    /// Drawn as plumbing drawings are: cold water blue, hot red dashed, sewer
    /// brown.
    pub fn style(self) -> (crate::style::DashStyle, [u8; 3]) {
        match self {
            Self::Cold => (crate::style::DashStyle::Solid, [40, 110, 210]),
            Self::Hot => (crate::style::DashStyle::Dash, [210, 60, 40]),
            Self::Sewer => (crate::style::DashStyle::Solid, [120, 90, 60]),
            Self::Vent => (crate::style::DashStyle::DashDot, [90, 140, 90]),
        }
    }

    /// The points a run of it ends at.
    pub fn serves(self, kind: PointKind) -> bool {
        match self {
            Self::Cold => kind == PointKind::Cold,
            Self::Hot => kind == PointKind::Hot,
            Self::Sewer | Self::Vent => matches!(kind, PointKind::Sewer | PointKind::Drain),
        }
    }
}

/// The pipe a line of the plan is, when it says.
pub fn pipe_of(line: &Polyline) -> Option<Pipe> {
    Pipe::parse(line.properties.get(PIPE_KEY)?)
}

/// Every plumbing point on the storey shown.
pub fn points(home: &Home) -> Vec<Point> {
    let view = home.level_view(home.current_level());
    let mut out: Vec<Point> = view
        .furniture
        .iter()
        .flat_map(Furniture::flatten)
        .filter_map(|f| {
            let kind = PointKind::of(&f.catalog)?;
            Some(Point {
                id: f.id,
                kind,
                name: f.name.clone(),
                room: view.room_at(f.position, view.current_level()).map(|r| r.id),
                at: f.position,
                z: f.elevation + f.height / 2.0,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (a.at.y, a.at.x)
            .partial_cmp(&(b.at.y, b.at.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// A group that is a fixture is one; its parts are looked at only when it
/// is not. Two pieces of one fixture — the sink and its cabinet — count once.
fn walk(piece: &Furniture, out: &mut Vec<(Fixture, Furniture)>) {
    match Fixture::of(piece) {
        Some(x) => {
            let twin = out
                .iter()
                .any(|(y, f)| *y == x && f.position.distance(piece.position) <= 40.0);
            if !twin {
                out.push((x, piece.clone()));
            }
        }
        None => {
            for child in &piece.children {
                walk(child, out);
            }
        }
    }
}

/// Every fixture on the storey shown, with the piece it is.
pub fn fixtures(home: &Home) -> Vec<(Fixture, Furniture)> {
    use crate::vocabulary::Mention as M;
    let view = home.level_view(home.current_level());
    let mut out = Vec::new();
    for piece in &view.furniture {
        walk(piece, &mut out);
    }
    // A bowl — "cuba", or the catalog's inset bowl, whatever it is called —
    // is what the room makes it: washed at in a bathroom, cooked beside in a
    // kitchen (a kitchen sink's branch, its fat trap), a laundry sink in the
    // laundry. One named or made as a basin stays one.
    for (fixture, piece) in &mut out {
        if !matches!(*fixture, Fixture::Basin | Fixture::KitchenSink) || !is_bowl(piece) {
            continue;
        }
        let Some(room) = view.room_at(piece.position, view.current_level()) else {
            continue;
        };
        let says = room.mentions();
        *fixture = if says.any(&[M::Bathroom, M::Lavatory]) {
            Fixture::Basin
        } else if says.any(&[M::Kitchen, M::Pantry, M::Gourmet]) {
            Fixture::KitchenSink
        } else if says.has(M::Laundry) {
            Fixture::LaundrySink
        } else {
            Fixture::Basin
        };
    }
    // Read again, two drawings of one bowl ("Pia" and "Cuba") are one fixture.
    let mut kept: Vec<(Fixture, Furniture)> = Vec::new();
    for (fixture, piece) in out {
        let twin = kept
            .iter()
            .any(|(k, f)| *k == fixture && f.position.distance(piece.position) <= 40.0);
        if !twin {
            kept.push((fixture, piece));
        }
    }
    kept
}

/// A bowl called by what it is, not by what it is for: a "cuba", or the
/// catalog's inset bowl unless it is named a basin. A catalog fixture —
/// a basin cabinet called "Cuba" — is what it was made as.
fn is_bowl(piece: &Furniture) -> bool {
    let name = crate::annotations::fold(&piece.name);
    !name.starts_with("lavatorio")
        && (piece.catalog == "sink-bowl"
            || (Fixture::by_catalog(&piece.catalog).is_none() && name.starts_with("cuba")))
}

/// How far from a fixture's centre its points may stand: half its size and
/// the length of a flexible hose or a short stub.
fn reach_of(piece: &Furniture) -> f64 {
    piece.width.max(piece.depth) / 2.0 + 60.0
}

/// The fixture a point serves: the nearest one in reach.
pub fn served_by(home: &Home, at: Point2) -> Option<Fixture> {
    fixtures(home)
        .into_iter()
        .filter(|(_, f)| f.position.distance(at) <= reach_of(f))
        .min_by(|a, b| {
            a.1.position
                .distance(at)
                .total_cmp(&b.1.position.distance(at))
        })
        .map(|(x, _)| x)
}

/// What a floor drain of the catalog is, from its makers' sheets (Tigre,
/// Wavin/Amanco, Krona) and NBR 8160.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct DrainSpec {
    /// Water seal, mm (0: none). A trap needs at least 50 (8160 5.1.1.1 a).
    pub seal_mm: u32,
    /// Outlet, mm.
    pub outlet_mm: u32,
    /// Most UHC it takes: its body (DN100 6, DN150 15, 5.1.1.2) and its outlet
    /// (DN40 3, DN50 6, DN75 20, table 5), whichever is less.
    pub max_uhc: u32,
    /// Height of its body under the finished floor, cm.
    pub depth_cm: f64,
    /// What to buy.
    pub item: &'static str,
}

impl DrainSpec {
    /// Whether it is a trap (desconector) by itself.
    pub fn is_trap(self) -> bool {
        self.seal_mm >= 50
    }
}

/// The spec of a floor drain by catalog id.
pub fn drain_spec(catalog: &str) -> Option<DrainSpec> {
    let spec = |seal_mm, outlet_mm, max_uhc, depth_cm, item| DrainSpec {
        seal_mm,
        outlet_mm,
        max_uhc,
        depth_cm,
        item,
    };
    Some(match catalog {
        "floor-drain" => spec(
            50,
            50,
            6,
            15.5,
            "Caixa sifonada 150×150×50 com grelha (7 entradas de 40 mm)",
        ),
        "floor-drain-100" => spec(50, 50, 6, 15.5, "Caixa sifonada 100×150×50 com grelha"),
        "floor-drain-75" => spec(
            50,
            75,
            15,
            18.5,
            "Caixa sifonada 150×185×75 com grelha (5 entradas de 40 mm)",
        ),
        "trap-drain-small" => spec(
            20,
            40,
            2,
            5.5,
            "Ralo sifonado 100 mm (fecho de 9 a 20 mm, não é desconector)",
        ),
        "dry-drain" => spec(0, 40, 2, 5.5, "Ralo seco 100 mm, saída 40"),
        "linear-drain" => spec(0, 40, 2, 4.2, "Ralo linear sem sifão, saída 40"),
        "linear-drain-trap" => spec(
            50,
            50,
            3,
            6.0,
            "Ralo linear sifonado, saída 50 (fecho de 50 mm)",
        ),
        _ => return None,
    })
}

/// The UHC a floor drain's trap box receives: the fixtures of its room that
/// may drain into it.
pub fn drain_uhc(home: &Home, point: &Point) -> u32 {
    let view = home.level_view(home.current_level());
    let Some(room) = point
        .room
        .and_then(|id| view.rooms.iter().find(|r| r.id == id))
    else {
        return 0;
    };
    fixtures(home)
        .iter()
        .filter(|(x, f)| x.drains_to_floor() && inside(&room.points, f.position))
        .map(|(x, _)| x.uhc())
        .sum()
}

/// The discharge diameter a sewer point takes, mm: its fixture's; for a
/// floor drain, the outlet of its model; and 50 when nothing says.
pub fn sewer_mm(home: &Home, point: &Point) -> u32 {
    if point.kind == PointKind::Drain {
        let catalog = home
            .level_view(home.current_level())
            .find_piece(point.id)
            .map(|f| f.catalog.clone())
            .unwrap_or_default();
        return drain_spec(&catalog).map_or(50, |d| d.outlet_mm);
    }
    let name = crate::annotations::fold(&point.name);
    if name.contains("vaso") || name.contains("bacia") {
        return 100;
    }
    served_by(home, point.at).map_or(50, Fixture::sewer_mm)
}

/// Minimum slope NBR 8160 4.2.3.2 recommends for a horizontal sewer pipe,
/// per unit: 2 % up to 75 mm, 1 % from 100.
pub fn slope(mm: u32) -> f64 {
    if mm <= 75 { 0.02 } else { 0.01 }
}

fn place(room: Option<&Room>, piece: &Furniture) -> String {
    match room {
        Some(r) if !r.name.trim().is_empty() => format!("{} {} ({})", piece.name, piece.id, r.name),
        _ => format!("{} {}", piece.name, piece.id),
    }
}

/// What each fixture and the project as a whole lack, weighed at the place
/// the project is — see [`crate::electrical::weighed`].
pub fn check(home: &Home) -> Vec<Finding> {
    crate::electrical::weighed(home, found(home))
}

#[allow(clippy::too_many_lines)]
fn found(home: &Home) -> Vec<Finding> {
    let view = home.level_view(home.current_level());
    let all = points(home);
    let fixtures = fixtures(home);
    let mut out = Vec::new();
    let near = |piece: &Furniture, kinds: &[PointKind]| {
        all.iter()
            .any(|p| kinds.contains(&p.kind) && p.at.distance(piece.position) <= reach_of(piece))
    };
    let has = |kind: PointKind| all.iter().any(|p| p.kind == kind);
    let hot_system = has(PointKind::Hot);
    for (fixture, piece) in &fixtures {
        let room = view.room_at(piece.position, view.current_level());
        let at = place(room, piece);
        let reach = reach_of(piece).round();
        if !near(piece, &[PointKind::Cold]) {
            out.push(Finding {
                key: format!("plumb:cold:{}", piece.id),
                accepted: None,
                severity: Severity::Erro,
                place: at.clone(),
                message: say!(
                    "Sem ponto de água fria a até {} cm: {} não tem de onde receber água.",
                    reach,
                    fixture.the()
                ),
                source: "nbr5626",
            });
        }
        if hot_system && fixture.takes_hot() && !near(piece, &[PointKind::Hot]) {
            out.push(Finding {
                key: format!("plumb:hot:{}", piece.id),
                accepted: None,
                severity: Severity::Alerta,
                place: at.clone(),
                message: say!(
                    "O projeto tem água quente e {} não recebe: falta o ponto a até {} cm.",
                    fixture.the(),
                    reach
                ),
                source: "nbr5626",
            });
        }
        let outlets: &[PointKind] = if fixture.drains_to_floor() {
            &[PointKind::Sewer, PointKind::Drain]
        } else {
            &[PointKind::Sewer]
        };
        if !near(piece, outlets) {
            let how = if fixture.drains_to_floor() {
                say!("ponto de esgoto ou ralo sifonado")
            } else {
                say!("ponto de esgoto próprio")
            };
            out.push(Finding {
                key: format!("plumb:sewer:{}", piece.id),
                accepted: None,
                severity: Severity::Erro,
                place: at,
                message: say!(
                    "Sem {} a até {} cm: {} não tem para onde escoar (ramal de {} mm).",
                    how,
                    reach,
                    fixture.the(),
                    fixture.sewer_mm()
                ),
                source: "nbr8160",
            });
        }
    }
    // Every bathroom, kitchen, copa and laundry takes water off its floor
    // (São Paulo's sanitary code, Decreto 12.342/78 art. 15 II); where there
    // is a shower or a tub it is a trap box (NBR 8160).
    for room in view.rooms.iter().filter(|r| r.points.len() >= 3) {
        let wet_name = {
            use crate::vocabulary::Mention as M;
            room.mentions()
                .any(&[M::Bathroom, M::Lavatory, M::Kitchen, M::Pantry, M::Laundry])
        };
        let in_room: Vec<Fixture> = fixtures
            .iter()
            .filter(|(_, f)| inside(&room.points, f.position))
            .map(|(x, _)| *x)
            .collect();
        let showers = in_room
            .iter()
            .any(|x| matches!(x, Fixture::Shower | Fixture::Bathtub));
        let drains: Vec<&Point> = all
            .iter()
            .filter(|p| p.kind == PointKind::Drain && p.room == Some(room.id))
            .collect();
        let place = format!("{} {}", room.name, room.id);
        if (wet_name || !in_room.is_empty()) && drains.is_empty() {
            out.push(Finding {
                key: format!("plumb:drain:{}", room.id),
                accepted: None,
                severity: Severity::Alerta,
                place: place.clone(),
                message: if showers {
                    "Sem ralo: a área do box pede um ralo sifonado (caixa sifonada), que também recebe o lavatório.".into()
                } else {
                    "Sem ralo no piso: o Código Sanitário de SP obriga captação de água no piso de banheiros, cozinhas, copas e lavanderias (pode ser ralo seco).".into()
                },
                source: if showers { "nbr8160" } else { "coe-municipal" },
            });
        }
        let uhc: u32 = in_room
            .iter()
            .filter(|x| x.drains_to_floor())
            .map(|x| x.uhc())
            .sum();
        let specs: Vec<(&Point, DrainSpec)> = drains
            .iter()
            .filter_map(|p| {
                view.find_piece(p.id)
                    .and_then(|f| drain_spec(&f.catalog))
                    .map(|d| (*p, d))
            })
            .collect();
        // A drain with no water seal of 50 mm is no trap: the room needs one
        // that is, or the smell comes back up.
        if !specs.is_empty() && !specs.iter().any(|(_, d)| d.is_trap()) {
            out.push(Finding {
                key: format!("plumb:trap:{}", room.id),
                accepted: None,
                severity: Severity::Alerta,
                place: place.clone(),
                message: say!("Nenhum ralo do cômodo é desconector: ralo seco, linear sem sifão ou sifonado pequeno (fecho de 9 a 20 mm) precisam desaguar numa caixa sifonada com fecho de 50 mm ({}).", specs.iter().map(|(p, _)| p.id.to_string()).collect::<Vec<_>>().join(", ")),
                source: "nbr8160",
            });
        }
        // What the trap boxes take together against the fixtures sent to them.
        let capacity: u32 = specs
            .iter()
            .filter(|(_, d)| d.is_trap())
            .map(|(_, d)| d.max_uhc)
            .sum();
        if capacity > 0 && uhc > capacity {
            out.push(Finding {
                key: format!("plumb:drain-load:{}", room.id),
                accepted: None,
                severity: Severity::Alerta,
                place: place.clone(),
                message: say!("{} UHC vão para o ralo e ele aguenta {}: a saída de 50 mm leva até 6 UHC; use a caixa sifonada 150×185×75 (até 15), divida entre duas caixas ou leve aparelhos a ramais próprios.", uhc, capacity),
                source: "nbr8160",
            });
        }
        // In a room with a shower the drain belongs inside the shower area,
        // where the floor falls 1,5 % to 2,5 % to it.
        let shower_pieces: Vec<&Furniture> = fixtures
            .iter()
            .filter(|(x, f)| {
                *x == Fixture::Shower
                    && inside(&room.points, f.position)
                    && f.width.min(f.depth) >= 50.0
            })
            .map(|(_, f)| f)
            .collect();
        if !drains.is_empty()
            && !shower_pieces.is_empty()
            && !drains.iter().any(|d| {
                shower_pieces.iter().any(|f| {
                    let mut area = (*f).clone();
                    area.width += 10.0;
                    area.depth += 10.0;
                    area.contains(d.at)
                })
            })
        {
            out.push(Finding {
                key: format!("plumb:drain-shower:{}", room.id),
                accepted: None,
                severity: Severity::Alerta,
                place: place.clone(),
                message: "Nenhum ralo dentro da área do box: a água do banho escorre para o resto do banheiro; ponha o ralo (ou um linear) dentro do box, com caimento de 1,5 % a 2,5 % para ele.".into(),
                source: "nbr13753",
            });
        }
        // An open balcony or terrace is rainwater: its drain never joins the
        // sewer (NBR 8160 4.1.3.1, NBR 10844).
        let open_air = {
            use crate::vocabulary::Mention as M;
            room.mentions().any(&[M::Uncovered, M::Outdoor])
        };
        if open_air && !drains.is_empty() {
            out.push(Finding {
                key: format!("plumb:rain:{}", room.id),
                accepted: None,
                severity: Severity::Alerta,
                place: place.clone(),
                message: "Área descoberta com ralo de esgoto: a água de chuva vai para o sistema pluvial, nunca para o esgoto; use o ralo pluvial.".into(),
                source: "nbr10844",
            });
        }
    }
    if fixtures.iter().any(|(x, _)| *x == Fixture::KitchenSink) && !has(PointKind::GreaseTrap) {
        out.push(Finding {
            key: "plumb:grease".into(),
            accepted: None,
            severity: Severity::Dica,
            place: "Pia de cozinha".into(),
            message: "Sem caixa de gordura: numa casa, uma pequena (18 L) ou simples (31 L) entre a pia e a rede; em prédio a pia desce por tubo de queda próprio até a caixa coletiva, e caixa individual no andar é vedada — nesse caso, aceite com esse motivo.".into(),
            source: "nbr8160",
        });
    }
    // Vents: at least one pipe carried above the roof, and every trap
    // within table 1's distance of one (NBR 8160 4.3.11).
    let traps: Vec<&Point> = all
        .iter()
        .filter(|p| matches!(p.kind, PointKind::Sewer | PointKind::Drain))
        .collect();
    let vents: Vec<&Point> = all.iter().filter(|p| p.kind == PointKind::Vent).collect();
    if !traps.is_empty() && vents.is_empty() {
        out.push(Finding {
            key: "plumb:vent".into(),
            accepted: None,
            severity: Severity::Alerta,
            place: "Esgoto".into(),
            message: "Sem ventilação: o esgoto pede ao menos um tubo ventilador prolongado acima da cobertura (em prédio, a coluna de ventilação no shaft); sem ele os fechos hídricos se rompem e o cheiro volta.".into(),
            source: "nbr8160",
        });
    }
    if !vents.is_empty() {
        for trap in &traps {
            let mm = sewer_mm(home, trap);
            let limit = match mm {
                0..=40 => 100.0,
                41..=50 => 120.0,
                51..=75 => 180.0,
                _ => 240.0,
            };
            // A vent branch drawn to the trap counts as the vented element.
            let by_branch = view
                .polylines
                .iter()
                .filter(|l| pipe_of(l) == Some(Pipe::Vent))
                .flat_map(|l| {
                    l.points
                        .windows(2)
                        .map(|w| (w[0], w[1]))
                        .collect::<Vec<_>>()
                })
                .map(|(a, b)| {
                    let (dx, dy) = (b.x - a.x, b.y - a.y);
                    let len2 = (dx * dx + dy * dy).max(1e-9);
                    let t =
                        (((trap.at.x - a.x) * dx + (trap.at.y - a.y) * dy) / len2).clamp(0.0, 1.0);
                    Point2::new(a.x + t * dx, a.y + t * dy).distance(trap.at)
                })
                .fold(f64::MAX, f64::min);
            let listed = view.polylines.iter().any(|l| {
                pipe_of(l) == Some(Pipe::Vent)
                    && l.properties
                        .get(ENDS_KEY)
                        .is_some_and(|ends| ends.split(',').any(|e| e == trap.id.to_string()))
            });
            let by_branch = if listed { 0.0 } else { by_branch };
            let nearest = vents
                .iter()
                .map(|v| v.at.distance(trap.at))
                .fold(by_branch, f64::min);
            if nearest > limit {
                out.push(Finding {
                    key: format!("plumb:vent-far:{}", trap.id),
                    accepted: None,
                    severity: Severity::Dica,
                    place: format!("{} {}", trap.name, trap.id),
                    message: say!("A {} cm do tubo ventilador mais próximo: um ramal de {} mm pede ventilação a até {} cm (em linha reta; trace o ramal com route kind=vent e ele passa a contar).", nearest.round(), mm, limit.round()),
                    source: "nbr8160",
                });
            }
        }
    }
    let boxes: Vec<&Point> = all
        .iter()
        .filter(|p| p.kind == PointKind::InspectionBox)
        .collect();
    // An inspection box is at least 60 cm across (NBR 8160 5.1.5.3).
    for b in &boxes {
        let stack = ["queda", "prumada", "coluna", "shaft", "tq-", "tq "]
            .iter()
            .any(|w| crate::annotations::fold(&b.name).contains(w));
        if let Some(f) = view.find_piece(b.id)
            && !stack
            && f.width.min(f.depth) + 0.5 < 60.0
        {
            out.push(Finding {
                key: format!("plumb:inspection-size:{}", b.id),
                accepted: None,
                severity: Severity::Alerta,
                place: format!("{} {}", b.name, b.id),
                message: say!("Caixa de inspeção de {} cm: a norma pede ao menos 60 cm de lado ou diâmetro, e até 1 m de profundidade (mais funda é poço de visita).", f.width.min(f.depth).round()),
                source: "nbr8160",
            });
        }
    }
    if !boxes.is_empty() {
        for p in all.iter().filter(|p| {
            matches!(p.kind, PointKind::Drain | PointKind::GreaseTrap)
                || (p.kind == PointKind::Sewer && sewer_mm(home, p) >= 100)
        }) {
            let nearest = boxes
                .iter()
                .map(|b| b.at.distance(p.at))
                .fold(f64::MAX, f64::min);
            if nearest > 1000.0 {
                out.push(Finding {
                    key: format!("plumb:inspection-far:{}", p.id),
                    accepted: None,
                    severity: Severity::Alerta,
                    place: format!("{} {}", p.name, p.id),
                    message: say!("A {} m da caixa de inspeção: vaso, caixa sifonada e caixa de gordura ficam a até 10 m de um dispositivo de inspeção.", crate::electrical::decimal(nearest / 100.0)),
                    source: "nbr8160",
                });
            }
        }
    }
    // A water or gas point set in glass or in an opening's span.
    for point in &all {
        if let Some(why) = view
            .find_piece(point.id)
            .and_then(|f| crate::mounting::blocked(home, f))
        {
            out.push(Finding {
                key: format!("plumb:mount:{}", point.id),
                accepted: None,
                severity: Severity::Erro,
                place: format!("{} {}", point.name, point.id),
                message: why,
                source: "nbr5626",
            });
        } else if let Some(why) = view
            .find_piece(point.id)
            .and_then(|f| crate::mounting::hidden(home, f))
        {
            out.push(Finding {
                key: format!("plumb:hidden:{}", point.id),
                accepted: None,
                severity: Severity::Alerta,
                place: format!("{} {}", point.name, point.id),
                message: why,
                source: "nbr8160",
            });
        }
    }
    // Lines of the project that are no pipe: they count nowhere.
    let untyped: Vec<String> = view
        .polylines
        .iter()
        .filter(|l| {
            l.discipline == Some(crate::style::Discipline::Plumbing) && pipe_of(l).is_none()
        })
        .map(|l| l.id.to_string())
        .collect();
    if !untyped.is_empty() {
        out.push(Finding {
            key: "plumb:untyped-lines".into(),
            accepted: None,
            severity: Severity::Dica,
            place: "Hidráulica".into(),
            message: say!("{} linha(s) desenhadas à mão sem dizer se são água fria, quente ou esgoto ({}): não entram nos metros de tubo; trace com route (que as substitui) ou apague.", untyped.len(), untyped.join(", ")),
            source: "nbr8160",
        });
    }
    // The premises: where the water comes from and where the sewer goes.
    if all
        .iter()
        .any(|p| matches!(p.kind, PointKind::Cold | PointKind::Hot))
        && !has(PointKind::WaterMeter)
        && !has(PointKind::Valve)
    {
        out.push(Finding {
            key: "plumb:source:water".into(),
            accepted: None,
            severity: Severity::Alerta,
            place: "Água fria".into(),
            message: "De onde vem a água? Coloque o hidrômetro ou o registro geral (em prédio, junto à prumada no shaft): é deles que os ramais saem.".into(),
            source: "nbr5626",
        });
    }
    if all
        .iter()
        .any(|p| matches!(p.kind, PointKind::Sewer | PointKind::Drain))
        && !has(PointKind::InspectionBox)
    {
        out.push(Finding {
            key: "plumb:source:sewer".into(),
            accepted: None,
            severity: Severity::Alerta,
            place: "Esgoto".into(),
            message: "Para onde vai o esgoto? Coloque a caixa de inspeção (em prédio, o tubo de queda no shaft) — é a ela que os ramais descem com caimento.".into(),
            source: "nbr8160",
        });
    }
    // Where pipes are drawn, each point of their kind needs one reaching it.
    // Venting is asked trap by trap (vent-far), not of every trap.
    for pipe in [Pipe::Cold, Pipe::Hot, Pipe::Sewer] {
        let runs: Vec<&Polyline> = view
            .polylines
            .iter()
            .filter(|l| pipe_of(l) == Some(pipe))
            .collect();
        if runs.is_empty() {
            continue;
        }
        let unreached: Vec<String> = all
            .iter()
            .filter(|p| pipe.serves(p.kind) && !runs.iter().any(|l| serves_point(l, p.id, p.at)))
            .map(|p| p.id.to_string())
            .collect();
        if !unreached.is_empty() {
            out.push(Finding {
                key: format!("plumb:unreached:{}", pipe.key()),
                accepted: None,
                severity: Severity::Alerta,
                place: pipe.name().into(),
                message: say!("Pontos sem tubulação chegando: {}.", unreached.join(", ")),
                source: if pipe == Pipe::Sewer {
                    "nbr8160"
                } else {
                    "nbr5626"
                },
            });
        }
    }
    for f in &mut out {
        f.accepted = home.accepted.get(&f.key).cloned();
    }
    out.sort_by_key(|f| f.severity);
    out
}

/// Acceptances whose finding is gone.
pub fn orphaned(home: &Home) -> Vec<(String, String)> {
    let live: std::collections::BTreeSet<String> = check(home).into_iter().map(|f| f.key).collect();
    home.accepted
        .iter()
        .filter(|(key, _)| key.starts_with("plumb:") && !live.contains(*key))
        .map(|(key, why)| (key.clone(), why.clone()))
        .collect()
}

/// Lines drawn by hand that a laid-out run of `pipe` replaces: plumbing
/// lines of no run that are of this pipe and reach one of `ends`, or that say
/// no pipe and start and finish at them (so a sewer line drawn beside a
/// water point is never taken for the water's).
pub fn drawn_runs(home: &Home, pipe: Pipe, ends: &[Point2]) -> Vec<crate::ids::PolylineId> {
    let near = |p: Point2| ends.iter().any(|e| e.distance(p) <= 30.0);
    home.level_view(home.current_level())
        .polylines
        .iter()
        .filter(|l| l.discipline == Some(crate::style::Discipline::Plumbing))
        .filter(|l| !l.properties.contains_key(RUN_KEY) && l.points.len() >= 2)
        .filter(|l| match pipe_of(l) {
            Some(p) => p == pipe && ends.iter().any(|e| reaches(l, *e)),
            None => near(l.points[0]) && near(l.points[l.points.len() - 1]),
        })
        .map(|l| l.id)
        .collect()
}

/// Pipe metres by kind, for the purchase: a laid-out run counts its real
/// length, a line drawn by hand its plan length with a tenth for the drops.
pub fn pipe_lengths(home: &Home) -> Vec<(Pipe, f64)> {
    let view = home.level_view(home.current_level());
    Pipe::ALL
        .into_iter()
        .map(|pipe| {
            let mut routed: std::collections::BTreeMap<String, f64> =
                std::collections::BTreeMap::new();
            let mut drawn = 0.0;
            for line in view.polylines.iter().filter(|l| pipe_of(l) == Some(pipe)) {
                match (
                    line.properties.get(RUN_KEY),
                    line.properties
                        .get(RUN_CM_KEY)
                        .and_then(|v| v.parse::<f64>().ok()),
                ) {
                    (Some(run), Some(cm)) => {
                        routed.insert(run.clone(), cm);
                    }
                    _ => {
                        drawn += line
                            .points
                            .windows(2)
                            .map(|w| w[0].distance(w[1]))
                            .sum::<f64>()
                            * 1.1;
                    }
                }
            }
            (pipe, metres(drawn + routed.values().sum::<f64>()))
        })
        .filter(|(_, m)| *m > 0.0)
        .collect()
}

/// Why a premise cannot carry a pipe, if it cannot.
///
/// Sewer runs by gravity: it never climbs to this storey's ceiling, and a
/// wall only takes it going down — lying in a wall it would have no fall.
/// Under the floor it is the slab's recess, or the ceiling of the storey
/// below, measured from this floor.
pub fn refuses(pipe: Pipe, via: Via) -> Option<&'static str> {
    match (pipe, via) {
        (Pipe::Sewer, Via::Ceiling) => Some(
            "esgoto corre por gravidade e não sobe ao forro deste andar: use floor (rebaixo da laje ou forro do andar de baixo)",
        ),
        (Pipe::Sewer, Via::Wall) => Some(
            "esgoto não corre deitado dentro da parede, sem caimento: na parede ele só desce; use floor",
        ),
        (Pipe::Vent, Via::Floor) => Some(
            "o ramal de ventilação sobe do desconector até a coluna, pela parede ou pelo forro; enterrado ele enche de água e não ventila",
        ),
        _ => None,
    }
}

/// The height a sewer run needs under the floor, cm: the largest fall of a
/// branch — its whole length at its own diameter's slope, which is on the
/// safe side where it joins a larger trunk —, the trunk pipe itself and 2 cm
/// to lay it on. `sizes` are the points' diameters, in the route's order.
pub fn sewer_depth(route: &Route, sizes: &[u32]) -> f64 {
    sewer_depth_with(route, sizes, 0.0)
}

/// [`sewer_depth`], and never less than the deepest drain body on the run.
pub fn sewer_depth_with(route: &Route, sizes: &[u32], drain_depth: f64) -> f64 {
    let trunk = sizes.iter().copied().max().unwrap_or(50);
    let fall = route
        .terminals
        .iter()
        .zip(&route.reach)
        .skip(1)
        .zip(sizes)
        .map(|((t, cm), mm)| (cm - t.z.max(0.0) - route.terminals[0].z.max(0.0)) * slope(*mm))
        .fold(0.0, f64::max);
    let cm = (fall + f64::from(trunk) / 10.0 + 2.0).max(drain_depth);
    (cm * 10.0).round() / 10.0
}

/// A point to lay a run to.
pub fn terminal(point: &Point) -> Terminal {
    Terminal {
        id: Some(point.id),
        at: point.at,
        z: point.z,
    }
}

/// What a laid-out run takes to build.
///
/// Cold water: PVC solvent-weld 25 mm in 6 m bars, a 90° elbow at each turn,
/// a tee at each branch, an elbow with a brass thread at each point and a
/// gate valve per room served. Hot water: CPVC 22 mm in 3 m bars with the
/// same fittings. Sewer: the trunk at the largest diameter it receives, a
/// point's own diameter from the trunk down to it, a 45° junction (a Y) at
/// each branch — a 90° tee does not join sewer lying down —, two 45° elbows
/// for each turn, a short 90° bend under each point, a sealing ring per
/// toilet and a trap box per floor drain.
#[allow(clippy::cast_precision_loss)] // counts of fittings, far below 2^52
pub fn materials(
    route: &Route,
    pipe: Pipe,
    points: &[(PointKind, u32)],
    rooms: usize,
) -> Vec<Material> {
    let n = points.len();
    let bends = route.bends.saturating_sub(if matches!(pipe, Pipe::Sewer) {
        n + 1
    } else {
        0
    });
    let item = |item: String, quantity: f64, unit: &'static str| Material {
        item,
        quantity,
        unit,
    };
    let mut out = Vec::new();
    match pipe {
        Pipe::Vent => {
            let metres = metres(route.length() * 1.05);
            out.push(item(
                "Tubo PVC esgoto série normal 50 mm (ramal de ventilação)".into(),
                metres,
                "m",
            ));
            out.push(item("Barras de 6 m".into(), (metres / 6.0).ceil(), "un"));
            out.push(item(
                "Joelho 45° 50 mm (dois por curva)".into(),
                (2 * route.bends) as f64,
                "un",
            ));
            out.push(item(
                "Junção simples 45° 50 × 50 mm".into(),
                route.branches as f64,
                "un",
            ));
            out.push(item(
                "Junção 45° de ligação ao ramal de descarga, acima do fecho hídrico".into(),
                n as f64,
                "un",
            ));
        }
        Pipe::Cold | Pipe::Hot => {
            let (tube, bar, size, thread) = match pipe {
                Pipe::Cold => (
                    "Tubo PVC soldável 25 mm (3/4\")",
                    6.0,
                    "PVC soldável 25 mm",
                    "Joelho 90° soldável com bucha de latão 25 mm × 1/2\"",
                ),
                _ => (
                    "Tubo CPVC 22 mm (água quente)",
                    3.0,
                    "CPVC 22 mm",
                    "Joelho 90° de transição CPVC 22 mm × 1/2\"",
                ),
            };
            let metres = metres(route.length() * 1.1);
            out.push(item(tube.into(), metres, "m"));
            out.push(item(
                format!("Barras de {bar} m"),
                (metres / bar).ceil(),
                "un",
            ));
            out.push(item(format!("Joelho 90° {size}"), route.bends as f64, "un"));
            out.push(item(format!("Tê {size}"), route.branches as f64, "un"));
            out.push(item(thread.into(), n as f64, "un"));
            out.push(item(
                format!("Registro de gaveta {size} (um por ambiente: boa prática; a NBR 5626 exige ao menos um antes dos sub-ramais de um ambiente sanitário)"),
                rooms.max(1) as f64,
                "un",
            ));
            let joints = 2 * route.bends + 3 * route.branches + n + 2 * rooms.max(1);
            out.push(item(
                if pipe == Pipe::Cold {
                    "Adesivo para PVC 175 g".into()
                } else {
                    "Adesivo para CPVC 175 g".into()
                },
                ((joints as f64) / 40.0).ceil().max(1.0),
                "un",
            ));
        }
        Pipe::Sewer => {
            let trunk = points.iter().map(|(_, mm)| *mm).max().unwrap_or(50);
            let drops: f64 = route
                .terminals
                .iter()
                .skip(1)
                .map(|t| t.z.max(0.0) + 30.0)
                .sum();
            let horizontal = metres((route.length() - drops).max(0.0) * 1.05);
            out.push(item(
                format!("Tubo PVC esgoto série normal {trunk} mm (ramal)"),
                horizontal,
                "m",
            ));
            let mut by_mm: std::collections::BTreeMap<u32, f64> = std::collections::BTreeMap::new();
            for ((_, mm), t) in points.iter().zip(route.terminals.iter().skip(1)) {
                *by_mm.entry(*mm).or_default() += t.z.max(0.0) + 30.0;
            }
            let mut bars = horizontal;
            for (mm, cm) in &by_mm {
                // Machines and sinks discharge hot water: série reforçada.
                let series = if *mm == 50 { "reforçada" } else { "normal" };
                out.push(item(
                    format!("Tubo PVC esgoto série {series} {mm} mm (descidas aos pontos)"),
                    metres(*cm),
                    "m",
                ));
                bars += metres(*cm);
            }
            out.push(item("Barras de 6 m".into(), (bars / 6.0).ceil(), "un"));
            let mut junctions: std::collections::BTreeMap<u32, usize> =
                std::collections::BTreeMap::new();
            for ((_, mm), degree) in points.iter().zip(route.degrees.iter().skip(1)) {
                if *degree > 1 {
                    *junctions.entry(*mm).or_default() += degree - 1;
                }
            }
            let source_split = route.degrees.first().map_or(0, |d| d.saturating_sub(1));
            if source_split > 0 {
                *junctions.entry(trunk).or_default() += source_split;
            }
            // A Y is not made smaller than 50: a 40 mm branch joins through
            // a 50 Y and a 50 × 40 bushing.
            let mut bushings = 0;
            for (mm, count) in junctions {
                let branch = mm.max(50);
                out.push(item(
                    format!("Junção simples 45° (Y) {} × {branch} mm", trunk.max(50)),
                    count as f64,
                    "un",
                ));
                if mm < 50 {
                    bushings += count;
                }
            }
            if bushings > 0 {
                out.push(item(
                    "Bucha de redução longa 50 × 40 mm".into(),
                    bushings as f64,
                    "un",
                ));
            }
            out.push(item(
                format!("Joelho 45° {trunk} mm (dois por curva)"),
                (2 * bends) as f64,
                "un",
            ));
            for (mm, count) in points.iter().fold(
                std::collections::BTreeMap::<u32, usize>::new(),
                |mut m, (_, mm)| {
                    *m.entry(*mm).or_default() += 1;
                    m
                },
            ) {
                out.push(item(
                    format!("Curva 90° curta {mm} mm (sob o ponto)"),
                    count as f64,
                    "un",
                ));
            }
            let toilets = points
                .iter()
                .filter(|(k, mm)| *k == PointKind::Sewer && *mm >= 100)
                .count();
            if toilets > 0 {
                out.push(item(
                    "Anel de vedação para bacia sanitária (nenhum outro ramal entra pela inspeção da curva do vaso, NBR 8160 4.2.3.5)".into(),
                    toilets as f64,
                    "un",
                ));
            }
            for (size, outlet) in [("150×150×50", 50), ("150×185×75", 75)] {
                let drains = points
                    .iter()
                    .filter(|(k, mm)| *k == PointKind::Drain && *mm == outlet)
                    .count();
                if drains > 0 {
                    out.push(item(
                        format!("Caixa sifonada {size} mm com grelha"),
                        drains as f64,
                        "un",
                    ));
                }
            }
            out.push(item(
                "Pasta lubrificante para junta elástica 400 g".into(),
                1.0,
                "un",
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::cast_precision_loss)] // counts and table values are exact

    use super::*;
    use crate::elements::Wall;
    use crate::ids::WallId;
    use crate::routing::lay_out;
    use crate::style::Discipline;

    fn piece(
        id: u64,
        catalog: &str,
        name: &str,
        at: (f64, f64),
        size: (f64, f64, f64),
    ) -> Furniture {
        Furniture {
            id: FurnitureId(id),
            catalog: catalog.into(),
            name: name.into(),
            position: Point2::new(at.0, at.1),
            width: size.0,
            depth: size.1,
            height: size.2,
            ..Furniture::default()
        }
    }

    fn point(id: u64, catalog: &str, name: &str, at: (f64, f64), elevation: f64) -> Furniture {
        let mut f = piece(id, catalog, name, at, (8.0, 8.0, 8.0));
        f.elevation = elevation;
        f.discipline = Some(Discipline::Plumbing);
        f
    }

    fn bathroom() -> Home {
        let mut home = Home::default();
        let pts = [(0.0, 0.0), (200.0, 0.0), (200.0, 250.0), (0.0, 250.0)];
        for k in 0..4 {
            let (a, b) = (pts[k], pts[(k + 1) % 4]);
            home.walls.push(Wall::new(
                WallId(k as u64 + 1),
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            ));
        }
        home.rooms.push(Room::new(
            RoomId(9),
            "Banho suíte",
            pts.iter().map(|p| Point2::new(p.0, p.1)).collect(),
        ));
        home.furniture = vec![
            piece(20, "toilet", "Vaso", (50.0, 35.0), (40.0, 63.0, 62.0)),
            piece(
                21,
                "basin-cabinet",
                "Gabinete",
                (150.0, 25.0),
                (60.0, 45.0, 85.0),
            ),
            piece(22, "shower", "Box", (150.0, 200.0), (90.0, 90.0, 200.0)),
        ];
        home
    }

    #[test]
    fn every_fixture_asks_for_its_water_and_its_sewer_and_the_premises_are_asked() {
        let mut home = bathroom();
        let keys = |home: &Home| check(home).into_iter().map(|f| f.key).collect::<Vec<_>>();
        let bare = keys(&home);
        for key in [
            "plumb:cold:f20",
            "plumb:sewer:f20",
            "plumb:cold:f22",
            "plumb:sewer:f22",
            "plumb:drain:r9",
        ] {
            assert!(bare.contains(&key.to_owned()), "{key} in {bare:?}");
        }
        // No point at all: no premise asked yet, nothing to feed.
        assert!(
            !bare.iter().any(|k| k.starts_with("plumb:source")),
            "{bare:?}"
        );

        home.furniture.extend([
            point(30, "cold-water", "AF — vaso", (30.0, 5.0), 20.0),
            point(31, "sewer", "Esgoto — vaso banho suíte", (50.0, 10.0), 0.0),
            point(32, "cold-water", "AF — lavatório", (150.0, 5.0), 60.0),
            point(33, "cold-water", "AF — chuveiro", (195.0, 200.0), 210.0),
            point(34, "floor-drain", "Ralo", (150.0, 200.0), 0.0),
            point(38, "sewer", "Esgoto — lavatório", (150.0, 10.0), 50.0),
        ]);
        let wired = keys(&home);
        assert!(
            !wired
                .iter()
                .any(|k| k.starts_with("plumb:cold") || k.starts_with("plumb:sewer:")),
            "{wired:?}"
        );
        // The shower drains through the floor drain, which the room had lacked.
        assert!(!wired.contains(&"plumb:drain:r9".to_owned()), "{wired:?}");
        assert!(
            wired.contains(&"plumb:source:water".to_owned()),
            "{wired:?}"
        );
        assert!(
            wired.contains(&"plumb:source:sewer".to_owned()),
            "{wired:?}"
        );
        // No hot water anywhere: an electric shower, nothing to ask.
        assert!(
            !wired.iter().any(|k| k.starts_with("plumb:hot")),
            "{wired:?}"
        );

        // Hot water in the project: the shower and the basin need it.
        home.furniture.push(point(
            35,
            "hot-water",
            "AQ — chuveiro",
            (180.0, 200.0),
            210.0,
        ));
        let hot = keys(&home);
        assert!(hot.contains(&"plumb:hot:f21".to_owned()), "{hot:?}");
        assert!(!hot.contains(&"plumb:hot:f22".to_owned()), "{hot:?}");
        assert!(
            !hot.contains(&"plumb:hot:f20".to_owned()),
            "a toilet takes no hot water: {hot:?}"
        );

        // The point named after the toilet is the toilet's 100 mm outlet.
        let points = points(&home);
        let sewer = points.iter().find(|p| p.id == FurnitureId(31)).unwrap();
        assert_eq!(sewer_mm(&home, sewer), 100);
        let drain = points.iter().find(|p| p.id == FurnitureId(34)).unwrap();
        assert_eq!(sewer_mm(&home, drain), 50);
    }

    #[test]
    fn a_cabinet_part_named_after_the_sink_is_not_a_sink_and_a_sink_counts_once() {
        let mut home = Home::default();
        let mut run = piece(
            40,
            "joinery",
            "Bancada da cozinha",
            (100.0, 30.0),
            (200.0, 60.0, 90.0),
        );
        run.children = vec![
            piece(
                41,
                "joinery",
                "Gavetão pia com recorte hidráulico — moldura vertical",
                (80.0, 58.0),
                (2.0, 2.0, 70.0),
            ),
            piece(
                42,
                "joinery",
                "Gavetão pia com recorte hidráulico — puxador pequeno dourado",
                (90.0, 60.0),
                (12.0, 2.0, 2.0),
            ),
            piece(
                43,
                "joinery",
                "10 — Pia: dois gavetões em U — módulo 70 cm",
                (100.0, 30.0),
                (70.0, 60.0, 87.0),
            ),
        ];
        home.furniture = vec![
            run,
            piece(
                44,
                "imported",
                "Pia centralizada na bancada",
                (105.0, 30.0),
                (60.0, 40.0, 20.0),
            ),
        ];
        let found = fixtures(&home);
        assert_eq!(
            found.len(),
            1,
            "{:?}",
            found.iter().map(|(x, f)| (x, f.id)).collect::<Vec<_>>()
        );
        assert_eq!(found[0].0, Fixture::KitchenSink);
    }

    #[test]
    fn a_bowl_is_what_its_room_makes_it() {
        use crate::RoomUse as U;
        let bowl_in = |room: &str, usage: crate::RoomUse| {
            let mut home = Home::default();
            let mut r = crate::Room::new(
                crate::RoomId(10),
                room,
                vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(300.0, 0.0),
                    Point2::new(300.0, 300.0),
                    Point2::new(0.0, 300.0),
                ],
            );
            r.usage = usage;
            home.rooms.push(r);
            home.furniture = vec![piece(
                40,
                "sink-bowl",
                "Cuba de inox (embutir)",
                (150.0, 30.0),
                (50.0, 40.0, 18.0),
            )];
            fixtures(&home)[0].0
        };
        assert_eq!(bowl_in("Cozinha integrada", U::Auto), Fixture::KitchenSink);
        assert_eq!(bowl_in("Varanda gourmet", U::Auto), Fixture::KitchenSink);
        assert_eq!(bowl_in("Lavanderia", U::Auto), Fixture::LaundrySink);
        assert_eq!(bowl_in("Lavabo", U::Auto), Fixture::Basin);
        assert_eq!(bowl_in("Banho master", U::Auto), Fixture::Basin);
        // What the room is declared to be wins over its name.
        assert_eq!(bowl_in("Espaço 3", U::Kitchen), Fixture::KitchenSink);
        assert_eq!(bowl_in("Cozinha antiga", U::Bathroom), Fixture::Basin);
        // Nowhere in particular, a bowl is still a basin.
        assert_eq!(bowl_in("Sala", U::Auto), Fixture::Basin);
        // Called "Pia" or "Lavatório", the catalog's bowl: the room decides the
        // first, the name the second.
        let named = |name: &str, room: &str| {
            let mut home = Home::default();
            home.rooms.push(crate::Room::new(
                crate::RoomId(10),
                room,
                vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(300.0, 0.0),
                    Point2::new(300.0, 300.0),
                    Point2::new(0.0, 300.0),
                ],
            ));
            home.furniture = vec![piece(
                40,
                "sink-bowl",
                name,
                (150.0, 30.0),
                (50.0, 40.0, 18.0),
            )];
            fixtures(&home)[0].0
        };
        assert_eq!(named("Pia do banheiro", "Banheiro"), Fixture::Basin);
        assert_eq!(named("Pia", "Lavanderia"), Fixture::LaundrySink);
        assert_eq!(named("Lavatório de apoio", "Cozinha"), Fixture::Basin);
        assert_eq!(
            named("Pia", "Sala"),
            Fixture::Basin,
            "a bowl elsewhere is a basin"
        );
        // Made as a basin, it is one wherever it is and whatever it is called.
        let mut home = Home::default();
        home.rooms.push(crate::Room::new(
            crate::RoomId(10),
            "Cozinha",
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(300.0, 0.0),
                Point2::new(300.0, 300.0),
                Point2::new(0.0, 300.0),
            ],
        ));
        home.furniture = vec![piece(
            40,
            "basin-cabinet",
            "Cuba",
            (150.0, 30.0),
            (60.0, 45.0, 85.0),
        )];
        assert_eq!(fixtures(&home)[0].0, Fixture::Basin);
        // Two drawings of one bowl, called "Pia" and "Cuba": one kitchen sink.
        let mut home = Home::default();
        home.rooms.push(crate::Room::new(
            crate::RoomId(10),
            "Cozinha",
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(300.0, 0.0),
                Point2::new(300.0, 300.0),
                Point2::new(0.0, 300.0),
            ],
        ));
        home.furniture = vec![
            piece(40, "sink-bowl", "Pia", (150.0, 30.0), (50.0, 40.0, 18.0)),
            piece(41, "sink-bowl", "Cuba", (152.0, 30.0), (50.0, 40.0, 18.0)),
        ];
        let found = fixtures(&home);
        assert_eq!(
            found.len(),
            1,
            "{:?}",
            found.iter().map(|(x, f)| (x, f.id)).collect::<Vec<_>>()
        );
        assert_eq!(found[0].0, Fixture::KitchenSink);
        // A bathroom inside a kitchen's outline is the room the bowl is in.
        let mut home = Home::default();
        for (id, name, size) in [(10, "Cozinha", 600.0), (11, "Lavabo", 200.0)] {
            home.rooms.push(crate::Room::new(
                crate::RoomId(id),
                name,
                vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(size, 0.0),
                    Point2::new(size, size),
                    Point2::new(0.0, size),
                ],
            ));
        }
        home.furniture = vec![piece(
            40,
            "sink-bowl",
            "Cuba",
            (100.0, 30.0),
            (50.0, 40.0, 18.0),
        )];
        assert_eq!(fixtures(&home)[0].0, Fixture::Basin);
    }

    #[test]
    fn a_fixture_is_named_with_its_own_article() {
        use crate::vocabulary::Language;
        let mut home = Home::default();
        home.furniture = vec![
            piece(
                40,
                "sink-counter",
                "Pia",
                (100.0, 30.0),
                (120.0, 60.0, 90.0),
            ),
            piece(41, "washer", "Máquina", (300.0, 30.0), (60.0, 60.0, 85.0)),
        ];
        let said: Vec<(String, String)> = check(&home)
            .iter()
            .filter(|f| f.key.starts_with("plumb:cold:"))
            .map(|f| {
                (
                    f.message.in_language(Language::Portuguese),
                    f.message.in_language(Language::English),
                )
            })
            .collect();
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].0.contains(": a pia de cozinha não tem"), "{said:?}");
        assert!(said[0].1.contains(": the kitchen sink has"), "{said:?}");
        assert!(
            said[1].0.contains(": a máquina de lavar não tem"),
            "{said:?}"
        );
        assert_eq!(
            Fixture::Dishwasher
                .the()
                .in_language(crate::vocabulary::Language::Portuguese),
            "a lava-louças"
        );
    }

    #[test]
    fn sewer_runs_under_the_floor_with_its_fall_and_joins_with_a_y() {
        let mut home = bathroom();
        home.furniture.extend([
            point(31, "sewer", "Esgoto — vaso", (50.0, 10.0), 0.0),
            point(34, "floor-drain", "Ralo", (150.0, 200.0), 0.0),
            point(36, "inspection-box", "Prumada", (0.0, 125.0), 0.0),
        ]);
        assert!(refuses(Pipe::Sewer, Via::Ceiling).is_some());
        assert!(refuses(Pipe::Sewer, Via::Wall).is_some());
        assert!(refuses(Pipe::Cold, Via::Ceiling).is_none());
        let all = points(&home);
        let source = terminal(
            all.iter()
                .find(|p| p.kind == PointKind::InspectionBox)
                .unwrap(),
        );
        let ends: Vec<&Point> = all.iter().filter(|p| Pipe::Sewer.serves(p.kind)).collect();
        let route = lay_out(
            &home,
            source,
            &ends.iter().map(|p| terminal(p)).collect::<Vec<_>>(),
            Via::Floor,
            280.0,
        );
        let sizes: Vec<(PointKind, u32)> =
            ends.iter().map(|p| (p.kind, sewer_mm(&home, p))).collect();
        let mms: Vec<u32> = sizes.iter().map(|(_, mm)| *mm).collect();
        let depth = sewer_depth(&route, &mms);
        // The drain's 50 mm branch falls at 2 %, more than the toilet's at 1 %:
        // the depth follows it, plus the 10 cm trunk and 2 cm under it.
        let longest = route.reach.iter().copied().fold(0.0, f64::max);
        assert!(
            depth > 12.0 && depth <= longest * 0.02 + 12.1,
            "{depth} for {longest}"
        );
        let trunk_only = sewer_depth(&route, &[100, 100]);
        assert!(depth > trunk_only, "{depth} > {trunk_only}");
        let bill = materials(&route, Pipe::Sewer, &sizes, 1);
        let text = format!("{bill:?}");
        assert!(text.contains("série normal 100 mm (ramal)"), "{bill:#?}");
        assert!(text.contains("Anel de vedação"), "{bill:#?}");
        assert!(text.contains("Caixa sifonada"), "{bill:#?}");
        assert!(!text.contains("Tê"), "no 90° tee joins sewer: {bill:#?}");
        assert!(
            route.branches == 0 || text.contains("Junção simples 45° (Y)"),
            "{bill:#?}"
        );
    }

    #[test]
    fn a_cold_water_branch_lists_tees_elbows_and_a_valve_per_room() {
        let mut home = bathroom();
        home.furniture.extend([
            point(30, "cold-water", "AF — vaso", (30.0, 5.0), 20.0),
            point(32, "cold-water", "AF — lavatório", (150.0, 5.0), 60.0),
            point(33, "cold-water", "AF — chuveiro", (195.0, 200.0), 210.0),
            point(37, "water-meter", "Hidrômetro", (0.0, 125.0), 100.0),
        ]);
        let all = points(&home);
        let source = terminal(
            all.iter()
                .find(|p| p.kind == PointKind::WaterMeter)
                .unwrap(),
        );
        let ends: Vec<Terminal> = all
            .iter()
            .filter(|p| p.kind == PointKind::Cold)
            .map(terminal)
            .collect();
        let route = lay_out(&home, source, &ends, Via::Ceiling, 280.0);
        let sizes = vec![(PointKind::Cold, 25); 3];
        let bill = materials(&route, Pipe::Cold, &sizes, 1);
        let get = |item: &str| {
            bill.iter()
                .find(|m| m.item.starts_with(item))
                .unwrap_or_else(|| panic!("{item}: {bill:#?}"))
                .quantity
        };
        assert!(get("Tubo PVC soldável 25 mm") >= route.length() / 100.0);
        assert_eq!(get("Joelho 90° soldável com bucha"), 3.0);
        assert_eq!(get("Tê PVC"), route.branches as f64);
        assert_eq!(get("Registro de gaveta"), 1.0);
        assert!(get("Barras de 6 m") * 6.0 >= get("Tubo PVC soldável 25 mm"));
    }

    #[test]
    fn the_norm_texts_hold_washer_floor_drains_vents_trap_boxes_and_inspection() {
        let mut home = bathroom();
        // A laundry with a washing machine: its 50 mm branch never goes to a
        // floor drain, and the room takes water off its floor (SP code).
        home.rooms.push(Room::new(
            RoomId(12),
            "Lavanderia",
            vec![
                Point2::new(0.0, 300.0),
                Point2::new(200.0, 300.0),
                Point2::new(200.0, 450.0),
                Point2::new(0.0, 450.0),
            ],
        ));
        home.furniture.extend([
            piece(50, "washer", "Máquina", (50.0, 400.0), (60.0, 60.0, 85.0)),
            point(51, "floor-drain", "Ralo lavanderia", (60.0, 430.0), 0.0),
            point(52, "cold-water", "AF máquina", (50.0, 440.0), 90.0),
        ]);
        let keys = |home: &Home| check(home).into_iter().map(|f| f.key).collect::<Vec<_>>();
        let k = keys(&home);
        assert!(
            k.contains(&"plumb:sewer:f50".to_owned()),
            "a floor drain does not take the washer: {k:?}"
        );
        assert!(!k.contains(&"plumb:drain:r12".to_owned()), "{k:?}");

        let mut kitchen = Home::default();
        kitchen.rooms.push(Room::new(
            RoomId(3),
            "Cozinha",
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(300.0, 0.0),
                Point2::new(300.0, 300.0),
                Point2::new(0.0, 300.0),
            ],
        ));
        let dry = check(&kitchen);
        let drain = dry
            .iter()
            .find(|f| f.key == "plumb:drain:r3")
            .expect("a kitchen with no drain");
        assert_eq!(drain.source, "coe-municipal", "{drain:?}");

        // Vents: none at all, then one too far from a 40 mm trap.
        let mut home = bathroom();
        home.furniture.extend([
            point(30, "sewer", "Esgoto vaso", (50.0, 10.0), 0.0),
            point(34, "floor-drain", "Ralo", (150.0, 200.0), 0.0),
            point(36, "inspection-box", "CI", (0.0, 125.0), 0.0),
        ]);
        assert!(keys(&home).contains(&"plumb:vent".to_owned()));
        home.furniture
            .push(point(37, "vent-pipe", "TV", (40.0, 5.0), 0.0));
        let k = keys(&home);
        assert!(!k.contains(&"plumb:vent".to_owned()), "{k:?}");
        assert!(
            k.contains(&"plumb:vent-far:f34".to_owned()),
            "the drain is 2 m from the vent: {k:?}"
        );
        assert!(
            !k.contains(&"plumb:vent-far:f30".to_owned()),
            "the toilet is within 2,4 m: {k:?}"
        );
        // A vent branch drawn to the drain is the vented element.
        let mut branch = crate::style::Polyline::new(
            crate::ids::PolylineId(90),
            vec![
                Point2::new(40.0, 5.0),
                Point2::new(150.0, 5.0),
                Point2::new(150.0, 190.0),
            ],
        );
        branch.discipline = Some(crate::style::Discipline::Plumbing);
        branch.properties.insert(PIPE_KEY.into(), "vent".into());
        home.polylines.push(branch);
        assert!(!keys(&home).contains(&"plumb:vent-far:f34".to_owned()));
        home.polylines.clear();

        // The trap box grows with its load: basin 1 + shower 2 + tub 2 + bidet 1
        // + laundry sink 3 = 9 UHC takes a 75 mm outlet.
        home.furniture.extend([
            piece(
                60,
                "bathtub",
                "Banheira",
                (100.0, 150.0),
                (70.0, 150.0, 55.0),
            ),
            piece(61, "imported", "Bidê", (100.0, 60.0), (36.0, 50.0, 40.0)),
            piece(
                62,
                "laundry-sink",
                "Tanque",
                (30.0, 150.0),
                (50.0, 50.0, 85.0),
            ),
        ]);
        let drain = points(&home)
            .into_iter()
            .find(|p| p.id == FurnitureId(34))
            .unwrap();
        assert_eq!(drain_uhc(&home, &drain), 9);
        // The 150×150×50 box's 50 mm outlet takes 6: said, with the way out.
        assert_eq!(sewer_mm(&home, &drain), 50);
        assert!(keys(&home).contains(&"plumb:drain-load:r9".to_owned()));
        home.furniture
            .iter_mut()
            .find(|f| f.id == FurnitureId(34))
            .unwrap()
            .catalog = "floor-drain-75".into();
        let drain = points(&home)
            .into_iter()
            .find(|p| p.id == FurnitureId(34))
            .unwrap();
        assert_eq!(sewer_mm(&home, &drain), 75);
        assert!(!keys(&home).contains(&"plumb:drain-load:r9".to_owned()));

        // An inspection box is 60 cm across at least.
        let mut small = home.clone();
        small.furniture.push({
            let mut b = point(70, "inspection-box", "CI pequena", (100.0, 100.0), 0.0);
            b.width = 40.0;
            b.depth = 40.0;
            b
        });
        assert!(keys(&small).contains(&"plumb:inspection-size:f70".to_owned()));

        // Inspection within 10 m.
        home.furniture.retain(|f| f.id != FurnitureId(36));
        home.furniture
            .push(point(36, "inspection-box", "CI", (1300.0, 125.0), 0.0));
        let k = keys(&home);
        assert!(k.contains(&"plumb:inspection-far:f30".to_owned()), "{k:?}");
    }

    #[test]
    fn drains_are_fixed_in_the_floor_and_checked_as_the_models_they_are() {
        use crate::mounting::{blocked, hidden, seat};
        assert!(drain_spec("floor-drain").unwrap().is_trap());
        assert!(
            !drain_spec("trap-drain-small").unwrap().is_trap(),
            "a 20 mm seal is no trap"
        );
        assert!(!drain_spec("dry-drain").unwrap().is_trap());
        assert_eq!(drain_spec("floor-drain-75").unwrap().max_uhc, 15);

        let mut home = bathroom();
        let keys = |home: &Home| check(home).into_iter().map(|f| f.key).collect::<Vec<_>>();
        // A dry drain in the box and nothing else: no trap in the room.
        home.furniture
            .push(point(40, "dry-drain", "Ralo seco", (150.0, 200.0), 0.0));
        let k = keys(&home);
        assert!(k.contains(&"plumb:trap:r9".to_owned()), "{k:?}");
        assert!(!k.contains(&"plumb:drain-shower:r9".to_owned()), "{k:?}");
        // A trap box added outside the box: a trap now, but the box has only the dry one.
        home.furniture.push(point(
            41,
            "floor-drain",
            "Caixa sifonada",
            (60.0, 150.0),
            0.0,
        ));
        let k = keys(&home);
        assert!(!k.contains(&"plumb:trap:r9".to_owned()), "{k:?}");
        // Only the trap box, outside the shower area: said.
        home.furniture.retain(|f| f.id != FurnitureId(40));
        assert!(keys(&home).contains(&"plumb:drain-shower:r9".to_owned()));

        // Fixed in the floor: inside the room, never in a wall or a door span,
        // and never under a cabinet.
        let mut d = point(42, "floor-drain", "Ralo", (100.0, 125.0), 7.0);
        seat(&home, &mut d).unwrap();
        assert!(d.elevation.abs() < 1e-9, "flush with the floor");
        let mut in_wall = d.clone();
        in_wall.position = Point2::new(100.0, 0.0);
        assert!(
            blocked(&home, &in_wall)
                .unwrap()
                .contains("dentro da parede")
        );
        let mut outside = d.clone();
        outside.position = Point2::new(100.0, 600.0);
        assert!(seat(&home, &mut outside).is_err());
        let mut cabinet_over = home.clone();
        cabinet_over.furniture.push(piece(
            43,
            "base-cabinet",
            "Gabinete",
            (100.0, 125.0),
            (60.0, 45.0, 85.0),
        ));
        assert!(
            hidden(&cabinet_over, &d)
                .unwrap()
                .contains("embaixo de Gabinete")
        );
        // The shower's own drain is where it belongs.
        let mut in_box = d.clone();
        in_box.position = Point2::new(150.0, 200.0);
        assert!(hidden(&home, &in_box).is_none());

        // An open terrace drains rainwater, never the sewer.
        let mut terrace = Home::default();
        terrace.rooms.push(Room::new(
            RoomId(5),
            "Terraço descoberto",
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(300.0, 0.0),
                Point2::new(300.0, 300.0),
                Point2::new(0.0, 300.0),
            ],
        ));
        terrace
            .furniture
            .push(point(50, "floor-drain", "Ralo", (150.0, 150.0), 0.0));
        assert!(keys(&terrace).contains(&"plumb:rain:r5".to_owned()));
        terrace.furniture[0].catalog = "rain-drain".into();
        assert!(!keys(&terrace).contains(&"plumb:rain:r5".to_owned()));
        assert!(!Pipe::Sewer.serves(PointKind::RainDrain));
    }
}
