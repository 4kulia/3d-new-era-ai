//! Imported 3D models, looked at from the inside: what a file became once
//! loaded, so a piece that looks wrong can be told apart from a file that
//! is wrong or a feature that is not drawn.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt::Write as _;

use super::NewEraMcp;
use super::reply::{core, invalid, ok};

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelParams {
    /// A piece that uses an imported model, e.g. `f12`.
    id: Option<String>,
    /// Instead of `id`: an .obj/.gltf/.glb file, before placing it.
    file: Option<String>,
    /// Only the parts whose name has these words (`braco`), all of them.
    part: Option<String>,
    /// `clashes`: which parts pass through one another, and faces with no area.
    #[schemars(extend("enum" = ["clashes"]))]
    check: Option<String>,
    /// check: how far a surface must pass the other to count, cm (default 0.05); touching is not a clash.
    tol: Option<f64>,
    /// Unit of the file's numbers, `m`, `cm`, `mm` or `in` (default: the piece's, or guessed).
    unit: Option<String>,
}

/// A unit named by a caller, or the one a piece was placed with.
fn unit_of(
    asked: Option<&str>,
    piece: Option<&newera_core::Furniture>,
) -> Result<Option<newera_catalog::Unit>, ErrorData> {
    if let Some(raw) = asked {
        return newera_catalog::Unit::parse(raw).map(Some).ok_or_else(|| {
            invalid(format!(
                "unit `{raw}`: {}",
                newera_catalog::Unit::NAMES.join(", ")
            ))
        });
    }
    Ok(piece
        .and_then(|p| p.properties.get(crate::edit::MODEL_UNIT_KEY))
        .and_then(|u| newera_catalog::Unit::parse(u)))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditModelParams {
    /// `replace` puts another file in the pieces, keeping each one's id and all the rest;
    /// `material` changes one of their materials, `part` one of their parts, by name;
    /// `lod` gives them a lighter file drawn from afar; `reference` keeps a photo of the
    /// product with them, `measure` a measurement of it.
    #[schemars(extend("enum" = ["replace", "material", "part", "lod", "reference", "measure"]))]
    action: String,
    /// Pieces whose model changes, e.g. `["f12"]`.
    ids: Vec<String>,
    /// replace/lod/reference: the new .obj/.gltf/.glb file (lod: the lighter one; reference: the photo).
    file: Option<String>,
    /// replace: also every other piece using the same file as `ids` (default: only `ids`).
    #[serde(default)]
    every: bool,
    /// replace: `keep` each piece's w/d/h (default) or take the new file's `natural` size.
    #[schemars(extend("enum" = ["keep", "natural"]))]
    size: Option<String>,
    /// Revision the change was prepared on; refused if the plan has moved on since.
    rev: Option<u64>,
    /// replace: unit of the new file's numbers, `m`, `cm`, `mm` or `in` (default: guessed).
    unit: Option<String>,
    /// material: which one, by the name `model` lists (`tecido`).
    material: Option<String>,
    /// material: its color `[r,g,b]`.
    color: Option<[u8; 3]>,
    /// material: an image over it, `img:trama.png`, or `none` for the file's own.
    mat: Option<String>,
    /// material: times its image repeats against the file's mapping (2 draws it at half size).
    repeat: Option<f64>,
    /// material/part: back to the file's own.
    #[serde(default)]
    clear: bool,
    /// part: which one, by the name `model` lists (`almofada_encosto`).
    part: Option<String>,
    /// part: hide it (true) or show it again (false).
    hide: Option<bool>,
    /// part: move it by `[x across, y to the front, z up]` cm from where the file has it.
    offset: Option<[f64; 3]>,
    /// part: resize it about its center, `[x, y, z]` factors.
    scale: Option<[f64; 3]>,
    /// lod: farther than this from the camera, cm, the lighter file is drawn (default 400).
    beyond: Option<f64>,
    /// lod: `always` draws the detailed file however far (a close review); `auto` switches again.
    #[schemars(extend("enum" = ["auto", "always"]))]
    detail: Option<String>,
    /// reference: the side the photo shows.
    #[schemars(extend("enum" = ["front", "back", "left", "right", "top", "aerial"]))]
    view: Option<String>,
    /// reference/measure: where it came from (a product page).
    source: Option<String>,
    /// reference: what it shows.
    note: Option<String>,
    /// reference: take out the one at this index (`model id=` lists them).
    remove: Option<usize>,
    /// measure: `width`, `depth`, `height` or `<part>.height` (its top above the floor).
    dimension: Option<String>,
    /// measure: the product's value, cm.
    cm: Option<f64>,
    /// measure: measured or stated by the maker, not estimated.
    #[serde(default)]
    confirmed: bool,
}

/// A short, stable name for the bytes of a file: which version was loaded.
fn fingerprint(path: &std::path::Path) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let bytes = newera_core::vfs::read(path).ok()?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    Some(format!(
        "{}B #{:08x}",
        bytes.len(),
        hash.finish() & 0xffff_ffff
    ))
}

/// `#rrggbb` of a material color in `0..=1`.
fn hex(c: [f32; 3]) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c[0]), byte(c[1]), byte(c[2]))
}

