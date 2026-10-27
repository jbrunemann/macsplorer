# Archive folder findings (Windows 10 build 10240)

How Explorer shows a `.zip` or `.cab` as a folder. These are the groundwork for browsing archives in Macsplorer.

| Binary | Version | What it is | Public symbols |
|---|---|---|---|
| `zipfldr.dll` | 10.0.10240.20747 | Compressed (zipped) Folders, since Plus! 98, Me and XP | 1,632 |
| `cabview.dll` | 10.0.10240.21072 | Cabinet File Viewer, since Windows 98 and 2000 | 693 |

Method: the same as `file-browser-findings.md`. Ghidra pseudocode by address through REA, names from `symbols/zipfldr.pubs.tsv` and `symbols/cabview.pubs.tsv`, tables read straight from the file. Static only.
**Observed** = read from code. **Inferred** = my interpretation.

Windows 10 keeps the UI text (string tables, dialogs, menus) in `zipfldr.dll.mui` and `cabview.dll.mui`, which we don't have. So every quoted label below comes from memory of XP/Win2000 and is marked as such. String resource IDs are given so they can be checked once the `.mui` files are available (`C:\Windows\System32\en-US\`).

Windows 2000 itself shipped only the CAB viewer. Zip folders came with Me and XP. For Macsplorer's Win2000 look, the CAB viewer is the period piece and the zip folder is the behavior to borrow.

## 1. Component map

| Part | zipfldr | cabview |
|---|---|---|
| The folder (`IShellFolder2`): listing, columns, sort, attributes | `CZipFolder` (57 methods) | `CCabFolder` (45) |
| View callback: window lifetime, help, notifications | `CZipFolderViewCB` | `CCabFolder::MessageSFVCB` |
| Item context menu | `CArchiveItemContextMenu` | `CCabItemMenu` |
| Archive index, cached per archive | `CEnumArchive`, `CArchiveIDList`, `CArchiveEntry` | `CCabItemList`, `CEnumCabObjs` (FDI callbacks `CCabItemsCB`) |
| Temp files for opened items | `CTempFileNameArray` (global `g_pCTFA`) | none |
| Extract All wizard | `CZipWiz` | none (Extract asks for a folder, `CCabExtract::Browse`) |
| Copy, drag out | `CArchiveItemDataObject`, `CZipTransfer` | `CCabObj`, `CObjFormats` |
| Writing into the archive (drop, paste, delete) | `CArchiveDropTarget`, `CZipWriteStream`, `PasteToZipFolder` | none |
| Password prompt | `CPassword` (global `g_password`) | none |
| Send To > Compressed (zipped) folder | `CSendTo` | none |

## 2. Zip folders (`zipfldr.dll`)

### 2.1 Columns

**Observed** (`CZipFolder::MapColumnToSCID` and `GetDefaultColumnState`, table at `0x171235cc0`, 9 entries): the column, its property, its default width in characters, and whether it shows by default.

| # | Property | Width | Default | Win10 label (inferred) | XP label (from memory) |
|---|---|---|---|---|---|
| 0 | `System.ItemNameDisplay` | 30 | on | Name | Name |
| 1 | `System.ItemTypeText` | 20 | on | Type | Type |
| 2 | `System.ZipFolder.CompressedSize` | 16 | on | Compressed size | Packed Size |
| 3 | `System.ZipFolder.Encrypted` | 10 | on | Password protected | Has a Password |
| 4 | `System.ZipFolder.Method` | 10 | off | Method | (none) |
| 5 | `System.Size` | 16 | on | Size | Size |
| 6 | `System.ZipFolder.Ratio` | 10 | on | Ratio | Ratio |
| 7 | `System.DateModified` | 20 | on | Date modified | Date |
| 8 | `System.ZipFolder.CRC32` | 10 | off | CRC-32 | (none) |

