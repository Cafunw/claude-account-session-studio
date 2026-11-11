//! Language of the backend's user-facing strings (tray, notifications, progress steps, errors).
//! The effective language is kept in one global, set from `AutomationSettings::language` at
//! startup and by `set_language`; every string picks its wording with `t("türkçe", "english")`.
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Tr,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::Tr => "tr",
            Lang::En => "en",
        }
    }
}

static ENGLISH: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    static TEST_LANG: std::cell::Cell<Option<Lang>> = const { std::cell::Cell::new(None) };
}

/// Tests run in parallel threads, so each one pins its own language instead of touching the global.
#[cfg(test)]
pub fn set_test_lang(lang: Lang) {
    TEST_LANG.with(|c| c.set(Some(lang)));
}

/// "tr" / "en" are explicit; anything else ("auto") follows the Windows display language.
pub fn resolve(setting: &str, os_turkish: bool) -> Lang {
    match setting {
        "tr" => Lang::Tr,
        "en" => Lang::En,
        _ if os_turkish => Lang::Tr,
        _ => Lang::En,
    }
}

/// True when the Windows UI language's primary language is Turkish (LANG_TURKISH = 0x1F).
pub fn os_is_turkish() -> bool {
    #[cfg(windows)]
    {
        let langid = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
        langid & 0x3ff == 0x1f
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Resolves `setting` and makes it the language of every backend string from now on.
pub fn apply(setting: &str) -> Lang {
    let lang = resolve(setting, os_is_turkish());
    ENGLISH.store(lang == Lang::En, Ordering::Relaxed);
    lang
}

pub fn lang() -> Lang {
    #[cfg(test)]
    if let Some(l) = TEST_LANG.with(|c| c.get()) {
        return l;
    }
    if ENGLISH.load(Ordering::Relaxed) {
        Lang::En
    } else {
        Lang::Tr
    }
}

/// Picks the wording for the current language.
pub fn t<T>(tr: T, en: T) -> T {
    match lang() {
        Lang::Tr => tr,
        Lang::En => en,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_auto_tr_en() {
        assert_eq!(resolve("tr", false), Lang::Tr);
        assert_eq!(resolve("en", true), Lang::En);
        assert_eq!(resolve("auto", true), Lang::Tr);
        assert_eq!(resolve("auto", false), Lang::En);
        assert_eq!(resolve("", false), Lang::En, "unknown values behave like auto");
    }

    #[test]
    fn t_follows_the_pinned_language() {
        set_test_lang(Lang::En);
        assert_eq!(t("Çıkış", "Quit"), "Quit");
        set_test_lang(Lang::Tr);
        assert_eq!(t("Çıkış", "Quit"), "Çıkış");
    }
}
