// Keep one search origin so extending or deleting the query cannot skip a match.
(() => {
    const selection = getSelection();
    const origin = selection.rangeCount ? selection.getRangeAt(0).cloneRange() : null;
    const x = scrollX, y = scrollY;
    const restore = () => {
        selection.removeAllRanges();
        if (origin && origin.startContainer.isConnected && origin.endContainer.isConnected) {
            selection.addRange(origin);
        }
    };
    window.__zwebFind = {
        preview(text) {
            restore();
            const found = text !== "" && window.find(text, false, false, true);
            if (!found) scrollTo(x, y);
            return found;
        },
        finish(accept) {
            if (!accept) {
                restore();
                scrollTo(x, y);
            } else {
                document.activeElement?.blur();
            }
            delete window.__zwebFind;
        }
    };
})();
