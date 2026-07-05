//! CLI d'analyse hors ligne des archives.
//!
//! ```text
//! pm-replay strike-validate --legacy <window_dir/raw.ndjson> [--t0-ms N] [--expected X] [--max-lines N]
//! pm-replay strike-validate --journal <journal.ndjson>... --t0-ms N [--expected X]
//! pm-replay cadence --legacy <raw.ndjson> | --journal <journal.ndjson>...
//! ```
//!
//! `strike-validate` affiche le strike selon les trois politiques et l'écart
//! à la valeur affichée par Polymarket si fournie (`--expected`). C'est
//! l'outil qui tranchera la politique définitive sur les données réelles
//! (fenêtres 1778341500 → 80466.61 et 1778343900 → 80714.87).

use anyhow::{bail, Context, Result};
use pm_core::window;
use pm_replay::{compare_policies, tick_cadence};
use std::path::PathBuf;

#[derive(Debug, Default)]
struct Args {
    command: String,
    legacy: Option<PathBuf>,
    journals: Vec<PathBuf>,
    t0_ms: Option<u64>,
    expected: Option<f64>,
    max_lines: Option<usize>,
}

fn parse_args() -> Result<Args> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = argv.first() else {
        bail!("usage: pm-replay <strike-validate|cadence> [options]");
    };
    let mut args = Args {
        command: command.clone(),
        ..Default::default()
    };
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--legacy" => {
                i += 1;
                args.legacy = Some(PathBuf::from(
                    argv.get(i).context("--legacy: chemin manquant")?,
                ));
            }
            "--journal" => {
                i += 1;
                while i < argv.len() && !argv[i].starts_with("--") {
                    args.journals.push(PathBuf::from(&argv[i]));
                    i += 1;
                }
                continue;
            }
            "--t0-ms" => {
                i += 1;
                args.t0_ms = Some(argv.get(i).context("--t0-ms: valeur manquante")?.parse()?);
            }
            "--expected" => {
                i += 1;
                args.expected = Some(
                    argv.get(i)
                        .context("--expected: valeur manquante")?
                        .parse()?,
                );
            }
            "--max-lines" => {
                i += 1;
                args.max_lines = Some(
                    argv.get(i)
                        .context("--max-lines: valeur manquante")?
                        .parse()?,
                );
            }
            other => bail!("argument inconnu: {other}"),
        }
        i += 1;
    }
    Ok(args)
}

fn load_ticks(args: &Args) -> Result<(Vec<pm_core::ResolutionTick>, Option<u64>)> {
    if let Some(path) = &args.legacy {
        let data = pm_replay::legacy::load_legacy_ndjson(path, args.max_lines)?;
        eprintln!(
            "legacy: window={:?} ticks_resolution={} clob={} lignes_illisibles={}",
            data.window_slug,
            data.resolution_ticks.len(),
            data.clob_events.len(),
            data.unparsed_lines
        );
        let t0 = data.window_epoch.map(|e| window::window_bounds_ms(e).0);
        Ok((data.resolution_ticks, t0))
    } else if !args.journals.is_empty() {
        let paths: Vec<&std::path::Path> = args.journals.iter().map(PathBuf::as_path).collect();
        let data = pm_replay::v2::load_journal(&paths)?;
        eprintln!(
            "journal v2: frames={} ticks_resolution={} fast={} clob={} illisibles={}",
            data.frames,
            data.resolution_ticks.len(),
            data.fast_ticks.len(),
            data.clob_events.len(),
            data.unparsed_lines
        );
        Ok((data.resolution_ticks, None))
    } else {
        bail!("préciser --legacy <fichier> ou --journal <fichiers...>")
    }
}

fn main() -> Result<()> {
    let args = parse_args()?;
    match args.command.as_str() {
        "strike-validate" => {
            let (ticks, t0_from_file) = load_ticks(&args)?;
            let t0 = args
                .t0_ms
                .or(t0_from_file)
                .context("--t0-ms requis (non déductible du fichier)")?;
            let cmp = compare_policies(&ticks, t0);
            println!("T0 = {t0} ms");
            for (name, c) in [
                ("last_at_or_before (défaut)", &cmp.last_at_or_before),
                ("first_at_or_after        ", &cmp.first_at_or_after),
                ("interpolate (legacy py)  ", &cmp.interpolate),
            ] {
                let v = c
                    .value
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "N/A".into());
                let delta = match (c.value, args.expected) {
                    (Some(v), Some(e)) => format!(" | écart vs affiché: {:+.2}", v - e),
                    _ => String::new(),
                };
                println!(
                    "  {name}: {v} (status={:?}, confidence={:.3}, gap={:?} ms){delta}",
                    c.status, c.confidence, c.used_gap_ms
                );
            }
            if let Some(e) = args.expected {
                println!("valeur affichée Polymarket: {e:.2}");
            }
        }
        "cadence" => {
            let (ticks, _) = load_ticks(&args)?;
            let c = tick_cadence(&ticks);
            println!("ticks résolution: {}", c.count);
            println!(
                "inter-arrivée ms: min={} p50={} moy={:.1} p99={} max={}",
                c.min_dt_ms, c.p50_dt_ms, c.mean_dt_ms, c.p99_dt_ms, c.max_dt_ms
            );
            println!(
                "latence médiane réception-source: {} ms",
                c.median_recv_lag_ms
            );
        }
        other => bail!("commande inconnue: {other} (attendu: strike-validate | cadence)"),
    }
    Ok(())
}
