/**
 * page-agent UI test runner for sqail (via tauri-driver)
 *
 * Usage:
 *   npx tsx tests/runner.ts <test-file>
 *   npx tsx tests/runner.ts tests/01-app-launch.test.md
 *
 * Requires:
 *   - Debug binary built: pnpm tauri build --debug --no-bundle
 *   - tauri-driver installed: cargo install tauri-driver
 *   - WebKitWebDriver on PATH (Linux)
 *   - LLM config in .env (see .env.example)
 */

import "dotenv/config";
import { remote, type Browser } from "webdriverio";
import { spawn, spawnSync, type ChildProcess } from "child_process";
import { createServer, type Server, type IncomingMessage, type ServerResponse } from "http";
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// ── Config ──────────────────────────────────────────────────────────────────

const LLM_MODEL = process.env.PAGE_AGENT_MODEL ?? "gpt-4o";
const LLM_BASE_URL =
  process.env.PAGE_AGENT_BASE_URL ?? "https://api.openai.com/v1";
const LLM_API_KEY = process.env.PAGE_AGENT_API_KEY ?? "";
const STEP_DELAY_MS = parseInt(process.env.TEST_STEP_DELAY ?? "1000", 10);
const LLM_PROXY_PORT = 18923;

const APP_BINARY = path.resolve(
  __dirname,
  "../src-tauri/target/debug/sqail",
);

const IIFE_PATH = path.resolve(
  __dirname,
  "../node_modules/page-agent/dist/iife/page-agent.demo.js",
);

const TAURI_DRIVER_PORT = 4444;

// ── Markdown parser ─────────────────────────────────────────────────────────

interface TestCase {
  name: string;
  preconditions: string;
  steps: string[];
  expected: string[];
}

function parseTestFile(filePath: string): { suite: string; tests: TestCase[] } {
  const content = fs.readFileSync(filePath, "utf-8");
  const lines = content.split("\n");

  let suite = "";
  const tests: TestCase[] = [];
  let current: Partial<TestCase> | null = null;
  let section: "steps" | "expected" | "preconditions" | null = null;

  for (const line of lines) {
    if (line.startsWith("# ") && !line.startsWith("## ")) {
      suite = line.replace("# ", "").trim();
    } else if (line.startsWith("## Test: ")) {
      if (current?.name) {
        tests.push(current as TestCase);
      }
      current = {
        name: line.replace("## Test: ", "").trim(),
        preconditions: "",
        steps: [],
        expected: [],
      };
      section = null;
    } else if (line.startsWith("**Preconditions:**")) {
      section = "preconditions";
      const inline = line.replace("**Preconditions:**", "").trim();
      if (inline && current) current.preconditions = inline;
    } else if (line.startsWith("**Steps:**")) {
      section = "steps";
    } else if (line.startsWith("**Expected:**")) {
      section = "expected";
    } else if (current && section) {
      const trimmed = line.replace(/^\d+\.\s*/, "").replace(/^-\s*/, "").trim();
      if (!trimmed) continue;
      if (section === "steps") current.steps!.push(trimmed);
      else if (section === "expected") current.expected!.push(trimmed);
      else if (section === "preconditions")
        current.preconditions += " " + trimmed;
    }
  }
  if (current?.name) tests.push(current as TestCase);

  return { suite, tests };
}

// ── tauri-driver lifecycle ──────────────────────────────────────────────────

let tauriDriverProcess: ChildProcess | null = null;

function startTauriDriver(): Promise<void> {
  return new Promise((resolve, reject) => {
    const driverBin =
      process.env.TAURI_DRIVER_PATH ??
      path.join(process.env.HOME ?? "", ".cargo", "bin", "tauri-driver");

    tauriDriverProcess = spawn(driverBin, [], {
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, WEBKIT_DISABLE_DMABUF_RENDERER: "1" },
    });

    tauriDriverProcess.on("error", (err) => {
      reject(new Error(`Failed to start tauri-driver: ${err.message}`));
    });

    // Give it a moment to start listening
    const timeout = setTimeout(() => resolve(), 2000);

    tauriDriverProcess.stderr?.on("data", (data: Buffer) => {
      const msg = data.toString();
      // tauri-driver logs to stderr when ready
      if (msg.includes("listening")) {
        clearTimeout(timeout);
        resolve();
      }
    });
  });
}

function stopTauriDriver() {
  if (tauriDriverProcess) {
    tauriDriverProcess.kill();
    tauriDriverProcess = null;
  }
}

// ── Build check ─────────────────────────────────────────────────────────────

