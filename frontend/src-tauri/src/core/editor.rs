//! Iris: mini editor de imagens (gdk-pixbuf, sem download) e de vídeos (FFmpeg gerenciado, download
//! opcional). A ordem é a mesma da prévia na interface: girar → espelhar → recortar (frações do quadro
//! já girado) → redimensionar → brilho, contraste e saturação (as fórmulas dos filtros CSS, para o
//! arquivo sair igual ao que se viu). Por padrão sai uma cópia ao lado; substituir é opcional.
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Maior lado de uma imagem editada.
pub const MAX_SIDE: u32 = 16_384;

/// Recorte em frações (0 a 1) do quadro já girado.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Crop {
    fn valid(&self) -> bool {
        let ok = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        ok(self.x) && ok(self.y) && ok(self.width) && ok(self.height) && self.width > 0.0 && self.height > 0.0
            && self.x + self.width <= 1.0 + 1e-6 && self.y + self.height <= 1.0 + 1e-6
    }
    /// O recorte inteiro não muda nada.
    fn is_full(&self) -> bool {
        self.x <= 1e-6 && self.y <= 1e-6 && self.width >= 1.0 - 1e-6 && self.height >= 1.0 - 1e-6
    }
    /// Em pixels (x, y, largura, altura), sempre dentro da imagem e com pelo menos 1 px.
    pub fn pixels(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let x = ((self.x * width as f64).round() as u32).min(width.saturating_sub(1));
        let y = ((self.y * height as f64).round() as u32).min(height.saturating_sub(1));
        let w = ((self.width * width as f64).round() as u32).clamp(1, width - x);
        let h = ((self.height * height as f64).round() as u32).clamp(1, height - y);
        (x, y, w, h)
    }
}

fn one() -> f64 { 1.0 }

#[derive(Deserialize, Clone, Debug)]
pub struct ImageEdit {
    /// Graus no sentido horário: 0, 90, 180 ou 270.
    #[serde(default)]
    pub rotate: u32,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
    #[serde(default)]
    pub crop: Option<Crop>,
    /// Tamanho final (depois do recorte); sem ele, fica o tamanho do recorte.
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Fatores como nos filtros CSS: 1 = sem mudança, de 0 a 2.
    #[serde(default = "one")]
    pub brightness: f64,
    #[serde(default = "one")]
    pub contrast: f64,
    #[serde(default = "one")]
    pub saturation: f64,
}

impl ImageEdit {
    pub fn check(&self) -> Result<(), String> {
        if ![0, 90, 180, 270].contains(&self.rotate) {
            return Err("Rotação inválida".into());
        }
        if self.crop.is_some_and(|c| !c.valid()) {
            return Err("Recorte inválido".into());
        }
        if [self.brightness, self.contrast, self.saturation].iter().any(|v| !v.is_finite() || !(0.0..=2.0).contains(v)) {
            return Err("Ajuste inválido".into());
        }
        match (self.width, self.height) {
            (None, None) => Ok(()),
            (Some(w), Some(h)) if (1..=MAX_SIDE).contains(&w) && (1..=MAX_SIDE).contains(&h) => Ok(()),
            (Some(_), Some(_)) => Err(format!("O tamanho vai de 1 a {MAX_SIDE} px de lado")),
            _ => Err("Informe largura e altura".into()),
        }
    }
}

