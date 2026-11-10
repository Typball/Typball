//! 設定ダイアログ
//!
//! 左にページ一覧 (Treebook)、右に各ページの内容、下に OK / キャンセル。
//! ページごとに「ウィジェットを作る」「OK 時にウィジェットから `Settings` へ値を読み戻す」
//! の 2 つを書けば項目を増やせる。読み戻しは `Readers` にクロージャとして登録する。
//! ページの中身は最初に表示されるときに作る ([`PAGES`])。ネイティブのコンボボックスなどは
//! 1 つ作るのに数十 ms かかるので、全ページぶんを先に作るとダイアログが開くまで待たされる。

use std::cell::RefCell;
use std::rc::Rc;

use wxdragon::id::{ID_CANCEL, ID_OK};
use wxdragon::prelude::*;

use crate::filetype::FileType;
use crate::i18n::{self, tr, tr_args};
use crate::settings::{
    self, BottomWidth, Channel, ExternalChange, InstanceMode, LastTab, LineNumbers, MarkdownMode,
    OpenMode, Panes, PanesOverride, SavedChanges, Settings, Wrap,
};

/// OK が押されたときに呼ばれる読み戻し処理の集まり
type Readers = Vec<Box<dyn Fn(&mut Settings)>>;

/// ページの中身を `panel` の中に作り、読み戻し処理を `readers` に登録する
type PageBuilder = fn(&Panel, &Settings, &mut Readers);

/// ページの並び: (タイトルのキー, 中身を作る関数, 直前の親ページの子か)。
/// 子ページは Treebook で親の下にぶら下がる (ファイル → Markdown / Typst の雛形)
const PAGES: [(&str, PageBuilder, bool); 7] = [
    ("settings.general.title", page_general, false),
    ("settings.editor.title", page_editor, false),
    ("settings.files.title", page_files, false),
    ("filetype.md", page_template_md, true),
    ("filetype.typ", page_template_typ, true),
    ("settings.preview.title", page_preview, false),
    ("settings.update.title", page_update, false),
];

const LABEL_FLAGS: SizerFlag = SizerFlag::AlignCenterVertical;
const GAP: i32 = 8;

