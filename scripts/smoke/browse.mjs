// Spec §5.9 and §7.5: open an image, browse it with the mouse and the keyboard, read
// properties, switch language, and copy diagnostics. TUXREAD_SMOKE_BIG may name a bare ext4
// image with a 10,000-entry folder `many` (the corpus ext4.img): then the list must stay
// virtualized.
import assert from "node:assert/strict";
import { join } from "node:path";

const tiny = join(import.meta.dirname, "..", "..", "crates", "core", "tests", "data", "tiny-ext4.img");
const q = (sel) => `document.querySelector(${JSON.stringify(sel)})`;
const rows = `[...document.querySelectorAll(".body .row:not(.head)")]`;
const names = `${rows}.map((r) => r.querySelector(".name").textContent)`;
const path = `[...document.querySelectorAll(".pathbar button")].map((b) => b.textContent).join(" / ")`;
const row = (name) => `${rows}.find((r) => r.querySelector(".name").textContent === ${JSON.stringify(name)})`;

/** Opens `image` as the "Open image…" dialog would, and returns once its volume is listed. */
export async function openImage(page, image) {
  await page.eval(`window.__TAURI_INTERNALS__.invoke("open_image", { path: ${JSON.stringify(image)} })`);
  await page.reload();
  await page.waitFor(`${q(".item.volume")} !== null`);
}

/** English strings, so the checks below can read them. */
export async function inEnglish(page) {
  await page.waitFor(`${q(".source-head, .open-image")} !== null`);
  await page.eval(`localStorage.setItem("tuxread.language", "en")`);
  await page.reload();
}

