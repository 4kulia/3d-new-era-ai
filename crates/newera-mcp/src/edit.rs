//! Parameter types and the edit logic behind the MCP tools, kept free of
//! protocol types so it can be unit-tested directly.

use newera_core::{
    BackgroundImage, Command, CoreError, Dimension, Document, Element, ElementId, Label, Material,
    Point2, Room, Wall, ops,
};
use schemars::JsonSchema;
use serde::Deserialize;

pub(crate) type EditResult<T> = Result<T, String>;

#[allow(clippy::needless_pass_by_value)] // used as `map_err(core)`
fn core(err: CoreError) -> String {
    err.to_string()
}

/// Parses a material in short form; `none` means "no finish".
pub(crate) fn material(raw: &str) -> EditResult<Option<Material>> {
    if raw.trim() == "none" {
        return Ok(None);
    }
    raw.parse().map(Some)
}

/// Sets a piece's finish. A finish that is only a color — `#c8b89a`, no
/// pattern and no image — is the piece's color: a piece draws its finish
/// from a pattern or an image, and a bare color there would change nothing
/// on screen. A `color` given in the same call wins.
fn finish(piece: &mut newera_core::Furniture, raw: &str, color_given: bool) -> EditResult<()> {
    match material(raw)? {
        Some(m) if m.pattern.is_none() && m.image.is_none() => {
            if !color_given {
                piece.color = m.color.or(piece.color);
            }
            piece.texture = None;
        }
        other => piece.texture = other,
    }
    Ok(())
}