/// ダイアログを開き、OK なら新しい設定を返す (キャンセルなら None)
pub fn show(parent: &Frame, current: &Settings) -> Option<Settings> {
    let dialog = Dialog::builder(parent, &tr("settings.title"))
        .with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
        .build();

    let book = Treebook::builder(&dialog).build();
    let readers: Rc<RefCell<Readers>> = Rc::new(RefCell::new(Vec::new()));

    // ページは空のパネルだけ先に登録し、中身は最初に表示されるときに作る。
    // 一度も表示されなかったページの設定はそのまま残る (読み戻し処理が無い)
    let pages: Vec<Panel> = PAGES
        .iter()
        .map(|(title_key, _, sub)| {
            let panel = Panel::builder(&book).build();
            if *sub {
                book.add_sub_page(&panel, &tr(title_key), false, -1);
            } else {
                book.add_page(&panel, &tr(title_key), false, -1);
            }
            panel
        })
        .collect();
    let build_page = {
        let current = current.clone();
        let readers = readers.clone();
        let built = RefCell::new(vec![false; PAGES.len()]);
        Rc::new(move |index: usize| {
            let Some((panel, (_, build, _))) = pages.get(index).zip(PAGES.get(index)) else {
                return;
            };
            if std::mem::replace(&mut built.borrow_mut()[index], true) {
                return;
            }
            build(panel, &current, &mut readers.borrow_mut());
            panel.layout();
        })
    };
    // 子ページが最初から見えるように、いったん最後の子ページを選んで親を展開させてから戻す
    // (wxDragon の Treebook に ExpandNode が無い。選択はツリーの EnsureVisible で親を開く)
    if let Some(last_sub) = PAGES.iter().rposition(|(_, _, sub)| *sub) {
        book.set_selection(last_sub);
    }
    build_page(0);
    book.set_selection(0);
    // 切り替え「前」(PageChanging) に作る。切り替え後だとページが見えている状態で部品が
    // 左上 (0, 0) に作られ、並べ終わるまでの一瞬それが見える
    book.on_page_changing({
        let build_page = build_page.clone();
        move |event| {
            if let Some(i) = event.get_selection() {
                build_page(i.max(0) as usize);
            }
        }
    });

    // 設定ファイルの場所 (移行時に探しやすいよう常に表示する)
    let location = StaticText::builder(&dialog)
        .with_label(&tr_args(
            "settings.file_location",
            &[("path", &settings::path().display().to_string())],
        ))
        .build();

    let ok = Button::builder(&dialog)
        .with_id(ID_OK)
        .with_label(&tr("settings.ok"))
        .build();
    let cancel = Button::builder(&dialog)
        .with_id(ID_CANCEL)
        .with_label(&tr("settings.cancel"))
        .build();
    let buttons = StdDialogButtonSizerBuilder::new().build();
    buttons.add_button(&ok);
    buttons.add_button(&cancel);
    buttons.realize();

    let root = BoxSizer::builder(Orientation::Vertical).build();
    root.add(&book, 1, SizerFlag::Expand | SizerFlag::All, GAP);
    root.add(
        &location,
        0,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
        GAP,
    );
    root.add_sizer(&buttons, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    dialog.set_sizer(root, true);
    // 長いパス (ポータブル運用など) はダイアログ幅で折り返す
    location.wrap(640);
    dialog.set_min_size(Size::new(640, 480));
    dialog.set_size(Size::new(760, 520));
    dialog.centre();
    // 見せる前に並べておく。wxMSW の wxDialog::Show は ::ShowWindow() の後に自分で WM_SIZE を
    // 送ってサイザーを動かすので、ここで並べておかないと部品が左上に固まった状態が先に見える
    dialog.layout();

    let result = dialog.show_modal();
    let out = if result == ID_OK {
        let mut s = current.clone();
        for read in readers.borrow().iter() {
            read(&mut s);
        }
        Some(s)
    } else {
        None
    };
    dialog.destroy();
    out
}

/// ラベル + ウィジェットの 2 列グリッドを作る
fn form_grid() -> FlexGridSizer {
    let grid = FlexGridSizer::builder(0, 2)
        .with_vgap(GAP)
        .with_hgap(GAP * 2)
        .build();
    grid.add_growable_col(1, 1);
    grid
}

fn label(parent: &Panel, key: &str) -> StaticText {
    StaticText::builder(parent).with_label(&tr(key)).build()
}

/// 選択肢を入れた Choice を作る。項目は描画を止めてから入れる (wx は項目を 1 つ入れるたびに
/// ドロップダウンの高さを計算し直してサイズを変えるので、数項目でも数十 ms かかる)
fn choice_with(parent: &Panel, items: Vec<String>) -> Choice {
    let choice = Choice::builder(parent).build();
    choice.freeze();
    for item in &items {
        choice.append(item);
    }
    choice.thaw();
    choice
}

// ---------------------------------------------------------------------------
// 一般
// ---------------------------------------------------------------------------

fn page_general(panel: &Panel, current: &Settings, readers: &mut Readers) {
    let grid = form_grid();

    // 表示言語: 先頭が "システムに従う"、以降は locales/ の言語
    let languages = i18n::available();
    let mut choices = vec![tr("settings.general.language_system")];
    choices.extend(languages.iter().map(|(_, name)| name.clone()));
    let choice = choice_with(panel, choices);
    let selected = languages
        .iter()
        .position(|(code, _)| *code == current.general.language)
        .map(|i| i as u32 + 1)
        .unwrap_or(0);
    choice.set_selection(selected);

    grid.add(
        &label(panel, "settings.general.language"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&choice, 0, SizerFlag::Expand, 0);

    let note = StaticText::builder(panel)
        .with_label(&tr("settings.general.language_note"))
        .build();

    // 最後のタブを閉じたとき
    let last_tab = choice_with(
        panel,
        LastTab::ALL.iter().map(|v| tr(last_tab_key(*v))).collect(),
    );
    let selected = LastTab::ALL
        .iter()
        .position(|v| *v == current.general.last_tab)
        .unwrap_or(0);
    last_tab.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.general.last_tab"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&last_tab, 0, SizerFlag::AlignLeft, 0);

    // 下のペインの幅
    let bottom_width = choice_with(
        panel,
        BottomWidth::ALL
            .iter()
            .map(|v| tr(bottom_width_key(*v)))
            .collect(),
    );
    let selected = BottomWidth::ALL
        .iter()
        .position(|v| *v == current.panes.bottom_width)
        .unwrap_or(0);
    bottom_width.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.general.bottom_width"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&bottom_width, 0, SizerFlag::AlignLeft, 0);

    // もう一度起動したとき (シングルインスタンス)
    let instance = choice_with(
        panel,
        InstanceMode::ALL
            .iter()
            .map(|v| tr(instance_mode_key(*v)))
            .collect(),
    );
    let selected = InstanceMode::ALL
        .iter()
        .position(|v| *v == current.instance.mode)
        .unwrap_or(0);
    instance.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.general.instance"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&instance, 0, SizerFlag::AlignLeft, 0);

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    sizer.add(&note, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    panel.set_sizer(sizer, true);
    note.wrap(360);

    readers.push(Box::new(move |s| {
        let idx = choice.get_selection().unwrap_or(0) as usize;
        s.general.language = if idx == 0 {
            i18n::SYSTEM.to_string()
        } else {
            languages
                .get(idx - 1)
                .map(|(code, _)| code.to_string())
                .unwrap_or_else(|| i18n::SYSTEM.to_string())
        };
        let idx = last_tab.get_selection().unwrap_or(0) as usize;
        s.general.last_tab = LastTab::ALL.get(idx).copied().unwrap_or_default();
        let idx = bottom_width.get_selection().unwrap_or(0) as usize;
        s.panes.bottom_width = BottomWidth::ALL.get(idx).copied().unwrap_or_default();
        let idx = instance.get_selection().unwrap_or(0) as usize;
        s.instance.mode = InstanceMode::ALL.get(idx).copied().unwrap_or_default();
    }));
}

fn instance_mode_key(value: InstanceMode) -> &'static str {
    match value {
        InstanceMode::Single => "settings.general.instance_single",
        InstanceMode::Multiple => "settings.general.instance_multiple",
    }
}

fn last_tab_key(value: LastTab) -> &'static str {
    match value {
        LastTab::KeepEmpty => "settings.general.last_tab_keep",
        LastTab::CloseWindow => "settings.general.last_tab_close",
    }
}

fn bottom_width_key(value: BottomWidth) -> &'static str {
    match value {
        BottomWidth::Editor => "settings.general.bottom_width_editor",
        BottomWidth::Window => "settings.general.bottom_width_window",
    }
}

