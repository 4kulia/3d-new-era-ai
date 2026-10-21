# Architecture

## The one rule

**Every change to a home is a `Command` executed by a `Document`.**

The desktop UI, the MCP server and the REST API are all just clients of that rule.
That is what makes these properties hold everywhere, for free:

- undo/redo works the same for a mouse click and for an agent's tool call;
- batches are atomic: if one step fails, nothing is applied;
- every client sees changes from every other client, because they share one document.

```text
                    ┌───────────────────────────────┐
                    │          newera-core          │
                    │  Home ─ Command ─ Document    │
                    │  (model)  (reversible) (history, revision)
                    └──────────────┬────────────────┘
                                   │ SharedDocument (Arc<RwLock<Document>>)
          ┌────────────────────────┼────────────────────────┐
┌─────────┴─────────┐   ┌──────────┴─────────┐   ┌──────────┴─────────┐
│    newera-app     │   │     newera-mcp     │   │   newera-server    │
│ egui + wgpu       │   │ rmcp tools         │   │ axum: /api, /mcp   │
│ plan 2D · view 3D │   │ stdio / HTTP       │   │                    │
└─────────┬─────────┘   └──────────┬─────────┘   └──────────┬─────────┘
          └────────────────────────┴────────────────────────┘
                                   │
                          ┌────────┴────────┐
                          │     newera      │  binary: picks the mode,
                          │  (main thread:  │  window on the main thread,
                          │   window)       │  servers on a tokio runtime
                          └─────────────────┘
```

## Crates

| Crate | Depends on | Responsibility |
|-------|-----------|----------------|
| `newera-core` | serde, schemars, geo | Model (`Home`, `Element`: walls, rooms, dimensions, labels), geometry (joins, triangulation, room detection), `Command`, `Document`, project format, the standards registry. No UI, no async, no I/O. |
| `newera-catalog` | core, tobj, gltf | Parametric furniture: procedural 3D meshes and plan symbols at any size; model import. |
| `newera-sh3d` | core, zip, image | Sweet Home 3D (`.sh3d`) import: its Java serialization read directly, embedded models and textures extracted. |
| `newera-ergonomics` | core, geo | Ergonomics and habitability review: clearances, occupancy, kitchens and accessibility, each finding tied to a source in the standards registry. |
| `newera-joinery` | core | Parametric joinery and interiors: cabinets, slatted panels, countertops, ceiling coves, sofas and cut lists. |
| `newera-draw` | core, catalog, tiny-skia | Plan scene (styled primitives in cm) and its PNG/SVG backends. |
| `newera-mcp` | core, draw, rmcp | MCP tools and their token-efficient wire format. One module per domain under `src/tools/`, each with its own router; `src/tools/mod.rs` maps them. |
| `newera-render` | core, catalog | 3D meshes, software renderer, photos, videos, GLB/OBJ export. |
| `newera-plugins` | core | Plugin discovery and runs: external programs that edit through the HTTP API. |
| `newera-server` | core, mcp, plugins, axum | HTTP transport: REST API, sessions, plugins and the Streamable HTTP MCP endpoint. It also reads the MCP traffic going by — the handshake's client name and each `tools/call` — into `Document::agents`, which is how the window can show that an agent is connected. |
| `newera-app` | core, eframe | Desktop editor (also built for the browser). Never talks to the network: what it knows about the AI side it reads from the document, beside the collaborators (`src/ai.rs`). |
| `newera-web`, `newera-editor-web` | core / app | WebAssembly viewer and the full editor in the browser. |
| `newera-relay` | axum | Lets an AI reach the editor in a browser tab: a room is two secrets, tool calls go down the tab's socket and answers come back. No database, no disk, no project data — it passes messages and forgets. |
| `newera-cloud` | mcp, relay, shop, sh3d, catalog, render, axum, sqlx | The hosted service: accounts and OAuth 2.1, cloud projects in Postgres, a headless engine and render queue, the account's library of shop products with their 3D models (a .glb attached by link, or a placeholder built at the shop's size until one is) put into the project the call that places them runs on, the priced estimate (lines added by hand, saved estimates), and billing. Mounts `newera-relay` as a library. See [DISTRIBUTION.md](DISTRIBUTION.md). |
| `newera-shop` | reqwest | Shop products for the hosted service: a product link becomes its identity (shop, market, article), its data comes from AnySite (the IKEA adapter, or the page parsed and, when that is not enough, rendered), and fixed rules read it into fields that each say where they came from; the estimate joins a project's takeoff with those fields in exact arithmetic (whole packs once per product, money as decimal text). No database and no language model. |
| `newera-telemetry` | tracing | Crash reports and usage notes sent to Sentry, off with one switch. A build without a DSN reports through the site's endpoint. |
| `newera` | all | CLI entry point and process wiring. |

