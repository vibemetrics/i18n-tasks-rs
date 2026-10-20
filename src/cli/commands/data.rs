//! ref: lib/i18n/tasks/command/commands/data.rb:77-95. `data-merge` writes,
//! as the gem's does; an exception to blocker B8.

use crate::cli::args::{Common, Format};
use crate::cli::commands::report_deletions;
use crate::cli::exit::ExitStatus;
use crate::cli::out::{out, outln};
use i18n_tasks_rs::report::{data, normalize};
use std::io::Read;

#[derive(clap::Args, Clone, Debug)]
pub(crate) struct DataArgs {
    #[command(flatten)]
    common: Common,
}

impl DataArgs {
    pub(crate) fn run(&self) -> Result<ExitStatus, String> {
        let s = self.common.open()?;
        if s.json {
            outln!("{}", data::tree(&s.store, &s.locales));
        } else {
            out!("{}", data::to_yaml(&s.store, &s.locales));
        }
        Ok(ExitStatus::Ok)
    }
}

#[derive(clap::Args, Clone, Debug)]
pub(crate) struct DataMergeArgs {
    #[command(flatten)]
    common: Common,
    /// Allow deleting a file that ends up with no keys.
    #[arg(long)]
    allow_delete: bool,
}

impl DataMergeArgs {
    pub(crate) fn run(&self) -> Result<ExitStatus, String> {
        if self.common.format() == Format::Text {
            return Err("data-merge reads and prints JSON. Pass `-f json`.".into());
        }
        if !self.common.requested_locales().is_empty() {
            return Err("data-merge takes no locales: the tree on stdin names them".into());
        }
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| format!("cannot read stdin: {e}"))?;
        let input: serde_json::Value = serde_json::from_str(&input)
            .map_err(|e| format!("the tree on stdin is not JSON: {e}"))?;

        let mut s = self.common.open()?;
        let (report, locales) = data::merge(&s.cfg, &mut s.store, &input)?;
        let deletions = report.deletions();
        report_deletions(&deletions);
        if !deletions.is_empty() && !self.allow_delete {
            return Err(
                "refusing to delete the files listed above. Pass `--allow-delete` to allow it."
                    .into(),
            );
        }
        normalize::apply(&report)?;
        outln!("{}", data::tree(&s.store, &locales));
        Ok(ExitStatus::Ok)
    }
}
