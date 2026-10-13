#!/usr/bin/env node
// Renders the browser simulation (docs/research/simulasi-konsep-ui.html) headlessly and writes
// every screen of the mission-control concept as plain text into docs/redesign/screens/.
// These files are the acceptance targets of docs/redesign/06-build-order.md, the way
// docs/screenshots/*.txt are for the shell of today. Run: node dev/sim-screens.js
const fs = require('fs');
const path = require('path');
const root = path.join(__dirname, '..');
const html = fs.readFileSync(path.join(root, 'docs/research/simulasi-konsep-ui.html'), 'utf8');
const src = html.slice(html.indexOf('<script>') + 8, html.lastIndexOf('</script>'));
const stub = { clientWidth: 2000, style: {}, innerHTML: '', setAttribute() {}, getAttribute() { return null; }, addEventListener() {}, focus() {}, onclick: null };
const document = { querySelector: () => stub, documentElement: stub, addEventListener() {}, activeElement: stub };
const window = { addEventListener() {}, matchMedia: () => ({ matches: false }) };
const localStorage = { getItem: () => 'done', setItem() {} };
const sim = new Function('document', 'window', 'localStorage', 'matchMedia', 'setInterval', 'setTimeout',
  src + '\nreturn {S, render, buf: () => buf, setSize, go, floatToggle, openPalette, explain, openWizard, selectNode, cancelAsk, startTour, noteStart};'
)(document, window, localStorage, window.matchMedia, () => 0, () => 0);
const out = path.join(root, 'docs/redesign/screens');
fs.mkdirSync(out, { recursive: true });
function text() { return sim.buf().c.map(r => r.map(c => c.ch).join('').replace(/\s+$/, '')).join('\n') + '\n'; }
function shot(name, w, h, setup) {
  const S = sim.S;
  Object.assign(S, { zoom: null, view: 'dash', focus: 'nodes', preset: 0, density: 'comfortable', replay: 0, palette: null, explain: null, wiz: null, dialog: null, tour: null, note: null, toasts: [], filter: { on: false, q: '' } });
  S.float.open = false; S.float.prefix = false; S.float.which = false;
  sim.setSize(w, h);
  if (setup) setup(S);
  sim.render();
  fs.writeFileSync(path.join(out, `${w}x${h}-${name}.txt`), text());
}
const zoom = id => S => { S.zoom = id; S.view = id; S.focus = id; };
for (const [w, h] of [[120, 36], [80, 24], [160, 48]]) {
  shot('dash', w, h);
  shot('dash-preset-fleet', w, h, S => { S.preset = 1; });
  shot('dash-preset-ops', w, h, S => { S.preset = 2; });
  for (const id of ['fleet', 'queue', 'nodes', 'node', 'airflow', 'jira', 'tape', 'sessions']) shot('zoom-' + id, w, h, zoom(id));
}
shot('dash-replay', 120, 36, S => { S.replay = 12; });
shot('dash-density-compact', 120, 36, S => { S.density = 'compact'; });
shot('dash-density-focus', 120, 36, S => { S.density = 'focus'; });
shot('palette', 120, 36, () => sim.openPalette(false));
shot('palette-help', 120, 36, () => sim.openPalette(true));
shot('palette-typed', 120, 36, S => { sim.openPalette(false); S.palette.q = 'exp'; });
shot('explain-node', 120, 36, S => { S.focus = 'nodes'; sim.explain(); });
shot('explain-user', 120, 36, S => { S.zoom = 'node'; S.view = 'node'; S.sel.user = 0; sim.explain(); });
shot('float-session', 120, 36, () => sim.floatToggle(true));
shot('float-session-prefix', 120, 36, S => { sim.floatToggle(true); S.float.prefix = true; S.float.which = true; });
shot('wizard', 120, 36, () => sim.openWizard(0));
shot('cancel-dialog', 120, 36, S => { S.zoom = 'queue'; S.view = 'queue'; S.cur.queue = 1; sim.cancelAsk(); });
shot('tour', 120, 36, () => sim.startTour());
shot('note', 120, 36, S => { S.zoom = 'tape'; S.view = 'tape'; sim.noteStart(); S.note = 'ch3 hot since the 14:30 reload; watching #7438'; });
console.log('wrote', fs.readdirSync(out).length, 'screens to', out);
