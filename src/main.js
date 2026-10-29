const { invoke } = window.__TAURI__.core;

const OS_NAME = { linux: "Linux", windows: "Windows", android: "Android", macos: "macOS" };
const KIND = { appimage: "AppImage", deb: ".deb", rpm: ".rpm", installer: "Installer", msi: "MSI", portable: "Portable", exe: ".exe", zip: "Zip", apk: "APK" };

const $ = (id) => document.getElementById(id);
const root = document.documentElement;
let device = null;
let apps = [];
let settings = null;
let releases = {}; // app id -> { tag, builds, error }
const busy = new Set();
const norm = (v) => (v || "").replace(/^v/i, "");

function say(text, isErr = false) {
  $("msg").textContent = text || "";
  $("msg").className = isErr ? "err" : "";
}

function el(tag, props = {}, ...kids) {
  const e = Object.assign(document.createElement(tag), props);
  kids.forEach((k) => e.append(k));
  return e;
}

function button(label, cls, onclick, disabled = false) {
  return el("button", { className: cls, textContent: label, onclick, disabled, type: "button" });
}

// ---------- device bar ----------

function renderDevice() {
  const os = OS_NAME[device.os] || device.os;
  const bits = [device.arch];
  if (device.ram_gb) bits.push(device.ram_gb + " GB RAM");
  if (device.gpus.length) bits.push(device.gpus.join(" + ") + " graphics");
  $("devname").textContent = "Detected: " + os;
  $("devspec").textContent = bits.join(" · ");
}

function renderSummary() {
  if (!apps.length || Object.keys(releases).length < apps.length) {
    $("devnote").textContent = "";
    return;
  }
  if (lastNetwork?.internet === "offline") {
    $("devnote").textContent = "Offline · what you already have still works";
    return;
  }
  const n = apps.filter((a) => (releases[a.id]?.builds || []).length || a.installed).length;
  $("devnote").textContent = n + " of " + apps.length + " apps have a build for this device";
}

// ---------- tiles ----------

/** A newer release exists than the one installed. */
function updateAvailable(app) {
  const latest = releases[app.id]?.tag;
  return !!(app.installed && latest && app.version && norm(latest) !== norm(app.version));
}

/** The user chose to skip exactly this release. A newer one brings the prompt back. */
function updateSkipped(app) {
  const latest = releases[app.id]?.tag;
  return !!(latest && app.ignored_version && norm(app.ignored_version) === norm(latest));
}

/** Show the Update prompt: newer release, and not skipped. */
function updatePrompt(app) {
  return updateAvailable(app) && !updateSkipped(app);
}

// Simple original line symbols, one per app, drawn inside the glass bubble (Look: Glass icons).
const ICON_PATHS = {
  "hop-tunes": '<path d="M9 18V6l10-2v12"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="16.5" cy="16" r="2.5"/>',
  "hopcast": '<circle cx="12" cy="12" r="2"/><path d="M7.8 7.8a6 6 0 0 0 0 8.4M16.2 7.8a6 6 0 0 1 0 8.4M4.9 4.9a10 10 0 0 0 0 14.2M19.1 4.9a10 10 0 0 1 0 14.2"/>',
  "alias-maker": '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M7 9l3 3-3 3M12 15h5"/>',
  "anti-social": '<circle cx="12" cy="12" r="9"/><path d="M5.6 5.6l12.8 12.8"/>',
  "sudo-make-art": '<path d="M12 3a9 9 0 1 0 0 18c1.5 0 2-1 1.6-2.2-.5-1.3.4-2.3 1.7-2.3H17a4 4 0 0 0 4-4C21 6.6 17 3 12 3z"/><circle cx="7.5" cy="11" r="1"/><circle cx="10.5" cy="7.5" r="1"/><circle cx="15.5" cy="7.5" r="1"/>',
  "sudo-make-music": '<path d="M4 12v0M8 8v8M12 5v14M16 8v8M20 11v2"/>',
};

