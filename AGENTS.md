# AGENTS.md

## Speed comes first

Speed is the most important property of zweb. Never sacrifice speed.

- Target: every interaction under 10 ms, from key press to pixels written.
- Do not accept a change that makes any interaction slower, or sends more bytes
  to the terminal, without the user's explicit approval.
- Measure before and after every change that touches input, rendering or the
  page scripts. Report the numbers (p50 and p90).
- When a look or a feature costs speed, offer a faster way first, and ask.

## Measuring

- `ZWEB_TRACE=1` logs the time of each stage to `~/.cache/zweb/zweb.log`.
- Harnesses in `bench/` (run from that directory, with the zweb binary as the
  first argument): `lat.py` (scroll latency), `inter.py` (page down/up,
  typing, hints, tab switch), `bench.py` (bytes per action), `scrollcpu.py`
  (CPU while scrolling), `idle.py` (bytes and CPU on an idle page).
  Example: `python3 -I lat.py ../target/release/zweb https://en.wikipedia.org/wiki/Cat 30`
- `zweb --selftest` must pass.
