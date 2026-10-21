// The Ferdium recipe API, as Orbit-Pathfinder offers it: the part recipes
// use (badge, loop, CSS, page scripts, links), so a recipe's webview.js
// runs unchanged. It runs in a world of its own — it sees the page, the
// page does not see it — and talks to Pathfinder through the
// "orbitRecipe" message handler.
//
// Pathfinder fills in the three markers below: the recipe's style sheets by
// name, its settings, and its webview.js.
(() => {
  if (window.top !== window) return;
  const files = __FILES__;
  const settings = __SETTINGS__;
  const post = message => {
    try {
      window.webkit.messageHandlers.orbitRecipe.postMessage(JSON.stringify(message));
    } catch (e) {
      console.error('orbit recipe:', e);
    }
  };
  const name = path => String(path).split('/').pop();
  let last = '';

  const Ferdium = {
    setBadge(direct = 0, indirect = 0) {
      const counts = [Ferdium.safeParseInt(direct), Ferdium.safeParseInt(indirect)];
      const key = counts.join(',');
      if (key !== last) {
        last = key;
        post({ badge: counts });
      }
    },
    safeParseInt(text) {
      if (text === undefined || text === null) return 0;
      const n = Number.parseInt(text, 10);
      return Number.isNaN(n) ? 0 : Math.max(n, 0);
    },
    loop(fn) {
      setInterval(() => {
        try {
          fn();
        } catch (e) {
          console.error('orbit recipe:', e);
        }
      }, 1000);
    },
    injectCSS(...paths) {
      for (const path of paths) {
        const css = files[name(path)];
        if (css === undefined) continue;
        const style = document.createElement('style');
        style.textContent = css;
        (document.head || document.documentElement).append(style);
      }
    },
    // Scripts that must run as the page's own: Pathfinder runs them there.
    injectJSUnsafe(...paths) {
      for (const path of paths) post({ unsafe: name(path) });
    },
    isImage(link) {
      const { role } = link.dataset;
      const href = link.getAttribute('href') || '';
      return role === 'img' || /\.(jpe?g|png|gif|webp|svg|avif)(\?|$)/i.test(href);
    },
    openNewWindow(url) {
      post({ open: String(url) });
    },
    handleDarkMode(callback) {
      const query = window.matchMedia('(prefers-color-scheme: dark)');
      callback(query.matches);
      query.addEventListener('change', e => callback(e.matches));
    },
    // Nothing for these to do here.
    setDialogTitle() {},
    setAvatarImage() {},
    onNotify() {},
    releaseServiceWorkers() {},
    clearStorageData() {},
  };
  Ferdium.safePerseInt = Ferdium.safeParseInt;

  const module = { exports: {} };
  const require = what => (what === 'path' ? { join: (...parts) => parts.join('/'), basename: name } : {});
  try {
    (function (module, exports, require, __dirname) {
__WEBVIEW__
    })(module, module.exports, require, '/recipe');
    const run = typeof module.exports === 'function' ? module.exports : module.exports.default;
    if (typeof run === 'function') run(Ferdium, settings);
  } catch (e) {
    console.error('orbit recipe:', e);
  }
})();