/** A clear glass bubble holding the app's symbol (or its first letter), tinted with the app's own colour. */
function glassIcon(app) {
  const bubble = el("span", { className: "gico" });
  bubble.setAttribute("aria-hidden", "true");
  const paths = ICON_PATHS[app.id];
  if (paths) {
    bubble.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${paths}</svg>`;
  } else {
    bubble.append(el("b", { textContent: (app.name || "?").trim().charAt(0).toUpperCase() }));
  }
  return bubble;
}

/** In the slot of an app that can no longer be fetched (and is not installed): a short message instead. */
function goneTile(app) {
  const t = el("section", { className: "tile gone off" }, el("p", { textContent: "I'm sorry, Dave. I can't help you." }));
  t.dataset.id = app.id;
  t.title = `${app.name} is no longer available`;
  t.setAttribute("aria-label", `${app.name} is no longer available. I'm sorry, Dave. I can't help you.`);
  return t;
}

function tile(app) {
  const rel = releases[app.id];
  // Only an app the user does NOT have is replaced. An installed app always keeps its tile and Launch button.
  if (rel?.source === "gone" && !app.installed) return goneTile(app);
  const working = busy.has(app.id);
  const os = OS_NAME[device.os] || device.os;
  const builds = rel?.builds || [];
  const latest = rel?.tag;
  const prompt = updatePrompt(app);
  const skipped = updateAvailable(app) && updateSkipped(app);
  const inPlace = settings?.existing_mode === "in_place";
  const untested = (app.untested_on || []).includes(device.os) ? " · untested" : "";

  const t = el("section", { className: "tile" });
  t.dataset.id = app.id;
  if (app.accent) t.style.setProperty("--a", app.accent);

  const grip = el("button", { className: "grip", type: "button", title: "Drag to move, or use the arrow keys" });
  grip.setAttribute("aria-label", `Move ${app.name}. Drag it, or press the arrow keys.`);
  const glass = look === "glass";
  const top = el("div", { className: "top" }, grip, ...(glass ? [glassIcon(app)] : []), el("div", { className: "titles" }, el("h2", { textContent: app.name, title: app.name }), el("div", { className: "tag", textContent: app.tagline, title: app.tagline })));
  if (app.installed) {
    const word = app.adopted ? "Forget" : "Uninstall";
    // In the glass look the label is a compact cross (so full names still fit); the words are still there for hover and screen readers.
    const x = button(glass ? "✕" : word, "x", () => uninstall(app), working);
    if (glass) { x.title = word; x.setAttribute("aria-label", `${word} ${app.name}`); }
    top.append(x);
  }

  const sel = el("select");
  sel.setAttribute("aria-label", "Version");
  const btns = el("div", { className: "btns" });

  if (app.installed) {
    const mine = app.adopted ? "Your copy" : "Installed";
    const ver = app.version ? ` · ${app.version}` : "";
    const getUpdate = () => (app.adopted && inPlace
      ? invoke("open_repo", { id: app.id }).catch((e) => say(String(e), true))
      : update(app, builds[0].file));
    const canUpdate = !!builds.length || (app.adopted && inPlace);
    if (prompt && canUpdate) {
      // A new release: the main button becomes Update, running what is installed stays one click away.
      sel.add(new Option(`Update available · ${latest}`, "update"));
      sel.add(new Option(`Skip ${latest} (stop asking)`, "skip"));
      sel.onchange = async () => {
        if (sel.value !== "skip") return;
        try { await invoke("skip_update", { id: app.id, tag: latest }); say(`Skipped ${latest}. You will be asked again when a newer release appears.`); }
        catch (e) { say(String(e), true); }
        await loadApps();
      };
      sel.disabled = working;
      btns.append(button(working ? "Updating…" : app.adopted && inPlace ? "Get update" : "Update", "go", getUpdate, working));
      btns.append(button("Launch anyway", "go alt", () => launch(app), working));
    } else if (skipped && canUpdate) {
      // The user already said no to this release: Launch stays the main button, and they can change their mind.
      sel.add(new Option(`${mine}${ver} (skipped ${latest})`, "keep"));
      sel.add(new Option(`Update to ${latest}`, "update"));
      sel.onchange = () => { if (sel.value === "update") getUpdate(); };
      sel.disabled = working;
      btns.append(button("Launch", "go", () => launch(app), working));
    } else {
      // If the source is gone or failing, say so quietly: the app still launches, it just cannot update.
      const noUpdates = rel && (rel.source === "gone" || rel.source === "trouble");
      const label = noUpdates ? `${app.version || mine} · can't update` : mine + ver;
      sel.add(new Option(prompt ? `Update available · ${latest}` : label));
      if (noUpdates) sel.title = "This app's source is not available right now, so it can't be updated. It still launches.";
      sel.disabled = true;
      btns.append(button("Launch", "go", () => launch(app), working));
    }
  } else if (builds.length) {
    builds.forEach((b, i) => sel.add(new Option((i === 0 ? "Recommended: " : "") + KIND[b.kind] + untested, b.file)));
    sel.disabled = working;
    btns.append(button(working ? "Installing…" : "Install", "go", () => install(app, sel.value), working));
  } else {
    t.classList.add("off");
    const why = rel?.source === "offline" ? "You're offline" : rel?.source === "trouble" ? "Can't reach GitHub right now" : rel?.error;
    sel.add(new Option(why ? why : rel ? `No ${os} build yet` : "Checking GitHub…"));
    sel.disabled = true;
    btns.append(button(rel ? "Unavailable" : "Please wait", "go", null, true));
  }

  t.append(top, el("div", { className: "row" }, sel, btns));
  const c = counts[app.id];
  if (settings?.show_downloads && c) {
    // One exact total per app: every version and every system (Linux, Windows, Android ...) added together.
    const n = c.downloads.toLocaleString();
    const chip = el("span", { className: "dl", textContent: "▼ " + n });
    chip.title = `Downloaded ${n} times in total, across every version and system, according to GitHub.` + (c.at ? ` Read ${new Date(c.at * 1000).toLocaleString()}.` : "");
    chip.setAttribute("aria-label", `${n} downloads in total`);
    t.append(chip);
  }
  return t;
}

function render() {
  if (dragging) return; // never rebuild the tiles under the user's pointer
  $("grid").replaceChildren(...apps.map(tile));
  renderSummary();
}

// ---------- actions ----------

async function loadApps() {
  apps = await invoke("list_apps");
  render();
}

async function install(app, file) {
  busy.add(app.id);
  say(`Downloading ${app.name}…`);
  render();
  try {
    const r = await invoke("install_app", { id: app.id, file });
    if (r.handed_off) say(`${app.name} ${r.tag}: opened its installer. Finish the setup there.`);
    else say(`${app.name} ${r.tag} installed` + (r.verified ? " (checksum verified)" : " (no checksum was published)"));
  } catch (e) {
    say(String(e), true);
  } finally {
    busy.delete(app.id);
    await loadApps();
  }
}

