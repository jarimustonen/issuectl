// Issuectl board — vanilla JS, no build step.
//
// Reads /api/issues, groups by status into Trello columns, lets the user
// filter by type/assignee/epic/label and a free-text search across slug+title.
// Clicking a card opens a <dialog> with the rendered markdown body fetched
// from /api/issues/<slug>.

(function () {
  // Status taxonomy mirrors src/main.rs ACTIVE_STATUSES + CLOSING_STATUSES.
  // Closing statuses collapse into one "Closed" column for at-a-glance use.
  var COLUMNS = [
    { id: 'open', label: 'Open', match: function (s) { return s === 'open'; } },
    { id: 'in-progress', label: 'In progress', match: function (s) { return s === 'in-progress'; } },
    { id: 'testing', label: 'Testing', match: function (s) { return s === 'testing'; } },
    { id: 'closed', label: 'Closed', match: function (s) {
        return ['done', 'fixed', 'wontfix', 'duplicate', 'cannot-reproduce', 'obsolete'].indexOf(s) >= 0;
      } },
  ];

  var state = {
    issues: [],
    filters: { search: '', type: '', assignee: '', epic: '', label: '' },
  };

  var els = {
    board: document.getElementById('board'),
    count: document.getElementById('issue-count'),
    refresh: document.getElementById('refresh'),
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
        populateFilters();
        render();
      })
      .catch(function (err) {
        els.board.innerHTML = '<p class="empty">Failed to load: ' + escapeHtml(String(err)) + '</p>';
      })
      .finally(function () { els.board.setAttribute('aria-busy', 'false'); });
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
      values.map(function (v) { return '<option value="' + escapeAttr(v) + '">' + escapeHtml(v) + '</option>'; }).join('');
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
    if (issue.type) meta.push('<span class="tag tag-type-' + escapeAttr(issue.type) + '">' + escapeHtml(issue.type) + '</span>');
    if (issue.priority && issue.priority !== 'normal') {
      meta.push('<span class="tag tag-priority-' + escapeAttr(issue.priority) + '">' + escapeHtml(issue.priority) + '</span>');
    }
    if (assignee) meta.push('<span>@' + escapeHtml(assignee) + '</span>');
    if (issue.epic) meta.push('<span>📌 ' + escapeHtml(issue.epic) + '</span>');
    card.innerHTML =
      '<div class="card-title">' + escapeHtml(issue.title || issue.slug) + '</div>' +
      '<div class="card-meta">' +
        '<span class="slug">' + escapeHtml(issue.slug) + '</span>' +
        meta.join('') +
      '</div>';
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
      .then(function (d) { els.detailBody.innerHTML = renderDetail(d); })
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
    return '<h2 style="margin-bottom:0.25rem">' + escapeHtml(d.title || d.slug) + '</h2>' +
      '<dl class="detail-meta">' + rows.join('') + '</dl>' +
      '<div class="markdown-body">' + (d.body_html || '') + '</div>';
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
  function escapeAttr(s) { return escapeHtml(s); }

  // Wire up filter inputs
  els.search.addEventListener('input', function (e) { state.filters.search = e.target.value; render(); });
  ['type', 'assignee', 'epic', 'label'].forEach(function (k) {
    els[k].addEventListener('change', function (e) { state.filters[k] = e.target.value; render(); });
  });
  els.refresh.addEventListener('click', load);
  els.detailClose.addEventListener('click', closeDetail);
  els.detail.addEventListener('click', function (e) {
    if (e.target === els.detail) closeDetail();
  });

  load();
})();
