// 図エディタ (.tbdg) のページ。Typball の WebView に読み込まれ、maxGraph で図を編集する。
//
// ファイルの形式: 中身は SVG。図の見た目はすべて SVG の図形と <text> で書き (Typst は foreignObject や
// CSS の色関数を描けないので使わない)、再編集用のデータ (maxGraph のモデルの XML) を
// <metadata id="typball-diagram"> に文字列として入れる。Typst からは `#image("x.tbdg")` で読める。
//
// Rust (diagram.rs) とのやり取り:
// - ページ → Rust: window.typball_host.postMessage(文字列)
//     "ready"            ページの準備ができた (Rust が load を呼ぶ)
//     "svg:<SVG 全文>"   図が変わった (ファイルの中身になる文字列)
//     "state:<JSON>"     編集メニューの有効 / 無効に使う状態 {undo, redo, sel, editing}
//     "clip:<XML>"       コピーした図形 (Rust がプロセス全体で覚えて、ほかの図のタブにも貼れるようにする)
//     "paste"            貼り付けたい (Rust が覚えている XML を command('paste', xml) で返す)
//     "key:<組み合わせ>"  ページでは使わないショートカット (ctrl+s など。Rust がメニューの処理へ回す)
//     "committed"        command('commit') の返事: 入力中の文字を確定し、その図を "svg:" で渡し終えた
//     "copytext:<文字列>" OS のクリップボードへ入れてほしい文字列 (Mermaid のコード)
// - Rust → ページ: window.typballDiagram.load(text, options) / command(name, arg) / setOptions(options)

import * as mx from './maxgraph.esm.min.js';

const SVG_NS = 'http://www.w3.org/2000/svg';
const META_ID = 'typball-diagram';
const META_FORMAT = 'maxgraph-model-xml';
/** 編集用データの版。読めなくなる変え方をしたときだけ上げる (spec/tbdg-format.md の「版」) */
const META_VERSION = 1;
/**
 * 図の文字のフォント。Typball が同梱している Noto Sans JP を、図専用の名前 (typball-core の
 * DIAGRAM_FONT_FAMILY) で指定する。「Noto Sans JP」のままだと、OS に可変フォント版があるとき Typst が
 * SVG の文字を極細 (Thin) で描いてしまう。ページでは diagram.css の @font-face が同じ名前を定義する
 */
const FONT = 'Typball Sans';
/** スタイルに書くフォントの並び (Typball の外で開いたときは後ろの名前に落ちる) */
const FONT_STACK = `${FONT}, Noto Sans JP, sans-serif`;
const GRID = 10;
/** 書き出す SVG の余白 (px)。線の太さや矢印がはみ出して切れないように */
const PAD = 2;
const MIN_SCALE = 0.1;
const MAX_SCALE = 8;

// ---------------------------------------------------------------------------
// 文言
// ---------------------------------------------------------------------------

const STRINGS = {
  ja: {
    shapes: '図形',
    flow: 'フローチャート',
    lines: '線',
    rect: '四角形',
    rounded: '角丸四角形',
    terminator: '端子 (開始 / 終了)',
    ellipse: '楕円',
    rhombus: 'ひし形 (判断)',
    parallelogram: '平行四辺形 (入出力)',
    hexagon: '六角形 (準備)',
    cylinder: '円柱 (データベース)',
    document: '書類',
    predefined: '定義済み処理',
    manualInput: '手操作入力',
    trapezoid: '台形 (手作業)',
    offpage: 'ページ外結合子',
    note: 'メモ',
    cloud: '雲',
    triangle: '三角形',
    actor: '人',
    doubleEllipse: '二重楕円',
    text: '文字',
    container: '枠 (グループ)',
    arrow: '矢印',
    line: '線',
    arrowBoth: '両矢印',
    arrowDashed: '破線の矢印',
    fill: '塗りの色',
    stroke: '線の色',
    fontColor: '文字の色',
    none: 'なし',
    custom: 'その他の色',
    strokeWidth: '線の太さ',
    dash: '線の種類',
    solid: '実線',
    dashed: '破線',
    dotted: '点線',
    fontSize: '文字の大きさ',
    bold: '太字 (Ctrl+B)',
    italic: '斜体 (Ctrl+I)',
    alignLeft: '左揃え',
    alignCenter: '中央揃え',
    alignRight: '右揃え',
    valign: '文字の上下位置',
    top: '上',
    middle: '中央',
    bottom: '下',
    roundedCorners: '角を丸める',
    shadow: '影',
    route: '線の引き方',
    straight: '直線',
    orthogonal: '直角',
    curved: '曲線',
    startArrow: '始点の形',
    endArrow: '終点の形',
    arrowNone: 'なし',
    arrowBlock: '矢印 (塗り)',
    arrowOpen: '矢印 (線)',
    arrowOval: '丸',
    arrowDiamond: 'ひし形',
    toFront: '最前面へ',
    toBack: '最背面へ',
    arrange: '整列',
    arrangeLeft: '左にそろえる',
    arrangeCenter: '左右中央にそろえる',
    arrangeRight: '右にそろえる',
    arrangeTop: '上にそろえる',
    arrangeMiddle: '上下中央にそろえる',
    arrangeBottom: '下にそろえる',
    distributeH: '左右に等間隔 (3 つ以上)',
    distributeV: '上下に等間隔 (3 つ以上)',
    grid: 'グリッド',
    zoomIn: '拡大 (Ctrl+ホイール)',
    zoomOut: '縮小 (Ctrl+ホイール)',
    zoomReset: '100% に戻す',
    fit: '全体を表示',
    cut: '切り取り',
    copy: 'コピー',
    paste: '貼り付け',
    duplicate: '複製',
    delete: '削除',
    selectAll: 'すべて選択',
    group: 'グループ化',
    ungroup: 'グループ解除',
    editText: '文字を編集',
    mode: 'モード',
    modeFree: '自由な図',
    modeMermaid: '簡略図 (Mermaid)',
    modeNetwork: 'ネットワーク図',
    modePorts: 'ポート図',
    portParts: 'ポートと図形',
    switchName: 'スイッチ',
    swHead: 'スイッチ - {n}ポート',
    swRow1: '{n} ポート (1 段)',
    swRow2: '{n} ポート (2 段)',
    swSfp: ' + SFP {u}',
    newOrder: '番号の並び順',
    newOrderTip: '新しく置く 2 段のスイッチの、ポート番号の並び順。置いた後は、下の「並び順」で変えられます',
    orderZigzag: '上下に交互 (上が奇数)',
    orderZigzagUp: '上下に交互 (下が奇数)',
    orderRows: '段ごと (上の段から)',
    orderRowsUp: '段ごと (下の段から)',
    swBarNone: 'スイッチを選ぶと、ポートの数・段・並び順を変えられます。',
    swPorts: 'ポート',
    swSfpCount: 'SFP',
    swRows: '段',
    swRowsN: '{n} 段',
    swOrder: '並び順',
    swOrderTip: '2 段のときのポート番号の並び順。番号と一緒に、接続先や色も新しい場所へ動きます',
    swTabs: '爪',
    swTabsTip: 'RJ-45 のポートの爪 (切り欠き) の向き。1 つずつ変えるときは、ポートを選んでツールバーの「爪の向きを変える」を押します',
    tabsDown: '下向き',
    tabsUp: '上向き',
    tabsOut: '上の段は上・下の段は下',
    tabsIn: '上の段は下・下の段は上',
    flipTab: '爪の向きを変える (選んだポート)',
    swNumberGroup: '番号',
    swNameGroup: 'ポート名',
    swStart: '開始',
    swSfpShort: 'SFP',
    swContinue: '続き',
    swStartTip: 'スイッチの外観に書かれた番号の、最初の数 (0 から始まる機種は 0)',
    swSfpStartTip: 'SFP の外観の番号の、最初の数。空なら、RJ-45 の続きの番号',
    swNameTip: '設定で使うポート名の、番号の前の部分 (「Gi1/0/」など)。入れると、全部のポートに「Gi1/0/1」のような名前が入ります',
    swNameStartTip: 'ポート名の番号の、最初の数 (「ge-0/0/0」のように 0 から始まる機種は 0)',
    swSfpNameTip: 'SFP のポート名の、番号の前の部分 (「Te1/1/」など)。空なら、RJ-45 と同じ',
    swSfpNameStartTip: 'SFP のポート名の番号の、最初の数。空なら、RJ-45 の続きの番号',
    thName: 'ポート名',
    rj45: 'ポート (RJ-45)',
    sfp: 'ポート (SFP)',
    portsTitle: 'ポートの一覧',
    portsHint: '表に書き込めます (行を選ぶと図のポートも選ばれます)。「コピー」で、選んだ形式の表にして取り出せます。',
    portsEmpty: 'ポートがありません。左のパレットからスイッチを置いてください。',
    thSwitch: 'スイッチ',
    thVlan: 'VLAN',
    thSpeed: '速度',
    iconSet: '絵柄',
    devices: '機器',
    basics: '図形と線',
    portSource: '線の始点側のポート名',
    portTarget: '線の終点側のポート名',
    portHint: 'ポート',
    tableTitle: '結線表',
    tableHint: '線でつないだ機器とポートの一覧です。線を 1 本選ぶと、ツールバーで両端のポート名を入れられます。',
    tableFormat: '結線表の形式',
    thDevice: '機器',
    thPort: 'ポート',
    thPeer: '接続先',
    thNote: 'メモ',
    unnamed: '(名前なし)',
    direction: '流れの向き',
    dirTD: '上から下',
    dirLR: '左から右',
    dirBT: '下から上',
    dirRL: '右から左',
    mermaidHint: '図形の中からドラッグすると矢印を引けます (何も無い所で離すと新しい図形)。配置は自動です。',
    copyCode: 'コピー',
    copyCodeTip: '下の欄の内容を、そのままコピーします',
    copyFence: 'コードブロックでコピー',
    copyFenceTip: '```mermaid で囲んだコードブロックの形でコピーします。Markdown にも Typst にも、そのまま貼れます',
    copied: 'コピーしました',
    mermaidConfirm:
      'Mermaid で表せない要素を、次のように変えてから切り替えます。\n\n{items}\n\n図形は自動で並べ直されます。「元に戻す」(Ctrl+Z) で切り替える前に戻せます。',
    mermaidShapes: '・Mermaid に無い図形 {n} 個 → 四角形にする',
    mermaidLines: '・図形と図形をつないでいない線 {n} 本 → 削除する',
    mermaidGroups: '・枠 / グループ {n} 個 → 解除する (中の図形は残す)',
    mermaidRotated: '・回転した図形 {n} 個 → 回転を戻す',
    mermaidSwitch: '切り替える',
    cancel: 'キャンセル',
    noData:
      'このファイルには図の編集用データが入っていません。\nTypball の図ファイル (.tbdg) ではないか、別のプログラムで書き換えられています。\n内容を守るため、編集はできません。',
    unsupported:
      'この図は、この版の Typball では編集できない形式 (新しい版の Typball で作られた図など) です。\n内容を守るため、編集はできません。',
    broken: '図のデータを読み込めませんでした。\n内容を守るため、編集はできません。\n\n{error}',
  },
  en: {
    shapes: 'Shapes',
    flow: 'Flowchart',
    lines: 'Lines',
    rect: 'Rectangle',
    rounded: 'Rounded rectangle',
    terminator: 'Terminator (start / end)',
    ellipse: 'Ellipse',
    rhombus: 'Diamond (decision)',
    parallelogram: 'Parallelogram (input / output)',
    hexagon: 'Hexagon (preparation)',
    cylinder: 'Cylinder (database)',
    document: 'Document',
    predefined: 'Predefined process',
    manualInput: 'Manual input',
    trapezoid: 'Trapezoid (manual operation)',
    offpage: 'Off-page connector',
    note: 'Note',
    cloud: 'Cloud',
    triangle: 'Triangle',
    actor: 'Actor',
    doubleEllipse: 'Double ellipse',
    text: 'Text',
    container: 'Container (group)',
    arrow: 'Arrow',
    line: 'Line',
    arrowBoth: 'Double arrow',
    arrowDashed: 'Dashed arrow',
    fill: 'Fill color',
    stroke: 'Line color',
    fontColor: 'Text color',
    none: 'None',
    custom: 'Other color',
    strokeWidth: 'Line width',
    dash: 'Line pattern',
    solid: 'Solid',
    dashed: 'Dashed',
    dotted: 'Dotted',
    fontSize: 'Font size',
    bold: 'Bold (Ctrl+B)',
    italic: 'Italic (Ctrl+I)',
    alignLeft: 'Align left',
    alignCenter: 'Align center',
    alignRight: 'Align right',
    valign: 'Vertical text position',
    top: 'Top',
    middle: 'Middle',
    bottom: 'Bottom',
    roundedCorners: 'Rounded corners',
    shadow: 'Shadow',
    route: 'Line routing',
    straight: 'Straight',
    orthogonal: 'Orthogonal',
    curved: 'Curved',
    startArrow: 'Line start',
    endArrow: 'Line end',
    arrowNone: 'None',
    arrowBlock: 'Arrow (filled)',
    arrowOpen: 'Arrow (open)',
    arrowOval: 'Circle',
    arrowDiamond: 'Diamond',
    toFront: 'Bring to front',
    toBack: 'Send to back',
    arrange: 'Align',
    arrangeLeft: 'Align left edges',
    arrangeCenter: 'Align horizontal centers',
    arrangeRight: 'Align right edges',
    arrangeTop: 'Align top edges',
    arrangeMiddle: 'Align vertical centers',
    arrangeBottom: 'Align bottom edges',
    distributeH: 'Distribute horizontally (3 or more)',
    distributeV: 'Distribute vertically (3 or more)',
    grid: 'Grid',
    zoomIn: 'Zoom in (Ctrl+wheel)',
    zoomOut: 'Zoom out (Ctrl+wheel)',
    zoomReset: 'Reset to 100%',
    fit: 'Fit to window',
    cut: 'Cut',
    copy: 'Copy',
    paste: 'Paste',
    duplicate: 'Duplicate',
    delete: 'Delete',
    selectAll: 'Select all',
    group: 'Group',
    ungroup: 'Ungroup',
    editText: 'Edit text',
    mode: 'Mode',
    modeFree: 'Free-form',
    modeMermaid: 'Simple diagram (Mermaid)',
    modeNetwork: 'Network diagram',
    modePorts: 'Port diagram',
    portParts: 'Ports and shapes',
    switchName: 'Switch',
    swHead: 'Switch - {n} ports',
    swRow1: '{n} ports (1 row)',
    swRow2: '{n} ports (2 rows)',
    swSfp: ' + {u} SFP',
    newOrder: 'Numbering',
    newOrderTip: 'How the ports of a new two-row switch are numbered. Change it later with "Order" below the figure',
    orderZigzag: 'Alternating (odd on top)',
    orderZigzagUp: 'Alternating (odd on bottom)',
    orderRows: 'Row by row (top first)',
    orderRowsUp: 'Row by row (bottom first)',
    swBarNone: 'Select a switch to change its number of ports, rows and numbering order.',
    swPorts: 'Ports',
    swSfpCount: 'SFP',
    swRows: 'Rows',
    swRowsN: '{n}',
    swOrder: 'Order',
    swOrderTip: 'How the ports are numbered in two rows. Peers and colours move with their numbers',
    swTabs: 'Tabs',
    swTabsTip: 'Which way the tab (notch) of the RJ-45 ports points. To change single ports, select them and press "Flip tab" in the toolbar',
    tabsDown: 'Down',
    tabsUp: 'Up',
    tabsOut: 'Top row up, bottom row down',
    tabsIn: 'Top row down, bottom row up',
    flipTab: 'Flip tab (selected ports)',
    swNumberGroup: 'Numbers',
    swNameGroup: 'Names',
    swStart: 'from',
    swSfpShort: 'SFP',
    swContinue: 'cont.',
    swStartTip: 'The first number printed on the front of the switch (0 for switches that count from 0)',
    swSfpStartTip: 'The first number printed on the SFP ports. Empty: continue after the RJ-45 ports',
    swNameTip: 'The part of the configuration name before the number ("Gi1/0/"). Every port gets a name like "Gi1/0/1"',
    swNameStartTip: 'The first number of the names (0 for names like "ge-0/0/0")',
    swSfpNameTip: 'The part of the SFP names before the number ("Te1/1/"). Empty: same as the RJ-45 ports',
    swSfpNameStartTip: 'The first number of the SFP names. Empty: continue after the RJ-45 ports',
    thName: 'Name',
    rj45: 'Port (RJ-45)',
    sfp: 'Port (SFP)',
    portsTitle: 'Ports',
    portsHint: 'Type into the table (selecting a row selects the port in the figure). "Copy" gives the table in the chosen format.',
    portsEmpty: 'No ports yet. Place a switch from the palette on the left.',
    thSwitch: 'Switch',
    thVlan: 'VLAN',
    thSpeed: 'Speed',
    iconSet: 'Icon style',
    devices: 'Devices',
    basics: 'Shapes and lines',
    portSource: 'Port name at the start of the line',
    portTarget: 'Port name at the end of the line',
    portHint: 'Port',
    tableTitle: 'Connections',
    tableHint: 'The devices and ports joined by lines. Select one line to enter the port names at both ends in the toolbar.',
    tableFormat: 'Format of the connection table',
    thDevice: 'Device',
    thPort: 'Port',
    thPeer: 'Connected to',
    thNote: 'Note',
    unnamed: '(unnamed)',
    direction: 'Flow direction',
    dirTD: 'Top to bottom',
    dirLR: 'Left to right',
    dirBT: 'Bottom to top',
    dirRL: 'Right to left',
    mermaidHint: 'Drag from inside a shape to draw an arrow (release on empty space for a new shape). The layout is automatic.',
    copyCode: 'Copy',
    copyCodeTip: 'Copy the text below as it is',
    copyFence: 'Copy as code block',
    copyFenceTip: 'Copy as a ```mermaid code block. Paste it into Markdown or Typst as it is',
    copied: 'Copied',
    mermaidConfirm:
      'Elements that Mermaid cannot express will be changed before switching:\n\n{items}\n\nShapes are rearranged automatically. Undo (Ctrl+Z) brings back the diagram as it was.',
    mermaidShapes: '- {n} shape(s) not in Mermaid -> rectangles',
    mermaidLines: '- {n} line(s) that do not join two shapes -> removed',
    mermaidGroups: '- {n} container(s) / group(s) -> ungrouped (the shapes inside stay)',
    mermaidRotated: '- {n} rotated shape(s) -> rotation reset',
    mermaidSwitch: 'Switch',
    cancel: 'Cancel',
    noData:
      'This file has no diagram data to edit.\nIt is not a Typball diagram (.tbdg), or another program rewrote it.\nEditing is disabled to keep its contents.',
    unsupported:
      'This diagram uses a format that this version of Typball cannot edit (for example, it was made by a newer Typball).\nEditing is disabled to keep its contents.',
    broken: 'The diagram data could not be read.\nEditing is disabled to keep its contents.\n\n{error}',
  },
};
let strings = STRINGS.ja;
const t = (key) => strings[key] ?? STRINGS.en[key] ?? key;

// ---------------------------------------------------------------------------
// Rust との橋渡し
// ---------------------------------------------------------------------------

function post(message) {
  try {
    const host = window.typball_host;
    if (host && host.postMessage) host.postMessage(message);
  } catch (_) {
    // 送れなくてもページは動かし続ける
  }
}

// ---------------------------------------------------------------------------
// 図形 (maxGraph に無いフローチャートの図形) と、その外周
// ---------------------------------------------------------------------------

/** 多角形の図形: 幅と高さから頂点の列を返す関数 */
const POLYGONS = {
  parallelogram: (w, h) => {
    const d = Math.min(w * 0.2, h * 0.6);
    return [[d, 0], [w, 0], [w - d, h], [0, h]];
  },
  trapezoid: (w, h) => {
    const d = Math.min(w * 0.15, h * 0.5);
    return [[0, 0], [w, 0], [w - d, h], [d, h]];
  },
  manualInput: (w, h) => {
    const d = Math.min(h * 0.3, 20);
    return [[0, d], [w, 0], [w, h], [0, h]];
  },
  offpage: (w, h) => {
    const d = Math.min(h * 0.4, w * 0.5);
    return [[0, 0], [w, 0], [w, h - d], [w / 2, h], [0, h - d]];
  },
};

function registerShapes() {
  for (const [name, points] of Object.entries(POLYGONS)) {
    class PolygonShape extends mx.AbstractPathShape {
      redrawPath(c, _x, _y, w, h) {
        const pts = points(w, h);
        c.moveTo(pts[0][0], pts[0][1]);
        for (let i = 1; i < pts.length; i += 1) c.lineTo(pts[i][0], pts[i][1]);
        c.close();
      }
    }
    mx.ShapeRegistry.add(name, PolygonShape);
    mx.PerimeterRegistry.add(`${name}Perimeter`, polygonPerimeter(points));
  }

  // 書類: 下の辺が波形
  class DocumentShape extends mx.AbstractPathShape {
    redrawPath(c, _x, _y, w, h) {
      const dy = Math.min(h * 0.25, 16);
      c.moveTo(0, 0);
      c.lineTo(w, 0);
      c.lineTo(w, h - dy / 2);
      c.quadTo((w * 3) / 4, h - dy * 1.4, w / 2, h - dy / 2);
      c.quadTo(w / 4, h + dy * 0.4, 0, h - dy / 2);
      c.close();
    }
  }
  mx.ShapeRegistry.add('document', DocumentShape);

  // 定義済み処理: 四角形の左右に縦線
  class PredefinedShape extends mx.Shape {
    constructor(bounds, fill, stroke, strokeWidth = 1) {
      super();
      this.bounds = bounds;
      this.fill = fill;
      this.stroke = stroke;
      this.strokeWidth = strokeWidth;
    }
    paintVertexShape(c, x, y, w, h) {
      const inset = Math.min(w * 0.12, 12);
      c.translate(x, y);
      c.begin();
      c.moveTo(0, 0);
      c.lineTo(w, 0);
      c.lineTo(w, h);
      c.lineTo(0, h);
      c.close();
      c.fillAndStroke();
      c.setShadow(false);
      c.begin();
      c.moveTo(inset, 0);
      c.lineTo(inset, h);
      c.moveTo(w - inset, 0);
      c.lineTo(w - inset, h);
      c.stroke();
    }
  }
  mx.ShapeRegistry.add('predefined', PredefinedShape);

  // メモ: 右上の角が折れた四角形
  class NoteShape extends mx.Shape {
    constructor(bounds, fill, stroke, strokeWidth = 1) {
      super();
      this.bounds = bounds;
      this.fill = fill;
      this.stroke = stroke;
      this.strokeWidth = strokeWidth;
    }
    paintVertexShape(c, x, y, w, h) {
      const d = Math.min(w * 0.25, h * 0.4, 14);
      c.translate(x, y);
      c.begin();
      c.moveTo(0, 0);
      c.lineTo(w - d, 0);
      c.lineTo(w, d);
      c.lineTo(w, h);
      c.lineTo(0, h);
      c.close();
      c.fillAndStroke();
      c.setShadow(false);
      c.begin();
      c.moveTo(w - d, 0);
      c.lineTo(w - d, d);
      c.lineTo(w, d);
      c.stroke();
    }
  }
  mx.ShapeRegistry.add('note', NoteShape);

  // アイコン (ネットワーク図の機器など): スタイルの icon ("セット/種類") の絵を、枠に収まる大きさで描く。
  // 色を変えられるセットは、線の色 (strokeColor) がアイコンの色になる
  class IconShape extends mx.Shape {
    constructor(bounds, fill, stroke, strokeWidth = 1) {
      super();
      this.bounds = bounds;
      this.fill = fill;
      this.stroke = stroke;
      this.strokeWidth = strokeWidth;
      // 絵の透明な所でもつかめるように、枠全体でマウスを受ける
      this.shapePointerEvents = true;
    }
    paintVertexShape(c, x, y, w, h) {
      const icon = resolveIcon(this.style.icon, this.style.strokeColor);
      if (!icon) {
        // 絵がまだ読めていない / この Typball に無いアイコン: 場所だけ示す
        c.setStrokeColor('#999999');
        c.setDashed(true);
        c.begin();
        c.moveTo(x, y);
        c.lineTo(x + w, y);
        c.lineTo(x + w, y + h);
        c.lineTo(x, y + h);
        c.close();
        c.stroke();
        return;
      }
      const scale = Math.min(w / icon.w, h / icon.h);
      const fw = icon.w * scale;
      const fh = icon.h * scale;
      c.image(x + (w - fw) / 2, y + (h - fh) / 2, fw, fh, icon.uri, false, false, false);
    }
  }
  mx.ShapeRegistry.add('icon', IconShape);

  // RJ-45 のポート (ポート図): 四角形の下に、ツメの入る切り欠きを描く
  class Rj45Shape extends mx.AbstractPathShape {
    redrawPath(c, _x, _y, w, h) {
      const notch = Math.min(w * 0.4, 12);
      const depth = Math.min(h * 0.18, 5);
      if (this.style && this.style.portTab === 'up') {
        // 爪が上: 上下を逆にした形
        c.moveTo(0, depth);
        c.lineTo((w - notch) / 2, depth);
        c.lineTo((w - notch) / 2, 0);
        c.lineTo((w + notch) / 2, 0);
        c.lineTo((w + notch) / 2, depth);
        c.lineTo(w, depth);
        c.lineTo(w, h);
        c.lineTo(0, h);
        c.close();
        return;
      }
      c.moveTo(0, 0);
      c.lineTo(w, 0);
      c.lineTo(w, h - depth);
      c.lineTo((w + notch) / 2, h - depth);
      c.lineTo((w + notch) / 2, h);
      c.lineTo((w - notch) / 2, h);
      c.lineTo((w - notch) / 2, h - depth);
      c.lineTo(0, h - depth);
      c.close();
    }
  }
  mx.ShapeRegistry.add('rj45', Rj45Shape);
}

