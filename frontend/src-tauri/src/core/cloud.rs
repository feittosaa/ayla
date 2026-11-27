//! IA na nuvem: plataformas, logins (OpenRouter e "Entrar com ChatGPT"), chaves
//! e streaming das respostas. O que pode ser enviado continua decidido pela
//! política de privacidade (`policy`) e pela revisão explícita na conversa.
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Api {
    /// `POST /chat/completions` compatível com a OpenAI.
    Compatible,
    /// API nativa de Messages da Anthropic.
    Anthropic,
    /// Responses API com o plano do ChatGPT ("Entrar com ChatGPT").
    ChatGpt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Login {
    Key,
    OpenRouter,
    ChatGpt,
}
/// Como a plataforma cria imagens (quando cria).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageApi {
    /// `POST /images/generations` compatível com a OpenAI (OpenAI, Gemini, xAI).
    Generations,
    /// `chat/completions` com saída de imagem (OpenRouter).
    OpenRouterChat,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub api: Api,
    pub login: Login,
    pub base: &'static str,
    /// Página da conta onde a pessoa cria a chave de API.
    pub keys_url: Option<&'static str>,
    /// Modelo sugerido quando a plataforma é escolhida (a lista real vem da conta).
    pub default_model: Option<&'static str>,
    /// Criação de imagens por esta plataforma (o plano do ChatGPT não cria em outros apps).
    pub images: Option<ImageApi>,
}

pub const CHATGPT_USAGE: &str = "https://chatgpt.com/settings/usage";
const CHATGPT_AUTH: &str = "https://auth.openai.com/api/accounts/authorize";
const CHATGPT_TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const CHATGPT_ISSUER: &str = "https://auth.openai.com";
const OPENAI_API: &str = "https://api.openai.com/v1";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";

pub fn providers() -> &'static [Provider] {
    &[
        Provider { id: "chatgpt", name: "ChatGPT", summary: "Usa o seu plano Plus ou Pro, sem chave de API", api: Api::ChatGpt, login: Login::ChatGpt, base: OPENAI_API, keys_url: None, default_model: None, images: None },
        Provider { id: "openrouter", name: "OpenRouter", summary: "Um login para modelos de várias empresas", api: Api::Compatible, login: Login::OpenRouter, base: "https://openrouter.ai/api/v1", keys_url: Some("https://openrouter.ai/settings/keys"), default_model: None, images: Some(ImageApi::OpenRouterChat) },
        Provider { id: "openai", name: "OpenAI", summary: "API da OpenAI, cobrada por uso", api: Api::Compatible, login: Login::Key, base: OPENAI_API, keys_url: Some("https://platform.openai.com/api-keys"), default_model: None, images: Some(ImageApi::Generations) },
        Provider { id: "anthropic", name: "Anthropic (Claude)", summary: "API do Claude, cobrada por uso", api: Api::Anthropic, login: Login::Key, base: "https://api.anthropic.com/v1", keys_url: Some("https://platform.claude.com/settings/keys"), default_model: Some("claude-sonnet-5-5"), images: None },
        Provider { id: "gemini", name: "Google Gemini", summary: "API do Gemini (Google AI Studio)", api: Api::Compatible, login: Login::Key, base: "https://generativelanguage.googleapis.com/v1beta/openai", keys_url: Some("https://aistudio.google.com/apikey"), default_model: None, images: Some(ImageApi::Generations) },
        Provider { id: "mistral", name: "Mistral", summary: "API da Mistral AI", api: Api::Compatible, login: Login::Key, base: "https://api.mistral.ai/v1", keys_url: Some("https://console.mistral.ai/api-keys"), default_model: None, images: None },
        Provider { id: "deepseek", name: "DeepSeek", summary: "API da DeepSeek", api: Api::Compatible, login: Login::Key, base: "https://api.deepseek.com/v1", keys_url: Some("https://platform.deepseek.com/api_keys"), default_model: None, images: None },
        Provider { id: "xai", name: "xAI (Grok)", summary: "API da xAI", api: Api::Compatible, login: Login::Key, base: "https://api.x.ai/v1", keys_url: Some("https://console.x.ai"), default_model: None, images: Some(ImageApi::Generations) },
        Provider { id: "groq", name: "Groq", summary: "Modelos abertos com respostas rápidas", api: Api::Compatible, login: Login::Key, base: "https://api.groq.com/openai/v1", keys_url: Some("https://console.groq.com/keys"), default_model: None, images: None },
    ]
}
pub fn provider(id: &str) -> Option<&'static Provider> {
    providers().iter().find(|p| p.id == id)
}
/// Conta no cofre do sistema para os segredos de uma plataforma.
pub fn vault_account(provider: &str) -> String {
    format!("provider:{provider}")
}
/// Links externos que a interface pode abrir: páginas de chaves e de uso.
pub fn allowed_link(url: &str) -> bool {
    url == CHATGPT_USAGE || providers().iter().any(|p| p.keys_url == Some(url))
}

/// Abre o navegador padrão (sem esperar).
pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    command.arg(url).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    command.spawn().map(|_| ()).map_err(|_| "Não foi possível abrir o navegador".into())
}

