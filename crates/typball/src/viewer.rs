//! プレビュー専用ウィンドウ (`Typball.exe --view ファイル`、ファイル → プレビューウィンドウで開く)
//!
//! ```text
//! frame (Frame)
//! ├ bar (Panel): ファイル名 ............ [編集]
//! ├ body: Typst はページ画像 (preview.rs) / Markdown は WebView (webpreview.rs) / テキストは読み取り専用
//! └ ステータスバー (Typst の診断)
//! ```
//!
//! エディタを持たないので、ファイルは読むだけ。2 秒ごとに更新時刻を見て、外部で書き換わったら
//! 読み直して描き直す (別のエディタで編集しながら眺める使い方)。「編集」を押すと同じプロセスの
//! エディタウィンドウでそのファイルを開き (無ければ作り)、このウィンドウは閉じる。
//! Windows の関連付けでは `installer/typball.iss` が「Typball でプレビュー」の動詞 (`--view`) を登録する。

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use wxdragon::prelude::*;

use crate::document::Document;
use crate::filetype::FileType;
use crate::i18n::{tr, tr_args};
use crate::preview::{paint_preview, render_preview, Preview, SCROLL_UNIT};
use crate::settings;
use crate::webpreview::{self, WebPreview};
use crate::window;

/// 外部変更を確認する間隔 (ms)
const EXTERNAL_CHECK_MS: i32 = 2000;
/// サイズ変更後に組み直すまでの待ち時間 (ms)
const RENDER_DELAY_MS: i32 = 20;
/// wxSTB_SIZEGRIP | wxSTB_SHOW_TIPS | wxSTB_ELLIPSIZE_END | wxFULL_REPAINT_ON_RESIZE
const STATUS_BAR_STYLE: i64 = 0x0010 | 0x0020 | 0x0100 | 0x0001_0000;

enum Content {
    Typst(Rc<RefCell<Preview>>),
    Web(Rc<WebPreview>),
    Text(TextCtrl),
}

pub struct Viewer {
    pub frame: Frame,
    doc: RefCell<Document>,
    text: RefCell<String>,
    content: Content,
    /// 外部変更の監視
    _watch: Timer<Frame>,
    /// サイズ変更後の組み直し (Typst)
    render_timer: Timer<Frame>,
}

thread_local! {
    static VIEWERS: RefCell<Vec<Rc<Viewer>>> = const { RefCell::new(Vec::new()) };
}

/// 各ファイルをプレビューウィンドウで開く。開けた数を返す (読めなかったファイルはダイアログで知らせる)
pub fn open(paths: &[PathBuf]) -> usize {
    paths.iter().filter(|p| open_one(p).is_some()).count()
}

