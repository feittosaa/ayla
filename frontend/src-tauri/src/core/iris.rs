//! Iris: respostas a partir dos arquivos guardados na Ayla (parte da própria Ayla).
//!
//! Os arquivos do projeto e da conversa são divididos em trechos (com a página, nos PDFs),
//! indexados por palavras (FTS5) e, havendo modelo de embeddings, por significado. Na resposta
//! local entram só os trechos mais relevantes, com o nome do arquivo para a citação.
//! Nada disso vai para a nuvem.
use super::files::{self, Kind};
use super::store::Chunk;
use std::path::Path;

/// Tamanho alvo de um trecho, em caracteres, e quanto ele repete do anterior.
pub const CHUNK_CHARS: usize = 900;
pub const OVERLAP_CHARS: usize = 150;
/// Trechos levados à resposta e orçamento total de caracteres.
pub const CONTEXT_CHUNKS: usize = 6;
pub const CONTEXT_BUDGET: usize = 6_000;
/// Vetores calculados por pedido, para a primeira pergunta não demorar demais; o resto fica
/// para as próximas (a busca por palavras já cobre o arquivo todo).
pub const EMBED_PER_REQUEST: usize = 96;
/// Semelhança mínima para um trecho entrar pela busca por significado.
pub const SEMANTIC_MIN: f64 = 0.35;

/// Divide o texto em trechos de até ~`CHUNK_CHARS`, preferindo quebras de parágrafo e de frase.
pub fn split(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for paragraph in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        let size = |s: &str| s.chars().count();
        if size(&current) + size(paragraph) + 2 <= CHUNK_CHARS {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(paragraph);
            continue;
        }
        if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        if size(paragraph) <= CHUNK_CHARS {
            current.push_str(paragraph);
            continue;
        }
        // Parágrafo longo: janelas com sobreposição, cortadas em fim de frase ou espaço.
        let chars: Vec<char> = paragraph.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let mut end = (start + CHUNK_CHARS).min(chars.len());
            if end < chars.len() {
                let window = &chars[start..end];
                let cut = window.iter().rposition(|c| matches!(c, '.' | '!' | '?' | ';'))
                    .filter(|i| *i > CHUNK_CHARS / 2)
                    .or_else(|| window.iter().rposition(|c| c.is_whitespace()).filter(|i| *i > CHUNK_CHARS / 2));
                if let Some(cut) = cut {
                    end = start + cut + 1;
                }
            }
            out.push(chars[start..end].iter().collect::<String>().trim().to_string());
            if end >= chars.len() {
                break;
            }
            start = end.saturating_sub(OVERLAP_CHARS).max(start + 1);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out.into_iter().filter(|c| !c.trim().is_empty()).collect()
}

/// Trechos de um arquivo guardado. PDFs são lidos página a página (para citar a página);
/// textos, pelo conteúdo; áudio e vídeo, pela transcrição já feita. Imagens não têm texto.
pub fn file_chunks(kind: Kind, path: &Path, extracted: Option<&str>) -> Vec<(Option<u32>, String)> {
    match kind {
        Kind::Pdf => {
            let owned = path.to_path_buf();
            let pages = std::panic::catch_unwind(move || pdf_extract::extract_text_by_pages(&owned)).ok().and_then(Result::ok);
            match pages {
                Some(pages) if pages.iter().any(|p| !p.trim().is_empty()) => pages
                    .iter()
                    .enumerate()
                    .flat_map(|(i, page)| split(&tidy(page)).into_iter().map(move |c| (Some(i as u32 + 1), c)))
                    .collect(),
                _ => extracted.map(|t| split(t).into_iter().map(|c| (None, c)).collect()).unwrap_or_default(),
            }
        }
        Kind::Text => {
            let text = files::text_file(path).ok().or_else(|| extracted.map(str::to_string)).unwrap_or_default();
            split(&text).into_iter().map(|c| (None, c)).collect()
        }
        Kind::Audio | Kind::Video => extracted.map(|t| split(t).into_iter().map(|c| (None, c)).collect()).unwrap_or_default(),
        Kind::Image | Kind::Gif => Vec::new(),
    }
}

