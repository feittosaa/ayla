"""Instalação por usuário. A opção QuickShell se aplica ao layout illogical-impulse."""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
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

write(exe, args.binary, 0o755)
write(icon, args.icon, 0o644)
entry = "[Desktop Entry]\nType=Application\nTerminal=false\nIcon=dev.feit.ayla\n"
launcher = exec_arg(exe)
applications = data / "applications"
write(applications / "dev.feit.ayla.desktop", entry + f"Name=Ayla\nComment=Assistente pessoal\nExec={launcher}\nCategories=Utility;\nKeywords=assistente;chat;\nActions=Quick;\n\n[Desktop Action Quick]\nName=Visão rápida\nExec={launcher} --quick\n", 0o644)
# Remove a entrada separada criada pelas versões anteriores do instalador.
(applications / "dev.feit.ayla-quick.desktop").unlink(missing_ok=True)
login = entry + f"Name=Ayla em segundo plano\nExec={launcher} --background\nNoDisplay=true\n"
write(config / "autostart/dev.feit.ayla.desktop", login, 0o644)
if args.quickshell:
    write(applications / "dev.feit.ayla-login.desktop", login, 0o644)
    for path, text in patches:
        patch_config(path, text)
print(f"Ayla instalada em {exe}. Procure por Ayla no menu de aplicativos.")
