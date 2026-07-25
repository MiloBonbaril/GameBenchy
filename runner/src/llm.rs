//! Adaptateur agent LLM local : API OpenAI-compatible (llama.cpp, Ollama, vLLM).
//!
//! L'agent ne voit que l'observation JSON du contrat §4 — jamais l'état typé.

use crate::agents::Agent;
use engine::*;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct Llm {
    name: String,
    host: String,
    model: Option<String>,
    max_tokens: u32,
    /// Raisonnement du modèle laissé actif (coûteux : cf. README §11).
    thinking: bool,
    tokens: u64,
    parse_failures: u32,
}

impl Llm {
    /// `spec` = `nom@host:port` (ex. `gemma@127.0.0.1:8080`), modèle optionnel
    /// après un `#` pour les serveurs multi-modèles, suffixe `+nothink` pour
    /// couper le raisonnement (indispensable quand le modèle déborde du GPU).
    pub fn new(spec: &str) -> Self {
        let (spec, thinking) = match spec.strip_suffix("+nothink") {
            Some(s) => (s, false),
            None => (spec, true),
        };
        let (name, rest) = spec.split_once('@').unwrap_or(("llm", spec));
        let (host, model) = match rest.split_once('#') {
            Some((h, m)) => (h, Some(m.to_string())),
            None => (rest, None),
        };
        Llm {
            name: name.to_string(),
            host: host
                .trim_start_matches("http://")
                .trim_end_matches('/')
                .into(),
            model,
            max_tokens: 1400,
            thinking,
            tokens: 0,
            parse_failures: 0,
        }
    }

    fn ask(&mut self, obs: &Value, last: Option<ErrorCode>) -> Option<Action> {
        let mut user = format!(
            "État de la partie :\n{}\n\nQuelle est ta prochaine action ?",
            serde_json::to_string_pretty(obs).ok()?
        );
        if let Some(e) = last {
            user = format!(
                "Ton action précédente a été REFUSÉE : {} — {}\n\n{user}",
                e.as_str(),
                e.message()
            );
        }
        let mut body = json!({
            "model": self.model.clone().unwrap_or_default(),
            "messages": [
                {"role": "system", "content": system_prompt()},
                {"role": "user", "content": user}
            ],
            "max_tokens": self.max_tokens,
            "temperature": 0.3,
        });
        if !self.thinking {
            body["chat_template_kwargs"] = json!({"enable_thinking": false});
        }
        let body = body.to_string();

        let raw = post_json(&self.host, "/v1/chat/completions", &body).ok()?;
        let v: Value = serde_json::from_str(&raw).ok()?;
        self.tokens += v["usage"]["total_tokens"].as_u64().unwrap_or(0);
        let msg = &v["choices"][0]["message"];
        // Les modèles à raisonnement peuvent tout mettre dans `reasoning_content`.
        let text = format!(
            "{}{}",
            msg["content"].as_str().unwrap_or_default(),
            msg["reasoning_content"].as_str().unwrap_or_default()
        );
        first_json_object(&text)
            .and_then(|o| serde_json::from_str::<server::ActionDto>(&o).ok())
            .and_then(|dto| dto.to_action().ok())
    }
}

impl Agent for Llm {
    fn name(&self) -> &str {
        &self.name
    }

    fn act(&mut self, _g: &Game, obs: &Value, last: Option<ErrorCode>) -> Action {
        // Un essai, puis un rattrapage : au-delà on clôt le round plutôt que de
        // boucler sur un modèle qui ne sait pas produire de JSON.
        for attempt in 0..2 {
            if let Some(a) = self.ask(obs, last) {
                return a;
            }
            self.parse_failures += 1;
            if attempt == 0 {
                self.max_tokens = 2000;
            }
        }
        Action::EndTurn
    }

    fn tokens(&self) -> u64 {
        self.tokens
    }

    fn parse_failures(&self) -> u32 {
        self.parse_failures
    }
}

