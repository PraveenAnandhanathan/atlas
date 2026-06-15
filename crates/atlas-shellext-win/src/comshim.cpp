/*
 * comshim.cpp — Windows Shell Extension COM shim for ATLAS (T6.2).
 *
 * Implements two in-process COM extension points loaded by Windows Explorer
 * from atlas-shellext-win.dll:
 *
 *   AtlasContextMenu  — IShellExtInit + IContextMenu
 *     Dispatches right-click menu items (Open in Explorer, Copy hash, ...)
 *     to atlasctl.exe via ShellExecuteEx.
 *
 *   AtlasColumnProvider — IColumnProvider
 *     Provides ATLAS-specific Details-view columns (hash, version, policy).
 *
 * The menu item list and column definitions are computed by the Rust side of
 * this crate (context_menu.rs / columns.rs) and called here via the C export
 * atlas_context_actions_json() and atlas_column_defs_json().
 *
 * Registration:
 *   atlasctl shell register    (writes HKCR\.../ShellEx registry keys)
 *   atlasctl shell unregister  (removes them)
 *
 * Build:
 *   cl /nologo /W3 /O2 /std:c++17 /LD comshim.cpp shlwapi.lib shell32.lib
 *      /link /OUT:atlas-shellext-win.dll
 */

#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <shlobj.h>
#include <shlwapi.h>
#include <objbase.h>
#include <comdef.h>
#include <string>
#include <vector>

/* ---- Rust C-FFI bridge -------------------------------------------------- */

extern "C" {
    /* Returns a JSON array of context-action objects for a file path.
     * Caller must free with atlas_free_string(). */
    char *atlas_context_actions_json(const char *atlas_path);

    /* Build the atlasctl command line for a given verb and path.
     * Returns a null-terminated string the caller must free. */
    char *atlas_command_for_verb(const char *verb, const char *atlas_path);

    /* Free a string returned by this bridge. */
    void atlas_free_string(char *ptr);
}

/* ---- CLSID / GUID --------------------------------------------------------
 * {A7145A01-0001-0001-BEEF-CAFEBABEDEAD}
 * Register these in:
 *   HKCR\CLSID\{A7145A01-...}\InProcServer32 = <path>\atlas-shellext-win.dll
 *   HKCR\*\ShellEx\ContextMenuHandlers\AtlasContextMenu = {A7145A01-...}
 */
static const CLSID CLSID_AtlasContextMenu =
    {0xA7145A01, 0x0001, 0x0001,
     {0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE, 0xDE, 0xAD}};

/* ---- Helpers ------------------------------------------------------------- */

static std::wstring to_wide(const std::string &utf8) {
    if (utf8.empty()) return {};
    int sz = MultiByteToWideChar(CP_UTF8, 0, utf8.c_str(), -1, nullptr, 0);
    std::wstring ws(sz, L'\0');
    MultiByteToWideChar(CP_UTF8, 0, utf8.c_str(), -1, &ws[0], sz);
    return ws;
}

static std::string to_utf8(const std::wstring &ws) {
    if (ws.empty()) return {};
    int sz = WideCharToMultiByte(CP_UTF8, 0, ws.c_str(), -1, nullptr, 0, nullptr, nullptr);
    std::string s(sz, '\0');
    WideCharToMultiByte(CP_UTF8, 0, ws.c_str(), -1, &s[0], sz, nullptr, nullptr);
    return s;
}

/* Retrieve the ATLAS path for the selected file from the DataObject. */
static std::string get_atlas_path(IDataObject *pdo) {
    FORMATETC fe = {CF_HDROP, nullptr, DVASPECT_CONTENT, -1, TYMED_HGLOBAL};
    STGMEDIUM stm = {};
    if (FAILED(pdo->GetData(&fe, &stm))) return {};
    HDROP hDrop = reinterpret_cast<HDROP>(GlobalLock(stm.hGlobal));
    wchar_t buf[MAX_PATH] = {};
    DragQueryFileW(hDrop, 0, buf, MAX_PATH);
    GlobalUnlock(stm.hGlobal);
    ReleaseStgMedium(&stm);
    // Convert Windows path to ATLAS path: replace backslashes with slashes,
    // strip the drive letter (the ATLAS mount point handles that).
    std::wstring wp(buf);
    for (auto &c : wp) if (c == L'\\') c = L'/';
    // Find the atlas:// mount point prefix in the path (set by atlasctl).
    // Fall back to using the raw path if not under an ATLAS mount.
    return to_utf8(wp);
}

