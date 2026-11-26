//! Iris: designs (estilo Canva). O projeto editável é o JSON do Fabric.js, guardado na pasta de
//! dados da Ayla, um diretório por design (`designs/<id>/`: `meta.json`, `design.json` e
//! `preview.png`). Também lê o pacote de recursos opcional (fontes e ícones) e monta o PDF da
//! exportação a partir de um JPEG, sem dependência nova.
use base64::Engine;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Limites: projeto (com imagens embutidas), prévia e lado do design.
pub const MAX_DATA: usize = 80 << 20;
pub const MAX_PREVIEW: usize = 4 << 20;
pub const MAX_SIDE: u32 = 8000;

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn folder(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) { return Err("Design inválido".into()); }
    Ok(root.join("designs").join(id))
}

/// Grava num temporário e troca, para um design nunca ficar pela metade.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).and_then(|_| fs::rename(&temporary, path)).map_err(|_| {
        let _ = fs::remove_file(&temporary);
        "Não foi possível guardar o design".to_string()
    })
}

fn clean_name(name: &str) -> String {
    let name: String = name.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(120).collect();
    if name.is_empty() { "Design sem título".into() } else { name }
}

pub struct Save<'a> { pub id: Option<&'a str>, pub name: &'a str, pub width: u32, pub height: u32, pub data: &'a str, pub preview: &'a str }

/// Cria (sem `id`) ou atualiza um design. Devolve o id.
pub fn save(root: &Path, design: Save, new_id: impl FnOnce() -> String, now: i64) -> Result<String, String> {
    if !(16..=MAX_SIDE).contains(&design.width) || !(16..=MAX_SIDE).contains(&design.height) {
        return Err(format!("O design deve ter de 16 a {MAX_SIDE} px de lado"));
    }
    if design.data.len() > MAX_DATA { return Err("O design passou de 80 MB (imagens grandes demais)".into()); }
    serde_json::from_str::<Value>(design.data).ok().filter(Value::is_object).ok_or("Design inválido")?;
    let preview = base64::engine::general_purpose::STANDARD.decode(design.preview).map_err(|_| "Prévia inválida")?;
    if preview.len() > MAX_PREVIEW || !preview.starts_with(b"\x89PNG\r\n\x1a\n") { return Err("Prévia inválida".into()); }
    let id = match design.id { Some(id) => id.to_string(), None => new_id() };
    let dir = folder(root, &id)?;
    let created = match design.id {
        Some(_) => {
            let meta = read_meta(&dir).ok_or("Este design não existe mais")?;
            meta["created"].as_i64().unwrap_or(now)
        }
        None => now,
    };
    fs::create_dir_all(&dir).map_err(|_| "Não foi possível guardar o design")?;
    write_atomic(&dir.join("design.json"), design.data.as_bytes())?;
    write_atomic(&dir.join("preview.png"), &preview)?;
    let meta = json!({"name": clean_name(design.name), "width": design.width, "height": design.height, "created": created, "modified": now});
    write_atomic(&dir.join("meta.json"), meta.to_string().as_bytes())?;
    Ok(id)
}

/// Rascunho criado pela Ayla na conversa (`{"ayla_draft": …}`), ainda sem prévia: a interface monta
/// com o Fabric.js e salva por cima, com a prévia, no mesmo id.
pub fn save_draft(root: &Path, name: &str, width: u32, height: u32, draft: &Value, id: String, now: i64) -> Result<String, String> {
    if !(16..=MAX_SIDE).contains(&width) || !(16..=MAX_SIDE).contains(&height) { return Err("Tamanho de design inválido".into()); }
    let data = json!({"ayla_draft": draft}).to_string();
    if data.len() > MAX_DATA { return Err("O design passou de 80 MB (imagens grandes demais)".into()); }
    let dir = folder(root, &id)?;
    fs::create_dir_all(&dir).map_err(|_| "Não foi possível guardar o design")?;
    write_atomic(&dir.join("design.json"), data.as_bytes())?;
    let meta = json!({"name": clean_name(name), "width": width, "height": height, "created": now, "modified": now});
    write_atomic(&dir.join("meta.json"), meta.to_string().as_bytes())?;
    Ok(id)
}

fn read_meta(dir: &Path) -> Option<Value> {
    serde_json::from_slice::<Value>(&fs::read(dir.join("meta.json")).ok()?).ok().filter(Value::is_object)
}

