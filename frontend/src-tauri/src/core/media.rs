//! Entrega de imagens, vídeos e PDFs à interface (Athena e Iris): trechos pedidos com `Range`
//! (vídeos grandes tocam sem carregar o arquivo inteiro) e miniaturas guardadas em cache.
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tauri::http::{header, Response, StatusCode};

/// Arquivo inteiro só até este tamanho; acima disso, apenas por trechos (`Range`).
pub const MAX_WHOLE: u64 = 80 * 1024 * 1024;
/// Maior trecho entregue de uma vez quando o pedido não diz onde termina.
pub const MAX_CHUNK: u64 = 4 * 1024 * 1024;
/// Lado maior das miniaturas, em pixels.
pub const THUMB_SIZE: i32 = 480;

/// `Range: bytes=início-fim` → (início, fim inclusivo), dentro do tamanho do arquivo.
pub fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let spec = value.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (start, end) = spec.split_once('-')?;
    if size == 0 {
        return None;
    }
    let (start, end) = match (start.trim(), end.trim()) {
        ("", "") => return None,
        // Últimos N bytes.
        ("", suffix) => {
            let n: u64 = suffix.parse().ok()?;
            (size.saturating_sub(n.max(1)), size - 1)
        }
        (start, "") => {
            let start: u64 = start.parse().ok()?;
            (start, (start + MAX_CHUNK - 1).min(size - 1))
        }
        (start, end) => (start.parse().ok()?, end.parse::<u64>().ok()?.min(size - 1)),
    };
    (start <= end && start < size).then_some((start, end))
}

fn fail(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder().status(status).body(Vec::new()).unwrap_or_default()
}

/// Resposta com o arquivo (ou o trecho pedido). Cabeçalhos bloqueiam scripts (SVG) e indicam
/// que o arquivo aceita trechos, como os vídeos precisam.
pub fn serve(path: &Path, mime: &str, range: Option<&str>) -> Response<Vec<u8>> {
    let Ok(mut file) = fs::File::open(path) else { return fail(StatusCode::NOT_FOUND) };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let base = || Response::builder().header(header::CONTENT_TYPE, mime).header(header::ACCEPT_RANGES, "bytes")
        .header("Content-Security-Policy", "default-src 'none'; img-src 'self' data:; media-src 'self'; style-src 'unsafe-inline'")
        .header(header::CACHE_CONTROL, "no-cache")
        // A página da Ayla (tauri://localhost) lê os trechos do player de vídeo por outra origem.
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "tauri://localhost")
        .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "Content-Range, Accept-Ranges, Content-Length");
    if let Some((start, end)) = range.and_then(|r| parse_range(r, size)) {
        let mut bytes = vec![0; (end - start + 1) as usize];
        if file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut bytes).is_err() {
            return fail(StatusCode::NOT_FOUND);
        }
        return base().status(StatusCode::PARTIAL_CONTENT).header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}")).header(header::CONTENT_LENGTH, bytes.len()).body(bytes).unwrap_or_default();
    }
    if range.is_some() {
        return Response::builder().status(StatusCode::RANGE_NOT_SATISFIABLE).header(header::CONTENT_RANGE, format!("bytes */{size}")).body(Vec::new()).unwrap_or_default();
    }
    if size > MAX_WHOLE {
        return fail(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    match file.read_to_end(&mut bytes) {
        Ok(_) => base().status(StatusCode::OK).header(header::CONTENT_LENGTH, bytes.len()).body(bytes).unwrap_or_default(),
        Err(_) => fail(StatusCode::NOT_FOUND),
    }
}

/// Abre o arquivo no aplicativo padrão do sistema (visualizador de imagens, de PDF, player).
pub fn open_in_system(path: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    Command::new("xdg-open").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map(|_| ()).map_err(|_| "Não foi possível abrir o arquivo".into())
}

/// Miniatura JPEG da imagem (ou da primeira página do PDF), criada uma vez em `cache` (pelo
/// caminho, tamanho e data do arquivo). Sem como gerar (formato que o sistema não lê, PDF sem o
/// `pdftoppm` instalado, ou fora do Linux), devolve `None`.
pub fn thumbnail(cache: &Path, path: &Path, pdf: bool) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let meta = fs::metadata(path).ok()?;
    let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let key = Sha256::digest(format!("{}|{}|{stamp}|{THUMB_SIZE}", path.to_string_lossy(), meta.len()).as_bytes());
    let name: String = key.iter().take(16).map(|b| format!("{b:02x}")).collect();
    let target = cache.join(format!("{name}.jpg"));
    if target.is_file() {
        return Some(target);
    }
    fs::create_dir_all(cache).ok()?;
    let made = if pdf { render_pdf_page(path, &target) } else { render_thumbnail(path, &target) };
    made.then_some(target)
}

/// Tempo máximo para desenhar a primeira página de um PDF (arquivos quebrados não travam a grade).
const PDF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Primeira página do PDF em JPEG pelo `pdftoppm` (poppler-utils) do sistema — escolha do usuário
/// (21/11/2026): nada novo dentro da Ayla; sem ele, o cartão fica com o ícone.
fn render_pdf_page(path: &Path, target: &Path) -> bool {
    use std::process::{Command, Stdio};
    let stem = target.with_extension("pdf-tmp");
    let produced = stem.with_extension("pdf-tmp.jpg");
    let Ok(mut child) = Command::new("pdftoppm")
        .args(["-f", "1", "-l", "1", "-singlefile", "-jpeg", "-jpegopt", "quality=82", "-scale-to", &THUMB_SIZE.to_string()])
        .arg(path).arg(&stem)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn() else { return false };
    let start = std::time::Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if start.elapsed() < PDF_TIMEOUT => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => { let _ = child.kill(); let _ = child.wait(); break false; }
        }
    };
    let saved = ok && produced.is_file() && fs::rename(&produced, target).is_ok();
    if !saved {
        let _ = fs::remove_file(&produced);
    }
    saved
}

