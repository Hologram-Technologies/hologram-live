// Build the one OpenAPI document for the whole hub endpoint, and the two discovery files derived from it.
//
// The hub is one base URL that answers in several dialects (Hugging Face, Ollama, OCI/ModelPack, MCP, Hologram
// objects). Until now only the Hologram Server's own six paths were described, by a document the server generates.
// This build keeps that document as the source of truth for those six paths, patches what it leaves empty
// (descriptions, tags, security, the request body of a binary upload), and adds every other route the endpoint
// answers.
//
// Writes, all under web/public, which the site build copies into dist:
//   openapi.json                      served at /openapi.json; /docs renders it, unchanged, from the same URL
//   agent.md                          what `curl gethologram.ai` answers: the whole hub in one screen, for the
//                                     agent that just arrived and has no idea what this is
//   models.md, registry.md            the same thing per section: what `curl gethologram.ai/models` answers,
//                                     so a section's chip on the site is a line you can run rather than a name
//   .well-known/agent-card.json       the same contract as skills, for frameworks that discover an agent card
//   robots.txt                        crawling policy, generated so it can never contradict the document
//
//   node scripts/openapi.build.mjs             build from the vendored server spec and the recorded evidence
//   node scripts/openapi.build.mjs --refresh   re-vendor the server spec from the live hub first
//   node scripts/openapi.build.mjs --check     build and fail if anything on disk is out of date
//
// Every example here is copied from qa/openapi/evidence.json, recorded by probe.mjs against the live hub.
import { mkdir, readFile, writeFile } from "node:fs/promises";

const HERE = new URL("./", import.meta.url);
const SERVER_SPEC = new URL("./openapi.server.json", HERE);
const PUBLIC = new URL("../public/", HERE);
const OUT = new URL("./openapi.json", PUBLIC);
const CARD = new URL("./.well-known/agent-card.json", PUBLIC);
const BRIEF = new URL("./agent.md", PUBLIC);
const SECTIONS = { models: new URL("./models.md", PUBLIC), registry: new URL("./registry.md", PUBLIC) };
const ROBOTS = new URL("./robots.txt", PUBLIC);
import { ORIGIN as BASE, HOST } from "../src/origin.mjs";

// ---------------------------------------------------------------- pieces used everywhere

const probe = (path, extra = {}) => ({ "x-hologram-probe": { path, status: 200, ...extra } });
const json = (schema, example) => ({ "application/json": { schema, ...(example === undefined ? {} : { example }) } });
const ref = (name) => ({ $ref: `#/components/schemas/${name}` });

const hubError = (status, description, example) => ({
  description,
  headers: {
    "x-error-code": { description: "A closed vocabulary: ReadOnly, GatedRepo, RepoNotFound, RevisionNotFound, EntryNotFound, SourceHasNotGotIt, UnknownSource, BadParameter, NotFound, HubError.", schema: { type: "string" } },
    "x-error-message": { description: "The same sentence as the body, readable in a browser's network panel.", schema: { type: "string" } },
  },
  content: json(ref("HubError"), example),
});

// Every route in this document can be asked for in a way it refuses. Saying so is part of the contract: a document
// that claims a route can only succeed is a document nobody can write a client against.
const READ_ONLY = {
  405: {
    description: "The hub endpoint is read-only: any method other than GET or HEAD on this path is refused.",
    headers: { "x-error-code": { description: "`ReadOnly`.", schema: { type: "string" } } },
    content: { "application/json": { schema: { $ref: "#/components/schemas/HubError" }, example: { error: "The hub endpoint is read-only." } } },
  },
};
const NOT_SERVED = {
  404: {
    description: "This document describes a deployment that serves this path; a hub that has not been updated to it answers 404 here. Fall back to `/llms.txt`, which every deployment serves.",
    content: { "text/plain": { schema: { type: "string" } } },
  },
};
const SERVER_METHOD = {
  404: {
    description: "The edge routes only GET and HEAD to this path; anything else falls through to the catch-all.",
    content: { "text/plain": { schema: { type: "string" }, example: "not found" } },
  },
};
const REGISTRY_WRITE = {
  401: {
    description: "A write was attempted without a registry token. Reads never need one.",
    content: { "text/plain": { schema: { type: "string" }, example: "write requires a registry token" } },
  },
};

const ociErrorResponse = (status, description, example) => ({ description, content: { "application/json": { schema: ref("OciError"), example } } });

const OWNER = { name: "owner", in: "path", required: true, description: "The owning organisation or user, exactly as on Hugging Face.", schema: { type: "string" }, example: "sentence-transformers" };
const NAME = { name: "name", in: "path", required: true, description: "The model name.", schema: { type: "string" }, example: "all-MiniLM-L6-v2" };
const REVISION = { name: "revision", in: "path", required: true, description: "`main`, or a commit prefix of at least seven characters. The hub indexes one revision per model and refuses any other, so `main` is always the pinned revision.", schema: { type: "string" }, example: "main" };
const FILEPATH = { name: "path", in: "path", required: true, description: "The file path inside the repository. It may contain slashes; do not encode them. The literal path `SHA256SUMS` is synthesised by the hub and is not a file of the repository.", schema: { type: "string" }, example: "config.json", "x-hologram-multi-segment": true };
const SOURCE = { name: "source", in: "path", required: true, description: "Pin one byte source instead of letting the hub choose.", schema: { type: "string", enum: ["huggingface", "modelscope", "ipfs"] }, example: "ipfs" };

// ---------------------------------------------------------------- the document