/// Linhas sem espaços no fim e sem sequências longas de linhas vazias (saída do pdf-extract).
fn tidy(page: &str) -> String {
    let lines = page.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    lines.split("\n\n\n").filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join("\n\n")
}

/// Como a resposta cita o trecho: "[relatorio.pdf, p. 3]".
pub fn citation(chunk: &Chunk) -> String {
    match chunk.page {
        Some(page) => format!("[{}, p. {page}]", chunk.file_name),
        None => format!("[{}]", chunk.file_name),
    }
}

/// Trechos dentro do orçamento, na ordem de relevância.
pub fn within_budget(chunks: Vec<Chunk>) -> Vec<Chunk> {
    let mut used = 0;
    chunks
        .into_iter()
        .take(CONTEXT_CHUNKS)
        .take_while(|c| {
            used += c.text.chars().count();
            used <= CONTEXT_BUDGET
        })
        .collect()
}

/// Bloco do prompt de sistema (só em respostas locais).
pub fn context_block(chunks: &[Chunk]) -> Option<String> {
    if chunks.is_empty() {
        return None;
    }
    let body = chunks.iter().map(|c| format!("{}\n{}", citation(c), c.text.trim())).collect::<Vec<_>>().join("\n\n");
    Some(format!(
        "\n\nTrechos dos arquivos guardados nesta conversa ou neste projeto. Use-os quando ajudarem a responder e indique de onde veio cada informação citando a origem entre colchetes, como {}. Se a resposta não estiver nos trechos, diga que não a encontrou nos arquivos; não invente.\n\n{body}",
        citation(&chunks[0])
    ))
}

/// Fontes mostradas abaixo da resposta: um item por arquivo, com as páginas usadas.
pub fn sources(chunks: &[Chunk]) -> Vec<serde_json::Value> {
    let mut out: Vec<(String, String, Vec<u32>)> = Vec::new();
    for chunk in chunks {
        let entry = match out.iter_mut().find(|(id, _, _)| *id == chunk.file_id) {
            Some(entry) => entry,
            None => {
                out.push((chunk.file_id.clone(), chunk.file_name.clone(), Vec::new()));
                out.last_mut().unwrap()
            }
        };
        if let Some(page) = chunk.page {
            if !entry.2.contains(&page) {
                entry.2.push(page);
            }
        }
    }
    out.into_iter()
        .map(|(id, name, mut pages)| {
            pages.sort_unstable();
            serde_json::json!({"id": id, "name": name, "pages": pages})
        })
        .collect()
}


// Comparação de fotos ao longo do tempo (ex.: evolução de um treino em 6 meses).

/// Pedido que compara imagens no tempo: palavra de comparação e, junto, uma de foto ou corpo
/// ou um período ("6 meses"), para não confundir com perguntas como "resultado do mês".
pub fn comparison_question(message: &str) -> bool {
    let text = message.to_lowercase();
    let compare = [
        "compar", "evolu", "progress", "antes e depois", "diferença", "diferenca", "mudou", "mudança", "mudanca",
        "surtindo efeito", "fazendo efeito", "ao longo",
    ]
    .iter()
    .any(|w| text.contains(w));
    let photo = ["foto", "imagem", "imagens", "corpo", "físico", "fisico", "treino", "aparência", "aparencia", "shape", "pele", "cabelo"]
        .iter()
        .any(|w| text.contains(w));
    compare && (photo || period(message).is_some())
}

/// Período citado no pedido ("6 meses", "três semanas", "1 ano"), em segundos.
pub fn period(message: &str) -> Option<i64> {
    let words: Vec<String> = message
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect();
    let number = |w: &str| -> Option<i64> {
        w.parse().ok().or(match w {
            "um" | "uma" => Some(1), "dois" | "duas" => Some(2), "três" | "tres" => Some(3), "quatro" => Some(4),
            "cinco" => Some(5), "seis" => Some(6), "sete" => Some(7), "oito" => Some(8), "nove" => Some(9),
            "dez" => Some(10), "onze" => Some(11), "doze" => Some(12), _ => None,
        })
    };
    const DAY: i64 = 86_400;
    words.windows(2).find_map(|pair| {
        let n = number(&pair[0]).filter(|n| (1..=120).contains(n))?;
        let unit = match pair[1].as_str() {
            "dia" | "dias" => DAY,
            "semana" | "semanas" => 7 * DAY,
            "mês" | "mes" | "meses" => 30 * DAY,
            "ano" | "anos" => 365 * DAY,
            _ => return None,
        };
        Some(n * unit)
    })
}