function ensureBinary() {
  if (fs.existsSync(APP_BINARY)) return;

  console.log("Debug binary not found, building...");
  const result = spawnSync(
    "pnpm",
    ["tauri", "build", "--debug", "--no-bundle"],
    {
      cwd: path.resolve(__dirname, ".."),
      stdio: "inherit",
      shell: true,
    },
  );
  if (result.status !== 0) {
    console.error("Build failed");
    process.exit(1);
  }
}

// ── LLM proxy server ────────────────────────────────────────────────────────
// The Tauri webview can't call external APIs directly (CORS).
// This tiny proxy runs in Node and forwards requests to the real LLM endpoint.

let proxyServer: Server | null = null;

function startLlmProxy(): Promise<void> {
  return new Promise((resolve) => {
    proxyServer = createServer(async (req: IncomingMessage, res: ServerResponse) => {
      // CORS headers so the webview can call us
      res.setHeader("Access-Control-Allow-Origin", "*");
      res.setHeader("Access-Control-Allow-Methods", "POST, OPTIONS");
      res.setHeader("Access-Control-Allow-Headers", "Content-Type, Authorization");

      if (req.method === "OPTIONS") {
        res.writeHead(204);
        res.end();
        return;
      }

      // Read incoming body
      const chunks: Buffer[] = [];
      for await (const chunk of req) chunks.push(chunk as Buffer);
      const body = Buffer.concat(chunks).toString();

      // Forward to the real LLM endpoint, preserving the request path
      try {
        const targetUrl = `${LLM_BASE_URL}${req.url ?? "/chat/completions"}`;
        const response = await fetch(targetUrl, {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            ...(LLM_API_KEY && { Authorization: `Bearer ${LLM_API_KEY}` }),
          },
          body,
        });

        const responseBody = await response.text();
        res.writeHead(response.status, { "Content-Type": "application/json" });
        res.end(responseBody);
      } catch (err: any) {
        res.writeHead(502, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ error: { message: err.message } }));
      }
    });

    proxyServer.listen(LLM_PROXY_PORT, "127.0.0.1", () => {
      console.log(`LLM proxy listening on http://127.0.0.1:${LLM_PROXY_PORT}`);
      resolve();
    });
  });
}

function stopLlmProxy() {
  if (proxyServer) {
    proxyServer.close();
    proxyServer = null;
  }
}

// ── Page-agent injection & execution ────────────────────────────────────────

async function injectPageAgent(browser: Browser): Promise<void> {
  let pageAgentCode = fs.readFileSync(IIFE_PATH, "utf-8");

  // Strip the demo auto-initialization at the end of the IIFE.
  // The demo script ends with a setTimeout that creates window.pageAgent
  // with a mask-enabled config — we don't want that.
  const demoMarker = "const DEMO_MODEL=";
  const markerIdx = pageAgentCode.lastIndexOf(demoMarker);
  if (markerIdx !== -1) {
    // Keep everything up to the demo block, close the IIFE
    pageAgentCode = pageAgentCode.substring(0, markerIdx) + "})();";
  }

  // Inject the cleaned IIFE bundle
  await browser.execute(pageAgentCode);

  // Wait for PageAgent constructor to be available
  await browser.waitUntil(
    async () => {
      const ready = await browser.execute(() => !!(window as any).PageAgent);
      return ready;
    },
    { timeout: 10_000, timeoutMsg: "PageAgent did not load" },
  );

  // Dispose any auto-created agent and remove overlay elements
  await browser.execute(() => {
    const w = window as any;
    if (w.pageAgent) {
      try { w.pageAgent.dispose(); } catch {}
      w.pageAgent = null;
    }
    // Remove mask and panel overlay elements
    document
      .querySelectorAll(
        "#page-agent-runtime_simulator-mask, #page-agent-runtime_agent-panel, #playwright-highlight-container",
      )
      .forEach((el) => el.remove());
  });

  // Create our agent instance pointing at the local LLM proxy
  await browser.execute(
    (model: string, proxyUrl: string) => {
      const PA = (window as any).PageAgent;
      (window as any).__testAgent = new PA({
        model,
        baseURL: proxyUrl,
        apiKey: "proxied",
        language: "en-US",
        enableMask: false,
        maxSteps: 10,
        maxRetries: 1,
      });
      // Permanently hide the panel — override show() so it can't re-appear
      const panel = (window as any).__testAgent.panel;
      if (panel) {
        panel.hide();
        panel.show = () => {};
      }
    },
    LLM_MODEL,
    `http://127.0.0.1:${LLM_PROXY_PORT}`,
  );

  console.log("page-agent injected and configured\n");
}

