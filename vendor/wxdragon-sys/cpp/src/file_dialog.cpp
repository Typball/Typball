#include <wx/wxprec.h>
#include <wx/wx.h>
#include "wx/filedlg.h"
#include "wx/arrstr.h"
#include "../include/wxdragon.h"
#include "wxd_utils.h" // For WXD_STR_TO_WX_STRING_UTF8_NULL_OK and GET_WX_STRING_RESULT

extern void
wxd_ArrayString_AssignFromWxArrayString(wxd_ArrayString_t* target, const wxArrayString& source);

// --- wxFileDialog ---

WXD_EXPORTED wxd_FileDialog_t*
wxd_FileDialog_Create(
    wxd_Window_t* parent, const char* message, const char* defaultDir, const char* defaultFile,
    const char* wildcard, wxd_Style_t style, int x,
    int y, // Note: wxFileDialog doesn't directly take x,y,width,height in constructor
    int width, int height)
{ // These are for wxWindow, but FileDialog often uses default/platform sizing

    wxWindow* parent_wx = (wxWindow*)parent;
    wxFileDialog* dlg =
        new wxFileDialog(parent_wx, WXD_STR_TO_WX_STRING_UTF8_NULL_OK(message),
                         WXD_STR_TO_WX_STRING_UTF8_NULL_OK(defaultDir),
                         WXD_STR_TO_WX_STRING_UTF8_NULL_OK(defaultFile),
                         WXD_STR_TO_WX_STRING_UTF8_NULL_OK(wildcard), style,
                         wxd_cpp_utils::to_wx(wxd_Point{x, y}) // Position can be passed
                         // Size is typically not passed or is wxDefaultSize for FileDialog
        );
    // If width/height are not -1 (DEFAULT_SIZE), we could call SetSize, but usually not done for file dialogs.
    // Use OR (not AND) so that specifying just one axis still applies - only skip
    // when BOTH width and height are the -1 "unspecified" sentinel.
    if (width != -1 || height != -1) {
        wxSize size = wxd_cpp_utils::to_wx(wxd_Size{width, height});
        dlg->SetSize(size.GetWidth(), size.GetHeight());
    }
    return (wxd_FileDialog_t*)dlg;
}

// ShowModal is inherited from wxd_Dialog_ShowModal

WXD_EXPORTED int
wxd_FileDialog_GetPath(const wxd_FileDialog_t* self, char* buffer, size_t bufLen)
{
    if (!self)
        return -1;
    wxFileDialog* dlg = (wxFileDialog*)self;
    wxString str = dlg->GetPath();
    return (int)wxd_cpp_utils::copy_wxstring_to_buffer(str, buffer, bufLen);
}

WXD_EXPORTED void
wxd_FileDialog_GetPaths(const wxd_FileDialog_t* self, wxd_ArrayString_t* paths_out)
{
    if (!self || !paths_out)
        return;
    wxFileDialog* dlg = (wxFileDialog*)self;
    wxArrayString wx_paths;
    dlg->GetPaths(wx_paths);
    wxd_ArrayString_AssignFromWxArrayString(paths_out, wx_paths);
}

WXD_EXPORTED int
wxd_FileDialog_GetFilename(const wxd_FileDialog_t* self, char* buffer, size_t bufLen)
{
    if (!self)
        return -1;
    wxFileDialog* dlg = (wxFileDialog*)self;
    wxString str = dlg->GetFilename();
    return (int)wxd_cpp_utils::copy_wxstring_to_buffer(str, buffer, bufLen);
}

WXD_EXPORTED void
wxd_FileDialog_GetFilenames(const wxd_FileDialog_t* self, wxd_ArrayString_t* filenames_out)
{
    if (!self || !filenames_out)
        return;
    wxFileDialog* dlg = (wxFileDialog*)self;
    wxArrayString wx_filenames;
    dlg->GetFilenames(wx_filenames);
    wxd_ArrayString_AssignFromWxArrayString(filenames_out, wx_filenames);
}

WXD_EXPORTED int
wxd_FileDialog_GetDirectory(const wxd_FileDialog_t* self, char* buffer, size_t bufLen)
{
    if (!self)
        return -1;
    wxFileDialog* dlg = (wxFileDialog*)self;
    wxString str = dlg->GetDirectory();
    return (int)wxd_cpp_utils::copy_wxstring_to_buffer(str, buffer, bufLen);
}

WXD_EXPORTED int
wxd_FileDialog_GetFilterIndex(const wxd_FileDialog_t* self)
{
    if (!self)
        return -1; // Or some other error indicator, wxNOT_FOUND maybe?
    wxFileDialog* dlg = (wxFileDialog*)self;
    return dlg->GetFilterIndex();
}

// --- Optional Setters ---
WXD_EXPORTED void
wxd_FileDialog_SetMessage(const wxd_FileDialog_t* self, const char* message)
{
    if (!self)
        return;
    ((wxFileDialog*)self)->SetMessage(WXD_STR_TO_WX_STRING_UTF8_NULL_OK(message));
}

WXD_EXPORTED void
wxd_FileDialog_SetPath(const wxd_FileDialog_t* self, const char* path)
{
    if (!self)
        return;
    ((wxFileDialog*)self)->SetPath(WXD_STR_TO_WX_STRING_UTF8_NULL_OK(path));
}

WXD_EXPORTED void
wxd_FileDialog_SetDirectory(const wxd_FileDialog_t* self, const char* directory)
{
    if (!self)
        return;
    ((wxFileDialog*)self)->SetDirectory(WXD_STR_TO_WX_STRING_UTF8_NULL_OK(directory));
}

