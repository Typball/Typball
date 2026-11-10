//! メインウィンドウ (とビューア) の位置・サイズの保存と復元。
//!
//! - サイズ (`[window] width / height`) は **DIP (拡大率 100% 換算)** で保存する。Windows (Per-Monitor V2) の
//!   wx はウィンドウのサイズを実ピクセルで扱うので、実ピクセルのまま保存すると、拡大率の違うモニターへ
//!   移ったとき (Windows が WM_DPICHANGED でウィンドウを拡大縮小する) に見た目と数値がずれ、
//!   保存・復元を繰り返すたびに大きくなっていく。DIP なら同じ見た目の大きさに戻る。
//!   GTK / macOS の wx は元から論理座標なので、`to_dip` / `from_dip` は何もしない。
//! - 初回起動 (サイズが未保存) は、置き先のモニターの作業領域 (タスクバーを除く) の縦横半分で中央に出す。
//! - 位置 (`x / y`) は画面 (仮想デスクトップ) の座標のまま。あえて画面からはみ出させた位置も尊重し、
//!   **タイトルバーをどのモニターでも掴めないときだけ** 一番近いモニターの中へ動かす
//!   (外したモニターの座標、古い版が最小化中に保存した -32000 など)。
//! - サイズは作業領域に合わせて縮めない (複数モニターにまたがる大きさもあり得る)。
//!   ただし全モニターを合わせた範囲より大きいものだけは、その範囲に縮める (壊れた値への備え)。
//! - 最小化したまま閉じたときは保存しない (位置が -32000 などになるため)。

use wxdragon::prelude::*;

use crate::settings;

/// 復元するときの最小サイズ (DIP)
const MIN_WIDTH: i32 = 400;
const MIN_HEIGHT: i32 = 300;
/// 初回起動の大きさ: 作業領域に対する割合 (%)
const INITIAL_PERCENT: i32 = 50;
/// 「掴める」とみなすタイトルバーの帯の高さと、見えていてほしい幅 (DIP)
const TITLE_BAR_HEIGHT: i32 = 30;
const GRAB_WIDTH: i32 = 100;

/// 画面上の矩形 (実ピクセル)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Area {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Area {
    fn from_rect(r: Rect) -> Area {
        Area {
            x: r.x,
            y: r.y,
            w: r.width,
            h: r.height,
        }
    }

    fn right(&self) -> i32 {
        self.x + self.w
    }

    fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// 重なる部分の幅と高さ (重ならなければ 0 以下)
    fn overlap(&self, other: &Area) -> (i32, i32) {
        (
            self.right().min(other.right()) - self.x.max(other.x),
            self.bottom().min(other.bottom()) - self.y.max(other.y),
        )
    }

    fn centre(&self) -> (i64, i64) {
        (
            self.x as i64 + self.w as i64 / 2,
            self.y as i64 + self.h as i64 / 2,
        )
    }
}

/// 作成直後 (表示前) のフレームに、保存されたサイズと位置を当てはめる。
/// `position` が None なら置き先のモニターの中央に出す。
/// フレームは `position` (無ければ既定の位置) で作っておくこと: そのモニターの拡大率で DIP を換算する
pub fn restore(frame: &Frame, saved: &settings::Window, position: Option<Point>) {
    let scale_before = frame.from_dip_int(1000);
    place(frame, saved, position);
    // 別のモニターへ動かしたことで拡大率が変わった場合、Windows がサイズを拡大縮小し直すので、
    // 移った先の拡大率でもう一度当てはめる (wx の wxTLWGeometryMSW::ApplyTo と同じ考え方)
    if frame.from_dip_int(1000) != scale_before {
        place(frame, saved, Some(frame.get_position()));
    }
}

fn place(frame: &Frame, saved: &settings::Window, position: Option<Point>) {
    let screens: Vec<Area> = Display::all()
        .map(|d| Area::from_rect(d.client_area()))
        .collect();
    let desktop: Vec<Area> = Display::all()
        .map(|d| Area::from_rect(d.geometry()))
        .collect();
    // フレームを作った場所のモニター (初回の大きさと、位置の指定が無いときの中央寄せに使う)
    let home = Display::from_window(frame)
        .or_else(|| Display::new(0))
        .map(|d| Area::from_rect(d.client_area()));
    let (Some(home), false) = (home, screens.is_empty()) else {
        // モニターの情報が取れない (通常は起きない): 保存値をそのまま当てる
        let size = frame.from_dip(Size::new(
            saved.width.unwrap_or(1000).max(MIN_WIDTH),
            saved.height.unwrap_or(700).max(MIN_HEIGHT),
        ));
        match position {
            Some(p) => frame.set_size_with_pos(p.x, p.y, size.width, size.height),
            None => frame.set_size(size),
        }
        return;
    };

    let min = frame.from_dip(Size::new(MIN_WIDTH, MIN_HEIGHT));
    let size = match (saved.width, saved.height) {
        (Some(w), Some(h)) => frame.from_dip(Size::new(w.max(MIN_WIDTH), h.max(MIN_HEIGHT))),
        _ => initial_size(home, min),
    };
    let size = cap_to_desktop(size, &desktop);
    let grab = (
        frame.from_dip_int(GRAB_WIDTH),
        frame.from_dip_int(TITLE_BAR_HEIGHT),
    );
    let r = position_window(&screens, home, size, position, grab);
    frame.set_size_with_pos(r.x, r.y, r.w, r.h);
}

