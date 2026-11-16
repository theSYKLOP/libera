/* Libéra — interface. JavaScript sans dépendance ; dialogue avec le moteur Rust via Tauri. */
(() => {
'use strict';

const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);
const listen = (ev, fn) => T.event.listen(ev, (e) => fn(e.payload));
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];

/* ───────── utilitaires ───────── */
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const GO = 1024 ** 3;
function fmtSize(b) {
  const u = ['o', 'Ko', 'Mo', 'Go', 'To'];
  let v = b, i = 0;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  if (i === 0) return `${b} o`;
  return `${v.toLocaleString('fr-FR', { minimumFractionDigits: 1, maximumFractionDigits: 1 })} ${u[i]}`;
}
const fmtInt = (n) => Number(n).toLocaleString('fr-FR');
const fmtDate = (s) => (s > 0 ? new Date(s * 1000).toLocaleDateString('fr-FR', { day: '2-digit', month: '2-digit', year: 'numeric' }) : '—');
const fmtPct = (p) => `${p.toLocaleString('fr-FR', { minimumFractionDigits: 1, maximumFractionDigits: 1 })} %`;
const fmtDur = (ms) => (ms < 90000 ? `${(ms / 1000).toLocaleString('fr-FR', { maximumFractionDigits: 1 })} s` : `${Math.floor(ms / 60000)} min ${Math.round((ms % 60000) / 1000)} s`);
const plural = (n, one, many) => (n > 1 ? many : one);

/* Logo Libéra : un disque (anneau) qui s'ouvre, et le point qui s'en échappe = l'espace libéré. Même dessin que assets/logo.svg. */
const LOGO = `<svg class="logo" width="24" height="24" viewBox="0 0 64 64" role="img" aria-label="Libéra"><defs><linearGradient id="lbg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#172554"/><stop offset="1" stop-color="#2563eb"/></linearGradient></defs><rect width="64" height="64" rx="15" fill="url(#lbg)"/><path d="M43.3 20.7A16 16 0 1 0 48 32" fill="none" stroke="#fff" stroke-width="5.4" stroke-linecap="round"/><circle cx="51" cy="24" r="3.4" fill="#fff"/></svg>`;
const APP = { nom: 'Libéra', version: '0.1.0', auteur: 'NGUEME OYONE FRANCK MEHDI', titre: 'fondateur de Vortex', licence: 'GPL-3.0' };

function showAbout() {
  modal(`<div class="modal-head" style="display:flex;gap:18px;align-items:center">${LOGO.replace('width="24" height="24"', 'width="56" height="56"')}
      <div><h1 style="font-size:22px">${APP.nom}</h1><p class="lead" style="margin:2px 0 0">Version ${APP.version} · Libère de l'espace, range ton PC, garde la main.</p></div></div>
    <div class="modal-body">
      <div class="kv" style="grid-template-columns:1fr"><div><span>Créé par</span><b style="font-size:17px">${APP.auteur}</b><small>${APP.titre}</small></div></div>
      <p class="muted" style="margin:18px 0 0">Libéra est un logiciel libre sous licence ${APP.licence} : tu peux l'utiliser, l'étudier, le modifier et le partager ; les versions modifiées que tu distribues doivent rester sous la même licence, avec leur code source. Construit avec Rust et Tauri.</p>
      <p class="muted" style="margin:10px 0 0">L'assistant utilise Claude Code, avec ton propre abonnement. Aucune donnée n'est envoyée ailleurs par Libéra.</p>
    </div>
    <div class="modal-foot"><button class="btn primary" id="m-close3">Fermer</button></div>`);
  $('#m-close3').addEventListener('click', closeModal);
}

/* Squelette de chargement : l'utilisateur voit tout de suite la forme de ce qui arrive. */
const skeleton = (rows = 7) => `<div class="table-wrap sk" aria-busy="true" aria-label="Chargement en cours">${Array.from({ length: rows }, (_, i) =>
  `<div class="sk-row" style="--i:${i}"><i class="sk-ico"></i><i class="sk-bar" style="width:${38 + ((i * 17) % 32)}%"></i><i class="sk-bar short"></i></div>`).join('')}</div>`;

/* icônes : un seul jeu, trait de 1,6 */
const PATHS = {
  gauge: 'M4 15a8 8 0 1 1 16 0M12 15l4-5',
  folder: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z',
  file: 'M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5',
  spark: 'M12 3l1.8 4.7L18.5 9.5l-4.7 1.8L12 16l-1.8-4.7L5.5 9.5l4.7-1.8zM19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z',
  grid: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z',
  copy: 'M9 9h11v11H9zM5 15V5a1 1 0 0 1 1-1h10',
  trash: 'M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6',
  play: 'M7 5l12 7-12 7z',
  stop: 'M6 6h12v12H6z',
  chev: 'M9 6l6 6-6 6',
  x: 'M6 6l12 12M18 6L6 18',
  plus: 'M12 5v14M5 12h14',
  check: 'M5 12l5 5 9-10',
  alert: 'M12 4l9 16H3zM12 10v4M12 17h.01',
  ext: 'M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5',
  shield: 'M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z',
  search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4',
  bag: 'M5 8h14l-1 12H6zM9 8V6a3 3 0 0 1 6 0v2',
  disk: 'M4 7a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2zM4 14h16M8 17h.01',
  clock: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2',
  bot: 'M12 3v3M5 9h14a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2zM9 14h.01M15 14h.01',
  send: 'M5 12h14M13 6l6 6-6 6',
  bolt: 'M13 3L5 14h6l-1 7 8-11h-6z',
};
const ico = (n, s = 16) => `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${PATHS[n]}"/></svg>`;

function toast(msg) {
  const t = $('#toast');
  t.textContent = msg;
  t.classList.add('on');
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.remove('on'), 3200);
}

/* ───────── état ───────── */
const MODES = [
  { id: 'rapide', nom: 'Rapide', desc: 'Caches, fichiers temporaires, corbeille et applications.', duree: '≈ 15 s' },
  { id: 'standard', nom: 'Standard', desc: 'Parcours complet du disque : dossiers et gros fichiers.', duree: '1 à 2 min' },
  { id: 'approfondie', nom: 'Approfondie', desc: 'Standard, plus tous les fichiers dès 1 Mo et la recherche de doublons (hors projets de code).', duree: '2 à 4 min' },
  { id: 'experimentale', nom: 'Expérimentale', desc: 'Lit directement la table MFT de NTFS : voit aussi les zones protégées, en quelques secondes. Mesures plus exactes (liens physiques comptés une fois).', duree: '≈ 20 s', admin: true },
];

const S = {
  view: 'apercu',
  status: null,
  summary: null,
  analysing: false,
  progress: null,
  mode: localStorage.getItem('mode') || 'standard',
  drive: 'C:\\',
  goal: Number(localStorage.getItem('objectif_go')) || 300,
  basket: new Map(),
  folderId: 0,
  folderTab: 'dossiers',
  bf: { min: 100, age: 0 },
  appFilter: '',
  appAge: 0,
  appDev: false,
  stale: { months: 12, protectMusic: true, protectDesign: true, report: null, level: 'all', cls: 'all', busy: false },
  settings: { configured: false, protect_dev: true, protect_music: true, protect_design: true, custom_protected: [], allowed_paths: [] },
  chat: { queue: [], msgs: [], busy: false, session: null, tokens: 0, check: null, draft: '', timer: null, model: localStorage.getItem('agent_model') || 'sonnet' },
  pg: null,
  del: null,
  watch: new Map(),
  opts: { recycle: false, fix_permissions: true, schedule_on_reboot: false },
};

const hasScan = () => !!(S.summary && S.summary.scanned);
const isElevated = () => !!(S.status && S.status.elevated);

/* ───────── panier ───────── */
const keyOf = (kind, v) => `${kind}:${String(v).toLowerCase()}`;
function basketSize() { let t = 0; S.basket.forEach((i) => (t += i.size)); return t; }
function inBasket(kind, v) { return S.basket.has(keyOf(kind, v)); }

function addItem(item) {
  const k = keyOf(item.kind, item.kind === 'cat' ? item.id : item.path);
  if (S.basket.has(k)) { S.basket.delete(k); } else { S.basket.set(k, { ...item, key: k }); }
  refreshBasketUI();
}
function removeItem(k) { S.basket.delete(k); refreshBasketUI(); renderDrawer(); }
/* L'assistant voit le même panier que l'interface : on lui en envoie une copie à chaque changement. */
let syncTimer = null;
function syncBasket() {
  clearTimeout(syncTimer);
  syncTimer = setTimeout(() => invoke('basket_sync', { items: [...S.basket.values()] }).catch(() => {}), 120);
}

function refreshBasketUI() {
  syncBasket();
  const b = $('#basket-btn');
  if (b) {
    const n = S.basket.size;
    b.classList.toggle('has', n > 0);
    $('.lbl', b).textContent = n ? `Panier · ${n}` : 'Panier';
    $('.sz', b).textContent = n ? fmtSize(basketSize()) : 'vide';
  }
  updateDiskBar();
  $$('[data-basket-key]').forEach((el) => {
    const on = S.basket.has(el.dataset.basketKey);
    if (el.classList.contains('chk')) { el.checked = on; return; }
    const act = $('.act-add', el);
    if (act) act.innerHTML = on ? `${ico('check', 14)} Dans le panier` : `${ico('plus', 14)} Ajouter`;
  });
}

