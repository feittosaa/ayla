"""Instalação por usuário. A opção QuickShell se aplica ao layout illogical-impulse."""
import argparse
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime

parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path)
parser.add_argument("icon", type=Path)
parser.add_argument("--quickshell", action="store_true")
args = parser.parse_args()
data = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
config = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
bindir = Path(os.environ.get("XDG_BIN_HOME") or Path.home() / ".local/bin")
exe = bindir / "ayla"
icon = data / "icons/hicolor/512x512/apps/dev.feit.ayla.png"

def write(path, content, mode):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as tmp:
        try:
            if isinstance(content, Path):
                with content.open("rb") as source:
                    shutil.copyfileobj(source, tmp)
            else:
                tmp.write(content.encode())
            tmp.flush()
            os.fchmod(tmp.fileno(), mode)
            os.replace(tmp.name, path)
        finally:
            Path(tmp.name).unlink(missing_ok=True)

def exec_arg(path):
    # Exec tem duas camadas de escape (desktop entry e argumento entre aspas).
    escaped = "".join(("\\" + c if c in '\\"$`' else c) for c in str(path))
    return '"' + escaped.replace("\\", "\\\\").replace("%", "%%") + '"'

def patch_config(path, text):
    # Uma configuração instalada pode ser um symlink para os dotfiles.
    path = path.resolve(strict=True)
    if path.read_text() == text:
        return
    backup = path.with_name(path.name + ".ayla-" + datetime.now().strftime("%Y%m%d-%H%M%S-%f") + ".bak")
    shutil.copy2(path, backup)
    write(path, text, path.stat().st_mode & 0o777)
    print(f"Configuração anterior em {backup}")

if not args.binary.is_file() or not args.icon.is_file():
    sys.exit("Executável ou ícone ausente.")
patches = []
if args.quickshell:
    bar = config / "quickshell/ii/modules/bar/BarContent.qml"
    hypr = config / "hypr/hyprland.conf"
    if not bar.is_file() or not hypr.is_file():
        sys.exit("A opção --quickshell requer a barra horizontal do illogical-impulse e hyprland.conf. Para os outros desktops use ./install.sh.")
    old = bar.read_text()
    marker = "            id: leftSectionRowLayout"
    button_marker = "// Ayla Quick View"
    margin = "Appearance.rounding.screenRounding"
    if button_marker not in old:
        if old.count(marker) != 1:
            sys.exit("O layout da barra é diferente do esperado. Nenhuma configuração foi editada.")
        button = """
            RippleButton { // Ayla Quick View
                Layout.leftMargin: MARGIN
                implicitWidth: 36
                implicitHeight: 36
                buttonRadius: Appearance.rounding.normal
                onClicked: Quickshell.execDetached([BIN, "--quick"])
                MaterialSymbol {
                    anchors.centerIn: parent
                    text: "chat_bubble"
                    color: Appearance.colors.colOnLayer0
                }
            }""".replace("MARGIN", margin).replace("BIN", json.dumps(str(exe), ensure_ascii=False))
        patches.append((bar, old.replace(marker, marker + "\n" + button)))
    else:
        legacy_margin = button_marker + "\n                Layout.leftMargin: Appearance.sizes.hyprlandGapsOut"
        if old.count(legacy_margin) == 1:
            updated_margin = button_marker + "\n                Layout.leftMargin: " + margin
            patches.append((bar, old.replace(legacy_margin, updated_margin, 1)))
    old = hypr.read_text()
    # gtk-launch trata argumentos como URIs. A entrada privativa já inclui --background.
    hook = "exec-once = gtk-launch dev.feit.ayla-login"
    shortcut = f"bindd = SUPER, A, Visão rápida da Ayla, exec, {shlex.quote(str(exe))} --quick"
    missing = [line for line in (hook, shortcut) if line not in old.splitlines()]
    if missing:
        patches.append((hypr, old.rstrip("\n") + "\n\n# Ayla Quick View\n" + "\n".join(missing) + "\n"))

applications = data / "applications"

def chosen_icon(entry, base):
    """Ícone escolhido em Aparência (`…/<base>-<hash>.png`), mantido ao atualizar se ainda existir."""
    try:
        found = re.search(r"^Icon=(.+)$", entry.read_text(), re.M)
    except OSError:
        return base
    path = Path(found.group(1)) if found else Path()
    if path.parent == icon.parent and re.fullmatch(re.escape(base) + r"-[0-9a-f]{8}\.png", path.name) and path.is_file():
        return str(path)
    return base

