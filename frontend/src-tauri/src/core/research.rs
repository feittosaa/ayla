//! O Automático classifica o texto no computador antes de acessar a conta na nuvem.
use super::{athena, iris, organs, policy, reminders};
use serde_json::{json, Value};

fn words(message: &str) -> String {
    let mut text = String::from(" ");
    for c in message.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'ã' | 'â' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_alphanumeric() { text.push(c); }
        else if !text.ends_with(' ') { text.push(' '); }
    }
    text.push(' ');
    text
}

/// Sempre local: dados sensíveis, órgãos, notas, lembretes, arquivos e pedidos para não usar a internet.
pub fn private_or_local(message: &str) -> bool {
    let text = words(message);
    let any = |terms: &[&str]| terms.iter().any(|term| text.contains(term));
    policy::suspect_content(message) || organs::finance_question(message)
        || athena::note_request(message) || reminders::is_request(message)
        || iris::comparison_question(message)
        || any(&[" que criei ", " eu criei ", " que escrevi ", " que conversamos ", " nas notas ",
            " no cofre ", " na pasta ", " no projeto ", " athena ", " iris ", " obsidian ",
            " anexo ", " anexado ", " nesta conversa ", " sem pesquisar ", " sem internet ",
            " nao pesquise ", " nao procure ", " nao busque ", " nao pesquisa ", " nao procura ",
            " sem usar a internet ", " sem usar a web ", " nao quero ", " nao me ", " somente local ", " so local ", " em anexo ", " confidencial ", " privado ", " particular ",
            " paciente ", " endereco ", " mora na ", " moro ", " telefone ", " chave de api ",
            " que te mandei ", " que enviei ", " aqui na conversa ", " que discutimos "])
        || super::memory::explicit_command(message)
        || own_data(&text)
}

/// "minhas notas", "meu arquivo", "minha conta"…: dados guardados da pessoa.
fn own_data(text: &str) -> bool {
    const OWN: [&str; 4] = [" meu ", " meus ", " minha ", " minhas "];
    const DATA: [&str; 30] = ["nota", "pasta", "arquivo", "documento", "projeto", "conversa", "cofre", "foto",
        "imagem", "video", "email", "mensage", "conta", "saldo", "senha", "exame", "dado", "historico",
        "planilha", "codigo", "computador", "contato", "agenda", "calendario", "cpf", "rg", "salario",
        "diagnostico", "prontuario", "npc"];
    OWN.iter().filter_map(|own| text.find(own).map(|i| &text[i + own.len()..]))
        .any(|after| after.split_whitespace().take(3).any(|word| DATA.iter().any(|d| word.starts_with(d))))
}

