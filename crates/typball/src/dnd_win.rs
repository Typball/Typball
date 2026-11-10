//! Windows 固有: ファイルのドロップ先 (OLE の IDropTarget を自前で実装)
//!
//! wx の wxFileDropTarget をウィンドウに付けると、そのウィンドウにもともと付いていたドロップ先
//! (エディタなら Scintilla のテキスト用) と入れ替わり、選択文字列をドラッグして動かす操作が
//! できなくなる。また wx は Explorer からのドロップに「移動」で応えるので、ポインタの説明文が
//! 「移動」になる (送り元によっては、移動で応えると元のファイルを消す合図になる)。
//!
//! ここでは IDropTarget を自前で実装してウィンドウに登録し、
//! - ファイルの一覧 (CF_HDROP) を持つドラッグは自分で受け、「コピー」の効果と
//!   「Typball で開く」の説明文 (DropDescription) で応える。説明文とドラッグ画像の追従には
//!   シェルの IDropTargetHelper を使う。
//! - それ以外 (テキストなど) は、もともと登録されていたドロップ先 (OLE がウィンドウプロパティ
//!   "OleDropTargetInterface" に入れている) にそのまま回す。
//!
//! ウィンドウが壊されるときに登録を外す (サブクラス化して WM_NCDESTROY を見る)。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::rc::Rc;

use wxdragon::prelude::*;

use crate::i18n::tr;

type Hwnd = *mut c_void;
type Hresult = i32;
type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
    Guid {
        data1,
        data2,
        data3,
        data4,
    }
}

