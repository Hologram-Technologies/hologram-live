// Shared by the static build (Node) and the browser. Pure functions: data in, HTML out.
import * as B from "./braille.mjs";

export const PAGE_SIZE = 42;
const NEW_DAYS = 14;

export const SORTS = [
  ["trending", "Trending"],
  ["downloads", "Downloads"],
  ["likes", "Likes"],
  ["newest", "Newest"],
  ["size", "Parameters"],
];

export const RECENCY = [["Last 30 days", 30], ["90 days", 90], ["6 months", 183], ["1 year", 365]];

// key: field on the model (arrays allowed), param: URL name.
export const FACETS = [
  { key: "modality", param: "modality", label: "Modality", tab: "main", icon: "grid" },
  { key: "bucket", param: "size", label: "Parameter count", tab: "main", icon: "tag", fixed: ["Under 1B", "1 to 3B", "4 to 9B", "10 to 20B", "21 to 40B", "41 to 100B", "101 to 300B", "Over 300B"] },
  { key: "family", param: "family", label: "Model family", tab: "main", icon: "layers", search: true },
  { key: "stateLabel", param: "status", label: "Status", tab: "main", icon: "seal", fixed: ["Verified", "Queued", "Unverified"] },
  { key: "sources", param: "source", label: "Source", tab: "main", icon: "nodes" },
  { key: "recency", param: "released", label: "Release recency", tab: "main", icon: "calendar", fixed: RECENCY.map(([l]) => l), plain: true },
  { key: "arch", param: "arch", label: "Architecture", tab: "arch", icon: "cpu", search: true },
  { key: "languages", param: "language", label: "Language", tab: "language", icon: "globe", search: true },
  { key: "format", param: "format", label: "Format", tab: "format", icon: "file" },
  { key: "library", param: "library", label: "Library", tab: "library", icon: "box", search: true },
  { key: "license", param: "license", label: "License", tab: "license", icon: "scale", search: true },
];

export const TABS = [
  ["main", "Main", "sliders"], ["arch", "Architecture", "cpu"], ["language", "Language", "globe"],
  ["format", "Format", "file"], ["library", "Library", "box"], ["license", "License", "scale"],
];

export const STATE_LABEL = { addressed: "Verified", pending: "Queued", skipped: "Unverified" };

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

const trim = (n) => (n >= 100 ? Math.round(n).toString() : n.toFixed(1).replace(/\.0$/, ""));

export function count(n) {
  if (n >= 1e6) return `${trim(n / 1e6)}M`;
  if (n >= 1e3) return `${trim(n / 1e3)}k`;
  return String(n);
}

export function params(n) {
  if (!n) return null;
  if (n >= 1e12) return `${trim(n / 1e12)}T`;
  if (n >= 1e9) return `${trim(n / 1e9)}B`;
  if (n >= 1e6) return `${Math.round(n / 1e6)}M`;
  return `${Math.round(n / 1e3)}K`;
}

export function context(n) {
  if (!n) return null;
  if (n >= 1048576 * 0.95) return `${trim(n / 1048576)}M`;
  return `${Math.round(n / 1024)}K`;
}

export function bytes(n) {
  if (n == null) return "";
  const u = ["B", "KB", "MB", "GB", "TB", "PB"];
  let i = 0;
  while (n >= 1000 && i < u.length - 1) { n /= 1000; i++; }
  return `${i ? trim(n) : n} ${u[i]}`;
}

export function month(iso) {
  if (!iso) return "";
  const [y, m] = iso.split("-");
  return `${MONTHS[Number(m) - 1]} ${y}`;
}

export function day(iso) {
  const [y, m, d] = iso.split("-");
  return `${MONTHS[Number(m) - 1]} ${Number(d)}, ${y}`;
}

export const shortAddress = (a) => {
  const [alg, hex] = String(a).split(":");
  return `${alg}:${hex.slice(0, 8)}…${hex.slice(-6)}`;
};

