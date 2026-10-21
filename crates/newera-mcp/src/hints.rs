//! What each tool is called for people, and what it does to the world.
//!
//! An AI client shows the title where a person approves a call, and reads the
//! hints to decide how carefully to ask: a read runs without a prompt, a write
//! asks, a destructive write asks harder. The directories that list this
//! server (the Claude and `ChatGPT` ones) check that every tool says so, and that it
//! says so truthfully.
//!
//! One table, applied where the router is assembled, so every transport — the
//! window, `serve`, stdio, the relay in front of a browser tab, the hosted
//! service — hands out the same answer. A tool missing from it fails
//! `every_tool_has_hints`.
//!
//! The hints describe the worst any action of a tool can do: `edit_cameras`
//! stores a view, but it also deletes one, so it is a destructive write. Reads
//! live in tools of their own (`cameras` lists), so they never share a tool
//! with a change: a client runs a read without asking. A tool that only adds
//! (a wall, a piece, a copy) is a write that is not destructive.

use rmcp::model::{Tool, ToolAnnotations};

/// What a tool does to the world outside the answer it gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    /// Reads the project, changes nothing.
    Read,
    /// Adds to the project and never changes or removes what is there.
    Add,
    /// Can change or remove what is there: undoable in the editor, but a
    /// change all the same.
    Change,
}

/// `(tool, title, effect, reaches beyond the project)`.
///
/// "Beyond the project" is the open-world hint: a tool whose effect is not
/// confined to the plan and the files the user names — one that sends
/// something to the developers or starts another program.
const HINTS: &[(&str, &str, Effect, bool)] = &[
    // Reads.
    ("home", "Read the home", Effect::Read, false),
    (
        "materials",
        "List wall types and finishes",
        Effect::Read,
        false,
    ),
    (
        "catalog",
        "Search the furniture catalog",
        Effect::Read,
        false,
    ),
    ("measure", "Measure the plan", Effect::Read, false),
    ("model", "Inspect an imported model", Effect::Read, false),
    ("library", "List the model library", Effect::Read, false),
    ("sessions", "List who is editing", Effect::Read, false),
    ("cameras", "List points of view", Effect::Read, false),
    ("video", "Read the video path", Effect::Read, false),
    ("levels", "List storeys", Effect::Read, false),
    ("variants", "List plan versions", Effect::Read, false),
    ("history", "List checkpoints", Effect::Read, false),
    ("plugins", "List plugins", Effect::Read, false),
    (
        "disciplines",
        "Show what the plan displays",
        Effect::Read,
        false,
    ),
    ("layout", "Check the layout", Effect::Read, false),
    (
        "rules",
        "Read the rules behind a review",
        Effect::Read,
        false,
    ),
    ("ergonomics", "Review ergonomics", Effect::Read, false),
    (
        "electrical",
        "Check the electrical project",
        Effect::Read,
        false,
    ),
    (
        "plumbing",
        "Check the plumbing project",
        Effect::Read,
        false,
    ),
    ("lighting", "Rate the lighting", Effect::Read, false),
    ("finishes", "List finishes on surfaces", Effect::Read, false),
    ("takeoff", "Measure quantities to buy", Effect::Read, false),
    (
        "annotations",
        "Read dimensions, tags and notes",
        Effect::Read,
        false,
    ),
    ("background", "Find walls in a scan", Effect::Read, false),
    ("cut_list", "Cut list", Effect::Read, false),
    ("render_plan", "Render the floor plan", Effect::Read, false),
    ("show_plan", "Show the plan", Effect::Read, false),
    ("render_3d", "Render a 3D view", Effect::Read, false),
    ("render_photo", "Render a photo", Effect::Read, false),
    // The hosted service's own reads, from its database.
    ("projects", "List your projects", Effect::Read, false),
    ("products", "List shop products", Effect::Read, false),
    // Writes that only add.
    ("create", "Draw walls, rooms and more", Effect::Add, false),
    (
        "place",
        "Place furniture, doors and windows",
        Effect::Add,
        false,
    ),
    (
        "edit_lighting",
        "Fill a room with light",
        Effect::Add,
        false,
    ),
    (
        "edit_disciplines",
        "Show or hide disciplines",
        Effect::Add,
        false,
    ),
    // Writes that change or remove.
    ("update", "Change elements", Effect::Change, false),
    (
        "edit_finishes",
        "Put products on surfaces",
        Effect::Change,
        false,
    ),
    ("move", "Move elements", Effect::Change, false),
    ("delete", "Delete elements", Effect::Change, false),
    ("arrange", "Copy, align and group", Effect::Change, false),
    ("edit_walls", "Split or merge walls", Effect::Change, false),
    ("edit_model", "Replace a model file", Effect::Change, false),
    (
        "edit_library",
        "Publish to the model library",
        Effect::Add,
        false,
    ),
    ("joinery", "Build joinery", Effect::Change, false),
    (
        "cabinet_run",
        "Fill a wall with cabinets",
        Effect::Change,
        false,
    ),
    ("embed", "Embed an appliance or sink", Effect::Change, false),
    ("fit_roof", "Fit walls to the roof", Effect::Change, false),
    ("edit_levels", "Change storeys", Effect::Change, false),
    (
        "edit_cameras",
        "Change points of view",
        Effect::Change,
        false,
    ),
    (
        "edit_video",
        "Change or render the video",
        Effect::Change,
        false,
    ),
    (
        "edit_variants",
        "Change plan versions",
        Effect::Change,
        false,
    ),
    (
        "edit_annotations",
        "Change dimensions and tags",
        Effect::Change,
        false,
    ),
    (
        "edit_electrical",
        "Change the electrical project",
        Effect::Change,
        false,
    ),
    ("edit_plumbing", "Lay a pipe run", Effect::Change, false),
    ("accept", "Accept findings", Effect::Change, false),
    ("edit_home", "Project settings", Effect::Change, false),
    (
        "edit_background",
        "Set or trace a scanned plan",
        Effect::Change,
        false,
    ),
    (
        "edit_history",
        "Undo, redo, checkpoints",
        Effect::Change,
        false,
    ),
    ("file", "New, open or save", Effect::Change, false),
    ("export", "Export to a file", Effect::Change, false),
    // Beyond the project.
    ("run_plugin", "Run a plugin", Effect::Change, true),
    // A shop's page, fetched through AnySite into the account's library.
    ("import_product", "Import a shop product", Effect::Add, true),
    (
        "edit_products",
        "Change shop products",
        Effect::Change,
        true,
    ),
    ("feedback", "Report to the developers", Effect::Add, true),
];

