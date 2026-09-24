// The Buckets page. It talks to /v2/ and nothing else: a bucket is an OCI index tree, so the
// page walks it the same way the CLI does, and checks every block it reads against the address
// that named it. No framework, no CDN. The chrome — header, nav, theme, account — is the site's
// own, so this file is only ever about buckets.
import { registry, readBucket, readHistory, readObject, writeObject, digestToCid, cidToDigest, visibilityOf, DEFAULT_QUOTA, INDEX_MT, MANIFEST_MT } from './lib/buckets-lib.mjs?v=4'
import { newKey, keyToText, keyFromText, keyCheck, sealName, ENC } from './lib/crypt.mjs?v=4'
import { build, objectManifest } from './lib/octree.mjs?v=4'

const reg = registry('')
// Live follow is served by the bucket service, not by /v2/. Same origin in production; in the
// dev server it is the gateway next door.
const EVENTS_BASE = new URLSearchParams(location.search).get('events') || ''
// Who is making buckets here. The hub's own sign-in (auth.js, the same module the header uses)
// says who is in; the namespace is that person's handle. `?owner=` overrides it for a test, and
// a person who is not signed in is asked in the dialog. The write credential is separate: the
// registry's, asked for once when a write is refused, kept in this tab.
let account = null
const OWNER_PARAM = new URLSearchParams(location.search).get('owner') || ''
const handleOf = who => String(who || '').split('@')[0].toLowerCase().replace(/[^a-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '')
const owner = () => OWNER_PARAM || (account && handleOf(account.email)) || ''
;(async () => {
  try {
    const base = document.documentElement.dataset.base || '/'
    const auth = await import(`${base}auth.js`)
    auth.onChange(user => { account = user })
    await auth.restore().catch(() => {})
  } catch { /* no sign-in on this build: the dialog asks */ }
})()

// A private bucket's key lives in this browser and nowhere else it did not choose.
const keyStore = {
  get (o, n) { try { const t = localStorage.getItem(`uor-bucket-key:${o}/${n}`); return t ? keyFromText(t) : null } catch { return null } },
  set (o, n, key) { try { localStorage.setItem(`uor-bucket-key:${o}/${n}`, keyToText(key)) } catch {} },
  forget (o, n) { try { localStorage.removeItem(`uor-bucket-key:${o}/${n}`) } catch {} }
}
let links = null
async function linksFor (id) {
  if (!links) { try { links = await (await fetch('links.json')).json() } catch { links = {} } }
  return links[id] || []
}
const $ = id => document.getElementById(id)
const el = (tag, cls, text) => { const n = document.createElement(tag); if (cls) n.className = cls; if (text != null) n.textContent = text; return n }

const human = n => {
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  while (n >= 1000 && i < u.length - 1) { n /= 1000; i++ }
  return (i ? n.toFixed(1) : String(n)) + ' ' + u[i]
}
const when = ms => (ms ? new Date(ms).toISOString().slice(0, 16).replace('T', ' ') : '')
const short = cid => (cid ? cid.slice(0, 10) + '…' + cid.slice(-4) : '')

const ICON = {
  folder: '<svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l2 2.5h7A1.5 1.5 0 0 1 19 10v7a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 17z"/></svg>',
  file: '<svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 3.5h7L18 8v12.5H6z"/><path d="M13 3.5V8h5"/></svg>',
  bucket: '<svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M4.5 7.5h15l-1.4 11.2a1.5 1.5 0 0 1-1.5 1.3H7.4a1.5 1.5 0 0 1-1.5-1.3Z"/><path d="M4.5 7.5c0-1.7 3.4-3 7.5-3s7.5 1.3 7.5 3"/></svg>'
}

let token = sessionStorage.getItem('uor-bucket-token') || ''
let state = null   // the bucket currently open
let pending = null // what to run once a credential arrives

// ---------------------------------------------------------------- routing
function route () {
  const hash = location.hash.replace(/^#\/?/, '')
  const parts = hash.split('/').filter(Boolean)
  if (parts.length < 2) return { view: 'list' }
  return { view: 'browse', owner: parts[0], name: parts[1], prefix: parts.slice(2).join('/') }
}
addEventListener('hashchange', render)

// ---------------------------------------------------------------- asking
// One dialog for New bucket, Settings and Delete: three questions, one shape, and never a
// browser prompt — a dialog the page owns can say what it is about to do.
function ask ({ title, why, fields = [], ok = 'Save' }) {
  return new Promise(resolve => {
    const dialog = $('dialog')
    $('dialog-title').textContent = title
    $('dialog-why').textContent = why || ''
    $('dialog-why').hidden = !why
    const body = $('dialog-body')
    body.replaceChildren()
    for (const f of fields) {
      if (f.type === 'checkbox') {
        const label = el('label', 'row')
        const input = el('input')
        input.type = 'checkbox'; input.id = 'f-' + f.name; input.checked = !!f.value
        label.append(input, el('span', null, f.label))
        body.appendChild(label)
      } else if (f.type === 'choice') {
        for (const c of f.options) {
          const label = el('label', 'choice' + (c.disabled ? ' off' : ''))
          const input = el('input')
          input.type = 'radio'; input.name = 'f-' + f.name; input.value = c.value; input.checked = c.value === f.value; input.disabled = !!c.disabled
          const text = el('span', null, c.label)
          if (c.why) text.appendChild(el('small', null, c.why))
          label.append(input, text)
          body.appendChild(label)
        }
      } else if (f.type === 'static') {
        const block = el('div', f.cls || 'muted', f.value)
        body.appendChild(block)
      } else {
        const label = el('label', 'field')
        const input = el('input')
        input.id = 'f-' + f.name
        input.placeholder = f.label
        input.value = f.value || ''
        input.autocomplete = 'off'
        label.appendChild(input)
        body.appendChild(label)
      }
    }
    $('dialog-ok').textContent = ok
    const done = value => { dialog.close(); $('dialog-form').onsubmit = null; resolve(value) }
    $('dialog-form').onsubmit = e => {
      e.preventDefault()
      const out = {}
      for (const f of fields) {
        if (f.type === 'static') continue
        if (f.type === 'choice') { out[f.name] = body.querySelector(`input[name="f-${f.name}"]:checked`)?.value; continue }
        const input = $('f-' + f.name)
        out[f.name] = f.type === 'checkbox' ? input.checked : input.value.trim()
      }
      done(out)
    }
    $('dialog-cancel').onclick = () => done(null)
    dialog.showModal()
    const first = body.querySelector('input')
    if (first) first.focus()
  })
}

const says = text => { $('note').textContent = text }
// A render can be superseded while it is awaiting: changing the hash and re-rendering by hand
// once produced two passes that both appended, and the list showed every bucket twice.
let renderSeq = 0
let nextNote = null
const READING = 'Read straight from /v2/. Every block is checked against its address in this tab before it counts as delivered.'

// ---------------------------------------------------------------- the bucket list
async function renderList () {
  document.title = 'Buckets · Hologram Models Hub'
  $('title').textContent = 'Buckets'
  $('sub').textContent = 'Storage for models, datasets and checkpoints. Every object carries the address of its own bytes.'
  $('crumbs').hidden = true; $('bar').hidden = true; $('drop').hidden = true
  $('readme').hidden = true; $('cred').hidden = true; $('settings').hidden = true
  $('history-button').hidden = true; $('history').hidden = true; $('keybar').hidden = true; $('linked').hidden = true
  closeDetail()
  $('head').innerHTML = '<tr><th>Bucket</th><th>Objects</th><th>Size</th><th class="hide">Created</th><th class="hide">Address of the current state</th></tr>'
  $('rows').replaceChildren()
  state = null

  const mine = ++renderSeq
  const repos = (await reg.catalogue()).filter(r => r.startsWith('buckets/'))
  if (mine !== renderSeq) return
  $('count').hidden = !repos.length
  $('count').textContent = `${repos.length} bucket${repos.length === 1 ? '' : 's'}`
  $('empty').hidden = repos.length > 0
  if (!repos.length) {
    $('empty').textContent = 'No buckets yet. Make one here, or from a shell with `buckets create <owner>/<name>`.'
    says('')
    return
  }

  for (const repo of repos.sort()) {
    const [, owner, name] = repo.split('/')
    const tr = el('tr')
    const a = el('a')
    a.href = `#/${owner}/${name}`
    a.innerHTML = `<span class="name">${ICON.bucket}<span>${owner} / <b>${name}</b></span></span>`
    const td = el('td'); td.appendChild(a)
    tr.append(td, el('td', 'num', '…'), el('td', 'num', ''), el('td', 'num hide', ''), el('td', 'addr hide', ''))
    $('rows').appendChild(tr)
    reg.manifest(repo, 'latest').then(head => {
      // Deleting a bucket removes its tag, not its repository, so the catalogue keeps listing
      // the name. A row with nothing behind it is a ghost; drop it.
      if (!head) {
        tr.remove()
        const left = $('rows').children.length
        $('count').textContent = `${left} bucket${left === 1 ? '' : 's'}`
        $('empty').hidden = left > 0
        return
      }
      const ann = head.json.annotations || {}
      tr.children[1].textContent = ann['foundation.uor.bucket.objects'] ?? '?'
      tr.children[2].textContent = human(Number(ann['foundation.uor.bucket.bytes'] || 0))
      tr.children[3].textContent = (ann['foundation.uor.bucket.created'] || '').slice(0, 10)
      const vis = visibilityOf(ann)
      if (vis !== 'public') tr.children[0].querySelector('.name').appendChild(el('span', 'pill', vis))
      tr.children[4].textContent = short(digestToCid(head.digest, 0x71))
      tr.children[4].title = head.digest
    }).catch(() => {})
  }
  says(nextNote || READING); nextNote = null
}

// ---------------------------------------------------------------- one bucket
async function renderBrowse (r) {
  const repo = `buckets/${r.owner}/${r.name}`
  document.title = `${r.owner}/${r.name} · Buckets`
  $('title').innerHTML = `<span class="owner">${r.owner} /</span> ${r.name}`
  $('crumbs').hidden = false
  $('bar').hidden = false
  $('empty').hidden = true
  $('settings').hidden = false

  $('history-button').hidden = false
  $('history').hidden = true
  try {
    state = await readBucket(reg, repo, { key: keyStore.get(r.owner, r.name) })
  } catch (e) {
    if (!e.wrongKey) throw e
    keyStore.forget(r.owner, r.name)
    state = await readBucket(reg, repo)
    says('The key kept here did not fit this bucket; it was forgotten.')
  }
  if (!state) {
    $('sub').textContent = 'No such bucket.'
    $('rows').replaceChildren()
    $('empty').hidden = false
    $('empty').textContent = 'This bucket does not exist.'
    $('settings').hidden = true
    return
  }
  const ann = state.annotations
  $('count').hidden = false
  $('count').textContent = `${state.entries.size} object${state.entries.size === 1 ? '' : 's'} · ${human(state.bytes)} of ${human(state.quota)}`
  $('sub').textContent = `${state.visibility}${state.encrypted ? ' — names and bytes are sealed; the key is in this browser' : ''} · state ${short(digestToCid(state.head, 0x71))}`
  $('sub').title = state.head
  $('drop').hidden = state.locked
  // Private and no key here: the names are sealed and nothing opens. Ask for the key, inline.
  $('keybar').hidden = !state.locked
  if (state.locked) $('keybar-value').focus()
  linksFor(`${r.owner}/${r.name}`).then(ids => {
    $('linked').hidden = !ids.length
    if (ids.length) {
      $('linked').replaceChildren(el('span', null, 'Linked from '))
      ids.forEach((id, i) => {
        const a = el('a', null, id); a.href = `${document.documentElement.dataset.base || '/'}models/${id}/`
        if (i) $('linked').appendChild(el('span', null, ', '))
        $('linked').appendChild(a)
      })
    }
  })

  crumbs(r)
  paint(r)
  if (nextNote) { says(nextNote); nextNote = null }
  follow(r.owner, r.name)
}

function crumbs (r) {
  const nav = $('crumbs')
  nav.replaceChildren()
  const add = (label, href, last) => {
    if (last) { nav.appendChild(el('b', null, label)); return }
    const a = el('a', null, label); a.href = href; nav.appendChild(a)
    nav.appendChild(el('span', null, '/'))
  }
  add('Buckets', '#/', false)
  const parts = r.prefix ? r.prefix.split('/').filter(Boolean) : []
  add(`${r.owner}/${r.name}`, `#/${r.owner}/${r.name}`, parts.length === 0)
  parts.forEach((p, i) => add(p, `#/${r.owner}/${r.name}/${parts.slice(0, i + 1).join('/')}`, i === parts.length - 1))
}

function paint (r) {
  const prefix = r.prefix ? r.prefix.replace(/\/$/, '') + '/' : ''
  const filter = $('q').value.trim().toLowerCase()
  const folders = new Map()
  const files = []
  for (const [key, v] of state.entries) {
    if (!key.startsWith(prefix)) continue
    const rest = key.slice(prefix.length)
    const cut = rest.indexOf('/')
    if (cut >= 0) {
      const dir = rest.slice(0, cut)
      const f = folders.get(dir) || { objects: 0, bytes: 0 }
      f.objects++; f.bytes += v.size
      folders.set(dir, f)
    } else if (!filter || rest.toLowerCase().includes(filter)) {
      files.push([rest, key, v])
    }
  }

  $('head').innerHTML = '<tr><th>Name</th><th>Size</th><th class="hide">Modified</th><th class="hide">Address</th><th>State</th></tr>'
  if (detail.key && !state.entries.has(detail.key)) closeDetail()
  const body = $('rows')
  body.replaceChildren()

  for (const [dir, f] of [...folders].sort(([a], [b]) => (a < b ? -1 : 1))) {
    if (filter && !dir.toLowerCase().includes(filter)) continue
    const tr = el('tr')
    const a = el('a')
    a.href = `#/${r.owner}/${r.name}/${prefix}${dir}`
    a.innerHTML = `<span class="name">${ICON.folder}<span>${dir}/</span></span>`
    const td = el('td'); td.appendChild(a)
    tr.append(td, el('td', 'num', human(f.bytes)),
      el('td', 'num hide', `${f.objects} object${f.objects === 1 ? '' : 's'}`),
      el('td', 'addr hide', ''), el('td', 'state', ''))
    body.appendChild(tr)
  }

  for (const [label, key, v] of files.sort(([a], [b]) => (a < b ? -1 : 1))) {
    const tr = el('tr')
    tr.dataset.key = key
    const button = el('button')
    button.innerHTML = `<span class="name">${ICON.file}<span>${label}</span></span>`
    button.addEventListener('click', () => openDetail(key, label, tr))
    const td = el('td'); td.appendChild(button)
    const addr = el('td', 'addr hide', short(v.root)); addr.title = v.root
    tr.append(td, el('td', 'num', human(v.size)), el('td', 'num hide', when(v.mtime)), addr, el('td', 'state', state.locked ? 'sealed' : ''))
    body.appendChild(tr)
  }

  $('empty').hidden = files.length + folders.size > 0
  if (!$('empty').hidden) $('empty').textContent = filter ? 'Nothing matches that filter.' : 'This folder is empty.'

  const readme = state.entries.get(prefix + 'README.md')
  $('readme').hidden = !readme
  if (readme) showReadme(readme)

  says(READING)
}

// ---------------------------------------------------------------- README
// A bucket's README is somebody else's text. It is escaped first, and only then does a very
// small subset of Markdown get turned back into elements.
function markdown (text) {
  const esc = s => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  const inline = s => esc(s)
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/\*([^*]+)\*/g, '<em>$1</em>')
    .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" rel="noopener noreferrer nofollow" target="_blank">$1</a>')
  const out = []
  let list = false
  for (const line of text.split(/\r?\n/)) {
    const h = /^(#{1,3})\s+(.*)$/.exec(line)
    const li = /^[-*]\s+(.*)$/.exec(line)
    if (li) { if (!list) { out.push('<ul>'); list = true } out.push(`<li>${inline(li[1])}</li>`); continue }
    if (list) { out.push('</ul>'); list = false }
    if (h) { out.push(`<h${h[1].length}>${inline(h[2])}</h${h[1].length}>`); continue }
    if (line.trim()) out.push(`<p>${inline(line)}</p>`)
  }
  if (list) out.push('</ul>')
  return out.join('')
}

async function showReadme (entry) {
  const box = $('readme')
  box.textContent = 'Reading README.md…'
  try {
    box.innerHTML = markdown(new TextDecoder().decode(await readObject(reg, state.repo, entry.root, () => {}, state.key)))
  } catch (e) {
    box.textContent = `README.md was refused: ${e.message}`
  }
}

$('keybar-save').addEventListener('click', async () => {
  const r = route()
  let key
  try { key = keyFromText($('keybar-value').value) } catch { return says('That is not a bucket key: 43 characters, base64url.') }
  try {
    const probe = await readBucket(reg, state.repo, { key })
    keyStore.set(r.owner, r.name, key)
    $('keybar-value').value = ''
    state = probe
    await renderBrowse(r)
    says('Opened. The key stays in this browser.')
  } catch (e) {
    says(e.wrongKey ? 'That key does not fit this bucket.' : `That did not work: ${e.message}`)
  }
})

// ---------------------------------------------------------------- one object
// What HF shows on a file page, and two things it cannot: the address the bytes answer to,
// and where those bytes live, as a path anyone can fetch and check without this page.
const detail = { key: null, tr: null }
function closeDetail () { detail.key = null; detail.tr = null; $('detail').hidden = true }
$('detail-close').addEventListener('click', closeDetail)

async function openDetail (key, label, tr) {
  const entry = state.entries.get(key)
  detail.key = key; detail.tr = tr
  const box = $('detail')
  box.hidden = false
  $('detail-name').textContent = key
  $('detail-state').textContent = ''
  $('detail-state').className = 'state'
  const digest = cidToDigest(entry.root)
  const facts = [
    ['Address', entry.root, 'mono'],
    ['Digest', digest, 'mono'],
    ['Size', `${human(entry.size)} (${entry.size.toLocaleString()} bytes)`],
    ['Modified', when(entry.mtime) || '—'],
    ['Blocks', '…'],
    ['Lives at', `/v2/${state.repo}/blobs/${digest}`, 'mono'],
    ['Named by', `/v2/${state.repo}/manifests/${entry.manifest}`, 'mono'],
    ...(entry.wire ? [['On the wire', `${entry.wire.slice(0, 24)}… — the name and every block are sealed with this bucket\u2019s key; the registry holds noise`, 'mono']] : [])
  ]
  const dl = $('detail-facts')
  dl.replaceChildren()
  for (const [k, v, cls] of facts) {
    dl.appendChild(el('dt', null, k))
    dl.appendChild(el('dd', cls || null, v))
  }
  box.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  // The block count is the object manifest's layer list: one line per 1 MiB block, plus the
  // DAG root when there is more than one. Read only when the panel is open.
  try {
    const om = await reg.manifest(state.repo, entry.manifest)
    if (detail.key !== key) return
    const layers = (om && om.json.layers) || []
    const blocks = layers.length > 1 ? layers.length - 1 : layers.length
    dl.children[9].textContent = `${blocks} × up to 1 MiB, each addressed by its own sha-256${layers.length > 1 ? ', joined by one root block' : ''}`
  } catch {
    dl.children[9].textContent = 'unknown'
  }
}

$('detail-download').addEventListener('click', () => {
  if (detail.key) download(detail.key, detail.key.split('/').pop(), detail.tr, $('detail-state'))
})
$('detail-check').addEventListener('click', async () => {
  if (!detail.key) return
  const cell = $('detail-state')
  cell.className = 'state'; cell.textContent = 'checking…'
  try {
    await readObject(reg, state.repo, state.entries.get(detail.key).root, ({ blocks }) => { cell.textContent = `checking… ${blocks} block${blocks === 1 ? '' : 's'}` }, state.key)
    cell.className = 'state ok'; cell.textContent = 'verified: every block is what its address names'
    mark(detail.tr, 'ok', 'verified')
  } catch (e) {
    cell.className = 'state bad'; cell.textContent = e.refused ? 'refused: these bytes are not what the address names. Nothing was delivered.' : `failed: ${e.message}`
    mark(detail.tr, 'bad', e.refused ? 'refused' : 'failed')
  }
})
$('detail-copy').addEventListener('click', async () => {
  if (!detail.key) return
  const address = state.entries.get(detail.key).root
  try { await navigator.clipboard.writeText(address); $('detail-state').className = 'state ok'; $('detail-state').textContent = 'address copied' }
  catch { $('detail-state').className = 'state'; $('detail-state').textContent = address }
})
function mark (tr, cls, text) { if (tr) { tr.children[4].className = 'state ' + cls; tr.children[4].textContent = text } }

// ---------------------------------------------------------------- reading
async function download (key, label, tr, also) {
  const cell = tr.children[4]
  const entry = state.entries.get(key)
  const say = (cls, text) => { cell.className = cls; cell.textContent = text; if (also) { also.className = cls; also.textContent = text } }
  say('state', 'reading…')
  try {
    const bytes = await readObject(reg, state.repo, entry.root, ({ done }) => {
      say('state', entry.size ? `${Math.round((done / entry.size) * 100)}%` : 'reading…')
    }, state.key)
    say('state ok', 'verified')
    const url = URL.createObjectURL(new Blob([bytes]))
    const a = document.createElement('a')
    a.href = url; a.download = label
    a.click()
    setTimeout(() => URL.revokeObjectURL(url), 10000)
  } catch (e) {
    say('state bad', e.refused ? 'refused' : 'failed')
    cell.title = e.message
  }
}

$('verify').addEventListener('click', async () => {
  const rows = [...$('rows').children].filter(tr => tr.dataset.key)
  $('verify').disabled = true
  let ok = 0; let refused = 0
  for (const tr of rows) {
    const entry = state.entries.get(tr.dataset.key)
    const cell = tr.children[4]
    cell.className = 'state'; cell.textContent = 'checking…'
    try {
      await readObject(reg, state.repo, entry.root, () => {}, state.key)
      cell.className = 'state ok'; cell.textContent = 'verified'; ok++
    } catch (e) {
      cell.className = 'state bad'; cell.textContent = e.refused ? 'refused' : 'failed'; cell.title = e.message; refused++
    }
  }
  $('verify').disabled = false
  says(refused
    ? `${refused} of ${rows.length} objects refused: their bytes are not what their addresses name. Nothing was delivered.`
    : `${ok} object${ok === 1 ? '' : 's'} checked against ${ok === 1 ? 'its address' : 'their addresses'} in this tab. Every byte matched.`)
})

$('q').addEventListener('input', () => { if (state) paint(route()) })

// ---------------------------------------------------------------- credentials
// Asked for inline, never in a browser dialog: a dialog is not available in every context this
// page runs in, and a password box that belongs to the page is clearer about where the secret
// is going. It stays in this tab and nowhere else.
// Try first, ask second. A registry with no login, or one that already knows this browser,
// should not be interrogated for a password it does not want — the credential bar appears
// only when a write is actually refused.
function askForCredential (why, retry) {
  pending = retry
  $('cred').hidden = false
  $('cred-why').textContent = why
  $('cred-value').focus()
}
$('cred-save').addEventListener('click', () => {
  const given = $('cred-value').value.trim()
  if (!given) return
  token = given.includes(':') && !/^(Bearer|Basic) /.test(given) ? 'Basic ' + btoa(given) : given
  sessionStorage.setItem('uor-bucket-token', token)
  $('cred-value').value = ''
  $('cred').hidden = true
  const queued = pending; pending = null
  if (queued) queued()
})
function forget () {
  token = ''
  sessionStorage.removeItem('uor-bucket-token')
}
function refused (e) { return /40[13]/.test(String(e && e.message)) }

// ---------------------------------------------------------------- making and unmaking
$('new-bucket').addEventListener('click', async () => {
  const answer = await ask({
    title: 'New bucket',
    why: 'A bucket is a place to put objects. Its name cannot change later; its contents can, at any time.',
    fields: [
      { name: 'owner', label: 'owner (your handle)', value: owner() },
      { name: 'name', label: 'bucket name, for example training-data' },
      { name: 'visibility', type: 'choice', value: 'public', options: [
        { value: 'public', label: 'Public', why: 'listed, readable by anyone' },
        { value: 'unlisted', label: 'Unlisted', why: 'off the list; readable by anyone holding an address' },
        { value: 'private', label: 'Private', why: 'names and bytes leave this browser sealed with a key that never does' }
      ] }
    ],
    ok: 'Create'
  })
  if (!answer || !answer.name) return
  const who = handleOf(answer.owner)
  if (!who) return says('A bucket needs an owner: sign in, or type a handle.')
  const name = answer.name.toLowerCase().replace(/[^a-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '')
  if (!name) return says('That name has nothing usable in it.')
  const run = async () => {
    try {
      const repo = `buckets/${who}/${name}`
      if (await reg.manifest(repo, 'latest')) return says(`${who}/${name} already exists.`)
      const extra = {}
      let key = null
      if (answer.visibility === 'private') {
        key = newKey()
        extra['foundation.uor.bucket.encryption'] = ENC
        extra['foundation.uor.bucket.keycheck'] = await keyCheck(key)
      }
      await publishRoot(repo, [], answer.visibility === 'private' ? 'private' : answer.visibility, extra, key)
      if (key) {
        keyStore.set(who, name, key)
        await ask({
          title: 'Keep this key',
          why: `${who}/${name} is sealed with it. It stays in this browser; to open the bucket anywhere else, paste it there. Without it the bucket is noise, and nobody can give it back.`,
          fields: [{ name: 'key', type: 'static', cls: 'key', value: keyToText(key) }],
          ok: 'I kept it'
        })
      }
      nextNote = `${who}/${name} is ready. Drop files into it.`
      if (location.hash === `#/${who}/${name}`) await render()
      else location.hash = `#/${who}/${name}`
    } catch (e) {
      if (refused(e)) { forget(); askForCredential('Making a bucket needs a credential. It stays in this tab.', run) } else says(`That did not land: ${e.message}`)
    }
  }
  run()
})

$('settings').addEventListener('click', async () => {
  const r = route()
  if (r.view !== 'browse' || !state) return
  const current = state.visibility
  const answer = await ask({
    title: `${r.owner}/${r.name}`,
    why: state.encrypted
      ? 'A private bucket stays private: its blocks are sealed. To publish something from it, copy it into a public bucket.'
      : 'Unlisted keeps a bucket off the list; anyone holding an address can still read it. A bucket is private from birth, so make a private one and copy into it.',
    fields: [
      { name: 'visibility', type: 'choice', value: current, options: [
        { value: 'public', label: 'Public', why: 'listed, readable by anyone', disabled: state.encrypted },
        { value: 'unlisted', label: 'Unlisted', why: 'off the list; readable by anyone holding an address', disabled: state.encrypted },
        { value: 'private', label: 'Private', why: 'sealed with a key; from birth only', disabled: !state.encrypted }
      ] },
      { name: 'confirm', label: 'to delete this bucket, type its name' }
    ],
    ok: 'Save'
  })
  if (!answer) return
  const run = async () => {
    try {
      if (answer.confirm === r.name) {
        // Every state, not just the head: a deleted bucket leaves no history behind.
        const states = new Set([state.head, ...(await readHistory(reg, state.repo)).map(h => h.state)])
        for (const digest of states) await reg.deleteManifest(state.repo, digest, token).catch(e => { if (!/404/.test(e.message)) throw e })
        nextNote = `${r.owner}/${r.name} is gone (${states.size} state${states.size === 1 ? '' : 's'}). Its objects stay in the store until the next collection.`
        if (location.hash === '#/' || location.hash === '') await render()
        else location.hash = '#/'
        return
      }
      const wanted = answer.visibility
      if (!wanted || wanted === current || state.encrypted) return
      await publishRoot(state.repo, [...state.entries], wanted)
      await renderBrowse(route())
      says(`${r.owner}/${r.name} is now ${wanted}.`)
    } catch (e) {
      if (refused(e)) { forget(); askForCredential('That needs a credential. It stays in this tab.', run) } else says(`That did not land: ${e.message}`)
    }
  }
  run()
})

// ---------------------------------------------------------------- writing
const drop = $('drop')
for (const type of ['dragenter', 'dragover']) drop.addEventListener(type, e => { e.preventDefault(); drop.classList.add('over') })
for (const type of ['dragleave', 'drop']) drop.addEventListener(type, () => drop.classList.remove('over'))
drop.addEventListener('drop', e => { e.preventDefault(); upload([...e.dataTransfer.files]) })
$('file').addEventListener('change', e => upload([...e.target.files]))

function upload (files) {
  if (!files.length || !state) return
  doUpload(files)
}

async function doUpload (files) {
  const r = route()
  const prefix = r.prefix ? r.prefix.replace(/\/$/, '') + '/' : ''
  const text = $('drop-text')
  try {
    if (state.locked) return says('This bucket is private and its key is not here.')
    // The ceiling, before a byte moves.
    let after = state.bytes
    for (const file of files) after += file.size - (state.entries.get(prefix + file.name)?.size || 0)
    if (after > state.quota) return says(`This bucket may hold ${human(state.quota)}; that would make it ${human(after)}. Nothing was moved.`)
    for (const file of files) {
      const key = prefix + file.name
      text.textContent = `Adding ${file.name}…`
      const object = await writeObject(reg, state.repo, key, file, token, ({ done, total }) => {
        text.textContent = `Adding ${file.name} — ${total ? Math.round((done / total) * 100) : 100}%`
      }, state.key)
      const om = objectManifest({ key: object.wire, root: object.root, size: object.size, mtime: object.mtime, blocks: object.blocks })
      await reg.putManifest(state.repo, om.digest, om.bytes, MANIFEST_MT, token)
      state.entries.set(key, { manifest: om.digest, root: object.root, size: object.size, mtime: object.mtime, manifestSize: om.size, ...(state.encrypted ? { wire: object.wire } : {}) })
    }
    text.textContent = 'Publishing the new state…'
    await publishRoot(state.repo, [...state.entries], state.visibility, {}, state.key)
    text.textContent = 'Drop files here to add them to this bucket, or choose files.'
    await renderBrowse(route())
  } catch (e) {
    text.textContent = 'Drop files here to add them to this bucket, or choose files.'
    if (refused(e)) {
      forget()
      askForCredential(`Adding ${files.length} file${files.length === 1 ? '' : 's'} needs a credential. It stays in this tab.`, () => doUpload(files))
    } else says(`That did not land: ${e.message}`)
  }
}

// Every write publishes a new root: the bucket is a pointer, so nothing is edited in place.
async function publishRoot (repo, entries, visibility, extra = {}, bkey = null) {
  // A bucket keeps its birthday, its key fingerprint and its ceiling. They live only on the
  // head, so read it before replacing it; the new head names this one as its parent.
  const prior = await reg.manifest(repo, 'latest').catch(() => null)
  const pa = (prior && prior.json.annotations) || {}
  const now = new Date().toISOString()
  const encrypted = !!(extra['foundation.uor.bucket.encryption'] || pa['foundation.uor.bucket.encryption'])
  const list = []
  for (const [key, v] of entries) {
    list.push([encrypted ? (v.wire || await sealName(bkey, key)) : key, {
      manifest: v.manifest, manifestSize: v.manifestSize || 0, root: v.root, size: v.size, mtime: v.mtime
    }])
  }
  const bytes = list.reduce((s, [, v]) => s + v.size, 0)
  const quota = Number(extra['foundation.uor.bucket.quota.bytes'] || pa['foundation.uor.bucket.quota.bytes'] || DEFAULT_QUOTA)
  if (bytes > quota && bytes > Number(pa['foundation.uor.bucket.bytes'] || 0)) throw new Error(`this bucket may hold ${human(quota)}; that would make it ${human(bytes)}`)
  const tree = build(list)
  for (const node of tree.nodes) {
    if (node.digest === tree.root) continue
    await reg.putManifest(repo, node.digest, node.bytes, INDEX_MT, token)
  }
  const root = JSON.parse(new TextDecoder().decode(tree.nodes.find(n => n.digest === tree.root).bytes))
  root.annotations = {
    ...root.annotations,
    'foundation.uor.bucket.type': 'application/vnd.uor.bucket.v1',
    'foundation.uor.bucket.objects': String(list.length),
    'foundation.uor.bucket.bytes': String(bytes),
    'foundation.uor.bucket.levels': String(tree.levels),
    'foundation.uor.bucket.visibility': visibility,
    'foundation.uor.bucket.created': pa['foundation.uor.bucket.created'] || now,
    'foundation.uor.bucket.quota.bytes': String(quota),
    ...(pa['foundation.uor.bucket.encryption'] ? { 'foundation.uor.bucket.encryption': pa['foundation.uor.bucket.encryption'], 'foundation.uor.bucket.keycheck': pa['foundation.uor.bucket.keycheck'] } : {}),
    ...(prior ? { 'foundation.uor.bucket.parent': prior.digest } : {}),
    'foundation.uor.bucket.published': now,
    ...extra
  }
  const head = new TextEncoder().encode(JSON.stringify(root))
  const digest = await reg.putManifest(repo, 'latest', head, INDEX_MT, token)
  // Every state stays listable: the same root, tagged by its moment.
  await reg.putManifest(repo, 'h-' + Date.now(), head, INDEX_MT, token)
  return digest
}

// ---------------------------------------------------------------- history
// Every publish left a state; here they are, newest first, with Restore. Restoring publishes
// the old root's entries as the newest head, so the state you leave stays listed too.
$('history-close').addEventListener('click', () => { $('history').hidden = true })
$('history-button').addEventListener('click', async () => {
  if (!state) return
  const box = $('history')
  box.hidden = false
  const rows = $('history-rows')
  rows.replaceChildren()
  const r = route()
  const states = await readHistory(reg, state.repo)
  if (!states.length) { rows.appendChild(el('tr')).appendChild(el('td', 'num', 'No states yet.')); return }
  for (const h of states) {
    const tr = el('tr')
    const when = el('td', 'when', h.published.slice(0, 16).replace('T', ' '))
    if (h.state === state.head) when.appendChild(el('span', 'pill', 'now'))
    const addr = el('td', 'addr hide', short(digestToCid(h.state, 0x71))); addr.title = h.state
    const act = el('td', 'act')
    if (h.state !== state.head) {
      const b = el('button', 'button', 'Restore')
      b.addEventListener('click', () => restore(h, r))
      act.appendChild(b)
    }
    tr.append(when, el('td', 'num', `${h.objects} object${h.objects === 1 ? '' : 's'}`), el('td', 'num', human(h.size)), addr, act)
    rows.appendChild(tr)
  }
  box.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
})
async function restore (h, r) {
  const run = async () => {
    try {
      // The old root's entries, as the wire holds them: the names are already what they must be.
      const entries = []
      const walk = async node => {
        const level = Number(node.annotations?.['foundation.uor.bucket.level'] ?? '0')
        for (const child of node.manifests || []) {
          if (level === 0) {
            const c = child.annotations || {}
            entries.push([c['foundation.uor.bucket.key'], { manifest: child.digest, manifestSize: child.size || 0, root: c['foundation.uor.object.root'], size: Number(c['foundation.uor.bucket.size'] || 0), mtime: Number(c['foundation.uor.bucket.mtime'] || 0), wire: state.encrypted ? c['foundation.uor.bucket.key'] : undefined }])
          } else await walk((await reg.manifest(state.repo, child.digest)).json)
        }
      }
      await walk(h.json)
      await publishRoot(state.repo, entries, state.visibility, {}, state.key)
      nextNote = `Restored the state from ${h.published.slice(0, 16).replace('T', ' ')} as the newest. The one you left is still in the history.`
      await renderBrowse(r)
      $('history').hidden = true
    } catch (e) {
      if (refused(e)) { forget(); askForCredential('Restoring needs a credential. It stays in this tab.', run) } else says(`That did not land: ${e.message}`)
    }
  }
  run()
}

// ---------------------------------------------------------------- live
// A bucket is mutable, so a page looking at one goes stale the moment somebody else writes.
// The stream says what changed; the page re-reads only then, and says so quietly.
let live = null
// Where there is no bucket service, `/api/buckets/…/events` is a 404 — and an EventSource
// whose endpoint 404s reconnects for as long as the tab is open. So a stream that never
// opened is closed for good, once, and the page simply does not follow. A stream that did
// open keeps the browser's reconnection, which is the behaviour worth having.
let noFollow = false
function follow (owner, name, cursor = null) {
  if (live) { live.close(); live = null }
  if (noFollow || typeof EventSource === 'undefined') return
  const url = `${EVENTS_BASE}/api/buckets/${owner}/${name}/events` + (cursor ? `?cursor=${encodeURIComponent(cursor)}` : '')
  let stream
  try { stream = new EventSource(url) } catch { return }
  live = stream
  let opened = false
  stream.addEventListener('open', () => { opened = true })
  stream.addEventListener('ready', () => { opened = true })
  stream.addEventListener('error', () => {
    if (opened) return
    stream.close()
    if (live === stream) live = null
    noFollow = true        // no service here; asking again on every bucket would be the same answer
  })
  stream.addEventListener('changes', async e => {
    const batch = JSON.parse(e.data)
    const words = batch.changes.map(c => `${c.op} ${c.path}`).slice(0, 3).join(', ')
    const more = batch.changes.length > 3 ? ` and ${batch.changes.length - 3} more` : ''
    const r = route()
    if (r.view === 'browse') await renderBrowse(r)
    // After the re-render, not before: painting the table rewrites this line.
    says(`${words}${more} — somebody else wrote to this bucket, and this page caught up without being reloaded.`)
  })
  // The service recycles a long-lived stream and hands back a cursor. Following it is simply
  // following again from there — with every listener, which the first version of this quietly
  // dropped, so a recycled page stopped noticing writes.
  stream.addEventListener('reconnect', e => follow(owner, name, JSON.parse(e.data).cursor))
  stream.addEventListener('reset', () => { const r = route(); if (r.view === 'browse') renderBrowse(r) })
}

// ---------------------------------------------------------------- go
async function render () {
  ++renderSeq
  const r = route()
  if (r.view === 'list') await renderList()
  else await renderBrowse(r)
}
render()
