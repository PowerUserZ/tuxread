// UI smoke test: drives a built TuxRead.exe through WebView2's DevTools port.
// Usage: node scripts/smoke/run.mjs <path to TuxRead.exe> [scenario ...]
// The scenarios are the other .mjs files in this folder; with no names given, all of them run.
// Each scenario gets a fresh app with a throwaway WebView2 profile, so it starts in a known state
// and leaves the user's own TuxRead settings alone.
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { pathToFileURL } from "node:url";

const here = import.meta.dirname;
const [exe, ...wanted] = process.argv.slice(2);
const names = readdirSync(here)
  .filter((f) => f.endsWith(".mjs") && f !== "run.mjs")
  .map((f) => basename(f, ".mjs"));
if (!exe || wanted.some((w) => !names.includes(w))) {
  console.error(`usage: node scripts/smoke/run.mjs <TuxRead.exe> [${names.join(" | ")} ...]`);
  process.exit(2);
}
if (!existsSync(exe)) {
  console.error(`${exe} does not exist: build the app first (npx tauri build --no-bundle)`);
  process.exit(2);
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// WebView2 picks a free port (a fixed one can sit in a range Windows reserves) and writes it to
// DevToolsActivePort in the profile.
const devtools = (profile) => {
  let port;
  try {
    port = readFileSync(join(profile, "EBWebView", "DevToolsActivePort"), "utf8").split("\n")[0];
  } catch {
    return undefined;
  }
  return fetch(`http://127.0.0.1:${port}/json`)
    .then((r) => r.json())
    .then((list) => list.find((t) => t.type === "page"))
    .catch(() => undefined);
};

async function launch() {
  const profile = mkdtempSync(join(tmpdir(), "tuxread-smoke-"));
  const app = spawn(exe, [], {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=0",
      WEBVIEW2_USER_DATA_FOLDER: profile,
    },
    stdio: "ignore",
  });
  let target;
  for (let i = 0; i < 150 && !target; i++) {
    await sleep(200);
    target = await devtools(profile);
  }
  if (!target) {
    app.kill();
    throw new Error("the window's DevTools port did not open");
  }
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error("could not connect to the window"));
  });
  let next = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    pending.get(msg.id)?.(msg);
    pending.delete(msg.id);
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++next;
      pending.set(id, (msg) => (msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result)));
      ws.send(JSON.stringify({ id, method, params }));
    });

  const page = {
    sleep,
    /** The app's process id. */
    pid: app.pid,
    /** Evaluates `expression` in the window (awaiting promises) and returns its JSON value. */
    async eval(expression) {
      const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      const thrown = r.exceptionDetails;
      if (thrown) throw new Error(thrown.exception?.description ?? String(thrown.exception?.value ?? thrown.text));
      return r.result.value;
    },
    async waitFor(expression, ms = 10000) {
      const end = Date.now() + ms;
      while (Date.now() < end) {
        if (await page.eval(expression).catch(() => false)) return;
        await sleep(100);
      }
      throw new Error(`timed out waiting for: ${expression}`);
    },
    /** Reloads the window, e.g. after changing the backend's state behind the UI's back. */
    async reload() {
      await page.eval("location.reload()").catch(() => {});
      await sleep(300);
      await page.waitFor(`document.readyState === "complete"`);
    },
    /** Presses `key` (a DOM key name) in the focused element. Modifiers: 1 Alt, 2 Ctrl, 8 Shift. */
    async key(key, modifiers = 0) {
      const codes = { Backspace: 8, Tab: 9, Enter: 13, Escape: 27, End: 35, Home: 36, ArrowLeft: 37, ArrowUp: 38, ArrowDown: 40, F5: 116 };
      const windowsVirtualKeyCode = codes[key] ?? key.toUpperCase().charCodeAt(0);
      // Enter also sends its character, as a real key press does: forms submit on it.
      const down = key === "Enter" ? { type: "keyDown", text: "\r" } : { type: "rawKeyDown" };
      for (const event of [down, { type: "keyUp" }]) {
        await send("Input.dispatchKeyEvent", { ...event, key, modifiers, windowsVirtualKeyCode });
      }
      await sleep(50);
    },
    /** Types `text` into the focused field. */
    async type(text) {
      await send("Input.insertText", { text });
    },
    async close() {
      ws.close();
      app.kill();
      // Wait for WebView2 to let go of the port and the profile before the next scenario.
      for (let i = 0; i < 50 && (await devtools()); i++) await sleep(100);
      for (let i = 0; i < 20; i++) {
        try {
          rmSync(profile, { recursive: true, force: true });
          break;
        } catch {
          await sleep(250);
        }
      }
    },
  };
  return page;
}

let failed = 0;
for (const name of wanted.length > 0 ? wanted : names) {
  const scenario = (await import(pathToFileURL(join(here, `${name}.mjs`)).href)).default;
  const page = await launch();
  try {
    const note = await scenario(page);
    console.log(`ok    ${name}${note ? ` (${note})` : ""}`);
  } catch (e) {
    failed++;
    console.log(`FAIL  ${name}: ${e.message}`);
  } finally {
    await page.close();
  }
}
console.log(failed > 0 ? `${failed} failed` : "all passed");
process.exit(failed > 0 ? 1 : 0);