**Observed** values (`CZipFolder::GetDetailsEx`):
- **Compressed size, Size, Ratio**: empty for folders.
- **Ratio** = `100 - compressed * 100 / size`, a whole percent. It is 0 when the size is 0 and when the result would leave 0..100 (a file that grew). Quirk: the zero test looks at the low 32 bits of the size only.
- **Password protected**: string 0x2760 for an encrypted entry (general-purpose flag bit 0), 0x2761 otherwise. Inferred: "Yes" and "No". Empty for folders.
- **Method**: a string per zip method number from a table: 0 = 0x271f, 1 = 0x2720, 2–5 = 0x2721, 6 = 0x2722, 7 = 0x2723, 8 = 0x2724, 9 = 0x27f6, 10 and above = 0x2725. Inferred from the zip spec numbering: Stored, Shrunk, Reduced, Imploded, Tokenized, Deflated, Deflate64, Unknown.
- **CRC-32**: formatted by `FUN_171203ba0` (presumably 8 hex digits, not checked).
- **Infotip and details pane**: `prop:System.ZipFolder.CompressedSize;System.Size;System.ZipFolder.Ratio;System.DateModified;System.ItemTypeText`.
- **File operation prompt** (conflict dialog lines): `prop:System.ItemTypeText;System.DateModified;System.Size`.

### 2.2 Sorting

**Observed** (`CZipFolder::CompareIDs`): folders always come before files, whichever column sorts. Then the column's values are compared (`VariantCompare`), and ties fall back to a second comparison (`FUN_1712190d4`, inferred to be by name). The canonical comparison (`SHCIDS_CANONICALONLY`) compares the full paths inside the archive.

### 2.3 What can be done with an item

**Observed** (`CZipFolder::GetAttributesOf`):
- Every item can be copied and has properties. In a writable archive, items can also be moved and deleted.
- No item can be renamed (`SFGAO_CANRENAME` is never set). `SetNameOf` exists but the shell never offers it.
- Folders inside are folders and drop targets, and say whether they have subfolders. Files are streams.
- The zip's hidden attribute is kept.
- The archive is read-only when a flag in the folder (`+0x46c`) is set. Then Delete and Cut leave the menu, and Move and Delete leave the attributes. Inferred: the flag is set when the zip file or its volume can't be written.