async function update(app, file) {
  if (app.adopted && !confirm(`You are using your own copy of ${app.name}:\n${app.path}\n\nDownload the new version into the launcher's folder and switch to it? Your file stays where it is.`)) return;
  await install(app, file);
}

async function launch(app) {
  try {
    await invoke("launch_app", { id: app.id });
    say(`${app.name} launched. You can close the launcher; the app keeps running on its own.`);
  } catch (e) {
    say(String(e), true);
  }
}

async function uninstall(app) {
  const warn = device.os === "android"
    ? `Remove ${app.name}? Android will ask you to confirm.`
    : app.adopted
    ? `Stop using your copy of ${app.name} in the launcher? Your file is not deleted.`
    : app.folder
      ? `Uninstall ${app.name}? This deletes its program folder, including any settings stored inside it.`
      : `Uninstall ${app.name}? Your settings and data are kept.`;
  if (!confirm(warn)) return;
  try {
    await invoke("uninstall_app", { id: app.id });
    say(app.adopted ? `${app.name} forgotten. Your file was left alone.` : `${app.name} uninstalled`);
  } catch (e) {
    say(String(e), true);
  }
  await loadApps();
  renderSettings();
}

let lastNetwork = null; // what the corner indicator last learned: { internet, github }
const counts = {};       // app id -> { downloads, at }: the last download total GitHub (or the saved copy) gave

function rememberCounts(list) {
  list.forEach((r) => { if (r.downloads != null) counts[r.id] = { downloads: r.downloads, at: r.downloads_at }; });
}

const GH_LABEL = { none: ["ok", "operational"], minor: ["warn", "minor issues"], major: ["bad", "outage"], critical: ["bad", "outage"], maintenance: ["warn", "maintenance"], unknown: ["off", "unknown"] };

/** Look at the internet and GitHub's status. Nothing waits for this and nothing needs it to succeed. */
async function updateStatus() {
  try {
    lastNetwork = await invoke("network_status");
  } catch (e) {
    lastNetwork = null;
  }
  renderStatus();
}

function renderStatus() {
  const box = $("netstatus");
  const n = lastNetwork;
  if (!n || n.internet === "off") { box.hidden = true; return; }
  box.hidden = false;
  const net = $("pill-net"), gh = $("pill-gh");
  const online = n.internet === "online";
  net.className = "pill " + (online ? "ok" : "off");
  net.querySelector("b").textContent = online ? "Internet: online" : "Internet: offline";
  net.title = online ? "Your internet connection works." : "No internet connection. Everything you already have still works.";
  const [tone, word] = GH_LABEL[n.github.indicator] || GH_LABEL.unknown;
  gh.className = "pill " + tone;
  gh.querySelector("b").textContent = "GitHub: " + word;
  gh.title = (n.github.description || "GitHub's status") + " (click to open githubstatus.com)";
}
$("pill-gh").onclick = (e) => { e.preventDefault(); invoke("open_github_status_page").catch((err) => say(String(err), true)); };
window.addEventListener("online", updateStatus);
window.addEventListener("offline", updateStatus);
setInterval(updateStatus, 5 * 60 * 1000); // a quick look every five minutes while open

const CHECK_EVERY = 3 * 60 * 60 * 1000; // look for new releases every 3 hours while the launcher is open
let lastCheck = Date.now();

async function refresh() {
  lastCheck = Date.now();
  $("refresh").disabled = true;
  releases = {};
  render();
  say("Checking GitHub…");
  updateStatus(); // runs alongside; the tiles do not wait for it
  try {
    const found = await invoke("check_releases");
    rememberCounts(found);
    found.forEach((r) => (releases[r.id] = r));
    const updates = apps.filter(updatePrompt).length;
    const failed = Object.values(releases).filter((r) => r.error && r.error !== "No release published yet").length;
    const offline = lastNetwork?.internet === "offline";
    say(offline ? "You're offline. Everything you already have still works." : failed ? "Some apps could not be checked right now." : updates ? `${updates} update${updates > 1 ? "s" : ""} available` : "Everything is up to date", !!failed);
  } catch (e) {
    say(String(e), true);
  } finally {
    $("refresh").disabled = false;
    render();
  }
}

// ---------- reordering ----------

let dragging = null;

async function saveOrder() {
  try {
    await invoke("set_app_order", { order: apps.map((a) => a.id) });
    settings = await invoke("get_settings");
  } catch (e) {
    say(String(e), true);
  }
}

/** Moves one app a place earlier (-1) or later (+1), for keyboard users. */
function moveApp(id, delta) {
  const i = apps.findIndex((a) => a.id === id);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= apps.length) return;
  [apps[i], apps[j]] = [apps[j], apps[i]];
  render();
  $("grid").querySelector(`[data-id="${id}"] .grip`)?.focus();
  saveOrder();
}

$("grid").addEventListener("keydown", (e) => {
  const grip = e.target.closest?.(".grip");
  if (!grip) return;
  const id = grip.closest(".tile").dataset.id;
  if (e.key === "ArrowLeft" || e.key === "ArrowUp") { e.preventDefault(); moveApp(id, -1); }
  else if (e.key === "ArrowRight" || e.key === "ArrowDown") { e.preventDefault(); moveApp(id, 1); }
});

