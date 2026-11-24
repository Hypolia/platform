# Hypolia Platform

Services communs du réseau Hypolia et déploiement Kubernetes.

| Composant | Rôle | Sources |
|---|---|---|
| `hypolia-orchestrator` | Files d'attente, matchmaking, flotte de serveurs de jeu (`kube-rs`), API lobby | [`crates/hypolia-orchestrator`](./crates/hypolia-orchestrator) |
| `hypolia-progression-pg` | Stockage Postgres de la progression et migrations (`hypolia-migrate`) | [`crates/hypolia-progression-pg`](./crates/hypolia-progression-pg) |
| `hypolia-velocity` | Plugin Velocity 4.3.1 : routage vers les lobbies et les serveurs de jeu | [`proxy/velocity-plugin`](./proxy/velocity-plugin) |
| Chart `hypolia` | Déploiement complet du réseau | [`deploy/charts/hypolia`](./deploy/charts/hypolia) |

Images publiées : `ghcr.io/hypolia/orchestrator`, `ghcr.io/hypolia/velocity`.

Dépend de [`hypolia/sdk`](https://github.com/hypolia/sdk), cloné dans `../sdk` et figé par `sdk.ref`.

## Proxy Velocity en local (`:25565`)

```bash
./proxy/download-velocity.sh
./proxy/run.sh
```

`download-velocity.sh` écrit le secret de forwarding dans `../lobby/server`, `../sheepwars/server` et `../towerwars/server`.

## Construire les images

```bash
docker buildx bake
```

## Déployer via Helm

Chaque image a son propre tag. Le chart les lit dans `velocity.image.tag`, `orchestrator.image.tag`, `lobby.pluginImage.tag`, `lobby.rspigotImage.tag`, `sheepwars.pluginSidecar.image.tag`, `sheepwars.rspigotImage.tag`, `towerwars.pluginSidecar.image.tag` et `towerwars.rspigotImage.tag`. Un tag vide retombe sur `global.imageTag`.

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

- [`docs/adr`](./docs/adr) : décisions d'architecture
- [`docs/glossary.md`](./docs/glossary.md)
- [`docs/specs`](./docs/specs)
