// Page search. `__zwebSearch` stays on the page: `mark(text)` paints every
// match yellow (CSS highlights: the page DOM does not change), `focus()` gives
// the link at the current match the keyboard focus, so Enter opens it, and
// `step(text, back)` moves to the next match.
window.__zwebSearch = window.__zwebSearch || (() => {
    const style = document.createElement("style");
    style.textContent = "::highlight(zweb-find){background-color:rgba(255,220,0,.45)}";
    let marked = "";
    const mark = text => {
        if (text === marked) return;
        marked = text;
        if (!text) return CSS.highlights.delete("zweb-find");
        const q = text.toLowerCase(), all = new Highlight();
        const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT);
        for (let n; (n = walker.nextNode());) {
            const s = n.data.toLowerCase();
            for (let i = s.indexOf(q); i >= 0; i = s.indexOf(q, i + q.length)) {
                const r = new Range();
                r.setStart(n, i);
                r.setEnd(n, i + q.length);
                all.add(r);
            }
        }
        if (!style.isConnected) document.documentElement.append(style);
        CSS.highlights.set("zweb-find", all);
    };
    const focus = () => {
        const selection = getSelection();
        document.activeElement?.blur();
        const node = selection.rangeCount && selection.getRangeAt(0).startContainer;
        const link = node && (node.nodeType === 1 ? node : node.parentElement)?.closest("a[href]");
        link?.focus({ preventScroll: true });
    };
    const step = (text, back) => {
        mark(text);
        const found = window.find(text, false, back, true);
        focus();
        return found;
    };
    return { mark, focus, step };
})();
// SEARCH SESSION: one `/` prompt.
// Keep one search origin so extending or deleting the query cannot skip a match.
(() => {
    const selection = getSelection();
    const origin = selection.rangeCount ? selection.getRangeAt(0).cloneRange() : null;
    const x = scrollX, y = scrollY;
    let found = false;
    const restore = () => {
        selection.removeAllRanges();
        if (origin && origin.startContainer.isConnected && origin.endContainer.isConnected) {
            selection.addRange(origin);
        }
    };
    window.__zwebFind = {
        preview(text) {
            restore();
            found = text !== "" && window.find(text, false, false, true);
            if (!found) scrollTo(x, y);
            return found;
        },
        finish(accept, text) {
            if (!accept) {
                restore();
                scrollTo(x, y);
            } else {
                if (text) __zwebSearch.mark(text);
                if (found) __zwebSearch.focus();
            }
            delete window.__zwebFind;
        }
    };
})();
