use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::elements::{BackgroundImage, Compass, Dimension, Element, Label, Level, Room, Wall};
use crate::error::{CoreError, CoreResult};
use crate::furniture::{Furniture, WallCut, wall_cuts};
use crate::geometry::Point2;
use crate::ids::{
    DimensionId, ElementId, FurnitureId, LabelId, LevelId, PolylineId, RoomId, WallId,
};
use crate::joins::wall_outlines;
use crate::style::{Cameras, Environment, Polyline, PrintSettings, Properties};

/// The whole project being edited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Home {
    pub name: String,
    #[serde(default)]
    pub walls: Vec<Wall>,
    #[serde(default)]
    pub rooms: Vec<Room>,
    #[serde(default)]
    pub dimensions: Vec<Dimension>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub furniture: Vec<Furniture>,
    /// Storeys, if the house has more than one. Empty means a single ground level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<Level>,
    /// Storey shown in the plan and where new elements go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_level: Option<LevelId>,
    #[serde(default)]
    pub compass: Compass,
    /// Findings looked at and accepted, by their key, with the reason.
    ///
    /// A review that keeps accusing what somebody already analysed makes the
    /// next agent redo the analysis — and a plan that is right never reaches
    /// zero, so nobody can tell "done" from "given up". These stay in the
    /// report, marked and explained, and stop costing score.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub accepted: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<BackgroundImage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub polylines: Vec<Polyline>,
    /// Height of new walls, cm.
    #[serde(default = "Home::default_wall_height")]
    pub wall_height: f64,
    #[serde(default, skip_serializing_if = "crate::style::is_default")]
    pub environment: Environment,
    #[serde(default, skip_serializing_if = "crate::style::is_default")]
    pub cameras: Cameras,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print: Option<PrintSettings>,
    /// Columns shown in the furniture list, e.g. `NAME`, `WIDTH`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub furniture_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub furniture_sort: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub furniture_descending: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub base_plan_locked: bool,
    /// Derived annotations shown on the plan.
    #[serde(default, skip_serializing_if = "crate::style::is_default")]
    pub annotations: crate::annotations::PlanAnnotations,
    /// Technical project being edited; new symbols and lines go there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_discipline: Option<crate::style::Discipline>,
    /// Technical projects hidden from the plan and 3D.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_disciplines: Vec<crate::style::Discipline>,
    /// Layers hidden from the plan drawing; the 3D keeps showing them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_layers: Vec<crate::layers::PlanLayer>,
    /// The 3D shows everything, ignoring hidden layers and projects: the
    /// whole building at once. Off, the 3D hides what the plan hides.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub show_all_in_3d: bool,
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub properties: Properties,
    /// Which shop product covers which floor, ceiling or wall side.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finish_assignments: Vec<crate::finishes::FinishAssignment>,
    /// Next number handed out for any id. Monotonic, so ids an agent saw
    /// earlier never point to a different element later.
    #[serde(default)]
    next_id: u64,
}

impl Default for Home {
    fn default() -> Self {
        Self {
            name: "Nova casa".to_owned(),
            walls: Vec::new(),
            rooms: Vec::new(),
            dimensions: Vec::new(),
            labels: Vec::new(),
            furniture: Vec::new(),
            levels: Vec::new(),
            selected_level: None,
            compass: Compass::default(),
            accepted: std::collections::BTreeMap::new(),
            background: None,
            polylines: Vec::new(),
            wall_height: Self::default_wall_height(),
            environment: Environment::default(),
            cameras: Cameras::default(),
            print: None,
            furniture_columns: Vec::new(),
            furniture_sort: None,
            furniture_descending: false,
            base_plan_locked: false,
            active_discipline: None,
            hidden_disciplines: Vec::new(),
            hidden_layers: Vec::new(),
            show_all_in_3d: false,
            annotations: crate::annotations::PlanAnnotations::default(),
            properties: Properties::new(),
            finish_assignments: Vec::new(),
            next_id: 1,
        }
    }
}