/** 凸多角形の外周: 中心 (直角の線なら相手と同じ x か y の位置) から相手へ向かう線と辺の交点 */
function polygonPerimeter(points) {
  return (bounds, _vertex, next, orthogonal = false) => {
    const pts = points(bounds.width, bounds.height).map(([x, y]) => [bounds.x + x, bounds.y + y]);
    let ox = bounds.getCenterX();
    let oy = bounds.getCenterY();
    if (orthogonal) {
      if (next.x >= bounds.x && next.x <= bounds.x + bounds.width) {
        ox = next.x;
      } else if (next.y >= bounds.y && next.y <= bounds.y + bounds.height) {
        oy = next.y;
      }
    }
    const dx = next.x - ox;
    const dy = next.y - oy;
    if (dx === 0 && dy === 0) return new mx.Point(ox, oy);
    let best = null;
    for (let i = 0; i < pts.length; i += 1) {
      const [x1, y1] = pts[i];
      const [x2, y2] = pts[(i + 1) % pts.length];
      const ex = x2 - x1;
      const ey = y2 - y1;
      const det = dx * ey - dy * ex;
      if (Math.abs(det) < 1e-9) continue;
      // 始点からの距離の割合 s (>0) と、辺の上の位置 u (0..1)
      const s = ((x1 - ox) * ey - (y1 - oy) * ex) / det;
      const u = ((x1 - ox) * dy - (y1 - oy) * dx) / det;
      if (s > 0 && u >= -1e-6 && u <= 1 + 1e-6 && (best === null || s < best)) best = s;
    }
    if (best === null) return mx.PerimeterRegistry.get('rectanglePerimeter')(bounds, _vertex, next, orthogonal);
    return new mx.Point(ox + dx * best, oy + dy * best);
  };
}

// ---------------------------------------------------------------------------
// スタイル
// ---------------------------------------------------------------------------

/** 図形の既定のスタイル。作るときに全部をセルに書き込む (あとで既定を変えても古い図の見た目が変わらないように) */
const VERTEX_STYLE = {
  fontFamily: FONT_STACK,
  fontSize: 12,
  fontColor: '#000000',
  fontStyle: 0,
  fillColor: '#ffffff',
  strokeColor: '#000000',
  strokeWidth: 1,
  align: 'center',
  verticalAlign: 'middle',
};

/** 線の既定のスタイル。ツールバーで線に当てた設定は、次に引く線にも使う */
const edgeDefaults = {
  fontFamily: FONT_STACK,
  fontSize: 11,
  fontColor: '#000000',
  fontStyle: 0,
  strokeColor: '#000000',
  strokeWidth: 1,
  edgeStyle: 'orthogonalEdgeStyle',
  rounded: false,
  curved: false,
  startArrow: 'none',
  endArrow: 'block',
  startFill: true,
  endFill: true,
  labelBackgroundColor: '#ffffff',
};
/** 次の線に引き継ぐスタイルのキー */
const EDGE_STICKY = ['strokeColor', 'strokeWidth', 'edgeStyle', 'rounded', 'curved', 'startArrow', 'endArrow', 'dashed', 'dashPattern', 'fontSize', 'fontColor'];

/** パレットの図形。icon は 32x26 の枠に描く SVG の中身 */
const SHAPE_ITEMS = [
  { group: 'shapes', id: 'rect', mermaid: true, w: 120, h: 60, style: {}, icon: '<rect x="3" y="5" width="26" height="16"/>' },
  { group: 'shapes', id: 'rounded', mermaid: true, w: 120, h: 60, style: { rounded: true }, icon: '<rect x="3" y="5" width="26" height="16" rx="4"/>' },
  { group: 'shapes', id: 'ellipse', mermaid: true, w: 120, h: 70, style: { shape: 'ellipse', perimeter: 'ellipsePerimeter' }, icon: '<ellipse cx="16" cy="13" rx="13" ry="8.5"/>' },
  { group: 'shapes', id: 'triangle', w: 80, h: 80, style: { shape: 'triangle', perimeter: 'trianglePerimeter', direction: 'north' }, icon: '<path d="M16 3 L29 23 L3 23 Z"/>' },
  { group: 'shapes', id: 'cloud', w: 130, h: 80, style: { shape: 'cloud', perimeter: 'ellipsePerimeter' }, icon: '<path d="M9 21 C3 21 2 14 7 13 C6 7 14 5 16 9 C19 4 27 7 25 12 C31 13 30 21 24 21 Z"/>' },
  { group: 'shapes', id: 'actor', w: 50, h: 80, style: { shape: 'actor', verticalAlign: 'top', verticalLabelPosition: 'bottom' }, icon: '<circle cx="16" cy="7" r="4"/><path d="M8 24 C8 15 11 13 16 13 C21 13 24 15 24 24 Z"/>' },
  { group: 'shapes', id: 'doubleEllipse', mermaid: true, w: 80, h: 80, style: { shape: 'doubleEllipse', perimeter: 'ellipsePerimeter' }, icon: '<ellipse cx="16" cy="13" rx="12" ry="10"/><ellipse cx="16" cy="13" rx="9" ry="7"/>' },
  { group: 'shapes', id: 'note', w: 120, h: 70, style: { shape: 'note', fillColor: '#fff9c4', align: 'left', verticalAlign: 'top', spacingLeft: 6, spacingTop: 4 }, icon: '<path d="M3 4 L23 4 L29 10 L29 22 L3 22 Z M23 4 L23 10 L29 10"/>' },
  { group: 'shapes', id: 'text', w: 100, h: 30, label: 'Text', style: { fillColor: 'none', strokeColor: 'none' }, icon: '<path d="M9 7 L23 7 M16 7 L16 21" fill="none" stroke-width="2"/>' },
  { group: 'shapes', id: 'container', w: 240, h: 180, style: { shape: 'swimlane', startSize: 24, fillColor: '#eeeeee', swimlaneFillColor: '#ffffff', fontStyle: 1 }, icon: '<rect x="3" y="4" width="26" height="18"/><path d="M3 9 L29 9"/>' },
  { group: 'flow', id: 'terminator', mermaid: true, w: 120, h: 40, style: { rounded: true, arcSize: 50 }, icon: '<rect x="3" y="7" width="26" height="12" rx="6"/>' },
  { group: 'flow', id: 'rect', w: 120, h: 60, style: {}, icon: '<rect x="3" y="5" width="26" height="16"/>' },
  { group: 'flow', id: 'rhombus', mermaid: true, w: 120, h: 80, style: { shape: 'rhombus', perimeter: 'rhombusPerimeter' }, icon: '<path d="M16 3 L29 13 L16 23 L3 13 Z"/>' },
  { group: 'flow', id: 'parallelogram', mermaid: true, w: 130, h: 60, style: { shape: 'parallelogram', perimeter: 'parallelogramPerimeter' }, icon: '<path d="M8 5 L29 5 L24 21 L3 21 Z"/>' },
  { group: 'flow', id: 'predefined', mermaid: true, w: 120, h: 60, style: { shape: 'predefined' }, icon: '<rect x="3" y="5" width="26" height="16"/><path d="M7 5 L7 21 M25 5 L25 21"/>' },
  { group: 'flow', id: 'document', w: 120, h: 70, style: { shape: 'document' }, icon: '<path d="M3 5 L29 5 L29 19 Q22.5 14 16 19 Q9.5 24 3 19 Z"/>' },
  { group: 'flow', id: 'cylinder', mermaid: true, w: 80, h: 80, style: { shape: 'cylinder' }, icon: '<path d="M6 7 C6 3 26 3 26 7 L26 19 C26 23 6 23 6 19 Z M6 7 C6 11 26 11 26 7"/>' },
  { group: 'flow', id: 'hexagon', mermaid: true, w: 130, h: 60, style: { shape: 'hexagon', perimeter: 'hexagonPerimeter' }, icon: '<path d="M9 5 L23 5 L29 13 L23 21 L9 21 L3 13 Z"/>' },
  { group: 'flow', id: 'manualInput', w: 120, h: 60, style: { shape: 'manualInput', perimeter: 'manualInputPerimeter' }, icon: '<path d="M3 10 L29 5 L29 21 L3 21 Z"/>' },
  { group: 'flow', id: 'trapezoid', mermaid: true, w: 120, h: 60, style: { shape: 'trapezoid', perimeter: 'trapezoidPerimeter' }, icon: '<path d="M3 5 L29 5 L24 21 L8 21 Z"/>' },
  { group: 'flow', id: 'offpage', w: 70, h: 70, style: { shape: 'offpage', perimeter: 'offpagePerimeter' }, icon: '<path d="M6 4 L26 4 L26 15 L16 23 L6 15 Z"/>' },
];

/** パレットの線 (図形につながっていない線を置く) */
const LINE_ITEMS = [
  { id: 'arrow', style: { endArrow: 'block' }, icon: '<path d="M4 20 L24 8" fill="none"/><path d="M28 6 L21 7 L25 12 Z" fill="currentColor"/>' },
  { id: 'line', style: { endArrow: 'none' }, icon: '<path d="M4 20 L28 6" fill="none"/>' },
  { id: 'arrowBoth', style: { startArrow: 'block', endArrow: 'block' }, icon: '<path d="M8 18 L24 8" fill="none"/><path d="M28 6 L21 7 L25 12 Z M4 20 L11 19 L7 14 Z" fill="currentColor"/>' },
  { id: 'arrowDashed', style: { dashed: true, endArrow: 'block' }, icon: '<path d="M4 20 L24 8" fill="none" stroke-dasharray="3 2"/><path d="M28 6 L21 7 L25 12 Z" fill="currentColor"/>' },
];

const COLORS = [
  '#000000', '#434343', '#666666', '#999999', '#b7b7b7', '#cccccc', '#d9d9d9', '#efefef', '#f3f3f3', '#ffffff',
  '#f8cecc', '#ffe6cc', '#fff2cc', '#d5e8d4', '#d0f0e8', '#dae8fc', '#e1d5e7', '#f5d5e5', '#fff9c4', '#eeeeee',
  '#ea9999', '#f9cb9c', '#ffe599', '#b6d7a8', '#a2c4c9', '#9fc5e8', '#b4a7d6', '#d5a6bd', '#e06666', '#f6b26b',
  '#cc0000', '#e69138', '#f1c232', '#6aa84f', '#45818e', '#3d85c6', '#674ea7', '#a64d79', '#b85450', '#d79b00',
  '#990000', '#b45f06', '#bf9000', '#38761d', '#134f5c', '#0b5394', '#351c75', '#741b47', '#82b366', '#6c8ebf',
];

// ---------------------------------------------------------------------------
// グラフ
// ---------------------------------------------------------------------------

const container = document.getElementById('graph');
const notice = document.getElementById('notice');
let graph = null;
let undoManager = null;
/** 読み込み中 (モデルの変更を Rust に知らせない) */
let loading = false;
/** 編集できない (読めないファイルを開いた) */
let readOnly = false;
/** 最後に Rust へ渡した (または Rust から受け取った) ファイルの中身 */
let lastText = '';
let exportTimer = 0;
let lastState = '';
/** このページでコピーした図形 (Rust が無いときの貼り付け用) と、続けて貼った回数 */
let localClip = '';
let pasteCount = 0;
let gridVisible = true;
/** 大きさが分かってから表示位置を決める (隠れたタブで読み込まれたとき) */
let pendingCenter = false;
/**
 * モードの一覧 (ツールバーに並ぶ順)。モードは「何を描く図か」で、パレット・ツールバー・下の欄・使える操作を決める。
 * 図のデータ (レイヤーのセルのスタイルの mode) に入っていて、ファイルに保存され、「元に戻す」でも一緒に戻る。
 * 書き出す SVG の <metadata> にも data-mode として書く (ファイルの中身を読まなくても種類が分かるように)。
 * モードを足すには: ここに名前を足し、MODE_LABELS に表示名、パレット (buildPalette)・ツールバー (buildToolbar)・
 * 下の欄 (PANELS) の分岐を足す。図の描き方そのものが maxGraph と違うもの (回路図など) は、別のページにする
 * (spec/tbdg-format.md の「形式とモード」)。
 */
const MODES = ['free', 'mermaid', 'network', 'ports'];
const MODE_LABELS = { free: 'modeFree', mermaid: 'modeMermaid', network: 'modeNetwork', ports: 'modePorts' };
/** 今のモード (図のデータから読んだ値。syncMode が更新する) と、モードごとの真偽 (よく使うので変数にしてある) */
let mode = 'free';
let mermaid = false;
let network = false;
let ports = false;
/** Mermaid モードの流れの向き */
let direction = 'TD';
/** 結線表・ポートの一覧の形式、パレットで選んでいる絵柄 */
let tableFormat = 'markdown';
let paletteSet = '';

function createGraph() {
  registerShapes();

  // 選択の枠とハンドルの見た目
  mx.VertexHandlerConfig.selectionColor = '#0b5cad';
  mx.VertexHandlerConfig.selectionDashed = false;
  mx.VertexHandlerConfig.rotationEnabled = true;
  mx.EdgeHandlerConfig.selectionColor = '#0b5cad';
  mx.EdgeHandlerConfig.selectionDashed = false;
  mx.EdgeHandlerConfig.virtualBendsEnabled = true;
  mx.HandleConfig.fillColor = '#ffffff';
  mx.HandleConfig.strokeColor = '#0b5cad';

  // 接続点の印 (maxGraph の既定は画像ファイル。同梱しないので SVG を埋め込む)
  const pointImage = new mx.ImageBox(
    'data:image/svg+xml,' +
      encodeURIComponent(
        `<svg xmlns="${SVG_NS}" width="9" height="9"><circle cx="4.5" cy="4.5" r="3.5" fill="#ffffff" stroke="#0b5cad" stroke-width="1.2"/></svg>`
      ),
    9,
    9
  );
  mx.ConstraintHandler.prototype.getImageForConstraint = () => pointImage;

  const g = new mx.Graph(container, undefined, [...mx.getDefaultPlugins(), mx.RubberBandHandler]);
  g.options.foldingEnabled = false;
  g.setHtmlLabels(false);
  g.setTooltips(false);
  g.setGridSize(GRID);
  g.setGridEnabled(true);
  g.setConnectable(true);
  g.setAllowDanglingEdges(true);
  g.setDisconnectOnMove(false);
  g.setMultigraph(true);
  g.setAllowLoops(true);
  g.setDropEnabled(true);
  g.setSplitEnabled(false);
  g.setCellsEditable(true);
  g.setPanning(true);
  g.setEnterStopsCellEditing(false);
  g.setInvokesStopCellEditing(true);

  // 既定のスタイル (セルにスタイルが無いときの保険。パレットから作るセルには全部書き込む)
  Object.assign(g.getStylesheet().getDefaultVertexStyle(), VERTEX_STYLE);
  Object.assign(g.getStylesheet().getDefaultEdgeStyle(), edgeDefaults);
  // 線の引き方 (edgeStyle) だけは既定に入れない: スタイルからキーを消すと「直線」になる
  delete g.getStylesheet().getDefaultEdgeStyle().edgeStyle;

  // 文字は SVG の <text> で出すので、折り返しは自前で改行を入れる (Typst は foreignObject を描けない)
  const baseGetLabel = g.getLabel.bind(g);
  g.getLabel = (cell) => {
    const label = baseGetLabel(cell);
    if (!label || !cell.isVertex()) return label;
    const geo = cell.getGeometry();
    if (!geo) return label;
    const style = g.getCurrentCellStyle(cell);
    // 線の端のポート名は大きさの無いセルなので、折り返さない
    if (style.portLabel || style.portKind) return label;
    const spacing = (style.spacing ?? 2) * 2 + (style.spacingLeft ?? 0) + (style.spacingRight ?? 0) + 4;
    // 図形の外に置くラベル (人の下の名前など) は折り返さない
    if (style.verticalLabelPosition === 'bottom' || style.verticalLabelPosition === 'top') return label;
    const room = geo.width * (TEXT_RATIO[style.shape] ?? 1) - spacing;
    return wrapText(String(label), Math.max(room, 10), style.fontSize ?? 12, style.fontStyle ?? 0);
  };

  // 接続点: 図形の各辺に 3 つずつ。図形の中からドラッグすると移動、接続点からドラッグすると線を引く。
  // 線の端を図形の中に落とすと、図形全体につながる (位置は外周の上を動く)
  const anchors = [
    [0.5, 0], [1, 0.5], [0.5, 1], [0, 0.5],
    [0.25, 0], [0.75, 0], [1, 0.25], [1, 0.75], [0.25, 1], [0.75, 1], [0, 0.25], [0, 0.75],
  ].map(([x, y]) => new mx.ConnectionConstraint(new mx.Point(x, y), true));
  // Mermaid モードでは接続点を使わない (線の出入りの位置は Mermaid が決める)。ネットワーク図モードも使わない:
  // 機器どうしをまっすぐ結ぶ線にしたいので、機器に重ねて出る印 (connectImage) から線を引く
  g.getAllConnectionConstraints = (terminal) =>
    mode === 'free' && terminal && terminal.cell && terminal.cell.isVertex() ? anchors : null;

  const connection = g.getPlugin('ConnectionHandler');
  // Mermaid モードでは図形を動かせないので、図形の中 (どこでも) からのドラッグで線を引く
  // ポート図モードでは線でつながない (ポートどうしの配線は、ネットワーク図か自由な図で描く)
  connection.isConnectableCell = () => !ports && (mermaid || network || g.isMouseDown);
  connection.marker.hotspotEnabled = false;
  connection.marker.validColor = '#0b5cad';
  connection.createEdgeState = () => {
    const edge = g.createEdge(null, null, '', null, null, newEdgeStyle());
    return new mx.CellState(g.view, edge, g.getCellStyle(edge));
  };
  // Mermaid モード: 何も無い所で離すと、元の図形と同じ形の新しい図形を作ってつなぐ
  connection.isCreateTarget = () => mermaid;
  const baseCreateTarget = connection.createTargetVertex.bind(connection);
  connection.createTargetVertex = (evt, source) => {
    const target = baseCreateTarget(evt, source);
    target.setValue('');
    return target;
  };
  // Mermaid モード: 元の図形の上で離したら、何もしない (図形を選ぶだけ)。図形の中からのドラッグで線を引くので、
  // クリックのつもりで少し動いただけでも「線を引いた」ことになる。そのままだと、自分への線か、新しい図形ができてしまう
  const baseConnect = connection.connect.bind(connection);
  connection.connect = (source, target, evt, dropTarget) => {
    if (mermaid && source && (target === source || (!target && dropTarget === source))) {
      g.setSelectionCell(source);
      return;
    }
    baseConnect(source, target, evt, dropTarget);
  };
  // つないだ後に選ぶもの: ふつうは引いた線。Mermaid モードで新しく作った図形は、続けて文字を入れられるように
  // 図形の方を選ぶ (`target` は新しく作ったときだけ渡される)
  connection.selectCells = (edge, target) => g.setSelectionCell(mermaid && target ? target : edge);
  connection.addListener(mx.InternalEvent.CONNECT, (_sender, evt) => {
    // 自動配置で動いた先を見えるようにする
    const target = evt.getProperty('terminal');
    if (mermaid && target) g.scrollCellToVisible(target);
  });

  // Mermaid モードの制限: 位置は自動なので動かせない。回転・線の折れ点・つながっていない線も無い
  const baseMovable = g.isCellMovable.bind(g);
  g.isCellMovable = (cell) => !mermaid && baseMovable(cell);
  const baseBendable = g.isCellBendable.bind(g);
  g.isCellBendable = (cell) => !mermaid && baseBendable(cell);
  const baseRotatable = g.isCellRotatable.bind(g);
  g.isCellRotatable = (cell) => !mermaid && baseRotatable(cell);
  g.isAllowDanglingEdges = () => !mermaid;
  // 自分自身につなぐ線も作らせない: 図形の中からのドラッグで線を引くので、クリックのつもりで少し動いただけで
  // 自分への線ができてしまう。しかも自動配置はその線を描かないので、見えないまま記述にだけ出る
  const baseAllowLoops = g.isAllowLoops.bind(g);
  g.isAllowLoops = () => !mermaid && baseAllowLoops();

  // Mermaid モードの自動配置。maxGraph の LayoutManager は、図を変える操作の最後 (同じ「元に戻す」の単位の中) で
  // 配置をやり直す。元に戻す / やり直しのときは走らない
  const layoutManager = new mx.LayoutManager(g);
  layoutManager.getLayout = (cell) => {
    const now = modeOf(g);
    return now.mode === 'mermaid' && cell === g.getDefaultParent() ? anchoredLayout(g, createLayout(g, now.direction), now.direction) : null;
  };

  const selection = g.getPlugin('SelectionHandler');
  selection.guidesEnabled = true;

  // 右ドラッグ・中ボタンのドラッグ・スペースを押しながらのドラッグで表示位置を動かす
  const panning = g.getPlugin('PanningHandler');
  const basePanningTrigger = panning.isPanningTrigger.bind(panning);
  panning.isPanningTrigger = (me) => {
    const evt = me.getEvent();
    return evt.button === 1 || spaceDown || basePanningTrigger(me);
  };

  // グループ化の入れ物は見えない枠にする
  g.createGroupCell = () => {
    const group = new mx.Cell('', new mx.Geometry(), { fillColor: 'none', strokeColor: 'none' });
    group.setVertex(true);
    group.setConnectable(false);
    return group;
  };

  undoManager = new mx.UndoManager(200);
  const onUndoable = (_sender, evt) => undoManager.undoableEditHappened(evt.getProperty('edit'));
  g.getDataModel().addListener(mx.InternalEvent.UNDO, onUndoable);
  g.getView().addListener(mx.InternalEvent.UNDO, onUndoable);

  g.getDataModel().addListener(mx.InternalEvent.CHANGE, () => {
    if (loading) return;
    // モードは図のデータに入っているので、元に戻す / やり直しでも変わる
    syncMode();
    updateCode();
    scheduleExport();
  });
  g.getSelectionModel().addListener(mx.InternalEvent.CHANGE, () => {
    updateToolbar();
    if (ports) {
      markSelectedPorts();
      updateSwitchBar();
    }
    postState();
  });
  for (const name of [mx.InternalEvent.SCALE, mx.InternalEvent.TRANSLATE, mx.InternalEvent.SCALE_AND_TRANSLATE]) {
    g.getView().addListener(name, updateViewport);
  }
  g.addListener(mx.InternalEvent.EDITING_STARTED, postState);
  g.addListener(mx.InternalEvent.EDITING_STOPPED, postState);
  return g;
}

