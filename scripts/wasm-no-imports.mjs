// Fails when a wasm module imports anything: web/viewer.js instantiates
// newera_web.wasm with an empty import object, so one import (a wasm-bindgen
// placeholder pulled in by a clock crate, say) is a viewer that never loads.
// Usage: node scripts/wasm-no-imports.mjs <module.wasm>
import { readFileSync } from "node:fs";

const file = process.argv[2];
const imports = WebAssembly.Module.imports(new WebAssembly.Module(readFileSync(file)));
if (imports.length > 0) {
  console.error(`${file} imports what its loader does not supply:`);
  for (const { module, name } of imports) console.error(`  ${module}.${name}`);
  process.exit(1);
}
console.log(`${file}: no imports`);
