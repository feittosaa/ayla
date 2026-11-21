//! Versão da Ayla e aviso de atualização pelas tags do repositório no GitHub.
//!
//! Só consulta a lista pública de tags (sem conta, sem dados da pessoa), no máximo a cada 6 horas.
//! Sem rede ou sem tags, mostra só a versão instalada.
use serde::Serialize;
use std::{sync::Mutex, time::{Duration, Instant}};

const REPOSITORY: &str = "feittosaa/ayla";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Serialize)]
pub struct Version {
    pub version: String,
    /// Tag mais nova que a versão instalada, se houver.
    pub latest: Option<String>,
    /// Página da versão nova no GitHub.
    pub url: Option<String>,
}

static CACHE: Mutex<Option<(Instant, Option<String>)>> = Mutex::new(None);

fn parse(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Maior tag de versão (`1.2.3` ou `v1.2.3`) mais nova que `current`; ignora outras tags.
pub fn newest<'a>(tags: impl IntoIterator<Item = &'a str>, current: &str) -> Option<String> {
    let current = parse(current)?;
    tags.into_iter()
        .filter_map(|tag| parse(tag).map(|v| (v, tag)))
        .filter(|(v, _)| *v > current)
        .max_by_key(|(v, _)| *v)
        .map(|(_, tag)| tag.to_string())
}

async fn tags() -> Option<Vec<String>> {
    let response = reqwest::Client::new()
        .get(format!("https://api.github.com/repos/{REPOSITORY}/tags?per_page=100"))
        .header("User-Agent", "Ayla")
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let list: Vec<serde_json::Value> = response.json().await.ok()?;
    Some(list.iter().filter_map(|t| t["name"].as_str().map(String::from)).collect())
}

#[tauri::command]
pub async fn app_version(app: tauri::AppHandle) -> Result<Version, String> {
    let version = app.package_info().version.to_string();
    let cached = CACHE.lock().ok().and_then(|c| c.clone()).filter(|(at, _)| at.elapsed() < CHECK_EVERY);
    let latest = match cached {
        Some((_, latest)) => latest,
        None => {
            let latest = tags().await.and_then(|tags| newest(tags.iter().map(String::as_str), &version));
            if let Ok(mut cache) = CACHE.lock() {
                *cache = Some((Instant::now(), latest.clone()));
            }
            latest
        }
    };
    let url = latest.as_ref().map(|tag| format!("https://github.com/{REPOSITORY}/releases/tag/{tag}"));
    Ok(Version { version, latest, url })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_avisa_tag_de_versao_mais_nova() {
        assert_eq!(newest(["0.1.0", "v0.2.0", "0.1.9", "teste", "1.0"], "0.1.0"), Some("v0.2.0".into()));
        assert_eq!(newest(["0.0.9", "0.1.0"], "0.1.0"), None);
        assert_eq!(newest([], "0.1.0"), None, "sem tags, sem aviso");
        assert_eq!(newest(["0.10.0", "0.9.0"], "0.2.0"), Some("0.10.0".into()), "compara números, não texto");
    }
}
