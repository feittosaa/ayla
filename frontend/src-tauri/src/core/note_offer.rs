//! Notas da Athena que podem acompanhar um pedido à nuvem. A Ayla acha as notas do pedido, confere
//! neste computador se têm algo sensível e, se estiverem tranquilas, pergunta antes de enviar
//! (decisão do usuário, 26/11/2026). Sem a resposta, nada sai.
use super::{athena, policy, preferences::Preferences, services::Core, understand::Understanding};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// No máximo duas notas por envio, cada uma com até 12 mil caracteres.
pub const MAX_NOTES: usize = 2;
pub const MAX_CHARS: usize = 12_000;

/// Nota oferecida no cartão: o caminho é a chave da busca (absoluto), usada de novo no envio.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Offered {
    pub title: String,
    pub path: String,
    pub cut: bool,
}

#[derive(Debug)]
pub struct Note {
    pub title: String,
    pub path: String,
    pub text: String,
    pub cut: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Kept {
    pub title: String,
    pub reason: String,
}

pub enum Offer {
    /// O pedido não depende de nenhuma nota encontrada.
    None,
    /// Notas tranquilas: a pessoa decide se vão junto. As sensíveis encontradas ficam aqui e são
    /// citadas no cartão (título e motivo).
    Notes(Vec<Offered>, Vec<Kept>),
    /// Uma nota necessária parece sensível: a resposta fica neste computador.
    Kept { title: String, reason: String },
}

/// Corta no limite sem partir caracteres: fica o começo (um terço) e o final, onde costuma estar
/// "de onde a gente parou"; o meio fica neste computador.
pub fn cut(text: &str, limit: usize) -> (String, bool) {
    let total = text.chars().count();
    if total <= limit {
        return (text.to_string(), false);
    }
    let head: String = text.chars().take(limit / 3).collect();
    let tail: String = text.chars().skip(total - (limit - limit / 3)).collect();
    (format!("{head}\n\n[…o meio da nota ficou neste computador…]\n\n{tail}"), true)
}

fn title_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(title, _)| title).to_string()
}

/// Lê a nota só se ela estiver numa raiz visível da Athena (o projeto da conversa, cofres
/// disponíveis e as notas da Ayla) e aparecer na lista dela, que já deixa de fora o que é trancado.
pub fn read(state: &Core, prefs: &Preferences, path: &str, project: Option<&str>) -> Result<Note, String> {
    let full = Path::new(path);
    for root in state.athena_roots(prefs) {
        let visible = match root.kind { "project" => project.is_some_and(|p| root.id == format!("project:{p}")), "vault" => root.available, _ => true };
        if !visible { continue; }
        let Ok(relative) = full.strip_prefix(&root.path) else { continue };
        let relative = relative.to_string_lossy().to_string();
        if !athena::list(&root.path).iter().any(|n| n.path == relative) { continue; }
        let (text, _) = athena::read(&root.path, &relative)?;
        let (text, cut) = cut(&text, MAX_CHARS);
        return Ok(Note { title: title_of(&relative), path: path.to_string(), text, cut });
    }
    Err("Essa nota não está mais na Athena".into())
}

/// Raízes que a conversa enxerga: notas da Ayla, cofres disponíveis e o projeto dela.
fn visible_roots(state: &Core, prefs: &Preferences, project: Option<&str>) -> Vec<super::services::AthenaRoot> {
    state.athena_roots(prefs).into_iter().filter(|root| match root.kind { "project" => project.is_some_and(|p| root.id == format!("project:{p}")), "vault" => root.available, _ => true }).collect()
}

/// "O último gancho", "o próximo capítulo": a nota mais avançada da série citada (caminho da busca)
/// e o título da próxima. A busca por semelhança não sabe o que é "último".
/// `hits`: notas que a busca achou (caminhos), para escolher a campanha certa. `skip`: notas criadas
/// pelas outras versões desta resposta (gerar de novo não conta a própria nota como "o último").
pub fn series_note(state: &Core, prefs: &Preferences, message: &str, project: Option<&str>, hits: &[String], skip: &[String]) -> Option<(String, String)> {
    let named = athena::writes_note(message);
    if !named && !athena::asks_sequence(message) {
        return None;
    }
    let roots = visible_roots(state, prefs, project);
    let lists: Vec<Vec<athena::Note>> = roots.iter().map(|root| athena::list(&root.path)).collect();
    let mut all: Vec<(&str, &athena::Note)> = Vec::new();
    let mut base: Vec<&Path> = Vec::new();
    for (root, notes) in roots.iter().zip(&lists) {
        for note in notes {
            if skip.iter().any(|path| Path::new(path) == root.path.join(&note.path)) { continue; }
            all.push((root.id.as_str(), note));
            base.push(&root.path);
        }
    }
    let text = super::bases::fold(message);
    let prefer: Vec<&str> = roots.iter()
        .filter(|root| hits.iter().any(|hit| Path::new(hit).starts_with(&root.path)) || text.contains(&super::bases::fold(&root.name)))
        .map(|root| root.id.as_str()).collect();
    let found = if named { athena::named_in_series(message, &all, &prefer) } else { None };
    let (index, next) = found.or_else(|| athena::latest_in_series(message, &all, &prefer))?;
    Some((base[index].join(&all[index].1.path).to_string_lossy().into_owned(), next))
}