/* ───────── barre latérale ───────── */
function renderSide() {
  const nav = [
    ['apercu', 'gauge', 'Aperçu', true],
    ['assistant', 'bot', 'Assistant', true],
    ['dossiers', 'folder', 'Dossiers', hasScan()],
    ['fichiers', 'file', 'Gros fichiers', hasScan()],
    ['nettoyage', 'spark', 'Nettoyage', !!S.summary],
    ['inutiles', 'clock', 'Inutilisés', true],
    ['apps', 'grid', 'Applications', !!S.summary],
    ['doublons', 'copy', 'Doublons', !!(S.summary && (S.summary.mode === 'approfondie' || S.summary.mode === 'experimentale'))],
    ['protections', 'shield', 'Protections', true],
  ];
  const n = S.basket.size;
  const adm = isElevated();
  $('#side').innerHTML = `
    <div class="brand">${LOGO}<span>Libéra</span></div>
    ${nav.map(([id, ic, nom, ok]) => `
      <button class="nav-item" data-nav="${id}" ${ok ? '' : 'disabled'} ${S.view === id ? 'aria-current="page"' : ''}
        ${ok ? '' : `title="${id === 'doublons' ? 'Disponible après une analyse Approfondie' : 'Disponible après une analyse Standard'}"`}>
        ${ico(ic)}<span>${nom}</span>
      </button>`).join('')}
    <div class="side-foot">
      <button class="basket-btn ${n ? 'has' : ''}" id="basket-btn">${ico('bag')}<span class="lbl">${n ? `Panier · ${n}` : 'Panier'}</span><span class="sz">${n ? fmtSize(basketSize()) : 'vide'}</span></button>
      <div class="admin-note">${adm
        ? `${ico('shield', 14)} <b>Administrateur</b> : toutes les zones sont accessibles.`
        : `Droits standard. Certaines zones de Windows ne sont accessibles qu'en administrateur. <button class="link-btn" id="admin-btn">Relancer en administrateur</button>`}</div>
      <button class="about" id="about-btn" title="À propos de ${APP.nom}"><span>Par ${APP.auteur}</span><span class="faint">Fondateur de Vortex · v${APP.version}</span></button>
    </div>`;
  $$('[data-nav]').forEach((b) => b.addEventListener('click', () => go(b.dataset.nav)));
  $('#basket-btn').addEventListener('click', openDrawer);
  $('#about-btn').addEventListener('click', showAbout);
  const ab = $('#admin-btn');
  if (ab) ab.addEventListener('click', async () => {
    try { await invoke('relaunch_admin'); } catch (e) { toast(String(e)); }
  });
}

function go(view) {
  S.view = view;
  renderSide();
  render();
  $('#main').scrollTop = 0;
}

/* ───────── rendu principal ───────── */
function render() {
  const m = $('#main');
  m.classList.toggle('chat-mode', S.view === 'assistant');
  const fn = { apercu: viewApercu, assistant: viewAssistant, protections: viewProtections, dossiers: viewDossiers, fichiers: viewFichiers, nettoyage: viewNettoyage, inutiles: viewInutiles, apps: viewApps, doublons: viewDoublons }[S.view];
  m.innerHTML = '<div class="page" id="page"></div>';
  fn($('#page'));
}

/* —— Aperçu —— */
function diskOf() {
  if (S.summary && S.summary.disk) return S.summary.disk;
  if (S.status && S.status.drives.length) return S.status.drives.find((d) => d.root === S.drive) || S.status.drives[0];
  return null;
}

function diskBlock() {
  const d = diskOf();
  if (!d) return '';
  const goalB = S.goal * GO;
  const over = d.used - goalB;
  const sel = basketSize();
  const afterB = d.used - sel;
  let verdict = '';
  if (over <= 0) {
    verdict = `<div class="verdict ok">Objectif atteint : le disque utilise <b>${fmtSize(d.used)}</b>, sous les <b>${S.goal} Go</b> visés.</div>`;
  } else if (sel > 0 && afterB <= goalB) {
    verdict = `<div class="verdict ok">Avec le contenu du panier (<b>${fmtSize(sel)}</b>), tu passes à <b>${fmtSize(afterB)}</b> : objectif atteint.</div>`;
  } else {
    const rest = Math.max(0, afterB - goalB);
    verdict = `<div class="verdict">Pour passer sous <b>${S.goal} Go</b>, il faut libérer <b>${fmtSize(over)}</b>.${sel > 0 ? ` Le panier en couvre <b>${fmtSize(sel)}</b> ; il manque encore <b>${fmtSize(rest)}</b>.` : ''}</div>`;
  }
  return `
  <div class="disk" id="disk-block">
    <div class="disk-line">
      <div class="big"><b>${fmtSize(d.used)}</b> utilisés sur ${fmtSize(d.total)} · ${fmtSize(d.free)} libres <span class="faint">(${esc(d.root)})</span></div>
      <label class="goal-edit muted">Objectif<input id="goal-in" type="number" min="10" max="5000" value="${S.goal}" aria-label="Espace utilisé visé en Go"> Go max.</label>
    </div>
    <div class="diskbar" id="diskbar">
      <div class="used" style="--p:${d.used / d.total}"></div>
      <div class="sel"></div>
      <div class="goal" data-label="Objectif"></div>
    </div>
    <div class="legend"><span><i style="background:var(--primary);opacity:.82"></i>Utilisé</span><span><i style="background:var(--accent)"></i>Dans le panier</span><span><i style="background:var(--ok)"></i>Objectif</span></div>
    <div id="verdict">${verdict}</div>
  </div>`;
}

function updateDiskBar() {
  const d = diskOf();
  const bar = $('#diskbar');
  if (!d || !bar) return;
  const sel = Math.min(basketSize(), d.used);
  const used = d.used / d.total;
  const selFrac = sel / d.total;
  const s = $('.sel', bar);
  s.style.setProperty('--a', used - selFrac);
  s.style.setProperty('--b', selFrac);
  $('.goal', bar).style.left = `${Math.min(100, (S.goal * GO / d.total) * 100)}%`;
  $('.used', bar).style.setProperty('--p', used);
  const blk = $('#disk-block');
  if (blk) {
    const tmp = document.createElement('div');
    tmp.innerHTML = diskBlock();
    $('#verdict', blk).innerHTML = $('#verdict', tmp).innerHTML;
  }
}

async function viewApercu(p) {
  const hasSum = !!S.summary;
  p.innerHTML = `
    <div class="page-head"><div>
      <h1>${hasSum ? 'Où part ton espace' : 'Analyser le disque'}</h1>
      <p class="lead">${hasSum
        ? `Analyse ${esc(S.summary.mode)} terminée en ${fmtDur(S.summary.elapsed_ms)}${S.summary.denied ? ` · ${fmtInt(S.summary.denied)} dossiers protégés non lus (droits administrateur)` : ''}.${S.summary.notice ? ` ${esc(S.summary.notice)}` : ''}`
        : 'Choisis le niveau d\'analyse. Rien n\'est supprimé tant que tu ne l\'as pas confirmé.'}</p>
    </div>${hasSum && !S.analysing ? `<button class="btn" id="again">${ico('play', 14)} Nouvelle analyse</button>` : ''}</div>
    ${S.settings.configured ? '' : `<div class="callout" style="margin:0 0 22px;display:flex;gap:14px;align-items:center;justify-content:space-between">
      <span>${ico('shield', 14)} <b>Rien n'est encore choisi.</b> Dis à Libéra ce qu'il doit garder (code, musique, création…). En attendant, tout est protégé par prudence.</span>
      <button class="btn sm" id="banner-prot">Choisir mes protections</button></div>`}
    ${diskBlock()}
    <div id="zone" class="section"></div>`;
  const bp = $('#banner-prot'); if (bp) bp.addEventListener('click', () => go('protections'));
  const gi = $('#goal-in');
  if (gi) gi.addEventListener('input', () => {
    S.goal = Math.max(10, Number(gi.value) || 300);
    localStorage.setItem('objectif_go', S.goal);
    updateDiskBar();
  });
  updateDiskBar();
  const again = $('#again');
  if (again) again.addEventListener('click', () => { S.summary = null; renderSide(); render(); });
  const z = $('#zone');
  if (S.analysing) return renderProgress(z);
  if (!hasSum) return renderModePicker(z);
  await renderFindings(z);
}

function renderModePicker(z) {
  const drives = (S.status && S.status.drives) || [];
  z.innerHTML = `
    <div class="modes" role="radiogroup" aria-label="Niveau d'analyse">
      ${MODES.map((m) => {
        const off = m.admin && !isElevated();
        return `
        <label class="mode ${off ? 'off' : ''}"><input type="radio" name="mode" value="${m.id}" ${S.mode === m.id && !off ? 'checked' : ''} ${off ? 'disabled' : ''}>
          <span><span class="t">${m.nom}</span> ${m.admin ? `<span class="badge ${off ? 'danger' : 'ok'}">${off ? 'Administrateur requis' : 'Administrateur'}</span>` : ''}<br><span class="d">${m.desc}</span></span>
          <span class="dur">${m.duree}</span></label>`;
      }).join('')}
    </div>
    <div class="start-row">
      <button class="btn primary lg" id="start">${ico('play', 15)} Lancer l'analyse</button>
      ${drives.length > 1 ? `<select class="select" id="drive" aria-label="Disque">${drives.map((d) => `<option value="${esc(d.root)}" ${d.root === S.drive ? 'selected' : ''}>${esc(d.root)}  ·  ${fmtSize(d.used)} utilisés</option>`).join('')}</select>` : ''}
    </div>`;
  if (S.mode === 'experimentale' && !isElevated()) S.mode = 'standard';
  $$('input[name=mode]', z).forEach((r) => { r.checked = r.value === S.mode; r.addEventListener('change', () => { S.mode = r.value; localStorage.setItem('mode', S.mode); }); });
  const dr = $('#drive');
  if (dr) dr.addEventListener('change', () => { S.drive = dr.value; updateDiskBar(); });
  $('#start').addEventListener('click', startAnalysis);
}

async function startAnalysis() {
  try {
    S.analysing = true;
    S.progress = { files: 0, dirs: 0, bytes: 0, expected: 0, current: '', phase: 'Préparation' };
    await invoke('start_analysis', { mode: S.mode, root: S.drive });
    render();
  } catch (e) { S.analysing = false; toast(String(e)); render(); }
}

/* Étapes affichées pendant une analyse, selon le mode. Les noms viennent du moteur. */
const PHASES = {
  rapide: ['Zones nettoyables', 'Applications installées'],
  standard: ['Zones nettoyables', 'Parcours du disque', 'Applications installées'],
  approfondie: ['Zones nettoyables', 'Parcours du disque', 'Applications installées', 'Recherche de doublons'],
  experimentale: ['Zones nettoyables', 'Lecture de la table MFT', 'Applications installées', 'Recherche de doublons'],
  inutiles: ['Recherche des fichiers inutilisés'],
};
const fmtEta = (s) => (s < 60 ? `${Math.max(5, Math.round(s / 5) * 5)} s` : `${Math.round(s / 60)} min`);

function renderProgress(z, kind = S.mode) {
  const phases = PHASES[kind] || PHASES.standard;
  S.pg = { t0: Date.now(), phaseT0: Date.now(), phase: '', phases };
  z.innerHTML = `
    <div class="progress" aria-live="polite">
      <ol class="steps">${phases.map((n) => `<li data-ph="${esc(n)}"><i></i>${esc(n)}</li>`).join('')}</ol>
      <div class="pg-head"><b class="big tnum" id="pp"></b><span class="muted" id="ph">Préparation…</span></div>
      <div class="pbar ind" id="pbar"><i></i></div>
      <div class="pg-sub"><span id="pv"></span><span id="pe"></span></div>
      <div class="stats">
        <div><span>Fichiers lus</span><b id="pf">0</b></div>
        <div><span>Dossiers</span><b id="pd">0</b></div>
        <div><span>Volume</span><b id="pb">0 o</b></div>
      </div>
      <div class="cur" id="pc" title=""></div>
      <div style="margin-top:22px"><button class="btn" id="cancel">${ico('stop', 14)} Annuler</button></div>
    </div>`;
  $('#cancel').addEventListener('click', (e) => { invoke('cancel'); e.currentTarget.disabled = true; e.currentTarget.lastChild.textContent = ' Annulation…'; });
  paintProgress();
}

function paintProgress() {
  const p = S.progress, g = S.pg;
  if (!p || !g || !$('#ph')) return;
  if (p.phase !== g.phase) { g.phase = p.phase; g.phaseT0 = Date.now(); }
  const idx = g.phases.indexOf(p.phase);
  $$('.steps li').forEach((li, i) => { li.className = idx < 0 ? '' : i < idx ? 'done' : i === idx ? 'cur' : ''; });
  $('#ph').textContent = `${p.phase}…`;
  // Pourcentage seulement quand la quantité attendue est connue ; sinon barre indéterminée + temps écoulé.
  const pct = p.expected > 0 ? Math.min(99, (p.bytes / p.expected) * 100) : null;
  const bar = $('#pbar');
  bar.classList.toggle('ind', pct === null);
  if (pct !== null) bar.style.setProperty('--p', pct / 100);
  $('#pp').textContent = pct === null ? '' : `${Math.floor(pct)} %`;
  $('#pv').textContent = pct === null ? '' : `${fmtSize(p.bytes)} sur ≈ ${fmtSize(p.expected)}`;
  const el = (Date.now() - g.phaseT0) / 1000;
  $('#pe').textContent = pct !== null && pct >= 4 && el >= 4 ? `≈ ${fmtEta((el * (100 - pct)) / pct)} restantes` : `${Math.floor(el)} s écoulées`;
  $('#pf').textContent = fmtInt(p.files);
  $('#pd').textContent = fmtInt(p.dirs);
  $('#pb').textContent = fmtSize(p.bytes);
  const c = $('#pc');
  c.textContent = '‎' + (p.current || '');
  c.title = p.current || '';
}

async function renderFindings(z) {
  const s = S.summary;
  const items = [];
  items.push(['nettoyage', 'spark', 'Zones nettoyables', `${s.junk_count} catégories de caches, journaux et fichiers temporaires`, s.junk_total]);
  items.push(['inutiles', 'clock', 'Fichiers inutilisés', 'Installateurs, archives déjà extraites, captures… sans toucher à tes projets, ta musique ni tes créations', null]);
  if (s.scanned) {
    if (s.rebuildable_total > 0) items.push(['nettoyage', 'folder', 'Dossiers reconstructibles', 'node_modules, target, .next… recréés par les outils de développement', s.rebuildable_total]);
    items.push(['fichiers', 'file', 'Gros fichiers', `${fmtInt(s.big_files)} fichiers de 32 Mo et plus`, null]);
  }
  items.push(['apps', 'grid', 'Applications installées', `${fmtInt(s.apps_count)} applications`, null]);
  if (s.mode === 'approfondie' || s.mode === 'experimentale') items.push(['doublons', 'copy', 'Doublons', `${fmtInt(s.dupes_groups)} groupes de fichiers identiques`, s.dupes_wasted]);
  let top = '';
  if (s.scanned) {
    try {
      const f = await invoke('get_children', { nodeId: 0 });
      top = `<div class="section"><h2>Les plus gros dossiers</h2><div class="table-wrap" style="margin-top:14px"><table>
        <colgroup><col style="width:44%"><col style="width:30%"><col style="width:26%"></colgroup>
        <thead><tr><th>Dossier</th><th>Part du disque analysé</th><th class="num">Taille</th></tr></thead>
        <tbody>${f.children.slice(0, 8).map((r) => `
          <tr tabindex="0" data-open="${r.id}"><td class="c-name"><div class="cn">${ico('folder')}<span class="nm">${esc(r.name)}</span></div></td>
          <td><div class="part"><div class="bar"><i style="width:${Math.max(r.pct, 0.5)}%"></i></div><span>${fmtPct(r.pct)}</span></div></td>
          <td class="num">${fmtSize(r.size)}</td></tr>`).join('')}</tbody></table></div></div>`;
    } catch (_) { /* pas d'arbre */ }
  }
  z.innerHTML = `
    <h2>Pistes pour libérer de l'espace</h2>
    <div class="list" style="margin-top:14px">${items.map(([v, ic, t, d, sz]) => `
      <div class="item" tabindex="0" data-go="${v}"><span class="ico">${ico(ic, 18)}</span>
        <span class="txt"><div class="t">${t}</div><div class="d">${d}</div></span>
        ${sz !== null ? `<span class="sz">${fmtSize(sz)}</span>` : '<span class="sz"></span>'}<span class="chev">${ico('chev')}</span></div>`).join('')}
    </div>${top}`;
  $$('[data-go]', z).forEach((el) => bindRow(el, () => go(el.dataset.go)));
  $$('[data-open]', z).forEach((el) => bindRow(el, () => { S.folderId = Number(el.dataset.open); S.folderTab = 'dossiers'; go('dossiers'); }));
}

function bindRow(el, fn) {
  el.addEventListener('click', (e) => { if (!e.target.closest('[data-act]')) fn(e); });
  el.addEventListener('keydown', (e) => { if (e.key === 'Enter' && e.target === el) fn(e); });
}

/* —— Dossiers —— */
async function viewDossiers(p) {
  p.innerHTML = skeleton();
  let f;
  try { f = await invoke('get_children', { nodeId: S.folderId }); } catch (e) { p.innerHTML = `<div class="empty"><b>Aucune analyse</b>${esc(e)}</div>`; return; }
  S.folderId = f.id;
  const crumbs = f.breadcrumb.map((c, i) => `${i ? ico('chev', 12) : ''}<button data-crumb="${c.id}">${esc(c.name)}</button>`).join('');
  p.innerHTML = `
    <div class="page-head"><div><h1>Dossiers</h1>
      <p class="lead">${fmtSize(f.size)} dans ${fmtInt(f.files)} fichiers${f.own_files ? ` · dont ${fmtSize(f.own_size)} directement ici` : ''}.</p></div>
      <button class="btn" id="reveal-here">${ico('ext', 14)} Ouvrir dans l'Explorateur</button></div>
    <nav class="crumbs" aria-label="Chemin">${crumbs}</nav>
    <div class="toolbar"><div class="seg"><button data-tab="dossiers" aria-pressed="${S.folderTab === 'dossiers'}">Sous-dossiers</button><button data-tab="fichiers" aria-pressed="${S.folderTab === 'fichiers'}">Fichiers ici</button></div>
      <span class="spacer"></span>${f.id !== 0 ? `<button class="btn ghost" id="add-here">${ico('plus', 14)} Ajouter ce dossier au panier</button>` : ''}</div>
    <div id="tbl"></div>`;
  $$('[data-crumb]', p).forEach((b) => b.addEventListener('click', () => { S.folderId = Number(b.dataset.crumb); S.folderTab = 'dossiers'; render(); }));
  $$('[data-tab]', p).forEach((b) => b.addEventListener('click', () => { S.folderTab = b.dataset.tab; render(); }));
  $('#reveal-here').addEventListener('click', () => invoke('reveal', { path: f.path }));
  const ah = $('#add-here');
  if (ah) ah.addEventListener('click', () => addFromRow({ path: f.path, name: f.name, size: f.size }));
  const t = $('#tbl');
  if (S.folderTab === 'dossiers') tableFolders(t, f); else await tableFiles(t, f);
}

function actionsHtml(row, extra = '') {
  const key = keyOf('path', row.path);
  const inb = S.basket.has(key);
  if (row.locked) return `<div class="actions"><span class="badge" title="${esc(row.locked)}">${ico('shield', 12)}&nbsp;Protégé</span></div>`;
  return `<div class="actions">${extra}<button class="btn sm act-add" data-act="add">${inb ? `${ico('check', 14)} Dans le panier` : `${ico('plus', 14)} Ajouter`}</button></div>`;
}

function tableFolders(t, f) {
  if (!f.children.length) { t.innerHTML = '<div class="table-wrap"><div class="empty"><b>Aucun sous-dossier</b>Ce dossier ne contient que des fichiers.</div></div>'; return; }
  t.innerHTML = `<div class="table-wrap"><table>
    <colgroup><col style="width:42%"><col style="width:26%"><col style="width:16%"><col style="width:16%"></colgroup>
    <thead><tr><th>Nom</th><th>Part du dossier</th><th class="num">Taille</th><th class="num">Fichiers</th></tr></thead>
    <tbody>${f.children.map((r) => `
      <tr tabindex="0" class="${r.locked ? 'is-locked' : ''}" data-id="${r.id}" data-has="${r.has_children}" data-path="${esc(r.path)}" data-name="${esc(r.name)}" data-size="${r.size}" data-files="${r.files}" data-basket-key="${keyOf('path', r.path)}" ${r.warning ? `data-warn="${esc(r.warning)}"` : ''}>
        <td class="c-name"><div class="cn">${ico('folder')}<span class="nm" title="${esc(r.path)}">${esc(r.name)}</span>${r.denied ? '<span class="badge warn" title="Certains éléments sont protégés">Accès limité</span>' : ''}</div></td>
        <td><div class="part"><div class="bar"><i style="width:${Math.max(r.pct, 0.4)}%"></i></div><span>${fmtPct(r.pct)}</span></div></td>
        <td class="num">${fmtSize(r.size)}</td>
        <td class="num muted-cell last">${fmtInt(r.files)}${actionsHtml(r, `<button class="btn sm ghost" data-act="reveal">${ico('ext', 14)}</button>`)}</td>
      </tr>`).join('')}</tbody></table></div>`;
  $$('tbody tr', t).forEach((tr) => {
    bindRow(tr, () => {
      if (tr.dataset.has === 'false') { S.folderTab = 'fichiers'; S.folderId = Number(tr.dataset.id); render(); return; }
      S.folderId = Number(tr.dataset.id); render();
    });
  });
  bindRowActions(t);
}

async function tableFiles(t, f) {
  t.innerHTML = skeleton();
  const files = await invoke('get_folder_files', { path: f.path });
  if (!files.length) { t.innerHTML = '<div class="table-wrap"><div class="empty"><b>Aucun fichier directement ici</b>Les fichiers sont dans les sous-dossiers.</div></div>'; return; }
  t.innerHTML = fileTable(files, { dir: false });
  bindRowActions(t);
}

function fileTable(files, { dir = true } = {}) {
  return `<div class="table-wrap"><table>
    <colgroup><col style="width:${dir ? 52 : 58}%"><col style="width:18%"><col style="width:${dir ? 30 : 24}%"></colgroup>
    <thead><tr><th>Nom</th><th>Modifié le</th><th class="num">Taille</th></tr></thead>
    <tbody>${files.map((r) => `
      <tr tabindex="0" class="${r.locked ? 'is-locked' : ''}" data-path="${esc(r.path)}" data-name="${esc(r.name)}" data-size="${r.size}" data-reveal="1" data-basket-key="${keyOf('path', r.path)}" ${r.warning ? `data-warn="${esc(r.warning)}"` : ''}>
        <td class="c-name"><div class="cn">${ico('file')}<span style="min-width:0"><span class="nm" title="${esc(r.path)}">${esc(r.name)}</span>${dir ? `<span class="sub" title="${esc(r.dir)}">${esc(r.dir)}</span>` : ''}</span></div></td>
        <td class="muted-cell">${fmtDate(r.mtime)}</td>
        <td class="num last">${fmtSize(r.size)}${actionsHtml(r)}</td>
      </tr>`).join('')}</tbody></table></div>`;
}

function bindRowActions(root) {
  root.addEventListener('click', (e) => {
    const b = e.target.closest('[data-act]');
    if (!b) return;
    e.stopPropagation();
    const tr = b.closest('tr, .dup-file, .item');
    if (b.dataset.act === 'add') addFromRow({ path: tr.dataset.path, name: tr.dataset.name, size: Number(tr.dataset.size), warning: tr.dataset.warn, files: tr.dataset.files ? Number(tr.dataset.files) : undefined, file: tr.hasAttribute('data-reveal') || tr.classList.contains('dup-file') });
    if (b.dataset.act === 'reveal') invoke('reveal', { path: tr.dataset.path });
  });
  $$('tr[data-reveal]', root).forEach((tr) => bindRow(tr, () => invoke('reveal', { path: tr.dataset.path })));
}

function addFromRow(r) {
  const was = inBasket('path', r.path);
  addItem({ kind: 'path', path: r.path, name: r.name, size: r.size, warning: r.warning || '', file: !!r.file, files: r.files });
  if (!was) toast(`« ${r.name} » ajouté au panier (${fmtSize(r.size)})`);
}

/* —— Gros fichiers —— */
async function viewFichiers(p) {
  p.innerHTML = `
    <div class="page-head"><div><h1>Gros fichiers</h1><p class="lead">Les fichiers les plus volumineux du disque, tous dossiers confondus.</p></div></div>
    <div class="toolbar">
      <label class="muted">Taille minimale <select class="select" id="f-min">${[32, 100, 500, 1024].map((v) => `<option value="${v}" ${S.bf.min === v ? 'selected' : ''}>${v >= 1024 ? v / 1024 + ' Go' : v + ' Mo'}</option>`).join('')}</select></label>
      <label class="muted">Modifié <select class="select" id="f-age">${[[0, 'à tout moment'], [180, 'il y a plus de 6 mois'], [365, 'il y a plus d\'un an'], [730, 'il y a plus de 2 ans']].map(([v, l]) => `<option value="${v}" ${S.bf.age === v ? 'selected' : ''}>${l}</option>`).join('')}</select></label>
    </div><div id="tbl"><div class="skeleton">Chargement…</div></div>`;
  const load = async () => {
    const rows = await invoke('get_big_files', { minMb: S.bf.min, olderThanDays: S.bf.age || null, limit: 300 });
    const t = $('#tbl');
    t.innerHTML = rows.length ? fileTable(rows) : '<div class="table-wrap"><div class="empty"><b>Aucun fichier ne correspond</b>Essaie une taille minimale plus basse.</div></div>';
    bindRowActions(t);
  };
  $('#f-min').addEventListener('change', (e) => { S.bf.min = Number(e.target.value); load(); });
  $('#f-age').addEventListener('change', (e) => { S.bf.age = Number(e.target.value); load(); });
  await load();
  if (S.summary && S.summary.mode !== 'approfondie') {
    $('#page').insertAdjacentHTML('beforeend', `<p class="muted" style="margin-top:18px">L'analyse Standard retient les fichiers de 32 Mo et plus. Lance une analyse Approfondie pour descendre à 1 Mo.</p>`);
  }
}

