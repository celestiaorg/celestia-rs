"use strict";

// Crawl data lives on the node-census-data branch so the scheduled job never
// writes to main. `?data=<relative path>` reads a local copy instead.
const DEFAULT_DATA = "https://raw.githubusercontent.com/celestiaorg/celestia-rs/node-census-data/";

// A same-origin override of the data location, or null. Comparing the
// resolved origin also rejects forms that browsers normalize to another host,
// such as a leading double backslash.
function sameOriginBase(p, here) {
  if (!p) return null;
  let u;
  try {
    u = new URL(p, here);
  } catch {
    return null;
  }
  if (u.origin !== new URL(here).origin) return null;
  return u.href.endsWith("/") ? u.href : u.href + "/";
}

function dataBase() {
  const p = new URLSearchParams(location.search).get("data");
  return sameOriginBase(p, location.href) ?? DEFAULT_DATA;
}

const BASE = dataBase();
const state = { network: "mainnet", history: null, latest: {} };
const $ = (id) => document.getElementById(id);
const fmtDate = (s) =>
  new Date(s * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
const fmtDay = (s) => new Date(s * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" });
const share = (s) => (s.reachable ? (100 * s.rust) / s.reachable : 0);

function el(tag, attrs = {}, text) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  if (text !== undefined) e.textContent = text;
  return e;
}

function svg(tag, attrs = {}) {
  const e = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  return e;
}

async function getJSON(path) {
  const r = await fetch(BASE + path, { cache: "no-store" });
  if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
  return r.json();
}

function niceMax(v) {
  if (v <= 0) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= v) return m * p;
  return 10 * p;
}

// One single-series line chart with a crosshair tooltip.
function lineChart({ title, cls, points, format, integer = false }) {
  const fig = el("figure");
  fig.append(el("h3", {}, title));
  const W = 520, H = 200, L = 40, R = 12, T = 10, B = 26;
  const root = svg("svg", { viewBox: `0 0 ${W} ${H}`, role: "img", "aria-label": title });
  let max = niceMax(Math.max(...points.map((p) => p.y)));
  // Four grid steps: keep integer axes on whole numbers.
  if (integer) max = Math.max(4, Math.ceil(max / 4) * 4);
  const x0 = points[0].x, x1 = points[points.length - 1].x;
  const sx = (x) => (x1 === x0 ? (L + W - R) / 2 : L + ((x - x0) / (x1 - x0)) * (W - L - R));
  const sy = (y) => T + (1 - y / max) * (H - T - B);
  for (let i = 0; i <= 4; i++) {
    const v = (max * i) / 4;
    root.append(svg("line", { class: "grid-line", x1: L, x2: W - R, y1: sy(v), y2: sy(v) }));
    const t = svg("text", { class: "axis", x: L - 6, y: sy(v) + 4, "text-anchor": "end" });
    t.textContent = format(v);
    root.append(t);
  }
  const ticks = x1 === x0 ? [x0] : [x0, (x0 + x1) / 2, x1];
  for (const x of ticks) {
    const t = svg("text", { class: "axis", x: sx(x), y: H - 6, "text-anchor": "middle" });
    t.textContent = fmtDay(x);
    root.append(t);
  }
  if (points.length > 1) {
    const d = points.map((p, i) => `${i ? "L" : "M"}${sx(p.x).toFixed(1)},${sy(p.y).toFixed(1)}`).join("");
    root.append(svg("path", { class: `series ${cls}`, d }));
  }
  const last = points[points.length - 1];
  root.append(svg("circle", { class: `dot ${cls}`, cx: sx(last.x), cy: sy(last.y), r: 4 }));
  const cross = svg("line", { class: "crosshair", y1: T, y2: H - B, visibility: "hidden" });
  const hover = svg("circle", { class: `dot ${cls}`, r: 4, visibility: "hidden" });
  root.append(cross, hover);
  const hit = svg("rect", { x: L, y: 0, width: W - L - R, height: H, fill: "transparent" });
  root.append(hit);
  const tip = $("tooltip");
  hit.addEventListener("pointermove", (ev) => {
    const box = root.getBoundingClientRect();
    const px = ((ev.clientX - box.left) / box.width) * W;
    let best = points[0];
    for (const p of points) if (Math.abs(sx(p.x) - px) < Math.abs(sx(best.x) - px)) best = p;
    cross.setAttribute("x1", sx(best.x));
    cross.setAttribute("x2", sx(best.x));
    hover.setAttribute("cx", sx(best.x));
    hover.setAttribute("cy", sy(best.y));
    cross.setAttribute("visibility", "visible");
    hover.setAttribute("visibility", "visible");
    tip.textContent = `${fmtDate(best.x)} · ${format(best.y)}`;
    tip.hidden = false;
    tip.style.left = `${ev.clientX + 12}px`;
    tip.style.top = `${ev.clientY + 12}px`;
  });
  hit.addEventListener("pointerleave", () => {
    cross.setAttribute("visibility", "hidden");
    hover.setAttribute("visibility", "hidden");
    tip.hidden = true;
  });
  fig.append(root);
  return fig;
}

