// Markdown プレビューのページ側の処理
// - typball.update(html): 本文を差し替える (Rust から run_script で呼ばれる)
// - typball.scrollToLine(n, anchor): エディタの行 n に対応する位置が、画面の anchor (0 = 上端,
//   1 = 下端) の高さに来るようにスクロールする。エディタ側はスクロール位置 (先頭 0 〜 末尾 1) を
//   そのまま anchor にして送るので、先頭では上端、末尾では下端で合い、途中は連続的に移る
// - スクロールとリンクのクリックを Rust へ知らせる (wx が注入する window.typball_host.postMessage)
(function () {
  "use strict";

  function post(message) {
    try {
      var host = window.typball_host;
      if (host && host.postMessage) host.postMessage(message);
    } catch (e) { /* ホスト無し (ブラウザで直接開いたとき) */ }
  }

  var suppressUntil = 0;

  function lineElements() {
    var list = document.querySelectorAll("[data-line]");
    var out = [];
    for (var i = 0; i < list.length; i++) {
      var line = parseInt(list[i].getAttribute("data-line"), 10);
      if (line > 0) out.push({ el: list[i], line: line });
    }
    out.sort(function (a, b) { return a.line - b.line; });
    return out;
  }

  function topOf(el) {
    return el.getBoundingClientRect().top + window.scrollY;
  }

  function renderMath(root) {
    // KaTeX が同梱されているときだけ描く (無ければ等幅のまま)
    if (!window.katex) return;
    var nodes = root.querySelectorAll(".math");
    for (var i = 0; i < nodes.length; i++) {
      var node = nodes[i];
      try {
        window.katex.render(node.textContent, node, {
          displayMode: node.classList.contains("math-display"),
          throwOnError: false
        });
      } catch (e) { /* そのまま残す */ }
    }
  }

  // --- Mermaid: ```mermaid のコードブロックを図にする -------------------------------------------
  // Mermaid 本体 (大きい) は、コードブロックがあるときに初めて読み込む。描いた SVG はコードごとに覚えて、
  // 本文を差し替えるたびに描き直さない (入力のたびに図がちらつかない)。書き間違いで描けないものは
  // コードブロックのまま残し、理由を title に入れる。
  var mermaidCache = {};
  var mermaidLoading = null;
  var mermaidSeq = 0;

  function mermaidTheme() {
    return document.documentElement.getAttribute("data-theme") === "dark" ? "dark" : "default";
  }

  function loadMermaid() {
    if (!mermaidLoading) {
      mermaidLoading = new Promise(function (resolve, reject) {
        var script = document.createElement("script");
        script.src = "/assets/mermaid.min.js";
        script.onload = function () {
          try {
            window.mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: mermaidTheme() });
            resolve(window.mermaid);
          } catch (e) { reject(e); }
        };
        script.onerror = function () { reject(new Error("mermaid.min.js could not be loaded")); };
        document.head.appendChild(script);
      });
    }
    return mermaidLoading;
  }

  // root の中の Mermaid のコードブロックを図にする。全部済んだら解決する Promise を返す
  function renderMermaid(root) {
    var codes = root.querySelectorAll("pre > code.language-mermaid");
    if (codes.length === 0) return Promise.resolve();
    var chain = Promise.resolve();
    var stale = false;
    Array.prototype.forEach.call(codes, function (code) {
      var pre = code.parentNode;
      var text = code.textContent;
      var key = mermaidTheme() + "\n" + text;
      var show = function (svg) {
        // 描いている間に本文が差し替わっていたら、後でもう一度 (覚えた SVG ですぐ入る)
        if (!pre.parentNode || !document.contains(pre)) { stale = true; return; }
        var box = document.createElement("div");
        box.className = "tb-mermaid";
        box.innerHTML = svg;
        pre.parentNode.replaceChild(box, pre);
      };
      if (mermaidCache[key]) { show(mermaidCache[key]); return; }
      chain = chain.then(loadMermaid).then(function (mermaid) {
        mermaidSeq += 1;
        return mermaid.render("tb-mermaid-" + mermaidSeq, text);
      }).then(function (result) {
        mermaidCache[key] = result.svg;
        show(result.svg);
      }).catch(function (e) {
        // 書き間違いのとき、Mermaid はエラーの絵をページに残す
        var left = document.getElementById("dtb-mermaid-" + mermaidSeq);
        if (left) left.remove();
        pre.classList.add("tb-mermaid-error");
        pre.title = "Mermaid: " + String((e && e.message) || e).split("\n")[0];
      });
    });
    return chain.then(function () {
      var content = document.getElementById("content");
      if (stale && content) return renderMermaid(content);
    });
  }

  function clamp01(v) { return Math.max(0, Math.min(1, v)); }

  window.typball = {
    update: function (html) {
      var content = document.getElementById("content");
      if (!content) return;
      var y = window.scrollY;
      content.innerHTML = html;
      renderMath(content);
      renderMermaid(content);
      window.scrollTo(0, y);
    },
    scrollToLine: function (line, anchor) {
      anchor = clamp01(anchor || 0);
      var items = lineElements();
      if (!items.length) return;
      var prev = items[0], next = null;
      for (var i = 0; i < items.length; i++) {
        if (items[i].line <= line) prev = items[i];
        else { next = items[i]; break; }
      }
      // 行に対応する文書上の y: 要素の間は行番号で比例配分。基準点が下端寄りなら要素の下辺を使う
      var top = topOf(prev.el);
      var bottom = top + prev.el.getBoundingClientRect().height;
      var y = top * (1 - anchor) + bottom * anchor;
      if (next && next.line > prev.line) {
        y += (topOf(next.el) - top) * (line - prev.line) / (next.line - prev.line);
      }
      var vh = window.innerHeight;
      var max = Math.max(0, document.documentElement.scrollHeight - vh);
      suppressUntil = Date.now() + 500;
      window.scrollTo(0, Math.max(0, Math.min(max, y - anchor * vh - 12 * (1 - anchor))));
    }
  };

  // プレビュー側のスクロール → エディタ。スクロール位置 (0..1) を基準点にして、画面のその高さに
  // ある要素の行を知らせる (エディタ側は同じ基準点でその行を置く)
  var pending = null;
  window.addEventListener("scroll", function () {
    if (Date.now() < suppressUntil || pending) return;
    pending = setTimeout(function () {
      pending = null;
      var items = lineElements();
      if (!items.length) return;
      var vh = window.innerHeight;
      var max = Math.max(0, document.documentElement.scrollHeight - vh);
      var anchor = max > 0 ? clamp01(window.scrollY / max) : 0;
      var probe = window.scrollY + anchor * vh;
      var prev = items[0], next = null;
      for (var i = 0; i < items.length; i++) {
        if (topOf(items[i].el) <= probe) prev = items[i];
        else { next = items[i]; break; }
      }
      var line = prev.line;
      if (next) {
        var a = topOf(prev.el), b = topOf(next.el);
        if (b > a) line += (next.line - prev.line) * clamp01((probe - a) / (b - a));
      }
      post("line:" + Math.round(line) + ":" + anchor.toFixed(3));
    }, 120);
  }, { passive: true });

  // リンク: ページ内はそのまま、外部はブラウザへ、相対パスは Typball で開く
  document.addEventListener("click", function (event) {
    var a = event.target.closest ? event.target.closest("a[href]") : null;
    if (!a) return;
    var href = a.getAttribute("href") || "";
    if (!href || href.charAt(0) === "#") return;
    event.preventDefault();
    if (/^[a-z][a-z0-9+.-]*:/i.test(href)) post("open:" + href);
    else post("file:" + href);
  });

  // 選択の有無を Rust へ知らせる (編集メニューの「コピー」の有効 / 無効。メニューを開くたびに
  // ページへ問い合わせると、wx の同期 RunScript が結果を待つループで固まることがある)
  var hadSelection = false;
  document.addEventListener("selectionchange", function () {
    var has = !!(window.getSelection && String(window.getSelection()).length);
    if (has !== hadSelection) {
      hadSelection = has;
      post("sel:" + (has ? 1 : 0));
    }
  });

  // 編集メニューの「コピー」「すべて選択」(Rust から非同期に呼ばれる)。コピーは選択文字列を
  // Rust に渡してクリップボードへ入れてもらう (スクリプトからの execCommand("copy") は不確実)
  window.typball.copySelection = function () {
    var text = window.getSelection ? String(window.getSelection()) : "";
    if (text) post("copy:" + text);
  };
  window.typball.selectAll = function () {
    var content = document.getElementById("content") || document.body;
    var range = document.createRange();
    range.selectNodeContents(content);
    var sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  };

  document.addEventListener("DOMContentLoaded", function () {
    var content = document.getElementById("content");
    if (content) renderMath(content);
    post("ready");
    // 図まで描き終えたら知らせる (PDF の書き出しは、この知らせを待って印刷する)
    var done = function () { post("rendered"); };
    (content ? renderMermaid(content) : Promise.resolve()).then(done, done);
  });
})();
