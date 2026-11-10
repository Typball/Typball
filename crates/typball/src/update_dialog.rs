//! 更新の確認と適用の GUI
//!
//! - 起動時 ([`check_on_startup`]): 設定の間隔が空いていれば別スレッドで確認し、新しい版が
//!   あるときだけダイアログを出す (スキップした版は出さない)
//! - ヘルプ → 更新を確認 ([`check_now`]): 結果を必ず表示する
//!
//! HTTP は別スレッドで行い、結果は `wxdragon::call_after` で GUI スレッドに戻す。ダウンロード中は
//! `ProgressDialog` を出し、タイマーで進捗を映す。適用の前に全ウィンドウで未保存の確認をし、
//! 新しい Typball を起動してから今のウィンドウを閉じる。

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use wxdragon::id::{ID_CANCEL, ID_HIGHEST, ID_OK};
use wxdragon::prelude::*;

use crate::i18n::{tr, tr_args};
use crate::settings;
use crate::update::{self, Check, InstallKind, Release};
use crate::window::{self, MainWindow, WindowRef};

const ID_SKIP: i32 = ID_HIGHEST + 300;
/// 進捗ダイアログの最大値 (0.1% 刻み)
const PROGRESS_MAX: i32 = 1000;
const PROGRESS_POLL_MS: i32 = 100;

/// 確認スレッドの行き先 (Weak は Send でないので GUI 側に置いておく)
struct Pending {
    win: Weak<RefCell<MainWindow>>,
    silent: bool,
}

/// ダウンロードの進み具合 (スレッド → タイマー)
#[derive(Default)]
struct Transfer {
    done: u64,
    total: Option<u64>,
    result: Option<Result<(), String>>,
}

/// 進行中のダウンロード (タイマーを生かしておくために持つ)
struct Download {
    timer: Timer<Frame>,
    dialog: ProgressDialog,
    state: Arc<Mutex<Transfer>>,
    cancel: Arc<AtomicBool>,
    release: Release,
    kind: InstallKind,
    dest: PathBuf,
    finished: Cell<bool>,
}

