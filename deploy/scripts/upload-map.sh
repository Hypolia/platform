#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 || $# -gt 4 ]]; then
  echo "Usage :" >&2
  echo "  $0 <jeu> <nom-map> <dossier-map>" >&2
  echo "  $0 <jeu> <nom-map> <world.zip> <arena.json>" >&2
  echo "<jeu> : sheepwars, towerwars ou deacoudre. La map va sous s3://<bucket>/<jeu>/<nom-map>/." >&2
  exit 1
fi

GAME="$1"
case "${GAME}" in
  sheepwars|towerwars|deacoudre) ;;
  *) echo "Erreur : jeu inconnu (${GAME}), attendu sheepwars, towerwars ou deacoudre." >&2; exit 1 ;;
esac
shift
MAP_NAME="$1"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

if [[ $# -eq 2 ]]; then
  MAP_DIR="$2"
  WORLD_ZIP="${MAP_DIR}/world.zip"
  ARENA_JSON="${MAP_DIR}/arena.json"
  if [[ ! -f "${WORLD_ZIP}" && -d "${MAP_DIR}/world" ]]; then
    WORLD_ZIP="$(mktemp -t "${MAP_NAME}-world.XXXXXX.zip")"
    trap 'rm -f "${WORLD_ZIP}"' EXIT
    (cd "${MAP_DIR}" && zip -rq "${WORLD_ZIP}" world)
  fi
else
  WORLD_ZIP="$2"
  ARENA_JSON="$3"
fi

if [[ ! -f "${WORLD_ZIP}" ]]; then
  echo "Erreur : archive introuvable (${WORLD_ZIP})." >&2
  exit 1
fi
if [[ ! -f "${ARENA_JSON}" ]]; then
  echo "Erreur : arena.json introuvable (${ARENA_JSON})." >&2
  exit 1
fi

KUBE_CONTEXT="${KUBE_CONTEXT:-k3d-hypolia}"
KUBE_NAMESPACE="${KUBE_NAMESPACE:-hypolia}"
S3_BUCKET="${S3_BUCKET:-hypolia-maps}"
S3_PREFIX="${GAME}/"
S3_REGION="${S3_REGION:-eu-west-3}"
S3_PATH_STYLE="${S3_PATH_STYLE:-true}"

PF_PID=""
cleanup() {
  if [[ -n "${PF_PID}" ]]; then
    kill "${PF_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if [[ -z "${S3_ENDPOINT:-}" ]]; then
  LOCAL_PORT="${S3_LOCAL_PORT:-19000}"
  kubectl --context "${KUBE_CONTEXT}" -n "${KUBE_NAMESPACE}" \
    port-forward svc/hypolia-rustfs "${LOCAL_PORT}:9000" >/dev/null 2>&1 &
  PF_PID=$!
  for _ in $(seq 1 30); do
    if curl -fsS "http://127.0.0.1:${LOCAL_PORT}/health/ready" >/dev/null 2>&1; then
      break
    fi
    sleep 0.5
  done
  export S3_ENDPOINT="http://127.0.0.1:${LOCAL_PORT}"
  export S3_ACCESS_KEY_ID="${S3_ACCESS_KEY_ID:-hypolia}"
  export S3_SECRET_ACCESS_KEY="${S3_SECRET_ACCESS_KEY:-hypolia-dev-s3-secret}"
fi

export S3_BUCKET S3_PREFIX S3_REGION S3_PATH_STYLE

cargo run --manifest-path "${ROOT_DIR}/Cargo.toml" \
  -p hypolia-orchestrator \
  --bin hypolia-map-sync -- \
  upload "${MAP_NAME}" "${WORLD_ZIP}" "${ARENA_JSON}"
