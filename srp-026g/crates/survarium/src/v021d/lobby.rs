//! The 0.21d lobby messages srp sends, as payloads (id byte first, no frame
//! length), byte for byte as `../srp-021d/docs/notes/lobby_spec.md` gives
//! them. The client reads none of them with a bounds check, so each layout
//! here is exact.

use super::account::{
    Account, Item, MAX_MODIFICATIONS, MAX_NAME_LEN, Profile, slot_carries_amount,
};
use super::skills::{MODIFIERS, SkillsTree};
use super::{loyalty, premium};
use crate::dictionary::PROFILE_SLOTS;
use crate::prices::PRICES;

pub const CONNECT_TO_MATCH_SERVER: u8 = 0x33;
pub const OPERATION_PERMITTED: u8 = 0x34;
pub const OPERATION_DENIED: u8 = 0x35;
pub const CLIENT_STATUS: u8 = 0x36;
pub const ONLINE_STATS: u8 = 0x38;
pub const CONNECTION_SUCCESSFUL: u8 = 0x30;
pub const SQUAD_STATUS: u8 = 0x3A;

/// What a skill reroll and a new profile cost (`36 08`) [a choice: srp's
/// 0.100b values].
pub const REROLL_COST: u32 = 100;
pub const ADD_PROFILE_COST: u32 = 200;

/// `query_info_types`: the sub-types of 0x36.
pub mod status {
    pub const CLIENT_STATE: u8 = 0x00;
    pub const PROFILES: u8 = 0x01;
    pub const PROFILE_CONTENTS: u8 = 0x02;
    pub const INVENTORY: u8 = 0x03;
    pub const PRICES: u8 = 0x04;
    pub const MONEY: u8 = 0x05;
    pub const SKILLS: u8 = 0x06;
    pub const SERVICE_PRICES: u8 = 0x08;
    pub const REPUTATIONS: u8 = 0x09;
    pub const BASE_INFO: u8 = 0x0A;
    pub const PLAYER_ELO: u8 = 0x0B;
    pub const ELO_LIST: u8 = 0x0C;
    pub const QUESTS: u8 = 0x0D;
    pub const SERVICES: u8 = 0x0F;
    pub const SQUAD_MEMBER_PROFILE: u8 = 0x10;
    pub const SHOP_ITEMS: u8 = 0x11;
    pub const UNLOCKED_FACTIONS: u8 = 0x12;
}

/// `client_state_enum`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientState {
    /// In the lobby: the only state in which Play is enabled.
    Lobby,
    InQueue {
        order_id: u32,
        /// 0 shows "match found".
        place: u32,
        queue_total: u32,
        current_wait_seconds: u32,
        average_wait_seconds: u32,
    },
    InMatch {
        order_id: u32,
        match_id: u32,
        team: u8,
    },
}

#[derive(Default)]
pub(super) struct Writer(Vec<u8>);