// ---------------------------------------------------------------------------
// 文字の折り返し
// ---------------------------------------------------------------------------

/** 文字を入れられる幅の割合 (斜めの辺や曲線のある図形は、外接する四角形より狭い) */
const TEXT_RATIO = {
  ellipse: 0.72,
  doubleEllipse: 0.66,
  cloud: 0.7,
  rhombus: 0.56,
  triangle: 0.5,
  hexagon: 0.78,
  parallelogram: 0.74,
  trapezoid: 0.78,
  offpage: 0.9,
};

const measure = document.createElement('canvas').getContext('2d');
const wrapCache = new Map();

/** 和文は 1 文字ずつ、欧文は単語ごとに、幅 maxWidth に収まるよう改行を入れる */
function wrapText(text, maxWidth, fontSize, fontStyle) {
  const key = `${maxWidth}|${fontSize}|${fontStyle}|${text}`;
  const cached = wrapCache.get(key);
  if (cached !== undefined) return cached;
  measure.font = `${fontStyle & 2 ? 'italic ' : ''}${fontStyle & 1 ? 'bold ' : ''}${fontSize}px "${FONT}"`;
  const out = [];
  for (const para of text.split('\n')) {
    const tokens = para.match(/[　-鿿豈-﫿＀-￯]|[^\s　-鿿豈-﫿＀-￯]+|\s+/g) || [''];
    let line = '';
    for (const tok of tokens) {
      if (line && measure.measureText((line + tok).trimEnd()).width > maxWidth) {
        out.push(line.trimEnd());
        line = tok.trimStart();
      } else {
        line += tok;
      }
    }
    out.push(line.trimEnd());
  }
  const result = out.join('\n');
  if (wrapCache.size > 2000) wrapCache.clear();
  wrapCache.set(key, result);
  return result;
}

// ---------------------------------------------------------------------------
// 読み込みと書き出し
// ---------------------------------------------------------------------------

/** ファイルの中身 (SVG) から編集用のモデルの XML を取り出す。空のファイルは空の図 */
function extractModel(text) {
  if (!text.trim()) return null;
  const doc = new DOMParser().parseFromString(text, 'image/svg+xml');
  if (doc.getElementsByTagName('parsererror').length > 0) throw new Error('nodata');
  const meta = Array.from(doc.getElementsByTagName('metadata')).find((m) => m.getAttribute('id') === META_ID);
  if (!meta) throw new Error('nodata');
  // 編集用データの形式と版: この Typball が知らないもの (新しい版・別のエディタの形式) は、開かずに断る
  const format = meta.getAttribute('data-format');
  const version = Number(meta.getAttribute('data-version') ?? '1');
  if ((format && format !== META_FORMAT) || version > META_VERSION) throw new Error('unsupported');
  // 印だけあって中身が空なのは、新しい図 (Rust の diagram::EMPTY)
  return meta.textContent.trim() ? meta.textContent : null;
}

function load(text, options) {
  setOptions(options);
  loading = true;
  try {
    graph.stopEditing(true);
    const model = graph.getDataModel();
    let xml = null;
    let failure = null;
    try {
      xml = extractModel(text);
    } catch (e) {
      failure = t(e && e.message === 'unsupported' ? 'unsupported' : 'noData');
    }
    graph.clearSelection();
    model.clear();
    rememberEmbeddedIcons(text);
    if (xml) {
      try {
        new mx.ModelXmlSerializer(model).import(xml);
      } catch (e) {
        model.clear();
        failure = t('broken').replace('{error}', String((e && e.message) || e));
      }
    }
    undoManager.clear();
    lastText = text;
    readOnly = failure !== null;
    graph.setEnabled(!readOnly);
    notice.hidden = !readOnly;
    notice.textContent = failure ?? '';
    document.getElementById('app').classList.toggle('readonly', readOnly);
  } finally {
    loading = false;
  }
  syncMode(true);
  updateCode();
  centerView(true);
  updateToolbar();
  lastState = '';
  postState();
  // 図が使っているアイコンを読み込んでから描き直す (読めるまでは場所だけの枠が出る)
  ensureIcons(usedIcons()).then((loaded) => {
    if (loaded) graph.refresh();
  });
}

/**
 * 今の図をファイルの中身 (SVG) にする。
 *
 * 画面に出ている図形は、今の拡大率と表示位置で作られている (座標の丸めや文字の大きさの測り方が拡大率で
 * 変わる)。そのまま書き出すと同じ図でも拡大率によって中身が少し変わるので、書き出しのときだけ
 * 拡大率 1 の図形を、原点のそばに別に作り直して描く (maxGraph の TemporaryCellStates。画面の図形には触らない)。
 * 同じ図からは必ず同じ文字列ができるので、元に戻して保存時の図になれば「変更なし」に戻せる
 */
function buildSvg() {
  const view = graph.view;
  const model = graph.getDataModel();
  const doc = document.implementation.createDocument(SVG_NS, 'svg', null);
  const root = doc.documentElement;

  // 編集用のデータ。Typst は中身から SVG と判定するので、<svg xmlns=...> を必ず先頭にしてその後ろに置く
  const meta = doc.createElementNS(SVG_NS, 'metadata');
  meta.setAttribute('id', META_ID);
  meta.setAttribute('data-format', META_FORMAT);
  meta.setAttribute('data-version', String(META_VERSION));
  meta.setAttribute('data-mode', modeOf().mode);
  meta.textContent = new mx.ModelXmlSerializer(model).export({ pretty: false });
  root.appendChild(meta);
  const group = doc.createElementNS(SVG_NS, 'g');
  root.appendChild(group);

  let width = 100;
  let height = 100;
  if (graph.getChildCells(graph.getDefaultParent(), true, true).length > 0) {
    // 作り直した図形は、画面には出ない仮の置き場に描かせる (文字の大きさを測るには文書の中に要る)
    const near = modelOrigin();
    const eventsEnabled = view.isEventsEnabled();
    view.setEventsEnabled(false);
    const { drawPane, overlayPane, translate } = view;
    view.drawPane = document.createElementNS(SVG_NS, 'g');
    view.overlayPane = document.createElementNS(SVG_NS, 'g');
    view.drawPane.style.visibility = 'hidden';
    view.overlayPane.style.visibility = 'hidden';
    view.canvas.appendChild(view.drawPane);
    view.canvas.appendChild(view.overlayPane);
    // 図のある場所を原点のそばへ寄せて描く (整数だけずらすので、できる SVG は図がどこにあっても同じ)。
    // 原点から遠い座標 (約 3,355 万 px = 2^25 より先) のまま描くと、ブラウザが文字の大きさを測るときに座標を
    // 丸めてしまい、図形が SVG の範囲の外へ出る
    view.translate = near ? new mx.Point(-Math.round(near.x), -Math.round(near.y)) : new mx.Point(0, 0);
    // (寄せる先は、画面の図形ではなくモデルの座標から決める: 画面の図形が遠くにあると、その範囲も丸められている)
    // 図形を作り直すと、maxGraph は選択中のセルのハンドル (選択枠) も新しい図形に付け替えようとする。
    // 仮の図形に付け替えられると画面の選択枠が消えるので、その間だけハンドルの一覧を隠しておく
    const selectionHandlers = graph.getPlugin('SelectionCellsHandler');
    const handlers = selectionHandlers.handlers;
    selectionHandlers.handlers = new Map();
    let temp = null;
    try {
      temp = new mx.TemporaryCellStates(view, 1, [model.getRoot()]);
      const bounds = view.getGraphBounds();
      if (bounds.width > 0 && bounds.height > 0) {
        // 左上を整数にそろえて、線がにじまないようにする
        const x0 = Math.floor(bounds.x - PAD);
        const y0 = Math.floor(bounds.y - PAD);
        width = Math.ceil(bounds.x + bounds.width + PAD) - x0;
        height = Math.ceil(bounds.y + bounds.height + PAD) - y0;
        const canvas = new mx.SvgCanvas2D(group, false);
        canvas.translate(-x0, -y0);
        new mx.ImageExport().drawState(view.getState(model.getRoot()), canvas);
        shareIcons(doc, root, group);
      }
    } finally {
      if (temp) temp.destroy();
      selectionHandlers.handlers = handlers;
      view.translate = translate;
      view.drawPane.remove();
      view.overlayPane.remove();
      view.drawPane = drawPane;
      view.overlayPane = overlayPane;
      view.setEventsEnabled(eventsEnabled);
    }
  }
  root.setAttribute('width', String(width));
  root.setAttribute('height', String(height));
  root.setAttribute('viewBox', `0 0 ${width} ${height}`);
  return `${new XMLSerializer().serializeToString(root)}\n`;
}

/** 前回の書き出しにかかった時間 (ms)。大きな図ほど、続けて編集している間は書き出しを待つ */
let lastExportMs = 0;

/** 図が変わった: 少し待ってから書き出す (続けて起きた変更は 1 回にまとめる) */
function scheduleExport() {
  if (exportTimer) clearTimeout(exportTimer);
  const delay = Math.min(400, Math.max(30, lastExportMs * 3));
  exportTimer = setTimeout(() => {
    exportTimer = 0;
    flushExport();
  }, delay);
}

function flushExport() {
  if (readOnly) return;
  // まだ読めていないアイコンがあれば、読めてから書き出す (場所だけの枠を書き出さないように)
  const missing = usedIcons().filter((id) => icons.files.has(id) && !icons.text.has(id));
  if (missing.length > 0) {
    ensureIcons(missing).then(() => {
      graph.refresh();
      scheduleExport();
    });
    return;
  }
  let text;
  try {
    const started = performance.now();
    text = buildSvg();
    lastExportMs = performance.now() - started;
  } catch (e) {
    post(`error:${(e && e.stack) || e}`);
    return;
  }
  if (text !== lastText) {
    lastText = text;
    post(`svg:${text}`);
  }
  updateToolbar();
  postState();
}

function postState() {
  if (!graph) return;
  syncImeCatcher();
  const state = JSON.stringify({
    undo: !readOnly && undoManager.canUndo(),
    redo: !readOnly && undoManager.canRedo(),
    sel: readOnly ? 0 : graph.getSelectionCount(),
    cells: graph.getChildCells(graph.getDefaultParent(), true, true).length,
    editing: graph.isEditing(),
    readonly: readOnly,
  });
  if (state !== lastState) {
    lastState = state;
    post(`state:${state}`);
  }
}

function setOptions(options) {
  if (!options) return;
  if (options.theme) document.documentElement.dataset.theme = options.theme === 'dark' ? 'dark' : 'light';
  if (options.lang) {
    const next = STRINGS[String(options.lang).slice(0, 2)] ?? STRINGS.en;
    if (next !== strings) {
      strings = next;
      buildChrome();
    }
  }
}

// ---------------------------------------------------------------------------
// 表示位置・拡大率・グリッド
// ---------------------------------------------------------------------------

/** 図の範囲 (拡大率 1 の座標)。空なら null */
function contentBounds() {
  const b = graph.getGraphBounds();
  if (!(b.width > 0 && b.height > 0)) return null;
  const s = graph.view.scale;
  const tr = graph.view.translate;
  return { x: b.x / s - tr.x, y: b.y / s - tr.y, width: b.width / s, height: b.height / s };
}

/** これより遠く (画面の px) に図があるときは、範囲を測る前に表示位置を図のそばへ移す */
const FAR_AWAY = 1e6;

/**
 * 図のだいたいの場所: いちばん上の階層の図形と線の、左上の座標。モデルの数値だけで決めるので、原点からどれだけ
 * 遠くても正しい (画面の図形の範囲は、画面の座標が約 3,355 万 px = 2^25 を超えるとブラウザに丸められる)。空なら null
 */
function modelOrigin() {
  let x = Infinity;
  let y = Infinity;
  const take = (p) => {
    if (p && Number.isFinite(p.x) && Number.isFinite(p.y)) {
      x = Math.min(x, p.x);
      y = Math.min(y, p.y);
    }
  };
  for (const cell of graph.getChildCells(graph.getDefaultParent(), true, true)) {
    const geometry = cell.getGeometry();
    if (!geometry) continue;
    if (cell.isVertex()) take(geometry);
    else for (const p of [geometry.sourcePoint, geometry.targetPoint, ...(geometry.points ?? [])]) take(p);
  }
  return Number.isFinite(x) && Number.isFinite(y) ? { x, y } : null;
}

/** 図を画面の中央に置く。`shrink` なら収まらないときだけ縮める (拡大はしない) */
function centerView(shrink, fit = false) {
  const cw = container.clientWidth;
  const ch = container.clientHeight;
  if (cw === 0 || ch === 0) {
    pendingCenter = true;
    return;
  }
  pendingCenter = false;
  // 図が今の表示位置からとても遠いと、下で測る範囲が正しくない。先に図のそばへ移してから測る
  const origin = modelOrigin();
  if (origin) {
    const { scale, translate } = graph.view;
    if (Math.abs((origin.x + translate.x) * scale) > FAR_AWAY || Math.abs((origin.y + translate.y) * scale) > FAR_AWAY) {
      graph.view.setTranslate(-origin.x, -origin.y);
    }
  }
  const b = contentBounds();
  if (!b) {
    graph.view.scaleAndTranslate(1, 0, 0);
    return;
  }
  let s = graph.view.scale;
  if (shrink || fit) {
    const room = Math.min((cw - 60) / b.width, (ch - 60) / b.height);
    s = fit ? room : Math.min(1, room);
    s = Math.min(Math.max(s, MIN_SCALE), fit ? 4 : 1);
  }
  graph.view.scaleAndTranslate(s, (cw / s - b.width) / 2 - b.x, (ch / s - b.height) / 2 - b.y);
}

/** 画面上の点 (px, py) を動かさずに拡大率を変える */
function zoomAt(scale, px, py) {
  const view = graph.view;
  const s0 = view.scale;
  const s1 = Math.min(Math.max(scale, MIN_SCALE), MAX_SCALE);
  if (s1 === s0) return;
  const tr = view.translate;
  view.scaleAndTranslate(s1, px / s1 - (px / s0 - tr.x), py / s1 - (py / s0 - tr.y));
}

function zoomBy(factor) {
  zoomAt(graph.view.scale * factor, container.clientWidth / 2, container.clientHeight / 2);
}

/** 拡大率の表示とグリッドの背景を今の表示位置に合わせる */
function updateViewport() {
  syncImeCatcher();
  const s = graph.view.scale;
  const tr = graph.view.translate;
  const label = document.getElementById('zoom-label');
  if (label) label.textContent = `${Math.round(s * 100)}%`;
  const minor = GRID * s;
  if (!gridVisible || minor < 4) {
    container.style.backgroundImage = 'none';
    return;
  }
  const major = minor * 5;
  const line = (color, dir) => `linear-gradient(to ${dir}, ${color} 1px, transparent 1px)`;
  container.style.backgroundImage = [line('#dcdcdc', 'right'), line('#dcdcdc', 'bottom'), line('#f0f0f0', 'right'), line('#f0f0f0', 'bottom')].join(',');
  container.style.backgroundSize = `${major}px ${major}px, ${major}px ${major}px, ${minor}px ${minor}px, ${minor}px ${minor}px`;
  // 模様は major ごとの繰り返しなので、余りだけずらす (遠くまで動かすと、大きな値はブラウザが丸めて格子がずれる)
  const offset = (v) => (((v * s) % major) + major) % major;
  container.style.backgroundPosition = `${offset(tr.x)}px ${offset(tr.y)}px`;
}

// ---------------------------------------------------------------------------
// 図形の追加
// ---------------------------------------------------------------------------

function viewCenter() {
  const s = graph.view.scale;
  const tr = graph.view.translate;
  return { x: container.clientWidth / 2 / s - tr.x, y: container.clientHeight / 2 / s - tr.y };
}

const snap = (v) => Math.round(v / GRID) * GRID;

/**
 * 画面の中央に続けて置いた図形が重なって隠れないよう、中心が既にある図形の中心と (ほぼ) 同じなら
 * 右下へ少しずつずらす。(x, y) は置く図形の左上、(w, h) は大きさ
 */
function freeSpot(x, y, w, h) {
  const centers = graph.getChildCells(graph.getDefaultParent(), true, false).flatMap((c) => {
    const g = c.getGeometry();
    return g ? [[g.x + g.width / 2, g.y + g.height / 2]] : [];
  });
  const taken = () => centers.some(([cx, cy]) => Math.abs(cx - (x + w / 2)) < GRID && Math.abs(cy - (y + h / 2)) < GRID);
  for (let i = 0; i < 50 && taken(); i += 1) {
    x += GRID * 2;
    y += GRID * 2;
  }
  return [x, y];
}

/** 図形を置く。(cx, cy) は中心 (省略すると画面の中央) */
function insertShape(item, cx, cy) {
  if (readOnly) return;
  const centered = cx === undefined;
  const c = centered ? viewCenter() : { x: cx, y: cy };
  let x = snap(c.x - item.w / 2);
  let y = snap(c.y - item.h / 2);
  if (centered) [x, y] = freeSpot(x, y, item.w, item.h);
  let cell = null;
  graph.batchUpdate(() => {
    cell = graph.insertVertex({
      parent: graph.getDefaultParent(),
      value: item.label ?? '',
      position: [x, y],
      size: [item.w, item.h],
      style: { ...VERTEX_STYLE, ...item.style },
    });
  });
  graph.setSelectionCell(cell);
  // Mermaid モードでは自動配置が置き場所を決めるので、置かれた所を見えるようにする
  if (mermaid) graph.scrollCellToVisible(cell);
  container.focus();
}

/** どこにもつながっていない線を置く */
function insertLine(item, cx, cy) {
  if (readOnly) return;
  const c = cx === undefined ? viewCenter() : { x: cx, y: cy };
  const edge = new mx.Cell('', new mx.Geometry(), { ...newEdgeStyle(), ...item.style });
  edge.setEdge(true);
  edge.geometry.relative = true;
  edge.geometry.setTerminalPoint(new mx.Point(snap(c.x - 60), snap(c.y)), true);
  edge.geometry.setTerminalPoint(new mx.Point(snap(c.x + 60), snap(c.y)), false);
  graph.batchUpdate(() => {
    graph.addCell(edge, graph.getDefaultParent());
  });
  graph.setSelectionCell(edge);
  container.focus();
}

// ---------------------------------------------------------------------------
// 編集の操作
// ---------------------------------------------------------------------------

const selected = () => graph.getSelectionCells();
const selectedEdges = () => selected().filter((c) => c.isEdge());
const selectedVertices = () => selected().filter((c) => c.isVertex());

/** 選んだセルを XML にする (貼り付け用。つながっている線の両端も一緒に選んであれば接続を保つ) */
function serializeCells(cells) {
  const top = mx.cellArrayUtils.getTopmostCells(graph.getExportableCells(cells));
  if (top.length === 0) return '';
  const clones = graph.cloneCells(top, true, {}, true);
  const temp = new mx.GraphDataModel();
  const parent = temp.getRoot().getChildAt(0);
  temp.beginUpdate();
  try {
    clones.forEach((c, i) => {
      // 親 (グループや枠) の中の座標を、図全体の座標に直す
      const state = graph.view.getState(top[i]);
      const geo = c.getGeometry();
      if (state && geo && c.isVertex() && top[i].getParent() !== graph.getDefaultParent()) {
        const s = graph.view.scale;
        const tr = graph.view.translate;
        geo.x = state.x / s - tr.x;
        geo.y = state.y / s - tr.y;
      }
      temp.add(parent, c);
    });
  } finally {
    temp.endUpdate();
  }
  return new mx.ModelXmlSerializer(temp).export({ pretty: false });
}

function copySelection() {
  const xml = serializeCells(selected());
  if (!xml) return false;
  localClip = xml;
  pasteCount = 0;
  post(`clip:${xml}`);
  return true;
}

function pasteXml(xml) {
  if (readOnly || !xml) return;
  const temp = new mx.GraphDataModel();
  try {
    new mx.ModelXmlSerializer(temp).import(xml);
  } catch (_) {
    return;
  }
  const layer = temp.getRoot().getChildAt(0);
  const cells = layer ? layer.getChildren().slice() : [];
  if (cells.length === 0) return;
  pasteCount += 1;
  const d = GRID * 2 * pasteCount;
  // ほかの図からコピーしたアイコンは、まだ読み込んでいないことがある。読めてから貼る
  ensureIcons(usedIcons(temp.getRoot())).then(() => {
    let pasted = [];
    graph.batchUpdate(() => {
      pasted = graph.importCells(cells, d, d, graph.getDefaultParent());
      // 自由な図からコピーしたものを Mermaid モードの図に貼った: 表せない要素を直す
      if (mermaid) normalizeForMermaid();
    });
    graph.setSelectionCells(pasted.filter((c) => graph.getDataModel().contains(c)));
    container.focus();
  });
}

function deleteSelection() {
  if (readOnly) return;
  const cells = graph.getDeletableCells(selected());
  // Mermaid モードでは、消した図形につながる線も一緒に消す (線は必ず図形と図形をつなぐ。残すと、相手のいない線が
  // キャンバスの原点へ向かって伸びる)
  if (cells.length > 0) graph.removeCells(cells, mermaid);
}

function duplicateSelection() {
  const xml = serializeCells(selected());
  if (!xml) return;
  const saved = pasteCount;
  pasteCount = 0;
  pasteXml(xml);
  pasteCount = saved;
}

function groupSelection() {
  const cells = selected();
  if (mermaid || cells.length < 2) return;
  graph.setSelectionCell(graph.groupCells(null, 0, cells));
}

function ungroupSelection() {
  const cells = selected().filter((c) => c.isVertex() && c.getChildCount() > 0);
  if (cells.length > 0) graph.setSelectionCells(graph.ungroupCells(cells));
}

/** グリッドが有効なら、グリッドに合わせた値。並べ直す処理 (整列・等間隔・自動配置) は、どれもこれを通す */
const gridSnap = (v) => (graph.isGridEnabled() ? snap(v) : v);

/**
 * 並べ直す対象: 選んだ図形の、位置 (レイヤーの座標) と大きさ。親も選んである図形は入れない (親と一緒に動く)。
 * 親の中の図形の座標は親からの相対なので、親の左上の位置を足して、同じ座標で比べられるようにする
 */
function selectionBoxes() {
  const chosen = new Set(selectedVertices());
  const boxes = [];
  for (const cell of chosen) {
    const geometry = cell.getGeometry();
    if (!geometry || geometry.relative || !graph.isCellMovable(cell)) continue;
    let inside = false;
    let ox = 0;
    let oy = 0;
    for (let p = cell.getParent(); p && p !== graph.getDefaultParent(); p = p.getParent()) {
      if (chosen.has(p)) inside = true;
      const pg = p.getGeometry();
      if (pg && !pg.relative) {
        ox += pg.x;
        oy += pg.y;
      }
    }
    if (!inside) boxes.push({ cell, geometry, x: ox + geometry.x, y: oy + geometry.y, w: geometry.width, h: geometry.height });
  }
  return boxes;
}

/** `boxes` の図形を、それぞれ (nx, ny) へ動かす。`place(box)` が新しい左上 (レイヤーの座標) を返す */
function moveBoxes(boxes, place) {
  const model = graph.getDataModel();
  graph.batchUpdate(() => {
    for (const b of boxes) {
      const [nx, ny] = place(b);
      if (nx === b.x && ny === b.y) continue;
      const moved = b.geometry.clone();
      moved.x += nx - b.x;
      moved.y += ny - b.y;
      model.setGeometry(b.cell, moved);
    }
  });
}

/**
 * 選んだ図形をそろえる。`how` は left / center / right / top / middle / bottom。
 * 基準は、選んだ図形全体の範囲 (左端・右端・左右の中央…)。選んだ順番には左右されない。
 * maxGraph の alignCells は使わない: 中央ぞろえの基準が選んだ順番で変わり (最初の図形の中央と、ほかの図形の
 * x 座標の小さい方になる)、画面の座標から計算する。ここではモデルの座標だけで計算する。
 * グリッドが有効なら、動かした先は必ずグリッドに乗せる (手で動かした図形と同じ位置に来るように)。そのため、
 * 幅が違う図形の中央ぞろえや、大きさがグリッドの倍数でない図形の右ぞろえは、最大でグリッドの半分だけ、
 * ぴったりの位置からずれる
 */