fn random(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|_| "Sem fonte de números aleatórios do sistema")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buffer))
}
/// Par PKCE (RFC 7636): verificador aleatório e desafio S256.
pub fn pkce() -> Result<(String, String), String> {
    let verifier = random(32)?;
    Ok((challenge(&verifier), verifier))
}
pub fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Espera o navegador voltar para `http://127.0.0.1:<porta><path>` e devolve a consulta.
/// Responde com uma página simples; desiste no cancelamento ou em 5 minutos.
pub struct Callback {
    listeners: Vec<TcpListener>,
    pub port: u16,
}
impl Callback {
    pub fn bind() -> Result<Self, String> {
        let first = TcpListener::bind("127.0.0.1:0").map_err(|_| "Não foi possível preparar o retorno do login")?;
        let port = first.local_addr().map_err(|_| "Porta inválida")?.port();
        let mut listeners = vec![first];
        // Alguns navegadores tentam `localhost` em IPv6 primeiro.
        if let Ok(v6) = TcpListener::bind(("::1", port)) {
            listeners.push(v6);
        }
        for listener in &listeners {
            listener.set_nonblocking(true).map_err(|_| "Não foi possível preparar o retorno do login")?;
        }
        Ok(Self { listeners, port })
    }
    pub async fn wait(self, path: &str, cancel: &CancellationToken) -> Result<Vec<(String, String)>, String> {
        let start = Instant::now();
        loop {
            if cancel.is_cancelled() {
                return Err("Login cancelado".into());
            }
            if start.elapsed() > Duration::from_secs(300) {
                return Err("O login não foi concluído a tempo".into());
            }
            for listener in &self.listeners {
                if let Ok((mut stream, _)) = listener.accept() {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut buffer = [0u8; 8192];
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    let target = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("");
                    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else { continue };
                    if url.path() != path {
                        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                    let page = "<!doctype html><meta charset=utf-8><title>Ayla</title><body style=\"font-family:system-ui;background:#111214;color:#ececee;display:grid;place-items:center;height:90vh\"><p>Pronto. Pode voltar para a Ayla.</p>";
                    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes());
                    return Ok(url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect());
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
fn param<'a>(query: &'a [(String, String)], name: &str) -> Option<&'a str> {
    query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Falha no cliente de rede".into())
}

/// Login no OpenRouter (OAuth PKCE): devolve uma chave de API da conta da pessoa.
pub async fn openrouter_login(cancel: &CancellationToken) -> Result<String, String> {
    let callback = Callback::bind()?;
    let (challenge, verifier) = pkce()?;
    let state = random(16)?;
    let mut url = reqwest::Url::parse("https://openrouter.ai/auth").map_err(|_| "Endereço inválido")?;
    url.query_pairs_mut()
        .append_pair("callback_url", &format!("http://localhost:{}/callback", callback.port))
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("key_label", "Ayla")
        .append_pair("state", &state);
    open_url(url.as_str())?;
    let query = callback.wait("/callback", cancel).await?;
    if param(&query, "state") != Some(state.as_str()) {
        return Err("O retorno do login não confere; tente de novo".into());
    }
    let code = param(&query, "code").ok_or("O OpenRouter não autorizou o acesso")?;
    let response = client()?
        .post("https://openrouter.ai/api/v1/auth/keys")
        .json(&json!({"code": code, "code_verifier": verifier, "code_challenge_method": "S256"}))
        .send().await.map_err(|_| "Não foi possível falar com o OpenRouter")?;
    if !response.status().is_success() {
        return Err("O OpenRouter recusou o login; tente de novo".into());
    }
    let body: Value = response.json().await.map_err(|_| "Resposta inválida do OpenRouter")?;
    body["key"].as_str().filter(|k| !k.is_empty()).map(String::from).ok_or_else(|| "O OpenRouter não devolveu a chave".into())
}

/// Credenciais do "Entrar com ChatGPT", guardadas no cofre do sistema.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ChatGptSession {
    pub client_id: String,
    pub redirect_uri: String,
    pub email: Option<String>,
    pub subject: String,
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix (segundos) em que o acesso expira.
    pub expires_at: i64,
    pub scopes: Vec<String>,
}
impl ChatGptSession {
    pub fn plan_enabled(&self) -> bool {
        self.scopes.iter().any(|s| s == PLAN_SCOPE)
    }
    pub fn needs_refresh(&self, now: i64) -> bool {
        self.expires_at - 60 <= now
    }
}

/// Endereço de autorização do ChatGPT; `previous` reaproveita o cliente já emitido.
pub fn chatgpt_authorize_url(port: u16, host_id: &str, challenge: &str, state: &str, nonce: &str, previous: Option<&ChatGptSession>) -> Result<(reqwest::Url, String), String> {
    let redirect = format!("http://127.0.0.1:{port}/auth/callback");
    let mut url = reqwest::Url::parse(CHATGPT_AUTH).map_err(|_| "Endereço inválido")?;
    {
        let mut q = url.query_pairs_mut();
        match previous {
            Some(session) => {
                q.append_pair("client_id", &session.client_id).append_pair("id_token_hint", &session.id_token);
                if let Some(email) = &session.email {
                    q.append_pair("login_hint", email);
                }
            }
            None => {
                q.append_pair("client_id", "dynamic_agent_client").append_pair("agent_name_hint", "Ayla");
            }
        }
        q.append_pair("ext_agent_host_id", host_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect)
            .append_pair("scope", &format!("openid profile email offline_access resource.invoke {PLAN_SCOPE}"))
            .append_pair("resource", OPENAI_API)
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", challenge);
    }
    Ok((url, redirect))
}

/// Declarações do ID token. Recebido direto do endpoint de token por TLS, o emissor é
/// garantido pelo transporte (OIDC Core 3.1.3.7); ainda conferimos emissor, público, validade e nonce.
pub fn id_claims(id_token: &str, client_id: &str, nonce: Option<&str>, now: i64) -> Result<(String, Option<String>), String> {
    let payload = id_token.split('.').nth(1).ok_or("ID token inválido")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|_| "ID token inválido")?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|_| "ID token inválido")?;
    let audience_ok = match &claims["aud"] {
        Value::String(aud) => aud == client_id,
        Value::Array(list) => list.iter().any(|a| a.as_str() == Some(client_id)),
        _ => false,
    };
    if claims["iss"].as_str() != Some(CHATGPT_ISSUER) || !audience_ok || claims["exp"].as_i64().is_none_or(|exp| exp + 300 < now) {
        return Err("O login do ChatGPT não pôde ser confirmado".into());
    }
    if let Some(expected) = nonce {
        if claims["nonce"].as_str() != Some(expected) {
            return Err("O login do ChatGPT não pôde ser confirmado".into());
        }
    }
    let subject = claims["sub"].as_str().ok_or("ID token sem identificação")?.to_string();
    Ok((subject, claims["email"].as_str().map(String::from)))
}