/// 1 つのファイルをプレビューウィンドウで開く。すでに開いていればそのウィンドウを前に出す
pub fn open_one(path: &Path) -> Option<Rc<Viewer>> {
    let existing = VIEWERS.with(|v| {
        v.borrow()
            .iter()
            .find(|viewer| viewer.doc.borrow().path.as_deref() == Some(path))
            .cloned()
    });
    if let Some(viewer) = existing {
        viewer.frame.raise();
        return Some(viewer);
    }
    match Document::open(path) {
        Ok((doc, text)) => Some(Viewer::new(doc, text)),
        Err(e) => {
            MessageDialog::builder(
                &Frame::builder().build(),
                &tr_args(
                    "dialog.open_error",
                    &[
                        ("path", &path.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                ),
                "Typball",
            )
            .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
            .build()
            .show_modal();
            None
        }
    }
}

impl Viewer {
    fn new(doc: Document, text: String) -> Rc<Viewer> {
        let settings = settings::get();
        let frame = Frame::builder().with_title(&window_title(&doc)).build();
        // メインウィンドウと同じ大きさ (DIP) で、画面の中央に出す
        crate::winplace::restore(&frame, &settings.window, None);
        crate::appicon::apply(&frame);
        crate::window::watch_system_theme(&frame);
        let status = frame.create_status_bar(1, STATUS_BAR_STYLE, -1, "status");
        let _ = status;

        // 上のバー: ファイル名と「編集」
        let bar = Panel::builder(&frame).build();
        let name = StaticText::builder(&bar)
            .with_label(
                &doc.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            )
            .build();
        let edit = Button::builder(&bar).with_label(&tr("viewer.edit")).build();
        edit.set_tooltip(&tr("viewer.edit_tip"));
        let hbox = BoxSizer::builder(Orientation::Horizontal).build();
        let center_all = SizerFlag::AlignCenterVertical | SizerFlag::All;
        hbox.add(&name, 1, center_all, frame.from_dip_int(6));
        hbox.add(&edit, 0, center_all, frame.from_dip_int(4));
        bar.set_sizer(hbox, true);

        let body = Panel::builder(&frame).build();
        let content = match doc.filetype {
            FileType::Typ => {
                let win = ScrolledWindow::builder(&body).build();
                win.set_scroll_rate(SCROLL_UNIT, SCROLL_UNIT);
                win.set_background_color(crate::preview::bg());
                win.set_background_style(BackgroundStyle::Paint);
                fill(&body, &win);
                Content::Typst(Rc::new(RefCell::new(Preview {
                    scrolled: win,
                    bitmap: None,
                    size: (0, 0),
                    rendered_width: -1,
                    frame,
                })))
            }
            FileType::Md => {
                let web = WebPreview::new(&body, frame, doc.path.as_deref());
                fill(&body, &web.container);
                Content::Web(web)
            }
            FileType::Txt => {
                let ctrl = TextCtrl::builder(&body)
                    .with_style(
                        TextCtrlStyle::MultiLine
                            | TextCtrlStyle::ReadOnly
                            | TextCtrlStyle::DontWrap
                            | TextCtrlStyle::Rich2,
                    )
                    .build();
                if let Some(font) = preview_font(&settings) {
                    ctrl.set_font(&font);
                }
                fill(&body, &ctrl);
                Content::Text(ctrl)
            }
        };

        let root = BoxSizer::builder(Orientation::Vertical).build();
        root.add(&bar, 0, SizerFlag::Expand, 0);
        root.add(&body, 1, SizerFlag::Expand, 0);
        frame.set_sizer(root, true);

        let viewer = Rc::new(Viewer {
            frame,
            doc: RefCell::new(doc),
            text: RefCell::new(text),
            content,
            _watch: Timer::new(&frame),
            render_timer: Timer::new(&frame),
        });
        VIEWERS.with(|v| v.borrow_mut().push(viewer.clone()));
        viewer.bind(&edit);
        frame.layout();
        frame.show(true);
        viewer.render();
        viewer
    }

    fn bind(self: &Rc<Self>, edit: &Button) {
        let weak = Rc::downgrade(self);
        edit.on_click({
            let weak = weak.clone();
            move |_| {
                if let Some(v) = weak.upgrade() {
                    v.open_in_editor();
                }
            }
        });
        // Esc で閉じる
        self.frame.on_char_hook({
            let weak = weak.clone();
            move |event| {
                if let (Some(v), WindowEventData::Keyboard(key)) = (weak.upgrade(), &event) {
                    if key.get_key_code() == Some(WXK_ESCAPE) {
                        v.frame.close(true);
                        return;
                    }
                }
                event.skip(true);
            }
        });
        self.frame.on_close({
            let weak = weak.clone();
            move |event| {
                if let Some(v) = weak.upgrade() {
                    VIEWERS.with(|list| list.borrow_mut().retain(|x| !Rc::ptr_eq(x, &v)));
                }
                event.skip(true);
            }
        });
        // 外部変更: 書き換わったら読み直す
        self._watch.on_tick({
            let weak = weak.clone();
            move |_| {
                if let Some(v) = weak.upgrade() {
                    v.check_external();
                }
            }
        });
        self._watch.start(EXTERNAL_CHECK_MS, false);
        self.render_timer.on_tick({
            let weak = weak.clone();
            move |_| {
                if let Some(v) = weak.upgrade() {
                    v.render();
                }
            }
        });
        match &self.content {
            Content::Typst(state) => {
                let win = state.borrow().scrolled;
                win.on_paint({
                    let state = state.clone();
                    move |_| paint_preview(&state)
                });
                win.on_erase_background(|_| {});
                // 幅が変わると収まる解像度が変わるので組み直す
                win.on_size({
                    let weak = weak.clone();
                    let state = state.clone();
                    move |_| {
                        let changed = state
                            .try_borrow()
                            .map(|st| st.rendered_width != win.get_client_size().width)
                            .unwrap_or(false);
                        if changed {
                            if let Some(v) = weak.upgrade() {
                                v.render_timer.start(RENDER_DELAY_MS, true);
                            }
                        }
                    }
                });
            }
            Content::Web(web) => {
                // 外部リンクはブラウザで、相対パスの .md などは別のプレビューウィンドウで
                let weak = weak.clone();
                web.on_message(move |msg| {
                    if let Some(url) = msg.strip_prefix("open:") {
                        webpreview::open_external(url);
                    } else if let Some(rel) = msg.strip_prefix("file:") {
                        let rel = crate::mdhtml::percent_decode(
                            rel.split(['?', '#']).next().unwrap_or(rel),
                        );
                        let base = weak.upgrade().and_then(|v| v.doc.borrow().path.clone());
                        let target = match base.as_ref().and_then(|p| p.parent()) {
                            Some(dir) => dir.join(&rel),
                            None => PathBuf::from(&rel),
                        };
                        if target.is_file() {
                            open_one(&target);
                        }
                    }
                });
            }
            Content::Text(_) => {}
        }
    }

    /// 今のテキストを描く
    fn render(&self) {
        let text = self.text.borrow().clone();
        match &self.content {
            Content::Typst(state) => {
                render_preview(&text, state);
            }
            Content::Web(web) => {
                let path = self.doc.borrow().path.clone();
                web.set_markdown(&text, path.as_deref());
            }
            Content::Text(ctrl) => ctrl.set_value(&text),
        }
    }

    fn check_external(&self) {
        if !self.doc.borrow().changed_externally() {
            return;
        }
        let name = self.doc.borrow().title();
        let reloaded = self.doc.borrow_mut().reload();
        match reloaded {
            Ok(text) => {
                *self.text.borrow_mut() = text;
                self.render();
                self.frame
                    .set_status_text(&tr_args("status.reloaded", &[("name", &name)]), 0);
            }
            Err(_) => {
                self.doc.borrow_mut().acknowledge_external();
                self.frame
                    .set_status_text(&tr_args("status.external_deleted", &[("name", &name)]), 0);
            }
        }
    }

    /// エディタで開いてこのウィンドウを閉じる
    fn open_in_editor(&self) {
        let Some(path) = self.doc.borrow().path.clone() else {
            return;
        };
        match window::all_windows().into_iter().next() {
            Some(win) => {
                window::open_file(&win, &path);
                win.borrow().frame.raise();
            }
            None => {
                window::create(std::slice::from_ref(&path), None);
            }
        }
        self.frame.close(true);
    }
}

/// プレビューのフォント設定 (テキストファイルの表示用)
fn preview_font(settings: &settings::Settings) -> Option<Font> {
    Font::builder()
        .with_face_name(&settings.preview.font_face)
        .with_point_size(settings.preview.font_size.clamp(6, 72))
        .build()
}

/// 設定が変わった: プレビューのフォントが変わっていれば開いているプレビューウィンドウを描き直す
pub fn apply_settings(before: &settings::Settings, after: &settings::Settings) {
    if before.preview.font_face == after.preview.font_face
        && before.preview.font_size == after.preview.font_size
    {
        return;
    }
    let viewers: Vec<Rc<Viewer>> = VIEWERS.with(|v| v.borrow().clone());
    for viewer in viewers {
        match &viewer.content {
            Content::Typst(_) => viewer.render(),
            Content::Web(web) => web.reload_page(),
            Content::Text(ctrl) => {
                if let Some(font) = preview_font(after) {
                    ctrl.set_font(&font);
                }
            }
        }
    }
}

/// 開いているプレビューウィンドウ
pub fn frames() -> Vec<Frame> {
    VIEWERS.with(|v| v.borrow().iter().map(|x| x.frame).collect())
}

/// 配色の明暗が変わった: プレビューウィンドウを描き直す
pub fn apply_theme(changed: crate::theme::Changed) {
    if !changed.preview && !changed.ui {
        return;
    }
    let viewers: Vec<Rc<Viewer>> = VIEWERS.with(|v| v.borrow().clone());
    for viewer in viewers {
        if changed.preview {
            match &viewer.content {
                Content::Typst(_) => viewer.render(),
                Content::Web(web) => {
                    viewer.render();
                    web.reload_page();
                }
                Content::Text(_) => {}
            }
        }
        if changed.ui {
            viewer.frame.refresh(true, None);
        }
    }
}

/// "report.typ (プレビュー) - Typball"。末尾はメインウィンドウと同じ " - Typball"
fn window_title(doc: &Document) -> String {
    format!("{} ({}) - Typball", doc.title(), tr("viewer.title"))
}

/// `content` を `parent` いっぱいに置く
fn fill<W: WxWidget>(parent: &Panel, content: &W) {
    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add(content, 1, SizerFlag::Expand, 0);
    parent.set_sizer(sizer, true);
}
