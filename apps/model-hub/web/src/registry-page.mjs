// Build time only: one page per indexed registry artifact, in the model page's shape and Docker Hub's order.
//
// The shape is the model page's (hero, address, facts, tabbed panel), so crossing from a model to an image
// feels like the same site. The order inside is Docker Hub's repository page, because that is the page most
// readers have already learned: name and publisher, the pull command, Overview | Tags, the latest tag first,
// then the README. Every value on the page names the API it came from (scripts/registry.mjs); nothing is
// written by hand, and a row whose source gave nothing says so rather than showing a blank.

import MarkdownIt from "markdown-it";
import { createHash } from "node:crypto";
import * as R from "./render.mjs";
import * as B from "./braille.mjs";
import { htmlToMarkdown } from "./overview.mjs";

const README_LIMIT = 200_000;

// Where a row's profile lives under registry/data/artifacts/: the id hashed, because ids carry slashes.
// scripts/registry.mjs writes with this and the build reads with it, so they cannot disagree.
export const artifactFile = (id) => createHash("sha256").update(id).digest("hex").slice(0, 16) + ".json";

// One mark per row, always: a source that publishes no logo gets a letter on a hue drawn from the name. A data
// URL, so it needs no network and nothing is hotlinked. Shared with scripts/registry.mjs, which writes it into the
// index, and used here again so a page is never without one.
export const mark = (id) => {
  let h = 0;
  for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  const hue = h % 360;
  const letter = (id.split("/").pop() || "?")[0].toUpperCase();
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect width="40" height="40" rx="9" fill="hsl(${hue} 32% 22%)"/><text x="20" y="27" font-family="system-ui,sans-serif" font-size="19" font-weight="500" fill="hsl(${hue} 55% 78%)" text-anchor="middle">${letter}</text></svg>`;
  return "data:image/svg+xml;utf8," + encodeURIComponent(svg);
};

const esc = R.esc;
const num = (n) => (n == null ? null : Number(n).toLocaleString("en-US"));
const short = (n) => (n == null ? null : n >= 1e9 ? `${(n / 1e9).toFixed(1).replace(/\.0$/, "")}B` : n >= 1e6 ? `${(n / 1e6).toFixed(1).replace(/\.0$/, "")}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1).replace(/\.0$/, "")}K` : String(n));
const when = (iso) => (iso && !Number.isNaN(Date.parse(iso)) ? R.day(new Date(iso).toISOString().slice(0, 10)) : null);
const ago = (iso, now = Date.now()) => {
  if (!iso) return null;
  const days = Math.floor((now - Date.parse(iso)) / 86400000);
  if (!isFinite(days)) return null;
  return days <= 0 ? "today" : days === 1 ? "yesterday" : days < 31 ? `${days} days ago` : days < 365 ? `${Math.floor(days / 30)} months ago` : `${Math.floor(days / 365)} years ago`;
};
const shortDigest = (d) => (d ? R.shortAddress(d) : null);
const copy = (text, shown) => `<button type="button" class="copy" data-copy="${esc(text)}" aria-label="Copy ${esc(text)}">${esc(shown)}${R.icon.copy}</button>`;
const fact = (label, value) => (value ? `<div><dt>${label}</dt><dd>${value}</dd></div>` : "");

// The page's path under /registry/. Ids are registry paths already; the two Artifact Hub names with parentheses
// are the only ones that need encoding, and encodeURIComponent per segment keeps the slashes.
export const artifactPath = (id) => id.split("/").map((s) => encodeURIComponent(s)).join("/");

// The reference a tool takes, per source. docker.io is implied and library/ is how Docker Hub writes
// "official" in a path, not in a command.
export const reference = (r) => r.id.replace(/^docker\.io\//, "").replace(/^library\//, "");

// ---------------------------------------------------------------- commands: what actually uses this thing

export function commands(r, p) {
  const ref = reference(r);
  const tag = r.tag && !/^sha256-/.test(r.tag) ? r.tag : "latest";
  const src = r.registry;
  const out = [];
  const push = (source, label, command, note) => out.push({ source, label, command, note });
  if (src === "Docker Hub") {
    if (r.kind === "Model") push(src, "Pull the model", `docker model pull ${ref}`);
    else if (r.kind === "Skill") push(src, "Run the server", `docker mcp gateway run --servers ${r.name}`);
    else if (r.kind === "Sandbox kit") push(src, "Run in a sandbox", `sbx run ${ref}:${tag}`);
    else push(src, "Pull it", `docker pull ${ref}:${tag}`);
    if (r.digest && r.kind !== "Model" && r.kind !== "Skill") push(src, "By digest, the exact bytes", `docker pull ${ref}@${r.digest}`);
  } else if (src === "Microsoft Artifact Registry") {
    push(src, "Pull it", `docker pull ${r.id}:${tag}`);
    if (r.digest) push(src, "By digest, the exact bytes", `docker pull ${r.id}@${r.digest}`);
  } else if (src === "Artifact Hub") {
    const repo = p?.repo;
    if (r.kind === "Helm chart" && repo?.url) {
      if (/^oci:\/\//.test(repo.url)) push(src, "Install the chart", `helm install my-${r.name} ${repo.url.replace(/\/$/, "")}/${r.name}${p.version ? ` --version ${p.version}` : ""}`);
      else push(src, "Install the chart", `helm repo add ${repo.name} ${repo.url}\nhelm install my-${r.name} ${repo.name}/${r.name}${p.version ? ` --version ${p.version}` : ""}`);
    } else if (r.kind === "Kubectl plugin") push(src, "Install the plugin", `kubectl krew install ${r.name}`);
    else if (r.kind === "Helm plugin" && repo?.url) push(src, "Install the plugin", `helm plugin install ${repo.url}`);
    else if (r.kind === "Container image" && p?.containersImages?.[0]) push(src, "Pull the image", `docker pull ${p.containersImages[0]}`);
    else if (p?.contentUrl) push(src, "Fetch the package", `curl -LO ${p.contentUrl}`);
    else if (p?.containersImages?.[0]) push(src, "The image it runs", `docker pull ${p.containersImages[0]}`);
    else push(src, "Find it", `open ${r.home || "https://artifacthub.io"}`);
  }
  if (r.kappa) {
    if (r.held === "whole" && r.kind === "Helm chart") push("Hologram", "Pull the chart from here", `helm pull oci://${r.kappa.replace(/@.*$/, "")}${r.tag ? ` --version ${r.tag}` : ""}`);
    else if (r.held === "whole") push("Hologram", "Pull the exact bytes from here", `oras pull ${r.kappa}`);
    else push("Hologram", "Pull the exact bytes from here", `docker pull ${r.kappa}`);
  }
  return out;
}

