//! セッションの一覧 (ファイル > セッション > セッションの一覧)
//!
//! 左に過去のセッション (今のセッション → 名前を付けて保存したもの → 自動で記録したものを新しい順)、
//! 右に選んだセッションのウィンドウとタブをチェック付きのツリーで見せる。「復元」で、チェックの付いた
//! タブをウィンドウごとに開き直す (開き直すのは `window.rs`。ここは選んだものを返すだけ)。
//! 保存していなかった無題の文書も、開き直せないが並べて見せる (「あのウィンドウには無題もいくつか
//! あった」という記憶でセッションを探せるように)。
//! 記録そのものは `session.rs`。

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use wxdragon::id::{ID_CANCEL, ID_OK, ID_YES};
use wxdragon::prelude::*;
use wxdragon::widgets::treelistctrl::{
    CheckboxState, TreeListCtrl, TreeListCtrlStyle, TreeListItem,
};

use crate::document::untitled_title;
use crate::filetype::FileType;
use crate::i18n::{tr, tr_args};
use crate::session::{self, Entry, Session, Status, TabState, WindowState};

const GAP: i32 = 8;

/// 一覧の 1 行
struct Row {
    label: String,
    session: Session,
    /// ファイルになっているセッション。None は今のセッション (このプロセスのもの)
    entry: Option<Entry>,
}

impl Row {
    fn status(&self) -> Option<Status> {
        self.entry.as_ref().map(|e| e.status)
    }
}

/// ツリーに出しているウィンドウ
struct ShownWindow {
    /// `Session::windows` の添字
    index: usize,
    item: TreeListItem,
    /// 開き直せるタブ (ファイルのあるタブ): `WindowState::tabs` の添字と項目
    files: Vec<(usize, TreeListItem)>,
    /// 保存していなかった無題のタブ。見せるだけで、開き直すものには数えない
    untitled: Vec<TreeListItem>,
}

type Shown = Vec<ShownWindow>;

struct Ui {
    dialog: Dialog,
    list: ListBox,
    info: StaticText,
    tree: TreeListCtrl,
    restore: Button,
    save: Button,
    delete: Button,
    /// 今のセッション (開いているウィンドウと、この起動中に閉じたウィンドウ)
    current: Option<Session>,
    rows: RefCell<Vec<Row>>,
    shown: RefCell<Shown>,
}

/// 日時を分までにする ("2026-11-19 15:30:12" → "2026-11-19 15:30")
fn short_time(time: &str) -> &str {
    time.get(..16).unwrap_or(time)
}

fn row_label(session: &Session, status: Option<Status>) -> String {
    let Some(status) = status else {
        return tr("session.current");
    };
    if status == Status::Saved {
        return tr_args("session.item_saved", &[("name", &session.name)]);
    }
    let (windows, tabs) = session.counts();
    let mut label = tr_args(
        "session.item",
        &[
            ("time", short_time(&session.updated)),
            ("windows", &windows.to_string()),
            ("tabs", &tabs.to_string()),
        ],
    );
    match status {
        Status::Interrupted => label.push_str(&tr("session.suffix_interrupted")),
        Status::Running => label.push_str(&tr("session.suffix_running")),
        _ => {}
    }
    label
}

/// 一覧の行: 今のセッション → 名前を付けて保存したもの (名前順) → 自動で記録したもの (最後に使った順)
fn build_rows(current: Option<&Session>) -> Vec<Row> {
    let mut rows = Vec::new();
    if let Some(session) = current {
        rows.push(Row {
            label: row_label(session, None),
            session: session.clone(),
            entry: None,
        });
    }
    let (saved, mut history): (Vec<Entry>, Vec<Entry>) = session::list()
        .into_iter()
        .partition(|e| e.status == Status::Saved);
    history.sort_by(|a, b| b.session.updated.cmp(&a.session.updated));
    for entry in saved.into_iter().chain(history) {
        rows.push(Row {
            label: row_label(&entry.session, Some(entry.status)),
            session: entry.session.clone(),
            entry: Some(entry),
        });
    }
    rows
}

