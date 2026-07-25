//! GameBenchy engine — deterministic, seedé, zéro I/O.
//!
//! Même seed + même séquence d'actions = même partie, bit pour bit.

pub mod sim;

use std::collections::VecDeque;

pub const BOARD_W: i32 = 10;
pub const BOARD_H: i32 = 10;
pub const START_GOLD: u32 = 180;
pub const START_LIVES: u32 = 20;
pub const BASE_INCOME: u32 = 30;
pub const MOVES_PER_ROUND: u32 = 1;
pub const ACTION_LIMIT: u32 = 20;

// ---------------------------------------------------------------- RNG

/// xorshift64*. Pas de dépendance : la reproductibilité exige un RNG dont
/// l'algorithme est figé dans ce dépôt, pas dans une version de crate.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng((seed ^ 0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Entier dans [0, n).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
}

// ---------------------------------------------------------------- Types

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
}

impl Pos {
    pub fn new(x: i32, y: i32) -> Self {
        Pos { x, y }
    }
    /// Distance euclidienne au carré, en entiers (pas de flottant : déterminisme).
    pub fn dist_sq(self, o: Pos) -> i32 {
        let (dx, dy) = (self.x - o.x, self.y - o.y);
        dx * dx + dy * dy
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildingType {
    Sniper,
    Flamethrower,
    AntiArmor,
    Mortar,
    Eco,
}

pub const BUILDING_TYPES: [BuildingType; 5] = [
    BuildingType::Sniper,
    BuildingType::Flamethrower,
    BuildingType::AntiArmor,
    BuildingType::Mortar,
    BuildingType::Eco,
];

/// Stats de tourelle. Les portées sont des distances **au carré** (entiers).
pub struct BuildingStats {
    pub name: &'static str,
    pub cost: u32,
    pub range_sq: i32,
    pub min_range_sq: i32,
    pub damage: u32,
    /// Dégâts additionnels contre les blindés.
    pub bonus_vs_armor: u32,
    /// Ticks entre deux tirs.
    pub cooldown: u32,
    /// Rayon² d'éclaboussure autour de la cible (0 = mono-cible).
    pub splash_sq: i32,
    pub hits_air: bool,
    pub income: u32,
}

impl BuildingType {
    pub fn stats(self) -> &'static BuildingStats {
        // ponytail: table constante en dur, c'est le fichier d'équilibrage.
        match self {
            BuildingType::Sniper => &BuildingStats {
                name: "sniper",
                cost: 50,
                range_sq: 16,
                min_range_sq: 0,
                damage: 7,
                bonus_vs_armor: 0,
                cooldown: 2,
                splash_sq: 0,
                hits_air: true,
                income: 0,
            },
            BuildingType::Flamethrower => &BuildingStats {
                name: "flamethrower",
                cost: 40,
                range_sq: 2,
                min_range_sq: 0,
                damage: 5,
                bonus_vs_armor: 0,
                cooldown: 2,
                splash_sq: 2,
                hits_air: false,
                income: 0,
            },
            BuildingType::AntiArmor => &BuildingStats {
                name: "anti_armor",
                cost: 70,
                range_sq: 9,
                min_range_sq: 0,
                damage: 5,
                bonus_vs_armor: 40,
                cooldown: 3,
                splash_sq: 0,
                hits_air: false,
                income: 0,
            },
            BuildingType::Mortar => &BuildingStats {
                name: "mortar",
                cost: 90,
                range_sq: 36,
                min_range_sq: 4,
                damage: 13,
                bonus_vs_armor: 0,
                cooldown: 5,
                splash_sq: 4,
                hits_air: false,
                income: 0,
            },
            BuildingType::Eco => &BuildingStats {
                name: "eco",
                cost: 60,
                range_sq: 0,
                min_range_sq: 0,
                damage: 0,
                bonus_vs_armor: 0,
                cooldown: 0,
                splash_sq: 0,
                hits_air: false,
                income: 8,
            },
        }
    }

    pub fn glyph(self) -> char {
        match self {
            BuildingType::Sniper => 'S',
            BuildingType::Flamethrower => 'F',
            BuildingType::AntiArmor => 'A',
            BuildingType::Mortar => 'M',
            BuildingType::Eco => 'E',
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        BUILDING_TYPES.into_iter().find(|b| b.stats().name == s)
    }
}

#[derive(Clone, Debug)]
pub struct Building {
    pub id: u32,
    pub kind: BuildingType,
    pub pos: Pos,
    /// Cumul sur toute la partie (feedback granulaire, cf. README §4).
    pub damage_dealt: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnemyKind {
    Infantry,
    Armor,
    Flyer,
}

pub const ENEMY_KINDS: [EnemyKind; 3] = [EnemyKind::Infantry, EnemyKind::Armor, EnemyKind::Flyer];

impl EnemyKind {
    pub fn name(self) -> &'static str {
        match self {
            EnemyKind::Infantry => "infantry",
            EnemyKind::Armor => "armor",
            EnemyKind::Flyer => "flyer",
        }
    }
    pub fn hp(self) -> u32 {
        match self {
            EnemyKind::Infantry => 14,
            EnemyKind::Armor => 80,
            EnemyKind::Flyer => 20,
        }
    }
    /// Blindage : retranché de chaque coup non spécialisé. C'est lui qui rend le
    /// counter obligatoire — sans anti-blindage, les chars passent.
    pub fn plating(self) -> u32 {
        match self {
            EnemyKind::Armor => 6,
            _ => 0,
        }
    }
    /// PV à la vague donnée : +8 % par vague, composé. C'est le gradient de
    /// difficulté continu — le revenu (donc la DPS achetable) croît linéairement,
    /// les PV géométriquement : toute partie finit par tomber.
    pub fn hp_at(self, wave: u32) -> u32 {
        let mut hp = self.hp() as u64;
        for _ in 1..wave {
            hp = hp * 108 / 100;
        }
        hp as u32
    }
    /// Ticks entre deux déplacements d'une case.
    pub fn move_period(self) -> u32 {
        match self {
            EnemyKind::Infantry => 2,
            EnemyKind::Armor => 4,
            EnemyKind::Flyer => 3,
        }
    }
    pub fn lives_cost(self) -> u32 {
        match self {
            EnemyKind::Infantry => 1,
            EnemyKind::Armor => 3,
            EnemyKind::Flyer => 1,
        }
    }
    pub fn glyph(self) -> char {
        match self {
            EnemyKind::Infantry => 'i',
            EnemyKind::Armor => 'a',
            EnemyKind::Flyer => 'v',
        }
    }
    pub fn index(self) -> usize {
        match self {
            EnemyKind::Infantry => 0,
            EnemyKind::Armor => 1,
            EnemyKind::Flyer => 2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Preparation,
    GameOver,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorCode {
    PathBlocked,
    InsufficientGold,
    CellOccupied,
    NoMovesLeft,
    InvalidCell,
    WrongPhase,
    UnknownBuilding,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::PathBlocked => "PATH_BLOCKED",
            ErrorCode::InsufficientGold => "INSUFFICIENT_GOLD",
            ErrorCode::CellOccupied => "CELL_OCCUPIED",
            ErrorCode::NoMovesLeft => "NO_MOVES_LEFT",
            ErrorCode::InvalidCell => "INVALID_CELL",
            ErrorCode::WrongPhase => "WRONG_PHASE",
            ErrorCode::UnknownBuilding => "UNKNOWN_BUILDING",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            ErrorCode::PathBlocked => "Ce placement rendrait la sortie inaccessible.",
            ErrorCode::InsufficientGold => "Or insuffisant.",
            ErrorCode::CellOccupied => "Case déjà occupée.",
            ErrorCode::NoMovesLeft => "Plus de déplacement disponible ce round.",
            ErrorCode::InvalidCell => "Case hors plateau ou interdite.",
            ErrorCode::WrongPhase => "Action impossible dans cette phase.",
            ErrorCode::UnknownBuilding => "Bâtiment inconnu.",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Build { kind: BuildingType, pos: Pos },
    Sell { id: u32 },
    Move { id: u32, pos: Pos },
    EndTurn,
}

/// Composition d'une vague : compte par type d'ennemi (indexé par `EnemyKind::index`).
pub type Composition = [u32; 3];

#[derive(Clone, Debug, Default)]
pub struct WaveReport {
    pub wave: u32,
    pub kills: Composition,
    pub leaked: Composition,
    pub lives_lost: u32,
    /// (building_id, dégâts infligés pendant cette vague)
    pub damage_by_building: Vec<(u32, u32)>,
    pub ticks: u32,
}

/// Compteurs pour les métriques du benchmark (README §6).
#[derive(Clone, Debug, Default)]
pub struct Metrics {
    pub errors: Vec<(ErrorCode, u32)>,
    pub moves_used: u32,
    pub gold_refunded: u32,
    pub forced_end_turns: u32,
}

impl Metrics {
    fn count(&mut self, code: ErrorCode) {
        match self.errors.iter_mut().find(|(c, _)| *c == code) {
            Some((_, n)) => *n += 1,
            None => self.errors.push((code, 1)),
        }
    }
    pub fn get(&self, code: ErrorCode) -> u32 {
        self.errors
            .iter()
            .find(|(c, _)| *c == code)
            .map_or(0, |(_, n)| *n)
    }
}

// ---------------------------------------------------------------- Game

pub struct Game {
    pub seed: u64,
    pub wave: u32,
    pub phase: Phase,
    pub lives: u32,
    pub gold: u32,
    pub entry: Pos,
    pub exit: Pos,
    pub buildings: Vec<Building>,
    pub moves_remaining: u32,
    pub actions_used: u32,
    pub last_report: Option<WaveReport>,
    pub metrics: Metrics,
    next_id: u32,
}

impl Game {
    pub fn new(seed: u64) -> Self {
        // Entrée/sortie fixes, décalées d'au moins une ligne (cf. README §2).
        let entry = Pos::new(0, BOARD_H / 2 - 1);
        let exit = Pos::new(BOARD_W - 1, BOARD_H / 2);
        Game {
            seed,
            wave: 1,
            phase: Phase::Preparation,
            lives: START_LIVES,
            gold: START_GOLD,
            entry,
            exit,
            buildings: Vec::new(),
            moves_remaining: MOVES_PER_ROUND,
            actions_used: 0,
            last_report: None,
            metrics: Metrics::default(),
            next_id: 1,
        }
    }

    pub fn income(&self) -> u32 {
        BASE_INCOME
            + self
                .buildings
                .iter()
                .map(|b| b.kind.stats().income)
                .sum::<u32>()
    }

    pub fn building_at(&self, p: Pos) -> Option<&Building> {
        self.buildings.iter().find(|b| b.pos == p)
    }

    pub fn occupied(&self, p: Pos) -> bool {
        self.building_at(p).is_some()
    }

    fn in_bounds(&self, p: Pos) -> bool {
        p.x >= 0 && p.y >= 0 && p.x < BOARD_W && p.y < BOARD_H
    }

    /// Chemin courant entrée → sortie, `None` si bloqué.
    pub fn current_path(&self) -> Option<Vec<Pos>> {
        let blocked: Vec<Pos> = self.buildings.iter().map(|b| b.pos).collect();
        shortest_path(self.entry, self.exit, &blocked)
    }

    /// Composition de la vague `wave` — dérivée du seed uniquement (rejouable).
    pub fn composition(&self, wave: u32) -> Composition {
        wave_composition(self.seed, wave)
    }

    /// Applique une action. Toute tentative (y compris en erreur) consomme un
    /// crédit d'action ; au-delà de `ACTION_LIMIT` le tour est clôturé d'office.
    pub fn apply(&mut self, action: Action) -> Result<(), ErrorCode> {
        if self.phase != Phase::Preparation {
            self.metrics.count(ErrorCode::WrongPhase);
            return Err(ErrorCode::WrongPhase);
        }
        self.actions_used += 1;
        let res = self.try_action(action);
        if let Err(code) = res {
            self.metrics.count(code);
        }
        if self.phase == Phase::Preparation && self.actions_used >= ACTION_LIMIT {
            self.metrics.forced_end_turns += 1;
            self.resolve_wave();
        }
        res
    }

    fn try_action(&mut self, action: Action) -> Result<(), ErrorCode> {
        match action {
            Action::Build { kind, pos } => self.build(kind, pos),
            Action::Sell { id } => self.sell(id),
            Action::Move { id, pos } => self.move_building(id, pos),
            Action::EndTurn => {
                self.resolve_wave();
                Ok(())
            }
        }
    }

    fn check_placement(&self, pos: Pos) -> Result<(), ErrorCode> {
        if !self.in_bounds(pos) || pos == self.entry || pos == self.exit {
            return Err(ErrorCode::InvalidCell);
        }
        if self.occupied(pos) {
            return Err(ErrorCode::CellOccupied);
        }
        Ok(())
    }

    /// Le chemin resterait-il ouvert si `moved` (optionnel) partait et `added` arrivait ?
    fn path_survives(&self, added: Option<Pos>, removed: Option<u32>) -> bool {
        let mut blocked: Vec<Pos> = self
            .buildings
            .iter()
            .filter(|b| Some(b.id) != removed)
            .map(|b| b.pos)
            .collect();
        blocked.extend(added);
        shortest_path(self.entry, self.exit, &blocked).is_some()
    }

    fn build(&mut self, kind: BuildingType, pos: Pos) -> Result<(), ErrorCode> {
        self.check_placement(pos)?;
        let cost = kind.stats().cost;
        if self.gold < cost {
            return Err(ErrorCode::InsufficientGold);
        }
        if !self.path_survives(Some(pos), None) {
            return Err(ErrorCode::PathBlocked);
        }
        self.gold -= cost;
        let id = self.next_id;
        self.next_id += 1;
        self.buildings.push(Building {
            id,
            kind,
            pos,
            damage_dealt: 0,
        });
        Ok(())
    }

    fn sell(&mut self, id: u32) -> Result<(), ErrorCode> {
        let idx = self
            .buildings
            .iter()
            .position(|b| b.id == id)
            .ok_or(ErrorCode::UnknownBuilding)?;
        let refund = self.buildings[idx].kind.stats().cost / 2;
        self.buildings.remove(idx);
        self.gold += refund;
        self.metrics.gold_refunded += refund;
        Ok(())
    }

    fn move_building(&mut self, id: u32, pos: Pos) -> Result<(), ErrorCode> {
        let idx = self
            .buildings
            .iter()
            .position(|b| b.id == id)
            .ok_or(ErrorCode::UnknownBuilding)?;
        if self.moves_remaining == 0 {
            return Err(ErrorCode::NoMovesLeft);
        }
        self.check_placement(pos)?;
        if !self.path_survives(Some(pos), Some(id)) {
            return Err(ErrorCode::PathBlocked);
        }
        self.buildings[idx].pos = pos;
        self.moves_remaining -= 1;
        self.metrics.moves_used += 1;
        Ok(())
    }

    /// Simule la vague courante, applique pertes et revenu, passe à la suivante.
    pub fn resolve_wave(&mut self) {
        let report = sim::run_wave(self);
        self.lives = self.lives.saturating_sub(report.lives_lost);
        for (id, dmg) in &report.damage_by_building {
            if let Some(b) = self.buildings.iter_mut().find(|b| b.id == *id) {
                b.damage_dealt += dmg;
            }
        }
        self.last_report = Some(report);
        if self.lives == 0 {
            self.phase = Phase::GameOver;
            return;
        }
        self.wave += 1;
        self.gold += self.income();
        self.moves_remaining = MOVES_PER_ROUND;
        self.actions_used = 0;
    }

    /// Score = dernière vague atteinte.
    pub fn score(&self) -> u32 {
        self.wave
    }
}

/// Dégâts d'un coup de `kind` sur `target`. Règle unique, partagée par la
/// simulation et les agents (aucune divergence possible).
pub fn effective_damage(kind: BuildingType, target: EnemyKind) -> u32 {
    let st = kind.stats();
    if target == EnemyKind::Armor {
        if st.bonus_vs_armor > 0 {
            return st.damage + st.bonus_vs_armor; // le spécialiste perce le blindage
        }
        return st.damage.saturating_sub(target.plating()).max(1);
    }
    st.damage
}

// ---------------------------------------------------------------- Pathfinding

/// Ordre d'exploration figé — c'est le tie-breaking documenté entre chemins de
/// même longueur : Droite, Bas, Haut, Gauche.
pub const DIRECTIONS: [(i32, i32); 4] = [(1, 0), (0, 1), (0, -1), (-1, 0)];

/// BFS coût uniforme. Le premier parent atteint gagne → chemin unique et stable.
pub fn shortest_path(from: Pos, to: Pos, blocked: &[Pos]) -> Option<Vec<Pos>> {
    let idx = |p: Pos| (p.y * BOARD_W + p.x) as usize;
    let mut prev = vec![usize::MAX; (BOARD_W * BOARD_H) as usize];
    let mut wall = vec![false; (BOARD_W * BOARD_H) as usize];
    for b in blocked {
        wall[idx(*b)] = true;
    }
    if wall[idx(from)] || wall[idx(to)] {
        return None;
    }
    let mut queue = VecDeque::from([from]);
    prev[idx(from)] = idx(from);
    while let Some(p) = queue.pop_front() {
        if p == to {
            let mut path = vec![to];
            let mut cur = idx(to);
            while cur != idx(from) {
                cur = prev[cur];
                path.push(Pos::new(cur as i32 % BOARD_W, cur as i32 / BOARD_W));
            }
            path.reverse();
            return Some(path);
        }
        for (dx, dy) in DIRECTIONS {
            let n = Pos::new(p.x + dx, p.y + dy);
            if n.x < 0 || n.y < 0 || n.x >= BOARD_W || n.y >= BOARD_H {
                continue;
            }
            if wall[idx(n)] || prev[idx(n)] != usize::MAX {
                continue;
            }
            prev[idx(n)] = idx(p);
            queue.push_back(n);
        }
    }
    None
}

/// Trajet des volants : ligne droite (Bresenham) entrée → sortie.
pub fn straight_line(from: Pos, to: Pos) -> Vec<Pos> {
    let (dx, dy) = ((to.x - from.x).abs(), -(to.y - from.y).abs());
    let (sx, sy) = (
        if from.x < to.x { 1 } else { -1 },
        if from.y < to.y { 1 } else { -1 },
    );
    let mut err = dx + dy;
    let mut cur = from;
    let mut path = vec![cur];
    while cur != to {
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            cur.x += sx;
        } else {
            err += dx;
            cur.y += sy;
        }
        path.push(cur);
    }
    path
}

// ---------------------------------------------------------------- Vagues

/// Composition de vague seedée. Croissance continue : jamais saturée, pas de
/// palier — c'est le gradient de difficulté du benchmark.
pub fn wave_composition(seed: u64, wave: u32) -> Composition {
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(wave as u64));
    let w = wave as u64;
    let infantry = 3 + w + rng.below(w / 2 + 2);
    let armor = if wave >= 5 {
        (w - 3) / 2 + rng.below(2)
    } else {
        0
    };
    let flyer = if wave >= 8 {
        (w - 6) / 3 + rng.below(2)
    } else {
        0
    };
    [infantry as u32, armor as u32, flyer as u32]
}

/// `incoming_intel` : annonce textuelle de la vague `wave` (README §2).
/// Mode `minimal` — le mode `detailed` (lore noyé) arrive en v0.4.
pub fn incoming_intel(seed: u64, wave: u32) -> String {
    let c = wave_composition(seed, wave);
    let mut bits = Vec::new();
    if c[1] > 0 {
        bits.push("des moteurs lourds grondent au loin");
    }
    if c[2] > 0 {
        bits.push("le ciel bourdonne");
    }
    if bits.is_empty() {
        bits.push("des colonnes d'infanterie se rassemblent");
    }
    format!("N+2 : {}.", bits.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_is_deterministic_and_tie_broken() {
        let g = Game::new(1);
        let p1 = g.current_path().unwrap();
        let p2 = g.current_path().unwrap();
        assert_eq!(p1, p2);
        // Plateau vide : le BFS part vers la droite avant de descendre.
        assert_eq!(p1[1], Pos::new(1, g.entry.y));
        assert_eq!(*p1.last().unwrap(), g.exit);
        // Longueur = distance de Manhattan + 1 case de départ.
        let d = (g.exit.x - g.entry.x).abs() + (g.exit.y - g.entry.y).abs();
        assert_eq!(p1.len() as i32, d + 1);
    }

    #[test]
    fn blocking_the_exit_is_rejected_and_counted() {
        let mut g = Game::new(1);
        g.gold = 10_000;
        // Mur vertical complet sur x=5, sauf la dernière case.
        for y in 0..BOARD_H - 1 {
            g.apply(Action::Build {
                kind: BuildingType::Eco,
                pos: Pos::new(5, y),
            })
            .unwrap();
        }
        let err = g.apply(Action::Build {
            kind: BuildingType::Eco,
            pos: Pos::new(5, BOARD_H - 1),
        });
        assert_eq!(err, Err(ErrorCode::PathBlocked));
        assert_eq!(g.metrics.get(ErrorCode::PathBlocked), 1);
        assert!(g.current_path().is_some());
        // Le mur allonge bien le chemin (le labyrinthe est la défense).
        assert!(g.current_path().unwrap().len() > 10);
    }

    #[test]
    fn placement_rules() {
        let mut g = Game::new(1);
        let entry = g.entry;
        assert_eq!(
            g.apply(Action::Build {
                kind: BuildingType::Sniper,
                pos: entry
            }),
            Err(ErrorCode::InvalidCell)
        );
        assert_eq!(
            g.apply(Action::Build {
                kind: BuildingType::Sniper,
                pos: Pos::new(99, 0)
            }),
            Err(ErrorCode::InvalidCell)
        );
        g.apply(Action::Build {
            kind: BuildingType::Sniper,
            pos: Pos::new(3, 3),
        })
        .unwrap();
        assert_eq!(
            g.apply(Action::Build {
                kind: BuildingType::Mortar,
                pos: Pos::new(3, 3)
            }),
            Err(ErrorCode::CellOccupied)
        );
        g.gold = 0;
        assert_eq!(
            g.apply(Action::Build {
                kind: BuildingType::Mortar,
                pos: Pos::new(4, 4)
            }),
            Err(ErrorCode::InsufficientGold)
        );
    }

    #[test]
    fn move_is_rationed_and_sell_refunds_half() {
        let mut g = Game::new(1);
        g.apply(Action::Build {
            kind: BuildingType::Sniper,
            pos: Pos::new(3, 3),
        })
        .unwrap();
        g.apply(Action::Move {
            id: 1,
            pos: Pos::new(4, 3),
        })
        .unwrap();
        assert_eq!(g.buildings[0].pos, Pos::new(4, 3));
        assert_eq!(
            g.apply(Action::Move {
                id: 1,
                pos: Pos::new(5, 3)
            }),
            Err(ErrorCode::NoMovesLeft)
        );
        let gold = g.gold;
        g.apply(Action::Sell { id: 1 }).unwrap();
        assert_eq!(g.gold, gold + BuildingType::Sniper.stats().cost / 2);
        assert_eq!(
            g.apply(Action::Sell { id: 42 }),
            Err(ErrorCode::UnknownBuilding)
        );
    }

    #[test]
    fn action_limit_forces_end_turn() {
        let mut g = Game::new(1);
        for _ in 0..ACTION_LIMIT {
            let _ = g.apply(Action::Sell { id: 999 }); // erreurs : elles comptent aussi
        }
        assert_eq!(g.wave, 2);
        assert_eq!(g.metrics.forced_end_turns, 1);
    }

    #[test]
    fn same_seed_same_game() {
        let play = |seed| {
            let mut g = Game::new(seed);
            let mut log = String::new();
            while g.phase == Phase::Preparation && g.wave < 25 {
                let p = g.current_path().unwrap();
                let cell = p[p.len() / 2];
                let _ = g.apply(Action::Build {
                    kind: BuildingType::Sniper,
                    pos: Pos::new(cell.x, (cell.y + 1) % BOARD_H),
                });
                g.apply(Action::EndTurn).unwrap();
                let r = g.last_report.clone().unwrap();
                log += &format!("{}:{:?}/{:?}/{} ", r.wave, r.kills, r.leaked, g.gold);
            }
            log
        };
        assert_eq!(play(7), play(7));
        assert_ne!(play(7), play(8));
    }

    #[test]
    fn doing_nothing_loses_the_game() {
        let mut g = Game::new(3);
        while g.phase == Phase::Preparation && g.wave < 100 {
            g.apply(Action::EndTurn).unwrap();
        }
        assert_eq!(g.phase, Phase::GameOver);
        assert!(
            g.score() < 6,
            "sans défense on doit mourir vite (vague {})",
            g.score()
        );
    }

    #[test]
    fn wave_composition_grows_and_introduces_types() {
        let c1 = wave_composition(1, 1);
        assert_eq!((c1[1], c1[2]), (0, 0), "vague 1 : infanterie seulement");
        assert_eq!(
            wave_composition(1, 4)[2],
            0,
            "pas de volants avant la vague 8"
        );
        assert!(wave_composition(1, 5)[1] > 0, "blindés dès la vague 5");
        assert!(wave_composition(1, 9)[2] > 0, "volants dès la vague 8");
        assert!(wave_composition(1, 20)[0] > wave_composition(1, 2)[0]);
    }
}