// ---------------------------------------------------------------- README: markdown, rendered here, images as text

function markdown(base) {
  const md = new MarkdownIt({ html: false, linkify: true, typographer: false });
  const absolute = (url) => {
    if (!url) return null;
    if (/^(https?:|mailto:)/i.test(url)) return url;
    if (url.startsWith("#")) return url;
    if (/^[a-z]+:/i.test(url)) return null;
    if (!base) return null;
    try { return new URL(url, base.endsWith("/") ? base : base + "/").href; } catch { return null; }
  };
  const defaultLink = md.renderer.rules.link_open || ((t, i, o, e, s) => s.renderToken(t, i, o));
  md.renderer.rules.link_open = (tokens, i, options, env, self) => {
    const t = tokens[i];
    const href = absolute(t.attrGet("href"));
    if (href) t.attrSet("href", href); else t.attrSet("href", "#");
    if (href && !href.startsWith("#")) { t.attrSet("target", "_blank"); t.attrSet("rel", "noopener nofollow"); }
    return defaultLink(tokens, i, options, env, self);
  };
  // An image would be fetched from wherever the README points, by every reader, which is exactly what this site
  // never does. The alt text stays; the picture is one click away on the source.
  md.renderer.rules.image = (tokens, i) => { const alt = esc(tokens[i].content || ""); return alt ? `<span class="img-alt">${alt}</span>` : ""; };
  return md;
}

export function readme(r, p) {
  if (!p?.readme) return "";
  const body = htmlToMarkdown(p.readme.replace(/^---\s*\n[\s\S]*?\n---\s*\n/, ""));
  const truncated = body.length > README_LIMIT || p.readme.length >= 160_000;
  const html = markdown(p.readmeBase || r.home).render(truncated ? body.slice(0, README_LIMIT) : body);
  return `<section class="ov-section" aria-labelledby="ov-readme"><h3 id="ov-readme">Overview</h3><div class="md">${html}${truncated ? `<p><a href="${esc(r.home)}" target="_blank" rel="noopener">Continue on ${esc(r.registry)}</a></p>` : ""}</div></section>`;
}

// ---------------------------------------------------------------- the panels