WXD_EXPORTED void
wxd_FileDialog_SetFilename(const wxd_FileDialog_t* self, const char* name)
{
    if (!self)
        return;
    ((wxFileDialog*)self)->SetFilename(WXD_STR_TO_WX_STRING_UTF8_NULL_OK(name));
}

WXD_EXPORTED void
wxd_FileDialog_SetWildcard(const wxd_FileDialog_t* self, const char* wildCard)
{
    if (!self)
        return;
    ((wxFileDialog*)self)->SetWildcard(WXD_STR_TO_WX_STRING_UTF8_NULL_OK(wildCard));
}

WXD_EXPORTED void
wxd_FileDialog_SetFilterIndex(const wxd_FileDialog_t* self, int filterIndex)
{
    if (!self)
        return;
    ((wxFileDialog*)self)->SetFilterIndex(filterIndex);
}

WXD_EXPORTED int
wxd_FileDialog_GetMessage(const wxd_FileDialog_t* self, char* buffer, size_t bufLen)
{
    if (!self)
        return -1;
    wxFileDialog* dlg = (wxFileDialog*)self;
    wxString msg = dlg->GetMessage();
    return (int)wxd_cpp_utils::copy_wxstring_to_buffer(msg, buffer, bufLen);
}

WXD_EXPORTED int
wxd_FileDialog_GetWildcard(const wxd_FileDialog_t* self, char* buffer, size_t bufLen)
{
    if (!self)
        return -1;
    wxFileDialog* dlg = (wxFileDialog*)self;
    wxString msg = dlg->GetWildcard();
    return (int)wxd_cpp_utils::copy_wxstring_to_buffer(msg, buffer, bufLen);
}

WXD_EXPORTED int
wxd_FileDialog_GetCurrentlySelectedFilterIndex(const wxd_FileDialog_t* self)
{
    if (!self)
        return -1;
    wxFileDialog* dlg = (wxFileDialog*)self;
#ifdef __WXMSW__
    return dlg->GetCurrentlySelectedFilterIndex();
#else
#ifdef __WXOSX__
    return dlg->GetCurrentlySelectedFilterIndex();
#else
    return -1; // Not implemented on this platform
#endif
#endif
}

// --- Typball patch: labelled choices inside the file dialog ---
//
// Shows the dialog with extra drop-down lists (the encoding and the line ending of the file to
// save) and reports which items were selected when the dialog was accepted.
//
// On MSW the native dialog (IFileDialog) is driven directly, so that the lists sit next to
// the Save button, like the encoding list of Notepad:
//
// - IFileDialogCustomize can put a label and a list there as one "visual group" made
//   "prominent" (MakeProminent), but only one: "Only one control can be marked in this way"
//   (a second call fails with E_UNEXPECTED, and a group holding two lists is not moved there
//   at all). The last list is added this way.
// - The label and the list of that group are plain child windows of the dialog (a "Static"
//   and a "ComboBox"), so the other lists are created the same way by us, as children of the
//   dialog, to the left of it (wxdButtonRow below).
// - Every list is also added the official way, as an ordinary group, which the dialog shows
//   in a row of its own below the file type. Those groups are kept hidden while our own
//   lists are shown, and are used instead when ours cannot be: the dialog is too narrow, or
//   the windows of the prominent group cannot be found (another Windows version may build
//   the dialog differently).
//
// wxFileDialogCustomize cannot do any of this (it only groups text fields with a label).
// Elsewhere, and if the native dialog cannot be created, wxFileDialogCustomizeHook is used,
// which wxDragon does not wrap.

#include "wx/filedlgcustomize.h"
#include <vector>

namespace {

// One labelled list. `selection` is the initial item, then the chosen one.
struct wxdChoice {
    wxString label;
    std::vector<wxString> items;
    int selection;
};

} // namespace

#ifdef __WXMSW__
#include "wx/filename.h"
#include "wx/utils.h"
#include <algorithm>
#include <commctrl.h>
#include <shobjidl.h>
#include <uxtheme.h>

