import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import "./styles.css";

type Action = "service_start" | "service_stop" | "service_restart";
type ServiceState = "ready" | "stopped" | "unknown";
type Theme = "light" | "dark";
type View = "console" | "chat" | "files" | "holo" | "modules";

type HealthInfo = {
  status: string;
  version: string;
  role: string;
  modules_ready: number;
};

type SystemInfo = {
  host: string;
  cores: number;
  memory_used_bytes: number;
  memory_total_bytes: number;
  disk_used_bytes: number;
  disk_total_bytes: number;
};

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

type WatchedHoloProject = {
  id: string;
  name: string;
  directory: string;
  manifest: string;
  status: "watching" | "compiling" | "ready" | "failed";
  archive_kappa?: string;
  archive_name?: string;
  last_compiled_at_millis?: number;
  error?: string;
};

type HoloLayer = {
  position: number;
  kind: string;
  content_kappa: string;
  entry: string;
  architecture: string | null;
  surface: string | null;
  engine: string | null;
};

type HoloDirectory = {
  schema_version: number;
  primary_layer: number | null;
  requires_kappa: string;
  layers: HoloLayer[];
  children: unknown[];
  blobs: { kappa: string; byte_length: number }[];
};

type HoloInspection = {
  kappa: string;
  application_kappa?: string;
  name: string;
  format_version: number;
  byte_length: number;
  archive_fingerprint: string;
  footer_verified: boolean;
  sections: { kind: string; offset: number; length: number }[];
  directory: HoloDirectory | null;
  directory_embedded: boolean;
};

type ConversationMessage = {
  role: string;
  content: string;
  created_at_millis: number;
};

type Conversation = {
  id: string;
  title: string;
  created_at_millis: number;
  updated_at_millis: number;
  messages: ConversationMessage[];
  archived?: boolean;
};

const notice = document.querySelector<HTMLDivElement>("#notice")!;
const modules = document.querySelector<HTMLDivElement>("#modules")!;
const objects = document.querySelector<HTMLDivElement>("#objects")!;
const navModuleCount = document.querySelector<HTMLElement>("#nav-module-count")!;
const navObjectCount = document.querySelector<HTMLElement>("#nav-object-count")!;
const navHoloCount = document.querySelector<HTMLElement>("#nav-holo-count")!;
const navThreadCount = document.querySelector<HTMLElement>("#nav-thread-count")!;
const content = document.querySelector<HTMLElement>(".content")!;
const threads = document.querySelector<HTMLDivElement>("#threads")!;
const threadSearch = document.querySelector<HTMLInputElement>("#thread-search")!;
const chatTitle = document.querySelector<HTMLElement>("#chat-title")!;
const chatDetail = document.querySelector<HTMLElement>("#chat-detail")!;
const chatEmpty = document.querySelector<HTMLDivElement>("#chat-empty")!;
const chatScroll = document.querySelector<HTMLDivElement>("#chat-scroll")!;
const messages = document.querySelector<HTMLDivElement>("#messages")!;
const chatForm = document.querySelector<HTMLFormElement>("#chat-form")!;
const chatInput = document.querySelector<HTMLTextAreaElement>("#chat-input")!;
const serviceMeta = document.querySelector<HTMLElement>("#service-meta")!;
const pageDescription = document.querySelector<HTMLElement>("#page-description")!;
const roleText = document.querySelector<HTMLElement>("#role-text")!;
const apiPill = document.querySelector<HTMLElement>("#api-pill")!;
const appVersion = document.querySelector<HTMLElement>("#app-version")!;
const systemDevice = document.querySelector<HTMLElement>("#system-device")!;
const storeBytes = document.querySelector<HTMLElement>("#store-bytes")!;
const storeCaption = document.querySelector<HTMLElement>("#store-caption")!;
const modulesReady = document.querySelector<HTMLElement>("#modules-ready")!;
const operationsCount = document.querySelector<HTMLElement>("#operations-count")!;
const tileThreads = document.querySelector<HTMLElement>("#tile-threads")!;
const tileMessages = document.querySelector<HTMLElement>("#tile-messages")!;
const metricObjects = document.querySelector<HTMLElement>("#metric-objects")!;
const metricModules = document.querySelector<HTMLElement>("#metric-modules")!;
const metricThreads = document.querySelector<HTMLElement>("#metric-threads")!;
const consoleModules = document.querySelector<HTMLElement>("#console-modules")!;
const sidebarCollapse = document.querySelector<HTMLButtonElement>("#sidebar-collapse")!;
const themeToggle = document.querySelector<HTMLButtonElement>("#theme-toggle")!;
const themeIcon = document.querySelector<HTMLElement>("#theme-icon")!;
const themeLabel = document.querySelector<HTMLElement>("#theme-label")!;
const holoWatches = document.querySelector<HTMLDivElement>("#holo-watches")!;
const holoArchives = document.querySelector<HTMLDivElement>("#holo-archives")!;
const holoInspector = document.querySelector<HTMLElement>("#holo-inspector")!;
let currentState: ServiceState = "unknown";
let health: HealthInfo | null = null;
let apiAddress: string | null = null;
let isBusy = false;
let noticeTimer: number | undefined;
let themePreference = storedTheme();
let conversations: Conversation[] = [];
let activeThreadId: string | null = null;
let chatBusy = false;
let archiveExpanded = false;
let catalogedHolos: HoloInspection[] = [];
let watchedHoloProjects: WatchedHoloProject[] = [];
let activeHoloKappa: string | null = null;

function storedTheme(): Theme | null {
  try {
    const value = window.localStorage.getItem("hologram-theme");
    return value === "light" || value === "dark" ? value : null;
  } catch {
    return null;
  }
}

function applyTheme(theme: Theme, remember = false) {
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  const nextTheme = theme === "light" ? "dark" : "light";
  const nextLabel = nextTheme === "dark" ? "Dark mode" : "Light mode";
  themeIcon.textContent = theme === "light" ? "☾" : "☀";
  themeLabel.textContent = nextLabel;
  themeToggle.setAttribute("aria-label", `Use ${nextLabel.toLowerCase()}`);
  themeToggle.title = `Use ${nextLabel.toLowerCase()}`;
  document.querySelector<HTMLMetaElement>('meta[name="theme-color"]')!.content = theme === "light" ? "#f6f7f9" : "#0b0d0f";
  if (remember) {
    themePreference = theme;
    try {
      window.localStorage.setItem("hologram-theme", theme);
    } catch {
      // The selected theme still applies for this session.
    }
  }
}