const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IDROPTARGET: Guid = guid(0x122, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const CLSID_DRAGDROPHELPER: Guid = guid(
    0x4657278a,
    0x411b,
    0x11d2,
    [0x83, 0x9a, 0x00, 0xc0, 0x4f, 0xd9, 0x18, 0xd0],
);
const IID_IDROPTARGETHELPER: Guid = guid(
    0x4657278b,
    0x411b,
    0x11d2,
    [0x83, 0x9a, 0x00, 0xc0, 0x4f, 0xd9, 0x18, 0xd0],
);

#[repr(C)]
#[derive(Clone, Copy)]
struct PointL {
    x: i32,
    y: i32,
}

#[repr(C)]
struct FormatEtc {
    cf_format: u16,
    ptd: *mut c_void,
    dw_aspect: u32,
    lindex: i32,
    tymed: u32,
}

#[repr(C)]
struct StgMedium {
    tymed: u32,
    /// 共用体 (hGlobal など)。ポインタ幅
    data: *mut c_void,
    punk_for_release: *mut c_void,
}

/// シェルがドラッグ画像の脇に出す説明文 (shobjidl_core.h の DROPDESCRIPTION)
#[repr(C)]
struct DropDescription {
    kind: i32,
    message: [u16; 260],
    insert: [u16; 260],
}

const S_OK: Hresult = 0;
const E_NOINTERFACE: Hresult = 0x8000_4002u32 as i32;
const E_POINTER: Hresult = 0x8000_4003u32 as i32;
const CF_HDROP: u16 = 15;
const DVASPECT_CONTENT: u32 = 1;
const TYMED_HGLOBAL: u32 = 1;
const DROPEFFECT_NONE: u32 = 0;
const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_LINK: u32 = 4;
const DROPIMAGE_INVALID: i32 = -1;
const DROPIMAGE_COPY: i32 = 1;
const CLSCTX_INPROC_SERVER: u32 = 1;
/// GMEM_MOVEABLE | GMEM_ZEROINIT
const GHND: u32 = 0x0042;
const WM_NCDESTROY: u32 = 0x0082;
const SUBCLASS_ID: usize = 0x7479_7064; // "typd"

#[link(name = "ole32")]
extern "system" {
    fn RegisterDragDrop(hwnd: Hwnd, target: *mut c_void) -> Hresult;
    fn RevokeDragDrop(hwnd: Hwnd) -> Hresult;
    fn ReleaseStgMedium(medium: *mut StgMedium);
    fn CoCreateInstance(
        clsid: *const Guid,
        outer: *mut c_void,
        context: u32,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> Hresult;
}

#[link(name = "shell32")]
extern "system" {
    fn DragQueryFileW(drop: *mut c_void, index: u32, buf: *mut u16, len: u32) -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn GetPropW(hwnd: Hwnd, name: *const u16) -> *mut c_void;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn GlobalLock(handle: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(handle: *mut c_void) -> i32;
    fn GlobalFree(handle: *mut c_void) -> *mut c_void;
}

#[link(name = "comctl32")]
extern "system" {
    fn SetWindowSubclass(hwnd: Hwnd, pfn: SubclassProc, id: usize, ref_data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: Hwnd, pfn: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
}

// ---------------------------------------------------------------------------
// COM の仮想関数表 (使うメソッドだけ型を付ける。並びは SDK のヘッダどおり)
// ---------------------------------------------------------------------------

#[repr(C)]
struct IUnknownVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IDropTargetVtbl {
    base: IUnknownVtbl,
    drag_enter:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, PointL, *mut u32) -> Hresult,
    drag_over: unsafe extern "system" fn(*mut c_void, u32, PointL, *mut u32) -> Hresult,
    drag_leave: unsafe extern "system" fn(*mut c_void) -> Hresult,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, PointL, *mut u32) -> Hresult,
}

#[repr(C)]
struct IDataObjectVtbl {
    base: IUnknownVtbl,
    get_data: unsafe extern "system" fn(*mut c_void, *const FormatEtc, *mut StgMedium) -> Hresult,
    get_data_here: *const c_void,
    query_get_data: unsafe extern "system" fn(*mut c_void, *const FormatEtc) -> Hresult,
    get_canonical_format_etc: *const c_void,
    set_data:
        unsafe extern "system" fn(*mut c_void, *const FormatEtc, *mut StgMedium, i32) -> Hresult,
    enum_format_etc: *const c_void,
    d_advise: *const c_void,
    d_unadvise: *const c_void,
    enum_d_advise: *const c_void,
}

#[repr(C)]
struct IDropTargetHelperVtbl {
    base: IUnknownVtbl,
    drag_enter:
        unsafe extern "system" fn(*mut c_void, Hwnd, *mut c_void, *const PointL, u32) -> Hresult,
    drag_leave: unsafe extern "system" fn(*mut c_void) -> Hresult,
    drag_over: unsafe extern "system" fn(*mut c_void, *const PointL, u32) -> Hresult,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, *const PointL, u32) -> Hresult,
    show: unsafe extern "system" fn(*mut c_void, i32) -> Hresult,
}

/// COM オブジェクトの先頭にある仮想関数表
unsafe fn vtbl<'a, T>(obj: *mut c_void) -> &'a T {
    &**(obj as *const *const T)
}

unsafe fn com_add_ref(obj: *mut c_void) {
    (vtbl::<IUnknownVtbl>(obj).add_ref)(obj);
}

unsafe fn com_release(obj: *mut c_void) {
    (vtbl::<IUnknownVtbl>(obj).release)(obj);
}

// ---------------------------------------------------------------------------
// ドロップ先本体
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// ドラッグされていない、または受け付けないデータ
    Idle,
    /// ファイルの一覧を自分で受けている
    Files,
    /// もともとのドロップ先に回している
    Inner,
}

#[repr(C)]
struct Target {
    vtbl: *const IDropTargetVtbl,
    refs: Cell<u32>,
    hwnd: Hwnd,
    /// もともと登録されていたドロップ先 (無ければ null)。参照を 1 つ持つ
    inner: *mut c_void,
    /// シェルの IDropTargetHelper (作れなければ null)
    helper: *mut c_void,
    mode: Cell<Mode>,
    /// ファイルのドラッグ中に持っておくデータオブジェクト (離れたときに説明文を戻す)
    data: Cell<*mut c_void>,
    on_files: Rc<dyn Fn(Vec<String>) -> bool>,
}