function document(server, evidence) {
  const ev = (id) => {
    const record = evidence.records.find((r) => r.id === id);
    if (!record) throw new Error(`no recorded evidence for ${id}: run web/qa/openapi/probe.mjs`);
    return record;
  };
  const body = (id) => ev(id).body;

  const spec = {
    openapi: "3.1.0",
    info: {
      title: "Hologram Model Hub",
      summary: "One endpoint for open models: find one, fetch it from a source that is up, and prove the bytes.",
      description: [
        `\`${BASE}\` is the only thing you configure. It answers in the dialect you already speak,`,
        "over one index in which every model file is named by the SHA-256 of its bytes and every hub object by the",
        "BLAKE3 of its bytes.",
        "",
        "**The short way.** Point the setting your tool already has at the hub and keep your commands:",
        `\`HF_ENDPOINT=${BASE}\` for anything built on \`huggingface_hub\` (transformers, diffusers,`,
        "sentence-transformers, vLLM, SGLang), `MODEL_ENDPOINT` for llama.cpp `-hf`,",
        `\`ollama pull ${HOST}/<owner>/<name>:<quant>\`, \`oras pull ${HOST}/<owner>/<name>:latest\`,`,
        "or the MCP server at `/mcp`.",
        "",
        "**The rules that make it safe.**",
        "The hub carries names, addresses and directions, never weight bytes: a file request is answered with a",
        "redirect to a source that passed the last probe, plus the SHA-256 the bytes must have.",
        "Expected hashes come from the index only, never from the source that serves the bytes.",
        "Nothing here verifies on your behalf, so hash what arrives: `…/resolve/main/SHA256SUMS` piped to",
        "`sha256sum -c` checks a whole download with no tool of ours.",
        "Reads are anonymous and permitted from any origin; only publishing needs a token.",
        "Credentials sent to whatever endpoint a client is pointed at are stripped at the edge and never logged.",
        "",
        "**Scope.** The hub is the file plane. Inference is deliberately out of scope and `/v1/*` is reserved and",
        "empty: the engines the hub feeds already speak the OpenAI API.",
      ].join("\n"),
      version: "1.0.0",
      license: { name: "MIT OR Apache-2.0", identifier: "MIT OR Apache-2.0" },
      contact: { name: "Hologram Technologies", url: "https://github.com/Hologram-Technologies/hologram-live" },
      "x-hologram-dialects": {
        huggingface: { setting: `HF_ENDPOINT=${BASE}`, paths: ["/api/models", "/{owner}/{name}/resolve/{revision}/{path}"] },
        ollama: { setting: `ollama pull ${HOST}/{owner}/{name}:{quant}`, paths: ["/v2/{owner}/{name}/manifests/{reference}", "/v2/{owner}/{name}/blobs/{digest}"] },
        oci: { setting: `oras pull ${HOST}/{owner}/{name}:latest`, paths: ["/v2/{owner}/{name}/manifests/{reference}"] },
        mcp: { setting: `${BASE}/mcp`, paths: ["/mcp"] },
        hologram: { setting: "GET /api/v1/objects/{address}", paths: ["/api/v1/objects/{address}"] },
      },
    },
    servers: [{ url: BASE, description: "Production. The hub is one host; there is no staging surface." }],
    externalDocs: { description: "The same endpoint explained for an agent in prose, shortest path first.", url: `${BASE}/llms.txt` },
    tags: [
      { name: "Discovery", description: "What this endpoint is, in a form a machine can bind to without reading prose." },
      { name: "Health", description: "Is the endpoint up, and which byte sources are up behind it." },
      { name: "Models", description: "Find a model and read its files, in Hugging Face's shape. This is the dialect every Python client already speaks." },
      { name: "Files", description: "Turn a model file into a URL that is up right now, together with the SHA-256 it must have." },
      { name: "Objects", description: "The content-addressed floor under every dialect. An object is named by the BLAKE3 of its bytes, so its answer can be cached forever." },
      { name: "Registry", description: "The OCI distribution dialect: Ollama pulls, CNCF ModelPack artifacts, and the hub's own daily index. Reads are anonymous; a write is refused at the edge unless it carries the registry token that `docker login` sends, and writes are not described here because only the hub publishes." },
      { name: "Account", description: "The only surface that needs a person. Everything else on this endpoint is anonymous, and signing in adds nothing to it: an account exists so that a saved list, an attributed model request and a recorded interest in publishing have somewhere to live. Signing in grants no read that anonymous callers do not already have, and publishing access is an operator's decision, not a form's." },
      { name: "MCP", description: "The Model Context Protocol server, for agents that bind to tools rather than to routes." },
    ],
    components: {
      securitySchemes: {
        publisherToken: { type: "http", scheme: "bearer", description: "A publisher token. Required to list, search or publish objects. Reads of a known address need no token." },
        privyToken: { type: "http", scheme: "bearer", bearerFormat: "JWT", description: "A Privy access token for the person signed in to the site, verified here against the app's ES256 public key. It is only ever accepted on `/api/account/*`; no read surface takes it, and it is stripped from every other route at the edge." },
      },
      schemas: schemas(),
    },
    security: [],
    paths: {},
  };

  // ---- Discovery
  spec.paths["/"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getDescriptor",
      summary: "The hub, answered three ways",
      description: [
        "One URL, three readers, chosen by `Accept`.",
        "",
        "A browser sends `text/html` and gets the website. A caller that asks for `application/json` by name gets the",
        "descriptor, which points at today's catalog address and at everything else here. Anything else — `*/*`, which",
        "is what curl, node's `fetch` and python's `requests` all send, and therefore what an arriving agent actually",
        "asks — gets `agent.md`: the whole hub on one screen, in the imperative, ending in a check it can run itself.",
        "",
        `So \`curl ${HOST}\` is the shortest useful thing an agent can be told about this service.`,
      ].join("\n"),
      parameters: [{ name: "Accept", in: "header", required: false, description: "`text/html` for the site, `application/json` for the descriptor, anything else for the brief.", schema: { type: "string" }, example: "*/*" }],
      responses: {
        200: {
          description: "The website, the descriptor or the brief, depending on what was asked for.",
          content: {
            ...json(ref("Descriptor"), body("root.descriptor")),
            "text/markdown": { schema: { type: "string", description: "The same bytes as `/agent.md`." } },
            "text/html": { schema: { type: "string" } },
          },
        },
        ...READ_ONLY,
      },
      ...probe("/", { headers: { accept: "application/json" }, contentType: "application/json" }),
    },
  };
  spec.paths["/.well-known/model-hub.json"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getWellKnownDescriptor",
      summary: "The hub descriptor at its well-known path",
      description: "The same bytes as `GET /` with `Accept: application/json`. Start here if you were given nothing but the host name.",
      responses: { 200: { description: "The descriptor.", content: json(ref("Descriptor"), body("descriptor")) }, ...NOT_SERVED },
      ...probe("/.well-known/model-hub.json", { contentType: "application/json" }),
    },
  };
  for (const path of ["/openapi.json", "/.well-known/openapi.json"]) {
    spec.paths[path] = {
      get: {
        tags: ["Discovery"],
        operationId: path.startsWith("/.well-known") ? "getWellKnownOpenapi" : "getOpenapi",
        summary: "This document",
        description: "The whole endpoint, every dialect, in one OpenAPI 3.1 document. `/docs` renders it. Both paths serve the same bytes; the well-known one exists because that is where an agent looks first.",
        responses: { 200: { description: "This document.", content: json({ type: "object" }) }, ...NOT_SERVED },
        ...probe(path, { contentType: "application/json" }),
      },
    };
  }
  spec.paths["/.well-known/agent-card.json"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getAgentCard",
      summary: "An agent card for this endpoint",
      description: "The hub described as a set of skills, for frameworks that discover services through an agent card rather than an OpenAPI document.",
      responses: { 200: { description: "The agent card.", content: json(ref("AgentCard")) }, ...NOT_SERVED },
      ...probe("/.well-known/agent-card.json", { contentType: "application/json" }),
    },
  };
  spec.paths["/agent.md"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getAgentBrief",
      summary: "The whole hub, for the agent that just arrived",
      description: "One screen of plain markdown: what this is, the one line that switches an existing tool over, the three requests that use it directly, and the rule that makes the bytes safe. It is what `GET /` answers to anything that is not a browser, and it ends with a check the reader can run to confirm the rest. The last line is a canary: a fetcher that summarises drops it, and an agent that cannot see it knows to fetch this path again verbatim.",
      responses: { 200: { description: "Markdown, a few hundred lines at most.", content: { "text/markdown": { schema: { type: "string" } }, "text/plain": { schema: { type: "string" } } } }, ...NOT_SERVED },
      ...probe("/agent.md", { contentType: "text/markdown" }),
    },
  };
  for (const [name, what] of [["models", "finding a model, proving it and fetching it"], ["registry", "pulling the same models as OCI artifacts"]]) {
    spec.paths[`/${name}`] = {
      get: {
        tags: ["Discovery"],
        operationId: name === "models" ? "getModelsSection" : "getRegistrySection",
        summary: `The ${name} section, answered two ways`,
        description: [
          `The site shows this address beside the ${name} heading. A browser gets the browse page; anything else`,
          `gets a brief covering ${what}: the few requests that do the job, in the order you would make them, with`,
          "the rule that makes the bytes safe.",
          "",
          "It adds no API — every route the brief names is already in this document. What was missing was an",
          "address that gathers them, which the site was already advertising. The trailing slash works either way,",
          "and a page below the section, such as a single model, is untouched.",
          name === "registry" ? "\n`/v2/` itself is deliberately left alone: it is a protocol endpoint and OCI clients depend on exactly what it returns." : "",
        ].filter(Boolean).join("\n"),
        responses: {
          200: {
            description: "The section brief, or the section's page.",
            headers: { vary: { description: "`Accept`, because this route has two representations.", schema: { type: "string" } } },
            content: { "text/markdown": { schema: { type: "string" } }, "text/html": { schema: { type: "string" } } },
          },
          ...NOT_SERVED,
        },
        ...probe(`/${name}`, { headers: { accept: "*/*" }, contentType: "text/markdown" }),
      },
    };
  }
  spec.paths["/models/"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getModelsPage",
      summary: "The models, browsed",
      description: "The browse page: every model the hub lists, with its files, sizes and addresses. For people; anything that is not a browser gets the section brief instead of 236 KB of markup. A page below this one, such as a single model, is untouched by that and always answers HTML.",
      responses: {
        200: {
          description: "The page to a browser; to anything else the section brief, the same bytes as `/models`.",
          headers: { vary: { description: "`Accept`, because this route has two representations.", schema: { type: "string" } } },
          content: { "text/html": { schema: { type: "string" } }, "text/markdown": { schema: { type: "string", description: "The section brief." } } },
        },
        ...NOT_SERVED,
      },
      ...probe("/models/", { headers: { accept: "text/html" }, contentType: "text/html" }),
    },
  };
  spec.paths["/docs"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getDocs",
      summary: "This document, rendered",
      description: "A reference page for people, built from `/openapi.json` at load time. It is the same contract as the document; nothing is written twice.",
      responses: { 200: { description: "An HTML page.", content: { "text/html": { schema: { type: "string" } } } }, ...NOT_SERVED },
      ...probe("/docs", { contentType: "text/html" }),
    },
  };
  // The documentation for people: /docs/ and one page per slug, each with a Markdown twin at /docs/<slug>.md so an
  // agent reads the same page without parsing markup. Built by web/build.mjs from web/docs/*.md; /llms.txt indexes them.
  spec.paths["/docs/"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getDocsIndex",
      summary: "The documentation",
      description: "Overview, quickstart, the concepts, one page per dialect, and a reference generated from this document. Every page has a Markdown twin at `/docs/{page}.md`, and `/llms.txt` lists them all.",
      responses: { 200: { description: "An HTML page.", content: { "text/html": { schema: { type: "string" } } } }, ...NOT_SERVED },
      ...probe("/docs/", { contentType: "text/html" }),
    },
  };
  spec.paths["/docs/{page}/"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getDocsPage",
      summary: "One documentation page",
      description: "The page named by its slug: `quickstart`, `verification`, `addresses`, `sources`, `huggingface`, `ollama`, `oci`, `mcp`, `objects`, `api`, `errors`, `limits`.",
      parameters: [{ name: "page", in: "path", required: true, schema: { type: "string" }, description: "The page slug, as listed in `/llms.txt`." }],
      responses: { 200: { description: "An HTML page.", content: { "text/html": { schema: { type: "string" } } } }, 404: { description: "No page has that slug.", content: { "text/html": { schema: { type: "string" } } } } },
      ...probe("/docs/quickstart/", { contentType: "text/html" }),
    },
  };
  spec.paths["/docs/{page}.md"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getDocsPageMarkdown",
      summary: "One documentation page, as Markdown",
      description: "The same page as `/docs/{page}/`, from the same source, with every link resolved to an absolute Markdown twin. `index.md` is the overview.",
      parameters: [{ name: "page", in: "path", required: true, schema: { type: "string" }, description: "The page slug, as listed in `/llms.txt`." }],
      responses: { 200: { description: "Markdown.", content: { "text/markdown": { schema: { type: "string" } } } }, 404: { description: "No page has that slug.", content: { "text/plain": { schema: { type: "string" } } } } },
      ...probe("/docs/quickstart.md", { contentType: "text/markdown" }),
    },
  };
  // The Registry page: the hub's own OCI registry, browsed. It ships with the site and loads nothing from another
  // origin (qa/registry-sealed.mjs); the registry it browses is `/v2/`.
  spec.paths["/registry/"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getRegistryPage",
      summary: "The hub's own registry, browsed",
      description: "A page over `/v2/`: every repository and tag the hub's registry holds, each layer verified in the browser against its digest. For people; a client uses `/v2/` directly, and anything that is not a browser gets the section brief instead of the markup.",
      responses: {
        200: {
          description: "The page to a browser; to anything else the section brief, the same bytes as `/registry`.",
          headers: { vary: { description: "`Accept`, because this route has two representations.", schema: { type: "string" } } },
          content: { "text/html": { schema: { type: "string" } }, "text/markdown": { schema: { type: "string", description: "The section brief." } } },
        },
        ...NOT_SERVED,
      },
      ...probe("/registry/", { headers: { accept: "text/html" }, contentType: "text/html" }),
    },
  };
  // The Spaces page: apps that run entirely in the visitor's browser, each in its own sealed frame. The page
  // reads its catalog from the site and asks `/v2/spaces/<id>/manifests/latest` whether a Space is published.
  spec.paths["/spaces/"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getSpacesPage",
      summary: "Apps that run entirely in the browser, each in its own sealed frame",
      description: "Three demo Spaces (speech, image, chat). Every file of a Space is sealed under one root digest, its model bytes are accepted only when they re-derive to the digest the model index names, and nothing runs on a server. A Space published to the registry lives at `/v2/spaces/<id>` as an OCI artifact of type `application/vnd.hologram.space.v1+json`.",
      responses: { 200: { description: "An HTML page.", content: { "text/html": { schema: { type: "string" } } } }, ...NOT_SERVED },
      ...probe("/spaces/", { contentType: "text/html" }),
    },
  };
  spec.paths["/spaces/spaces.json"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getSpacesCatalog",
      summary: "The Spaces catalog: id, sealed root, files, models and their sizes",
      description: "`{ format: \"hologram.spaces.catalog/v1\", spaces: [{ id, name, task, tagline, root, bytes, files, models, modelHost, modelBytes, source, requires, entry }] }`. `root` is SHA-256 over the Space's file map; `entry` is the page to open.",
      responses: { 200: { description: "JSON.", content: { "application/json": { schema: { type: "object" } } } }, ...NOT_SERVED },
      ...probe("/spaces/spaces.json", { contentType: "application/json" }),
    },
  };
  spec.paths["/llms.txt"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getLlmsTxt",
      summary: "The endpoint explained for an agent, shortest path first",
      description: "Prose, not a schema: what to do, in order, with the tools an agent already has. Read this when you would otherwise guess.",
      responses: { 200: { description: "Plain text.", content: { "text/plain": { schema: { type: "string" } } } }, ...NOT_SERVED },
      ...probe("/llms.txt", { contentType: "text/plain" }),
    },
  };
  spec.paths["/archive.json"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getArchive",
      summary: "Every day the hub has indexed",
      description: "One row per day: the index address, its IPFS CID, how many models and files it held, and the previous day. The chain lets anyone replay the hub's history and check each step.",
      responses: { 200: { description: "The archive ledger.", content: json(ref("Archive"), trim(body("archive"), "days")) }, ...NOT_SERVED },
      ...probe("/archive.json", { contentType: "application/json" }),
    },
  };
  spec.paths["/pins.json"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getPins",
      summary: "Which models are pinned on IPFS",
      description: "The models whose weights the hub can still serve when Hugging Face and ModelScope are both unreachable, with the IPFS root of each and the gateway to read it through.",
      responses: { 200: { description: "The pin set.", content: json(ref("Pins")) }, ...NOT_SERVED },
      ...probe("/pins.json", { contentType: "application/json" }),
    },
  };
  spec.paths["/robots.txt"] = {
    get: {
      tags: ["Discovery"],
      operationId: "getRobots",
      summary: "Crawling policy",
      description: "Allows the website, the discovery paths and this document; asks crawlers to leave redirect and blob routes alone, because they cost a byte source real traffic.",
      responses: { 200: { description: "Plain text.", content: { "text/plain": { schema: { type: "string" } } } }, ...NOT_SERVED },
      ...probe("/robots.txt", { contentType: "text/plain" }),
    },
  };

  // ---- Health
  spec.paths["/healthz"] = {
    get: {
      tags: ["Health"],
      operationId: "getHealth",
      summary: "Is the endpoint up",
      description: "The Hologram Server behind the object routes. `status: ready` means the object plane answers; it says nothing about byte sources, which have their own probe.",
      responses: { 200: { description: "The server is up.", content: json(ref("HealthResponse"), body("healthz")) }, ...SERVER_METHOD },
      ...probe("/healthz", { contentType: "application/json" }),
    },
  };
  spec.paths["/api/hub/health"] = {
    get: {
      tags: ["Health"],
      operationId: "getSourceHealth",
      summary: "Which byte sources are up, as seen from the hub",
      description: "The result of the hub's own periodic probe of every source it can redirect to, and the order it prefers them in. A file request is sent to the first source in `order` that is `ok`.\n\n**This is measured from the hub, not from you.** A source reported `ok` here can still be unreachable from the network you are on — `ipfs.filebase.io` in particular is blocked or TLS-terminated on some networks while the hub reaches it fine. Read this as the hub's routing preference, not as a promise about your own failover. If failover matters to you, test a `/via/{source}` fetch from where your code will actually run.",
      responses: { 200: { description: "Per-source liveness and the preference order.", content: json(ref("HubHealth"), body("hub.health")) }, ...READ_ONLY },
      ...probe("/api/hub/health", { contentType: "application/json" }),
    },
  };

  // ---- Models
  spec.paths["/api/models"] = {
    get: {
      tags: ["Models"],
      operationId: "listModels",
      summary: "Search the index",
      description: [
        "Hugging Face's list shape, so `HfApi.list_models` works unchanged, plus a `hologram` block on every row",
        "carrying the facts Hugging Face has no field for: the index address of the model, how many files it has,",
        "how many bytes of weights, and which sources hold them. A search costs a few hundred bytes; reading the",
        "whole catalog object costs several hundred KB, so search here rather than there.",
        "",
        "**This route lists everything the hub holds**, not only what is trending today. Rows for models on the",
        "current browse list carry the full facts — task, parameters, library, downloads, licence. Models the browse",
        "list has forgotten, but whose bytes this hub has published and still serves, appear as thin rows: the name,",
        "the index address, and `hologram.listed: false`. The facts a browse row carries were never published with",
        "the object, so a thin row says so rather than reporting zeros as if they were measurements. Filter on",
        "`hologram.listed` if you want one kind or the other.",
        "",
        "Query parameters are checked rather than coerced: a `limit` outside 1–500 or one that is not a whole number,",
        "and a `sort` outside the enum, each refuse with `400 BadParameter` and a sentence saying what is allowed.",
      ].join("\n"),
      parameters: [
        { name: "search", in: "query", description: "Case-insensitive substring of the model id.", schema: { type: "string" }, example: "qwen" },
        { name: "author", in: "query", description: "Exact owner match.", schema: { type: "string" }, example: "Qwen" },
        { name: "pipeline_tag", in: "query", description: "Exact task match.", schema: { type: "string" }, example: "text-generation" },
        { name: "library", in: "query", description: "Exact library match.", schema: { type: "string" }, example: "transformers" },
        { name: "filter", in: "query", description: "Tag that must be present. Repeat the parameter, or comma-separate. Tags include the task, the library, the format, the architecture, `license:<id>` and languages.", schema: { type: "array", items: { type: "string" } }, style: "form", explode: true, example: ["gguf"] },
        { name: "sort", in: "query", description: "Sort key.", schema: { type: "string", enum: ["downloads", "likes", "trendingScore", "trending_score", "createdAt", "created_at"], default: "trendingScore" } },
        { name: "direction", in: "query", description: "`1` sorts ascending. Anything else sorts descending.", schema: { type: "string" } },
        { name: "limit", in: "query", description: "Rows to return, 1 to 500.", schema: { type: "integer", minimum: 1, maximum: 500, default: 50 }, example: 5 },
      ],
      responses: {
        400: hubError(400, "A parameter is outside what this operation accepts.", { error: "limit must be between 1 and 500." }),
        200: {
          description: "Matching rows, most relevant first by the chosen sort.",
          headers: { "x-total-count": { description: "How many rows matched before `limit` was applied.", schema: { type: "integer" } } },
          content: json({ type: "array", items: ref("ModelRow") }, body("models.list")),
        },
        ...READ_ONLY,
      },
      ...probe("/api/models?limit=2&sort=downloads", { contentType: "application/json" }),
    },
  };
  spec.paths["/api/models/{owner}/{name}"] = {
    get: {
      tags: ["Models"],
      operationId: "getModel",
      summary: "One model at the indexed revision",
      description: "Hugging Face's model-info shape. `sha` is the revision the hub pinned; `siblings` lists every file. For sizes and hashes ask for the tree instead.",
      parameters: [OWNER, NAME],
      responses: {
        200: { description: "The model.", content: json(ref("ModelInfo"), body("models.info")) },
        403: hubError(403, "The model is gated on Hugging Face. The hub serves public models only and will not proxy a gate.", { error: "meta-llama/Llama-3-8B is gated on Hugging Face. The hub serves public models only; use huggingface.co directly for this one." }),
        404: hubError(404, "Not in the index. The request is recorded and considered for the next index run.", body("models.missing")),
      },
      ...probe(`/api/models/${evidence.sample.model}`, { contentType: "application/json" }),
    },
  };
  spec.paths["/api/models/{owner}/{name}/revision/{revision}"] = {
    get: {
      tags: ["Models"],
      operationId: "getModelAtRevision",
      summary: "One model, asserting the revision",
      description: "The same answer as `getModel`, refused unless the revision you name is the one the hub indexed. Use it to fail loudly rather than silently receive a different revision.",
      parameters: [OWNER, NAME, REVISION],
      responses: {
        200: { description: "The model.", content: json(ref("ModelInfo")) },
        404: hubError(404, "Either the model is not indexed, or it is indexed at another revision.", { error: "The hub has sentence-transformers/all-MiniLM-L6-v2 at 1110a243fdf4706b3f48f1d95db1a4f5529b4d41 only." }),
      },
      ...probe(`/api/models/${evidence.sample.model}/revision/${evidence.sample.revision}`, { contentType: "application/json" }),
    },
  };
  spec.paths["/api/models/{owner}/{name}/refs"] = {
    get: {
      tags: ["Models"],
      operationId: "listModelRefs",
      summary: "The branches the hub knows",
      description: "Always exactly one branch, `main`, pointing at the indexed revision. Clients call this before a download to turn `main` into a commit.",
      parameters: [OWNER, NAME],
      responses: {
        200: { description: "One branch, no tags.", content: json(ref("Refs"), body("models.refs")) },
        404: hubError(404, "Not in the index.", body("models.missing")),
      },
      ...probe(`/api/models/${evidence.sample.model}/refs`, { contentType: "application/json" }),
    },
  };
  spec.paths["/api/models/{owner}/{name}/tree/{revision}"] = {
    get: {
      tags: ["Models"],
      operationId: "listModelFiles",
      summary: "Every file, with its size and its SHA-256",
      description: "`oid` is the SHA-256 of the file's bytes, which is the value you check a download against. This is the only place the expected hashes come from: never take a hash from the source that serves the bytes.",
      parameters: [OWNER, NAME, REVISION],
      responses: {
        200: { description: "One entry per file.", content: json({ type: "array", items: ref("TreeEntry") }, trimArray(body("models.tree"))) },
        404: hubError(404, "Not in the index, or indexed at another revision.", { error: "The hub has sentence-transformers/all-MiniLM-L6-v2 at 1110a243fdf4706b3f48f1d95db1a4f5529b4d41 only." }),
      },
      ...probe(`/api/models/${evidence.sample.model}/tree/main`, { contentType: "application/json" }),
    },
  };
  spec.paths["/api/models/{owner}/{name}/tree/{revision}/{prefix}"] = {
    get: {
      tags: ["Models"],
      operationId: "listModelFilesUnder",
      summary: "The files under one directory",
      description: "The same entries, filtered to one directory prefix.",
      parameters: [OWNER, NAME, REVISION, { name: "prefix", in: "path", required: true, description: "Directory prefix, with or without a trailing slash.", schema: { type: "string" }, example: "1_Pooling", "x-hologram-multi-segment": true }],
      responses: {
        200: { description: "One entry per file under the prefix; empty if nothing matches.", content: json({ type: "array", items: ref("TreeEntry") }) },
        404: hubError(404, "Not in the index, or indexed at another revision.", { error: "The hub has sentence-transformers/all-MiniLM-L6-v2 at 1110a243fdf4706b3f48f1d95db1a4f5529b4d41 only." }),
      },
    },
  };
  spec.paths["/api/models/{owner}/{name}/xet-read-token/{revision}"] = {
    get: {
      tags: ["Models"],
      operationId: "getXetReadToken",
      summary: "Hand a Xet client back to Hugging Face",
      description: "`huggingface_hub` follows the hub's redirect, meets Hugging Face's Xet headers there, then asks this endpoint for a Xet read token. That token is Hugging Face's to give, so the hub sends the client there and issues nothing itself.",
      parameters: [OWNER, NAME, REVISION],
      responses: { 307: { description: "Redirect to the same path on huggingface.co.", headers: { location: { description: "The Hugging Face URL.", schema: { type: "string", format: "uri" } } } }, ...READ_ONLY },
      ...probe(`/api/models/${evidence.sample.model}/xet-read-token/main`, { status: 307, method: "HEAD" }),
    },
  };

  // ---- Files
  spec.paths["/{owner}/{name}/resolve/{revision}/{path}"] = {
    get: {
      tags: ["Files"],
      operationId: "resolveFile",
      summary: "A URL for one file, at a source that is up",
      description: [
        "The route every download goes through. The hub answers `302` to the first source in its preference order",
        "that passed the last probe, and puts the expected SHA-256 in `ETag` and `X-Linked-Etag` and the serving",
        "source in `X-Hub-Source`. No weight byte passes through the hub.",
        "",
        "Ask for the path `SHA256SUMS` and the hub synthesises the checksum file for the whole model instead, so a",
        "download can be checked with `sha256sum -c` and no tool of ours.",
      ].join("\n"),
      parameters: [OWNER, NAME, REVISION, FILEPATH],
      responses: {
        302: {
          description: "Follow `Location` for the bytes, then check them against `ETag`.",
          headers: {
            location: { description: "Where the bytes are, right now.", schema: { type: "string", format: "uri" } },
            etag: { description: "The SHA-256 the bytes must have, quoted. Identical to `X-Linked-Etag`.", schema: { type: "string" } },
            "x-linked-etag": { description: "The SHA-256 the bytes must have, quoted.", schema: { type: "string" } },
            "x-linked-size": { description: "The size in bytes.", schema: { type: "integer" } },
            "x-repo-commit": { description: "The revision this file belongs to.", schema: { type: "string" } },
            "x-hub-source": { description: "Which source was chosen: `huggingface.co`, `modelscope.cn` or `ipfs`.", schema: { type: "string" } },
            "accept-ranges": { description: "`bytes`. Range reads work through the redirect.", schema: { type: "string" } },
          },
        },
        200: { description: "Only for the synthesised `SHA256SUMS` path: the checksum file itself.", content: { "text/plain": { schema: { type: "string" }, example: "dcd602d2fd35c203a247304a06fec6654a12f7941b739f9221a064fe8dc3b7f0  README.md\n" } } },
        404: hubError(404, "The model is not indexed, the revision is not the indexed one, or the file is not in it.", body("resolve.entry.missing")),
      },
      ...probe(`/${evidence.sample.model}/resolve/main/${evidence.sample.file}`, { status: 302, method: "HEAD" }),
    },
  };
  spec.paths["/via/{source}/{owner}/{name}/resolve/{revision}/{path}"] = {
    get: {
      tags: ["Files"],
      operationId: "resolveFileVia",
      summary: "A URL for one file, from the source you name",
      description: [
        "The caller states the policy and the hub executes it: `/via/ipfs/...` serves from IPFS even when Hugging",
        "Face is up. The prefix works in front of any Models or Files route, not only this one.",
        "",
        "**A pin is a constraint, not a hint.** If the named source does not hold the file this refuses with",
        "`404 SourceHasNotGotIt`, naming the sources that do; an unrecognised source name refuses with",
        "`404 UnknownSource`. It does not quietly serve you something else. That matters most for the obvious use of",
        "a pin — fetching the same file through two sources and comparing them — which is worthless if one fetch can",
        "silently come from the other's host.",
        "",
        "`X-Hub-Source` still names who served, and on this route it will always equal the source you asked for.",
        "Drop the prefix to let the hub choose by health and order.",
      ].join("\n"),
      parameters: [SOURCE, OWNER, NAME, REVISION, FILEPATH],
      responses: {
        302: {
          description: "Redirect to a source. Check `X-Hub-Source`: it names who actually served, which is not always the source you asked for.",
          headers: {
            location: { description: "Where the bytes are.", schema: { type: "string", format: "uri" } },
            "x-hub-source": { description: "Who actually served: `huggingface.co`, `modelscope.cn` or `ipfs`. Compare it against the source you named; a different value means the hub fell back.", schema: { type: "string", enum: ["huggingface.co", "modelscope.cn", "ipfs"] } },
            etag: { description: "The SHA-256 the bytes must have, quoted. Returned here exactly as on the unprefixed route.", schema: { type: "string" } },
            "x-linked-etag": { description: "The same SHA-256, quoted.", schema: { type: "string" } },
            "x-linked-size": { description: "The size in bytes.", schema: { type: "integer" } },
            "x-repo-commit": { description: "The revision this file belongs to.", schema: { type: "string" } },
          },
        },
        200: { description: "Only for the synthesised `SHA256SUMS` path, which the prefix reaches like any other.", content: { "text/plain": { schema: { type: "string" } } } },
        404: hubError(404, "Unknown model, revision or file; `SourceHasNotGotIt` when the named source does not hold this file, naming the ones that do; or `UnknownSource` when the source name is not one this hub knows. A malformed source segment, such as one with capitals, is refused by the edge as a bare 404 with an empty body rather than in this shape.", { error: "modelscope does not hold config.json of BAAI/bge-base-en-v1.5. This file is on: huggingface.co. Drop the /via/ prefix to let the hub choose." }),
        405: READ_ONLY[405],
      },
      ...probe(`/via/ipfs/${evidence.sample.model}/resolve/main/${evidence.sample.file}`, { status: 302, method: "HEAD" }),
    },
  };
  spec.paths["/via/{source}/api/models"] = {
    get: {
      tags: ["Files"],
      operationId: "listModelsVia",
      summary: "Search with a source pinned for what follows",
      description: `The same rows as \`listModels\`. The prefix is accepted on every read route so a client can be configured once, with \`HF_ENDPOINT=${BASE}/via/ipfs\`, and never choose again.`,
      parameters: [SOURCE],
      responses: { 200: { description: "Matching rows.", content: json({ type: "array", items: ref("ModelRow") }) }, ...READ_ONLY },
      ...probe("/via/ipfs/api/models?limit=1", { contentType: "application/json" }),
    },
  };

  // ---- Registry
  spec.paths["/v2/"] = {
    get: {
      tags: ["Registry"],
      operationId: "getRegistryBase",
      summary: "The OCI distribution entry point",
      description: "`200 {}` means version 2 of the distribution API is supported and reads are anonymous. `docker login` is needed only to push.",
      responses: { 200: { description: "The registry speaks the v2 API.", content: json({ type: "object" }, {}) }, ...REGISTRY_WRITE },
      ...probe("/v2/", { contentType: "application/json" }),
    },
  };
  spec.paths["/v2/_catalog"] = {
    get: {
      tags: ["Registry"],
      operationId: "listRegistryRepositories",
      summary: "The repositories the hub's own registry holds",
      description: "The hub's registry namespace only, `model-hub/index` among them. Model repositories are answered from the index and do not appear here; ask for a model's tags directly.",
      responses: { 200: { description: "Repository names.", content: json(ref("OciCatalog"), body("oci.catalog")) }, ...REGISTRY_WRITE },
      ...probe("/v2/_catalog", { contentType: "application/json" }),
    },
  };
  spec.paths["/v2/{owner}/{name}/tags/list"] = {
    get: {
      tags: ["Registry"],
      operationId: "listModelTags",
      summary: "The tags a model can be pulled by",
      description: "`latest` always, plus one tag per single-file GGUF quantisation, which is what `ollama pull …:Q4_K_M` asks for. Owner and name must be lowercase: OCI references are case-sensitive and lowercase-only.",
      parameters: [OWNER, NAME],
      responses: {
        200: { description: "The tag list.", content: json(ref("OciTagList"), body("oci.tags")) },
        404: ociErrorResponse(404, "Not in the index.", { errors: [{ code: "NAME_UNKNOWN", message: "nobody/nothing is not in the Hologram index yet." }] }),
      },
      ...probe(`/v2/${evidence.sample.model.toLowerCase()}/tags/list`, { contentType: "application/json" }),
    },
  };
  spec.paths["/v2/{owner}/{name}/manifests/{reference}"] = {
    get: {
      tags: ["Registry"],
      operationId: "getModelManifest",
      summary: "A model as an OCI artifact",
      description: [
        "One route, two answers, chosen by `Accept`. Send the OCI manifest type and the hub answers a CNCF ModelPack",
        "artifact whose layers are the model's files, each layer digest being that file's SHA-256, which `oras`,",
        "`modctl`, `skopeo` and `crane` all verify for you. Send the Docker manifest type, which is what Ollama sends,",
        "and the hub answers a GGUF model manifest; models with no GGUF file refuse it.",
      ].join("\n"),
      parameters: [OWNER, NAME, { name: "reference", in: "path", required: true, description: "A tag, or a manifest digest asked for again.", schema: { type: "string" }, example: "latest" }, { name: "Accept", in: "header", required: false, description: "`application/vnd.oci.image.manifest.v1+json` for ModelPack, `application/vnd.docker.distribution.manifest.v2+json` for Ollama.", schema: { type: "string" } }],
      responses: {
        200: {
          description: "The manifest. Every layer digest is the SHA-256 of one file of the model.",
          headers: { "docker-content-digest": { description: "The digest of the manifest itself.", schema: { type: "string" } }, "x-repo-commit": { description: "The revision the artifact describes.", schema: { type: "string" } } },
          content: { "application/vnd.oci.image.manifest.v1+json": { schema: ref("OciManifest") }, "application/vnd.docker.distribution.manifest.v2+json": { schema: ref("OciManifest") } },
        },
        404: ociErrorResponse(404, "Not in the index, or asked for a GGUF manifest of a model that has none.", body("oci.manifest.docker")),
      },
      ...probe(`/v2/${evidence.sample.model.toLowerCase()}/manifests/latest`, { headers: { accept: "application/vnd.oci.image.manifest.v1+json" }, contentType: "application/vnd.oci.image.manifest.v1+json" }),
    },
  };
  spec.paths["/v2/{owner}/{name}/blobs/{digest}"] = {
    get: {
      tags: ["Registry"],
      operationId: "getModelBlob",
      summary: "One layer of a model artifact",
      description: "Small layers (config, chat template, parameters) are answered directly, each checked against its digest before the hub keeps it. Weight layers are a redirect to a live source; the client verifies the digest, as every OCI client already does. `HEAD` answers `200` with the size directly, because newer Ollama refuses a cross-host redirect on `HEAD`.",
      parameters: [OWNER, NAME, { name: "digest", in: "path", required: true, description: "`sha256:<64 hex>`, the SHA-256 of the file's bytes.", schema: { type: "string", pattern: "^sha256:[0-9a-f]{64}$" } }],
      responses: {
        200: { description: "A small layer, in full.", content: { "application/octet-stream": { schema: { type: "string", format: "binary" } } } },
        307: { description: "A weight layer: follow `Location`, then verify the digest.", headers: { location: { description: "Where the bytes are.", schema: { type: "string", format: "uri" } } } },
        404: ociErrorResponse(404, "No such blob in this model at the indexed revision.", { errors: [{ code: "BLOB_UNKNOWN", message: "No layer with that digest in this model." }] }),
      },
    },
  };

  // ---- MCP
  spec.paths["/mcp"] = {
    post: {
      tags: ["MCP"],
      operationId: "callMcp",
      summary: "The Model Context Protocol server",
      description: [
        "Streamable HTTP, stateless, anonymous, `POST` only. Protocol versions `2026-07-28`, `2025-11-25`,",
        "`2025-06-18` and `2025-03-26` are accepted. Three tools: `search_models` finds one, `get_model` lists its",
        "files with sizes and hashes, `resolve_file` gives a URL, the SHA-256 it must have, and the command that",
        "hands the file to an engine.",
        "",
        "Weight bytes never travel in a tool result: the tool returns the instruction, the agent's shell or engine",
        "does the download.",
        "",
        "`get_model` returns at most 200 files. When a model has more it keeps every config, tokenizer and other",
        "small text file first, fills the remainder with the largest weights, and sets `files_truncated` and",
        "`files_truncated_note`; `files_total` always counts the whole model. It truncates by usefulness rather than",
        "alphabetically, because the tail of a model directory is where the tokenizer lives and a caller cannot",
        "proceed without it. For the complete list use `listModelFiles`.",
      ].join("\n"),
      requestBody: { required: true, description: "A JSON-RPC 2.0 request.", content: json(ref("JsonRpcRequest"), { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "search_models", arguments: { query: "qwen", limit: 2 } } }) },
      responses: {
        200: { description: "A JSON-RPC 2.0 response. A tool that fails answers `200` with `result.isError` set, as the protocol requires.", content: json(ref("JsonRpcResponse"), body("mcp.initialize")) },
        405: { description: "`GET` and `DELETE` are refused: the server keeps no session, so there is no stream to open or close.", content: json(ref("HubError")) },
      },
      ...probe("/mcp", { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream" }, body: { jsonrpc: "2.0", id: 1, method: "tools/list", params: {} }, contentType: "application/json" }),
    },
  };

  // ---- Account
  const accountErrors = {
    401: { description: "Not signed in, or the token is not this hub's to accept.", content: json(ref("AccountError"), { error: "sign in to use this" }) },
    429: { description: "Too many requests from this account.", content: json(ref("AccountError"), { error: "too many requests, slow down" }) },
    503: { description: "Sign-in is not configured on this hub. Every anonymous surface still works.", content: json(ref("AccountError"), { error: "sign-in is not configured on this hub" }) },
  };
  const account = (operationId, summary, description, extra = {}) => ({
    tags: ["Account"],
    operationId,
    summary,
    description,
    security: [{ privyToken: [] }],
    ...extra,
    responses: { ...extra.responses, ...accountErrors },
  });

  spec.paths["/api/account/health"] = {
    get: {
      tags: ["Account"],
      operationId: "getAccountHealth",
      summary: "Is sign-in available",
      description: "The one account route that needs no token. `configured: false` means this hub has no sign-in keys, which changes nothing for anonymous callers.",
      responses: {
        200: { description: "Whether the account service is up and configured.", content: json(ref("AccountHealth"), body("account.health")) },
        404: { description: "Any other path under `/api/account` that is not a route answers 404 once the caller is signed in.", content: json(ref("AccountError"), { error: "no such account route" }) },
      },
      ...probe("/api/account/health", { contentType: "application/json" }),
    },
  };
  spec.paths["/api/account/me"] = {
    get: account("getAccount", "The signed-in account", "What the hub holds for this person: nothing but a saved list, the models they asked for, and a wallet address they chose to report.", {
      responses: { 200: { description: "The account.", content: json(ref("Account"), { did: "did:privy:cm2…", created: "2026-09-22T10:00:00.000Z", wallet: null, saved: ["Qwen/Qwen3-0.6B"], requests: 2 }) } },
    }),
    patch: account("updateAccountWallet", "Record or clear a wallet address", "The address is Privy's to mint and the page's to report. It is recorded and never trusted for anything.", {
      requestBody: { required: true, content: json({ type: "object", required: ["wallet"], properties: { wallet: { type: ["string", "null"], pattern: "^0x[0-9a-fA-F]{40}$", description: "An address, or null to clear it." } } }, { wallet: "0x0000000000000000000000000000000000000000" }) },
      responses: {
        200: { description: "The address as it now stands.", content: json({ type: "object", properties: { wallet: { type: ["string", "null"] } } }) },
        400: { description: "Not an address.", content: json(ref("AccountError"), { error: "not an address" }) },
      },
    }),
  };
  spec.paths["/api/account/saved"] = {
    post: account("saveModel", "Save a model to this account", "A list the person keeps, nothing more. It is not a download, a pin or a claim on anything.", {
      requestBody: { required: true, content: json({ type: "object", required: ["model"], properties: { model: { type: "string", example: "Qwen/Qwen3-0.6B" } } }) },
      responses: {
        200: { description: "The saved list.", content: json(ref("SavedList")) },
        400: { description: "Not a model id.", content: json(ref("AccountError"), { error: "not a model id" }) },
        409: { description: "The saved list is full.", content: json(ref("AccountError"), { error: "saved list is full" }) },
      },
    }),
  };
  spec.paths["/api/account/saved/{model}"] = {
    delete: account("unsaveModel", "Remove a model from the saved list", "Removing something that is not there is not an error.", {
      parameters: [{ name: "model", in: "path", required: true, description: "The model id, URL-encoded (the slash included).", schema: { type: "string" }, example: "Qwen%2FQwen3-0.6B" }],
      responses: { 200: { description: "The saved list as it now stands.", content: json(ref("SavedList")) } },
    }),
  };
  spec.paths["/api/account/request"] = {
    post: account("requestModel", "Ask for a model to be indexed", "The anonymous version of this already happens: the endpoint records every model it is asked for and does not have. Signing in only adds attribution, so the operator can tell one person asking ten times from ten people asking once.", {
      requestBody: { required: true, content: json({ type: "object", required: ["model"], properties: { model: { type: "string", example: "Qwen/Qwen3-0.6B" } } }) },
      responses: {
        200: { description: "Recorded.", content: json({ type: "object", properties: { requested: { type: "string" } } }) },
        400: { description: "Not a model id.", content: json(ref("AccountError"), { error: "not a model id" }) },
        409: { description: "This account has asked for enough for now.", content: json(ref("AccountError"), { error: "you have asked for enough for now" }) },
      },
    }),
  };
  spec.paths["/api/account/publisher-request"] = {
    post: account("requestPublisherAccess", "Record an interest in publishing", "It issues nothing and grants nothing. Who may write to this registry is an operator's decision, and a decision is not something a web form gets to make.", {
      requestBody: { required: true, content: json({ type: "object", properties: { note: { type: "string", maxLength: 500 } } }, { note: "I maintain three GGUF conversions and would like to publish them here." }) },
      responses: { 202: { description: "Recorded, and nothing was issued.", content: json({ type: "object", properties: { recorded: { const: true }, issued: { const: false } } }, { recorded: true, issued: false }) } },
    }),
  };

  // ---- Objects: the server's own six paths, kept from its document and patched
  Object.assign(spec.paths, objectPaths(server, evidence));

  // Keep the server's schemas, minus the ones its paths no longer reference after the patch.
  for (const [name, schema] of Object.entries(server.components?.schemas || {})) {
    if (!spec.components.schemas[name]) spec.components.schemas[name] = schema;
  }
  prune(spec);
  return spec;
}