fn token_error(body: &Value) -> String {
    body["error"].as_str().or_else(|| body["error"]["code"].as_str()).unwrap_or("").to_string()
}

/// "Entrar com ChatGPT": navegador, retorno local e troca do código por tokens.
pub async fn chatgpt_login(host_id: &str, previous: Option<&ChatGptSession>, cancel: &CancellationToken, now: i64) -> Result<ChatGptSession, String> {
    let callback = Callback::bind()?;
    let (challenge, verifier) = pkce()?;
    let (state, nonce) = (random(16)?, random(16)?);
    let (url, redirect) = chatgpt_authorize_url(callback.port, host_id, &challenge, &state, &nonce, previous)?;
    open_url(url.as_str())?;
    let query = callback.wait("/auth/callback", cancel).await?;
    if param(&query, "state") != Some(state.as_str()) {
        return Err("O retorno do login não confere; tente de novo".into());
    }
    if param(&query, "error") == Some("access_denied") {
        return Err("O acesso não foi autorizado no ChatGPT".into());
    }
    let code = param(&query, "code").ok_or("O ChatGPT não devolveu a autorização")?;
    let client_id = match (param(&query, "client_id"), previous) {
        (Some(issued), Some(session)) if issued != session.client_id => return Err("O retorno do login não confere; tente de novo".into()),
        (Some(issued), _) => issued.to_string(),
        (None, Some(session)) => session.client_id.clone(),
        (None, None) => return Err("O ChatGPT não liberou este app. O \"Entrar com ChatGPT\" ainda pode estar restrito a parceiros da OpenAI.".into()),
    };
    let response = client()?
        .post(CHATGPT_TOKEN)
        .form(&[("grant_type", "authorization_code"), ("client_id", client_id.as_str()), ("code", code), ("code_verifier", verifier.as_str()), ("redirect_uri", redirect.as_str()), ("resource", OPENAI_API)])
        .send().await.map_err(|_| "Não foi possível falar com a OpenAI")?;
    let ok = response.status().is_success();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !ok {
        return Err(format!("A OpenAI recusou o login ({})", token_error(&body)));
    }
    session_from_tokens(&body, client_id, redirect, Some(&nonce), None, now)
}

fn session_from_tokens(body: &Value, client_id: String, redirect_uri: String, nonce: Option<&str>, previous: Option<&ChatGptSession>, now: i64) -> Result<ChatGptSession, String> {
    let access_token = body["access_token"].as_str().ok_or("A OpenAI não devolveu o acesso")?.to_string();
    let refresh_token = body["refresh_token"].as_str().map(String::from).or_else(|| previous.map(|p| p.refresh_token.clone())).ok_or("A OpenAI não devolveu a renovação do acesso")?;
    let id_token = body["id_token"].as_str().map(String::from).or_else(|| previous.map(|p| p.id_token.clone())).ok_or("A OpenAI não devolveu a identificação")?;
    let (subject, email) = match (body["id_token"].as_str(), previous) {
        (Some(token), _) => id_claims(token, &client_id, nonce, now)?,
        (None, Some(p)) => (p.subject.clone(), p.email.clone()),
        (None, None) => return Err("A OpenAI não devolveu a identificação".into()),
    };
    let scopes: Vec<String> = body["scope"].as_str().map(|s| s.split_whitespace().map(String::from).collect()).or_else(|| previous.map(|p| p.scopes.clone())).unwrap_or_default();
    let session = ChatGptSession { client_id, redirect_uri, email, subject, id_token, access_token, refresh_token, expires_at: now + body["expires_in"].as_i64().unwrap_or(3600), scopes };
    if !session.plan_enabled() {
        return Err("O login funcionou, mas a sua conta não liberou o uso do plano do ChatGPT neste app".into());
    }
    Ok(session)
}

/// Renova o acesso (o token de renovação é rotativo). `Err(true)` = entrar de novo.
pub async fn chatgpt_refresh(session: &ChatGptSession, now: i64) -> Result<ChatGptSession, (bool, String)> {
    let response = client().map_err(|e| (false, e))?
        .post(CHATGPT_TOKEN)
        .form(&[("grant_type", "refresh_token"), ("client_id", session.client_id.as_str()), ("refresh_token", session.refresh_token.as_str()), ("resource", OPENAI_API)])
        .send().await.map_err(|_| (false, "Não foi possível falar com a OpenAI".to_string()))?;
    let ok = response.status().is_success();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !ok {
        let code = token_error(&body);
        let terminal = matches!(code.as_str(), "invalid_grant" | "invalid_refresh_token" | "token_expired" | "refresh_token_expired" | "refresh_token_invalidated" | "refresh_token_reused");
        return Err((terminal, if terminal { "O acesso ao ChatGPT expirou. Entre de novo em Configurações.".into() } else { format!("Não foi possível renovar o acesso ao ChatGPT ({code})") }));
    }
    session_from_tokens(&body, session.client_id.clone(), session.redirect_uri.clone(), None, Some(session), now).map_err(|e| (false, e))
}

/// Encerra o acesso na OpenAI (melhor esforço) antes de apagar as credenciais locais.
pub async fn chatgpt_revoke(session: &ChatGptSession) {
    let Ok(client) = client() else { return };
    let Ok(discovery) = client.get(format!("{CHATGPT_ISSUER}/.well-known/openid-configuration")).send().await else { return };
    let Ok(config) = discovery.json::<Value>().await else { return };
    if let Some(endpoint) = config["revocation_endpoint"].as_str() {
        let _ = client.post(endpoint).form(&[("token", session.refresh_token.as_str()), ("token_type_hint", "refresh_token"), ("client_id", session.client_id.as_str())]).send().await;
    }
}

/// Credencial para falar com a plataforma.
pub enum Secret {
    Key(String),
    ChatGpt(String),
}