// Pointer events work the same for a mouse and for a finger on a phone.
$("grid").addEventListener("pointerdown", (e) => {
  const grip = e.target.closest(".grip");
  if (!grip || e.button > 0) return;
  e.preventDefault();
  const tile = grip.closest(".tile");
  const grid = $("grid");
  dragging = tile;
  tile.classList.add("dragging");

  const move = (ev) => {
    const edge = 48; // scroll the page when dragging near its top or bottom edge
    if (ev.clientY < edge) window.scrollBy(0, -16);
    else if (ev.clientY > innerHeight - edge) window.scrollBy(0, 16);
    const over = document.elementsFromPoint(ev.clientX, ev.clientY).find((n) => n.classList?.contains("tile") && n !== tile);
    if (!over) return;
    const kids = [...grid.children];
    if (kids.indexOf(tile) < kids.indexOf(over)) over.after(tile);
    else over.before(tile);
  };
  const end = () => {
    document.removeEventListener("pointermove", move);
    document.removeEventListener("pointerup", end);
    document.removeEventListener("pointercancel", end);
    tile.classList.remove("dragging");
    dragging = null;
    const ids = [...grid.children].map((t) => t.dataset.id);
    apps.sort((a, b) => ids.indexOf(a.id) - ids.indexOf(b.id));
    saveOrder();
  };
  document.addEventListener("pointermove", move);
  document.addEventListener("pointerup", end);
  document.addEventListener("pointercancel", end);
});

/** Looks for new releases without clearing the screen, and says so when one appears. */
async function quietCheck() {
  if (!device || busy.size || Date.now() - lastCheck < CHECK_EVERY) return;
  lastCheck = Date.now();
  updateStatus();
  const before = Object.fromEntries(Object.entries(releases).map(([id, r]) => [id, r.tag]));
  try {
    const found = await invoke("check_releases");
    rememberCounts(found);
    found.forEach((r) => {
      if (!r.error || !releases[r.id]?.tag) releases[r.id] = r; // a failed check keeps the last good answer
    });
    const fresh = apps.filter((a) => before[a.id] && releases[a.id]?.tag !== before[a.id] && updatePrompt(a));
    if (fresh.length) say(`New release: ${fresh.map((a) => `${a.name} ${releases[a.id].tag}`).join(", ")}`);
    render();
  } catch (e) {
    /* offline: try again at the next tick */
  }
}

// ---------- background picture ----------

let wallpaperUrl = null;
let bgMessage = ""; // shown inside Settings, where the user is looking

function setWallpaperLook(dim, blur) {
  root.style.setProperty("--bg-dim", String(dim / 100));
  root.style.setProperty("--bg-blur", blur + "px");
}

/** Show (or remove) the user's own picture behind everything. Desktop only for now. */
async function applyWallpaper() {
  const layer = $("wallpaper");
  if (!settings?.bg_has || device?.os === "android") {
    layer.classList.remove("on");
    layer.style.backgroundImage = "";
    if (wallpaperUrl) { URL.revokeObjectURL(wallpaperUrl); wallpaperUrl = null; }
    return;
  }
  try {
    const bytes = await invoke("read_background"); // raw bytes, so even a big picture stays quick
    const url = URL.createObjectURL(new Blob([bytes], { type: settings.bg_mime || "image/png" }));
    if (wallpaperUrl) URL.revokeObjectURL(wallpaperUrl);
    wallpaperUrl = url;
    layer.style.backgroundImage = `url("${url}")`;
    setWallpaperLook(settings.bg_dim, settings.bg_blur);
    layer.classList.add("on");
  } catch (e) {
    layer.classList.remove("on");
    bgMessage = String(e);
  }
}

async function chooseBackground() {
  const path = await window.__TAURI__.dialog.open({
    multiple: false, directory: false, title: "Choose a background picture",
    filters: [{ name: "Pictures", extensions: ["png", "jpg", "jpeg", "gif", "webp"] }],
  });
  if (!path) return;
  try {
    settings = await invoke("set_background", { path });
    bgMessage = "";
    await applyWallpaper();
  } catch (e) {
    bgMessage = String(e); // for example "That picture is 20 MB. Please choose one under 12 MB."
  }
  renderSettings();
}

async function removeBackground() {
  try { settings = await invoke("clear_background"); bgMessage = ""; } catch (e) { bgMessage = String(e); }
  await applyWallpaper();
  renderSettings();
}

function slider(label, max, value, unit, onLive, onSave) {
  const out = el("span", { textContent: value + unit });
  const input = el("input", { type: "range", min: 0, max, step: 1, value });
  input.setAttribute("aria-label", label);
  input.oninput = () => { out.textContent = input.value + unit; onLive(+input.value); }; // moves live
  input.onchange = () => onSave(+input.value);                                           // saved when let go
  return el("label", { className: "range" }, el("span", { textContent: label }), input, out);
}

// ---------- Internet Archive backup ----------

const archiveState = { items: null, error: null, app: "all", system: "mine", search: "", extras: false, note: "", noteIsError: false };
const archiveSaved = new Set();
let archiveStarted = false;

function fmtSize(bytes) {
  if (!bytes) return "size unknown";
  if (bytes < 1048576) return Math.max(1, Math.round(bytes / 1024)) + " KB";
  if (bytes < 1073741824) return (bytes / 1048576).toFixed(1) + " MB";
  return (bytes / 1073741824).toFixed(2) + " GB";
}
function fmtDate(secs) {
  return secs ? new Date(secs * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }) : "date unknown";
}

