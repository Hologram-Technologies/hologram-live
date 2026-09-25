// gethologram.ai — the account service.
//
// The hub is anonymous by design: every read surface (the HF dialect, the Ollama and OCI dialects, /mcp, /v2 reads,
// the archive) works with no account and no token, and nothing here changes that. This service only serves people who
// chose to sign in, and only for things that did not exist before: a saved list, an attributed request for a model,
// and a recorded request for publish access.
//
//   node hub-account.mjs
//
// PRIVY_APP_ID                 the app id the access token must be addressed to (aud). Public.
// PRIVY_VERIFICATION_KEY_FILE  a file holding the app's ES256 public key in PEM (Dashboard → Configuration → App
//                              settings). Preferred: a PEM is several lines, and a multi-line value in an env file
//                              is a quoting accident waiting to happen.
// PRIVY_VERIFICATION_KEY       the same key inline, its newlines written as \n. The file wins when both are set.
// HUB_ACCOUNT_STATE            where per-user records live. Default /state.
//
// No npm dependencies, like hub-resolve.mjs next door: node:http, node:crypto, node:fs.

import http from "node:http";
import { createHash, createPublicKey, randomUUID, verify as verifySignature } from "node:crypto";
import { readFileSync } from "node:fs";
import { mkdir, readFile, writeFile, rename } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const APP_ID = process.env.PRIVY_APP_ID || "";
const PORT = Number(process.env.PORT || 8091);
const SKEW = 60;              // seconds of clock tolerance on exp/iat
const MAX_BODY = 8 * 1024;    // a saved-model id or a one-line note; nothing here is a document
const MAX_SAVED = 500;
const MAX_REQUESTS = 100;

// ---- the token
//
// A Privy access token is a JWT signed with ES256. Two details decide whether this is real verification or theatre:
// the algorithm is read from OUR list and never from the token's own header, and the signature is raw r‖s as JOSE
// specifies, which Node only accepts when told — its default is DER, and every valid token fails without this.
const b64url = (s) => Buffer.from(s.replace(/-/g, "+").replace(/_/g, "/"), "base64");

export function verifyAccessToken(token, { appId = APP_ID, publicKey, now = Date.now() / 1000 } = {}) {
  if (typeof token !== "string") return { ok: false, reason: "no token" };
  const parts = token.split(".");
  if (parts.length !== 3) return { ok: false, reason: "not a JWT" };
  const [h, p, s] = parts;

  let header, claims;
  try {
    header = JSON.parse(b64url(h).toString("utf8"));
    claims = JSON.parse(b64url(p).toString("utf8"));
  } catch {
    return { ok: false, reason: "malformed" };
  }
  // The token does not get to choose how it is checked.
  if (header.alg !== "ES256") return { ok: false, reason: `alg ${header.alg} refused` };
  if (header.typ && header.typ !== "JWT") return { ok: false, reason: "typ refused" };

  const signature = b64url(s);
  if (signature.length !== 64) return { ok: false, reason: "signature is not raw r‖s" };
  const signed = Buffer.from(`${h}.${p}`, "utf8");
  // dsaEncoding: JOSE signatures are the raw 64-byte pair; Node's default is DER.
  const good = verifySignature("sha256", signed, { key: publicKey, dsaEncoding: "ieee-p1363" }, signature);
  if (!good) return { ok: false, reason: "bad signature" };

  if (claims.iss !== "privy.io") return { ok: false, reason: `issuer ${claims.iss} refused` };
  if (!appId || claims.aud !== appId) return { ok: false, reason: "audience refused" };
  if (typeof claims.exp !== "number" || claims.exp + SKEW < now) return { ok: false, reason: "expired" };
  if (typeof claims.iat === "number" && claims.iat - SKEW > now) return { ok: false, reason: "issued in the future" };
  if (typeof claims.sub !== "string" || !claims.sub) return { ok: false, reason: "no subject" };

  return { ok: true, did: claims.sub, sessionId: claims.sid || null, expires: claims.exp };
}