#[tauri::command]
pub fn suggests_web_search(message: &str) -> bool {
    if private_or_local(message) { return false; }
    let text = words(message);
    let any = |terms: &[&str]| terms.iter().any(|term| text.contains(term));
    let search = any(&[" pesquise ", " pesquisa na ", " pesquisa sobre ", " pesquisar ", " procura ",
        " procure ", " procurar ", " busque ", " busca ", " buscar ", " buscando ", " encontre ",
        " na internet ", " na web ", " com fontes ", " com links ", " onde comprar ", " onde assistir ",
        " ultimas noticias ", " noticias de hoje ", " previsao do tempo ", " preco atual ",
        " versao mais recente ", " ultima versao ", " cotacao de hoje ", " que horas joga ", " horario de hoje ", " noticias recentes ", " em cartaz ", " placar de hoje "]);
    let recommend = any(&[" indica ", " indique ", " recomenda ", " recomende ", 
        " recomendacoes ", " sugira ", " sugere ", " sugestoes de ", " qual o melhor ", " quais os melhores "]);
    let resource = any(&[" youtube ", " video ", " videos ", " links ", " link ", " tutoriais ",
        " artigos ", " documentacao ", " cursos ", " podcasts ", " sites ", " opcoes "]);
    search || recommend || resource && any(&[" manda ", " mande ", " envia ", " envie ", " quero ",
        " preciso ", " encontra ",  " tem ", " onde ", " mostre ", " mostra ", " qual ", " quais "])
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AutoRoute { Local, Cloud, Search, Ask }

fn public_route(message: &str) -> AutoRoute {
    if suggests_web_search(message) { AutoRoute::Search } else { AutoRoute::Cloud }
}

/// Automático: o modelo local entende o pedido e classifica a privacidade pelo sentido. Dado
/// inequívoco (trava) fica local; sem entendimento ou em dúvida, a pessoa escolhe (`Ask`).
/// O entendimento fica guardado para a resposta usar na busca.
pub async fn automatic_route(state: &super::services::Core, prefs: &super::preferences::Preferences,
    message: &str, cancel: &tokio_util::sync::CancellationToken) -> AutoRoute {
    use super::understand::{guard, understand, Privacy};
    if guard(message) { return AutoRoute::Local; }
    let Ok(models) = state.models() else { return AutoRoute::Ask };
    let Ok(model) = super::models::choose(&models, super::models::Capability::Text, state.budget(prefs), prefs.context_tokens, None)
        else { return AutoRoute::Ask };
    let Some(found) = understand(state, &model, prefs, json!(message), cancel).await else { return AutoRoute::Ask };
    // A agenda é dado da Athena: fica neste computador em qualquer modo. Designs são montados pelo
    // modelo local e guardados na Iris.
    let route = if found.offline || found.agenda || found.design { AutoRoute::Local } else { match found.privacy {
        Privacy::Search => AutoRoute::Search,
        Privacy::Public => AutoRoute::Cloud,
        Privacy::Local => AutoRoute::Local,
        Privacy::Doubt => AutoRoute::Ask,
    } };
    if let Ok(mut kept) = state.understood.lock() { *kept = Some((message.to_string(), found)); }
    route
}

/// Escolha feita pela pessoa depois da pergunta. O que é sempre local continua local.
pub fn confirmed_route(message: &str, choice: &str) -> AutoRoute {
    if super::understand::guard(message) { return AutoRoute::Local; }
    match choice {
        "search" => AutoRoute::Search,
        "cloud" => public_route(message),
        _ => AutoRoute::Local,
    }
}

/// Classifica antes do envio, para a interface perguntar quando houver dúvida.
#[tauri::command]
pub async fn automatic_check(state: tauri::State<'_, super::services::Core>, message: String) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let token = tokio_util::sync::CancellationToken::new();
    // O classificador usa o modelo local: espera a vez como as respostas.
    let route = if super::understand::guard(&message) { AutoRoute::Local } else {
        let _gate = state.take_gate(&token, || {}).await?;
        automatic_route(&state, &prefs, &message, &token).await
    };
    let name = match route { AutoRoute::Local => "local", AutoRoute::Cloud => "cloud", AutoRoute::Search => "search", AutoRoute::Ask => "ask" };
    Ok(json!({"route": name, "search": suggests_web_search(&message)}))
}

// Apenas a saída final de uma pesquisa concluída pode virar resposta na conversa.
pub fn answer(response: &Value, videos: bool) -> Result<String, String> {
    let missing = "A pesquisa não encontrou fontes diretas para esse pedido. Tente detalhar o que deseja encontrar.";
    let items = response["output"].as_array().ok_or(missing)?;
    if !items.iter().any(|item| item["type"] == "web_search_call" && item["status"] == "completed") {
        return Err("O modelo não concluiu uma pesquisa na web.".into());
    }
    let mut paragraphs = Vec::new();
    for part in items.iter().filter(|item| item["type"] == "message")
        .filter_map(|item| item["content"].as_array()).flatten().filter(|p| p["type"] == "output_text") {
        let Some(text) = part["text"].as_str() else { continue };
        let mut spans = Vec::new();
        let chars: Vec<_> = text.char_indices().map(|(i, _)| i).chain([text.len()]).collect();
        for citation in part["annotations"].as_array().into_iter().flatten() {
            if citation["type"] != "url_citation" { continue; }
            let Some(url) = citation["url"].as_str().and_then(|s| reqwest::Url::parse(s).ok()) else { continue };
            if !matches!(url.scheme(), "https" | "http") || !url.username().is_empty() || url.password().is_some() { continue; }
            if videos && !video_url(&url) { continue; }
            let offsets = |key: &str| citation[key].as_u64().and_then(|n| usize::try_from(n).ok()).and_then(|n| chars.get(n).copied());
            if let (Some(start), Some(end)) = (offsets("start_index"), offsets("end_index")) {
                if start <= end { spans.push((start, end, url, citation["title"].as_str().unwrap_or("Fonte"))); }
            }
        }
        if spans.is_empty() { continue; }
        spans.sort_by_key(|s| (s.0,s.1));
        let mut cited = Vec::new();
        let mut extra_link = false;
        let has_link = |s: &str| {
            let lower = s.to_lowercase();
            ["://", "www.", "](", "]:", "]["].iter().any(|marker| lower.contains(marker))
        };
        let mut pos = 0;
        let mut output = String::new();
        for (start,end,url,title) in spans {
            if start < pos { continue; }
            let before = &text[pos..start];
            extra_link |= has_link(before);
            output.push_str(before);
            let label = title.replace(['[',']','\\','\n','\r'], " ");
            let link = format!("[{label}](<{}>)",url.as_str());
            let marked = text[start..end].trim();
            if !marked.is_empty() && !has_link(marked) && !marked.starts_with(['[', '(', '']) {
                output.push_str(&text[start..end]);
            }
            output.push(' ');
            output.push_str(&link);
            if !cited.contains(&link) { cited.push(link); }
            pos = end;
        }
        output.push_str(&text[pos..]);
        if extra_link || has_link(&text[pos..]) {
            paragraphs.push(cited.join("\n\n"));
        } else {
            paragraphs.push(output);
        }
    }
    if paragraphs.is_empty() { return Err(missing.into()); }
    Ok(paragraphs.join("\n\n"))
}

pub fn wants_youtube(message: &str) -> bool {
    let text = words(message);
    text.contains(" youtube ") && (text.contains(" video ") || text.contains(" videos ") ||
        [" no youtube ", " do youtube ", " pelo youtube "].iter().any(|phrase| text.contains(phrase))
        && ![" canal ", " canais ", " playlist ", " playlists ", " documentacao ", " politica ", " politicas ", " regras "].iter().any(|phrase| text.contains(phrase)))
}

fn video_url(url: &reqwest::Url) -> bool {
    let valid_id = |s: &str| s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    match url.host_str() {
        Some("youtu.be") => valid_id(url.path().trim_start_matches('/')),
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com") =>
            url.path() == "/watch" && url.query_pairs().any(|(k,v)| k == "v" && valid_id(&v))
            || ["/shorts/", "/live/"].iter().any(|p| url.path().strip_prefix(p).is_some_and(valid_id)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn public_research_is_general_and_private_tasks_stay_local() {
        for message in ["oi ayla, me manda uma receita de frango do youtube", "Me indica três podcasts de história",
            "Quero links para aprender aquarela", "Qual a versão mais recente do Blender?", "Pesquise horários de trem"] {
            assert!(suggests_web_search(message), "{message}");
        }
        for message in ["Me ajuda a escrever uma história", "Pesquise na minha pasta de NPCs", "Me indica algo com base no meu saldo",
            "Pesquise meu CPF", "Qual NPC que eu criei é o melhor?", "Sem internet, me indica um livro",
            "Me lembra de pesquisar vídeos amanhã", "Pesquise na Athena", "Não me mande vídeos do YouTube"] {
            assert!(!suggests_web_search(message), "{message}");
        }
    }

    #[tokio::test]
    async fn automatic_keeps_unmistakable_data_local_and_asks_without_a_model() {
        let dir = std::env::temp_dir().join(format!("ayla-auto-{}", crate::core::store::id()));
        let state = crate::core::services::Core::open(dir.clone()).unwrap();
        let prefs = state.prefs().unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        for (prompt, expected) in [
            ("Me indica um vídeo. CPF 12345678900", AutoRoute::Local),
            ("Quanto gastei no cartão este mês?", AutoRoute::Local),
            // Sem modelo local para entender o pedido, a pessoa escolhe; nada vai à nuvem sozinho.
            ("Explique como funciona a fotossíntese", AutoRoute::Ask),
            ("Pesquise nas minhas notas uma receita de frango", AutoRoute::Ask),
            ("Qual a melhor dieta para o meu caso?", AutoRoute::Ask),
        ] {
            assert_eq!(automatic_route(&state,&prefs,prompt,&token).await,expected,"{prompt}");
        }
        drop(state);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn confirmed_choice_never_overrides_what_is_always_local() {
        assert_eq!(confirmed_route("Qual a melhor dieta para o meu caso?", "cloud"), AutoRoute::Cloud);
        assert_eq!(confirmed_route("Me indica três podcasts de história", "cloud"), AutoRoute::Search);
        assert_eq!(confirmed_route("Qual a melhor dieta para o meu caso?", "search"), AutoRoute::Search);
        assert_eq!(confirmed_route("Qual a melhor dieta para o meu caso?", "local"), AutoRoute::Local);
        for message in ["Pesquise meu CPF 12345678900", "Quanto gastei no cartão este mês?"] {
            assert_eq!(confirmed_route(message, "cloud"), AutoRoute::Local, "{message}");
            assert_eq!(confirmed_route(message, "search"), AutoRoute::Local, "{message}");
        }
        assert_eq!(confirmed_route("Explique a fotossíntese", "qualquer"), AutoRoute::Local);
    }

    #[test]
    fn automatic_model_uses_available_text_model_in_account_order() {
        use crate::core::cloud::{automatic_model,parse_models,Api};
        let catalog = parse_models(Api::ChatGpt, &json!({"models":[
            {"slug":"hidden","visibility":"hidden"},{"slug":"gpt-image","visibility":"list"},
            {"slug":"eligible","visibility":"list"},{"slug":"second","visibility":"list"}]}));
        assert_eq!(automatic_model("chatgpt",&catalog).unwrap(),"eligible");
        let catalog = parse_models(Api::Compatible,&json!({"data":[{"id":"tts-1"},{"id":"gpt-image"},{"id":"gpt-text"}]}));
        assert_eq!(automatic_model("openai",&catalog).unwrap(),"gpt-text");
        assert!(automatic_model("openai",&catalog[..2]).is_err());
    }

    fn response() -> Value {
        let text = "Há duas opções.\n\nAo molho [1]\n\nEmpanado [2]";
        let cite = |marker: &str, title: &str, url: &str| {
            let offset = text[..text.find(marker).unwrap()].chars().count();
            json!({"type":"url_citation", "start_index":offset, "end_index":offset+3, "title":title,"url":url})
        };
        json!({"output":[{"type":"web_search_call","status":"completed"},{"type":"message","content":[{"type":"output_text",
            "text":text,"annotations":[cite("[1]","Primeiro vídeo","https://www.youtube.com/watch?v=abcdefghijk"),
            cite("[2]","Segundo vídeo","https://youtu.be/12345678901")]}]}]})
    }

    #[test]
    fn research_requires_completed_search_and_direct_sources() {
        let mut body = response();
        let answer_text = answer(&body, true).unwrap();
        assert!(answer_text.contains("Ao molho") && answer_text.contains("Empanado") && answer_text.contains("Segundo vídeo"));
        assert!(answer_text.contains("youtu.be/12345678901") && !answer_text.contains("[1]"));
        body["output"][0]["status"] = json!("failed");
        assert!(answer(&body, true).is_err());
        let mut body = response();
        let part = &mut body["output"][1]["content"][0];
        part["annotations"][0]["url"] = json!("https://www.youtube.com/results?search_query=receita");
        part["annotations"][1]["url"] = json!("https://example.org/watch?v=12345678901");
        assert!(answer(&body, true).is_err());
        assert!(answer(&body, false).is_ok());
        body["output"][1]["content"][0]["annotations"] = json!([]);
        assert!(answer(&body, false).is_err());
    }


    #[test]
    fn ungrounded_urls_in_prose_are_discarded() {
        let mut body = response();
        let text = &mut body["output"][1]["content"][0]["text"];
        *text = json!(format!("{} veja https://example.org/inventado", text.as_str().unwrap()));
        let result = answer(&body,true).unwrap();
        assert!(result.contains("Primeiro vídeo") && !result.contains("inventado"));
    }

    #[tokio::test]
    async fn completed_research_is_emitted_once_after_grounding() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use tokio_util::sync::CancellationToken;
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/responses", server.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut headers = [0; 2048];
            socket.read(&mut headers).unwrap();
            let draft = json!({"type":"response.output_text.delta","delta":"rascunho sem fonte"});
            let done = json!({"type":"response.completed","response":response()});
            let body = format!("data: {draft}\n\ndata: {done}\n\n");
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        });
        let outgoing = reqwest::Client::builder().no_proxy().build().unwrap().post(endpoint);
        let mut events = Vec::new();
        let result = crate::core::cloud::stream_search(outgoing, &CancellationToken::new(), true, |event| events.push(event)).await.unwrap();
        worker.join().unwrap();
        assert_eq!(events, vec![json!({"type":"token","text":result})]);
        assert!(result.contains("Empanado") && !result.contains("rascunho"));
    }

    #[tokio::test]
    async fn research_accepts_items_streamed_before_an_empty_completion() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use tokio_util::sync::CancellationToken;
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/responses", server.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut headers = [0; 2048];
            socket.read(&mut headers).unwrap();
            let mut body = String::new();
            for item in response()["output"].as_array().unwrap() {
                body.push_str(&format!("data: {}\n\n", json!({"type":"response.output_item.done","item":item})));
            }
            body.push_str(&format!("data: {}\n\n", json!({"type":"response.completed","response":{"output":[]}})));
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        });
        let outgoing = reqwest::Client::builder().no_proxy().build().unwrap().post(endpoint);
        let result = crate::core::cloud::stream_search(outgoing, &CancellationToken::new(), true, |_| {}).await.unwrap();
        worker.join().unwrap();
        assert!(result.contains("Empanado") && result.contains("youtu.be/12345678901"));
    }

    #[test]
    fn research_uses_connected_account_and_requires_search_tool() {
        use crate::core::cloud::{search_request,Secret};
        for (platform, secret) in [("chatgpt", Secret::ChatGpt("fixture".into())), ("openai", Secret::Key("fixture".into()))] {
            let request = search_request(platform,&secret,"model",Some("low"),"Personalidade revisada","Pedido público").unwrap().build().unwrap();
            assert_eq!(request.url().as_str(),"https://api.openai.com/v1/responses");
            let body: Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
            assert_eq!(body["tools"][0]["type"], "web_search");
            assert_eq!(body["tool_choice"], "required");
            assert_eq!(body["store"],false);
            assert_eq!(body["stream"],true);
            assert_eq!(body["input"].as_array().unwrap().len(),1);
            assert_eq!(body["input"][0]["content"],"Pedido público");
            assert!(body["instructions"].as_str().unwrap().contains("Personalidade revisada"));
        }
        assert!(search_request("custom",&Secret::Key("fixture".into()),"model",None,"p","q").is_err());
    }
}
