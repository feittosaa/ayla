//! Iris: ampliar imagens com o Real-ESRGAN (componente opcional "upscale", roda na placa de vídeo
//! por Vulkan). O original nunca muda: o resultado é uma cópia ao lado ("foto-ampliada-4x.jpg").
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Maior lado da imagem ampliada.
pub const MAX_SIDE: u32 = 16_384;
/// Os modelos x4plus só montam a imagem certa em 4x: com "-s 2" ou "-s 3" o programa encaixa os
/// blocos fora do lugar. Por isso amplia sempre em 4x e reduz depois para 2x ou 3x.
pub const NATIVE_SCALE: u32 = 4;

/// Modelo do Real-ESRGAN pelo nome mostrado na Iris.
pub fn model_name(kind: &str) -> Option<&'static str> {
    match kind {
        "foto" => Some("realesrgan-x4plus"),
        "anime" => Some("realesrgan-x4plus-anime"),
        _ => None,
    }
}

/// Formato de saída: JPEG e WebP continuam no formato; o resto vira PNG.
pub fn output_extension(source: &Path) -> &'static str {
    match source.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("jpg" | "jpeg") => "jpg",
        Some("webp") => "webp",
        _ => "png",
    }
}

/// O Real-ESRGAN lê JPEG, PNG e WebP; os outros formatos passam por um PNG temporário.
pub fn reads_directly(source: &Path) -> bool {
    matches!(source.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("jpg" | "jpeg" | "png" | "webp"))
}

/// "foto.jpg" → "foto-ampliada-4x.jpg", com " 2", " 3"… se já existir na pasta.
pub fn output_name(original: &str, scale: u32, extension: &str, exists: impl Fn(&str) -> bool) -> String {
    let stem = original.rsplit_once('.').map_or(original, |(stem, _)| stem);
    let base = format!("{stem}-ampliada-{scale}x");
    let mut name = format!("{base}.{extension}");
    let mut n = 2;
    while exists(&name) {
        name = format!("{base} {n}.{extension}");
        n += 1;
    }
    name
}

/// Confere o tamanho final antes de ampliar.
pub fn check_size(width: u32, height: u32, scale: u32) -> Result<(), String> {
    if !(2..=4).contains(&scale) {
        return Err("Escolha 2x, 3x ou 4x".into());
    }
    let limit = MAX_SIDE / NATIVE_SCALE;
    if width.max(height) > limit {
        return Err(format!("O upscale aceita imagens de até {limit} px de lado (esta tem {width}×{height})."));
    }
    Ok(())
}

/// "53.25%" na saída do Real-ESRGAN.
pub fn progress_of(line: &str) -> Option<f64> {
    line.trim().strip_suffix('%')?.trim().parse::<f64>().ok().filter(|p| (0.0..=100.0).contains(p))
}

/// Upscayl já instalado pelo usuário: a Ayla só chama o programa dele (`upscayl-bin`, AGPL) como
/// processo separado, sem janela; nada do Upscayl é copiado ou distribuído com a Ayla.
pub struct Upscayl {
    pub binary: PathBuf,
    pub models: PathBuf,
}

/// Onde os pacotes do Upscayl para Linux (.rpm/.deb) instalam o programa e os modelos.
pub fn find_upscayl() -> Option<Upscayl> {
    ["/opt/Upscayl/resources", "/opt/upscayl/resources", "/usr/lib/upscayl/resources", "/usr/lib/upscayl"].iter().map(Path::new).find_map(|dir| {
        let binary = dir.join("bin").join("upscayl-bin");
        let models = dir.join("models");
        (binary.is_file() && models.is_dir()).then_some(Upscayl { binary, models })
    })
}

/// Modelos 4x do Upscayl (pares .param + .bin), em ordem alfabética.
pub fn upscayl_models(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|entry| {
        let name = entry.file_name().to_string_lossy().strip_suffix(".param")?.to_string();
        (name.ends_with("-4x") && dir.join(format!("{name}.bin")).is_file()).then_some(name)
    }).collect();
    names.sort();
    names
}

