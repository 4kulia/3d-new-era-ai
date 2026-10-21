//! Furniture, doors and windows: real objects with real dimensions.
//!
//! A piece is a box of `width × depth × height` cm placed on the plan by its
//! center, rotated by `angle` and lifted by `elevation`. What it looks like
//! comes from its catalog entry (or an imported model); the core only knows
//! its size, which is what layout, collisions and wall openings need.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::elements::Wall;
use crate::error::{CoreError, CoreResult};
use crate::geometry::Point2;
use crate::ids::{FurnitureId, LevelId};
use crate::materials::Material;
use crate::style::{Properties, TextStyle, is_default, is_zero};

fn yes() -> bool {
    true
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if signature
fn is_true(value: &bool) -> bool {
    *value
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpeningKind {
    #[default]
    Door,
    Window,
    /// An open passage without a leaf.
    Passage,
}

/// A door or window leaf that turns around a vertical axis. Values are
/// fractions of the piece's width (x, leaf width) and depth (y).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sash {
    pub x_axis: f64,
    pub y_axis: f64,
    pub width: f64,
    /// Degrees.
    pub start_angle: f64,
    pub end_angle: f64,
}

/// Where and how a door or window cuts the wall, as fractions of the
/// piece's size (depth for thickness/distance, width for width/left, height
/// for height/top) plus an optional outline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WallCutOut {
    pub wall_thickness: f64,
    pub wall_distance: f64,
    pub wall_width: f64,
    pub wall_left: f64,
    pub wall_height: f64,
    pub wall_top: f64,
    /// SVG path of the hole in a unit square (front view), when not rectangular.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    /// Cut through the whole wall, not only the part the piece overlaps.
    #[serde(default)]
    pub both_sides: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bound_to_wall: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub width_depth_deformable: bool,
}

impl Default for WallCutOut {
    fn default() -> Self {
        Self {
            wall_thickness: 1.0,
            wall_distance: 0.0,
            wall_width: 1.0,
            wall_left: 0.0,
            wall_height: 1.0,
            wall_top: 0.0,
            shape: None,
            both_sides: false,
            bound_to_wall: false,
            width_depth_deformable: true,
        }
    }
}

/// Makes a piece cut a hole through the wall it sits in.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Opening {
    pub kind: OpeningKind,
    /// Hinges on the right side (seen from the front) instead of the left.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hinge_right: bool,
    /// Number of leaves (1 or 2).
    #[serde(default = "Opening::one", skip_serializing_if = "Opening::is_one")]
    pub leaves: u8,
    /// Sliding leaves: no swing area.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sliding: bool,
    /// Explicit leaves (imported doors); overrides `leaves`/`hinge_right` when set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sashes: Vec<Sash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_out: Option<WallCutOut>,
}

/// A light emitter inside a piece, in fractions of its size.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LightSource {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub color: [u8; 3],
    /// Fraction of the piece's width; `None` for a point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter: Option<f64>,
}

/// Makes a piece emit light.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Light {
    /// 0 to 1.
    pub power: f64,
    pub sources: Vec<LightSource>,
    /// Model materials that glow.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_materials: Vec<String>,
    /// Luminous flux, lm (overrides `watts` and `power`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lumens: Option<f64>,
    /// Electrical power, W; flux follows from the lamp's efficacy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watts: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lamp: Option<crate::lighting::LampType>,
    /// Color temperature, K (overrides the sources' color).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kelvin: Option<f64>,
    /// Full beam angle of a spot pointing down, degrees (at half intensity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beam: Option<f64>,
    /// Emitting panel facing down, width × depth cm (LED panels and strips).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<[f64; 2]>,
    /// An area emitter faces the ceiling instead of the floor (cove lighting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_upward: Option<bool>,
}

/// Override of one material of an imported model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelMaterial {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<Material>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shininess: Option<f64>,
    /// How many times the material's image repeats, against the file's own
    /// mapping: 2 draws the weave at half its size, the piece unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<f64>,
}

/// A change to one named part of an imported model (the names `model`
/// lists), in cm of the piece: x across its width, y toward its front, z up.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ModelPart {
    pub name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Moved by `[x, y, z]` cm.
    #[serde(default, skip_serializing_if = "is_default")]
    pub offset: [f64; 3],
    /// Scaled about its center, per axis `[x, y, z]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<[f64; 3]>,
}

/// A lighter file drawn for a piece seen from afar — a weave as an alpha
/// sheet, fewer segments — in the same box, its materials and parts
/// answering by the same names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FarModel {
    pub file: String,
    /// Farther than this from the camera, cm, the lighter file is drawn.
    pub beyond: f64,
    /// The detailed file always, however far (a close review).
    #[serde(default, skip_serializing_if = "is_false")]
    pub off: bool,
}

impl FarModel {
    /// The file to draw for a camera `distance` cm away, or `None` for the
    /// piece's own.
    #[must_use]
    pub fn file_at(&self, distance: Option<f64>) -> Option<&str> {
        (!self.off && distance.is_some_and(|d| d > self.beyond)).then_some(self.file.as_str())
    }
}

/// A photo of the product a piece reproduces, kept with the project so a
/// review can put the model next to it again.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Reference {
    /// The image file; it travels with the project.
    pub file: String,
    /// The side it shows: `front`, `back`, `left`, `right`, `top`, `aerial`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// The part of the model it shows up close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// Where it came from: a product page, a catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A measurement of the product, with where it came from: what the model
/// is checked against, apart from the box it was fitted to.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Measure {
    /// `width`, `depth`, `height`, or `<part>.height` (its top above the floor).
    pub what: String,
    pub cm: f64,
    /// Where it came from: a product page, a drawing, a tape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Measured or stated by the maker, not estimated.
    #[serde(default, skip_serializing_if = "is_false")]
    pub confirmed: bool,
}

/// Descriptive data that doesn't affect geometry.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct PieceInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub information: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Id in the catalog it came from (e.g. `eTeks#frontDoor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_catalog_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vat_percentage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// Commercial model name or code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Product page or reference link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Catalog icon image file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Top view image drawn in the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_icon: Option<String>,
    /// The shop product it is, by the hosted service's product id. Prices
    /// live there, never in the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// Whether the estimate buys it; `None` is [`PieceInfo::purchase`]'s
    /// default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purchase: Option<Purchase>,
}