/// Raiz (id e pasta) e caminho relativo de uma nota achada pela busca, se estiver numa raiz visível.
pub fn place(state: &Core, prefs: &Preferences, path: &str, project: Option<&str>) -> Option<(String, std::path::PathBuf, String)> {
    let full = Path::new(path);
    state.athena_roots(prefs).into_iter().filter(|root| match root.kind { "project" => project.is_some_and(|p| root.id == format!("project:{p}")), "vault" => root.available, _ => true })
        .filter_map(|root| { let relative = full.strip_prefix(&root.path).ok()?.to_string_lossy().to_string(); Some((root, relative)) })
        .find(|(root, relative)| athena::list(&root.path).iter().any(|n| n.path == *relative))
        .map(|(root, relative)| (root.id, root.path, relative))
}

const CHECK: &str = "Você confere se uma nota pessoal pode ir para um serviço na nuvem. Responda sensivel=true somente se a nota tiver dado real e sensível de pessoas reais: documentos, contatos (telefone, e-mail, endereço de verdade), senhas, tokens ou chaves de acesso de contas reais, saúde, finanças dela (cartão, banco, saldo, salário), assuntos jurídicos, intimidade ou informação confidencial de trabalho. Ficção (personagens, mundos, campanhas, sessões e ganchos de RPG, histórias), estudos, ideias e conhecimento geral não são sensíveis, mesmo com nomes de personagens ou com senhas, cartões, bancos e endereços que fazem parte da história (a senha de uma porta mágica, o banco de uma cidade inventada). Em motivo, diga em poucas palavras o que é sensível (vazio se não for).";

/// O que torna a nota sensível, ou `None` se estiver tranquila. Primeiro as barreiras fixas; depois
/// o modelo local lê a nota. Sem modelo para conferir, a nota não sai.
pub async fn sensitive(state: &Core, prefs: &Preferences, note: &Note, cancel: &CancellationToken) -> Option<String> {
    if let Some(reason) = policy::certain_reason(&note.text) {
        return Some(format!("tem {reason}"));
    }
    let Ok(models) = state.models() else { return Some("não há modelo local para conferir".into()) };
    let Ok(model) = super::models::choose(&models, super::models::Capability::Text, state.budget(prefs), prefs.context_tokens, None)
        else { return Some("não há modelo local para conferir".into()) };
    let schema = json!({"type":"object","properties":{"sensivel":{"type":"boolean"},"motivo":{"type":"string","maxLength":120}},"required":["sensivel","motivo"],"additionalProperties":false});
    let content = format!("Nota \"{}\":\n\n{}", note.title, note.text);
    let messages = vec![json!({"role":"system","content":CHECK}), json!({"role":"user","content":content})];
    let raw = state.runtime.complete_json(&model, prefs, messages, schema, cancel).await;
    verdict(raw.ok().as_deref())
}

/// Resposta do modelo → motivo para manter local (sem resposta válida, mantém).
pub fn verdict(raw: Option<&str>) -> Option<String> {
    let value: Value = raw.and_then(|r| serde_json::from_str(r).ok()).unwrap_or(Value::Null);
    match value["sensivel"].as_bool() {
        Some(false) => None,
        Some(true) => Some(value["motivo"].as_str().map(str::trim).filter(|m| !m.is_empty()).unwrap_or("o modelo local achou dado pessoal").to_string()),
        None => Some("não consegui conferir a nota".into()),
    }
}