/// Monta o pedido de resposta em streaming para a plataforma.
pub fn chat_request(api: Api, base: &str, secret: &Secret, model: &str, effort: Option<&str>, system: &str, message: &str) -> Result<reqwest::RequestBuilder, String> {
    let base = base.trim_end_matches('/');
    let client = client()?;
    Ok(match (api, secret) {
        (Api::Compatible, Secret::Key(key)) => client.post(format!("{base}/chat/completions")).bearer_auth(key).json(&compatible_body(model, effort, system, message)),
        (Api::Anthropic, Secret::Key(key)) => client.post(format!("{base}/messages")).header("x-api-key", key).header("anthropic-version", "2023-06-01")
            .json(&anthropic_body(model, effort, system, message)),
        (Api::ChatGpt, Secret::ChatGpt(token)) => client.post(format!("{OPENAI_API}/responses")).bearer_auth(token).json(&responses_body(model, effort, system, message)),
        _ => return Err("Credencial incompatível com a plataforma".into()),
    })
}

pub fn search_request(platform: &str, secret: &Secret, model: &str, effort: Option<&str>, system: &str, message: &str) -> Result<reqwest::RequestBuilder, String> {
    let key = match (platform, secret) {
        ("chatgpt", Secret::ChatGpt(token)) | ("openai", Secret::Key(token)) => token,
        _ => return Err("Para pesquisar na web, escolha uma conta do ChatGPT ou da OpenAI em Configurações › Nuvem.".into()),
    };
    let instructions = format!("{system}\n\nPesquise na web antes de responder. Se o pedido trouxer links, abra essas páginas primeiro e use o que elas mostram; diga com clareza quando uma página não abrir ou não mostrar o que foi pedido, sem inventar valores. Responda em português. Para pedidos abertos, traga de 3 a 4 opções distintas, ordenadas da melhor para a pior segundo o seu julgamento, e explique em uma frase por que cada uma funciona melhor ou pior. Respeite a quantidade pedida explicitamente. Cite junto de cada opção a página específica ou o vídeo encontrado usando a ferramenta de citações. Escreva os nomes em texto simples e deixe os links para as citações. Use apenas fontes pertinentes à plataforma solicitada. Se não encontrar o suficiente, seja claro sobre isso.");
    let mut body = responses_body(model, effort, &instructions, message);
    body["tools"] = json!([{"type":"web_search"}]);
    body["tool_choice"] = json!("required");
    Ok(client()?.post(format!("{OPENAI_API}/responses")).bearer_auth(key).json(&body))
}

/// Esforço só vai quando a pessoa escolhe; sem escolha, cada plataforma usa o padrão dela.
pub fn compatible_body(model: &str, effort: Option<&str>, system: &str, message: &str) -> Value {
    let mut body = json!({"model": model, "messages": [{"role": "system", "content": system}, {"role": "user", "content": message}], "stream": true});
    if let Some(effort) = effort {
        body["reasoning_effort"] = json!(effort);
    }
    body
}
/// Claude: esforço vira orçamento de raciocínio estendido; a resposta ganha espaço além dele.
pub fn anthropic_body(model: &str, effort: Option<&str>, system: &str, message: &str) -> Value {
    let budget = match effort {
        Some("low") => Some(2_048),
        Some("medium") => Some(8_000),
        Some("high") => Some(16_000),
        _ => None,
    };
    let mut body = json!({"model": model, "max_tokens": 16000, "system": system, "messages": [{"role": "user", "content": message}], "stream": true});
    if let Some(budget) = budget {
        body["max_tokens"] = json!(budget + 8_000);
        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
    }
    body
}
pub fn responses_body(model: &str, effort: Option<&str>, system: &str, message: &str) -> Value {
    let mut body = json!({"model": model, "instructions": system, "input": [{"role": "user", "content": message}], "store": false, "stream": true});
    if let Some(effort) = effort {
        body["reasoning"] = json!({"effort": effort});
    }
    body
}

/// Um evento do streaming já interpretado.
#[derive(Debug, PartialEq)]
pub enum Step {
    Text(String),
    Done,
    Fail(String),
    Skip,
}
/// `chat/completions` compatível com a OpenAI.
pub fn parse_compatible(event: &Value) -> Step {
    if let Some(message) = event["error"]["message"].as_str() {
        return Step::Fail(message.chars().take(200).collect());
    }
    if event["choices"][0]["finish_reason"].as_str() == Some("length") {
        return Step::Fail("A resposta atingiu o limite de tamanho da plataforma".into());
    }
    match event["choices"][0]["delta"]["content"].as_str() {
        Some(text) if !text.is_empty() => Step::Text(text.into()),
        _ => Step::Skip,
    }
}
/// Messages da Anthropic: só o texto; o raciocínio interno não aparece.
pub fn parse_anthropic(event: &Value) -> Step {
    match event["type"].as_str() {
        Some("content_block_delta") if event["delta"]["type"] == "text_delta" => Step::Text(event["delta"]["text"].as_str().unwrap_or("").into()),
        Some("message_delta") if event["delta"]["stop_reason"] == "max_tokens" => Step::Fail("A resposta atingiu o limite de tamanho da plataforma".into()),
        Some("message_stop") => Step::Done,
        Some("error") => Step::Fail(event["error"]["message"].as_str().unwrap_or("Erro da plataforma").chars().take(200).collect()),
        _ => Step::Skip,
    }
}
/// Responses API (plano do ChatGPT): sucesso só com `response.completed`.
pub fn parse_responses(event: &Value) -> Step {
    match event["type"].as_str() {
        Some("response.output_text.delta") => Step::Text(event["delta"].as_str().unwrap_or("").into()),
        Some("response.completed") => Step::Done,
        Some("response.failed") => Step::Fail(format!("{}{}", chatgpt_error(event["response"]["error"]["code"].as_str().unwrap_or("")), resets(&event["response"]["error"]))),
        Some("response.incomplete") => Step::Fail("A resposta do ChatGPT ficou incompleta".into()),
        Some("error") => Step::Fail(chatgpt_error(event["code"].as_str().or_else(|| event["error"]["code"].as_str()).unwrap_or(""))),
        _ => Step::Skip,
    }
}
/// Quando a cota volta, se o ChatGPT disser ("resets_in_seconds" ou "resets_at").
fn resets(error: &Value) -> String {
    let seconds = error["resets_in_seconds"].as_i64()
        .or_else(|| error["resets_at"].as_i64().map(|at| at - std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)));
    match seconds {
        Some(s) if s > 0 && s < 3600 => format!(" Volta em cerca de {} min.", (s + 59) / 60),
        Some(s) if s >= 3600 && s < 72 * 3600 => format!(" Volta em cerca de {} h.", (s + 1799) / 3600),
        Some(s) if s >= 72 * 3600 => format!(" Volta em cerca de {} dias.", (s + 43199) / 86400),
        _ => String::new(),
    }
}