impl PieceInfo {
    /// Whether the piece is to be bought: as declared, else bought when it
    /// is a shop product and left out when it is not.
    #[must_use]
    pub fn purchase(&self) -> Purchase {
        self.purchase.unwrap_or(if self.product.is_some() {
            Purchase::Buy
        } else {
            Purchase::Excluded
        })
    }
}

/// What the estimate does with a piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Purchase {
    /// Bought: a line of the estimate.
    Buy,
    /// Already owned: drawn, not bought.
    Existing,
    /// Left out of the estimate.
    Excluded,
}

/// What the user may change on a piece.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PieceLocks {
    pub movable: bool,
    pub resizable: bool,
    /// Width, depth and height can change independently.
    pub deformable: bool,
    pub texturable: bool,
    /// Can be tilted (pitch/roll).
    pub horizontally_rotatable: bool,
}

impl Default for PieceLocks {
    fn default() -> Self {
        Self {
            movable: true,
            resizable: true,
            deformable: true,
            texturable: true,
            horizontally_rotatable: true,
        }
    }
}

/// How an imported model file maps onto the piece's box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelTransform {
    /// Rotation applied to the model before fitting it to the box, rows of
    /// a 3×3 matrix in model axes (y up).
    pub rotation: [[f64; 3]; 3],
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub centered_at_origin: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub back_face_shown: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub flags: u32,
    /// Size of the model file in bytes, as reported by its source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
}

impl Default for ModelTransform {
    fn default() -> Self {
        Self {
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            centered_at_origin: true,
            back_face_shown: false,
            flags: 0,
            file_size: None,
        }
    }
}

impl Opening {
    fn one() -> u8 {
        1
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn is_one(value: &u8) -> bool {
        *value == 1
    }
}

/// A solid made from a polygon instead of a catalog model. Points are in cm,
/// relative to the piece's center, and are scaled with its box when resized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SolidShape {
    /// Plan outline `[x, y]` raised by the piece height: slabs, mezzanines, decks.
    Outline(Vec<[f64; 2]>),
    /// Cross-section `[x across the width, z up]` swept along the depth:
    /// triangular gables, profiles, ramps.
    Profile(Vec<[f64; 2]>),
}

/// A piece of furniture, a door or a window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Furniture {
    pub id: FurnitureId,
    /// Catalog entry that defines its look, e.g. `bed-double`.
    pub catalog: String,
    pub name: String,
    /// Center on the plan, cm.
    pub position: Point2,
    /// Height of its bottom above the floor, cm.
    #[serde(default)]
    pub elevation: f64,
    /// Clockwise rotation in degrees.
    #[serde(default)]
    pub angle: f64,
    /// Size along its local x axis, cm.
    pub width: f64,
    /// Size along its local y axis (front to back), cm.
    pub depth: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub mirrored: bool,
    /// Main color override, `[r, g, b]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening: Option<Opening>,
    /// Imported 3D model file, when not built from the catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
    /// Storey it belongs to; `None` means the lowest level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<LevelId>,
    /// Technical project it belongs to; `None` is the architectural plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discipline: Option<crate::style::Discipline>,
    /// Tilt around its width axis, degrees.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pitch: f64,
    /// Tilt around its depth axis, degrees.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub roll: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub info: PieceInfo,
    #[serde(default, skip_serializing_if = "is_default")]
    pub locks: PieceLocks,
    #[serde(default, skip_serializing_if = "is_default")]
    pub model_transform: ModelTransform,
    /// Texture applied to the whole model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<Material>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shininess: Option<f64>,
    /// 0 (invisible) to 1 (opaque); glass panels, water, reference boards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// Built from a polygon instead of its catalog model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<SolidShape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<ModelMaterial>,
    /// Parts of its imported model hidden, moved or resized.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_parts: Vec<ModelPart>,
    /// A lighter file for its imported model, drawn from afar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_far: Option<FarModel>,
    /// Photos of the product it reproduces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    /// Measurements of the product, with their source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measures: Vec<Measure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<Light>,
    /// Pieces of a group, positioned in plan coordinates like top-level ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Furniture>,
    /// Fraction of the height where pieces dropped on top rest.
    #[serde(
        default = "Furniture::default_drop_on_top",
        skip_serializing_if = "Furniture::is_default_drop_on_top"
    )]
    pub drop_on_top: f64,
    /// Outline of the hole stairs cut in the floor above (SVG path, unit square).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staircase_cut_out: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub name_visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_style: Option<TextStyle>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub name_offset: [f64; 2],
    #[serde(default, skip_serializing_if = "is_zero")]
    pub name_angle: f64,
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub properties: Properties,
}

impl Default for Furniture {
    fn default() -> Self {
        Self {
            id: FurnitureId(0),
            catalog: String::new(),
            name: String::new(),
            position: Point2::new(0.0, 0.0),
            elevation: 0.0,
            angle: 0.0,
            width: 100.0,
            depth: 100.0,
            height: 100.0,
            mirrored: false,
            color: None,
            opening: None,
            model: None,
            visible: true,
            level: None,
            pitch: 0.0,
            roll: 0.0,
            discipline: None,
            info: PieceInfo::default(),
            locks: PieceLocks::default(),
            model_transform: ModelTransform::default(),
            texture: None,
            shininess: None,
            opacity: None,
            shape: None,
            materials: Vec::new(),
            model_parts: Vec::new(),
            model_far: None,
            references: Vec::new(),
            measures: Vec::new(),
            light: None,
            children: Vec::new(),
            drop_on_top: 1.0,
            staircase_cut_out: None,
            name_visible: false,
            name_style: None,
            name_offset: [0.0, 0.0],
            name_angle: 0.0,
            properties: Properties::new(),
        }
    }
}

impl Furniture {
    /// Explicit functional role, independent of the display name.
    pub const ROLE_KEY: &str = "newera:role";

    fn default_drop_on_top() -> f64 {
        1.0
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn is_default_drop_on_top(value: &f64) -> bool {
        (*value - 1.0).abs() < f64::EPSILON
    }

    /// Moves the piece and, for groups, every piece inside.
    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.position = Point2::new(self.position.x + dx, self.position.y + dy);
        for child in &mut self.children {
            child.translate(dx, dy);
        }
    }