impl Ui {
    /// 一覧を読み直して、`select` 番目 (無ければ先頭) を選ぶ
    fn reload(&self, select: usize) {
        let rows = build_rows(self.current.as_ref());
        self.list.clear();
        for row in &rows {
            self.list.append(&row.label);
        }
        let count = rows.len();
        *self.rows.borrow_mut() = rows;
        if count > 0 {
            let index = select.min(count - 1);
            self.list.set_selection(index as u32, true);
            self.show_row(index);
        } else {
            self.tree.delete_all_items();
            self.shown.borrow_mut().clear();
            self.info.set_label("");
            self.update_buttons();
        }
    }

    /// 選んでいる行
    fn selected(&self) -> Option<usize> {
        let index = self.list.get_selection()? as usize;
        (index < self.rows.borrow().len()).then_some(index)
    }

    /// `index` 番目のセッションの中身をツリーに出す
    fn show_row(&self, index: usize) {
        let rows = self.rows.borrow();
        let Some(row) = rows.get(index) else {
            return;
        };
        let status = row.status();
        let session = &row.session;
        self.info.set_label(&match status {
            None => tr("session.info_current"),
            Some(status) => {
                let state = tr(match status {
                    Status::Ended => "session.state_ended",
                    Status::Interrupted => "session.state_interrupted",
                    Status::Running => "session.state_running",
                    Status::Saved => "session.state_saved",
                });
                tr_args(
                    "session.info",
                    &[
                        ("started", short_time(&session.started)),
                        ("updated", short_time(&session.updated)),
                        ("state", &state),
                    ],
                )
            }
        });

        self.tree.delete_all_items();
        let mut shown: Shown = Vec::new();
        let root = self.tree.get_root_item();
        let mut number = 0;
        for (wi, window) in session.windows.iter().enumerate() {
            // ファイルのあるタブと、保存していなかった無題のタブ (作っただけの空の無題は出さない)
            let tabs: Vec<(usize, &TabState)> = window
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.is_listed())
                .collect();
            if tabs.is_empty() {
                continue;
            }
            number += 1;
            let label = tr_args(
                "session.window",
                &[
                    ("n", &number.to_string()),
                    ("tabs", &tabs.len().to_string()),
                ],
            );
            let Some(item) = self.tree.append_item(&root, &label) else {
                continue;
            };
            // ウィンドウの行の 2 列目: 途中で閉じたウィンドウは閉じた日時、今のセッションの開いているものはその旨
            if window.is_closed() {
                let closed = tr_args(
                    "session.window_closed",
                    &[("time", short_time(&window.closed))],
                );
                self.tree.set_item_text(&item, 1, &closed);
            } else if status.is_none() {
                self.tree
                    .set_item_text(&item, 1, &tr("session.window_open"));
            }
            // 今も開いているウィンドウ (今のセッションと、別の Typball が使っているセッション)
            let live = matches!(status, None | Some(Status::Running)) && !window.is_closed();
            let mut files = Vec::new();
            let mut untitled = Vec::new();
            for (ti, tab) in tabs {
                let Some(path) = tab.path.as_deref().map(Path::new) else {
                    // 無題の文書: 開き直せないが、あったことが分かるように並べる
                    let name =
                        untitled_title(tab.untitled, FileType::from_extension(&tab.filetype));
                    if let Some(child) = self.tree.append_item(&item, &name) {
                        let state = if live {
                            "session.untitled_open"
                        } else {
                            "session.untitled_gone"
                        };
                        self.tree.set_item_text(&child, 1, &tr(state));
                        untitled.push(child);
                    }
                    continue;
                };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                let Some(child) = self.tree.append_item(&item, &name) else {
                    continue;
                };
                let folder = path
                    .parent()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default();
                self.tree.set_item_text(&child, 1, &folder);
                files.push((ti, child));
            }
            let shown_window = ShownWindow {
                index: wi,
                item,
                files,
                untitled,
            };
            // 最初のチェック: 終了したセッションは「終了時に開いていたウィンドウ」。今のセッションと、
            // 別の Typball が使っているセッションは、選んだものだけ開き直す
            let checked = matches!(
                status,
                Some(Status::Ended | Status::Interrupted | Status::Saved)
            ) && !window.is_closed();
            if checked {
                for (_, file) in &shown_window.files {
                    self.tree.check_item(file, CheckboxState::Checked);
                }
            }
            self.sync_window(&shown_window);
            self.tree.expand(&item);
            shown.push(shown_window);
        }
        drop(rows);
        *self.shown.borrow_mut() = shown;
        self.update_buttons();
    }

    /// ウィンドウの印 (全部 / 一部 / 無し) を、その下のファイルのチェックに合わせる。無題の文書は
    /// 開き直せないので自分のチェックを持たせず、ウィンドウが「全部」のときだけ付いて見えるようにする
    fn sync_window(&self, window: &ShownWindow) {
        let checked = window
            .files
            .iter()
            .filter(|(_, item)| self.tree.is_checked(item))
            .count();
        let all = checked > 0 && checked == window.files.len();
        let state = if all {
            CheckboxState::Checked
        } else if checked == 0 {
            CheckboxState::Unchecked
        } else {
            CheckboxState::Undetermined
        };
        self.tree.check_item(&window.item, state);
        for item in &window.untitled {
            let state = if all {
                CheckboxState::Checked
            } else {
                CheckboxState::Unchecked
            };
            self.tree.check_item(item, state);
        }
    }

    /// チェックが押された: ウィンドウならその下のファイル全部に写し、印を付け直す
    fn item_checked(&self, item: &TreeListItem) {
        let shown = self.shown.borrow();
        let Some(window) = shown.iter().find(|w| {
            w.item == *item || w.files.iter().any(|(_, i)| i == item) || w.untitled.contains(item)
        }) else {
            return;
        };
        if window.item == *item {
            let state = if self.tree.is_checked(item) {
                CheckboxState::Checked
            } else {
                CheckboxState::Unchecked
            };
            for (_, file) in &window.files {
                self.tree.check_item(file, state);
            }
        }
        self.sync_window(window);
    }

    /// チェックの付いたタブを、ウィンドウごとにまとめる (チェックの無いウィンドウは入らない)
    fn checked(&self) -> Vec<WindowState> {
        let rows = self.rows.borrow();
        let Some(row) = self.selected().and_then(|i| rows.get(i)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for shown in self.shown.borrow().iter() {
            let Some(window) = row.session.windows.get(shown.index) else {
                continue;
            };
            let picked: Vec<usize> = shown
                .files
                .iter()
                .filter(|(_, item)| self.tree.is_checked(item))
                .map(|(ti, _)| *ti)
                .collect();
            if picked.is_empty() {
                continue;
            }
            out.push(WindowState {
                // 選んでいたタブも開き直すなら、それを選び直す
                active: picked
                    .iter()
                    .position(|ti| *ti == window.active)
                    .unwrap_or(0),
                tabs: picked
                    .iter()
                    .filter_map(|ti| window.tabs.get(*ti).cloned())
                    .collect(),
                ..window.clone()
            });
        }
        out
    }

    /// ボタンの有効 / 無効を、選んでいる行とチェックに合わせる
    fn update_buttons(&self) {
        let rows = self.rows.borrow();
        let row = self.selected().and_then(|i| rows.get(i));
        let any_checked = self
            .shown
            .borrow()
            .iter()
            .any(|w| w.files.iter().any(|(_, item)| self.tree.is_checked(item)));
        self.restore.enable(any_checked);
        // 名前を付けて保存: 開いていたウィンドウに、ファイルのあるタブがあるとき
        self.save.enable(
            row.is_some_and(|r| r.session.open_windows().iter().any(WindowState::has_files)),
        );
        // 消せるのはファイルになっているもの (動いている Typball のものは消さない)
        self.delete.enable(row.is_some_and(|r| {
            matches!(
                r.status(),
                Some(Status::Ended | Status::Interrupted | Status::Saved)
            )
        }));
    }

    /// 選んでいるセッションに名前を付けて保存し、保存したものを選ぶ
    fn save_selected(&self) {
        let Some(session) = self
            .selected()
            .and_then(|i| self.rows.borrow().get(i).map(|r| r.session.clone()))
        else {
            return;
        };
        let Some(name) = save_as(&self.dialog, &session, &session.name) else {
            return;
        };
        self.reload(0);
        let index = self
            .rows
            .borrow()
            .iter()
            .position(|r| r.status() == Some(Status::Saved) && r.session.name == name);
        if let Some(index) = index {
            self.list.set_selection(index as u32, true);
            self.show_row(index);
        }
    }

    /// 選んでいるセッションを消す (確認する)
    fn delete_selected(&self) {
        let Some(index) = self.selected() else {
            return;
        };
        let Some((label, entry)) = self
            .rows
            .borrow()
            .get(index)
            .and_then(|r| Some((r.label.clone(), r.entry.clone()?)))
        else {
            return;
        };
        let confirm = MessageDialog::builder(
            &self.dialog,
            &tr_args("session.delete_confirm", &[("name", &label)]),
            "Typball",
        )
        .with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
        .build();
        if confirm.show_modal() != ID_YES {
            return;
        }
        if let Err(e) = session::delete(&entry) {
            error(
                &self.dialog,
                &tr_args("session.delete_error", &[("error", &e.to_string())]),
            );
        }
        self.reload(index);
    }
}

