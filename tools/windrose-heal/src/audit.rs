use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::summary::Summary;

pub struct Audit {
    pub log_path: Option<PathBuf>,
    pub run_id: String,
}

#[derive(Debug, Serialize)]
struct AuditEntry<'a> {
    run_id: &'a str,
    timestamp: String,
    db_path: String,
    case: &'a str,
    strategy: &'a str,
    before: &'a Summary,
    after: &'a Summary,
}

impl Audit {
    pub fn new(log_path: Option<PathBuf>, run_id: String) -> Self {
        Self { log_path, run_id }
    }

    pub fn record(
        &self,
        db_path: &Path,
        case: &str,
        before: &Summary,
        after: &Summary,
        strategy: &str,
    ) -> Result<()> {
        let entry = AuditEntry {
            run_id: &self.run_id,
            timestamp: chrono::Utc::now().to_rfc3339(),
            db_path: db_path.display().to_string(),
            case,
            strategy,
            before,
            after,
        };
        let line = serde_json::to_string(&entry)?;
        match &self.log_path {
            Some(p) => {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
                    .with_context(|| format!("open audit log {:?}", p))?;
                writeln!(f, "{}", line)?;
            }
            None => {
                eprintln!("AUDIT: {}", line);
            }
        }
        Ok(())
    }
}
