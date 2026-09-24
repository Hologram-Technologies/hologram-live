// The documentation: web/docs/*.md → dist/docs/**.
//
// One Markdown file per page, with a small frontmatter (title, description, group, order). Each page is written
// three ways from that one source, so none of them can drift: the HTML page at /docs/<slug>/, a Markdown twin at
// /docs/<slug>.md for an agent that would rather not parse markup, and one line in /llms.txt, the index of all of
// them. The API reference is not written by hand at all: it is generated here from public/openapi.json, the
// document every route is already held to, so a route cannot be documented without being described.

import { readFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import MarkdownIt from "markdown-it";

const GROUPS = ["Start", "Concepts", "Connect", "Reference"];
// The reader's order, not the document's: what you call first sits first.
const TAG_ORDER = ["Models", "Files", "Registry", "MCP", "Objects", "Account", "Health", "Discovery"];
// Paths a page may link to with a leading slash that the site does not ship itself, because another service
// on the same host answers them. Everything else must exist in dist/ or the build refuses.
const SERVED_ELSEWHERE = new Set(["/docs"]);

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

function frontmatter(text) {
  const m = text.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n/);
  if (!m) throw new Error("a docs page needs frontmatter: title, description, group, order");
  const meta = {};
  for (const line of m[1].split(/\r?\n/)) {
    const i = line.indexOf(":");
    if (i > 0) meta[line.slice(0, i).trim()] = line.slice(i + 1).trim();
  }
  for (const k of ["title", "description", "group", "order"]) if (!meta[k]) throw new Error(`docs page is missing ${k}`);
  if (!GROUPS.includes(meta.group)) throw new Error(`docs page group "${meta.group}" is not one of ${GROUPS.join(", ")}`);
  return { meta, body: text.slice(m[0].length) };
}

// A relative link names another page by slug; an absolute one names a site path. Both are resolved against the
// base the site is served under, and both are checked by check() after the build.
function href(target, base, { md = false } = {}) {
  if (/^([a-z]+:|#)/i.test(target)) return target;
  if (target.startsWith("/")) return `${base}${target.slice(1)}`;
  const [slug, hash = ""] = target.split("#");
  if (md) return `${base}docs/${slug}.md${hash ? `#${hash}` : ""}`;
  return `${base}docs/${slug === "index" ? "" : `${slug}/`}${hash ? `#${hash}` : ""}`;
}

function renderer(base) {
  const md = new MarkdownIt({ html: true, linkify: false, typographer: false });
  const open = md.renderer.rules.link_open || ((tokens, idx, options, env, self) => self.renderToken(tokens, idx, options));
  md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
    const t = tokens[idx];
    const target = t.attrGet("href") || "";
    t.attrSet("href", href(target, base));
    if (/^[a-z]+:/i.test(target)) { t.attrSet("rel", "noopener"); t.attrSet("target", "_blank"); }
    return open(tokens, idx, options, env, self);
  };
  // Headings carry an id so a page can be linked into. Slugs are the heading text, lowercased, dashed.
  // A wide table scrolls inside its own box instead of pushing the page sideways; the site already has .scroll.
  md.renderer.rules.table_open = () => `<div class="scroll"><table>`;
  md.renderer.rules.table_close = () => `</table></div>`;
  md.renderer.rules.heading_open = (tokens, idx, options, env, self) => {
    const t = tokens[idx];
    const text = tokens[idx + 1].children.filter((c) => c.type === "text" || c.type === "code_inline").map((c) => c.content).join("");
    t.attrSet("id", text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, ""));
    return self.renderToken(tokens, idx, options);
  };
  return md;
}

export async function load(dir) {
  const pages = [];
  for (const f of (await readdir(dir)).filter((n) => n.endsWith(".md")).sort()) {
    const { meta, body } = frontmatter(await readFile(join(dir, f), "utf8"));
    pages.push({ slug: f.replace(/\.md$/, ""), ...meta, order: Number(meta.order), body });
  }
  pages.sort((a, b) => a.order - b.order);
  const seen = new Set();
  for (const p of pages) { if (seen.has(p.order)) throw new Error(`two docs pages share order ${p.order}`); seen.add(p.order); }
  return pages;
}

// ---- the API reference, from the document

const METHODS = ["get", "post", "put", "patch", "delete", "head"];
const inline = (md, s) => md.renderInline(String(s ?? ""));
const schemaText = (s = {}) => {
  if (s.enum) return s.enum.map((v) => `<code>${esc(v)}</code>`).join(", ");
  if (s.type === "array") return `array of ${s.items?.type || "string"}`;
  return esc(s.type || (s.$ref ? s.$ref.split("/").pop() : "object"));
};

