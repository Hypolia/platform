#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${HERE}/../../.." && pwd)"
RSPIGOT_DIR="${RSPIGOT_DIR:-/opt/minecraft/dev/rspigot}"
SECRET_PATH="${HERE}/proxy/forwarding.secret"

if [[ ! -s "${SECRET_PATH}" ]]; then
  openssl rand -hex 24 | tr -d '\n' > "${SECRET_PATH}"
  chmod 600 "${SECRET_PATH}"
fi

python3 -I - "${ROOT}" "$(cat "${SECRET_PATH}")" <<'PY'
import pathlib, re, sys

root, secret = pathlib.Path(sys.argv[1]), sys.argv[2]
for rel in ("lobby/server/spigot.yml", "sheepwars/server/spigot.yml", "towerwars/server/spigot.yml"):
    path = root / rel
    example = root / f"{rel}.example"
    if not path.exists() and example.exists():
        path.write_text(example.read_text())
    if not path.exists():
        continue
    path.write_text(re.sub(
        r"(  velocity:\n\s+enabled:\s*)(?:true|false)(\n\s+secret:\s*).*",
        rf"\g<1>true\g<2>'{secret}'",
        path.read_text(),
    ))
PY

cargo run --release --manifest-path "${RSPIGOT_DIR}/Cargo.toml" -p rspigot-proxy -- "${HERE}/proxy"