// The Hologram Server generates these six with empty descriptions, no tags, no security and a request body typed as
// an array of integers. The wire is right; the document is not. Patch it here rather than in the server, because the
// server is upstream and its document is generated for a different audience.
function objectPaths(server, evidence) {
  const p = structuredClone(server.paths);
  const set = (path, method, patch) => Object.assign(p[path][method], patch);
  const rename = { capabilities: "getCapabilities", list_modules: "listModules", list_objects: "listObjects", put_object: "publishObject", search_objects: "searchObjects", get_object: "getObject", healthz: "getServerHealth" };
  for (const [path, item] of Object.entries(p)) {
    for (const [method, op] of Object.entries(item)) {
      op.tags = ["Objects"];
      if (rename[op.operationId]) op.operationId = rename[op.operationId];
      for (const response of Object.values(op.responses || {})) if (!response.description) response.description = "Success.";
    }
  }
  delete p["/healthz"];        // described above, with a real description
  delete p["/api/v1/objects/search"];

  set("/api/v1/capabilities", "get", {
    summary: "What this server can do",
    description: "Every operation the server exposes, which modules are loaded, and the hard limits. `maximum_message_bytes` is 32 MiB: anything larger is published as chunks.",
    responses: { 200: { description: "The capability manifest.", content: json(ref("CapabilityManifest"), evidence.records.find((r) => r.id === "capabilities").body) }, ...SERVER_METHOD },
    ...probe("/api/v1/capabilities", { contentType: "application/json" }),
  });
  set("/api/v1/modules", "get", {
    summary: "The modules that are loaded",
    description: "One entry per module, with the operations it owns and whether it is ready.",
    responses: { ...p["/api/v1/modules"].get.responses, ...SERVER_METHOD },
    ...probe("/api/v1/modules", { contentType: "application/json" }),
  });
  set("/api/v1/objects/{id}", "get", {
    summary: "One object, by the hash of its bytes",
    description: "The content-addressed floor. The answer for an address never changes, so it is served `immutable` with a one-year lifetime and may be cached forever. The server does not verify on read: hash what arrives and keep it only if the BLAKE3 equals the address you asked for.",
    responses: {
      ...p["/api/v1/objects/{id}"].get.responses,
      200: {
        description: "The object's bytes, served under the media type they were published with. The hub publishes `application/vnd.hologram.model-hub.catalog.v1+json` for a catalog, `…model.v1+json` for one model revision and `…source.v1+json` for one place its bytes can be fetched; anything else published here keeps its own type, and an object stored without one is served as opaque bytes. Match on the address, not on the media type.",
        headers: {
          etag: { description: "The address, quoted. It cannot change, because the address is the hash of these bytes.", schema: { type: "string" } },
          "cache-control": { description: "`public, max-age=31536000, immutable`.", schema: { type: "string" } },
        },
        content: {
          "application/vnd.hologram.model-hub.catalog.v1+json": { schema: { type: "object", description: "Today's browse rows, the map from model id to addresses, and `prev`." } },
          "application/vnd.hologram.model-hub.model.v1+json": { schema: { type: "object", description: "One revision of one model: id, revision, license, files with sizes and SHA-256s, and `prev`." } },
          "application/vnd.hologram.model-hub.source.v1+json": { schema: { type: "object", description: "One place the bytes of one revision can be fetched: ipfs, http, or chunks on any Hologram Server." } },
          "application/octet-stream": { schema: { type: "string", format: "binary" } },
        },
      },
      400: { description: "The path is not an address. An address is `blake3:` followed by 64 hexadecimal characters; anything else under this prefix is refused at the edge rather than passed to the server.", content: json(ref("ApiError"), { code: "LIVE_BAD_REQUEST", message: "an object address is blake3: followed by 64 hexadecimal characters" }) },
      404: { description: "No object at that address.", content: json(ref("ApiError"), evidence.records.find((r) => r.id === "objects.missing").body) },
    },
    ...probe(`/api/v1/objects/${evidence.sample.catalog}`, { method: "HEAD" }),
  });
  set("/api/v1/objects", "get", {
    summary: "List stored objects",
    description: "Needs a publisher token. Reading a known address needs none. There is no anonymous listing, on purpose: the catalog is the public index and it is one object away.",
    security: [{ publisherToken: [] }],
    responses: { ...p["/api/v1/objects"].get.responses, 401: { description: "No publisher token.", content: { "text/plain": { schema: { type: "string" }, example: "publish requires a publisher token" } } } },
  });
  set("/api/v1/objects", "post", {
    summary: "Publish an object",
    description: "The body is the bytes; the response `id` is their BLAKE3 address. Up to 8 MiB per request at the edge, 32 MiB at the server. Larger payloads are published as chunks and joined by a source record.",
    security: [{ publisherToken: [] }],
    requestBody: { required: true, description: "The object's bytes.", content: { "application/octet-stream": { schema: { type: "string", format: "binary" } } } },
    responses: { ...p["/api/v1/objects"].post.responses, 401: { description: "No publisher token.", content: { "text/plain": { schema: { type: "string" }, example: "publish requires a publisher token" } } } },
  });
  // The server names the path parameter `id`; say what it is.
  for (const param of p["/api/v1/objects/{id}"].get.parameters || []) {
    if (param.name === "id") Object.assign(param, { description: "`blake3:<64 hex>`, the hash of the object's bytes.", schema: { type: "string", pattern: "^blake3:[0-9a-f]{64}$" }, example: evidence.sample.catalog });
  }
  return p;
}

