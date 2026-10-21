//! What each place the tools are served from offers, and says about itself.
//!
//! One surface, three places: the desktop (the window, `serve`, stdio), a
//! browser tab reached through the relay, and the hosted service. A place
//! offers a tool only when it can run it — a tool listed and then refused is
//! a detour the agent pays for — and says what a tool means there when that
//! differs (a save in a tab is a download). Everything a transport changes is
//! here, so the relay, the tab and the hosted service hand out one answer,
//! and `every_offered_tool_is_in_one_category` keeps the server instructions
//! naming every tool a place offers.

use serde_json::{Value, json};

/// Tools the hosted service adds of its own and answers from its database:
/// the account's projects, its library of shop products and the estimate. Defined in
/// `hosted.rs`; never run by [`crate::call`].
pub const HOSTED_ONLY: [&str; 6] = [
    "projects",
    "import_product",
    "products",
    "edit_products",
    "estimate",
    "edit_estimate",
];

/// Where the tools are served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// The desktop window, `newera serve` and `newera mcp`.
    Native,
    /// The editor in a browser tab, reached through the relay.
    Browser,
    /// The hosted service, on the account's projects (or its open tab).
    Cloud,
}

/// Tools that need the person's own machine: they read or write files on its
/// disk (a scanned plan, a video), run its programs (plugins), or keep notes
/// there (feedback). Neither a tab nor the hosted service has one.
const DESKTOP_ONLY: [&str; 9] = [
    "feedback",
    "library",
    "edit_library",
    "plugins",
    "run_plugin",
    "video",
    "edit_video",
    "background",
    "edit_background",
];

impl Transport {
    /// Whether this place offers `tool`: lists it, and runs it when called.
    #[must_use]
    pub fn offers(self, tool: &str) -> bool {
        if HOSTED_ONLY.contains(&tool) {
            return self == Self::Cloud;
        }
        self == Self::Native || !DESKTOP_ONLY.contains(&tool)
    }

    /// What `tool` does here, when that differs from the desktop.
    fn meaning(self, tool: &str) -> Option<&'static str> {
        match (self, tool) {
            (Self::Browser, "file") => Some(
                "Start or keep the project in this browser tab. new: an empty project in place of this one. save {path?}: downloads a .newera backup; path supplies a filename only; reports download_started (not disk confirmation) and autosave recovery status, and no server file is written. open reads files on a desktop only.",
            ),
            (Self::Cloud, "file") => Some(
                "Start, open or keep the account's projects. new {name?}: a new, empty floor plan project, made the active one. open {path}: one of the account's projects by name (projects lists them), made the active one. save {path?}: keeps the active project now — it is also kept after every change; path renames it. With the editor open and signed in at 3dneweraai.com/app, calls reach that tab instead: save downloads a .newera backup there, and open reads files on a desktop only.",
            ),
            (Self::Cloud, "export") => Some(
                "Export the active project to a file, by the extension of path. what=plan (default): .pdf (A3; scale=50/100 or fit), .svg or .png, or the 3D model as .glb or .obj. what=cut_list: the joinery's cut list as .csv, or .dxf/.svg sheets. Reply: a link to the file, good for a day.",
            ),
            _ => None,
        }
    }
}

/// The tools a place offers, as the JSON its `tools/list` answers: the
/// hosted service's own come after the editor's.
#[must_use]
pub fn tools(transport: Transport) -> Vec<Value> {
    crate::tools()
        .into_iter()
        .chain(crate::hosted::tools())
        .filter(|t| transport.offers(&t.name))
        .filter_map(|t| serde_json::to_value(t).ok())
        .map(|mut tool| {
            let name = tool["name"].as_str().unwrap_or_default().to_owned();
            if let Some(meaning) = transport.meaning(&name) {
                tool["description"] = json!(meaning);
            }
            match (transport, name.as_str()) {
                (Transport::Browser, "home") => {
                    let said = tool["description"].as_str().unwrap_or_default();
                    tool["description"] = json!(format!(
                        "{said} Browser replies also include recovery state, current_revision, saved_revision or restored_from_revision, timestamp and any storage failure."
                    ));
                }
                // A project here has a name of its own, given when it starts.
                (Transport::Cloud, "file") => {
                    let props = &mut tool["inputSchema"]["properties"];
                    props["name"] =
                        json!({"type": "string", "description": "For `new`: the project's name"});
                    props["path"] = json!({
                        "type": "string",
                        "description": "For `open` and `save`: a project's name — open takes one projects lists, save renames the active one",
                    });
                }
                _ => {}
            }
            tool
        })
        .collect()
}

