//! The General page of an item's Properties: what it shows and how it is
//! worded, after shell32's `CFilePropSheetPage` (see
//! `docs/file-browser-findings.md`, "Properties").

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::SystemTime;

use chrono::NaiveDate;

use crate::dirscan::{scan, Scratch};
use crate::listing::{format_byte_size, group_digits};

/// One item's facts for the General page, read off the UI thread.
#[derive(Clone, Debug)]
pub struct ItemInfo {
    pub is_dir: bool,
    /// File size in bytes; folders are measured by [`FolderSize`].
    pub size: u64,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    /// On a FAT or exFAT volume, which keeps access dates without a time.
    pub on_fat: bool,
    /// `st_mode` of the item itself, or of its target for a symlink (what a
    /// `chmod` on the link would change).
    pub mode: u32,
    /// Where the item points, if it is a symlink.
    pub link: Option<LinkInfo>,
}

/// A symlink's target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInfo {
    /// What the link holds, as `readlink` returns it: possibly relative.
    pub target: PathBuf,
    /// Nothing is there.
    pub broken: bool,
}

/// Reads `path`'s metadata, following a symlink like the listing does.
pub fn item_info(path: &Path) -> io::Result<ItemInfo> {
    let target = fs::metadata(path);
    let link = fs::read_link(path).ok().map(|to| LinkInfo { target: to, broken: target.is_err() });
    let meta = target.or_else(|_| fs::symlink_metadata(path))?;
    Ok(ItemInfo {
        is_dir: meta.is_dir(),
        size: meta.len(),
        created: meta.created().ok(),
        modified: meta.modified().ok(),
        accessed: meta.accessed().ok(),
        on_fat: is_fat_volume(path),
        mode: meta.mode(),
        link,
    })
}

/// The Target line: the link's text, with a note when nothing is there.
pub fn format_target(link: &LinkInfo) -> String {
    let target = link.target.display();
    if link.broken { format!("{target} (not found)") } else { target.to_string() }
}

/// The nine permission bits as check boxes: rows are owner, group and
/// others, columns read, write and execute.
pub fn permissions(mode: u32) -> [[bool; 3]; 3] {
    [6, 3, 0].map(|shift| [4, 2, 1].map(|bit| mode >> shift & bit != 0))
}

/// `mode` with its nine permission bits replaced by `bits` (laid out as
/// [`permissions`] returns them). The file type and the special bits stay.
pub fn with_permissions(mode: u32, bits: [[bool; 3]; 3]) -> u32 {
    let mut out = mode & !0o777;
    for (row, shift) in bits.into_iter().zip([6, 3, 0]) {
        for (on, bit) in row.into_iter().zip([4, 2, 1]) {
            if on {
                out |= bit << shift;
            }
        }
    }
    out
}

/// Sets `path`'s permission bits to `bits`, keeping the special bits. A
/// symlink's target is changed, like `chmod`, which is also what the
/// dialog shows.
pub fn set_permissions(path: &Path, bits: [[bool; 3]; 3]) -> io::Result<()> {
    let mode = with_permissions(fs::metadata(path)?.mode(), bits) & 0o7777;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// One item of a multiple selection, as far as the size walk cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectedItem {
    /// A file and its size.
    File(u64),
    /// A folder or package, to be walked.
    Folder(PathBuf),
}

/// Running totals of a folder walk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FolderTotals {
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
}

/// A folder's size and contents, counted on a worker thread and read by the
/// UI while it grows, like `CFolderContentsInfo::SizeThreadProc` with its
/// `CFolderSizeTreeWalkerCB`. Explorer reads the totals on a 250 ms timer
/// and once more when the walk ends.
#[derive(Debug, Default)]
pub struct FolderSize {
    bytes: AtomicU64,
    files: AtomicU64,
    folders: AtomicU64,
    done: AtomicBool,
    cancelled: AtomicBool,
}