/// Resolves a wall type id; `none` clears it.
fn wall_type(raw: &str) -> EditResult<Option<&'static newera_core::WallType>> {
    if raw == "none" {
        return Ok(None);
    }
    newera_core::wall_type(raw)
        .map(Some)
        .ok_or_else(|| format!("unknown wall type `{raw}` (see the materials tool)"))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct WallPath {
    /// Polyline vertices; N points make N-1 joined walls.
    pub pts: Vec<Point2>,
    /// Join the last point back to the first.
    #[serde(default)]
    pub closed: bool,
    /// Thickness cm (default 15).
    pub t: Option<f64>,
    /// Height cm (default 250).
    pub h: Option<f64>,
    /// Height cm at each point, for sloping walls and gables (overrides `h`).
    pub hs: Option<Vec<f64>>,
    /// Arc extent in degrees applied to every segment (positive bulges left).
    pub arc: Option<f64>,
    /// Wall type id, e.g. `drywall-95`; sets the thickness unless `t` is given.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Finish on both sides, e.g. `#f2efe6` or `subway 20x10`.
    pub sides: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct RoomSpec {
    /// Declared room program, independent of name; auto restores inference.
    pub room_use: Option<newera_core::RoomUse>,
    /// The room's name, e.g. `Sala` or `Banheiro`.
    pub name: String,
    /// Floor polygon; or omit and give `at`.
    pub pts: Option<Vec<Point2>>,
    /// A point inside a space enclosed by walls (auto-detects the polygon).
    pub at: Option<Point2>,
    /// Floor finish, e.g. `wood` or `tiles #ffffff 60`.
    pub floor_mat: Option<String>,
    /// Ceiling finish.
    pub ceil_mat: Option<String>,
    /// Flat ceiling at storey height (default) or following wall profiles (false).
    pub ceiling_flat: Option<bool>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct DimSpec {
    /// Start of a free dimension, `[x,y]` cm.
    pub a: Option<Point2>,
    /// End of a free dimension, `[x,y]` cm.
    pub b: Option<Point2>,
    /// Measure a wall: `side` picks the line, `chain` splits at openings.
    pub wall: Option<String>,
    /// `out` (outer face, default for chains), `in` (inner face) or `axis` (default).
    pub side: Option<String>,
    /// With `wall`: one dimension per wall piece and opening along the face.
    #[serde(default)]
    pub chain: bool,
    /// Clear width and depth of a room, e.g. `r5` (two dimensions).
    pub room: Option<String>,
    /// Offset cm from the measured line (default 40 for walls; a/b: left of a→b is positive).
    pub off: Option<f64>,
    /// Also draw it in 3D.
    #[serde(default)]
    pub in3d: bool,
    /// With `in3d`: height of the line above the floor, cm.
    pub elev: Option<f64>,
    /// With `in3d`: tilt around its line, degrees (90 = offset upwards).
    pub pitch: Option<f64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct LabelSpec {
    /// What the label says.
    pub text: String,
    /// Center of the text, `[x,y]` cm.
    pub at: Point2,
    /// Text height cm (default 24).
    pub size: Option<f64>,
    /// Clockwise degrees.
    pub angle: Option<f64>,
    /// Show in 3D tilted this much: 0 lying on the floor, 90 standing.
    pub pitch: Option<f64>,
    /// Height cm in 3D.
    pub elev: Option<f64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct PolylineSpec {
    /// The line's points, `[[x,y],…]` cm.
    pub pts: Vec<Point2>,
    /// Join the last point back to the first.
    #[serde(default)]
    pub closed: bool,
    /// Line width cm (default 1).
    pub t: Option<f64>,
    /// `[r,g,b]` (default black).
    pub color: Option<[u8; 3]>,
    /// `solid`, `dot`, `dash`, `dash_dot`, `dash_dot_dot`.
    pub dash: Option<newera_core::DashStyle>,
    /// Smooth curve through the points.
    #[serde(default)]
    pub curved: bool,
    /// `[start, end]` arrows: `none`, `delta`, `open`, `disc`.
    pub arrows: Option<[newera_core::ArrowStyle; 2]>,
    /// Room divider: splits an open space into rooms without a wall (dashed).
    #[serde(default)]
    pub divider: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct CreateParams {
    /// Walls, each a chain of points joined corner to corner.
    #[serde(default)]
    pub walls: Vec<WallPath>,
    /// Rooms: an outline of points, or `at` inside closed walls.
    #[serde(default)]
    pub rooms: Vec<RoomSpec>,
    /// Dimension lines: between two points, along a wall, or across a room.
    #[serde(default)]
    pub dims: Vec<DimSpec>,
    /// Text on the plan.
    #[serde(default)]
    pub labels: Vec<LabelSpec>,
    /// Free lines: annotations, arrows, electrical or plumbing runs.
    #[serde(default)]
    pub polylines: Vec<PolylineSpec>,
    /// Pitched roofs over a rectangle, as one group of panels.
    #[serde(default)]
    pub roofs: Vec<RoofSpec>,
    /// Solids: outlines raised by `h` (slabs) or sections swept `a`→`b` (gables, ramps).
    #[serde(default)]
    pub solids: Vec<SolidSpec>,
    /// Plan version (tab) to write to; switches to it first.
    pub v: Option<usize>,
    /// Points are background-image pixels; lengths stay in cm.
    #[serde(default)]
    pub px: bool,
    /// Keep wall ends as given; no snap onto nearby walls.
    #[serde(default)]
    pub exact: bool,
}

impl CreateParams {
    /// Converts every point with `f` (pixels of the background → cm).
    pub(crate) fn map_points(&mut self, f: &dyn Fn(Point2) -> Point2) {
        let all = |pts: &mut Vec<Point2>| pts.iter_mut().for_each(|p| *p = f(*p));
        for w in &mut self.walls {
            all(&mut w.pts);
        }
        for r in &mut self.rooms {
            if let Some(pts) = &mut r.pts {
                all(pts);
            }
            r.at = r.at.map(f);
        }
        for d in &mut self.dims {
            d.a = d.a.map(f);
            d.b = d.b.map(f);
        }
        for l in &mut self.labels {
            l.at = f(l.at);
        }
        for p in &mut self.polylines {
            all(&mut p.pts);
        }
        for solid in &mut self.solids {
            if let Some(pts) = &mut solid.pts {
                all(pts);
            }
            solid.a = solid.a.map(f);
            solid.b = solid.b.map(f);
        }
        for r in &mut self.roofs {
            all(&mut r.pts);
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize, JsonSchema)]
pub(crate) struct SkylightSpec {
    /// Center on the plan, `[x,y]` cm.
    pub at: Point2,
    /// Size along the ridge, cm.
    pub w: Option<f64>,
    /// Size across the ridge, cm.
    pub d: Option<f64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct SolidSpec {
    /// Plan outline `[[x,y],…]` cm.
    pub pts: Option<Vec<Point2>>,
    /// Instead of `pts`: cross-section `[[u,z],…]` cm swept from `a` to `b`
    /// (+u is +x when a→b goes down the plan; z above the floor).
    pub profile: Option<Vec<Point2>>,
    /// With `profile`: where the sweep starts, `[x,y]` cm.
    pub a: Option<Point2>,
    /// With `profile`: where the sweep ends, `[x,y]` cm.
    pub b: Option<Point2>,
    /// Outline thickness cm (default 15).
    pub h: Option<f64>,
    /// Outline bottom above the floor cm (default 0).
    pub elev: Option<f64>,
    /// Finish (`wood`, `concrete`, `img:…`).
    pub mat: Option<String>,
    /// `[r,g,b]`.
    pub color: Option<[u8; 3]>,
    /// 0..1; e.g. 0.3 for glass.
    pub opacity: Option<f64>,
    /// The piece's name.
    pub name: Option<String>,
}

/// A piece built from a polygon.
fn solid(doc: &mut Document, spec: &SolidSpec) -> EditResult<newera_core::Furniture> {
    use newera_core::SolidShape;
    let bounds = |pts: &[Point2]| {
        pts.iter().fold(
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
        )
    };
    let mut piece = newera_core::Furniture {
        id: doc.new_furniture_id(),
        catalog: "solid".into(),
        name: spec.name.clone().unwrap_or_else(|| "Sólido".into()),
        color: spec.color,
        ..newera_core::Furniture::default()
    };
    match (&spec.pts, &spec.profile) {
        (Some(pts), None) => {
            if pts.len() < 3 || newera_core::polygon_area(pts) < 1.0 {
                return Err("a solid outline needs at least 3 points enclosing an area".into());
            }
            let (lo, hi) = bounds(pts);
            let center = Point2::new(f64::midpoint(lo.x, hi.x), f64::midpoint(lo.y, hi.y));
            piece.position = center;
            piece.width = (hi.x - lo.x).max(0.1);
            piece.depth = (hi.y - lo.y).max(0.1);
            piece.height = spec.h.unwrap_or(15.0).max(0.1);
            piece.elevation = spec.elev.unwrap_or(0.0);
            piece.shape = Some(SolidShape::Outline(
                pts.iter()
                    .map(|p| [p.x - center.x, p.y - center.y])
                    .collect(),
            ));
        }
        (None, Some(profile)) => {
            let (Some(a), Some(b)) = (spec.a, spec.b) else {
                return Err("a profile solid needs `a` and `b` (the path it runs along)".into());
            };
            if profile.len() < 3 || newera_core::polygon_area(profile) < 1.0 {
                return Err("a profile needs at least 3 points enclosing an area".into());
            }
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let length = dx.hypot(dy);
            if length < 0.5 {
                return Err("`a` and `b` must be apart".into());
            }
            let (lo, hi) = bounds(profile);
            let mid_u = f64::midpoint(lo.x, hi.x);
            // The piece's local x axis (its width) for this heading.
            let right = (dy / length, -dx / length);
            piece.angle = (-dx).atan2(dy).to_degrees();
            piece.position = Point2::new(
                f64::midpoint(a.x, b.x) + right.0 * mid_u,
                f64::midpoint(a.y, b.y) + right.1 * mid_u,
            );
            piece.width = (hi.x - lo.x).max(0.1);
            piece.depth = length;
            piece.height = (hi.y - lo.y).max(0.1);
            piece.elevation = lo.y;
            piece.shape = Some(SolidShape::Profile(
                profile.iter().map(|p| [p.x - mid_u, p.y - lo.y]).collect(),
            ));
        }
        _ => return Err("a solid needs either `pts` or `profile`".into()),
    }
    if let Some(raw) = &spec.mat {
        finish(&mut piece, raw, spec.color.is_some())?;
    }
    piece.opacity = spec.opacity.filter(|o| *o < 1.0).map(|o| o.clamp(0.0, 1.0));
    Ok(piece)
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct RoofSpec {
    /// Rectangle corners in order; the ridge runs along pts[0]→pts[1].
    pub pts: Vec<Point2>,
    /// Default `gable` (steep = A-frame); `shed` rises from the pts[0]→pts[1] side.
    #[schemars(extend("enum" = ["gable", "shed"]))]
    pub kind: Option<String>,
    /// Slope in degrees (default 30), unless `ridge_h` is given.
    pub pitch: Option<f64>,
    /// Ridge (gable) or high side (shed) height above the floor, cm.
    pub ridge_h: Option<f64>,
    /// Eave height above the storey floor, cm (default 250).
    pub h: Option<f64>,
    /// Eave and gable overhang, cm (default 30).
    pub overhang: Option<f64>,
    /// Panel thickness cm (default 12).
    pub t: Option<f64>,
    /// Panel color `[r,g,b]`.
    pub color: Option<[u8; 3]>,
    /// Also close the gable ends with sloping walls.
    #[serde(default)]
    pub gables: bool,
    /// Glazed roof openings: center `at`, `w` along the ridge, `d` across, cm.
    #[serde(default)]
    pub skylights: Vec<SkylightSpec>,
    /// The roof group's name.
    pub name: Option<String>,
}

/// Pulls the ends of `walls` onto the walls of the storey they nearly touch.
///
/// Coordinates written from a description land a few centimetres short of a
/// wall, past it, or beside a corner, and the junction then shows as a stub
/// crossing the hatch. Drawing by hand has magnetism for that; this is the
/// same thing for everything written through the tools, so a T or a corner
/// holds together without the caller having to work out the face it should
/// stop at. Ends further than [`newera_core::TOUCH_TOLERANCE`] from anything
/// are left exactly where they were.
///
/// Returns each end it moved, `w3.b→[x,y]` (`a` the start, `b` the end), so
/// the caller is told its coordinates changed.
fn weld(doc: &Document, walls: &mut [Wall]) -> Vec<String> {
    if walls.is_empty() {
        return Vec::new();
    }
    let home = doc.home();
    let mut all: Vec<Wall> = home
        .level_view(home.current_level())
        .walls
        .into_iter()
        .filter(|w| !walls.iter().any(|edited| edited.id == w.id))
        .collect();
    all.extend(walls.iter().cloned());
    let ids: Vec<newera_core::WallId> = walls.iter().map(|w| w.id).collect();
    let mut moved = Vec::new();
    for (id, at_start, to) in newera_core::weld_ends(&all, &ids) {
        if let Some(wall) = walls.iter_mut().find(|w| w.id == id) {
            if at_start {
                wall.start = to;
            } else {
                wall.end = to;
            }
            let end = if at_start { 'a' } else { 'b' };
            moved.push(format!("{id}.{end}→[{:.1},{:.1}]", to.x, to.y));
        }
    }
    moved
}

/// What [`create`] made: the new ids in order, and the wall ends it moved
/// onto walls they nearly touched (see [`weld`]).
#[derive(Debug, Default)]
pub(crate) struct Created {
    pub ids: Vec<String>,
    pub welded: Vec<String>,
}

/// Creates everything in one undoable step.
pub(crate) fn create(doc: &mut Document, params: CreateParams) -> EditResult<Created> {
    let mut commands = Vec::new();
    let mut ids = Vec::new();
    let mut new_walls = Vec::new();

    for path in params.walls {
        if path.pts.len() < 2 {
            return Err("each wall path needs at least 2 points".into());
        }
        let kind = path.kind.as_deref().map(wall_type).transpose()?.flatten();
        let sides = path.sides.as_deref().map(material).transpose()?.flatten();
        let mut pts = path.pts;
        let mut heights = path.hs.clone();
        if let Some(hs) = &heights
            && hs.len() != pts.len()
        {
            return Err(format!("`hs` needs one height per point ({})", pts.len()));
        }
        if path.closed && pts.len() > 2 {
            pts.push(pts[0]);
            if let Some(hs) = &mut heights {
                hs.push(hs[0]);
            }
        }
        for (i, pair) in pts.windows(2).enumerate() {
            let (height, height_at_end) = match &heights {
                Some(hs) => (
                    hs[i],
                    ((hs[i + 1] - hs[i]).abs() > 1e-6).then_some(hs[i + 1]),
                ),
                None => (path.h.unwrap_or(Wall::DEFAULT_HEIGHT), None),
            };
            let mut wall = Wall {
                height,
                height_at_end,
                arc_extent: path.arc.filter(|a| *a != 0.0),
                left_side: sides.clone(),
                right_side: sides.clone(),
                ..Wall::new(doc.new_wall_id(), pair[0], pair[1])
            };
            if let Some(kind) = kind {
                wall.apply_type(kind);
            }
            if let Some(t) = path.t {
                wall.thickness = t;
            }
            ids.push(wall.id.to_string());
            new_walls.push(wall);
        }
    }
    let welded = if params.exact {
        Vec::new()
    } else {
        weld(doc, &mut new_walls)
    };
    commands.extend(new_walls.iter().cloned().map(Command::insert));

    // Dividers already on this storey plus the ones created now.
    let mut dividers: Vec<newera_core::Polyline> = {
        let home = doc.home();
        home.level_view(home.current_level())
            .polylines
            .into_iter()
            .filter(|p| p.room_divider)
            .collect()
    };
    for spec in params.polylines.iter().filter(|p| p.divider) {
        let mut line = newera_core::Polyline::new(newera_core::PolylineId(0), spec.pts.clone());
        line.closed = spec.closed;
        dividers.push(line);
    }
    for spec in params.rooms {
        let (points, auto) = match (spec.pts, spec.at) {
            (Some(pts), _) => (pts, false),
            (None, Some(at)) => {
                let home = doc.home();
                let mut walls = home.level_view(home.current_level()).walls;
                walls.extend(new_walls.iter().cloned());
                let dividers: Vec<&newera_core::Polyline> = dividers.iter().collect();
                let points = newera_core::detect_room_with_dividers(&walls, &dividers, at)
                    .ok_or_else(|| {
                        format!("no space enclosed by walls around [{}, {}]", at.x, at.y)
                    })?;
                (points, true)
            }
            (None, None) => return Err(format!("room `{}` needs `pts` or `at`", spec.name)),
        };
        let mut room = Room::new(doc.new_room_id(), spec.name, points);
        room.auto = auto;
        room.usage = spec.room_use.unwrap_or_default();
        room.ceiling_flat = spec.ceiling_flat.unwrap_or(true);
        room.floor_material = spec
            .floor_mat
            .as_deref()
            .map(material)
            .transpose()?
            .flatten();
        room.ceiling_material = spec
            .ceil_mat
            .as_deref()
            .map(material)
            .transpose()?
            .flatten();
        ids.push(room.id.to_string());
        commands.push(Command::insert(room));
    }

    for spec in params.dims {
        let side = match spec.side.as_deref() {
            None | Some("axis") => ops::WallSide::Axis,
            Some("out") => ops::WallSide::Outer,
            Some("in") => ops::WallSide::Inner,
            Some(other) => return Err(format!("unknown side `{other}` (out, in, axis)")),
        };
        let dims: Vec<Dimension> = match (spec.wall, spec.room, spec.a, spec.b) {
            (Some(wall), _, _, _) => {
                let id = wall.parse().map_err(|e| format!("{e}"))?;
                let gap = spec.off.map_or(40.0, f64::abs);
                if spec.chain {
                    ops::wall_chain_dimensions(doc, id, side, gap).map_err(core)?
                } else {
                    vec![ops::wall_side_dimension(doc, id, side, gap).map_err(core)?]
                }
            }
            (None, Some(room), _, _) => {
                let id = room.parse().map_err(|e| format!("{e}"))?;
                ops::room_dimensions(doc, id).map_err(core)?
            }
            (None, None, Some(a), Some(b)) => vec![Dimension {
                id: doc.new_dimension_id(),
                start: a,
                end: b,
                offset: spec.off.unwrap_or(0.0),
                level: None,
                ..Default::default()
            }],
            _ => return Err("dimension needs `a` and `b`, `wall` or `room`".into()),
        };
        for mut dim in dims {
            if spec.in3d || spec.elev.is_some() || spec.pitch.is_some() {
                dim.visible_in_3d = true;
                dim.elevation = [spec.elev.unwrap_or(0.0); 2];
                dim.pitch = spec.pitch.unwrap_or(0.0);
            }
            ids.push(dim.id.to_string());
            commands.push(Command::insert(dim));
        }
    }

    for spec in params.labels {
        let label = Label {
            id: doc.new_label_id(),
            text: spec.text,
            position: spec.at,
            size: spec.size.unwrap_or(Label::DEFAULT_SIZE),
            angle: spec.angle.unwrap_or(0.0),
            level: None,
            pitch: spec.pitch.or(spec.elev.map(|_| 90.0)),
            elevation: spec.elev.unwrap_or(0.0),
            ..Default::default()
        };
        ids.push(label.id.to_string());
        commands.push(Command::insert(label));
    }

    for spec in &params.solids {
        let piece = solid(doc, spec)?;
        ids.push(piece.id.to_string());
        commands.push(Command::insert(piece));
    }

    for spec in params.roofs {
        let (roof, gables) = roof(doc, &spec)?;
        for wall in gables {
            ids.push(wall.id.to_string());
            commands.push(Command::insert(wall));
        }
        ids.push(roof.id.to_string());
        commands.push(Command::insert(roof));
    }

    for spec in params.polylines {
        let mut line = newera_core::Polyline::new(doc.new_polyline_id(), spec.pts);
        line.closed = spec.closed;
        line.thickness = spec.t.unwrap_or(1.0);
        line.color = spec.color.unwrap_or([0, 0, 0]);
        line.dash = spec.dash.unwrap_or_default();
        if spec.curved {
            line.join = newera_core::LineJoin::Curved;
        }
        if let Some([start, end]) = spec.arrows {
            line.start_arrow = start;
            line.end_arrow = end;
        }
        if spec.divider {
            line.room_divider = true;
            line.dash = spec.dash.unwrap_or(newera_core::DashStyle::Dash);
            line.color = spec.color.unwrap_or([120, 120, 120]);
        }
        ids.push(line.id.to_string());
        commands.push(Command::insert(line));
    }

    if commands.is_empty() {
        return Err("nothing to create".into());
    }
    doc.execute(Command::Batch { commands }).map_err(core)?;
    Ok(Created { ids, welded })
}

/// Fields to change, each for the kinds that have it; any other is
/// refused.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateSpec {
    /// Room program; auto infers it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_use: Option<newera_core::RoomUse>,
    /// The element to change.
    #[serde(skip_serializing)]
    pub id: String,
    /// Start point (wall, dimension).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<Point2>,
    /// End point (wall, dimension).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b: Option<Point2>,
    /// Wall thickness cm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
    /// Height cm (wall, furniture).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    /// Wall arc degrees (0 = straight).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arc: Option<f64>,
    /// Name (room, furniture).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Room polygon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pts: Option<Vec<Point2>>,
    /// The piece a label is about (`f12`); `""` unties it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Room floor visible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<bool>,
    /// Room ceiling visible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ceiling: Option<bool>,
    /// Room ceiling flat at storey height, false following the walls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ceiling_flat: Option<bool>,
    /// Label text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Position (label, furniture center).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Point2>,
    /// Label size cm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Clockwise degrees (label, furniture).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
    /// Dimension offset cm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub off: Option<f64>,
    /// Furniture width cm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    /// Furniture depth cm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d: Option<f64>,
    /// Elevation cm (furniture, level, 3D label/dimension).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elev: Option<f64>,
    /// Room re-detected when its walls change; `pts` turns it off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,
    /// Polyline is a room divider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divider: Option<bool>,
    /// Wall height at its end cm (sloping wall).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h_end: Option<f64>,
    /// Furniture tilt around its depth axis, degrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roll: Option<f64>,
    /// Furniture color `[r,g,b]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    /// Furniture finish (`wood`, `img:…`, `none` clears).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mat: Option<String>,
    /// Furniture opacity 0..1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// Furniture mirrored left to right.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<bool>,
    /// Face that stays put on a resize: back, front, left, right (as the
    /// piece faces), bottom, top, or +x, -x, +y, -y; default the center.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// On a group resize, the parts that take the change; the others keep
    /// their size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stretch: Option<Vec<String>>,
    /// Plan layer: lighting, appliances, joinery or none; empty restores
    /// its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Show or hide the element.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    /// Furniture light {lm|w, lamp, k, beam, area, z, on}.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light: Option<LightSpec>,
    /// Door hinge on the right.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hinge_right: Option<bool>,
    /// Guard or closure glass: laminated, tempered, tempered-laminated or
    /// wired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glass: Option<String>,
    /// Clear gap between a railing's bars, cm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<f64>,
    /// Furniture: `true` fixed joinery, `false` free-standing; `""` lets the
    /// name decide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed: Option<Fixed>,
    /// Move it to this storey, e.g. `lv2`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// Level slab thickness cm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slab: Option<f64>,
    /// Wall type id (sets thickness unless `t`); `none` clears.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Wall finish on the left of a→b; `none` clears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
    /// Wall finish on the right of a→b.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    /// Wall finish on both sides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sides: Option<String>,
    /// Room floor finish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor_mat: Option<String>,
    /// Room ceiling finish.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ceil_mat: Option<String>,
    /// Furniture brand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Role, whatever the name: trim, backsplash, counter, stair; empty:
    /// automatic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Furniture commercial model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Furniture product link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Shop product id; `""` clears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// With a product, bought unless `existing` or `excluded`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("enum" = ["buy", "existing", "excluded"]))]
    pub purchase: Option<String>,
    /// Label bold text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// Label/dimension shown in 3D.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in3d: Option<bool>,
    /// 3D tilt degrees: label 0 flat, 90 standing; dimension around its line;
    /// furniture around its width axis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    /// Label italic text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// Label alignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<newera_core::TextAlign>,
}

impl UpdateSpec {
    fn fields(&self) -> Vec<String> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map.keys().cloned().collect(),
            _ => Vec::new(),
        }
    }
}

/// Why an id does not resolve, in the terms of whoever asked.
///
/// The commonest miss is a part of a group: those ids come out of diffs and
/// of `layout`, so trying one is fair, and "not found" would send the
/// caller looking for a typo instead of at the parent it belongs to.
pub(crate) fn missing(home: &newera_core::Home, id: ElementId) -> String {
    if let ElementId::Furniture(piece) = id
        && let Some(owner) = home.part_owner(piece)
    {
        let part = owner
            .flatten()
            .into_iter()
            .find(|p| p.id == piece)
            .map_or_else(String::new, |p| format!(" ({})", p.name));
        return format!(
            "{id}{part} is a part of {}; edit {} instead — changing the group rebuilds its parts",
            owner.id, owner.id
        );
    }
    format!("{id} not found")
}

pub(crate) fn update(doc: &mut Document, items: Vec<UpdateSpec>) -> EditResult<()> {
    let mut commands = Vec::with_capacity(items.len());
    let mut reshaped: Vec<Wall> = Vec::new();
    for spec in items {
        let id: ElementId = spec.id.parse().map_err(|e| format!("{e}"))?;
        let Some(element) = doc.home().element(id) else {
            rename_part(doc.home(), id, spec, &mut commands)?;
            continue;
        };
        let allowed: &[&str] = match element {
            Element::Polyline(_) => &["pts", "t", "color", "level", "divider"],
            Element::Wall(_) => &[
                "a", "b", "t", "h", "h_end", "arc", "level", "type", "left", "right", "sides",
            ],
            Element::Room(_) => &[
                "room_use",
                "name",
                "pts",
                "floor",
                "ceiling",
                "ceiling_flat",
                "level",
                "floor_mat",
                "ceil_mat",
                "auto",
            ],
            Element::Dimension(_) => &["a", "b", "off", "level", "in3d", "elev", "pitch"],
            Element::Label(_) => &[
                "text", "at", "size", "angle", "level", "bold", "italic", "align", "color", "in3d",
                "elev", "pitch", "about",
            ],
            Element::Level(_) => &["name", "elev", "h", "slab"],
            Element::Furniture(_) => &[
                "at",
                "angle",
                "w",
                "d",
                "h",
                "elev",
                "pitch",
                "roll",
                "name",
                "color",
                "mat",
                "opacity",
                "mirror",
                "anchor",
                "stretch",
                "role",
                "layer",
                "visible",
                "hinge_right",
                "glass",
                "gap",
                "fixed",
                "level",
                "light",
                "brand",
                "model_name",
                "url",
                "product",
                "purchase",
            ],
        };
        if let Some(bad) = spec
            .fields()
            .into_iter()
            .find(|f| !allowed.contains(&f.as_str()))
        {
            return Err(format!(
                "`{bad}` does not apply to {id} (allowed: {})",
                allowed.join(", ")
            ));
        }
        let level = match &spec.level {
            Some(raw) => {
                let level_id: newera_core::LevelId = raw.parse().map_err(|e| format!("{e}"))?;
                if doc.home().level(level_id).is_none() {
                    return Err(format!("{raw} not found"));
                }
                Some(Some(level_id))
            }
            None => None,
        };
        let mut updated = match element {
            Element::Polyline(mut p) => {
                p.points = spec.pts.unwrap_or(p.points);
                p.thickness = spec.t.unwrap_or(p.thickness);
                if let Some([r, g, b]) = spec.color {
                    p.color = [r, g, b];
                }
                if let Some(divider) = spec.divider {
                    p.room_divider = divider;
                }
                Element::Polyline(p)
            }
            Element::Level(mut l) => {
                l.name = spec.name.unwrap_or(l.name);
                l.elevation = spec.elev.unwrap_or(l.elevation);
                l.height = spec.h.unwrap_or(l.height);
                l.floor_thickness = spec.slab.unwrap_or(l.floor_thickness);
                Element::Level(l)
            }
            Element::Wall(mut w) => {
                w.start = spec.a.unwrap_or(w.start);
                w.end = spec.b.unwrap_or(w.end);
                if let Some(raw) = &spec.kind {
                    match wall_type(raw)? {
                        Some(kind) => w.apply_type(kind),
                        None => w.wall_type = None,
                    }
                }
                if let Some(raw) = &spec.sides {
                    w.left_side = material(raw)?;
                    w.right_side = w.left_side.clone();
                }
                if let Some(raw) = &spec.left {
                    w.left_side = material(raw)?;
                }
                if let Some(raw) = &spec.right {
                    w.right_side = material(raw)?;
                }
                w.thickness = spec.t.unwrap_or(w.thickness);
                w.height = spec.h.unwrap_or(w.height);
                if let Some(end) = spec.h_end {
                    w.height_at_end = ((end - w.height).abs() > 1e-6).then_some(end);
                }
                if let Some(arc) = spec.arc {
                    w.arc_extent = (arc != 0.0).then_some(arc);
                }
                // An end that was moved is welded like a new one.
                if spec.a.is_some() || spec.b.is_some() {
                    reshaped.push(w.clone());
                }
                Element::Wall(w)
            }
            Element::Room(mut r) => {
                r.usage = spec.room_use.unwrap_or(r.usage);
                r.name = spec.name.unwrap_or(r.name);
                if spec.pts.is_some() {
                    r.auto = false;
                }
                if spec.auto == Some(true) {
                    let home = doc.home();
                    let view = home.level_view(r.level);
                    let dividers: Vec<&newera_core::Polyline> =
                        view.polylines.iter().filter(|l| l.room_divider).collect();
                    let inside = newera_core::interior_point(&r.points)
                        .ok_or("room outline has too few points")?;
                    r.points =
                        newera_core::detect_room_with_dividers(&view.walls, &dividers, inside)
                            .ok_or("no space enclosed by walls around this room")?;
                }
                r.auto = spec.auto.unwrap_or(r.auto);
                r.points = spec.pts.unwrap_or(r.points);
                r.floor_visible = spec.floor.unwrap_or(r.floor_visible);
                r.ceiling_visible = spec.ceiling.unwrap_or(r.ceiling_visible);
                r.ceiling_flat = spec.ceiling_flat.unwrap_or(r.ceiling_flat);
                if let Some(raw) = &spec.floor_mat {
                    r.floor_material = material(raw)?;
                }
                if let Some(raw) = &spec.ceil_mat {
                    r.ceiling_material = material(raw)?;
                }
                Element::Room(r)
            }
            Element::Dimension(mut d) => {
                d.start = spec.a.unwrap_or(d.start);
                d.end = spec.b.unwrap_or(d.end);
                d.offset = spec.off.unwrap_or(d.offset);
                d.visible_in_3d = spec
                    .in3d
                    .unwrap_or(d.visible_in_3d || spec.elev.is_some() || spec.pitch.is_some());
                if let Some(elev) = spec.elev {
                    d.elevation = [elev; 2];
                }
                d.pitch = spec.pitch.unwrap_or(d.pitch);
                Element::Dimension(d)
            }
            Element::Label(mut l) => {
                l.text = spec.text.unwrap_or(l.text);
                l.position = spec.at.unwrap_or(l.position);
                l.size = spec.size.unwrap_or(l.size);
                l.angle = spec.angle.unwrap_or(l.angle);
                l.bold = spec.bold.unwrap_or(l.bold);
                l.italic = spec.italic.unwrap_or(l.italic);
                l.align = spec.align.unwrap_or(l.align);
                l.color = spec.color.or(l.color);
                l.elevation = spec.elev.unwrap_or(l.elevation);
                if let Some(raw) = &spec.about {
                    l.about = match raw.trim() {
                        "" => None,
                        id => Some(id.parse().map_err(|e| format!("about: {e}"))?),
                    };
                }
                l.pitch = match (spec.in3d, spec.pitch) {
                    (Some(false), _) => None,
                    (_, Some(pitch)) => Some(pitch),
                    (Some(true), None) => l.pitch.or(Some(90.0)),
                    (None, None) => l.pitch.or(spec.elev.map(|_| 90.0)),
                };
                Element::Label(l)
            }
            Element::Furniture(mut f) => {
                let before = f.clone();
                let was = (f.width, f.depth, f.height, f.elevation);
                f.position = spec.at.unwrap_or(f.position);
                f.angle = spec.angle.unwrap_or(f.angle);
                f.width = spec.w.unwrap_or(f.width);
                f.depth = spec.d.unwrap_or(f.depth);
                f.height = spec.h.unwrap_or(f.height);
                f.elevation = spec.elev.unwrap_or(f.elevation);
                if let Some(anchor) = &spec.anchor {
                    hold_face(&mut f, anchor, was)?;
                }
                if let Some(parts) = &spec.stretch {
                    if !f.is_group() {
                        return Err(format!("stretch applies to a group; {id} has no parts"));
                    }
                    let parts: Vec<newera_core::FurnitureId> = parts
                        .iter()
                        .map(|raw| raw.parse().map_err(|e| format!("stretch: {e}")))
                        .collect::<Result<_, _>>()?;
                    f.follow_group_stretch(&before, &parts)?;
                }
                f.pitch = spec.pitch.unwrap_or(f.pitch);
                f.roll = spec.roll.unwrap_or(f.roll);
                f.name = spec.name.unwrap_or(f.name);
                f.color = spec.color.or(f.color);
                if let Some(raw) = &spec.mat {
                    finish(&mut f, raw, spec.color.is_some())?;
                }
                if let Some(o) = spec.opacity {
                    f.opacity = (o < 1.0).then_some(o.clamp(0.0, 1.0));
                }
                f.mirrored = spec.mirror.unwrap_or(f.mirrored);
                f.visible = spec.visible.unwrap_or(f.visible);
                if let Some(light) = &spec.light {
                    apply_light(&mut f, light);
                }
                let text = |v: Option<String>, old: Option<String>| match v {
                    Some(v) if v.is_empty() => None,
                    Some(v) => Some(v),
                    None => old,
                };
                if let Some(role) = spec.role {
                    match role.as_str() {
                        "" => {
                            f.properties.remove(newera_core::Furniture::ROLE_KEY);
                        }
                        "trim" | "backsplash" | "counter" | "stair" => {
                            f.properties
                                .insert(newera_core::Furniture::ROLE_KEY.into(), role);
                        }
                        _ => {
                            return Err(
                                "role must be trim, backsplash, counter, stair or empty".into()
                            );
                        }
                    }
                }
                f.info.brand = text(spec.brand, f.info.brand.take());
                f.info.model_name = text(spec.model_name, f.info.model_name.take());
                f.info.url = text(spec.url, f.info.url.take());
                f.info.product = text(spec.product, f.info.product.take());
                if let Some(raw) = &spec.purchase {
                    f.info.purchase = Some(purchase(raw)?);
                }
                if let Some(layer) = &spec.layer {
                    set_layer(&mut f, layer)?;
                }
                if let (Some(right), Some(opening)) = (spec.hinge_right, f.opening.as_mut()) {
                    opening.hinge_right = right;
                }
                if let Some(glass) = &spec.glass {
                    let glass = glass.trim().to_lowercase();
                    if !["laminated", "tempered", "tempered-laminated", "wired"]
                        .contains(&glass.as_str())
                    {
                        return Err(format!(
                            "glass: laminated, tempered, tempered-laminated or wired (not {glass})"
                        ));
                    }
                    f.properties
                        .insert(newera_core::guard::GLASS_KEY.into(), glass);
                }
                if let Some(gap) = spec.gap {
                    f.properties
                        .insert(newera_core::guard::GAP_KEY.into(), gap.to_string());
                }
                match &spec.fixed {
                    Some(Fixed::Declared(fixed)) => {
                        f.properties
                            .insert(newera_core::mounting::FIXED_KEY.into(), fixed.to_string());
                    }
                    Some(Fixed::Cleared(raw)) if raw.trim().is_empty() => {
                        f.properties.remove(newera_core::mounting::FIXED_KEY);
                    }
                    Some(Fixed::Cleared(raw)) => {
                        return Err(format!("fixed: true, false, or \"\" to clear (not {raw})"));
                    }
                    None => {}
                }
                Element::Furniture(f)
            }
        };
        if let Some(level) = level {
            updated.set_level(level);
        }
        commands.push(Command::Update { element: updated });
    }
    let _ = weld(doc, &mut reshaped);
    for wall in reshaped {
        if let Some(command) = commands
            .iter_mut()
            .find(|c| matches!(c, Command::Update { element: Element::Wall(w) } if w.id == wall.id))
        {
            *command = Command::Update {
                element: Element::Wall(wall),
            };
        }
    }
    doc.execute(Command::Batch { commands }).map_err(core)
}

/// A rename by rule: `pattern` replaced by `to` wherever it matches.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct RenameSpec {
    /// Regular expression, e.g. `^\\d+ — ` for a numbered prefix.
    pub pattern: String,
    /// Replacement; `$1` refers to a group, empty removes the match.
    #[serde(default)]
    pub to: String,
    /// Default `names`, parts of groups included.
    #[schemars(extend("enum" = ["names", "labels"]))]
    pub what: Option<String>,
}

/// Renames every piece (parts included) or label matching a rule, as one
/// step — the 59 names of an import that all began with an old index number
/// were one rule, and without it each passed whole through an agent's
/// context.
pub(crate) fn rename(doc: &mut Document, spec: &RenameSpec) -> EditResult<()> {
    let re = regex_lite::Regex::new(&spec.pattern).map_err(|e| format!("pattern: {e}"))?;
    let mut commands = Vec::new();
    match spec.what.as_deref().unwrap_or("names") {
        "names" => {
            fn walk(piece: &mut newera_core::Furniture, re: &regex_lite::Regex, to: &str) -> bool {
                let renamed = re.replace_all(&piece.name, to).trim().to_owned();
                let mut changed = renamed != piece.name;
                if changed {
                    piece.name = renamed;
                }
                for child in &mut piece.children {
                    changed |= walk(child, re, to);
                }
                changed
            }
            for top in &doc.home().furniture {
                let mut copy = top.clone();
                if walk(&mut copy, &re, &spec.to) {
                    commands.push(Command::update(copy));
                }
            }
        }
        "labels" => {
            for label in &doc.home().labels {
                let renamed = re
                    .replace_all(&label.text, spec.to.as_str())
                    .trim()
                    .to_owned();
                if renamed != label.text {
                    let mut copy = label.clone();
                    copy.text = renamed;
                    commands.push(Command::update(copy));
                }
            }
        }
        other => return Err(format!("rename what: names or labels (not {other})")),
    }
    if commands.is_empty() {
        return Err(format!("rename: nothing matches `{}`", spec.pattern));
    }
    doc.execute(Command::Batch { commands }).map_err(core)
}

/// Fields a part of a group takes on its own: what it is called and what it
/// is, never where it is or how big — the group owns that and rebuilds it.
const PART_FIELDS: [&str; 8] = [
    "id",
    "name",
    "brand",
    "model_name",
    "url",
    "product",
    "purchase",
    "layer",
];

/// What the takeoff does with a piece, by its name.
fn purchase(raw: &str) -> EditResult<newera_core::Purchase> {
    match raw {
        "buy" => Ok(newera_core::Purchase::Buy),
        "existing" => Ok(newera_core::Purchase::Existing),
        "excluded" => Ok(newera_core::Purchase::Excluded),
        other => Err(format!("purchase: buy, existing or excluded (not {other})")),
    }
}

/// What an item set into joinery — a sink bowl, a cooktop, an oven — takes
/// besides: its own finish. It is not rebuilt with its host but carried
/// whole, so a finish given to it stays.
const EMBEDDED_FIELDS: [&str; 3] = ["color", "mat", "opacity"];

/// A piece declared fixed or free-standing, or `""` to clear it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum Fixed {
    Declared(bool),
    Cleared(String),
}

// By hand, so a wrong value says what the right ones are instead of
// "did not match any variant".
impl<'de> Deserialize<'de> for Fixed {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Bool(b) => Ok(Self::Declared(b)),
            serde_json::Value::String(s) => Ok(Self::Cleared(s)),
            other => Err(serde::de::Error::custom(format!(
                "fixed: true, false, or \"\" to let the name decide (not {other})"
            ))),
        }
    }
}

/// Puts a piece in a plan layer by hand, or back in the one it is in by itself.
fn set_layer(piece: &mut newera_core::Furniture, raw: &str) -> EditResult<()> {
    match raw.trim() {
        "" => {
            piece.properties.remove(newera_core::LAYER_KEY);
        }
        chosen @ ("none" | "lighting" | "appliances" | "joinery") => {
            piece
                .properties
                .insert(newera_core::LAYER_KEY.to_owned(), chosen.to_owned());
        }
        other => {
            return Err(format!(
                "layer: lighting, appliances, joinery, none, or empty (not {other})"
            ));
        }
    }
    Ok(())
}

/// Renames a part of a group, or says why it cannot be edited alone.
///
/// A part's name is where a joiner writes its size ("tampo aberto 110 × 30"),
/// and after the group is resized it is the part's name that is wrong — the
/// one thing `stale` rightly reports and nobody could fix.
fn rename_part(
    home: &newera_core::Home,
    id: ElementId,
    spec: UpdateSpec,
    commands: &mut Vec<Command>,
) -> EditResult<()> {
    fn find(
        piece: &mut newera_core::Furniture,
        id: newera_core::FurnitureId,
    ) -> Option<&mut newera_core::Furniture> {
        if piece.id == id {
            return Some(piece);
        }
        piece.children.iter_mut().find_map(|c| find(c, id))
    }
    let (ElementId::Furniture(part), Some(owner)) = (
        id,
        match id {
            ElementId::Furniture(part) => home.part_owner(part),
            _ => None,
        },
    ) else {
        return Err(missing(home, id));
    };
    let embedded = owner
        .flatten()
        .into_iter()
        .any(|p| p.id == part && p.properties.contains_key(newera_joinery::EMBED_KEY));
    let takes: Vec<&str> = PART_FIELDS
        .iter()
        .chain(if embedded { &EMBEDDED_FIELDS[..] } else { &[] })
        .copied()
        .collect();
    if let Some(bad) = spec
        .fields()
        .into_iter()
        .find(|f| !takes.contains(&f.as_str()))
    {
        return Err(format!(
            "{}; {} takes only {} on its own (`{bad}` belongs to the group)",
            missing(home, id),
            if embedded {
                "an item set into it"
            } else {
                "a part"
            },
            takes[1..].join(", ")
        ));
    }
    // Two parts of one group in one call edit the same copy of the group.
    let at = commands.iter().position(
        |c| matches!(c, Command::Update { element: Element::Furniture(f) } if f.id == owner.id),
    );
    let mut group = match at {
        Some(k) => match commands.remove(k) {
            Command::Update {
                element: Element::Furniture(f),
            } => f,
            _ => unreachable!("matched above"),
        },
        None => owner.clone(),
    };
    let piece = find(&mut group, part).ok_or_else(|| missing(home, id))?;
    let text = |v: Option<String>, old: Option<String>| match v {
        Some(v) if v.is_empty() => None,
        Some(v) => Some(v),
        None => old,
    };
    piece.name = spec.name.unwrap_or_else(|| piece.name.clone());
    piece.info.brand = text(spec.brand, piece.info.brand.take());
    piece.info.model_name = text(spec.model_name, piece.info.model_name.take());
    piece.info.url = text(spec.url, piece.info.url.take());
    piece.info.product = text(spec.product, piece.info.product.take());
    if let Some(raw) = &spec.purchase {
        piece.info.purchase = Some(purchase(raw)?);
    }
    if let Some(layer) = &spec.layer {
        set_layer(piece, layer)?;
    }
    piece.color = spec.color.or(piece.color);
    if let Some(raw) = &spec.mat {
        piece.texture = material(raw)?;
    }
    if let Some(o) = spec.opacity {
        piece.opacity = (o < 1.0).then_some(o.clamp(0.0, 1.0));
    }
    commands.push(Command::Update {
        element: Element::Furniture(group),
    });
    Ok(())
}

/// Moves a resized piece so that one of its faces stays where it was.
///
/// `update(d=…)` alone keeps the center, so every change of depth means
/// recalculating `at` — the commonest source of a run of cabinets drifting
/// off the wall it was aligned to.
pub(crate) fn hold_face(
    piece: &mut newera_core::Furniture,
    anchor: &str,
    was: (f64, f64, f64, f64),
) -> EditResult<()> {
    let (old_w, old_d, old_h, old_elev) = was;
    let raw = anchor.trim().to_ascii_lowercase();
    // Height is its own axis: `elev` already holds the bottom, so only the
    // top needs the elevation moved by what the piece grew.
    if let "bottom" | "top" = raw.as_str() {
        if raw == "top" {
            piece.elevation = old_elev + old_h - piece.height;
        }
        return Ok(());
    }

    // The held face, in the piece's own frame: along its width or its depth,
    // toward the positive local direction or the negative one.
    let front = newera_core::facing(piece);
    let mut side = turn_left(front);
    if piece.mirrored {
        side = opposite(side);
    }
    let (along_width, sign) = match raw.as_str() {
        "front" => (false, 1.0),
        "back" => (false, -1.0),
        "right" => (true, 1.0),
        "left" => (true, -1.0),
        d @ ("+x" | "-x" | "+y" | "-y") => {
            if d == front {
                (false, 1.0)
            } else if d == opposite(front) {
                (false, -1.0)
            } else if d == side {
                (true, 1.0)
            } else {
                (true, -1.0)
            }
        }
        _ => {
            return Err(format!(
                "anchor `{anchor}`: back, front, left, right, bottom, top, +x, -x, +y or -y"
            ));
        }
    };
    let grew = if along_width {
        piece.width - old_w
    } else {
        piece.depth - old_d
    };
    // Holding a face moves the center away from it by half the growth.
    let shift = -sign * grew / 2.0;
    let local = if along_width {
        (shift, 0.0)
    } else {
        (0.0, shift)
    };
    piece.position = piece.to_plan(local);
    Ok(())
}

/// The plan direction opposite this one.
fn opposite(dir: &str) -> &'static str {
    match dir {
        "+x" => "-x",
        "-x" => "+x",
        "+y" => "-y",
        _ => "+y",
    }
}

