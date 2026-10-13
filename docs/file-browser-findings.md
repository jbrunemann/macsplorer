# File browser findings (Windows 10 build 10240)

Targets (all in `analysis/`, PDBs from the Microsoft symbol server, names recovered into `symbols/*.pubs.tsv`):

| Binary | Version | Functions named |
|---|---|---|
| `ExplorerFrame.dll` | 10.0.10240.21072 | 11,129 |
| `shell32.dll` | 10.0.10240.21161 | 38,591 |
| `Windows.Storage.dll` | 10.0.10240.21161 | 50,733 |

Method: Ghidra 12.1.4 pseudocode through REA (by address; Ghidra headless did not load the PDBs) plus `llvm-objdump` with PDB names. Static only, nothing executed.
**Observed** = read from code. **Inferred** = my interpretation. Section 3 is design advice for macsplorer, not Windows behavior.

## 1. Component map

Which class owns which part of the window. Method counts come from PDB public symbols.

| UI part | Class (DLL) | Notable methods |
|---|---|---|
| Window/frame, toolbar layout, saved size/position | `CExplorerFrame` (EF) | `ComputeInitialWindowLocation`, `ComputeMinimumFrameSize`, `_LoadFrameState`/`SaveFrameState`, `RestartInSeparateProcess` |
| Browser core: navigation, view hosting | `CShellBrowser` (EF, 385) | |
| Back/forward buttons + history | `CTravelBand`, `CTravelLog`, `CTravelLogUI` (EF) | `_ShowBackForwardMenu`, `AddEntry`, `FastBack`, `GetToolTipText`, `InsertMenuEntries`, `_FindEntryByPidl` |
| Up button | `CUpBand` (EF) | |
| Address bar container | `CAddressBand` (EF) | `SetEditMode`, `RevertEditChanges`, `_EnsureBreadcrumbBar`, `_EnsureAddressEditBox` |
| Breadcrumb | `CBreadcrumbBar` (EF) | `_AddButtonForIDList`, `_AddRootDropdownItems`, `_OnDropDown`, `_GoToEditMode`, `_CopyToClipboard`, `_OnDragOut`, `OnDropDDT`, `_OnFSNotify`, `_LastCrumbHasExpando`, `_AddFilterButtons`/`_IsFilterPidl` |
| Address autocomplete | `CAddressEditBox`, `CSuggestPopup` (EF), `CAutoComplete` (shell32) | |
| Search box | `CSearchBox`, `CSearchEditBox`, `CUniversalSearchBand` (EF) | `SetSearchText`, `AppendOrReplaceFilter`, `HasSuggestions`, `SetMRULocation` |
| Navigation pane (tree) | `CNscTree` (EF, 294) | `_ExpandToItem`, `_StartBackgroundEnum`, `_OnSHNotifyCreate/Delete/Rename/UpdateDir`, `_PersistChildOrder`, `_SortByName`, `_ShouldInsertChildCheckPin`, `CreateNewFolder`, `MoveItemUpOrDown` |
| Ribbon | `CExplorerRibbon` (EF) | |
| Item view (DirectUI) | `UIItemsView`, `CItemsView`, `ItemGridLayout`, `ItemRowLayout`, `UIViewHeader`, `UIColumnHeader`, `CSelectionManager` (EF) | |
| In-place rename UI | `CInplaceRename`, `UIRenameTextElement` (EF) | see §2.1 |
| View controller | `CDefView` (shell32, 604) | view modes, sort/group/filter, selection, context menus, "try harder" search suggestions |
| List host | `CListViewHost` (shell32, 272) | `EditItemLabel`, `BeginSubItemEdit`, `GhostSelectedItems` (cut state) |
| Per-folder view memory | `CViewSettings` (Windows.Storage, 145) | `SaveViewSettings`, `SetAsFolderTypeDefault`, `ResetToFolderTypeDefault`, `_GetPropertyBagFromFolder/Template`, `_LoadSortColumns`, `_LoadGroupBy`, column widths |
| File system namespace | `CFSFolder` (shell32 413 / storage 401) | `CompareIDs` (sorting) |
| Copy/move/delete engine | `CFileOperation`, `CCopyWorkItem`, `CFSTransfer` (storage) | |
| Groups | `CDefGroup`, `CDetailCategorizer` (storage) | |
| Libraries, Quick Access, This PC | `CLibraryFolder`, `CHomeFolder`, `CDrivesFolder` (storage) | |

**Inferred:** the window is a frame (`CExplorerFrame`) hosting a browser (`CShellBrowser`), which hosts bands (travel, address/breadcrumb, search) and one view (`CDefView`) per location. The view gets its items from a folder object (`IShellFolder`: `CFSFolder`, `CDrivesFolder`, …), and view settings persist per folder or folder type through `CViewSettings`.

## 2. Verified behaviors

### 2.1 Rename (F2 / slow click)

