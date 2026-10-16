//! Dates and numbers in the system's format, as Windows takes them from the
//! user locale. macOS asks Foundation (`NSLocale`, `NSDateFormatter`), so the
//! formats chosen in System Settings apply; Linux uses the C library's
//! locale (`LC_NUMERIC`, `LC_TIME`).

use std::path::Path;
use std::time::SystemTime;

#[cfg(unix)]
use chrono::{DateTime, Local};

#[cfg(unix)]
use macsplorer_core::properties::{RelativeDay, relative_day};

/// Reads the number separators. Call once at startup, before any thread
/// starts (Linux's `setlocale` isn't thread-safe).
#[cfg(unix)]
pub fn install() {
    let (group, decimal) = imp::separators();
    // Narrow no-break spaces (French and others) are missing from Tahoma.
    macsplorer_core::listing::set_number_format(&group.replace('\u{202f}', "\u{a0}"), &decimal);
}

/// A Properties date line, like `SHFormatDateTimeW` with `FDTF_RELATIVE |
/// FDTF_LONGDATE | FDTF_LONGTIME`: the locale's long date with "Today" or
/// "Yesterday" in place of the weekday, then the long time. Blank for a
/// date the file system doesn't keep.
pub fn long_date_time(when: Option<SystemTime>, with_time: bool) -> String {
    let Some(when) = when else { return String::new() };
    #[cfg(unix)]
    {
        let relative = relative_day(DateTime::<Local>::from(when).date_naive(), Local::now().date_naive());
        imp::long_date_time(when, relative, with_time)
    }
    #[cfg(not(unix))]
    {
        let _ = (when, with_time);
        String::new()
    }
}

/// The Details view's Modified column: the locale's short date and short
/// time.
pub fn short_date_time(when: SystemTime) -> String {
    #[cfg(unix)]
    return imp::short_date_time(when);
    #[cfg(not(unix))]
    {
        let _ = when;
        String::new()
    }
}

