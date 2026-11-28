//! Designs sobre documentos: PDFs e imagens da Athena/Iris abertos no editor de designs. O PDF
//! continua o original: o que a pessoa adiciona (texto, formas, imagens, traços) vira uma camada
//! transparente por página, carimbada por cima do conteúdo dele (o texto do PDF segue
//! selecionável). O resultado sai sempre de uma cópia intacta do original (a "base"), então salvar
//! de novo não duplica as marcas e os elementos continuam editáveis no design.
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::io::Write;

fn erro(_: lopdf::Error) -> String { "Não foi possível montar o PDF".into() }

/// Atributo herdável da página (procura nos pais).
fn herdado<'a>(doc: &'a Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut node = Some(page);
    while let Some(id) = node {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(value) = dict.get(key) { return Some(value); }
        node = dict.get(b"Parent").and_then(Object::as_reference).ok();
    }
    None
}

fn numero(o: &Object) -> Option<f64> { o.as_float().ok().map(f64::from).or_else(|| o.as_i64().ok().map(|v| v as f64)) }

fn caixa(doc: &Document, page: ObjectId, key: &[u8]) -> Option<[f64; 4]> {
    let value = herdado(doc, page, key)?;
    let array = match value { Object::Reference(r) => doc.get_object(*r).ok()?.as_array().ok()?, o => o.as_array().ok()? };
    let v: Vec<f64> = array.iter().filter_map(numero).collect();
    (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

/// Área visível da página (CropBox dentro da MediaBox, como o pdf.js) e a rotação (0/90/180/270).
pub fn area(doc: &Document, page: ObjectId) -> ([f64; 4], i64) {
    let media = caixa(doc, page, b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let area = caixa(doc, page, b"CropBox").map(|c| [c[0].max(media[0]), c[1].max(media[1]), c[2].min(media[2]), c[3].min(media[3])])
        .filter(|c| c[2] > c[0] && c[3] > c[1]).unwrap_or(media);
    let rotate = herdado(doc, page, b"Rotate").and_then(|o| o.as_i64().ok()).unwrap_or(0).rem_euclid(360);
    (area, rotate)
}

/// Matriz (`cm`) que leva o quadrado unitário da imagem da camada para a página, como ela aparece
/// na tela (já girada): a camada foi desenhada sobre a página vista pela pessoa.
pub fn matriz(area: [f64; 4], rotate: i64) -> [f64; 6] {
    let [x0, y0, x1, y1] = area;
    let (w, h) = (x1 - x0, y1 - y0);
    match rotate {
        90 => [0.0, h, -w, 0.0, x0 + w, y0],
        180 => [-w, 0.0, 0.0, -h, x0 + w, y0 + h],
        270 => [0.0, -h, w, 0.0, x0, y0 + h],
        _ => [w, 0.0, 0.0, h, x0, y0],
    }
}

/// Camada de uma página: pixels RGBA (largura, altura, bytes).
pub struct Camada { pub largura: u32, pub altura: u32, pub rgba: Vec<u8> }

#[cfg(target_os = "linux")]
pub fn decodificar_png(png: &[u8]) -> Result<Camada, String> {
    use gdk_pixbuf::prelude::PixbufLoaderExt;
    if !png.starts_with(b"\x89PNG") { return Err("Camada inválida".into()); }
    let loader = gdk_pixbuf::PixbufLoader::with_type("png").map_err(|_| "Não foi possível ler a camada")?;
    loader.write(png).map_err(|_| "Não foi possível ler a camada")?;
    loader.close().map_err(|_| "Não foi possível ler a camada")?;
    let pixbuf = loader.pixbuf().ok_or("Não foi possível ler a camada")?;
    let (w, h, stride, n) = (pixbuf.width() as usize, pixbuf.height() as usize, pixbuf.rowstride() as usize, pixbuf.n_channels() as usize);
    let bytes = pixbuf.read_pixel_bytes();
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &bytes[y * stride..];
        for x in 0..w {
            let p = &row[x * n..x * n + n];
            rgba.extend_from_slice(&[p[0], p[1], p[2], if n == 4 { p[3] } else { 255 }]);
        }
    }
    Ok(Camada { largura: w as u32, altura: h as u32, rgba })
}

#[cfg(not(target_os = "linux"))]
pub fn decodificar_png(_: &[u8]) -> Result<Camada, String> { Err("Editar PDFs ainda não funciona neste sistema".into()) }

fn comprimir(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

/// Camada vazia (tudo transparente)?
fn vazia(c: &Camada) -> bool { c.rgba.chunks_exact(4).all(|p| p[3] == 0) }

/// Carimba as camadas (índice da página a partir de 0 → camada) por cima do PDF base.
#[cfg(test)]
pub fn carimbar(base: &[u8], camadas: Vec<(usize, Camada)>) -> Result<Vec<u8>, String> {
    montar(base, None, camadas)
}

/// Uma folha do PDF montado: uma página do original (índice a partir de 0) ou uma página em
/// branco nova (largura e altura em pontos).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Folha { Original(usize), Branca(f64, f64) }

/// Monta o PDF a partir da base: com `folhas`, na ordem pedida (páginas do original tiradas ou
/// repetidas não; páginas em branco novas no meio), e com as camadas (índice da folha → camada).
pub fn montar(base: &[u8], folhas: Option<&[Folha]>, camadas: Vec<(usize, Camada)>) -> Result<Vec<u8>, String> {
    let mut doc = Document::load_mem(base).map_err(|_| "Não foi possível abrir o PDF original (protegido por senha?)".to_string())?;
    if let Some(folhas) = folhas { reorganizar(&mut doc, folhas)?; }
    carimbar_em(&mut doc, camadas)?;
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|_| "Não foi possível gravar o PDF".to_string())?;
    Ok(out)
}

/// Atributos que a página herda dos pais e precisa levar consigo quando a árvore é refeita.
const HERDAVEIS: [&[u8]; 4] = [b"MediaBox", b"CropBox", b"Rotate", b"Resources"];

/// Refaz a árvore de páginas na ordem das folhas (uma lista plana sob a raiz). Sem mudança
/// (todas as páginas do original, na ordem), não mexe em nada.
fn reorganizar(doc: &mut Document, folhas: &[Folha]) -> Result<(), String> {
    let originais: Vec<ObjectId> = doc.get_pages().into_values().collect();
    if folhas.is_empty() { return Err("O PDF precisa de pelo menos uma página".into()); }
    if folhas.len() == originais.len() && folhas.iter().enumerate().all(|(i, f)| *f == Folha::Original(i)) { return Ok(()); }
    let mut usadas = std::collections::HashSet::new();
    for folha in folhas {
        match *folha {
            Folha::Original(i) if i >= originais.len() => return Err(format!("O PDF não tem a página {}", i + 1)),
            Folha::Original(i) if !usadas.insert(i) => return Err("Página repetida na montagem do PDF".into()),
            Folha::Branca(w, h) if !(w.is_finite() && h.is_finite() && (1.0..=14_400.0).contains(&w) && (1.0..=14_400.0).contains(&h)) => return Err("Tamanho de página inválido".into()),
            _ => {}
        }
    }
    let raiz = doc.catalog().map_err(erro)?.get(b"Pages").and_then(Object::as_reference).map_err(erro)?;
    let mut kids = Vec::with_capacity(folhas.len());
    for folha in folhas {
        let id = match *folha {
            Folha::Original(i) => {
                let page = originais[i];
                let herdados: Vec<(&[u8], Object)> = HERDAVEIS.iter().filter(|k| doc.get_dictionary(page).is_ok_and(|d| d.get(k).is_err()))
                    .filter_map(|k| herdado(doc, page, k).cloned().map(|v| (*k, v))).collect();
                let dict = doc.get_dictionary_mut(page).map_err(erro)?;
                for (k, v) in herdados { dict.set(k.to_vec(), v); }
                page
            }
            Folha::Branca(w, h) => {
                let conteudo = doc.add_object(Stream::new(Dictionary::new(), Vec::new()));
                let mut d = Dictionary::new();
                d.set("Type", Object::Name(b"Page".to_vec()));
                d.set("MediaBox", Object::Array(vec![Object::Integer(0), Object::Integer(0), Object::Real(w as f32), Object::Real(h as f32)]));
                d.set("Resources", Object::Dictionary(Dictionary::new()));
                d.set("Contents", Object::Reference(conteudo));
                doc.add_object(d)
            }
        };
        doc.get_dictionary_mut(id).map_err(erro)?.set("Parent", Object::Reference(raiz));
        kids.push(Object::Reference(id));
    }
    let dict = doc.get_dictionary_mut(raiz).map_err(erro)?;
    // As páginas já levam o que herdavam; a raiz não pode passar nada às páginas novas.
    for k in HERDAVEIS { dict.remove(k); }
    dict.set("Count", Object::Integer(kids.len() as i64));
    dict.set("Kids", Object::Array(kids));
    doc.prune_objects();
    Ok(())
}

fn carimbar_em(doc: &mut Document, camadas: Vec<(usize, Camada)>) -> Result<(), String> {
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    for (indice, camada) in camadas {
        let Some(&page) = pages.get(indice) else { return Err(format!("O PDF não tem a página {}", indice + 1)) };
        if vazia(&camada) { continue; }
        let (area, rotate) = area(doc, page);
        let rgb: Vec<u8> = camada.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let alpha: Vec<u8> = camada.rgba.chunks_exact(4).map(|p| p[3]).collect();
        let imagem = |cores: &str, data: Vec<u8>, smask: Option<ObjectId>| {
            let mut d = Dictionary::new();
            d.set("Type", Object::Name(b"XObject".to_vec()));
            d.set("Subtype", Object::Name(b"Image".to_vec()));
            d.set("Width", Object::Integer(i64::from(camada.largura)));
            d.set("Height", Object::Integer(i64::from(camada.altura)));
            d.set("ColorSpace", Object::Name(cores.as_bytes().to_vec()));
            d.set("BitsPerComponent", Object::Integer(8));
            d.set("Filter", Object::Name(b"FlateDecode".to_vec()));
            if let Some(m) = smask { d.set("SMask", Object::Reference(m)); }
            Stream::new(d, comprimir(&data))
        };
        let mascara = doc.add_object(imagem("DeviceGray", alpha, None));
        let figura = doc.add_object(imagem("DeviceRGB", rgb, Some(mascara)));
        let nome = format!("AylaCamada{}", figura.0);
        let m = matriz(area, rotate);
        let desenho = format!("q\n{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} cm\n/{nome} Do\nQ\n", m[0], m[1], m[2], m[3], m[4], m[5]);
        let antes = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
        let depois = doc.add_object(Stream::new(Dictionary::new(), format!("Q\n{desenho}").into_bytes()));
        // Recursos próprios da página (copiando os herdados), com a camada nos XObjects.
        let mut recursos = match herdado(doc, page, b"Resources") {
            Some(Object::Dictionary(d)) => d.clone(),
            Some(Object::Reference(r)) => doc.get_dictionary(*r).cloned().unwrap_or_default(),
            _ => Dictionary::new(),
        };
        let mut xobjects = match recursos.get(b"XObject") {
            Ok(Object::Dictionary(d)) => d.clone(),
            Ok(Object::Reference(r)) => doc.get_dictionary(*r).cloned().unwrap_or_default(),
            _ => Dictionary::new(),
        };
        xobjects.set(nome.as_bytes().to_vec(), Object::Reference(figura));
        recursos.set("XObject", Object::Dictionary(xobjects));
        let dict = doc.get_dictionary_mut(page).map_err(erro)?;
        let mut conteudo: Vec<Object> = match dict.get(b"Contents") {
            Ok(Object::Array(a)) => a.clone(),
            Ok(o @ Object::Reference(_)) => vec![o.clone()],
            _ => Vec::new(),
        };
        // O conteúdo original entre q/Q: o que ele deixar no estado gráfico não afeta a camada.
        conteudo.insert(0, Object::Reference(antes));
        conteudo.push(Object::Reference(depois));
        dict.set("Contents", Object::Array(conteudo));
        dict.set("Resources", Object::Dictionary(recursos));
    }
    Ok(())
}

/// Formato da imagem pelos primeiros bytes ("png", "jpg", "webp").
pub fn formato_imagem(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG") { Some("png") }
    else if bytes.starts_with(&[0xFF, 0xD8]) { Some("jpg") }
    else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" { Some("webp") }
    else { None }
}

/// A extensão do arquivo aceita estes bytes (para gravar a imagem editada no mesmo formato).
pub fn mesmo_formato(extensao: &str, bytes: &[u8]) -> bool {
    let ext = extensao.to_ascii_lowercase();
    match formato_imagem(bytes) {
        Some("png") => ext == "png",
        Some("jpg") => matches!(ext.as_str(), "jpg" | "jpeg" | "jfif"),
        Some("webp") => ext == "webp",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf(paginas: &[(&str, i64)]) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for (media, rotate) in paginas {
            let content = doc.add_object(Stream::new(Dictionary::new(), b"BT /F1 12 Tf 72 720 Td (Ola) Tj ET".to_vec()));
            let mut d = Dictionary::new();
            d.set("Type", Object::Name(b"Page".to_vec()));
            d.set("Parent", Object::Reference(pages_id));
            d.set("MediaBox", Object::Array(media.split(' ').map(|v| Object::Integer(v.parse().unwrap())).collect()));
            d.set("Contents", Object::Reference(content));
            if *rotate != 0 { d.set("Rotate", Object::Integer(*rotate)); }
            kids.push(Object::Reference(doc.add_object(d)));
        }
        let mut pages = Dictionary::new();
        pages.set("Type", Object::Name(b"Pages".to_vec()));
        pages.set("Count", Object::Integer(kids.len() as i64));
        pages.set("Kids", Object::Array(kids));
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog = doc.add_object(catalog);
        doc.trailer.set("Root", Object::Reference(catalog));
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn camada(w: u32, h: u32, marcada: bool) -> Camada {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        if marcada { rgba[0..4].copy_from_slice(&[255, 0, 0, 255]); }
        Camada { largura: w, altura: h, rgba }
    }

    #[test]
    fn matriz_segue_a_rotacao_da_pagina() {
        let a = [0.0, 0.0, 600.0, 800.0];
        // Canto de cima à esquerda da camada (s=0, t=1) cai no canto que a pessoa vê em cima à esquerda.
        let ponto = |m: [f64; 6], s: f64, t: f64| (m[0] * s + m[2] * t + m[4], m[1] * s + m[3] * t + m[5]);
        assert_eq!(ponto(matriz(a, 0), 0.0, 1.0), (0.0, 800.0));
        assert_eq!(ponto(matriz(a, 90), 0.0, 1.0), (0.0, 0.0), "girada 90: em cima à esquerda é a origem");
        assert_eq!(ponto(matriz(a, 180), 0.0, 1.0), (600.0, 0.0));
        assert_eq!(ponto(matriz(a, 270), 0.0, 1.0), (600.0, 800.0));
        assert_eq!(matriz([10.0, 20.0, 110.0, 220.0], 0), [100.0, 0.0, 0.0, 200.0, 10.0, 20.0]);
    }

    #[test]
    fn carimba_so_as_paginas_com_algo_e_mantem_o_texto() {
        let base = pdf(&[("0 0 612 792", 0), ("0 0 612 792", 90), ("0 0 612 792", 0)]);
        let out = carimbar(&base, vec![(0, camada(4, 4, true)), (1, camada(4, 4, true)), (2, camada(4, 4, false))]).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
        assert_eq!(pages.len(), 3);
        let contents = |p: ObjectId| doc.get_dictionary(p).unwrap().get(b"Contents").unwrap().clone();
        assert!(matches!(contents(pages[0]), Object::Array(ref a) if a.len() == 3), "q + original + Q/camada");
        assert!(matches!(contents(pages[2]), Object::Reference(_)), "camada vazia: página intacta");
        // O conteúdo original (com o texto) continua no meio, intacto.
        let Object::Array(a) = contents(pages[0]) else { panic!() };
        let original = doc.get_object(a[1].as_reference().unwrap()).unwrap().as_stream().unwrap();
        assert!(String::from_utf8_lossy(&original.decompressed_content().unwrap_or(original.content.clone())).contains("(Ola) Tj"));
        let xobj = doc.get_dictionary(pages[1]).unwrap().get(b"Resources").unwrap().as_dict().unwrap().get(b"XObject").unwrap().as_dict().unwrap().len();
        assert_eq!(xobj, 1);
        assert!(carimbar(&base, vec![(7, camada(2, 2, true))]).is_err());
        // Salvar de novo a partir da mesma base não acumula camadas.
        let de_novo = carimbar(&base, vec![(0, camada(4, 4, true))]).unwrap();
        let d2 = Document::load_mem(&de_novo).unwrap();
        let p0 = *d2.get_pages().values().next().unwrap();
        assert!(matches!(d2.get_dictionary(p0).unwrap().get(b"Contents").unwrap(), Object::Array(a) if a.len() == 3));
    }

    #[test]
    fn monta_com_paginas_novas_tiradas_e_herdadas() {
        // MediaBox e Rotate só na raiz: as páginas herdam; a página nova não pode herdar o Rotate.
        let mut base_doc = Document::load_mem(&pdf(&[("0 0 612 792", 0), ("0 0 300 400", 0), ("0 0 612 792", 0)])).unwrap();
        let raiz = base_doc.catalog().unwrap().get(b"Pages").unwrap().as_reference().unwrap();
        let p0 = *base_doc.get_pages().values().next().unwrap();
        base_doc.get_dictionary_mut(p0).unwrap().remove(b"MediaBox");
        base_doc.get_dictionary_mut(raiz).unwrap().set("MediaBox", Object::Array(vec![0.into(), 0.into(), 500.into(), 700.into()]));
        base_doc.get_dictionary_mut(raiz).unwrap().set("Rotate", Object::Integer(90));
        let mut base = Vec::new();
        base_doc.save_to(&mut base).unwrap();

        let folhas = [Folha::Original(1), Folha::Branca(200.0, 100.0), Folha::Original(0)];
        let out = montar(&base, Some(&folhas), vec![(1, camada(4, 4, true))]).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
        assert_eq!(pages.len(), 3, "a 3ª página do original saiu e entrou uma em branco");
        assert_eq!(area(&doc, pages[0]).0, [0.0, 0.0, 300.0, 400.0]);
        assert_eq!(area(&doc, pages[1]), ([0.0, 0.0, 200.0, 100.0], 0), "página nova: tamanho pedido, sem herdar o giro");
        assert_eq!(area(&doc, pages[2]), ([0.0, 0.0, 500.0, 700.0], 90), "a página do original leva o que herdava");
        assert!(matches!(doc.get_dictionary(pages[1]).unwrap().get(b"Contents").unwrap(), Object::Array(a) if a.len() == 3), "camada na página nova");

        // Sem mudança na ordem: igual a só carimbar.
        let igual = montar(&base, Some(&[Folha::Original(0), Folha::Original(1), Folha::Original(2)]), vec![]).unwrap();
        assert_eq!(Document::load_mem(&igual).unwrap().get_pages().len(), 3);
        assert!(montar(&base, Some(&[Folha::Original(0), Folha::Original(0)]), vec![]).is_err(), "página repetida");
        assert!(montar(&base, Some(&[Folha::Original(5)]), vec![]).is_err());
        assert!(montar(&base, Some(&[]), vec![]).is_err());
        assert!(montar(&base, Some(&[Folha::Branca(0.0, 10.0)]), vec![]).is_err());
    }

    #[test]
    fn formatos_de_imagem() {
        assert!(mesmo_formato("PNG", b"\x89PNG\r\n"));
        assert!(mesmo_formato("jpeg", &[0xFF, 0xD8, 0xFF]));
        assert!(!mesmo_formato("png", &[0xFF, 0xD8, 0xFF]));
        assert!(mesmo_formato("webp", b"RIFF\0\0\0\0WEBPVP8 "));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn le_a_camada_png() {
        let pixbuf = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 3, 2).unwrap();
        pixbuf.fill(0xff000080);
        let png = pixbuf.save_to_bufferv("png", &[]).unwrap();
        let c = decodificar_png(&png).unwrap();
        assert_eq!((c.largura, c.altura, c.rgba.len()), (3, 2, 24));
        assert_eq!(&c.rgba[0..4], &[255, 0, 0, 128]);
    }
}