// Dark is the default look; the system preference only applies once the user opts into light.
applyTheme(themePreference ?? "dark");

function syncControls() {
  document.querySelectorAll<HTMLButtonElement>(".command-button").forEach((button) => {
    const chatAction = button.id === "send-message" || button.id === "new-thread";
    button.disabled = isBusy || (chatBusy && chatAction) || (button.dataset.when !== undefined && button.dataset.when !== currentState);
  });
  chatInput.disabled = currentState !== "ready" || chatBusy;
  threadSearch.disabled = currentState !== "ready";
}

function setBusy(busy: boolean) {
  isBusy = busy;
  syncControls();
}

function setState(value: ServiceState) {
  currentState = value;
  document.querySelectorAll<HTMLElement>(".state-dot").forEach((dot) => {
    dot.className = `state-dot ${value}`;
  });
  document.querySelectorAll<HTMLElement>(".state-label").forEach((label) => {
    label.textContent = value === "ready" ? "Ready" : value === "stopped" ? "Stopped" : "Checking";
  });
  roleText.textContent = value === "ready" ? (health?.role ?? "node") : value === "stopped" ? "stopped" : "checking";
  renderIdentity();
  syncControls();
}

function showNotice(message: string, tone: "neutral" | "success" | "error" = "neutral", persistent = false) {
  window.clearTimeout(noticeTimer);
  // Only failures interrupt. Routine progress is already visible in the status pills,
  // and a toast over the composer was covering the send button.
  if (tone !== "error") {
    notice.classList.add("is-hidden");
    return;
  }
  notice.textContent = message;
  notice.className = `notice ${tone}`;
  if (!persistent) {
    noticeTimer = window.setTimeout(() => notice.classList.add("is-hidden"), 5000);
  }
}

function showView(view: View) {
  document.querySelectorAll<HTMLElement>("[data-page]").forEach((page) => {
    page.hidden = page.dataset.page !== view;
  });
  document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((item) => {
    const active = item.dataset.view === view;
    item.classList.toggle("active", active);
    active ? item.setAttribute("aria-current", "page") : item.removeAttribute("aria-current");
  });
  content.classList.toggle("chat-content", view === "chat");
  if (view === "chat" && currentState === "ready") {
    window.setTimeout(() => chatInput.focus(), 0);
  }
}

async function execute(action: Action) {
  setBusy(true);
  const present = action === "service_start" ? "Starting" : action === "service_stop" ? "Stopping" : "Restarting";
  showNotice(`${present} Hologram…`, "neutral", true);
  try {
    await invoke<string>(action);
    if (action === "service_stop") {
      setState("stopped");
      renderModules([], true);
      renderObjects([], true);
      renderChatUnavailable();
      showNotice("Hologram has stopped.", "success");
    } else {
      await refresh();
      showNotice(action === "service_start" ? "Hologram is ready." : "Hologram restarted successfully.", "success");
    }
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
    setState("stopped");
  } finally {
    setBusy(false);
  }
}

async function refresh() {
  try {
    const status = await invoke<string>("service_status");
    try {
      health = JSON.parse(status) as HealthInfo;
    } catch {
      health = null;
    }
    setState("ready");
    const results = await Promise.all([refreshModules(), refreshObjects(), refreshHoloWorkspace(), refreshChatThreads(true)]);
    showNotice(results.every(Boolean) ? "Everything is up to date." : "Some items couldn’t be loaded.", results.every(Boolean) ? "success" : "error");
  } catch {
    health = null;
    setState("stopped");
    showNotice("Start Hologram to use your local workspace.", "neutral");
    renderModules([], true);
    renderObjects([], true);
    renderHoloWorkspace([], [], true);
    renderChatUnavailable();
  }
}

async function refreshHoloWorkspace() {
  try {
    const [watches, archives] = await Promise.all([
      invoke<WatchedHoloProject[]>("holo_watch_list"),
      invoke<string>("holo_catalog_list"),
    ]);
    watchedHoloProjects = watches;
    catalogedHolos = JSON.parse(archives) as HoloInspection[];
    if (activeHoloKappa !== null && !catalogedHolos.some((item) => item.kappa === activeHoloKappa)) {
      activeHoloKappa = null;
      renderHoloInspector(null);
    }
    renderHoloWorkspace(watchedHoloProjects, catalogedHolos);
    return true;
  } catch (error) {
    renderHoloWorkspace(watchedHoloProjects, [], true);
    showNotice(friendlyError(error), "error", true);
    return false;
  }
}

async function addHoloDirectory() {
  const path = await open({ multiple: false, directory: true, title: "Add a Hologram application directory" });
  if (path === null) return;
  setBusy(true);
  try {
    await invoke<WatchedHoloProject>("holo_watch_add", { path });
    showView("holo");
    await refreshHoloWorkspace();
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setBusy(false);
  }
}

async function removeHoloWatch(project: WatchedHoloProject) {
  setBusy(true);
  try {
    await invoke<void>("holo_watch_remove", { id: project.id });
    await refreshHoloWorkspace();
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setBusy(false);
  }
}

async function inspectHolo(kappa: string) {
  activeHoloKappa = kappa;
  renderHoloArchives(catalogedHolos);
  holoInspector.innerHTML = '<div class="holo-inspector-loading"><div class="loading-row"><span></span><span></span></div></div>';
  try {
    const result = await invoke<string>("holo_catalog_inspect", { kappa });
    if (activeHoloKappa === kappa) renderHoloInspector(JSON.parse(result) as HoloInspection);
  } catch (error) {
    if (activeHoloKappa === kappa) {
      holoInspector.innerHTML = `<div class="holo-inspector-empty"><span class="empty-symbol" aria-hidden="true">!</span><strong>Inspection failed</strong><p>${escapeHtml(friendlyError(error))}</p></div>`;
    }
  }
}

async function refreshObjects() {
  try {
    const result = await invoke<string>("objects_list");
    renderObjects(JSON.parse(result) as ObjectMetadata[]);
    return true;
  } catch (error) {
    setObjectCount("—");
    objects.innerHTML = emptyState("!", "Files couldn’t be loaded", "Check that Hologram is ready, then try again.", "Try again", "retry-objects");
    objects.querySelector(".retry-objects")!.addEventListener("click", () => void refreshObjects());
    syncControls();
    showNotice(friendlyError(error), "error", true);
    return false;
  }
}

