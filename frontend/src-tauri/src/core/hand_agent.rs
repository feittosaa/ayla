//! Hand · tarefas: a Ayla opera o computador olhando a tela. A pessoa aprova o plano uma vez;
//! depois ela segue sozinha (captura do monitor em foco → modelo de visão local → uma ação),
//! parando para perguntar antes de algo irreversível. Parar: atalho global (Super+Esc enquanto
//! opera) ou o botão Parar. Tudo neste computador.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Passos no máximo por tarefa (cada um é uma ação na tela).
pub const MAX_PASSOS: usize = 30;

// ---------------------------------------------------------------------------------------------
// Parar

/// Tarefa em andamento (uma por vez), com um número para saber se ainda é a mesma ao terminar.
static ATUAL: Mutex<Option<(u64, CancellationToken)>> = Mutex::new(None);
static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Começa uma tarefa (cancela a anterior, se houver).
pub fn comecar() -> (u64, CancellationToken) {
    let id = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let token = CancellationToken::new();
    if let Ok(mut atual) = ATUAL.lock() {
        if let Some((_, antiga)) = atual.replace((id, token.clone())) { antiga.cancel(); }
    }
    (id, token)
}

/// A tarefa `id` terminou (ou pausou): o atalho não tem mais o que parar.
pub fn terminar(id: u64) {
    if let Ok(mut atual) = ATUAL.lock() {
        if atual.as_ref().is_some_and(|(atual_id, _)| *atual_id == id) { *atual = None; }
    }
}

/// Para a tarefa em andamento (atalho global, botão Parar ou notificação).
pub fn parar() -> bool {
    ATUAL.lock().ok().and_then(|mut a| a.take()).map(|(_, t)| t.cancel()).is_some()
}

/// Atalho global enquanto opera. Usa Super+Esc; se a pessoa já tem esse atalho, Super+Shift+Esc.
/// Devolve o atalho registrado (para mostrar e para tirar depois).
pub fn registrar_atalho(binario: &Path) -> Option<(String, String)> {
    let binds = Command::new("hyprctl").args(["binds", "-j"]).output().ok()
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok()).unwrap_or(json!([]));
    let taken = |mods: u64| binds.as_array().into_iter().flatten().any(|b| b["key"].as_str() == Some("Escape") && b["modmask"].as_u64() == Some(mods));
    let (mods, label) = if !taken(64) { ("SUPER", "Super+Esc") } else if !taken(65) { ("SUPER SHIFT", "Super+Shift+Esc") } else { return None };
    let bind = format!("{mods},Escape,exec,{} --hand-parar", binario.display());
    let ok = Command::new("hyprctl").args(["keyword", "bind", &bind]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "ok");
    ok.then(|| (mods.to_string(), label.to_string()))
}

pub fn tirar_atalho(mods: &str) {
    let _ = Command::new("hyprctl").args(["keyword", "unbind", &format!("{mods},Escape")]).output();
}

// ---------------------------------------------------------------------------------------------
// Olhar a tela

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor { pub name: String, pub x: i64, pub y: i64, pub width: f64, pub height: f64 }

/// Monitor em foco, em coordenadas lógicas (as do cursor no Hyprland).
pub fn monitor_em_foco() -> Result<Monitor, String> {
    let out = Command::new("hyprctl").args(["monitors", "-j"]).output().map_err(|_| "Para olhar a tela, a Ayla precisa do Hyprland")?;
    let list: Value = serde_json::from_slice(&out.stdout).map_err(|_| "Resposta inválida do Hyprland")?;
    let m = list.as_array().into_iter().flatten().find(|m| m["focused"] == true).ok_or("Nenhum monitor em foco")?;
    monitor_de(m).ok_or_else(|| "Monitor inválido".to_string())
}

pub fn monitor_de(m: &Value) -> Option<Monitor> {
    let scale = m["scale"].as_f64().filter(|s| *s > 0.0)?;
    let (mut w, mut h) = (m["width"].as_f64()?, m["height"].as_f64()?);
    if m["transform"].as_i64().unwrap_or(0) % 2 == 1 { std::mem::swap(&mut w, &mut h); }
    Some(Monitor { name: m["name"].as_str()?.to_string(), x: m["x"].as_i64()?, y: m["y"].as_i64()?, width: w / scale, height: h / scale })
}