    /// Visible pieces to draw: the piece itself, or for a group the visible
    /// non-group pieces inside it.
    pub fn visible_leaves(&self) -> Vec<&Self> {
        if !self.visible {
            return Vec::new();
        }
        if self.children.is_empty() {
            return vec![self];
        }
        self.children
            .iter()
            .flat_map(Self::visible_leaves)
            .collect()
    }

    /// After a group's box was moved, turned or resized from `before`, carries
    /// its pieces along: positions, angles, sizes and elevations follow.
    ///
    /// Joinery is resized the way a joiner resizes it, not by a factor: the
    /// boards on the edges of the box — sides, back, top, bottom, 6 cm thick
    /// or less — keep their thickness and stay on their edge, and the room
    /// between them takes the change. A board inside keeps its thickness too,
    /// holding on to the edge board it touches. A cupboard 30 cm wide made 40
    /// keeps 2 cm sides and 15 mm slides; its back grows 10 cm. A group with
    /// no board on an edge — a table and its chairs — scales as before.
    pub fn follow_group_change(&mut self, before: &Self) {
        if self.children.is_empty() {
            return;
        }
        let parts: Vec<&Self> = before.flatten().into_iter().skip(1).collect();
        let maps = [
            AxisMap::new(&parts, before, Along::Width, before.width, self.width),
            AxisMap::new(&parts, before, Along::Depth, before.depth, self.depth),
            AxisMap::new(&parts, before, Along::Height, before.height, self.height),
        ];
        let turn = self.angle - before.angle;
        let new = self.clone();
        for child in &mut self.children {
            carry_part(child, before, &new, &maps, turn);
        }
    }

    /// Like [`Self::follow_group_change`], but only the pieces in `stretch`
    /// change size along the width and the depth; every other piece keeps its
    /// size and moves with the stretch that happens before it.
    ///
    /// Joinery does not grow by a factor: the uprights keep their thickness
    /// and the opening between them takes the difference. Scaling the whole
    /// group turned a 5.8 cm upright into 4.2 and shrank the table the change
    /// was made for. Along an axis where the size changed and nothing listed
    /// spans it, the answer is an error rather than a guess. Heights still
    /// follow the group by proportion.
    pub fn follow_group_stretch(
        &mut self,
        before: &Self,
        stretch: &[crate::ids::FurnitureId],
    ) -> Result<(), String> {
        /// Along one axis: old size, change, the listed stretches merged,
        /// and their total length.
        type Stretch = (f64, f64, Vec<(f64, f64)>, f64);
        /// A local interval of `piece` inside `group`, along the group's x
        /// (`along_x`) or y.
        fn interval(group: &Furniture, piece: &Furniture, along_x: bool) -> (f64, f64) {
            let (cx, cy) = group.to_local(piece.position);
            let turn = (piece.angle - group.angle).rem_euclid(180.0);
            let sideways = (turn - 90.0).abs() < 45.0;
            let half = if along_x == sideways {
                piece.depth / 2.0
            } else {
                piece.width / 2.0
            };
            let c = if along_x { cx } else { cy };
            (c - half, c + half)
        }
        fn carry(
            piece: &mut Furniture,
            before: &Furniture,
            new: &Furniture,
            stretch: &[crate::ids::FurnitureId],
            map: &dyn Fn(usize, f64) -> f64,
            sz: f64,
            turn: f64,
        ) {
            let original = piece.clone();
            let (cx, cy) = before.to_local(original.position);
            let (x0, x1) = interval(before, &original, true);
            let (y0, y1) = interval(before, &original, false);
            let (nx, ny) = if stretch.contains(&original.id) {
                let (a, b) = (map(0, x0), map(0, x1));
                let (c, d) = (map(1, y0), map(1, y1));
                let turn_rel = (original.angle - before.angle).rem_euclid(180.0);
                let sideways = (turn_rel - 90.0).abs() < 45.0;
                let (along_w, along_d) = if sideways {
                    (d - c, b - a)
                } else {
                    (b - a, d - c)
                };
                piece.width = along_w;
                piece.depth = along_d;
                (a.midpoint(b), c.midpoint(d))
            } else {
                (map(0, cx), map(1, cy))
            };
            piece.position = new.to_plan((nx, ny));
            piece.angle += turn;
            piece.height *= sz;
            piece.elevation = new.elevation + (original.elevation - before.elevation) * sz;
            for child in &mut piece.children {
                carry(child, before, new, stretch, map, sz, turn);
            }
        }
        if self.children.is_empty() {
            return Ok(());
        }
        let pieces: Vec<&Furniture> = before.flatten().into_iter().skip(1).collect();
        let listed: Vec<&Furniture> = pieces
            .iter()
            .copied()
            .filter(|p| stretch.contains(&p.id))
            .collect();
        if let Some(missing) = stretch
            .iter()
            .find(|id| !pieces.iter().any(|p| p.id == **id))
        {
            return Err(format!("{missing} is not a part of {}", self.id));
        }
        // For each plan axis of the group: old size, change, and the stretch
        // map from an old local coordinate to a new one.
        let mut maps: Vec<Stretch> = Vec::new();
        for (along_x, old, new) in [
            (true, before.width, self.width),
            (false, before.depth, self.depth),
        ] {
            let delta = new - old;
            let mut spans: Vec<(f64, f64)> = listed
                .iter()
                .map(|p| interval(before, p, along_x))
                .collect();
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut union: Vec<(f64, f64)> = Vec::new();
            for (a, b) in spans {
                match union.last_mut() {
                    Some(last) if a <= last.1 => last.1 = last.1.max(b),
                    _ => union.push((a, b)),
                }
            }
            let length: f64 = union.iter().map(|(a, b)| b - a).sum();
            if delta.abs() > 1e-6 && length < 1e-6 {
                return Err(format!(
                    "nothing in stretch spans the group's {}; list the parts that take the change",
                    if along_x { "width" } else { "depth" }
                ));
            }
            maps.push((old, delta, union, length));
        }
        let map = |axis: usize, u: f64| {
            let (old, delta, union, length) = &maps[axis];
            if delta.abs() <= 1e-6 {
                return u;
            }
            let covered: f64 = union.iter().map(|(a, b)| (u.min(*b) - a).max(0.0)).sum();
            u + old / 2.0 - (old + delta) / 2.0 + delta * covered / length
        };
        let sz = if before.height.abs() > 1e-9 {
            self.height / before.height
        } else {
            1.0
        };
        let turn = self.angle - before.angle;
        let new = self.clone();
        for child in &mut self.children {
            carry(child, before, &new, stretch, &map, sz, turn);
        }
        Ok(())
    }

