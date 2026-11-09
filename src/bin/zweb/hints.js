// Finds every visible clickable element; returns [{k, x, y, l, t, href, edit}].
// (x, y) is the element's centre, (l, t) its top-left corner, in viewport CSS
// pixels. The page is not changed: zweb draws the labels in the terminal.
// One JSON string comes back: an array of objects costs ~20 ms to convert.
JSON.stringify((() => {
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
  // Vimium's codes: prefix-free, as short as the count allows, so some
  // targets get one letter. Home row first.
  const abc = 'sadfjklewcmpgh';
  let codes = [''], offset = 0;
  while (codes.length - offset < found.length || codes.length === 1) {
    const c = codes[offset++];
    for (const ch of abc) codes.push(ch + c);
  }
  codes = codes.slice(offset, offset + found.length).sort().map(c => [...c].reverse().join(''));
  const label = i => codes[i];
  return found.map((f, i) => {
    const tag = f.e.tagName;
    const edit = f.e.isContentEditable || tag === 'TEXTAREA' ||
      (tag === 'INPUT' && !/^(button|submit|reset|checkbox|radio|file|image|range|color)$/i.test(f.e.type));
    return { k: label(i), x: f.x, y: f.y, l: Math.max(f.r.left, 0), t: Math.max(f.r.top, 0), href: f.e.href && f.e.href.length < 2048 ? f.e.href : null, edit };
  });
})())
