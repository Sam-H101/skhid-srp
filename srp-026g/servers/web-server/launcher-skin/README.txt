Original Survarium 0.26g launcher artwork, unchanged.
Source: the game's game/launcher/public/ directory, dated 2014-10-11.
These are game assets by Vostok Games, not newly authored project artwork.

back.jpg     899 x 335, 18790 bytes
sha256 f45037b8a66242c530746a70b168538d7d6cfe25f3f4282b96cc74ae991f907b
shadow.png   caption overlay, 125 bytes
sha256 0f45b64dea987068ca846f9018d564d63d8bc239980b991811edd1d6bc16c192

style.css is shared by launcher_pages.py and the web server. Both embed these
files as data URLs, avoiding new routes, external requests, and extra files
in the player's ZIP. Each URL fits Internet Explorer 8's 32768-character
limit. The game's news pictures and outbound links are not used.
