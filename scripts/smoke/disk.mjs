// Spec §5.1, §5.7 and §6.3 on a real disk: unlock it through the elevated helper, browse and
// copy, then lose the helper while browsing and while copying. Needs TUXREAD_SMOKE_DISK, the
// number of an attached disk with ext4 volumes (scripts/attach-vhd.ps1 attaches the corpus
// mbr-logical image), and an account that can approve UAC prompts: the helper starts elevated,
// and only an elevated taskkill can stop it.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { inEnglish } from "./browse.mjs";

const q = (sel) => `document.querySelector(${JSON.stringify(sel)})`;
const rows = `[...document.querySelectorAll(".body .row:not(.head)")]`;
const row = (name) => `${rows}.find((r) => r.querySelector(".name").textContent === ${JSON.stringify(name)})`;
const lastJob = `[...document.querySelectorAll(".job")].at(-1)`;
const jobLine = `${lastJob}.querySelector(".job-line").textContent`;

/** The helper's process id: the TuxRead.exe that is not the app. */
function helperPid(page) {
  const out = execFileSync("tasklist", ["/FI", "IMAGENAME eq TuxRead.exe", "/FO", "CSV", "/NH"], { encoding: "utf8" });
  const pids = [...out.matchAll(/^"TuxRead\.exe","(\d+)"/gm)].map((m) => Number(m[1])).filter((p) => p !== page.pid);
  assert.equal(pids.length, 1, "one helper runs");
  return pids[0];
}

function stopHelper(pid) {
  execFileSync("powershell", [
    "-NoProfile",
    "-Command",
    `Start-Process taskkill -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '/F','/PID','${pid}'`,
  ]);
}

async function copyAll(page, dest) {
  await page.eval(`${q(".filelist")}.focus()`);
  await page.key("a", 2);
  await page.key("c", 2);
  await page.waitFor(`${q("dialog[open] input[name=dest]")} !== null`);
  await page.eval(`${q("dialog[open] input[name=dest]")}.focus()`);
  await page.type(dest);
  await page.eval(`${q("dialog[open] .dialog-actions .primary")}.click()`);
}

export default async function (page) {
  if (!process.env.TUXREAD_SMOKE_DISK) return "TUXREAD_SMOKE_DISK not set: skipped";
  const number = Number(process.env.TUXREAD_SMOKE_DISK);
  const dest = mkdtempSync(join(tmpdir(), "tuxread-smoke-disk-"));
  try {
    await inEnglish(page);
    // The sidebar lists disks in the backend's order.
    const { disks } = await page.eval(`window.__TAURI_INTERNALS__.invoke("list_sources")`);
    const index = disks.findIndex((d) => d.key === number);
    assert.ok(index >= 0, `disk ${number} is listed`);
    const disk = `document.querySelectorAll(".source")[${index}]`;
    const head = `${disk}.querySelector(".source-head")`;
    const volume = (n) => `${disk}.querySelectorAll(".item.volume")[${n}]`;

    assert.equal(await page.eval(`${head}.classList.contains("locked")`), true, "disks start locked");
    await page.eval(`${head}.click()`);
    await page.waitFor(`${disk}.querySelectorAll(".item.volume").length > 0`, 60000);
    const volumes = await page.eval(`${disk}.querySelectorAll(".item.volume").length`);
    assert.ok((await page.eval(`${disk}.querySelectorAll(".diskbar .seg.linux").length`)) > 0);
    await page.eval(`${volume(0)}.click()`);
    await page.waitFor(`${rows}.length > 0`, 30000);
    await copyAll(page, dest);
    await page.waitFor(`${lastJob}.querySelector(".bar") === null`, 600000);
    assert.match(await page.eval(jobLine), / 0 failed$/);

    // The helper goes away. Reads the cache can answer still work; the first one that needs
    // the disk says the helper stopped, the view resets and the disk locks again.
    stopHelper(helperPid(page));
    await page.eval(`${volume(volumes - 1)}.click()`);
    await page.waitFor(`${rows}.length > 0 || ${q(".banner.error")} !== null`, 15000);
    if (await page.eval(`${q(".banner.error")} === null`)) {
      await page.eval(`${row("many")}.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }))`);
    }
    await page.waitFor(`${q(".banner.error")} !== null`, 15000);
    assert.match(await page.eval(`${q(".banner.error")}.innerText`), /^The disk helper stopped/);
    await page.waitFor(`${head}.classList.contains("locked")`);
    assert.equal(await page.eval(`${q(".empty")}?.textContent`), "Choose a volume on the left to see its files.");

    // Selecting the disk again starts a new helper.
    await page.eval(`${head}.click()`);
    await page.waitFor(`${disk}.querySelectorAll(".item.volume").length === ${volumes}`, 60000);
    const pid = helperPid(page);
    await page.eval(`${volume(volumes - 1)}.click()`);
    await page.waitFor(`${rows}.length > 0 && ${q(".banner.error")} === null`, 30000);

    // The helper goes away before a copy of data nothing has read yet (stopping it takes
    // seconds, longer than the copy may run): those files fail, the copy goes on to the end,
    // and the disk locks again.
    stopHelper(pid);
    mkdirSync(join(dest, "second"));
    await copyAll(page, join(dest, "second"));
    await page.waitFor(`${lastJob}.querySelector(".bar") === null`, 120000);
    const summary = await page.eval(jobLine);
    assert.match(summary, / [1-9][\d,]* failed$/);
    await page.waitFor(`${head}.classList.contains("locked")`);
    return `the copy without its helper: ${summary}`;
  } finally {
    rmSync(dest, { recursive: true, force: true });
  }
}