async function uploadFile() {
  const path = await open({ multiple: false, directory: false, title: "Add a file to Hologram" });
  if (path === null) return;
  setBusy(true);
  showNotice("Adding your file…", "neutral", true);
  try {
    await invoke<string>("file_put", { path });
    await refreshObjects();
    showNotice("File added successfully.", "success");
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setBusy(false);
  }
}

async function downloadObject(item: ObjectMetadata) {
  const path = await save({ defaultPath: safeFilename(item), title: "Save file" });
  if (path === null) return;
  setBusy(true);
  showNotice("Saving your file…", "neutral", true);
  try {
    await invoke<string>("object_get", { id: item.id, output: path });
    showNotice("File saved successfully.", "success");
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setBusy(false);
  }
}

async function refreshModules() {
  try {
    const result = await invoke<string>("modules_list");
    renderModules(JSON.parse(result) as ModuleInfo[]);
    return true;
  } catch (error) {
    setModuleCount("—");
    modules.innerHTML = emptyState("!", "Modules couldn’t be loaded", "Check that Hologram is ready, then try again.", "Try again", "retry-modules");
    modules.querySelector(".retry-modules")!.addEventListener("click", () => void refreshModules());
    syncControls();
    showNotice(friendlyError(error), "error", true);
    return false;
  }
}

async function refreshChatThreads(selectFirst = false) {
  try {
    const result = await invoke<string>("history_list");
    conversations = JSON.parse(result) as Conversation[];
    const unarchived = conversations.filter((item) => item.archived !== true);
    setThreadCount(String(unarchived.length));
    tileMessages.textContent = String(unarchived.reduce((total, item) => total + item.messages.length, 0));
    if (activeThreadId !== null && !conversations.some((item) => item.id === activeThreadId)) {
      activeThreadId = null;
    }
    if (activeThreadId === null && selectFirst && unarchived.length > 0) {
      activeThreadId = unarchived[0].id;
    }
    renderThreadList();
    const active = conversations.find((item) => item.id === activeThreadId);
    if (active !== undefined) renderConversation(active);
    else renderNewChat();
    return true;
  } catch (error) {
    setThreadCount("—");
    tileMessages.textContent = "—";
    threads.innerHTML = '<div class="thread-empty">Chats couldn’t be loaded.</div>';
    renderChatUnavailable("Chat history couldn’t be loaded.");
    showNotice(friendlyError(error), "error", true);
    return false;
  }
}

function threadRow(conversation: Conversation) {
  const lastMessage = conversation.messages.at(-1)?.content.replace(/\s+/g, " ").trim() || "No messages yet";
  const archived = conversation.archived === true;
  return `<div class="thread-row${conversation.id === activeThreadId ? " active" : ""}">
    <button class="thread-item" data-thread-id="${escapeHtml(conversation.id)}" type="button">
      <span class="thread-glyph" aria-hidden="true">◌</span>
      <span><strong>${escapeHtml(conversation.title)}</strong><small>${escapeHtml(lastMessage)}</small></span>
      <time>${escapeHtml(formatThreadTime(conversation.updated_at_millis))}</time>
    </button>
    <button class="thread-archive command-button" data-when="ready" data-archive-id="${escapeHtml(conversation.id)}" data-archived="${archived}" type="button"
      title="${archived ? "Restore chat" : "Archive chat"}" aria-label="${archived ? "Restore chat" : "Archive chat"}">${archived ? "↩" : "▤"}</button>
  </div>`;
}

function renderThreadList() {
  const query = threadSearch.value.trim().toLocaleLowerCase();
  const matches = conversations.filter((item) => item.title.toLocaleLowerCase().includes(query));
  const active = matches.filter((item) => item.archived !== true);
  const archived = matches.filter((item) => item.archived === true);

  if (matches.length === 0) {
    threads.innerHTML = `<div class="thread-empty">${query === "" ? "No chats yet." : "No matching chats."}</div>`;
    return;
  }

  const activeMarkup = active.length > 0
    ? active.map(threadRow).join("")
    : `<div class="thread-empty">${query === "" ? "No active chats." : "No matching active chats."}</div>`;
  const archivedMarkup = archived.length === 0
    ? ""
    : `<div class="thread-group">
        <button id="archive-toggle" class="thread-group-head" type="button" aria-expanded="${archiveExpanded}">
          <span class="thread-group-caret" aria-hidden="true">${archiveExpanded ? "⌄" : "›"}</span>
          <span>ARCHIVED</span>
          <span class="thread-group-count">${archived.length}</span>
        </button>
        ${archiveExpanded ? archived.map(threadRow).join("") : ""}
      </div>`;
  threads.innerHTML = activeMarkup + archivedMarkup;

  threads.querySelectorAll<HTMLButtonElement>(".thread-item").forEach((button) => {
    button.addEventListener("click", () => void selectThread(button.dataset.threadId!));
  });
  threads.querySelectorAll<HTMLButtonElement>(".thread-archive").forEach((button) => {
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      void setThreadArchived(button.dataset.archiveId!, button.dataset.archived !== "true");
    });
  });
  threads.querySelector<HTMLButtonElement>("#archive-toggle")?.addEventListener("click", () => {
    archiveExpanded = !archiveExpanded;
    renderThreadList();
  });
  syncControls();
}

async function setThreadArchived(id: string, archived: boolean) {
  setBusy(true);
  try {
    await invoke<string>("history_archive", { id, archived });
    if (archived && activeThreadId === id) {
      activeThreadId = null;
    }
    await refreshChatThreads(archived);
    showNotice(archived ? "Chat archived." : "Chat restored.", "success");
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setBusy(false);
  }
}

async function selectThread(id: string) {
  if (chatBusy || id === activeThreadId) return;
  setChatBusy(true);
  try {
    const result = await invoke<string>("history_get", { id });
    const conversation = JSON.parse(result) as Conversation;
    activeThreadId = id;
    const index = conversations.findIndex((item) => item.id === id);
    if (index >= 0) conversations[index] = conversation;
    renderThreadList();
    renderConversation(conversation);
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setChatBusy(false);
  }
}

function startNewThread() {
  if (currentState !== "ready" || chatBusy) return;
  activeThreadId = null;
  threadSearch.value = "";
  renderThreadList();
  renderNewChat();
  chatInput.focus();
}

