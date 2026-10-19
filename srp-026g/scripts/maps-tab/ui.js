// The Maps tab: one map at a time, its spawns for the chosen mode, every kind toggled on its own.
// World coordinates throughout: SVG x = world x, SVG y = -z (north up).
(() => {
  const UK = (new URLSearchParams(location.search).get("lang") || document.documentElement.lang) === "uk";
  const T = (en, uk) => (UK ? uk : en);
  const N = o => (UK ? o.uk : o.en);
  const $ = id => document.getElementById(id);
  const SVG = "http://www.w3.org/2000/svg";
  const svgEl = (tag, attrs = {}, ...kids) => {
    const e = document.createElementNS(SVG, tag);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    e.append(...kids);
    return e;
  };
  const el = (tag, attrs = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) k === "text" ? (e.textContent = v) : e.setAttribute(k, v);
    e.append(...kids);
    return e;
  };
  const store = {
    get(k) { try { return JSON.parse(localStorage.getItem("maps." + k)); } catch { return null; } },
    set(k, v) { try { localStorage.setItem("maps." + k, JSON.stringify(v)); } catch { /* private window */ } },
  };

  const LAYERS = [
    { key: "respawn", label: T("Player spawns", "Відродження") },
    { key: "spawners", label: T("Battery spawns", "Батареї") },
    { key: "stations", label: T("Chargers", "Зарядки") },
    { key: "containers", label: T("Containers", "Контейнери") },
    { key: "resupply", label: T("Ammo stashes", "Поповнення амуніції") },
    ...Object.entries(MAPS.artefacts).map(([k, a]) => ({ key: "art:" + k, label: N(a.name), icon: a.icon })),
  ];
  const state = {
    map: MAPS.maps[0].key, mode: null, view: store.get("view") || "render",
    on: Object.assign(Object.fromEntries(LAYERS.map(l => [l.key, true])), store.get("on") || {}),
    vb: null,
  };
  const fromHash = () => {
    const [key, mode] = decodeURIComponent(location.hash.slice(1)).split("/");
    if (MAPS.maps.some(m => m.key === key)) { state.map = key; state.mode = Number(mode) || null; }
  };
  fromHash();
  const map = () => MAPS.maps.find(m => m.key === state.map);
  // Two maps in (nearly) one frame: a port and its original.
  const sameFrame = (a, b) => a.box.every((v, i) => Math.abs(v - b.box[i]) < 2);
  const mode = () => map().modes.find(m => m.mode === state.mode) || map().modes[0];

  document.title = T("Survarium maps", "Мапи Survarium");
  $("title").textContent = T("Maps", "Мапи");
  $("stamp").textContent = T(`Skhid revision ${MAPS.revision}`, `Схід, ревізія ${MAPS.revision}`);

  // A chip group: one pressed.
  function chips(host, items, current, pick) {
    host.replaceChildren(...items.map(([value, text]) => {
      const b = el("button", { type: "button", class: "chip", "aria-pressed": String(value === current), text });
      b.addEventListener("click", () => pick(value));
      return b;
    }));
  }
  const modeName = (m, all) => {
    const same = all.filter(o => N(o.name) === N(m.name));
    return same.length > 1 && m.stations.length ? `${N(m.name)} · ${T("chargers", "зарядки")}` : N(m.name);
  };

  // The HUD minimap's icons (MAPS.hud). A team's icon is painted in its
  // colour by the icon's own lightness, measured by the build, solid, with
  // the HUD's dark outline. Others keep their colours and get the outline.
  const TEAM = { t0: "#3b82f6", t1: "#e5484d", tx: "#9aa0a6" };
  const defs = svgEl("svg", { width: 0, height: 0, "aria-hidden": "true", style: "position:absolute" }, svgEl("defs"));
  document.body.append(defs);
  // A crisp one-pixel dark edge round the icon (no blur), after its paint.
  const filter = (id, values) => {
    if (!document.getElementById(id))
      defs.firstChild.append(svgEl("filter", { id, "color-interpolation-filters": "sRGB", primitiveUnits: "objectBoundingBox", x: "-20%", y: "-20%", width: "140%", height: "140%" },
        svgEl("feColorMatrix", { type: "matrix", values: values || "1 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 1 0", result: "icon" }),
        svgEl("feMorphology", { in: "icon", operator: "dilate", radius: 0.05, result: "wide" }),
        svgEl("feFlood", { "flood-color": "#000", "flood-opacity": 0.85 }),
        svgEl("feComposite", { in2: "wide", operator: "in", result: "edge" }),
        svgEl("feMerge", {}, svgEl("feMergeNode", { in: "edge" }), svgEl("feMergeNode", { in: "icon" }))));
    return `url(#${id})`;
  };
  const outline = () => filter("outline");
  // Colour x (0.7 + 0.3 x lightness / the icon's own): its faces are the
  // colour itself, its shading at most 30% darker, so every team icon reads
  // as the same blue or red.
  const paint = (name, tint) => {
    const i = MAPS.hud[name], [r, g, b] = [1, 3, 5].map(k => parseInt(TEAM[tint].slice(k, k + 2), 16) / 255);
    const row = c => { const k = (0.3 * c) / i.light; return `${0.3 * k} ${0.59 * k} ${0.11 * k} 0 ${0.7 * c}`; };
    return filter(`paint-${name}-${tint}`, `${row(r)} ${row(g)} ${row(b)} 0 0 0 ${1 / i.opaque} 0`);
  };
  const tintOf = t => (t === 0 ? "t0" : t === 1 ? "t1" : "tx");
  // One icon centred on (x, y), `px` screen pixels high: painted in a team's
  // colour when `tint` is given.
  function hud(name, x, y, px, u, tint = null, rotate = 0) {
    const i = MAPS.hud[name], k = (px * u) / i.h;
    const attrs = { href: i.src, x: x - i.ax * k, y: y - i.ay * k, width: i.w * k, height: i.h * k, filter: tint ? paint(name, tint) : outline() };
    if (rotate) attrs.transform = `rotate(${rotate} ${x} ${y})`;
    return svgEl("image", attrs);
  }
  // Legend swatches, drawn as the markers are.
  const swatch = key => {
    const s = svgEl("svg", { viewBox: "-11 -11 22 22", "aria-hidden": "true" });
    if (key === "respawn") s.append(hud("player", -5, 1, 15, 1, "t0"), hud("player", 6, 1, 15, 1, "t1"));
    if (key === "spawners") s.append(hud("battery", 0, 0, 14, 1));
    if (key === "stations") s.append(hud("charger", 0, 0, 22, 1));
    if (key === "containers") s.append(hud("base", -5, 2, 17, 1, "t0"), hud("base", 6, 2, 17, 1, "t1"));
    if (key === "resupply") s.append(hud("resupply", -4, -1, 18, 1, "t0"), hud("resupply", 5, 1, 18, 1, "t1"));
    return s;
  };

  function drawControls() {
    const m = map();
    if (!m.modes.some(o => o.mode === state.mode)) state.mode = m.modes[0].mode;
    // A map in the same frame (a port and its original) keeps the view, for comparing.
    chips($("maps"), MAPS.maps.map(o => [o.key, N(o.name)]), state.map, k => {
      const before = map();
      state.map = k;
      if (!sameFrame(map(), before)) state.vb = null;
      go();
    });
    chips($("modes"), m.modes.map(o => [o.mode, modeName(o, m.modes)]), state.mode, k => { state.mode = k; go(); });
    chips($("views"), [["render", T("From above", "Згори")], ["minimap", T("Minimap", "Мінімапа")]], state.view,
      v => { state.view = v; store.set("view", v); go(); });
    const mo = mode();
    const counts = {
      respawn: mo.respawn.length, spawners: mo.spawners.length, stations: mo.stations.length, containers: mo.containers.length,
      resupply: mo.resupply.length,
      ...Object.fromEntries(Object.keys(MAPS.artefacts).map(k => ["art:" + k, m.artefacts.filter(a => a[3].includes(k)).length])),
    };
    $("layers").replaceChildren(...LAYERS.map(l => {
      const n = counts[l.key];
      const extra = l.key === "stations" && mo.stations_active && mo.stations_active < n ? ` (${T(`${mo.stations_active} active`, `${mo.stations_active} активні`)})` : "";
      const b = el("button", { type: "button", class: "layer", "aria-pressed": String(state.on[l.key]) },
        l.icon ? el("img", { src: l.icon, alt: "" }) : swatch(l.key), document.createTextNode(l.label), el("b", { text: n + extra }));
      b.hidden = !n;
      b.addEventListener("click", () => { state.on[l.key] = !state.on[l.key]; store.set("on", state.on); go(); });
      return b;
    }));
    const tdm = !mo.spawners.length && !mo.stations.length;
    $("note").textContent = mo.batteries && !tdm ? T(`${mo.batteries} batteries to win.`, `${mo.batteries} батарей для перемоги.`) : "";
  }

  // The view box: the play area to start, then wherever the reader zooms and drags.
  const svg = $("map");
  // The play area, grown to hold every point of the map: of every map in
  // its frame, so maps that share one start in the same place.
  const fit = () => {
    const same = MAPS.maps.filter(m => sameFrame(m, map()));
    let [x0, y0, x1, y1] = map().play;
    for (const m of same) { x0 = Math.min(x0, m.play[0]); y0 = Math.min(y0, m.play[1]); x1 = Math.max(x1, m.play[2]); y1 = Math.max(y1, m.play[3]); }
    const points = same.flatMap(m => [...m.artefacts, ...m.modes.flatMap(o => [...o.respawn, ...o.spawners, ...o.stations, ...o.containers, ...o.resupply])]);
    for (const [x, z] of points) { x0 = Math.min(x0, x); x1 = Math.max(x1, x); y0 = Math.min(y0, -z); y1 = Math.max(y1, -z); }
    const pad = Math.max(x1 - x0, y1 - y0) * 0.03;
    return [x0 - pad, y0 - pad, x1 - x0 + 2 * pad, y1 - y0 + 2 * pad];
  };
  // Metres per screen pixel, so markers keep their size on screen at any zoom
  // (the HUD's bitmaps at their own pixel size: crisp).
  const unit = () => {
    const r = svg.getBoundingClientRect(), [, , w, h] = state.vb;
    return Math.max(w / (r.width || 800), h / (r.height || 800));
  };

  function drawMap() {
    const m = map(), mo = mode();
    state.vb = state.vb || fit();
    svg.setAttribute("viewBox", state.vb.join(" "));
    svg.setAttribute("aria-label", N(m.name));
    const u = unit();
    const kids = [];
    const img = state.view === "render" && m.render ? m.render : { src: m.minimap, box: m.box };
    const [bx0, by0, bx1, by1] = img.box;
    kids.push(svgEl("image", { href: img.src, x: bx0, y: by0, width: bx1 - bx0, height: by1 - by0, preserveAspectRatio: "none" }));
    const team = t => (t === 0 ? "var(--team0)" : t === 1 ? "var(--team1)" : "var(--team-x)");
    const teamName = t => (t === 0 || t === 1 ? T(`team ${t + 1}`, `команда ${t + 1}`) : T("any team", "будь-яка команда"));
    const tip = text => svgEl("title", {}, text);
    if (state.on.stations && state.on.spawners) {
      for (const [x, z, , links] of mo.stations) for (const i of links) {
        const [sx, sz] = mo.spawners[i];
        kids.push(svgEl("line", { x1: x, y1: -z, x2: sx, y2: -sz, stroke: "var(--link)", "stroke-width": 1.2 * u, "stroke-dasharray": `${3 * u} ${3 * u}`, opacity: 0.8 }));
      }
    }
    if (state.on.resupply) for (const [x, z, , t] of mo.resupply)
      kids.push(svgEl("g", { class: "mk" }, hud("resupply", x, -z, MAPS.hud.resupply.h, u, tintOf(t)), tip(`${T("Ammo stash", "Поповнення амуніції")}, ${teamName(t)}`)));
    if (state.on.containers) for (const [x, z, , t] of mo.containers)
      kids.push(svgEl("g", { class: "mk" }, hud("base", x, -z, MAPS.hud.base.h, u, tintOf(t)), tip(`${T("Container", "Контейнер")}, ${teamName(t)}`)));
    if (state.on.spawners) for (const [x, z] of mo.spawners)
      kids.push(svgEl("g", { class: "mk" }, hud("battery", x, -z, 20, u), tip(T("Battery spawn", "Поява батареї"))));
    if (state.on.stations) for (const [x, z] of mo.stations)
      kids.push(svgEl("g", { class: "mk" }, hud("charger", x, -z, MAPS.hud.charger.h, u), tip(T("Charger", "Зарядка"))));
    for (const [x, z, , kinds] of m.artefacts) {
      const shown = kinds.filter(k => state.on["art:" + k]);
      shown.forEach((k, i) => {
        const s = 24 * u, dx = (i - (shown.length - 1) / 2) * s * 0.9;
        kids.push(svgEl("g", { class: "mk" },
          svgEl("image", { href: MAPS.artefacts[k].icon, x: x + dx - s / 2, y: -z - s / 2, width: s, height: s, filter: outline() }),
          tip(kinds.map(k => N(MAPS.artefacts[k].name)).join(", "))));
      });
    }
    // The HUD's player triangle, turned the way the player faces at spawn.
    if (state.on.respawn) for (const [x, z, , t, facing] of mo.respawn)
      kids.push(svgEl("g", { class: "mk" }, hud("player", x, -z, MAPS.hud.player.h, u, tintOf(t), facing), tip(`${T("Spawn", "Відродження")}, ${teamName(t)}`)));
    svg.replaceChildren(...kids);
  }

  function go() {
    history.replaceState(null, "", `#${state.map}/${state.mode || ""}`);
    drawControls();
    drawMap();
  }

  // Zoom and drag: the wheel, two fingers, the buttons; a double click fits the map again.
  const zoomAt = (k, cx, cy) => {
    const [x, y, w, h] = state.vb, full = fit();
    const nw = Math.min(full[2] * 1.5, Math.max(full[2] / 12, w * k)), f = nw / w;
    state.vb = [cx - (cx - x) * f, cy - (cy - y) * f, w * f, h * f];
    drawMap();
  };
  const toWorld = (px, py) => {
    const r = svg.getBoundingClientRect(), p = svg.createSVGPoint();
    p.x = px; p.y = py;
    const q = p.matrixTransform(svg.getScreenCTM().inverse());
    return r.width ? [q.x, q.y] : [state.vb[0] + state.vb[2] / 2, state.vb[1] + state.vb[3] / 2];
  };
  const centre = () => [state.vb[0] + state.vb[2] / 2, state.vb[1] + state.vb[3] / 2];
  svg.addEventListener("wheel", e => { e.preventDefault(); zoomAt(e.deltaY > 0 ? 1.2 : 1 / 1.2, ...toWorld(e.clientX, e.clientY)); }, { passive: false });
  svg.addEventListener("dblclick", () => { state.vb = fit(); drawMap(); });
  $("zin").addEventListener("click", () => zoomAt(1 / 1.4, ...centre()));
  $("zout").addEventListener("click", () => zoomAt(1.4, ...centre()));
  $("zfit").addEventListener("click", () => { state.vb = fit(); drawMap(); });
  const pointers = new Map();
  let pinch = null;
  svg.addEventListener("pointerdown", e => { svg.setPointerCapture(e.pointerId); pointers.set(e.pointerId, [e.clientX, e.clientY]); svg.classList.add("drag"); pinch = null; });
  svg.addEventListener("pointermove", e => {
    if (!pointers.has(e.pointerId)) return;
    const prev = pointers.get(e.pointerId);
    pointers.set(e.pointerId, [e.clientX, e.clientY]);
    if (pointers.size === 1) {
      const u = unit();
      state.vb = [state.vb[0] - (e.clientX - prev[0]) * u, state.vb[1] - (e.clientY - prev[1]) * u, state.vb[2], state.vb[3]];
      drawMap();
    } else if (pointers.size === 2) {
      const [a, b] = [...pointers.values()], d = Math.hypot(a[0] - b[0], a[1] - b[1]);
      if (pinch) zoomAt(pinch / d, ...toWorld((a[0] + b[0]) / 2, (a[1] + b[1]) / 2));
      pinch = d;
    }
  });
  const up = e => { pointers.delete(e.pointerId); pinch = null; if (!pointers.size) svg.classList.remove("drag"); };
  svg.addEventListener("pointerup", up);
  svg.addEventListener("pointercancel", up);
  addEventListener("resize", () => drawMap());
  addEventListener("hashchange", () => { fromHash(); state.vb = null; go(); });
  go();
})();
