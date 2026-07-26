//! Runner de benchmark : joue des campagnes, agrège les métriques (README §6),
//! sort un CSV et un tableau markdown.
//!
//! ```sh
//! cargo run -p runner                                  # random + greedy, 3 seeds x 3 runs
//! cargo run -p runner -- --agent llm:gemma@127.0.0.1:8080 --max-waves 12
//! cargo run -p runner -- --agent llm:gemma@127.0.0.1:8080 --mode detailed
//! cargo run -p runner -- balance                       # table mono-tourelle (équilibrage)
//! ```

mod agents;
mod llm;

use agents::{Agent, Greedy, Random};
use engine::*;
use std::fmt::Write as _;

const ERROR_CODES: [ErrorCode; 7] = [
    ErrorCode::PathBlocked,
    ErrorCode::InsufficientGold,
    ErrorCode::CellOccupied,
    ErrorCode::NoMovesLeft,
    ErrorCode::InvalidCell,
    ErrorCode::WrongPhase,
    ErrorCode::UnknownBuilding,
];

struct RunResult {
    agent: String,
    seed: u64,
    run: u32,
    wave: u32,
    actions: u32,
    errors: [u32; 7],
    forced: u32,
    moves: u32,
    refunded: u32,
    parse_failures: u32,
    tokens: u64,
}

impl RunResult {
    fn illegal(&self) -> u32 {
        self.errors.iter().sum()
    }
}

/// Une partie complète. L'agent ne peut agir que par `Action` : même canal,
/// mêmes limites et mêmes métriques que par l'API. `mode` ne change que le
/// texte de `incoming_intel` — c'est tout l'objet de la comparaison v0.4.
fn play(
    agent: &mut dyn Agent,
    seed: u64,
    run: u32,
    max_waves: u32,
    mode: server::Mode,
) -> RunResult {
    let mut g = Game::new(seed);
    let (mut actions, mut last) = (0u32, None);
    let (t0, p0) = (agent.tokens(), agent.parse_failures());
    while g.phase == Phase::Preparation && g.wave <= max_waves {
        let obs = server::observation("g0", &g, mode);
        let action = agent.act(&g, &obs, last);
        // La limite d'actions du moteur garantit la terminaison même si l'agent
        // ne clôt jamais son round : à 20 tentatives la vague part d'office.
        last = g.apply(action).err();
        actions += 1;
        if matches!(action, Action::EndTurn) {
            last = None;
        }
    }
    RunResult {
        // Le mode fait partie de l'identité de la ligne : c'est la clé de fusion
        // des campagnes, et les deux modes doivent cohabiter dans le tableau.
        agent: match mode {
            server::Mode::Detailed => format!("{}+detailed", agent.name()),
            server::Mode::Minimal => agent.name().to_string(),
        },
        seed,
        run,
        wave: g.score(),
        actions,
        errors: ERROR_CODES.map(|c| g.metrics.get(c)),
        forced: g.metrics.forced_end_turns,
        moves: g.metrics.moves_used,
        refunded: g.metrics.gold_refunded,
        parse_failures: agent.parse_failures() - p0,
        tokens: agent.tokens() - t0,
    }
}

fn make_agent(spec: &str, seed: u64, run: u32) -> Box<dyn Agent> {
    match spec {
        "random" => Box::new(Random::new(seed ^ ((run as u64) << 40))),
        "greedy" => Box::new(Greedy::new(None)),
        s => Box::new(llm::Llm::new(s.trim_start_matches("llm:"))),
    }
}

// ---------------------------------------------------------------- Sorties

fn csv(rows: &[RunResult]) -> String {
    let mut s = String::from(
        "agent,seed,run,wave_reached,actions,illegal,path_blocked,insufficient_gold,\
         cell_occupied,no_moves_left,invalid_cell,wrong_phase,unknown_building,\
         forced_end_turns,moves_used,gold_refunded,parse_failures,tokens\n",
    );
    for r in rows {
        let e = r.errors.map(|n| n.to_string()).join(",");
        let _ = writeln!(
            s,
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            r.agent,
            r.seed,
            r.run,
            r.wave,
            r.actions,
            r.illegal(),
            e,
            r.forced,
            r.moves,
            r.refunded,
            r.parse_failures,
            r.tokens
        );
    }
    s
}

/// Relit une campagne précédente : une machine ne tient qu'un LLM en mémoire,
/// donc les modèles se comparent en plusieurs passes. Un agent rejoué remplace
/// ses anciennes lignes ; supprimer le CSV remet le tableau à zéro.
fn read_csv(path: &str) -> Vec<RunResult> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            if f.len() != 18 {
                return None;
            }
            let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
            Some(RunResult {
                agent: f[0].to_string(),
                seed: n(1),
                run: n(2) as u32,
                wave: n(3) as u32,
                actions: n(4) as u32,
                errors: std::array::from_fn(|i| n(6 + i) as u32),
                forced: n(13) as u32,
                moves: n(14) as u32,
                refunded: n(15) as u32,
                parse_failures: n(16) as u32,
                tokens: n(17),
            })
        })
        .collect()
}