    /// A piece nested anywhere inside this one (not this one itself).
    pub fn find_part_mut(&mut self, id: crate::ids::FurnitureId) -> Option<&mut Self> {
        self.children.iter_mut().find_map(|child| {
            if child.id == id {
                Some(child)
            } else {
                child.find_part_mut(id)
            }
        })
    }

    pub fn is_group(&self) -> bool {
        !self.children.is_empty()
    }

    /// This piece and every piece nested in its groups, depth first.
    pub fn flatten(&self) -> Vec<&Self> {
        let mut out = vec![self];
        for child in &self.children {
            out.extend(child.flatten());
        }
        out
    }

    /// Maps a point in the piece's local frame (cm, origin at its center, x
    /// along the width, y along the depth) to plan coordinates.
    pub fn to_plan(&self, local: (f64, f64)) -> Point2 {
        let x = if self.mirrored { -local.0 } else { local.0 };
        let a = self.angle.to_radians();
        let (sin, cos) = a.sin_cos();
        Point2::new(
            self.position.x + x * cos - local.1 * sin,
            self.position.y + x * sin + local.1 * cos,
        )
    }

    /// Maps a plan point into the piece's local frame (inverse of [`Self::to_plan`]).
    pub fn to_local(&self, p: Point2) -> (f64, f64) {
        let (dx, dy) = (p.x - self.position.x, p.y - self.position.y);
        let a = self.angle.to_radians();
        let (sin, cos) = a.sin_cos();
        let x = dx * cos + dy * sin;
        let y = -dx * sin + dy * cos;
        (if self.mirrored { -x } else { x }, y)
    }

    /// Corners of its box after pitch and roll, in the local frame
    /// `(x, y up from its bottom, depth)`, cm.
    pub(crate) fn tilted_corners(&self) -> Vec<[f64; 3]> {
        let (hw, hd, hh) = (self.width / 2.0, self.depth / 2.0, self.height / 2.0);
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let (sr, cr) = self.roll.to_radians().sin_cos();
        let mut out = Vec::with_capacity(8);
        for x in [-hw, hw] {
            for y in [-hh, hh] {
                for z in [-hd, hd] {
                    // Roll around the depth axis, then pitch around the width axis.
                    let (x1, y1) = (x * cr - y * sr, x * sr + y * cr);
                    let (y2, z2) = (y1 * cp - z * sp, y1 * sp + z * cp);
                    out.push([x1, y2 + hh, z2]);
                }
            }
        }
        out
    }

    /// Plan footprint of what it really covers once tilted.
    pub fn projected_footprint(&self) -> [Point2; 4] {
        if self.pitch == 0.0 && self.roll == 0.0 {
            return self.footprint();
        }
        let corners = self.tilted_corners();
        let (x0, x1, z0, z1) = corners.iter().fold(
            (f64::MAX, f64::MIN, f64::MAX, f64::MIN),
            |(x0, x1, z0, z1), c| (x0.min(c[0]), x1.max(c[0]), z0.min(c[2]), z1.max(c[2])),
        );
        [(x0, z0), (x1, z0), (x1, z1), (x0, z1)].map(|p| self.to_plan(p))
    }

    /// Lowest and highest point above the floor, cm.
    pub fn height_range(&self) -> (f64, f64) {
        if self.pitch == 0.0 && self.roll == 0.0 {
            return (self.elevation, self.elevation + self.height);
        }
        let (lo, hi) = self
            .tilted_corners()
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), c| {
                (lo.min(c[1]), hi.max(c[1]))
            });
        (self.elevation + lo, self.elevation + hi)
    }

    /// Height of the transformed box underside, cm. Returns positive infinity
    /// outside the box projection; prefer `vertical_range_at` to handle absence.
    pub fn underside_at(&self, p: Point2) -> f64 {
        self.vertical_range_at(p)
            .map_or(f64::INFINITY, |range| range.0)
    }

    /// Height of the transformed box top, cm. Returns negative infinity
    /// outside the box projection; prefer `vertical_range_at` to handle absence.
    pub fn top_at(&self, p: Point2) -> f64 {
        self.vertical_range_at(p)
            .map_or(f64::NEG_INFINITY, |range| range.1)
    }

    /// Plan corners of its bounding box, counter-clockwise on screen.
    pub fn footprint(&self) -> [Point2; 4] {
        let (hw, hd) = (self.width / 2.0, self.depth / 2.0);
        [(-hw, -hd), (hw, -hd), (hw, hd), (-hw, hd)].map(|p| self.to_plan(p))
    }

    /// Stairs open a hole in the floor of the storey they climb to.
    /// Catalog ids starting with `stairs` are stairs, and so is a flight
    /// built by hand — a group or a solid — given the role `stair`.
    pub fn is_stairs(&self) -> bool {
        self.catalog.starts_with("stairs")
            || self
                .properties
                .get(Self::ROLE_KEY)
                .is_some_and(|r| r == "stair")
    }

    /// Something people sit on, which belongs pushed under its table.
    ///
    /// A seat overlapping the table it serves is how a plan is drawn, not a
    /// clash — and the two are impossible to tell apart from boxes alone,
    /// so this is the one place a layout check reads a name.
    pub fn is_seat(&self) -> bool {
        const CATALOGS: [&str; 8] = [
            "win",
            "ares",
            "chair",
            "stool",
            "office-chair",
            "armchair",
            "bench",
            "dining-set",
        ];
        if CATALOGS.iter().any(|c| self.catalog.starts_with(c)) {
            return true;
        }
        let name = self.name.to_lowercase();
        [
            "cadeira", "banqueta", "poltrona", "banco ", "chair", "stool",
        ]
        .iter()
        .any(|w| name.contains(w))
    }

    /// A worktop or table someone sits or works at: its top is at that
    /// height and it stands on the floor.
    pub fn is_table_height(&self) -> bool {
        let (lo, hi) = self.height_range();
        lo <= 5.0 && (65.0..=115.0).contains(&hi)
    }

    pub fn is_opening(&self) -> bool {
        self.opening.is_some()
    }

    pub fn contains(&self, p: Point2) -> bool {
        let (x, y) = self.to_local(p);
        x.abs() <= self.width / 2.0 && y.abs() <= self.depth / 2.0
    }

    pub(crate) fn validate(&self) -> CoreResult<()> {
        if !(self.position.is_finite() && self.elevation.is_finite() && self.angle.is_finite()) {
            return Err(CoreError::InvalidGeometry(
                "furniture position, elevation and angle must be finite".into(),
            ));
        }
        if !(self.width > 0.0 && self.depth > 0.0 && self.height > 0.0) {
            return Err(CoreError::InvalidGeometry(
                "furniture width, depth and height must be positive".into(),
            ));
        }
        if self.width > 100_000.0 || self.depth > 100_000.0 || self.height > 100_000.0 {
            return Err(CoreError::InvalidGeometry(
                "furniture is larger than 1 km".into(),
            ));
        }
        if self.catalog.trim().is_empty() {
            return Err(CoreError::InvalidGeometry(
                "furniture needs a catalog id".into(),
            ));
        }
        Ok(())
    }
}

