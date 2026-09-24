// Spaces — turn three Hugging Face static Spaces into sealed, self-contained runtimes under public/spaces/.
//
//   node scripts/spaces.build.mjs <hf-space-sources> <holo-os usr/lib/holo>
//
// The transform is small, recorded here, and deterministic (same sources → same locks, same roots):
//   1. root-absolute asset paths become relative (a Space assumed it owned an origin; here it owns a folder)
//   2. every third-party load is removed (no CDN script, no HF favicon); the page loads only its own files,
//      the hub's brand kit, and the one model host it declares — enforced by a CSP written into the page
//   3. a prelude installs the verified fetch (runtime/holo-spaces-hf-fetch.mjs) before the bundle runs: model
//      bytes are accepted only when they re-derive to the digest the model index names, then kept in this
//      Space's own OPFS store so the second open needs no network; progress goes out on a BroadcastChannel
//   4. the brand kit's tokens and type are applied, and the source Space's own headline/branding is hidden —
//      the card on the Spaces page already says what it is
//   5. each Space is sealed: holospace.lock.json (sha-256 per file, root over the map) + holospace.json
// The catalog public/spaces/spaces.json is written from the same pass. Nothing here runs at site build time.
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, readdirSync, statSync, rmSync, existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const [SRC, OSLIB] = process.argv.slice(2);
if (!SRC || !OSLIB) { console.error("usage: node scripts/spaces.build.mjs <hf-space-sources> <holo-os usr/lib/holo>"); process.exit(1); }
const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(WEB, "public", "spaces");
const RUNTIME = join(OUT, "runtime");
mkdirSync(RUNTIME, { recursive: true });
for (const f of ["holo-spaces-hf-fetch.mjs", "holo-opfs-kappastore.mjs"]) copyFileSync(join(OSLIB, f), join(RUNTIME, f));
// ONNX Runtime, pinned per Space (see `ort` below), vendored from <sources>/ort/<pin>/ — the exact files the
// bundles would otherwise fetch from jsDelivr, so a Space loads nothing from a CDN.
if (existsSync(join(SRC, "ort"))) for (const pin of readdirSync(join(SRC, "ort"))) {
  mkdirSync(join(RUNTIME, "ort", pin), { recursive: true });
  for (const f of readdirSync(join(SRC, "ort", pin))) copyFileSync(join(SRC, "ort", pin, f), join(RUNTIME, "ort", pin, f));
}
const sharedDigest = (relPath) => { const b = readFileSync(join(OUT, relPath)); return { sha256: sha256(b), bytes: b.length }; };

const HOST = "https://huggingface.co";
// connect-src: the model host and the places its `resolve` redirects to (the LFS/Xet CDNs). Nothing else.
const CONNECT = "'self' https://huggingface.co https://*.huggingface.co https://*.hf.co";
const csp = `default-src 'self'; script-src 'self' blob: 'wasm-unsafe-eval' 'unsafe-inline'; worker-src 'self' blob:; connect-src ${CONNECT}; img-src 'self' blob: data:; media-src 'self' blob: data:; style-src 'self' 'unsafe-inline'; font-src 'self'`;