// ---- per-user records
//
// Keyed by the hash of the Privy DID, so a directory listing names nobody. No email, no token, no wallet key ever
// reaches this disk: the wallet address is written only because the signed-in page shows it back to its owner.
const state = () => process.env.HUB_ACCOUNT_STATE || "/state";
const keyOf = (did) => createHash("sha256").update(did).digest("hex");
const fileOf = (did) => join(state(), "users", `${keyOf(did)}.json`);
const blank = (did) => ({ did, created: new Date().toISOString(), wallet: null, saved: [], requests: [] });

async function load(did) {
  try {
    return JSON.parse(await readFile(fileOf(did), "utf8"));
  } catch {
    return blank(did);
  }
}

// Written to a neighbour and renamed: a reader never sees half a record, and a crash never leaves one.
async function save(record) {
  const path = fileOf(record.did);
  const tmp = `${path}.${randomUUID()}`;
  await mkdir(join(state(), "users"), { recursive: true });
  await writeFile(tmp, JSON.stringify(record));
  await rename(tmp, path);
  return record;
}

async function append(name, line) {
  await mkdir(state(), { recursive: true });
  const { appendFile } = await import("node:fs/promises");
  await appendFile(join(state(), name), `${line}\n`);
}

// ---- http
const json = (res, status, body, headers = {}) => {
  const text = JSON.stringify(body);
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(text),
    // A person's own record is never cached or shared: no store, and no Access-Control-Allow-Origin anywhere in
    // this file. The read surfaces of the hub are open to every origin; this one is not.
    "cache-control": "no-store",
    "referrer-policy": "no-referrer",
    "x-content-type-options": "nosniff",
    ...headers,
  });
  res.end(res.req.method === "HEAD" ? undefined : text);
};
const refuse = (res, status, message) => json(res, status, { error: message });