/// 初回起動の大きさ: 作業領域の縦横 INITIAL_PERCENT % (最小サイズ未満にはしない)
fn initial_size(area: Area, min: Size) -> Size {
    Size::new(
        (area.w * INITIAL_PERCENT / 100).max(min.width),
        (area.h * INITIAL_PERCENT / 100).max(min.height),
    )
}

/// 全モニターを合わせた範囲 (外接矩形) より大きいサイズだけ、その範囲に縮める
fn cap_to_desktop(size: Size, desktop: &[Area]) -> Size {
    if desktop.is_empty() {
        return size;
    }
    let left = desktop.iter().map(|a| a.x).min().unwrap_or(0);
    let top = desktop.iter().map(|a| a.y).min().unwrap_or(0);
    let right = desktop.iter().map(|a| a.right()).max().unwrap_or(0);
    let bottom = desktop.iter().map(|a| a.bottom()).max().unwrap_or(0);
    Size::new(
        size.width.min(right - left).max(1),
        size.height.min(bottom - top).max(1),
    )
}

/// タイトルバー (上端の帯) が、どれかのモニターの作業領域で `grab` (幅, 高さ) 以上見えているか。
/// 帯の高さの半分以上・幅 `grab.0` 以上 (ウィンドウがそれより狭ければその幅) が重なっていれば掴める
fn title_bar_reachable(screens: &[Area], window: Area, (grab_w, title_h): (i32, i32)) -> bool {
    let bar = Area {
        h: title_h,
        ..window
    };
    let need_w = grab_w.min(window.w).max(1);
    let need_h = (title_h / 2).max(1);
    screens.iter().any(|s| {
        let (w, h) = bar.overlap(s);
        w >= need_w && h >= need_h
    })
}

/// 位置を決める。指定が無ければ `home` の中央。指定があればそのまま使い、タイトルバーを掴めないときだけ
/// 一番近いモニターの作業領域の中へ動かす (大きすぎてはみ出す分は右と下へ。タイトルバーは必ず見える)
fn position_window(
    screens: &[Area],
    home: Area,
    size: Size,
    position: Option<Point>,
    grab: (i32, i32),
) -> Area {
    let (w, h) = (size.width, size.height);
    let Some(p) = position else {
        return Area {
            x: home.x + (home.w - w) / 2,
            y: home.y + (home.h - h).max(0) / 2,
            w,
            h,
        };
    };
    let window = Area {
        x: p.x,
        y: p.y,
        w,
        h,
    };
    if title_bar_reachable(screens, window, grab) {
        return window;
    }
    let target = nearest(screens, window).unwrap_or(home);
    Area {
        x: p.x.min(target.right() - w).max(target.x),
        y: p.y.min(target.bottom() - h).max(target.y),
        w,
        h,
    }
}

/// ウィンドウの中心に一番近いモニター (中心どうしの距離)
fn nearest(screens: &[Area], window: Area) -> Option<Area> {
    let (cx, cy) = window.centre();
    screens.iter().copied().min_by_key(|s| {
        let (sx, sy) = s.centre();
        (sx - cx).pow(2) + (sy - cy).pow(2)
    })
}