/** Show or hide the sidebar to match the setting. Nothing is fetched while it is off. */
async function applyArchive() {
  const on = !!settings?.archive_enabled;
  $("layout").classList.toggle("with-archive", on);
  $("archive").hidden = !on;
  if (!on) return;
  renderArchive();
  if (!archiveStarted) {
    archiveStarted = true;
    widenForArchive();
    loadArchive();
  }
}
// Switching it off and on again asks the archive afresh.
function resetArchive() { archiveStarted = false; archiveState.items = null; archiveState.error = null; }

/** Make the window wide enough for the list to sit beside the tiles (desktop only). */
async function widenForArchive() {
  if (device?.os === "android" || window.innerWidth >= 1240) return;
  try {
    const { getCurrentWindow } = window.__TAURI__.window;
    const { LogicalSize } = window.__TAURI__.dpi;
    await getCurrentWindow().setSize(new LogicalSize(1320, Math.max(window.innerHeight, 800)));
  } catch (e) { /* the list simply moves below the tiles when the window stays narrow */ }
}

async function loadArchive() {
  archiveState.items = null;
  archiveState.error = null;
  renderArchive();
  try {
    archiveState.items = await invoke("archive_list");
  } catch (e) {
    archiveState.error = friendlyArchive(String(e));
  }
  renderArchive();
}

/** "Could not reach" almost always means the user is offline, and that is not a problem. */
function friendlyArchive(message) {
  return /could not reach/i.test(message) ? "Can't reach the Internet Archive right now (are you offline?). Everything else still works." : message;
}

function archiveVisible(file, item) {
  const s = archiveState;
  if (s.app !== "all" && item.app_id !== s.app) return false;
  if (s.search && !file.name.toLowerCase().includes(s.search.toLowerCase())) return false;
  if (!s.extras && (file.group !== "program" || file.debug)) return false;
  if (s.system === "mine" && file.group === "program" && (file.os !== device.os || (file.arch !== "any" && file.arch !== device.arch))) return false;
  return true;
}

function renderArchive() {
  const box = $("archive");
  if (box.hidden) return;
  const s = archiveState;
  const head = el("div", { className: "head" }, el("h2", { id: "archive-title", textContent: "Internet Archive backup" }));
  const profile = el("a", { href: "#", textContent: "archive.org/details/@gameh0pper" });
  profile.onclick = (e) => { e.preventDefault(); invoke("open_archive_profile").catch((err) => say(String(err), true)); };
  head.append(el("div", { className: "cap" }, document.createTextNode("Older versions, kept as a safety net. Use the tiles first. These backups come from the developer's page on the Internet Archive: "), profile));

  const appSel = el("select"); appSel.setAttribute("aria-label", "App");
  appSel.add(new Option("All apps", "all"));
  (s.items || []).forEach((i) => appSel.add(new Option(i.app_name, i.app_id)));
  appSel.value = s.app;
  appSel.onchange = () => { s.app = appSel.value; renderArchive(); };
  const sysSel = el("select"); sysSel.setAttribute("aria-label", "System");
  sysSel.add(new Option("My system", "mine")); sysSel.add(new Option("All systems", "all"));
  sysSel.value = s.system;
  sysSel.onchange = () => { s.system = sysSel.value; renderArchive(); };
  const search = el("input", { type: "search", placeholder: "Search file names, e.g. 1.2.0", value: s.search });
  search.setAttribute("aria-label", "Search file names");
  search.oninput = () => {
    s.search = search.value;
    renderArchive();
    // the sidebar was rebuilt, so put the cursor back in the new box, at the end of what was typed
    const again = $("archive").querySelector("input[type=search]");
    if (again) { again.focus(); again.setSelectionRange(again.value.length, again.value.length); }
  };
  const extras = el("input", { type: "checkbox", checked: s.extras });
  extras.onchange = () => { s.extras = extras.checked; renderArchive(); };
  const filters = el("div", { className: "filters" }, el("div", { className: "two" }, appSel, sysSel), search,
    el("label", { className: "opt" }, extras, document.createTextNode("Also show debug, source and packaging files")));

  const refresh = button("Refresh", "", () => loadArchive());
  const list = el("div", { className: "list" });
  let shown = 0, total = 0;
  if (s.items === null && !s.error) {
    list.append(el("div", { className: "note", textContent: "Asking the Internet Archive…" }));
  } else if (s.error) {
    list.append(el("div", { className: "note", textContent: friendlyArchive(s.error) }));
  } else {
    s.items.forEach((item) => {
      total += item.files.length;
      if (s.app !== "all" && item.app_id !== s.app) return;
      const files = item.files.filter((f) => archiveVisible(f, item));
      shown += files.length;
      list.append(el("div", { className: "grp" }, document.createTextNode(item.app_name), el("span", { textContent: item.error ? "" : `${files.length} file${files.length === 1 ? "" : "s"}` })));
      if (item.error) list.append(el("div", { className: "note", textContent: friendlyArchive(item.error) }));
      else if (!files.length) list.append(el("div", { className: "note", textContent: item.files.length ? "Nothing matches these filters." : "No files uploaded yet." }));
      files.forEach((f) => list.append(archiveRow(item, f)));
    });
  }
  const count = el("div", { className: "count" }, el("span", { textContent: s.items ? `Showing ${shown} of ${total} files · newest first` : "" }), refresh);
  const status = el("div", { className: "status", textContent: s.note, hidden: !s.note });
  if (s.noteIsError) status.style.color = "var(--warn)";
  const foot = el("div", { className: "foot", textContent: "Downloads go to your Downloads folder and are checked against the archive's checksums." });
  box.replaceChildren(head, filters, count, status, list, foot);
}