function alignSelection(how) {
  if (readOnly) return;
  const boxes = selectionBoxes();
  if (boxes.length < 2) return;
  const left = Math.min(...boxes.map((b) => b.x));
  const right = Math.max(...boxes.map((b) => b.x + b.w));
  const top = Math.min(...boxes.map((b) => b.y));
  const bottom = Math.max(...boxes.map((b) => b.y + b.h));
  moveBoxes(boxes, (b) => {
    if (how === 'left') return [gridSnap(left), b.y];
    if (how === 'right') return [gridSnap(right - b.w), b.y];
    if (how === 'center') return [gridSnap((left + right) / 2 - b.w / 2), b.y];
    if (how === 'top') return [b.x, gridSnap(top)];
    if (how === 'bottom') return [b.x, gridSnap(bottom - b.h)];
    return [b.x, gridSnap((top + bottom) / 2 - b.h / 2)];
  });
}

/**
 * 選んだ図形 (3 つ以上) を、左右か上下に等間隔で並べる (図形と図形の隙間を同じにする)。並び順は今の位置の順。
 * グリッドが有効なら、隙間をグリッドの倍数に丸め、いちばん手前 (左か上) の図形から順に置いていく。隙間がそろい、
 * どの図形もグリッドに乗るかわりに、いちばん奥の図形は少し動くことがある。
 * グリッドが無効なら、両端の図形を動かさずに、間の図形を均等に置く
 */
function distributeSelection(horizontal) {
  if (readOnly) return;
  const boxes = selectionBoxes();
  if (boxes.length < 3) return;
  const start = (b) => (horizontal ? b.x : b.y);
  const size = (b) => (horizontal ? b.w : b.h);
  boxes.sort((a, b) => start(a) + size(a) / 2 - (start(b) + size(b) / 2));
  const from = Math.min(...boxes.map(start));
  const to = Math.max(...boxes.map((b) => start(b) + size(b)));
  const total = boxes.reduce((sum, b) => sum + size(b), 0);
  const gap = gridSnap((to - from - total) / (boxes.length - 1));
  const target = new Map();
  let cursor = gridSnap(from);
  for (const b of boxes) {
    const at = gridSnap(cursor);
    target.set(b, at);
    cursor = at + size(b) + gap;
  }
  moveBoxes(boxes, (b) => (horizontal ? [target.get(b), b.y] : [b.x, target.get(b)]));
}

function nudge(dx, dy) {
  const cells = graph.getMovableCells(selected());
  if (cells.length > 0) graph.moveCells(cells, dx, dy);
}

function undo() {
  if (readOnly) return;
  graph.stopEditing(false);
  undoManager.undo();
}

function redo() {
  if (readOnly) return;
  graph.stopEditing(false);
  undoManager.redo();
}

/** 入力中の文字を確定し、待たずに書き出して Rust に渡す */
function commitEditing() {
  if (graph.isEditing()) graph.stopEditing(false);
  // 書き出しは、図が変わって予約されているときだけ (変わっていないのに書き出すと、別の版で作られた
  // ファイルを開いただけで「変更あり」になってしまう)
  if (exportTimer) {
    clearTimeout(exportTimer);
    exportTimer = 0;
    flushExport();
  }
  postState();
}

/** Rust (編集メニュー・ツールバー) からの操作 */
function command(name, arg) {
  if (!graph) return;
  if (name === 'commit') {
    // 入力中の文字を確定して、その結果をすぐ Rust に渡す (保存やタブを閉じる前に呼ばれる)。
    // 確定した図 ("svg:") を渡し終えてから知らせるので、Rust はこの知らせの後なら最新の中身を持っている
    commitEditing();
    post('committed');
    return;
  }
  if (graph.isEditing()) {
    // ラベルの編集中は、入力欄に対する操作にする
    if (name === 'paste') return;
    const map = { undo: 'undo', redo: 'redo', cut: 'cut', copy: 'copy', delete: 'delete', selectAll: 'selectAll' };
    if (map[name]) document.execCommand(map[name]);
    return;
  }
  switch (name) {
    case 'undo': undo(); break;
    case 'redo': redo(); break;
    case 'copy': copySelection(); break;
    case 'cut': if (copySelection()) deleteSelection(); break;
    case 'paste': pasteXml(arg ?? localClip); break;
    case 'delete': deleteSelection(); break;
    case 'selectAll': if (!readOnly) graph.selectAll(); break;
    default: break;
  }
}

// ---------------------------------------------------------------------------
// スタイルの変更 (ツールバー)
// ---------------------------------------------------------------------------

function applyStyle(key, value, cells = selected()) {
  if (readOnly || cells.length === 0) return;
  graph.setCellStyles(key, value, cells);
  if (EDGE_STICKY.includes(key) && cells.some((c) => c.isEdge())) edgeDefaults[key] = value;
}

function applyStyles(pairs, cells) {
  if (readOnly || cells.length === 0) return;
  graph.batchUpdate(() => {
    for (const [key, value] of Object.entries(pairs)) applyStyle(key, value, cells);
  });
}

function toggleFontFlag(flag) {
  const cells = selected();
  if (readOnly || cells.length === 0) return;
  const on = ((graph.getCurrentCellStyle(cells[0]).fontStyle ?? 0) & flag) !== 0;
  graph.setCellStyleFlags('fontStyle', flag, !on, cells);
}

const ROUTES = {
  straight: { edgeStyle: null, curved: false },
  orthogonal: { edgeStyle: 'orthogonalEdgeStyle', curved: false },
  curved: { edgeStyle: 'orthogonalEdgeStyle', curved: true },
};

function routeOf(style) {
  if (!style.edgeStyle) return 'straight';
  return style.curved ? 'curved' : 'orthogonal';
}

const DASHES = {
  solid: { dashed: false, dashPattern: null },
  dashed: { dashed: true, dashPattern: '6 4' },
  dotted: { dashed: true, dashPattern: '1 3' },
};

function dashOf(style) {
  if (!style.dashed) return 'solid';
  return style.dashPattern === '1 3' ? 'dotted' : 'dashed';
}

/** 矢印の形: スタイルの (種類, 塗り) の組 */
const ARROWS = {
  none: ['none', true],
  block: ['block', true],
  open: ['open', true],
  oval: ['oval', true],
  diamond: ['diamond', true],
};

// ---------------------------------------------------------------------------
// アイコン (ネットワーク図の機器など)
// ---------------------------------------------------------------------------
//
// アイコンは Typball に同梱の SVG (assets/diagram/icons/<セット>/<種類>.svg)。一覧 (files.json) と表示名
// (catalog.json) をページの起動時に読み、絵の中身は使うときに読む。セルのスタイルには名前だけを書く:
//   { shape: 'icon', icon: 'affinity-circle/router', strokeColor: '#2d67b9' }
// ファイル (SVG) には、使っているアイコンの絵を <defs> に 1 回ずつ入れ、置いた場所からは <use> で参照する。
// 同じ機器を何十個置いてもファイルは太らず、アイコンを収録していない Typball で開いても、<defs> の絵から
// そのまま描ける (rememberEmbeddedIcons)。

const XLINK_NS = 'http://www.w3.org/1999/xlink';

const icons = {
  /** セット (catalog.json の sets に、そのセットにある種類の並び kinds を足したもの) */
  sets: new Map(),
  /** 種類の表示名 */
  kinds: new Map(),
  /** 同梱しているアイコンの名前 ("セット/種類") */
  files: new Set(),
  /** 読み込んだ絵 (名前 → SVG の文字列) */
  text: new Map(),
  /** 描くための絵 ("名前|色" → { uri, w, h, id, tint }) と、その逆引き (data URI → 同じもの) */
  resolved: new Map(),
  byUri: new Map(),
  /** 開いたファイルに入っていた絵 ("名前|色" → 同じ形)。同梱していないアイコンを描くのに使う */
  embedded: new Map(),
};

async function loadIconCatalog() {
  try {
    const [catalog, files] = await Promise.all(
      ['catalog.json', 'files.json'].map((name) => fetch(`/assets/diagram/icons/${name}`).then((r) => r.json()))
    );
    for (const name of files) icons.files.add(name);
    const order = new Map();
    (catalog.kinds ?? []).forEach((kind, i) => {
      icons.kinds.set(kind.id, kind);
      order.set(kind.id, i);
    });
    for (const set of catalog.sets ?? []) {
      // 並びは catalog.json の順。書かれていない種類は、名前の順で末尾に
      const kinds = files
        .filter((name) => name.startsWith(`${set.id}/`))
        .map((name) => name.slice(set.id.length + 1))
        .sort((a, b) => (order.get(a) ?? 1e6) - (order.get(b) ?? 1e6) || a.localeCompare(b));
      if (kinds.length > 0) icons.sets.set(set.id, { ...set, kinds });
    }
  } catch (_) {
    // アイコンが読めなくても、図エディタは動かす (ネットワーク図の機器が並ばないだけ)
  }
}

const lang = () => (strings === STRINGS.ja ? 'ja' : 'en');
const setLabel = (set) => set[lang()] ?? set.en ?? set.id;
const kindLabel = (kind) => {
  const entry = icons.kinds.get(kind);
  return (entry && (entry[lang()] ?? entry.en)) ?? kind;
};

/** 図 (か、渡したセル以下) が使っているアイコンの名前 */
function usedIcons(root = graph.getDataModel().getRoot()) {
  const ids = new Set();
  for (const cell of root.getDescendants()) {
    const icon = (cell.getStyle() ?? {}).icon;
    if (icon) ids.add(String(icon));
  }
  return Array.from(ids);
}

/** アイコンの絵を読み込む。新しく読んだものがあれば true */
async function ensureIcons(ids) {
  const missing = ids.filter((id) => icons.files.has(id) && !icons.text.has(id));
  await Promise.all(
    missing.map((id) =>
      fetch(`/assets/diagram/icons/${id}.svg`)
        .then((r) => (r.ok ? r.text() : Promise.reject(new Error(String(r.status)))))
        .then((text) => icons.text.set(id, text))
        .catch(() => {})
    )
  );
  return missing.length > 0;
}

/** SVG の元の大きさ (viewBox、無ければ width / height) */
function svgSize(text) {
  const box = /viewBox="\s*[-\d.]+[\s,]+[-\d.]+[\s,]+([\d.]+)[\s,]+([\d.]+)\s*"/.exec(text);
  if (box) return [Number(box[1]) || 100, Number(box[2]) || 100];
  const w = /<svg[^>]*\swidth="([\d.]+)/.exec(text);
  const h = /<svg[^>]*\sheight="([\d.]+)/.exec(text);
  return [w ? Number(w[1]) : 100, h ? Number(h[1]) : 100];
}

function toDataUri(svg) {
  const bytes = new TextEncoder().encode(svg);
  let binary = '';
  for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return `data:image/svg+xml;base64,${btoa(binary)}`;
}

/** アイコンを描くための絵にする。`color` はセルの線の色 (色を変えられるセットなら、アイコンの色になる) */
function resolveIcon(id, color) {
  if (!id) return null;
  const set = icons.sets.get(String(id).split('/')[0]);
  const wanted = typeof color === 'string' ? color.toLowerCase() : '';
  const tint = set && set.tint && wanted && wanted !== 'none' && wanted !== String(set.color ?? '').toLowerCase() ? wanted : '';
  const key = `${id}|${tint}`;
  const hit = icons.resolved.get(key);
  if (hit) return hit;
  const text = icons.text.get(id);
  // 同梱していない (まだ読めていない) アイコン: 開いたファイルに入っていた絵があれば使う
  if (text === undefined) return icons.embedded.get(key) ?? icons.embedded.get(`${id}|`) ?? null;
  let svg = text;
  if (tint) for (const literal of set.tint) svg = svg.split(literal).join(tint);
  const [w, h] = svgSize(text);
  const made = { uri: toDataUri(svg), w, h, id: String(id), tint };
  icons.resolved.set(key, made);
  icons.byUri.set(made.uri, made);
  return made;
}

/** 開くファイル (SVG) の <defs> に入っているアイコンの絵を覚える */
function rememberEmbeddedIcons(text) {
  icons.embedded.clear();
  if (!text.includes('data-icon')) return;
  const doc = new DOMParser().parseFromString(text, 'image/svg+xml');
  for (const image of Array.from(doc.getElementsByTagName('image'))) {
    const id = image.getAttribute('data-icon');
    const uri = image.getAttributeNS(XLINK_NS, 'href') || image.getAttribute('href');
    if (!id || !uri) continue;
    const tint = image.getAttribute('data-tint') ?? '';
    const made = { uri, w: Number(image.getAttribute('width')) || 100, h: Number(image.getAttribute('height')) || 100, id, tint };
    icons.embedded.set(`${id}|${tint}`, made);
    icons.byUri.set(uri, made);
  }
}

/**
 * 書き出した SVG の中のアイコンをまとめる: 同じ絵は <defs> に 1 回だけ置き、置いた場所は <use> にする
 * (Typst も <use> で参照した <image> を描ける。PDF でもベクターのまま)
 */
function shareIcons(doc, root, group) {
  const defs = doc.createElementNS(SVG_NS, 'defs');
  const ids = new Map();
  const num = (v) => String(Number(v.toFixed(5)));
  for (const image of Array.from(group.getElementsByTagName('image'))) {
    const uri = image.getAttributeNS(XLINK_NS, 'href');
    const icon = uri ? icons.byUri.get(uri) : null;
    if (!icon) continue;
    let id = ids.get(uri);
    if (!id) {
      id = `icon${ids.size + 1}`;
      ids.set(uri, id);
      const def = doc.createElementNS(SVG_NS, 'image');
      def.setAttribute('id', id);
      def.setAttribute('width', String(icon.w));
      def.setAttribute('height', String(icon.h));
      def.setAttribute('data-icon', icon.id);
      if (icon.tint) def.setAttribute('data-tint', icon.tint);
      def.setAttributeNS(XLINK_NS, 'xlink:href', uri);
      defs.appendChild(def);
      // 絵の表示範囲の外に図形が入っているアイコンがある (isoflow)。ブラウザは範囲で切り取るが、Typst は
      // 埋め込んだ画像を切り取らないので、範囲の外の図形が点になって出る。切り取り枠を付けておく
      const clip = doc.createElementNS(SVG_NS, 'clipPath');
      clip.setAttribute('id', `${id}c`);
      const box = doc.createElementNS(SVG_NS, 'rect');
      box.setAttribute('width', String(icon.w));
      box.setAttribute('height', String(icon.h));
      clip.appendChild(box);
      defs.appendChild(clip);
    }
    const use = doc.createElementNS(SVG_NS, 'use');
    use.setAttributeNS(XLINK_NS, 'xlink:href', `#${id}`);
    use.setAttribute('clip-path', `url(#${id}c)`);
    const x = Number(image.getAttribute('x'));
    const y = Number(image.getAttribute('y'));
    const sx = Number(image.getAttribute('width')) / icon.w;
    const sy = Number(image.getAttribute('height')) / icon.h;
    use.setAttribute('transform', `${image.getAttribute('transform') ?? ''}translate(${num(x)},${num(y)}) scale(${num(sx)},${num(sy)})`);
    if (image.hasAttribute('opacity')) use.setAttribute('opacity', image.getAttribute('opacity'));
    image.replaceWith(use);
  }
  if (ids.size > 0) {
    root.setAttributeNS('http://www.w3.org/2000/xmlns/', 'xmlns:xlink', XLINK_NS);
    root.insertBefore(defs, group);
  }
}

/** アイコンを置く。(cx, cy) は中心 (省略すると画面の中央) */
async function insertIcon(id, cx, cy) {
  if (readOnly) return;
  await ensureIcons([id]);
  const set = icons.sets.get(id.split('/')[0]);
  const icon = resolveIcon(id, '');
  if (!set || !icon) return;
  // 枠は正方形で、一辺はグリッド 2 つぶん (20) の倍数にする (catalog.json の size): 中心がグリッドに乗るので、ほかの図形と
  // 中心をそろえられ、横に並べた機器の名前 (枠の下に出る) の高さもそろう。絵は枠の中に、縦横比を保って収まる
  const w = set.size ?? 60;
  const h = w;
  const centered = cx === undefined;
  const c = centered ? viewCenter() : { x: cx, y: cy };
  let x = snap(c.x - w / 2);
  let y = snap(c.y - h / 2);
  if (centered) [x, y] = freeSpot(x, y, w, h);
  const style = {
    ...VERTEX_STYLE,
    shape: 'icon',
    icon: id,
    fontSize: 11,
    fillColor: 'none',
    // 色を変えられるセットは、線の色がアイコンの色になる (ツールバーの「線の色」で変える)
    strokeColor: set.tint ? set.color : 'none',
    // 機器の名前は絵の下に出す
    verticalLabelPosition: 'bottom',
    verticalAlign: 'top',
  };
  if (set.perimeter === 'ellipse') style.perimeter = 'ellipsePerimeter';
  let cell = null;
  graph.batchUpdate(() => {
    cell = graph.insertVertex({ parent: graph.getDefaultParent(), value: '', position: [x, y], size: [w, h], style });
  });
  graph.setSelectionCell(cell);
  container.focus();
}

// ---------------------------------------------------------------------------
// ネットワーク図モード
// ---------------------------------------------------------------------------
//
// 機器のアイコンを置いて線で結ぶ図。配置は自由 (自由な図と同じ操作ができる)。違いは:
// - パレットに機器のアイコンが並ぶ (絵柄を選べる)
// - 線は機器どうしをまっすぐ結び、矢印は付かない。機器に重ねて出る印からドラッグして引く
// - 線の両端にポート名を付けられる (線の子のセル。機器を動かしても線の端に付いてくる)
// - 下の欄に結線表 (機器 / ポート / 接続先 / ポート / メモ) が出て、Markdown・Typst・CSV でコピーできる

let connectImageBox = null;

/** 機器に重ねて出る「ここから線を引く」印 */
function connectImage() {
  if (!connectImageBox) {
    const svg =
      `<svg xmlns="${SVG_NS}" width="18" height="18"><circle cx="9" cy="9" r="8" fill="#0b5cad" fill-opacity="0.9" stroke="#ffffff"/>` +
      '<path d="M5 9 H12 M9.5 6 L12.5 9 L9.5 12" fill="none" stroke="#ffffff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>';
    connectImageBox = new mx.ImageBox(`data:image/svg+xml,${encodeURIComponent(svg)}`, 18, 18);
  }
  return connectImageBox;
}

/** ポート名のセルのスタイル */
const PORT_STYLE = {
  fontFamily: FONT_STACK,
  fontSize: 9,
  fontColor: '#000000',
  fontStyle: 0,
  fillColor: 'none',
  strokeColor: 'none',
  labelBackgroundColor: '#ffffff',
  align: 'center',
  verticalAlign: 'middle',
  resizable: false,
  rotatable: false,
};

/** 線の端のポート名のセル (無ければ null)。`source` が true なら始点側 */
function portCell(edge, source) {
  const which = source ? 'source' : 'target';
  return (edge.getChildren() ?? []).find((c) => (c.getStyle() ?? {}).portLabel === which) ?? null;
}

/** 線の端のポート名を付ける / 変える / (空なら) 消す */
function setPort(edge, source, text) {
  if (readOnly) return;
  const value = String(text ?? '').trim();
  const cell = portCell(edge, source);
  if ((cell ? String(cell.getValue() ?? '') : '') === value) return;
  graph.batchUpdate(() => {
    if (cell && !value) {
      graph.removeCells([cell], false);
    } else if (cell) {
      graph.getDataModel().setValue(cell, value);
    } else {
      // 線に沿った位置 (-1 = 始点、1 = 終点) に置く。大きさは持たず、文字だけが出る
      const geometry = new mx.Geometry(source ? -0.6 : 0.6, 0, 0, 0);
      geometry.relative = true;
      const label = new mx.Cell(value, geometry, { ...PORT_STYLE, portLabel: source ? 'source' : 'target' });
      label.setVertex(true);
      label.setConnectable(false);
      graph.addCell(label, edge);
    }
  });
}

/** 機器の名前: ラベルの 1 行目 (IP アドレスなどは 2 行目以降に書く想定) */
function deviceName(cell) {
  const first = String(cell.getValue() ?? '')
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line !== '');
  return first ?? t('unnamed');
}

/** 結線表の行: [機器, ポート, 接続先, ポート, メモ]。機器の名前・ポートの順に並べる */
function connectionRows() {
  const rows = [];
  for (const edge of graph.getDataModel().getRoot().getDescendants()) {
    if (!edge.isEdge()) continue;
    const a = edge.getTerminal(true);
    const b = edge.getTerminal(false);
    if (!a || !b || !a.isVertex() || !b.isVertex()) continue;
    const port = (source) => String((portCell(edge, source) || { getValue: () => '' }).getValue() ?? '').trim();
    const note = String(edge.getValue() ?? '')
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line !== '')
      .join(' ');
    rows.push([deviceName(a), port(true), deviceName(b), port(false), note]);
  }
  const compare = (x, y) => x.localeCompare(y, undefined, { numeric: true });
  return rows.sort((x, y) => compare(x[0], y[0]) || compare(x[1], y[1]) || compare(x[2], y[2]) || compare(x[3], y[3]));
}

/** 結線表を、文書に貼れる形の文字列にする */
function connectionTable(format) {
  return tableText(format, [t('thDevice'), t('thPort'), t('thPeer'), t('thPort'), t('thNote')], connectionRows());
}

// ---------------------------------------------------------------------------
// ポート図モード
// ---------------------------------------------------------------------------
//
// スイッチ 1 台ごとの、ポートの並びと割り当てを書くための図。スイッチは「枠」(swimlane) で、中にポートのセルが
// 並ぶ。ポートのセルは portKind ('rj45' / 'sfp') の印を持ち、ラベルがポート番号、接続先・VLAN・速度・メモは
// portInfo (JSON の文字列) に入る。下の欄は、その一覧を書き込める表にしたもの。表は Markdown・Typst・CSV で
// コピーでき、図 (.tbdg) と一緒に文書に貼る。線でつなぐ操作は無い。

/**
 * スイッチのひな形: ポート (RJ-45) の数、段の数、右側の SFP の数。パレットでは、ポートの数ごとの見出しの下に並ぶ。
 * ここに足せばパレットに出る。置いた後は、下の欄 (スイッチの設定) でポートの数・段・並び順を変えられる
 */
const SWITCH_ITEMS = [
  { ports: 5, rows: 1, uplinks: 0 },
  { ports: 8, rows: 1, uplinks: 0 },
  { ports: 8, rows: 2, uplinks: 0 },
  { ports: 8, rows: 1, uplinks: 2 },
  { ports: 8, rows: 2, uplinks: 2 },
  { ports: 12, rows: 1, uplinks: 0 },
  { ports: 12, rows: 2, uplinks: 0 },
  { ports: 12, rows: 2, uplinks: 2 },
  { ports: 16, rows: 1, uplinks: 0 },
  { ports: 16, rows: 2, uplinks: 0 },
  { ports: 16, rows: 2, uplinks: 2 },
  { ports: 16, rows: 2, uplinks: 4 },
  { ports: 24, rows: 1, uplinks: 0 },
  { ports: 24, rows: 2, uplinks: 0 },
  { ports: 24, rows: 2, uplinks: 2 },
  { ports: 24, rows: 2, uplinks: 4 },
  { ports: 48, rows: 2, uplinks: 0 },
  { ports: 48, rows: 2, uplinks: 2 },
  { ports: 48, rows: 2, uplinks: 4 },
];

/**
 * 2 段のときの番号の並び順 (スイッチの枠のスタイル portOrder に入る値と、文言のキー)。
 * - zigzag: 上下に交互。上の段が 1, 3, 5…、下の段が 2, 4, 6…
 * - zigzag-up: 上下に交互。下の段が 1, 3, 5…
 * - rows: 段ごと。上の段を左から右へ数えきってから、下の段
 * - rows-up: 段ごと。下の段から
 */