/// 一覧を開く。「復元」で閉じたら、開き直すウィンドウ (チェックの付いたタブだけ) を返す
pub fn show(parent: &Frame, current: Option<Session>) -> Option<Vec<WindowState>> {
    let dialog = Dialog::builder(parent, &tr("session.title"))
        .with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
        .build();
    // 寸法は DIP で書き、ダイアログのある画面の拡大率で実ピクセルにする
    let px = |v: i32| dialog.from_dip_int(v);

    let list_label = StaticText::builder(&dialog)
        .with_label(&tr("session.list_label"))
        .build();
    let list = ListBox::builder(&dialog).build();
    let info = StaticText::builder(&dialog).with_label("").build();
    let tree = TreeListCtrl::builder(&dialog)
        .with_style(TreeListCtrlStyle::Checkbox | TreeListCtrlStyle::Three_State)
        .build();
    tree.append_column(&tr("session.column_name"), px(250), ListColumnFormat::Left);
    tree.append_column(
        &tr("session.column_folder"),
        px(330),
        ListColumnFormat::Left,
    );
    let note = StaticText::builder(&dialog)
        .with_label(&tr("session.note"))
        .build();

    let save = Button::builder(&dialog)
        .with_label(&tr("session.save"))
        .build();
    let delete = Button::builder(&dialog)
        .with_label(&tr("session.delete"))
        .build();
    let restore = Button::builder(&dialog)
        .with_id(ID_OK)
        .with_label(&tr("session.restore"))
        .build();
    let close = Button::builder(&dialog)
        .with_id(ID_CANCEL)
        .with_label(&tr("session.close"))
        .build();

    let left = BoxSizer::builder(Orientation::Vertical).build();
    left.add(&list_label, 0, SizerFlag::Bottom, px(GAP / 2));
    left.add(&list, 1, SizerFlag::Expand, 0);
    let right = BoxSizer::builder(Orientation::Vertical).build();
    right.add(&info, 0, SizerFlag::Expand | SizerFlag::Bottom, px(GAP / 2));
    right.add(&tree, 1, SizerFlag::Expand, 0);
    let top = BoxSizer::builder(Orientation::Horizontal).build();
    top.add_sizer(&left, 2, SizerFlag::Expand | SizerFlag::Right, px(GAP));
    top.add_sizer(&right, 5, SizerFlag::Expand, 0);
    let buttons = BoxSizer::builder(Orientation::Horizontal).build();
    buttons.add(&save, 0, SizerFlag::Right, px(GAP));
    buttons.add(&delete, 0, SizerFlag::AlignCenterVertical, 0);
    buttons.add_stretch_spacer(1);
    buttons.add(&restore, 0, SizerFlag::Right, px(GAP));
    buttons.add(&close, 0, SizerFlag::AlignCenterVertical, 0);
    let root = BoxSizer::builder(Orientation::Vertical).build();
    root.add_sizer(&top, 1, SizerFlag::Expand | SizerFlag::All, px(GAP));
    root.add(
        &note,
        0,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
        px(GAP),
    );
    root.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, px(GAP));
    dialog.set_sizer(root, true);
    dialog.set_min_size(dialog.from_dip(Size::new(720, 420)));
    dialog.set_size(dialog.from_dip(Size::new(980, 560)));
    dialog.centre();

    let ui = Rc::new(Ui {
        dialog,
        list,
        info,
        tree,
        restore,
        save,
        delete,
        current,
        rows: RefCell::new(Vec::new()),
        shown: RefCell::new(Vec::new()),
    });
    ui.reload(0);
    // 最初は、今のセッションではなく前回のもの (自動で記録したうちの一番新しいもの) を選んでおく:
    // 開き直したいのは大抵それ
    let previous = ui
        .rows
        .borrow()
        .iter()
        .position(|r| matches!(r.status(), Some(Status::Ended | Status::Interrupted)));
    if let Some(index) = previous {
        ui.list.set_selection(index as u32, true);
        ui.show_row(index);
    }

    ui.list.on_selection_changed({
        let ui = ui.clone();
        move |_| {
            if let Some(index) = ui.selected() {
                ui.show_row(index);
            }
        }
    });
    // ウィンドウのチェックはその下のタブ全部に、タブのチェックは親のウィンドウの印 (全部 / 一部 / 無し) に写す
    ui.tree.on_item_checked({
        let ui = ui.clone();
        move |event| {
            if let Some(item) = event.get_item() {
                ui.item_checked(&item);
            }
            ui.update_buttons();
        }
    });
    ui.save.on_click({
        let ui = ui.clone();
        move |_| ui.save_selected()
    });
    ui.delete.on_click({
        let ui = ui.clone();
        move |_| ui.delete_selected()
    });

    // 見せる前に並べておく (wxMSW の wxDialog は表示した後に並べるので、部品が左上に固まって見える)
    ui.dialog.layout();
    let chosen = if ui.dialog.show_modal() == ID_OK {
        Some(ui.checked()).filter(|windows| !windows.is_empty())
    } else {
        None
    };
    ui.dialog.destroy();
    chosen
}

