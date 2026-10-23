//! Archives as folders: which files are zip, tar and similar archives, what
//! is inside them, and getting items out.
//!
//! Explorer shows `.zip` files as Compressed (zipped) Folders (`zipfldr.dll`)
//! and `.cab` files through the Cabinet File Viewer (`cabview.dll`)
//! (`docs/archive-findings.md`). Macsplorer browses zip and tar archives the
//! same way, read-only. A place inside an archive is a path that runs through
//! the archive file, `~/Downloads/foo.zip/src/main.rs`: a file can't have
//! children on disk, so such a path never names a real item ([`split`]).
//!
//! The archive's index is read once and kept, keyed by the file's path, size
//! and modification time, as `CZipFolder::BuildMainEnumerator` keeps it.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::fileops::{ITEM_UNITS, Progress};
use crate::listing::{Entry, is_hidden_name, is_package_name};
use crate::rename::RenameKind;
use crate::undo::UndoOp;

/// How a tar archive is compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar(Compression),
    /// A Microsoft cabinet.
    Cab,
    SevenZip,
    Rar,
}

impl Format {
    /// Whether Macsplorer can open it as a folder. The others are only
    /// recognized (their names are colored) and open in their app.
    pub fn browsable(self) -> bool {
        matches!(self, Format::Zip | Format::Tar(_))
    }
}

/// Single extensions and the format they stand for.
const EXTENSIONS: &[(&str, Format)] = &[
    ("zip", Format::Zip),
    ("tar", Format::Tar(Compression::None)),
    ("tgz", Format::Tar(Compression::Gzip)),
    ("taz", Format::Tar(Compression::Gzip)),
    ("tbz", Format::Tar(Compression::Bzip2)),
    ("tbz2", Format::Tar(Compression::Bzip2)),
    ("txz", Format::Tar(Compression::Xz)),
    ("tzst", Format::Tar(Compression::Zstd)),
    ("cab", Format::Cab),
    ("7z", Format::SevenZip),
    ("rar", Format::Rar),
];

/// The compressor extensions that make a `.tar.<ext>` a compressed tar.
const TAR_COMPRESSIONS: &[(&str, Compression)] =
    &[("gz", Compression::Gzip), ("bz2", Compression::Bzip2), ("xz", Compression::Xz), ("zst", Compression::Zstd)];

/// The archive format a file name says, ignoring case. A bare `.gz` or
/// `.xz` (one compressed file, not an archive) is none.
pub fn format_of(name: &str) -> Option<Format> {
    let lower = name.to_lowercase();
    let (stem, ext) = lower.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    if let Some((_, c)) = TAR_COMPRESSIONS.iter().find(|(e, _)| *e == ext) {
        return stem.ends_with(".tar").then_some(Format::Tar(*c)).filter(|_| stem.len() > ".tar".len());
    }
    EXTENSIONS.iter().find(|(e, _)| *e == ext).map(|(_, f)| *f)
}

/// The name without the archive's extension (all of `.tar.gz`), for the
/// folder Extract All makes: `foo.tar.gz` gives `foo`.
pub fn stem(name: &str) -> &str {
    let lower = name.to_lowercase();
    let cut = TAR_COMPRESSIONS
        .iter()
        .map(|(ext, _)| format!(".tar.{ext}"))
        .chain(EXTENSIONS.iter().map(|(ext, _)| format!(".{ext}")))
        .find(|suffix| lower.ends_with(suffix.as_str()) && lower.len() > suffix.len())
        .map_or(0, |suffix| suffix.len());
    &name[..name.len() - cut]
}

/// A file Macsplorer opens as a folder.
pub fn is_browsable(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).and_then(format_of).is_some_and(Format::browsable) && path.is_file()
}

/// Splits a path that runs through an archive into the archive file and the
/// path inside it (`/`-separated, empty for the archive's top). `None` for
/// a path that doesn't. The innermost real archive wins; an archive inside
/// an archive is just a file in it.
pub fn split(path: &Path) -> Option<(PathBuf, String)> {
    let archive = path.ancestors().find(|a| is_browsable(a))?;
    let rest = path.strip_prefix(archive).ok()?;
    let inner = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
    Some((archive.to_path_buf(), inner))
}

/// Whether `path` is inside an archive (not the archive file itself).
pub fn is_inside(path: &Path) -> bool {
    split(path).is_some_and(|(archive, _)| archive != path)
}