/// Designs guardados, do mais recente ao mais antigo, com a prévia como data URL.
pub fn list(root: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(root.join("designs")) else { return Vec::new() };
    let mut out: Vec<Value> = entries.flatten().filter_map(|entry| {
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid_id(&id) || !entry.path().join("design.json").is_file() { return None; }
        let mut meta = read_meta(&entry.path())?;
        let preview = fs::read(entry.path().join("preview.png")).ok()
            .map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)));
        meta["id"] = json!(id);
        meta["preview"] = json!(preview);
        Some(meta)
    }).collect();
    out.sort_by_key(|m| std::cmp::Reverse(m["modified"].as_i64().unwrap_or(0)));
    out
}

pub fn read(root: &Path, id: &str) -> Result<Value, String> {
    let dir = folder(root, id)?;
    let mut meta = read_meta(&dir).ok_or("Este design não existe mais")?;
    let data = fs::read_to_string(dir.join("design.json")).map_err(|_| "Não foi possível abrir o design")?;
    meta["id"] = json!(id);
    meta["data"] = json!(data);
    Ok(meta)
}

/// Pasta do design, para mandar à lixeira do sistema.
pub fn path(root: &Path, id: &str) -> Result<PathBuf, String> {
    let dir = folder(root, id)?;
    if !dir.join("meta.json").is_file() { return Err("Este design não existe mais".into()); }
    Ok(dir)
}

/// Nome de arquivo da exportação: o nome do design sem caracteres de caminho.
pub fn export_name(name: &str, extension: &str) -> String {
    let stem: String = clean_name(name).chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c }).collect();
    let stem = stem.trim_matches(|c: char| c == '.' || c.is_whitespace());
    format!("{}.{extension}", if stem.is_empty() { "Design" } else { stem })
}

/// PDF de uma página com a imagem JPEG ocupando a página inteira (`page` em pontos).
pub fn pdf_from_jpeg(jpeg: &[u8], pixels: (u32, u32), page: (f64, f64)) -> Result<Vec<u8>, String> {
    if !jpeg.starts_with(&[0xFF, 0xD8]) { return Err("Imagem da exportação inválida".into()); }
    let (w, h) = page;
    if !(w.is_finite() && h.is_finite() && w > 1.0 && h > 1.0 && w <= 14_400.0 && h <= 14_400.0) {
        return Err("Tamanho de página inválido".into());
    }
    let content = format!("q\n{w:.2} 0 0 {h:.2} 0 0 cm\n/Im0 Do\nQ\n");
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>").into_bytes(),
    ];
    let mut image = format!("<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n", pixels.0, pixels.1, jpeg.len()).into_bytes();
    image.extend_from_slice(jpeg);
    image.extend_from_slice(b"\nendstream");
    objects.push(image);
    objects.push(format!("<< /Length {} >>\nstream\n{content}endstream", content.len()).into_bytes());
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets { out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes()); }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    Ok(out)
}

/// Nome de exibição das fontes do pacote.
const FAMILIES: &[(&str, &str)] = &[
    ("inter", "Inter"), ("montserrat", "Montserrat"), ("poppins", "Poppins"), ("playfair-display", "Playfair Display"),
    ("bebas-neue", "Bebas Neue"), ("oswald", "Oswald"), ("lobster", "Lobster"), ("pacifico", "Pacifico"),
    ("dancing-script", "Dancing Script"), ("roboto-slab", "Roboto Slab"), ("lora", "Lora"), ("anton", "Anton"),
];

/// Fontes instaladas: família e, para cada peso/estilo, os arquivos (`latin` e `latin-ext`).
pub fn fonts(pack: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(pack.join("fonts")) else { return Vec::new() };
    let files: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".woff2")).collect();
    FAMILIES.iter().filter_map(|(slug, family)| {
        let mut faces = Vec::new();
        for weight in [400, 700] {
            for style in ["normal", "italic"] {
                let mut found: Vec<&String> = files.iter().filter(|f| {
                    f.strip_prefix(slug).and_then(|r| r.strip_prefix('-')).is_some_and(|r| r == format!("latin-{weight}-{style}.woff2") || r == format!("latin-ext-{weight}-{style}.woff2"))
                }).collect();
                found.sort();
                if !found.is_empty() { faces.push(json!({"weight": weight, "style": style, "files": found})); }
            }
        }
        (!faces.is_empty()).then(|| json!({"family": family, "faces": faces}))
    }).collect()
}

