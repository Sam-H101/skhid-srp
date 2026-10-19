package
{
    import survarium.controls.Message;

    // Every "Get Gold" of the lobby opened the online store, which is gone:
    // it opens srp's exchange window (SrpExchange) instead, to buy gold with
    // silver. A dialog behind it (the premium window, a buy dialog) waits
    // under it and comes back when it closes.
    public class SrpBuyGold
    {
        // The dialogs call it with their data; the premium window's gold
        // button with its click: neither is needed.
        public static function open(... unused):void
        {
            // The third argument shows it at once over a dialog still open;
            // without it the window would wait for that dialog to close.
            Message.showMessage("exchange", {"label": "", "srpBuyGold": true}, true);
        }
    }
}