/// 閉じる前のフレームの位置とサイズを `saved` に書き込む (サイズは DIP)。
/// 最大化中は最大化の印だけ更新し、元に戻したときの位置とサイズは前回の値を残す。最小化中は何もしない
pub fn save(frame: &Frame, saved: &mut settings::Window) {
    if frame.is_iconized() {
        return;
    }
    saved.maximized = frame.is_maximized();
    if !saved.maximized {
        let size = frame.to_dip(frame.get_size());
        let pos = frame.get_position();
        saved.width = Some(size.width);
        saved.height = Some(size.height);
        saved.x = Some(pos.x);
        saved.y = Some(pos.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Area = Area {
        x: 0,
        y: 0,
        w: 1920,
        h: 1032,
    };
    /// 右隣のモニター (上下にずれている)
    const RIGHT: Area = Area {
        x: 1920,
        y: -200,
        w: 2560,
        h: 1400,
    };
    const GRAB: (i32, i32) = (100, 30);

    fn at(x: i32, y: i32) -> Option<Point> {
        Some(Point::new(x, y))
    }

    #[test]
    fn initial_size_is_half_of_the_work_area() {
        assert_eq!(initial_size(MAIN, Size::new(400, 300)), Size::new(960, 516));
        // 小さい画面でも最小サイズ (175% なら 700x525 の実ピクセル) は下回らない
        let small = Area {
            x: 0,
            y: 0,
            w: 1366,
            h: 728,
        };
        assert_eq!(
            initial_size(small, Size::new(700, 525)),
            Size::new(700, 525)
        );
    }

    #[test]
    fn centres_without_a_position() {
        let r = position_window(&[MAIN], MAIN, Size::new(960, 516), None, GRAB);
        assert_eq!(
            r,
            Area {
                x: 480,
                y: 258,
                w: 960,
                h: 516
            }
        );
    }

    #[test]
    fn keeps_a_window_that_sticks_out_on_purpose() {
        // 左へ半分以上はみ出している / 下へはみ出している / 画面より大きい: タイトルバーが見えていればそのまま
        for (x, y, w, h) in [
            (-700, 100, 1000, 700),
            (300, 800, 1000, 700),
            (-50, 0, 4000, 3000),
        ] {
            let r = position_window(&[MAIN], MAIN, Size::new(w, h), at(x, y), GRAB);
            assert_eq!(r, Area { x, y, w, h }, "({x}, {y}, {w}, {h})");
        }
        // 2 枚のモニターにまたがる
        let r = position_window(
            &[MAIN, RIGHT],
            MAIN,
            Size::new(1500, 900),
            at(1200, 100),
            GRAB,
        );
        assert_eq!(
            r,
            Area {
                x: 1200,
                y: 100,
                w: 1500,
                h: 900
            }
        );
    }

    #[test]
    fn moves_a_window_whose_title_bar_cannot_be_grabbed() {
        // 最小化中に保存された -32000
        let r = position_window(&[MAIN], MAIN, Size::new(900, 600), at(-32000, -32000), GRAB);
        assert_eq!(
            r,
            Area {
                x: 0,
                y: 0,
                w: 900,
                h: 600
            }
        );
        // タイトルバーが画面の上へ出ている
        let r = position_window(&[MAIN], MAIN, Size::new(900, 600), at(200, -100), GRAB);
        assert_eq!(
            r,
            Area {
                x: 200,
                y: 0,
                w: 900,
                h: 600
            }
        );
        // 左にほぼ出ていて、見えているタイトルバーが 100 DIP 未満
        let r = position_window(&[MAIN], MAIN, Size::new(900, 600), at(-850, 100), GRAB);
        assert_eq!(
            r,
            Area {
                x: 0,
                y: 100,
                w: 900,
                h: 600
            }
        );
        // 外したモニター (右隣があった場所) の座標: 一番近い MAIN の右端へ寄せる
        let r = position_window(&[MAIN], MAIN, Size::new(900, 600), at(2500, 300), GRAB);
        assert_eq!(
            r,
            Area {
                x: 1020,
                y: 300,
                w: 900,
                h: 600
            }
        );
        // 画面より大きいウィンドウは左上に合わせる (タイトルバーが見える)
        let r = position_window(&[MAIN], MAIN, Size::new(3000, 2000), at(-5000, 0), GRAB);
        assert_eq!(
            r,
            Area {
                x: 0,
                y: 0,
                w: 3000,
                h: 2000
            }
        );
    }

    #[test]
    fn moves_to_the_nearest_monitor() {
        // 右隣のモニターのさらに右: RIGHT の中へ
        let r = position_window(
            &[MAIN, RIGHT],
            MAIN,
            Size::new(800, 600),
            at(6000, 100),
            GRAB,
        );
        assert_eq!(
            r,
            Area {
                x: 1920 + 2560 - 800,
                y: 100,
                w: 800,
                h: 600
            }
        );
    }

    #[test]
    fn caps_only_beyond_all_monitors() {
        let desktop = [MAIN, RIGHT];
        // 2 枚分 (幅 4480・高さ 1400 の外接矩形 -200..1200) までは縮めない
        assert_eq!(
            cap_to_desktop(Size::new(4000, 1300), &desktop),
            Size::new(4000, 1300)
        );
        assert_eq!(
            cap_to_desktop(Size::new(9000, 5000), &desktop),
            Size::new(4480, 1400)
        );
    }
}