/// Mensagens claras para os erros do plano do ChatGPT.
pub fn chatgpt_error(code: &str) -> String {
    match code {
        // O "Entrar com ChatGPT" usa a cota do plano para outros apps (a mesma do Codex), separada do
        // chat no site do ChatGPT, que continua funcionando.
        "subscription_sharing_usage_limit_exceeded" => "Acabou, por agora, a cota do seu plano do ChatGPT para outros apps — é a mesma do Codex, separada do chat no site (que continua funcionando). Ela volta quando o limite do Codex renovar (veja em Gerenciar uso). Enquanto isso, responda neste computador.".into(),
        "subscription_sharing_user_not_eligible" => "Sua conta do ChatGPT não pode usar o plano em outros apps.".into(),
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => "O plano do ChatGPT está indisponível agora. Tente de novo em instantes.".into(),
        "subscription_sharing_invalid_user" => "O acesso ao ChatGPT expirou. Entre de novo em Configurações.".into(),
        "" => "O ChatGPT não concluiu a resposta".into(),
        other => format!("O ChatGPT recusou o pedido ({other})"),
    }
}

/// Erro HTTP antes do streaming, em linguagem simples.
pub fn http_error(status: u16, body: &str) -> String {
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = parsed["error"]["code"].as_str().or_else(|| parsed["error"]["type"].as_str()).unwrap_or("");
    if code.starts_with("subscription_sharing") || code == "usage_limit_reached" {
        return format!("{}{}", chatgpt_error(if code == "usage_limit_reached" { "subscription_sharing_usage_limit_exceeded" } else { code }), resets(&parsed["error"]));
    }
    match status {
        401 => "A plataforma não aceitou a credencial. Confira a chave ou entre de novo.".into(),
        402 => "A conta na plataforma está sem créditos.".into(),
        403 => "A plataforma recusou o acesso para esta conta ou região.".into(),
        404 => "Modelo não encontrado nesta plataforma. Escolha outro em Configurações.".into(),
        429 => "Limite de uso da plataforma atingido. Tente mais tarde.".into(),
        500..=599 => "A plataforma está com problemas agora. Tente de novo em instantes.".into(),
        _ => format!("A plataforma recusou o pedido (HTTP {status})"),
    }
}

/// Lê o streaming SSE e repassa o texto; falha se terminar sem conclusão.
pub async fn stream(request: reqwest::RequestBuilder, api: Api, cancel: &CancellationToken, mut report: impl FnMut(Value)) -> Result<String, String> {
    read_stream(request, api, cancel, None, &mut report).await
}

pub async fn stream_search(request: reqwest::RequestBuilder, cancel: &CancellationToken, videos: bool, mut report: impl FnMut(Value)) -> Result<String, String> {
    read_stream(request, Api::ChatGpt, cancel, Some(videos), &mut report).await
}

async fn read_stream(request: reqwest::RequestBuilder, api: Api, cancel: &CancellationToken, search: Option<bool>, report: &mut impl FnMut(Value)) -> Result<String, String> {
    use futures_util::StreamExt;
    let response = tokio::select! {_ = cancel.cancelled() => return Err("Resposta interrompida".into()), r = request.send() => r.map_err(|_| "Não foi possível conectar à plataforma")?};
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        return Err(http_error(status, &body.chars().take(4000).collect::<String>()));
    }
    let mut stream = response.bytes_stream();
    let (mut buffer, mut full, mut done) = (Vec::new(), String::new(), false);
    // Alguns streams só entregam os itens em `output_item.done` e fecham com `output` vazio.
    let mut items = Vec::new();
    while !done {
        let item = tokio::select! {_ = cancel.cancelled() => return Err("Resposta interrompida".into()), item = stream.next() => item};
        let Some(item) = item else { break };
        buffer.extend(item.map_err(|_| "Resposta interrompida")?);
        if buffer.len() > 8 * 1024 * 1024 {
            return Err("Resposta excede o limite de processamento".into());
        }
        while let Some(index) = buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=index).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:") else { continue };
            let data = data.trim();
            if data == "[DONE]" {
                done = true;
                break;
            }
            let Ok(event) = serde_json::from_str::<Value>(data) else { continue };
            if let Some(videos) = search {
                match event["type"].as_str() {
                    Some("response.output_item.done") => { items.push(event["item"].clone()); continue; }
                    Some("response.completed") => {
                        let mut result = event["response"].clone();
                        if result["output"].as_array().is_none_or(|output| output.is_empty()) {
                            result["output"] = Value::Array(std::mem::take(&mut items));
                        }
                        let answer = super::research::answer(&result, videos)?;
                        report(json!({"type":"token","text":answer}));
                        return Ok(answer);
                    }
                    Some("error" | "response.failed" | "response.incomplete") => {}
                    _ => continue,
                }
            }
            let step = match api { Api::Compatible => parse_compatible(&event), Api::Anthropic => parse_anthropic(&event), Api::ChatGpt => parse_responses(&event) };
            match step {
                Step::Text(text) => {
                    full.push_str(&text);
                    report(json!({"type":"token","text":text}));
                    if full.len() > 4 * 1024 * 1024 {
                        return Err("Resposta excede o limite configurado".into());
                    }
                }
                Step::Done => { done = true; break; }
                Step::Fail(message) => return Err(message),
                Step::Skip => {}
            }
        }
    }
    // Compatíveis às vezes fecham sem [DONE]; Anthropic e ChatGPT exigem a conclusão.
    if !done && (api != Api::Compatible || full.is_empty()) {
        return Err("A conexão terminou sem concluir a resposta".into());
    }
    if full.trim().is_empty() {
        return Err("A plataforma não produziu uma resposta".into());
    }
    Ok(full)
}

