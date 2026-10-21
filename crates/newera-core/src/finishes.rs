//! Which shop product covers which surface: a room's floor or ceiling, or
//! one side of a wall, whole or in a height band (tile below, paint above).
//!
//! The project keeps only the reference — the hosted service's product id —
//! and what the quantities need besides the geometry: the band, the coats
//! of paint and the waste. Prices and packs stay with the products, on the
//! server. The look of the surface is its [`crate::Material`], set beside.

use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{CoreError, CoreResult};
use crate::home::Home;
use crate::ids::{RoomId, WallId};

/// One side of a wall, as seen walking from its start to its end on the
/// plan — the same sides as [`crate::Wall::left_side`] and `right_side`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// The other side: what a mirror turns this one into.
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// A surface a finish covers, written `r3:floor`, `r3:ceiling`, `w5:left`
/// or `w5:right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Surface {
    Floor(RoomId),
    Ceiling(RoomId),
    Wall(WallId, Side),
}

impl Surface {
    /// The element the surface belongs to.
    #[must_use]
    pub fn element(self) -> crate::ElementId {
        match self {
            Self::Floor(room) | Self::Ceiling(room) => room.into(),
            Self::Wall(wall, _) => wall.into(),
        }
    }

    /// Whether the surface is still in the home.
    #[must_use]
    pub fn exists(self, home: &Home) -> bool {
        home.contains(self.element())
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Floor(room) => write!(f, "{room}:floor"),
            Self::Ceiling(room) => write!(f, "{room}:ceiling"),
            Self::Wall(wall, Side::Left) => write!(f, "{wall}:left"),
            Self::Wall(wall, Side::Right) => write!(f, "{wall}:right"),
        }
    }
}

/// Why a surface did not read, with the forms that do.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("surface `{0}`: r3:floor, r3:ceiling, w5:left or w5:right")]
pub struct ParseSurfaceError(String);

impl FromStr for Surface {
    type Err = ParseSurfaceError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let wrong = || ParseSurfaceError(raw.to_owned());
        let (element, face) = raw.trim().split_once(':').ok_or_else(wrong)?;
        match face {
            "floor" => Ok(Self::Floor(element.parse().map_err(|_| wrong())?)),
            "ceiling" => Ok(Self::Ceiling(element.parse().map_err(|_| wrong())?)),
            "left" => Ok(Self::Wall(
                element.parse().map_err(|_| wrong())?,
                Side::Left,
            )),
            "right" => Ok(Self::Wall(
                element.parse().map_err(|_| wrong())?,
                Side::Right,
            )),
            _ => Err(wrong()),
        }
    }
}

impl Serialize for Surface {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Surface {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = std::borrow::Cow::<str>::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Surface {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Surface".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^(r[0-9]+:(floor|ceiling)|w[0-9]+:(left|right))$",
            "description": "A room's floor or ceiling, or a wall side: r3:floor, w5:left",
        })
    }
}

/// A height band on a wall side, cm above its storey's floor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Band {
    pub from: f64,
    pub to: f64,
}

impl Band {
    /// Whether two bands share some height. `None` is the whole side, which
    /// shares height with every band.
    #[must_use]
    pub fn overlaps(a: Option<Self>, b: Option<Self>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => a.from < b.to && b.from < a.to,
            _ => true,
        }
    }
}

/// A product put on a surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FinishAssignment {
    pub surface: Surface,
    /// The hosted service's product id.
    pub product: String,
    /// Only this height of a wall side; `None` covers all of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<Band>,
    /// Coats of paint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coats: Option<u32>,
    /// Extra bought for cuts and breakage, percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waste: Option<f64>,
}

fn invalid(why: impl Into<String>) -> CoreError {
    CoreError::InvalidFinish(why.into())
}

impl FinishAssignment {
    /// Whether `self` is the assignment found at `surface` and `band`.
    #[must_use]
    pub fn is_at(&self, surface: Surface, band: Option<Band>) -> bool {
        self.surface == surface && self.band == band
    }

