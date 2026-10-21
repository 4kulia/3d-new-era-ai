//! Modal dialogs: element properties, background calibration, home settings.

use eframe::egui::{self, DragValue, RichText};
use egui_phosphor::regular as icon;
use newera_core::{
    BackgroundImage, Command, Compass, Dimension, Element, Furniture, Label, Material, Pattern,
    Point2, Room, WALL_TYPES, Wall, WallId,
};

use crate::app::{NewEraApp, Pending};

#[derive(Debug, Clone)]
pub(crate) enum Dialog {
    ModifyWalls {
        ids: Vec<WallId>,
        /// Only for a single wall: editable endpoints.
        points: Option<(Point2, Point2)>,
        thickness: f64,
        height: f64,
        arc: f64,
        finish: WallFinish,
    },
    ModifyRoom(Room),
    ModifyDimension(Dimension),
    ModifyLabel(Label),
    ModifyFurniture {
        piece: Furniture,
        keep_ratio: bool,
    },
    NewLabel {
        at: Point2,
        text: String,
    },
    Calibrate {
        a: Point2,
        b: Point2,
        distance: f64,
    },
    Background(BackgroundImage),
    HomeSettings {
        name: String,
        compass: Compass,
    },
    ConfirmDiscard(Pending),
    ConfirmCloseVariant {
        index: usize,
        name: String,
    },
    Compare(Vec<crate::tabs::VariantStats>),
    ModifyLevel(newera_core::Level),
    ModifyPolyline(newera_core::Polyline),
    Quantities(Vec<crate::tabs::QuantityRow>),
    /// The load schedule and what NBR 5410 finds.
    Electrical,
    ConfirmDeleteLevel {
        id: newera_core::LevelId,
        name: String,
    },
    Help,
    /// How to point an AI at this window, and what it is doing right now.
    ConnectAi {
        /// Which client's snippet is on show.
        client: usize,
    },
    /// The account's projects on the service, to open one.
    #[cfg(target_arch = "wasm32")]
    ServerProjects,
    /// The name of a new project on the service.
    #[cfg(target_arch = "wasm32")]
    NewServerProject {
        name: String,
    },
}

/// Type and side finishes being edited for one or more walls. Values start
/// from the first wall; only fields the user touched are applied to all.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct WallFinish {
    pub(crate) wall_type: Option<String>,
    pub(crate) left: Option<Material>,
    pub(crate) right: Option<Material>,
    pub(crate) type_changed: bool,
    pub(crate) left_changed: bool,
    pub(crate) right_changed: bool,
}

impl WallFinish {
    fn apply(&self, wall: &mut Wall) {
        if self.type_changed {
            wall.wall_type.clone_from(&self.wall_type);
        }
        if self.left_changed {
            wall.left_side.clone_from(&self.left);
        }
        if self.right_changed {
            wall.right_side.clone_from(&self.right);
        }
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // short-lived: returned once per frame
pub(crate) enum DialogOutcome {
    Keep(Dialog),
    Close,
}

impl Dialog {
    /// The properties dialog for a selection, if its elements share one.
    pub(crate) fn modify(elements: &[Element]) -> Option<Self> {
        let walls: Vec<&Wall> = elements
            .iter()
            .filter_map(|e| match e {
                Element::Wall(w) => Some(w),
                _ => None,
            })
            .collect();
        if !walls.is_empty() && walls.len() == elements.len() {
            let first = walls[0];
            return Some(Self::ModifyWalls {
                ids: walls.iter().map(|w| w.id).collect(),
                points: (walls.len() == 1).then_some((first.start, first.end)),
                thickness: first.thickness,
                height: first.height,
                arc: first.arc_extent.unwrap_or(0.0),
                finish: WallFinish {
                    wall_type: first.wall_type.clone(),
                    left: first.left_side.clone(),
                    right: first.right_side.clone(),
                    ..WallFinish::default()
                },
            });
        }
        match elements {
            [Element::Room(r)] => Some(Self::ModifyRoom(r.clone())),
            [Element::Dimension(d)] => Some(Self::ModifyDimension(d.clone())),
            [Element::Label(l)] => Some(Self::ModifyLabel(l.clone())),
            [Element::Level(l)] => Some(Self::ModifyLevel(l.clone())),
            [Element::Polyline(p)] => Some(Self::ModifyPolyline(p.clone())),
            [Element::Furniture(f)] => Some(Self::ModifyFurniture {
                piece: f.clone(),
                keep_ratio: false,
            }),
            _ => None,
        }
    }
}

fn cm(value: &mut f64, range: std::ops::RangeInclusive<f64>) -> DragValue<'_> {
    DragValue::new(value)
        .range(range)
        .suffix(" cm")
        .speed(1.0)
        .max_decimals(1)
}

/// Shows a dialog with OK/Cancel. Returns `Some(true)` on OK, `Some(false)` on cancel.
fn modal(ctx: &egui::Context, title: &str, body: impl FnOnce(&mut egui::Ui)) -> Option<bool> {
    let mut result = None;
    let response = crate::theme::modal(ctx, egui::Id::new("newera-dialog")).show(ctx, |ui| {
        // One width for every form in the window, so opening two in a row
        // does not move the fields around under the eye.
        ui.set_width(430.0);
        crate::theme::title(ui, title);
        crate::theme::body(ui, body);
        crate::theme::footer(ui, |ui| {
            // Shift+Enter stays available for new lines in multi-line fields.
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
            if crate::theme::primary(ui, crate::i18n::tr("OK")).clicked() || enter {
                result = Some(true);
            }
            if crate::theme::secondary(ui, crate::i18n::tr("Cancelar")).clicked() {
                result = Some(false);
            }
        });
    });
    if response.should_close() && result.is_none() {
        result = Some(false);
    }
    result
}

/// Editor for an optional surface finish. Returns true when it changed.
pub(crate) fn material_editor(
    ui: &mut egui::Ui,
    id: &str,
    material: &mut Option<Material>,
) -> bool {
    let before = material.clone();
    ui.vertical(|ui| {
        let current = match material {
            None => crate::i18n::tr("Sem acabamento").to_owned(),
            Some(m) if m.image.is_some() => crate::i18n::tr("Imagem").to_owned(),
            Some(Material {
                pattern: Some(p), ..
            }) => crate::i18n::tr(p.label()).to_owned(),
            Some(_) => crate::i18n::tr("Pintura").to_owned(),
        };
        egui::ComboBox::from_id_salt(id)
            .selected_text(current)
            .width(190.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(material.is_none(), crate::i18n::tr("Sem acabamento"))
                    .clicked()
                {
                    *material = None;
                }
                let paint = material
                    .as_ref()
                    .is_some_and(|m| m.pattern.is_none() && m.image.is_none());
                if ui
                    .selectable_label(paint, crate::i18n::tr("Pintura"))
                    .clicked()
                {
                    *material = Some(Material::paint([242, 239, 230]));
                }
                for pattern in Pattern::ALL {
                    let on = material
                        .as_ref()
                        .is_some_and(|m| m.pattern == Some(pattern) && m.image.is_none());
                    if ui
                        .selectable_label(on, crate::i18n::tr(pattern.label()))
                        .clicked()
                    {
                        *material = Some(Material::pattern(pattern));
                    }
                }
                #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
                let image = material.as_ref().is_some_and(|m| m.image.is_some());
                #[cfg(not(target_arch = "wasm32"))]
                if ui
                    .selectable_label(image, crate::i18n::tr("Imagem…"))
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter(
                            crate::i18n::tr("Imagem"),
                            &["png", "jpg", "jpeg", "webp", "bmp"],
                        )
                        .pick_file()
                {
                    *material = Some(Material {
                        image: Some(path.display().to_string()),
                        ..Material::default()
                    });
                }
            });
        let Some(m) = material else {
            return;
        };
        ui.horizontal(|ui| {
            let default = m.base_color([242, 239, 230]);
            let mut tinted = m.color.is_some();
            if ui.checkbox(&mut tinted, crate::i18n::tr("Cor")).changed() {
                m.color = tinted.then_some(default);
            }
            if let Some(color) = &mut m.color {
                egui::color_picker::color_edit_button_srgb(ui, color);
            }
        });
        if m.pattern.is_some() || m.image.is_some() {
            ui.horizontal(|ui| {
                let [mut w, mut h] = m.tile_size();
                ui.label(crate::i18n::tr("Peça"));
                let changed = ui.add(cm(&mut w, 1.0..=1000.0)).changed()
                    | ui.add(cm(&mut h, 1.0..=1000.0)).changed();
                if changed {
                    m.tile = Some([w, h]);
                }
                ui.add(
                    DragValue::new(&mut m.angle)
                        .range(-360.0..=360.0)
                        .suffix("°")
                        .speed(1.0),
                );
            });
        }
    });
    *material != before
}

