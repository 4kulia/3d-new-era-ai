//! The rules the tool surface keeps as it grows.
//!
//! `tests/fixtures/tool-surface.json` says *that* the surface changed; these
//! say whether the change keeps it usable by an agent:
//!
//! - what each tool costs to carry, against a budget that only goes down
//!   (`tool-budget.json`);
//! - what a name promises: a read is a noun, the tool that changes it is
//!   `edit_<noun>`, and a verb never names a read;
//! - what a description may say: no orders to the model, no advertising, no
//!   name of a tool that is not there;
//! - that an argument choosing one of a few things lists them, and that an
//!   argument nobody declared is refused instead of dropped;
//! - that a lexical tool search — what clients run when they defer tool
//!   definitions — still finds the right tool for the requests agents make,
//!   and that no two tools read alike without a reason (`tool-search.json`).
//!
//! `docs/MCP-TOOLS.md` is the same list, for people.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// Past this, a description is a manual: the rules behind a review belong in
/// its answer, where they are paid for when asked.
const DESCRIPTION_CAP: usize = 1_200;
/// Past this, an input schema is several tools sharing one name.
const INPUT_CAP: usize = 6_000;
/// A shared definition is referenced from many places; its description is
/// what the type is, in a line.
const DEF_DESCRIPTION_CAP: usize = 300;

/// Every tool there is: the editor's, and the hosted service's own.
fn every_tool() -> Vec<rmcp::model::Tool> {
    crate::tools()
        .into_iter()
        .chain(crate::hosted::tools())
        .collect()
}

fn surface() -> Vec<(String, Value)> {
    every_tool()
        .into_iter()
        .map(|t| (t.name.to_string(), serde_json::to_value(&t).expect("tool")))
        .collect()
}

fn bytes(value: &Value) -> usize {
    serde_json::to_string(value).expect("json").len()
}

fn is_read(tool: &Value) -> bool {
    tool["annotations"]["readOnlyHint"].as_bool() == Some(true)
}

/// The server instructions as the desktop hands them out, and as the hosted
/// service does: the words every description rule holds them to.
fn instructions() -> Vec<(String, String)> {
    use rmcp::ServerHandler as _;
    let server = crate::NewEraMcp::new(newera_core::SharedDocument::new(
        newera_core::Document::default(),
    ));
    vec![
        (
            "instructions".to_owned(),
            server.get_info().instructions.unwrap_or_default(),
        ),
        (
            "hosted instructions".to_owned(),
            crate::surface::instructions(crate::Transport::Cloud),
        ),
    ]
}

