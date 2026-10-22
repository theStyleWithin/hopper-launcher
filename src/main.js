const { invoke } = window.__TAURI__.core;
const grid = document.getElementById("grid");
const statusEl = document.getElementById("status");
const refreshBtn = document.getElementById("refresh");

let apps = [];
let latest = {};   // id -> latest release tag from GitHub
const busy = new Set();

function say(msg, isErr = false) {
  statusEl.textContent = msg || "";
  statusEl.className = isErr ? "err" : "";
}

function render() {
  grid.replaceChildren(...apps.map(tile));
}

function tile(app) {
  const el = document.createElement("section");
  el.className = "tile";
  if (app.accent) el.style.setProperty("--a", app.accent);

  const hasUpdate = app.installed && app.version && latest[app.id] && latest[app.id] !== app.version;
  const working = busy.has(app.id);

  const info = document.createElement("div");
  const h = document.createElement("h2");
  h.textContent = app.name;
  const t = document.createElement("div");
  t.className = "tag";
  t.textContent = app.tagline;
  const v = document.createElement("div");
  v.className = "ver";
  v.textContent = hasUpdate
    ? `update available: ${latest[app.id]}`
    : app.version ? `installed ${app.version}` : app.installed ? "installed" : "";
  info.append(h, t, v);

  const actions = document.createElement("div");
  actions.className = "actions";

  const main = document.createElement("button");
  main.disabled = working;
  if (!app.installed) {
    main.textContent = working ? "Installing…" : "Install";
    main.onclick = () => install(app);
  } else {
    main.textContent = "Launch";
    main.onclick = () => launch(app);
  }
  actions.append(main);

  if (hasUpdate) {
    const up = document.createElement("button");
    up.className = "ghost";
    up.disabled = working;
    up.textContent = working ? "Updating…" : "Update";
    up.onclick = () => install(app);
    actions.append(up);
  }

  const gh = document.createElement("button");
  gh.className = "ghost icon";
  gh.title = "Open on GitHub";
  gh.textContent = "GitHub";
  gh.onclick = () => invoke("open_repo", { id: app.id }).catch((e) => say(String(e), true));
  actions.append(gh);

  el.append(info, actions);
  return el;
}

async function loadApps() {
  apps = await invoke("list_apps");
  render();
}

async function launch(app) {
  try {
    await invoke("launch_app", { id: app.id });
    say(`Launched ${app.name}`);
  } catch (e) {
    say(String(e), true);
  }
}

async function install(app) {
  busy.add(app.id);
  say(`Downloading ${app.name}…`);
  render();
  try {
    const tag = await invoke("install_app", { id: app.id });
    say(`${app.name} ${tag} installed`);
  } catch (e) {
    say(String(e), true);
  } finally {
    busy.delete(app.id);
    await loadApps();
  }
}

async function checkUpdates() {
  refreshBtn.disabled = true;
  say("Checking GitHub…");
  try {
    latest = await invoke("check_updates");
    const n = apps.filter((a) => a.installed && a.version && latest[a.id] && latest[a.id] !== a.version).length;
    say(n ? `${n} update${n > 1 ? "s" : ""} available` : "Everything is up to date");
    render();
  } catch (e) {
    say(String(e), true);
  } finally {
    refreshBtn.disabled = false;
  }
}

refreshBtn.onclick = checkUpdates;
loadApps().then(checkUpdates).catch((e) => say(String(e), true));
