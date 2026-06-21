use crate::domain::UiLanguage;

pub(super) const APP_NAME: &str = "j3GridDocker";
pub(super) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(super) const PROJECT_URL: &str = env!("CARGO_PKG_REPOSITORY");

pub(super) fn about_version_label_text() -> String {
    format!("{APP_NAME} {APP_VERSION}")
}

pub(super) fn about_window_title_text() -> String {
    format!("About {APP_NAME}")
}

pub(super) fn about_notice_text(_language: UiLanguage) -> String {
    include_str!("../../about.txt").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_notice_includes_gpl_and_source_information() {
        let english = about_notice_text(UiLanguage::English);
        assert!(english.contains("j3GridDocker"));
        assert!(english.contains("GPL-3.0-or-later"));
        assert!(english.contains("LICENSE"));
        assert!(english.contains("Source Code"));
        assert!(english.contains(PROJECT_URL));
        assert_eq!(
            about_version_label_text(),
            format!("j3GridDocker {APP_VERSION}")
        );

        assert_eq!(about_notice_text(UiLanguage::Korean), english);
    }
}
