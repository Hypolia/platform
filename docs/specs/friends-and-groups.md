# Amis et groupes

## Commandes (plugin proxy `hypolia-proxy`, sur tous les serveurs)

| Commande | Effet |
|---|---|
| `/friend` ou `/friend list` | Amis en ligne (avec leur serveur), amis hors ligne, demandes reçues avec `[Accepter]` `[Refuser]` |
| `/friend add <joueur>` | Envoie une demande. Si l'autre en avait déjà envoyé une, l'amitié est créée tout de suite |
| `/friend accept <joueur>`, `/friend deny <joueur>` | Répond à une demande reçue |
| `/friend remove <joueur>` | Retire un ami, ou annule une demande envoyée |
| `/msg <joueur> <message>` (`/tell`, `/w`), `/r <message>` | Message privé à un joueur connecté, et réponse au dernier |
| `/group` ou `/group list` | Le groupe, son chef et les invitations en cours |
| `/group invite <joueur>` | Invite un joueur connecté (60 s pour accepter). Crée le groupe si besoin |
| `/group accept <chef>` | Rejoint le groupe |
| `/group leave`, `/group kick <joueur>` | Quitte, ou exclut (chef seulement) |
| `/gc <message>` | Chat du groupe |

`/f` et `/g` (ou `/party`) sont des alias.

## Règles

- Amitiés : 100 au plus par joueur, stockées en base (`friendships`, `friend_requests`). On peut ajouter un joueur hors ligne s'il s'est déjà connecté une fois.
- À la connexion, le joueur voit le nombre d'amis en ligne et ses demandes en attente. Ses amis connectés voient « X s'est connecté », puis « X s'est déconnecté ».
- Groupe : 4 joueurs au plus, en mémoire dans l'orchestrateur (perdu si l'orchestrateur redémarre). Le chef est celui qui a invité. S'il part, le membre suivant devient chef. À un seul membre, le groupe est dissous. Se déconnecter fait quitter le groupe.
- File : seul le chef lance une partie. Tout le groupe entre dans la file avec lui, et la file ne sépare jamais un groupe : il attend un serveur avec assez de places. Un membre qui tente de lancer une partie reçoit `NotPartyLeader`. En TowerWars, la taille de la partie monte jusqu'à la taille du groupe.

## API de l'orchestrateur

- `GET /api/v1/players/{uuid}/friends` : `FriendsList`
- `POST /api/v1/friends/{add|accept|deny|remove}` avec `FriendRequest` : `FriendOutcome`
- `GET /api/v1/players/{uuid}/party` : `Party` ou `null`
- `POST /api/v1/parties/{invite|accept|leave|kick}` avec `PartyRequest` : `PartyOutcome`

Les types sont dans `hypolia_protocol::social`.

## Pas encore fait

- Même équipe en SheepWars pour un groupe.
- Menu Amis / Groupe dans le lobby.
