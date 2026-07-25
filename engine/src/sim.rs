//! Résolution d'une vague : simulation à ticks discrets, entièrement déterministe.
//!
//! Ordre d'un tick, figé : spawn → tir des tourelles → retrait des morts →
//! déplacement des ennemis. Aucun flottant, aucune horloge.

use crate::*;

/// Ticks entre deux apparitions d'ennemis.
const SPAWN_PERIOD: u32 = 3;
/// Garde-fou : une vague ne peut pas boucler indéfiniment.
const MAX_TICKS: u32 = 5000;

struct Enemy {
    id: u32,
    kind: EnemyKind,
    hp: u32,
    /// Index sur son chemin (progression : sert aussi au ciblage).
    idx: usize,
    acc: u32,
}

/// Simule la vague courante de `game` sans muter l'état de jeu.
pub fn run_wave(game: &Game) -> WaveReport {
    let ground_path = game.current_path().unwrap_or_default();
    let air_path = straight_line(game.entry, game.exit);
    let mut report = WaveReport {
        wave: game.wave,
        ..Default::default()
    };

    // File d'apparition : ordre mélangé (l'agent connaît la composition, pas l'ordre).
    let mut queue = spawn_order(game.seed, game.wave, game.composition(game.wave));
    queue.reverse(); // pop_back = premier arrivé

    let mut alive: Vec<Enemy> = Vec::new();
    let mut cooldowns = vec![0u32; game.buildings.len()];
    let mut damage = vec![0u32; game.buildings.len()];
    let mut next_id = 0;

    for tick in 0..MAX_TICKS {
        if queue.is_empty() && alive.is_empty() {
            report.ticks = tick;
            break;
        }
        // 1. Spawn
        if tick % SPAWN_PERIOD == 0
            && let Some(kind) = queue.pop()
        {
            alive.push(Enemy {
                id: next_id,
                kind,
                hp: kind.hp_at(game.wave),
                idx: 0,
                acc: 0,
            });
            next_id += 1;
        }

        // 2. Tir
        for (i, b) in game.buildings.iter().enumerate() {
            let st = b.kind.stats();
            if st.damage == 0 {
                continue;
            }
            if cooldowns[i] > 0 {
                cooldowns[i] -= 1;
                continue;
            }
            let Some(t) = pick_target(b, &alive, &ground_path, &air_path) else {
                continue;
            };
            let target_pos = pos_of(&alive[t], &ground_path, &air_path);
            cooldowns[i] = st.cooldown;
            for e in alive.iter_mut() {
                if e.kind == EnemyKind::Flyer && !st.hits_air {
                    continue;
                }
                let p = pos_of(e, &ground_path, &air_path);
                if p.dist_sq(target_pos) > st.splash_sq {
                    continue;
                }
                let dealt = effective_damage(b.kind, e.kind).min(e.hp);
                e.hp -= dealt;
                damage[i] += dealt;
            }
        }

        // 3. Morts
        alive.retain(|e| {
            if e.hp == 0 {
                report.kills[e.kind.index()] += 1;
                false
            } else {
                true
            }
        });

        // 4. Déplacement (les fuyards atteignent la sortie)
        alive.retain_mut(|e| {
            let path = path_for(e.kind, &ground_path, &air_path);
            e.acc += 1;
            if e.acc < e.kind.move_period() {
                return true;
            }
            e.acc = 0;
            e.idx += 1;
            // Sortie atteinte (ou chemin vide : plateau bloqué) → fuite.
            if path.is_empty() || e.idx >= path.len() - 1 {
                leak(&mut report, e.kind);
                return false;
            }
            true
        });
        report.ticks = tick + 1;
    }

    report.damage_by_building = game
        .buildings
        .iter()
        .zip(&damage)
        .filter(|(_, d)| **d > 0)
        .map(|(b, d)| (b.id, *d))
        .collect();
    report
}

fn leak(report: &mut WaveReport, kind: EnemyKind) {
    report.leaked[kind.index()] += 1;
    report.lives_lost += kind.lives_cost();
}

