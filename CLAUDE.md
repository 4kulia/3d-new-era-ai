# CLAUDE.md

Fork of `leandrodaf/3d-new-era-ai` (remote `upstream`), extended with shop-product import and
estimates. Upstream's own rules are in `CONTRIBUTING.md` and `docs/`.

- The plan in `docs/SHOP-IMPORT-PLAN.md` is approved and carried out step after step without
  waiting for the operator: each reviewed step, with README.md's section on this fork brought up
  to date, is pushed to `main` and deployed to interioragent.dutch-atlas.com; questions go out as
  chat text while independent work continues.
- interioragent.dutch-atlas.com is the operator's working instance, not production, though the
  deploy workflow calls it `production`: for it alone the canon's «Production» approval is set
  aside, and changes for the work in hand (container files, /opt/interioragent config) need no
  asking. Other services on the host keep the canon's rule.
- When `Test (macos-latest)` is CI's only non-success, cancelled with "not acquired by Runner",
  re-run it once; cancelled again, dispatch `deploy-interioragent.yml` on `main`. A failed job is
  never bypassed this way.
- One code-writing agent at a time in this working tree: the MCP tool fixtures
  (`crates/newera-mcp/tests/fixtures/tool-surface.json`, `tool-budget.json`, `tool-search.json`),
  newera-cloud `mcp.rs`/`lib.rs` and flow tests are shared by almost every unit. A second unit
  starts when the first is committed.
- The browser e2e scripts (`scripts/web-mcp-e2e.mjs`, `scripts/web-editor-e2e.mjs`) run only in
  CI. A change to code the wasm tab runs (newera-app and the crates it links, newera-mcp and
  newera-render included; newera-editor-web; web/) is checked against what those scripts assert:
  each scripted call through it is traced with the script's arguments (on wasm32 std's clock,
  thread, temp_dir or process::id panics) and the changed messages, sizes and counts are
  searched; the scripts are updated in the same change; the builder and the reviewer both do this.
- The operator's Mac has ~8 GB free. Local cargo runs set `CARGO_PROFILE_DEV_DEBUG=0`, build only
  the crates under work (never `--workspace`), one build at a time across agents (wait on
  `pgrep -x cargo` and `pgrep -x rustc`, never `pgrep -f`: it matches any command line naming
  cargo), each in a `CARGO_TARGET_DIR` named for its agent and removed by that agent alone when its
  step ends; when another's leaves no room, wait for its step to end.
- Our working documents (plans, notes) live in `docs/` but never enter git: each such path is
  listed in `.git/info/exclude`, never in `.gitignore`.
