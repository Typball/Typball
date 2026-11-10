//! メインウィンドウ: フレーム・メニュー・タブの管理・ファイル操作・外部変更の監視
//!
//! ウィンドウは複数開ける (ファイル > 新しいウィンドウ / タブを新しいウィンドウへ移動 /
//! タブをバーの外へドラッグ)。すべてのウィンドウは同じプロセスにあり、[`all_windows`] で
//! 列挙できる。タブを別のウィンドウのタブバーへドロップするとそのウィンドウへ移る。
//! 「アクション」メニューは選択中のタブのファイル種別で中身が変わる。
//! タブの右クリックで、そのタブに対する操作 (閉じる・保存・読み直し・場所を開くなど) を出す。
//!
//! タブの帯は `tabbar.rs` の自前描画。ページ本体は置き場のパネル (`TabHost::pages`) の子で、
//! 選択中のページだけを置き場いっぱいに広げて見せ、他は隠す (wxSimplebook は使わない:
//! 選択中のページの前に挿入したり選択中のページを抜いたりすると隣のページを表示・フォーカス
//! してしまい、見えている内容と入力先がずれる)。`tabs` はタブバーと同じ並び。

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use wxdragon::id::{ID_ABOUT, ID_EXIT, ID_HIGHEST, ID_NO, ID_OK, ID_SAVE, ID_YES};
use wxdragon::prelude::*;

use crate::actions::Action;
#[cfg(windows)]
use crate::dnd_win;
use crate::document::Document;
use crate::editmenu::{self, EditItem};
use crate::filetype::FileType;
use crate::i18n::{self, tr, tr_args};
use crate::settings::{self, ExternalChange, LastTab, LineNumbers, Settings, Wrap};
use crate::settings_dialog;
use crate::tab::{Tab, TabHost, STATUS_FIELD_WIDTHS};
use crate::tabbar::{DragTarget, TabBar};
use crate::theme::{contains, inflate};
use crate::update_dialog;

/// wxID_PREFERENCES。macOS ではこの ID のメニュー項目がアプリメニューの「設定…」に移される。
const ID_PREFERENCES: i32 = 5022;
const ID_NEW: i32 = ID_HIGHEST + 1;
/// 形式を選んで新規: ID_NEW_TYPE_BASE + FileType::ALL の添字
const ID_NEW_TYPE_BASE: i32 = ID_HIGHEST + 2;
const ID_OPEN: i32 = ID_HIGHEST + 10;
const ID_SAVE_AS: i32 = ID_HIGHEST + 11;
const ID_CLOSE_TAB: i32 = ID_HIGHEST + 12;
const ID_NEW_WINDOW: i32 = ID_HIGHEST + 13;
const ID_MOVE_TAB: i32 = ID_HIGHEST + 14;
const ID_CLOSE_OTHERS: i32 = ID_HIGHEST + 15;
const ID_CLOSE_ALL: i32 = ID_HIGHEST + 16;
const ID_RELOAD: i32 = ID_HIGHEST + 17;
const ID_OPEN_FOLDER: i32 = ID_HIGHEST + 18;
const ID_COPY_PATH: i32 = ID_HIGHEST + 19;
const ID_VIEW_OUTLINE: i32 = ID_HIGHEST + 20;
const ID_VIEW_PREVIEW: i32 = ID_HIGHEST + 21;
/// 折り返し: ID_WRAP_BASE + Wrap::ALL の添字
const ID_WRAP_BASE: i32 = ID_HIGHEST + 22;
/// 行番号の位置: ID_LINE_NUMBERS_BASE + LineNumbers::ALL の添字
const ID_LINE_NUMBERS_BASE: i32 = ID_HIGHEST + 25;
const ID_VIEW_RULER: i32 = ID_HIGHEST + 29;
const ID_VIEW_CHANGES: i32 = ID_HIGHEST + 30;
const ID_NO_ACTIONS: i32 = ID_HIGHEST + 31;
const ID_CLEAR_CHANGES: i32 = ID_HIGHEST + 32;
const ID_CLEAR_SAVED: i32 = ID_HIGHEST + 33;
const ID_CHECK_UPDATE: i32 = ID_HIGHEST + 34;
const ID_FIND: i32 = ID_HIGHEST + 35;
const ID_FIND_NEXT: i32 = ID_HIGHEST + 36;
const ID_FIND_PREV: i32 = ID_HIGHEST + 37;
const ID_REPLACE: i32 = ID_HIGHEST + 38;
const ID_GOTO_LINE: i32 = ID_HIGHEST + 39;
const ID_VIEW_CONSOLE: i32 = ID_HIGHEST + 40;
const ID_OPEN_VIEWER: i32 = ID_HIGHEST + 41;
// 編集項目は editmenu.rs (ID_HIGHEST + 60 ..)、アクションは actions.rs (+100 ..)

/// 表示メニューの折り返しの並び (EmEditor と同じ順)
const WRAP_MENU_ORDER: [Wrap; 3] = [Wrap::None, Wrap::Column, Wrap::Window];

/// メニューバー内の位置 (置き換え・チェック更新に使う)
const MENU_EDIT: usize = 1;
const MENU_VIEW: usize = 3;
const MENU_ACTIONS: usize = 4;

/// 外部変更を確認する間隔 (ms)
const EXTERNAL_CHECK_MS: i32 = 2000;

/// ステータスバーのスタイル: wxSTB_SIZEGRIP | wxSTB_SHOW_TIPS | wxSTB_ELLIPSIZE_END
/// | wxFULL_REPAINT_ON_RESIZE (= wxSTB_DEFAULT_STYLE)。右下にサイズ変更のつまみが出る
const STATUS_BAR_STYLE: i64 = 0x0010 | 0x0020 | 0x0100 | 0x0001_0000;

/// タブバーの外にドロップしたとき、このぶんだけ外れていても「そのバーへ」とみなす (px)
const DROP_MARGIN: i32 = 24;

pub struct MainWindow {
    pub frame: Frame,
    host: TabHost,
    /// タブバーと同じ並び
    tabs: Vec<Tab>,
    /// Drop されるとタイマーが止まるので保持する
    _timer: Option<Timer<Frame>>,
    /// 外部変更のダイアログを出している間は再入しない
    checking: bool,
}

pub type WindowRef = Rc<RefCell<MainWindow>>;

thread_local! {
    /// 開いているウィンドウ。閉じたら外す
    static WINDOWS: RefCell<Vec<WindowRef>> = const { RefCell::new(Vec::new()) };
    /// 最後にアクティブになったウィンドウ (別プロセスから渡されたファイルを開く先)
    static ACTIVE: RefCell<Weak<RefCell<MainWindow>>> = const { RefCell::new(Weak::new()) };
}

pub fn all_windows() -> Vec<WindowRef> {
    WINDOWS.with(|w| w.borrow().clone())
}

/// 最後にアクティブになったウィンドウ。無ければ (閉じていれば) 残っているウィンドウの先頭
pub fn active_window() -> Option<WindowRef> {
    ACTIVE
        .with(|a| a.borrow().upgrade())
        .or_else(|| all_windows().into_iter().next())
}

fn set_active(win: &WindowRef) {
    ACTIVE.with(|a| *a.borrow_mut() = Rc::downgrade(win));
}