function table(id, head, rows, numeric = []) {
  const t = $(id);
  t.replaceChildren();
  const tr = el("tr");
  head.forEach((h, i) => tr.append(el("th", numeric.includes(i) ? { class: "num" } : {}, h)));
  t.append(el("thead"));
  t.tHead.append(tr);
  const body = el("tbody");
  if (!rows.length) {
    const r = el("tr");
    r.append(el("td", { colspan: head.length }, "None observed."));
    body.append(r);
  }
  for (const row of rows) {
    const r = el("tr");
    row.forEach((c, i) => {
      const td = el("td", numeric.includes(i) ? { class: "num" } : {});
      if (typeof c === "object") td.append(c);
      else td.textContent = c;
      r.append(td);
    });
    body.append(r);
  }
  t.append(body);
}

function render() {
  const net = state.network;
  document.querySelectorAll(".filters button").forEach((b) =>
    b.setAttribute("aria-selected", String(b.dataset.network === net)),
  );
  const entries = (state.history?.entries ?? []).filter((e) => e.network === net);
  const latest = state.latest[net];
  if (!entries.length || !latest) {
    $("status").textContent = `No crawl data for ${net} yet.`;
    $("tiles").replaceChildren();
    $("charts").replaceChildren();
    table("versions", ["Version", "Implementation", "Nodes"], [], [2]);
    table("rust-nodes", ["Node ID", "Moniker", "Version", "Source"], []);
    return;
  }
  const s = latest.summary;
  $("status").textContent = `Latest crawl: ${fmtDate(latest.crawled_at)} · chain ${latest.chain_id} · ${entries.length} crawls recorded`;
  const tiles = [
    [s.rust, "celestia-rs nodes"],
    [s.go, "Go nodes"],
    [`${share(s).toFixed(1)}%`, "celestia-rs share"],
    [s.reachable, "Observed nodes"],
    [s.direct, "Reached directly"],
    [s.unknown, "Unknown version"],
  ];
  $("tiles").replaceChildren(
    ...tiles.map(([v, label]) => {
      const d = el("div", { class: "tile" });
      d.append(el("b", {}, String(v)), el("span", {}, label));
      return d;
    }),
  );
  const pts = (f) => entries.map((e) => ({ x: e.crawled_at, y: f(e.summary) }));
  const int = (v) => String(Math.round(v));
  $("charts").replaceChildren(
    lineChart({ title: "celestia-rs nodes", cls: "rust", points: pts((x) => x.rust), format: int, integer: true }),
    lineChart({ title: "Go (celestia-core) nodes", cls: "go", points: pts((x) => x.go), format: int, integer: true }),
    lineChart({
      title: "celestia-rs share of observed nodes",
      cls: "rust",
      points: pts(share),
      format: (v) => `${v.toFixed(1)}%`,
    }),
  );
  const impl = Object.fromEntries(latest.nodes.map((n) => [n.version, n.implementation]));
  const versions = Object.entries(s.versions).sort((a, b) => b[1] - a[1]);
  table(
    "versions",
    ["Version", "Implementation", "Nodes"],
    versions.map(([v, n]) => [el("code", {}, v || "(empty)"), impl[v] ?? "", String(n)]),
    [2],
  );
  table(
    "rust-nodes",
    ["Node ID", "Moniker", "Version", "Source"],
    latest.nodes
      .filter((n) => n.implementation === "rust")
      .map((n) => [el("code", {}, n.id.slice(0, 12) + "…"), n.moniker, n.version, n.source]),
  );
}

async function main() {
  document.querySelectorAll(".filters button").forEach((b) =>
    b.addEventListener("click", () => {
      state.network = b.dataset.network;
      render();
    }),
  );
  try {
    state.history = await getJSON("history.json");
    await Promise.all(
      ["mainnet", "mocha"].map(async (n) => {
        try {
          state.latest[n] = await getJSON(`latest/${n}.json`);
        } catch {
          // A network with no crawl yet renders as empty.
        }
      }),
    );
    render();
  } catch (e) {
    $("status").textContent = `Could not load crawl data (${e.message}).`;
  }
}

main();