/// Captura do monitor (no máximo 1280 px de largura, JPEG) para o modelo olhar.
pub fn olhar(monitor: &Monitor, destino: &Path) -> Result<(), String> {
    let factor = (1280.0 / monitor.width).min(1.0);
    let ok = Command::new("grim").args(["-o", &monitor.name, "-s", &format!("{factor:.3}"), "-t", "jpeg", "-q", "80"]).arg(destino)
        .status().map_err(|_| "Para olhar a tela, instale o grim")?.success();
    if ok && destino.is_file() { Ok(()) } else { Err("Não foi possível capturar a tela".into()) }
}

/// Ponto que o modelo indicou (0–1000 na imagem) → coordenadas da área de trabalho.
pub fn para_tela(monitor: &Monitor, x: f64, y: f64) -> (i64, i64) {
    let (x, y) = (x.clamp(0.0, 1000.0) / 1000.0, y.clamp(0.0, 1000.0) / 1000.0);
    (monitor.x + (x * monitor.width).round() as i64, monitor.y + (y * monitor.height).round() as i64)
}

// ---------------------------------------------------------------------------------------------
// Plano e próxima ação

/// `contexto`: o que a conversa acabou de mover, achar ou capturar ("eles", "um deles").
pub fn plano_pedido(objetivo: &str, janelas: &str, pastas: &str, contexto: &str) -> Vec<Value> {
    vec![
        json!({"role":"system","content":"Você planeja tarefas no computador (Linux, Hyprland, bash) da pessoa. Responda em português do Brasil, só com o JSON. \
Primeiro veja se dá para fazer SEM a tela: mover, copiar ou renomear arquivos, criar pastas, trocar o papel de parede, abrir um site ou um arquivo. \
Se der, escreva em `comando` um único comando bash curto que faz tudo (caminhos completos entre aspas, `mv -n` e `cp -n` para nunca sobrescrever, sem sudo, sem apagar nada, sem `rm`) e deixe `passos` vazio; a pessoa confere o comando antes de rodar. \
Só use `comando` se ele fizer a tarefa INTEIRA; se depois for preciso mexer no programa (tocar, escrever, procurar, entrar), não é comando: são passos. \
Se não der, deixe `comando` vazio e escreva em `passos` só ações na tela, curtas e concretas, na ordem, que a Ayla vai fazer com o teclado e o mouse da pessoa. \
Use o MENOR número de passos que resolve: a maioria das tarefas cabe em 2 a 5; nunca encha a lista até o limite. Sem passos de conferir, verificar, fechar ou sair, a não ser que a tarefa peça. \
Para abrir um programa, o passo é só \"Abrir o <programa>\" com o nome que a pessoa usou (a Ayla abre pelo menu de aplicativos, sem clicar em ícones). \
Exemplo: \"abre a calculadora e calcula 12 vezes 7\" → passos [\"Abrir a Calculadora\", \"Digitar 12*7 e apertar Enter\"]. \
As janelas abertas servem só para saber o que já está aberto: só use uma janela se a tarefa precisar dela; nunca traga para a frente, clique ou feche janelas que a tarefa não pede. \
Nunca planeje digitar senha nem fazer login: suponha que a pessoa já está com a conta aberta; se não estiver, a Ayla pergunta na hora. Dúvidas para a pessoa (qual conta, qual pasta, qual arquivo) vão só em `precisa`, numa frase; nunca nos passos. Não invente caminhos nem o que não sabe: se a pessoa não disse de onde vêm os arquivos ou quais são, deixe `comando` e `passos` vazios e pergunte em `precisa`; nunca use todos os arquivos de uma pasta (`pasta/*`) no lugar dos que ela pediu. \
O que a pessoa salvou ou baixou pela Ayla fica guardado na Iris, dentro da Ayla, e não em Downloads. Se ela diz que baixou ou salvou algo sem dizer onde, deixe `comando` e `passos` vazios e pergunte em `precisa` se foi pela Ayla (Iris) ou pelo navegador (Downloads)."}),
        json!({"role":"user","content":format!("Tarefa: {objetivo}\nPastas da pessoa: {pastas}\nJanelas já abertas (só use se a tarefa precisar): {janelas}{}", if contexto.is_empty() { String::new() } else { format!("\nNesta conversa (\"eles\", \"esses\", \"um deles\" falam disto): {contexto}") })}),
    ]
}