async function executeStep(
  browser: Browser,
  instruction: string,
): Promise<string> {
  const result = await browser.executeAsync(
    (cmd: string, done: (result: string) => void) => {
      const agent = (window as any).__testAgent;
      agent
        .execute(cmd)
        .then((r: any) => {
          const data = r?.data ?? "";
          done(r?.success === false ? `error: ${data}` : "ok");
        })
        .catch((e: any) => done(`error: ${e.message ?? e}`));
    },
    instruction,
  );
  return result as string;
}

async function verifyExpected(
  browser: Browser,
  expectations: string[],
): Promise<{ pass: boolean; details: string }> {
  const prompt =
    `Look at the current page and verify these expectations. ` +
    `For each one, determine if it is true or false based on what you see.\n\n` +
    expectations.map((e, i) => `${i + 1}. ${e}`).join("\n");

  const result = await browser.executeAsync(
    (cmd: string, done: (result: string) => void) => {
      const agent = (window as any).__testAgent;
      agent
        .execute(cmd)
        .then(() => done("ok"))
        .catch((e: any) => done(`error: ${e.message ?? e}`));
    },
    prompt,
  );

  const resultStr = result as string;
  const pass = !resultStr.startsWith("error");
  return { pass, details: resultStr };
}

// ── Runner ──────────────────────────────────────────────────────────────────

async function run() {
  const args = process.argv.slice(2);
  const testFile = args.find((a) => !a.startsWith("--"));

  if (!testFile) {
    console.error("Usage: npx tsx tests/runner.ts <test-file>");
    process.exit(1);
  }

  const filePath = path.resolve(testFile);
  if (!fs.existsSync(filePath)) {
    console.error(`Test file not found: ${filePath}`);
    process.exit(1);
  }

  if (!LLM_API_KEY) {
    console.error("Set PAGE_AGENT_API_KEY in .env (see .env.example)");
    process.exit(1);
  }

  ensureBinary();

  const { suite, tests } = parseTestFile(filePath);
  console.log(`\n━━━ ${suite} ━━━ (${tests.length} tests)\n`);

  // Start LLM proxy and tauri-driver
  await startLlmProxy();
  console.log("Starting tauri-driver ...");
  await startTauriDriver();

  let browser: Browser | null = null;
  let passed = 0;
  let failed = 0;

  try {
    // Connect via WebDriver to the real Tauri app
    browser = await remote({
      hostname: "127.0.0.1",
      port: TAURI_DRIVER_PORT,
      logLevel: "warn",
      capabilities: {
        "tauri:options": {
          application: APP_BINARY,
        },
        "timeouts": {
          script: 120_000,
          pageLoad: 30_000,
          implicit: 5_000,
        },
      } as any,
    });

    // Wait for the app to render
    console.log("App launched, waiting for UI ...");
    await browser.pause(2000);

    // Skip splash screen via JS injection
    await browser.execute(() => {
      const btn = document.querySelector("[data-splash-skip]");
      if (btn) (btn as HTMLElement).click();
    });

    // Inject page-agent (mask disabled, panel hidden)
    console.log("Injecting page-agent ...");
    await injectPageAgent(browser);

    for (const test of tests) {
      console.log(`▶ ${test.name}`);

      if (test.preconditions) {
        console.log(`  preconditions: ${test.preconditions}`);
      }

      // Execute steps
      let stepFailed = false;
      for (let i = 0; i < test.steps.length; i++) {
        const step = test.steps[i];
        console.log(`  step ${i + 1}: ${step}`);
        const result = await executeStep(browser, step);
        if (result.startsWith("error")) {
          console.log(`  ✗ STEP FAILED: ${result}`);
          stepFailed = true;
          break;
        }
        console.log(`  step ${i + 1}: done`);
        if (STEP_DELAY_MS > 0) await browser.pause(STEP_DELAY_MS);
      }

      if (stepFailed) {
        console.log(`  ✗ FAIL\n`);
        failed++;
        continue;
      }

      // Verify expectations
      const { pass, details } = await verifyExpected(browser, test.expected);
      if (pass) {
        console.log(`  ✓ PASS\n`);
        passed++;
      } else {
        console.log(`  ✗ FAIL: ${details}\n`);
        failed++;
      }
    }

    console.log(`━━━ Results: ${passed} passed, ${failed} failed ━━━\n`);
  } finally {
    if (browser) {
      try {
        await browser.deleteSession();
      } catch {}
    }
    stopTauriDriver();
    stopLlmProxy();
  }

  process.exit(failed > 0 ? 1 : 0);
}

// Cleanup on unexpected exit
process.on("SIGINT", () => {
  stopTauriDriver();
  process.exit(1);
});
process.on("SIGTERM", () => {
  stopTauriDriver();
  process.exit(1);
});

run().catch((err) => {
  console.error(err);
  stopTauriDriver();
  process.exit(1);
});
