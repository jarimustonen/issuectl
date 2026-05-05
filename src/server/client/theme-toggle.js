(function () {
  var html = document.documentElement;
  var btn = document.getElementById('theme-toggle');
  if (!btn) return;

  function validTheme(t) {
    return t === 'auto' || t === 'light' || t === 'dark';
  }
  function systemTheme() {
    try {
      return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    } catch (e) {
      return 'light';
    }
  }
  function resolved(t) { return t === 'auto' ? systemTheme() : t; }

  // Read + validate stored value. theme-bootstrap.js already resolved the
  // initial paint; we keep `current` as the user's stored preference (which
  // may be 'auto') so the toggle button cycles correctly.
  var stored = null;
  try { stored = localStorage.getItem('issuectl-theme'); } catch (e) {}
  var current = validTheme(stored) ? stored : 'auto';
  // Reflect resolved theme into the DOM (bootstrap may not have run, e.g.
  // when JS modules load out of order or this script runs in isolation).
  html.setAttribute('data-theme', resolved(current));

  function update() {
    var r = resolved(current);
    btn.textContent = r === 'dark' ? '☀' : '☾';
    btn.title = r === 'dark' ? 'Switch to light theme' : 'Switch to dark theme';
  }
  update();

  btn.addEventListener('click', function () {
    var r = resolved(current);
    current = r === 'dark' ? 'light' : 'dark';
    html.setAttribute('data-theme', current);
    try { localStorage.setItem('issuectl-theme', current); } catch (e) {}
    update();
  });

  // Older Safari (<14) used MediaQueryList.addListener(); the addEventListener
  // path covers everything else. Guard so an exception on one path doesn't
  // break theming.
  try {
    var mq = window.matchMedia('(prefers-color-scheme: dark)');
    var onChange = function () {
      if (current === 'auto') {
        html.setAttribute('data-theme', resolved(current));
        update();
      }
    };
    if (mq.addEventListener) mq.addEventListener('change', onChange);
    else if (mq.addListener) mq.addListener(onChange);
  } catch (e) { /* ignore */ }
})();