/// ウィンドウを前面に出す (最小化していれば戻す)。別プロセスから渡されたファイルを開いたときに使う。
/// Windows では送った側が AllowSetForegroundWindow を呼んでいるので、wx の Raise (SetForegroundWindow) が効く
pub fn bring_to_front(frame: &Frame) {
    if frame.is_iconized() {
        frame.iconize(false);
    }
    frame.raise();
}

fn unregister(win: &WindowRef) {
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        if a.upgrade().is_some_and(|x| Rc::ptr_eq(&x, win)) {
            *a = Weak::new();
        }
    });
    let last = WINDOWS.with(|w| {
        let mut w = w.borrow_mut();
        w.retain(|x| !Rc::ptr_eq(x, win));
        w.is_empty()
    });
    // 最後のウィンドウが閉じたら、サイトモードで起動した開発サーバーを止める
    // (wx のメインループから戻らずにプロセスが終わることがあるので main.rs の後始末だけには頼らない)
    if last {
        crate::site::stop_all();
    }
}

impl MainWindow {
    /// 選択中のタブ
    pub fn current_tab(&self) -> Option<Tab> {
        self.host
            .tabbar
            .selected()
            .and_then(|i| self.tabs.get(i))
            .cloned()
    }

    fn tab_for_path(&self, path: &Path) -> Option<Tab> {
        self.tabs
            .iter()
            .find(|t| t.doc.borrow().path.as_deref() == Some(path))
            .cloned()
    }

    fn tab_at(&self, index: usize) -> Option<Tab> {
        self.tabs.get(index).cloned()
    }

    fn index_of(&self, tab: &Tab) -> Option<usize> {
        self.tabs.iter().position(|t| t.key() == tab.key())
    }

    /// `tab` のページだけを見せる: 他のページを隠し、このページを置き場いっぱいに広げて表示する。
    /// 隠す → 広げる → 出す の順にしておくと、途中で別のページが見えることがない
    fn show_tab_page(&self, tab: &Tab) {
        for other in &self.tabs {
            if other.key() != tab.key() && other.page.is_shown() {
                other.page.show(false);
            }
        }
        self.fit_page(tab);
        if !tab.page.is_shown() {
            tab.page.show(true);
            // 隠れている間に同じ形式の他のタブでペインの大きさが変わっていれば合わせる
            tab.apply_pane_sizes();
        }
    }

    /// タブバーの `index` 番目のタブのページを表示する
    fn show_page(&self, index: usize) {
        if let Some(tab) = self.tabs.get(index) {
            self.show_tab_page(tab);
        }
    }

    /// ページを置き場いっぱいに広げる
    fn fit_page(&self, tab: &Tab) {
        let size = self.host.pages.get_client_size();
        tab.page
            .set_size_with_pos(0, 0, size.width.max(0), size.height.max(0));
    }

    /// 置き場の大きさが変わった: 表示中のページを合わせる (隠れているページは表示するときに合わせる)
    fn fit_current_page(&self) {
        if let Some(tab) = self.current_tab() {
            self.fit_page(&tab);
        }
    }
}

// ---------------------------------------------------------------------------
// 生成
// ---------------------------------------------------------------------------

/// ウィンドウを作って表示する。`files` が空なら既定の形式の新規タブを 1 つ開く
/// (`initial_text` があればその内容で)。
pub fn create(files: &[PathBuf], initial_text: Option<&str>) -> WindowRef {
    let win = build_window(None);
    if files.is_empty() {
        new_tab(&win, default_filetype(&settings::get()), initial_text);
    } else {
        for path in files {
            open_file(&win, path);
        }
    }
    win
}

/// タブ無しのウィンドウ (タブの移動用)。`position` を指定しなければ設定の位置
fn create_empty(position: Option<Point>) -> WindowRef {
    build_window(position)
}

/// フレーム・メニュー・タブバー・ステータスバーを作って表示する (タブは入れない)
fn build_window(position: Option<Point>) -> WindowRef {
    let settings = settings::get();
    let offset = all_windows().len() as i32 * 30;

    // ウィンドウは位置・サイズ・中身をすべて決めてから表示する (起動時のちらつき防止)。
    // サイズは作ったあと、置き先のモニターの拡大率で DIP から換算して当てる (winplace::restore)
    let mut builder = Frame::builder().with_title("Typball");
    let saved_position = match (settings.window.x, settings.window.y) {
        (Some(x), Some(y)) => Some(Point::new(x + offset, y + offset)),
        _ => None,
    };
    let position = position.or(saved_position);
    if let Some(p) = position {
        builder = builder.with_position(p);
    }
    let frame = builder.build();
    crate::winplace::restore(&frame, &settings.window, position);
    crate::appicon::apply(&frame);
    let status = frame.create_status_bar(
        STATUS_FIELD_WIDTHS.len() as i32,
        STATUS_BAR_STYLE,
        -1,
        "status",
    );
    status.set_status_widths(&STATUS_FIELD_WIDTHS);

    let panel = Panel::builder(&frame).build();
    let tabbar = TabBar::new(&panel);
    let pages = Panel::builder(&panel).build();
    let root = BoxSizer::builder(Orientation::Vertical).build();
    root.add(tabbar.window(), 0, SizerFlag::Expand, 0);
    root.add(&pages, 1, SizerFlag::Expand, 0);
    panel.set_sizer(root, true);

    let win: WindowRef = Rc::new(RefCell::new(MainWindow {
        frame,
        host: TabHost {
            pages,
            tabbar: tabbar.clone(),
            frame,
            status,
            on_panes_changed: Rc::new(|| {}),
            on_layout_changed: Rc::new(|| {}),
            on_action: Rc::new(|_| {}),
            on_drop_files: Rc::new(|_| false),
        },
        tabs: Vec::new(),
        _timer: None,
        checking: false,
    }));
    WINDOWS.with(|w| w.borrow_mut().push(win.clone()));
    set_active(&win);
    let weak = Rc::downgrade(&win);
    pages.on_size({
        let weak = weak.clone();
        move |_| {
            if let Some(win) = weak.upgrade() {
                if let Ok(w) = win.try_borrow() {
                    w.fit_current_page();
                }
            }
        }
    });
    win.borrow_mut().host.on_drop_files = Rc::new({
        let weak = weak.clone();
        move |files| match weak.upgrade() {
            Some(win) => drop_files(&win, files),
            None => false,
        }
    });
    win.borrow_mut().host.on_panes_changed = Rc::new({
        let weak = weak.clone();
        move || {
            if let Some(win) = weak.upgrade() {
                update_dynamic_menus(&win);
            }
        }
    });
    win.borrow_mut().host.on_layout_changed = Rc::new(relayout_all_panes);
    win.borrow_mut().host.on_action = Rc::new({
        let weak = weak.clone();
        move |action| {
            if let Some(win) = weak.upgrade() {
                handle_menu(&win, action.id());
            }
        }
    });
    build_menu_bar(&win);

    frame.on_menu({
        let weak = weak.clone();
        move |event| {
            if let Some(win) = weak.upgrade() {
                handle_menu(&win, event.get_id());
            }
        }
    });
    // メニューを開くたびに編集項目の有効 / 無効をフォーカスのあるコントロールに合わせる
    frame.on_menu_opened({
        let weak = weak.clone();
        move |_| {
            if let Some(win) = weak.upgrade() {
                update_edit_menu(&win);
            }
        }
    });
    bind_tabbar(&win, &tabbar);
    frame.on_activate({
        let weak = weak.clone();
        move |event| {
            if let Some(win) = weak.upgrade() {
                if let WindowEventData::Activate(a) = &event {
                    if a.is_active() {
                        set_active(&win);
                    }
                }
                check_external_changes(&win);
            }
        }
    });
    frame.on_close({
        let weak = weak.clone();
        move |event| {
            let Some(win) = weak.upgrade() else {
                event.skip(true);
                return;
            };
            // 更新の再起動では確認済みなので聞き直さない
            if !RESTARTING.with(|r| r.get()) && !confirm_close_all(&win) {
                if let WindowEventData::General(e) = &event {
                    e.veto();
                }
                return;
            }
            save_window_geometry(&win.borrow().frame);
            unregister(&win);
            event.skip(true);
        }
    });
    // エクスプローラーなどからのファイルのドロップ。フレームに付けると、自分のドロップ先を
    // 持たない子ウィンドウ (タブバー・ペイン・ステータスバーなど) の上もまとめて受けられる。
    // エディタ (Scintilla) は自分のドロップ先を持つので tab.rs で別に付ける
    let on_drop = {
        let weak = weak.clone();
        move |files: Vec<String>| match weak.upgrade() {
            Some(win) => drop_files(&win, files),
            None => false,
        }
    };
    #[cfg(windows)]
    dnd_win::install(&frame, Rc::new(on_drop));
    #[cfg(not(windows))]
    FileDropTarget::builder(&frame)
        .with_on_enter(|_, _, _| DragResult::Copy)
        .with_on_drag_over(|_, _, _| DragResult::Copy)
        .with_on_drop_files(move |files, _x, _y| on_drop(files))
        .build();
    let timer = Timer::new(&frame);
    timer.on_tick({
        let weak = weak.clone();
        move |_| {
            if let Some(win) = weak.upgrade() {
                check_external_changes(&win);
            }
        }
    });
    timer.start(EXTERNAL_CHECK_MS, false);
    win.borrow_mut()._timer = Some(timer);

    if settings.window.maximized && offset == 0 && position.is_none() {
        frame.maximize(true);
    }
    frame.show(true);
    win
}

