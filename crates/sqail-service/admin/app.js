// sqail-service admin page. Plain JavaScript, no build step: this file is
// compiled into the service binary. Every call goes to the service's own
// /v1 API with the admin token; nothing is sent anywhere else.
'use strict';

const TOKEN_KEY = 'sqail.admin.token';
const $ = (sel) => document.querySelector(sel);

let token = null;
let status = null; // last /v1/admin/status

// ------------------------------------------------------------- helpers --

/** Build an element. Children are nodes or strings (never parsed as HTML). */
function h(tag, attrs, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === undefined || v === null || v === false) continue;
    if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'class') el.className = v;
    // Through CSSOM: the page's CSP blocks inline style attributes.
    else if (k === 'style') el.style.cssText = v;
    else if (k === 'value') el.value = v;
    else if (k === 'checked') el.checked = !!v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) {
    if (c === undefined || c === null || c === false) continue;
    el.append(c instanceof Node ? c : String(c));
  }
  return el;
}

class ApiError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

async function api(method, path, body) {
  const res = await fetch(path, {
    method,
    headers: {
      Authorization: `Bearer ${token}`,
      ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
    cache: 'no-store',
  });
  if (res.status === 401) {
    signOut('Your token is no longer valid. Sign in again.');
    throw new ApiError(401, 'signed out');
  }
  const text = await res.text();
  const data = text ? JSON.parse(text) : null;
  if (!res.ok) {
    throw new ApiError(res.status, (data && (data.detail || data.title)) || `HTTP ${res.status}`);
  }
  return data;
}

function toast(message, isError) {
  const el = h('div', { class: 'toast' + (isError ? ' err' : '') }, message);
  $('#toasts').append(el);
  setTimeout(() => el.remove(), isError ? 8000 : 3500);
}

/** Run an async action, reporting failures as a toast. */
async function attempt(fn) {
  try {
    return await fn();
  } catch (e) {
    if (!(e instanceof ApiError && e.status === 401)) toast(e.message || String(e), true);
    return undefined;
  }
}

function busy(text) {
  $('#busy-text').textContent = text || '';
  $('#busy').hidden = !text;
}

function copyButton(value) {
  return h('button', {
    class: 'small',
    type: 'button',
    onclick: async () => {
      try {
        await navigator.clipboard.writeText(value);
        toast('Copied');
      } catch {
        toast('Copy failed; select the text instead', true);
      }
    },
  }, 'Copy');
}

function copyable(value) {
  return h('span', { class: 'copy' }, h('code', {}, value), copyButton(value));
}

function when(iso) {
  if (!iso) return 'never';
  const d = new Date(iso);
  return d.toLocaleString();
}

function ago(iso) {
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 90) return 'just now';
  if (s < 5400) return `${Math.round(s / 60)} minutes ago`;
  if (s < 129600) return `${Math.round(s / 3600)} hours ago`;
  return `${Math.round(s / 86400)} days ago`;
}

/** Show a modal. `build(close)` returns [title, body nodes, footer buttons]. */
function modal(build) {
  const dlg = $('#dialog');
  dlg.replaceChildren();
  const close = () => dlg.close();
  const [title, body, buttons] = build(close);
  const form = h('form', { method: 'dialog', onsubmit: (e) => e.preventDefault() },
    h('header', {}, h('h2', {}, title)),
    h('div', { class: 'body stack' }, body),
    h('footer', {}, buttons));
  dlg.append(form);
  dlg.showModal();
  return close;
}

function confirmModal(title, message, action, danger = true) {
  return new Promise((resolve) => {
    modal((close) => [title, [h('div', {}, message)], [
      h('button', { type: 'button', onclick: () => { close(); resolve(false); } }, 'Cancel'),
      h('button', { type: 'button', class: danger ? 'primary danger' : 'primary', onclick: () => { close(); resolve(true); } }, action),
    ]]);
  });
}

function field(label, input, hint) {
  return h('label', {}, label, input, hint ? h('div', { class: 'hint' }, hint) : null);
}

function check(label, input, hint) {
  return h('div', {}, h('label', { class: 'check' }, input, label), hint ? h('div', { class: 'hint' }, hint) : null);
}

function select(options, value) {
  return h('select', {}, options.map(([v, text]) => {
    const o = h('option', { value: v }, text);
    o.selected = v === value;
    return o;
  }));
}

function num(value, min) {
  return h('input', { type: 'number', min: String(min ?? 0), step: '1', value: String(value) });
}

function intOf(input, name) {
  const v = Number(input.value);
  if (!Number.isInteger(v) || v < 0) throw new Error(`${name} must be a whole number`);
  return v;
}

// -------------------------------------------------------------- sign in --

function signOut(message) {
  token = null;
  try { sessionStorage.removeItem(TOKEN_KEY); } catch { /* storage may be off */ }
  $('#app').hidden = true;
  $('#login').hidden = false;
  const err = $('#login-error');
  err.textContent = message || '';
  err.hidden = !message;
  $('#login-token').focus();
}