fn mean(v: impl Iterator<Item = f64>) -> f64 {
    let (mut n, mut sum) = (0.0, 0.0);
    for x in v {
        sum += x;
        n += 1.0;
    }
    if n == 0.0 { 0.0 } else { sum / n }
}

fn markdown<'a>(rows: &'a [RunResult], seeds: &[u64], runs: u32) -> String {
    let mut agents: Vec<&str> = Vec::new();
    for r in rows {
        if !agents.contains(&r.agent.as_str()) {
            agents.push(&r.agent);
        }
    }
    let of = |a: &'a str| rows.iter().filter(move |r| r.agent == a);

    let mut s = format!(
        "# Baselines GameBenchy\n\n{} seeds × {} runs, {} parties par agent. \
         Métrique principale : vague atteinte.\n\n\
         | agent | vague moy. | min–max | actions | illégales | rounds forcés | \
         déplacements | or remboursé | tokens/vague | JSON illisibles |\n\
         |---|---|---|---|---|---|---|---|---|---|\n",
        seeds.len(),
        runs,
        seeds.len() as u32 * runs
    );
    for a in &agents {
        let waves: Vec<u32> = of(a).map(|r| r.wave).collect();
        let tokens: u64 = of(a).map(|r| r.tokens).sum();
        let total_waves: u32 = waves.iter().sum();
        let _ = writeln!(
            s,
            "| {} | **{:.1}** | {}–{} | {:.1} | {:.1} | {:.1} | {:.1} | {:.0} | {} | {} |",
            a,
            mean(waves.iter().map(|w| *w as f64)),
            waves.iter().min().unwrap_or(&0),
            waves.iter().max().unwrap_or(&0),
            mean(of(a).map(|r| r.actions as f64)),
            mean(of(a).map(|r| r.illegal() as f64)),
            mean(of(a).map(|r| r.forced as f64)),
            mean(of(a).map(|r| r.moves as f64)),
            mean(of(a).map(|r| r.refunded as f64)),
            if tokens == 0 {
                "—".into()
            } else {
                format!("{}", tokens / total_waves.max(1) as u64)
            },
            of(a).map(|r| r.parse_failures).sum::<u32>(),
        );
    }

    // Écart minimal vs detailed (README §6) : la même aiguille, plus de foin.
    let wave_of = |a: &str| mean(rows.iter().filter(|r| r.agent == a).map(|r| r.wave as f64));
    let pairs: Vec<&&str> = agents
        .iter()
        .filter(|a| a.ends_with("+detailed") && agents.contains(&&a[..a.len() - 9]))
        .collect();
    if !pairs.is_empty() {
        s.push_str(
            "\n## Robustesse au bruit : minimal vs detailed\n\n\
             | agent | minimal | detailed | écart | tokens/vague min → det |\n\
             |---|---|---|---|---|\n",
        );
        for a in pairs {
            let base = &a[..a.len() - 9];
            let tok = |x: &str| {
                let (t, w): (u64, u32) = rows
                    .iter()
                    .filter(|r| r.agent == x)
                    .fold((0, 0), |(t, w), r| (t + r.tokens, w + r.wave));
                if t == 0 {
                    "—".into()
                } else {
                    format!("{}", t / w.max(1) as u64)
                }
            };
            let (m, d) = (wave_of(base), wave_of(a));
            let _ = writeln!(
                s,
                "| {base} | {m:.1} | {d:.1} | **{:+.1}** | {} → {} |",
                d - m,
                tok(base),
                tok(a)
            );
        }
    }

    s.push_str("\n## Profil d'erreurs (total par code)\n\n| agent |");
    for c in ERROR_CODES {
        let _ = write!(s, " {} |", c.as_str());
    }
    s.push_str("\n|---|");
    s.push_str(&"---|".repeat(ERROR_CODES.len()));
    s.push('\n');
    for a in &agents {
        let _ = write!(s, "| {a} |");
        for i in 0..ERROR_CODES.len() {
            let _ = write!(s, " {} |", of(a).map(|r| r.errors[i]).sum::<u32>());
        }
        s.push('\n');
    }
    s
}

// ---------------------------------------------------------------- Modes