/// The hole an opening cuts through a wall, in the wall's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallCut {
    pub furniture: FurnitureId,
    /// Distance along the wall from its start, cm.
    pub from: f64,
    pub to: f64,
    /// Heights above the floor, cm.
    pub bottom: f64,
    pub top: f64,
}

/// Openings cut into each straight wall, in `walls` order. A door or window
/// cuts every wall it is aligned with and lies in: one that sits where a
/// gable is split in two walls opens both halves.
pub fn wall_cuts(walls: &[Wall], furniture: &[Furniture]) -> Vec<Vec<WallCut>> {
    let mut cuts = vec![Vec::new(); walls.len()];
    for piece in furniture.iter().filter(|f| f.is_opening() && f.visible) {
        let hits: Vec<(usize, f64, f64)> = walls
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.is_arc())
            .filter_map(|(i, w)| {
                let len = w.start.distance(w.end);
                if len < 1e-9 {
                    return None;
                }
                let dir = ((w.end.x - w.start.x) / len, (w.end.y - w.start.y) / len);
                let wall_angle = dir.1.atan2(dir.0).to_degrees();
                let diff = (piece.angle - wall_angle).rem_euclid(180.0);
                if diff.min(180.0 - diff) > 5.0 {
                    return None;
                }
                let (dx, dy) = (piece.position.x - w.start.x, piece.position.y - w.start.y);
                let along = dx * dir.0 + dy * dir.1;
                let across = (-dx * dir.1 + dy * dir.0).abs();
                let reach = w.thickness.max(piece.depth) / 2.0 + 1.0;
                let half = piece.width / 2.0;
                (across <= reach && along + half > 0.1 && along - half < len - 0.1)
                    .then_some((i, along, len))
            })
            .collect();
        for (i, along, len) in hits {
            let wall = &walls[i];
            let (from, to) = (
                (along - piece.width / 2.0).max(0.0),
                (along + piece.width / 2.0).min(len),
            );
            // Sloping walls (gables): the opening fits under the lower side.
            let height_at = |s: f64| {
                wall.height + (wall.height_at_end.unwrap_or(wall.height) - wall.height) * (s / len)
            };
            let limit = height_at(from).min(height_at(to));
            let cut = WallCut {
                furniture: piece.id,
                from,
                to,
                bottom: piece.elevation.clamp(0.0, limit),
                top: (piece.elevation + piece.height).clamp(0.0, limit),
            };
            if cut.to - cut.from > 0.1 && cut.top - cut.bottom > 0.1 {
                cuts[i].push(cut);
            }
        }
    }
    for list in &mut cuts {
        list.sort_by(|a, b| a.from.total_cmp(&b.from));
    }
    cuts
}

/// Plan polygons of a wall with its door and window holes removed.
///
/// `outline` is the joined outline of `wall`; openings split it into pieces.
pub fn cut_outline(outline: &[Point2], wall: &Wall, cuts: &[WallCut]) -> Vec<Vec<Point2>> {
    use geo::{BooleanOps, Coord, LineString, Polygon};

    if cuts.is_empty() || outline.len() < 3 {
        return vec![outline.to_vec()];
    }
    let ring = |pts: &[Point2]| {
        let mut coords: Vec<Coord<f64>> = pts.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
        coords.push(coords[0]);
        LineString::new(coords)
    };
    let len = wall.start.distance(wall.end).max(1e-9);
    let dir = (
        (wall.end.x - wall.start.x) / len,
        (wall.end.y - wall.start.y) / len,
    );
    let normal = (-dir.1, dir.0);
    let reach = wall.thickness; // generous: beyond both faces
    let mut shape = geo::MultiPolygon::new(vec![Polygon::new(ring(outline), vec![])]);
    for cut in cuts {
        let at = |along: f64, side: f64| {
            Point2::new(
                wall.start.x + dir.0 * along + normal.0 * side,
                wall.start.y + dir.1 * along + normal.1 * side,
            )
        };
        let hole = [
            at(cut.from, -reach),
            at(cut.to, -reach),
            at(cut.to, reach),
            at(cut.from, reach),
        ];
        shape = shape.difference(&Polygon::new(ring(&hole), vec![]));
    }
    shape
        .iter()
        .map(|poly| {
            let mut pts: Vec<Point2> = poly
                .exterior()
                .coords()
                .map(|c| Point2::new(c.x, c.y))
                .collect();
            pts.pop();
            pts
        })
        .filter(|pts| pts.len() >= 3)
        .collect()
}