/// Generates typed accessors, id allocation and the generic element
/// operations for every element collection.
macro_rules! collections {
    ($($variant:ident, $ty:ident, $id:ident, $field:ident, $get:ident, $new_id:ident;)+) => {
        impl Home {
            $(
                pub fn $get(&self, id: $id) -> Option<&$ty> {
                    self.$field.iter().find(|e| e.id == id)
                }

                pub fn $new_id(&mut self) -> $id {
                    $id(self.allocate_id())
                }
            )+

            fn max_used_id(&self) -> u64 {
                [$(self.$field.iter().map(|e| e.id.0).max().unwrap_or(0)),+]
                    .into_iter()
                    .max()
                    .unwrap_or(0)
            }

            pub fn contains(&self, id: ElementId) -> bool {
                self.element(id).is_some()
            }

            /// A copy of the element with this id.
            pub fn element(&self, id: ElementId) -> Option<Element> {
                match id {
                    $(ElementId::$variant(id) => self.$get(id).cloned().map(Element::$variant),)+
                }
            }

            /// All elements, in a stable order (by kind, then insertion).
            pub fn elements(&self) -> impl Iterator<Item = Element> + '_ {
                std::iter::empty()
                    $(.chain(self.$field.iter().cloned().map(Element::$variant)))+
            }

            pub(crate) fn insert(&mut self, element: Element, index: Option<usize>) -> CoreResult<()> {
                element.validate()?;
                let id = element.id();
                if self.contains(id) {
                    return Err(CoreError::AlreadyExists(id));
                }
                match element {
                    $(Element::$variant(e) => {
                        let at = index.unwrap_or(self.$field.len()).min(self.$field.len());
                        self.$field.insert(at, e);
                    })+
                }
                Ok(())
            }

            /// Removes an element, returning it with the index it had.
            pub(crate) fn remove(&mut self, id: ElementId) -> CoreResult<(Element, usize)> {
                match id {
                    $(ElementId::$variant(typed) => {
                        let at = self.$field.iter().position(|e| e.id == typed).ok_or(CoreError::NotFound(id))?;
                        Ok((Element::$variant(self.$field.remove(at)), at))
                    })+
                }
            }

            /// Replaces an element with the same id, returning the previous one.
            pub(crate) fn replace(&mut self, element: Element) -> CoreResult<Element> {
                element.validate()?;
                let id = element.id();
                match element {
                    $(Element::$variant(e) => {
                        let slot = self.$field.iter_mut().find(|x| x.id == e.id).ok_or(CoreError::NotFound(id))?;
                        Ok(Element::$variant(std::mem::replace(slot, e)))
                    })+
                }
            }
        }
    };
}

impl Home {
    /// The piece a nested id belongs to: the one whose group holds it.
    ///
    /// Ids of parts show up in what reads and writes report — a group that
    /// rebuilt lists the parts that changed — so they are a fair thing for a
    /// caller to try. They are not editable on their own, and saying whose
    /// they are is the difference between a dead end and the next step.
    #[must_use]
    pub fn part_owner(&self, id: crate::ids::FurnitureId) -> Option<&Furniture> {
        self.furniture
            .iter()
            .find(|top| top.id != id && top.flatten().iter().any(|p| p.id == id))
    }
}

collections! {
    Wall, Wall, WallId, walls, wall, new_wall_id;
    Room, Room, RoomId, rooms, room, new_room_id;
    Dimension, Dimension, DimensionId, dimensions, dimension, new_dimension_id;
    Label, Label, LabelId, labels, label, new_label_id;
    Furniture, Furniture, FurnitureId, furniture, piece, new_furniture_id;
    Level, Level, LevelId, levels, level, new_level_id;
    Polyline, Polyline, PolylineId, polylines, polyline, new_polyline_id;
}

impl Home {
    fn default_wall_height() -> f64 {
        Wall::DEFAULT_HEIGHT
    }

    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    fn allocate_id(&mut self) -> u64 {
        // Also guards against files whose counter lags behind their elements.
        let id = self.next_id.max(self.max_used_id() + 1);
        self.next_id = id + 1;
        id
    }

    /// Floor outline of every wall with corners joined, in `walls` order.
    pub fn wall_outlines(&self) -> Vec<Vec<Point2>> {
        wall_outlines(&self.walls)
    }

    /// Levels sorted from the ground up.
    pub fn sorted_levels(&self) -> Vec<&Level> {
        let mut levels: Vec<&Level> = self.levels.iter().collect();
        levels.sort_by(|a, b| {
            a.elevation
                .total_cmp(&b.elevation)
                .then(a.elevation_index.cmp(&b.elevation_index))
        });
        levels
    }

    /// The level `None` stands for: the lowest one, if any exist.
    pub fn base_level(&self) -> Option<LevelId> {
        self.sorted_levels().first().map(|l| l.id)
    }

