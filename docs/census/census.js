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

function niceStep(v, integer) {
  if (v <= 0) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10]) {
    const step = m * p;
    if (step >= v && (!integer || Number.isInteger(step))) return step;
  }
  return 10 * p;
}

// A line chart of several series on one axis, with a crosshair tooltip that
// lists every series at the hovered crawl.
function lineChart(opts) {
  const { title, series, format, integer = false } = opts;
  const fig = el("figure");
  fig.append(el("h3", {}, title));
  const legend = el("div", { class: "chart-legend" });
  for (const s of series) {
    const item = el("span");
    item.append(el("i", { class: s.cls }), document.createTextNode(s.label));
    legend.append(item);
  }
  fig.append(legend);
  const W = 960, H = 340, L = 44, R = 16, T = 12, B = 28;
  const root = svg("svg", { viewBox: `0 0 ${W} ${H}`, role: "img", "aria-label": title });
  const all = series.flatMap((s) => s.points);
  // Four grid steps of a round size; whole numbers on integer axes.
  const max = 4 * niceStep(Math.max(...all.map((p) => p.y)) / 4, integer);
  const xs = series[0].points.map((p) => p.x);
  const x0 = xs[0], x1 = xs[xs.length - 1];
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
  const hovers = [];
  for (const s of series) {
    if (s.points.length > 1) {
      const d = s.points.map((p, i) => `${i ? "L" : "M"}${sx(p.x).toFixed(1)},${sy(p.y).toFixed(1)}`).join("");
      root.append(svg("path", { class: `series ${s.cls}`, d }));
    }
    const last = s.points[s.points.length - 1];
    root.append(svg("circle", { class: `dot ${s.cls}`, cx: sx(last.x), cy: sy(last.y), r: 4 }));
    const label = svg("text", { class: `end-label ${s.cls}`, x: sx(last.x) - 8, y: sy(last.y) - 8, "text-anchor": "end" });
    label.textContent = format(last.y);
    root.append(label);
    hovers.push(svg("circle", { class: `dot ${s.cls}`, r: 4, visibility: "hidden" }));
  }
  const cross = svg("line", { class: "crosshair", y1: T, y2: H - B, visibility: "hidden" });
  root.append(cross, ...hovers);
  const hit = svg("rect", { x: L, y: 0, width: W - L - R, height: H, fill: "transparent" });
  root.append(hit);
  const tip = $("tooltip");
  hit.addEventListener("pointermove", (ev) => {
    const box = root.getBoundingClientRect();
    const px = ((ev.clientX - box.left) / box.width) * W;
    let i = 0;
    xs.forEach((x, j) => {
      if (Math.abs(sx(x) - px) < Math.abs(sx(xs[i]) - px)) i = j;
    });
    cross.setAttribute("x1", sx(xs[i]));
    cross.setAttribute("x2", sx(xs[i]));
    cross.setAttribute("visibility", "visible");
    series.forEach((s, k) => {
      hovers[k].setAttribute("cx", sx(s.points[i].x));
      hovers[k].setAttribute("cy", sy(s.points[i].y));
      hovers[k].setAttribute("visibility", "visible");
    });
    tip.textContent = `${fmtDate(xs[i])} · ${series.map((s) => `${s.label} ${format(s.points[i].y)}`).join(" · ")}`;
    tip.hidden = false;
    tip.style.left = `${ev.clientX + 12}px`;
    tip.style.top = `${ev.clientY + 12}px`;
  });
  hit.addEventListener("pointerleave", () => {
    cross.setAttribute("visibility", "hidden");
    hovers.forEach((h) => h.setAttribute("visibility", "hidden"));
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

const GO_HEAD = ["celestia-app version", "Nodes"];
const RUST_HEAD = ["celestia-rs version", "Nodes"];
const RUST_NODES_HEAD = ["Node ID", "Moniker", "celestia-rs version", "Source"];
const UNKNOWN_RELEASE = "Unknown (release unavailable)";

// `0.38.17-celestia-rs.0.1.0-rc.9` → `v0.1.0-rc.9`; null without the suffix.
function rustVersion(handshake) {
  const i = handshake.indexOf("-celestia-rs.");
  return i < 0 ? null : `v${handshake.slice(i + "-celestia-rs.".length)}`;
}

// [label, count] rows, one per distinct key(node), most nodes first; nodes
// whose key is null are counted under `unknown`, always the last row.
function countRows(nodes, key, unknown) {
  const counts = new Map();
  let missing = 0;
  for (const n of nodes) {
    const k = key(n);
    if (k === null) missing += 1;
    else counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  const rows = [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  if (missing) rows.push([unknown, missing]);
  return rows.map(([label, count]) => [label, String(count)]);
}

function render() {
  const net = state.network;
  document.querySelectorAll(".filters button").forEach((b) =>
    b.setAttribute("aria-pressed", String(b.dataset.network === net)),
  );
  const entries = (state.history?.entries ?? []).filter((e) => e.network === net);
  const latest = state.latest[net];
  if (!entries.length || !latest) {
    $("status").textContent = `No crawl data for ${net} yet.`;
    $("tiles").replaceChildren();
    $("charts").replaceChildren();
    table("go-versions", GO_HEAD, [], [1]);
    table("rust-versions", RUST_HEAD, [], [1]);
    table("rust-nodes", RUST_NODES_HEAD, []);
    $("release-coverage").textContent = "";
    return;
  }
  const s = latest.summary;
  // Nodes with an unrecognized version string (for example a crawler) are
  // left out of every count, table and chart.
  const known = latest.nodes.filter((n) => n.implementation !== "unknown");
  $("status").textContent = `Latest crawl: ${fmtDate(latest.crawled_at)} · chain ${latest.chain_id} · ${entries.length} crawls recorded`;
  const tiles = [
    [s.rust, "celestia-rs nodes"],
    [s.go, "Go nodes"],
    [s.rust + s.go, "Observed nodes"],
    [known.filter((n) => n.source === "p2p").length, "Reached directly"],
  ];
  $("tiles").replaceChildren(
    ...tiles.map(([v, label]) => {
      const d = el("div", { class: "tile" });
      d.append(el("b", {}, String(v)), el("span", {}, label));
      return d;
    }),
  );
  const pts = (f) => entries.map((e) => ({ x: e.crawled_at, y: f(e.summary) }));
  $("charts").replaceChildren(
    lineChart({
      title: "Consensus nodes by implementation",
      series: [
        { label: "Go (celestia-app)", cls: "go", points: pts((x) => x.go) },
        { label: "Rust", cls: "rust", points: pts((x) => x.rust) },
      ],
      format: (v) => String(Math.round(v)),
      integer: true,
    }),
  );
  const go = known.filter((n) => n.implementation === "go");
  const rust = known.filter((n) => n.implementation === "rust");
  // Go releases come from peer metadata or the node's own RPC.
  table("go-versions", GO_HEAD, countRows(go, (n) => (n.app_release ? `v${n.app_release}` : null), UNKNOWN_RELEASE), [1]);
  table("rust-versions", RUST_HEAD, countRows(rust, (n) => rustVersion(n.version), "Unknown"), [1]);
  table(
    "rust-nodes",
    RUST_NODES_HEAD,
    rust.map((n) => [el("code", {}, n.id.slice(0, 12) + "…"), n.moniker, rustVersion(n.version) ?? "–", n.source]),
  );
  const withRelease = go.filter((n) => n.app_release).length;
  $("release-coverage").textContent =
    `The latest ${net} crawl found the app release for ${withRelease} of ${go.length} Go nodes.`;
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
