//! The lobby tutorial of the 0.26g client, as far as the server has a part
//! in it (`docs/notes/premium.md` §5). The strict client model
//! ([`super::client_model::Client`]) holds one and feeds it every message.
//!
//! The tutorial is a state machine in the exe (states 0x2A4240..0x2A5320).
//! Entering a state clears **every** UI handler of the lobby (0x281580: the
//! 112 slots at lobby_menu+0x1C00) and switches on only the ones the state
//! names; `root.show_slide` puts a sheet over the whole stage and makes the
//! highlight layer the modal clip (`LobbyMenu.show_slide`:
//! `FocusManager.setModalClip`). So in a tutorial state **only what the
//! state highlights can be clicked or dragged**, besides "Skip tutorial".
//!
//! The states this model follows (PROVEN, the exe):
//!
//! | state | entered | highlights | leaves on |
//! |---|---|---|---|
//! | `disabled_state` | at start | - | `9D` with `show` (0x283190) |
//! | `tutorial_state_click_inventory` 0x2A44E0 | `9D`: bit 0 of `a`, `b` == 0 | the inventory tab | `subsystem_activated` |
//! | `tutorial_state_choose_wpn` 0x2A45D0 | | **nothing yet** | `handle_drag_start` |
//! | `tutorial_state_drag_wpn` 0x2A4730 | | weapon slot 1 (`paperDoll.getSlotAt(7)`) | `relocate_item` of dict 106 or 107 (filter 0x2A3F70) |
//! | `tutorial_state_click_ammo` 0x2A4940 | | | (not followed further here) |
//!
//! **The step the server has a part in.** `tutorial_state_choose_wpn` sends
//! `49 02 <active kit>` and `46 0A`, and registers 0x283470 for the lobby's
//! own event `inventory_content_filled` (slot 63, lobby_menu+0x23E0). That
//! event is raised by the exe and by nothing else, at the end of
//! `setup_inventory_data` (0x288520 @0x2886F5), which runs when an `89`
//! arrives (0x274C45 -> 0x21D720 case 0x89 -> 0x21D822). 0x283470 calls
//! `root.highlight_inventory_items([106, 107])`: the PPSh-41 and the Mosin
//! carbine, by dict id (`InventoryList.filterItems`, `getItemRenderer`).
//! **Without an `89` after `46 0A` nothing is highlighted and no weapon can
//! be dragged**: the first live session's fault (2026-10-12).
//!
//! `highlight_inventory_items` highlights `getItemRenderer(dict)` for each
//! of the two; for a dict the inventory lacks that is `null`, and
//! `highlight_element(null)` reads `target.parent` [LIKELY: the script
//! error ends the call, the other weapon's highlight with it]. The model
//! therefore refuses an inventory without both.

use super::ServerId;
use super::client_model::{Decoded, ModelError};

/// The weapons the "choose a weapon" step highlights, by dict id: the
/// PPSh-41 (106) and the Mosin carbine (107). Constants of the exe
/// (0x283499, 0x2834A9; again at 0x2A47C9 for the drop filter).
pub const CHOICE: [u16; 2] = [0x6A, 0x6B];
/// The slot the "drag the weapon" step highlights: weapon slot 1
/// (`root.player_profile.paperDoll.getSlotAt(7)`, 0x2A478D).
pub const WEAPON_SLOT: u8 = 7;

/// The tutorial's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// `disabled_state`: no `9D` with `show` came yet.
    Disabled,
    /// `default_state`: no tutorial (done, skipped, or over).
    Default,
    ChooseNickname,
    ClickInventory,
    ChooseWeapon,
    DragWeapon,
    ClickAmmo,
    /// After the first match (`b` != 0).
    EquipDrop,
}

/// Why the player can not do a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stuck {
    /// The tutorial is in another state.
    NotIn(State),
    /// The item is under the tutorial's sheet: not highlighted.
    NotHighlighted(u16),
}

/// The tutorial of one client.
#[derive(Clone, Debug)]
pub struct Tutorial {
    state: State,
    /// The dict ids `highlight_inventory_items` was called with last.
    highlighted: Vec<u16>,
}