async function signIn(candidate) {
  token = candidate;
  let info;
  try {
    info = await api('GET', '/v1/info');
  } catch (e) {
    if (!(e instanceof ApiError && e.status === 401)) signOut(e.message);
    return;
  }
  if (info.scope !== 'admin') {
    signOut(`That is a "${info.scope}" token; the admin page needs an admin token.`);
    return;
  }
  try { sessionStorage.setItem(TOKEN_KEY, token); } catch { /* per-tab only */ }
  $('#version').textContent = `v${info.version}`;
  $('#login').hidden = true;
  $('#app').hidden = false;
  if (!location.hash.startsWith('#/')) location.hash = '#/overview';
  else render();
}

$('#login-form').addEventListener('submit', (e) => {
  e.preventDefault();
  signIn($('#login-token').value.trim());
});
$('#logout').addEventListener('click', () => signOut());

// --------------------------------------------------------------- router --

const pages = { overview, connections, tokens, settings, audit };

async function render() {
  if (!token) return;
  const name = (location.hash.match(/^#\/(\w+)/) || [])[1] || 'overview';
  const page = pages[name] || overview;
  for (const a of document.querySelectorAll('.nav nav a')) {
    a.classList.toggle('active', a.dataset.page === name);
  }
  const root = $('#page');
  root.replaceChildren(h('p', { class: 'muted' }, 'Loading…'));
  try {
    const content = await page();
    root.replaceChildren(...[content].flat());
  } catch (e) {
    if (e instanceof ApiError && e.status === 401) return;
    root.replaceChildren(h('div', { class: 'banner err' }, h('b', {}, 'Could not load this page'), e.message));
  }
}

window.addEventListener('hashchange', render);

function head(title, subtitle, ...actions) {
  return h('div', { class: 'page-head' },
    h('div', {}, h('h1', {}, title), subtitle ? h('p', { class: 'muted' }, subtitle) : null),
    h('div', { class: 'spacer' }),
    actions);
}

// ------------------------------------------------------------- overview --

async function overview() {
  status = await api('GET', '/v1/admin/status');
  const s = status;
  const banners = [];
  if (s.last_restart_error) {
    banners.push(h('div', { class: 'banner err' }, h('b', {}, 'The last settings change did not work'), s.last_restart_error));
  }
  if (s.loopback_only) {
    banners.push(h('div', { class: 'banner warn' }, h('b', {}, 'Only this computer can connect'),
      'To let other computers use this service, choose ', h('a', { href: '#/settings' }, 'Settings → Network'), '.'));
  }
  if (s.active_tokens <= 1) {
    banners.push(h('div', { class: 'banner info' }, h('b', {}, 'Next: give people access'),
      'Create a token for each person or machine under ', h('a', { href: '#/tokens' }, 'Tokens'),
      ', and add the databases they may use under ', h('a', { href: '#/connections' }, 'Connections'), '.'));
  } else if (s.connections === 0) {
    banners.push(h('div', { class: 'banner info' }, h('b', {}, 'Next: add a database'),
      h('a', { href: '#/connections' }, 'Add a connection'), ' so sqail users have something to query.'));
  }

  return [
    head('Overview', `Running since ${when(s.started_at)} · ${s.listen}`,
      h('button', { onclick: backup }, 'Back up'),
      h('button', { onclick: restartService }, 'Restart')),
    banners,
    h('div', { class: 'cards' },
      stat(s.connections, 'connections', '#/connections'),
      stat(s.active_tokens, 'active tokens', '#/tokens'),
      stat(s.sessions, 'open sessions'),
      stat(s.running_queries, 'running queries')),
    h('div', { class: 'card' },
      h('h2', {}, 'How sqail users connect'),
      h('p', { class: 'muted' }, 'In sqail: Service → Connect to a service…, then enter:'),
      h('dl', { class: 'kv' },
        h('dt', {}, 'URL'), h('dd', {}, s.urls.map((u) => h('div', {}, copyable(u)))),
        h('dt', {}, 'Token'), h('dd', {}, 'one per person, from ', h('a', { href: '#/tokens' }, 'Tokens')),
        h('dt', {}, 'Fingerprint'), h('dd', {}, copyable(s.fingerprint),
          h('div', { class: 'hint' }, s.self_signed
            ? 'Self-signed certificate: users compare this value when sqail asks them to trust it.'
            : 'Your own certificate: users can choose "Use system trust" if it is issued by a CA they trust.')))),
    h('div', { class: 'card' },
      h('h2', {}, 'Service'),
      h('dl', { class: 'kv' },
        h('dt', {}, 'Version'), h('dd', {}, s.version),
        h('dt', {}, 'Listening on'), h('dd', {}, h('code', {}, s.listen)),
        h('dt', {}, 'Certificate'), h('dd', {}, s.self_signed ? 'self-signed (generated)' : 'your own', s.mutual_tls ? ', client certificates required' : ''),
        h('dt', {}, 'SQLite'), h('dd', {}, s.sqlite_enabled ? 'enabled' : 'off (no folders allowed)'),
        h('dt', {}, 'Data folder'), h('dd', {}, h('code', {}, s.data_dir)),
        h('dt', {}, 'Settings file'), h('dd', {}, h('code', {}, s.config_file)))),
  ];
}

function stat(value, label, href) {
  const body = [h('b', {}, value), h('span', {}, label)];
  return href ? h('a', { class: 'stat', href, style: 'text-decoration:none;color:inherit' }, body) : h('div', { class: 'stat' }, body);
}

async function backup() {
  const res = await attempt(() => api('POST', '/v1/admin/backup'));
  if (!res) return;
  modal((close) => ['Backup written', [
    h('p', {}, 'A consistent copy of service.db is on the service host at:'),
    h('p', {}, copyable(res.path)),
    h('div', { class: 'banner warn' }, h('b', {}, 'master.key is not included'),
      'Stored database passwords can only be decrypted with master.key from the data folder. Keep a copy of it somewhere other than the backups.'),
  ], [h('button', { type: 'button', class: 'primary', onclick: close }, 'Done')]]);
}

async function restartService() {
  const ok = await confirmModal('Restart the service?',
    'The settings file is read again. Open sessions are closed and running queries are cancelled.', 'Restart', false);
  if (!ok) return;
  const res = await attempt(() => api('POST', '/v1/admin/restart'));
  if (res) await waitForRestart(res.port);
}

/** After a restart request: wait until the service answers again, then reload. */
async function waitForRestart(port) {
  busy('Restarting the service…');
  const samePort = String(port) === (location.port || '443');
  if (!samePort) {
    // Another origin: the token has to travel in the fragment.
    await sleep(2500);
    location.href = `${location.protocol}//${location.hostname}:${port}/admin/#token=${encodeURIComponent(token)}`;
    return;
  }
  await sleep(800);
  for (let i = 0; i < 40; i++) {
    try {
      const s = await api('GET', '/v1/admin/status');
      busy();
      if (s.last_restart_error) toast('The service kept its previous settings: ' + s.last_restart_error, true);
      else toast('Service restarted');
      render();
      return;
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) { busy(); return; }
      await sleep(500);
    }
  }
  busy();
  modal((close) => ['The service did not answer', [
    h('p', {}, 'It may be using a new certificate that your browser does not trust yet. Reload the page and accept the certificate.'),
  ], [h('button', { type: 'button', class: 'primary', onclick: () => { close(); location.reload(); } }, 'Reload')]]);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------- connections --

const ENGINES = [['mssql', 'SQL Server'], ['postgres', 'PostgreSQL'], ['sqlite', 'SQLite']];

function target(c) {
  const p = c.params;
  if (p.engine === 'sqlite') return p.path;
  if (p.engine === 'postgres') return `${p.host}:${p.port}/${p.database}`;
  const host = p.instance ? `${p.host}\\${p.instance}` : `${p.host}:${p.port}`;
  return p.database ? `${host}/${p.database}` : host;
}

async function connections() {
  const list = await api('GET', '/v1/connections');
  const rows = list.map((c) => h('tr', {},
    h('td', {}, c.color ? h('span', { class: 'swatch', style: `background:${c.color}` }) : null, h('b', {}, c.name),
      c.folder ? h('div', { class: 'hint' }, c.folder) : null),
    h('td', {}, (ENGINES.find(([k]) => k === c.engine) || [0, c.engine])[1]),
    h('td', { class: 'mono' }, target(c)),
    h('td', {}, c.environment || ''),
    h('td', {}, c.read_only ? h('span', { class: 'pill' }, 'read-only') : ''),
    h('td', { class: 'actions' },
      h('button', { class: 'small', onclick: (e) => testSaved(c, e.target) }, 'Test'),
      h('button', { class: 'small', onclick: () => editConnection(c) }, 'Edit'),
      h('button', { class: 'small danger', onclick: () => deleteConnection(c) }, 'Delete'))));
  return [
    head('Connections', 'Databases that sqail users can query through this service. Passwords stay here, encrypted.',
      h('button', { class: 'primary', onclick: () => editConnection(null) }, 'New connection')),
    h('div', { class: 'card table-wrap' }, list.length
      ? h('table', {}, h('thead', {}, h('tr', {}, ['Name', 'Engine', 'Target', 'Environment', '', ''].map((t) => h('th', {}, t)))), h('tbody', {}, rows))
      : h('div', { class: 'empty' }, 'No connections yet.')),
  ];
}

async function testSaved(c, button) {
  button.disabled = true;
  button.textContent = 'Testing…';
  const res = await attempt(() => api('POST', `/v1/connections/${c.id}/test`));
  button.disabled = false;
  button.textContent = 'Test';
  if (res) showTest(res);
}

function showTest(res) {
  if (res.ok) toast(`Connected in ${res.latency_ms} ms: ${res.server_version || 'ok'}`);
  else toast(`Connection failed: ${res.error}`, true);
}

async function deleteConnection(c) {
  const ok = await confirmModal(`Delete "${c.name}"?`,
    'sqail users lose access to this connection. Its stored password is deleted.', 'Delete');
  if (!ok) return;
  if (await attempt(() => api('DELETE', `/v1/connections/${c.id}`)) !== undefined) {
    toast('Connection deleted');
    render();
  }
}

function editConnection(existing) {
  const p = existing ? existing.params : { engine: 'mssql' };
  const f = {
    name: h('input', { value: existing ? existing.name : '', required: true, maxlength: '200' }),
    engine: select(ENGINES, p.engine),
    host: h('input', { value: p.host || '', placeholder: 'sqlserver01.corp.local' }),
    port: num(p.port || (p.engine === 'postgres' ? 5432 : 1433), 1),
    instance: h('input', { value: p.instance || '', placeholder: 'SQLEXPRESS' }),
    database: h('input', { value: p.database || '' }),
    auth: select([['sql', 'SQL Server login'], ['integrated', 'Windows (as the service account)']],
      p.auth && p.auth.method === 'integrated' ? 'integrated' : 'sql'),
    user: h('input', { value: p.user || (p.auth && p.auth.user) || '', autocomplete: 'off' }),
    password: h('input', { type: 'password', autocomplete: 'new-password',
      placeholder: existing && existing.has_password ? '(unchanged)' : '' }),
    clearPassword: h('input', { type: 'checkbox' }),
    encrypt: select([['required', 'Required'], ['on', 'If the server supports it'], ['off', 'Login only']], p.encrypt || 'required'),
    trust: h('input', { type: 'checkbox', checked: p.trust_server_certificate }),
    sslMode: select([['disable', 'disable'], ['prefer', 'prefer'], ['require', 'require'], ['verify-full', 'verify-full']], p.ssl_mode || 'prefer'),
    path: h('input', { value: p.path || '', placeholder: 'C:\\data\\app.db or /srv/data/app.db' }),
    create: h('input', { type: 'checkbox', checked: p.create }),
    readOnly: h('input', { type: 'checkbox', checked: existing && existing.read_only }),
    environment: h('input', { value: (existing && existing.environment) || '', placeholder: 'dev, test, prod…' }),
    color: h('input', { value: (existing && existing.color) || '', placeholder: '#c0392b' }),
    folder: h('input', { value: (existing && existing.folder) || '' }),
  };

  const groups = {
    network: field('Host', f.host, 'Also accepts HOST\\INSTANCE and HOST,PORT, as in SSMS.'),
    port: field('Port', f.port),
    instance: field('Instance', f.instance, 'Named instance; needs SQL Browser (UDP 1434).'),
    database: field('Database', f.database),
    auth: field('Authentication', f.auth),
    user: field('User', f.user),
    password: field('Password', f.password),
    clear: check('Remove the stored password', f.clearPassword),
    encrypt: field('Encryption', f.encrypt),
    trust: check('Trust the server certificate without checking it (test servers only)', f.trust),
    ssl: field('SSL mode', f.sslMode),
    path: field('File', f.path, status && !status.sqlite_enabled
      ? 'SQLite is off: allow a folder under Settings → SQLite first.'
      : 'Absolute path on the service host, inside an allowed SQLite folder.'),
    create: check('Create the file if it does not exist', f.create),
  };

  const engineBox = h('div', { class: 'grid two' });
  function layout() {
    const e = f.engine.value;
    const show = e === 'sqlite' ? ['path', 'create']
      : e === 'postgres' ? ['network', 'port', 'database', 'user', 'password', 'ssl']
      : ['network', 'port', 'instance', 'database', 'auth', 'user', 'password', 'encrypt', 'trust'];
    if (existing && existing.has_password && e !== 'sqlite') show.push('clear');
    if (e === 'mssql' && f.auth.value === 'integrated') {
      show.splice(show.indexOf('user'), 1);
      show.splice(show.indexOf('password'), 1);
    }
    engineBox.replaceChildren(...show.map((k) => groups[k]));
    if (!existing) f.port.value = e === 'postgres' ? '5432' : '1433';
  }
  f.engine.addEventListener('change', layout);
  f.auth.addEventListener('change', layout);
  layout();
  if (existing) f.engine.disabled = true;

  const result = h('div');

  function input() {
    const e = f.engine.value;
    let params;
    if (e === 'sqlite') {
      params = { engine: 'sqlite', path: f.path.value.trim(), create: f.create.checked };
    } else if (e === 'postgres') {
      params = { engine: 'postgres', host: f.host.value.trim(), port: intOf(f.port, 'Port'),
        database: f.database.value.trim(), user: f.user.value.trim(), ssl_mode: f.sslMode.value };
    } else {
      params = { engine: 'mssql', host: f.host.value.trim(), port: intOf(f.port, 'Port'),
        auth: f.auth.value === 'integrated' ? { method: 'integrated' } : { method: 'sql', user: f.user.value.trim() },
        encrypt: f.encrypt.value, trust_server_certificate: f.trust.checked };
      if (f.instance.value.trim()) params.instance = f.instance.value.trim();
      if (f.database.value.trim()) params.database = f.database.value.trim();
    }
    const body = { name: f.name.value.trim(), params, read_only: f.readOnly.checked };
    for (const k of ['environment', 'color', 'folder']) {
      if (f[k].value.trim()) body[k] = f[k].value.trim();
    }
    const usesPassword = e !== 'sqlite' && !(e === 'mssql' && f.auth.value === 'integrated');
    if (!usesPassword) {
      if (existing && existing.has_password) body.password = '';
    } else if (f.password.value) {
      body.password = f.password.value;
    } else if (f.clearPassword.checked) {
      body.password = '';
    } else if (!existing) {
      body.password = '';
    }
    return body;
  }

  async function test(button) {
    let body;
    try { body = input(); } catch (e) { toast(e.message, true); return; }
    button.disabled = true;
    result.replaceChildren(h('p', { class: 'muted' }, 'Connecting…'));
    // Unchanged password on a saved profile: the service has it, we don't.
    const saved = existing && existing.has_password && body.password === undefined;
    const res = await attempt(() => saved
      ? api('POST', `/v1/connections/${existing.id}/test`)
      : api('POST', '/v1/connections/test', body));
    button.disabled = false;
    if (!res) { result.replaceChildren(); return; }
    result.replaceChildren(h('div', { class: 'banner ' + (res.ok ? 'info' : 'err') },
      h('b', {}, res.ok ? `Connected in ${res.latency_ms} ms` : 'Could not connect'),
      res.ok ? res.server_version : res.error,
      saved ? h('div', { class: 'hint' }, 'Tested the saved settings (the stored password is used). Save first to test changes.') : null));
  }

  async function save(close) {
    let body;
    try { body = input(); } catch (e) { toast(e.message, true); return; }
    if (!body.name) { toast('Give the connection a name', true); return; }
    const res = await attempt(() => existing
      ? api('PUT', `/v1/connections/${existing.id}`, body)
      : api('POST', '/v1/connections', body));
    if (res) {
      close();
      toast(existing ? 'Connection saved' : 'Connection added');
      render();
    }
  }

  modal((close) => [existing ? `Edit ${existing.name}` : 'New connection', [
    h('div', { class: 'grid two' }, field('Name', f.name, 'Shown in sqail.'), field('Database engine', f.engine)),
    engineBox,
    h('h3', {}, 'In sqail'),
    h('div', { class: 'grid two' },
      field('Environment', f.environment),
      field('Colour', f.color, 'e.g. #c0392b to mark production.'),
      field('Folder', f.folder)),
    check('Read-only', f.readOnly, '"read" tokens may only query read-only connections. Use a database login without write rights for real protection.'),
    result,
  ], [
    h('button', { type: 'button', onclick: (e) => test(e.target) }, 'Test'),
    h('div', { class: 'spacer' }),
    h('button', { type: 'button', onclick: close }, 'Cancel'),
    h('button', { type: 'button', class: 'primary', onclick: () => save(close) }, 'Save'),
  ]]);
  f.name.focus();
}

// --------------------------------------------------------------- tokens --

const SCOPES = [
  ['query', 'query: run SQL on every connection'],
  ['read', 'read: browse, and query read-only connections'],
  ['admin', 'admin: everything, including this page'],
];

async function tokens() {
  const list = await api('GET', '/v1/tokens');
  list.sort((a, b) => (a.revoked - b.revoked) || b.created_at.localeCompare(a.created_at));
  const rows = list.map((t) => h('tr', {},
    h('td', {}, h('b', {}, t.name)),
    h('td', {}, h('span', { class: 'pill' + (t.scope === 'admin' ? ' admin' : '') }, t.scope)),
    h('td', {}, when(t.created_at)),
    h('td', {}, t.last_used_at ? ago(t.last_used_at) : h('span', { class: 'muted' }, 'never')),
    h('td', {}, t.revoked ? h('span', { class: 'pill bad' }, 'revoked') : h('span', { class: 'pill ok' }, 'active')),
    h('td', { class: 'actions' }, t.revoked ? '' : h('button', { class: 'small danger', onclick: () => revokeToken(t) }, 'Revoke'))));
  return [
    head('Tokens', 'Give each person or machine its own token, so the audit log tells them apart and you can revoke one alone.',
      h('button', { class: 'primary', onclick: createToken }, 'New token')),
    h('div', { class: 'card table-wrap' }, list.length
      ? h('table', {}, h('thead', {}, h('tr', {}, ['Name', 'Scope', 'Created', 'Last used', 'Status', ''].map((t) => h('th', {}, t)))), h('tbody', {}, rows))
      : h('div', { class: 'empty' }, 'No tokens.')),
  ];
}

function createToken() {
  const name = h('input', { required: true, maxlength: '100', placeholder: 'alice, reporting-server…' });
  const scope = select(SCOPES, 'query');
  modal((close) => ['New token', [
    field('Name', name, 'Who or what uses it.'),
    field('Scope', scope),
  ], [
    h('button', { type: 'button', onclick: close }, 'Cancel'),
    h('button', { type: 'button', class: 'primary', onclick: async () => {
      if (!name.value.trim()) { toast('Give the token a name', true); return; }
      const res = await attempt(() => api('POST', '/v1/tokens', { name: name.value.trim(), scope: scope.value }));
      if (!res) return;
      close();
      showNewToken(res);
      render();
    } }, 'Create'),
  ]]);
  name.focus();
}

async function showNewToken(created) {
  if (!status) status = await attempt(() => api('GET', '/v1/admin/status'));
  const url = status ? status.urls[0] : location.origin;
  modal((close) => [`Token for ${created.info.name}`, [
    h('div', { class: 'banner warn' }, h('b', {}, 'Shown only now'), 'Copy it and hand it over through a secure channel. It cannot be displayed again.'),
    h('div', { class: 'secret mono' }, created.token),
    h('div', {}, copyButton(created.token)),
    h('h3', {}, 'In sqail: Service → Connect to a service…'),
    h('dl', { class: 'kv' },
      h('dt', {}, 'URL'), h('dd', {}, copyable(url)),
      h('dt', {}, 'Token'), h('dd', {}, '(above)'),
      h('dt', {}, 'Fingerprint'), h('dd', {}, status ? copyable(status.fingerprint) : '')),
  ], [h('button', { type: 'button', class: 'primary', onclick: close }, 'Done')]]);
}

async function revokeToken(t) {
  const ok = await confirmModal(`Revoke "${t.name}"?`,
    t.scope === 'admin'
      ? 'Anything using this admin token stops working immediately. If it is the token you signed in with, you are signed out.'
      : 'Anything using this token stops working immediately. This cannot be undone.',
    'Revoke');
  if (!ok) return;
  if (await attempt(() => api('DELETE', `/v1/tokens/${t.id}`)) !== undefined) {
    toast('Token revoked');
    render();
  }
}

// ------------------------------------------------------------- settings --

async function settings() {
  const [doc, st] = await Promise.all([api('GET', '/v1/admin/settings'), api('GET', '/v1/admin/status')]);
  status = st;
  const s = doc.settings;
  const env = new Set(doc.env_overrides);
  const locked = (key, el) => {
    if (env.has(key)) el.disabled = true;
    return el;
  };
  const envNote = (key) => env.has(key) ? h('div', { class: 'hint' }, `Set by an environment variable on the service host; change it there.`) : null;

  // --- network
  const [bindHost, bindPort] = splitBind(s.bind);
  const mode = bindHost === '127.0.0.1' || bindHost === '::1' ? 'local'
    : bindHost === '0.0.0.0' || bindHost === '::' ? 'network' : 'custom';
  const radio = (value, label, hint) => {
    const r = locked('bind', h('input', { type: 'radio', name: 'bind-mode', value, checked: mode === value }));
    return h('div', {}, h('label', { class: 'check' }, r, label), hint ? h('div', { class: 'hint', style: 'margin-left:22px' }, hint) : null);
  };
  const customHost = locked('bind', h('input', { value: mode === 'custom' ? bindHost : '', placeholder: '10.0.0.5' }));
  const port = locked('bind', num(bindPort, 1));
  const network = h('fieldset', {}, h('legend', {}, 'Network'),
    h('div', { class: 'radio-list' },
      radio('local', 'This computer only', 'sqail on this machine can connect. The safe default.'),
      radio('network', 'Other computers too', 'Listens on every network interface. Allow the port in the firewall, and consider your own certificate.'),
      radio('custom', 'One specific address of this computer', null)),
    h('div', { class: 'grid', style: 'margin-top:10px' }, field('Address', customHost), field('Port', port)),
    envNote('bind'));

  // --- certificate
  const hasOwn = !!(s.tls.cert && s.tls.key);
  const certMode = (value, label) => h('label', { class: 'check' },
    h('input', { type: 'radio', name: 'cert-mode', value, checked: (value === 'own') === hasOwn }), label);
  const certFile = h('input', { type: 'file', accept: '.pem,.crt,.cer' });
  const keyFile = h('input', { type: 'file', accept: '.pem,.key' });
  const tls12 = h('input', { type: 'checkbox', checked: s.tls.allow_tls12 });
  const clientCa = h('input', { value: s.tls.client_ca || '', placeholder: 'path to a CA certificate (PEM) on the service host' });
  const certificate = h('fieldset', {}, h('legend', {}, 'Certificate'),
    h('div', { class: 'radio-list' },
      certMode('self', 'Self-signed (generated by the service)'),
      h('div', { class: 'hint', style: 'margin-left:22px' }, 'Users confirm its fingerprint the first time they connect.'),
      certMode('own', 'My own certificate')),
    hasOwn ? h('p', { class: 'hint' }, 'Current: ', h('code', {}, s.tls.cert)) : null,
    h('div', { class: 'grid two', style: 'margin-top:8px' },
      field('Certificate (PEM, leaf first)', certFile, hasOwn ? 'Leave empty to keep the current one.' : null),
      field('Private key (PEM)', keyFile)),
    h('div', { style: 'margin-top:10px' }, check('Also allow TLS 1.2 (default: TLS 1.3 only)', tls12)),
    h('div', { style: 'margin-top:10px' }, field('Require client certificates signed by (mutual TLS)', clientCa,
      'Optional. Leave empty to rely on tokens alone.')));

  // --- sqlite
  const sqliteDirs = locked('sqlite.allowed_dirs', h('textarea', { placeholder: 'One folder per line' }));
  sqliteDirs.value = s.sqlite.allowed_dirs.join('\n');
  const sqlite = h('fieldset', {}, h('legend', {}, 'SQLite'),
    field('Folders SQLite connections may open', sqliteDirs,
      'Absolute paths on the service host, one per line. Empty turns SQLite off. The service account needs access to them.'),
    envNote('sqlite.allowed_dirs'));

  // --- limits and sessions
  const L = s.limits;
  const lim = {
    default_max_rows: num(L.default_max_rows, 1), max_rows: num(L.max_rows, 1),
    default_timeout_ms: num(L.default_timeout_ms), max_timeout_ms: num(L.max_timeout_ms),
    requests_per_second: num(L.requests_per_second, 1), burst: num(L.burst, 1),
    pool_size: num(L.pool_size, 1), request_timeout_secs: num(L.request_timeout_secs, 1),
    body_limit_bytes: num(L.body_limit_bytes, 4096),
  };
  const idle = num(s.sessions.idle_timeout_secs, 10);
  const maxSessions = num(s.sessions.max_per_token, 1);
  const limits = h('fieldset', {}, h('legend', {}, 'Limits'),
    h('div', { class: 'grid' },
      field('Rows per result (default)', lim.default_max_rows),
      field('Rows per result (max)', lim.max_rows),
      field('Query timeout, ms (default)', lim.default_timeout_ms, '0 = none'),
      field('Query timeout, ms (max)', lim.max_timeout_ms),
      field('Requests per second, per token', lim.requests_per_second),
      field('Burst', lim.burst),
      field('Connections per database', lim.pool_size),
      field('Request timeout, s', lim.request_timeout_secs, 'Not for streamed query results.'),
      field('Largest request, bytes', lim.body_limit_bytes),
      field('Idle session timeout, s', idle, 'Idle transactions roll back.'),
      field('Sessions per token', maxSessions)));

  // --- audit and extras
  const logSql = h('input', { type: 'checkbox', checked: s.audit.log_sql });
  const maxSql = num(s.audit.max_sql_len);
  const docsUi = locked('docs_ui', h('input', { type: 'checkbox', checked: s.docs_ui }));
  const adminUi = locked('admin_ui', h('input', { type: 'checkbox', checked: s.admin_ui }));
  const other = h('fieldset', {}, h('legend', {}, 'Audit log and pages'),
    check('Record query text in the audit log (never row values)', logSql),
    h('div', { class: 'grid', style: 'margin:8px 0 12px' }, field('Longest recorded query, bytes', maxSql)),
    check('Interactive API documentation at /docs', docsUi),
    check('This admin page', adminUi, 'Switching it off also disables /v1/admin. Turn it back on in the settings file.'));

  async function save(button) {
    let file;
    try {
      file = JSON.parse(JSON.stringify(s));
      const m = document.querySelector('input[name=bind-mode]:checked').value;
      const hostPart = m === 'local' ? '127.0.0.1' : m === 'network' ? '0.0.0.0' : customHost.value.trim();
      if (!hostPart) throw new Error('Enter the address to listen on');
      const p = intOf(port, 'Port');
      if (p < 1 || p > 65535) throw new Error('Port must be 1-65535');
      file.bind = hostPart.includes(':') ? `[${hostPart}]:${p}` : `${hostPart}:${p}`;
      for (const [k, el] of Object.entries(lim)) file.limits[k] = intOf(el, k.replaceAll('_', ' '));
      file.sessions.idle_timeout_secs = intOf(idle, 'Idle session timeout');
      file.sessions.max_per_token = intOf(maxSessions, 'Sessions per token');
      file.sqlite.allowed_dirs = sqliteDirs.value.split('\n').map((x) => x.trim()).filter(Boolean);
      file.tls.allow_tls12 = tls12.checked;
      if (clientCa.value.trim()) file.tls.client_ca = clientCa.value.trim(); else delete file.tls.client_ca;
      file.audit.log_sql = logSql.checked;
      file.audit.max_sql_len = intOf(maxSql, 'Longest recorded query');
      file.docs_ui = docsUi.checked;
      file.admin_ui = adminUi.checked;
    } catch (e) {
      toast(e.message, true);
      return;
    }

    const own = document.querySelector('input[name=cert-mode]:checked').value === 'own';
    const warnings = [];
    if (own && (certFile.files.length || keyFile.files.length)) {
      if (!certFile.files.length || !keyFile.files.length) { toast('Choose both the certificate and the private key', true); return; }
    } else if (own && !hasOwn) {
      toast('Choose the certificate and private key files', true);
      return;
    }
    const remote = !['127.0.0.1', 'localhost', '[::1]'].includes(location.hostname);
    if (remote && file.bind.startsWith('127.0.0.1')) warnings.push('You are connected from another computer. After this change only the service host itself can reach this page.');
    if (!file.admin_ui) warnings.push('The admin page will be switched off. To get it back, edit the settings file on the service host.');
    const ok = await confirmModal('Apply and restart?', [
      'The service restarts with these settings. Open sessions are closed and running queries are cancelled. If it cannot start with them, it keeps the current settings.',
      ...warnings.map((w) => h('div', { class: 'banner warn', style: 'margin-top:10px' }, w)),
    ], 'Apply', false);
    if (!ok) return;

    button.disabled = true;
    try {
      if (own && certFile.files.length) {
        const saved = await api('POST', '/v1/admin/certificate', {
          cert_pem: await certFile.files[0].text(),
          key_pem: await keyFile.files[0].text(),
        });
        file.tls.cert = saved.cert;
        file.tls.key = saved.key;
      } else if (!own) {
        delete file.tls.cert;
        delete file.tls.key;
      }
      const res = await api('PUT', '/v1/admin/settings', file);
      await waitForRestart(res.port);
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 401)) toast(e.message, true);
    } finally {
      button.disabled = false;
    }
  }

  const banner = status.last_restart_error
    ? h('div', { class: 'banner err' }, h('b', {}, 'The last change did not work; the service kept its previous settings'), status.last_restart_error)
    : null;
  const saveButton = h('button', { class: 'primary', onclick: (e) => save(e.target) }, 'Apply and restart');
  return [
    head('Settings', h('span', {}, 'Saved in ', h('code', {}, doc.config_file))),
    banner,
    network, certificate, sqlite, limits, other,
    h('div', { class: 'row end' }, saveButton),
  ];
}