/// Les règles, dérivées des constantes du moteur : le prompt ne peut pas mentir.
fn system_prompt() -> String {
    let shop = BUILDING_TYPES
        .iter()
        .map(|k| {
            let s = k.stats();
            format!(
                "- {} : {} or, portée² {}, dégâts {}{}{}{}",
                s.name,
                s.cost,
                s.range_sq,
                s.damage,
                if s.bonus_vs_armor > 0 {
                    format!(" (+{} contre les blindés)", s.bonus_vs_armor)
                } else {
                    String::new()
                },
                if s.splash_sq > 0 { ", zone" } else { "" },
                if s.hits_air {
                    ", touche les volants".to_string()
                } else if s.income > 0 {
                    format!(", ne tire pas, +{} or/vague", s.income)
                } else {
                    ", ne touche pas les volants".to_string()
                },
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Tu joues à un tower defense sur une grille {BOARD_W}x{BOARD_H} (x vers la droite, y vers le bas).\n\
         Les ennemis entrent en `entry` et marchent vers `exit` par le plus court chemin ; \
         tes bâtiments sont des murs : ils allongent ce chemin. Les volants ignorent le labyrinthe \
         et volent en ligne droite. Chaque fuyard coûte des vies ; à 0 vie la partie est finie.\n\
         Ton but : survivre au plus grand nombre de vagues.\n\n\
         Bâtiments :\n{shop}\n\n\
         Règles :\n\
         - Bloquer complètement le chemin est interdit (erreur PATH_BLOCKED).\n\
         - Impossible de bâtir sur `entry`, `exit` ou une case occupée.\n\
         - Vendre rembourse 50 %. Un seul déplacement gratuit par round.\n\
         - {ACTION_LIMIT} actions maximum par round, erreurs comprises ; au-delà le round est clôturé d'office.\n\
         - `end_turn` lance la vague : ne le fais que quand tu as fini d'acheter.\n\n\
         Réponds UNIQUEMENT par un objet JSON, sans commentaire, dans l'un de ces formats :\n\
         {{\"action\":\"build\",\"building_type\":\"sniper\",\"x\":3,\"y\":5}}\n\
         {{\"action\":\"sell\",\"building_id\":\"b2\"}}\n\
         {{\"action\":\"move\",\"building_id\":\"b1\",\"x\":4,\"y\":6}}\n\
         {{\"action\":\"end_turn\"}}"
    )
}

/// Premier objet JSON équilibré du texte (les modèles enrobent : ```json, prose…).
fn first_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for (i, c) in text[start..].char_indices() {
        match c {
            _ if esc => esc = false,
            '\\' if in_str => esc = true,
            '"' => in_str = !in_str,
            '{' if !in_str => depth += 1,
            '}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// ponytail: HTTP/1.1 minimal sur TCP — un POST JSON non streamé vers localhost.
/// Pas de TLS, pas de chunked : on force `Connection: close` et on lit jusqu'à EOF.
/// Passer à un vrai client le jour où l'endpoint sort de la machine.
fn post_json(host: &str, path: &str, body: &str) -> std::io::Result<String> {
    let mut s = TcpStream::connect(host)?;
    s.set_read_timeout(Some(Duration::from_secs(300)))?;
    write!(
        s,
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut raw = String::new();
    s.read_to_string(&mut raw)?;
    raw.split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .ok_or_else(|| std::io::Error::other("réponse HTTP malformée"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_action_from_a_chatty_answer() {
        let text = "Voici mon coup :\n```json\n{\"action\": \"build\", \
                    \"building_type\": \"sniper\", \"x\": 3, \"y\": 2}\n```\nVoilà.";
        let obj = first_json_object(text).unwrap();
        let dto: server::ActionDto = serde_json::from_str(&obj).unwrap();
        assert!(matches!(
            dto.to_action().unwrap(),
            Action::Build {
                kind: BuildingType::Sniper,
                ..
            }
        ));
        // Accolade dans une chaîne : ne doit pas fermer l'objet trop tôt.
        let tricky = r#"blah {"action":"sell","building_id":"b{1}"} fin"#;
        assert_eq!(
            first_json_object(tricky).unwrap(),
            r#"{"action":"sell","building_id":"b{1}"}"#
        );
        assert!(first_json_object("aucun json ici").is_none());
    }

    #[test]
    fn the_prompt_states_the_real_prices() {
        let p = system_prompt();
        assert!(p.contains("sniper : 50 or"), "{p}");
        assert!(p.contains("20 actions maximum"), "{p}");
    }
}