// Icons: 1.5 strokes on a 24 grid; size comes from CSS.
const I = (d) => `<svg class="i" viewBox="0 0 24 24" aria-hidden="true">${d}</svg>`;
export const icon = {
  heart: I('<path d="M19.5 12.6 12 20l-7.5-7.4A4.6 4.6 0 0 1 12 6.6a4.6 4.6 0 0 1 7.5 6Z"/>'),
  down: I('<path d="M12 4v11m-5-5 5 5 5-5M5 20h14"/>'),
  calendar: I('<rect x="4" y="5" width="16" height="15" rx="2"/><path d="M4 10h16M9 3v4m6-4v4"/>'),
  search: I('<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>'),
  chevron: I('<path d="m6 9 6 6 6-6"/>'),
  left: I('<path d="m15 6-6 6 6 6"/>'),
  right: I('<path d="m9 6 6 6-6 6"/>'),
  copy: I('<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>'),
  check: I('<path d="m5 12 5 5 9-10"/>'),
  sliders: I('<path d="M4 7h10m4 0h2M4 17h2m4 0h10"/><circle cx="16" cy="7" r="2"/><circle cx="8" cy="17" r="2"/>'),
  reset: I('<path d="M4 12a8 8 0 1 0 2.4-5.7M4 4v4h4"/>'),
  moon: I('<path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5Z"/>'),
  sun: I('<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M4.6 4.6 6 6M18 18l1.4 1.4M2.5 12h2M19.5 12h2M4.6 19.4 6 18M18 6l1.4-1.4"/>'),
  image: I('<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="m3 17 5.5-5.5 4 4L15 13l6 6"/><circle cx="15.5" cy="8.5" r="1.5"/>'),
  nodes: I('<circle cx="6" cy="6" r="2.5"/><circle cx="18" cy="6" r="2.5"/><circle cx="12" cy="18" r="2.5"/><path d="M8.5 6h7M7.3 8.2l3.4 7.6M16.7 8.2l-3.4 7.6"/>'),
  close: I('<path d="m6 6 12 12M18 6 6 18"/>'),
  github: '<svg class="i mark" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 .5a11.5 11.5 0 0 0-3.64 22.41c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.04-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.96.1-.75.4-1.25.73-1.54-2.56-.29-5.25-1.28-5.25-5.68 0-1.26.45-2.28 1.19-3.08-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.8 1.19 1.82 1.19 3.08 0 4.41-2.7 5.38-5.26 5.67.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 12 .5Z"/></svg>',
  external: I('<path d="M14 4h6v6m0-6-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>'),
  grid: I('<rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/>'),
  tag: I('<path d="M3 12V4h8l9 9-8 8-9-9Z"/><circle cx="7.5" cy="8" r="1.2"/>'),
  layers: I('<path d="m12 4 9 5-9 5-9-5 9-5Z"/><path d="m3 14 9 5 9-5"/>'),
  seal: I('<path d="M12 3 20 7v5c0 5-3.5 8-8 9-4.5-1-8-4-8-9V7l8-4Z"/><path d="m9 12 2 2 4-4"/>'),
  cpu: I('<rect x="6" y="6" width="12" height="12" rx="2"/><path d="M10 10h4v4h-4zM9 3v3m6-3v3M9 18v3m6-3v3M3 9h3m-3 6h3m12-6h3m-3 6h3"/>'),
  globe: I('<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c3 3.5 3 14.5 0 18M12 3c-3 3.5-3 14.5 0 18"/>'),
  file: I('<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/>'),
  box: I('<path d="m12 3 8 4.5v9L12 21l-8-4.5v-9L12 3Z"/><path d="m4 7.5 8 4.5 8-4.5M12 12v9"/>'),
  scale: I('<path d="M12 4v16M7 20h10M5 7h14M5 7l-3 7a3 3 0 0 0 6 0L5 7Zm14 0-3 7a3 3 0 0 0 6 0l-3-7Z"/>'),
  sortAz: I('<path d="M4 8h9M4 12h6M4 16h3M17 5v14m-3-3 3 3 3-3"/>'),
  sortCount: I('<path d="M4 8h3M4 12h6M4 16h9M17 5v14m-3-3 3 3 3-3"/>'),
};

// Faceted mesh seeded by the model's own address: same bytes, same surface.
// Facets are lit only for addressed models; queued and gated models show the bare wireframe.
const ART_W = 260, ART_H = 120;
export function art(seed, lit) {
  let h = 2166136261;
  for (const c of String(seed)) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  const r = () => { h ^= h << 13; h ^= h >>> 17; h ^= h << 5; return ((h >>> 0) % 100000) / 100000; };
  const f = (n) => n.toFixed(1);
  const cols = 9, rows = 4, gx = ART_W / (cols - 1), gy = ART_H / (rows - 1), p = [];
  for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++) {
    p.push([x * gx + (r() - 0.5) * gx * 0.5, y * gy + (y && y < rows - 1 ? (r() - 0.5) * gy * 0.5 : 0)]);
  }
  let edges = "", faces = "";
  for (let y = 0; y < rows - 1; y++) for (let x = 0; x < cols - 1; x++) {
    const a = p[y * cols + x], b = p[y * cols + x + 1], c = p[(y + 1) * cols + x], d = p[(y + 1) * cols + x + 1];
    for (const t of [[a, b, d], [a, d, c]]) {
      const path = `M${t.map((q) => `${f(q[0])} ${f(q[1])}`).join("L")}Z`;
      edges += path;
      const v = r();
      if (lit && v < 0.35) faces += `<path d="${path}" opacity="${f(0.02 + v * 0.12)}"/>`;
    }
  }
  return `<svg class="art${lit ? " lit" : ""}" viewBox="0 0 ${ART_W} ${ART_H}" preserveAspectRatio="xMaxYMid slice" aria-hidden="true"><g class="facets">${faces}</g><path class="edges" d="${edges}"/></svg>`;
}

