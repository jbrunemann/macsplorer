# File browser findings (Windows 10 build 10240)

Targets (all in `analysis/`, PDBs from the Microsoft symbol server, names recovered into `symbols/*.pubs.tsv`):

| Binary | Version | Functions named |
|---|---|---|
| `ExplorerFrame.dll` | 10.0.10240.21072 | 11,129 |
| `shell32.dll` | 10.0.10240.21161 | 38,591 |
| `Windows.Storage.dll` | 10.0.10240.21161 | 50,733 |
| `comctl32.dll` (v6, WinSxS) | 6.10 (th1.251008-0227) | 12,862 symbols |

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

### 2.3 Copy as path

`CCopyAsPathMenu` (shell32; verb `copyaspath`, `CLSID_CopyAsPathMenu`) is a
context-menu handler. `GetState` hides it unless the selection has
`SFGAO_FILESYSTEM` and the view is not the Recycle Bin. `Invoke` builds an
`IShellItemArray` from the data object and asks each item for its display
name with `SIGDN_DESKTOPABSOLUTEPARSING` (a flag bit picks `SIGDN_URL`
instead). The text builder wraps every path in `"` (a flag turns that off),
joins items with a separator string (not decoded, assumed to be a line
break) and has a "first item only" flag. The result is put on the clipboard
with `SetClipboardData(CF_UNICODETEXT)`.
Not traced: why Explorer shows it only with Shift held (probably an extended
verb in the registration, not this code).

Macsplorer: "Copy as Path" in the Edit and item context menus, quoted path
per line (`core::copy_path`). Cmd+Option+C is Finder's shortcut for the same
(Finder: Option turns "Copy" into "Copy "name" as Pathname", plain text, not
verified here).

### 2.4 View modes (Large Icons, List, Details)

**Mode switch, observed.** `CDefView::SetViewModeAndIconSize` (shell32 0x18027be20) accepts a
`FOLDERVIEWMODE` of -1 or 1..8 and an icon size of -1 or 16..256 px (else `E_INVALIDARG`), then
calls `CDefView::_SwitchToViewModeAndIconSize` (0x1802890c4), which:

1. Logs which menu choice this was: List, Details, Tiles, Content by mode; otherwise by icon size
   16 = Small, 48 = Medium, 96 = Large, 256 = Extra large icons. So the Windows 10 "Large icons"
   are 96 px, not Win2000's 32 px.
2. Folds the icon modes together. `LogicalViewModeFromViewMode` (0x18014f2a0) maps List → list,
   Details → details, Tile → tiles, Content → content, and everything else (Icon, SmallIcon,
   Thumbnail, Thumbstrip) → icons. Inside the icons group, the icon size picks the layout:
   `FVM_ICON` at ≤ 32 px becomes `FVM_SMALLICON`, `FVM_SMALLICON` above 32 px becomes
   `FVM_THUMBNAIL`, `FVM_THUMBSTRIP` becomes `FVM_THUMBNAIL` (a folder flag, bit 17 of the view
   settings flags, suppresses the first two).
3. Stores mode + size in the view settings and fires change notifications (one for the mode, one
   for the size, only if each actually changed).
4. Starts a 1 s timer on the view window. **Inferred:** a deferred save of the per-folder view
   state (`CViewSettings`).

Nothing in this path touches the selection, focus item or sort. **Inferred:** they carry over a
mode switch, because the item view is rebuilt over the same model (next paragraph).

**Layout per mode, observed.** In ExplorerFrame, `UIItemsView::_CalculateViewMode` (0x18001e6fc)
turns the logical mode into a layout: details → 4, list → 3, tiles → 6, content → 8, icons → 1,
and icons at ≤ 32 px → 2. On a change, the view destroys its children and instantiates the DirectUI
template for that layout (`FUN_18002099c`, table of `rootitem.details`, `.list`, `.smallicons`,
`.icons`, `.tiles`, `.smart`). Details uses `ItemRowLayout`; the others use `ItemGridLayout`.

Template values from the ExplorerFrame markup resource (a string pool, so which attribute owns a
value is read from order, **inferred**; `rp` = pixels at 96 DPI):

