//! Modular cabinets in sheet material: carcass, back in a groove, plinth,
//! dividers, shelves, drawers and doors with workshop clearances.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{MDF_WHITE, Output, Part, cm, num};

/// Front of a cabinet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoorType {
    /// Open shelving.
    None,
    /// Overlay doors on cup hinges.
    #[default]
    Hinged,
    /// Two or more leaves on a track.
    Sliding,
    /// Drawers over the whole height.
    Drawers,
}

/// A cabinet. Sizes in cm, board thicknesses in mm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CabinetParams {
    /// Width, cm (default 100).
    pub w: f64,
    /// Height including plinth, cm (default 210).
    pub h: f64,
    /// Depth including doors, cm (default 55).
    pub d: f64,
    /// Carcass and door board, mm: 15, 18 or 25 (default 18).
    pub t: f64,
    /// Back panel, mm, set in a groove (default 6).
    pub back: f64,
    pub door: DoorType,
    /// Number of doors or leaves (default: one per 60 cm).
    pub doors: Option<u32>,
    /// Shelves per bay (default 2).
    pub shelves: u32,
    /// Drawers at the bottom of each bay (with `door: drawers`, the drawer count).
    pub drawers: u32,
    /// Vertical dividers making bays (default: as many as the shelves need).
    pub dividers: Option<u32>,
    /// Plinth height, cm (default 10; 0 for wall cabinets).
    pub plinth: f64,
    /// Holds a cooktop on top: needs the depth for its niche.
    pub cooktop: bool,
    /// Rectangular opening through the wooden top, `[x, y, w, d]` cm from
    /// the cabinet's left-back corner. Match the appliance body and the
    /// countertop cutout; the cut list keeps one board with machining.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_cutout: Option<[f64; 4]>,
    /// Carcass color `[r,g,b]` (default white MDF).
    pub color: Option<[u8; 3]>,
    /// Door and drawer front finish, e.g. `wood` or `#5f6e4a`.
    pub front: Option<String>,
    /// Blind corner: cm of the front on the left covered by a fixed panel
    /// (where a cabinet run on the other wall meets this one).
    pub blind_left: f64,
    /// Blind corner on the right, cm.
    pub blind_right: f64,
    /// Wardrobe: a hanging rail under a top shelf (maleiro) instead of shelves.
    pub rod: bool,
    /// Open niches for built-in appliances (oven, microwave): no door across them.
    pub niches: Vec<Niche>,
    /// Handles on doors and drawer fronts (default `bar`).
    pub handle: HandleStyle,
    /// Handle color `[r,g,b]` or `#rrggbb` (default brushed steel).
    pub handle_color: Option<HandleColor>,
}

/// Handles on doors and drawer fronts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleStyle {
    /// Stainless bar pull on two posts.
    #[default]
    Bar,
    /// Aluminum edge profile (perfil puxador) along the front.
    Profile,
    /// Small knob.
    Knob,
    /// Finger groove along the front (cava).
    Cava,
    /// Push-to-open, no handle.
    None,
}

/// A color given as `[r,g,b]` or as `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum HandleColor {
    Rgb([u8; 3]),
    Hex(String),
}

/// A doorless opening across the cabinet's inside, cm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Niche {
    /// Height of its floor above the room floor.
    pub bottom: f64,
    /// Free height inside.
    pub height: f64,
}

impl Default for CabinetParams {
    fn default() -> Self {
        Self {
            w: 100.0,
            h: 210.0,
            d: 55.0,
            t: 18.0,
            back: 6.0,
            door: DoorType::Hinged,
            doors: None,
            shelves: 2,
            drawers: 0,
            dividers: None,
            plinth: 10.0,
            cooktop: false,
            top_cutout: None,
            color: None,
            front: None,
            blind_left: 0.0,
            blind_right: 0.0,
            rod: false,
            niches: Vec::new(),
            handle: HandleStyle::Bar,
            handle_color: None,
        }
    }
}

/// Gap between doors and around fronts, cm (2 mm, for cup hinges).
const GAP: f64 = 0.2;
/// Groove for the back, cm from the rear edge.
const BACK_INSET: f64 = 1.0;
/// Groove depth into sides, top and bottom, cm.
const GROOVE: f64 = 0.8;
/// Telescopic slide lengths on sale, cm.
const SLIDES: [f64; 8] = [25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0];
/// Clearance per side for drawer slides, cm.
const SLIDE_CLEARANCE: f64 = 1.3;
/// Widest single hinged door, cm.
const MAX_DOOR: f64 = 60.0;
/// Cooktops need this much depth for their niche, cm.
const COOKTOP_NICHE: f64 = 50.0;
/// Brushed stainless steel.
const STEEL: [u8; 3] = [190, 192, 196];
/// Shadow inside a finger groove.
const GROOVE_SHADOW: [u8; 3] = [46, 44, 42];
/// Doors at least this tall get a long handle at hand height, cm.
const TALL_DOOR: f64 = 120.0;
/// Center of a tall door's handle above the floor, cm.
const HAND_HEIGHT: f64 = 105.0;
/// How far a bar pull stands off the front, cm.
const BAR_REACH: f64 = 2.5;
/// Bar section, cm.
const BAR: f64 = 1.2;

/// Where a handle goes on one front.
#[derive(Debug, Clone, Copy)]
enum Grip {
    /// A door opening on the `right` side; `upper` for wall cabinets
    /// (handle near the bottom), `tall` for wardrobe and pantry doors.
    Door {
        right: bool,
        upper: bool,
        tall: bool,
    },
    /// Horizontal, centered near the top.
    Drawer,
    /// A sliding leaf, pulled at its `right` or left edge.
    Leaf { right: bool },
}

/// One front to put a handle on: `[x, width, z, height]` and its face plane y, cm.
struct Front<'a> {
    label: &'a str,
    rect: [f64; 4],
    face: f64,
    grip: Grip,
}

/// Handles bought, as `(description, count)` gathered for the hardware list.
#[derive(Default)]
struct Handles {
    bars: Vec<(u32, u32)>,
    knobs: u32,
    profile: (u32, f64),
    grooves: u32,
    push: u32,
}

impl Handles {
    fn lines(&self, hardware: &mut Vec<String>, notes: &mut Vec<String>) {
        let plural = |n: u32, one: &str, many: &str| {
            if n == 1 {
                format!("1 {one}")
            } else {
                format!("{n} {many}")
            }
        };
        for &(mm, n) in &self.bars {
            hardware.push(format!(
                "{} {mm} mm",
                plural(n, "puxador barra inox", "puxadores barra inox")
            ));
        }
        if self.knobs > 0 {
            hardware.push(plural(self.knobs, "puxador botão", "puxadores botão"));
        }
        if self.profile.0 > 0 {
            hardware.push(format!(
                "{} ({} cm no total)",
                plural(
                    self.profile.0,
                    "perfil puxador de alumínio",
                    "perfis puxador de alumínio"
                ),
                num(self.profile.1)
            ));
        }
        if self.grooves > 0 {
            notes.push(format!(
                "{} com cava usinada: peça a fresagem junto com o corte.",
                plural(self.grooves, "frente", "frentes")
            ));
        }
        if self.push > 0 {
            hardware.push(plural(
                self.push,
                "fecho toque (push-to-open)",
                "fechos toque (push-to-open)",
            ));
        }
    }

