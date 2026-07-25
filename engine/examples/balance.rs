//! Harnais d'équilibrage : joue des parties avec un agent scripté greedy et
//! affiche la vague de mort. Sert à voir si le jeu est trivial sans jouer 40 parties.
//!
//! `cargo run -q -p engine --example balance [seed...]`

use engine::*;

/// Meilleure (tourelle, case) : dégâts attendus contre la composition annoncée,
/// pondérés par la couverture du chemin, rapportés au coût.
fn best_build(g: &Game, path: &[Pos], only: Option<BuildingType>) -> Option<(BuildingType, Pos)> {
    let comp = g.composition(g.wave);
    let air_path = straight_line(g.entry, g.exit);
    // Part de PV de la vague par type d'ennemi : là où il faut taper.
    let hp_share: Vec<u32> = ENEMY_KINDS
        .iter()
        .map(|k| comp[k.index()] * k.hp_at(g.wave))
        .collect();

    let mut best: Option<(u32, BuildingType, Pos)> = None;
    let mut affordable: Option<(u32, BuildingType, Pos)> = None;
    for y in 0..BOARD_H {
        for x in 0..BOARD_W {
            let pos = Pos::new(x, y);
            if g.occupied(pos) || pos == g.entry || pos == g.exit || path.contains(&pos) {
                continue;
            }
            for kind in BUILDING_TYPES {
                let st = kind.stats();
                if only.is_some_and(|k| k != kind) || st.damage == 0 {
                    continue;
                }
                let cover = |p: &[Pos]| {
                    p.iter()
                        .filter(|c| {
                            let d = pos.dist_sq(**c);
                            d <= st.range_sq && d >= st.min_range_sq
                        })
                        .count() as u32
                };
                let (cg, ca) = (cover(path), cover(&air_path));
                let splash = if st.splash_sq > 0 { 2 } else { 1 };
                let ground = (effective_damage(kind, EnemyKind::Infantry) * hp_share[0]
                    + effective_damage(kind, EnemyKind::Armor) * hp_share[1])
                    * cg
                    * splash;
                let air = if st.hits_air {
                    effective_damage(kind, EnemyKind::Flyer) * hp_share[2] * ca
                } else {
                    0
                };
                let score = (ground + air) / (st.cooldown + 1) / st.cost;
                if score == 0 {
                    continue;
                }
                if best.is_none_or(|(b, _, _)| score > b) {
                    best = Some((score, kind, pos));
                }
                if st.cost <= g.gold && affordable.is_none_or(|(b, _, _)| score > b) {
                    affordable = Some((score, kind, pos));
                }
            }
        }
    }
    // Épargner plutôt que dépenser quand le vrai counter coûte plus cher.
    match (best, affordable) {
        (Some((bs, _, _)), Some((as_, k, p))) if as_ * 2 >= bs => Some((k, p)),
        _ => None,
    }
}

fn play(seed: u64, verbose: bool, only: Option<BuildingType>) -> u32 {
    let mut g = Game::new(seed);
    while g.phase == Phase::Preparation && g.wave <= 100 {
        // Investissement éco au début, défense ensuite.
        if g.wave <= 3 && g.gold >= BuildingType::Eco.stats().cost + 50 {
            let spot = (0..BOARD_H)
                .flat_map(|y| (0..BOARD_W).map(move |x| Pos::new(x, y)))
                .find(|p| !g.occupied(*p) && *p != g.entry && *p != g.exit);
            if let Some(p) = spot {
                let _ = g.apply(Action::Build {
                    kind: BuildingType::Eco,
                    pos: p,
                });
            }
        }
        while g.actions_used < ACTION_LIMIT - 1 {
            let Some(path) = g.current_path() else { break };
            let Some((kind, pos)) = best_build(&g, &path, only) else {
                break;
            };
            if g.apply(Action::Build { kind, pos }).is_err() {
                break;
            }
        }
        let (wave, gold, lives, towers) = (g.wave, g.gold, g.lives, g.buildings.len());
        g.apply(Action::EndTurn).ok();
        if verbose {
            let r = g.last_report.clone().unwrap();
            println!(
                "vague {wave:>3} | or {gold:>4} | vies {lives:>3} | bâtiments {towers:>2} | \
                 tués {:?} | fuites {:?} (-{} vies)",
                r.kills, r.leaked, r.lives_lost
            );
        }
    }
    if verbose {
        let mix: Vec<String> = BUILDING_TYPES
            .iter()
            .map(|k| {
                let n = g.buildings.iter().filter(|b| b.kind == *k).count();
                format!("{} {}", k.stats().name, n)
            })
            .collect();
        println!("parc final : {}", mix.join(", "));
    }
    g.score()
}

fn main() {
    let seeds: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let seeds = if seeds.is_empty() {
        vec![1, 2, 3, 42, 1337]
    } else {
        seeds
    };
    let verbose = seeds.len() == 1;
    let mut total = 0;
    for s in &seeds {
        let score = play(*s, verbose, None);
        println!("seed {s} → vague {score}");
        total += score;
    }
    println!("moyenne mixte : {:.1}", total as f64 / seeds.len() as f64);

    // Identité des tourelles : aucune ne doit dominer toutes les autres.
    println!("\nmono-tourelle (moyenne sur les mêmes seeds) :");
    for kind in BUILDING_TYPES {
        if kind.stats().damage == 0 {
            continue;
        }
        let sum: u32 = seeds.iter().map(|s| play(*s, false, Some(kind))).sum();
        println!(
            "  {:>12} : {:.1}",
            kind.stats().name,
            sum as f64 / seeds.len() as f64
        );
    }
}
