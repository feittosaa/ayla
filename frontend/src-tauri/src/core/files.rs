//! Arquivos inseridos nas conversas e projetos: tipos aceitos, extração de texto
//! e o trecho que entra no prompt. Tudo local; anexos nunca vão para a nuvem.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Image,
    Gif,
    Pdf,
    Text,
    Audio,
    Video,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Gif => "gif",
            Kind::Pdf => "pdf",
            Kind::Text => "text",
            Kind::Audio => "audio",
            Kind::Video => "video",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "image" => Kind::Image,
            "gif" => Kind::Gif,
            "pdf" => Kind::Pdf,
            "text" => Kind::Text,
            "audio" => Kind::Audio,
            "video" => Kind::Video,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Image => "imagem",
            Kind::Gif => "GIF",
            Kind::Pdf => "PDF",
            Kind::Text => "texto",
            Kind::Audio => "áudio",
            Kind::Video => "vídeo",
        }
    }
}

pub const IMAGE: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp"];
pub const TEXT: &[&str] = &["txt", "md", "markdown", "csv", "tsv", "json", "xml", "html", "htm", "yaml", "yml", "toml", "log", "srt", "vtt", "rs", "py", "js", "ts", "tsx", "jsx", "java", "c", "h", "cpp", "go", "rb", "php", "sh", "sql", "css"];
pub const AUDIO: &[&str] = &["mp3", "wav", "ogg", "oga", "opus", "m4a", "aac", "flac", "wma", "amr"];
pub const VIDEO: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "mpeg", "mpg", "3gp"];

/// Limite de tamanho por arquivo (vídeos longos); o processamento usa no máximo 10 minutos de áudio.
pub const MAX_BYTES: u64 = 1024 * 1024 * 1024;
/// Quanto texto extraído fica guardado por arquivo.
pub const MAX_EXTRACTED: usize = 200_000;
/// Orçamento de texto de anexos por resposta (cabe no contexto padrão de 8192 tokens).
pub const PROMPT_BUDGET: usize = 8_000;
/// Imagens (fotos, quadros de vídeo e de GIF) por resposta.
pub const MAX_IMAGES: usize = 4;

fn extension(name: &str) -> String {
    Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// Tipo pelo nome e pelos primeiros bytes; `None` para formatos não suportados.
pub fn kind_of(name: &str, head: &[u8]) -> Option<Kind> {
    let ext = extension(name);
    if head.starts_with(b"%PDF") || ext == "pdf" {
        return head.starts_with(b"%PDF").then_some(Kind::Pdf);
    }
    if head.starts_with(b"GIF8") || ext == "gif" {
        return head.starts_with(b"GIF8").then_some(Kind::Gif);
    }
    if head.starts_with(b"\x89PNG") || head.starts_with(&[0xff, 0xd8, 0xff]) || (head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP")) || head.starts_with(b"BM") && IMAGE.contains(&ext.as_str()) {
        return Some(Kind::Image);
    }
    if AUDIO.contains(&ext.as_str()) {
        return Some(Kind::Audio);
    }
    if VIDEO.contains(&ext.as_str()) {
        return Some(Kind::Video);
    }
    if TEXT.contains(&ext.as_str()) && !head.contains(&0) {
        return Some(Kind::Text);
    }
    None
}

/// Nome seguro para gravar no disco: sem pastas, sem caracteres de controle.
pub fn safe_name(name: &str) -> String {
    let base = Path::new(name).file_name().and_then(|n| n.to_str()).unwrap_or("arquivo");
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim().to_string();
    let cleaned: String = cleaned.chars().rev().take(120).collect::<Vec<_>>().into_iter().rev().collect();
    if cleaned.is_empty() { "arquivo".into() } else { cleaned }
}

fn clip(text: &str, limit: usize) -> String {
    let mut out: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        out.push_str("\n[…]");
    }
    out
}

/// Texto de um arquivo de texto (UTF-8, com troca de bytes inválidos).
pub fn text_file(path: &Path) -> io::Result<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.take((MAX_EXTRACTED * 4) as u64).read_to_end(&mut bytes)?;
    Ok(clip(&String::from_utf8_lossy(&bytes), MAX_EXTRACTED))
}

/// Texto de um PDF. PDFs só com imagens (escaneados) voltam vazios.
pub fn pdf_text(path: &Path) -> Result<String, String> {
    let path = path.to_path_buf();
    // A biblioteca pode entrar em pânico com PDFs malformados; isolar.
    let text = std::panic::catch_unwind(move || pdf_extract::extract_text(&path))
        .map_err(|_| "Este PDF não pôde ser lido".to_string())?
        .map_err(|_| "Este PDF não pôde ser lido (pode estar protegido por senha)".to_string())?;
    let compact = text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    let compact = compact.split("\n\n\n").filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join("\n\n");
    Ok(clip(compact.trim(), MAX_EXTRACTED))
}

