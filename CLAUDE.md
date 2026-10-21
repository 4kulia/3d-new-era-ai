# CLAUDE.md

Fork of `leandrodaf/3d-new-era-ai` (remote `upstream`), extended with shop-product import and
estimates. Upstream's own rules are in `CONTRIBUTING.md` and `docs/`.

- The plan in `docs/SHOP-IMPORT-PLAN.md` is approved and carried out step after step without
  waiting for the operator: each reviewed step is committed and pushed to `main`, and the
  pipeline deploys it to interioragent.dutch-atlas.com; questions go out as chat text while work
  that does not depend on them continues.
- interioragent.dutch-atlas.com is the operator's own working instance, not a production service,
  although the deploy workflow names its environment `production`: for this service only, the
  canon's «Production» approval rule is set aside, and direct changes on it for the work in hand
  (files into the service container, config in /opt/interioragent) are made without asking.
  Every other service on the same host keeps the canon's rule.
- GitHub often has no macOS runner for this fork: the CI job `Test (macos-latest)` is cancelled
  with "not acquired by Runner", CI ends as failure and the deploy is skipped. When that cancelled
  job is CI's only non-success, re-run it once; if it is cancelled again, dispatch
  `deploy-interioragent.yml` on `main` by hand. A job that failed is never bypassed this way.
- One code-writing agent at a time in this working tree: the MCP tool fixtures
  (`crates/newera-mcp/tests/fixtures/tool-surface.json`, `tool-budget.json`, `tool-search.json`),
  newera-cloud `mcp.rs`/`lib.rs` and flow tests are shared by
  almost every unit, and two writers regenerate fixtures over each other's work. A second unit
  starts when the first is committed.
- The browser e2e scripts (`scripts/web-mcp-e2e.mjs`, `scripts/web-editor-e2e.mjs`) run only in
  CI. A change to editor or tab behaviour (crates/newera-app, newera-editor-web, web/) is checked
  against what those scripts assert — searched for the changed messages, sizes and counts — and the
  scripts are updated in the same change; the builder and the reviewer both do this.
- The operator's Mac has only ~8 GB free, and a full workspace build fills it. Local cargo runs
  set `CARGO_PROFILE_DEV_DEBUG=0`, build only the crates under work (never `--workspace`; CI checks
  the whole workspace), run one build at a time across agents, and `target/` is removed when a step
  ends.
- Our working documents (plans, notes) live in `docs/` but never enter git: each such path is
  listed in `.git/info/exclude`, never in `.gitignore`.