/// Rounded to a millimeter, as every other size the tools answer.
fn cm(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// The named parts of a model: `[name, triangles, [x, y, z] min, [x, y, z]
/// max, materials, "hidden"?]`, in cm of the mesh — for a piece, fitted to
/// it: x across its width from the center, y from the center to its front,
/// z up from its bottom.
pub(crate) fn parts(
    mesh: &newera_catalog::Mesh,
    hidden: &[String],
    only: Option<&str>,
) -> Vec<Value> {
    let words: Vec<String> = only
        .map(newera_core::fold)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    mesh.parts
        .iter()
        .filter(|p| {
            let name = newera_core::fold(&p.name);
            words.iter().all(|w| name.contains(w.as_str()))
        })
        .filter_map(|p| {
            let (min, max) = mesh.part_bounds(p)?;
            // Mesh axes: x width, y up, z depth (front +z).
            let at = |v: [f32; 3]| {
                [
                    cm(f64::from(v[0])),
                    cm(f64::from(v[2])),
                    cm(f64::from(v[1])),
                ]
            };
            let mut materials: Vec<&str> = mesh
                .indices
                .get(p.start * 3..(p.start + p.count) * 3)?
                .iter()
                .filter_map(|&i| mesh.material_of(i as usize).map(|m| m.name.as_str()))
                .collect();
            materials.sort_unstable();
            materials.dedup();
            let mut row = json!([p.name, p.count, at(min), at(max), materials]);
            if hidden.contains(&p.name) {
                row.as_array_mut().expect("row").push(json!("hidden"));
            }
            Some(row)
        })
        .collect()
}

/// Answers `check=clashes`: the parts whose surfaces cross, where and how
/// deep, the faces with no area, and what was and was not looked at.
fn check(out: &mut Value, mesh: &newera_catalog::Mesh, tol: f32) {
    let name = |k: usize| mesh.parts.get(k).map_or("(no part)", |p| p.name.as_str());
    let at = |v: [f32; 3]| {
        [
            cm(f64::from(v[0])),
            cm(f64::from(v[2])),
            cm(f64::from(v[1])),
        ]
    };
    let clashes: Vec<Value> = newera_catalog::clashes(mesh, tol)
        .iter()
        .map(|c| {
            json!([
                name(c.a),
                name(c.b),
                c.pairs,
                at(c.at),
                cm(f64::from(c.depth))
            ])
        })
        .collect();
    let degenerate: Vec<Value> = newera_catalog::degenerate(mesh)
        .into_iter()
        .map(|(k, n)| json!([name(k), n]))
        .collect();
    out["clashes"] = json!(clashes);
    out["degenerate"] = json!(degenerate);
    out["checked"] = json!(format!(
        "surfaces of different parts crossing by more than {tol} cm (touching is not a clash), and faces with no area; not checked: a part wholly inside another, clearances, flipped normals, open meshes{}",
        if mesh.parts.len() < 2 {
            " — this file has one part, so there is nothing to cross"
        } else {
            ""
        }
    ));
}

/// A piece's reference photos, and its product measurements against what
/// is drawn: `[what, product cm, drawn cm, source, confirmed]`, with a
/// warning for each more than a centimeter off.
fn provenance(out: &mut Value, piece: &newera_core::Furniture, drawn: &newera_catalog::Mesh) {
    if !piece.references.is_empty() {
        out["references"] = json!(
            piece
                .references
                .iter()
                .enumerate()
                .map(|(i, r)| json!([i, r.file, r.view, r.part, r.source, r.note]))
                .collect::<Vec<_>>()
        );
    }
    if piece.measures.is_empty() {
        return;
    }
    let value = |what: &str| -> Option<f64> {
        match what {
            "width" => Some(piece.width),
            "depth" => Some(piece.depth),
            "height" => Some(piece.height),
            _ => {
                let part = what.strip_suffix(".height")?;
                let found = drawn.parts.iter().find(|p| p.name == part)?;
                let (_, max) = drawn.part_bounds(found)?;
                Some(piece.elevation + f64::from(max[1]))
            }
        }
    };
    let mut rows = Vec::new();
    for m in &piece.measures {
        let now = value(&m.what).map(cm);
        if let Some(now) = now
            && (now - m.cm).abs() > 1.0
        {
            out["warnings"]
                .as_array_mut()
                .expect("list")
                .push(json!(format!(
                    "{} drawn {now} cm, the product's is {} cm{}",
                    m.what,
                    cm(m.cm),
                    m.source
                        .as_ref()
                        .map(|s| format!(" ({s})"))
                        .unwrap_or_default()
                )));
        }
        rows.push(json!([m.what, m.cm, now, m.source, m.confirmed]));
    }
    out["measures"] = json!(rows);
}

/// Triangles in a model before it is worth a lighter file for afar.
const HEAVY: usize = 50_000;

/// Where a model's triangles go: the parts that weigh most, with their
/// share, heaviest first.
fn cost(mesh: &newera_catalog::Mesh) -> Value {
    let total = mesh.indices.len() / 3;
    let mut parts: Vec<(&str, usize)> = mesh
        .parts
        .iter()
        .map(|p| (p.name.as_str(), p.count))
        .collect();
    parts.sort_by_key(|p| std::cmp::Reverse(p.1));
    #[allow(clippy::cast_precision_loss)]
    let share = |n: usize| ((n as f64 / total.max(1) as f64) * 1000.0).round() / 10.0;
    json!({
        "tris": total,
        "heaviest": parts.iter().take(5).map(|(name, n)| json!([name, n, share(*n)])).collect::<Vec<_>>(),
    })
}

/// What `model` answers for a loaded file.
pub(crate) fn report(file: &str, loaded: &newera_catalog::ImportedModel) -> Value {
    let mesh = &loaded.mesh;
    let mut tris = vec![0usize; mesh.materials.len()];
    for tri in mesh.indices.chunks(3) {
        if let Some(k) = mesh.vertex_materials.get(tri[0] as usize)
            && let Some(n) = tris.get_mut(usize::from(*k))
        {
            *n += 1;
        }
    }
    let materials: Vec<Value> = mesh
        .materials
        .iter()
        .zip(&tris)
        .map(|(m, n)| {
            let image = m
                .texture
                .as_ref()
                .and_then(|t| t.file_name())
                .map(|f| f.to_string_lossy().into_owned());
            json!([m.name, hex(m.color), image, n])
        })
        .collect();
    let images: std::collections::BTreeSet<_> = mesh
        .materials
        .iter()
        .filter_map(|m| m.texture.as_ref())
        .collect();
    let r = &loaded.report;
    json!({
        "file": file,
        "format": r.format,
        "unit": r.unit,
        "guessed": r.guessed,
        "raw": r.raw_size.map(|v| (v * 1000.0).round() / 1000.0),
        "size": loaded.size.map(cm),
        "tris": mesh.indices.len() / 3,
        "meshes": r.meshes,
        "materials": materials,
        "images": images.len(),
        "uv": mesh.uvs.iter().any(|uv| *uv != [0.0, 0.0]),
        "parts": parts(mesh, &[], None),
        "cost": cost(mesh),
        "warnings": r.warnings,
    })
}

#[tool_router(router = model_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        description = "Inspect an imported 3D model, a piece's (id) or a file before placing it: the unit taken (guessed from its size unless unit= or the piece says), natural size, triangles and where they go, materials with their images, named parts (OBJ objects and groups, glTF nodes: arms, seat, frame) with bounds and materials, and warnings for what is not drawn as the file says (a texture or material library not found, normal and roughness maps, extensions). For a piece that looks wrong after place model=…, instead of reading the file. part=<words> keeps the parts named so; check=clashes finds parts passing through one another (a cushion through a rail), not those that touch. Reply {file, format, unit, raw, size cm, tris, materials [[name, color, image, tris]], parts [[name, tris, min, max, materials, \"hidden\" when hidden]] in cm of the piece (x across, y to the front, z up), cost {tris, heaviest [[part, tris, %]], copies, in_project}, warnings}; with id also piece {size, scale, far}; check adds clashes [[part, part, pairs, at, depth cm]], degenerate and checked (what was looked at). place model=… imports one."
    )]
    pub(crate) fn model(
        &self,
        Parameters(p): Parameters<ModelParams>,
    ) -> Result<String, ErrorData> {
        // The lock only to copy out what is read: a heavy file or a clash
        // check must not keep the editor from saving or drawing.
        let (file, piece, path, copies) = {
            let doc = self.document.read();
            let (file, piece) = match (&p.id, &p.file) {
                (Some(raw), None) => {
                    let id = raw
                        .parse::<newera_core::FurnitureId>()
                        .map_err(|e| invalid(format!("id: {e}")))?;
                    let piece = doc
                        .home()
                        .find_piece(id)
                        .ok_or_else(|| invalid(format!("{raw} not found")))?;
                    let model = piece.model.clone().ok_or_else(|| {
                        invalid(format!(
                            "{raw} is built from the catalog (`{}`), not an imported model",
                            piece.catalog
                        ))
                    })?;
                    (model, Some(piece.clone()))
                }
                (None, Some(file)) => (file.clone(), None),
                _ => return Err(invalid("give `id` (a placed piece) or `file`, one of them")),
            };
            let path = doc.resolve_asset(&file);
            let copies = doc
                .home()
                .furniture
                .iter()
                .flat_map(newera_core::Furniture::flatten)
                .filter(|f| f.model.as_deref() == Some(file.as_str()))
                .count();
            (file, piece, path, copies)
        };
        let unit = unit_of(p.unit.as_deref(), piece.as_ref())?;
        let loaded = newera_catalog::load_model_in(&path, unit)
            .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
        let mut out = report(&file, &loaded);
        let tris = loaded.mesh.indices.len() / 3;
        out["cost"]["copies"] = json!(copies);
        out["cost"]["in_project"] = json!(copies * tris);
        if tris > HEAVY && piece.as_ref().is_none_or(|p| p.model_far.is_none()) {
            let top = out["cost"]["heaviest"][0].clone();
            out["warnings"].as_array_mut().expect("list").push(json!(format!(
                "heavy: {tris} triangles{}; a lighter file for afar (edit_model action=lod) keeps a room of them quick",
                top.as_array()
                    .map(|t| format!(", {}% of them in {}", t[2], t[0].as_str().unwrap_or_default()))
                    .unwrap_or_default()
            )));
        }
        // A piece's model as it is drawn: turned, fitted to its box, its
        // parts moved and resized; hidden ones listed, but not checked.
        let (listed, drawn, hidden) = match &piece {
            Some(piece) => {
                let mut fitted = loaded.mesh.clone();
                fitted.rotate(piece.model_transform.rotation);
                fitted.fit_to(piece.width, piece.depth, piece.height);
                let mut listed = fitted.clone();
                listed.edit_parts(&piece.model_parts, false);
                fitted.edit_parts(&piece.model_parts, true);
                let hidden: Vec<String> = piece
                    .model_parts
                    .iter()
                    .filter(|e| e.hidden)
                    .map(|e| e.name.clone())
                    .collect();
                (listed, fitted, hidden)
            }
            None => (loaded.mesh.clone(), loaded.mesh.clone(), Vec::new()),
        };
        out["parts"] = json!(parts(&listed, &hidden, p.part.as_deref()));
        if let Some(piece) = &piece {
            provenance(&mut out, piece, &listed);
        }
        match p.check.as_deref() {
            None => {}
            Some("clashes") => {
                #[allow(clippy::cast_possible_truncation)]
                let tol = p.tol.unwrap_or(0.05).max(0.0) as f32;
                check(&mut out, &drawn, tol);
            }
            Some(other) => return Err(invalid(format!("check `{other}`: clashes"))),
        }
        if let Some(piece) = piece {
            let size = [piece.width, piece.depth, piece.height];
            let scale: Vec<f64> = size
                .iter()
                .zip(loaded.size)
                .map(|(s, natural)| (s / natural * 1000.0).round() / 1000.0)
                .collect();
            out["piece"] =
                json!({ "id": piece.id.to_string(), "size": size.map(cm), "scale": scale });
            if let Some(far) = &piece.model_far {
                out["piece"]["far"] = json!({ "file": far.file, "beyond": far.beyond, "detail": if far.off { "always" } else { "auto" } });
            }
            let (lo, hi) = scale
                .iter()
                .fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
            if hi > lo * 1.02 {
                out["warnings"]
                    .as_array_mut()
                    .expect("list")
                    .push(json!(format!(
                        "stretched unevenly (scale {scale:?} for width, depth, height): thicknesses and curves are distorted"
                    )));
            }
        }
        Ok(out.to_string())
    }
}

