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

### 2.1.1 Slow-click trigger (comctl32 v6.10)

The trigger is in the ListView, not Explorer. Code path: `CLVMouseManager::HandleMouse` (comctl32 0x171a31328) → `CLVInPlaceEditingManager::SetupPendingNameEdit` (0x171bafd54), then `CListView::OnTimer` (0x171a4fdf4) → `CLVInPlaceEditingManager::EditItem` → `LVN_BEGINLABELEDIT` → §2.1. **Observed** (Ghidra pseudocode):

1. A single click arms a pending rename only when all of these hold:
   - not a double-click, and no `MK_RBUTTON`, `MK_SHIFT` or `MK_CONTROL` in the key flags;
   - the hit test says `LVHT_ONITEMLABEL` (the label, not the icon);
   - the item was selected before the click, and a selection counter equals 1 (**inferred:** it was the only selected item);
   - the list had keyboard focus when the button went down (`GetFocus()` is the ListView, or an internal flag is set);
   - neither `LVS_EX_ONECLICKACTIVATE` nor `LVS_EX_TWOCLICKACTIVATE` is set;
   - the drag-detect loop inside `HandleMouse` ended without a drag (a drag takes another path).
2. Arming saves the item, sets a "pending edit" flag and calls `SetTimer(id 0x2A, GetDoubleClickTime())`, 500 ms by default. The drag-detect loop returns on button-up, so the wait effectively starts at release.
3. `CancelPendingEdit` clears the flag and kills the timer. It's called for a double-click (`HandleMouse` with the double-click flag), from `CLVKeyboardManager::OnKey` (any key), and when a label edit finishes.
4. When timer 0x2A fires, `OnTimer` does nothing if an edit is already open or the flag was cleared. Otherwise it scrolls the item into view and starts the label edit.

Unverified: whether Windows 2000's comctl32 had exactly these conditions; the binary read here is v6.10 from build 10240.

**macsplorer** (`slow_click`): the same conditions, minus the activate styles (there is no single-click mode). Keyboard focus means the item view was the active pane. A click on any other item, a marquee, a click on empty space, moving focus to the tree, or navigating also drops the pending rename. The delay is `slow_click::DELAY_MS` (500 ms), and egui's double-click interval is set to the same value, as on Windows.

### 2.2 Name sorting

**Observed:** `StrCmpLogicalRestricted` (Windows.Storage 0x18015d9c8):

```
if SHRestricted(REST_NOSTRCMPLOGICAL /*0x4000007E*/) -> StrCmpIW   (plain, case-insensitive)
else                                                 -> StrCmpLogicalW (natural: "file2" < "file10")
```

Called from `CFSFolder::CompareIDs`, `CDrivesFolder::CompareIDs`, `CompareIDsAlphabetical`, `ILCompareRelIDs`, `CompareVariantsDirection` (column sorting) and `CDetailCategorizer::OnCompareCategory` (group headers). **Inferred:** every name-based ordering in Explorer is natural sort by default.

**Not traced, from the API documentation:** `StrCmpLogicalW` and `StrCmpIW` compare letters with `CompareStringW` in the user locale, case-insensitively, so accented letters sort by the locale's rules ("Ä" with the A's in German, after Z in Swedish).

**macsplorer** (`natural`): ICU4X collation (CLDR data) at secondary strength, which ignores case but not accents; the natural sort turns on CLDR's numeric ordering (`kn`), which reads digit runs as numbers. Names equal under that compare ordinally, to keep the order total. The language is `NSLocale.currentLocale.collatorIdentifier` on macOS (the first preferred language, or the List sort order in System Settings), `LC_COLLATE` on Linux. Digit runs compare as whole strings with the collation (Finder's `localizedStandardCompare:` works the same way), not chunk by chunk, so `a_1` sorts before `a1` because `_` sorts before digits; whether `StrCmpLogicalW` agrees there wasn't checked. Type-ahead and the search box match names in NFC, lowercase (`natural::fold`), so a name stored decomposed (HFS+, SMB, zip) is found by typing the composed letter.

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
  the horizontal inset is capped at the small icon width, `CLVSelectionManager::_DragSelectItem`
  0x171bbbb18; in Details with full-row select there's no inset). Only items
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
| hidden (`SFGAO_GHOSTED`), shown with "Hidden files and folders" | **Inferred:** drawn as cut (the shell hands ghosted items to the list view as `LVIS_CUT`; the folder tree does the same with `TVIS_CUT`), label normal. macsplorer calls dot-files hidden. | normal |

The label colors are inferred from the border-select branch, which picks `COLOR_HIGHLIGHT` vs
`COLOR_BTNFACE` from the same return flags; the label painter itself (`FUN_171a48290`) wasn't
decoded. **Inferred:** comctl32 v5 (Windows 2000) used the same blends.

### 2.6 Column header resizing (SysHeader32, comctl32 v6.10)

The Details view's header is a `SysHeader32` created by `CLVHeaderControlManager::CreateHeader`
(0x171a4dca0) with `HDS_FULLDRAG` always set. Header code from build 10240; **inferred:** the
classic (unthemed) paths are unchanged from Windows 2000's comctl32 v5.

**Hit test, observed** (`Header_HitTest` 0x171ad6154; slop set in `Header_OnCreate` 0x171a34fb8):

- The divider slop is 8 · `SM_CXBORDER` = 8 px, capped at a quarter of the column being tested.
- Walking the columns left to right, a point inside column *i* is on divider *i − 1* if it is less
  than the slop past the column's left edge (*i* > 0), on divider *i* if it is within the slop of
  the right edge, otherwise on the column. The empty header past the last column is tested as one
  more column, so the last divider can be grabbed from the right too; the rest of it is
  `HHT_NOWHERE`.
- Zero-width columns are skipped. Right after one, the left-edge zone is `HHT_ONDIVOPEN` for the
  hidden column (strictly right of the edge) and plain `HHT_ONDIVIDER` exactly on it, so a hidden
  column can always be dragged back open.
- Columns with `HDF_FIXEDWIDTH`, or a header with `HDS_NOSIZING`, have no divider zones.
- Cursors (`Header_OnSetCursor` 0x171ad770c): comctl32 resource 106 (split) on a divider, 107 (split
  open, double bar) on `HHT_ONDIVOPEN`, the arrow elsewhere.

**Drag, observed** (`Header_OnLButtonDown` 0x171ad6ff4, `Header_OnMouseMove` 0x171ad7480,
`Header_OnLButtonUp` 0x171ad7200, `Header_WndProc` inner 0x171a35b00):