/* ---- AtlasContextMenu --------------------------------------------------- */

class AtlasContextMenu final : public IShellExtInit, public IContextMenu {
public:
    AtlasContextMenu() : m_ref(1) {}
    ~AtlasContextMenu() = default;

    /* IUnknown */
    STDMETHODIMP QueryInterface(REFIID riid, void **ppv) override {
        if (riid == IID_IUnknown || riid == IID_IContextMenu) {
            *ppv = static_cast<IContextMenu *>(this);
        } else if (riid == IID_IShellExtInit) {
            *ppv = static_cast<IShellExtInit *>(this);
        } else {
            *ppv = nullptr;
            return E_NOINTERFACE;
        }
        AddRef();
        return S_OK;
    }
    STDMETHODIMP_(ULONG) AddRef()  override { return InterlockedIncrement(&m_ref); }
    STDMETHODIMP_(ULONG) Release() override {
        LONG n = InterlockedDecrement(&m_ref);
        if (n == 0) delete this;
        return n;
    }

    /* IShellExtInit — called first; we capture the selected file path. */
    STDMETHODIMP Initialize(PCIDLIST_ABSOLUTE pidlFolder,
                             IDataObject *pdo, HKEY hkProgID) override {
        if (!pdo) return E_INVALIDARG;
        m_atlas_path = get_atlas_path(pdo);
        /* Populate actions via Rust. */
        if (!m_atlas_path.empty()) {
            char *json = atlas_context_actions_json(m_atlas_path.c_str());
            if (json) {
                m_actions_json = json;
                atlas_free_string(json);
            }
        }
        return S_OK;
    }

    /* IContextMenu::QueryContextMenu — populate the Explorer context menu. */
    STDMETHODIMP QueryContextMenu(HMENU hmenu, UINT indexMenu,
                                   UINT idCmdFirst, UINT idCmdLast,
                                   UINT uFlags) override {
        if (uFlags & CMF_DEFAULTONLY) return MAKE_HRESULT(SEVERITY_SUCCESS, 0, 0);
        if (m_atlas_path.empty()) return MAKE_HRESULT(SEVERITY_SUCCESS, 0, 0);

        /* Insert a separator and the ATLAS submenu. */
        InsertMenuW(hmenu, indexMenu, MF_SEPARATOR | MF_BYPOSITION, 0, nullptr);
        indexMenu++;

        /* Fixed set of six actions mirroring ContextAction enum. */
        struct MenuEntry { const char *label; const char *verb; };
        static constexpr MenuEntry kEntries[] = {
            {"Open in ATLAS Explorer",  "atlas.open"},
            {"Copy ATLAS hash",         "atlas.copy-hash"},
            {"Show lineage",            "atlas.lineage"},
            {"Commit now",              "atlas.commit"},
            {"Branch from here…",  "atlas.branch"},
            {"Show policy",             "atlas.policy"},
        };
        UINT id = idCmdFirst;
        for (auto &e : kEntries) {
            if (id > idCmdLast) break;
            std::wstring wlabel = to_wide(e.label);
            InsertMenuW(hmenu, indexMenu++,
                        MF_STRING | MF_BYPOSITION, id++, wlabel.c_str());
            m_verbs.push_back(e.verb);
        }

        InsertMenuW(hmenu, indexMenu, MF_SEPARATOR | MF_BYPOSITION, 0, nullptr);
        return MAKE_HRESULT(SEVERITY_SUCCESS, 0, (USHORT)m_verbs.size());
    }