/// Bytes de um arquivo de fonte do pacote (só nomes simples `.woff2`).
pub fn font(pack: &Path, file: &str) -> Result<Vec<u8>, String> {
    if !file.ends_with(".woff2") || file.contains('/') || file.contains("..") || file.starts_with('.') {
        return Err("Fonte inválida".into());
    }
    fs::read(pack.join("fonts").join(file)).map_err(|_| "Fonte não encontrada".into())
}

/// Ícones do pacote pelo nome ou pelas palavras-chave do Lucide (em inglês), até `limit`.
/// Ícones do pacote: todos (sem busca) ou os que casam com a busca, em páginas (`offset`/`limit`).
/// `more` diz se há mais para carregar.
pub fn icons(pack: &Path, query: &str, offset: usize, limit: usize) -> Value {
    let tags: serde_json::Map<String, Value> = fs::read(pack.join("icons/tags.json")).ok()
        .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let query = query.trim().to_lowercase();
    let mut names: Vec<(u8, &String)> = tags.iter().filter_map(|(name, words)| {
        if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') { return None; }
        if query.is_empty() { return Some((1, name)); }
        if name == &query { return Some((0, name)); }
        if name.contains(&query) { return Some((1, name)); }
        words.as_array()?.iter().filter_map(Value::as_str).any(|w| w.to_lowercase().contains(&query)).then_some((2, name))
    }).collect();
    names.sort();
    let total = names.len();
    let items: Vec<Value> = names.into_iter().skip(offset).take(limit).filter_map(|(_, name)| {
        let svg = fs::read_to_string(pack.join("icons").join(format!("{name}.svg"))).ok()?;
        Some(json!({"name": name, "svg": svg}))
    }).collect();
    json!({"items": items, "more": offset + limit < total, "total": total})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-design-{}", super::super::store::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    #[test]
    fn saves_lists_reads_and_updates() {
        let root = temp();
        let id = save(&root, Save { id: None, name: "  Post\n de teste ", width: 1080, height: 1080, data: r#"{"objects":[]}"#, preview: PNG }, || "abc".into(), 10).unwrap();
        assert_eq!(id, "abc");
        let listed = list(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["name"], "Post de teste");
        assert!(listed[0]["preview"].as_str().unwrap().starts_with("data:image/png;base64,"));
        save(&root, Save { id: Some("abc"), name: "Novo", width: 1080, height: 1920, data: r#"{"objects":[1]}"#, preview: PNG }, || unreachable!(), 20).unwrap();
        let read = read(&root, "abc").unwrap();
        assert_eq!((read["name"].as_str(), read["height"].as_u64(), read["created"].as_i64(), read["modified"].as_i64()), (Some("Novo"), Some(1920), Some(10), Some(20)));
        assert_eq!(read["data"], r#"{"objects":[1]}"#);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn draft_is_listed_without_preview_and_replaced_by_the_editor() {
        let root = temp();
        let id = save_draft(&root, "Promo", 1080, 1920, &json!({"format": "story", "objects": []}), "rasc1".into(), 5).unwrap();
        let listed = list(&root);
        assert_eq!((listed[0]["name"].as_str(), listed[0]["preview"].is_null()), (Some("Promo"), true));
        assert_eq!(serde_json::from_str::<Value>(read(&root, &id).unwrap()["data"].as_str().unwrap()).unwrap()["ayla_draft"]["format"], "story");
        save(&root, Save { id: Some(&id), name: "Promo", width: 1080, height: 1920, data: r#"{"objects":[]}"#, preview: PNG }, || unreachable!(), 9).unwrap();
        assert_eq!((list(&root)[0]["created"].as_i64(), list(&root)[0]["preview"].is_string()), (Some(5), true));
        assert!(save_draft(&root, "x", 1080, 1080, &json!({}), "../fora".into(), 1).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_bad_input() {
        let root = temp();
        let base = |data, preview, width| Save { id: None, name: "x", width, height: 100, data, preview };
        assert!(save(&root, base("{}", PNG, 8), || "a".into(), 0).is_err());
        assert!(save(&root, base("[]", PNG, 100), || "a".into(), 0).is_err());
        assert!(save(&root, base("{}", "AAAA", 100), || "a".into(), 0).is_err());
        assert!(save(&root, Save { id: Some("nao-existe"), ..base("{}", PNG, 100) }, || "a".into(), 0).is_err());
        assert!(read(&root, "../x").is_err() && path(&root, "a/b").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_names_are_plain_files() {
        assert_eq!(export_name("Cartaz/A4: festa?", "pdf"), "Cartaz-A4- festa-.pdf");
        assert_eq!(export_name("  ..  ", "png"), "Design.png");
    }

    #[test]
    fn pdf_wraps_the_jpeg_in_one_page() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 0xFF, 0xD9];
        let pdf = pdf_from_jpeg(&jpeg, (10, 20), (595.0, 842.0)).unwrap();
        let has = |needle: &[u8]| pdf.windows(needle.len()).any(|w| w == needle);
        assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
        assert!(has(b"/MediaBox [0 0 595.00 842.00]") && has(b"/Width 10 /Height 20") && has(b"/Filter /DCTDecode") && has(&jpeg));
        let tail = String::from_utf8_lossy(&pdf[pdf.len() - 40..]).into_owned();
        let xref: usize = tail.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        let table = String::from_utf8_lossy(&pdf[xref..]).into_owned();
        assert!(table.starts_with("xref"));
        for (n, line) in table.lines().skip(3).take(5).enumerate() {
            let offset: usize = line[..10].parse().unwrap();
            assert!(pdf[offset..].starts_with(format!("{} 0 obj", n + 1).as_bytes()));
        }
        assert!(pdf_from_jpeg(b"PNG", (1, 1), (10.0, 10.0)).is_err());
        assert!(pdf_from_jpeg(&jpeg, (1, 1), (f64::NAN, 10.0)).is_err());
    }

    #[test]
    fn pack_entries_keep_only_what_is_used() {
        use crate::core::runtime::design_pack_entry as entry;
        assert_eq!(entry("inter", "package/files/inter-latin-400-normal.woff2").as_deref(), Some("fonts/inter-latin-400-normal.woff2"));
        assert_eq!(entry("inter", "package/files/inter-latin-ext-700-italic.woff2").as_deref(), Some("fonts/inter-latin-ext-700-italic.woff2"));
        assert!(entry("inter", "package/files/inter-cyrillic-400-normal.woff2").is_none());
        assert!(entry("inter", "package/files/inter-latin-300-normal.woff2").is_none());
        assert!(entry("inter", "package/files/inter-latin-400-normal.woff").is_none());
        assert_eq!(entry("lucide", "package/icons/heart.svg").as_deref(), Some("icons/heart.svg"));
        assert_eq!(entry("lucide", "package/tags.json").as_deref(), Some("icons/tags.json"));
        assert_eq!(entry("lucide", "package/LICENSE").as_deref(), Some("licencas/lucide.txt"));
        assert!(entry("lucide", "package/font/lucide.ttf").is_none() && entry("lucide", "package/icons/../x.svg").is_none());
        assert!(entry("lucide", "outro/icons/heart.svg").is_none());
    }

    #[test]
    fn pack_fonts_and_icons() {
        let pack = temp();
        fs::create_dir_all(pack.join("fonts")).unwrap();
        fs::create_dir_all(pack.join("icons")).unwrap();
        for f in ["inter-latin-400-normal.woff2", "inter-latin-ext-400-normal.woff2", "inter-latin-700-normal.woff2", "anton-latin-400-normal.woff2"] {
            fs::write(pack.join("fonts").join(f), b"w").unwrap();
        }
        let fonts = fonts(&pack);
        assert_eq!(fonts.iter().map(|f| f["family"].as_str().unwrap()).collect::<Vec<_>>(), ["Inter", "Anton"]);
        assert_eq!(fonts[0]["faces"][0]["files"].as_array().unwrap().len(), 2);
        assert!(font(&pack, "../x.woff2").is_err() && font(&pack, "inter-latin-400-normal.woff2").is_ok());
        fs::write(pack.join("icons/tags.json"), r#"{"heart":["love","like"],"house":["home"],"heart-crack":["broken"]}"#).unwrap();
        for n in ["heart", "house", "heart-crack"] { fs::write(pack.join("icons").join(format!("{n}.svg")), "<svg/>").unwrap(); }
        let names = |q: &str| icons(&pack, q, 0, 10)["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names("heart"), ["heart", "heart-crack"]);
        assert_eq!(names("home"), ["house"]);
        assert_eq!(names("").len(), 3);
        let page = icons(&pack, "", 1, 1);
        assert_eq!(page["items"][0]["name"], "heart-crack");
        assert_eq!((page["more"].as_bool(), page["total"].as_u64()), (Some(true), Some(3)));
        assert_eq!(icons(&pack, "", 2, 1)["more"], false);
        let _ = fs::remove_dir_all(pack);
    }
}
