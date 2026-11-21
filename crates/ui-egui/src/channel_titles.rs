//! Photo's native/drawn caption rules, with an explicitly selected channel name.

pub(crate) fn os_title(app_name: &str, document: Option<(&str, bool)>, show_file_name: bool) -> String {
    let Some((name, dirty)) = document else { return app_name.into() };
    let prefix = if dirty { "*" } else { "" };
    if show_file_name { format!("{prefix}{name} \u{2014} {app_name}") } else { format!("{prefix}{app_name}") }
}

pub(crate) fn drawn_title(app_name: &str, document: Option<(&str, bool)>, show_file_name: bool, mark_document_channel: bool) -> String {
    let Some((name, dirty)) = document else { return app_name.into() };
    let label = if show_file_name { name } else { app_name };
    let channel = if show_file_name && mark_document_channel { format!(" \u{2014} {app_name}") } else { String::new() };
    format!("{label}{channel}{}", if dirty { "  •" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nightly_native_caption_keeps_channel_across_document_dirty_and_hidden_name_updates() {
        let channel = "PhotoCraft Nightly";
        assert_eq!(os_title(channel, None, true), channel);
        for dirty in [false, true] {
            let shown = os_title(channel, Some(("my document.psd", dirty)), true);
            assert!(shown.contains("my document.psd"));
            assert!(shown.ends_with(channel));
            assert_eq!(shown.starts_with('*'), dirty);
            let hidden = os_title(channel, Some(("my document.psd", dirty)), false);
            assert!(hidden.ends_with(channel));
            assert!(!hidden.contains("my document.psd"));
            assert_eq!(hidden.starts_with('*'), dirty);
        }
    }

    #[test]
    fn drawn_nightly_caption_keeps_channel_when_a_document_is_open() {
        let channel = "PhotoCraft Nightly";
        for show_name in [false, true] {
            for dirty in [false, true] {
                let title = drawn_title(channel, Some(("work.psd", dirty)), show_name, true);
                assert!(title.contains(channel));
                assert_eq!(title.contains("work.psd"), show_name);
                assert_eq!(title.ends_with('•'), dirty);
            }
        }
    }

    #[test]
    fn stable_drawn_caption_preserves_the_document_only_style_and_dirty_dot() {
        let stable = "PhotoCraft";
        assert_eq!(drawn_title(stable, None, true, false), stable);
        let clean = drawn_title(stable, Some(("work.psd", false)), true, false);
        assert_eq!(clean, "work.psd");
        let dirty = drawn_title(stable, Some(("work.psd", true)), true, false);
        assert!(dirty.starts_with(&clean));
        assert!(dirty.ends_with('•'));
        assert!(!dirty.contains(stable));
        let hidden = drawn_title(stable, Some(("work.psd", true)), false, false);
        assert!(hidden.starts_with(stable));
        assert!(hidden.ends_with('•'));
        assert!(!hidden.contains("work.psd"));
    }
}
