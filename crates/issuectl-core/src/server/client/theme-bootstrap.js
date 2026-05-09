// Synchronous, blocking script that runs in <head> BEFORE first paint.
// Reads the persisted theme (light/dark/auto) and resolves "auto" to the
// system preference, so the painted page already has the right tokens
// applied — no flash of the wrong theme.
//
// This also lets the stylesheet keep dark tokens in a single
// `[data-theme="dark"]` block: with JS, "auto" never reaches CSS. The
// `@media (prefers-color-scheme: dark)` rule farther down is the
// JS-disabled fallback only.
(function () {
  var stored = null;
  try { stored = localStorage.getItem('issuectl-theme'); } catch (e) {}
  // Validate the stored value before reflecting it into the DOM; an
  // unrelated app on the same origin could have written garbage.
  if (stored !== 'auto' && stored !== 'light' && stored !== 'dark') {
    stored = 'auto';
  }
  var theme = stored;
  if (theme === 'auto') {
    try {
      theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    } catch (e) {
      theme = 'light';
    }
  }
  document.documentElement.setAttribute('data-theme', theme);
})();