const SPACES = [
  {
    id: "kokoro-tts", name: "Kokoro TTS", task: "Speech", src: "kokoro-webgpu",
    tagline: "Type a sentence, hear it. 82M parameters, on your GPU.",
    source: "https://huggingface.co/spaces/webml-community/kokoro-webgpu",
    models: ["onnx-community/Kokoro-82M-v1.0-ONNX"], modelBytes: 325532232 + 522240,
    page: "assets/index-DQNH7Ttg.js", worker: "assets/worker-BJyeiA2_.js", workerRef: "/assets/worker-BJyeiA2_.js",
    ort: "transformers-3.3.3", ortPath: "wasmPaths=`https://cdn.jsdelivr.net/npm/@huggingface/transformers@${z.env.version}/dist/`",
    icon: `<path d="M32 20v24M24 26v12M40 26v12M16 30v4M48 30v4"/>`,
  },
  {
    id: "remove-background", name: "Remove Background", task: "Image", src: "remove-background-webgpu",
    tagline: "Drop a photo; the subject comes back cut out. RMBG 1.4.",
    source: "https://huggingface.co/spaces/Xenova/remove-background-webgpu",
    models: ["briaai/RMBG-1.4"], modelBytes: 176220000,
    page: "assets/index-17_Va4sS.js", prelude: "page",
    ort: "onnxruntime-web-1.17.1", ortIn: "page", ortPath: 'wasmPaths="https://cdn.jsdelivr.net/npm/onnxruntime-web@1.17.1/dist/"',
    example: "https://images.pexels.com/photos/5965592/pexels-photo-5965592.jpeg?auto=compress&cs=tinysrgb&w=1024",   // Pexels licence: free to use; vendored as example.jpg
    icon: `<rect x="16" y="18" width="32" height="28" rx="4"/><circle cx="27" cy="29" r="4"/><path d="M18 44l10-10 6 6 6-8 8 12"/>`,
  },
  {
    id: "smollm-chat", name: "SmolLM Chat", task: "Chat", src: "smollm-webgpu",
    tagline: "A 360M-parameter assistant. The whole conversation stays on your device.",
    source: "https://huggingface.co/spaces/webml-community/smollm-webgpu",
    models: ["HuggingFaceTB/SmolLM-360M-Instruct"], modelBytes: 386700000,
    page: "assets/index-DJnDi8CK.js", worker: "assets/worker-ClBgkh03.js", workerRef: "/assets/worker-ClBgkh03.js",
    ort: "transformers-3.0.0-alpha.7", ortPath: "wasmPaths=`https://cdn.jsdelivr.net/npm/@huggingface/transformers@${H.env.version}/dist/`",
    icon: `<path d="M18 22h28v18H30l-8 7v-7h-4z"/><path d="M26 31h12"/>`,
  },
];

const sha256 = (b) => createHash("sha256").update(b).digest("hex");
const walk = (dir, base = dir, out = []) => { for (const n of readdirSync(dir)) { const p = join(dir, n); if (statSync(p).isDirectory()) walk(p, base, out); else out.push(relative(base, p).split("\\").join("/")); } return out.sort(); };

// The prelude, in the two shapes it takes: a module script in the page, or the head of a module worker.
// `rel` is the path from the file that carries it to public/spaces/runtime/.
const prelude = (s, rel) => `// ── Hologram Spaces prelude: every model byte is verified against the model index's digest before the
// runtime sees it, then kept in this Space's own store by digest. The host is a location, never the identity.
import { installVerifiedFetch } from "${rel}holo-spaces-hf-fetch.mjs";
import { OpfsKappaStore } from "${rel}holo-opfs-kappastore.mjs";
const __holoStoreP = OpfsKappaStore.open(${JSON.stringify("holo-space:" + s.id)}).catch(() => null);
const __holoChan = (() => { try { return new BroadcastChannel(${JSON.stringify("holo-spaces:" + s.id)}); } catch { return null; } })();
const __holoSay = (e) => { try { __holoChan && __holoChan.postMessage(e); } catch {} };
// a failure inside this context is a fact the page should show, not a silent spinner
self.addEventListener("error", (ev) => __holoSay({ kind: "error", error: String((ev && ev.message) || ev) }));
self.addEventListener("unhandledrejection", (ev) => __holoSay({ kind: "error", error: String((ev && ev.reason && (ev.reason.message || ev.reason)) || ev) }));
installVerifiedFetch({
  host: ${JSON.stringify(HOST)}, models: ${JSON.stringify(s.models)},
  store: { getByKey: async (a, h) => { const st = await __holoStoreP; return st ? st.getByKey(a, h) : null; },
           putVerified: async (a, h, b) => { const st = await __holoStoreP; if (st) await st.putVerified(a, h, b); } },
  onEvent: __holoSay,
}).then((vf) => __holoSay({ kind: "armed" }), (e) => __holoSay({ kind: "index-failed", error: String(e && e.message || e) }));
`;

