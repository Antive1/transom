# transom – dizajnový systém

Jediný zdroj pravdy je kód:
- `crates/transom-render/src/design.rs`: typografická stupnica, rozostupy, polomery, tlačidlá (druhy × stavy), karty,
  zaškrtávacie políčka, rámik fokusu.
- `fonts.rs`: výber písma.
- `theme.rs`: farebné tokeny tém a testy kontrastu.
- `brand.rs`: farby a geometria značky.
- `scene.rs`: rozmery lišty.

Súbory v `assets/brand/` sa **generujú** (`cargo run -p transom-render --example brand_export`) a test `brand` ich porovnáva
s kódom (drift = červený test). Ikony sa neťahajú z cudzích zdrojov; značka je vlastná. Všetko sa kreslí vlastným GDI
rendererom: žiadny UI framework ani pribalené písmo.

## Značka
- **Motív:** veľké „T“, ktorého priečka je lišta cez celú šírku (horný pruh obrazovky) a stonok dohora prechádza zo svetlej
  do farby lišty (`Role::Fade`); pod ňou svetlý obrys obrazovky
  (boky a spodok). Transom je priečnik – trám nad dverami, ako lišta nad obrazovkou. Ostré a čitateľné aj v 16 px
  (obrys pri malých veľkostiach zarovnaný na pixelovú mriežku).
- **Varianty:**
  - ikona aplikácie (dlaždica Ink + „T“ v Signal + svetlý obrys obrazovky);
  - značka bez dlaždice (glyph tmavý/svetlý);
  - wordmark „transom“ (monoline, `t` = word_b v akcente, `ransom` = word_a);
  - horizontálny lockup.

  Vektorový zdroj je v `assets/brand/*.svg`.
- **Ikony Windows:**
  - `transom.ico` (16, 20, 24, 32, 40, 48, 64, 128, 256);
  - `transom-tray-dark.ico` / `transom-tray-light.ico` (16–48, bez dlaždice; podľa témy **panela úloh**, nie aplikácie).

  Ikona je vložená do `.exe` (`build_support/icon_res.rs`) a za behu sa nastaví (`WM_SETICON`) pre okno nastavení.
- **Použitie:**
  - minimálna veľkosť 16 px;
  - okolo značky voľné miesto ≥ 1/4 jej šírky;
  - nemeniť farby mimo variantov, nenaťahovať, nepridávať tiene;
  - žiadne logá tretích strán (ani Apple, ani logá poskytovateľov AI) – poskytovateľ je len text.

## Písmo
- **Segoe UI Variable** (systémové písmo Windows 11), keď je naozaj k dispozícii.
  - Dostupnosť sa overí raz za proces: písmo sa vytvorí, vyberie do pamäťového DC a `GetTextFaceW` musí vrátiť požadovaný názov.
    Pri chýbajúcom písme GDI ticho podstrčí Arial, preto nestačí len `CreateFontW`.
  - Musia existovať všetky použité inštancie, inak sa celé rozhranie prepne na zálohu.
- **Optická veľkosť podľa veľkosti textu:** **Small** do 12 lp, **Text** 13–19 lp, **Display** od 20 lp. GDI uvádza pomenované
  inštancie pod názvom skráteným na 31 znakov (`Segoe UI Variable Display Semib`); tieto názvy sú v `fonts.rs`.
- **Polotučné písmo je skutočný Semibold (600)** cez pomenovanú inštanciu „… Semibold“.
  - Pôvodné „tučné“ písmo (Segoe UI s váhou 600) GDI kreslilo ako **Bold**. Rodina Segoe UI má v GDI len Regular/Bold;
    pri 14 px je Bold široký 183 px, Semibold 163 px.
- **Záloha** (Windows 10, odstránené písmo): Segoe UI + „Segoe UI Semibold“ (ak ani ten nie je, Segoe UI s váhou 600).
  Kontrola: `transom-settings --shot … --font classic` a `TRANSOM_GALLERY_FONT=classic` pre galériu aplikácie.
