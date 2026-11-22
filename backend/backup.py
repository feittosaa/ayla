"""Backup consistente do SQLite, sem ler conteúdo pessoal no terminal.

Uso: python backup.py [--source /caminho/banco.db] [--output /caminho/backup.db]
Restauração: com a Ayla parada, preserve o banco atual e copie o backup sobre ele.
"""

import argparse
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path
import sqlite3
from app.core.persistent_memory import PersistentMemory


def backup_database(source: Path, destination: Path):
    source, destination = source.resolve(), destination.resolve()
    if not source.is_file():
        raise FileNotFoundError("Banco de origem não encontrado")
    destination.parent.mkdir(parents=True, exist_ok=True)
    # Não sobrescrever backups existentes, inclusive quando origem == destino.
    with destination.open("xb"):
        pass
    try:
        with closing(sqlite3.connect(source.as_uri() + "?mode=ro", uri=True)) as src:
            with closing(sqlite3.connect(destination)) as dst:
                src.backup(dst)
    except BaseException:
        destination.unlink(missing_ok=True)
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Backup do SQLite da Ayla")
    parser.add_argument("--source", type=Path, default=PersistentMemory().path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    destination = args.output or args.source.parent / "backups" / (
        "ayla-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ") + ".db"
    )
    backup_database(args.source, destination)
    print(f"Backup criado: {destination}")
