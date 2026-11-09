// Labels every visible clickable element; returns [{k, x, y, href, edit}].
// Coordinates are the element's centre in viewport CSS pixels.
(() => {
  const old = document.getElementById('__zweb_hints');
  if (old) old.remove();
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
  const abc = 'asdfghjkl';
  let len = 1;
  while (abc.length ** len < found.length) len++;
  const label = i => { let s = ''; for (let j = 0; j < len; j++) { s = abc[i % abc.length] + s; i = Math.floor(i / abc.length); } return s; };
  const box = document.createElement('div');
  box.id = '__zweb_hints';
  box.style.cssText = 'all:initial;position:fixed;inset:0;pointer-events:none;z-index:2147483647';
  const out = found.map((f, i) => {
    const k = label(i);
    const s = document.createElement('span');
    s.textContent = k.toUpperCase();
    s.style.cssText = 'all:initial;position:fixed;font:bold 12px monospace;padding:0 2px;' +
      'background:#ffd400;color:#000;border:1px solid #c38a22;border-radius:3px;' +
      `left:${Math.max(f.r.left, 0)}px;top:${Math.max(f.r.top, 0)}px`;
    s.dataset.k = k;
    box.appendChild(s);
    const tag = f.e.tagName;
    const edit = f.e.isContentEditable || tag === 'TEXTAREA' ||
      (tag === 'INPUT' && !/^(button|submit|reset|checkbox|radio|file|image|range|color)$/i.test(f.e.type));
    return { k, x: f.x, y: f.y, href: f.e.href || null, edit };
  });
  document.documentElement.appendChild(box);
  return out;
})()