- **Celá lišta** používa `Font::Tabular`, teda klasické Segoe UI (polotučné = Bold ako doteraz).
  - Jeho číslice majú rovnakú šírku. Na tom stojí prekreslenie hodín po sekundách na mieste a stabilná šírka percent
    a odpočtov.
  - Segoe UI Variable a Segoe UI Semibold majú číslice proporcionálne: „1“ je užšia, pri 14 px 6 px namiesto 8 px.
  - V lište sa tak nestretnú dva tvary číslic (napr. „76 %“ vedľa „17:25“) a lišta vyzerá rovnako nepriehľadná aj
    priehľadná.
  - Rovnaké písmo má aj ukážka hodín v nastaveniach.
- **Text s japonskými, čínskymi alebo kórejskými znakmi** sa kreslí klasickým Segoe UI s pôvodnou váhou.
  - Prepojenie písiem pre pomenované inštancie Segoe UI Variable (hlavne „… Semibold“) dávalo znakom CJK inú veľkosť a
    rozostupy než latinke vedľa nich (zachytené na snímkach v čínštine a japončine).
  - Rozhoduje obsah reťazca, takže to platí aj pre japonský názov skladby či súboru v ľubovoľnom jazyku rozhrania.
- **Natívne vstupné polia nastavení** (HEX, R/G/B, dátum) dostanú rovnaké písmo ako kreslené popisky (Text 13 lp).
- **Vyhladzovanie:**
  - nepriehľadná lišta a okná: ClearType;
  - priehľadná lišta (vrstvené okno): sivé vyhladzovanie. Kreslí sa vždy klasickým Segoe UI: slabo hintované Segoe UI
    Variable je v sivom vyhladzovaní pri 13 px citeľne tenšie a bledšie (porovnané na zväčšených snímkach);
  - ikony (Segoe Fluent Icons): vždy sivé.

  Japončina a čínština: Windows má pre všetky inštancie záznamy `FontLink\SystemLink`, takže znaky dostanú náhradné písmo.
  Na tomto PC je však v reťazci prvé japonské Meiryo UI. Zjednodušené čínske znaky, ktoré v ňom chýbajú, preto prichádzajú
  z ďalších písiem a čínsky text môže mať miestami zmiešaný tvar. Je to systémové poradie, rovnaké ako pred redizajnom.

## Typografická stupnica (`design::Ty`; lp = logické px pri 96 DPI, škáluje sa DPI)
| Rola | Veľkosť / riadok | Hrúbka | Použitie |
|---|---|---|---|
| PageTitle | 24 / 32 | Semibold (Display) | nadpis stránky nastavení (zalamuje sa) |
| Section | 15 / 22 | Semibold | nadpis karty sekcie, nadpis popoveru |
| Subsection | 13 / 20 | Semibold | zoznam alebo skupina v sekcii |
| Body | 13 / 20 | Regular | popisok políčka, názov riadku volieb, položka zoznamu |
| BodyStrong | 13 / 20 | Semibold | zvýraznená hodnota (napr. „25 %“) |
| Control | 12 / 18 | Regular (vybraná voľba Semibold) | text tlačidla a voľby (Small) |
| Secondary | 12 / 18 | Regular, `muted` | popis pod ovládačom, vysvetlivky (medzi odsekmi 6 lp) |
| Caption | 11 / 16 | Regular, `muted` | metaúdaje, päta, „live preview“ |
| Badge | 10 / 14 | Semibold | odznaky stavu (STALE, Primary) vo `foreground`; štítok plánu (Pro/Max) je `muted` (≥ 4,5 : 1) |

- **Lišta:** text 13 lp a hodiny 14 lp (klasické Segoe UI), ikony 16 lp; kompaktná výška 28 lp sa nemení.
- **Popovery:** nadpis 15 Semibold, riadok zoznamu 13 Regular + metaúdaje 11–12 `muted`, tlačidlá 12.
- **Hrúbka písma sa nemení so stavom ovládača.** Popisok zapnutého políčka ani vybraný výsledok hľadania nie sú tučnejšie.
  Výnimkou sú voľby, ktoré sa merajú vybraným (polotučným) štýlom, takže výber nikdy nezmení rozloženie.