export function apiReference(spec, md) {
  const byTag = new Map();
  for (const [path, item] of Object.entries(spec.paths)) {
    for (const m of METHODS) {
      const op = item[m];
      if (!op) continue;
      const tag = (op.tags || ["Other"])[0];
      if (!byTag.has(tag)) byTag.set(tag, []);
      byTag.get(tag).push({ method: m.toUpperCase(), path, op, params: [...(item.parameters || []), ...(op.parameters || [])] });
    }
  }
  const tags = [...byTag.keys()].sort((a, b) => (TAG_ORDER.indexOf(a) + 1 || 99) - (TAG_ORDER.indexOf(b) + 1 || 99));
  const tagInfo = Object.fromEntries((spec.tags || []).map((t) => [t.name, t.description || ""]));
  const out = [];
  for (const tag of tags) {
    const id = `ref-${tag.toLowerCase()}`;
    out.push(`<section class="api-group" id="${id}"><h2>${esc(tag)}</h2>${tagInfo[tag] ? `<p class="api-lead">${inline(md, tagInfo[tag].split("\n")[0])}</p>` : ""}`);
    for (const { method, path, op, params } of byTag.get(tag)) {
      const auth = (op.security || spec.security || []).length ? Object.keys((op.security || spec.security)[0] || {}).join(", ") : "";
      out.push(`<article class="api-op" id="${esc(op.operationId || `${method}-${path}`)}">`);
      out.push(`<h3><span class="method ${method.toLowerCase()}">${method}</span><code>${esc(path)}</code></h3>`);
      if (op.summary) out.push(`<p class="api-summary">${inline(md, op.summary)}.</p>`);
      if (op.description) out.push(`<div class="api-desc">${md.render(op.description)}</div>`);
      if (auth) out.push(`<p class="api-auth">Auth: <code>${esc(auth)}</code></p>`);
      if (params.length) {
        out.push(`<div class="scroll"><table><thead><tr><th>Parameter</th><th>In</th><th>Type</th><th>Description</th></tr></thead><tbody>`);
        for (const p of params) out.push(`<tr><td><code>${esc(p.name)}</code>${p.required ? "" : '<span class="opt"> optional</span>'}</td><td>${esc(p.in)}</td><td>${schemaText(p.schema)}</td><td>${inline(md, p.description)}</td></tr>`);
        out.push(`</tbody></table></div>`);
      }
      const rb = op.requestBody?.content;
      if (rb) out.push(`<p class="api-body">Body: ${Object.keys(rb).map((t) => `<code>${esc(t)}</code>`).join(", ")}${op.requestBody.description ? ` — ${inline(md, op.requestBody.description)}` : ""}</p>`);
      const responses = Object.entries(op.responses || {});
      if (responses.length) {
        out.push(`<div class="scroll"><table class="api-responses"><thead><tr><th>Status</th><th>Meaning</th></tr></thead><tbody>`);
        for (const [code, r] of responses) {
          const types = Object.keys(r.content || {});
          out.push(`<tr><td><code>${esc(code)}</code></td><td>${inline(md, r.description)}${types.length ? ` <span class="types">${types.map(esc).join(", ")}</span>` : ""}</td></tr>`);
        }
        out.push(`</tbody></table></div>`);
      }
      out.push(`</article>`);
    }
    out.push(`</section>`);
  }
  return out.join("\n");
}

// ---- pages

export function render(pages, { base, spec }) {
  const md = renderer(base);
  const apiHtml = apiReference(spec, md);
  return pages.map((p) => {
    let html = md.render(p.body);
    if (html.includes("<!--openapi:reference-->")) html = html.replace("<!--openapi:reference-->", apiHtml);
    return { ...p, html, path: p.slug === "index" ? "docs/index.html" : `docs/${p.slug}/index.html`, url: href(p.slug, base) };
  });
}

export function sidebar(pages, current, base) {
  const groups = GROUPS.map((g) => [g, pages.filter((p) => p.group === g)]).filter(([, ps]) => ps.length);
  return `<nav class="docs-nav" aria-label="Documentation">${groups.map(([g, ps]) => `<h2>${esc(g)}</h2><ul>${ps
    .map((p) => `<li><a href="${href(p.slug, base)}"${p.slug === current ? ' aria-current="page"' : ""}>${esc(p.title)}</a></li>`)
    .join("")}</ul>`).join("")}</nav>`;
}

export function article(page, pages, base) {
  const i = pages.findIndex((p) => p.slug === page.slug);
  const prev = pages[i - 1], next = pages[i + 1];
  return `<article class="docs-page">
<header class="docs-head"><p class="docs-group">${esc(page.group)}</p><h1>${esc(page.title)}</h1><p class="docs-lead">${esc(page.description)}</p><a class="docs-twin" href="${href(page.slug, base, { md: true })}" title="This page as Markdown">Markdown</a></header>
${page.html}
<footer class="docs-foot">${prev ? `<a class="prev" href="${href(prev.slug, base)}"><span>Previous</span>${esc(prev.title)}</a>` : "<span></span>"}${next ? `<a class="next" href="${href(next.slug, base)}"><span>Next</span>${esc(next.title)}</a>` : ""}</footer>
</article>`;
}

// The Markdown twin: the same source, links resolved to absolute .md twins, a header that names the index.
export function twin(page, { endpoint }) {
  const body = page.body
    .replace(/\]\(([^)]+)\)/g, (m, target) => `](${/^([a-z]+:|#)/i.test(target) ? target : `${endpoint}${href(target, "/", { md: !target.startsWith("/") })}`})`)
    .replace("<!--openapi:reference-->", `The reference itself is generated from ${endpoint}/openapi.json; read that document directly.`);
  return `> Documentation index: ${endpoint}/llms.txt\n\n# ${page.title}\n\n> ${page.description}\n\n${body.trim()}\n`;
}