Dependencies only point downwards. `newera-core` stays free of heavy
dependencies so it compiles to WebAssembly for the browser editor and viewer.

## Standards are data, not prose

A rule that cites its source inside a sentence cannot be clicked, cannot say
which edition it followed, and cannot tell how much it matters. So every
reference lives once in `newera_core::standards` — code, title, edition, who
publishes it and over what territory, what kind of source it is, and link — and
a rule carries only the short code. How much a source obliges (A obliges … E
only describes) is computed against the place the project declares, never
stored: an ABNT standard obliges in São Paulo and informs in Miami. The numbers
the rules compare against live there too, as figures with the authority that
demands each one, so a city's decree stops at the city limits and a country's
code judges only there. The tier is the ceiling on what a finding may claim, and
figures we could not confirm at the source may warn but never accuse. The
`ergonomics` reply resolves the codes it used once in `sources`, so an agent pays
for a title once instead of once per sentence, and the editor shows the same
citation as a clickable chip. See [STANDARDS.md](STANDARDS.md) and
[JURISDICTIONS.md](JURISDICTIONS.md).

## Units and coordinates

- Plan units are **centimeters** (`f64`), matching Sweet Home 3D and architectural drawings.
- Plan axes: **x grows right, y grows down** (screen-like, as in Sweet Home 3D).
- 3D view: plan `(x, y)` cm maps to world `(x, height, y)` **meters**, Y up.
- The compass `north_degrees` is clockwise from plan up (−y) to geographic north.

## Commands and history

Every element kind is one variant of `Element`, so there are only three element
commands — `Insert`, `Update` and `Remove` — plus home-level ones (`RenameHome`,
`SetCompass`, `SetBackground`) and `Batch`. Composite edits (split a wall, move
with joined walls, dimension a wall) live in `newera_core::ops` and are shared by the
editor and MCP.

`Command::apply(self, &mut Home) -> Result<Command>` applies a change and returns
its inverse. `Document` keeps two stacks of inverses:

```text
execute(c): inverse = c.apply(home); undo.push(inverse); redo.clear()
undo():     c = undo.pop(); redo.push(c.apply(home))
redo():     c = redo.pop(); undo.push(c.apply(home))
```

`Command::Batch` applies its children in order and, on the first error, applies the
inverses of what already ran, so the home is untouched. Removals remember their index,
so undo restores the original order exactly.

`Document::revision` increments on every successful change. Views compare it to
decide when to rebuild derived data (the 3D mesh is rebuilt only when it changes).

## Ids

Ids are short and typed: `w12` (wall), `r3` (room), and more prefixes as element
types are added. They come from one monotonic counter stored in `Home`, so an id is
**never reused**, even after undo. An agent can safely hold on to an id it saw earlier.

## MCP design: tokens are a budget

Agents pay for every byte they read and write, so the protocol surface is designed
like an API for a slow network:

| Principle | Example |
|-----------|---------|
| Short ids | `w12` instead of a 36-character UUID |
| Compact geometry | `[800, 0]` instead of `{"x": 800.0, "y": 0.0}` |
| Omit defaults | walls include `t`/`h` only when not 15/250 cm |
| Round numbers | 0.1 cm precision; integers without `.0` |
| Writes don't echo state | `ok rev=7 ids=w8,w9` |
| Coarse-grained tools | one `create_walls` polyline instead of N `create_wall` calls |
| Atomic multi-target tools | `delete` takes many ids of any type |