export function avatar(m, base, cls = "avatar") {
  return m.avatar
    ? `<img class="${cls}" src="${base}avatars/${esc(m.avatar)}" alt="" loading="lazy" decoding="async">`
    : `<span class="${cls} initials" aria-hidden="true">${esc(m.org.slice(0, 2).toUpperCase())}</span>`;
}

export function tags(m, { full = false } = {}) {
  return [
    full ? "" : `<span class="tag">${esc(m.org)}</span>`,
    `<span class="tag state ${m.state}">${STATE_LABEL[m.state]}${m.state === "pending" ? B.loader("scan") : ""}</span>`,
    m.isNew ? `<span class="tag new">New</span>` : "",
    m.params ? `<span class="tag">${params(m.params)}</span>` : "",
    m.context ? `<span class="tag">${context(m.context)}</span>` : "",
    m.modality !== "Other" && (full || m.modality !== "Text") ? `<span class="tag">${esc(m.modality)}</span>` : "",
    full && m.format !== "Other" ? `<span class="tag">${esc(m.format)}</span>` : "",
  ].join("");
}

export function meta(m) {
  // Most telling first; items that do not fit drop whole, never cut (the date goes first).
  const items = [`${icon.heart}${count(m.likes)}`, `${icon.down}${count(m.downloads)}`];
  const title = m.sources?.length > 1 ? ` title="Identical bytes on ${esc(m.sources.join(" and "))}" aria-label="${m.sources.length} sources"` : "";
  const spans = items.map((p) => `<span>${p}</span>`);
  if (title) spans.push(`<span${title}>${icon.nodes}${m.sources.length}</span>`);
  if (m.created) spans.push(`<span>${icon.calendar}${month(m.created)}</span>`);
  return spans.join("");
}

export function card(m, { base }) {
  return `<a class="card" href="${base}models/${esc(m.id)}/" title="${esc(m.id)}">
  ${art(m.manifest || m.id, m.state === "addressed")}
  <span class="tags">${tags(m)}</span>
  <span class="title">${esc(m.name)}</span>
  <span class="meta">${meta(m)}</span>
  ${avatar(m, base)}
</a>`;
}

// ---- Filtering, shared so the static first page equals the hydrated page.

export function prepare(models, snapshot) {
  const now = Date.parse(snapshot);
  for (const m of models) {
    m.stateLabel = STATE_LABEL[m.state];
    const age = m.created ? (now - Date.parse(m.created)) / 864e5 : Infinity;
    m.recency = RECENCY.filter(([, d]) => age <= d).map(([l]) => l);
    m.isNew = age <= NEW_DAYS;
  }
  return models;
}

export function parseState(search) {
  const p = new URLSearchParams(search);
  const state = { q: p.get("q") || "", sort: p.get("sort") || "trending", page: Math.max(1, Number(p.get("page")) || 1), tab: p.get("tab") || "main", f: {} };
  if (!SORTS.some(([k]) => k === state.sort)) state.sort = "trending";
  if (!TABS.some(([k]) => k === state.tab)) state.tab = "main";
  for (const f of FACETS) {
    const v = p.get(f.param);
    if (v) state.f[f.key] = v.split(",").filter(Boolean);
  }
  return state;
}

export function stateToSearch(state) {
  const p = new URLSearchParams();
  if (state.q) p.set("q", state.q);
  if (state.sort !== "trending") p.set("sort", state.sort);
  for (const f of FACETS) if (state.f[f.key]?.length) p.set(f.param, state.f[f.key].join(","));
  if (state.tab !== "main") p.set("tab", state.tab);
  if (state.page > 1) p.set("page", state.page);
  const s = p.toString().replace(/%2C/g, ",").replace(/\+/g, "%20");
  return s ? `?${s}` : "";
}

const valuesOf = (m, key) => { const v = m[key]; return Array.isArray(v) ? v : v == null ? [] : [String(v)]; };

function matches(m, state, skip) {
  if (state.q) {
    const hay = `${m.id} ${m.arch || ""} ${m.family || ""}`.toLowerCase();
    if (!state.q.toLowerCase().split(/\s+/).every((w) => hay.includes(w))) return false;
  }
  for (const f of FACETS) {
    if (f.key === skip) continue;
    const want = state.f[f.key];
    if (want?.length && !valuesOf(m, f.key).some((v) => want.includes(v))) return false;
  }
  return true;
}

const SORTERS = {
  trending: (a, b) => a.rank - b.rank,
  downloads: (a, b) => b.downloads - a.downloads,
  likes: (a, b) => b.likes - a.likes,
  newest: (a, b) => (b.created || "").localeCompare(a.created || ""),
  size: (a, b) => (b.params || 0) - (a.params || 0),
};