function archiveRow(item, f) {
  const row = el("div", { className: "arow" });
  const name = el("b", { textContent: f.name, title: f.name });
  const bits = [f.version ? "v" + f.version : null, f.kind, fmtSize(f.size), fmtDate(f.mtime)].filter(Boolean);
  const meta = el("small", { textContent: bits.join(" · "), title: bits.join(" · ") });
  const btn = el("button", { type: "button", textContent: archiveSaved.has(f.name) ? "Saved ✓" : "Download" });
  btn.dataset.file = f.name;
  btn.onclick = () => archiveDownload(item, f, btn);
  row.append(name, meta, btn);
  return row;
}

/** Shows the result right in the sidebar, where the user is looking. */
function archiveNote(text, isErr = false) {
  archiveState.note = text;
  archiveState.noteIsError = isErr;
  const status = $("archive").querySelector(".status");
  if (status) { status.textContent = text; status.hidden = false; status.style.color = isErr ? "var(--warn)" : ""; }
  say(text, isErr);
}

async function archiveDownload(item, f, btn) {
  btn.disabled = true;
  btn.textContent = "0%";
  archiveNote(`Downloading ${f.name}…`);
  try {
    const r = await invoke("archive_download", { id: item.app_id, file: f.name });
    archiveSaved.add(f.name);
    const checks = r.verified.length ? "Checked: " + r.verified.join(" and ") + "." : "No checksum was available to check it.";
    archiveNote(r.handed_off ? `${f.name} downloaded. Android's installer opened. ${checks}` : `Saved to ${r.path}. ${checks}`);
    btn.textContent = "Saved ✓";
  } catch (e) {
    archiveNote(String(e), true);
    btn.textContent = "Download";
  } finally {
    btn.disabled = false;
  }
}

// Progress arrives as events while a file downloads.
try {
  window.__TAURI__.event.listen("archive-progress", (e) => {
    const { file, done, total } = e.payload;
    const btn = [...document.querySelectorAll("#archive .arow button")].find((b) => b.dataset.file === file);
    if (btn && total) btn.textContent = Math.min(100, Math.floor((done / total) * 100)) + "%";
  });
} catch (e) { /* no progress numbers; the download still works */ }

// ---------- settings ----------

const MODES = [
  ["auto", "Auto-detect", "Look in the usual folders for apps you already have, and ask before using each one."],
  ["manual", "Choose myself", "Never search. Point the launcher at each app you already have, one by one."],
  ["in_place", "Keep in place", "Use apps you already have right where they are. The launcher never downloads its own copy of them, so you update those yourself."],
];
let found = null; // null = not scanned yet
let welcome = false;

const dialog = $("settings");
const pickFile = (title) => window.__TAURI__.dialog.open({ multiple: false, directory: false, title });
const pickFolder = (title) => window.__TAURI__.dialog.open({ multiple: false, directory: true, title });

function openSettings(isWelcome = false) {
  welcome = isWelcome;
  found = null;
  renderSettings();
  if (!dialog.open) dialog.showModal();
  if (settings.scanning) scan();
}

async function closeSettings() {
  dialog.close();
  if (welcome) {
    welcome = false;
    await invoke("finish_first_run");
  }
}

async function scan() {
  found = null;
  renderSettings();
  try {
    found = await invoke("scan_existing");
  } catch (e) {
    say(String(e), true);
    found = [];
  }
  renderSettings();
}

async function adopt(id, path) {
  try {
    await invoke("adopt_app", { id, path });
    say("Using your copy. It stays where it is.");
  } catch (e) {
    say(String(e), true);
  }
  await loadApps();
  if (found) found = found.filter((c) => c.app_id !== id);
  renderSettings();
}

async function choose(app) {
  const path = await pickFile(`Choose your existing copy of ${app.name}`);
  if (path) await adopt(app.id, path);
}

async function setMode(mode, searchFolders) {
  settings = await invoke("set_existing_mode", { mode, searchFolders });
  found = null;
  render();
  renderSettings();
  if (settings.scanning) scan();
}

