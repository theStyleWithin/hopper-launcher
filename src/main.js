const { invoke } = window.__TAURI__.core;

const OS_NAME = { linux: "Linux", windows: "Windows", android: "Android", macos: "macOS" };
const KIND = { appimage: "AppImage", deb: ".deb", rpm: ".rpm", installer: "Installer", msi: "MSI", portable: "Portable", exe: ".exe", zip: "Zip", apk: "APK" };

const $ = (id) => document.getElementById(id);
const root = document.documentElement;
let device = null;
let apps = [];
let releases = {}; // app id -> { tag, builds, error }
const busy = new Set();

function say(text, isErr = false) {
  $("msg").textContent = text || "";
  $("msg").className = isErr ? "err" : "";
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

function button(label, cls, onclick, disabled = false) {
  const b = document.createElement("button");
  b.className = cls;
  b.textContent = label;
  b.disabled = disabled;
  if (onclick) b.onclick = onclick;
  return b;
}

function tile(app) {
  const rel = releases[app.id];
  const working = busy.has(app.id);
  const os = OS_NAME[device.os] || device.os;
  const builds = rel?.builds || [];
  const latest = rel?.tag;
  const hasUpdate = app.installed && latest && app.version && latest !== app.version;
  const untested = (app.untested_on || []).includes(device.os) ? " · untested" : "";

  const el = document.createElement("section");
  el.className = "tile";
  if (app.accent) el.style.setProperty("--a", app.accent);

  const top = document.createElement("div");
  top.className = "top";
  const titles = document.createElement("div");
  const h = document.createElement("h2");
  h.textContent = app.name;
  const tag = document.createElement("div");
  tag.className = "tag";
  tag.textContent = app.tagline;
  titles.append(h, tag);
  top.append(titles);
  if (app.installed) top.append(button("Uninstall", "x", () => uninstall(app), working));

  const row = document.createElement("div");
  row.className = "row";
  const sel = document.createElement("select");
  sel.setAttribute("aria-label", "Version");
  const btns = document.createElement("div");
  btns.className = "btns";

  const offline = !rel ? "Checking GitHub…" : rel.error;

  if (app.installed) {
    sel.add(new Option(hasUpdate ? `Update available · ${latest}` : `Installed · ${app.version || ""}`.trim()));
    sel.disabled = true;
    btns.append(button("Launch", "go", () => launch(app), working));
    if (hasUpdate && builds.length) {
      btns.append(button(working ? "Updating…" : "Update", "go alt", () => install(app, builds[0].file), working));
    }
  } else if (builds.length) {
    builds.forEach((b, i) => sel.add(new Option((i === 0 ? "Recommended: " : "") + KIND[b.kind] + untested, b.file)));
    btns.append(button(working ? "Installing…" : "Install", "go", () => install(app, sel.value), working));
    sel.disabled = working;
  } else {
    el.classList.add("off");
    sel.add(new Option(rel?.error ? offline : `No ${os} build yet`));
    sel.disabled = true;
    btns.append(button(rel ? "Unavailable" : "Please wait", "go", null, true));
  }

  row.append(sel, btns);
  el.append(top, row);
  return el;
}

function render() {
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

async function launch(app) {
  try {
    await invoke("launch_app", { id: app.id });
    say(`${app.name} launched. You can close the launcher; the app keeps running on its own.`);
  } catch (e) {
    say(String(e), true);
  }
}

async function uninstall(app) {
  const warn = app.folder
    ? `Uninstall ${app.name}? This deletes its program folder, including any settings stored inside it.`
    : `Uninstall ${app.name}? Your settings and data are kept.`;
  if (!confirm(warn)) return;
  try {
    await invoke("uninstall_app", { id: app.id });
    say(`${app.name} uninstalled`);
  } catch (e) {
    say(String(e), true);
  }
  await loadApps();
}

async function refresh() {
  $("refresh").disabled = true;
  releases = {};
  render();
  say("Checking GitHub…");
  try {
    (await invoke("check_releases")).forEach((r) => (releases[r.id] = r));
    const updates = apps.filter((a) => a.installed && releases[a.id]?.tag && a.version && releases[a.id].tag !== a.version).length;
    const failed = Object.values(releases).filter((r) => r.error && r.error !== "No release published yet").length;
    say(failed ? "Some apps could not be checked. Are you online?" : updates ? `${updates} update${updates > 1 ? "s" : ""} available` : "Everything is up to date", !!failed);
  } catch (e) {
    say(String(e), true);
  } finally {
    $("refresh").disabled = false;
    render();
  }
}

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

(async () => {
  try {
    device = await invoke("get_device");
    renderDevice();
    await loadApps();
    await refresh();
  } catch (e) {
    say(String(e), true);
  }
})();
