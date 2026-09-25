// Likes and comments under an open App. The account service holds them (deploy/hub-account.mjs, /api/account/apps/…):
// reading is anonymous, writing needs the sign-in the header already offers, and an author is shown as a handle the
// service derives from a hash, never a name or an address. Nothing here reaches another origin.
//
//   const social = createSocial({ onNeedSignIn });
//   social.stats(id)                → { likes, dislikes, comments, mine: { react } | null } or null when unavailable
//   social.react(id, 1 | -1 | 0)    → the same, after the change
//   social.mountComments(el, id)    → draws the whole comments section for that App

const base = document.documentElement.dataset.base || "/";
const API = `${base}api/account`;
const configured = Boolean(document.getElementById("privy"));

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const I = (d) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;
export const ICON = {
  up: I('<path d="M7 10v11"/><path d="M15 5.9 14 10h5.8a2 2 0 0 1 1.9 2.6l-2.3 8a2 2 0 0 1-1.9 1.4H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.8a2 2 0 0 0 1.8-1.1L12 2a3.1 3.1 0 0 1 3 3.9Z"/>'),
  down: I('<path d="M17 14V3"/><path d="M9 18.1 10 14H4.2a2 2 0 0 1-1.9-2.6l2.3-8A2 2 0 0 1 6.5 2H20a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-2.8a2 2 0 0 0-1.8 1.1L12 22a3.1 3.1 0 0 1-3-3.9Z"/>'),
  sort: I('<path d="M4 6h16M4 12h11M4 18h6"/>'),
  chevron: I('<path d="m6 9 6 6 6-6"/>'),
};

// A hue per handle, so a thread reads at a glance without a single picture being fetched.
const hueOf = (s) => { let h = 0; for (const c of String(s)) h = (h * 31 + c.charCodeAt(0)) >>> 0; return h % 360; };
const avatar = (handle, cls = "") => `<span class="av ${cls}" style="--h:${hueOf(handle)}" aria-hidden="true">${esc((handle || "?").replace(/^user-/, "").charAt(0).toUpperCase())}</span>`;
export function ago(iso, now = Date.now()) {
  const s = Math.max(0, (now - Date.parse(iso)) / 1000);
  const [n, u] = s < 60 ? [0, "now"] : s < 3600 ? [Math.floor(s / 60), "minute"] : s < 86400 ? [Math.floor(s / 3600), "hour"] : s < 2592000 ? [Math.floor(s / 86400), "day"]
    : s < 31536000 ? [Math.floor(s / 2592000), "month"] : [Math.floor(s / 31536000), "year"];
  return u === "now" ? "just now" : `${n} ${u}${n === 1 ? "" : "s"} ago`;
}

