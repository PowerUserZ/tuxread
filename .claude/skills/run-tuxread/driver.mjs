// Drives the TuxRead window for an agent: launches TuxRead.exe with a throwaway WebView2 profile
// and a DevTools port, runs the commands it reads from stdin (one per line), prints each result,
// and closes the app at the end of the input.
//
// Usage: node .claude/skills/run-tuxread/driver.mjs [TuxRead.exe] < commands
//   (default exe: target\release\TuxRead.exe, relative to the current folder)
//
// Commands (blank lines and lines starting with # are skipped):
//   lang en|tr          set the window's language and reload
//   open <image>        open a disk image, as "Open image…" would, and wait for its volume
//   click <css>         click the first element matching <css>
//   focus <css>         focus it
//   key <Key> [mods]    press a key (DOM key name: Enter, Tab, ArrowDown, a, …); mods 1 Alt 2 Ctrl 8 Shift
//   type <text>         type text into the focused field
//   wait <js>           wait up to 10 s until the expression is truthy
//   eval <js>           evaluate in the window and print the JSON result
//   text [css]          print an element's innerText (default: the whole window)
//   rows                print the file list's visible rows
//   ss <file.png>       save a screenshot of the window
//   sleep <ms>
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

const exe = resolve(process.argv[2] ?? "target/release/TuxRead.exe");
if (!existsSync(exe)) {
  console.error(`${exe} does not exist: build it first (npx tauri build --no-bundle)`);
  process.exit(2);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const q = (sel) => `document.querySelector(${JSON.stringify(sel)})`;

// WebView2 picks a free port and writes it to DevToolsActivePort in the profile.
const profile = mkdtempSync(join(tmpdir(), "tuxread-driver-"));
const app = spawn(exe, [], {
  env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=0", WEBVIEW2_USER_DATA_FOLDER: profile },
  stdio: "ignore",
});
let target;
for (let i = 0; i < 150 && !target; i++) {
  await sleep(200);
  try {
    const port = readFileSync(join(profile, "EBWebView", "DevToolsActivePort"), "utf8").split("\n")[0];
    const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    target = list.find((t) => t.type === "page");
  } catch {}
}
if (!target) {
  app.kill();
  console.error("no DevTools port: an elevated console makes WebView2 ignore WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS (see SKILL.md)");
  process.exit(1);
}

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((ok, fail) => ((ws.onopen = ok), (ws.onerror = () => fail(new Error("could not connect to the window")))));
let next = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  pending.get(msg.id)?.(msg);
  pending.delete(msg.id);
};
const send = (method, params = {}) =>
  new Promise((ok, fail) => {
    const id = ++next;
    pending.set(id, (msg) => (msg.error ? fail(new Error(msg.error.message)) : ok(msg.result)));
    ws.send(JSON.stringify({ id, method, params }));
  });
async function evaluate(expression) {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
}
async function waitFor(expression, ms = 10000) {
  for (const end = Date.now() + ms; Date.now() < end; await sleep(100)) {
    if (await evaluate(expression).catch(() => false)) return;
  }
  throw new Error(`timed out waiting for: ${expression}`);
}
async function reload() {
  await evaluate("location.reload()").catch(() => {});
  await sleep(300);
  await waitFor(`document.readyState === "complete" && ${q("#root")}?.childElementCount > 0`);
}
const codes = { Backspace: 8, Tab: 9, Enter: 13, Escape: 27, End: 35, Home: 36, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Delete: 46, F5: 116 };

const commands = {
  async lang(code) {
    await evaluate(`localStorage.setItem("tuxread.language", ${JSON.stringify(code)})`);
    await reload();
  },
  async open(image) {
    await evaluate(`window.__TAURI_INTERNALS__.invoke("open_image", { path: ${JSON.stringify(resolve(image))} })`);
    await reload();
    await waitFor(`${q(".item.volume")} !== null`);
  },
  click: (css) => evaluate(`${q(css)}.click()`),
  focus: (css) => evaluate(`${q(css)}.focus()`),
  async key(key, mods = "0") {
    const windowsVirtualKeyCode = codes[key] ?? key.toUpperCase().charCodeAt(0);
    // Enter also sends its character, as a real key press does: forms submit on it.
    const down = key === "Enter" ? { type: "keyDown", text: "\r" } : { type: "rawKeyDown" };
    for (const e of [down, { type: "keyUp" }]) {
      await send("Input.dispatchKeyEvent", { ...e, key, modifiers: Number(mods), windowsVirtualKeyCode });
    }
    await sleep(50);
  },
  async type(text) {
    await send("Input.insertText", { text });
  },
  wait: (js) => waitFor(js),
  eval: async (js) => JSON.stringify(await evaluate(js)),
  text: (css) => evaluate(css ? `${q(css)}.innerText` : "document.body.innerText"),
  rows: async () =>
    (await evaluate(`[...document.querySelectorAll(".body .row:not(.head)")].map((r) => r.innerText.replace(/\\s+/g, " ").trim())`)).join("\n"),
  async ss(file) {
    const { data } = await send("Page.captureScreenshot", { format: "png" });
    mkdirSync(dirname(resolve(file)), { recursive: true });
    writeFileSync(resolve(file), Buffer.from(data, "base64"));
    return `saved ${resolve(file)}`;
  },
  sleep: (ms) => sleep(Number(ms)),
};

let code = 0;
let input = "";
for await (const chunk of process.stdin) input += chunk;
// The DevTools target exists before the app's page loads; wait until the window has rendered.
await waitFor(`${q("#root")}?.childElementCount > 0`, 30000);
for (const line of input.split(/\r?\n/).map((l) => l.trim())) {
  if (!line || line.startsWith("#")) continue;
  const [name, ...rest] = line.split(" ");
  const arg = rest.join(" ");
  const cmd = commands[name];
  console.log(`> ${line}`);
  try {
    if (!cmd) throw new Error(`unknown command ${name}`);
    const out = name === "key" ? await cmd(...rest) : await cmd(arg);
    if (out !== undefined && out !== "") console.log(out);
  } catch (e) {
    console.log(`ERROR: ${e.message}`);
    code = 1;
    break;
  }
}
ws.close();
app.kill();
for (let i = 0; i < 20; i++) {
  try {
    rmSync(profile, { recursive: true, force: true });
    break;
  } catch {
    await sleep(250);
  }
}
process.exit(code);