thread_local! {
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    static ACTIVE: RefCell<Option<Rc<Download>>> = const { RefCell::new(None) };
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 起動時の自動確認 (設定で切れる。間隔が空いていなければ何もしない)
pub fn check_on_startup(win: &WindowRef) {
    let s = settings::get();
    if !s.update.check_on_startup {
        return;
    }
    let interval = u64::from(s.update.interval_hours.max(1)) * 3600;
    if now_secs().saturating_sub(s.update.last_check) < interval {
        return;
    }
    start_check(win, true);
}

/// ヘルプメニューからの確認 (結果を必ず表示する)
pub fn check_now(win: &WindowRef) {
    start_check(win, false);
}

fn start_check(win: &WindowRef, silent: bool) {
    if BUSY.with(|b| b.replace(true)) {
        return;
    }
    if !silent {
        win.borrow()
            .frame
            .set_status_text(&tr("update.checking"), 0);
    }
    PENDING.with(|p| {
        *p.borrow_mut() = Some(Pending {
            win: Rc::downgrade(win),
            silent,
        })
    });
    let channel = settings::get().update.channel;
    std::thread::spawn(move || {
        let result = update::check(channel);
        wxdragon::call_after(Box::new(move || {
            BUSY.with(|b| b.set(false));
            let pending = PENDING.with(|p| p.borrow_mut().take());
            if let Some(win) = pending.as_ref().and_then(|p| p.win.upgrade()) {
                finish_check(&win, result, pending.map(|p| p.silent).unwrap_or(true));
            }
        }));
        // call_after の中身はアイドル時に実行されるので、何も起きていないアプリを起こす
        wxdragon::wake_up_idle();
    });
}

fn finish_check(win: &WindowRef, result: Check, silent: bool) {
    // 確認した時刻は失敗でも記録する (つながらない環境で起動のたびに待たせない)
    let mut s = settings::get();
    s.update.last_check = now_secs();
    settings::set(s.clone());
    let _ = s.save();

    let frame = win.borrow().frame;
    if !silent {
        frame.set_status_text("", 0);
    }
    match result {
        Check::Available(release) => {
            if silent && s.update.skipped_version == release.id() {
                return;
            }
            offer(win, release);
        }
        Check::UpToDate if !silent => info(
            &frame,
            &tr_args(
                "update.up_to_date",
                &[("version", &update::version_label())],
            ),
        ),
        Check::NoRelease if !silent => info(
            &frame,
            &tr_args(
                "update.no_release",
                &[("version", &update::version_label())],
            ),
        ),
        Check::Failed(e) if !silent => {
            error(&frame, &tr_args("update.check_failed", &[("error", &e)]))
        }
        _ => {}
    }
}

enum Choice {
    Install,
    Skip,
    Later,
}

fn offer(win: &WindowRef, release: Release) {
    let frame = win.borrow().frame;
    let kind = update::install_kind();
    let can_install = release.package(kind).is_some();
    match show_release_dialog(&frame, &release, can_install) {
        Choice::Install if can_install => install(win, release, kind),
        Choice::Install => crate::webpreview::open_external(&release.url),
        Choice::Skip => {
            let mut s = settings::get();
            s.update.skipped_version = release.id();
            settings::set(s.clone());
            let _ = s.save();
        }
        Choice::Later => {}
    }
}

/// 「新しい版があります」のダイアログ。リリースノートと Releases ページへのリンクを出す
fn show_release_dialog(frame: &Frame, release: &Release, can_install: bool) -> Choice {
    let dialog = Dialog::builder(frame, &tr("update.title"))
        .with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
        .build();
    // 寸法は DIP で書き、ダイアログのある画面の拡大率で実ピクセルにする
    let px = |v: i32| dialog.from_dip_int(v);

    // 開発版 (nightly) はその旨を見出しで断る
    let headline = StaticText::builder(&dialog)
        .with_label(&tr_args(
            if release.is_nightly() {
                "update.available_nightly"
            } else {
                "update.available"
            },
            &[
                ("new", &release.label()),
                ("current", &update::version_label()),
            ],
        ))
        .build();
    headline.wrap(px(520));
    let title = StaticText::builder(&dialog)
        .with_label(&release.title)
        .build();
    let notes = TextCtrl::builder(&dialog)
        .with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly)
        .build();
    let notes_text = if release.notes.trim().is_empty() {
        tr("update.no_notes")
    } else {
        release.notes.clone()
    };
    notes.set_value(&notes_text);
    // 複数行の TextCtrl は中身の行数 (最大 10 行) を最小の高さにするので、長いリリースノートだと
    // ダイアログの最小の高さが膨らみ、小さい画面でボタンが下にはみ出す。小さく抑えて残りを伸ばす
    notes.set_min_size(dialog.from_dip(Size::new(200, 80)));
    let link = HyperlinkCtrl::builder(&dialog)
        .with_label(&tr("update.open_releases"))
        .with_url(&release.url)
        .build();

    let install = Button::builder(&dialog)
        .with_id(ID_OK)
        .with_label(&tr(if can_install {
            "update.install"
        } else {
            "update.open_releases"
        }))
        .build();
    let skip = Button::builder(&dialog)
        .with_id(ID_SKIP)
        .with_label(&tr("update.skip"))
        .build();
    let later = Button::builder(&dialog)
        .with_id(ID_CANCEL)
        .with_label(&tr("update.later"))
        .build();
    skip.on_click({
        let dialog = dialog.clone();
        move |_| dialog.end_modal(ID_SKIP)
    });
    install.set_default();

    // リンクはボタンと同じ行に置かない (日本語のボタン 3 つと並べると幅が足りなくなる)
    let buttons = BoxSizer::builder(Orientation::Horizontal).build();
    buttons.add_stretch_spacer(1);
    buttons.add(&skip, 0, SizerFlag::Right, px(8));
    buttons.add(&later, 0, SizerFlag::Right, px(8));
    buttons.add(&install, 0, SizerFlag::Left, 0);

    let root = BoxSizer::builder(Orientation::Vertical).build();
    root.add(
        &headline,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Top,
        px(12),
    );
    root.add(
        &title,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Top,
        px(12),
    );
    root.add(&notes, 1, SizerFlag::Expand | SizerFlag::All, px(12));
    root.add(
        &link,
        0,
        SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(12),
    );
    root.add_sizer(
        &buttons,
        0,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(12),
    );
    // 最小サイズは中身 (見出しの折り返し・ボタンの幅) から求める。固定値だと言語や拡大率によっては
    // ボタンが入りきらず、ウィンドウを縮めたときに隠れる
    dialog.set_sizer_and_fit(root, true);
    let fitted = dialog.get_size();
    let floor = dialog.from_dip(Size::new(480, 320));
    let min = Size::new(
        fitted.width.max(floor.width),
        fitted.height.max(floor.height),
    );
    dialog.set_min_size(min);
    let want = dialog.from_dip(Size::new(560, 420));
    dialog.set_size(Size::new(
        want.width.max(min.width),
        want.height.max(min.height),
    ));
    dialog.centre();
    dialog.layout();

    let result = dialog.show_modal();
    dialog.destroy();
    match result {
        ID_OK => Choice::Install,
        ID_SKIP => Choice::Skip,
        _ => Choice::Later,
    }
}

