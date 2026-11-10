// Hop-style hints (hop.nvim): the page is dimmed, and each clickable
// element's first characters are replaced by its code, in the page's own
// font. `__zwebHop.show(typed)` keeps the codes that start with `typed`
// and shows what is left of them; `__zwebHop.clear()` removes it all.
// Returns one JSON string: [{k, x, y, href, edit}], (x, y) the element's
// centre in viewport CSS pixels.
//
// The reading part (targets, character boxes, fonts, colours) only reads the
// page: zweb runs it ahead, when the page is still (`__zwebHopPrepare()`),
// and `f` then only draws, unless the page moved or changed since.
window.__zwebHopPrepare = window.__zwebHopPrepare || (() => {
  let changed = true;
  new MutationObserver(() => { changed = true; })
    .observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  const stamp = () => [scrollX, scrollY, innerWidth, innerHeight].join();
  let ready = null;
  const prepare = () => { ready = scan(); ready.stamp = stamp(); changed = false; };
  prepare.take = () => {
    const r = ready && !changed && ready.stamp === stamp() ? ready : scan();
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

  return { found, codes, spots, pageBg, colors, dimmed };
}
})();

JSON.stringify((() => {
  if (window.__zwebHop) window.__zwebHop.clear();
  const { found, codes, spots, pageBg, colors, dimmed } = window.__zwebHopPrepare.take();
  const DIM = 0.6;
  const rgb = c => `rgb(${c.map(Math.round).join(',')})`;
  const root = document.createElement('div');
  root.style.cssText = 'all:initial;position:fixed;inset:0;pointer-events:none;z-index:2147483647;contain:strict';
  const dim = document.createElement('div');
  dim.style.cssText = `position:fixed;inset:0;background:${rgb(pageBg)};opacity:${DIM}`;
  root.appendChild(dim);
  const layer = document.createElement('div');
  root.appendChild(layer);
  document.documentElement.appendChild(root);

  // One box a target, over its first characters: as wide as the code, and
  // then to the end of the last character it reaches, so none shows in part.
  // Styles set through the DOM: font names hold quotes.
  const measure = document.createElement('canvas').getContext('2d');
  const show = typed => {
    const out = document.createDocumentFragment();
    codes.forEach((code, i) => {
      if (!code.startsWith(typed)) return;
      const rest = code.slice(typed.length), sp = spots[i], c0 = sp.chars[0];
      measure.font = `700 ${sp.size} ${sp.family}`;
      const right = c0.l + measure.measureText(rest).width;
      const reached = sp.chars.filter(c => c.l < right - 0.5 && Math.abs(c.t - c0.t) < c0.h / 2);
      const end = Math.max(right, ...reached.map(c => c.l + c.w));
      const box = document.createElement('span');
      Object.assign(box.style, {
        position: 'fixed', left: c0.l + 'px', top: c0.t + 'px', width: end - c0.l + 'px', height: c0.h + 'px',
        font: `700 ${sp.size}/${c0.h}px ${sp.family}`, background: dimmed(sp.bg), whiteSpace: 'pre',
      });
      [...rest].forEach((ch, j) => {
        const letter = document.createElement('span');
        letter.textContent = ch;
        letter.style.color = colors[Math.min(j, 2)];
        box.appendChild(letter);
      });
      out.appendChild(box);
    });
    layer.replaceChildren(out);
  };
  window.__zwebHop = { show, clear: () => { root.remove(); delete window.__zwebHop; } };
  show('');

  return found.map((f, i) => {
    const tag = f.e.tagName;
    const edit = f.e.isContentEditable || tag === 'TEXTAREA' ||
      (tag === 'INPUT' && !/^(button|submit|reset|checkbox|radio|file|image|range|color)$/i.test(f.e.type));
    return { k: codes[i], x: f.x, y: f.y, href: f.e.href && f.e.href.length < 2048 ? f.e.href : null, edit };
  });
})())
