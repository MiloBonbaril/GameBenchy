//! API HTTP : 3 endpoints, erreurs catégorisées, persistance SQLite.
//!
//! Persistance = seed + log d'actions. L'état n'est **jamais** stocké : il est
//! rejoué depuis le log à chaque requête. Le moteur étant déterministe, le log
//! est la seule source de vérité — toute partie est rejouable et auditable.
//!
//! `cargo run -p server -- [addr] [db]`  (défaut : 127.0.0.1:3000 gamebenchy.db)

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use engine::*;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

type Db = Arc<Mutex<Connection>>;
type Reply = (StatusCode, Json<Value>);

// ---------------------------------------------------------------- Entrées

#[derive(Deserialize)]
struct NewGame {
    seed: u64,
    #[serde(default)]
    mode: Mode,
}

#[derive(Deserialize, Serialize, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Minimal,
    Detailed,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Minimal => "minimal",
            Mode::Detailed => "detailed",
        }
    }
}

#[derive(Deserialize)]
struct GameId {
    game_id: String,
}

/// Le contrat agent, mot pour mot (README §4).
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ActionDto {
    Build {
        building_type: String,
        x: i32,
        y: i32,
    },
    Sell {
        building_id: String,
    },
    Move {
        building_id: String,
        x: i32,
        y: i32,
    },
    EndTurn,
}

/// `"b12"` → 12. Tout autre format est un bâtiment inconnu.
fn parse_id(s: &str) -> Result<u32, ErrorCode> {
    s.strip_prefix('b')
        .and_then(|n| n.parse().ok())
        .ok_or(ErrorCode::UnknownBuilding)
}

impl ActionDto {
    fn to_action(&self) -> Result<Action, ErrorCode> {
        Ok(match self {
            ActionDto::Build {
                building_type,
                x,
                y,
            } => Action::Build {
                kind: BuildingType::from_name(building_type).ok_or(ErrorCode::UnknownBuilding)?,
                pos: Pos::new(*x, *y),
            },
            ActionDto::Sell { building_id } => Action::Sell {
                id: parse_id(building_id)?,
            },
            ActionDto::Move { building_id, x, y } => Action::Move {
                id: parse_id(building_id)?,
                pos: Pos::new(*x, *y),
            },
            ActionDto::EndTurn => Action::EndTurn,
        })
    }
}

// ---------------------------------------------------------------- Sorties

#[derive(Serialize)]
struct StateDto {
    game_id: String,
    seed: u64,
    mode: &'static str,
    wave: u32,
    phase: &'static str,
    lives: u32,
    gold: u32,
    income_per_wave: u32,
    board: BoardDto,
    current_path: Vec<PosDto>,
    next_wave: NextWave,
    incoming_intel: String,
    moves_remaining: u32,
    actions_remaining: u32,
    shop: Vec<ShopItem>,
    last_wave_report: Option<ReportDto>,
}

#[derive(Serialize)]
struct PosDto {
    x: i32,
    y: i32,
}

#[derive(Serialize)]
struct BoardDto {
    width: i32,
    height: i32,
    entry: PosDto,
    exit: PosDto,
    buildings: Vec<BuildingDto>,
}

#[derive(Serialize)]
struct BuildingDto {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    x: i32,
    y: i32,
    damage_dealt: u32,
}

#[derive(Serialize)]
struct CompDto {
    infantry: u32,
    armor: u32,
    flyer: u32,
}

#[derive(Serialize)]
struct NextWave {
    composition: CompDto,
}

#[derive(Serialize)]
struct ShopItem {
    #[serde(rename = "type")]
    kind: &'static str,
    cost: u32,
}

#[derive(Serialize)]
struct ReportDto {
    wave: u32,
    kills: CompDto,
    leaked: CompDto,
    lives_lost: u32,
    damage_by_building: BTreeMap<String, u32>,
}

fn comp(c: Composition) -> CompDto {
    CompDto {
        infantry: c[0],
        armor: c[1],
        flyer: c[2],
    }
}

fn pos(p: Pos) -> PosDto {
    PosDto { x: p.x, y: p.y }
}