/// Tabela de brilho e contraste (CSS `brightness()` e depois `contrast()`, cada passo limitado a 0–255).
pub fn tone_table(brightness: f64, contrast: f64) -> [u8; 256] {
    let mut table = [0u8; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let v = (i as f64 / 255.0 * brightness).clamp(0.0, 1.0);
        let v = ((v - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
        *slot = (v * 255.0).round() as u8;
    }
    table
}

/// CSS `saturate()` (matriz da especificação de Filter Effects).
pub fn saturate(rgb: [u8; 3], s: f64) -> [u8; 3] {
    let [r, g, b] = rgb.map(f64::from);
    let out = [
        (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * b,
    ];
    out.map(|v| v.round().clamp(0.0, 255.0) as u8)
}

/// Aplica brilho, contraste e saturação em pixels RGB ou RGBA (alfa intacto).
pub fn adjust(pixels: &mut [u8], width: usize, height: usize, stride: usize, channels: usize, edit: &ImageEdit) {
    let table = tone_table(edit.brightness, edit.contrast);
    let tone = edit.brightness != 1.0 || edit.contrast != 1.0;
    let color = edit.saturation != 1.0;
    for y in 0..height {
        let row = &mut pixels[y * stride..];
        for x in 0..width {
            let p = &mut row[x * channels..x * channels + 3];
            if tone { for v in p.iter_mut() { *v = table[*v as usize]; } }
            if color { let s = saturate([p[0], p[1], p[2]], edit.saturation); p.copy_from_slice(&s); }
        }
    }
}

/// Formato de saída da imagem: JPEG, PNG e WebP (se gravável) continuam no formato; o resto vira PNG.
#[cfg(target_os = "linux")]
pub fn image_extension(source: &Path) -> &'static str {
    match super::upscale::output_extension(source) {
        "webp" if !super::upscale::writes_webp() => "png",
        other => other,
    }
}

/// Dá para substituir o original sem mudar o formato?
#[cfg(target_os = "linux")]
pub fn image_replaceable(source: &Path) -> bool {
    let ext = source.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("jpg" | "jpeg" | "png")) || (ext.as_deref() == Some("webp") && super::upscale::writes_webp())
}

/// Edita a imagem e grava em `target` (formato pela extensão).
#[cfg(target_os = "linux")]
pub fn edit_image(source: &Path, target: &Path, edit: &ImageEdit) -> Result<(u32, u32), String> {
    use gdk_pixbuf::{Pixbuf, PixbufRotation};
    edit.check()?;
    let image = Pixbuf::from_file(source).map_err(|_| "Não foi possível ler a imagem")?;
    let mut image = image.apply_embedded_orientation().unwrap_or(image);
    let rotation = match edit.rotate { 90 => Some(PixbufRotation::Clockwise), 180 => Some(PixbufRotation::Upsidedown), 270 => Some(PixbufRotation::Counterclockwise), _ => None };
    if let Some(rotation) = rotation { image = image.rotate_simple(rotation).ok_or("Não foi possível girar a imagem")?; }
    if edit.flip_h { image = image.flip(true).ok_or("Não foi possível espelhar a imagem")?; }
    if edit.flip_v { image = image.flip(false).ok_or("Não foi possível espelhar a imagem")?; }
    if let Some(crop) = edit.crop.filter(|c| !c.is_full()) {
        let (x, y, w, h) = crop.pixels(image.width() as u32, image.height() as u32);
        image = image.new_subpixbuf(x as i32, y as i32, w as i32, h as i32).copy().ok_or("Não foi possível recortar a imagem")?;
    }
    if let (Some(w), Some(h)) = (edit.width, edit.height) {
        if (w as i32, h as i32) != (image.width(), image.height()) {
            image = image.scale_simple(w as i32, h as i32, gdk_pixbuf::InterpType::Hyper).ok_or("Não foi possível redimensionar a imagem")?;
        }
    }
    if edit.brightness != 1.0 || edit.contrast != 1.0 || edit.saturation != 1.0 {
        let (width, height, stride, channels) = (image.width() as usize, image.height() as usize, image.rowstride() as usize, image.n_channels() as usize);
        let mut pixels = image.read_pixel_bytes().to_vec();
        adjust(&mut pixels, width, height, stride, channels, edit);
        image = Pixbuf::from_bytes(&gdk_pixbuf::glib::Bytes::from_owned(pixels), image.colorspace(), image.has_alpha(), image.bits_per_sample(), width as i32, height as i32, stride as i32);
    }
    let (format, options): (&str, &[(&str, &str)]) = match super::upscale::output_extension(target) {
        "jpg" => ("jpeg", &[("quality", "95")]),
        "webp" => ("webp", &[("quality", "95")]),
        _ => ("png", &[]),
    };
    // JPEG não tem transparência.
    if format == "jpeg" && image.has_alpha() {
        let flat = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, image.width(), image.height()).ok_or("Sem memória para a imagem")?;
        flat.fill(0xffffffff);
        image.composite(&flat, 0, 0, image.width(), image.height(), 0.0, 0.0, 1.0, 1.0, gdk_pixbuf::InterpType::Nearest, 255);
        image = flat;
    }
    image.savev(target, format, options).map_err(|_| "Não foi possível gravar a imagem editada")?;
    Ok((image.width() as u32, image.height() as u32))
}

#[derive(Deserialize, Clone, Debug)]
pub struct VideoEdit {
    /// Início e fim do trecho, em segundos.
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: Option<f64>,
    /// Duração do vídeo (só para o progresso).
    pub duration: f64,
    #[serde(default)]
    pub rotate: u32,
    #[serde(default)]
    pub crop: Option<Crop>,
    #[serde(default)]
    pub mute: bool,
}