/// Nome amigável dos modelos conhecidos do Upscayl.
pub fn upscayl_label(model: &str) -> String {
    match model {
        "upscayl-standard-4x" => "Upscayl padrão".into(),
        "upscayl-lite-4x" => "Upscayl leve".into(),
        "digital-art-4x" => "Arte digital".into(),
        "high-fidelity-4x" => "Alta fidelidade".into(),
        "remacri-4x" => "Remacri".into(),
        "ultramix-balanced-4x" => "Ultramix equilibrado".into(),
        "ultrasharp-4x" => "UltraSharp".into(),
        other => other.trim_end_matches("-4x").replace('-', " "),
    }
}

/// Roda o Real-ESRGAN (ou o `upscayl-bin`, que aceita as mesmas opções) e informa o progresso (0 a 100).
/// `models`: pasta dos modelos. Interrompido, encerra o processo.
pub fn run(binary: &Path, models: &Path, input: &Path, output: &Path, model: &str, cancel: &CancellationToken, mut progress: impl FnMut(f64)) -> Result<(), String> {
    let mut child = Command::new(binary)
        .args(["-i".as_ref(), input.as_os_str(), "-o".as_ref(), output.as_os_str(), "-n".as_ref(), model.as_ref(), "-s".as_ref(), NATIVE_SCALE.to_string().as_ref(), "-m".as_ref(), models.as_os_str()])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped())
        .spawn().map_err(|_| "Não foi possível iniciar o upscale")?;
    let stderr = child.stderr.take().ok_or("Não foi possível ler o progresso")?;
    let (sender, lines) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        let mut last = Vec::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            last.push(line.clone());
            if last.len() > 8 { last.remove(0); }
            let _ = sender.send(line);
        }
        last
    });
    loop {
        match lines.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => { if let Some(p) = progress_of(&line) { progress(p); } }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(output);
            return Err("Upscale interrompido".into());
        }
    }
    let status = child.wait().map_err(|_| "O upscale parou")?;
    let tail = reader.join().unwrap_or_default();
    if !status.success() || !output.is_file() {
        let _ = std::fs::remove_file(output);
        let vulkan = tail.iter().any(|l| l.to_lowercase().contains("vulkan") || l.contains("invalid gpu"));
        return Err(if vulkan { "O upscale não encontrou uma placa de vídeo com Vulkan.".into() } else { "O upscale não conseguiu processar esta imagem.".into() });
    }
    progress(100.0);
    Ok(())
}

/// Converte para PNG (formatos que o Real-ESRGAN não lê), respeitando a orientação da foto.
#[cfg(target_os = "linux")]
pub fn to_png(source: &Path, target: &Path) -> Result<(), String> {
    let image = gdk_pixbuf::Pixbuf::from_file(source).map_err(|_| "Não foi possível ler a imagem")?;
    let image = image.apply_embedded_orientation().unwrap_or(image);
    image.savev(target, "png", &[]).map_err(|_| "Não foi possível preparar a imagem".into())
}

/// Reduz a ampliação em 4x para o tamanho pedido e grava no formato de `target`.
#[cfg(target_os = "linux")]
pub fn shrink(source: &Path, target: &Path, width: u32, height: u32) -> Result<(), String> {
    let image = gdk_pixbuf::Pixbuf::from_file(source).map_err(|_| "Não foi possível ler o upscale")?;
    let image = image.scale_simple(width as i32, height as i32, gdk_pixbuf::InterpType::Hyper).ok_or("Não foi possível reduzir o upscale")?;
    let (format, options): (&str, &[(&str, &str)]) = match output_extension(target) { "jpg" => ("jpeg", &[("quality", "95")]), "webp" => ("webp", &[("quality", "95")]), _ => ("png", &[]) };
    image.savev(target, format, options).map_err(|_| "Não foi possível gravar a imagem do upscale".into())
}