function renderNewChat() {
  chatTitle.textContent = "New chat";
  chatDetail.textContent = "Messages are saved to this thread";
  messages.hidden = true;
  messages.innerHTML = "";
  setChatWelcome(
    "Chat with Hologram",
    "Start a conversation. Hologram answers with the inference engine configured on the server and saves both sides to this thread.",
    "Conversations stay in your local history.",
  );
}

function renderConversation(conversation: Conversation, pending = false) {
  chatTitle.textContent = conversation.title;
  chatDetail.textContent = `${conversation.messages.length} saved message${conversation.messages.length === 1 ? "" : "s"}`;
  if (conversation.messages.length === 0) {
    messages.hidden = true;
    setChatWelcome(
      "Start this conversation",
      "Send a message and the configured inference engine will answer.",
      "This thread is stored in your local history.",
    );
    return;
  }
  chatEmpty.hidden = true;
  messages.hidden = false;
  messages.innerHTML = conversation.messages.map((message) => messageMarkup(message)).join("") + (pending
    ? '<article class="message assistant pending"><span class="message-author">Hologram</span><div class="typing" aria-label="Hologram is responding"><i></i><i></i><i></i></div></article>'
    : "");
  window.requestAnimationFrame(() => {
    chatScroll.scrollTop = chatScroll.scrollHeight;
  });
}

function messageMarkup(message: ConversationMessage) {
  const role = message.role === "user" ? "user" : "assistant";
  const author = role === "user" ? "You" : "Hologram";
  return `<article class="message ${role}">
    <span class="message-author">${author}</span>
    <div>${escapeHtml(message.content)}</div>
    <time>${escapeHtml(formatMessageTime(message.created_at_millis))}</time>
  </article>`;
}

function setChatWelcome(title: string, copy: string, status: string) {
  chatEmpty.hidden = false;
  chatEmpty.querySelector("h2")!.textContent = title;
  chatEmpty.querySelector("p")!.textContent = copy;
  chatEmpty.querySelector("div")!.lastChild!.textContent = ` ${status}`;
}

function renderChatUnavailable(copy = "Start Hologram to load your chat threads.") {
  conversations = [];
  activeThreadId = null;
  setThreadCount("—");
  threads.innerHTML = '<div class="thread-empty">Hologram is stopped.</div>';
  chatTitle.textContent = "Chat";
  chatDetail.textContent = "Conversation history is unavailable";
  messages.hidden = true;
  messages.innerHTML = "";
  setChatWelcome("Chat is unavailable", copy, "Your saved threads will appear when Hologram is ready.");
  syncControls();
}

async function sendChatMessage() {
  const text = chatInput.value.trim();
  if (text === "" || currentState !== "ready" || chatBusy) return;
  setChatBusy(true);
  chatInput.value = "";
  resizeComposer();
  showNotice("Sending message…", "neutral", true);
  try {
    let conversation = conversations.find((item) => item.id === activeThreadId);
    if (conversation === undefined) {
      const result = await invoke<string>("history_create", { title: threadTitle(text) });
      conversation = JSON.parse(result) as Conversation;
      activeThreadId = conversation.id;
      conversations.unshift(conversation);
      setThreadCount(String(conversations.length));
      renderThreadList();
    }
    const optimistic: Conversation = {
      ...conversation,
      messages: [...conversation.messages, { role: "user", content: text, created_at_millis: Date.now() }],
    };
    renderConversation(optimistic, true);
    const result = await sendChatWithCompatibilityRecovery(conversation.id, text);
    const updated = JSON.parse(result) as Conversation;
    activeThreadId = updated.id;
    renderConversation(updated);
    await refreshChatThreads();
    showNotice("Reply saved to this thread.", "success");
  } catch (error) {
    chatInput.value = text;
    resizeComposer();
    showNotice(friendlyError(error), "error", true);
    const active = conversations.find((item) => item.id === activeThreadId);
    if (active !== undefined) renderConversation(active);
    else renderNewChat();
  } finally {
    setChatBusy(false);
    chatInput.focus();
  }
}

async function sendChatWithCompatibilityRecovery(id: string, content: string) {
  try {
    return await invoke<string>("chat_send", { id, content });
  } catch (error) {
    if (!isMissingChatCapability(error)) throw error;
    showNotice("Updating the local chat demo…", "neutral", true);
    await invoke<string>("service_restart");
    setState("ready");
    await refreshModules();
    return invoke<string>("chat_send", { id, content });
  }
}

function isMissingChatCapability(error: unknown) {
  const message = String(error).replaceAll("\\", "");
  return message.includes("LIVE_CAPABILITY_MISSING") && message.includes("chat.send");
}

function setChatBusy(busy: boolean) {
  chatBusy = busy;
  threads.setAttribute("aria-busy", String(busy));
  syncControls();
}

function resizeComposer() {
  chatInput.style.height = "auto";
  chatInput.style.height = `${Math.min(chatInput.scrollHeight, 150)}px`;
}

function threadTitle(message: string) {
  const title = message.replace(/\s+/g, " ").trim();
  return title.length <= 48 ? title : `${title.slice(0, 47).trimEnd()}…`;
}

function formatThreadTime(value: number) {
  const date = new Date(value);
  const today = new Date();
  return date.toDateString() === today.toDateString()
    ? date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
    : date.toLocaleDateString([], { month: "short", day: "numeric" });
}

