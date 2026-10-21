use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::command::Command;
use crate::error::{CoreError, CoreResult};
use crate::home::Home;
use crate::ids::{DimensionId, FurnitureId, LabelId, LevelId, PolylineId, RoomId, WallId};

/// One version of the project, with its own edit history.
#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    home: Home,
    undo_stack: Vec<Command>,
    redo_stack: Vec<Command>,
}

impl Variant {
    fn new(name: impl Into<String>, home: Home) -> Self {
        Self {
            name: name.into(),
            home,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn home(&self) -> &Home {
        &self.home
    }
}

/// Summary of a variant, for tabs and comparisons.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantInfo {
    pub index: usize,
    pub name: String,
    pub active: bool,
}

/// A project: one or more variants (versions of the same plan), each a
/// [`Home`] with its own undo history. Every edit applies to the active one.
///
/// `revision` increases on every successful change (including undo/redo and
/// switching variants), so views can cheaply detect when they need to rebuild
/// derived data such as 3D meshes.
#[derive(Debug)]
pub struct Document {
    variants: Vec<Variant>,
    active: usize,
    revision: u64,
    saved_revision: u64,
    /// Project file this document was loaded from or saved to.
    path: Option<PathBuf>,
    /// Directory relative asset paths resolve against, when it isn't the
    /// project file's directory (unpacked bundles, fresh imports).
    asset_dir: Option<PathBuf>,
    /// People and agents working on it right now (not saved, not undoable).
    sessions: crate::collab::Sessions,
    /// The HTTP API serving this document, when one runs.
    server: Option<crate::collab::ServerInfo>,
    /// AI clients talking MCP to this window (not saved, not undoable).
    agents: crate::collab::Agents,
}

impl Default for Document {
    fn default() -> Self {
        Self::new(Home::default())
    }
}

pub const FIRST_VARIANT_NAME: &str = "Versão 1";

impl Document {
    pub fn new(home: Home) -> Self {
        Self {
            variants: vec![Variant::new(FIRST_VARIANT_NAME, home)],
            active: 0,
            revision: 0,
            saved_revision: 0,
            path: None,
            asset_dir: None,
            sessions: crate::collab::Sessions::default(),
            server: None,
            agents: crate::collab::Agents::default(),
        }
    }

    pub fn sessions(&self) -> &crate::collab::Sessions {
        &self.sessions
    }

    pub fn sessions_mut(&mut self) -> &mut crate::collab::Sessions {
        &mut self.sessions
    }

    /// The AI clients talking MCP to this document, and their last calls.
    pub fn agents(&self) -> &crate::collab::Agents {
        &self.agents
    }

    pub fn agents_mut(&mut self) -> &mut crate::collab::Agents {
        &mut self.agents
    }

    pub fn server(&self) -> Option<&crate::collab::ServerInfo> {
        self.server.as_ref()
    }

    pub fn set_server(&mut self, server: Option<crate::collab::ServerInfo>) {
        self.server = server;
    }

    fn current(&self) -> &Variant {
        &self.variants[self.active]
    }

    fn current_mut(&mut self) -> &mut Variant {
        &mut self.variants[self.active]
    }

    /// The active variant's home.
    pub fn home(&self) -> &Home {
        &self.current().home
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// How many changes can still be undone on the active variant.
    ///
    /// A checkpoint is this number: undoing back down to it puts the plan
    /// exactly where it was, with every id intact — which duplicating the
    /// variant cannot promise.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.current().undo_stack.len()
    }

    /// Allocates a fresh wall id. Ids are never reused, even after undo.
    pub fn new_wall_id(&mut self) -> WallId {
        self.current_mut().home.new_wall_id()
    }

    /// Allocates a fresh room id. Ids are never reused, even after undo.
    pub fn new_room_id(&mut self) -> RoomId {
        self.current_mut().home.new_room_id()
    }

    pub fn new_dimension_id(&mut self) -> DimensionId {
        self.current_mut().home.new_dimension_id()
    }

    pub fn new_label_id(&mut self) -> LabelId {
        self.current_mut().home.new_label_id()
    }

    pub fn new_furniture_id(&mut self) -> FurnitureId {
        self.current_mut().home.new_furniture_id()
    }

    pub fn new_polyline_id(&mut self) -> PolylineId {
        self.current_mut().home.new_polyline_id()
    }