/// One file or folder in an archive.
#[derive(Clone, Debug)]
pub struct Item {
    /// The path inside the archive, `/`-separated, without a leading or
    /// trailing `/`.
    pub path: String,
    pub dir: bool,
    /// Uncompressed size; 0 for folders.
    pub size: u64,
    /// The compressed size, where the format keeps one per item (zip).
    pub packed: Option<u64>,
    pub modified: Option<SystemTime>,
    /// `st_mode` with the file type bits.
    pub mode: u32,
    /// Needs a password (zip).
    pub encrypted: bool,
    /// A symbolic link; its target is the content (zip) or in the header (tar).
    pub symlink: bool,
    /// Where the item is in the zip's central directory.
    zip_index: Option<usize>,
}

impl Item {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    fn folder(path: String) -> Self {
        Self { path, dir: true, size: 0, packed: None, modified: None, mode: DIR_MODE, encrypted: false, symlink: false, zip_index: None }
    }
}

const DIR_MODE: u32 = 0o040755;
const FILE_MODE: u32 = 0o100644;
const TYPE_MASK: u32 = 0o170000;
const SYMLINK_TYPE: u32 = 0o120000;

/// What an archive holds.
#[derive(Debug)]
pub struct Index {
    pub format: Format,
    items: Vec<Item>,
    by_path: HashMap<String, usize>,
    /// Each folder's items, by the folder's path ("" for the top).
    children: HashMap<String, Vec<usize>>,
}

impl Index {
    fn new(format: Format, listed: Vec<Item>) -> Self {
        let mut index = Index { format, items: Vec::new(), by_path: HashMap::new(), children: HashMap::new() };
        index.children.insert(String::new(), Vec::new());
        for item in listed {
            index.add_parents(&item.path);
            match index.by_path.get(&item.path) {
                // A folder made up for an earlier item gets the real one's details.
                Some(&i) => {
                    if index.items[i].dir && item.dir {
                        index.items[i] = item;
                    }
                }
                None => index.push(item),
            }
        }
        index
    }

    fn add_parents(&mut self, path: &str) {
        let mut end = 0;
        while let Some(slash) = path[end..].find('/') {
            end += slash;
            let parent = &path[..end];
            if !self.by_path.contains_key(parent) {
                self.push(Item::folder(parent.to_owned()));
            }
            end += 1;
        }
    }

    fn push(&mut self, item: Item) {
        let parent = item.path.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
        let i = self.items.len();
        self.by_path.insert(item.path.clone(), i);
        if item.dir {
            self.children.entry(item.path.clone()).or_default();
        }
        self.children.entry(parent).or_default().push(i);
        self.items.push(item);
    }

    /// The item at `path` inside the archive.
    pub fn item(&self, path: &str) -> Option<&Item> {
        self.by_path.get(path).map(|&i| &self.items[i])
    }

    /// What the folder at `path` holds ("" for the top), `None` if there is no
    /// such folder.
    pub fn children(&self, path: &str) -> Option<impl Iterator<Item = &Item>> {
        Some(self.children.get(path)?.iter().map(|&i| &self.items[i]))
    }

    /// `path` and everything below it ("" for the whole archive).
    pub fn under<'a>(&'a self, path: &'a str) -> impl Iterator<Item = &'a Item> {
        self.items.iter().filter(move |item| path.is_empty() || item.path == path || item.path.strip_prefix(path).is_some_and(|rest| rest.starts_with('/')))
    }