/// Finder's Kind for an item, in the system language: "Microsoft Excel
/// Workbook", "Plain Text Document", "Application". Windows' Properties
/// shows the registered type name the same way (`SHGetFileInfoW` with
/// `SHGFI_TYPENAME` in `_InitSingleFilePrsht`). Reads metadata, so call it
/// off the UI thread. `None` on Linux or when the system has no name.
pub fn kind_name(path: &Path) -> Option<String> {
    #[cfg(unix)]
    return imp::kind_name(path);
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::time::{SystemTime, UNIX_EPOCH};

    use objc2::rc::Retained;
    use objc2_foundation::{NSDate, NSDateFormatter, NSDateFormatterStyle as Style, NSLocale, NSString, NSURL, NSURLLocalizedTypeDescriptionKey};

    use super::RelativeDay;

    pub fn kind_name(path: &std::path::Path) -> Option<String> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        let mut value = None;
        // SAFETY: the key is a valid static, and AnyObject fits any value.
        unsafe { url.getResourceValue_forKey_error(&mut value, NSURLLocalizedTypeDescriptionKey) }.ok()?;
        Some(value?.downcast::<NSString>().ok()?.to_string())
    }

    pub fn separators() -> (String, String) {
        let locale = NSLocale::currentLocale();
        (locale.groupingSeparator().to_string(), locale.decimalSeparator().to_string())
    }

    /// Formatters are slow to create, so each thread keeps its own.
    struct Formatters {
        long_date: Retained<NSDateFormatter>,
        weekday: Retained<NSDateFormatter>,
        relative: Retained<NSDateFormatter>,
        long_time: Retained<NSDateFormatter>,
        short_date: Retained<NSDateFormatter>,
        short_time: Retained<NSDateFormatter>,
    }

    fn styled(date: Style, time: Style) -> Retained<NSDateFormatter> {
        let f = NSDateFormatter::new();
        f.setDateStyle(date);
        f.setTimeStyle(time);
        f
    }

    thread_local! {
        static FORMATTERS: Formatters = Formatters {
            long_date: styled(Style::FullStyle, Style::NoStyle),
            weekday: {
                let f = NSDateFormatter::new();
                f.setLocalizedDateFormatFromTemplate(&NSString::from_str("EEEE"));
                f
            },
            relative: {
                let f = styled(Style::MediumStyle, Style::NoStyle);
                f.setDoesRelativeDateFormatting(true);
                f
            },
            long_time: styled(Style::NoStyle, Style::MediumStyle),
            short_date: styled(Style::ShortStyle, Style::NoStyle),
            short_time: styled(Style::NoStyle, Style::ShortStyle),
        };
    }

    fn ns_date(when: SystemTime) -> Retained<NSDate> {
        let secs = match when.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs_f64(),
            Err(e) => -e.duration().as_secs_f64(),
        };
        NSDate::dateWithTimeIntervalSince1970(secs)
    }

    /// The relative formatter knows the word in the user's language.
    pub fn long_date_time(when: SystemTime, relative: Option<RelativeDay>, with_time: bool) -> String {
        let date = ns_date(when);
        FORMATTERS.with(|f| {
            let mut s = f.long_date.stringFromDate(&date).to_string();
            if relative.is_some() {
                let word = f.relative.stringFromDate(&date).to_string();
                let weekday = f.weekday.stringFromDate(&date).to_string();
                s = if s.contains(&weekday) { s.replacen(&weekday, &word, 1) } else { format!("{word}, {s}") };
            }
            if with_time {
                s = format!("{s}, {}", f.long_time.stringFromDate(&date));
            }
            s
        })
    }

    pub fn short_date_time(when: SystemTime) -> String {
        let date = ns_date(when);
        FORMATTERS.with(|f| format!("{} {}", f.short_date.stringFromDate(&date), f.short_time.stringFromDate(&date)))
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::{CStr, CString};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::RelativeDay;

    pub fn kind_name(_path: &std::path::Path) -> Option<String> {
        None
    }

    pub fn separators() -> (String, String) {
        // SAFETY: called at startup before other threads; "" means the
        // environment's locale. localeconv's strings stay valid until the
        // next setlocale.
        unsafe {
            libc::setlocale(libc::LC_NUMERIC, c"".as_ptr());
            libc::setlocale(libc::LC_TIME, c"".as_ptr());
            let conv = &*libc::localeconv();
            let text = |p: *const libc::c_char| if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() };
            let decimal = text(conv.decimal_point);
            (text(conv.thousands_sep), if decimal.is_empty() { ".".into() } else { decimal })
        }
    }

    /// `strftime` in the `LC_TIME` locale, for local time.
    fn strftime(when: SystemTime, format: &str) -> String {
        let secs = match when.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as libc::time_t,
            Err(e) => -(e.duration().as_secs() as libc::time_t),
        };
        let Ok(format) = CString::new(format) else { return String::new() };
        let mut buf = [0u8; 128];
        // SAFETY: tm is plain old data; localtime_r fills it from a valid
        // time_t, and strftime writes at most buf.len() bytes.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&secs, &mut tm).is_null() {
                return String::new();
            }
            let n = libc::strftime(buf.as_mut_ptr().cast(), buf.len(), format.as_ptr(), &tm);
            String::from_utf8_lossy(&buf[..n]).into_owned()
        }
    }

    /// The C library has no long date format or words for today and
    /// yesterday: the weekday and short date stand in, and the words stay
    /// English.
    pub fn long_date_time(when: SystemTime, relative: Option<RelativeDay>, with_time: bool) -> String {
        let day = match relative {
            Some(RelativeDay::Today) => "Today".to_owned(),
            Some(RelativeDay::Yesterday) => "Yesterday".to_owned(),
            None => strftime(when, "%A"),
        };
        let mut s = format!("{day}, {}", strftime(when, "%x"));
        if with_time {
            s = format!("{s}, {}", strftime(when, "%X"));
        }
        s
    }

    pub fn short_date_time(when: SystemTime) -> String {
        strftime(when, "%x %R")
    }
}