namespace {

// Releases a COM interface pointer when it goes out of scope.
template <class T> struct wxdComPtr {
    T* p = nullptr;
    ~wxdComPtr()
    {
        if (p)
            p->Release();
    }
    T*
    operator->() const
    {
        return p;
    }
};

// IDs of the controls added to the native dialog (any values unique within the dialog).
DWORD
wxdGroupId(size_t choice)
{
    return (DWORD)(choice + 1) * 10 + 1;
}
DWORD
wxdComboId(size_t choice)
{
    return (DWORD)(choice + 1) * 10 + 2;
}
DWORD
wxdItemId(size_t choice, int item)
{
    return (DWORD)(choice + 1) * 1000 + (DWORD)item;
}

// --- Our own lists in the button row ---

// One of our lists: the windows we created for a choice.
struct wxdRowExtra {
    const wxdChoice* choice;
    HWND labelWnd;
    HWND comboWnd;
};

struct wxdButtonRow {
    HWND dialog = nullptr;
    // The prominent group made by the dialog itself (the list next to the Save button)
    HWND anchorLabel = nullptr;
    HWND anchorCombo = nullptr;
    // Our own lists, left to right
    std::vector<wxdRowExtra> extras;
    HFONT font = nullptr;
    bool inLayout = false;
    // The dialog is (about to be) visible. Before that it has a provisional size, far too
    // narrow for our lists, so whether they fit is not decided until it is shown: switching
    // to the official lists and back would make the visible dialog change its height.
    bool live = false;
    // Whether our lists have room in the row (they are hidden when they do not)
    bool fits = true;
    // Called (from a posted message, not from inside the layout) when `fits` has changed
    void (*onFitsChanged)(void* context) = nullptr;
    void* context = nullptr;
};

// Extra distance between the lists and the Save button, the distance between two lists, and
// the narrowest list we show, in pixels at 96 DPI (the dialog itself leaves 12 between the
// list and the button).
const int WXD_ROW_BUTTON_GAP = 20;
const int WXD_ROW_GROUP_GAP = 18;
const int WXD_ROW_MIN_COMBO = 90;
const UINT_PTR WXD_ROW_SUBCLASS = 0x5459;
const int WXD_ROW_FIRST_ID = 0x7100;
const UINT WXD_ROW_FITS_CHANGED = WM_APP + 0x545;

int
wxdRowScale(HWND hwnd, int px)
{
    // Windows 10 1607 and later; older systems are not per-monitor DPI aware anyway.
    typedef UINT(WINAPI * GetDpiForWindow_t)(HWND);
    static const GetDpiForWindow_t getDpi = (GetDpiForWindow_t)(void*)::GetProcAddress(
        ::GetModuleHandleW(L"user32.dll"), "GetDpiForWindow");
    const UINT dpi = getDpi ? getDpi(hwnd) : 96;
    return ::MulDiv(px, dpi ? dpi : 96, 96);
}

// The rectangle of a child window in the coordinates of its parent.
RECT
wxdRowRect(HWND hwnd)
{
    RECT r = {0, 0, 0, 0};
    ::GetWindowRect(hwnd, &r);
    ::MapWindowPoints(nullptr, ::GetParent(hwnd), (POINT*)&r, 2);
    return r;
}

int
wxdRowTextWidth(HWND hwnd, HFONT font, const wxString& text)
{
    HDC dc = ::GetDC(hwnd);
    HGDIOBJ old = font ? ::SelectObject(dc, font) : nullptr;
    SIZE size = {0, 0};
    ::GetTextExtentPoint32W(dc, text.wc_str(), (int)text.length(), &size);
    if (old)
        ::SelectObject(dc, old);
    ::ReleaseDC(hwnd, dc);
    return size.cx;
}

// Whether the dialog paints itself dark (it follows the app mode of Windows, not the theme
// of the application): ask it for the background of its own label.
bool
wxdRowIsDark(const wxdButtonRow& row)
{
    HDC dc = ::GetDC(row.anchorLabel);
    HBRUSH brush = (HBRUSH)::SendMessageW(row.dialog, WM_CTLCOLORSTATIC, (WPARAM)dc,
                                          (LPARAM)row.anchorLabel);
    COLORREF colour = ::GetBkColor(dc);
    LOGBRUSH lb;
    if (brush && ::GetObjectW(brush, sizeof(lb), &lb) == sizeof(lb) && lb.lbStyle == BS_SOLID)
        colour = lb.lbColor;
    ::ReleaseDC(row.anchorLabel, dc);
    return GetRValue(colour) * 299 + GetGValue(colour) * 587 + GetBValue(colour) * 114 < 128000;
}

// Puts our lists to the left of the prominent group, or hides them if there is no room.
// Called whenever the dialog has moved that group (it lays the row out again when its size
// changes).
void
wxdRowLayout(wxdButtonRow& row)
{
    if (row.inLayout || !::IsWindow(row.anchorLabel) || !::IsWindow(row.anchorCombo))
        return;
    row.inLayout = true;

    // The font follows the dialog (it changes with the DPI of the monitor).
    HFONT font = (HFONT)::SendMessageW(row.anchorCombo, WM_GETFONT, 0, 0);
    const bool newFont = font != row.font;
    row.font = font;

    const RECT label = wxdRowRect(row.anchorLabel);
    const RECT combo = wxdRowRect(row.anchorCombo);
    const int labelGap = combo.left - label.right;
    const int groupGap = wxdRowScale(row.dialog, WXD_ROW_GROUP_GAP);
    const int minCombo = wxdRowScale(row.dialog, WXD_ROW_MIN_COMBO);

    // Do not run into what the dialog keeps at the left of the row ("Hide Folders").
    int leftLimit = wxdRowScale(row.dialog, 12);
    for (HWND child = ::GetWindow(row.dialog, GW_CHILD); child;
         child = ::GetWindow(child, GW_HWNDNEXT)) {
        wchar_t cls[32] = L"";
        ::GetClassNameW(child, cls, 32);
        // Its own visibility, not IsWindowVisible(): this also runs just before the dialog is
        // shown, when nothing in it counts as visible yet.
        if (::lstrcmpiW(cls, L"ToolbarWindow32") != 0 ||
            !(::GetWindowLongW(child, GWL_STYLE) & WS_VISIBLE))
            continue;
        const RECT r = wxdRowRect(child);
        if (r.top < combo.bottom && r.bottom > combo.top && r.right < label.left)
            leftLimit = std::max(leftLimit, (int)r.right + groupGap);
    }

    // The widths wanted, and the least we can do with: every list at its narrowest.
    std::vector<int> labelWidths, comboWidths;
    int wanted = 0, least = 0;
    for (const wxdRowExtra& extra : row.extras) {
        if (newFont) {
            ::SendMessageW(extra.labelWnd, WM_SETFONT, (WPARAM)font, FALSE);
            ::SendMessageW(extra.comboWnd, WM_SETFONT, (WPARAM)font, FALSE);
        }
        const int labelWidth = wxdRowTextWidth(extra.labelWnd, font, extra.choice->label) + 2;
        int comboWidth = combo.right - combo.left;
        for (const wxString& item : extra.choice->items) {
            const int needed = wxdRowTextWidth(extra.comboWnd, font, item) +
                               ::GetSystemMetrics(SM_CXVSCROLL) + wxdRowScale(row.dialog, 14);
            comboWidth = std::max(comboWidth, needed);
        }
        labelWidths.push_back(labelWidth);
        comboWidths.push_back(comboWidth);
        wanted += labelWidth + labelGap + comboWidth + groupGap;
        least += labelWidth + labelGap + std::min(comboWidth, minCombo) + groupGap;
    }
    const int room = label.left - leftLimit;
    const bool fits = !row.live || room >= least;
    // When the dialog is narrow the lists give up width in equal parts.
    const int shortage = std::max(0, wanted - room);
    const int cut = row.extras.empty() ? 0 : (shortage + (int)row.extras.size() - 1) / (int)row.extras.size();

    int right = label.left - groupGap;
    for (size_t i = row.extras.size(); i-- > 0;) {
        const wxdRowExtra& extra = row.extras[i];
        if (!fits) {
            ::ShowWindow(extra.comboWnd, SW_HIDE);
            ::ShowWindow(extra.labelWnd, SW_HIDE);
            continue;
        }
        const int comboWidth = std::max(std::min(comboWidths[i], minCombo), comboWidths[i] - cut);
        const UINT flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW;
        ::SetWindowPos(extra.comboWnd, nullptr, right - comboWidth, combo.top, comboWidth,
                       combo.bottom - combo.top, flags);
        right -= comboWidth + labelGap;
        ::SetWindowPos(extra.labelWnd, nullptr, right - labelWidths[i], label.top, labelWidths[i],
                       label.bottom - label.top, flags);
        right -= labelWidths[i] + groupGap;
        ::InvalidateRect(extra.labelWnd, nullptr, TRUE);
    }
    if (fits != row.fits) {
        row.fits = fits;
        ::PostMessageW(row.anchorCombo, WXD_ROW_FITS_CHANGED, 0, 0);
    }
    row.inLayout = false;
}

// Subclass of the label and the list of the prominent group: keeps them a bit further from
// the Save button than the dialog puts them, and lays our lists out again after they moved.
LRESULT CALLBACK
wxdRowAnchorProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam, UINT_PTR id,
                 DWORD_PTR data)
{
    wxdButtonRow* const row = (wxdButtonRow*)data;
    if (msg == WM_WINDOWPOSCHANGING) {
        WINDOWPOS* const pos = (WINDOWPOS*)lParam;
        if (pos && !(pos->flags & SWP_NOMOVE))
            pos->x -= wxdRowScale(hwnd, WXD_ROW_BUTTON_GAP);
    }
    else if (msg == WXD_ROW_FITS_CHANGED) {
        if (row && row->onFitsChanged)
            row->onFitsChanged(row->context);
        return 0;
    }
    else if (msg == WM_NCDESTROY) {
        ::RemoveWindowSubclass(hwnd, wxdRowAnchorProc, id);
    }
    const LRESULT result = ::DefSubclassProc(hwnd, msg, wParam, lParam);
    if (msg == WM_WINDOWPOSCHANGED && row)
        wxdRowLayout(*row);
    return result;
}