export default async function (page) {
  await inEnglish(page);
  await openImage(page, tiny);
  assert.equal(await page.eval(`${q(".item.volume .item-name")}.textContent`), "ext4");

  // The keyboard reaches the tree, and Enter opens the volume.
  await page.eval(`${q(".item.volume")}.focus()`);
  await page.key("Enter");
  await page.waitFor(`${rows}.length === 4`);
  assert.deepEqual(await page.eval(names), ["dir", "lost+found", "a.txt", "link"], "folders first, then by name");
  // The chosen volume is now the tree's one Tab stop, so Tab comes back to it.
  assert.equal(await page.eval(`${q(".item.volume")}.tabIndex`), 0, "the chosen volume is the Tab stop");
  assert.equal(await page.eval(`${q(".source-head")}.tabIndex`), -1);
  assert.equal(await page.eval(path), "ext4");
  assert.match(await page.eval(`${q(".statusbar")}.textContent`), /^4 items.*ext4, .* used of 512 KB$/);

  // Into a folder and out again: double-click, Backspace, Alt+Left, the path bar.
  await page.eval(`${row("dir")}.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }))`);
  await page.waitFor(`${path} === "ext4 / dir"`);
  assert.deepEqual(await page.eval(names), ["b.txt"]);
  await page.eval(`${q(".filelist")}.focus()`);
  await page.key("Backspace");
  await page.waitFor(`${path} === "ext4"`);
  await page.key("ArrowLeft", 1);
  await page.waitFor(`${path} === "ext4 / dir"`);
  await page.eval(`${q(".pathbar button")}.click()`);
  await page.waitFor(`${path} === "ext4" && ${rows}.length === 4`);

  // The column headers sort from the keyboard: Tab from the list reaches "Name".
  await page.eval(`${q(".filelist")}.focus()`);
  await page.key("Tab");
  assert.equal(await page.eval("document.activeElement.textContent"), "Name ▴");
  await page.key("Enter");
  await page.waitFor(`${names}.join() === "lost+found,dir,link,a.txt"`);
  await page.key("Enter");
  await page.waitFor(`${names}.join() === "dir,lost+found,a.txt,link"`);

  // Arrow keys move the focus; Enter on a folder enters it.
  await page.eval(`${q(".filelist")}.focus()`);
  await page.key("ArrowDown");
  assert.equal(await page.eval(`${q(".row.focused .name")}.textContent`), "dir");
  await page.key("Enter");
  await page.waitFor(`${path} === "ext4 / dir"`);
  await page.key("Backspace");
  await page.waitFor(`${rows}.length === 4`);

  // Properties of the symlink, from the toolbar.
  await page.eval(`${row("link")}.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }))`);
  await page.eval(`[...document.querySelectorAll(".actions button")].find((b) => b.textContent === "Properties").click()`);
  await page.waitFor(`${q("dialog[open] .props")} !== null`);
  const props = await page.eval(`${q("dialog[open] .props")}.innerText`);
  assert.match(props, /Symbolic link/);
  assert.match(props, /Points to\s+a\.txt/);
  await page.key("Escape");
  await page.waitFor(`${q("dialog[open]")} === null`);

  // Errors come back as codes with an English detail.
  const missing = await page.eval(
    `window.__TAURI_INTERNALS__.invoke("open_image", { path: "C:\\\\no\\\\such.img" }).then(() => null, (e) => e)`,
  );
  assert.equal(missing.code, "notFound");
  const unknown = await page.eval(
    `window.__TAURI_INTERNALS__.invoke("stat", { volume: 999, entry: 0 }).then(() => null, (e) => e)`,
  );
  assert.deepEqual(unknown, { code: "notFound", message: "no volume #999" });

  // Diagnostics: copied with a notice, and free of paths and names.
  const diagnostics = await page.eval(`window.__TAURI_INTERNALS__.invoke("diagnostics")`);
  assert.match(diagnostics, /opened image #\d+\s+512\.0 KiB/);
  assert.doesNotMatch(diagnostics, /tiny-ext4|a\.txt|\\\\/);
  await page.eval(`[...document.querySelectorAll(".sidebar-foot button")].find((b) => b.textContent === "Copy diagnostics").click()`);
  await page.waitFor(`${q(".banner[role=status]")}?.textContent.includes("Diagnostics copied")`);

  // Language: Turkish from About, kept after a reload.
  await page.eval(`[...document.querySelectorAll(".sidebar-foot button")].find((b) => b.textContent === "About").click()`);
  await page.waitFor(`${q("dialog[open] select")} !== null`);
  await page.eval(`(() => {
    const select = ${q("dialog[open] select")};
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(select, "tr");
    select.dispatchEvent(new Event("change", { bubbles: true }));
  })()`);
  await page.waitFor(`${q("dialog[open] h2")}.textContent === "TuxRead hakkında"`);
  await page.reload();
  await page.waitFor(`${q(".sidebar .heading")}?.textContent === "Diskler"`);

  const big = process.env.TUXREAD_SMOKE_BIG;
  if (!big) return "TUXREAD_SMOKE_BIG not set: skipped the 10,000-entry folder";
  await openImage(page, big);
  await page.eval(`[...document.querySelectorAll(".item.volume")].at(-1).click()`);
  await page.waitFor(`${row("many")} !== undefined`);
  const start = Date.now();
  await page.eval(`${row("many")}.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }))`);
  await page.waitFor(`/^10[.,]000 öğe/.test(${q(".statusbar")}.textContent)`, 30000);
  const ms = Date.now() - start;
  assert.ok((await page.eval(`${rows}.length`)) < 100, "only the rows in view are in the page");
  await page.eval(`${q(".filelist")}.focus()`);
  await page.key("End");
  await page.waitFor(`${q(".row.focused .name")}?.textContent === "f09999"`);
  assert.equal(
    await page.eval(`(() => {
      const list = ${q(".body")}.getBoundingClientRect();
      const focused = ${q(".row.focused")}.getBoundingClientRect();
      return focused.top >= list.top && focused.bottom <= list.bottom + 1;
    })()`),
    true,
    "End scrolls the last row into view",
  );
  return `10,000 entries listed in ${ms} ms`;
}
