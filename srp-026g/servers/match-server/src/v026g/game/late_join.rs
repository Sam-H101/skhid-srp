//! Admission for the explicitly selected, experimental growing-roster dialect.
//! Preparation changes no simulation membership. Existing peers keep their
//! clocks, histories and normal input relay while this state machine waits.

use super::*;
use survarium::v026g::match_msgs::late_join::{Prepare, Prepared, Proposal, Status};
use survarium::v026g::match_msgs::{MAX_PLAYERS, id};

const RETRY: Duration = Duration::from_secs(1);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) struct LoadingProfile {
    proposal: Proposal,
    sent: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdmissionPhase {
    Preparing,
    CatchingUp { snapshot_after: u32 },
    Entered { snapshot_after: u32 },
    Failed,
    Rejected,
}

pub(super) struct Admission {
    since: Instant,
    phase: AdmissionPhase,
    catchup_hashes: VecDeque<(u32, u32)>,
}

impl Admission {
    pub(super) fn entered(&self) -> bool {
        matches!(self.phase, AdmissionPhase::Entered { .. })
    }

    pub(super) fn enter(&mut self) {
        let (AdmissionPhase::CatchingUp { snapshot_after }
        | AdmissionPhase::Entered { snapshot_after }) = self.phase
        else {
            unreachable!("entry requires successful preparation and catch-up");
        };
        self.phase = AdmissionPhase::Entered { snapshot_after };
        self.catchup_hashes.clear();
    }

    fn failed(&self) -> bool {
        matches!(
            self.phase,
            AdmissionPhase::Failed | AdmissionPhase::Rejected
        )
    }
}

impl Game {
    pub(crate) fn growing_roster(&self) -> bool {
        self.formed.growing_roster
    }

    pub(crate) fn roster_size(&self) -> usize {
        self.players()
    }

    pub(super) fn snapshot_players(&self) -> usize {
        if self.growing_roster() {
            MAX_PLAYERS
        } else {
            self.players()
        }
    }

    /// Registry updates only append immutable descriptors. Take the new Arc
    /// before routing a new connection and before each game-loop iteration.
    pub(crate) fn sync_roster(&mut self, formed: Arc<Formed>, now: Instant) {
        if !self.growing_roster() || formed.places.len() <= self.players() {
            return;
        }
        assert_eq!(formed.match_id, self.id);
        assert_eq!(&formed.places[..self.players()], &self.formed.places);
        for _ in self.players()..formed.places.len() {
            self.peers.push(None);
            self.best_streaks.push(0);
            self.admissions.push(Some(Admission {
                since: now,
                phase: AdmissionPhase::Preparing,
                catchup_hashes: VecDeque::new(),
            }));
        }
        self.formed = formed;
    }

    pub(crate) fn admitting(&self) -> bool {
        self.growing_roster()
            && self.players() < MAX_PLAYERS
            && self.rule.state == rules::state::IN_PROCESS
            && self.mode_end_at.is_none()
            && self.ending.is_none()
            && self.finished_at.is_none()
            && self.live() > 0
            && self.admissions.iter().flatten().all(Admission::entered)
            && self
                .peers
                .iter()
                .flatten()
                .filter(|p| p.stage != Stage::Gone)
                .all(|p| p.resources_loaded && p.installed_roster == self.players())
    }

    pub(crate) fn admission_deadline(&self, now: Instant) -> Option<Instant> {
        if !self.admitting() {
            return None;
        }
        let remaining = Duration::from_millis(u64::from(
            self.rule.time_limit_at()?.saturating_sub(self.clock(now)),
        ));
        let open_for = remaining.checked_sub(session::matchmaking_policy::JOIN_CUTOFF)?;
        if open_for.is_zero() {
            return None;
        }
        now.checked_add(open_for)
    }

    /// Failed newcomers can queue elsewhere. Their descriptors remain
    /// reserved here so partially installed slots are never repurposed.
    pub(crate) fn take_rejected_joins(&mut self) -> Vec<(usize, u32)> {
        let mut rejected = Vec::new();
        for (index, admission) in self.admissions.iter_mut().enumerate() {
            if let Some(admission) = admission
                && admission.phase == AdmissionPhase::Failed
            {
                admission.phase = AdmissionPhase::Rejected;
                rejected.push((index, self.formed.places[index].account_id));
            }
        }
        rejected
    }

    /// Whether a game of the match has a place still to install. Until it
    /// has, its world and the world of a game that has the place differ by
    /// what the place's kits put into the saved world before its player is
    /// let in (a byte for each grenade and trap, patch revision 7; the
    /// games of 5 and 6 differed too), and neither is wrong. A place whose
    /// admission failed is not asked for any more.
    pub(super) fn installing(&self) -> bool {
        self.growing_roster()
            && self.peers.iter().flatten().any(|peer| {
                peer.stage != Stage::Gone
                    && peer.resources_loaded
                    && peer.installed_roster < self.players()
                    && !self.admissions[peer.installed_roster]
                        .as_ref()
                        .is_some_and(Admission::failed)
            })
    }

    pub(super) fn join_ready(&self, index: usize) -> bool {
        let Some(admission) = &self.admissions[index] else {
            return true;
        };
        admission.entered()
            || (matches!(admission.phase, AdmissionPhase::CatchingUp { .. })
                && self
                    .peers
                    .iter()
                    .flatten()
                    .filter(|p| p.stage != Stage::Gone)
                    .all(|p| p.resources_loaded && p.installed_roster > index))
    }

    pub(super) fn after_preparation(&self, index: usize, time: u32) -> bool {
        self.admissions[index].as_ref().is_none_or(|a| {
            matches!(a.phase,
                AdmissionPhase::CatchingUp { snapshot_after }
                | AdmissionPhase::Entered { snapshot_after } if time >= snapshot_after)
        })
    }

    /// A place taken into the match being played whose player is still
    /// to enter, and has not failed to.
    pub(super) fn coming(&self, index: usize) -> bool {
        self.admissions
            .get(index)
            .and_then(Option::as_ref)
            .is_some_and(|admission| !admission.entered() && !admission.failed())
    }

    pub(super) fn waiting_entry(&self, index: usize) -> bool {
        self.admissions[index]
            .as_ref()
            .is_some_and(|a| !a.entered())
    }

    /// A snapshot's transport ACK alone does not prove application/catch-up.
    /// Keep this newcomer's reports separate from the existing world's votes
    /// until it demonstrates a recent matching simulation state.
    pub(super) fn catchup_hash(&mut self, index: usize, time: u32, crc: u32) {
        let Some(admission) = self.admissions[index].as_mut() else {
            return;
        };
        if !matches!(admission.phase, AdmissionPhase::CatchingUp { .. }) {
            return;
        }
        admission.catchup_hashes.push_back((time, crc));
        while admission.catchup_hashes.len() > 32 {
            admission.catchup_hashes.pop_front();
        }
    }

    pub(super) fn reset_catchup(&mut self, index: usize, new_connection: bool) {
        if let Some(admission) = &mut self.admissions[index]
            && !admission.entered()
            && !admission.failed()
        {
            admission.catchup_hashes.clear();
            if new_connection {
                admission.phase = AdmissionPhase::Preparing;
            }
        }
    }

    pub(super) fn accept_prepared(&mut self, index: usize, message_id: u8, payload: &[u8]) -> bool {
        if !self.growing_roster() || message_id != id::LEAVE {
            return false;
        }
        let Ok(ack) = Prepared::decode(payload) else {
            return false;
        };
        let Some(peer) = self.peers[index].as_mut() else {
            return true;
        };
        if peer.stage == Stage::Gone
            || !peer.resources_loaded
            || !peer
                .preparing
                .as_ref()
                .is_some_and(|p| p.proposal == ack.proposal)
        {
            return true;
        }
        if self.admissions[usize::from(ack.proposal.place)]
            .as_ref()
            .is_some_and(Admission::failed)
        {
            return true;
        }
        match ack.status {
            Status::Installed => {
                log::debug!(
                    "Match {:#x}: player {index} installed place {} (proposal {})",
                    self.id,
                    ack.proposal.place,
                    ack.proposal.serial
                );
                peer.installed_roster = usize::from(ack.proposal.place) + 1;
                peer.preparing = None;
            }
            Status::Busy => {} // retry the same immutable proposal
            Status::Failed => {
                log::warn!(
                    "Match {:#x}: player {index} failed to prepare place {}",
                    self.id,
                    ack.proposal.place
                );
                if let Some(admission) = &mut self.admissions[usize::from(ack.proposal.place)] {
                    admission.phase = AdmissionPhase::Failed;
                }
            }
        }
        true
    }

    pub(super) fn poll_roster(&mut self, now: Instant) {
        if !self.growing_roster()
            || self.mode_end_at.is_some()
            || self.ending.is_some()
            || self.finished_at.is_some()
        {
            return;
        }
        let count = self.players();
        for peer in self.peers.iter_mut().flatten() {
            if peer.stage == Stage::Gone || !peer.resources_loaded || peer.installed_roster >= count
            {
                continue;
            }
            let place = peer.installed_roster;
            if self.admissions[place]
                .as_ref()
                .is_some_and(Admission::failed)
            {
                continue;
            }
            if let Some(pending) = &peer.preparing {
                if now.saturating_duration_since(pending.sent) < RETRY {
                    continue;
                }
            } else {
                // A new connection receives a new serial, so a delayed ready
                // packet from its previous incarnation cannot pass the gate.
                let Some(serial) = self.next_proposal.checked_add(1) else {
                    continue;
                };
                self.next_proposal = serial;
                peer.preparing = Some(LoadingProfile {
                    proposal: Proposal {
                        match_id: super::super::registry::static_id(self.id),
                        serial,
                        place: place as u8,
                    },
                    sent: now,
                });
            }
            let pending = peer.preparing.as_mut().unwrap();
            let prepare = Prepare {
                proposal: pending.proposal,
                profile: self.formed.profiles[place].clone(),
            };
            let bytes = prepare
                .encode(Dictionary::shared())
                .expect("registry validated profile");
            if peer.transport.send(id::STATIC_INFO, &bytes).is_ok() {
                pending.sent = now;
            }
        }
        let clock = self.clock(now);
        let mut caught_up = Vec::new();
        for index in self.formed.initial_players..count {
            let ready = self.peers[index]
                .as_ref()
                .is_some_and(|p| p.resources_loaded && p.stage != Stage::Gone)
                && self
                    .peers
                    .iter()
                    .flatten()
                    .filter(|p| p.stage != Stage::Gone)
                    .all(|p| p.resources_loaded && p.installed_roster > index);
            let admission = self.admissions[index].as_mut().unwrap();
            if admission.entered() {
                continue;
            }
            if !admission.failed()
                && now.saturating_duration_since(admission.since) >= PREPARE_TIMEOUT
            {
                log::warn!(
                    "Match {:#x}: new place {index} timed out during preparation or catch-up",
                    self.id
                );
                admission.phase = AdmissionPhase::Failed;
            }
            if admission.failed() {
                if let Some(peer) = self.peers[index].as_mut()
                    && peer.stage != Stage::Gone
                {
                    peer.transport.disconnect(now);
                    self.leave(index, now);
                }
                continue;
            }
            if ready && admission.phase == AdmissionPhase::Preparing {
                // Require a world captured after the barrier. Old cached
                // snapshots and the newcomer's initial upload cannot qualify.
                admission.phase = AdmissionPhase::CatchingUp {
                    snapshot_after: grid_up(clock),
                };
                log::info!(
                    "Match {:#x}: new place {index} prepared; requesting a world from {} ms or later",
                    self.id,
                    grid_up(clock)
                );
                self.request_upload(index, now, Duration::ZERO);
            }
            let ready_to_enter = ready
                && self.peers[index]
                    .as_ref()
                    .is_some_and(|p| p.stage == Stage::Live)
                && self.admissions[index]
                    .as_ref()
                    .unwrap()
                    .catchup_hashes
                    .iter()
                    .any(|&(time, crc)| {
                        time <= clock
                            && clock - time <= FRESH_SNAPSHOT_MS
                            && self.after_preparation(index, time)
                            && self.majority(time) == Some(crc)
                    });
            if ready_to_enter {
                caught_up.push(index);
            }
        }
        // One reservation group is pending at a time. Its members enter or
        // fail together, even when their resources and snapshots finish apart.
        if self.admissions.iter().flatten().any(Admission::failed) {
            for admission in self.admissions.iter_mut().flatten() {
                if !admission.entered() && !admission.failed() {
                    admission.phase = AdmissionPhase::Failed;
                }
            }
            return;
        }
        let waiting = self
            .admissions
            .iter()
            .flatten()
            .filter(|a| !a.entered())
            .count();
        if waiting > 0 && caught_up.len() == waiting {
            let at = grid_up(clock + ROSTER_LEAD_MS);
            for index in caught_up {
                log::info!(
                    "Match {:#x}: new place {index} caught up; entry scheduled for {at} ms",
                    self.id
                );
                self.roster_change(None, index, true, at);
            }
        }
    }
}