impl Writer {
    pub(super) fn new(id: u8) -> Self {
        Self(vec![id])
    }
    pub(super) fn u8(&mut self, value: u8) -> &mut Self {
        self.0.push(value);
        self
    }
    pub(super) fn u16(&mut self, value: u16) -> &mut Self {
        self.0.extend(value.to_le_bytes());
        self
    }
    pub(super) fn u32(&mut self, value: u32) -> &mut Self {
        self.0.extend(value.to_le_bytes());
        self
    }
    /// `u8 len` and the bytes, cut to the client's buffer.
    pub(super) fn str(&mut self, value: &str, max_len: usize) -> &mut Self {
        let mut end = value.len().min(max_len).min(255);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.u8(end as u8);
        self.0.extend(&value.as_bytes()[..end]);
        self
    }
    pub(super) fn f32(&mut self, value: f32) -> &mut Self {
        self.0.extend(value.to_le_bytes());
        self
    }
    pub(super) fn done(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

fn status_writer(kind: u8) -> Writer {
    let mut writer = Writer::new(CLIENT_STATUS);
    writer.u8(kind);
    writer
}

/// `inventory_item_descr` in the given serialize mode.
fn write_item(writer: &mut Writer, item: &Item, with_amount: bool) {
    writer
        .u16(item.dict_id)
        .u32(item.item_id)
        .u16(item.condition_or_stack);
    if with_amount {
        writer.u32(item.amount);
    }
    let modifications = &item.modifications[..item.modifications.len().min(MAX_MODIFICATIONS)];
    writer.u8(modifications.len() as u8);
    for &modification in modifications {
        writer.u32(modification);
    }
}

/// `30 <state>`: the lobby has signed the client in. Send `Lobby`'s 0.
pub fn connection_successful() -> Vec<u8> {
    Writer::new(CONNECTION_SUCCESSFUL).u8(0).done()
}

/// `38`: mandatory. The third value is the minutes to maintenance: under
/// 60 the client shows a warning with the minutes, at 30 or less it locks
/// Play, and 0 quits the game (it started the launcher, which srp's client
/// zip lacks).
/// No maintenance is planned: a day, well over the 30 minutes below which
/// the client locks Play.
pub const MINUTES_TO_MAINTENANCE: u32 = 1440;

pub fn online_stats(
    players_online: u32,
    games_online: u32,
    minutes_to_maintenance: u32,
) -> Vec<u8> {
    Writer::new(ONLINE_STATS)
        .u32(players_online)
        .u32(games_online)
        .u32(minutes_to_maintenance)
        .done()
}

/// `36 00`.
pub fn client_state(state: ClientState) -> Vec<u8> {
    let mut writer = status_writer(status::CLIENT_STATE);
    match state {
        ClientState::Lobby => {
            writer.u8(0);
        }
        ClientState::InQueue {
            order_id,
            place,
            queue_total,
            current_wait_seconds,
            average_wait_seconds,
        } => {
            writer
                .u8(1)
                .u32(order_id)
                .u32(place)
                .u32(queue_total)
                .u32(current_wait_seconds)
                .u32(average_wait_seconds);
        }
        ClientState::InMatch {
            order_id,
            match_id,
            team,
        } => {
            writer.u8(2).u32(order_id).u32(match_id).u8(team);
        }
    }
    writer.done()
}

/// `36 01`: the profile list. The client answers with `21 02` for each.
pub fn profiles(account: &Account) -> Vec<u8> {
    let mut writer = status_writer(status::PROFILES);
    let listed = &account.profiles[..account.profiles.len().min(super::account::MAX_PROFILES)];
    writer.u8(listed.len() as u8);
    for profile in listed {
        writer
            .u32(profile.id)
            .str(&profile.name, MAX_NAME_LEN)
            .u8(profile.autobuy.into())
            .u8(profile.level)
            .u32(profile.exp_total.max(profile.exp_level_start))
            .u32(profile.exp_level_start)
            .u32(profile.exp_next_level)
            .u32(profile.revision);
    }
    writer.done()
}

/// `player_profile::deserialize_static`, as 0x36 02 and 0x36 10 carry it
/// with team 0, and 0x51 with the player's team and name.
pub(super) fn write_profile_static(writer: &mut Writer, profile: &Profile, team: u8, name: &str) {
    // Team, then the flag the client overwrites.
    writer.u8(team).u8(0).str(name, MAX_NAME_LEN);
    // New-player damage protection: the factor is max(0, 1 - 0.1 * value),
    // so 10 or more is none [spec: P].
    writer.u32(10);
    let mask = profile
        .slots
        .iter()
        .enumerate()
        .filter(|(_, item)| item.is_some())
        .fold(0_u32, |mask, (slot, _)| mask | 1 << slot);
    writer.u32(mask);
    for (slot, item) in profile.slots.iter().enumerate().take(PROFILE_SLOTS) {
        if let Some(item) = item {
            write_item(writer, item, slot_carries_amount(slot));
        }
    }
    // The modifiers its skills give: a mask, then the set ones in bit order.
    // The player reads them only from here, in a match too.
    let modifiers = SkillsTree::shared().modifiers(&profile.skills);
    let mask = (0..MODIFIERS)
        .filter(|&i| modifiers[i] != 0.0)
        .fold(0_u32, |mask, i| mask | 1 << i);
    writer.u32(mask);
    for modifier in modifiers.into_iter().filter(|&value| value != 0.0) {
        writer.f32(modifier);
    }
}

/// `36 02`: one profile's slots. The inventory (0x36 03) must already hold
/// every item it names, or the client's paper doll stays empty.
pub fn profile_contents(profile: &Profile) -> Vec<u8> {
    let mut writer = status_writer(status::PROFILE_CONTENTS);
    writer.u32(profile.id);
    write_profile_static(&mut writer, profile, 0, &profile.name);
    writer.u32(profile.revision);
    writer.done()
}

/// `36 10`: a squad mate's profile, for the client to show its character.
/// `profile_id` is the id the squad status (0x3A) gave it.
pub fn squad_member_profile(profile_id: u32, profile: &Profile) -> Vec<u8> {
    let mut writer = status_writer(status::SQUAD_MEMBER_PROFILE);
    writer.u32(profile_id);
    write_profile_static(&mut writer, profile, 0, &profile.name);
    writer.u32(profile.revision);
    writer.done()
}

/// One member in a squad status (0x3A).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SquadMember {
    /// The id the client sends back to drop the member (`2A 03`).
    pub account_id: u32,
    /// The member's profile. For the receiving player's own entry, one of its
    /// own profile ids. For a mate, an id that is none of them: the client
    /// then asks for it with `21 10` and shows the mate's character.
    pub profile_id: u32,
    /// Compared with the revision of the character the client shows; another
    /// value makes it ask again.
    pub profile_revision: u32,
    pub name: String,
    pub online: bool,
    pub ready: bool,
}

/// The most squad members the client reads (`read_squad_info`).
pub const MAX_SQUAD_MEMBERS: usize = 3;

/// `3A`: the player's squad (spec §2.7), or none for `squad_id` 0.
/// `commander` puts the client in commander mode: its Play is then unlocked
/// only when all the other members are ready.
pub fn squad_status(
    squad_id: u32,
    members: &[SquadMember],
    commander: bool,
    revision: u32,
) -> Vec<u8> {
    let mut writer = Writer::new(SQUAD_STATUS);
    writer.u32(squad_id);
    if squad_id == 0 {
        return writer.done();
    }
    let members = &members[..members.len().min(MAX_SQUAD_MEMBERS)];
    // The byte after the count has no known use [spec: P].
    writer.u8(members.len() as u8).u8(0);
    for member in members {
        writer
            .u32(member.account_id)
            .u32(member.profile_id)
            .u32(member.profile_revision)
            .str(&member.name, MAX_NAME_LEN)
            .u8(member.online.into())
            .u8(member.ready.into());
    }
    writer.u8(commander.into()).u32(revision);
    writer.done()
}

/// `36 03`: everything the account owns.
pub fn inventory(account: &Account) -> Vec<u8> {
    let mut writer = status_writer(status::INVENTORY);
    writer.u32(account.inventory.len() as u32);
    for item in &account.inventory {
        write_item(&mut writer, item, true);
    }
    writer.done()
}

/// `36 04`: what each faction sells, for how much (`prices::PRICES`) and from
/// which reputation rank (`prices::rank`). The shop shows the price of a
/// clip of ammo (`item_params::shop_unit`), so ammo's is its rounds' price.
pub fn prices() -> Vec<u8> {
    let mut writer = status_writer(status::PRICES);
    let entries: Vec<(u16, u32, u8, u8)> = PRICES
        .iter()
        .flat_map(|price| {
            price.sellers.iter().map(|&seller| {
                let unit = super::item_params::shop_unit(price.dict_id);
                let rank = crate::prices::rank(price.dict_id, seller);
                (price.dict_id, u32::from(price.cost) * unit, rank, seller)
            })
        })
        .collect();
    writer.u16(entries.len() as u16);
    for (dict_id, cost, rank, seller) in entries {
        writer.u32(cost).u16(dict_id).u8(rank).u8(seller);
    }
    writer.done()
}

/// `36 05`. Send after 0x36 0A, which gives the money bar its nickname.
pub fn money(account: &Account) -> Vec<u8> {
    status_writer(status::MONEY)
        .u32(account.generic_money)
        .u32(account.premium_money)
        .done()
}

/// `36 06`: one profile's skills and perks. Send after 0x36 01, or the client
/// misreads it.
pub fn skills(profile: &Profile) -> Vec<u8> {
    let mut writer = status_writer(status::SKILLS);
    writer
        .u32(profile.id)
        .u8(profile.level)
        .u32(profile.exp_total.max(profile.exp_level_start))
        .u32(profile.exp_next_level)
        .u32(profile.exp_level_start);
    writer.u8(profile.skills.len() as u8);
    for &(skill_id, points) in &profile.skills {
        writer.u8(skill_id).u8(points);
    }
    writer.u8(profile.perks.len() as u8);
    for &perk in &profile.perks {
        writer.u8(perk);
    }
    writer.done()
}

/// `36 08`: the costs of a skill reroll and of a new profile; the third
/// value is never read.
pub fn service_prices(reroll: u32, add_profile: u32) -> Vec<u8> {
    status_writer(status::SERVICE_PRICES)
        .u32(reroll)
        .u32(add_profile)
        .u32(0)
        .done()
}

/// `36 09`. Send after 0x36 12.
pub fn reputations(account: &Account) -> Vec<u8> {
    let mut writer = status_writer(status::REPUTATIONS);
    writer.u8(account.reputations.len() as u8);
    for &(faction, points) in &account.reputations {
        writer.u32(faction.into()).u16(0).u16(points);
    }
    writer.done()
}

/// `36 0A`: the account's nickname, and how many players the ranking has.
pub fn base_info(account: &Account, ranked_players: u32) -> Vec<u8> {
    status_writer(status::BASE_INFO)
        .u32(ranked_players)
        .str(&account.name, MAX_NAME_LEN)
        .done()
}

/// `36 0B`: the account's own row of the ranking page: its place (from 1),
/// rating and rating before its last match (`set_player_elo_rating`
/// 0x177BBF shows the difference).
pub fn player_elo(place: u32, account: &Account) -> Vec<u8> {
    status_writer(status::PLAYER_ELO)
        .u32(place)
        .u16(account.rating)
        .u16(account.rating_before)
        .done()
}

/// A row of the ranking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ranked {
    pub account_id: u32,
    pub name: String,
    pub rating: u16,
    pub rating_before: u16,
}