impl Default for Tutorial {
    fn default() -> Self {
        Self {
            state: State::Disabled,
            highlighted: Vec::new(),
        }
    }
}

impl Tutorial {
    pub fn state(&self) -> State {
        self.state
    }

    /// The inventory items the player can drag now, by dict id. Empty in
    /// `tutorial_state_choose_wpn` until an `89` came.
    pub fn highlighted(&self) -> &[u16] {
        &self.highlighted
    }

    /// Whether the tutorial's sheet covers the lobby.
    pub fn is_modal(&self) -> bool {
        !matches!(self.state, State::Disabled | State::Default)
    }

    fn enter(&mut self, state: State) {
        // 0x2A3EA0: `root.clean_highlight`.
        self.highlighted.clear();
        self.state = state;
    }

    /// A message the client read.
    pub(super) fn received(&mut self, decoded: &Decoded) -> Result<(), ModelError> {
        match decoded {
            // 0x283190: only out of the disabled state.
            Decoded::Tutorial { show: true, a, b } if self.state == State::Disabled => {
                let state = if a & 0x22 != 0 {
                    State::Default
                } else if a & 1 == 0 {
                    State::ChooseNickname
                } else if *b == 0 {
                    State::ClickInventory
                } else {
                    State::EquipDrop
                };
                self.enter(state);
            }
            // `inventory_content_filled` -> 0x283470.
            Decoded::Inventory { items, .. } if self.state == State::ChooseWeapon => {
                for dict_id in CHOICE {
                    if !items.iter().any(|item| item.dict_id == dict_id) {
                        return Err(ModelError::Bad(
                            ServerId::Inventory,
                            "the inventory lacks a weapon the tutorial highlights",
                        ));
                    }
                }
                self.highlighted = CHOICE.to_vec();
            }
            _ => {}
        }
        Ok(())
    }

    /// The player opened the inventory (`subsystem_activated`). The
    /// requests the client sends: `49 02 <active kit>` and `46 0A`.
    pub fn inventory_opened(&mut self, active_kit: u32) -> Result<Vec<(u8, Vec<u8>)>, Stuck> {
        if self.state != State::ClickInventory {
            return Err(Stuck::NotIn(State::ClickInventory));
        }
        self.enter(State::ChooseWeapon);
        let mut active = vec![0x02];
        active.extend_from_slice(&active_kit.to_le_bytes());
        Ok(vec![(0x49, active), (0x46, vec![0x0A])])
    }

    /// The player starts to drag an inventory item (`handle_drag_start`).
    pub fn drag_started(&mut self, dict_id: u16) -> Result<(), Stuck> {
        if self.state != State::ChooseWeapon {
            return Err(Stuck::NotIn(State::ChooseWeapon));
        }
        if !self.highlighted.contains(&dict_id) {
            return Err(Stuck::NotHighlighted(dict_id));
        }
        // 0x2A4730 keeps the items' highlight (it has its own copy of the
        // two dict ids for the drop).
        self.state = State::DragWeapon;
        Ok(())
    }

    /// The player dropped the dragged weapon on weapon slot 1
    /// (`relocate_item`, which sends `46 00`).
    pub fn dropped_on_weapon_slot(&mut self, dict_id: u16) -> Result<(), Stuck> {
        if self.state != State::DragWeapon {
            return Err(Stuck::NotIn(State::DragWeapon));
        }
        if !CHOICE.contains(&dict_id) {
            return Err(Stuck::NotHighlighted(dict_id));
        }
        self.enter(State::ClickAmmo);
        Ok(())
    }

