//! Comandos do Hand para o sistema: programas (grade, avaliação, desinstalar), atalhos do
//! Hyprland e macros de teclado e mouse.
use super::*;
use crate::core::hand_atalhos::{self as atalhos, Atalho, Combo, Fonte, Mudancas, Novo};
use crate::core::hand_macros::{self as macros, Macro};
use crate::core::hand_programas::{self as programas, App, Avaliacao, Origem};
use std::sync::atomic::{AtomicBool, Ordering};

fn dados(state: &Core) -> PathBuf { state.root.join("hand") }

/// Atalhos, macros e rotinas da pessoa: na configuração, que viaja com os dotfiles. Na primeira
/// vez, traz os arquivos antigos dos dados da Ayla.
pub(crate) fn config(local: &std::path::Path) -> PathBuf {
    use crate::core::ayla_config;
    let dir = ayla_config::preparar().unwrap_or_else(|_| ayla_config::pasta());
    for nome in ["atalhos.json", "macros.json"] { ayla_config::migrar(&local.join(nome), &dir.join(nome)); }
    dir
}

// ---------------------------------------------------------------------------------------------
// Programas

static AVALIANDO: AtomicBool = AtomicBool::new(false);

fn cache_path(state: &Core) -> PathBuf { dados(state).join("programas.json") }

/// Programas do menu com ícone, origem e a avaliação guardada (se já houver).
#[tauri::command]
pub async fn hand_programas_grade(state: tauri::State<'_, Core>) -> Result<Value, String> {
    let apps = tauri::async_runtime::spawn_blocking(programas::listar).await.map_err(|_| "Falha ao listar os programas")?;
    let cache = programas::cache_load(&cache_path(&state));
    let list: Vec<Value> = apps.iter().map(|a| { let mut v = json!(a); v["avaliacao"] = json!(cache.get(&a.chave())); v }).collect();
    Ok(json!({"programas": list, "avaliando": AVALIANDO.load(Ordering::Relaxed)}))
}

/// Avalia os programas novos ou atualizados (e os que só tinham a descrição do pacote, quando
/// há modelo local agora). Manda o progresso; se já está avaliando, espera aquela terminar.
#[tauri::command]
pub async fn hand_programas_avaliar(state: tauri::State<'_, Core>, events: Channel<Value>) -> Result<Value, String> {
    if AVALIANDO.swap(true, Ordering::SeqCst) {
        while AVALIANDO.load(Ordering::SeqCst) { tokio::time::sleep(std::time::Duration::from_millis(500)).await; }
        return Ok(json!({"avaliados": 0}));
    }
    let result = avaliar(&state, &events).await;
    AVALIANDO.store(false, Ordering::SeqCst);
    result
}