    /// Resolves an element's level to a concrete one (or `None` without levels).
    pub fn resolve_level(&self, level: Option<LevelId>) -> Option<LevelId> {
        level
            .filter(|id| self.level(*id).is_some())
            .or_else(|| self.base_level())
    }

    /// Whether a plan layer is out of sight: hidden itself, or lighting with
    /// the electrical project hidden — its lamps and fixtures are points of
    /// that project, and hiding it takes every one of them away, not only
    /// the outlets.
    pub fn layer_hidden(&self, layer: crate::layers::PlanLayer) -> bool {
        self.hidden_layers.contains(&layer)
            || (layer == crate::layers::PlanLayer::Lighting
                && self
                    .hidden_disciplines
                    .contains(&crate::style::Discipline::Electrical))
    }

    /// Whether a piece, a line or a note of this discipline and layer is
    /// shown in 3D: what the plan shows, unless the 3D is set to show all.
    pub fn shown_in_3d(
        &self,
        discipline: Option<crate::style::Discipline>,
        layer: Option<crate::layers::PlanLayer>,
    ) -> bool {
        self.show_all_in_3d
            || (discipline.is_none_or(|d| !self.hidden_disciplines.contains(&d))
                && layer.is_none_or(|l| !self.layer_hidden(l)))
    }

    /// The level currently shown and edited.
    pub fn current_level(&self) -> Option<LevelId> {
        self.resolve_level(self.selected_level)
    }

    /// Elevation (cm) of an element's floor.
    pub fn elevation_of(&self, level: Option<LevelId>) -> f64 {
        self.resolve_level(level)
            .and_then(|id| self.level(id))
            .map_or(0.0, |l| l.elevation)
    }

    /// Whether an element on `element_level` belongs to `level`.
    pub fn on_level(&self, element_level: Option<LevelId>, level: Option<LevelId>) -> bool {
        self.resolve_level(element_level) == self.resolve_level(level)
    }

    /// The piece with this id, looking inside groups.
    ///
    /// [`Home::piece`] only sees the tops of the furniture tree, but layout
    /// checks, measurements and joinery all report the leaf that is actually
    /// in the way — so anything resolving an id from a report needs this.
    pub fn find_piece(&self, id: FurnitureId) -> Option<&Furniture> {
        self.furniture
            .iter()
            .find_map(|top| top.flatten().into_iter().find(|f| f.id == id))
    }

    /// The storey a piece belongs to, found through its group.
    pub fn piece_level(&self, id: FurnitureId) -> Option<LevelId> {
        let top = self
            .furniture
            .iter()
            .find(|top| top.flatten().iter().any(|f| f.id == id))?;
        self.resolve_level(top.level)
    }

    /// Whether a level is a reference layer (a tracing, an earlier version).
    pub fn is_reference_level(&self, level: Option<LevelId>) -> bool {
        self.resolve_level(level)
            .and_then(|id| self.level(id))
            .is_some_and(crate::Level::is_reference)
    }

    /// Storeys that are being designed: everything but the reference layers.
    pub fn design_levels(&self) -> Vec<LevelId> {
        self.levels
            .iter()
            .filter(|l| !l.is_reference())
            .map(|l| l.id)
            .collect()
    }

    /// Pairs of non-reference storeys whose floor-to-ceiling ranges overlap.
    ///
    /// Two storeys at the same elevation are almost always layers — an
    /// imported plan, a previous layout — and every check that mixes them
    /// reports pieces "colliding" with their own older copies. Reads warn
    /// about this so nobody spends a session chasing it.
    pub fn stacked_levels(&self) -> Vec<(LevelId, LevelId)> {
        let mut out = Vec::new();
        let levels: Vec<&Level> = self.levels.iter().filter(|l| !l.is_reference()).collect();
        for (i, a) in levels.iter().enumerate() {
            for b in levels.iter().skip(i + 1) {
                let (a0, a1) = (a.elevation, a.elevation + a.height);
                let (b0, b1) = (b.elevation, b.elevation + b.height);
                if a0 < b1 - 1.0 && b0 < a1 - 1.0 {
                    out.push((a.id, b.id));
                }
            }
        }
        out
    }

