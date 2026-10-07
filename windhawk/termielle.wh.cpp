// ==WindhawkMod==
// @id              termielle
// @name            Termielle
// @description     Termielle's shared Rust bar/pill/notch runtime in an isolated tool host. Keeps the native taskbar.
// @version         0.3.0
// @author          Termielle contributors
// @include         windhawk.exe
// @include         termielle-windhawk-host.exe
// @architecture    x86
// @architecture    x86-64
// @compilerOptions -lshell32
// @license         Apache-2.0
// ==/WindhawkMod==

// ==WindhawkModReadme==
/*
# Termielle — Windhawk tool edition (initial live preview)

Windhawk 1.7.3's manager is x86. Compile both adapters: x86 launches the owned
x64 `termielle-windhawk-host.exe`; Windhawk injects the x64 adapter into that
host, which loads `termielle_runtime.dll`. Matching assets/themes are required.
This is Termielle's own bar/pill/notch window, not an Explorer/taskbar-injected renderer.

Set PayloadDirectory to your built/installed payload. An empty Profile uses
~/.termielle/config.json. Standalone and hosted editions share the event pipe
and single-instance claim: exit standalone before enabling this mod. Persistent
Termielle Off remains authoritative; this mod will never silently clear it.

Profile retains Bar, Island and Notch; Classic becomes an ephemeral Island view.
Surface can explicitly select Bar, Island or Notch without changing the native
layout. The bar honors the profile's reserved-space preference; pill surfaces
never reserve. The native taskbar is never hidden/replaced. Profile surface settings are kept
on save. Name visibility, theme and session settings are shared. Layout selection
is read-only while hosted. Disable/re-enable to change payload/profile paths or
restart; profile edits reload in place. Do not enable the standalone logon task
and this mod at the same time unless you accept whichever frontend starts first.

The runtime DLL is one-shot and process-owned. It MUST NOT be unloaded or called
again in a long-lived process. Unload ends this dedicated host; no helper engine
or Explorer hooks are used. Existing decoder/OS workers die with their owner.

Tool-host compatibility pattern learned from devcode90/Dynamic-Island-for-Windows
(snapshot 0ce97cdd4e1d98c79ae2946d419004c35c6ffe12, MIT), and Windhawk's official
mods-as-tools documentation. See windhawk/LICENSES/Dynamic-Island-MIT.txt in the Termielle distribution.
No clipboard, weather, notification scraping or UIAccess behavior was imported.
*/
// ==/WindhawkModReadme==

// ==WindhawkModSettings==
/*
- PayloadDirectory: '%LOCALAPPDATA%\Termielle\bin'
  $name: Termielle payload directory
  $description: Absolute directory containing the matching termielle_runtime.dll, assets and themes. Disable/re-enable after changing.
- Profile: ''
  $name: Optional profile path
  $description: Absolute config.json path. Empty uses the shared user profile. Disable/re-enable after changing.
- Surface: Profile
  $name: Windhawk surface
  $description: Host-only appearance. Profile retains Bar/Island/Notch (Classic becomes Island). Bar honors saved reserved space. Never hides the native taskbar or changes the standalone layout. Disable/re-enable after changing.
  $options:
  - Profile: Follow profile
  - Bar: Full bar
  - Island: Floating Island
  - Notch: Attached Notch
*/
// ==/WindhawkModSettings==

#include <windhawk_api.h>
#include <shellapi.h>
#include <string>

