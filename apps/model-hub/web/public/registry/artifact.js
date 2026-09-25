// One artifact's page: tabs, the Pull menu, and Verify. Copy buttons are the site's own (app.js, which every page
// built by build.mjs carries, binds every [data-copy]); the tabs are bound here because app.js binds its tabs only
// inside a model page, and the Verify button is `data-check`, not `data-verify`, because app.js takes the latter for
// a model and would look for a manifest this page does not have.
//
// Verify is the one thing this page can do that the source's own page cannot: fetch the manifest this row is
// addressed by, from this origin, and hash the bytes here. A κ is the upstream's own digest (or a hash the
// upstream published), so the check is against a number that did not come from whoever served the bytes.
// Nothing here reaches another origin: the manifest comes from /v2/ on this host, and a host that does not
// hold it yet says so rather than pretending.

const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];

// ---- tabs: Overview | Tags, kept in the hash, moved with arrow keys (the model page's behaviour)
function panelTabs() {
  const tabs = $$(".panel-tabs [role=tab]");
  if (!tabs.length) return;
  const select = (tab, focus) => {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      document.getElementById(t.getAttribute("aria-controls")).hidden = !on;
    }
    if (focus) tab.focus();
    const hash = tab.id === "tab-tags" ? "#tags" : "";
    if (location.hash !== hash) history.replaceState(null, "", `${location.pathname}${location.search}${hash}`);
  };
  for (const t of tabs) {
    t.addEventListener("click", () => select(t));
    t.addEventListener("keydown", (e) => {
      const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
      if (step) { e.preventDefault(); select(tabs[(tabs.indexOf(t) + step + tabs.length) % tabs.length], true); }
    });
  }
  if (location.hash === "#tags") select(tabs[1]);
}

// ---- the Pull menu: one button, one row per source, each row copies its command (app.js does the copying)
function pullMenu() {
  const button = $("#pull-all"), menu = $("#pull-menu");
  if (!button || !menu) return;
  const open = (on) => { menu.hidden = !on; button.setAttribute("aria-expanded", String(on)); if (on) menu.querySelector("[role=menuitem]:not(:disabled)")?.focus(); };
  button.addEventListener("click", () => open(menu.hidden));
  document.addEventListener("click", (e) => { if (!menu.hidden && !e.target.closest(".download-all")) open(false); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !menu.hidden) { open(false); button.focus(); } });
  menu.addEventListener("click", (e) => {
    const row = e.target.closest("[role=menuitem][data-copy]");
    if (!row) return;
    const act = row.querySelector(".act");
    if (act) { act.textContent = "Copied"; setTimeout(() => { act.textContent = "Copy"; open(false); }, 900); }
  });
}

// ---- Verify: the manifest, from this host, hashed here
const ACCEPT = [
  "application/vnd.oci.image.manifest.v1+json",
  "application/vnd.oci.image.index.v1+json",
  "application/vnd.docker.distribution.manifest.v2+json",
  "application/vnd.docker.distribution.manifest.list.v2+json",
].join(", ");
const hex = (buf) => [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");

function verify() {
  const button = $("[data-check]");
  if (!button) return;
  const verdict = $("#verdict");
  const source = $('.sources li[data-source="hologram"]');
  const say = (cls, html) => { verdict.hidden = false; verdict.className = `verdict ${cls}`; verdict.innerHTML = html; };
  button.addEventListener("click", async () => {
    const kappa = button.dataset.kappa, want = button.dataset.digest;
    const repo = kappa.replace(/^[^/]+\//, "").replace(/@.*$/, "");
    const url = `/v2/${repo}/manifests/${want}`;
    button.classList.add("busy"); button.disabled = true;
    if (source) source.dataset.state = "busy";
    say("", "Fetching the manifest from this host…");
    const started = performance.now();
    try {
      const res = await fetch(url, { headers: { Accept: ACCEPT } });
      if (res.status === 404) throw Object.assign(new Error("not served here yet"), { code: 404 });
      if (!res.ok) throw new Error(`answered ${res.status}`);
      const bytes = await res.arrayBuffer();
      const got = "sha256:" + hex(await crypto.subtle.digest("SHA-256", bytes));
      const ms = Math.round(performance.now() - started);
      if (got === want) {
        if (source) source.dataset.state = "ok";
        say("ok", `<b>The manifest matches its address.</b> ${bytes.byteLength.toLocaleString("en-US")} bytes fetched from this host and hashed in your browser in ${ms} ms: sha256 equals the digest the upstream registry reports.`);
      } else {
        if (source) source.dataset.state = "bad";
        say("bad", `<b>The bytes do not match.</b> This host served ${got.slice(0, 19)}… for an address that names ${want.slice(0, 19)}…. Do not use them.`);
      }
    } catch (e) {
      if (source) source.dataset.state = e.code === 404 ? "" : "bad";
      say(e.code === 404 ? "" : "bad", e.code === 404
        ? "This host does not serve that manifest yet: the κ mirror for this row is published with the next registry sync. Until then, pull by digest from the upstream and your client checks the same number."
        : `<b>Could not fetch the manifest.</b> ${e.message}.`);
    }
    button.classList.remove("busy"); button.disabled = false;
  });
}

panelTabs();
pullMenu();
verify();