/// O gdk-pixbuf deste computador grava WebP? (sem o módulo, 2x e 3x de WebP saem em PNG)
#[cfg(target_os = "linux")]
pub fn writes_webp() -> bool {
    gdk_pixbuf::Pixbuf::formats().iter().any(|f| f.name().is_some_and(|n| n == "webp") && f.is_writable())
}

/// Largura e altura da imagem (sem decodificar tudo).
#[cfg(target_os = "linux")]
pub fn dimensions(source: &Path) -> Option<(u32, u32)> {
    gdk_pixbuf::Pixbuf::file_info(source).map(|(_, w, h)| (w.max(0) as u32, h.max(0) as u32))
}

/// Pasta temporária dentro da pasta de destino (oculta: a Iris não lista enquanto amplia).
pub fn working_file(dir: &Path, extension: &str) -> PathBuf {
    dir.join(format!(".ayla-ampliando-{}.{extension}", super::store::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nomes_formatos_e_limites() {
        assert_eq!(model_name("foto"), Some("realesrgan-x4plus"));
        assert_eq!(model_name("anime"), Some("realesrgan-x4plus-anime"));
        assert_eq!(model_name("outro"), None);
        assert_eq!(output_extension(Path::new("a/Foto.JPEG")), "jpg");
        assert_eq!(output_extension(Path::new("a.webp")), "webp");
        assert_eq!(output_extension(Path::new("a.bmp")), "png");
        assert!(reads_directly(Path::new("x.PNG")) && !reads_directly(Path::new("x.avif")));
        let taken = ["foto-ampliada-4x.jpg", "foto-ampliada-4x 2.jpg"];
        assert_eq!(output_name("foto.jpg", 4, "jpg", |n| taken.contains(&n)), "foto-ampliada-4x 3.jpg");
        assert_eq!(output_name("sem extensão", 2, "png", |_| false), "sem extensão-ampliada-2x.png");
        assert!(check_size(4096, 2160, 4).is_ok());
        assert!(check_size(4097, 100, 4).is_err(), "passa de 16384 px");
        assert!(check_size(4097, 100, 2).is_err(), "a ampliação intermediária é sempre 4x");
        assert!(check_size(100, 100, 5).is_err());
        assert_eq!(progress_of("53.25%"), Some(53.25));
        assert_eq!(progress_of("[0 AMD Radeon]"), None);
        assert_eq!(upscayl_label("ultrasharp-4x"), "UltraSharp");
        assert_eq!(upscayl_label("meu-modelo-4x"), "meu modelo");
        let dir = std::env::temp_dir().join(format!("ayla-upscayl-{}", super::super::store::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["b-4x.param", "b-4x.bin", "a-4x.param", "a-4x.bin", "sem-bin-4x.param", "dois-2x.param", "dois-2x.bin"] { std::fs::write(dir.join(f), "").unwrap(); }
        assert_eq!(upscayl_models(&dir), ["a-4x", "b-4x"], "só pares completos e 4x");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Com o Upscayl instalado no sistema: amplia sem embaralhar e reduz de volta perto do original.
    #[test]
    #[ignore = "precisa do Upscayl e de Vulkan"]
    fn amplia_com_upscayl() {
        let found = find_upscayl().expect("Upscayl instalado");
        let model = upscayl_models(&found.models).into_iter().next().expect("um modelo 4x");
        let dir = std::env::temp_dir().join(format!("ayla-upscayl-real-{}", super::super::store::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (600usize, 400usize);
        let pixels: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x * 255 / w) as u8, (y * 255 / h) as u8, 128])).collect();
        let original = gdk_pixbuf::Pixbuf::from_bytes(&gdk_pixbuf::glib::Bytes::from(&pixels), gdk_pixbuf::Colorspace::Rgb, false, 8, w as i32, h as i32, (w * 3) as i32);
        original.savev(dir.join("in.png"), "png", &[]).unwrap();
        let mut seen = Vec::new();
        run(&found.binary, &found.models, &dir.join("in.png"), &dir.join("4x.png"), &model, &CancellationToken::new(), |p| seen.push(p)).unwrap();
        assert_eq!(dimensions(&dir.join("4x.png")), Some((2400, 1600)));
        assert!(seen.len() > 2, "progresso: {seen:?}");
        shrink(&dir.join("4x.png"), &dir.join("volta.png"), w as u32, h as u32).unwrap();
        let back = gdk_pixbuf::Pixbuf::from_file(dir.join("volta.png")).unwrap();
        let (a, b, stride) = (original.read_pixel_bytes(), back.read_pixel_bytes(), back.rowstride() as usize);
        let diff: u64 = (0..h).flat_map(|y| (0..w * 3).map(move |x| (y, x))).map(|(y, x)| (a[y * w * 3 + x] as i64 - b[y * stride + x] as i64).unsigned_abs()).sum();
        assert!((diff as f64 / (w * h * 3) as f64) < 12.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Com o ampliador instalado (`AYLA_UPSCALE_BIN`), amplia uma imagem de verdade e interrompe outra.
    #[test]
    #[ignore = "precisa do Real-ESRGAN e de Vulkan"]
    fn amplia_de_verdade() {
        let binary = PathBuf::from(std::env::var("AYLA_UPSCALE_BIN").expect("AYLA_UPSCALE_BIN"));
        let dir = std::env::temp_dir().join(format!("ayla-upscale-{}", super::super::store::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.png");
        let image = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 64, 48).unwrap();
        image.fill(0x3366ccff);
        image.savev(&input, "png", &[]).unwrap();
        let output = dir.join("out.png");
        let mut seen = Vec::new();
        let models = binary.parent().unwrap().join("models");
        run(&binary, &models, &input, &output, "realesrgan-x4plus", &CancellationToken::new(), |p| seen.push(p)).unwrap();
        assert_eq!(dimensions(&output), Some((256, 192)));
        shrink(&output, &dir.join("2x.jpg"), 128, 96).unwrap();
        assert_eq!(dimensions(&dir.join("2x.jpg")), Some((128, 96)));
        assert_eq!(seen.last(), Some(&100.0));
        // Imagem maior que um bloco do Real-ESRGAN: reduzida de volta, tem de lembrar o original
        // (com "-s 2" os blocos saíam embaralhados).
        let (w, h) = (700usize, 500usize);
        let mut pixels = Vec::with_capacity(w * h * 3);
        for y in 0..h { for x in 0..w { pixels.extend([(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) % 256) as u8]); } }
        let original = gdk_pixbuf::Pixbuf::from_bytes(&gdk_pixbuf::glib::Bytes::from(&pixels), gdk_pixbuf::Colorspace::Rgb, false, 8, w as i32, h as i32, (w * 3) as i32);
        let big = dir.join("grande.png");
        original.savev(&big, "png", &[]).unwrap();
        run(&binary, &models, &big, &dir.join("grande-4x.png"), "realesrgan-x4plus", &CancellationToken::new(), |_| {}).unwrap();
        shrink(&dir.join("grande-4x.png"), &dir.join("volta.png"), w as u32, h as u32).unwrap();
        let back = gdk_pixbuf::Pixbuf::from_file(dir.join("volta.png")).unwrap();
        let (a, b) = (original.read_pixel_bytes(), back.read_pixel_bytes());
        let stride = back.rowstride() as usize;
        let mut diff = 0u64;
        for y in 0..h { for x in 0..w * 3 { diff += (a[y * w * 3 + x] as i64 - b[y * stride + x] as i64).unsigned_abs(); } }
        let mean = diff as f64 / (w * h * 3) as f64;
        assert!(mean < 12.0, "ampliação embaralhada (diferença média {mean:.1})");
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(run(&binary, &models, &input, &dir.join("parado.png"), "realesrgan-x4plus", &cancel, |_| {}).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
