//! What each tool answers, as data a client can check.
//!
//! The tools answer in text an agent reads: compact JSON for reads, a short
//! `ok rev=N ids=…` line for writes, an image for renders. A client that
//! wants fields instead of words finds them in `structuredContent`, and the
//! tool's `outputSchema` says what they are.
//!
//! Nothing here changes what the tools say. [`structure`] reads the answer
//! the way an agent would and lays it out as an object — a JSON object as it
//! is, a JSON array under `rows`, an `ok` line split into its fields with the
//! line itself kept in `text`, an image as its type — and [`schema`] names
//! the fields each tool can answer with. One table, applied where the router
//! is assembled and where the hosted service answers, so every transport
//! hands out the same contract.
//!
//! A client that checks the answer against the schema refuses it when a
//! field the schema calls required is missing, so `required` lists only what
//! every successful answer carries; the rest is described, not demanded.
//! `every_answer_fits_its_schema` holds the two together.

use serde_json::{Map, Value, json};

/// What the server is, in the handshake: directories show it beside the name.
pub const DESCRIPTION: &str =
    "Design homes with your AI: floor plans, furniture, joinery, lighting and photos, live.";

/// Where people read about it.
pub const WEBSITE: &str = "https://3dneweraai.com";

/// The browser editor an answer links to when nothing says another: the
/// project's own site. A hosted service names its own (`NEWERA_EDITOR_URL`).
pub const EDITOR_URL: &str = "https://3dneweraai.com/app/";

/// The server's icons, as the site serves them.
#[must_use]
pub fn icons() -> Vec<rmcp::model::Icon> {
    vec![
        rmcp::model::Icon::new(format!("{WEBSITE}/icon-256.png"))
            .with_mime_type("image/png")
            .with_sizes(vec!["256x256".to_owned()]),
        rmcp::model::Icon::new(format!("{WEBSITE}/favicon.svg"))
            .with_mime_type("image/svg+xml")
            .with_sizes(vec!["any".to_owned()]),
    ]
}

/// The fields an `ok` line can carry, in the order a reply writes them.
/// Counts are integers; the rest are comma-separated lists.
const COUNTS: [&str; 6] = ["rev", "v", "i", "fitted", "accepted", "removed"];
const LISTS: [&str; 3] = ["faces", "turned", "ids"];

/// Lays a successful answer out as an object, unless it already has one.
/// Refusals (`isError`) stay as they are: a schema describes what a tool
/// gives, not how it declines.
pub fn structure(result: &mut rmcp::model::CallToolResult) {
    if result.is_error == Some(true) || result.structured_content.is_some() {
        return;
    }
    let content = serde_json::to_value(&result.content).unwrap_or(Value::Null);
    result.structured_content = from_content(&content);
}

/// The same, for an answer already turned into JSON (the hosted service,
/// and a browser tab behind the relay, hand them around that way).
pub fn structure_json(result: &mut Value) {
    if result.get("isError") == Some(&Value::Bool(true))
        || result
            .get("structuredContent")
            .is_some_and(|s| !s.is_null())
    {
        return;
    }
    if let Some(structured) = result.get("content").and_then(from_content)
        && let Some(object) = result.as_object_mut()
    {
        object.insert("structuredContent".to_owned(), structured);
    }
}

fn from_content(content: &Value) -> Option<Value> {
    let blocks = content.as_array()?;
    let text: Vec<&str> = blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    if text.is_empty() {
        let image = blocks.iter().find(|b| b["type"] == "image")?;
        return Some(json!({"mimeType": image["mimeType"]}));
    }
    Some(from_text(text.join("\n").trim()))
}

/// An answer in words, as fields.
pub(crate) fn from_text(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(object @ Value::Object(_)) => return object,
        Ok(Value::Array(rows)) => return json!({"rows": rows}),
        _ => {}
    }
    if text == "ok" || text.starts_with("ok ") {
        return written(text);
    }
    // One object per line: `get_home ndjson=true`.
    let lines: Option<Vec<Value>> = text
        .lines()
        .map(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .filter(Value::is_object)
        })
        .collect();
    match lines {
        Some(lines) if lines.len() > 1 => json!({"lines": lines}),
        _ => json!({"text": text}),
    }
}

/// `ok rev=3 v=1 faces=f3:+y(seat) ids=f3 {"changed":[…]}` → its fields.
///
/// A value runs to the next known field: `faces` and `turned` hold spaces
/// (`f3:back to w2`). A JSON object at the end of the first line is a diff,
/// and its keys join the fields. The whole reply stays in `text`, so a line
/// that says more than these fields (a saved path, an import's warnings)
/// loses nothing.
fn written(text: &str) -> Value {
    let mut fields = Map::new();
    fields.insert("ok".to_owned(), Value::Bool(true));
    let first = text.lines().next().unwrap_or_default();
    let mut head = first;
    if let Some(at) = first.find(" {")
        && let Ok(Value::Object(diff)) = serde_json::from_str::<Value>(&first[at + 1..])
    {
        fields.extend(diff);
        head = &first[..at];
    }
    let mut marks: Vec<(usize, &str)> = COUNTS
        .iter()
        .chain(LISTS.iter())
        .flat_map(|key| {
            let needle = format!(" {key}=");
            head.match_indices(&needle)
                .map(|(at, _)| (at, *key))
                .collect::<Vec<_>>()
        })
        .collect();
    marks.sort_unstable();
    for (n, (at, key)) in marks.iter().enumerate() {
        let from = at + key.len() + 2;
        let to = marks.get(n + 1).map_or(head.len(), |(next, _)| *next);
        let value = head[from..to].trim();
        if COUNTS.contains(key) {
            if let Ok(count) = value.parse::<u64>() {
                fields.insert((*key).to_owned(), json!(count));
            }
        } else {
            let items: Vec<&str> = value.split(',').filter(|s| !s.is_empty()).collect();
            fields.insert((*key).to_owned(), json!(items));
        }
    }
    fields.insert("text".to_owned(), Value::String(text.to_owned()));
    Value::Object(fields)
}

// ---------------------------------------------------------------------------
// Schemas.

fn t(kind: &str, description: &str) -> Value {
    json!({"type": kind, "description": description})
}
fn num(description: &str) -> Value {
    t("number", description)
}
fn int(description: &str) -> Value {
    t("integer", description)
}
fn text(description: &str) -> Value {
    t("string", description)
}
fn flag(description: &str) -> Value {
    t("boolean", description)
}
fn list(description: &str) -> Value {
    t("array", description)
}
fn object(description: &str) -> Value {
    t("object", description)
}
fn strings(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": description})
}