/// A quarter turn counterclockwise in plan (x right, y down): given where a
/// piece's front looks, this is where its local `+x` points.
fn turn_left(dir: &str) -> &'static str {
    match dir {
        "+x" => "-y",
        "-y" => "-x",
        "-x" => "+y",
        _ => "+x",
    }
}

pub(crate) fn parse_ids(raw: &[String]) -> EditResult<Vec<ElementId>> {
    raw.iter()
        .map(|s| s.parse::<ElementId>().map_err(|e| format!("{e}")))
        .collect()
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct Calibration {
    /// First image pixel.
    pub a: Point2,
    /// Second image pixel.
    pub b: Point2,
    /// Real distance between them, cm.
    pub cm: f64,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct BackgroundParams {
    /// Image file (png/jpg/webp/bmp). Required when there is no background yet.
    pub path: Option<String>,
    /// Scale directly, cm per image pixel.
    pub cm_per_px: Option<f64>,
    /// Or scale by marking two pixels with a known real distance.
    pub calibrate: Option<Calibration>,
    /// Several known distances (e.g. one across, one down): fits separate
    /// horizontal and vertical scales when they differ, uniform otherwise.
    #[serde(default)]
    pub calibrations: Vec<Calibration>,
    /// Vertical scale cm/px when it differs from the horizontal one.
    pub cm_per_px_y: Option<f64>,
    /// Clockwise turn of the image, degrees.
    pub angle: Option<f64>,
    /// Plan position cm of the image's top-left corner.
    pub offset: Option<Point2>,
    /// How opaque the image is drawn, 0..1.
    pub opacity: Option<f64>,
    /// Show or hide the image without removing it.
    pub visible: Option<bool>,
    /// Remove the background.
    #[serde(default)]
    pub clear: bool,
}

/// Applies background settings. `image_size` reads an image's pixel size.
pub(crate) fn set_background(
    doc: &mut Document,
    params: &BackgroundParams,
    image_size: &dyn Fn(&str) -> EditResult<[u32; 2]>,
) -> EditResult<()> {
    if params.clear {
        return doc
            .execute(Command::SetBackground { background: None })
            .map_err(core);
    }
    let current = doc.home().background.clone();
    let path = match (&params.path, &current) {
        (Some(p), _) => p.clone(),
        (None, Some(bg)) => bg.path.clone(),
        (None, None) => return Err("`path` is required to add a background".into()),
    };
    let size_px = match (&params.path, &current) {
        (None, Some(bg)) => bg.size_px,
        _ => image_size(&path)?,
    };
    let mut pairs: Vec<(Point2, Point2, f64)> = params
        .calibrations
        .iter()
        .map(|c| (c.a, c.b, c.cm))
        .collect();
    if let Some(c) = &params.calibrate {
        pairs.push((c.a, c.b, c.cm));
    }
    let (cm_per_px, cm_per_px_y) = if pairs.is_empty() {
        let x = params
            .cm_per_px
            .unwrap_or_else(|| current.as_ref().map_or(1.0, |bg| bg.cm_per_px));
        let y = params.cm_per_px_y.or_else(|| {
            if params.cm_per_px.is_some() {
                None
            } else {
                current.as_ref().and_then(|bg| bg.cm_per_px_y)
            }
        });
        (x, y)
    } else {
        let (x, y) = BackgroundImage::fit_scale(&pairs)
            .ok_or("calibration points must differ and distances be positive")?;
        (x, ((y - x).abs() > x * 1e-3).then_some(y))
    };
    let background = BackgroundImage {
        cm_per_px_y,
        angle: params
            .angle
            .or(current.as_ref().map(|bg| bg.angle))
            .unwrap_or(0.0),
        path,
        size_px,
        cm_per_px,
        offset: params
            .offset
            .or(current.as_ref().map(|bg| bg.offset))
            .unwrap_or_default(),
        opacity: params
            .opacity
            .or(current.as_ref().map(|bg| bg.opacity))
            .unwrap_or(0.5),
        visible: params
            .visible
            .or(current.as_ref().map(|bg| bg.visible))
            .unwrap_or(true),
    };
    doc.execute(Command::SetBackground {
        background: Some(background),
    })
    .map_err(core)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(doc: &mut Document) -> Vec<String> {
        create(
            doc,
            CreateParams {
                walls: vec![WallPath {
                    pts: [(0.0, 0.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)]
                        .iter()
                        .map(|&(x, y)| Point2::new(x, y))
                        .collect(),
                    closed: true,
                    ..WallPath::default()
                }],
                ..CreateParams::default()
            },
        )
        .unwrap()
        .ids
    }

    #[test]
    fn create_walls_and_detected_room_in_one_call() {
        let mut doc = Document::default();
        let ids = create(
            &mut doc,
            CreateParams {
                walls: vec![WallPath {
                    pts: vec![
                        Point2::new(0.0, 0.0),
                        Point2::new(400.0, 0.0),
                        Point2::new(400.0, 300.0),
                        Point2::new(0.0, 300.0),
                    ],
                    closed: true,
                    ..WallPath::default()
                }],
                rooms: vec![RoomSpec {
                    name: "Sala".into(),
                    at: Some(Point2::new(200.0, 150.0)),
                    pts: None,
                    ..RoomSpec::default()
                }],
                labels: vec![LabelSpec {
                    text: "Entrada".into(),
                    at: Point2::new(0.0, -50.0),
                    ..LabelSpec::default()
                }],
                ..CreateParams::default()
            },
        )
        .unwrap()
        .ids;
        assert_eq!(ids, ["w1", "w2", "w3", "w4", "r5", "t6"]);
        assert_eq!(doc.revision(), 1, "single undoable step");
        let room = &doc.home().rooms[0];
        assert!((room.area() - 385.0 * 285.0).abs() < 0.5, "{}", room.area());
    }

    #[test]
    fn failed_detection_creates_nothing() {
        let mut doc = Document::default();
        let err = create(
            &mut doc,
            CreateParams {
                walls: vec![WallPath {
                    pts: vec![Point2::new(0.0, 0.0), Point2::new(400.0, 0.0)],
                    ..WallPath::default()
                }],
                rooms: vec![RoomSpec {
                    name: "X".into(),
                    at: Some(Point2::new(10.0, 10.0)),
                    pts: None,
                    ..RoomSpec::default()
                }],
                ..CreateParams::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("no space enclosed"));
        assert_eq!(doc.home().walls, [] as [newera_core::Wall; 0]);
    }

    #[test]
    fn measured_walls_stay_exact_and_welds_are_reported() {
        let make = |exact: bool| {
            let mut doc = Document::default();
            let wall = |pts: [[f64; 2]; 2]| WallPath {
                pts: pts.iter().map(|p| Point2::new(p[0], p[1])).collect(),
                ..WallPath::default()
            };
            create(
                &mut doc,
                CreateParams {
                    walls: vec![wall([[0.0, 0.0], [400.0, 0.0]])],
                    ..CreateParams::default()
                },
            )
            .unwrap();
            // A stub that stops 3 cm short of the wall's face.
            let created = create(
                &mut doc,
                CreateParams {
                    walls: vec![wall([[200.0, 200.0], [200.0, 10.5]])],
                    exact,
                    ..CreateParams::default()
                },
            )
            .unwrap();
            (doc.home().walls[1].end, created.welded)
        };

        let (end, welded) = make(false);
        assert!(end.y < 10.5, "a near end is pulled onto the wall");
        assert_eq!(welded.len(), 1, "and the reply says so: {welded:?}");
        assert!(welded[0].starts_with("w2.b→"), "{welded:?}");

        let (end, welded) = make(true);
        assert_eq!(end, Point2::new(200.0, 10.5), "exact keeps the point given");
        assert_eq!(welded, [] as [std::string::String; 0]);
    }

    #[test]
    fn wall_types_and_finishes() {
        let mut doc = Document::default();
        let params: CreateParams = serde_json::from_str(
            r##"{"walls":[{"pts":[[0,0],[400,0],[400,300],[0,300]],"closed":true,"type":"drywall-95","sides":"#f2efe6"}],
                "rooms":[{"name":"Sala","at":[200,150],"floor_mat":"wood"}]}"##,
        )
        .unwrap();
        create(&mut doc, params).unwrap();
        let wall = &doc.home().walls[0];
        assert_eq!(
            (wall.wall_type.as_deref(), wall.thickness),
            (Some("drywall-95"), 9.5)
        );
        assert_eq!(wall.left_side.as_ref().unwrap().to_string(), "#f2efe6");
        assert_eq!(
            doc.home().rooms[0].floor_material,
            Some(Material::pattern(newera_core::Pattern::Wood))
        );

        let spec: UpdateSpec =
            serde_json::from_str(r#"{"id":"w1","type":"tijolo-14","right":"brick","left":"none"}"#)
                .unwrap();
        update(&mut doc, vec![spec]).unwrap();
        let wall = &doc.home().walls[0];
        assert!((wall.thickness - 19.0).abs() < 1e-9);
        assert!(wall.left_side.is_none());
        assert_eq!(wall.right_side.as_ref().unwrap().to_string(), "brick");

        let bad: UpdateSpec = serde_json::from_str(r#"{"id":"w1","type":"papelao"}"#).unwrap();
        assert!(
            update(&mut doc, vec![bad])
                .unwrap_err()
                .contains("unknown wall type")
        );
        let bad: UpdateSpec = serde_json::from_str(r#"{"id":"r5","floor_mat":"lava"}"#).unwrap();
        assert!(update(&mut doc, vec![bad]).is_err());
    }

    #[test]
    fn explicit_furniture_roles_are_readable_reversible_and_validated() {
        let mut doc = Document::default();
        let f = newera_core::Furniture {
            id: newera_core::FurnitureId(1),
            catalog: "box".into(),
            ..newera_core::Furniture::default()
        };
        doc.execute(Command::insert(f)).unwrap();
        let set = |doc: &mut Document, role: &str| {
            update(
                doc,
                vec![serde_json::from_value(serde_json::json!({"id":"f1","role":role})).unwrap()],
            )
        };
        set(&mut doc, "backsplash").unwrap();
        let f = &doc.home().furniture[0];
        assert_eq!(
            crate::compact::piece(doc.home(), &[], f)["role"],
            "backsplash"
        );
        set(&mut doc, "invalid").unwrap_err();
        assert_eq!(
            doc.home().furniture[0].properties[newera_core::Furniture::ROLE_KEY],
            "backsplash"
        );
        set(&mut doc, "").unwrap();
        assert!(
            !doc.home().furniture[0]
                .properties
                .contains_key(newera_core::Furniture::ROLE_KEY)
        );
        doc.undo().unwrap();
        assert_eq!(
            doc.home().furniture[0].properties[newera_core::Furniture::ROLE_KEY],
            "backsplash"
        );
        set(&mut doc, "stair").unwrap();
        assert!(
            doc.home().furniture[0].is_stairs(),
            "a flight built by hand"
        );
    }

    #[test]
    fn update_rejects_fields_of_other_kinds() {
        let mut doc = Document::default();
        square(&mut doc);
        update(
            &mut doc,
            vec![UpdateSpec {
                id: "w1".into(),
                t: Some(25.0),
                arc: Some(30.0),
                ..UpdateSpec::default()
            }],
        )
        .unwrap();
        let wall = doc.home().walls[0].clone();
        assert_eq!((wall.thickness, wall.arc_extent), (25.0, Some(30.0)));
        let err = update(
            &mut doc,
            vec![UpdateSpec {
                id: "w1".into(),
                text: Some("x".into()),
                ..UpdateSpec::default()
            }],
        )
        .unwrap_err();
        assert!(err.contains("does not apply"), "{err}");
    }

    #[test]
    fn update_refuses_a_field_that_does_not_exist() {
        // `props` was dropped in silence and the reply said nothing changed.
        let err = serde_json::from_str::<UpdateSpec>(r#"{"id":"f1","props":{"elec:modules":24}}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field `props`"), "{err}");
    }

    #[test]
    fn wall_dimension_via_create() {
        let mut doc = Document::default();
        square(&mut doc);
        let ids = create(
            &mut doc,
            CreateParams {
                dims: vec![DimSpec {
                    wall: Some("w1".into()),
                    ..DimSpec::default()
                }],
                ..CreateParams::default()
            },
        )
        .unwrap()
        .ids;
        assert_eq!(ids, ["d5"]);
        assert!((doc.home().dimensions[0].length() - 400.0).abs() < 1e-9);
    }

    #[test]
    fn background_calibration_keeps_previous_values() {
        let mut doc = Document::default();
        let size = |_: &str| Ok([1000, 800]);
        set_background(
            &mut doc,
            &BackgroundParams {
                path: Some("planta.png".into()),
                calibrate: Some(Calibration {
                    a: Point2::new(0.0, 0.0),
                    b: Point2::new(200.0, 0.0),
                    cm: 500.0,
                }),
                ..BackgroundParams::default()
            },
            &size,
        )
        .unwrap();
        set_background(
            &mut doc,
            &BackgroundParams {
                opacity: Some(0.8),
                ..BackgroundParams::default()
            },
            &size,
        )
        .unwrap();
        let bg = doc.home().background.clone().unwrap();
        assert_eq!(
            (bg.cm_per_px, bg.opacity, bg.size_px),
            (2.5, 0.8, [1000, 800])
        );
    }
}

/// Where a placed piece's front looks.
#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum Facing {
    /// A point `[x, y]` to turn toward.
    At(Point2),
    /// A side (`+x`, `-x`, `+y`, `-y`) or the id of an element to turn toward.
    Toward(String),
}

impl<'de> Deserialize<'de> for Facing {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wrong = |v: &serde_json::Value| {
            serde::de::Error::custom(format!(
                "facing: a side (+x -x +y -y), [x,y] or an element id (not {v})"
            ))
        };
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) => Ok(Self::Toward(s)),
            v @ serde_json::Value::Array(_) => serde_json::from_value(v.clone())
                .map(Self::At)
                .map_err(|_| wrong(&v)),
            v => Err(wrong(&v)),
        }
    }
}

impl Facing {
    /// The clockwise angle that turns a piece at `from` toward this.
    fn angle(&self, doc: &Document, from: Point2) -> EditResult<f64> {
        let target = match self {
            Self::At(p) => *p,
            Self::Toward(raw) => {
                if let Some(dir) = newera_core::front::side_vector(raw) {
                    return Ok(newera_core::front::angle_facing(dir));
                }
                let id: newera_core::ElementId = raw.trim().parse().map_err(|_| {
                    format!("facing `{raw}`: a side (+x -x +y -y), [x,y] or an element id")
                })?;
                let (min, max) = newera_core::measure::element_bounds(doc.home(), id)
                    .ok_or_else(|| format!("facing: {raw} not found"))?;
                Point2::new(min.x.midpoint(max.x), min.y.midpoint(max.y))
            }
        };
        let dir = Point2::new(target.x - from.x, target.y - from.y);
        if dir.x.hypot(dir.y) < 1e-6 {
            return Err("facing: the point is where the piece is".into());
        }
        Ok(newera_core::front::angle_facing(dir))
    }
}

#[derive(Debug, Default, Clone, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct PlaceSpec {
    /// Catalog id, e.g. `bed-double` (see the `catalog` tool).
    #[serde(default)]
    pub cat: String,
    /// Instead of `cat`: path of an .obj/.gltf/.glb model to import.
    pub model: Option<String>,
    /// With `model`: unit of its numbers, `m`, `cm`, `mm` or `in` (default: guessed from its size).
    pub unit: Option<String>,
    /// With `model`: w/d/h may stretch it unevenly; otherwise one or two of them scale it evenly.
    pub stretch: Option<bool>,
    /// Instead of `cat`: id of a piece already in the project to copy —
    /// its model (even one embedded from an old import), finish, size and
    /// parts — to `at` or `wall`, with any size, angle or name given here
    /// on top.
    pub copy: Option<String>,
    /// Center on the plan. Optional when `wall` is given; a door or window
    /// placed `at` a point near a wall snaps into it.
    pub at: Option<Point2>,
    /// Put it in/against this wall (doors and windows cut it).
    pub wall: Option<String>,
    /// Distance along the wall from its start, cm (default: middle).
    pub along: Option<f64>,
    /// Clockwise degrees: 0 turns the front to +y (down the plan), 90 to
    /// −x, 180 to −y, 270 to +x. `facing` says the same without the sums.
    pub angle: Option<f64>,
    /// Which way its front looks, instead of `angle`: a side (`+x`, `-x`,
    /// `+y`, `-y`), or a point `[x,y]` or an element id to turn toward (an
    /// armchair toward the TV). With `wall` it also picks the wall's side.
    pub facing: Option<Facing>,
    /// Width override, cm.
    pub w: Option<f64>,
    /// Depth override, cm.
    pub d: Option<f64>,
    /// Height override, cm.
    pub h: Option<f64>,
    /// Bottom above the floor, cm.
    pub elev: Option<f64>,
    /// Tilt around the width axis, degrees (rafters, ramps, panels).
    pub pitch: Option<f64>,
    /// Tilt around the depth axis, degrees.
    pub roll: Option<f64>,
    /// The piece's name (default: the catalog's).
    pub name: Option<String>,
    /// `[r,g,b]`.
    pub color: Option<[u8; 3]>,
    /// Finish over the whole piece: `wood`, `marble #222 60`, `img:photo.jpg 100x80`.
    pub mat: Option<String>,
    /// 0..1; e.g. 0.3 for glass.
    pub opacity: Option<f64>,
    /// Mirror it left to right.
    pub mirror: Option<bool>,
    /// Doors: hinge on the right.
    pub hinge_right: Option<bool>,
    /// Glass of a guard or a balcony closure: `laminated`, `tempered`,
    /// `tempered-laminated` or `wired`.
    pub glass: Option<String>,
    /// Clear gap between a railing's bars, cm.
    pub gap: Option<f64>,
    /// Doors: a point `[x,y]` on the side the leaf swings into (e.g. inside the bathroom).
    pub into: Option<Point2>,
    /// With `cat:"beam"`: end points `[x,y,z]` cm (z above the storey floor);
    /// `w`×`h` is the section (default 10×20).
    pub a: Option<[f64; 3]>,
    /// With `cat:"beam"`: the other end point `[x,y,z]` cm.
    pub b: Option<[f64; 3]>,
    /// Light it gives (fixtures come with theirs).
    pub light: Option<LightSpec>,
}

/// A piece's light, photometric.
#[derive(Debug, Default, Clone, Deserialize, serde::Serialize, JsonSchema)]
pub(crate) struct LightSpec {
    /// Luminous flux lm.
    pub lm: Option<f64>,
    /// Or electrical power W (flux from the lamp's efficacy).
    pub w: Option<f64>,
    /// led (100 lm/W), fluorescent (65), halogen (16), incandescent (12).
    pub lamp: Option<newera_core::LampType>,
    /// Color temperature K (2700 warm, 3000, 4000 neutral, 6500 daylight).
    pub k: Option<f64>,
    /// Spot beam angle degrees, pointing down; 0 = every direction.
    pub beam: Option<f64>,
    /// Emitting panel facing down, [w, d] cm.
    pub area: Option<[f64; 2]>,
    /// Source height, fraction of the piece's (default: base for spots and
    /// panels, else middle).
    pub z: Option<f64>,
    /// `false` turns it off.
    pub on: Option<bool>,
}

/// Applies a light spec over the piece's current light.
pub(crate) fn apply_light(piece: &mut newera_core::Furniture, spec: &LightSpec) {
    let mut light = piece
        .light
        .clone()
        .or_else(|| newera_catalog::light_for(piece))
        .unwrap_or_else(|| newera_core::Light::led(800.0, 2700.0, (0.5, 0.5, 0.5)));
    // Off keeps the fixture's data but gives no light (a catalog fixture
    // without a light of its own would fall back to its preset).
    if spec.on == Some(false) {
        light.lumens = Some(0.0);
        piece.light = Some(light);
        return;
    }
    if spec.on == Some(true) && light.lumens == Some(0.0) {
        light.lumens = newera_catalog::light_for(piece).and_then(|l| l.lumens);
    }
    if spec.lm.is_some() || spec.w.is_some() {
        light.lumens = spec.lm;
        light.watts = spec.w;
    }
    if spec.lamp.is_some() {
        light.lamp = spec.lamp;
    }
    if let Some(k) = spec.k {
        light.kelvin = Some(k.clamp(1000.0, 20_000.0));
    }
    if let Some(beam) = spec.beam {
        light.beam = (beam > 0.0).then_some(beam.min(179.0));
    }
    if let Some(area) = spec.area {
        light.area = (area[0] > 0.0 && area[1] > 0.0).then_some(area);
    }
    let directional = light.beam.is_some() || light.area.is_some();
    let z = spec.z.unwrap_or(if directional { 0.0 } else { 0.5 });
    if spec.z.is_some() || light.sources.is_empty() {
        light.sources = vec![newera_core::LightSource {
            x: 0.5,
            y: 0.5,
            z: z.clamp(0.0, 1.0),
            color: [255, 255, 255],
            diameter: None,
        }];
    }
    piece.light = Some(light);
}

/// Shortens a beam from `a` toward `b` to where its top would
/// reach the underside of a roof panel above it.
fn under_roof(home: &newera_core::Home, a: [f64; 3], b: [f64; 3], h: f64) -> ([f64; 3], [f64; 3]) {
    let view = home.level_view(home.current_level());
    let at = |t: f64| {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let length = ((b[0] - a[0]).hypot(b[1] - a[1])).hypot(b[2] - a[2]);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let steps = (length.ceil() as usize).clamp(2, 3000);
    let clear = |t: f64| {
        let p = at(t);
        newera_core::roof_height_at(&view, Point2::new(p[0], p[1]), p[2])
            .is_none_or(|roof| roof + 2.0 >= p[2] + h / 2.0)
    };
    // Keep the stretch around the start that stays clear.
    #[allow(clippy::cast_precision_loss)]
    let t_of = |i: usize| i as f64 / steps as f64;
    let first_blocked = (0..=steps).find(|&i| !clear(t_of(i)));
    match first_blocked {
        // Starting inside the roof (a tail under the eave): leave it as drawn.
        Some(0) | None => (a, b),
        Some(i) => (a, at(t_of(i - 1))),
    }
}

/// A box from `a` to `b` (plan cm, height cm above the storey floor) with a
/// `w` × `h` section: beams, rafters, posts, braces and sloping panels.
fn beam(
    doc: &mut Document,
    a: [f64; 3],
    b: [f64; 3],
    w: f64,
    h: f64,
) -> EditResult<newera_core::Furniture> {
    let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    let horizontal = dx.hypot(dy);
    let length = horizontal.hypot(dz);
    if length < 0.5 {
        return Err("a beam needs two distinct points".into());
    }
    let item = newera_catalog::find("box").ok_or("catalog has no box")?;
    let center = [
        f64::midpoint(a[0], b[0]),
        f64::midpoint(a[1], b[1]),
        f64::midpoint(a[2], b[2]),
    ];
    let mut piece = item.instantiate(doc.new_furniture_id(), Point2::new(center[0], center[1]));
    piece.name = "Viga".into();
    piece.width = w;
    piece.depth = length;
    piece.height = h;
    // The depth axis points from a to b on the plan; pitch lifts its far end.
    piece.angle = if horizontal > 1e-6 {
        (-dx).atan2(dy).to_degrees()
    } else {
        0.0
    };
    piece.pitch = -(dz / length).asin().to_degrees();
    if horizontal <= 1e-6 {
        piece.pitch = if dz > 0.0 { -90.0 } else { 90.0 };
    }
    piece.elevation = center[2] - h / 2.0;
    Ok(piece)
}

/// A pitched roof over a rectangle: a group of sloping panels, plus gable
/// walls when asked.
fn roof(doc: &mut Document, spec: &RoofSpec) -> EditResult<(newera_core::Furniture, Vec<Wall>)> {
    let [p0, p1, p2, ..] = spec.pts[..] else {
        return Err("a roof needs the 4 corners of a rectangle in `pts`".into());
    };
    let along = (p1.x - p0.x, p1.y - p0.y);
    let length = along.0.hypot(along.1);
    if length < 1.0 {
        return Err("roof corners pts[0] and pts[1] must differ".into());
    }
    let u = (along.0 / length, along.1 / length);
    // Across the ridge, toward pts[2].
    let mut n = (-u.1, u.0);
    let span = (p2.x - p0.x) * n.0 + (p2.y - p0.y) * n.1;
    if span < 0.0 {
        n = (-n.0, -n.1);
    }
    let span = span.abs();
    if span < 1.0 {
        return Err("roof rectangle has no width".into());
    }
    let shed = match spec.kind.as_deref().unwrap_or("gable") {
        "gable" | "a-frame" => false,
        "shed" => true,
        other => return Err(format!("unknown roof kind `{other}` (gable, shed)")),
    };
    let eave = spec.h.unwrap_or(Wall::DEFAULT_HEIGHT);
    let run = if shed { span } else { span / 2.0 };
    let rise = match (spec.ridge_h, spec.pitch) {
        (Some(ridge), _) => ridge - eave,
        (None, pitch) => run * pitch.unwrap_or(30.0).clamp(1.0, 85.0).to_radians().tan(),
    };
    if rise <= 0.0 {
        return Err("the ridge must be higher than the eaves".into());
    }
    let slope = rise / run;
    let overhang = spec.overhang.unwrap_or(30.0).max(0.0);
    let t = spec.t.unwrap_or(12.0).max(1.0);
    let at = |s: f64, k: f64| Point2::new(p0.x + u.0 * s + n.0 * k, p0.y + u.1 * s + n.1 * k);
    let mid = length / 2.0;
    // Each panel runs from its eave (overhang included) up to the ridge line,
    // with its centerline half a thickness above the rafters.
    let slopes: Vec<(f64, f64)> = if shed {
        vec![(0.0, span)]
    } else {
        vec![(0.0, span / 2.0), (span, span / 2.0)]
    };
    let mut panels = Vec::new();
    let lift = t / 2.0 * (1.0 + slope * slope).sqrt();
    let color = spec.color.unwrap_or([150, 75, 55]);
    // Skylights in plan coordinates along the ridge (s) and across it (k).
    let holes: Vec<(f64, f64, f64, f64)> = spec
        .skylights
        .iter()
        .map(|h| {
            let s = (h.at.x - p0.x) * u.0 + (h.at.y - p0.y) * u.1;
            let k = (h.at.x - p0.x) * n.0 + (h.at.y - p0.y) * n.1;
            let (w, d) = (h.w.unwrap_or(80.0) / 2.0, h.d.unwrap_or(100.0) / 2.0);
            (s - w, s + w, k - d, k + d)
        })
        .collect();
    for (from, to) in slopes {
        let dir = (to - from).signum();
        // e: horizontal distance up the slope from the eave line.
        let e_of = |k: f64| (k - from) * dir;
        let k_of = |e: f64| from + dir * e;
        let mut make = |doc: &mut Document,
                        s0: f64,
                        s1: f64,
                        e0: f64,
                        e1: f64,
                        glass: bool|
         -> EditResult<()> {
            if s1 - s0 < 0.5 || e1 - e0 < 0.5 {
                return Ok(());
            }
            let sm = f64::midpoint(s0, s1);
            let (a, b) = (at(sm, k_of(e0)), at(sm, k_of(e1)));
            let thickness = if glass { (t * 0.3).max(1.0) } else { t };
            let mut panel = beam(
                doc,
                [a.x, a.y, eave + e0 * slope + lift],
                [b.x, b.y, eave + e1 * slope + lift],
                s1 - s0,
                thickness,
            )?;
            if glass {
                panel.name = "Claraboia".into();
                panel.color = Some([168, 206, 226]);
                panel.opacity = Some(0.35);
            } else {
                panel.name = "Água do telhado".into();
                panel.color = Some(color);
            }
            panels.push(panel);
            Ok(())
        };
        let run = (to - from).abs();
        let (start, end) = (-overhang, length + overhang);
        let mut mine: Vec<(f64, f64, f64, f64)> = holes
            .iter()
            .map(|&(s0, s1, k0, k1)| {
                let (e0, e1) = (e_of(k0).min(e_of(k1)), e_of(k0).max(e_of(k1)));
                (s0.max(start), s1.min(end), e0.max(0.0), e1.min(run))
            })
            .filter(|&(s0, s1, e0, e1)| s1 > s0 && e1 > e0)
            .collect();
        mine.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut cursor = start;
        for &(s0, s1, e0, e1) in &mine {
            if s0 < cursor {
                return Err("skylights on the same slope overlap along the ridge".into());
            }
            make(doc, cursor, s0, -overhang, run, false)?;
            make(doc, s0, s1, -overhang, e0, false)?;
            make(doc, s0, s1, e0, e1, true)?;
            make(doc, s0, s1, e1, run, false)?;
            cursor = s1;
        }
        make(doc, cursor, end, -overhang, run, false)?;
    }
    // Ridge cap: the panels' square ends leave a V notch along the top
    // (t·sinθ wide, t/2·cosθ deep) that shows the sky; a cap closes it.
    if !shed {
        let hyp = (1.0 + slope * slope).sqrt();
        let (sin, cos) = (slope / hyp, 1.0 / hyp);
        let notch_depth = t / 2.0 * cos;
        let cap_h = notch_depth + 3.0;
        let top = eave + rise + lift + t / 2.0 * cos;
        let (a, b) = (at(-overhang, span / 2.0), at(length + overhang, span / 2.0));
        let z = top - notch_depth + cap_h / 2.0;
        let mut cap = beam(doc, [a.x, a.y, z], [b.x, b.y, z], t * sin + 6.0, cap_h)?;
        cap.name = "Cumeeira".into();
        cap.color = Some(color);
        panels.push(cap);
    }
    let mut gables = Vec::new();
    if spec.gables {
        for s in [0.0, length] {
            let heights = if shed {
                vec![eave, eave + rise]
            } else {
                vec![eave, eave + rise, eave]
            };
            let pts: Vec<Point2> = if shed {
                vec![at(s, 0.0), at(s, span)]
            } else {
                vec![at(s, 0.0), at(s, span / 2.0), at(s, span)]
            };
            for (i, pair) in pts.windows(2).enumerate() {
                let mut wall = Wall::new(doc.new_wall_id(), pair[0], pair[1]);
                wall.height = heights[i].max(1.0);
                wall.height_at_end = Some(heights[i + 1].max(1.0));
                gables.push(wall);
            }
        }
    }
    let item = newera_catalog::find("box").ok_or("catalog has no box")?;
    let center = at(mid, span / 2.0);
    let mut group = item.instantiate(doc.new_furniture_id(), center);
    group.catalog = "group".into();
    group.name = spec.name.clone().unwrap_or_else(|| "Telhado".into());
    group.angle = (-n.0).atan2(n.1).to_degrees();
    group.width = length + 2.0 * overhang;
    group.depth = span + 2.0 * overhang;
    group.elevation = eave - overhang * slope;
    group.height = rise + overhang * slope + 2.0 * t;
    group.children = panels;
    Ok((group, gables))
}

/// Moves a piece already aligned on a wall's axis so its back touches the
/// wall face. An explicit angle chooses the side; otherwise face the house centre.
pub(crate) fn back_to_wall(
    doc: &Document,
    piece: &mut newera_core::Furniture,
    wall: &Wall,
    angle: Option<f64>,
) {
    let a = piece.angle.to_radians();
    let front = (-a.sin(), a.cos());
    let center = doc.home().bounds().map_or(piece.position, |(min, max)| {
        Point2::new(min.x.midpoint(max.x), min.y.midpoint(max.y))
    });
    let toward_center =
        (center.x - piece.position.x) * front.0 + (center.y - piece.position.y) * front.1;
    let toward = angle.map_or(toward_center, |a| {
        let a = a.to_radians();
        -a.sin() * front.0 + a.cos() * front.1
    });
    let side = if toward >= 0.0 { 1.0 } else { -1.0 };
    piece.angle = angle.unwrap_or(piece.angle + if side < 0.0 { 180.0 } else { 0.0 });
    // Project the final footprint onto the wall normal, including oblique pieces.
    let (sin, cos) = piece.angle.to_radians().sin_cos();
    let offset = wall.thickness / 2.0
        + (cos * front.0 + sin * front.1).abs() * piece.width / 2.0
        + (-sin * front.0 + cos * front.1).abs() * piece.depth / 2.0;
    piece.position = Point2::new(
        piece.position.x + front.0 * side * offset,
        piece.position.y + front.1 * side * offset,
    );
}

/// The wall of the current storey closest to `at` within snapping distance,
/// with the distance along it.
fn nearest_wall(doc: &Document, at: Point2) -> Option<(newera_core::Wall, f64)> {
    let home = doc.home();
    let level = home.current_level();
    home.walls
        .iter()
        .filter(|w| home.on_level(w.level, level))
        .filter_map(|w| {
            let (a, b) = (w.start, w.end);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len2 = (dx * dx + dy * dy).max(1e-9);
            let t = (((at.x - a.x) * dx + (at.y - a.y) * dy) / len2).clamp(0.0, 1.0);
            let p = Point2::new(a.x + dx * t, a.y + dy * t);
            let distance = p.distance(at);
            (distance <= (w.thickness / 2.0 + 30.0)).then(|| (distance, w, t * len2.sqrt()))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, w, along)| (w.clone(), along))
}

/// Turns a door so its leaf swings to the side of `into`, keeping the hinge
/// at the same end of the opening.
fn swing_into(piece: &mut newera_core::Furniture, into: Point2) {
    let Some(swing) = newera_core::door_swing(piece) else {
        return;
    };
    #[allow(clippy::cast_precision_loss)]
    let n = swing.len() as f64;
    let (cx, cy) = swing
        .iter()
        .fold((0.0, 0.0), |(x, y), p| (x + p.x / n, y + p.y / n));
    let a = piece.angle.to_radians();
    let normal = (-a.sin(), a.cos());
    let side =
        |x: f64, y: f64| (x - piece.position.x) * normal.0 + (y - piece.position.y) * normal.1;
    if side(cx, cy) * side(into.x, into.y) < 0.0 {
        piece.angle += 180.0;
        if let Some(opening) = piece.opening.as_mut() {
            opening.hinge_right = !opening.hinge_right;
        }
    }
}

/// Fills what each item leaves out from `defaults` (e.g. the same `cat`,
/// section and color for a row of rafters).
pub(crate) fn with_defaults(
    items: Vec<PlaceSpec>,
    defaults: Option<&PlaceSpec>,
) -> EditResult<Vec<PlaceSpec>> {
    let Some(defaults) = defaults else {
        return Ok(items);
    };
    let base = serde_json::to_value(defaults).map_err(|e| e.to_string())?;
    items
        .into_iter()
        .map(|item| {
            let mut value = serde_json::to_value(&item).map_err(|e| e.to_string())?;
            if let (Some(target), Some(base)) = (value.as_object_mut(), base.as_object()) {
                for (key, default) in base {
                    let missing = match target.get(key) {
                        None | Some(serde_json::Value::Null) => true,
                        Some(serde_json::Value::String(s)) => s.is_empty(),
                        _ => false,
                    };
                    if missing && !default.is_null() {
                        target.insert(key.clone(), default.clone());
                    }
                }
            }
            serde_json::from_value(value).map_err(|e| e.to_string())
        })
        .collect()
}

/// The unit a piece's model file was placed with, when it was given.
pub(crate) const MODEL_UNIT_KEY: &str = "newera:model-unit";

/// A model's size from its natural one and the w/d/h asked: all three as
/// given with `stretch`, else scaled evenly by the one or two given (they
/// must agree), so a chair asked 90 cm tall keeps its proportions.
fn model_size(
    natural: [f64; 3],
    asked: [Option<f64>; 3],
    stretch: Option<bool>,
) -> EditResult<[f64; 3]> {
    if stretch == Some(true) {
        return Ok(std::array::from_fn(|k| asked[k].unwrap_or(natural[k])));
    }
    let factors: Vec<f64> = asked
        .iter()
        .zip(natural)
        .filter_map(|(a, n)| a.map(|a| a / n))
        .collect();
    let Some(&first) = factors.first() else {
        return Ok(natural);
    };
    if factors.iter().any(|f| (f - first).abs() > 0.02 * first) {
        return Err(format!(
            "w/d/h {:?} would stretch the model's {:?} cm unevenly (×{:?}): give one of them to scale it evenly, or stretch=true",
            asked.map(|a| a.map(|v| (v * 10.0).round() / 10.0)),
            natural.map(|v| (v * 10.0).round() / 10.0),
            factors
                .iter()
                .map(|f| (f * 1000.0).round() / 1000.0)
                .collect::<Vec<_>>()
        ));
    }
    Ok(std::array::from_fn(|k| {
        asked[k].unwrap_or(natural[k] * first)
    }))
}

/// Places catalog pieces in one undoable step; returns their ids.
pub(crate) fn place(doc: &mut Document, items: Vec<PlaceSpec>) -> EditResult<Vec<String>> {
    place_noting(doc, items).map(|placed| placed.ids)
}

/// What [`place_noting`] did besides placing.
#[derive(Debug, Default)]
pub(crate) struct Placed {
    pub ids: Vec<String>,
    /// Pieces turned to put their back on a wall: `f3:back to w2`.
    pub turned: Vec<String>,
    /// Imported models that are not drawn as their file is:
    /// `f12 chair.glb: 2 warnings (normal map of material tecido is not drawn; …)`.
    pub imported: Vec<String>,
}

/// [`place`], also naming the pieces it turned on its own: a piece whose
/// back belongs on a wall, put `at` a point beside one with no `angle` or
/// `facing`, goes back to that wall — `f3:back to w2`.
pub(crate) fn place_noting(doc: &mut Document, items: Vec<PlaceSpec>) -> EditResult<Placed> {
    let mut turned = Vec::new();
    let mut imported = Vec::new();
    if items.is_empty() {
        return Err("nothing to place".into());
    }
    let mut commands = Vec::with_capacity(items.len());
    let mut ids = Vec::with_capacity(items.len());
    let mut placed_here: Vec<newera_core::Furniture> = Vec::new();
    let count = items.len();
    let mut refused: Vec<String> = Vec::new();
    let mut one = |mut spec: PlaceSpec| -> EditResult<()> {
        // `library:<name>@<v>`: that version's file, at the size, in the
        // unit and with the photos and measurements it was published with.
        let library = match spec
            .model
            .as_deref()
            .and_then(crate::tools::library::resolve)
        {
            Some(found) => Some(found?),
            None => None,
        };
        if let Some((entry, path)) = &library {
            spec.model = Some(path.clone());
            spec.unit = spec.unit.take().or_else(|| entry.unit.clone());
            if spec.w.is_none() && spec.d.is_none() && spec.h.is_none() {
                [spec.w, spec.d, spec.h] = entry.size.map(Some);
                spec.stretch = Some(true);
            }
        }
        if spec.model.is_none() && (spec.unit.is_some() || spec.stretch.is_some()) {
            return Err("unit and stretch are for model=<file>".into());
        }
        if spec.cat == "beam" && (spec.a.is_some() || spec.b.is_some()) {
            let (Some(a), Some(b)) = (spec.a, spec.b) else {
                return Err("a beam needs `a` and `b` as [x,y,z]".into());
            };
            let (w, h) = (spec.w.unwrap_or(10.0), spec.h.unwrap_or(20.0));
            // Rafters and posts stop under the roof instead of piercing it.
            let (a, b) = under_roof(doc.home(), a, b, h);
            let mut piece = beam(doc, a, b, w, h)?;
            piece.name = spec.name.clone().unwrap_or(piece.name);
            piece.color = spec.color.or(Some([176, 132, 92]));
            ids.push(piece.id.to_string());
            commands.push(Command::insert(piece));
            return Ok(());
        }
        // The two ways an agent reaches for a piece already in the project
        // before finding `copy`: the model path `catalog(scope=project)`
        // gives — embedded in the project, not a file on disk — and the id
        // itself in `cat`. Both mean "this one again".
        let source_id = spec.copy.clone().or_else(|| {
            let home = doc.home();
            let pieces = || {
                home.furniture
                    .iter()
                    .flat_map(newera_core::Furniture::flatten)
            };
            if let Some(model) = &spec.model
                && !doc.resolve_asset(model).exists()
            {
                return pieces()
                    .find(|f| f.model.as_deref() == Some(model.as_str()))
                    .map(|f| f.id.to_string());
            }
            (spec.model.is_none() && newera_catalog::find(&spec.cat).is_none())
                .then(|| spec.cat.parse::<newera_core::FurnitureId>().ok())
                .flatten()
                .and_then(|id| home.find_piece(id))
                .map(|f| f.id.to_string())
        });
        if let Some(raw) = &source_id
            && (spec.unit.is_some() || spec.stretch.is_some())
        {
            return Err(format!(
                "unit and stretch are for a model file: this repeats {raw}, at its unit and size"
            ));
        }
        let copied = match &source_id {
            Some(raw) => {
                let id: newera_core::FurnitureId = raw.parse().map_err(|e| format!("copy: {e}"))?;
                let mut piece = doc
                    .home()
                    .find_piece(id)
                    .ok_or_else(|| format!("copy: {raw} not found"))?
                    .clone();
                newera_core::arrange::renumber_piece(doc, &mut piece);
                piece.level = None;
                Some(piece)
            }
            None => None,
        };
        // The copy as it was, with its new ids: what its parts follow from.
        let source = copied.clone();
        let mut piece = if let Some(mut piece) = copied {
            piece.position = spec.at.unwrap_or(piece.position);
            piece
        } else if let Some(model) = &spec.model {
            let path = doc.resolve_asset(model);
            let unit = spec
                .unit
                .as_deref()
                .map(|u| {
                    newera_catalog::Unit::parse(u).ok_or_else(|| {
                        format!("unit `{u}`: {}", newera_catalog::Unit::NAMES.join(", "))
                    })
                })
                .transpose()?;
            let loaded = newera_catalog::load_model_in(&path, unit)
                .map_err(|e| {
                    format!(
                        "{}: {e} (a model embedded in the project is repeated with copy=<id of a piece using it>)",
                        path.display()
                    )
                })?;
            let name = path
                .file_stem()
                .map_or_else(|| "Modelo".to_owned(), |n| n.to_string_lossy().into_owned());
            let id = doc.new_furniture_id();
            let size = model_size(loaded.size, [spec.w, spec.d, spec.h], spec.stretch)?;
            let mut warnings = loaded.report.warnings.clone();
            // A wrong guess makes a piece 10 or 100 times its size: said
            // when what came in is no size for furniture.
            let largest = loaded.size.iter().copied().fold(0.0, f64::max);
            if loaded.report.guessed && !(10.0..=400.0).contains(&largest) {
                warnings.push(format!(
                    "unit guessed as {}, so it is {largest:.0} cm across: unit= says what its numbers are",
                    loaded.report.unit
                ));
            }
            let scale: Vec<f64> = size
                .iter()
                .zip(loaded.size)
                .map(|(s, n)| (s / n * 1000.0).round() / 1000.0)
                .collect();
            if scale.iter().any(|f| (f - scale[0]).abs() > 0.02 * scale[0]) {
                warnings.push(format!("stretched ×{scale:?} (width, depth, height)"));
            }
            if !warnings.is_empty() {
                imported.push(format!(
                    "{id} {model}: {} warning{} ({})",
                    warnings.len(),
                    if warnings.len() == 1 { "" } else { "s" },
                    warnings.join("; ")
                ));
            }
            let mut properties = newera_core::Properties::new();
            if let Some(unit) = unit {
                properties.insert(MODEL_UNIT_KEY.into(), unit.name().into());
            }
            newera_core::Furniture {
                properties,
                id,
                catalog: "imported".to_owned(),
                name,
                position: spec.at.unwrap_or_default(),
                elevation: 0.0,
                angle: 0.0,
                width: size[0],
                depth: size[1],
                height: size[2],
                mirrored: false,
                color: None,
                opening: None,
                model: Some(model.clone()),
                visible: true,
                level: None,
                ..Default::default()
            }
        } else {
            let item = newera_catalog::find(&spec.cat).ok_or_else(|| {
                format!(
                    "unknown catalog id `{}` (use the catalog tool; to repeat a piece already in the project, copy=<its id>)",
                    spec.cat
                )
            })?;
            item.instantiate(doc.new_furniture_id(), spec.at.unwrap_or_default())
        };
        piece.width = spec.w.unwrap_or(piece.width);
        piece.depth = spec.d.unwrap_or(piece.depth);
        piece.height = spec.h.unwrap_or(piece.height);
        piece.elevation = spec.elev.unwrap_or(piece.elevation);
        piece.pitch = spec.pitch.unwrap_or(piece.pitch);
        piece.roll = spec.roll.unwrap_or(piece.roll);
        piece.name = spec.name.unwrap_or(piece.name);
        if let Some((entry, _)) = library {
            piece.info.model_name = piece.info.model_name.take().or(entry.title);
            piece.info.brand = piece.info.brand.take().or(entry.brand);
            piece.info.url = piece.info.url.take().or(entry.source);
            piece.references = entry.references;
            piece.measures = entry.measures;
            piece.materials = entry.materials;
            piece.model_parts = entry.model_parts;
            piece.model_transform = entry.model_transform.unwrap_or_default();
            piece.model_far = entry.model_far;
        }
        piece.color = spec.color.or(piece.color);
        if let Some(raw) = &spec.mat {
            finish(&mut piece, raw, spec.color.is_some())?;
        }
        if let Some(o) = spec.opacity {
            piece.opacity = (o < 1.0).then_some(o.clamp(0.0, 1.0));
        }
        piece.mirrored = spec
            .mirror
            .unwrap_or(source.as_ref().is_some_and(|s| s.mirrored));
        // Fixtures resized keep their panel matching the new size.
        if piece.light.is_some()
            && newera_catalog::find(&piece.catalog).is_some_and(|i| i.light.is_some())
        {
            piece.light = newera_catalog::light_for(&piece);
        }
        if let Some(light) = &spec.light {
            apply_light(&mut piece, light);
        }
        if let (Some(right), Some(opening)) = (spec.hinge_right, piece.opening.as_mut()) {
            opening.hinge_right = right;
        }
        if let Some(glass) = &spec.glass {
            let glass = glass.trim().to_lowercase();
            if !["laminated", "tempered", "tempered-laminated", "wired"].contains(&glass.as_str()) {
                return Err(format!(
                    "glass: laminated, tempered, tempered-laminated or wired (not {glass})"
                ));
            }
            piece
                .properties
                .insert(newera_core::guard::GLASS_KEY.into(), glass);
        }
        if let Some(gap) = spec.gap {
            piece
                .properties
                .insert(newera_core::guard::GAP_KEY.into(), gap.to_string());
        }
        if spec.angle.is_some() && spec.facing.is_some() {
            return Err("give `angle` or `facing`, not both".into());
        }
        let mut angle = spec.angle;
        // It stands on the storey being edited — or, given a wall, on that
        // wall's storey, whichever one is shown.
        piece.level = doc.home().current_level();
        match (&spec.wall, spec.at) {
            (Some(wall), _) => {
                let wall_id = wall.parse().map_err(|e| format!("{e}"))?;
                let wall = doc
                    .home()
                    .wall(wall_id)
                    .ok_or_else(|| format!("{wall} not found"))?
                    .clone();
                piece.level = doc.home().resolve_level(wall.level);
                let along = spec
                    .along
                    .unwrap_or_else(|| wall.start.distance(wall.end) / 2.0);
                newera_core::align_to_wall(&mut piece, &wall, along);
                if let Some(depth) = spec.d {
                    piece.depth = depth;
                }
                if angle.is_none()
                    && let Some(facing) = &spec.facing
                {
                    angle = Some(facing.angle(doc, piece.position)?);
                }
                if !piece.is_opening() {
                    back_to_wall(doc, &mut piece, &wall, angle);
                }
            }
            (None, Some(at)) => {
                if piece.is_opening()
                    && let Some((wall, along)) = nearest_wall(doc, at)
                {
                    newera_core::align_to_wall(&mut piece, &wall, along);
                    if let Some(depth) = spec.d {
                        piece.depth = depth;
                    }
                } else if angle.is_none()
                    && let Some(facing) = &spec.facing
                {
                    angle = Some(facing.angle(doc, at)?);
                } else if angle.is_none()
                    && source.is_none()
                    && newera_core::front::of(&piece.catalog).stance
                        == newera_core::front::Stance::Wall
                    && let Some(wall) = back_against_nearest_wall(doc, &mut piece)
                {
                    turned.push(format!("{}:back to {wall}", piece.id));
                }
            }
            (None, None) => {
                return Err(format!(
                    "`{}` needs `at` or `wall`",
                    spec.copy.as_deref().unwrap_or(&spec.cat)
                ));
            }
        }
        if let Some(into) = spec.into {
            swing_into(&mut piece, into);
        }
        if let Some(angle) = angle {
            piece.angle = angle;
        }
        // A copied group's parts follow its box to the new place and size.
        if let Some(source) = &source
            && piece.is_group()
        {
            piece.follow_group_change(source);
        }
        // A fixed point goes onto its structure: a wall point onto a wall's
        // face, a ceiling point up to the ceiling — never loose, never into
        // glass or an opening's span.
        // Against the plan with the pieces this same call placed before it:
        // a counter and the tower set into it can come together.
        let staged;
        let context: &newera_core::Home = if placed_here.is_empty() {
            doc.home()
        } else {
            let mut home = doc.home().clone();
            home.furniture.extend(placed_here.iter().cloned());
            staged = home;
            &staged
        };
        // A pendant given a drop and no height hangs that drop from the
        // ceiling; otherwise its shade stays where it is and the cord
        // reaches up (Document keeps every ceiling piece on the ceiling).
        if spec.h.is_some()
            && spec.elev.is_none()
            && newera_core::mounting::on_ceiling(&piece)
            && let Some(ceiling) = newera_core::mounting::ceiling_over(context, &piece)
        {
            piece.elevation = ceiling - piece.height.min(ceiling);
        }
        if spec.wall.is_none() && source.is_none() {
            newera_core::mounting::seat(context, &mut piece)?;
            // Behind a counter, an outlet or a network point goes above its top.
            if spec.elev.is_none()
                && let Some(top) = newera_core::mounting::counter_in_front(context, &piece)
            {
                piece.elevation = top + 15.0;
            }
        }
        if let Some(why) = newera_core::mounting::blocked(context, &piece) {
            return Err(match newera_core::mounting::nearest_free(context, &piece) {
                Some(free) => format!(
                    "{why} O lugar livre mais próximo nesta parede: at=[{}, {}].",
                    free.x.round(),
                    free.y.round()
                ),
                None => why.to_string(),
            });
        }
        placed_here.push(piece.clone());
        ids.push(piece.id.to_string());
        commands.push(Command::insert(piece));
        Ok(())
    };
    // Every item is looked at, so a batch says all it cannot place at once;
    // one refused and none of them is placed.
    for (k, spec) in items.into_iter().enumerate() {
        let what = spec.copy.clone().unwrap_or_else(|| spec.cat.clone());
        if let Err(why) = one(spec) {
            refused.push(if count == 1 {
                why
            } else {
                format!("items[{k}] `{what}`: {why}")
            });
        }
    }
    match refused.len() {
        0 => {}
        _ if count == 1 => return Err(refused.remove(0)),
        n => {
            return Err(format!(
                "nothing placed: {n} of the {count} items refused, the others are fine as given — {}",
                refused.join(" | ")
            ));
        }
    }
    doc.execute(Command::Batch { commands }).map_err(core)?;
    Ok(Placed {
        ids,
        turned,
        imported,
    })
}

/// How far behind a piece's back a wall may be and still be the wall it
/// was meant to stand against, cm.
const BACK_REACH: f64 = 30.0;

/// Turns a piece that belongs on a wall so its back is on the wall right
/// behind it, front to the room, keeping the gap it was given (and taking
/// it out of the wall if it was in it). `None` when no wall is that close.
fn back_against_nearest_wall(
    doc: &Document,
    piece: &mut newera_core::Furniture,
) -> Option<newera_core::WallId> {
    let home = doc.home();
    let level = home.current_level();
    let at = piece.position;
    let (wall, foot, normal, gap) = home
        .walls
        .iter()
        .filter(|w| home.on_level(w.level, level))
        .filter_map(|w| {
            let (a, b) = (w.start, w.end);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len = dx.hypot(dy);
            if len < 1.0 {
                return None;
            }
            let t = ((at.x - a.x) * dx + (at.y - a.y) * dy) / (len * len);
            // Behind the piece, not past the end of the wall.
            if !(0.0..=1.0).contains(&t) {
                return None;
            }
            // A curb or a parapet lower than the piece is not what its back
            // was meant for; a gable is, where it stands tall enough.
            let tall = w.height + (w.height_at_end.unwrap_or(w.height) - w.height) * t;
            if tall < piece.elevation + piece.height {
                return None;
            }
            let foot = Point2::new(a.x + dx * t, a.y + dy * t);
            let distance = foot.distance(at);
            if distance < 1e-6 {
                return None;
            }
            let normal = Point2::new((at.x - foot.x) / distance, (at.y - foot.y) / distance);
            // What is left between its back and the wall face once turned.
            let gap = distance - w.thickness / 2.0 - piece.depth / 2.0;
            (gap <= BACK_REACH).then_some((w, foot, normal, gap))
        })
        .min_by(|a, b| a.3.abs().total_cmp(&b.3.abs()))?;
    piece.angle = newera_core::front::angle_facing(normal);
    let keep = wall.thickness / 2.0 + piece.depth / 2.0 + gap.max(0.0);
    piece.position = Point2::new(foot.x + normal.x * keep, foot.y + normal.y * keep);
    Some(wall.id)
}

#[cfg(test)]
mod place_tests {
    use super::*;

    #[test]
    fn a_finish_that_is_only_a_color_paints_the_piece() {
        let mut doc = Document::default();
        let ids = place(
            &mut doc,
            vec![
                PlaceSpec {
                    cat: "wardrobe".into(),
                    at: Some(Point2::new(100.0, 100.0)),
                    mat: Some("#c8b89a".into()),
                    ..PlaceSpec::default()
                },
                PlaceSpec {
                    cat: "wardrobe".into(),
                    at: Some(Point2::new(300.0, 100.0)),
                    mat: Some("wood #c8b89a".into()),
                    ..PlaceSpec::default()
                },
                PlaceSpec {
                    cat: "wardrobe".into(),
                    at: Some(Point2::new(500.0, 100.0)),
                    color: Some([1, 2, 3]),
                    mat: Some("#c8b89a".into()),
                    ..PlaceSpec::default()
                },
            ],
        )
        .unwrap();
        let piece = |doc: &Document, id: &str| {
            doc.home()
                .furniture
                .iter()
                .find(|f| f.id.to_string() == id)
                .unwrap()
                .clone()
        };
        let plain = piece(&doc, &ids[0]);
        assert_eq!(plain.color, Some([0xc8, 0xb8, 0x9a]));
        assert_eq!(plain.texture, None);
        // A pattern is a finish; its color tints it.
        let wood = piece(&doc, &ids[1]);
        assert!(wood.texture.is_some_and(|t| t.pattern.is_some()));
        assert_eq!(piece(&doc, &ids[2]).color, Some([1, 2, 3]), "color wins");
        // Changed later, the same.
        update(
            &mut doc,
            vec![UpdateSpec {
                id: ids[1].clone(),
                mat: Some("#445566".into()),
                ..UpdateSpec::default()
            }],
        )
        .unwrap();
        let repainted = piece(&doc, &ids[1]);
        assert_eq!(repainted.color, Some([0x44, 0x55, 0x66]));
        assert_eq!(repainted.texture, None, "the wood is gone");
    }

    #[test]
    fn a_batch_says_every_item_it_refuses_and_places_none() {
        let mut doc = Document::default();
        let spec = |cat: &str, x: f64| PlaceSpec {
            cat: cat.into(),
            at: Some(Point2::new(x, 100.0)),
            ..PlaceSpec::default()
        };
        let before = doc.home().furniture.len();
        let why = place(
            &mut doc,
            vec![
                spec("bed-double", 100.0),
                spec("no-such-thing", 300.0),
                spec("outlet-low", 500.0),
            ],
        )
        .unwrap_err();
        assert!(
            why.starts_with("nothing placed: 2 of the 3 items refused"),
            "{why}"
        );
        assert!(
            why.contains("items[1] `no-such-thing`: unknown catalog id"),
            "{why}"
        );
        assert!(why.contains("items[2] `outlet-low`: "), "{why}");
        assert!(!why.contains("items[0]"), "{why}");
        assert_eq!(doc.home().furniture.len(), before, "none placed");
        // Alone, an item is refused in its own words.
        let alone = place(&mut doc, vec![spec("no-such-thing", 300.0)]).unwrap_err();
        assert!(alone.starts_with("unknown catalog id"), "{alone}");
    }

    #[test]
    fn doors_snap_to_the_nearest_wall_and_swing_into_a_chosen_side() {
        let mut doc = Document::default();
        create(
            &mut doc,
            CreateParams {
                walls: vec![WallPath {
                    pts: vec![Point2::new(0.0, 0.0), Point2::new(500.0, 0.0)],
                    ..WallPath::default()
                }],
                ..CreateParams::default()
            },
        )
        .unwrap();
        for into_y in [100.0, -100.0] {
            let ids = place(
                &mut doc,
                vec![PlaceSpec {
                    cat: "door".into(),
                    at: Some(Point2::new(203.0, 12.0)),
                    into: Some(Point2::new(200.0, into_y)),
                    ..PlaceSpec::default()
                }],
            )
            .unwrap();
            let door = doc
                .home()
                .furniture
                .iter()
                .find(|f| f.id.to_string() == ids[0])
                .unwrap()
                .clone();
            // Snapped onto the wall axis, as thick as the wall.
            assert!(door.position.y.abs() < 1e-6 && (door.position.x - 203.0).abs() < 1e-6);
            assert!((door.depth - doc.home().walls[0].thickness).abs() < 1e-9);
            let swing = newera_core::door_swing(&door).unwrap();
            #[allow(clippy::cast_precision_loss)]
            let cy = swing.iter().map(|p| p.y).sum::<f64>() / swing.len() as f64;
            assert!(cy * into_y > 0.0, "swings toward {into_y}: {cy}");
        }
        // Far from every wall a door stays where it was put.
        let ids = place(
            &mut doc,
            vec![PlaceSpec {
                cat: "door".into(),
                at: Some(Point2::new(200.0, 300.0)),
                ..PlaceSpec::default()
            }],
        )
        .unwrap();
        let door = doc
            .home()
            .furniture
            .iter()
            .find(|f| f.id.to_string() == ids[0])
            .unwrap();
        assert!((door.position.y - 300.0).abs() < 1e-6);
    }

    #[test]
    fn doors_align_to_walls_and_furniture_backs_onto_them() {
        let mut doc = Document::default();
        create(
            &mut doc,
            CreateParams {
                walls: vec![WallPath {
                    pts: vec![
                        Point2::new(0.0, 0.0),
                        Point2::new(500.0, 0.0),
                        Point2::new(500.0, 400.0),
                        Point2::new(0.0, 400.0),
                    ],
                    closed: true,
                    ..WallPath::default()
                }],
                ..CreateParams::default()
            },
        )
        .unwrap();
        let ids = place(
            &mut doc,
            vec![
                PlaceSpec {
                    cat: "door".into(),
                    wall: Some("w1".into()),
                    along: Some(100.0),
                    ..PlaceSpec::default()
                },
                PlaceSpec {
                    cat: "sofa-3".into(),
                    wall: Some("w3".into()),
                    ..PlaceSpec::default()
                },
                PlaceSpec {
                    cat: "rug".into(),
                    at: Some(Point2::new(250.0, 200.0)),
                    w: Some(250.0),
                    ..PlaceSpec::default()
                },
            ],
        )
        .unwrap();
        assert_eq!(ids, ["f5", "f6", "f7"]);
        let home = doc.home();
        let door = &home.furniture[0];
        assert_eq!((door.position, door.depth), (Point2::new(100.0, 0.0), 15.0));
        assert_eq!(home.wall_cuts()[0].len(), 1, "door cuts w1");
        let sofa = &home.furniture[1];
        // w3 runs (500,400)->(0,400); the sofa sits inside the room with its back on the wall.
        assert!(
            (sofa.position.y - (400.0 - 7.5 - 45.0)).abs() < 1e-6,
            "{:?}",
            sofa.position
        );
        assert!(
            newera_core::check_layout(home).is_empty(),
            "{:?}",
            newera_core::check_layout(home)
        );
        assert!((home.furniture[2].width - 250.0).abs() < 1e-9);
        assert!(
            place(
                &mut doc,
                vec![PlaceSpec {
                    cat: "nope".into(),
                    at: Some(Point2::default()),
                    ..PlaceSpec::default()
                }]
            )
            .is_err()
        );
    }
}