impl VideoEdit {
    pub fn check(&self) -> Result<(), String> {
        if ![0, 90, 180, 270].contains(&self.rotate) {
            return Err("Rotação inválida".into());
        }
        if self.crop.is_some_and(|c| !c.valid()) {
            return Err("Recorte inválido".into());
        }
        if !self.start.is_finite() || self.start < 0.0 || !self.duration.is_finite() || self.duration <= 0.0 {
            return Err("Trecho inválido".into());
        }
        if self.end.is_some_and(|end| !end.is_finite() || end <= self.start + 0.05) {
            return Err("O fim do trecho precisa vir depois do início".into());
        }
        Ok(())
    }
    /// Segundos que sairão no vídeo final.
    pub fn length(&self) -> f64 {
        (self.end.unwrap_or(self.duration).min(self.duration) - self.start).max(0.05)
    }
}

/// Só MP4 é substituído no lugar (a saída é sempre MP4).
pub fn video_replaceable(source: &Path) -> bool {
    matches!(source.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("mp4" | "m4v"))
}

/// Filtros do FFmpeg na ordem da prévia; termina com lados pares (exigência do H.264 em yuv420p).
pub fn video_filters(edit: &VideoEdit) -> String {
    let mut filters: Vec<String> = Vec::new();
    match edit.rotate {
        90 => filters.push("transpose=clock".into()),
        180 => filters.push("hflip,vflip".into()),
        270 => filters.push("transpose=cclock".into()),
        _ => {}
    }
    if let Some(c) = edit.crop.filter(|c| !c.is_full()) {
        filters.push(format!("crop=trunc(iw*{:.6}/2)*2:trunc(ih*{:.6}/2)*2:trunc(iw*{:.6}):trunc(ih*{:.6})", c.width, c.height, c.x, c.y));
    }
    filters.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".into());
    filters.join(",")
}

/// Argumentos do FFmpeg: trecho, filtros, H.264 + AAC (ou sem som) em MP4, progresso na saída padrão.
pub fn video_args(edit: &VideoEdit, input: &Path, output: &Path) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = ["-nostdin", "-hide_banner", "-loglevel", "error", "-y"].map(Into::into).into();
    if edit.start > 0.0 { args.extend(["-ss".into(), format!("{:.3}", edit.start).into()]); }
    args.extend(["-i".into(), input.as_os_str().to_owned()]);
    if edit.end.is_some() { args.extend(["-t".into(), format!("{:.3}", edit.length()).into()]); }
    args.extend(["-map", "0:v:0"].map(Into::into));
    if !edit.mute { args.extend(["-map", "0:a:0?"].map(Into::into)); }
    args.extend(["-vf".into(), video_filters(edit).into()]);
    args.extend(["-c:v", "libx264", "-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p"].map(Into::into));
    if edit.mute { args.push("-an".into()); } else { args.extend(["-c:a", "aac", "-b:a", "192k"].map(Into::into)); }
    args.extend(["-movflags", "+faststart", "-f", "mp4", "-progress", "pipe:1", "-nostats"].map(Into::into));
    args.push(output.as_os_str().to_owned());
    args
}

/// Segundos já gravados ("out_time_us=1234567" no `-progress`).
pub fn progress_seconds(line: &str) -> Option<f64> {
    let value = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms="))?;
    value.trim().parse::<i64>().ok().filter(|v| *v >= 0).map(|v| v as f64 / 1_000_000.0)
}

/// Roda o FFmpeg com progresso (0 a 100); interrompido, encerra o processo e apaga a saída.
pub fn run_video(binary: &Path, edit: &VideoEdit, input: &Path, output: &Path, cancel: &CancellationToken, mut progress: impl FnMut(f64)) -> Result<(), String> {
    edit.check()?;
    let mut child = Command::new(binary).args(video_args(edit, input, output))
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|_| "Não foi possível iniciar o FFmpeg")?;
    let stdout = child.stdout.take().ok_or("Não foi possível ler o progresso")?;
    let stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr { let _ = std::io::Read::read_to_string(&mut std::io::Read::take(pipe, 64 * 1024), &mut text); }
        text
    });
    let (sender, lines) = mpsc::channel::<String>();
    std::thread::spawn(move || { for line in BufReader::new(stdout).lines().map_while(Result::ok) { if sender.send(line).is_err() { break; } } });
    let total = edit.length();
    loop {
        match lines.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => { if let Some(done) = progress_seconds(&line) { progress((done / total * 100.0).clamp(0.0, 99.0)); } }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(output);
            return Err("Edição interrompida".into());
        }
    }
    let status = child.wait().map_err(|_| "O FFmpeg parou")?;
    let _ = errors.join();
    if !status.success() || !output.is_file() {
        let _ = std::fs::remove_file(output);
        return Err("O FFmpeg não conseguiu editar este vídeo.".into());
    }
    progress(100.0);
    Ok(())
}