/// Places a piece in a wall: centered on it at `along` cm from its start,
/// aligned with it and, for openings, as deep as the wall is thick.
pub fn align_to_wall(piece: &mut Furniture, wall: &Wall, along: f64) {
    let len = wall.start.distance(wall.end).max(1e-9);
    let t = (along / len).clamp(0.0, 1.0);
    piece.position = wall.point_at(t);
    let (a, b) = (
        wall.point_at((t - 0.01).max(0.0)),
        wall.point_at((t + 0.01).min(1.0)),
    );
    piece.angle = (b.y - a.y).atan2(b.x - a.x).to_degrees();
    if piece.is_opening() {
        piece.depth = wall.thickness;
    }
}

/// Seats a door or window already in place in `wall` again, at `along`,
/// keeping the side it opens to.
///
/// [`align_to_wall`] takes the angle from the wall's direction, which is the
/// right thing for a new piece and the wrong one for a piece being nudged: a
/// bathroom door moved 4 cm along its own wall came out turned 180°, opening
/// onto the kitchen. Whichever of the two wall directions is nearer the angle
/// the piece had is kept.
pub(crate) fn reseat_in_wall(piece: &mut Furniture, wall: &Wall, along: f64) {
    let before = piece.angle;
    align_to_wall(piece, wall, along);
    let turned = (piece.angle - before).rem_euclid(360.0);
    if turned > 90.0 && turned < 270.0 {
        piece.angle = (piece.angle + 180.0).rem_euclid(360.0);
    }
}

/// One axis of a group, in the group's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Along {
    Width,
    Depth,
    Height,
}

/// Thickest a board can be and still keep its thickness on a resize, cm.
const BOARD: f64 = 6.0;
/// How close to an edge a board has to sit to hold on to it, cm.
const ON_EDGE: f64 = 1.0;

/// Where a piece of a group sits along one axis of it, in the group's frame:
/// centered for width and depth, from the group's bottom for height.
fn part_span(group: &Furniture, part: &Furniture, along: Along) -> (f64, f64) {
    if along == Along::Height {
        let lo = part.elevation - group.elevation;
        return (lo, lo + part.height);
    }
    let (cx, cy) = group.to_local(part.position);
    let turn = (part.angle - group.angle).rem_euclid(180.0);
    let sideways = (turn - 90.0).abs() < 45.0;
    let along_x = along == Along::Width;
    let half = if along_x == sideways {
        part.depth / 2.0
    } else {
        part.width / 2.0
    };
    let c = if along_x { cx } else { cy };
    (c - half, c + half)
}

/// How old coordinates along one axis of a group become new ones.
#[derive(Debug, Clone, Copy)]
struct AxisMap {
    lo: f64,
    old: f64,
    new: f64,
    /// The stretch between the edge boards, when there are edge boards and
    /// room left between them.
    zone: Option<(f64, f64)>,
}

impl AxisMap {
    fn new(parts: &[&Furniture], group: &Furniture, along: Along, old: f64, new: f64) -> Self {
        let lo = if along == Along::Height {
            0.0
        } else {
            -old / 2.0
        };
        let hi = lo + old;
        let (mut zone_lo, mut zone_hi, mut boards) = (lo, hi, 0);
        for part in parts {
            let (a, b) = part_span(group, part, along);
            if b - a > BOARD {
                continue;
            }
            if a - lo <= ON_EDGE {
                zone_lo = zone_lo.max(b);
                boards += 1;
            } else if hi - b <= ON_EDGE {
                zone_hi = zone_hi.min(a);
                boards += 1;
            }
        }
        let delta = new - old;
        let zone = (boards > 0
            && (old - new).abs() > 1e-9
            && zone_hi - zone_lo > 1e-6
            && zone_hi - zone_lo + delta > 1e-6)
            .then_some((zone_lo, zone_hi));
        Self { lo, old, new, zone }
    }

    fn new_lo(&self) -> f64 {
        if (self.lo).abs() < 1e-12 {
            0.0
        } else {
            -self.new / 2.0
        }
    }

    /// An old coordinate, mapped.
    fn point(&self, u: f64) -> f64 {
        let new_lo = self.new_lo();
        match self.zone {
            None => {
                let k = if self.old.abs() > 1e-9 {
                    self.new / self.old
                } else {
                    1.0
                };
                new_lo + (u - self.lo) * k
            }
            Some((z0, z1)) => {
                let hi = self.lo + self.old;
                if u <= z0 {
                    new_lo + (u - self.lo)
                } else if u >= z1 {
                    new_lo + self.new - (hi - u)
                } else {
                    let length = z1 - z0;
                    new_lo + (z0 - self.lo) + (u - z0) * (length + self.new - self.old) / length
                }
            }
        }
    }

    /// An old span, mapped: a board keeps its thickness, held by the edge it
    /// touches, anything else stretches with the room it spans.
    fn span(&self, (a, b): (f64, f64)) -> (f64, f64) {
        let Some((z0, z1)) = self.zone else {
            return (self.point(a), self.point(b));
        };
        if b - a > BOARD {
            return (self.point(a), self.point(b));
        }
        let size = b - a;
        if a <= z0 + ON_EDGE {
            let start = self.point(a.min(z0).max(a));
            (start, start + size)
        } else if b >= z1 - ON_EDGE {
            let end = self.point(b.max(z1).min(b));
            (end - size, end)
        } else {
            let c = self.point(f64::midpoint(a, b));
            (c - size / 2.0, c + size / 2.0)
        }
    }
}

