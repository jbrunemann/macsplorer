# explorer.exe findings (Windows 10.0.10240.21072, x64)

Target: `1473C777…681100.blob`, SHA-256 `774b02e3…2e366f`, OriginalFilename `EXPLORER.EXE`.
Symbols: `explorer.pdb` (GUID `6AEA0EEA60AA42B5B2AD1AA39E681E0D`, age 1) from the Microsoft symbol server.
Method: static only. PDB public symbols + import table + `llvm-objdump` disassembly. Nothing was executed.

Files in `symbols/`:

| File | Content |
|---|---|
| `explorer.pdb` | Public PDB (no types, no locals) |
| `explorer.pubs.tsv` | `VA \t F/D \t mangled name`, 16,446 symbols (10,093 functions). Demangle with `llvm-cxxfilt`. |
| `explorer.imports.txt`, `imports.tsv` | Import table |
| `explorer.asm` | Full Intel-syntax disassembly |

Confidence labels: **Observed** = read directly from the binary/PDB. **Inferred** = my interpretation of observed facts.

## 1. Where the file browser lives

- **Observed:** explorer.exe registers and references `CabinetWClass` (the folder window class) only in `CImpWorkerWndProc::s_WndProc` and `IsExplorerWindow` (via `g_atomCabinetClass`). It contains no folder view, navigation pane, breadcrumb or ribbon code. The bulk of its 10k functions are `CTaskBand`, `CTaskListWnd`, `CTray*`, `StartPanel*`, `SFTBarHost`, jump lists and logon tasks.
- **Inferred:** The browser window itself is implemented in `ExplorerFrame.dll` (frame, address bar, nav pane, ribbon) and `shell32.dll` / `windows.storage.dll` (`DefView` folder view, shell namespace, file operations). Recreating the browser UI faithfully requires those DLLs from the same build.

## 2. Process startup: shell vs. folder window

From `wWinMain` (0x140062008), in order:

1. Registers ETW providers, sets the AppUserModelID, `SetErrorMode`, `SetPriorityClass`, `EnableMouseInPointer`. *(Observed)*
2. `Cabinet_InitGlobalMetrics`, then opens a registry key. *(Observed)*
3. `ShouldStartDesktopAndTray()` (0x140062f00) returns an `EXPLORERSERVER` enum. It calls only `ExplorerIsShell()` (0x140062fb8) and `IsDesktopWindowAlreadyPresent()` (0x140062a08). *(Observed)*
   - **Inferred:** "become the shell" requires (a) explorer being configured as the shell and (b) no desktop window existing yet. Otherwise this process is just a folder-window host.
4. `CreateMutexW` + `WaitForSingleObject`, a second `IsDesktopWindowAlreadyPresent()` check, then `SetExplorerServerMode` (SHELL32 ordinal import). *(Observed)* **Inferred:** a mutex serializes racing launches so only one instance becomes the desktop.
5. `GetCommandLineW` → `PathGetArgsW`, then a check for empty args and `SHIsCurrentAppElevated`. *(Observed)* **Inferred:** a launch with arguments and/or elevation changes whether the request goes to an existing instance.
6. `ShellDDEInit` (DDE, for legacy `Folders` DDE commands), `SetProcessShutdownParameters`, `OleInitialize`, COM global options. *(Observed)*
7. DPI/theme setup: `CheckDefaultUIFonts`, `ChangeUIfontsToNewDPI`, `CheckHighContrast`, and **`IsPerMonitorAwareFileExplorerEnabled()`**, which gates `EnablePerMonitorDialogScaling` / `EnablePerMonitorMenuScaling`. *(Observed)*
8. `CoCreateInstance` of `CLSID_DesktopExplorerHost` / `CLSID_ExplorerHostCreator` (both referenced from `wWinMain`). *(Observed)* **Inferred:** this is the COM server that other launches use to ask the running explorer to open a new folder window.
9. Only on the shell path: `RunAllLogonTasks`, `CreateShellDirectories`, `CreateDesktopAndTray` (0x140062a70). *(Observed: guarded by the same flag register)*
10. `RegisterApplicationRestart`, then **`SHDesktopMessageLoop`** (SHELL32 ordinal), then `SHCloseDesktopHandle`, `WaitForRefs`, `WriteCleanShutdown`, OLE/COM teardown. *(Observed)*
    - **Inferred:** a single message loop in shell32 drives both the desktop and every folder window the process owns. Windows stay alive through a process/thread ref count (`CProcessAndThreadRefHost`, `CThreadRefHost::WaitForRefs`), so the process exits only when the last window closes.

### Related settings strings (Observed)

`SeparateProcess` (the "Launch folder windows in a separate process" option), `CLSID_SeparateMultipleProcessExplorerHost`, `StartupDelayInMSec`, `RestartExplorerMutex`, `WaitForExplorerRestart`.

## 3. Other window-related behavior (Observed)

- `IsShellFrameWindow` is used by `CTaskBand::_HandleShellHook`. The taskbar treats folder windows specially when grouping and switching.
- `LaunchNewInstanceWithOptionalElevate` exists (taskbar "open new window" path).
- `_InvokeVerbOnKnownFolder(HWND, GUID, verb)` opens known folders (Documents, Downloads, …) by GUID plus a verb rather than by path.
- The shell namespace imports (`SHParseDisplayName`, `SHCreateItemFromParsingName`, `SHGetKnownFolderIDList`, `SHBindToObject`, `SHGetPropertyStoreFromIDList`, `SHChangeNotifyRegisterThread`) show the PIDL/ShellItem model the browser builds on.

## 4. Mapping to macsplorer (design choices, not observed behavior)

| Windows behavior | Rust/macOS approach |
|---|---|
| One process hosts every folder window; COM `ExplorerHostCreator` forwards new launches | Single `NSApplication` process. Handle `application:openURLs:` / `application(_:open:)` and a CLI that forwards over a Unix socket or Apple Events to the running instance. |
| `SeparateProcess` setting | Optional; usually not worth it on macOS. |
| Ref-counted process lifetime | Keep running when windows close, like Finder, or quit on last window (`applicationShouldTerminateAfterLastWindowClosed`). Make it a preference. |
| Mutex-serialized startup | Not needed. LaunchServices enforces single instance per bundle ID. |
| Known folders by GUID + verb | A `KnownFolder` enum resolved via `NSFileManager.urls(for:in:)` (`objc2-foundation`). |
| PIDL / ShellItem namespace | An abstract `Location` type (filesystem path, virtual roots such as "This Mac", Network, Trash) with a provider trait. Don't model everything as plain paths. |
| `SHChangeNotifyRegisterThread` | FSEvents (the `fsevent`/`notify` crates) per open folder. |
| Per-monitor DPI opt-in | Free in AppKit (backing scale factor). |

## 5. Gaps

- **Open:** exact meaning of each `EXPLORERSERVER` enum value and the command-line switches (`/e`, `/select`, `/root`, `/n`). The switch strings aren't in explorer.exe; they're parsed in shell32 / ExplorerFrame.
- **Open:** everything inside the browser window (layout, view modes, nav pane, breadcrumb, sorting/grouping, column handling). Needs `ExplorerFrame.dll`, `shell32.dll` and `windows.storage.dll` from build 10240.
- **Uncertain:** two `CLSID_*` references were attributed to a telemetry function. That function has no PDB symbol of its own, so the containing function is probably mislabeled.
- Hopper decompilation crashed and was abandoned; no pseudocode was produced. All conclusions come from symbol names, call order and imports.