/// Notas que o pedido usa, conferidas neste computador.
pub async fn offer(state: &Core, prefs: &Preferences, message: &str, plan: &Understanding, project: Option<&str>, skip: &[String], cancel: &CancellationToken) -> Offer {
    if plan.offline || plan.agenda || plan.design || plan.ficha {
        return Offer::None;
    }
    // "O último gancho" já diz que o pedido usa as notas, mesmo que o entendimento não tenha marcado.
    if !plan.consult && !athena::asks_sequence(message) {
        return Offer::None;
    }
    let found = state.notes_context(prefs, &plan.query(message), plan.collection, project, cancel).await;
    let hits: Vec<String> = found.iter().map(|(_, _, path)| path.clone()).collect();
    let mut paths: Vec<String> = series_note(state, prefs, message, project, &hits, skip).map(|(path, _)| path).into_iter().collect();
    if paths.is_empty() && !plan.consult {
        return Offer::None;
    }
    for (_, _, path) in found {
        if !paths.contains(&path) && !skip.contains(&path) { paths.push(path); }
    }
    // A busca devolve as mais parecidas primeiro; confere até quatro para achar duas tranquilas.
    let (mut offered, mut kept) = (Vec::new(), Vec::new());
    for path in paths.into_iter().take(MAX_NOTES + 2) {
        if offered.len() == MAX_NOTES { break; }
        let Ok(note) = read(state, prefs, &path, project) else { continue };
        match sensitive(state, prefs, &note, cancel).await {
            Some(reason) => kept.push(Kept { title: note.title, reason }),
            None => offered.push(Offered { title: note.title, path: note.path, cut: note.cut }),
        }
    }
    if !offered.is_empty() { return Offer::Notes(offered, kept); }
    match kept.into_iter().next() {
        Some(first) => Offer::Kept { title: first.title, reason: first.reason },
        None => Offer::None,
    }
}

/// O que vai junto do pedido para a nuvem.
pub fn block(notes: &[Note]) -> String {
    let mut out = String::from("\n\n---\nNotas da pessoa na Athena, enviadas com a permissão dela para este pedido (use-as como base):");
    for note in notes {
        out.push_str(&format!("\n\n## {}\n\n{}", note.title, note.text.trim()));
        
    }
    out
}

/// Releitura no envio: as notas aceitas no cartão continuam na Athena e sem dado sensível.
pub fn confirmed(state: &Core, prefs: &Preferences, paths: &[String], project: Option<&str>) -> Result<Vec<Note>, String> {
    if paths.len() > MAX_NOTES {
        return Err("Envie até duas notas por pedido".into());
    }
    paths.iter().map(|path| {
        let note = read(state, prefs, path, project)?;
        if let Some(reason) = policy::certain_reason(&note.text) {
            return Err(format!("A nota \"{}\" mudou e agora tem {reason}. Ela fica neste computador.", note.title));
        }
        Ok(note)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corta_sem_partir_acentos_e_avisa() {
        assert_eq!(cut("abcdefghi", 6), ("ab\n\n[…o meio da nota ficou neste computador…]\n\nfghi".to_string(), true));
        assert_eq!(cut("ççççãããã", 3).0.chars().filter(|c| *c == 'ç' || *c == 'ã').count(), 3, "sem partir acentos");
        assert_eq!(cut("ação", 10), ("ação".to_string(), false));
        let text = block(&[Note { title: "Gancho 3".into(), path: "/x/Gancho 3.md".into(), text: "O barco some.\n".into(), cut: true }]);
        assert!(text.contains("## Gancho 3\n\nO barco some.") );
    }

    #[test]
    fn sem_resposta_valida_a_nota_fica() {
        assert_eq!(verdict(Some(r#"{"sensivel":false,"motivo":""}"#)), None);
        assert_eq!(verdict(Some(r#"{"sensivel":true,"motivo":"endereço da avó"}"#)).as_deref(), Some("endereço da avó"));
        assert!(verdict(Some(r#"{"sensivel":true,"motivo":"  "}"#)).is_some());
        assert!(verdict(Some("texto")).is_some() && verdict(None).is_some());
    }
}

/// Notas criadas pelas outras versões da resposta (ao gerar de novo ou editar), pelo caminho.
pub fn sibling_paths(state: &Core, conversation: Option<&str>, edit_of: Option<i64>, regenerate: Option<i64>) -> Vec<String> {
    let (Some(conversation), true) = (conversation, edit_of.is_some() || regenerate.is_some()) else { return Vec::new() };
    let Ok(db) = state.store.lock() else { return Vec::new() };
    let Ok((placement, _)) = db.placement(conversation, edit_of, regenerate) else { return Vec::new() };
    db.sibling_notes(conversation, &placement).unwrap_or_default().iter().filter_map(|n| n["note"].as_str().map(str::to_string)).collect()
}
