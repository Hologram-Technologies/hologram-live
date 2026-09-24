// The header, on every page of the site.
//
// The theme switch and the sign-in control are the same two controls on the browse table, on a model page,
// on the landing and on the Registry page — and the Registry page ships as its own file, so this module is
// what keeps them one behaviour rather than two that look alike. It reads only the header's own markup and
// leaves quietly when a page does not carry a control, so a page can adopt the row a piece at a time.

const base = document.documentElement.dataset.base;
const $ = (s, el = document) => el.querySelector(s);

// Everything the header does, for whatever part of the header this page has.
export function mountChrome() {
  themeSwitch();
  if ($("#account")) account();
}

// Dark, Light, Immersive. Dark for first visits; the choice is kept on this device.
function themeSwitch() {
  const KEY = "hologram-models-hub.theme";
  const root = document.documentElement, button = $("#theme-button"), menu = $("#theme-menu");
  if (!button) return;
  const walls = JSON.parse($("#wallpapers").textContent);
  const read = () => { try { return JSON.parse(localStorage.getItem(KEY)) || {}; } catch { return {}; } };
  let warmed = false;

  function sync() {
    const mode = root.dataset.theme, wall = root.dataset.wallpaper;
    for (const b of menu.querySelectorAll("[data-theme-mode]")) b.setAttribute("aria-checked", String(b.dataset.themeMode === mode));
    for (const b of menu.querySelectorAll("button.wall")) b.setAttribute("aria-checked", String(mode === "immersive" && b.dataset.wallpaper === wall));
    $("#walls").classList.toggle("on", mode === "immersive");
  }

  function apply(mode, wallpaper = root.dataset.wallpaper) {
    const run = () => {
      root.dataset.theme = mode;
      root.dataset.wallpaper = wallpaper;
      root.classList.toggle("dark", mode !== "light");
      try { localStorage.setItem(KEY, JSON.stringify({ mode, wallpaper })); } catch {}
      sync();
    };
    const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (document.startViewTransition && !calm && !document.hidden) {
      const t = document.startViewTransition(run);
      for (const p of [t.ready, t.finished, t.updateCallbackDone]) p?.catch(() => {});
    } else run();
  }

  // Full size wallpapers load the moment the menu opens, so Immersive appears instantly.
  function warm() {
    if (warmed) return;
    warmed = true;
    for (const w of walls) { const img = new Image(); img.decoding = "async"; img.src = `${base}wallpapers/${w.key}.jpg`; }
  }

  const items = () => [...menu.querySelectorAll('[role="menuitemradio"]')];
  function open(show, focusFirst) {
    menu.hidden = !show;
    button.setAttribute("aria-expanded", String(show));
    if (show) { warm(); sync(); if (focusFirst) (menu.querySelector('[aria-checked="true"]') || items()[0]).focus(); }
  }

  button.addEventListener("click", (e) => { e.stopPropagation(); open(menu.hidden, e.detail === 0); });
  button.addEventListener("pointerenter", warm, { once: true });
  menu.addEventListener("click", (e) => {
    const mode = e.target.closest("button[data-theme-mode]"), wall = e.target.closest("button.wall");
    if (mode) apply(mode.dataset.themeMode);
    else if (wall) apply("immersive", wall.dataset.wallpaper);
  });
  menu.addEventListener("keydown", (e) => {
    const list = items(), i = list.indexOf(document.activeElement);
    const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[e.key];
    if (step) { e.preventDefault(); list[(i + step + list.length) % list.length].focus(); }
    if (e.key === "Escape") { open(false); button.focus(); }
  });
  document.addEventListener("click", (e) => { if (!menu.hidden && !e.target.closest(".appearance")) open(false); });
  window.addEventListener("storage", (e) => { if (e.key === KEY) { const s = read(); if (s.mode) apply(s.mode, s.wallpaper || "alps"); } });
  sync();
}