const PORT_ORDERS = [
  ['zigzag', 'orderZigzag'],
  ['zigzag-up', 'orderZigzagUp'],
  ['rows', 'orderRows'],
  ['rows-up', 'orderRowsUp'],
];
const PORT_ORDER_KEY = 'typball.diagram.portOrder';
const isPortOrder = (value) => PORT_ORDERS.some(([id]) => id === value);

/** 新しく置くスイッチの並び順 (パレットで選ぶ。次に開いたときのために覚えておく) */
let newSwitchOrder = 'zigzag';
try {
  const saved = window.localStorage.getItem(PORT_ORDER_KEY);
  if (isPortOrder(saved)) newSwitchOrder = saved;
} catch (_) {
  /* 覚えられない環境では既定のまま */
}

// 寸法はどれもグリッド (10) の倍数にする: ひな形のポートと、手で足して動かしたポート (グリッドに吸着する) が
// 同じ位置にそろうように
const PORT_W = 30;
const PORT_H = 20;
const PORT_GAP = 10;
/** 何列かごとに空ける隙間、ポートの列と SFP の間、箱の内側の余白、名前の帯の高さ、箱の幅の下限 */
const PORT_BLOCK_GAP = 10;
const PORT_UPLINK_GAP = 20;
const SWITCH_PAD = 10;
const SWITCH_TITLE = 30;
const SWITCH_MIN_W = 120;
/** 設定欄で指定できる数の上限 */
const SWITCH_MAX_PORTS = 96;
const SWITCH_MAX_SFP = 16;

const PORT_STYLE_BASE = { ...VERTEX_STYLE, fontSize: 9, fillColor: '#ffffff', strokeColor: '#333333' };
/** RJ-45 の番号を、爪の無い側へ寄せる量 (爪の深さぶん。番号が、爪を除いた四角の中央に来る) */
const PORT_LABEL_SHIFT = 4;
/** パレットのポート単体 (スイッチに足すとき用) */
const PORT_ITEMS = [
  { id: 'rj45', w: PORT_W, h: PORT_H, label: '1', style: { ...PORT_STYLE_BASE, shape: 'rj45', portKind: 'rj45', portInfo: '{}', spacingBottom: PORT_LABEL_SHIFT }, icon: '<path d="M6 5 L26 5 L26 18 L20 18 L20 21 L12 21 L12 18 L6 18 Z"/>' },
  { id: 'sfp', w: PORT_W, h: PORT_H, label: '1', style: { ...PORT_STYLE_BASE, rounded: true, portKind: 'sfp', portInfo: '{}' }, icon: '<rect x="5" y="7" width="22" height="12" rx="2"/>' },
];

/** `count` 個を `rows` 段に並べたときの列の数 */
function portCols(count, rows) {
  return Math.ceil(count / rows);
}

/** 隙間を空ける列の数: 6 列か 4 列ごと (割り切れるほう。どちらでも割り切れなければ隙間なし) */
function portGroup(cols) {
  if (cols % 6 === 0) return 6;
  return cols % 4 === 0 ? 4 : 0;
}

/** 列 `col` (0 始まり) の左端の x */
function portColumnX(col, group) {
  return SWITCH_PAD + col * (PORT_W + PORT_GAP) + (group ? Math.floor(col / group) * PORT_BLOCK_GAP : 0);
}

/** `count` 個のうち `i` 番目 (0 始まり) のポートが入る列と段 */
function portSlot(i, count, rows, order) {
  if (rows < 2) return { col: i, row: 0 };
  const cols = Math.ceil(count / 2);
  const up = order.endsWith('-up');
  if (order.startsWith('rows')) {
    const first = i < cols;
    return { col: first ? i : i - cols, row: first === up ? 1 : 0 };
  }
  return { col: Math.floor(i / 2), row: (i % 2 === 0) === up ? 1 : 0 };
}

/** スイッチの箱の大きさと、並べるための寸法。`spec` は { ports, uplinks, rows, order } */
function switchMetrics(spec) {
  const rows = spec.rows === 1 ? 1 : 2;
  const cols = portCols(spec.ports, rows);
  const upCols = portCols(spec.uplinks, rows);
  const group = portGroup(cols);
  const portsRight = cols > 0 ? portColumnX(cols - 1, group) + PORT_W : 0;
  const upX = cols > 0 ? portsRight + PORT_UPLINK_GAP : SWITCH_PAD;
  const right = upCols > 0 ? upX + upCols * (PORT_W + PORT_GAP) - PORT_GAP : portsRight;
  return {
    w: Math.max(right + SWITCH_PAD, SWITCH_MIN_W),
    h: SWITCH_TITLE + SWITCH_PAD * 2 + rows * PORT_H + (rows - 1) * PORT_GAP,
    rows,
    cols,
    upCols,
    group,
    upX,
  };
}

/** パレットの絵: 箱の中に、ひな形と同じ並びの小さなポート (塗りが RJ-45、枠だけが SFP) */
function switchIcon(item) {
  const rows = item.rows === 1 ? 1 : 2;
  const cols = portCols(item.ports, rows);
  const upCols = portCols(item.uplinks, rows);
  const group = portGroup(cols);
  const gaps = group ? Math.ceil(cols / group) - 1 : 0;
  // 幅 72 に収まるように 1 列の幅を決める (隙間は 0.6 列ぶん、SFP は 1.4 列ぶんで、前を 1 列ぶん空ける)
  const units = cols + gaps * 0.6 + (upCols ? upCols * 1.4 + 1 : 0);
  const unit = Math.min(72 / units, 6);
  const side = Math.min(unit * 0.72, 4.2);
  const ys = rows === 2 ? [11 - side - 0.7, 11 + 0.7] : [11 - side / 2];
  const n = (v) => Math.round(v * 100) / 100;
  let body = '<rect x="1" y="2" width="78" height="18" rx="1.5"/>';
  for (let col = 0; col < cols; col += 1) {
    const x = 4 + (col + (group ? Math.floor(col / group) * 0.6 : 0)) * unit;
    for (let row = 0; row < rows; row += 1) {
      if (col * rows + row >= item.ports) break;
      body += `<rect x="${n(x)}" y="${n(ys[row])}" width="${n(side)}" height="${n(side)}" fill="currentColor" stroke="none"/>`;
    }
  }
  for (let col = 0; col < upCols; col += 1) {
    const x = 4 + (cols + gaps * 0.6 + 1 + col * 1.4) * unit;
    for (let row = 0; row < rows; row += 1) {
      if (col * rows + row >= item.uplinks) break;
      body += `<rect x="${n(x)}" y="${n(ys[row])}" width="${n(unit * 1.1)}" height="${n(side)}" fill="none" stroke-width="0.8"/>`;
    }
  }
  return `<svg viewBox="0 0 80 22" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round">${body}</svg>`;
}

/** パレットに出す名前: 「8 ポート (2 段) + SFP 2」 */
function switchTitle(item) {
  const base = t(item.rows === 1 ? 'swRow1' : 'swRow2').replace('{n}', item.ports);
  return item.uplinks ? base + t('swSfp').replace('{u}', item.uplinks) : base;
}

/** スイッチの中のポート (種類ごと。並びは作った順 = ひな形の番号の順) */
function switchPorts(chassis) {
  const kids = chassis.getChildren() ?? [];
  const of = (kind) => kids.filter((c) => c.isVertex() && (c.getStyle() ?? {}).portKind === kind);
  return { rj45: of('rj45'), sfp: of('sfp') };
}

/** スイッチ (ポートの並ぶ枠) か */
function isSwitch(cell) {
  if (!cell || !cell.isVertex()) return false;
  const style = cell.getStyle() ?? {};
  if (style.shape !== 'swimlane') return false;
  if (style.portOrder) return true;
  const found = switchPorts(cell);
  return found.rj45.length + found.sfp.length > 0;
}

/**
 * 番号とポート名の振り方の既定。スイッチの枠のスタイル portNaming (JSON の文字列) に、既定と違うものだけ入る。
 * - start / sfpStart: 外観の番号 (ポートのラベル) の最初の数。sfpStart が null なら、RJ-45 の続きの番号
 * - name / nameStart: 設定で使うポート名 (「Gi1/0/1」) の、番号の前の部分と最初の数。name が空なら名前を振らない
 * - sfpName / sfpNameStart: SFP のポート名。sfpName が空なら name を使い、sfpNameStart が null なら続きの番号
 * 外観の番号とポート名は、始まりの数も、SFP で振り直すかどうかも機種ごとに違うので、別々に持つ
 */
const NAMING_DEFAULTS = { start: 1, sfpStart: null, name: '', nameStart: 1, sfpName: '', sfpNameStart: null };
const NAMING_MAX = 9999;

/** スイッチの、番号とポート名の振り方 */
function switchNaming(chassis) {
  let raw = {};
  try {
    const value = JSON.parse(String((chassis.getStyle() ?? {}).portNaming ?? '{}'));
    if (value && typeof value === 'object') raw = value;
  } catch (_) {
    /* 壊れていたら既定 */
  }
  const number = (value, fallback) => (Number.isInteger(value) && value >= 0 && value <= NAMING_MAX ? value : fallback);
  const text = (value) => (typeof value === 'string' ? value : '');
  return {
    start: number(raw.start, NAMING_DEFAULTS.start),
    sfpStart: number(raw.sfpStart, null),
    name: text(raw.name),
    nameStart: number(raw.nameStart, NAMING_DEFAULTS.nameStart),
    sfpName: text(raw.sfpName),
    sfpNameStart: number(raw.sfpNameStart, null),
  };
}

/** 振り方をスタイルに入れる文字列にする (既定と同じ項目は書かない。全部既定なら null) */
function namingJson(spec) {
  const out = {};
  for (const key of Object.keys(NAMING_DEFAULTS)) {
    if (spec[key] !== NAMING_DEFAULTS[key] && spec[key] !== undefined) out[key] = spec[key];
  }
  return Object.keys(out).length > 0 ? JSON.stringify(out) : null;
}

/** `kind` の `i` 番目 (0 始まり) のポートの、外観の番号 */
function portNumber(spec, kind, i) {
  if (kind === 'sfp') return String((spec.sfpStart ?? spec.start + spec.ports) + i);
  return String(spec.start + i);
}

/** `kind` の `i` 番目 (0 始まり) のポートの、設定で使う名前 (振らないなら空) */
function portName(spec, kind, i) {
  if (kind === 'sfp') {
    const prefix = spec.sfpName || spec.name;
    return prefix ? `${prefix}${(spec.sfpNameStart ?? spec.nameStart + spec.ports) + i}` : '';
  }
  return spec.name ? `${spec.name}${spec.nameStart + i}` : '';
}

/**
 * 爪 (RJ-45 の切り欠き) の向きの決め方 (スイッチの枠のスタイル portTabs に入る値と、文言のキー)。既定は down。
 * 2 段のスイッチは、上の段の爪が上・下の段の爪が下 (out) のものが多い。ポートごとの向きは、ポートのスタイルの
 * portTab ('up'。無ければ下)
 */
const PORT_TABS = [
  ['down', 'tabsDown'],
  ['up', 'tabsUp'],
  ['out', 'tabsOut'],
  ['in', 'tabsIn'],
];
const isPortTabs = (value) => PORT_TABS.some(([id]) => id === value);

/** `spec.tabs` の決め方で、`row` 段目 (0 = 上の段) のポートの爪が上か */
function tabUp(spec, row) {
  const two = spec.rows !== 1;
  if (spec.tabs === 'up') return true;
  if (spec.tabs === 'out') return two && row === 0;
  if (spec.tabs === 'in') return two && row === 1;
  return false;
}

const isTabUp = (cell) => (cell.getStyle() ?? {}).portTab === 'up';

/** ポートのスタイルを、爪が上 / 下のものにする (番号は、爪の無い側へ寄せる) */
function tabStyle(style, up) {
  const next = { ...(style ?? {}) };
  delete next.portTab;
  delete next.spacingTop;
  delete next.spacingBottom;
  if (up) {
    next.portTab = 'up';
    next.spacingTop = PORT_LABEL_SHIFT;
  } else {
    next.spacingBottom = PORT_LABEL_SHIFT;
  }
  return next;
}

/** 選んだ RJ-45 のポートの、爪の向きを逆にする */
function flipPortTabs() {
  if (readOnly) return;
  const cells = selectedVertices().filter((c) => (c.getStyle() ?? {}).portKind === 'rj45');
  if (cells.length === 0) return;
  const model = graph.getDataModel();
  graph.batchUpdate(() => {
    for (const cell of cells) model.setStyle(cell, tabStyle(cell.getStyle(), !isTabUp(cell)));
  });
}

/** スイッチの今の設定 { ports, uplinks, rows, order, tabs, と番号・ポート名の振り方 } */
function switchSpec(chassis) {
  const style = chassis.getStyle() ?? {};
  const { rj45, sfp } = switchPorts(chassis);
  let rows = Number(style.portRows);
  if (rows !== 1 && rows !== 2) {
    // 設定を持たない (前の版で作った) スイッチ: ポートの高さが 1 通りなら 1 段
    const ys = new Set([...rj45, ...sfp].map((c) => Math.round(c.getGeometry()?.y ?? 0)));
    rows = ys.size === 1 ? 1 : 2;
  }
  return {
    ports: rj45.length,
    uplinks: sfp.length,
    rows,
    order: isPortOrder(style.portOrder) ? style.portOrder : 'zigzag',
    tabs: isPortTabs(style.portTabs) ? style.portTabs : 'down',
    ...switchNaming(chassis),
  };
}

/**
 * スイッチを `spec` (switchSpec と同じ形) のとおりに作り直す: ポートを足し引きして並べ直し、番号とポート名を振る。
 * ポートは「何番目か」で場所が決まるので、並び順を変えると、番号も接続先も色も一緒に新しい場所へ動く。
 * 番号とポート名は、前の振り方のとおりのまま (書き換えていない) ものだけ振り直す。書き換えたものはそのまま
 */
function applySwitch(chassis, spec) {
  const model = graph.getDataModel();
  const m = switchMetrics(spec);
  graph.batchUpdate(() => {
    const old = switchSpec(chassis);
    const now = switchPorts(chassis);
    const each = (fn) => {
      now.rj45.forEach((cell, i) => fn(cell, 'rj45', i));
      now.sfp.forEach((cell, i) => fn(cell, 'sfp', i));
    };
    const autoNumber = new Set();
    const autoName = new Set();
    const autoTab = new Set();
    each((cell, kind, i) => {
      if (String(cell.getValue() ?? '') === portNumber(old, kind, i)) autoNumber.add(cell);
      if (String(portInfo(cell).name ?? '') === portName(old, kind, i)) autoName.add(cell);
      if (kind === 'rj45' && isTabUp(cell) === tabUp(old, portSlot(i, old.ports, old.rows, old.order).row)) autoTab.add(cell);
    });
    const fit = (cells, count, item) => {
      if (cells.length > count) graph.removeCells(cells.splice(count), false);
      while (cells.length < count) {
        const cell = graph.insertVertex({ parent: chassis, value: '', position: [0, 0], size: [PORT_W, PORT_H], style: item.style });
        autoNumber.add(cell);
        autoName.add(cell);
        autoTab.add(cell);
        cells.push(cell);
      }
    };
    fit(now.rj45, spec.ports, PORT_ITEMS[0]);
    fit(now.sfp, spec.uplinks, PORT_ITEMS[1]);

    const top = SWITCH_TITLE + SWITCH_PAD;
    each((cell, kind, i) => {
      const sfp = kind === 'sfp';
      const slot = portSlot(i, sfp ? spec.uplinks : spec.ports, m.rows, spec.order);
      const x = sfp ? m.upX + slot.col * (PORT_W + PORT_GAP) : portColumnX(slot.col, m.group);
      const y = top + slot.row * (PORT_H + PORT_GAP);
      const before = cell.getGeometry() ?? new mx.Geometry();
      if (before.x !== x || before.y !== y || before.width !== PORT_W || before.height !== PORT_H) {
        const geometry = before.clone();
        geometry.x = x;
        geometry.y = y;
        geometry.width = PORT_W;
        geometry.height = PORT_H;
        model.setGeometry(cell, geometry);
      }
      if (autoNumber.has(cell)) {
        const number = portNumber(spec, kind, i);
        if (String(cell.getValue() ?? '') !== number) model.setValue(cell, number);
      }
      if (!sfp && autoTab.has(cell) && isTabUp(cell) !== tabUp(spec, slot.row)) {
        model.setStyle(cell, tabStyle(cell.getStyle(), tabUp(spec, slot.row)));
      }
      if (autoName.has(cell)) {
        const name = portName(spec, kind, i);
        const info = portInfo(cell);
        if (String(info.name ?? '') !== name) {
          if (name) info.name = name;
          else delete info.name;
          model.setStyle(cell, { ...(cell.getStyle() ?? {}), portInfo: JSON.stringify(info) });
        }
      }
    });

    const box = chassis.getGeometry();
    if (box.width !== m.w || box.height !== m.h) {
      const geometry = box.clone();
      geometry.width = m.w;
      geometry.height = m.h;
      model.setGeometry(chassis, geometry);
    }
    const style = chassis.getStyle() ?? {};
    const naming = namingJson(spec);
    const tabs = isPortTabs(spec.tabs) && spec.tabs !== 'down' ? spec.tabs : null;
    if (style.portRows !== m.rows || style.portOrder !== spec.order || (style.portNaming ?? null) !== naming || (style.portTabs ?? null) !== tabs) {
      const next = { ...style, portRows: m.rows, portOrder: spec.order };
      if (naming) next.portNaming = naming;
      else delete next.portNaming;
      if (tabs) next.portTabs = tabs;
      else delete next.portTabs;
      model.setStyle(chassis, next);
    }
  });
}

/** スイッチを置く: 名前の帯の付いた箱と、ひな形のとおりに並んだポート */
function insertSwitch(item, cx, cy) {
  if (readOnly) return;
  const spec = { ports: item.ports, uplinks: item.uplinks, rows: item.rows, order: newSwitchOrder, tabs: 'down', ...NAMING_DEFAULTS };
  const size = switchMetrics(spec);
  const centered = cx === undefined;
  const c = centered ? viewCenter() : { x: cx, y: cy };
  let x = snap(c.x - size.w / 2);
  let y = snap(c.y - size.h / 2);
  // パレットをクリックして置くとき: 既に図があれば、その下に左をそろえて並べる (スイッチは大きいので重なりやすい)
  const content = centered ? contentBounds() : null;
  if (content) {
    x = snap(content.x);
    y = snap(content.y + content.height + GRID * 2);
  }
  let chassis = null;
  graph.batchUpdate(() => {
    chassis = graph.insertVertex({
      parent: graph.getDefaultParent(),
      value: t('switchName'),
      position: [x, y],
      size: [size.w, size.h],
      style: {
        ...VERTEX_STYLE,
        shape: 'swimlane',
        startSize: SWITCH_TITLE,
        fillColor: '#dddddd',
        swimlaneFillColor: '#f4f4f4',
        strokeColor: '#333333',
        fontStyle: 1,
        align: 'left',
        spacingLeft: 8,
      },
    });
    applySwitch(chassis, spec);
  });
  graph.setSelectionCell(chassis);
  container.focus();
}

/** 設定欄が扱うスイッチ: 選んでいるスイッチ (か、選んでいるポートの入ったスイッチ)。何も選んでいなければ、図に 1 台だけのとき、その 1 台 */
function currentSwitch() {
  const selected = graph.getSelectionCells();
  const owners = new Set();
  for (const cell of selected) {
    if (isSwitch(cell)) owners.add(cell);
    else if (isSwitch(cell.getParent())) owners.add(cell.getParent());
  }
  if (owners.size === 1) return owners.values().next().value;
  if (owners.size > 1 || selected.length > 0) return null;
  const all = graph.getChildCells(graph.getDefaultParent(), true, false).filter(isSwitch);
  return all.length === 1 ? all[0] : null;
}

/** スイッチの設定欄の入力欄。`kind` は count (個数) / start (空にできない数) / optional (空 = 続きの番号) / text / select */
const SWITCH_FIELDS = [
  { key: 'ports', id: 'sw-ports', kind: 'count', max: SWITCH_MAX_PORTS, group: 'shape', label: 'swPorts' },
  { key: 'uplinks', id: 'sw-sfp', kind: 'count', max: SWITCH_MAX_SFP, group: 'shape', label: 'swSfpCount' },
  { key: 'rows', id: 'sw-rows', kind: 'select', group: 'shape', label: 'swRows' },
  { key: 'order', id: 'sw-order', kind: 'select', group: 'shape', label: 'swOrder', tip: 'swOrderTip' },
  { key: 'tabs', id: 'sw-tabs', kind: 'select', group: 'shape', label: 'swTabs', tip: 'swTabsTip' },
  { key: 'start', id: 'sw-start', kind: 'start', group: 'number', label: 'swStart', tip: 'swStartTip' },
  { key: 'sfpStart', id: 'sw-sfp-start', kind: 'optional', group: 'number', label: 'swSfpShort', tip: 'swSfpStartTip' },
  { key: 'name', id: 'sw-name-prefix', kind: 'text', group: 'name', label: null, tip: 'swNameTip', placeholder: 'Gi1/0/' },
  { key: 'nameStart', id: 'sw-name-start', kind: 'start', group: 'name', label: 'swStart', tip: 'swNameStartTip' },
  { key: 'sfpName', id: 'sw-sfp-name', kind: 'text', group: 'name', label: 'swSfpShort', tip: 'swSfpNameTip', placeholder: 'Te1/1/' },
  { key: 'sfpNameStart', id: 'sw-sfp-name-start', kind: 'optional', group: 'name', label: 'swStart', tip: 'swSfpNameStartTip' },
];
/** 選択リストの欄: 項目の一覧と、選んだ値の読み方 */
const SWITCH_SELECTS = {
  rows: { options: () => [1, 2].map((n) => [String(n), t('swRowsN').replace('{n}', n)]), parse: (raw) => (Number(raw) === 1 ? 1 : 2) },
  order: { options: () => PORT_ORDERS.map(([id, label]) => [id, t(label)]), parse: (raw) => (isPortOrder(raw) ? raw : 'zigzag') },
  tabs: { options: () => PORT_TABS.map(([id, label]) => [id, t(label)]), parse: (raw) => (isPortTabs(raw) ? raw : 'down') },
};
/** 設定欄のまとまりと、その見出し (無ければ見出しなし) */
const SWITCH_GROUPS = [
  ['shape', null],
  ['number', 'swNumberGroup'],
  ['name', 'swNameGroup'],
];

/** スイッチの設定欄 (ポートの数・段・並び順・番号とポート名の振り方) を作る。モードか言語が変わるたびに作り直す */
function buildSwitchBar() {
  const bar = document.getElementById('switch-bar');
  bar.textContent = '';
  bar.hidden = !ports;
  if (!ports) return;
  const name = el('span', { id: 'sw-name' });
  const hint = el('span', { id: 'sw-hint' });
  hint.textContent = t('swBarNone');
  const controls = new Map();
  const groups = new Map();
  for (const [id, title] of SWITCH_GROUPS) {
    const group = el('div', { class: 'sw-group' });
    if (title) {
      const head = el('span', { class: 'sw-group-title' });
      head.textContent = t(title);
      group.appendChild(head);
    }
    groups.set(id, group);
  }
  for (const field of SWITCH_FIELDS) {
    let control;
    if (field.kind === 'select') {
      control = el('select', { id: field.id, class: 'tb-select' });
      for (const [value, label] of SWITCH_SELECTS[field.key].options()) {
        const o = el('option', { value });
        o.textContent = label;
        control.appendChild(o);
      }
    } else if (field.kind === 'text') {
      control = el('input', { id: field.id, class: 'tb-input sw-text', type: 'text', spellcheck: 'false', maxlength: '40', placeholder: field.placeholder });
    } else {
      control = el('input', { id: field.id, class: 'tb-input', type: 'number', min: '0', max: String(field.max ?? NAMING_MAX), step: '1' });
      if (field.kind === 'optional') control.placeholder = t('swContinue');
    }
    if (field.tip) control.title = t(field.tip);
    const label = el('label');
    if (field.label) {
      const text = el('span');
      text.textContent = t(field.label);
      label.appendChild(text);
    }
    label.appendChild(control);
    groups.get(field.group).appendChild(label);
    controls.set(field.key, control);
  }

  // 欄に入ったときのスイッチに当てる: 数を打ってから図の別のスイッチをクリックすると、確定 (change) より先に
  // 選択が変わるので、今の選択を見ると別のスイッチを作り直してしまう
  let target = null;
  const apply = () => {
    const chassis = target && graph.getDataModel().contains(target) ? target : currentSwitch();
    if (!chassis || readOnly) return;
    const spec = {};
    for (const field of SWITCH_FIELDS) {
      const control = controls.get(field.key);
      const raw = control.value.trim();
      const number = Math.max(0, Math.min(field.max ?? NAMING_MAX, Math.round(Number(raw) || 0)));
      if (field.kind === 'select') spec[field.key] = SWITCH_SELECTS[field.key].parse(raw);
      else if (field.kind === 'text') spec[field.key] = raw;
      else if (field.kind === 'optional') spec[field.key] = raw === '' ? null : number;
      else spec[field.key] = field.kind === 'start' && raw === '' ? NAMING_DEFAULTS[field.key] : number;
    }
    applySwitch(chassis, spec);
    // 範囲の外の数を打ったときなどは、直した値を欄に戻す
    for (const control of controls.values()) delete control.dataset.typing;
    showSwitchSpec(spec);
  };
  for (const control of controls.values()) {
    control.addEventListener('focus', () => {
      target = currentSwitch();
    });
    // 打っている途中の欄は、図が変わっても書き換えない (updateSwitchBar)
    control.addEventListener('input', () => {
      control.dataset.typing = '1';
    });
    control.addEventListener('change', apply);
    control.addEventListener('blur', () => {
      delete control.dataset.typing;
      target = null;
      updateSwitchBar();
    });
  }
  bar.append(name, ...groups.values(), hint);
  updateSwitchBar();
}

