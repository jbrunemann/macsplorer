//! Reading a folder's entries with their types and sizes in as few system
//! calls as possible, for walks that touch every item below a folder.
//!
//! On macOS, `getattrlistbulk` returns the name, type and data length of many
//! entries per call, where `read_dir` plus a `lstat` per file costs one call
//! per item. Elsewhere (and on volumes that lack it) it is `read_dir`.
//! Not from Explorer.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;

/// One item in a folder. Symlinks are files here, and `len` is the size of
/// the link itself, like `lstat`.
pub struct Entry<'a> {
    pub name: &'a OsStr,
    pub is_dir: bool,
    pub len: u64,
}

/// Scratch space for [`scan`], one per thread so walks don't allocate per
/// folder.
pub struct Scratch {
    #[cfg(target_os = "macos")]
    buf: Vec<u64>,
}

// Empty off macOS, where a derive would do.
#[cfg_attr(not(target_os = "macos"), allow(clippy::derivable_impls))]
impl Default for Scratch {
    fn default() -> Self {
        Scratch {
            #[cfg(target_os = "macos")]
            buf: vec![0; bulk::BUF_BYTES / 8],
        }
    }
}

/// Calls `each` for every item in `dir`. Items that vanish or can't be read
/// are skipped; an unreadable folder is an error.
pub fn scan(dir: &Path, scratch: &mut Scratch, mut each: impl FnMut(Entry)) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    match bulk::scan(dir, &mut scratch.buf, &mut each) {
        Err(bulk::Failure::Unsupported) => {}
        Err(bulk::Failure::Io(e)) => return Err(e),
        Ok(()) => return Ok(()),
    }
    #[cfg(not(target_os = "macos"))]
    let _ = scratch;

    for item in fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let Ok(kind) = item.file_type() else { continue };
        let len = if kind.is_dir() {
            0
        } else {
            // DirEntry::metadata doesn't follow symlinks.
            item.metadata().map_or(0, |m| m.len())
        };
        each(Entry { name: &item.file_name(), is_dir: kind.is_dir(), len });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod bulk {
    use super::Entry;
    use std::ffi::{CString, OsStr};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub const BUF_BYTES: usize = 256 * 1024;
    const VDIR: u32 = 2;

    pub enum Failure {
        /// The volume doesn't do `getattrlistbulk`; use `read_dir`.
        Unsupported,
        Io(io::Error),
    }

    /// Reads a `T` at `at`, which the kernel only aligns to 4 bytes.
    ///
    /// # Safety
    /// `at..at + size_of::<T>()` must lie inside the buffer.
    unsafe fn read<T>(at: *const u8) -> T {
        // SAFETY: by the caller; read_unaligned has no alignment needs.
        unsafe { at.cast::<T>().read_unaligned() }
    }

    pub fn scan(dir: &Path, buf: &mut [u64], each: &mut impl FnMut(Entry)) -> Result<(), Failure> {
        let cpath = CString::new(dir.as_os_str().as_bytes())
            .map_err(|e| Failure::Io(io::Error::new(io::ErrorKind::InvalidInput, e)))?;
        // SAFETY: valid NUL-terminated path.
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(Failure::Io(io::Error::last_os_error()));
        }
        let result = read_entries(fd, buf, each);
        // SAFETY: fd was opened above and isn't used again.
        unsafe { libc::close(fd) };
        result
    }

    fn read_entries(fd: i32, buf: &mut [u64], each: &mut impl FnMut(Entry)) -> Result<(), Failure> {
        let mut request = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_NAME | libc::ATTR_CMN_OBJTYPE,
            volattr: 0,
            dirattr: 0,
            fileattr: libc::ATTR_FILE_DATALENGTH,
            forkattr: 0,
        };
        let mut first = true;
        loop {
            // SAFETY: buf is writable for BUF_BYTES and request is a valid attrlist.
            let count = unsafe {
                libc::getattrlistbulk(fd, (&raw mut request).cast(), buf.as_mut_ptr().cast(), BUF_BYTES, 0)
            };
            if count < 0 {
                let err = io::Error::last_os_error();
                let unsupported = matches!(err.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL));
                return Err(if first && unsupported { Failure::Unsupported } else { Failure::Io(err) });
            }
            if count == 0 {
                return Ok(());
            }
            first = false;

            let base = buf.as_ptr().cast::<u8>();
            let mut offset = 0;
            for _ in 0..count {
                // Each entry is: length, the attributes that were returned,
                // then those attributes in bit order.
                if offset + 24 > BUF_BYTES {
                    break;
                }
                // SAFETY: the entry header is inside the buffer, checked above.
                let (len, returned) = unsafe {
                    let at = base.add(offset);
                    (read::<u32>(at) as usize, read::<[u32; 5]>(at.add(4)))
                };
                if len < 24 || offset + len > BUF_BYTES {
                    break;
                }
                let end = offset + len;
                let mut at = offset + 24;
                let (mut name, mut is_dir, mut data_len) = (None, false, 0u64);
                if returned[0] & libc::ATTR_CMN_NAME != 0 && at + 8 <= end {
                    // SAFETY: the reference is inside the entry, checked above.
                    let (rel, size) = unsafe { (read::<i32>(base.add(at)), read::<u32>(base.add(at + 4))) };
                    let start = at as isize + rel as isize;
                    // The length counts the NUL.
                    if start >= 0 && size > 0 && start as usize + size as usize <= end {
                        // SAFETY: the name is inside the entry, checked above.
                        name = Some(unsafe {
                            std::slice::from_raw_parts(base.add(start as usize), size as usize - 1)
                        });
                    }
                    at += 8;
                }
                if returned[0] & libc::ATTR_CMN_OBJTYPE != 0 && at + 4 <= end {
                    // SAFETY: inside the entry, checked above.
                    is_dir = unsafe { read::<u32>(base.add(at)) } == VDIR;
                    at += 4;
                }
                if returned[3] & libc::ATTR_FILE_DATALENGTH != 0 && at + 8 <= end {
                    // SAFETY: inside the entry, checked above.
                    data_len = unsafe { read::<i64>(base.add(at)) }.max(0) as u64;
                }
                if let Some(name) = name {
                    each(Entry { name: OsStr::from_bytes(name), is_dir, len: if is_dir { 0 } else { data_len } });
                }
                offset = end;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::os::unix::fs::MetadataExt;

    /// The bulk reader must agree with `read_dir` plus `lstat` on every
    /// folder of a real tree (the workspace), links included.
    #[test]
    fn matches_read_dir() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut scratch = Scratch::default();
        let mut stack = vec![root];
        let mut checked = 0;
        while let Some(dir) = stack.pop() {
            if dir.file_name().is_some_and(|n| n == "target" || n == ".git") {
                continue;
            }
            let mut got = BTreeMap::new();
            scan(&dir, &mut scratch, |e| {
                got.insert(e.name.to_owned(), (e.is_dir, e.len));
            })
            .unwrap();
            let mut want = BTreeMap::new();
            for item in fs::read_dir(&dir).unwrap() {
                let item = item.unwrap();
                let meta = fs::symlink_metadata(item.path()).unwrap();
                let is_dir = meta.is_dir();
                want.insert(item.file_name(), (is_dir, if is_dir { 0 } else { meta.size() }));
                if is_dir {
                    stack.push(item.path());
                }
            }
            assert_eq!(got, want, "{}", dir.display());
            checked += 1;
        }
        assert!(checked > 5);
    }
}