    /// Refuses an assignment that cannot stand in `home`: a surface that is
    /// not there, no product, a band off a wall or upside down, no coats, a
    /// waste that is no percentage.
    ///
    /// # Errors
    /// The first thing wrong, in words.
    pub fn validate(&self, home: &Home) -> CoreResult<()> {
        if !self.surface.exists(home) {
            return Err(CoreError::NotFound(self.surface.element()));
        }
        if self.product.trim().is_empty() {
            return Err(invalid("a finish needs a product id"));
        }
        if let Some(band) = self.band {
            if !matches!(self.surface, Surface::Wall(..)) {
                return Err(invalid(format!(
                    "a band is a height on a wall side, and {} is not one",
                    self.surface
                )));
            }
            if !(band.from.is_finite() && band.to.is_finite() && band.from >= 0.0) {
                return Err(invalid("band: from and to, cm from 0 up"));
            }
            if band.to <= band.from {
                return Err(invalid(format!(
                    "band: to ({}) must be above from ({})",
                    band.to, band.from
                )));
            }
        }
        if self.coats == Some(0) {
            return Err(invalid("coats: 1 or more"));
        }
        if let Some(waste) = self.waste
            && !(waste.is_finite() && (0.0..100.0).contains(&waste))
        {
            return Err(invalid(format!(
                "waste: a percentage from 0 to below 100 (not {waste})"
            )));
        }
        Ok(())
    }
}

/// The products on wall `wall`'s sides, as they are now: what a copy of
/// the wall keeps, to put on the copy with [`onto`] later.
#[must_use]
pub fn of_wall(home: &Home, wall: WallId) -> Vec<FinishAssignment> {
    home.finish_assignments
        .iter()
        .filter(|a| matches!(a.surface, Surface::Wall(w, _) if w == wall))
        .cloned()
        .collect()
}

/// The commands that put `kept` (a wall's products, from [`of_wall`]) on
/// wall `to`, side for side — or each on the other side, with `flip`, for a
/// mirrored copy. Products on other surfaces are left out.
#[must_use]
pub fn onto(kept: &[FinishAssignment], to: WallId, flip: bool) -> Vec<crate::Command> {
    kept.iter()
        .filter_map(|a| match a.surface {
            Surface::Wall(_, side) => Some(crate::Command::AssignFinish {
                assignment: FinishAssignment {
                    surface: Surface::Wall(to, if flip { side.other() } else { side }),
                    ..a.clone()
                },
                index: None,
            }),
            _ => None,
        })
        .collect()
}

/// The commands that give wall `to` the products wall `from` has now.
#[must_use]
pub fn copied(home: &Home, from: WallId, to: WallId, flip: bool) -> Vec<crate::Command> {
    onto(&of_wall(home, from), to, flip)
}

/// Puts `assignment` in `home` at `index` (the end when `None`), replacing
/// the one on the same surface and band. Returns the command that undoes it.
pub(crate) fn assign(
    home: &mut Home,
    assignment: FinishAssignment,
    index: Option<usize>,
) -> CoreResult<crate::Command> {
    assignment.validate(home)?;
    let (surface, band) = (assignment.surface, assignment.band);
    if let Some(other) = home
        .finish_assignments
        .iter()
        .find(|a| a.surface == surface && a.band != band && Band::overlaps(a.band, band))
    {
        return Err(invalid(format!(
            "{surface} already has {} on {}: remove it first, or give a band that does not overlap",
            other.product,
            other.band.map_or_else(
                || "the whole side".to_owned(),
                |b| format!("{}–{} cm", b.from, b.to)
            ),
        )));
    }
    if let Some(at) = home
        .finish_assignments
        .iter()
        .position(|a| a.is_at(surface, band))
    {
        let previous = std::mem::replace(&mut home.finish_assignments[at], assignment);
        return Ok(crate::Command::AssignFinish {
            assignment: previous,
            index: Some(at),
        });
    }
    let at = index
        .unwrap_or(home.finish_assignments.len())
        .min(home.finish_assignments.len());
    home.finish_assignments.insert(at, assignment);
    Ok(crate::Command::RemoveFinish { surface, band })
}