    pub fn new_level_id(&mut self) -> LevelId {
        self.current_mut().home.new_level_id()
    }

    /// The active plan, alone, showing storey `level` at this document's
    /// revision: what a read of another storey works on, leaving the storey
    /// the person is looking at alone.
    #[must_use]
    pub fn showing(&self, level: Option<LevelId>) -> Self {
        let mut home = self.home().clone();
        home.selected_level = level;
        let mut copy = Self::new(home);
        copy.revision = self.revision;
        copy.saved_revision = self.saved_revision;
        copy.path.clone_from(&self.path);
        copy.asset_dir.clone_from(&self.asset_dir);
        copy
    }

    /// Selects the storey shown in the plan and used for new elements. It is
    /// view state: saved with the project but not part of the undo history.
    pub fn select_level(&mut self, level: Option<LevelId>) {
        let home = &mut self.current_mut().home;
        if home.selected_level != level {
            home.selected_level = level;
            self.revision += 1;
        }
    }

    /// Chooses the technical project being edited (`None`: architecture).
    /// View state, like the selected level.
    pub fn set_active_discipline(&mut self, discipline: Option<crate::style::Discipline>) {
        let home = &mut self.current_mut().home;
        if home.active_discipline != discipline {
            home.active_discipline = discipline;
            self.revision += 1;
        }
    }

    /// Switches the view: architecture, or one technical project.
    ///
    /// Architecture shows architecture — choosing it hides the electrical
    /// and plumbing projects, which stay one checkbox away — and a technical
    /// project is shown the moment it is chosen, with the architecture
    /// stepping back behind it. The plan and the 3D follow the same view.
    pub fn choose_view(&mut self, discipline: Option<crate::style::Discipline>) {
        self.set_active_discipline(discipline);
        match discipline {
            None => {
                for d in crate::style::Discipline::ALL {
                    self.set_discipline_visible(d, false);
                }
            }
            Some(d) => self.set_discipline_visible(d, true),
        }
    }

    /// Makes the 3D show everything, or only what the plan shows.
    pub fn set_show_all_in_3d(&mut self, all: bool) {
        let home = &mut self.current_mut().home;
        if home.show_all_in_3d != all {
            home.show_all_in_3d = all;
            self.revision += 1;
        }
    }

    /// Shows or hides a technical project.
    pub fn set_discipline_visible(&mut self, discipline: crate::style::Discipline, visible: bool) {
        let home = &mut self.current_mut().home;
        let hidden = home.hidden_disciplines.contains(&discipline);
        if hidden == visible {
            if visible {
                home.hidden_disciplines.retain(|d| *d != discipline);
            } else {
                home.hidden_disciplines.push(discipline);
            }
            self.revision += 1;
        }
    }