/* —— Nettoyage —— */
async function viewNettoyage(p) {
  p.innerHTML = skeleton();
  const [junk, reb] = await Promise.all([invoke('get_junk'), hasScan() ? invoke('get_rebuildable') : Promise.resolve([])]);
  const groups = {};
  junk.forEach((j) => { (groups[j.group] ||= []).push(j); });
  const order = ['Windows', 'Navigateurs', 'Applications', 'Développement', 'Système'];
  const catRow = (c) => {
    const k = keyOf('cat', c.id);
    const adm = c.needs_admin && !isElevated();
    return `<label class="item cat" ${adm ? 'title="Nécessite les droits administrateur"' : ''}>
      <input type="checkbox" class="chk" data-cat="${esc(c.id)}" data-basket-key="${k}" ${S.basket.has(k) ? 'checked' : ''}>
      <span class="txt"><div class="t">${esc(c.name)} ${c.risk === 'modere' ? '<span class="badge warn">À vérifier</span>' : ''} ${c.needs_admin ? `<span class="badge ${adm ? 'danger' : ''}">${adm ? 'Administrateur requis' : 'Administrateur'}</span>` : ''}</div><div class="d">${esc(c.description)}</div></span>
      <span class="sz">${c.size_known ? fmtSize(c.size) : '—'}</span></label>`;
  };
  const total = junk.reduce((a, c) => a + c.size, 0);
  p.innerHTML = `
    <div class="page-head"><div><h1>Nettoyage</h1><p class="lead">Zones connues pour être recréées par Windows ou les applications. ${fmtSize(total)} récupérables.</p></div>
      <button class="btn" id="sel-safe">Cocher tout ce qui est sûr</button></div>
    ${order.filter((g) => groups[g]).map((g) => `<div class="group-title">${g}</div><div class="list">${groups[g].map(catRow).join('')}</div>`).join('')}
    ${reb.length ? `<div class="group-title">Dossiers reconstructibles</div>
      <p class="muted" style="margin:-4px 0 12px">Générés par tes outils de développement (dépendances, compilation). Ils sont recréés à la prochaine installation ou compilation.</p>
      <div id="reb"></div>` : ''}`;
  $$('.chk[data-cat]', p).forEach((c) => c.addEventListener('change', () => {
    const cat = junk.find((j) => j.id === c.dataset.cat);
    const k = keyOf('cat', cat.id);
    if (c.checked !== S.basket.has(k)) addItem({ kind: 'cat', id: cat.id, name: cat.name, size: cat.size, admin: cat.needs_admin });
  }));
  $('#sel-safe').addEventListener('click', () => {
    junk.filter((j) => j.risk === 'sur' && j.size > 0 && (!j.needs_admin || isElevated()) && !S.basket.has(keyOf('cat', j.id))).forEach((j) => addItem({ kind: 'cat', id: j.id, name: j.name, size: j.size, admin: j.needs_admin }));
    render();
    toast('Éléments sûrs ajoutés au panier');
  });
  if (reb.length) {
    const t = $('#reb');
    t.innerHTML = `<div class="table-wrap"><table><colgroup><col style="width:60%"><col style="width:40%"></colgroup>
      <thead><tr><th>Dossier</th><th class="num">Taille</th></tr></thead><tbody>${reb.map((r) => `
      <tr tabindex="0" data-path="${esc(r.path)}" data-name="${esc(r.name)}" data-size="${r.size}" data-reveal="1" data-basket-key="${keyOf('path', r.path)}">
        <td class="c-name"><div class="cn">${ico('folder')}<span style="min-width:0"><span class="nm">${esc(r.name)}</span><span class="sub" title="${esc(r.path)}">${esc(r.path)}</span></span></div></td>
        <td class="num last">${fmtSize(r.size)}${actionsHtml(r)}</td></tr>`).join('')}</tbody></table></div>`;
    bindRowActions(t);
  }
}

/* —— Applications —— */
async function viewApps(p) {
  p.innerHTML = skeleton();
  const apps = await invoke('get_apps');
  p.innerHTML = `
    <div class="page-head"><div><h1>Applications</h1><p class="lead">${fmtInt(apps.length)} applications installées. La désinstallation passe par le programme de désinstallation officiel de chacune.</p></div></div>
    <div class="toolbar"><div class="search">${ico('search', 16)}<input id="app-q" type="search" placeholder="Rechercher une application" value="${esc(S.appFilter)}" aria-label="Rechercher"></div>
      <label class="muted">Afficher <select class="select" id="app-age">${[[0, 'toutes les applications'], [90, 'non ouvertes depuis 3 mois'], [180, 'non ouvertes depuis 6 mois'], [365, 'non ouvertes depuis 1 an']].map(([v, l]) => `<option value="${v}" ${S.appAge === v ? 'selected' : ''}>${l}</option>`).join('')}</select></label>
      <label class="opt" style="align-items:center"><input type="checkbox" class="chk" id="app-dev" ${S.appDev ? 'checked' : ''}><span>Inclure aussi les applications protégées par mes réglages</span></label></div>
    <div id="tbl"></div>`;
  const nowS = Date.now() / 1000;
  const installedTs = (a) => { const m = /^(\d{2})\/(\d{2})\/(\d{4})$/.exec(a.install_date || ''); return m ? new Date(`${m[3]}-${m[2]}-${m[1]}`).getTime() / 1000 : 0; };
  const draw = () => {
    const q = S.appFilter.trim().toLowerCase();
    const limit = S.appAge ? nowS - S.appAge * 86400 : 0;
    const rows = apps.filter((a) => {
      if (q && !a.name.toLowerCase().includes(q) && !a.publisher.toLowerCase().includes(q)) return false;
      if (!limit) return true;
      if (kindProtected(a.kind) && !S.appDev) return false; // protégées par tes réglages (et le système, toujours)
      if (a.last_used) return a.last_used < limit;
      const inst = installedTs(a); // jamais ouverte : on ne la propose que si elle est installée depuis longtemps
      return inst > 0 && inst < limit;
    });
    const t = $('#tbl');
    t.innerHTML = rows.length ? `<div class="table-wrap"><table>
      <colgroup><col style="width:46%"><col style="width:20%"><col style="width:16%"><col style="width:18%"></colgroup>
      <thead><tr><th>Application</th><th>Dernière ouverture</th><th>Installée le</th><th class="num">Taille</th></tr></thead>
      <tbody>${rows.map((a, i) => `<tr tabindex="0" data-i="${apps.indexOf(a)}">
        <td class="c-name"><div class="cn">${ico('grid')}<span style="min-width:0"><span class="nm">${esc(a.name)}</span> ${a.kind ? `<span class="badge">${{ dev: 'Développement', music: 'Musique', creation: 'Création', system: 'Système' }[a.kind]}</span>` : ''}${S.watch.has(a.reg_key) ? '<span class="badge accent"><span class="spinner tiny"></span>Désinstallation en cours</span>' : ''}<span class="sub">${esc([a.publisher, a.version].filter(Boolean).join(' · ') || '—')}</span></span></div></td>
        <td class="muted-cell" ${a.last_used ? '' : 'title="Windows n\'a enregistré aucune ouverture (les programmes lancés en ligne de commande n\'y figurent pas)"'}>${a.last_used ? fmtDate(a.last_used) : 'Aucune enregistrée'}</td>
        <td class="muted-cell">${esc(a.install_date || '—')}</td>
        <td class="num last">${a.size ? `${a.size_measured ? '' : '<span class="faint" title="Taille estimée par l\'installateur">≈ </span>'}${fmtSize(a.size)}` : '—'}
          <div class="actions">${a.location ? `<button class="btn sm ghost" data-act="loc" title="Ouvrir le dossier">${ico('ext', 14)}</button>` : ''}<button class="btn sm ghost" data-act="force" title="À utiliser si la désinstallation normale échoue">${ico('bolt', 14)} Forcer</button><button class="btn sm" data-act="uninstall">${ico('trash', 14)} Désinstaller</button></div></td></tr>`).join('')}</tbody></table></div>`
      : '<div class="table-wrap"><div class="empty"><b>Aucune application trouvée</b>Essaie un autre mot.</div></div>';
    $$('tbody tr', t).forEach((tr) => {
      const a = apps[Number(tr.dataset.i)];
      bindRow(tr, () => { if (a.location) invoke('reveal', { path: a.location }); });
      tr.addEventListener('click', async (e) => {
        const b = e.target.closest('[data-act]');
        if (!b) return;
        if (b.dataset.act === 'loc') invoke('reveal', { path: a.location });
        if (b.dataset.act === 'uninstall') uninstallApp(a, draw);
        if (b.dataset.act === 'force') forceRemove(a);
      });
    });
  };
  S.appsRedraw = draw;
  $('#app-q').addEventListener('input', (e) => { S.appFilter = e.target.value; draw(); });
  $('#app-age').addEventListener('change', (e) => { S.appAge = Number(e.target.value); draw(); });
  $('#app-dev').addEventListener('change', (e) => { S.appDev = e.target.checked; draw(); });
  draw();
}

