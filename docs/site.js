// Keeps the visitor's light or dark choice between pages. Without this file the site follows the
// system setting and shows no theme button, so every page still works with scripts turned off.
(function () {
  var root = document.documentElement, key = 'rocket-theme';
  try {
    var saved = localStorage.getItem(key);
    if (saved === 'light' || saved === 'dark') root.setAttribute('data-theme', saved);
  } catch (e) {}

  document.addEventListener('DOMContentLoaded', function () {
    var button = document.querySelector('.theme');
    if (button) {
      button.hidden = false;
      button.addEventListener('click', function () {
        var shown = root.getAttribute('data-theme') || (matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark');
        var next = shown === 'light' ? 'dark' : 'light';
        root.setAttribute('data-theme', next);
        try { localStorage.setItem(key, next); } catch (e) {}
      });
    }

    // The phone menu closes once it has done its job, or on Escape.
    var menu = document.querySelector('.menu');
    if (menu) {
      menu.addEventListener('click', function (event) {
        if (event.target.closest('a')) menu.open = false;
      });
      menu.addEventListener('keydown', function (event) {
        if (event.key === 'Escape' && menu.open) {
          menu.open = false;
          menu.querySelector('summary').focus();
        }
      });
    }
  });
})();
