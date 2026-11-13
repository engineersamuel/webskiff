// Hop-style hints (hop.nvim): the page is dimmed, and each clickable
// element's first characters are replaced by its code, in the page's own
// font. `__zwebHop.show(typed)` keeps the codes that start with `typed`
// and shows what is left of them; `__zwebHop.clear()` removes it all.
// Returns one JSON string: [{k, x, y, href, edit, t}], (x, y) the element's
// centre in viewport CSS pixels, t its text (at most 64 characters).
// `window.__zwebHopKeys` gives the code keys (digits for filtered hints);
// unset or empty, hop's letters. In the page, `__zwebHop.pick(indices)`
// shows only those targets' whole codes.
//
// With `window.__zwebHopGrid = [cols, rows]` set (Kitty graphics), nothing
// goes in the page: zweb shows the hints as terminal images over it, a dim
// layer and one small PNG "atlas" of the code boxes. Each box is cut to the
// whole cells it covers, see-through around the box, and `cells` says where:
// [col, row, cols, rows, x, y, w, h], (x, y, w, h) its pixels in the atlas.
// The result is {id, png, cells, dim, list}; `show(typed)` returns
// {png, cells} of the boxes left. Chromium then draws nothing, and no page
// tiles change. Without it, the overlay is one canvas in the page.
//
// All the work (targets, character boxes, fonts, colours, the overlay, the
// PNG) is done ahead, when the page is still: `__zwebHopPrepare()` returns
// that result, and zweb keeps it, and says so in `__zwebHopKept`. `f` then returns
// only {id} while that is still right: the page did not move, and did not
// change, or changed but its hints read again are the same. One is kept for
// each set of code keys.
window.__zwebHopPrepare = window.__zwebHopPrepare || (() => {
  let changes = 0;
  new MutationObserver(() => { changes++; })
    .observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  const stamp = () => [scrollX, scrollY, innerWidth, innerHeight].join();
  const ready = {};
  let id = 0;
  const prepare = (keys = '') => {
    const r = ready[keys] = build(scan(keys));
    r.stamp = stamp(); r.id = `${performance.timeOrigin}:${++id}`; r.changes = changes;
    if (!r.atlas) return null;
    // At once, not by toBlob: a page that draws no frames never calls it
    // back, and zweb's later scripts would wait behind it.
    r.png = png(r.atlas.canvas);
    return JSON.stringify(r.result({ id: r.id }));
  };
  prepare.take = (keys = '') => {
    let r = ready[keys] && ready[keys].stamp === stamp() ? ready[keys] : null;
    const sc = r && r.changes === changes ? null : scan(keys);
    if (sc && !(r && sc.sig === r.sig)) r = build(sc);
    if (r.atlas && !r.png) r.png = png(r.atlas.canvas);
    delete ready[keys];
    return r;
  };
  return prepare;
function targets(vw, vh) {
  const sel = 'a[href],button,input:not([type=hidden]),textarea,select,summary,label[for],' +
    '[onclick],[role=button],[role=link],[role=tab],[role=menuitem],[role=checkbox],' +
    '[contenteditable=""],[contenteditable=true],[tabindex]:not([tabindex="-1"])';
  const found = [];
  for (const e of document.querySelectorAll(sel)) {
    const r = [...e.getClientRects()].find(r => r.width > 1 && r.height > 1);
    if (!r || r.bottom < 0 || r.top > vh || r.right < 0 || r.left > vw) continue;
    const x = Math.min(Math.max((Math.max(r.left, 0) + Math.min(r.right, vw)) / 2, 1), vw - 1);
    const y = Math.min(Math.max((Math.max(r.top, 0) + Math.min(r.bottom, vh)) / 2, 1), vh - 1);
    const hit = document.elementFromPoint(x, y);
    if (!hit || !(hit === e || e.contains(hit) || hit.contains(e))) continue;
    if (found.some(f => f.e.contains(e) && f.e.tagName === 'A')) continue;
    found.push({ e, r, x, y });
  }
  return found;
}

function characterBoxes(node, start) {
  const range = document.createRange();
  const chars = [];
  for (let j = start; j < node.data.length && chars.length < 8; j++) {
    range.setStart(node, j); range.setEnd(node, j + 1);
    const r = range.getBoundingClientRect();
    if (r.width < 1) { if (chars.length) break; else continue; }
    chars.push({ l: r.left, t: r.top, w: r.width, h: r.height });
  }
  return chars;
}

function hintSpot(f, vh, bgOf, pageBg) {
  const walker = document.createTreeWalker(f.e, NodeFilter.SHOW_TEXT);
  for (let n; (n = walker.nextNode());) {
    const i = n.data.search(/\S/);
    if (i < 0) continue;
    const chars = characterBoxes(n, i);
    if (!chars.length || chars[0].t > vh || chars[0].t + chars[0].h < 0) continue;
    const s = getComputedStyle(n.parentElement);
    return { chars, size: s.fontSize, family: s.fontFamily, bg: bgOf(n.parentElement) || pageBg };
  }
  // No text (an icon, a field): at its top-left corner.
  const h = Math.min(Math.max(f.r.height, 14), 18);
  return { chars: [{ l: Math.max(f.r.left, 0), t: Math.max(f.r.top, 0), w: h * 0.62, h }],
    size: Math.round(h * 0.8) + 'px', family: '-apple-system, sans-serif', bg: bgOf(f.e) || pageBg };
}

// What a target says: its text, a field's value or placeholder, else its
// label, title or image's alt; spaces collapsed, at most 64 characters.
function textOf(e) {
  const field = e.tagName === 'INPUT' || e.tagName === 'TEXTAREA' || e.tagName === 'SELECT';
  const clean = t => (t || '').replace(/\s+/g, ' ').trim();
  const t = (field ? clean(e.value) || clean(e.placeholder) : clean(e.textContent.slice(0, 1000))) ||
    clean(e.getAttribute('aria-label')) || clean(e.title) || clean(e.querySelector('img[alt]')?.alt);
  return t.slice(0, 64);
}

function scan(keys) {
  const vw = innerWidth, vh = innerHeight;
  const found = targets(vw, vh);
  // Codes from hop's keys, prefix-free; the first targets in reading order
  // get one letter.
  keys = keys || 'asdghklqwertyuiopzxcvbnmfj';
  let codes = [''], offset = 0;
  while (codes.length - offset < found.length || codes.length === 1) {
    const c = codes[offset++];
    for (const ch of keys) codes.push(c + ch);
  }
  codes = codes.slice(offset, offset + found.length);

  // Colours: the first background up the tree that is not see-through.
  const parse = c => (c.match(/[\d.]+/g) || []).map(Number);
  const bgOf = el => {
    for (; el; el = el.parentElement) {
      const c = parse(getComputedStyle(el).backgroundColor);
      if (c.length >= 3 && (c.length < 4 || c[3] > 0.5)) return c.slice(0, 3);
    }
    return null;
  };
  const pageBg = bgOf(document.body) || bgOf(document.documentElement) || [255, 255, 255];
  const dark = (0.299 * pageBg[0] + 0.587 * pageBg[1] + 0.114 * pageBg[2]) < 128;
  const colors = dark ? ['#ff007c', '#00dfff', '#2b8db3'] : ['#e0006b', '#0086a8', '#2b6f99'];
  const DIM = 0.75;
  const rgb = c => `rgb(${c.map(Math.round).join(',')})`;
  const dimmed = c => rgb(c.map((v, i) => v * (1 - DIM) + pageBg[i] * DIM));

  // Where each code goes: over the element's first visible characters.
  const spots = found.map(f => hintSpot(f, vh, bgOf, pageBg));

  const list = found.map((f, i) => {
    const tag = f.e.tagName;
    const edit = f.e.isContentEditable || tag === 'TEXTAREA' ||
      (tag === 'INPUT' && !/^(button|submit|reset|checkbox|radio|file|image|range|color)$/i.test(f.e.type));
    return { k: codes[i], x: f.x, y: f.y, href: f.e.href && f.e.href.length < 2048 ? f.e.href : null, edit, t: textOf(f.e) };
  });
  // All that the overlay and the result are made of.
  const sig = JSON.stringify([list, spots, pageBg]);
  return { list, codes, spots, pageBg, colors, dimmed, sig };
}

// The overlay with the codes all shown, and the result.
function build({ list, codes, spots, pageBg, colors, dimmed, sig }) {
  const DIM = 0.75;
  const rgb = c => `rgb(${c.map(Math.round).join(',')})`;
  const vw = innerWidth, vh = innerHeight, px = devicePixelRatio || 1;

  // One box a target, over its first characters: as wide as the code, and
  // then to the end of the last character it reaches, so none shows in part.
  // The code sits on the line as the page's own text would.
  // `paint(g, i, rest)` draws it at its place in viewport CSS pixels.
  const measure = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
  const fonts = spots.map(sp => `700 ${sp.size} ${sp.family}`);
  const backs = spots.map(sp => dimmed(sp.bg));
  const box = (i, rest) => {
    const sp = spots[i], c0 = sp.chars[0];
    measure.font = fonts[i];
    const right = c0.l + measure.measureText(rest).width;
    let end = right;
    for (const c of sp.chars) {
      if (c.l < right - 0.5 && Math.abs(c.t - c0.t) < c0.h / 2) end = Math.max(end, c.l + c.w);
    }
    return { l: c0.l, t: c0.t, w: end - c0.l, h: c0.h };
  };
  const paint = (g, i, rest, b) => {
    g.font = fonts[i];
    g.fillStyle = backs[i];
    g.fillRect(b.l, b.t, b.w, b.h);
    const m = g.measureText(rest);
    const asc = m.fontBoundingBoxAscent, base = b.t + (b.h - asc - m.fontBoundingBoxDescent) / 2 + asc;
    let x = b.l;
    for (let j = 0; j < rest.length; j++) {
      g.fillStyle = colors[Math.min(j, 2)];
      g.fillText(rest[j], x, base);
      x += g.measureText(rest[j]).width;
    }
  };
  const left = typed => codes.flatMap((code, i) => code.startsWith(typed) ? [[i, code.slice(typed.length)]] : []);

  const grid = window.__zwebHopGrid;
  if (grid) {
    // The atlas: each box in the whole cells it covers, packed in shelves.
    const [cols, rows] = grid, cw = vw / cols, ch = vh / rows;
    const make = typed => {
      const parts = left(typed).map(([i, rest]) => {
        const b = box(i, rest);
        const c0 = Math.max(Math.floor(b.l / cw), 0), c1 = Math.min(Math.ceil((b.l + b.w) / cw), cols);
        const r0 = Math.max(Math.floor(b.t / ch), 0), r1 = Math.min(Math.ceil((b.t + b.h) / ch), rows);
        const w = Math.max(Math.round((c1 - c0) * cw * px), 1), h = Math.max(Math.round((r1 - r0) * ch * px), 1);
        return { i, rest, b, c0, r0, nc: Math.max(c1 - c0, 1), nr: Math.max(r1 - r0, 1), w, h };
      });
      const width = Math.max(Math.ceil(vw * px), ...parts.map(p => p.w));
      let x = 0, y = 0, shelf = 0;
      for (const p of parts) {
        if (x + p.w > width) { x = 0; y += shelf; shelf = 0; }
        p.x = x; p.y = y; x += p.w; shelf = Math.max(shelf, p.h);
      }
      const canvas = document.createElement('canvas');
      canvas.width = width; canvas.height = Math.max(y + shelf, 1);
      // In memory, not on the GPU: the PNG is read back at once, and a GPU
      // read back waits for the GPU (seconds where it is slow or stuck).
      const g = canvas.getContext('2d', { willReadFrequently: true });
      for (const p of parts) {
        g.setTransform(px, 0, 0, px, p.x - p.c0 * cw * px, p.y - p.r0 * ch * px);
        g.save();
        g.beginPath(); g.rect(p.c0 * cw, p.r0 * ch, p.w / px, p.h / px); g.clip();
        paint(g, p.i, p.rest, p.b);
        g.restore();
      }
      return { canvas, cells: parts.map(p => [p.c0, p.r0, p.nc, p.nr, p.x, p.y, p.w, p.h]) };
    };
    const atlas = make('');
    const dim = [...pageBg.map(Math.round), Math.round(DIM * 255)];
    const r = {
      atlas, list, sig,
      result: extra => Object.assign({ png: r.png, cells: atlas.cells, dim, list }, extra),
      show: typed => {
        const a = make(typed);
        return { png: a.cells.length ? png(a.canvas) : '', cells: a.cells };
      },
    };
    return r;
  }

  // In the page: one canvas, dimmed, with the boxes.
  const root = document.createElement('canvas');
  root.width = Math.round(vw * px); root.height = Math.round(vh * px);
  root.style.cssText = `all:initial;position:fixed;left:0;top:0;width:${vw}px;height:${vh}px;` +
    'pointer-events:none;z-index:2147483647';
  const g = root.getContext('2d');
  g.scale(px, px);
  const draw = parts => {
    g.clearRect(0, 0, vw, vh);
    g.globalAlpha = DIM; g.fillStyle = rgb(pageBg); g.fillRect(0, 0, vw, vh); g.globalAlpha = 1;
    for (const [i, rest] of parts) paint(g, i, rest, box(i, rest));
  };
  const show = typed => draw(left(typed));
  const pick = ids => draw(ids.map(i => [i, codes[i]]));
  show('');
  return { root, show, pick, list, sig };
}

function png(canvas) {
  return canvas.toDataURL('image/png').slice('data:image/png;base64,'.length);
}
})();

(() => {
  if (window.__zwebHop) window.__zwebHop.clear();
  const r = window.__zwebHopPrepare.take(window.__zwebHopKeys);
  const { root, show, pick, list } = r;
  if (r.atlas) {
    window.__zwebHop = { show: typed => JSON.stringify(show(typed)), clear: () => { delete window.__zwebHop; } };
    return JSON.stringify(r.id && r.id === window.__zwebHopKept ? { id: r.id } : r.result({}));
  }
  document.documentElement.appendChild(root);
  window.__zwebHop = { show, pick, clear: () => { root.remove(); delete window.__zwebHop; } };
  return JSON.stringify(list);
})()
