use std::collections::HashMap;
use std::time::{Duration, Instant};

use hypolia_protocol::social::{PARTY_MAX_SIZE, Party, PartyOutcome, PlayerRef};
use uuid::Uuid;

pub const INVITE_TTL: Duration = Duration::from_secs(60);

struct Invite {
    player: PlayerRef,
    until: Instant,
}

struct Group {
    id: Uuid,
    leader: PlayerRef,
    members: Vec<PlayerRef>,
    invites: Vec<Invite>,
}

impl Group {
    fn view(&self, now: Instant) -> Party {
        Party {
            id: self.id,
            leader: self.leader.clone(),
            members: self.members.clone(),
            invited: self
                .invites
                .iter()
                .filter(|invite| invite.until > now)
                .map(|invite| invite.player.clone())
                .collect(),
        }
    }

    fn is_full(&self) -> bool {
        self.members.len() >= PARTY_MAX_SIZE
    }
}

#[derive(Default)]
pub struct PartyBook {
    groups: HashMap<Uuid, Group>,
    membership: HashMap<Uuid, Uuid>,
}

impl PartyBook {
    pub fn party_of(&self, now: Instant, player: Uuid) -> Option<Party> {
        let id = self.membership.get(&player)?;
        self.groups.get(id).map(|group| group.view(now))
    }

    fn drop_if_alone(&mut self, player: Uuid) {
        let Some(id) = self.membership.get(&player).copied() else {
            return;
        };
        if self
            .groups
            .get(&id)
            .is_some_and(|group| group.members.len() <= 1)
        {
            self.groups.remove(&id);
            self.membership.remove(&player);
        }
    }

    pub fn invite(&mut self, now: Instant, leader: PlayerRef, target: PlayerRef) -> PartyOutcome {
        if leader.uuid == target.uuid {
            return PartyOutcome::Yourself;
        }
        self.drop_if_alone(target.uuid);
        if self.membership.contains_key(&target.uuid) {
            return PartyOutcome::TargetInParty;
        }
        let id = match self.membership.get(&leader.uuid) {
            Some(id) => *id,
            None => {
                let id = Uuid::new_v4();
                self.groups.insert(
                    id,
                    Group {
                        id,
                        leader: leader.clone(),
                        members: vec![leader.clone()],
                        invites: Vec::new(),
                    },
                );
                self.membership.insert(leader.uuid, id);
                id
            }
        };
        let Some(group) = self.groups.get_mut(&id) else {
            return PartyOutcome::NotInParty;
        };
        if group.leader.uuid != leader.uuid {
            return PartyOutcome::NotLeader;
        }
        if group.is_full() {
            return PartyOutcome::PartyFull;
        }
        group
            .invites
            .retain(|invite| invite.until > now && invite.player.uuid != target.uuid);
        group.invites.push(Invite {
            player: target.clone(),
            until: now + INVITE_TTL,
        });
        PartyOutcome::Invited {
            party: group.view(now),
            target,
        }
    }

    pub fn accept(&mut self, now: Instant, player: PlayerRef, leader: Uuid) -> PartyOutcome {
        self.drop_if_alone(player.uuid);
        if self.membership.contains_key(&player.uuid) {
            return PartyOutcome::AlreadyInParty;
        }
        let Some(id) = self.membership.get(&leader).copied() else {
            return PartyOutcome::NoInvite;
        };
        let Some(group) = self.groups.get_mut(&id) else {
            return PartyOutcome::NoInvite;
        };
        let invited = group
            .invites
            .iter()
            .any(|invite| invite.player.uuid == player.uuid && invite.until > now);
        if !invited {
            return PartyOutcome::NoInvite;
        }
        if group.is_full() {
            return PartyOutcome::PartyFull;
        }
        group
            .invites
            .retain(|invite| invite.player.uuid != player.uuid);
        group.members.push(player.clone());
        self.membership.insert(player.uuid, id);
        PartyOutcome::Joined {
            party: group.view(now),
        }
    }

    pub fn leave(&mut self, now: Instant, player: Uuid) -> PartyOutcome {
        let Some(id) = self.membership.remove(&player) else {
            return PartyOutcome::NotInParty;
        };
        let Some(group) = self.groups.get_mut(&id) else {
            return PartyOutcome::NotInParty;
        };
        let Some(index) = group
            .members
            .iter()
            .position(|member| member.uuid == player)
        else {
            return PartyOutcome::NotInParty;
        };
        let left = group.members.remove(index);
        if group.members.len() <= 1 {
            let members = std::mem::take(&mut group.members);
            for member in &members {
                self.membership.remove(&member.uuid);
            }
            self.groups.remove(&id);
            let mut everyone = members;
            everyone.push(left);
            return PartyOutcome::Disbanded { members: everyone };
        }
        if group.leader.uuid == player {
            group.leader = group.members[0].clone();
        }
        PartyOutcome::Left {
            party: Some(group.view(now)),
            player: left,
        }
    }