function renderSettings() {
  if (!settings) return;
  const sheet = el("div", { className: "sheet" });
  sheet.append(el("h2", { id: "settings-title", textContent: welcome ? "Do you already have some of these apps?" : "Settings" }));
  const phone = device.os === "android";
  if (!phone) sheet.append(el("p", { textContent: "If you installed any of these apps before using the launcher, choose how it should treat them. Your files are never moved or deleted." }));
  if (phone) sheet.append(el("p", { textContent: "On Android, apps you install here are managed by Android itself, so there is nothing to set up." }));
  if (!phone) sheet.append(el("h3", { textContent: "Apps you already have" }));
  if (!phone) MODES.forEach(([value, title, desc]) => {
    const radio = el("input", { type: "radio", name: "mode", value, checked: settings.existing_mode === value });
    radio.onchange = () => setMode(value, settings.search_folders);
    sheet.append(el("label", { className: "choice" }, radio, el("b", { textContent: title }), el("span", { textContent: desc })));
  });
  if (!phone && settings.existing_mode === "in_place") {
    const box = el("input", { type: "checkbox", checked: settings.search_folders });
    box.onchange = () => setMode("in_place", box.checked);
    sheet.append(el("label", { className: "sub" }, box, document.createTextNode("Also search the usual folders for them")));
  }

  if (!phone && settings.scanning) {
    sheet.append(el("h3", { textContent: "Found on this computer" }));
    const rows = el("div", { className: "rows" });
    if (found === null) rows.append(el("div", {}, el("span", { className: "empty", textContent: "Searching…" })));
    else if (!found.length) rows.append(el("div", {}, el("span", { className: "empty", textContent: "Nothing new found. Apps you have elsewhere can be added below." })));
    else found.forEach((c) => rows.append(el("div", {},
      el("div", { className: "main" }, el("b", { textContent: c.app_name + (c.version ? " " + c.version : "") }), el("small", { textContent: c.path })),
      el("div", { className: "acts" }, button("Use this", "go", () => adopt(c.app_id, c.path))))));
    sheet.append(rows);

    sheet.append(el("h3", { textContent: "Folders searched" }));
    const fr = el("div", { className: "rows" });
    settings.folders.forEach((f) => fr.append(el("div", {},
      el("div", { className: "main" }, el("small", { textContent: f.path + (f.exists ? "" : "  (not found)") })),
      el("div", { className: "acts" }, f.custom ? button("Remove", "ghost", async () => { settings = await invoke("remove_scan_folder", { path: f.path }); scan(); }) : el("span")))));
    fr.append(el("div", {}, el("span", { className: "empty", textContent: "Keep your apps somewhere else?" }),
      el("div", { className: "acts" }, button("Add folder…", "ghost", async () => {
        const dir = await pickFolder("Add a folder to search");
        if (dir) { settings = await invoke("add_scan_folder", { path: dir }); scan(); }
      }))));
    sheet.append(fr);
  }

  sheet.append(el("h3", { textContent: "Look" }));
  [["classic", "Classic", "The look it has always had: clean text tiles."],
   ["glass", "Glass icons", "A clear glass bubble icon beside each app's name, tinted with that app's colour."]].forEach(([value, title, text]) => {
    const radio = el("input", { type: "radio", name: "look", value, checked: look === value });
    radio.onchange = () => setLook(value);
    sheet.append(el("label", { className: "choice" }, radio, el("b", { textContent: title }), el("span", { textContent: text })));
  });

  if (!phone) {
    sheet.append(el("h3", { textContent: "Background" }));
    sheet.append(el("p", { textContent: settings.bg_has
      ? "Showing your own picture behind the apps. The tiles stay solid so everything stays easy to read."
      : "No picture chosen, so the plain theme background is used. You can use your own PNG, JPEG, GIF or WebP picture (under 12 MB; GIFs animate). A copy is kept inside the launcher, so it keeps working if the original moves." }));
    const row = el("div", { className: "bgrow" }, button(settings.bg_has ? "Choose a different image…" : "Choose an image…", "ghost", chooseBackground));
    if (settings.bg_has) row.append(button("Remove", "ghost", removeBackground));
    sheet.append(row);
    if (bgMessage) sheet.append(el("p", { className: "bgwarn", textContent: bgMessage }));
    if (settings.bg_has) {
      const save = async (dim, blur) => { try { settings = await invoke("set_background_look", { dim, blur }); } catch (e) { bgMessage = String(e); } };
      sheet.append(slider("Dim", 90, settings.bg_dim, "%", (v) => setWallpaperLook(v, settings.bg_blur), (v) => save(v, settings.bg_blur)));
      sheet.append(slider("Blur", 20, settings.bg_blur, " px", (v) => setWallpaperLook(settings.bg_dim, v), (v) => save(settings.bg_dim, v)));
    }
  }

  sheet.append(el("h3", { textContent: "Download counts" }));
  const countBox = el("input", { type: "checkbox", checked: settings.show_downloads });
  countBox.onchange = async () => {
    try { settings = await invoke("set_show_downloads", { enabled: countBox.checked }); } catch (e) { say(String(e), true); }
    if (settings.show_downloads) refresh(); else render();
    renderSettings();
  };
  sheet.append(el("label", { className: "choice" }, countBox, el("b", { textContent: "Show how many times each app has been downloaded" }),
    el("span", { textContent: "On by default. A small green number on each app's tile: the total for that app across every version and system, from GitHub's own public numbers. Nothing is sent and nobody is identified." })));

  sheet.append(el("h3", { textContent: "Status in the corner" }));
  const statusBox = el("input", { type: "checkbox", checked: settings.show_status });
  statusBox.onchange = async () => {
    try { settings = await invoke("set_show_status", { enabled: statusBox.checked }); } catch (e) { say(String(e), true); }
    if (settings.show_status) updateStatus(); else { lastNetwork = null; renderStatus(); }
    renderSettings();
  };
  sheet.append(el("label", { className: "choice" }, statusBox, el("b", { textContent: "Show whether you are online and whether GitHub is working" }),
    el("span", { textContent: "On by default. Two small indicators in the top-right corner. It reads GitHub's own status page (githubstatus.com). Nothing in the launcher needs the internet, so being offline only turns the indicator grey." })));

  sheet.append(el("h3", { textContent: "Internet Archive backup" }));
  const archiveBox = el("input", { type: "checkbox", checked: settings.archive_enabled });
  archiveBox.onchange = async () => {
    try {
      settings = await invoke("set_archive_enabled", { enabled: archiveBox.checked });
    } catch (e) { say(String(e), true); }
    if (!settings.archive_enabled) resetArchive();
    applyArchive();
    renderSettings();
  };
  sheet.append(el("label", { className: "choice" }, archiveBox, el("b", { textContent: "Show the Internet Archive backup" }),
    el("span", { textContent: "Off by default. A safety net of older versions kept on the Internet Archive, in case a repository or GitHub itself is ever unavailable. When it is on, a list appears beside the apps and you pick a file yourself. The launcher only contacts archive.org while it is on." })));

  sheet.append(el("h3", { textContent: "Your apps" }));
  if (settings.custom_order) {
    sheet.append(button("Reset the order to alphabetical", "ghost", async () => {
      try {
        await invoke("reset_app_order");
        settings = await invoke("get_settings");
        await loadApps();
      } catch (e) { say(String(e), true); }
      renderSettings();
    }));
  }
  const list = el("div", { className: "rows" });
  apps.forEach((a) => {
    const state = !a.installed ? "Not set up" : a.adopted ? "Your copy" : "Installed by the launcher";
    const acts = el("div", { className: "acts" });
    acts.append(button("Report issue", "ghost", () => invoke("open_issues", { id: a.id }).catch((e) => say(String(e), true))));
    if (a.adopted) acts.append(button("Forget", "ghost", () => uninstall(a)));
    const own = a.installed && !a.adopted;
    if (!phone) {
      const use = button(a.adopted ? "Change…" : "Use an existing install…", "ghost", () => choose(a), own);
      if (own) use.title = "Uninstall the launcher's copy first";
      acts.append(use);
    }
    list.append(el("div", {}, el("div", { className: "main" }, el("b", { textContent: a.name }), el("small", { textContent: a.adopted ? a.path : state })), acts));
  });
  sheet.append(list);

  sheet.append(el("div", { className: "foot" }, button(welcome ? "Continue" : "Done", "go", closeSettings)));
  dialog.replaceChildren(sheet);
}