function formatMessageTime(value: number) {
  return new Date(value).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function renderModules(items: ModuleInfo[], unavailable = false) {
  setModuleCount(unavailable ? "—" : String(items.length));
  const ready = items.filter((module) => module.state === "ready").length;
  modulesReady.textContent = unavailable ? "—" : `${ready} / ${items.length}`;
  operationsCount.textContent = unavailable
    ? "—"
    : String(items.reduce((total, module) => total + module.operations.length, 0));
  renderConsoleModules(items, unavailable);
  if (items.length === 0) {
    modules.innerHTML = unavailable
      ? emptyState("◇", "Hologram is stopped", "Start Hologram to see its available modules.", "Go to Console", "open-console")
      : emptyState("◇", "No modules are enabled", "Enabled modules will appear here when they are available.");
    modules.querySelector(".open-console")?.addEventListener("click", () => showView("console"));
    syncControls();
    return;
  }
  modules.innerHTML = items.map((module) => `<article class="module-row">
    <div class="module-description">
      <div><strong>${escapeHtml(module.name)}</strong><span class="module-state">${escapeHtml(module.state)}</span></div>
      <code>${escapeHtml(module.id)}</code>
      <p>${module.operations.length} operation${module.operations.length === 1 ? "" : "s"} · Version ${escapeHtml(module.version)}</p>
    </div>
  </article>`).join("");
}

function renderObjects(items: ObjectMetadata[], unavailable = false) {
  setObjectCount(unavailable ? "—" : String(items.length));
  const bytes = items.reduce((total, item) => total + item.size, 0);
  storeBytes.textContent = unavailable ? "—" : formatBytes(bytes);
  storeCaption.textContent = unavailable
    ? "content-addressed objects"
    : `across ${items.length} object${items.length === 1 ? "" : "s"}`;
  if (items.length === 0) {
    objects.innerHTML = unavailable
      ? emptyState("□", "Hologram is stopped", "Start Hologram to browse your local files.", "Go to Console", "open-console")
      : emptyState("+", "Add your first file", "Files you add to Hologram will appear here.", "Add file", "empty-upload");
    objects.querySelector(".open-console")?.addEventListener("click", () => showView("console"));
    objects.querySelector(".empty-upload")?.addEventListener("click", () => void uploadFile());
    syncControls();
    return;
  }
  objects.innerHTML = items.map((item, index) => `<article class="object-row">
    <div class="object-description">
      <div><strong>${escapeHtml(item.filename ?? "Unnamed file")}</strong><span class="kind">${escapeHtml(item.kind)}</span></div>
      <code title="${escapeHtml(item.id)}">${escapeHtml(item.id)}</code>
      <p>${escapeHtml(item.media_type)} · ${formatBytes(item.size)}</p>
    </div>
    <div class="object-actions">
      <button class="secondary command-button rename-object" data-when="ready" data-index="${index}">Rename</button>
      <button class="secondary command-button download-object" data-when="ready" data-index="${index}">Download</button>
    </div>
  </article>`).join("");
  objects.querySelectorAll<HTMLButtonElement>(".rename-object").forEach((button) => {
    button.addEventListener("click", () => beginObjectRename(items, Number(button.dataset.index)));
  });
  objects.querySelectorAll<HTMLButtonElement>(".download-object").forEach((button) => {
    button.addEventListener("click", () => void downloadObject(items[Number(button.dataset.index)]));
  });
  syncControls();
}

function renderHoloWorkspace(watches: WatchedHoloProject[], archives: HoloInspection[], unavailable = false) {
  navHoloCount.textContent = unavailable ? "—" : String(archives.length);
  renderHoloWatches(watches, unavailable);
  renderHoloArchives(archives, unavailable);
}

function renderHoloWatches(items: WatchedHoloProject[], unavailable = false) {
  if (items.length === 0) {
    holoWatches.innerHTML = unavailable
      ? emptyState("◇", "Hologram is stopped", "Start Hologram to compile watched application projects.", "Go to Console", "open-console")
      : emptyState("+", "Add an application directory", "Choose a directory containing hologram.json. Hologram will compile it now and whenever its files change.", "Add directory", "empty-add-holo");
    holoWatches.querySelector(".open-console")?.addEventListener("click", () => showView("console"));
    holoWatches.querySelector(".empty-add-holo")?.addEventListener("click", () => void addHoloDirectory());
    syncControls();
    return;
  }
  holoWatches.innerHTML = items.map((project, index) => {
    const detail = project.status === "failed"
      ? project.error ?? "Compilation failed"
      : project.status === "compiling"
        ? "Compiling and importing changes…"
        : project.last_compiled_at_millis === undefined
          ? "Waiting for the first build"
          : `Last built ${formatThreadTime(project.last_compiled_at_millis)}`;
    return `<article class="holo-watch-row">
      <span class="watch-state ${escapeHtml(project.status)}" aria-hidden="true"></span>
      <div class="holo-watch-description">
        <div><strong>${escapeHtml(project.name)}</strong><span class="watch-status ${escapeHtml(project.status)}">${escapeHtml(project.status)}</span></div>
        <code title="${escapeHtml(project.directory)}">${escapeHtml(project.directory)}</code>
        <p class="${project.status === "failed" ? "watch-error" : ""}">${escapeHtml(detail)}</p>
      </div>
      <button class="secondary command-button stop-watch" data-index="${index}" type="button">Stop watching</button>
    </article>`;
  }).join("");
  holoWatches.querySelectorAll<HTMLButtonElement>(".stop-watch").forEach((button) => {
    button.addEventListener("click", () => void removeHoloWatch(items[Number(button.dataset.index)]));
  });
  syncControls();
}

function renderHoloArchives(items: HoloInspection[], unavailable = false) {
  if (items.length === 0) {
    holoArchives.innerHTML = unavailable
      ? emptyState("◇", "Applications couldn’t be loaded", "Start Hologram to browse the local .holo catalog.")
      : emptyState("◇", "No .holo applications yet", "Add a watched directory or import an archive with the CLI.");
    return;
  }
  holoArchives.innerHTML = items.map((item, index) => {
    const watched = watchedHoloProjects.find((project) => project.archive_kappa === item.kappa);
    const title = watched?.name ?? item.name;
    const layers = item.directory?.layers.length ?? 0;
    return `<button class="holo-archive-row${item.kappa === activeHoloKappa ? " active" : ""}" data-index="${index}" type="button">
      <span class="holo-archive-mark" aria-hidden="true">H</span>
      <span class="holo-archive-description">
        <span><strong>${escapeHtml(title)}</strong><small>.holo v${item.format_version}</small></span>
        <code title="${escapeHtml(item.kappa)}">${escapeHtml(item.kappa)}</code>
        <small>${formatBytes(item.byte_length)} · ${layers} layer${layers === 1 ? "" : "s"} · ${item.footer_verified ? "verified" : "unverified"}</small>
      </span>
      <span class="inspect-caret" aria-hidden="true">›</span>
    </button>`;
  }).join("");
  holoArchives.querySelectorAll<HTMLButtonElement>(".holo-archive-row").forEach((button) => {
    button.addEventListener("click", () => void inspectHolo(items[Number(button.dataset.index)].kappa));
  });
}

function renderHoloInspector(item: HoloInspection | null) {
  if (item === null) {
    holoInspector.innerHTML = `<div class="holo-inspector-empty">
      <span class="empty-symbol" aria-hidden="true">◇</span>
      <strong>Select an application</strong>
      <p>Inspect its verified identities, layers, capabilities, sections, and embedded blobs.</p>
    </div>`;
    return;
  }
  const directory = item.directory;
  const layers = directory?.layers.map((layer) => `<article class="inspect-layer">
    <div><strong>${layer.position}. ${escapeHtml(layer.kind)}</strong>${directory.primary_layer === layer.position ? '<span class="kind">primary</span>' : ""}</div>
    <p>${escapeHtml(layer.entry)}${layer.architecture === null ? "" : ` · ${escapeHtml(layer.architecture)}`}${layer.surface === null ? "" : ` · ${escapeHtml(layer.surface)}`}${layer.engine === null ? "" : ` · ${escapeHtml(layer.engine)}`}</p>
    <code>${escapeHtml(layer.content_kappa)}</code>
  </article>`).join("") ?? '<p class="inspect-muted">No application directory.</p>';
  const sections = item.sections.map((section) => `<li><span>${escapeHtml(section.kind)}</span><code>${section.offset} + ${section.length}</code></li>`).join("");
  holoInspector.innerHTML = `<div class="holo-inspector-head">
      <div><span class="kind">verified archive</span><h2>${escapeHtml(item.name)}</h2></div>
      <span class="verified-mark" title="Footer verified">${item.footer_verified ? "✓" : "!"}</span>
    </div>
    <dl class="inspect-facts">
      <div><dt>Archive</dt><dd><code>${escapeHtml(item.kappa)}</code></dd></div>
      <div><dt>Application</dt><dd><code>${escapeHtml(item.application_kappa ?? "not present")}</code></dd></div>
      <div><dt>Fingerprint</dt><dd><code>${escapeHtml(item.archive_fingerprint)}</code></dd></div>
      <div><dt>Format</dt><dd>v${item.format_version} · ${formatBytes(item.byte_length)}</dd></div>
      <div><dt>Directory</dt><dd>${directory === null ? "derived / unavailable" : `schema ${directory.schema_version} · ${item.directory_embedded ? "embedded" : "derived"}`}</dd></div>
      <div><dt>Capabilities</dt><dd><code>${escapeHtml(directory?.requires_kappa ?? "not present")}</code></dd></div>
    </dl>
    <section class="inspect-section"><h3>Layers</h3><div class="inspect-layers">${layers}</div></section>
    <section class="inspect-section"><h3>Physical sections</h3><ul class="inspect-sections">${sections}</ul></section>
    <p class="inspect-summary">${directory?.blobs.length ?? 0} embedded blob${directory?.blobs.length === 1 ? "" : "s"} · ${directory?.children.length ?? 0} child application${directory?.children.length === 1 ? "" : "s"}</p>`;
}

function beginObjectRename(items: ObjectMetadata[], index: number) {
  const item = items[index];
  const row = objects.querySelectorAll<HTMLElement>(".object-row")[index];
  if (item === undefined || row === undefined) return;
  const description = row.querySelector<HTMLElement>(".object-description")!;
  const currentName = item.filename ?? "";
  description.innerHTML = `<form class="rename-form">
    <label class="visually-hidden" for="rename-file-${index}">File name</label>
    <input id="rename-file-${index}" maxlength="255" autocomplete="off" placeholder="Enter a file name" value="${escapeHtml(currentName)}" />
    <button class="primary command-button rename-save" data-when="ready" type="submit">Save</button>
    <button class="secondary command-button rename-cancel" data-when="ready" type="button">Cancel</button>
  </form>
  <code title="${escapeHtml(item.id)}">${escapeHtml(item.id)}</code>
  <p>${escapeHtml(item.media_type)} · ${formatBytes(item.size)}</p>`;
  row.querySelector<HTMLElement>(".object-actions")!.hidden = true;
  const form = row.querySelector<HTMLFormElement>(".rename-form")!;
  const input = form.querySelector<HTMLInputElement>("input")!;
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void renameObject(item, input.value);
  });
  form.querySelector<HTMLButtonElement>(".rename-cancel")!.addEventListener("click", () => renderObjects(items));
  input.addEventListener("keydown", (event) => {
    if (event.key === "Escape") renderObjects(items);
  });
  input.focus();
  input.select();
}