// A schema nothing can reach is a schema nobody can trust: drop what the patched paths no longer reference.
function prune(spec) {
  const used = new Set();
  const walk = (node) => {
    if (Array.isArray(node)) return node.forEach(walk);
    if (!node || typeof node !== "object") return;
    for (const [key, value] of Object.entries(node)) {
      if (key === "$ref" && typeof value === "string" && value.startsWith("#/components/schemas/")) {
        const name = value.slice("#/components/schemas/".length);
        if (!used.has(name)) { used.add(name); walk(spec.components.schemas[name]); }
      } else walk(value);
    }
  };
  walk(spec.paths);
  for (const name of Object.keys(spec.components.schemas)) if (!used.has(name)) delete spec.components.schemas[name];
}

// ---------------------------------------------------------------- schemas

function schemas() {
  const address = { type: "string", pattern: "^blake3:[0-9a-f]{64}$", description: "The BLAKE3 hash of an object's bytes, which is its name." };
  const sha256 = { type: "string", pattern: "^[0-9a-f]{64}$", description: "The SHA-256 of a file's bytes." };
  return {
    Descriptor: {
      type: "object",
      description: "What the hub is and where everything under it lives. One fetch, no schema needed to read it.",
      required: ["format", "catalog", "fetch", "openapi"],
      properties: {
        format: { const: "hologram.model-hub.descriptor/v1" },
        name: { type: "string" },
        about: { type: "string" },
        catalog: { ...address, description: "Today's catalog object: the whole index in one address." },
        snapshot: { type: "string", format: "date", description: "The day this catalog was built." },
        fetch: { type: "string", description: "The template for fetching any object by address." },
        find: { type: "string" },
        publish: { type: "string" },
        openapi: { type: "string", description: "Where this document lives." },
        capabilities: { type: "string" },
        registry: { type: "string" },
        guide: { type: "string" },
        addresses: { type: "object", additionalProperties: { type: "string" } },
        kinds: { type: "object", additionalProperties: { type: "string" }, description: "The object kinds the hub publishes and what each holds." },
      },
    },
    AgentCard: {
      type: "object",
      description: "The hub as a set of skills, for frameworks that discover a service through an agent card.",
      required: ["name", "description", "url", "skills"],
      properties: {
        name: { type: "string" },
        description: { type: "string" },
        url: { type: "string", format: "uri" },
        version: { type: "string" },
        provider: { type: "object", properties: { organization: { type: "string" }, url: { type: "string", format: "uri" } } },
        documentationUrl: { type: "string", format: "uri" },
        capabilities: { type: "object", additionalProperties: true },
        defaultInputModes: { type: "array", items: { type: "string" } },
        defaultOutputModes: { type: "array", items: { type: "string" } },
        skills: { type: "array", items: { type: "object", required: ["id", "name", "description"], properties: { id: { type: "string" }, name: { type: "string" }, description: { type: "string" }, tags: { type: "array", items: { type: "string" } }, examples: { type: "array", items: { type: "string" } } } } },
      },
    },
    HologramFacts: {
      type: "object",
      description: "What Hugging Face's row has no field for: where this model sits in the hub's index.",
      required: ["manifest", "files"],
      properties: {
        manifest: { ...address, description: "The index object for this model at this revision." },
        files: { type: "integer", description: "How many files the revision has." },
        weight_bytes: { type: "integer", description: "Total bytes of weight files." },
        parameters: { type: "integer", description: "Parameter count, when it is known." },
        listed: { type: "boolean", description: "Absent or true on a model the current browse list carries, which is a row with the full facts. `false` on a model the browse list has forgotten: the hub still holds its bytes and serves every dialect for it, but the task, parameter count and download figures were never published with the object, so they are absent rather than zero." },
        context: { type: "integer", description: "Context length, when it is known." },
        sources: { type: "array", items: { type: "string", enum: ["Hugging Face", "ModelScope", "IPFS", "P2P"] }, description: "Which sources hold these bytes today, in display spelling. Three vocabularies describe the same sources and nothing else maps between them, so map here: `Hugging Face` is `huggingface.co` in `/api/hub/health` and `X-Hub-Source`, and `huggingface` in a `/via/` prefix; `ModelScope` is `modelscope.cn` and `modelscope`; `IPFS` is `ipfs` in both. `P2P` is a fourth source that appears on a few rows and has no health entry and no `/via/` token — `/via/p2p/` is not a route." },
      },
    },
    ModelRow: {
      type: "object",
      description: "One search result, in Hugging Face's list shape plus a `hologram` block.",
      required: ["id", "modelId", "sha", "hologram"],
      properties: {
        _id: { type: "string" }, id: { type: "string", example: "Qwen/Qwen-Image-2.1" }, modelId: { type: "string" }, author: { type: "string" },
        sha: { type: "string", description: "The indexed revision." },
        private: { const: false }, gated: { const: false }, disabled: { const: false },
        likes: { type: "integer" }, downloads: { type: "integer" }, trendingScore: { type: "integer" },
        createdAt: { type: "string", format: "date-time" },
        pipeline_tag: { type: "string" }, library_name: { type: "string" },
        tags: { type: "array", items: { type: "string" } },
        hologram: ref("HologramFacts"),
      },
    },
    ModelInfo: {
      type: "object",
      description: "One model at the indexed revision, in Hugging Face's model-info shape.",
      required: ["id", "modelId", "sha", "siblings"],
      properties: {
        _id: { type: "string" }, id: { type: "string" }, modelId: { type: "string" },
        sha: { type: "string", description: "The indexed revision, always a full commit hash." },
        private: { const: false }, gated: { const: false }, disabled: { const: false },
        tags: { type: "array", items: { type: "string" } }, downloads: { type: "integer" }, likes: { type: "integer" },
        siblings: { type: "array", description: "Every file of the revision, by name only.", items: { type: "object", required: ["rfilename"], properties: { rfilename: { type: "string" } } } },
      },
    },
    TreeEntry: {
      type: "object",
      description: "One file, with the hash a download must match.",
      required: ["type", "oid", "size", "path"],
      properties: {
        type: { const: "file", description: "Only files: the index holds no directory entries." },
        oid: { ...sha256, description: "The SHA-256 of the file's bytes, 64 hex characters, and the value a download must be checked against. **This differs from Hugging Face**, whose `oid` is a 40-character git blob SHA-1 and which carries the SHA-256 only inside `lfs`, only on large files. Here every file reports its SHA-256 in this field; the length is the tell. A client written against Hugging Face's meaning will read a perfectly good hash as an unusable one." },
        size: { type: "integer" },
        path: { type: "string" },
        lfs: { type: "object", description: "Present on large files, for clients that branch on it. Its `oid` repeats the SHA-256 above rather than differing from it.", properties: { oid: sha256, size: { type: "integer" }, pointerSize: { type: "integer" } } },
      },
    },
    Refs: {
      type: "object",
      description: "The branches the hub knows: exactly one.",
      required: ["branches", "tags", "converts"],
      properties: {
        branches: { type: "array", items: { type: "object", required: ["name", "ref", "targetCommit"], properties: { name: { const: "main" }, ref: { const: "refs/heads/main" }, targetCommit: { type: "string" } } } },
        tags: { type: "array", items: { type: "object" } },
        converts: { type: "array", items: { type: "object" } },
      },
    },
    HubHealth: {
      type: "object",
      description: "Which byte sources are up, and the order the hub prefers them in.",
      required: ["sources", "order"],
      properties: {
        sources: { type: "object", additionalProperties: { type: "object", required: ["ok", "checked"], properties: { ok: { type: "boolean", description: "Whether the hub reached it on its last probe." }, checked: { type: "string", format: "date-time" }, reason: { type: "string", description: "Why the hub believes it, in one word. `verified` means the hub fetched bytes and they hashed correctly — from the hub's network." } } } },
        order: { type: "array", items: { type: "string" }, description: "Preference order. A file goes to the first source here that is `ok` and holds it." },
      },
    },
    Archive: {
      type: "object",
      description: "Every day the hub has indexed, chained backwards, and the three places any day can be read from.",
      required: ["format", "days"],
      properties: {
        format: { const: "hologram.model-hub.archive/v1" },
        days: { type: "array", items: ref("ArchiveDay") },
        gateway: { type: "string", format: "uri", description: "The IPFS gateway a day can be read through." },
        mirror: { type: "string", format: "uri", description: "The hub own copy of the archive." },
        registry: { type: "string", description: "The OCI repository that holds every day index." },
      },
    },
    ArchiveDay: {
      type: "object",
      required: ["date", "index"],
      properties: {
        date: { type: "string", format: "date" }, archived: { type: "string", format: "date-time" },
        index: { ...address, description: "That day's index object." },
        catalog: { ...address, description: "That day's catalog object: the browse rows and the map from model id to address." },
        cid: { type: "string", description: "The IPFS CID the day was pinned under." },
        models: { type: "integer" }, addressed: { type: "integer" }, files: { type: "integer" }, bytes: { type: "integer" },
        prev: { type: ["string", "null"] }, prev_ledger: { type: ["string", "null"] }, source: { type: "string", description: "The commit of the code that built it." },
      },
    },
    Pins: {
      type: "object",
      description: "The models whose weights survive both Hugging Face and ModelScope being unreachable.",
      required: ["format", "gateway", "models"],
      properties: {
        format: { const: "hologram.model-hub.pins/v1" },
        gateway: { type: "string", format: "uri", description: "The IPFS gateway the hub redirects through." },
        models: { type: "object", additionalProperties: { type: "object", required: ["root", "revision"], properties: { root: { type: "string", description: "The IPFS root CID of the model directory." }, revision: { type: "string" }, pinned: { type: "string", format: "date" }, hidden: { type: "boolean" } } } },
      },
    },
    AccountHealth: {
      type: "object",
      description: "Whether this hub offers sign-in at all.",
      required: ["ok"],
      properties: { ok: { type: "boolean" }, configured: { type: "boolean", description: "False when the hub has no sign-in keys. Every anonymous surface works either way." } },
    },
    Account: {
      type: "object",
      description: "Everything the hub holds about one signed-in person, which is deliberately almost nothing.",
      required: ["did"],
      properties: {
        did: { type: "string", description: "The Privy decentralised identifier the token was issued for." },
        created: { type: "string", format: "date-time" },
        wallet: { type: ["string", "null"], description: "An address the person chose to report. Recorded, never trusted." },
        saved: { type: "array", items: { type: "string" } },
        requests: { type: "integer", description: "How many models this account has asked for." },
      },
    },
    SavedList: { type: "object", required: ["saved"], properties: { saved: { type: "array", items: { type: "string" } } } },
    AccountError: {
      type: "object",
      description: "How the account service refuses: one sentence, no code, because this surface has no clients but the site.",
      required: ["error"],
      properties: { error: { type: "string" } },
    },
    HubError: {
      type: "object",
      description: "How every dialect route refuses: one sentence that names the fix, with a machine code in `X-Error-Code`.",
      required: ["error"],
      properties: { error: { type: "string" } },
    },
    OciError: {
      type: "object",
      description: "How the registry dialect refuses, in the shape the OCI distribution spec requires.",
      required: ["errors"],
      properties: { errors: { type: "array", items: { type: "object", required: ["code", "message"], properties: { code: { type: "string", example: "MANIFEST_UNKNOWN" }, message: { type: "string" } } } } },
    },
    OciCatalog: { type: "object", required: ["repositories"], properties: { repositories: { type: "array", items: { type: "string" } } } },
    OciTagList: { type: "object", required: ["name", "tags"], properties: { name: { type: "string" }, tags: { type: "array", items: { type: "string" } } } },
    OciManifest: {
      type: "object",
      description: "An OCI image manifest. Every layer is one file of the model and its digest is that file's SHA-256, which is what makes a registry with no bytes of its own verifiable.",
      required: ["schemaVersion", "mediaType", "config", "layers"],
      properties: {
        schemaVersion: { const: 2 }, mediaType: { type: "string" }, artifactType: { type: "string" },
        config: ref("OciDescriptor"),
        layers: { type: "array", items: ref("OciDescriptor") },
        annotations: { type: "object", additionalProperties: { type: "string" } },
      },
    },
    OciDescriptor: {
      type: "object",
      required: ["mediaType", "digest", "size"],
      properties: { mediaType: { type: "string" }, digest: { type: "string", pattern: "^sha256:[0-9a-f]{64}$" }, size: { type: "integer" }, annotations: { type: "object", additionalProperties: { type: "string" } } },
    },
    JsonRpcRequest: {
      type: "object",
      description: "A JSON-RPC 2.0 request. `method` is an MCP method: `initialize`, `tools/list`, `tools/call`, `ping`, `resources/list`, `prompts/list`.",
      required: ["jsonrpc", "method"],
      properties: { jsonrpc: { const: "2.0" }, id: { type: ["string", "integer"] }, method: { type: "string" }, params: { type: "object", additionalProperties: true } },
    },
    JsonRpcResponse: {
      type: "object",
      description: "A JSON-RPC 2.0 response. A failing tool answers `200` with `result.isError`, as MCP requires; only a malformed call gets `error`.",
      required: ["jsonrpc"],
      properties: { jsonrpc: { const: "2.0" }, id: { type: ["string", "integer", "null"] }, result: { type: "object", additionalProperties: true }, error: { type: "object", required: ["code", "message"], properties: { code: { type: "integer" }, message: { type: "string" }, data: true } } },
    },
  };
}