/** 設定欄の入力欄に `spec` の値を入れる (打っている途中の欄はそのまま) */
function showSwitchSpec(spec) {
  for (const field of SWITCH_FIELDS) {
    const control = document.getElementById(field.id);
    const value = spec[field.key] === null || spec[field.key] === undefined ? '' : String(spec[field.key]);
    if (control && !control.dataset.typing && control.value !== value) control.value = value;
  }
}

/** スイッチの設定欄を、今のスイッチに合わせる */
function updateSwitchBar() {
  const bar = document.getElementById('switch-bar');
  if (!ports || !bar.firstChild) return;
  const chassis = currentSwitch();
  const off = !chassis || readOnly;
  for (const field of SWITCH_FIELDS) document.getElementById(field.id).disabled = off;
  for (const group of bar.querySelectorAll('.sw-group')) group.classList.toggle('off', !chassis);
  document.getElementById('sw-hint').hidden = !!chassis;
  document.getElementById('sw-name').textContent = chassis ? deviceName(chassis) : '';
  if (!chassis) return;
  const spec = switchSpec(chassis);
  showSwitchSpec(spec);
  // 1 段のときは並び順を選べない (左から右へ並ぶだけ)。SFP が無ければ SFP の欄も要らない
  document.getElementById('sw-order').disabled = off || spec.rows === 1;
  document.getElementById('sw-tabs').disabled = off || spec.ports === 0;
  for (const id of ['sw-sfp-start', 'sw-sfp-name', 'sw-sfp-name-start']) document.getElementById(id).disabled = off || spec.uplinks === 0;
}

/** ポートの一覧に出す項目 (portInfo のキーと、見出しの文言のキー) */
const PORT_FIELDS = [
  ['name', 'thName'],
  ['peer', 'thPeer'],
  ['vlan', 'thVlan'],
  ['speed', 'thSpeed'],
  ['note', 'thNote'],
];

function portInfo(cell) {
  try {
    const info = JSON.parse(String((cell.getStyle() ?? {}).portInfo ?? '{}'));
    return info && typeof info === 'object' ? info : {};
  } catch (_) {
    return {};
  }
}

/** 図の中のポート: スイッチ (親の枠) の並び順、その中では RJ-45・SFP の順で、それぞれポート番号の順 */
function portCells() {
  const cells = graph.getDataModel().getRoot().getDescendants().filter((c) => c.isVertex() && (c.getStyle() ?? {}).portKind);
  const layer = graph.getDefaultParent();
  const order = new Map();
  cells.forEach((c) => {
    if (!order.has(c.getParent())) order.set(c.getParent(), order.size);
  });
  const compare = (a, b) => String(a.getValue() ?? '').localeCompare(String(b.getValue() ?? ''), undefined, { numeric: true });
  // SFP は RJ-45 の後ろ (番号を振り直す機種では、同じ番号が RJ-45 と SFP の両方にある)
  const kind = (c) => ((c.getStyle() ?? {}).portKind === 'sfp' ? 1 : 0);
  cells.sort((a, b) => order.get(a.getParent()) - order.get(b.getParent()) || kind(a) - kind(b) || compare(a, b));
  return cells.map((cell) => ({ cell, owner: cell.getParent() === layer ? '' : deviceName(cell.getParent()) }));
}

/** ポートの一覧の行: [スイッチ, ポート, 接続先, VLAN, 速度, メモ] と、スイッチの列を出すか (2 台以上あるとき) */
function portRows() {
  const list = portCells();
  const owners = new Set(list.map((p) => p.cell.getParent()));
  const rows = list.map(({ cell, owner }) => {
    const info = portInfo(cell);
    return [owner, String(cell.getValue() ?? ''), ...PORT_FIELDS.map(([key]) => String(info[key] ?? ''))];
  });
  return { rows, multiple: owners.size > 1 };
}

/** ポートの一覧を、文書に貼れる形の文字列にする (結線表と同じ 3 つの形式) */
function portTableText(format) {
  const { rows, multiple } = portRows();
  const head = [t('thSwitch'), t('thPort'), ...PORT_FIELDS.map(([, label]) => t(label))];
  const cut = (row) => (multiple ? row : row.slice(1));
  return tableText(format, cut(head), rows.map(cut));
}

/** 表を Markdown / Typst / CSV の文字列にする */
function tableText(format, head, rows) {
  if (format === 'csv') {
    const cell = (v) => (/[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v);
    return `${[head, ...rows].map((row) => row.map(cell).join(',')).join('\n')}\n`;
  }
  if (format === 'typst') {
    // 文字列として書くので、Typst の記号 (_ や * や #) が入った名前もそのまま出る
    const cell = (v) => `"${v.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
    const lines = ['#table(', `  columns: ${head.length},`, `  table.header(${head.map((h) => `[${h}]`).join(', ')}),`];
    for (const row of rows) lines.push(`  ${row.map(cell).join(', ')},`);
    lines.push(')');
    return `${lines.join('\n')}\n`;
  }
  const cell = (v) => v.replace(/\\/g, '\\\\').replace(/\|/g, '\\|');
  const line = (row) => `| ${row.map(cell).join(' | ')} |`;
  return `${[line(head), `|${head.map(() => '---').join('|')}|`, ...rows.map(line)].join('\n')}\n`;
}

/** 表から書き込む: ポート番号はセルのラベル、それ以外は portInfo */
function setPortField(cell, key, value) {
  if (readOnly) return;
  const text = String(value ?? '').trim();
  graph.batchUpdate(() => {
    if (key === 'label') {
      if (String(cell.getValue() ?? '') !== text) graph.getDataModel().setValue(cell, text);
      return;
    }
    const info = portInfo(cell);
    if (String(info[key] ?? '') === text) return;
    if (text) info[key] = text;
    else delete info[key];
    graph.getDataModel().setStyle(cell, { ...(cell.getStyle() ?? {}), portInfo: JSON.stringify(info) });
  });
}

/** 今の表の並び (セルの ID を並べたもの)。同じなら、入力欄を作り直さずに値だけ合わせる */
let portTableKey = '';
let portTableCells = [];

/** ポートの一覧 (書き込める表) を今の図に合わせる */
function renderPortTable() {
  const host = document.getElementById('port-table');
  const list = portCells();
  const multiple = new Set(list.map((p) => p.cell.getParent())).size > 1;
  const key = `${multiple}|${list.map((p) => `${p.cell.getId()}:${p.owner}`).join(',')}`;
  const columns = ['label', ...PORT_FIELDS.map(([name]) => name)];
  if (key !== portTableKey) {
    // 行の並びが変わった: 表ごと作り直す
    portTableKey = key;
    portTableCells = list.map((p) => p.cell);
    host.textContent = '';
    if (list.length === 0) {
      const empty = el('div', { class: 'empty' });
      empty.textContent = t('portsEmpty');
      host.appendChild(empty);
      return;
    }
    const table = el('table');
    const head = el('tr');
    for (const label of [...(multiple ? ['thSwitch'] : []), 'thPort', ...PORT_FIELDS.map(([, l]) => l)]) {
      const th = el('th');
      th.textContent = t(label);
      head.appendChild(th);
    }
    table.appendChild(head);
    list.forEach(({ cell, owner }, rowIndex) => {
      const tr = el('tr');
      if (multiple) {
        const td = el('td', { class: 'name' });
        td.textContent = owner;
        tr.appendChild(td);
      }
      columns.forEach((column, columnIndex) => {
        const td = el('td');
        const input = el('input', { type: 'text', spellcheck: 'false' });
        input.addEventListener('change', () => setPortField(cell, column, input.value));
        // 行を選ぶと、図のポートも選ぶ
        input.addEventListener('focus', () => {
          if (graph.getDataModel().contains(cell) && !graph.isCellSelected(cell)) graph.setSelectionCell(cell);
        });
        // Enter で下の行の同じ列へ (表計算と同じ)
        input.addEventListener('keydown', (e) => {
          if (e.key !== 'Enter') return;
          e.preventDefault();
          input.blur();
          const next = host.querySelectorAll('tr')[rowIndex + 2];
          const target = next ? next.querySelectorAll('input')[columnIndex] : null;
          if (target) target.focus();
        });
        td.appendChild(input);
        tr.appendChild(td);
      });
      table.appendChild(tr);
    });
    host.appendChild(table);
  }
  // 値を合わせる (入力中の欄には触らない)
  const rows = host.querySelectorAll('tr');
  list.forEach(({ cell }, rowIndex) => {
    const info = portInfo(cell);
    const inputs = rows[rowIndex + 1].querySelectorAll('input');
    columns.forEach((column, columnIndex) => {
      const value = column === 'label' ? String(cell.getValue() ?? '') : String(info[column] ?? '');
      const input = inputs[columnIndex];
      if (input !== document.activeElement && input.value !== value) input.value = value;
    });
  });
  markSelectedPorts();
}

/** 図で選んでいるポートの行に印を付ける (表の外で選んだときは、その行が見えるようにする) */
function markSelectedPorts() {
  const host = document.getElementById('port-table');
  const rows = host.querySelectorAll('tr');
  let first = null;
  portTableCells.forEach((cell, i) => {
    const row = rows[i + 1];
    if (!row) return;
    const on = graph.isCellSelected(cell);
    row.classList.toggle('sel', on);
    if (on && !first) first = row;
  });
  if (first && !host.contains(document.activeElement)) first.scrollIntoView({ block: 'nearest' });
}

// ---------------------------------------------------------------------------
// Mermaid モード
// ---------------------------------------------------------------------------
//
// Markdown に貼る Mermaid のフローチャートを GUI で作るためのモード。Mermaid は図形の位置を自分で決め、
// 使える図形や線の種類も決まっているので、このモードでは操作をその範囲に絞る:
// - 図形は Mermaid のフローチャートにあるものだけ。位置は自動配置で、手では動かせない
// - 線は必ず図形と図形をつなぐ。図形の中からドラッグして引く
// - 文字の書式・重なり順・グループ・回転は無い
// モードと流れの向きは、図のデータ (レイヤーのセルのスタイル) に入れる。ファイルに保存され、
// 「元に戻す」でも一緒に戻る。画面の下に出すコードが、Markdown に貼る Mermaid の記述になる。

/** 流れの向き: Mermaid の書き方 → 自動配置の向き */
const DIRECTIONS = { TD: 'north', LR: 'west', BT: 'south', RL: 'east' };

/** Mermaid で書ける図形と、文字を囲む括弧 */
const MERMAID_BRACKETS = {
  rect: ['[', ']'],
  rounded: ['(', ')'],
  stadium: ['([', '])'],
  ellipse: ['((', '))'],
  doubleEllipse: ['(((', ')))'],
  rhombus: ['{', '}'],
  hexagon: ['{{', '}}'],
  parallelogram: ['[/', '/]'],
  trapezoid: ['[\\', '/]'],
  predefined: ['[[', ']]'],
  cylinder: ['[(', ')]'],
};

/** 図のデータに入っているモード */
function modeOf(g = graph) {
  const style = g.getDefaultParent().getStyle() ?? {};
  let name = MODES.includes(style.mode) ? style.mode : 'free';
  // 最初の版の書き方 (mode ができる前の、モードごとの印)
  if (!style.mode && Number(style.mermaid) === 1) name = 'mermaid';
  else if (!style.mode && Number(style.network) === 1) name = 'network';
  return { mode: name, direction: DIRECTIONS[style.direction] ? style.direction : 'TD' };
}

/** 図形が Mermaid のどの形に当たるか (当たるものが無ければ null) */
function mermaidKind(style) {
  const shape = style.shape ?? 'rectangle';
  if (shape === 'rectangle' || shape === 'label') {
    if (!style.rounded) return 'rect';
    return (style.arcSize ?? 15) >= 40 ? 'stadium' : 'rounded';
  }
  return MERMAID_BRACKETS[shape] ? shape : null;
}

/** これから引く線のスタイル。Mermaid モードの線は、自動配置が決めた点を通るなめらかな曲線にする */
function newEdgeStyle() {
  const style = { ...edgeDefaults };
  if (mermaid) {
    delete style.edgeStyle;
    style.curved = true;
    style.rounded = false;
    // Mermaid の線の端は「なし」か「矢印」
    for (const key of ['startArrow', 'endArrow']) {
      if (style[key] && style[key] !== 'none') style[key] = 'block';
    }
  }
  if (network) {
    // ネットワーク図の線: 機器どうしをまっすぐ結び、矢印は付けない
    delete style.edgeStyle;
    style.curved = false;
    style.startArrow = 'none';
    style.endArrow = 'none';
  }
  return style;
}

function createLayout(g, dir) {
  const layout = new mx.HierarchicalLayout(g, DIRECTIONS[dir] ?? 'north');
  layout.intraCellSpacing = 40;
  layout.interRankCellSpacing = 50;
  layout.interHierarchySpacing = 40;
  layout.parallelEdgeSpacing = 16;
  layout.parentBorder = 20;
  // 線のスタイルは自分で決める (配置が線ごとに noEdgeStyle を書き込まないように)
  layout.disableEdgeStyle = false;
  return layout;
}

/** つながっていない図形どうし、つながっている図とその後ろの図形の間隔 (自動配置の段の間隔と同じ) */
const LOOSE_GAP = 50;

/**
 * まだ線でつないでいない図形を、流れの向きに順に並べる (上から下の図なら、縦に 1 列)。つながっている図があれば、
 * その後ろ (流れの先) に続ける。
 * 配置 (maxGraph) は、つながっていない図形を流れと直角の向きに並べる (上から下の図でも、横 1 列。本物の Mermaid の
 * 描画も同じ)。それだと、図形を先にいくつか置いてから線でつなぐときに、次の相手が横に居て引きにくく、つなぐたびに
 * 大きく動く。流れの向きに並べておけば、順につないでいく間、図形はほとんど動かない
 */
function stackLooseShapes(g, parent, dir) {
  const vertices = g.getChildCells(parent, true, false).filter((v) => v.getGeometry());
  const loose = vertices.filter((v) => v.getEdgeCount() === 0);
  if (loose.length === 0 || (loose.length === 1 && vertices.length === 1)) return;
  const joined = vertices.filter((v) => v.getEdgeCount() > 0);
  const horizontal = dir === 'LR' || dir === 'RL';
  const backward = dir === 'BT' || dir === 'RL';
  // 流れの向きの座標 (main) と、それと直角の向きの座標 (cross)
  const box = (v) => {
    const q = v.getGeometry();
    return horizontal ? { main: q.x, size: q.width, cross: q.y, breadth: q.height } : { main: q.y, size: q.height, cross: q.x, breadth: q.width };
  };
  let center;
  let cursor;
  if (joined.length > 0) {
    const boxes = joined.map(box);
    const low = Math.min(...boxes.map((b) => b.cross));
    const high = Math.max(...boxes.map((b) => b.cross + b.breadth));
    center = (low + high) / 2;
    cursor = backward ? Math.min(...boxes.map((b) => b.main)) - LOOSE_GAP : Math.max(...boxes.map((b) => b.main + b.size)) + LOOSE_GAP;
  } else {
    const first = box(loose[0]);
    center = first.cross + first.breadth / 2;
    cursor = backward ? first.main + first.size : first.main;
  }
  const model = g.getDataModel();
  for (const cell of loose) {
    const b = box(cell);
    const main = backward ? cursor - b.size : cursor;
    cursor = backward ? main - LOOSE_GAP : main + b.size + LOOSE_GAP;
    const cross = center - b.breadth / 2;
    const geometry = cell.getGeometry().clone();
    geometry.x = horizontal ? main : cross;
    geometry.y = horizontal ? cross : main;
    model.setGeometry(cell, geometry);
  }
}

/**
 * 自動配置を「いちばん古い図形を動かさない」ようにする。maxGraph の配置は、いつもキャンバスの原点のそば (左上) から
 * 並べるので、そのままだと、最初の図形をどこに置いても左上へ飛ぶ (2 個目からも、図はずっとそこに居続ける)。
 * 配置の後で全体を同じだけずらして、いちばん古い図形を元の場所に戻す。ついでに、図形をグリッドに乗せる。
 * ずらすのも配置と同じ「元に戻す」の単位に入る
 */
function anchoredLayout(g, layout, dir) {
  const base = layout.execute.bind(layout);
  layout.execute = (parent, ...rest) => {
    const model = g.getDataModel();
    // このモードに置けない線 (自分自身につながる線・相手のいない線) が残っていたら消す。前の版で作った図や、
    // 図形だけを消した後に残ったもの。配置はこれらを描かないか、原点へ伸ばして描く
    for (const edge of g.getChildCells(parent, false, true)) {
      const source = edge.getTerminal(true);
      const target = edge.getTerminal(false);
      if (!source || !target || source === target) model.remove(edge);
    }
    const anchor = g.getChildCells(parent, true, false)[0];
    const before = anchor ? anchor.getGeometry() : null;
    const at = before ? { x: before.x, y: before.y } : null;
    base(parent, ...rest);
    stackLooseShapes(g, parent, dir);
    const after = anchor ? anchor.getGeometry() : null;
    if (!at || !after) return;
    const dx = at.x - after.x;
    const dy = at.y - after.y;
    const copy = (p) => (p ? new mx.Point(p.x, p.y) : p);
    for (const cell of g.getChildCells(parent, true, true)) {
      const geometry = cell.getGeometry();
      if (!geometry) continue;
      // 点は写しを作ってから動かす (元の点を書き換えると「元に戻す」で戻らない)
      const moved = geometry.clone();
      if (geometry.points) moved.points = geometry.points.map(copy);
      moved.sourcePoint = copy(geometry.sourcePoint);
      moved.targetPoint = copy(geometry.targetPoint);
      moved.translate(dx, dy);
      // 図形はグリッドに乗せる (配置は段の中央にそろえるので、そのままだと半端な位置になる。自由な図へ
      // 切り替えて手で動かした図形とそろうように)
      if (cell.isVertex() && g.isGridEnabled()) {
        moved.x = snap(moved.x);
        moved.y = snap(moved.y);
      }
      if (moved.x !== geometry.x || moved.y !== geometry.y || dx !== 0 || dy !== 0) model.setGeometry(cell, moved);
    }
  };
  return layout;
}

/** 図の中の、Mermaid で表せない要素 */
function findIncompatible() {
  const cells = graph.getChildCells(graph.getDefaultParent(), true, true);
  const found = { shapes: [], lines: [], groups: [], rotated: [] };
  for (const cell of cells) {
    if (cell.isEdge()) {
      const source = cell.getTerminal(true);
      const target = cell.getTerminal(false);
      // 自分自身につながる線も入れる (Mermaid モードでは作れず、自動配置でも描かれない)
      if (!source || !target || !source.isVertex() || !target.isVertex() || source === target) found.lines.push(cell);
      continue;
    }
    if (cell.getChildCount() > 0) {
      found.groups.push(cell);
      continue;
    }
    const style = graph.getCurrentCellStyle(cell);
    if (mermaidKind(style) === null) found.shapes.push(cell);
    if (style.rotation) found.rotated.push(cell);
  }
  return found;
}

/**
 * 図を Mermaid で表せる形に直す (モードを切り替えるとき・貼り付けたとき。呼び出し側の batchUpdate の中で使う)。
 * 枠とグループは解除し、Mermaid に無い図形は四角形にし、つながっていない線は消す。線は接続点と折れ点を捨てて、
 * 自動配置に任せる
 */
function normalizeForMermaid() {
  const parent = graph.getDefaultParent();
  const model = graph.getDataModel();
  // 入れ子の枠も、無くなるまで外から順に解除する
  for (let i = 0; i < 20; i += 1) {
    const groups = graph.getChildCells(parent, true, false).filter((c) => c.getChildCount() > 0);
    if (groups.length === 0) break;
    graph.ungroupCells(groups);
  }
  const found = findIncompatible();
  if (found.lines.length > 0) graph.removeCells(found.lines, false);
  dropStyleKeys(found.shapes, ['icon', 'portKind', 'portInfo', 'portRows', 'portOrder', 'portNaming', 'portTabs', 'portTab', 'spacingBottom', 'shape', 'perimeter', 'rounded', 'arcSize', 'verticalLabelPosition', 'spacingLeft', 'spacingTop', 'startSize', 'swimlaneFillColor']);
  if (found.shapes.length > 0) {
    graph.setCellStyles('align', 'center', found.shapes);
    graph.setCellStyles('verticalAlign', 'middle', found.shapes);
  }
  dropStyleKeys(found.rotated, ['rotation']);

  for (const edge of graph.getChildCells(parent, false, true)) {
    // 線の端のポート名 (ネットワーク図) は Mermaid に無い
    if (edge.getChildCount() > 0) graph.removeCells(edge.getChildren().slice(), false);
    dropStyleKeys([edge], ['edgeStyle', 'exitX', 'exitY', 'entryX', 'entryY', 'exitPerimeter', 'entryPerimeter', 'rounded']);
    const own = edge.getStyle() ?? {};
    if (!own.curved) graph.setCellStyles('curved', true, [edge]);
    for (const key of ['startArrow', 'endArrow']) {
      if (own[key] && own[key] !== 'none' && own[key] !== 'block') graph.setCellStyles(key, 'block', [edge]);
    }
    const geo = edge.getGeometry();
    if (geo && geo.points && geo.points.length > 0) {
      const next = geo.clone();
      next.points = [];
      model.setGeometry(edge, next);
    }
  }
}

/**
 * セルのスタイルからキーを消す。maxGraph の setCellStyles(key, null) はキーを残して値を null にするだけで、
 * 接続点 (exitX など) は null でも「指定あり」と扱われて、線が図形の左上の角につながってしまう
 */
function dropStyleKeys(cells, keys) {
  const model = graph.getDataModel();
  for (const cell of cells) {
    const own = cell.getStyle() ?? {};
    if (!keys.some((key) => key in own)) continue;
    const next = { ...own };
    for (const key of keys) delete next[key];
    model.setStyle(cell, next);
  }
}

/** 今のモードの名前 (ツールバーの選択リストの値) */
function modeName() {
  return mode;
}

/** モードを書き込んだレイヤーのスタイル (最初の版の印は消す) */
function modeStyle(parent, name, dir) {
  const style = { ...(parent.getStyle() ?? {}), mode: name, direction: dir };
  delete style.mermaid;
  delete style.network;
  return style;
}

/** モードを切り替える。Mermaid にするときに表せない要素を変えることになるなら、先に確かめる */
function setMode(name) {
  if (readOnly || name === modeName()) return;
  const on = name === 'mermaid';
  const parent = graph.getDefaultParent();
  const apply = () => {
    graph.batchUpdate(() => {
      if (on) normalizeForMermaid();
      graph.getDataModel().setStyle(parent, modeStyle(parent, name, direction));
    });
    container.focus();
  };
  // 選択リストは、切り替えが済むまで今のモードのままにしておく (取り消したときに戻す手間を省く)
  controls.mode.value = modeName();
  if (!on) {
    apply();
    return;
  }
  const found = findIncompatible();
  const items = [
    ['mermaidShapes', found.shapes.length],
    ['mermaidLines', found.lines.length],
    ['mermaidGroups', found.groups.length],
    ['mermaidRotated', found.rotated.length],
  ]
    .filter(([, n]) => n > 0)
    .map(([key, n]) => t(key).replace('{n}', String(n)));
  if (items.length === 0) {
    apply();
    return;
  }
  confirmBox(t('mermaidConfirm').replace('{items}', items.join('\n')), t('mermaidSwitch'), apply);
}

function setDirection(dir) {
  if (readOnly || !mermaid || !DIRECTIONS[dir]) return;
  const parent = graph.getDefaultParent();
  graph.batchUpdate(() => {
    graph.getDataModel().setStyle(parent, modeStyle(parent, 'mermaid', dir));
  });
  container.focus();
}

/** 画面 (ツールバー・パレット・コードの欄) を、図のデータに入っているモードに合わせる */
function syncMode(force = false) {
  if (!graph) return;
  const now = modeOf();
  const modeChanged = now.mode !== mode;
  const directionChanged = now.direction !== direction;
  if (!force && !modeChanged && !directionChanged) return;
  mode = now.mode;
  mermaid = mode === 'mermaid';
  network = mode === 'network';
  ports = mode === 'ports';
  direction = now.direction;
  document.getElementById('app').dataset.mode = mode;
  document.getElementById('code').hidden = !PANELS[mode];
  // ネットワーク図モードでは、機器に重ねて出る印から線を引く
  graph.getPlugin('ConnectionHandler').connectImage = network ? connectImage() : null;
  if (force || modeChanged) {
    buildToolbar();
    buildPalette();
    buildCodePanel();
    // 選択枠のハンドル (回転など) を今のモードのものに作り直す
    graph.getPlugin('SelectionCellsHandler').refresh();
  }
  updateToolbar();
  updateViewport();
  // 並べ直されたので、図を画面の中央に戻す (読み込みのときは load が行う)
  if (!force && mermaid) centerView(true);
}

/** Mermaid の文字列: 引用符で囲み、中の引用符と改行を Mermaid の書き方にする */
function mermaidText(value) {
  const text = String(value ?? '').trim();
  return `"${(text || ' ').replace(/"/g, '#quot;').replace(/\r?\n/g, '<br/>')}"`;
}

