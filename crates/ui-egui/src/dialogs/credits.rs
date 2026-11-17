//! The About window's data: every contributor (GitHub login always; display and real names only with recorded
//! consent) and the AI models that helped. build.rs bakes the tables into the binary from
//! contributors/contributors.json; nothing is read at run time. The framework's `AboutDialog` draws them.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{AboutInfo, AboutLink, AboutTab, Contributor as CreditedPerson, Credits, HostedAbout, HostedReport, ModelCredit};

use crate::PhotocraftApp;

/// One contributor row as build.rs bakes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contributor {
    /// The GitHub login.
    pub login: &'static str,
    /// The display name, present only with recorded consent.
    pub display_name: Option<&'static str>,
    /// The real name, present only with recorded consent.
    pub real_name: Option<&'static str>,
    /// Merged pull requests.
    pub prs: u64,
    /// Commits.
    pub commits: u64,
    /// Lines added.
    pub lines_added: u64,
    /// Lines deleted.
    pub lines_deleted: u64,
    /// Binary assets added.
    pub binary_added: u64,
    /// Binary assets deleted.
    pub binary_deleted: u64,
    /// ISO 8601 UTC; empty for someone credited only through a merged PR.
    pub first_commit: &'static str,
    /// ISO 8601 UTC of the last commit; empty like `first_commit`.
    pub last_commit: &'static str,
}

/// One AI model row as build.rs bakes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    /// The company that made the model.
    pub company: &'static str,
    /// The model's name.
    pub model: &'static str,
    /// The model's version.
    pub version: &'static str,
    /// Commits the model took part in.
    pub commits: u64,
    /// Lines added.
    pub lines_added: u64,
    /// Lines deleted.
    pub lines_deleted: u64,
}

include!(concat!(env!("OUT_DIR"), "/credits.rs"));

/// The framework's credits rows, built from the baked tables.
pub fn credits() -> Credits {
    let people: Vec<CreditedPerson> = CONTRIBUTORS
        .iter()
        .map(|c| {
            let mut person = CreditedPerson::new(c.login)
                .counts(c.prs, c.commits)
                .lines(c.lines_added, c.lines_deleted)
                .binary(c.binary_added, c.binary_deleted)
                .active(c.first_commit, c.last_commit);
            if let Some(name) = c.display_name {
                person = person.display_name(name);
            }
            if let Some(name) = c.real_name {
                person = person.real_name(name);
            }
            person
        })
        .collect();
    let models: Vec<ModelCredit> =
        MODELS.iter().map(|m| ModelCredit::new(m.company, m.model, m.version).work(m.commits, m.lines_added, m.lines_deleted)).collect();
    Credits::new(people, models, TOTAL_COMMITS)
}

/// The About tab's contents: name, blurb, version, the community button and the project links.
pub fn about_info(_app: &PhotocraftApp) -> AboutInfo {
    AboutInfo::new("PhotoCraft", tl!("PhotoCraft — an open-source, native image editor written in Rust."), photocraft_engine::build_info::long_version())
        .community(AboutLink::new(tl!("Join us on Discord"), crate::links::DISCORD))
        .links([
            AboutLink::new(tl!("PhotoCraft website"), crate::links::APP_PAGE),
            AboutLink::new(tl!("GitHub"), crate::links::GITHUB),
            AboutLink::new(tl!("ArtCraft"), crate::links::ARTCRAFT_WEBSITE),
        ])
        .built_with("egui · wgpu · photocraft-engine")
}

/// Opens the About dialog on `tab`.
pub fn open_about(app: &mut PhotocraftApp, tab: AboutTab) -> DialogId {
    let dialog = HostedAbout::new(about_info(app), credits()).tab(tab);
    app.framework.dialogs.open(dialog)
}

/// Opens Help › System Info: the version line as the summary, the platform and graphics facts as lines.
pub fn open_system_info(app: &mut PhotocraftApp) -> DialogId {
    let mut lines = crate::gpu_status::system_info_ui(app);
    let summary = if lines.is_empty() { String::new() } else { lines.remove(0) };
    app.framework.dialogs.open(HostedReport::new(tl!("System Info"), summary, lines))
}

/// The result pump (a `TAKERS` row): opens a link the About dialog took and copies System Info. Each call takes only
/// what the dialogs filed since the last call.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(url) = app.framework.dialogs.get_mut::<HostedAbout>().and_then(HostedAbout::take_link) {
        crate::links::open(app, ctx, &url);
    }
    // Issue reports take the English lines whatever the interface language, as the Copy button always has.
    if app.framework.dialogs.get_mut::<HostedReport>().and_then(HostedReport::take_copied).is_some() {
        ctx.copy_text(crate::gpu_status::system_info(app).join("\n"));
    }
}