/// Gives a tool its title and hints. A tool not in the table is left as it
/// is, and the test below says which one.
pub(crate) fn apply(tool: &mut Tool) {
    let Some(&(_, title, effect, open)) = HINTS.iter().find(|(name, ..)| *name == tool.name) else {
        return;
    };
    let mut hints = ToolAnnotations::default();
    hints.title = Some(title.to_owned());
    hints.read_only_hint = Some(effect == Effect::Read);
    if effect != Effect::Read {
        hints.destructive_hint = Some(effect == Effect::Change);
    }
    hints.open_world_hint = Some(open);
    tool.title = Some(title.to_owned());
    tool.annotations = Some(hints);
}

#[cfg(test)]
mod tests {
    use super::HINTS;

    /// Every tool says what it does. A new tool without a row here would
    /// reach the directories unlabelled, and be refused there.
    #[test]
    fn every_tool_has_hints() {
        for tool in crate::tools().into_iter().chain(crate::hosted::tools()) {
            let hints = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no hints: add it to HINTS", tool.name));
            assert!(tool.title.is_some(), "{} has no title", tool.name);
            assert!(hints.read_only_hint.is_some(), "{}", tool.name);
            assert!(hints.open_world_hint.is_some(), "{}", tool.name);
            if hints.read_only_hint == Some(false) {
                assert!(hints.destructive_hint.is_some(), "{}", tool.name);
            }
        }
    }