| Template | Item | Layout |
|---|---|---|
| `icons` | `baselinelayout()`: image `bottomcenter`, label `topcenter\|wrapleft\|endellipsis\|editcontrol` (label under the icon, centered, wrapped) | alignment `baseline\|justify\|jaggedbottom`, `minimumicontextwidth` 74rp |
| `list` | icon left of label, label `MaxPixelWidth` 280rp, min size 16rp | `virtualdirection horizontal`, alignment `jaggedbottom`, item spacing 10rp × 1rp, item width 60rp..800rp |
| `smallicons` | icon left of label | column width 230rp, item spacing 1rp × 1rp |
| `details` | rows | `columnpadding` 10rp |

**Grid column width, observed.** `ItemGridLayout::_DetermineColumnWidth` (EF 0x180068014) runs
when alignment bit 0 is set and the host is a `UIItemsView`:

```
w = (icon + 9 * (icon - 16) / 240 + 6) * dpi / 96
column_width = w > minimum_icon_text_width ? w : minimum_icon_text_width
```

For icons up to ~100 px the 74 px minimum text width wins, so the column is 74 px wide.

**Flow direction, observed.** `ItemLayout::VirtualDirection` is `horizontal` = 1, `vertical` = 2.
`ItemGridLayout::GetVerticalSpacing` (EF 0x180006a10) returns the item spacing's width instead of
its height when the direction is horizontal, i.e. the grid is transposed: List fills a column top
to bottom, then the next column, and scrolls sideways.

**Arrow keys, observed.** `ItemGridLayout::ItemGetAdjacent` (EF 0x18014a700), with `n` items,
`per` items per line (`ColumnCount`), `lines` lines (`SectionCount`) and focus index `i`. In a
horizontal-direction grid (List) Up/Left and Down/Right are swapped first (`FUN_18014b638`), so
the rules below are written for a row-major grid:

| Key | Target |
|---|---|
| Left / previous | `i - 1` |
| Right / next | `i + 1` |
| Up | `i - per` |
| Down | `i + per`, clamped to `n - 1`, only if `i / per + 1 < lines` (so Down from the row above a short last row lands on the last item) |
| Home / End | `0` / `n - 1` |

Out-of-range targets do nothing (`ItemGridLayout::_RealizeItemByIndex` ignores `< 0` and `≥ n`).
Left/Right do not stop at the row edge: Left on the first item of a row moves to the last item of
the row above. In List that means Up/Down walk the items in order across column breaks.

Windows 2000 drew the view with SysListView32 instead; its geometry is in §2.5.

### 2.5 SysListView32 geometry (comctl32 v6.10, build 10240)

Windows 2000's Explorer view was the comctl32 ListView. The v6 control in build 10240 is a C++
rewrite (`CListView` with `CLVIconView`, `CLVListView`, `CLVView`, `CLVImageListManager`,
`CLVKeyboardManager`, `CLVSlotsManager`), but it keeps the classic metrics. Addresses below are in
`comctl32.dll` (base 0x171a00000). System metrics at 96 DPI: `SM_CXBORDER`/`SM_CYBORDER` 1,
`SM_CXEDGE`/`SM_CYEDGE` 2.

**Icon spacing, observed.** `CLVImageListManager::OnSetIconSpacing(-1)` (0x171a4bfe0), the default:

```
cx_spacing = image_cx - SM_CXICON + SM_CXICONSPACING
cy_spacing = image_cy - SM_CYICON + SM_CYICONSPACING
```

With 32 px images that is just `SM_CXICONSPACING` × `SM_CYICONSPACING`. Those come from the user's
"Icon Spacing" setting, which defaults to 75 × 75 px (registry `IconSpacing` /
`IconVerticalSpacing` = -1125 twips; the default is **not** in this binary, it's user32's).

**Large Icons cell, observed** (`CLVIconView::GetRects` 0x171bc16f4,
`CLVImageListManager::GetIconBufferX/Y` 0x171a4cb28 / 0x171a4caf8, `FUN_171aa862c`):

- Icon rect: the full spacing width × (image height + 2 · 2 px). The image is centered
  horizontally (`buffer_x = (cx_spacing - image_cx) / 2`) and starts 2 px (`2 · SM_CYBORDER`) below
  the top.
- Label: top = image bottom + `SM_CYBORDER + SM_CYEDGE` (3 px); width = measured text width +
  2 · `SM_CXEDGE`, centered on the icon rect.

