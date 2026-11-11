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

// --- Typball patch: a labelled choice inside the file dialog ---
//
// wxDragon does not wrap wxFileDialogCustomizeHook. This shows the dialog with one extra
// drop-down list (on MSW it becomes a combo box in the native dialog, via IFileDialogCustomize)
// and reports which item was selected when the dialog was accepted.

#include "wx/filedlgcustomize.h"
#include <vector>

namespace {

class wxdChoiceHook : public wxFileDialogCustomizeHook {
public:
    wxdChoiceHook(const wxString& label, const std::vector<wxString>& items, int selection)
        : m_label(label), m_items(items), m_selection(selection), m_choice(nullptr)
    {
    }

    void
    AddCustomControls(wxFileDialogCustomize& customizer) override
    {
        if (m_items.empty())
            return;
#ifdef __WXMSW__
        // wxMSW numbers the items of the first choice downwards from (DWORD)-1, and
        // IFileDialogCustomize::SetSelectedControlItem silently ignores that ID, so the first
        // item could never be preselected. A hidden one-item choice takes that ID instead.
        const wxString placeholder(" ");
        if (wxFileDialogChoice* dummy = customizer.AddChoice(1, &placeholder))
            dummy->Hide();
#endif
        if (!m_label.empty())
            customizer.AddStaticText(m_label);
        m_choice = customizer.AddChoice(m_items.size(), m_items.data());
        if (m_choice && m_selection >= 0 && m_selection < (int)m_items.size())
            m_choice->SetSelection(m_selection);
    }

    void
    TransferDataFromCustomControls() override
    {
        if (m_choice)
            m_selection = m_choice->GetSelection();
    }

    int
    GetSelection() const
    {
        return m_selection;
    }

private:
    wxString m_label;
    std::vector<wxString> m_items;
    int m_selection;
    wxFileDialogChoice* m_choice;
};

} // namespace

WXD_EXPORTED int
wxd_FileDialog_ShowModalWithChoice(wxd_FileDialog_t* self, const char* label,
                                   const char* const* items, int count, int* selection)
{
    if (!self)
        return wxID_CANCEL;
    wxFileDialog* dlg = (wxFileDialog*)self;
    std::vector<wxString> wx_items;
    for (int i = 0; items && i < count; ++i)
        wx_items.push_back(WXD_STR_TO_WX_STRING_UTF8_NULL_OK(items[i]));
    wxdChoiceHook hook(WXD_STR_TO_WX_STRING_UTF8_NULL_OK(label), wx_items,
                       selection ? *selection : 0);
    // The hook must outlive ShowModal; it is not used after the dialog closes.
    dlg->SetCustomizeHook(hook);
    const int result = dlg->ShowModal();
    if (selection)
        *selection = hook.GetSelection();
    return result;
}
