# Buy and Sell in the craft window

The rules the code follows. Code: `client-patches/craft_menu_026g.py`, `craft_menu/SrpCraftMenu.as`,
`craft_menu/PatchCraftMenu.java`; README "Buy and Sell in the craft window". The owner's rule:
2026-10-20 ("rmb on bought item -> upgrade -NEW-> rmb on any item in the list -> buy/sell (NOT
upgrade)").

- **Where:** the craft window (right-click an owned item → **Upgrade**: two items of a kind make a
  better one). Its list shows the items that can go in (copies of the same item).
- **What:** right-click on an item of that list gives **Buy** and **Sell**, nothing else (no
  Upgrade, Wear, Fix, Modification, Appearance). Before, it gave no menu: the window locks the list.
- **Buy:** as in the inventory: listed when the shop sells the item and the account has unlocked it;
  for an owned item whatever its modifications (`buy-modified.md`). It buys a stock item at the
  shop's price, without the "already purchased" question.
- **Sell:** as in the inventory: an owned item or a stack, not in the shop's list; the game's sell
  dialog asks first.
- **Elsewhere:** the inventory's right-click menu is as before; a list locked for another reason
  (another window) still gives no menu.
- **Server:** nothing changes.
