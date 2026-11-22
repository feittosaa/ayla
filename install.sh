#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if [[ ${1:-} == --help ]]; then
  echo 'Uso: ./install.sh [--quickshell] [--com-moneta] — instala a Ayla para o usuário. --quickshell acrescenta um botão à barra horizontal do illogical-impulse e inicia a Ayla ao entrar no Hyprland. --com-moneta baixa também a Moneta (finanças), que roda num contêiner Docker separado; dá para baixar ou remover depois em Configurações › Órgãos.'
  exit 0
fi
[[ $(uname -s) == Linux ]] || { echo 'Esta instalação requer Linux.' >&2; exit 1; }
# Primeira instalação: ainda não há Ayla para este usuário. Atualizações nunca perguntam pelos órgãos.
bindir=${XDG_BIN_HOME:-$HOME/.local/bin}
first=0; [[ -e $bindir/ayla ]] || first=1
options=(); moneta=0
for option in "$@"; do
  case $option in
    --quickshell) options+=(--quickshell) ;;
    --com-moneta) moneta=1 ;;
    *) echo 'Opção inválida. Consulte --help.' >&2; exit 2 ;;
  esac
done
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
python3 "$root/linux/install.py" "$binary" "$icon" "${options[@]}"
# Órgãos são opcionais: na primeira instalação (com terminal), a pessoa escolhe quais baixar.
organs=(); (( moneta )) && organs+=(moneta)
if (( first && ! moneta )) && [[ -t 0 && -d $root/organs ]]; then
  echo
  echo 'Órgãos opcionais. Dá para baixar ou remover depois em Configurações › Órgãos.'
  while IFS=$'\t' read -r id name description; do
    read -r -p "  $name — $description Baixar agora? [s/N] " answer < /dev/tty
    [[ ${answer,,} == s* ]] && organs+=("$id")
  done < <(python3 -c 'import json,sys
for path in sorted(sys.argv[1:]):
    o = json.load(open(path))
    print(o["id"], o["name"], o["description"], sep="\t")' "$root"/organs/*/organ.json)
fi
# O próprio binário da Ayla baixa o repositório oficial de cada órgão e prepara o contêiner.
for id in "${organs[@]}"; do "$binary" "--instalar-$id"; done
