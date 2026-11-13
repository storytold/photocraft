//! `photocraft-cli models list|fetch`: the command-line face of `photocraft_ml::download`
//! (Settings › AI Models is the other).

use std::io::Write;

use photocraft_ml::{Model, model_path, models_dir};

use crate::Result;
use crate::args::{ModelChoice, ModelsAction, ModelsArgs};

fn models(choice: ModelChoice) -> Vec<Model> {
    match choice {
        ModelChoice::All => Model::ALL.to_vec(),
        ModelChoice::Birefnet => vec![Model::BirefnetLite],
        ModelChoice::Lama => vec![Model::Lama],
    }
}

pub(crate) fn run(args: &ModelsArgs, out: &mut dyn Write) -> Result {
    match args.action {
        ModelsAction::List => {
            let dir = models_dir();
            for model in Model::ALL {
                let path = dir
                    .as_ref()
                    .map_or_else(|| format!("{} (models directory unavailable)", model.file_name()), |dir| dir.join(model.file_name()).display().to_string());
                let installed = if model_path(model).is_some() { "yes" } else { "no" };
                writeln!(out, "{}\t{}\t{path}\tinstalled: {installed}", model.label(), model.licence())?;
            }
        }
        ModelsAction::Fetch { model } => {
            for model in models(model) {
                // Progress goes to stderr in 10% steps, so stdout stays one line per model.
                let shown = std::cell::Cell::new(0u64);
                let progress = |done: u64, total: u64| {
                    let pct = done.saturating_mul(100).checked_div(total).unwrap_or(0);
                    if pct >= shown.get() + 10 || (done == total && shown.get() < 100) {
                        shown.set(pct);
                        eprintln!("{}: {pct}%", model.label());
                    }
                };
                let path = photocraft_ml::download(model, &progress, &|| false).map_err(|e| crate::CliError::Other(format!("{}: {e}", model.label())))?;
                writeln!(out, "Installed {} at {}", model.label(), path.display())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{Command, parse};

    fn action(words: &[&str]) -> std::result::Result<ModelsAction, String> {
        let args = std::iter::once("models").chain(words.iter().copied()).map(std::ffi::OsString::from).collect::<Vec<_>>();
        match parse(&args).map_err(|e| e.to_string())?.command {
            Command::Models(m) => Ok(m.action),
            _ => Err("parsed as another subcommand".into()),
        }
    }

    fn fetched(words: &[&str]) -> Vec<Model> {
        match action(words).unwrap() {
            ModelsAction::Fetch { model } => models(model),
            ModelsAction::List => panic!("{words:?} parsed as list"),
        }
    }

    #[test]
    fn parses_models_actions() {
        assert!(matches!(action(&["list"]).unwrap(), ModelsAction::List));
        assert_eq!(fetched(&["fetch"]), Model::ALL.to_vec());
        assert_eq!(fetched(&["fetch", "--model", "all"]), Model::ALL.to_vec());
        assert_eq!(fetched(&["fetch", "--model", "birefnet"]), vec![Model::BirefnetLite]);
        assert_eq!(fetched(&["fetch", "--model=lama"]), vec![Model::Lama]);
    }

    #[test]
    fn rejects_invalid_arguments_without_network() {
        for words in [
            vec![],
            vec!["unknown"],
            vec!["fetch", "extra"],
            vec!["list", "--model", "all"],
            vec!["fetch", "--model"],
            vec!["fetch", "--model", "other"],
            vec!["fetch", "--model="],
            vec!["fetch", "--unknown"],
            vec!["fetch", "--model", "lama", "--model", "all"],
        ] {
            assert!(action(&words).is_err(), "{words:?}");
        }
    }

    #[test]
    fn list_reports_model_metadata_and_loader_status() {
        let mut out = Vec::new();
        run(&ModelsArgs { action: ModelsAction::List }, &mut out).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert_eq!(output.lines().count(), Model::ALL.len());
        for model in Model::ALL {
            let status = if model_path(model).is_some() { "yes" } else { "no" };
            let line = output.lines().find(|line| line.contains(model.label())).unwrap();
            assert!(line.contains(model.licence()));
            assert!(line.contains(model.file_name()));
            assert!(line.ends_with(&format!("installed: {status}")));
        }
    }
}
