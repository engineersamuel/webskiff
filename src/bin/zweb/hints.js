// Hop-style hints (hop.nvim): the page is dimmed, and each clickable
// element's first characters are replaced by its code, in the page's own
// font. `__zwebHop.show(typed)` keeps the codes that start with `typed`
// and shows what is left of them; `__zwebHop.clear()` removes it all.
// Returns one JSON string: [{k, x, y, href, edit}], (x, y) the element's
// centre in viewport CSS pixels.
//
// The overlay is one canvas. With `window.__zwebHopImage` set (Kitty
// graphics), it is not put in the page: the result is {id, png, list}, the
// canvas as a PNG for zweb to show as a terminal image over the page, and
// `show(typed)` returns the new PNG. Chromium then draws nothing, and no page
// tiles change. Without it, the canvas goes in the page.
//
// All the work (targets, character boxes, fonts, colours, the overlay, the
// PNG) is done ahead, when the page is still: `__zwebHopPrepare()` returns
// that result (the PNG made in Chromium's idle time, so keys do not wait for
// it), and zweb keeps it, and says so in `__zwebHopKept`. `f` then returns
// only {id} while that is still right: the page did not move, and did not
// change, or changed but its hints read again are the same.
window.__zwebHopPrepare = window.__zwebHopPrepare || (() => {
  let changed = true;
  new MutationObserver(() => { changed = true; })
    .observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  const stamp = () => [scrollX, scrollY, innerWidth, innerHeight].join();
  let ready = null, id = 0;
  const prepare = () => {
    const r = ready = build(scan());
    r.stamp = stamp(); r.id = `${performance.timeOrigin}:${++id}`; changed = false;
    if (!window.__zwebHopImage) return null;
    return new Promise(done => r.root.toBlob(blob => {
      const read = new FileReader();
      read.onload = () => {
        r.png = read.result.slice(read.result.indexOf(',') + 1);
        done(JSON.stringify({ id: r.id, png: r.png, list: r.list }));
      };
      read.readAsDataURL(blob);
    }, 'image/png'));
  };
  prepare.take = () => {
    let r = ready && ready.stamp === stamp() ? ready : null;
    const sc = r && !changed ? null : scan();
    if (sc && !(r && sc.sig === r.sig)) r = build(sc);
    if (window.__zwebHopImage && !r.png) r.png = png(r.root);
    ready = null;
    return r;
  };
  return prepare;
function scan() {
  const sel = 'a[href],button,input:not([type=hidden]),textarea,select,summary,label[for],' +
    '[onclick],[role=button],[role=link],[role=tab],[role=menuitem],[role=checkbox],' +
    '[contenteditable=""],[contenteditable=true],[tabindex]:not([tabindex="-1"])';
  const vw = innerWidth, vh = innerHeight;
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

  // Codes from hop's keys, prefix-free; the first targets in reading order
  // get one letter.
  const keys = 'asdghklqwertyuiopzxcvbnmfj';
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
  const DIM = 0.6;
  const rgb = c => `rgb(${c.map(Math.round).join(',')})`;
  const dimmed = c => rgb(c.map((v, i) => v * (1 - DIM) + pageBg[i] * DIM));

  // Where each code goes: over the element's first visible characters.
  const spots = found.map(f => {
    const walker = document.createTreeWalker(f.e, NodeFilter.SHOW_TEXT);
    for (let n; (n = walker.nextNode());) {
      const i = n.data.search(/\S/);
      if (i < 0) continue;
      const range = document.createRange();
      const chars = [];
      for (let j = i; j < n.data.length && chars.length < 8; j++) {
        range.setStart(n, j); range.setEnd(n, j + 1);
        const r = range.getBoundingClientRect();
        if (r.width < 1) { if (chars.length) break; else continue; }
        chars.push({ l: r.left, t: r.top, w: r.width, h: r.height });
      }
      if (!chars.length || chars[0].t > vh || chars[0].t + chars[0].h < 0) continue;
      const s = getComputedStyle(n.parentElement);
      return { chars, size: s.fontSize, family: s.fontFamily, bg: bgOf(n.parentElement) || pageBg };
    }
    // No text (an icon, a field): at its top-left corner.
    const h = Math.min(Math.max(f.r.height, 14), 18);
    return { chars: [{ l: Math.max(f.r.left, 0), t: Math.max(f.r.top, 0), w: h * 0.62, h }],
      size: Math.round(h * 0.8) + 'px', family: '-apple-system, sans-serif', bg: bgOf(f.e) || pageBg };
  });

  const list = found.map((f, i) => {
    const tag = f.e.tagName;
    const edit = f.e.isContentEditable || tag === 'TEXTAREA' ||
      (tag === 'INPUT' && !/^(button|submit|reset|checkbox|radio|file|image|range|color)$/i.test(f.e.type));
    return { k: codes[i], x: f.x, y: f.y, href: f.e.href && f.e.href.length < 2048 ? f.e.href : null, edit };
  });
  // All that the overlay and the result are made of.
  const sig = JSON.stringify([list, spots, pageBg]);
  return { list, codes, spots, pageBg, colors, dimmed, sig };
}

// The overlay, not attached, with the codes all shown, and the result.
function build({ list, codes, spots, pageBg, colors, dimmed, sig }) {
  const DIM = 0.6;
  const rgb = c => `rgb(${c.map(Math.round).join(',')})`;
  const vw = innerWidth, vh = innerHeight, px = devicePixelRatio || 1;
  const root = document.createElement('canvas');
  root.width = Math.round(vw * px); root.height = Math.round(vh * px);
  root.style.cssText = `all:initial;position:fixed;left:0;top:0;width:${vw}px;height:${vh}px;` +
    'pointer-events:none;z-index:2147483647';
  const g = root.getContext('2d');
  g.scale(px, px);

  // One box a target, over its first characters: as wide as the code, and
  // then to the end of the last character it reaches, so none shows in part.
  // The code sits on the line as the page's own text would.
  const fonts = spots.map(sp => `700 ${sp.size} ${sp.family}`);
  const backs = spots.map(sp => dimmed(sp.bg));
  const show = typed => {
    g.clearRect(0, 0, vw, vh);
    g.globalAlpha = DIM; g.fillStyle = rgb(pageBg); g.fillRect(0, 0, vw, vh); g.globalAlpha = 1;
    let font = null;
    codes.forEach((code, i) => {
      if (!code.startsWith(typed)) return;
      const rest = code.slice(typed.length), sp = spots[i], c0 = sp.chars[0];
      if (font !== fonts[i]) g.font = font = fonts[i];
      const m = g.measureText(rest);
      const right = c0.l + m.width;
      let end = right;
      for (const c of sp.chars) {
        if (c.l < right - 0.5 && Math.abs(c.t - c0.t) < c0.h / 2) end = Math.max(end, c.l + c.w);
      }
      g.fillStyle = backs[i];
      g.fillRect(c0.l, c0.t, end - c0.l, c0.h);
      const asc = m.fontBoundingBoxAscent, base = c0.t + (c0.h - asc - m.fontBoundingBoxDescent) / 2 + asc;
      let x = c0.l;
      for (let j = 0; j < rest.length; j++) {
        g.fillStyle = colors[Math.min(j, 2)];
        g.fillText(rest[j], x, base);
        x += g.measureText(rest[j]).width;
      }
    });
  };
  show('');

  if (!window.__zwebHopImage) return { root, show, list, sig };
  return { root, show: typed => (show(typed), png(root)), list, sig };
}

function png(canvas) {
  return canvas.toDataURL('image/png').slice('data:image/png;base64,'.length);
}
})();

(() => {
  if (window.__zwebHop) window.__zwebHop.clear();
  const { root, show, list, png, id } = window.__zwebHopPrepare.take();
  if (png) {
    window.__zwebHop = { show, clear: () => { delete window.__zwebHop; } };
    return JSON.stringify(id && id === window.__zwebHopKept ? { id } : { png, list });
  }
  document.documentElement.appendChild(root);
  window.__zwebHop = { show, clear: () => { root.remove(); delete window.__zwebHop; } };
  return JSON.stringify(list);
})()