// Examples are evidence, but a 500-entry example helps nobody: keep the shape, cut the length.
const trimArray = (value, keep = 3) => (Array.isArray(value) ? value.slice(0, keep) : value);
const trim = (value, key, keep = 2) => (value && Array.isArray(value[key]) ? { ...value, [key]: value[key].slice(0, keep) } : value);

// ---------------------------------------------------------------- derived discovery files

// One brief per section, answered at the section's own URL to anything that is not a browser.
//
// The site shows an endpoint beside each section heading. A name is not usable: an agent that follows it gets the
// browse page, a quarter of a megabyte of markup, which is the same failure the bare host name had. So each
// section answers the way the root does — the page to a browser, and to everything else the few requests that
// actually do that section's job, in the order you would make them.
//
// This adds no API. Every route named below already existed and is already in the document; what was missing was
// an address that gathers them, which the site was already advertising.
function sectionBrief(name) {
  const common = [
    "",
    "## The rule that makes it safe",
    "",
    "Hash what arrives. The expected hash comes from the index and never from the source that served the bytes,",
    "and this server does not verify on read.",
    "",
    "## More",
    "",
    `    ${BASE}/agent.md        the whole hub in one screen`,
    `    ${BASE}/openapi.json    every operation, OpenAPI 3.1, bindable directly`,
    `    ${BASE}/mcp             the same hub as MCP tools, no key`,
    "",
    "<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch",
    `     ${BASE}/${name} again and read it verbatim. -->`,
    "",
  ];

  if (name === "models") {
    return [
      `# ${HOST}/models`,
      "",
      "Open models: find one, prove it, and fetch it from whichever source is up. No account, no key, no SDK, and",
      "every request below is anonymous.",
      "",
      "## One line, if you can set an environment variable",
      "",
      `    HF_ENDPOINT=${BASE}`,
      "",
      "Everything built on huggingface_hub then reads from here with the commands you already have. llama.cpp uses",
      "MODEL_ENDPOINT instead.",
      "",
      "## Or four requests",
      "",
      "    GET /api/models?search=qwen&filter=gguf&limit=5",
      "        Find one. A few hundred bytes. This lists everything the hub holds, including models that have",
      "        fallen off the trending list; those carry `hologram.listed: false` and only their name is certain.",
      "",
      "    GET /api/models/{owner}/{name}/tree/main",
      "        Its files, each with `oid`: the SHA-256 the bytes must have. This is the only place an expected",
      "        hash may come from. Note it is a 64-character SHA-256, where Hugging Face puts a 40-character git",
      "        blob SHA-1 in the same field.",
      "",
      "    GET /{owner}/{name}/resolve/main/{path}",
      "        302 to a source that was up a moment ago, carrying the expected hash in `ETag` and the server in",
      "        `X-Hub-Source`. No weight byte passes through this host.",
      "",
      "    GET /{owner}/{name}/resolve/main/SHA256SUMS",
      "        The whole model's checksums, synthesised, so `sha256sum -c` checks a download with no tool of ours.",
      "",
      "## Choosing the source yourself",
      "",
      "    GET /via/{huggingface|modelscope|ipfs}/{owner}/{name}/resolve/main/{path}",
      "        Pins one source. It refuses rather than falling back, so fetching the same file through two of them",
      "        and comparing is two unrelated hosts agreeing and not one host repeating itself.",
      "",
      "    GET /api/hub/health",
      "        Which sources are up and the order this hub prefers them. Measured from the hub, not from you.",
      ...common,
    ].join("\n");
  }

  return [
    `# ${HOST}/registry`,
    "",
    "The same models as OCI artifacts, so the tools you already use for containers work unchanged. Reads are",
    "anonymous; only publishing needs a credential.",
    "",
    "## Pull with what you have",
    "",
    `    ollama pull ${HOST}/<org>/<name>:<quant>`,
    "        Ollama verifies the SHA-256 itself. Any GGUF repository in the index.",
    "",
    `    oras pull ${HOST}/<org>/<name>:latest`,
    "        A CNCF ModelPack artifact: every layer is one file of the model and its digest is that file's",
    "        SHA-256, which oras, modctl, skopeo and crane all check for you. Lowercase the reference.",
    "",
    `    hologram pull ${HOST}/model-hub/index:<YYYY-MM-DD>`,
    "        The hub's own daily index, one tag per day, kept for ever.",
    "",
    "## Or speak the protocol",
    "",
    "    GET /v2/                                      the distribution version check",
    "    GET /v2/{owner}/{name}/tags/list              latest, plus one tag per single-file GGUF quantisation",
    "    GET /v2/{owner}/{name}/manifests/{reference}  ModelPack by Accept, or Ollama's manifest",
    "    GET /v2/{owner}/{name}/blobs/{digest}         small layers directly, weights as a redirect",
    "",
    "The weights are never held here: a blob request for one is a redirect to a source that is up, and the digest",
    "you verify is the file's own SHA-256.",
    ...common,
  ].join("\n");
}