/// `36 0C`: the ranking from place `start + 1` on, as `21 0C <start>
/// <count>` asks: {name, place, rating, change}. The page files each row
/// under the index it asked for, in order (`PlayerRanking.fillPlayers`).
pub fn elo_list(start: u32, rows: &[Ranked]) -> Vec<u8> {
    let mut writer = status_writer(status::ELO_LIST);
    writer.u8(rows.len().min(u8::MAX.into()) as u8);
    for (index, row) in rows.iter().take(u8::MAX.into()).enumerate() {
        let change = i32::from(row.rating) - i32::from(row.rating_before);
        writer
            .str(&row.name, MAX_NAME_LEN)
            .u32(start + index as u32 + 1)
            .u16(row.rating)
            .u16(change.clamp(i16::MIN.into(), i16::MAX.into()) as i16 as u16);
    }
    writer.done()
}

/// `36 0D`: the account's quests at `now` (Unix seconds), each {instance,
/// quest id, games won towards it, minutes left, 1 active or 0 a proposal,
/// 3 bytes of padding}.
pub fn quests(account: &Account, now: u64) -> Vec<u8> {
    let mut writer = status_writer(status::QUESTS);
    writer.u32(account.quests.len() as u32);
    for quest in &account.quests {
        let minutes = quest.expires_at.saturating_sub(now).div_ceil(60);
        writer
            .u32(quest.instance_id)
            .u32(quest.quest_id)
            .u32(quest.games)
            .u32(minutes.min(i32::MAX as u64) as u32)
            .u8(quest.active.into())
            .u8(0)
            .u8(0)
            .u8(0);
    }
    writer.done()
}