/// The two columns every dialog is made of: what the property is called on
/// the left, quiet as a spec sheet, and what it is set to on the right.
fn grid(ui: &mut egui::Ui, id: &str, rows: impl FnOnce(&mut egui::Ui)) {
    let t = crate::theme::of(ui.visuals());
    ui.scope(|ui| {
        ui.visuals_mut().widgets.noninteractive.fg_stroke.color = t.ink_dim;
        ui.spacing_mut().text_edit_width = (ui.available_width() - 130.0).max(160.0);
        egui::Grid::new(id)
            .num_columns(2)
            .min_col_width(108.0)
            .spacing([16.0, 8.0])
            .show(ui, rows);
    });
}

pub(crate) fn show(app: &mut NewEraApp, ctx: &egui::Context, dialog: Dialog) -> DialogOutcome {
    let unit = app.unit();
    match dialog {
        Dialog::ModifyWalls {
            ids,
            mut points,
            mut thickness,
            mut height,
            mut arc,
            mut finish,
        } => {
            let title = if ids.len() == 1 {
                crate::i18n::tr("Modificar parede").to_owned()
            } else {
                crate::i18n::fill("Modificar {} paredes", &[&ids.len()])
            };
            let answer = modal(ctx, &title, |ui| {
                grid(ui, "walls", |ui| {
                    if let Some((start, end)) = &mut points {
                        ui.label(crate::i18n::tr("Início (x, y)"));
                        ui.horizontal(|ui| {
                            ui.add(cm(&mut start.x, -1e6..=1e6));
                            ui.add(cm(&mut start.y, -1e6..=1e6));
                        });
                        ui.end_row();
                        ui.label(crate::i18n::tr("Fim (x, y)"));
                        ui.horizontal(|ui| {
                            ui.add(cm(&mut end.x, -1e6..=1e6));
                            ui.add(cm(&mut end.y, -1e6..=1e6));
                        });
                        ui.end_row();
                        ui.label(crate::i18n::tr("Comprimento"));
                        ui.label(RichText::new(unit.format_length(start.distance(*end))).strong());
                        ui.end_row();
                    }
                    ui.label(crate::i18n::tr("Tipo"));
                    let type_name = finish
                        .wall_type
                        .as_deref()
                        .and_then(newera_core::wall_type)
                        .map_or(crate::i18n::tr("Personalizada"), |t| {
                            crate::i18n::tr(t.name)
                        });
                    egui::ComboBox::from_id_salt("wall_type")
                        .selected_text(type_name)
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    finish.wall_type.is_none(),
                                    crate::i18n::tr("Personalizada"),
                                )
                                .clicked()
                            {
                                finish.wall_type = None;
                                finish.type_changed = true;
                            }
                            for kind in WALL_TYPES {
                                let label = format!(
                                    "{} · {}",
                                    crate::i18n::tr(kind.name),
                                    unit.format_length(kind.thickness)
                                );
                                if ui
                                    .selectable_label(
                                        finish.wall_type.as_deref() == Some(kind.id),
                                        label,
                                    )
                                    .on_hover_text(crate::i18n::tr(kind.description))
                                    .clicked()
                                {
                                    finish.wall_type = Some(kind.id.to_owned());
                                    finish.type_changed = true;
                                    thickness = kind.thickness;
                                }
                            }
                        });
                    ui.end_row();
                    ui.label(crate::i18n::tr("Espessura"));
                    ui.add(cm(&mut thickness, 0.5..=500.0));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Altura"));
                    ui.add(cm(&mut height, 1.0..=5000.0));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Arco"));
                    ui.add(
                        DragValue::new(&mut arc)
                            .range(-270.0..=270.0)
                            .suffix("°")
                            .speed(1.0),
                    );
                    ui.end_row();
                    ui.label(crate::i18n::tr("Lado esquerdo"));
                    finish.left_changed |= material_editor(ui, "wall_left", &mut finish.left);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Lado direito"));
                    finish.right_changed |= material_editor(ui, "wall_right", &mut finish.right);
                    ui.end_row();
                });
            });
            match answer {
                Some(true) => {
                    app.run(|doc| {
                        let commands = ids
                            .iter()
                            .filter_map(|id| doc.home().wall(*id).cloned())
                            .map(|mut w| {
                                if let Some((start, end)) = points {
                                    w.start = start;
                                    w.end = end;
                                }
                                w.thickness = thickness;
                                w.height = height;
                                w.arc_extent = (arc.abs() >= 1.0).then_some(arc);
                                finish.apply(&mut w);
                                Command::update(w)
                            })
                            .collect();
                        doc.execute(Command::Batch { commands })
                    });
                    DialogOutcome::Close
                }
                Some(false) => DialogOutcome::Close,
                None => DialogOutcome::Keep(Dialog::ModifyWalls {
                    ids,
                    points,
                    thickness,
                    height,
                    arc,
                    finish,
                }),
            }
        }
        Dialog::ModifyRoom(mut room) => {
            let answer = modal(ctx, crate::i18n::tr("Modificar cômodo"), |ui| {
                grid(ui, "room", |ui| {
                    ui.label(crate::i18n::tr("Nome"));
                    ui.text_edit_singleline(&mut room.name).request_focus();
                    ui.end_row();
                    ui.label(crate::i18n::tr("Área"));
                    ui.label(RichText::new(unit.format_area(room.area())).strong());
                    ui.end_row();
                    ui.label(crate::i18n::tr("Exibir"));
                    ui.vertical(|ui| {
                        ui.checkbox(&mut room.area_visible, crate::i18n::tr("Área na planta"));
                        ui.checkbox(&mut room.floor_visible, crate::i18n::tr("Piso"));
                        ui.checkbox(&mut room.ceiling_visible, crate::i18n::tr("Teto"));
                    });
                    ui.end_row();
                    ui.label(crate::i18n::tr("Contorno"));
                    ui.checkbox(&mut room.auto, crate::i18n::tr("Acompanha as paredes"))
                        .on_hover_text(crate::i18n::tr(
                            "O contorno é detectado pelas paredes e divisores e se ajusta quando eles mudam",
                        ));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Piso"));
                    material_editor(ui, "room_floor", &mut room.floor_material);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Teto"));
                    material_editor(ui, "room_ceiling", &mut room.ceiling_material);
                    ui.end_row();
                });
            });
            if room.auto {
                let home = app.document.read().home().clone();
                let view = home.level_view(room.level);
                let dividers: Vec<&newera_core::Polyline> =
                    view.polylines.iter().filter(|l| l.room_divider).collect();
                if let Some(points) = newera_core::interior_point(&room.points)
                    .and_then(|p| newera_core::detect_room_with_dividers(&view.walls, &dividers, p))
                {
                    room.points = points;
                }
            }
            finish(app, answer, Dialog::ModifyRoom(room.clone()), || {
                Command::update(room)
            })
        }
        Dialog::ModifyDimension(mut dim) => {
            let answer = modal(ctx, crate::i18n::tr("Modificar cota"), |ui| {
                grid(ui, "dim", |ui| {
                    ui.label(crate::i18n::tr("Medida"));
                    ui.label(RichText::new(unit.format_length(dim.length())).strong());
                    ui.end_row();
                    ui.label(crate::i18n::tr("Afastamento"));
                    ui.add(cm(&mut dim.offset, -10_000.0..=10_000.0));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Vista 3D"));
                    ui.checkbox(&mut dim.visible_in_3d, crate::i18n::tr("Mostrar em 3D"));
                    ui.end_row();
                    if dim.visible_in_3d {
                        ui.label(crate::i18n::tr("Elevação"));
                        ui.horizontal(|ui| {
                            ui.add(cm(&mut dim.elevation[0], -1000.0..=10_000.0));
                            ui.add(cm(&mut dim.elevation[1], -1000.0..=10_000.0));
                        });
                        ui.end_row();
                        ui.label(crate::i18n::tr("Inclinação"));
                        ui.add(
                            DragValue::new(&mut dim.pitch)
                                .range(-180.0..=180.0)
                                .suffix("°")
                                .speed(1.0),
                        );
                        ui.end_row();
                    }
                });
            });
            finish(app, answer, Dialog::ModifyDimension(dim.clone()), || {
                Command::update(dim)
            })
        }
        Dialog::ModifyLabel(mut label) => {
            let answer = modal(ctx, crate::i18n::tr("Modificar texto"), |ui| {
                label_fields(ui, &mut label.text, &mut label.size, &mut label.angle);
                ui.add_space(4.0);
                grid(ui, "label_style", |ui| {
                    ui.label(crate::i18n::tr("Estilo"));
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut label.bold, crate::i18n::tr("Negrito"));
                        ui.checkbox(&mut label.italic, crate::i18n::tr("Itálico"));
                    });
                    ui.end_row();
                    ui.label(crate::i18n::tr("Alinhamento"));
                    ui.horizontal(|ui| {
                        use newera_core::TextAlign;
                        for (align, name) in [
                            (TextAlign::Left, crate::i18n::tr("Esquerda")),
                            (TextAlign::Center, crate::i18n::tr("Centro")),
                            (TextAlign::Right, crate::i18n::tr("Direita")),
                        ] {
                            ui.selectable_value(&mut label.align, align, name);
                        }
                    });
                    ui.end_row();
                    ui.label(crate::i18n::tr("Cor"));
                    optional_color(ui, &mut label.color, [40, 40, 48]);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Contorno"));
                    optional_color(ui, &mut label.outline, [255, 255, 255]);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Vista 3D"));
                    let mut shown = label.pitch.is_some();
                    if ui
                        .checkbox(&mut shown, crate::i18n::tr("Mostrar em 3D"))
                        .changed()
                    {
                        label.pitch = shown.then_some(90.0);
                    }
                    ui.end_row();
                    if let Some(pitch) = &mut label.pitch {
                        ui.label(crate::i18n::tr("Elevação"));
                        ui.add(cm(&mut label.elevation, -1000.0..=10_000.0));
                        ui.end_row();
                        ui.label(crate::i18n::tr("Inclinação"));
                        ui.horizontal(|ui| {
                            ui.selectable_value(pitch, 0.0, crate::i18n::tr("Deitado"));
                            ui.selectable_value(pitch, 90.0, crate::i18n::tr("Em pé"));
                            ui.add(DragValue::new(pitch).range(0.0..=90.0).suffix("°"));
                        });
                        ui.end_row();
                    }
                });
            });
            finish(app, answer, Dialog::ModifyLabel(label.clone()), || {
                Command::update(label)
            })
        }
        Dialog::ModifyFurniture {
            mut piece,
            mut keep_ratio,
        } => {
            let before = (piece.width, piece.depth, piece.height);
            let answer = modal(
                ctx,
                &crate::i18n::fill("Modificar {}", &[&piece.name.to_lowercase()]),
                |ui| {
                    grid(ui, "furniture-name", |ui| {
                        ui.label(crate::i18n::tr("Nome"));
                        ui.text_edit_singleline(&mut piece.name);
                        ui.end_row();
                    });
                    crate::theme::section(ui, crate::i18n::tr("Planta"));
                    grid(ui, "furniture-place", |ui| {
                        ui.label(crate::i18n::tr("Posição (x, y)"));
                        ui.horizontal(|ui| {
                            ui.add(cm(&mut piece.position.x, -1e6..=1e6));
                            ui.add(cm(&mut piece.position.y, -1e6..=1e6));
                        });
                        ui.end_row();
                        ui.label(crate::i18n::tr("Elevação"));
                        ui.add(cm(&mut piece.elevation, -1000.0..=10_000.0));
                        ui.end_row();
                        ui.label(crate::i18n::tr("Ângulo"));
                        ui.add(
                            DragValue::new(&mut piece.angle)
                                .range(-360.0..=360.0)
                                .suffix("°")
                                .speed(1.0),
                        );
                        ui.end_row();
                    });
                    crate::theme::section(ui, crate::i18n::tr("Tamanho"));
                    grid(ui, "furniture-size", |ui| {
                        ui.label(crate::i18n::tr("Largura"));
                        ui.add(cm(&mut piece.width, 1.0..=10_000.0));
                        ui.end_row();
                        ui.label(crate::i18n::tr("Profundidade"));
                        ui.add(cm(&mut piece.depth, 1.0..=10_000.0));
                        ui.end_row();
                        ui.label(crate::i18n::tr("Altura"));
                        ui.add(cm(&mut piece.height, 1.0..=10_000.0));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut keep_ratio, crate::i18n::tr("Manter proporções"));
                        ui.end_row();
                    });
                    crate::theme::section(ui, crate::i18n::tr("Aparência"));
                    grid(ui, "furniture-look", |ui| {
                        ui.label(crate::i18n::tr("Cor"));
                        ui.horizontal(|ui| {
                            let mut custom = piece.color.is_some();
                            if ui
                                .checkbox(&mut custom, crate::i18n::tr("Personalizada"))
                                .changed()
                            {
                                piece.color = custom.then_some([180, 180, 180]);
                            }
                            if let Some(color) = &mut piece.color {
                                ui.color_edit_button_srgb(color);
                            }
                        });
                        ui.end_row();
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.checkbox(&mut piece.mirrored, crate::i18n::tr("Espelhado"));
                            ui.checkbox(&mut piece.visible, crate::i18n::tr("Visível"));
                            if let Some(opening) = piece
                                .opening
                                .as_mut()
                                .filter(|o| o.kind == newera_core::OpeningKind::Door && !o.sliding)
                            {
                                ui.checkbox(
                                    &mut opening.hinge_right,
                                    crate::i18n::tr("Dobradiça à direita"),
                                );
                            }
                        });
                        ui.end_row();
                        if let Some(light) = &mut piece.light {
                            ui.label(crate::i18n::tr("Potência da luz"));
                            ui.add(egui::Slider::new(&mut light.power, 0.0..=1.0));
                            ui.end_row();
                        }
                        if piece.is_group() {
                            ui.label(crate::i18n::tr("Grupo"));
                            ui.label(crate::i18n::fill(
                                "{} peças",
                                &[&(piece.flatten().len() - 1)],
                            ));
                            ui.end_row();
                        }
                    });
                    egui::CollapsingHeader::new(crate::i18n::tr("Informações"))
                        .id_salt("piece_info")
                        .show(ui, |ui| {
                            grid(ui, "piece_info_grid", |ui| {
                                for (label, value) in [
                                    (crate::i18n::tr("Marca"), &mut piece.info.brand),
                                    (crate::i18n::tr("Modelo"), &mut piece.info.model_name),
                                    (crate::i18n::tr("Link"), &mut piece.info.url),
                                    (crate::i18n::tr("Descrição"), &mut piece.info.description),
                                    (crate::i18n::tr("Informações"), &mut piece.info.information),
                                    (crate::i18n::tr("Autor"), &mut piece.info.creator),
                                    (crate::i18n::tr("Licença"), &mut piece.info.license),
                                    (crate::i18n::tr("Preço"), &mut piece.info.price),
                                ] {
                                    ui.label(label);
                                    let mut text = value.clone().unwrap_or_default();
                                    if ui.text_edit_singleline(&mut text).changed() {
                                        *value = (!text.is_empty()).then_some(text);
                                    }
                                    ui.end_row();
                                }
                                if let Some(url) =
                                    piece.info.url.clone().filter(|u| u.starts_with("http"))
                                {
                                    ui.label("");
                                    ui.hyperlink_to(crate::i18n::tr("Abrir link"), url);
                                    ui.end_row();
                                }
                                if let Some(id) = &piece.info.source_catalog_id {
                                    ui.label(crate::i18n::tr("Catálogo de origem"));
                                    ui.weak(id);
                                    ui.end_row();
                                }
                            });
                        });
                },
            );
            if keep_ratio {
                // Scale the other sizes by whichever one changed.
                let (w0, d0, h0) = before;
                let factor = if (piece.width - w0).abs() > 1e-9 {
                    piece.width / w0
                } else if (piece.depth - d0).abs() > 1e-9 {
                    piece.depth / d0
                } else if (piece.height - h0).abs() > 1e-9 {
                    piece.height / h0
                } else {
                    1.0
                };
                (piece.width, piece.depth, piece.height) = (w0 * factor, d0 * factor, h0 * factor);
            }
            finish(
                app,
                answer,
                Dialog::ModifyFurniture {
                    piece: piece.clone(),
                    keep_ratio,
                },
                || Command::update(piece),
            )
        }
        Dialog::NewLabel { at, mut text } => {
            let mut size = Label::DEFAULT_SIZE;
            let mut angle = 0.0;
            let answer = modal(ctx, crate::i18n::tr("Adicionar texto"), |ui| {
                label_fields(ui, &mut text, &mut size, &mut angle);
            });
            match answer {
                Some(true) if !text.trim().is_empty() => {
                    app.run(|doc| {
                        let label = Label {
                            id: doc.new_label_id(),
                            text: text.trim_end().to_owned(),
                            position: at,
                            size,
                            angle,
                            level: None,
                            ..Default::default()
                        };
                        doc.execute(Command::insert(label))
                    });
                    DialogOutcome::Close
                }
                Some(_) => DialogOutcome::Close,
                None => DialogOutcome::Keep(Dialog::NewLabel { at, text }),
            }
        }
        Dialog::Calibrate { a, b, mut distance } => {
            let answer = modal(ctx, crate::i18n::tr("Calibrar escala da imagem"), |ui| {
                ui.label(crate::i18n::fill(
                    "Os pontos marcados estão a {} na escala atual.",
                    &[&unit.format_length(a.distance(b))],
                ));
                ui.add_space(4.0);
                grid(ui, "calib", |ui| {
                    ui.label(crate::i18n::tr("Distância real"));
                    ui.add(cm(&mut distance, 1.0..=1e6)).request_focus();
                    ui.end_row();
                });
            });
            match answer {
                Some(true) => {
                    app.run(|doc| {
                        let Some(mut bg) = doc.home().background.clone() else {
                            return Ok(());
                        };
                        let measured = a.distance(b);
                        if measured > 1e-6 {
                            // Scale around the first point so it stays where it was marked.
                            let factor = distance / measured;
                            bg.cm_per_px *= factor;
                            bg.cm_per_px_y = bg.cm_per_px_y.map(|y| y * factor);
                            bg.offset = Point2::new(
                                a.x - (a.x - bg.offset.x) * factor,
                                a.y - (a.y - bg.offset.y) * factor,
                            );
                        }
                        doc.execute(Command::SetBackground {
                            background: Some(bg),
                        })
                    });
                    app.set_status(crate::i18n::tr("Escala calibrada. Arraste para posicionar a imagem ou troque de ferramenta."));
                    DialogOutcome::Close
                }
                Some(false) => DialogOutcome::Close,
                None => DialogOutcome::Keep(Dialog::Calibrate { a, b, distance }),
            }
        }
        Dialog::Background(mut bg) => {
            let mut remove = false;
            let answer = modal(ctx, crate::i18n::tr("Imagem de fundo"), |ui| {
                grid(ui, "bg", |ui| {
                    ui.label(crate::i18n::tr("Arquivo"));
                    ui.label(RichText::new(&bg.path).small());
                    ui.end_row();
                    ui.label(crate::i18n::tr("Escala"));
                    ui.add(
                        DragValue::new(&mut bg.cm_per_px)
                            .range(0.001..=1000.0)
                            .speed(0.01)
                            .suffix(" cm/px"),
                    );
                    ui.end_row();
                    ui.label(crate::i18n::tr("Escala vertical"));
                    ui.horizontal(|ui| {
                        let mut separate = bg.cm_per_px_y.is_some();
                        if ui
                            .checkbox(&mut separate, crate::i18n::tr("Diferente"))
                            .changed()
                        {
                            bg.cm_per_px_y = separate.then_some(bg.cm_per_px);
                        }
                        if let Some(y) = &mut bg.cm_per_px_y {
                            ui.add(
                                DragValue::new(y)
                                    .range(0.001..=1000.0)
                                    .speed(0.01)
                                    .suffix(" cm/px"),
                            );
                        }
                    });
                    ui.end_row();
                    ui.label(crate::i18n::tr("Rotação"));
                    ui.add(
                        DragValue::new(&mut bg.angle)
                            .range(-180.0..=180.0)
                            .speed(0.1)
                            .suffix("°"),
                    );
                    ui.end_row();
                    ui.label(crate::i18n::tr("Posição (x, y)"));
                    ui.horizontal(|ui| {
                        ui.add(cm(&mut bg.offset.x, -1e6..=1e6));
                        ui.add(cm(&mut bg.offset.y, -1e6..=1e6));
                    });
                    ui.end_row();
                    ui.label(crate::i18n::tr("Opacidade"));
                    ui.add(egui::Slider::new(&mut bg.opacity, 0.0..=1.0));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut bg.visible, crate::i18n::tr("Visível"));
                    ui.end_row();
                });
                ui.add_space(6.0);
                if ui
                    .button(format!(
                        "{} {}",
                        icon::TRASH,
                        crate::i18n::tr("Remover imagem")
                    ))
                    .clicked()
                {
                    remove = true;
                }
            });
            if remove {
                app.run(|doc| doc.execute(Command::SetBackground { background: None }));
                return DialogOutcome::Close;
            }
            finish(app, answer, Dialog::Background(bg.clone()), || {
                Command::SetBackground {
                    background: Some(bg),
                }
            })
        }
        Dialog::HomeSettings {
            mut name,
            mut compass,
        } => {
            let answer = modal(ctx, crate::i18n::tr("Casa e bússola"), |ui| {
                grid(ui, "home", |ui| {
                    ui.label(crate::i18n::tr("Nome do projeto"));
                    ui.text_edit_singleline(&mut name);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Norte"));
                    ui.add(
                        DragValue::new(&mut compass.north_degrees)
                            .range(0.0..=360.0)
                            .suffix("°")
                            .speed(1.0),
                    );
                    ui.end_row();
                    ui.label(crate::i18n::tr("Centro (x, y)"));
                    ui.horizontal(|ui| {
                        ui.add(cm(&mut compass.center.x, -1e6..=1e6));
                        ui.add(cm(&mut compass.center.y, -1e6..=1e6));
                    });
                    ui.end_row();
                    ui.label(crate::i18n::tr("Diâmetro"));
                    ui.add(cm(&mut compass.diameter, 10.0..=10_000.0));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut compass.visible, crate::i18n::tr("Mostrar bússola"));
                    ui.end_row();
                });
            });
            let (n, c) = (name.clone(), compass.clone());
            finish(app, answer, Dialog::HomeSettings { name, compass }, || {
                Command::Batch {
                    commands: vec![
                        Command::RenameHome { name: n },
                        Command::SetCompass { compass: c },
                    ],
                }
            })
        }
        Dialog::ConfirmDiscard(action) => {
            let mut choice = None;
            crate::theme::modal(ctx, egui::Id::new("confirm-discard")).show(ctx, |ui| {
                ui.set_min_width(380.0);
                crate::theme::title(ui, crate::i18n::tr("Salvar alterações?"));
                ui.label(crate::i18n::tr(
                    "O projeto tem alterações que ainda não foram salvas.",
                ));
                crate::theme::footer(ui, |ui| {
                    if crate::theme::primary(ui, crate::i18n::tr("Salvar")).clicked() {
                        choice = Some(0);
                    }
                    if crate::theme::secondary(ui, crate::i18n::tr("Descartar")).clicked() {
                        choice = Some(1);
                    }
                    if crate::theme::secondary(ui, crate::i18n::tr("Cancelar")).clicked() {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    // The save runs in the background; what was waiting on it
                    // happens when it lands.
                    app.save_then(false, Some(action));
                    DialogOutcome::Close
                }
                Some(1) => {
                    app.perform(action);
                    DialogOutcome::Close
                }
                Some(_) => DialogOutcome::Close,
                None => DialogOutcome::Keep(Dialog::ConfirmDiscard(action)),
            }
        }
        Dialog::ConfirmCloseVariant { index, name } => {
            let mut choice = None;
            crate::theme::modal(ctx, egui::Id::new("close-variant")).show(ctx, |ui| {
                ui.set_min_width(380.0);
                crate::theme::title(ui, &crate::i18n::fill("Fechar “{}”?", &[&name]));
                ui.label(crate::i18n::tr(
                    "Esta versão e o histórico dela serão removidos do projeto.",
                ));
                crate::theme::footer(ui, |ui| {
                    if crate::theme::destructive(ui, crate::i18n::tr("Fechar versão")).clicked() {
                        choice = Some(true);
                    }
                    if crate::theme::secondary(ui, crate::i18n::tr("Cancelar")).clicked() {
                        choice = Some(false);
                    }
                });
            });
            match choice {
                Some(true) => {
                    app.run(|doc| doc.remove_variant(index));
                    app.after_variant_change();
                    DialogOutcome::Close
                }
                Some(false) => DialogOutcome::Close,
                None => DialogOutcome::Keep(Dialog::ConfirmCloseVariant { index, name }),
            }
        }
        Dialog::Quantities(rows) => {
            let mut close = false;
            let lengths: Vec<(newera_core::Discipline, f64)> = {
                let doc = app.document.read();
                newera_core::Discipline::ALL
                    .iter()
                    .map(|d| {
                        let total = doc
                            .home()
                            .polylines
                            .iter()
                            .filter(|p| p.discipline == Some(*d))
                            .map(|p| {
                                p.points
                                    .windows(2)
                                    .map(|s| s[0].distance(s[1]))
                                    .sum::<f64>()
                            })
                            .sum();
                        (*d, total)
                    })
                    .collect()
            };
            crate::theme::modal(ctx, egui::Id::new("quantities")).show(ctx, |ui| {
                ui.set_min_width(420.0);
                crate::theme::title(
                    ui,
                    &format!(
                        "{} {}",
                        icon::LIST_NUMBERS,
                        crate::i18n::tr("Quantitativos")
                    ),
                );
                for d in newera_core::Discipline::ALL {
                    ui.strong(d.name());
                    let mine: Vec<_> = rows.iter().filter(|r| r.discipline == d).collect();
                    if mine.is_empty() {
                        ui.weak(crate::i18n::tr("Nenhum ponto"));
                    }
                    egui::Grid::new(format!("q-{d:?}"))
                        .striped(true)
                        .show(ui, |ui| {
                            for row in mine {
                                ui.label(&row.name);
                                ui.label(RichText::new(row.count.to_string()).strong());
                                ui.end_row();
                            }
                            if let Some((_, length)) = lengths.iter().find(|(x, _)| *x == d)
                                && *length > 0.0
                            {
                                ui.label(crate::i18n::tr("Linhas (tubulação / eletroduto)"));
                                ui.label(RichText::new(unit.format_length(*length)).strong());
                                ui.end_row();
                            }
                        });
                    ui.add_space(8.0);
                }
                crate::theme::footer(ui, |ui| {
                    if crate::theme::primary(ui, crate::i18n::tr("Fechar")).clicked() {
                        close = true;
                    }
                });
            });
            if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                DialogOutcome::Close
            } else {
                DialogOutcome::Keep(Dialog::Quantities(rows))
            }
        }
        Dialog::Electrical => {
            let mut close = false;
            let (circuits, findings, cables) = {
                let doc = app.document.read();
                let home = doc.home();
                (
                    newera_core::electrical::circuits(home),
                    newera_core::electrical::check(home),
                    newera_core::electrical::cable_lengths(home),
                )
            };
            crate::theme::modal(ctx, egui::Id::new("electrical")).show(ctx, |ui| {
                ui.set_min_width(560.0);
                crate::theme::title(
                    ui,
                    &format!(
                        "{} {}",
                        icon::LIGHTNING,
                        crate::i18n::tr("Quadro de cargas e NBR 5410")
                    ),
                );
                if circuits.is_empty() {
                    ui.weak(crate::i18n::tr(
                        "Nenhum circuito: atribua os pontos a circuitos (MCP electrical assign).",
                    ));
                } else {
                    egui::Grid::new("load-schedule")
                        .striped(true)
                        .show(ui, |ui| {
                            for head in [
                                "Circuito",
                                "Tipo",
                                "Pontos",
                                "VA",
                                "V",
                                "A",
                                "Fio mm²",
                                "Disjuntor",
                                "DR",
                            ] {
                                ui.strong(crate::i18n::tr(head));
                            }
                            ui.end_row();
                            for c in &circuits {
                                ui.label(&c.name);
                                ui.label(
                                    c.kinds
                                        .iter()
                                        .map(|k| k.name())
                                        .collect::<Vec<_>>()
                                        .join("+"),
                                );
                                ui.label(c.points.len().to_string());
                                ui.label(format!("{:.0}", c.va));
                                ui.label(format!("{:.0}", c.volts));
                                ui.label(newera_core::electrical::decimal(c.amps));
                                ui.label(format!("{}", c.wire_mm2).replace('.', ","));
                                ui.label(format!("{} A", c.breaker_a));
                                ui.label(if c.rcd { "30 mA" } else { "—" });
                                ui.end_row();
                            }
                        });
                    let total: f64 = circuits.iter().map(|c| c.va).sum();
                    ui.strong(format!(
                        "{} {total:.0} VA",
                        crate::i18n::tr("Total instalado:")
                    ));
                }
                for (cable, metres) in &cables {
                    ui.label(format!(
                        "{}: {} m",
                        cable.name(),
                        newera_core::electrical::decimal(*metres)
                    ));
                }
                ui.add_space(8.0);
                if findings.is_empty() {
                    ui.label(crate::i18n::tr("Nada a apontar."));
                }
                // A long list scrolls instead of pushing the window off screen.
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for f in &findings {
                            let color = match f.severity {
                                newera_core::electrical::Severity::Erro => {
                                    egui::Color32::from_rgb(200, 60, 50)
                                }
                                newera_core::electrical::Severity::Alerta => {
                                    egui::Color32::from_rgb(200, 140, 40)
                                }
                                newera_core::electrical::Severity::Dica => {
                                    ui.visuals().weak_text_color()
                                }
                            };
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&f.place).strong().color(color));
                                ui.label(f.message.in_language(crate::i18n::findings_language()));
                            });
                        }
                    });
                ui.add_space(6.0);
                crate::theme::footer(ui, |ui| {
                    if crate::theme::primary(ui, crate::i18n::tr("Fechar")).clicked() {
                        close = true;
                    }
                });
            });
            if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                DialogOutcome::Close
            } else {
                DialogOutcome::Keep(Dialog::Electrical)
            }
        }
        Dialog::ModifyPolyline(mut line) => {
            use newera_core::{ArrowStyle, DashStyle, LineJoin};
            let answer = modal(ctx, crate::i18n::tr("Modificar linha"), |ui| {
                grid(ui, "polyline", |ui| {
                    ui.label(crate::i18n::tr("Espessura"));
                    ui.add(cm(&mut line.thickness, 0.1..=100.0));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Cor"));
                    ui.color_edit_button_srgb(&mut line.color);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Traço"));
                    egui::ComboBox::from_id_salt("dash")
                        .selected_text(format!("{:?}", line.dash))
                        .show_ui(ui, |ui| {
                            for (dash, name) in [
                                (DashStyle::Solid, crate::i18n::tr("Contínuo")),
                                (DashStyle::Dot, crate::i18n::tr("Pontilhado")),
                                (DashStyle::Dash, crate::i18n::tr("Tracejado")),
                                (DashStyle::DashDot, crate::i18n::tr("Traço e ponto")),
                                (
                                    DashStyle::DashDotDot,
                                    crate::i18n::tr("Traço e dois pontos"),
                                ),
                            ] {
                                ui.selectable_value(&mut line.dash, dash, name);
                            }
                        });
                    ui.end_row();
                    ui.label(crate::i18n::tr("Ambientes"));
                    ui.checkbox(
                        &mut line.room_divider,
                        crate::i18n::tr("Divisor de ambiente"),
                    )
                    .on_hover_text(crate::i18n::tr(
                        "Separa ambientes sem parede (ex.: sala e jantar integrados)",
                    ));
                    ui.end_row();
                    for (label, arrow, salt) in [
                        (
                            crate::i18n::tr("Início"),
                            &mut line.start_arrow,
                            "start_arrow",
                        ),
                        (crate::i18n::tr("Fim"), &mut line.end_arrow, "end_arrow"),
                    ] {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(salt)
                            .selected_text(format!("{arrow:?}"))
                            .show_ui(ui, |ui| {
                                for (style, name) in [
                                    (ArrowStyle::None, crate::i18n::tr("Sem seta")),
                                    (ArrowStyle::Delta, crate::i18n::tr("Seta cheia")),
                                    (ArrowStyle::Open, crate::i18n::tr("Seta aberta")),
                                    (ArrowStyle::Disc, crate::i18n::tr("Disco")),
                                ] {
                                    ui.selectable_value(arrow, style, name);
                                }
                            });
                        ui.end_row();
                    }
                    ui.label("");
                    ui.vertical(|ui| {
                        let mut curved = line.join == LineJoin::Curved;
                        if ui
                            .checkbox(&mut curved, crate::i18n::tr("Curva suave"))
                            .changed()
                        {
                            line.join = if curved {
                                LineJoin::Curved
                            } else {
                                LineJoin::Miter
                            };
                        }
                        ui.checkbox(&mut line.closed, crate::i18n::tr("Fechada"));
                    });
                    ui.end_row();
                });
            });
            finish(app, answer, Dialog::ModifyPolyline(line.clone()), || {
                Command::update(line)
            })
        }
        Dialog::ModifyLevel(mut level) => {
            let answer = modal(ctx, crate::i18n::tr("Modificar andar"), |ui| {
                grid(ui, "level", |ui| {
                    ui.label(crate::i18n::tr("Nome"));
                    ui.text_edit_singleline(&mut level.name);
                    ui.end_row();
                    ui.label(crate::i18n::tr("Elevação do piso"));
                    ui.add(cm(&mut level.elevation, -10_000.0..=100_000.0));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Pé-direito"));
                    ui.add(cm(&mut level.height, 0.0..=2_000.0));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Espessura da laje"));
                    ui.add(cm(&mut level.floor_thickness, 0.0..=200.0));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Ordem (mesma elevação)"))
                        .on_hover_text(crate::i18n::tr(
                            "Níveis na mesma elevação funcionam como layouts alternativos",
                        ));
                    ui.add(DragValue::new(&mut level.elevation_index).range(0..=99));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut level.viewable, crate::i18n::tr("Visível no 3D"));
                    ui.end_row();
                });
            });
            finish(app, answer, Dialog::ModifyLevel(level.clone()), || {
                Command::update(level)
            })
        }
        Dialog::ConfirmDeleteLevel { id, name } => {
            let mut choice = None;
            crate::theme::modal(ctx, egui::Id::new("delete-level")).show(ctx, |ui| {
                ui.set_min_width(380.0);
                crate::theme::title(ui, &crate::i18n::fill("Excluir “{}”?", &[&name]));
                ui.label(crate::i18n::tr(
                    "O andar e tudo o que está nele serão removidos (dá para desfazer).",
                ));
                crate::theme::footer(ui, |ui| {
                    if crate::theme::destructive(ui, crate::i18n::tr("Excluir andar")).clicked() {
                        choice = Some(true);
                    }
                    if crate::theme::secondary(ui, crate::i18n::tr("Cancelar")).clicked() {
                        choice = Some(false);
                    }
                });
            });
            match choice {
                Some(true) => {
                    app.run(|doc| newera_core::ops::delete_level(doc, id));
                    app.after_variant_change();
                    DialogOutcome::Close
                }
                Some(false) => DialogOutcome::Close,
                None => DialogOutcome::Keep(Dialog::ConfirmDeleteLevel { id, name }),
            }
        }
        Dialog::Compare(rows) => {
            let mut close = false;
            let mut switch = None;
            crate::theme::modal(ctx, egui::Id::new("compare-variants")).show(ctx, |ui| {
                crate::theme::title(
                    ui,
                    &format!(
                        "{} {}",
                        icon::CHART_BAR,
                        crate::i18n::tr("Comparar versões")
                    ),
                );
                egui::Grid::new("compare")
                    .striped(true)
                    .spacing([18.0, 6.0])
                    .show(ui, |ui| {
                        for title in [
                            crate::i18n::tr("Versão"),
                            crate::i18n::tr("Paredes"),
                            crate::i18n::tr("Cômodos"),
                            crate::i18n::tr("Área"),
                            crate::i18n::tr("Móveis"),
                            crate::i18n::tr("Problemas"),
                            "",
                        ] {
                            ui.label(RichText::new(title).strong());
                        }
                        ui.end_row();
                        let best_area = rows.iter().map(|r| r.area).fold(0.0, f64::max);
                        for (i, row) in rows.iter().enumerate() {
                            let name = RichText::new(&row.name);
                            ui.label(if row.active { name.strong() } else { name });
                            ui.label(row.walls.to_string());
                            ui.label(row.rooms.to_string());
                            let area = RichText::new(unit.format_area(row.area));
                            ui.label(
                                if rows.len() > 1
                                    && row.area > 0.0
                                    && (row.area - best_area).abs() < 1e-6
                                {
                                    area.strong()
                                } else {
                                    area
                                },
                            );
                            ui.label(row.furniture.to_string());
                            let issues = RichText::new(row.issues.to_string());
                            ui.label(if row.issues > 0 {
                                issues.color(ui.visuals().warn_fg_color)
                            } else {
                                issues
                            });
                            if row.active {
                                ui.weak(crate::i18n::tr("atual"));
                            } else if ui.small_button(crate::i18n::tr("Abrir")).clicked() {
                                switch = Some(i);
                            }
                            ui.end_row();
                        }
                    });
                crate::theme::footer(ui, |ui| {
                    if crate::theme::primary(ui, crate::i18n::tr("Fechar")).clicked() {
                        close = true;
                    }
                });
            });
            if let Some(index) = switch {
                app.run(|doc| doc.switch_variant(index));
                app.after_variant_change();
                return DialogOutcome::Close;
            }
            if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                DialogOutcome::Close
            } else {
                DialogOutcome::Keep(Dialog::Compare(rows))
            }
        }
        Dialog::ConnectAi { client } => {
            let mut client = client;
            if crate::ai::panel(app, ctx, &mut client) {
                DialogOutcome::Close
            } else {
                DialogOutcome::Keep(Dialog::ConnectAi { client })
            }
        }
        #[cfg(target_arch = "wasm32")]
        Dialog::ServerProjects => server_projects(app, ctx),
        #[cfg(target_arch = "wasm32")]
        Dialog::NewServerProject { name } => new_server_project(app, ctx, name),
        Dialog::Help => {
            let mut close = false;
            crate::theme::modal(ctx, egui::Id::new("help")).show(ctx, |ui| {
                ui.set_min_width(460.0);
                crate::theme::title(
                    ui,
                    &format!(
                        "{} {}",
                        icon::KEYBOARD,
                        crate::i18n::tr("Atalhos e ferramentas")
                    ),
                );
                grid(ui, "help", |ui| {
                    for (keys, what) in [
                        (
                            "V · H · W · R · D · T",
                            crate::i18n::tr(
                                "Selecionar · Mover vista · Paredes · Cômodos · Cotas · Texto",
                            ),
                        ),
                        (
                            crate::i18n::tr("Digitar número + Enter"),
                            crate::i18n::tr("Comprimento exato da parede sendo desenhada"),
                        ),
                        (
                            crate::i18n::tr("Shift (segurado)"),
                            crate::i18n::tr("Desliga o ímã (ângulos de 15°, pontos e grade)"),
                        ),
                        (
                            crate::i18n::tr("Duplo clique"),
                            crate::i18n::tr(
                                "Encerra paredes · fecha/detecta cômodo · cota parede · modifica",
                            ),
                        ),
                        (
                            crate::i18n::tr("Scroll · botão do meio · F"),
                            crate::i18n::tr("Zoom · mover vista · enquadrar"),
                        ),
                        (
                            crate::i18n::tr("Setas (+Shift)"),
                            crate::i18n::tr("Move a seleção 1 cm (10 cm)"),
                        ),
                        (
                            "Ctrl+Z · Ctrl+Shift+Z",
                            crate::i18n::tr("Desfazer · Refazer (inclusive o que a IA fez)"),
                        ),
                        (
                            "Ctrl+C · X · V · D",
                            crate::i18n::tr("Copiar · Recortar · Colar · Duplicar"),
                        ),
                        (
                            "Enter · Del · Esc",
                            crate::i18n::tr("Modificar · Excluir · Cancelar"),
                        ),
                        (
                            "Ctrl+T · Ctrl+Tab",
                            crate::i18n::tr("Duplicar versão · Próxima versão (guias)"),
                        ),
                        (
                            "Ctrl+1 · Ctrl+2 · Ctrl+3",
                            crate::i18n::tr("Catálogo · Planta · 3D (telas estreitas)"),
                        ),
                        (
                            "Ctrl+Shift+M",
                            crate::i18n::tr("Conectar sua IA (no navegador, liga e desliga)"),
                        ),
                    ] {
                        ui.label(RichText::new(keys).monospace().strong());
                        ui.label(what);
                        ui.end_row();
                    }
                });
                crate::theme::footer(ui, |ui| {
                    if crate::theme::primary(ui, crate::i18n::tr("Fechar")).clicked() {
                        close = true;
                    }
                });
            });
            if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                DialogOutcome::Close
            } else {
                DialogOutcome::Keep(Dialog::Help)
            }
        }
    }
}

