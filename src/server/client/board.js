// Issuectl board — vanilla JS, no build step.
//
// Reads /api/issues, groups by status into Trello columns, lets the user
// filter by type/assignee/epic/label and a free-text search across slug+title.
// Clicking a card opens a <dialog> with the rendered markdown body fetched
// from /api/issues/<slug>; additional `*.md` files in the issue directory
// are listed alongside item.md and fetched on demand.
//
// M2 adds an in-place body editor: the detail dialog can flip between
// "view" and "edit" modes. Edit mode is a <textarea> with a live preview
// pane (POST /api/preview, debounced) plus a `localStorage` draft keyed
// by `(slug, started_editing_at)`. Saves go through PUT /body with an
// `expected_version` token; 409 surfaces a split-pane "your draft vs.
// current on disk" with three resolution actions, never overwriting the
// textarea (the user's typing is the authoritative draft until they
// explicitly discard it). The originating tab dedupes its own SSE echo
// by version equality so no flash on save.

(function () {
  // Status taxonomy mirrors src/main.rs ACTIVE_STATUSES + CLOSING_STATUSES.
  // Closing statuses collapse into one "Closed" column for at-a-glance use.
  // The "Other" trailing column catches any status the server emits that we
  // don't recognise — without it, manual frontmatter edits or future status
  // additions would silently disappear from the board.
  var COLUMNS = [
    { id: 'open', label: 'Open', match: function (s) { return s === 'open'; } },
    { id: 'in-progress', label: 'In progress', match: function (s) { return s === 'in-progress'; } },
    { id: 'testing', label: 'Testing', match: function (s) { return s === 'testing'; } },
    { id: 'closed', label: 'Closed', match: function (s) {
        return ['done', 'fixed', 'wontfix', 'duplicate', 'cannot-reproduce', 'obsolete'].indexOf(s) >= 0;
      } },
    { id: 'other', label: 'Other', match: function () { return true; } },
  ];

  var FILTER_KEYS = ['search', 'type', 'assignee', 'epic', 'label'];

  var state = {
    issues: [],
    warnings: [],
    invalid: {},          // slug -> [LoadWarning]; surfaced from IssueInvalid SSE
    filters: { search: '', type: '', assignee: '', epic: '', label: '' },
    snapshot_seq: 0,
    instance_id: null,
    csrf_token: null,
    // local_versions tracks the canonical version this tab last *wrote*
    // for each slug. The originating tab uses this to silently
    // reconcile its own SSE echo (§6.4); without it every save would
    // re-render the dialog and clobber the textarea with stale state.
    local_versions: {},
    // M2 review F3: the SSE event for our own write can land before
    // the HTTP PUT response resolves. While a save is in flight we
    // queue same-slug events and reconcile them by version once the
    // response arrives — own echo is dropped, others are processed.
    pending_writes: {},
    deferred_events: {},
  };

  var els = {
    board: document.getElementById('board'),
    count: document.getElementById('issue-count'),
    refresh: document.getElementById('refresh'),
    warnings: document.getElementById('warnings'),
    search: document.getElementById('filter-search'),
    type: document.getElementById('filter-type'),
    assignee: document.getElementById('filter-assignee'),
    epic: document.getElementById('filter-epic'),
    label: document.getElementById('filter-label'),
    detail: document.getElementById('detail'),
    detailBody: document.getElementById('detail-body'),
    detailClose: document.getElementById('detail-close'),
  };

  function effectiveAssignee(i) { return i.assignee || i.owner || ''; }

  // === Session bootstrap ===

  function fetchSession() {
    return fetch('/api/session', { headers: { Accept: 'application/json' } })
      .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); })
      .then(function (s) {
        state.csrf_token = s.csrf_token || '';
        state.instance_id = s.instance_id || null;
      })
      .catch(function () { /* best-effort: writes will 403 */ });
  }

  // === Data load ===

  function load() {
    els.board.setAttribute('aria-busy', 'true');
    fetch('/api/issues', { headers: { Accept: 'application/json' } })
      .then(function (r) {
        if (!r.ok) throw new Error('HTTP ' + r.status);
        return r.json();
      })
      .then(function (data) {
        state.issues = data.issues || [];
        state.warnings = data.warnings || [];
        state.snapshot_seq = data.snapshot_seq || 0;
        if (data.instance_id) state.instance_id = data.instance_id;
        renderWarnings();
        populateFilters();
        normalizeFiltersToOptions();
        applyFiltersToInputs();
        syncFiltersToUrl();
        render();
        openSse();
      })
      .catch(function (err) {
        els.board.innerHTML = '<p class="empty">Failed to load: ' + escapeHtml(String(err)) + '</p>';
      })
      .finally(function () { els.board.setAttribute('aria-busy', 'false'); });
  }

  function renderWarnings() {
    var parseWarnings = state.warnings || [];
    var invalidSlugs = Object.keys(state.invalid);
    if (parseWarnings.length === 0 && invalidSlugs.length === 0) {
      els.warnings.hidden = true;
      els.warnings.innerHTML = '';
      return;
    }
    els.warnings.hidden = false;
    var lines = [];
    parseWarnings.forEach(function (w) {
      var label = w.slug ? (w.folder + '/' + w.slug) : (w.folder || '?');
      lines.push('<li><code>' + escapeHtml(label) + '</code> — ' + escapeHtml(w.message) + '</li>');
    });
    invalidSlugs.forEach(function (slug) {
      var ws = state.invalid[slug] || [];
      var msg = ws.map(function (w) { return w.message; }).join('; ') || 'invalid';
      lines.push('<li><code>' + escapeHtml(slug) + '</code> — ' + escapeHtml(msg) + '</li>');
    });
    var total = parseWarnings.length + invalidSlugs.length;
    els.warnings.innerHTML =
      '<h2>' + total + ' parse warning' + (total === 1 ? '' : 's') + '</h2>' +
      '<ul>' + lines.join('') + '</ul>';
  }

  function populateFilters() {
    var types = new Set(), assignees = new Set(), epics = new Set(), labels = new Set();
    state.issues.forEach(function (i) {
      if (i.type) types.add(i.type);
      var a = effectiveAssignee(i);
      if (a) assignees.add(a);
      if (i.epic) epics.add(i.epic);
      if (Array.isArray(i.labels)) i.labels.forEach(function (l) { labels.add(l); });
    });
    fillSelect(els.type, types);
    fillSelect(els.assignee, assignees);
    fillSelect(els.epic, epics);
    fillSelect(els.label, labels);
  }

  function fillSelect(sel, set) {
    var current = sel.value;
    var values = Array.from(set).sort();
    sel.innerHTML = '<option value="">all</option>' +
      values.map(function (v) { return '<option value="' + escapeHtml(v) + '">' + escapeHtml(v) + '</option>'; }).join('');
    if (values.indexOf(current) >= 0) sel.value = current;
  }

  function applyFilters(issues) {
    var f = state.filters;
    var q = f.search.trim().toLowerCase();
    return issues.filter(function (i) {
      if (f.type && i.type !== f.type) return false;
      if (f.assignee && effectiveAssignee(i) !== f.assignee) return false;
      if (f.epic && i.epic !== f.epic) return false;
      if (f.label) {
        if (!Array.isArray(i.labels) || i.labels.indexOf(f.label) < 0) return false;
      }
      if (q) {
        var hay = (i.slug + ' ' + (i.title || '')).toLowerCase();
        if (hay.indexOf(q) < 0) return false;
      }
      return true;
    });
  }

  function render() {
    var visible = applyFilters(state.issues);
    var byCol = COLUMNS.map(function (c) { return { col: c, items: [] }; });
    visible.forEach(function (i) {
      for (var k = 0; k < byCol.length; k++) {
        if (byCol[k].col.match(i.status)) { byCol[k].items.push(i); return; }
      }
    });

    els.count.textContent = visible.length + ' of ' + state.issues.length + ' issue' + (state.issues.length === 1 ? '' : 's');
    els.board.innerHTML = '';
    byCol.forEach(function (group) {
      if (group.col.id === 'other' && group.items.length === 0) return;
      var col = document.createElement('section');
      col.className = 'column';
      col.innerHTML =
        '<div class="column-header"><h2>' + escapeHtml(group.col.label) + '</h2>' +
        '<span class="column-count">' + group.items.length + '</span></div>';
      if (group.items.length === 0) {
        var e = document.createElement('p'); e.className = 'empty'; e.textContent = '—';
        col.appendChild(e);
      } else {
        group.items.forEach(function (i) { col.appendChild(renderCard(i)); });
      }
      els.board.appendChild(col);
    });
  }

  function renderCard(issue) {
    var card = document.createElement('button');
    card.type = 'button';
    card.className = 'card';
    if (state.invalid[issue.slug]) card.classList.add('card-invalid');
    card.setAttribute('data-slug', issue.slug);
    var assignee = effectiveAssignee(issue);
    var meta = [];
    if (issue.type) meta.push('<span class="tag tag-type-' + classSuffix(issue.type) + '">' + escapeHtml(issue.type) + '</span>');
    if (issue.priority && issue.priority !== 'normal') {
      meta.push('<span class="tag tag-priority-' + classSuffix(issue.priority) + '">' + escapeHtml(issue.priority) + '</span>');
    }
    if (['done', 'fixed', 'wontfix', 'duplicate', 'cannot-reproduce', 'obsolete'].indexOf(issue.status) >= 0) {
      meta.push('<span class="tag tag-status-' + classSuffix(issue.status) + '">' + escapeHtml(issue.status) + '</span>');
    }
    if (assignee) meta.push('<span>@' + escapeHtml(assignee) + '</span>');
    if (issue.epic) meta.push('<span>📌 ' + escapeHtml(issue.epic) + '</span>');
    if (state.invalid[issue.slug]) meta.push('<span class="tag tag-invalid">invalid</span>');
    card.innerHTML =
      '<span class="card-title">' + escapeHtml(issue.title || issue.slug) + '</span>' +
      '<span class="card-meta">' +
        '<span class="slug">' + escapeHtml(issue.slug) + '</span>' +
        meta.join('') +
      '</span>';
    card.addEventListener('click', function () { openDetail(issue.slug); });
    return card;
  }

  // === Detail dialog ===

  // The detail dialog has two modes:
  //   - "view"  : rendered markdown + meta table (existing behaviour)
  //   - "edit"  : <textarea> + side-by-side preview + autosave
  // Edit state lives on the dialog DOM element so closing/reopening clears
  // it; the localStorage draft keyed by (slug, started_editing_at) is the
  // crash-safe backup that survives reload.
  var editor = null; // populated while dialog is in edit mode

  function openDetail(slug) {
    els.detailBody.innerHTML = '<p class="empty">Loading…</p>';
    if (typeof els.detail.showModal === 'function') {
      els.detail.showModal();
    } else {
      els.detail.setAttribute('open', '');
    }
    fetch('/api/issues/' + encodeURIComponent(slug))
      .then(function (r) {
        if (!r.ok) throw new Error('HTTP ' + r.status);
        return r.json();
      })
      .then(function (d) {
        renderDetailView(d);
      })
      .catch(function (e) { els.detailBody.innerHTML = '<p class="empty">' + escapeHtml(String(e)) + '</p>'; });
  }

  function renderDetailView(d) {
    editor = null;
    els.detailBody.innerHTML = renderDetail(d);
    wireDocLinks(d);
    var editBtn = els.detailBody.querySelector('#edit-body');
    if (editBtn) editBtn.addEventListener('click', function () { enterEditMode(d); });
  }

  function renderDetail(d) {
    var rows = [];
    function row(label, value) {
      if (value === undefined || value === null || value === '' ||
          (Array.isArray(value) && value.length === 0)) return;
      var v = Array.isArray(value) ? value.map(escapeHtml).join(', ') : escapeHtml(String(value));
      rows.push('<dt>' + escapeHtml(label) + '</dt><dd>' + v + '</dd>');
    }
    row('Slug', d.slug);
    row('Status', d.status + ' (' + d.folder + ')');
    row('Type', d.type);
    row('Priority', d.priority);
    row('Assignee', d.assignee);
    row('Owner', d.owner);
    row('Reporter', d.reporter);
    row('Epic', d.epic);
    row('Labels', d.labels);
    row('Related', d.related);
    row('Created', d.created);
    row('Updated', d.updated);
    row('Closed', d.closed);

    if (Array.isArray(d.commits) && d.commits.length) {
      var commits = d.commits.map(function (c) {
        return '<code>' + escapeHtml(c.hash || '') + '</code> ' + escapeHtml(c.summary || '');
      }).join('<br>');
      rows.push('<dt>Commits</dt><dd>' + commits + '</dd>');
    }

    var docsNav = '';
    if (Array.isArray(d.docs) && d.docs.length > 0) {
      docsNav = '<nav class="doc-nav"><span class="doc-nav-label">Docs:</span>' +
        '<button type="button" class="doc-link doc-link-active" data-doc="">item.md</button>' +
        d.docs.map(function (name) {
          return '<button type="button" class="doc-link" data-doc="' +
            encodeURIComponent(name) + '" data-slug="' + encodeURIComponent(d.slug) +
            '">' + escapeHtml(name) + '</button>';
        }).join('') +
        '</nav>';
    }

    return '<h2 class="detail-title">' + escapeHtml(d.title || d.slug) + '</h2>' +
      '<dl class="detail-meta">' + rows.join('') + '</dl>' +
      docsNav +
      '<div class="detail-actions">' +
        '<button type="button" id="edit-body">Edit body</button>' +
      '</div>' +
      '<div class="markdown-body" id="doc-body">' + (d.body_html || '') + '</div>';
  }

  function wireDocLinks(detail) {
    var nav = els.detailBody.querySelector('.doc-nav');
    if (!nav) return;
    nav.addEventListener('click', function (ev) {
      var btn = ev.target.closest('.doc-link');
      if (!btn) return;
      var docName = btn.getAttribute('data-doc') || '';
      nav.querySelectorAll('.doc-link').forEach(function (b) { b.classList.remove('doc-link-active'); });
      btn.classList.add('doc-link-active');
      var body = els.detailBody.querySelector('#doc-body');
      if (!body) return;
      if (docName === '') {
        body.innerHTML = detail.body_html || '';
        return;
      }
      body.innerHTML = '<p class="empty">Loading…</p>';
      fetch('/api/issues/' + encodeURIComponent(detail.slug) + '/docs/' + docName)
        .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); })
        .then(function (d) { body.innerHTML = d.body_html || ''; })
        .catch(function (e) { body.innerHTML = '<p class="empty">' + escapeHtml(String(e)) + '</p>'; });
    });
  }

  // === Edit mode ===

  // Tunables. Centralised so the trade-offs are reviewable in one place
  // rather than buried at call sites.
  var PREVIEW_DEBOUNCE_MS = 250;
  var AUTOSAVE_DEBOUNCE_MS = 5000;
  // Drafts older than this are pruned at startup. Long enough to cover
  // a long weekend; short enough that orphaned localStorage doesn't
  // keep silently restoring stale text after a tab crash.
  var DRAFT_TTL_MS = 7 * 24 * 60 * 60 * 1000;

  function draftKey(slug, startedAt) {
    return 'issuectl-draft:' + slug + ':' + startedAt;
  }

  function readDraft(rawValue) {
    // M2 review F2: drafts must round-trip the version they were started
    // against, otherwise a reload pairs an old body with the *current*
    // server version and silently overwrites whatever changed in the
    // gap. Tolerate legacy plain-string drafts so a newly-deployed
    // build doesn't lose drafts written by the previous version.
    if (rawValue == null) return null;
    if (rawValue.charAt && rawValue.charAt(0) === '{') {
      try {
        var parsed = JSON.parse(rawValue);
        return {
          body: parsed.body || '',
          base_version: parsed.base_version || null,
          started_at: parsed.started_at || null,
        };
      } catch (e) { /* fall through to legacy string handling */ }
    }
    return { body: String(rawValue), base_version: null, started_at: null };
  }

  function writeDraft(key, body, base_version, started_at) {
    try {
      localStorage.setItem(key, JSON.stringify({
        body: body,
        base_version: base_version,
        started_at: started_at,
      }));
    } catch (e) { /* quota / private mode */ }
  }

  function removeDraftsForSlug(slug) {
    var prefix = 'issuectl-draft:' + slug + ':';
    try {
      var keys = [];
      for (var i = 0; i < localStorage.length; i++) {
        var k = localStorage.key(i);
        if (k && k.indexOf(prefix) === 0) keys.push(k);
      }
      keys.forEach(function (k) { localStorage.removeItem(k); });
    } catch (e) {}
  }

  function pruneOldDrafts() {
    var prefix = 'issuectl-draft:';
    var now = Date.now();
    try {
      var keys = [];
      for (var i = 0; i < localStorage.length; i++) {
        var k = localStorage.key(i);
        if (!k || k.indexOf(prefix) !== 0) continue;
        var lastColon = k.lastIndexOf(':');
        var startedAt = parseInt(k.slice(lastColon + 1), 10);
        if (!startedAt || now - startedAt > DRAFT_TTL_MS) keys.push(k);
      }
      keys.forEach(function (k) { localStorage.removeItem(k); });
    } catch (e) {}
  }

  function enterEditMode(detail) {
    var startedAt = Date.now();
    var key = draftKey(detail.slug, startedAt);
    var initialBody = detail.body || '';
    var baseVersion = detail.version || null;
    editor = {
      slug: detail.slug,
      startedAt: startedAt,
      key: key,
      // base_version is the version the textarea was *started* against.
      // expected_version is what the next PUT will send — equal to
      // base_version until a save advances it. Keeping both lets the
      // localStorage restore path send the *original* base instead of
      // the freshly-fetched current version (M2 review F2).
      base_version: baseVersion,
      expected_version: baseVersion,
      saving: false,
      // dirty_during_save is set when an `input` arrives while a save
      // is in flight, so the success/failure handler can schedule a
      // follow-up save instead of silently dropping the keystrokes.
      dirty_during_save: false,
      previewTimer: null,
      autosaveTimer: null,
      lastSavedBody: initialBody,
    };
    state.lastDetailSlug = detail.slug;
    els.detailBody.innerHTML = renderEditMode(detail, initialBody);
    wireEditMode(initialBody);
    schedulePreview(initialBody);
  }

  function renderEditMode(detail, initialBody) {
    return '<h2 class="detail-title">Editing ' + escapeHtml(detail.title || detail.slug) + '</h2>' +
      '<p class="description"><code>' + escapeHtml(detail.slug) + '</code> · v' +
      escapeHtml(String(detail.version || '?').slice(0, 16)) + '…</p>' +
      '<div class="edit-toolbar">' +
        '<button type="button" id="save-body">Save</button>' +
        '<button type="button" id="cancel-edit">Cancel</button>' +
        '<span id="save-status" class="save-status"></span>' +
      '</div>' +
      '<div class="editor-pane">' +
        '<textarea id="body-editor" spellcheck="false">' + escapeHtml(initialBody) + '</textarea>' +
        '<div id="body-preview" class="markdown-body"></div>' +
      '</div>' +
      '<div id="conflict-pane" hidden></div>';
  }

  function wireEditMode(initialBody) {
    var ta = els.detailBody.querySelector('#body-editor');
    var saveBtn = els.detailBody.querySelector('#save-body');
    var cancelBtn = els.detailBody.querySelector('#cancel-edit');

    function onInput() {
      if (!editor) return;
      var body = ta.value;
      // Crash-safe backup (§6.3). Includes base_version so a reload
      // restores the *correct* expected_version, not whatever happens
      // to be on disk now (M2 review F2).
      writeDraft(editor.key, body, editor.base_version, editor.startedAt);
      if (editor.saving) editor.dirty_during_save = true;
      schedulePreview(body);
      scheduleAutosave();
    }
    ta.addEventListener('input', onInput);
    ta.addEventListener('blur', function () { if (editor) saveNow(false); });
    ta.addEventListener('keydown', function (ev) {
      // Ctrl+S / Cmd+S — manual save. Without preventDefault the
      // browser would offer to save the page itself.
      if ((ev.ctrlKey || ev.metaKey) && ev.key === 's') {
        ev.preventDefault();
        saveNow(true);
      }
    });
    saveBtn.addEventListener('click', function () { saveNow(true); });
    cancelBtn.addEventListener('click', function () {
      // Cancel discards every draft for this slug, not just the current
      // session's key. M2 review F8: timestamped keys plus
      // "scan-latest-draft" restore made Cancel resurrect the next-newest
      // zombie draft on the same slug.
      var slug = editor && editor.slug;
      if (slug) removeDraftsForSlug(slug);
      editor = null;
      if (slug) openDetail(slug);
      else closeDetail();
    });

    // Restore an existing draft if the user reloaded mid-edit. The key
    // includes started_editing_at so reloads start a *new* session;
    // we fall back to scanning for any draft for this slug.
    var existing = scanLatestDraft(editor.slug);
    if (existing && existing.key !== editor.key) {
      ta.value = existing.body;
      editor.key = existing.key;
      editor.startedAt = existing.started_at || editor.startedAt;
      // Pull the base_version the draft was started against; falls back
      // to the current version only if the draft predates this fix.
      if (existing.base_version) {
        editor.base_version = existing.base_version;
        editor.expected_version = existing.base_version;
      }
      editor.lastSavedBody = existing.body;
      schedulePreview(existing.body);
    }
  }

  function scanLatestDraft(slug) {
    var prefix = 'issuectl-draft:' + slug + ':';
    var best = null;
    try {
      for (var i = 0; i < localStorage.length; i++) {
        var k = localStorage.key(i);
        if (!k || k.indexOf(prefix) !== 0) continue;
        var startedAt = parseInt(k.slice(prefix.length), 10);
        if (!startedAt) continue;
        if (!best || startedAt > best.startedAt) {
          var draft = readDraft(localStorage.getItem(k));
          if (!draft) continue;
          best = {
            key: k,
            startedAt: startedAt,
            body: draft.body,
            base_version: draft.base_version,
            started_at: draft.started_at || startedAt,
          };
        }
      }
    } catch (e) { return null; }
    return best;
  }

  function schedulePreview(body) {
    if (!editor) return;
    if (editor.previewTimer) clearTimeout(editor.previewTimer);
    editor.previewTimer = setTimeout(function () {
      fetch('/api/preview', {
        method: 'POST',
        headers: csrfJson(),
        body: JSON.stringify({ body: body }),
      })
        .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); })
        .then(function (d) {
          var pane = els.detailBody.querySelector('#body-preview');
          if (pane) pane.innerHTML = d.body_html || '';
        })
        .catch(function () { /* preview is best-effort */ });
    }, PREVIEW_DEBOUNCE_MS);
  }

  function scheduleAutosave() {
    if (!editor) return;
    if (editor.autosaveTimer) clearTimeout(editor.autosaveTimer);
    // 5 s debounce per design D3=C. Avoids 409 storms while a user
    // is mid-keystroke.
    editor.autosaveTimer = setTimeout(function () { saveNow(false); }, AUTOSAVE_DEBOUNCE_MS);
  }

  function saveNow(manual) {
    if (!editor) return;
    var ta = els.detailBody.querySelector('#body-editor');
    if (!ta) return;
    var body = ta.value;
    if (editor.saving) {
      // M2 review F5: don't silently drop intent. Mark dirty so the
      // currently-running save's resolution handler schedules a
      // follow-up. Manual saves (Ctrl+S, Save button, blur) take
      // priority — schedule an immediate follow-up regardless.
      editor.dirty_during_save = true;
      if (manual) editor.manual_during_save = true;
      return;
    }
    if (!manual && body === editor.lastSavedBody) return;
    editor.saving = true;
    editor.dirty_during_save = false;
    editor.manual_during_save = false;
    // M2 review F3: track the in-flight write so SSE echoes that arrive
    // before the HTTP response can be deferred and reconciled by
    // version once the response lands.
    state.pending_writes[editor.slug] = (state.pending_writes[editor.slug] || 0) + 1;
    state.deferred_events[editor.slug] = state.deferred_events[editor.slug] || [];
    setSaveBusy(true);
    setSaveStatus('Saving…');
    fetch('/api/issues/' + encodeURIComponent(editor.slug) + '/body', {
      method: 'PUT',
      headers: csrfJson(),
      body: JSON.stringify({ expected_version: editor.expected_version, body: body }),
    })
      .then(function (r) {
        return r.json().then(function (d) { return { status: r.status, body: d }; },
          // The server occasionally returns an empty body on internal
          // errors; tolerate that instead of breaking the chain.
          function () { return { status: r.status, body: {} }; });
      })
      .then(function (res) {
        // Editor may have been torn down (Cancel, dialog close) while
        // the request was in flight. If so, we still need to drain the
        // pending-write counter so SSE echoes don't queue forever.
        var slug = state.lastDetailSlug;
        var responseVersion = res.body && res.body.version;
        finishPendingWrite(slug, responseVersion);
        if (!editor) return;

        editor.saving = false;
        setSaveBusy(false);
        if (res.status >= 200 && res.status < 300) {
          editor.expected_version = res.body.version;
          editor.base_version = res.body.version;
          editor.lastSavedBody = body;
          state.local_versions[editor.slug] = res.body.version;
          // Drop the localStorage draft on confirmed save — once the
          // server has the bytes the client-side backup is no longer
          // load-bearing.
          try { localStorage.removeItem(editor.key); } catch (e) {}
          setSaveStatus('Saved');
          var pane = els.detailBody.querySelector('#body-preview');
          if (pane && res.body.body_html) pane.innerHTML = res.body.body_html;
          // M2 review F4: apply server response to the board state too,
          // otherwise the originating tab's card stays stale (e.g. a
          // body edit that changed `# Heading` would not refresh the
          // card title until the next reload).
          if (res.body.issue) applyIssueToBoard(res.body.issue);
        } else if (res.status === 409 && res.body && res.body.code === 'version_mismatch') {
          // M2 review F1: the current version lives at the top level of
          // the 409 envelope; the embedded `issue` is the
          // `IssueDetailResponse` shape (M2 server fix added
          // `version` + `body_html` inside it too).
          showConflict(res.body.issue || {}, res.body.version || null);
        } else if (res.status === 429) {
          setSaveStatus('Rate limited; will retry on next edit');
        } else {
          var detail = (res.body && res.body.detail) || ('HTTP ' + res.status);
          setSaveStatus('Save failed: ' + detail);
        }
        // M2 review F5: if more keystrokes arrived during the save,
        // either run them immediately (manual intent) or schedule the
        // next autosave so the buffer doesn't stall waiting for input.
        var dirty = editor.dirty_during_save || (ta.value !== editor.lastSavedBody);
        var manualPending = editor.manual_during_save;
        editor.dirty_during_save = false;
        editor.manual_during_save = false;
        if (dirty) {
          if (manualPending) saveNow(true);
          else scheduleAutosave();
        }
      })
      .catch(function (e) {
        finishPendingWrite(state.lastDetailSlug, null);
        if (!editor) return;
        editor.saving = false;
        setSaveBusy(false);
        setSaveStatus('Save failed: ' + e);
      });
  }

  function setSaveBusy(busy) {
    var saveBtn = els.detailBody.querySelector('#save-body');
    var cancelBtn = els.detailBody.querySelector('#cancel-edit');
    if (saveBtn) saveBtn.disabled = !!busy;
    // Cancel stays enabled — it's the user's escape hatch even mid-save.
    if (cancelBtn) cancelBtn.disabled = false;
  }

  function applyIssueToBoard(issue) {
    var idx = state.issues.findIndex(function (i) { return i.slug === issue.slug; });
    var summary = {
      slug: issue.slug,
      folder: issue.folder,
      title: issue.title,
      type: issue.type,
      status: issue.status,
      priority: issue.priority,
      assignee: issue.assignee,
      owner: issue.owner,
      epic: issue.epic,
      labels: issue.labels || [],
    };
    if (idx >= 0) state.issues[idx] = Object.assign({}, state.issues[idx], summary);
    else state.issues.push(summary);
    populateFilters();
    normalizeFiltersToOptions();
    applyFiltersToInputs();
    render();
  }

  function finishPendingWrite(slug, responseVersion) {
    if (!slug) return;
    var pending = state.pending_writes[slug] || 0;
    if (pending > 0) state.pending_writes[slug] = pending - 1;
    var deferred = state.deferred_events[slug] || [];
    state.deferred_events[slug] = [];
    deferred.forEach(function (evt) {
      // Drop our own echo (matches the response version); process
      // anything else as an external edit so concurrent writers from
      // other clients land correctly.
      if (responseVersion && evt.version === responseVersion) {
        state.local_versions[slug] = evt.version;
        return;
      }
      handleEvent(evt);
    });
  }

  function setSaveStatus(msg) {
    var s = els.detailBody.querySelector('#save-status');
    if (s) s.textContent = msg;
  }

  function showConflict(theirs, currentVersion) {
    if (!editor) return;
    var pane = els.detailBody.querySelector('#conflict-pane');
    if (!pane) return;
    pane.hidden = false;
    var theirBody = (theirs && theirs.body) || '';
    pane.innerHTML =
      '<h3>Conflict — body changed on disk</h3>' +
      '<p>Your draft is in the textarea above. The current on-disk body is shown below. Pick one:</p>' +
      '<div class="conflict-actions">' +
        '<button type="button" id="conflict-keep-mine">Keep mine (overwrite theirs)</button>' +
        '<button type="button" id="conflict-keep-theirs">Keep theirs (discard my draft)</button>' +
        '<button type="button" id="conflict-dismiss">Manual merge in textarea</button>' +
      '</div>' +
      '<pre class="conflict-theirs">' + escapeHtml(theirBody) + '</pre>';
    pane.querySelector('#conflict-keep-mine').addEventListener('click', function () {
      // M2 review F1: pull the *new* version from the server-supplied
      // top-level field; the embedded issue object lacked it pre-fix
      // and clicking Keep mine looped on 409 forever.
      if (currentVersion) {
        editor.expected_version = currentVersion;
        editor.base_version = currentVersion;
      }
      pane.hidden = true;
      saveNow(true);
    });
    pane.querySelector('#conflict-keep-theirs').addEventListener('click', function () {
      var ta = els.detailBody.querySelector('#body-editor');
      ta.value = theirBody;
      if (currentVersion) {
        editor.expected_version = currentVersion;
        editor.base_version = currentVersion;
      }
      editor.lastSavedBody = theirBody;
      removeDraftsForSlug(editor.slug);
      // M2 review F10: refresh the preview pane so it reflects the
      // textarea's new content rather than the discarded draft.
      schedulePreview(theirBody);
      pane.hidden = true;
      setSaveStatus('Discarded local draft');
    });
    pane.querySelector('#conflict-dismiss').addEventListener('click', function () {
      // Manual merge: keep the conflict pane visible so the user can
      // copy/paste from "theirs" while editing the textarea (M2 review
      // F9 — hiding it before merging defeated the purpose). Just hide
      // the action buttons; leave the rendered diff/pane in place.
      var actions = pane.querySelector('.conflict-actions');
      if (actions) actions.hidden = true;
      // Do NOT advance expected_version — the next save will conflict
      // again until the user explicitly chooses a side.
    });
  }

  function csrfJson() {
    var h = { 'Content-Type': 'application/json', Accept: 'application/json' };
    if (state.csrf_token) h['X-Issuectl-CSRF'] = state.csrf_token;
    return h;
  }

  function closeDetail() {
    // Esc / backdrop dismiss preserves the draft on purpose — it's the
    // crash-recovery path the design's "localStorage every keystroke"
    // (§6.3) is built around. Explicit Cancel is what discards. The
    // startup prune (DRAFT_TTL_MS) keeps abandoned drafts from
    // accumulating long-term.
    if (typeof els.detail.close === 'function') els.detail.close();
    else els.detail.removeAttribute('open');
    editor = null;
  }

  // === SSE ===

  function openSse() {
    if (typeof EventSource === 'undefined') return;
    if (state.sse) { try { state.sse.close(); } catch (e) {} }
    var url = '/events?since=' + encodeURIComponent(state.snapshot_seq || 0) +
      (state.instance_id ? '&instance=' + encodeURIComponent(state.instance_id) : '');
    var es = new EventSource(url);
    state.sse = es;
    es.onmessage = function (ev) {
      var evt;
      try { evt = JSON.parse(ev.data); } catch (e) { return; }
      handleEvent(evt);
    };
    es.onerror = function () { /* EventSource auto-reconnects */ };
  }

  function handleEvent(evt) {
    if (!evt || !evt.type) return;
    switch (evt.type) {
      case 'IssueUpserted':
      case 'IssueMoved': {
        // M2 review F3: if a save is in flight on this slug, the SSE
        // echo can arrive before fetch() resolves. Defer instead of
        // suppressing or re-fetching; the save handler reconciles by
        // version once the response lands.
        if ((state.pending_writes[evt.slug] || 0) > 0) {
          state.deferred_events[evt.slug] = state.deferred_events[evt.slug] || [];
          state.deferred_events[evt.slug].push(evt);
          return;
        }
        // Echo suppression: if this version matches what *this* tab
        // last wrote for this slug, we already have the up-to-date
        // state from the PUT/PATCH 200 response. Don't re-fetch — it
        // would clobber the textarea mid-edit.
        if (state.local_versions[evt.slug] === evt.version) return;
        // Other tabs' edits still propagate via a refetch.
        load();
        // Clear stale invalid marker if the issue is now valid.
        if (state.invalid[evt.slug]) {
          delete state.invalid[evt.slug];
          renderWarnings();
        }
        return;
      }
      case 'IssueRemoved': {
        state.issues = state.issues.filter(function (i) { return i.slug !== evt.slug; });
        if (state.invalid[evt.slug]) { delete state.invalid[evt.slug]; renderWarnings(); }
        render();
        return;
      }
      case 'IssueInvalid': {
        state.invalid[evt.slug] = evt.warnings || [];
        renderWarnings();
        render();
        return;
      }
      case 'Resync':
      case 'Degraded': {
        // M2 review F6 / design §5.7: discard all per-issue local
        // version state on Resync. Stale entries can otherwise suppress
        // legitimate IssueUpserted events after bulk operations.
        state.local_versions = {};
        load();
        return;
      }
    }
  }

  // === Helpers ===

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  }

  function classSuffix(s) {
    return String(s).toLowerCase().replace(/[^a-z0-9_-]/g, '-');
  }

  // === Filter state in URL ===
  function readFiltersFromUrl() {
    var params = new URLSearchParams(window.location.search);
    FILTER_KEYS.forEach(function (k) {
      var v = params.get(k);
      if (v != null) state.filters[k] = v;
    });
  }

  function syncFiltersToUrl() {
    var params = new URLSearchParams();
    FILTER_KEYS.forEach(function (k) {
      if (state.filters[k]) params.set(k, state.filters[k]);
    });
    var search = params.toString();
    var url = window.location.pathname + (search ? '?' + search : '');
    window.history.replaceState(null, '', url);
  }

  function applyFiltersToInputs() {
    els.search.value = state.filters.search || '';
    ['type', 'assignee', 'epic', 'label'].forEach(function (k) {
      els[k].value = state.filters[k] || '';
    });
  }

  function selectHasValue(sel, value) {
    return Array.prototype.some.call(sel.options, function (o) { return o.value === value; });
  }

  function normalizeFiltersToOptions() {
    ['type', 'assignee', 'epic', 'label'].forEach(function (k) {
      var v = state.filters[k];
      if (v && !selectHasValue(els[k], v)) {
        state.filters[k] = '';
      }
    });
  }

  readFiltersFromUrl();
  applyFiltersToInputs();

  els.search.addEventListener('input', function (e) {
    state.filters.search = e.target.value; syncFiltersToUrl(); render();
  });
  ['type', 'assignee', 'epic', 'label'].forEach(function (k) {
    els[k].addEventListener('change', function (e) {
      state.filters[k] = e.target.value; syncFiltersToUrl(); render();
    });
  });
  els.refresh.addEventListener('click', load);
  els.detailClose.addEventListener('click', closeDetail);
  els.detail.addEventListener('click', function (e) {
    if (e.target === els.detail) closeDetail();
  });

  pruneOldDrafts();
  fetchSession().then(load);
})();