// Subclass of the dialog itself: decides whether our lists fit just before the dialog
// becomes visible, when it has its real size.
LRESULT CALLBACK
wxdRowDialogProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam, UINT_PTR id, DWORD_PTR data)
{
    wxdButtonRow* const row = (wxdButtonRow*)data;
    const bool showing =
        (msg == WM_SHOWWINDOW && wParam) ||
        (msg == WM_WINDOWPOSCHANGING && lParam && (((WINDOWPOS*)lParam)->flags & SWP_SHOWWINDOW));
    if (showing && row && !row->live) {
        row->live = true;
        wxdRowLayout(*row);
        // Not from the posted message: the official lists must be in place before the
        // dialog is painted for the first time.
        if (!row->fits && row->onFitsChanged)
            row->onFitsChanged(row->context);
    }
    else if (msg == WM_NCDESTROY) {
        ::RemoveWindowSubclass(hwnd, wxdRowDialogProc, id);
    }
    return ::DefSubclassProc(hwnd, msg, wParam, lParam);
}

// Looks for the label and the list of the prominent group among the children of the dialog.
struct wxdRowFind {
    HWND dialog;
    const wxdChoice* choice;
    HWND labelWnd;
    HWND comboWnd;
};

BOOL CALLBACK
wxdRowFindProc(HWND hwnd, LPARAM lParam)
{
    wxdRowFind* const find = (wxdRowFind*)lParam;
    if (::GetParent(hwnd) != find->dialog)
        return TRUE;
    wchar_t cls[32] = L"";
    ::GetClassNameW(hwnd, cls, 32);
    wchar_t text[256] = L"";
    if (!find->labelWnd && ::lstrcmpiW(cls, L"Static") == 0) {
        ::GetWindowTextW(hwnd, text, 256);
        if (find->choice->label == text)
            find->labelWnd = hwnd;
    }
    else if (!find->comboWnd && ::lstrcmpiW(cls, L"ComboBox") == 0) {
        if (::SendMessageW(hwnd, CB_GETCOUNT, 0, 0) == (LRESULT)find->choice->items.size() &&
            ::SendMessageW(hwnd, CB_GETLBTEXTLEN, 0, 0) < 256) {
            ::SendMessageW(hwnd, CB_GETLBTEXT, 0, (LPARAM)text);
            if (find->choice->items[0] == text)
                find->comboWnd = hwnd;
        }
    }
    return TRUE;
}

