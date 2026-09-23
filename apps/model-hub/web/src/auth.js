// Sign-in, loaded only when someone reaches for it.
//
// The hub works without an account and always will: this module is never imported during a normal anonymous visit.
// app.js pulls it in on the first sign of intent — a pointer over the button, a focus, a click — or on load for
// someone who was already signed in last time. Everything Privy ships stays behind that door.
//
// One door, one step. There is no separate "create an account": the same email that signs an existing person in
// makes a new person's account, because a second path would only ask people to remember which one they used.

import { esc, icon } from "./render.mjs";

const base = document.documentElement.dataset.base;
const cfg = JSON.parse(document.getElementById("privy")?.textContent || "null");
const SEEN = "hologram-models-hub.account";   // a hint, never a credential: "someone was signed in here"
const LAST = "hologram-models-hub.last-login"; // the method that worked last time, offered first

const read = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
const write = (k, v) => { try { v === null ? localStorage.removeItem(k) : localStorage.setItem(k, v); } catch { /* private window */ } };

// ---- the SDK, fetched once
let sdkPromise = null;
const sdk = () => (sdkPromise ??= import(`${base}${cfg.bundle}`));
export const warm = () => { if (cfg) sdk().catch(() => {}); };

let privy = null;
async function client() {
  if (privy) return privy;
  const { Privy, LocalStorage } = await sdk();
  privy = new Privy({ appId: cfg.appId, clientId: cfg.clientId, storage: new LocalStorage() });
  await privy.initialize();
  return privy;
}

// ---- who is signed in
const listeners = new Set();
let current = null;
export const onChange = (fn) => { listeners.add(fn); fn(current); };
const announce = () => { for (const fn of listeners) fn(current); };

// A person is a list of linked accounts, and each kind names its own fields: an email account carries `address`,
// an OAuth account carries `email`, and GitHub may carry only a `username`. Taken from the SDK's own types rather
// than guessed, because guessing here shows someone "your account" instead of their name and looks broken.
const accounts = (user) => user?.linked_accounts || [];
const nameOf = (a) => (a.type === "email" ? a.address : a.email || a.username || null);
// Whoever they actually came in as, in the order the sheet offers.
const whoOf = (user) => {
  for (const type of ["email", "google_oauth", "github_oauth"]) {
    const found = accounts(user).filter((a) => a.type === type).map(nameOf).find(Boolean);
    if (found) return found;
  }
  return accounts(user).map(nameOf).find(Boolean) || null;
};

// A picture, if the provider they used actually hands one over. Privy carries `profile_picture_url` on Farcaster,
// Twitter, Line and custom OAuth accounts — and NOT on google_oauth, github_oauth or email, which are the three
// ways into this hub. So today this is always null and the mark is a letter; it is written anyway so that turning
// on such a provider later is a dashboard switch rather than a code change. Nothing is ever fetched from a third
// party to manufacture one.
const photoOf = (user) => {
  const url = accounts(user).map((a) => a.profile_picture_url || a.profile_picture).find(Boolean);
  return typeof url === "string" && url.startsWith("https://") ? url : null;
};

// The address of the Ethereum wallet Privy made for this person. Shown back to its owner, trusted for nothing —
// and the Ethereum one specifically, since that is the only shape the service will store.
const walletOf = (user) =>
  accounts(user).find((a) => a.type === "wallet" && a.connector_type === "embedded" && a.chain_type === "ethereum")?.address || null;

function adopt(user) {
  current = user ? { id: user.id, email: whoOf(user), wallet: walletOf(user), photo: photoOf(user) } : null;
  write(SEEN, user ? "1" : null);
  announce();
  if (user) record().catch(() => {});
  return current;
}