    /// A copy of the home keeping only the elements of one level (and the
    /// home-wide settings). Geometry that must not mix storeys — wall joins,
    /// room detection, plan drawing, layout checks — works on such views.
    #[must_use]
    pub fn level_view(&self, level: Option<LevelId>) -> Self {
        let level = self.resolve_level(level);
        let keep = |l: Option<LevelId>| self.resolve_level(l) == level;
        Self {
            name: self.name.clone(),
            levels: self.levels.clone(),
            selected_level: self.selected_level,
            compass: self.compass.clone(),
            accepted: self.accepted.clone(),
            background: self.background.clone(),
            wall_height: self.wall_height,
            environment: self.environment.clone(),
            cameras: self.cameras.clone(),
            print: self.print.clone(),
            furniture_columns: self.furniture_columns.clone(),
            furniture_sort: self.furniture_sort.clone(),
            furniture_descending: self.furniture_descending,
            base_plan_locked: self.base_plan_locked,
            active_discipline: self.active_discipline,
            annotations: self.annotations,
            hidden_disciplines: self.hidden_disciplines.clone(),
            hidden_layers: self.hidden_layers.clone(),
            show_all_in_3d: self.show_all_in_3d,
            properties: self.properties.clone(),
            finish_assignments: self
                .finish_assignments
                .iter()
                .filter(|a| {
                    self.element(a.surface.element())
                        .is_some_and(|e| keep(e.level()))
                })
                .cloned()
                .collect(),
            next_id: self.next_id,
            polylines: self
                .polylines
                .iter()
                .filter(|e| keep(e.level))
                .cloned()
                .collect(),
            walls: self
                .walls
                .iter()
                .filter(|e| keep(e.level))
                .cloned()
                .collect(),
            rooms: self
                .rooms
                .iter()
                .filter(|e| keep(e.level))
                .cloned()
                .collect(),
            dimensions: self
                .dimensions
                .iter()
                .filter(|e| keep(e.level))
                .cloned()
                .collect(),
            labels: self
                .labels
                .iter()
                .filter(|e| keep(e.level))
                .cloned()
                .collect(),
            furniture: self
                .furniture
                .iter()
                .filter(|e| keep(e.level))
                .cloned()
                .collect(),
        }
    }

    /// Visits every file path the home refers to (models, textures, icons,
    /// background images), for packing and relocating assets.
    pub fn for_each_asset_mut(&mut self, visit: &mut impl FnMut(&mut String)) {
        fn material(visit: &mut impl FnMut(&mut String), m: &mut Option<crate::Material>) {
            if let Some(path) = m.as_mut().and_then(|m| m.image.as_mut()) {
                visit(path);
            }
        }
        fn piece(visit: &mut impl FnMut(&mut String), f: &mut Furniture) {
            for path in [&mut f.model, &mut f.info.icon, &mut f.info.plan_icon]
                .into_iter()
                .flatten()
            {
                visit(path);
            }
            if let Some(far) = &mut f.model_far {
                visit(&mut far.file);
            }
            for reference in &mut f.references {
                visit(&mut reference.file);
            }
            material(visit, &mut f.texture);
            for m in &mut f.materials {
                material(visit, &mut m.texture);
            }
            for child in &mut f.children {
                piece(visit, child);
            }
        }
        if let Some(bg) = &mut self.background {
            visit(&mut bg.path);
        }
        for level in &mut self.levels {
            if let Some(bg) = &mut level.background {
                visit(&mut bg.path);
            }
        }
        for wall in &mut self.walls {
            material(visit, &mut wall.left_side);
            material(visit, &mut wall.right_side);
            for b in [&mut wall.left_baseboard, &mut wall.right_baseboard]
                .into_iter()
                .flatten()
            {
                material(visit, &mut b.material);
            }
        }
        for room in &mut self.rooms {
            material(visit, &mut room.floor_material);
            material(visit, &mut room.ceiling_material);
        }
        for f in &mut self.furniture {
            piece(visit, f);
        }
        material(visit, &mut self.environment.ground_texture);
        material(visit, &mut self.environment.sky_texture);
    }