// Finds the prominent group (the one of `anchor`) and creates a list for each of `extras`
// next to it. False if that group does not exist (yet) or the windows could not be created;
// nothing is left behind then.
bool
wxdRowBuild(wxdButtonRow& row, HWND dialog, const wxdChoice& anchor,
            const std::vector<const wxdChoice*>& extras)
{
    if (!dialog || !::IsWindow(dialog))
        return false;
    wxdRowFind find = {dialog, &anchor, nullptr, nullptr};
    ::EnumChildWindows(dialog, wxdRowFindProc, (LPARAM)&find);
    if (!find.labelWnd || !find.comboWnd)
        return false;
    row.dialog = dialog;
    row.anchorLabel = find.labelWnd;
    row.anchorCombo = find.comboWnd;
    // Built after the dialog was shown (from the timer): its size is already the real one.
    row.live = ::IsWindowVisible(dialog) != 0;

    HINSTANCE instance = (HINSTANCE)::GetWindowLongPtrW(dialog, GWLP_HINSTANCE);
    const bool dark = wxdRowIsDark(row);
    // Just before the prominent group in the Z order, which is also the tab order.
    HWND after = ::GetWindow(row.anchorLabel, GW_HWNDPREV);
    if (!after)
        after = HWND_TOP;
    bool ok = true;
    for (size_t i = 0; i < extras.size(); ++i) {
        wxdRowExtra extra = {extras[i], nullptr, nullptr};
        extra.labelWnd = ::CreateWindowExW(0, L"Static", extra.choice->label.wc_str(),
                                           WS_CHILD | SS_LEFT | SS_NOPREFIX, 0, 0, 0, 0, dialog,
                                           (HMENU)(INT_PTR)(WXD_ROW_FIRST_ID + 2 * i), instance,
                                           nullptr);
        extra.comboWnd = ::CreateWindowExW(
            0, L"ComboBox", L"", WS_CHILD | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST, 0, 0, 0,
            0, dialog, (HMENU)(INT_PTR)(WXD_ROW_FIRST_ID + 2 * i + 1), instance, nullptr);
        row.extras.push_back(extra);
        if (!extra.labelWnd || !extra.comboWnd) {
            ok = false;
            break;
        }
        for (const wxString& item : extra.choice->items)
            ::SendMessageW(extra.comboWnd, CB_ADDSTRING, 0, (LPARAM)item.wc_str());
        ::SendMessageW(extra.comboWnd, CB_SETCURSEL, extra.choice->selection, 0);
        if (dark) {
            // The theme the dialog itself uses for its lists when it is dark; uxtheme
            // ordinal 133 is AllowDarkModeForWindow.
            typedef bool(WINAPI * AllowDark_t)(HWND, bool);
            static const AllowDark_t allowDark = (AllowDark_t)(void*)::GetProcAddress(
                ::GetModuleHandleW(L"uxtheme.dll"), MAKEINTRESOURCEA(133));
            if (allowDark)
                allowDark(extra.comboWnd, true);
            ::SetWindowTheme(extra.comboWnd, L"DarkMode_CFD", nullptr);
        }
        ::SetWindowPos(extra.labelWnd, after, 0, 0, 0, 0,
                       SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        ::SetWindowPos(extra.comboWnd, extra.labelWnd, 0, 0, 0, 0,
                       SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        after = extra.comboWnd;
    }
    ok = ok && ::SetWindowSubclass(row.anchorLabel, wxdRowAnchorProc, WXD_ROW_SUBCLASS,
                                   (DWORD_PTR)&row);
    ok = ok && ::SetWindowSubclass(row.anchorCombo, wxdRowAnchorProc, WXD_ROW_SUBCLASS,
                                   (DWORD_PTR)&row);
    ok = ok && ::SetWindowSubclass(dialog, wxdRowDialogProc, WXD_ROW_SUBCLASS, (DWORD_PTR)&row);
    if (!ok) {
        ::RemoveWindowSubclass(row.anchorLabel, wxdRowAnchorProc, WXD_ROW_SUBCLASS);
        ::RemoveWindowSubclass(row.anchorCombo, wxdRowAnchorProc, WXD_ROW_SUBCLASS);
        ::RemoveWindowSubclass(dialog, wxdRowDialogProc, WXD_ROW_SUBCLASS);
        for (wxdRowExtra& extra : row.extras) {
            if (extra.labelWnd)
                ::DestroyWindow(extra.labelWnd);
            if (extra.comboWnd)
                ::DestroyWindow(extra.comboWnd);
        }
        row.extras.clear();
        row.anchorLabel = row.anchorCombo = nullptr;
        return false;
    }
    // Move the group to where it already is: the subclass shifts it, then lays our lists out.
    for (HWND hwnd : {row.anchorLabel, row.anchorCombo}) {
        const RECT r = wxdRowRect(hwnd);
        ::SetWindowPos(hwnd, nullptr, r.left, r.top, 0, 0,
                       SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    wxdRowLayout(row);
    return true;
}

// --- The dialog ---

// Receives the events of the dialog: builds our lists once its window exists, and keeps the
// official (hidden) lists and ours in step. Lives on the stack of wxdShowNativeWithChoices(),
// so the reference count is not used.
class wxdDialogEvents : public IFileDialogEvents {
public:
    wxdDialogEvents(IFileDialogCustomize* fdc, const std::vector<wxdChoice>& choices)
        : m_fdc(fdc), m_choices(choices), m_timer(0), m_tries(0), m_built(false),
          m_official(false)
    {
        m_row.onFitsChanged = &wxdDialogEvents::OnFitsChanged;
        m_row.context = this;
    }
    ~wxdDialogEvents()
    {
        StopTimer();
    }

    STDMETHODIMP
    QueryInterface(REFIID iid, void** out) override
    {
        if (!out)
            return E_POINTER;
        if (iid == IID_IUnknown || iid == IID_IFileDialogEvents) {
            *out = static_cast<IFileDialogEvents*>(this);
            return S_OK;
        }
        *out = nullptr;
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG)
    AddRef() override
    {
        return 1;
    }
    STDMETHODIMP_(ULONG)
    Release() override
    {
        return 1;
    }

    STDMETHODIMP
    OnFileOk(IFileDialog*) override
    {
        // The official lists are what is read after the dialog has closed.
        if (m_built && !m_official)
            CopyOursToOfficial();
        return S_OK;
    }
    STDMETHODIMP
    OnFolderChanging(IFileDialog* fd, IShellItem*) override
    {
        Build(fd);
        return S_OK;
    }
    STDMETHODIMP
    OnFolderChange(IFileDialog* fd) override
    {
        Build(fd);
        return S_OK;
    }
    STDMETHODIMP
    OnSelectionChange(IFileDialog* fd) override
    {
        Build(fd);
        return S_OK;
    }
    STDMETHODIMP
    OnShareViolation(IFileDialog*, IShellItem*, FDE_SHAREVIOLATION_RESPONSE*) override
    {
        return E_NOTIMPL; // the default handling
    }
    STDMETHODIMP
    OnTypeChange(IFileDialog* fd) override
    {
        Build(fd);
        return S_OK;
    }
    STDMETHODIMP
    OnOverwrite(IFileDialog*, IShellItem*, FDE_OVERWRITE_RESPONSE*) override
    {
        return E_NOTIMPL; // the default handling
    }

private:
    // Called for the events the dialog sends while it is being set up and used.
    //
    // The dialog sends OnTypeChange first, before it has created the prominent group, then
    // OnFolderChanging and OnFolderChange once the group exists, and only then shows itself.
    // Building our lists from those events puts them in place before anything is visible
    // (built later, they would be seen appearing one after another). The timer is only for
    // a dialog that sends no further event.
    void
    Build(IFileDialog* fd)
    {
        if (m_built || m_official || !fd)
            return;
        wxdComPtr<IOleWindow> window;
        if (FAILED(fd->QueryInterface(IID_IOleWindow, reinterpret_cast<void**>(&window.p))) ||
            !window.p)
            return;
        m_dialog = nullptr;
        if (FAILED(window->GetWindow(&m_dialog)) || !m_dialog)
            return;
        if (TryBuild()) {
            StopTimer();
        }
        else if (!m_timer) {
            s_waiting = this;
            m_timer = ::SetTimer(nullptr, 0, 30, &wxdDialogEvents::OnTimer);
        }
    }

    bool
    TryBuild()
    {
        std::vector<const wxdChoice*> extras;
        for (size_t c = 0; c + 1 < m_choices.size(); ++c)
            extras.push_back(&m_choices[c]);
        m_built = wxdRowBuild(m_row, m_dialog, m_choices.back(), extras);
        return m_built;
    }

    // Shows the lists the official way (a row of their own) instead of ours, or back.
    void
    UseOfficial(bool official)
    {
        if (official == m_official)
            return;
        m_official = official;
        if (m_built) {
            if (official)
                CopyOursToOfficial();
            else
                CopyOfficialToOurs();
        }
        const CDCONTROLSTATEF state = official ? CDCS_ENABLEDVISIBLE : CDCS_INACTIVE;
        for (size_t c = 0; c + 1 < m_choices.size(); ++c) {
            m_fdc->SetControlState(wxdComboId(c), state);
            m_fdc->SetControlState(wxdGroupId(c), state);
        }
    }

    void
    CopyOursToOfficial()
    {
        for (size_t c = 0; c < m_row.extras.size(); ++c) {
            const LRESULT selected = ::SendMessageW(m_row.extras[c].comboWnd, CB_GETCURSEL, 0, 0);
            if (selected >= 0 && selected < (LRESULT)m_choices[c].items.size())
                m_fdc->SetSelectedControlItem(wxdComboId(c), wxdItemId(c, (int)selected));
        }
    }

    void
    CopyOfficialToOurs()
    {
        for (size_t c = 0; c < m_row.extras.size(); ++c) {
            const DWORD first = wxdItemId(c, 0);
            DWORD selected = 0;
            if (SUCCEEDED(m_fdc->GetSelectedControlItem(wxdComboId(c), &selected)) &&
                selected >= first && selected < first + (DWORD)m_choices[c].items.size())
                ::SendMessageW(m_row.extras[c].comboWnd, CB_SETCURSEL, selected - first, 0);
        }
    }

    static void
    OnFitsChanged(void* context)
    {
        wxdDialogEvents* const self = (wxdDialogEvents*)context;
        self->UseOfficial(!self->m_row.fits);
    }

    void
    StopTimer()
    {
        if (m_timer) {
            ::KillTimer(nullptr, m_timer);
            m_timer = 0;
        }
        if (s_waiting == this)
            s_waiting = nullptr;
    }

    static VOID CALLBACK
    OnTimer(HWND, UINT, UINT_PTR id, DWORD)
    {
        wxdDialogEvents* const self = s_waiting;
        if (!self || self->m_timer != id) {
            ::KillTimer(nullptr, id);
            return;
        }
        if (self->TryBuild()) {
            self->StopTimer();
        }
        else if (++self->m_tries >= 30) {
            // About a second in all: the prominent group either shows up right away or the
            // dialog is not built the way we expect. Fall back to the official lists.
            self->StopTimer();
            self->UseOfficial(true);
        }
    }

    // The one object whose timer is running (file dialogs are modal, and only the GUI thread
    // shows them).
    static wxdDialogEvents* s_waiting;

    IFileDialogCustomize* const m_fdc;
    const std::vector<wxdChoice>& m_choices;
    wxdButtonRow m_row;
    HWND m_dialog = nullptr;
    UINT_PTR m_timer;
    int m_tries;
    // Our lists exist in the button row
    bool m_built;
    // The official lists are shown (ours do not exist or have no room)
    bool m_official;
};

wxdDialogEvents* wxdDialogEvents::s_waiting = nullptr;

// Shows the native dialog configured like wxFileDialog::ShowIFileDialog() does (wxWidgets
// src/msw/filedlg.cpp), with the labelled lists added. Returns wxID_NONE if the dialog could
// not be set up, in which case nothing was shown.
int
wxdShowNativeWithChoices(wxFileDialog* dlg, std::vector<wxdChoice>& choices)
{
    if (choices.empty() || dlg->HasFlag(wxFD_MULTIPLE))
        return wxID_NONE;
    for (const wxdChoice& choice : choices) {
        if (choice.label.empty() || choice.items.empty())
            return wxID_NONE;
    }

    wxdComPtr<IFileDialog> fd;
    HRESULT hr = ::CoCreateInstance(
        dlg->HasFlag(wxFD_SAVE) ? CLSID_FileSaveDialog : CLSID_FileOpenDialog, nullptr,
        CLSCTX_INPROC_SERVER, IID_IFileDialog, reinterpret_cast<void**>(&fd.p));
    if (FAILED(hr) || !fd.p)
        return wxID_NONE;
    wxdComPtr<IFileDialogCustomize> fdc;
    hr = fd->QueryInterface(IID_IFileDialogCustomize, reinterpret_cast<void**>(&fdc.p));
    if (FAILED(hr) || !fdc.p)
        return wxID_NONE;

    // Each label and its list as one group. The last one goes next to the Save button; the
    // others stay hidden as long as our own lists stand in for them (see the top of this
    // section).
    for (size_t c = 0; c < choices.size(); ++c) {
        const wxdChoice& choice = choices[c];
        if (FAILED(fdc->StartVisualGroup(wxdGroupId(c), choice.label.wc_str())))
            return wxID_NONE;
        if (FAILED(fdc->AddComboBox(wxdComboId(c))))
            return wxID_NONE;
        const int count = (int)choice.items.size();
        for (int i = 0; i < count; ++i)
            fdc->AddControlItem(wxdComboId(c), wxdItemId(c, i), choice.items[i].wc_str());
        const int initial =
            (choice.selection >= 0 && choice.selection < count) ? choice.selection : 0;
        fdc->SetSelectedControlItem(wxdComboId(c), wxdItemId(c, initial));
        fdc->EndVisualGroup();
    }
    // If the last group cannot be made prominent, all the lists are simply shown the official
    // way and nothing of ours is needed.
    const bool prominent = SUCCEEDED(fdc->MakeProminent(wxdGroupId(choices.size() - 1)));
    if (prominent) {
        for (size_t c = 0; c + 1 < choices.size(); ++c) {
            fdc->SetControlState(wxdComboId(c), CDCS_INACTIVE);
            fdc->SetControlState(wxdGroupId(c), CDCS_INACTIVE);
        }
    }

    fd->SetTitle(dlg->GetMessage().wc_str());

    wxArrayString descriptions, filters;
    const int filterCount =
        wxParseCommonDialogsFilter(dlg->GetWildcard(), descriptions, filters);
    if (filterCount > 0) {
        std::vector<COMDLG_FILTERSPEC> specs(filterCount);
        for (int i = 0; i < filterCount; ++i) {
            specs[i].pszName = descriptions[i].wc_str();
            specs[i].pszSpec = filters[i].wc_str();
        }
        fd->SetFileTypes(filterCount, specs.data());
        int index = dlg->GetFilterIndex();
        if (index < 0 || index >= filterCount)
            index = 0;
        fd->SetFileTypeIndex(index + 1);
        // Without a default extension the dialog never appends one; with it, the dialog
        // appends the extension of the file type that is selected when it is accepted.
        const wxString ext = filters[index].BeforeFirst(';').AfterFirst('.');
        if (!ext.empty() && ext != wxS("*"))
            fd->SetDefaultExtension(ext.wc_str());
    }

    wxString dir = dlg->GetDirectory();
    if (!dir.empty()) {
        // SHCreateItemFromParsingName() does not accept slashes. A folder that does not
        // exist is not an error, the dialog just opens where it wants to.
        dir.Replace("/", "\\", true);
        wxdComPtr<IShellItem> folder;
        if (SUCCEEDED(::SHCreateItemFromParsingName(dir.wc_str(), nullptr, IID_IShellItem,
                                                    reinterpret_cast<void**>(&folder.p))) &&
            folder.p)
            fd->SetFolder(folder.p);
    }
    if (!dlg->GetFilename().empty())
        fd->SetFileName(dlg->GetFilename().wc_str());

    DWORD options = FOS_FORCEFILESYSTEM;
    if (dlg->HasFlag(wxFD_OVERWRITE_PROMPT))
        options |= FOS_OVERWRITEPROMPT;
    if (!dlg->HasFlag(wxFD_CHANGE_DIR))
        options |= FOS_NOCHANGEDIR;
    if (dlg->HasFlag(wxFD_FILE_MUST_EXIST))
        options |= FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST;
    if (dlg->HasFlag(wxFD_SHOW_HIDDEN))
        options |= FOS_FORCESHOWHIDDEN;
    if (dlg->HasFlag(wxFD_NO_FOLLOW))
        options |= FOS_NODEREFERENCELINKS;
    if (FAILED(fd->SetOptions(options)))
        return wxID_NONE;

    // The native dialog disables its owner itself; the other top level windows are ours to
    // disable.
    wxWindow* const parent = dlg->GetParentForModalDialog();
    wxWindowDisabler disableOthers(dlg, parent);
    wxdDialogEvents events(fdc.p, choices);
    DWORD cookie = 0;
    const bool advised = prominent && SUCCEEDED(fd->Advise(&events, &cookie));
    if (prominent && !advised) {
        // Without the events our lists cannot be built: show the official ones.
        for (size_t c = 0; c + 1 < choices.size(); ++c) {
            fdc->SetControlState(wxdComboId(c), CDCS_ENABLEDVISIBLE);
            fdc->SetControlState(wxdGroupId(c), CDCS_ENABLEDVISIBLE);
        }
    }
    hr = fd->Show(parent ? (HWND)parent->GetHWND() : nullptr);
    if (advised)
        fd->Unadvise(cookie);

    // A double click that accepted the dialog may leave mouse messages behind, which would
    // otherwise reach the window under the dialog.
    MSG msg;
    for (int i = 0; i < 1000; ++i) {
        if (!::PeekMessage(&msg, nullptr, WM_MOUSEFIRST, WM_MOUSELAST, PM_REMOVE | PM_QS_INPUT))
            break;
    }

    if (FAILED(hr))
        return wxID_CANCEL; // cancelled by the user (ERROR_CANCELLED), or failed to show

    wxdComPtr<IShellItem> item;
    if (FAILED(fd->GetResult(&item.p)) || !item.p)
        return wxID_CANCEL;
    LPWSTR chosen = nullptr;
    if (FAILED(item->GetDisplayName(SIGDN_FILESYSPATH, &chosen)) || !chosen)
        return wxID_CANCEL;
    const wxString path(chosen);
    ::CoTaskMemFree(chosen);

    // Make GetPath(), GetFilename(), GetDirectory() and GetFilterIndex() report the result.
    dlg->SetPath(path);
    UINT typeIndex = 0;
    if (SUCCEEDED(fd->GetFileTypeIndex(&typeIndex)) && typeIndex > 0)
        dlg->SetFilterIndex((int)typeIndex - 1);
    // The selections of our own lists were copied to the official ones when the dialog was
    // accepted (wxdDialogEvents::OnFileOk).
    for (size_t c = 0; c < choices.size(); ++c) {
        const DWORD first = wxdItemId(c, 0);
        DWORD selected = 0;
        if (SUCCEEDED(fdc->GetSelectedControlItem(wxdComboId(c), &selected)) &&
            selected >= first && selected < first + (DWORD)choices[c].items.size())
            choices[c].selection = (int)(selected - first);
    }
    return wxID_OK;
}

} // namespace
#endif // __WXMSW__

namespace {

class wxdChoiceHook : public wxFileDialogCustomizeHook {
public:
    explicit wxdChoiceHook(std::vector<wxdChoice>& choices)
        : m_choices(choices), m_controls(choices.size(), nullptr)
    {
    }

    void
    AddCustomControls(wxFileDialogCustomize& customizer) override
    {
#ifdef __WXMSW__
        // wxMSW numbers the items of the first choice downwards from (DWORD)-1, and
        // IFileDialogCustomize::SetSelectedControlItem silently ignores that ID, so the first
        // item could never be preselected. A hidden one-item choice takes that ID instead.
        const wxString placeholder(" ");
        if (wxFileDialogChoice* dummy = customizer.AddChoice(1, &placeholder))
            dummy->Hide();
#endif
        for (size_t c = 0; c < m_choices.size(); ++c) {
            const wxdChoice& choice = m_choices[c];
            if (choice.items.empty())
                continue;
            if (!choice.label.empty())
                customizer.AddStaticText(choice.label);
            m_controls[c] = customizer.AddChoice(choice.items.size(), choice.items.data());
            if (m_controls[c] && choice.selection >= 0 &&
                choice.selection < (int)choice.items.size())
                m_controls[c]->SetSelection(choice.selection);
        }
    }

    void
    TransferDataFromCustomControls() override
    {
        for (size_t c = 0; c < m_choices.size(); ++c) {
            if (m_controls[c])
                m_choices[c].selection = m_controls[c]->GetSelection();
        }
    }

private:
    std::vector<wxdChoice>& m_choices;
    std::vector<wxFileDialogChoice*> m_controls;
};

} // namespace

// `labels` and `counts` have `choices` entries; `items` holds the items of all the lists one
// after another (`counts[i]` of them for list i). `selections` (also `choices` entries) gives
// the initial item of each list and receives the chosen ones.
WXD_EXPORTED int
wxd_FileDialog_ShowModalWithChoices(wxd_FileDialog_t* self, int choices,
                                    const char* const* labels, const char* const* items,
                                    const int* counts, int* selections)
{
    if (!self)
        return wxID_CANCEL;
    wxFileDialog* dlg = (wxFileDialog*)self;
    std::vector<wxdChoice> wx_choices;
    if (labels && items && counts && selections) {
        int next = 0;
        for (int c = 0; c < choices; ++c) {
            wxdChoice choice;
            choice.label = WXD_STR_TO_WX_STRING_UTF8_NULL_OK(labels[c]);
            for (int i = 0; i < counts[c]; ++i) {
                // The macro evaluates its argument twice: no side effects inside it.
                const char* const item = items[next++];
                choice.items.push_back(WXD_STR_TO_WX_STRING_UTF8_NULL_OK(item));
            }
            choice.selection = selections[c];
            wx_choices.push_back(choice);
        }
    }
    int result = wxID_NONE;
#ifdef __WXMSW__
    result = wxdShowNativeWithChoices(dlg, wx_choices);
#endif
    if (result == wxID_NONE) {
        wxdChoiceHook hook(wx_choices);
        // The hook must outlive ShowModal; it is not used after the dialog closes.
        dlg->SetCustomizeHook(hook);
        result = dlg->ShowModal();
    }
    for (size_t c = 0; c < wx_choices.size(); ++c)
        selections[c] = wx_choices[c].selection;
    return result;
}