// ---- our own side
export async function api(path, options = {}) {
  const p = await client();
  const token = await p.getAccessToken();
  if (!token) throw new Error("not signed in");
  const res = await fetch(`${base}api/account${path}`, {
    ...options,
    headers: { ...(options.body ? { "content-type": "application/json" } : {}), authorization: `Bearer ${token}`, ...options.headers },
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error || `request failed (${res.status})`);
  return res.json();
}

// First sight of this person on our side, and the wallet address they can copy. Best effort: the hub is readable
// whether or not this succeeds.
const record = async () => { const me = await api("/me"); if (current && me.wallet !== current.wallet) await api("/me", { method: "PATCH", body: { wallet: current.wallet } }); };

// ---- restoring a session, and finishing an OAuth round trip
export async function restore() {
  if (!cfg) return null;
  const params = new URLSearchParams(location.search);
  const code = params.get("privy_oauth_code"), state = params.get("privy_oauth_state");
  const p = await client();
  if (code && state) {
    try {
      const { user } = await p.auth.oauth.loginWithCode(code, state);
      adopt(user);
    } catch { /* a stale or replayed link: fall through to whatever session exists */ }
    // The codes are single use and ugly. Take them out of the address bar and out of the back button.
    const url = new URL(location.href);
    for (const k of ["privy_oauth_code", "privy_oauth_state", "privy_oauth_provider"]) url.searchParams.delete(k);
    history.replaceState(history.state, "", url);
    const back = sessionStorage.getItem("hologram-models-hub.return");
    if (back) { sessionStorage.removeItem("hologram-models-hub.return"); if (back !== location.href) location.replace(back); }
    if (current) return current;
  }
  try {
    const { user } = await p.user.get();
    return adopt(user);
  } catch {
    return adopt(null);
  }
}
export const wasSignedIn = () => read(SEEN) === "1";

export async function signOut() {
  const p = await client();
  try { if (current) await p.auth.logout({ userId: current.id }); } finally { adopt(null); }
}

// ---- the door
//
// Two ways in, ordered by how little they ask of you. A provider button is one click and no typing. Email is one
// field, then six digits. Whichever you used last time is the one offered first when you come back.
const PROVIDERS = [
  ["google", "Google", '<svg class="i mark" viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M21.6 12.2c0-.7-.06-1.35-.18-2H12v3.8h5.4a4.6 4.6 0 0 1-2 3v2.5h3.24c1.9-1.75 2.96-4.33 2.96-7.3Z"/><path fill="currentColor" d="M12 22c2.7 0 4.96-.9 6.61-2.43l-3.23-2.5c-.9.6-2.05.95-3.38.95-2.6 0-4.8-1.75-5.59-4.1H3.08v2.58A10 10 0 0 0 12 22Z" opacity=".75"/><path fill="currentColor" d="M6.41 13.92a6 6 0 0 1 0-3.84V7.5H3.08a10 10 0 0 0 0 9l3.33-2.58Z" opacity=".5"/><path fill="currentColor" d="M12 5.98c1.47 0 2.79.5 3.83 1.5l2.86-2.86C16.95 2.99 14.7 2 12 2a10 10 0 0 0-8.92 5.5l3.33 2.58C7.2 7.73 9.4 5.98 12 5.98Z" opacity=".9"/></svg>'],
  ["github", "GitHub", icon.github],
];

let dialog = null;
function build() {
  if (dialog) return dialog;
  dialog = document.createElement("dialog");
  dialog.className = "auth-sheet";
  dialog.id = "sign-in";
  dialog.innerHTML = `
    <form method="dialog" class="auth-close"><button type="submit" class="control square" aria-label="Close">${icon.close}</button></form>
    <div class="auth-head">
      <h2>Sign in</h2>
      <p>New here? Signing in creates your account. No password needed.</p>
    </div>
    <div class="auth-body" id="sign-in-body">
      <div class="providers">${PROVIDERS.map(([k, label, mark]) => `<button type="button" class="provider" data-provider="${k}">${mark}<span class="label">Continue with ${label}</span></button>`).join("")}</div>
      <div class="or"><span>or</span></div>
      <form class="email-step" id="email-step" novalidate>
        <label class="field"><input type="email" id="sign-in-email" name="email" placeholder="you@example.com" autocomplete="email" required aria-label="Email address" enterkeyhint="go"></label>
        <button type="submit" class="button primary" id="email-go">Continue with email</button>
      </form>
      <form class="code-step" id="code-step" hidden novalidate>
        <p class="code-sent">Enter the six digit code sent to <b id="code-to"></b>.</p>
        <label class="field"><input type="text" id="sign-in-code" inputmode="numeric" pattern="[0-9]*" maxlength="6" autocomplete="one-time-code" placeholder="000000" aria-label="Six digit code" enterkeyhint="go"></label>
        <button type="submit" class="button primary" id="code-go">Sign in</button>
        <div class="code-foot"><button type="button" class="link" id="code-again">Send a new code</button><button type="button" class="link" id="code-back">Change email</button></div>
      </form>
      <p class="auth-error" id="sign-in-error" role="alert" hidden></p>
    </div>
    <p class="auth-foot">No account needed to browse, download or verify.</p>`;
  document.body.append(dialog);
  wire();
  return dialog;
}

const $ = (s) => dialog.querySelector(s);
let busy = false;
function fail(err) {
  const box = $("#sign-in-error");
  box.textContent = err ? (err.message || String(err)) : "";
  box.hidden = !err;
}
function working(on, button, label) {
  busy = on;
  for (const b of dialog.querySelectorAll("button")) b.disabled = on;
  if (button) {
    const slot = button.querySelector(".label") || button;
    slot.dataset.label ??= slot.textContent;
    slot.textContent = on ? label : slot.dataset.label;
  }
}

// Nothing here may leave the sheet disabled with a spinner and no way out. If the network or the provider is not
// answering, say so and give the buttons back.
const SLOW = 20_000;
const guard = (work) => Promise.race([
  work,
  new Promise((_, reject) => setTimeout(() => reject(new Error("Sign-in is not answering. Check your connection and try again.")), SLOW)),
]);

function wire() {
  // Offer the way in that worked last time, first.
  const last = read(LAST);
  if (last) {
    const preferred = dialog.querySelector(`[data-provider="${last}"]`);
    if (preferred) { preferred.classList.add("preferred"); preferred.parentElement.prepend(preferred); }
    if (last === "email") $("#sign-in-email").autofocus = true;
  }
  for (const b of dialog.querySelectorAll("[data-provider]")) {
    b.addEventListener("click", async () => {
      fail(null);
      write(LAST, b.dataset.provider);
      try {
        working(true, b, "Opening…");
        const p = await guard(client());
        sessionStorage.setItem("hologram-models-hub.return", location.href);
        // 0.76.2 answers {url}; the published example assigns the result directly, which navigates to
        // "[object Object]". Take either shape, and refuse anything that is not an address we can open.
        const answer = await guard(p.auth.oauth.generateURL(b.dataset.provider, `${location.origin}${base}auth/`));
        const url = typeof answer === "string" ? answer : answer?.url;
        if (!/^https:\/\//.test(url || "")) throw new Error("That sign-in could not be started. Try another way in.");
        location.assign(url);
      } catch (err) { working(false, b); fail(err); }
    });
  }

  const emailStep = $("#email-step"), codeStep = $("#code-step"), emailInput = $("#sign-in-email"), codeInput = $("#sign-in-code");
  const go = $("#email-go"), codeGo = $("#code-go");
  let address = "";

  const send = async () => {
    fail(null);
    address = emailInput.value.trim();
    if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(address)) return fail(new Error("That does not look like an email address."));
    try {
      working(true, go, "Sending…");
      const p = await guard(client());
      await guard(p.auth.email.sendCode(address));
      write(LAST, "email");
      working(false, go);
      $("#code-to").textContent = address;
      emailStep.hidden = true;
      codeStep.hidden = false;
      dialog.classList.add("on-code");
      codeInput.value = "";
      codeInput.focus();
    } catch (err) { working(false, go); fail(err); }
  };

  emailStep.addEventListener("submit", (e) => { e.preventDefault(); if (!busy) send(); });
  $("#code-again").addEventListener("click", () => { if (!busy) send(); });
  $("#code-back").addEventListener("click", () => { codeStep.hidden = true; emailStep.hidden = false; dialog.classList.remove("on-code"); fail(null); emailInput.focus(); });

  const submitCode = async () => {
    const otp = codeInput.value.replace(/\D/g, "");
    if (otp.length !== 6) return;
    fail(null);
    try {
      working(true, codeGo, "Signing in…");
      const p = await guard(client());
      const { user } = await guard(p.auth.email.loginWithCode(address, otp));
      adopt(user);
      working(false, codeGo);
      close();
    } catch (err) { working(false, codeGo); codeInput.select(); fail(new Error("That code did not work. Check it, or send another.")); }
  };
  codeStep.addEventListener("submit", (e) => { e.preventDefault(); if (!busy) submitCode(); });
  // Six digits in, and it goes. Nobody should have to find the button.
  codeInput.addEventListener("input", () => {
    codeInput.value = codeInput.value.replace(/\D/g, "").slice(0, 6);
    if (codeInput.value.length === 6 && !busy) submitCode();
  });

  dialog.addEventListener("close", () => { fail(null); codeStep.hidden = true; emailStep.hidden = false; dialog.classList.remove("on-code"); });
  // A click on the backdrop closes it, the way every sheet on this site does.
  dialog.addEventListener("click", (e) => { if (e.target === dialog) close(); });
}

export function open() {
  build();
  if (!dialog.open) dialog.showModal();
  const first = dialog.querySelector(".preferred, [data-provider], #sign-in-email");
  (read(LAST) === "email" ? $("#sign-in-email") : first)?.focus();
}
export const close = () => dialog?.close();

// ---- the signed-in menu
//
// Lives here rather than in app.js so that what a signed-in person sees is written next to what signing in means,
// and so it can be rendered against any person without one existing.
export const initialOf = (user) => (user?.email || "?").trim().charAt(0).toUpperCase() || "?";
export const shortAddress = (a) => `${a.slice(0, 6)}…${a.slice(-4)}`;

export const accountMenu = (user) => `
  <div class="account-who">
    <span class="label">Signed in</span>
    <b title="${esc(user.email || "")}">${esc(user.email || "your account")}</b>
  </div>
  <div class="account-rows">
    ${user.wallet ? `<button type="button" role="menuitem" class="account-row copy" data-copy="${esc(user.wallet)}" title="Copy your wallet address">
      ${icon.seal}<span class="label">Wallet<span class="sub">${esc(shortAddress(user.wallet))}</span></span>${icon.copy}
    </button>` : `<div class="account-row quiet">${icon.seal}<span class="label">Wallet<span class="sub">Not set up yet</span></span></div>`}
    <button type="button" role="menuitem" class="account-row" id="sign-out">${icon.reset}<span class="label">Sign out</span></button>
  </div>`;
