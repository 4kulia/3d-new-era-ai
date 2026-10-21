//! Core domain of 3D New Era AI.
//!
//! This crate knows nothing about windows, GPUs or networks. Every client —
//! the desktop UI, the MCP server, the HTTP API — mutates a [`Home`] through
//! the same [`Command`]s executed by a [`Document`], so undo/redo and change
//! tracking behave identically no matter who made the change.
//!
//! All lengths are in **centimeters**, matching architectural drawings.

mod analysis;
mod annotations;
pub mod arrange;
mod ceiling;
pub mod collab;
mod command;
mod detect;
mod document;
pub mod electrical;
mod elements;
mod error;
pub mod finishes;
pub mod front;
mod furniture;
mod geometry;
pub mod guard;
mod home;
mod ids;
pub mod images;
mod joins;
mod layers;
mod levels;
pub mod library;
pub mod lighting;
mod materials;
pub mod measure;
pub mod mounting;
pub mod ops;
pub mod plumbing;
pub mod progress;
mod project;
mod roof_fit;
pub mod routing;
mod solid;
pub mod standards;
mod style;
pub mod takeoff;
pub mod tape;
pub mod text;
mod units;
pub mod vfs;
pub mod vocabulary;
mod wallrun;
pub mod wifi;

pub use analysis::{
    Issue, Overlap, Storeys, check_layout, check_layout_in, door_blocked_by, door_swing,
};
pub use annotations::{
    PlanAnnotations, ReferenceItem, RoomReference, TAG_KEY, auto_dimensions, cleared_references,
    fold, room_references, untagged_references,
};
pub use ceiling::{CeilingTriangle, room_ceiling};
pub use command::Command;
pub use detect::{detect_room, detect_room_with_dividers, interior_point, rooms_following_walls};
pub use document::{Document, FIRST_VARIANT_NAME, SharedDocument, Variant, VariantInfo};
pub use elements::{
    BackgroundImage, Baseboard, Compass, Dimension, Element, Hold, Label, Level, Room, RoomUse,
    Wall,
};
pub use error::{CoreError, CoreResult};
pub use furniture::{
    FarModel, Furniture, Light, LightSource, Measure, ModelMaterial, ModelPart, ModelTransform,
    Opening, OpeningKind, PieceInfo, PieceLocks, Purchase, Reference, Sash, SolidShape, WallCut,
    WallCutOut, align_to_wall, cut_outline, wall_cuts,
};
pub use geometry::{Point2, polygon_area, polygon_centroid, signed_area, to_polygon, triangulate};
pub use home::Home;
pub use ids::{
    DimensionId, ElementId, FurnitureId, LabelId, LevelId, ParseIdError, PolylineId, RoomId, WallId,
};
pub use joins::{JOIN_TOLERANCE, TOUCH_TOLERANCE, wall_outlines, weld_ends};
pub use layers::{LAYER_KEY, PlanLayer, layer_in_group, layer_of};
pub use levels::{FloorShape, floor_shapes, stair_holes};
pub use lighting::{LampType, RoomLighting};
pub use materials::{Material, Pattern, WALL_TYPES, WallFamily, WallType, wall_type};
pub use measure::{
    AnnotationCheck, Axis, Clearance, Dir, LooseEnd, Obstacle, Solid, Span, Stale,
    anchor_dimensions, built_frame, check_annotations, clearance, dimensions_following,
    element_bounds, facing, facing_disagrees, free_span, hold_point, loose_end, obstacles,
    plan_bounds, stale_annotations, wall_bounds,
};
pub use project::{
    BundledFiles, PROJECT_EXTENSION, Project, ProjectError, cache_dir, from_project_json,
    model_companions, open_project, project_from_bytes, resolve_asset, resolve_project_path,
    save_project, to_bundle_bytes, to_project_bytes, to_project_json,
};
pub use roof_fit::{ROOF_FIT_ABOVE, ROOF_FIT_KEY, fit_commands, fit_to_roof, roof_height_at};
pub use standards::{
    Confidence, MunicipalCode, Place, Severity, Standard, Tier, municipal, standard,
};
pub use style::{
    ArrowStyle, Camera, Cameras, DashStyle, Discipline, DrawingMode, Environment, LineCap,
    LineJoin, PaperOrientation, PhotoSettings, Polyline, PrintSettings, Properties, TextAlign,
    TextStyle, VideoSettings,
};
pub use units::LengthUnit;
pub use wallrun::{RunBlock, RunObstacle, WallRun, wall_run};