pub fn plano_schema() -> Value {
    json!({"type":"object","properties":{
        "comando":{"type":"string"},
        "passos":{"type":"array","items":{"type":"string"},"maxItems":10},
        "precisa":{"type":"string"}
    },"required":["comando","passos","precisa"]})
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Plano {
    pub passos: Vec<String>,
    #[serde(default)] pub precisa: String,
    /// Feito sem a tela: um comando que a pessoa confere antes de rodar.
    #[serde(default)] pub comando: String,
}

pub fn ler_plano(raw: &str) -> Option<Plano> {
    let mut plano: Plano = serde_json::from_str(raw).ok()?;
    // Perguntas que o modelo pôs nos passos ("precisa: qual conta?") vão para `precisa`.
    let (perguntas, passos): (Vec<String>, Vec<String>) = plano.passos.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty())
        .partition(|p| super::hand::fold(p).starts_with("precisa"));
    plano.passos = passos;
    for pergunta in perguntas {
        let texto = pergunta.split_once(':').map_or(pergunta.as_str(), |(_, t)| t).trim().to_string();
        if !plano.precisa.contains(&texto) { plano.precisa = if plano.precisa.trim().is_empty() { texto } else { format!("{} {texto}", plano.precisa.trim()) }; }
    }
    plano.passos.truncate(10);
    plano.comando = plano.comando.trim().to_string();
    // "mv -n ~/Downloads/* …" leva a pasta inteira no lugar do que foi pedido: pergunta em vez disso.
    if comando_amplo(&plano.comando) {
        plano.comando.clear();
        if plano.precisa.trim().is_empty() { plano.precisa = "Quais arquivos são (nome, tipo ou data) e de onde eles vêm?".into(); }
    }
    plano.precisa = plano.precisa.trim().to_string();
    (!plano.passos.is_empty() || !plano.comando.is_empty() || !plano.precisa.is_empty()).then_some(plano)
}

/// Primeiro programa do comando que não existe neste computador (o modelo às vezes inventa um).
/// Olha o início de cada parte (`a && b | c; d`); comandos internos do bash passam.
pub fn programa_inexistente(comando: &str, existe: impl Fn(&str) -> bool) -> Option<String> {
    const INTERNOS: [&str; 22] = ["cd", "echo", "export", "test", "[", "true", "false", "set", "source", ".", "read", "printf", "for", "do", "done", "if", "then", "else", "fi", "while", "exit", "pwd"];
    comando.split(|c: char| c == ';' || c == '|' || c == '&' || c == '\n').filter_map(|parte| {
        parte.split_whitespace().find(|t| !(t.contains('=') && !t.starts_with('=')))
    }).map(|t| t.trim_matches(|c| c == '"' || c == '\'' || c == '(' || c == ')'))
        .filter(|t| !t.is_empty() && !t.starts_with('$') && !t.starts_with('`') && !INTERNOS.contains(t))
        .find(|t| !existe(t)).map(str::to_string)
}

/// Está no PATH (ou é um caminho executável)?
pub fn no_path(programa: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let ok = |p: &std::path::Path| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if programa.contains('/') { return ok(std::path::Path::new(programa)); }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| ok(&dir.join(programa))))
}