**Label wrapping, observed** (`CLVView::_ComputeLabelSizeWorker` 0x171a4ae3c,
`CLVView::v_RecomputeLabelSize` 0x171a4a130, `CLVIconView::_RefoldLabelRect` 0x171aa9284):

- Text is measured with `DrawText(DT_CALCRECT | DT_CENTER | DT_NOPREFIX | DT_EDITCONTROL)` in a
  box `cx_spacing - 2 · SM_CXEDGE` wide (71 px at 75 spacing), word-wrapped (`DT_WORDBREAK`)
  unless `LVS_NOLABELWRAP`. A third measurement uses `DT_WORD_ELLIPSIS`.
- Stored per item: single-line width, wrapped width, ellipsized height and wrapped height, each
  plus 2 · `SM_CXEDGE` / `SM_CYEDGE`.
- **Folded** label height = min(wrapped height, ellipsized height, `SM_CYEDGE` + 2 lines). So a
  label shows at most two lines, the second ending in "…". `GetRects` skips folding for items
  `CLVIconView::IsItemUnfolded` reports. **Inferred** from the name and Win2000 screenshots: that is
  the focused item while the view has focus, which shows its full label.

**Auto-arrange slots, observed** (`CLVSlotsManager::GetSlotCountEx` 0x171a4b95c,
`CLVSlotsManager::CalcSlotRect` 0x171a4b838):