/// The first argument a call to one of [`HOSTED_ONLY`] was given that the
/// tool's schema does not declare, as the sentence an agent needs to fix
/// its call — the check every other tool gets inside [`crate::call`].
/// `None` when the call fits, or the tool is not hosted.
#[must_use]
pub fn unknown_hosted_argument(tool: &str, args: &Value) -> Option<String> {
    crate::hosted::unknown_argument(tool, args)
}

/// The tools by what they are for: how the instructions name them, so a
/// client that defers the definitions still shows the model every one.
const CATEGORIES: &[(&str, &[&str])] = &[
    (
        "project",
        &[
            "home",
            "file",
            "edit_home",
            "history",
            "edit_history",
            "variants",
            "edit_variants",
            "levels",
            "edit_levels",
            "sessions",
            "projects",
        ],
    ),
    ("products", &["import_product", "products", "edit_products"]),
    (
        "finishes, quantities and costs",
        &[
            "finishes",
            "edit_finishes",
            "takeoff",
            "estimate",
            "edit_estimate",
        ],
    ),
    (
        "drawing",
        &[
            "create",
            "update",
            "delete",
            "move",
            "edit_walls",
            "place",
            "arrange",
            "catalog",
            "materials",
            "measure",
            "model",
            "edit_model",
            "library",
            "edit_library",
            "fit_roof",
            "background",
            "edit_background",
        ],
    ),
    ("joinery", &["joinery", "cabinet_run", "embed", "cut_list"]),
    (
        "reviews",
        &[
            "layout",
            "ergonomics",
            "electrical",
            "plumbing",
            "lighting",
            "accept",
            "rules",
        ],
    ),
    (
        "projects on the plan",
        &[
            "edit_electrical",
            "edit_plumbing",
            "edit_lighting",
            "annotations",
            "edit_annotations",
            "disciplines",
            "edit_disciplines",
        ],
    ),
    (
        "images and files",
        &[
            "render_plan",
            "render_3d",
            "render_photo",
            "show_plan",
            "cameras",
            "edit_cameras",
            "video",
            "edit_video",
            "export",
        ],
    ),
    ("extras", &["plugins", "run_plugin", "feedback"]),
];