    /// Whether anything at or below these paths needs a password.
    pub fn needs_password(&self, paths: &[String]) -> bool {
        paths.iter().any(|p| self.under(p).any(|item| item.encrypted))
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A path inside an archive made safe: `\` read as `/`, empty and `.` parts
/// dropped, a leading `/` dropped. `None` for one that climbs out with `..`,
/// which is never listed or extracted (`cabview.dll` skips such names too,
/// `docs/archive-findings.md` §3.1).
fn clean_path(name: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The Mac's resource-fork folder that Finder's Compress adds; Archive
/// Utility leaves it out too.
fn is_mac_metadata(path: &str) -> bool {
    path == "__MACOSX" || path.starts_with("__MACOSX/")
}

fn invalid(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

/// The error for a missing or wrong password, recognized by [`is_password_error`].
#[derive(Debug)]
struct PasswordNeeded;

impl std::fmt::Display for PasswordNeeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a password is needed, or the one given is wrong")
    }
}

impl std::error::Error for PasswordNeeded {}

fn password_error() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, PasswordNeeded)
}

/// The extraction stopped for want of the right password.
pub fn is_password_error(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<PasswordNeeded>())
}

fn zip_error(e: zip::result::ZipError) -> io::Error {
    match e {
        zip::result::ZipError::Io(e) => e,
        zip::result::ZipError::InvalidPassword => password_error(),
        zip::result::ZipError::UnsupportedArchive(msg) if msg == zip::result::ZipError::PASSWORD_REQUIRED => password_error(),
        e => invalid(e),
    }
}

/// A zip time is local time without a zone (MS-DOS).
fn dos_time(t: zip::DateTime) -> Option<SystemTime> {
    use chrono::TimeZone;
    let date = chrono::NaiveDate::from_ymd_opt(t.year().into(), t.month().into(), t.day().into())?;
    let local = date.and_hms_opt(t.hour().into(), t.minute().into(), t.second().into())?;
    chrono::Local.from_local_datetime(&local).earliest().map(SystemTime::from)
}

fn read_zip(archive: &Path) -> io::Result<Vec<Item>> {
    let mut zip = zip::ZipArchive::new(BufReader::new(File::open(archive)?)).map_err(zip_error)?;
    let mut items = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i).map_err(zip_error)?;
        let Some(path) = clean_path(file.name()).filter(|p| !is_mac_metadata(p)) else { continue };
        let dir = file.is_dir();
        // Info-ZIP and Finder's Compress keep the time in UTC in an extra field.
        let utc = file.extra_data_fields().find_map(|f| match f {
            zip::extra_fields::ExtraField::ExtendedTimestamp(ts) => ts.mod_time(),
            _ => None,
        });
        let modified = utc.map(|s| UNIX_EPOCH + Duration::from_secs(s.into())).or_else(|| file.last_modified().and_then(dos_time));
        let symlink = file.is_symlink();
        let mode = match file.unix_mode() {
            Some(m) if m & TYPE_MASK != 0 => m,
            Some(m) => m | if dir { 0o040000 } else { 0o100000 },
            None if dir => DIR_MODE,
            None => FILE_MODE,
        };
        items.push(Item {
            path,
            dir,
            size: if dir { 0 } else { file.size() },
            packed: (!dir).then(|| file.compressed_size()),
            modified,
            mode,
            encrypted: file.encrypted(),
            symlink,
            zip_index: Some(i),
        });
    }
    Ok(items)
}

/// A reader for the tar inside `archive`, decompressed.
fn tar_stream(archive: &Path, compression: Compression) -> io::Result<Box<dyn Read>> {
    let file = BufReader::with_capacity(1 << 16, File::open(archive)?);
    Ok(match compression {
        Compression::None => Box::new(file),
        Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(file)),
        Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(file)),
        Compression::Xz => Box::new(lzma_rust2::XzReader::new(file, true)),
        Compression::Zstd => Box::new(ruzstd::decoding::StreamingDecoder::new(file).map_err(invalid)?),
    })
}

fn tar_item(entry: &tar::Entry<'_, impl Read>) -> io::Result<Option<Item>> {
    use tar::EntryType;
    let header = entry.header();
    let kind = header.entry_type();
    let (dir, symlink) = match kind {
        EntryType::Directory => (true, false),
        EntryType::Regular | EntryType::Continuous | EntryType::GNUSparse => (false, false),
        EntryType::Symlink => (false, true),
        // Hard links, devices, FIFOs and the format's own records aren't shown.
        _ => return Ok(None),
    };
    let Some(path) = clean_path(&entry.path()?.to_string_lossy()) else { return Ok(None) };
    let bits = header.mode().unwrap_or(if dir { 0o755 } else { 0o644 }) & 0o7777;
    let mode = bits | if dir { 0o040000 } else if symlink { SYMLINK_TYPE } else { 0o100000 };
    let modified = header.mtime().ok().map(|s| UNIX_EPOCH + Duration::from_secs(s));
    Ok(Some(Item { path, dir, size: if dir { 0 } else { entry.size() }, packed: None, modified, mode, encrypted: false, symlink, zip_index: None }))
}

fn read_tar(archive: &Path, compression: Compression) -> io::Result<Vec<Item>> {
    let mut tar = tar::Archive::new(tar_stream(archive, compression)?);
    let mut items = Vec::new();
    for entry in tar.entries()? {
        if let Some(item) = tar_item(&entry?)? {
            items.push(item);
        }
    }
    Ok(items)
}

fn read_index(archive: &Path) -> io::Result<Index> {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let items = match format_of(&name) {
        Some(Format::Zip) => read_zip(archive)?,
        Some(Format::Tar(c)) => read_tar(archive, c)?,
        _ => return Err(io::Error::new(io::ErrorKind::Unsupported, "this kind of archive can't be opened as a folder")),
    };
    Ok(Index::new(format_of(&name).unwrap_or(Format::Zip), items))
}

/// The file's identity for the cache: size and modification time.
type Stamp = (u64, Option<SystemTime>);

/// The indexes read so far, by archive.
type Cache = Mutex<HashMap<PathBuf, (Stamp, Arc<Index>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// How many archives' indexes are kept.
const CACHED: usize = 8;

/// The index of `archive`, read the first time and again when the file
/// changes. Reading a large compressed tar takes a while: call from a worker.
pub fn index(archive: &Path) -> io::Result<Arc<Index>> {
    let meta = fs::metadata(archive)?;
    let stamp = (meta.len(), meta.modified().ok());
    if let Some((s, index)) = cache().lock().ok().and_then(|c| c.get(archive).cloned())
        && s == stamp
    {
        return Ok(index);
    }
    let index = Arc::new(read_index(archive)?);
    if let Ok(mut cache) = cache().lock() {
        if cache.len() >= CACHED && !cache.contains_key(archive) {
            let drop = cache.keys().next().cloned();
            drop.map(|k| cache.remove(&k));
        }
        cache.insert(archive.to_path_buf(), (stamp, index.clone()));
    }
    Ok(index)
}

fn not_a_folder(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotADirectory, format!("{} is not a folder in the archive", path.display()))
}

fn entry_of(item: &Item, path: PathBuf) -> Entry {
    let name = item.name().to_owned();
    let kind = match item.dir {
        true if is_package_name(&name) => RenameKind::Package,
        true => RenameKind::Folder,
        false => RenameKind::File,
    };
    Entry {
        hidden: is_hidden_name(&name),
        name,
        path,
        kind,
        size: (!item.dir).then_some(item.size),
        modified: item.modified,
        created: None,
        accessed: None,
        mode: item.mode,
        owner: String::new(),
        group: String::new(),
        comment: None,
        custom_icon: false,
        folder_style: None,
        branch: None,
        link: None,
    }
}

/// Lists the folder at `path` inside an archive, like `listing::list_dir`
/// lists one on disk. `None` when the path isn't inside an archive.
pub fn list(path: &Path, show_hidden: bool) -> Option<io::Result<Vec<Entry>>> {
    let (archive, inner) = split(path)?;
    let listed = index(&archive).and_then(|index| {
        let children = index.children(&inner).ok_or_else(|| not_a_folder(path))?;
        Ok(children.filter(|item| show_hidden || !is_hidden_name(item.name())).map(|item| entry_of(item, path.join(item.name()))).collect())
    });
    Some(listed)
}

/// The item at a path inside an archive, as a listing would show it.
pub fn entry(path: &Path) -> Option<Entry> {
    let (archive, inner) = split(path)?;
    let index = index(&archive).ok()?;
    index.item(&inner).map(|item| entry_of(item, path.to_path_buf()))
}

/// Where items from `archive` are put to be opened: one folder per archive
/// in the cache folder, reused, like `CTempFileNameArray::GetTempLocation`'s
/// folder per archive in `%TEMP%`.
pub fn open_folder(archive: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    archive.hash(&mut hash);
    fs::metadata(archive).ok().and_then(|m| m.modified().ok()).hash(&mut hash);
    let name = archive.file_name()?.to_string_lossy();
    Some(staging_root()?.join(format!("{name}-{:08x}", hash.finish() as u32)))
}

/// A new empty folder to extract into before items are moved to where
/// they go (a paste, a drop, Extract All).
pub fn scratch_folder() -> io::Result<PathBuf> {
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = staging_root().ok_or_else(|| io::Error::other("no cache folder"))?;
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = root.join(format!(".extract-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn staging_root() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("macsplorer").join("archives"))
}

/// Removes what earlier runs extracted to open, a day after it was made.
/// Windows deletes its copies when the last zip window closes
/// (`CZipFolderViewCB::s_DeleteTempZipFilesThreadProc`); a file can still be
/// open in its app then, so Macsplorer waits longer.
pub fn clean_staging() {
    let Some(root) = staging_root() else { return };
    let Ok(dir) = fs::read_dir(&root) else { return };
    let day = Duration::from_secs(24 * 60 * 60);
    for entry in dir.flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age > day));
        if old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The paths inside one archive, grouped: (archive, inner paths). `None` if
/// they aren't all inside the same archive.
pub fn group(paths: &[PathBuf]) -> Option<(PathBuf, Vec<String>)> {
    let mut archive = None;
    let mut inner = Vec::new();
    for path in paths {
        let (a, i) = split(path)?;
        if archive.get_or_insert_with(|| a.clone()) != &a {
            return None;
        }
        inner.push(i);
    }
    Some((archive?, inner))
}

/// The quarantine mark macOS puts on downloads. Archive Utility copies the
/// archive's onto everything it extracts, so Gatekeeper checks an app from
/// a downloaded zip; Explorer does the same with the zone of the zip
/// (`IAttachmentExecute`, `docs/archive-findings.md` §2.5).
#[cfg(target_os = "macos")]
const QUARANTINE: &std::ffi::CStr = c"com.apple.quarantine";

struct Marker {
    #[cfg(target_os = "macos")]
    quarantine: Option<Vec<u8>>,
}

impl Marker {
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    fn of(archive: &Path) -> Self {
        Self {
            #[cfg(target_os = "macos")]
            quarantine: crate::xattr::read(archive, QUARANTINE),
        }
    }

    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    fn mark(&self, path: &Path) {
        #[cfg(target_os = "macos")]
        if let Some(q) = &self.quarantine {
            let _ = crate::xattr::set(path, QUARANTINE, q);
        }
    }
}

/// Extracts `inner` (paths inside `archive`; "" for all of it) into `dest`,
/// each under its own name, the way a copy would place them. Returns the
/// top-level items made. A cancel removes what was made and returns
/// `Interrupted`; a missing or wrong password returns an error that
/// [`is_password_error`] recognizes.
pub fn extract(archive: &Path, inner: &[String], dest: &Path, password: Option<&str>, progress: &Progress) -> io::Result<Vec<PathBuf>> {
    let index = index(archive)?;
    // Where each wanted item goes.
    let mut wanted: HashMap<&str, PathBuf> = HashMap::new();
    let mut top = Vec::new();
    for sel in inner {
        let base = match sel.rsplit_once('/') {
            Some((parent, _)) => parent.len() + 1,
            None => 0,
        };
        if !sel.is_empty() && index.item(sel).is_none() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("\"{sel}\" isn't in the archive")));
        }
        for item in index.under(sel) {
            let rel = if sel.is_empty() { &item.path[..] } else { &item.path[base..] };
            let out = dest.join(rel);
            if !rel.contains('/') {
                top.push(out.clone());
            }
            wanted.insert(&item.path, out);
        }
    }
    if password.is_none() && index.needs_password(inner) {
        return Err(password_error());
    }
    let total: u64 = wanted.keys().filter_map(|p| index.item(p)).map(|i| i.size + ITEM_UNITS).sum();
    progress.begin(total.max(1));
    let result = (|| {
        // Folders first, so every file has somewhere to go.
        let mut dirs: Vec<(&Item, &PathBuf)> = wanted.iter().filter_map(|(p, out)| index.item(p).filter(|i| i.dir).map(|i| (i, out))).collect();
        dirs.sort_by_key(|(_, out)| out.as_os_str().len());
        for (_, out) in &dirs {
            fs::create_dir_all(out)?;
            progress.add(ITEM_UNITS);
        }
        let marker = Marker::of(archive);
        match index.format {
            Format::Zip => extract_zip(archive, &index, &wanted, password, progress, &marker)?,
            Format::Tar(c) => extract_tar(archive, c, &wanted, progress, &marker)?,
            _ => return Err(io::Error::new(io::ErrorKind::Unsupported, "this kind of archive can't be extracted")),
        }
        // Folder times last: filling a folder changes its time.
        for (item, out) in dirs.iter().rev() {
            marker.mark(out);
            set_times(out, item);
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(top),
        Err(e) => {
            for path in &top {
                let _ = if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
            }
            Err(e)
        }
    }
}

fn set_times(path: &Path, item: &Item) {
    if let (Some(t), Ok(file)) = (item.modified, File::open(path)) {
        let _ = file.set_modified(t);
    }
}

/// Writes one file from `src`, a piece at a time so a cancel takes effect
/// within a big file.
fn write_file(src: &mut dyn Read, out: &Path, item: &Item, progress: &Progress, marker: &Marker) -> io::Result<()> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    progress.set_item(Path::new(&item.path), out.parent().unwrap_or(out));
    let mut file = File::create(out)?;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        if progress.is_cancelled() {
            drop(file);
            let _ = fs::remove_file(out);
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        progress.add(n as u64);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bits = item.mode & 0o777;
        if bits != 0 {
            let _ = file.set_permissions(fs::Permissions::from_mode(bits | 0o200));
        }
    }
    if let Some(t) = item.modified {
        let _ = file.set_modified(t);
    }
    drop(file);
    marker.mark(out);
    progress.add(ITEM_UNITS);
    Ok(())
}

#[cfg(unix)]
fn make_symlink(target: &str, out: &Path, progress: &Progress) -> io::Result<()> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    std::os::unix::fs::symlink(target, out)?;
    progress.add(ITEM_UNITS);
    Ok(())
}

fn extract_zip(archive: &Path, index: &Index, wanted: &HashMap<&str, PathBuf>, password: Option<&str>, progress: &Progress, marker: &Marker) -> io::Result<()> {
    let mut zip = zip::ZipArchive::new(BufReader::new(File::open(archive)?)).map_err(zip_error)?;
    // In archive order, which reads the file front to back.
    let mut files: Vec<(&Item, &PathBuf)> = wanted.iter().filter_map(|(p, out)| index.item(p).filter(|i| !i.dir).map(|i| (i, out))).collect();
    files.sort_by_key(|(item, _)| item.zip_index);
    for (item, out) in files {
        let Some(i) = item.zip_index else { continue };
        let mut file = match (item.encrypted, password) {
            (true, Some(pw)) => zip.by_index_decrypt(i, pw.as_bytes()),
            _ => zip.by_index(i),
        }
        .map_err(zip_error)?;
        if item.symlink {
            #[cfg(unix)]
            {
                let mut target = String::new();
                file.read_to_string(&mut target)?;
                make_symlink(&target, out, progress)?;
            }
            continue;
        }
        // ZipCrypto lets about one wrong password in 256 through its check;
        // the CRC at the end of the data catches it as a read error.
        write_file(&mut file, out, item, progress, marker).map_err(|e| if item.encrypted && e.kind() == io::ErrorKind::InvalidData { password_error() } else { e })?;
    }
    Ok(())
}

fn extract_tar(archive: &Path, compression: Compression, wanted: &HashMap<&str, PathBuf>, progress: &Progress, marker: &Marker) -> io::Result<()> {
    let mut left = wanted.len();
    let mut tar = tar::Archive::new(tar_stream(archive, compression)?);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let Some(item) = tar_item(&entry)? else { continue };
        let Some(out) = wanted.get(item.path.as_str()) else { continue };
        if item.dir {
            // Made already.
        } else if item.symlink {
            #[cfg(unix)]
            if let Some(target) = entry.link_name()? {
                make_symlink(&target.to_string_lossy(), out, progress)?;
            }
        } else {
            write_file(&mut entry, out, &item, progress, marker)?;
        }
        left -= 1;
        // A tar has no index to jump with: stop reading once all are out.
        if left == 0 {
            break;
        }
    }
    Ok(())
}

/// What undoing an extraction does: the items moved out of the scratch
/// folder become copies to remove, and anything they replaced comes back.
pub fn moved_as_copied(op: UndoOp) -> UndoOp {
    match op {
        UndoOp::Move(pairs) => UndoOp::Copy(pairs.into_iter().map(|(_, to)| to).collect()),
        UndoOp::Group(ops) => UndoOp::Group(ops.into_iter().map(moved_as_copied).collect()),
        other => other,
    }
}

/// Where Extract All puts an archive's items by default: a folder named
/// after it beside it, as `CZipWiz` suggests; for an archive in a temporary
/// folder (opened straight from a browser or mail), in Documents instead
/// (`PathIsTemporaryW`, `docs/archive-findings.md` §2.8).
pub fn default_destination(archive: &Path) -> PathBuf {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = stem(&name).to_owned();
    let parent = archive.parent().unwrap_or(Path::new("/"));
    let temporary = [std::env::temp_dir(), PathBuf::from("/tmp"), PathBuf::from("/private/tmp"), PathBuf::from("/var/folders")]
        .iter()
        .chain(staging_root().iter())
        .any(|t| parent.starts_with(t));
    match dirs::document_dir().filter(|_| temporary) {
        Some(docs) => docs.join(folder),
        None => parent.join(folder),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_extensions() {
        assert_eq!(format_of("a.zip"), Some(Format::Zip));
        assert_eq!(format_of("Setup.CAB"), Some(Format::Cab));
        assert_eq!(format_of("x.7z"), Some(Format::SevenZip));
        assert_eq!(format_of("x.rar"), Some(Format::Rar));
        assert_eq!(format_of("src.tgz"), Some(Format::Tar(Compression::Gzip)));
        assert_eq!(format_of("src.tar"), Some(Format::Tar(Compression::None)));
    }

    #[test]
    fn compressed_tars() {
        assert_eq!(format_of("src.tar.gz"), Some(Format::Tar(Compression::Gzip)));
        assert_eq!(format_of("src.TAR.BZ2"), Some(Format::Tar(Compression::Bzip2)));
        assert_eq!(format_of("src.tar.xz"), Some(Format::Tar(Compression::Xz)));
        assert_eq!(format_of("src.tar.zst"), Some(Format::Tar(Compression::Zstd)));
    }

    #[test]
    fn not_archives() {
        // One compressed file is not an archive.
        assert_eq!(format_of("log.gz"), None);
        assert_eq!(format_of("data.xz"), None);
        assert_eq!(format_of(".tar.gz"), None);
        assert_eq!(format_of(".zip"), None);
        assert_eq!(format_of("zip"), None);
        assert_eq!(format_of("notes.txt"), None);
        // Disk images and installers mount or install; they aren't browsed.
        assert_eq!(format_of("App.dmg"), None);
        assert_eq!(format_of("App.pkg"), None);
    }

    #[test]
    fn stems() {
        assert_eq!(stem("foo.zip"), "foo");
        assert_eq!(stem("foo.tar.gz"), "foo");
        assert_eq!(stem("Foo.1.2.TGZ"), "Foo.1.2");
        assert_eq!(stem("notes.txt"), "notes.txt");
        assert_eq!(stem(".zip"), ".zip");
    }

    #[test]
    fn unsafe_names_are_dropped() {
        assert_eq!(clean_path("a/b.txt").as_deref(), Some("a/b.txt"));
        assert_eq!(clean_path("/etc/passwd").as_deref(), Some("etc/passwd"));
        assert_eq!(clean_path("./a//b/").as_deref(), Some("a/b"));
        assert_eq!(clean_path("a\\b.txt").as_deref(), Some("a/b.txt"));
        assert_eq!(clean_path("../evil"), None);
        assert_eq!(clean_path("a/../../evil"), None);
        assert_eq!(clean_path("/"), None);
    }

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("macsplorer-archive-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A zip with a file at the top, one in a folder whose folder entry is
    /// missing, Finder's `__MACOSX` folder and a name that climbs out.
    fn make_zip(path: &Path, password: Option<&str>) {
        use zip::write::SimpleFileOptions;
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let plain = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(0o644);
        let options = match password {
            Some(pw) => plain.with_aes_encryption(zip::AesMode::Aes256, pw),
            None => plain,
        };
        zip.start_file("readme.txt", options).unwrap();
        zip.write_all(b"hello\n").unwrap();
        zip.start_file("src/main.rs", options).unwrap();
        zip.write_all(&b"fn main() {}\n".repeat(100)).unwrap();
        zip.start_file("__MACOSX/._readme.txt", plain).unwrap();
        zip.write_all(b"junk").unwrap();
        zip.start_file("../evil.txt", plain).unwrap();
        zip.write_all(b"evil").unwrap();
        zip.finish().unwrap();
    }

    fn make_tgz(path: &Path) {
        let gz = flate2::write::GzEncoder::new(File::create(path).unwrap(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut add = |name: &str, data: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_mtime(1_700_000_000);
            header.set_cksum();
            tar.append_data(&mut header, name, data).unwrap();
        };
        add("pkg/bin/run.sh", b"#!/bin/sh\necho hi\n");
        add("pkg/README", b"read me\n");
        tar.into_inner().unwrap().finish().unwrap();
    }

    fn names(entries: &[Entry]) -> Vec<String> {
        let mut names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_zip_is_listed_like_a_folder() {
        let dir = fixture("list");
        let zip = dir.join("a.zip");
        make_zip(&zip, None);
        assert!(is_browsable(&zip));
        assert_eq!(split(&zip.join("src/main.rs")), Some((zip.clone(), "src/main.rs".into())));
        assert_eq!(split(&zip), Some((zip.clone(), String::new())));
        assert_eq!(split(&dir), None);
        assert!(is_inside(&zip.join("src")) && !is_inside(&zip));

        let top = list(&zip, false).unwrap().unwrap();
        assert_eq!(names(&top), ["readme.txt", "src"]);
        let src = top.iter().find(|e| e.name == "src").unwrap();
        assert!(src.is_folder() && src.path == zip.join("src"));
        let inside = list(&zip.join("src"), false).unwrap().unwrap();
        assert_eq!(names(&inside), ["main.rs"]);
        assert_eq!(inside[0].size, Some(1300));
        assert!(list(&zip.join("readme.txt"), false).unwrap().is_err());
        assert!(list(&dir, false).is_none());
    }

    #[test]
    fn extracting_from_a_zip() {
        let dir = fixture("zip");
        let zip = dir.join("a.zip");
        make_zip(&zip, None);
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        let progress = Progress::default();
        let made = extract(&zip, &["src".into(), "readme.txt".into()], &out, None, &progress).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!(fs::read(out.join("readme.txt")).unwrap(), b"hello\n");
        assert_eq!(fs::read(out.join("src/main.rs")).unwrap().len(), 1300);
        assert!(!out.join("evil.txt").exists() && !dir.join("evil.txt").exists());

        // Everything, for Extract All; the Mac's metadata stays out.
        let all = dir.join("all");
        fs::create_dir_all(&all).unwrap();
        extract(&zip, &[String::new()], &all, None, &progress).unwrap();
        assert!(all.join("src/main.rs").is_file() && !all.join("__MACOSX").exists());
    }

    #[test]
    fn a_password_is_asked_for_and_checked() {
        let dir = fixture("password");
        let zip = dir.join("secret.zip");
        make_zip(&zip, Some("open sesame"));
        let index = index(&zip).unwrap();
        assert!(index.item("readme.txt").unwrap().encrypted);
        assert!(index.needs_password(&["src".into()]));
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        let progress = Progress::default();
        let e = extract(&zip, &["readme.txt".into()], &out, None, &progress).unwrap_err();
        assert!(is_password_error(&e));
        let e = extract(&zip, &["readme.txt".into()], &out, Some("wrong"), &progress).unwrap_err();
        assert!(is_password_error(&e), "{e}");
        assert!(!out.join("readme.txt").exists());
        extract(&zip, &["readme.txt".into()], &out, Some("open sesame"), &progress).unwrap();
        assert_eq!(fs::read(out.join("readme.txt")).unwrap(), b"hello\n");
    }

    #[test]
    fn a_tar_gz_is_listed_and_extracted() {
        let dir = fixture("tgz");
        let tgz = dir.join("pkg.tar.gz");
        make_tgz(&tgz);
        let top = list(&tgz, false).unwrap().unwrap();
        assert_eq!(names(&top), ["pkg"]);
        assert_eq!(names(&list(&tgz.join("pkg"), false).unwrap().unwrap()), ["README", "bin"]);
        let run = entry(&tgz.join("pkg/bin/run.sh")).unwrap();
        assert_eq!(run.modified, Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)));

        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        extract(&tgz, &["pkg/bin".into()], &out, None, &Progress::default()).unwrap();
        let script = out.join("bin/run.sh");
        assert_eq!(fs::read(&script).unwrap(), b"#!/bin/sh\necho hi\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&script).unwrap().permissions().mode() & 0o777, 0o755);
        }
        assert!(!out.join("README").exists());
    }

    /// A plain tar of `pkg/README`, to compress in the formats a tar comes in.
    fn tar_bytes() -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(8);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "pkg/README", &b"read me\n"[..]).unwrap();
        tar.into_inner().unwrap()
    }

    #[test]
    fn xz_bzip2_and_zstd_tars_are_read() {
        let dir = fixture("formats");
        let tar = tar_bytes();
        let mut xz = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
        xz.write_all(&tar).unwrap();
        fs::write(dir.join("a.tar.xz"), xz.finish().unwrap()).unwrap();
        let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        bz.write_all(&tar).unwrap();
        fs::write(dir.join("a.tbz2"), bz.finish().unwrap()).unwrap();
        fs::write(dir.join("a.tar.zst"), ruzstd::encoding::compress_to_vec(&tar[..], ruzstd::encoding::CompressionLevel::Fastest)).unwrap();
        fs::write(dir.join("a.tar"), &tar).unwrap();
        for name in ["a.tar.xz", "a.tbz2", "a.tar.zst", "a.tar"] {
            let archive = dir.join(name);
            assert_eq!(names(&list(&archive.join("pkg"), false).unwrap().unwrap()), ["README"], "{name}");
            let out = dir.join(format!("out-{name}"));
            fs::create_dir_all(&out).unwrap();
            extract(&archive, &["pkg/README".into()], &out, None, &Progress::default()).unwrap();
            assert_eq!(fs::read(out.join("README")).unwrap(), b"read me\n", "{name}");
        }
    }

    #[test]
    fn a_cancel_removes_what_was_made() {
        let dir = fixture("cancel");
        let zip = dir.join("a.zip");
        make_zip(&zip, None);
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        let progress = Progress::default();
        progress.cancel();
        let e = extract(&zip, &["src".into()], &out, None, &progress).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
        assert!(!out.join("src").exists());
    }

    #[test]
    fn extract_all_goes_beside_the_archive() {
        assert_eq!(default_destination(Path::new("/Users/me/Downloads/foo.tar.gz")), PathBuf::from("/Users/me/Downloads/foo"));
    }

    #[test]
    fn undoing_an_extraction_removes_the_copies() {
        let op = UndoOp::Move(vec![(PathBuf::from("/scratch/a"), PathBuf::from("/dest/a"))]);
        assert!(matches!(moved_as_copied(op), UndoOp::Copy(paths) if paths == [PathBuf::from("/dest/a")]));
    }
}
