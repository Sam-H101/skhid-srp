package
{
    import flash.display.DisplayObjectContainer;
    import survarium.controls.GangShop;
    import survarium.controls.InventoryList;
    import survarium.controls.SrvContextMenu;
    import survarium.data.LocalizationData;
    import survarium.messages.MessageCraftItem;

    // The craft window ("Upgrade": two items of a kind make a better one)
    // locks the inventory list, and a locked list gave no right-click menu.
    // While it is open, right-click on an item of the list gives Buy and Sell
    // only, by the list's own rules for them; nothing else (no Upgrade).
    public class SrpCraftMenu
    {
        // InventoryList.showContextMenu reads `this._inventoryLocked` through
        // this: true ends the method there.
        public static function locked(value:*, list:InventoryList, slot:*):Boolean
        {
            if (!crafting(list))
                return Boolean(value);
            var item:Object = slot != null ? slot.data : null;
            if (item == null || !(item.enabled || item.canBuy))
                return true;
            var owned:Boolean = item.descriptor <= 100 && item.enabled;
            var options:Array = [];
            // As the list's menu: Buy for an owned item, a stack or an item
            // of no rarity; Sell for an owned item or a stack, not in the shop.
            if (item.canBuy && (item.isStack || !item.rank || owned))
                options.push({"name": LocalizationData.getName("buy"),
                              "click_function": function(e:* = null):void { buy(list, item); }});
            if ((owned || item.isStack) && !list.isShop)
                options.push({"name": LocalizationData.getName("sell"),
                              "click_function": function(e:* = null):void { list.sellItem(item.itemId); }});
            if (options.length > 0)
                SrvContextMenu.show(options);
            return true;
        }

        private static function crafting(list:InventoryList):Boolean
        {
            var lobby:DisplayObjectContainer = list.parent;
            if (lobby == null)
                return false;
            for (var i:int = 0; i < lobby.numChildren; i++)
                if (lobby.getChildAt(i) is MessageCraftItem)
                    return true;
            return false;
        }

        // As the item card's Buy: by the dictionary id, at the shop's price;
        // the shop names the seller. A buy on an owned item is not asked
        // whether the account wants another (buy_modified_026g.py).
        private static function buy(list:InventoryList, item:Object):void
        {
            var profile:* = list.parent != null ? list.parent["player_profile"] : null;
            if (profile == null)
                return;
            var price:int = item.category == 100 ? int(item.cost) : int(GangShop.getInstance().getItemPrice(item.dictId));
            GangShop.buyFunction(profile.profileId, item.itemId, item.dictId, item.descriptor, 100, 1, price, -1);
        }
    }
}
