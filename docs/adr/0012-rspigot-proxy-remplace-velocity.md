# 0012. rspigot-proxy remplace Velocity

Status: accepted
Date: 2026-11-25

## Context

Le réseau passait par Velocity 4.3.1 avec un plugin Java (`hypolia-velocity`). Chaque changement de ce plugin redémarrait le proxy et déconnectait tous les joueurs. C'était aussi le seul code Java de Hypolia, avec sa JVM : environ 350 Mo de mémoire au pic pour 50 joueurs pendant notre test de charge.

rspigot fournit maintenant `rspigot-proxy` (ferrislabs/rspigot#453). Il fait le même travail que Velocity devant des serveurs rspigot, et ses plugins tournent dans leur propre processus, connectés en gRPC.

## Decision

Le réseau passe par `rspigot-proxy`, construit depuis `ferrislabs/rspigot` au commit de `rspigot.ref`. Le plugin Java devient `hypolia-proxy`, écrit avec `rspigot-proxy-plugin` :

- il lit la flotte de l'orchestrateur (`/api/v1/proxy/state` et le flux d'événements) et enregistre chaque pod lobby comme serveur de secours ;
- il exécute les transferts de l'orchestrateur et les acquitte ;
- il choisit le lobby le moins chargé à la connexion et après un kick en partie ;
- il signale les déconnexions ;
- il fournit `/hub` (et `/lobby`), plus `/server` réservé aux grades admin, mod et dev.

Le proxy et le plugin sont deux Deployments séparés. Le plugin se connecte au port des plugins du proxy (`25579`) avec un token. Le Service `hypolia-proxy` remplace `hypolia-velocity` sur le port 25565.

## Consequences

- Ce qui devient plus simple : déployer le plugin ne déconnecte plus personne. Le Java disparaît du dépôt, et le proxy expose `/metrics` et `/health`.
- Ce que ça coûte : une image de plus (`proxy-plugin`), un Secret de plus (le token du plugin) et la dépendance à `rspigot.ref` dans ce dépôt.
- Ce qui devient plus difficile : le proxy ne gère que la version 26.3 du client, comme les serveurs rspigot. Velocity ne servait de toute façon qu'à cette version.