static VTBL: IDropTargetVtbl = IDropTargetVtbl {
    base: IUnknownVtbl {
        query_interface,
        add_ref,
        release,
    },
    drag_enter,
    drag_over,
    drag_leave,
    drop: on_drop,
};

impl Drop for Target {
    fn drop(&mut self) {
        // SAFETY: どのポインタも自分で参照を取ったもの
        unsafe {
            self.forget_data();
            if !self.inner.is_null() {
                com_release(self.inner);
            }
            if !self.helper.is_null() {
                com_release(self.helper);
            }
        }
    }
}

impl Target {
    unsafe fn remember_data(&self, data: *mut c_void) {
        self.forget_data();
        if !data.is_null() {
            com_add_ref(data);
            self.data.set(data);
        }
    }

    unsafe fn forget_data(&self) {
        let data = self.data.replace(ptr::null_mut());
        if !data.is_null() {
            com_release(data);
        }
    }

    unsafe fn inner_vtbl(&self) -> &IDropTargetVtbl {
        vtbl::<IDropTargetVtbl>(self.inner)
    }

    unsafe fn helper_vtbl(&self) -> Option<&IDropTargetHelperVtbl> {
        (!self.helper.is_null()).then(|| vtbl::<IDropTargetHelperVtbl>(self.helper))
    }
}

unsafe fn target<'a>(this: *mut c_void) -> &'a Target {
    &*(this as *const Target)
}