// The one file an agent reads. It is what `curl gethologram.ai` answers, because curl and every HTTP client an
// agent is built on send `Accept: */*` and would otherwise get 78 KB of markup they cannot use.
//
// Rules it lives by, learned from the ones that work (OpenRouter's learn.md, Hugging Face's own docs):
//   short enough to read whole, imperative, no SDK, no key, and it ends with something the agent can do right now
//   and check for itself. The paths are taken from the built document, so this file cannot describe a route that
//   does not exist. The last line is a canary: a summarising fetcher drops it, and the agent knows to re-fetch.
function brief(spec) {
  const has = (path) => (spec.paths[path] ? "" : " (missing)");
  const ops = Object.values(spec.paths).reduce((n, item) => n + Object.keys(item).filter((k) => k !== "parameters").length, 0);
  return [
    `# ${HOST}`,
    "",
    "You are an agent and you just fetched a model hub. One name, the dialects you already speak, and every file",
    "named by the SHA-256 of its bytes so you never have to trust the host that served them, this one included.",
    "No account, no key, no SDK, and every read below is anonymous.",
    "",
    "## One line, if you can set an environment variable",
    "",
    `    HF_ENDPOINT=${BASE}`,
    "",
    "Everything built on huggingface_hub now reads from here, with the commands you already have: transformers,",
    "diffusers, sentence-transformers, vLLM, SGLang. llama.cpp uses MODEL_ENDPOINT instead. Same cache, same files.",
    "",
    "## Or call it directly. Three requests, no key",
    "",
    `    GET /api/models?search=qwen&limit=5${has("/api/models")}`,
    "        Find a model. A few hundred bytes, not a catalogue. This lists everything the hub holds, including",
    "        models that have fallen off the trending list; those come back as thin rows carrying the name and",
    "        `hologram.listed: false`, because their browse facts were never published with the bytes.",
    `    GET /api/models/{owner}/{name}/tree/main${has("/api/models/{owner}/{name}/tree/{revision}")}`,
    "        Its files. Each one carries `oid`: the SHA-256 the bytes must have.",
    "        Read that field carefully if you already know this dialect. Hugging Face puts a 40-character git",
    "        blob SHA-1 in `oid` and hides the SHA-256 inside `lfs` on large files only. Here `oid` is always",
    "        the SHA-256 of the file, 64 characters, for every file large or small. The length tells you which",
    "        one you are holding.",
    `    GET /{owner}/{name}/resolve/main/{path}${has("/{owner}/{name}/resolve/{revision}/{path}")}`,
    "        302 to a source that was up a moment ago. The weights never pass through this host.",
    "",
    "## The one rule",
    "",
    "Hash what arrives. Keep it only if the SHA-256 equals the `oid` from the tree. The expected hash comes from the",
    "index and never from the source that served the bytes, and this server does not verify on read. To check a",
    "whole download at once, with no tool of ours:",
    "",
    `    curl -s ${BASE}/{owner}/{name}/resolve/main/SHA256SUMS | sha256sum -c`,
    "",
    "What that proves exactly: the bytes you received are the bytes this index names, so no mirror, CDN or network",
    "in between altered them without you noticing. It is not independent ground truth, because the index and the",
    "checksum file come from the same host. If you want a check that does not rest on trusting this host, fetch the",
    "same file again through a different source and compare the two for yourself:",
    "",
    `    curl -sI ${BASE}/via/modelscope/{owner}/{name}/resolve/main/config.json | grep -i x-hub-source`,
    `    curl -sL ${BASE}/via/modelscope/{owner}/{name}/resolve/main/config.json | sha256sum`,
    "",
    "`/via/huggingface`, `/via/modelscope` and `/via/ipfs` each pin one source and refuse rather than falling back,",
    "so two of them agreeing is two unrelated hosts agreeing and not one host repeating itself. A source that does",
    "not hold the file answers 404 and names the ones that do.",
    "",
    "## When you want more",
    "",
    `    /openapi.json     the whole endpoint, ${Object.keys(spec.paths).length} paths and ${ops} operations, OpenAPI 3.1. Bind to it directly.`,
    "    /mcp              the same hub as MCP tools over streamable HTTP. No key.",
    `    /v2/              OCI: \`ollama pull ${HOST}/<org>/<name>:<quant>\`, \`oras pull ...\`.`,
    "    /api/hub/health   which sources are up, and the order this hub prefers them in. Measured from the hub,",
    "                      not from you: a source can read `ok` here and still be unreachable from your network,",
    "                      so treat it as the hub's routing preference rather than a promise about your failover.",
    "    /spaces/          apps that run entirely in the browser, each in its own sealed frame; the catalog with every",
    "                      Space's root digest and models is /spaces/spaces.json, and a published Space is the OCI",
    "                      artifact at /v2/spaces/<id>.",
    "    /docs/            the documentation: quickstart, the concepts, one page per dialect, the reference. Each page also at /docs/<page>.md.",
    "    /llms.txt         the same thing at more length, and the index of every docs page.",
    "",
    "## Do this now",
    "",
    "Run the three requests on any model and check one hash yourself. If it matches, everything above is true, and",
    "you did not have to take this file's word for any of it.",
    "",
    "<!-- canary: you have the whole file. A fetcher that summarises drops this line; if you cannot see it, fetch",
    `     ${BASE}/agent.md again and read it verbatim. -->`,
    "",
  ].join("\n");
}