export function createSocial({ onNeedSignIn = () => {} } = {}) {
  let mod = null, user = null;
  const auth = () => (configured ? (mod ??= import(`${base}auth.js`).then((m) => { m.onChange((u) => { const was = user; user = u; if (!!was !== !!u) changed.forEach((f) => f(u)); }); return m; })) : Promise.resolve(null));
  const changed = new Set();
  auth().catch(() => {});

  // A read goes with the token when there is one, so the answer says which likes and comments are this reader's.
  async function get(path) {
    const m = await auth().catch(() => null);
    if (m && user) { try { return await m.api(path); } catch {} }
    const res = await fetch(`${API}${path}`, { headers: { accept: "application/json" } });
    if (!res.ok || !(res.headers.get("content-type") || "").includes("json")) throw new Error(`answered ${res.status}`);
    return res.json();
  }
  // Acting without a session opens the header's own sign-in sheet; nothing is sent until there is one.
  async function ask() {
    const m = await auth().catch(() => null);
    if (!m) { toast("Sign-in is not available on this host"); return; }
    onNeedSignIn();
    m.open();
  }
  async function write(path, options) {
    const m = await auth();
    if (!m) throw Object.assign(new Error("sign-in is not available on this host"), { code: "off" });
    if (!user) { ask(); throw Object.assign(new Error("sign in first"), { code: "signin" }); }
    return m.api(path, options);
  }

  const stats = (id) => get(`/apps/${id}`).catch(() => null);
  const react = (id, value) => write(`/apps/${id}/react`, { method: "POST", body: { value } });

  function mountComments(el, id) {
    let sort = "top", data = null, alive = true;
    const signedIn = () => Boolean(user);

    const composer = ({ parent = null, placeholder = "Add a comment…", autofocus = false, small = false } = {}) => {
      const box = document.createElement("div");
      box.className = "cm-new";
      const me = signedIn() ? avatar(user.email || user.id || "me", small ? "small" : "") : `<span class="av me ${small ? "small" : ""}" aria-hidden="true"></span>`;
      box.innerHTML = `${me}<div class="cm-box"><textarea rows="1" maxlength="2000" placeholder="${esc(configured ? placeholder : "Comments open once sign-in is available on this host")}" aria-label="${esc(placeholder)}"${configured ? "" : " disabled"}></textarea>
        <div class="cm-btns"${autofocus ? "" : " hidden"}><button type="button" class="pill cancel">Cancel</button><button type="button" class="pill primary send" disabled>${parent ? "Reply" : "Comment"}</button></div></div>`;
      const ta = box.querySelector("textarea"), btns = box.querySelector(".cm-btns"), send = box.querySelector(".send");
      const fit = () => { ta.style.height = "auto"; ta.style.height = ta.scrollHeight + "px"; };
      ta.addEventListener("focus", () => { if (!signedIn()) { ta.blur(); ask(); return; } btns.hidden = false; });
      ta.addEventListener("input", () => { fit(); send.disabled = !ta.value.trim(); });
      ta.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !send.disabled) send.click(); });
      box.querySelector(".cancel").addEventListener("click", () => { ta.value = ""; fit(); send.disabled = true; if (parent) box.remove(); else btns.hidden = true; });
      send.addEventListener("click", async () => {
        send.disabled = true;
        try {
          await write(`/apps/${id}/comments`, { method: "POST", body: { text: ta.value, parent } });
          ta.value = ""; fit();
          if (parent) box.remove(); else btns.hidden = true;
          await load();
        } catch (e) { if (e.code !== "signin") toast(e.message); send.disabled = !ta.value.trim(); }
      });
      if (autofocus) queueMicrotask(() => ta.focus());
      return box;
    };

    const one = (c, reply = false) => {
      const el = document.createElement("div");
      el.className = "cm" + (reply ? " reply" : "");
      el.dataset.id = c.id;
      const r = c.mine?.react || 0;
      el.innerHTML = `${avatar(c.handle, reply ? "small" : "")}<div class="cm-main">
        <div class="cm-meta"><b>@${esc(c.handle)}</b><span title="${esc(new Date(c.at).toLocaleString())}">${ago(c.at)}</span></div>
        <p class="cm-text">${esc(c.text)}</p>
        <div class="cm-act">
          <button type="button" class="like" aria-pressed="${r === 1}" aria-label="Like">${ICON.up}<span class="n">${c.likes || ""}</span></button>
          <button type="button" class="dislike" aria-pressed="${r === -1}" aria-label="Dislike">${ICON.down}</button>
          <button type="button" class="txt reply-btn">Reply</button>
          ${c.mine?.own ? '<button type="button" class="txt del">Delete</button>' : ""}
        </div>
        <div class="cm-compose-slot"></div>
        ${!reply && c.replies?.length ? `<button type="button" class="cm-replies" aria-expanded="false">${ICON.chevron}${c.replies.length} ${c.replies.length === 1 ? "reply" : "replies"}</button><div class="cm-thread" hidden></div>` : ""}
      </div>`;
      const text = el.querySelector(".cm-text");
      requestAnimationFrame(() => {
        text.classList.add("clamp");
        if (text.scrollHeight > text.clientHeight + 2) {
          const more = document.createElement("button");
          more.type = "button"; more.className = "cm-readmore"; more.textContent = "Read more";
          more.addEventListener("click", () => { const open = text.classList.toggle("clamp"); more.textContent = open ? "Read more" : "Show less"; });
          text.after(more);
        } else text.classList.remove("clamp");
      });
      const vote = async (value) => {
        try {
          const out = await write(`/apps/${id}/comments/${c.id}/react`, { method: "POST", body: { value } });
          c.likes = out.likes; c.mine = out.mine;
          el.querySelector(".like").setAttribute("aria-pressed", String(out.mine.react === 1));
          el.querySelector(".dislike").setAttribute("aria-pressed", String(out.mine.react === -1));
          el.querySelector(".like .n").textContent = out.likes || "";
        } catch (e) { if (e.code !== "signin") toast(e.message); }
      };
      el.querySelector(".like").addEventListener("click", () => vote((c.mine?.react || 0) === 1 ? 0 : 1));
      el.querySelector(".dislike").addEventListener("click", () => vote((c.mine?.react || 0) === -1 ? 0 : -1));
      el.querySelector(".reply-btn").addEventListener("click", () => {
        if (!signedIn()) { ask(); return; }
        const slot = el.querySelector(".cm-compose-slot");
        if (!slot.firstChild) slot.appendChild(composer({ parent: c.id, placeholder: "Add a reply…", autofocus: true, small: true }));
      });
      el.querySelector(".del")?.addEventListener("click", async () => {
        if (!confirm("Delete this comment? Its replies go with it.")) return;
        try { await write(`/apps/${id}/comments/${c.id}`, { method: "DELETE" }); await load(); } catch (e) { toast(e.message); }
      });
      const toggle = el.querySelector(".cm-replies");
      if (toggle) {
        const box = el.querySelector(".cm-thread");
        toggle.addEventListener("click", () => {
          const open = toggle.getAttribute("aria-expanded") !== "true";
          toggle.setAttribute("aria-expanded", String(open));
          box.hidden = !open;
          if (open && !box.firstChild) for (const rep of c.replies) box.appendChild(one(rep, true));
        });
      }
      return el;
    };

    function draw() {
      if (!alive) return;
      el.innerHTML = "";
      const count = data ? data.count : null;
      const top = document.createElement("div");
      top.className = "cm-top";
      top.innerHTML = `<h2>${count == null ? "Comments" : `${count.toLocaleString("en-US")} ${count === 1 ? "Comment" : "Comments"}`}</h2>
        <div class="cm-sort"><button type="button" aria-haspopup="menu" aria-expanded="false">${ICON.sort}Sort by</button>
          <div class="menu" role="menu" hidden><button type="button" role="menuitemradio" data-sort="top" aria-checked="${sort === "top"}">Top comments</button><button type="button" role="menuitemradio" data-sort="new" aria-checked="${sort === "new"}">Newest first</button></div></div>`;
      const sb = top.querySelector(".cm-sort > button"), menu = top.querySelector(".cm-sort .menu");
      sb.addEventListener("click", (e) => { e.stopPropagation(); menu.hidden = !menu.hidden; sb.setAttribute("aria-expanded", String(!menu.hidden)); });
      menu.addEventListener("click", (e) => { const b = e.target.closest("[data-sort]"); if (!b) return; sort = b.dataset.sort; load(); });
      el.appendChild(top);
      el.appendChild(composer());
      if (!data) { const p = document.createElement("p"); p.className = "cm-empty"; p.textContent = "Comments are not available on this host right now."; el.appendChild(p); return; }
      if (!data.comments.length) { const p = document.createElement("p"); p.className = "cm-empty"; p.textContent = "No comments yet. Start the conversation."; el.appendChild(p); return; }
      data.comments.forEach((c, i) => { const node = one(c); node.style.animationDelay = `${Math.min(i, 8) * 30}ms`; el.appendChild(node); });
    }
    async function load() {
      data = await get(`/apps/${id}/comments?sort=${sort}`).catch(() => null);
      draw();
    }
    const again = () => load();
    changed.add(again);
    // one listener for the page's lifetime, not one per redraw: a click anywhere else folds the sort menu
    const fold = (e) => { if (e.target.closest(".cm-sort")) return; const m = el.querySelector(".cm-sort .menu"); if (m) { m.hidden = true; el.querySelector(".cm-sort > button")?.setAttribute("aria-expanded", "false"); } };
    document.addEventListener("click", fold);
    draw();
    load();
    return { reload: load, destroy() { alive = false; changed.delete(again); document.removeEventListener("click", fold); } };
  }

  return { stats, react, mountComments, onUser: (fn) => changed.add(fn), get signedIn() { return Boolean(user); }, configured };
}

let toastTimer = null;
export function toast(text) {
  const t = document.getElementById("toast");
  if (!t) return;
  t.textContent = text;
  t.classList.add("on");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("on"), 2200);
}