async function renameObject(item: ObjectMetadata, filename: string) {
  const nextName = filename.trim();
  if (nextName.length === 0) {
    showNotice("Enter a file name.", "error", true);
    return;
  }
  setBusy(true);
  showNotice("Renaming your file…", "neutral", true);
  try {
    await invoke<string>("file_rename", { id: item.id, filename: nextName });
    await refreshObjects();
    showNotice(`Renamed to ${nextName}.`, "success");
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setBusy(false);
  }
}

function setModuleCount(value: string) {
  navModuleCount.textContent = value;
  metricModules.textContent = value;
}

function setObjectCount(value: string) {
  navObjectCount.textContent = value;
  metricObjects.textContent = value;
}

function setThreadCount(value: string) {
  navThreadCount.textContent = value;
  metricThreads.textContent = value;
  tileThreads.textContent = value;
}

function renderIdentity() {
  const parts = currentState === "ready" && health !== null
    ? [health.role, `v${health.version}`, apiAddress].filter((part): part is string => Boolean(part))
    : [];
  serviceMeta.textContent = parts.length > 0 ? parts.join(" · ") : "local module host";
  appVersion.textContent = health !== null ? `v${health.version}` : "—";
  pageDescription.textContent = currentState === "ready" && health !== null
    ? `${health.role} · ${health.modules_ready} module${health.modules_ready === 1 ? "" : "s"} ready${apiAddress === null ? "" : ` · ${apiAddress}`}`
    : currentState === "stopped"
      ? "Start Hologram to use your local workspace"
      : "Checking your local workspace…";
}

// The address is parsed out of `config show`, which emits TOML rather than JSON.
async function refreshApiAddress() {
  try {
    const config = await invoke<string>("config_show");
    apiAddress = /^\s*listen\s*=\s*"([^"]+)"/m.exec(config)?.[1] ?? null;
  } catch {
    apiAddress = null;
  }
  apiPill.textContent = apiAddress === null ? "API —" : `API ${apiAddress}`;
  renderIdentity();
}

async function refreshSystemInfo() {
  try {
    const info = await invoke<SystemInfo>("system_info");
    setMeter("disk", info.disk_used_bytes, info.disk_total_bytes);
    setMeter("memory", info.memory_used_bytes, info.memory_total_bytes);
    systemDevice.textContent = info.cores > 0 ? `${info.host} · ${info.cores} cores` : info.host;
  } catch {
    systemDevice.textContent = "This device";
  }
}

