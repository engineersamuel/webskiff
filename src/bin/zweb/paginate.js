// Finds the next (true) or previous (false) page link. Runs only on a key press.
// Returns its absolute href, or null.
(next => {
  const rels = next ? ['next'] : ['prev', 'previous'];
  const words = next ? ['next', '›', '»', 'older', 'more'] : ['previous', 'prev', '‹', '«', 'newer'];
  const byRel = tag => document.querySelector(rels.map(r => `${tag}[href][rel~=${r} i]`).join());
  const labelled = a => [a.textContent, a.getAttribute('aria-label') || '']
    .some(t => words.includes(t.trim().toLowerCase()));
  const link = byRel('link') || byRel('a')
    || [...document.querySelectorAll('a[href]')].find(a => a.getClientRects().length && labelled(a));
  return link ? link.href : null;
})