const section = (id, title, body) => (body ? `<section class="ov-section" aria-labelledby="ov-${id}"><h3 id="ov-${id}">${title}</h3>${body}</section>` : "");
const facts = (rows) => {
  const body = rows.filter(([, v]) => v != null && v !== "").map(([label, value, src]) => `<div${src ? ` title="Source: ${esc(src)}"` : ""}><dt>${label}</dt><dd>${value}</dd></div>`).join("");
  return body ? `<dl class="ov-facts">${body}</dl>` : "";
};

function latestTag(r, p) {
  const t = (p?.tags || []).find((x) => x.name === r.tag) || (p?.tags || [])[0];
  if (!t) return "";
  return facts([
    ["Tag", esc(t.name)],
    // Artifact Hub's digest is the package's own (the chart archive); the row's address above is the wrapped
    // artifact this host holds, so the two differ by design and are named differently.
    [r.registry === "Artifact Hub" ? "Package digest" : "Digest", t.digest ? copy(t.digest, shortDigest(t.digest)) : null],
    ["Size", t.size != null && t.size > 0 ? R.bytes(t.size) : null],
    ["Platforms", t.platforms?.length ? esc(t.platforms.filter((x) => x !== "unknown/unknown").join(", ") || t.platforms.join(", ")) : null],
    ["Media type", t.mediaType ? `<span title="${esc(t.mediaType)}">${esc(t.mediaType.replace(/^application\/vnd\./, "").replace(/\+json$/, ""))}</span>` : null],
    ["App version", t.appVersion ? esc(t.appVersion) : null],
    ["Pushed", when(t.pushed)],
  ]);
}

function glance(r, p) {
  const sec = p?.security;
  const secText = sec ? ["critical", "high", "medium", "low"].filter((k) => sec[k]).map((k) => `${num(sec[k])} ${k}`).join(", ") : null;
  const api = p?.source?.api;
  return facts([
    ["Kind", esc(r.kind)],
    ["Publisher", p?.publisher?.url ? `<a href="${esc(p.publisher.url)}" target="_blank" rel="noopener">${esc(p.publisher.name)}</a>` : esc(p?.publisher?.name || r.publisher), api],
    ["Category", (p?.categories || []).length ? esc(p.categories.join(", ")) : r.category ? esc(r.category) : null, api],
    ["Licence", p?.license || r.license ? esc(p?.license || r.license) : null, api],
    ["Platforms", (r.architectures || []).filter((a) => a !== "unknown/unknown").length ? esc(r.architectures.filter((a) => a !== "unknown/unknown").join(", ")) : null],
    ["Media types", p?.mediaTypes?.length ? esc(p.mediaTypes.map((m) => m.replace(/^application\/vnd\./, "")).join(", ")) : null, api],
    ["Version", p?.version ? esc(p.version + (p.appVersion ? ` (app ${p.appVersion})` : "")) : null, api],
    ["Registered", when(p?.registered), api],
    ["Last pushed", when(p?.updated || r.updated), api],
    ["Tags", p?.tagsTotal != null ? num(p.tagsTotal) : null, api],
    ["Storage", r.storage != null ? R.bytes(r.storage) : null, api],
    ["Known CVEs", secText ? `<span class="${sec.critical || sec.high ? "bad" : "dim"}">${esc(secText)}</span>` : sec ? "none reported" : null, api ? `${api} (security_report_summary)` : null],
    ["Keywords", p?.keywords?.length ? esc(p.keywords.slice(0, 8).join(", ")) : null, api],
    ["Deploys", p?.containersImages?.length ? p.containersImages.slice(0, 4).map((i) => `<span class="mono" title="${esc(i)}">${esc(i.replace(/@sha256:.*$/, "").split("/").pop())}</span>`).join(", ") : null, api],
  ]);
}

function links(p) {
  const list = (p?.links || []).filter((l) => l?.url && /^https?:/.test(l.url));
  const people = (p?.maintainers || []).map((m) => m.name).filter(Boolean);
  if (!list.length && !people.length) return "";
  return `<ul class="ov-list">${list.map((l) => `<li><a href="${esc(l.url)}" target="_blank" rel="noopener">${esc(l.name)}${R.icon.external}</a></li>`).join("")}${people.length ? `<li class="ov-lede">Maintained by ${esc(people.slice(0, 5).join(", "))}${people.length > 5 ? ` and ${people.length - 5} more` : ""}</li>` : ""}</ul>`;
}

