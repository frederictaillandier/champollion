// Small SVG charts for this site: horizontal bars, columns (linear or log)
// and lines. Each chart gets value labels, an optional target line, a hover
// tooltip and a table view, and redraws when its width or the theme changes.
// Colours are the CSS tokens --series-1..3 (validated palette).
(function () {
  "use strict";
  const NS = "http://www.w3.org/2000/svg";
  const BAR = 22; // bar thickness, at most 24px
  const R = 4; // rounded data end

  function css(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }
  function el(name, attrs, parent) {
    const node = document.createElementNS(NS, name);
    for (const [k, v] of Object.entries(attrs || {})) node.setAttribute(k, v);
    if (parent) parent.appendChild(node);
    return node;
  }
  function text(parent, x, y, str, cls, anchor) {
    const t = el("text", { x, y, class: cls, "text-anchor": anchor || "start" }, parent);
    t.textContent = str;
    return t;
  }
  const measureCtx = document.createElement("canvas").getContext("2d");
  function textWidth(str, font) {
    measureCtx.font = font;
    return measureCtx.measureText(str).width;
  }
  function niceStep(span, count) {
    const raw = span / count;
    const mag = Math.pow(10, Math.floor(Math.log10(raw)));
    const norm = raw / mag;
    return (norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 2.5 ? 2.5 : norm <= 5 ? 5 : 10) * mag;
  }
  function ticks(max, count) {
    const step = niceStep(max, count);
    const top = Math.ceil(max / step) * step;
    const out = [];
    for (let v = 0; v <= top + step / 2; v += step) out.push(+v.toFixed(6));
    return out;
  }
  function compact(v) {
    if (v >= 1000) return (v / 1000).toLocaleString("en", { maximumFractionDigits: 1 }) + "K";
    return v.toLocaleString("en", { maximumFractionDigits: 2 });
  }

  // A path for a bar with a rounded data end and a square base.
  function hbarPath(x0, x1, y, h) {
    const r = Math.min(R, (x1 - x0) / 2, h / 2);
    return `M${x0},${y}H${x1 - r}Q${x1},${y} ${x1},${y + r}V${y + h - r}Q${x1},${y + h} ${x1 - r},${y + h}H${x0}Z`;
  }
  function vbarPath(x, w, y0, y1) {
    const r = Math.min(R, (y0 - y1) / 2, w / 2);
    return `M${x},${y0}V${y1 + r}Q${x},${y1} ${x + r},${y1}H${x + w - r}Q${x + w},${y1} ${x + w},${y1 + r}V${y0}Z`;
  }

  function setup(root, opts) {
    root.classList.add("chart");
    root.innerHTML = "";
    if (opts.legend && opts.legend.length >= 2) {
      const legend = document.createElement("div");
      legend.className = "legend";
      for (const item of opts.legend) {
        const span = document.createElement("span");
        const swatch = document.createElement("i");
        swatch.style.background = `var(--${item.color})`;
        span.append(swatch, item.name);
        legend.appendChild(span);
      }
      root.appendChild(legend);
    }
    const holder = document.createElement("div");
    holder.style.position = "relative";
    root.appendChild(holder);
    const tip = document.createElement("div");
    tip.className = "tooltip";
    tip.hidden = true;
    holder.appendChild(tip);
    if (opts.table) {
      const details = document.createElement("details");
      details.className = "data";
      const summary = document.createElement("summary");
      summary.textContent = "Show the data as a table";
      const wrap = document.createElement("div");
      wrap.className = "table-wrap";
      const table = document.createElement("table");
      const head = table.createTHead().insertRow();
      opts.table.head.forEach((h, i) => {
        const th = document.createElement("th");
        th.textContent = h;
        if (i > 0) th.className = "num";
        head.appendChild(th);
      });
      const body = table.createTBody();
      for (const row of opts.table.rows) {
        const tr = body.insertRow();
        row.forEach((c, i) => {
          const td = tr.insertCell();
          td.textContent = c;
          if (i > 0) td.className = "num";
        });
      }
      wrap.appendChild(table);
      details.append(summary, wrap);
      root.appendChild(details);
    }
    return { holder, tip };
  }

  function showTip(tip, html, x, y) {
    tip.innerHTML = html;
    tip.hidden = false;
    const holderWidth = tip.parentElement.clientWidth;
    const half = tip.offsetWidth / 2;
    tip.style.left = Math.max(half, Math.min(holderWidth - half, x)) + "px";
    tip.style.top = y + "px";
  }

  // Horizontal bars: opts.data = [{label, value, color, note}]
  function drawBars(root, opts, parts) {
    const { holder, tip } = parts;
    holder.querySelector("svg")?.remove();
    const width = Math.max(300, holder.clientWidth);
    const fmt = opts.format || compact;
    const labelFont = `13px ${css("--sans")}`;
    const narrow = width < 520;
    const labelW = narrow ? 0 : Math.min(width * 0.42, Math.max(...opts.data.map((d) => textWidth(d.label, labelFont))) + 14);
    const valueW = Math.max(...opts.data.map((d) => textWidth(fmt(d.value), `600 12.5px ${css("--sans")}`))) + 12;
    const row = narrow ? 52 : 38;
    const top = opts.target ? 26 : 8;
    const plotX = labelW;
    const plotW = width - labelW - valueW - 4;
    const max = opts.max || Math.max(...opts.data.map((d) => d.value), opts.target ? opts.target.value : 0);
    const tickVals = ticks(max, narrow ? 3 : 5);
    const domain = tickVals[tickVals.length - 1];
    const sx = (v) => plotX + (v / domain) * plotW;
    const height = top + opts.data.length * row + 24;
    const svg = el("svg", { viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": opts.title || "" });
    holder.insertBefore(svg, tip);

    for (const t of tickVals) {
      const x = sx(t);
      el("line", { x1: x, x2: x, y1: top, y2: top + opts.data.length * row, class: t === 0 ? "baseline" : "grid" }, svg);
      text(svg, x, height - 6, opts.tick ? opts.tick(t) : compact(t), "axis-text", "middle");
    }
    opts.data.forEach((d, i) => {
      const y0 = top + i * row;
      const barY = narrow ? y0 + 22 : y0 + (row - BAR) / 2;
      if (narrow) text(svg, plotX, y0 + 15, d.label, "cat-text");
      else text(svg, plotX - 12, barY + BAR / 2 + 4.5, d.label, "cat-text", "end");
      const x1 = Math.max(sx(d.value), plotX + 1);
      const mark = el("path", { d: hbarPath(plotX, x1, barY, BAR), fill: css(`--${d.color || "series-1"}`), class: "mark" }, svg);
      // A label starting on the target line moves just past it.
      let labelX = x1 + 6;
      if (opts.target) {
        const tx = sx(opts.target.value);
        if (labelX > tx - 8 && labelX < tx + 6) labelX = tx + 6;
      }
      text(svg, labelX, barY + BAR / 2 + 4.5, fmt(d.value), "label-text");
      const hit = el("rect", { x: 0, y: y0, width, height: row, class: "hit" }, svg);
      hit.addEventListener("mousemove", (e) => {
        mark.classList.add("hover");
        const box = holder.getBoundingClientRect();
        showTip(tip, `<b>${d.label}</b><br>${fmt(d.value)}${opts.unit ? " " + opts.unit : ""}${d.note ? "<br>" + d.note : ""}`, e.clientX - box.left, (barY / height) * holder.clientHeight);
      });
      hit.addEventListener("mouseleave", () => {
        mark.classList.remove("hover");
        tip.hidden = true;
      });
    });
    if (opts.target) {
      const x = sx(opts.target.value);
      if (narrow) {
        // Labels sit above the bars here: the line only crosses the bars.
        opts.data.forEach((_, i) => {
          const barY = top + i * row + 22;
          el("line", { x1: x, x2: x, y1: barY - 3, y2: barY + BAR + 3, class: "target" }, svg);
        });
      } else {
        el("line", { x1: x, x2: x, y1: top - 6, y2: top + opts.data.length * row, class: "target" }, svg);
      }
      const anchor = x > width - 120 ? "end" : x < plotX + 60 ? "start" : "middle";
      text(svg, x, top - 11, opts.target.label, "target-text", anchor);
    }
  }

  // Columns: opts.data = [{label, value, color, note}], opts.log for a log scale.
  function drawColumns(root, opts, parts) {
    const { holder, tip } = parts;
    holder.querySelector("svg")?.remove();
    const width = Math.max(300, holder.clientWidth);
    const fmt = opts.format || compact;
    const height = opts.height || 300;
    const left = 64;
    const bottom = 44;
    const top = 26;
    const plotW = width - left - 8;
    const plotH = height - top - bottom;
    const values = opts.data.map((d) => d.value);
    let sy, tickVals;
    if (opts.log) {
      const lo = opts.logMin || Math.pow(10, Math.floor(Math.log10(Math.min(...values))));
      const hi = Math.pow(10, Math.ceil(Math.log10(Math.max(...values))));
      sy = (v) => top + plotH - ((Math.log10(v) - Math.log10(lo)) / (Math.log10(hi) - Math.log10(lo))) * plotH;
      tickVals = opts.logTicks || [];
    } else {
      tickVals = ticks(opts.max || Math.max(...values), 4);
      const domain = tickVals[tickVals.length - 1];
      sy = (v) => top + plotH - (v / domain) * plotH;
    }
    const svg = el("svg", { viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": opts.title || "" });
    holder.insertBefore(svg, tip);
    for (const t of tickVals) {
      const v = typeof t === "object" ? t.value : t;
      const y = sy(v);
      el("line", { x1: left, x2: width - 8, y1: y, y2: y, class: "grid" }, svg);
      text(svg, left - 8, y + 4, typeof t === "object" ? t.label : compact(t), "axis-text", "end");
    }
    const base = top + plotH;
    el("line", { x1: left, x2: width - 8, y1: base, y2: base, class: "baseline" }, svg);
    const band = plotW / opts.data.length;
    const w = Math.min(24, band * 0.6);
    opts.data.forEach((d, i) => {
      const cx = left + band * i + band / 2;
      const y1 = sy(d.value);
      const mark = el("path", { d: vbarPath(cx - w / 2, w, base, y1), fill: css(`--${d.color || "series-1"}`), class: "mark" }, svg);
      text(svg, cx, y1 - 7, fmt(d.value), "label-text", "middle");
      const lines = String(d.label).split("\n");
      lines.forEach((line, j) => text(svg, cx, base + 17 + j * 14, line, "axis-text", "middle"));
      const hit = el("rect", { x: cx - band / 2, y: top, width: band, height: plotH + bottom, class: "hit" }, svg);
      hit.addEventListener("mousemove", () => {
        mark.classList.add("hover");
        showTip(tip, `<b>${d.label.replace("\n", " ")}</b><br>${fmt(d.value)}${d.note ? "<br>" + d.note : ""}`, (cx / width) * holder.clientWidth, (y1 / height) * holder.clientHeight);
      });
      hit.addEventListener("mouseleave", () => {
        mark.classList.remove("hover");
        tip.hidden = true;
      });
    });
  }

  // Lines: opts.x = [..], opts.series = [{name, color, values}] (null = no point).
  function drawLines(root, opts, parts) {
    const { holder, tip } = parts;
    holder.querySelector("svg")?.remove();
    const width = Math.max(300, holder.clientWidth);
    const fmt = opts.format || compact;
    const height = opts.height || 280;
    const left = 48;
    const right = width < 520 ? 16 : 120;
    const top = 16;
    const bottom = 44;
    const plotW = width - left - right;
    const plotH = height - top - bottom;
    const all = opts.series.flatMap((s) => s.values.filter((v) => v != null));
    const tickVals = ticks(Math.max(...all, opts.target ? opts.target.value : 0), 4);
    const domain = tickVals[tickVals.length - 1];
    const sx = (i) => left + (opts.x.length === 1 ? plotW / 2 : (i / (opts.x.length - 1)) * plotW);
    const sy = (v) => top + plotH - (v / domain) * plotH;
    const svg = el("svg", { viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": opts.title || "" });
    holder.insertBefore(svg, tip);
    for (const t of tickVals) {
      el("line", { x1: left, x2: left + plotW, y1: sy(t), y2: sy(t), class: t === 0 ? "baseline" : "grid" }, svg);
      text(svg, left - 8, sy(t) + 4, compact(t), "axis-text", "end");
    }
    opts.x.forEach((x, i) => text(svg, sx(i), top + plotH + 18, String(x), "axis-text", "middle"));
    if (opts.xLabel) text(svg, left + plotW / 2, height - 4, opts.xLabel, "axis-text", "middle");
    if (opts.target) {
      el("line", { x1: left, x2: left + plotW, y1: sy(opts.target.value), y2: sy(opts.target.value), class: "target" }, svg);
      text(svg, left + 4, sy(opts.target.value) - 6, opts.target.label, "target-text");
    }
    const ends = [];
    for (const s of opts.series) {
      const color = css(`--${s.color}`);
      const pts = s.values.map((v, i) => (v == null ? null : [sx(i), sy(v), v])).filter(Boolean);
      el("path", { d: pts.map((p, i) => (i ? "L" : "M") + p[0] + "," + p[1]).join(""), stroke: color, "stroke-width": 2, fill: "none", "stroke-linejoin": "round", "stroke-linecap": "round" }, svg);
      for (const p of pts) el("circle", { cx: p[0], cy: p[1], r: 4.5, fill: color, stroke: css("--surface"), "stroke-width": 2 }, svg);
      const last = pts[pts.length - 1];
      ends.push({ s, x: last[0], y: last[1], v: last[2] });
    }
    if (right > 40) {
      ends.sort((a, b) => a.y - b.y);
      for (let i = 1; i < ends.length; i++) if (ends[i].y - ends[i - 1].y < 30) ends[i].y = ends[i - 1].y + 30;
      for (const e of ends) {
        text(svg, e.x + 10, e.y - 2, fmt(e.v), "label-text");
        text(svg, e.x + 10, e.y + 13, e.s.name, "axis-text");
      }
    }
    const cross = el("line", { x1: 0, x2: 0, y1: top, y2: top + plotH, class: "grid", visibility: "hidden" }, svg);
    const hit = el("rect", { x: left - 10, y: top, width: plotW + 20, height: plotH, class: "hit" }, svg);
    hit.addEventListener("mousemove", (e) => {
      const box = svg.getBoundingClientRect();
      const px = ((e.clientX - box.left) / box.width) * width;
      let best = 0;
      opts.x.forEach((_, i) => { if (Math.abs(sx(i) - px) < Math.abs(sx(best) - px)) best = i; });
      cross.setAttribute("x1", sx(best));
      cross.setAttribute("x2", sx(best));
      cross.setAttribute("visibility", "visible");
      const rows = opts.series
        .filter((s) => s.values[best] != null)
        .map((s) => `${s.name}: <b>${fmt(s.values[best])}</b>${opts.unit ? " " + opts.unit : ""}`)
        .join("<br>");
      showTip(tip, `<b>${opts.xName || ""} ${opts.x[best]}</b><br>${rows}`, (sx(best) / width) * holder.clientWidth, (top / height) * holder.clientHeight + 30);
    });
    hit.addEventListener("mouseleave", () => {
      cross.setAttribute("visibility", "hidden");
      tip.hidden = true;
    });
  }

  function mount(root, opts, draw) {
    const parts = setup(root, opts);
    const redraw = () => draw(root, opts, parts);
    redraw();
    let lastWidth = parts.holder.clientWidth;
    new ResizeObserver(() => {
      if (parts.holder.clientWidth !== lastWidth) {
        lastWidth = parts.holder.clientWidth;
        redraw();
      }
    }).observe(parts.holder);
    window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", redraw);
  }

  window.Charts = {
    bars: (root, opts) => mount(root, opts, drawBars),
    columns: (root, opts) => mount(root, opts, drawColumns),
    lines: (root, opts) => mount(root, opts, drawLines),
  };
})();
