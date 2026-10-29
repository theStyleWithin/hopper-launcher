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

function tile(app) {
  const rel = releases[app.id];
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
  const top = el("div", { className: "top" }, grip, el("div", { className: "titles" }, el("h2", { textContent: app.name, title: app.name }), el("div", { className: "tag", textContent: app.tagline, title: app.tagline })));
  if (app.installed) top.append(button(app.adopted ? "Forget" : "Uninstall", "x", () => uninstall(app), working));

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
      sel.add(new Option(prompt ? `Update available · ${latest}` : mine + ver));
      sel.disabled = true;
      btns.append(button("Launch", "go", () => launch(app), working));
    }
  } else if (builds.length) {
    builds.forEach((b, i) => sel.add(new Option((i === 0 ? "Recommended: " : "") + KIND[b.kind] + untested, b.file)));
    sel.disabled = working;
    btns.append(button(working ? "Installing…" : "Install", "go", () => install(app, sel.value), working));
  } else {
    t.classList.add("off");
    sel.add(new Option(rel?.error ? rel.error : rel ? `No ${os} build yet` : "Checking GitHub…"));
    sel.disabled = true;
    btns.append(button(rel ? "Unavailable" : "Please wait", "go", null, true));
  }

  t.append(top, el("div", { className: "row" }, sel, btns));
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

const CHECK_EVERY = 3 * 60 * 60 * 1000; // look for new releases every 3 hours while the launcher is open
let lastCheck = Date.now();

async function refresh() {
  lastCheck = Date.now();
  $("refresh").disabled = true;
  releases = {};
  render();
  say("Checking GitHub…");
  try {
    (await invoke("check_releases")).forEach((r) => (releases[r.id] = r));
    const updates = apps.filter(updatePrompt).length;
    const failed = Object.values(releases).filter((r) => r.error && r.error !== "No release published yet").length;
    say(failed ? "Some apps could not be checked. Are you online?" : updates ? `${updates} update${updates > 1 ? "s" : ""} available` : "Everything is up to date", !!failed);
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
  const before = Object.fromEntries(Object.entries(releases).map(([id, r]) => [id, r.tag]));
  try {
    (await invoke("check_releases")).forEach((r) => {
      if (!r.error || !releases[r.id]?.tag) releases[r.id] = r; // a failed check keeps the last good answer
    });
    const fresh = apps.filter((a) => before[a.id] && releases[a.id]?.tag !== before[a.id] && updatePrompt(a));
    if (fresh.length) say(`New release: ${fresh.map((a) => `${a.name} ${releases[a.id].tag}`).join(", ")}`);
    render();
  } catch (e) {
    /* offline: try again at the next tick */
  }
}

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
    if (settings.first_run && device.os !== "android") openSettings(true);
    await refresh();
  } catch (e) {
    say(String(e), true);
  }
})();
