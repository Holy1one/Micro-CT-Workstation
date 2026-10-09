// Immediate MSI UI action: the Windows shell owns DPI layout and folder icons.
// No files are installed, created or removed; Cancel retains the current path.
#include <windows.h>
#include <msi.h>
#include <msiquery.h>
#include <shobjidl.h>
#include <shlobj.h>
#include <wrl/client.h>
#include <string>
#include <thread>
#include <vector>
#include <filesystem>

using Microsoft::WRL::ComPtr;

static std::wstring property(MSIHANDLE session, const wchar_t* name) {
    DWORD length = 0;
    wchar_t empty = L'\0';
    UINT status = MsiGetPropertyW(session, name, &empty, &length);
    if (status != ERROR_MORE_DATA && status != ERROR_SUCCESS) return {};
    std::vector<wchar_t> buffer(static_cast<size_t>(length) + 1);
    DWORD capacity = length + 1;
    if (MsiGetPropertyW(session, name, buffer.data(), &capacity) != ERROR_SUCCESS) return {};
    return std::wstring(buffer.data(), capacity);
}

class DpiScope {
    using SetContext = DPI_AWARENESS_CONTEXT(WINAPI*)(DPI_AWARENESS_CONTEXT);
    SetContext set_ = nullptr;
    DPI_AWARENESS_CONTEXT previous_ = nullptr;
public:
    DpiScope() {
        // Resolve dynamically so the action also loads on early Windows 10.
        auto address = GetProcAddress(GetModuleHandleW(L"user32.dll"), "SetThreadDpiAwarenessContext");
        if (address) {
            static_assert(sizeof(set_) == sizeof(address));
            memcpy(&set_, &address, sizeof(set_));
            previous_ = set_(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            if (!previous_) previous_ = set_(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE);
        }
    }
    ~DpiScope() { if (previous_) set_(previous_); }
};

static HWND installer_owner(const std::wstring& product) {
    HWND owner = GetForegroundWindow();
    wchar_t title[512] = {};
    wchar_t type[64] = {};
    GetWindowTextW(owner, title, ARRAYSIZE(title));
    GetClassNameW(owner, type, ARRAYSIZE(type));
    if (wcscmp(type, L"MsiDialogCloseClass") == 0 && std::wstring(title).find(product) == 0) return owner;
    struct Search { const std::wstring& product; HWND window; } search{product, nullptr};
    EnumWindows([](HWND window, LPARAM context) -> BOOL {
        auto& candidate = *reinterpret_cast<Search*>(context);
        wchar_t name[512] = {}, window_class[64] = {};
        GetWindowTextW(window, name, ARRAYSIZE(name));
        GetClassNameW(window, window_class, ARRAYSIZE(window_class));
        if (IsWindowVisible(window) && wcscmp(window_class, L"MsiDialogCloseClass") == 0 && std::wstring(name).find(candidate.product) == 0) {
            candidate.window = window;
            return FALSE;
        }
        return TRUE;
    }, reinterpret_cast<LPARAM>(&search));
    return search.window;
}

static HRESULT pick_folder(HWND owner, const std::wstring& current, std::wstring& selected) {
    DpiScope dpi;
    ULONG languages = 0;
    SetThreadPreferredUILanguages(MUI_LANGUAGE_NAME, L"en-US\0", &languages);
    SetThreadUILanguage(MAKELANGID(LANG_ENGLISH, SUBLANG_ENGLISH_US));
    HRESULT initialized = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    if (FAILED(initialized)) return initialized;
    HRESULT result = [&]() -> HRESULT {
        ComPtr<IFileOpenDialog> dialog;
        HRESULT hr = CoCreateInstance(CLSID_FileOpenDialog, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&dialog));
        if (FAILED(hr)) return hr;
        FILEOPENDIALOGOPTIONS options;
        hr = dialog->GetOptions(&options);
        if (FAILED(hr)) return hr;
        hr = dialog->SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR | FOS_DONTADDTORECENT);
        if (FAILED(hr)) return hr;
        dialog->SetTitle(L"Choose installation folder");
        dialog->SetOkButtonLabel(L"Select folder");
        // Find an existing ancestor when the default installation folder is new.
        auto initial = std::filesystem::path(current);
        std::error_code error;
        while (!initial.empty() && !std::filesystem::is_directory(initial, error)) {
            auto parent = initial.parent_path();
            if (parent == initial) break;
            initial = parent;
        }
        ComPtr<IShellItem> folder;
        if (!initial.empty() && SUCCEEDED(SHCreateItemFromParsingName(initial.c_str(), nullptr, IID_PPV_ARGS(&folder)))) dialog->SetFolder(folder.Get());
        hr = dialog->Show(owner);
        if (FAILED(hr)) return hr;
        ComPtr<IShellItem> item;
        hr = dialog->GetResult(&item);
        if (FAILED(hr)) return hr;
        PWSTR path = nullptr;
        hr = item->GetDisplayName(SIGDN_FILESYSPATH, &path);
        if (SUCCEEDED(hr)) {
            selected = path;
            CoTaskMemFree(path);
            if (!selected.empty() && selected.back() != L'\\') selected += L'\\';
        }
        return hr;
    }();
    CoUninitialize();
    return result;
}

extern "C" __declspec(dllexport) UINT __stdcall ChooseInstallFolder(MSIHANDLE session) {
    MsiSetPropertyW(session, L"NATIVE_FOLDER_SELECTED", L"0");
    const auto current = property(session, L"INSTALLDIR");
    const auto product = property(session, L"ProductName");
    HWND owner = installer_owner(product);
    std::wstring selected;
    HRESULT result = E_FAIL;
    try {
        // MSI's calling thread can be MTA. The shell picker requires STA.
        std::thread picker([&] { try { result = pick_folder(owner, current, selected); } catch (...) { result = E_FAIL; } });
        // The picker sends messages to its owner. Keep MSI's UI thread pumping
        // while the STA is active, including cross-thread activation messages.
        HANDLE worker = picker.native_handle();
        while (MsgWaitForMultipleObjectsEx(1, &worker, INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) == WAIT_OBJECT_0 + 1) {
            MSG message;
            while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        picker.join();
    } catch (...) { result = E_FAIL; }
    if (result == HRESULT_FROM_WIN32(ERROR_CANCELLED)) return ERROR_SUCCESS;
    if (FAILED(result) || selected.empty()) {
        MessageBoxW(owner, L"The folder chooser could not be opened. Try again or enter the installation path directly.", L"Micro-CT Workstation Setup", MB_OK | MB_ICONERROR);
        return ERROR_SUCCESS;
    }
    if (MsiSetTargetPathW(session, L"INSTALLDIR", selected.c_str()) != ERROR_SUCCESS) return ERROR_INSTALL_FAILURE;
    if (MsiSetPropertyW(session, L"INSTALLDIR", selected.c_str()) != ERROR_SUCCESS) return ERROR_INSTALL_FAILURE;
    if (MsiSetPropertyW(session, L"NATIVE_SELECTED_PATH", selected.c_str()) != ERROR_SUCCESS) return ERROR_INSTALL_FAILURE;
    return MsiSetPropertyW(session, L"NATIVE_FOLDER_SELECTED", L"1");
}