/// Modelos disponíveis na conta (id, nome); `images` = só os que criam imagens.
pub async fn models(api: Api, base: &str, secret: &Secret, images: bool) -> Result<Vec<(String, String)>, String> {
    let base = base.trim_end_matches('/');
    let client = client()?;
    let openrouter = base.starts_with("https://openrouter.ai/");
    let request = match (api, secret) {
        (Api::Compatible, Secret::Key(key)) if images && openrouter => client.get(format!("{base}/models?output_modalities=image")).bearer_auth(key),
        (Api::Compatible, Secret::Key(key)) => client.get(format!("{base}/models")).bearer_auth(key),
        (Api::Anthropic, Secret::Key(key)) => client.get(format!("{base}/models?limit=100")).header("x-api-key", key).header("anthropic-version", "2023-06-01"),
        (Api::ChatGpt, Secret::ChatGpt(token)) => client.get(format!("{OPENAI_API}/models")).bearer_auth(token),
        _ => return Err("Credencial incompatível com a plataforma".into()),
    };
    let response = request.send().await.map_err(|_| "Não foi possível falar com a plataforma")?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        return Err(http_error(status, &response.text().await.unwrap_or_default()));
    }
    let body: Value = response.json().await.map_err(|_| "Lista de modelos inválida")?;
    let list = parse_models(api, &body);
    Ok(if images && !openrouter { list.into_iter().filter(|(id, _)| is_image_model(id)).collect() } else { list })
}
/// Modelos de imagem pelo nome (as listas compatíveis não dizem a modalidade).
pub fn is_image_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    ["image", "imagen", "dall-e", "flux", "stable-diffusion"].iter().any(|k| id.contains(k))
}

/// Cria uma imagem a partir do pedido revisado; devolve os bytes e a extensão.
pub async fn generate_image(provider: &Provider, key: &str, model: &str, prompt: &str) -> Result<(Vec<u8>, &'static str), String> {
    let api = provider.images.ok_or("Esta plataforma não cria imagens")?;
    let base = provider.base.trim_end_matches('/');
    let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(240)).redirect(reqwest::redirect::Policy::none()).build().map_err(|_| "Falha no cliente de rede")?;
    let request = match api {
        ImageApi::Generations => {
            let mut body = json!({"model": model, "prompt": prompt, "n": 1});
            // Os modelos gpt-image sempre devolvem base64 e recusam o parâmetro.
            if !(provider.id == "openai" && model.starts_with("gpt-image")) {
                body["response_format"] = json!("b64_json");
            }
            client.post(format!("{base}/images/generations")).bearer_auth(key).json(&body)
        }
        ImageApi::OpenRouterChat => client.post(format!("{base}/chat/completions")).bearer_auth(key)
            .json(&json!({"model": model, "messages": [{"role": "user", "content": prompt}], "modalities": ["image", "text"]})),
    };
    let response = request.send().await.map_err(|_| "Não foi possível falar com a plataforma")?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        return Err(http_error(status, &response.text().await.unwrap_or_default().chars().take(4000).collect::<String>()));
    }
    let body: Value = response.json().await.map_err(|_| "Resposta de imagem inválida")?;
    let bytes = match image_payload(api, &body)? {
        ImageOut::Bytes(bytes) => bytes,
        ImageOut::Url(url) => {
            let link = reqwest::Url::parse(&url).map_err(|_| "Endereço de imagem inválido")?;
            if link.scheme() != "https" {
                return Err("Endereço de imagem inválido".into());
            }
            let download = client.get(link).send().await.map_err(|_| "Não foi possível baixar a imagem")?;
            if !download.status().is_success() || download.content_length().unwrap_or(0) > 25 * 1024 * 1024 {
                return Err("Não foi possível baixar a imagem".into());
            }
            download.bytes().await.map_err(|_| "Não foi possível baixar a imagem")?.to_vec()
        }
    };
    let extension = image_extension(&bytes).ok_or("A plataforma não devolveu uma imagem válida")?;
    if bytes.len() > 25 * 1024 * 1024 {
        return Err("Imagem grande demais".into());
    }
    Ok((bytes, extension))
}
#[derive(Debug, PartialEq)]
pub enum ImageOut {
    Bytes(Vec<u8>),
    Url(String),
}
/// Onde cada API devolve a imagem: base64, `data:` URL ou link para baixar.
pub fn image_payload(api: ImageApi, body: &Value) -> Result<ImageOut, String> {
    let decode = |text: &str| base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|_| "Imagem inválida".to_string());
    let from_url = |url: &str| -> Result<ImageOut, String> {
        match url.strip_prefix("data:") {
            Some(rest) => Ok(ImageOut::Bytes(decode(rest.split_once(',').ok_or("Imagem inválida")?.1)?)),
            None => Ok(ImageOut::Url(url.to_string())),
        }
    };
    match api {
        ImageApi::Generations => {
            if let Some(data) = body["data"][0]["b64_json"].as_str() {
                return Ok(ImageOut::Bytes(decode(data)?));
            }
            body["data"][0]["url"].as_str().map(from_url).unwrap_or_else(|| Err("A plataforma não devolveu a imagem".into()))
        }
        ImageApi::OpenRouterChat => body["choices"][0]["message"]["images"][0]["image_url"]["url"].as_str()
            .map(from_url).unwrap_or_else(|| Err("Este modelo não devolveu imagem; escolha um modelo de imagem".into())),
    }
}
pub fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG") { Some("png") }
    else if bytes.starts_with(&[0xff, 0xd8, 0xff]) { Some("jpg") }
    else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") { Some("webp") }
    else { None }
}
/// A lista do plano ChatGPT já vem na ordem recomendada para a conta.
pub fn automatic_model<'a>(platform: &str, catalog: &'a [(String, String)]) -> Result<&'a str, String> {
    catalog.iter().map(|(id, _)| id.as_str()).find(|id| {
        let name = id.to_ascii_lowercase();
        !id.is_empty() && !is_image_model(id)
            && !["embed", "whisper", "tts", "audio", "transcri", "moderation", "realtime", "dall-e", "sora", "rerank", "deep-research", "search-api", "search-preview"].iter().any(|tag| name.contains(tag))
            && (platform != "openai" || name.starts_with("gpt-") && !name.contains("instruct"))
    }).ok_or_else(|| "A conta conectada não ofereceu um modelo de conversa disponível.".to_string())
}

