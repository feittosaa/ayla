#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if [[ ${1:-} == --help ]]; then
  echo 'Uso: ./install.sh [--quickshell] — instala a Ayla para o usuário. --quickshell acrescenta um botão à barra horizontal do illogical-impulse e inicia a Ayla ao entrar no Hyprland.'
  exit 0
fi
[[ $(uname -s) == Linux ]] || { echo 'Esta instalação requer Linux.' >&2; exit 1; }
if [[ $# -gt 1 || ( $# -eq 1 && $1 != --quickshell ) ]]; then echo 'Opção inválida. Consulte --help.' >&2; exit 2; fi
if [[ -x "$root/linux/ayla" ]]; then
  binary="$root/linux/ayla"
  icon="$root/linux/icon.png"
else
  cd "$root/frontend"
  [[ -x node_modules/.bin/vite ]] || npm ci
  npm run build
  cargo build --manifest-path src-tauri/Cargo.toml --locked --release --features tauri/custom-protocol
  binary="${CARGO_TARGET_DIR:-$root/frontend/src-tauri/target}/release/app"
  icon="$root/frontend/src-tauri/icons/icon.png"
fi
exec python3 "$root/linux/install.py" "$binary" "$icon" "$@"