#[tool_router(router = edit_model_router, vis = "pub(crate)")]
impl NewEraMcp {
    #[tool(
        description = "Change the imported model of placed pieces (ids), one undo step each; rev refuses a change if the plan moved on. replace swaps the file (library:<name>@<v> too; every=true: on every piece using it), keeping id, place, size (size=natural takes the file's), finish, overrides and links; the new file is read first, and the reply gives the version loaded, overrides left matching nothing and what the file leaves out. material changes one material by the name model lists: color, mat (an image), repeat (texture scale: 2 halves it), clear. part hides, moves (offset cm: x across, y to the front, z up) or resizes one part by name — the back cushion alone. lod adds a lighter file drawn beyond a camera distance (beyond cm, default 400); detail=always holds the full one. reference keeps a product photo (file, view, part, source) that render_3d ref= shows beside the model; measure keeps a product size (dimension: width|depth|height|<part>.height, cm, source, confirmed) that model compares with the drawing. model inspects; place imports."
    )]
    pub(crate) fn edit_model(
        &self,
        Parameters(p): Parameters<EditModelParams>,
    ) -> Result<String, ErrorData> {
        match p.action.as_str() {
            "replace" => self.replace_model(&p),
            "material" => self.model_material(p),
            "part" => self.model_part(&p),
            "lod" => self.model_lod(&p),
            "reference" => self.model_reference(&p),
            "measure" => self.model_measure(&p),
            other => Err(invalid(format!(
                "action `{other}`: replace, material, part, lod, reference or measure"
            ))),
        }
    }
}

/// The pieces `ids` names, each built from a model file.
fn model_pieces(
    doc: &newera_core::Document,
    ids: &[String],
) -> Result<Vec<newera_core::Furniture>, ErrorData> {
    let mut out = Vec::new();
    for raw in ids {
        let id = raw
            .parse::<newera_core::FurnitureId>()
            .map_err(|e| invalid(format!("ids: {e}")))?;
        let piece = doc
            .home()
            .find_piece(id)
            .ok_or_else(|| invalid(format!("{raw} not found")))?;
        if piece.model.is_none() {
            return Err(invalid(format!(
                "{raw} is built from the catalog (`{}`), not an imported model",
                piece.catalog
            )));
        }
        if !out
            .iter()
            .any(|t: &newera_core::Furniture| t.id == piece.id)
        {
            out.push(piece.clone());
        }
    }
    if out.is_empty() {
        return Err(invalid("ids: the pieces whose model changes"));
    }
    Ok(out)
}

/// Models read ahead, with no lock held, by path and unit.
struct Preloaded(
    Vec<(
        std::path::PathBuf,
        Option<newera_catalog::Unit>,
        Result<newera_catalog::ImportedModel, String>,
    )>,
);

impl Preloaded {
    /// The model read for `path` in `unit`; `None` when the piece's file
    /// changed since, and it is read again.
    fn get(
        &self,
        path: &std::path::Path,
        unit: Option<newera_catalog::Unit>,
    ) -> Option<Result<&newera_catalog::ImportedModel, &str>> {
        self.0
            .iter()
            .find(|(p, u, _)| p == path && *u == unit)
            .map(|(_, _, loaded)| loaded.as_ref().map_err(String::as_str))
    }
}

impl NewEraMcp {
    /// The models of the pieces `ids`, each read once in the unit `unit_of`
    /// gives, with the plan unlocked: a heavy file must not stop the editor.
    fn preload(
        &self,
        ids: &[String],
        unit_of: impl Fn(&newera_core::Furniture) -> Result<Option<newera_catalog::Unit>, ErrorData>,
    ) -> Result<Preloaded, ErrorData> {
        let wanted = {
            let doc = self.document.read();
            model_pieces(&doc, ids)?
                .iter()
                .map(|piece| {
                    let file = piece.model.as_deref().unwrap_or_default();
                    Ok((doc.resolve_asset(file), unit_of(piece)?))
                })
                .collect::<Result<Vec<_>, ErrorData>>()?
        };
        let mut out = Preloaded(Vec::new());
        for (path, unit) in wanted {
            if out.get(&path, unit).is_none() {
                let loaded = newera_catalog::load_model_in(&path, unit).map_err(|e| e.to_string());
                out.0.push((path, unit, loaded));
            }
        }
        Ok(out)
    }
}

/// Refuses a change prepared on another revision of the plan.
fn at_revision(doc: &newera_core::Document, rev: Option<u64>) -> Result<(), ErrorData> {
    match rev {
        Some(rev) if rev != doc.revision() => Err(invalid(format!(
            "the plan is at rev {} now, not {rev}: read it again (home ids=…) and decide on what is there",
            doc.revision()
        ))),
        _ => Ok(()),
    }
}

/// The named parts of a piece's model, as the file has them.
fn part_names(
    doc: &newera_core::Document,
    piece: &newera_core::Furniture,
) -> Result<Vec<String>, ErrorData> {
    let file = piece.model.clone().unwrap_or_default();
    let path = doc.resolve_asset(&file);
    let loaded = newera_catalog::load_model_in(&path, unit_of(None, Some(piece))?)
        .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
    Ok(loaded.mesh.parts.iter().map(|p| p.name.clone()).collect())
}

/// Refuses a part a piece's model does not have, naming the ones it has.
fn known_part(
    doc: &newera_core::Document,
    piece: &newera_core::Furniture,
    part: &str,
) -> Result<(), ErrorData> {
    let names = part_names(doc, piece)?;
    if names.iter().any(|n| n == part) {
        Ok(())
    } else {
        Err(invalid(format!(
            "{}: no part `{part}` ({})",
            piece.id,
            if names.is_empty() {
                "its model has no named parts".to_owned()
            } else {
                names.join(", ")
            }
        )))
    }
}

impl NewEraMcp {
    fn model_reference(&self, p: &EditModelParams) -> Result<String, ErrorData> {
        let mut doc = self.document.write();
        at_revision(&doc, p.rev)?;
        let pieces = model_pieces(&doc, &p.ids)?;
        let mut commands = Vec::with_capacity(pieces.len());
        for mut piece in pieces {
            if p.clear {
                piece.references.clear();
            } else if let Some(k) = p.remove {
                if k >= piece.references.len() {
                    return Err(invalid(format!(
                        "{} has {} references (0..{})",
                        piece.id,
                        piece.references.len(),
                        piece.references.len().saturating_sub(1)
                    )));
                }
                piece.references.remove(k);
            } else {
                let file = p
                    .file
                    .clone()
                    .ok_or_else(|| invalid("reference: file, the photo (or remove=<i>, clear)"))?;
                if !newera_core::vfs::exists(&doc.resolve_asset(&file)) {
                    return Err(invalid(format!("reference: {file} not found")));
                }
                if let Some(part) = &p.part {
                    known_part(&doc, &piece, part)?;
                }
                piece.references.push(newera_core::Reference {
                    file,
                    view: p.view.clone(),
                    part: p.part.clone(),
                    source: p.source.clone(),
                    note: p.note.clone(),
                });
            }
            commands.push(newera_core::Command::update(piece));
        }
        doc.execute(newera_core::Command::Batch { commands })
            .map_err(core)?;
        Ok(ok(&doc, &p.ids))
    }

    fn model_measure(&self, p: &EditModelParams) -> Result<String, ErrorData> {
        let mut doc = self.document.write();
        at_revision(&doc, p.rev)?;
        let pieces = model_pieces(&doc, &p.ids)?;
        let mut commands = Vec::with_capacity(pieces.len());
        for mut piece in pieces {
            match (&p.dimension, p.clear) {
                (what, true) => piece
                    .measures
                    .retain(|m| what.as_ref().is_some_and(|w| *w != m.what)),
                (None, false) => {
                    return Err(invalid(
                        "measure: dimension (width, depth, height or <part>.height) and cm",
                    ));
                }
                (Some(what), false) => {
                    let cm =
                        p.cm.filter(|v| v.is_finite() && *v > 0.0)
                            .ok_or_else(|| invalid("measure: cm, the product's value, above 0"))?;
                    match what.rsplit_once('.') {
                        None if ["width", "depth", "height"].contains(&what.as_str()) => {}
                        Some((part, "height")) => known_part(&doc, &piece, part)?,
                        _ => {
                            return Err(invalid(format!(
                                "measure `{what}`: width, depth, height or <part>.height"
                            )));
                        }
                    }
                    piece.measures.retain(|m| m.what != *what);
                    piece.measures.push(newera_core::Measure {
                        what: what.clone(),
                        cm,
                        source: p.source.clone(),
                        confirmed: p.confirmed,
                    });
                }
            }
            commands.push(newera_core::Command::update(piece));
        }
        doc.execute(newera_core::Command::Batch { commands })
            .map_err(core)?;
        Ok(ok(&doc, &p.ids))
    }