- Tracking starts on the press itself, no drag threshold; the header captures the mouse and sends
  `HDN_BEGINTRACK` (the parent can veto). A press on a divider never sorts.
- The divider keeps its offset from the press point. The new right edge is clamped to the column's
  left edge, so the minimum width is 0; there's no maximum. Columns to the right shift; nothing is
  squeezed to fit, the list just scrolls sideways.
- With full drag (`HDS_FULLDRAG` and "Show window contents while dragging") each move sends
  `HDN_TRACK` and resizes the column live. Without it the header and the ListView
  (`CLVHeaderControlManager::_RHeaderTrack` 0x171bb9450) draw an inverted `SM_CXBORDER` line down
  the view, and the width is set only on release (`HDN_ENDTRACK`).
- Escape held, a right-button press or losing capture cancels: `HDN_ENDTRACK` with the original
  width, which is put back.

**Double-click, observed:** a double-click on `HHT_ONDIVIDER` (not `HHT_ONDIVOPEN`) sends
`HDN_DIVIDERDBLCLICK`; `CLVHeaderControlManager::HeaderNotify` (0x171a4d898) answers with
`SetColumnWidth(i, LVSCW_AUTOSIZE)` (0x171a13154). `_GetAutoWidth` (0x171bb8ee8) takes the widest
item in the column (`_ComputeColumnWidthForItem`, not decoded), then `_ValidateMinWidth`
(0x171a4de98) raises it to at least the header item's own width (caption plus 3 · `SM_CXEDGE` each
side, `Header_ComputeItemWidth`) plus 25 px at 96 dpi. **Unverified:** whether comctl32 v5 had that
minimum; it looks like a Vista addition.

**Empty header, observed** (`Header_Draw` 0x171a18194): without a theme and without `HDS_FLAT`,
the space past the last column is drawn as one more raised, empty button out to the edge.

**Persistence, unverified, from memory:** Windows 2000 kept column widths per folder with the rest
of the view state (`Explorer\Streams`, a limited MRU of folders). macsplorer deliberately keeps one
set of widths for all folders until the window closes.

### 2.7 Undo (names only, not decompiled)

From the public symbols, not traced further. Each finished file operation
adds undo records to the thread's undo manager
(`CFileOperation::_AddUndoRootsToUndoManager` in Windows.Storage;
`SHGetThreadUndoManager`, `CUndoManager` in shell32). The manager keeps a
stack of units (`CCommonParentUndoUnit::_PushStack` / `_PopStack` /
`_PeekStack`), so undo reverses the newest operation first, and its size is
bounded by memory use (`_GetMemoryUsage`, `_GetStackSize`) rather than, as
far as the names show, a fixed count. `CUndoNode` records per item source,
destination and name (`_GetOpSrcFolder`, `_SetItemAndName`,
`_CheckSameName`). The Edit menu text comes from `GetFileUndoText`
("Undo Rename", "Undo Move", ...). The undo itself runs on a worker thread
(`_UndoThread`).

Macsplorer (`core::undo`): rename, move, copy and delete are recorded and
undone newest first from the toolbar's Undo button, Edit > Undo <operation>
and Cmd+Z, on a worker thread. Guesses: the stack keeps the last 10
operations; undoing a copy moves the copies to the Trash; undoing a delete
moves the items back from the Trash (macOS: where `trashItemAtURL` said it put
them; Linux: the newest freedesktop Trash entry with that original path); an
undo never replaces an item that has since taken the name, and stops at the
first item that fails. No redo (Windows 2000 had none either).

### 2.8 Properties, General page (shell32 `CFilePropSheetPage`)

Decompiled with Ghidra. The dialog strings and layout live in
`shell32.dll.mui`, which isn't in this setup, so the wording and layout
come from a Windows 2000 screenshot of a folder's Properties.

Observed:

- `_InitSingleFilePrsht` fills the page: the item's icon, the name in an
  edit box, the type name (`SHGetFileInfoW`, the registered name such as
  "Microsoft Excel Worksheet"), and Location, which is the full path with its
  last part removed (`PathRemoveFileSpecW`). Location is set with
  `SetDlgItemTextWithToolTip`, so a path too long for the box shows its
  full text as a tooltip.
- `_UpdateSizeField`, files: Size is a message built from
  `StrFormatByteSizeW` and `AddCommas64`, "211 MB (221,664,650 bytes)". A
  zero size uses a different string id (0x1016 instead of 0x1017).
  Created, Modified and Accessed follow. On a volume whose file system
  name contains "FAT", Accessed is shown without the time.
- `_UpdateSizeField`, folders: only Created is set directly. Size and
  Contains come from `CFolderContentsInfo::CreateSizeThread`.
- `CFolderContentsInfo::SizeThreadProc` walks the items on a worker
  thread. Files add their size and count as files. Folders go through
  `FolderSize`, a `CShellTreeWalker` whose `CFolderSizeTreeWalkerCB`
  adds each found file to the totals (`FoundFile`) and counts each entered
  subfolder (`EnterFolder`). The walk stops when the dialog clears its
  "keep going" flag. The thread starts a 250 ms `SetTimer` on the dialog
  and posts a final `WM_TIMER` when done. The dialog proc answers each
  `WM_TIMER` with `_UpdateSizeCount`, which rewrites Size and Contains
  (string 0x2207, "1,624 Files, 169 Folders").
- `SetDateTimeTextEx` leaves the line blank when the `FILETIME` is zero.
  Otherwise it calls `SHFormatDateTimeW` with flags 0x1c (`FDTF_RELATIVE |
  FDTF_LONGDATE | FDTF_LONGTIME`), or 0x14 (no time) for Accessed on FAT:
  "Yesterday, November 21, 2025, 2:39:49 PM".
- OK and Apply rename the item through `_ApplyRename` (`PSN_APPLY`).

