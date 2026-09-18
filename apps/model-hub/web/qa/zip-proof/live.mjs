// The real button on the real site: open a model page, press the green Download, choose a source, let the engine save the zip.
//
//   node live.mjs <chromium|firefox|webkit> <org/name> [source kind] [site]        (inside mcr.microsoft.com/playwright)
import { chromium, firefox, webkit } from "playwright";
import fs from "node:fs";
import path from "node:path";

const [engineName = "chromium", model = "Qwen/Qwen3.5-2B", source = "huggingface.co", site = "https://hub.uor.foundation/"] = process.argv.slice(2);
const OUT = path.resolve("out");
fs.mkdirSync(OUT, { recursive: true });
const result = { engine: engineName, model, site };
const browser = await { chromium, firefox, webkit }[engineName].launch({ downloadsPath: OUT });
try {
  result.version = browser.version();
  const page = await (await browser.newContext({ acceptDownloads: true })).newPage();
  await page.goto(`${site}models/${model}/`);
  // A first visit is not controlled by the worker yet; the second is, like any returning visitor.
  await page.waitForFunction(() => navigator.serviceWorker?.ready.then(() => true), null, { timeout: 60000 });
  await page.reload();
  await page.waitForFunction(() => !!navigator.serviceWorker.controller, null, { timeout: 30000 });
  result.declared = await page.evaluate(async (m) => Number((await fetch(`zip/${m}/huggingface.co.zip`.replace(/^/, document.documentElement.dataset.base), { method: "HEAD" })).headers.get("content-length")), model);
  await page.evaluate(() => { new BroadcastChannel("model-hub-zip").onmessage = (e) => { window.__last = e.data; }; });
  const started = Date.now();
  const [download] = await Promise.all([page.waitForEvent("download", { timeout: 120000 }), page.click("#dl-all").then(() => page.click(`#dl-menu [data-kind="${source}"]`))]);
  result.filename = download.suggestedFilename();
  const file = await download.path().catch(() => null);
  result.failure = await download.failure();
  result.seconds = Math.round((Date.now() - started) / 1000);
  result.saved = file ? fs.statSync(file).size : 0;
  result.sizeMatches = result.saved === result.declared;
  result.MBps = Math.round(result.saved / 1e6 / Math.max(1, result.seconds));
  await page.waitForTimeout(500);
  result.button = await page.evaluate(() => document.querySelector("#dl-all span").textContent);
  result.message = await page.evaluate(() => document.querySelector("#dl-progress")?.textContent || document.querySelector("#dl-status")?.textContent);
  result.worker = await page.evaluate(() => window.__last && { state: window.__last.state, sent: window.__last.sent, total: window.__last.total, files: window.__last.files, detail: window.__last.detail });
  if (file) fs.renameSync(file, path.join(OUT, "live.zip"));
} catch (error) {
  result.error = String(error.message || error).split(String.fromCharCode(10))[0];
} finally {
  await browser.close().catch(() => {});
}
console.log(JSON.stringify(result));