    /// "Skip tutorial" (`skip_tutorial` 0x28CCB0): the request `4B 02`.
    /// The sheet and the handlers' lock go (`default_state` 0x2A4240:
    /// `root.close_slide`, every handler back), after the faction choice
    /// for an account with no quest faction (`state_choose_faction`).
    pub fn skipped(&mut self) -> (u8, Vec<u8>) {
        self.enter(State::Default);
        (0x4B, vec![0x02])
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_model::Client;
    use super::super::{self as lobby, ItemDescr};
    use super::*;

    fn inventory(dicts: &[u16]) -> lobby::ServerMessage {
        let items: Vec<ItemDescr> = dicts
            .iter()
            .zip(1..)
            .map(|(&dict_id, item_id)| ItemDescr::plain(item_id, dict_id, 10_000))
            .collect();
        lobby::inventory(&items).unwrap()
    }

    #[test]
    fn the_state_a_9d_starts() {
        for (a, b, state) in [
            (0, 0, State::ChooseNickname),
            (1, 0, State::ClickInventory),
            (1, 1, State::EquipDrop),
            (3, 0, State::Default),
            (0x21, 0, State::Default),
        ] {
            let mut client = Client::signed_in();
            client.receive(&lobby::tutorial(true, a, b)).unwrap();
            assert_eq!(client.tutorial().state(), state, "a {a:#x} b {b}");
        }
        // Not shown: disabled, as srp had it before.
        let mut client = Client::signed_in();
        client.receive(&lobby::tutorial(false, 1, 0)).unwrap();
        assert_eq!(client.tutorial().state(), State::Disabled);
        assert!(!client.tutorial().is_modal());
    }

    #[test]
    fn no_weapon_can_be_dragged_until_the_inventory_comes_again() {
        let mut client = Client::signed_in();
        // Sign-in: the inventory, then the tutorial.
        client.receive(&inventory(&[18, 106, 107])).unwrap();
        client.receive(&lobby::tutorial(true, 1, 0)).unwrap();
        // Nothing but the inventory tab until it is opened.
        assert_eq!(
            client.tutorial_mut().drag_started(106),
            Err(Stuck::NotIn(State::ChooseWeapon))
        );
        let requests = client.tutorial_mut().inventory_opened(7).unwrap();
        assert_eq!(requests, [(0x49, vec![2, 7, 0, 0, 0]), (0x46, vec![0x0A])]);
        // The fault of the first live session: the server answered `49 02`
        // alone, and the weapons stayed under the sheet.
        assert!(client.tutorial().is_modal());
        assert_eq!(client.tutorial().highlighted(), [] as [u16; 0]);
        assert_eq!(
            client.tutorial_mut().drag_started(106),
            Err(Stuck::NotHighlighted(106))
        );
        // The inventory again: the two weapons are highlighted.
        client.receive(&inventory(&[18, 106, 107])).unwrap();
        assert_eq!(client.tutorial().highlighted(), CHOICE);
        // The pistol stays under the sheet.
        assert_eq!(
            client.tutorial_mut().drag_started(18),
            Err(Stuck::NotHighlighted(18))
        );
        client.tutorial_mut().drag_started(107).unwrap();
        assert_eq!(client.tutorial().state(), State::DragWeapon);
        client.tutorial_mut().dropped_on_weapon_slot(107).unwrap();
        assert_eq!(client.tutorial().state(), State::ClickAmmo);
    }

    #[test]
    fn an_inventory_without_both_weapons_is_refused_at_that_step() {
        let mut client = Client::signed_in();
        client.receive(&lobby::tutorial(true, 1, 0)).unwrap();
        // Before the step any inventory is taken.
        client.receive(&inventory(&[18])).unwrap();
        client.tutorial_mut().inventory_opened(1).unwrap();
        assert!(matches!(
            client.receive(&inventory(&[18, 106])),
            Err(ModelError::Bad(ServerId::Inventory, _))
        ));
        client.receive(&inventory(&[107, 106])).unwrap();
        assert_eq!(client.tutorial().highlighted(), CHOICE);
    }

    #[test]
    fn skipping_lifts_the_sheet() {
        let mut client = Client::signed_in();
        client.receive(&lobby::tutorial(true, 1, 0)).unwrap();
        client.tutorial_mut().inventory_opened(1).unwrap();
        assert_eq!(client.tutorial_mut().skipped(), (0x4B, vec![2]));
        assert_eq!(client.tutorial().state(), State::Default);
        assert!(!client.tutorial().is_modal());
        // A later inventory highlights nothing.
        client.receive(&inventory(&[106, 107])).unwrap();
        assert!(client.tutorial().highlighted().is_empty());
    }
}
