//! Client humain : jouer, équilibrer, débugger.
//!
//! `cargo run -p tui -- [seed]`

use engine::*;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{DefaultTerminal, Frame};

struct App {
    game: Game,
    seed: u64,
    cursor: Pos,
    /// Bâtiment sélectionné pour un déplacement.
    moving: Option<u32>,
    message: String,
}

impl App {
    fn new(seed: u64) -> Self {
        App {
            game: Game::new(seed),
            seed,
            cursor: Pos::new(BOARD_W / 2, BOARD_H / 2),
            moving: None,
            message: String::from("Construisez, puis Entrée pour lancer la vague."),
        }
    }

    fn act(&mut self, action: Action) {
        match self.game.apply(action) {
            Ok(()) => self.message.clear(),
            Err(e) => self.message = format!("{} — {}", e.as_str(), e.message()),
        }
    }

    fn key(&mut self, code: KeyCode) -> bool {
        let (dx, dy) = match code {
            KeyCode::Left | KeyCode::Char('h') => (-1, 0),
            KeyCode::Right | KeyCode::Char('l') => (1, 0),
            KeyCode::Up | KeyCode::Char('k') => (0, -1),
            KeyCode::Down | KeyCode::Char('j') => (0, 1),
            _ => (0, 0),
        };
        if (dx, dy) != (0, 0) {
            self.cursor.x = (self.cursor.x + dx).clamp(0, BOARD_W - 1);
            self.cursor.y = (self.cursor.y + dy).clamp(0, BOARD_H - 1);
            return true;
        }
        match code {
            KeyCode::Char('q') => return false,
            KeyCode::Char('r') => *self = App::new(self.seed),
            KeyCode::Char(c @ '1'..='5') => {
                let kind = BUILDING_TYPES[c as usize - '1' as usize];
                self.act(Action::Build {
                    kind,
                    pos: self.cursor,
                });
            }
            KeyCode::Char('s') => match self.game.building_at(self.cursor) {
                Some(b) => {
                    let id = b.id;
                    self.act(Action::Sell { id });
                }
                None => self.message = "Rien à vendre ici.".into(),
            },
            KeyCode::Char('m') => match self.moving {
                None => match self.game.building_at(self.cursor) {
                    Some(b) => {
                        self.moving = Some(b.id);
                        self.message =
                            "Déplacement : choisissez la case, Entrée pour valider.".into();
                    }
                    None => self.message = "Aucun bâtiment sous le curseur.".into(),
                },
                Some(_) => {
                    self.moving = None;
                    self.message = "Déplacement annulé.".into();
                }
            },
            KeyCode::Enter | KeyCode::Char(' ') => match self.moving.take() {
                Some(id) => self.act(Action::Move {
                    id,
                    pos: self.cursor,
                }),
                None => self.act(Action::EndTurn),
            },
            _ => {}
        }
        true
    }
}

fn main() -> std::io::Result<()> {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1);
    let terminal = ratatui::init();
    let res = run(terminal, App::new(seed));
    ratatui::restore();
    res
}

fn run(mut terminal: DefaultTerminal, mut app: App) -> std::io::Result<()> {
    loop {
        terminal.draw(|f| draw(f, &app))?;
        if let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
            && !app.key(k.code)
        {
            return Ok(());
        }
    }
}

fn draw(f: &mut Frame, app: &App) {
    let [top, status] =
        Layout::vertical([Constraint::Min(12), Constraint::Length(3)]).areas(f.area());
    let [left, right] =
        Layout::horizontal([Constraint::Length(26), Constraint::Min(30)]).areas(top);
    board(f, left, app);
    panel(f, right, app);

    let msg = if app.game.phase == Phase::GameOver {
        format!(
            "PARTIE TERMINÉE — vague atteinte : {}. [r] rejouer  [q] quitter",
            app.game.score()
        )
    } else {
        format!(
            "{}   |  hjkl  1-5 bâtir  s vendre  m déplacer  ⏎ vague  q quitter",
            app.message
        )
    };
    f.render_widget(Paragraph::new(msg).block(Block::bordered()), status);
}

