// Records what Hugging Face clients ask an endpoint for. A pass-through in front of huggingface.co: every request is
// forwarded unchanged (redirects are handed back, not followed), and one JSON line per request goes to stdout with the
// parts a drop-in endpoint must reproduce. Authorization values are never logged.
//
//   node recorder.mjs            listens on :8080; point a client at it with HF_ENDPOINT=http://<host>:8080
import http from "node:http";
import https from "node:https";

const UPSTREAM = "huggingface.co";
const ASKED = ["user-agent", "range", "accept-encoding", "if-none-match", "accept"];
const ANSWERED = ["etag", "x-repo-commit", "x-linked-etag", "x-linked-size", "content-length", "content-type", "accept-ranges", "x-xet-hash", "x-xet-refresh-route", "link", "x-error-code"];

http.createServer((req, res) => {
  const headers = { ...req.headers, host: UPSTREAM };
  const upstream = https.request({ host: UPSTREAM, path: req.url, method: req.method, headers }, (up) => {
    const location = up.headers.location;
    console.log(JSON.stringify({
      method: req.method, path: req.url, authorization: req.headers.authorization ? "sent" : "none",
      asked: Object.fromEntries(ASKED.filter((h) => req.headers[h]).map((h) => [h, String(req.headers[h]).slice(0, 80)])),
      status: up.statusCode,
      answered: Object.fromEntries(ANSWERED.filter((h) => up.headers[h]).map((h) => [h, String(up.headers[h]).slice(0, 90)])),
      location: location ? (location.startsWith("http") ? `absolute → ${new URL(location).host}` : `relative → ${location.slice(0, 80)}`) : undefined,
    }));
    res.writeHead(up.statusCode, up.headers);
    up.pipe(res);
  });
  upstream.on("error", (e) => { res.writeHead(502); res.end(String(e.message)); });
  req.pipe(upstream);
}).listen(8080, () => console.error("recording on :8080"));
