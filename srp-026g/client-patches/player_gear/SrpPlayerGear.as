package
{
    import flash.display.DisplayObject;
    import flash.events.MouseEvent;
    import flash.external.ExternalInterface;
    import flash.utils.Dictionary;
    import flash.utils.getTimer;
    import scaleform.gfx.MouseEventEx;
    import survarium.controls.SrvContextMenu;
    import survarium.data.LocalizationData;

    // "Show player's information" outside the results screen (srp's own;
    // the owner's of 2026-10-20: "you should be able to go to rankings and
    // rmb to do the same", "for every single player"). A right-click on a
    // player opens the game's right-click menu (SrvContextMenu, as the
    // results screen's) with the game's own "Show player's information";
    // it asks as the results screen's double-click on a row does
    // (`victory_screen_request_player_data`, `3D 00 <player> <match>`), with
    // a player no results row has (as srp's History asks for a replay), and
    // the answer (`93`) opens the game's own window ("player_info",
    // LobbyMenu.show_player_info), as the results screen's double-click
    // does. The server answers from the account as it is now
    // (servers/lobby-server/src/v026g/gear.rs; docs/notes/player_gear.md).
    public class SrpPlayerGear
    {
        // The request's `player` (`3D 00 <how> <player>`, as srp's History
        // asks for a replay): its `match` is then the place of a row of the
        // ranking (1 the first; srp's lobby::GEAR_BY_PLACE) or an account's
        // id (lobby::GEAR_BY_ACCOUNT, the rooms' lists).
        public static const BY_PLACE:uint = 0xFFFFFF03;
        public static const BY_ACCOUNT:uint = 0xFFFFFF02;
        // An answer later than this is not ours: the results screen's.
        private static const WAIT_MS:int = 6000;
        private static var _name:String = null;
        private static var _at:int = 0;
        private static var _hooked:Dictionary = new Dictionary(true);

        // The game's right-click menu with the one item, at the mouse.
        public static function menu(nickname:String, player:uint, how:uint):void
        {
            if (nickname == null || nickname == "")
                return;
            SrvContextMenu.show([{"name": LocalizationData.getName("show_player_info_button_hint"),
                "click_function": onItem, "callBack": true, "nickname": nickname, "player": player,
                "how": how}]);
        }

        private static function onItem(item:Object):void
        {
            ask(String(item.nickname), uint(item.player), uint(item.how));
        }

        public static function ask(nickname:String, player:uint, how:uint):void
        {
            _name = nickname;
            _at = getTimer();
            try
            {
                ExternalInterface.call("victory_screen_request_player_data", how, player);
            }
            catch (e:Error)
            {
            }
        }

        // LobbyMenu.fill_victory_screen_player, first: the answer to our
        // ask opens the window here, as VictoryScreen.setPlayerData and
        // LobbyMenu.onVSPlayerInfo do for a row of the results (they keep
        // it in the screen's data, which no results screen has here). Any
        // other answer is the results screen's, as before.
        public static function take(lobby:*, value:Object):Boolean
        {
            if (_name == null || getTimer() - _at > WAIT_MS)
                return false;
            var nickname:String = _name;
            _name = null;
            if (!value)
                return true;
            if (!(value is Array))
            {
                value.name = "";
                value.level = 1;
                value = [value];
            }
            try
            {
                lobby.show_player_info({"nickname": nickname, "profiles": value});
            }
            catch (e:Error)
            {
            }
            return true;
        }

        // PlayerRanking.onActivate, last: a right-click on a row of the
        // Rankings window (either tab, any page) or on the player's own
        // place under them.
        public static function ranking(page:*):void
        {
            if (page == null || _hooked[page])
                return;
            _hooked[page] = true;
            page.addEventListener(MouseEvent.CLICK, onRankingClick);
        }

        private static function onRankingClick(e:MouseEvent):void
        {
            var ex:MouseEventEx = e as MouseEventEx;
            if (ex == null || ex.buttonIdx != 1)
                return;
            for (var at:DisplayObject = e.target as DisplayObject; at != null; at = at.parent)
            {
                var row:Object = rankingRow(at);
                if (row != null)
                {
                    menu(String(row.nickname), uint(row.place), BY_PLACE);
                    return;
                }
            }
        }

        // A row's data (RankingItemRenderer's): its place and nickname.
        private static function rankingRow(at:DisplayObject):Object
        {
            try
            {
                var data:Object = Object(at).data;
                if (data != null && data.nickname != null && int(data.place) > 0)
                    return data;
            }
            catch (e:Error)
            {
            }
            return null;
        }
    }
}