/** 今の図を Mermaid のフローチャートの記述にする */
function generateMermaid() {
  const parent = graph.getDefaultParent();
  const id = (cell) => `n${cell.getId()}`;
  const lines = [`flowchart ${direction}`];
  const styles = [];
  // 色: 既定 (白地に黒) から変えたものだけ書く。書かなければ、貼った先の Mermaid の配色になる。
  // 「なし」は maxGraph が描くときのスタイルから消すので、セルに書かれた値を見る
  const colorOf = (cell, style, key, standard) => {
    const value = (cell.getStyle() ?? {})[key] ?? style[key];
    return typeof value === 'string' && value !== '' && value !== standard ? value : null;
  };

  for (const cell of graph.getChildCells(parent, true, false)) {
    const style = graph.getCurrentCellStyle(cell);
    const [open, close] = MERMAID_BRACKETS[mermaidKind(style) ?? 'rect'];
    lines.push(`    ${id(cell)}${open}${mermaidText(cell.getValue())}${close}`);
    const css = [];
    const fill = colorOf(cell, style, 'fillColor', '#ffffff');
    const stroke = colorOf(cell, style, 'strokeColor', '#000000');
    const color = colorOf(cell, style, 'fontColor', '#000000');
    if (fill) css.push(`fill:${fill}`);
    if (stroke) css.push(`stroke:${stroke}`);
    if ((style.strokeWidth ?? 1) !== 1) css.push(`stroke-width:${style.strokeWidth}px`);
    if (style.dashed) css.push('stroke-dasharray:5 5');
    if (color) css.push(`color:${color}`);
    if (css.length > 0) styles.push(`    style ${id(cell)} ${css.join(',')}`);
  }

  let index = 0;
  for (const edge of graph.getChildCells(parent, false, true)) {
    let source = edge.getTerminal(true);
    let target = edge.getTerminal(false);
    if (!source || !target || !source.isVertex() || !target.isVertex()) continue;
    const style = graph.getCurrentCellStyle(edge);
    let start = !!style.startArrow && style.startArrow !== 'none';
    let end = !!style.endArrow && style.endArrow !== 'none';
    // 始点だけに矢印がある線は Mermaid に無いので、向きを入れ替えて書く
    if (start && !end) {
      [source, target] = [target, source];
      [start, end] = [false, true];
    }
    let op;
    if (style.dashed) op = `${start ? '<' : ''}-.-${end ? '>' : ''}`;
    else if ((style.strokeWidth ?? 1) >= 3) op = `${start ? '<' : ''}==${end ? '>' : '='}`;
    else op = `${start ? '<' : ''}--${end ? '>' : '-'}`;
    const label = String(edge.getValue() ?? '').trim();
    lines.push(`    ${id(source)} ${op}${label ? `|${mermaidText(label)}|` : ''} ${id(target)}`);
    const stroke = colorOf(edge, style, 'strokeColor', '#000000');
    if (stroke && stroke !== 'none') styles.push(`    linkStyle ${index} stroke:${stroke}`);
    index += 1;
  }
  return `${[...lines, ...styles].join('\n')}\n`;
}

/** コードの欄を今の図に合わせる */
function updateCode() {
  if (!graph) return;
  const panel = PANELS[mode];
  const table = !!(panel && panel.table);
  document.getElementById('code-text').hidden = table;
  document.getElementById('port-table').hidden = !table;
  if (table) renderPortTable();
  else document.getElementById('code-text').value = codeText();
  updateSwitchBar();
}

/**
 * 下の欄 (モードごと。無いモードでは欄を出さない): 題・説明・出す文字列・表の形式を選べるか・
 * Markdown のコードブロックとしてコピーするボタンがあるか・書き込める表にするか
 */
const PANELS = {
  mermaid: { title: () => 'Mermaid', hint: 'mermaidHint', text: () => generateMermaid(), formats: false, fence: true },
  network: { title: () => t('tableTitle'), hint: 'tableHint', text: () => connectionTable(tableFormat), formats: true },
  ports: { title: () => t('portsTitle'), hint: 'portsHint', text: () => portTableText(tableFormat), formats: true, table: true },
};

/** 下の欄に出す (コピーする) 文字列 */
function codeText() {
  const panel = PANELS[mode];
  return panel ? panel.text() : '';
}

/** コードの欄の文言とボタン (モードか言語が変わるたびに作り直す) */
function buildCodePanel() {
  const panel = PANELS[mode] ?? { title: () => '', hint: '', formats: false };
  document.getElementById('code-title').textContent = panel.title();
  buildSwitchBar();
  document.getElementById('code-hint').textContent = panel.hint ? t(panel.hint) : '';
  // 表の形式 (結線表とポートの一覧)
  const old = document.getElementById('code-format');
  const format = old.cloneNode(false);
  old.replaceWith(format);
  format.hidden = !panel.formats;
  format.title = t('tableFormat');
  for (const [value, label] of [['markdown', 'Markdown'], ['typst', 'Typst'], ['csv', 'CSV']]) {
    const o = el('option', { value });
    o.textContent = label;
    format.appendChild(o);
  }
  format.value = tableFormat;
  format.addEventListener('change', () => {
    tableFormat = format.value;
    updateCode();
  });
  const bind = (id, label, title, text) => {
    const old = document.getElementById(id);
    // 言語が変わるたびに呼ばれるので、前の処理が残らないよう作り直す
    const btn = old.cloneNode(false);
    old.replaceWith(btn);
    btn.textContent = label;
    btn.title = title;
    btn.addEventListener('click', () => {
      post(`copytext:${text()}`);
      btn.textContent = t('copied');
      setTimeout(() => {
        btn.textContent = label;
      }, 1200);
    });
  };
  bind('code-copy', t('copyCode'), t('copyCodeTip'), codeText);
  bind('code-copy-md', t('copyFence'), t('copyFenceTip'), () => `\`\`\`mermaid\n${generateMermaid()}\`\`\`\n`);
  document.getElementById('code-copy-md').hidden = !panel.fence;
  // 書き込める表は、言語が変わったら見出しごと作り直す
  portTableKey = '';
}

// ---------------------------------------------------------------------------
// ツールバーとパレット
// ---------------------------------------------------------------------------

const el = (tag, attrs = {}, html = '') => {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') node.className = v;
    else node.setAttribute(k, v);
  }
  if (html) node.innerHTML = html;
  return node;
};

const icon = (body, fill = false) =>
  `<svg viewBox="0 0 16 16" fill="${fill ? 'currentColor' : 'none'}" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;

/** ツールバーの部品 (updateToolbar が選択に合わせて値と有効 / 無効を直す) */
const controls = {};

function button(parent, key, title, html, onClick) {
  const b = el('button', { class: 'tb-btn', title, type: 'button' }, html);
  b.addEventListener('click', () => {
    onClick(b);
    container.focus();
  });
  parent.appendChild(b);
  if (key) controls[key] = b;
  return b;
}

function select(parent, key, title, options, onChange) {
  const s = el('select', { class: 'tb-select', title });
  for (const [value, label] of options) {
    const o = el('option', { value });
    o.textContent = label;
    s.appendChild(o);
  }
  s.addEventListener('change', () => {
    onChange(s.value, s);
    container.focus();
  });
  parent.appendChild(s);
  if (key) controls[key] = s;
  return s;
}

function colorButton(parent, key, title, glyph, styleKey, allowNone, targets) {
  const b = button(parent, key, title, `${glyph}<span class="swatch"></span>`, (btn) => {
    showColors(btn, allowNone, (color) => applyStyle(styleKey, color, targets()));
  });
  b.classList.add('color');
  return b;
}

function group(parent) {
  const g = el('div', { class: 'group' });
  parent.appendChild(g);
  return g;
}

function separator(parent) {
  parent.appendChild(el('div', { class: 'sep' }));
}

function buildChrome() {
  buildToolbar();
  buildPalette();
  buildCodePanel();
  if (graph) {
    updateToolbar();
    updateViewport();
  }
}

function buildToolbar() {
  const bar = document.getElementById('toolbar');
  bar.textContent = '';
  for (const key of Object.keys(controls)) delete controls[key];

  // モード: 自由な図か、Mermaid で表せる範囲に絞った図か
  let g = group(bar);
  select(g, 'mode', t('mode'), MODES.map((name) => [name, t(MODE_LABELS[name])]), (v) => setMode(v));
  controls.mode.value = modeName();
  if (mermaid) {
    select(g, 'direction', t('direction'), Object.keys(DIRECTIONS).map((d) => [d, t(`dir${d}`)]), (v) => setDirection(v));
    controls.direction.value = direction;
  }
  separator(bar);

  g = group(bar);
  colorButton(g, 'fill', t('fill'), icon('<path d="M3 9 L8 3 L13 9 L8 14 Z"/>', true), 'fillColor', true, selectedVertices);
  colorButton(g, 'stroke', t('stroke'), icon('<path d="M3 13 L13 3"/>'), 'strokeColor', true, selected);
  colorButton(g, 'fontColor', t('fontColor'), icon('<path d="M4 13 L8 3 L12 13 M5.5 9.5 L10.5 9.5"/>'), 'fontColor', false, selected);
  separator(bar);

  g = group(bar);
  select(g, 'strokeWidth', t('strokeWidth'), [1, 2, 3, 4, 6, 8].map((n) => [String(n), `${n} px`]), (v) => applyStyle('strokeWidth', Number(v)));
  select(g, 'dash', t('dash'), [['solid', t('solid')], ['dashed', t('dashed')], ['dotted', t('dotted')]], (v) => applyStyles(DASHES[v], selected()));
  if (mermaid) {
    // Mermaid モード: 角の丸め・影・文字の書式・線の引き方・重なり順・整列は Mermaid に無いので出さない
    separator(bar);
    g = group(bar);
    const arrows = [['none', t('arrowNone')], ['block', t('arrowBlock')]];
    select(g, 'startArrow', t('startArrow'), arrows.map(([v, l]) => [v, `◀ ${l}`]), (v) => applyStyles({ startArrow: ARROWS[v][0], startFill: ARROWS[v][1] }, selectedEdges()));
    select(g, 'endArrow', t('endArrow'), arrows.map(([v, l]) => [v, `▶ ${l}`]), (v) => applyStyles({ endArrow: ARROWS[v][0], endFill: ARROWS[v][1] }, selectedEdges()));
    separator(bar);
    buildViewControls(bar);
    return;
  }
  button(g, 'rounded', t('roundedCorners'), icon('<path d="M3 13 L3 7 Q3 3 7 3 L13 3"/>'), () => {
    const cells = selected();
    if (cells.length === 0) return;
    applyStyle('rounded', !graph.getCurrentCellStyle(cells[0]).rounded, cells);
  });
  button(g, 'shadow', t('shadow'), icon('<rect x="2.5" y="2.5" width="9" height="9"/><path d="M13.5 5 L13.5 13.5 L5 13.5"/>'), () => {
    const cells = selectedVertices();
    if (cells.length === 0) return;
    applyStyle('shadow', !graph.getCurrentCellStyle(cells[0]).shadow, cells);
  });
  separator(bar);

  g = group(bar);
  select(g, 'fontSize', t('fontSize'), [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 32, 40, 48].map((n) => [String(n), `${n} pt`]), (v) => applyStyle('fontSize', Number(v)));
  button(g, 'bold', t('bold'), '<b>B</b>', () => toggleFontFlag(1));
  button(g, 'italic', t('italic'), '<i>I</i>', () => toggleFontFlag(2));
  button(g, 'alignLeft', t('alignLeft'), icon('<path d="M2 4 L14 4 M2 8 L10 8 M2 12 L12 12"/>'), () => applyStyle('align', 'left', selectedVertices()));
  button(g, 'alignCenter', t('alignCenter'), icon('<path d="M2 4 L14 4 M4 8 L12 8 M3 12 L13 12"/>'), () => applyStyle('align', 'center', selectedVertices()));
  button(g, 'alignRight', t('alignRight'), icon('<path d="M2 4 L14 4 M6 8 L14 8 M4 12 L14 12"/>'), () => applyStyle('align', 'right', selectedVertices()));
  select(g, 'valign', t('valign'), [['top', t('top')], ['middle', t('middle')], ['bottom', t('bottom')]], (v) => applyStyle('verticalAlign', v, selectedVertices()));
  separator(bar);

  g = group(bar);
  select(g, 'route', t('route'), [['orthogonal', t('orthogonal')], ['straight', t('straight')], ['curved', t('curved')]], (v) => applyStyles(ROUTES[v], selectedEdges()));
  const arrowOptions = [['none', t('arrowNone')], ['block', t('arrowBlock')], ['open', t('arrowOpen')], ['oval', t('arrowOval')], ['diamond', t('arrowDiamond')]];
  select(g, 'startArrow', t('startArrow'), arrowOptions.map(([v, l]) => [v, `◀ ${l}`]), (v) => applyStyles({ startArrow: ARROWS[v][0], startFill: ARROWS[v][1] }, selectedEdges()));
  select(g, 'endArrow', t('endArrow'), arrowOptions.map(([v, l]) => [v, `▶ ${l}`]), (v) => applyStyles({ endArrow: ARROWS[v][0], endFill: ARROWS[v][1] }, selectedEdges()));
  if (network) {
    // ネットワーク図モード: 選んだ線の両端のポート名 (線の端に出て、結線表にも入る)
    for (const [key, source] of [['portSource', true], ['portTarget', false]]) {
      const input = el('input', { class: 'tb-input', type: 'text', title: t(key), placeholder: t('portHint'), spellcheck: 'false' });
      input.addEventListener('change', () => {
        const edges = selectedEdges();
        if (edges.length === 1) setPort(edges[0], source, input.value);
      });
      input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') input.blur();
      });
      g.appendChild(input);
      controls[key] = input;
    }
  }
  separator(bar);

  g = group(bar);
  button(g, 'toFront', t('toFront'), icon('<rect x="2" y="2" width="8" height="8" fill="currentColor"/><rect x="6" y="6" width="8" height="8"/>'), () => graph.orderCells(false, selected()));
  button(g, 'toBack', t('toBack'), icon('<rect x="6" y="6" width="8" height="8" fill="currentColor"/><rect x="2" y="2" width="8" height="8"/>'), () => graph.orderCells(true, selected()));
  select(g, 'arrange', t('arrange'), [
    ['', t('arrange')],
    ['left', t('arrangeLeft')], ['center', t('arrangeCenter')], ['right', t('arrangeRight')],
    ['top', t('arrangeTop')], ['middle', t('arrangeMiddle')], ['bottom', t('arrangeBottom')],
    ['distributeH', t('distributeH')], ['distributeV', t('distributeV')],
  ], (v, s) => {
    if (v === 'distributeH' || v === 'distributeV') distributeSelection(v === 'distributeH');
    else if (v) alignSelection(v);
    s.value = '';
  });
  if (ports) {
    button(g, 'flipTab', t('flipTab'), icon('<path d="M3 13 L13 13 L13 6 L10 6 L10 3 L6 3 L6 6 L3 6 Z"/>'), flipPortTabs);
  }
  separator(bar);
  buildViewControls(bar);
}

/** ツールバーの右端: グリッドと拡大率 (どのモードでも出す) */
function buildViewControls(bar) {
  const g = group(bar);
  button(g, 'grid', t('grid'), icon('<path d="M2 6 L14 6 M2 10 L14 10 M6 2 L6 14 M10 2 L10 14"/>'), () => {
    gridVisible = !gridVisible;
    graph.setGridEnabled(gridVisible);
    updateToolbar();
    updateViewport();
  });
  button(g, 'zoomOut', t('zoomOut'), icon('<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 L14 14 M5 7 L9 7"/>'), () => zoomBy(1 / 1.25));
  const label = button(g, 'zoomLabel', t('zoomReset'), '100%', () => zoomAt(1, container.clientWidth / 2, container.clientHeight / 2));
  label.id = 'zoom-label';
  button(g, 'zoomIn', t('zoomIn'), icon('<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 L14 14 M5 7 L9 7 M7 5 L7 9"/>'), () => zoomBy(1.25));
  button(g, 'fit', t('fit'), icon('<path d="M2 6 L2 2 L6 2 M10 2 L14 2 L14 6 M14 10 L14 14 L10 14 M6 14 L2 14 L2 10"/>'), () => centerView(false, true));
}

/** ツールバーの値と有効 / 無効を選択に合わせる */
function updateToolbar() {
  if (!graph || !controls.fill) return;
  const cells = readOnly ? [] : selected();
  const vertices = cells.filter((c) => c.isVertex());
  const edges = cells.filter((c) => c.isEdge());
  const first = cells[0] ? graph.getCurrentCellStyle(cells[0]) : null;
  const vertex = vertices[0] ? graph.getCurrentCellStyle(vertices[0]) : null;
  const edge = edges[0] ? graph.getCurrentCellStyle(edges[0]) : null;

  // Mermaid モードでは無い部品がある
  const enable = (key, on) => {
    if (controls[key]) controls[key].disabled = !on;
  };
  const toggle = (key, on) => {
    if (controls[key]) controls[key].classList.toggle('on', on);
  };
  const swatch = (key, color) => {
    const s = controls[key].querySelector('.swatch');
    const none = !color || color === 'none';
    s.style.background = none ? 'transparent' : color;
    s.classList.toggle('none', none);
  };
  const any = cells.length > 0;
  enable('fill', vertices.length > 0);
  swatch('fill', vertex ? vertex.fillColor : null);
  enable('stroke', any);
  swatch('stroke', first ? first.strokeColor : null);
  enable('fontColor', any);
  swatch('fontColor', first ? first.fontColor : null);

  enable('strokeWidth', any);
  enable('dash', any);
  enable('rounded', any);
  enable('shadow', vertices.length > 0);
  if (first) {
    setSelect(controls.strokeWidth, String(first.strokeWidth ?? 1), `${first.strokeWidth ?? 1} px`);
    controls.dash.value = dashOf(first);
  }
  toggle('rounded', !!(first && first.rounded));
  toggle('shadow', !!(vertex && vertex.shadow));

  enable('fontSize', any);
  enable('bold', any);
  enable('italic', any);
  if (first && controls.fontSize) setSelect(controls.fontSize, String(first.fontSize ?? 12), `${first.fontSize ?? 12} pt`);
  toggle('bold', !!(first && (first.fontStyle ?? 0) & 1));
  toggle('italic', !!(first && (first.fontStyle ?? 0) & 2));
  for (const [key, value] of [['alignLeft', 'left'], ['alignCenter', 'center'], ['alignRight', 'right']]) {
    enable(key, vertices.length > 0);
    toggle(key, !!(vertex && (vertex.align ?? 'center') === value));
  }
  enable('valign', vertices.length > 0);
  if (vertex && controls.valign) controls.valign.value = vertex.verticalAlign ?? 'middle';

  // 線の設定は、線を選んでいなければ「次に引く線」の設定を表す
  const edgeStyle = edge ?? edgeDefaults;
  const arrowValue = (value) => {
    if (!value || value === 'none') return 'none';
    // Mermaid モードの選択肢は「なし / 矢印」だけ
    return mermaid || !ARROWS[value] ? 'block' : value;
  };
  if (controls.route) controls.route.value = routeOf(edgeStyle);
  controls.startArrow.value = arrowValue(edgeStyle.startArrow);
  controls.endArrow.value = arrowValue(edgeStyle.endArrow);
  const edgeControls = edges.length > 0 || cells.length === 0;
  enable('route', edgeControls && !readOnly);
  enable('startArrow', edgeControls && !readOnly);
  enable('endArrow', edgeControls && !readOnly);

  enable('toFront', any);
  enable('toBack', any);
  enable('arrange', vertices.length > 1);
  enable('flipTab', vertices.some((c) => (c.getStyle() ?? {}).portKind === 'rj45'));
  // ポート名の欄: 線を 1 本だけ選んでいるときに使える。どちら側かは、つないだ機器の名前で示す
  for (const [key, source] of [['portSource', true], ['portTarget', false]]) {
    const input = controls[key];
    if (!input || document.activeElement === input) continue;
    const one = edges.length === 1 ? edges[0] : null;
    input.disabled = !one;
    input.value = one ? String((portCell(one, source) || { getValue: () => '' }).getValue() ?? '') : '';
    const device = one ? one.getTerminal(source) : null;
    input.placeholder = device ? `${deviceName(device)} ${t('portHint')}` : t('portHint');
  }
  enable('mode', !readOnly);
  enable('direction', !readOnly);
  controls.mode.value = modeName();
  if (controls.direction) controls.direction.value = direction;
  controls.grid.classList.toggle('on', gridVisible);
}

/** 選択リストに値を入れる。一覧に無い値 (ほかで付けた太さなど) は項目を足して表示する */
function setSelect(control, value, label) {
  if (!Array.from(control.options).some((o) => o.value === value)) {
    const o = el('option', { value });
    o.textContent = label;
    control.appendChild(o);
  }
  control.value = value;
}

/**
 * パレットの項目を、キャンバスへドラッグして置けるようにする。図形は中心を掴む: カーソルの位置 (グリッドに吸着した点) が
 * 図形の中心になる。`drop(cx, cy)` に、その中心が渡る。(w, h) は置く図形の大きさ
 */
function dragFromPalette(node, w, h, drop) {
  if (!graph) return;
  const preview = el('div', { class: 'drag-preview' });
  preview.style.width = `${w}px`;
  preview.style.height = `${h}px`;
  // maxGraph は、プレビューの左上をカーソルに合わせ、落としたときにその左上の座標を渡してくる。プレビューを大きさの
  // 半分だけ左上へずらしておくと、カーソルがプレビューの中心に来て、渡ってくる座標 + 半分 = カーソルの位置になる。
  // キャンバスの外 (パレットの上) で出る絵は拡大されないが、キャンバスの上のプレビューは拡大率が掛かる
  const source = mx.gestureUtils.makeDraggable(node, graph, (_g, _evt, _target, x, y) => drop(x + w / 2, y + h / 2), preview, -w / 2, -h / 2, true, true, false);
  source.isGuidesEnabled = () => false;
  Object.defineProperty(source, 'previewOffset', {
    configurable: true,
    get: () => new mx.Point((-w * graph.view.scale) / 2, (-h * graph.view.scale) / 2),
    set: () => {},
  });
  // グリッドへの吸着は自前で行う。maxGraph はカーソルの位置を吸着させてからプレビューをずらすので、大きさの半分が
  // グリッドの倍数でない図形 (幅 210 など) は、プレビューがグリッドから半分ずれて出て、置いた瞬間に動いて見える。
  // ここでは、プレビューの左上を「置かれる場所」(= 図形の左上をグリッドに吸着させた位置) に合わせる
  source.isGridEnabled = () => false;
  const baseDragOver = source.dragOver.bind(source);
  source.dragOver = (g, evt) => {
    baseDragOver(g, evt);
    if (!source.currentPoint || !source.previewElement) return;
    const s = g.view.scale;
    const tr = g.view.translate;
    const x = (gridSnap(source.currentPoint.x / s - tr.x) + tr.x) * s;
    const y = (gridSnap(source.currentPoint.y / s - tr.y) + tr.y) * s;
    source.previewElement.style.left = `${Math.round(x)}px`;
    source.previewElement.style.top = `${Math.round(y)}px`;
    source.currentPoint = new mx.Point(x, y);
  };
}

function buildPalette() {
  const palette = document.getElementById('palette');
  palette.textContent = '';
  const section = (title) => {
    const h = el('h3');
    h.textContent = title;
    palette.appendChild(h);
    const items = el('div', { class: 'items' });
    palette.appendChild(items);
    return items;
  };
  const svg = (body) =>
    `<svg viewBox="0 0 32 26" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" stroke-linecap="round">${body}</svg>`;
  const add = (parent, item, width, height, insert) => {
    const node = el('div', { class: 'pal-item', title: t(item.id) }, svg(item.icon));
    node.addEventListener('click', () => insert(item));
    dragFromPalette(node, width, height, (cx, cy) => insert(item, cx, cy));
    parent.appendChild(node);
  };
  if (ports) {
    // ポート図モード: 足し引き用のポート単体と説明用の図形、その下にスイッチ (ポートの並んだ箱) のひな形。
    // ひな形は、ポートの数ごとの見出しの下に並べる
    const parts = section(t('portParts'));
    for (const item of PORT_ITEMS) add(parts, item, item.w, item.h, insertShape);
    for (const id of ['rect', 'rounded', 'text', 'note']) {
      const item = SHAPE_ITEMS.find((i) => i.id === id);
      if (item) add(parts, item, item.w, item.h, insertShape);
    }
    for (const item of LINE_ITEMS.slice(0, 2)) add(parts, item, 120, 1, insertLine);

    // 新しく置くスイッチの、番号の並び順
    const orderHead = el('h3');
    orderHead.textContent = t('newOrder');
    palette.appendChild(orderHead);
    const order = el('select', { class: 'tb-select', title: t('newOrderTip') });
    for (const [id, label] of PORT_ORDERS) {
      const o = el('option', { value: id });
      o.textContent = t(label);
      order.appendChild(o);
    }
    order.value = newSwitchOrder;
    order.addEventListener('change', () => {
      newSwitchOrder = order.value;
      try {
        window.localStorage.setItem(PORT_ORDER_KEY, newSwitchOrder);
      } catch (_) {
        /* 覚えられなくても、開いている間は効く */
      }
    });
    palette.appendChild(order);

    let count = null;
    let group = null;
    for (const item of SWITCH_ITEMS) {
      if (item.ports !== count) {
        count = item.ports;
        group = section(t('swHead').replace('{n}', count));
        group.classList.add('wide');
      }
      const size = switchMetrics({ ...item, order: newSwitchOrder });
      const node = el('div', { class: 'pal-item pal-switch', title: switchTitle(item) }, switchIcon(item));
      node.addEventListener('click', () => insertSwitch(item));
      dragFromPalette(node, size.w, size.h, (cx, cy) => insertSwitch(item, cx, cy));
      group.appendChild(node);
    }
    return;
  }
  if (network) {
    // ネットワーク図モード: 絵柄 (アイコンのセット) を選んで、その機器を並べる。下に基本の図形と線
    const sets = Array.from(icons.sets.values());
    if (!icons.sets.has(paletteSet)) paletteSet = sets.length > 0 ? sets[0].id : '';
    if (sets.length > 0) {
      const devices = section(t('devices'));
      const choose = el('select', { class: 'tb-select', title: t('iconSet') });
      for (const set of sets) {
        const o = el('option', { value: set.id });
        o.textContent = setLabel(set);
        choose.appendChild(o);
      }
      choose.value = paletteSet;
      choose.addEventListener('change', () => {
        paletteSet = choose.value;
        buildPalette();
      });
      palette.insertBefore(choose, devices);
      const set = icons.sets.get(paletteSet);
      for (const kind of set.kinds) {
        const item = { id: `${set.id}/${kind}`, w: set.size ?? 60, h: set.size ?? 60 };
        const node = el('div', { class: 'pal-item', title: kindLabel(kind) });
        node.appendChild(el('img', { src: `/assets/diagram/icons/${item.id}.svg`, alt: '', draggable: 'false' }));
        node.addEventListener('click', () => insertIcon(item.id));
        dragFromPalette(node, item.w, item.h, (cx, cy) => insertIcon(item.id, cx, cy));
        devices.appendChild(node);
      }
    }
    const basics = section(t('basics'));
    for (const id of ['rect', 'rounded', 'text', 'container', 'cloud', 'note']) {
      const item = SHAPE_ITEMS.find((i) => i.id === id);
      if (item) add(basics, item, item.w, item.h, insertShape);
    }
    for (const item of LINE_ITEMS) add(basics, item, 120, 1, insertLine);
    return;
  }
  if (mermaid) {
    // Mermaid モード: Mermaid のフローチャートにある図形だけ (線は図形からドラッグして引く)
    const parent = section(t('shapes'));
    const seen = new Set();
    for (const item of SHAPE_ITEMS.filter((i) => i.mermaid)) {
      if (seen.has(item.id)) continue;
      seen.add(item.id);
      add(parent, item, item.w, item.h, insertShape);
    }
    return;
  }
  for (const [groupId, title] of [['shapes', t('shapes')], ['flow', t('flow')]]) {
    const parent = section(title);
    for (const item of SHAPE_ITEMS.filter((i) => i.group === groupId)) add(parent, item, item.w, item.h, insertShape);
  }
  const lines = section(t('lines'));
  for (const item of LINE_ITEMS) add(lines, item, 120, 1, insertLine);
}

