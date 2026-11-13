// Visual mode. Loaded on `v` only. The page's own selection is the cursor:
// each key extends it with getSelection().modify (app.rs `visual_script`),
// Chromium paints it, and `text()` gives it to the app on `y`.
window.__zwebCaret = window.__zwebCaret || (() => {
    // The first text at the top left of the viewport, scanned row by row.
    const firstVisible = () => {
        for (let y = 4; y < innerHeight; y += 16) {
            for (let x = 4; x < innerWidth; x += 48) {
                const r = document.caretRangeFromPoint(x, y);
                const n = r && r.startContainer;
                if (n && n.nodeType === 3 && n.data.trim()) return r;
            }
        }
        return null;
    };
    const reveal = () => {
        const s = getSelection();
        if (!s.focusNode) return;
        const r = document.createRange();
        r.setStart(s.focusNode, s.focusOffset);
        const b = r.getBoundingClientRect();
        if (!b.height) return;
        if (b.bottom > innerHeight) scrollBy(0, b.bottom - innerHeight);
        else if (b.top < 0) scrollBy(0, b.top);
    };
    // A find match is kept: it starts there. Else the first visible text.
    const start = () => {
        const s = getSelection();
        if (s.rangeCount && !s.isCollapsed) {
            s.collapseToStart();
        } else {
            const r = firstVisible();
            if (!r) return false;
            s.removeAllRanges();
            s.addRange(r);
        }
        s.modify("extend", "forward", "character");
        reveal();
        return true;
    };
    const collapse = () => {
        const s = getSelection();
        if (s.rangeCount) s.collapseToEnd();
    };
    const text = () => {
        const t = getSelection().toString();
        collapse();
        return t;
    };
    return { start, reveal, collapse, text };
})();