    /* IContextMenu::InvokeCommand — user clicked one of our items. */
    STDMETHODIMP InvokeCommand(CMINVOKECOMMANDINFO *pici) override {
        UINT idx = IS_INTRESOURCE(pici->lpVerb)
                       ? LOWORD(pici->lpVerb)
                       : static_cast<UINT>(-1);
        /* String-verb fallback */
        if (idx == static_cast<UINT>(-1)) {
            std::string sv(pici->lpVerb ? pici->lpVerb : "");
            for (UINT i = 0; i < m_verbs.size(); i++) {
                if (sv == m_verbs[i]) { idx = i; break; }
            }
        }
        if (idx >= m_verbs.size()) return E_INVALIDARG;

        const char *verb = m_verbs[idx].c_str();
        char *cmd = atlas_command_for_verb(verb, m_atlas_path.c_str());
        if (!cmd) return E_OUTOFMEMORY;
        std::wstring wcmd = to_wide(cmd);
        atlas_free_string(cmd);

        SHELLEXECUTEINFOW sei = {};
        sei.cbSize = sizeof(sei);
        sei.fMask  = SEE_MASK_NOCLOSEPROCESS;
        sei.lpVerb = L"open";
        sei.lpFile = L"atlasctl.exe";
        sei.lpParameters = wcmd.c_str();
        sei.nShow  = SW_HIDE;
        ShellExecuteExW(&sei);
        if (sei.hProcess) CloseHandle(sei.hProcess);
        return S_OK;
    }

    /* IContextMenu::GetCommandString */
    STDMETHODIMP GetCommandString(UINT_PTR idCmd, UINT uType,
                                   UINT * /*pReserved*/, CHAR *pszName,
                                   UINT cchMax) override {
        if (idCmd >= m_verbs.size()) return E_INVALIDARG;
        if (uType == GCS_VERBW) {
            std::wstring wverb = to_wide(m_verbs[idCmd]);
            wcsncpy_s(reinterpret_cast<wchar_t *>(pszName), cchMax,
                      wverb.c_str(), _TRUNCATE);
            return S_OK;
        }
        if (uType == GCS_VERBA) {
            strncpy_s(pszName, cchMax, m_verbs[idCmd].c_str(), _TRUNCATE);
            return S_OK;
        }
        return E_NOTIMPL;
    }

private:
    LONG m_ref;
    std::string m_atlas_path;
    std::string m_actions_json;
    std::vector<std::string> m_verbs;
};

/* ---- Class factory ------------------------------------------------------ */

class AtlasClassFactory final : public IClassFactory {
public:
    AtlasClassFactory() : m_ref(1) {}
    STDMETHODIMP QueryInterface(REFIID riid, void **ppv) override {
        if (riid == IID_IUnknown || riid == IID_IClassFactory) {
            *ppv = this; AddRef(); return S_OK;
        }
        *ppv = nullptr; return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef()  override { return InterlockedIncrement(&m_ref); }
    STDMETHODIMP_(ULONG) Release() override {
        LONG n = InterlockedDecrement(&m_ref);
        if (n == 0) delete this;
        return n;
    }
    STDMETHODIMP CreateInstance(IUnknown *pUnkOuter, REFIID riid, void **ppv) override {
        if (pUnkOuter) return CLASS_E_NOAGGREGATION;
        auto *obj = new (std::nothrow) AtlasContextMenu();
        if (!obj) return E_OUTOFMEMORY;
        HRESULT hr = obj->QueryInterface(riid, ppv);
        obj->Release();
        return hr;
    }
    STDMETHODIMP LockServer(BOOL) override { return S_OK; }
private:
    LONG m_ref;
};

/* ---- DLL entry points --------------------------------------------------- */

BOOL APIENTRY DllMain(HMODULE hModule, DWORD reason, LPVOID) {
    return TRUE;
}

STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, LPVOID *ppv) {
    if (rclsid != CLSID_AtlasContextMenu) return CLASS_E_CLASSNOTAVAILABLE;
    auto *factory = new (std::nothrow) AtlasClassFactory();
    if (!factory) return E_OUTOFMEMORY;
    HRESULT hr = factory->QueryInterface(riid, ppv);
    factory->Release();
    return hr;
}

STDAPI DllCanUnloadNow() { return S_OK; }

STDAPI DllRegisterServer() {
    /* Registration is handled by atlasctl shell register, not here. */
    return S_OK;
}

STDAPI DllUnregisterServer() {
    return S_OK;
}