dialog.addEventListener("cancel", (e) => { e.preventDefault(); closeSettings(); });
$("open-settings").onclick = () => openSettings(false);

// ---------- theme and accent ----------

function lum(hex) {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((v) => (v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4)));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
}
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((m, n) => n - m); return (x + 0.05) / (y + 0.05); };
const toHex = (rgb) => "#" + rgb.match(/\d+/g).slice(0, 3).map((n) => (+n).toString(16).padStart(2, "0")).join("");
const store = {
  get: (k) => { try { return localStorage.getItem(k); } catch (e) { return null; } },
  set: (k, v) => { try { localStorage.setItem(k, v); } catch (e) {} },
  del: (k) => { try { localStorage.removeItem(k); } catch (e) {} },
};

function syncPicker() {
  const v = getComputedStyle(root).getPropertyValue("--accent").trim();
  if (/^#[0-9a-f]{6}$/i.test(v)) $("accent").value = v;
}
function checkContrast() {
  const accent = getComputedStyle(root).getPropertyValue("--accent").trim();
  const bg = toHex(getComputedStyle(document.body).backgroundColor);
  $("hint").textContent = /^#[0-9a-f]{6}$/i.test(accent) && ratio(accent, bg) < 3
    ? "This accent is hard to see on this theme. Try a lighter or darker colour." : "";
}
function setAccent(hex, save) {
  root.style.setProperty("--accent", hex);
  root.style.setProperty("--accent-ink", lum(hex) > 0.4 ? "#00141a" : "#ffffff");
  $("accent").value = hex;
  if (save) store.set("accent", hex);
  checkContrast();
}

$("theme").onchange = (e) => {
  root.dataset.theme = e.target.value;
  store.set("theme", e.target.value);
  if (!store.get("accent")) syncPicker();
  checkContrast();
};
$("accent").oninput = (e) => setAccent(e.target.value, true);
$("reset").onclick = () => {
  root.style.removeProperty("--accent");
  root.style.removeProperty("--accent-ink");
  store.del("accent");
  syncPicker();
  checkContrast();
};
$("refresh").onclick = refresh;

// ---------- look: classic or glass icons ----------

let look = store.get("look") === "glass" ? "glass" : "classic"; // classic stays the default
document.body.dataset.look = look;

function setLook(value) {
  look = value;
  store.set("look", value);
  document.body.dataset.look = value;
  render();
  renderSettings();
}

// ---------- start ----------

const savedTheme = store.get("theme");
if (savedTheme && [...$("theme").options].some((o) => o.value === savedTheme)) {
  root.dataset.theme = savedTheme;
  $("theme").value = savedTheme;
}
const savedAccent = store.get("accent");
if (/^#[0-9a-f]{6}$/i.test(savedAccent || "")) setAccent(savedAccent, false);
else syncPicker();
checkContrast();

document.addEventListener("visibilitychange", () => {
  if (document.hidden) return;
  if (device?.os === "android") loadApps().catch(() => {});
  quietCheck(); // a laptop that slept may have missed a few hours
});
setInterval(quietCheck, 15 * 60 * 1000); // wakes up four times an hour, but only asks GitHub every 3 hours

(async () => {
  try {
    device = await invoke("get_device");
    renderDevice();
    settings = await invoke("get_settings");
    await loadApps();
    applyArchive();
    applyWallpaper();
    updateStatus();
    if (settings.first_run && device.os !== "android") openSettings(true);
    await refresh();
  } catch (e) {
    say(String(e), true);
  }
})();
