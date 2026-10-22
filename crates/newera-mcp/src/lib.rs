//! MCP server for 3D New Era AI.
//!
//! Tools are thin adapters: they translate MCP parameters into
//! [`newera_core::Command`]s and execute them on the [`SharedDocument`]
//! that the desktop UI is also rendering. Whatever an agent does shows up on
//! screen immediately and can be undone with Ctrl+Z.

pub mod app;
mod args;
mod compact;
mod edit;
mod hints;
mod hosted;
pub mod output;
mod schema;
pub mod surface;
#[cfg(test)]
mod surface_rules;
mod tools;
mod trace;

use newera_core::SharedDocument;

pub use surface::Transport;
pub use tools::NewEraMcp;
pub use tools::render::PhotoAsk;

// The transports need an operating system to talk through: a pipe, a socket.
// A browser tab has neither, and does not need them — there the tools are
// called straight through [`call`], and the protocol is spoken by the relay in
// front of the tab.
#[cfg(not(target_arch = "wasm32"))]
mod transports {
    use std::sync::Arc;

    use newera_core::SharedDocument;
    use rmcp::ServiceExt;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    use tokio_util::sync::CancellationToken;

    use super::NewEraMcp;

    /// Serves MCP over stdin/stdout until the client disconnects.
    pub async fn serve_stdio(document: SharedDocument) -> std::io::Result<()> {
        let service = NewEraMcp::new(document)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(std::io::Error::other)?;
        service.waiting().await.map_err(std::io::Error::other)?;
        Ok(())
    }