Code path: `CInplaceRename::_PlaceAndShowControl` (EF 0x180167920) → `IListControlHost` → `CDefView::OnBeginLabelEdit` (shell32 0x180279070) → `SHBeginLabelEdit` (shell32 0x1805042c8) → back in EF `CInplaceRename::_SetInitialSelection` (0x18016822c). **Observed** (Ghidra pseudocode):

1. The item must have `SFGAO_CANRENAME`. Otherwise the rename fails with `MessageBeep(0)`.
2. The edit box gets the **editing name** (`SIGDN_PARENTRELATIVEEDITING`).
3. **Initial selection:**
   - Skipped for real folders (`SFGAO_FOLDER` without `SFGAO_STREAM`). Zip files are FOLDER+STREAM, so they're treated as files.
   - Skipped when the editing name differs (case-insensitive) from the parsing name, i.e. when **"hide extensions" is on**.
   - Otherwise it reads the item's extension property. If the name **ends with** that extension (case-insensitive) and the extension is **shorter than** the whole name, it selects `[0, len - ext_len)`. If not, it selects all of it.
   - `_SetInitialSelection`: if nothing is selected after that (`EM_GETSEL` returns 0,0), select all (`EM_SETSEL 0,-1`).
4. Invalid filename characters are blocked while typing (`SHLimitInputEdit`, driven by the folder's `IItemNameLimits`).
5. Max length = the folder's limit for this name, or **128** characters when the folder reports 0 (`EM_LIMITTEXT`).
6. The edit control's font size matches the item label. It's placed over the label and the label is hidden while editing. A mouse hook ends editing on outside clicks (`_HookMouse`).

What this gives you:

| Item | Selected on rename |
|---|---|
| `report.final.docx` | `report.final` |
| `archive.tar.gz` | `archive.tar` |
| `.gitignore` | everything (extension = whole name) |
| `Makefile` | everything |
| folder `My.Folder` | everything |
| `photo.JPG` with extensions hidden | everything (box shows `photo`) |

### 2.2 Name sorting

**Observed:** `StrCmpLogicalRestricted` (Windows.Storage 0x18015d9c8):

```
if SHRestricted(REST_NOSTRCMPLOGICAL /*0x4000007E*/) -> StrCmpIW   (plain, case-insensitive)
else                                                 -> StrCmpLogicalW (natural: "file2" < "file10")
```

Called from `CFSFolder::CompareIDs`, `CDrivesFolder::CompareIDs`, `CompareIDsAlphabetical`, `ILCompareRelIDs`, `CompareVariantsDirection` (column sorting) and `CDetailCategorizer::OnCompareCategory` (group headers). **Inferred:** every name-based ordering in Explorer is natural sort by default.

## 3. Mapping to macsplorer (design choices)

| Windows | Rust/macOS |
|---|---|
| Rename selection rule (§2.1) | Port it directly. `NSTextView.setSelectedRange` after `beginEditing`. Use the real extension from `NSURL.pathExtension`, and only when extensions are visible. |
| `IItemNameLimits` | On APFS/HFS+ only `/` is invalid (`:` is shown as `/` in Finder). Block `/` and NUL, cap at 255 UTF-8 bytes (`NAME_MAX`) instead of 128 chars. |
| `StrCmpLogicalW` | `localizedStandardCompare:` (Finder's own natural sort), or a Rust natural-sort crate (`natord`, `lexical-sort`) if you need determinism across locales. Keep a "plain sort" setting that matches `NoStrCmpLogical`. |
| `CViewSettings` per-folder/per-type view memory | Store view state keyed by folder path plus a "folder type" fallback (Documents/Pictures/Music/Videos/General) in your own SQLite/JSON file. Don't write `.DS_Store` files. |
| `CTravelLog` | `Vec<Location>` with a cursor. The back/forward dropdown menu lists entries (`_ShowBackForwardMenu`). |
| Breadcrumb (`CBreadcrumbBar`) | Each crumb is a button with a chevron dropdown listing subfolders (`_OnDropDown`). Click empty space → edit mode with the full path (`_GoToEditMode`). Crumbs are drag sources and drop targets. Collapse overflow into a root dropdown (`_AddRootDropdownItems`). |
| `CNscTree` change notifications | FSEvents per expanded node; enumerate in the background (`_StartBackgroundEnum`). |

## 4. Next candidates

Each one can be answered from this setup:

- Sort tiebreakers in `CFSFolder::CompareIDs` (folders before files? secondary column?), plus `_CreateSortColumnArrayWithSecondaryColumn`.
- Breadcrumb overflow rules and dropdown contents.
- Travel log limits and back-menu length.
- Per-folder view defaults per folder type (`CViewSettings::_GetPropertyBagFromTemplate`).
- Copy/move conflict dialog rules (`CFileOperation`, `CCopyWorkItem`).
- Navigation pane default roots and expand-to-current-folder behavior.
- Search box: query syntax and filters.

Unknown: what the folder flag tested in `CDefView::OnBeginLabelEdit` means (`FOLDERFLAGS & 0x80000000`, passed as `SHBeginLabelEdit`'s 5th parameter). When the flag is clear, it removes `WS_EX_RTLREADING` from the edit control.
