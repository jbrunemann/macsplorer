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

/// The locale in use, for the About box: the identifier dates and numbers
/// follow, and the collation names sort by.
#[cfg(unix)]
static SUMMARY: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// "en_DE (sorting: en)"; blank before `install` and on other systems.
pub fn summary() -> String {
    #[cfg(unix)]
    return SUMMARY.get().cloned().unwrap_or_default();
    #[cfg(not(unix))]
    String::new()
}

/// Reads the number separators and the sort order. Call once at startup,
/// before any thread starts (Linux's `setlocale` isn't thread-safe).
#[cfg(unix)]
pub fn install() {
    let collation = imp::collation();
    let _ = SUMMARY.set(format!("{} (sorting: {})", imp::identifier(), if collation.is_empty() { "root" } else { &collation }));
    macsplorer_core::natural::set_collation_locale(&collation);
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

    /// The language names sort in: the first preferred language, or the
    /// List sort order chosen in System Settings > Language & Region.
    pub fn collation() -> String {
        NSLocale::currentLocale().collatorIdentifier().to_string()
    }

    /// The locale identifier, like "en_DE" (language and region).
    pub fn identifier() -> String {
        NSLocale::currentLocale().localeIdentifier().to_string()
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

#[cfg(linux_desktop)]
mod imp {
    use std::ffi::{CStr, CString};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::RelativeDay;

    pub fn kind_name(_path: &std::path::Path) -> Option<String> {
        None
    }

    /// What `LC_ALL`, `var` or `LANG` says, like "de_DE.UTF-8". Termux's C library (Bionic) knows only the C
    /// locale, so `setlocale` can't be asked there.
    #[cfg(target_os = "android")]
    fn from_environment(var: &str) -> Option<String> {
        ["LC_ALL", var, "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|l| !l.is_empty()))
    }

    /// `LC_COLLATE` from the environment, like "sv_SE.UTF-8".
    pub fn collation() -> String {
        #[cfg(target_os = "android")]
        if let Some(name) = from_environment("LC_COLLATE") {
            return name;
        }
        // SAFETY: called at startup before other threads; the returned
        // string stays valid until the next setlocale.
        unsafe {
            let name = libc::setlocale(libc::LC_COLLATE, c"".as_ptr());
            if name.is_null() { String::new() } else { CStr::from_ptr(name).to_string_lossy().into_owned() }
        }
    }

    /// `LC_TIME` from the environment, like "de_DE.UTF-8".
    pub fn identifier() -> String {
        #[cfg(target_os = "android")]
        if let Some(name) = from_environment("LC_TIME") {
            return name;
        }
        // SAFETY: called at startup before other threads; the returned
        // string stays valid until the next setlocale.
        unsafe {
            let name = libc::setlocale(libc::LC_TIME, c"".as_ptr());
            if name.is_null() { "C".into() } else { CStr::from_ptr(name).to_string_lossy().into_owned() }
        }
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
    /// yesterday: the weekday and short date stand in. Words stay English
    /// like the rest of the app (`strftime`'s `%A` would be the locale's
    /// language); the order and separators of the numbers are the locale's.
    pub fn long_date_time(when: SystemTime, relative: Option<RelativeDay>, with_time: bool) -> String {
        let day = match relative {
            Some(RelativeDay::Today) => "Today".to_owned(),
            Some(RelativeDay::Yesterday) => "Yesterday".to_owned(),
            None => chrono::DateTime::<chrono::Local>::from(when).format("%A").to_string(),
        };
        let mut s = format!("{day}, {}", strftime(when, "%x"));
        if with_time {
            s = format!("{s}, {}", strftime(when, "%X"));
        }
        s
    }

    pub fn short_date_time(when: SystemTime) -> String {
        strftime(when, &format!("%x {}", short_time_format()))
    }

    /// The locale's time without seconds: 12-hour where its time format
    /// is ("3:04 PM" in en_US), else 24-hour ("15:04").
    fn short_time_format() -> &'static str {
        // Bionic has no nl_langinfo in the libc crate, and only the C locale: 24-hour.
        #[cfg(target_os = "android")]
        return "%R";
        #[cfg(not(target_os = "android"))]
        {
            // SAFETY: nl_langinfo's string stays valid until the next setlocale,
            // which only runs at startup.
            let t_fmt = unsafe {
                let p = libc::nl_langinfo(libc::T_FMT);
                if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
            };
            if ["%r", "%I", "%l", "%p"].iter().any(|f| t_fmt.contains(f)) { "%-I:%M %p" } else { "%R" }
        }
    }
}

/// Android's C library knows only the C locale: the phone's region settings
/// come from Java once (`android::region`), and chrono formats with them.
#[cfg(android_app)]
mod imp {
    use std::sync::OnceLock;
    use std::time::SystemTime;

    use chrono::{DateTime, Local};

    use super::{RelativeDay, icu_to_chrono};
    use crate::android::Region;

    static REGION: OnceLock<Option<Region>> = OnceLock::new();

    fn region() -> Option<&'static Region> {
        REGION.get_or_init(crate::android::region).as_ref()
    }

    pub fn kind_name(_path: &std::path::Path) -> Option<String> {
        None
    }

    pub fn collation() -> String {
        region().map(|r| r.tag.clone()).unwrap_or_default()
    }

    pub fn identifier() -> String {
        region().map_or_else(|| "C".into(), |r| r.tag.clone())
    }

    pub fn separators() -> (String, String) {
        region().map_or_else(|| (",".into(), ".".into()), |r| (r.group.clone(), r.decimal.clone()))
    }

    fn format(when: SystemTime, pattern: &str) -> String {
        DateTime::<Local>::from(when).format(pattern).to_string()
    }

    /// Today and Yesterday take the weekday's place, as on the Mac.
    pub fn long_date_time(when: SystemTime, relative: Option<RelativeDay>, with_time: bool) -> String {
        let (date, time) = region().map_or(("%A, %x", "%X"), |r| (r.long_date.as_str(), r.long_time.as_str()));
        let mut date = icu_to_chrono(date);
        if let Some(day) = relative {
            let word = match day {
                RelativeDay::Today => "Today",
                RelativeDay::Yesterday => "Yesterday",
            };
            date = if date.contains("%A") { date.replacen("%A", word, 1) } else { format!("{word}, {date}") };
        }
        let mut s = format(when, &date);
        if with_time {
            s = format!("{s}, {}", format(when, &icu_to_chrono(time)));
        }
        s
    }

    pub fn short_date_time(when: SystemTime) -> String {
        match region() {
            Some(r) => format(when, &format!("{} {}", icu_to_chrono(&r.short_date), icu_to_chrono(&r.short_time))),
            None => format(when, "%x %R"),
        }
    }
}

/// An ICU date pattern (`dd.MM.y`, `h:mm a`, `EEEE, MMMM d, y`) as a chrono
/// format string. Names come out in English, like every word of the app.
/// Letters chrono has no counterpart for are left out.
#[cfg(any(android_app, test))]
fn icu_to_chrono(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut quoted = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // '' is a quote, in or out of quoted text; a single ' starts or ends literal text.
        if c == '\'' {
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
            } else {
                quoted = !quoted;
                i += 1;
            }
            continue;
        }
        if quoted || !c.is_ascii_alphabetic() {
            out.push_str(if c == '%' { "%%" } else { "" });
            if c != '%' {
                out.push(c);
            }
            i += 1;
            continue;
        }
        let run = chars[i..].iter().take_while(|&&d| d == c).count();
        out.push_str(match (c, run) {
            ('y', 2) => "%y",
            ('y', _) => "%Y",
            ('M' | 'L', 1) => "%-m",
            ('M' | 'L', 2) => "%m",
            ('M' | 'L', 3) => "%b",
            ('M' | 'L', _) => "%B",
            ('d', 1) => "%-d",
            ('d', _) => "%d",
            ('E' | 'c', 4..) => "%A",
            ('E' | 'c', _) => "%a",
            ('H', 1) => "%-H",
            ('H', _) => "%H",
            ('h', 1) => "%-I",
            ('h', _) => "%I",
            ('m', _) => "%M",
            ('s', _) => "%S",
            ('a' | 'b' | 'B', _) => "%p",
            _ => "",
        });
        i += run;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::icu_to_chrono;

    #[test]
    fn icu_patterns_become_chrono_formats() {
        assert_eq!(icu_to_chrono("M/d/y"), "%-m/%-d/%Y");
        assert_eq!(icu_to_chrono("dd.MM.y"), "%d.%m.%Y");
        assert_eq!(icu_to_chrono("h:mm a"), "%-I:%M %p");
        assert_eq!(icu_to_chrono("HH:mm:ss"), "%H:%M:%S");
        assert_eq!(icu_to_chrono("EEEE, MMMM d, y"), "%A, %B %-d, %Y");
        assert_eq!(icu_to_chrono("EEEE d MMMM y 'г'."), "%A %-d %B %Y г.");
        assert_eq!(icu_to_chrono("d 'de' MMMM 'de' y"), "%-d de %B de %Y");
        assert_eq!(icu_to_chrono("h 'o''clock' 100%"), "%-I o'clock 100%%");
    }
}