fn state_dto(game_id: &str, g: &Game, mode: Mode) -> StateDto {
    StateDto {
        game_id: game_id.to_string(),
        seed: g.seed,
        mode: mode.as_str(),
        wave: g.wave,
        phase: match g.phase {
            Phase::Preparation => "preparation",
            Phase::GameOver => "game_over",
        },
        lives: g.lives,
        gold: g.gold,
        income_per_wave: g.income(),
        board: BoardDto {
            width: BOARD_W,
            height: BOARD_H,
            entry: pos(g.entry),
            exit: pos(g.exit),
            buildings: g
                .buildings
                .iter()
                .map(|b| BuildingDto {
                    id: format!("b{}", b.id),
                    kind: b.kind.stats().name,
                    x: b.pos.x,
                    y: b.pos.y,
                    damage_dealt: b.damage_dealt,
                })
                .collect(),
        },
        current_path: g
            .current_path()
            .unwrap_or_default()
            .into_iter()
            .map(pos)
            .collect(),
        next_wave: NextWave {
            composition: comp(g.composition(g.wave)),
        },
        // ponytail: le mode `detailed` (lore noyé, needle-in-haystack) est v0.4 ;
        // le champ est figé au contrat dès maintenant, le texte reste `minimal`.
        incoming_intel: incoming_intel(g.seed, g.wave + 1),
        moves_remaining: g.moves_remaining,
        actions_remaining: ACTION_LIMIT - g.actions_used,
        shop: BUILDING_TYPES
            .iter()
            .map(|k| ShopItem {
                kind: k.stats().name,
                cost: k.stats().cost,
            })
            .collect(),
        last_wave_report: g.last_report.as_ref().map(|r| ReportDto {
            wave: r.wave,
            kills: comp(r.kills),
            leaked: comp(r.leaked),
            lives_lost: r.lives_lost,
            damage_by_building: r
                .damage_by_building
                .iter()
                .map(|(id, d)| (format!("b{id}"), *d))
                .collect(),
        }),
    }
}

fn ok(game_id: &str, g: &Game, mode: Mode) -> Reply {
    (
        StatusCode::OK,
        Json(json!({"ok": true, "state": state_dto(game_id, g, mode)})),
    )
}

fn fail(status: StatusCode, code: &str, message: &str) -> Reply {
    (
        status,
        Json(json!({"ok": false, "error_code": code, "message": message})),
    )
}

fn unknown_game() -> Reply {
    fail(
        StatusCode::NOT_FOUND,
        "UNKNOWN_GAME",
        "Partie inconnue ou supprimée.",
    )
}

// ---------------------------------------------------------------- Persistance

fn schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS games (
             id   INTEGER PRIMARY KEY AUTOINCREMENT,
             seed INTEGER NOT NULL,
             mode TEXT    NOT NULL);
         CREATE TABLE IF NOT EXISTS actions (
             game_id INTEGER NOT NULL,
             n       INTEGER NOT NULL,
             payload TEXT    NOT NULL,
             PRIMARY KEY (game_id, n));",
    )
}

/// `"g12"` → 12.
fn parse_game_id(game_id: &str) -> Option<i64> {
    game_id.strip_prefix('g')?.parse().ok()
}

/// Rejoue la partie depuis son log. Les actions en erreur sont rejouées aussi :
/// elles consomment un crédit d'action et alimentent les métriques.
fn load(conn: &Connection, rowid: i64) -> Option<(Game, Mode)> {
    let (seed, mode): (i64, String) = conn
        .query_row("SELECT seed, mode FROM games WHERE id = ?1", [rowid], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .ok()?;
    let mut stmt = conn
        .prepare("SELECT payload FROM actions WHERE game_id = ?1 ORDER BY n")
        .ok()?;
    let payloads: Vec<String> = stmt
        .query_map([rowid], |r| r.get(0))
        .ok()?
        .filter_map(|r| r.ok())
        .collect();

    // ponytail: rejeu complet à chaque requête (une partie = quelques milliers de
    // ticks). Cache en mémoire le jour où la latence du rejeu se voit.
    let mut g = Game::new(seed as u64);
    for p in payloads {
        if let Ok(dto) = serde_json::from_str::<ActionDto>(&p)
            && let Ok(a) = dto.to_action()
        {
            let _ = g.apply(a);
        }
    }
    let mode = if mode == Mode::Detailed.as_str() {
        Mode::Detailed
    } else {
        Mode::Minimal
    };
    Some((g, mode))
}

// ---------------------------------------------------------------- Endpoints

async fn new_game(State(db): State<Db>, Json(req): Json<NewGame>) -> Reply {
    let conn = db.lock().unwrap();
    if conn
        .execute(
            "INSERT INTO games (seed, mode) VALUES (?1, ?2)",
            rusqlite::params![req.seed as i64, req.mode.as_str()],
        )
        .is_err()
    {
        return fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE",
            "Écriture impossible.",
        );
    }
    let game_id = format!("g{}", conn.last_insert_rowid());
    ok(&game_id, &Game::new(req.seed), req.mode)
}

async fn get_state(State(db): State<Db>, Query(q): Query<GameId>) -> Reply {
    let conn = db.lock().unwrap();
    match parse_game_id(&q.game_id).and_then(|id| load(&conn, id)) {
        Some((g, mode)) => ok(&q.game_id, &g, mode),
        None => unknown_game(),
    }
}

