// Rebuilds nginx's autoindex listing of https://rationalazimuth.com/orbit/
// with Orbit's look: header, breadcrumbs, a download card for the ISOs,
// the apt instructions at the top level, and a table of the entries.
// A folder with pictures (screenshots/) also gets a grid of thumbnails and
// a viewer; the top level shows a few of them under the ISO, so whoever
// downloads can see Orbit first. Thumbnails come from the folder's hidden
// .thumbs/ (make-thumbs.py); an animated JXL plays where the browser has
// JPEG XL and shows its still elsewhere.
(function () {
  "use strict";

  var BASE = "/orbit/";
  var THEME = BASE + ".theme/";
  var REPO = location.origin + "/orbit";

  var ICONS = {
    dir: '<path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h4l2 2h9A1.5 1.5 0 0 1 21 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"/>',
    up: '<path d="M12 19V6M6 11l6-6 6 6"/>',
    disc: '<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="2.5"/>',
    pkg: '<path d="M12 3 4 7v10l8 4 8-4V7z"/><path d="m4 7 8 4 8-4M12 11v10"/>',
    key: '<circle cx="8" cy="15" r="4"/><path d="m11 12 8-8M16 7l3 3"/>',
    file: '<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/>',
    down: '<path d="M12 4v11M7 10l5 5 5-5M5 20h14"/>',
    copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3"/>',
    img: '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="10" r="2"/><path d="m21 16-5-5-9 9"/>',
    prev: '<path d="m15 18-6-6 6-6"/>',
    next: '<path d="m9 18 6-6-6-6"/>',
    close: '<path d="M6 6l12 12M18 6 6 18"/>'
  };

  var SHOTS = "screenshots";
  var IMAGE = /\.(png|jpe?g|webp|gif|jxl)$/i;

  function icon(name, cls) {
    return '<svg class="o-ico ' + (cls || "") + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
      'stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + ICONS[name] + "</svg>";
  }

  function esc(s) {
    return String(s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  function human(bytes) {
    if (bytes === null) return "—";
    var units = ["B", "KB", "MB", "GB", "TB"], i = 0, n = bytes;
    while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
    return (i === 0 ? n : n.toFixed(n < 10 ? 1 : 0)) + " " + units[i];
  }

  var MONTHS = { Jan: "01", Feb: "02", Mar: "03", Apr: "04", May: "05", Jun: "06",
                 Jul: "07", Aug: "08", Sep: "09", Oct: "10", Nov: "11", Dec: "12" };

  function isoDate(d) {
    var m = /^(\d{2})-(\w{3})-(\d{4}) (\d{2}:\d{2})$/.exec(d || "");
    return m ? m[3] + "-" + (MONTHS[m[2]] || "??") + "-" + m[1] + " " + m[4] : (d || "");
  }

  function kind(name, isDir) {
    if (isDir) return ["dir", "o-dir"];
    if (/\.iso$/i.test(name)) return ["disc", "o-disc"];
    if (/\.u?deb$/i.test(name)) return ["pkg", ""];
    if (/\.(gpg|asc|sig)$|^Release\.gpg$|^InRelease$/i.test(name)) return ["key", ""];
    if (IMAGE.test(name)) return ["img", "o-dir"];
    return ["file", ""];
  }

  // Read the entries nginx wrote into <pre>: an <a> followed by
  // "dd-Mon-yyyy hh:mm   size" (size is "-" for folders).
  function parse(pre) {
    var out = [];
    var links = pre.querySelectorAll("a");
    for (var i = 0; i < links.length; i++) {
      var a = links[i];
      var href = a.getAttribute("href");
      if (href === "../") continue;
      var tail = a.nextSibling && a.nextSibling.nodeType === 3 ? a.nextSibling.nodeValue : "";
      var m = /(\d{2}-\w{3}-\d{4} \d{2}:\d{2})\s+(\S+)/.exec(tail);
      var isDir = href.slice(-1) === "/";
      var name;
      try { name = decodeURIComponent(href.replace(/\/$/, "")); } catch (e) { name = a.textContent; }
      var size = m && /^\d+$/.test(m[2]) ? parseInt(m[2], 10) : null;
      out.push({ href: href, name: name, dir: isDir, date: m ? m[1] : "", size: size });
    }
    out.sort(function (x, y) {
      if (x.dir !== y.dir) return x.dir ? -1 : 1;
      return x.name.localeCompare(y.name, undefined, { numeric: true });
    });
    return out;
  }

  function crumbs(path) {
    var parts = path.replace(/^\/|\/$/g, "").split("/");
    var html = "", acc = "/";
    for (var i = 0; i < parts.length; i++) {
      acc += parts[i] + "/";
      var label = i === 0 ? "orbit" : decodeURIComponent(parts[i]);
      html += (i ? '<span class="o-sep">/</span>' : "") +
        (i === parts.length - 1 ? "<span>" + esc(label) + "</span>" : '<a href="' + esc(acc) + '">' + esc(label) + "</a>");
    }
    return html;
  }

  function isoCard(e, sumHref) {
    var id = "sum-" + e.name.replace(/[^\w]/g, "");
    var html =
      '<div class="o-card o-iso">' +
        icon("disc", "o-disc") +
        '<div class="o-iso-text">' +
          "<h2>" + esc(e.name) + "</h2>" +
          "<p>Live and installer · amd64 · " + human(e.size) + " · " + esc(isoDate(e.date)) + "</p>" +
          (sumHref ? '<code class="o-sum" id="' + id + '">SHA-256: …</code>' : "") +
        "</div>" +
        '<a class="o-btn" href="' + esc(e.href) + '" download>' + icon("down") + "Download ISO</a>" +
      "</div>";
    if (sumHref) {
      fetch(sumHref).then(function (r) { return r.ok ? r.text() : ""; }).then(function (t) {
        var el = document.getElementById(id);
        var hash = (t.match(/[0-9a-f]{64}/i) || [""])[0];
        if (el) el.innerHTML = hash ? "SHA-256: " + esc(hash) + ' · <a href="' + esc(sumHref) + '">.sha256</a>' : "";
      }).catch(function () {});
    }
    return html;
  }

  function aptCard() {
    var cmd =
      "curl -fsSL " + REPO + "/orbit-archive-keyring.gpg \\\n" +
      "  | sudo tee /usr/share/keyrings/orbit-archive-keyring.gpg >/dev/null\n" +
      'echo "deb [signed-by=/usr/share/keyrings/orbit-archive-keyring.gpg] ' + REPO + ' trixie main" \\\n' +
      "  | sudo tee /etc/apt/sources.list.d/orbit.list\n" +
      "sudo apt update && sudo apt install orbit-desktop";
    return (
      '<div class="o-card o-apt">' +
        '<div class="o-apt-head"><div><h2>Use the apt repository</h2>' +
        "<p>Debian 13 (trixie), amd64. Turns a Debian system into Orbit.</p></div>" +
        '<button class="o-btn o-ghost" type="button" id="o-copy">' + icon("copy") + "Copy</button></div>" +
        '<pre id="o-cmd">' + esc(cmd) + "</pre>" +
      "</div>"
    );
  }

  // --- Gallery -----------------------------------------------------------

  // "wallpaper-fire-blue.jxl" -> "Wallpaper fire blue"
  function caption(name) {
    var s = name.replace(/\.[^.]+$/, "").replace(/[-_]+/g, " ");
    return s.charAt(0).toUpperCase() + s.slice(1);
  }

  function pictures(dir, entries) {
    return entries.filter(function (e) { return !e.dir && IMAGE.test(e.name); }).map(function (e) {
      var base = e.name.replace(/\.[^.]+$/, "");
      return { name: e.name, full: dir + e.href, thumb: dir + ".thumbs/" + encodeURIComponent(base) + ".webp",
               jxl: /\.jxl$/i.test(e.name), size: e.size };
    });
  }

  var shown = [], at = 0, box = null;

  function tile(p, i) {
    return '<button class="o-tile" type="button" data-i="' + i + '" title="' + esc(caption(p.name)) + '">' +
      '<img loading="lazy" src="' + esc(p.thumb) + '" alt="' + esc(caption(p.name)) + '"' +
      // no thumbnail yet: a still picture falls back to the file itself
      (p.jxl ? "" : " onerror=\"if(!this.dataset.f){this.dataset.f=1;this.src='" + esc(p.full) + "'}\"") + ">" +
      (p.jxl ? '<span class="o-badge">animated</span>' : "") +
      '<span class="o-cap">' + esc(caption(p.name)) + "</span></button>";
  }

  function grid(list) {
    return '<div class="o-grid">' + list.map(tile).join("") + "</div>";
  }

  function view(i) {
    at = (i + shown.length) % shown.length;
    var p = shown[at];
    var pic = p.jxl
      ? '<picture><source srcset="' + esc(p.full) + '" type="image/jxl"><img src="' + esc(p.thumb) + '" alt="' + esc(caption(p.name)) + '"></picture>'
      : '<img src="' + esc(p.full) + '" alt="' + esc(caption(p.name)) + '">';
    box.querySelector(".o-lb-pic").innerHTML = pic;
    box.querySelector(".o-lb-cap").innerHTML =
      "<strong>" + esc(caption(p.name)) + "</strong> · " + (at + 1) + " / " + shown.length +
      ' · <a href="' + esc(p.full) + '">' + esc(p.name) + "</a>" + (p.size !== null ? " (" + human(p.size) + ")" : "") +
      (p.jxl ? '<br><span class="o-lb-note">Animated JPEG XL: browsers without JPEG XL show a still frame.</span>' : "");
  }

  function openViewer(list, i) {
    shown = list;
    if (!box) {
      box = document.createElement("div");
      box.className = "o-lb";
      box.setAttribute("role", "dialog");
      box.setAttribute("aria-modal", "true");
      box.innerHTML =
        '<button class="o-lb-btn o-lb-x" type="button" aria-label="Close">' + icon("close") + "</button>" +
        '<button class="o-lb-btn o-lb-p" type="button" aria-label="Previous">' + icon("prev") + "</button>" +
        '<div class="o-lb-pic"></div>' +
        '<button class="o-lb-btn o-lb-n" type="button" aria-label="Next">' + icon("next") + "</button>" +
        '<p class="o-lb-cap"></p>';
      box.addEventListener("click", function (ev) {
        var t = ev.target;
        if (t.closest(".o-lb-p")) view(at - 1);
        else if (t.closest(".o-lb-n")) view(at + 1);
        else if (t.closest(".o-lb-x") || t === box || t.classList.contains("o-lb-pic")) closeViewer();
      });
      document.addEventListener("keydown", function (ev) {
        if (!box.classList.contains("on")) return;
        if (ev.key === "Escape") closeViewer();
        else if (ev.key === "ArrowLeft") view(at - 1);
        else if (ev.key === "ArrowRight") view(at + 1);
      });
      document.body.appendChild(box);
    }
    view(i);
    box.classList.add("on");
    document.documentElement.style.overflow = "hidden";
  }

  function closeViewer() {
    box.classList.remove("on");
    box.querySelector(".o-lb-pic").innerHTML = "";   // stops a big JXL from going on downloading
    document.documentElement.style.overflow = "";
  }

  function wire(root, list) {
    root.addEventListener("click", function (ev) {
      var t = ev.target.closest(".o-tile");
      if (t) openViewer(list, parseInt(t.dataset.i, 10));
    });
  }

  // At the top level: a few screenshots under the ISO, read from screenshots/'s own listing.
  function teaser(slot) {
    var dir = BASE + SHOTS + "/";
    fetch(dir).then(function (r) { return r.ok ? r.text() : ""; }).then(function (html) {
      var pre = new DOMParser().parseFromString(html, "text/html").querySelector("body > pre");
      if (!pre) return;
      var list = pictures(dir, parse(pre));
      if (!list.length) return;
      slot.innerHTML =
        '<div class="o-card o-shots">' +
          '<div class="o-apt-head"><div><h2>See Orbit before you download</h2>' +
          "<p>The desktop, its applications and the animated wallpapers.</p></div>" +
          '<a class="o-btn o-ghost" href="' + esc(dir) + '">' + icon("img") + "All " + list.length + " screenshots</a></div>" +
          grid(list.slice(0, 4)) +
        "</div>";
      wire(slot, list);
    }).catch(function () {});
  }

  function build() {
    var pre = document.querySelector("body > pre");
    if (!pre) return;
    var path = location.pathname;
    var entries = parse(pre);
    var names = {};
    entries.forEach(function (e) { names[e.name] = e; });

    var isos = entries.filter(function (e) { return !e.dir && /\.iso$/i.test(e.name); });
    var top = path === BASE;

    var rows = '<div class="o-row o-head"><span></span><span>Name</span><span class="o-d">Modified</span><span class="o-s">Size</span></div>';
    if (!top) {
      rows += '<div class="o-row">' + icon("up") + '<span class="o-n"><a href="../">..</a></span><span class="o-d"></span><span class="o-s"></span></div>';
    }
    entries.forEach(function (e) {
      var k = kind(e.name, e.dir);
      rows += '<div class="o-row">' + icon(k[0], k[1]) +
        '<span class="o-n"><a href="' + esc(e.href) + '" title="' + esc(e.name) + '">' + esc(e.name) + (e.dir ? "/" : "") + "</a></span>" +
        '<span class="o-d">' + esc(isoDate(e.date)) + "</span>" +
        '<span class="o-s">' + (e.dir ? "" : human(e.size)) + "</span></div>";
    });
    if (!entries.length) rows += '<div class="o-empty">This folder is empty.</div>';

    var pics = pictures(path, entries);

    var html =
      '<div class="o-wrap">' +
        '<header class="o-top"><picture><source srcset="' + THEME + 'mark-light.svg" media="(prefers-color-scheme: light)">' +
          '<img src="' + THEME + 'mark.svg" alt=""></picture>' +
          '<div><p class="o-name">Orbit Linux <span>· downloads</span></p>' +
          '<p class="o-sub">Live ISO and the apt repository of the Orbit desktop</p></div></header>' +
        '<nav class="o-crumbs">' + crumbs(path) + "</nav>" +
        isos.map(function (e) {
          var sum = names[e.name + ".sha256"];
          return isoCard(e, sum ? sum.href : null);
        }).join("") +
        (top ? '<div id="o-teaser"></div>' + aptCard() : "") +
        (pics.length ? '<div class="o-card o-shots" id="o-gallery"><h2>' + pics.length + " pictures</h2>" + grid(pics) + "</div>" : "") +
        '<div class="o-list">' + rows + "</div>" +
        '<p class="o-foot"><a href="https://github.com/julas23/orbit">github.com/julas23/orbit</a></p>' +
      "</div>";

    document.title = "Orbit Linux · " + decodeURIComponent(path);
    document.body.innerHTML = html;

    if (pics.length) wire(document.getElementById("o-gallery"), pics);
    if (top) teaser(document.getElementById("o-teaser"));

    var btn = document.getElementById("o-copy");
    if (btn) {
      btn.addEventListener("click", function () {
        var text = document.getElementById("o-cmd").textContent;
        var done = function () {
          btn.lastChild.nodeValue = "Copied";
          setTimeout(function () { btn.lastChild.nodeValue = "Copy"; }, 1500);
        };
        if (navigator.clipboard) navigator.clipboard.writeText(text).then(done, function () {});
      });
    }
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", build);
  else build();
})();