/// 名前を聞いて、セッションを保存する (`Session/Saved/<名前>.toml`)。保存したら、その名前を返す。
/// 同じ名前があれば上書きするか聞く
pub fn save_as(parent: &dyn WxWidget, session: &Session, default: &str) -> Option<String> {
    let mut default = default.to_string();
    loop {
        let dialog = TextEntryDialog::builder(
            parent,
            &tr("session.save_prompt"),
            &tr("session.save_title"),
        )
        .with_default_value(&default)
        .build();
        if dialog.show_modal() != ID_OK {
            return None;
        }
        let name = dialog.get_value().unwrap_or_default().trim().to_string();
        default.clone_from(&name);
        let Some(path) = session::saved_path(&name) else {
            error(parent, &tr("session.save_invalid"));
            continue;
        };
        if path.exists() {
            let overwrite = MessageDialog::builder(
                parent,
                &tr_args("session.save_overwrite", &[("name", &name)]),
                "Typball",
            )
            .with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
            .build();
            if overwrite.show_modal() != ID_YES {
                continue;
            }
        }
        return match session::save_named(&name, session, &session::now()) {
            Ok(_) => Some(name),
            Err(e) => {
                error(
                    parent,
                    &tr_args("session.save_error", &[("error", &e.to_string())]),
                );
                None
            }
        };
    }
}

fn error(parent: &dyn WxWidget, message: &str) {
    MessageDialog::builder(parent, message, "Typball")
        .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
        .build()
        .show_modal();
}