/// "foto.jpg" → "foto-editada.jpg", com " 2", " 3"… se já existir.
pub fn copy_name(original: &str, extension: &str, exists: impl Fn(&str) -> bool) -> String {
    let stem = original.rsplit_once('.').map_or(original, |(stem, _)| stem);
    let mut name = format!("{stem}-editada.{extension}");
    let mut n = 2;
    while exists(&name) {
        name = format!("{stem}-editada {n}.{extension}");
        n += 1;
    }
    name
}

/// Arquivo oculto de trabalho na pasta de destino (a Iris não o lista).
pub fn working_file(dir: &Path, extension: &str) -> PathBuf {
    dir.join(format!(".ayla-editando-{}.{extension}", super::store::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> ImageEdit {
        ImageEdit { rotate: 0, flip_h: false, flip_v: false, crop: None, width: None, height: None, brightness: 1.0, contrast: 1.0, saturation: 1.0 }
    }

    #[test]
    fn recorte_ajustes_e_validacao() {
        let crop = Crop { x: 0.25, y: 0.5, width: 0.5, height: 0.5 };
        assert_eq!(crop.pixels(200, 100), (50, 50, 100, 50));
        // Nunca sai da imagem, mesmo com arredondamento.
        assert_eq!(Crop { x: 0.999, y: 0.0, width: 0.001, height: 1.0 }.pixels(10, 10), (9, 0, 1, 10));
        assert!(!Crop { x: 0.6, y: 0.0, width: 0.6, height: 1.0 }.valid());
        assert!(ImageEdit { rotate: 45, ..plain() }.check().is_err());
        assert!(ImageEdit { brightness: 3.0, ..plain() }.check().is_err());
        assert!(ImageEdit { width: Some(10), ..plain() }.check().is_err(), "largura sem altura");
        assert!(ImageEdit { width: Some(20_000), height: Some(10), ..plain() }.check().is_err());
        // Iguais aos filtros CSS: brilho 1,5 + contraste 1 dobra 100 → 150; contraste 0 deixa tudo cinza.
        assert_eq!(tone_table(1.5, 1.0)[100], 150);
        assert_eq!(tone_table(1.0, 0.0)[0], 128);
        assert_eq!(tone_table(1.0, 1.0)[77], 77);
        assert_eq!(saturate([200, 40, 90], 1.0), [200, 40, 90]);
        let gray = saturate([200, 40, 90], 0.0);
        assert!(gray[0] == gray[1] && gray[1] == gray[2], "saturação 0 é cinza");
        let mut rgba = vec![100, 100, 100, 7, 0, 0, 0, 0];
        adjust(&mut rgba, 1, 1, 8, 4, &ImageEdit { brightness: 2.0, ..plain() });
        assert_eq!(rgba[..4], [200, 200, 200, 7], "alfa intacto");
        assert_eq!(copy_name("foto.jpg", "jpg", |n| n == "foto-editada.jpg"), "foto-editada 2.jpg");
    }

    #[test]
    fn argumentos_do_video() {
        let edit = VideoEdit { start: 2.0, end: Some(7.5), duration: 10.0, rotate: 90, crop: Some(Crop { x: 0.1, y: 0.0, width: 0.5, height: 1.0 }), mute: true };
        assert!(edit.check().is_ok());
        assert_eq!(edit.length(), 5.5);
        let args: Vec<String> = video_args(&edit, Path::new("in.mov"), Path::new("out.mp4")).iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let joined = args.join(" ");
        assert!(joined.contains("-ss 2.000 -i in.mov -t 5.500"), "{joined}");
        assert!(joined.contains("transpose=clock,crop=trunc(iw*0.500000/2)*2:trunc(ih*1.000000/2)*2:trunc(iw*0.100000):trunc(ih*0.000000),scale="), "{joined}");
        assert!(args.contains(&"-an".into()) && !joined.contains("0:a:0"));
        assert_eq!(args.last().map(String::as_str), Some("out.mp4"));
        let whole = VideoEdit { start: 0.0, end: None, duration: 10.0, rotate: 0, crop: None, mute: false };
        let joined = video_args(&whole, Path::new("a.mp4"), Path::new("b.mp4")).iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        assert!(!joined.contains("-ss") && !joined.contains("-t ") && joined.contains("-map 0:a:0? ") && joined.contains("-c:a aac"), "{joined}");
        assert!(VideoEdit { end: Some(1.0), start: 1.0, ..whole.clone() }.check().is_err());
        assert_eq!(progress_seconds("out_time_us=2500000"), Some(2.5));
        assert_eq!(progress_seconds("out_time_us=N/A"), None);
        assert!(video_replaceable(Path::new("x.MP4")) && !video_replaceable(Path::new("x.webm")));
    }

    #[test]
    fn edita_imagem_de_verdade() {
        let dir = std::env::temp_dir().join(format!("ayla-editor-{}", super::super::store::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 40×20: metade esquerda vermelha, direita azul.
        let image = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 40, 20).unwrap();
        image.fill(0x0000ffff);
        image.new_subpixbuf(0, 0, 20, 20).fill(0xff0000ff);
        let source = dir.join("a.png");
        image.savev(&source, "png", &[]).unwrap();
        let pixel = |path: &Path, x: i32, y: i32| {
            let p = gdk_pixbuf::Pixbuf::from_file(path).unwrap();
            let bytes = p.read_pixel_bytes();
            let i = (y * p.rowstride() + x * p.n_channels()) as usize;
            [bytes[i], bytes[i + 1], bytes[i + 2]]
        };
        // Girar 90° (fica 20×40, vermelho em cima), recortar a metade de cima e ampliar para 40×40.
        let out = dir.join("b.png");
        let size = edit_image(&source, &out, &ImageEdit { rotate: 90, crop: Some(Crop { x: 0.0, y: 0.0, width: 1.0, height: 0.5 }), width: Some(40), height: Some(40), ..plain() }).unwrap();
        assert_eq!(size, (40, 40));
        assert_eq!(pixel(&out, 20, 20), [255, 0, 0]);
        // Espelhar na horizontal: o azul vai para a esquerda; saturação 0 deixa cinza; JPEG grava.
        let out = dir.join("c.jpg");
        edit_image(&source, &out, &ImageEdit { flip_h: true, saturation: 0.0, ..plain() }).unwrap();
        let [r, g, b] = pixel(&out, 5, 10);
        assert!(r.abs_diff(g) < 6 && g.abs_diff(b) < 6, "cinza: {r} {g} {b}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Com o FFmpeg gerenciado (`AYLA_FFMPEG_BIN`): corta, gira, recorta, tira o som e interrompe.
    #[test]
    #[ignore = "precisa do FFmpeg"]
    fn edita_video_de_verdade() {
        let ffmpeg = PathBuf::from(std::env::var("AYLA_FFMPEG_BIN").expect("AYLA_FFMPEG_BIN"));
        let dir = std::env::temp_dir().join(format!("ayla-editor-video-{}", super::super::store::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("in.mkv");
        let made = Command::new(&ffmpeg).args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=321x241:rate=25:duration=4", "-f", "lavfi", "-i", "sine=duration=4", "-c:v", "libx264", "-c:a", "aac", "-shortest"]).arg(&source).status().unwrap();
        assert!(made.success());
        let output = dir.join("out.mp4");
        let edit = VideoEdit { start: 1.0, end: Some(3.0), duration: 4.0, rotate: 90, crop: Some(Crop { x: 0.0, y: 0.0, width: 1.0, height: 0.5 }), mute: true };
        let mut seen = Vec::new();
        run_video(&ffmpeg, &edit, &source, &output, &CancellationToken::new(), |p| seen.push(p)).unwrap();
        assert_eq!(seen.last(), Some(&100.0));
        let probe = Command::new(&ffmpeg).args(["-hide_banner", "-i"]).arg(&output).output().unwrap();
        let info = String::from_utf8_lossy(&probe.stderr);
        // 321×241 girado → 241×321; metade de cima → 240×160 (lados pares).
        assert!(info.contains("240x160"), "{info}");
        assert!(!info.contains("Audio:"), "sem som: {info}");
        let seconds = super::super::files::parse_duration(&info).unwrap();
        assert!((seconds - 2.0).abs() < 0.2, "duração {seconds}");
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(run_video(&ffmpeg, &edit, &source, &dir.join("parado.mp4"), &cancel, |_| {}).is_err());
        assert!(!dir.join("parado.mp4").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