async fn avaliar(state: &Core, events: &Channel<Value>) -> Result<Value, String> {
    let path = cache_path(state);
    let apps = tauri::async_runtime::spawn_blocking(programas::listar).await.map_err(|_| "Falha ao listar os programas")?;
    let mut cache = programas::cache_load(&path);
    // Avaliações de programas que saíram ou mudaram de versão não servem mais.
    let atuais: std::collections::HashSet<String> = apps.iter().map(App::chave).collect();
    let antes = cache.len();
    cache.retain(|k, _| atuais.contains(k));
    if cache.len() != antes { programas::cache_save(&path, &cache)?; }
    let prefs = state.prefs()?;
    let model = models::choose(&state.models()?, Capability::Text, state.budget(&prefs), prefs.context_tokens, None).ok();
    let mut seen = std::collections::HashSet::new();
    let pendentes: Vec<App> = apps.into_iter().filter(|a| seen.insert(a.chave()))
        .filter(|a| cache.get(&a.chave()).is_none_or(|c| !c.modelo && model.is_some() && a.origem != Origem::Manual)).collect();
    let total = pendentes.len();
    if total == 0 { return Ok(json!({"avaliados": 0})); }
    let _ = events.send(json!({"feitos": 0, "total": total}));
    let pacotes: Vec<String> = pendentes.iter().filter(|a| a.origem == Origem::Sistema).map(|a| a.pacote.clone()).collect();
    let descricoes = tauri::async_runtime::spawn_blocking(move || programas::descricoes(&pacotes)).await.unwrap_or_default();
    let protegidos = programas::protegidos();
    let mut feitos = 0;
    for lote in pendentes.chunks(6) {
        // Simulação do dnf e texto básico (sem modelo).
        let lote_owned = lote.to_vec();
        let simulacoes = tauri::async_runtime::spawn_blocking(move || lote_owned.iter().map(|a| (a.origem == Origem::Sistema).then(|| programas::simular(&a.pacote).ok()).flatten()).collect::<Vec<_>>())
            .await.unwrap_or_default();
        let mut base: Vec<(String, Avaliacao, String, String)> = lote.iter().enumerate().map(|(i, a)| {
            let (resumo, descricao) = descricoes.get(&a.pacote).cloned().unwrap_or_default();
            (a.chave(), programas::avaliacao_basica(a, &resumo, simulacoes.get(i).and_then(Option::as_ref), &protegidos), resumo, descricao)
        }).collect();
        if let Some(model) = &model {
            let itens: Vec<(String, &App, String, String, &Avaliacao)> = lote.iter().filter(|a| a.origem != Origem::Manual)
                .map(|a| { let b = base.iter().find(|b| b.0 == a.chave()).expect("base"); (b.0.clone(), a, b.2.clone(), b.3.clone(), &b.1) }).collect();
            if !itens.is_empty() {
                let token = CancellationToken::new();
                if let Ok(raw) = state.runtime.complete_json_sized(model, &prefs, programas::avaliacao_pedido(&itens), programas::avaliacao_schema(), 900, 0.2, &token).await {
                    let textos = programas::ler_avaliacao(&raw);
                    for (chave, avaliacao, _, _) in base.iter_mut() {
                        if let Some((resumo, desinstalar)) = textos.get(chave) {
                            avaliacao.resumo = resumo.clone();
                            // O que é essencial continua dito pelas regras, não pelo modelo.
                            if !avaliacao.essencial && !desinstalar.is_empty() { avaliacao.desinstalar = desinstalar.clone(); }
                            avaliacao.modelo = true;
                        }
                    }
                }
            }
        }
        for (chave, avaliacao, _, _) in base { cache.insert(chave, avaliacao); }
        programas::cache_save(&path, &cache)?;
        feitos += lote.len();
        let _ = events.send(json!({"feitos": feitos, "total": total}));
    }
    Ok(json!({"avaliados": feitos}))
}

/// Ícone de um programa (svg/png de uma pasta de ícones).
#[tauri::command]
pub fn hand_icone(caminho: String) -> Result<tauri::ipc::Response, String> {
    let path = PathBuf::from(&caminho);
    if !programas::icone_valido(&path) { return Err("Ícone inválido".into()); }
    let meta = std::fs::metadata(&path).map_err(|_| "Ícone não encontrado")?;
    if meta.len() > 2 << 20 { return Err("Ícone grande demais".into()); }
    Ok(tauri::ipc::Response::new(std::fs::read(&path).map_err(|_| "Ícone não encontrado")?))
}

fn app_por_id(id: &str) -> Result<App, String> {
    programas::listar().into_iter().find(|a| a.id == id).ok_or_else(|| "Programa não encontrado".into())
}

/// O que aconteceria ao desinstalar, agora (simulação nova do dnf), e o comando que rodaria.
#[tauri::command]
pub async fn hand_programa_previa(id: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let app = app_por_id(&id)?;
        let protegidos = programas::protegidos();
        let sim = (app.origem == Origem::Sistema).then(|| programas::simular(&app.pacote)).transpose()?;
        let avaliacao = programas::avaliacao_basica(&app, "", sim.as_ref(), &protegidos);
        let comando = programas::comando_desinstalar(&app).map(|(p, a)| format!("{p} {}", a.join(" ")));
        Ok(json!({"origem": app.origem, "pacote": app.pacote, "sairiam": sim.as_ref().map(|s| s.sairiam.clone()).unwrap_or_default(),
            "libera": sim.as_ref().map(|s| s.libera.clone()).unwrap_or_default(), "essencial": avaliacao.essencial, "explicacao": avaliacao.desinstalar,
            "comando": if avaliacao.essencial { None } else { comando }}))
    }).await.map_err(|_| "A simulação parou")?
}