    /// Builds a Streamable HTTP MCP service, ready to be mounted on a router.
    ///
    /// `allowed_hosts` guards against DNS rebinding; keep it to loopback names
    /// unless the server is intentionally exposed.
    pub fn http_service(
        document: SharedDocument,
        allowed_hosts: Vec<String>,
        shutdown: CancellationToken,
    ) -> StreamableHttpService<NewEraMcp, LocalSessionManager> {
        let mut config = StreamableHttpServerConfig::default();
        config.allowed_hosts = allowed_hosts;
        config.cancellation_token = shutdown;
        StreamableHttpService::new(
            move || Ok(NewEraMcp::new(document.clone())),
            Arc::new(LocalSessionManager::default()),
            config,
        )
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use transports::{http_service, serve_stdio};

/// How a call names a shop product of the hosted service where a model file
/// or a product id goes: `place model=product:<id>`, `edit_finishes
/// product=product:<id>`. The hosted service puts the product's file in the
/// project and rewrites the reference before the call runs; one that arrives
/// here unrewritten — on the desktop, or in a tab on the standalone relay —
/// is refused with [`PRODUCTS_ARE_HOSTED`].
pub const PRODUCT_REF: &str = "product:";

/// Why a `product:<id>` reference is refused where there is no product library.
pub const PRODUCTS_ARE_HOSTED: &str = "products exist only on the hosted service";

/// Every tool this server has, with the schema an AI client needs to call it.
///
/// The desktop hands this out over the MCP handshake; a browser tab has no
/// handshake of its own, so it asks here and the relay in front of it answers
/// `tools/list` with exactly what this window can do.
pub fn tools() -> Vec<rmcp::model::Tool> {
    NewEraMcp::new(SharedDocument::new(newera_core::Document::default())).tools()
}

/// What `render_photo` with `args` would render, for a renderer outside
/// this process: the scene, the size and the quality, the way the tool
/// itself reads them. `None` when the call renders nothing (`estimate`,
/// `cancel`), which the tool answers itself.
///
/// # Errors
///
/// As [`call`] refuses the same arguments: the reason.
pub fn photo_scene(
    document: SharedDocument,
    args: serde_json::Value,
) -> Result<Option<PhotoAsk>, String> {
    let (server, p) = photo_call(document, args)?;
    server.photo_scene(&p).map_err(|e| e.message.to_string())
}

/// `render_photo` with `args`, for a call that started at `started` and
/// spent the time since on another renderer: its `max_s` counts from
/// `started`, and once spent the photo is refused as the tool refuses one
/// past it. Answers as [`call`] does.
///
/// # Errors
///
/// As [`call`]: the reason.
pub fn render_photo_since(
    document: SharedDocument,
    args: serde_json::Value,
    started: web_time::Instant,
) -> Result<rmcp::model::CallToolResult, String> {
    let (server, p) = photo_call(document, args)?;
    let mut result = server
        .render_photo_since(&p, started)
        .map_err(|e| e.message.to_string())?;
    output::structure(&mut result);
    Ok(result)
}

/// The tools on `document`, and `render_photo`'s arguments read from `args`
/// as [`call`] reads them.
fn photo_call(
    document: SharedDocument,
    args: serde_json::Value,
) -> Result<(NewEraMcp, tools::render::PhotoParams), String> {
    let server = NewEraMcp::new(document);
    if let Some(why) = server.unknown_argument("render_photo", &args) {
        return Err(why);
    }
    Ok((server, params(args)?))
}

/// A tool's arguments; none given reads as an empty object.
fn params<T: serde::de::DeserializeOwned>(args: serde_json::Value) -> Result<T, String> {
    let args = if args.is_null() {
        serde_json::json!({})
    } else {
        args
    };
    serde_json::from_value(args).map_err(|e| format!("arguments: {e}"))
}

/// Runs one tool against a document, with no MCP session in sight.
///
/// The protocol — the handshake, the session, the transport — is one thing;
/// doing the work is another. Everything that cannot hold a session of its own
/// comes through here: the REST analyses, and the editor running in a browser
/// tab, whose tool calls arrive over a relay instead of a socket.
///
/// Unknown names and arguments that do not fit come back as `Err` with the
/// reason, which is what an agent needs to fix its own call.
pub fn call(
    document: SharedDocument,
    name: &str,
    args: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, String> {
    let mut result = run(document, name, args)?;
    output::structure(&mut result);
    Ok(result)
}

/// [`call`], before its answer is laid out as fields.
fn run(
    document: SharedDocument,
    name: &str,
    args: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, String> {
    use rmcp::handler::server::wrapper::Parameters;

    // What a tool that answers in words gives back, in the shape the protocol
    // wants.
    fn said(text: String) -> rmcp::model::CallToolResult {
        rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text(text)])
    }
    let reason = |e: rmcp::ErrorData| e.message.to_string();
    let server = NewEraMcp::new(document);
    if let Some(why) = server.unknown_argument(name, &args) {
        return Err(why);
    }
    match name {
        "disciplines" => Ok(said(
            server
                .read_disciplines(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_disciplines" => Ok(said(
            server
                .edit_disciplines(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "annotations" => Ok(said(
            server
                .read_annotations(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_annotations" => Ok(said(
            server
                .edit_annotations(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "background" => Ok(said(
            server
                .read_trace(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "embed" => Ok(said(
            server.embed(Parameters(params(args)?)).map_err(reason)?,
        )),
        "cabinet_run" => Ok(said(
            server
                .cabinet_run(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "cameras" => Ok(said(
            server
                .list_cameras(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_cameras" => Ok(said(
            server
                .edit_cameras(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "video" => Ok(said(
            server
                .list_video(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_video" => Ok(said(
            server
                .edit_video(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "ergonomics" => Ok(said(
            server
                .read_ergonomics(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "layout" => Ok(said(
            server
                .read_check_layout(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "accept" => Ok(said(
            server.accept(Parameters(params(args)?)).map_err(reason)?,
        )),
        "electrical" => Ok(said(
            server
                .read_electrical(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_electrical" => Ok(said(
            server
                .edit_electrical(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "create" => Ok(said(
            server.create(Parameters(params(args)?)).map_err(reason)?,
        )),
        "update" => Ok(said(
            server.update(Parameters(params(args)?)).map_err(reason)?,
        )),
        "delete" => Ok(said(
            server.delete(Parameters(params(args)?)).map_err(reason)?,
        )),
        "move" => Ok(said(
            server
                .move_elements(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "place" => Ok(said(
            server.place(Parameters(params(args)?)).map_err(reason)?,
        )),
        "arrange" => Ok(said(
            server.arrange(Parameters(params(args)?)).map_err(reason)?,
        )),
        "joinery" => Ok(said(
            server.joinery(Parameters(params(args)?)).map_err(reason)?,
        )),
        "cut_list" => Ok(said(
            server
                .read_cut_list(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "levels" => Ok(said(
            server
                .list_levels(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_levels" => Ok(said(
            server
                .edit_levels(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "lighting" => Ok(said(
            server
                .read_lighting(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_lighting" => Ok(said(
            server
                .fill_lighting(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "measure" => Ok(said(
            server.measure(Parameters(params(args)?)).map_err(reason)?,
        )),
        "model" => Ok(said(
            server.model(Parameters(params(args)?)).map_err(reason)?,
        )),
        "library" => Ok(said(
            server.library(Parameters(params(args)?)).map_err(reason)?,
        )),
        "edit_library" => Ok(said(
            server
                .edit_library(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_model" => Ok(said(
            server
                .edit_model(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "plumbing" => Ok(said(
            server
                .read_plumbing(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_plumbing" => Ok(said(
            server
                .edit_plumbing(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "finishes" => Ok(said(
            server
                .read_finishes(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_finishes" => Ok(said(
            server
                .edit_finishes(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "takeoff" => Ok(said(
            server.takeoff(Parameters(params(args)?)).map_err(reason)?,
        )),
        "edit_home" => Ok(said(
            server.set_home(Parameters(params(args)?)).map_err(reason)?,
        )),
        "show_plan" => Ok(server.show_plan()),
        "plugins" => Ok(said(
            server
                .list_plugins(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "run_plugin" => Ok(said(
            server
                .run_plugin(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "sessions" => Ok(said(server.sessions())),
        "variants" => Ok(said(
            server
                .list_variants(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_variants" => Ok(said(
            server
                .edit_variants(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "history" => Ok(said(
            server
                .list_checkpoints(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "home" => Ok(said(
            server.get_home(Parameters(params(args)?)).map_err(reason)?,
        )),
        "materials" => Ok(said(server.materials())),
        "catalog" => Ok(said(server.catalog(Parameters(params(args)?)))),
        "render_plan" => server
            .render_plan(Parameters(params(args)?))
            .map_err(reason),
        "render_3d" => server.render_3d(Parameters(params(args)?)).map_err(reason),
        "render_photo" => server
            .render_photo(Parameters(params(args)?))
            .map_err(reason),
        "file" => Ok(said(
            server.file(Parameters(params(args)?)).map_err(reason)?,
        )),
        "edit_history" => Ok(said(
            server
                .edit_history(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "edit_walls" => Ok(said(
            server
                .edit_walls(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "export" => Ok(said(
            server.export(Parameters(params(args)?)).map_err(reason)?,
        )),
        "edit_background" => Ok(said(
            server
                .edit_background(Parameters(params(args)?))
                .map_err(reason)?,
        )),
        "rules" => Ok(said(
            server.rules(Parameters(params(args)?)).map_err(reason)?,
        )),
        "fit_roof" => Ok(said(
            server.fit_roof(Parameters(params(args)?)).map_err(reason)?,
        )),
        "feedback" => Ok(said(
            server.feedback(Parameters(params(args)?)).map_err(reason)?,
        )),
        other => Err(format!("no tool named {other}")),
    }
}

/// The analyses an agent asks the MCP tools for, without an MCP session:
/// `layout`, `ergonomics`, `measure` and `annotations`, with the same
/// arguments and the same answer.
///
/// A script that edits the plan over REST could not ask whether the edit
/// was any good — the questions lived only behind the MCP handshake — so it
/// had to hand the question back to the agent. Unknown names and arguments
/// that do not fit answer `Err` with the reason.
pub fn analysis(
    document: SharedDocument,
    name: &str,
    args: serde_json::Value,
) -> Result<String, String> {
    // Reads only: these four answer questions and change nothing, which is why
    // they are the ones a plain HTTP request may ask for.
    if !matches!(name, "layout" | "ergonomics" | "measure" | "annotations") {
        return Err(format!(
            "no analysis {name}: layout, ergonomics, measure or annotations"
        ));
    }
    let result = call(document, name, args)?;
    Ok(said_in(&result))
}

/// What a tool said, as plain text: the analyses answer in words, and their
/// callers want the words, not the protocol's wrapping.
fn said_in(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod call_tests {
    use newera_core::{Document, SharedDocument};

    /// Every tool the server offers can be called without a session. A tool
    /// added to a router and forgotten here would be invisible to the browser,
    /// and nothing else would notice.
    #[test]
    fn every_tool_answers_without_a_session() {
        let missing: Vec<String> = super::tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .filter(|name| {
                let document = SharedDocument::new(Document::default());
                // Called with nothing: a tool that needs arguments complains
                // about the arguments, which is an answer. Only "no tool named"
                // means it is not wired here at all.
                matches!(
                    super::call(document, name, serde_json::Value::Null),
                    Err(ref why) if why.starts_with("no tool named")
                )
            })
            .collect();
        assert!(missing.is_empty(), "tools nobody can call: {missing:?}");
        assert!(super::tools().len() >= 44, "the surface shrank");
    }

    /// And the work really happens: a wall goes in, and the plan says so.
    #[test]
    fn a_call_changes_the_document() {
        let document = SharedDocument::new(Document::default());
        let made = super::call(
            document.clone(),
            "create",
            serde_json::json!({"walls": [{"pts": [[0, 0], [400, 0]]}]}),
        )
        .expect("the wall goes in");
        assert_ne!(made.content, [] as [rmcp::model::ContentBlock; 0]);
        assert_eq!(document.read().home().walls.len(), 1);

        let read = super::call(document, "home", serde_json::Value::Null).expect("read back");
        let text = super::said_in(&read);
        assert!(
            text.contains("400"),
            "the plan should mention the wall: {text}"
        );
    }

    /// A name nobody has is a mistake worth reporting, not a panic.
    #[test]
    fn an_unknown_tool_says_so() {
        let document = SharedDocument::new(Document::default());
        let answer = super::call(document, "make_coffee", serde_json::Value::Null);
        assert_eq!(answer.unwrap_err(), "no tool named make_coffee");
    }
}