#[cfg(target_os = "linux")]
fn render_thumbnail(path: &Path, target: &Path) -> bool {
    // gdk-pixbuf já vem com o GTK do Tauri: lê JPEG, PNG, GIF, WebP etc. pelos módulos do sistema.
    let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_file_at_scale(path, THUMB_SIZE, THUMB_SIZE, true) else { return false };
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let pixbuf = if pixbuf.has_alpha() { flatten(&pixbuf) } else { pixbuf };
    let temporary = target.with_extension("tmp");
    let saved = pixbuf.savev(&temporary, "jpeg", &[("quality", "82")]).is_ok() && fs::rename(&temporary, target).is_ok();
    if !saved {
        let _ = fs::remove_file(&temporary);
    }
    saved
}

/// JPEG não tem transparência: compõe sobre fundo escuro (o da Iris), numa imagem sem alfa.
#[cfg(target_os = "linux")]
fn flatten(pixbuf: &gdk_pixbuf::Pixbuf) -> gdk_pixbuf::Pixbuf {
    let (width, height) = (pixbuf.width(), pixbuf.height());
    let Some(base) = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, width, height) else { return pixbuf.clone() };
    base.fill(0x1f1f23ff);
    pixbuf.composite(&base, 0, 0, width, height, 0.0, 0.0, 1.0, 1.0, gdk_pixbuf::InterpType::Bilinear, 255);
    base
}

#[cfg(not(target_os = "linux"))]
fn render_thumbnail(_path: &Path, _target: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trechos_pedidos_pelo_range() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=0-", 100 * 1024 * 1024), Some((0, MAX_CHUNK - 1)), "sem fim: trecho limitado");
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=0-5000", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=5-2", 1000), None);
        assert_eq!(parse_range("linhas=0-1", 1000), None);
    }

    #[test]
    fn entrega_inteira_ou_por_trecho() {
        let dir = std::env::temp_dir().join(format!("ayla-media-{}", super::super::store::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v.mp4");
        fs::write(&path, b"0123456789").unwrap();
        let whole = serve(&path, "video/mp4", None);
        assert_eq!((whole.status(), whole.body().as_slice()), (StatusCode::OK, b"0123456789".as_slice()));
        let part = serve(&path, "video/mp4", Some("bytes=2-4"));
        assert_eq!((part.status(), part.body().as_slice()), (StatusCode::PARTIAL_CONTENT, b"234".as_slice()));
        assert_eq!(part.headers()[header::CONTENT_RANGE], "bytes 2-4/10");
        assert_eq!(serve(&path, "video/mp4", Some("bytes=20-")).status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(serve(&dir.join("nada.mp4"), "video/mp4", None).status(), StatusCode::NOT_FOUND);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn miniatura_criada_uma_vez_e_reaproveitada() {
        let dir = std::env::temp_dir().join(format!("ayla-thumb-{}", super::super::store::id()));
        fs::create_dir_all(&dir).unwrap();
        let image = dir.join("foto.png");
        let pixbuf = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 1600, 800).unwrap();
        pixbuf.fill(0x4080c0ff);
        pixbuf.savev(&image, "png", &[]).unwrap();
        let cache = dir.join("cache");
        let thumb = thumbnail(&cache, &image, false).expect("miniatura");
        let small = gdk_pixbuf::Pixbuf::from_file(&thumb).unwrap();
        assert_eq!((small.width(), small.height()), (THUMB_SIZE, THUMB_SIZE / 2));
        let modified = fs::metadata(&thumb).unwrap().modified().unwrap();
        assert_eq!(thumbnail(&cache, &image, false), Some(thumb.clone()));
        assert_eq!(fs::metadata(&thumb).unwrap().modified().unwrap(), modified, "reaproveita o cache");
        fs::write(dir.join("quebrada.png"), b"nada").unwrap();
        fs::write(dir.join("quebrado.pdf"), b"nada").unwrap();
        assert_eq!(thumbnail(&cache, &dir.join("quebrado.pdf"), true), None, "PDF quebrado não vira miniatura");
        assert!(fs::read_dir(&cache).unwrap().all(|e| e.unwrap().path().extension().is_some_and(|x| x == "jpg")), "sem sobras temporárias");
        assert_eq!(thumbnail(&cache, &dir.join("quebrada.png"), false), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Precisa do `pdftoppm` (poppler-utils) instalado.
    #[test]
    fn primeira_pagina_do_pdf_vira_miniatura() {
        if std::process::Command::new("pdftoppm").arg("-v").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("ayla-pdf-{}", super::super::store::id()));
        fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("doc.pdf");
        fs::write(&pdf, "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 600]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n").unwrap();
        let thumb = thumbnail(&dir.join("cache"), &pdf, true).expect("miniatura do PDF");
        let bytes = fs::read(&thumb).unwrap();
        assert_eq!(&bytes[..2], &[0xff, 0xd8], "JPEG");
        let _ = fs::remove_dir_all(&dir);
    }
}