unsafe extern "system" fn query_interface(
    this: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    if !iid.is_null() && (*iid == IID_IUNKNOWN || *iid == IID_IDROPTARGET) {
        add_ref(this);
        *out = this;
        S_OK
    } else {
        *out = ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    let t = target(this);
    t.refs.set(t.refs.get() + 1);
    t.refs.get()
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    let t = target(this);
    let n = t.refs.get().saturating_sub(1);
    t.refs.set(n);
    if n == 0 {
        std::mem::drop(Box::from_raw(this as *mut Target));
    }
    n
}

/// 送り元が許す効果のうち、ファイルを開くのにふさわしいもの。
/// 「移動」は返さない (送り元が元のファイルを消すことがある)
fn file_effect(allowed: u32) -> u32 {
    if allowed & DROPEFFECT_COPY != 0 {
        DROPEFFECT_COPY
    } else if allowed & DROPEFFECT_LINK != 0 {
        DROPEFFECT_LINK
    } else {
        DROPEFFECT_NONE
    }
}

unsafe extern "system" fn drag_enter(
    this: *mut c_void,
    data: *mut c_void,
    keys: u32,
    pt: PointL,
    effect: *mut u32,
) -> Hresult {
    let t = target(this);
    if effect.is_null() {
        return E_POINTER;
    }
    if !data.is_null() && has_files(data) {
        t.mode.set(Mode::Files);
        t.remember_data(data);
        set_description(data, DROPIMAGE_COPY, &tr("dnd.open"));
        *effect = file_effect(*effect);
        if let Some(h) = t.helper_vtbl() {
            (h.drag_enter)(t.helper, t.hwnd, data, &pt, *effect);
        }
        return S_OK;
    }
    if !t.inner.is_null() {
        t.mode.set(Mode::Inner);
        return (t.inner_vtbl().drag_enter)(t.inner, data, keys, pt, effect);
    }
    t.mode.set(Mode::Idle);
    *effect = DROPEFFECT_NONE;
    S_OK
}

unsafe extern "system" fn drag_over(
    this: *mut c_void,
    keys: u32,
    pt: PointL,
    effect: *mut u32,
) -> Hresult {
    let t = target(this);
    if effect.is_null() {
        return E_POINTER;
    }
    match t.mode.get() {
        Mode::Files => {
            *effect = file_effect(*effect);
            if let Some(h) = t.helper_vtbl() {
                (h.drag_over)(t.helper, &pt, *effect);
            }
            S_OK
        }
        Mode::Inner => (t.inner_vtbl().drag_over)(t.inner, keys, pt, effect),
        Mode::Idle => {
            *effect = DROPEFFECT_NONE;
            S_OK
        }
    }
}

unsafe extern "system" fn drag_leave(this: *mut c_void) -> Hresult {
    let t = target(this);
    let mode = t.mode.replace(Mode::Idle);
    match mode {
        Mode::Files => {
            // 説明文を既定に戻してから離す (戻さないと他のウィンドウでも「開く」のまま)
            let data = t.data.get();
            if !data.is_null() {
                set_description(data, DROPIMAGE_INVALID, "");
            }
            if let Some(h) = t.helper_vtbl() {
                (h.drag_leave)(t.helper);
            }
            t.forget_data();
            S_OK
        }
        Mode::Inner => (t.inner_vtbl().drag_leave)(t.inner),
        Mode::Idle => S_OK,
    }
}

unsafe extern "system" fn on_drop(
    this: *mut c_void,
    data: *mut c_void,
    keys: u32,
    pt: PointL,
    effect: *mut u32,
) -> Hresult {
    let t = target(this);
    if effect.is_null() {
        return E_POINTER;
    }
    let mode = t.mode.replace(Mode::Idle);
    match mode {
        Mode::Files => {
            let files = if data.is_null() {
                Vec::new()
            } else {
                read_files(data)
            };
            // パニックを FFI 境界の外へ出さない (プロセスが落ちる)
            let on_files = t.on_files.clone();
            let opened = !files.is_empty()
                && catch_unwind(AssertUnwindSafe(|| on_files(files))).unwrap_or(false);
            *effect = if opened {
                file_effect(*effect)
            } else {
                DROPEFFECT_NONE
            };
            if let Some(h) = t.helper_vtbl() {
                (h.drop)(t.helper, data, &pt, *effect);
            }
            t.forget_data();
            S_OK
        }
        Mode::Inner => (t.inner_vtbl().drop)(t.inner, data, keys, pt, effect),
        Mode::Idle => {
            *effect = DROPEFFECT_NONE;
            S_OK
        }
    }
}

// ---------------------------------------------------------------------------
// データオブジェクトの読み書き
// ---------------------------------------------------------------------------

const fn hdrop_format() -> FormatEtc {
    FormatEtc {
        cf_format: CF_HDROP,
        ptd: ptr::null_mut(),
        dw_aspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    }
}

/// ファイルの一覧 (CF_HDROP) を持っているか
unsafe fn has_files(data: *mut c_void) -> bool {
    let fe = hdrop_format();
    (vtbl::<IDataObjectVtbl>(data).query_get_data)(data, &fe) == S_OK
}

/// ファイルの一覧を取り出す
unsafe fn read_files(data: *mut c_void) -> Vec<String> {
    let fe = hdrop_format();
    let mut medium = StgMedium {
        tymed: 0,
        data: ptr::null_mut(),
        punk_for_release: ptr::null_mut(),
    };
    if (vtbl::<IDataObjectVtbl>(data).get_data)(data, &fe, &mut medium) != S_OK
        || medium.data.is_null()
    {
        return Vec::new();
    }
    let drop = medium.data;
    let count = DragQueryFileW(drop, 0xffff_ffff, ptr::null_mut(), 0);
    let mut files = Vec::with_capacity(count as usize);
    for i in 0..count {
        let len = DragQueryFileW(drop, i, ptr::null_mut(), 0);
        if len == 0 {
            continue;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let got = DragQueryFileW(drop, i, buf.as_mut_ptr(), len + 1);
        files.push(String::from_utf16_lossy(&buf[..got as usize]));
    }
    ReleaseStgMedium(&mut medium);
    files
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// "DropDescription" クリップボード形式の番号 (初回に登録する)
fn drop_description_format() -> u16 {
    thread_local! {
        static CF: Cell<u16> = const { Cell::new(0) };
    }
    CF.with(|cf| {
        if cf.get() == 0 {
            let name = wide("DropDescription");
            // SAFETY: NUL 終端の文字列
            cf.set(unsafe { RegisterClipboardFormatW(name.as_ptr()) } as u16);
        }
        cf.get()
    })
}

/// ドラッグ画像の説明文を送り元のデータオブジェクトに書く (対応していない送り元では無視される)
unsafe fn set_description(data: *mut c_void, kind: i32, message: &str) {
    let cf = drop_description_format();
    if cf == 0 {
        return;
    }
    let handle = GlobalAlloc(GHND, std::mem::size_of::<DropDescription>());
    if handle.is_null() {
        return;
    }
    let p = GlobalLock(handle) as *mut DropDescription;
    if p.is_null() {
        GlobalFree(handle);
        return;
    }
    (*p).kind = kind;
    for (dst, src) in (*p)
        .message
        .iter_mut()
        .zip(message.encode_utf16().take(259))
    {
        *dst = src;
    }
    GlobalUnlock(handle);
    let fe = FormatEtc {
        cf_format: cf,
        ptd: ptr::null_mut(),
        dw_aspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    };
    let mut medium = StgMedium {
        tymed: TYMED_HGLOBAL,
        data: handle,
        punk_for_release: ptr::null_mut(),
    };
    // 最後の引数 TRUE: 成功したらメモリの持ち主はデータオブジェクトになる
    if (vtbl::<IDataObjectVtbl>(data).set_data)(data, &fe, &mut medium, 1) != S_OK {
        GlobalFree(handle);
    }
}

// ---------------------------------------------------------------------------
// 登録と後始末
// ---------------------------------------------------------------------------

thread_local! {
    /// 登録したドロップ先 (HWND → Target)。ウィンドウが壊れるときに外して参照を返す
    static TARGETS: RefCell<HashMap<usize, *mut c_void>> = RefCell::new(HashMap::new());
}

/// ウィンドウにファイルのドロップ先を付ける。`on_files` はドロップされたパスを受け取り、
/// 開けたら true を返す。もともと付いていたドロップ先 (Scintilla のテキスト用など) は
/// ファイル以外のドラッグのためにそのまま使われる。
pub fn install<W: WxWidget>(window: &W, on_files: Rc<dyn Fn(Vec<String>) -> bool>) {
    let hwnd = window.get_handle();
    if hwnd.is_null() {
        return;
    }
    let prop = wide("OleDropTargetInterface");
    // SAFETY: hwnd は生存中のウィンドウ。OLE はこのスレッドで初期化済み (wx が行う)。
    // COM の呼び出しはすべて同じ UI スレッドから行う
    unsafe {
        let inner = GetPropW(hwnd, prop.as_ptr());
        if !inner.is_null() {
            com_add_ref(inner);
            RevokeDragDrop(hwnd);
        }
        let mut helper: *mut c_void = ptr::null_mut();
        if CoCreateInstance(
            &CLSID_DRAGDROPHELPER,
            ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IDROPTARGETHELPER,
            &mut helper,
        ) != S_OK
        {
            helper = ptr::null_mut();
        }
        let target = Box::into_raw(Box::new(Target {
            vtbl: &VTBL,
            refs: Cell::new(1),
            hwnd,
            inner,
            helper,
            mode: Cell::new(Mode::Idle),
            data: Cell::new(ptr::null_mut()),
            on_files,
        })) as *mut c_void;
        if RegisterDragDrop(hwnd, target) != S_OK {
            // 元に戻す
            if !inner.is_null() {
                RegisterDragDrop(hwnd, inner);
            }
            com_release(target);
            return;
        }
        // 自分の参照は WM_NCDESTROY まで持つ (OLE が持つ参照とは別)
        TARGETS.with(|m| m.borrow_mut().insert(hwnd as usize, target));
        SetWindowSubclass(hwnd, subclass_proc, SUBCLASS_ID, 0);
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: Hwnd,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    _data: usize,
) -> isize {
    if msg == WM_NCDESTROY {
        // wx が先に外していることもある (その場合は失敗するだけ)
        RevokeDragDrop(hwnd);
        let target = TARGETS.with(|m| m.borrow_mut().remove(&(hwnd as usize)));
        if let Some(t) = target {
            com_release(t);
        }
        RemoveWindowSubclass(hwnd, subclass_proc, SUBCLASS_ID);
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}