# Tipos para o "Abrir com": a conversa aceita os anexos do chat; Iris, as mídias; Athena, os textos.
media_types = "image/png;image/jpeg;image/gif;image/webp;image/bmp;image/svg+xml;image/avif;application/pdf;video/mp4;video/webm;video/quicktime;video/ogg;video/x-matroska;"
text_types = "text/plain;text/markdown;text/x-markdown;"
chat_types = ("image/png;image/jpeg;image/gif;image/webp;image/bmp;application/pdf;" + text_types
    + "text/csv;text/tab-separated-values;application/json;application/xml;text/html;application/x-yaml;application/toml;text/x-log;application/x-subrip;text/vtt;"
    + "text/x-rust;text/x-python;text/javascript;application/javascript;text/x-java;text/x-csrc;text/x-chdr;text/x-c++src;text/x-go;application/x-ruby;application/x-php;application/x-shellscript;application/sql;text/css;"
    + "audio/mpeg;audio/x-wav;audio/wav;audio/ogg;audio/x-opus+ogg;audio/mp4;audio/aac;audio/flac;audio/x-ms-wma;audio/amr;"
    + "video/mp4;video/quicktime;video/x-matroska;video/webm;video/x-msvideo;video/x-ms-wmv;video/mpeg;video/3gpp;")

write(exe, args.binary, 0o755)
write(icon, args.icon, 0o644)
entry = "[Desktop Entry]\nType=Application\nTerminal=false\nIcon={icon}\n"
ayla_icon = chosen_icon(applications / "dev.feit.ayla.desktop", "dev.feit.ayla")
launcher = exec_arg(exe)
# A janela da Ayla se identifica como "ayla"; StartupWMClass liga a janela a esta entrada (ícone na barra).
write(applications / "dev.feit.ayla.desktop", entry.format(icon=ayla_icon) + f"Name=Ayla\nComment=Assistente pessoal\nExec={launcher} --abrir ayla %F\nStartupWMClass=ayla\nMimeType={chat_types}\nCategories=Utility;\nKeywords=assistente;chat;\nActions=Quick;\n\n[Desktop Action Quick]\nName=Visão rápida\nExec={launcher} --quick\n", 0o644)
# Iris e Athena só aparecem no "Abrir com" (fora do menu); nenhuma vira aplicativo padrão.
for organ, name, comment, types in (("iris", "Iris", "Ver imagens, vídeos e PDFs na Ayla", media_types), ("athena", "Athena", "Ler e editar notas e textos na Ayla", text_types)):
    base = f"dev.feit.ayla.{organ}"
    organ_icon = chosen_icon(applications / f"{base}.desktop", base)
    if organ_icon == base:
        organ_icon = ayla_icon
    write(applications / f"{base}.desktop", entry.format(icon=organ_icon) + f"Name={name}\nComment={comment}\nExec={launcher} --abrir {organ} %F\nMimeType={types}\nNoDisplay=true\n", 0o644)
# Remove a entrada separada criada pelas versões anteriores do instalador.
(applications / "dev.feit.ayla-quick.desktop").unlink(missing_ok=True)
login = entry.format(icon=ayla_icon) + f"Name=Ayla em segundo plano\nExec={launcher} --background\nNoDisplay=true\n"
write(config / "autostart/dev.feit.ayla.desktop", login, 0o644)
if args.quickshell:
    write(applications / "dev.feit.ayla-login.desktop", login, 0o644)
    for path, text in patches:
        patch_config(path, text)
# Atualiza o índice de tipos do "Abrir com" e o cache de ícones do usuário (um cache antigo
# criado por outros programas esconde da barra e dos menus os ícones que não estavam nele).
def refresh(command, *paths):
    if shutil.which(command[0]):
        subprocess.run([*command, *map(str, paths)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
refresh(["update-desktop-database", "-q"], applications)
theme = icon.parents[2]
if (theme / "icon-theme.cache").exists():
    refresh(["gtk-update-icon-cache", "-q", "-f", "-t"], theme)
print(f"Ayla instalada em {exe}. Procure por Ayla no menu de aplicativos.")
