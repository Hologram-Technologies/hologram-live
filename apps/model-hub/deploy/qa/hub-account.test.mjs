// Proof that hub-account only accepts a token Privy actually signed for this app.
//
//   node --test deploy/hub/qa/hub-account.test.mjs
//
// Every case mints a real ES256 token with a locally generated P-256 key, so a passing run means the verifier
// agrees with a real signer, not with a fixture someone wrote to match it.

import test from "node:test";
import assert from "node:assert/strict";
import { generateKeyPairSync, sign as signBytes, createHmac } from "node:crypto";
import { join } from "node:path";
import { verifyAccessToken, createApp } from "../hub-account.mjs";

const { publicKey, privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
const other = generateKeyPairSync("ec", { namedCurve: "P-256" });
const APP_ID = "test-app-id";
const NOW = 1_800_000_000;

const b64 = (buf) => Buffer.from(buf).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const part = (obj) => b64(JSON.stringify(obj));

function mint({ header = {}, claims = {}, key = privateKey, encoding = "ieee-p1363" } = {}) {
  const h = part({ alg: "ES256", typ: "JWT", ...header });
  const p = part({ iss: "privy.io", aud: APP_ID, sub: "did:privy:abc123", iat: NOW - 10, exp: NOW + 3600, ...claims });
  const sig = signBytes("sha256", Buffer.from(`${h}.${p}`), { key, dsaEncoding: encoding });
  return `${h}.${p}.${b64(sig)}`;
}
const check = (token) => verifyAccessToken(token, { appId: APP_ID, publicKey, now: NOW });

test("a token this app signed is accepted", () => {
  const r = check(mint());
  assert.equal(r.ok, true);
  assert.equal(r.did, "did:privy:abc123");
});

test("a DER-encoded signature is refused", () => {
  // The trap this verifier exists to survive: Node signs DER by default, JOSE is raw r‖s. If the verifier were
  // written without dsaEncoding it would accept this and reject every real Privy token.
  assert.equal(check(mint({ encoding: "der" })).ok, false);
});

test("an expired token is refused", () => {
  assert.equal(check(mint({ claims: { exp: NOW - 120 } })).ok, false);
});

test("a token still inside the clock-skew allowance is accepted", () => {
  assert.equal(check(mint({ claims: { exp: NOW - 30 } })).ok, true);
});

test("a token for another app is refused", () => {
  assert.equal(check(mint({ claims: { aud: "someone-elses-app" } })).ok, false);
});

test("a token from another issuer is refused", () => {
  assert.equal(check(mint({ claims: { iss: "evil.example" } })).ok, false);
});

test("a token signed by a different key is refused", () => {
  assert.equal(check(mint({ key: other.privateKey })).ok, false);
});

test("alg none is refused", () => {
  const h = part({ alg: "none", typ: "JWT" });
  const p = part({ iss: "privy.io", aud: APP_ID, sub: "did:privy:abc123", exp: NOW + 3600 });
  assert.equal(check(`${h}.${p}.`).ok, false);
});

test("an HS256 token signed with the public key as the secret is refused", () => {
  // Algorithm confusion: the public key is public, so if the verifier trusted the header's alg this would pass.
  const h = part({ alg: "HS256", typ: "JWT" });
  const p = part({ iss: "privy.io", aud: APP_ID, sub: "did:privy:abc123", exp: NOW + 3600 });
  const pem = publicKey.export({ type: "spki", format: "pem" });
  const sig = createHmac("sha256", pem).update(`${h}.${p}`).digest();
  assert.equal(check(`${h}.${p}.${b64(sig)}`).ok, false);
});

test("a tampered payload is refused", () => {
  const [h, , s] = mint().split(".");
  const p = part({ iss: "privy.io", aud: APP_ID, sub: "did:privy:someone-else", exp: NOW + 3600 });
  assert.equal(check(`${h}.${p}.${s}`).ok, false);
});

test("a token with no subject is refused", () => {
  assert.equal(check(mint({ claims: { sub: "" } })).ok, false);
});

test("a token issued in the future is refused", () => {
  assert.equal(check(mint({ claims: { iat: NOW + 600 } })).ok, false);
});

test("junk is refused without throwing", () => {
  for (const junk of ["", "x", "a.b", "a.b.c", "....", null, undefined, 42, "a.b.c.d"]) {
    assert.equal(check(junk).ok, false);
  }
});

// ---- the service
async function call(app, { method = "GET", path = "/api/account/me", token, body } = {}) {
  const chunks = body === undefined ? [] : [Buffer.from(JSON.stringify(body))];
  const req = Object.assign(
    (async function* () { yield* chunks; })(),
    { method, url: path, headers: token ? { authorization: `Bearer ${token}` } : {} },
  );
  return new Promise((resolve) => {
    const res = {
      req,
      writeHead(status, headers) { this.status = status; this.headers = headers; },
      end(text) { resolve({ status: this.status, headers: this.headers, body: text ? JSON.parse(text) : null }); },
    };
    req.res = res;
    app(req, res);
  });
}

test("health answers without a token and says whether sign-in is configured", async () => {
  const off = await call(createApp({ publicKey: null, appId: "" }), { path: "/api/account/health" });
  assert.equal(off.status, 200);
  assert.equal(off.body.configured, false);
  const on = await call(createApp({ publicKey, appId: APP_ID }), { path: "/api/account/health" });
  assert.equal(on.body.configured, true);
});

test("every other route refuses an anonymous caller", async () => {
  const app = createApp({ publicKey, appId: APP_ID });
  for (const path of ["/api/account/me", "/api/account/saved", "/api/account/request", "/api/account/publisher-request"]) {
    const r = await call(app, { path, method: "POST" });
    assert.equal(r.status, 401, path);
  }
});

test("no account response carries a CORS header", async () => {
  const app = createApp({ publicKey, appId: APP_ID });
  for (const r of [await call(app, { path: "/api/account/health" }), await call(app, { token: mint() })]) {
    const names = Object.keys(r.headers).map((h) => h.toLowerCase());
    assert.ok(!names.some((h) => h.startsWith("access-control-")), names.join(","));
    assert.equal(r.headers["cache-control"], "no-store");
  }
});

// A token the service will accept against the real clock, since the service does not take a `now`.
const live = () => {
  const t = Math.floor(Date.now() / 1000);
  return mint({ claims: { iat: t - 10, exp: t + 3600 } });
};

test("a saved model round-trips and a request is recorded, issuing nothing", async () => {
  const { mkdtemp, readFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const dir = await mkdtemp(join(tmpdir(), "hub-account-"));
  process.env.HUB_ACCOUNT_STATE = dir;
  const app = createApp({ publicKey, appId: APP_ID });
  const token = live();

  assert.equal((await call(app, { method: "POST", path: "/api/account/saved", token, body: { model: "hexgrad/Kokoro-82M" } })).status, 200);
  assert.deepEqual((await call(app, { token })).body.saved, ["hexgrad/Kokoro-82M"]);
  assert.equal((await call(app, { method: "POST", path: "/api/account/saved", token, body: { model: "not a model id" } })).status, 400);
  assert.deepEqual((await call(app, { method: "DELETE", path: "/api/account/saved/hexgrad%2FKokoro-82M", token })).body.saved, []);

  const pub = await call(app, { method: "POST", path: "/api/account/publisher-request", token, body: { note: "I maintain two models" } });
  assert.equal(pub.status, 202);
  assert.equal(pub.body.issued, false);

  // What lands on disk names a hash, never a person.
  const line = JSON.parse(await readFile(join(dir, "publisher-requests.jsonl"), "utf8"));
  assert.match(line.did, /^[0-9a-f]{64}$/);
  assert.ok(!JSON.stringify(line).includes("did:privy:"));
});

// ---- Apps: likes and comments
test("an App's likes and comments read anonymously, and writing needs a sign-in", async () => {
  const { mkdtemp, readFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const dir = await mkdtemp(join(tmpdir(), "hub-apps-"));
  process.env.HUB_ACCOUNT_STATE = dir;
  const app = createApp({ publicKey, appId: APP_ID });
  const alice = live();
  const t = Math.floor(Date.now() / 1000);
  const bob = mint({ claims: { sub: "did:privy:bob", iat: t - 10, exp: t + 3600 } });

  // empty, anonymous
  const empty = await call(app, { path: "/api/account/apps/kokoro-tts" });
  assert.equal(empty.status, 200);
  assert.deepEqual([empty.body.likes, empty.body.comments, empty.body.mine], [0, 0, null]);
  assert.equal((await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/comments", body: { text: "hi" } })).status, 401);
  assert.equal((await call(app, { path: "/api/account/apps/Not_An_App" })).status, 400);

  // like, then take it back
  assert.equal((await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/react", token: alice, body: { value: 1 } })).body.likes, 1);
  assert.equal((await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/react", token: bob, body: { value: 1 } })).body.likes, 2);
  assert.equal((await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/react", token: bob, body: { value: 0 } })).body.likes, 1);
  assert.equal((await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/react", token: bob, body: { value: 7 } })).status, 400);

  // a comment, a reply, a reply to the reply (joins the same thread), a like on the comment
  const c = await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/comments", token: alice, body: { text: "  first  " } });
  assert.equal(c.status, 201);
  assert.equal(c.body.text, "first");
  assert.match(c.body.handle, /^user-[0-9a-f]{6}$/);
  const r1 = await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/comments", token: bob, body: { text: "reply", parent: c.body.id } });
  const r2 = await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/comments", token: alice, body: { text: "reply to reply", parent: r1.body.id } });
  assert.equal(r2.body.parent, c.body.id);
  assert.equal((await call(app, { method: "POST", path: `/api/account/apps/kokoro-tts/comments/${c.body.id}/react`, token: bob, body: { value: 1 } })).body.likes, 1);
  assert.equal((await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/comments", token: bob, body: { text: "   " } })).status, 400);
  assert.equal((await call(app, { method: "POST", path: "/api/account/apps/kokoro-tts/comments", token: bob, body: { text: "x".repeat(2001) } })).status, 400);

  // what anyone reads: one thread, two replies in order, nothing that names a person
  const list = await call(app, { path: "/api/account/apps/kokoro-tts/comments" });
  assert.equal(list.body.count, 3);
  assert.equal(list.body.comments.length, 1);
  assert.deepEqual(list.body.comments[0].replies.map((x) => x.text), ["reply", "reply to reply"]);
  assert.equal(list.body.comments[0].mine, null);
  assert.ok(!JSON.stringify(list.body).includes("did:privy"));
  assert.ok(!JSON.stringify(list.body).includes("author"));

  // signed in, the reader learns only which are theirs
  const mine = await call(app, { path: "/api/account/apps/kokoro-tts/comments", token: bob });
  assert.equal(mine.body.comments[0].mine.own, false);
  assert.equal(mine.body.comments[0].mine.react, 1);
  assert.equal(mine.body.comments[0].replies[0].mine.own, true);

  // only the author deletes, and a comment takes its replies with it
  assert.equal((await call(app, { method: "DELETE", path: `/api/account/apps/kokoro-tts/comments/${c.body.id}`, token: bob })).status, 403);
  assert.equal((await call(app, { method: "DELETE", path: `/api/account/apps/kokoro-tts/comments/${c.body.id}`, token: alice })).status, 200);
  assert.equal((await call(app, { path: "/api/account/apps/kokoro-tts/comments" })).body.count, 0);

  // on disk: a hash for each author, never a DID
  const disk = await readFile(join(dir, "apps", "kokoro-tts.json"), "utf8");
  assert.ok(!disk.includes("did:privy"));
});

test("comments written at once are all kept", async () => {
  const { mkdtemp } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  process.env.HUB_ACCOUNT_STATE = await mkdtemp(join(tmpdir(), "hub-apps-race-"));
  const app = createApp({ publicKey, appId: APP_ID });
  const t = Math.floor(Date.now() / 1000);
  const tokens = Array.from({ length: 8 }, (_, i) => mint({ claims: { sub: `did:privy:u${i}`, iat: t - 10, exp: t + 3600 } }));
  await Promise.all(tokens.map((token, i) => call(app, { method: "POST", path: "/api/account/apps/smollm-chat/comments", token, body: { text: `c${i}` } })));
  assert.equal((await call(app, { path: "/api/account/apps/smollm-chat/comments" })).body.count, 8);
});