function setMeter(name: string, used: number, total: number) {
  const meter = document.querySelector<HTMLElement>(`.meter[data-meter="${name}"]`);
  if (meter === null) return;
  const value = meter.querySelector<HTMLElement>(".meter-value")!;
  const fill = meter.querySelector<HTMLElement>(".meter-fill")!;
  if (total <= 0) {
    value.textContent = "—";
    // Width is set through CSSOM: the content security policy forbids style attributes.
    fill.style.width = "0%";
    return;
  }
  value.textContent = `${formatBytes(used)} / ${formatBytes(total)}`;
  fill.style.width = `${Math.min(100, Math.round((used / total) * 100))}%`;
}

function renderConsoleModules(items: ModuleInfo[], unavailable = false) {
  if (unavailable || items.length === 0) {
    consoleModules.innerHTML = `<p class="preset-empty">${unavailable ? "Start Hologram to see its modules." : "No modules are enabled."}</p>`;
    return;
  }
  consoleModules.innerHTML = items.slice(0, 3).map((module) => `<article class="preset-card">
    <div><strong>${escapeHtml(module.name)}</strong><span class="module-state">${escapeHtml(module.state)}</span></div>
    <p>${module.operations.length} operation${module.operations.length === 1 ? "" : "s"} · v${escapeHtml(module.version)}</p>
    <code>${escapeHtml(module.id)}</code>
  </article>`).join("");
}

function emptyState(symbol: string, title: string, copy: string, action?: string, actionClass?: string) {
  return `<article class="empty-state">
    <span class="empty-symbol" aria-hidden="true">${escapeHtml(symbol)}</span>
    <strong>${escapeHtml(title)}</strong>
    <p>${escapeHtml(copy)}</p>
    ${action === undefined ? "" : `<button class="secondary command-button ${actionClass}" data-when="${actionClass === "empty-upload" ? "ready" : currentState}">${escapeHtml(action)}</button>`}
  </article>`;
}

function safeFilename(item: ObjectMetadata) {
  const name = item.filename?.split(/[\\/]/).pop();
  return name || item.id.replace(":", "_");
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

function friendlyError(error: unknown) {
  const message = String(error).replace(/daemon/gi, "Hologram").replace(/^Error:\s*/i, "");
  return message.trim() || "Hologram couldn’t complete that action.";
}

function escapeHtml(value: string) {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;",
  })[character]!);
}

document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((button) => {
  button.addEventListener("click", () => showView(button.dataset.view as View));
});
document.querySelectorAll<HTMLButtonElement>(".open-view").forEach((button) => {
  button.addEventListener("click", () => showView(button.dataset.target as View));
});
document.querySelector("#start")!.addEventListener("click", () => void execute("service_start"));
document.querySelector("#restart")!.addEventListener("click", () => void execute("service_restart"));
document.querySelector("#stop")!.addEventListener("click", () => void execute("service_stop"));
document.querySelector("#refresh-modules")!.addEventListener("click", () => void refreshModules());
document.querySelector("#refresh-objects")!.addEventListener("click", () => void refreshObjects());
document.querySelector("#upload-file")!.addEventListener("click", () => void uploadFile());
document.querySelector("#refresh-holo")!.addEventListener("click", () => void refreshHoloWorkspace());
document.querySelector("#add-holo-directory")!.addEventListener("click", () => void addHoloDirectory());
document.querySelector("#new-thread")!.addEventListener("click", startNewThread);
threadSearch.addEventListener("input", renderThreadList);
chatForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void sendChatMessage();
});
chatInput.addEventListener("input", resizeComposer);
chatInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
    event.preventDefault();
    chatForm.requestSubmit();
  }
});
themeToggle.addEventListener("click", () => {
  const current = document.documentElement.dataset.theme === "dark" ? "dark" : "light";
  applyTheme(current === "light" ? "dark" : "light", true);
});

void listen<ServiceState>("service-state-changed", (event) => {
  const previous = currentState;
  setState(event.payload);
  if (event.payload === "stopped") {
    renderModules([], true);
    renderObjects([], true);
    renderHoloWorkspace([], [], true);
    renderChatUnavailable();
    showNotice("Hologram has stopped.", "neutral");
  } else if (!isBusy && previous !== "ready") {
    void Promise.all([refreshModules(), refreshObjects(), refreshHoloWorkspace(), refreshChatThreads(true)]);
    showNotice("Hologram is ready.", "success");
  }
});

void listen("holo-watch-changed", () => {
  if (currentState === "ready") void refreshHoloWorkspace();
});

// --- Text size -------------------------------------------------------------
// One custom property scales every font size; layout stays in pixels.
const TEXT_SCALES = [0.9, 1, 1.1, 1.25, 1.4, 1.6];
const textSmaller = document.querySelector<HTMLButtonElement>("#text-smaller")!;
const textLarger = document.querySelector<HTMLButtonElement>("#text-larger")!;
let textScaleIndex = storedTextScaleIndex();

function storedTextScaleIndex() {
  try {
    const value = Number(window.localStorage.getItem("hologram-text-scale"));
    const index = TEXT_SCALES.indexOf(value);
    return index === -1 ? TEXT_SCALES.indexOf(1.1) : index;
  } catch {
    return TEXT_SCALES.indexOf(1.1);
  }
}

function applyTextScale(index: number, announce = false) {
  textScaleIndex = Math.min(TEXT_SCALES.length - 1, Math.max(0, index));
  const scale = TEXT_SCALES[textScaleIndex];
  document.documentElement.style.setProperty("--text-scale", String(scale));
  textSmaller.disabled = textScaleIndex === 0;
  textLarger.disabled = textScaleIndex === TEXT_SCALES.length - 1;
  try {
    window.localStorage.setItem("hologram-text-scale", String(scale));
  } catch {
    // The size still applies for this session.
  }
  if (announce) showNotice(`Text size ${Math.round(scale * 100)}%.`, "neutral");
}

textSmaller.addEventListener("click", () => applyTextScale(textScaleIndex - 1, true));
textLarger.addEventListener("click", () => applyTextScale(textScaleIndex + 1, true));

// --- Command palette -------------------------------------------------------
type Command = {
  id: string;
  label: string;
  hint: string;
  run: () => void;
  enabled?: () => boolean;
};

const palette = document.querySelector<HTMLDivElement>("#palette")!;
const paletteScrim = document.querySelector<HTMLDivElement>("#palette-scrim")!;
const paletteInput = document.querySelector<HTMLInputElement>("#palette-input")!;
const paletteList = document.querySelector<HTMLDivElement>("#palette-list")!;
let paletteIndex = 0;
let paletteMatches: Command[] = [];