    /// Shows or hides a layer of the plan drawing (lighting, appliances,
    /// joinery). View state: the 3D is not affected.
    pub fn set_layer_visible(&mut self, layer: crate::layers::PlanLayer, visible: bool) {
        let home = &mut self.current_mut().home;
        let hidden = home.hidden_layers.contains(&layer);
        if hidden == visible {
            if visible {
                home.hidden_layers.retain(|l| *l != layer);
            } else {
                home.hidden_layers.push(layer);
            }
            self.revision += 1;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.current().undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.current().redo_stack.is_empty()
    }

    /// Executes a command on the active variant, recording it in its history.
    pub fn execute(&mut self, command: Command) -> CoreResult<()> {
        let variant = self.current_mut();
        let shape = |home: &Home| {
            (
                home.walls.clone(),
                home.polylines
                    .iter()
                    .filter(|p| p.room_divider)
                    .cloned()
                    .collect::<Vec<_>>(),
                home.rooms.iter().filter(|r| r.auto).count(),
            )
        };
        let before = shape(&variant.home);
        let hung = crate::mounting::ceiling_pieces(&variant.home);
        let mut inverse = command.apply(&mut variant.home)?;
        // Rooms detected from walls follow them, in the same undo step.
        if variant.home.rooms.iter().any(|r| r.auto) && shape(&variant.home) != before {
            let mut undo_rooms = Vec::new();
            for room in crate::detect::rooms_following_walls(&variant.home) {
                if let Ok(previous) = Command::update(room).apply(&mut variant.home) {
                    undo_rooms.push(previous);
                }
            }
            if !undo_rooms.is_empty() {
                undo_rooms.reverse();
                undo_rooms.push(inverse);
                inverse = Command::Batch {
                    commands: undo_rooms,
                };
            }
        }
        // Dimensions that hold onto what they mark are measured again, in
        // the same undo step: a plan whose numbers drift is worse than one
        // with no numbers at all.
        if variant.home.dimensions.iter().any(|d| d.holds.is_some()) {
            let mut undo_dims = Vec::new();
            for dim in crate::measure::dimensions_following(&variant.home) {
                if let Ok(previous) = Command::update(dim).apply(&mut variant.home) {
                    undo_dims.push(previous);
                }
            }
            if !undo_dims.is_empty() {
                undo_dims.reverse();
                undo_dims.push(inverse);
                inverse = Command::Batch {
                    commands: undo_dims,
                };
            }
        }
        // Pieces that joined the schedule get the next free reference number,
        // in the same undo step, and keep it from then on.
        let untagged = crate::annotations::untagged_references(&variant.home);
        if !untagged.is_empty() {
            let mut undo_tags = Vec::new();
            for piece in untagged {
                if let Ok(previous) = Command::update(piece).apply(&mut variant.home) {
                    undo_tags.push(previous);
                }
            }
            if !undo_tags.is_empty() {
                undo_tags.reverse();
                undo_tags.push(inverse);
                inverse = Command::Batch {
                    commands: undo_tags,
                };
            }
        }
        // Walls and panels that follow the roof, in the same undo step.
        let follows = |home: &Home| {
            home.walls
                .iter()
                .any(|w| w.properties.contains_key(crate::roof_fit::ROOF_FIT_KEY))
                || home
                    .furniture
                    .iter()
                    .any(|f| f.properties.contains_key(crate::roof_fit::ROOF_FIT_KEY))
        };
        if follows(&variant.home) {
            let snapshot = variant.home.clone();
            let mut ids = variant.home.clone();
            let refit = crate::roof_fit::refit_marked(&snapshot, &mut || ids.new_wall_id());
            if !refit.is_empty() {
                // Keep the id counter in step with the walls just added.
                for _ in refit.iter().filter(|c| matches!(c, Command::Insert { .. })) {
                    variant.home.new_wall_id();
                }
                let mut undo_fit = Vec::new();
                for command in refit {
                    if let Ok(previous) = command.apply(&mut variant.home) {
                        undo_fit.push(previous);
                    }
                }
                if !undo_fit.is_empty() {
                    undo_fit.reverse();
                    undo_fit.push(inverse);
                    inverse = Command::Batch { commands: undo_fit };
                }
            }
        }
        // Spots, panels and pendants stay on the ceiling, in the same undo
        // step: moved into another room, under a storey made taller, or
        // resized, none is left floating under it or poking through it.
        let reseat = crate::mounting::ceilings_following(&variant.home, &hung);
        if !reseat.is_empty() {
            let mut undo_hung = Vec::new();
            for piece in reseat {
                if let Ok(previous) = Command::update(piece).apply(&mut variant.home) {
                    undo_hung.push(previous);
                }
            }
            if !undo_hung.is_empty() {
                undo_hung.reverse();
                undo_hung.push(inverse);
                inverse = Command::Batch {
                    commands: undo_hung,
                };
            }
        }
        variant.undo_stack.push(inverse);
        variant.redo_stack.clear();
        self.revision += 1;
        Ok(())
    }

    pub fn undo(&mut self) -> CoreResult<()> {
        let variant = self.current_mut();
        let command = variant.undo_stack.pop().ok_or(CoreError::NothingToUndo)?;
        let inverse = command.apply(&mut variant.home)?;
        variant.redo_stack.push(inverse);
        self.revision += 1;
        Ok(())
    }

    pub fn redo(&mut self) -> CoreResult<()> {
        let variant = self.current_mut();
        let command = variant.redo_stack.pop().ok_or(CoreError::NothingToRedo)?;
        let inverse = command.apply(&mut variant.home)?;
        variant.undo_stack.push(inverse);
        self.revision += 1;
        Ok(())
    }

    /// Replaces the whole project with a single variant. Counts as saved.
    pub fn load(&mut self, home: Home) {
        self.load_variants(vec![(FIRST_VARIANT_NAME.to_owned(), home)], 0);
    }

    /// Replaces the whole project with these variants. Counts as saved.
    pub fn load_variants(&mut self, variants: Vec<(String, Home)>, active: usize) {
        self.variants = variants
            .into_iter()
            .map(|(name, mut home)| {
                // A project saved with a lamp through its ceiling opens with
                // it hung right: the fit is what every edit keeps anyway.
                for piece in crate::mounting::ceilings_following(&home, &[]) {
                    if let Some(slot) = home.furniture.iter_mut().find(|f| f.id == piece.id) {
                        *slot = piece;
                    }
                }
                Variant::new(name, home)
            })
            .collect();
        if self.variants.is_empty() {
            self.variants
                .push(Variant::new(FIRST_VARIANT_NAME, Home::default()));
        }
        self.active = active.min(self.variants.len() - 1);
        self.revision += 1;
        self.saved_revision = self.revision;
    }

    // --- Variants ---------------------------------------------------------------

    pub fn variants(&self) -> impl Iterator<Item = &Variant> {
        self.variants.iter()
    }

    pub fn variant_infos(&self) -> Vec<VariantInfo> {
        self.variants
            .iter()
            .enumerate()
            .map(|(index, v)| VariantInfo {
                index,
                name: v.name.clone(),
                active: index == self.active,
            })
            .collect()
    }

    pub fn active_variant(&self) -> usize {
        self.active
    }

    pub fn variant_count(&self) -> usize {
        self.variants.len()
    }

    /// Adds a variant after the active one and switches to it. With
    /// `duplicate`, it starts as a copy of the active home; otherwise empty.
    /// Returns its index.
    pub fn add_variant(&mut self, name: Option<String>, duplicate: bool) -> usize {
        let home = if duplicate {
            self.home().clone()
        } else {
            let mut home = Home::new(self.home().name.clone());
            home.compass = self.home().compass.clone();
            home
        };
        let name = name.unwrap_or_else(|| self.next_variant_name(duplicate));
        let index = self.active + 1;
        self.variants.insert(index, Variant::new(name, home));
        self.active = index;
        self.revision += 1;
        index
    }

    fn next_variant_name(&self, duplicate: bool) -> String {
        if duplicate {
            let base = format!("{} (cópia)", self.current().name);
            let mut name = base.clone();
            let mut n = 2;
            while self.variants.iter().any(|v| v.name == name) {
                name = format!("{base} {n}");
                n += 1;
            }
            name
        } else {
            let mut n = self.variants.len() + 1;
            loop {
                let name = format!("Versão {n}");
                if !self.variants.iter().any(|v| v.name == name) {
                    return name;
                }
                n += 1;
            }
        }
    }

    pub fn switch_variant(&mut self, index: usize) -> CoreResult<()> {
        if index >= self.variants.len() {
            return Err(CoreError::NoSuchVariant(index));
        }
        if index != self.active {
            self.active = index;
            self.revision += 1;
        }
        Ok(())
    }

    pub fn rename_variant(&mut self, index: usize, name: impl Into<String>) -> CoreResult<()> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(CoreError::InvalidGeometry(
                "variant name must not be empty".into(),
            ));
        }
        let variant = self
            .variants
            .get_mut(index)
            .ok_or(CoreError::NoSuchVariant(index))?;
        variant.name = name;
        self.revision += 1;
        Ok(())
    }