/* Une application est « protégée » si sa nature est protégée dans les réglages (le système l'est toujours). */
function kindProtected(k) {
  const s = S.settings;
  return k === 'system' || (k === 'dev' && s.protect_dev) || (k === 'music' && s.protect_music) || (k === 'creation' && s.protect_design);
}

/* Désinstallation : on lance le désinstalleur officiel, puis on SURVEILLE l'entrée du registre
   pour savoir quand c'est vraiment terminé (et ne jamais annoncer un succès qu'on n'a pas constaté). */
async function uninstallApp(a, redraw) {
  if (!a.uninstall) { toast('Aucun désinstalleur connu pour cette application : utilise « Forcer ».'); return; }
  try {
    await invoke('uninstall_app', { command: a.uninstall });
  } catch (err) { toast(String(err)); return; }
  toast(`Le désinstalleur de « ${a.name} » s'ouvre. Windows peut te demander ta validation.`);
  if (!a.reg_key || S.watch.has(a.reg_key)) return;
  const w = { name: a.name, t0: Date.now(), timer: null };
  S.watch.set(a.reg_key, w);
  redraw();
  w.timer = setInterval(async () => {
    const still = await invoke('app_installed', { regKey: a.reg_key }).catch(() => true);
    if (!still) {
      clearInterval(w.timer); S.watch.delete(a.reg_key);
      toast(`« ${a.name} » est désinstallée.`);
      refreshApps();
    } else if (Date.now() - w.t0 > 20 * 60 * 1000) {
      clearInterval(w.timer); S.watch.delete(a.reg_key);
      if (S.view === 'apps' && S.appsRedraw) S.appsRedraw();
    }
  }, 3000);
}

async function refreshApps() {
  try { await invoke('reload_apps'); } catch (_) { /* liste inchangée */ }
  if (S.view === 'apps') render();
}

