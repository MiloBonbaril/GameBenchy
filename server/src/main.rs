//! Point d'entrée du serveur. Tout le contrat vit dans `lib.rs`.
//!
//! `cargo run -p server -- [addr] [db]`  (défaut : 127.0.0.1:3000 gamebenchy.db)

use rusqlite::Connection;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:3000".into());
    let path = args.next().unwrap_or_else(|| "gamebenchy.db".into());
    let conn = Connection::open(&path).expect("ouverture SQLite");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("gamebenchy sur http://{addr} (base {path})");
    axum::serve(listener, server::app(conn)).await
}
