//! Interface agent et baselines scriptées.
//!
//! `act(observation) -> action` (README §5). Les baselines lisent l'état typé,
//! les agents LLM lisent l'observation JSON : c'est le même état, sérialisé.
//! Le seul canal d'action est `Action`, donc les métriques restent comparables.

use engine::*;
use serde_json::Value;

pub trait Agent {
    fn name(&self) -> &str;
    /// `last_error` = résultat de l'action précédente du même round (feedback).
    fn act(&mut self, g: &Game, obs: &Value, last_error: Option<ErrorCode>) -> Action;
    /// Tokens consommés (agents LLM uniquement).
    fn tokens(&self) -> u64 {
        0
    }
    /// Réponses inexploitables (agents LLM uniquement).
    fn parse_failures(&self) -> u32 {
        0
    }
}

// ---------------------------------------------------------------- Random

/// Plancher du benchmark : actions tirées au hasard, légales ou non.
pub struct Random {
    rng: Rng,
    name: String,
}

impl Random {
    pub fn new(seed: u64) -> Self {
        Random {
            rng: Rng::new(seed),
            name: "random".into(),
        }
    }
    fn cell(&mut self) -> Pos {
        Pos::new(
            self.rng.below(BOARD_W as u64) as i32,
            self.rng.below(BOARD_H as u64) as i32,
        )
    }
}

impl Agent for Random {
    fn name(&self) -> &str {
        &self.name
    }

    fn act(&mut self, g: &Game, _obs: &Value, _last: Option<ErrorCode>) -> Action {
        let pick_id = |rng: &mut Rng, g: &Game| {
            g.buildings
                .get(rng.below(g.buildings.len() as u64) as usize)
                .map(|b| b.id)
                .unwrap_or(1)
        };
        match self.rng.below(10) {
            0 => Action::EndTurn,
            1 => Action::Sell {
                id: pick_id(&mut self.rng, g),
            },
            2 => {
                let id = pick_id(&mut self.rng, g);
                Action::Move {
                    id,
                    pos: self.cell(),
                }
            }
            _ => Action::Build {
                kind: BUILDING_TYPES[self.rng.below(BUILDING_TYPES.len() as u64) as usize],
                pos: self.cell(),
            },
        }
    }
}

// ---------------------------------------------------------------- Greedy

/// Baseline sérieuse : achète la tourelle au meilleur rapport
/// dégâts-attendus-contre-la-vague-annoncée / coût, épargne quand le vrai
/// counter est hors budget. Sert aussi de harnais d'équilibrage.
pub struct Greedy {
    name: String,
    /// Restriction mono-tourelle (mode équilibrage).
    pub only: Option<BuildingType>,
    last_eco_wave: u32,
}

impl Greedy {
    pub fn new(only: Option<BuildingType>) -> Self {
        Greedy {
            name: match only {
                Some(k) => format!("greedy[{}]", k.stats().name),
                None => "greedy".into(),
            },
            only,
            last_eco_wave: 0,
        }
    }
}

impl Agent for Greedy {
    fn name(&self) -> &str {
        &self.name
    }

    fn act(&mut self, g: &Game, _obs: &Value, last: Option<ErrorCode>) -> Action {
        // Une action refusée signifie que le plan est caduc : on clôt le round
        // plutôt que de brûler des crédits (et de polluer les métriques).
        if last.is_some() {
            return Action::EndTurn;
        }
        let Some(path) = g.current_path() else {
            return Action::EndTurn;
        };
        // Investissement éco au début, une fois par round.
        let eco = BuildingType::Eco;
        if g.wave <= 3
            && self.last_eco_wave != g.wave
            && g.gold >= eco.stats().cost + 50
            && let Some(pos) = free_cells(g, &path).next()
        {
            self.last_eco_wave = g.wave;
            return Action::Build { kind: eco, pos };
        }
        match best_build(g, &path, self.only) {
            Some((kind, pos)) => Action::Build { kind, pos },
            None => Action::EndTurn,
        }
    }
}

/// Cases constructibles : ni occupées, ni entrée/sortie, ni sur le chemin
/// (bâtir sur le chemin le rallonge mais déplace la défense — hors heuristique).
fn free_cells<'a>(g: &'a Game, path: &'a [Pos]) -> impl Iterator<Item = Pos> + 'a {
    (0..BOARD_H)
        .flat_map(|y| (0..BOARD_W).map(move |x| Pos::new(x, y)))
        .filter(move |p| !g.occupied(*p) && *p != g.entry && *p != g.exit && !path.contains(p))
}

/// Meilleure (tourelle, case) : dégâts attendus contre la composition annoncée,
/// pondérés par la couverture du chemin, rapportés au coût.
pub fn best_build(
    g: &Game,
    path: &[Pos],
    only: Option<BuildingType>,
) -> Option<(BuildingType, Pos)> {
    let comp = g.composition(g.wave);
    let air_path = straight_line(g.entry, g.exit);
    // Part de PV de la vague par type d'ennemi : là où il faut taper.
    let hp_share: Vec<u32> = ENEMY_KINDS
        .iter()
        .map(|k| comp[k.index()] * k.hp_at(g.wave))
        .collect();

    let mut best: Option<(u32, BuildingType, Pos)> = None;
    let mut affordable: Option<(u32, BuildingType, Pos)> = None;
    for pos in free_cells(g, path) {
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
    // Épargner plutôt que dépenser quand le vrai counter coûte plus cher.
    match (best, affordable) {
        (Some((bs, _, _)), Some((as_, k, p))) if as_ * 2 >= bs => Some((k, p)),
        _ => None,
    }
}
