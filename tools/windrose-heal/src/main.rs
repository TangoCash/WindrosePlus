use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod audit;
mod heal;
mod levels;
mod player_db;
mod summary;
mod zip_repair;

#[derive(Parser)]
#[command(name = "windrose-heal")]
#[command(about = "Surgical repair tool for drifted Windrose client character saves")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    #[arg(long, global = true, default_value = "info")]
    log_level: String,

    #[arg(long, global = true)]
    audit_log: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Read-only inspection — parse the save, print progression fields and heal diagnosis
    Inspect {
        /// Path to a Players/<PlayerId> RocksDB directory, or a SaveProfiles/<steamid> tree
        #[arg(long)]
        input: PathBuf,
    },

    /// Apply Case A heal (zero progression drift, no spent points) and write back
    Repair {
        /// Path to a Players/<PlayerId> RocksDB directory
        #[arg(long)]
        input: PathBuf,

        /// Strategy: "safe" (refuse if spent points detected) or "force-reset" (zero trees)
        #[arg(long, default_value = "safe")]
        strategy: String,
    },

    /// Round-trip self-test — parse and re-emit, assert byte-identical
    Roundtrip {
        #[arg(long)]
        input: PathBuf,
    },

    /// Repair an uploaded SaveProfiles zip and write a repaired zip
    RepairZip {
        /// Path to a .zip containing SaveProfiles/<steamid>/RocksDB/0.10.0/Players
        #[arg(long)]
        input: PathBuf,

        /// Destination .zip path for the repaired copy
        #[arg(long)]
        output: PathBuf,

        /// Strategy: "safe" refuses spent-point saves; "force-reset" zeroes spent trees
        #[arg(long, default_value = "safe")]
        strategy: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = tracing_subscriber::EnvFilter::try_new(&cli.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let run_id = uuid::Uuid::new_v4().to_string();
    let audit = audit::Audit::new(cli.audit_log.clone(), run_id.clone());

    match cli.command {
        Command::Inspect { input } => {
            tracing::info!(run_id = %run_id, path = ?input, "inspect");
            cmd_inspect(&input).with_context(|| format!("inspect {:?}", input))?;
        }
        Command::Repair { input, strategy } => {
            tracing::info!(run_id = %run_id, path = ?input, strategy = %strategy, "repair");
            cmd_repair(&input, &strategy, &audit).with_context(|| format!("repair {:?}", input))?;
        }
        Command::Roundtrip { input } => {
            tracing::info!(run_id = %run_id, path = ?input, "roundtrip");
            cmd_roundtrip(&input).with_context(|| format!("roundtrip {:?}", input))?;
        }
        Command::RepairZip {
            input,
            output,
            strategy,
        } => {
            tracing::info!(run_id = %run_id, input = ?input, output = ?output, strategy = %strategy, "repair zip");
            let strategy = heal::Strategy::parse(&strategy)?;
            let report = zip_repair::repair_zip(&input, &output, strategy, &audit)
                .with_context(|| format!("repair zip {:?}", input))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }

    Ok(())
}

fn cmd_inspect(path: &std::path::Path) -> Result<()> {
    let db_path = player_db::resolve_player_db(path)?;
    let (key, value) = player_db::read_single_r5bl_player_row(&db_path)?;
    let doc = player_db::parse_bson(&value)?;
    player_db::assert_roundtrip(&doc, &value)?;

    let s = summary::Summary::from_doc(&doc, &key, value.len())?;
    println!("{}", serde_json::to_string_pretty(&s)?);

    let levels = levels::vanilla_da_hero_levels();
    let diagnosis = heal::diagnose(&doc, &levels)?;
    println!("\n=== Diagnosis ===");
    println!("{}", serde_json::to_string_pretty(&diagnosis)?);

    Ok(())
}

fn cmd_repair(path: &std::path::Path, strategy: &str, audit: &audit::Audit) -> Result<()> {
    let db_path = player_db::resolve_player_db(path)?;
    let (key, value_before) = player_db::read_single_r5bl_player_row(&db_path)?;
    let doc_before = player_db::parse_bson(&value_before)?;
    player_db::assert_roundtrip(&doc_before, &value_before)?;

    let levels = levels::vanilla_da_hero_levels();
    let diagnosis = heal::diagnose(&doc_before, &levels)?;
    tracing::info!(?diagnosis, "pre-repair diagnosis");

    let strategy_enum = heal::Strategy::parse(strategy)?;

    let (doc_after, case_applied) =
        heal::apply_heal(doc_before.clone(), &diagnosis, strategy_enum)?;
    let value_after = player_db::emit_bson(&doc_after)?;

    // Re-decode our output as sanity check
    let doc_reparsed = player_db::parse_bson(&value_after)?;
    player_db::assert_roundtrip(&doc_reparsed, &value_after)?;

    // Write back
    player_db::write_r5bl_player_row(&db_path, &key, &value_after)?;

    // Post-write verification
    let (_k2, value_verify) = player_db::read_single_r5bl_player_row(&db_path)?;
    if value_verify != value_after {
        anyhow::bail!("post-write verification failed: reread value does not match what we wrote");
    }

    let summary_before = summary::Summary::from_doc(&doc_before, &key, value_before.len())?;
    let summary_after = summary::Summary::from_doc(&doc_after, &key, value_after.len())?;

    audit.record(
        &db_path,
        &case_applied,
        &summary_before,
        &summary_after,
        strategy,
    )?;

    println!("=== Repair applied ===");
    println!("Case: {}", case_applied);
    println!("Before: {}", serde_json::to_string_pretty(&summary_before)?);
    println!("After:  {}", serde_json::to_string_pretty(&summary_after)?);

    Ok(())
}

fn cmd_roundtrip(path: &std::path::Path) -> Result<()> {
    let db_path = player_db::resolve_player_db(path)?;
    let (key, value) = player_db::read_single_r5bl_player_row(&db_path)?;
    let doc = player_db::parse_bson(&value)?;
    player_db::assert_roundtrip(&doc, &value)?;
    println!(
        "roundtrip OK  key={} value_len={}",
        hex::encode(&key),
        value.len()
    );
    Ok(())
}