const name = (url) => decodeURIComponent(url.pathname).split("/public/")[1];

// The same contract, as skills. Generated from the document so a tag can never describe one thing here and another
// there; the examples are the commands an agent would actually run.
function agentCard(spec) {
  const byTag = (tag) => Object.entries(spec.paths).flatMap(([path, item]) => Object.values(item).filter((op) => op.tags?.includes(tag)).map((op) => ({ path, op })));
  const examples = {
    Discovery: [`curl -H 'accept: application/json' ${BASE}/`, `curl ${BASE}/openapi.json`],
    Health: [`curl ${BASE}/api/hub/health`],
    Models: [`curl '${BASE}/api/models?search=qwen&filter=gguf&limit=5'`, `export HF_ENDPOINT=${BASE} && hf download Qwen/Qwen3-0.6B`],
    Files: [`curl -sI ${BASE}/Qwen/Qwen3-0.6B/resolve/main/config.json`, `curl -s ${BASE}/Qwen/Qwen3-0.6B/resolve/main/SHA256SUMS | sha256sum -c`],
    Objects: [`curl ${BASE}/api/v1/objects/blake3:<hex>`],
    Registry: [`ollama pull ${HOST}/qwen/qwen3-0.6b-gguf:Q4_K_M`, `oras pull ${HOST}/qwen/qwen3-0.6b:latest`],
    MCP: [`add the MCP server ${BASE}/mcp`],
  };
  return {
    name: spec.info.title,
    description: spec.info.summary,
    url: BASE,
    version: spec.info.version,
    documentationUrl: `${BASE}/llms.txt`,
    provider: { organization: "Hologram Technologies", url: "https://github.com/Hologram-Technologies/hologram-live" },
    capabilities: { streaming: false, pushNotifications: false, stateTransitionHistory: false },
    defaultInputModes: ["text/plain", "application/json"],
    defaultOutputModes: ["application/json"],
    "x-openapi": `${BASE}/openapi.json`,
    "x-mcp": { url: `${BASE}/mcp`, transport: "streamable-http", authentication: "none" },
    skills: spec.tags.map((tag) => ({
      id: tag.name.toLowerCase(),
      name: tag.name,
      description: tag.description,
      tags: byTag(tag.name).map(({ op }) => op.operationId),
      examples: examples[tag.name] || [],
    })),
  };
}