namespace {
using Run = unsigned long(WINAPI*)(const unsigned char*, unsigned long);
using Stop = void(WINAPI*)();
using Abi = unsigned long(WINAPI*)();
HMODULE runtimeModule;
HANDLE runtimeThread;
Stop stopRuntime;
std::string request;
bool launcher;
HANDLE toolMutex;

std::wstring setting(PCWSTR name) {
    PCWSTR value = Wh_GetStringSetting(name);
    std::wstring result = value ? value : L"";
    if (value) Wh_FreeStringSetting(value);
    return result;
}
std::wstring expand(const std::wstring& value) {
    DWORD size = ExpandEnvironmentStringsW(value.c_str(), nullptr, 0);
    if (!size || size > 32768) return {};
    std::wstring result(size, L'\0');
    if (!ExpandEnvironmentStringsW(value.c_str(), result.data(), size)) return {};
    result.resize(size - 1);
    return result;
}
std::string quote(const std::wstring& value) {
    int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.c_str(),
                                  static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
    if (!size && !value.empty()) return {};
    std::string utf8(size, '\0');
    WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.c_str(),
                       static_cast<int>(value.size()), utf8.data(), size, nullptr, nullptr);
    std::string result = "\"";
    for (unsigned char c : utf8) {
        if (c == '"' || c == '\\') result += '\\';
        if (c < 0x20) return {};
        result += c;
    }
    return result + "\"";
}
DWORD WINAPI runtimeMain(void* entry) {
    auto run = reinterpret_cast<Run>(entry);
    DWORD result = run(reinterpret_cast<const unsigned char*>(request.data()),
                       static_cast<DWORD>(request.size()));
    Wh_Log(L"Termielle runtime ended: %lu (3=other frontend, 4=Off, 11=restart requested)", result);
    // Only our dedicated host. Do not leave process-owned workers resident or
    // unload their Rust code. Windhawk can create a fresh host on next enable.
    ExitProcess(result == 1 || result == 2 ? 1 : 0);
}
}

BOOL WhTool_ModInit() {
    if (sizeof(void*) != 8) { Wh_Log(L"Runtime needs the dedicated x64 host"); return FALSE; }
    auto directory = expand(setting(L"PayloadDirectory"));
    if (directory.empty() || directory.size() > 30000) return FALSE;
    auto path = directory + L"\\termielle_runtime.dll";
    runtimeModule = LoadLibraryExW(path.c_str(), nullptr,
                                  LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!runtimeModule) { Wh_Log(L"Cannot load matching Termielle runtime: %lu", GetLastError()); return FALSE; }
    auto abi = reinterpret_cast<Abi>(GetProcAddress(runtimeModule, "termielle_runtime_abi_v1"));
    auto run = reinterpret_cast<Run>(GetProcAddress(runtimeModule, "termielle_runtime_run_v1"));
    stopRuntime = reinterpret_cast<Stop>(GetProcAddress(runtimeModule, "termielle_runtime_stop_v1"));
    if (!abi || !run || !stopRuntime || abi() != 1) { Wh_Log(L"Unsupported Termielle ABI"); return FALSE; }
    auto quotedDirectory = quote(directory);
    if (quotedDirectory.empty()) return FALSE;
    request = "{\"abi_version\":1,\"payload_dir\":" + quotedDirectory;
    auto profile = expand(setting(L"Profile"));
    if (!profile.empty()) {
        auto quotedProfile = quote(profile);
        if (quotedProfile.empty()) return FALSE;
        request += ",\"config_path\":" + quotedProfile;
    }
    auto surface = setting(L"Surface");
    if (surface == L"Bar") request += ",\"surface\":\"bar\"";
    else if (surface == L"Island") request += ",\"surface\":\"island\"";
    else if (surface == L"Notch") request += ",\"surface\":\"notch\"";
    else if (!surface.empty() && surface != L"Profile") { Wh_Log(L"Unknown Surface setting"); return FALSE; }
    request += "}";
    if (request.size() > 16384) return FALSE;
    runtimeThread = CreateThread(nullptr, 0, runtimeMain, reinterpret_cast<void*>(run), 0, nullptr);
    return runtimeThread != nullptr;
}
void WhTool_ModSettingsChanged() {
    Wh_Log(L"Payload/profile/surface changes require disable/re-enable. Shared profile contents reload automatically.");
}
void WhTool_ModUninit() {
    if (stopRuntime) stopRuntime();
    if (runtimeThread) {
        if (WaitForSingleObject(runtimeThread, 5000) == WAIT_TIMEOUT)
            Wh_Log(L"UI teardown still pending; dedicated host exit will release its process-owned resources.");
        CloseHandle(runtimeThread);
        runtimeThread = nullptr;
    }
    // NEVER FreeLibrary: all Rust code remains mapped until this host exits.
}