fn board(f: &mut Frame, area: Rect, app: &App) {
    let g = &app.game;
    let path = g.current_path().unwrap_or_default();
    let mut lines = Vec::new();
    for y in 0..BOARD_H {
        let mut spans = Vec::new();
        for x in 0..BOARD_W {
            let p = Pos::new(x, y);
            let (ch, mut style) = if p == g.entry {
                ('E', Style::new().fg(Color::Green).bold())
            } else if p == g.exit {
                ('X', Style::new().fg(Color::Red).bold())
            } else if let Some(b) = g.building_at(p) {
                let color = if Some(b.id) == app.moving {
                    Color::Magenta
                } else {
                    Color::Yellow
                };
                (b.kind.glyph(), Style::new().fg(color).bold())
            } else if path.contains(&p) {
                ('·', Style::new().fg(Color::Cyan))
            } else {
                ('.', Style::new().fg(Color::DarkGray))
            };
            if p == app.cursor {
                style = style.add_modifier(Modifier::REVERSED);
            }
            spans.push(Span::styled(format!("{ch} "), style));
        }
        lines.push(Line::from(spans));
    }
    let title = format!(" chemin {} ", path.len());
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(title)),
        area,
    );
}

fn panel(f: &mut Frame, area: Rect, app: &App) {
    let g = &app.game;
    let comp = g.composition(g.wave);
    let mut lines = vec![
        Line::from(vec![
            Span::raw(format!("vague {}   ", g.wave)),
            Span::styled(format!("♥ {}", g.lives), Style::new().fg(Color::Red)),
            Span::styled(format!("   ⛁ {}", g.gold), Style::new().fg(Color::Yellow)),
        ]),
        Line::from(format!(
            "revenu {}/vague   déplacements {}   actions {}/{}",
            g.income(),
            g.moves_remaining,
            g.actions_used,
            ACTION_LIMIT
        )),
        Line::from(""),
        Line::from("VAGUE ENTRANTE".bold()),
        Line::from(composition_line(comp)),
        Line::from(Span::styled(
            intel(g.seed, g.wave + 1),
            Style::new().italic(),
        )),
        Line::from(""),
        Line::from("BÂTIMENTS".bold()),
    ];
    for (i, kind) in BUILDING_TYPES.iter().enumerate() {
        let st = kind.stats();
        let affordable = if st.cost <= g.gold {
            Color::White
        } else {
            Color::DarkGray
        };
        lines.push(Line::styled(
            format!(
                "  [{}] {} {:<13} {:>3} or  {}",
                i + 1,
                kind.glyph(),
                st.name,
                st.cost,
                describe(*kind)
            ),
            Style::new().fg(affordable),
        ));
    }

    if let Some(r) = &g.last_report {
        lines.push(Line::from(""));
        lines.push(Line::from(format!("RAPPORT VAGUE {}", r.wave).bold()));
        lines.push(Line::from(format!(
            "  tués {} · fuites {} · -{} vies",
            composition_line(r.kills),
            composition_line(r.leaked),
            r.lives_lost
        )));
        let dmg: Vec<String> = r
            .damage_by_building
            .iter()
            .map(|(id, d)| format!("b{id}:{d}"))
            .collect();
        if !dmg.is_empty() {
            lines.push(Line::from(format!("  dégâts {}", dmg.join(" "))));
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" état ")),
        area,
    );
}

fn composition_line(c: Composition) -> String {
    ENEMY_KINDS
        .iter()
        .filter(|k| c[k.index()] > 0)
        .map(|k| format!("{}{} {}", k.glyph(), c[k.index()], k.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn describe(kind: BuildingType) -> &'static str {
    match kind {
        BuildingType::Sniper => "longue portée, seul anti-aérien",
        BuildingType::Flamethrower => "AoE au contact, aveugle au ciel",
        BuildingType::AntiArmor => "perce le blindage, faible sinon",
        BuildingType::Mortar => "AoE longue portée, cadence lente",
        BuildingType::Eco => "+8 or/vague, ne défend pas",
    }
}

/// `incoming_intel` v0.1 : lore sec (mode `minimal` du contrat agent).
fn intel(seed: u64, wave: u32) -> String {
    let c = wave_composition(seed, wave);
    if wave > 100 {
        return String::new();
    }
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
    use ratatui::{Terminal, backend::TestBackend};

    /// Une touche → une action moteur → un rendu lisible : la boucle complète.
    #[test]
    fn keys_play_the_game_and_the_frame_shows_it() {
        let mut app = App::new(1);
        app.key(KeyCode::Char('1')); // sniper sous le curseur
        assert_eq!(app.game.buildings.len(), 1);
        app.key(KeyCode::Enter); // lance la vague
        assert_eq!(app.game.wave, 2);

        let mut term = Terminal::new(TestBackend::new(100, 32)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
        let screen: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(screen.contains("vague 2"), "{screen}");
        assert!(screen.contains("RAPPORT VAGUE 1"), "{screen}");
        assert!(screen.contains("E"), "entrée absente du plateau");
        assert!(!app.key(KeyCode::Char('q')), "q doit quitter");
    }
}