function splitBind(bind) {
  const m = bind.match(/^\[(.+)\]:(\d+)$/) || bind.match(/^(.+):(\d+)$/);
  return m ? [m[1], Number(m[2])] : [bind, 7443];
}

// ---------------------------------------------------------------- audit --

async function audit() {
  const body = h('tbody');
  const more = h('button', {}, 'Load more');
  let before = null;
  async function load() {
    more.disabled = true;
    const page = await attempt(() => api('GET', `/v1/audit?limit=200${before ? `&before=${before}` : ''}`));
    more.disabled = false;
    if (!page) return;
    for (const e of page.items) {
      body.append(h('tr', {},
        h('td', { style: 'white-space:nowrap' }, when(e.at)),
        h('td', {}, e.actor),
        h('td', {}, h('code', {}, e.action)),
        h('td', { class: 'mono small', style: 'max-width:420px;overflow-wrap:anywhere' }, e.detail || e.target || ''),
        h('td', {}, e.duration_ms !== null && e.duration_ms !== undefined ? `${e.duration_ms} ms` : ''),
        h('td', {}, e.success ? h('span', { class: 'pill ok' }, 'ok') : h('span', { class: 'pill bad' }, 'failed'))));
    }
    before = page.next_before;
    more.hidden = !before;
  }
  more.addEventListener('click', load);
  await load();
  return [
    head('Audit log', 'Every token, connection, query and settings change, newest first. Row values are never recorded.'),
    h('div', { class: 'card table-wrap' },
      h('table', {}, h('thead', {}, h('tr', {}, ['When', 'Who', 'Action', 'Detail', 'Took', ''].map((t) => h('th', {}, t)))), body)),
    h('div', { class: 'row end' }, more),
  ];
}

// ---------------------------------------------------------------- start --

(function start() {
  // A sign-in link carries the token in the fragment (never sent to the
  // server, never logged). Take it and drop it from the address bar.
  const m = location.hash.match(/^#token=([^&]+)/);
  if (m) {
    history.replaceState(null, '', location.pathname + '#/overview');
    signIn(decodeURIComponent(m[1]));
    return;
  }
  let saved = null;
  try { saved = sessionStorage.getItem(TOKEN_KEY); } catch { /* storage may be off */ }
  if (saved) signIn(saved);
  else signOut();
})();