/// Duração informada pelo ffmpeg (`Duration: 00:01:23.45`), em segundos.
pub fn parse_duration(stderr: &str) -> Option<f64> {
    let rest = stderr.split("Duration: ").nth(1)?;
    let stamp = rest.split(',').next()?.trim();
    let mut parts = stamp.split(':');
    let (h, m, s) = (parts.next()?.parse::<f64>().ok()?, parts.next()?.parse::<f64>().ok()?, parts.next()?.parse::<f64>().ok()?);
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Momentos (em segundos) dos quadros que representam um vídeo: espaçados pelo meio de cada trecho.
pub fn frame_times(duration: f64, count: usize) -> Vec<f64> {
    if duration <= 0.0 || count == 0 {
        return vec![0.0];
    }
    (0..count).map(|i| duration * (i as f64 + 0.5) / count as f64).collect()
}

/// Trecho de um anexo no prompt, com nome e tipo, limitado ao orçamento.
pub fn prompt_block(name: &str, kind: Kind, text: &str, limit: usize) -> String {
    let what = match kind {
        Kind::Audio => "transcrição do áudio",
        Kind::Video => "transcrição do áudio do vídeo",
        _ => "conteúdo",
    };
    format!("[Arquivo anexado: {name} ({}) — {what}]\n{}", kind.label(), clip(text.trim(), limit))
}

/// Divide o orçamento entre os anexos: os primeiros (mais recentes) recebem mais se sobrar.
pub fn budgets(lengths: &[usize], total: usize) -> Vec<usize> {
    let mut out = vec![0; lengths.len()];
    let mut remaining = total;
    let mut open: Vec<usize> = (0..lengths.len()).filter(|&i| lengths[i] > 0).collect();
    while remaining > 0 && !open.is_empty() {
        let share = (remaining / open.len()).max(1);
        let mut next = Vec::new();
        for &i in &open {
            let give = share.min(lengths[i] - out[i]).min(remaining);
            out[i] += give;
            remaining -= give;
            if out[i] < lengths[i] {
                next.push(i);
            }
        }
        if next.len() == open.len() && share == 1 && remaining == 0 {
            break;
        }
        open = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kinds_follow_content_and_extension() {
        assert_eq!(kind_of("relatorio.PDF", b"%PDF-1.7"), Some(Kind::Pdf));
        assert_eq!(kind_of("falso.pdf", b"MZ\x90"), None, "extensão sem o conteúdo esperado é recusada");
        assert_eq!(kind_of("gato.gif", b"GIF89a"), Some(Kind::Gif));
        assert_eq!(kind_of("foto", b"\x89PNG\r\n"), Some(Kind::Image));
        assert_eq!(kind_of("foto.jpg", &[0xff, 0xd8, 0xff, 0xe0]), Some(Kind::Image));
        assert_eq!(kind_of("foto.webp", b"RIFF\0\0\0\0WEBPVP8 "), Some(Kind::Image));
        assert_eq!(kind_of("aula.mp3", b"ID3"), Some(Kind::Audio));
        assert_eq!(kind_of("Reunião.MP4", b"\0\0\0 ftyp"), Some(Kind::Video));
        assert_eq!(kind_of("notas.md", b"# Notas"), Some(Kind::Text));
        assert_eq!(kind_of("binario.txt", b"\0\x01\x02"), None);
        assert_eq!(kind_of("app.exe", b"MZ"), None);
        for kind in [Kind::Image, Kind::Gif, Kind::Pdf, Kind::Text, Kind::Audio, Kind::Video] {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
    }
    #[test]
    fn names_are_safe_on_disk() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\pasta\\nota.txt"), "C__pasta_nota.txt");
        assert_eq!(safe_name(".oculto"), "oculto");
        assert_eq!(safe_name("a\u{0}b.txt"), "a_b.txt");
        assert_eq!(safe_name(""), "arquivo");
        assert!(safe_name(&format!("{}.pdf", "x".repeat(300))).ends_with(".pdf"));
    }
    #[test]
    fn durations_frames_and_budgets() {
        assert_eq!(parse_duration("Input #0\n  Duration: 00:01:30.50, start: 0.0"), Some(90.5));
        assert_eq!(parse_duration("Duration: N/A, bitrate"), None);
        assert_eq!(frame_times(80.0, 4), vec![10.0, 30.0, 50.0, 70.0]);
        assert_eq!(frame_times(0.0, 4), vec![0.0]);
        assert_eq!(budgets(&[100, 100], 1000), vec![100, 100]);
        assert_eq!(budgets(&[5000, 100, 9000], 6000), vec![2950, 100, 2950]);
        assert_eq!(budgets(&[0, 50], 10), vec![0, 10]);
        let block = prompt_block("aula.mp3", Kind::Audio, &"palavra ".repeat(100), 20);
        assert!(block.starts_with("[Arquivo anexado: aula.mp3 (áudio) — transcrição do áudio]") && block.ends_with("[…]"));
    }
    #[test]
    fn text_files_and_invalid_pdfs() {
        let root = std::env::temp_dir().join(format!("ayla-files-{}", super::super::store::id()));
        fs::create_dir_all(&root).unwrap();
        let note = root.join("nota.txt");
        fs::write(&note, b"Ol\xc3\xa1, mundo \xff!").unwrap();
        assert_eq!(text_file(&note).unwrap(), "Olá, mundo \u{fffd}!");
        let broken = root.join("quebrado.pdf");
        fs::write(&broken, b"%PDF-1.4 sem conteudo").unwrap();
        assert!(pdf_text(&broken).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