fn path_for<'a>(kind: EnemyKind, ground: &'a [Pos], air: &'a [Pos]) -> &'a [Pos] {
    if kind == EnemyKind::Flyer {
        air
    } else {
        ground
    }
}

fn pos_of(e: &Enemy, ground: &[Pos], air: &[Pos]) -> Pos {
    let path = path_for(e.kind, ground, air);
    path.get(e.idx).copied().unwrap_or(*path.last().unwrap())
}

/// Cible prioritaire : l'ennemi le plus avancé à portée, id croissant en cas d'égalité.
fn pick_target(b: &Building, alive: &[Enemy], ground: &[Pos], air: &[Pos]) -> Option<usize> {
    let st = b.kind.stats();
    alive
        .iter()
        .enumerate()
        .filter(|(_, e)| st.hits_air || e.kind != EnemyKind::Flyer)
        .filter(|(_, e)| {
            let d = b.pos.dist_sq(pos_of(e, ground, air));
            d <= st.range_sq && d >= st.min_range_sq
        })
        .max_by_key(|(_, e)| (e.idx, std::cmp::Reverse(e.id)))
        .map(|(i, _)| i)
}

/// Ordre d'apparition mélangé, dérivé du seed et du numéro de vague.
fn spawn_order(seed: u64, wave: u32, comp: Composition) -> Vec<EnemyKind> {
    let mut list: Vec<EnemyKind> = ENEMY_KINDS
        .iter()
        .flat_map(|k| std::iter::repeat_n(*k, comp[k.index()] as usize))
        .collect();
    // Fisher-Yates seedé.
    let mut rng = Rng::new(seed ^ ((wave as u64) << 32));
    for i in (1..list.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        list.swap(i, j);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snipers_kill_infantry_and_report_damage() {
        let mut g = Game::new(1);
        g.gold = 10_000;
        // Une batterie de snipers le long du couloir d'entrée.
        for y in [g.entry.y - 1, g.entry.y + 1] {
            for x in 1..5 {
                g.apply(Action::Build {
                    kind: BuildingType::Sniper,
                    pos: Pos::new(x, y),
                })
                .unwrap();
            }
        }
        let r = run_wave(&g);
        assert!(r.kills[0] > 0);
        assert_eq!(r.leaked, [0, 0, 0]);
        assert!(!r.damage_by_building.is_empty());
    }

    #[test]
    fn flyers_ignore_the_maze_and_only_snipers_touch_them() {
        let mut g = Game::new(1);
        g.gold = 10_000;
        g.wave = 12; // vague avec des volants
        assert!(g.composition(12)[2] > 0);
        // Que des lance-flammes : aveugles au ciel.
        for x in 1..9 {
            let _ = g.apply(Action::Build {
                kind: BuildingType::Flamethrower,
                pos: Pos::new(x, g.entry.y + 2),
            });
        }
        let r = run_wave(&g);
        assert_eq!(r.kills[2], 0, "les lance-flammes ne visent pas le ciel");
        assert_eq!(r.leaked[2], g.composition(12)[2]);
    }

    #[test]
    fn anti_armor_beats_armor_faster_than_sniper() {
        let kills = |kind| {
            let mut g = Game::new(4);
            g.wave = 8;
            g.gold = 10_000;
            for x in 2..6 {
                g.apply(Action::Build {
                    kind,
                    pos: Pos::new(x, g.entry.y + 1),
                })
                .unwrap();
            }
            run_wave(&g).kills[1]
        };
        assert!(kills(BuildingType::AntiArmor) > kills(BuildingType::Sniper));
    }

    #[test]
    fn simulation_is_reproducible() {
        let mut g = Game::new(99);
        g.gold = 10_000;
        g.wave = 7;
        g.apply(Action::Build {
            kind: BuildingType::Mortar,
            pos: Pos::new(4, 2),
        })
        .unwrap();
        let a = run_wave(&g);
        let b = run_wave(&g);
        assert_eq!(a.kills, b.kills);
        assert_eq!(a.leaked, b.leaked);
        assert_eq!(a.ticks, b.ticks);
        assert_eq!(a.damage_by_building, b.damage_by_building);
    }
}