## Farby (tokeny tém: `theme.rs`)
| Token | Tmavá | Svetlá | Použitie |
|---|---|---|---|
| background | Ink `#171B21` | Mist `#F2F4F6` | pozadie lišty, okien, navigácie |
| surface | `#1E232A` | `#FFFFFF` | karta sekcie a karta v popoveri (zdvihnutá plocha) |
| foreground | `#E8ECF1` | `#14181E` | hlavný text |
| muted | `#98A3B0` | `#505B69` | pomocný text, zastarané hodnoty |
| accent | Signal `#3FE0B5` | Signal deep `#00806A` | hlavná akcia, zapnuté políčko, značka výberu, pruhy |
| accent_hover / accent_pressed | `#62E7C3` / `#36C9A2` | `#00735F` / `#00665A` | akcentová výplň pod myšou / stlačená |
| accent_text | `#5FE8C4` | `#006454` | text v akcente, **rámik fokusu** |
| on_accent | `#06231C` | `#FFFFFF` | text na akcente |
| selected | `#203837` | `#E0F0ED` | výplň vybranej voľby (vždy s akcentovým okrajom a polotučným textom) |
| control / control_hover / control_pressed | `#2A313B` / `#2F3640` / `#262C35` | `#FAFBFC` / `#F0F3F6` / `#E6EAEE` | výplň bežného tlačidla, poľa, drážky |
| control_edge | `#3A424D` | `#C9D0D8` | okraj ovládača, dráha posuvníka |
| edge | `#2E353F` | `#D3D9E0` | vlas: okraj karty, okna popoveru, oddeľovač |
| hover / pressed / placeholder | `#2A313B` / `#20262E` / `#232933` | `#DFE4EA` / `#CDD3DB` / `#E3E7EC` | riadok pod myšou / vybraný riadok, zástupné plochy |
| danger | `#F2766E` | `#A8281F` | text a okraj nebezpečnej akcie, chyba |
| danger_tint | `#382B30` | `#F9EDED` | nebezpečné tlačidlo pod myšou, riadok čakajúci na potvrdenie zmazania |
| danger_fill / danger_hover, on_danger | `#C42B1C` / `#A8251A`, biela | rovnaké | potvrdenie nebezpečnej akcie |
| low | `#F2766E` | `#A8281F` | málo kapacity (vždy aj so znakom „!“) |

**Kontrast** (testy `theme.rs` a `design.rs`):
- text (`foreground`, `muted`, `accent_text`) ≥ 4,5 : 1 na každej ploche, na ktorej sa kreslí: background, surface, placeholder,
  hover, pressed, control (všetky stavy) a selected;
- `danger` ≥ 4,5 : 1 na pozadí, karte, ovládači a nádychu; `on_accent` a `on_danger` na svojich výplniach ≥ 4,5 : 1;
- rámik fokusu a okraj vybranej voľby ≥ 3 : 1;
- karta sa odlišuje od pozadia a okraj ovládača je na karte viditeľný.

Stav (zastarané, reset prešiel, bez času resetu, málo, vybrané, potvrdenie) nikdy nenesie len farba – vždy aj slovo, znak,
okraj alebo hrúbka písma.

**Vlastná farba lišty** (`Theme::for_bar`, `on_background`):
- Farba nahradí len pozadie a text s ikonami idú podľa témy.
- Ak by text témy mal na zvolenej farbe kontrast < 4,5 : 1 a text opačnej témy by bol čitateľnejší, lišta použije farby
  opačnej témy. Rovnako to zobrazí aj náhľad v nastaveniach a pomer kontrastu.
- Tlmený a chybový text sa posunie k hlavnému textu, kým nemá 4,5 : 1. Akcent (kruh AI) sa posunie, kým nemá 3 : 1.
  Na strednej sivej, kde ani jedna farba textu tém 4,5 : 1 nedosiahne, je tlmený text aspoň taký čitateľný ako hlavný.
  Test prechádza celú sivú škálu.
