import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import "./styles.css";

type Action = "daemon_start" | "daemon_stop" | "daemon_restart" | "daemon_status";

type ModuleInfo = {
  id: string;
  name: string;
  version: string;
  state: string;
  dependencies: string[];
  operations: string[];
};

type ObjectMetadata = {
  id: string;
  kind: string;
  media_type: string;
  filename: string | null;
  size: number;
  created_at_millis: number;
};

const output = document.querySelector<HTMLPreElement>("#output")!;
const state = document.querySelector<HTMLSpanElement>("#state")!;
const modules = document.querySelector<HTMLDivElement>("#modules")!;
const objects = document.querySelector<HTMLDivElement>("#objects")!;

function setBusy(busy: boolean) {
  document.querySelectorAll<HTMLButtonElement>("button").forEach((button) => {
    button.disabled = busy;
  });
}

function setState(value: "ready" | "stopped" | "unknown") {
  state.className = `state ${value}`;
  state.textContent = value === "ready" ? "Running" : value === "stopped" ? "Stopped" : "Checking";
}

async function execute(action: Action) {
  setBusy(true);
  output.textContent = "Working…";
  try {
    const result = await invoke<string>(action);
    output.textContent = result.trim() || "Command completed.";
    if (action === "daemon_stop") {
      setState("stopped");
    } else {
      await refresh();
    }
  } catch (error) {
    output.textContent = String(error);
    setState("stopped");
  } finally {
    setBusy(false);
  }
}

async function refresh() {
  try {
    const result = await invoke<string>("daemon_status");
    output.textContent = result.trim();
    setState("ready");
    await Promise.all([refreshModules(), refreshObjects()]);
  } catch (error) {
    output.textContent = String(error);
    setState("stopped");
    renderModules([]);
    renderObjects([]);
  }
}

async function refreshObjects() {
  try {
    const result = await invoke<string>("objects_list");
    renderObjects(JSON.parse(result) as ObjectMetadata[]);
  } catch (error) {
    objects.innerHTML = `<p class="empty">${escapeHtml(String(error))}</p>`;
  }
}

async function uploadFile() {
  const path = await open({ multiple: false, directory: false, title: "Store a file in Hologram" });
  if (path === null) return;
  setBusy(true);
  output.textContent = `Uploading ${path}…`;
  try {
    const result = await invoke<string>("file_put", { path });
    output.textContent = result.trim();
    await refreshObjects();
  } catch (error) {
    output.textContent = String(error);
  } finally {
    setBusy(false);
  }
}

async function downloadObject(item: ObjectMetadata) {
  const path = await save({
    defaultPath: safeFilename(item),
    title: "Save Hologram object",
  });
  if (path === null) return;
  setBusy(true);
  output.textContent = `Downloading ${item.id}…`;
  try {
    output.textContent = (await invoke<string>("object_get", { id: item.id, output: path })).trim();
  } catch (error) {
    output.textContent = String(error);
  } finally {
    setBusy(false);
  }
}

async function refreshModules() {
  try {
    const result = await invoke<string>("modules_list");
    renderModules(JSON.parse(result) as ModuleInfo[]);
  } catch (error) {
    modules.innerHTML = `<p class="empty">${escapeHtml(String(error))}</p>`;
  }
}

function renderModules(items: ModuleInfo[]) {
  if (items.length === 0) {
    modules.innerHTML = '<p class="empty">No modules are available.</p>';
    return;
  }
  modules.innerHTML = items
    .map(
      (module) => `<article class="module-card">
        <div><span class="module-state">${escapeHtml(module.state)}</span><h3>${escapeHtml(module.name)}</h3></div>
        <code>${escapeHtml(module.id)}</code>
        <p>${module.operations.length} operation${module.operations.length === 1 ? "" : "s"} · v${escapeHtml(module.version)}</p>
      </article>`,
    )
    .join("");
}

function renderObjects(items: ObjectMetadata[]) {
  if (items.length === 0) {
    objects.innerHTML = '<p class="empty">No objects are stored yet.</p>';
    return;
  }
  objects.innerHTML = items
    .map(
      (item, index) => `<article class="object-row">
        <div class="object-description">
          <div><strong>${escapeHtml(item.filename ?? "Unnamed object")}</strong><span class="kind">${escapeHtml(item.kind)}</span></div>
          <code title="${escapeHtml(item.id)}">${escapeHtml(item.id)}</code>
          <p>${escapeHtml(item.media_type)} · ${formatBytes(item.size)}</p>
        </div>
        <button class="secondary download-object" data-index="${index}">Download</button>
      </article>`,
    )
    .join("");
  objects.querySelectorAll<HTMLButtonElement>(".download-object").forEach((button) => {
    button.addEventListener("click", () => void downloadObject(items[Number(button.dataset.index)]));
  });
}

function safeFilename(item: ObjectMetadata) {
  const name = item.filename?.split(/[\\/]/).pop();
  return name || item.id.replace(":", "_");
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function escapeHtml(value: string) {
  return value.replace(/[&<>'"]/g, (character) => {
    const entities: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "'": "&#39;",
      '"': "&quot;",
    };
    return entities[character];
  });
}

document.querySelector("#start")!.addEventListener("click", () => execute("daemon_start"));
document.querySelector("#restart")!.addEventListener("click", () => execute("daemon_restart"));
document.querySelector("#stop")!.addEventListener("click", () => execute("daemon_stop"));
document.querySelector("#refresh-modules")!.addEventListener("click", () => void refreshModules());
document.querySelector("#refresh-objects")!.addEventListener("click", () => void refreshObjects());
document.querySelector("#upload-file")!.addEventListener("click", () => void uploadFile());

void refresh();
