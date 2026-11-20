// Runs a wasm32-unknown-unknown binary (`cargo build --target wasm32-unknown-unknown --bin …`)
// under node, with no imports: `main` returns when every check held; a panic or an abort traps.
const fs = require("fs");
const path = process.argv[2];
if (!path) { console.error("usage: node run_wasm_test.js <binary>.wasm"); process.exit(2); }
WebAssembly.instantiate(fs.readFileSync(path), {})
  .then((m) => {
    const main = m.instance.exports.main;
    if (typeof main !== "function") { console.error("no `main` export"); process.exit(2); }
    const code = main(0, 0);
    console.log(`${path}: ok (main returned ${code})`);
    process.exit(code === 0 ? 0 : 1);
  })
  .catch((e) => { console.error(`${path}: ${e}`); process.exit(1); });
