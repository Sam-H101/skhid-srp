use crate::lobby_server::server;
use session::Account;
use survarium::player_profile;
use vostok::config;

pub struct ConnectionState {
    pub session_id: u32,
    pub account_id: u32,
    /// The id the next bought item gets.
    pub next_item_id: u32,
    pub name: String,

    pub generic_money: u32,
    pub premium_money: u32,
    pub skills_points: u8,

    pub total_experience: u32,
    pub next_level_experience: u32,
    pub prev_level_experience: u32,

    pub player_skills: [server::raw::player_skill; 5],
    pub profile_contents: Vec<player_profile::raw::player_profile>, // 1..4
    /// Everything the account owns, stacks at their full count.
    pub inventory: Vec<player_profile::raw::inventory_item_instance>, // 0..
    /// The profile the client last edited or readied with: the inventory the
    /// client sees is what this profile leaves free.
    pub selected_profile_id: u32,
    pub reps: [server::raw::player_reputation; 4],
    /// The reward notice for the chat, from the last message's handling.
    pub reward_notice: Option<String>,
    /// Where this client finds the match server (see
    /// `vostok::config::ServerConfig::host_for`).
    pub match_host: String,
}

impl ConnectionState {
    /// Snapshot the shared [`Account`] into per-connection state for the given
    /// session. The lobby writes what the player changes back with
    /// [`Self::save_to`].
    pub fn from_account(session_id: u32, account: &Account) -> Self {
        Self {
            session_id,
            account_id: account.id,
            next_item_id: account.next_item_id,
            name: account.name.clone(),
            generic_money: account.generic_money,
            premium_money: account.premium_money,
            skills_points: account.skills_points,
            total_experience: account.total_experience,
            next_level_experience: account.next_level_experience,
            prev_level_experience: account.prev_level_experience,
            player_skills: account.player_skills,
            profile_contents: account.profile_contents.clone(),
            inventory: account.inventory.clone(),
            selected_profile_id: account
                .profile_contents
                .first()
                .map_or(0, |profile| profile.profile_id),
            reps: account.reps,
            reward_notice: None,
            match_host: config::get().match_server.public_host.clone(),
        }
    }

    /// Write what changes in the lobby (purchases, money, loadouts, and what
    /// matches earned and used up) back to the account, so the next lobby
    /// connection, after a match, starts from it.
    pub fn save_to(&self, account: &mut Account) {
        account.next_item_id = self.next_item_id;
        account.generic_money = self.generic_money;
        account.premium_money = self.premium_money;
        account.skills_points = self.skills_points;
        account.total_experience = self.total_experience;
        account.next_level_experience = self.next_level_experience;
        account.prev_level_experience = self.prev_level_experience;
        account.profile_contents.clone_from(&self.profile_contents);
        account.inventory.clone_from(&self.inventory);
    }

    /// The inventory as the client's lobby lists it: a stack counts what the
    /// selected profile leaves of it (the client hides a stack at 0, and
    /// offers this much to equip); one-piece items show whether equipped or
    /// not (`fill_inventory_contents`, `InventoryList`).
    pub fn free_inventory(&self) -> Vec<player_profile::raw::inventory_item_instance> {
        let profile = self
            .profile_contents
            .iter()
            .find(|profile| profile.profile_id == self.selected_profile_id);
        self.inventory
            .iter()
            .map(|item| {
                let mut item = *item;
                if player_profile::is_stack(item.dict_id) {
                    let equipped: u32 = profile
                        .into_iter()
                        .flat_map(|profile| profile.slots.iter())
                        .filter(|slot| slot.id == item.id)
                        .map(|slot| slot.condition_or_stack)
                        .sum();
                    item.condition_or_stack = item.condition_or_stack.saturating_sub(equipped);
                }
                item
            })
            .collect()
    }
}
