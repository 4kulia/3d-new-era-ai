# 3D New Era AI

**English** · [Português (Brasil)](README.pt-BR.md) · **[Website](https://3dneweraai.com/?lang=en)** · **[Try it in the browser](https://3dneweraai.com/app/)**

[![CI](https://github.com/leandrodaf/3d-new-era-ai/actions/workflows/ci.yml/badge.svg)](https://github.com/leandrodaf/3d-new-era-ai/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/leandrodaf/3d-new-era-ai)](https://github.com/leandrodaf/3d-new-era-ai/releases/latest)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

An open-source **home design, floor plan and interior design editor** written in Rust,
built so that **your AI can design with you**. It ships with a
[Model Context Protocol](https://modelcontextprotocol.io) server: Claude, ChatGPT, Codex,
Gemini, Cursor or any other MCP client draws the walls, furnishes the rooms, builds the
joinery, checks lighting and ergonomics against real standards and renders the photos,
while every change shows up live on screen and is one Ctrl+Z away.

A Sweet Home 3D alternative for Windows, macOS, Linux and the browser, in English,
Portuguese, Spanish and French.

![Editor with a furnished 105 m² apartment: catalog, rendered floor plan and live 3D view](docs/images/editor.png)

## InteriorAgent (this fork)

This repository is a fork of [3D New Era AI](https://github.com/leandrodaf/3d-new-era-ai)
that turns real shop products into a furnished, finished and priced interior, all over MCP.
An agent imports furniture and finishes from shop links, places them in the plan as 3D
models at the shop's size, puts tiles, laminate, paint and wallpaper on the surfaces, renders
photos and produces a priced estimate. It is hosted at
**<https://interioragent.dutch-atlas.com>**: one person's instance, signed in with Google and
open only to the owner's address. The rest of this README is upstream's, marked where this
fork adds to it: its site, downloads and hosted connector are upstream's own.

What it does now:

- **Product import from a shop link** (`import_product`), through [AnySite](https://anysite.io):
  IKEA through its product adapter; any other shop's page is parsed first and rendered in a
  browser only when the parse left out what the product needs. Each field records where it
  came from — VAT included by the Dutch market's rule where the shop does not say, as for
  IKEA NL — and what no page showed is listed as missing for the agent to fill
  (`edit_products`), a tile's or plank's size included when neither the table nor the title
  gives it. The same article imported again is the one kept.
- **A product library in the server's database**, with price history (`refresh` reads
  today's price and VAT again).
- **3D models generated on a GPU server** (Hunyuan3D-2.0) from the product photo, scaled to
  the shop's width, depth and height: about two minutes per piece. A placed piece shows a
  placeholder until its model is ready, then the model replaces it; `place` names every piece
  drawn as a placeholder, with its job's state or how to get a model. A failed or unreachable
  generation is tried up to five times in all, with growing waits between tries.
- **Finishes that look like the product**: a finish on a whole floor, ceiling or wall side is
  drawn with its swatch picture tiled at its real size, or a paint's colour. Swatches, model
  files and the generator's product photos are fetched as a browser asks for them, since
  shops' CDNs turn other clients away. A band of a wall
  (tile below, paint above) counts for purchase only.
- **Takeoff and a priced estimate**: m² per surface with doors and windows deducted, packs,
  cans and rolls with waste and coats, furniture by the piece, lines of your own (labour,
  delivery), totals per shop, currency and VAT basis, saved estimates, and a CSV download
  whose `packs` column gives the whole packs, cans, rolls or tiles of every line that counts
  them.
- **Photos on the server**: renders always run on the server, after an open editor saves its
  changes. `render_photo` uses Blender Cycles on the GPU server when it answers, and the
  built-in path tracer otherwise; a photo that comes out dark comes with a note saying how dark
  and what would brighten it.
- **Server projects in the web editor**: the editor at
  <https://interioragent.dutch-atlas.com/app/> opens the account's projects, saves them
  automatically and refuses a save over a newer revision. A project opened again draws its
  products' current models and finishes.

### Connect ChatGPT or Claude

In Claude or ChatGPT, add a custom connector with the address
**`https://interioragent.dutch-atlas.com/mcp`**. The client discovers the OAuth sign-in by
itself; sign in with the owner's Google account and allow the client. With the editor open
and signed in, the agent's changes appear there live; otherwise it works on the account's
active project on the server.

### How an agent works with it

1. `import_product {url}` for each piece and finish; `products` shows what is missing.
2. `edit_products set` fills it: `width_cm`, `depth_cm`, `height_cm` for a piece (all three
   are needed for its model), `finish_type` (`covering`, `paint`, `wallpaper`), `swatch_image`
   with its tile or roll size, or `colour_hex`. The set that completes a piece's photo and
   size starts its model; `edit_products generate` remakes it, e.g. at a new size.
3. `place model=product:<id>` puts the piece in the plan.
4. `edit_finishes assign {surface, product}` covers a floor, ceiling or wall side.
5. `takeoff` measures, `estimate` prices (`format=csv` for a sheet), `edit_estimate` adds
   lines and saves.
6. `render_photo` (`quality`, `max_s`) for the picture.

The full tool list is under [MCP tools](#mcp-tools); how tools are designed and checked is in
[docs/MCP-TOOLS.md](docs/MCP-TOOLS.md).

## Three ways to use it

| | What you get | Account |
|---|---|---|
| **[Desktop app](#install)** | The full editor on your machine, with a local MCP server at `127.0.0.1:7878`. Nothing leaves your computer. | none |
| **[Browser editor](https://3dneweraai.com/app/)** | The same editor in a tab (WebGPU or WebGL). Switch on its MCP and your AI edits the tab's project. | none |
| **[Hosted connector](https://3dneweraai.com/connector/)** | Add `https://mcp.3dneweraai.com/mcp` to Claude or ChatGPT and design from the chat, with projects kept in your account. | email sign-in |

The desktop app and the browser editor are free, open source and need no account. The
hosted connector is free within a quota; see [pricing](https://3dneweraai.com/pricing/).

## Install

**Windows** — in PowerShell:

```powershell
irm https://raw.githubusercontent.com/leandrodaf/3d-new-era-ai/main/scripts/install-windows.ps1 | iex
```

**macOS** (Apple Silicon and Intel) — in Terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/leandrodaf/3d-new-era-ai/main/scripts/install-macos.sh | bash
```

or with Homebrew: `brew install --cask leandrodaf/tap/3d-new-era-ai`.

The installers work for your user only, with no administrator password: the app appears
in the Start menu or Launchpad, `newera` goes on your `PATH`, the MCP server is registered
in Claude Code and Codex if you have them, and running the same command again updates it.
Uninstall with `install-macos.sh --uninstall`, or from *Settings → Apps* on Windows.

Prefer a plain download?

| [Windows (x64)](https://github.com/leandrodaf/3d-new-era-ai/releases/latest/download/newera-windows-x64.zip) | [macOS Apple Silicon](https://github.com/leandrodaf/3d-new-era-ai/releases/latest/download/newera-macos-apple-silicon.zip) | [macOS Intel](https://github.com/leandrodaf/3d-new-era-ai/releases/latest/download/newera-macos-intel.zip) | [Linux (x64)](https://github.com/leandrodaf/3d-new-era-ai/releases/latest/download/newera-linux-x64.tar.gz) |
|:---:|:---:|:---:|:---:|

<details>
<summary>First launch of a plain download</summary>

The builds are not signed with a paid Apple or Microsoft certificate, so the system asks
once:

- **Windows:** unzip and open `newera-gui.exe`; if SmartScreen appears, click
  *More info* → *Run anyway*.
- **macOS:** unzip and move *3D New Era AI* to Applications, then right-click it →
  *Open* → *Open*. On macOS 15 and later, open it once, then go to *System Settings* →
  *Privacy & Security* → *Open Anyway*, or run
  `xattr -dr com.apple.quarantine "/Applications/3D New Era AI.app"`.
- **Linux:** `tar xzf newera-linux-x64.tar.gz && ./newera/newera`. Run
  `./newera/install-desktop.sh` to add the menu entry, the icon and the `.newera` file
  type (`--uninstall` undoes it).

The one-line installers skip these prompts.
</details>

## Connect your AI

**Hosted, nothing to install.** In Claude or ChatGPT, add a custom connector with the
address `https://mcp.3dneweraai.com/mcp` and sign in with your email. Step by step:
<https://3dneweraai.com/connector/>.

**With the desktop app.** The editor serves MCP at `http://127.0.0.1:7878/mcp` while it is
open (or run `newera serve` for no window). The **AI** menu, or the MCP chip in the status
bar, shows the snippet for your client, can register it for you, and lists the tools the
agent calls as it calls them.

**From a browser tab.** Open <https://3dneweraai.com/app/> and switch the MCP on in the
**AI** panel (Ctrl+Shift+M). The tab gets an address your AI can reach through a small
relay, because a tab cannot listen on a port. The relay passes messages and stores
nothing; the project stays in the tab, and the address stops answering when you switch it
off or close the tab. Treat that address like a password. Video rendering is not offered
there, since it would hold the tab for minutes.

Then just ask, for example: *"Draw a 4 × 5 m bedroom with a door and a window, furnish it
and render a photo."*

### Client setup

One click, with the app installed:

[![Install in Cursor](https://cursor.com/deeplink/mcp-install-dark.svg)](https://cursor.com/en/install-mcp?name=newera&config=eyJjb21tYW5kIjoibmV3ZXJhIiwiYXJncyI6WyJtY3AiXX0%3D)
[![Install in VS Code](https://img.shields.io/badge/VS_Code-Install_newera-0098FF?logo=visualstudiocode&logoColor=white)](https://insiders.vscode.dev/redirect/mcp/install?name=newera&config=%7B%22type%22%3A%22stdio%22%2C%22command%22%3A%22newera%22%2C%22args%22%3A%5B%22mcp%22%5D%7D)
[![Install in VS Code Insiders](https://img.shields.io/badge/VS_Code_Insiders-Install_newera-24bfa5?logo=visualstudiocode&logoColor=white)](https://insiders.vscode.dev/redirect/mcp/install?name=newera&config=%7B%22type%22%3A%22stdio%22%2C%22command%22%3A%22newera%22%2C%22args%22%3A%5B%22mcp%22%5D%7D&quality=insiders)

**Claude Desktop:** download `newera-mcp.mcpb` from the
[latest release](https://github.com/leandrodaf/3d-new-era-ai/releases/latest) and open it.

**Plugins with skills:**

| Client | Command |
|---|---|
| Claude Code | `/plugin marketplace add leandrodaf/3d-new-era-ai`, then `/plugin install 3d-new-era-ai-desktop@3d-new-era-ai` (the app on your screen) or `/plugin install 3d-new-era-ai@3d-new-era-ai` (hosted) |
| Codex | `codex plugin marketplace add leandrodaf/3d-new-era-ai` |
| Gemini CLI | `gemini extensions install https://github.com/leandrodaf/3d-new-era-ai` |

<details>
<summary>Manual setup for each client</summary>

**Claude Code** (already set up inside a clone of this repository by `.mcp.json`)

```sh
claude mcp add --transport http newera http://127.0.0.1:7878/mcp
```

**Codex CLI**

```sh
codex mcp add newera --url http://127.0.0.1:7878/mcp
```

**Gemini CLI**

```sh
gemini mcp add --transport http newera http://127.0.0.1:7878/mcp
```

**VS Code (Copilot agent mode)**

```sh
code --add-mcp '{"name":"newera","type":"http","url":"http://127.0.0.1:7878/mcp"}'
```

**Cursor** — `~/.cursor/mcp.json`

```json
{ "mcpServers": { "newera": { "url": "http://127.0.0.1:7878/mcp" } } }
```

**Windsurf** — `~/.codeium/windsurf/mcp_config.json`

```json
{ "mcpServers": { "newera": { "serverUrl": "http://127.0.0.1:7878/mcp" } } }
```

**Claude Desktop without the bundle** — *Settings → Developer → Edit Config* (needs
[Node.js](https://nodejs.org) for the `mcp-remote` bridge)

```json
{ "mcpServers": { "newera": { "command": "npx", "args": ["-y", "mcp-remote", "http://127.0.0.1:7878/mcp"] } } }
```

**DeepSeek, Qwen, Llama and other models** — the model does not matter, the app you chat
in does. Use one that speaks MCP: [Cline](https://cline.bot) or
[Roo Code](https://roocode.com) in VS Code, [Cherry Studio](https://cherry-ai.com),
[LM Studio](https://lmstudio.ai) or [opencode](https://opencode.ai) (`opencode.json`):

```json
{ "mcp": { "newera": { "type": "remote", "url": "http://127.0.0.1:7878/mcp" } } }
```

**Any other client** — Streamable HTTP at `http://127.0.0.1:7878/mcp`, or stdio for
clients that start a process:

```json
{ "mcpServers": { "newera": { "command": "newera", "args": ["mcp"] } } }
```

Over stdio, with the editor open the agent edits the plan in that window; with no window
(or `newera mcp --standalone`) it works on a project of its own.
</details>

## Showcase

A real 105 m² apartment, traced from an openly licensed floor plan and furnished, lit
and photographed by an AI agent through MCP alone: walls from the scan, doors that swing
the right way, a slatted TV wall, an L kitchen and wardrobes from `cabinet_run`,
downlights sized by photometry against NBR ISO/CIE 8995-1, and path-traced photos. Every
image below came out of the editor. **[See how it was built →](docs/SHOWCASE.md)**

| Floor plan with real finishes | Aerial cutaway |
|---|---|
| ![Rendered floor plan with real finishes](docs/images/showcase/13-rendered-plan.jpg) | ![Aerial cutaway of the apartment](docs/images/showcase/12-aerial.jpg) |

| Daylight | At dusk, lamps on |
|---|---|
| ![Living room with slatted TV wall](docs/images/showcase/01-living-window.jpg) | ![Dining table under the pendant at dusk](docs/images/showcase/09-dining-night.jpg) |
| ![Dining and living toward the window](docs/images/showcase/02-dining-living.jpg) | ![L-shaped kitchen at night](docs/images/showcase/10-kitchen-night.jpg) |
| ![L-shaped kitchen with terrazzo floor](docs/images/showcase/04-kitchen.jpg) | ![Master bedroom at night](docs/images/showcase/11-master-bedroom-night.jpg) |

| Joinery and rooms | |
|---|---|
| ![Slatted TV wall with the TV embedded](docs/images/showcase/03-tv-wall.jpg) | ![Home office with desk and bookcase](docs/images/showcase/05-office.jpg) |
| ![Master bedroom with tall wardrobe run](docs/images/showcase/07-master-bedroom.jpg) | ![Bathroom with Nero marble wall](docs/images/showcase/08-bathroom.jpg) |

## Features

- **Plans from anything.** Draw walls, arcs and sloping walls, or drop in a scanned plan
  at real scale and let `background` find the walls. Opens Sweet Home 3D (`.sh3d`)
  projects as they are.
- **Joinery a workshop can build.** Parametric cabinets, wardrobes, slatted panels,
  countertops with exact sink and cooktop cutouts, plaster coves with LED and modular
  sofas. `cabinet_run` fills a wall with even modules around doors, windows and corners;
  `cut_list` exports the boards as CSV or DXF/SVG sheets.
- **Checked against the people who live there.** An ergonomics review (circulation, beds
  and bathrooms per person, the kitchen triangle, wheelchair turning space) and lux per
  room by photometry, each finding tied to its [source](docs/STANDARDS.md) and with the
  fix ready to apply.
- **Electrical and plumbing plans** over the same floor plan: circuits and panel, Wi-Fi
  coverage, runs and checks against NBR 5410, NBR 5626 and NBR 8160.
- **Pictures that sell the project.** Rendered floor plans with real finishes, elevations
  and sections, aerial cutaways, path-traced photos with the sun from the compass and the
  lamps you placed, and videos along a camera path. They render on the CPU, so a headless
  server can take them too.
- **Roofs and structure.** Pitched roofs with skylights, walls and glass that follow the
  roof above, beams, storeys, pools and decks.
- **Furniture always true to size.** The catalog is generated by code, so a
  158 × 208 cm bed is exactly that in the plan, in 3D and in collision checks.
- **Try alternatives safely.** Duplicate the plan into a new tab, change it and compare
  versions side by side; name checkpoints and come back to them.
- **Built for agents.** People and agents go through the same undoable commands. Short
  ids (`w12`), `[x, y]` points in centimeters, compact reads and one-line write replies
  keep token use low.
- **Open and extensible.** Plugins in any language over the HTTP API, several people on
  the same project with named cursors, and exports to PDF, SVG, PNG, GLB and OBJ.

## MCP tools

Reads and changes are separate tools: a read never changes the plan, so an AI client can
run it without asking, and asks before each change.

| Tool | What it does |
|------|--------------|
| `home` | Compact state (`detail=summary` for counts, bounds and room areas) |
| `create` | Walls (polylines, arcs, sloping), rooms (polygon or detected from walls), dimensions, labels, roofs with skylights and solids, in one atomic call |
| `update` / `move` / `delete` | Edit any element by id |
| `arrange` | Copies in a row, rotate, mirror, group/ungroup, drawing order |
| `edit_walls` | Split a wall in two, or join walls on one line |
| `edit_home` | Project name, compass (north), the city whose building code applies and who lives there |
| `background` / `edit_background` | Scanned plan at real scale (calibration, X/Y scale, rotation); find the walls in it, and create them |
| `catalog` / `place` | Search the parametric catalog; place furniture, doors and windows (snapped into walls), beams, finishes and glass |
| `joinery` | Parametric cabinets, slatted panels, countertops with cutouts, plaster coves, shadow gaps and modular sofas |
| `cabinet_run` | Fill a wall with cabinets sized for it: even modules around corners, doors, windows, fridge and stove |
| `embed` | Set a sink or cooktop into a countertop (exact cutout) or an oven into a cabinet niche |
| `fit_roof` | Walls, glass and panels take the shape of the roof above and keep following it |
| `cut_list` | Boards, edge banding and hardware of the joinery, as CSV or DXF/SVG sheets |
| `layout` | Overlaps, pieces in walls, blocked doors, pieces outside rooms, areas against references |
| `ergonomics` | Review for the people living there: circulation, occupancy, kitchen, doors, ceiling heights, windows, wheelchair use |
| `lighting` / `edit_lighting` | Lux per room by photometry against NBR ISO/CIE 8995-1; place the fixtures a room needs |
| `electrical` / `plumbing` | Electrical and plumbing projects over the plan, changed through `edit_electrical` / `edit_plumbing` |
| `measure` | Free floor around a piece, the gap between two, what a straight probe runs into, how far a piece can grow |
| `model` | What an imported OBJ/glTF/GLB became: unit taken, size, triangles, materials, named parts, and what is not drawn; `check=clashes` finds parts passing through one another |
| `edit_model` | Swap the file of placed pieces in one undo step (keeping ids, size, finishes and links), change a material or a part by name, or give a lighter file for distant views |
| `library` / `edit_library` | A library of approved model versions, with their photos, measurements and source, placed in any project as `library:<name>@<v>` |
| `accept` | Mark findings of any review as seen, with the reason |
| `rules` | What a review checks against: the standards' tables, what each layout problem means, how ergonomics scores |
| `materials` / `disciplines` / `annotations` | Finishes, visible projects and layers, dimension chains and schedules; `stale` finds notes that no longer match the drawing |
| `levels` / `cameras` / `variants` / `video` | Storeys, points of view, plan versions and camera-path videos, each changed through its `edit_*` tool |
| `render_plan` / `show_plan` | PNG of the plan as the user sees it; the plan inside the chat as an interactive viewer (MCP Apps) |
| `render_3d` / `render_photo` | Software 3D views (aerial, visitor, cameras, elevations, sections); path-traced photos with sun and lamps |
| `finishes` / `edit_finishes` / `takeoff` | Which shop product covers which floor, ceiling or wall side, and the quantities an estimate needs (this fork) |
| `export` | PDF, SVG or PNG plan; GLB or OBJ model; the cut list as CSV or DXF/SVG sheets |
| `file` | New, open and save: projects (`.newera`) and Sweet Home 3D import (`.sh3d`) |
| `edit_history` / `history` | Undo and redo, shared with the user, and named checkpoints to come back to |
| `plugins` / `run_plugin` / `sessions` | External plugins and the people working on the project |
| `feedback` | A note to the developers about what a tool could do better (desktop only) |

The hosted connector adds `projects`, to list and switch between the projects in your
account, and, in this fork, `import_product`, `products` and `edit_products` for the shop
product library and `estimate` and `edit_estimate` for the priced estimate; it leaves out
`feedback`, the plugin tools and `library` / `edit_library` (the
library is a folder on your machine), which only make sense on your own machine; the
browser tab leaves out the library tools too.

## Command line

```sh
newera [FILE]           # desktop editor with the embedded HTTP + MCP server
newera --demo           # start with a sample house
newera gui --no-server  # editor only
newera serve [FILE]     # headless HTTP + MCP server
newera mcp              # MCP over stdio (attaches to the open window, if any)
```

Options: `--addr 127.0.0.1:7878` (or `NEWERA_ADDR`); `--token` (or `NEWERA_TOKEN`),
required to listen beyond loopback; `NEWERA_LOG` for the log filter;
`NEWERA_RENDER_THREADS` for photo renders (half the cores by default). The server binds to
loopback by default and validates the `Host` header. On Windows, `newera-gui.exe` opens
the editor without a console window.

## Build from source

You need Rust 1.95 or newer ([rustup](https://rustup.rs)). On Linux, also
`libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev`.

```sh
git clone https://github.com/leandrodaf/3d-new-era-ai
cd 3d-new-era-ai
make run          # editor + HTTP + MCP, with a sample house
make release      # optimized binary in target/release/newera
make web-serve    # browser editor and viewer at http://127.0.0.1:8790
```

Run `make` to list every command. To host the browser editor yourself, serve `web/` as
static files, with `.wasm` sent as `application/wasm` and compressed (about 15 MB raw,
a quarter of that gzipped). No backend or special headers are needed. The public site is
built by `scripts/site-build.sh` and deployed on every push to `main`.

**Browsers:** Chrome, Edge and Safari 26 use WebGPU. Where it is missing (Firefox today)
the editor falls back to WebGL 2 and still draws everything; a browser with neither gets a
message pointing at the download.

### Project layout

```text
crates/
  newera-core        domain model, commands, undo/redo, geometry, standards — no UI, no I/O
  newera-catalog     parametric furniture and fixtures at exact sizes; OBJ/glTF import
  newera-joinery     parametric joinery and interiors, workshop rules, cut lists
  newera-ergonomics  habitability review: clearances, occupancy, kitchens, accessibility
  newera-draw        plan scene shared by the editor, PNG renders and SVG/PDF export
  newera-render      3D meshes, software renderer, path-traced photos, videos
  newera-sh3d        Sweet Home 3D (.sh3d) import
  newera-mcp         the MCP tools, over stdio and Streamable HTTP
  newera-server      REST API + MCP endpoint, sessions (axum)
  newera-plugins     external programs that edit the home over the HTTP API
  newera-app         desktop editor (egui + wgpu)
  newera-editor-web  the editor in the browser (WebGPU or WebGL)
  newera-web         lightweight WebAssembly viewer
  newera-relay       lets an AI reach the editor in a browser tab
  newera-cloud       the hosted service: accounts, OAuth, cloud projects, billing
  newera-telemetry   crash reports and usage notes
  newera             the binary that wires everything together
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md): crates, the command model, MCP design and security
- [Standards and sources](docs/STANDARDS.md): where every rule in the reviews comes from
- [Showcase](docs/SHOWCASE.md): a full apartment built by an agent
- [Distribution](docs/DISTRIBUTION.md): channels, releases and the hosted service
- [Roadmap](docs/ROADMAP.md) and [changelog](CHANGELOG.md)

## Privacy and telemetry

The app sends crash reports through Sentry, along with the notes agents leave with the MCP
`feedback` tool. It is **on by default** and off with one click in
**Help → Send error reports**, or `newera telemetry off`; `NEWERA_TELEMETRY=0` turns it off
for a single run. Nothing of the project is sent, nor the IP address or the machine's name,
and the MCP token is removed from every report. Notes are always kept locally in the config
folder (`notes.jsonl`).

Under the same switch, opening the app counts one `app_open` — a random installation id, the
version, the operating system, the architecture and the mode — and the installers count one
install and one uninstall, saying which system and whether the app came from a release or was
compiled on the machine. Both go to `3dneweraai.com/ping`, an endpoint of the site that holds
the Google Analytics secret and forwards the count without passing the IP address on: the
country Cloudflare already knows goes instead. A build made without the project's keys reports
through the same endpoint, so installing from source counts like any other install;
`cargo build` on a clone and CI count nothing. The hosted service keeps only what it needs:
your email, your projects and usage, never the conversation. See the
[privacy policy](https://3dneweraai.com/privacy/).

## Support the project

Everything above is free and stays free. If it saves you time, the
[Supporter plan](https://3dneweraai.com/pricing/) (US$ 5/month or US$ 48/year) raises the
cloud quotas for your account and pays for the server.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md); `make check` runs the
same checks as CI.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