/// `36 0F`: the premium account's hours left, if any, and each faction
/// loyalty above 0. Each entry: `u8 type` (0 premium, 1..=4 a faction's
/// loyalty), `u8` pad, `u32 a` (premium hours), `u32 b` (loyalty %).
pub fn services(account: &Account, now: u64) -> Vec<u8> {
    let mut writer = status_writer(status::SERVICES);
    let premium = premium::is_active(account, now);
    // Type 1..=4 is the faction; the client keeps the low byte of `b` as
    // the "+N%" its shop shows, and none for a faction left out.
    let loyal: Vec<(u8, u8)> = (1..=4)
        .map(|faction| (faction, loyalty::of(account, faction)))
        .filter(|&(_, percent)| percent > 0)
        .collect();
    writer.u8(u8::from(premium) + loyal.len() as u8);
    if premium {
        writer
            .u8(0)
            .u8(0)
            .u32(premium::hours_left(account, now))
            .u32(0);
    }
    for (faction, percent) in loyal {
        writer.u8(faction).u8(0).u32(0).u32(percent.into());
    }
    writer.done()
}

/// The longest label and description the client keeps.
const SHOP_TEXT_LEN: usize = 31;
/// `shop_items_container.shelf`: the only one the client shows.
const PREMIUM_SHELF: u8 = 0;
/// `money_type`: gold.
const PREMIUM_MONEY: u8 = 1;

