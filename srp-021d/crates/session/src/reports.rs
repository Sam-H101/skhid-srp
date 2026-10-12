//! Finished 0.21d matches: their results screen and what each player got,
//! for the lobby to show when the players are back.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use survarium::v021d::results::{PlayerStats, Reward};

use crate::LockExt;

/// The reports kept for players asking again (`21 13`, `21 0E`).
const KEPT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchReport {
    pub match_id: u32,
    /// `game_team_id`: 0, 1, 2 (a draw) or 3 (unknown).
    pub winner: u8,
    pub team_points: [u8; 2],
    /// By roster index, with each player's account and reward.
    pub players: Vec<PlayerStats>,
    pub accounts: Vec<u32>,
    pub rewards: Vec<Reward>,
}

impl MatchReport {
    pub fn index_of(&self, account_id: u32) -> Option<usize> {
        self.accounts
            .iter()
            .position(|account| *account == account_id)
    }

    /// The results screen (`36 13`) for the account's player.
    pub fn screen_for(&self, account_id: u32) -> Option<Vec<u8>> {
        let index = self.index_of(account_id)?;
        Some(survarium::v021d::results::match_stats(
            self.match_id,
            self.winner,
            self.team_points,
            &self.players,
            &self.players[index],
            &self.rewards[index],
        ))
    }

    /// `36 0E` for the player the screen calls `player_id`.
    pub fn player_details(&self, player_id: u32) -> Option<Vec<u8>> {
        let index = self
            .players
            .iter()
            .position(|player| player.id == player_id)?;
        Some(survarium::v021d::results::player_match_stats(
            &self.players[index],
            &self.rewards[index],
        ))
    }
}

#[derive(Default)]
pub struct Reports {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    reports: VecDeque<Arc<MatchReport>>,
    /// The report each account has yet to be shown.
    unseen: HashMap<u32, u32>,
}

impl Reports {
    pub fn add(&self, report: MatchReport) {
        let mut inner = self.inner.locked();
        for account in &report.accounts {
            inner.unseen.insert(*account, report.match_id);
        }
        if inner.reports.len() == KEPT {
            inner.reports.pop_front();
        }
        inner.reports.push_back(Arc::new(report));
    }

    /// The report the account hasn't been shown yet, once.
    pub fn take_unseen(&self, account_id: u32) -> Option<Arc<MatchReport>> {
        let mut inner = self.inner.locked();
        let match_id = inner.unseen.remove(&account_id)?;
        inner.find(match_id)
    }

    pub fn find(&self, match_id: u32) -> Option<Arc<MatchReport>> {
        self.inner.locked().find(match_id)
    }
}

impl Inner {
    fn find(&self, match_id: u32) -> Option<Arc<MatchReport>> {
        self.reports
            .iter()
            .rev()
            .find(|report| report.match_id == match_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(match_id: u32) -> MatchReport {
        let player = PlayerStats {
            id: 1,
            name: "alice".into(),
            ..PlayerStats::default()
        };
        MatchReport {
            match_id,
            winner: 0,
            team_points: [1, 0],
            players: vec![player],
            accounts: vec![77],
            rewards: vec![Reward::default()],
        }
    }

    #[test]
    fn a_report_is_shown_once_and_kept_for_asking() {
        let reports = Reports::default();
        reports.add(report(0x123));
        let unseen = reports.take_unseen(77).unwrap();
        assert_eq!(unseen.match_id, 0x123);
        assert!(reports.take_unseen(77).is_none());
        let report = reports.find(0x123).unwrap();
        assert_eq!(report.screen_for(77).unwrap()[..2], [0x36, 0x13]);
        assert_eq!(report.screen_for(78), None);
        assert_eq!(report.player_details(1).unwrap()[..2], [0x36, 0x0E]);
    }
}