export function query(models, state, order = {}) {
  const results = models.filter((m) => matches(m, state)).sort(SORTERS[state.sort]);
  const facets = FACETS.map((f) => {
    const counts = new Map(f.fixed ? f.fixed.map((v) => [v, 0]) : []);
    for (const m of models) {
      const hit = matches(m, state, f.key);
      for (const v of valuesOf(m, f.key)) {
        if (f.key === "modality" && v === "Other") continue;
        counts.set(v, (counts.get(v) || 0) + (hit ? 1 : 0));
      }
    }
    const active = state.f[f.key] || [];
    const list = [...counts].filter(([v, n]) => n > 0 || active.includes(v));
    if (f.fixed) list.sort((a, b) => f.fixed.indexOf(a[0]) - f.fixed.indexOf(b[0]));
    else if (order[f.key] === "az") list.sort((a, b) => a[0].localeCompare(b[0]));
    else list.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
    return { ...f, values: list };
  });
  const pages = Math.max(1, Math.ceil(results.length / PAGE_SIZE));
  const page = Math.min(state.page, pages);
  return { results, facets, pages, page, slice: results.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE) };
}

// ---- Browse panels

export function filters(r, state, order = {}) {
  const tabs = TABS.map(([k, label, ic]) => `<button type="button" class="tab" role="tab" data-tab="${k}" aria-selected="${state.tab === k}">${icon[ic]}<span>${label}</span></button>`).join("");
  const sections = r.facets.filter((f) => f.tab === state.tab).map((f) => {
    const active = state.f[f.key] || [];
    const chips = f.values.map(([v, n]) => {
      const on = active.includes(v);
      return `<button type="button" class="chip${f.plain ? " plain" : ""}" data-facet="${f.key}" data-value="${esc(v)}" aria-pressed="${on}">${icon[f.icon]}<span class="label">${esc(v)}</span>${f.plain ? "" : `<span class="n">${count(n)}</span>`}</button>`;
    }).join("");
    const az = order[f.key] === "az";
    const tools = f.search
      ? `<div class="facet-tools"><label class="field compact">${icon.search}<input type="search" placeholder="Filter ${f.label.toLowerCase()}" data-facet-search="${f.key}" autocomplete="off" aria-label="Filter ${f.label.toLowerCase()}"></label><button type="button" class="control square" data-order="${f.key}" aria-label="${az ? "Sort by count" : "Sort A to Z"}" title="${az ? "Sort by count" : "Sort A to Z"}">${az ? icon.sortCount : icon.sortAz}</button></div>`
      : "";
    return `<section class="facet" data-key="${f.key}">
  <header><h2>${f.label}</h2><button type="button" class="control reset" data-reset="${f.key}"${active.length ? "" : " disabled"}>${icon.reset}Reset</button></header>
  ${tools}
  <div class="chips">${chips || '<p class="none">No values</p>'}</div>
  <button type="button" class="control more" hidden></button>
</section>`;
  }).join("");
  return `<div class="tabs" role="tablist">${tabs}</div><div class="sections">${sections}</div>`;
}

export function grid(r, { base }) {
  if (!r.results.length) return `<div class="empty"><p>No models match these filters.</p><button type="button" class="link" data-clear>Clear filters</button></div>`;
  return r.slice.map((m) => card(m, { base })).join("");
}

export function pager(r, state) {
  if (r.pages < 2) return "";
  const link = (p, label, extra = "") => `<a href="${stateToSearch({ ...state, page: p }) || "?"}" data-page="${p}"${extra}>${label}</a>`;
  const nums = [];
  for (let p = 1; p <= r.pages; p++) {
    if (p === 1 || p === r.pages || Math.abs(p - r.page) <= 1) nums.push(p);
    else if (nums[nums.length - 1] !== "…") nums.push("…");
  }
  return [
    r.page > 1 ? link(r.page - 1, icon.left, ' aria-label="Previous page"') : `<span class="off">${icon.left}</span>`,
    ...nums.map((p) => (p === "…" ? `<span class="gap">…</span>` : p === r.page ? `<span class="now" aria-current="page">${p}</span>` : link(p, p))),
    r.page < r.pages ? link(r.page + 1, icon.right, ' aria-label="Next page"') : `<span class="off">${icon.right}</span>`,
  ].join("");
}

export function title(state) {
  const parts = FACETS.flatMap((f) => state.f[f.key] || []);
  let t = parts.length ? `${parts.slice(0, 3).join(", ")}${parts.length > 3 ? ` +${parts.length - 3}` : ""} models` : "Models";
  if (state.page > 1) t += `, page ${state.page}`;
  return `${t} · Hologram Models Hub`;
}