    /// And the table names only tools that exist: a row left behind by a
    /// renamed tool would label nothing, and hide that the new name has none.
    #[test]
    fn every_row_names_a_tool() {
        let names: Vec<String> = crate::tools()
            .iter()
            .chain(&crate::hosted::tools())
            .map(|t| t.name.to_string())
            .collect();
        for (name, ..) in HINTS {
            assert!(names.iter().any(|n| n == name), "no tool named {name}");
        }
        let mut seen: Vec<&str> = HINTS.iter().map(|(name, ..)| *name).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), HINTS.len(), "a tool has two rows");
    }

    /// A tool labelled a read changes nothing: called on a plan with walls,
    /// a room and a piece, the revision stays where it was. A client runs
    /// these without asking, so a label that lied would let a change through
    /// unseen.
    #[test]
    fn reads_change_nothing() {
        use newera_core::{Document, SharedDocument};
        let document = SharedDocument::new(Document::default());
        crate::call(
            document.clone(),
            "create",
            serde_json::json!({
                "walls": [{"pts": [[0, 0], [400, 0], [400, 300], [0, 300]], "closed": true}],
                "rooms": [{"name": "Sala", "at": [200, 150]}],
            }),
        )
        .expect("a room to read");
        let before = document.read().revision();
        for tool in crate::tools() {
            let read = tool
                .annotations
                .as_ref()
                .and_then(|h| h.read_only_hint)
                .unwrap_or(false);
            // The photo is a path tracer: minutes in a debug build, and a
            // read like the other renders.
            if !read || tool.name == "render_photo" {
                continue;
            }
            let _ = crate::call(document.clone(), &tool.name, serde_json::json!({}));
            assert_eq!(
                document.read().revision(),
                before,
                "{} is labelled a read but changed the plan",
                tool.name
            );
        }
    }

    /// A write tool refuses a read, and says which tool answers it.
    #[test]
    fn a_write_tool_points_reads_elsewhere() {
        use newera_core::{Document, SharedDocument};
        let document = SharedDocument::new(Document::default());
        for (tool, read) in [
            ("edit_cameras", "cameras"),
            ("edit_levels", "levels"),
            ("edit_video", "video"),
            ("edit_variants", "variants"),
            ("edit_disciplines", "disciplines"),
            ("edit_finishes", "finishes"),
        ] {
            let why = crate::call(
                document.clone(),
                tool,
                serde_json::json!({"action": "list"}),
            )
            .expect_err("a read is refused");
            assert!(why.contains(read), "{tool}: {why}");
        }
    }

    /// A read given a write's argument refuses it by name, instead of
    /// answering as if the change had been made.
    #[test]
    fn a_read_refuses_a_write_argument() {
        use newera_core::{Document, SharedDocument};
        let document = SharedDocument::new(Document::default());
        for (tool, args) in [
            ("cameras", serde_json::json!({"action": "delete", "i": 0})),
            ("levels", serde_json::json!({"action": "add"})),
            ("video", serde_json::json!({"action": "clear"})),
            ("variants", serde_json::json!({"action": "new"})),
            ("history", serde_json::json!({"label": "a"})),
            ("plugins", serde_json::json!({"name": "x"})),
            ("plumbing", serde_json::json!({"action": "route"})),
            ("disciplines", serde_json::json!({"d": "plumbing"})),
            (
                "electrical",
                serde_json::json!({"ids": ["f1"], "circuit": "C1"}),
            ),
            ("lighting", serde_json::json!({"fill": "downlight"})),
            ("annotations", serde_json::json!({"anchor": true})),
            ("layout", serde_json::json!({"prune": true})),
            ("ergonomics", serde_json::json!({"accept": [["k", "r"]]})),
            ("background", serde_json::json!({"create": true})),
            ("cut_list", serde_json::json!({"path": "/tmp/x.csv"})),
        ] {
            let why =
                crate::call(document.clone(), tool, args).expect_err("a write argument is refused");
            assert!(
                why.contains("unknown argument") || why.contains("only"),
                "{tool}: {why}"
            );
        }
    }

    /// Titles are for people, and the directories cap tool names at 64
    /// characters; keep titles well under that.
    #[test]
    fn titles_are_short() {
        for (name, title, ..) in HINTS {
            assert!(title.len() <= 40, "{name}: title too long");
        }
    }
}