Macsplorer (`core::properties`, `app/properties.rs`): one General tab in
its own borderless window, opened from Properties at the bottom of the item,
empty-space and Folders pane context menus, File > Properties and Cmd+I
(Finder's Get Info key, instead of Alt+Enter). Left out on purpose: the
Sharing and Security tabs, Size on disk, Opens with, Advanced, and the
caption's "?" button. Metadata is read and folders are
walked on a worker thread, and the totals are shown every 250 ms as in
Windows. A rename from the dialog goes through the same checks and undo as
an in-place rename. When a rename fails, the dialog stays open and the
error goes to the status bar instead of a message box. Guesses: the walk
counts hidden items, doesn't count the folder itself, and treats symlinks
as files without following them. Packages (`.app`) get the folder page.
Not from Explorer: the Windows Attributes check boxes (Read-only, Hidden)
become the Unix permissions, a grid of Owner, Group and Others by Read, Write
and Execute at the bottom of the page. A click toggles a box, and Apply or
OK runs a `chmod` (the special bits stay) after any rename; Apply is enabled
while the name or a box differs from what is on disk. A failed `chmod` keeps
the dialog open and goes to the status bar. Changing permissions isn't in
the undo stack. The boxes are disabled for the root and mounted volumes, like
the name. For a symlink they show, and change, the target's permissions, as
`chmod` does. A symlink also gets a
Source line (the link's own path) and a Target line (what `readlink` returns,
as written, with "(not found)" after a dangling one). Finder aliases get
neither. The window's height follows the rows an item has (folder, file,
link) and stays fixed for it.
Apply is enabled only while the name differs from the current one (Windows
enables it on any edit). The type name of files and packages is Finder's
Kind (`NSURLLocalizedTypeDescriptionKey`, read on the worker thread), which
plays the role of `SHGetFileInfoW`'s registered name; plain folders keep
"File Folder", and Linux falls back to "XLSX File" style names.

Several selected items (Properties from the item menu, File > Properties or
Cmd+I with two or more items selected) open one dialog, as Explorer does:
the name box lists the names and can't be edited, Type reads "All of type X"
or "Multiple types", Location "All in <folder>", and Size and Contains add
the items up (`SizeThreadProc` goes through the selected items: files add
their size, a selected folder counts as a folder and is walked). No dates or
permissions are shown. The title is just "Properties". The page's wording
wasn't traced; it follows Windows 2000. Not traced: the multi-item icon.

Dates and numbers follow the system locale, as Windows takes them from the
user locale (`app/locale.rs`). On macOS a date line is `NSDateFormatter`'s
full date, with the weekday replaced by the relative formatter's word for
today or yesterday, then ", " and the medium time. The Details view's
Modified column uses the short date and short time. Thousands and decimal
separators come from `NSLocale` and apply everywhere (Size column, status
bar, Properties). On Linux the C library's `LC_NUMERIC` and `LC_TIME` are
used: the weekday and short date (`%A, %x`) stand in for the long date,
and "Today" and "Yesterday" stay English. Byte units and the words
("bytes", "Files", "Folders") stay English. With several items selected
the dialog shows just one of them (Windows has a separate multi-item page,
`_InitMultiplePrsht`).

**Customize tab (macOS, not from Explorer).** Folders that can be renamed
get a second tab for Finder's folder customization (macOS 26 and later):
the folder color and an emblem, an emoji or an SF Symbol. The tab's name and
its Restore Default button follow XP's Customize tab from memory (Win2000
has none). Finder's storage, found by reading folders Finder customized and
checking `NSWorkspace`'s drawing of folders written by hand
(`macsplorer_core::folder_style`): the emblem is JSON in
`com.apple.icon.folder#S` (`{"emoji":"…"}` or `{"sym":"star.fill"}`), drawn
only while `kHasCustomIcon` is set; the color is the *last* tag with a color
in `_kMDItemUserTags` (a binary plist array of `"Name\nN"`). The label bits
in the Finder flags are kept in step but don't change the drawing. A new
color replaces Finder's standard color tags (Red, Orange, ...) and goes
last; the user's own tags stay. "In Finder" is `NSWorkspace`'s icon for an
empty folder in the temporary directory given the chosen style. Typed text
made of ASCII letters, digits and dots is a symbol name, anything else an
emoji. Ctrl+Tab and Ctrl+Shift+Tab switch tabs, like a property sheet.
Not checked: what Finder does with "no color" when a tag of the user's own
has a color (that tag keeps coloring the folder here).

