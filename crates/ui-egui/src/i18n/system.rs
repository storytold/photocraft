//! The system language and number format, read through the framework's craft-i18n.

use super::Lang;
#[cfg(test)]
use super::lang_from_tag;

/// Resolve Auto against the current registry: the framework reads the system's language
/// preferences once and matches them against the offered languages (English when none fits).
pub fn system_lang() -> Lang {
    #[cfg(test)]
    {
        TEST_SYSTEM_TAGS.with(|tags| resolve(&tags.borrow()))
    }
    #[cfg(not(test))]
    {
        let i18n = super::framework_i18n();
        Lang::from_code(i18n.code(i18n.system_lang())).unwrap_or(Lang::EN)
    }
}

/// Tests resolve their own tag list so they do not depend on the developer's locale.
#[cfg(test)]
fn resolve(tags: &[String]) -> Lang {
    tags.iter().find_map(|tag| lang_from_tag(tag)).unwrap_or(Lang::EN)
}

/// The decimal separator of the system's number format, when it is `.` or `,`: Photoshop's numeric
/// fields take only that one (a German system refuses a typed `.`). `None` where the system sets
/// none or another symbol: the fields then take both.
pub fn decimal_separator() -> Option<char> {
    // Tests take both separators, whatever the developer's own number format is.
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        craft::i18n::Region::system().decimal.filter(|c| matches!(c, '.' | ','))
    }
}

#[cfg(test)]
thread_local! {
    // Keep ordinary tests independent of the developer's locale, without mutating OS or
    // process-global preferences. Startup tests supply the OS result on their own thread.
    static TEST_SYSTEM_TAGS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(super) fn with_system_tags<R>(tags: &[&str], run: impl FnOnce() -> R) -> R {
    struct Restore(Vec<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_SYSTEM_TAGS.set(std::mem::take(&mut self.0));
        }
    }
    let _restore = Restore(TEST_SYSTEM_TAGS.replace(tags.iter().map(|tag| (*tag).to_owned()).collect()));
    run()
}