/// What a cut list answers, read or written.
fn cut_list_fields() -> Vec<(&'static str, Value)> {
    vec![
        (
            "rows",
            list("[part, board, qty, length, width, thickness, edge, cutouts?]"),
        ),
        ("hardware", strings("Hardware by build: \"<id>: <item>\"")),
        sources(),
        ("drawn", strings("Hand-drawn builds left out")),
        ("skipped", list("[id, name] left out")),
        ("sheets", list(".dxf/.svg: [board, sheets]")),
    ]
}

/// An object schema: its fields, and the ones every answer has.
fn shape(fields: &[(&str, Value)], required: &[&str]) -> Value {
    let properties: Map<String, Value> = fields
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    if required.is_empty() {
        json!({"type": "object", "properties": properties})
    } else {
        json!({"type": "object", "properties": properties, "required": required})
    }
}

/// The fields of an `ok` line.
fn ok_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("ok", flag("Always true")),
        ("rev", int("Plan revision")),
        ("v", int("Plan version written to, when there are several")),
        ("ids", strings("Ids created or touched")),
        ("text", text("The reply as written")),
    ]
}

/// What an undoable change reports it changed (`compact::diff`); past 20
/// entries a list becomes its count.
fn diff_fields() -> Vec<(&'static str, Value)> {
    vec![
        (
            "changed",
            json!({"type": ["array", "integer"], "description": "{id, from, to} per element changed; a count past 20"}),
        ),
        (
            "added",
            json!({"type": ["array", "integer"], "description": "Ids added, or their count"}),
        ),
        (
            "gone",
            json!({"type": ["array", "integer"], "description": "Ids removed, or their count"}),
        ),
        ("annotations", object("Annotation switches {from, to}")),
        ("properties", strings("Project properties changed")),
    ]
}

/// What `dry` answers instead of writing (`reply::preview_with`).
fn dry_fields() -> Vec<(&'static str, Value)> {
    let mut fields = diff_fields();
    fields.extend([
        ("dry", flag("Nothing was written")),
        ("score_scope", object("Disciplines scored")),
        (
            "clearances",
            object("Free floor around touched pieces, by id"),
        ),
        ("issues_resolved", list("Layout issues settled")),
        ("issues_new", list("Layout issues created")),
        ("issues_changed", list("Layout issues resized")),
        ("score", list("Score [before, after]")),
        ("resolved", list("Findings settled")),
        ("new_findings", list("Findings created")),
        ("findings_changed", list("Findings changed")),
        (
            "acceptance_cleanup",
            object("Acceptances orphaned, and the calls that clear them"),
        ),
        ("changed_count", int("Roots changed")),
        ("added_count", int("Roots added")),
    ]);
    fields
}