// Dedicated-host compatibility loader, adapted from the MIT-licensed reference
// cited above. Only its lifecycle pattern is used; no Explorer/taskbar hooks.
void WINAPI toolEntryPoint() { ExitThread(0); }
BOOL Wh_ModInit() {
    DWORD session = 0;
    if (!ProcessIdToSessionId(GetCurrentProcessId(), &session) || session == 0) return FALSE;
    int argc = 0;
    LPWSTR* argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    if (!argv) return FALSE;
    bool tool = false, ours = false, excluded = false;
    for (int i = 1; i < argc; ++i) {
        if (wcscmp(argv[i], L"-service") == 0 || wcscmp(argv[i], L"-service-start") == 0 || wcscmp(argv[i], L"-service-stop") == 0) excluded = true;
        if (wcscmp(argv[i], L"-tool-mod") == 0 && i + 1 < argc) {
            tool = true; ours = wcscmp(argv[i + 1], WH_MOD_ID) == 0;
        }
    }
    LocalFree(argv);
    if (excluded || (tool && !ours)) return FALSE;
    if (!ours) {
        // Only Windhawk's manager may launch. An unflagged bootstrap host must
        // never recursively create more hosts.
        wchar_t executable[32768];
        DWORD length = GetModuleFileNameW(nullptr, executable, ARRAYSIZE(executable));
        if (!length || length >= ARRAYSIZE(executable)) return FALSE;
        PCWSTR name = wcsrchr(executable, L'\\');
        if (_wcsicmp(name ? name + 1 : executable, L"windhawk.exe") != 0) return FALSE;
        launcher = true; return TRUE;
    }
    toolMutex = CreateMutexW(nullptr, TRUE, L"windhawk-tool-mod_" WH_MOD_ID);
    if (!toolMutex || GetLastError() == ERROR_ALREADY_EXISTS) { ExitProcess(1); }
    if (!WhTool_ModInit()) { ExitProcess(1); }
    auto base = reinterpret_cast<unsigned char*>(GetModuleHandleW(nullptr));
    auto dos = reinterpret_cast<IMAGE_DOS_HEADER*>(base);
    auto nt = reinterpret_cast<IMAGE_NT_HEADERS*>(base + dos->e_lfanew);
    if (!Wh_SetFunctionHook(base + nt->OptionalHeader.AddressOfEntryPoint,
                            reinterpret_cast<void*>(toolEntryPoint), nullptr)) {
        Wh_Log(L"Cannot install dedicated-host entry-point hook");
        ExitProcess(1); // fail closed: never fall through to another Windhawk GUI
    }
    return TRUE;
}
void Wh_ModAfterInit() {
    if (!launcher) return;
    auto directory = expand(setting(L"PayloadDirectory"));
    if (directory.empty()) return;
    std::wstring path = directory + L"\\termielle-windhawk-host.exe";
    std::wstring command = L"\"" + path + L"\" -tool-mod \"" WH_MOD_ID L"\"";
    // Follow the supported 1.7.3 tool-loader path: bypass launcher interception
    // of public CreateProcessW. This resolves an OS export, not an Explorer hook.
    HMODULE kernel = GetModuleHandleW(L"kernelbase.dll");
    if (!kernel) kernel = GetModuleHandleW(L"kernel32.dll");
    using CreateInternal = BOOL(WINAPI*)(HANDLE, LPCWSTR, LPWSTR, LPSECURITY_ATTRIBUTES,
        LPSECURITY_ATTRIBUTES, BOOL, DWORD, LPVOID, LPCWSTR, LPSTARTUPINFOW,
        LPPROCESS_INFORMATION, PHANDLE);
    auto create = kernel ? reinterpret_cast<CreateInternal>(GetProcAddress(kernel, "CreateProcessInternalW")) : nullptr;
    if (!create) { Wh_Log(L"Cannot resolve dedicated-host launcher"); return; }
    STARTUPINFOW startup{sizeof(startup)};
    startup.dwFlags = STARTF_FORCEOFFFEEDBACK;
    PROCESS_INFORMATION process{};
    if (create(nullptr, path.c_str(), command.data(), nullptr, nullptr, FALSE, NORMAL_PRIORITY_CLASS, nullptr, nullptr, &startup, &process, nullptr)) {
        CloseHandle(process.hProcess); CloseHandle(process.hThread);
    } else { Wh_Log(L"Cannot start tool host: %lu", GetLastError()); }
}
void Wh_ModSettingsChanged() { if (!launcher) WhTool_ModSettingsChanged(); }
void Wh_ModUninit() {
    if (launcher) return;
    WhTool_ModUninit();
    ExitProcess(0); // only the dedicated tool process, never Explorer/service
}
