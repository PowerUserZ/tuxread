// Spec §6.2 and §7.5: "Copy to…" with the report, a copy that meets conflicts, and Cancel.
// TUXREAD_SMOKE_BIG (see browse.mjs) adds a copy of 10,000 files that is cancelled.
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { inEnglish, openImage } from "./browse.mjs";

const tiny = join(import.meta.dirname, "..", "..", "crates", "core", "tests", "data", "tiny-ext4.img");
const q = (sel) => `document.querySelector(${JSON.stringify(sel)})`;
const rows = `[...document.querySelectorAll(".body .row:not(.head)")]`;
const row = (name) => `${rows}.find((r) => r.querySelector(".name").textContent === ${JSON.stringify(name)})`;
const lastJob = `[...document.querySelectorAll(".job")].at(-1)`;

/** Selects everything, copies it to `dest` with the `conflict` choice (pressing Enter), and returns
 *  the job's summary. */
async function copyAll(page, dest, conflict = null) {
  await page.eval(`${q(".filelist")}.focus()`);
  await page.key("a", 2);
  await page.key("c", 2);
  await page.waitFor(`${q("dialog[open] input[name=dest]")} !== null`);
  await page.eval(`${q("dialog[open] input[name=dest]")}.focus()`);
  await page.type(dest);
  if (conflict) await page.eval(`${q(`dialog[open] input[value=${conflict}]`)}.click()`);
  // Enter in the folder field copies, as in any Windows dialog.
  await page.eval(`${q("dialog[open] input[name=dest]")}.focus()`);
  await page.key("Enter");
  await page.waitFor(`${lastJob} !== undefined && ${lastJob}.querySelector(".bar") === null`, 60000);
  return page.eval(`${lastJob}.querySelector(".job-line").textContent`);
}

export default async function (page) {
  const dest = mkdtempSync(join(tmpdir(), "tuxread-smoke-copy-"));
  try {
    await inEnglish(page);
    await openImage(page, tiny);
    await page.eval(`${q(".item.volume")}.click()`);
    await page.waitFor(`${rows}.length === 4`);

    assert.equal(await copyAll(page, dest), "4 copied, 0 renamed, 1 skipped, 0 failed");
    assert.deepEqual(readdirSync(dest).sort(), ["a.txt", "dir", "lost+found"]);
    assert.equal(readFileSync(join(dest, "a.txt"), "utf8"), "alpha\n");
    assert.ok(existsSync(join(dest, "dir", "b.txt")));

    // The report lists what needs a look first: the symlink, which Windows gets no copy of.
    await page.eval(`[...${lastJob}.querySelectorAll("button")].find((b) => b.textContent === "Show report").click()`);
    await page.waitFor(`document.querySelectorAll("dialog[open] .report li").length === 5`);
    assert.match(await page.eval(`${q("dialog[open] .report li")}.innerText`), /^Skipped\s+\/link\s+symbolic link -> a\.txt/);
    await page.key("Escape");
    await page.waitFor(`${q("dialog[open]")} === null`);

    // Again into the same folder: "keep both" (the default) renames the three top-level items
    // (a renamed folder's contents count as copied); "skip" skips them.
    assert.equal(await copyAll(page, dest), "1 copied, 3 renamed, 1 skipped, 0 failed");
    assert.ok(existsSync(join(dest, "a (2).txt")));
    assert.ok(existsSync(join(dest, "dir (2)", "b.txt")));
    assert.equal(await copyAll(page, dest, "skip"), "0 copied, 0 renamed, 4 skipped, 0 failed");

    // Into a folder that does not exist: the copy does not start.
    assert.match(await copyAll(page, join(dest, "missing")), /^The copy couldn't start\. That is not a folder\./);
    assert.ok(!existsSync(join(dest, "missing")));

    const big = process.env.TUXREAD_SMOKE_BIG;
    if (!big) return "TUXREAD_SMOKE_BIG not set: skipped Cancel";
    await openImage(page, big);
    await page.eval(`[...document.querySelectorAll(".item.volume")].at(-1).click()`);
    await page.waitFor(`${row("many")} !== undefined`);
    await page.eval(`${row("many")}.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }))`);
    await page.eval(`${q(".filelist")}.focus()`);
    await page.key("c", 2);
    await page.waitFor(`${q("dialog[open] input[name=dest]")} !== null`);
    await page.eval(`${q("dialog[open] input[name=dest]")}.focus()`);
    mkdirSync(join(dest, "big"));
    await page.type(join(dest, "big"));
    await page.eval(`${q("dialog[open] .dialog-actions .primary")}.click()`);
    await page.waitFor(`${lastJob}?.querySelector("button:not([disabled])")?.textContent === "Cancel"`);
    await page.eval(`${lastJob}.querySelector("button").click()`);
    await page.waitFor(`${lastJob}.querySelector(".bar") === null`, 30000);
    assert.match(await page.eval(`${lastJob}.querySelector(".job-line").textContent`), /^Cancelled\. /);
  } finally {
    rmSync(dest, { recursive: true, force: true });
  }
}