/// タブバーの操作をウィンドウの処理につなぐ
fn bind_tabbar(win: &WindowRef, tabbar: &TabBar) {
    let weak = Rc::downgrade(win);
    tabbar.on_select({
        let weak = weak.clone();
        move |index| {
            if let Some(win) = weak.upgrade() {
                win.borrow().show_page(index);
                on_tab_changed(&win);
            }
        }
    });
    tabbar.on_close({
        let weak = weak.clone();
        move |index| {
            if let Some(win) = weak.upgrade() {
                let tab = win.borrow().tab_at(index);
                if let Some(tab) = tab {
                    close_tab(&win, &tab);
                }
            }
        }
    });
    tabbar.on_context({
        let weak = weak.clone();
        move |_| {
            if let Some(win) = weak.upgrade() {
                show_tab_context_menu(&win);
            }
        }
    });
    tabbar.on_reorder({
        let weak = weak.clone();
        move |from, to| {
            if let Some(win) = weak.upgrade() {
                reorder_tab(&win, from, to);
            }
        }
    });
    tabbar.on_drag_end({
        let weak = weak.clone();
        move |index, screen| {
            if let Some(win) = weak.upgrade() {
                let tab = win.borrow().tab_at(index);
                if let Some(tab) = tab {
                    drop_tab(&win, &tab, screen);
                }
            }
        }
    });
    tabbar.on_new_tab({
        let weak = weak.clone();
        move |_| {
            if let Some(win) = weak.upgrade() {
                new_tab(&win, default_filetype(&settings::get()), None);
            }
        }
    });
    tabbar.on_drag_target({
        let weak = weak.clone();
        move |_, screen| match weak.upgrade() {
            Some(win) => drag_target(&win, screen),
            None => DragTarget::Nothing,
        }
    });
}

/// 全ウィンドウの全タブのペインを設定の置き場に合わせて並べ直す
fn relayout_all_panes() {
    for win in all_windows() {
        let tabs = win.borrow().tabs.clone();
        for tab in &tabs {
            tab.relayout_panes();
        }
    }
}

/// 設定の既定の形式 (不正な値なら txt)
fn default_filetype(settings: &Settings) -> FileType {
    FileType::from_extension(&settings.files.default_filetype)
}

