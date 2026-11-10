//! 自前描画する部品 (タブバー・ペインのヘッダ・アクションバー) で共通の配色とフォント、
//! ドラッグ中に出すカーソルの生成

use wxdragon::dc::{Point as DcPoint, PolygonFillMode};
use wxdragon::prelude::*;

use crate::editor::rgb;

/// バーの背景 (ウィンドウの背景と同系の薄いグレー)
pub const BAR_BG: u32 = 0xf0f0f0;
/// バーやタブの境界線
pub const BORDER: u32 = 0xc8c8c8;
/// 選択されていないタブ
pub const TAB_INACTIVE: u32 = 0xe2e2e2;
/// マウスが乗っているタブ
pub const TAB_HOVER: u32 = 0xebebeb;
/// 選択中のタブ (ページと同じ白でつながって見える)
pub const TAB_ACTIVE: u32 = 0xffffff;
/// 通常の文字色
pub const TEXT: u32 = 0x1a1a1a;
/// 控えめな文字色 (ヘッダのタイトルなど)
pub const TEXT_DIM: u32 = 0x505050;
/// ボタンにマウスが乗ったときの背景
pub const HOVER_BG: u32 = 0xd6d6d6;
/// ボタンを押している間の背景
pub const PRESSED_BG: u32 = 0xc0c0c0;
/// 閉じるボタンにマウスが乗ったときの背景 (Windows 風の赤)
pub const CLOSE_HOVER: u32 = 0xe81123;
/// ペインのヘッダの背景
pub const HEADER_BG: u32 = 0xe8e8e8;
/// タブバーの挿入位置マーカーと ↓ バッジの色
pub const DROP_MARKER: u32 = 0x0b5cad;

/// ウィンドウの拡大率 (DPI / 96)。自前で描く部品の寸法・座標は DIP (拡大率 100% 換算) で書き、
/// 描画・当たり判定・サイズ指定のたびにこれで実ピクセルに直す。
///
/// Windows (Per-Monitor V2) の wx はウィンドウ座標が実ピクセルで、拡大率はモニターごとに違う。
/// 一方フォントは wx が描く先のウィンドウの DPI に合わせて大きくする (wxMSWDCImpl::SetFont の
/// WXAdjustToPPI) ので、寸法を固定ピクセルで書くと 175% のモニターでは文字だけが大きくなって
/// 枠からはみ出す (タブの "M" やアクションバーの "B" が欠けた)。拡大率はウィンドウが別の
/// モニターへ移ると変わるので、覚えておかずに使う直前に取る。GTK / macOS は論理座標なので常に 1
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dip(f64);

impl Dip {
    pub fn of<W: WxWidget>(window: &W) -> Dip {
        // from_dip_int は DPI / 96 を掛けて丸めるので、大きな値で割合を取る
        const BASE: i32 = 9600;
        let scaled = window.from_dip_int(BASE);
        Dip(if scaled > 0 {
            scaled as f64 / BASE as f64
        } else {
            1.0
        })
    }

    /// DIP → 実ピクセル
    pub fn px(self, v: i32) -> i32 {
        (v as f64 * self.0).round() as i32
    }

    /// DIP → 実ピクセル (小数。角丸の半径など)
    pub fn pxf(self, v: f64) -> f64 {
        v * self.0
    }

    /// 絵 (アイコン) の線の太さ。100% で 1px の線を拡大率に合わせて太らせる (150% 以上で 2px)
    pub fn pen(self) -> i32 {
        self.px(1).max(1)
    }
}

/// OS の既定 GUI フォントが取れなかったときの UI フォント: OS の標準的な日本語フォント
const SYSTEM_FONT_FACE: &str = if cfg!(target_os = "windows") {
    "Yu Gothic UI"
} else if cfg!(target_os = "macos") {
    "Hiragino Sans"
} else {
    "Noto Sans CJK JP"
};