const commands: Command[] = [
  { id: "console", label: "Go to Console", hint: "View", run: () => showView("console") },
  { id: "chat", label: "Go to Chat", hint: "View", run: () => showView("chat") },
  { id: "files", label: "Go to Files", hint: "View", run: () => showView("files") },
  { id: "holo", label: "Go to Applications", hint: "View", run: () => showView("holo") },
  { id: "modules", label: "Go to Modules", hint: "View", run: () => showView("modules") },
  {
    id: "new-chat",
    label: "New chat",
    hint: "Chat",
    enabled: () => currentState === "ready",
    run: () => {
      showView("chat");
      renderNewChat();
    },
  },
  {
    id: "archive-active",
    label: "Archive current chat",
    hint: "Chat",
    enabled: () => currentState === "ready" && activeThreadId !== null,
    run: () => void setThreadArchived(activeThreadId!, true),
  },
  {
    id: "add-file",
    label: "Add file",
    hint: "Files",
    enabled: () => currentState === "ready",
    run: () => void uploadFile(),
  },
  {
    id: "add-holo-directory",
    label: "Add application directory",
    hint: "Applications",
    enabled: () => currentState === "ready",
    run: () => void addHoloDirectory(),
  },
  { id: "refresh", label: "Refresh everything", hint: "Service", run: () => void refresh() },
  {
    id: "start",
    label: "Start Hologram",
    hint: "Service",
    enabled: () => currentState === "stopped",
    run: () => void execute("service_start"),
  },
  {
    id: "restart",
    label: "Restart Hologram",
    hint: "Service",
    enabled: () => currentState === "ready",
    run: () => void execute("service_restart"),
  },
  {
    id: "stop",
    label: "Stop Hologram",
    hint: "Service",
    enabled: () => currentState === "ready",
    run: () => void execute("service_stop"),
  },
  {
    id: "theme",
    label: "Toggle light / dark",
    hint: "Appearance",
    run: () => applyTheme(document.documentElement.dataset.theme === "dark" ? "light" : "dark", true),
  },
  { id: "text-larger", label: "Larger text", hint: "Appearance", run: () => applyTextScale(textScaleIndex + 1, true) },
  { id: "text-smaller", label: "Smaller text", hint: "Appearance", run: () => applyTextScale(textScaleIndex - 1, true) },
  {
    id: "sidebar",
    label: "Toggle sidebar",
    hint: "Appearance",
    run: () => sidebarCollapse.click(),
  },
];

function renderPalette() {
  const query = paletteInput.value.trim().toLocaleLowerCase();
  paletteMatches = commands.filter(
    (command) =>
      (command.enabled === undefined || command.enabled()) &&
      (query === "" || command.label.toLocaleLowerCase().includes(query) || command.hint.toLocaleLowerCase().includes(query)),
  );
  paletteIndex = Math.min(paletteIndex, Math.max(0, paletteMatches.length - 1));
  if (paletteMatches.length === 0) {
    paletteList.innerHTML = '<p class="palette-empty">No matching commands.</p>';
    return;
  }
  paletteList.innerHTML = paletteMatches
    .map(
      (command, index) => `<button class="palette-item${index === paletteIndex ? " active" : ""}" role="option"
        aria-selected="${index === paletteIndex}" data-index="${index}" type="button">
        <span>${escapeHtml(command.label)}</span><small>${escapeHtml(command.hint)}</small>
      </button>`,
    )
    .join("");
  paletteList.querySelectorAll<HTMLButtonElement>(".palette-item").forEach((button) => {
    button.addEventListener("click", () => runPalette(Number(button.dataset.index)));
  });
}

function openPalette() {
  paletteInput.value = "";
  paletteIndex = 0;
  palette.hidden = false;
  renderPalette();
  paletteInput.focus();
}

function closePalette() {
  palette.hidden = true;
}

function runPalette(index: number) {
  const command = paletteMatches[index];
  closePalette();
  command?.run();
}

paletteInput.addEventListener("input", () => {
  paletteIndex = 0;
  renderPalette();
});
paletteScrim.addEventListener("click", closePalette);
paletteInput.addEventListener("keydown", (event) => {
  if (event.key === "ArrowDown") {
    event.preventDefault();
    paletteIndex = Math.min(paletteMatches.length - 1, paletteIndex + 1);
    renderPalette();
  } else if (event.key === "ArrowUp") {
    event.preventDefault();
    paletteIndex = Math.max(0, paletteIndex - 1);
    renderPalette();
  } else if (event.key === "Enter") {
    event.preventDefault();
    runPalette(paletteIndex);
  }
});

window.addEventListener("keydown", (event) => {
  const accel = event.metaKey || event.ctrlKey;
  if (accel && event.key.toLowerCase() === "k") {
    event.preventDefault();
    palette.hidden ? openPalette() : closePalette();
    return;
  }
  if (event.key === "Escape" && !palette.hidden) {
    event.preventDefault();
    closePalette();
    return;
  }
  if (!accel) return;
  if (event.key === "=" || event.key === "+") {
    event.preventDefault();
    applyTextScale(textScaleIndex + 1, true);
  } else if (event.key === "-" || event.key === "_") {
    event.preventDefault();
    applyTextScale(textScaleIndex - 1, true);
  } else if (event.key === "0") {
    event.preventDefault();
    applyTextScale(TEXT_SCALES.indexOf(1.1), true);
  }
});

sidebarCollapse.addEventListener("click", () => {
  const collapsed = document.body.classList.toggle("sidebar-collapsed");
  sidebarCollapse.setAttribute("aria-label", collapsed ? "Expand sidebar" : "Collapse sidebar");
  sidebarCollapse.title = collapsed ? "Expand sidebar" : "Collapse sidebar";
  try {
    window.localStorage.setItem("hologram-sidebar", collapsed ? "collapsed" : "expanded");
  } catch {
    // The choice still applies for this session.
  }
});

try {
  if (window.localStorage.getItem("hologram-sidebar") === "collapsed") {
    document.body.classList.add("sidebar-collapsed");
  }
} catch {
  // Default to the expanded sidebar.
}

applyTextScale(textScaleIndex);
setState("unknown");
void refreshApiAddress();
void refreshSystemInfo();
window.setInterval(() => void refreshSystemInfo(), 5000);
void refresh();