/// 無題の番号: 開いている無題の文書が使っていない最小の番号 (閉じた番号は再利用する)
fn next_untitled_number() -> usize {
    let mut used: Vec<usize> = all_windows()
        .iter()
        .flat_map(|w| {
            w.borrow()
                .tabs
                .iter()
                .filter_map(|t| {
                    let doc = t.doc.borrow();
                    doc.path.is_none().then_some(doc.untitled)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    used.sort_unstable();
    used.dedup();
    (1..).find(|n| !used.contains(n)).unwrap_or(1)
}

// ---------------------------------------------------------------------------
// メニュー
// ---------------------------------------------------------------------------

/// 現在の言語でメニューバーを作り直す
pub fn build_menu_bar(win: &WindowRef) {
    let frame = win.borrow().frame;

    let file = Menu::builder()
        .append_item(ID_NEW, &tr("menu.new"), &tr("menu.help_text.new"))
        .build();
    let new_as = Menu::builder().build();
    for (i, ft) in FileType::ALL.iter().enumerate() {
        new_as.append(
            ID_NEW_TYPE_BASE + i as i32,
            &tr(ft.name_key()),
            "",
            ItemKind::Normal,
        );
    }
    file.append_submenu(new_as, &tr("menu.new_as"), "");
    file.append(
        ID_OPEN,
        &tr("menu.open"),
        &tr("menu.help_text.open"),
        ItemKind::Normal,
    );
    file.append_separator();
    file.append(
        ID_SAVE,
        &tr("menu.save"),
        &tr("menu.help_text.save"),
        ItemKind::Normal,
    );
    file.append(
        ID_SAVE_AS,
        &tr("menu.save_as"),
        &tr("menu.help_text.save_as"),
        ItemKind::Normal,
    );
    file.append(
        ID_CLOSE_TAB,
        &tr("menu.close_tab"),
        &tr("menu.help_text.close_tab"),
        ItemKind::Normal,
    );
    file.append_separator();
    file.append(
        ID_OPEN_VIEWER,
        &tr("menu.open_viewer"),
        &tr("menu.help_text.open_viewer"),
        ItemKind::Normal,
    );
    file.append_separator();
    file.append(
        ID_NEW_WINDOW,
        &tr("menu.new_window"),
        &tr("menu.help_text.new_window"),
        ItemKind::Normal,
    );
    file.append(
        ID_MOVE_TAB,
        &tr("menu.move_tab"),
        &tr("menu.help_text.move_tab"),
        ItemKind::Normal,
    );
    file.append_separator();
    file.append(
        ID_EXIT,
        &tr("menu.exit"),
        &tr("menu.help_text.exit"),
        ItemKind::Normal,
    );

    let edit = editmenu::edit_menu();

    let search = Menu::builder()
        .append_item(ID_FIND, &tr("menu.find"), &tr("menu.help_text.find"))
        .append_item(ID_FIND_NEXT, &tr("menu.find_next"), "")
        .append_item(ID_FIND_PREV, &tr("menu.find_prev"), "")
        .append_item(
            ID_REPLACE,
            &tr("menu.replace"),
            &tr("menu.help_text.replace"),
        )
        .append_separator()
        .append_item(
            ID_GOTO_LINE,
            &tr("menu.goto_line"),
            &tr("menu.help_text.goto_line"),
        )
        .build();

    let view = Menu::builder().build();
    for wrap in WRAP_MENU_ORDER {
        view.append(wrap_id(wrap), &tr(wrap_menu_key(wrap)), "", ItemKind::Radio);
    }
    view.append_separator();
    let line_numbers = Menu::builder().build();
    for (i, v) in LineNumbers::ALL.iter().enumerate() {
        line_numbers.append(
            ID_LINE_NUMBERS_BASE + i as i32,
            &tr(line_numbers_menu_key(*v)),
            "",
            ItemKind::Radio,
        );
    }
    view.append_submenu(line_numbers, &tr("menu.line_numbers"), "");
    view.append(ID_VIEW_RULER, &tr("menu.view_ruler"), "", ItemKind::Check);
    view.append(
        ID_VIEW_CHANGES,
        &tr("menu.view_changes"),
        "",
        ItemKind::Check,
    );
    view.append(
        ID_CLEAR_CHANGES,
        &tr("menu.clear_changes"),
        &tr("menu.help_text.clear_changes"),
        ItemKind::Normal,
    );
    view.append(
        ID_CLEAR_SAVED,
        &tr("menu.clear_saved"),
        &tr("menu.help_text.clear_saved"),
        ItemKind::Normal,
    );
    view.append_separator();
    view.append(
        ID_VIEW_OUTLINE,
        &tr("menu.view_outline"),
        "",
        ItemKind::Check,
    );
    view.append(
        ID_VIEW_PREVIEW,
        &tr("menu.view_preview"),
        "",
        ItemKind::Check,
    );
    view.append(
        ID_VIEW_CONSOLE,
        &tr("menu.view_console"),
        &tr("menu.help_text.view_console"),
        ItemKind::Check,
    );
    let actions = actions_menu(None);
    let tools = Menu::builder()
        .append_item(
            ID_PREFERENCES,
            &tr("menu.preferences"),
            &tr("menu.help_text.preferences"),
        )
        .build();
    let help = Menu::builder()
        .append_item(
            ID_CHECK_UPDATE,
            &tr("menu.check_update"),
            &tr("menu.help_text.check_update"),
        )
        .append_separator()
        .append_item(ID_ABOUT, &tr("menu.about"), &tr("menu.help_text.about"))
        .build();
    let bar = MenuBar::builder()
        .append(file, &tr("menu.file"))
        .append(edit, &tr("menu.edit"))
        .append(search, &tr("menu.search"))
        .append(view, &tr("menu.view"))
        .append(actions, &tr("menu.actions"))
        .append(tools, &tr("menu.tools"))
        .append(help, &tr("menu.help"))
        .build();
    frame.set_menu_bar(bar);
    update_dynamic_menus(win);
}

fn wrap_id(wrap: Wrap) -> i32 {
    ID_WRAP_BASE + Wrap::ALL.iter().position(|w| *w == wrap).unwrap_or(0) as i32
}

fn wrap_menu_key(wrap: Wrap) -> &'static str {
    match wrap {
        Wrap::None => "menu.wrap_none",
        Wrap::Window => "menu.wrap_window",
        Wrap::Column => "menu.wrap_column",
    }
}

fn line_numbers_menu_key(value: LineNumbers) -> &'static str {
    match value {
        LineNumbers::None => "menu.line_numbers_none",
        LineNumbers::Left => "menu.line_numbers_left",
        LineNumbers::Right => "menu.line_numbers_right",
        LineNumbers::Both => "menu.line_numbers_both",
    }
}

/// 「アクション」メニューの中身 (選択中のタブの種別で変わる)。一覧は `actions.rs`
fn actions_menu(filetype: Option<FileType>) -> Menu {
    let menu = Menu::builder().build();
    let actions = filetype.map(Action::for_filetype).unwrap_or(&[]);
    if actions.is_empty() {
        menu.append(ID_NO_ACTIONS, &tr("menu.no_actions"), "", ItemKind::Normal);
        menu.enable_item(ID_NO_ACTIONS, false);
        return menu;
    }
    for action in actions {
        if action.separator_before() {
            menu.append_separator();
        }
        menu.append(
            action.id(),
            &tr(&action.menu_key()),
            &tr(&action.tip_key()),
            ItemKind::Normal,
        );
    }
    menu
}

/// 表示メニューのチェック状態とアクションメニューを選択中のタブと設定に合わせる
fn update_dynamic_menus(win: &WindowRef) {
    let Ok(w) = win.try_borrow() else {
        return;
    };
    let Some(bar) = w.frame.get_menu_bar() else {
        return;
    };
    let tab = w.current_tab();
    let filetype = tab.as_ref().map(|t| t.filetype());
    // タブが 1 つだけのウィンドウで「新しいウィンドウへ移動」しても同じ状態のウィンドウができるだけ
    bar.enable_item(ID_MOVE_TAB, w.tabs.len() > 1);
    // プレビューウィンドウは保存済みのファイルを読むので、無題のタブでは使えない
    let has_path = tab
        .as_ref()
        .map(|t| t.doc.borrow().path.is_some())
        .unwrap_or(false);
    bar.enable_item(ID_OPEN_VIEWER, has_path);
    if let Some(view) = bar.get_menu(MENU_VIEW) {
        let s = settings::get();
        view.check_item(wrap_id(s.editor.wrap), true);
        if let Some(i) = LineNumbers::ALL
            .iter()
            .position(|v| *v == s.editor.line_numbers)
        {
            view.check_item(ID_LINE_NUMBERS_BASE + i as i32, true);
        }
        view.check_item(ID_VIEW_RULER, s.editor.show_ruler);
        view.check_item(ID_VIEW_CHANGES, s.editor.highlight_changes);
        let can_clear = tab.is_some() && s.editor.highlight_changes;
        view.enable_item(ID_CLEAR_CHANGES, can_clear);
        view.enable_item(ID_CLEAR_SAVED, can_clear);
        let (outline, preview, console) = tab
            .as_ref()
            .map(|t| (t.outline_shown(), t.preview_shown(), t.console_shown()))
            .unwrap_or((false, false, false));
        view.check_item(ID_VIEW_OUTLINE, outline);
        view.check_item(ID_VIEW_PREVIEW, preview);
        view.check_item(ID_VIEW_CONSOLE, console);
        view.enable_item(
            ID_VIEW_CONSOLE,
            filetype.map(|f| f.has_console()).unwrap_or(false),
        );
        view.enable_item(
            ID_VIEW_OUTLINE,
            filetype.map(|f| f.has_outline()).unwrap_or(false),
        );
        view.enable_item(
            ID_VIEW_PREVIEW,
            filetype.map(|f| f.has_preview()).unwrap_or(false),
        );
    }
    bar.replace(MENU_ACTIONS, actions_menu(filetype), &tr("menu.actions"));
}

/// 編集メニューの有効 / 無効を選択中のタブのフォーカスに合わせる
fn update_edit_menu(win: &WindowRef) {
    let Ok(w) = win.try_borrow() else {
        return;
    };
    let Some(bar) = w.frame.get_menu_bar() else {
        return;
    };
    if let Some(edit) = bar.get_menu(MENU_EDIT) {
        let target = w.current_tab().map(|t| t.edit_target());
        editmenu::update_enabled(&edit, target.as_ref());
    }
}

/// 「指定行へ移動」のダイアログ
fn goto_line_dialog(win: &WindowRef, tab: &Tab) {
    let frame = win.borrow().frame;
    let last = tab.stc.get_line_count().max(1);
    let current = tab.stc.get_current_line() + 1;
    let dialog = TextEntryDialog::builder(
        &frame,
        &tr_args("find.goto_prompt", &[("max", &last.to_string())]),
        &tr("find.goto_title"),
    )
    .with_default_value(&current.to_string())
    .build();
    if dialog.show_modal() == ID_OK {
        if let Some(line) = dialog
            .get_value()
            .and_then(|v| v.trim().parse::<i32>().ok())
        {
            tab.go_to_line(line);
        }
    }
}

fn handle_menu(win: &WindowRef, id: i32) {
    // 選択中のタブ。ここで取り出しておき、各処理の中では win を借りたままにしない
    // (タブの追加・削除は win.borrow_mut() するので、借りたままだと二重借用になる)
    let current = win.borrow().current_tab();
    match id {
        ID_NEW => {
            let ft = default_filetype(&settings::get());
            new_tab(win, ft, None);
        }
        ID_OPEN => open_dialog(win),
        ID_SAVE | ID_SAVE_AS => {
            if let Some(tab) = current {
                save_tab(win, &tab, id == ID_SAVE_AS);
            }
        }
        ID_CLOSE_TAB => {
            if let Some(tab) = current {
                close_tab(win, &tab);
            }
        }
        ID_NEW_WINDOW => {
            create(&[], None);
        }
        ID_MOVE_TAB => {
            if let Some(tab) = current {
                detach_tab(win, &tab, None);
            }
        }
        ID_CLOSE_OTHERS | ID_CLOSE_ALL => {
            let (current, tabs) = {
                let w = win.borrow();
                (w.current_tab(), w.tabs.clone())
            };
            for tab in tabs {
                let is_current = current
                    .as_ref()
                    .map(|c| c.key() == tab.key())
                    .unwrap_or(false);
                if id == ID_CLOSE_OTHERS && is_current {
                    continue;
                }
                if !close_tab(win, &tab) {
                    break;
                }
            }
        }
        ID_RELOAD => {
            if let Some(tab) = current {
                reload_tab(win, &tab);
            }
        }
        ID_OPEN_FOLDER => {
            if let Some(tab) = current {
                let path = tab.doc.borrow().path.clone();
                if let Some(path) = path {
                    reveal_in_file_manager(&path);
                }
            }
        }
        ID_COPY_PATH => {
            if let Some(tab) = current {
                let path = tab.doc.borrow().path.clone();
                if let Some(path) = path {
                    copy_to_clipboard(&path.display().to_string());
                }
            }
        }
        ID_VIEW_OUTLINE | ID_VIEW_PREVIEW | ID_VIEW_CONSOLE => {
            if let Some(tab) = current {
                match id {
                    ID_VIEW_OUTLINE => tab.show_outline(!tab.outline_shown()),
                    ID_VIEW_PREVIEW => tab.show_preview(!tab.preview_shown()),
                    _ => tab.show_console(!tab.console_shown()),
                }
            }
            update_dynamic_menus(win);
        }
        ID_VIEW_RULER => change_settings(win, |s| s.editor.show_ruler = !s.editor.show_ruler),
        ID_VIEW_CHANGES => change_settings(win, |s| {
            s.editor.highlight_changes = !s.editor.highlight_changes
        }),
        ID_CLEAR_CHANGES | ID_CLEAR_SAVED => {
            if let Some(tab) = current {
                tab.clear_change_marks(id == ID_CLEAR_SAVED);
            }
        }
        ID_FIND | ID_REPLACE => {
            if let Some(tab) = current {
                tab.show_find(id == ID_REPLACE);
            }
        }
        ID_FIND_NEXT | ID_FIND_PREV => {
            if let Some(tab) = current {
                tab.find_next(id == ID_FIND_NEXT);
            }
        }
        ID_GOTO_LINE => {
            if let Some(tab) = current {
                goto_line_dialog(win, &tab);
            }
        }
        ID_OPEN_VIEWER => {
            if let Some(tab) = current {
                let path = tab.doc.borrow().path.clone();
                if let Some(path) = path {
                    crate::viewer::open_one(&path);
                }
            }
        }
        ID_PREFERENCES => open_settings(win),
        ID_ABOUT => show_about(&win.borrow().frame),
        ID_CHECK_UPDATE => update_dialog::check_now(win),
        ID_EXIT => {
            for w in all_windows() {
                let frame = w.borrow().frame;
                // close() は on_close を同期的に呼ぶ。キャンセルされた (まだ登録に残っている)
                // ウィンドウがあればそこで止める
                frame.close(false);
                if all_windows().iter().any(|x| Rc::ptr_eq(x, &w)) {
                    break;
                }
            }
        }
        id if (ID_NEW_TYPE_BASE..ID_NEW_TYPE_BASE + FileType::ALL.len() as i32).contains(&id) => {
            let ft = FileType::ALL[(id - ID_NEW_TYPE_BASE) as usize];
            new_tab(win, ft, None);
        }
        id if (ID_WRAP_BASE..ID_WRAP_BASE + Wrap::ALL.len() as i32).contains(&id) => {
            let wrap = Wrap::ALL[(id - ID_WRAP_BASE) as usize];
            change_settings(win, |s| s.editor.wrap = wrap);
        }
        id if (ID_LINE_NUMBERS_BASE..ID_LINE_NUMBERS_BASE + LineNumbers::ALL.len() as i32)
            .contains(&id) =>
        {
            let value = LineNumbers::ALL[(id - ID_LINE_NUMBERS_BASE) as usize];
            change_settings(win, |s| s.editor.line_numbers = value);
        }
        id if EditItem::from_id(id).is_some() => {
            // 編集 (メニューバーとエディタの右クリックメニューの両方からここに来る)
            if let (Some(item), Some(tab)) = (EditItem::from_id(id), current) {
                item.apply(&tab.edit_target());
            }
        }
        id => {
            // アクション (メニューの「アクション」とアクションバーの両方からここに来る)
            if let (Some(action), Some(tab)) = (Action::from_id(id), current) {
                if !Action::for_filetype(tab.filetype()).contains(&action) {
                    return;
                }
                match action {
                    Action::ExportPdf => export_pdf(win),
                    _ => tab.apply_action(action),
                }
            }
        }
    }
}

/// タブの右クリック: 選択中 (右クリックで選ばれた) タブに対するコンテキストメニューを出す
fn show_tab_context_menu(win: &WindowRef) {
    let (Some(tab), tabbar, tab_count) = ({
        let w = win.borrow();
        (w.current_tab(), w.host.tabbar.clone(), w.tabs.len())
    }) else {
        return;
    };
    let has_path = tab.doc.borrow().path.is_some();
    let mut menu = Menu::builder().build();
    menu.append(ID_CLOSE_TAB, &tr("tabmenu.close"), "", ItemKind::Normal);
    menu.append(
        ID_CLOSE_OTHERS,
        &tr("tabmenu.close_others"),
        "",
        ItemKind::Normal,
    );
    menu.append(ID_CLOSE_ALL, &tr("tabmenu.close_all"), "", ItemKind::Normal);
    menu.append_separator();
    menu.append(ID_SAVE, &tr("tabmenu.save"), "", ItemKind::Normal);
    menu.append(ID_SAVE_AS, &tr("tabmenu.save_as"), "", ItemKind::Normal);
    menu.append(ID_RELOAD, &tr("tabmenu.reload"), "", ItemKind::Normal);
    menu.enable_item(ID_RELOAD, has_path);
    menu.append_separator();
    menu.append(
        ID_MOVE_TAB,
        &tr("tabmenu.move_to_window"),
        "",
        ItemKind::Normal,
    );
    menu.enable_item(ID_MOVE_TAB, tab_count > 1);
    menu.append_separator();
    menu.append(
        ID_OPEN_FOLDER,
        &tr("tabmenu.open_folder"),
        "",
        ItemKind::Normal,
    );
    menu.enable_item(ID_OPEN_FOLDER, has_path);
    menu.append(ID_COPY_PATH, &tr("tabmenu.copy_path"), "", ItemKind::Normal);
    menu.enable_item(ID_COPY_PATH, has_path);
    // 位置 None = マウスの現在位置。選ばれた項目は frame.on_menu に届く
    tabbar.window().popup_menu(&mut menu, None);
}

/// ディスクから読み直す (未保存の変更があれば確認する)
fn reload_tab(win: &WindowRef, tab: &Tab) {
    let frame = win.borrow().frame;
    let name = tab.doc.borrow().title();
    if tab.doc.borrow().path.is_none() {
        return;
    }
    if tab.is_modified() {
        let dialog = MessageDialog::builder(
            &frame,
            &tr_args("dialog.reload_confirm", &[("name", &name)]),
            "Typball",
        )
        .with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
        .build();
        if dialog.show_modal() != ID_YES {
            return;
        }
    }
    let result = tab.doc.borrow_mut().reload();
    match result {
        Ok(text) => {
            tab.set_text(&text);
            status(&frame, &tr_args("status.reloaded", &[("name", &name)]));
        }
        Err(e) => error_dialog(
            &frame,
            &tr_args(
                "dialog.open_error",
                &[("path", &name), ("error", &e.to_string())],
            ),
        ),
    }
}

/// OS のファイルマネージャでファイルの場所を開く
fn reveal_in_file_manager(path: &Path) {
    use std::process::Command;
    let result = if cfg!(target_os = "windows") {
        Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg("-R").arg(path).spawn()
    } else {
        let dir = path.parent().unwrap_or(path);
        Command::new("xdg-open").arg(dir).spawn()
    };
    if let Err(e) = result {
        eprintln!("Typball: could not open file manager: {e}");
    }
}

fn copy_to_clipboard(text: &str) {
    let clipboard = Clipboard::get();
    if clipboard.open() {
        let mut data = TextDataObject::new(text);
        clipboard.set_data(&mut data);
        clipboard.close();
    }
}

/// タブが切り替わった: タイトル・ステータスバー・メニューを合わせ、外部変更を確認する
fn on_tab_changed(win: &WindowRef) {
    let tab = win.borrow().current_tab();
    if let Some(tab) = tab {
        tab.update_title();
        tab.update_status();
        tab.stc.set_focus();
    }
    update_dynamic_menus(win);
    check_external_changes(win);
}

fn status(frame: &Frame, text: &str) {
    frame.set_status_text(text, 0);
}

// ---------------------------------------------------------------------------
// タブ操作
// ---------------------------------------------------------------------------

/// 新しいタブを開く。`text` が無ければ種別の既定テキスト
pub fn new_tab(win: &WindowRef, filetype: FileType, text: Option<&str>) -> Tab {
    let doc = Document::untitled(filetype, next_untitled_number());
    // 形式ごとの雛形 (設定 [filetypes.*] template、無ければ言語ごとの既定)
    let default_text = filetype.template(&settings::get());
    add_tab(win, doc, text.unwrap_or(&default_text), false, None)
}

fn add_tab(
    win: &WindowRef,
    doc: Document,
    text: &str,
    modified: bool,
    index: Option<usize>,
) -> Tab {
    let host = win.borrow().host.clone();
    let tab = Tab::new(&host, doc, text, modified, index);
    {
        let mut w = win.borrow_mut();
        let index = index.unwrap_or(w.tabs.len()).min(w.tabs.len());
        w.tabs.insert(index, tab.clone());
        w.show_page(index);
    }
    on_tab_changed(win);
    tab
}

/// タブバーで選ぶ (ページも切り替える)
fn select_tab(win: &WindowRef, index: usize) {
    {
        let w = win.borrow();
        w.host.tabbar.select(index);
        w.show_page(index);
    }
    on_tab_changed(win);
}

/// ドラッグで並べ替えられた (タブバー側の並びは変更済み): `tabs` も同じ並びにする
fn reorder_tab(win: &WindowRef, from: usize, to: usize) {
    let mut w = win.borrow_mut();
    if from >= w.tabs.len() || to >= w.tabs.len() {
        return;
    }
    let tab = w.tabs.remove(from);
    w.tabs.insert(to, tab);
}

/// 無題・未変更・空のタブ (開く操作で置き換えてよいタブ)
fn is_blank(tab: &Tab) -> bool {
    tab.doc.borrow().path.is_none() && !tab.is_modified() && tab.text().is_empty()
}

/// ファイルを開く (既に開いていればそのタブへ)
pub fn open_file(win: &WindowRef, path: &Path) -> Option<Tab> {
    let existing = win.borrow().tab_for_path(path);
    if let Some(tab) = existing {
        if let Some(i) = tab.index() {
            select_tab(win, i);
        }
        return Some(tab);
    }
    let frame = win.borrow().frame;
    match Document::open(path) {
        Ok((doc, text)) => {
            let blank = win.borrow().current_tab().filter(is_blank);
            let tab = add_tab(win, doc, &text, false, None);
            if let Some(blank) = blank {
                remove_tab(win, &blank);
            }
            Some(tab)
        }
        Err(e) => {
            error_dialog(
                &frame,
                &tr_args(
                    "dialog.open_error",
                    &[
                        ("path", &path.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                ),
            );
            None
        }
    }
}

pub fn open_files(win: &WindowRef, paths: &[PathBuf]) {
    for path in paths {
        open_file(win, path);
    }
}

/// ドロップされたファイルを開く (フォルダは無視)。開いたら true
fn drop_files(win: &WindowRef, files: Vec<String>) -> bool {
    let paths: Vec<PathBuf> = files
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();
    if paths.is_empty() {
        return false;
    }
    open_files(win, &paths);
    win.borrow().frame.raise();
    true
}

fn open_dialog(win: &WindowRef) {
    let frame = win.borrow().frame;
    let dialog = FileDialog::builder(&frame)
        .with_message(&tr("dialog.open_title"))
        .with_wildcard(&FileType::open_wildcard())
        .with_style(
            FileDialogStyle::Open | FileDialogStyle::FileMustExist | FileDialogStyle::Multiple,
        )
        .build();
    if dialog.show_modal() != ID_OK {
        return;
    }
    let paths: Vec<PathBuf> = dialog.get_paths().into_iter().map(PathBuf::from).collect();
    open_files(win, &paths);
}

/// 保存する。必要なら (無題か「名前を付けて保存」なら) 保存先を聞く。保存できたら true
fn save_tab(win: &WindowRef, tab: &Tab, save_as: bool) -> bool {
    let frame = win.borrow().frame;
    let needs_name = save_as || tab.doc.borrow().path.is_none();
    if needs_name {
        let (name, dir, filetype) = {
            let doc = tab.doc.borrow();
            let dir = doc
                .path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|d| d.display().to_string());
            (doc.title(), dir, doc.filetype)
        };
        let mut builder = FileDialog::builder(&frame)
            .with_message(&tr("dialog.save_title"))
            .with_default_file(&name)
            .with_wildcard(&filetype.save_wildcard())
            .with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt);
        if let Some(dir) = dir {
            builder = builder.with_default_dir(&dir);
        }
        let dialog = builder.build();
        if dialog.show_modal() != ID_OK {
            return false;
        }
        let Some(path) = dialog.get_path() else {
            return false;
        };
        let mut path = PathBuf::from(path);
        if path.extension().is_none() {
            path.set_extension(filetype.extension());
        }
        tab.doc.borrow_mut().set_path(path);
    }
    let text = tab.text();
    let result = tab.doc.borrow_mut().save(&text);
    let path = tab.doc.borrow().path.clone().unwrap_or_default();
    match result {
        Ok(()) => {
            tab.mark_saved();
            tab.filetype_changed();
            update_dynamic_menus(win);
            status(
                &frame,
                &tr_args("status.saved", &[("path", &path.display().to_string())]),
            );
            true
        }
        Err(e) => {
            error_dialog(
                &frame,
                &tr_args(
                    "dialog.save_error",
                    &[
                        ("path", &path.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                ),
            );
            false
        }
    }
}

/// 未保存なら保存するか確認する。続行してよければ true
fn confirm_discard(win: &WindowRef, tab: &Tab) -> bool {
    if !tab.is_modified() {
        return true;
    }
    let frame = win.borrow().frame;
    if let Some(i) = tab.index() {
        select_tab(win, i);
    }
    let message = tr_args(
        "dialog.save_changes",
        &[("name", &tab.doc.borrow().title())],
    );
    let dialog = MessageDialog::builder(&frame, &message, "Typball")
        .with_style(
            MessageDialogStyle::YesNo
                | MessageDialogStyle::Cancel
                | MessageDialogStyle::IconQuestion,
        )
        .build();
    match dialog.show_modal() {
        ID_YES => save_tab(win, tab, false),
        ID_NO => true,
        _ => false,
    }
}

/// タブを閉じる (未保存なら確認)。最後のタブを閉じたときは設定 `[general] last_tab` に従って
/// 空のタブを 1 つ残すか、ウィンドウを閉じる (最後のウィンドウなら終了になる)
fn close_tab(win: &WindowRef, tab: &Tab) -> bool {
    if !confirm_discard(win, tab) {
        return false;
    }
    remove_tab(win, tab);
    if win.borrow().tabs.is_empty() {
        let settings = settings::get();
        match settings.general.last_tab {
            LastTab::KeepEmpty => {
                new_tab(win, default_filetype(&settings), None);
            }
            LastTab::CloseWindow => {
                // on_close はタブが無いので確認無しで通る。破棄は wx がアイドル時に行う
                let frame = win.borrow().frame;
                frame.close(false);
            }
        }
    }
    true
}

/// 確認なしにタブを外して破棄する
fn remove_tab(win: &WindowRef, tab: &Tab) {
    {
        let mut w = win.borrow_mut();
        if let Some(i) = w.index_of(tab) {
            w.host.tabbar.remove(i);
            w.tabs.remove(i);
        }
        tab.destroy();
        // タブバーが選び直したタブのページを出す
        if let Some(sel) = w.host.tabbar.selected() {
            w.show_page(sel);
        }
    }
    on_tab_changed(win);
}

/// ウィンドウを閉じる前に全タブの未保存を確認する
fn confirm_close_all(win: &WindowRef) -> bool {
    let tabs = win.borrow().tabs.clone();
    tabs.iter().all(|tab| confirm_discard(win, tab))
}

thread_local! {
    /// 更新の適用後の終了中 (on_close で未保存の確認をしない)
    static RESTARTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// すべてのウィンドウで未保存の確認をする (更新で再起動する前)。続行してよければ true
pub fn confirm_close_all_windows() -> bool {
    all_windows().iter().all(confirm_close_all)
}

/// 全ウィンドウで開いているファイル (再起動後に開き直す)
pub fn open_paths() -> Vec<PathBuf> {
    all_windows()
        .iter()
        .flat_map(|w| {
            w.borrow()
                .tabs
                .iter()
                .filter_map(|t| t.doc.borrow().path.clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// 確認なしに全ウィンドウを閉じる (更新を適用して新しい Typball を起動した後)
pub fn close_all_for_restart() {
    RESTARTING.with(|r| r.set(true));
    for w in all_windows() {
        let frame = w.borrow().frame;
        frame.close(true);
    }
}

/// タブを新しいウィンドウへ移す (テキストと未保存状態ごと)。`position` は新しいウィンドウの位置。
/// 元のウィンドウが空になったらそのウィンドウは閉じる
fn detach_tab(win: &WindowRef, tab: &Tab, position: Option<Point>) {
    let new_win = create_empty(position);
    move_tab_to_window(win, tab, &new_win, None);
}

/// タブを別のウィンドウへ移す (`index` はそのウィンドウでの位置、None なら末尾)
fn move_tab_to_window(win: &WindowRef, tab: &Tab, target: &WindowRef, index: Option<usize>) {
    if Rc::ptr_eq(win, target) {
        return;
    }
    let text = tab.text();
    let doc = tab.doc.borrow().clone();
    let modified = tab.is_modified();
    add_tab(target, doc, &text, modified, index);
    remove_tab(win, tab);
    target.borrow().frame.raise();
    let frame = win.borrow().frame;
    if win.borrow().tabs.is_empty() {
        frame.close(true);
    }
}

/// 画面座標 `screen` の上にタブバーがある別のウィンドウ
fn window_under_tabbar(win: &WindowRef, screen: Point) -> Option<WindowRef> {
    all_windows().into_iter().find(|other| {
        if Rc::ptr_eq(other, win) {
            return false;
        }
        let rect = inflate(&other.borrow().host.tabbar.screen_rect(), DROP_MARGIN);
        contains(&rect, screen)
    })
}

/// タブバーの外で離したら何が起きるか (ドラッグ中のカーソルのバッジ用)。
/// あわせて、ポインタの下にある別ウィンドウのタブバーに挿入位置のマーカーを出し、
/// それ以外のバーのマーカーを消す。`screen` が None (自分のバーの中、またはドラッグ終了) なら消すだけ
fn drag_target(win: &WindowRef, screen: Option<Point>) -> DragTarget {
    let under = screen.and_then(|p| window_under_tabbar(win, p));
    for other in all_windows() {
        let marker = match (&under, screen) {
            (Some(u), Some(p)) if Rc::ptr_eq(u, &other) => {
                Some(other.borrow().host.tabbar.drop_index(p))
            }
            _ => None,
        };
        other.borrow().host.tabbar.set_drop_marker(marker);
    }
    if under.is_some() {
        DragTarget::OtherWindow
    } else if screen.is_none() || win.borrow().tabs.len() <= 1 {
        DragTarget::Nothing
    } else {
        DragTarget::NewWindow
    }
}

/// タブをタブバーの外で離した: 他のウィンドウのタブバーの上ならそこへ、そうでなければ
/// 離した位置に新しいウィンドウを作って移す
fn drop_tab(win: &WindowRef, tab: &Tab, screen: Point) {
    if let Some(other) = window_under_tabbar(win, screen) {
        let index = other.borrow().host.tabbar.drop_index(screen);
        move_tab_to_window(win, tab, &other, Some(index));
        return;
    }
    // 1 つしかタブが無いウィンドウからの切り離しは、ウィンドウを動かすのと同じなので何もしない
    if win.borrow().tabs.len() <= 1 {
        return;
    }
    detach_tab(win, tab, Some(Point::new(screen.x - 80, screen.y - 20)));
}

// ---------------------------------------------------------------------------
// アクション
// ---------------------------------------------------------------------------

fn export_pdf(win: &WindowRef) {
    let Some(tab) = win.borrow().current_tab() else {
        return;
    };
    if tab.filetype() != FileType::Typ {
        return;
    }
    let frame = win.borrow().frame;
    let (name, dir) = {
        let doc = tab.doc.borrow();
        let stem = Path::new(&doc.title())
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document".to_string());
        let dir = doc
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.display().to_string());
        (format!("{stem}.pdf"), dir)
    };
    let mut builder = FileDialog::builder(&frame)
        .with_message(&tr("dialog.export_pdf_title"))
        .with_default_file(&name)
        .with_wildcard("*.pdf|*.pdf")
        .with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt);
    if let Some(dir) = dir {
        builder = builder.with_default_dir(&dir);
    }
    let dialog = builder.build();
    if dialog.show_modal() != ID_OK {
        return;
    }
    let Some(path) = dialog.get_path() else {
        return;
    };
    let result = typball_core::export_pdf(&tab.text())
        .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
    match result {
        Ok(()) => status(&frame, &tr_args("status.exported_pdf", &[("path", &path)])),
        Err(e) => error_dialog(&frame, &tr_args("dialog.export_error", &[("error", &e)])),
    }
}

// ---------------------------------------------------------------------------
// 外部変更
// ---------------------------------------------------------------------------

/// 開いているファイルが外部で変わっていないか調べ、設定に従って読み直す・確認する・知らせる
fn check_external_changes(win: &WindowRef) {
    let Ok(w) = win.try_borrow() else {
        return;
    };
    if w.checking {
        return;
    }
    let tabs = w.tabs.clone();
    let frame = w.frame;
    drop(w);
    let mode = settings::get().files.external_change;
    for tab in tabs {
        if !tab.doc.borrow().changed_externally() {
            continue;
        }
        win.borrow_mut().checking = true;
        handle_external_change(&frame, &tab, mode);
        win.borrow_mut().checking = false;
    }
}

fn handle_external_change(frame: &Frame, tab: &Tab, mode: ExternalChange) {
    let (name, exists) = {
        let doc = tab.doc.borrow();
        (
            doc.title(),
            doc.path.as_ref().map(|p| p.is_file()).unwrap_or(false),
        )
    };
    // 同じ変更について二度聞かない
    tab.doc.borrow_mut().acknowledge_external();
    if !exists {
        status(
            frame,
            &tr_args("status.external_deleted", &[("name", &name)]),
        );
        return;
    }
    let ask = |key: &str| {
        let dialog = MessageDialog::builder(frame, &tr_args(key, &[("name", &name)]), "Typball")
            .with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
            .build();
        dialog.show_modal() == ID_YES
    };
    let reload = match mode {
        ExternalChange::Ignore => false,
        ExternalChange::Reload if !tab.is_modified() => true,
        ExternalChange::Reload => ask("dialog.reload_modified"),
        ExternalChange::Ask if tab.is_modified() => ask("dialog.reload_modified"),
        ExternalChange::Ask => ask("dialog.reload"),
    };
    if !reload {
        status(
            frame,
            &tr_args("status.external_changed", &[("name", &name)]),
        );
        return;
    }
    let result = tab.doc.borrow_mut().reload();
    match result {
        Ok(text) => {
            tab.set_text(&text);
            status(frame, &tr_args("status.reloaded", &[("name", &name)]));
        }
        Err(e) => error_dialog(
            frame,
            &tr_args(
                "dialog.open_error",
                &[("path", &name), ("error", &e.to_string())],
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// 設定・バージョン情報・ウィンドウ位置
// ---------------------------------------------------------------------------

fn error_dialog(frame: &Frame, message: &str) {
    MessageDialog::builder(frame, message, "Typball")
        .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
        .build()
        .show_modal();
}

/// 設定ダイアログを開き、OK なら保存して全ウィンドウに反映する
fn open_settings(win: &WindowRef) {
    let frame = win.borrow().frame;
    let before = settings::get();
    let Some(after) = settings_dialog::show(&frame, &before) else {
        return;
    };
    if after == before {
        return;
    }
    settings::set(after.clone());
    if let Err(e) = after.save() {
        status(
            &frame,
            &tr_args("status.settings_save_error", &[("error", &e)]),
        );
    }
    apply_settings(&before, &after);
}

/// 設定を 1 か所変えて保存し、全ウィンドウに反映する (表示メニューからの変更用)
fn change_settings(win: &WindowRef, change: impl FnOnce(&mut Settings)) {
    let before = settings::get();
    let mut after = before.clone();
    change(&mut after);
    if after == before {
        return;
    }
    settings::set(after.clone());
    if let Err(e) = after.save() {
        status(
            &win.borrow().frame,
            &tr_args("status.settings_save_error", &[("error", &e)]),
        );
    }
    apply_settings(&before, &after);
}

/// 変更された設定を開いているすべてのウィンドウに反映する
fn apply_settings(before: &Settings, after: &Settings) {
    let language_changed = after.general.language != before.general.language;
    if language_changed {
        i18n::set_language(&after.general.language);
    }
    // Typst の既定フォント (文書が #set text で指定しないとき)。タブの組み直しより先に
    typball_core::set_default_text(&after.preview.font_face, after.preview.font_size as f64);
    crate::viewer::apply_settings(before, after);
    let tabs_changed = after.editor != before.editor
        || after.filetypes != before.filetypes
        || after.files != before.files
        || after.panes != before.panes
        || after.preview != before.preview
        || language_changed;
    for win in all_windows() {
        if language_changed {
            build_menu_bar(&win);
        }
        if tabs_changed {
            let tabs = win.borrow().tabs.clone();
            for tab in &tabs {
                tab.apply_settings(before, after);
            }
        }
        // 表示メニューのチェック (折り返し・行番号など) を合わせる
        update_dynamic_menus(&win);
    }
}

/// バージョン情報
fn show_about(frame: &Frame) {
    let mut info = AboutDialogInfo::new();
    info.set_name("Typball");
    info.set_version(&crate::update::version_label());
    info.set_description(&tr("about.description"));
    info.set_copyright(&tr("about.copyright"));
    info.set_website_ex(env!("CARGO_PKG_REPOSITORY"), &tr("about.website"));
    // Typball 自身の LICENSE に続けて、同梱フォント・ライブラリのライセンスの一覧 (全文は THIRD-PARTY-NOTICES.txt)
    info.set_licence(&format!(
        "{}\n\n{}",
        include_str!("../../../LICENSE").trim_end(),
        tr("about.third_party")
    ));
    // 大きめのアイコン (96 DIP 相当。画面の拡大率に合わせて実ピクセルを決める)
    if let Some(icon) = crate::appicon::large_bitmap(frame.from_dip_int(96).max(1) as u32) {
        info.set_icon(&icon);
    }
    show_about_box(&info, Some(frame));
}

/// ウィンドウを閉じるときに位置とサイズを保存する (最後に閉じたウィンドウのものが残る)
fn save_window_geometry(frame: &Frame) {
    let mut s = settings::get();
    crate::winplace::save(frame, &mut s.window);
    settings::set(s.clone());
    let _ = s.save();
}