/// UI 部品 (タブバー・ペインのヘッダ・アクションバーなど) のフォント。OS の既定の GUI フォント
/// (メニューと同じ。Windows 11 なら Yu Gothic UI 9pt) に合わせ、`delta_pt` でポイント数を増減する。
/// 設定のフォントはエディタの中身だけに使い、ここでは見ない
pub fn ui_font(bold: bool, italic: bool, delta_pt: i32) -> Option<Font> {
    let system = SystemSettings::get_font(SystemFont::DefaultGuiFont);
    let face = system
        .as_ref()
        .map(|f| f.get_face_name())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| SYSTEM_FONT_FACE.to_string());
    let base = system
        .as_ref()
        .map(|f| f.get_point_size())
        .filter(|pt| *pt > 0)
        .unwrap_or(9);
    Font::builder()
        .with_face_name(&face)
        .with_point_size((base + delta_pt).clamp(5, 24))
        .with_weight(if bold {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        })
        .with_style(if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        })
        .build()
}

/// 塗りつぶしの角丸矩形 (枠線無し)
pub fn fill_round_rect(
    dc: &dyn DeviceContext,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: f64,
    colour: u32,
) {
    dc.set_pen(rgb(colour), 1, PenStyle::Solid);
    dc.set_brush(rgb(colour), BrushStyle::Solid);
    dc.draw_rounded_rectangle(x, y, w, h, r);
}

/// × 印 (閉じるボタン)。`size` は × の一辺、`pen` は線の太さ (どちらも実ピクセル)
pub fn draw_cross(dc: &dyn DeviceContext, cx: i32, cy: i32, size: i32, pen: i32, colour: u32) {
    dc.set_pen(rgb(colour), pen, PenStyle::Solid);
    let h = size / 2;
    dc.draw_line(cx - h, cy - h, cx + h + 1, cy + h + 1);
    dc.draw_line(cx - h, cy + h, cx + h + 1, cy - h - 1);
}

/// 矩形に点が入っているか
pub fn contains(rect: &Rect, p: Point) -> bool {
    p.x >= rect.x && p.y >= rect.y && p.x < rect.x + rect.width && p.y < rect.y + rect.height
}

/// 矩形を四方に `margin` だけ広げる
pub fn inflate(rect: &Rect, margin: i32) -> Rect {
    Rect::new(
        rect.x - margin,
        rect.y - margin,
        rect.width + margin * 2,
        rect.height + margin * 2,
    )
}

/// マウスイベントの位置 (ボタン・移動のどちらのイベントでも取れる)
pub fn mouse_pos(event: &WindowEventData) -> Option<Point> {
    match event {
        WindowEventData::MouseButton(e) => e.get_position(),
        WindowEventData::MouseMotion(e) => e.get_position(),
        _ => None,
    }
}

/// 文字列が `max_width` に収まるように末尾を "…" で省略する
pub fn ellipsize<W: WxWidget>(window: &W, text: &str, max_width: i32) -> String {
    if window.get_text_extent(text).width <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 0 {
        n -= 1;
        let candidate: String = chars[..n].iter().collect::<String>() + "…";
        if window.get_text_extent(&candidate).width <= max_width {
            return candidate;
        }
    }
    "…".to_string()
}

// ---------------------------------------------------------------------------
// ドラッグ中のカーソル
// ---------------------------------------------------------------------------

/// ドラッグ先を表すバッジ (カーソルの右下に出す小さな丸)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
    /// バッジ無し (並べ替えなど、同じ場所の中での移動)
    None,
    /// 離すと新しいウィンドウができる (+)
    NewWindow,
    /// 離すとそこ (別のウィンドウのタブバーの、マーカーの位置) に入る (↓)
    Insert,
    /// ここでは離せない
    Forbidden,
}

/// カーソルの一辺 (DIP)
const CURSOR_SIZE: i32 = 36;
/// 透明にする色 (描画には使わない色)
const KEY: u32 = 0xff00ff;