// ---------------------------------------------------------------------------
// エディタ
// ---------------------------------------------------------------------------

fn page_editor(panel: &Panel, current: &Settings, readers: &mut Readers) {
    let grid = form_grid();

    // 等幅フォントを先に、残りを後に (どちらも名前順)。手入力もできる
    let families = typball_core::system_font_families();
    let mut choices: Vec<String> = families
        .iter()
        .filter(|f| f.monospace)
        .map(|f| f.name.clone())
        .collect();
    choices.extend(
        families
            .iter()
            .filter(|f| !f.monospace)
            .map(|f| f.name.clone()),
    );
    // 一覧は最初にドロップダウンを開いたときに入れる (数百のフォント名を入れるのに数百 ms
    // かかり、ダイアログが開くのが遅くなる)。まとめて追加し、描画も止めておく
    let font_face = ComboBox::builder(panel)
        .with_value(&current.editor.font_face)
        .build();
    let filled = std::rc::Rc::new(std::cell::Cell::new(false));
    font_face.on_dropdown({
        let filled = filled.clone();
        move |_| {
            if filled.replace(true) {
                return;
            }
            let refs: Vec<&str> = choices.iter().map(|s| s.as_str()).collect();
            font_face.freeze();
            font_face.append_items(&refs);
            font_face.thaw();
        }
    });
    grid.add(
        &label(panel, "settings.editor.font_face"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&font_face, 0, SizerFlag::Expand, 0);

    let font_size = SpinCtrl::builder(panel)
        .with_range(6, 72)
        .with_initial_value(current.editor.font_size)
        .build();
    grid.add(
        &label(panel, "settings.editor.font_size"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&font_size, 0, SizerFlag::AlignLeft, 0);

    let tab_width = SpinCtrl::builder(panel)
        .with_range(1, 16)
        .with_initial_value(current.editor.tab_width)
        .build();
    grid.add(
        &label(panel, "settings.editor.tab_width"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&tab_width, 0, SizerFlag::AlignLeft, 0);

    // 折り返し: Wrap::ALL の並び順。文字数は「指定文字数で折り返す」のときだけ使う
    let wrap = choice_with(panel, Wrap::ALL.iter().map(|v| tr(wrap_key(*v))).collect());
    let selected = Wrap::ALL
        .iter()
        .position(|v| *v == current.editor.wrap)
        .unwrap_or(1);
    wrap.set_selection(selected as u32);
    grid.add(&label(panel, "settings.editor.wrap"), 0, LABEL_FLAGS, 0);
    grid.add(&wrap, 0, SizerFlag::AlignLeft, 0);

    let wrap_column = SpinCtrl::builder(panel)
        .with_range(10, 1000)
        .with_initial_value(current.editor.wrap_column)
        .build();
    grid.add(
        &label(panel, "settings.editor.wrap_column_width"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&wrap_column, 0, SizerFlag::AlignLeft, 0);

    // 行番号の位置: LineNumbers::ALL の並び順で選択肢を作る
    let line_numbers = choice_with(
        panel,
        LineNumbers::ALL
            .iter()
            .map(|v| tr(line_numbers_key(*v)))
            .collect(),
    );
    let selected = LineNumbers::ALL
        .iter()
        .position(|v| *v == current.editor.line_numbers)
        .unwrap_or(1);
    line_numbers.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.editor.line_numbers"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&line_numbers, 0, SizerFlag::AlignLeft, 0);

    // 保存したときの編集行の強調: SavedChanges::ALL の並び順
    let saved_changes = choice_with(
        panel,
        SavedChanges::ALL
            .iter()
            .map(|v| tr(saved_changes_key(*v)))
            .collect(),
    );
    let selected = SavedChanges::ALL
        .iter()
        .position(|v| *v == current.editor.saved_changes)
        .unwrap_or(0);
    saved_changes.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.editor.saved_changes"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&saved_changes, 0, SizerFlag::AlignLeft, 0);

    let highlight_changes = CheckBox::builder(panel)
        .with_label(&tr("settings.editor.highlight_changes"))
        .with_value(current.editor.highlight_changes)
        .build();
    let show_ruler = CheckBox::builder(panel)
        .with_label(&tr("settings.editor.show_ruler"))
        .with_value(current.editor.show_ruler)
        .build();

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    sizer.add(
        &highlight_changes,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        GAP,
    );
    sizer.add(&show_ruler, 0, SizerFlag::Left | SizerFlag::Right, GAP);
    panel.set_sizer(sizer, true);

    readers.push(Box::new(move |s| {
        s.editor.font_face = font_face.get_value().trim().to_string();
        s.editor.font_size = font_size.value();
        s.editor.tab_width = tab_width.value();
        s.editor.wrap = wrap
            .get_selection()
            .and_then(|i| Wrap::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.editor.wrap_column = wrap_column.value();
        s.editor.line_numbers = line_numbers
            .get_selection()
            .and_then(|i| LineNumbers::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.editor.highlight_changes = highlight_changes.is_checked();
        s.editor.saved_changes = saved_changes
            .get_selection()
            .and_then(|i| SavedChanges::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.editor.show_ruler = show_ruler.is_checked();
    }));
}

fn wrap_key(value: Wrap) -> &'static str {
    match value {
        Wrap::None => "settings.editor.wrap_none",
        Wrap::Window => "settings.editor.wrap_window",
        Wrap::Column => "settings.editor.wrap_column",
    }
}

fn saved_changes_key(value: SavedChanges) -> &'static str {
    match value {
        SavedChanges::Keep => "settings.editor.saved_changes_keep",
        SavedChanges::Clear => "settings.editor.saved_changes_clear",
    }
}

// ---------------------------------------------------------------------------
// ファイル
// ---------------------------------------------------------------------------

fn page_files(panel: &Panel, current: &Settings, readers: &mut Readers) {
    let grid = form_grid();

    // 新規タブの形式
    let default_type = choice_with(
        panel,
        FileType::ALL.iter().map(|t| tr(t.name_key())).collect(),
    );
    let current_type = FileType::from_extension(&current.files.default_filetype);
    let selected = FileType::ALL
        .iter()
        .position(|t| *t == current_type)
        .unwrap_or(0);
    default_type.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.files.default_filetype"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&default_type, 0, SizerFlag::AlignLeft, 0);

    // 外部で変更されたとき
    let external = choice_with(
        panel,
        ExternalChange::ALL
            .iter()
            .map(|v| tr(external_change_key(*v)))
            .collect(),
    );
    let selected = ExternalChange::ALL
        .iter()
        .position(|v| *v == current.files.external_change)
        .unwrap_or(0);
    external.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.files.external_change"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&external, 0, SizerFlag::AlignLeft, 0);

    // 新規文書の番号
    let numbering = CheckBox::builder(panel)
        .with_label(&tr("settings.files.untitled_numbering"))
        .with_value(current.files.untitled_numbering)
        .build();

    // 種別ごとに開いたときのペインとバー
    let note = StaticText::builder(panel)
        .with_label(&tr("settings.files.panes_note"))
        .build();
    let panes_grid = FlexGridSizer::builder(0, 5)
        .with_vgap(GAP / 2)
        .with_hgap(GAP * 2)
        .build();
    let mut pane_boxes: Vec<(FileType, CheckBox, CheckBox, CheckBox, CheckBox)> = Vec::new();
    for ft in FileType::ALL {
        let panes = ft.panes(current);
        let outline = CheckBox::builder(panel)
            .with_label(&tr("settings.files.show_outline"))
            .with_value(panes.show_outline && ft.has_outline())
            .build();
        let preview = CheckBox::builder(panel)
            .with_label(&tr("settings.files.show_preview"))
            .with_value(panes.show_preview && ft.has_preview())
            .build();
        let console = CheckBox::builder(panel)
            .with_label(&tr("settings.files.show_console"))
            .with_value(panes.show_console && ft.has_console())
            .build();
        let toolbar = CheckBox::builder(panel)
            .with_label(&tr("settings.files.show_actionbar"))
            .with_value(panes.show_actionbar && ft.has_actionbar())
            .build();
        outline.enable(ft.has_outline());
        preview.enable(ft.has_preview());
        console.enable(ft.has_console());
        toolbar.enable(ft.has_actionbar());
        panes_grid.add(&label(panel, ft.name_key()), 0, LABEL_FLAGS, 0);
        panes_grid.add(&outline, 0, LABEL_FLAGS, 0);
        panes_grid.add(&preview, 0, LABEL_FLAGS, 0);
        panes_grid.add(&console, 0, LABEL_FLAGS, 0);
        panes_grid.add(&toolbar, 0, LABEL_FLAGS, 0);
        pane_boxes.push((ft, outline, preview, console, toolbar));
    }

    // スクロール同期 (方向ごと。同期できる形式だけ)
    let sync_note = StaticText::builder(panel)
        .with_label(&tr("settings.files.sync_note"))
        .build();
    let sync_grid = FlexGridSizer::builder(0, 3)
        .with_vgap(GAP / 2)
        .with_hgap(GAP * 2)
        .build();
    let mut sync_boxes: Vec<(FileType, CheckBox, CheckBox)> = Vec::new();
    for ft in FileType::ALL.into_iter().filter(|ft| ft.has_scroll_sync()) {
        let panes = ft.panes(current);
        let to_preview = CheckBox::builder(panel)
            .with_label(&tr("settings.files.sync_to_preview"))
            .with_value(panes.sync_scroll_to_preview)
            .build();
        let to_editor = CheckBox::builder(panel)
            .with_label(&tr("settings.files.sync_to_editor"))
            .with_value(panes.sync_scroll_to_editor)
            .build();
        sync_grid.add(&label(panel, ft.name_key()), 0, LABEL_FLAGS, 0);
        sync_grid.add(&to_preview, 0, LABEL_FLAGS, 0);
        sync_grid.add(&to_editor, 0, LABEL_FLAGS, 0);
        sync_boxes.push((ft, to_preview, to_editor));
    }

    // ファイルを渡されて起動したときの開き方 (プレビューのある形式だけ)
    let open_note = StaticText::builder(panel)
        .with_label(&tr("settings.files.open_note"))
        .build();
    let open_grid = FlexGridSizer::builder(0, 2)
        .with_vgap(GAP / 2)
        .with_hgap(GAP * 2)
        .build();
    let mut open_boxes: Vec<(FileType, CheckBox)> = Vec::new();
    for ft in FileType::ALL.into_iter().filter(|ft| ft.has_preview()) {
        let in_viewer = CheckBox::builder(panel)
            .with_label(&tr("settings.files.open_in_viewer"))
            .with_value(ft.open_mode(current) == OpenMode::Viewer)
            .build();
        open_grid.add(&label(panel, ft.name_key()), 0, LABEL_FLAGS, 0);
        open_grid.add(&in_viewer, 0, LABEL_FLAGS, 0);
        open_boxes.push((ft, in_viewer));
    }

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    sizer.add(
        &numbering,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        GAP,
    );
    sizer.add(
        &note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Top,
        GAP,
    );
    sizer.add_sizer(&panes_grid, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    sizer.add(
        &sync_note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Top,
        GAP,
    );
    sizer.add_sizer(&sync_grid, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    sizer.add(
        &open_note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Top,
        GAP,
    );
    sizer.add_sizer(&open_grid, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    panel.set_sizer(sizer, true);

    readers.push(Box::new(move |s| {
        s.files.default_filetype = default_type
            .get_selection()
            .and_then(|i| FileType::ALL.get(i as usize))
            .map(|t| t.extension().to_string())
            .unwrap_or_else(|| "txt".to_string());
        s.files.external_change = external
            .get_selection()
            .and_then(|i| ExternalChange::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.files.untitled_numbering = numbering.is_checked();
        for (ft, outline, preview, console, toolbar) in &pane_boxes {
            // 開き方とペインの大きさは今の値から始める (開き方はチェックがある形式だけ変える)。
            // 同期のチェックが無い形式は既定のまま
            let sync = sync_boxes.iter().find(|(t, _, _)| t == ft);
            let defaults = ft.default_panes();
            let mut panes = PanesOverride::explicit(
                Panes {
                    show_outline: outline.is_checked(),
                    show_preview: preview.is_checked(),
                    show_console: console.is_checked(),
                    show_actionbar: toolbar.is_checked(),
                    sync_scroll_to_preview: sync
                        .map(|(_, cb, _)| cb.is_checked())
                        .unwrap_or(defaults.sync_scroll_to_preview),
                    sync_scroll_to_editor: sync
                        .map(|(_, _, cb)| cb.is_checked())
                        .unwrap_or(defaults.sync_scroll_to_editor),
                },
                ft.overrides(s),
            );
            if let Some((_, cb)) = open_boxes.iter().find(|(t, _)| t == ft) {
                panes.open_mode = if cb.is_checked() {
                    OpenMode::Viewer
                } else {
                    OpenMode::Editor
                };
            }
            match ft {
                FileType::Txt => s.filetypes.txt = panes,
                FileType::Md => s.filetypes.md = panes,
                FileType::Typ => s.filetypes.typ = panes,
            }
        }
    }));
}

// ---------------------------------------------------------------------------
// ファイル → Markdown / Typst: 新しいファイルの雛形
// ---------------------------------------------------------------------------

fn page_template_md(panel: &Panel, current: &Settings, readers: &mut Readers) {
    page_template(panel, current, readers, FileType::Md);
}

fn page_template_typ(panel: &Panel, current: &Settings, readers: &mut Readers) {
    page_template(panel, current, readers, FileType::Typ);
}

/// 形式ごとの雛形 (Ctrl+N と起動時の新規タブの中身)。複数行のテキストと「既定に戻す」
fn page_template(panel: &Panel, current: &Settings, readers: &mut Readers, ft: FileType) {
    let note = StaticText::builder(panel)
        .with_label(&tr_args(
            "settings.files.template_note",
            &[("name", &tr(ft.name_key()))],
        ))
        .build();
    let text = TextCtrl::builder(panel)
        .with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::DontWrap | TextCtrlStyle::Rich2)
        .build();
    text.set_value(&ft.template(current));
    if let Some(font) = Font::builder()
        .with_face_name(&current.editor.font_face)
        .with_point_size(current.editor.font_size.clamp(6, 32))
        .build()
    {
        text.set_font(&font);
    }
    let reset = Button::builder(panel)
        .with_label(&tr("settings.files.template_reset"))
        .build();
    reset.on_click(move |_| text.set_value(&ft.default_template()));

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add(&note, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    sizer.add(
        &text,
        1,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
        GAP,
    );
    sizer.add(&reset, 0, SizerFlag::AlignRight | SizerFlag::All, GAP);
    panel.set_sizer(sizer, true);

    readers.push(Box::new(move |s| {
        let value = text.get_value();
        // 既定のままなら書き出さない (表示言語を変えたときに既定も切り替わるように)
        ft.overrides_mut(s).template = (value != ft.default_template()).then_some(value);
    }));
}

// ---------------------------------------------------------------------------
// プレビュー
// ---------------------------------------------------------------------------

fn page_preview(panel: &Panel, current: &Settings, readers: &mut Readers) {
    let grid = form_grid();

    // プレビューのフォント (Typst の既定・Markdown の本文・プレビューウィンドウ)。同梱の Noto Sans JP を先頭に
    let embedded = typball_core::EMBEDDED_FONT_FAMILY;
    let mut choices: Vec<String> = vec![embedded.to_string()];
    choices.extend(
        typball_core::system_font_families()
            .iter()
            .map(|f| f.name.clone())
            .filter(|name| name != embedded),
    );
    let font_face = ComboBox::builder(panel)
        .with_value(&current.preview.font_face)
        .build();
    let filled = std::rc::Rc::new(std::cell::Cell::new(false));
    font_face.on_dropdown({
        let filled = filled.clone();
        move |_| {
            if filled.replace(true) {
                return;
            }
            let refs: Vec<&str> = choices.iter().map(|s| s.as_str()).collect();
            font_face.freeze();
            font_face.append_items(&refs);
            font_face.thaw();
        }
    });
    grid.add(
        &label(panel, "settings.preview.font_face"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&font_face, 0, SizerFlag::Expand, 0);
    let font_size = SpinCtrl::builder(panel)
        .with_range(6, 72)
        .with_initial_value(current.preview.font_size)
        .build();
    grid.add(
        &label(panel, "settings.preview.font_size"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&font_size, 0, SizerFlag::AlignLeft, 0);
    let font_note = StaticText::builder(panel)
        .with_label(&tr("settings.preview.font_note"))
        .build();

    // Markdown のプレビュー方式
    let mode = choice_with(
        panel,
        MarkdownMode::ALL
            .iter()
            .map(|m| tr(markdown_mode_key(*m)))
            .collect(),
    );
    let selected = MarkdownMode::ALL
        .iter()
        .position(|m| *m == current.preview.markdown_mode)
        .unwrap_or(0);
    mode.set_selection(selected as u32);
    grid.add(
        &label(panel, "settings.preview.markdown_mode"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&mode, 0, SizerFlag::AlignLeft, 0);

    // 開発サーバーのコマンドと URL
    let command = TextCtrl::builder(panel).build();
    command.set_value(&current.preview.site_command);
    grid.add(
        &label(panel, "settings.preview.site_command"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&command, 0, SizerFlag::Expand, 0);
    let url = TextCtrl::builder(panel).build();
    url.set_value(&current.preview.site_url);
    grid.add(
        &label(panel, "settings.preview.site_url"),
        0,
        LABEL_FLAGS,
        0,
    );
    grid.add(&url, 0, SizerFlag::Expand, 0);

    let note = StaticText::builder(panel)
        .with_label(&tr("settings.preview.site_note"))
        .build();

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    sizer.add(
        &font_note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        GAP,
    );
    sizer.add(
        &note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        GAP,
    );
    panel.set_sizer(sizer, true);
    // 折り返しはサイザーに入れてから (先に Wrap すると wxMSW で高さが折り返し前のまま残り、末尾の行が隠れる)
    font_note.wrap(520);
    note.wrap(520);
    panel.layout();

    readers.push(Box::new(move |s| {
        let face = font_face.get_value();
        s.preview.font_face = if face.trim().is_empty() {
            typball_core::EMBEDDED_FONT_FAMILY.to_string()
        } else {
            face.trim().to_string()
        };
        s.preview.font_size = font_size.value().clamp(6, 72);
        s.preview.markdown_mode = mode
            .get_selection()
            .and_then(|i| MarkdownMode::ALL.get(i as usize).copied())
            .unwrap_or_default();
        let defaults = crate::settings::Preview::default();
        let command = command.get_value();
        s.preview.site_command = if command.trim().is_empty() {
            defaults.site_command
        } else {
            command
        };
        let url = url.get_value();
        s.preview.site_url = if url.trim().is_empty() {
            defaults.site_url
        } else {
            url
        };
    }));
}

fn page_update(panel: &Panel, current: &Settings, readers: &mut Readers) {
    let check = CheckBox::builder(panel)
        .with_label(&tr("settings.update.check_on_startup"))
        .build();
    check.set_value(current.update.check_on_startup);

    let grid = form_grid();
    let channel = choice_with(
        panel,
        Channel::ALL.iter().map(|c| tr(channel_key(*c))).collect(),
    );
    let selected = Channel::ALL
        .iter()
        .position(|c| *c == current.update.channel)
        .unwrap_or(0);
    channel.set_selection(selected as u32);
    grid.add(&label(panel, "settings.update.channel"), 0, LABEL_FLAGS, 0);
    grid.add(&channel, 0, SizerFlag::AlignLeft, 0);
    let interval = SpinCtrl::builder(panel)
        .with_range(1, 24 * 30)
        .with_initial_value(current.update.interval_hours.clamp(1, 24 * 30) as i32)
        .build();
    grid.add(&label(panel, "settings.update.interval"), 0, LABEL_FLAGS, 0);
    grid.add(&interval, 0, SizerFlag::AlignLeft, 0);

    let version = crate::update::version_label();
    let note = StaticText::builder(panel)
        .with_label(&tr_args(
            "settings.update.note",
            &[("url", crate::update::RELEASES_URL), ("version", &version)],
        ))
        .build();
    note.wrap(520);

    let sizer = BoxSizer::builder(Orientation::Vertical).build();
    sizer.add(&check, 0, SizerFlag::All, GAP);
    sizer.add_sizer(&grid, 0, SizerFlag::Expand | SizerFlag::All, GAP);
    sizer.add(
        &note,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        GAP,
    );
    panel.set_sizer(sizer, true);

    readers.push(Box::new(move |s| {
        s.update.check_on_startup = check.get_value();
        s.update.channel = channel
            .get_selection()
            .and_then(|i| Channel::ALL.get(i as usize).copied())
            .unwrap_or_default();
        s.update.interval_hours = interval.value().max(1) as u32;
    }));
}

fn channel_key(value: Channel) -> &'static str {
    match value {
        Channel::Stable => "settings.update.channel_stable",
        Channel::Nightly => "settings.update.channel_nightly",
    }
}

fn markdown_mode_key(value: MarkdownMode) -> &'static str {
    match value {
        MarkdownMode::Auto => "settings.preview.markdown_mode_auto",
        MarkdownMode::Markdown => "settings.preview.markdown_mode_markdown",
        MarkdownMode::Site => "settings.preview.markdown_mode_site",
    }
}

fn external_change_key(value: ExternalChange) -> &'static str {
    match value {
        ExternalChange::Ask => "settings.files.external_change_ask",
        ExternalChange::Reload => "settings.files.external_change_reload",
        ExternalChange::Ignore => "settings.files.external_change_ignore",
    }
}

/// 行番号の位置の表示名のキー
fn line_numbers_key(value: LineNumbers) -> &'static str {
    match value {
        LineNumbers::None => "settings.editor.line_numbers_none",
        LineNumbers::Left => "settings.editor.line_numbers_left",
        LineNumbers::Right => "settings.editor.line_numbers_right",
        LineNumbers::Both => "settings.editor.line_numbers_both",
    }
}