impl FolderSize {
    pub fn totals(&self) -> FolderTotals {
        FolderTotals {
            bytes: self.bytes.load(Ordering::Relaxed),
            files: self.files.load(Ordering::Relaxed),
            folders: self.folders.load(Ordering::Relaxed),
        }
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    /// Stops the walk at the next item (the dialog was closed).
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Walks everything below `dir`: hidden items count, the folder itself
    /// doesn't, symlinks count as files and aren't followed (a guess; the
    /// tree walker's flags weren't traced), unreadable folders are skipped.
    /// Several threads walk at once, which Explorer doesn't do; the walk is
    /// mostly file system latency; more than 8 to 12 threads stopped helping.
    pub fn measure(&self, dir: &Path) {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(4, 16);
        self.measure_with_threads(dir, threads);
    }

    /// [`measure`](Self::measure) with a given number of threads.
    pub fn measure_with_threads(&self, dir: &Path, threads: usize) {
        self.walk(dir, threads);
        self.done.store(true, Ordering::Release);
    }

    /// Measures a selection of several items, like `SizeThreadProc` going
    /// through the selected items: files add their size, and each selected
    /// folder counts as a folder and is walked.
    pub fn measure_selection(&self, items: &[SelectedItem]) {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(4, 16);
        for item in items {
            if self.cancelled.load(Ordering::Relaxed) {
                break;
            }
            match item {
                SelectedItem::File(bytes) => {
                    self.bytes.fetch_add(*bytes, Ordering::Relaxed);
                    self.files.fetch_add(1, Ordering::Relaxed);
                }
                SelectedItem::Folder(dir) => {
                    self.folders.fetch_add(1, Ordering::Relaxed);
                    self.walk(dir, threads);
                }
            }
        }
        self.done.store(true, Ordering::Release);
    }

    fn walk(&self, dir: &Path, threads: usize) {
        let queue = Queue { state: Mutex::new((vec![dir.to_path_buf()], 1)), wake: Condvar::new() };
        std::thread::scope(|scope| {
            for _ in 0..threads.max(1) {
                scope.spawn(|| self.work(&queue));
            }
        });
    }

    /// One walker thread: takes folders off the queue until none are left
    /// or queued and every other thread is out of work too.
    fn work(&self, queue: &Queue) {
        let mut scratch = Scratch::default();
        let mut subfolders = Vec::new();
        while let Some(dir) = queue.take() {
            if !self.cancelled.load(Ordering::Relaxed) {
                let (mut bytes, mut files) = (0, 0);
                let _ = scan(&dir, &mut scratch, |item| {
                    if item.is_dir {
                        subfolders.push(dir.join(item.name));
                    } else {
                        bytes += item.len;
                        files += 1;
                    }
                });
                self.bytes.fetch_add(bytes, Ordering::Relaxed);
                self.files.fetch_add(files, Ordering::Relaxed);
                self.folders.fetch_add(subfolders.len() as u64, Ordering::Relaxed);
            }
            queue.finish(&mut subfolders);
        }
    }
}

/// The folders still to walk, and how many are queued or being walked.
struct Queue {
    state: Mutex<(Vec<PathBuf>, usize)>,
    wake: Condvar,
}

impl Queue {
    /// The next folder, waiting for one if the others are busy; `None` once
    /// the whole walk is over.
    fn take(&self) -> Option<PathBuf> {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(dir) = state.0.pop() {
                return Some(dir);
            }
            if state.1 == 0 {
                return None;
            }
            state = self.wake.wait(state).unwrap();
        }
    }

    /// Reports a folder as walked and queues the folders found in it.
    fn finish(&self, found: &mut Vec<PathBuf>) {
        let mut state = self.state.lock().unwrap();
        let added = found.len();
        state.1 = state.1 + added - 1;
        state.0.append(found);
        if state.1 == 0 {
            self.wake.notify_all();
        } else if added > 0 {
            for _ in 0..added.min(8) {
                self.wake.notify_one();
            }
        }
    }
}

/// The Size line: "211 MB (221,664,650 bytes)", from `StrFormatByteSize`
/// and `AddCommas64`. A zero size has a string of its own, "0 bytes".
pub fn format_size_line(bytes: u64) -> String {
    if bytes == 0 {
        return "0 bytes".into();
    }
    format!("{} ({} bytes)", format_byte_size(bytes), group_digits(bytes))
}

/// The Contains line: "1,624 Files, 169 Folders", with no singular forms.
pub fn format_contains(totals: FolderTotals) -> String {
    format!("{} Files, {} Folders", group_digits(totals.files), group_digits(totals.folders))
}

