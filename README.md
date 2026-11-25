# Hypolia Platform

Services communs du réseau Hypolia et déploiement Kubernetes.

| Composant | Rôle | Sources |
|---|---|---|
| `hypolia-orchestrator` | Files d'attente, matchmaking, flotte de serveurs de jeu (`kube-rs`), API lobby | [`crates/hypolia-orchestrator`](./crates/hypolia-orchestrator) |
| `hypolia-progression-pg` | Stockage Postgres de la progression et migrations (`hypolia-migrate`) | [`crates/hypolia-progression-pg`](./crates/hypolia-progression-pg) |
| `hypolia-proxy` | Plugin de `rspigot-proxy` : routage vers les lobbies et les parties, transferts, `/hub` et `/server` | [`crates/hypolia-proxy`](./crates/hypolia-proxy) |
| Chart `hypolia` | Déploiement complet du réseau | [`deploy/charts/hypolia`](./deploy/charts/hypolia) |

Images publiées : `ghcr.io/hypolia/orchestrator`, `ghcr.io/hypolia/proxy` (`rspigot-proxy`) et `ghcr.io/hypolia/proxy-plugin` (`hypolia-proxy`).

Dépend de [`hypolia/sdk`](https://github.com/hypolia/sdk), cloné dans `../sdk` et figé par `sdk.ref`, et de [`ferrislabs/rspigot`](https://github.com/ferrislabs/rspigot), cloné dans `/opt/minecraft/dev/rspigot` et figé par `rspigot.ref`.

## Proxy en local (`:25565`)

```bash
./deploy/local/run-proxy.sh
cargo run -p hypolia-proxy
```

Le premier lance `rspigot-proxy` avec [`deploy/local/proxy/proxy.toml`](./deploy/local/proxy/proxy.toml). Il crée le secret de forwarding et le recopie dans `../lobby/server`, `../sheepwars/server` et `../towerwars/server`. Le second branche le plugin Hypolia sur le proxy et sur l'orchestrateur (`ORCHESTRATOR_URL`, par défaut `http://127.0.0.1:8080`).

## Construire les images

```bash
docker buildx bake
```

## Déployer via Helm

Chaque image a son propre tag. Le chart les lit dans `proxy.image.tag`, `proxyPlugin.image.tag`, `orchestrator.image.tag`, `lobby.pluginImage.tag`, `lobby.rspigotImage.tag`, `sheepwars.pluginSidecar.image.tag`, `sheepwars.rspigotImage.tag`, `towerwars.pluginSidecar.image.tag` et `towerwars.rspigotImage.tag`. Un tag vide retombe sur `global.imageTag`.

```bash
helm upgrade --install hypolia ./deploy/charts/hypolia \
  --namespace hypolia --create-namespace \
  -f ./deploy/charts/hypolia/values-k3d.yaml
```

Nombre max de serveurs SheepWars et seuil de lancement :

```bash
helm upgrade --install hypolia ./deploy/charts/hypolia \
  --namespace hypolia --create-namespace \
  --set orchestrator.sheepwars.maxReplicas=10 \
  --set orchestrator.queue.minPlayers=8
```

TowerWars est désactivé par défaut (`orchestrator.towerwars.enabled=false`) et activé dans `values-k3d.yaml`. Il a sa propre file, sa propre flotte et son CRD `towerwarsservers.hypolia.io` (`kubectl get tws`).

## Maps sur S3

Avec `s3.enabled=true`, les maps vivent dans le bucket `hypolia-maps`, une par dossier `<jeu>/<map>/{world.zip,arena.json}` :

- `sheepwars/<map>/` : l'orchestrateur tourne entre toutes les maps présentes, et l'`initContainer` `map-sync` copie la map choisie dans chaque pod ;
- `towerwars/<map>/` : la map précuite nommée par `orchestrator.towerwars.map`.

En k3d, un serveur RustFS est déployé dans le cluster.

```bash
./deploy/scripts/upload-map.sh sheepwars steampunk ../sheepwars/server
./deploy/scripts/upload-map.sh towerwars imperial ../towerwars/dist/towerwars-maps/imperial
./deploy/scripts/upload-map.sh sheepwars ma-map /chemin/world.zip /chemin/arena.json
```

## Documentation

- [`docs/adr`](./docs/adr) : décisions d'architecture, dont le passage à `rspigot-proxy` ([ADR 0012](./docs/adr/0012-rspigot-proxy-remplace-velocity.md))
- [`docs/glossary.md`](./docs/glossary.md)
- [`docs/specs`](./docs/specs)