    fn bar(&mut self, mm: u32) {
        match self.bars.iter_mut().find(|(m, _)| *m == mm) {
            Some((_, n)) => *n += 1,
            None => self.bars.push((mm, 1)),
        }
    }
}

/// Hole spacing of a bar pull for a front `width` cm wide, mm.
fn bar_spacing(width: f64) -> u32 {
    match width {
        w if w <= 60.0 => 128,
        w if w <= 80.0 => 160,
        _ => 320,
    }
}

/// Adds the handle of one front to `parts`.
#[allow(clippy::too_many_lines)]
fn add_handle(
    style: HandleStyle,
    color: [u8; 3],
    front: &Front,
    parts: &mut Vec<Part>,
    count: &mut Handles,
) {
    let [x, width, z, height] = front.rect;
    let face = front.face;
    let label = front.label;
    // Vertical or horizontal, the handle's center and the length it may take.
    let (vertical, cx, cz, room) = match front.grip {
        Grip::Drawer => (false, x + width / 2.0, z + height - 3.5, width - 8.0),
        Grip::Leaf { right } => (
            true,
            if right { x + width - 3.5 } else { x + 3.5 },
            HAND_HEIGHT.clamp(z + height * 0.25, z + height * 0.75),
            height - 8.0,
        ),
        Grip::Door { right, upper, .. } => {
            let cx = if right { x + width - 3.5 } else { x + 3.5 };
            (true, cx, if upper { z } else { z + height }, height - 8.0)
        }
    };
    let steel = |name: String, at: [f64; 3], size: [f64; 3]| {
        let mut part = Part::solid(&name, at, size, color);
        part.finish = Some(newera_core::Material {
            color: Some(color),
            shininess: 0.8,
            ..newera_core::Material::default()
        });
        part
    };
    match style {
        HandleStyle::None => count.push += 1,
        HandleStyle::Cava | HandleStyle::Profile => {
            let (name, depth, y, tint) = if style == HandleStyle::Cava {
                count.grooves += 1;
                // A dark recess just proud of the face so it reads.
                (format!("Cava {label}"), 0.3, face - 0.28, GROOVE_SHADOW)
            } else {
                (format!("Puxador perfil {label}"), 0.5, face - 0.2, color)
            };
            let band = 1.8;
            let (at, size) = match front.grip {
                Grip::Drawer => ([x, y, z + height - band], [width, depth, band]),
                Grip::Door {
                    upper, tall: false, ..
                } => {
                    let bz = if upper { z } else { z + height - band };
                    ([x, y, bz], [width, depth, band])
                }
                // Tall doors and leaves: along the opening edge.
                Grip::Door { right, .. } | Grip::Leaf { right } => (
                    [if right { x + width - band } else { x }, y, z],
                    [band, depth, height],
                ),
            };
            if style == HandleStyle::Profile {
                count.profile.0 += 1;
                count.profile.1 += size[0].max(size[2]);
                parts.push(steel(name, at, size));
            } else {
                parts.push(Part::solid(&name, at, size, tint));
            }
        }
        HandleStyle::Knob => {
            count.knobs += 1;
            let cz = match front.grip {
                Grip::Door { tall: true, .. } => {
                    HAND_HEIGHT.clamp(z + 5.0, (z + height - 5.0).max(z + 5.0))
                }
                Grip::Door { upper: true, .. } => cz + 5.0,
                Grip::Door { .. } => cz - 5.0,
                _ => cz,
            };
            let s = 2.4;
            parts.push(steel(
                format!("Puxador {label}"),
                [cx - s / 2.0, face, cz - s / 2.0],
                [s, BAR_REACH, s],
            ));
        }
        HandleStyle::Bar => {
            let tall = matches!(
                front.grip,
                Grip::Door { tall: true, .. } | Grip::Leaf { .. }
            );
            let mut spacing = if tall {
                384
            } else {
                bar_spacing(if vertical { 60.0 } else { width })
            };
            // Short or narrow fronts take the next size down.
            while spacing > 96 && f64::from(spacing) / 10.0 + 4.0 > room {
                spacing = match spacing {
                    384 => 320,
                    320 => 160,
                    160 => 128,
                    _ => 96,
                };
            }
            count.bar(spacing);
            let gap = f64::from(spacing) / 10.0;
            let length = gap + 4.0;
            let cz = match front.grip {
                Grip::Door { tall: true, .. } => HAND_HEIGHT.clamp(
                    z + 4.0 + length / 2.0,
                    (z + height - 4.0 - length / 2.0).max(z + 4.0 + length / 2.0),
                ),
                Grip::Door { upper: true, .. } => cz + 4.0 + length / 2.0,
                Grip::Door { .. } => cz - 4.0 - length / 2.0,
                _ => cz,
            };
            let post = 1.0;
            let off = face + BAR_REACH - BAR;
            if vertical {
                parts.push(steel(
                    format!("Puxador {label}"),
                    [cx - BAR / 2.0, off, cz - length / 2.0],
                    [BAR, BAR, length],
                ));
                for dz in [-gap / 2.0, gap / 2.0] {
                    parts.push(steel(
                        format!("Apoio do puxador {label}"),
                        [cx - post / 2.0, face, cz + dz - post / 2.0],
                        [post, off - face, post],
                    ));
                }
            } else {
                parts.push(steel(
                    format!("Puxador {label}"),
                    [cx - length / 2.0, off, cz - BAR / 2.0],
                    [length, BAR, BAR],
                ));
                for dx in [-gap / 2.0, gap / 2.0] {
                    parts.push(steel(
                        format!("Apoio do puxador {label}"),
                        [cx + dx - post / 2.0, face, cz - post / 2.0],
                        [post, off - face, post],
                    ));
                }
            }
        }
    }
}

/// Where shelves go: the whole zone, or with niches the tallest stretch
/// left between them. Returns `(floor, height)`.
fn shelves_zone(niches: &[(f64, f64)], floor: f64, zone: f64, t: f64) -> (f64, f64) {
    let mut best = (floor, if niches.is_empty() { zone } else { 0.0 });
    let mut from = floor;
    for &(nb, nt) in niches {
        let height = (nb - t - from).max(0.0);
        if height > best.1 {
            best = (from, height);
        }
        from = nt + t;
    }
    if !niches.is_empty() {
        let height = (floor + zone - from).max(0.0);
        if height > best.1 {
            best = (from, height);
        }
    }
    best
}

/// Cup hinges a door of `height` cm needs.
fn hinges(height: f64) -> u32 {
    match height {
        h if h <= 90.0 => 2,
        h if h <= 160.0 => 3,
        h if h <= 220.0 => 4,
        _ => 5,
    }
}