/// 配布物をダウンロードして検証し、適用して再起動する
fn install(win: &WindowRef, release: Release, kind: InstallKind) {
    let frame = win.borrow().frame;
    let Some(asset) = release.package(kind).cloned() else {
        return;
    };
    let Some(sums) = release.sums().cloned() else {
        error(
            &frame,
            &tr_args("update.failed", &[("error", "SHA256SUMS.txt not found")]),
        );
        return;
    };
    let dest = update::download_dir().join(&asset.name);

    let dialog = ProgressDialog::builder(
        &frame,
        &tr("update.downloading_title"),
        &tr_args("update.downloading", &[("done", "0"), ("total", "?")]),
        PROGRESS_MAX,
    )
    .with_style(
        ProgressDialogStyle::AppModal
            | ProgressDialogStyle::CanAbort
            | ProgressDialogStyle::ElapsedTime
            | ProgressDialogStyle::RemainingTime,
    )
    .build();

    let state = Arc::new(Mutex::new(Transfer::default()));
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let state = state.clone();
        let cancel = cancel.clone();
        let dest = dest.clone();
        std::thread::spawn(move || {
            let size = (asset.size > 0).then_some(asset.size);
            let result = update::download(&asset.url, &dest, &cancel, |done, total| {
                if let Ok(mut s) = state.lock() {
                    s.done = done;
                    s.total = total.or(size);
                }
            })
            .and_then(|()| update::verify_with_sums(&dest, &sums.url, &asset.name));
            if result.is_err() {
                let _ = std::fs::remove_file(&dest);
            }
            if let Ok(mut s) = state.lock() {
                s.result = Some(result);
            }
        });
    }

    let download = Rc::new(Download {
        timer: Timer::new(&frame),
        dialog,
        state,
        cancel,
        release,
        kind,
        dest,
        finished: Cell::new(false),
    });
    let weak_dl = Rc::downgrade(&download);
    let weak_win = Rc::downgrade(win);
    download.timer.on_tick(move |_| {
        if let (Some(dl), Some(win)) = (weak_dl.upgrade(), weak_win.upgrade()) {
            poll_download(&win, &dl);
        }
    });
    download.timer.start(PROGRESS_POLL_MS, false);
    // 前のダウンロードのタイマーはここで入れ替わる (自分の tick の中で drop しないため、終了時には消さない)
    ACTIVE.with(|a| *a.borrow_mut() = Some(download));
}

/// タイマーから: 進捗を映し、終わっていれば次へ
fn poll_download(win: &WindowRef, dl: &Rc<Download>) {
    if dl.finished.get() {
        return;
    }
    let (done, total, result) = {
        let Ok(mut s) = dl.state.lock() else {
            return;
        };
        (s.done, s.total, s.result.take())
    };
    if let Some(result) = result {
        dl.finished.set(true);
        dl.timer.stop();
        dl.dialog.destroy();
        on_downloaded(win, dl, result);
        return;
    }
    let mb = |b: u64| format!("{:.1}", b as f64 / 1_048_576.0);
    let keep_going = match total {
        Some(total) if total > 0 => {
            let value = ((done as f64 / total as f64) * PROGRESS_MAX as f64) as i32;
            dl.dialog.update(
                value.min(PROGRESS_MAX - 1),
                Some(&tr_args(
                    "update.downloading",
                    &[("done", &mb(done)), ("total", &mb(total))],
                )),
            )
        }
        _ => dl.dialog.pulse(Some(&tr_args(
            "update.downloading",
            &[("done", &mb(done)), ("total", "?")],
        ))),
    };
    if !keep_going {
        dl.cancel.store(true, Ordering::Relaxed);
    }
}

fn on_downloaded(win: &WindowRef, dl: &Download, result: Result<(), String>) {
    let frame = win.borrow().frame;
    match result {
        Err(e) if e == update::CANCELLED => frame.set_status_text(&tr("update.cancelled"), 0),
        Err(e) => error(&frame, &tr_args("update.failed", &[("error", &e)])),
        Ok(()) => {
            // 再起動するので、未保存のタブは今のうちに保存してもらう
            if !window::confirm_close_all_windows() {
                frame.set_status_text(&tr("update.cancelled"), 0);
                return;
            }
            let files = window::open_paths();
            frame.set_status_text(&tr("update.restarting"), 0);
            match update::apply(&dl.dest, dl.kind, &files) {
                Ok(()) => window::close_all_for_restart(),
                Err(e) => error(
                    &frame,
                    &tr_args(
                        "update.apply_failed",
                        &[("error", &e), ("url", &dl.release.url)],
                    ),
                ),
            }
        }
    }
}

fn info(frame: &Frame, message: &str) {
    MessageDialog::builder(frame, message, &tr("update.title"))
        .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation)
        .build()
        .show_modal();
}

fn error(frame: &Frame, message: &str) {
    MessageDialog::builder(frame, message, &tr("update.title"))
        .with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
        .build()
        .show_modal();
}