    /// Every file path the home refers to.
    pub fn asset_paths(&self) -> Vec<String> {
        fn material(out: &mut Vec<String>, m: Option<&crate::Material>) {
            if let Some(path) = m.and_then(|m| m.image.as_ref()) {
                out.push(path.clone());
            }
        }
        fn piece(out: &mut Vec<String>, f: &Furniture) {
            out.extend(
                [&f.model, &f.info.icon, &f.info.plan_icon]
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            out.extend(f.model_far.iter().map(|far| far.file.clone()));
            out.extend(f.references.iter().map(|r| r.file.clone()));
            material(out, f.texture.as_ref());
            for m in &f.materials {
                material(out, m.texture.as_ref());
            }
            for child in &f.children {
                piece(out, child);
            }
        }
        let mut out = Vec::new();
        if let Some(bg) = &self.background {
            out.push(bg.path.clone());
        }
        for level in &self.levels {
            if let Some(bg) = &level.background {
                out.push(bg.path.clone());
            }
        }
        for wall in &self.walls {
            material(&mut out, wall.left_side.as_ref());
            material(&mut out, wall.right_side.as_ref());
            for b in [&wall.left_baseboard, &wall.right_baseboard]
                .into_iter()
                .flatten()
            {
                material(&mut out, b.material.as_ref());
            }
        }
        for room in &self.rooms {
            material(&mut out, room.floor_material.as_ref());
            material(&mut out, room.ceiling_material.as_ref());
        }
        for f in &self.furniture {
            piece(&mut out, f);
        }
        material(&mut out, self.environment.ground_texture.as_ref());
        material(&mut out, self.environment.sky_texture.as_ref());
        out
    }

    /// Door and window holes in each wall, in `walls` order. An opening
    /// cuts only the walls of its own storey: the window upstairs stands
    /// over the wall below, at a height measured from another floor.
    pub fn wall_cuts(&self) -> Vec<Vec<WallCut>> {
        self.by_storey(|walls, storey| {
            let pieces: Vec<Furniture> = self
                .furniture
                .iter()
                .filter(|top| self.resolve_level(top.level) == storey)
                .flat_map(Furniture::flatten)
                .cloned()
                .collect();
            wall_cuts(walls, &pieces)
        })
    }

    /// The outline of every wall joined with the walls of its own storey
    /// only, in `walls` order: [`Home::wall_outlines`] joins whatever
    /// meets on the plan, a wall upstairs included.
    pub fn storey_wall_outlines(&self) -> Vec<Vec<Point2>> {
        self.by_storey(|walls, _| wall_outlines(walls))
    }

    /// `per` asked once for each storey's walls, its answers put back in
    /// `walls` order.
    fn by_storey<T: Clone + Default>(
        &self,
        per: impl Fn(&[Wall], Option<LevelId>) -> Vec<T>,
    ) -> Vec<T> {
        let mut out = vec![T::default(); self.walls.len()];
        let mut storeys: Vec<Option<LevelId>> = Vec::new();
        for wall in &self.walls {
            let storey = self.resolve_level(wall.level);
            if storeys.contains(&storey) {
                continue;
            }
            storeys.push(storey);
            let at: Vec<usize> = (0..self.walls.len())
                .filter(|i| self.resolve_level(self.walls[*i].level) == storey)
                .collect();
            let walls: Vec<Wall> = at.iter().map(|i| self.walls[*i].clone()).collect();
            for (i, answer) in at.into_iter().zip(per(&walls, storey)) {
                out[i] = answer;
            }
        }
        out
    }

    /// The smallest room on storey `level` with `p` inside: where rooms
    /// nest, the inner one.
    #[must_use]
    pub fn room_at(&self, p: Point2, level: Option<LevelId>) -> Option<&Room> {
        self.rooms
            .iter()
            .filter(|r| self.on_level(r.level, level))
            .filter(|r| r.points.len() >= 3 && crate::electrical::inside(&r.points, p))
            .min_by(|a, b| a.area().total_cmp(&b.area()))
    }

    /// Bounds of what is built (walls, rooms, furniture), ignoring texts,
    /// dimensions and lines that may sit far from the house. Falls back to
    /// [`Home::bounds`] for drawings without construction.
    pub fn building_bounds(&self) -> Option<(Point2, Point2)> {
        let points = self
            .walls
            .iter()
            .flat_map(Wall::centerline)
            .chain(self.rooms.iter().flat_map(|r| r.points.iter().copied()))
            .chain(
                self.furniture
                    .iter()
                    .filter(|f| f.visible)
                    .flat_map(Furniture::footprint),
            );
        points
            .fold(None, |acc: Option<(Point2, Point2)>, p| {
                let (min, max) = acc.unwrap_or((p, p));
                Some((
                    Point2::new(min.x.min(p.x), min.y.min(p.y)),
                    Point2::new(max.x.max(p.x), max.y.max(p.y)),
                ))
            })
            .or_else(|| self.bounds())
    }

    /// Axis-aligned bounds `(min, max)` of the drawing.
    pub fn bounds(&self) -> Option<(Point2, Point2)> {
        let points = self
            .walls
            .iter()
            .flat_map(Wall::centerline)
            .chain(self.rooms.iter().flat_map(|r| r.points.iter().copied()))
            .chain(self.dimensions.iter().flat_map(|d| [d.start, d.end]))
            .chain(self.labels.iter().map(|l| l.position))
            .chain(self.polylines.iter().flat_map(|p| p.points.iter().copied()))
            .chain(self.furniture.iter().flat_map(Furniture::footprint));
        points.fold(None, |acc, p| {
            let (min, max) = acc.unwrap_or((p, p));
            Some((
                Point2::new(min.x.min(p.x), min.y.min(p.y)),
                Point2::new(max.x.max(p.x), max.y.max(p.y)),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_asset_paths_matches_the_relocation_visitor_for_every_surface() {
        let material = |name: &str| {
            Some(crate::Material {
                image: Some(name.into()),
                ..crate::Material::default()
            })
        };
        let background = |name: &str| {
            Some(crate::BackgroundImage {
                path: name.into(),
                ..crate::BackgroundImage::default()
            })
        };
        let mut home = Home {
            background: background("background"),
            ..Home::default()
        };
        home.levels.push(crate::Level {
            background: background("level"),
            ..crate::Level::default()
        });
        let mut wall = Wall::new(WallId(1), Point2::new(0.0, 0.0), Point2::new(100.0, 0.0));
        wall.left_side = material("wall-left");
        wall.right_side = material("wall-right");
        wall.left_baseboard = Some(crate::Baseboard {
            thickness: 1.0,
            height: 10.0,
            material: material("base-left"),
        });
        wall.right_baseboard = Some(crate::Baseboard {
            thickness: 1.0,
            height: 10.0,
            material: material("base-right"),
        });
        home.walls.push(wall);
        let mut room = Room::new(RoomId(2), "Room", vec![]);
        room.floor_material = material("floor");
        room.ceiling_material = material("ceiling");
        home.rooms.push(room);
        let mut piece = Furniture {
            model: Some("model".into()),
            texture: material("piece"),
            ..Furniture::default()
        };
        piece.info.icon = Some("icon".into());
        piece.info.plan_icon = Some("plan-icon".into());
        piece.materials.push(crate::ModelMaterial {
            name: "finish".into(),
            key: None,
            color: None,
            texture: material("override"),
            shininess: None,
            repeat: None,
        });
        piece.children.push(Furniture {
            model: Some("child".into()),
            ..Furniture::default()
        });
        home.furniture.push(piece);
        home.environment.ground_texture = material("ground");
        home.environment.sky_texture = material("sky");
        let read = home.asset_paths();
        let mut visited = Vec::new();
        home.for_each_asset_mut(&mut |p| visited.push(p.clone()));
        assert_eq!(read.len(), 16);
        assert_eq!(read, visited);
    }

    #[test]
    fn allocated_ids_never_collide_with_existing_elements() {
        let mut home = Home::default();
        home.walls.push(Wall::new(
            WallId(40),
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
        ));
        assert_eq!(home.new_wall_id(), WallId(41));
        assert_eq!(home.new_room_id(), RoomId(42));
        assert_eq!(home.new_label_id(), LabelId(43));
    }

    #[test]
    fn generic_operations_round_trip() {
        let mut home = Home::default();
        let id = home.new_label_id();
        let label = Label {
            id,
            text: "Entrada".into(),
            position: Point2::new(10.0, 20.0),
            size: Label::DEFAULT_SIZE,
            angle: 0.0,
            level: None,
            ..Default::default()
        };
        home.insert(label.clone().into(), None).unwrap();
        assert_eq!(home.element(id.into()), Some(Element::Label(label.clone())));
        assert!(matches!(
            home.insert(label.clone().into(), None),
            Err(CoreError::AlreadyExists(_))
        ));
        let (removed, index) = home.remove(id.into()).unwrap();
        assert_eq!((removed, index), (Element::Label(label), 0));
        assert!(matches!(
            home.remove(id.into()),
            Err(CoreError::NotFound(_))
        ));
    }
}
