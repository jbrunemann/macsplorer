# Macsplorer User Manual

Macsplorer is a file browser for your Mac. It looks and works like the Windows 2000 Explorer: a folder tree on the left, your files on the right, and the full path of where you are along the top. You can use it next to Finder or instead of it. Your files open in the same apps as always.

<img src="images/window.png" alt="The Macsplorer window" width="900">

**Contents**

1. [Getting started](#1-getting-started)
2. [The window at a glance](#2-the-window-at-a-glance)
3. [Getting around](#3-getting-around)
4. [Choosing how files are shown](#4-choosing-how-files-are-shown)
5. [Selecting files](#5-selecting-files)
6. [Opening files](#6-opening-files)
7. [Renaming](#7-renaming)
8. [Copying, moving and deleting](#8-copying-moving-and-deleting)
9. [Undo](#9-undo)
10. [Properties](#10-properties)
11. [Finding files](#11-finding-files)
12. [Making it your own](#12-making-it-your-own)
13. [Keyboard shortcuts](#13-keyboard-shortcuts)
14. [Questions and answers](#14-questions-and-answers)

---

## 1. Getting started

**Installing.** Unzip the download and drag **Macsplorer** into your **Applications** folder.

**The first launch.** Your Mac may warn that it can't check Macsplorer for harmful software, because the app isn't registered with Apple. To open it anyway, right-click (or Control-click) Macsplorer in Applications, choose **Open**, then confirm with **Open**. You only need to do this once.

**Where it starts.** Macsplorer opens in your home folder, the one with your name on it.

**More windows.** Choose **File › New Window** (⌘E, or ⌘N) to open a second window, for example to copy files from one folder to another. Each new window opens on the folder you're in and appears slightly offset from the one before.

**Closing.** Click the **×** in the top right corner, choose **File › Close**, or press ⌘Q.

## 2. The window at a glance

From top to bottom:

| Part | What it does |
|---|---|
| **Title bar** | Shows the name of the folder you're in. Drag it to move the window. The buttons on the right minimize, maximize and close the window, and double-clicking the bar also maximizes it. |
| **Menu bar** | File, Edit, View and Help. |
| **Toolbar** | Back, Forward, Up, Folders, Refresh, Undo and Views. Hold the pointer over a button to see what it does and its shortcut. |
| **Address bar** | The path to the current folder. You can click any part of it. |
| **Search box** | To the right of the address bar. Shows only the items whose name contains what you type. |
| **Folders pane** | On the left: your favorite places, your disks, and all the folders on them as a tree. |
| **File list** | On the right: the contents of the current folder. |
| **Status bar** | At the bottom: how many items there are, how big they are, how much free space the disk has, and your computer's name. Inside a Git repository it also shows the branch and the state of the repository (see [Git repositories](#git-repositories)). |

### Git repositories

While you are anywhere inside a Git repository, the status bar gets an extra pane with the branch symbol and a summary in the format of [posh-git](https://github.com/dahlbyk/posh-git#git-status-summary-information):

`[main ≡ +0 ~2 -1 | +1 ~1 -0 !]`

<img src="images/git-status.png" alt="The status bar inside a Git repository" width="900">

- The branch name comes first, or `(abc1234...)` when you are not on a branch. While a merge or rebase is under way it says so, for example `main|MERGING`.
- Next to the branch: `≡` means it matches its remote, `↑3` that you are 3 commits ahead (time to push), `↓2` that you are 2 behind (time to pull), `↓2 ↑3` both, and `×` that its remote branch is gone. Nothing means it has no remote.
- Before the `|` are the staged changes (added `+`, modified `~`, removed `-`, conflicted `!`), after it the ones that are not staged yet. Files Git doesn't know yet count as added.
- A final `!` says there are changes that are not staged, `~` that everything that changed is staged.

To see which folders are Git repositories, turn on the **Branch** column (right-click a column heading, see [Details](#details)). It shows the branch of every folder that is a repository, and nothing for the rest.

<img src="images/git-branch-column.png" alt="The Branch column" width="900">

The summary and the **Branch** column need Git to be installed. Without it they are simply left out. The summary is worked out when you enter a repository and kept while you move around inside it, so big repositories stay quick. It is worked out again with **View › Refresh**, after you change something in Macsplorer, and when another program changes the folder you're looking at. Changes deeper inside the repository show up at the next refresh.

## 3. Getting around

### The Folders pane

The tree on the left has two sections:

- **Favorites**: Desktop, Downloads, Documents, Pictures, Music, Movies, your home folder, Applications and the Trash.
- **Locations**: your Mac's disk, plus any USB sticks, external drives or network disks you have connected.

Click a folder's name to open it. Click the small **+** box (or double-click the name) to unfold the folders inside it without leaving where you are, and **−** to fold them away again. With the keyboard, the arrow keys move up and down, → unfolds and ← folds, + and − do the same without moving, Home and End jump to the first and last folder, and Page Up and Page Down move a pageful at a time. Typing the first letters of a folder's name jumps to it, among the folders you can see. When you move with the keys, the folder opens on the right half a second after you stop, so you can run through the tree quickly without waiting for every folder on the way.

As you move around, the tree follows you: it unfolds to and highlights the folder you're in. To turn that off, uncheck **View › Expand to open folder**.

Right-click a folder in the tree for **Open**, **Properties**, and **Eject** for disks you can safely remove.

To hide the pane and give the files more room, click **Folders** on the toolbar or the **×** at the top of the pane. Click **Folders** again to bring it back.

### The address bar

The address bar shows each folder on the way to where you are. Click any of them to jump there.

The small **›** after each folder opens a list of the folders inside it, so you can go sideways or deeper in one click. When the path is too long to fit, the first part is folded into a **«** at the start. Click it to see the rest.

<img src="images/chevron-menu.png" alt="The list of subfolders under the › in the address bar" width="500">

To type a path instead, click the empty space in the address bar or press ⌘L (F4 and ⌥D work too), type the location and press Enter (or click **Go**). A **~** at the start stands for your home folder, so `~/Documents` takes you to your Documents folder. Press Esc to stop typing and go back to the clickable path.

### Back, Forward and Up

- **Back** returns to the folder you were in before, and **Forward** goes ahead again after going back. The small arrow beside each one shows a list of the places you've been, so you can jump several steps at once.
- **Up** goes to the folder that contains the current one.
- **Refresh** reloads the list and the Folders pane. You rarely need it: when another program adds, removes, renames or changes files in the folder you're looking at, or in a folder that is open in the Folders pane, Macsplorer updates by itself within a second.

## 4. Choosing how files are shown

Pick a view under **View**, or with the **Views** button at the right of the toolbar.

<img src="images/views-menu.png" alt="The Views button and its menu" width="700">

### Details

A table with one row per item and columns for its name, size, the date it was last changed and its attributes. This is the view the app starts in.

- **Sort** by clicking a column heading. Click it again to reverse the order. The small triangle shows which column is sorted and in which direction.
- **Resize a column** by dragging the line between two headings. Double-click the line to make the column just wide enough for its contents. Press Esc during the drag to cancel.

**Choosing columns.** Right-click any column heading to turn columns on or off.

<img src="images/header-menu.png" alt="The column heading menu" width="900">

The available columns are:

| Column | Shows |
|---|---|
| **Name** | The item's name. This one is always shown. |
| **Size** | How big a file is. Folders are left blank. |
| **Type** | What kind of file it is, for example "Plain Text Document". |
| **Modified** | When it was last changed. |
| **Attributes** | Who may read, change or run it (see [Properties](#10-properties)). |
| **Comment** | The comment you gave it in Finder's Get Info window. |
| **Created** | When it was made. |
| **Accessed** | When it was last opened. |
| **Owner**, **Group** | Which user and group it belongs to. |
| **Branch** | The Git branch of a folder that is a Git repository, with the branch symbol in front. Blank for everything else. |

Choose **More...** at the bottom of that menu for the **Column Settings** window. There you can check and uncheck columns, put them in a different order with **Move Up** and **Move Down**, and type an exact width for the selected column.

<img src="images/column-settings.png" alt="The Column Settings window" width="352">

Your columns, their order and their widths are the same in every folder and every window, and Macsplorer remembers them for next time.

### Large Icons

Big icons laid out in rows, with the names underneath. Long names are cut short after two lines. Click one to see its full name.

<img src="images/large-icons.png" alt="Large Icons view" width="900">

### List

Small icons with names, arranged in columns that run from top to bottom. When there are more items than fit, the list scrolls sideways.

<img src="images/list.png" alt="List view" width="900">

### What the icons tell you

- **Yellow folders** are folders. The ones in your Favorites carry a small symbol, the same one Finder uses for them.
- **Paper pages** are files. A colored mark on the page shows the kind of file, for example a spreadsheet, a picture, a PDF or a program's source code. A plain page with gray lines is a file of a kind Macsplorer doesn't know.
- **Apps**, and anything you have given a custom icon in Finder, show their real icon.

### Colored names

With **View › Special files in color** turned on (it is at first), some names are colored so they stand out:

- **Green**: programs and scripts that can be run.
- **Turquoise**: links (aliases) that point to another file or folder.
- **Red**: archives such as `.zip` files and disk images, and links whose original has gone missing.

## 5. Selecting files

- **Click** an item to select it.
- **⌘-click** to add an item to the selection, or to take it out again.
- **⇧-click** to select everything between the item you selected first and this one.
- **Drag** from an empty spot to draw a dotted rectangle: everything it touches gets selected. In Details you can start beside a name. Hold ⌘ while dragging to switch items on and off, or ⇧ to add to what's already selected. If you drag past the edge, the list scrolls along.
- **Edit › Select All** (⌘A) selects everything in the folder, and **Edit › Invert Selection** swaps what's selected and what isn't.
- **Arrow keys** move the selection. In Large Icons they move to the nearest icon in that direction. Hold ⇧ to select everything from the first item to where you move.
- **Space** adds the item with the dotted focus rectangle to the selection, or takes it out again. ⇧-Space selects everything from the first item to it.
- **Type the first letters** of a name to jump to it. Typing `re` quickly selects the first item starting with "re". Pause for a second to start over.

The status bar shows how many items you have selected and how much space they take up together.

## 6. Opening files

- **Double-click** an item, or select it and press Enter. Folders open in Macsplorer, and files open in the app your Mac normally uses for them.
- **Right-click** an item for more choices:

<img src="images/item-menu.png" alt="The menu for a file" width="900">

| Choice | What it does |
|---|---|
| **Open** | Same as a double-click. |
| **Show Package Contents** | For apps and other packages: shows the files inside them as a folder instead of opening them. **Back** returns to where you were. |
| **Open With** | Pick another app to open the file with. The apps that can open it are listed with their icons. **Choose Program...** opens a list where you pick an app, or use **Other...** for any app on your Mac. Tick **Always use this program to open these files** and double-clicking that kind of file (all `.jpg` and `.jpeg` files, say) opens it with that app from then on, in Macsplorer only. Finder isn't affected. |
| **Show in Finder** | Opens a Finder window with the item selected. |
| **Open in Terminal** | For folders: opens a Terminal window in that folder. Also in the Folders pane's menu, and in the menu for empty space (for the open folder). |
| **Eject** | For disks and drives: safely disconnects them. |
| **Cut**, **Copy**, **Copy as Path**, **Paste**, **Move Here** | See [Copying, moving and deleting](#8-copying-moving-and-deleting). |
| **Delete** | Moves the item to the Trash. |
| **Rename** | See [Renaming](#7-renaming). |
| **Properties** | See [Properties](#10-properties). |

<img src="images/open-with-menu.png" alt="The Open With list" width="900">

Right-click an empty spot in the list for **Paste**, **Move Here**, **Refresh**, and **Properties** of the folder you're in.

## 7. Renaming

Start renaming in any of these ways:

- select the item and press **F2**,
- right-click it and choose **Rename**,
- or click the name of an item that's already selected, then wait a moment.

The name turns into a text box. Only the part before the extension is selected, so you can type a new name straight away without losing the `.txt` or `.pdf` at the end.

<img src="images/rename.png" alt="Renaming a file in place" width="900">

Press **Enter** or click somewhere else to keep the new name. Press **Esc** to cancel. The `/` character can't be used in a name. If an item with that name already exists, nothing is changed and the status bar tells you why.

## 8. Copying, moving and deleting

### Copy and Paste

1. Select the items and choose **Edit › Copy** (⌘C).
2. Go to the folder where you want them.
3. Choose **Edit › Paste** (⌘V).

You can also right-click a folder in the list and choose **Paste** to put the items straight into it.

If you paste into the folder the items came from, each copy is called "Name - Copy", then "Name - Copy (2)" and so on.

### When the name is already taken

If the folder you paste into already has an item with the same name, Macsplorer asks what to do:

<img src="images/confirm-file-replace.png" alt="The Confirm File Replace question" width="430">

- **Yes** replaces the item that's there. It isn't lost: it goes to the Trash, and ⌘Z brings it back.
- **Yes to All** replaces this one and every other item with a name that's taken, without asking again.
- **No** leaves both alone and goes on with the next item.
- **Cancel** stops. What was already copied or moved stays where it is.

When a folder goes into a folder of the same name, the two are merged: the contents end up together in one folder. If none of the names inside clash, this happens without a question. If some do, Macsplorer first asks whether to merge the folders, then asks about each file whose name is taken.

### Watching it work

When copying or moving takes more than a second, a window shows how it's going: the item it's working on, where it goes, a bar, and about how long is left.

<img src="images/copy-progress.png" alt="The Copying window" width="380">

**Cancel** (or Esc) stops it. What was already copied or moved stays where it is, and the item that was halfway is left out completely: no half-copied files. If that item was replacing one with the same name, the old one is put back.

### Cut and Paste (moving)

Use **Edit › Cut** (⌘X) instead of Copy to move the items. They are shown faded until you paste them somewhere. If the destination already has an item with the same name, Macsplorer asks first, as described above.

### Dragging

You can also move items by dragging them with the mouse. Press on an item (or on one of several selected items), hold the button and move: the items follow the pointer, faded. A folder you move over is highlighted, and letting go puts the items into it. Letting go over empty space in the list puts them into the folder you're looking at. Folders in the Folders pane take items too, and a closed one opens if you hold the pointer over it for a moment.

<img src="images/drag-items.png" alt="Two files dragged onto the Pictures folder" width="560">

What happens depends on where the items go, as in Finder:

- On the same disk they are **moved**.
- To another disk (a USB stick, a network drive) they are **copied**.
- Hold **⌥ Option** while letting go to copy instead, or **⌘ Command** to move.
- Hold **⌥ Option** and **⌘ Command** together to make a **shortcut** instead: a small file that opens the original. It's called "Name - Shortcut".

The pointer shows what will happen: a plus sign for a copy, a little arrow for a shortcut, and a "no" sign where the items can't go, such as onto a file or into the folder they came from. Press **Esc** while dragging to call it off. A drag into a Macsplorer window can be undone with ⌘Z like any other move or copy.

Drag items out of the window to put them in a Finder window, on the desktop, into another Macsplorer window or another app, or onto the Trash in the Dock. Items dragged from Finder can be dropped on Macsplorer in the same way.

### Between windows and Finder

Copy and Cut use the Mac's own clipboard, so you can copy in one Macsplorer window and paste in another, or copy in Finder and paste in Macsplorer, and the other way round.

Finder has no Cut. Items you cut in Macsplorer are pasted as copies in Finder; to move them there, use **Edit › Move Item Here** (⌥⌘V) in Finder. The same goes for Macsplorer: items copied (or cut with a helper such as Command X) in Finder are pasted here as copies, and **Edit › Move Here** (⌥⌘V) moves them instead. It is also in the right-click menus.

### Copy as Path

**Edit › Copy as Path** (⌘⌥C) copies the full location of the selected items as text, for pasting into an email, a document or a chat.

### Delete

Select the items and press the forward **Delete** key (⌦), press ⌘⌫, or choose **Edit › Delete**. They go to the Trash, just as in Finder, so you can get them back with Undo or from the **Trash** in the Folders pane.

macOS keeps the Trash private. The first time you open it, Macsplorer may say "Operation not permitted". To allow it, open **System Settings › Privacy & Security › Full Disk Access**, switch on Macsplorer (or the terminal you started it from), and start Macsplorer again.

## 9. Undo

Made a mistake? Click **Undo** on the toolbar, choose **Edit › Undo**, or press ⌘Z. The menu says what will be undone, for example "Undo Rename".

Undo works for renaming, moving, copying and deleting, and goes back up to 10 steps. Undoing a delete brings the items back out of the Trash.

## 10. Properties

To see the details of an item, select it and press ⌘I, choose **File › Properties**, or right-click it and choose **Properties**. With nothing selected, you get the properties of the folder you're in.

<img src="images/properties-file.png" alt="Properties of a file" width="380"> <img src="images/properties-folder.png" alt="Properties of a folder" width="380">

The window shows:

- **The name**, in a box you can type in to rename the item.
- **The type**: what kind of file it is.
- **Location**: the folder it is in.
- **Size**: for a folder, the total of everything inside it, plus how many files and folders it **Contains**. Big folders take a moment to count. The numbers go up while it counts.
- **Created, Modified, Accessed**: when the item was made, last changed and last opened.
- **Permissions**: who may **Read** the item, **Write** (change) it, and **Execute** it (run it as a program). "Owner" is you, "Group" is a group of users on your Mac, and "Others" is everyone else. Check or uncheck the boxes to change them.

Click **OK** to save your changes and close the window, **Apply** to save them and keep it open, or **Cancel** to close without saving.

If you select several items first, the window shows them together: their combined size and how many files and folders they contain.

<img src="images/properties-selection.png" alt="Properties of three selected items" width="380">

## 11. Finding files

Type in the **Search this folder** box at the top right. The list shows only the items in the current folder whose names contain what you typed, upper or lower case doesn't matter. Clear the box to see everything again.

<img src="images/search.png" alt="The search box showing only the items that contain 're'" width="900">

⌘F or F3 takes you to the search box from anywhere in the window.

To jump to an item without hiding the others, just type the first letters of its name in the list (see [Selecting files](#5-selecting-files)).

## 12. Making it your own

All of these are in the **View** menu.

<img src="images/view-menu.png" alt="The View menu" width="500">

| Option | What it does |
|---|---|
| **Explorer Bar › Folders** | Shows or hides the Folders pane. |
| **Large Icons, List, Details** | Picks a view (see [Choosing how files are shown](#4-choosing-how-files-are-shown)). |
| **Text Size** | Makes everything bigger (see below). |
| **File name extensions** | Shows the ending of file names, such as `.pdf` or `.jpg`. When you hide them, an ending is still shown where hiding it could be misleading, such as an app named `Invoice.pdf.app`. |
| **Hidden files and folders** | Shows the items your Mac normally hides, such as settings files whose names start with a dot. They appear faded so you can tell them apart. |
| **Special files in color** | Turns the [colored names](#colored-names) on or off. |
| **Dark mode** | Light text on a dark background. |
| **Expand to open folder** | Lets the Folders pane follow you to the folder you're in. |
| **Natural number sorting** | Sorts numbers in names by their value, so "File 2" comes before "File 10". Turn it off to sort letter by letter. |
| **Mix folders and files** | Sorts folders in between the files instead of putting them all at the top. |
| **Refresh** | Reloads the list. |

<img src="images/hidden-items.png" alt="Hidden files shown faded" width="900">

### Text Size

If the text is too small to read comfortably, choose **View › Text Size** and pick **Large (125%)**, **Larger (150%)** or **Largest (200%)**. Everything grows together, the text as well as the icons, buttons and spacing, so the window keeps its look. **Normal (100%)** sets it back.

<img src="images/view-menu-text-size.png" alt="The Text Size menu" width="500">

You can also press **⌘+** to go one size up, **⌘−** to go one size down, and **⌘0** to return to Normal. You may want to make the window bigger after choosing a larger size.

<img src="images/text-size-larger.png" alt="Macsplorer at 150% text size" width="900">

### Using another terminal

**Open in Terminal** starts the Terminal app. To use another one, add its app name to `~/Library/Application Support/macsplorer/settings.json`, for example `"terminal": "iTerm"` or `"terminal": "Ghostty"`. The name is the one shown in the Applications folder, without `.app`. **Help › Open config file** opens that file in your text editor. The next **Open in Terminal** uses it, no restart needed. If the name is wrong, the status bar says so.

### Choosing which app opens which files

The **Open With** dialog remembers one app per kind of file (see chapter 6). You can also set several at once by editing `~/Library/Application Support/macsplorer/settings.json` (**Help › Open config file** opens it). Add an `associations` list that matches a file type to an app. For example, to open all the common picture formats in Preview:

```json
"associations": {
  "public.jpeg": "/System/Applications/Preview.app",
  "public.png": "/System/Applications/Preview.app",
  "com.compuserve.gif": "/System/Applications/Preview.app",
  "public.tiff": "/System/Applications/Preview.app",
  "com.microsoft.bmp": "/System/Applications/Preview.app",
  "public.heic": "/System/Applications/Preview.app",
  "org.webmproject.webp": "/System/Applications/Preview.app",
  "public.svg-image": "/System/Applications/Preview.app"
}
```

Each line is a file type, then the app's full path. File types are the names macOS uses for them, so `.jpg` and `.jpeg` files are both `public.jpeg`. Each type needs its own line. To find the name for another kind of file, run `mdls -raw -name kMDItemContentType` followed by a file's path in Terminal, or tick **Always use this program** once in the Open With dialog and look at the line it adds.

Keep the commas between lines and put the list inside the outer `{ }`. A mistake in the file makes Macsplorer ignore all your settings until it is fixed. A change counts the next time you open a file, no restart needed. If the app is no longer installed, the file opens the normal way.

### Dark mode

<img src="images/window-dark.png" alt="Macsplorer in dark mode" width="900">

Macsplorer remembers your text size, dark mode, columns, view, sort order and window size the next time you open it. Windows that are already open pick up a change the next time they're opened.

## 13. Keyboard shortcuts

You can use Macsplorer without the mouse. **Tab** moves from the file list to the address bar, then to the search box, then to the Folders pane and back to the list; **⇧Tab** goes the other way. F6 does the same as Tab. The part of the window you're in shows a dotted rectangle around the item that the keys work on, and the selection in the other parts turns gray.

**Moving around**

| Keys | Action |
|---|---|
| ⌘[ or ⌥← or ⌫ | Back |
| ⌘] or ⌥→ | Forward |
| ⌘↑ or ⌥↑ | Up to the containing folder |
| Tab or F6 | Go to the next part of the window (⇧ goes back) |
| ⌘L or F4 or ⌥D | Type a location in the address bar |
| ⌘F or F3 | Search this folder |
| ⌘R or F5 | Refresh |
| Arrow keys | Move the selection (in the tree: → unfolds, ← folds) |
| Home, End, Page Up, Page Down | First item, last item, a pageful up or down |
| Space | Select the item with the focus rectangle, or take it out of the selection |
| ⇧ with arrow keys or Space | Select a range |
| + and − | In the Folders pane: unfold and fold |
| Letters | Jump to the first item whose name starts with them |

**Working with files**

| Keys | Action |
|---|---|
| Enter | Open the selected item |
| F2 | Rename |
| ⌘A | Select all |
| ⌘C | Copy |
| ⌘X | Cut |
| ⌘V | Paste |
| ⌥⌘V | Move Here (paste as a move) |
| ⌘⌥C | Copy as Path |
| ⌦ or ⌘⌫ | Move to Trash |
| ⌘Z | Undo |
| ⌘I | Properties |

**Window**

| Keys | Action |
|---|---|
| ⌘E (or ⌘N) | New window |
| ⌘+ / ⌘− / ⌘0 | Bigger text, smaller text, normal text |
| ⌘Q | Close |

On Linux, use Ctrl wherever this manual says ⌘.

## 14. Questions and answers

**I added a file in another app. Do I need to refresh?**
No. Macsplorer notices changes in the folder you're looking at and in the folders open in the Folders pane, and updates within a second. **Refresh** (⌘R) is still there for everything else.

**Can I drag files to move them?**
Yes, see [Dragging](#dragging). On the same disk dragging moves; hold ⌥ Option to copy instead.

**What happens if I paste a file where one with the same name already exists?**
Macsplorer asks whether to replace it. If you say yes, the old one goes to the Trash, so ⌘Z can bring it back. See [When the name is already taken](#when-the-name-is-already-taken).

**I deleted something by mistake.**
Press ⌘Z right away, or open the **Trash** in the Folders pane.

**Some of my files are missing from the list.**
Check that the search box is empty, then turn on **View › Hidden files and folders** to see the items your Mac normally hides.

**The Git summary or the Branch column is missing.**
Both need Git to be installed on your Mac. Macsplorer looks for it when it starts, so if you install Git while Macsplorer is open, close and reopen it. If you have Git and they still don't show, [Git repositories](#git-repositories) explains when they appear. After changes made by other programs, press ⌘R to bring the summary up to date.

**Which version do I have?**
Choose **Help › About Macsplorer**. A small window shows the app icon and the version. Press OK, Return or Escape to close it.