function run(r, p) {
  const list = commands(r, p);
  if (!list.length) return "";
  return `<div class="ov-run">${list.map((c) => `<div class="cmd"><span class="lbl">${esc(c.label)} <span class="dim">via ${esc(c.source)}</span></span><pre class="cmdline"><code>${esc(c.command)}</code>${copy(c.command, "Copy")}</pre></div>`).join("")}${p?.install ? `<details class="ov-card"><summary>Install notes from ${esc(r.registry)}</summary><div class="md">${markdown(p.readmeBase || r.home).render(htmlToMarkdown(p.install))}</div></details>` : ""}</div>`;
}

function tagsPane(r, p) {
  const tags = p?.tags || [];
  if (!tags.length) return `<p class="note">${p?.error ? `The tag list could not be read from ${esc(r.registry)} (${esc(String(p.error))}). ` : ""}No tags are recorded for this artifact in the index.${r.home ? ` <a href="${esc(r.home)}" target="_blank" rel="noopener">See it on ${esc(r.registry)}</a>.` : ""}</p>`;
  const ref = reference(r);
  const pull = (t) => r.registry === "Artifact Hub" ? null : r.registry === "Docker Hub" && (r.kind === "Model" || r.kind === "Skill") ? null : `docker pull ${r.registry === "Docker Hub" ? ref : r.id}:${t.name}`;
  const rows = tags.map((t) => {
    const cmd = pull(t);
    const plats = (t.platforms || []).filter((x) => x !== "unknown/unknown");
    return `<tr><td class="path" title="${esc(t.name)}">${esc(t.name)}${t.status && t.status !== "active" ? ` <span class="tag">${esc(t.status)}</span>` : ""}</td><td class="addr">${t.digest ? copy(t.digest, shortDigest(t.digest)) : "<span class=\"dim\">—</span>"}</td><td>${plats.length ? esc(plats.join(", ")) : t.appVersion ? `<span class="dim">app ${esc(t.appVersion)}</span>` : ""}</td><td class="size">${t.size != null && t.size > 0 ? R.bytes(t.size) : ""}</td><td>${when(t.pushed) || ""}</td><td class="dl">${cmd ? `<button type="button" class="dl-yes" data-copy="${esc(cmd)}" title="Copy: ${esc(cmd)}" aria-label="Copy pull command for ${esc(t.name)}">${R.icon.copy}</button>` : ""}</td></tr>`;
  }).join("\n");
  const total = p.tagsTotal ?? tags.length;
  return `<div class="section-head files-head"><p class="note">${tags.length === total ? `${num(total)} tag${total === 1 ? "" : "s"}` : `The ${num(tags.length)} most recent of ${num(total)} tags`}, as ${esc(r.registry)} lists them${r.home ? `; <a href="${esc(r.home)}" target="_blank" rel="noopener">all of them there</a>` : ""}.</p></div>
    <div class="scroll"><table id="tags">
      <thead><tr><th>Tag</th><th>Digest</th><th>Platforms</th><th class="size">Size</th><th>Pushed</th><th class="dl">Pull</th></tr></thead>
      <tbody>${rows}</tbody>
    </table></div>`;
}

const TRUST = {
  "upstream-digest": "the upstream registry reports this same digest",
  "upstream-attested": "the bytes hash to the digest the upstream published",
  "first-seen": "no upstream hash exists; first recorded here",
};

function signature(digest) {
  const bytes = B.hexToBytes(digest.split(":")[1]);
  return `<div class="signature" title="${esc(digest)}">
    <span class="label">Address</span>
    <span class="bx glyph" id="glyph" aria-hidden="true"><span>${B.cells(bytes.slice(0, 16))}</span><span>${B.cells(bytes.slice(16))}</span></span>
  </div>`;
}

function sources(r, p) {
  const list = [];
  if (r.home) list.push({ kind: r.registry, name: r.registry, page: r.home, external: true });
  if (r.kappa) list.push({ kind: "hologram", name: "Hologram", page: `/v2/${r.kappa.replace(/^[^/]+\//, "").replace(/@.*$/, "")}/manifests/${r.digest}`, external: false, title: `Held on this host: ${r.held === "whole" ? "the whole artifact" : "manifest and config; layers redirect to the upstream"}. ${TRUST[r.trust] || ""}` });
  if (!list.length) return "";
  return `<div class="sources">
    <span class="label">${list.length > 1 ? "Identical bytes on" : "Available from"}</span>
    <ul>${list.map((s) => `<li data-source="${esc(s.kind)}"${s.title ? ` title="${esc(s.title)}"` : ""}><span class="state">${R.icon.seal}${B.loader("orbit")}${R.icon.check}${R.icon.close}</span><a href="${esc(s.page)}"${s.external ? ' target="_blank" rel="noopener"' : ""}>${esc(s.name)}${s.external ? R.icon.external : ""}</a></li>`).join("")}</ul>
  </div>`;
}