// ---- sign-in
//
// The whole of it lives in auth.js, and auth.js is only fetched when someone reaches for the door: a pointer over
// the button, a keyboard focus, a click, or a returning visit by someone who was signed in here before. An
// anonymous visit downloads none of it.
function account() {
  const box = $("#account"), signIn = $("#sign-in-button"), mark = $("#account-button"), menu = $("#account-menu");
  let mod = null, modReady = null;
  // The module handle is kept the moment it resolves: paint() runs from its own change events and must not wait.
  const load = () => (mod ??= import(`${base}auth.js`).then((m) => (modReady = m)));

  const root = document.documentElement;
  function paint(user) {
    // The same switch the pre-paint threw, now that the session is known for certain: signing in or out in this
    // tab moves the control at once, and a hint that turns out to be stale gives the Sign in control back.
    if (user) root.setAttribute("data-account", "in");
    else root.removeAttribute("data-account");
    if (!user) { root.style.removeProperty("--hh-account-initial"); menu.hidden = true; menu.replaceChildren(); return; }
    const m = mod && modReady;
    // A picture when the provider gave one, the brand mark with their initial when it did not. The photo is
    // dropped if it fails to load, so a broken image never stands where a person's mark should be.
    const initial = m ? m.initialOf(user) : "";
    root.style.setProperty("--hh-account-initial", JSON.stringify(initial));
    mark.classList.toggle("has-photo", Boolean(user.photo));
    let img = mark.querySelector("img");
    if (user.photo) {
      if (!img) { img = new Image(); img.alt = ""; img.width = 32; img.height = 32; img.decoding = "async"; mark.prepend(img); }
      img.onerror = () => { img.remove(); mark.classList.remove("has-photo"); };
      if (img.src !== user.photo) img.src = user.photo;
    } else if (img) img.remove();
    mark.title = user.email || "Your account";
    if (m) menu.innerHTML = m.accountMenu(user);
    $("#sign-out")?.addEventListener("click", async () => { open(false); (await load()).signOut(); });
  }
  // No paint(null) here on purpose: the markup and the pre-paint already say which control belongs on screen, and
  // calling it would throw a returning person back to "Sign in" until Privy answered — the very flash this avoids.

  const open = (show) => { menu.hidden = !show; mark.setAttribute("aria-expanded", String(show)); };
  mark.addEventListener("click", (e) => { e.stopPropagation(); open(menu.hidden); });
  menu.addEventListener("keydown", (e) => { if (e.key === "Escape") { open(false); mark.focus(); } });
  document.addEventListener("click", (e) => { if (!menu.hidden && !e.target.closest("#account")) open(false); });

  // Intent, not load: reaching for the button is enough to have the sign-in ready by the time it is pressed. A
  // phone has no hover, so the press itself is the first signal there — pointerdown still lands before the click.
  const warm = () => load().then((m) => m.warm());
  for (const signal of ["pointerenter", "pointerdown", "focus"]) signIn.addEventListener(signal, warm, { once: true });
  signIn.addEventListener("click", async () => (await load()).open());

  // Two reasons to load it without being asked: this is the page a provider sends people back to, or this browser
  // was signed in here before. The hint is read straight from storage, not from auth.js, so that asking the
  // question costs an anonymous visitor nothing.
  const landing = $("#auth-landing");
  const returning = (() => { try { return Boolean(localStorage.getItem("hologram-models-hub.account")); } catch { return false; } })();
  if (landing || returning) {
    load().then(async (m) => {
      // Subscribe only once the session is settled. onChange fires immediately with whatever it holds, which is
      // nobody until Privy has answered — subscribing first would paint the signed-out control over the one the
      // pre-paint correctly put there, which is the flash all of this exists to remove.
      let user = null;
      try {
        user = await m.restore();
      } catch {
        // The SDK could not be reached. The hint says this browser was signed in, and nothing has disproved it,
        // so leave the mark where it is rather than flipping to "Sign in" over a network failure.
        if (landing) landing.textContent = "Something went wrong signing you in. Try again from any page.";
        return;
      }
      m.onChange(paint);
      if (!landing) return;
      if (user) location.replace(base);
      else landing.textContent = "That sign-in link did not work. Try again from any page.";
    }).catch(() => {});
  }
}