/// The Location line of several items: "All in /Users/me" when they share a
/// parent. The wording for items from different folders is a guess (the
/// page's strings weren't traced; a selection only spans folders in search).
pub fn format_common_location<'a>(parents: impl IntoIterator<Item = &'a Path>) -> String {
    let mut parents = parents.into_iter();
    let Some(first) = parents.next() else { return String::new() };
    if parents.all(|p| p == first) { format!("All in {}", first.display()) } else { "Several locations".into() }
}

/// A day a date line names instead of its weekday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelativeDay {
    Today,
    Yesterday,
}

/// Whether a date line starts with "Today" or "Yesterday" instead of the
/// weekday, as `SHFormatDateTimeW` does with `FDTF_RELATIVE` (the flags
/// `SetDateTimeTextEx` passes). The words and the date itself come from the
/// system locale, in the app.
pub fn relative_day(date: NaiveDate, today: NaiveDate) -> Option<RelativeDay> {
    if date == today {
        Some(RelativeDay::Today)
    } else if today.pred_opt() == Some(date) {
        Some(RelativeDay::Yesterday)
    } else {
        None
    }
}

/// True for FAT and exFAT volumes. Explorer looks for "FAT" in the file
/// system name and drops the time from the Accessed line there.
#[cfg(target_os = "macos")]
fn is_fat_volume(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: statfs is plain old data, so all-zero is a valid value.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid NUL-terminated path and a statfs buffer to fill.
    if unsafe { libc::statfs(cpath.as_ptr(), &mut st) } != 0 {
        return false;
    }
    // SAFETY: the kernel NUL-terminates f_fstypename.
    let name = unsafe { std::ffi::CStr::from_ptr(st.f_fstypename.as_ptr()) };
    matches!(name.to_bytes(), b"msdos" | b"exfat")
}