- Pri **nepriehľadnosti pod 50 %** je pod lištou hlavne plocha, ktorú nepoznáme. Text ostáva podľa témy a farba je len
  pozadie. Výsledok závisí od plochy, čo nastavenia uvádzajú pri kontraste.

## Rozostupy, polomery, rozmery
- **Stupnica rozostupov:** 2 / 4 / 8 / 12 / 16 / 24 / 32 lp (`design::space`). Rozmery sú na mriežke 2 lp, aby pri 150 % padli
  na celé px.
- **Polomery:**
  - ovládače, polia a čipy 4 lp;
  - riadky zoznamov a položky menu 6 lp;
  - karty 8 lp;
  - deň v kalendári pilulka (polovica výšky).

  Rohy okien popoverov zaobľuje DWM.
- **Výšky:**
  - tlačidlo 32 lp (cieľ kliknutia ≥ 24 lp aj pri 100 %);
  - riadok so zaškrtávacím políčkom ≥ 28 lp (celý riadok je cieľom);
  - políčko 20 lp;
  - lišta 28 lp.
- **Vlasové čiary** majú vždy aspoň 1 px.
  - Okraj políčka: 1 px pri 100 a 150 %, 2 px pri 200 %.
  - Okraje ovládačov, kariet, vybranej voľby a nebezpečnej akcie majú 1 lp (pri 200 % 2 px).
  - Obrys farebných vzoriek je `muted` (≥ 3 : 1), aby vzorka v odtieni plochy mala zreteľnú hranicu.
- **Nastavenia:**
  - navigácia 200 lp, položka 36 lp;
  - obsah začína 28 lp od navigácie;
  - karta presahuje stĺpec obsahu o 16 lp vľavo aj vpravo, takže poloha ovládačov sa kartou nemení;
  - vnútorný okraj karty hore a dole 14 lp, medzera medzi kartami 12 lp.

## Komponenty (`design.rs`)
- **Tlačidlá** (`design::button`, `look`). Akcent znamená jednu vec: hlavnú akciu.

  | Druh | Pokoj | Pod myšou / stlačené | Ďalší stav |
  |---|---|---|---|
  | Primary (najviac jedna v skupine: Pridať dátum, Importovať vybrané, Pripojiť Claude Code, Uložiť poznámku) | `accent` + `on_accent` Semibold | `accent_hover` / `accent_pressed` | – |
  | Secondary (bežné akcie) | `control` + 1 px `control_edge` | `control_hover` / `control_pressed` | – |
  | Quiet | bez výplne | výplň ovládača | – |
  | Destructive (Zmazať dáta AI, Zmazať archív, Vyčistiť Policu/Schránku/Poznámky, Zahodiť poznámku, Nahradiť) | `control` + okraj a text `danger` + ikona koša | `danger_tint` | potvrdenie (druhý krok): `danger_fill`, biely Semibold text a **znak varovania** |
  | Choice (voľby v riadku) | ako Secondary | ako Secondary | vybraná: `selected` + 1 px `accent` + Semibold |

  - **Zakázané tlačidlo:** `disabled` text, na myš nereaguje.
  - **Kde sa stavy kreslia:**
    - všetky druhy a stavy sú v hárku komponentov (`gallery_components`);
    - tlačidlá nastavení ukazujú hover, fokus, výber a potvrdenie; stlačenie v nastaveniach zatiaľ nie (okno ho nesleduje);
    - popovery (Nástroje, Zvuk) ukazujú aj stlačenie;
    - Quiet a Disabled zatiaľ nepoužíva žiadna obrazovka.
  - **Ikony** (Segoe Fluent Icons) len tam, kde pomáhajú pri prehliadaní: pridať, odstrániť, zmazať, obnoviť, prehrať ukážku,
    identifikovať, vybrať farbu, otvoriť, priečinok, hore/dole.
  - Ak sa ikona s textom nezmestí, vynechá sa ikona; text sa nikdy neskracuje kvôli ikone.
  - **Prístupný názov tlačidla je vždy jeho text.** Ikona do neho nevstupuje (overené porovnaním výpisu MSAA pred a po).