/// Takes the assignment at `surface` and `band` out of `home`. Returns the
/// command that puts it back where it was.
pub(crate) fn remove(
    home: &mut Home,
    surface: Surface,
    band: Option<Band>,
) -> CoreResult<crate::Command> {
    let at = home
        .finish_assignments
        .iter()
        .position(|a| a.is_at(surface, band))
        .ok_or_else(|| {
            invalid(format!(
                "{surface} has no finish{}",
                band.map_or_else(String::new, |b| format!(" on {}–{} cm", b.from, b.to))
            ))
        })?;
    let assignment = home.finish_assignments.remove(at);
    Ok(crate::Command::AssignFinish {
        assignment,
        index: Some(at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Document, Point2, Room, Wall};

    fn home() -> Document {
        let mut doc = Document::default();
        let wall = Wall::new(
            doc.new_wall_id(),
            Point2::new(0.0, 0.0),
            Point2::new(400.0, 0.0),
        );
        let room = Room::new(
            doc.new_room_id(),
            "Sala",
            vec![
                Point2::new(0.0, 0.0),
                Point2::new(400.0, 0.0),
                Point2::new(400.0, 300.0),
                Point2::new(0.0, 300.0),
            ],
        );
        doc.execute(Command::Batch {
            commands: vec![Command::insert(wall), Command::insert(room)],
        })
        .unwrap();
        doc
    }

    fn tile(surface: &str, band: Option<(f64, f64)>) -> FinishAssignment {
        FinishAssignment {
            surface: surface.parse().unwrap(),
            product: "p-tile".into(),
            band: band.map(|(from, to)| Band { from, to }),
            coats: None,
            waste: Some(10.0),
        }
    }

    #[test]
    fn surfaces_read_and_write_as_their_short_form() {
        for raw in ["r3:floor", "r3:ceiling", "w5:left", "w5:right"] {
            let surface: Surface = raw.parse().unwrap();
            assert_eq!(surface.to_string(), raw);
            assert_eq!(serde_json::to_value(surface).unwrap(), raw);
        }
        assert_eq!(
            "w5:floor".parse::<Surface>().unwrap_err().to_string(),
            "surface `w5:floor`: r3:floor, r3:ceiling, w5:left or w5:right"
        );
        assert!("r3".parse::<Surface>().is_err());
        assert!("w5:top".parse::<Surface>().is_err());
    }

    #[test]
    fn an_assignment_is_added_replaced_and_removed_and_each_undoes() {
        let mut doc = home();
        let empty = doc.home().clone();
        doc.execute(Command::AssignFinish {
            assignment: tile("w1:left", Some((0.0, 120.0))),
            index: None,
        })
        .unwrap();
        let tiled = doc.home().clone();
        assert_eq!(
            tiled.finish_assignments,
            vec![tile("w1:left", Some((0.0, 120.0)))]
        );

        let paint = FinishAssignment {
            surface: "w1:left".parse().unwrap(),
            product: "p-paint".into(),
            band: Some(Band {
                from: 120.0,
                to: 250.0,
            }),
            coats: Some(2),
            waste: None,
        };
        doc.execute(Command::AssignFinish {
            assignment: paint.clone(),
            index: None,
        })
        .unwrap();
        let both = doc.home().clone();
        assert_eq!(
            both.finish_assignments,
            vec![tile("w1:left", Some((0.0, 120.0))), paint.clone()]
        );

        // The same surface and band again replaces it.
        let mut other_tile = tile("w1:left", Some((0.0, 120.0)));
        other_tile.product = "p-tile-2".into();
        doc.execute(Command::AssignFinish {
            assignment: other_tile.clone(),
            index: None,
        })
        .unwrap();
        assert_eq!(
            doc.home().finish_assignments,
            vec![other_tile.clone(), paint.clone()]
        );

        doc.execute(Command::RemoveFinish {
            surface: "w1:left".parse().unwrap(),
            band: Some(Band {
                from: 0.0,
                to: 120.0,
            }),
        })
        .unwrap();
        assert_eq!(doc.home().finish_assignments, vec![paint]);

        doc.undo().unwrap();
        assert_eq!(
            doc.home().finish_assignments,
            vec![other_tile.clone(), both.finish_assignments[1].clone()],
            "the removed one comes back where it was"
        );
        doc.undo().unwrap();
        assert_eq!(doc.home(), &both);
        doc.undo().unwrap();
        assert_eq!(doc.home(), &tiled);
        doc.undo().unwrap();
        assert_eq!(doc.home(), &empty);
        doc.redo().unwrap();
        assert_eq!(doc.home(), &tiled);
        doc.redo().unwrap();
        assert_eq!(doc.home(), &both);
        doc.redo().unwrap();
        doc.redo().unwrap();
        assert_eq!(
            doc.home().finish_assignments,
            vec![both.finish_assignments[1].clone()]
        );
    }

    #[test]
    fn a_finish_batched_with_its_look_is_one_undo_step() {
        let mut doc = home();
        let before = doc.home().clone();
        let mut room = doc.home().rooms[0].clone();
        room.floor_material = Some(crate::Material::paint([200, 180, 150]));
        doc.execute(Command::Batch {
            commands: vec![
                Command::AssignFinish {
                    assignment: tile("r2:floor", None),
                    index: None,
                },
                Command::update(room),
            ],
        })
        .unwrap();
        assert_eq!(doc.home().finish_assignments.len(), 1);
        assert!(doc.home().rooms[0].floor_material.is_some());
        doc.undo().unwrap();
        assert_eq!(doc.home(), &before);
    }

    #[test]
    fn what_cannot_stand_is_refused_and_changes_nothing() {
        let mut doc = home();
        let before = doc.home().clone();
        let cases = [
            (tile("w9:left", None), "w9 not found".to_owned()),
            (
                FinishAssignment {
                    product: " ".into(),
                    ..tile("w1:left", None)
                },
                "a finish needs a product id".to_owned(),
            ),
            (
                tile("r2:floor", Some((0.0, 100.0))),
                "a band is a height on a wall side, and r2:floor is not one".to_owned(),
            ),
            (
                tile("w1:left", Some((120.0, 100.0))),
                "band: to (100) must be above from (120)".to_owned(),
            ),
            (
                tile("w1:left", Some((-5.0, 100.0))),
                "band: from and to, cm from 0 up".to_owned(),
            ),
            (
                FinishAssignment {
                    coats: Some(0),
                    ..tile("w1:left", None)
                },
                "coats: 1 or more".to_owned(),
            ),
            (
                FinishAssignment {
                    waste: Some(100.0),
                    ..tile("w1:left", None)
                },
                "waste: a percentage from 0 to below 100 (not 100)".to_owned(),
            ),
        ];
        for (assignment, why) in cases {
            let err = doc
                .execute(Command::AssignFinish {
                    assignment,
                    index: None,
                })
                .unwrap_err();
            assert_eq!(err.to_string(), why);
        }
        doc.execute(Command::AssignFinish {
            assignment: tile("w1:left", Some((0.0, 120.0))),
            index: None,
        })
        .unwrap();
        let tiled = doc.home().clone();
        let err = doc
            .execute(Command::AssignFinish {
                assignment: tile("w1:left", Some((100.0, 200.0))),
                index: None,
            })
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "w1:left already has p-tile on 0–120 cm: remove it first, or give a band that does not overlap"
        );
        let err = doc
            .execute(Command::AssignFinish {
                assignment: tile("w1:left", None),
                index: None,
            })
            .unwrap_err();
        assert!(err.to_string().starts_with("w1:left already has p-tile"));
        let err = doc
            .execute(Command::RemoveFinish {
                surface: "w1:right".parse().unwrap(),
                band: None,
            })
            .unwrap_err();
        assert_eq!(err.to_string(), "w1:right has no finish");
        assert_eq!(doc.home(), &tiled);
        doc.undo().unwrap();
        assert_eq!(doc.home(), &before);
    }

    #[test]
    fn assignments_survive_the_project_file_and_old_files_have_none() {
        let mut doc = home();
        doc.execute(Command::AssignFinish {
            assignment: FinishAssignment {
                coats: Some(2),
                ..tile("w1:right", Some((0.0, 120.5)))
            },
            index: None,
        })
        .unwrap();
        let json = crate::to_project_json(&doc);
        assert!(
            json.contains(r#""finish_assignments":[{"surface":"w1:right","product":"p-tile","band":{"from":0.0,"to":120.5},"coats":2,"waste":10.0}]"#),
            "{json}"
        );
        assert!(json.contains("\"version\":2"), "no version bump");
        let mut loaded = Document::default();
        crate::from_project_json(&json)
            .unwrap()
            .load_into(&mut loaded);
        assert_eq!(loaded.home(), doc.home());

        let old = r#"{"format":"3d-new-era-ai","version":2,"variants":[{"name":"A","home":{"name":"Antiga"}}],"active":0}"#;
        let project = crate::from_project_json(old).unwrap();
        assert_eq!(project.variants[0].1.finish_assignments, vec![]);
        let mut plain = Document::default();
        project.load_into(&mut plain);
        assert!(!crate::to_project_json(&plain).contains("finish_assignments"));
    }

    #[test]
    fn a_storey_view_keeps_the_finishes_of_its_own_surfaces() {
        let mut home = Home::default();
        for (id, elevation) in [(1, 0.0), (2, 262.0)] {
            home.levels.push(crate::Level {
                id: crate::LevelId(id),
                elevation,
                ..crate::Level::default()
            });
        }
        for (id, level) in [(3, 1), (4, 2)] {
            let mut wall = Wall::new(
                crate::WallId(id),
                Point2::new(0.0, 0.0),
                Point2::new(400.0, 0.0),
            );
            wall.level = Some(crate::LevelId(level));
            home.walls.push(wall);
        }
        home.finish_assignments = vec![
            tile("w3:left", None),
            tile("w4:left", None),
            tile("w9:left", None),
        ];
        assert_eq!(
            home.level_view(Some(crate::LevelId(2))).finish_assignments,
            vec![tile("w4:left", None)]
        );
        assert_eq!(
            home.level_view(None).finish_assignments,
            vec![tile("w3:left", None)]
        );
    }

    fn net(doc: &Document) -> f64 {
        let taken = crate::takeoff::takeoff(doc.home(), crate::takeoff::Only::default());
        (taken.finishes.iter().map(|p| p.gross).sum::<f64>() * 1e6).round() / 1e6
    }

    #[test]
    fn both_halves_of_a_split_wall_keep_its_finishes() {
        let mut doc = home();
        let paint = FinishAssignment {
            surface: "w1:right".parse().unwrap(),
            product: "p-paint".into(),
            band: None,
            coats: Some(2),
            waste: None,
        };
        for assignment in [tile("w1:left", Some((0.0, 120.0))), paint.clone()] {
            doc.execute(Command::AssignFinish {
                assignment,
                index: None,
            })
            .unwrap();
        }
        let before = doc.home().clone();
        let area = net(&doc);
        let half = crate::ops::split_wall(&mut doc, crate::WallId(1), 0.5).unwrap();
        assert_eq!(half, crate::WallId(3));
        assert_eq!(
            doc.home().finish_assignments,
            vec![
                tile("w1:left", Some((0.0, 120.0))),
                paint.clone(),
                tile("w3:left", Some((0.0, 120.0))),
                FinishAssignment {
                    surface: "w3:right".parse().unwrap(),
                    ..paint
                },
            ]
        );
        assert_eq!(net(&doc), area, "the halves cover what the wall did");
        doc.undo().unwrap();
        // Ids are never handed out twice, so only the counter stays moved.
        assert_eq!(
            (&doc.home().walls, &doc.home().finish_assignments),
            (&before.walls, &before.finish_assignments)
        );
    }

    #[test]
    fn a_copied_wall_is_finished_as_the_one_it_copies() {
        let mut doc = home();
        doc.execute(Command::AssignFinish {
            assignment: tile("w1:left", Some((0.0, 120.0))),
            index: None,
        })
        .unwrap();
        let before = doc.home().clone();
        let copies = crate::arrange::apply(
            &mut doc,
            &[crate::WallId(1).into()],
            crate::arrange::Transform::Translate {
                dx: 0.0,
                dy: 500.0,
                dz: 0.0,
            },
            true,
        )
        .unwrap();
        assert_eq!(copies, vec![crate::WallId(3).into()]);
        let mirrored = crate::arrange::apply(
            &mut doc,
            &[crate::WallId(1).into()],
            crate::arrange::Transform::Mirror {
                a: Point2::new(0.0, -50.0),
                b: Point2::new(400.0, -50.0),
            },
            true,
        )
        .unwrap();
        assert_eq!(mirrored, vec![crate::WallId(4).into()]);
        assert_eq!(
            doc.home().finish_assignments,
            vec![
                tile("w1:left", Some((0.0, 120.0))),
                tile("w3:left", Some((0.0, 120.0))),
                tile("w4:right", Some((0.0, 120.0))),
            ]
        );
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert_eq!(
            (&doc.home().walls, &doc.home().finish_assignments),
            (&before.walls, &before.finish_assignments)
        );
    }

    #[test]
    fn a_wall_s_finishes_are_kept_apart_and_put_on_another_wall() {
        let mut doc = home();
        for assignment in [tile("w1:left", Some((0.0, 120.0))), tile("r2:floor", None)] {
            doc.execute(Command::AssignFinish {
                assignment,
                index: None,
            })
            .unwrap();
        }
        let kept = of_wall(doc.home(), crate::WallId(1));
        assert_eq!(kept, vec![tile("w1:left", Some((0.0, 120.0)))]);
        // The wall they were taken from may be gone by the time they land.
        doc.execute(Command::remove(crate::WallId(1))).unwrap();
        assert_eq!(
            onto(&kept, crate::WallId(7), false),
            vec![Command::AssignFinish {
                assignment: tile("w7:left", Some((0.0, 120.0))),
                index: None,
            }]
        );
        assert_eq!(
            onto(&kept, crate::WallId(7), true),
            vec![Command::AssignFinish {
                assignment: tile("w7:right", Some((0.0, 120.0))),
                index: None,
            }]
        );
    }

    #[test]
    fn a_mirror_moves_a_wall_finish_to_the_side_it_lands_on() {
        let mut doc = home();
        doc.execute(Command::AssignFinish {
            assignment: tile("w1:left", Some((0.0, 120.0))),
            index: None,
        })
        .unwrap();
        let mut look = doc.home().walls[0].clone();
        look.left_side = Some(crate::Material::paint([10, 20, 30]));
        doc.execute(Command::update(look)).unwrap();
        let before = doc.home().clone();
        crate::arrange::apply(
            &mut doc,
            &[crate::WallId(1).into()],
            crate::arrange::Transform::Mirror {
                a: Point2::new(0.0, 50.0),
                b: Point2::new(400.0, 50.0),
            },
            false,
        )
        .unwrap();
        let wall = &doc.home().walls[0];
        assert!(wall.right_side.is_some() && wall.left_side.is_none());
        assert_eq!(
            doc.home().finish_assignments,
            vec![tile("w1:right", Some((0.0, 120.0)))]
        );
        doc.undo().unwrap();
        assert_eq!(doc.home(), &before);
    }
}
