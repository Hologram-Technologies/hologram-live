// Build time only: the Overview tab, rendered from hologram-api v1/overview (hologram.overview/v1) and the model card.
// Same sections, same order for every model; a section with no trustworthy data is omitted.

import MarkdownIt from "markdown-it";
import * as R from "./render.mjs";

const HF = "https://huggingface.co";
const IMAGE_HOSTS = /^https:\/\/(huggingface\.co|cdn-uploads\.huggingface\.co|cdn-avatars\.huggingface\.co)\//;
const CARD_LIMIT = 200_000;

const v = (f) => (f && typeof f === "object" && "value" in f ? f.value : undefined);
const num = (n) => Number(n).toLocaleString("en-US");
const plural = (n, one, many = `${one}s`) => `${num(n)} ${n === 1 ? one : many}`;

function markdown(repo, revision) {
  const md = new MarkdownIt({ html: false, linkify: true, typographer: false });
  const absolute = (url, kind) => {
    if (!url || /^(https?:|mailto:|#)/i.test(url)) return url;
    if (/^[a-z]+:/i.test(url)) return null; // javascript:, data:, anything else
    const path = url.replace(/^\.?\//, "");
    return `${HF}/${repo}/${kind}/${revision}/${path}`;
  };
  const defaultLink = md.renderer.rules.link_open || ((t, i, o, e, s) => s.renderToken(t, i, o));
  md.renderer.rules.link_open = (tokens, i, options, env, self) => {
    const t = tokens[i];
    const href = absolute(t.attrGet("href"), "blob");
    if (href) t.attrSet("href", href); else t.attrSet("href", "#");
    if (href && !href.startsWith("#")) { t.attrSet("target", "_blank"); t.attrSet("rel", "noopener nofollow"); }
    return defaultLink(tokens, i, options, env, self);
  };
  md.renderer.rules.image = (tokens, i) => {
    const t = tokens[i];
    const src = absolute(t.attrGet("src"), "resolve");
    const alt = R.esc(t.content || "");
    return src && IMAGE_HOSTS.test(src) ? `<img src="${R.esc(src)}" alt="${alt}" loading="lazy" decoding="async">` : (alt ? `<span class="img-alt">${alt}</span>` : "");
  };
  return md;
}

const section = (id, title, body) => (body ? `<section class="ov-section" aria-labelledby="ov-${id}"><h3 id="ov-${id}">${title}</h3>${body}</section>` : "");

function facts(rows) {
  const body = rows.filter(([, value]) => value !== undefined && value !== null && value !== "").map(([label, value, src]) =>
    `<div${src ? ` title="Source: ${R.esc(src)}"` : ""}><dt>${label}</dt><dd>${value}</dd></div>`).join("");
  return body ? `<dl class="ov-facts">${body}</dl>` : "";
}

const src = (f) => (f && f.source ? f.source.replace(/@sha256:([0-9a-f]{8})[0-9a-f]+/, "@sha256:$1…") : "");

function glance(o) {
  const g = o.glance || {};
  const lic = v(g.license);
  const base = v(g.base);
  const params = v(g.parameters);
  const experts = v(g.experts), active = v(g.experts_active);
  const date = (iso) => (iso ? R.month(iso) : undefined);
  return facts([
    ["Task", R.esc(v(g.task)), src(g.task)],
    ["Input", v(g.input)?.map((x) => R.esc(x)).join(", "), src(g.input)],
    ["Output", v(g.output)?.map((x) => R.esc(x)).join(", "), src(g.output)],
    ["Parameters", params ? `${R.params(params)}${experts ? `, ${num(experts)} experts, ${num(active)} active` : ""}` : undefined, src(g.parameters)],
    ["Architecture", R.esc(v(g.architecture)), src(g.architecture)],
    ["Context", v(g.context) ? `${R.context(v(g.context))} tokens` : undefined, src(g.context)],
    ["Precision", R.esc(v(g.precision)), src(g.precision)],
    ["Format", R.esc(v(g.format)), src(g.format)],
    ["Library", R.esc(v(g.library)), src(g.library)],
    ["License", lic ? `${R.esc(lic.name)}${lic.commercial ? `<span class="ov-note ${lic.commercial}">${lic.commercial === "yes" ? "Commercial use" : lic.commercial === "no" ? "Non commercial" : "Conditions apply"}</span>` : ""}` : undefined, src(g.license)],
    ["Languages", v(g.languages)?.length ? R.esc(v(g.languages).slice(0, 8).join(", ") + (v(g.languages).length > 8 ? ` +${v(g.languages).length - 8}` : "")) : undefined, src(g.languages)],
    ["Base model", base ? `${R.esc(base.relation)} ${base.models.map((m) => `<a href="${HF}/${R.esc(m)}" target="_blank" rel="noopener">${R.esc(m)}</a>`).join(", ")}` : undefined, src(g.base)],
    ["Released", date(v(g.released)), src(g.released)],
    ["Updated", date(v(g.updated)), src(g.updated)],
    ["Likes", v(g.likes) !== undefined ? num(v(g.likes)) : undefined, src(g.likes)],
    ["Downloads, all time", v(g.downloads_all_time) !== undefined ? num(v(g.downloads_all_time)) : undefined, src(g.downloads_all_time)],
  ]);
}

function architecture(o) {
  const a = o.architecture || {};
  if (Object.keys(a).length < 2) return "";
  const heads = v(a.heads), kv = v(a.kv_heads);
  return facts([
    ["Layers", v(a.layers) !== undefined ? num(v(a.layers)) : undefined, src(a.layers)],
    ["Hidden size", v(a.hidden) !== undefined ? num(v(a.hidden)) : undefined, src(a.hidden)],
    ["Attention", heads ? `${plural(heads, "head")}${kv && kv !== heads ? `, ${kv === 1 ? "multi query" : "grouped query"}, ${plural(kv, "KV head")}` : ""}` : undefined, src(a.heads)],
    ["Experts", v(a.experts) ? `${num(v(a.experts))} total, ${num(v(a.experts_active) || 0)} active per token` : undefined, src(a.experts)],
    ["Vocabulary", v(a.vocab) !== undefined ? num(v(a.vocab)) : undefined, src(a.vocab)],
    ["Positions", v(a.context) !== undefined ? num(v(a.context)) : undefined, src(a.context)],
    ["RoPE theta", v(a.rope_theta) !== undefined ? num(v(a.rope_theta)) : undefined, src(a.rope_theta)],
    ["Tied embeddings", v(a.tied) !== undefined ? (v(a.tied) ? "Yes" : "No") : undefined, src(a.tied)],
    ["Quantization", R.esc(v(a.quantization)), src(a.quantization)],
    ["Vision encoder", R.esc(v(a.vision_encoder)), src(a.vision_encoder)],
    ["Audio encoder", R.esc(v(a.audio_encoder)), src(a.audio_encoder)],
  ]);
}

function benchmarks(o) {
  const rows = v(o.benchmarks) || [];
  if (!rows.length) return "";
  const verified = rows.filter((r) => r.verified).length;
  const body = rows.slice(0, 24).map((r) => `<tr>
    <td>${r.dataset && /\//.test(r.dataset) ? `<a href="${HF}/datasets/${R.esc(r.dataset)}" target="_blank" rel="noopener">${R.esc(r.dataset)}</a>` : R.esc(r.dataset || "")}${r.task && r.task !== r.dataset ? `<span class="ov-sub">${R.esc(r.task)}</span>` : ""}</td>
    <td class="size">${R.esc(typeof r.value === "number" ? String(Math.round(r.value * 100) / 100) : r.value)}</td>
    <td>${r.date ? R.month(r.date) : ""}</td>
    <td>${r.verified ? `<span class="ov-note yes">Verified by Hugging Face</span>` : `<span class="ov-note">Reported${r.source ? ` by <a href="${R.esc(r.source)}" target="_blank" rel="noopener">source</a>` : ""}</span>`}</td>
  </tr>`).join("");
  return `<p class="ov-lede">${plural(rows.length, "result")}, ${verified ? `${num(verified)} verified by Hugging Face` : "self reported by the authors"}.</p>
  <div class="scroll"><table class="ov-table"><thead><tr><th>Benchmark</th><th class="size">Score</th><th>Date</th><th>Evidence</th></tr></thead><tbody>${body}</tbody></table></div>`;
}

function family(o, repo) {
  const children = v(o.family?.children_raw) || {};
  const labels = { quantized: "Quantized", finetune: "Fine tuned", adapter: "Adapters", merge: "Merges" };
  const chips = Object.entries(labels).filter(([k]) => children[k]).map(([k, label]) =>
    `<a class="ov-chip" href="${HF}/models?other=base_model:${k}:${encodeURIComponent(repo)}" target="_blank" rel="noopener">${label}<span class="n">${R.count(children[k])}</span></a>`).join("");
  return chips ? `<p class="ov-lede">Models built on ${R.esc(repo.split("/")[1])}.</p><div class="ov-chips">${chips}</div>` : "";
}

function run(o, m) {
  const r = o.run || {};
  const lib = v(o.glance?.library), format = v(o.glance?.format);
  const rev = o.revision ? `, revision="${o.revision}"` : "";
  let code;
  if (format === "GGUF") code = `llama-server -hf ${m.id}`;
  else if (lib === "sentence-transformers") code = `from sentence_transformers import SentenceTransformer\n\nmodel = SentenceTransformer("${m.id}"${rev})`;
  else if (lib === "diffusers") code = `from diffusers import DiffusionPipeline\n\npipe = DiffusionPipeline.from_pretrained("${m.id}"${rev})`;
  else if (v(r.auto_model)) {
    const cls = [v(r.auto_model), v(r.processor)].filter(Boolean);
    code = `from transformers import ${cls.join(", ")}\n\nmodel = ${cls[0]}.from_pretrained("${m.id}"${rev})${cls[1] ? `\nprocessor = ${cls[1]}.from_pretrained("${m.id}"${rev})` : ""}`;
  } else code = `hf download ${m.id}${o.revision ? ` --revision ${o.revision}` : ""}`;
  const providers = v(r.providers) || [];
  return `${v(r.auto_model) ? `<p class="ov-lede">Loads with Transformers <code>${R.esc(v(r.auto_model))}</code>${v(r.processor) ? ` and <code>${R.esc(v(r.processor))}</code>` : ""}, pinned to the indexed revision.</p>` : `<p class="ov-lede">Pinned to the indexed revision.</p>`}
  <div class="ov-code"><pre><code>${R.esc(code)}</code></pre><button type="button" class="copy" data-copy="${R.esc(code)}" aria-label="Copy code">${R.icon.copy}</button></div>
  ${providers.length ? `<p class="ov-lede">Hosted inference, live now</p><div class="ov-chips">${providers.map((p) => `<span class="ov-chip">${R.esc(p.provider)}</span>`).join("")}</div>` : ""}`;
}

function papers(o) {
  const list = (v(o.papers) || []).filter((p) => p.title);
  const doi = v(o.doi);
  if (!list.length && !doi) return "";
  return `<ul class="ov-list">${list.map((p) => `<li><a href="${HF}/papers/${R.esc(p.id)}" target="_blank" rel="noopener">${R.esc(p.title)}</a><span class="ov-sub">${R.esc(p.first_author || "")}${p.authors > 1 ? " et al." : ""}${p.year ? `, ${R.esc(p.year)}` : ""}</span></li>`).join("")}${doi ? `<li><a href="https://doi.org/${R.esc(doi)}" target="_blank" rel="noopener">Cite with DOI ${R.esc(doi)}</a></li>` : ""}</ul>`;
}

function datasets(o) {
  const list = v(o.datasets) || [];
  return list.length ? `<div class="ov-chips">${list.slice(0, 12).map((d) => `<a class="ov-chip" href="${HF}/datasets/${R.esc(d)}" target="_blank" rel="noopener">${R.esc(d)}</a>`).join("")}</div>` : "";
}

function spaces(o) {
  const s = v(o.spaces);
  if (!s?.count) return "";
  return `<p class="ov-lede">Used in ${s.count >= 100 ? "100+" : num(s.count)} Spaces.</p><div class="ov-chips">${s.top.map((id) => `<a class="ov-chip" href="${HF}/spaces/${R.esc(id)}" target="_blank" rel="noopener">${R.esc(id)}</a>`).join("")}</div>`;
}

// Model cards mix markdown with HTML. Raw HTML never renders: common tags become markdown, the rest become text.
// Fenced code is left untouched.
function htmlToMarkdown(text) {
  const attr = (tag, name) => tag.match(new RegExp(`\\b${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`, "i"))?.slice(1).find((x) => x !== undefined) || "";
  return text.split(/(```[\s\S]*?```|~~~[\s\S]*?~~~)/).map((part, i) => {
    if (i % 2) return part;
    return part
      .replace(/<!--[\s\S]*?-->/g, "")
      .replace(/<(script|style|iframe|object|embed|svg|math|audio|video|picture|form)\b[\s\S]*?<\/\1\s*>/gi, "")
      .replace(/<(script|style|iframe|object|embed|audio|video|source|track|input|button)\b[^>]*\/?>/gi, "")
      .replace(/<img\b[^>]*>/gi, (tag) => { const src = attr(tag, "src"); return src ? `![${attr(tag, "alt").replace(/[[\]]/g, "")}](${src.replace(/\s/g, "%20")})` : ""; })
      .replace(/<a\b[^>]*>([\s\S]*?)<\/a\s*>/gi, (tag, inner) => { const href = attr(tag, "href"); const label = inner.replace(/<[^>]+>/g, "").trim(); return href ? `[${label || href}](${href.replace(/\s/g, "%20")})` : label; })
      .replace(/<br\s*\/?>/gi, "  \n")
      .replace(/<\/?(p|div|center|details|section|article|header|footer|figure|table|thead|tbody|tr|h[1-6])\b[^>]*>/gi, "\n")
      .replace(/<\/?[a-z][a-z0-9-]*\b[^>]*>/gi, "");
  }).join("");
}

function card(repo, revision, readme) {
  if (!readme) return "";
  const body = htmlToMarkdown(readme.replace(/^---\s*\n[\s\S]*?\n---\s*\n/, ""));
  const truncated = body.length > CARD_LIMIT;
  const html = markdown(repo, revision).render(truncated ? body.slice(0, CARD_LIMIT) : body);
  return `<details class="ov-card"><summary>Read the full model card</summary><div class="md">${html}${truncated ? `<p><a href="${HF}/${repo}" target="_blank" rel="noopener">Continue on Hugging Face</a></p>` : ""}</div></details>`;
}

function provenance(o) {
  const p = o.provenance || {};
  const addr = (s) => (s ? s.match(/^file:([^@]+)@sha256:([0-9a-f]+)/) : null);
  const parts = [addr(p.readme), addr(p.config)].filter(Boolean).map(([, path, hex]) =>
    `${R.esc(path)} <button type="button" class="copy" data-copy="sha256:${hex}" aria-label="Copy address of ${R.esc(path)}">sha256:${hex.slice(0, 8)}…${R.icon.copy}</button>`);
  return `<p class="ov-provenance">Derived on ${R.day(o.generated)} from Hugging Face${o.revision ? ` at revision <span class="mono">${R.esc(o.revision.slice(0, 8))}</span>` : ""}${parts.length ? `, ${parts.join(", ")}` : ""}${o.addressed ? "" : ". This model is not yet addressed, so its card comes from the main branch"}.</p>`;
}

export function overview(m, o, readme) {
  if (!o) return `<p class="note">The overview for this model is generated on the next daily index.</p>`;
  const summary = v(o.summary);
  return `<div class="ov">
    ${summary ? `<p class="ov-summary"${o.summary.source === "derived:template" ? ' data-template=""' : ""}>${R.esc(summary)}</p>` : ""}
    ${section("glance", "At a glance", glance(o))}
    ${section("architecture", "Architecture", architecture(o))}
    ${section("benchmarks", "Benchmarks", benchmarks(o))}
    ${section("family", "Family", family(o, m.id))}
    ${section("run", "Run it", run(o, m))}
    ${section("papers", "Papers", papers(o))}
    ${section("datasets", "Datasets", datasets(o))}
    ${section("spaces", "Spaces", spaces(o))}
    ${card(m.id, o.revision, readme)}
    ${provenance(o)}
  </div>`;
}

export const metaDescription = (o) => v(o?.summary);