/// Desinstala (depois da confirmação na interface). Os pacotes do sistema pedem a senha pelo
/// diálogo do sistema; os essenciais nunca.
#[tauri::command]
pub async fn hand_programa_desinstalar(id: String) -> Result<Value, String> {
    let app = tauri::async_runtime::spawn_blocking({ let id = id.clone(); move || app_por_id(&id) }).await.map_err(|_| "Falha ao achar o programa")??;
    if app.origem == Origem::Sistema {
        let protegidos = programas::protegidos();
        let pacote = app.pacote.clone();
        let sim = tauri::async_runtime::spawn_blocking(move || programas::simular(&pacote)).await.map_err(|_| "A simulação parou")??;
        if programas::avaliacao_basica(&app, "", Some(&sim), &protegidos).essencial { return Err("Esse programa faz parte do sistema: a Ayla não desinstala.".into()); }
    }
    let (programa, args) = programas::comando_desinstalar(&app).ok_or("Esse programa não veio de um pacote: não dá para desinstalar por aqui.")?;
    let out = tokio::process::Command::new(&programa).args(&args).env("LC_ALL", "C").stdin(std::process::Stdio::null()).output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(900), out).await.map_err(|_| "A desinstalação passou de 15 minutos")?
        .map_err(|_| format!("Não foi possível rodar o {programa}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        // 126/127: a senha foi recusada ou o diálogo fechado.
        if programa == "pkexec" && matches!(out.status.code(), Some(126 | 127)) { return Err("A senha não foi confirmada; nada mudou.".into()); }
        return Err(format!("Não deu certo: {}", text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("erro desconhecido").trim()));
    }
    Ok(json!({"ok": true, "nome": app.name}))
}

// ---------------------------------------------------------------------------------------------
// Atalhos

fn mudancas_path(state: &Core) -> PathBuf { config(&dados(state)).join("atalhos.json") }

/// Todos os atalhos (dotfiles, seus e da Ayla), o binário da Ayla (para macros) e se é Hyprland.
#[tauri::command]
pub fn hand_atalhos(state: tauri::State<'_, Core>) -> Result<Value, String> {
    let hypr = atalhos::pasta_hypr();
    let lista = atalhos::lista(&hypr, &atalhos::carregar(&mudancas_path(&state)));
    Ok(json!({"atalhos": lista, "hyprland": hypr.join("hyprland.conf").is_file(), "arquivo": atalhos::arquivo_ayla(&hypr)}))
}

fn salvar_e_aplicar(state: &Core, mudancas: &Mudancas) -> Result<(), String> {
    let hypr = atalhos::pasta_hypr();
    let anterior = atalhos::carregar(&mudancas_path(state));
    atalhos::aplicar(&hypr, mudancas, &state.root.join("backups"), atalhos::recarregar_hyprland())?;
    if let Err(e) = atalhos::guardar(&mudancas_path(state), mudancas) {
        let _ = atalhos::aplicar(&hypr, &anterior, &state.root.join("backups"), atalhos::recarregar_hyprland());
        return Err(e);
    }
    Ok(())
}

fn achar(lista: &[Atalho], id: &str) -> Result<Atalho, String> {
    lista.iter().find(|a| a.id == id).cloned().ok_or_else(|| "Atalho não encontrado (a configuração mudou?)".into())
}