/// 矢印の右下に `icon` とバッジを描いたカーソルを作る。ホットスポットは左上 (矢印の先)。
/// `icon(dc, x, y, size)` は左上 (x, y) に一辺 `size` px のアイコンを描く。
/// 大きさはドラッグしているウィンドウの拡大率 `dip` に合わせる (OS のカーソルも拡大率で大きくなる)。
/// マウスをつかんでいる間は [`show_drag_cursor`] で出すと、ウィンドウの外でもこのカーソルになる
pub fn drag_cursor(
    icon: &dyn Fn(&dyn DeviceContext, i32, i32, i32),
    badge: Badge,
    dip: Dip,
) -> Option<Cursor> {
    let d = |v: i32| dip.px(v);
    let size = d(CURSOR_SIZE);
    let pen = dip.pen();
    let mut bitmap = Bitmap::new(size, size)?;
    {
        let mut dc = MemoryDC::new();
        dc.select_object(&mut bitmap);
        dc.set_background(rgb(KEY));
        dc.clear();
        // 矢印
        dc.set_pen(rgb(0x000000), pen, PenStyle::Solid);
        dc.set_brush(rgb(0xffffff), BrushStyle::Solid);
        let o = pen / 2;
        let arrow = [
            (0, 0),
            (0, 15),
            (4, 11),
            (7, 17),
            (9, 16),
            (6, 10),
            (11, 10),
        ]
        .map(|(x, y)| DcPoint::new(o + d(x), o + d(y)));
        dc.draw_polygon(&arrow, 0, 0, PolygonFillMode::OddEven);
        // アイコン (白い角丸の台座に乗せる)
        fill_round_rect(&dc, d(12), d(12), d(22), d(22), dip.pxf(4.0), 0xffffff);
        dc.set_pen(rgb(BORDER), 1, PenStyle::Solid);
        dc.set_brush(rgb(0xffffff), BrushStyle::Transparent);
        dc.draw_rounded_rectangle(d(12), d(12), d(22), d(22), dip.pxf(4.0));
        icon(&dc, d(15), d(15), d(16));
        // バッジ
        let (cx, cy, r) = (d(29), d(29), d(6));
        let stroke = d(2).max(2);
        match badge {
            Badge::None => {}
            Badge::NewWindow => {
                circle(&dc, cx, cy, r, 0x2e8b57);
                dc.set_pen(rgb(0xffffff), stroke, PenStyle::Solid);
                dc.draw_line(cx - d(3), cy, cx + d(4), cy);
                dc.draw_line(cx, cy - d(3), cx, cy + d(4));
            }
            Badge::Insert => {
                circle(&dc, cx, cy, r, DROP_MARKER);
                dc.set_pen(rgb(0xffffff), stroke, PenStyle::Solid);
                dc.draw_line(cx, cy - d(3), cx, cy + d(4));
                dc.draw_line(cx - d(3), cy + d(1), cx, cy + d(4));
                dc.draw_line(cx + d(3), cy + d(1), cx, cy + d(4));
            }
            Badge::Forbidden => {
                circle(&dc, cx, cy, r, 0xc0392b);
                dc.set_pen(rgb(0xffffff), stroke, PenStyle::Solid);
                dc.draw_line(cx - d(3), cy - d(3), cx + d(4), cy + d(4));
            }
        }
    }
    // 下地の色を透明に、それ以外を不透明にする (アルファ無しのビットマップから取った
    // データはアルファが不定なので、ここで決め直す)
    let mut data = bitmap.get_rgba_data()?;
    for px in data.chunks_exact_mut(4) {
        let key = px[0] == 0xff && px[1] == 0x00 && px[2] == 0xff;
        px[3] = if key { 0 } else { 0xff };
    }
    let image = Bitmap::from_rgba(&data, size as u32, size as u32)?;
    Cursor::from_bitmap(&image)
}

fn circle(dc: &dyn DeviceContext, cx: i32, cy: i32, r: i32, colour: u32) {
    dc.set_pen(rgb(0xffffff), 1, PenStyle::Solid);
    dc.set_brush(rgb(colour), BrushStyle::Solid);
    dc.draw_circle(cx, cy, r);
}

/// ドラッグ中のカーソルを出す / 戻す (None)。ビジーカーソルの仕組みを借りるので、
/// マウスをつかんだままウィンドウの外へ出ても効く
pub fn show_drag_cursor(cursor: Option<&Cursor>) {
    if wxdragon::cursor::is_busy() {
        wxdragon::cursor::end_busy_cursor();
    }
    if let Some(c) = cursor {
        wxdragon::cursor::begin_busy_cursor(Some(c));
    }
}
