// Spec §5.8: only TuxRead's own commands run, there is no global Tauri object, and the
// content security policy blocks network requests and inline scripts.
import assert from "node:assert/strict";

const invoke = (cmd) =>
  `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}).then(() => "allowed", (e) => "refused: " + e)`;

export default async function (page) {
  await page.waitFor(`document.getElementById("root")?.childElementCount > 0`);
  assert.equal(await page.eval("typeof window.__TAURI__"), "undefined", "withGlobalTauri is off");
  for (const cmd of ["plugin:app|version", "plugin:window|close", "plugin:opener|open_url"]) {
    assert.match(await page.eval(invoke(cmd)), /^refused/, `${cmd} is not granted`);
  }
  assert.match(
    await page.eval(`fetch("https://example.com").then(() => "reached", (e) => "blocked: " + e.message)`),
    /^blocked/,
    "the CSP blocks network requests",
  );
  assert.equal(
    await page.eval(`(() => {
      const s = document.createElement("script");
      s.textContent = "window.inlineRan = true";
      document.body.append(s);
      return window.inlineRan === true;
    })()`),
    false,
    "the CSP blocks inline scripts",
  );
  assert.equal(await page.eval("window.isSecureContext"), true, "the clipboard needs a secure context");

}
