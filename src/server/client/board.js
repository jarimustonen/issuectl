// Issuectl board — vanilla JS, no build step.
//
// Reads /api/issues, groups by status into Trello columns, lets the user
// filter by type/assignee/epic/label and a free-text search across slug+title.
// Clicking a card opens a <dialog> with the rendered markdown body fetched
// from /api/issues/<slug>; additional `*.md` files in the issue directory
// are listed alongside item.md and fetched on demand.

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
    filters: { search: '', type: '', assignee: '', epic: '', label: '' },
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
        renderWarnings();
        populateFilters();
        // populateFilters rebuilds <option> lists from the loaded data; a
        // URL-supplied filter value (e.g. `?epic=stale-slug` from a
        // bookmark) might point at a value the data no longer contains.
        // Drop those before re-applying — otherwise the <select> shows
        // "all" while state.filters still has the stale value, and the
        // board renders zero matches with no UI affordance to recover.
        normalizeFiltersToOptions();
        applyFiltersToInputs();
        syncFiltersToUrl();
        render();
      })
      .catch(function (err) {
        els.board.innerHTML = '<p class="empty">Failed to load: ' + escapeHtml(String(err)) + '</p>';
      })
      .finally(function () { els.board.setAttribute('aria-busy', 'false'); });
  }

  function renderWarnings() {
    if (!state.warnings || state.warnings.length === 0) {
      els.warnings.hidden = true;
      els.warnings.innerHTML = '';
      return;
    }
    els.warnings.hidden = false;
    els.warnings.innerHTML =
      '<h2>' + state.warnings.length + ' parse warning' +
        (state.warnings.length === 1 ? '' : 's') + '</h2>' +
      '<ul>' +
      state.warnings.map(function (w) {
        var label = w.slug ? (w.folder + '/' + w.slug) : (w.folder || '?');
        return '<li><code>' + escapeHtml(label) + '</code> — ' + escapeHtml(w.message) + '</li>';
      }).join('') +
      '</ul>';
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
      // Hide the "Other" catchall when empty so it doesn't always sit at the
      // end of the board taking up a slot.
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
    card.setAttribute('data-slug', issue.slug);
    var assignee = effectiveAssignee(issue);
    var meta = [];
    if (issue.type) meta.push('<span class="tag tag-type-' + classSuffix(issue.type) + '">' + escapeHtml(issue.type) + '</span>');
    if (issue.priority && issue.priority !== 'normal') {
      meta.push('<span class="tag tag-priority-' + classSuffix(issue.priority) + '">' + escapeHtml(issue.priority) + '</span>');
    }
    // For closed issues, show the actual closing status ("fixed" vs "wontfix"
    // matters at a glance, even though we collapse them into one column).
    if (['done', 'fixed', 'wontfix', 'duplicate', 'cannot-reproduce', 'obsolete'].indexOf(issue.status) >= 0) {
      meta.push('<span class="tag tag-status-' + classSuffix(issue.status) + '">' + escapeHtml(issue.status) + '</span>');
    }
    if (assignee) meta.push('<span>@' + escapeHtml(assignee) + '</span>');
    if (issue.epic) meta.push('<span>📌 ' + escapeHtml(issue.epic) + '</span>');
    // <button> can't legally contain block-level elements; use <span>s and
    // style them as blocks in CSS.
    card.innerHTML =
      '<span class="card-title">' + escapeHtml(issue.title || issue.slug) + '</span>' +
      '<span class="card-meta">' +
        '<span class="slug">' + escapeHtml(issue.slug) + '</span>' +
        meta.join('') +
      '</span>';
    card.addEventListener('click', function () { openDetail(issue.slug); });
    return card;
  }

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
      .then(function (d) { els.detailBody.innerHTML = renderDetail(d); wireDocLinks(d); })
      .catch(function (e) { els.detailBody.innerHTML = '<p class="empty">' + escapeHtml(String(e)) + '</p>'; });
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

  function closeDetail() {
    if (typeof els.detail.close === 'function') els.detail.close();
    else els.detail.removeAttribute('open');
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  }

  /// Reduce a frontmatter value to a CSS-class-token-safe suffix. Today's
  /// type/priority/status values are all enum-validated, but board.js can
  /// also receive hand-edited or future values — keep the class tokens
  /// well-formed instead of trusting the input.
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

  load();
})();