/// `36 11`: the premium store's offers, each with no shop items.
pub fn shop_items() -> Vec<u8> {
    let mut writer = status_writer(status::SHOP_ITEMS);
    writer.u32(premium::OFFERS.len() as u32);
    for offer in &premium::OFFERS {
        writer
            .u32(offer.id)
            .u8(PREMIUM_SHELF)
            .u32(offer.price)
            .str(offer.label, SHOP_TEXT_LEN)
            .str(offer.description, SHOP_TEXT_LEN)
            .u8(0)
            .u8(PREMIUM_MONEY);
    }
    writer.done()
}

/// `34 24 02`: a premium offer bought; the client asks for its money and
/// services.
pub fn premium_bought() -> Vec<u8> {
    Writer::new(OPERATION_PERMITTED).u8(op::SHOP).u8(2).done()
}

/// `36 12`: bit f-1 unlocks faction f.
pub fn unlocked_factions(mask: u8) -> Vec<u8> {
    status_writer(status::UNLOCKED_FACTIONS).u8(mask).done()
}

/// The client messages a 0x34 or 0x35 answers.
pub mod op {
    pub const PLAY: u8 = 0x20;
    pub const MOVE_ITEMS: u8 = 0x23;
    pub const SHOP: u8 = 0x24;
    pub const SKILLS: u8 = 0x25;
    pub const PROFILES: u8 = 0x29;
    pub const SQUAD: u8 = 0x2A;
    pub const QUEST: u8 = 0x2B;
}

/// `35 <op> 00 <text>`: the client shows the text (a localization key) in a
/// message box for 0x20 and 0x23..0x29, and says nothing for 0x2A.
pub fn operation_denied(op: u8, text: &str) -> Vec<u8> {
    Writer::new(OPERATION_DENIED)
        .u8(op)
        .u8(0)
        .str(text, 255)
        .done()
}

/// `34 24 00`: bought. Each entry adds `value` to the stack with that
/// `item_id` if the client has it, or is a new item with that condition or
/// count.
pub fn bought(items: &[(u16, u32, u32)]) -> Vec<u8> {
    let mut writer = Writer::new(OPERATION_PERMITTED);
    writer.u8(op::SHOP).u8(0).u8(items.len() as u8);
    for &(dict_id, item_id, value) in items {
        writer.u16(dict_id).u32(item_id).u32(value);
    }
    writer.done()
}

/// `34 24 01`: sold. The client takes `amount` off a stack (or drops the
/// item), then asks for each listed profile again.
pub fn sold(item_id: u32, amount: u32, profiles: &[u32]) -> Vec<u8> {
    let mut writer = Writer::new(OPERATION_PERMITTED);
    writer.u8(op::SHOP).u8(1).u32(item_id).u32(amount);
    let profiles = &profiles[..profiles.len().min(255)];
    writer.u32(profiles.len() as u32);
    for &profile in profiles {
        writer.u32(profile);
    }
    writer.done()
}

/// `34 2B`: a quest was changed; the client asks for its quests again.
pub fn quest_changed() -> Vec<u8> {
    Writer::new(OPERATION_PERMITTED).u8(op::QUEST).done()
}

/// `34 29 03`: the autobuy box of a profile.
pub fn autobuy_set(profile_id: u32, on: bool) -> Vec<u8> {
    Writer::new(OPERATION_PERMITTED)
        .u8(op::PROFILES)
        .u8(3)
        .u32(profile_id)
        .u8(on.into())
        .done()
}

/// `33`: go to the match server. Host at most 63 bytes.
pub fn connect_to_match_server(host: &str, port: u16, match_id: u32, team: u8) -> Vec<u8> {
    Writer::new(CONNECT_TO_MATCH_SERVER)
        .str(host, MAX_NAME_LEN)
        .u16(port)
        .u32(match_id)
        .u8(team)
        .done()
}