/// Carries one piece of a group, and its own pieces, from `before` to `new`.
fn carry_part(
    piece: &mut Furniture,
    before: &Furniture,
    new: &Furniture,
    maps: &[AxisMap; 3],
    turn: f64,
) {
    let original = piece.clone();
    let (x0, x1) = maps[0].span(part_span(before, &original, Along::Width));
    let (y0, y1) = maps[1].span(part_span(before, &original, Along::Depth));
    let (z0, z1) = maps[2].span(part_span(before, &original, Along::Height));
    let rel = (original.angle - before.angle).rem_euclid(180.0);
    let sideways = (rel - 90.0).abs() < 45.0;
    let (along_w, along_d) = if sideways {
        (y1 - y0, x1 - x0)
    } else {
        (x1 - x0, y1 - y0)
    };
    piece.width = along_w;
    piece.depth = along_d;
    piece.height = z1 - z0;
    piece.elevation = new.elevation + z0;
    piece.position = new.to_plan((x0.midpoint(x1), y0.midpoint(y1)));
    piece.angle += turn;
    for child in &mut piece.children {
        carry_part(child, before, new, maps, turn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::WallId;

    fn piece(id: u64, at: (f64, f64), size: (f64, f64, f64)) -> Furniture {
        Furniture {
            id: FurnitureId(id),
            catalog: "box".into(),
            name: "Caixa".into(),
            position: Point2::new(at.0, at.1),
            elevation: 0.0,
            angle: 0.0,
            width: size.0,
            depth: size.1,
            height: size.2,
            mirrored: false,
            color: None,
            opening: None,
            model: None,
            visible: true,
            level: None,
            ..Default::default()
        }
    }

    #[test]
    fn local_and_plan_frames_are_inverse_with_rotation_and_mirror() {
        let mut f = piece(1, (100.0, 50.0), (200.0, 90.0, 80.0));
        f.angle = 90.0;
        f.mirrored = true;
        let p = f.to_plan((40.0, 10.0));
        let back = f.to_local(p);
        assert!((back.0 - 40.0).abs() < 1e-9 && (back.1 - 10.0).abs() < 1e-9);
        // 90° clockwise on screen: local +x points down (+y).
        let mut g = piece(2, (0.0, 0.0), (10.0, 10.0, 10.0));
        g.angle = 90.0;
        let q = g.to_plan((10.0, 0.0));
        assert!(q.x.abs() < 1e-9 && (q.y - 10.0).abs() < 1e-9, "{q:?}");
    }

    #[test]
    fn doors_cut_the_wall_they_sit_in() {
        let walls = vec![
            Wall::new(WallId(1), Point2::new(0.0, 0.0), Point2::new(500.0, 0.0)),
            Wall::new(
                WallId(2),
                Point2::new(500.0, 0.0),
                Point2::new(500.0, 400.0),
            ),
        ];
        let mut door = piece(3, (0.0, 0.0), (80.0, 15.0, 210.0));
        door.opening = Some(Opening::default());
        align_to_wall(&mut door, &walls[1], 100.0);
        assert!((door.angle - 90.0).abs() < 1e-6);
        assert_eq!(door.position, Point2::new(500.0, 100.0));

        let mut window = piece(4, (250.0, 3.0), (120.0, 15.0, 110.0));
        window.elevation = 100.0;
        window.opening = Some(Opening {
            kind: OpeningKind::Window,
            ..Opening::default()
        });
        let unrelated = piece(5, (250.0, 200.0), (80.0, 60.0, 70.0));

        let cuts = wall_cuts(&walls, &[door, window, unrelated]);
        assert_eq!(cuts[0].len(), 1);
        assert_eq!((cuts[0][0].from, cuts[0][0].to), (190.0, 310.0));
        assert_eq!((cuts[0][0].bottom, cuts[0][0].top), (100.0, 210.0));
        assert_eq!(cuts[1].len(), 1);
        assert_eq!(
            (cuts[1][0].from, cuts[1][0].to, cuts[1][0].top),
            (60.0, 140.0, 210.0)
        );

        // The window splits the free-standing wall outline in two pieces.
        let lone = Wall::new(WallId(9), Point2::new(0.0, 0.0), Point2::new(500.0, 0.0));
        let outline = crate::joins::wall_outlines(std::slice::from_ref(&lone)).remove(0);
        let pieces = cut_outline(&outline, &lone, &cuts[0]);
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        let area: f64 = pieces
            .iter()
            .map(|p| crate::geometry::polygon_area(p))
            .sum();
        assert!((area - (500.0 - 120.0) * 15.0).abs() < 1e-6, "{area}");
    }

    #[test]
    fn a_door_where_a_gable_is_split_opens_both_halves() {
        let mut left = Wall::new(WallId(1), Point2::new(0.0, 0.0), Point2::new(300.0, 0.0));
        left.height = 55.0;
        left.height_at_end = Some(675.0);
        let mut right = Wall::new(WallId(2), Point2::new(300.0, 0.0), Point2::new(600.0, 0.0));
        right.height = 675.0;
        right.height_at_end = Some(55.0);
        let mut door = piece(3, (300.0, 0.0), (90.0, 10.0, 210.0));
        door.elevation = 55.0;
        door.opening = Some(Opening::default());
        let cuts = wall_cuts(&[left, right], &[door]);
        assert_eq!((cuts[0][0].from, cuts[0][0].to), (255.0, 300.0));
        assert_eq!((cuts[1][0].from, cuts[1][0].to), (0.0, 45.0));
        assert!(cuts.iter().all(|c| (c[0].top - 265.0).abs() < 1e-9));
    }

    #[cfg(test)]
    mod group_tests {
        use super::*;

        #[test]
        fn a_group_stretches_what_is_listed_and_keeps_the_rest() {
            let part = |id: u64, x0: f64, x1: f64, d: f64| Furniture {
                id: FurnitureId(id),
                position: Point2::new(x0.midpoint(x1), 0.0),
                width: x1 - x0,
                depth: d,
                height: 90.0,
                ..Furniture::default()
            };
            // A peninsula face 180 cm wide: an upright, a 110 cm table with
            // an arm under it, and a filler — laid out from x = -90.
            let mut before = Furniture {
                id: FurnitureId(1),
                width: 180.0,
                depth: 60.0,
                height: 90.0,
                ..Furniture::default()
            };
            before.children = vec![
                part(2, -90.0, -84.2, 60.0),
                part(3, -84.2, 25.8, 60.0),
                part(4, -60.0, -57.0, 24.0),
                part(5, 25.8, 90.0, 60.0),
            ];
            let span = |g: &Furniture, id: u64| {
                let p = g.children.iter().find(|c| c.id.0 == id).unwrap();
                (p.position.x - p.width / 2.0, p.position.x + p.width / 2.0)
            };
            let close =
                |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;

            // 131 cm, the filler takes it all: the upright and the table keep
            // their size, and the group still starts at its left face.
            let mut after = Furniture {
                width: 131.0,
                ..before.clone()
            };
            after
                .follow_group_stretch(&before, &[FurnitureId(5)])
                .unwrap();
            assert!(
                close(span(&after, 2), (-65.5, -59.7)),
                "{:?}",
                span(&after, 2)
            );
            assert!(
                close(span(&after, 3), (-59.7, 50.3)),
                "{:?}",
                span(&after, 3)
            );
            assert!(
                close(span(&after, 5), (50.3, 65.5)),
                "{:?}",
                span(&after, 5)
            );
            assert!(
                (span(&after, 4).1 - span(&after, 4).0 - 3.0).abs() < 1e-6,
                "the arm keeps its size"
            );

            // The table grows to the tower and the filler goes: both listed,
            // the change split by their widths — and the upright is still 5.8.
            let mut both = Furniture {
                width: 131.0,
                ..before.clone()
            };
            both.follow_group_stretch(&before, &[FurnitureId(3), FurnitureId(5)])
                .unwrap();
            assert!((span(&both, 2).1 - span(&both, 2).0 - 5.8).abs() < 1e-6);
            assert!((span(&both, 5).1 - -span(&both, 2).0 - 131.0 + 65.5 + 65.5).abs() < 1e-6);
            assert!(
                close(span(&both, 3), (-59.7, -59.7 + 110.0 * 125.2 / 174.2)),
                "{:?}",
                span(&both, 3)
            );

            // Depth changed and nothing listed spans it: said, not guessed.
            let mut deeper = Furniture {
                depth: 70.0,
                ..before.clone()
            };
            let err = deeper.follow_group_stretch(&before, &[]).unwrap_err();
            assert!(err.contains("depth"), "{err}");
            assert!(
                before
                    .clone()
                    .follow_group_stretch(&before, &[FurnitureId(9)])
                    .is_err()
            );
        }

        #[test]
        fn a_cupboard_made_wider_keeps_its_boards_and_widens_its_room() {
            let part =
                |id: u64, name: &str, x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, h: f64| {
                    Furniture {
                        id: FurnitureId(id),
                        name: name.to_owned(),
                        position: Point2::new(x0.midpoint(x1), y0.midpoint(y1)),
                        width: x1 - x0,
                        depth: y1 - y0,
                        height: h,
                        elevation: z0,
                        ..Furniture::default()
                    }
                };
            // A broom cupboard 30 wide at x 615–645, as drawn in a real plan.
            let mut before = part(1, "vassoureiro", 615.0, 645.0, 415.6, 484.1, 0.0, 280.0);
            before.children = vec![
                part(
                    957,
                    "lateral esquerda",
                    615.0,
                    617.0,
                    419.1,
                    484.1,
                    0.0,
                    280.0,
                ),
                part(
                    1209,
                    "lateral direita",
                    643.0,
                    645.0,
                    419.1,
                    484.1,
                    0.0,
                    280.0,
                ),
                part(959, "fundo", 617.0, 643.0, 482.1, 484.1, 0.0, 280.0),
                part(963, "topo", 617.0, 643.0, 420.1, 483.1, 278.0, 2.0),
                part(1210, "frente", 615.0, 645.0, 417.1, 419.1, 0.0, 195.0),
                part(1212, "puxador", 628.5, 631.5, 415.6, 417.1, 100.0, 30.0),
                part(
                    1214,
                    "corrediça esquerda",
                    617.0,
                    618.5,
                    421.1,
                    481.1,
                    2.0,
                    4.0,
                ),
                part(1216, "fundo do cesto", 617.0, 643.0, 421.1, 481.1, 2.0, 2.0),
            ];
            let mut after = before.clone();
            after.width = 40.0;
            after.position.x = 635.0; // held on its left face
            after.follow_group_change(&before);
            let span = |id: u64| {
                let p = after
                    .flatten()
                    .into_iter()
                    .find(|p| p.id.0 == id)
                    .unwrap()
                    .clone();
                (p.position.x - p.width / 2.0, p.position.x + p.width / 2.0)
            };
            let close =
                |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
            assert!(
                close(span(957), (615.0, 617.0)),
                "left side stays 2 cm: {:?}",
                span(957)
            );
            assert!(
                close(span(1209), (653.0, 655.0)),
                "right side stays 2 cm, on the new edge: {:?}",
                span(1209)
            );
            assert!(
                close(span(959), (617.0, 653.0)),
                "the back takes the 10 cm: {:?}",
                span(959)
            );
            assert!(
                close(span(1210), (615.0, 655.0)),
                "the front spans it all: {:?}",
                span(1210)
            );
            assert!(
                close(span(1214), (617.0, 618.5)),
                "the slide keeps 15 mm on its side: {:?}",
                span(1214)
            );
            assert!(close(span(1216), (617.0, 653.0)), "{:?}", span(1216));
            assert!(
                (span(1212).1 - span(1212).0 - 3.0).abs() < 1e-6,
                "the pull keeps its size"
            );
            assert!(
                (span(1212).0.midpoint(span(1212).1) - 635.0).abs() < 1e-6,
                "centered on the front"
            );

            // Taller: the top keeps 2 cm and stays on top.
            let mut taller = before.clone();
            taller.height = 290.0;
            taller.follow_group_change(&before);
            let top = taller.children.iter().find(|p| p.id.0 == 963).unwrap();
            assert!(
                (top.height - 2.0).abs() < 1e-6 && (top.elevation - 288.0).abs() < 1e-6,
                "{top:?}"
            );
        }

        #[test]
        fn groups_carry_their_pieces() {
            let child = Furniture {
                position: Point2::new(110.0, 100.0),
                width: 20.0,
                ..Furniture::default()
            };
            let before = Furniture {
                position: Point2::new(100.0, 100.0),
                width: 40.0,
                children: vec![child],
                ..Furniture::default()
            };
            let mut after = before.clone();
            after.position = Point2::new(200.0, 100.0);
            after.width = 80.0;
            after.angle = 90.0;
            after.follow_group_change(&before);
            let moved = &after.children[0];
            assert!(
                (moved.position.x - 200.0).abs() < 1e-9,
                "{:?}",
                moved.position
            );
            assert!(
                (moved.position.y - 120.0).abs() < 1e-9,
                "{:?}",
                moved.position
            );
            assert!((moved.width - 40.0).abs() < 1e-9 && (moved.angle - 90.0).abs() < 1e-9);
        }
    }
}
