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
  // Custom-board mode is selected by the server-rendered shell setting
  // `<body data-board-name="...">` for `/board/<name>`. When non-null
  // the board config (group_by, columns, read-only flag) is fetched
  // from `/api/boards/<name>` and drives column rendering + drag PATCH
  // shape. The default (status) board flow is the `else` branch
  // throughout — kept in one file so helpers (toasts, SSE, CSRF, drag
  // chrome, detail dialog) are shared.
  var BOARD_NAME = document.body.getAttribute('data-board-name') || null;

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

  function columnIdFor(status) {
    for (var i = 0; i < COLUMNS.length; i++) {
      if (COLUMNS[i].match(status)) return COLUMNS[i].id;
    }
    return 'other';
  }

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
    // M3: server reflects --no-watch via /api/session.watch_enabled.
    // When false, the SSE stream only carries write-originated events
    // (mutate-layer publishes); external editor saves and `git pull`
    // do NOT propagate. The UI shows the manual refresh button as the
    // primary "see what changed" affordance and labels the watcher as
    // disabled in the degraded banner.
    watch_enabled: true,
    // M3: latched degraded reason from the server. Cleared on Resync
    // (which emits at every successful watcher (re)start). Lives
    // separately from per-issue parse warnings so the two strips don't
    // collide visually.
    degraded_reason: null,
    // slug -> canonical version. Source for `expected_version` on the
    // drag-and-drop PATCH path. Populated from /api/issues, SSE
    // IssueUpserted events, and write responses; differs from
    // `local_versions` (which is the narrower "what *this* tab last
    // wrote" cache used to dedupe self-echo).
    versions: {},
    // slug -> opId of the latest optimistic drag-drop mutation. Lives
    // in a side map so the `_optimisticDrop` tag never pollutes the
    // serialised issue summary (no leak through filter/render/JSON
    // paths). revertDrop only undoes the mutation if the tag still
    // matches its opId.
    optimistic_tags: {},
    // Active drag bookkeeping. `cancelled` is set if SSE delivers a
    // change for the dragged slug mid-drag — the drop then becomes a
    // no-op rather than racing against an already-superseded version.
    dragging: null,
    // Custom-board metadata (BoardResponse from /api/boards/<name>).
    // Null on the default status board. When set, the render + drop
    // paths branch off the status code path.
    board: null,
    // slug -> resolved group_value for the active custom board. Mirrors
    // `versions[]` in role: locally tracked alongside the issue list so
    // optimistic moves can update without re-fetching.
    group_values: {},
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
    degraded: document.getElementById('degraded-banner'),
    toastHost: document.getElementById('toast-host'),
    statusPicker: document.getElementById('status-picker'),
  };

  // Maximum simultaneous toasts on screen. Older ones are evicted FIFO so
  // an SSE/error storm can't fill the screen and push later updates out
  // of view.
  var MAX_TOASTS = 5;
  // Toasts auto-dismiss after this; long enough to read an error, short
  // enough to not pile up. Pause-on-hover/focus extends the window.
  var TOAST_TTL_MS = 6000;

  // Columns the user can drop onto. Active columns map directly to a
  // status. "closed" is special: it spans six closing statuses
  // (done/fixed/wontfix/duplicate/cannot-reproduce/obsolete) so the drop
  // opens a status-picker modal rather than guessing. "Other" remains
  // an invalid drop target — unknown statuses don't get normalised by
  // accident.
  var ACTIVE_DROP_TARGETS = { 'open': 'open', 'in-progress': 'in-progress', 'testing': 'testing' };
  var CLOSING_STATUSES = ['done', 'fixed', 'wontfix', 'duplicate', 'cannot-reproduce', 'obsolete'];
  function isDropTargetColumn(columnId) {
    return ACTIVE_DROP_TARGETS.hasOwnProperty(columnId) || columnId === 'closed';
  }

  function effectiveAssignee(i) { return i.assignee || i.owner || ''; }

  // === Session bootstrap ===

  function fetchSession() {
    return fetch('/api/session', { headers: { Accept: 'application/json' } })
      .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); })
      .then(function (s) {
        state.csrf_token = s.csrf_token || '';
        state.instance_id = s.instance_id || null;
        // `watch_enabled` defaults to true if missing so a server that
        // predates M3 keeps the previous behaviour. --no-watch flips
        // this to false; the banner makes the implication visible.
        state.watch_enabled = s.watch_enabled !== false;
        // M3: a fresh client connecting *after* the server already
        // emitted Degraded over SSE would otherwise miss the banner —
        // the event is past their replay window. Bootstrap reads the
        // latched reason from /api/session so the banner appears
        // immediately on page load if the watcher is already down.
        if (typeof s.degraded_reason === 'string' && s.degraded_reason) {
          state.degraded_reason = s.degraded_reason;
        }
        renderDegradedBanner();
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
        // Snapshot-authoritative: every slug in the response gets its
        // server-supplied version, replacing whatever was cached.
        // Content hashes have no ordering, so a "newer wins" merge by
        // truthiness was wrong (kept stale entries forever after slug
        // reuse, after Resync, or after an external edit reflected in
        // the snapshot but not yet in cache). The cost of dropping a
        // freshly-arrived SSE version is one wasted 409 round-trip;
        // the cost of pinning a stale version is silent corruption.
        var nextVersions = {};
        state.issues.forEach(function (i) {
          if (i.version) nextVersions[i.slug] = i.version;
        });
        state.versions = nextVersions;
        // Snapshot replaces the authoritative state — drop stale
        // optimistic tags so a later PATCH-failure revert can't pin
        // an even-older `prevValue` on top of this fresh data.
        state.optimistic_tags = {};
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

  // M3: board-level health banner. Surfaces (1) `--no-watch` mode so
  // the user knows external edits won't propagate, and (2) the
  // `Degraded` SSE event the server emits after 3 failed watcher
  // restarts (§8.5). Distinct from `#warnings`, which is per-issue
  // parse errors — collapsing the two would hide the watcher state
  // when an unrelated issue happens to have malformed YAML.
  function renderDegradedBanner() {
    var parts = [];
    if (!state.watch_enabled) {
      parts.push(
        '<p><strong>Live updates off.</strong> The server is running ' +
        'with <code>--no-watch</code>; external edits won\'t propagate. ' +
        'Use the refresh button to reload the board.</p>'
      );
    }
    if (state.degraded_reason) {
      // The supervisor exits permanently after 3 failed restarts, so
      // there is no in-process recovery — only a server restart re-
      // enables live updates. Earlier copy promised "until the server
      // recovers" which trapped users into refreshing forever.
      // `degraded_reason` is server-supplied; escape defensively.
      parts.push(
        '<p><strong>Watcher unavailable.</strong> Reason: <code>' +
        escapeHtml(state.degraded_reason) +
        '</code>. Restart <code>issuectl serve</code> to re-enable ' +
        'live updates. Manual refresh still works.</p>'
      );
    }
    if (parts.length === 0) {
      els.degraded.hidden = true;
      els.degraded.innerHTML = '';
      return;
    }
    els.degraded.hidden = false;
    els.degraded.innerHTML = parts.join('');
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
    var epicTitleBySlug = {};
    state.issues.forEach(function (i) {
      if (i.type) types.add(i.type);
      var a = effectiveAssignee(i);
      if (a) assignees.add(a);
      if (i.epic) epics.add(i.epic);
      if (Array.isArray(i.labels)) i.labels.forEach(function (l) { labels.add(l); });
      if (i.type === 'epic' && i.slug && i.title) epicTitleBySlug[i.slug] = i.title;
    });
    fillSelect(els.type, types);
    fillSelect(els.assignee, assignees);
    fillSelect(els.epic, epics, function (slug) {
      var title = epicTitleBySlug[slug];
      return title ? title + ' (' + slug + ')' : slug;
    });
    fillSelect(els.label, labels);
  }

  function fillSelect(sel, set, labelFor) {
    var current = sel.value;
    var values = Array.from(set).sort(function (a, b) {
      var la = labelFor ? labelFor(a) : a;
      var lb = labelFor ? labelFor(b) : b;
      return la.localeCompare(lb);
    });
    sel.innerHTML = '<option value="">all</option>' +
      values.map(function (v) {
        var label = labelFor ? labelFor(v) : v;
        return '<option value="' + escapeHtml(v) + '">' + escapeHtml(label) + '</option>';
      }).join('');
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

  // === Custom-board (group_by != status) mode ===

  function loadBoard() {
    els.board.setAttribute('aria-busy', 'true');
    fetch('/api/boards/' + encodeURIComponent(BOARD_NAME), {
      headers: { Accept: 'application/json' },
    })
      .then(function (r) {
        if (!r.ok) {
          // 422 (Unprocessable Entity) signals a hard YAML
          // validation error; surface its detail so the operator
          // can fix the file. Other non-2xx → generic message.
          if (r.status === 422) {
            return r.json().then(function (d) {
              throw new Error(d.detail || 'invalid board YAML');
            }, function () { throw new Error('HTTP 422'); });
          }
          throw new Error('HTTP ' + r.status);
        }
        return r.json();
      })
      .then(function (data) {
        state.board = data;
        // Snapshot-authoritative: rebuild side maps from scratch.
        // Stale entries for removed slugs would otherwise linger and
        // mislead optimistic-drop revert logic.
        var nextGroupValues = {};
        state.issues = (data.issues || []).map(function (i) {
          var clone = Object.assign({}, i);
          nextGroupValues[clone.slug] = i.group_value || '';
          delete clone.group_value;
          return clone;
        });
        state.group_values = nextGroupValues;
        // Clear optimistic tags too: a snapshot replaces the
        // authoritative state for every slug it carries, so a later
        // PATCH-failure revert against this load's data must not
        // restore an even-older `prevValue`.
        state.optimistic_tags = {};
        state.warnings = data.warnings || [];
        state.snapshot_seq = data.snapshot_seq || 0;
        if (data.instance_id) state.instance_id = data.instance_id;
        var nextVersions = {};
        state.issues.forEach(function (i) {
          if (i.version) nextVersions[i.slug] = i.version;
        });
        state.versions = nextVersions;
        renderBoardBanner();
        renderWarnings();
        applyFilterBarVisibility(data.filters || []);
        render();
        openSse();
      })
      .catch(function (err) {
        els.board.innerHTML = '<p class="empty">Failed to load board: ' +
          escapeHtml(String(err)) + '</p>';
      })
      .finally(function () { els.board.setAttribute('aria-busy', 'false'); });
  }

  // Show only the filter-bar fields the board YAML opts into via
  // `filters: [...]`. Empty list (default) hides the whole row.
  function applyFilterBarVisibility(visibleKeys) {
    var fb = document.querySelector('.filter-bar');
    if (!fb) return;
    if (!visibleKeys || visibleKeys.length === 0) {
      fb.hidden = true;
      return;
    }
    fb.hidden = false;
    // Each <label> wraps one input. The map mirrors FILTER_KEYS from
    // boards.rs and the JS `state.filters` keys.
    var inputs = {
      search: els.search,
      type: els.type,
      assignee: els.assignee,
      epic: els.epic,
      label: els.label,
    };
    Object.keys(inputs).forEach(function (key) {
      var input = inputs[key];
      if (!input) return;
      var label = input.closest('label');
      if (!label) return;
      label.hidden = visibleKeys.indexOf(key) < 0;
    });
  }

  function renderBoardBanner() {
    var el = document.getElementById('board-banner');
    if (!el || !state.board) return;
    if (state.board.read_only) {
      el.hidden = false;
      var reasons = state.board.read_only_reasons || [];
      el.textContent = 'Read-only: ' +
        (reasons.length ? reasons.join('; ') : 'board misconfigured');
    } else {
      el.hidden = true;
      el.textContent = '';
    }
  }

  function renderCustomBoard() {
    var board = state.board;
    var byCol = board.columns.map(function (c) { return { col: c, items: [] }; });
    var unmatched = 0;
    state.issues.forEach(function (i) {
      var v = state.group_values[i.slug] || '';
      var hit = false;
      for (var k = 0; k < byCol.length; k++) {
        if (byCol[k].col.value === v) { byCol[k].items.push(i); hit = true; break; }
      }
      if (!hit) unmatched++;
    });
    var totalShown = state.issues.length - unmatched;
    els.count.textContent = totalShown + ' of ' + state.issues.length +
      ' issue' + (state.issues.length === 1 ? '' : 's');
    els.board.innerHTML = '';
    byCol.forEach(function (group) {
      var col = document.createElement('section');
      col.className = 'column';
      // Column id is the literal group_by value; the data attribute
      // doubles as the PATCH payload value (with empty == clear).
      col.setAttribute('data-column-id', group.col.value);
      col.innerHTML =
        '<div class="column-header"><h2>' + escapeHtml(group.col.label) + '</h2>' +
        '<span class="column-count">' + group.items.length + '</span></div>';
      if (group.items.length === 0) {
        var e = document.createElement('p'); e.className = 'empty'; e.textContent = '—';
        col.appendChild(e);
      } else {
        group.items.forEach(function (i) { col.appendChild(renderCard(i)); });
      }
      if (!state.board || !state.board.read_only) {
        wireCustomColumnDrop(col, group.col.value);
      }
      els.board.appendChild(col);
    });
  }

  function wireCustomColumnDrop(col, columnValue) {
    col.addEventListener('dragover', function (ev) {
      if (!state.dragging) return;
      var sameColumn = state.dragging.sourceColumnValue === columnValue;
      if (sameColumn) {
        col.classList.add('drop-invalid');
        col.classList.remove('drop-target');
        return;
      }
      ev.preventDefault();
      if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'move';
      col.classList.add('drop-target');
      col.classList.remove('drop-invalid');
    });
    col.addEventListener('dragleave', function (ev) {
      if (col.contains(ev.relatedTarget)) return;
      col.classList.remove('drop-target', 'drop-invalid');
    });
    col.addEventListener('drop', function (ev) {
      col.classList.remove('drop-target', 'drop-invalid');
      if (!state.dragging) return;
      if (state.dragging.sourceColumnValue === columnValue) return;
      ev.preventDefault();
      handleCustomDrop(state.dragging, columnValue);
    });
  }

  function handleCustomDrop(drag, newValue) {
    performDragWrite(drag, newValue, customBoardMode());
  }

  function render() {
    if (state.board) { renderCustomBoard(); return; }
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
      col.setAttribute('data-column-id', group.col.id);
      col.innerHTML =
        '<div class="column-header"><h2>' + escapeHtml(group.col.label) + '</h2>' +
        '<span class="column-count">' + group.items.length + '</span></div>';
      if (group.items.length === 0) {
        var e = document.createElement('p'); e.className = 'empty'; e.textContent = '—';
        col.appendChild(e);
      } else {
        group.items.forEach(function (i) { col.appendChild(renderCard(i)); });
      }
      wireColumnDrop(col, group.col.id);
      els.board.appendChild(col);
    });
  }

  // === Drag and drop ===

  function wireColumnDrop(col, columnId) {
    col.addEventListener('dragover', function (ev) {
      if (!state.dragging) return;
      var sameColumn = state.dragging.sourceColumnId === columnId;
      if (!isDropTargetColumn(columnId) || sameColumn) {
        col.classList.add('drop-invalid');
        col.classList.remove('drop-target');
        // No preventDefault — the browser shows the "no-drop" cursor and
        // the drop event won't fire. Source-column drops are silently a
        // no-op rather than a confusing failure toast.
        return;
      }
      ev.preventDefault();
      // effectAllowed was set to 'move' on dragstart; mirror it here so
      // the OS shows the move cursor consistently across browsers.
      if (ev.dataTransfer) ev.dataTransfer.dropEffect = 'move';
      col.classList.add('drop-target');
      col.classList.remove('drop-invalid');
    });
    col.addEventListener('dragleave', function (ev) {
      // dragleave fires when crossing into a child element too. Ignore
      // unless the cursor actually left the column subtree; otherwise
      // the highlight flickers as the user moves over individual cards.
      if (col.contains(ev.relatedTarget)) return;
      col.classList.remove('drop-target', 'drop-invalid');
    });
    col.addEventListener('drop', function (ev) {
      col.classList.remove('drop-target', 'drop-invalid');
      if (!state.dragging) return;
      if (!isDropTargetColumn(columnId)) return;
      if (state.dragging.sourceColumnId === columnId) return;
      ev.preventDefault();
      var drag = state.dragging;
      // Closed column: open the picker so the user names the exact
      // closing status. The picker dispatches `handleDrop` once a
      // status is selected, or cancels.
      if (columnId === 'closed') {
        openClosingStatusPicker(drag);
        return;
      }
      handleDrop(drag, ACTIVE_DROP_TARGETS[columnId]);
    });
  }

  function findIssueIndex(slug) {
    return state.issues.findIndex(function (i) { return i.slug === slug; });
  }

  // Monotonic counter tagging each optimistic move so revertDrop can
  // refuse to clobber state that has advanced past the move it was
  // meant to undo (e.g. a concurrent SSE load() that already produced
  // the authoritative status).
  var nextOpId = 1;

  // Shared drag-write lifecycle. Both the status board (`handleDrop`)
  // and custom boards (`handleCustomDrop`) parameterise this with a
  // `mode` object that names the per-axis bits: how to capture the
  // previous value, how to apply the optimistic mutation locally, how
  // to build the PATCH body, how to apply the server response, how to
  // heal a 409, and which `render`/`refresh` to invoke.
  //
  // Keeping the lifecycle in one place is what closed the original
  // class of custom-board bugs (response not applied for !builtin,
  // `applyIssueToBoard` skipped, post-success `group_values` drift).
  // The price is that two simple drop paths now share one descriptor;
  // the savings is that any future race fix lands once for both.
  function performDragWrite(drag, target, mode) {
    if (drag.cancelled) {
      // External SSE write landed mid-drag (or the slug was removed).
      // Bail without a PATCH; the refresh the SSE handler triggered
      // already carries authoritative state.
      state.dragging = null;
      showToast('Drop cancelled — issue changed in another window', 'error');
      return;
    }
    var idx = findIssueIndex(drag.slug);
    if (idx < 0) {
      state.dragging = null;
      return;
    }
    // Block overlapping writes on the same slug. A failed first PATCH
    // followed by a failed second PATCH would otherwise leave the
    // optimistic state of the second permanently applied: the first
    // revert refuses (opId mismatch), and the second reverts against
    // the already-overwritten "previous" value.
    if ((state.pending_writes[drag.slug] || 0) > 0) {
      state.dragging = null;
      showToast('Move already in progress for this issue', 'error');
      return;
    }
    var expected = state.versions[drag.slug];
    if (!expected) {
      // No version cached → optimistic concurrency would degenerate to
      // an unconditional write. Refuse and refresh.
      state.dragging = null;
      showToast('Cannot move — version unknown, refreshing…', 'error');
      mode.refresh();
      return;
    }
    var prev = mode.capturePrev(drag.slug);
    var opId = nextOpId++;
    drag.patchStarted = true;
    state.optimistic_tags[drag.slug] = opId;
    mode.applyOptimistic(drag.slug, target);
    mode.render();

    beginPendingWrite(drag.slug);
    fetch('/api/issues/' + encodeURIComponent(drag.slug), {
      method: 'PATCH',
      headers: csrfJson(),
      body: JSON.stringify(mode.buildPatch(expected, target)),
    })
      .then(function (r) {
        return r.json().then(
          function (d) { return { status: r.status, body: d, headers: r.headers }; },
          function () { return { status: r.status, body: {}, headers: r.headers }; }
        );
      })
      .then(function (res) {
        var responseVersion = res.body && res.body.version;
        finishPendingWrite(drag.slug, responseVersion);
        if (state.dragging === drag) state.dragging = null;

        if (res.status >= 200 && res.status < 300) {
          if (responseVersion) {
            state.local_versions[drag.slug] = responseVersion;
          }
          mode.applySuccess(drag.slug, target, res, responseVersion);
          clearOptimisticTag(drag.slug, opId);
          return;
        }
        // Failure: revert only if our optimistic tag is still the
        // latest write for this slug. A concurrent reload may have
        // produced authoritative state we must not clobber.
        if (state.optimistic_tags[drag.slug] === opId) {
          delete state.optimistic_tags[drag.slug];
          mode.revert(drag.slug, prev);
          mode.render();
        }
        if (res.status === 409 && res.body && res.body.code === 'version_mismatch') {
          mode.healOnConflict(res, responseVersion);
          showToast('This issue changed externally — refreshed', 'error');
        } else if (res.status === 429) {
          var retry = (res.headers && res.headers.get && res.headers.get('Retry-After')) || '?';
          showToast('Rate limited — retry after ' + retry + 's', 'error');
        } else {
          var detail = (res.body && res.body.detail) || ('HTTP ' + res.status);
          showToast('Move failed: ' + detail, 'error');
        }
      })
      .catch(function (err) {
        finishPendingWrite(drag.slug, null);
        if (state.dragging === drag) state.dragging = null;
        if (state.optimistic_tags[drag.slug] === opId) {
          delete state.optimistic_tags[drag.slug];
          mode.revert(drag.slug, prev);
          mode.render();
        }
        showToast('Move failed: ' + err, 'error');
      });
  }

  // Status-board mode descriptor.
  var STATUS_MODE = {
    capturePrev: function (slug) {
      var idx = findIssueIndex(slug);
      return idx >= 0 ? state.issues[idx].status : null;
    },
    applyOptimistic: function (slug, target) {
      var idx = findIssueIndex(slug);
      if (idx >= 0) {
        state.issues[idx] = Object.assign({}, state.issues[idx], { status: target });
      }
    },
    revert: function (slug, prev) {
      if (prev == null) return;
      var idx = findIssueIndex(slug);
      if (idx >= 0) {
        state.issues[idx] = Object.assign({}, state.issues[idx], { status: prev });
      }
    },
    buildPatch: function (expected, target) {
      return { expected_version: expected, status: target };
    },
    applySuccess: function (slug, target, res, responseVersion) {
      if (res.body.issue) applyIssueToBoard(res.body.issue, responseVersion);
    },
    healOnConflict: function (res, responseVersion) {
      // 409 envelope carries the current issue state; refresh the
      // card in place. If the envelope is missing `issue` (proxy
      // truncation, server bug), fall back to a full reload.
      if (res.body.issue) applyIssueToBoard(res.body.issue, responseVersion);
      else load();
    },
    refresh: function () { load(); },
    render: function () { render(); },
  };

  // Custom-board mode descriptor (group_by != status). Built fresh per
  // call so it captures the current `state.board` reference; the JSON
  // can change underfoot via SSE-triggered reloads.
  function customBoardMode() {
    return {
      capturePrev: function (slug) { return state.group_values[slug] || ''; },
      applyOptimistic: function (slug, target) { state.group_values[slug] = target; },
      revert: function (slug, prev) { state.group_values[slug] = prev || ''; },
      buildPatch: function (expected, target) {
        // Empty-bucket drop clears the field. The loader already
        // rejected this for non-nullable built-ins, so by the time
        // we get here `null` is a legal payload.
        var fieldValue = target === '' ? null : target;
        var patch = { expected_version: expected };
        if (state.board.builtin_group_by) {
          patch[state.board.group_by] = fieldValue;
        } else {
          var custom = {};
          custom[state.board.group_by] = fieldValue;
          patch.custom_fields = custom;
        }
        return patch;
      },
      applySuccess: function (slug, target, res, responseVersion) {
        if (state.board.builtin_group_by && res.body.issue) {
          // Built-in field: the PATCH response carries the canonical
          // issue. applyIssueToBoard projects the IssueSummary
          // explicitly; group_values still needs the new value
          // (server-canonicalized) since it's a side map.
          applyIssueToBoard(res.body.issue, responseVersion);
          state.group_values[slug] = String(res.body.issue[state.board.group_by] || '');
        } else {
          // Custom field: the response does not include `extra`, so
          // there's no server-canonical value to mirror. The 2xx
          // means the server accepted `target`; trust it.
          if (responseVersion) state.versions[slug] = responseVersion;
          state.group_values[slug] = target;
        }
      },
      healOnConflict: function (res, responseVersion) {
        if (state.board.builtin_group_by && res.body.issue) {
          applyIssueToBoard(res.body.issue, responseVersion);
          state.group_values[res.body.issue.slug] = String(
            res.body.issue[state.board.group_by] || ''
          );
          renderCustomBoard();
        } else {
          // Custom group_by: full reload — without `extra` in the
          // 409 envelope, in-place healing can't compute the new
          // group_value.
          loadBoard();
        }
      },
      refresh: function () { loadBoard(); },
      render: function () { renderCustomBoard(); },
    };
  }

  function handleDrop(drag, newStatus) {
    performDragWrite(drag, newStatus, STATUS_MODE);
  }

  function clearOptimisticTag(slug, opId) {
    if (state.optimistic_tags[slug] === opId) {
      delete state.optimistic_tags[slug];
    }
  }

  function openClosingStatusPicker(drag) {
    // Same ownership-transfer flag as handleDrop: keep state.dragging
    // alive across dragend so SSE arriving while the modal is open
    // can still mark the drag as cancelled before the user picks.
    drag.patchStarted = true;
    var dialog = els.statusPicker;
    if (!dialog) {
      // Fallback: if the modal element is missing, use the legacy
      // "drop is invalid" behaviour rather than guessing a status.
      state.dragging = null;
      showToast('Status picker unavailable — please use the issue dialog', 'error');
      return;
    }
    var idx = findIssueIndex(drag.slug);
    var title = idx >= 0 ? (state.issues[idx].title || drag.slug) : drag.slug;
    dialog.innerHTML =
      '<form method="dialog" class="status-picker-form" aria-labelledby="status-picker-title">' +
        '<h2 id="status-picker-title">Close issue</h2>' +
        '<p>Pick a closing status for <code>' + escapeHtml(drag.slug) + '</code> — ' +
        escapeHtml(title) + '.</p>' +
        '<div class="status-picker-options">' +
          CLOSING_STATUSES.map(function (s, i) {
            // autofocus on the first option so keyboard users land
            // ready to pick. native showModal()'s default focus is
            // browser-dependent.
            return '<button type="button" class="status-option"' +
              (i === 0 ? ' autofocus' : '') +
              ' data-status="' + escapeHtml(s) + '">' + escapeHtml(s) + '</button>';
          }).join('') +
        '</div>' +
        '<div class="status-picker-actions">' +
          '<button type="button" id="status-picker-cancel">Cancel</button>' +
        '</div>' +
      '</form>';

    // AbortController centralises listener cleanup so a closed modal
    // can't leak a stale `cancel` handler that nulls a *future* drag's
    // state.dragging when the user reopens the picker.
    var ctl = new AbortController();
    var sig = ctl.signal;
    var settled = false;

    function closePicker() {
      ctl.abort();
      if (dialog.open && typeof dialog.close === 'function') dialog.close();
      else dialog.removeAttribute('open');
    }
    function pick(status) {
      if (settled) return;
      settled = true;
      closePicker();
      handleDrop(drag, status);
    }
    function cancel(ev) {
      if (settled) return;
      settled = true;
      // The native `cancel` event on <dialog> would close the dialog
      // without our null'ing logic; preventDefault keeps closePicker
      // in charge of the close call.
      if (ev && ev.type === 'cancel') ev.preventDefault();
      closePicker();
      // Guard: only null state.dragging if it still points to *our*
      // drag. A second drag that started while the modal was open
      // would have replaced state.dragging; don't kill it.
      if (state.dragging === drag) state.dragging = null;
    }

    dialog.querySelectorAll('.status-option').forEach(function (b) {
      b.addEventListener('click', function () {
        pick(b.getAttribute('data-status'));
      }, { signal: sig });
    });
    dialog.querySelector('#status-picker-cancel').addEventListener('click', cancel, { signal: sig });
    // Backdrop click and Esc both cancel without writing.
    dialog.addEventListener('click', function (ev) {
      if (ev.target === dialog) cancel(ev);
    }, { signal: sig });
    dialog.addEventListener('cancel', cancel, { signal: sig });

    if (typeof dialog.showModal === 'function') dialog.showModal();
    else dialog.setAttribute('open', '');
  }

  function showToast(msg, kind) {
    if (!els.toastHost) return;
    // Cap concurrent toasts so an SSE/error storm can't fill the
    // viewport. FIFO-evict the oldest first.
    while (els.toastHost.children.length >= MAX_TOASTS) {
      els.toastHost.removeChild(els.toastHost.firstChild);
    }
    var t = document.createElement('div');
    t.className = 'toast' + (kind === 'error' ? ' toast-error' : '');
    // The toast carries its own live-region role; the host element
    // does not declare aria-live, so polite-vs-assertive isn't
    // contradicted across nested regions.
    t.setAttribute('role', kind === 'error' ? 'alert' : 'status');

    var text = document.createElement('span');
    text.className = 'toast-text';
    text.textContent = msg;
    t.appendChild(text);

    var close = document.createElement('button');
    close.type = 'button';
    close.className = 'toast-close';
    close.setAttribute('aria-label', 'Dismiss notification');
    close.textContent = '×';
    close.addEventListener('click', function () {
      if (t.parentNode) t.parentNode.removeChild(t);
    });
    t.appendChild(close);

    els.toastHost.appendChild(t);

    var timer = setTimeout(function () {
      if (t.parentNode) t.parentNode.removeChild(t);
    }, TOAST_TTL_MS);
    // Pause the auto-dismiss while the user is reading. focusin covers
    // keyboard users tabbing onto the close button.
    function pause() { clearTimeout(timer); }
    t.addEventListener('mouseenter', pause);
    t.addEventListener('focusin', pause);
  }

  function renderCard(issue) {
    var card = document.createElement('button');
    card.type = 'button';
    card.className = 'card';
    if (state.invalid[issue.slug]) card.classList.add('card-invalid');
    card.setAttribute('data-slug', issue.slug);
    // HTML5 DnD on a <button> works in Chrome/Firefox/Safari; the click
    // handler still fires when the user releases without dragging.
    card.draggable = true;
    // `dragOccurred` distinguishes "click that happened to be wrapped
    // in dragstart/dragend" (which still fires `click`) from "real
    // drag that ended in any state". Without this, a cancelled drag
    // (Esc, drop outside a column) opens the detail dialog on release.
    var dragOccurred = false;
    // Track the drag object this card started so dragend knows whether
    // to clear `state.dragging`. If handleDrop has already begun (drop
    // landed in a real target), it owns the lifecycle and dragend must
    // not null `state.dragging` — that would lose the SSE-cancellation
    // observation window during the in-flight PATCH.
    var startedDrag = null;
    card.addEventListener('dragstart', function (ev) {
      dragOccurred = true;
      // Re-read status from state at drag start — never trust the
      // closure capture from render time, which goes stale if a future
      // code path mutates status without re-rendering.
      var idx = findIssueIndex(issue.slug);
      var currentStatus = idx >= 0 ? state.issues[idx].status : issue.status;
      startedDrag = {
        slug: issue.slug,
        sourceColumnId: columnIdFor(currentStatus),
        // Custom-board source-column key; null on the status board.
        // Same role as `sourceColumnId` but uses the group_value
        // directly so the drop handler can compare without
        // round-tripping through `columnIdFor`.
        sourceColumnValue: state.board ? (state.group_values[issue.slug] || '') : null,
        cancelled: false,
      };
      state.dragging = startedDrag;
      card.classList.add('card-dragging');
      if (ev.dataTransfer) {
        ev.dataTransfer.effectAllowed = 'move';
        // setData is required for Firefox to initiate a drag at all;
        // the value itself is unused — we read state from `state.dragging`.
        try { ev.dataTransfer.setData('text/plain', issue.slug); } catch (e) {}
      }
    });
    card.addEventListener('dragend', function () {
      card.classList.remove('card-dragging');
      // Clean up any column highlight that lingered (e.g. drop landed
      // outside any column, so no `drop` event cleared the class).
      els.board.querySelectorAll('.column.drop-target, .column.drop-invalid')
        .forEach(function (c) { c.classList.remove('drop-target', 'drop-invalid'); });
      // Reset on a microtask so the immediately-following synthetic
      // `click` (some browsers fire it after a no-op drag) sees the
      // suppression flag and bails.
      setTimeout(function () { dragOccurred = false; }, 0);
      // Only clear state.dragging when this dragstart's drag is still
      // the one in flight AND handleDrop hasn't taken ownership yet.
      // After handleDrop starts, it tags the drag with `.patchStarted`
      // and is responsible for clearing state on PATCH resolution —
      // clearing here would close the SSE-cancellation window early.
      if (state.dragging === startedDrag && !startedDrag.patchStarted) {
        state.dragging = null;
      }
      startedDrag = null;
    });
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
    card.addEventListener('click', function (ev) {
      if (dragOccurred) {
        // Some browsers fire `click` after `dragend` even when the
        // user dragged with intent. Swallow it so a half-completed
        // drag (cancelled with Esc, dropped on an invalid target)
        // doesn't open the detail dialog as a surprise.
        ev.preventDefault();
        ev.stopPropagation();
        return;
      }
      openDetail(issue.slug);
    });
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
    // gap. M3 adds `base_body` (the body that pairs with `base_version`)
    // so the three-way merge UI can show the *real* base after a
    // reload — without it, the "Base" pane silently shows the current
    // on-disk body and lies to the user. Tolerate legacy plain-string
    // and pre-M3 JSON drafts so newly-deployed builds don't lose work
    // written by older versions; the conflict UI's "approximate" copy
    // fires when `base_body` is null.
    if (rawValue == null) return null;
    if (rawValue.charAt && rawValue.charAt(0) === '{') {
      try {
        var parsed = JSON.parse(rawValue);
        return {
          body: parsed.body || '',
          base_version: parsed.base_version || null,
          base_body: typeof parsed.base_body === 'string' ? parsed.base_body : null,
          started_at: parsed.started_at || null,
        };
      } catch (e) { /* fall through to legacy string handling */ }
    }
    return { body: String(rawValue), base_version: null, base_body: null, started_at: null };
  }

  function writeDraft(key, body, base_version, base_body, started_at) {
    try {
      localStorage.setItem(key, JSON.stringify({
        body: body,
        base_version: base_version,
        base_body: base_body,
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
      // M3: snapshot the body the textarea was started against. The
      // three-way merge UI uses this as the *base* in (base, ours,
      // theirs) — without it we could only show ours-vs-theirs, which
      // hides which side actually changed. Updated when a save lands
      // (the new on-disk body becomes the new base) so subsequent
      // conflicts in the same session continue to show meaningful
      // three-way context.
      base_body: initialBody,
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
      writeDraft(editor.key, body, editor.base_version, editor.base_body, editor.startedAt);
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
      // M3: pair base_version with the body that was on disk at the
      // time. If the draft predates the M3 base_body field, set null
      // explicitly — the conflict UI's "approximate" copy fires only
      // for null, so leaving it as the current on-disk body would
      // silently lie about what the base actually was.
      editor.base_body = existing.base_body != null ? existing.base_body : null;
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
            base_body: draft.base_body,
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
    beginPendingWrite(editor.slug);
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
          // C5: server canonicalises the body before hashing (CRLF→LF,
          // trailing-newline trim per design §3.2). Use the post-write
          // body the server returned as the new base — without this,
          // a Windows user's CRLF draft and the canonical disk body
          // diverge, and the next conflict's "Base" pane shows
          // newlines that exist nowhere on disk.
          var canonicalBody =
            res.body && res.body.issue && typeof res.body.issue.body === 'string'
              ? res.body.issue.body : body;
          editor.expected_version = res.body.version;
          editor.base_version = res.body.version;
          editor.base_body = canonicalBody;
          editor.lastSavedBody = canonicalBody;
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
          if (res.body.issue) applyIssueToBoard(res.body.issue, res.body.version);
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

  function applyIssueToBoard(issue, version) {
    var idx = state.issues.findIndex(function (i) { return i.slug === issue.slug; });
    // Mirror the full IssueSummary projection from /api/issues so a
    // status PATCH that ripples `closed:` / `updated:` / `folder` is
    // reflected in board state, not silently lost until the next
    // load(). Hand-maintained partial projections drift; this list
    // tracks IssueSummary in src/repo.rs verbatim.
    var summary = {
      slug: issue.slug,
      folder: issue.folder,
      created: issue.created,
      status: issue.status,
      updated: issue.updated,
      priority: issue.priority,
      type: issue.type,
      reporter: issue.reporter,
      assignee: issue.assignee,
      owner: issue.owner,
      epic: issue.epic,
      related: issue.related || [],
      labels: issue.labels || [],
      closed: issue.closed,
      commits: issue.commits || [],
      title: issue.title,
    };
    // The board summary's `version` is what the next drag-and-drop PATCH
    // will send. Prefer the explicit value (server-confirmed post-write)
    // and fall back to whatever was on the issue payload — both
    // PUT/PATCH responses carry the new version, but defensive code
    // means a stale cache won't silently issue 409s on the next drop.
    if (version) summary.version = version;
    else if (issue.version) summary.version = issue.version;
    if (summary.version) state.versions[issue.slug] = summary.version;
    if (idx >= 0) state.issues[idx] = Object.assign({}, state.issues[idx], summary);
    else state.issues.push(summary);
    populateFilters();
    normalizeFiltersToOptions();
    applyFiltersToInputs();
    render();
  }

  // Shared write-lifecycle hooks. Body PUT and drag-and-drop PATCH both
  // call begin/finishPendingWrite so SSE echoes that race ahead of the
  // HTTP response are deferred and reconciled by version, instead of
  // being treated as external edits and triggering a redundant load().
  function beginPendingWrite(slug) {
    if (!slug) return;
    state.pending_writes[slug] = (state.pending_writes[slug] || 0) + 1;
    state.deferred_events[slug] = state.deferred_events[slug] || [];
  }

  function finishPendingWrite(slug, responseVersion) {
    if (!slug) return;
    var pending = state.pending_writes[slug] || 0;
    if (pending > 0) state.pending_writes[slug] = pending - 1;
    if (state.pending_writes[slug] === 0) delete state.pending_writes[slug];
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

  // M3: three-way conflict UI. The textarea (ours) is never overwritten
  // by the conflict surface; instead we render the three sides of the
  // conflict — base = body at expected_version when this edit started,
  // ours = current textarea content, theirs = body the server just sent
  // back in the 409 — and let the user merge by hand. "Keep mine" /
  // "Keep theirs" remain as one-click shortcuts for the common simple
  // cases. Side-by-side panes are rendered as monospace <pre> blocks
  // because diffing in JS without a library would be either huge or
  // wrong; literal-content side-by-side is honest about what we're
  // showing and lets the user use browser find/copy as needed.
  function showConflict(theirs, currentVersion) {
    if (!editor) return;
    var pane = els.detailBody.querySelector('#conflict-pane');
    if (!pane) return;
    pane.hidden = false;
    var theirBody = (theirs && theirs.body) || '';
    var baseBody = editor.base_body || '';
    var oursBody = '';
    var ta = els.detailBody.querySelector('#body-editor');
    if (ta) oursBody = ta.value;
    var baseSummary = editor.base_body == null
      ? 'unknown (draft restored without base body — three-way context is approximate)'
      : 'body at the version you started editing';
    pane.innerHTML =
      '<h3>Conflict — body changed on disk</h3>' +
      '<p>Three-way view: <strong>base</strong> is the ' + escapeHtml(baseSummary) +
      ', <strong>ours</strong> is the textarea above (still the source of truth — it is never overwritten),' +
      ' <strong>theirs</strong> is the body that landed on disk while you were editing. Edit the textarea by hand to merge, or use a shortcut:</p>' +
      '<div class="conflict-actions">' +
        '<button type="button" id="conflict-keep-mine">Keep mine (overwrite theirs)</button>' +
        '<button type="button" id="conflict-keep-theirs">Keep theirs (discard my draft)</button>' +
        '<button type="button" id="conflict-dismiss">Manual merge in textarea</button>' +
      '</div>' +
      '<div class="three-way-merge">' +
        '<div class="three-way-pane"><h4>Base</h4>' +
          '<pre class="conflict-base">' + escapeHtml(baseBody) + '</pre></div>' +
        '<div class="three-way-pane"><h4>Ours (your draft)</h4>' +
          '<pre class="conflict-ours">' + escapeHtml(oursBody) + '</pre></div>' +
        '<div class="three-way-pane"><h4>Theirs (on disk)</h4>' +
          '<pre class="conflict-theirs">' + escapeHtml(theirBody) + '</pre></div>' +
      '</div>';
    // H2: synchronise vertical scroll across the three panes.
    // Without this the panes drift apart as the user scrolls one and
    // the side-by-side comparison becomes useless. Re-entrancy guard
    // (`syncing`) prevents an event-storm cascade between the three
    // listeners.
    var prePanes = pane.querySelectorAll('.three-way-pane pre');
    var syncing = false;
    prePanes.forEach(function (p) {
      p.addEventListener('scroll', function () {
        if (syncing) return;
        syncing = true;
        prePanes.forEach(function (q) {
          if (q !== p) q.scrollTop = p.scrollTop;
        });
        syncing = false;
      });
    });

    pane.querySelector('#conflict-keep-mine').addEventListener('click', function () {
      // M2 review F1: pull the *new* version from the server-supplied
      // top-level field; the embedded issue object lacked it pre-fix
      // and clicking Keep mine looped on 409 forever.
      if (currentVersion) {
        editor.expected_version = currentVersion;
        editor.base_version = currentVersion;
        // After accepting "theirs as base" we're effectively rebasing
        // our draft on top of the server's body. Update base_body so a
        // subsequent conflict in the same session shows a useful three
        // way view (base = the body our work was rebased onto).
        editor.base_body = theirBody;
      }
      pane.hidden = true;
      saveNow(true);
    });
    pane.querySelector('#conflict-keep-theirs').addEventListener('click', function () {
      var taLocal = els.detailBody.querySelector('#body-editor');
      taLocal.value = theirBody;
      if (currentVersion) {
        editor.expected_version = currentVersion;
        editor.base_version = currentVersion;
      }
      editor.base_body = theirBody;
      editor.lastSavedBody = theirBody;
      removeDraftsForSlug(editor.slug);
      // M2 review F10: refresh the preview pane so it reflects the
      // textarea's new content rather than the discarded draft.
      schedulePreview(theirBody);
      pane.hidden = true;
      setSaveStatus('Discarded local draft');
    });
    pane.querySelector('#conflict-dismiss').addEventListener('click', function () {
      // M3: manual merge is a real third resolution path. Advance
      // expected_version onto the server's current version so the
      // user's hand-merged save lands on the next click; without
      // this, save 409s again with the same `theirs` and the user
      // is forced to click "Keep mine" anyway, defeating the
      // purpose of the manual-merge button. If a *third* writer
      // lands during the merge, the next save will still 409
      // correctly. base_body advances to theirs because the user is
      // rebasing their textarea on top of the server's body.
      if (currentVersion) {
        editor.expected_version = currentVersion;
        editor.base_version = currentVersion;
        editor.base_body = theirBody;
      }
      // Keep the rendered base/ours/theirs panes visible so the user
      // can copy/paste from "theirs" while editing the textarea
      // (M2 review F9 — hiding the diff before merging defeated the
      // purpose). Hide only the action buttons.
      var actions = pane.querySelector('.conflict-actions');
      if (actions) actions.hidden = true;
      setSaveStatus('Edit the textarea, then Save to write your merged version');
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
        // Drag cancellation: only cancel when the *version* actually
        // changed. A no-op write (or an event that re-publishes the
        // same version, e.g. a watcher resync) shouldn't kill an
        // in-progress drag. We compare the SSE event's version to the
        // visibly-applied cache; the cache is updated by load() below
        // (snapshot-authoritative), not pre-applied here. Pre-applying
        // would let the user issue a drag PATCH with a version they
        // haven't visibly observed, silently overwriting an external
        // edit instead of 409-ing.
        if (
          state.dragging &&
          state.dragging.slug === evt.slug &&
          evt.version &&
          evt.version !== state.versions[evt.slug]
        ) {
          state.dragging.cancelled = true;
        }
        // Other tabs' edits still propagate via a refetch.
        if (state.board) loadBoard(); else load();
        // Clear stale invalid marker if the issue is now valid.
        if (state.invalid[evt.slug]) {
          delete state.invalid[evt.slug];
          renderWarnings();
        }
        return;
      }
      case 'IssueRemoved': {
        state.issues = state.issues.filter(function (i) { return i.slug !== evt.slug; });
        delete state.versions[evt.slug];
        if (state.dragging && state.dragging.slug === evt.slug) {
          state.dragging.cancelled = true;
        }
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
      case 'Resync': {
        // M2 review F6 / design §5.7: discard all per-issue local
        // version state on Resync. Stale entries can otherwise suppress
        // legitimate IssueUpserted events after bulk operations.
        state.local_versions = {};
        // M3: a successful (re)start clears the degraded latch. The
        // server emits Resync on every watcher restart (§5.8) so this
        // is the right moment to drop the banner.
        if (evt.reason === 'watcher_restart' && state.degraded_reason) {
          state.degraded_reason = null;
          renderDegradedBanner();
        }
        if (state.board) loadBoard(); else load();
        return;
      }
      case 'Degraded': {
        // §8.5: 3 failed restart attempts. The server has given up on
        // the watcher. Show the banner; the manual refresh button is
        // the user's only "see what changed" affordance until they
        // restart `serve`. Don't refetch — there's no new state to
        // load that we don't already have, and a refetch storm right
        // after a watcher crash would just amplify whatever caused it.
        state.degraded_reason = evt.reason || 'watcher_unavailable';
        renderDegradedBanner();
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
  els.refresh.addEventListener('click', function () {
    if (BOARD_NAME) loadBoard(); else load();
  });
  els.detailClose.addEventListener('click', closeDetail);
  els.detail.addEventListener('click', function (e) {
    if (e.target === els.detail) closeDetail();
  });

  pruneOldDrafts();
  fetchSession().then(function () {
    if (BOARD_NAME) loadBoard(); else load();
  });
})();