async function body(req) {
  const chunks = [];
  let size = 0;
  for await (const chunk of req) {
    size += chunk.length;
    if (size > MAX_BODY) throw new Error("too large");
    chunks.push(chunk);
  }
  if (!size) return {};
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

// One signed-in person cannot outpace a person: enough for a real session, not enough to be a write endpoint.
const seen = new Map();
function withinRate(key, limit = 60, windowMs = 60_000) {
  const now = Date.now();
  const hits = (seen.get(key) || []).filter((t) => now - t < windowMs);
  hits.push(now);
  seen.set(key, hits);
  if (seen.size > 10_000) for (const [k, v] of seen) if (!v.some((t) => now - t < windowMs)) seen.delete(k);
  return hits.length <= limit;
}

const MODEL = /^[A-Za-z0-9][\w.-]{0,95}\/[A-Za-z0-9][\w.-]{0,95}$/;

// ---- Apps: likes and comments
//
// The one place on the hub where one person's words are shown to another, so the rules are few and hard. Reading is
// anonymous, like every other read on this host. Writing needs a verified sign-in. An author is shown as a handle
// derived from the hash of their Privy DID — stable, and naming no one: no email, no wallet, nothing Privy knows
// ever reaches another reader. Each App's thread is one file, written through a per-App queue and a rename, so two
// comments arriving together cannot overwrite each other and a reader never sees half a file.
const APP = /^[a-z0-9][a-z0-9-]{0,63}$/;
const MAX_COMMENT = 2000;
const MAX_COMMENTS = 5000;          // per App; past it the thread is read-only until an operator archives it
const handleOf = (key) => `user-${key.slice(0, 6)}`;
const appFile = (id) => join(state(), "apps", `${id}.json`);
const blankApp = (id) => ({ id, reactions: {}, comments: [] });

async function loadApp(id) {
  try { return JSON.parse(await readFile(appFile(id), "utf8")); } catch { return blankApp(id); }
}
const queues = new Map();
function withApp(id, fn) {
  const run = (queues.get(id) || Promise.resolve()).then(async () => {
    const record = await loadApp(id);
    const out = await fn(record);
    if (out?.write !== false) {
      await mkdir(join(state(), "apps"), { recursive: true });
      const tmp = `${appFile(id)}.${randomUUID()}`;
      await writeFile(tmp, JSON.stringify(record));
      await rename(tmp, appFile(id));
    }
    return out?.value;
  });
  queues.set(id, run.catch(() => {}));
  return run;
}

const tally = (reactions = {}) => {
  let likes = 0, dislikes = 0;
  for (const v of Object.values(reactions)) if (v > 0) likes++; else if (v < 0) dislikes++;
  return { likes, dislikes };
};
// What a reader is shown of a comment. The author's key stays on disk; the reader gets the handle, and `own`
// when the reader is its author, so the page can offer Delete to that person and no one else.
function present(c, me) {
  return {
    id: c.id, parent: c.parent || null, handle: c.handle, text: c.text, at: c.at,
    likes: tally(c.reactions).likes,
    mine: me ? { react: c.reactions?.[me] || 0, own: c.author === me } : null,
  };
}
function thread(record, me, sort) {
  const top = record.comments.filter((c) => !c.parent);
  const replies = new Map();
  for (const c of record.comments) if (c.parent) { if (!replies.has(c.parent)) replies.set(c.parent, []); replies.get(c.parent).push(c); }
  const byNew = (a, b) => b.at.localeCompare(a.at);
  const byTop = (a, b) => tally(b.reactions).likes - tally(a.reactions).likes || byNew(a, b);
  return top.sort(sort === "new" ? byNew : byTop).map((c) => ({
    ...present(c, me),
    replies: (replies.get(c.id) || []).sort((a, b) => a.at.localeCompare(b.at)).map((r) => present(r, me)),
  }));
}
const reaction = (v) => (v === 1 || v === -1 || v === 0 ? v : null);

async function apps(req, res, parts, url, who) {
  const [id, section, cid, action] = parts;
  if (!APP.test(id || "")) return refuse(res, 400, "not an app id");
  const me = who ? keyOf(who) : null;
  const read = req.method === "GET" || req.method === "HEAD";

  // GET /apps/<id>: the counts under the frame, and this reader's own like when signed in
  if (!section && read) {
    const record = await loadApp(id);
    return json(res, 200, { id, ...tally(record.reactions), comments: record.comments.length, mine: me ? { react: record.reactions[me] || 0 } : null });
  }
  // GET /apps/<id>/comments?sort=top|new
  if (section === "comments" && !cid && read) {
    const record = await loadApp(id);
    const sort = url.searchParams.get("sort") === "new" ? "new" : "top";
    return json(res, 200, { id, count: record.comments.length, sort, comments: thread(record, me, sort) });
  }

  if (!me) return refuse(res, 401, "sign in to use this");
  if (!withinRate(`w:${me}`, 20)) return refuse(res, 429, "too many requests, slow down");

  // POST /apps/<id>/react {value: 1 | -1 | 0}
  if (section === "react" && !cid && req.method === "POST") {
    const value = reaction((await body(req)).value);
    if (value === null) return refuse(res, 400, "value is 1, -1 or 0");
    const out = await withApp(id, (r) => { if (value) r.reactions[me] = value; else delete r.reactions[me]; return { value: { ...tally(r.reactions), mine: { react: value } } }; });
    return json(res, 200, out);
  }
  // POST /apps/<id>/comments {text, parent?}
  if (section === "comments" && !cid && req.method === "POST") {
    const { text, parent } = await body(req);
    const clean = String(text || "").replace(/\r\n?/g, "\n").replace(/\n{3,}/g, "\n\n").trim();
    if (!clean) return refuse(res, 400, "a comment needs some words");
    if (clean.length > MAX_COMMENT) return refuse(res, 400, `a comment is at most ${MAX_COMMENT} characters`);
    const out = await withApp(id, (r) => {
      if (r.comments.length >= MAX_COMMENTS) return { write: false, value: { error: "this thread is full" } };
      const target = parent ? r.comments.find((c) => c.id === parent) : null;
      if (parent && !target) return { write: false, value: { error: "that comment is gone" } };
      // A reply to a reply joins the same thread, one level deep, as YouTube does.
      const c = { id: randomUUID(), parent: target ? (target.parent || target.id) : null, author: me, handle: handleOf(me), text: clean, at: new Date().toISOString(), reactions: {} };
      r.comments.push(c);
      return { value: present(c, me) };
    });
    return out.error ? refuse(res, 409, out.error) : json(res, 201, out);
  }
  // POST /apps/<id>/comments/<cid>/react {value}
  if (section === "comments" && cid && action === "react" && req.method === "POST") {
    const value = reaction((await body(req)).value);
    if (value === null) return refuse(res, 400, "value is 1, -1 or 0");
    const out = await withApp(id, (r) => {
      const c = r.comments.find((x) => x.id === cid);
      if (!c) return { write: false, value: null };
      c.reactions ||= {};
      if (value) c.reactions[me] = value; else delete c.reactions[me];
      return { value: present(c, me) };
    });
    return out ? json(res, 200, out) : refuse(res, 404, "that comment is gone");
  }
  // DELETE /apps/<id>/comments/<cid>: its author only; its replies go with it
  if (section === "comments" && cid && !action && req.method === "DELETE") {
    const out = await withApp(id, (r) => {
      const c = r.comments.find((x) => x.id === cid);
      if (!c) return { write: false, value: 404 };
      if (c.author !== me) return { write: false, value: 403 };
      r.comments = r.comments.filter((x) => x.id !== cid && x.parent !== cid);
      return { value: 200 };
    });
    return out === 200 ? json(res, 200, { deleted: cid }) : refuse(res, out, out === 403 ? "only its author can delete a comment" : "that comment is gone");
  }
  return refuse(res, 404, "no such apps route");
}

export function createApp({ publicKey, appId = APP_ID }) {
  return async (req, res) => {
    const url = new URL(req.url, "http://hub-account");
    const path = url.pathname.replace(/^\/api\/account/, "") || "/";

    if (path === "/health") return json(res, 200, { ok: true, configured: Boolean(publicKey && appId) });

    // Apps' likes and comments read anonymously; a valid token, when one is sent, only adds "mine".
    if (path.startsWith("/apps/")) {
      const auth = req.headers.authorization || "";
      const token = auth.startsWith("Bearer ") ? auth.slice(7).trim() : "";
      const who = token && publicKey && appId ? verifyAccessToken(token, { appId, publicKey }) : null;
      if (token && !who?.ok && !(req.method === "GET" || req.method === "HEAD")) return refuse(res, 401, "sign in to use this");
      // Behind Caddy every request arrives from Caddy, so the reader is the first X-Forwarded-For entry Caddy wrote.
      const client = String(req.headers["x-forwarded-for"] || "").split(",")[0].trim() || req.socket?.remoteAddress || "?";
      if (!withinRate(`ip:${client}`, 240)) return refuse(res, 429, "too many requests, slow down");
      try {
        return await apps(req, res, path.slice("/apps/".length).split("/").map(decodeURIComponent), url, who?.ok ? who.did : null);
      } catch (err) {
        if (err instanceof SyntaxError || err.message === "too large") return refuse(res, 400, "bad request body");
        console.error(`apps error ${req.method} ${path}: ${err.message}`);
        return refuse(res, 500, "something went wrong");
      }
    }

    if (!publicKey || !appId) return refuse(res, 503, "sign-in is not configured on this hub");

    const auth = req.headers.authorization || "";
    const token = auth.startsWith("Bearer ") ? auth.slice(7).trim() : "";
    const check = verifyAccessToken(token, { appId, publicKey });
    if (!check.ok) return refuse(res, 401, "sign in to use this");
    const { did } = check;

    if (!withinRate(keyOf(did))) return refuse(res, 429, "too many requests, slow down");

    try {
      const record = await load(did);

      if (path === "/me" && (req.method === "GET" || req.method === "HEAD")) {
        return json(res, 200, { did, created: record.created, wallet: record.wallet, saved: record.saved, requests: record.requests.length });
      }

      // The wallet address is Privy's to mint and the page's to report. Recorded, never trusted for anything.
      if (path === "/me" && req.method === "PATCH") {
        const { wallet } = await body(req);
        if (wallet !== null && !/^0x[0-9a-fA-F]{40}$/.test(String(wallet || ""))) return refuse(res, 400, "not an address");
        record.wallet = wallet === null ? null : String(wallet);
        await save(record);
        return json(res, 200, { wallet: record.wallet });
      }

      if (path === "/saved" && req.method === "POST") {
        const { model } = await body(req);
        if (!MODEL.test(String(model || ""))) return refuse(res, 400, "not a model id");
        if (record.saved.length >= MAX_SAVED) return refuse(res, 409, "saved list is full");
        if (!record.saved.includes(model)) record.saved.unshift(model);
        await save(record);
        return json(res, 200, { saved: record.saved });
      }

      if (path.startsWith("/saved/") && req.method === "DELETE") {
        const model = decodeURIComponent(path.slice("/saved/".length));
        record.saved = record.saved.filter((m) => m !== model);
        await save(record);
        return json(res, 200, { saved: record.saved });
      }

      // The anonymous version of this already exists (hub-resolve writes requested.txt for any model it is asked for
      // and does not know). Signing in only adds attribution, so the operator can tell one person asking ten times
      // from ten people asking once.
      if (path === "/request" && req.method === "POST") {
        const { model } = await body(req);
        if (!MODEL.test(String(model || ""))) return refuse(res, 400, "not a model id");
        if (record.requests.length >= MAX_REQUESTS) return refuse(res, 409, "you have asked for enough for now");
        if (!record.requests.includes(model)) {
          record.requests.unshift(model);
          await save(record);
          await append("requested.jsonl", JSON.stringify({ at: new Date().toISOString(), did: keyOf(did), model }));
        }
        return json(res, 200, { requested: model });
      }

      // Records an interest in publishing. It issues nothing and grants nothing: who may write to this registry is
      // an operator's decision, and a decision is not something a web form gets to make.
      if (path === "/publisher-request" && req.method === "POST") {
        const { note } = await body(req);
        await append("publisher-requests.jsonl", JSON.stringify({ at: new Date().toISOString(), did: keyOf(did), note: String(note || "").slice(0, 500) }));
        return json(res, 202, { recorded: true, issued: false });
      }

      return refuse(res, 404, "no such account route");
    } catch (err) {
      if (err instanceof SyntaxError || err.message === "too large") return refuse(res, 400, "bad request body");
      console.error(`account error ${req.method} ${path}: ${err.message}`);
      return refuse(res, 500, "something went wrong");
    }
  };
}

function publicKeyFromEnv() {
  const path = process.env.PRIVY_VERIFICATION_KEY_FILE;
  let pem = "";
  if (path) {
    try {
      pem = readFileSync(path, "utf8");
    } catch (err) {
      console.error(`PRIVY_VERIFICATION_KEY_FILE ${path} could not be read: ${err.message}`);
      return null;
    }
  } else {
    pem = process.env.PRIVY_VERIFICATION_KEY || "";
  }
  // The same key arrives in three shapes and all three are the same key, so none of them is allowed to be the
  // reason sign-in is down: pasted into an env file its newlines become \n, written on Windows it carries carriage
  // returns, and read from Privy's own app endpoint it has no line breaks at all — which openssl refuses outright
  // ("DECODER routines::unsupported"). Strip the armour, rewrap the base64 at 64 columns, put it back.
  pem = pem.replace(/\\n/g, "\n").replace(/\r/g, "").trim();
  if (!pem) return null;
  const body = pem.replace(/-----(BEGIN|END) [A-Z ]+-----/g, "").replace(/\s+/g, "");
  if (body) pem = `-----BEGIN PUBLIC KEY-----\n${(body.match(/.{1,64}/g) || []).join("\n")}\n-----END PUBLIC KEY-----\n`;
  let key;
  try {
    key = createPublicKey(pem);
  } catch (err) {
    console.error(`the Privy verification key is not usable: ${err.message}`);
    return null;
  }
  // An ES256 token is signed on P-256. Privy's own docs call the same key "Ed25519" in one breath and the token
  // "a standard ES256 JWT" in the next, so say plainly at startup which one actually arrived: the alternative is a
  // service that looks healthy and refuses every real visitor.
  const { namedCurve, type } = key.asymmetricKeyDetails || {};
  if (type !== "ec" && key.asymmetricKeyType !== "ec") {
    console.error(`the Privy verification key is ${key.asymmetricKeyType}, not an EC key: an ES256 token cannot be checked with it`);
    return null;
  }
  if (namedCurve && namedCurve !== "prime256v1") {
    console.error(`the Privy verification key is on ${namedCurve}, not P-256: an ES256 token cannot be checked with it`);
    return null;
  }
  return key;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const publicKey = publicKeyFromEnv();
  if (!publicKey || !APP_ID) console.warn("sign-in is not configured: /api/account/health answers, everything else is 503");
  else console.log(`sign-in configured for app ${APP_ID} with a P-256 verification key`);
  http.createServer(createApp({ publicKey })).listen(PORT, () => console.log(`hub-account on :${PORT}`));
}