// /llms.txt: the shortest path first, as the endpoint's own research asks, then every page of these docs with
// its Markdown twin, then the machine-readable documents. Built from the same pages, so it lists exactly what ships.
export function llms(pages, { endpoint, spec, snapshot, models }) {
  const ops = Object.values(spec.paths).reduce((n, item) => n + METHODS.filter((m) => item[m]).length, 0);
  const line = (p) => `- [${p.title}](${endpoint}/docs/${p.slug}.md): ${p.description}`;
  const host = endpoint.replace(/^https?:\/\//, "");
  return [
    "# Hologram Model Hub",
    "",
    "> One base URL for open models. Every file is named by the SHA-256 of its bytes and every object by its BLAKE3; fetch from a source that is up, check it yourself, and trust no host, this one included. No account, no key, no SDK.",
    "",
    "## The short way",
    "",
    `- \`export HF_ENDPOINT=${endpoint}\` and every tool built on \`huggingface_hub\` reads from here: \`hf download <owner>/<name>\`, \`snapshot_download\`, \`from_pretrained\`, vLLM, SGLang. llama.cpp reads \`MODEL_ENDPOINT\` instead. Same commands, same cache; \`main\` is the indexed revision.`,
    `- \`ollama pull ${host}/<owner>/<name>:<quant>\` for any GGUF in the index; Ollama verifies the SHA-256 itself.`,
    `- \`oras pull ${host}/<owner>/<name>:latest\` for a whole model as an OCI artifact, every layer a raw file whose digest is its SHA-256.`,
    "- `GET /api/models?search=qwen&limit=5` to find a model in a few hundred bytes; `GET /api/models/<owner>/<name>/tree/main` for its files, each with `oid`, the SHA-256 it must have.",
    "- `GET /<owner>/<name>/resolve/main/<path>` answers `302` to a source that is up right now; `X-Hub-Source` says which. Put `/via/ipfs`, `/via/modelscope` or `/via/huggingface` in front to pin one.",
    `- Check a whole download with no tool of ours: \`curl -s ${endpoint}/<owner>/<name>/resolve/main/SHA256SUMS | sha256sum -c\``,
    `- MCP: \`${endpoint}/mcp\`, Streamable HTTP, no key; tools \`search_models\`, \`get_model\`, \`resolve_file\`.`,
    "",
    "## The one rule",
    "",
    "Hash what arrives. Keep a file only if its SHA-256 equals the value from the index (`oid` in the tree, `sha256` in the model object, the layer digest in a manifest). The expected hash never comes from the source that served the bytes, and this server does not verify on read.",
    "",
    "## Documentation",
    "",
    ...GROUPS.flatMap((g) => { const ps = pages.filter((p) => p.group === g); return ps.length ? [`### ${g}`, "", ...ps.map(line), ""] : []; }),
    "## Machine-readable",
    "",
    `- [OpenAPI 3.1](${endpoint}/openapi.json): the whole endpoint, ${Object.keys(spec.paths).length} paths and ${ops} operations, every example recorded from the live hub`,
    `- [agent.md](${endpoint}/agent.md): the whole hub on one screen, for an agent that just arrived`,
    `- [Hub descriptor](${endpoint}/.well-known/model-hub.json): today's catalog address and the object routes`,
    `- [Agent card](${endpoint}/.well-known/agent-card.json): the same contract as skills`,
    `- [Source health](${endpoint}/api/hub/health): which byte sources are up and the order preferred`,
    `- [Capabilities](${endpoint}/api/v1/capabilities): what the object server can do and its limits`,
    "",
    `Index of ${snapshot}: ${models} models. Everything here is anonymous and read-only; sign-in exists only for a saved list and model requests.`,
    "",
  ].join("\n");
}

// ---- the gate
//
// Refuses a build in which a docs page links to something that is not there, or has nothing a reader can run.
export function check(pages, { exists }) {
  const slugs = new Set(pages.map((p) => p.slug));
  const problems = [];
  for (const p of pages) {
    if (!/```/.test(p.body)) problems.push(`${p.slug}: no code block; a page with nothing to run is not a page`);
    for (const [, target] of p.body.matchAll(/\]\(([^)\s]+)\)/g)) {
      if (/^([a-z]+:|#)/i.test(target)) continue;
      if (target.startsWith("/")) {
        const path = target.split("#")[0];
        if (!SERVED_ELSEWHERE.has(path) && !exists(path)) problems.push(`${p.slug}: links to ${target}, which the build does not ship`);
      } else if (!slugs.has(target.split("#")[0])) problems.push(`${p.slug}: links to page "${target}", which does not exist`);
    }
  }
  if (problems.length) throw new Error(`docs:\n  ${problems.join("\n  ")}`);
  return `docs ok: ${pages.length} pages, every internal link resolves, every page has something to run`;
}