    fn model_lod(&self, p: &EditModelParams) -> Result<String, ErrorData> {
        let off = match p.detail.as_deref() {
            None => None,
            Some("auto") => Some(false),
            Some("always") => Some(true),
            Some(other) => return Err(invalid(format!("detail `{other}`: auto or always"))),
        };
        if p.beyond.is_some_and(|b| !(b.is_finite() && b >= 0.0)) {
            return Err(invalid("beyond: a distance in cm, 0 or more"));
        }
        if !p.clear && p.file.is_none() && off.is_none() && p.beyond.is_none() {
            return Err(invalid(
                "lod: give file (the lighter one), beyond, detail or clear",
            ));
        }
        // The lighter file is read first, with no lock held, and must be one.
        let far_tris = match &p.file {
            Some(file) => {
                let path = self.document.read().resolve_asset(file);
                let loaded = newera_catalog::load_model(&path).map_err(|e| {
                    invalid(format!("{}: {e} — nothing was changed", path.display()))
                })?;
                Some(loaded.mesh.indices.len() / 3)
            }
            None => None,
        };
        let mut doc = self.document.write();
        at_revision(&doc, p.rev)?;
        let pieces = model_pieces(&doc, &p.ids)?;
        let mut commands = Vec::with_capacity(pieces.len());
        let mut lines = Vec::with_capacity(pieces.len());
        for mut piece in pieces {
            if p.clear {
                piece.model_far = None;
                lines.push(format!("{}: one file at every distance", piece.id));
            } else {
                let mut far = match (piece.model_far.take(), &p.file) {
                    (_, Some(file)) => newera_core::FarModel {
                        file: file.clone(),
                        beyond: 400.0,
                        off: false,
                    },
                    (Some(far), None) => far,
                    (None, None) => {
                        return Err(invalid(format!(
                            "{} has no lighter file yet: file=<the lighter .obj/.glb>",
                            piece.id
                        )));
                    }
                };
                if let Some(beyond) = p.beyond {
                    far.beyond = beyond;
                }
                if let Some(off) = off {
                    far.off = off;
                }
                let mut line = format!(
                    "{}: {} beyond {} cm{}",
                    piece.id,
                    far.file,
                    cm(far.beyond),
                    if far.off {
                        ", but the detailed file always for now"
                    } else {
                        ""
                    }
                );
                if let Some(n) = far_tris {
                    let _ = write!(line, " ({n} triangles)");
                }
                lines.push(line);
                piece.model_far = Some(far);
            }
            commands.push(newera_core::Command::update(piece));
        }
        doc.execute(newera_core::Command::Batch { commands })
            .map_err(core)?;
        Ok(format!("{}\n{}", ok(&doc, &p.ids), lines.join("\n")))
    }

    fn model_part(&self, p: &EditModelParams) -> Result<String, ErrorData> {
        let name = p
            .part
            .clone()
            .ok_or_else(|| invalid("part: which one, by the name `model` lists"))?;
        if !p.clear && p.hide.is_none() && p.offset.is_none() && p.scale.is_none() {
            return Err(invalid("part: give hide, offset, scale or clear"));
        }
        if p.scale
            .is_some_and(|s| s.iter().any(|v| !(v.is_finite() && *v > 0.0)))
        {
            return Err(invalid("scale: three numbers above 0"));
        }
        let preloaded = self.preload(&p.ids, |piece| unit_of(None, Some(piece)))?;
        let mut doc = self.document.write();
        at_revision(&doc, p.rev)?;
        let pieces = model_pieces(&doc, &p.ids)?;
        let mut commands = Vec::with_capacity(pieces.len());
        for mut piece in pieces {
            let file = piece.model.clone().unwrap_or_default();
            let path = doc.resolve_asset(&file);
            let unit = unit_of(None, Some(&piece))?;
            let fresh;
            let loaded = if let Some(loaded) = preloaded.get(&path, unit) {
                loaded.map_err(|e| invalid(format!("{}: {e}", path.display())))?
            } else {
                fresh = newera_catalog::load_model_in(&path, unit)
                    .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
                &fresh
            };
            if !loaded.mesh.parts.iter().any(|q| q.name == name) {
                let names: Vec<&str> = loaded.mesh.parts.iter().map(|q| q.name.as_str()).collect();
                return Err(invalid(format!(
                    "{}: no part `{name}` in {file} ({})",
                    piece.id,
                    if names.is_empty() {
                        "it has no named parts".to_owned()
                    } else {
                        names.join(", ")
                    }
                )));
            }
            let slot = piece.model_parts.iter().position(|q| q.name == name);
            if p.clear {
                if let Some(k) = slot {
                    piece.model_parts.remove(k);
                }
            } else {
                let k = slot.unwrap_or_else(|| {
                    piece.model_parts.push(newera_core::ModelPart {
                        name: name.clone(),
                        ..newera_core::ModelPart::default()
                    });
                    piece.model_parts.len() - 1
                });
                let edit = &mut piece.model_parts[k];
                if let Some(hide) = p.hide {
                    edit.hidden = hide;
                }
                if let Some(offset) = p.offset {
                    edit.offset = offset;
                }
                if let Some(scale) = p.scale {
                    edit.scale = scale
                        .iter()
                        .any(|v| (v - 1.0).abs() > 1e-9)
                        .then_some(scale);
                }
                if *edit
                    == (newera_core::ModelPart {
                        name: name.clone(),
                        ..Default::default()
                    })
                {
                    piece.model_parts.remove(k);
                }
            }
            commands.push(newera_core::Command::update(piece));
        }
        doc.execute(newera_core::Command::Batch { commands })
            .map_err(core)?;
        Ok(ok(&doc, &p.ids))
    }

    fn model_material(&self, p: EditModelParams) -> Result<String, ErrorData> {
        let name = p
            .material
            .ok_or_else(|| invalid("material: which one, by the name `model` lists"))?;
        if !p.clear && p.color.is_none() && p.mat.is_none() && p.repeat.is_none() {
            return Err(invalid("material: give color, mat, repeat or clear"));
        }
        if p.repeat.is_some_and(|r| !(r.is_finite() && r > 0.0)) {
            return Err(invalid("repeat: a number above 0"));
        }
        let texture = match p.mat.as_deref() {
            None => None,
            Some(raw) => {
                let m = crate::edit::material(raw).map_err(invalid)?;
                if m.as_ref().is_some_and(|m| m.image.is_none()) {
                    return Err(invalid(
                        "mat: an image (`img:file 60x60`) or none; a plain color is `color`",
                    ));
                }
                Some(m)
            }
        };
        let preloaded = self.preload(&p.ids, |_| Ok(None))?;
        let mut doc = self.document.write();
        at_revision(&doc, p.rev)?;
        let pieces = model_pieces(&doc, &p.ids)?;
        let mut commands = Vec::with_capacity(pieces.len());
        for mut piece in pieces {
            let file = piece.model.clone().unwrap_or_default();
            let path = doc.resolve_asset(&file);
            let fresh;
            let loaded = if let Some(loaded) = preloaded.get(&path, None) {
                loaded.map_err(|e| invalid(format!("{}: {e}", path.display())))?
            } else {
                fresh = newera_catalog::load_model(&path)
                    .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
                &fresh
            };
            if !loaded.mesh.materials.iter().any(|m| m.name == name) {
                let names: Vec<&str> = loaded
                    .mesh
                    .materials
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect();
                return Err(invalid(format!(
                    "{}: no material `{name}` in {file} ({})",
                    piece.id,
                    names.join(", ")
                )));
            }
            let slot = piece.materials.iter().position(|m| m.name == name);
            if p.clear {
                if let Some(k) = slot {
                    piece.materials.remove(k);
                }
            } else {
                let k = slot.unwrap_or_else(|| {
                    piece.materials.push(newera_core::ModelMaterial {
                        name: name.clone(),
                        key: None,
                        color: None,
                        texture: None,
                        shininess: None,
                        repeat: None,
                    });
                    piece.materials.len() - 1
                });
                let m = &mut piece.materials[k];
                if let Some(c) = p.color {
                    m.color = Some(c);
                }
                if let Some(t) = texture.clone() {
                    m.texture = t;
                }
                if let Some(r) = p.repeat {
                    m.repeat = ((r - 1.0).abs() > 1e-9).then_some(r);
                }
                // An override left with nothing to change is none at all.
                if m.key.is_none()
                    && m.color.is_none()
                    && m.texture.is_none()
                    && m.shininess.is_none()
                    && m.repeat.is_none()
                {
                    piece.materials.remove(k);
                }
            }
            commands.push(newera_core::Command::update(piece));
        }
        let ids = p.ids.clone();
        doc.execute(newera_core::Command::Batch { commands })
            .map_err(core)?;
        Ok(ok(&doc, &ids))
    }

