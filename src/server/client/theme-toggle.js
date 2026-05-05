(function() {
  var html = document.documentElement;
  var btn = document.getElementById('theme-toggle');
  if (!btn) return;
  var specDefault = html.getAttribute('data-theme') || 'auto';

  function systemTheme() {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  function resolved(t) { return t === 'auto' ? systemTheme() : t; }

  var stored = null;
  try { stored = localStorage.getItem('issuectl-theme'); } catch (e) {}
  var current = stored || specDefault;
  html.setAttribute('data-theme', current);

  function update() {
    var r = resolved(current);
    btn.textContent = r === 'dark' ? '☀' : '☾';
    btn.title = r === 'dark' ? 'Switch to light theme' : 'Switch to dark theme';
  }
  update();

  btn.addEventListener('click', function() {
    var r = resolved(current);
    current = r === 'dark' ? 'light' : 'dark';
    html.setAttribute('data-theme', current);
    try { localStorage.setItem('issuectl-theme', current); } catch (e) {}
    update();
  });

  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', function() {
    if (current === 'auto') update();
  });
})();