/// Dias desde 1970-01-01 para uma data civil (algoritmo de Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "2026:05:12 10:30:00" (formato EXIF) em segundos Unix, sem fuso (horário da câmera).
pub fn exif_seconds(value: &str) -> Option<i64> {
    let parts: Vec<i64> = value.split(|c: char| c == ':' || c == ' ' || c == '-').filter(|p| !p.is_empty()).map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    let [y, mo, d, h, mi, s] = parts.get(..6)?.try_into().ok()?;
    if !(1900..=2200).contains(&y) || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3_600 + mi * 60 + s)
}

/// Data em que a foto foi tirada, pelo EXIF (original, depois a geral).
pub fn photo_date(path: &Path) -> Option<i64> {
    let file = std::fs::File::open(path).ok()?;
    let data = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)).ok()?;
    [exif::Tag::DateTimeOriginal, exif::Tag::DateTime].iter().find_map(|tag| {
        let field = data.get_field(*tag, exif::In::PRIMARY)?;
        match &field.value {
            exif::Value::Ascii(values) => values.first().and_then(|v| exif_seconds(&String::from_utf8_lossy(v))),
            _ => None,
        }
    })
}

/// "12/05/2026" para segundos Unix.
pub fn date_label(seconds: i64) -> String {
    let z = seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{day:02}/{month:02}/{year}")
}

/// Até `max` itens espaçados de uma lista em ordem: sempre o primeiro e o último.
pub fn spread<T: Clone>(sorted: &[T], max: usize) -> Vec<T> {
    if sorted.len() <= max || max < 2 {
        return sorted.iter().take(max.max(sorted.len().min(max))).cloned().collect();
    }
    (0..max).map(|i| sorted[i * (sorted.len() - 1) / (max - 1)].clone()).collect()
}

/// Fotos de um período: as que estão até `period` antes da mais recente.
pub fn within_period<T: Clone>(dated: &[(T, i64)], period: Option<i64>) -> Vec<(T, i64)> {
    let Some(latest) = dated.iter().map(|(_, d)| *d).max() else { return Vec::new() };
    dated.iter().filter(|(_, d)| period.is_none_or(|p| *d >= latest - p)).cloned().collect()
}