/// Everything the client needs after `30`, in an order that meets each of
/// its reading dependencies (spec §6).
pub fn sign_in_push(
    account: &Account,
    now: u64,
    players_online: u32,
    games_online: u32,
    minutes_to_maintenance: u32,
    (ranked, place): (u32, u32),
) -> Vec<Vec<u8>> {
    let mut push = vec![
        connection_successful(),
        // The ranking page's player count, then the account's row.
        base_info(account, ranked),
        player_elo(place, account),
        money(account),
        // Before the reputations: the shop makes every item buyable when
        // the prices come (`GangShop.setupShopItems`), and locks those above
        // the rank only when the reputations do.
        prices(),
        unlocked_factions(0x0F),
        reputations(account),
        service_prices(REROLL_COST, ADD_PROFILE_COST),
        services(account, now),
        shop_items(),
        inventory(account),
        profiles(account),
    ];
    if let Some(active) = account.profiles.first() {
        push.push(skills(active));
    }
    push.extend([
        quests(account, now),
        client_state(ClientState::Lobby),
        online_stats(players_online, games_online, minutes_to_maintenance),
    ]);
    push
}

#[cfg(test)]
mod squad_tests {
    use super::*;

    #[test]
    fn no_squad_is_its_id_alone() {
        assert_eq!(squad_status(0, &[], true, 7), [0x3A, 0, 0, 0, 0]);
    }