    fn replace_model(&self, p: &EditModelParams) -> Result<String, ErrorData> {
        let file = p
            .file
            .clone()
            .ok_or_else(|| invalid("replace: file, the new .obj/.gltf/.glb"))?;
        // A library version: its file, in the unit it was published with.
        let (file, library_unit) = match super::library::resolve(&file) {
            Some(found) => {
                let (entry, path) = found.map_err(invalid)?;
                (path, entry.unit)
            }
            None => (file, None),
        };
        let natural = match p.size.as_deref() {
            None | Some("keep") => false,
            Some("natural") => true,
            Some(other) => return Err(invalid(format!("size `{other}`: keep or natural"))),
        };
        // The new file is read with no lock held; the plan is locked after.
        let path = self.document.read().resolve_asset(&file);
        let unit = unit_of(p.unit.as_deref().or(library_unit.as_deref()), None)?;
        let loaded = newera_catalog::load_model_in(&path, unit)
            .map_err(|e| invalid(format!("{}: {e} — nothing was replaced", path.display())))?;
        let mut doc = self.document.write();
        at_revision(&doc, p.rev)?;
        let mut targets = model_pieces(&doc, &p.ids)?;
        let home = doc.home();
        if p.every {
            let files: Vec<Option<String>> = targets.iter().map(|t| t.model.clone()).collect();
            for piece in home
                .furniture
                .iter()
                .flat_map(newera_core::Furniture::flatten)
            {
                if files.contains(&piece.model) && !targets.iter().any(|t| t.id == piece.id) {
                    targets.push(piece.clone());
                }
            }
        }
        // The materials some face uses: a library can define more.
        let used: std::collections::BTreeSet<u16> =
            loaded.mesh.vertex_materials.iter().copied().collect();
        let names: Vec<&str> = loaded
            .mesh
            .materials
            .iter()
            .enumerate()
            .filter(|(k, _)| u16::try_from(*k).is_ok_and(|k| used.contains(&k)))
            .map(|(_, m)| m.name.as_str())
            .collect();
        let stem = |file: &str| {
            std::path::Path::new(file)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        };
        let version = fingerprint(&path).unwrap_or_default();
        let mut commands = Vec::with_capacity(targets.len());
        let mut lines = Vec::with_capacity(targets.len());
        for mut piece in targets {
            let old = piece.model.replace(file.clone()).unwrap_or_default();
            // A name that was only the old file's is the new file's now.
            if stem(&old).as_deref() == Some(piece.name.as_str())
                && let Some(new) = stem(&file)
            {
                piece.name = new;
            }
            // The unit belongs to the file: a new file keeps none it was not given.
            match unit {
                Some(u) => {
                    piece
                        .properties
                        .insert(crate::edit::MODEL_UNIT_KEY.into(), u.name().into());
                }
                None => {
                    piece.properties.remove(crate::edit::MODEL_UNIT_KEY);
                }
            }
            if natural {
                piece.width = loaded.size[0];
                piece.depth = loaded.size[1];
                piece.height = loaded.size[2];
            }
            let mut line = format!("{}: {old} → {file} ({version})", piece.id);
            let orphans: Vec<&str> = piece
                .materials
                .iter()
                .map(|m| m.name.as_str())
                .filter(|n| !names.contains(n))
                .collect();
            if !orphans.is_empty() {
                let _ = write!(
                    line,
                    "; overrides matching no material now: {}",
                    orphans.join(", ")
                );
            }
            let scale = [piece.width, piece.depth, piece.height]
                .iter()
                .zip(loaded.size)
                .map(|(s, n)| s / n)
                .collect::<Vec<_>>();
            let (lo, hi) = scale
                .iter()
                .fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
            if hi > lo * 1.02 {
                let _ = write!(
                    line,
                    "; the kept size {:?} stretches the file's {:?} unevenly",
                    [piece.width, piece.depth, piece.height].map(cm),
                    loaded.size.map(cm)
                );
            }
            if !loaded.report.warnings.is_empty() {
                let _ = write!(line, "; not drawn: {}", loaded.report.warnings.join("; "));
            }
            lines.push(line);
            commands.push(newera_core::Command::update(piece));
        }
        doc.execute(newera_core::Command::Batch { commands })
            .map_err(core)?;
        let ids: Vec<String> = lines
            .iter()
            .filter_map(|l| l.split(':').next().map(str::to_owned))
            .collect();
        Ok(format!("{}\n{}", ok(&doc, &ids), lines.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::server;

    /// A file's path to write inside JSON: `\\` would escape, `/` works everywhere.
    fn path(p: &std::path::Path) -> String {
        p.display().to_string().replace('\\', "/")
    }

    fn call(s: &NewEraMcp, json: &str) -> Result<Value, ErrorData> {
        s.model(Parameters(serde_json::from_str(json).unwrap()))
            .map(|r| serde_json::from_str(&r).unwrap())
    }

    #[test]
    fn a_model_says_what_it_kept_and_what_it_left_out() {
        let dir = std::env::temp_dir().join(format!("newera-model-tool-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("chair.obj"),
            "mtllib chair.mtl\nv 0 0 0\nv 0.5 0 0\nv 0.5 0.8 0\nv 0 0 0.4\nvt 0 0\nvt 1 0\nvt 1 1\n\
             usemtl tecido\nf 1/1 2/2 3/3\nusemtl madeira\nf 1 3 4\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("chair.mtl"),
            "newmtl tecido\nKd 0.2 0.4 0.6\nmap_Kd trama.png\nnewmtl madeira\nKd 0.5 0.3 0.1\nmap_Bump veio.png\n",
        )
        .unwrap();
        let file = path(&dir.join("chair.obj"));
        let s = server();
        let seen = call(&s, &format!(r#"{{"file":"{file}"}}"#)).unwrap();
        assert_eq!(seen["unit"], "m", "{seen}");
        assert_eq!(seen["size"], json!([50.0, 40.0, 80.0]), "{seen}");
        assert_eq!(seen["tris"], 2);
        assert_eq!(
            seen["materials"],
            json!([
                ["tecido", "#336699", "trama.png", 1],
                ["madeira", "#804d1a", null, 1]
            ]),
            "{seen}"
        );
        let warnings = seen["warnings"].to_string();
        assert!(
            warnings.contains("texture trama.png of material tecido not found"),
            "{warnings}"
        );
        assert!(
            warnings.contains("normal/bump map of material madeira is not drawn"),
            "{warnings}"
        );

        // Placed, the reply says it too, and the piece answers by id.
        let reply = s
            .place(Parameters(
                serde_json::from_str(&format!(
                    r#"{{"items":[{{"model":"{file}","at":[100,100],"h":100,"stretch":true}}]}}"#
                ))
                .unwrap(),
            ))
            .unwrap();
        let id = reply
            .lines()
            .next()
            .and_then(|l| l.rsplit("ids=").next())
            .unwrap()
            .to_owned();
        assert!(
            reply.contains(&format!("\nimported {id} {file}: 3 warnings"))
                && reply.contains("stretched ×[1.0, 1.0, 1.25]"),
            "{reply}"
        );
        let piece = call(&s, &format!(r#"{{"id":"{id}"}}"#)).unwrap();
        assert_eq!(piece["piece"]["scale"], json!([1.0, 1.0, 1.25]), "{piece}");
        assert!(
            piece["warnings"].to_string().contains("stretched unevenly"),
            "{piece}"
        );

        assert!(call(&s, "{}").is_err());
        s.place(Parameters(
            serde_json::from_str(r#"{"items":[{"cat":"sofa-3","at":[300,300]}]}"#).unwrap(),
        ))
        .unwrap();
        let catalog = call(&s, r#"{"id":"f2"}"#).unwrap_err();
        assert!(
            catalog.message.contains("built from the catalog"),
            "{catalog:?}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_new_version_of_a_file_takes_the_place_of_the_old_one_under_the_same_id() {
        let dir = std::env::temp_dir().join(format!("newera-edit-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let obj = |w: f64, material: &str| {
            format!(
                "mtllib m.mtl\nusemtl {material}\nv 0 0 0\nv {w} 0 0\nv {w} 0.8 0\nv 0 0 0.6\nf 1 2 3\nf 1 3 4\n"
            )
        };
        std::fs::write(
            dir.join("m.mtl"),
            "newmtl tecido\nKd 1 0 0\nnewmtl linho\nKd 0 1 0\n",
        )
        .unwrap();
        std::fs::write(dir.join("win-v3.obj"), obj(0.8, "tecido")).unwrap();
        std::fs::write(dir.join("win-v4.obj"), obj(0.7, "linho")).unwrap();
        std::fs::write(dir.join("broken.obj"), "not a model").unwrap();
        let v3 = path(&dir.join("win-v3.obj"));
        let v4 = path(&dir.join("win-v4.obj"));
        let s = server();
        let place = |json: String| {
            s.place(Parameters(serde_json::from_str(&json).unwrap()))
                .unwrap()
        };
        place(format!(
            r#"{{"items":[{{"model":"{v3}","at":[100,100],"facing":"+x"}},{{"model":"{v3}","at":[300,100]}}]}}"#
        ));
        let piece = |id: &str| {
            let doc = s.document.read();
            doc.home().find_piece(id.parse().unwrap()).unwrap().clone()
        };
        // A material override and some metadata, set between versions.
        {
            let mut changed = piece("f1");
            changed.info.brand = Some("Example Furniture".into());
            changed.materials.push(newera_core::ModelMaterial {
                name: "tecido".into(),
                key: None,
                color: Some([0, 0, 255]),
                texture: None,
                shininess: None,
                repeat: None,
            });
            s.document
                .write()
                .execute(newera_core::Command::update(changed))
                .unwrap();
        }
        let before = piece("f1");
        let rev = s.document.read().revision();
        let edit = |json: String| s.edit_model(Parameters(serde_json::from_str(&json).unwrap()));

        // A file that does not load changes nothing.
        let broken = path(&dir.join("broken.obj"));
        let refused = edit(format!(
            r#"{{"action":"replace","ids":["f1"],"file":"{broken}"}}"#
        ))
        .unwrap_err();
        assert!(
            refused.message.contains("nothing was replaced"),
            "{refused:?}"
        );
        // Nor does one prepared on an older plan.
        let stale = edit(format!(
            r#"{{"action":"replace","ids":["f1"],"file":"{v4}","rev":{}}}"#,
            rev - 1
        ))
        .unwrap_err();
        assert!(stale.message.contains("read it again"), "{stale:?}");
        assert_eq!(piece("f1"), before);

        let reply = edit(format!(
            r#"{{"action":"replace","ids":["f1"],"file":"{v4}","rev":{rev}}}"#
        ))
        .unwrap();
        let after = piece("f1");
        assert_eq!(after.model.as_deref(), Some(v4.as_str()));
        assert_eq!(after.name, "win-v4", "the name was the old file's");
        assert_eq!(
            (
                after.position,
                after.angle,
                after.width,
                after.info.brand.as_deref()
            ),
            (
                before.position,
                before.angle,
                before.width,
                Some("Example Furniture")
            )
        );
        assert!(reply.contains(&format!("f1: {v3} → {v4} (")), "{reply}");
        assert!(
            reply.contains("overrides matching no material now: tecido"),
            "{reply}"
        );
        assert!(reply.contains("stretches the file's"), "{reply}");
        // Only f1: the other copy of v3 stays, and one undo brings v3 back.
        assert_eq!(piece("f2").model.as_deref(), Some(v3.as_str()));
        assert_eq!(s.document.read().home().furniture.len(), 2);
        s.document.write().undo().unwrap();
        assert_eq!(piece("f1"), before);

        // A repeated id is one piece: one change, one line.
        let reply = edit(format!(
            r#"{{"action":"replace","ids":["f1","f1"],"file":"{v4}"}}"#
        ))
        .unwrap();
        assert_eq!(
            reply.matches(&format!("f1: {v3} → {v4}")).count(),
            1,
            "{reply}"
        );
        s.document.write().undo().unwrap();
        assert_eq!(piece("f1"), before);

        // every=true: every piece on the same file, at the new file's size.
        edit(format!(
            r#"{{"action":"replace","ids":["f1"],"file":"{v4}","every":true,"size":"natural"}}"#
        ))
        .unwrap();
        for id in ["f1", "f2"] {
            let p = piece(id);
            assert_eq!(p.model.as_deref(), Some(v4.as_str()));
            assert!((p.width - 70.0).abs() < 1e-3, "{}", p.width);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_arms_and_cushions_answer_by_name_after_the_project_is_reopened() {
        let dir = std::env::temp_dir().join(format!("newera-model-parts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut obj = String::from("mtllib win.mtl\n");
        let mut n = 0;
        for (name, material, x0, x1, z0, z1) in [
            ("braco_esquerdo", "madeira", 0.0, 0.1, 0.0, 0.6),
            ("braco_direito", "madeira", 0.7, 0.8, 0.0, 0.6),
            ("almofada_assento", "tecido", 0.1, 0.7, 0.0, 0.45),
            ("almofada_encosto", "tecido", 0.1, 0.7, 0.45, 0.8),
        ] {
            let _ = write!(
                obj,
                "o {name}\nusemtl {material}\nv {x0} {z0} 0\nv {x1} {z0} 0\nv {x1} {z1} 0.7\nf {} {} {}\n",
                n + 1,
                n + 2,
                n + 3
            );
            n += 3;
        }
        std::fs::write(dir.join("win.obj"), obj).unwrap();
        std::fs::write(
            dir.join("win.mtl"),
            "newmtl madeira\nKd 0.5 0.3 0.1\nnewmtl tecido\nKd 0.2 0.4 0.3\n",
        )
        .unwrap();
        let s = server();
        let file = path(&dir.join("win.obj"));
        s.place(Parameters(
            serde_json::from_str(&format!(r#"{{"items":[{{"model":"{file}","at":[0,0]}}]}}"#))
                .unwrap(),
        ))
        .unwrap();
        let arms = call(&s, r#"{"id":"f1","part":"braco"}"#).unwrap();
        assert_eq!(
            arms["parts"],
            json!([
                [
                    "braco_esquerdo",
                    1,
                    [-40.0, -35.0, 0.0],
                    [-30.0, 35.0, 60.0],
                    ["madeira"]
                ],
                [
                    "braco_direito",
                    1,
                    [30.0, -35.0, 0.0],
                    [40.0, 35.0, 60.0],
                    ["madeira"]
                ]
            ]),
            "{arms}"
        );
        // Saved and opened again, the same names come back.
        let saved = dir.join("win.newera");
        let file_call = |args: Value| crate::call(s.document.clone(), "file", args).unwrap();
        file_call(json!({"action": "save", "path": saved}));
        file_call(json!({"action": "new"}));
        file_call(json!({"action": "open", "path": saved}));
        let back = call(&s, r#"{"id":"f1","part":"almofada"}"#).unwrap();
        let names: Vec<&str> = back["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p[0].as_str().unwrap())
            .collect();
        assert_eq!(names, ["almofada_assento", "almofada_encosto"], "{back}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// OBJ text for axis-aligned boxes, one object each, in cm.
    fn boxes(parts: &[(&str, [f64; 3], [f64; 3])]) -> String {
        let mut out = String::new();
        let mut n = 0;
        for (name, lo, hi) in parts {
            let _ = writeln!(out, "o {name}");
            for k in 0..8 {
                let pick = |axis: usize| {
                    if k >> axis & 1 == 0 {
                        lo[axis]
                    } else {
                        hi[axis]
                    }
                };
                let _ = writeln!(out, "v {} {} {}", pick(0), pick(1), pick(2));
            }
            for face in [
                [0, 2, 3, 1],
                [4, 5, 7, 6],
                [0, 1, 5, 4],
                [2, 6, 7, 3],
                [0, 4, 6, 2],
                [1, 3, 7, 5],
            ] {
                let _ = writeln!(
                    out,
                    "f {} {} {} {}",
                    face[0] + n + 1,
                    face[1] + n + 1,
                    face[2] + n + 1,
                    face[3] + n + 1
                );
            }
            n += 8;
        }
        out
    }

    #[test]
    fn a_cushion_through_a_rail_is_a_clash_and_one_resting_on_it_is_not() {
        let dir = std::env::temp_dir().join(format!("newera-model-clash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Heights up (y), in cm: a 60 × 4 rail, a cushion 3 cm into it, one on top of it.
        std::fs::write(
            dir.join("poltrona.obj"),
            boxes(&[
                ("travessa", [0.0, 40.0, 0.0], [60.0, 44.0, 5.0]),
                ("almofada", [5.0, 41.0, 6.0], [55.0, 50.0, 30.0]),
                ("almofada_apoiada", [5.0, 44.0, -30.0], [55.0, 50.0, 2.0]),
                ("encosto", [0.0, 0.0, 31.0], [60.0, 90.0, 35.0]),
            ])
            .replace("v 5 41 6", "v 5 41 3")
            .replace("v 55 41 6", "v 55 41 3")
            .replace("v 5 50 6", "v 5 50 3")
            .replace("v 55 50 6", "v 55 50 3"),
        )
        .unwrap();
        let file = path(&dir.join("poltrona.obj"));
        let s = server();
        let seen = call(&s, &format!(r#"{{"file":"{file}","check":"clashes"}}"#)).unwrap();
        let clashes = seen["clashes"].as_array().unwrap();
        assert_eq!(clashes.len(), 1, "{seen}");
        assert_eq!(
            (clashes[0][0].as_str(), clashes[0][1].as_str()),
            (Some("travessa"), Some("almofada"))
        );
        assert!(
            seen["checked"].as_str().unwrap().contains("not checked"),
            "{seen}"
        );
        assert_eq!(seen["degenerate"], json!([]));
        // Touching only: a looser tolerance than the 3 cm finds nothing.
        let loose = call(
            &s,
            &format!(r#"{{"file":"{file}","check":"clashes","tol":4}}"#),
        )
        .unwrap();
        assert_eq!(loose["clashes"], json!([]), "{loose}");
        // Placed at twice its size, the crossing is twice as deep, in the piece's cm.
        s.place(Parameters(
            serde_json::from_str(&format!(
                r#"{{"items":[{{"model":"{file}","at":[100,100]}}]}}"#
            ))
            .unwrap(),
        ))
        .unwrap();
        {
            let mut doc = s.document.write();
            let mut piece = doc
                .home()
                .find_piece("f1".parse().unwrap())
                .unwrap()
                .clone();
            piece.width *= 2.0;
            piece.depth *= 2.0;
            piece.height *= 2.0;
            doc.execute(newera_core::Command::update(piece)).unwrap();
        }
        let placed = call(&s, r#"{"id":"f1","check":"clashes"}"#).unwrap();
        let depth = |v: &Value| v["clashes"][0][4].as_f64().unwrap();
        assert!(
            (depth(&placed) - 2.0 * depth(&seen)).abs() < 0.1,
            "{placed}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_small_part_drawn_in_centimeters_comes_in_at_its_size_when_the_unit_is_said() {
        let dir = std::env::temp_dir().join(format!("newera-model-unit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A knob 5 × 4 × 3 cm, its numbers in centimeters.
        std::fs::write(
            dir.join("puxador.obj"),
            boxes(&[("puxador", [0.0, 0.0, 0.0], [5.0, 3.0, 4.0])]),
        )
        .unwrap();
        let file = path(&dir.join("puxador.obj"));
        let s = server();
        let place = |item: &str| {
            s.place(Parameters(
                serde_json::from_str(&format!(
                    r#"{{"items":[{{"model":"{file}","at":[0,0]{item}}}]}}"#
                ))
                .unwrap(),
            ))
        };
        let size = |id: &str| {
            let doc = s.document.read();
            let p = doc.home().find_piece(id.parse().unwrap()).unwrap();
            // Whole centimeters: every size here is one.
            #[allow(clippy::cast_possible_truncation)]
            [p.width, p.depth, p.height].map(|v| v.round() as i64)
        };
        // Guessed, it is 5 m across, and the reply says so.
        let guessed = place("").unwrap();
        assert_eq!(size("f1"), [500, 400, 300]);
        assert!(
            guessed.contains("unit guessed as m, so it is 500 cm across"),
            "{guessed}"
        );
        // Said, it is 5 cm, and `model` reads the piece in the same unit.
        let said = place(r#","unit":"cm""#).unwrap();
        assert_eq!(size("f2"), [5, 4, 3]);
        assert!(!said.contains("imported"), "{said}");
        let seen = call(&s, r#"{"id":"f2"}"#).unwrap();
        assert_eq!(
            (seen["unit"].as_str(), seen["guessed"].as_bool()),
            (Some("cm"), Some(false))
        );
        assert_eq!(seen["piece"]["scale"], json!([1.0, 1.0, 1.0]), "{seen}");
        // One size scales it evenly; two that disagree are refused; stretch lets them.
        place(r#","unit":"cm","h":6"#).unwrap();
        assert_eq!(size("f3"), [10, 8, 6]);
        let refused = place(r#","unit":"cm","w":10,"h":3"#).unwrap_err();
        assert!(refused.message.contains("stretch=true"), "{refused:?}");
        let stretched = place(r#","unit":"cm","w":10,"h":3,"stretch":true"#).unwrap();
        assert!(
            stretched.contains("stretched ×[2.0, 1.0, 1.0]"),
            "{stretched}"
        );
        let catalog = s
            .place(Parameters(
                serde_json::from_str(r#"{"items":[{"cat":"sofa-3","at":[0,0],"unit":"cm"}]}"#)
                    .unwrap(),
            ))
            .unwrap_err();
        assert!(catalog.message.contains("for model="), "{catalog:?}");
        let beam = s
            .place(Parameters(
                serde_json::from_str(
                    r#"{"items":[{"cat":"beam","a":[0,0,250],"b":[300,0,250],"stretch":true}]}"#,
                )
                .unwrap(),
            ))
            .unwrap_err();
        assert!(beam.message.contains("for model="), "{beam:?}");
        // With the file gone, the same path repeats a piece: its unit stays that piece's.
        std::fs::remove_file(dir.join("puxador.obj")).unwrap();
        let repeated = place(r#","unit":"m""#).unwrap_err();
        assert!(repeated.message.contains("this repeats f1"), "{repeated:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_material_changes_by_name_and_its_weave_by_repeat() {
        let dir =
            std::env::temp_dir().join(format!("newera-model-material-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("win.obj"),
            "mtllib win.mtl\nusemtl tecido\nv 0 0 0\nv 0.6 0 0\nv 0.6 0.8 0.5\nf 1 2 3\n",
        )
        .unwrap();
        std::fs::write(dir.join("win.mtl"), "newmtl tecido\nKd 0.2 0.4 0.3\n").unwrap();
        let file = path(&dir.join("win.obj"));
        let s = server();
        s.place(Parameters(
            serde_json::from_str(&format!(r#"{{"items":[{{"model":"{file}","at":[0,0]}}]}}"#))
                .unwrap(),
        ))
        .unwrap();
        let edit = |json: &str| s.edit_model(Parameters(serde_json::from_str(json).unwrap()));
        let overrides = || {
            let doc = s.document.read();
            doc.home()
                .find_piece("f1".parse().unwrap())
                .unwrap()
                .materials
                .clone()
        };
        let size = || {
            let doc = s.document.read();
            let p = doc.home().find_piece("f1".parse().unwrap()).unwrap();
            format!("{:.1} {:.1} {:.1}", p.width, p.depth, p.height)
        };
        let before = size();
        edit(r#"{"action":"material","ids":["f1"],"material":"tecido","color":[20,60,40],"repeat":2}"#).unwrap();
        let set = overrides();
        assert_eq!(
            (set.len(), set[0].color, set[0].repeat),
            (1, Some([20, 60, 40]), Some(2.0))
        );
        assert_eq!(size(), before, "the weave changes, the chair does not");
        edit(r#"{"action":"material","ids":["f1"],"material":"tecido","mat":"img:trama.png 5x5"}"#)
            .unwrap();
        assert!(
            overrides()[0]
                .texture
                .as_ref()
                .is_some_and(|t| t.image.as_deref() == Some("trama.png"))
        );
        assert_eq!(
            overrides()[0].color,
            Some([20, 60, 40]),
            "what was not given stays"
        );
        let unknown =
            edit(r#"{"action":"material","ids":["f1"],"material":"couro","color":[0,0,0]}"#)
                .unwrap_err();
        assert!(
            unknown.message.contains("no material `couro`") && unknown.message.contains("tecido"),
            "{unknown:?}"
        );
        let pattern =
            edit(r##"{"action":"material","ids":["f1"],"material":"tecido","mat":"#ff0000"}"##)
                .unwrap_err();
        assert!(pattern.message.contains("an image"), "{pattern:?}");
        edit(r#"{"action":"material","ids":["f1"],"material":"tecido","clear":true}"#).unwrap();
        assert_eq!(overrides(), [] as [newera_core::ModelMaterial; 0]);
        // Back to repeat 1 with nothing else set, no override stays behind.
        edit(r#"{"action":"material","ids":["f1"],"material":"tecido","repeat":2}"#).unwrap();
        edit(r#"{"action":"material","ids":["f1"],"material":"tecido","repeat":1}"#).unwrap();
        assert!(overrides().is_empty(), "{:?}", overrides());
        edit(r#"{"action":"material","ids":["f1"],"material":"tecido","repeat":1}"#).unwrap();
        assert!(overrides().is_empty(), "{:?}", overrides());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_back_cushion_moves_back_and_the_arms_stay_where_they_are() {
        let dir = std::env::temp_dir().join(format!("newera-model-part-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // x across, y up, z toward the front, cm: arms, seat, back cushion.
        std::fs::write(
            dir.join("win.obj"),
            boxes(&[
                ("braco_esquerdo", [0.0, 0.0, 0.0], [10.0, 60.0, 80.0]),
                ("braco_direito", [70.0, 0.0, 0.0], [80.0, 60.0, 80.0]),
                ("assento", [10.0, 0.0, 10.0], [70.0, 45.0, 80.0]),
                ("almofada_encosto", [10.0, 45.0, 10.0], [70.0, 85.0, 25.0]),
            ]),
        )
        .unwrap();
        let file = path(&dir.join("win.obj"));
        let s = server();
        s.place(Parameters(
            serde_json::from_str(&format!(
                r#"{{"items":[{{"model":"{file}","at":[0,0],"unit":"cm"}}]}}"#
            ))
            .unwrap(),
        ))
        .unwrap();
        let edit = |json: &str| s.edit_model(Parameters(serde_json::from_str(json).unwrap()));
        let part = |name: &str| {
            let seen = call(&s, &format!(r#"{{"id":"f1","part":"{name}"}}"#)).unwrap();
            seen["parts"][0].clone()
        };
        let arm = part("braco_direito");
        let back = part("almofada_encosto");
        edit(r#"{"action":"part","ids":["f1"],"part":"almofada_encosto","offset":[0,-5,0]}"#)
            .unwrap();
        let moved = part("almofada_encosto");
        let front = |row: &Value| row[2][1].as_f64().unwrap();
        assert!(
            (front(&moved) - (front(&back) - 5.0)).abs() < 1e-6,
            "{moved}"
        );
        assert_eq!(part("braco_direito"), arm, "the arms stay");
        // Seat and arm heights read on their own, whatever the total height.
        assert_eq!(part("assento")[3][2], json!(45.0));
        assert_eq!(arm[3][2], json!(60.0));

        edit(r#"{"action":"part","ids":["f1"],"part":"braco_direito","hide":true}"#).unwrap();
        assert_eq!(part("braco_direito")[5], json!("hidden"));
        let unknown =
            edit(r#"{"action":"part","ids":["f1"],"part":"pe","hide":true}"#).unwrap_err();
        assert!(
            unknown.message.contains("no part `pe`") && unknown.message.contains("assento"),
            "{unknown:?}"
        );
        edit(r#"{"action":"part","ids":["f1"],"part":"almofada_encosto","clear":true}"#).unwrap();
        edit(r#"{"action":"part","ids":["f1"],"part":"braco_direito","hide":false}"#).unwrap();
        assert_eq!(part("almofada_encosto"), back);
        let doc = s.document.read();
        assert_eq!(
            doc.home()
                .find_piece("f1".parse().unwrap())
                .unwrap()
                .model_parts,
            [] as [newera_core::ModelPart; 0]
        );
        drop(doc);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_heavy_model_says_where_its_triangles_go_and_takes_a_lighter_file_for_afar() {
        let dir = std::env::temp_dir().join(format!("newera-model-lod-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A frame of 12 triangles and a weave of 60,000.
        let mut obj = boxes(&[("moldura", [0.0, 0.0, 0.0], [47.0, 81.0, 5.0])]);
        obj.push_str("o palhinha\n");
        let n = 100;
        for i in 0..=n {
            for j in 0..=n / 2 {
                let _ = writeln!(obj, "v {} {} 2", f64::from(i) * 0.4, f64::from(j) * 0.8);
            }
        }
        let row = n / 2 + 1;
        let mut faces = 0;
        'grid: for i in 0..n {
            for j in 0..n / 2 {
                let a = 8 + i * row + j + 1;
                let _ = writeln!(obj, "f {} {} {}", a, a + row, a + row + 1);
                let _ = writeln!(obj, "f {} {} {}", a, a + row + 1, a + 1);
                faces += 2;
                if faces >= 60_000 {
                    break 'grid;
                }
            }
        }
        // Six layers of the same grid make it heavy.
        let grid = obj.lines().filter(|l| l.starts_with("f ")).skip(12).fold(
            String::new(),
            |mut out, l| {
                let _ = writeln!(out, "{l}");
                out
            },
        );
        for _ in 0..5 {
            obj.push_str(&grid);
        }
        std::fs::write(dir.join("ares.obj"), obj).unwrap();
        std::fs::write(
            dir.join("ares-leve.obj"),
            boxes(&[("moldura", [0.0, 0.0, 0.0], [47.0, 81.0, 5.0])]),
        )
        .unwrap();
        let file = path(&dir.join("ares.obj"));
        let light = path(&dir.join("ares-leve.obj"));
        let s = server();
        s.place(Parameters(
            serde_json::from_str(&format!(r#"{{"items":[{{"model":"{file}","at":[0,0],"unit":"cm"}},{{"model":"{file}","at":[100,0],"unit":"cm"}}]}}"#)).unwrap(),
        ))
        .unwrap();
        let seen = call(&s, r#"{"id":"f1"}"#).unwrap();
        let cost = &seen["cost"];
        let tris = cost["tris"].as_u64().unwrap();
        assert!(tris > 50_000, "{cost}");
        assert_eq!(cost["heaviest"][0][0], "palhinha", "{cost}");
        assert!(cost["heaviest"][0][2].as_f64().unwrap() > 99.0, "{cost}");
        assert_eq!(
            (cost["copies"].as_u64(), cost["in_project"].as_u64()),
            (Some(2), Some(2 * tris))
        );
        assert!(seen["warnings"].to_string().contains("heavy"), "{seen}");

        let edit = |json: String| s.edit_model(Parameters(serde_json::from_str(&json).unwrap()));
        assert!(
            edit(r#"{"action":"lod","ids":["f1"],"beyond":300}"#.to_owned())
                .unwrap_err()
                .message
                .contains("no lighter file yet")
        );
        let reply = edit(format!(
            r#"{{"action":"lod","ids":["f1","f2"],"file":"{light}","beyond":350}}"#
        ))
        .unwrap();
        assert!(reply.contains("beyond 350 cm (12 triangles)"), "{reply}");
        edit(r#"{"action":"lod","ids":["f1"],"detail":"always"}"#.to_owned()).unwrap();
        let held = call(&s, r#"{"id":"f1"}"#).unwrap();
        assert_eq!(
            held["piece"]["far"],
            json!({"file": light, "beyond": 350.0, "detail": "always"})
        );
        assert!(!held["warnings"].to_string().contains("heavy"), "{held}");
        edit(r#"{"action":"lod","ids":["f1"],"clear":true}"#.to_owned()).unwrap();
        let doc = s.document.read();
        let far = |id: &str| {
            doc.home()
                .find_piece(id.parse().unwrap())
                .unwrap()
                .model_far
                .clone()
        };
        assert!(far("f1").is_none());
        assert_eq!(far("f2").map(|f| f.beyond), Some(350.0));
        drop(doc);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn photos_and_measurements_of_the_product_stay_with_the_piece_and_check_it() {
        let dir = std::env::temp_dir().join(format!("newera-model-refs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cadeira.obj"),
            boxes(&[
                ("perna", [0.0, 0.0, 0.0], [4.0, 40.0, 4.0]),
                ("assento", [0.0, 40.0, 0.0], [47.0, 47.0, 50.0]),
                ("encosto", [0.0, 47.0, 0.0], [47.0, 81.0, 4.0]),
            ]),
        )
        .unwrap();
        image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]))
            .save(dir.join("frente.png"))
            .unwrap();
        let file = path(&dir.join("cadeira.obj"));
        let photo = path(&dir.join("frente.png"));
        let s = server();
        s.place(Parameters(
            serde_json::from_str(&format!(
                r#"{{"items":[{{"model":"{file}","at":[0,0],"unit":"cm"}}]}}"#
            ))
            .unwrap(),
        ))
        .unwrap();
        let edit = |json: String| s.edit_model(Parameters(serde_json::from_str(&json).unwrap()));
        let page = "https://example.com/cadeira-ares";
        edit(format!(
            r#"{{"action":"reference","ids":["f1"],"file":"{photo}","view":"front","source":"{page}","note":"frente"}}"#
        ))
        .unwrap();
        assert!(
            edit(r#"{"action":"reference","ids":["f1"],"file":"nada.png"}"#.to_owned()).is_err()
        );
        assert!(
            edit(format!(
                r#"{{"action":"reference","ids":["f1"],"file":"{photo}","part":"braco"}}"#
            ))
            .unwrap_err()
            .message
            .contains("assento")
        );
        for (what, value) in [("height", 81.0), ("assento.height", 49.5)] {
            edit(format!(
                r#"{{"action":"measure","ids":["f1"],"dimension":"{what}","cm":{value},"source":"{page}","confirmed":true}}"#
            ))
            .unwrap();
        }
        assert!(
            edit(r#"{"action":"measure","ids":["f1"],"dimension":"altura","cm":3}"#.to_owned())
                .is_err()
        );
        let seen = call(&s, r#"{"id":"f1"}"#).unwrap();
        assert_eq!(
            seen["references"][0],
            json!([0, photo, "front", null, page, "frente"])
        );
        assert_eq!(
            seen["measures"],
            json!([
                ["height", 81.0, 81.0, page, true],
                ["assento.height", 49.5, 47.0, page, true]
            ])
        );
        assert!(
            seen["warnings"]
                .to_string()
                .contains("assento.height drawn 47 cm, the product's is 49.5 cm"),
            "{seen}"
        );
        // Side by side: the photo, scaled to the render's height, then the render.
        let result = s
            .render_3d(Parameters(
                serde_json::from_str(r#"{"piece":"f1","ref":0,"w":96,"h":72}"#).unwrap(),
            ))
            .unwrap();
        let rmcp::model::ContentBlock::Image(image) = &result.content[0] else {
            panic!("expected image")
        };
        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &image.data)
            .unwrap();
        let both = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(both.dimensions(), (144 + 96, 72));
        assert_eq!(both.get_pixel(10, 10).0, [200, 30, 30, 255]);
        // Saved and opened again, they are still there.
        let saved = dir.join("estudo.newera");
        let file_call = |args: Value| crate::call(s.document.clone(), "file", args).unwrap();
        file_call(json!({"action": "save", "path": saved}));
        file_call(json!({"action": "new"}));
        file_call(json!({"action": "open", "path": saved}));
        let back = call(&s, r#"{"id":"f1"}"#).unwrap();
        assert_eq!(back["measures"].as_array().unwrap().len(), 2, "{back}");
        let kept = back["references"][0][1].as_str().unwrap().to_owned();
        let doc = s.document.read();
        assert!(
            newera_core::vfs::exists(&doc.resolve_asset(&kept)),
            "{kept}"
        );
        drop(doc);
        edit(r#"{"action":"reference","ids":["f1"],"remove":0}"#.to_owned()).unwrap();
        edit(r#"{"action":"measure","ids":["f1"],"dimension":"height","clear":true}"#.to_owned())
            .unwrap();
        let after = call(&s, r#"{"id":"f1"}"#).unwrap();
        assert!(after.get("references").is_none(), "{after}");
        assert_eq!(after["measures"].as_array().unwrap().len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