/// Texto que acompanha as fotos, na mesma ordem em que vão para o modelo de visão.
pub fn photos_block(photos: &[(String, i64)]) -> String {
    let list = photos.iter().enumerate().map(|(i, (name, date))| format!("Foto {}: {name}, de {}", i + 1, date_label(*date))).collect::<Vec<_>>().join("\n");
    format!(
        "[Fotos do projeto em ordem cronológica, para comparar]\n{list}\nCompare as fotos na ordem, citando cada uma pela data. Descreva só mudanças visíveis e diga com cautela o que pode ter mudado; diferenças de pose, ângulo, luz, roupa e distância também alteram a aparência, então aponte quando elas atrapalham a comparação. Não faça diagnóstico de saúde."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: i64, file: &str, page: Option<u32>, text: &str) -> Chunk {
        Chunk { id, file_id: file.into(), file_name: format!("{file}.pdf"), page, text: text.into() }
    }

    #[test]
    fn divide_por_paragrafo_e_janela_com_sobreposicao() {
        let short = split("Primeiro parágrafo.\n\nSegundo parágrafo.\n\n\n");
        assert_eq!(short, vec!["Primeiro parágrafo.\n\nSegundo parágrafo."]);
        let sentence = "Frase com algumas palavras para encher o trecho. ";
        let long = sentence.repeat(60);
        let parts = split(&long);
        assert!(parts.len() >= 3, "{}", parts.len());
        assert!(parts.iter().all(|p| p.chars().count() <= CHUNK_CHARS));
        assert!(parts.iter().all(|p| p.ends_with('.')), "corta em fim de frase");
        // Sobreposição: o começo de um trecho repete o fim do anterior.
        let tail: String = parts[0].chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
        assert!(parts[1].contains(tail.trim()));
        assert!(split("   \n\n  ").is_empty());
    }

    #[test]
    fn contexto_cita_arquivo_e_pagina_e_respeita_orcamento() {
        let chunks = vec![chunk(1, "relatorio", Some(3), "O prazo é 12 de março."), chunk(2, "notas", None, "Reunião na sexta.")];
        let block = context_block(&chunks).unwrap();
        assert!(block.contains("[relatorio.pdf, p. 3]\nO prazo é 12 de março."));
        assert!(block.contains("[notas.pdf]\nReunião na sexta."));
        assert!(block.contains("como [relatorio.pdf, p. 3]"));
        assert!(context_block(&[]).is_none());
        let many: Vec<Chunk> = (0..10).map(|i| chunk(i, "a", None, &"x".repeat(1_500))).collect();
        assert_eq!(within_budget(many).len(), 4);
    }

    #[test]
    fn comparacao_por_periodo_e_datas() {
        assert!(comparison_question("o treino novo tá surtindo efeito nesses 6 meses?"));
        assert!(comparison_question("compara minhas fotos"));
        assert!(!comparison_question("qual o prazo do contrato?"));
        assert!(!comparison_question("qual a diferença entre os dois contratos?"));
        assert!(comparison_question("mudou alguma coisa em 3 meses?"));
        assert_eq!(period("nesses 6 meses"), Some(180 * 86_400));
        assert_eq!(period("nas últimas três semanas"), Some(21 * 86_400));
        assert_eq!(period("em um ano"), Some(365 * 86_400));
        assert_eq!(period("compara as fotos"), None);
        let may = exif_seconds("2026:05:12 10:30:00").unwrap();
        assert_eq!(date_label(may), "12/05/2026");
        assert_eq!(date_label(exif_seconds("2024:02:29 00:00:00").unwrap()), "29/02/2024");
        assert_eq!(exif_seconds("0000:00:00 00:00:00"), None);
        let dated: Vec<(u32, i64)> = (0..10).map(|i| (i, i as i64 * 30 * 86_400)).collect();
        let recent = within_period(&dated, Some(180 * 86_400));
        assert_eq!(recent.first().unwrap().0, 3, "só os últimos 6 meses a partir da foto mais recente");
        assert_eq!(spread(&(0..10).collect::<Vec<_>>(), 4), vec![0, 3, 6, 9]);
        assert_eq!(spread(&[1, 2], 4), vec![1, 2]);
        let block = photos_block(&[("a.jpg".into(), may), ("b.jpg".into(), may + 86_400)]);
        assert!(block.contains("Foto 1: a.jpg, de 12/05/2026\nFoto 2: b.jpg, de 13/05/2026"));
    }

    /// Foto real com EXIF: `AYLA_EXIF_SAMPLE` aponta uma pasta com com_exif.jpg (DateTimeOriginal
    /// 2025:05:12 10:30:00, DateTime 2026:01:02) e sem_exif.png.
    #[test]
    #[ignore]
    fn le_a_data_da_camera() {
        let dir = std::path::PathBuf::from(std::env::var("AYLA_EXIF_SAMPLE").expect("AYLA_EXIF_SAMPLE"));
        let date = photo_date(&dir.join("com_exif.jpg")).expect("data do EXIF");
        assert_eq!(date_label(date), "12/05/2025", "prefere a data original da câmera");
        assert_eq!(photo_date(&dir.join("sem_exif.png")), None);
    }

    #[test]
    fn fontes_agrupam_paginas_por_arquivo() {
        let chunks = vec![chunk(1, "a", Some(5), ""), chunk(2, "b", None, ""), chunk(3, "a", Some(2), ""), chunk(4, "a", Some(5), "")];
        let list = sources(&chunks);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["name"], "a.pdf");
        assert_eq!(list[0]["pages"], serde_json::json!([2, 5]));
        assert_eq!(list[1]["pages"], serde_json::json!([]));
    }
}