async fn post_action(
    State(db): State<Db>,
    Query(q): Query<GameId>,
    Json(payload): Json<Value>,
) -> Reply {
    let conn = db.lock().unwrap();
    let Some((rowid, (mut g, mode))) =
        parse_game_id(&q.game_id).and_then(|id| Some((id, load(&conn, id)?)))
    else {
        return unknown_game();
    };
    let Ok(dto) = serde_json::from_value::<ActionDto>(payload.clone()) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "UNKNOWN_ACTION",
            "Action inconnue ou champs manquants.",
        );
    };
    let action = match dto.to_action() {
        Ok(a) => a,
        Err(code) => return fail(StatusCode::BAD_REQUEST, code.as_str(), code.message()),
    };

    let result = g.apply(action);
    // Journalisée même en erreur : l'erreur fait partie de l'état (crédit d'action
    // consommé, métrique incrémentée) — sans elle le rejeu diverge.
    let _ = conn.execute(
        "INSERT INTO actions (game_id, n, payload)
         VALUES (?1, (SELECT COALESCE(MAX(n), 0) + 1 FROM actions WHERE game_id = ?1), ?2)",
        rusqlite::params![rowid, payload.to_string()],
    );
    match result {
        Ok(()) => ok(&q.game_id, &g, mode),
        Err(code) => fail(StatusCode::BAD_REQUEST, code.as_str(), code.message()),
    }
}

fn app(conn: Connection) -> Router {
    schema(&conn).expect("schéma SQLite");
    Router::new()
        .route("/game", post(new_game))
        .route("/state", get(get_state))
        .route("/action", post(post_action))
        .with_state(Arc::new(Mutex::new(conn)))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:3000".into());
    let path = args.next().unwrap_or_else(|| "gamebenchy.db".into());
    let conn = Connection::open(&path).expect("ouverture SQLite");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("gamebenchy sur http://{addr} (base {path})");
    axum::serve(listener, app(conn)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn call(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// Une partie par HTTP : création, action légale, action illégale, vague
    /// résolue — et l'état relu est reconstruit à l'identique depuis le log.
    #[tokio::test]
    async fn http_round_trip_is_replayed_from_the_log() {
        let app = app(Connection::open_in_memory().unwrap());
        let (st, r) = call(&app, "POST", "/game", json!({"seed": 42})).await;
        assert_eq!(st, StatusCode::OK);
        let id = r["state"]["game_id"].as_str().unwrap().to_string();
        assert_eq!(r["state"]["gold"], START_GOLD);
        assert_eq!(r["state"]["mode"], "minimal");
        assert_eq!(r["state"]["shop"][0]["type"], "sniper");
        assert!(!r["state"]["current_path"].as_array().unwrap().is_empty());

        let url = format!("/action?game_id={id}");
        let build = json!({"action": "build", "building_type": "sniper", "x": 3, "y": 2});
        let (st, _) = call(&app, "POST", &url, build.clone()).await;
        assert_eq!(st, StatusCode::OK);

        // Erreur catégorisée : même case, deux fois. Le crédit est consommé quand même.
        let (st, r) = call(&app, "POST", &url, build).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(r["error_code"], "CELL_OCCUPIED");
        let state_url = format!("/state?game_id={id}");
        let (_, r) = call(&app, "GET", &state_url, Value::Null).await;
        assert_eq!(r["state"]["actions_remaining"], ACTION_LIMIT - 2);

        let (_, r) = call(&app, "POST", &url, json!({"action": "end_turn"})).await;
        assert_eq!(r["state"]["wave"], 2);
        assert!(
            r["state"]["last_wave_report"]["kills"]["infantry"]
                .as_u64()
                .unwrap()
                > 0
        );

        let (_, s) = call(&app, "GET", &state_url, Value::Null).await;
        assert_eq!(s["state"], r["state"], "le rejeu doit rendre le même état");
        assert_eq!(s["state"]["board"]["buildings"][0]["id"], "b1");

        let (st, r) = call(&app, "GET", "/state?game_id=g999", Value::Null).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert_eq!(r["error_code"], "UNKNOWN_GAME");
    }

    /// Un type de bâtiment inconnu est une erreur de protocole, pas un panic.
    #[tokio::test]
    async fn unknown_building_type_is_categorised() {
        let app = app(Connection::open_in_memory().unwrap());
        let (_, r) = call(
            &app,
            "POST",
            "/game",
            json!({"seed": 1, "mode": "detailed"}),
        )
        .await;
        let id = r["state"]["game_id"].as_str().unwrap().to_string();
        assert_eq!(r["state"]["mode"], "detailed");
        let url = format!("/action?game_id={id}");
        let (st, r) = call(
            &app,
            "POST",
            &url,
            json!({"action": "build", "building_type": "railgun", "x": 3, "y": 2}),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(r["error_code"], "UNKNOWN_BUILDING");
        let (st, r) = call(&app, "POST", &url, json!({"action": "teleport"})).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(r["error_code"], "UNKNOWN_ACTION");
    }
}