/// The account's projects on the service: each opens on a click, the way a
/// link to it opens it.
#[cfg(target_arch = "wasm32")]
fn server_projects(app: &mut NewEraApp, ctx: &egui::Context) -> DialogOutcome {
    let view = crate::ai_web::server_view(&app.ai_link);
    let (mut close, mut open, mut again) = (false, None, false);
    crate::theme::modal(ctx, egui::Id::new("server-projects")).show(ctx, |ui| {
        ui.set_min_width(460.0);
        crate::theme::title(
            ui,
            &format!(
                "{} {}",
                icon::CLOUD_ARROW_DOWN,
                crate::i18n::tr("Projetos no servidor")
            ),
        );
        match &view.list {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(crate::i18n::tr("Carregando os projetos…"));
                });
            }
            Some(Err(why)) => {
                ui.label(crate::i18n::fill(
                    "⚠ Não foi possível listar os projetos: {}",
                    &[why],
                ));
                again = ui.button(crate::i18n::tr("Tentar de novo")).clicked();
            }
            Some(Ok(list)) if list.is_empty() => {
                ui.label(crate::i18n::tr("Nenhum projeto no servidor ainda."));
            }
            Some(Ok(list)) => {
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        grid(ui, "server-projects", |ui| {
                            ui.label(crate::i18n::tr("Nome"));
                            ui.label(crate::i18n::tr("Última alteração"));
                            ui.end_row();
                            for project in list {
                                if ui
                                    .add_enabled(!view.busy, egui::Button::new(&project.name))
                                    .clicked()
                                {
                                    open = Some((project.id.clone(), project.name.clone()));
                                }
                                ui.label(crate::ai_web::when(project.updated_at));
                                ui.end_row();
                            }
                        });
                    });
            }
        }
        if view.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(crate::i18n::tr("Abrindo projeto"));
            });
        }
        if let Some(why) = &view.error {
            ui.label(crate::i18n::fill("⚠ O servidor recusou: {}", &[why]));
        }
        crate::theme::footer(ui, |ui| {
            if crate::theme::secondary(ui, crate::i18n::tr("Fechar")).clicked() {
                close = true;
            }
        });
    });
    if view.busy || view.list.is_none() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        crate::ai_web::forget_asked(&app.ai_link);
        return DialogOutcome::Close;
    }
    if again {
        crate::ai_web::list_projects(&app.ai_link);
    }
    if let Some((id, name)) = open {
        crate::ai_web::open_project(&app.ai_link, &id, &name);
    }
    DialogOutcome::Keep(Dialog::ServerProjects)
}