- **Riadok volieb** (`choice_row`):
  - popisok (Body) sa zalomí v stĺpci popisku a pri viac ako dvoch riadkoch ide nad voľby;
  - voľby sa zalamujú a riadky sa vyvážia (5 volieb = 3 + 2, nie 4 + 1).
- **Rad tlačidiel** (`flow_buttons`): šírka podľa textu (aspoň 120 lp), zalamuje sa na šírku obsahu – nič nepretečie okno
  (opravené orezané „Zrušiť“ pri potvrdení zmazania).
- **Karta sekcie** (`design::card`):
  - `surface` + 1 px `edge`, polomer 8 lp; nadpis sekcie (Section) je v karte;
  - nastavenia kreslia obsah stránky najprv do záznamu (`record::Recorder`), potom karty a nakoniec obsah – rozmer karty je
    známy až po rozložení, bez druhého rozkladania a bez blikania;
  - stĺpce rozloženia lišty nie sú v karte (sú to vlastné zóny presúvania).
- **Zaškrtávacie políčko** (`design::checkbox`):
  - zapnuté = `accent` + značka `on_accent`;
  - vypnuté = 1 px okraj `muted` (pod myšou `foreground`) a výplň ovládača;
  - závislé voľby (napr. „Text“ pod „Uchovávať históriu schránky“) sú odsadené o 32 lp.
- **Rámik fokusu** (`design::focus_ring`, `focus_ring_inset`):
  - dvojfarebný: 1 lp čiara farby plochy a za ňou 2 lp `accent_text`. Je viditeľný aj okolo akcentovej či červenej výplne
    a odlišuje sa od výberu, ktorý má vlastnú výplň a okraj;
  - kde nie je miesto (záložky sekcií v paneli Nástroje, riadky zoznamu, tlačidlá hlavičky okna), kreslí sa dovnútra;
  - zobrazuje sa len pri fokuse z klávesnice.
- **Informačný pruh** (`infobar`): jemný červený nádych, okraj, znak varovania a text. Použitie: AI položka skrytá v lište,
  záloha monitora, chýbajúci zdroj archívu, konfigurácia len na čítanie.
- **Zoznamy** (priečinky, aplikácie):
  - podnadpis, poznámka a riadky „Odstrániť …“ (ikona ✕ a text vľavo);
  - pod nimi „Pridať …“ so šírkou podľa textu.
- **Výber v zozname** (Nástroje, Hľadanie, menu):
  - zdvihnutá plocha `hover` + 3 lp akcentová značka vľavo;
  - riadok pod myšou má slabý odtieň (35 %), aby sa nepodobal výberu;
  - riadok čakajúci na druhý Delete je `danger_tint` s okrajom `danger` a znakom varovania. Rámik fokusu je vnútri okraja
    a výzva v päte je polotučná v `danger`.
- **Polia** (Hľadanie, editor poznámky): plocha ovládača, okraj `control_edge`, lupa a 2 lp akcentová spodná linka poľa
  s fokusom.
- **Karty v popoveri AI:** `surface` + `edge`. Prepínače (Left/Used, obdobie, metrika) sú voľby: `selected` + akcentový okraj
  + Semibold.
- **Kalendár:**
  - dnešok = plná akcentová pilulka;
  - vybraný deň = akcentový obrys a polotučné číslo;
  - pod myšou plocha;
  - šípky majú zaoblenú plochu ovládača.

## Okno nastavení
- **Hlavička** (`chrome.rs`, 40 lp):
  - na ploche okna s vlasom `edge`, logo + wordmark + názov (muted);
  - tlačidlá 46 × 40 lp so znakmi Segoe Fluent Icons; zavrieť pod myšou = `#C42B1C` s bielym znakom;
  - v neaktívnom okne sú znaky aj názov `muted` (čitateľné);
  - fokus je vnútorný dvojfarebný rámik; na červenom „Zavrieť“ s bielou vnútornou čiarou.