Finder only draws the color on a folder that has an emblem: a tag alone,
or the flag with `{}`, leaves the default folder. A blank emoji
(`{"emoji":" "}`) gives the colored folder with nothing on it; Macsplorer
uses that on temporary folders to draw a colored folder without its emblem
and never writes it to a real one. Finder reads no color from the JSON
(`color`, `tint`, `hex`, `rgb` and a few value shapes were tried). Colors
of Macsplorer's own ("Other...", the Color dialog) go in
`dev.macsplorer.folder-color` as `#rrggbb`; Finder gets the nearest tag
color: Gray when the Oklab chroma is under 0.03, else the tag with the
nearest Oklab hue (a plain distance calls dark colors gray). The own color
is only used while the tag still is its nearest one, so a color changed in
Finder wins. It's drawn by recoloring Finder's Gray folder: pixels that
differ between a Gray and a Blue drawing belong to the folder and get the
color with the Gray's shading (`folder_style::tint`); the rest, an emoji,
stays. Where it shows: Large Icons and the dialogs' 32 px icons draw color
and emblem, List and Details only the color (an emblem-only folder is the
yellow folder there), the Folders pane never either (the user's choice).
The Color dialog is `ChooseColor` from memory (comdlg32 isn't analyzed):
48 basic colors, 16 custom boxes kept in `settings.json`
(`custom_colors`), "Define Custom Colors >>" with the hue/sat spectrum, the
lum bar, Color|Solid and the six number boxes; HLS uses Microsoft's
published KB 29240 conversions (`macsplorer_core::hls`). A double click on
a box is OK. Picking from the spectrum while lum is 0 or 240 starts at 120
(macsplorer's choice). Help is left out.

Linux has colors only (`folder_style::EMBLEMS`): `user.macsplorer.folder-color`
holds `#rrggbb`, which no other program reads; Finder's seven colors are
just presets there. Every folder in a listing gets one `getxattr` (there
are no Finder flags to check first). A colored folder is the Win2000
folder art in the color (`theme::paint_colored_folder`: edge and highlight
derived like the yellow folder's), which macOS also shows while Finder's
drawing is still being made. File systems without user attributes refuse
the write and the error goes to the status bar.

### 2.9 Type-ahead search (comctl32 v6.10 `CLVKeyboardManager`)

Typing in the item view is the list view's incremental search, not something Explorer
implements itself. Traced in `CLVKeyboardManager::OnChar` (comctl32 0x171bbf83c),
`IsISearchTimedOut` (0x171ab4a14), `CLVItemStore::OnFindItem` (0x171a0f968) and
`CLVItemStore::_CompareString` (0x171bacb48). The notification `LVN_INCREMENTALSEARCH`
is `_IncrementalSearch` (0x171bc00bc, `LVN_FIRST - 63`).

**Per `WM_CHAR`, observed:**

1. The view keeps a typed-text buffer and the time of the last character. If more than
   **1000 ms** (a fixed constant, not the double-click time) passed since the last one
   (`GetMessageTime` difference), the buffer starts over. The time updates with every character,
   whether or not it found anything.
2. A character below `0x20` (Backspace, Tab, Enter, Esc) clears the buffer and the rest is
   skipped. `OnKey` also clears it whenever an arrow/Home/End/Page key moved the focus.
3. Otherwise the character is appended. A buffer of exactly one space is not a search: it
   toggles the selection of the focused item (the Space key) and clears the buffer.
4. The view sends `LVN_INCREMENTALSEARCH` to the owner. If it answers `-3` (handled), the view
   does nothing more. **Unverified:** whether the shell's `CDefView` answers it; no public symbol
   names its notify handler.
5. Otherwise it searches (`OnFindItem` with `LVFI_STRING | LVFI_SUBSTRING | LVFI_WRAP`, flags
   `0x26`) for the first item whose **displayed name starts with the buffer**, trying every
   item once, wrapping at the end. Matching is `CompareStringW(LOCALE_USER_DEFAULT,
   NORM_IGNORECASE | NORM_LINGUISTIC_CASING)` on the first *n* characters of both, so it ignores
   case but not accents. It reads the same item text the view shows (so the extension counts
   when it is displayed).
6. Where the search starts:
   - first character of the buffer: **after** the focused item;
   - buffer made only of the same character ("aaa"): after the focused item again, and the search
     text is that single character. Pressing one letter repeatedly cycles through the items
     starting with it (`FUN_171ab6573` tests "all characters equal the first");
   - any other buffer ("ab"): **at** the focused item (the code starts one before the focus and
     `OnFindItem` begins at the next item), so extending the text keeps the current item while it
     still matches.
7. A match becomes the only selected item, the focus item, and is scrolled into view
   (`EnsureVisible`). **Inferred:** with Ctrl held (bit 0 of a manager flag, source not traced) the view only moves the focus and leaves the selection.
8. No match: nothing changes, and `MessageBeep(0)` sounds once until the buffer clears (a flag
   next to the buffer).

Macsplorer (`core::isearch`, `Explorer::keyboard` → `Action::TypeAhead`): steps 1 to 8 for the
item view in all three view modes, matching on `Entry::display_name`, in the order shown (sort and
search filter included). The Space toggle is in (§2.12). Left out: the beep and the Ctrl "focus
only" variant. Typing in the Folders pane reuses the same search, see §2.12. Mouse clicks don't clear the buffer; the 1 s timeout covers them. Rust's
`to_lowercase` stands in for the locale's case folding.

### 2.10 Choosing columns (shell32 `CDefView`, `CColumnDlg`)

**Header menu, observed.** A right click on the details header goes from `CListViewHost::_DoColumnsMenu`
(0x18041fb90, which hit-tests the header for the clicked column) to `CDefView::OnVisibleColumnMenu`
(0x18027a350). That builds the menu in `_AddColumnVerbsToMenu` (0x18027cf6c):

- In Details view only, and only on Windows 7 and later: "Size Column to Fit" (0x7049, left out for one
  particular column key), "Size All Columns to Fit" (0x704a), then a separator. Win2000 had neither.
- The column manager's column 0 (Name) as command 0x7500, `MF_CHECKED | MF_GRAYED`: it can't be hidden.
- Every other column in the column manager's full-list order as 0x7500 + index, checked when shown.
  Columns with `SHCOLSTATE_HIDDEN` are skipped. A `SHCOLSTATE_SECONDARYUI` column that isn't shown is left
  out of the menu, and so is everything after the 25th entry.
- If anything was left out: a separator and "More..." (0x7033, string 0x1099), which runs
  `DoCustomizeColumns`.

`CDefView::ToggleVisibleColumn` (0x18027cbb0) handles a column command: it takes the shown columns in
order; a shown column is removed; a hidden one is inserted at its index in the full list, or appended
if fewer columns are shown; then `SetColumns`. **Inferred:** how the column manager orders hidden
columns in its full list wasn't traced. The Win2000 menu (screenshot) lists Name, Size, Type, Modified,
Attributes, Comment, Created, Accessed, then More..., so the first eight are the primary columns.

**"More..." dialog, observed** (`CColumnDlg`, Win2000 title "Column Settings", XP "Choose Details"):

- `_InitListView` (0x18042a640): a list view with check boxes. First the shown columns, checked, in
  display order; then the remaining ones with a non-empty name, unchecked, sorted by name. Row 0 starts
  selected and focused. The width box takes at most 3 characters.
- `DlgProc` (0x18042a268): unchecking an always-visible column (Name) is refused with a beep. A
  double-click (`NM_DBLCLK`) toggles the row's check. Move Up / Move Down move the selected row
  (`_MoveItem` 0x18042aad8 swaps it with its neighbor, keeps it selected and scrolls to it, beeps at the
  ends). Show / Hide check or uncheck the selected row. Editing the width box sets the selected
  column's width, marked as changed.
- `_UpdateDlgButtons` (0x18042af6c): Move Up is enabled unless on the first row, Move Down unless on the
  last. Show only for an unchecked row and Hide only for a checked one, neither for Name. The width box
  shows the selected column's width and is disabled for fixed-width columns.
- OK runs `_SaveState` (0x18042acd8), only if something changed: the checked rows in list order become
  the shown columns, then the widths typed in the box are applied. Cancel changes nothing.

**Attributes text, not traced.** `CFSFolder::_BuildAttributeString` (Windows.Storage 0x18027470c) just calls
`PSFormatForDisplay(PKEY_FileAttributes)`, so the letters come from propsys.dll, which isn't in this setup.
From memory: one letter per set attribute (R, H, S, A, C, E...), blank if none.

### 2.11 Name conflicts in a copy or move (Windows.Storage `CCopyWorkItem`)

Decompiled from Windows.Storage.dll build 10240 (Ghidra, names from the PDB publics).

- `CCopyWorkItem::_CheckSameName` (0x18001a1e0): when the destination name equals the source's
  name (`StrCmpCW`) and the destination folder is the source's own folder
  (`CompareItemsByParsingName` == 0), the item is reported with `FileFailure(...,
  COPYENGINE_E_SAME_FILE)`. That is the case Explorer turns into "Name - Copy" for a copy;
  a move into its own folder does nothing.
- `CCopyWorkItem::ConfirmOverwrite` (0x180018920), when the destination has an item of that name:
  - If the source is a folder (`IsFolder`) and the existing item merges
    (`IShellItem_ShouldMergeOnConflict`): in a folder that allows only unique names
    (`IShellItem_ParentHasUniqueFileNames`), `CFileOperation::GetMergeFolderStats` counts the
    conflicts inside. With none, and not a rename, the result is `COPYENGINE_S_MERGE` (0x270006)
    without a prompt. Otherwise the prompt is `COPYENGINE_E_ALREADY_EXISTS_FOLDER` (0x8027002C).
    A parent without unique names gives `COPYENGINE_S_KEEP_BOTH` (0x27000C).
  - Anything else prompts with `COPYENGINE_E_ALREADY_EXISTS_NORMAL` (0x80270029). With operation
    flag 0x400000 set (not used by Explorer's paste, not traced further) it doesn't prompt but
    compares the times (`CFileOperation::CompareFileTimes`): `COPYENGINE_S_YES` (replace) for
    one result, `COPYENGINE_S_USER_IGNORED` (skip) for two others.
  - The prompt (`_ConfirmWorker` → `CFileOperation::PromptUserOrQueue`) is only shown while the
    work item's stored answer (offset 0x398) is still `COPYENGINE_S_NOT_HANDLED` (0x270003):
    an "apply to all" answer stands for the rest of the operation (`_AllowsApplyToAll`).

Not traced: the dialog itself. Build 10240 shows the Windows 8 "Replace or Skip Files" UI
(`COperationStatusBroker`, `CFileOperationConflictItems`); its strings are in the MUI file.
macsplorer draws Windows 2000's "Confirm File Replace" / "Confirm Folder Replace" from memory.

### 2.12 Keyboard use: the focus cycle, list and tree keys

Traced in ExplorerFrame.dll: `CShellBrowser::_CycleFocus` (0x18011f974),
`_CycleFocusHelper` (0x18011fa60), `_MayGetNextToolbarFocus` (0x1801218d4), `_OnFocusChange`
(0x180047c7c), `_ExplorerTreeHasFocus` (0x18011ff04), `CBandSite::_GetNextTabstopBand`
(0x1800db308) and the `IsVK_TABCycler` helper (0x1800acff4). In comctl32 v6.10:
`CLVKeyboardManager::OnKey` (0x171a33128), `TV_KeyDown` (0x171ad9a28), `TV_OnChar`
(0x171ad9dc0). The accelerator tables (F4, Alt+D, Ctrl+F, Backspace and so on) live in the `.mui`
resource files, which aren't part of this setup, so the key list for the frame is from Windows'
documentation, not traced.

**Focus cycle, observed:**

1. A key cycles when it is a `WM_KEYDOWN` of **Tab or F6** with **Ctrl up**. Shift reverses the
   direction. Any other key, or Ctrl+Tab, goes to the focused control as usual.
2. The same check sends `WM_CHANGEUISTATE(UIS_CLEAR, UISF_HIDEFOCUS)` to the window, so focus
   rectangles become visible from then on (the "hide keyboard navigation indicators" state).
   `OnKey` and `TV_KeyDown` do the same after a navigation key.
3. The frame keeps its toolbars (bands and explorer bars) in an array plus the index of the one
   holding the focus, `-1` for the view. From the view, forward starts at toolbar 0 and backward
   at the last one. From toolbar *i* it tries *i ± 1* onwards, each toolbar deciding whether it takes
   the focus (`UIActivateIO`). When none does, the focus goes back to the view.
4. Before leaving the view forward, the view itself is asked first, so a view with several parts
   can step through them.
5. Inside a rebar the bands are visited in band order, and a band only counts when it is visible
   and has `WS_TABSTOP`.

**Inferred:** the order of the toolbars in the array. In Windows 2000 the rebar with the menu,
the buttons and the address bar comes first and the Folders explorer bar after it, so Tab from
the list reaches the address bar, then the Folders pane. Build 10240's order wasn't decoded (its
`AddToolbar` calls go through the vtable).

**List view keys (`OnKey`), observed:**

- Arrows, Home, End, Page Up and Down move the focus and select the new item alone. **Shift**
  selects from the anchor to it. **Ctrl** moves only the focus: the selection and the anchor stay.
- **Ctrl+Space** toggles the focused item and sets the anchor there. **Shift+Space** selects from
  the anchor to the focus. A plain Space reaches `OnChar` as a one-space search buffer, which
  toggles the focused item (§2.9 step 3).
- With **Alt** held, `OnKey` returns without doing anything (Alt+arrows belong to the frame).
- In Details, Left and Right scroll sideways (a line, a page with Ctrl) instead of
  moving the focus.
- **Ctrl+keypad +** sizes every Details column to fit (`LVSCW_AUTOSIZE` for each).
- Enter sends `NM_RETURN`, which the shell turns into the default verb (Open).

**Tree view keys (`TV_KeyDown`), observed:**

- Up and Down move the selection through the shown rows. Home and End go to the first and last
  shown row. Page Up and Down move by the number of fully visible rows **minus one**.
- Right expands a collapsed item that has children; on an expanded one it moves to the first
  child. Left collapses an expanded item, otherwise it moves to the parent. Backspace moves to
  the parent too.
- Keypad **+** expands and **−** collapses without moving; keypad __*__ expands the whole branch.
- With **Ctrl**, the arrows, Page and Home/End keys scroll the tree and leave the selection.

**Tree type-ahead (`TV_OnChar`), observed:** it shares the list's typed-text buffer rules
(`IncrementalSearch`, 1 s timeout). The search starts **after** the item that had the focus when
the text began and goes through the shown rows, wrapping once, comparing the first *n* characters
without case. Repeating one letter ("aaa") also tries the last letter alone, so the letter steps
through the rows. A match is selected as if by keyboard (`TVC_BYKEYBOARD`).

**Keyboard selection opens later (`CNscTree::_OnNotify`, ExplorerFrame 0x18003d1e0), observed:**
on `TVN_SELCHANGED` the tree opens the folder in the view at once only when a flag set around a
mouse click is on. Otherwise it kills and restarts a timer (id 0x87) of `GetDoubleClickTime()`,
500 ms by default, and opens the folder when the timer fires. Arrow keys and type-ahead therefore
move only the highlight while keys keep coming, and the view lists just the folder where they stop.
The selected row is scrolled into view by the tree control itself, only as far as needed
(`TVM_ENSUREVISIBLE`), without animation.

Macsplorer (`core::focus`, `Explorer::keyboard`, `tree_key_action`):

- Tab and F6 cycle through the item view, the address bar (switched to its text field with the
  path selected, like the Win2000 address combo), the search field and the Folders pane, and back.
  Shift reverses; Ctrl, Alt and Command don't cycle. egui's own Tab traversal through buttons is
  turned off in the main window. A hidden Folders pane is skipped. While a rename is open, Tab does
  nothing. When the view gets the focus with no focused item, the first item gets it (inferred).
- Focus rectangles are drawn once any of these keys was used, around the focused item's label in
  all three views and around the selected row of the tree, in the color an XOR gives on that
  background. While the address or search field has the keyboard, the panes draw their selection
  gray as unfocused.
- F4, Alt+D and Cmd+L go to the address bar; F3 and Cmd+F to the search field (Windows: F3, Ctrl+F,
  Ctrl+E; Cmd+E is New Window here). Esc in either field gives the keyboard back to the pane that
  had it.
- List: Space and Shift+Space as observed. Ctrl+arrows and Ctrl+Space are left out on purpose:
  on macOS they are system shortcuts (Mission Control, input sources). Details Page Up and Down move by the rows that fit minus one, the tree's rule (inferred; the report view's
  paging wasn't traced).
- Tree: Home, End, Page Up/Down, + and − as observed (egui doesn't tell keypad keys apart, so the
  main + and − keys do it). Typing selects with `core::isearch`, which gives the same results as
  `TV_OnChar` except right after a wrap. Backspace stays Back (the frame's accelerator).
  Keyboard selections open after `tree::OPEN_DELAY_MS` (500 ms, the default double-click time;
  macOS's own setting isn't read) from the last move; clicks open at once, and navigating
  elsewhere first drops the pending open.
- Scrolling an item or row into view jumps there in all three views and the tree, as the Windows
  controls do; egui's scroll areas would otherwise ease to it.
- **Not done:** Shift+F10 and the menu key (egui's popup menus can't be driven by the keyboard),
  Alt and F10 to the menu bar, Ctrl+scroll in the tree and Details, Ctrl+keypad +, keypad *, and
  Tab inside the view.

### 2.13 The copy progress dialog (shell32 `CProgressDialog`)

Disassembled from shell32.dll build 10240 (`tools/re/dis.py`, names from the PDB publics). The copy
engine reports to `CProgressDialog`, whose window is the DirectUI `CProgressDialogUI`.

- `CProgressDialog::GetDialogDisplayDelayTime` (0x18016b4f0) returns 1000 ms.
- `CProgressDialogUI::_HandleTimerMessage` (0x18000cd08), timer 1: once
  `CProgressTimer::GetMillisecondsProgress` reaches 1000 (0x3e8) it kills the timer and calls
  `_DisplayDialog`. An operation that ends sooner never shows the dialog. That clock is the
  progress time, which `CProgressDialog::PauseTimer` / `ResumeTimer` stop and restart, so time
  spent on a question doesn't count (Inferred from the names: the pause callers weren't traced).
- Same function, timer 2 (ending): when the dialog was displayed (flag at 0x178) it ends only
  once `GetMillisecondsActualElapsed` minus the time stored at 0x220 reaches 1000 ms, so a shown
  dialog stays up at least a second. Not displayed: `EndDialog` at once.

Not traced: the layout and wording (Windows 10 has its own DirectUI layout; the strings are in
the MUI file) and how the time left is estimated (`_UpdateTimedStats`). macsplorer draws Windows
2000's "Copying..." / "Moving..." dialog from memory: animation, item name, "From 'X' to 'Y'",
the block progress bar, "N Seconds Remaining" / "N Minutes Remaining", Cancel.

### 2.14 Drag and drop (shell32 `CFSDropTarget`, comctl32)

Decompiled with Ghidra from shell32.dll, ExplorerFrame.dll and comctl32.dll build 10240 (names
from the PDB publics). `CFSDropTarget` is the drop target of a file system folder, in the view
and in the tree alike.

- **Drag threshold.** comctl32's `CheckForDragBeginEx` (0x171a31750) makes a box of
  `GetSystemMetrics(SM_CXDRAG)` by `SM_CYDRAG` on each side of the press (raised to 4 if smaller;
  4 is the default) and waits: a move that leaves the box (`PtInRect`, so the right and bottom
  edges are outside) starts the drag; a button message first ends it as a click. The tree uses the
  same function (`TV_CheckForDragBegin` 0x171ad8678).
- **Keys** (`CFSDropTarget::_GetDefaultEffect` 0x1804268c8). The key state masked to Shift, Ctrl
  and Alt: Shift alone moves, Ctrl alone copies, Ctrl+Shift or Alt (with anything) makes a shortcut.
  Without keys, a preferred effect the source put in the data object wins if allowed, then the
  default below. The result is masked with the allowed effects, so a forced effect the source
  doesn't offer refuses the drop.
- **Default** (`_FilesystemAdjustedDefaultEffect` 0x180425d44). An effect stored for the source
  folder in a per-drop list (a clipboard format, not identified) wins; then the target's
  `desktop.ini` `[.ShellClassInfo] DefaultDropEffect` (`_EffectFromFolder` 0x180425c30); then: the
  source is a drive's root (`PathIsRootW`) → shortcut; `PathIsSameRootW(source, target)` → move;
  another drive → copy, unless a per-item property check (`FUN_180510680`, a property compared
  with a GUID for every item, not identified) says otherwise, which moves. A second untraced check
  on the source (`FUN_18021768c`) also gives a shortcut. Last: a move becomes a copy when the
  source allows only copy (of copy and move), and a copy a move when it allows only move.
- **Allowed effects** (`_DetermineEffects` 0x180425178): each clipboard format the data object
  offers has a handler that adds what it supports; when the default isn't among them, the first of
  copy, move and shortcut that is.
- **Auto-scroll.** shell32's `DAD_AutoScroll` (ordinal 129) is an empty stub in this build (the
  export shares a body that returns 0 with a dozen others), though `CDelegateDropTarget::DragOver`
  (ExplorerFrame 0x180173670) still calls it. Not traced further: the margin and speed.
- **Not traced:** the tree's hover-to-expand delay while dragging, the drag image's exact
  look (`CLVDragDropManager::OnCreateDragImage` builds it from the items' icons and labels),
  right-drag's Copy Here / Move Here / Create Shortcuts Here menu (`_DragDropMenu`).

## 3. Mapping to macsplorer (design choices)

| Windows | Rust/macOS |
|---|---|
| Rename selection rule (§2.1) | Port it directly. `NSTextView.setSelectedRange` after `beginEditing`. Use the real extension from `NSURL.pathExtension`, and only when extensions are visible. |
| "Hide extensions" | Not traced; macsplorer's own rule (`Entry::hidden_extension`): the extension stays visible when the rest of the name has one too (`Invoice.pdf.app`, `photo.jpg.command`), so apps and scripts can't pass for documents. Hidden extensions are put back on rename, for packages as well as files. |
| `IItemNameLimits` | On APFS/HFS+ only `/` is invalid (`:` is shown as `/` in Finder). Block `/` and NUL, cap at 255 UTF-8 bytes (`NAME_MAX`) instead of 128 chars. |
| `StrCmpLogicalW` | `localizedStandardCompare:` (Finder's own natural sort), or a Rust natural-sort crate (`natord`, `lexical-sort`) if you need determinism across locales. Keep a "plain sort" setting that matches `NoStrCmpLogical`. |
| `CViewSettings` per-folder/per-type view memory | Store view state keyed by folder path plus a "folder type" fallback (Documents/Pictures/Music/Videos/General) in your own SQLite/JSON file. Don't write `.DS_Store` files. |
| Status bar free space | **Unverified, from Windows 2000 behavior (not traced in build 10240, whose status bar only shows the item count).** With nothing selected the first pane reads `N object(s) (Disk free space: 4.65 GB)`; with a selection it reads `N object(s) selected` and drops the free space. The value is the user-available bytes on the folder's volume (`GetDiskFreeSpaceEx`, `lpFreeBytesAvailable`), formatted with `StrFormatByteSize`. macsplorer uses `statvfs` (`f_bavail * f_frsize`) on the listing worker thread. |
| Toolbar | The Windows 2000 standard buttons Macsplorer has: Back, Forward, Up, Folders, Refresh (not on Win2000's bar), Undo, and Views, a whole-button drop-down (`BTNS_WHOLEDROPDOWN`) with the view modes. Search, History, Move To, Copy To and Delete are left out. Icons are hand-drawn after the Win2000 ones. |
| View modes (§2.4, §2.5) | Win2000 look, so follow the ListView (§2.5): 75 × 75 slots, 32 px icon centered 2 px from the top, label 3 px below, wrapped in 71 px and folded to two lines unless focused; clicks hit the image ± 8 px or the label, not the whole slot; selected icons blended 50% (25% when the view is unfocused) toward the highlight color; spatial arrow keys. List as traced: columns as wide as the widest name + 28 px, filled top to bottom, index arrow keys; the List view's marquee auto-scrolls at the left and right edges instead (inferred, the horizontal case wasn't traced). Marquee as traced in both views; Details uses the icon plus the label's highlight box as select bounds, the band is drawn as black dots since egui can't XOR, and auto-scroll runs while the pointer is at or past the view's top or bottom edge at a guessed 20 lines a second. From build 10240 (§2.4): selection, focus and sort survive a mode switch. Labels use Tahoma 12 px instead of 8 pt, so rows and two-line labels come out a little taller than Win2000. View mode is per window until per-folder memory exists. |
| Column header (§2.6) | Hit test and drag as traced (`macsplorer_core::columns`), always full drag. macOS has no split-open cursor, so both divider zones use the system resize cursor. Auto-size measures the cells with macsplorer's own padding (`_ComputeColumnWidthForItem` wasn't decoded) and applies the caption + 37 px minimum. The double-click is recognized on the second release (egui), not the second press. Widths are shared by all folders and last until the window closes. |
| Choosing columns (§2.10) | Header menu, toggle and More... dialog as traced (`macsplorer_core::columns::ColumnSet`, `column_dialog.rs`), without the Win7 "Size Column to Fit" items. Shown by default: Name, Size, Modified, Attributes (the user's choice; Windows shows Name, Size, Type, Modified). Columns: the Win2000 eight plus Owner and Group as secondary columns (More... only until shown). **Attributes** shows the Unix permissions as `ls -l` does (`-rw-r--r--`, symlinks as the link itself): like Explorer's attribute letters, each position says whether a flag is set, and it reads the same as `ls`, Finder's Get Info and Linux file managers, where octal (`0644`) needs decoding. **Comment** is Finder's comment (the `kMDItemFinderComment` attribute; Linux: `user.xdg.comment`). Created is the birth time, Accessed the atime. Hidden columns keep their place in the full list, so the menu order doesn't change after a toggle (inferred). Name can be moved in the dialog like in Windows. No beep when Name is unchecked. The dialog wording is XP's from memory (the strings are in the MUI file). Shown columns are per window like the widths. Default widths for the new columns are guesses. |
| Text size | **Not from Explorer and not traced; from Windows documentation.** Explorer has no text size option of its own: its list view and tree take the system Icon font, and Windows 2000 scaled every program through Display Properties > Settings > Advanced > Font Size (Small Fonts 96 DPI, Large Fonts 120 DPI = 125%, or a custom percentage; a restart applies it) and the Appearance schemes "Windows Standard (large)" and "(extra large)". Build 10240 has DPI scaling at 100 to 200% in Settings > Display, plus "Change only the text size" per element in Control Panel. A DPI change grows text and layout together, so Macsplorer does the same with egui's zoom factor rather than enlarging only the letters (`macsplorer_core::text_size`): View > Text Size, worded after Internet Explorer 5's menu of that name, with Normal 100%, Large 125% (the Win2000 Large Fonts), Larger 150% and Largest 200%. Cmd + / Cmd - step through those choices and Cmd 0 goes back to Normal. Remembered in `settings.json`. |
| Show Package Contents | **Not from Explorer, from Finder (macOS 27, disassembled).** `-[TBaseBrowserViewController validateShowPackageContents:]` enables it for exactly one selected item that is a package (two further checks on the item weren't traced). `cmdShowPackageContents:` passes that item and the originating window to `+[TGlobalWindowController targetSelectOrCreateWindow:inTarget:selectNodes:]`, which reads the modifier keys like any folder open (`optionsForModifiers:windowController:`): without modifiers the package opens as a folder in the same window, with nothing selected. macsplorer puts it in the item menu under Open for `RenameKind::Package` and navigates in place; the new window with Command isn't done. |
| Name conflicts (§2.11) | `macsplorer_core::fileops::transfer` asks a callback, which the paste worker turns into a question on a channel and waits on; `conflict_dialog.rs` shows Win2000's dialogs with Yes, Yes to All, No and Cancel (Enter, Y, A, N, Esc). The folder question's second sentence says what macsplorer does: it asks about each file inside. Yes to All covers both kinds of question. **Not from Windows:** a replaced item goes to the Trash instead of being destroyed, and Undo brings it back (`UndoOp::Group`). A move removes the source folders a merge emptied, and Undo recreates them. A file and a folder of one name get the replace question, like build 10240 (only folder onto folder merges). |
| Drag and drop (§2.14) | `macsplorer_core::dragdrop` has the traced rules for plain files and folders: same volume moves, another copies, a volume's root links, keys force an effect, refused when the source doesn't allow it. The keys are the Mac's (Option copies, Command moves, both make a shortcut) since AppKit applies those to every drag from Finder anyway; Linux uses Windows' (Shift, Ctrl, Ctrl+Shift or Alt). A shortcut is a symbolic link named "Name - Shortcut.ext" (Windows: "Name.ext - Shortcut.lnk"). egui's click distance is set to the 4 px threshold, though egui measures a circle and not a box. Inside the window the drag is the app's own (`item_drag.rs`): the dragged items drawn half transparent and selected under the pointer, the target folder drawn selected, Explorer's cursors; the tree's selection isn't highlighted while one of its rows is the target (from memory). Over a file the drop is refused. **Not from Windows:** moving items into the folder they're in, or a folder into itself, is refused (Explorer repositions icons there, `CDefView::IsDropOnSource`). Guesses: a collapsed tree folder expands after the pointer rests on it for 1 s; auto-scroll within one row of an edge at 20 rows a second. On macOS (`native_drag.rs`) a drag that leaves the window becomes an AppKit drag of file URLs offering every operation, so the target decides (Finder moves on the same volume and copies to another, the Dock's Trash deletes), with the system icons as its image and the arrow cursor (the window's last cursor, often the "no" sign over the items' own folder, would otherwise stay up over Finder, as winit sets cursors only inside the window). After a drop or paste the new items are selected and the first is focused and scrolled into view (`SVSI_FOCUSED | SVSI_ENSUREVISIBLE`, from memory). Drops from Finder and other apps reach the window's view, which gets AppKit's drop methods (winit's own handler reports no position) and answers from the last frame. Not done: right-drag menu, drop on an app to open with it, dragging out on Linux, dropping in on Linux (winit 0.30 gives no position on X11 and nothing on Wayland). |
| Address autocomplete (`CAddressEditBox`, `CAutoComplete`) | **Not from Explorer, which drops down a list; this follows the Fish shell's autosuggestion** (`macsplorer_core::autosuggest`). While the address field is edited, the rest of the folder name being typed shows dimmed after the text, and Right or End with the cursor at the end takes it (Enter and Go still use what was typed). Only folders are predicted, in the folder before the last `/`, for paths that start with `/` or `~/`; nothing shows before a first letter is typed, for a name that is typed in full (the prediction would be a lone `/`; a longer sibling such as `Documents` for `Doc` is still offered), or after Backspace or Delete shortened the text, until something is typed again (it would only bring back what was just deleted). A name in the typed case beats one in another case (taken in its own case when accepted), a name that only matches with its accents left out comes last (`A` finds `Äpfel`; typed accents have to fit, so `ä` doesn't find `Apfel`), the first in natural order wins, hidden folders need a leading dot, and names match in NFC. The folder's names are read on a worker thread and kept until the field closes, so the prediction appears a moment after a new folder is entered. Fish also predicts from history first and completes with Tab; here Tab keeps cycling the panes (§2.12), and history isn't used. |
| `CTravelLog` | `Vec<Location>` with a cursor. The back/forward dropdown menu lists entries (`_ShowBackForwardMenu`). |
| Breadcrumb (`CBreadcrumbBar`) | Each crumb is a button with a chevron dropdown listing subfolders (`_OnDropDown`). Click empty space → edit mode with the full path (`_GoToEditMode`). Crumbs are drag sources and drop targets. Collapse overflow into a root dropdown (`_AddRootDropdownItems`). |
| `CNscTree` change notifications | `macsplorer_core::watch`: `notify` (FSEvents on macOS, inotify on Linux) watches the open folder and every expanded Folders pane folder, each without its subfolders, and reloads them on worker threads as usual. **Not traced:** how Explorer coalesces `SHChangeNotify` events. A folder reloads 0.2 s after its last change, or at the latest 1 s after the first, so a file being written updates about once a second (guesses). Focus, anchor and selection follow their items across a reload. Changes inside `.git` don't reach the repository folder's watch, so the Git summary only refreshes for changes in the open folder itself. |
| Copy progress (§2.13) | `macsplorer_core::copy_progress` has the traced timing (shows after 1 s of progress, paused while a name-conflict question is up; once shown, stays at least 1 s); `progress_dialog.rs` draws it in its own window. `fileops::transfer_with` counts the sources first, then reports units of bytes plus 4 KiB per item (a move within a volume counts one item per source), not traced. Files are copied one by one: `copyfile` per file with a status callback on macOS (bytes as they go, cloned on APFS), 1 MiB pieces on Linux, so Cancel stops within a big file. Cancel keeps what was done, removes the incomplete copy of the current item, and puts back what that item had replaced; a cancelled move to another volume leaves the original. The time left is the average rate so far, refreshed once a second (guess). |

## 4. Next candidates

Each one can be answered from this setup:

- Sort tiebreakers in `CFSFolder::CompareIDs` (folders before files? secondary column?), plus `_CreateSortColumnArrayWithSecondaryColumn`.
- Breadcrumb overflow rules and dropdown contents.
- Travel log limits and back-menu length.
- Per-folder view defaults per folder type (`CViewSettings::_GetPropertyBagFromTemplate`).
- Navigation pane default roots and expand-to-current-folder behavior.
- Search box: query syntax and filters.
- Submenu show delay: user32 waits `MenuShowDelay` (`SPI_GETMENUSHOWDELAY`, 400 ms by default) of rest on a submenu item before it opens; a click opens it at once. `menu_delay::SHOW_DELAY_MS`, applied to Open With by `delayed_menu_button` (from memory, user32 not traced). Not done: the same delay before an open submenu closes when the pointer moves to another item (egui closes it at once, though it keeps it open while the pointer heads for it), and the other submenus (View > Explorer Bar, Text Size) still open on contact.
- "Open With" submenu contents and order (`COpenWithMenu::QueryContextMenu`, `COpenWithMenu::InitMenuPopup` in shell32). macsplorer currently guesses: default app first, the rest alphabetical, then "Choose Program...".
  The Choose Program dialog (list, "Always use this program to open these files", Other...) is Windows 2000's from memory, untraced. Where Windows keeps the choice per extension in the registry, macsplorer keeps it per type (UTI) in `settings.json`, and the chosen app is listed first.

Unknown: what the folder flag tested in `CDefView::OnBeginLabelEdit` means (`FOLDERFLAGS & 0x80000000`, passed as `SHBeginLabelEdit`'s 5th parameter). When the flag is clear, it removes `WS_EX_RTLREADING` from the edit control.
