use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::elements::{BackgroundImage, Compass, Element};
use crate::error::CoreResult;
use crate::home::Home;
use crate::ids::ElementId;

/// A reversible change to a [`Home`].
///
/// Applying a command returns its inverse, which is what makes undo/redo work
/// for every client. Commands are serializable so they can travel over MCP or
/// HTTP, or be recorded as macros.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    /// Adds a new element.
    Insert {
        element: Element,
        /// Position within its collection; used internally to restore order on undo.
        #[serde(default, skip)]
        index: Option<usize>,
    },
    /// Replaces an existing element that has the same id.
    Update {
        element: Element,
    },
    /// Deletes an element.
    Remove {
        id: ElementId,
    },
    RenameHome {
        name: String,
    },
    SetCompass {
        compass: Compass,
    },
    SetBackground {
        background: Option<BackgroundImage>,
    },
    /// Aerial/visitor cameras and stored points of view.
    SetCameras {
        cameras: crate::style::Cameras,
    },
    /// Derived annotations shown on the plan (dimension chains, references).
    SetAnnotations {
        annotations: crate::annotations::PlanAnnotations,
    },
    /// Findings already looked at and accepted, by key, with the reason.
    SetAccepted {
        accepted: std::collections::BTreeMap<String, String>,
    },
    /// The home's own key/value notes, checkpoints among them.
    SetProperties {
        properties: crate::style::Properties,
    },
    /// Look of the 3D world (sky, ground, light, photo settings).
    SetEnvironment {
        environment: crate::style::Environment,
    },
    /// Puts a product on a surface, replacing the one on the same surface
    /// and band.
    AssignFinish {
        assignment: crate::finishes::FinishAssignment,
        /// Position in the list; used internally to restore order on undo.
        #[serde(default, skip)]
        index: Option<usize>,
    },
    /// Takes the product off a surface band (`None`: the whole surface).
    RemoveFinish {
        surface: crate::finishes::Surface,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        band: Option<crate::finishes::Band>,
    },
    /// Several commands applied atomically: either all succeed or none does.
    Batch {
        commands: Vec<Command>,
    },
}

impl Command {
    pub fn insert(element: impl Into<Element>) -> Self {
        Self::Insert {
            element: element.into(),
            index: None,
        }
    }

    pub fn update(element: impl Into<Element>) -> Self {
        Self::Update {
            element: element.into(),
        }
    }

    pub fn remove(id: impl Into<ElementId>) -> Self {
        Self::Remove { id: id.into() }
    }

    /// Applies the command and returns the command that reverts it.
    ///
    /// On error the home is left untouched.
    pub(crate) fn apply(self, home: &mut Home) -> CoreResult<Self> {
        match self {
            Self::Insert { mut element, index } => {
                let id = element.id();
                // Elements without a storey land on the one being edited.
                if element.level().is_none()
                    && let Some(selected) = home
                        .selected_level
                        .filter(|l| Some(*l) != home.base_level())
                {
                    element.set_level(Some(selected));
                }
                // Symbols and lines drawn while editing a technical project join it.
                if element.takes_discipline() && element.discipline().is_none() {
                    element.set_discipline(home.active_discipline);
                }
                home.insert(element, index)?;
                Ok(Self::Remove { id })
            }
            Self::Update { mut element } => {
                // A group whose box changed but whose pieces weren't edited
                // carries its pieces along.
                if let crate::Element::Furniture(new) = &mut element
                    && new.is_group()
                    && let Some(old) = home.piece(new.id)
                    && old.children == new.children
                {
                    let old = old.clone();
                    new.follow_group_change(&old);
                }
                // A part of a group is replaced where it lives, inside its
                // group: the REST API reaches parts the way MCP does, so a
                // batch of renames need not split into two paths.
                if let crate::Element::Furniture(part) = &element
                    && home.piece(part.id).is_none()
                    && home.part_owner(part.id).is_some()
                {
                    element.validate()?;
                    let crate::Element::Furniture(part) = element else {
                        unreachable!("matched above")
                    };
                    let slot = home
                        .furniture
                        .iter_mut()
                        .find_map(|top| top.find_part_mut(part.id))
                        .ok_or(crate::CoreError::NotFound(part.id.into()))?;
                    let previous = std::mem::replace(slot, part);
                    return Ok(Self::Update {
                        element: crate::Element::Furniture(previous),
                    });
                }
                let previous = home.replace(element)?;
                Ok(Self::Update { element: previous })
            }
            Self::Remove { id } => {
                let (element, index) = home.remove(id)?;
                Ok(Self::Insert {
                    element,
                    index: Some(index),
                })
            }
            Self::RenameHome { name } => {
                let previous = std::mem::replace(&mut home.name, name);
                Ok(Self::RenameHome { name: previous })
            }
            Self::SetCompass { compass } => {
                compass.validate()?;
                let previous = std::mem::replace(&mut home.compass, compass);
                Ok(Self::SetCompass { compass: previous })
            }
            Self::SetBackground { background } => {
                if let Some(bg) = &background {
                    bg.validate()?;
                }
                let previous = std::mem::replace(&mut home.background, background);
                Ok(Self::SetBackground {
                    background: previous,
                })
            }
            Self::SetAnnotations { annotations } => {
                let previous = std::mem::replace(&mut home.annotations, annotations);
                Ok(Self::SetAnnotations {
                    annotations: previous,
                })
            }
            Self::SetAccepted { accepted } => {
                let previous = std::mem::replace(&mut home.accepted, accepted);
                Ok(Self::SetAccepted { accepted: previous })
            }
            Self::SetProperties { properties } => {
                let previous = std::mem::replace(&mut home.properties, properties);
                Ok(Self::SetProperties {
                    properties: previous,
                })
            }
            Self::SetCameras { cameras } => {
                let previous = std::mem::replace(&mut home.cameras, cameras);
                Ok(Self::SetCameras { cameras: previous })
            }
            Self::SetEnvironment { environment } => {
                let previous = std::mem::replace(&mut home.environment, environment);
                Ok(Self::SetEnvironment {
                    environment: previous,
                })
            }
            Self::AssignFinish { assignment, index } => {
                crate::finishes::assign(home, assignment, index)
            }
            Self::RemoveFinish { surface, band } => crate::finishes::remove(home, surface, band),
            Self::Batch { commands } => {
                let mut inverses = Vec::with_capacity(commands.len());
                for command in commands {
                    match command.apply(home) {
                        Ok(inverse) => inverses.push(inverse),
                        Err(err) => {
                            // Roll back what was already applied, newest first.
                            for inverse in inverses.into_iter().rev() {
                                inverse
                                    .apply(home)
                                    .expect("inverse of an applied command must apply");
                            }
                            return Err(err);
                        }
                    }
                }
                inverses.reverse();
                Ok(Self::Batch { commands: inverses })
            }
        }
    }
}
