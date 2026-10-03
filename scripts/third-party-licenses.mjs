// Writes THIRD-PARTY-LICENSES.txt, which ships next to TuxRead.exe: the license texts of every
// Rust crate in the Windows builds (cargo-about, with about.toml and about.hbs) and of every npm
// package in the window's bundle (package-lock.json entries that are neither dev nor optional).
// Usage: node scripts/third-party-licenses.mjs <output file>
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = join(import.meta.dirname, "..");
const out = process.argv[2];
if (!out) {
  console.error("usage: node scripts/third-party-licenses.mjs <output file>");
  process.exit(2);
}

// cargo-about refuses to write to a redirected stdout on Windows, so it writes a file.
const work = mkdtempSync(join(tmpdir(), "tuxread-licenses-"));
const rustFile = join(work, "rust.txt");
execFileSync("cargo", ["about", "generate", "--locked", "about.hbs", "-o", rustFile], { cwd: root, stdio: "inherit" });
const rust = readFileSync(rustFile, "utf8");
rmSync(work, { recursive: true, force: true });

const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
const npm = Object.entries(lock.packages)
  .filter(([path, entry]) => path.startsWith("node_modules/") && !entry.dev && !entry.optional)
  .map(([path, entry]) => {
    const dir = join(root, path);
    const files = readdirSync(dir).filter((f) => /^(licen[sc]e|copying)/i.test(f));
    if (files.length === 0) throw new Error(`${path} has no license file`);
    const texts = files.map((f) => readFileSync(join(dir, f), "utf8").trim()).join("\n\n");
    return `--------------------------------------------------------------------------------\n${path.slice("node_modules/".length)} ${entry.version} (npm), ${entry.license}\n\n${texts}\n`;
  });

const header = `TuxRead third-party licenses

TuxRead itself is free software under the MIT or the Apache-2.0 license (LICENSE-MIT,
LICENSE-APACHE). It is built with the components below; each license text follows the
components it covers. Their sources are on https://crates.io (Rust crates) and
https://www.npmjs.com (npm packages), at the versions given.
`;
writeFileSync(out, `${header}${rust}\n${npm.join("\n")}`.replace(/\r\n/g, "\n"));
console.log(`wrote ${out}: ${rust.split("\n---").length - 1} Rust license blocks, ${npm.length} npm packages`);