/* Suppression forcée : pour les programmes dont le désinstalleur échoue (service ou pilote bloqué, désinstalleur absent…). */
async function forceRemove(a) {
  if (!a.location) { toast('Dossier d\'installation inconnu : impossible de forcer cette suppression.'); return; }
  modal(`<div class="modal-head"><h1 style="font-size:20px">Analyse de « ${esc(a.name)} »…</h1></div>
    <div class="modal-body"><div class="verif"><span class="spinner"></span><div><b>Recherche des programmes en cours et des services liés</b>
    <div class="muted">Cela prend quelques secondes</div></div></div><div class="pbar ind"><i></i></div></div>`);
  let plan;
  try { plan = await invoke('force_plan', { location: a.location }); } catch (e) { closeModal(); toast(String(e)); return; }
  if (plan.verdict.niveau === 'Interdit') {
    modal(`<div class="modal-head"><h1 style="font-size:20px">Suppression forcée impossible</h1></div>
      <div class="modal-body"><div class="callout danger">${esc(plan.verdict.message)}</div></div>
      <div class="modal-foot"><button class="btn primary" id="m-close2">Fermer</button></div>`);
    $('#m-close2').addEventListener('click', closeModal);
    return;
  }
  const needAdmin = plan.services.length > 0 && !isElevated();
  modal(`
    <div class="modal-head"><h1 style="font-size:20px">Forcer la suppression de « ${esc(a.name)} » ?</h1>
      <p class="lead">À utiliser quand le désinstalleur échoue : les fichiers sont retirés sans passer par lui. Irréversible.</p></div>
    <div class="modal-body">
      <div class="kv"><div><span>Dossier</span><b>${fmtSize(plan.size)}</b></div><div><span>Fichiers</span><b>${fmtInt(plan.files)}</b></div><div><span>Services</span><b>${plan.services.length}</b></div></div>
      <ul class="todo">
        <li>${ico('check', 14)}<span>Arrêter les programmes en cours${plan.processes.length ? ` : <b>${esc(plan.processes.join(', '))}</b>` : ' (aucun détecté)'}</span></li>
        ${plan.services.length ? `<li>${ico('check', 14)}<span>Arrêter et supprimer ${plan.services.length === 1 ? 'le service' : 'les services'} <b>${esc(plan.services.join(', '))}</b></span></li>` : ''}
        <li>${ico('check', 14)}<span>Supprimer le dossier <span class="mono">${esc(plan.location)}</span></span></li>
        <li>${ico('check', 14)}<span>Retirer l'entrée de la liste des applications Windows</span></li>
      </ul>
      ${needAdmin ? '<div class="callout warn">Ce programme a un service Windows : il faut les droits administrateur pour le retirer proprement. <button class="link-btn" id="m-adm">Relancer en administrateur</button></div>' : ''}
      ${a.kind ? `<div class="callout warn">Cette application est classée « ${{ dev: 'développement', music: 'musique', creation: 'création', system: 'système' }[a.kind]} » : vérifie que tu n'en as plus besoin.</div>` : ''}
      <div class="callout">Les pilotes et cartes réseau virtuelles éventuellement installés (VPN…) ne sont pas retirés par cette méthode. Tes données dans AppData ne sont pas supprimées : elles te seront seulement listées.</div>
      <label class="opt" style="margin-top:14px"><input type="checkbox" class="chk" id="m-ack"><span>Je comprends que cette suppression est définitive</span></label>
    </div>
    <div class="modal-foot"><button class="btn" id="m-cancel">Annuler</button>
      <button class="btn danger" id="m-ok" disabled>${ico('bolt', 15)} Forcer la suppression</button></div>`);
  $('#m-cancel').addEventListener('click', closeModal);
  const adm = $('#m-adm'); if (adm) adm.addEventListener('click', () => invoke('relaunch_admin').catch((e) => toast(String(e))));
  const ack = $('#m-ack'); ack.addEventListener('change', () => { $('#m-ok').disabled = !ack.checked; });
  $('#m-ok').addEventListener('click', async () => {
    modal(`<div class="modal-head"><h1 style="font-size:20px">Suppression forcée en cours</h1><p class="lead">« ${esc(a.name)} »</p></div>
      <div class="modal-body"><div class="verif"><span class="spinner"></span><div><b id="fz-l">Arrêt des programmes et des services…</b>
      <div class="muted">Puis suppression du dossier (${fmtSize(plan.size)}). Ne ferme pas l'application.</div></div></div>
      <div class="pbar ind"><i></i></div><div class="cur" id="fz-c"></div></div>`);
    S.forceLive = true;
    let rep;
    try { rep = await invoke('force_remove_app', { name: a.name, location: a.location, regKey: a.reg_key || '' }); } catch (e) { S.forceLive = false; closeModal(); toast(String(e)); return; }
    S.forceLive = false;
    const ok = rep.steps.every((s) => s.ok);
    modal(`
      <div class="modal-head"><h1 style="font-size:20px">${ok ? 'Programme retiré' : 'Terminé, avec des points à vérifier'}</h1></div>
      <div class="modal-body">
        <div class="kv"><div><span>Espace libéré</span><b>${fmtSize(rep.freed_bytes)}</b></div></div>
        <ul class="todo done">${rep.steps.map((s) => `<li class="${s.ok ? 'ok' : 'ko'}">${ico(s.ok ? 'check' : 'alert', 14)}<span><b>${esc(s.label)}</b><br><span class="muted">${esc(s.detail)}</span></span></li>`).join('')}</ul>
        ${rep.failures.length ? `<div class="callout warn">${rep.failures.length} ${plural(rep.failures.length, 'élément résiste', 'éléments résistent')} (fichiers utilisés ou protégés). Un redémarrage puis une nouvelle tentative les retirera.</div>` : ''}
        ${rep.leftovers.length ? `<h2 style="margin:22px 0 8px">Restes possibles (non supprimés)</h2>
          ${rep.leftovers.map((l, i) => `<div class="fail"><div class="p">${esc(l.path)}</div><div class="h">${fmtSize(l.size)}</div></div>`).join('')}` : ''}
      </div>
      <div class="modal-foot">${rep.leftovers.length ? '<button class="btn" id="m-left">Ajouter les restes au panier</button>' : ''}<button class="btn primary" id="m-close2">Fermer</button></div>`);
    $('#m-close2').addEventListener('click', () => { closeModal(); refreshApps(); });
    const lb = $('#m-left');
    if (lb) lb.addEventListener('click', () => {
      rep.leftovers.forEach((l) => { if (!inBasket('path', l.path)) addItem({ kind: 'path', path: l.path, name: l.path.split('\\').pop(), size: l.size, warning: '', file: false }); });
      toast('Restes ajoutés au panier'); lb.disabled = true;
    });
    refreshApps();
  });
}

/* —— Protections : ce que l'utilisateur choisit de garder (rien n'est imposé, tout est réglable) —— */
async function patchSettings(patch) {
  S.settings = { ...S.settings, ...patch, configured: true };
  try { await invoke('settings_set', { s: S.settings }); } catch (e) { toast(String(e)); }
  if (S.view === 'protections') render();
}

async function viewProtections(p) {
  const s = S.settings;
  const row = (id, title, desc, on) => `<label class="item sw-row"><span class="txt"><div class="t">${title}</div><div class="d">${desc}</div></span>
    <span class="switch"><input type="checkbox" id="${id}" ${on ? 'checked' : ''} role="switch" aria-label="${esc(title)}"><i></i></span></label>`;
  const pathRows = (arr, act) => arr.length
    ? `<div class="list">${arr.map((x, i) => `<div class="item" style="cursor:default"><span class="ico">${ico('folder', 16)}</span><span class="txt"><div class="t mono-s" title="${esc(x)}">${esc(x)}</div></span><button class="btn sm ghost" data-${act}="${i}">Retirer</button></div>`).join('')}</div>`
    : '<div class="muted" style="padding:6px 2px">Aucun.</div>';
  p.innerHTML = `
    <div class="page-head"><div><h1>Protections</h1>
      <p class="lead">Choisis ce que Libéra doit garder. Rien n'est imposé et tout se change à tout moment. ${s.configured ? '' : '<b>Pas encore confirmé :</b> par prudence, tout est protégé en attendant ton choix.'}</p></div></div>
    <div class="list">
      ${row('pr-dev', 'Projets de code', 'Dépôts Git, package.json, dossiers « Projets »… ces fichiers ne sont pas proposés à la suppression.', s.protect_dev)}
      ${row('pr-music', 'Musique', 'Projets FL Studio et autres logiciels, samples, plugins, fichiers audio de production.', s.protect_music)}
      ${row('pr-design', 'Création graphique', 'Fichiers PSD, AI, affiches, logos et dossiers de création (les packs téléchargés restent proposables).', s.protect_design)}
    </div>
    <div class="section"><h2>Dossiers à ne jamais toucher</h2>
      <div style="margin:12px 0">${pathRows(s.custom_protected, 'rmp')}</div>
      <button class="btn" id="pr-add">${ico('plus', 14)} Ajouter un dossier…</button></div>
    <div class="section"><h2>Autorisations accordées</h2>
      <p class="muted" style="margin:6px 0 12px">Éléments que tu as autorisés dans le chat malgré les protections. Retire une ligne pour les protéger de nouveau.</p>
      ${pathRows(s.allowed_paths, 'rma')}</div>
    <div class="callout" style="margin-top:36px">${ico('shield', 14)} Les dossiers vitaux de Windows (système, programmes, profil) restent toujours protégés, quoi que tu choisisses : les supprimer rendrait l'ordinateur inutilisable.</div>`;
  const bind = (id, key) => $('#' + id).addEventListener('change', (e) => patchSettings({ [key]: e.target.checked }));
  bind('pr-dev', 'protect_dev'); bind('pr-music', 'protect_music'); bind('pr-design', 'protect_design');
  $('#pr-add').addEventListener('click', async () => {
    try {
      const d = await T.dialog.open({ directory: true, multiple: false, title: 'Dossier à ne jamais toucher' });
      if (d) patchSettings({ custom_protected: [...s.custom_protected, d] });
    } catch (e) { toast(String(e)); }
  });
  $$('[data-rmp]', p).forEach((b) => b.addEventListener('click', () => patchSettings({ custom_protected: s.custom_protected.filter((_, i) => i !== Number(b.dataset.rmp)) })));
  $$('[data-rma]', p).forEach((b) => b.addEventListener('click', () => patchSettings({ allowed_paths: s.allowed_paths.filter((_, i) => i !== Number(b.dataset.rma)) })));
}

/* —— Fichiers inutilisés —— */
const LEVELS = { tres_probable: ['Très probable', 'ok'], probable: ['Probable', 'accent'], a_verifier: ['À vérifier', 'warn'] };

async function viewInutiles(p) {
  const st = S.stale;
  p.innerHTML = `
    <div class="page-head"><div><h1>Fichiers inutilisés</h1>
      <p class="lead">Repère ce qui traîne sans servir : installateurs déjà installés, archives déjà extraites, captures, vieux téléchargements. Chaque proposition est expliquée ; ton code, tes projets, ta musique et tes créations graphiques sont protégés.</p></div></div>
    <div class="list ctrl" style="padding:20px 24px;margin-bottom:28px">
      <div class="toolbar" style="margin-bottom:14px">
        <label class="muted">Sans activité depuis <select class="select" id="st-months">${[[6, '6 mois'], [12, '1 an'], [24, '2 ans'], [36, '3 ans']].map(([v, l]) => `<option value="${v}" ${st.months === v ? 'selected' : ''}>${l}</option>`).join('')}</select></label>
        <span class="spacer"></span>
        <button class="btn primary" id="st-go" ${st.busy ? 'disabled' : ''}>${ico('search', 15)} Rechercher</button>
      </div>
      <div style="display:flex;flex-direction:column;gap:10px">
        <label class="opt" style="align-items:center"><input type="checkbox" class="chk" id="st-dev" ${S.settings.protect_dev ? 'checked' : ''}><span>Protéger les projets de code (dépôts, dossiers Projets, outils)</span></label>
        <label class="opt" style="align-items:center"><input type="checkbox" class="chk" id="st-music" ${S.settings.protect_music ? 'checked' : ''}><span>Protéger la musique (projets FL Studio et consorts, samples, wav)</span></label>
        <label class="opt" style="align-items:center"><input type="checkbox" class="chk" id="st-design" ${S.settings.protect_design ? 'checked' : ''}><span>Protéger la création graphique (PSD, AI, affiches, logos ; les packs téléchargés restent proposables)</span></label>
      </div>
      <p class="muted" style="margin:12px 0 0;font-size:12.5px">Ces choix sont les mêmes que dans l'onglet Protections et s'appliquent à toute l'application.</p>
    </div>
    <div id="st-zone"></div>`;
  $('#st-months').addEventListener('change', (e) => { st.months = Number(e.target.value); });
  $('#st-dev').addEventListener('change', (e) => patchSettings({ protect_dev: e.target.checked }));
  $('#st-music').addEventListener('change', (e) => patchSettings({ protect_music: e.target.checked }));
  $('#st-design').addEventListener('change', (e) => patchSettings({ protect_design: e.target.checked }));
  $('#st-go').addEventListener('click', runStale);
  const z = $('#st-zone');
  if (st.busy) renderProgress(z, 'inutiles');
  else if (st.report) drawStale(z);
  else z.innerHTML = '<div class="empty"><b>Pas encore de recherche</b>Choisis la durée d\'inactivité puis lance la recherche : elle prend quelques secondes.</div>';
}

async function runStale() {
  const st = S.stale;
  st.busy = true;
  S.progress = { files: 0, dirs: 0, bytes: 0, expected: 0, current: '', phase: 'Recherche des fichiers inutilisés' };
  render();
  try {
    st.report = await invoke('find_stale', { opts: { months: st.months, protect_dev: S.settings.protect_dev, protect_music: S.settings.protect_music, protect_design: S.settings.protect_design, min_kb: 256 } });
  } catch (e) { toast(String(e)); }
  st.busy = false;
  if (S.view === 'inutiles') render();
}

function drawStale(z) {
  const st = S.stale, r = st.report;
  const items = r.items.filter((i) => (st.level === 'all' || i.level === st.level) && (st.cls === 'all' || i.class === st.cls));
  const sum = (a) => a.reduce((t, i) => t + i.size, 0);
  const sure = r.items.filter((i) => i.level === 'tres_probable');
  const classes = [...new Map(r.items.map((i) => [i.class, i.class_label])).entries()];
  const prot = r.protected.reduce((t, x) => t + x.bytes, 0);
  const protN = r.protected.reduce((t, x) => t + x.files, 0);
  z.innerHTML = `
    <div class="verdict" style="margin:0 0 22px">${r.items.length
      ? `<b>${fmtSize(sum(r.items))}</b> à examiner dans ${fmtInt(r.items.length)} fichiers, dont <b>${fmtSize(sum(sure))}</b> très probables. ${fmtInt(r.scanned_files)} fichiers ont été lus en ${fmtDur(r.elapsed_ms)}.`
      : `Rien à proposer avec ce seuil : ${fmtInt(r.scanned_files)} fichiers lus. Essaie une durée plus courte.`}</div>
    ${r.items.length ? `
    <div class="toolbar"><div class="seg">${[['all', 'Tout'], ['tres_probable', 'Très probable'], ['probable', 'Probable'], ['a_verifier', 'À vérifier']].map(([v, l]) => `<button data-lv="${v}" aria-pressed="${st.level === v}">${l}</button>`).join('')}</div>
      <select class="select" id="st-cls"><option value="all">Tous les types</option>${classes.map(([id, l]) => `<option value="${id}" ${st.cls === id ? 'selected' : ''}>${esc(l)}</option>`).join('')}</select>
      <span class="spacer"></span>
      ${sure.length ? `<button class="btn" id="st-sure">${ico('plus', 14)} Ajouter les très probables (${fmtSize(sum(sure))})</button>` : ''}</div>
    <div id="st-tbl"></div>` : ''}
    <details class="prot"><summary>${ico('shield', 14)} ${fmtSize(prot)} protégés dans ${fmtInt(protN)} fichiers : jamais proposés</summary>
      <div class="prot-list">${r.protected.map((x) => `<div><span>${esc(x.reason)}</span><span class="muted tnum">${fmtInt(x.files)} fichiers · ${fmtSize(x.bytes)}</span></div>`).join('')}</div></details>`;
  $$('[data-lv]', z).forEach((b) => b.addEventListener('click', () => { st.level = b.dataset.lv; drawStale(z); }));
  const cs = $('#st-cls'); if (cs) cs.addEventListener('change', () => { st.cls = cs.value; drawStale(z); });
  const sb = $('#st-sure');
  if (sb) sb.addEventListener('click', () => {
    sure.forEach((i) => { if (!inBasket('path', i.path)) addItem({ kind: 'path', path: i.path, name: i.name, size: i.size, warning: '', file: true }); });
    toast(`${sure.length} fichiers ajoutés au panier`);
    drawStale(z);
  });
  const t = $('#st-tbl');
  if (!t) return;
  t.innerHTML = items.length ? `<div class="table-wrap"><table>
    <colgroup><col style="width:54%"><col style="width:16%"><col style="width:14%"><col style="width:16%"></colgroup>
    <thead><tr><th>Fichier et raisons</th><th>Dernière activité</th><th class="num">Taille</th><th>Confiance</th></tr></thead>
    <tbody>${items.slice(0, 400).map((i) => `<tr tabindex="0" data-path="${esc(i.path)}" data-name="${esc(i.name)}" data-size="${i.size}" data-reveal="1" data-basket-key="${keyOf('path', i.path)}">
      <td class="c-name"><div class="cn">${ico('file')}<span style="min-width:0"><span class="nm" title="${esc(i.path)}">${esc(i.name)}</span><span class="sub" title="${esc(i.reasons.join(' · '))}">${esc(i.reasons.slice(0, 3).join(' · '))}</span></span></div></td>
      <td class="muted-cell">${fmtDate(i.last_activity)}</td>
      <td class="num">${fmtSize(i.size)}</td>
      <td class="last"><span class="badge ${LEVELS[i.level][1]}" title="Score ${i.score}/100">${LEVELS[i.level][0]}</span>${actionsHtml({ path: i.path })}</td></tr>`).join('')}</tbody></table></div>
    ${items.length > 400 ? `<p class="muted" style="margin-top:12px">${fmtInt(items.length - 400)} autres propositions : affine avec les filtres.</p>` : ''}`
    : '<div class="table-wrap"><div class="empty"><b>Aucun résultat pour ce filtre</b></div></div>';
  bindRowActions(t);
}

/* —— Doublons —— */
async function viewDoublons(p) {
  p.innerHTML = skeleton();
  const groups = await invoke('get_dupes');
  const wasted = groups.reduce((a, g) => a + g.wasted, 0);
  p.innerHTML = `
    <div class="page-head"><div><h1>Doublons</h1><p class="lead">${groups.length ? `${fmtInt(groups.length)} groupes de fichiers identiques (contenu vérifié), ${fmtSize(wasted)} récupérables en gardant un exemplaire de chaque.` : 'Aucun doublon de plus de 1 Mo trouvé hors dossiers système.'}</p></div></div>
    ${groups.length ? `<div class="list" id="dups">${groups.map((g, gi) => `
      <div class="dup" data-g="${gi}"><div class="dup-head" tabindex="0" aria-expanded="false"><span class="chev">${ico('chev')}</span>
        <span class="txt"><div class="t">${g.files.length} copies de ${esc(g.files[0].path.split('\\').pop())}</div><div class="d">${fmtSize(g.size)} chacune</div></span>
        <span class="sz item" style="margin-left:auto;padding:0;border:0;cursor:inherit">${fmtSize(g.wasted)} récupérables</span></div>
        <div class="dup-files" hidden></div></div>`).join('')}</div>` : ''}`;
  $$('.dup', p).forEach((d) => {
    const g = groups[Number(d.dataset.g)];
    const head = $('.dup-head', d), box = $('.dup-files', d);
    const toggle = () => {
      const open = head.getAttribute('aria-expanded') === 'true';
      head.setAttribute('aria-expanded', String(!open));
      box.hidden = open;
      if (!open && !box.innerHTML) {
        box.innerHTML = g.files.map((f, i) => `<div class="dup-file" data-path="${esc(f.path)}" data-name="${esc(f.path.split('\\').pop())}" data-size="${f.size}" data-basket-key="${keyOf('path', f.path)}">
          <span class="p" title="${esc(f.path)}">\u200E${esc(f.path)}</span><span class="muted-cell tnum">${fmtDate(f.mtime)}</span>
          ${i === 0 ? '<span class="badge ok">Le plus ancien</span>' : ''}
          ${i === 0 ? '' : `<button class="btn sm act-add" data-act="add">${ico('plus', 14)} Ajouter</button>`}</div>`).join('') +
          `<div class="dup-file"><button class="btn sm" data-all="1">${ico('plus', 14)} Ajouter toutes les copies sauf la plus ancienne</button></div>`;
        bindRowActions(box);
        $('[data-all]', box).addEventListener('click', () => {
          g.files.slice(1).forEach((f) => { if (!inBasket('path', f.path)) addItem({ kind: 'path', path: f.path, name: f.path.split('\\').pop(), size: f.size, warning: '' }); });
          toast(`${g.files.length - 1} copies ajoutées au panier`);
        });
      }
    };
    head.addEventListener('click', toggle);
    head.addEventListener('keydown', (e) => { if (e.key === 'Enter') toggle(); });
  });
}

/* ───────── panier : tiroir ───────── */
function openDrawer() { const d = $('#drawer'); d.hidden = false; renderDrawer(); }
function closeDrawer() { $('#drawer').hidden = true; }
function renderDrawer() {
  const d = $('#drawer');
  if (d.hidden) return;
  const items = [...S.basket.values()];
  const cats = items.some((i) => i.kind === 'cat' && i.admin) && !isElevated();
  d.innerHTML = `
    <div class="drawer-panel" role="dialog" aria-label="Panier de suppression" aria-modal="true">
      <div class="drawer-head"><h1 style="font-size:19px">Panier</h1><button class="btn ghost sm" id="d-close" aria-label="Fermer">${ico('x', 16)}</button></div>
      <div class="drawer-body">
        ${items.length ? items.slice(0, 200).map((i) => `
          <div class="b-item"><span class="faint">${ico(i.kind === 'cat' ? 'spark' : (i.file ? 'file' : 'folder'))}</span>
            <span style="min-width:0;flex:1"><div class="nm" title="${esc(i.kind === 'cat' ? i.name : i.path)}">${esc(i.name)}</div>${i.kind === 'path' ? `<div class="sub" title="${esc(i.path)}">\u200E${esc(i.path)}</div>` : '<div class="sub" style="direction:ltr">Catégorie de nettoyage</div>'}</span>
            <span class="sz">${fmtSize(i.size)}</span><button class="btn ghost sm rm" data-rm="${esc(i.key)}" aria-label="Retirer">${ico('x', 14)}</button></div>`).join('') + (items.length > 200 ? `<p class="muted" style="padding:12px 0">… et ${fmtInt(items.length - 200)} autres éléments, tous inclus dans la suppression.</p>` : '')
          : '<div class="empty"><b>Le panier est vide</b>Ajoute des dossiers, fichiers ou catégories de nettoyage pour les supprimer ensemble.</div>'}
      </div>
      ${items.length ? `<div class="drawer-foot">
        ${cats ? `<div class="callout warn" style="margin:0">Certaines catégories exigent les droits administrateur ; sans eux elles seront ignorées.</div>` : ''}
        <label class="opt"><input type="checkbox" class="chk" id="o-fix" ${S.opts.fix_permissions ? 'checked' : ''}><span>Corriger les droits si l'accès est refusé<small>Utilise les outils Windows takeown et icacls sur ce qui résiste.</small></span></label>
        <label class="opt"><input type="checkbox" class="chk" id="o-reboot" ${S.opts.schedule_on_reboot ? 'checked' : ''}><span>Planifier au redémarrage ce qui reste verrouillé<small>Nécessite les droits administrateur.</small></span></label>
        <label class="opt"><input type="checkbox" class="chk" id="o-recycle" ${S.opts.recycle ? 'checked' : ''}><span>Envoyer à la Corbeille au lieu de supprimer<small>Récupérable, mais l'espace n'est libéré qu'une fois la Corbeille vidée.</small></span></label>
        <div class="total-line"><span>Total</span><b>${fmtSize(basketSize())}</b></div>
        <div style="display:flex;gap:10px"><button class="btn" id="d-clear">Vider le panier</button><button class="btn danger" id="d-go" style="flex:1">${ico('trash', 15)} Supprimer…</button></div>
      </div>` : ''}
    </div>`;
  $('#d-close').addEventListener('click', closeDrawer);
  d.onclick = (e) => { if (e.target === d) closeDrawer(); };
  $$('[data-rm]', d).forEach((b) => b.addEventListener('click', () => removeItem(b.dataset.rm)));
  const c = $('#d-clear'); if (c) c.addEventListener('click', () => { S.basket.clear(); refreshBasketUI(); renderDrawer(); render(); });
  const g = $('#d-go'); if (g) g.addEventListener('click', confirmDelete);
  [['o-fix', 'fix_permissions'], ['o-reboot', 'schedule_on_reboot'], ['o-recycle', 'recycle']].forEach(([id, k]) => { const e = $('#' + id); if (e) e.addEventListener('change', () => (S.opts[k] = e.checked)); });
}

/* ───────── confirmation, suppression, rapport ───────── */
function modal(html) {
  const m = $('#modal');
  m.hidden = false;
  m.innerHTML = `<div class="modal" role="dialog" aria-modal="true">${html}</div>`;
  return m;
}
function closeModal() { $('#modal').hidden = true; $('#modal').innerHTML = ''; }

async function confirmDelete() {
  if ($('#m-ok') || $('#m-stop')) return; // une confirmation ou une suppression est déjà affichée
  const items = [...S.basket.values()];
  const paths = items.filter((i) => i.kind === 'path');
  const cats = items.filter((i) => i.kind === 'cat');
  modal(`<div class="modal-head"><h1 style="font-size:20px">Vérification…</h1></div>
    <div class="modal-body"><div class="verif"><span class="spinner"></span><div><b>Contrôle des protections</b>
    <div class="muted">Projets de code, musique, création graphique et fichiers système</div></div></div><div class="pbar ind"><i></i></div></div>`);
  let prev = [];
  try {
    // Les tailles sont déjà connues : l'aperçu ne remesure rien et reste instantané.
    if (paths.length) prev = await invoke('preview_delete', { items: paths.map((p) => ({ path: p.path, size: p.size, files: p.files ?? (p.file ? 1 : null) })) });
  } catch (e) { closeModal(); toast(String(e)); return; }
  const refused = prev.filter((p) => p.verdict.niveau === 'Interdit');
  const warned = prev.filter((p) => p.verdict.niveau === 'Attention');
  const okPaths = prev.filter((p) => p.verdict.niveau !== 'Interdit');
  const total = okPaths.reduce((a, p) => a + p.size, 0) + cats.reduce((a, c) => a + c.size, 0);
  const permanent = !S.opts.recycle;
  modal(`
    <div class="modal-head"><h1 style="font-size:20px">${permanent ? 'Supprimer définitivement ?' : 'Envoyer à la Corbeille ?'}</h1>
      <p class="lead">${permanent ? 'Cette action est irréversible : les éléments ne passent pas par la Corbeille.' : 'Les éléments pourront être restaurés depuis la Corbeille.'}</p></div>
    <div class="modal-body">
      <div class="kv"><div><span>À libérer</span><b>${fmtSize(total)}</b></div><div><span>Éléments</span><b>${fmtInt(okPaths.length + cats.length)}</b></div><div><span>Suppression</span><b>${permanent ? 'Définitive' : 'Corbeille'}</b></div></div>
      ${refused.length ? `<div class="callout danger"><b>${refused.length} ${plural(refused.length, 'élément protégé ne sera pas touché', 'éléments protégés ne seront pas touchés')} :</b><br>${refused.map((r) => `${esc(r.path)}<br><span class="muted">${esc(r.verdict.message)}</span>`).join('<br>')}</div>` : ''}
      ${warned.length ? `<div class="callout warn"><b>${warned.length} ${plural(warned.length, 'élément demande', 'éléments demandent')} ton attention :</b><br>${warned.map((r) => `${esc(r.path)}<br><span>${esc(r.verdict.message)}</span>`).join('<br>')}</div>
        <label class="opt" style="margin-top:14px"><input type="checkbox" class="chk" id="m-ack"><span>Je confirme vouloir supprimer ces éléments malgré l'avertissement</span></label>` : ''}
      ${cats.some((c) => c.admin) && !isElevated() ? '<div class="callout warn">Sans les droits administrateur, les catégories marquées « Administrateur » seront ignorées.</div>' : ''}
      ${cats.some((c) => c.id === 'windows_old') ? '<div class="callout warn">La suppression de Windows.old empêche tout retour à la version précédente de Windows.</div>' : ''}
    </div>
    <div class="modal-foot"><button class="btn" id="m-cancel">Annuler</button>
      <button class="btn ${permanent ? 'danger' : 'primary'}" id="m-ok" ${okPaths.length + cats.length === 0 || warned.length ? 'disabled' : ''}>${ico('trash', 15)} ${permanent ? 'Supprimer définitivement' : 'Envoyer à la Corbeille'}</button></div>`);
  $('#m-cancel').addEventListener('click', closeModal);
  const ack = $('#m-ack');
  if (ack) ack.addEventListener('change', () => { $('#m-ok').disabled = !ack.checked || okPaths.length + cats.length === 0; });
  $('#m-ok').addEventListener('click', () => runDelete(okPaths.map((p) => p.path), cats, total));
}

async function runDelete(paths, cats, expected) {
  const stepsTotal = paths.length + cats.length;
  let before = 0;
  try { before = ((await invoke('disk_now')) || {}).free || 0; } catch (_) { /* mesure indisponible */ }
  S.del = { t0: Date.now(), expected: expected || 0, base: 0, stepBase: 0, stepsTotal };
  modal(`
    <div class="modal-head"><h1 style="font-size:20px">Suppression en cours</h1><p class="lead" id="dl">Préparation…</p></div>
    <div class="modal-body">
      <div class="pg-head"><b class="big tnum" id="dp">0 %</b><span class="muted tnum" id="dv"></span></div>
      <div class="pbar ${expected ? '' : 'ind'}" id="dbar" style="--p:0"><i></i></div>
      <div class="pg-sub"><span id="ds"></span><span id="de"></span></div>
      <div class="stats" style="grid-template-columns:repeat(2,1fr)"><div><span>Fichiers supprimés</span><b id="df">0</b></div><div><span>Espace libéré</span><b id="db">0 o</b></div></div>
      <div class="cur" id="dc"></div>
      <div class="callout" id="dn" hidden></div>
    </div>
    <div class="modal-foot"><button class="btn" id="m-stop">${ico('stop', 14)} <span>Arrêter</span></button></div>`);
  $('#m-stop').addEventListener('click', (e) => { invoke('cancel'); e.currentTarget.disabled = true; $('span', e.currentTarget).textContent = 'Arrêt en cours…'; });

  const agg = { requested: 0, succeeded: 0, freed_bytes: 0, files_deleted: 0, failures: [], scheduled: [], notes: [], elapsed_ms: 0 };
  const merge = (r) => { agg.requested += r.requested; agg.succeeded += r.succeeded; agg.freed_bytes += r.freed_bytes; agg.files_deleted += r.files_deleted; agg.failures.push(...r.failures); agg.scheduled.push(...r.scheduled); agg.notes.push(...r.notes); agg.elapsed_ms += r.elapsed_ms; };
  try {
    if (cats.length) {
      const r = await invoke('clean_categories', { ids: cats.map((c) => c.id), opts: S.opts });
      merge(r);
      S.del.base += r.freed_bytes; S.del.stepBase += cats.length; // la 2e commande repart de zéro côté moteur
    }
    if (paths.length) merge(await invoke('delete_paths', { paths, opts: S.opts }));
  } catch (e) { agg.notes.push(String(e)); }
  S.del = null;

  // le panier ne garde que ce qui n'est pas parti
  S.basket.forEach((it, k) => { if (it.kind === 'cat') S.basket.delete(k); });
  for (const [k, it] of [...S.basket]) {
    if (it.kind === 'path' && agg.failures.every((f) => !f.path.toLowerCase().startsWith(it.path.toLowerCase()))) S.basket.delete(k);
  }
  let after = 0;
  try { const d = await invoke('disk_now'); after = d.free; if (S.summary) S.summary.disk = d; } catch (_) { /* ignoré */ }
  S.summary = (await invoke('get_summary')) || S.summary;
  refreshBasketUI();
  renderDrawer();
  showReport(agg, { measured: before && after ? Math.max(0, after - before) : 0 });
  render();
}

function paintDelete(p) {
  if (S.forceLive) { const fc = $('#fz-c'); if (fc) fc.textContent = '‎' + (p.current || ''); return; }
  const g = S.del;
  if (!g || !$('#dp')) return;
  const bytes = g.base + p.bytes;
  const pct = g.expected > 0 ? Math.min(99, (bytes / g.expected) * 100) : null;
  const bar = $('#dbar');
  bar.classList.toggle('ind', pct === null);
  if (pct !== null) bar.style.setProperty('--p', pct / 100);
  $('#dp').textContent = pct === null ? '…' : `${Math.floor(pct)} %`;
  $('#dv').textContent = g.expected > 0 ? `${fmtSize(bytes)} sur ${fmtSize(g.expected)}` : '';
  const step = g.stepBase + Math.max(1, p.step || 1);
  $('#dl').textContent = `Étape ${Math.min(step, g.stepsTotal)} sur ${g.stepsTotal}${p.label ? ` · ${p.label}` : ''}`;
  const el = (Date.now() - g.t0) / 1000;
  const rate = el > 1 ? bytes / el : 0;
  $('#ds').textContent = rate > 0 ? `${fmtSize(rate)}/s` : '';
  $('#de').textContent = pct !== null && pct >= 3 && el >= 3 ? `≈ ${fmtEta((el * (100 - pct)) / pct)} restantes` : `${Math.floor(el)} s écoulées`;
  $('#df').textContent = fmtInt(p.files);
  $('#db').textContent = fmtSize(bytes);
  const c = $('#dc'); c.textContent = '‎' + (p.current || ''); c.title = p.current || '';
  const n = $('#dn');
  const slow = /composants windows/i.test(p.label || '');
  n.hidden = !slow;
  if (slow) n.textContent = 'Cette étape est réalisée par l\'outil Windows DISM : elle peut durer plusieurs minutes sans afficher de progression.';
}

function groupFailures(fails) {
  const m = new Map();
  for (const f of fails) {
    if (!m.has(f.reason)) m.set(f.reason, { reason: f.reason, hint: f.hint, items: [], lockers: new Map(), inUse: f.code === 32 || f.code === 33 });
    const g = m.get(f.reason);
    g.items.push(f);
    f.locked_by.forEach((l) => { const n = l.replace(/\s*\(PID.*$/, ''); g.lockers.set(n, (g.lockers.get(n) || 0) + 1); });
  }
  return [...m.values()].sort((a, b) => b.items.length - a.items.length);
}

function showReport(r, ctx = {}) {
  const failed = r.failures.length;
  const groups = groupFailures(r.failures);
  const onlyInUse = failed > 0 && groups.every((g) => g.inUse);
  const recycle = S.opts.recycle;
  const freed = ctx.measured > 0 ? ctx.measured : r.freed_bytes;
  const extra = r.notes.filter((n) => /autres éléments/.test(n));
  const other = r.notes.filter((n) => !/autres éléments/.test(n));
  modal(`
    <div class="modal-head"><h1 style="font-size:20px">${failed && !onlyInUse ? 'Terminé, avec quelques résistances' : 'Terminé'}</h1>
      ${onlyInUse ? '<p class="lead">Quelques fichiers étaient utilisés par des programmes ouverts : ils sont restés en place, c\'est normal.</p>' : ''}</div>
    <div class="modal-body">
      <div class="kv"><div><span>${recycle ? 'Envoyé à la Corbeille' : 'Espace libéré'}</span><b>${fmtSize(freed)}</b><small>${ctx.measured > 0 ? 'mesuré sur le disque' : 'calculé par le moteur'}</small></div>
        <div><span>Fichiers supprimés</span><b>${fmtInt(r.files_deleted)}</b></div><div><span>Durée</span><b>${fmtDur(r.elapsed_ms)}</b></div></div>
      ${other.map((n) => `<div class="callout">${esc(n)}</div>`).join('')}
      ${r.scheduled.length ? `<div class="callout ok">${r.scheduled.length} ${plural(r.scheduled.length, 'élément sera supprimé', 'éléments seront supprimés')} au prochain redémarrage.</div>` : ''}
      ${failed ? `<h2 style="margin:24px 0 6px">Ce qui n'a pas pu être supprimé</h2>
        ${groups.map((g) => `<details class="grp"><summary><span class="badge ${g.inUse ? '' : 'warn'}">${fmtInt(g.items.length)}</span><span class="gt">${esc(g.reason)}</span>
            ${g.lockers.size ? `<span class="muted gl">${[...g.lockers].sort((a, b) => b[1] - a[1]).slice(0, 4).map(([n, c]) => `${esc(n)} ×${c}`).join(', ')}</span>` : ''}</summary>
          <div class="grp-body"><p class="muted">${esc(g.hint)}</p>${g.items.slice(0, 12).map((f) => `<div class="p" title="${esc(f.path)}">‎${esc(f.path)}</div>`).join('')}${g.items.length > 12 ? `<div class="muted">… et ${g.items.length - 12} autres</div>` : ''}</div></details>`).join('')}
        ${extra.length ? `<p class="muted" style="margin-top:12px">${extra.map(esc).join(' · ')}</p>` : ''}` : ''}
    </div>
    <div class="modal-foot">
      ${failed && !isElevated() && !onlyInUse ? '<button class="btn" id="m-admin">Relancer en administrateur</button>' : ''}
      ${failed ? '<button class="btn" id="m-retry">Réessayer ce qui résiste</button>' : ''}
      <button class="btn primary" id="m-close">Fermer</button></div>`);
  $('#m-close').addEventListener('click', () => { closeModal(); closeDrawer(); renderSide(); render(); });
  const a = $('#m-admin'); if (a) a.addEventListener('click', () => invoke('relaunch_admin').catch((e) => toast(String(e))));
  const rt = $('#m-retry');
  if (rt) rt.addEventListener('click', () => runDelete([...new Set(r.failures.map((f) => f.path))], [], 0));
}

/* ───────── assistant (agent IA via Claude Code) ───────── */
const AGENT_MODELS = [
  { id: 'haiku', nom: 'Économe · Haiku', desc: 'Très rapide, consomme très peu de ton quota. Suffit pour les tâches simples.' },
  { id: 'sonnet', nom: 'Équilibré · Sonnet', desc: 'Bon compromis vitesse, qualité et consommation.' },
  { id: 'opus', nom: 'Puissant · Opus', desc: 'Raisonnement le plus poussé, mais consomme le plus.' },
];
const STEP_LABELS = {
  etat: 'Lecture de l\'état du disque', lancer_analyse: 'Analyse du disque', gros_dossiers: 'Exploration des gros dossiers',
  gros_fichiers: 'Recherche des gros fichiers', reconstructibles: 'Recherche des dossiers reconstructibles', zones_nettoyables: 'Lecture des zones nettoyables',
  fichiers_inutilises: 'Recherche des fichiers inutilisés', applications: 'Examen des applications', doublons: 'Recherche de doublons',
  verifier: 'Vérification des protections', panier_voir: 'Lecture du panier', panier_ajouter: 'Ajout au panier', panier_retirer: 'Retrait du panier',
  panier_vider: 'Panier vidé', demander_suppression: 'Demande de confirmation', medias: 'Recherche des images et vidéos hors projets',
  plugins_audio: 'Recherche des plugins audio', projets: 'Recherche des projets', expliquer: 'Examen du dossier', protections_voir: 'Lecture des protections',
  protections_definir: 'Enregistrement des protections', demander_autorisation: 'Demande d\'autorisation', desinstaller_applications: 'Préparation des désinstallations',
  archiver: 'Archivage', bureau_analyser: 'Analyse du Bureau', bureau_proposer: 'Plan de rangement du Bureau',
};
const EXAMPLES = [
  'Libère de la place : dis-moi d\'abord ce que tu peux toucher sans risque',
  'Désinstalle les applications que je n\'utilise plus',
  'Range mon Bureau par domaines sans rien casser',
  'Archive mes projets de musique dans un zip, puis propose de supprimer les originaux',
];

/* ── rendu riche du texte de l'assistant : titres, tableaux, listes, citations, réponses rapides, chemins cliquables ── */
const EMOJI_RE = /[\p{Extended_Pictographic}️‍]/gu;
const NUM_CELL = /^[~≈]?\s*[\d\s.,]+\s*(Go|Mo|Ko|To|o|%|fichiers?|éléments?)?$/i;
const isPath = (s) => /^[A-Za-z]:\\/.test(s);

function inlineMd(s) {
  return s
    .replace(/`([^`]+)`/g, (_, c) => (isPath(c.replace(/&amp;/g, '&'))
      ? `<button class="path" data-path="${c}" title="Afficher dans l'Explorateur">${c}</button>` : `<code>${c}</code>`))
    .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
    .replace(/(^|[\s(])\*([^*\s][^*]*?)\*(?=[\s).,;:!?]|$)/g, '$1<i>$2</i>');
}
const mdCells = (l) => l.trim().replace(/^\||\|$/g, '').split('|').map((c) => c.trim());

function mdTable(head, rows, aligns) {
  const num = head.map((_, ci) => rows.length > 0 && rows.every((r) => !r[ci] || NUM_CELL.test(r[ci])) || aligns[ci] === 'r');
  const cell = (tag, c, ci) => `<${tag}${num[ci] ? ' class="num"' : ''}>${inlineMd(esc(c || ''))}</${tag}>`;
  return `<div class="md-table"><table><thead><tr>${head.map((h, i) => cell('th', h, i)).join('')}</tr></thead>
    <tbody>${rows.map((r) => `<tr>${head.map((_, i) => cell('td', r[i], i)).join('')}</tr>`).join('')}</tbody></table></div>`;
}

function richText(src) {
  let text = src.replace(EMOJI_RE, '').replace(/[ \t]+\n/g, '\n');
  let replies = null;
  text = text.replace(/```reponses[ \t]*\n([\s\S]*?)```/g, (_, body) => {
    replies = body.split('\n').map((s) => s.replace(/^[-*•\d.)\s]+/, '').trim()).filter(Boolean).slice(0, 6);
    return '';
  });
  text = text.replace(/```reponses[\s\S]*$/, ''); // bloc encore en cours d'écriture : on le cache
  const blocks = [];
  text = text.replace(/```\w*[ \t]*\n([\s\S]*?)```/g, (_, code) => { blocks.push(`<pre>${esc(code.trimEnd())}</pre>`); return `\u0000${blocks.length - 1}\u0000`; });
  text = text.replace(/```[\s\S]*$/, '');
  const lines = text.split('\n');
  let html = '', list = null, i = 0;
  const close = () => { if (list) { html += `</${list}>`; list = null; } };
  while (i < lines.length) {
    const line = lines[i].trim();
    let m;
    if ((m = /^\u0000(\d+)\u0000$/.exec(line))) { close(); html += blocks[+m[1]]; i++; continue; }
    if (line.startsWith('|') && i + 1 < lines.length && /^\|?\s*:?-{2,}/.test(lines[i + 1].trim())) {
      close();
      const head = mdCells(line);
      const aligns = mdCells(lines[i + 1]).map((c) => (c.endsWith(':') ? 'r' : ''));
      i += 2;
      const rows = [];
      while (i < lines.length && lines[i].trim().startsWith('|')) { rows.push(mdCells(lines[i])); i++; }
      html += mdTable(head, rows, aligns);
      continue;
    }
    if ((m = /^(#{1,4})\s+(.*)$/.exec(line))) { close(); html += `<h4 class="md-h">${inlineMd(esc(m[2]))}</h4>`; i++; continue; }
    if (/^(-{3,}|\*{3,})$/.test(line)) { close(); html += '<hr>'; i++; continue; }
    if ((m = /^[-*•]\s+(.*)$/.exec(line))) { if (list !== 'ul') { close(); html += '<ul>'; list = 'ul'; } html += `<li>${inlineMd(esc(m[1]))}</li>`; i++; continue; }
    if ((m = /^\d+[.)]\s+(.*)$/.exec(line))) { if (list !== 'ol') { close(); html += '<ol>'; list = 'ol'; } html += `<li>${inlineMd(esc(m[1]))}</li>`; i++; continue; }
    if ((m = /^>\s?(.*)$/.exec(line))) { close(); html += `<blockquote>${inlineMd(esc(m[1]))}</blockquote>`; i++; continue; }
    close();
    if (line) html += `<p>${inlineMd(esc(line))}</p>`;
    i++;
  }
  close();
  return { html, replies };
}

/* ── vue Assistant ── */
async function viewAssistant(p) {
  const c = S.chat;
  if (!c.check) { try { c.check = await invoke('agent_check'); } catch (_) { c.check = { found: false }; } }
  p.classList.add('chatpage');
  const model = AGENT_MODELS.find((m) => m.id === c.model) || AGENT_MODELS[1];
  p.innerHTML = `
    <div class="chat-head"><div><h1>Assistant</h1>
      <p class="lead">Dis-lui ce que tu veux faire, en français. Quand un élément est protégé, il te demande l'autorisation ici, d'un bouton. Il ne supprime rien sans ta confirmation finale.</p></div></div>
    ${c.check.found ? '' : `<div class="callout warn" style="margin:0 0 14px"><b>Claude Code est introuvable.</b> Installe-le puis connecte-toi avec ton abonnement : ouvre un terminal, lance <code>claude</code>, puis <code>/login</code>.</div>`}
    <div class="toolbar" style="margin-bottom:6px">
      <label class="muted">Modèle <select class="select" id="ag-model">${AGENT_MODELS.map((m) => `<option value="${m.id}" ${m.id === model.id ? 'selected' : ''}>${m.nom}</option>`).join('')}</select></label>
      <span class="muted" id="ag-desc">${model.desc}</span><span class="spacer"></span>
      <span class="muted tnum" id="ag-tok" title="Tokens traités dans cette conversation (entrée + sortie)"></span>
      <button class="btn ghost sm" id="ag-new">Nouvelle conversation</button></div>
    <div class="msgs" id="msgs"></div>
    <div class="composer"><textarea id="ag-in" rows="1" placeholder="Ex. : libère 100 Go sans toucher à mes projets" aria-label="Message pour l'assistant"></textarea>
      <button class="btn primary" id="ag-send"></button></div>`;
  const input = $('#ag-in');
  input.value = c.draft || '';
  const grow = () => { input.style.height = 'auto'; input.style.height = Math.min(160, input.scrollHeight) + 'px'; };
  input.addEventListener('input', () => { c.draft = input.value; grow(); });
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendChat(); } });
  $('#ag-send').addEventListener('click', () => (c.busy ? stopChat() : sendChat()));
  $('#ag-model').addEventListener('change', (e) => { c.model = e.target.value; localStorage.setItem('agent_model', c.model); $('#ag-desc').textContent = (AGENT_MODELS.find((m) => m.id === c.model) || {}).desc; });
  $('#ag-new').addEventListener('click', () => { if (c.busy) stopChat(); c.msgs = []; c.session = null; c.tokens = 0; c.queue = []; $('#msgs').innerHTML = ''; drawChat(); });
  chatDelegates($('#msgs'));
  grow();
  drawChat(true);
  input.focus();
}

/* ── mise à jour incrémentale : on ne reconstruit jamais ce qui n'a pas changé (plus de clignotement) ── */
function drawChat(force) {
  const c = S.chat, box = $('#msgs');
  if (!box) return;
  const send = $('#ag-send');
  if (send && (force || send._busy !== c.busy)) {
    send._busy = c.busy;
    send.innerHTML = c.busy ? `${ico('stop', 14)} <span>Arrêter</span>` : `${ico('send', 15)} <span>Envoyer</span>`;
    send.classList.toggle('primary', !c.busy);
  }
  const tok = $('#ag-tok');
  if (tok) tok.textContent = c.tokens ? `${c.tokens >= 1000 ? (c.tokens / 1000).toFixed(1).replace('.', ',') + ' k' : c.tokens} tokens` : '';
  if (!c.msgs.length) {
    if (!box.querySelector('.chat-empty')) {
      box.innerHTML = `<div class="chat-empty"><b>Que veux-tu faire ?</b><div class="chips">${EXAMPLES.map((e) => `<button class="chip" data-chip="${esc(e)}">${esc(e)}</button>`).join('')}</div></div>`;
    }
    return;
  }
  const empty = box.querySelector('.chat-empty'); if (empty) empty.remove();
  const near = box.scrollHeight - box.scrollTop - box.clientHeight < 90;
  c.msgs.forEach((m, i) => {
    let el = box.children[i];
    if (!el) { el = document.createElement('div'); el.className = `msg ${m.role === 'user' ? 'user' : 'bot'} enter`; el.dataset.mi = i; box.appendChild(el); }
    patchMsg(el, m, i, i === c.msgs.length - 1);
  });
  if (near || c.busy) box.scrollTop = box.scrollHeight;
}

function patchMsg(el, m, mi, isLast) {
  const c = S.chat;
  if (m.role === 'user') {
    if (el._t !== m.text) { el.textContent = m.text; el._t = m.text; }
    el.classList.toggle('auto', !!m.auto);
    return;
  }
  let think = el.querySelector(':scope > .think');
  if (!think) {
    think = document.createElement('div');
    think.className = 'think';
    think.innerHTML = '<span class="dots"><i></i><i></i><i></i></span><span class="ag-el">Réflexion en cours</span>';
    el.appendChild(think);
  }
  m.parts.forEach((p, pi) => {
    let pe = el.querySelector(`:scope > [data-p="${pi}"]`);
    if (!pe) {
      pe = document.createElement('div');
      pe.dataset.p = pi;
      pe.className = p.t === 'text' ? 'part-text' : p.t === 'step' ? 'step' : 'card';
      el.insertBefore(pe, think);
    }
    if (p.t === 'text') {
      const live = c.busy && isLast && pi === m.parts.length - 1;
      const r = richText(p.text);
      const used = p.used;
      const chips = r.replies ? `<div class="chips replies">${r.replies.map((x) => `<button class="chip" data-chip="${esc(x)}" ${c.busy || used ? 'disabled' : ''}>${esc(x)}</button>`).join('')}</div>` : '';
      const k = r.html + chips + (live ? '|live' : '');
      pe.classList.toggle('live', live);
      if (pe._k !== k) { pe.innerHTML = r.html + chips; pe._k = k; }
    } else if (p.t === 'step') {
      const k = p.st + '|' + (p.extra || '');
      if (pe._k !== k) {
        pe.className = `step ${p.st}`;
        pe.innerHTML = `${p.st === 'run' ? '<span class="spinner"></span>' : p.st === 'err' ? ico('alert', 13) : ico('check', 13)}<span>${esc(STEP_LABELS[p.name] || p.name)}</span>${p.extra ? `<span class="muted tnum">${esc(p.extra)}</span>` : ''}`;
        pe._k = k;
      }
    } else {
      const html = renderCard(p, mi, pi);
      if (pe._k !== html) { pe.innerHTML = html; pe._k = html; }
    }
  });
  const hasText = m.parts.some((x) => x.t === 'text');
  think.hidden = !(c.busy && isLast && !hasText);
  let err = el.querySelector(':scope > .callout.danger');
  if (m.error) {
    if (!err) { err = document.createElement('div'); err.className = 'callout danger'; el.appendChild(err); }
    if (err.textContent !== m.error) err.textContent = m.error;
  }
}

/* ── cartes interactives dans le chat ── */
function renderCard(p, mi, pi) {
  const at = `data-mi="${mi}" data-pi="${pi}"`;
  if (p.kind === 'permission') {
    const scope = p.portee === 'chemins'
      ? `<ul class="paths">${p.chemins.slice(0, 6).map((x) => `<li><button class="path" data-path="${esc(x)}">${esc(x)}</button></li>`).join('')}${p.chemins.length > 6 ? `<li class="muted">… et ${p.chemins.length - 6} autres</li>` : ''}</ul>`
      : `<p class="muted">Cela lève la protection « ${{ code: 'Projets de code', musique: 'Musique', creation: 'Création graphique' }[p.portee]} » pour l'ensemble de ton PC. Tu pourras la remettre à tout moment dans l'onglet Protections.</p>`;
    return `<div class="card-box ${p.state}"><div class="card-h">${ico('shield', 15)} Autorisation demandée</div>
      <p>${esc(p.motif || '')}</p>${scope}
      <div class="card-actions">${p.state === 'pending'
        ? `<button class="btn primary sm" data-act="perm-allow" ${at}>Autoriser</button><button class="btn sm" data-act="perm-deny" ${at}>Refuser</button>`
        : `<span class="badge ${p.state === 'allowed' ? 'ok' : ''}">${p.state === 'allowed' ? 'Autorisé' : 'Refusé'}</span>`}</div></div>`;
  }
  if (p.kind === 'organize') {
    const plan = p.plan;
    const groups = {};
    plan.moves.forEach((m, i) => { (groups[m.domain] ||= []).push([m, i]); });
    const n = p.sel.filter(Boolean).length;
    return `<div class="card-box ${p.state}"><div class="card-h">${ico('folder', 15)} Rangement du Bureau</div>
      ${plan.moves.length ? Object.entries(groups).map(([d, arr]) => `<div class="org-g"><div class="org-h"><b>${esc(d)}</b><span class="muted">${arr.length} ${plural(arr.length, 'élément', 'éléments')}</span></div>
        ${arr.map(([m, i]) => `<label class="org-row"><input type="checkbox" class="chk" data-act="org-sel" data-oi="${i}" ${at} ${p.sel[i] ? 'checked' : ''} ${p.state !== 'pending' ? 'disabled' : ''}>
          <span class="nm" title="${esc(m.from)}">${esc(m.name)}</span>${m.risk === 'projet' ? '<span class="badge warn">Projet</span>' : ''}<span class="muted tnum">${fmtSize(m.size)}</span></label>`).join('')}</div>`).join('')
        : '<p class="muted">Rien à déplacer.</p>'}
      ${plan.kept.length ? `<p class="muted">Laissés en place : ${plan.kept.map((k) => esc(k[0])).join(', ')}.</p>` : ''}
      ${plan.fixes.length ? `<p class="muted">${plan.fixes.length} ${plural(plan.fixes.length, 'raccourci du Bureau sera réparé', 'raccourcis du Bureau seront réparés')} automatiquement.</p>` : ''}
      ${plan.warnings.map((w) => `<div class="callout" style="margin:8px 0 0">${esc(w)}</div>`).join('')}
      <div class="card-actions">${p.state === 'pending'
        ? `<button class="btn primary sm" data-act="org-apply" ${at} ${n ? '' : 'disabled'}>Appliquer ${n} ${plural(n, 'déplacement', 'déplacements')}</button>`
        : p.state === 'applying' ? '<span class="muted"><span class="spinner tiny"></span>Déplacement en cours…</span>'
        : `<span class="badge ok">${p.result.moved.length} ${plural(p.result.moved.length, 'élément rangé', 'éléments rangés')}</span>${p.result.fixed_shortcuts ? `<span class="muted">${p.result.fixed_shortcuts} raccourci(s) réparé(s)</span>` : ''}${p.result.failed.length ? `<span class="muted">${p.result.failed.length} échec(s)</span>` : ''}${p.state === 'done' ? `<button class="btn sm" data-act="org-undo" ${at}>Annuler le rangement</button>` : '<span class="badge">Rangement annulé</span>'}`}</div></div>`;
  }
  if (p.kind === 'uninstall') {
    const sel = p.apps.filter((a) => a.sel).length;
    const canQuiet = p.apps.some((a) => a.quiet);
    return `<div class="card-box ${p.state}"><div class="card-h">${ico('trash', 15)} Désinstaller ${p.apps.length} ${plural(p.apps.length, 'application', 'applications')}</div>
      <div class="org-g">${p.apps.map((a, i) => `<label class="org-row"><input type="checkbox" class="chk" data-act="un-sel" data-oi="${i}" ${at} ${a.sel ? 'checked' : ''} ${p.state !== 'pending' ? 'disabled' : ''}>
        <span class="nm">${esc(a.name)}</span><span class="muted tnum">${a.size ? fmtSize(a.size) : ''}</span>
        ${a.st === 'run' ? `<span class="badge accent"><span class="spinner tiny"></span>En cours</span><button class="btn sm ghost" data-act="un-skip" data-oi="${i}" ${at}>Passer</button>` : a.st === 'ok' ? '<span class="badge ok">Désinstallée</span>' : a.st === 'err' ? `<span class="badge danger" title="${esc(a.err || '')}">Échec</span>` : a.st === 'skip' ? '<span class="badge">Ignorée</span>' : ''}</label>`).join('')}</div>
      ${p.state === 'pending' && canQuiet ? `<label class="opt" style="margin-top:10px"><input type="checkbox" class="chk" data-act="un-silent" ${at} ${p.silent ? 'checked' : ''}><span>Désinstallation silencieuse quand c'est possible<small>Sans fenêtres, avec la commande prévue par l'éditeur.</small></span></label>` : ''}
      <div class="card-actions">${p.state === 'pending' ? `<button class="btn danger sm" data-act="un-run" ${at} ${sel ? '' : 'disabled'}>Désinstaller ${sel} ${plural(sel, 'application', 'applications')}</button>`
        : p.state === 'running' ? '<span class="muted">Les désinstalleurs s\'ouvrent un par un. Windows peut demander ta validation.</span>' : '<span class="badge ok">Terminé</span>'}</div>
      ${p.state === 'pending' ? '<p class="muted" style="margin:8px 0 0">Chaque programme est retiré par son désinstalleur officiel. Si l\'un échoue, tu pourras le forcer depuis l\'onglet Applications.</p>' : ''}</div>`;
  }
  return '';
}

function chatDelegates(box) {
  if (box._wired) return;
  box._wired = true;
  box.addEventListener('click', async (e) => {
    const chip = e.target.closest('[data-chip]');
    if (chip && !chip.disabled) {
      // une réponse rapide est consommée : les autres boutons du même bloc se grisent
      const mi = chip.closest('.msg')?.dataset.mi;
      if (mi != null) S.chat.msgs[mi].parts.forEach((x) => { if (x.t === 'text') x.used = true; });
      sendChat(chip.dataset.chip);
      return;
    }
    const path = e.target.closest('.path');
    if (path) { invoke('reveal', { path: path.dataset.path }); return; }
    const b = e.target.closest('[data-act]');
    if (!b) return;
    const part = S.chat.msgs[b.dataset.mi]?.parts[b.dataset.pi];
    if (!part) return;
    const redraw = () => drawChat();
    switch (b.dataset.act) {
      case 'perm-allow': case 'perm-deny': {
        const allow = b.dataset.act === 'perm-allow';
        part.state = allow ? 'allowed' : 'denied'; redraw();
        try { const msg = await invoke('permission_answer', { id: part.id, allow }); autoSend(`${msg} ${allow ? 'Continue.' : ''}`.trim()); }
        catch (err) { toast(String(err)); }
        break;
      }
      case 'org-sel': part.sel[Number(b.dataset.oi)] = b.checked; redraw(); break;
      case 'org-apply': {
        const names = part.plan.moves.filter((_, i) => part.sel[i]).map((m) => m.name);
        part.state = 'applying'; redraw();
        try { part.result = await invoke('organize_apply', { names }); part.state = 'done'; autoSend(`Rangement appliqué : ${part.result.moved.length} éléments déplacés${part.result.failed.length ? `, ${part.result.failed.length} échecs` : ''}.`); }
        catch (err) { part.state = 'pending'; toast(String(err)); }
        redraw(); break;
      }
      case 'org-undo': {
        try { await invoke('organize_undo'); part.state = 'undone'; toast('Rangement annulé : tout est remis à sa place.'); } catch (err) { toast(String(err)); }
        redraw(); break;
      }
      case 'un-sel': part.apps[Number(b.dataset.oi)].sel = b.checked; redraw(); break;
      case 'un-silent': part.silent = b.checked; break;
      case 'un-skip': part.apps[Number(b.dataset.oi)].skip = true; break;
      case 'un-run': runUninstalls(part); break;
      default:
    }
  });
  box.addEventListener('change', (e) => { const b = e.target.closest('input[data-act]'); if (b) b.click && 0; });
}

/* Désinstallations en file : une par une, on attend la disparition de l'entrée du registre avant de passer à la suivante. */
async function runUninstalls(card) {
  card.state = 'running';
  card.apps.forEach((a) => { if (!a.sel) a.st = 'skip'; });
  drawChat();
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  for (const a of card.apps) {
    if (!a.sel) continue;
    a.st = 'run'; drawChat();
    try { await invoke('uninstall_app', { command: card.silent && a.quiet ? a.quiet : a.uninstall }); }
    catch (err) { a.st = 'err'; a.err = String(err); drawChat(); continue; }
    let gone = false;
    if (!a.reg_key) { a.st = 'ok'; drawChat(); continue; }
    const t0 = Date.now();
    while (Date.now() - t0 < 15 * 60 * 1000 && !a.skip) {
      await sleep(2000);
      if (!(await invoke('app_installed', { regKey: a.reg_key }).catch(() => true))) { gone = true; break; }
    }
    a.st = gone ? 'ok' : a.skip ? 'skip' : 'err';
    if (a.st === 'err') a.err = 'Toujours installée après 15 minutes';
    drawChat();
  }
  card.state = 'done';
  await invoke('reload_apps').catch(() => {});
  drawChat();
  const ok = card.apps.filter((a) => a.st === 'ok').length, ko = card.apps.filter((a) => a.st === 'err').length;
  autoSend(`Désinstallations terminées : ${ok} réussie(s), ${ko} échec(s). Continue.`);
  if (S.view === 'apps') render();
}

/* ── envoi, arrêt, fin ── */
function autoSend(text) {
  const c = S.chat;
  if (c.busy) { (c.queue ||= []).push(text); return; }
  sendChat(text, true);
}

async function sendChat(forced, auto) {
  const c = S.chat, input = $('#ag-in');
  const text = (forced ?? ((input && input.value) || '')).trim();
  if (!text || c.busy) return;
  if (!c.check || !c.check.found) { toast('Claude Code est introuvable : voir le message en haut de la page.'); return; }
  c.msgs.push({ role: 'user', text, auto: !!auto });
  c.msgs.push({ role: 'bot', parts: [], t0: Date.now() });
  c.busy = true;
  if (input && forced == null) { c.draft = ''; input.value = ''; input.style.height = 'auto'; }
  clearInterval(c.timer);
  c.timer = setInterval(() => { const el = document.querySelector('.msg.bot:last-child .think .ag-el'); const m = c.msgs[c.msgs.length - 1]; if (el && m) el.textContent = `Réflexion en cours · ${Math.floor((Date.now() - m.t0) / 1000)} s`; }, 1000);
  drawChat();
  try {
    c.session = await invoke('agent_send', { prompt: text, model: c.model, session: c.session, goal: S.goal });
  } catch (e) { endChat(String(e)); }
}

function stopChat() { invoke('agent_stop'); }

function endChat(error) {
  const c = S.chat;
  clearInterval(c.timer);
  c.busy = false;
  const m = c.msgs[c.msgs.length - 1];
  if (m && m.role === 'bot') {
    m.parts.forEach((x) => { if (x.t === 'step' && x.st === 'run') x.st = 'ok'; });
    if (error) m.error = error;
  }
  drawChat();
  const i = $('#ag-in'); if (i) i.focus();
  if (c.queue && c.queue.length) sendChat(c.queue.shift(), true); // réponse automatique en attente (autorisation, fin de désinstallation…)
}

function lastBot() {
  const c = S.chat;
  let m = c.msgs[c.msgs.length - 1];
  if (!m || m.role !== 'bot') { m = { role: 'bot', parts: [], t0: Date.now() }; c.msgs.push(m); }
  return m;
}

function agentEvent(line) {
  const c = S.chat;
  let ev; try { ev = JSON.parse(line); } catch (_) { return; }
  const m = c.msgs[c.msgs.length - 1];
  if (!m || m.role !== 'bot') return;
  if (ev.type === 'system' && ev.session_id) c.session = ev.session_id;
  else if (ev.type === 'stream_event') {
    const d = ev.event && ev.event.delta;
    if (ev.event && ev.event.type === 'content_block_delta' && d && d.type === 'text_delta') {
      const last = m.parts[m.parts.length - 1];
      if (last && last.t === 'text') last.text += d.text; else m.parts.push({ t: 'text', text: d.text });
    }
  } else if (ev.type === 'assistant' && ev.message && Array.isArray(ev.message.content)) {
    for (const b of ev.message.content) {
      if (b.type === 'tool_use' && !m.parts.some((x) => x.id === b.id)) m.parts.push({ t: 'step', id: b.id, name: String(b.name).replace(/^mcp__libera__/, ''), st: 'run' });
    }
  } else if (ev.type === 'user' && ev.message && Array.isArray(ev.message.content)) {
    for (const b of ev.message.content) {
      if (b.type === 'tool_result') { const s = m.parts.find((x) => x.id === b.tool_use_id); if (s) s.st = b.is_error ? 'err' : 'ok'; }
    }
  } else if (ev.type === 'result') {
    const u = ev.usage || {};
    c.tokens += (u.input_tokens || 0) + (u.output_tokens || 0) + (u.cache_creation_input_tokens || 0);
    if (!m.parts.some((x) => x.t === 'text') && ev.result) m.parts.push({ t: 'text', text: ev.result });
    if (ev.is_error && ev.result) m.error = ev.result;
  }
  scheduleDraw();
}

let drawQueued = false;
function scheduleDraw() { if (drawQueued) return; drawQueued = true; requestAnimationFrame(() => { drawQueued = false; drawChat(); }); }

/* L'assistant agit sur le panier : on applique ses ajouts à l'interface. */
function applyAgentBasket(e) {
  if (e.op === 'add') {
    for (const it of e.items) {
      const k = keyOf(it.kind === 'cat' ? 'cat' : 'path', it.kind === 'cat' ? it.id : it.path);
      if (!S.basket.has(k)) S.basket.set(k, { ...it, key: k, file: !!it.file });
    }
    toast(`${e.items.length} ${plural(e.items.length, 'élément ajouté', 'éléments ajoutés')} au panier par l'assistant`);
  } else if (e.op === 'remove') {
    for (const [k, it] of [...S.basket]) if (e.values.some((v) => v.toLowerCase() === (it.path || '').toLowerCase() || v === it.id)) S.basket.delete(k);
  } else if (e.op === 'clear') S.basket.clear();
  refreshBasketUI();
  renderDrawer();
}

/* ───────── événements du moteur ───────── */
function wireEvents() {
  listen('progress', (p) => { S.progress = p; paintProgress(); });
  listen('delete-progress', paintDelete);
  listen('analysis-started', (mode) => {
    if (S.analysing) return;
    S.analysing = true; S.summary = null; S.mode = mode;
    S.progress = { files: 0, dirs: 0, bytes: 0, expected: 0, current: '', phase: 'Préparation' };
    renderSide();
    if (S.view === 'apercu') render();
  });
  listen('analysis-done', async (sum) => {
    S.analysing = false;
    S.summary = sum;
    S.folderId = 0;
    S.status = await invoke('get_status');
    if (sum && sum.cancelled) toast('Analyse annulée : résultats partiels');
    renderSide();
    if (S.view === 'apercu') render();
  });
  listen('analysis-error', (m) => { S.analysing = false; toast(String(m)); render(); });
  listen('agent-event', agentEvent);
  listen('agent-done', (d) => {
    const hint = /login|auth|credential|not logged/i.test(d.stderr || '') ? 'Claude Code n\'est pas connecté : ouvre un terminal, lance `claude`, puis /login.' : '';
    const m = S.chat.msgs[S.chat.msgs.length - 1];
    const empty = m && m.role === 'bot' && !m.parts.some((x) => x.t === 'text' || x.t === 'card');
    endChat(d.code !== 0 && empty ? (hint || (d.stderr || 'L\'assistant s\'est arrêté.').trim().split('\n').slice(0, 3).join(' ')) : '');
  });
  listen('agent-basket', applyAgentBasket);
  listen('agent-request-delete', () => { if (S.basket.size) { openDrawer(); confirmDelete(); } });
  listen('agent-permission', (p) => { lastBot().parts.push({ t: 'card', kind: 'permission', id: p.id, motif: p.motif, portee: p.portee, chemins: p.chemins || [], state: 'pending' }); scheduleDraw(); });
  listen('agent-organize', (plan) => { lastBot().parts.push({ t: 'card', kind: 'organize', plan, sel: plan.moves.map((m) => m.risk !== 'projet'), state: 'pending' }); scheduleDraw(); });
  listen('agent-uninstall', (d) => { lastBot().parts.push({ t: 'card', kind: 'uninstall', apps: d.apps.map((a) => ({ ...a, sel: true })), silent: false, state: 'pending' }); scheduleDraw(); });
  listen('archive-progress', (p) => {
    const m = S.chat.msgs[S.chat.msgs.length - 1];
    const step = m && m.parts.slice().reverse().find((x) => x.t === 'step' && x.name === 'archiver');
    if (!step) return;
    step.extra = p.done ? '' : `${p.phase || ''} · ${fmtSize(p.bytes)} sur ${fmtSize(p.total)}${p.total ? ` (${Math.min(99, Math.floor((p.bytes / p.total) * 100))} %)` : ''}`;
    scheduleDraw();
  });
  listen('settings-changed', async () => { S.settings = await invoke('settings_get'); if (S.view === 'protections') render(); });
}

document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  if (!$('#modal').hidden && !$('#m-ok') && !$('#m-stop')) closeModal();
  else if ($('#modal').hidden && !$('#drawer').hidden) closeDrawer();
});

/* ───────── démarrage ───────── */
(async function init() {
  wireEvents();
  try {
    S.status = await invoke('get_status');
    S.summary = (await invoke('get_summary')) || null;
    S.settings = await invoke('settings_get');
    S.analysing = S.status.busy;
    const c = S.status.drives.find((d) => d.root.toUpperCase().startsWith('C')) || S.status.drives[0];
    if (c) S.drive = c.root;
  } catch (e) { toast(String(e)); }
  renderSide();
  render();
})();
})();