- **Navigácia:**
  - na ploche okna, oddelená vlasom;
  - vybraná položka = plocha `hover` + akcentová značka + Semibold;
  - pod myšou polovičný odtieň;
  - ikony monoline (`navicons.rs`).
- **Obsah:**
  - nadpis stránky a úvod sú mimo kariet, sekcie sú v kartách s nadpisom
    (napr. AI: Zdroje údajov, Pripojiť Claude Code, Archív, Ukazovateľ v lište, Uložené údaje, Kalendár);
  - riadky volieb majú na všetkých stránkach rovnaký stĺpec popisku (180 lp);
  - posuvný obsah (koleso, PgUp/PgDn, Home/End);
  - pevný stavový riadok;
  - texty sa zalamujú (nič sa potichu neorezáva).

## Štýl ikon
- Monoline 1,5 jednotky na mriežke 16 × 16, zaoblené konce, bez výplní okrem jedného „polovičného“ kontrastu.
- Kreslia sa vektorovo (`navicons.rs`, `resulticons.rs`) nad ľubovoľným podkladom (AA cez `Painter::blend_rgba`).
- Ikony lišty a tlačidiel používajú Segoe Fluent Icons (systémové).

## Lišta
- **Kompaktná:** výška 28 lp, hover/pressed „čip“ zaoblenie 4 lp, odstup 3 lp, delič 14 lp. Delič je z farby textu nad
  skutočným pozadím, takže funguje s vlastnou farbou aj pri priehľadnosti.
- **Kruh AI:** `kruh + „62 % used“ + (voliteľný) „Claude · 5 h · resets in 2 h 14 min“`.
  - Voliteľný text sa skrýva ako celok, nikdy sa neskracuje.
  - Stavy: `~62 %` + „stale“, `unavailable`, `reset passed`, `! 95 %` + farba `low`.
- **Ticker:**
  - stála rýchlosť 20 / 35 / 60 lp/s, rozbeh a dobeh 400 ms, snímky po celých pixeloch (≥ 16 ms);
  - okno aspoň 120 lp, inak statický úsek s „…“.

## Panel AI
- **Popover (440 lp):** karta na poskytovateľa; riadok okna = názov + odpočet vľavo, **zostávajúce %** (18 lp) + „left“ vpravo.
  Pod ním pruh 10 lp (dráha `control_edge`) a malé „X % used“.
- **História:** mesačný graf so stavmi tvarom.
  - plný = zaznamenané;
  - pruhovaný = čiastočné;
  - bodkovaná čiara = bez histórie;
  - prázdny obrys = bez ceny;
  - čiarka = nula.

  Zvolený deň = plocha + akcentový obrys.
- **Archív a dvojkrokové mazanie** sú v nastaveniach v kartách s nebezpečnými tlačidlami (pozri Komponenty).

## Kontroly (ako overiť vzhľad)
- **Nastavenia:** `transom-settings --shot <adresár> [--lang all] [--dpi 96,144,192] [--png] [--full] [--font classic]`.
  - Audit textov: orezanie, výška, prekryv, kontrast voči skutočnej ploche pod textom.
  - `--full` uloží celú stránku na jednu snímku.
- **Galéria aplikácie:** `TRANSOM_GALLERY=<adresár> cargo test -p transom-app gallery_ -- --ignored --test-threads=1`.
  - Lišta: predvolená, hover, stlačenie, vlastná farba, priehľadná.
  - Menu, panel AI, kalendár, Zvuk, Nástroje (všetky sekcie, fokus, potvrdenie, editor), Hľadanie.
  - Vždy pri 100/150/200 % v oboch témach a len z vymyslených dát.
- **Výpis prístupnosti pred/po** (názov, rola, stav, hodnota, akcia):
  - nastavenia: `TRANSOM_A11Y_DUMP=<adresár> cargo test -p transom-settings a11y_dump -- --ignored`;
  - popovery: `gallery_a11y_dump`.
- **Audity popoverov** v testoch `transom-app` (každý jazyk, DPI aj téma) a testy kontrastu v `theme.rs` a `design.rs`.

Overené pokrytie tejto verzie je v `docs/PLAN.md` §8.27.