/// O comando pega tudo de uma pasta (`pasta/*`, `*` sozinho, `pasta/.*`)? Filtros como `*.pdf` passam.
pub fn comando_amplo(comando: &str) -> bool {
    comando.split(|c: char| c.is_whitespace() || c == ';' || c == '&' || c == '|').map(|t| t.trim_matches(|c| c == '"' || c == '\'')).any(|t| t == "*" || t == ".*" || t.ends_with("/*") || t.ends_with("/.*"))
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Tipo { Clicar, CliqueDuplo, CliqueDireito, Digitar, Teclas, Rolar, AbrirPrograma, Esperar, Concluido, Perguntar }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Acao {
    pub acao: Tipo,
    #[serde(default)] pub x: f64,
    #[serde(default)] pub y: f64,
    #[serde(default)] pub texto: String,
    /// Por que (aparece no progresso: "Clicar em Entrar").
    #[serde(default)] pub motivo: String,
    /// O modelo acha que não dá para desfazer (enviar, comprar, apagar…).
    #[serde(default)] pub irreversivel: bool,
}

pub fn acao_pedido(objetivo: &str, plano: &[String], feitos: &[String], largura: u32, altura: u32) -> String {
    format!(
        "Tarefa: {objetivo}\nPlano aprovado:\n{}\nJá feito (do mais antigo ao mais recente):\n{}\n\n\
A imagem é a tela agora ({largura}×{altura}). Escolha UMA próxima ação. Coordenadas x e y de 0 a 1000 na imagem \
(0,0 no canto superior esquerdo; 1000,1000 no inferior direito), no centro do que vai clicar.\n\
Ações: clicar, clique_duplo, clique_direito (com x,y); digitar (texto); teclas (texto = atalho, ex.: \"ctrl+l\", \"enter\"); \
rolar (texto = \"cima\" ou \"baixo\"); abrir_programa (texto = nome); esperar (a tela ainda está carregando); \
concluido (a tarefa terminou; motivo = resumo); perguntar (precisa da pessoa; motivo = a pergunta).\n\
Marque irreversivel=true antes de enviar, publicar, comprar, pagar, apagar, confirmar ou digitar senha. \
Nunca digite senhas: use perguntar. motivo: frase curta em português do que a ação faz.",
        plano.iter().enumerate().map(|(i, p)| format!("{}. {p}", i + 1)).collect::<Vec<_>>().join("\n"),
        if feitos.is_empty() { "(nada ainda)".to_string() } else { feitos.iter().rev().take(12).rev().cloned().collect::<Vec<_>>().join("\n") },
    )
}

pub fn acao_schema() -> Value {
    json!({"type":"object","properties":{
        "acao":{"type":"string","enum":["clicar","clique_duplo","clique_direito","digitar","teclas","rolar","abrir_programa","esperar","concluido","perguntar"]},
        "x":{"type":"number"},"y":{"type":"number"},"texto":{"type":"string"},"motivo":{"type":"string"},"irreversivel":{"type":"boolean"}
    },"required":["acao","x","y","texto","motivo","irreversivel"]})
}

pub fn ler_acao(raw: &str) -> Option<Acao> {
    let acao: Acao = serde_json::from_str(raw).ok()?;
    (acao.x.is_finite() && acao.y.is_finite()).then_some(acao)
}

/// Palavras que fazem a Ayla parar e perguntar, mesmo que o modelo não tenha marcado.
const SENSIVEIS: &[&str] = &["enviar", "envie", "send", "publicar", "postar", "post", "comprar", "compra", "pagar", "pagamento", "pay", "checkout",
    "apagar", "excluir", "deletar", "delete", "remover", "esvaziar", "confirmar", "confirm", "finalizar", "transferir", "transferência", "pix",
    "senha", "password", "assinar", "desinstalar", "formatar", "sair da conta", "logout"];

/// Precisa da aprovação da pessoa antes de fazer? (irreversível ou com cara de ser).
pub fn precisa_aprovar(acao: &Acao) -> bool {
    if acao.irreversivel || acao.acao == Tipo::Perguntar { return true; }
    let texto = super::hand::fold(&format!("{} {}", acao.motivo, acao.texto));
    SENSIVEIS.iter().any(|w| { let w = super::hand::fold(w); texto.split(|c: char| !c.is_alphanumeric()).any(|t| t == w) || (w.contains(' ') && texto.contains(&w)) })
}

/// Texto do progresso ("Clicar: Entrar").
pub fn descrever(acao: &Acao) -> String {
    let what = match acao.acao {
        Tipo::Clicar => "Clicar", Tipo::CliqueDuplo => "Clique duplo", Tipo::CliqueDireito => "Clique direito", Tipo::Digitar => "Digitar",
        Tipo::Teclas => "Teclas", Tipo::Rolar => "Rolar", Tipo::AbrirPrograma => "Abrir programa", Tipo::Esperar => "Esperar",
        Tipo::Concluido => "Concluído", Tipo::Perguntar => "Pergunta",
    };
    let detail = if !acao.motivo.trim().is_empty() { acao.motivo.trim().to_string() } else { acao.texto.trim().to_string() };
    if detail.is_empty() { what.to_string() } else { format!("{what}: {detail}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordenadas_do_modelo_viram_pontos_do_monitor() {
        let m = monitor_de(&json!({"name":"DP-1","x":1920,"y":0,"width":3840,"height":2160,"scale":2.0,"transform":0,"focused":true})).unwrap();
        assert_eq!((m.width, m.height), (1920.0, 1080.0));
        assert_eq!(para_tela(&m, 0.0, 0.0), (1920, 0));
        assert_eq!(para_tela(&m, 500.0, 500.0), (1920 + 960, 540));
        assert_eq!(para_tela(&m, 1200.0, -5.0), (1920 + 1920, 0), "fora da imagem fica na borda");
        let vertical = monitor_de(&json!({"name":"HDMI","x":0,"y":0,"width":1920,"height":1080,"scale":1.0,"transform":1})).unwrap();
        assert_eq!((vertical.width, vertical.height), (1080.0, 1920.0));
    }

    #[test]
    fn acoes_sensiveis_param_para_perguntar() {
        let a = |acao: Tipo, motivo: &str, texto: &str, irreversivel: bool| Acao { acao, x: 0.0, y: 0.0, texto: texto.into(), motivo: motivo.into(), irreversivel };
        assert!(!precisa_aprovar(&a(Tipo::Clicar, "Abrir a caixa de entrada", "", false)));
        assert!(precisa_aprovar(&a(Tipo::Clicar, "Clicar em Enviar", "", false)));
        assert!(precisa_aprovar(&a(Tipo::Digitar, "Digitar a senha", "hunter2", false)));
        assert!(precisa_aprovar(&a(Tipo::Clicar, "Finalizar compra", "", false)));
        assert!(precisa_aprovar(&a(Tipo::Teclas, "", "enter", true)), "o modelo marcou");
        assert!(precisa_aprovar(&a(Tipo::Perguntar, "Qual conta?", "", false)));
    }

    #[test]
    fn plano_e_acao_do_modelo_sao_lidos_com_limites() {
        let comando = ler_plano(r#"{"comando":" mv -n \"/home/x/a.jpg\" \"/home/x/Pictures/\" ","passos":[],"precisa":""}"#).unwrap();
        assert_eq!((comando.comando.as_str(), comando.passos.len()), ("mv -n \"/home/x/a.jpg\" \"/home/x/Pictures/\"", 0), "sem a tela: só o comando");
        let plano = ler_plano(r#"{"passos":["Abrir o Firefox"," ","Ir ao Gmail"],"precisa":""}"#).unwrap();
        assert_eq!(plano.passos, vec!["Abrir o Firefox", "Ir ao Gmail"]);
        assert!(ler_plano(r#"{"passos":[],"precisa":""}"#).is_none());
        // Pasta inteira no lugar do que foi pedido: vira pergunta, nunca comando.
        let amplo = ler_plano(r#"{"comando":"mv -n /home/pessoa/Downloads/* /home/pessoa/Pictures/Wallpapers/","passos":[],"precisa":""}"#).unwrap();
        assert!(amplo.comando.is_empty() && amplo.passos.is_empty() && !amplo.precisa.is_empty());
        assert!(comando_amplo("cp -n \"/home/x/Downloads\"/* /tmp") && comando_amplo("cd ~/Downloads && mv * ../x") && comando_amplo("mv ~/a/.* ~/b"));
        let tem = |p: &str| ["mv", "ls", "head", "pactl", "mkdir", "xdg-open"].contains(&p);
        assert_eq!(programa_inexistente("hybwp /home/x/Wallpapers/$(ls -t /home/x/Wallpapers | head -n 1)", tem).as_deref(), Some("hybwp"));
        assert_eq!(programa_inexistente("mkdir -p \"/home/x/a\" && xdg-open \"/home/x/a\"", tem), None);
        assert_eq!(programa_inexistente("cd /home/x && LANG=C pactl set-sink-volume @DEFAULT_SINK@ 30%", tem), None);
        assert_eq!(programa_inexistente("mv -n a b; feh --bg-fill a", tem).as_deref(), Some("feh"));
        assert!(no_path("sh") && !no_path("programa-que-nao-existe-ayla"));
        assert!(!comando_amplo("cp -n /home/x/Downloads/*.pdf /home/x/Documents/") && !comando_amplo("mkdir -p \"/home/x/Documents/Faculdade\""));
        let misturado = ler_plano(r#"{"passos":["precisa: qual é o serviço de e-mail?","Abrir o Firefox"],"precisa":""}"#).unwrap();
        assert_eq!((misturado.passos, misturado.precisa.as_str()), (vec!["Abrir o Firefox".to_string()], "qual é o serviço de e-mail?"));
        let acao = ler_acao(r#"{"acao":"clique_duplo","x":120,"y":40,"texto":"","motivo":"Abrir a pasta","irreversivel":false}"#).unwrap();
        assert_eq!((acao.acao.clone(), descrever(&acao)), (Tipo::CliqueDuplo, "Clique duplo: Abrir a pasta".to_string()));
        assert!(ler_acao(r#"{"acao":"voar","x":1,"y":1,"texto":"","motivo":"","irreversivel":false}"#).is_none());
    }

    #[test]
    fn parar_cancela_a_tarefa_em_andamento() {
        let (id, token) = comecar();
        assert!(!token.is_cancelled());
        assert!(parar());
        assert!(token.is_cancelled());
        assert!(!parar(), "nada em andamento");
        let (_, segunda) = comecar();
        terminar(id);
        assert!(parar() && segunda.is_cancelled(), "terminar a antiga não solta a nova");
    }
}