/// A name for a new project on the service, which opens once it is made.
#[cfg(target_arch = "wasm32")]
fn new_server_project(app: &mut NewEraApp, ctx: &egui::Context, mut name: String) -> DialogOutcome {
    let view = crate::ai_web::server_view(&app.ai_link);
    let answer = modal(ctx, crate::i18n::tr("Novo projeto no servidor"), |ui| {
        grid(ui, "new-server-project", |ui| {
            ui.label(crate::i18n::tr("Nome do projeto"));
            ui.add_enabled(!view.busy, egui::TextEdit::singleline(&mut name))
                .request_focus();
            ui.end_row();
        });
        if view.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(crate::i18n::tr("Criando o projeto…"));
            });
        }
        if let Some(why) = &view.error {
            ui.label(crate::i18n::fill("⚠ O servidor recusou: {}", &[why]));
        }
    });
    if view.busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    match answer {
        Some(true) if !view.busy && !name.trim().is_empty() => {
            crate::ai_web::make_project(&app.ai_link, &name);
            DialogOutcome::Keep(Dialog::NewServerProject { name })
        }
        Some(false) => {
            crate::ai_web::forget_asked(&app.ai_link);
            DialogOutcome::Close
        }
        _ => DialogOutcome::Keep(Dialog::NewServerProject { name }),
    }
}