const catalog = [];
for (const s of SPACES) {
  const from = join(SRC, s.src), to = join(OUT, s.id);
  rmSync(to, { recursive: true, force: true }); mkdirSync(join(to, "assets"), { recursive: true });

  // the page
  let html = readFileSync(join(from, "index.html"), "utf8");
  html = html.replace(/\s+crossorigin(?=[\s>])/g, "");
  html = html.replace(/(src|href)="\/assets\//g, '$1="./assets/');
  html = html.replace(/<link rel="icon"[^>]*>\s*/g, "");                                // the source's favicon
  html = html.replace(/<script[^>]*src="https?:\/\/[^"]*"[^>]*>\s*<\/script>\s*/g, "");   // any CDN script (MathJax)
  // MathJax came from a CDN; the chat calls MathJax.typeset() after each reply. A no-op stub keeps the app whole
  // (formulas render as text) without a third-party script.
  html = html.replace(/<script>\s*window\.MathJax[\s\S]*?<\/script>\s*/g, `<script>window.MathJax={typeset(){},typesetPromise(){return Promise.resolve()},startup:{promise:Promise.resolve()}};</script>\n`);
  html = html.replace(/<h1>[\s\S]*?<\/h1>\s*/g, "").replace(/<h4>[\s\S]*?<\/h4>\s*/g, "");   // the source's headline
  html = html.replace(/<title>[^<]*<\/title>/, `<title>${s.name} · Spaces · Hologram</title>`);
  html = html.replace(/<html lang="en">/i, `<html lang="en" class="dark" data-theme="dark" data-space="${s.id}">`);
  const head = [
    `<meta http-equiv="Content-Security-Policy" content="${csp}">`,
    `<link rel="icon" href="./icon.svg" type="image/svg+xml">`,
    `<link rel="stylesheet" href="/kit/hologram-warm.css">`,
    `<link rel="stylesheet" href="/kit/hologram-gap-tokens.css">`,
    `<link rel="stylesheet" href="/tokens.css">`,
    ...(s.prelude === "page" ? [`<script type="module">${prelude(s, "../runtime/")}</script>`] : []),
  ].join("\n");
  html = html.replace(/(<meta name="viewport"[^>]*>)/i, `$1\n${head}`);
  html = html.replace(/<\/head>/i, `<link rel="stylesheet" href="../runtime/space.css">\n</head>`);
  // loads only: a script/img/frame src or a stylesheet href on another origin. Anchors and xmlns are not loads.
  if (/(\ssrc|<link[^>]*\shref)=["']https?:\/\//i.test(html)) throw new Error(s.id + ": index.html still loads a foreign origin");
  writeFileSync(join(to, "index.html"), html);

  // the bundle: the worker path becomes relative to the bundle
  for (const f of walk(from)) {
    if (f === "index.html" || f === "README.md" || f.startsWith(".")) continue;
    const dest = join(to, f); mkdirSync(dirname(dest), { recursive: true });
    // ONNX Runtime's wasm: the bundles fetch it from jsDelivr by default. A Space loads nothing from a CDN,
    // so the path is rewritten to the runtime the site ships (public/spaces/runtime/ort/<pin>/), which the
    // Space's lock names by digest like everything else it depends on.
    // Absolute on purpose: ONNX Runtime resolves the glue from the bundle's URL and the .wasm from the glue's
    // own URL, so a relative prefix lands in two different places. The site ships at the root.
    // Origin-qualified too: ONNX Runtime 1.17's proxy runs inside a blob: worker, where a path-only URL
    // cannot even be parsed ("Failed to parse URL from /spaces/…"). `self` exists in a page and a worker alike.
    const ortFrom = () => `self.location.origin+${JSON.stringify(`/spaces/runtime/ort/${s.ort}/`)}`;
    if (f === s.page) {
      let js = readFileSync(join(from, f), "utf8");
      if (s.workerRef) {
        const before = js.length;
        js = js.split(`new URL("${s.workerRef}",import.meta.url)`).join(`new URL("./${s.worker.split("/").pop()}",import.meta.url)`);
        if (js.length === before) throw new Error(s.id + ": worker URL rewrite did not apply");
      }
      // the "try example" photo: served with the Space instead of fetched from a third party at click time
      if (s.example) { const before = js.length; js = js.split(s.example).join("./example.jpg"); if (js.length === before) throw new Error(s.id + ": example URL rewrite did not apply"); }
      if (s.ortIn === "page") { const before = js.length; js = js.split(s.ortPath).join(`wasmPaths=${ortFrom()}`); if (js.length === before) throw new Error(s.id + ": ort path rewrite did not apply (page)"); }
      writeFileSync(dest, js);
    } else if (f === s.worker) {
      let js = readFileSync(join(from, f), "utf8");
      const before = js.length; js = js.split(s.ortPath).join(`wasmPaths=${ortFrom()}`); if (js.length === before) throw new Error(s.id + ": ort path rewrite did not apply (worker)");
      writeFileSync(dest, prelude(s, "../../runtime/") + js);
    } else copyFileSync(join(from, f), dest);
  }
  writeFileSync(join(to, "icon.svg"), `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">${s.icon}</svg>\n`);

  // the manifest, then the seal over everything in the folder
  const manifest = {
    format: "hologram.space/v1", id: s.id, name: s.name, task: s.task, tagline: s.tagline, entry: "index.html",
    space: { source: s.source, sdk: "static", models: s.models, modelHost: HOST, modelBytes: s.modelBytes },
    requires: ["webgpu", "opfs"], capabilities: { storage: ["holo-space:" + s.id] },
  };
  writeFileSync(join(to, "holospace.json"), JSON.stringify(manifest, null, 2) + "\n");
  const files = {};
  for (const f of walk(to)) { if (f === "holospace.lock.json") continue; const b = readFileSync(join(to, f)); files[f] = { sha256: sha256(b), bytes: b.length }; }
  // the runtime the Space depends on, by digest: the verified fetch, the store, and its ONNX Runtime pin
  const shared = {};
  for (const f of ["runtime/holo-spaces-hf-fetch.mjs", "runtime/holo-opfs-kappastore.mjs", "runtime/space.css", ...readdirSync(join(RUNTIME, "ort", s.ort)).map((f) => `runtime/ort/${s.ort}/${f}`)].sort()) shared[f] = sharedDigest(f);
  const root = sha256(Buffer.from(JSON.stringify({ files, shared })));   // keys are sorted; the map is canonical
  const bytes = Object.values(files).reduce((a, f) => a + f.bytes, 0);
  writeFileSync(join(to, "holospace.lock.json"), JSON.stringify({ format: "hologram.space.lock/v1", id: s.id, root: "sha256:" + root, files, shared }, null, 2) + "\n");
  catalog.push({ id: s.id, name: s.name, task: s.task, tagline: s.tagline, root: "sha256:" + root, bytes, files: Object.keys(files).length,
    models: s.models, modelHost: HOST, modelBytes: s.modelBytes, source: s.source, requires: manifest.requires, entry: `${s.id}/index.html` });
  console.log(`sealed ${s.id}  root sha256:${root.slice(0, 12)}…  ${Object.keys(files).length} files  ${(bytes / 1e6).toFixed(1)} MB`);
}
writeFileSync(join(OUT, "spaces.json"), JSON.stringify({ format: "hologram.spaces.catalog/v1", spaces: catalog }, null, 2) + "\n");
console.log("catalog", join(OUT, "spaces.json"));
