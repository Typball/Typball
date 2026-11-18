// Markdown プレビューのページ側の処理
// - typball.update(html, lines): 本文を差し替える (Rust から run_script で呼ばれる)。lines は文書の行数
// - typball.scrollToLine(n, anchor): エディタの行 n に対応する位置が、画面の anchor (0 = 上端,
//   1 = 下端) の高さに来るようにスクロールする。計算は scrollsync.js (サイトモードと共通)。
//   対応点は data-line の付いた要素
// - スクロールとリンクのクリックを Rust へ知らせる (wx が注入する window.typball_host.postMessage)
(function () {
  "use strict";

  function post(message) {
    try {
      var host = window.typball_host;
      if (host && host.postMessage) host.postMessage(message);
    } catch (e) { /* ホスト無し (ブラウザで直接開いたとき) */ }
  }

  // 文書の行数 (update で受け取る。最後の対応点から末尾までの配分に使う)
  var totalLines = 0;

  // data-line の付いた要素 (行の昇順)。本文を差し替えるまで覚えておく (スクロールのたびに探し直さない)
  var lineCache = null;

  function lineElements() {
    if (lineCache) return lineCache;
    var list = document.querySelectorAll("[data-line]");
    var out = [];
    for (var i = 0; i < list.length; i++) {
      var line = parseInt(list[i].getAttribute("data-line"), 10);
      if (line > 0) out.push({ el: list[i], line: line });
    }
    out.sort(function (a, b) { return a.line - b.line; });
    // 読み込みの途中 (本文がまだ無い) の結果は覚えない
    if (document.readyState !== "loading") lineCache = out;
    return out;
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

  // スクロールの同期 (scrollsync.js)。対応点は、今の本文の data-line の付いた要素
  var sync = window.typballScrollSync({
    anchors: lineElements,
    totalLines: function () { return totalLines; },
    minAnchors: 1,
    post: post
  });

  window.typball = {
    update: function (html, lines) {
      var content = document.getElementById("content");
      if (!content) return;
      if (lines > 0) totalLines = lines;
      var y = window.scrollY;
      content.innerHTML = html;
      lineCache = null;
      renderMath(content);
      renderMermaid(content);
      // 差し替えで位置がずれても、それをエディタへ返さない
      sync.contentChanged();
      window.scrollTo(0, y);
    },
    scrollToLine: function (line, anchor) {
      sync.scrollToLine(line, anchor);
    }
  };

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