// ---------------------------------------------------------------- the page body

export function artifactBody(r, p, { base, rank, now = Date.now() }) {
  const cmds = commands(r, p);
  const first = cmds[0];
  const cover = `<img class="avatar cover" src="${r.cover ? `${base}registry/${esc(r.cover)}` : esc(r.logo || mark(r.id))}" alt="" decoding="async">`;
  const publisher = p?.publisher?.name || r.publisher || r.org;
  const state = r.kappa ? ["addressed", r.trust === "first-seen" ? "Addressed" : "Verified"] : r.here ? ["addressed", "Held here"] : r.kappaStatus === "gone" ? ["skipped", "Gone upstream"] : ["pending", "Indexed"];
  const isNew = p?.registered && (now - Date.parse(p.registered)) / 86400000 <= 30;
  const tags = [
    `<span class="tag state ${state[0]}"${r.kappa ? ` title="${esc(TRUST[r.trust] || r.trust)}"` : ""}>${state[1]}</span>`,
    isNew ? `<span class="tag new">New</span>` : "",
    r.official ? `<span class="tag official">Official</span>` : "",
    p?.publisher?.verified && !r.official ? `<span class="tag">Verified publisher</span>` : "",
    r.signed ? `<span class="tag">Signed</span>` : "",
    `<span class="tag">${esc(r.kind)}</span>`,
    `<span class="tag">${esc(r.registry)}</span>`,
  ].join("");

  const menu = cmds.length ? `<div class="download-all">
        <button type="button" class="button success" id="pull-all" aria-haspopup="menu" aria-expanded="false" aria-controls="pull-menu" title="Choose a source; the command is copied">${R.icon.down}<span>Pull</span></button>
        <div class="menu" id="pull-menu" role="menu" aria-label="Pull" hidden>
          <p class="menu-note">Choose where to pull from. The command is copied to your clipboard.</p>
          ${cmds.map((c) => `<button type="button" role="menuitem" class="src" data-source="${esc(c.source)}" data-copy="${esc(c.command)}" title="${esc(c.command)}"><span class="state" aria-hidden="true">${B.loader("orbit")}</span><span class="label">${esc(c.source)}<span class="sub">${esc(c.label)}</span></span><span class="act">Copy</span></button>`).join("")}
          ${r.kappa ? "" : `<button type="button" role="menuitem" class="src" disabled data-state="off" title="This artifact is not addressed on this host yet"><span class="state" aria-hidden="true"></span><span class="label">Hologram<span class="sub">not addressed here yet</span></span><span class="act">Copy</span></button>`}
        </div>
      </div>` : "";
  const actions = r.kappa
    ? `<button type="button" class="button primary" data-check="${esc(r.id)}" data-kappa="${esc(r.kappa)}" data-digest="${esc(r.digest)}">${R.icon.check}${B.loader("orbit")}<span>Verify</span></button>${menu}`
    : `${menu}${r.home ? `<a class="button" href="${esc(r.home)}" target="_blank" rel="noopener">${esc(r.registry)}${R.icon.external}</a>` : ""}`;

  const factsPanel = [
    fact("Status", `<span class="${state[0] === "addressed" ? "ok" : state[0] === "skipped" ? "bad" : "dim"}">${state[1]}</span>`),
    rank ? fact("Trending", `#${rank}`) : "",
    r.pulls != null ? fact("Pulls", short(r.pulls)) : p?.stats?.pullsText ? fact("Pulls", esc(p.stats.pullsText)) : "",
    r.stars != null && r.stars > 0 ? fact("Stars", short(r.stars)) : "",
    p?.stats?.subscriptions ? fact("Subscribers", short(p.stats.subscriptions)) : "",
    r.storage != null ? fact("Storage", R.bytes(r.storage)) : r.size != null && r.size > 0 ? fact("Size", R.bytes(r.size)) : "",
    p?.tagsTotal != null ? fact("Tags", num(p.tagsTotal)) : "",
    fact("Sources", String(1 + (r.kappa ? 1 : 0))),
    r.tag ? fact("Tag", `<span title="${esc(r.tag)}">${esc(r.tag.length > 22 ? r.tag.slice(0, 20) + "…" : r.tag)}</span>`) : "",
    r.digest ? fact("Digest", copy(r.digest, shortDigest(r.digest))) : "",
    r.kappa ? fact("Address", copy(r.kappa, r.kappa.replace(/^[^/]+\//, "").replace(/@sha256:([0-9a-f]{8})[0-9a-f]+$/, "@sha256:$1…"))) : "",
    r.kappa ? fact("Held", r.held === "whole" ? "whole artifact" : "manifest and config") : "",
  ].join("");

  const summary = r.description ? `<p class="ov-summary">${esc(r.description)}</p>` : "";
  const provenance = `<p class="ov-provenance">Indexed on ${esc(R.day((p?.fetched || new Date(now).toISOString()).slice(0, 10)))} from ${esc(p?.source?.api ? p.source.api.replace(/^https?:\/\//, "").replace(/\?.*$/, "") : r.provenance)}${r.cover && r.coverVia ? `; logo from ${esc(r.coverVia)}` : r.cover ? "" : "; no logo published, so the mark is drawn from the name"}${r.kappa ? `; addressed here as ${esc(r.trust)}` : ""}.${p?.error ? ` The last refresh failed (${esc(String(p.error))}); this is the previous profile.` : ""}</p>`;

  return `<div class="head back-row"><a class="back" href="${base}registry/">${R.icon.left}Registry</a></div>
<section class="panel">
  <div class="hero">
    ${cover}
    <div class="who">
      <p class="org">${r.org && r.org !== "library" ? `<span class="mono">${esc(r.org)}</span> · ` : ""}By ${p?.publisher?.url ? `<a href="${esc(p.publisher.url)}" target="_blank" rel="noopener">${esc(publisher)}</a>` : esc(publisher)}${r.updated ? ` · Updated ${esc(ago(r.updated, now))}` : ""}</p>
      <h1>${esc(r.name)}</h1>
      <div class="tags">${tags}</div>
    </div>
    <div class="actions">${actions}</div>
  </div>
  ${r.digest || r.home ? `<div class="provenance">${r.digest ? signature(r.digest) : ""}${sources(r, p)}</div>` : ""}
  <p class="verdict" id="verdict" role="status" hidden></p>
</section>
<main class="detail">
  <section class="panel"><dl class="facts">${factsPanel}</dl></section>
  <section class="panel">
    <div class="panel-tabs" role="tablist" aria-label="Artifact">
      <button type="button" class="tab" role="tab" id="tab-overview" aria-controls="pane-overview" aria-selected="true">Overview</button>
      <button type="button" class="tab" role="tab" id="tab-tags" aria-controls="pane-tags" aria-selected="false" tabindex="-1">Tags${p?.tagsTotal != null ? `<span class="pill">${num(p.tagsTotal)}</span>` : ""}</button>
    </div>
    <div class="pane" id="pane-overview" role="tabpanel" aria-labelledby="tab-overview"><div class="ov">
      ${summary}
      ${section("latest", ((p?.tags || []).find((x) => x.name === r.tag) || (p?.tags || [])[0])?.name === r.tag ? "Latest tag" : "Newest tag", latestTag(r, p))}
      ${section("glance", "At a glance", glance(r, p))}
      ${section("run", first ? "Run it" : "", run(r, p))}
      ${section("links", "Links", links(p))}
      ${readme(r, p)}
      ${provenance}
    </div></div>
    <div class="pane" id="pane-tags" role="tabpanel" aria-labelledby="tab-tags" hidden>${tagsPane(r, p)}</div>
  </section>
</main>`;
}

// One agent-facing document per page, at the same address: the row and its profile, README included.
export const artifactJson = (r, p, { base, rank }) => JSON.stringify({
  format: "hologram.registry.artifact/v1",
  id: r.id,
  page: `${base}registry/${artifactPath(r.id)}/`,
  address: r.kappa || null,
  digest: r.digest || null,
  trust: r.trust || null,
  held: r.held || null,
  rank: rank || null,
  ...r,
  commands: commands(r, p),
  profile: p || null,
});

export const metaLine = (r, p) => (r.description || (p?.readme ? p.readme.replace(/^#.*$/m, "").replace(/[#*`>_\[\]!]/g, "").trim().split(/\n+/)[0]?.slice(0, 200) : "") || `${r.id} on ${r.registry}`);