fn ok_line(extra: &[(&'static str, Value)]) -> Value {
    let mut fields = ok_fields();
    fields.extend(extra.iter().cloned());
    shape(&fields, &["ok", "text"])
}

/// A write answering `ok …` with a diff, or a dry run answering the preview.
fn write_or_dry(extra: &[(&'static str, Value)]) -> Value {
    let mut fields = ok_fields();
    fields.extend(dry_fields());
    fields.extend(extra.iter().cloned());
    shape(&fields, &[])
}

fn image() -> Value {
    shape(
        &[(
            "mimeType",
            text("Type of the image in the content (image/png)"),
        )],
        &["mimeType"],
    )
}

fn sources() -> (&'static str, Value) {
    (
        "sources",
        object("Standards cited, by code: [title, tier, url]"),
    )
}

fn findings_rows() -> (&'static str, Value) {
    (
        "findings",
        list("Rows [severity erro|alerta|dica, place, message, source, key, accepted reason?]"),
    )
}

fn orphaned() -> (&'static str, Value) {
    (
        "orphaned",
        list("Acceptances whose finding is gone: [key, reason, successor key?]"),
    )
}

fn issues() -> Vec<(&'static str, Value)> {
    vec![
        (
            "overlap",
            list("Pieces that overlap: {key, a, b, kind, extent}"),
        ),
        (
            "blocked",
            list("Pieces whose use is blocked: {key, piece, against, cm}"),
        ),
        (
            "backwards",
            list("Pieces turned to face a wall: {key, piece, wall, cm, fix}"),
        ),
        ("in_wall", list("Pieces inside a wall: {key, piece, wall}")),
        ("blocks_door", list("Doors blocked: {key, door, by}")),
        (
            "blocks_window",
            list("Windows blocked: {key, window, by, extent}"),
        ),
        ("outside_rooms", list("Pieces in no room")),
        ("loose_opening", list("Doors or windows in no wall")),
        ("unrated_light", list("Lights without a rating")),
        (
            "loose",
            list("Pieces floating or unsupported: {key, piece, why}"),
        ),
        (
            "outgrew_niche",
            list("Pieces bigger than their niche: {key, piece, host, over}"),
        ),
        (
            "above_ceiling",
            list("Pieces through the ceiling: {key, piece, room, ceiling, top, over}"),
        ),
        (
            "no_door",
            list("Rooms nobody can walk into: {key, name, room, passages}"),
        ),
        (
            "unclear_front",
            list("Pieces whose front is a guess: {key, piece, candidates, placed}"),
        ),
        (
            "turned",
            list("Pieces placed turned from how they were built: {key, piece, built, placed}"),
        ),
        (
            "accepted",
            list("Issues accepted as they are: {key, kind, why}"),
        ),
        (
            "overlap_kinds",
            object("Overlaps by kind: {collision, nesting, served, cross_level}"),
        ),
        ("warnings", strings("Storeys stacked at the same elevation")),
    ]
}

fn analysis_route() -> Vec<(&'static str, Value)> {
    vec![
        ("run", text("The run: <system>:<circuit|all|ids>")),
        (
            "replaced_drawn",
            strings("Hand-drawn lines this run replaced"),
        ),
        ("via", text("Where it runs: ceiling, floor, wall (or tape)")),
        ("suggested", text("The route the rules suggest")),
        ("length_m", object("Metres: {horizontal, vertical, total}")),
        (
            "by_premise_m",
            object(
                "Total metres by route: {ceiling, floor, wall}; a reason instead where one is impossible",
            ),
        ),
        ("bends", int("Bends along the run")),
        (
            "materials",
            list("Bill of materials: [item, quantity, unit]"),
        ),
        ("rev", int("Plan revision")),
    ]
}

/// A product of the library, in full.
fn product_fields() -> Vec<(&'static str, Value)> {
    vec![
        (
            "id",
            text("The product, for products, edit_products and place"),
        ),
        ("shop", text("The shop's domain")),
        ("market", text("The shop's market, e.g. nl")),
        (
            "sku",
            text("The article: the shop's number, or the link's path"),
        ),
        ("url", text("The product's page")),
        ("kind", text("furniture or finish")),
        ("title", text("The product's name")),
        (
            "fields",
            object("By name: {value, evidence (adapter, text, market or agent), from}"),
        ),
        (
            "missing",
            strings("Fields its kind needs that nobody has filled"),
        ),
        (
            "fetched_at",
            int("When the shop was last read (unix seconds)"),
        ),
        (
            "prices",
            list("Price history: [price, currency, sale_unit, origin, seen (unix seconds)]"),
        ),
        (
            "files",
            list(
                "Its 3D model, or a finish's texture from its swatch, once kept: [kind (model or texture), source (shop, generated or placeholder), generator, bytes, kept (unix seconds)]",
            ),
        ),
        (
            "job",
            object(
                "Its 3D model being made on the server: {id, status (queued, running, done or failed), error?, attempts, updated (unix seconds)}",
            ),
        ),
    ]
}

/// The output schema of a tool, by name. `None` for a name nobody has.
#[allow(clippy::too_many_lines, reason = "one row per tool, like hints")]
pub(crate) fn schema(name: &str) -> Option<Value> {
    let lighting_row =
        "[id, name, m², average lx, minimum lx, uniformity, reference lx, fixtures, W/m², verdict]";
    Some(match name {
        // Reads.
        "home" => shape(
            &[
                ("rev", int("Plan revision")),
                ("name", text("Project name")),
                (
                    "walls",
                    list("Walls: {id, a, b, t?, h?, arc?, type?, sides?|left?, right?}"),
                ),
                (
                    "rooms",
                    list("Rooms: {id, name, pts, m2, …}; with detail=summary, rows [id, name, m²]"),
                ),
                ("dims", list("Dimensions: {id, a, b, len, off?}")),
                ("labels", list("Labels: {id, text, at, …}")),
                (
                    "furniture",
                    list("Pieces: {id, cat, at, bounds, faces, wdh?, angle?, elev?, name?, …}"),
                ),
                ("polylines", list("Free lines: {id, pts, t, color, …}")),
                ("counts", object("detail=summary: how many of each kind")),
                (
                    "bounds",
                    list("detail=summary: [[minx,miny],[maxx,maxy]] cm"),
                ),
                ("north", num("Degrees from plan up to north, when set")),
                ("background", object("The scanned plan under the drawing")),
                (
                    "levels",
                    list(
                        "Storeys: [id, name, elevation, height, selected, index, viewable, reference]",
                    ),
                ),
                ("warnings", strings("What the read could not do as asked")),
                ("parts", text("Note when parts=true found no groups")),
                (
                    "lines",
                    list("ndjson=true: the head, then one element per line"),
                ),
            ],
            &[],
        ),
        "rules" => shape(&[("text", text("The rules, as Markdown"))], &["text"]),
        "materials" => shape(
            &[
                ("wall_types", list("[id, name, thickness cm]")),
                ("patterns", list("[key, label, #rrggbb, tile WxH cm]")),
            ],
            &["wall_types", "patterns"],
        ),
        "catalog" => shape(
            &[
                ("items", list("[id, name, w, d, h, front?] cm")),
                ("more", int("How many more matched than the limit")),
                (
                    "categories",
                    strings("Categories to search by, when neither q nor cat was given"),
                ),
                ("used", list("scope=project: [kind, name, count, id]")),
            ],
            &[],
        ),
        "library" => shape(
            &[
                (
                    "rows",
                    list("[name, latest version, title, brand, versions]"),
                ),
                ("name", text("name=: the entry")),
                (
                    "versions",
                    list(
                        "name=: [v, title, size, unit, source, fidelity, notes, photos, measures]",
                    ),
                ),
            ],
            &[],
        ),
        "model" => shape(
            &[
                ("file", text("The model file")),
                ("format", text("obj, gltf or glb")),
                (
                    "unit",
                    text("Unit taken for the file's numbers: m, cm, mm or in"),
                ),
                (
                    "guessed",
                    flag("Whether the unit was guessed from its size"),
                ),
                ("raw", list("[w, d, h] in the file's units")),
                ("size", list("Natural [w, d, h] cm")),
                ("tris", int("Triangles")),
                ("meshes", int("Meshes read")),
                (
                    "materials",
                    list("[name, #rrggbb, image or null, triangles]"),
                ),
                ("images", int("Images its materials use")),
                ("uv", flag("Whether it has texture coordinates")),
                (
                    "parts",
                    list("[name, triangles, min [x,y,z], max, materials] cm of the piece"),
                ),
                ("warnings", strings("What is not drawn as the file says")),
                (
                    "piece",
                    object("With id: {id, size cm, scale per axis, far?}"),
                ),
                (
                    "cost",
                    object("{tris, heaviest [[part, tris, %]], copies, in_project}"),
                ),
                (
                    "references",
                    list("With id: [index, file, view, part, source, note]"),
                ),
                (
                    "measures",
                    list("With id: [what, product cm, drawn cm, source, confirmed]"),
                ),
                (
                    "clashes",
                    list("check: [part, part, triangle pairs, at [x,y,z], depth] cm"),
                ),
                ("degenerate", list("check: [part, faces with no area]")),
                ("checked", text("check: what was and was not looked at")),
            ],
            &[
                "file",
                "format",
                "unit",
                "size",
                "tris",
                "materials",
                "warnings",
            ],
        ),
        "measure" => shape(
            &[
                ("spans", list("Probe: [from, to, id, name] along the axis")),
                ("fit", object("Probe with gap: {free, max, blocked_by}")),
                ("cm", num("Distance, cm")),
                ("axis", text("Axis measured along")),
                ("x", num("Gap along x, cm")),
                ("y", num("Gap along y, cm")),
                ("id", text("The element measured")),
                ("opening", object("A door or window: {basis, hosts}")),
                ("bounds", list("The piece's [[minx,miny],[maxx,maxy]] cm")),
                ("faces", text("Side its front looks to")),
                (
                    "clear",
                    object("Free floor by side: [cm, against id, name]"),
                ),
            ],
            &[],
        ),
        "sessions" => shape(
            &[
                ("rev", int("Plan revision")),
                (
                    "rows",
                    list("[id, name, cursor [x,y]|null, selection, edits]"),
                ),
                (
                    "render",
                    object("While one runs: {tool, seconds, phase, done, total, threads}"),
                ),
            ],
            &["rev", "rows"],
        ),
        "cameras" => shape(
            &[
                ("active", text("The camera in use: visitor or aerial")),
                (
                    "rows",
                    list("Stored views: [i, name, x, y, z, yaw, pitch, fov]"),
                ),
            ],
            &["active", "rows"],
        ),
        "video" => shape(
            &[
                ("fps", int("Frames per second")),
                ("speed", num("Camera speed m/s")),
                ("secs", num("Length in seconds")),
                ("rows", list("Keyframes: [i, x, y, z, yaw, pitch, fov]")),
            ],
            &["fps", "speed", "secs", "rows"],
        ),
        "levels" => shape(
            &[(
                "rows",
                list(
                    "Storeys: [id, name, elevation, height, selected, index, viewable, reference]",
                ),
            )],
            &["rows"],
        ),
        "variants" => shape(
            &[(
                "rows",
                list("Plan versions: [i, name, active, walls, rooms, m², pieces, issues]"),
            )],
            &["rows"],
        ),
        "history" => shape(
            &[("checkpoints", list("[label, changes ago]"))],
            &["checkpoints"],
        ),
        "plugins" => shape(&[("rows", list("[name, title, description]"))], &["rows"]),
        "finishes" => shape(
            &[
                ("columns", strings("What each row holds")),
                (
                    "rows",
                    list("[surface, room, product, band [from, to] cm, coats, waste %]"),
                ),
                (
                    "gone",
                    strings("Surfaces deleted since their finish was put on"),
                ),
            ],
            &["columns", "rows"],
        ),
        "takeoff" => shape(
            &[
                (
                    "finishes",
                    list(
                        "By product: {product, net_m2, gross_m2, openings_m2, surfaces: [{surface, level?, room?, band?, coats?, waste?, gross_m2, openings_m2, net_m2, openings?, spans_cm (wall sides): face stretches the band covers [width, tallest height in band]}]}",
                    ),
                ),
                (
                    "furniture",
                    list("Pieces bought, by product: {product, pieces, ids, rooms?}"),
                ),
                (
                    "gone",
                    list("Finishes whose surface was deleted: {surface, product}"),
                ),
            ],
            &["finishes", "furniture"],
        ),
        "disciplines" | "edit_disciplines" => shape(
            &[
                (
                    "active",
                    json!({"type": ["string", "null"], "description": "Discipline drawn on top: electrical, plumbing, or null"}),
                ),
                ("hidden", strings("Disciplines hidden")),
                (
                    "layers",
                    object("Plan layers: {lighting, appliances, joinery} → {pieces, hidden}"),
                ),
                ("show_all_3d", flag("Every discipline shown in 3D")),
                (
                    "electrical",
                    list("action=quantities: [kind, count, names?]"),
                ),
                ("plumbing", list("action=quantities: [kind, count, names?]")),
                (
                    "lines_cm",
                    object("action=quantities: drawn lines by discipline, cm"),
                ),
            ],
            &[],
        ),
        "layout" => {
            let mut fields = issues();
            fields.push((
                "orphaned",
                list("Acceptances whose issue is gone: [key, reason]"),
            ));
            fields.push((
                "areas",
                list("With areas: [name, expected m², actual m², difference %]"),
            ));
            shape(&fields, &[])
        }
        "ergonomics" => shape(
            &[
                ("score", int("Habitability score 0-100")),
                (
                    "score_basis",
                    text("What the score is: habitability_heuristic"),
                ),
                (
                    "scope",
                    object("Disciplines counted: {electrical, plumbing}"),
                ),
                (
                    "scores",
                    object("Score by discipline: {architecture, electrical, plumbing}"),
                ),
                (
                    "layout",
                    object("Layout issues of the storey, as layout gives them"),
                ),
                ("coverage", object("What was checked and what was not")),
                (
                    "capacity",
                    object("What the home holds: beds, bedrooms, bathrooms, seats, wardrobe cm"),
                ),
                (
                    "findings",
                    list(
                        "{sev, place, msg, key, weight, discipline, in_scope, src?, fix?, accepted?}",
                    ),
                ),
                sources(),
                orphaned(),
            ],
            &["score", "findings"],
        ),
        "electrical" => shape(
            &[
                ("points", object("Points by kind")),
                ("cables_m", object("Cable metres: {power, data, tv}")),
                findings_rows(),
                ("pending", int("Findings not accepted")),
                orphaned(),
                sources(),
                (
                    "circuits",
                    list(
                        "action=circuits: [name, kinds, points, VA, V, A, wire mm², breaker A, DR]",
                    ),
                ),
                ("total_va", num("action=circuits: total load VA")),
                ("standby_w", num("action=circuits: standby watts")),
                ("panel", object("action=circuits: the distribution panel")),
                (
                    "main_breaker",
                    object("action=circuits: {a, phases, load_a_per_phase}"),
                ),
                (
                    "access_points",
                    list("action=wifi: [id, standard, bands, uplink]"),
                ),
                (
                    "coverage",
                    list("action=wifi: [room, band, median, worst, good, grade]"),
                ),
                (
                    "suggested",
                    object("action=wifi: where access points would go"),
                ),
                ("rev", int("action=wifi: plan revision")),
            ],
            &[],
        ),
        "plumbing" => shape(
            &[
                ("points", object("Points by kind")),
                ("pipes_m", object("Pipe metres: {cold, hot, sewer, vent}")),
                findings_rows(),
                ("pending", int("Findings not accepted")),
                orphaned(),
                sources(),
            ],
            &[
                "points", "pipes_m", "findings", "pending", "orphaned", "sources",
            ],
        ),
        "lighting" => shape(
            &[
                ("rooms", list(lighting_row)),
                ("fixtures", int("Light fixtures in the plan")),
                ("lm", num("Total lumens")),
                ("W", num("Total watts")),
                sources(),
            ],
            &["rooms", "fixtures", "lm", "W", "sources"],
        ),
        "annotations" => shape(
            &[
                ("rev", int("Plan revision")),
                ("dims", flag("Dimensions shown")),
                ("refs", flag("Reference tags shown")),
                ("details", flag("Details shown")),
                ("legend", flag("Legend shown")),
                (
                    "rooms",
                    list("The schedule: [room, [[tag, name, w, d, h, brand?, model?, url?]]]"),
                ),
                ("labels", list("With q: labels {id, text, at, …}")),
                (
                    "stale",
                    list("stale=true: [id, written, measured, against, text]"),
                ),
                ("checked", object("stale=true: what was checked")),
                (
                    "unverified",
                    list("stale=true: [id, text] nobody can check"),
                ),
            ],
            &[],
        ),
        "background" => shape(
            &[("rows", list("Walls found: [[x1,y1], [x2,y2], thickness]"))],
            &["rows"],
        ),
        "cut_list" => shape(&cut_list_fields(), &["rows", "hardware", "sources"]),
        // A plan answers an `ok` line; a cut list, its rows as written.
        "export" => {
            let mut fields = ok_fields();
            fields.extend(cut_list_fields());
            shape(&fields, &[])
        }
        "render_plan" | "render_3d" => image(),
        "render_photo" => shape(
            &[
                (
                    "mimeType",
                    text("Type of the image in the content (image/png)"),
                ),
                (
                    "budget",
                    object("estimate: {threads, of, cores, one_at_a_time}"),
                ),
                (
                    "work",
                    object("estimate: {pixels, samples, bounces, rays, triangles, images}"),
                ),
                ("memory_mb", num("estimate: memory it needs, estimated")),
                ("measured", text("estimate: what is and is not measured")),
                ("text", text("cancel: what was stopped")),
            ],
            &[],
        ),
        "show_plan" => shape(
            &[
                ("name", text("Project name")),
                ("svg", text("The plan as SVG")),
                ("summary", object("{rooms, area, walls, pieces}")),
                ("editor", text("Link that opens the project in the editor")),
            ],
            &["name", "svg", "summary", "editor"],
        ),
        // Writes answering an `ok` line.
        "create" | "edit_walls" | "edit_background" | "edit_levels" | "edit_cameras"
        | "edit_home" | "file" | "arrange" | "edit_model" | "edit_library" | "edit_finishes" => {
            ok_line(&[])
        }
        "delete" => ok_line(&[
            (
                "labels_left",
                list("Labels left pointing at deleted pieces: [id, text]"),
            ),
            ("labels_deleted", strings("Labels deleted with them")),
        ]),
        "fit_roof" => ok_line(&[("fitted", int("Panels fitted under the roof"))]),
        "edit_variants" => ok_line(&[("i", int("Index of the new version"))]),
        "accept" => ok_line(&[
            ("accepted", int("Findings accepted")),
            ("removed", int("Acceptances removed")),
        ]),
        "edit_history" => {
            let mut fields = diff_fields();
            fields.extend(ok_fields());
            shape(&fields, &["ok", "text"])
        }
        "edit_video" => shape(
            &{
                let mut fields = ok_fields();
                fields.extend([
                    ("frames", int("action=render: frames written")),
                    ("secs", num("action=render: length in seconds")),
                    ("bytes", int("action=render: file size")),
                ]);
                fields
            },
            &[],
        ),
        "update" => write_or_dry(&[
            (
                "unchanged",
                list("Asked ids that came out as they were: {id, now}"),
            ),
            ("unchanged_note", text("Why they did not change")),
        ]),
        "move" => write_or_dry(&[]),
        "place" => write_or_dry(&[
            (
                "faces",
                strings("Where each front looks: <id>:<side>(<front>)"),
            ),
            (
                "turned",
                strings("Pieces turned back to a wall: <id>:back to <wall>"),
            ),
        ]),
        // Writes answering an object.
        "joinery" => shape(
            &[
                ("id", text("The build's group id (empty when dry)")),
                ("name", text("Its name")),
                ("size", list("[w, d, h] cm")),
                ("parts", int("Parts it is made of")),
                ("hardware", strings("Hardware it needs")),
                ("notes", strings("What a workshop would say about it")),
            ],
            &["id", "name", "size", "parts", "hardware", "notes"],
        ),
        "embed" => shape(
            &[
                ("host", text("The countertop or cabinet it went into")),
                ("item", text("The embedded piece (empty when dry)")),
                ("kind", text("sink, cooktop, oven, microwave, …")),
                ("notes", strings("What the rules say about it")),
                ("cutout", list("Countertop: the hole [w, d] cm")),
                (
                    "cut_below",
                    list("Countertop: cabinets under it whose wooden top was cut for the item"),
                ),
                ("x", num("Countertop or panel: center along it, cm")),
                ("niche", list("Cabinet: the niche [w, h] cm")),
                ("bottom", num("Bottom above the floor, cm")),
            ],
            &["host", "item", "kind", "notes"],
        ),
        "cabinet_run" => shape(
            &[
                ("wall", text("The wall the run is on")),
                ("row", text("base, wall or tall")),
                ("modules", list("[id, role, from cm, w cm]")),
                ("removed", strings("Cabinets it replaced")),
                ("notes", strings("What the rules say about it")),
                ("adjusted", list("Neighbouring runs replanned, same shape")),
            ],
            &["wall", "row", "modules", "removed", "notes"],
        ),
        "edit_lighting" => shape(
            &[
                ("placed", strings("Fixtures placed")),
                ("before", list(lighting_row)),
                ("after", list(lighting_row)),
                ("note", text("Why nothing was placed")),
            ],
            &["placed", "before"],
        ),
        "edit_annotations" => {
            let mut fields = ok_fields();
            fields.extend([
                (
                    "anchored",
                    strings("anchor=true: labels now tied to what they measure"),
                ),
                ("released", strings("anchor=true: labels let go")),
                (
                    "left",
                    list("anchor=true: [id, written, measured, near id] left as they were"),
                ),
                ("dims", flag("Dimensions shown")),
                ("refs", flag("Reference tags shown")),
                ("details", flag("Details shown")),
                ("legend", flag("Legend shown")),
                ("changed", strings("Switches that changed")),
                ("chains", list("dims=true: [[x,y], [x,y], cm]")),
                (
                    "symbols",
                    object("legend=true: {electrical?, plumbing?} → [name, count]"),
                ),
                (
                    "rooms",
                    list("The schedule: [room, [[tag, name, w, d, h, …]]]"),
                ),
            ]);
            shape(&fields, &[])
        }
        "edit_electrical" => {
            let mut fields = ok_fields();
            fields.extend(diff_fields());
            fields.extend(analysis_route());
            fields.extend([
                (
                    "suggested",
                    json!({"type": ["string", "object"], "description": "Route: the one the rules suggest; action=wifi: where access points would go"}),
                ),
                ("tape", object("via=tape: the tape light system")),
                ("warnings", strings("Data points past 90 m")),
                ("access_points", list("action=wifi: [id, standard, bands, uplink]")),
                ("coverage", list("action=wifi: [room, band, median, worst, good, grade]")),
            ]);
            shape(&fields, &[])
        }
        "edit_plumbing" => {
            let mut fields = analysis_route();
            fields.extend([
                (
                    "merged_into",
                    json!({"type": ["string", "null"], "description": "The run this one joined, or null"}),
                ),
                ("branches", int("Branches off the trunk")),
                ("notes", strings("What the rules say about the run")),
                ("trunk_mm", int("Sewer: trunk diameter mm")),
                ("slope_pct", num("Sewer: slope %")),
                ("needs_depth_cm", num("Sewer: depth it needs under the floor, cm")),
            ]);
            shape(
                &fields,
                &["run", "via", "length_m", "bends", "materials", "rev"],
            )
        }
        "run_plugin" => shape(
            &[
                ("ok", flag("The plugin exited cleanly")),
                (
                    "code",
                    json!({"type": ["integer", "null"], "description": "Exit code; null when it was stopped"}),
                ),
                ("timed_out", flag("It ran out of time")),
                ("stdout", text("What it printed")),
                ("stderr", text("What it complained")),
                ("edits", int("Changes it made")),
                ("revision", int("Plan revision after it")),
                (
                    "usage",
                    object("{peak_mb, cpu_s, measured}: what it used, where measurable"),
                ),
                ("limits", strings("Limits applied to its process")),
                (
                    "not_applied",
                    strings("Limits it asked for that this system could not apply"),
                ),
            ],
            &["ok", "stdout", "stderr", "edits", "revision"],
        ),
        "feedback" => shape(
            &[
                ("kept", text("Where the report was written")),
                ("sent", flag("Whether it reached the developers")),
            ],
            &["kept", "sent"],
        ),
        // The hosted service's own.
        "projects" => shape(
            &[
                (
                    "rows",
                    list("[name, id, kb, updated (unix seconds), active]"),
                ),
                ("columns", strings("What each row holds")),
                ("used_kb", int("Space the projects take, kB")),
                ("limits", object("What the account's plan allows")),
            ],
            &["rows", "columns", "used_kb", "limits"],
        ),
        "import_product" => {
            let mut fields = product_fields();
            fields.extend([
                (
                    "stored",
                    flag("The article was already in the library: nothing was fetched"),
                ),
                ("fetched", list("Calls made: [endpoint, outcome, credits]")),
            ]);
            shape(
                &fields,
                &["id", "shop", "kind", "fields", "missing", "stored"],
            )
        }
        "products" => {
            let mut fields = product_fields();
            fields.extend([
                (
                    "rows",
                    list("[id, kind, shop, title, price, currency, sale_unit, missing]"),
                ),
                ("columns", strings("What each row holds")),
            ]);
            shape(&fields, &[])
        }
        "edit_products" => {
            let mut fields = product_fields();
            fields.push(("removed", flag("The product was deleted")));
            fields.push((
                "note",
                text("For set: what the change leaves as it was, e.g. a generated model made at the old size"),
            ));
            fields.push((
                "drawn",
                text("For attach_model, or set of a finish's swatch or size: where the new file is drawn already, and where it waits for the product's next use"),
            ));
            shape(&fields, &["id"])
        }
        "estimate" => shape(
            &[
                ("project", object("The project priced: {id, name}")),
                (
                    "from",
                    text("Where its quantities were measured: tab (the open editor) or server"),
                ),
                ("group_by", text("shop, room or kind")),
                (
                    "groups",
                    list(
                        "{name, lines, totals per currency and VAT}: lines {kind, product?, line?, shop, title, url?, quantity, unit, packs?, need? [amount, unit], unit_price, currency, vat_included?, total (null: unpriced), rooms?, surfaces?, pieces?, notes?}",
                    ),
                ),
                (
                    "totals",
                    list(
                        "Per shop, currency and VAT basis: {shop, currency, vat (included, excluded, unknown), total, complete}",
                    ),
                ),
                (
                    "by_currency",
                    list(
                        "Per currency and VAT basis, never converted: {currency, vat, total, complete}",
                    ),
                ),
                ("complete", flag("Every line is priced")),
                (
                    "gone",
                    list("Finishes whose surface was deleted: {surface, product}"),
                ),
                (
                    "saved",
                    list("The project's saved estimates: [id, name, saved (unix seconds)]"),
                ),
                (
                    "saved_as",
                    object("For saved: the saved estimate read, {id, name, saved, project}"),
                ),
                (
                    "csv",
                    text("For format=csv: the sheet's download link, good for a day"),
                ),
                (
                    "note",
                    text(
                        "Why the quantities are of the active project as stored, not of what the open editor shows",
                    ),
                ),
            ],
            &["complete"],
        ),
        "edit_estimate" => shape(
            &[
                (
                    "line",
                    object("For add_line: the line as estimate shows it"),
                ),
                ("removed", flag("The line or saved estimate was deleted")),
                ("id", text("For remove_line: the line removed")),
                (
                    "saved",
                    object("For save and remove_saved: {id, name, saved (unix seconds), project}"),
                ),
                (
                    "by_currency",
                    list("For save: the totals kept, per currency and VAT basis"),
                ),
                ("complete", flag("For save: every line kept is priced")),
                (
                    "note",
                    text(
                        "For save: why it is of the active project as stored, not of what the open editor shows",
                    ),
                ),
            ],
            &[],
        ),
        _ => return None,
    })
}

/// Gives a tool its output schema.
pub(crate) fn apply(tool: &mut rmcp::model::Tool) {
    if let Some(mut schema) = schema(&tool.name) {
        undescribe(&mut schema);
        if let Value::Object(schema) = schema {
            tool.output_schema = Some(std::sync::Arc::new(schema));
        }
    }
}

/// The output schema goes out without its descriptions. The model reads the
/// answer, not its schema — the tool's description already says what comes
/// back — and a client validates the shape, which the descriptions do not
/// change; they cost 20 KB on every conversation. They stay in the table
/// above, for whoever maintains it.
fn undescribe(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("description");
            if let Some(Value::Object(props)) = map.get_mut("properties") {
                props.values_mut().for_each(undescribe);
            }
            for key in ["items", "additionalProperties"] {
                if let Some(inner) = map.get_mut(key) {
                    undescribe(inner);
                }
            }
            for key in ["anyOf", "oneOf"] {
                if let Some(Value::Array(branches)) = map.get_mut(key) {
                    branches.iter_mut().for_each(undescribe);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(undescribe),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks `value` against the parts of JSON Schema these schemas use:
    /// `type` (one or several), `properties`, `required` and `items`.
    fn fits(schema: &Value, value: &Value, at: &str) -> Result<(), String> {
        if let Some(kind) = schema.get("type") {
            let kinds: Vec<&str> = match kind {
                Value::String(k) => vec![k.as_str()],
                Value::Array(ks) => ks.iter().filter_map(Value::as_str).collect(),
                _ => vec![],
            };
            let is = |k: &str| match k {
                "object" => value.is_object(),
                "array" => value.is_array(),
                "string" => value.is_string(),
                "boolean" => value.is_boolean(),
                "number" => value.is_number(),
                "integer" => value.is_u64() || value.is_i64(),
                "null" => value.is_null(),
                _ => false,
            };
            if !kinds.iter().any(|k| is(k)) {
                return Err(format!("{at}: {value} is not {kinds:?}"));
            }
        }
        if let Some(Value::Array(required)) = schema.get("required") {
            for key in required.iter().filter_map(Value::as_str) {
                if value.get(key).is_none() {
                    return Err(format!("{at}: missing {key}"));
                }
            }
        }
        if let (Some(items), Some(values)) = (schema.get("items"), value.as_array()) {
            for (n, item) in values.iter().enumerate() {
                fits(items, item, &format!("{at}[{n}]"))?;
            }
        }
        if let (Some(Value::Object(properties)), Some(object)) =
            (schema.get("properties"), value.as_object())
        {
            for (key, field) in object {
                if let Some(rule) = properties.get(key) {
                    fits(rule, field, &format!("{at}.{key}"))?;
                }
            }
        }
        Ok(())
    }

    #[test]
    fn an_ok_line_becomes_its_fields() {
        assert_eq!(
            from_text(
                "ok rev=3 v=1 faces=f3:+y(seat and chaise),f4:-x(doors) turned=f3:back to w2 ids=f3,f4"
            ),
            json!({
                "ok": true, "rev": 3, "v": 1,
                "faces": ["f3:+y(seat and chaise)", "f4:-x(doors)"],
                "turned": ["f3:back to w2"],
                "ids": ["f3", "f4"],
                "text": "ok rev=3 v=1 faces=f3:+y(seat and chaise),f4:-x(doors) turned=f3:back to w2 ids=f3,f4",
            })
        );
        let undo = from_text(
            r#"ok rev=7 {"changed":[{"id":"w1","from":{"t":15},"to":{"t":20}}],"gone":["f2"]}"#,
        );
        assert_eq!(undo["rev"], 7);
        assert_eq!(undo["gone"], json!(["f2"]));
        assert_eq!(undo["changed"][0]["id"], "w1");
        assert_eq!(
            from_text("ok /tmp/a b.newera")["text"],
            "ok /tmp/a b.newera"
        );
        assert_eq!(from_text("ok rev=2 ids=r1 fitted=3")["fitted"], 3);
    }

    #[test]
    fn arrays_and_lines_become_objects() {
        assert_eq!(from_text("[[0,\"A\"]]"), json!({"rows": [[0, "A"]]}));
        assert_eq!(
            from_text("{\"rev\":1}\n{\"k\":\"walls\",\"id\":\"w1\"}"),
            json!({"lines": [{"rev": 1}, {"k": "walls", "id": "w1"}]})
        );
        assert_eq!(
            from_text("shown: 1 rooms"),
            json!({"text": "shown: 1 rooms"})
        );
    }

    /// Every tool has a schema, and it is an object schema a client accepts.
    #[test]
    fn every_tool_has_an_output_schema() {
        for tool in crate::tools().into_iter().chain(crate::hosted::tools()) {
            let schema = tool.output_schema.as_ref().unwrap_or_else(|| {
                panic!(
                    "{} has no output schema: add it to output::schema",
                    tool.name
                )
            });
            assert_eq!(schema["type"], "object", "{}", tool.name);
            // Descriptions leave on the way out (`undescribe`); the table
            // keeps them for whoever maintains it.
            let table = super::schema(&tool.name).expect("in the table");
            for (key, field) in table["properties"].as_object().expect("properties") {
                assert!(
                    field.get("description").is_some(),
                    "{}.{key} has no description",
                    tool.name
                );
            }
        }
    }

    /// Every argument says what it is, in the tool's own fields and in the
    /// shapes they point to.
    #[test]
    fn every_argument_is_described() {
        fn walk(tool: &str, at: &str, schema: &Value, missing: &mut Vec<String>) {
            if let Some(Value::Object(properties)) = schema.get("properties") {
                for (key, field) in properties {
                    if field.get("description").is_none() {
                        missing.push(format!("{tool}: {at}{key}"));
                    }
                }
            }
            if let Some(Value::Object(defs)) = schema.get("$defs") {
                for (name, def) in defs {
                    walk(tool, &format!("{name}."), def, missing);
                }
            }
        }
        let mut missing = Vec::new();
        for tool in crate::tools().into_iter().chain(crate::hosted::tools()) {
            let schema = Value::Object((*tool.input_schema).clone());
            walk(&tool.name, "", &schema, &mut missing);
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "arguments without a description:\n{}",
            missing.join("\n")
        );
    }

    /// What the tools really answer fits what their schemas say: every call
    /// below runs on one plan, in order, and each answer is checked.
    #[test]
    fn every_answer_fits_its_schema() {
        use newera_core::{Document, SharedDocument};
        let document = SharedDocument::new(Document::default());
        let dir = std::env::temp_dir().join(format!("newera-mcp-output-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("box.obj"),
            "v 0 0 0\nv 50 0 0\nv 50 80 0\nv 0 0 40\nf 1 2 3\nf 1 3 4\n",
        )
        .unwrap();
        let calls: Vec<(&str, Value)> = vec![
            (
                "create",
                json!({"walls": [{"pts": [[0,0],[500,0],[500,400],[0,400],[0,0]]}]}),
            ),
            (
                "create",
                json!({"rooms": [{"name": "Sala", "at": [250, 200]}], "labels": [{"text": "Sala", "at": [250, 200]}], "dims": [{"a": [0,0], "b": [500,0]}]}),
            ),
            (
                "place",
                json!({"items": [{"cat": "sofa-3", "at": [250, 350]}, {"cat": "door", "wall": "w1"}]}),
            ),
            (
                "place",
                json!({"items": [{"cat": "bed-double", "at": [100, 100]}], "dry": true}),
            ),
            (
                "place",
                json!({"items": [{"cat": "bed-double", "at": [100, 100]}], "dry": "summary"}),
            ),
            ("update", json!({"items": [{"id": "w1", "t": 20}]})),
            (
                "update",
                json!({"items": [{"id": "w1", "t": 25}], "dry": true}),
            ),
            ("move", json!({"ids": ["w1"], "dx": 0, "dy": 0})),
            (
                "arrange",
                json!({"action": "array", "ids": ["w1"], "n": 1, "dx": 10}),
            ),
            ("edit_walls", json!({"action": "split", "id": "w2"})),
            ("edit_history", json!({"action": "undo"})),
            ("edit_history", json!({"action": "redo"})),
            ("home", json!({})),
            ("home", json!({"detail": "summary"})),
            ("home", json!({"ndjson": true})),
            ("materials", json!({})),
            ("catalog", json!({"q": "sofa"})),
            ("catalog", json!({})),
            ("measure", json!({"from": "w1", "to": "w2"})),
            ("model", json!({"file": dir.join("box.obj")})),
            (
                "model",
                json!({"file": dir.join("box.obj"), "check": "clashes"}),
            ),
            ("sessions", json!({})),
            ("render_photo", json!({"estimate": true, "threads": 1})),
            ("cameras", json!({})),
            ("edit_cameras", json!({"action": "store", "name": "A"})),
            ("video", json!({})),
            ("edit_video", json!({"action": "orbit"})),
            (
                "edit_history",
                json!({"action": "checkpoint", "label": "a"}),
            ),
            ("history", json!({})),
            ("disciplines", json!({})),
            ("layout", json!({})),
            ("ergonomics", json!({})),
            ("electrical", json!({})),
            ("plumbing", json!({})),
            ("lighting", json!({})),
            (
                "edit_lighting",
                json!({"room": "r5", "fixture": "downlight"}),
            ),
            (
                "edit_lighting",
                json!({"room": "r5", "fixture": "downlight"}),
            ),
            ("annotations", json!({})),
            ("joinery", json!({"kind": "cabinet", "at": [250, 50]})),
            ("cabinet_run", json!({"wall": "w3"})),
            ("cut_list", json!({})),
            ("show_plan", json!({})),
            ("edit_home", json!({"name": "Casa"})),
            ("delete", json!({"ids": ["w1"]})),
            ("levels", json!({})),
            ("edit_levels", json!({"action": "add", "name": "Superior"})),
            ("levels", json!({})),
            ("variants", json!({})),
            ("edit_variants", json!({"action": "duplicate", "name": "B"})),
            ("variants", json!({})),
            ("electrical", json!({"action": "circuits"})),
            ("electrical", json!({"action": "wifi"})),
            (
                "edit_electrical",
                json!({"action": "voltage", "volts": 220}),
            ),
            (
                "edit_finishes",
                json!({"action": "assign", "surface": "w3:left", "product": "p1", "band": [0, 120], "waste": 10, "mat": "tiles #ffffff 30x60"}),
            ),
            (
                "edit_finishes",
                json!({"action": "assign", "surface": "r5:floor", "product": "p2"}),
            ),
            ("finishes", json!({})),
            ("takeoff", json!({})),
            ("takeoff", json!({"room": "r5", "product": "p2"})),
            (
                "edit_finishes",
                json!({"action": "remove", "surface": "w3:left"}),
            ),
            ("edit_annotations", json!({"dims": true, "legend": true})),
            ("edit_annotations", json!({"refs": true})),
            ("render_plan", json!({"w": 64, "h": 48})),
            ("export", json!({"path": dir.join("plan.svg")})),
            (
                "file",
                json!({"action": "save", "path": dir.join("casa.newera")}),
            ),
            (
                "file",
                json!({"action": "open", "path": dir.join("casa.newera")}),
            ),
            ("file", json!({"action": "new"})),
        ];
        let mut failures = Vec::new();
        for (name, args) in calls {
            let result = match crate::call(document.clone(), name, args.clone()) {
                Ok(result) => result,
                Err(why) => {
                    failures.push(format!("{name} {args}: {why}"));
                    continue;
                }
            };
            if result.is_error == Some(true) {
                failures.push(format!("{name} {args}: refused {:?}", result.content));
                continue;
            }
            let structured = result
                .structured_content
                .unwrap_or_else(|| panic!("{name} answered no structuredContent"));
            let schema = schema(name).expect("schema");
            if let Err(why) = fits(&schema, &structured, name) {
                failures.push(format!("{name} {args}: {why}"));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// `edit_model` needs a piece built from a file, which the plan above
    /// has none of by the time it would run.
    #[test]
    fn a_replaced_model_answers_what_its_schema_says() {
        use newera_core::{Document, SharedDocument};
        let document = SharedDocument::new(Document::default());
        let dir = std::env::temp_dir().join(format!("newera-mcp-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.obj"),
            "v 0 0 0\nv 50 0 0\nv 50 80 0\nv 0 0 40\nf 1 2 3\nf 1 3 4\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("b.obj"),
            "v 0 0 0\nv 60 0 0\nv 60 80 0\nv 0 0 40\nf 1 2 3\nf 1 3 4\n",
        )
        .unwrap();
        let placed = crate::call(
            document.clone(),
            "place",
            json!({"items": [{"model": dir.join("a.obj"), "at": [0, 0]}]}),
        )
        .unwrap();
        assert_ne!(placed.is_error, Some(true), "{placed:?}");
        let result = crate::call(
            document,
            "edit_model",
            json!({"action": "replace", "ids": ["f1"], "file": dir.join("b.obj")}),
        )
        .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let structured = result.structured_content.expect("structured");
        fits(&schema("edit_model").unwrap(), &structured, "edit_model").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