/// Every `description` in a schema, with where it sits.
fn descriptions<'a>(value: &'a Value, path: &str, out: &mut Vec<(String, &'a str)>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                if key == "description" {
                    if let Some(text) = inner.as_str() {
                        out.push((path.to_owned(), text));
                    }
                } else {
                    descriptions(inner, &format!("{path}.{key}"), out);
                }
            }
        }
        Value::Array(items) => {
            for (i, inner) in items.iter().enumerate() {
                descriptions(inner, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// Every property name and enum value in a schema: the words a description
/// may use with an underscore in them without being a tool.
fn vocabulary(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(props)) = map.get("properties") {
                out.extend(props.keys().cloned());
            }
            if let Some(Value::Array(values)) = map.get("enum") {
                out.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            for inner in map.values() {
                vocabulary(inner, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| vocabulary(i, out)),
        _ => {}
    }
}

// ---------------------------------------------------------------- budget

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Cost {
    total: usize,
    description: usize,
    input: usize,
    output: usize,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Budget {
    /// `tools/list` as one compact JSON array, in bytes.
    total: usize,
    tools: BTreeMap<String, Cost>,
}

fn costs() -> (usize, BTreeMap<String, Cost>) {
    let tools = every_tool();
    let total = serde_json::to_string(&tools).expect("json").len();
    let each = surface()
        .into_iter()
        .map(|(name, tool)| {
            let cost = Cost {
                total: bytes(&tool),
                description: tool["description"].as_str().map_or(0, str::len),
                input: bytes(&tool["inputSchema"]),
                output: tool.get("outputSchema").map_or(0, bytes),
            };
            (name, cost)
        })
        .collect();
    (total, each)
}

fn budget_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tool-budget.json")
}

/// The surface costs what the budget says, or less. A tool that grows past
/// what was recorded fails here, and a new one has to fit the caps; shrinking
/// is free, and `update_tool_budget` then locks the gain in. Growing on
/// purpose means recording it, which that command refuses for a part already
/// over its cap.
#[test]
fn every_tool_fits_its_budget() {
    let budget: Budget =
        serde_json::from_str(&std::fs::read_to_string(budget_path()).expect("tool-budget.json"))
            .expect("budget");
    let (total, each) = costs();
    let mut over = Vec::new();
    for (name, cost) in &each {
        let Some(recorded) = budget.tools.get(name) else {
            if cost.description > DESCRIPTION_CAP || cost.input > INPUT_CAP {
                over.push(format!(
                    "{name} is new and over the caps (description {} of {DESCRIPTION_CAP}, input {} of {INPUT_CAP})",
                    cost.description, cost.input
                ));
            } else {
                over.push(format!(
                    "{name} is new: record it with `cargo test -p newera-mcp update_tool_budget -- --ignored`"
                ));
            }
            continue;
        };
        for (part, now, was) in [
            ("description", cost.description, recorded.description),
            ("input", cost.input, recorded.input),
            ("output", cost.output, recorded.output),
            ("total", cost.total, recorded.total),
        ] {
            if now > was {
                over.push(format!("{name}.{part}: {now} bytes, budget {was}"));
            }
        }
    }
    if total > budget.total {
        over.push(format!(
            "tools/list: {total} bytes, budget {}",
            budget.total
        ));
    }
    assert!(
        over.is_empty(),
        "the tool surface grew past its budget (docs/MCP-TOOLS.md says how to pay for it):\n  {}",
        over.join("\n  ")
    );
}

/// Explicit maintenance command: records what the surface costs now. It
/// refuses to record a part that is over its cap and grew — that one has to
/// shrink, not be written down.
#[test]
#[ignore = "rewrites tests/fixtures/tool-budget.json; review the diff"]
fn update_tool_budget() {
    let old: Option<Budget> = std::fs::read_to_string(budget_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    // The first record takes the surface as it is: the caps bind from then on.
    let first = old.as_ref().is_none_or(|b| b.tools.is_empty());
    let (total, each) = costs();
    for (name, cost) in &each {
        if first {
            break;
        }
        let was = old.as_ref().and_then(|b| b.tools.get(name));
        for (part, now, before, cap) in [
            (
                "description",
                cost.description,
                was.map(|c| c.description),
                DESCRIPTION_CAP,
            ),
            ("input", cost.input, was.map(|c| c.input), INPUT_CAP),
        ] {
            assert!(
                now <= cap || before.is_some_and(|b| now <= b),
                "{name}.{part} is {now} bytes, over the cap of {cap}: shrink it instead"
            );
        }
    }
    let mut rows: Vec<_> = each.iter().collect();
    rows.sort_by_key(|(_, c)| std::cmp::Reverse(c.total));
    println!(
        "{} tools, {total} bytes (~{} tokens)",
        each.len(),
        total / 4
    );
    for (name, c) in rows {
        println!(
            "{:6} {name:20} description {:5} input {:5} output {:5}",
            c.total, c.description, c.input, c.output
        );
    }
    let budget = Budget { total, tools: each };
    let json = serde_json::to_string_pretty(&budget).expect("json");
    std::fs::write(budget_path(), format!("{json}\n")).expect("write");
}

// ---------------------------------------------------------------- names

/// How a write starts. A read named like one would be approved as a write,
/// and a write named like a read would run without asking.
const WRITE_VERBS: &[&str] = &[
    "edit_", "set_", "create", "update", "delete", "move", "place", "arrange", "accept", "new_",
    "open_", "save_", "export", "run_", "fill_", "split_", "merge_", "embed", "undo", "redo",
];

/// Kinds of element an `edit_` tool may change that have no read of their
/// own: `home` reads them, with every other element.
const READ_IN_HOME: &[&str] = &["walls"];

#[test]
fn names_say_what_a_tool_does() {
    let tools = surface();
    let by_name: BTreeMap<&str, &Value> = tools.iter().map(|(n, t)| (n.as_str(), t)).collect();
    let mut wrong = Vec::new();
    for (name, tool) in &tools {
        let shaped = name.len() <= 32
            && name.starts_with(|c: char| c.is_ascii_lowercase())
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !shaped {
            wrong.push(format!("{name}: snake_case, at most 32 characters"));
        }
        if is_read(tool)
            && let Some(verb) = WRITE_VERBS.iter().find(|v| name.starts_with(**v))
        {
            wrong.push(format!(
                "{name} is a read named with the write verb `{verb}`"
            ));
        }
        if let Some(read) = name.strip_prefix("edit_") {
            if READ_IN_HOME.contains(&read) {
                continue;
            }
            match by_name.get(read) {
                Some(t) if is_read(t) => {}
                Some(_) => wrong.push(format!("{name} edits `{read}`, which is not a read")),
                None => wrong.push(format!("{name} edits `{read}`, and no tool reads it")),
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

// ---------------------------------------------------------------- words

/// How a tool name with an underscore starts. A snake-case word in a
/// description that starts like one and is no tool, argument or answer field
/// is a tool that was renamed or never was.
const TOOL_PREFIXES: &[&str] = &[
    "edit", "set", "get", "new", "open", "save", "export", "render", "show", "trace", "fill",
    "split", "merge", "run", "check", "fit", "cut", "cabinet",
];

/// Names that once were tools. Any of them in a description, the server
/// instructions or a skill sends an agent to a tool that is not there.
const RETIRED: &[&str] = &[
    "get_home",
    "set_home",
    "check_layout",
    "trace_background",
    "trace_walls",
    "set_background",
    "fill_lighting",
    "split_wall",
    "merge_walls",
    "export_plan",
    "export_cut_list",
    "new_home",
    "open_home",
    "save_home",
];

/// A description names tools that exist. A rename that leaves the old name
/// in another tool's description — or in the server instructions — sends
/// agents to a tool that is not there.
#[test]
fn descriptions_name_tools_that_exist() {
    let tools = surface();
    let names: BTreeSet<String> = tools.iter().map(|(n, _)| n.clone()).collect();
    let mut known = names.clone();
    for (_, tool) in &tools {
        vocabulary(tool, &mut known);
    }
    let word = regex_lite::Regex::new(r"\b[a-z][a-z0-9]*(?:_[a-z0-9]+)+\b").expect("regex");
    let bare = regex_lite::Regex::new(r"\b[a-z][a-z0-9_]*\b").expect("regex");
    let mut texts: Vec<(String, String)> = instructions();
    for (name, tool) in &tools {
        let mut found = Vec::new();
        descriptions(tool, name, &mut found);
        texts.extend(found.into_iter().map(|(p, t)| (p, t.to_owned())));
    }
    let mut unknown = BTreeSet::new();
    for (place, text) in &texts {
        for m in word.find_iter(text) {
            let w = m.as_str();
            let prefix = w.split('_').next().unwrap_or(w);
            if TOOL_PREFIXES.contains(&prefix) && !known.contains(w) {
                unknown.insert(format!("{place}: `{w}`"));
            }
        }
        for m in bare.find_iter(text) {
            if RETIRED.contains(&m.as_str()) && !names.contains(m.as_str()) {
                unknown.insert(format!("{place}: `{}` was retired", m.as_str()));
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "descriptions name tools that do not exist:\n  {}",
        unknown.into_iter().collect::<Vec<_>>().join("\n  ")
    );
}

/// What `docs/DISTRIBUTION.md` promises the directories: a description says
/// what the tool does and never orders the model around or sells.
const BANNED: &[&str] = &[
    "you must",
    "you should",
    "always use",
    "always call",
    "make sure",
    "important:",
    "do not forget",
    "don't forget",
    "powerful",
    "amazing",
    "seamless",
    "world-class",
    "state of the art",
    "best-in-class",
    "!",
];

#[test]
fn descriptions_neither_order_nor_sell() {
    let tools = surface();
    let mut texts: Vec<(String, String)> = instructions();
    for (name, tool) in &tools {
        let mut found = Vec::new();
        descriptions(tool, name, &mut found);
        texts.extend(found.into_iter().map(|(p, t)| (p, t.to_owned())));
    }
    let mut bad = Vec::new();
    for (place, text) in &texts {
        let lower = text.to_lowercase();
        for phrase in BANNED {
            if lower.contains(phrase) {
                bad.push(format!("{place}: \"{phrase}\""));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// A shared type says what it is in a line. A long doc comment above one is
/// usually a function's, landed on the type by accident — and it is paid for
/// in every tool that uses the type.
#[test]
fn shared_definitions_are_described_in_a_line() {
    let mut long = Vec::new();
    for (name, tool) in surface() {
        if let Some(Value::Object(defs)) = tool["inputSchema"].get("$defs") {
            for (def, schema) in defs {
                let n = schema["description"].as_str().map_or(0, str::len);
                if n > DEF_DESCRIPTION_CAP {
                    long.push(format!("{name}.$defs.{def}: {n} bytes"));
                }
            }
        }
    }
    assert!(long.is_empty(), "{}", long.join("\n"));
}

// ---------------------------------------------------------------- arguments

/// Arguments that pick one of a few things. Their choices belong in the
/// schema as an `enum`, where a client validates them and a search finds them.
const CHOICE_ARGUMENTS: &[&str] = &[
    "action", "kind", "view", "format", "what", "mode", "quality",
];

fn open_choices(value: &Value, path: &str, out: &mut BTreeSet<String>) {
    let Value::Object(map) = value else { return };
    if let Some(Value::Object(props)) = map.get("properties") {
        for (key, schema) in props {
            let text = schema["type"] == "string" && schema.get("enum").is_none();
            if CHOICE_ARGUMENTS.contains(&key.as_str()) && text {
                out.insert(format!("{path}.{key}"));
            }
        }
    }
    for (key, inner) in map {
        open_choices(inner, &format!("{path}.{key}"), out);
    }
}

#[test]
fn choices_are_listed_in_the_schema() {
    let mut found = BTreeSet::new();
    for (name, tool) in surface() {
        open_choices(&tool["inputSchema"], &name, &mut found);
    }
    assert!(
        found.is_empty(),
        "choice arguments typed as free text — give them an enum (#[schemars(extend(\"enum\" = [...]))]):\n  {}",
        found.into_iter().collect::<Vec<_>>().join("\n  ")
    );
}

/// Tools too slow to call for a probe, and whose arguments are checked the
/// same way as their neighbours'.
const TOO_SLOW_TO_PROBE: &[&str] = &["render_photo"];

/// An argument nobody declared is refused by name. Dropped in silence, a
/// misspelled `dry` would write, and a write's argument sent to a read would
/// look like a change that was made. `crate::args` checks every call against
/// the tool's schema; this makes sure nothing goes around it.
#[test]
fn unknown_arguments_are_refused_by_name() {
    use newera_core::{Document, SharedDocument};
    let mut loose = BTreeSet::new();
    for (name, _) in surface() {
        if TOO_SLOW_TO_PROBE.contains(&name.as_str()) {
            continue;
        }
        let probe = serde_json::json!({"zz_not_an_argument": 1});
        // The hosted service checks its own tools the same way, before it
        // runs them.
        let named = if crate::surface::HOSTED_ONLY.contains(&name.as_str()) {
            crate::surface::unknown_hosted_argument(&name, &probe)
                .is_some_and(|why| why.contains("zz_not_an_argument"))
        } else {
            let document = SharedDocument::new(Document::default());
            let answer = crate::call(document, &name, probe);
            matches!(&answer, Err(why) if why.contains("zz_not_an_argument"))
        };
        if !named {
            loose.insert(name);
        }
    }
    assert!(
        loose.is_empty(),
        "these tools accept an argument they do not know (crate::args should refuse it):\n  {}",
        loose.into_iter().collect::<Vec<_>>().join("\n  ")
    );
}

// ---------------------------------------------------------------- search

/// A tool search index built the way clients build theirs: the name, the
/// description, the argument names and their descriptions, ranked by BM25.
struct Index {
    names: Vec<String>,
    docs: Vec<BTreeMap<String, f64>>,
    lengths: Vec<f64>,
    frequency: BTreeMap<String, f64>,
}

const STOP: &[&str] = &[
    "a", "an", "the", "of", "to", "in", "on", "at", "by", "for", "and", "or", "is", "it", "its",
    "with", "as", "be", "that", "this", "from", "one", "every", "each", "what", "when", "which",
    "are", "no", "not", "any", "all", "into", "than", "then", "there", "so", "if", "else", "me",
    "my", "i", "we", "our", "you", "your", "please", "can", "do", "does", "how", "gives", "e", "g",
];

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .filter(|w| !STOP.contains(&w.as_str()))
        .map(|w| {
            if let Some(stem) = w.strip_suffix("ies") {
                format!("{stem}y")
            } else if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
                w[..w.len() - 1].to_owned()
            } else {
                w
            }
        })
        .collect()
}

impl Index {
    fn new(tools: &[(String, Value)]) -> Self {
        let mut names = Vec::new();
        let mut docs = Vec::new();
        let mut lengths = Vec::new();
        let mut frequency: BTreeMap<String, f64> = BTreeMap::new();
        for (name, tool) in tools {
            let mut text = format!("{name} {name} {name} ");
            text.push_str(tool["description"].as_str().unwrap_or_default());
            let mut props = BTreeSet::new();
            vocabulary(&tool["inputSchema"], &mut props);
            for p in props {
                text.push(' ');
                text.push_str(&p);
            }
            let mut found = Vec::new();
            descriptions(&tool["inputSchema"], "", &mut found);
            for (_, d) in found {
                text.push(' ');
                text.push_str(d);
            }
            let mut counts: BTreeMap<String, f64> = BTreeMap::new();
            let tokens = words(&text);
            for t in &tokens {
                *counts.entry(t.clone()).or_default() += 1.0;
            }
            for t in counts.keys() {
                *frequency.entry(t.clone()).or_default() += 1.0;
            }
            #[allow(clippy::cast_precision_loss)]
            lengths.push(tokens.len() as f64);
            names.push(name.clone());
            docs.push(counts);
        }
        Self {
            names,
            docs,
            lengths,
            frequency,
        }
    }

    /// Tool names, best match first.
    fn rank(&self, query: &str) -> Vec<&str> {
        const K1: f64 = 1.2;
        const B: f64 = 0.75;
        #[allow(clippy::cast_precision_loss)]
        let n = self.docs.len() as f64;
        #[allow(clippy::cast_precision_loss)]
        let average = self.lengths.iter().sum::<f64>() / n;
        let terms = words(query);
        let mut scored: Vec<(f64, &str)> = self
            .docs
            .iter()
            .enumerate()
            .map(|(i, doc)| {
                let score = terms
                    .iter()
                    .map(|t| {
                        let tf = doc.get(t).copied().unwrap_or(0.0);
                        let df = self.frequency.get(t).copied().unwrap_or(0.0);
                        let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                        idf * tf * (K1 + 1.0)
                            / (tf + K1 * (1.0 - B + B * self.lengths[i] / average))
                    })
                    .sum::<f64>();
                (score, self.names[i].as_str())
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
        scored.into_iter().map(|(_, name)| name).collect()
    }
}

#[derive(Debug, serde::Deserialize)]
struct Floor {
    top3: f64,
    top5: f64,
}

#[derive(Debug, serde::Deserialize)]
struct Search {
    /// The share of queries whose tool must rank in the first three and
    /// five: a floor that only rises.
    floor: Floor,
    /// `[query, tool]`: what an agent searches for, and the tool that answers.
    queries: Vec<(String, String)>,
    /// `[query, why]`: requests the desktop answers and the hosted service
    /// cannot, so they are not ranked among the hosted service's tools.
    #[serde(default)]
    not_on_cloud: Vec<(String, String)>,
    /// `[tool, tool, why]`: pairs allowed to read alike, and why they must.
    alike: Vec<(String, String, String)>,
}

fn search_fixture() -> Search {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tool-search.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("tool-search.json"))
        .expect("tool-search.json")
}

/// A client that defers tool definitions shows the model only the names and
/// lets it search the rest. A tool its request cannot find is a tool that
/// does not exist, however good it is. Each place is searched as its client
/// sees it. The editor's tools hold the floor on the desktop and on the
/// hosted service, where the service's own tools (`projects`, `import_product`…)
/// compete with them. The service's own tools are searched where they are
/// offered, and each of their requests finds its tool in the first three.
#[test]
fn a_tool_search_finds_the_right_tool() {
    let fixture = search_fixture();
    let places = [
        (
            "desktop",
            crate::tools()
                .into_iter()
                .map(|t| (t.name.to_string(), serde_json::to_value(&t).expect("tool")))
                .collect::<Vec<_>>(),
        ),
        (
            "hosted",
            crate::surface::tools(crate::Transport::Cloud)
                .into_iter()
                .map(|t| (t["name"].as_str().unwrap_or_default().to_owned(), t))
                .collect(),
        ),
    ];
    for (query, tool) in &fixture.queries {
        assert!(
            places
                .iter()
                .any(|(_, tools)| tools.iter().any(|(n, _)| n == tool)),
            "query \"{query}\" expects `{tool}`, which is not a tool"
        );
    }
    // What is ranked where, and the floor each holds: the desktop's tools
    // among the desktop's, the editor's tools among the hosted service's
    // too (its own tools compete with them there), and the service's own
    // tools among the hosted service's.
    let groups = [
        ("desktop", 0, Some(false), None),
        ("hosted, editor tools", 1, Some(false), None),
        ("hosted, own tools", 1, Some(true), Some((1.0, 1.0))),
    ];
    let mut worse = Vec::new();
    for (place, at, own_tools, fixed) in groups {
        let tools = &places[at].1;
        let index = Index::new(tools);
        let names: BTreeSet<&str> = tools.iter().map(|(n, _)| n.as_str()).collect();
        let (mut top3, mut top5, mut n) = (0.0, 0.0, 0.0);
        let mut misses = Vec::new();
        for (query, tool) in &fixture.queries {
            let own = crate::surface::HOSTED_ONLY.contains(&tool.as_str());
            let cannot =
                place.starts_with("hosted") && fixture.not_on_cloud.iter().any(|(q, _)| q == query);
            if !names.contains(tool.as_str()) || Some(own) != own_tools || cannot {
                continue;
            }
            n += 1.0;
            let ranked = index.rank(query);
            let found = ranked.iter().position(|n| n == tool).unwrap_or(usize::MAX);
            if found < 3 {
                top3 += 1.0;
            }
            // The service's own tools hold a floor of the first three alone.
            if found < 5 && (found < 3 || fixed.is_none()) {
                top5 += 1.0;
            }
            if found >= 3 {
                misses.push(format!(
                    "\"{query}\" → `{tool}` ranked {}; first: {}",
                    found.saturating_add(1),
                    ranked[..3].join(", ")
                ));
            }
        }
        let (top3, top5) = (top3 / n, top5 / n);
        println!("{place}: top3 {top3:.3} top5 {top5:.3} over {n} queries");
        for miss in &misses {
            println!("  {miss}");
        }
        let (floor3, floor5) = fixed.unwrap_or((fixture.floor.top3, fixture.floor.top5));
        if top3 + 1e-9 < floor3 || top5 + 1e-9 < floor5 {
            worse.push(format!(
                "{place}: top3 {top3:.3} (floor {floor3}), top5 {top5:.3} (floor {floor5}); misses:\n  {}",
                misses.join("\n  ")
            ));
        }
    }
    assert!(
        worse.is_empty(),
        "tool search got worse: {}",
        worse.join("\n")
    );
}

/// How alike two descriptions read, as a search sees them: the cosine of
/// their TF-IDF vectors over names and descriptions only.
fn likeness(tools: &[(String, Value)]) -> Vec<(f64, String, String)> {
    let texts: Vec<BTreeMap<String, f64>> = tools
        .iter()
        .map(|(name, tool)| {
            let mut counts = BTreeMap::new();
            let text = format!(
                "{name} {}",
                tool["description"].as_str().unwrap_or_default()
            );
            for w in words(&text) {
                *counts.entry(w).or_insert(0.0) += 1.0;
            }
            counts
        })
        .collect();
    let mut df: BTreeMap<&str, f64> = BTreeMap::new();
    for t in &texts {
        for w in t.keys() {
            *df.entry(w.as_str()).or_default() += 1.0;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let n = texts.len() as f64;
    let vectors: Vec<BTreeMap<&str, f64>> = texts
        .iter()
        .map(|t| {
            t.iter()
                .map(|(w, tf)| (w.as_str(), tf * (n / df[w.as_str()]).ln()))
                .collect()
        })
        .collect();
    let norm = |v: &BTreeMap<&str, f64>| v.values().map(|x| x * x).sum::<f64>().sqrt();
    let mut pairs = Vec::new();
    for i in 0..tools.len() {
        for j in i + 1..tools.len() {
            let dot: f64 = vectors[i]
                .iter()
                .filter_map(|(w, x)| vectors[j].get(w).map(|y| x * y))
                .sum();
            let cos = dot / (norm(&vectors[i]) * norm(&vectors[j])).max(f64::EPSILON);
            let (a, b) = if tools[i].0 < tools[j].0 {
                (&tools[i].0, &tools[j].0)
            } else {
                (&tools[j].0, &tools[i].0)
            };
            pairs.push((cos, a.clone(), b.clone()));
        }
    }
    pairs.sort_by(|x, y| y.0.total_cmp(&x.0));
    pairs
}

/// Past this, two tools read as one to a search and to a model choosing
/// between them.
const ALIKE: f64 = 0.35;

/// Two tools that read alike are chosen between by luck. Every such pair is
/// written down with the reason it has to stay two tools; a pair that no
/// longer reads alike leaves the list.
#[test]
fn no_two_tools_read_alike_without_a_reason() {
    let tools = surface();
    let fixture = search_fixture();
    let listed: BTreeSet<(String, String)> = fixture
        .alike
        .iter()
        .map(|(a, b, _)| {
            if a < b {
                (a.clone(), b.clone())
            } else {
                (b.clone(), a.clone())
            }
        })
        .collect();
    let pairs = likeness(&tools);
    for (cos, a, b) in pairs.iter().take(15) {
        println!("{cos:.3} {a} {b}");
    }
    let found: BTreeSet<(String, String)> = pairs
        .iter()
        .filter(|(cos, ..)| *cos >= ALIKE)
        .map(|(_, a, b)| (a.clone(), b.clone()))
        .collect();
    let new: Vec<_> = found
        .difference(&listed)
        .map(|(a, b)| format!("{a} ~ {b}"))
        .collect();
    let gone: Vec<_> = listed
        .difference(&found)
        .map(|(a, b)| format!("{a} ~ {b}"))
        .collect();
    assert!(
        new.is_empty(),
        "these tools read alike — tell them apart, or say why in tool-search.json `alike`:\n  {}",
        new.join("\n  ")
    );
    assert!(
        gone.is_empty(),
        "these no longer read alike; take them out of tool-search.json `alike`:\n  {}",
        gone.join("\n  ")
    );
}