The REST API (`/api/home`) serves the full, self-describing model for tooling and
debugging; MCP serves the compact view.

## One drawing, every output

`newera_draw::plan_scene` turns a home into styled primitives (fills with precomputed
triangles, lines, texts, images) in plan centimeters. The editor paints them with egui,
`render_png` rasterizes them with tiny-skia for MCP and export, and `to_svg` writes them
at true scale. What an agent sees in `render_plan` is exactly what the user sees.

## Furniture

A piece is a box (`width × depth × height`, elevation, angle) plus a catalog id. The core
only needs the box for layout: collisions, door swings, and which wall a door or window
cuts (`wall_cuts`). Looks come from `newera-catalog`, where each item is a generator that
builds its mesh and plan symbol for the requested size — so resizing never distorts
proportions that matter (a sofa's armrests stay armrest-sized) and nothing is ever out of
scale. Imported models are fitted to the box instead.

## Desktop editor

- `eframe`/`egui` with the `wgpu` backend.
- Layout follows Sweet Home 3D: catalog and home tree on the left, floor plan on top,
  3D view below; both splitters are resizable.
- The 3D view renders into its **own offscreen texture** (MSAA + depth) that egui shows
  as an image. Owning the pass keeps 3D rendering independent from egui's renderer.
- Drags edit a scratch copy of the home for live preview and commit one command on release.
- Interaction is tested headlessly with `egui_kittest` (real pointer/keyboard events).
- The app polls the document revision a few times per second while idle, so edits
  from MCP/HTTP appear without user input.

## Threads

The window must own the main thread (a macOS requirement). The binary starts a tokio
runtime on a background thread for the HTTP/MCP server, binds the port **before**
opening the window so a busy port fails loudly, and cancels the server when the
window closes.

## Collaboration and plugins

Everyone edits the same `Document` through the same commands: the window, MCP
agents, REST clients and plugins. Collaborators join with `POST /api/sessions`,
report cursor, storey and selection (`POST /api/sessions/{id}`) and are listed in
`GET /api/sessions`, on the `sessions` SSE event and as named cursors on the plan.
Presence lives beside the document: it is never saved and never undone. An edit
may carry `base_revision` — if someone changed the project since, it is rejected
with 409 so the client reloads instead of overwriting blindly — and `session`,
which credits it to that collaborator. Undo history stays shared per variant.

A plugin is a folder with `plugin.json` (`name`, `title`, `description`, `command`)
in `NEWERA_PLUGINS` or `<config>/3d-new-era-ai/plugins`. Running it (Plugins menu,
`POST /api/plugins/{name}/run`, MCP `plugins`) starts the command with
`NEWERA_URL`, `NEWERA_TOKEN` and `NEWERA_SESSION`, arguments as JSON on stdin; the
program reads and edits through the public API like any other client, under its
own session, and its output is returned. `examples/plugins/room-areas` is an example in
plain Python.

A plugin as heavy as a modeler (a Blender bridge) declares `limits` —
`{"heavy": true, "threads": 2, "memory_mb": 4096, "nice": 15, "timeout_s": 900}` — and
they are applied to its process, not asked of it: on Linux, where the tools are
installed, it is pinned to that many of the cores this process may use (`taskset`), its
address space is capped (`prlimit`) and its priority lowered (`nice`, relative); a limit
whose tool is missing, or elsewhere than Linux, is not applied and the answer says so; it gets `NEWERA_THREADS`/`OMP_NUM_THREADS` for the program's own
thread flag. A heavy plugin shares one slot with photo renders, so heavy work runs one at
a time. The answer reports the peak memory and processor time read from `/proc` (not
measured elsewhere), the limits applied and those this system could not apply.

## Security

The server binds to `127.0.0.1` by default. The MCP endpoint validates the `Host`
header against loopback names to block DNS rebinding from web pages. Exposing it on
a network interface is an explicit choice (`--addr`) and requires a token
(`--token` or `NEWERA_TOKEN`, sent as a Bearer header or `?token=`); the binary
refuses to start without one.