/// A color that can be left unset (uses the default look).
fn optional_color(ui: &mut egui::Ui, color: &mut Option<[u8; 3]>, default: [u8; 3]) {
    ui.horizontal(|ui| {
        let mut custom = color.is_some();
        if ui
            .checkbox(&mut custom, crate::i18n::tr("Personalizada"))
            .changed()
        {
            *color = custom.then_some(default);
        }
        if let Some(c) = color {
            ui.color_edit_button_srgb(c);
        }
    });
}

fn label_fields(ui: &mut egui::Ui, text: &mut String, size: &mut f64, angle: &mut f64) {
    grid(ui, "label", |ui| {
        ui.label(crate::i18n::tr("Texto"));
        ui.text_edit_multiline(text).request_focus();
        ui.end_row();
        ui.label(crate::i18n::tr("Tamanho"));
        ui.add(cm(size, 1.0..=1000.0));
        ui.end_row();
        ui.label(crate::i18n::tr("Rotação"));
        ui.add(
            DragValue::new(angle)
                .range(-360.0..=360.0)
                .suffix("°")
                .speed(1.0),
        );
        ui.end_row();
    });
}

fn finish(
    app: &mut NewEraApp,
    answer: Option<bool>,
    keep: Dialog,
    command: impl FnOnce() -> Command,
) -> DialogOutcome {
    match answer {
        Some(true) => {
            app.run(|doc| doc.execute(command()));
            DialogOutcome::Close
        }
        Some(false) => DialogOutcome::Close,
        None => DialogOutcome::Keep(keep),
    }
}