/// Table mono-tourelle : aucune tourelle ne doit dominer toutes les autres.
fn balance(seeds: &[u64]) {
    println!("greedy (mixte) :");
    let mut total = 0;
    for s in seeds {
        let w = play(&mut Greedy::new(None), *s, 0, 100, server::Mode::Minimal).wave;
        println!("  seed {s} → vague {w}");
        total += w;
    }
    println!("  moyenne : {:.1}\n", total as f64 / seeds.len() as f64);
    println!("mono-tourelle (moyenne sur les mêmes seeds) :");
    for kind in BUILDING_TYPES {
        if kind.stats().damage == 0 {
            continue;
        }
        let sum: u32 = seeds
            .iter()
            .map(|s| {
                play(
                    &mut Greedy::new(Some(kind)),
                    *s,
                    0,
                    100,
                    server::Mode::Minimal,
                )
                .wave
            })
            .sum();
        println!(
            "  {:>12} : {:.1}",
            kind.stats().name,
            sum as f64 / seeds.len() as f64
        );
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let list = |name: &str, default: Vec<u64>| {
        flag(name).map_or(default, |v| {
            v.split(',').filter_map(|s| s.parse().ok()).collect()
        })
    };

    if args.first().is_some_and(|a| a == "balance") {
        balance(&list("--seeds", vec![1, 2, 3, 42, 1337]));
        return Ok(());
    }

    // Seeds d'éval, distinctes des seeds de dev (README §5).
    let seeds = list("--seeds", vec![101, 102, 103]);
    let runs: u32 = flag("--runs").and_then(|s| s.parse().ok()).unwrap_or(3);
    let max_waves: u32 = flag("--max-waves")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let mut specs: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| *a == "--agent" && args.len() > i + 1)
        .map(|(i, _)| args[i + 1].clone())
        .collect();
    if specs.is_empty() {
        specs = vec!["random".into(), "greedy".into()];
    }
    let out = flag("--out").unwrap_or_else(|| "results".into());
    let mode = match flag("--mode").as_deref() {
        Some("detailed") => server::Mode::Detailed,
        _ => server::Mode::Minimal,
    };

    let mut rows = Vec::new();
    for spec in &specs {
        for &seed in &seeds {
            for run in 0..runs {
                let mut agent = make_agent(spec, seed, run);
                let r = play(&mut *agent, seed, run, max_waves, mode);
                println!(
                    "{:>28} seed {seed} run {run} → vague {:>3} ({} actions, {} illégales)",
                    r.agent,
                    r.wave,
                    r.actions,
                    r.illegal()
                );
                rows.push(r);
            }
        }
    }

    let path = format!("{out}/campaign.csv");
    let fresh: Vec<String> = rows.iter().map(|r| r.agent.clone()).collect();
    let mut all = read_csv(&path);
    all.retain(|r| !fresh.contains(&r.agent));
    all.extend(rows);
    let rows = all;

    let md = markdown(&rows, &seeds, runs);
    std::fs::create_dir_all(&out)?;
    std::fs::write(&path, csv(&rows))?;
    std::fs::write(format!("{out}/campaign.md"), &md)?;
    println!("\n{md}\n→ {out}/campaign.csv, {out}/campaign.md");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le random doit produire beaucoup d'illégal et mourir vite ; le greedy doit
    /// survivre nettement plus longtemps. C'est le sens même d'une baseline.
    #[test]
    fn baselines_are_ordered_and_metrics_are_collected() {
        let m = server::Mode::Minimal;
        let r = play(&mut Random::new(7), 101, 0, 100, m);
        let g = play(&mut Greedy::new(None), 101, 0, 100, m);
        assert!(r.illegal() > 0, "le random doit taper dans les erreurs");
        assert!(
            g.wave > r.wave * 2,
            "greedy {} vs random {}",
            g.wave,
            r.wave
        );
        assert_eq!(g.parse_failures, 0);
        assert!(r.actions > 0 && g.actions > 0);

        let text = csv(&[r, g]);
        assert_eq!(text.lines().count(), 3);
        assert_eq!(
            text.lines().next().unwrap().split(',').count(),
            text.lines().nth(1).unwrap().split(',').count(),
            "en-tête et lignes doivent avoir le même nombre de colonnes"
        );

        // Aller-retour CSV : c'est ce qui permet de fusionner deux passes de
        // campagne (un LLM à la fois).
        let tmp = std::env::temp_dir().join("gamebenchy_test_campaign.csv");
        std::fs::write(&tmp, &text).unwrap();
        let back = read_csv(tmp.to_str().unwrap());
        assert_eq!(csv(&back), text);
        std::fs::remove_file(&tmp).ok();
    }

    /// Rejouer la même campagne doit donner exactement les mêmes chiffres.
    #[test]
    fn campaign_is_reproducible() {
        let one = || {
            let rows: Vec<RunResult> = [101u64, 102]
                .iter()
                .map(|s| play(&mut Random::new(*s), *s, 0, 20, server::Mode::Minimal))
                .collect();
            csv(&rows)
        };
        assert_eq!(one(), one());
    }

    /// Le mode `detailed` change le texte vu par l'agent, pas le jeu : un agent
    /// qui ne lit pas l'intel doit faire exactement le même score, et les deux
    /// lignes doivent cohabiter dans le tableau (clé de fusion = agent+mode).
    #[test]
    fn detailed_mode_is_a_separate_row_and_changes_only_the_text() {
        let mini = play(&mut Greedy::new(None), 101, 0, 30, server::Mode::Minimal);
        let det = play(&mut Greedy::new(None), 101, 0, 30, server::Mode::Detailed);
        assert_eq!(mini.wave, det.wave, "greedy ne lit pas l'intel");
        assert_eq!(det.agent, "greedy+detailed");

        let md = markdown(&[mini, det], &[101], 1);
        assert!(md.contains("minimal vs detailed"), "{md}");
        assert!(md.contains("**+0.0**"), "{md}");
    }
}