/// What the server tells an agent about itself, as this place says it.
#[must_use]
pub fn instructions(transport: Transport) -> String {
    let intro = match transport {
        Transport::Native => "Home design editor, live in the user's window.",
        Transport::Browser => {
            "Home design editor running in someone's browser tab, reached through a relay. \
             Everything you change appears on their screen as you do it and can be undone with \
             Ctrl+Z. The project lives in that tab: it is not saved anywhere here, and closing \
             the tab ends the session."
        }
        Transport::Cloud => {
            "Home design editor (3D New Era AI), acting for the signed-in account. When the \
             person has the editor open at 3dneweraai.com/app and signed in, calls reach it and \
             every change appears there; otherwise they work on the account's active project in \
             the cloud, kept after every change (projects lists them). Renders always run here, \
             on the active project, after the open editor saves its changes."
        }
    };
    let map: Vec<String> = CATEGORIES
        .iter()
        .filter_map(|(what, names)| {
            let offered: Vec<&str> = names
                .iter()
                .copied()
                .filter(|n| transport.offers(n))
                .collect();
            (!offered.is_empty()).then(|| format!("{what}: {}", offered.join(", ")))
        })
        .collect();
    let mut text = format!(
        "{intro} Units: cm. Plan axes: x right, y down. Points are [x,y]. \
Id prefixes: w wall, r room, d dimension, t label, f furniture/door/window, lv storey. \
All kinds share one id counter, and composite pieces (roofs, joinery, cabinet runs) also number \
their parts, so ids have gaps: use the ids a reply returns, never guess the next one. \
Reads omit defaults (wall t=15 h=250) and never change the plan. Writes reply `ok rev=N [ids=...]`; \
don't re-read unless needed. Every change is one undoable step. \
Tools, by what they are for — most reads are nouns, and a read that can be changed has an \
edit_<noun> beside it; file, export and the element verbs (create, update, place…) are named for \
what they do: {}. \
Furniture has a front (seat, doors, foot of the bed; catalog names it): place facing= says which \
way it looks, or wall=<id> puts its back on a wall; layout lists pieces turned to face a wall as \
backwards. Spots, panels and pendants are kept on the ceiling for you: a pendant takes elev (shade \
height) or h (drop). render_plan shows the plan to you, show_plan to the person. A project can hold \
several plan versions; tools act on the active one. Finishes are short strings: `#rrggbb` paint, a \
pattern like `tiles #ffffff 60x60 r45` (tint, tile cm, rotation) or `img:path 90x90`; `none` clears.",
        map.join("; ")
    );
    if transport.offers("feedback") {
        text.push_str(
            " When a tool answers less than you asked, makes you take a detour, or leads you to a \
wrong conclusion before the right one, report it with feedback as it happens, with the whole case \
(the call, the literal reply, what was true, what it cost, the change that would help and what must \
not get worse) — then carry on.",
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{CATEGORIES, DESKTOP_ONLY, HOSTED_ONLY, Transport, instructions, tools};

    const ALL: [Transport; 3] = [Transport::Native, Transport::Browser, Transport::Cloud];

    /// Every tool a place offers is named once in its instructions' map, and
    /// the map names no tool that is not there: a tool left out of the map is
    /// one a client deferring definitions never shows the model.
    #[test]
    fn every_offered_tool_is_in_one_category() {
        let every: Vec<String> = crate::tools()
            .iter()
            .chain(&crate::hosted::tools())
            .map(|t| t.name.to_string())
            .collect();
        let mut mapped: Vec<&str> = CATEGORIES
            .iter()
            .flat_map(|(_, n)| n.iter().copied())
            .collect();
        mapped.sort_unstable();
        let before = mapped.len();
        mapped.dedup();
        assert_eq!(before, mapped.len(), "a tool is in two categories");
        for name in &every {
            assert!(mapped.contains(&name.as_str()), "{name} is in no category");
        }
        for name in &mapped {
            assert!(
                every.iter().any(|n| n == name),
                "category names {name}, which is not a tool"
            );
        }
        for transport in ALL {
            let said = instructions(transport);
            for tool in tools(transport) {
                let name = tool["name"].as_str().unwrap();
                assert!(
                    said.contains(name),
                    "{transport:?} instructions leave out {name}"
                );
            }
        }
    }

    /// What a place does not offer, it does not list, and its instructions do
    /// not name.
    #[test]
    fn a_place_lists_only_what_it_runs() {
        for transport in [Transport::Browser, Transport::Cloud] {
            let listed: Vec<String> = tools(transport)
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_owned())
                .collect();
            let said = instructions(transport);
            for name in DESKTOP_ONLY {
                assert!(
                    !listed.iter().any(|n| n == name),
                    "{transport:?} lists {name}"
                );
                assert!(
                    !said.contains(&format!(" {name},")) && !said.contains(&format!(" {name};")),
                    "{transport:?} instructions name {name}"
                );
            }
        }
        assert_eq!(tools(Transport::Native).len(), crate::tools().len());
        // The hosted service's own tools are its alone.
        for transport in [Transport::Native, Transport::Browser] {
            let said = instructions(transport);
            for name in HOSTED_ONLY {
                assert!(!transport.offers(name), "{transport:?} offers {name}");
                assert!(
                    !said.contains(&format!(" {name},")) && !said.contains(&format!(" {name};")),
                    "{transport:?} names {name}"
                );
            }
        }
        let hosted: Vec<String> = crate::hosted::tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(hosted, HOSTED_ONLY, "HOSTED_ONLY names the hosted tools");
        let listed = tools(Transport::Cloud);
        for name in HOSTED_ONLY {
            assert!(
                listed.iter().any(|t| t["name"] == name),
                "Cloud leaves out {name}"
            );
        }
    }

    /// The places differ only where they say so: every other tool goes out
    /// exactly as the desktop hands it out.
    #[test]
    fn a_place_rewrites_only_what_it_means_differently() {
        let native = tools(Transport::Native);
        for transport in [Transport::Browser, Transport::Cloud] {
            let mut rewritten = Vec::new();
            for tool in tools(transport) {
                let name = tool["name"].as_str().unwrap();
                // The hosted service's own tools have no desktop original.
                let Some(original) = native.iter().find(|t| t["name"] == name) else {
                    assert!(HOSTED_ONLY.contains(&name), "{name}");
                    continue;
                };
                if &tool != original {
                    rewritten.push(name.to_owned());
                }
            }
            let expected: &[&str] = match transport {
                Transport::Browser => &["file", "home"],
                _ => &["export", "file"],
            };
            assert_eq!(rewritten, expected, "{transport:?}");
        }
    }
}