**Observed** (`CArchiveItemContextMenu::QueryContextMenu` and `InvokeCommand`, verbs at `0x171235800`):
- The item menu comes from menu resource 0x66, or 0x69 with Shift (extended verbs).
- Commands: Open (0x66), Explore (0x67, folders only), Copy (0x69), Delete (0x6f), Properties (0x70), Cut (0x71). Verb names: `delete`, `copy`, `cut`, `properties`.
- **The default item** is Open, except that a folder opened from a tree-style (Explore) context defaults to Explore.
- **Delete** checks that the volume has free space for the rewrite (`FUN_171217f48`: free bytes must exceed the zip's size). If not, it shows message 0x27d0. Otherwise it deletes, with confirmation unless the caller asked for no UI, then re-reads the archive.

### 2.4 Opening a folder inside

**Observed** (`_OpenExploreDirectory`): the folder is browsed in the same window, through the site's `IShellBrowser::BrowseObject` (open or explore mode). Without a browser it falls back to `ShellExecuteEx` with the verb `open` or `explore` on class `Folder`.

### 2.5 Opening a file inside

**Observed** (`CArchiveItemContextMenu::_OpenExploreItem`, `_OpenExploreTempFile`):

1. The wait cursor shows.
2. A temp folder for this archive is found or made (§2.6).
3. **Programs get a warning first.** This applies only when the archive holds 2 or more entries and the file's extension is `.exe`, `.bat` or `.com`. It shows dialog 0xb0, which has three outcomes:
   - Cancel: nothing happens.
   - Extract all (button id 0x46b): the Extract All wizard starts instead (§2.8).
   - Otherwise: the program is run alone.

   Inferred from XP: "This application might depend on other compressed files in this folder. For the application to run properly, it is recommended that you extract all files first."
4. **Setup programs get everything.** A file whose name without extension matches string 0x27c8 or 0x27c9 (inferred: `setup` and `install`) gets no warning. Instead every entry of the archive is extracted to the temp folder, then the file is run.
5. Otherwise only the one file is extracted (the password prompt may come up, §2.7). `/` in the stored path becomes `\`.
6. **The zip's origin is passed on.** The extracted file goes through Attachment Services (`IAttachmentExecute`: local path, the archive as the source, `Save`). So a file from a zip downloaded off the Internet is marked as from the Internet. If that check refuses the file, it is deleted.
7. The file is opened with its default verb (`SHInvokeDefault`-style, ordinal 540).
8. **Errors**: a cancel shows nothing. A too-long path shows message 0x28bc. Other failures show 0x28bd for files and 0x28be for folders, under caption 0x2747.

### 2.6 Temp files

**Observed** (`CTempFileNameArray::GetTempLocation`, `_TryCreatingInPath`, `ReferenceTempFile`, `DeleteFiles`, `CZipFolderViewCB::MessageSFVCB`):
- **Where**: under `%TEMP%`, one folder per archive, named by `%1_%2.%3\` from a fresh UUID string and the archive's file name. The mapping from archive to folder is kept, so later opens from the same archive reuse it. If the name exists it tries again, up to 99 times.
- **What gets tracked**: each extracted file, plus every folder between it and the temp root, is recorded for deletion. The exception is when the opening app's type is registered with the `ReadWriteTempFile` attribute: then the file is not recorded, so it stays.
- **When it's cleaned up**: each zip folder view counts itself when its window is created (`SFVM_WINDOWCREATED`, which also adds the zip to Recent Documents). When the **last** zip folder window is destroyed, a background thread deletes all recorded files and folders.
- Inferred: files still open in another app fail to delete and stay behind. XP warned that changes made to such files are lost, because nothing is written back into the zip.

### 2.7 Password prompt

**Observed** (`CPassword::GetDecryptPassword`, dialog 0x9a, `FUN_17120f400`):
- **Message**: the dialog names the file in the archive, its path shortened to fit (`PathCompactPath`) and inserted into a template from the dialog's own text.
- **Icon**: the stock key icon (`SIID_KEY`).
- **Password field**: at most 64 characters. OK stays disabled until something is typed.
- **Buttons**: OK, Cancel, and a third button (0x42e, inferred "Skip File") that skips this file and goes on. The third button is disabled when only one file is being extracted.
- **Remembered**: the password is held in a global (`g_password`). Inferred: it is tried again for the next encrypted entry in the same session.

### 2.8 Extract All wizard

**Observed** (`CZipWiz::RunWizard`, `_ExtractDlgProc`, `_ValidatePath`, `_PerformExtraction`):
- **Pages**: a property-sheet wizard with four pages: extract, nothing to extract, can't open, cancelled. It starts on "can't open" when the zip can't be read, and on "nothing to extract" when the zip has no files.
- **Destination**:
  - It defaults to the zip's own path with the extension removed, so `Downloads\foo.zip` extracts to `Downloads\foo`.
  - If the zip is in a temporary folder (`PathIsTemporaryW`, e.g. opened straight from a browser or mail), the default becomes `Documents\foo` instead.
  - The field autocompletes folders. Browse (0x3fd) picks one.
- **"Show extracted files when complete"** (0x44a) is remembered in the registry (`ExtractionWizard\ShowFiles`) and is on by default. When checked, the destination opens in a new window afterwards.
- **Checks on Extract**:
  - Whitespace is trimmed.
  - A path ending in `.` is invalid (0x28b8).
  - A relative path is refused (0x28b9).
  - A path that names an existing file is refused (0x28ba).
- **Extraction**:
  - A destination that doesn't exist is created (error 0x2775).
  - All items are copied with the shell's normal copy engine. That gives the usual progress dialog and the usual name-conflict questions.
  - If the copy fails and the wizard had created the folder, the folder is deleted silently.
  - A too-long path shows 0x28bb.

### 2.9 Index caching and change tracking

**Observed**:
- **Index cache** (`CZipFolder::BuildMainEnumerator`): the archive's index is cached globally, keyed by the zip's path and the later of its creation and write times. The zip is re-read when that time changes.
- **Change tracking** (`MessageSFVCB` `SFVM_GETNOTIFY`): a zip folder view registers for change notifications on the zip file itself, so it refreshes when the zip changes.
- **Ribbon** (`GetPaneState`): Win10 shows its ribbon by default here, for the "Compressed Folder Tools > Extract" tab. That has no Win2000 counterpart.

## 3. CAB folders (`cabview.dll`)

### 3.1 Flat list with a Path column

**Observed** (`CCabFolder::MapColumnToSCID`, table at `0x17120d160`, 5 entries, all shown by default):

| # | Property | Width | Label (from memory) |
|---|---|---|---|
| 0 | `System.ItemNameDisplay` | 20 | Name |
| 1 | `System.Size` | 16 | Size |
| 2 | `System.ItemTypeText` | 20 | Type |
| 3 | `System.DateModified` | 20 | Modified |
| 4 | `{0cd7a5c0-9f37-11ce-ae65-08002b2e1262}` 0 | 30 | Path |

**Observed**:
- **No subfolders**: a cabinet is shown as one flat list (`GetAttributesOf` allows only `SFGAO_CANCOPY`). Each entry's folder inside the cabinet goes in the Path column.
- **Dates** are stored as DOS local time and converted to UTC for display.
- **Sorting**: the canonical comparison is the name, without case.

**Observed: entries that are never listed** (`CCabFolder::EnumToList`, `FUN_171209dec`):
- absolute paths;
- paths that change when canonicalized, i.e. that contain `.` or `..` segments;
- paths with a reserved DOS device name in any segment (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with or without an extension).

This guards against entries that would extract outside the destination.

### 3.2 Item menu: Extract and Copy, no Open

**Observed** (`CCabItemMenu::QueryContextMenu`, `InvokeCommand`):
- **The menu** (resource 0x65) has Extract and Copy. The **default (double-click) is Extract**. A cabinet's files are never opened in place. With Shift, item 8 is removed from the menu.
- **Extract** shows a folder picker: `IFileOpenDialog` with "pick folders", "file system only" and "path must exist", and title and OK label from strings 0x45 and 0x46. The selected items are copied there with the shell's copy engine.
- **Copy** puts the items on the clipboard as a data object. Pasting extracts them.

### 3.3 Icon

**Observed** (`cabview.dll` resources, icon groups 1 and 105): Win10 has only the Vista-era art.
- **Group 1** is a gray filing cabinet with an open yellow drawer.
- **Group 105** is a blank page.
- The 16- and 256-color variants are dithered down from the 32-bit art, not hand-drawn.

The Windows 98/2000 CAB icon is not in this binary. It has to come from a screenshot.

## 4. Mapping to Macsplorer

What Macsplorer does (`macsplorer_core::archive`, `app.rs` `run_archive_job`, `extract_dialog.rs`, `password_dialog.rs`), and where it departs.

| Windows | Macsplorer |
|---|---|
| A zip is a folder in the same window (§2.4) | Show Package Contents in the item menu (or the address bar) opens `.zip`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`, `.tar.xz` and `.tar.zst` as folders. Inside, addresses look like `~/Downloads/foo.zip/src`. A path that runs through a file can't name a real item, so these paths are never ambiguous (`archive::split`). Double-click opens an archive with its program, as it does an app (user's choice, not from Windows, where it is the folder). `.7z`, `.rar` and `.cab` are recognized but have no folder view. **Not from Windows:** Finder's `__MACOSX` folder is left out, as Archive Utility does. |
| Columns (§2.1) | **Not done yet:** inside an archive the folder's own columns show (Name, Size, Modified, Attributes from the stored Unix mode). Packed Size, Password, Ratio, Method and CRC-32 are still to come. |
| Folders first (§2.2) | Already Macsplorer's rule. |
| No rename; read-only when the zip can't be written (§2.3) | Always read-only. Rename, Delete, Cut, Paste, Move Here and New Folder are hidden or greyed out, and refused with a message if reached by a key. Properties of items inside isn't offered either. Show in Finder shows the archive. |
| Open = extract to temp, then open (§2.5) | The same, into `~/Library/Caches/macsplorer/archives/<name>-<hash>/` (one folder per archive and version, reused), keeping the item's folders. An app bundle inside is extracted whole. An archive inside an archive is extracted and then opens with its program. **Not done:** the warning before running a program from a multi-file archive (macOS programs are mostly app bundles, which come out whole). |
| Mark of the Web (§2.5) | The archive's `com.apple.quarantine` attribute is copied onto everything extracted, as Archive Utility does, so Gatekeeper checks an app from a downloaded zip. |
| Temp cleanup on last window close (§2.6) | Whatever is older than a day goes when the app starts (`archive::clean_staging`), so files still open in their app are left alone. |
| Password prompt (§2.7) | Password Needed, with OK disabled until something is typed, asked when a job finds encrypted items and no password, and again after a wrong one (AES is checked exactly; for ZipCrypto the CRC catches what its 1-in-256 check lets through). The password is kept per archive while the window is open. **Not done:** Skip File. |
| Extract All wizard (§2.8) | One page in Windows 7's wording. The default destination is the archive's path without its extension (all of `.tar.gz`), or `~/Documents/<name>` when the archive is in a temporary folder. The checks are the wizard's: a full path, not an existing file, and not inside an archive. "Show extracted files when complete" is remembered in `settings.json` (`extract_show`) and goes to the folder in the same window, where Windows opens a new one. The items are extracted to a scratch folder and moved into place with `fileops::transfer_with`, so the progress dialog, Confirm File Replace and Undo work as for a paste. A destination folder made for nothing is removed again. **Not done:** Browse.... |
| Copy and drag out (`CArchiveItemDataObject`) | Copy extracts to a scratch folder and puts those files on the pasteboard, so Finder can paste them too. A drop on a folder in the window extracts and copies, always a copy. A drag that leaves the window extracts at once, up to 64 MB, and hands AppKit the files. |
| Index cache keyed by path and time (§2.9) | Cached by path, size and modification time, eight archives at a time. The folder holding the archive is watched, so a changed archive reloads. |
| CAB: flat list with Path, Extract as default, unsafe names skipped (§3) | The unsafe-name rule applies to every format: absolute paths are made relative, and names with `..` are never listed or extracted. **Not done:** CAB itself. |
| CAB and zip icons (§3.3) | Zip and tar files, which open as folders, have a zipper: the yellow folder with a zipper down its front when large (`theme::Icon::CompressedFolder`, after XP's Compressed (zipped) Folder icon, from memory), the paper page with one in the List and Details views (`Icon::CompressedFile`, not from Windows). The address bar shows the zipper folder anywhere inside an archive, and the archive's name in its red; Win10's zipfldr.dll art wasn't used. Other archives keep their page icon, so the icon says what opens in Macsplorer. A cabinet icon drawn from memory was tried and dropped. |
| `.app` | Show Package Contents (`file-browser-findings.md` §3). Bundles are real folders and need none of the above. |

## 5. UI text, from memory

The `.mui` files aren't available, so this wording is **from memory** of XP and Windows 7, not read from the binaries. The string IDs are what the code loads.

| Where | ID | Text (from memory) |
|---|---|---|
| Password protected column | 0x2760 / 0x2761 | Yes / No |
| Method column | 0x271f–0x2725, 0x27f6 | Stored, Shrunk, Reduced, Imploded, Tokenized, Deflated, Unknown, Deflate64 |
| Run a program from the zip (dialog 0xb0) | | Caption "Compressed (zipped) Folders". "This application might depend on other compressed files in this folder. For the application to run properly, it is recommended that you first extract all files." Buttons **Extract all**, **Run**, **Cancel**. |
| Password dialog (0x9a) | | Caption "Password needed". "The file '%1' is password protected. Please enter the password below." **Password:** field. Buttons **OK**, **Skip File**, **Cancel**. |
| Extract All, XP wizard | | "Welcome to the Compressed (zipped) Folders Extraction Wizard". "Select a Destination": "Files will be extracted to this directory:" with **Browse...**. "Extraction Complete": "Files have been successfully extracted from the compressed (zipped) folder." with **Show extracted files**. |
| Extract All, Windows 7 single page | | Caption "Extract Compressed (Zipped) Folders". "Select a Destination and Extract Files". "Files will be extracted to this folder:" with **Browse...**. **Show extracted files when complete**. Buttons **Extract**, **Cancel**. |
| Nothing to extract (wizard page) | | "The compressed (zipped) folder is empty." |
| Can't open (wizard page) | | "Windows cannot open the folder. The Compressed (zipped) Folder '%1' is invalid." |
| Bad destination | 0x28b8–0x28ba | "The specified destination path is invalid." / "...must be a full path." / "A file with that name already exists." (the last two are paraphrased) |
| CAB Extract folder picker | 0x45 / 0x46 | Title "Select a destination", OK button **Extract** |
| CAB item menu | menu 0x65 | **Extract**, **Copy** |

For Macsplorer: use Windows 7's one-page Extract wording, which is simpler than XP's wizard, and Win2000's capitalization for column headers (**Packed Size**, **Password**, **Ratio**, **Method**, **CRC-32**).

## 6. Not traced

- `CZipFolder::EnumObjects` and the archive reader. Unknown: whether the zip reader filters `..` and absolute entries like cabview does, and how stored names in other code pages are decoded (the general-purpose UTF-8 flag, bit 11).
- Writing into a zip: drag in, paste, Send To > Compressed (zipped) folder (`CArchiveDropTarget`, `PasteToZipFolder`, `CSendTo`). Left out of the Macsplorer design.
- Whether the Folders pane lists zip files as folders. That would be `SFGAO_FOLDER` on the `.zip` item from shell32's file system folder with zipfldr's handler registration, not this DLL. XP did list them.
- All UI text, until `zipfldr.dll.mui` and `cabview.dll.mui` are available.