/// Builds the parts of a cabinet from `p`.
///
/// What a workshop would say about the request is said, not enforced: a board
/// thickness nobody stocks, a shelf that will sag, a drawer front too short
/// to grip, a niche the cooktop standard wants deeper — all of it is built
/// and noted. Somebody learning, sketching or just looking at an idea is not
/// stopped by the workshop's opinion; the notes are there when the drawing
/// becomes something to cut.
///
/// It refuses only what has no geometry at all: a box whose inside is zero or
/// negative, a part that would come out backwards. That is not a judgement on
/// the project, it is arithmetic.
#[allow(clippy::too_many_lines)]
pub(crate) fn generate(p: &CabinetParams) -> Result<Output, String> {
    let mut asked = p.clone();
    let p = &mut asked;
    let mut notes: Vec<String> = Vec::new();
    let (w, h, d) = (p.w, p.h, p.d);
    if ![15.0, 18.0, 25.0].contains(&p.t) {
        notes.push(format!(
            "Chapa de {} mm não é padrão para caixaria (15, 18 ou 25 mm): a marcenaria vai ter de cortar sob encomenda.",
            num(p.t)
        ));
    }
    if p.t <= 0.0 {
        return Err("A chapa precisa de espessura maior que zero.".into());
    }
    let t = cm(p.t);
    let back = cm(p.back.clamp(3.0, 18.0));
    let board = format!("MDF {}", num(p.t));
    let back_board = format!("MDF {}", num(p.back));
    let carcass = p.color.unwrap_or(MDF_WHITE);
    let front_finish = p
        .front
        .as_deref()
        .map(str::parse::<newera_core::Material>)
        .transpose()?;
    // Fronts a shade off the carcass so the reveals between them read.
    let front_color = carcass.map(|c| (f64::from(c) * 0.95).round() as u8);
    let handle_color = match &p.handle_color {
        None => STEEL,
        Some(HandleColor::Rgb(c)) => *c,
        Some(HandleColor::Hex(text)) => text
            .parse::<newera_core::Material>()
            .ok()
            .and_then(|m| m.color)
            .ok_or_else(|| {
                format!("Cor de puxador `{text}` não reconhecida; use #rrggbb ou [r,g,b].")
            })?,
    };
    let mut handles = Handles::default();
    // Wall cabinets hang without a plinth: handles near the bottom.
    let wall_cabinet = p.plinth <= 0.0 && h <= TALL_DOOR;
    if w <= 2.0 * t || h <= 2.0 * t || d <= t {
        return Err(format!(
            "Armário de {} × {} × {} cm não tem lado de dentro: com chapa de {} mm as peças sairiam ao contrário.",
            num(w),
            num(d),
            num(h),
            num(p.t)
        ));
    }
    if w < 2.0 * t + 10.0 || h < 20.0 || d < 20.0 {
        notes.push(format!(
            "Armário de {} × {} × {} cm é menor do que se faz na prática (a partir de {} cm de largura e 20 de altura e profundidade).",
            num(w),
            num(d),
            num(h),
            num(2.0 * t + 10.0)
        ));
    }
    if p.cooktop && d < COOKTOP_NICHE {
        notes.push(format!(
            "Profundidade de {} cm com cooktop: a norma do aparelho pede nicho de {} cm, e a 55 cm o tampo respira.",
            num(d),
            num(COOKTOP_NICHE)
        ));
    }
    let (mut blind_l, mut blind_r) = (p.blind_left.max(0.0), p.blind_right.max(0.0));
    if blind_l + blind_r > 0.0 {
        // A blind corner is a fixed panel beside a hinged door; with drawers
        // or sliding leaves there is nothing to hold it, so the cabinet is
        // built without it rather than not built at all.
        if p.door != DoorType::Hinged || p.drawers > 0 {
            notes.push(
                "Canto cego pede portas de giro e nenhuma gaveta: o armário saiu sem o painel cego."
                    .to_owned(),
            );
            blind_l = 0.0;
            blind_r = 0.0;
        } else if w - blind_l - blind_r < 30.0 {
            notes.push(format!(
                "Com {} cm de painel cego sobram {} cm de porta; abaixo de 30 cm a folha não serve para nada.",
                num(blind_l + blind_r),
                num(w - blind_l - blind_r)
            ));
        }
    }
    let plinth = p.plinth.max(0.0);
    let front_t = match p.door {
        DoorType::None => 0.0,
        DoorType::Hinged | DoorType::Drawers => t,
        // Two tracks of leaves.
        DoorType::Sliding => 2.0 * t + 1.0,
    };
    let dc = d - front_t;
    let hc = h - plinth;
    if dc <= t || hc <= 2.0 * t {
        return Err(format!(
            "Com frentes de {} cm e rodapé de {} cm não sobra caixa ({} × {} cm): aumente d ou h, ou reduza plinth.",
            num(front_t),
            num(plinth),
            num(dc),
            num(hc)
        ));
    }
    if dc < 20.0 || hc < 2.0 * t + 10.0 {
        notes.push(format!(
            "Com frentes de {} cm e rodapé de {} cm a caixa fica com {} × {} cm — pouco para guardar qualquer coisa.",
            num(front_t),
            num(plinth),
            num(dc),
            num(hc)
        ));
    }
    let (iw, ih) = (w - 2.0 * t, hc - 2.0 * t);
    let id = dc - BACK_INSET - back;
    let mut parts = Vec::new();
    let mut hardware = Vec::new();

    // Carcass.
    parts.push(
        Part::board(
            "Lateral esquerda",
            [0.0, 0.0, plinth],
            [t, dc, hc],
            &board,
            carcass,
        )
        .banded(1, 0),
    );
    parts.push(
        Part::board(
            "Lateral direita",
            [w - t, 0.0, plinth],
            [t, dc, hc],
            &board,
            carcass,
        )
        .banded(1, 0),
    );
    parts.push(Part::board("Base", [t, 0.0, plinth], [iw, dc, t], &board, carcass).banded(1, 0));
    let mut extra_cuts = Vec::new();
    let mut top = Part::board("Tampo", [t, 0.0, h - t], [iw, dc, t], &board, carcass).banded(1, 0);
    if let Some([x, y, width, depth]) = p.top_cutout {
        if ![x, y, width, depth].iter().all(|v| v.is_finite())
            || width <= 0.0
            || depth <= 0.0
            || x < t
            || y < 0.0
            || x + width > w - t
            || y + depth > dc
        {
            return Err("O recorte superior deve caber entre as laterais e dentro da profundidade da caixa; use top_cutout = [x, y, largura, profundidade] em cm.".into());
        }
        // Four disjoint solids leave an actual void in the rendered model.
        // They are not four boards to manufacture: retain one machined top.
        for (label, at, size) in [
            ("esquerda", [t, 0.0, h - t], [x - t, dc, t]),
            (
                "direita",
                [x + width, 0.0, h - t],
                [w - t - x - width, dc, t],
            ),
            ("atrás", [x, 0.0, h - t], [width, y, t]),
            ("frente", [x, y + depth, h - t], [width, dc - y - depth, t]),
        ] {
            if size[0] > 0.0 && size[1] > 0.0 {
                let mut strip = Part::board(
                    &format!("Tampo — {label} do recorte"),
                    at,
                    size,
                    &board,
                    carcass,
                );
                strip.board = None;
                parts.push(strip);
            }
        }
        top.holes.push([x - t, y, width, depth]);
        extra_cuts.push(top);
    } else {
        parts.push(top);
        if p.cooktop {
            notes.push("Armário para cooktop sem recorte no tampo de madeira: defina top_cutout = [x, y, largura, profundidade] conforme o corpo do aparelho; recortar só a pedra não libera a caixa abaixo.".into());
        }
    }
    parts.push(Part::board(
        "Fundo",
        [t - GROOVE, BACK_INSET, plinth + t - GROOVE],
        [iw + 2.0 * GROOVE, back, ih + 2.0 * GROOVE],
        &back_board,
        carcass,
    ));
    if plinth > 0.0 {
        parts.push(
            Part::board(
                "Rodapé frontal",
                [t, dc - 5.0 - t, 0.0],
                [iw, t, plinth],
                &board,
                carcass,
            )
            .banded(1, 0),
        );
        parts.push(Part::board(
            "Travessa traseira do rodapé",
            [t, BACK_INSET, 0.0],
            [iw, t, plinth],
            &board,
            carcass,
        ));
    }

    // Bays: shelves longer than the board can span get dividers.
    let span_limit = if p.t >= 25.0 {
        120.0
    } else if p.t >= 18.0 {
        90.0
    } else {
        70.0
    };
    // Hanging rails sag past about a meter.
    let span_limit = if p.rod {
        f64::min(span_limit, 100.0)
    } else {
        span_limit
    };
    let needed = if p.shelves > 0 || p.rod {
        (((iw + t) / (span_limit + t)).ceil() as u32).saturating_sub(1)
    } else {
        0
    };
    // A niche takes the whole inside: no automatic dividers through it.
    let dividers = p
        .dividers
        .unwrap_or(if p.niches.is_empty() { needed } else { 0 });
    let bays = dividers + 1;
    let bay = (iw - f64::from(dividers) * t) / f64::from(bays);
    if bay <= 0.0 {
        return Err(format!(
            "{dividers} divisões não cabem em {} cm de vão interno.",
            num(iw)
        ));
    }
    if bay < 15.0 {
        notes.push(format!(
            "Com {} divisões cada vão fica com {} cm; abaixo de 15 cm não entra nada de pé.",
            dividers,
            num(bay)
        ));
    }
    if bay > span_limit && (p.shelves > 0 || p.rod) {
        notes.push(format!(
            "Prateleiras de {} mm vencem {} cm e cada vão tem {} cm: vão barrigar com peso. dividers = {} ou chapa mais grossa resolve.",
            num(p.t),
            num(span_limit),
            num(bay),
            needed.max(1)
        ));
    }
    for k in 0..dividers {
        let x = t + f64::from(k + 1) * bay + f64::from(k) * t;
        parts.push(
            Part::board(
                &format!("Divisória {}", k + 1),
                [x, BACK_INSET + back, plinth + t],
                [t, id, ih],
                &board,
                carcass,
            )
            .banded(1, 0),
        );
    }

    // Drawers: the whole front, or a stack at the bottom of each bay.
    let drawer_count = if p.door == DoorType::Drawers {
        p.drawers.max(1)
    } else {
        p.drawers
    };
    let drawer_zone = if p.door == DoorType::Drawers {
        ih
    } else {
        f64::from(drawer_count) * 18.0
    };
    let mut drawer_count = drawer_count;
    let mut drawer_zone = drawer_zone;
    if drawer_zone > ih {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let fits = (ih / 18.0).floor().max(0.0) as u32;
        notes.push(format!(
            "{drawer_count} gavetas de 18 cm não cabem no vão interno de {} cm: saíram {fits}.",
            num(ih)
        ));
        drawer_count = fits;
        drawer_zone = f64::from(fits) * 18.0;
    }
    if drawer_count > 0 {
        let front_h = drawer_zone / f64::from(drawer_count);
        if front_h < 12.0 {
            notes.push(format!(
                "Frente de gaveta com {} cm: abaixo de 12 cm não há onde pegar.",
                num(front_h)
            ));
        }
        let slide = SLIDES.iter().rev().copied().find(|s| *s <= id - 2.0).ok_or_else(|| {
            format!(
                "A profundidade interna de {} cm não comporta corrediça de 25 cm; aumente d para pelo menos {} cm.",
                num(id),
                num(d + (27.0 - id))
            )
        })?;
        let box_w = bay - 2.0 * SLIDE_CLEARANCE;
        let side = cm(15.0);
        for b in 0..bays {
            let x0 = t + f64::from(b) * (bay + t);
            for k in 0..drawer_count {
                let z0 = plinth + t + f64::from(k) * front_h;
                let label = if bays > 1 {
                    format!(" {}.{}", b + 1, k + 1)
                } else {
                    format!(" {}", k + 1)
                };
                let box_h = front_h - 4.0;
                let y0 = dc - slide;
                parts.push(
                    Part::board(
                        &format!("Gaveta{label} lateral esq."),
                        [x0 + SLIDE_CLEARANCE, y0, z0 + 2.0],
                        [side, slide, box_h],
                        "MDF 15",
                        MDF_WHITE,
                    )
                    .banded(1, 0),
                );
                parts.push(
                    Part::board(
                        &format!("Gaveta{label} lateral dir."),
                        [x0 + bay - SLIDE_CLEARANCE - side, y0, z0 + 2.0],
                        [side, slide, box_h],
                        "MDF 15",
                        MDF_WHITE,
                    )
                    .banded(1, 0),
                );
                parts.push(Part::board(
                    &format!("Gaveta{label} traseira"),
                    [x0 + SLIDE_CLEARANCE + side, y0, z0 + 2.0],
                    [box_w - 2.0 * side, side, box_h],
                    "MDF 15",
                    MDF_WHITE,
                ));
                parts.push(Part::board(
                    &format!("Gaveta{label} contrafrente"),
                    [x0 + SLIDE_CLEARANCE + side, dc - side, z0 + 2.0],
                    [box_w - 2.0 * side, side, box_h],
                    "MDF 15",
                    MDF_WHITE,
                ));
                parts.push(Part::board(
                    &format!("Gaveta{label} fundo"),
                    [x0 + SLIDE_CLEARANCE, y0, z0 + 1.4],
                    [box_w, slide, cm(6.0)],
                    "MDF 6",
                    MDF_WHITE,
                ));
                if front_t > 0.0 {
                    let mut front = Part::board(
                        &format!("Frente da gaveta{label}"),
                        [x0 - t / 2.0 + GAP, dc, z0 + GAP],
                        [bay + t - 2.0 * GAP, t, front_h - 2.0 * GAP],
                        &board,
                        front_color,
                    )
                    .banded(2, 2);
                    front.finish.clone_from(&front_finish);
                    let rect = [front.at[0], front.size[0], front.at[2], front.size[2]];
                    parts.push(front);
                    add_handle(
                        p.handle,
                        handle_color,
                        &Front {
                            label: &format!("da gaveta{label}"),
                            rect,
                            face: d,
                            grip: Grip::Drawer,
                        },
                        &mut parts,
                        &mut handles,
                    );
                }
            }
        }
        hardware.push(format!(
            "{} pares de corrediças telescópicas de {} cm",
            drawer_count * bays,
            num(slide)
        ));
    }

    // Niches: fixed boards around each, no shelves or doors across them.
    let mut niches: Vec<(f64, f64)> = p
        .niches
        .iter()
        .map(|n| (n.bottom, n.bottom + n.height))
        .collect();
    niches.sort_by(|a, b| a.0.total_cmp(&b.0));
    // A niche is a hole through the carcass: it needs the whole bay and a
    // front that is not in the way. Where the request cannot have both, the
    // cabinet is built without the niche and says so.
    if !niches.is_empty() {
        let no_room = if p.rod {
            Some("Cabideiro e nicho no mesmo módulo não combinam: saiu sem o nicho.".to_owned())
        } else if !matches!(p.door, DoorType::Hinged | DoorType::None) {
            Some("Nicho pede porta de giro ou armário aberto: saiu sem o nicho.".to_owned())
        } else if bays > 1 {
            Some(format!(
                "O nicho ocupa o vão inteiro e há {} divisões: saiu sem o nicho (vão interno de {} cm).",
                bays - 1,
                num(iw)
            ))
        } else {
            None
        };
        if let Some(why) = no_room {
            notes.push(why);
            niches.clear();
        }
    }
    let floor = plinth + t + drawer_zone;
    // A niche outside the carcass is moved into it rather than refused: the
    // drawing keeps the idea, and the note keeps the number that was asked.
    let mut placed: Vec<(f64, f64)> = Vec::new();
    for &(nb, nt) in &niches {
        let height = (nt - nb).min(h - t - floor).max(0.0);
        if height <= 0.0 {
            notes.push(format!(
                "Não sobra altura para o nicho entre {} e {} cm do chão; saiu sem ele.",
                num(floor),
                num(h - t)
            ));
            continue;
        }
        let mut bottom = nb.clamp(floor, h - t - height);
        if let Some(&(_, last_top)) = placed.last()
            && bottom < last_top + t
        {
            bottom = last_top + t;
        }
        let top = bottom + height;
        if top > h - t + 0.05 {
            notes.push(format!(
                "O nicho de {} cm não cabe acima de {} cm; saiu sem ele.",
                num(height),
                num(bottom)
            ));
            continue;
        }
        if (bottom - nb).abs() > 0.05 {
            notes.push(format!(
                "Nicho pedido a {} cm do chão: saiu a {} cm, que é onde o armário tem lugar para ele.",
                num(nb),
                num(bottom)
            ));
        }
        placed.push((bottom, top));
    }
    let niches = placed;

    for (k, &(nb, nt)) in niches.iter().enumerate() {
        let below_is_board = k > 0 && (nb - t - (niches[k - 1].1)).abs() < 0.5;
        if nb - t > floor + 0.5 && !below_is_board {
            parts.push(
                Part::board(
                    "Base do nicho",
                    [t, BACK_INSET + back, nb - t],
                    [iw, id, t],
                    &board,
                    carcass,
                )
                .banded(1, 0),
            );
        }
        if nt + t < h - t - 0.5 {
            parts.push(
                Part::board(
                    "Topo do nicho",
                    [t, BACK_INSET + back, nt],
                    [iw, id, t],
                    &board,
                    carcass,
                )
                .banded(1, 0),
            );
        }
        notes.push(format!(
            "Nicho livre de {} × {} × {} cm (largura × altura × profundidade) a {} cm do chão.",
            num(iw),
            num(nt - nb),
            num(id + back + BACK_INSET),
            num(nb)
        ));
    }

    // Shelves above the drawers, evenly spaced.
    let shelf_zone = ih
        - if p.door == DoorType::Drawers {
            ih
        } else {
            drawer_zone
        };
    if p.rod && p.door != DoorType::Drawers {
        // Top shelf for suitcases, the rail 6 cm under it.
        let shelf_z = plinth + hc - t - 35.0;
        let rail_z = shelf_z - 6.0;
        let hanging = rail_z - (plinth + t + drawer_zone);
        if hanging < 95.0 {
            notes.push(format!(
                "Cabideiro com {} cm livres: camisa pede 95 e casaco 140. h = {} resolveria.",
                num(hanging),
                num(h + 95.0 - hanging)
            ));
        }
        for b in 0..bays {
            let x0 = t + f64::from(b) * (bay + t);
            let suffix = if bays > 1 {
                format!(" {}", b + 1)
            } else {
                String::new()
            };
            parts.push(
                Part::board(
                    &format!("Maleiro{suffix}"),
                    [x0 + 0.1, BACK_INSET + back, shelf_z],
                    [bay - 0.2, id - 1.0, t],
                    &board,
                    carcass,
                )
                .banded(1, 0),
            );
            let mut rail = Part::solid(
                &format!("Tubo cabideiro{suffix}"),
                [x0 + 0.2, BACK_INSET + back + (id - 2.5) / 2.0, rail_z],
                [bay - 0.4, 2.5, 1.5],
                [180, 180, 185],
            );
            rail.finish = "#b4b4b9".parse().ok();
            parts.push(rail);
        }
        hardware.push(format!(
            "{bays} tubo(s) oblongo(s) de {} cm com {} suportes",
            num(bay - 0.4),
            bays * 2
        ));
        if hanging < 140.0 {
            notes.push(format!(
                "Cabideiro com {} cm livres: bom para camisas; vestidos e casacos longos pedem 150 cm.",
                num(hanging)
            ));
        }
    } else if p.shelves > 0 && p.door != DoorType::Drawers {
        let (shelf_floor, shelf_zone) =
            shelves_zone(&niches, plinth + t + drawer_zone, shelf_zone, t);
        let step = shelf_zone / f64::from(p.shelves + 1);
        if step <= 0.0 {
            return Err(format!(
                "{} prateleiras não cabem em {} cm de vão.",
                p.shelves,
                num(shelf_zone)
            ));
        }
        if step < 15.0 {
            notes.push(format!(
                "{} prateleiras deixam {} cm entre elas; abaixo de 15 cm não entra prato de pé.",
                p.shelves,
                num(step)
            ));
        }
        for b in 0..bays {
            let x0 = t + f64::from(b) * (bay + t);
            for k in 0..p.shelves {
                let z = shelf_floor + f64::from(k + 1) * step - t / 2.0;
                parts.push(
                    Part::board(
                        &format!(
                            "Prateleira {}",
                            if bays > 1 {
                                format!("{}.{}", b + 1, k + 1)
                            } else {
                                (k + 1).to_string()
                            }
                        ),
                        [x0 + 0.1, BACK_INSET + back, z],
                        [bay - 0.2, id - 1.0, t],
                        &board,
                        carcass,
                    )
                    .banded(1, 0),
                );
            }
        }
        hardware.push(format!("{} suportes de prateleira", p.shelves * bays * 4));
        if bay > span_limit * 0.8 {
            notes.push(format!(
                "Prateleiras de {} cm estão perto do limite de {} cm para {} mm; evite carga pesada.",
                num(bay),
                num(span_limit),
                num(p.t)
            ));
        }
    }

    // Doors over the part without drawers.
    let door_h = hc - drawer_zone;
    match p.door {
        DoorType::Hinged if door_h > 10.0 => {
            let open = w - blind_l - blind_r;
            let n = p
                .doors
                .unwrap_or_else(|| (open / MAX_DOOR).ceil() as u32)
                .max(1);
            let leaf = (open - GAP * f64::from(n + 1)) / f64::from(n);
            if leaf > MAX_DOOR {
                notes.push(format!(
                    "Porta de giro de {} cm: acima de {} cm a folha empena e força as dobradiças; doors = {} divide.",
                    num(leaf),
                    num(MAX_DOOR),
                    (open / MAX_DOOR).ceil() as u32
                ));
            }
            // Doors cover the front, except across a niche.
            let segments: Vec<(f64, f64, &str)> = {
                let mut out = Vec::new();
                let mut from = plinth + drawer_zone;
                for (k, &(nb, nt)) in niches.iter().enumerate() {
                    out.push((
                        from,
                        nb,
                        if k == 0 {
                            "Porta"
                        } else {
                            "Porta entre nichos"
                        },
                    ));
                    from = nt;
                }
                out.push((
                    from,
                    plinth + hc,
                    if niches.is_empty() {
                        "Porta"
                    } else {
                        "Porta superior"
                    },
                ));
                out.into_iter().filter(|(a, b, _)| b - a > 10.0).collect()
            };
            if blind_l + blind_r > 0.0 {
                notes.push(format!(
                    "Canto cego: {} cm de painel fixo; acesso pela porta de {} cm.",
                    num(blind_l + blind_r),
                    num(open)
                ));
            }
            let mut hinge_count = 0;
            for (bottom, top, label) in segments {
                let z0 = bottom + GAP;
                let height = top - bottom - 2.0 * GAP;
                for (name, x0, width) in [
                    ("Painel cego esquerdo", 0.0, blind_l),
                    ("Painel cego direito", w - blind_r, blind_r),
                ] {
                    if width > 0.0 {
                        let mut panel = Part::board(
                            name,
                            [x0 + GAP, dc, z0],
                            [width - GAP, t, height],
                            &board,
                            front_color,
                        )
                        .banded(2, 2);
                        panel.finish.clone_from(&front_finish);
                        parts.push(panel);
                    }
                }
                for k in 0..n {
                    let x = blind_l + GAP + f64::from(k) * (leaf + GAP);
                    let mut door = Part::board(
                        &format!("{label} {}", k + 1),
                        [x, dc, z0],
                        [leaf, t, height],
                        &board,
                        front_color,
                    )
                    .banded(2, 2);
                    door.finish.clone_from(&front_finish);
                    parts.push(door);
                    // Pairs meet in the middle; a lone door beside a blind
                    // panel hinges on the far side.
                    let right = if n == 1 {
                        blind_l <= 0.0
                    } else {
                        k % 2 == 0 && k + 1 < n
                    };
                    add_handle(
                        p.handle,
                        handle_color,
                        &Front {
                            label: &format!("{label} {}", k + 1),
                            rect: [x, leaf, z0, height],
                            face: d,
                            grip: Grip::Door {
                                right,
                                upper: wall_cabinet || z0 > 110.0,
                                tall: height >= TALL_DOOR,
                            },
                        },
                        &mut parts,
                        &mut handles,
                    );
                }
                hinge_count += n * hinges(height);
            }
            hardware.push(format!(
                "{hinge_count} dobradiças de caneco 35 mm (folga de 2 mm entre portas)"
            ));
        }
        DoorType::Sliding if door_h > 10.0 => {
            let n = p.doors.unwrap_or(2).max(2);
            let overlap = 3.0;
            let leaf = (w + overlap * f64::from(n - 1)) / f64::from(n);
            for k in 0..n {
                let x = f64::from(k) * (leaf - overlap);
                let y = dc + if k % 2 == 0 { t + 1.0 } else { 0.0 };
                let mut door = Part::board(
                    &format!("Folha de correr {}", k + 1),
                    [x, y, plinth + drawer_zone],
                    [leaf, t, door_h],
                    &board,
                    front_color,
                )
                .banded(2, 2);
                door.finish.clone_from(&front_finish);
                parts.push(door);
                // Leaves slide past each other: flush pulls only.
                let style = match p.handle {
                    HandleStyle::Bar | HandleStyle::Knob => HandleStyle::Profile,
                    other => other,
                };
                add_handle(
                    style,
                    handle_color,
                    &Front {
                        label: &format!("da folha {}", k + 1),
                        rect: [x, leaf, plinth + drawer_zone, door_h],
                        face: y + t,
                        grip: Grip::Leaf { right: k > 0 },
                    },
                    &mut parts,
                    &mut handles,
                );
            }
            hardware.push(format!(
                "trilho de correr duplo de {} cm (superior e inferior)",
                num(w)
            ));
        }
        _ => {}
    }
    handles.lines(&mut hardware, &mut notes);

    Ok(Output {
        parts,
        size: [w, d, h],
        hardware,
        notes,
        extra_cuts,
        name: format!("Armário {} × {} × {} cm", num(w), num(d), num(h)),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    fn part<'a>(out: &'a Output, name: &str) -> &'a Part {
        out.parts
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
    }

    #[test]
    fn a_cooktop_cutout_is_a_real_void_and_one_machined_board() {
        let p = CabinetParams {
            w: 70.0,
            h: 87.0,
            d: 60.0,
            shelves: 0,
            cooktop: true,
            top_cutout: Some([4.0, 4.0, 62.0, 52.0]),
            ..CabinetParams::default()
        };
        let out = generate(&p).unwrap();
        let top: Vec<_> = out
            .parts
            .iter()
            .filter(|p| p.name.starts_with("Tampo"))
            .collect();
        assert_eq!(top.len(), 4);
        let area: f64 = top.iter().map(|p| p.size[0] * p.size[1]).sum();
        assert!((area - ((70.0 - 3.6) * (60.0 - 1.8) - 62.0 * 52.0)).abs() < 1e-8);
        for part in top {
            assert!(part.board.is_none());
            assert!(
                part.at[0] + part.size[0] <= 4.0
                    || part.at[0] >= 66.0
                    || part.at[1] + part.size[1] <= 4.0
                    || part.at[1] >= 56.0
            );
        }
        assert_eq!(out.extra_cuts.len(), 1);
        assert_eq!(out.extra_cuts[0].holes, vec![[2.2, 4.0, 62.0, 52.0]]);
        let restored: CabinetParams =
            serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(restored.top_cutout, p.top_cutout);
        for cut in [
            [0.0, 4.0, 62.0, 52.0],
            [4.0, 4.0, 70.0, 52.0],
            [4.0, 4.0, 62.0, -1.0],
            [4.0, 4.0, 62.0, 60.0],
        ] {
            assert!(
                generate(&CabinetParams {
                    top_cutout: Some(cut),
                    ..p.clone()
                })
                .is_err()
            );
        }
        let closed = generate(&CabinetParams {
            top_cutout: None,
            ..p
        })
        .unwrap();
        assert!(closed.notes.iter().any(|n| n.contains("sem recorte")));
        assert!(closed.parts.iter().any(|p| p.name == "Tampo"));
    }

    #[test]
    fn wardrobe_boards_clearances_and_hardware() {
        let out = generate(&CabinetParams {
            w: 200.0,
            h: 240.0,
            d: 60.0,
            dividers: Some(2),
            shelves: 4,
            ..CabinetParams::default()
        })
        .unwrap();
        // 18 mm sides full carcass height (240 − 10 plinth), depth 60 − 1,8 door.
        let side = part(&out, "Lateral esquerda");
        assert_eq!(side.size, [1.8, 58.2, 230.0]);
        // Back 6 mm in 8 mm grooves: 200 − 3,6 + 1,6 wide.
        let back = part(&out, "Fundo");
        assert!((back.size[0] - 198.0).abs() < 1e-9 && (back.size[1] - 0.6).abs() < 1e-9);
        // Four doors of 60 cm max with 2 mm gaps: (200 − 5 × 0,2) / 4 = 49,75.
        let doors: Vec<&Part> = out
            .parts
            .iter()
            .filter(|p| p.name.starts_with("Porta"))
            .collect();
        assert_eq!(doors.len(), 4);
        assert!((doors[0].size[0] - 49.75).abs() < 1e-9);
        assert!(
            (doors[1].at[0] - doors[0].at[0] - 49.95).abs() < 1e-9,
            "2 mm between doors"
        );
        // Door height 230 − 0,4 needs 5 hinges each.
        assert!(
            out.hardware.iter().any(|h| h.starts_with("20 dobradiças")),
            "{:?}",
            out.hardware
        );
        // 3 bays × 4 shelves.
        assert_eq!(
            out.parts
                .iter()
                .filter(|p| p.name.starts_with("Prateleira"))
                .count(),
            12
        );
        let bay = (200.0 - 3.6 - 2.0 * 1.8) / 3.0;
        assert!((part(&out, "Prateleira 1.1").size[0] - (bay - 0.2)).abs() < 1e-9);
    }

    #[test]
    fn drawers_get_slides_that_fit_and_rules_explain_themselves() {
        let out = generate(&CabinetParams {
            w: 60.0,
            h: 85.0,
            d: 55.0,
            door: DoorType::Drawers,
            drawers: 3,
            ..CabinetParams::default()
        })
        .unwrap();
        // Inner depth 55 − 1,8 − 1 − 0,6 = 51,6 → 45 cm slides (51,6 − 2 = 49,6 ≥ 45).
        assert!(
            out.hardware
                .iter()
                .any(|h| h == "3 pares de corrediças telescópicas de 45 cm"),
            "{:?}",
            out.hardware
        );
        let side = part(&out, "Gaveta 1 lateral esq.");
        // Box fits the bay: 60 − 3,6 − 2 × 1,3 wide.
        let box_w = part(&out, "Gaveta 1 fundo").size[0];
        assert!((box_w - (60.0 - 3.6 - 2.6)).abs() < 1e-9);
        assert_eq!(side.size[1], 45.0);

        // What a workshop would say is said, and the cabinet is built anyway:
        // somebody sketching an idea is not stopped by the shop's opinion.
        let cooktop = generate(&CabinetParams {
            d: 35.0,
            cooktop: true,
            ..CabinetParams::default()
        })
        .unwrap();
        assert!(
            cooktop.notes.iter().any(|n| n.contains("cooktop")),
            "{:?}",
            cooktop.notes
        );
        let wide = generate(&CabinetParams {
            w: 150.0,
            doors: Some(2),
            shelves: 0,
            ..CabinetParams::default()
        })
        .unwrap();
        assert!(
            wide.notes.iter().any(|n| n.contains("doors = 3")),
            "{:?}",
            wide.notes
        );
        // Without dividers the shelves get them automatically…
        let auto = generate(&CabinetParams {
            w: 160.0,
            t: 15.0,
            ..CabinetParams::default()
        })
        .unwrap();
        assert_eq!(
            auto.parts
                .iter()
                .filter(|p| p.name.starts_with("Divisória"))
                .count(),
            2
        );
        // …and an explicit choice that will sag is built, with the warning.
        let span = generate(&CabinetParams {
            w: 160.0,
            t: 15.0,
            dividers: Some(0),
            ..CabinetParams::default()
        })
        .unwrap();
        assert!(
            span.notes.iter().any(|n| n.contains("dividers = 2")),
            "{:?}",
            span.notes
        );
        // More drawers than the box holds: it gets the ones that fit.
        let many = generate(&CabinetParams {
            h: 80.0,
            drawers: 5,
            ..CabinetParams::default()
        })
        .unwrap();
        assert!(
            many.notes.iter().any(|n| n.contains("não cabem")),
            "{:?}",
            many.notes
        );
        let drawers = |out: &Output, n: u32| {
            out.parts
                .iter()
                .any(|p| p.name.starts_with(&format!("Gaveta 1.{n} ")))
        };
        assert!(
            drawers(&many, 2) && !drawers(&many, 4),
            "the drawers that fit were built"
        );
        // A drawer needs a slide that exists: that one has no answer at all.
        let shallow = generate(&CabinetParams {
            d: 26.0,
            door: DoorType::Drawers,
            ..CabinetParams::default()
        })
        .unwrap_err();
        assert!(shallow.contains("corrediça"), "{shallow}");
        let wardrobe = generate(&CabinetParams {
            w: 90.0,
            h: 230.0,
            d: 58.0,
            rod: true,
            drawers: 2,
            ..CabinetParams::default()
        })
        .unwrap();
        let rail = part(&wardrobe, "Tubo cabideiro");
        let shelf = part(&wardrobe, "Maleiro");
        // Shelf 35 cm under the top inside, rail 6 cm below it, over 2 drawers of 18 cm.
        assert!((shelf.at[2] - (230.0 - 1.8 - 35.0)).abs() < 1e-9);
        assert!((rail.at[2] - (shelf.at[2] - 6.0)).abs() < 1e-9 && rail.board.is_none());
        assert!(
            wardrobe
                .parts
                .iter()
                .all(|p| !p.name.starts_with("Prateleira"))
        );
        assert!(wardrobe.hardware.iter().any(|h| h.contains("tubo")));
        // A rod with no room to hang is built, and says what it is good for.
        let low = generate(&CabinetParams {
            h: 150.0,
            rod: true,
            drawers: 2,
            ..CabinetParams::default()
        })
        .unwrap();
        assert!(
            low.notes.iter().any(|n| n.contains("Cabideiro")),
            "{:?}",
            low.notes
        );
        // A tall cabinet with an oven niche: doors below and above, none across it.
        let tower = generate(&CabinetParams {
            w: 64.0,
            h: 220.0,
            d: 58.0,
            shelves: 2,
            niches: vec![Niche {
                bottom: 80.0,
                height: 62.0,
            }],
            ..CabinetParams::default()
        })
        .unwrap();
        let below = part(&tower, "Porta 1");
        let above = part(&tower, "Porta superior 1");
        assert!(
            (below.at[2] + below.size[2] - (80.0 - 0.2)).abs() < 1e-9,
            "{below:?}"
        );
        assert!((above.at[2] - 142.2).abs() < 1e-9, "{above:?}");
        assert!((part(&tower, "Base do nicho").at[2] - 78.2).abs() < 1e-9);
        assert!((part(&tower, "Topo do nicho").at[2] - 142.0).abs() < 1e-9);
        // Shelves in the taller part, above the niche.
        assert!(
            tower
                .parts
                .iter()
                .filter(|p| p.name.starts_with("Prateleira"))
                .all(|p| p.at[2] > 142.0)
        );
        // A niche that runs past the top is moved down into the carcass,
        // and the note keeps the number that was asked for.
        let high = generate(&CabinetParams {
            w: 60.0,
            h: 120.0,
            niches: vec![Niche {
                bottom: 80.0,
                height: 60.0,
            }],
            ..CabinetParams::default()
        })
        .unwrap();
        assert!(
            high.notes.iter().any(|n| n.contains("saiu a")),
            "{:?}",
            high.notes
        );
        assert!(high.parts.iter().any(|p| p.name == "Base do nicho"));
        let corner = generate(&CabinetParams {
            w: 100.0,
            h: 87.0,
            blind_left: 58.0,
            shelves: 1,
            ..CabinetParams::default()
        })
        .unwrap();
        let panel = part(&corner, "Painel cego esquerdo");
        let door = part(&corner, "Porta 1");
        assert!((panel.size[0] - 57.8).abs() < 1e-9);
        assert!((door.at[0] - 58.2).abs() < 1e-9 && (door.size[0] - 41.6).abs() < 1e-9);
        let narrow = generate(&CabinetParams {
            w: 80.0,
            blind_left: 58.0,
            ..CabinetParams::default()
        })
        .unwrap();
        assert!(
            narrow
                .notes
                .iter()
                .any(|n| n.contains("não serve para nada")),
            "{:?}",
            narrow.notes
        );
        // A board nobody stocks is cut to order, not refused.
        let odd = generate(&CabinetParams {
            t: 16.0,
            ..CabinetParams::default()
        })
        .unwrap();
        assert!(
            odd.notes.iter().any(|n| n.contains("15, 18 ou 25")),
            "{:?}",
            odd.notes
        );
    }

    fn handles(out: &Output) -> Vec<&Part> {
        out.parts
            .iter()
            .filter(|p| p.name.starts_with("Puxador"))
            .collect()
    }

    #[test]
    fn doors_and_drawers_get_handles_where_hands_reach() {
        // A base cabinet: one door, its bar vertical near the top on the right.
        let base = generate(&CabinetParams {
            w: 50.0,
            h: 87.0,
            d: 55.0,
            shelves: 1,
            ..CabinetParams::default()
        })
        .unwrap();
        let door = part(&base, "Porta 1");
        let [bar] = handles(&base)[..] else {
            panic!("{:?}", handles(&base))
        };
        assert!(bar.size[2] > bar.size[0] && bar.board.is_none(), "{bar:?}");
        assert!(bar.at[0] > door.at[0] + door.size[0] / 2.0, "opening side");
        let top = door.at[2] + door.size[2];
        assert!((top - (bar.at[2] + bar.size[2]) - 4.0).abs() < 1e-9);
        assert!(
            (bar.at[1] + bar.size[1] - 57.5).abs() < 1e-9,
            "2,5 cm off the front"
        );
        assert_eq!(bar.color, STEEL);
        assert!(
            base.hardware
                .iter()
                .any(|h| h == "1 puxador barra inox 128 mm"),
            "{:?}",
            base.hardware
        );
        // The overall size stays the carcass: handles stand past it.
        assert_eq!(base.size, [50.0, 55.0, 87.0]);
        // Reveals read: fronts a shade off the carcass.
        assert_ne!(door.color, part(&base, "Lateral esquerda").color);

        // Double doors meet in the middle.
        let double = generate(&CabinetParams {
            w: 80.0,
            h: 87.0,
            shelves: 1,
            ..CabinetParams::default()
        })
        .unwrap();
        let bars = handles(&double);
        assert_eq!(bars.len(), 2);
        let centers: Vec<f64> = bars.iter().map(|b| b.at[0] + b.size[0] / 2.0).collect();
        assert!((centers[0] + centers[1] - 80.0).abs() < 1e-9, "{centers:?}");
        assert!(centers[0] > 30.0 && centers[1] < 50.0, "{centers:?}");

        // A drawer stack: one horizontal bar centered near the top of each front.
        let drawers = generate(&CabinetParams {
            w: 60.0,
            h: 87.0,
            door: DoorType::Drawers,
            drawers: 3,
            ..CabinetParams::default()
        })
        .unwrap();
        let bars = handles(&drawers);
        assert_eq!(bars.len(), 3);
        for k in 1..=3 {
            let front = part(&drawers, &format!("Frente da gaveta {k}"));
            let bar = part(&drawers, &format!("Puxador da gaveta {k}"));
            assert!(bar.size[0] > bar.size[2]);
            assert!((bar.at[0] + bar.size[0] / 2.0 - 30.0).abs() < 1e-9);
            let center = bar.at[2] + bar.size[2] / 2.0;
            assert!(
                center > front.at[2] + front.size[2] / 2.0 && center < front.at[2] + front.size[2]
            );
        }

        // Wall cabinets: near the bottom; tall doors: long bar at hand height.
        let upper = generate(&CabinetParams {
            w: 40.0,
            h: 70.0,
            d: 35.0,
            plinth: 0.0,
            shelves: 1,
            ..CabinetParams::default()
        })
        .unwrap();
        let bar = handles(&upper)[0];
        assert!((bar.at[2] - (0.2 + 4.0)).abs() < 1e-9, "{bar:?}");
        let tall = generate(&CabinetParams {
            w: 50.0,
            h: 220.0,
            ..CabinetParams::default()
        })
        .unwrap();
        let bar = handles(&tall)[0];
        assert!((bar.at[2] + bar.size[2] / 2.0 - HAND_HEIGHT).abs() < 1e-9);
        assert!(bar.size[2] > 30.0 && bar.size[2] < 60.0);

        // Push-to-open: no handles, the latches in the hardware.
        let push = generate(&CabinetParams {
            w: 80.0,
            h: 87.0,
            handle: HandleStyle::None,
            ..CabinetParams::default()
        })
        .unwrap();
        assert_eq!(handles(&push), [] as [&Part; 0]);
        assert!(
            push.hardware
                .iter()
                .any(|h| h == "2 fechos toque (push-to-open)")
        );
        // Profiles run the whole top edge; a groove is no hardware.
        let profile = generate(&CabinetParams {
            w: 50.0,
            h: 87.0,
            handle: HandleStyle::Profile,
            handle_color: Some(HandleColor::Hex("#202020".into())),
            ..CabinetParams::default()
        })
        .unwrap();
        let strip = handles(&profile)[0];
        assert_eq!(
            (strip.color, strip.size[0]),
            ([32, 32, 32], part(&profile, "Porta 1").size[0])
        );
        let cava = generate(&CabinetParams {
            w: 50.0,
            h: 87.0,
            handle: HandleStyle::Cava,
            ..CabinetParams::default()
        })
        .unwrap();
        assert!(handles(&cava).is_empty() && cava.parts.iter().any(|p| p.name == "Cava Porta 1"));
        // Old saved parameters without handles get bars.
        let old: CabinetParams = serde_json::from_str(r#"{"w":60,"h":87}"#).unwrap();
        assert_eq!(old.handle, HandleStyle::Bar);
        let rgb: CabinetParams =
            serde_json::from_str(r#"{"handle":"knob","handle_color":[1,2,3]}"#).unwrap();
        assert_eq!(rgb.handle_color, Some(HandleColor::Rgb([1, 2, 3])));
    }

    #[test]
    fn handles_go_to_hardware_not_to_the_cut_list() {
        let out = generate(&CabinetParams {
            w: 60.0,
            h: 87.0,
            door: DoorType::Drawers,
            drawers: 2,
            ..CabinetParams::default()
        })
        .unwrap();
        let rows = crate::cut_list(&out);
        assert!(rows.iter().all(|r| !r.name.contains("uxador")), "{rows:?}");
        assert!(
            out.hardware
                .iter()
                .any(|h| h == "2 puxadores barra inox 128 mm"),
            "{:?}",
            out.hardware
        );
    }
}