pub fn parse_models(api: Api, body: &Value) -> Vec<(String, String)> {
    let empty = Vec::new();
    if api == Api::ChatGpt {
        return body["models"].as_array().unwrap_or(&empty).iter()
            .filter(|m| m["visibility"] == "list")
            .filter_map(|m| Some((m["slug"].as_str()?.to_string(), m["display_name"].as_str().or(m["slug"].as_str())?.to_string())))
            .collect();
    }
    body["data"].as_array().unwrap_or(&empty).iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?.trim_start_matches("models/").to_string();
            let name = m["display_name"].as_str().or(m["name"].as_str()).unwrap_or(&id).to_string();
            Some((id, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn esforco_so_vai_quando_escolhido_e_no_formato_de_cada_api() {
        assert!(compatible_body("m", None, "s", "q").get("reasoning_effort").is_none());
        assert_eq!(compatible_body("m", Some("high"), "s", "q")["reasoning_effort"], "high");
        assert_eq!(responses_body("m", Some("low"), "s", "q")["reasoning"]["effort"], "low");
        assert!(responses_body("m", None, "s", "q").get("reasoning").is_none());
        let claude = anthropic_body("m", Some("medium"), "s", "q");
        assert_eq!(claude["thinking"]["budget_tokens"], 8000);
        assert!(claude["max_tokens"].as_u64().unwrap() > 8000, "resposta cabe além do raciocínio");
        assert!(anthropic_body("m", None, "s", "q").get("thinking").is_none());
    }

    use super::*;
    #[test]
    fn pkce_follows_rfc_7636() {
        // Vetor de teste do apêndice B da RFC 7636.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let (c, v) = pkce().unwrap();
        assert_eq!((v.len(), c, v.contains('=')), (43, challenge(&v), false));
        assert_ne!(pkce().unwrap().1, v, "verificador novo a cada login");
    }
    #[test]
    fn catalog_links_and_vault_accounts() {
        let ids: Vec<_> = providers().iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), ids.iter().collect::<std::collections::HashSet<_>>().len());
        for p in providers() {
            assert!(p.base.starts_with("https://"));
            assert_eq!(p.login == Login::Key, p.keys_url.is_some() && p.id != "openrouter");
            if let Some(url) = p.keys_url { assert!(allowed_link(url) && url.starts_with("https://")); }
        }
        assert!(allowed_link(CHATGPT_USAGE) && !allowed_link("https://exemplo.com") && !allowed_link("file:///etc/passwd"));
        assert_eq!(vault_account("openai"), "provider:openai");
        assert_eq!(provider("anthropic").unwrap().api, Api::Anthropic);
    }
    #[test]
    fn chatgpt_authorization_url_registers_once_and_reuses_the_client() {
        let (url, redirect) = chatgpt_authorize_url(1455, "urn:uuid:host", "desafio", "estado", "nonce", None).unwrap();
        assert_eq!(redirect, "http://127.0.0.1:1455/auth/callback");
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.as_str().split('?').next(), Some(CHATGPT_AUTH));
        assert_eq!(q["client_id"], "dynamic_agent_client");
        assert_eq!(q["agent_name_hint"], "Ayla");
        assert_eq!(q["scope"], "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct");
        assert_eq!((q["resource"].as_str(), q["code_challenge_method"].as_str(), q["redirect_uri"].as_str()), (OPENAI_API, "S256", redirect.as_str()));
        let session = ChatGptSession { client_id: "oaiapp_1".into(), redirect_uri: redirect.clone(), email: Some("a@b.c".into()), subject: "s".into(), id_token: "idt".into(), access_token: "a".into(), refresh_token: "r".into(), expires_at: 100, scopes: vec![PLAN_SCOPE.into()] };
        let (again, _) = chatgpt_authorize_url(5000, "urn:uuid:host", "d", "e", "n", Some(&session)).unwrap();
        let q: std::collections::HashMap<_, _> = again.query_pairs().into_owned().collect();
        assert_eq!((q["client_id"].as_str(), q["id_token_hint"].as_str(), q["login_hint"].as_str()), ("oaiapp_1", "idt", "a@b.c"));
        assert!(!q.contains_key("agent_name_hint"));
        assert!(session.needs_refresh(41) && !session.needs_refresh(39) && session.plan_enabled());
    }
    fn token(claims: Value) -> String {
        let enc = |v: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
        format!("{}.{}.assinatura", enc(&json!({"alg":"RS256"})), enc(&claims))
    }
    #[test]
    fn id_token_claims_are_checked() {
        let good = token(json!({"iss": CHATGPT_ISSUER, "aud": "oaiapp_1", "exp": 2000, "nonce": "n", "sub": "user-1", "email": "a@b.c"}));
        assert_eq!(id_claims(&good, "oaiapp_1", Some("n"), 1000).unwrap(), ("user-1".into(), Some("a@b.c".into())));
        assert!(id_claims(&good, "outro", Some("n"), 1000).is_err(), "público errado");
        assert!(id_claims(&good, "oaiapp_1", Some("x"), 1000).is_err(), "nonce errado");
        assert!(id_claims(&good, "oaiapp_1", Some("n"), 9000).is_err(), "expirado");
        let issuer = token(json!({"iss": "https://falso", "aud": "oaiapp_1", "exp": 2000, "sub": "u"}));
        assert!(id_claims(&issuer, "oaiapp_1", None, 1000).is_err());
        let body = json!({"access_token": "a", "refresh_token": "r", "id_token": good, "expires_in": 3600, "scope": "openid offline_access"});
        assert!(session_from_tokens(&body, "oaiapp_1".into(), "x".into(), Some("n"), None, 1000).unwrap_err().contains("não liberou o uso do plano"));
    }
    #[test]
    fn streams_are_parsed_per_platform() {
        assert_eq!(parse_compatible(&json!({"choices":[{"delta":{"content":"Olá"}}]})), Step::Text("Olá".into()));
        assert_eq!(parse_compatible(&json!({"choices":[{"delta":{},"finish_reason":"stop"}]})), Step::Skip);
        assert!(matches!(parse_compatible(&json!({"choices":[{"delta":{},"finish_reason":"length"}]})), Step::Fail(_)));
        assert_eq!(parse_anthropic(&json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"Oi"}})), Step::Text("Oi".into()));
        assert_eq!(parse_anthropic(&json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"…"}})), Step::Skip);
        assert_eq!(parse_anthropic(&json!({"type":"message_stop"})), Step::Done);
        assert_eq!(parse_anthropic(&json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}})), Step::Fail("Overloaded".into()));
        assert_eq!(parse_responses(&json!({"type":"response.output_text.delta","delta":"Tudo"})), Step::Text("Tudo".into()));
        assert_eq!(parse_responses(&json!({"type":"response.completed"})), Step::Done);
        assert!(matches!(parse_responses(&json!({"type":"response.failed","response":{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}})), Step::Fail(m) if m.contains("cota do seu plano")));
        assert!(matches!(parse_responses(&json!({"type":"response.incomplete"})), Step::Fail(_)));
        assert!(http_error(401, "").contains("credencial"));
        assert!(http_error(429, r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}"#).contains("plano do ChatGPT"));
        let limite = http_error(429, r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","resets_in_seconds":7000}}"#);
        assert!(limite.contains("mesma do Codex") && limite.contains("Volta em cerca de 2 h"), "{limite}");
        assert!(http_error(429, r#"{"error":{"type":"usage_limit_reached","resets_in_seconds":600}}"#).contains("10 min"));
    }
    #[test]
    fn model_lists_per_platform() {
        let chatgpt = json!({"models":[{"slug":"gpt-a","display_name":"GPT A","visibility":"list"},{"slug":"oculto","visibility":"hide"}]});
        assert_eq!(parse_models(Api::ChatGpt, &chatgpt), vec![("gpt-a".into(), "GPT A".into())]);
        let compatible = json!({"data":[{"id":"models/gemini-x"},{"id":"claude-sonnet-5-5","display_name":"Claude Sonnet 5.5"},{"id":"r/m","name":"Roteado"}]});
        assert_eq!(parse_models(Api::Compatible, &compatible), vec![("gemini-x".into(), "gemini-x".into()), ("claude-sonnet-5-5".into(), "Claude Sonnet 5.5".into()), ("r/m".into(), "Roteado".into())]);
    }
    #[test]
    fn images_come_from_base64_data_urls_or_links() {
        let png = [0x89u8, b'P', b'N', b'G', 1, 2];
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        assert_eq!(image_payload(ImageApi::Generations, &json!({"data":[{"b64_json": b64}]})).unwrap(), ImageOut::Bytes(png.to_vec()));
        assert_eq!(image_payload(ImageApi::Generations, &json!({"data":[{"url":"https://cdn.exemplo/x.png"}]})).unwrap(), ImageOut::Url("https://cdn.exemplo/x.png".into()));
        let routed = json!({"choices":[{"message":{"images":[{"image_url":{"url": format!("data:image/png;base64,{b64}")}}]}}]});
        assert_eq!(image_payload(ImageApi::OpenRouterChat, &routed).unwrap(), ImageOut::Bytes(png.to_vec()));
        assert!(image_payload(ImageApi::OpenRouterChat, &json!({"choices":[{"message":{"content":"só texto"}}]})).is_err());
        assert_eq!((image_extension(&png), image_extension(&[0xff, 0xd8, 0xff, 0]), image_extension(b"GIF89a")), (Some("png"), Some("jpg"), None));
        assert!(is_image_model("gpt-image-1") && is_image_model("imagen-4.0-generate") && !is_image_model("gpt-5"));
        let creators: Vec<_> = providers().iter().filter(|p| p.images.is_some()).map(|p| p.id).collect();
        assert_eq!(creators, vec!["openrouter", "openai", "gemini", "xai"]);
    }
    #[tokio::test]
    async fn local_callback_returns_the_query_and_ignores_other_paths() {
        let callback = Callback::bind().unwrap();
        let port = callback.port;
        let token = CancellationToken::new();
        let browser = std::thread::spawn(move || {
            let send = |path: &str| {
                let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                s.write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes()).unwrap();
                let mut out = String::new();
                let _ = s.read_to_string(&mut out);
                out
            };
            let missing = send("/favicon.ico");
            let ok = send("/auth/callback?code=abc&state=xyz");
            (missing, ok)
        });
        let query = callback.wait("/auth/callback", &token).await.unwrap();
        let (missing, ok) = browser.join().unwrap();
        assert!(missing.starts_with("HTTP/1.1 404") && ok.contains("Pode voltar para a Ayla"));
        assert_eq!((param(&query, "code"), param(&query, "state")), (Some("abc"), Some("xyz")));
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(Callback::bind().unwrap().wait("/x", &cancelled).await.is_err());
    }
}