// ---------------------------------------------------------------------------
// ポップアップ (色・右クリックメニュー)
// ---------------------------------------------------------------------------

let popup = null;

function closePopup() {
  if (popup) {
    popup.remove();
    popup = null;
  }
}

function openPopup(node, x, y) {
  closePopup();
  popup = node;
  document.body.appendChild(node);
  const r = node.getBoundingClientRect();
  node.style.left = `${Math.max(4, Math.min(x, window.innerWidth - r.width - 4))}px`;
  node.style.top = `${Math.max(4, Math.min(y, window.innerHeight - r.height - 4))}px`;
}

function showColors(anchor, allowNone, onPick) {
  const box = el('div', { class: 'popup' });
  const grid = el('div', { class: 'colors' });
  const pick = (color) => {
    closePopup();
    onPick(color);
    container.focus();
  };
  if (allowNone) {
    const none = el('button', { class: 'c none', title: t('none'), type: 'button' });
    none.addEventListener('click', () => pick('none'));
    grid.appendChild(none);
  }
  for (const color of COLORS) {
    const c = el('button', { class: 'c', title: color, type: 'button' });
    c.style.background = color;
    c.addEventListener('click', () => pick(color));
    grid.appendChild(c);
  }
  box.appendChild(grid);
  const custom = el('label', { class: 'color-custom' });
  const input = el('input', { type: 'color', value: '#3d85c6' });
  input.addEventListener('change', () => pick(input.value));
  custom.appendChild(input);
  custom.appendChild(document.createTextNode(t('custom')));
  box.appendChild(custom);
  const r = anchor.getBoundingClientRect();
  openPopup(box, r.left, r.bottom + 2);
}

/** 確認の小さな窓。「はい」に当たるボタンを押したら onOk を呼ぶ (外を押すか取り消しで閉じるだけ) */
function confirmBox(message, okLabel, onOk) {
  const box = el('div', { class: 'popup confirm' });
  box.appendChild(document.createTextNode(message));
  const buttons = el('div', { class: 'buttons' });
  const cancel = el('button', { class: 'code-btn', type: 'button' });
  cancel.textContent = t('cancel');
  cancel.addEventListener('click', closePopup);
  const ok = el('button', { class: 'code-btn primary', type: 'button' });
  ok.textContent = okLabel;
  ok.addEventListener('click', () => {
    closePopup();
    onOk();
  });
  buttons.appendChild(cancel);
  buttons.appendChild(ok);
  box.appendChild(buttons);
  openPopup(box, 0, 0);
  const r = box.getBoundingClientRect();
  box.style.left = `${Math.max(4, (window.innerWidth - r.width) / 2)}px`;
  box.style.top = `${Math.max(4, (window.innerHeight - r.height) / 3)}px`;
}

function showContextMenu(x, y) {
  const count = graph.getSelectionCount();
  const menu = el('div', { class: 'popup menu' });
  const item = (label, key, enabled, run) => {
    const node = el('div', { class: `item${enabled ? '' : ' disabled'}` });
    node.appendChild(document.createTextNode(label));
    const k = el('span', { class: 'key' });
    k.textContent = key;
    node.appendChild(k);
    node.addEventListener('click', () => {
      closePopup();
      run();
      container.focus();
    });
    menu.appendChild(node);
  };
  const rule = () => menu.appendChild(el('hr'));
  const any = count > 0;
  item(t('cut'), 'Ctrl+X', any, () => command('cut'));
  item(t('copy'), 'Ctrl+C', any, () => command('copy'));
  item(t('paste'), 'Ctrl+V', true, requestPaste);
  item(t('duplicate'), 'Ctrl+D', any, duplicateSelection);
  item(t('delete'), 'Del', any, deleteSelection);
  rule();
  item(t('editText'), 'F2', count === 1, () => graph.startEditingAtCell(graph.getSelectionCell()));
  rule();
  if (!mermaid) {
    // 重なり順とグループは Mermaid に無い
    item(t('toFront'), '', any, () => graph.orderCells(false, selected()));
    item(t('toBack'), '', any, () => graph.orderCells(true, selected()));
    rule();
    item(t('group'), 'Ctrl+G', count > 1, groupSelection);
    item(t('ungroup'), 'Ctrl+Shift+G', selected().some((c) => c.isVertex() && c.getChildCount() > 0), ungroupSelection);
    rule();
  }
  item(t('selectAll'), 'Ctrl+A', true, () => graph.selectAll());
  openPopup(menu, x, y);
}

/** 貼り付け: Rust が覚えている内容をもらう (Rust が無ければこのページでコピーしたもの) */
function requestPaste() {
  if (window.typball_host) post('paste');
  else pasteXml(localClip);
}

// ---------------------------------------------------------------------------
// キーボードとマウス
// ---------------------------------------------------------------------------

let spaceDown = false;

function isTextTarget(target) {
  if (!target || !target.tagName) return false;
  // キャンバスの代わりにキーを受けている入力欄は、キャンバスとして扱う
  if (target === imeCatcher) return false;
  const tag = target.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable;
}

// ---------------------------------------------------------------------------
// IME
// ---------------------------------------------------------------------------
//
// 図形を選んで文字のキーを押すと、その文字から入力が始まる。ただしブラウザは、文字を打てる欄にフォーカスが無いと
// IME を働かせない。キャンバス (ただの div) にフォーカスがあると、IME を有効にしていても 1 文字目だけ半角英字の
// まま入り、2 文字目から (入力欄ができてから) IME が効く。
// そこで、図形を 1 つ選んでいる間は、見えない入力欄 (#ime-catcher) にフォーカスを置いておく:
// - IME が無効なら: キーは今までどおり届き、onKeyDown が文字の編集を始める (押した文字は、できた入力欄に入る)
// - IME が有効なら: 変換がこの欄で始まる。変換している間だけ欄を図形の文字の位置に見せ、確定したら、
//   その文字を入れた状態で図形の文字の編集を始める (続きは、ふつうの入力欄で打つ)
// 選んでいる図形が 1 つでないときは、欄を読み取り専用にして IME を働かせない (打っても何も起きないので)

/** キャンバスの代わりにキーを受ける入力欄 */
const imeCatcher = document.getElementById('ime-catcher');
/** IME で変換している途中の図形 (変換していなければ null) */
let imeCell = null;

/** いま文字のキーで編集を始められる図形 (無ければ null) */
function typingCell() {
  if (!graph || readOnly || graph.isEditing() || graph.getSelectionCount() !== 1) return null;
  const cell = graph.getSelectionCell();
  return cell && graph.isCellEditable(cell) ? cell : null;
}

/** 入力欄を、選んでいる図形の文字の位置に合わせる (変換の候補の窓が、最初からそこに出るように) */
function syncImeCatcher() {
  if (!imeCatcher || !graph || imeCell) return;
  const cell = typingCell();
  imeCatcher.readOnly = !cell;
  const state = cell ? graph.view.getState(cell) : null;
  if (!state) return;
  const s = graph.view.scale;
  const style = graph.getCurrentCellStyle(cell);
  const font = (style.fontSize ?? 12) * s;
  const line = font * 1.3;
  let box = { x: state.x, y: state.y, w: state.width, h: state.height };
  if (cell.isEdge()) {
    // 線の文字は、線の途中 (ラベルの位置) に出る
    const at = state.absoluteOffset ?? { x: state.getCenterX(), y: state.getCenterY() };
    box = { x: at.x - 80 * s, y: at.y - line / 2, w: 160 * s, h: line };
  } else if (style.verticalLabelPosition === 'bottom') {
    // 機器のアイコンなど: 文字は図形の下
    box = { x: state.x - 40 * s, y: state.y + state.height, w: state.width + 80 * s, h: line };
  } else if (style.shape === 'swimlane') {
    // 枠: 文字は上の帯
    box.h = (style.startSize ?? 24) * s;
  }
  const css = imeCatcher.style;
  css.left = `${Math.round(box.x)}px`;
  css.top = `${Math.round(box.y)}px`;
  css.width = `${Math.max(Math.round(box.w), 24)}px`;
  css.height = `${Math.max(Math.round(box.h), Math.ceil(line))}px`;
  css.fontSize = `${font}px`;
  css.textAlign = style.align ?? 'center';
  css.padding = `${Math.max(0, Math.floor((box.h - line) / 2))}px 2px 0`;
}

function bindIme() {
  if (!imeCatcher) return;
  // キャンバスにフォーカスが来たら、入力欄へ回す (文字の編集中は、その入力欄がフォーカスを持つ)。
  // クリックで来たフォーカスは focus イベントで、プログラムからの container.focus() (maxGraph の中のものも) は
  // メソッドの差し替えで回す
  const toCatcher = () => {
    if (graph && !graph.isEditing()) imeCatcher.focus({ preventScroll: true });
  };
  container.addEventListener('focus', toCatcher);
  const nativeFocus = container.focus.bind(container);
  container.focus = (options) => {
    if (graph && !graph.isEditing()) toCatcher();
    else nativeFocus(options);
  };
  // 文字の編集が終わったら、フォーカスを戻す (そのまま次の文字を打てるように)。ほかの欄 (ポートの表など) を
  // クリックして終わったときは、その欄から奪わない
  graph.addListener(mx.InternalEvent.EDITING_STOPPED, () => {
    setTimeout(() => {
      const active = document.activeElement;
      if (!active || active === document.body || container.contains(active)) toCatcher();
    }, 0);
  });
  imeCatcher.addEventListener('compositionstart', () => {
    syncImeCatcher();
    imeCell = typingCell();
    if (imeCell) imeCatcher.classList.add('on');
  });
  imeCatcher.addEventListener('compositionend', (e) => {
    const cell = imeCell;
    const text = imeCatcher.value || e.data || '';
    imeCell = null;
    imeCatcher.classList.remove('on');
    imeCatcher.value = '';
    if (!cell || !text || readOnly || !graph.getDataModel().contains(cell)) {
      syncImeCatcher();
      return;
    }
    // 確定した文字を入れた状態で、編集を始める。編集を始めると今の文字が全部選ばれているので、確定した文字で
    // 置き換わる (文字のキーで編集を始めたときと同じ)。カーソルはその後ろに来る
    graph.startEditingAtCell(cell);
    if (graph.isEditing()) document.execCommand('insertText', false, text);
  });
  // 変換を通らずに欄に入った文字は捨てる (ここに文字をためない)
  imeCatcher.addEventListener('input', (e) => {
    if (!e.isComposing && !imeCell) imeCatcher.value = '';
  });
}

function onKeyDown(e) {
  // IME が処理しているキー (変換中の Enter や矢印など) には触らない
  if (e.isComposing || e.keyCode === 229) return;
  const ctrl = e.ctrlKey || e.metaKey;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  const editing = graph.isEditing();

  if (key === 'Escape') {
    if (popup) closePopup();
    else if (editing) graph.stopEditing(true);
    else {
      graph.escape(e);
      graph.clearSelection();
    }
    e.preventDefault();
    return;
  }

  // アプリのショートカット (保存など) は Rust へ。ブラウザ自身のショートカット (再読み込み・印刷・検索) は止める
  const combo = `${ctrl ? 'ctrl+' : ''}${e.shiftKey ? 'shift+' : ''}${e.altKey ? 'alt+' : ''}${key}`;
  const forwarded = ['ctrl+s', 'ctrl+shift+s', 'ctrl+n', 'ctrl+shift+n', 'ctrl+o', 'ctrl+w', 'ctrl+e', 'ctrl+tab', 'ctrl+shift+tab'];
  if (forwarded.includes(combo)) {
    e.preventDefault();
    // 入力中の文字を確定した結果 (新しい SVG) を先に渡してから、保存などを頼む
    commitEditing();
    post(`key:${combo}`);
    return;
  }
  const blocked = ['ctrl+r', 'ctrl+shift+r', 'ctrl+p', 'ctrl+f', 'ctrl+h', 'ctrl+j', 'ctrl+l', 'ctrl+u', 'ctrl+=', 'ctrl+-', 'ctrl+0', 'F3', 'F5', 'F7', 'F12', 'ctrl+shift+i'];
  if (blocked.includes(combo) || blocked.includes(key)) {
    e.preventDefault();
    return;
  }

  if (editing || isTextTarget(e.target)) return;
  if (readOnly) return;

  if (key === ' ') {
    spaceDown = true;
    container.style.cursor = 'grab';
    e.preventDefault();
    return;
  }

  let handled = true;
  if (ctrl) {
    switch (combo) {
      case 'ctrl+z': undo(); break;
      case 'ctrl+y':
      case 'ctrl+shift+z': redo(); break;
      case 'ctrl+c': copySelection(); break;
      case 'ctrl+x': if (copySelection()) deleteSelection(); break;
      case 'ctrl+v': requestPaste(); break;
      case 'ctrl+d': duplicateSelection(); break;
      case 'ctrl+a': graph.selectAll(); break;
      case 'ctrl+g': groupSelection(); break;
      case 'ctrl+shift+g': ungroupSelection(); break;
      case 'ctrl+b': if (!mermaid) toggleFontFlag(1); break;
      case 'ctrl+i': if (!mermaid) toggleFontFlag(2); break;
      default: handled = false;
    }
  } else {
    const step = e.shiftKey ? 1 : GRID;
    switch (key) {
      case 'Delete':
      case 'Backspace': deleteSelection(); break;
      case 'ArrowLeft': nudge(-step, 0); break;
      case 'ArrowRight': nudge(step, 0); break;
      case 'ArrowUp': nudge(0, -step); break;
      case 'ArrowDown': nudge(0, step); break;
      case 'F2':
      case 'Enter':
        if (graph.getSelectionCount() === 1) graph.startEditingAtCell(graph.getSelectionCell());
        break;
      default:
        handled = false;
        // 文字のキーを押したら、選んでいる図形の文字の編集を始める (押した文字から入力される)
        if (key.length === 1 && !e.altKey && graph.getSelectionCount() === 1) {
          graph.startEditingAtCell(graph.getSelectionCell());
        }
    }
  }
  if (handled) e.preventDefault();
}

function onWheel(e) {
  if (popup) closePopup();
  e.preventDefault();
  const rect = container.getBoundingClientRect();
  // deltaMode 1 = 行単位 (マウスによる)
  const unit = e.deltaMode === 1 ? 32 : 1;
  if (e.ctrlKey) {
    const factor = Math.exp(-(e.deltaY * unit) / 400);
    zoomAt(graph.view.scale * factor, e.clientX - rect.left, e.clientY - rect.top);
    return;
  }
  const s = graph.view.scale;
  const tr = graph.view.translate;
  let dx = e.deltaX * unit;
  let dy = e.deltaY * unit;
  if (e.shiftKey && dx === 0) [dx, dy] = [dy, 0];
  graph.view.setTranslate(tr.x - dx / s, tr.y - dy / s);
}

function bindInput() {
  bindIme();
  document.addEventListener('keydown', onKeyDown);
  document.addEventListener('keyup', (e) => {
    if (e.key === ' ') {
      spaceDown = false;
      container.style.cursor = '';
    }
  });
  window.addEventListener('blur', () => {
    spaceDown = false;
    container.style.cursor = '';
  });
  container.addEventListener('wheel', onWheel, { passive: false });
  // ファイルを落とされても、WebView がそのファイルを開いてページを置き換えないようにする
  for (const name of ['dragover', 'drop']) {
    document.addEventListener(name, (e) => e.preventDefault());
  }
  // クリックでキャンバスにフォーカスを移す (キー操作を受けるため)。ポップアップは外を押したら閉じる
  document.addEventListener(
    'pointerdown',
    (e) => {
      if (popup && !popup.contains(e.target)) closePopup();
      if (container.contains(e.target) && !graph.isEditing()) container.focus();
    },
    true
  );
  // 右クリック: 動かさずに離したらメニュー (右ドラッグは表示位置の移動)
  let rightDown = null;
  container.addEventListener('pointerdown', (e) => {
    rightDown = e.button === 2 ? { x: e.clientX, y: e.clientY } : null;
  });
  document.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    if (readOnly || !container.contains(e.target) || graph.isEditing()) return;
    if (rightDown && Math.hypot(e.clientX - rightDown.x, e.clientY - rightDown.y) > 4) return;
    const rect = container.getBoundingClientRect();
    const cell = graph.getCellAt(e.clientX - rect.left, e.clientY - rect.top);
    if (cell && !graph.isCellSelected(cell)) graph.setSelectionCell(cell);
    else if (!cell) graph.clearSelection();
    showContextMenu(e.clientX, e.clientY);
  });
  new ResizeObserver(() => {
    if (pendingCenter) centerView(true);
  }).observe(container);
}

// ---------------------------------------------------------------------------
// 起動
// ---------------------------------------------------------------------------

async function start() {
  // 折り返しの位置を測るので、図のフォントが読めてから始める (読めなくても始める)
  try {
    await Promise.all([document.fonts.load(`12px "${FONT}"`), document.fonts.load(`bold 12px "${FONT}"`)]);
  } catch (_) {
    // フォントが無くても動かす
  }
  await loadIconCatalog();
  graph = createGraph();
  buildChrome();
  bindInput();
  updateViewport();
  window.typballDiagram = { load, command, setOptions, buildSvg, graph };
  post('ready');
}

start().catch((e) => post(`error:${(e && e.stack) || e}`));
