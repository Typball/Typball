//! 時間のかかるファイル操作を GUI を止めずに待つ
//!
//! 読み書きは別スレッドで行い、GUI スレッドはまず短時間 (`QUIET_MS`) だけ黙って待つ。
//! それで終わらなければ (不安定な USB メモリやネットワークドライブ) 進捗ダイアログを出し、
//! そのモーダルループでタイマーが結果を待つ。ユーザーは「キャンセル」で待つのをやめられる:
//! スレッドは止められない (ブロックした WriteFile は戻ってこない) が、GUI は動き続けるので、
//! 別の場所へ保存し直したり、そのまま編集を続けたりできる。結果は後から [`Pending`] で受け取る。
//!
//! 進捗ダイアログは wxDialog の `show_modal` を使う (wxProgressDialog の Pulse は Windows の
//! ネイティブ版だと GUI スレッドのメッセージを処理しないので、長く待つと「応答なし」になる)。

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use wxdragon::id::ID_CANCEL;
use wxdragon::prelude::*;

use crate::i18n::tr;

/// これだけ待っても終わらなければダイアログを出す
const QUIET_MS: u64 = 400;
/// ダイアログ表示中に結果を確かめる間隔
const POLL_MS: i32 = 50;

/// まだ終わっていない処理の受け口
pub type Pending<T> = Receiver<T>;

pub enum Outcome<T> {
    Done(T),
    /// ユーザーが待つのをやめた。処理は裏で続いていて、終われば受け取れる
    Abandoned(Pending<T>),
}

thread_local! {
    /// 進捗ダイアログを出して待っている最中 (入れ子を避ける)
    static WAITING: Cell<bool> = const { Cell::new(false) };
}

/// 進捗ダイアログで待っている最中か (外部変更の監視などはこの間は休む)
pub fn is_waiting() -> bool {
    WAITING.with(|w| w.get())
}

/// `work` を別スレッドで実行し、終わるまで待つ。長引けば `message` の進捗ダイアログを出す
///
/// `note` は進捗バーの下に出す説明 (待つのをやめたらどうなるか、途中で止まったらどうなるか)
pub fn run<T, F>(parent: &Frame, message: &str, note: &str, work: F) -> Outcome<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if is_waiting() {
        // 進捗ダイアログのモーダルループの中から呼ばれた (別プロセスからのファイル要求など)。
        // 入れ子のダイアログは避け、その場で実行する
        return Outcome::Done(work());
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    match rx.recv_timeout(Duration::from_millis(QUIET_MS)) {
        Ok(v) => return Outcome::Done(v),
        Err(RecvTimeoutError::Disconnected) => return Outcome::Abandoned(rx),
        Err(RecvTimeoutError::Timeout) => {}
    }
    WAITING.with(|w| w.set(true));
    let result = wait_with_dialog(parent, message, note, rx);
    WAITING.with(|w| w.set(false));
    result
}

fn wait_with_dialog<T: Send + 'static>(
    parent: &Frame,
    message: &str,
    note: &str,
    rx: Receiver<T>,
) -> Outcome<T> {
    let dialog = Dialog::builder(parent, "Typball")
        .with_style(DialogStyle::Caption | DialogStyle::SystemMenu)
        .build();
    let px = |v: i32| dialog.from_dip_int(v);
    let text = StaticText::builder(&dialog).with_label(message).build();
    let note = StaticText::builder(&dialog).with_label(note).build();
    let gauge = Gauge::builder(&dialog).with_range(100).build();
    let cancel = Button::builder(&dialog)
        .with_id(ID_CANCEL)
        .with_label(&tr("dialog.busy_cancel"))
        .build();
    let vbox = BoxSizer::builder(Orientation::Vertical).build();
    vbox.add(&text, 0, SizerFlag::Expand | SizerFlag::All, px(12));
    vbox.add(
        &gauge,
        0,
        SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right,
        px(12),
    );
    vbox.add(&note, 0, SizerFlag::Expand | SizerFlag::All, px(12));
    vbox.add(
        &cancel,
        0,
        SizerFlag::AlignRight | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
        px(12),
    );
    dialog.set_sizer(vbox, true);
    dialog.set_min_size(Size::new(px(420), -1));
    dialog.fit();
    dialog.centre();

    // 結果はタイマーが受け取り、モーダルループを終える。キャンセルは wx の既定の処理で
    // end_modal(ID_CANCEL) になる
    let slot: Rc<Cell<Option<T>>> = Rc::new(Cell::new(None));
    let rx = Rc::new(Cell::new(Some(rx)));
    let timer = Timer::new(&dialog);
    timer.on_tick({
        let dialog = dialog.clone();
        let gauge = gauge.clone();
        let slot = slot.clone();
        let rx = rx.clone();
        move |_| {
            // wxDragon に Pulse が無いので、バーを進め続けて「動いている」ことだけ示す
            gauge.set_value((gauge.get_value() + 4) % 100);
            let Some(receiver) = rx.take() else {
                return;
            };
            match receiver.try_recv() {
                Ok(v) => {
                    slot.set(Some(v));
                    dialog.end_modal(ID_OK_CODE);
                }
                Err(mpsc::TryRecvError::Empty) => rx.set(Some(receiver)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    // スレッドが panic した。受け口は閉じているので「終わらない」扱い
                    rx.set(Some(receiver));
                    dialog.end_modal(ID_CANCEL);
                }
            }
        }
    });
    timer.start(POLL_MS, false);
    dialog.show_modal();
    timer.stop();
    dialog.destroy();
    match slot.take() {
        Some(v) => Outcome::Done(v),
        None => match rx.take() {
            Some(receiver) => Outcome::Abandoned(receiver),
            None => unreachable!("receiver is returned to the slot on every path"),
        },
    }
}

const ID_OK_CODE: i32 = wxdragon::id::ID_OK;