    #[test]
    fn a_squad_status_lists_its_members_then_the_commander_flag() {
        let member = SquadMember {
            account_id: 0x0102_0304,
            profile_id: 5,
            profile_revision: 6,
            name: "ab".into(),
            online: true,
            ready: false,
        };
        let mut expected = vec![0x3A, 9, 0, 0, 0, 1, 0];
        expected.extend([4, 3, 2, 1, 5, 0, 0, 0, 6, 0, 0, 0, 2, b'a', b'b', 1, 0]);
        expected.extend([1, 8, 0, 0, 0]);
        assert_eq!(squad_status(9, &[member], true, 8), expected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::Dictionary;
    use crate::v021d::account::STARTING_ROUNDS;

    fn hex(bytes: &str) -> Vec<u8> {
        bytes
            .split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect()
    }

    /// The spec's account: loadout A, its items numbered 1000 + dict id.
    /// The account the spec's dumps were taken with: the test loadout as the
    /// game lists it, with its Renaissance torso (58) and 120 rounds per
    /// ammo kind, which srp's starter loadout lightens.
    fn spec_account() -> Account {
        let mut account = Account::with_test_loadout("Player", &Dictionary::v021d());
        let as_dumped = |item: &mut Item| {
            if item.dict_id == 31 {
                item.dict_id = 58;
            }
            if item.condition_or_stack == STARTING_ROUNDS {
                item.condition_or_stack = 120;
                item.amount = 120;
            }
        };
        account.inventory.iter_mut().for_each(as_dumped);
        account.profiles[0]
            .slots
            .iter_mut()
            .flatten()
            .for_each(as_dumped);
        account.generic_money = 50_000;
        account.premium_money = 0;
        let renumber = |item: &mut Item| item.item_id = 1000 + u32::from(item.dict_id);
        account.inventory.iter_mut().for_each(renumber);
        let profile = &mut account.profiles[0];
        profile.name = "Default".into();
        profile.slots.iter_mut().flatten().for_each(renumber);
        account
    }

    #[test]
    fn fixed_messages_match_the_spec() {
        let account = spec_account();
        assert_eq!(connection_successful(), hex("30 00"));
        assert_eq!(
            base_info(&account, 1),
            hex("36 0A 01 00 00 00 06 50 6C 61 79 65 72")
        );
        assert_eq!(money(&account), hex("36 05 50 C3 00 00 00 00 00 00"));
        assert_eq!(unlocked_factions(0x0F), hex("36 12 0F"));
        assert_eq!(
            reputations(&account),
            hex("36 09 04 01 00 00 00 00 00 00 00 02 00 00 00 00 00 00 00
                 03 00 00 00 00 00 00 00 04 00 00 00 00 00 00 00")
        );
        assert_eq!(
            service_prices(100, 1000),
            hex("36 08 64 00 00 00 E8 03 00 00 00 00 00 00")
        );
        let mut account = spec_account();
        assert_eq!(services(&account, 0), hex("36 0F 00"));
        account.premium_until = 3600 * 30;
        assert_eq!(
            services(&account, 3600 * 5 + 1),
            hex("36 0F 01 00 00 19 00 00 00 00 00 00 00")
        );
        account.premium_until = 0;
        // Loyalty: after premium, each faction above 0, `b` its percent.
        account.loyalty = [0, 10, 0, 25];
        assert_eq!(
            services(&account, 0),
            hex("36 0F 02 02 00 00 00 00 00 0A 00 00 00
                       04 00 00 00 00 00 19 00 00 00")
        );
        account.loyalty = [0; 4];
        let store = shop_items();
        assert_eq!(&store[..6], &hex("36 11 06 00 00 00")[..]);
        assert_eq!(
            &store[6..6 + 47],
            &hex("01 00 00 00 00 FA 00 00 00
                  0E 73 74 5F 70 72 65 6D 69 75 6D 5F 64 61 79
                  14 73 74 5F 70 72 65 6D 69 75 6D 5F 64 61 79 5F 64 65 73 63 72
                  00 01")[..]
        );
        assert_eq!(premium_bought(), hex("34 24 02"));
        assert_eq!(quests(&account, 0), hex("36 0D 00 00 00 00"));
        account.quests.push(crate::v021d::quests::QuestInstance {
            instance_id: 7,
            quest_id: 10100,
            games: 1,
            expires_at: 3600,
            active: true,
        });
        assert_eq!(
            quests(&account, 59),
            hex("36 0D 01 00 00 00 07 00 00 00 74 27 00 00 01 00 00 00 3C 00 00 00 01 00 00 00")
        );
        assert_eq!(client_state(ClientState::Lobby), hex("36 00 00"));
        assert_eq!(
            online_stats(1, 0, 1440),
            hex("38 01 00 00 00 00 00 00 00 A0 05 00 00")
        );
        // The minutes go out as they are: 45 shows the warning, 0 quits.
        assert_eq!(
            online_stats(1, 0, 45),
            hex("38 01 00 00 00 00 00 00 00 2D 00 00 00")
        );
    }

    #[test]
    fn profile_list_and_skills_match_the_spec() {
        let account = spec_account();
        assert_eq!(
            profiles(&account),
            hex("36 01 01 01 00 00 00 07 44 65 66 61 75 6C 74 00 01
                 00 00 00 00 00 00 00 00 F4 01 00 00 01 00 00 00")
        );
        assert_eq!(
            skills(&account.profiles[0]),
            hex("36 06 01 00 00 00 01 00 00 00 00 F4 01 00 00 00 00 00 00 00 00")
        );
    }

    /// The spec's dumps load 60 of 120 rounds into each ammo slot.
    #[test]
    fn inventory_and_profile_match_the_spec_layout() {
        let mut account = spec_account();
        for item in account.profiles[0].slots.iter_mut().flatten() {
            if item.condition_or_stack == 120 {
                item.condition_or_stack = 60;
            }
        }
        assert_eq!(
            inventory(&account),
            hex("36 03 0D 00 00 00
                 3C 00 24 04 00 00 64 00 01 00 00 00 00
                 2B 00 13 04 00 00 64 00 01 00 00 00 00
                 3A 00 22 04 00 00 64 00 01 00 00 00 00
                 31 00 19 04 00 00 64 00 01 00 00 00 00
                 3B 00 23 04 00 00 64 00 01 00 00 00 00
                 29 00 11 04 00 00 64 00 01 00 00 00 00
                 3E 00 26 04 00 00 64 00 01 00 00 00 00
                 7E 00 66 04 00 00 64 00 01 00 00 00 00
                 46 00 2E 04 00 00 78 00 78 00 00 00 00
                 67 00 4F 04 00 00 78 00 78 00 00 00 00
                 7C 00 64 04 00 00 64 00 01 00 00 00 00
                 16 00 FE 03 00 00 78 00 78 00 00 00 00
                 48 00 30 04 00 00 78 00 78 00 00 00 00")
        );
        let contents = profile_contents(&account.profiles[0]);
        let expected = hex(
            "36 02 01 00 00 00 00 00 07 44 65 66 61 75 6C 74 0A 00 00 00 FF 1F 00 00
             3C 00 24 04 00 00 64 00 00
             2B 00 13 04 00 00 64 00 00
             3A 00 22 04 00 00 64 00 00
             31 00 19 04 00 00 64 00 00
             3B 00 23 04 00 00 64 00 00
             29 00 11 04 00 00 64 00 00
             3E 00 26 04 00 00 64 00 00
             7E 00 66 04 00 00 64 00 00
             46 00 2E 04 00 00 3C 00 78 00 00 00 00
             67 00 4F 04 00 00 3C 00 78 00 00 00 00
             7C 00 64 04 00 00 64 00 00
             16 00 FE 03 00 00 3C 00 78 00 00 00 00
             48 00 30 04 00 00 3C 00 78 00 00 00 00
             00 00 00 00 01 00 00 00",
        );
        assert_eq!(contents, expected);

        // With skills, their modifiers: physical training 2 points gives
        // stamina regeneration +10 %, movement speed +1 %, carried weight
        // +2 kg (bits 3, 4, 5), in bit order before the revision.
        let mut skilled = account.profiles[0].clone();
        skilled.skills = vec![(2, 2)];
        let contents = profile_contents(&skilled);
        let tail = [
            &0x38_u32.to_le_bytes()[..],
            &(10.0_f32 / 100.0).to_le_bytes(),
            &(1.0_f32 / 100.0).to_le_bytes(),
            &2.0_f32.to_le_bytes(),
            &1_u32.to_le_bytes(),
        ]
        .concat();
        assert_eq!(
            contents[..expected.len() - 8],
            expected[..expected.len() - 8]
        );
        assert_eq!(contents[expected.len() - 8..], tail);
    }

    #[test]
    fn replies() {
        assert_eq!(
            operation_denied(op::SHOP, "st_cannot_buy_item"),
            [&[0x35, 0x24, 0][..], &[18], b"st_cannot_buy_item"].concat()
        );
        assert_eq!(
            bought(&[(13, 7, 100)]),
            hex("34 24 00 01 0D 00 07 00 00 00 64 00 00 00")
        );
        assert_eq!(
            sold(7, 1, &[1]),
            hex("34 24 01 07 00 00 00 01 00 00 00 01 00 00 00 01 00 00 00")
        );
        assert_eq!(autobuy_set(1, true), hex("34 29 03 01 00 00 00 01"));
    }

    #[test]
    fn prices_list_one_entry_per_seller() {
        let bytes = prices();
        let count = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
        assert_eq!(bytes.len(), 4 + 8 * count);
        let seller_entries: usize = PRICES.iter().map(|price| price.sellers.len()).sum();
        assert_eq!(count, seller_entries);
        // The AKS-74U: 10500, Black Market, from its rank 5 (Elite); the
        // TT-33, 400, from any rank; buckshot, a clip of 20 at 1 a round.
        let has = |entry: &str| bytes[4..].chunks(8).any(|chunk| chunk == hex(entry));
        assert!(has("04 29 00 00 0D 00 05 02"));
        assert!(has("90 01 00 00 12 00 00 01"));
        assert!(has("14 00 00 00 16 00 00 01"));
    }

    #[test]
    fn ranking_rows_and_own_row() {
        let mut account = spec_account();
        account.rating = 1016;
        assert_eq!(
            player_elo(3, &account),
            hex("36 0B 03 00 00 00 F8 03 E8 03")
        );
        let rows = [Ranked {
            account_id: 1,
            name: "Ab".into(),
            rating: 984,
            rating_before: 1000,
        }];
        assert_eq!(
            elo_list(4, &rows),
            hex("36 0C 01 02 41 62 05 00 00 00 D8 03 F0 FF")
        );
        assert_eq!(elo_list(0, &[]), hex("36 0C 00"));
    }

    #[test]
    fn sign_in_push_order() {
        let account = spec_account();
        let push = sign_in_push(&account, 0, 1, 0, MINUTES_TO_MAINTENANCE, (1, 1));
        let kinds: Vec<(u8, Option<u8>)> = push
            .iter()
            .map(|message| {
                (
                    message[0],
                    (message[0] == CLIENT_STATUS).then(|| message[1]),
                )
            })
            .collect();
        let at = |kind| {
            kinds
                .iter()
                .position(|&k| k == (CLIENT_STATUS, Some(kind)))
                .unwrap()
        };
        assert_eq!(kinds[0], (CONNECTION_SUCCESSFUL, None));
        assert!(at(status::BASE_INFO) < at(status::MONEY));
        assert!(at(status::BASE_INFO) < at(status::PLAYER_ELO));
        assert!(at(status::UNLOCKED_FACTIONS) < at(status::REPUTATIONS));
        // The shop locks items only when the reputations follow the prices.
        assert!(at(status::PRICES) < at(status::REPUTATIONS));
        assert!(at(status::INVENTORY) < at(status::PROFILES));
        assert!(at(status::PROFILES) < at(status::SKILLS));
        assert_eq!(kinds.last(), Some(&(ONLINE_STATS, None)));
    }
}