#[cfg(target_os = "linux")]
fn is_fat_volume(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    const MSDOS_SUPER_MAGIC: i64 = 0x4d44;
    const EXFAT_SUPER_MAGIC: i64 = 0x2011_bab0;
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: statfs is plain old data, so all-zero is a valid value.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid NUL-terminated path and a statfs buffer to fill.
    if unsafe { libc::statfs(cpath.as_ptr(), &mut st) } != 0 {
        return false;
    }
    #[allow(clippy::unnecessary_cast)]
    let magic = st.f_type as i64;
    matches!(magic, MSDOS_SUPER_MAGIC | EXFAT_SUPER_MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_location() {
        let a = Path::new("/a");
        assert_eq!(format_common_location([a, a]), "All in /a");
        assert_eq!(format_common_location([a, Path::new("/b")]), "Several locations");
        assert_eq!(format_common_location([]), "");
    }

    #[test]
    fn selection_counts_files_and_folders() {
        let dir = std::env::temp_dir().join(format!("macsplorer-sel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub/inner")).unwrap();
        fs::write(dir.join("sub/a"), [0u8; 10]).unwrap();
        fs::write(dir.join("sub/inner/b"), [0u8; 5]).unwrap();
        let size = FolderSize::default();
        size.measure_selection(&[SelectedItem::File(100), SelectedItem::Folder(dir.join("sub"))]);
        assert!(size.is_done());
        // The selected folder counts, and so does the one inside it.
        assert_eq!(size.totals(), FolderTotals { bytes: 115, files: 3, folders: 2 });
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn size_line() {
        assert_eq!(format_size_line(221_664_650), "211 MB (221,664,650 bytes)");
        assert_eq!(format_size_line(512), "512 bytes (512 bytes)");
        assert_eq!(format_size_line(1_500), "1.46 KB (1,500 bytes)");
        assert_eq!(format_size_line(0), "0 bytes");
    }

    #[test]
    fn contains_line() {
        assert_eq!(format_contains(FolderTotals { bytes: 0, files: 1_624, folders: 169 }), "1,624 Files, 169 Folders");
        assert_eq!(format_contains(FolderTotals { bytes: 0, files: 1, folders: 0 }), "1 Files, 0 Folders");
    }

    #[test]
    fn relative_days() {
        let day = |d| NaiveDate::from_ymd_opt(2025, 11, d).unwrap();
        assert_eq!(relative_day(day(22), day(22)), Some(RelativeDay::Today));
        assert_eq!(relative_day(day(21), day(22)), Some(RelativeDay::Yesterday));
        assert_eq!(relative_day(day(20), day(22)), None);
        assert_eq!(relative_day(day(23), day(22)), None);
    }

    #[test]
    fn folder_walk() {
        let dir = std::env::temp_dir().join(format!("macsplorer-props-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::create_dir_all(dir.join("c")).unwrap();
        fs::write(dir.join("one.txt"), [0u8; 10]).unwrap();
        fs::write(dir.join("a/.hidden"), [0u8; 20]).unwrap();
        fs::write(dir.join("a/b/two.txt"), [0u8; 30]).unwrap();
        // A link to a folder counts as a file and isn't walked into.
        std::os::unix::fs::symlink(dir.join("a"), dir.join("c/link")).unwrap();

        let size = FolderSize::default();
        assert!(!size.is_done());
        size.measure(&dir);
        assert!(size.is_done());
        let totals = size.totals();
        assert_eq!((totals.files, totals.folders), (4, 3));
        let link_len = fs::symlink_metadata(dir.join("c/link")).unwrap().len();
        assert_eq!(totals.bytes, 60 + link_len);

        let cancelled = FolderSize::default();
        cancelled.cancel();
        cancelled.measure(&dir);
        assert!(cancelled.is_done());
        assert_eq!(cancelled.totals(), FolderTotals::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn permission_boxes() {
        assert_eq!(permissions(0o100750), [[true, true, true], [true, false, true], [false; 3]]);
        assert_eq!(permissions(0o040000), [[false; 3]; 3]);
        // The special bits aren't among the nine.
        assert_eq!(permissions(0o104644), [[true, true, false], [true, false, false], [true, false, false]]);
    }

    #[test]
    fn permissions_round_trip() {
        let mode = 0o104750;
        assert_eq!(with_permissions(mode, permissions(mode)), mode);
        // Only the nine bits change.
        assert_eq!(with_permissions(0o104750, [[true, true, false], [false; 3], [true, false, false]]), 0o104604);
        assert_eq!(with_permissions(0o040755, [[false; 3]; 3]), 0o040000);
    }

    #[test]
    fn setting_permissions_follows_links() {
        let dir = std::env::temp_dir().join(format!("macsplorer-props-chmod-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f");
        fs::write(&file, b"x").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o4644)).unwrap();
        std::os::unix::fs::symlink("f", dir.join("link")).unwrap();

        set_permissions(&dir.join("link"), [[true, true, true], [true, false, true], [false; 3]]).unwrap();
        // The target changed, with its set-user-ID bit kept.
        assert_eq!(fs::metadata(&file).unwrap().mode() & 0o7777, 0o4750);
        assert!(set_permissions(&dir.join("missing"), [[false; 3]; 3]).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn info_for_links() {
        let dir = std::env::temp_dir().join(format!("macsplorer-props-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("real"), [0u8; 7]).unwrap();
        std::os::unix::fs::symlink("real", dir.join("good")).unwrap();
        std::os::unix::fs::symlink("missing", dir.join("gone")).unwrap();

        assert_eq!(item_info(&dir.join("real")).unwrap().link, None);
        let good = item_info(&dir.join("good")).unwrap();
        let link = good.link.unwrap();
        assert_eq!(link, LinkInfo { target: "real".into(), broken: false });
        assert_eq!(format_target(&link), "real");
        // A link shows its target's size and permissions.
        assert_eq!(good.size, 7);
        assert_eq!(good.mode & 0o170000, 0o100000);

        let gone = item_info(&dir.join("gone")).unwrap();
        let link = gone.link.unwrap();
        assert!(link.broken);
        assert_eq!(format_target(&link), "missing (not found)");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn info_for_file_and_folder() {
        let dir = std::env::temp_dir().join(format!("macsplorer-props-info-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("f"), [0u8; 7]).unwrap();
        let file = item_info(&dir.join("f")).unwrap();
        assert!(!file.is_dir);
        assert_eq!(file.size, 7);
        assert!(file.modified.is_some());
        assert!(item_info(&dir).unwrap().is_dir);
        assert!(item_info(&dir.join("missing")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