/// Pedido da interface para criar, trocar a tecla de um atalho ou desativar.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OpAtalho {
    Criar { mods: String, tecla: String, descricao: String, acao: String, arg: String, #[serde(default)] substituir: bool },
    Trocar { id: String, mods: String, tecla: String, #[serde(default)] substituir: bool },
    Desativar { id: String },
    Restaurar { id: String },
}

/// Aplica uma mudança. Se a combinação já é usada por outro atalho ativo e `substituir` não veio,
/// devolve o conflito sem mudar nada.
#[tauri::command]
pub fn hand_atalho_mudar(state: tauri::State<'_, Core>, op: OpAtalho) -> Result<Value, String> {
    let hypr = atalhos::pasta_hypr();
    let mut mudancas = atalhos::carregar(&mudancas_path(&state));
    let lista = atalhos::lista(&hypr, &mudancas);
    let novo_id = || format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let checar = |combo: &Combo, ignorar: Option<&str>, substituir: bool, mudancas: &mut Mudancas| -> Result<Option<Value>, String> {
        if let Some(outro) = atalhos::conflito(&lista, combo, ignorar) {
            if !substituir { return Ok(Some(json!({"conflito": outro}))); }
            let outro = outro.clone();
            atalhos::desativar(mudancas, &outro);
        }
        Ok(None)
    };
    match op {
        OpAtalho::Criar { mods, tecla, descricao, acao, arg, substituir } => {
            let novo = Novo { id: novo_id(), combo: Combo::novo(&mods, &tecla), flags: String::new(), descricao, acao, arg, substitui: None };
            atalhos::validar(&novo)?;
            if let Some(c) = checar(&novo.combo, None, substituir, &mut mudancas)? { return Ok(c); }
            mudancas.novos.push(novo);
        }
        OpAtalho::Trocar { id, mods, tecla, substituir } => {
            let atalho = achar(&lista, &id)?;
            let combo = Combo::novo(&mods, &tecla);
            if combo.igual(&atalho.combo) { return Ok(json!({"ok": true})); }
            if let Some(c) = checar(&combo, Some(&id), substituir, &mut mudancas)? { return Ok(c); }
            if atalho.fonte == Fonte::Ayla {
                let alvo = mudancas.novos.iter_mut().find(|n| format!("ayla:{}", n.id) == id).ok_or("Atalho não encontrado")?;
                alvo.combo = combo;
                atalhos::validar(alvo)?;
            } else {
                let novo = Novo { id: novo_id(), combo, flags: atalho.flags.clone(), descricao: atalho.descricao.clone(), acao: atalho.acao.clone(), arg: atalho.arg.clone(), substitui: Some(atalho.combo.clone()) };
                atalhos::validar(&novo)?;
                atalhos::desativar(&mut mudancas, &atalho);
                mudancas.novos.push(novo);
            }
        }
        OpAtalho::Desativar { id } => atalhos::desativar(&mut mudancas, &achar(&lista, &id)?),
        OpAtalho::Restaurar { id } => atalhos::restaurar(&mut mudancas, &achar(&lista, &id)?.combo),
    }
    salvar_e_aplicar(&state, &mudancas)?;
    Ok(json!({"ok": true}))
}

// ---------------------------------------------------------------------------------------------
// Macros

#[tauri::command]
pub fn hand_macros(state: tauri::State<'_, Core>) -> Result<Value, String> {
    Ok(json!({"macros": macros::carregar(&config(&dados(&state))), "gravando": macros::gravando(), "falta": macros::falta_para_gravar(),
        "binario": std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "ayla".into())}))
}

#[tauri::command]
pub fn hand_macro_salvar(state: tauri::State<'_, Core>, item: Macro) -> Result<(), String> {
    macros::validar(&item)?;
    let dir = dados(&state);
    let mut list = macros::carregar(&config(&dir));
    match list.iter_mut().find(|m| m.id == item.id) { Some(m) => *m = item, None => list.push(item) }
    macros::guardar(&config(&dir), &list)
}

/// Apaga a macro e os atalhos que a rodavam.
#[tauri::command]
pub fn hand_macro_apagar(state: tauri::State<'_, Core>, id: String) -> Result<(), String> {
    let dir = dados(&state);
    let mut list = macros::carregar(&config(&dir));
    list.retain(|m| m.id != id);
    let mut mudancas = atalhos::carregar(&mudancas_path(&state));
    let antes = mudancas.novos.len();
    mudancas.novos.retain(|n| !n.arg.ends_with(&format!("--macro {id}")));
    if mudancas.novos.len() != antes { salvar_e_aplicar(&state, &mudancas)?; }
    macros::guardar(&config(&dir), &list)
}

/// Roda uma macro guardada (o atalho de parar e o botão Parar interrompem).
pub async fn rodar_macro(dir: PathBuf, id: String) -> Result<(), String> {
    let item = macros::carregar(&config(&dir)).into_iter().find(|m| m.id == id).ok_or("Macro não encontrada")?;
    let (numero, token) = crate::core::hand_agent::comecar();
    let result = tauri::async_runtime::spawn_blocking(move || macros::rodar(&dir, &item.passos, &token)).await.map_err(|_| "A macro parou");
    crate::core::hand_agent::terminar(numero);
    result?
}

#[tauri::command]
pub async fn hand_macro_rodar(state: tauri::State<'_, Core>, id: String, espera: Option<u64>) -> Result<(), String> {
    if let Some(s) = espera { tokio::time::sleep(std::time::Duration::from_secs(s.min(10))).await; }
    rodar_macro(dados(&state), id).await
}

#[tauri::command]
pub fn hand_gravar_comecar() -> Result<(), String> { macros::comecar_gravacao() }

#[tauri::command]
pub fn hand_gravar_parar() -> Result<Vec<macros::PassoMacro>, String> { macros::parar_gravacao() }

// ---------------------------------------------------------------------------------------------
// Dotfiles

use crate::core::hand_dotfiles as dotfiles;