    pub fn kick(&mut self, now: Instant, leader: Uuid, target: Uuid) -> PartyOutcome {
        if leader == target {
            return PartyOutcome::Yourself;
        }
        let Some(id) = self.membership.get(&leader).copied() else {
            return PartyOutcome::NotInParty;
        };
        let Some(group) = self.groups.get(&id) else {
            return PartyOutcome::NotInParty;
        };
        if group.leader.uuid != leader {
            return PartyOutcome::NotLeader;
        }
        if self.membership.get(&target) != Some(&id) {
            return PartyOutcome::NotInParty;
        }
        let target_ref = group
            .members
            .iter()
            .find(|member| member.uuid == target)
            .cloned();
        match self.leave(now, target) {
            PartyOutcome::Left {
                party: Some(party), ..
            } => PartyOutcome::Kicked {
                party,
                target: target_ref.unwrap_or(PlayerRef {
                    uuid: target,
                    name: String::new(),
                }),
            },
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: u128, name: &str) -> PlayerRef {
        PlayerRef {
            uuid: Uuid::from_u128(id),
            name: name.to_owned(),
        }
    }

    fn steve() -> PlayerRef {
        player(1, "Steve")
    }

    fn alex() -> PlayerRef {
        player(2, "Alex")
    }

    #[test]
    fn an_invite_then_an_accept_forms_a_party_led_by_the_inviter() {
        let mut book = PartyBook::default();
        let now = Instant::now();
        let PartyOutcome::Invited { party, .. } = book.invite(now, steve(), alex()) else {
            panic!("expected an invite");
        };
        assert_eq!(party.members, [steve()]);
        assert_eq!(party.invited, [alex()]);
        let PartyOutcome::Joined { party } = book.accept(now, alex(), steve().uuid) else {
            panic!("expected a join");
        };
        assert_eq!(party.leader, steve());
        assert_eq!(party.members, [steve(), alex()]);
        assert!(party.invited.is_empty());
        assert_eq!(book.party_of(now, alex().uuid).unwrap().id, party.id);
    }

    #[test]
    fn an_old_invite_cannot_be_accepted() {
        let mut book = PartyBook::default();
        let now = Instant::now();
        book.invite(now, steve(), alex());
        assert_eq!(
            book.accept(now + INVITE_TTL, alex(), steve().uuid),
            PartyOutcome::NoInvite
        );
    }

    #[test]
    fn only_the_leader_invites_and_kicks() {
        let mut book = PartyBook::default();
        let now = Instant::now();
        book.invite(now, steve(), alex());
        book.accept(now, alex(), steve().uuid);
        assert_eq!(
            book.invite(now, alex(), player(3, "Notch")),
            PartyOutcome::NotLeader
        );
        assert_eq!(
            book.kick(now, alex().uuid, steve().uuid),
            PartyOutcome::NotLeader
        );
        assert!(matches!(
            book.kick(now, steve().uuid, alex().uuid),
            PartyOutcome::Disbanded { .. }
        ));
    }

    #[test]
    fn a_party_holds_four_players_at_most() {
        let mut book = PartyBook::default();
        let now = Instant::now();
        for id in 2..=4 {
            let member = player(id, &format!("p{id}"));
            book.invite(now, steve(), member.clone());
            book.accept(now, member, steve().uuid);
        }
        assert_eq!(
            book.invite(now, steve(), player(5, "p5")),
            PartyOutcome::PartyFull
        );
    }

    #[test]
    fn the_next_member_leads_when_the_leader_leaves() {
        let mut book = PartyBook::default();
        let now = Instant::now();
        for id in 2..=3 {
            let member = player(id, &format!("p{id}"));
            book.invite(now, steve(), member.clone());
            book.accept(now, member, steve().uuid);
        }
        let PartyOutcome::Left {
            party: Some(party), ..
        } = book.leave(now, steve().uuid)
        else {
            panic!("expected the party to go on");
        };
        assert_eq!(party.leader.uuid, Uuid::from_u128(2));
        assert_eq!(party.members.len(), 2);
        assert!(book.party_of(now, steve().uuid).is_none());
    }

    #[test]
    fn a_party_of_one_is_disbanded() {
        let mut book = PartyBook::default();
        let now = Instant::now();
        book.invite(now, steve(), alex());
        book.accept(now, alex(), steve().uuid);
        let PartyOutcome::Disbanded { members } = book.leave(now, alex().uuid) else {
            panic!("expected the party to end");
        };
        assert_eq!(members.len(), 2);
        assert!(book.party_of(now, steve().uuid).is_none());
        assert_eq!(book.leave(now, steve().uuid), PartyOutcome::NotInParty);
    }

    #[test]
    fn a_party_that_never_grew_does_not_block_anyone() {
        let mut book = PartyBook::default();
        let now = Instant::now();
        book.invite(now, alex(), player(3, "Notch"));
        assert!(matches!(
            book.invite(now, steve(), alex()),
            PartyOutcome::Invited { .. }
        ));
        assert!(matches!(
            book.accept(now, alex(), steve().uuid),
            PartyOutcome::Joined { .. }
        ));
    }

    #[test]
    fn players_already_in_a_party_cannot_be_invited_or_join_another() {
        let mut book = PartyBook::default();
        let now = Instant::now();
        book.invite(now, steve(), alex());
        book.accept(now, alex(), steve().uuid);
        assert_eq!(
            book.invite(now, player(3, "Notch"), alex()),
            PartyOutcome::TargetInParty
        );
        book.invite(now, player(3, "Notch"), player(4, "Dinnerbone"));
        assert_eq!(
            book.accept(now, alex(), Uuid::from_u128(3)),
            PartyOutcome::AlreadyInParty
        );
    }
}