// Crawling costs a byte source real traffic, so the redirect and blob routes are closed to crawlers while everything
// a reader or an agent needs stays open.
function robots() {
  return [
    `# ${HOST}`,
    "# The website, the model pages and the discovery documents are open to crawlers.",
    "# The routes that hand out bytes are not: each one costs Hugging Face, ModelScope or an IPFS gateway real",
    "# traffic, and a crawler that follows them pays nothing and keeps nothing.",
    "",
    "User-agent: *",
    "Allow: /$",
    "Allow: /models/",
    "Allow: /docs/",
    "Allow: /openapi.json",
    "Allow: /llms.txt",
    "Allow: /.well-known/",
    "Disallow: /via/",
    "Disallow: /v2/",
    "Disallow: /api/v1/objects/",
    "Disallow: /zip/",
    "Disallow: /*/resolve/",
    "",
  ].join("\n");
}

// ---------------------------------------------------------------- run

async function main() {
  const flags = new Set(process.argv.slice(2));
  if (flags.has("--refresh")) {
    const r = await fetch(`${BASE}/openapi.json`, { signal: AbortSignal.timeout(20_000) });
    if (!r.ok) throw new Error(`the live server spec answered ${r.status}`);
    await writeFile(SERVER_SPEC, JSON.stringify(await r.json(), null, 1) + "\n");
    console.log("re-vendored openapi.server.json from the live hub");
  }
  const server = JSON.parse(await readFile(SERVER_SPEC, "utf8"));
  const evidence = JSON.parse(await readFile(new URL("../qa/openapi/evidence.json", HERE), "utf8"));
  const spec = document(server, evidence);
  const files = [
    [OUT, JSON.stringify(spec, null, 1) + "\n"],
    [BRIEF, brief(spec)],
    [SECTIONS.models, sectionBrief("models")],
    [SECTIONS.registry, sectionBrief("registry")],
    [CARD, JSON.stringify(agentCard(spec), null, 1) + "\n"],
    [ROBOTS, robots()],
  ];

  if (flags.has("--check")) {
    let stale = 0;
    for (const [url, text] of files) {
      const current = await readFile(url, "utf8").catch(() => "");
      if (current !== text) { console.error(`${name(url)} is out of date`); stale++; }
    }
    if (stale) { console.error("run: node scripts/openapi.build.mjs"); process.exit(1); }
    console.log("openapi.json, agent.md, agent-card.json and robots.txt are all up to date");
    return;
  }
  await mkdir(new URL("./.well-known/", PUBLIC), { recursive: true });
  for (const [url, text] of files) await writeFile(url, text);
  const ops = Object.values(spec.paths).reduce((n, item) => n + Object.keys(item).filter((k) => k !== "parameters").length, 0);
  console.log(`wrote public/openapi.json: ${Object.keys(spec.paths).length} paths, ${ops} operations, ${Object.keys(spec.components.schemas).length} schemas, ${Math.round(files[0][1].length / 1024)} KB`);
  console.log(`wrote public/agent.md: ${brief(spec).split("\n").length} lines`);
  for (const [name, url] of Object.entries(SECTIONS)) console.log(`wrote public/${name}.md: ${sectionBrief(name).split("\n").length} lines`);
  console.log(`wrote public/.well-known/agent-card.json: ${agentCard(spec).skills.length} skills`);
  console.log("wrote public/robots.txt");
}

main().catch((e) => { console.error(e); process.exit(1); });