fn repo_dotfiles() -> Result<PathBuf, String> {
    dotfiles::achar().ok_or_else(|| "Não achei um repositório git de dotfiles (procurei onde está a configuração do Hyprland, ~/dotfiles e ~/.dotfiles).".into())
}

fn explicacoes_path(state: &Core) -> PathBuf { dados(state).join("dotfiles.json") }

fn explicacoes(state: &Core) -> std::collections::HashMap<String, String> {
    std::fs::read(explicacoes_path(state)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Ramo, remoto e o que vai (sem acessar a rede), com a explicação guardada de cada arquivo e o
/// aviso dos que parecem ter senha ou chave.
#[tauri::command]
pub async fn hand_dotfiles_estado(state: tauri::State<'_, Core>) -> Result<Value, String> {
    let guardadas = explicacoes(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let dir = repo_dotfiles()?;
        let estado = dotfiles::estado(&dir)?;
        let mut v = json!(estado);
        v["mudancas"] = json!(estado.mudancas.iter().map(|m| json!({"caminho": m.caminho, "estado": m.estado,
            "explicacao": guardadas.get(&m.caminho), "sensivel": dotfiles::sensivel(&dir, &m.caminho, &m.estado)})).collect::<Vec<_>>());
        Ok(v)
    }).await.map_err(|_| "Falha ao ler o repositório")?
}

static EXPLICANDO: AtomicBool = AtomicBool::new(false);

/// A Ayla explica (modelo local) os arquivos da lista que ainda não têm explicação e guarda.
#[tauri::command]
pub async fn hand_dotfiles_explicar(state: tauri::State<'_, Core>) -> Result<usize, String> {
    if EXPLICANDO.swap(true, Ordering::SeqCst) { return Ok(0); }
    let result = explicar_dotfiles(&state).await;
    EXPLICANDO.store(false, Ordering::SeqCst);
    result
}

async fn explicar_dotfiles(state: &Core) -> Result<usize, String> {
    let mut guardadas = explicacoes(state);
    let pendentes = tauri::async_runtime::spawn_blocking({
        let guardadas = guardadas.clone();
        move || -> Result<Vec<(String, String, String)>, String> {
            let dir = repo_dotfiles()?;
            Ok(dotfiles::estado(&dir)?.mudancas.into_iter().filter(|m| !guardadas.contains_key(&m.caminho))
                .map(|m| { let amostra = dotfiles::amostra(&dir, &m.caminho, &m.estado); (m.caminho, m.estado, amostra) }).collect())
        }
    }).await.map_err(|_| "Falha ao ler o repositório")??;
    if pendentes.is_empty() { return Ok(0); }
    let prefs = state.prefs()?;
    let model = models::choose(&state.models()?, Capability::Text, state.budget(&prefs), prefs.context_tokens, None).map_err(|_| "Sem modelo local para explicar os arquivos")?;
    let mut feitos = 0;
    for lote in pendentes.chunks(6) {
        let token = CancellationToken::new();
        let Ok(raw) = state.runtime.complete_json_sized(&model, &prefs, dotfiles::explicacao_pedido(lote), dotfiles::explicacao_schema(), 700, 0.2, &token).await else { continue };
        for (arquivo, texto) in dotfiles::ler_explicacoes(&raw) {
            if lote.iter().any(|(c, _, _)| *c == arquivo) { guardadas.insert(arquivo, texto); feitos += 1; }
        }
        let path = explicacoes_path(state);
        if let Some(d) = path.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(&path, serde_json::to_vec_pretty(&guardadas).unwrap_or_default());
    }
    Ok(feitos)
}

/// Texto de um arquivo da lista (e a diferença, se mudou).
#[tauri::command]
pub async fn hand_dotfiles_conteudo(caminho: String) -> Result<dotfiles::Conteudo, String> {
    tauri::async_runtime::spawn_blocking(move || dotfiles::conteudo(&repo_dotfiles()?, &caminho)).await.map_err(|_| "Falha ao ler o repositório")?
}

/// Commit de tudo o que a lista mostrou, traz o que mudou no remoto e envia (depois da confirmação).
#[tauri::command]
pub async fn hand_dotfiles_sincronizar(arquivos: Vec<String>, mensagem: String) -> Result<dotfiles::Resultado, String> {
    tauri::async_runtime::spawn_blocking(move || dotfiles::sincronizar(&repo_dotfiles()?, &arquivos, &mensagem)).await.map_err(|_| "A sincronização parou")?
}
