"""The pages the Skhid launcher shows from its own folder
(`game/binaries/x86/launcher_skhid/`), in the place of the game's
`game/launcher/index-*.html`:

    index-blank-<code>.html   while the server's page is being fetched
    index-<code>.html         when it could not be fetched

for the launcher's five languages (en, ru, ua, de, pl). The text is
Ukrainian for `ua` and English for the others, as on the server's page.
They are made for the launcher's embedded browser (Internet Explorer's
engine, 899 by 335 pixels): fixed sizes, old CSS, no script, and embedded
original background artwork. The server and SRP Discord links open in the
player's browser.

    python launcher_pages.py <folder> [--site=https://skhid.link]
        writes the ten files into <folder> (to look at them)
"""

import base64
import html
from pathlib import Path
import os
import sys

CODES = ("en", "ru", "ua", "de", "pl")

# One skin for the web server and both local states. Each image is below
# IE8's 32 KiB data-URI limit; no network or relative path is needed.
HERE = Path(__file__).resolve().parent
SKIN = HERE.parents[1] / "servers" / "web-server" / "launcher-skin"
DISCORD_LINK = (SKIN / "discord-link.html").read_text(encoding="utf-8").strip()
STYLE = (SKIN / "style.css").read_text(encoding="utf-8")
for slot, name, mime in (("BACKGROUND", "back.jpg", "image/jpeg"),
                         ("SHADOW", "shadow.png", "image/png")):
    image = base64.b64encode((SKIN / name).read_bytes()).decode("ascii")
    STYLE = STYLE.replace(f"@{slot}@", f"data:{mime};base64,{image}")

TEXT = {
    "en": {
        "lang": "en",
        "title": "Skhid &middot; Survarium <i>0.26g</i> community server",
        "loading": "Loading the server's page&hellip;",
        "offline": "The server's page could not be loaded.",
        "hint": "The game itself may still work: watch the line under this page. "
                "If it shows an error, check the internet connection and start the launcher again.",
        "page": "The server's page",
        "legal_title": "Legal notice",
        "legal": "Unofficial community server. Not affiliated with or endorsed by Vostok Games. "
                 "Survarium is a trademark of its owners.",
        "launcher": "Launcher",
        "about": "This launcher keeps the Skhid files of the game up to date and starts the game "
                 "in the language chosen above.",
    },
    "ua": {
        "lang": "uk",
        "title": "Схід &middot; ком'юніті сервер Survarium <i>0.26g</i>",
        "loading": "Завантаження сторінки сервера&hellip;",
        "offline": "Не вдалося завантажити сторінку сервера.",
        "hint": "Сама гра може працювати: дивіться рядок під цією сторінкою. "
                "Якщо там помилка, перевірте інтернет-з'єднання і запустіть лаунчер ще раз.",
        "page": "Сторінка сервера",
        "legal_title": "Правова інформація",
        "legal": "Неофіційний ком'юніті сервер. Не пов'язаний із Vostok Games і не схвалений ними. "
                 "Survarium є торговельною маркою її власників.",
        "launcher": "Лаунчер",
        "about": "Цей лаунчер оновлює файли Схід для гри і запускає гру мовою, обраною вгорі.",
    },
}


def page(code, site, blank):
    text = TEXT["ua" if code == "ua" else "en"]
    host = site.split("//", 1)[1].split("/", 1)[0]
    if blank:
        left = f"<p class=big>{text['loading']}</p>"
    else:
        left = f"<p>{text['offline']}</p><p>{text['hint']}</p>"
    link = f'<a href="{html.escape(site, quote=True)}/" target="_blank">{html.escape(host)}</a>'
    return (
        "<!DOCTYPE html><html lang={lang}><head>"
        '<meta http-equiv="X-UA-Compatible" content="IE=edge">'
        '<meta http-equiv="content-type" content="text/html; charset=UTF-8">'
        '<title>Skhid</title><style type="text/css">{style}</style></head><body>'
        "<div id=container>"
        "<div id=left><h1>{title}</h1>{left}</div>"
        "<div id=right>"
        "<div class=\"tile first\"><p>{link}</p><div class=cap>{page}</div></div>"
        "<div class=\"tile second\"><p class=small>{about}</p><div class=cap>{launcher}</div></div>"
        "<div class=\"tile third\"><p class=small>{legal}</p><div class=cap>{discord}{legal_title}</div></div>"
        "</div></div></body></html>\r\n"
    ).format(style=STYLE, left=left, link=link, discord=DISCORD_LINK, **{k: v for k, v in text.items()
                                                   if k not in ("loading", "offline", "hint")}).encode("utf-8")


def pages(site):
    """{file name: bytes} of the ten pages for the server page `site`."""
    out = {}
    for code in CODES:
        out[f"index-blank-{code}.html"] = page(code, site, True)
        out[f"index-{code}.html"] = page(code, site, False)
    return out


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    site = next((a[7:] for a in sys.argv[1:] if a.startswith("--site=")), "https://skhid.link")
    if len(args) != 1:
        sys.exit(__doc__)
    os.makedirs(args[0], exist_ok=True)
    for name, data in pages(site.rstrip("/")).items():
        with open(os.path.join(args[0], name), "wb") as out:
            out.write(data)
        print(f"wrote {os.path.join(args[0], name)} ({len(data)} bytes)")
