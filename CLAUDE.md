# CLAUDE.md

Fork of `leandrodaf/3d-new-era-ai` (remote `upstream`), extended with shop-product import and
estimates. Upstream's own rules are in `CONTRIBUTING.md` and `docs/`.

- The plan in `docs/SHOP-IMPORT-PLAN.md` is approved and carried out step after step without
  waiting for the operator: each reviewed step is committed and pushed to `main`, and the
  pipeline deploys it to interioragent.dutch-atlas.com; questions go out as chat text while work
  that does not depend on them continues.
- Our working documents (plans, notes) live in `docs/` but never enter git: each such path is
  listed in `.git/info/exclude`, never in `.gitignore`.
