// Simulated host for previewing the View. It answers in the primary's protocol (src/hub.rs) with invented data
// kept in this browser's sessionStorage, and says so on the page.
//
// Scenarios: Qwen/Qwen3.8-27B starts on the node; any other model pulls in about four seconds; a model whose
// reference contains "kokoro" fails verification with a mismatched layer; "llama" is not in the registry.

(() => {
  const KEY = "model-hub-preview.node";
  const hex = (seed) => Array.from({ length: 64 }, (_, i) => ((seed * 31 + i * 7) % 16).toString(16)).join("");
  const load = () => { try { return JSON.parse(sessionStorage.getItem(KEY)); } catch { return null; } };
  const state = load() || {
    next: 1,
    jobs: {},
    artifacts: [{ reference: "localhost:5000/huggingface/qwen/qwen3.8-27b:1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0", manifest_digest: "sha256:" + hex(3), archive_kappa: "blake3:" + hex(1), layers: 33, bytes: 55_600_000_000, recorded_at_millis: Date.now() - 86_400_000 }],
  };
  const save = () => sessionStorage.setItem(KEY, JSON.stringify(state));
  const reply = (data) => ({ ok: true, data });

  function tick(job) {
    const done = Math.min(job.layers_total, Math.floor((Date.now() - job.started) / 700));
    job.layers_done = done;
    job.bytes_done = Math.round((job.bytes * done) / job.layers_total);
    if (job.state !== "running" || done < job.layers_total) return;
    if (job.kind === "verify" && job.reference.includes("kokoro")) {
      Object.assign(job, { state: "failed", layers_done: 2, mismatch: job.archive.replace(/.{6}$/, "c0ffee"), error: "layer_mismatch" });
    } else if (job.kind === "pull" && job.reference.includes("llama")) {
      Object.assign(job, { state: "failed", layers_done: 0, error: "not_found" });
    } else {
      job.state = "succeeded";
      if (job.kind === "pull") state.artifacts.unshift({ reference: `localhost:5000/${job.reference}`, manifest_digest: "sha256:" + hex(state.next + 9), archive_kappa: "blake3:" + hex(state.next + 20), layers: job.layers_total, bytes: job.bytes, recorded_at_millis: Date.now() });
    }
  }

  window.modelHubInvoke = async (request) => {
    await new Promise((resolve) => setTimeout(resolve, 80));
    try {
      switch (request.op) {
        case "library":
          return reply({ artifacts: state.artifacts, next_cursor: null });
        case "pull": {
          if (/\s/.test(request.reference ?? "")) return { ok: false, error: "invalid_reference" };
          const id = `job-${state.next++}`;
          state.jobs[id] = { id, kind: "pull", state: "running", reference: request.reference, started: Date.now(), layers_total: 6, bytes: 4_200_000_000, mismatch: null, error: null };
          return reply({ job: id });
        }
        case "verify": {
          const artifact = state.artifacts.find((a) => a.archive_kappa === request.archive);
          if (!artifact) return { ok: false, error: "not_found" };
          const id = `job-${state.next++}`;
          state.jobs[id] = { id, kind: "verify", state: "running", reference: artifact.reference, archive: artifact.archive_kappa, started: Date.now(), layers_total: Math.min(artifact.layers, 6), bytes: artifact.bytes, mismatch: null, error: null };
          return reply({ job: id });
        }
        case "status": {
          const job = state.jobs[request.job];
          if (!job) return { ok: false, error: "not_found" };
          tick(job);
          const { id, kind, state: s, layers_done, layers_total, bytes_done, mismatch, error } = job;
          return reply({ id, kind, state: s, layers_done, layers_total, bytes_done, mismatch, error });
        }
        case "remove":
          state.artifacts = state.artifacts.filter((a) => a.archive_kappa !== request.archive);
          return reply(null);
        default:
          return { ok: false, error: "invalid_request" };
      }
    } finally {
      save();
    }
  };

  addEventListener("DOMContentLoaded", () => {
    const note = document.createElement("p");
    note.textContent = "Preview with a simulated host. Nothing is pulled or verified.";
    note.className = "preview-note";
    note.style.cssText = "margin:0;padding:var(--space-2) var(--space-4);text-align:center;font-size:var(--text-sm);background:var(--hh-surface);color:var(--hh-text)";
    document.body.prepend(note);
  });
})();