    /// Removes a variant; the last one can't be removed.
    pub fn remove_variant(&mut self, index: usize) -> CoreResult<()> {
        if index >= self.variants.len() {
            return Err(CoreError::NoSuchVariant(index));
        }
        if self.variants.len() == 1 {
            return Err(CoreError::InvalidGeometry(
                "a project needs at least one variant".into(),
            ));
        }
        self.variants.remove(index);
        if self.active > index || self.active >= self.variants.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.revision += 1;
        Ok(())
    }

    // --- Files ------------------------------------------------------------------

    /// Marks the current state as saved to `path`.
    pub fn mark_saved(&mut self, path: impl Into<PathBuf>) {
        self.saved_revision = self.revision;
        self.path = Some(path.into());
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Directory relative asset paths resolve against.
    pub fn asset_dir(&self) -> Option<PathBuf> {
        self.asset_dir.clone().or_else(|| {
            self.path
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
    }

    pub fn set_asset_dir(&mut self, dir: Option<PathBuf>) {
        self.asset_dir = dir;
    }

    /// Resolves a path stored in the home to a file on disk.
    pub fn resolve_asset(&self, stored: &str) -> PathBuf {
        crate::project::resolve_asset(self.asset_dir().as_deref(), stored)
    }

    /// Sets the project file path (e.g. after opening a file).
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    /// True when there are changes since the last save or load.
    pub fn is_modified(&self) -> bool {
        self.revision != self.saved_revision
    }
}

/// Thread-safe handle shared by the UI thread and the async servers.
#[derive(Debug, Clone, Default)]
pub struct SharedDocument(Arc<RwLock<Document>>);

impl SharedDocument {
    pub fn new(document: Document) -> Self {
        Self(Arc::new(RwLock::new(document)))
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Document> {
        // A panic while holding the lock cannot leave a half-applied command
        // (commands roll back on error), so recovering from poison is safe.
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Document> {
        self.0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Point2, Wall};

    fn with_wall(doc: &mut Document) {
        let wall = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(300.0, 0.0),
        );
        doc.execute(Command::insert(wall)).unwrap();
    }

    #[test]
    fn duplicated_variants_are_independent_with_their_own_history() {
        let mut doc = Document::default();
        with_wall(&mut doc);
        let copy = doc.add_variant(None, true);
        assert_eq!((copy, doc.active_variant()), (1, 1));
        assert_eq!(doc.variant_infos()[1].name, "Versão 1 (cópia)");
        assert_eq!(doc.home().walls.len(), 1, "copy starts equal");
        assert!(!doc.can_undo(), "copy starts with a clean history");

        with_wall(&mut doc);
        assert_eq!(doc.home().walls.len(), 2);
        doc.switch_variant(0).unwrap();
        assert_eq!(doc.home().walls.len(), 1, "original untouched");
        assert!(doc.can_undo(), "original keeps its history");

        let empty = doc.add_variant(None, false);
        assert_eq!(doc.variant_infos()[empty].name, "Versão 3");
        assert_eq!(doc.home().walls, [] as [crate::elements::Wall; 0]);
    }

    #[test]
    fn removing_and_renaming_variants_keeps_a_valid_active_one() {
        let mut doc = Document::default();
        doc.add_variant(Some("B".into()), true);
        doc.add_variant(Some("C".into()), true);
        assert_eq!(doc.active_variant(), 2);
        doc.remove_variant(2).unwrap();
        assert_eq!(doc.active_variant(), 1);
        doc.rename_variant(0, "Térreo").unwrap();
        assert_eq!(doc.variant_infos()[0].name, "Térreo");
        doc.remove_variant(0).unwrap();
        assert_eq!((doc.variant_count(), doc.active_variant()), (1, 0));
        assert!(doc.remove_variant(0).is_err(), "last variant stays");
        assert!(doc.switch_variant(5).is_err());
    }

    #[test]
    fn variant_changes_mark_the_project_modified() {
        let mut doc = Document::default();
        doc.mark_saved("/tmp/x.newera");
        doc.add_variant(None, true);
        assert!(doc.is_modified());
    }

    /// A room 400 × 400 with walls 280 tall and its ceiling at the storey
    /// height, and a pendant and a spot put in it at the catalog's heights.
    fn lit_room() -> (Document, crate::FurnitureId, crate::FurnitureId) {
        use crate::{Furniture, FurnitureId, Point2, Room, RoomId, Wall, WallId};
        let mut doc = Document::default();
        let corners = [(0.0, 0.0), (400.0, 0.0), (400.0, 400.0), (0.0, 400.0)];
        let mut commands: Vec<Command> = (0..4)
            .map(|k| {
                let (a, b) = (corners[k], corners[(k + 1) % 4]);
                let mut w = Wall::new(
                    WallId(k as u64 + 1),
                    Point2::new(a.0, a.1),
                    Point2::new(b.0, b.1),
                );
                w.height = 280.0;
                Command::insert(w)
            })
            .collect();
        commands.push(Command::insert(Room::new(
            RoomId(9),
            "Jantar",
            corners.iter().map(|c| Point2::new(c.0, c.1)).collect(),
        )));
        doc.execute(Command::Batch { commands }).unwrap();
        // As the catalog makes them: 170 + 90 = 260, a spot at 259.4 + 3.
        let (pendant, spot) = (FurnitureId(20), FurnitureId(21));
        doc.execute(Command::Batch {
            commands: vec![
                Command::insert(Furniture {
                    id: pendant,
                    catalog: "pendant".into(),
                    position: Point2::new(200.0, 200.0),
                    width: 35.0,
                    depth: 35.0,
                    height: 90.0,
                    elevation: 170.0,
                    ..Furniture::default()
                }),
                Command::insert(Furniture {
                    id: spot,
                    catalog: "downlight".into(),
                    position: Point2::new(100.0, 100.0),
                    width: 10.0,
                    depth: 10.0,
                    height: 3.0,
                    elevation: 259.4,
                    ..Furniture::default()
                }),
            ],
        })
        .unwrap();
        (doc, pendant, spot)
    }

    /// Bottom and top of a piece, to the tenth of a millimetre.
    fn top(doc: &Document, id: crate::FurnitureId) -> (f64, f64) {
        let f = doc.home().find_piece(id).unwrap();
        let round = |v: f64| (v * 100.0).round() / 100.0;
        (round(f.elevation), round(f.elevation + f.height))
    }

    #[test]
    fn ceiling_pieces_hang_from_the_ceiling_whatever_changes_under_them() {
        let (mut doc, pendant, spot) = lit_room();
        // The ceiling drawn is the storey's, 250: the pendant's 260 went
        // through it. Its shade stays at 170 and its cord is 80.
        assert!((doc.home().wall_height - 250.0).abs() < 1e-9);
        assert_eq!(top(&doc, pendant), (170.0, 250.0));
        assert_eq!(top(&doc, spot), (247.0, 250.0));
        // A taller storey: both go up with it, the shade still at 170.
        let mut home = doc.home().clone();
        home.wall_height = 280.0;
        doc.load(home);
        doc.execute(Command::update(doc.home().rooms[0].clone()))
            .unwrap();
        assert_eq!(top(&doc, pendant), (170.0, 280.0));
        assert_eq!(top(&doc, spot), (277.0, 280.0));
    }

    #[test]
    fn an_edit_to_a_pendant_keeps_what_it_meant_and_undo_puts_it_back() {
        let (mut doc, pendant, _) = lit_room();
        let edit = |doc: &mut Document, f: &dyn Fn(&mut crate::Furniture)| {
            let mut piece = doc.home().find_piece(pendant).unwrap().clone();
            f(&mut piece);
            doc.execute(Command::update(piece)).unwrap();
        };
        // Lowered over a table: the shade goes to 150, the cord grows.
        edit(&mut doc, &|p| p.elevation = 150.0);
        assert_eq!(top(&doc, pendant), (150.0, 250.0));
        // A shorter drop typed alone: 40 cm of pendant, hung from the ceiling.
        edit(&mut doc, &|p| p.height = 40.0);
        assert_eq!(top(&doc, pendant), (210.0, 250.0));
        // Moved: it is still on the ceiling, nothing else changes.
        edit(&mut doc, &|p| p.position = crate::Point2::new(300.0, 300.0));
        assert_eq!(top(&doc, pendant), (210.0, 250.0));
        // One undo is the whole edit, the ceiling fit included.
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert_eq!(top(&doc, pendant), (150.0, 250.0));
    }

    #[test]
    fn a_sloping_ceiling_is_met_under_every_corner_of_a_panel() {
        let (mut doc, _, spot) = lit_room();
        // The room follows the walls, 200 at x=0 up to 400 at x=400.
        let mut home = doc.home().clone();
        for w in &mut home.walls {
            let h = |x: f64| 200.0 + x / 2.0;
            w.height = h(w.start.x);
            w.height_at_end = Some(h(w.end.x));
        }
        home.rooms[0].ceiling_flat = false;
        doc.load(home);
        let mut panel = doc.home().find_piece(spot).unwrap().clone();
        panel.catalog = "led-panel".into();
        (panel.width, panel.depth, panel.height) = (62.0, 62.0, 2.0);
        doc.execute(Command::update(panel)).unwrap();
        // Centered at x=100 (ceiling 250), its low edge at x=69 is 234.5.
        let (_, t) = top(&doc, spot);
        assert!((t - 234.5).abs() < 0.01, "{t}");
    }
}