- Slots per row = max(1, available width / slot width) with `LVS_ALIGNTOP` (Explorer's), per
  column with `LVS_ALIGNLEFT`. Slot = `cx_spacing` × `cy_spacing`.
- The width is first computed without a vertical scroll bar. If `rows · columns` slots can't hold
  every item, it is computed again with the scroll bar's width (`SM_CXVSCROLL`) taken off.
- Slot `i` is at `((i % per) · cx_spacing, (i / per) · cy_spacing)` minus the scroll origin. No
  extra space is spread between columns unless `LVS_EX_JUSTIFYCOLUMNS` (Vista+), so the grid is
  left-aligned with the leftover width on the right.

**List mode, observed** (`CLVListView::GetRects` 0x171bb1198, `CalcViewItem` 0x171bb1104,
`GetNewColWidth` 0x171a323e4, `MaybeResizeListColumns` 0x171a324b0, `ResizeListColumns`
0x171bb1344, `CLVView::ComputeCYItemSize` 0x171a4ad7c):

- Row height = max(small icon height, font line height) + `SM_CYBORDER`.
- Items per column = rows that fit the client height (at least 1). Item `i` is in column
  `i / per`, row `i % per`, so the list fills downwards and then to the right.
- **Column width** = the widest single-line label (text + 2 · `SM_CXEDGE`) + small icon width +
  8 · `SM_CXBORDER`, capped at the screen width. All columns share it. `ResizeListColumns`
  recomputes it over all items; `MaybeResizeListColumns` only widens it when inserted items are
  wider. Both do nothing once the app has set a width (`LVM_SETCOLUMNWIDTH`).
- Within a cell: icon rect = small icon width × row height at the cell's left, label rect = the
  rest of the column.

**Arrow keys, observed** (`CLVKeyboardManager::OnArrow` 0x171a3303c, `FindNearestItem`
0x171bbecd4, `_NearestItemCommon` 0x171bc0100). Key handling depends on the mode:

- **List** (and Details), by index with `per` = items per column:

  | Key | Target |
  |---|---|
  | Up / Down | `i - 1` / `i + 1` |
  | Left / Right | `i - per` / `i + per` |
  | Home / End | `0` / `n - 1` |
  | PgUp / PgDn | ± `per` · max(1, visible columns), where a column counts once ⅓ of it is visible. PgUp stays in the same row if it would go past the start (adds `per` until ≥ 0). PgDn steps back by `per` until it's < `n`. |

  Out-of-range targets do nothing.
- **Large Icons** (and Small Icons), spatial. From the focused item's top-left corner, look at
  every other item's top-left corner `(dx, dy)`:
  - Left: `dx < 0` and `|dx| ≥ |dy|`; Right: `dx > 0` and `|dx| ≥ |dy|`;
    Up: `dy < 0` and `|dy| ≥ |dx|`; Down: `dy > 0` and `|dy| ≥ |dx|`.
  - Of those, pick the smallest `dx² + dy²`. If there are none, focus stays.
  - Home / End = the first / last item.
  - PgUp / PgDn move the reference point a page (client height minus half an item, at least
    1.5 items) up or down and take the nearest item, clamping to Home / End.

  On an auto-arranged grid this means Right at the end of a row does **not** wrap (Windows 10's
  DirectUI grid does, §2.4). With square 75 × 75 slots, Down from above a short last row reaches
  the item one column to the left (`|dx| = |dy|` counts), but nothing further away.
  (Owner-data list views use slot arithmetic with the same no-wrap rule instead; Explorer's wasn't
  owner-data.)

**Hit testing in Large Icons, observed** (`CLVIconView::v_ItemHitTest` 0x171a48f90,
`CLVIconView::_HitTestItemRect` 0x171a30854). With `p` = the image's top-left corner:

- Icon hit rect: from `p.x - 8 · SM_CXBORDER` to `p.x + image_cx + 8 · SM_CXBORDER` (the image
  plus 8 px each side), from `p.y - 2` down to the label's top.
- Label hit rect: the label rect, folded unless the item is unfolded, but starting at the icon
  rect's bottom.
- Anything else in the slot is empty space (a click there deselects / starts a marquee). Items are
  tested in z-order and the first hit wins.

**Marquee selection, observed** (`CListView::HandleMouse` 0x171a31328,
`CLVSelectionManager::DragSelect` 0x171bba69c, `_DragSelectItem` 0x171bbbb18,
`CLVScrollingManager::ShouldScroll` 0x171bb7cf4):

- **Start.** A left press where the hit test finds no item, in a multi-select view
  (not `LVS_SINGLESEL`), that moves past the drag threshold (`CheckForDragBeginEx`) sends
  `LVN_MARQUEEBEGIN`; unless the parent refuses, the selection is cleared when neither Ctrl nor
  Shift is down, then the drag starts. A press that doesn't become a drag also clears the selection
  without Ctrl/Shift. In Details without `LVS_EX_FULLROWSELECT`, a press on a subitem column counts
  as no item. **Inferred:** column 0 to the right of the label is empty space too (the details hit
  test itself wasn't decoded).
- **Band.** From the press point to the pointer, normalized. The pointer end is clamped to the
  client rect; the start is kept in content coordinates, so it scrolls with the items. Classic
  (not double-buffered) list views draw it with `DrawFocusRect`, the dotted XOR outline; the
  translucent band is only for `LVS_EX_DOUBLEBUFFER`.
- **Which items change.** On every pointer move that changes the band, each item that may lie in
  the union of the old and new band is looked at. Its test rect is `LVIR_SELECTBOUNDS` (icon plus
  label) shrunk by a quarter of its width and a quarter of its height on every side (in List view
  the horizontal inset is capped; in Details with full-row select there's no inset). Only items
  whose test rect intersected exactly one of the old and new band change: without Ctrl they become
  "selected iff inside the new band", with Ctrl their selection flips. Items the band never crossed
  keep their state, so Shift+drag adds to the selection and an item left behind is deselected again.
  Modifier keys are read from each move message, not from the press.
- **Anchor.** On a move without Ctrl or Shift, the Shift+click anchor becomes the item inside the
  new band whose test rect's top-left corner is farthest from the pointer. Focus isn't moved.
- **End.** Any mouse button message other than a move ends the drag and keeps the selection;
  Escape deselects everything and ends it.
- **Auto-scroll.** While the pointer is outside the window (or at the screen edge), the view scrolls
  a step toward it and the band is recomputed, from a timer as well as from moves. The step size
  and rate (`OnScrollSelectSmooth`) weren't decoded.

**Label unfolding, observed** (`CLVIconView::IsItemUnfolded` 0x171a30ab4, `IsItemUnfoldedPtr`
0x171bc1bfc): only in icon view, only while a control state flag (`0x80` in the flags at
`+0xa8`) is set, and only for the item equal to the control's focus index. **Inferred:** the flag is
"control has keyboard focus", so the focused item shows its whole label while the view has focus
and folds back to two lines when focus leaves.

**Selected icon drawing, observed** (`CLVView::v_DrawItem` 0x171a4a6a0,
`CLVImageListManager::DrawImageEx2` 0x171a4c1f0), for the classic style (no
`LVS_EX_BORDERSELECT`, no theme):

| Item state | Image | Label |
|---|---|---|
| selected, view focused (or drop target) | `ILD_BLEND50` with `CLR_DEFAULT`: 50% blend toward `COLOR_HIGHLIGHT` | highlight background |
| selected, view not focused (`LVS_SHOWSELALWAYS`) | `ILD_BLEND25`: 25% blend toward `COLOR_HIGHLIGHT` | `COLOR_BTNFACE` background |
| cut (`LVIS_CUT`) | `ILD_BLEND50` toward the list background color (ghosted) | normal |

The label colors are inferred from the border-select branch, which picks `COLOR_HIGHLIGHT` vs
`COLOR_BTNFACE` from the same return flags; the label painter itself (`FUN_171a48290`) wasn't
decoded. **Inferred:** comctl32 v5 (Windows 2000) used the same blends.

## 3. Mapping to macsplorer (design choices)

| Windows | Rust/macOS |
|---|---|
| Rename selection rule (§2.1) | Port it directly. `NSTextView.setSelectedRange` after `beginEditing`. Use the real extension from `NSURL.pathExtension`, and only when extensions are visible. |
| "Hide extensions" | Not traced; macsplorer's own rule (`Entry::hidden_extension`): the extension stays visible when the rest of the name has one too (`Invoice.pdf.app`, `photo.jpg.command`), so apps and scripts can't pass for documents. Hidden extensions are put back on rename, for packages as well as files. |
| `IItemNameLimits` | On APFS/HFS+ only `/` is invalid (`:` is shown as `/` in Finder). Block `/` and NUL, cap at 255 UTF-8 bytes (`NAME_MAX`) instead of 128 chars. |
| `StrCmpLogicalW` | `localizedStandardCompare:` (Finder's own natural sort), or a Rust natural-sort crate (`natord`, `lexical-sort`) if you need determinism across locales. Keep a "plain sort" setting that matches `NoStrCmpLogical`. |
| `CViewSettings` per-folder/per-type view memory | Store view state keyed by folder path plus a "folder type" fallback (Documents/Pictures/Music/Videos/General) in your own SQLite/JSON file. Don't write `.DS_Store` files. |
| Status bar free space | **Unverified, from Windows 2000 behavior (not traced in build 10240, whose status bar only shows the item count).** With nothing selected the first pane reads `N object(s) (Disk free space: 4.65 GB)`; with a selection it reads `N object(s) selected` and drops the free space. The value is the user-available bytes on the folder's volume (`GetDiskFreeSpaceEx`, `lpFreeBytesAvailable`), formatted with `StrFormatByteSize`. macsplorer uses `statvfs` (`f_bavail * f_frsize`) on the listing worker thread. |
| View modes (§2.4, §2.5) | Win2000 look, so follow the ListView (§2.5): 75 × 75 slots, 32 px icon centered 2 px from the top, label 3 px below, wrapped in 71 px and folded to two lines unless focused; clicks hit the image ± 8 px or the label, not the whole slot; selected icons blended 50% (25% when the view is unfocused) toward the highlight color; spatial arrow keys. List mode is traced above but deliberately not offered. Marquee as traced in both views; Details uses the icon plus the label's highlight box as select bounds, the band is drawn as black dots since egui can't XOR, and auto-scroll runs while the pointer is at or past the view's top or bottom edge at a guessed 20 lines a second. From build 10240 (§2.4): selection, focus and sort survive a mode switch. Labels use Tahoma 12 px instead of 8 pt, so rows and two-line labels come out a little taller than Win2000. View mode is per window until per-folder memory exists. |
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
- "Open With" submenu contents and order (`COpenWithMenu::QueryContextMenu`, `COpenWithMenu::InitMenuPopup` in shell32). macsplorer currently guesses: default app first, the rest alphabetical, then "Choose Program...".

Unknown: what the folder flag tested in `CDefView::OnBeginLabelEdit` means (`FOLDERFLAGS & 0x80000000`, passed as `SHBeginLabelEdit`'s 5th parameter). When the flag is clear, it removes `WS_EX_RTLREADING` from the edit control.
