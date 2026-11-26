# transom – plán produktu a techniky

Stav: **implementácia MVP** (pôvodne návrh bez kódu). Vytvorené 2026-11-15, revízia 2026-11-16 (full-width lišta,
grafické nastavenia, rozhodnutie o vykresľovaní až podľa merania), 2026-11-20 (míľnik stavových položiek, §8.2); posledná revízia 2026-11-21 (§8.13 archív, §8.14 audit spikov, bezpečnosti a Git). Licencia projektu: MIT. Cieľový systém: iba Windows 11.

Značky: **[Overené]** = skontrolované 2026-11-15 v dokumentácii/crates.io (zdroje na konci).
**[Odhad]** = môj úsudok alebo číslo bez merania. **[Test]** = treba overiť v spike.

---

## 1. Produkt a UX

**Plnošírkový** tenký pás (predvolene 28 logických px, nastaviteľné) ako macOS menu bar: od ľavej po pravú
hranu každého vybraného monitora, bez odsadenia, bez zaoblených rohov a bez "plávajúceho" tvaru.
Rezervuje miesto pri hornej hrane, takže maximalizované okná začínajú pod ním.
Monitory, na ktorých sa zobrazí, si používateľ vyberie v grafickom okne nastavení otvorenom z vlastnej ikony
aplikácie v oblasti oznámení.

- Vľavo: tlačidlo **Search**, potom **médiá** (názov – interpret, predchádzajúca / prehrať-pauza / ďalšia).
- Vpravo: tlačidlo **Tray**, **batéria** (len ak je), **čas**.
- Doplnkové monitory môžu mať kompaktný variant (napr. len čas) – nastaviteľné.
- Bez launchera, prepínania aplikácií, shell hackov. Panel úloh sa nemení.
- Nastavenia: pravé tlačidlo na lište → ponuka transom (Nastavenia, Využitie AI, Ukončiť) – od 2026-11-20, §8.4; vlastná
  ikona v oblasti oznámení: od §8.7 (tray ikona s menu Nastavenia/Ukončiť).
- Konfigurácia: `%APPDATA%\transom\config.json` (verzovaný JSON, §8.4; pôvodne plánované `config.toml`).
- Pracovné ciele spotreby (**ciele, nie namerané**): ~0 % CPU v pokoji, malý pracovný set.

## 1a. Dôsledky full-width dizajnu

- **Geometria:** obdĺžnik lišty = `rcMonitor.left .. rcMonitor.right`, `top .. top + výška` (vo fyzických px).
  Žiadne okraje ani tieň okna. Voliteľne 1 px oddeľovacia čiara kreslená *vnútri* lišty.
- **Výška vo fyzických px po monitore:** logická výška × mierka monitora, zaokrúhlená na celé px
  (28 lp = 28/35/42/49/56 px pri 100/125/150/175/200 %). Každý monitor má vlastnú výšku v px,
  vizuálna výška v lp ostáva rovnaká. Pri `WM_DPICHANGED` sa navrhnutý obdĺžnik ignoruje
  a z `rcMonitor` sa vypočíta nový, potom znovu `QUERYPOS/SETPOS`.
- **Rohy a pozadie:** okno bez zaoblenia (`DWMWA_WINDOW_CORNER_PREFERENCE` = nezaobľovať) a pozadie
  Mica/Acrylic (`DWMWA_SYSTEMBACKDROP_TYPE`) alebo plné farebné; oboje je z mojich znalostí **[Neoverené]**
  pre okno bez titulku – **[Test]** (S9). Záloha: plné nepriehľadné pozadie.
- **Obsah:** položky zoskupené vľavo (Search, médiá) a vpravo (Tray, batéria, čas); stred zostáva voľný.
  Na ultra-širokých monitoroch sa nerozťahujú, médiá majú maximálnu šírku s elipsou. Prázdna plocha je inertná.
- **Monitory nad sebou:** pás nižšieho monitora susedí s dolnou hranou horného. Ovplyvňuje len ťahanie okien a kurzor
  (**[Test]** S2).
- **Rezervácia a okná:** všetky okná (nielen maximalizované) sa po zmene výšky preusporiadajú; zmena výšky
  v nastaveniach je teda viditeľný zásah do cudzích okien – robiť zriedka a s potvrdením.
- **`keep_visible` pri fullscreene:** pri plnej šírke prekrýva celý horný pás hry, nie len roh.
- **Panel úloh:** Windows 11 má panel úloh dole; AppBar na hornej hrane sa s ním nekríži. `QUERYPOS` aj tak upraví
  obdĺžnik, ak by sa prekrýval **[Overené]**. Že Windows 11 nepodporuje panel úloh hore je môj predpoklad.
- **Voliteľné rohy ako na macOS** (čierne rohy pod lištou): mimo MVP.

## 2. AppBar vs overlay

| | AppBar (`SHAppBarMessage`) | Overlay (topmost okno) |
|---|---|---|
| Maximalizované okná | zmestia sa pod pás | pás zakrýva titulok a tlačidlá okna vpravo hore |
| Zásah do systému | podporované API, mení len pracovnú plochu | žiadny |
| Zložitosť | vyššia (protokol, notifikácie) | nízka |
| Riziko | posúvanie cudzích okien, čistenie po páde | prekrývanie ovládacích prvkov |

**[Overené]** Postup: `ABM_NEW` → `ABM_QUERYPOS` (systém môže obdĺžnik upraviť) → `ABM_SETPOS`
(systém môže upraviť znova, preto sa použije vrátený obdĺžnik) → `MoveWindow`.
Pri zmene polohy panela úloh a pod. chodí notifikácia a appbar má znovu poslať QUERYPOS/SETPOS.
`ABM_REMOVE` appbar odregistruje. Pri spustení/ukončení fullscreen aplikácie appbar dostane
`ABN_FULLSCREENAPP`; dokumentácia hovorí, že pri otvorení sa má appbar presunúť na spodok z-poradia
a po zatvorení ho obnoviť.

**Odporúčanie:** AppBar ako jediný režim v MVP (zhoda s požiadavkou "rezervuje miesto").
Overlay/auto-hide odložiť, nie je v požiadavkách.

**[Test]** čo sa stane s rezerváciou po páde procesu; viacero appbarov na rôznych monitoroch;
správanie pri zmene DPI a rozlíšenia; interakcia s Windows 11 panelom úloh; správanie snap layouts.

## 3. Výber monitorov a jedna lišta na monitor

- **Identifikácia:** `HMONITOR` a `\\.\DISPLAYn` nie sú stabilné. Pre konfiguráciu použiť stabilnejší identifikátor
  (cesta zariadenia monitora cez DisplayConfig API) **[Test]**, s fallbackom na názov a pozíciu.
- **Konfigurácia:** režim `all` / `primary` / `selected`; pre `selected` zoznam
  `{ id, label, enabled, výška, variant }`. Chýbajúci monitor sa preskočí a lišta sa vytvorí po jeho pripojení.
- **Správca monitorov (`MonitorManager`):** pri `WM_DISPLAYCHANGE`, `WM_DPICHANGED`,
  zmene pracovnej plochy a zmene konfigurácie porovná požadovanú a existujúcu množinu líšt
  a vytvorí, presunie alebo zruší `BarWindow`. Každé `BarWindow` vlastní svoju AppBar registráciu.
- **Nastavenia (grafické okno z ikony v oblasti oznámení):** zoznam monitorov s náhľadom rozloženia,
  číslami a tlačidlom "Identifikovať" (krátke zobrazenie čísla na monitore), zapnutie lišty po monitore.
  Podrobnosti o procese a komunikácii sú v §7.7.
- **DPI:** manifest PerMonitorV2, rozloženie v logických px, vykresľovanie vo fyzických px,
  prepočet pri `WM_DPICHANGED`. Po zmene vždy znovu `QUERYPOS/SETPOS`. **[Test]** zmiešané DPI.

## 4. Fullscreen

Nastavenie: `fullscreen = "hide" | "keep_visible"` (voliteľne per monitor). Predvolene `hide`.

**Detekcia (dve nezávislé vrstvy):**
1. `ABN_FULLSCREENAPP` od systému **[Overené]**.
2. Vlastný detektor: `SetWinEventHook` (zmena popredia, presun/zmena veľkosti), okno pokrýva celé
   `rcMonitor` (nie `rcWork`), nie je minimalizované ani cloaked, nie je to shell okno ani naše.
   Doplnkový signál `SHQueryUserNotificationState` (`QUNS_BUSY`, `QUNS_RUNNING_D3D_FULL_SCREEN`) **[Overené]**.

**Správanie:**
- `hide`: lišta sa na dotknutom monitore skryje (alebo klesne na spodok z-poradia podľa dokumentácie),
  po skončení sa obnoví. Ostatné monitory sú nedotknuté.
- `keep_visible`: lišta zostane topmost a ignoruje odporúčanie z `ABN_FULLSCREENAPP`.
  Limity:
  - **Borderless fullscreen:** lišta sa dá zobraziť nad oknom, ale prekryje horných ~28 px obsahu hry.
  - **Klasický exkluzívny fullscreen:** viditeľnosť **nie je zaručená** a závisí od spôsobu prezentácie
    (**[Test]**; **[Odhad]** že niektoré aplikácie lištu neukážu).
  - **[Odhad]** prekrytie môže znížiť výkon hry (strata priameho prezentovania); treba zmerať.
  - Kliknutie nesmie aktivovať lištu (`WS_EX_NOACTIVATE`), inak hra môže stratiť fokus.
- UI nastavenia má jasne popísať tieto limity.

## 5. Tlačidlá Tray a Search

Žiadne z nich nemá podporované verejné API na priame otvorenie UI. Praktická cesta je simulácia skratiek.

- **Search:** `Win+S`; **Tray:** `Win+B` (presunie fokus do oblasti oznámení). **[Overené]** skratky v Microsoft zozname.
- Implementácia: `SendInput` so sekvenciou LWin down, kláves down/up, LWin up; použiť virtuálne kódy.
- **[Overené]** `SendInput` podlieha UIPI: vstup možno vložiť len do procesov s rovnakou alebo nižšou
  integritou. Zlyhanie z dôvodu UIPI sa **nenahlási** ani návratovou hodnotou, ani `GetLastError`.
  Z návratovej hodnoty ho teda detegovať nemožno; lišta preto vopred kontroluje eleváciu popredia a potom overuje
  zmenu popredia (§5.1). Tlačidlo nemá fallback.
- **[Test]** matica: popredné okno bežné vs. spustené ako správca; skryté/zobrazené panel úloh;
  Win klávesy zakázané politikou; rôzne rozloženia klávesnice; fyzicky držané modifikátory;
  či `Win+B` na Windows 11 otvorí aj flyout skrytých ikon alebo len presunie fokus.
- Lišta musí byť bez aktivácie, aby sa fokus nemenil pred odoslaním skratky.
- Vlastné zobrazenie cudzích tray ikon (UI Automation, nedokumentované štruktúry) – mimo MVP.

### 5.1 Overenie z lišty (2026-11-20, `transom-app`, klik cez `tools/click-bar.ps1`)
- **Search:** klik na tlačidlo nezmenil popredie (`WS_EX_NOACTIVATE`; pred aj po kliku Brave), `Win+S` otvoril
  `SearchHost.exe` (`Windows.UI.Core.CoreWindow`), fokus v „Search box“; overenie po 600 ms prešlo, Esc vrátil fokus do Brave.
- **Tray:** `Win+B` presunul fokus do `Shell_TrayWnd` (overené po 400 ms). UI Automation: prvok s fokusom
  `name='Show Hidden Icons'`, `AutomationId='SystemTrayIcon'`, trieda `SystemTray.NormalButton`. **AutomationId aj trieda
  sú spoločné aj pre iné ikony** (napr. indikátor vstupu), rozlišuje len **lokalizovaný názov**. Lišta preto pošle Enter
  len pri presnej zhode názvu „Show Hidden Icons“ (anglické UI) → otvoril sa flyout `TopLevelWindowForOverflowXamlIsland`
  (overené). Pri inom jazyku alebo inom prvku Enter **nepošle** (riziko aktivácie cudzej ikony) a zobrazí oznam
  „Fokus je v oblasti oznámení…“. Po flyoute zostáva fokus v paneli úloh (S4) – lišta ho nevracia.
- **UIPI/fokus:** pred odoslaním sa zistí elevácia popredia (`OpenProcessToken`/`TokenElevation`); ak je zvýšené
  a lišta nie, skratka sa **neodošle** a zobrazí sa oznam (UIPI by ju ticho zahodil – S4). Pri držanom Shift/Ctrl/Alt/Win
  sa neodošle (`Win+Shift+S` = výstrižok). Po odoslaní sa účinok overí jedným časovačom; úspech sa nikdy nehlási,
  zlyhanie áno (oznam 7 s). Ak elevácia nie je zistiteľná (proces nemožno otvoriť), skratka sa pošle a rozhodne overenie.
- **Neoverené na tomto stroji:** popredie spustené ako správca (vyžaduje UAC – výsledok S4 platí s jeho výhradou),
  Win klávesy zakázané politikou, iný jazyk UI (Enter sa nepošle – overené len unit testom), skrytý panel úloh.
- Alternatíva k Search po MVP: vlastné vyhľadávacie okno nad Windows Search indexom.

## 6. Médiá

**[Overené]** `GlobalSystemMediaTransportControlsSessionManager` (Windows.Media.Control): od Windows 10 1809,
dokumentácia uvádza capability `globalMediaControl` (relevantné pre balené aplikácie).
`...PlaybackControls` ponúka príznaky `IsPlayEnabled`, `IsPauseEnabled`, `IsPlayPauseToggleEnabled`,
`IsNextEnabled`, `IsPlaybackPositionEnabled` a ďalšie.

**Návrh:**
- Samostatné vlákno s WinRT (MTA); udalosti sa posielajú do UI vlákna cez správu okna. UI vlákno jediné
  sahá na okná.
- Odber: zmena aktuálnej session / zoznamu sessions; pre vybranú session `MediaPropertiesChanged`,
  `PlaybackInfoChanged`. Časová os len keď je otvorený popover (šetrí CPU).
- **Výber session (rozhodnuté po S5):** systémová „aktuálna session“ sa nepoužíva ako zdroj pravdy (v S5 sa
  prepla na nehrajúcu šumovú session Riot Client). Poradie:
  1. používateľom zvolená aplikácia (podľa `SourceAppUserModelId`), ak je nakonfigurovaná a má session;
  2. inak session v stave `Playing` s platnými metadátami (neprázdny názov);
  3. ak zostáva viac vierohodných kandidátov (napr. dve hrajúce), výber nechá na používateľovi (výber v popoveri
     a zapamätanie voľby); session bez názvu a bez obalu sa neponúkajú.
- Tlačidlá Predchádzajúca / Prehrať-Pauza / Ďalšia sú vždy viditeľné; nepodporovaný príkaz (`Is*Enabled == false`)
  = zakázané (stlmené) tlačidlo bez akcie (§8.3). Nie je to Spotify-špecifické.
- **Zánik session:** keď session zmizne (zatvorenie prehrávača, `Closed`, `SessionsChanged`), položka médií sa čisto
  vyprázdni (žiadny starý text ani tlačidlá) a odhlásia sa jej handlery.
- **Časová os:** nezobrazuje sa nulová časová os a seek sa neaktivuje, kým session nehlási platnú časovú os
  (`EndTime > 0` a platná pozícia); v S5 mal Spotify krátko po štarte `pos=0/end=0`.
- **Spotify bez session:** ak beží `Spotify.exe`, ale GSMTC pre neho nemá session, lišta zobrazí stručnú nápovedu
  (napr. „Spotify nezdieľa ovládanie médií – skúste v Spotify Nastavenia → „Show desktop overlay when using media keys““).
  Text opisuje **pozorovanú koreláciu**, nie oficiálne potvrdenú príčinu.
- **Bez záložnej cesty:** `WM_APPCOMMAND` / čítanie titulku okna **nie je súčasťou MVP**.
- Obal albumu / náhľad videa: zobrazený v položke médií, načítaný na media vlákne, cache podľa obsahu (§8.3).

**Limity:**
- Len aplikácie, ktoré publikujú SMTC session. Spotify desktop publikuje session len za podmienok z §6.1.
- Dostupné príkazy závisia od session; reklamy a nezvyčajné metadáta môžu zobraziť neúplné údaje.
- `stop` (napr. v prehliadači) zatvorí session; nepoužívať ako bežné tlačidlo.

**Implementované (2026-11-20, `transom-media` + `transom-core::media`):** MTA vlákno, odber `SessionsChanged`,
`MediaPropertiesChanged`, `PlaybackInfoChanged` (tokeny sa odhlasujú), zlúčenie dávky udalostí (30 ms), snímka sa zverejní
len pri zmene zobrazených údajov (bez časovej osi). `TimelinePropertiesChanged` sa **neodoberá** (Spotify ~15×/min).
Pozícia sa v statickej lište **nezobrazuje**; časová os sa len validuje (`Timeline::valid`: `end > start`, pozícia v rozsahu,
`LastUpdatedTime > 0`), takže `pos=0/end=0` nikdy nevedie k falošnej nule. Seek nie je v lište (prípadný popover ho
povolí len pri platnej osi). Preferovaná aplikácia: argument `--media-app` (kým nie je `transom-config`). Výber pri viacerých
vierohodných: indikátor `i/n` za názvom, klik prepne a voľba platí, kým session existuje; **klik má prednosť aj pred
nakonfigurovanou aplikáciou** (inak by prepínač pri `--media-app` nič nerobil) a nakonfigurovaná zostáva medzi kandidátmi. Session tej istej aplikácie
dostanú kľúč `AUMID#2`… podľa poradia v `GetSessions` (poradie sa môže pri zmene zoznamu zmeniť – limit).
Nápoveda pre Spotify: ak beží `Spotify.exe` (Toolhelp snímka pri media udalosti a pri minútovom ticku – bez vlastného
budenia) a GSMTC nemá Spotify session – **aj keď je zvolená iná session** (napr. pozastavená karta prehliadača, situácia z S5).
Má vlastné miesto za médiami (oblasť oznamu; dočasný oznam ju na chvíľu nahradí), text hovorí „podľa pozorovania pomáha“,
kliknutím sa skryje do ukončenia Spotify.
Obal sa načítava od 2026-11-20 (§8.3).

### 6.1 Výsledky S5 (2026-11-18; `spikes/s5-media/RESULTS.md`, `data/`)
- **GSMTC funguje bez package identity** pre nebalenú Win32 aplikáciu: `RequestAsync` OK, session sa čítajú aj sledujú.
  Overené s **Brave (YouTube)**, **Windows Media Player (Store)** a **Spotify desktop 1.3.9.2055** (nezmenená inštalácia).
- **Metadáta a obal:** Brave 19–22 kB, Spotify 168 kB; Media Player pri wav súboroch bez interpreta/albumu/obalu.
- **Príkazy overené s účinkom na stave/titule/pozícii** (len tie, ktoré príznaky povoľujú): toggle, pause, play, seek (Brave, Spotify),
  next/prev (Media Player, Spotify), shuffle (Media Player, Spotify, vrátené). `stop`: Spotify → Paused (session ostane),
  Brave → session sa zatvorí. Seek pri Media Playeri je netestovateľný (skladba 9 s) – chyba testu, nie prehrávača. Repeat len čítaný.
- **Spotify a nastavenie:** pred zapnutím „Show desktop overlay when using media keys“ Spotify **nepublikoval** žiadnu session (overené
  pri hraní aj pri pauze/play); po zapnutí sa session objavila. Je to **pozorovaná korelácia** (komunitné vlákna prepínač s GSMTC
  nespájajú, oficiálne potvrdenie chýba). Produkt je závislý od používateľského nastavenia tretej strany.
- **30-minútové behy (odber udalostí, načítanie obalov všetkých session pri každej udalosti):**
  - Media Player riadený každých 20 s + Brave: 300 zmien metadát, 61 zmien prehrávania, 363 načítaní obalu (7,5 MB), 89/89 príkazov, 0 chýb;
    WS 20,05→20,24 MB, private 3,42→3,51 MB (sklon +0,006 MB/min), CPU 0,062 % jadra; handles 243→260 (kolísalo, po návrate Brave session).
  - **Spotify bez riadenia** (len počúvanie): 16 zmien skladby, 449 zmien časovej osi, 34 načítaní obalu (2,7 MB), 0 chýb;
    WS 19,51→19,88 MB, private 2,71→3,02 MB (sklon +0,003 MB/min), handles 218→220, CPU 0,014 % jadra.
  - Pamäť po zahriatí (5 min) stabilná v rámci šumu vo oboch behoch.
- **Limity S5:** n=1 na beh, jeden stroj; Spotify beh mal málo udalostí; riadenie robil nástroj, nie používateľ; `pos=0/end=0` u Spotify
  tesne po štarte; nevyriešený výber pri viacerých hrajúcich (rieši návrh vyššie); netestované Store verzia Spotify, iná verzia
  Spotify, reklamy a podcasty, viac zdrojov v jednom prehliadači.

## 7. Vykresľovanie a UI: rozhodnutie podľa merania

**Princíp:** nepredpokladáme, že Direct2D alebo egui bude ľahšie či ťažšie. Zatiaľ **neexistuje žiadne
meranie** a všetky očakávania nižšie sú **[Odhad]**, ktorý prototyp má vyvrátiť alebo potvrdiť.

- Hostiteľ lišty (AppBar, hooky, DPI, SendInput, power, GSMTC vlákno) je v `transom-platform` a je
  nezávislý od renderera. Každá varianta potrebuje vlastný HWND, takže integrácia AppBaru sa nelíši.
- Renderer je za malým rozhraním `Painter` (zmerať text, vyplniť obdĺžnik, nakresliť ikonu, orezanie,
  prezentovať zmenenú oblasť). Backend sa vyberie v bráne G1 po meraní.

### 7.1 Kandidáti (licencie [Overené] na crates.io)

| ID | Variant | Závislosti a licencia | Hypotéza na overenie |
|---|---|---|---|
| R0 | GDI (`DrawText`, DIB s alpha, `UpdateLayeredWindow`) | len `windows` 0.62.2, MIT OR Apache-2.0 | Najnižšia pamäť a žiadne GPU. Riziko: alpha a ClearType na priehľadnom pozadí, vzhľad ikon. |
| R1 | Direct2D + DirectWrite, `ID2D1HwndRenderTarget` | len `windows` | Najlepší text, malé závislosti. Neznáme: GPU vs. softvérový režim, pamäť. |
| R2 | Direct2D + DirectWrite + DirectComposition (swap chain) | len `windows` | Potrebné pre kvalitný backdrop/priehľadnosť; **[Odhad]** vyššia pamäť než R1. |
| R3 | CPU: tiny-skia 0.12.0 (BSD-3-Clause) + cosmic-text 0.19.0 (MIT OR Apache-2.0), výstup cez `UpdateLayeredWindow` alebo softbuffer 0.4.8 | viac crate-ov | Prenosné, menej GPU. Neznáme: kvalita hintingu a čas rasterizácie. |
| R4 | egui/eframe 0.37.0 (MIT OR Apache-2.0), predvolený backend | veľký strom závislostí | Najrýchlejší vývoj. Neznáme: footprint, ovládanie HWND po vytvorení okna, správanie v pokoji. |

Mimo prototypu: **Slint** (licenčné podmienky z minulého kola: GPL-3.0 alebo Royalty-free 2.0 s atribúciou
a obmedzeniami – nezlučiteľné s jednoduchým MIT), Tauri/WebView2, iced a xilem/floem (zrelosť, API pred 1.0).
Ak to chcete, iced možno doplniť ako R5.

### 7.2 Spoločná scéna prototypu (rovnaká pre všetkých)
Plnošírkový pás 28 lp na dvoch monitoroch so zmiešaným DPI; vľavo 2 ikony a text média s elipsou,
vpravo 3 ikony/text a čas; hover zvýraznenie jednej položky; minútová zmena času; simulovaná zmena textu média
každých 5 s; varianty pozadia: plné a Mica/Acrylic (kde to backend dovolí). Každý prototyp používa ten istý AppBar kód.

### 7.3 Meranie
- Scenáre: **A** 10 min pokoja, 1 monitor; **B** 10 min, 2 monitory so zmiešaným DPI; **C** 60 s pohybu myši
  po lište; **D** zmena textu každých 5 s; **E** 30 min pokoja (rast pamäte).
- Metriky: pracovný set, súkromné bajty, CPU čas (rozdiel `TotalProcessorTime`), počet prebudení/prepnutí
  kontextu (ETW/WPR, Process Explorer), prezentované snímky v pokoji (PresentMon alebo ETW), GPU pamäť
  (Správca úloh), počet vlákien a handle, čas štartu, veľkosť `.exe`, čas buildu, počet tranzitívnych závislostí,
  rozsah kódu prototypu a čas vývoja (miera náročnosti).
- Každý beh 3×, uvádza sa medián. Zapísať: verziu Windows (v tomto prostredí 10.0.26300, **[Overené]**),
  GPU a ovládač, DPI a rozlíšenia monitorov.
- Vzhľad: snímky obrazovky pri 100/150/200 % (text, ikony, hrany, backdrop), hodnotené vizuálne a uložené.

### 7.4 Rozhodovacie pravidlo (brána G1; prahy schválené používateľom 2026-11-16)
1. **Vyradenie (tvrdé brány):**
   - **Priemerné CPU v pokoji < 0,1 %**, meraný 10 minút (scenáre A, B). Vždy sa uvádza aj absolútny CPU čas
     (`TotalProcessorTime`) za celé obdobie.
   - **Žiadna súvislá slučka prekresľovania alebo animácie** v pokoji. Ojedinelé prekreslenia pri minútovom ticku
     hodín a pri aktualizácii médií sú očakávané a povolené. Overenie: počet prezentovaných snímkov a prebudení
     v pokoji (PresentMon/ETW) – musia zodpovedať iba týmto udalostiam.
   - **Pamäť sa po zahriatí stabilizuje** v rámci šumu merania (scenár E, 30 min); nevyžaduje sa absolútna nula zmeny.
     Definícia zahriatia a šumu sa určí pri prvom meraní (napr. prvých 5 min zahriatie; šum z opakovaných behov).
   - **Správne AppBar správanie** (S1, S2, S3) a správne DPI.
2. **Hodnotenie zvyšných** (len orientačná pomôcka, nie presné bodovanie): footprint 40 %, vzhľad (text, backdrop) 25 %,
   náročnosť 20 %, kontrola nad HWND 15 %. Kľúčové sú tvrdé brány: správny AppBar a nízka spotreba v pokoji.
3. **Pri nerozhodnom výsledku:** R1 (Direct2D/DirectWrite) kvôli nulovým externým závislostiam a kvalite textu
   (**[Odhad]**, ide len o rozhodovací kompromis, nie o tvrdenie o výkone).

### 7.5 Praktická cesta
Hneď postaviť `Painter` a hostiteľa AppBaru, prototypy R0–R4 vyvinúť ako malé samostatné binárky v `spikes/`,
rozhodnúť po G1. Žiadny backend nie je pred meraním "víťaz".

### 7.8 Rozhodnutie o rendereri pre MVP (2026-11-17, po S6/S7; v produkcii je GDI, pozri poznámku z auditu §8.14)
**Zvolené predbežne: R0 (GDI + `UpdateLayeredWindow`).** R1 (Direct2D + DirectWrite) zostáva záložný, ak neskoršia
skutočná kontrola UI ukáže významný rozdiel v kvalite textu alebo v udržiavateľnosti. R4 (egui) neprešiel tvrdou
bránou CPU pri čítaní „% jedného jadra“ a má ~5× väčšiu pamäť; R2/R3 neboli skúmané.

Namerané (medián n=3, podrobnosti a surové dáta: `spikes/s7-measure/RESULTS*.md`, `data/`):
| | R0 GDI | R1 D2D+DWrite | R4 egui |
|---|---|---|---|
| CPU idle, 1 jadro (A 1 mon. / B 2 mon.) | ~0 / ~0 | 0,032 / 0,035 % | 0,114 / 0,143 % |
| WS / private (2 lišty) | 11 / 2 MB | 45 / 56 MB | 226 / 434 MB |
| GPU dedikovaná | 0 | 14 MB | 248 MB |
| CPU pri pohybe myši (umelom, 60 s) | 0,18 % | 0,34 % | 5,5 % |
| Čistý release build, závislosti, exe | 12 s, 22, 0,22 MB | 12 s, 23, 0,23 MB | 56 s, 229, 13,85 MB |
G1 (idle CPU <0,1 %, bez slučky prekresľovania, pamäť stabilná v ~10 min okne, AppBar S1–S3, DPI flip 100↔150 %):
R0 aj R1 prešli; R4 prešiel len pri čítaní „% z celkového CPU“. Výsledok R0 vs. R1 **nie je remíza**, preto sa
pravidlo „pri remíze R1“ (§7.4 bod 3) neuplatňuje; rozhodujú nižšia pamäť, GPU a CPU u R0.

**Limity merania (nie sú to overené výsledky):**
- 30-minútový test pamäte (scenár E) sa **nerobil**; „pamäť stabilná“ platí len pre ~10-minútové okno.
- Vzhľad pri 150 %: prvé „150 %“ snímky vertikálneho monitora boli v skutočnosti pri 100 % (DPI sa medzitým vrátilo).
  Pri DPI flipe sa zachytila len jedna snímka vertikálnej lišty pri 150 % na kandidáta (`%TEMP%\s6\vis\r*-vert150.png`),
  nie systematické vizuálne hodnotenie; 200 % sa nemeralo. Kvalita textu je posúdená okom zo screenshotov.
- Pohyb myši (`SetCursorPos`) a médiá (3 rotujúce reťazce) boli simulované; scenár D bežal pri 100 % + 100 %.
- Jedno meranie buildu; bez PresentMon/WPR (prekreslenia = vlastné počítadlo); jeden stroj.
- Riziká R0 na overenie neskôr: kvalita ikon/textu pri ďalších fontoch a mierkach, alfa/priehľadnosť (MVP je
  nepriehľadné pozadie podľa S9), vlastný layout a orezanie textu bez DirectWrite.

### 7.6 Stack mimo renderera
`windows` 0.62.2 (MIT OR Apache-2.0), `windows-future` 0.3.2 (tranzitívne cez `windows`, `.join()`; pôvodne uvedené 0.100.0 bolo chybné), `serde` 1.0.229, `toml` 0.9.8, `tracing` 0.1.44 (MIT).
Vlastná tray ikona cez `Shell_NotifyIcon`. Kontrola licencií cez `cargo-deny` v CI.

### 7.7 Grafické okno nastavení
- *Stav 2026-11-20 (§8.4):* `transom-settings` existuje – vlastné GDI cez `transom-render` (nie egui, S8 sa nerobil), spúšťa sa
  z ponuky na pravom tlačidle lišty, konfigurácia je JSON (nie TOML).
- **Samostatný proces `transom-settings`** spustený z ponuky tray ikony; jedna inštancia (pomenovaný mutex).
  Bežiaca lišta tak nenesie náklady na UI toolkit pre nastavenia.
- **Toolkit:** egui/eframe (MIT OR Apache-2.0) je prijateľný, lebo proces žije len počas otvoreného okna;
  alternatíva je natívne Win32. Rozhodnutie po spike S8 (vzhľad na Windows 11, tmavý režim, DPI, čas štartu).
- **Komunikácia:** spoločný crate `transom-config` s atomickým zápisom (dočasný súbor a premenovanie).
  Settings po uložení pošle zaregistrovanú správu (`RegisterWindowMessage`) skrytému oknu lišty, ktorá
  konfiguráciu znovu načíta; zmeny platia naživo. "Identifikovať" posiela správu rovnakou cestou.
- **Obsah:** výber monitorov (zoznam + náhľad, prepínač po monitore), výška lišty, fullscreen `hide`/`keep_visible`,
  položky a poradie, politika výberu média, autostart, téma (svetlá/tmavá/podľa systému).

## 8. Etapy

### Etapa 0 – technický spike (jednorazové prototypy v `spikes/`)
Predpoklad: **[Overené]** na tomto stroji `cargo`/`rustc` nie sú v PATH – treba nainštalovať Rust
(rustup, MSVC toolchain) a skontrolovať Visual Studio Build Tools. Inštaláciu treba schváliť.

| Spike | Cieľ / kritérium |
|---|---|
| S1 AppBar | Plnošírková lišta hore: maximalizované okná končia pod ňou; stav po páde; notifikácie o zmene polohy. |
| S2 Monitory + DPI | 2+ lišty, mix DPI (100/150/200 %), pripojenie/odpojenie, monitory nad sebou, stabilné ID. |
| S3 Fullscreen | Detektor + `ABN_FULLSCREENAPP`; klasický vs borderless; hide vs keep_visible; vplyv na výkon hry. |
| S4 Skratky | `Win+S`, `Win+B` cez `SendInput`; matica UIPI/fokus/klávesnica. |
| S5 Médiá | GSMTC: Spotify, prehliadač, ďalší prehrávač; udalosti, tlačidlá, obal, bez package identity. **Hotové 2026-11-18**, výsledky §6.1. |
| S6 Vykresľovanie | R0–R4 podľa §7.1, spoločná scéna. |
| S7 Meranie | Protokol §7.3 pre všetkých kandidátov. |
| S8 Okno nastavení | Štart z tray ikony, toolkit, tmavý režim, DPI, IPC s lištou a živá zmena. |
| S9 Vzhľad full-width | Rohy, Mica/Acrylic na okne bez titulku, oddeľovacia čiara, správanie na monitore s inou mierkou. |

Brána G1 po S6/S7 (výber renderera).

### Etapy implementácie
1. Kostra: `BarWindow`, AppBar, `transom-config`, hodiny, `Painter` s vybraným backendom.
   *Stav (2026-11-18): hotové workspace (`Cargo.toml`, toolchain 1.100.0, `deny.toml`, `ci.yml`, MIT), `transom-platform`
   (AppBar hostiteľ, monitory/DPI, reconcile pri `WM_DISPLAYCHANGE`, `TaskbarCreated`, `ABN_FULLSCREENAPP`) a `transom-app`
   (jednofarebné pozadie, jedna inštancia). Ešte nie: `transom-config`, `Painter`/R0, hodiny, položky. Overené: AppBar brána,
   DPI flip 100↔150 %, odpojenie/pripojenie monitora, idle 0 CPU; nie overené: CI (lokálny Git od 2026-11-21, §8.14; CI sa ešte nespustilo), fullscreen exkluzívny, hot-plug naživo (iba logika `reconcile` a simulácia, pozri §8.10 a §8.14).*
   *Stav (2026-11-20): pribudol `transom-render` (`Painter` trait so zmluvou, témy svetlá/tmavá/podľa systému, scéna,
   backend R0 GDI) a hodiny v strede lišty (prekreslenie len pri štarte, zmene polohy/DPI, zmene témy a zmene minúty;
   časovač na hranicu minúty, `WM_TIMECHANGE`/resume → prepočet). Téma sa zatiaľ volí len argumentom `--theme`
   (`transom-config` ešte nie je). Overené: build/clippy/fmt/21 unit testov/cargo-deny, vizuálne 100 % a 150 % (svetlá aj
   tmavá), prepnutie témy za behu, prekreslenie na hranici minúty. Neoverené: skutočná zmena časovej zóny/systémového
   času (len simulovaná správa `WM_TIMECHANGE`), 12-hodinový formát, 200 % DPI.*
   *Stav (2026-11-20, míľnik stavových položiek): pozri §8.2.*
2. `MonitorManager`, výber monitorov, DPI.
3. Okno nastavení (výber monitorov, výška, fullscreen) a tray ikona aplikácie.
4. Fullscreen služba a nastavenie.
5. Batéria (`GetSystemPowerStatus` + power notifikácie), tlačidlá Tray a Search. *Hotové 2026-11-20 (§8.2).*
6. Médiá. *Hotové 2026-11-20 (§8.2), ovládanie, obal a prístupnosť 2026-11-20 (§8.3); popover nie.*
7. Autostart (`HKCU\...\Run`), balenie, dokumentácia, CI. Vydanie podmienené nameranými prahmi.

### 8.2 Míľnik: stavové položky (2026-11-20)

**Implementované**
- `transom-core` (bez Win32): model `StatusItem` (časti ikona/text/batéria, akcie bez closures, `drop_rank`), rozloženie
  (Search + médiá + oznam vľavo; Tray, batéria, hodiny vpravo; hodiny sa presunuli zo stredu doprava podľa §1), politika
  výberu médií, validácia časovej osi, batéria zo `SYSTEM_POWER_STATUS`.
- Úzka šírka: najprv sa skracujú pružné texty (názov skladby max 420 lp, min 60 lp; nápoveda; oznam), potom sa skryjú,
  potom sa zahadzujú položky v poradí oznam → médiá → Tray → Search → batéria; hodiny zostanú vždy (v krajnom prípade
  orezané). Na širokých monitoroch sa nič nerozťahuje (stred voľný).
- `transom-platform`: `power` (registrácia `GUID_ACDC_POWER_SOURCE`, `GUID_BATTERY_PERCENTAGE_REMAINING`,
  `GUID_POWER_SAVING_STATUS` + `PBT_APMPOWERSTATUSCHANGE` → `BarEvent::PowerChanged`, odregistrovanie pri ukončení),
  `input` (SendInput, UIPI/modifikátory, overenie popredia, UI Automation na krátkom MTA vlákne), `process`, myš na lište
  (`WM_MOUSEMOVE`/`WM_MOUSELEAVE`/`WM_LBUTTONUP`, bez aktivácie), pomenované jednorazové časovače, `Notifier` (PostMessage
  z iných vlákien).
- `transom-media`: GSMTC služba (§6). `transom-render`: scéna kreslí položky, ikony Segoe Fluent Icons 16 lp
  (`ANTIALIASED_QUALITY` – ClearType robil farebné okraje na pauze; ikony médií od §8.3 vektorové), hover len na klikateľných častiach, batéria kreslená
  obdĺžnikmi (≤ 15 % červená výplň), blesk pri nabíjaní.
- Batéria sa zobrazí len pri systémovej batérii (`BatteryFlag & 128 == 0`); bez dotazovania.
- Prekresľovanie len pri: štarte, polohe/DPI, téme, minúte, napájaní, zmene zobrazených médií, zmene hover časti, kliku
  a konci oznamu. Aplikácia porovná položky pred/po zmene stavu a prekreslí len pri rozdiele.
- WM_APPCOMMAND záloha **nie je** pridaná (GSMTC ovládal Brave aj Spotify).
- Nastavenia, `transom-config`, popover médií, tray ikona aplikácie: mimo tohto míľnika.

**Overené na tomto stroji** (Windows 11 26300, 2560×1440 @100 % primárny + 1080×1920 @100 %, desktop bez batérie;
rozlíšenie, DPI, systémový čas ani časová zóna sa nemenili)
- fmt, clippy `-D warnings`, release build, 57 unit testov (core 26, render 12, platform 12, app 6, media 1), `cargo deny`
  (advisories/bans/licenses/sources ok). Release exe 314 KB (predtým ~0,22 MB s R0 bez médií).
- Batéria: `GetSystemPowerStatus` → skrytá (desktop). Registrácia power notifikácií vyvolá 2× `PowerChanged` pri štarte,
  bez zmeny obsahu. Viditeľný stav len **simulovaný** (`--sim-battery 57,charging` / `12`) na test vzhľadu – svetlá aj tmavá téma.
- Médiá: Brave (hrá) + Spotify (pozastavený) → vybraný Brave, len tlačidlo pauza (Brave nemá next/prev); klik na pauzu
  → `Pause` → Paused; pri dvoch pozastavených indikátor `2/2`, klik prepne na Spotify (prev/play/next podľa príznakov),
  späť na Brave a Play → pôvodný stav obnovený. Časová os oboch platná (`tl=ok`). Chyby príkazov 0.
  S `--media-app spotify` (Spotify pozastavený, Brave hrá): zvolený Spotify `1/2`, klik → Brave `2/2`, klik → Spotify.
- AppBar brána po zmenách hostiteľa (`tools/gate-appbar.ps1`): rezervácia 28/-377, maximalizované okno pod lištou,
  `ABN_FULLSCREENAPP` skryje/obnoví lištu, po zabití procesu plocha späť na 0/-405 za 0,12 s.
- Search a Tray z lišty: §5.1. Pracovná plocha po každom behu obnovená na pôvodné hodnoty, fokus vrátený do Brave.
- **Idle meranie** (n=1, okno 271 s bez interakcie, 2 lišty, Brave hrá YouTube, Spotify pozastavený): CPU < 15,6 ms
  (pod rozlíšením `TotalProcessorTime`), WS 19,15 → 19,29 MB, private 2,99 → 2,93 MB, handles 205 → 188, vlákna 11 → 9;
  prekreslenia len 1× za minútu na lištu. Celý 400 s beh: 32 prekreslení (16 párov), 2 media notifikácie. Ukončenie
  (ABM_REMOVE + join media vlákna) 34 ms. Pamäť je vyššia než R0 bez médií (11 MB WS v S7) kvôli WinRT/GSMTC – v súlade s S5 (~20 MB).

**Limity a riziká**
- Bez dlhého behu (30 min) s médiami v lište; meranie n=1; bez PresentMon/ETW (prekreslenia z vlastného logu).
- Každé prekreslenie v behu s logovaním hoveru (400 s) má v logu príčinu (štart, médiá, hover, klik, oznam, minúta).
  Krátka séria prekreslení v skoršom behu nastala pred pridaním logovania hoveru (pravdepodobne pohyb kurzora); dva
  „stratené“ kliky boli chyba testovacieho príkazu (`Select-Object -First 1` ukončil skript pred klikom), nie lišty.
- Idle meranie len s hrajúcim Brave; beh s pozastavenými médiami ani s `--no-media` sa nerobil.
- Živo overené: Search, Win+B → Enter → flyout, pauza/play, prepínanie session (aj s `--media-app`), skrytá batéria.
  Len unit testami alebo simuláciou: viditeľná batéria (simulovaná), nápoveda Spotify, oznamy pri zvýšenom popredí,
  držaných modifikátoroch a odmietnutom vstupe, iný jazyk UI pri Tray.
- Tray Enter len pre anglické UI; ostatné jazyky končia pri fokuse v oblasti oznámení.
- Hover/klik invaliduje obe lišty (jednoduchosť; 2 lišty, zanedbateľné).
- Spotify nápoveda a viditeľná batéria sú overené len unit testami (nastavenie Spotify sa nemenilo, stroj nemá batériu).
- Kontrola `Spotify.exe` cez Toolhelp pri minútovom ticku, kým Spotify nemá session (**[Odhad]** ~1 ms).
- Ikony v svetlej téme sú s grayscale vyhladzovaním trochu svetlejšie než text.

**Ako bezpečne otestovať to, čo sa tu nerobilo**
- Zmena času/časovej zóny: na VM alebo testovacom stroji; inak `tools/sim-settings.ps1 -What timechange` (len správa).
- UIPI: otvoriť okno ako správca (UAC), dať ho do popredia a kliknúť na Search – očakávaný oznam bez odoslania skratky.
- Batéria: notebook; odpojiť/pripojiť napájanie, sledovať `PowerChanged` v `--log`.
- Spotify nápoveda: na testovacom profile vypnúť „Show desktop overlay when using media keys“ a reštartovať Spotify.
- DPI 150/200 %: len na testovacom stroji (tu sa DPI nemenilo).

**Ďalší míľnik (prekonané – `transom-config` a nastavenia sú hotové v §8.4):** `transom-config` (`%APPDATA%\transom\config.toml`: téma, výška, monitory, preferovaná media aplikácia,
zapamätanie výberu session) + tray ikona aplikácie (Nastavenia/Ukončiť) a živé načítanie konfigurácie (S8, §7.7);
potom 30-min meranie lišty s médiami a vizuálna kontrola 150/200 % na testovacom stroji.

### 8.3 Míľnik: ovládanie médií, obal, prístupnosť (2026-11-20)

**Implementované**
- **Tlačidlá:** Predchádzajúca, Prehrať/Pauza, Ďalšia sú vždy viditeľné pri zvolenej session. Stredné ukazuje Pause len
  v stave `Playing`, inak Play (Paused/Stopped/Opened). Prechodný `Changing` (pri prepínaní skladby) ponechá predošlý
  stav (`stable_state`) – ikona nebliká. Nepodporovaný príkaz → zakázané tlačidlo: stlmená farba, bez hoveru/stlačenia,
  klik nič neodošle, tooltip „… – zdroj to nepodporuje“, MSAA stav `UNAVAILABLE`. Prev/Next volajú `TrySkipPrevious/Next`
  zvolenej session.
- **Ikony médií:** vlastné vektorové tvary (`transom-render/src/icons.rs`), zvislé hrany zarovnané na pixely, šikmé hrany
  supersampling 4×4, farba = lineárna zmes popredia a pozadia (test: žiadny pixel mimo úsečky fg–bg → žiadne farebné
  okraje – vlastnosť konštrukcie, test ju overuje nezávisle odvodením podielu z jedného kanála), box 14 lp
  (14/21/28 px pri 100/150/200 %). Kreslia sa cez nové `Painter::blit` (GDI `SetDIBitsToDevice`).
  Search/Tray zostávajú z písma Segoe Fluent Icons.
- **Stavy:** hover (tlačidlá a klikateľný text), stlačené (`WM_LBUTTONDOWN`; akcia až pri uvoľnení nad tou istou časťou,
  odchod myši stlačenie zruší), nové farby témy `pressed`, `disabled`, `placeholder` (svetlá/tmavá/podľa systému).
- **Tooltipy:** Win32 tooltip control (`TTS_ALWAYSTIP` – lišta je neaktívne okno; `cbSize` V2 kvôli comctl32 v5 bez
  manifestu); aktualizujú sa len pri zmene zoznamu oblastí. **Prístupné názvy:** MSAA `IAccessible` na `WM_GETOBJECT`
  (`OBJID_CLIENT`): prvky s názvom, rolou (tlačidlo/statický text), stavom a `accDoDefaultAction` (pošle správu liste,
  ktorá akciu vykoná ako klik). Hit-testing, tooltipy aj MSAA berú jeden zoznam oblastí (`transom-platform/src/regions.rs`).
- **Obal / náhľad:** `MediaProperties.Thumbnail` → `DataReader` (max 8 MB) → hash obsahu → WIC (stredový štvorec,
  96×96 Fant, `32bppPBGRA`) na media vlákne; cache podľa hashu (max 24, nepoužívané sa zahodia). Náhľad sa číta len pri
  `MediaPropertiesChanged` danej session, pri prihlásení k session a po `SessionsChanged` (kľúče `AUMID#n` sa môžu
  prečíslovať; rovnaký obsah sa vďaka cache nededekóduje znova), nie pri zmene stavu prehrávania. Obal sa viaže na identitu
  média (session + názov + interpret + album, `resolve_art`): pri zmene skladby bez nového náhľadu sa starý zahodí, chýbajúci
  náhľad = neutrálny zástupný štvorec. V lište 22 lp (22/32/44 px pri 100/150/200 %), zložený na aktuálne pozadie v Ruste (box filter).
- Úzka šírka: obal sa skryje pred názvom, tlačidlá zostávajú až do zahodenia celej položky.
- Bez Spotify-špecifického API, bez sťahovania/scrapovania YouTube, bez `WM_APPCOMMAND`.

**Overené na tomto stroji** (Brave = jediný bežiaci prehliadač; Edge/Chrome netestované)
- **Spotify desktop 1.3.9.2055:** obal 100 547 B dekódovaný za 79–81 ms (prvé WIC volanie), ďalšie 2–12 ms. Klik **Next**
  z lišty: nový názov aj nový obal (iný hash) prišli **v tej istej** udalosti `MediaPropertiesChanged` – zastaraný obal sa
  nepozoroval; **Prev** vrátil skladbu, obal z cache bez dekódovania (citania 4, dekódovania 3, cache 1). Next pri
  pozastavenom Spotify spustil prehrávanie → Pause z lišty → stav obnovený (Paused, tá istá skladba); **pozícia v skladbe
  sa nezachovala**: Next/Prev začína skladbu od začiatku a medzi Prev a Pause hral Spotify ~11 s; pôvodnú pozíciu som
  pred testom nezaznamenal. Ikona Pause pri `Playing`, Play pri `Paused` overená na snímkach.
- **Neúmyselný zásah:** pri teste MSAA prvé vyvolanie prepnulo zdroj na Spotify, takže druhé (#2) poslalo skutočné
  **Prev** do Spotify (nie na zakázané tlačidlo Brave). Spotify zostal pozastavený na tej istej skladbe, bez zverejnenej
  zmeny; pozícia mohla byť znova vynulovaná. Test zakázaného tlačidla som potom zopakoval správne (Brave, #2 → `E_FAIL`).
- **YouTube v Brave:** GSMTC dodáva náhľad videa (25 797 B, dekódovanie 9–12 ms) – zobrazený v lište. Prev/Next pre jedno
  video nepodporované → zakázané. Play → `Playing` (ikona Pause), Pause → `Paused` (vrátené). Prechod autoplay na ďalšie
  video sa **netestoval** (zmenil by obsah používateľa). Či Chromium pri prechode nepošle nový názov so starým
  náhľadom, **nie je overené**: v takom prípade by sa starý obrázok na chvíľu naviazal na nové video (pravidlo identity
  chráni len pred prechodom *bez* nového čítania). Ochrana: každá ďalšia `MediaPropertiesChanged` náhľad znova prečíta,
  takže neskorý správny náhľad nesprávny nahradí. Overenie: nechať prebehnúť jeden autoplay pod `--log` a porovnať názov
  a hash v riadkoch `nahlad`.
- Ikona YouTube Play/Pause overená aj vizuálne (snímky pri `Playing` a `Paused`).
- Kliky na tlačidlá médií nemenili popredné okno (pred aj po kliku to isté okno; `WS_EX_NOACTIVATE`). Popredím bol
  počas týchto testov terminál, nie prehrávač; že lišta nevezme fokus prehrávaču, plynie z toho istého mechanizmu
  (overeného v §5.1 s Brave v popredí), samostatne s prehrávačom v popredí to v tomto míľniku nebolo zopakované.
- Klik myšou na zakázané Next (Brave): Down/Click prišli, žiadny hover, akcia ani príkaz.
- Hover, stlačenie (stlačenie a uvoľnenie mimo lišty → žiadna akcia) a tooltip na zakázanom Prev zachytené na snímkach.
- **MSAA z iného procesu** (`cargo run -p transom-platform --example msaa_probe`): 7 prvkov s názvami, role 43/41,
  zakázané Prev/Next `STATE_UNAVAILABLE`, polohy zodpovedajú rozloženiu, neplatné ID → `E_INVALIDARG`. Natívny UIA klient
  (`IUIAutomation`, ako Narrator) vidí tie isté prvky (Button/Text, `IsEnabled`). `accDoDefaultAction` na prepínači zdroja
  vykonal akciu; na zakázanom tlačidle vrátil `E_FAIL` a nič neodoslal. COM na UI vlákne netreba inicializovať.
  Spravovaný .NET klient (`System.Windows.Automation`) deti nevidí (vidí len okno) – obmedzenie tohto klienta.
  Samotný Narrator spustený nebol.
- **DPI 100/150/200 %** bez zmeny systémového DPI: `cargo run -p transom-render --example preview -- <adresár>` kreslí
  skutočným `render_bar` + `GdiPainter` do pamäťového DC pri 96/144/192 DPI (svetlá/tmavá, normál/hover/stlačené, obal aj
  zástupný symbol). Vektorové ikony ostré pri všetkých mierkach; v svetlej téme je ikona Search (písmo) pri 100 % svetlejšia.
- fmt, clippy `-D warnings`, release build, **78 unit testov** (core 35, render 21, platform 13, app 7, media 2 – vrátane
  skutočného WIC dekódovania), `cargo deny` ok. Release exe 364 KB (predtým 314 KB). Nová priama závislosť `windows-core`
  0.62.2 (už bola tranzitívne; makro `#[implement]`).
- AppBar brána po zmenách hostiteľa prešla (28/-377, maximalizované pod lištou, fullscreen skryje/obnoví, po zabití 0/-405
  za 0,11 s). Korektné ukončenie 0,3 s, upratanie 39 ms.
- **Idle** (n=1, 271 s, 2 lišty, Spotify aj Brave **pozastavené** – iné podmienky než §8.2, kde Brave hral): CPU 15,6 ms
  (= 1 tik `TotalProcessorTime`, 0,006 % jadra), WS 25,44 → 25,66 MB, private 3,75 → 3,75 MB, handles 259 → 251, vlákna 9.
  Oproti §8.2 (WS 19,2 MB, private 3,0 MB) **+6 MB WS / +0,75 MB private**; príčina **[Odhad]**: načítané WIC,
  comctl32 (tooltipy), oleacc a 3 obrázky po 36 KB. Meranie nasledovalo po MSAA/UIA skúškach a tooltipoch v tom istom behu. Prekreslenia: 20 za beh, každé má v logu príčinu (štart, médiá, hover/stlačenie, minúta).

**Limity**
- YouTube náhľad je ten, ktorý Brave (Chromium Media Session) odovzdá GSMTC; ak ho stránka alebo prehliadač nenastaví,
  zobrazí sa zástupný symbol. Spoľahlivé náhľady mimo GSMTC by vyžadovali podporovanú integráciu (napr. rozšírenie
  prehliadača s Media Session alebo YouTube Data API s kľúčom) – mimo rozsahu, nič také sa nerobí.
- Edge, Chrome, Firefox a ďalšie prehrávače netestované. Popover s väčším obalom a seek nie je.
- „Bez farebných okrajov“ platí pre štyri vektorové ikony médií; Search a Tray sú stále z písma (grayscale AA).
- Stlačená farba v tmavej téme je od pozadia odlíšená len mierne (tmavšia než hover).
- Idle meranie len s pozastavenými médiami a n=1; 30-minútový beh s obalmi sa nerobil.

### 8.4 Míľnik: aktivácia zdroja médií, rozloženie, hodiny a kalendár, využitie AI (2026-11-20)

**Schopnosti podľa zdroja (rozhodnutie pred implementáciou)**
| Funkcia | Zdroj | Stav |
|---|---|---|
| Klik na obal → aplikácia | `SHGetPropertyStoreForWindow` + `PKEY_AppUserModel_ID`, proces okna, `SetForegroundWindow`, `shell:AppsFolder\<AUMID>` | zdokumentované, implementované |
| Konkrétna karta/URL YouTube | GSMTC ju neposkytuje | **nedostupné** – aktivuje sa okno prehliadača (prednostne s titulkom videa); presná karta by vyžadovala podporovanú integráciu (napr. voliteľné rozšírenie prehliadača) |
| Hodiny, dátum, prvý deň týždňa | `GetTimeFormatEx`, `GetDateFormatEx`, `GetLocaleInfoEx` (`LOCALE_STIMEFORMAT`, `LOCALE_IFIRSTDAYOFWEEK`: 0 = pondelok … 6 = nedeľa, overené na Microsoft Learn) | implementované |
| Poloha natívneho flyoutu oblasti oznámení | žiadne verejné API | **nedostupné** – Win+B (+ Enter pre skryté ikony) zostáva; flyout Windows otvára pri paneli úloh (tu dole), nie pod našou lištou. Cudzie ikony sa nekopírujú. Vlastná ponuka transom (pravé tlačidlo na lište) nie je náhrada tray ikon |
| Claude Code tokeny | lokálne prepisy `~/.claude/projects/**/*.jsonl` – dokumentácia: „The entry format is internal to Claude Code and changes between versions“ | **odhad / nezdokumentovaný formát**, voliteľné |
| Claude Code odhad ceny relácie, limity Pro/Max | stdin JSON pre `statusLine` (code.claude.com/docs/en/statusline): `cost.total_cost_usd` („Estimated…“), `rate_limits.five_hour/seven_day.used_percentage` (len Pro/Max/Ultra, po prvej odpovedi) | zdokumentované, voliteľný most `transom-statusline` – používateľ ho sám nastaví |
| Codex tokeny a snímok limitu plánu | `~/.codex/sessions/**/*.jsonl` (`token_usage_record`, `token_count.rate_limits`) – dokumentácia spomína len `CODEX_HOME` a `history.jsonl` | **odhad / nezdokumentovaná schéma**, voliteľné; limit = „zaznamenané Codexom v čase X“, nie aktuálny stav |
| Fakturované tokeny a cena API | Anthropic Usage & Cost Analytics API (`/v1/organizations/analytics/usage` a `/cost`, Admin kľúč `sk-ant-admin` alebo kľúč `read:analytics`), OpenAI `/v1/organization/usage/completions` a `/costs` (admin kľúč; stránka čítaná čiastočne, názvy polí neoverené) | zdokumentované, **neimplementované** – tu nie je organizačný kľúč a pri spotrebiteľských plánoch (Codex plán `go`, Claude predplatné) by tieto API využitie nezahŕňali. Credential Manager preto nie je potrebný |
| OpenTelemetry (Claude Code `claude_code.token.usage`, Codex `[otel]`) | zdokumentované | neimplementované – vyžaduje vlastný OTLP prijímač; ďalší krok |
| Meter limitu predplatného (Pro/Max, ChatGPT) | žiadne verejné API (okrem statusline pre Claude) | nedostupné – zobrazuje sa len zaznamenaný snímok |

**Implementované**
- **Klik na obal** (`Action::MediaOpenSource`): aktivácia cez frontu požiadaviek riadiaceho okna (nie v `on_event`). Okno sa hľadá
  podľa AUMID okna, inak podľa procesu (GSMTC vracia `Spotify.exe`), inak odhadom podľa mena (označené). Výsledok sa overí
  časovačom po 400 ms (prepnutie popredia je asynchrónne). Ovládacie tlačidlá médií zostávajú neaktivujúce.
- **Hostiteľ popoverov** (`transom-platform/src/popup.rs`): fronta požiadaviek (popover, aktivácia, kontextové menu, spustenie
  programu) – `ShowWindow`/`SetForegroundWindow`/`TrackPopupMenu` sa volajú až v riadiacom okne, mimo výpožičky handlera.
  Popover je aktivovateľný (zámerná akcia), zatvorí sa pri strate aktivácie, Esc, opakovanom kliku, fullscreene, zmene DPI
  a odstránení lišty. Dvojitý buffer.
- **Kalendár** (klik na hodiny): mesiac podľa locale, šípky/Home, dnešok (akcent), zvolený deň (rámček + dlhý dátum v päte),
  týždeň podľa nastaveného prvého dňa; prekresľuje sa len pri zmene zobrazenia, minúte/sekunde a `WM_TIMECHANGE`.
- **Hodiny**: 12/24 h, úvodná nula, sekundy, deň, krátky/dlhý dátum, predvoľby – úprava formátu locale (`time_picture`)
  zachová oddeľovače a poradie jazyka. So sekundami sa prekresľuje len úsek hodín (`invalidate_span`).
- **Konfigurácia** `transom-config` (JSON, `version`, odchýlka od pôvodného `config.toml`): migrácia súboru bez verzie,
  zachovanie neznámych položiek a kľúčov, novší/poškodený súbor = len na čítanie (neprepíše sa), atomický zápis,
  `TRANSOM_CONFIG`/`TRANSOM_DATA` pre izolované behy. Zmenu oznamuje broadcast `transom-config-changed-v1`, lišta sa obnoví naživo.
- **Rozloženie**: skupiny vľavo/stred/vpravo, poradie a skrytie podľa konfigurácie. Stred je centrovaný, pri kolízii sa
  posunie do voľného miesta (nikdy neprekryje ľavú/pravú skupinu), pri nedostatku miesta sa zahadzuje podľa `drop_rank`.
  Vstup do nastavení, ktorý sa nedá skryť: pravé tlačidlo na lište → ponuka transom (*od §8.5 len na prázdnom mieste,
  nie nad ovládacím prvkom*).
- **Okno nastavení** `transom-settings` (samostatný proces, jedna inštancia, GDI cez `transom-render` – nie egui): stránky
  Rozloženie (drag & drop medzi stĺpcami Vľavo/Stred/Vpravo/Skryté, oko na skrytie, náhľad lišty skutočným rendererom,
  obnovenie predvolených; *od §8.5 bez stĺpca Skryté a oka – jediné políčko „Zobraziť v lište“*), Hodiny a kalendár (predvoľby, voľby, živý náhľad), AI a súkromie (zdroje predvolene vypnuté,
  čo presne sa číta, interval obnovy, „Vypnúť všetky zdroje“, „Zmazať lokálne dáta transom“, kopírovanie nastavenia statusLine).
- **Využitie AI** (`transom-usage`, položka „AI …“ a panel; *od §8.5 je položkou kruhový indikátor 5-hodinového limitu*): Dnes / 7 dní (dnešok + 6 dní, lokálne kalendárne dni) /
  kalendárny mesiac; podľa poskytovateľa a modelu; vstup, výstup, čítanie a zápis cache zvlášť; „≈ odhad … podľa cenníka
  k <dátum> – nie faktúra“ a „pri predplatnom sa za tokeny neplatí“; počet volaní bez ceny (neznámy model, `fast`, dlhý
  kontext nad hranicou tarify) – nikdy $0 namiesto „nevieme“; príznaky ZASTARANÉ (posledné čítanie starsie než 2× interval)
  a „neúplné: najstarší záznam …“. Zber beží na samostatnom vlákne len pri zapnutých zdrojoch: po zapnutí, pri otvorení
  panelu (dáta > 5 min), tlačidlom Obnoviť a v intervale (predvolene 60 min; *od §8.6 predvolene 5 min a nikdy castejsie*). Cache obsahuje len denné súčty
  (`%LOCALAPPDATA%\transom\usage-cache.json`).
- **Cenník** (`transom-usage/src/pricing.rs`): Anthropic k 2026-11-03 (Claude API skill: Opus 5.5 $4/$20, Sonnet 5.5 $2/$10,
  Fable 5.2 $8/$40, Haiku 5.5 $0,10/$0,50 do 100K; čítanie cache Opus/Fable $0,20, Sonnet $0,10, Haiku 0,1× **[Odhad]**
  podľa všeobecného pravidla; zápis 1,25×/2×, pri Haiku 5.5 **[Odhad]**);
  OpenAI k 2026-11-20 (developers.openai.com/api/docs/pricing, stránka bez dátumu verzie; GPT-6 Luna $0,10 / cache $0,01 /
  $0,50, do 272K vstupu). Codex `input_tokens` obsahuje cache (overené: total = input + output), preto necachovaný vstup =
  input − cached − cache_write.
- **Súkromie**: všetko predvolene vypnuté; čítajú sa len `projects/` a `sessions/` (nikdy `auth.json`, `history.jsonl` Claude,
  nastavenia); z riadkov len čísla, model a čas; cache a most statusline neobsahujú cesty, `cwd`, ID relácií (len hash) ani
  text; nič sa neodosiela. „Vypnúť všetky zdroje“ zastaví aj zápis mostu statusline (most číta konfiguráciu).

**Overené na tomto stroji** (nemenil sa čas, pásmo, rozlíšenie ani DPI; testy v izolovaných `TRANSOM_CONFIG`/`TRANSOM_DATA`,
skutočné `%APPDATA%\transom` a `%LOCALAPPDATA%\transom` nevznikli)
- Klik na obal: Brave (YouTube) – nájdené podľa **AUMID okna** (`Brave`), popredie overené; Spotify – podľa **názvu procesu**
  z GSMTC (`Spotify.exe`), popredie overené. Žiadny príkaz médiám sa neposlal; fokus vrátený do pôvodného okna. Prvá verzia
  overovala popredie hneď po `SetForegroundWindow` a hlásila falošné „Refused“ – opravené na asynchrónne overenie.
- Kalendár: otvorenie klikom na hodiny (popredie = popover), šípka na december, výber 17. 12. (rámček, päta), Esc zavrie a
  fokus sa vráti do predošlého okna. Prvý deň týždňa podľa locale (pondelok).
- Nastavenia: otvorené z ponuky na pravom tlačidle; drag & drop „Hodiny“ zo stĺpca Vpravo do Stred → `config.json`
  `"group": "center"`, lišta dostala broadcast a hodiny sú v strede (x ≈ 1280). Voľby hodín (24 h, nula, sekundy, deň,
  krátky dátum) sa prejavili v lište („Fri 26/11/20 15:41:13“ – krátky dátum je podľa nastavenia Windows tohto účtu).
  Zapnutie zdrojov AI zobrazilo položku „AI 123,47 M“ a panel s údajmi; záložka Mesiac a Obnoviť (notifikácia) overené.
- Reálne dáta: Claude 648 odpovedí v 10 súboroch (deduplikácia: 1 410 riadkov → unikátne odpovede), Codex 39 záznamov;
  zber 24 ms. Snímok Codex: plán `go`, 30 % z 30-dňového okna (zaznamenané Codexom).
- Regresia: Search (SearchHost), Tray (Win+B → flyout), Play/Pause na Brave (stav vrátený), AppBar brána
  (28/-377, fullscreen, 0/-405 za 0,12 s).
- **Idle, predvolená konfigurácia** (n=1, 271 s, 2 lišty, médiá pozastavené, AI vypnuté): CPU 15,6 ms (= 1 tik,
  0,006 % jadra), WS 23,00 → 23,04 MB, private 3,43 → 3,41 MB, handles 250 → 232, 10 prekreslení (minútové tiky).
- **So sekundami** (60 s, 2 lišty): pôvodne 0,21–0,23 % jadra (nezávisle od logovania); po cache fontov GDI a prekresľovaní
  len úseku hodín **0,13 % jadra** – nad cieľom 0,1 % pre idle; sekundy sú preto voliteľné a predvolene vypnuté.
- Most statusline: vstup s `cwd`, `session_id` → v súbore len čísla, hash relácie a model; neplatný vstup → vypíše „Claude“.
- fmt, clippy `-D warnings`, release build, **135 unit testov** (core 48, render 23, usage 21, platform 16, app 15, config 7,
  settings 3, media 2), `cargo deny` ok. Nové závislosti: `serde` 1.0.229, `serde_json` 1.0.154 (MIT OR Apache-2.0).
  Binárky: lišta 622 KB, nastavenia 293 KB, most 228 KB.

**Opravy po kontrole (overené živo)**
- Ponuka na pravom tlačidle nechávala lištu v popredí → po zatvorení sa popredie vracia predošlému oknu (Esc: terminál
  0x414AA → menu → terminál). „Využitie AI…“ z ponuky sa ukotví na monitor a miesto kliknutia.
- Sekundový tik zisťoval novú minútu z textu (nefungovalo pri 12 h s AM/PM) → podľa skutočnej sekundy; overené 12 h + sekundy
  („03:56:22 PM“, plná cesta presne pri 03:57:00 PM).
- `parse_iso_utc` mohol spanikáriť na ne-ASCII znaku (release = abort) → bezpečné rezy; súbory sa čítajú po bajtoch (stratovo).
- Rozpis zápisu do cache (`ephemeral_5m/1h`) bol v 8 odpovediach menší ako súčet → zvyšok sa počíta ako 5 min; po oprave sa
  mesačné súčty podľa modelu **presne zhodujú** s nezávislým výpočtom v Pythone (napr. opus-5-5: 297 odpovedí, zápis 848 046).
- Prvý zber pri štarte bežal bez notifiera a nechal príznak „načítava sa“ navždy → zber až po vytvorení hostiteľa + poistka.
- Obal, hodiny a položka AI dostali tooltip a prístupný názov (MSAA). „Zmazať lokálne dáta“ oznámi lište (broadcast), lišta
  zahodí aj údaje v pamäti; ak sú zdroje zapnuté, súčty sa pri ďalšej obnove vytvoria znova (text v nastaveniach).
  Overené: „Vypnúť všetky zdroje“ → všetky príznaky `false`; zmazanie → `usage-cache.json` preč, lišta prijala obe správy.

**Neoverené živo**: oko na skrytie, obnovenie predvolených, preusporiadanie v rámci stĺpca, prepísanie prvého dňa týždňa,
obnovenie minimalizovaného okna pri kliku na obal, zatvorenie popoveru druhým klikom alebo klikom mimo, kopírovanie statusLine,
most statusline so skutočným Claude Code (len syntetický vstup). Počas testov sa dvakrát omylom otvoril flyout skrytých ikon
(nepresný klik, zatvorený Esc).

**Limity**
- YouTube: aktivuje sa okno prehliadača, nie konkrétna karta; ak video nie je v aktívnej karte, okno sa vyberie bez zhody titulku.
- Spotify/Brave: zhoda podľa názvu procesu/AUMID; „odhad podľa názvu aplikácie“ sa použije, len keď nie je presná zhoda.
- Lokálne formáty Claude Code a Codex sa môžu zmeniť – zber potom nájde 0 záznamov (zobrazí sa „bez záznamov“), nie zlé čísla.
- Statusline most nebol overený s reálnym Claude Code (vyžaduje zmenu `~/.claude/settings.json`, ktorú transom zámerne nerobí);
  overený len syntetickým vstupom podľa dokumentácie.
- Odhady cien sú z tabuliek s dátumom (Anthropic z cache skillu, nie priamo z cenníkovej stránky); pre `gpt-6-luna` stránka
  nemá dátum verzie. Pri predplatnom nejde o platbu.
- Kalendár: popover je aktivovateľný (vezme fokus kým je otvorený); výber dňa zatiaľ nič ďalšie nerobí (žiadne udalosti).
- Okno nastavení: vlastné GDI ovládacie prvky – bez klávesnicovej navigácie a MSAA (na rozdiel od lišty); dlhé texty sa orezávajú.
- Idle merania n=1; 30-min beh a beh so zapnutým AI zberom v intervale sa nerobili.

**Ďalší míľnik:** OpenTelemetry prijímač (Claude Code a Codex, zdokumentované metriky) ako presnejší zdroj než lokálne súbory;
voliteľne Admin API pre organizačné účty (Credential Manager); prístupnosť a klávesnica v okne nastavení; vlastná ikona
v oblasti oznámení (`Shell_NotifyIcon`); vizuálna kontrola 150/200 % na testovacom stroji.

### 8.5 Míľnik: kruhový indikátor limitu AI, jednoduchšia viditeľnosť, jazyk aplikácie (2026-11-20)

**Zdroj a čerstvosť limitov plánu (presne to, čo transom zobrazuje; percento sa nikdy nepočíta z tokenov ani z cien API)**
| Poskytovateľ | 5-hodinové okno | Týždenné (7-dňové) okno | Kedy je hodnota k dispozícii | Čerstvosť |
|---|---|---|---|---|
| Claude Code | statusline stdin `rate_limits.five_hour.used_percentage` + `resets_at` (zdokumentované, code.claude.com/docs/en/statusline) | `rate_limits.seven_day.*` (to isté) | len predplatné Pro/Max/Ultra, až po prvej odpovedi v relácii a len ak používateľ sám nastavil `statusLine` na `transom-statusline.exe`; pri API kľúči pole chýba → „nedostupné“ | čas, keď most zapísal záznam (pri obnove statusline v Claude Code); lišta zistí nový záznam pri minútovom tiku (len `metadata` súboru) |
| Codex | `~/.codex/sessions/**/*.jsonl`, `token_count.rate_limits.primary/secondary` s `window_minutes` = 300 (**nezdokumentovaná schéma**) | to isté s `window_minutes` = 10080 | len ak Codex také okno zaznamenal. **Na tomto stroji plán `go` hlási jediné okno 43 200 min (30 dní)** → 5 h aj týždeň „nedostupné“, 30-dňové okno sa ukáže v paneli ako „30-dňové okno“. Okná 300/10080 pri Plus/Pro sú predpoklad, na tomto stroji neoverené | čas záznamu v relácii Codexu; zber pri obnove (interval, panel, Obnoviť) |

- Okno sa rozpoznáva **podľa minút** (300, 10080), nie podľa textu. Z viacerých snímkov vyhráva najnovší zaznamenaný.
- Stavy (`transom_usage::limits::freshness`): **aktuálne** (zaznamenané pred < 30 min a okno sa neobnovilo),
  **posledná zaznamenaná hodnota** (staršie – kruh sivý, text s „~“, panel „⚠ … nemusí byť aktuálna“),
  **neplatné** (`now ≥ resets_at` – kruh prázdny, „5 h: nedostupné“; nikdy sa nedopočítava 0 %).
  Prepočet stavu ide z času pri každom prekreslení; minútový tik stačí – žiadne ďalšie prebúdzanie.
- Odhad ceny (statusline `cost.total_cost_usd`, cenník pri lokálnych tokenoch) zostáva v paneli s textom „nie faktúra“
  a „pri predplatnom sa za tokeny neplatí“; s limitmi sa nemieša.

**Implementované**
- `Part::Ring` (transom-core) + vlastný AA rasterizér medzikružia (`icons::ring_coverage`, supersampling 4×4, bez ClearType
  okrajov). Kruh 16 lp, oblúk od 12. hodiny: **využité = v smere hodinových ručičiek akcentom, zostávajúce = proti smeru**;
  vedľa vždy text („62 % využité“ / „38 % zostáva“ / „~62 % …“ / „5 h: nedostupné“ / „AI: vypnuté“), takže význam nie je
  len vo farbe. Za kruhom voliteľné označenie poskytovateľa („Claude“/„Codex“) – pri nedostatku miesta sa skryje skôr než
  položka. Klik → panel AI. Prístupný názov a tooltip: „Využitie AI, Claude Code: 5-hodinový limit 38 % zostáva,
  zaznamenané …“ (pri starej hodnote „… naposledy zaznamenané … – nemusí byť aktuálne“, pri neplatnej „… už neplatí“).
- Nastavenia AI: poskytovateľ v lište Automaticky/Claude Code/Codex (vybraný poskytovateľ sa nikdy nenahradí iným;
  Automaticky = prvý zapnutý s platnou 5 h hodnotou, inak prvý zapnutý), kruh ukazuje využitú/zbývajúcu časť.
  Panel vždy ukazuje všetkých zapnutých poskytovateľov: 5 h, 7 dní a ďalšie hlásené okná so zdrojom, časom záznamu a stavom,
  alebo „nedostupné – zdroj ho nehlási“; zachované Dnes/7 dní/Mesiac s tokenmi a odhadom ceny.
- Viditeľnosť: stĺpec „Skryté“ a ikona oka zrušené; každá položka má v svojej skupine jediné políčko **„Zobraziť v lište“**,
  presun ťahaním viditeľnosť nemení (`Layout::move_within`). Zapnutie zdroja AI už položku samo nezobrazí – stránka AI
  vysvetľuje, že zdroje riadia len čítanie, a ak je položka skrytá, zobrazí poznámku. „Obnoviť predvolené rozloženie“,
  „Vypnúť všetky zdroje AI“ a „Zmazať lokálne dáta AI…“ (druhý klik potvrdí, „Zrušiť“, zruší sa aj prechodom na inú stránku).
- Jazyk: crate `transom-i18n` – zdrojové súbory `locales/<kód>.txt` (kľúč = text, `{0}` parametre) vložené do binárky,
  hľadanie aktuálny jazyk → angličtina → kľúč. Nastavenie `language`: `system` (prvý jazyk UI z
  `GetUserPreferredUILanguages`, len čítanie) alebo sk/en/cs/de/fr/es/pl/it/pt/ja/zh-Hans; nepodporovaný jazyk aj tradičná
  čínština → angličtina. Prepne sa hneď: lišta, tooltipy, MSAA (aj názov predvolenej akcie), menu, panely, kalendár,
  nastavenia vrátane názvu okna a text mostu statusline. Formát času/dátumu a názvy dní/mesiacov v kalendári ostávajú podľa
  regionálnych nastavení Windows. Jazyk Windows sa nemení.
- Pravé tlačidlo: ponuka (Nastavenia / Využitie AI… / Ukončiť transom, preložené, prístupné názvy) sa otvorí **len nad
  prázdnym miestom alebo textom bez akcie** (názov skladby, batéria); nad tlačidlom (aj vypnutým), obalom, kruhom, hodinami
  či prepínačom zdroja sa nestane nič.
- Panel AI 620 × 660 lp; ak by sa obsah nezmestil, päta ukáže „… nezmestilo sa všetko – skúste inú záložku“ (nič sa neodreže
  potichu). Chyba zdroja a „neúplné: najstarší záznam …“ sú na vlastnom riadku.
- Konfigurácia len na čítanie: okno nastavení ukáže preložený dôvod (`transom_config::Issue`: poškodená / novšia verzia /
  nečitateľná); technická poznámka ide len do logu.
- Konfigurácia v2 (`language`, `usage.bar_provider`, `usage.ring_mode`), migrácia z v1, neznáme hodnoty sa čítajú
  zhovievavo (predvolené).

**Overené** (izolované `TRANSOM_CONFIG`/`TRANSOM_DATA`; skutočné súbory transom nevznikli; nemenil sa čas, pásmo, rozlíšenie,
DPI ani jazyk Windows; hlavný monitor ostal na 100 %)
- Živé hodnoty na tomto stroji: **žiadny skutočný 5 h ani týždenný údaj** – statusline most nie je v Claude Code nastavený
  a Codex `go` hlási len 30-dňové okno. Overené so skutočnými `~/.claude` a `~/.codex` (len čítanie, izolované
  `TRANSOM_DATA`): panel pre Codex ukazuje „5-hodinové okno: nedostupné – zdroj ho nehlási“, to isté pre 7 dní,
  „30-dňové okno: 30 % využité · reset 26/12/15“ s „⚠ posledná zaznamenaná hodnota“; všetky tri záložky (Dnes / 7 dní /
  Mesiac) s oboma poskytovateľmi a riadkami tokenov a odhadu ceny sa zmestia. Vyplnený kruh preto len zo **simulovaného** záznamu, ktorý prešiel skutočným mostom
  `transom-statusline.exe` (syntetický vstup: 62 % / 40 %).
- Lišta naživo: „62 % used“ (Windows UI `en-US` → angličtina pri „Podľa Windows“), prepnutie na zostávajúce → „38 % left“
  (oblúk proti smeru), slovenčina → „38 % zostáva“ a „Nastavenia transom“ v názve okna, zjednodušená čínština → „剩余 38 %“.
  Prechody len minútovým tikom: aktuálne 62 % → po novom zázname spred 40 min „~71 % využité“ (sivé) → po resete okna
  „5 h: nedostupné“ (prázdny kruh); panel ukazuje „⚠ okno sa medzitým obnovilo – hodnota už neplatí“.
- Pravé tlačidlo na kruhu aj na **vypnutom** „Predchádzajúca skladba“: menu sa neotvorilo, akcia sa nespustila (log „bez
  menu a bez akcie“); na názve skladby a na prázdnom mieste menu
  (UIA: „Settings“, „AI usage…“, „Quit transom“), Esc vráti fokus; Menu → Nastavenia otvorí okno nastavení v popredí.
- MSAA (`msaa_probe`): kruh aj označenie poskytovateľa majú celý stav v názve, predvolená akcia preložená („Stlačiť“, „按下“).
- Políčko „Zobraziť v lište“ (Batéria vyp/zap → `visible` false/true), voľby poskytovateľa a kruhu, potvrdenie mazania
  (prvý klik nič nezmazal, Zrušiť).
- Náhľad `cargo run -p transom-render --example preview`: kruh vo všetkých stavoch (využité, zostáva, ~, nedostupné, hover,
  stlačené) a japonský/čínsky text cez GDI v tmavej aj svetlej téme pri 96/144/192 DPI (60 obrázkov).
- `cargo run -p transom-settings --example fit_check`: všetky texty nastavení a panelu AI zmerané GDI vo všetkých 11 jazykoch
  proti šírke v rozložení – po skrátení prekladov, zalomení predvolieb hodín (3 na riadok) a riadku prvého dňa (2 na riadok)
  sa nič neoreže.
- Testy (149): used/remaining, posledná/neplatná/chýbajúca hodnota, Codex bez 5 h okna, vybraný poskytovateľ sa nenahradí,
  názvy okien podľa minút, rasterizér kruhu (podiel a smer pri 16/24/32 px), migrácia konfigurácie v1 → v2, zhovievavé
  hodnoty, `move_within`, úplnosť prekladov (každý jazyk má každý kľúč s rovnakými parametrami), výber jazyka a záloha
  (pt-BR → pt, zh-TW → en, hu-HU → en, neznámy kód → en). fmt, clippy `-D warnings`, release build, `cargo deny` ok.
  Binárky: lišta 764 KB, nastavenia 428 KB, most 352 KB (most teraz číta jazyk Windows cez `transom-platform`).

**Opravy nájdené pri živom teste a kontrole**
- Výsledok zberu sa prevzal pred porovnaním obsahu, takže lišta po novom zázname statusline nezistila zmenu a neprekreslila
  kruh (ostal 62 %). Teraz sa výsledok preberá vnútri `update`; aj prevzatie počas kreslenia panelu prekreslí lištu.
- Pravé tlačidlo nad vypnutým tlačidlom (bez akcie) otváralo menu → rozhoduje typ zasiahnutej časti, nie len akcia.
- Panel AI s tromi zdrojmi potichu odrezal riadky Codexu a riadok zdroja vodorovne → vyšší panel, značka v päte, zalomenie.
- Prvý minútový tik vždy spustil zber navyše (čas zmeny mostu nebol zapamätaný) → zapamätá sa pri načítaní cache.

**Limity**
- Preklady okrem slovenčiny a angličtiny (cs, de, fr, es, pl, it, pt, ja, zh-Hans) vytvoril model a **nikto ich nerevidoval**.
- Claude 5 h/7 dní existuje len pri predplatnom a len cez statusline most, ktorý si používateľ musí nastaviť sám; hodnota je
  stav pri poslednej odpovedi, nie priebežný. Most sa so skutočným Claude Code neoveroval.
- Codex: schéma `rate_limits` nie je zdokumentovaná; okná 300/10080 min pri iných plánoch sú predpoklad.
- Okno nastavení: stále bez klávesnicovej navigácie a MSAA pre vlastné ovládacie prvky.
- `OVERFLOW_NAMES` (tlačidlo skrytých ikon) závisí od jazyka Windows, nie od jazyka aplikácie.

**Ďalší míľnik:** klávesnica a prístupnosť okna nastavení; OpenTelemetry prijímač; revízia prekladov rodenými hovoriacimi.

### 8.6 Míľnik: posuvný názov skladby, pozadie lišty (farba, priehľadnosť), obnova AI po 5 minútach (2026-11-20)

**Premisa zadania, ktorá neplatila:** lišta *nebola* vrstvené okno. Bola (a pri 100 % je) obyčajné nepriehľadné `WS_POPUP`
kreslené v `WM_PAINT` cez GDI s ClearType. Priehľadnosť preto nebola „doladenie vrstveného okna“, ale nová cesta kreslenia
(nižšie); pri 100 % ostáva pôvodná cesta bez zmeny.

**Výklad „smer zľava doprava“** (v histórii ani v PLAN.md nie je skoršia požiadavka, „as requested“ sa viaže len na toto zadanie):
predvolený smer je *čítanie zľava doprava* – ukáže sa začiatok názvu a okno sa posúva ku koncu (text sa pohybuje doľava);
druhá voľba je opačná (začína koncom, text sa pohybuje doprava). Obe sú v nastaveniach označené pohybom textu, takže
nesprávny výklad je jedno kliknutie.

#### 1. Posuvný názov skladby (marquee)
- **Čistá časová funkcia** `transom-core/src/marquee.rs` (bez Win32, bez stavu): ping-pong, pauza **2 s** na oboch koncoch,
  pohyb so sinusovým rozbehom/dobehom, trvanie pohybu z rýchlosti (pomaly 20 / normálne 35 / rýchlo 60 lp/s × mierka DPI),
  min. 600 ms. `Policy`: zapnuté v nastaveniach ∧ nie je obmedzený pohyb ∧ prehráva sa (alebo „posúvať aj pri pozastavení“)
  ∧ text pretečie. Nepretečený text sa nehýbe nikdy.
- **Kreslenie** (`transom-render`): `Painter::draw_text_scrolled` (GDI: `IntersectClipRect` + `DrawTextW` bez „…“), scéna hlási
  pretečenie (`RenderInfo`) a text kreslí posunutý a orezaný presne na svoje miesto; tlačidlá, obal a hodiny stoja. Scéna
  preskakuje časti mimo orezávacej oblasti (`GetClipBox`), pretečenie sa však hlási vždy (animácia by inak zanikla pri prekreslení hodín).
- **Riadenie** (`transom-app/src/ticker.rs`): jeden spoločný čas, geometria a posun **pre každú lištu zvlášť** (monitory majú
  rôznu šírku/DPI), prekresľuje sa len úsek textu (`invalidate_span`). Stav animácie nie je v `StatusItem` (inak by každý snímok
  zmenil položky a prekreslila by sa celá lišta). Jednorazový časovač `T_MARQUEE`: pri pohybe 33 ms, počas pauzy na konci
  zvyšok pauzy (žiadne medzisnímky), pri statickom texte žiadny. Reštart pri zmene textu, stavu prehrávania a nastavení.
  Zastavenie: pozastavené prehrávanie (predvolene), text sa zmestí, posuv vypnutý, **obmedzený pohyb vo Windows**
  (`SPI_GETCLIENTAREAANIMATION`, zmena cez `WM_SETTINGCHANGE` 0x1043), lišta skrytá fullscreenom, zrušenie lišty. Po zastavení sa
  raz obnoví statický text s „…“ (aj pri vypnutí posuvu a reduced motion).
- **Nastavenia** (stránka Médiá): posuv zap/vyp, smer, rýchlosť, pri pozastavení (text stojí / posúvať ďalej), živý náhľad
  (~30 snímok/s len na tej stránke), stav animácií vo Windows. Konfigurácia `media.marquee*` (v3).
- **Nájdená a opravená chyba:** `DrawTextW` s prázdnym textom (ukazovateľ na prázdny `Vec`) padal s prístupovou chybou –
  v aplikácii by to nastalo pri media session bez názvu. `GdiPainter` už prázdny text nekreslí (regresný test).
- **Obmedzený pohyb:** stav sa znova číta aj pri minútovom tiku (nezávisle od toho, čo Windows pri zmene rozošle), takže
  nezávisí od neoverenej hodnoty `wParam` v `WM_SETTINGCHANGE`.
- **Kadencia:** časovač beží po 33 ms len počas pohybu, ale prekresľuje sa **iba pri zmene celého pixelu posunu** – pri
  normálnej rýchlosti 35 lp/s a 100 % DPI je to ~17 kreslení/s na lištu (nameraných 2 290 kreslení za ~68 s na 2 lišty), pri
  „Rýchlo“ alebo 200 % DPI (70–120 px/s) až ~30/s s krokom ~2–4 px (horná hranica časovača). Plynulosť pohybu som posudzoval
  len zo snímok a čísel (nie zo živého pohľadu) a nie je to 60 Hz plynulosť.
- **Rozdielne lišty naživo:** s tým istým skutočným názvom („<dlhý názov skladby>“, simulovaný vstup) a stiesnenou lištou 1080 px (dlhý dátum + sekundy + kruh AI + batéria) pretečie text len na nej
  (**28 px pri šírke 366**, log `posuvny text …`) a posúva sa, kým na lište 2560 px sa zmestí a stojí.
- **Testovací vstup** `--sim-media "Názov[|paused]"` (len test vzhľadu, v logu označené ako SIMULOVANÉ) – aby sa nemenilo
  skutočné prehrávanie.

**Namerané** (release, 2 lišty 1080 a 2560 px pri 100 % DPI, `tools/cpu-sample.ps1`/`measure.ps1`, 60 s po 6 s rozbehu, n = 1;
rozlíšenie `TotalProcessorTime` je 15,6 ms; „kreslenia“ = počet dokončených kreslení celkom od štartu vrátane rozbehu)
| Stav | CPU (% jedného jadra) | kreslenia celkom |
|---|---|---|
| bez médií | 0,000 | 4 |
| prehráva sa **krátky** názov (zmestí sa) | 0,000 | 6 (žiadne priebežné kreslenie) |
| prehráva sa **dlhý** názov, posuv beží | **1,04** (~17 kreslení/s na lištu) | 2 290 za ~68 s |
| dlhý názov **pozastavený** (predvolene) | 0,000 | 4 |
| dlhý názov, posuv **vypnutý** (statický „…“) | 0,000 | 4 |
| dlhý názov, posuv beží, pozadie **50 %** (vrstvené) | 1,33 | 2 294 |
| bez médií, pozadie 50 % | 0,000 | 4 |
Prvé meranie (bez preskakovania častí mimo orezu) bolo 1,72 %; po `GetClipBox` 0,83–1,04 %. Vzorky sú krátke a n = 1 – ide o
rád veľkosti, nie o štatistiku. Pamäť: WS 11,6 → 11,7 MB (vrstvený režim 13,3 MB).

#### 2. Pozadie lišty (farba, priehľadnosť)
- **Konfigurácia v3** (`appearance`: `bg_color` `#RRGGBB`, `bg_opacity` 10–100, `bg_transparent`; `media.marquee*`;
  `usage.refresh_minutes`): migrácia v1/v2 → v3 doplní predvolené hodnoty, zachová ostatné; číselné polia sa čítajú
  zhovievavo (hodnota 300 alebo text nevyhodí celú konfiguráciu do „len na čítanie“, len sa oreže/nahradí predvolenou).
- **Farba prepisuje len pozadie.** `App::bar_theme()` odvodzuje tému lišty zakaždým znova z `self.theme` (zmena témy systému
  ju neprepíše); popovery používajú `self.theme` a ostávajú nepriehľadné. Svetlý/tmavý/systém sa nemení.
- **Nastavenia** (stránka Pozadie): predvoľby (tému + 8 farieb), systémový dialóg výberu farby (`ChooseColorW`), polia HEX a
  R/G/B (natívne `EDIT`; neplatná hodnota = červený rám a text, platná sa prejaví v náhľade hneď a uloží sa po 450 ms ticha
  alebo pri strate fokusu), „Priehľadné pozadie“, posuvník nepriehľadnosti (tlačidlá −/+; nahľad pri ťahaní, **uloženie a
  rozoslanie až pri uvoľnení myši**), kontrast textu (WCAG, varovanie pod 4,5 : 1 pri nepriehľadnom pozadí), náhľad nad tmavou
  a svetlou plochou skutočným rendererom.
- **AppBar rezervácia nezávisí od vzhľadu:** `ABM_SETPOS` sa nemení. Overené s `tools/gate-appbar.ps1` pri 50 % (vrstvené):
  pracovná plocha 28/-377, maximalizované okno pod lištou, skrytie pri fullscreene aj návrat, uvoľnenie za 0,11 s; aj pri
  „úplne priehľadné“ ostáva rezervácia (28 px).
- **Ako to funguje s GDI (bez Acrylic/Mica, bez zmeny backendu):** pri 100 % pôvodná cesta (ClearType, `WM_PAINT`). Pod 100 %:
  `WS_EX_LAYERED` + `UpdateLayeredWindow` s alfou po pixeloch. GDI alfu nekreslí, preto sa **scéna vykreslí dvakrát** na
  čierne a biele pozadie a alfa sa odvodí `a = 1 − (W − B)/255`, `B` je už premultiplied farba (`platform/layered.rs`);
  obsah sa potom zloží nad pozadie s požadovanou alfou. Nepriehľadné prvky (obal, hover) majú `B = W`, teda a = 1.
  - **Okrajové pixely sú naozaj polopriehľadné** (žiadne „halo“ farby pozadia). Samokontrola `alpha_check`: skutočná scéna cez
    GDI zložená nad 4 podkladmi (jasný, stredný, tmavý, modrý) vs. priame vykreslenie na ten istý podklad, 96/144/192 DPI,
    obe témy, nepriehľadnosť 255/200/128/64/0: **priemerná odchýlka 0,06–0,76 z 255, najväčšia 14 na pár okrajových pixelov**.
  - **Živé pozorovanie nad jasným podkladom** (nie len matematika): `tools/backdrop.ps1` položí pod lištu neaktivujúce okno
    28 px (biela / červená / sivá pásma) a `tools/shot-bar.ps1` (BitBlt s `CAPTUREBLT`; overené na známom 50 % okne WinForms →
    (255,128,128)) nasníma lištu. Prázdne pixely lišty vs. `blend(farba, podklad, alfa)`: pri 100 % (30,34,40) na všetkých
    pásmach; pri 50 % (142,144,147) / (142,17,20) / (79,81,84) – **zhoda 0**; pri „priehľadné“ (254,254,254) / (254,0,0) /
    (127,127,127) – **zhoda 0–1**; pri 50 % s farbou #204060 (143,159,175) / (143,32,48) / (80,96,112) – **zhoda 0**. Okraje
    pri 50 % sú čisté (žiadne tmavé lemovanie; ClearType pri 100 % má farebné okraje). *Prvý pokus o toto meranie bol
    neplatný (skript okna sa neskompiloval, podklad tam nebol) – zachytilo sa „nad čiernou“; opravené a zopakované.*
  - **ClearType sa pri priehľadnom pozadí nedá použiť** (farebné okraje podľa subpixelov potrebujú známe nepriehľadné
    pozadie) → sivé vyhladzovanie; text je pri 100 % o niečo ostrejší.
  - **Čitateľnosť závisí od tapety** – nie je to chyba renderovania: pri „úplne priehľadné“ sa biely text tmavej témy stráca
    na jasnej tapete a tmavý text svetlej témy na tmavej (vidno na obrázkoch z `alpha_check`). Najlepšia podporovaná kvalita: **100 %
    (ClearType)**; pri priehľadnosti **≥ ~60 %** alebo vlastná farba pozadia zhodná s tapetou. Nastavenia to hovoria
    a vypisujú kontrast. Neimplementované (ponuka): jemný tieň/obrys textu pre nízku nepriehľadnosť, automatický výber
    svetlého/tmavého textu podľa farby pozadia, rozostrenie za oknom (Acrylic – zámerne mimo rozsahu).
  - **Alfa najmenej 1:** pixely s alfou 0 prepúšťajú myš na plochu; pri „úplne priehľadné“ preto pravý klik na prázdne miesto
    (menu) a hover dalej fungujú – overené živo.
  - Prepínanie za behu (config + broadcast): 100 → 50 % pridá `WS_EX_LAYERED`, späť ho odoberie; overené stylom okna.
  - Vrstvené okno nedostáva `WM_PAINT`: prekreslenia sa zlučujú do vlastnej správy `WM_APP+0x34` (nikdy sa nekreslí priamo
    z kontextu, kde môže byť vypožičaný handler; kolízia ID s `WM_TRANSOM_INVOKE` odhalená kompilátorom). Ak je handler
    vypožičaný, neplatná oblasť sa **zachová** a prekreslenie sa zopakuje o 15 ms cez časovač (nie okamžitým opakovaným
    `PostMessage`).

#### 3. Obnova údajov AI
- Predvolený interval **5 min**, minimum **5 min**, možnosti Na požiadanie / 5 / 15 / 30 / 60 min / 4 h (nikdy nie 1–4: pri
  načítaní sa zvýšia na 5). **Migrácia:** v1/v2 mali predvolených 60 min, čo sa nedalo odlíšiť od vedomej voľby → hodnota 60
  sa pri migrácii zmení na 5 (30 a 240 ostávajú; v3 s 60 ostáva).
- **Jedna brána** `AiState::refresh(kind)` / `auto_allowed(now, last_started, interval)`: automatické spúšťače (časovač,
  kontrola mostu statusline pri minútovom tiku, otvorenie panelu, štart) smú najviac raz za **nastavený interval** (15/30/60 min,
  4 h; nikdy menej než 5 min); pri „Na požiadanie“ sa automaticky nečíta nič okrem **prvého čítania, keď ešte nie sú dáta ani
  cache** (inak by zapnutý zdroj ostal prázdny). Otvorenie panelu je automatický spúšťač, teda sa riadi intervalom (pri 60 min
  panel neobnoví dáta staršie než 5 min – ukáže čas posledného úspechu, stav a tlačidlo Obnoviť). Len *Obnoviť* a zmena
  zdrojov (vedomý čin používateľa) idú mimo brány.
  Kontrola mostu porovnáva čas súboru s časom pri **spustení posledného zberu**, takže zmena zadržaná bránou sa nestratí.
  Zmena kruhu (poskytovateľ/režim) obnovu už nespúšťa. Po reštarte sa brána počíta od času cache.
  - **Overené:** (a) predvolených 5 min: 560 s, nový záznam mostu každú minútu → **2 zbery** (0,13 s a 300,14 s);
    (b) „Na požiadanie“ (`refresh_minutes: 0`): 560 s a 9 nových záznamov mostu → **1 zber** (0,15 s, pri štarte), potom nič; (c) čistý test pre 5/15/60/240 min a „na požiadanie“ za 6 hodín
    minútových tikov: 72 / 24 / 6 / 2 / 1 zberov.
- **Posledné úspešné obnovenie:** `SourceStatus.last_ok_at` (pri chybe sa prenáša z predchádzajúceho zberu, takže neúspech
  nikdy neposunie čas); zastaranosť = 2 × interval (min. 10 min; pri „na požiadanie“ 1 h) od posledného *úspešného* čítania.
  Panel: riadok „Posledné úspešné obnovenie: … · ZASTARALÉ“ a „Automatické obnovenie každých 5 min“; pri každom zdroji čas
  posledného úspechu alebo „nikdy“. Nedostupné/vypnuté/zastarané údaje nikdy nevyzerajú ako nula (kruh: „5 h: nedostupné“,
  „~hodnota“ pri starej, prázdny kruh po resete okna); zdroje, popisky a „nie faktúra“ nezmenené; súkromie nezmenené.

#### 4. Prehľad referencie Vorssaint (issue #1691) – len ako UX inšpirácia
Čítané: issue (verejné, bez komentárov, bez snímok, bez odkazov na kód či PR) a koreň repozitára. Zdroje kódu som **neotváral**.
- *Čo issue navrhuje:* jedno miesto s využitím a limitmi viacerých AI agentov (Codex, Claude Code) „na prvý pohľad“, v
  kompaktnom prehľade (menu bar / systémový monitor): percento využitia, zostávajúca kvóta, krátkodobé a dlhodobé limity,
  čas do resetu, tokeny/kontext ak sú k dispozícii, rozšíriteľnosť na ďalších agentov bez natvrdo zapísaných poskytovateľov.
- *Dátová metóda:* issue ju **neuvádza** a netvrdí, že je zdokumentovaná. README repozitára len spomína funkciu „AI agents“
  s limitmi plánu a časmi resetu „tam, kde sú k dispozícii“; ako sa limity získavajú, som nezistil a **nepredpokladám, že je
  to bezpečné ani zdokumentované**. Repozitár je pod **GPL-3.0 alebo novšou** – kód sa do transom (MIT) nesmie kopírovať.
- *Užitočné pre transom (prezentácia, nie dáta):* (1) kompaktný ukazovateľ, ktorý hovorí, *ktorý* agent ešte má kapacitu –
  transom ho má (kruh + voľba poskytovateľa, Automaticky vyberá prvého s platnou hodnotou); možné rozšírenie: dva malé kruhy
  naraz. (2) Krátkodobý a dlhodobý limit vedľa seba s časom do resetu – panel má 5 h a 7 dní s absolútnym časom resetu;
  **relatívny čas „o 2 h 14 min“ je lacné a užitočné vylepšenie** (nepotrebuje nové dáta; neimplementované). (3) Poskytovatelia
  nie sú natvrdo v UI – transom rozpoznáva okná podľa minút a zobrazí aj neznáme (30-dňové okno Codexu). (4) Tokeny/kontext
  „ak sú k dispozícii“ – transom to už rozlišuje (dostupné / odhad / nedostupné).
- *Čo si transom nechá vlastné:* označenie zdroja a čerstvosti pri každej hodnote, „odhad podľa cenníka – nie faktúra“, žiadne
  percento limitu z cien tokenov, žiadne neoverené metódy získavania dát.

#### Overenie a obmedzenia
- fmt, clippy `-D warnings`, release build, **191 testov**, `cargo deny` ok. Nové testy: marquee (aktivácia, smer, pauza/
  pokračovanie, obmedzený pohyb, rozsah, rýchlosť vs. dĺžka, frekvencia snímok), ticker (po lištách, zastavenie, skryté lišty,
  reštart, DPI), scéna (orezanie na miesto textu, preskakovanie mimo orezu, hlásenie pretečenia), skladanie alfy (priehľadné
  pozadie, alfa 1, okraje bez halo, zhoda nad ľubovoľnou farbou), HEX/RGB (prísne), limity nepriehľadnosti, kontrast, migrácia
  v1/v2 → v3, zhovievavé hodnoty, hranice intervalu obnovy, brána obnovy, posledný úspech pri chybe, rozloženie stránky
  Pozadie pri 100/150/200 %.
- Príklady: `marquee_check` (**mimo miesta textu 0 odlišných pixelov** pri 96/144/192 DPI × 2 témy × 3 posuny, GDI), `alpha_check`,
  `fit_check` (texty nových stránok vo všetkých 11 jazykoch sa zmestia po skrátení nemčiny a zalamovaní volieb).
- Vizuálne: svetlá aj tmavá téma na lište naživo (100 %), 96/144/192 DPI cez príklady (bez zmeny systémového DPI).
- Spotify a Brave/YouTube: na stroji boli pozastavené relácie so **skutočnými** názvami (Spotify „<dlhý názov skladby>“, časť v lište 406 px, YouTube „<dlhý názov videa>“) a obe sa zmestili do 420 lp
  → správne sa **nehýbali** (aj so zapnutým posuvom pri pozastavení). Dlhé názvy som preto overil **simulovanou** session
  (`--sim-media`); prehrávanie ani karty sa nemenili, nebolo čo obnovovať.
- Neoverené: skutočný dlhý názov zo Spotify/YouTube (Spotify medzitým sám prešiel na kratšiu skladbu); reštart Explorera
  s vrstveným oknom (cesta `ABM_NEW` sa nemenila);
  zmena „Efekty animácie“ vo Windows naživo (zámerne som systémové nastavenie nemenil – pokrývajú ju testy politiky a ticker);
  150/200 % len cez príklady (nie živá lišta); vrstvená cesta 150/200 % tiež len cez `alpha_check`; svetlá téma okna nastavení
  (okno nastavení sleduje systémovú tému, tá ostala tmavá); ClearType vs. sivé vyhladzovanie nie je kvantifikované.
- Známe: v okne nastavení nie je klávesnicová navigácia (Tab medzi vlastnými ovládačmi) ani MSAA mimo natívnych polí;
  `Ctrl+A` vo vstupnom poli nefunguje (štandardný jednoriadkový `EDIT`); preklady okrem sk/en sú stále nerevidované.

### 8.7 Míľnik: odpočty resetu limitov AI, vizuálna identita a logo, prepracované UI lišty a nastavení (2026-11-20)

Dizajnový systém: `docs/DESIGN.md` (farby, typografia, rozmery, štýl ikon, použitie loga). Zdroj pravdy je kód, `assets/brand/*`
sa generuje a test porovnáva drift.

#### 1. Odpočty do resetu (5 h / 7 dní / ľubovoľné okno zdroja)
- `transom_usage::limits::reset_state(resets_at, freshness, now)`: **len** z `resets_at` zdroja a `now`. Stavy: `In{secs,last_recorded}`,
  `Passed` („čas resetu uplynul; čaká sa na aktualizované údaje“ – **nepredpokladá sa, že reset prebehol**), `Unavailable`
  („čas resetu nie je k dispozícii“ – nikdy sa nedopočíta z názvu ani dĺžky okna). Hodnota `<= 0` = chýba.
- Formát `2 h 14 min` / `45 min` / `< 1 min` / `3 d 4 h` (zaokrúhlenie nahor, od 1 dňa na hodiny; skratky jednotiek v každom jazyku,
  bez plurálov). Aktualizuje sa z uloženého času pri existujúcom minútovom ticku (zmena tooltipu → jedno prekreslenie za minútu),
  **bez** nového dotazu a bez animácie. Zastaraný záznam označí odpočet ako založený na poslednom zaznamenanom čase resetu.
- Testy: platný / zastaraný / chýbajúci / uplynulý čas, hranice (59 s, 60 s, 1 h, 24 h, dni; zmena času a letný čas – počíta sa v
  absolútnych sekundách), reálne parsery (statusline Claude, `rate_limits` Codexu) → odpočet.

#### 2. Identita a logo
- **Motív „bar nad w“:** vlastná geometria (zaoblené obdĺžniky a ťahy), žiadne logo tretích strán. Jedna geometria → SVG
  (`assets/brand/*.svg`), ICO (BMP 32 bpp + AND maska; `transom.ico` 16–256, `transom-tray-dark/light.ico` 16–48) aj runtimové rastre.
- Vložená do `.exe` (`build_support/icon_res.rs` → `.res` bez ďalšej závislosti), okno nastavení ju nastavuje cez `WM_SETICON`
  (malá aj veľká veľkosť podľa DPI), navigácia nastavení ukazuje značku + wordmark.
- **Ikona v oblasti oznámení** (`Shell_NotifyIconW`, `NOTIFYICON_VERSION_4`): variant podľa témy **panela úloh**
  (`SystemUsesLightTheme`), nová pri zmene témy a po `TaskbarCreated`; ľavý klik = nastavenia, pravý = menu (Nastavenia / Využitie AI /
  Ukončiť); odstráni sa pri ukončení pred zánikom okna; vypínateľná v nastaveniach (Pozadie → „ikona transom v oblasti oznámení“,
  konfigurácia **v4**, `appearance.tray_icon`, migrácia z v3 s testom), `--no-tray` na testy. Ak ju systém odmietne, nezapamätá sa a
  skúsi sa znova.

#### 3. UI lišty a nastavení
- Lišta: vlas na spodnej hrane, jemné delič medzi položkami, zaoblené hover/pressed čipy, polotučné hodiny; farby okrajov a čipov
  sa **počítajú z farby textu nad skutočným pozadím**, takže fungujú s vlastnou farbou aj s priehľadnosťou (dvojprechodová alfa).
- Nastavenia: navigácia 200 lp s logom a ikonami, zaoblené tlačidlá/políčka/predvoľby/posuvník, posuvný obsah (koleso, PgUp/PgDn,
  Home/End, indikátor polohy, okno sa zmenší do pracovnej plochy), pevný stavový riadok, **zalamovanie všetkých dlhších textov** a
  zloženie popisku nad tlačidlá, ak sa vedľa seba nezmestia – nič sa potichu neorezáva.
- `Painter` rozšírený o `blend_rgba`, `push_clip/pop_clip`, `fill_round_rect`, `stroke_round_rect` (predvolene bez alfa backendu =
  nič/obdĺžnik); GDI cez `AlphaBlend` a `SaveDC/IntersectClipRect`.
- **Audit** (`transom_render::audit`, `offscreen`): `Audit` hlási orezanie textu, presah výšky, presah povrchu, prekryv textov a
  kontrast voči ploche pod textom; `transom-settings --shot <adresár> [--png]` ho spustí nad celou maticou, test v `shot.rs`
  (de, ja × 96/192 DPI). Opravené nálezy: nadpis navigácie (zalomenie), pomocné texty (zalomenie), verzie popisov zdrojov AI (cesta
  k súboru sa zalamuje), riadky volieb s dlhými prekladmi (popisok nad tlačidlami). Starý príklad `fit_check` je nahradený (dával
  falošné nálezy pre texty, ktoré sa už zalamujú).
- Drobná oprava: prázdny súčet nákladov statusline sa zobrazoval ako „$-0.00“ (`-0.0` zo `sum()`), teraz `$0.00` (test).
- Konfigurácia toleruje UTF-8 BOM (Poznámkový blok / PowerShell 5.1) – predtým sa taký súbor považoval za poškodený (test).

#### Overenie a obmedzenia
- fmt, clippy `-D warnings`, release build, `cargo deny` (advisories, bans, licenses, sources ok), **244 testov** + 2 pomalé (`--ignored`).
- Audit nastavení: **0 nálezov** pre 11 jazykov × 6 stránok (+ potvrdenie mazania) × 96/144/192 DPI × tmavá/svetlá téma.
- Naživo (100 %, hlavný monitor, bez zmeny DPI/jazyka/času): ikona v skrytých ikonách (Windows 11 ju štandardne uloží do
  „Zobraziť skryté ikony“), ľavý klik otvoril nastavenia, pravý klik menu; po ukončení cez menu proces zanikol (zmiznutie samotnej ikony som po ukončení zvlášť nenasnímal).
  Okno nastavení s ikonou v titulku. Prepnutie témy panela úloh na svetlú (dočasne, **hodnota vrátená na 0**): aplikácia zalogovala
  výmenu variantu, snímka flyoutu je z prechodu témy (nie čistá). Priehľadnosť 50 % nad pásmami (sivá/červená):
  pásma presvitajú cez lištu a ikonu hľadania, bez tmavých lemov (vizuálne; presnú číselnú zhodu som v tomto behu nemeral,
  posledné číselné meranie je v §8.6). Nečinnosť: **0,000 % CPU za 30 s**, 11,9 MB, 4 prekreslenia za
  ~45 s (minútový tik), žiadne súvislé prekresľovanie. `alpha_check` najväčšia odchýlka 12 (predtým 14), `marquee_check` OK.
- Neoverené / obmedzenia: 150 % a 200 % len offscreen (GDI), nie na skutočnom monitore; svetlá téma okna nastavení len offscreen;
  ikona tlačidla panela úloh a Alt+Tab pre okno nastavení nie sú nasnímané zvlášť (ikona je v `.exe` aj cez `WM_SETICON`; zdroj
  `.exe` overený cez `ExtractAssociatedIcon`); preklady okrem sk/en sú nerevidované; v okne nastavení stále chýba klávesnicová
  navigácia medzi vlastnými ovládačmi.
- Poznámka k testovaniu: raz som spustil lištu so zapnutými zdrojmi AI nad skutočným `%USERPROFILE%` (len lokálne čítanie, výsledok
  = súhrny v `%LOCALAPPDATA%\transom\usage-cache.json`, ktorý som **zmazal**); ďalšie behy idú len cez `CLAUDE_CONFIG_DIR`,
  `CODEX_HOME`, `TRANSOM_DATA`, `TRANSOM_CONFIG` do scratch adresára. Po násilnom ukončení lišty s ikonou ostáva v oblasti
  oznámení „duch“ do prvého prejdenia myšou (preto `--no-tray` pri testoch).

### 8.8 Míľnik: prepracované využitie AI – ukazovateľ v lište a prehľad všetkých poskytovateľov (2026-11-20)

Referencia (Vorssaint issue #1691) je len zoznam cieľov (prehľad viacerých agentov, krátke/dlhé limity, zostávajúca kvóta, reset,
tokeny/kontext tam, kde sú); dizajn je vlastný, žiadny cudzí kód ani vzhľad (pozri aj §8.6/4).

#### Model
- **Okná podľa dát poskytovateľa, nie natvrdo 5 h.** `limits::provider_windows(plans, provider, now, max_age)` = pre každú dĺžku okna
  (minúty) najnovší záznam, v ktorom sa okno vyskytlo, zoradené od najkratšieho; okno, ktoré v najnovšom zázname chýba (napr. 5 h po
  resete), ostane zo starého záznamu ako zastarané/expirované. Odtiaľ čerpá kruh, tooltip, karty aj odznak.
  `Provider::ALL` = zoznam poskytovateľov (nový agent = variant + zdroj, žiadny plugin runtime).
- Statusline most teraz drží **snímok pre každé okno** (predtým len posledný, takže 5 h okno po resete zmizlo) a zaznamenáva dve
  zdokumentované čísla kontextu (`context_window.used_percentage`, `context_window_size`; len čísla, nič z cesty/prepisu).
- **Okno kruhu** = najkratšie hlásené okno, aj expirované (vtedy „reset passed“; kruh sa potichu neprepne na iné okno).
  **Automatický poskytovateľ:** prvý zapnutý s *aktuálnou* hodnotou kruhu → inak s poslednou zaznamenanou → inak s expirovanou → inak
  prvý zapnutý. Výslovná voľba (Claude/Codex) sa nikdy potichu nemení.
- **„MOST LEFT“** len pre aktuálne hodnoty rovnakej dĺžky okna aspoň dvoch poskytovateľov, jednoznačné maximum; zastaraná/expirovaná
  hodnota nikdy nevyhrá ani nesúťaží. Okná rôznych dĺžok sa nikdy nezlučujú.

#### Lišta
`kruh + „62 % used“` (alebo „left“) a voliteľný text `Claude · 5 h · resets in 2 h 14 min` (poskytovateľ · okno z dát · odpočet len z
`resets_at`). Voliteľný text sa pri nedostatku miesta skryje ako celok **skôr než** názov skladby (poradie `drop_rank`), nikdy sa
neskracuje (test: najdlhší text vo všetkých jazykoch ≤ 380 lp pri 96 aj 192 DPI). Stavy majú slová/znaky: `~62 %` + „stale“,
`unavailable`, `reset passed`, `! 95 %` + farba `low` pri ≤ 10 % zostávajúcej kapacity (nikdy len farba). Tooltip vymenuje všetky
okná poskytovateľa s %, odpočtom a stavom. Živý test s fixture dátami: `62 % used · Claude · 5 h · resets in 2 h 14 min` a po slovensky
`~31 % využité · Codex · 30 d · zastarané · reset o 25 d 0 h` (Codex s 30-dňovým oknom a zastaraným záznamom, Claude bez limitov →
Auto vyberie Codex a nezobrazí vymyslené „5 h nedostupné“, ako to robila predošlá verzia).

#### Popover (620 × 660 lp)
1. **Aktuálne limity** – karta na poskytovateľa (dve vedľa seba, pri jednej plná šírka, ďalšie sa zalamujú): názov + plán, zdroj a čas
   záznamu (raz na kartu, ak sú rovnaké; inak pri každom okne), blok na každé okno: názov okna, odznaky (STALE / RESET PASSED /
   NO RESET TIME / MOST LEFT), pruh (expirované okno má len prázdnu dráhu a text „reset prešiel“, žiadne percento), „X % used · Y % left“,
   odpočet, jedno vysvetlenie zastaranosti na kartu, kontext poslednej relácie (len Claude statusline). Sekcia sa **neorezáva**.
2. **História** (vizuálne druhotná, nadpis + záložky Dnes/7 dní/Mesiac + poznámka „tokeny a odhady ceny podľa API – nie kvóta plánu“):
   zdroje, tokeny po modeloch, „≈ odhad podľa cenníka – nie faktúra“. Ustupuje ako prvá; ak nie je miesto, pata ukáže „nezmestilo sa“.
Žiadne animácie (nič nové sa nekreslí mimo udalostí); odpočty z uložených časov pri minútovom ticku, bez dotazu na poskytovateľa;
interval obnovy 5 min (predvolený aj minimálny), opt-in zdroje, lokálne spracovanie a „odhad, nie faktúra“ nezmenené.

#### Overenie
- Nové testy: okná podľa poskytovateľa (len 30 d / len 7 d / 5 h + 7 d, okno chýbajúce v novom zázname), pravidlo okna kruhu vrátane
  expirovaného, výber poskytovateľa (živý prípad Claude bez dát + Codex 30 d zastaraný), `most_left` (jednoznačný/zhoda/zastarané/rôzne
  dĺžky), odznaky (zastarané, reset prešiel, bez resetu), okno bez resetu, statusline po jednom snímku na okno, kontext, `~`/`!` v štítkoch,
  popover offscreen (fixtures: aktuálne, **najhorší prípad** = 2 poskytovatelia × 3 okná × zastarané + chyby všetkých zdrojov,
  reset prešiel + bez času resetu, Codex 30 d) – `Audit` 0 nálezov pre de/fr/ja/en × 96/192 DPI v teste a **0 nálezov v plnej matici
  (11 jazykov × 96/144/192 × obe témy × 4 fixtures × 2 záložky, `-- --ignored popover_audit_full_matrix`)**; rozpočet výšky: sekcia limitov
  v najhoršom prípade vo všetkých jazykoch zmestí do panela s miestom aspoň na nadpis histórie.
- Obmedzenia: popover má pevnú výšku a nescrolluje – v najhoršom prípade sa história skryje (s označením); **kontext** zdroje hlásia len
  pri Claude statusline (zdokumentované); Codex kontext sa nezbiera (schéma nezdokumentovaná); **`rate_limits.spend_limit`** statusline
  (okno s periódou denne/týždenne/mesačne, `used_usd` je odhad brány) sa nezbiera – nemá pevnú dĺžku v minútach a vyžaduje model okien
  podľa periódy; popover som po prepracovaní **naživo neklikal** (otvorenie vyžaduje syntetický vstup, ktorý som vedome nespúšťal):
  overené offscreen cez GDI a živou lištou s fixture dátami; pred prepracovaním som panel otvoril skutočným menu ikony a ukázal
  skutočné Codex okno `30-day window · 31 % used · resets in 25 d 4 h` so stavom „zastarané“.
- Preklady nových reťazcov (okrem sk/en) sú nerevidované.

### 8.9 Míľnik: plynulý ticker, presne vycentrovaná stredná skupina, jednoduchší popover AI, udalosti v kalendári (2026-11-20)

#### 1. Ticker a stredná skupina
- **Meranie najprv** (`--ticker-stats` + `tools/ticker-measure.ps1`: izolovaná lišta so simulovaným dlhým názvom, 2 lišty 1080 a 2560 px,
  100 % DPI, 30 s po rozbehu, n = 1): pôvodný stav (`SetTimer` 33 ms, krivka smoothstep) nebol plynulý – `WM_TIMER` chodí v násobkoch
  15,6 ms, takže požadovaných 33 ms prišlo ako 47 ms.

| | snímok / ~35 s | interval ms: priemer / odchýlka / p95 | posun na snímku (1 / 2 / 3+ px) | CPU |
|---|---|---|---|---|
| **pôvodný** pomaly (20 lp/s) | 599 | 51,9 / **18,2** / 93,3 | 462 / 137 / 0 | 0,99 % |
| pôvodný normálne (35) | 567 | 51,4 / **17,8** / 93,1 | 214 / 278 / 75 | 1,09 % |
| pôvodný rýchlo (60) | 582 | 48,8 / **12,0** / 61,4 | 87 / 101 / 394 | 0,89 % |
| **nový** pomaly | 655 | 50,1 / **2,4** / 50,6 | **655** / 0 / 0 | 0,47 % |
| nový normálne | 1 062 | 28,9 / **3,2** / 29,6 | **1 062** / 0 / 0 | 1,04 % |
| nový rýchlo | 1 750 | 17,0 / **2,7** / 17,6 | **1 750** / 0 / 0 | 2,71 % |

  Nový pohyb má **každý snímok presne 1 px** a odchýlku intervalu 2–3 ms (predtým 12–18 ms, snímky po 2–4 px). CPU pri normálnej
  rýchlosti je rovnaké (1,04 vs 1,09 %) pri päťkrát nižšej odchýlke; pri „Rýchlo“ je vyššie (0,89 → 2,7 %), lebo sa kreslí naozaj ~60×/s
  na každú lištu (~0,23 ms na prekreslenie). Rýchlosti som **nezmenil** (20 / 35 / 60 lp/s): pri ≤ 60 px/s vychádza jeden pixel na ≥ 16,7 ms,
  teda krok je vždy 1 px; pri 200 % DPI („Rýchlo“ = 120 px/s) sú snímky 2 px po 16 ms (nameraný je len 100 % DPI). Čitateľnosť rýchlostí
  som nemeral (nemám na to objektívnu metriku).
- **Čo sa zmenilo:** (1) rýchlosť je **stála** (lichobežníkový profil: rozbeh a dobeh po 400 ms, medzi tým stála rýchlosť; smoothstep robil
  stred 1,5× rýchlejší a koniec pomalý), (2) ďalší snímok sa plánuje na okamih **ďalšej zmeny celého pixelu** (najskôr za 16 ms, najneskôr
  60 ms), nie naslepo, (3) snímky riadi **vysokorozlišovací waitable časovač** (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`, vlákno
  `transom-frame` posiela `Notify`); `timeBeginPeriod(1)` pre `SetTimer` nepomohol (namerané), preto sa nepoužíva; ak časovač nevznikne, ostáva
  `SetTimer`. Vlákno spí, kým nie je časovač nastavený.
- **Neaktívny stav (CPU / prekreslenia, 30 s):** krátky názov (zmestí sa) 0,000 % a 2 prekreslenia (len štart); dlhý názov pozastavený
  0,000 %, 4; posuv vypnutý 0,000 %, 2; po zastavení sa zruší aj čakajúce prebudenie. Obmedzený pohyb vo Windows a nastavenia tickera ostávajú v platnosti
  (testy politiky).
- **Minimálna čitateľná šírka okna:** 120 lp (`MIN_READABLE_LP`; škáluje sa s DPI). V užšom okne sa text **nehýbe**, ostane statický úsek
  s „…“; pod 80 lp (nový `TITLE_FLEX.min_lp`) sa názov skryje celý (cez rozloženie). Text sa orezáva len na svoje miesto
  (`marquee_check` OK: 0 odlišných pixelov mimo textu pri 96/144/192 DPI).
- **Stredná skupina presne na polovici lišty** (`transom-core/layout.rs`): jej stred = šírka / 2 (±1 px) bez ohľadu na šírku ľavej a pravej
  strany; nikdy sa nesúva do „zvyšného miesta“. Pri kolízii sa skracuje/skrýva obsah (najprv pružný názov, potom podľa `drop_rank`), nie poloha;
  rast pružných textov sa počíta tak, aby stred ostal. Naživo (media v strede, dlhý názov): lišta 1080 px → skupina 274–806, stred **540**; lišta
  2560 px → 1014–1546, stred **1280**. Testy: rôzne šírky strán/dátumu, zužovanie 2600 → 200 px po 13, DPI 1/1,5/2, len stredné položky.

#### 2. Popover AI podľa referenčných snímok
Snímky (4 ks, produkt Vorssaint) sú **len vizuálna referencia hierarchie**; kód, farby ani texty som neprevzal.
- **Nasledované:** karta na poskytovateľa s názvom a štítkom plánu vpravo (referencia „Max 5×“ / „Plus“ → u nás `Pro/Max/Ultra`, `go`); riadky okien
  „názov · odpočet … percento“ s hrubým zaoblenými pruhom (referencia „Session ⟳ 2h13m 63 %“ → `5-hour window  resets in 2 h 14 min  62 % used`);
  jedna karta „Spending“ s veľkou sumou, „API value“, tokenmi a „% from cache“ a zoznamom modelov s pruhmi → karta **História** (`≈ $…`, „API-equivalent
  estimate – not a bill“, `tokeny · % z cache`, modely s pruhmi podľa odhadu); výber obdobia (u nás tri záložky); jednoduchá, tmavá a zaoblená karta.
- **Nenasledované (zámerne):** mriežka aktivity, „Now“ a trend (nemáme tie dáta), zoznam projektov (nezbierame; súkromie), tlačidlo „Use a reset“
  (cudzia funkcia), farba podľa poskytovateľa (stav sa nikdy neodlišuje len farbou; jeden akcent), stavová bodka (máme slová), **značky tempa na pruhoch**
  (vyžadovali by odvodzovať uplynutú časť z dĺžky okna; zadanie to zakazuje).
- **Normálny pohľad:** hlavička + prepínač **Podrobnosti**; karty limitov (poskytovateľ, plán, okná s odpočtom a %, pruh, odznaky STALE (+ čas záznamu, ak sa
  líši) / RESET PASSED / MOST LEFT; „NO RESET TIME“ sa nekreslí, hovorí to riadok odpočtu); História (druhotná). **Žiadne cesty AppData, popisy
  formátov ani zdroje** v bežnom pohľade. Zastarané okno má „~“, odznak so slovom a pod názvom karty „zaznamenané …“ (raz na kartu).
- **Podrobnosti** (diagnostika, nahrádza históriu): zdroj a čas záznamu každého okna, vysvetlenie zastaranosti, stav zdrojov tokenov (čas posledného
  úspešného čítania, chyby, neúplnosť), cena relácie zo statusline, poznámky k odhadu (cenník, „nie faktúra“). Dáta, zdroje ani 5 min interval sa nemenili.
- Šírka panela 620 → **440 lp** (výška 660); pri nedostatku miesta ustúpi história (a označí sa „nezmestilo sa“), limity nie.
- Overenie: audit popoveru **0 nálezov v plnej matici** (11 jazykov × 96/144/192 × obe témy × 4 fixtures × [dnes, mesiac, podrobnosti]); rozpočet výšky v najhoršom
  prípade (2 poskytovatelia × 3 okná, všetko zastarané) vo viacerých jazykoch. Opravené nálezy auditu: šírka záložiek podľa najdlhšieho prekladu
  (francúzske „Aujourd'hui“), šírka tlačidla Podrobnosti (tučné písmo).

#### 3. Udalosti v kalendári: reset a fakturácia
- **Zdroj fakturácie – zistenie:** v dokumentácii, ktorú som čítal (Claude Code `statusline` a `costs`, `monitoring-usage` len čiastočne – posledných ~42 tis. znakov
  som nečítal; Codex `pricing` a `config-reference` – posledných ~27 tis. znakov som nečítal; OTel a `/status` Codexu som nenačítal) **nie je žiadne pole s dátumom
  fakturácie/obnovenia** (najbližšie sú pohyblivé `resets_at` okien a `spend_limit.period` brány). Nič sa neskrabe, nič sa neodvodzuje z limitov → fakturácia je **len ručná**.
- **Automatický reset:** len pri spoľahlivom `resets_at` (aj zo starého záznamu – označí sa „zastarané“), ktorý ešte nenastal; deň = **miestny kalendárny deň**
  okamihu resetu. Pre poskytovateľov so zapnutým zdrojom. Po čase resetu, bez `resets_at` alebo pre expirované okno sa nič nezobrazí.
- **Ručné dátumy** (Nastavenia → AI a súkromie → „Kalendár“; konfigurácia **v5**, `usage.dates`, najviac 12, neplatné položky sa pri čítaní vynechajú bez „len na čítanie“):
  poskytovateľ, druh (reset / fakturácia), **prvý dátum** `RRRR-MM-DD` (pole prijíma aj `D.M.RRRR`, neplatný deň nikdy neopraví), opakovanie jednorazovo / **každých N dní
  (predvolene 6, 1–366)** / **mesačne**. Zoznam ukazuje „ďalší: …“ (jednoznačne), každý záznam sa dá upraviť a odstrániť. Mesačné opakovanie v kratšom mesiaci padne na posledný deň
  mesiaca, ale deň kotvy sa nestráca (31. → 28./29./30. → znova 31.).
- **Dátumová logika** (`transom-core/schedule.rs`) pracuje len s kalendárnymi dňami, nikdy s časom → letný čas ani zmena pásma nič neposunie; testy pre 6-dňové opakovanie
  (cez mesiace, rok, priestupný deň), mesačné (31., február, priestupný rok), DST dni a konzistenciu `next_on_or_after` ↔ `occurs_on` pre 800 dní × 3 pravidlá.
- **Kalendár:** pod číslom dňa **tvar**: kruh = reset, kosoštvorec = fakturácia (obe vedľa seba, ak sú v jeden deň), vysvetlivky pod mriežkou; po výbere dňa riadky
  `Claude Code: 5-hour window resets at 13:00 (automatic)`, `… Reset (manual, every 6 days)`, `… Billing (manual, monthly)`; najviac 4 + „+ N ďalších“. Automatický a ručný dátum
  v ten istý deň sa **nikdy neprepíšu** – sú to dve udalosti s označením „automatic“ / „manual“. Dni susedných mesiacov majú teraz kontrast ≥ 4,5 : 1 (predtým `disabled`, 2,8 : 1). Tlačidlo
  „Dnes“ sa mení podľa prekladu (francúzske „Aujourd'hui“ sa orezávalo). Popover 300 × 480 lp.
- Testy: automatický reset (platný / zastaraný / chýbajúci / uplynulý / vypnutý zdroj / mimo úseku), hranice dňa a pásma (UTC, +2, −5, +1), ručné jednorazové / 6-dňové / mesačné,
  automatický + ručný v ten istý deň, tvary značiek (rôzne, symetrické, kruh plnší) a ich kreslenie v bunkách, audit popoveru 11 jazykov × 96/192 × obe témy (výber dňa so 5 udalosťami).

#### Overenie a obmedzenia
- fmt, clippy `-D warnings`, release build, **268 testov** + 3 pomalé (`--ignored`), `cargo deny` ok; audit nastavení 0 nálezov (vrátane zoznamu dátumov a rozpracovaného formulára);
  `alpha_check` 12, `marquee_check` OK.
- **Neoverené / obmedzenia:** ticker som meral len pri 100 % DPI (150/200 % len cez testy; pri 120 px/s sú kroky 2 px); úzke okno s názvom pod 120 lp som naživo nepozoroval (len testy);
  natívne pole dátumu v nastaveniach sa offscreen nevykreslí (poloha je overená výpočtom a posunom pri scrolle, nie živým klikaním); popover AI a kalendár naživo neklikané
  (vykreslenie cez GDI offscreen a audit); preklady nových reťazcov okrem sk/en sú nerevidované; automatický reset nemá reálne dáta z Claude Code (statusline sa so skutočným Claude
  Code neoveroval); kontext sa zbiera len zo statusline Claude.

### 8.10 Míľnik: jednoduchší popover AI, diagnostika Claude „unavailable“, menu lišty, stránka Displays (2026-11-20)

**Postup.** Tri paralelné, len čítajúce prieskumy (žiadny agent nič neupravoval; úpravy robil jeden autor): (1) cesta dát Claude + CodexBar,
(2) návrh rozloženia podľa snímok a CodexBar, (3) menu/skrytie/Displays v existujúcom kóde. Konflikty a moje rozhodnutia: hlavná hodnota popoveru je
**zostávajúce %** a nezávisí od nastavenia „kruh ukazuje použité/zostávajúce“ (to ovplyvňuje len kruh v lište); skrytie lišty **nie je trvalé** (prieskum
navrhoval `state.json`; po reštarte je lišta zobrazená – menej rizika, že používateľ „stratí“ lištu); z menu som vynechal „AI usage…“ (zadanie má štyri položky).

#### 1. Prečo je Claude „unavailable“ – zistenie
- **Na tomto stroji:** `claude_local` aj `claude_statusline` boli vypnuté, `claude-statusline.jsonl` neexistuje a v `~/.claude/settings.json` **nie je `statusLine`** (prieskum
  kontroloval len prítomnosť kľúča, obsah súboru ani prihlasovacie údaje sa nečítali). **Chyba v kóde nie je** (polia `five_hour`/`seven_day`, `used_percentage`,
  `resets_at` v epoch sekundách sedia s dokumentáciou).
- **Cesta dát:** plánové okná Claude vznikajú **len** z mosta `transom-statusline` (zdokumentované `statusLine` → `rate_limits`; len claude.ai Pro/Max/Ultra a až po prvej
  odpovedi relácie). Lokálne prepisy (`~/.claude/projects`) dávajú **len tokeny** a nedokazujú limit predplatného. Codex funguje, lebo limit zapisuje sám do svojich relácií.
- **CodexBar** berie limity cez (a) OAuth endpoint s tokenom z Keychain/`.credentials.json`, (b) cookie `sessionKey` z prehliadača, (c) `claude` v PTY (`/usage`). (a) a (b) sú
  **nedokumentované a vyžadujú tajné údaje** → transom ich **neimplementuje a nezapína ani ako opt-in** (zadanie zakazuje čítať tokeny/cookies). (c) je krehké a nespúšťa sa.
  Jediný bezpečný zdroj plánových limitov Claude ostáva zdokumentovaný `statusLine`.
- **Diagnostika** (`ai::provider_hint`, bez ciest a tajomstiev; test kontroluje, že texty neobsahujú `AppData`, `credentials`, `.json`, …): *zdroj statusline vypnutý* → zapnúť v Nastaveniach;
  *most nemá záznamy* → skopírovať nastavenie statusLine do nastavení Claude Code a poslať jednu správu; *záznamy sú, ale žiadny nenesie limity* (`SourceStatus.limits_seen`) → účet limity
  nehlási (API/Console); Codex bez limitu → „zatiaľ nie je zaznamenaná relácia“. Veta ide do karty poskytovateľa („Limits unavailable“ + vysvetlenie), do tooltipu kruhu a do Podrobností
  (počet záznamov mosta a čas najnovšieho). Chýbajúca hodnota sa **nikdy** nezobrazí ako 0 %.
- Obmedzenia: statusLine už používateľa musí byť zreťazený sám (Claude Code pozná len jeden príkaz); `rate_limits.spend_limit` sa nezbiera; most sa so skutočným účtom Pro/Max na tomto stroji
  **neoveril** (nebol nastavený) – overené fixtures a parserom podľa dokumentácie.

#### 2. Popover AI (440 × 660 lp)
- **Normálny pohľad:** hlavička + prepínač *Details*; karta na poskytovateľa (`Claude` / `Codex`, štítok plánu len ak je známy); riadok okna: názov (`Session · 5 h`,
  `Weekly · 7 d`, `Monthly · 30 d`, inak podľa minút) a odpočet hneď za ním (plný text → skrátený `↻ 2 h 14 min` → riadok pod pruhom), vpravo **zostávajúce %** veľkým písmom + „left“,
  hrubý pruh zostávajúcej časti, pod ním malé „38 % used“. Stav slovom: `~62 %` + odznak **STALE · čas záznamu**, **RESET PASSED** + „waiting for new data“ (hodnota „—“, prázdny pruh),
  `!` + farba `low` pri ≤ 10 %. Nezlučujú sa nesúmerné okná. **História** (druhotná): záložky obdobia, `≈ $` odhad + „API-equivalent estimate · not a bill“, tokeny a % z cache, najviac 3 modely
  s pruhmi. Pätička: jeden riadok „Updated …“ (+ STALE) a tlačidlá.
- **Odstránené z bežného pohľadu:** cesty a popisy zdrojov, „recorded …“ na karte, kontext relácie, odznak MOST LEFT, NO RESET TIME, dva stavové riadky, poznámky o cenníku (sú v Details).
- **Details** nahrádza históriu: diagnostika Claude (dôvod + počet záznamov mosta), zdroj a čas záznamu okien, kontext, stav zdrojov tokenov (chyby, neúplnosť), cena relácie zo statusline,
  interval obnovy (**5 min predvolene aj minimum**, bez zmeny), poznámky k odhadu.
- **Referencia (snímky + CodexBar):** nasledované je len poradie informácií (provider → okná s odpočtom a veľkým %, odhad a modely nižšie, „Details“ mimo hlavného pohľadu); nie je
  prevzatý kód, farby ani texty; značky tempa a „On pace“ sú vynechané (odvodzovali by sa).
- Overenie: audit popoveru **0 nálezov v plnej matici** (11 jazykov × 96/144/192 × obe témy × 5 fixtures × [dnes, mesiac, podrobnosti]), rozpočet výšky v najhoršom prípade; fixtures: aktuálne,
  najhorší prípad zastarané + chyby, reset prešiel + bez resetu, Claude `statusLine` nepripojený, Codex 30 d.

#### 3. Menu lišty a skrytie
- Pravý klik na **prázdne** miesto lišty (nie na aktívny prvok – existujúce pravidlo `on_control`): **Settings / Customize bar / Hide bar / Quit transom** (lokalizované; test pre 11 jazykov: neprázdne a navzájom odlišné).
  *Customize bar* = `transom-settings --page layout` (aj `displays`, `media`, …); ak už nastavenia bežia, druhá instancia ich obnoví z minimalizácie, pošle správu `transom-settings-page-v1` (index stránky),
  dá do popredia a skončí; spúšťač povolí `AllowSetForegroundWindow`.
- **Hide bar:** všetky lišty sa zrušia (`ABM_REMOVE`, **pracovná plocha sa uvoľní**), popover sa zatvorí; ikona v oblasti oznámení sa **vynúti aj keď ju používateľ vypol** (`tray.tip_hidden`);
  ľavý klik na ňu = *Show bar*, pravý klik = menu *Show bar / Settings / Quit*; ďalšia cesta späť: **opätovné spustenie transom** pošle `transom-restore-v1` a skončí. Skrytie sa neukladá (reštart = lišta
  viditeľná). V nastaveniach nie je žiadny duplicitný prepínač viditeľnosti.
- Obmedzenie: na Windows 11 môže ikona skončiť v „skrytých ikonách“ (app nesiaha do registra); preto je tu aj opätovné spustenie.

#### 4. Stránka Displays
- Zoznam pripojených monitorov: číslo, **názov** (`QueryDisplayConfig`; napr. „LG ULTRAGEAR+“, „24G2W1G4“ – overené naživo), rozlíšenie, **mierka DPI**, štítok *Primary*, zapnutie zvlášť; režim
  *All / Primary / Selected* mení **len režim** (uložený výber ostáva); posledný zapnutý monitor sa vypnúť nedá; *Identify displays* na 2 s ukáže číslo a názov na každom monitore (okno bez fokusu,
  nad všetkým, nechytá klik). Zmeny sa uplatnia hneď (broadcast konfigurácie → `Request::SetMonitors`; pridanie/odobratie líšt cez existujúci `reconcile`, rezervácia pracovnej plochy a DPI sa nemenia).
- Konfigurácia **v6**: `displays { mode, selected:[{id, label}] }`, ID = device interface path (stabilné v rámci relácie; **medzi reštartmi neoverené**), popis sa ukladá na zobrazenie odpojeného monitora.
- **Odpojený monitor:** uložené ID sa **nikdy ticho nenahradí iným**: ak je pripojená časť vybraných, lišta je len na nich; ak **žiadny** vybraný nie je pripojený, lišta sa zobrazí na primárnom **s trvalým
  oznamom** a stránka ukáže varovanie + riadok „… not connected“ s tlačidlom *Remove*; po opätovnom pripojení sa lišta vráti sama. Zmena primárneho monitora: režim *Primary* ho nasleduje, *Selected* nie.
- Testy: jeden/viac monitorov, vyber ID bez ohľadu na veľkosť písmen, fallback na primárny a príznak, čiastočné pripojenie, odpojenie všetkých, zmena primárneho, migrácia v5 → v6 a tolerantné čítanie,
  prepínanie režimov bez straty výberu, posledný zapnutý, nepripojený záznam sa pri prepnutí zachová, popis (rozlíšenie/mierka 100/125/150/200 %). Audit stránky vo všetkých jazykoch 0 nálezov.
- **Živo overené (bez klikania a bez zmeny displejov; `tools`/skript `live-hide`, izolovaná konfigurácia):** `--start-hidden` → pracovná plocha **uvoľnená** (work area = celý monitor na oboch
  displejoch), opätovné spustenie transom ju vrátilo (28 px hore, log „lista zobrazena“), po ukončení čisto; výber *len nepripojený/druhý monitor* → rezervácia len na ňom (1 lišta); *Primary* → len primárny;
  *chýbajúci uložený monitor* → lišta na primárnom a log „vybrané monitory nepripojené, záloha na primárnom“ (viditeľný oznam som nenasnímal). Zoznam monitorov
  (`--example monitors`): „LG ULTRAGEAR+“ 2560 × 1440 100 % primárny; „24G2W1G4“ 1080 × 1920 100 %.
- **Neoverené:** hot-plug, odpojenie/opätovné pripojenie a zmena primárneho **naživo** (nesmiem meniť usporiadanie displejov – kryje to logika výberu a `reconcile`); zmiešané DPI iba cez fixture (150 %);
  *Identify*, spustenie nastavení z menu *Customize bar* a samotné kliknutie na menu (bez syntetického vstupu) naživo nebežali; stabilita ID monitora medzi reštartmi; vzhľad oznamu „vybraný monitor nie je pripojený“.
- Pri testovaní som s vaším súhlasom na pár minút ukončil vašu inštanciu transom (+ okno nastavení) a potom transom znova spustil z nového buildu s vaším konfiguračným súborom (po `Stop-Process` môže ostať duch ikony do prejdenia myšou).

### 8.11 Míľnik: pripojenie Claude Code, vlastné menu lišty, vlastný rám okna nastavení (2026-11-20)

**Postup.** Tri paralelné, len čítajúce prieskumy (žiadny agent nič neupravoval; úpravy robil jeden autor): (1) dokumentácia a inštalácia Claude Code (statusLine, Desktop vs. CLI) bez čítania
prihlasovacích údajov, (2) návrh vlastného menu na infraštruktúre popoverov, (3) návrh vlastného rámu okna (WM_NCCALCSIZE / WM_NCHITTEST / DWM). Rozhodnutia: F11 (celá obrazovka) som
**vynechal** (v dialógu nastavení by len pridala tretí stav okna; maximalizácia a Aero Snap ostávajú skutočné a vratné); prístupnosť hlavičky ide cez `OBJID_TITLEBAR` (nenahrádza sa klientsky objekt okna,
takže natívne polia ostávajú dostupné); menu používa popover (aktivovateľné okno + zachytenie myši), nie nový typ okna.

#### 1. Claude Code
- **Zistenia z dokumentácie** (code.claude.com/docs/en/statusline, settings, desktop): `statusLine` = `{"type":"command","command":"…"}` v `settings.json` (používateľský, projektový, lokálny; platí jediný kľúč, projektový
  prepíše používateľský); príkaz dostane JSON na stdin a musí skončiť kódom 0 s neprázdnym výstupom, inak je riadok prázdny; spúšťa sa po každej odpovedi (debounce 300 ms), zmeny nastavení sa preberajú bez reštartu;
  `rate_limits` iba pre claude.ai Pro/Max/Ultra a až po prvej odpovedi; do dôveryhodného priečinka treba potvrdiť dialóg dôvery. **Windows:** príkaz beží cez **Git Bash** (ak je nainštalovaný) – spätné lomky sa
  požierajú, preto sa píše cesta s lomkami (a v úvodzovkách, ak má medzeru). Predchádzajúce tlačidlo „Copy statusLine setting“ kopírovalo cestu so spätnými lomkami (**chyba – riadok by zostal prázdny**); opravené.
- **Desktop:** dokumentácia hovorí, že Desktop a CLI čítajú rovnaké `~/.claude/settings.json`, ale **nikde netvrdí**, že záložka Code `statusLine` spúšťa (je to funkcia terminálového rozhrania). Preto UI otvorene píše, že
  to funguje v termináli a pre Desktop to nie je zdokumentované. **Neoverené:** skutočná relácia v Desktope (potrebuje interakciu používateľa); stroj má CLI 2.1.329 a Desktop (MSIX) 5.11492.5.0, vlastnú verziu Claude Code
  v Desktope som nezistil. Riziko MSIX: zápis mosta z procesu Desktopu by mohol skončiť v presmerovanom profile balíka – „Test connection“ to hlási, ak nájde záznamy len tam.
- **Nastavenia → AI a súkromie → Pripojiť Claude Code** (`transom-usage/src/claude_setup.rs`, `transom-settings/src/cc.rs`): vysvetlí, **čo sa pridá a kam** (zobrazí sa cesta); pred zápisom **záloha** `settings.json.transom-backup-<čas>`
  (nikdy sa neprepíše); **textové vloženie jediného kľúča** `statusLine` (zvyšok súboru – poradie, odsadenie, CRLF, BOM – ostáva bajt po bajte; test porovnáva predponu) s **overením parserom** (všetky pôvodné kľúče rovnaké,
  inak sa nič nezapíše); atomický zápis cez dočasný súbor; **Vrátiť späť** (pôvodný súbor presne, alebo ak sa medzitým zmenil, chirurgické odstránenie nášho kľúča a obnova predošlého); neplatný JSON sa **nikdy nemení**;
  **cudzí `statusLine` sa neprepíše bez dvojkrokového rozhodnutia** (Nahradiť… → Nahradiť (záloha) / Ponechať môj; vysvetlenie, ako ho zreťaziť – Claude Code pozná len jeden príkaz; predošlá hodnota sa pri návrate obnoví).
  Stavy: nenastavené / nastavené, čaká sa na prvú snímku / pripojené (s limitmi alebo bez) / cudzí statusLine / chyba. **Test connection** spustí `transom-statusline --check` s vymysleným vzorkom (nič nezapisuje, nič z reálneho vstupu
  sa nezobrazuje) a porovná so záznamami mosta; pri chýbajúcej snímke sa zobrazí riešenie problémov (správa v terminálovej relácii v dôveryhodnom priečinku, účet Pro/Max/Ultra). Pripojenie je **vedomé povolenie** zdroja – zapne
  `usage.claude_statusline`. Žiadne OAuth/cookies/`.credentials.json`, nič sa nelogguje ani neposiela. Čítanie záznamov mosta berie len posledných 512 kB.
- **Overené (izolovaný profil `CLAUDE_CONFIG_DIR`, skript `cc-e2e.ps1`, nástroj `examples/claude_connect`):** súbor s kľúčmi (aj vnorenými) → pripojenie: kľúče nezmenené, záloha = pôvodný súbor; príkaz zo `statusLine` spustený
  **cez Git Bash** presne ako Claude Code (stdin = vymyslený JSON s `rate_limits`) → výstup „Sample · ≈$0.42 (estimate) · 5h 32 %“, kód 0, snímka zapísaná, stav **Connected (limity)**; vrátenie → pôvodný súbor **bajt po bajte**;
  súbor bez `statusLine` aj **bez súboru** (vytvorí sa, vrátením sa odstráni); cudzí `statusLine` bez rozhodnutia nezmenený, s rozhodnutím nahradený a vrátením obnovený; poškodený JSON nedotknutý. 11 unit testov
  (`claude_setup`). **Váš skutočný `~/.claude/settings.json` som nemenil** – stlačte *Pripojiť Claude Code* sami.

#### 2. Vlastné menu lišty
- Pravý klik na **prázdne** miesto lišty (nie na ovládací prvok – pravidlo `on_control` ostalo) a pravý klik na ikonu v oblasti oznámení otvárajú **vlastné menu** (`transom-app/src/menu.rs`, popover `POP_MENU`): *Settings*,
  *Customize bar* (otvorí Nastavenia rovno na stránke Layout), *AI usage…*, *Hide bar* (pri skrytej lište *Show bar*), oddeľovač, *Quit transom*; lokalizované vo všetkých 11 jazykoch. Žiadne natívne biele menu.
- **Klávesnica:** ↑/↓ (cyklicky, preskakujú oddeľovače a vypnuté položky), Tab, Home/End, Enter/medzerník, písmeno = skok na položku, Esc = zatvoriť. **Fokus je viditeľný** (zaoblené pozadie + akcentová značka vľavo), nie len farbou.
  **Myš:** hover = aktívna položka, klik = výber; **klik mimo menu** (aj na lište či v inej aplikácii) alebo strata aktivácie ho zatvorí; pravé tlačidlo vnútri sa ignoruje.
- **Platforma** (`transom-platform/src/popup.rs`): `Request::ShowMenu` s `MenuAnchor::{Bar, Screen}` (z ikony sa otvára smerom k stredu monitora, v hraniciach pracovnej plochy), veľkosť meria aplikácia (`BarHandler::popover_size`,
  texty v aktuálnom jazyku a DPI cieľového monitora), `SetCapture` pre klik mimo, **vrátenie popredia** pôvodnému oknu pri zatvorení (len ak bol popover ešte v popredí; pri prepnutí na panel AI sa popredie nevracia),
  zaoblené rohy cez DWM. Kalendár a panel AI sa nemenia.
- **Prístupnosť:** `MenuAccessible` (MSAA): koren `MENUPOPUP`, položky `MENUITEM`/`SEPARATOR`, stav fokus/hot-track aktívnej položky, `accFocus`, predvolená akcia, udalosti `MENUPOPUPSTART/END` a `OBJECT_FOCUS`.
- **Overené:** unit testy navigácie (cyklenie, preskakovanie, Home/End, Enter bez aktívnej, písmená, prázdne menu), rozloženie vo všetkých DPI, hit-test; **offscreen audit menu 11 jazykov × 96/144/192 DPI × obe témy × obe varianty × 3 stavy
  výberu – 0 nálezov**; **živý test v procese** (`--test-menu`: správy sa posielajú priamo oknu menu, nie ako vstup na ploche): menu sa otvorí pod lištou, štyri ↓ + Enter vyberú *Hide bar* (lišta sa skryje a obnoví),
  Esc ho zatvorí bez akcie; snímka obrazovky potvrdila vzhľad (zaoblené rohy, aktívna položka).
- **Neoverené naživo:** skutočný pravý klik a klik mimo menu (bez syntetického vstupu nesmiem); **vrátenie popredia a aktivácia popoveru** – v teste bol v popredí cudzí proces a `SetForegroundWindow` zlyhalo (test nemal vstup
  používateľa; pri skutočnom kliknutí proces vstup má, ako pri kalendári); čítačky obrazovky (Narrator) som nespúšťal.

#### 3. Vlastný rám okna nastavení
- Biela systémová hlavička je preč. Okno ostáva `WS_OVERLAPPEDWINDOW` (systémové menu, Alt+F4, Alt+Medzerník, tieň, Aero Snap, Windows 11 snap layouts), `WM_NCCALCSIZE` odstráni hlavičku (klient začína na hornom okraji; pri maximalizácii
  sa kompenzuje presah rámu), `WM_NCHITTEST` vracia `HTCAPTION` (ťahanie, **dvojklik = maximalizovať/obnoviť**, pravý klik = systémové menu), `HTMINBUTTON`/`HTMAXBUTTON`/`HTCLOSE` (snap layouts), `HTTOP*` (horný pás zmeny veľkosti
  vo vnútri klienta, 5 lp; ľavý/pravý/dolný rám je systémový), `WM_GETMINMAXINFO` (min. 640 × 480 lp, najviac pracovná plocha). Hlavička (`chrome.rs`, 40 lp): logo + wordmark + názov, tlačidlá s hoverom/stlačením (zatvoriť červené),
  neaktívne okno tlmené, tmavý/svetlý režim podľa Windows **aj za behu** (`WM_SETTINGCHANGE`), DWM: tmavý režim, farba okraja, zaoblené rohy. Logo sa presunulo z bočného panela do hlavičky; **všetky stránky a funkcie ostali**.
- **Klávesnica a prístupnosť:** Tab / Shift+Tab prechádza tlačidlá hlavičky, ←/→ medzi nimi, Enter/medzerník vykoná, Esc zruší; fokus = 2 px akcentový rámik. Tlačidlá majú lokalizované názvy a rolu `PUSHBUTTON` v `OBJID_TITLEBAR`
  (Minimalizovať / Maximalizovať ↔ Obnoviť / Zavrieť; 11 jazykov).
- **Overené:** 6 unit testov čistých funkcií (rozloženie pre 96–192 DPI, každý pixel hlavičky je presne jedna zóna, tlačidlá sa neprekrývajú, rohy majú prednosť pred tlačidlom Close, maximalizované okno nemá zmenu veľkosti,
  cyklus fokusu, min. veľkosť, príkazy); **audit všetkých stránok (vrátane nových stavov Claude) × 11 jazykov × 96/144/192 × obe témy – 0 nálezov**; **živý test okna** (správy priamo oknu, izolovaná konfigurácia): klient
  824 × 712 pod horným okrajom okna, `WM_NCHITTEST` pre titulok/Min/Max/Close/horný okraj/roh/obsah/ľavý a dolný rám = `HTCAPTION`/`HTMINBUTTON`/`HTMAXBUTTON`/`HTCLOSE`/`HTTOP`/`HTTOPLEFT`/`HTCLIENT`/`HTLEFT`/`HTBOTTOM`;
  kliknutie na Maximalizovať → `IsZoomed`, klient **presne vyplní pracovnú plochu** (2560 × 1440, T = 0), v maximalizovanom stave je pravý horný roh `HTCLOSE`, Obnoviť vráti 840 × 720, Minimalizovať/obnovenie z minimalizácie, Zavrieť ukončí proces;
  snímky normálneho a maximalizovaného okna skontrolované.
- **Neoverené:** Aero Snap ťahaním a Windows 11 snap layouts (vyžadujú skutočnú myš), pocit z hoveru a tahania, Narrator, správanie DWM okraja/tieňa na Windows 10, vyššie DPI naživo (kryje ho len audit; DPI nesmiem meniť);
  **F11 / celá obrazovka nie je implementovaná** (zámerne). Okno nastavení stále nepodporuje Tab medzi ostatnými prvkami stránky (nikdy nepodporovalo); klávesnica sa týka hlavičky.

#### 4. Nástroje na test a poznámky
- `--test-menu [esc]` (transom-app, **len test**): po prvom vykreslení otvorí menu a pošle klávesy priamo jeho oknu; `TRANSOM_TEST_INSTANCE` (transom-settings, **len test**): oddelí mutex jedinej inštancie, aby test nerušil vaše okno nastavení;
  `transom-statusline --check`: výpis bez zápisu. Skripty (v scratchpade relácie): `cc-e2e.ps1`, `live-menu.ps1`, `live-chrome.ps1`.
- Bridge `claude-statusline.jsonl` rastie neobmedzene (každá aktualizácia = riadok); čítanie berie len koniec súboru, orezávanie zatiaľ nie je.
- Pri testovaní som nemenil čas, časové pásmo, rozlíšenie, DPI ani jazyk systému; jediný súbor mimo projektu, ktorý by transom upravil, je `~/.claude/settings.json` – a to až po vašom kliknutí na *Pripojiť Claude Code*
  (so zálohou a vrátením späť).

### 8.12 Míľnik: mesačná história využitia a prepínač Left / Used v paneli AI (2026-11-21)

**Postup.** Prehliadka implementácie (model dát `Collected` / `DayTotals` / `SourceStatus`, panel `ai.rs`, karty `ai_cards.rs`, `usage.ring_mode`, PLAN §8.8–§8.11), kontrola skutočných zdrojov (len metadáta súborov a zapnuté zdroje v
konfigurácii) a poradca pred návrhom. Rozhodnutia: (1) modul histórie je **čistá funkcia v `transom-usage`** (`history.rs`), kreslenie je v `transom-app/src/ai_month.rs`; (2) Left/Used **znovu používa `usage.ring_mode`** (žiadny nový kľúč,
verzia konfigurácie sa nemení) – tým sa naraz prepne panel aj kruh v lište; (3) **neukladám vlastný archív** denných súčtov (do §8.12; voliteľný archív pribudol v §8.13) – história je len taká dlhá, ako lokálne záznamy (obmedzenie nižšie), aby sa nerozšírila trvalá stopa
v súkromí bez výslovného súhlasu.

#### 1. Mesačná história (karta „History“ → záložka **Month**)
- **Čo to je:** denné **tokeny** alebo **odhad ceny** (API-ekvivalent z cenníka, nie faktúra) podľa skutočného dátumu záznamu (lokálny deň Windows, rovnaké `by_day` ako zvyšok), za zvolený mesiac. **Čo to nie je:** história spotreby
  kvóty plánu – relačné / týždenné / mesačné limity sú len aktuálny snímok v kartách nad ním; popis pod grafom to hovorí vždy („Tokens from local logs · not plan quota“, „≈ API-equivalent estimate … · not a bill or plan quota“).
- **Ovládanie:** ‹ predošlý mesiac / dátum (**názov mesiaca podľa jazyka Windows**, `GetDateFormatEx`) / ďalší ›, **Tento mesiac** (zobrazí sa len mimo aktuálneho mesiaca), prepínač **Tokens | Est. cost**, filter **All | Claude | Codex**
  (len ak sú zapnutí aspoň dvaja poskytovatelia), klik na stĺpec = **rozpis dňa** (dátum podľa Windows, hodnota, poskytovateľ · model · tokeny · odhad ceny, najviac 3 + „+N more“); bez vybraného dňa sa ukáže rozpis mesiaca. Navigácia je
  ohraničená: dopredu len po tento mesiac, dozadu len po mesiac najstaršej známej histórie. Stav pohľadu (mesiac, deň, jednotka, filter) **nie je trvalý** – po otvorení panela je tento mesiac, tokeny, všetci.
- **Úplnosť dát (nikdy sa chýbajúci údaj nevykreslí ako nula)** – explicitný stav každého dňa, odlíšený **tvarom, nie farbou**: *recorded* plný stĺpec (skutočná nula = krátka čiarka), *partial* **pruhovaný** stĺpec (dolný odhad: prvý
  deň histórie zdroja mohol začať uprostred dňa, zdroj mal chyby čítania, alebo vo filtri „All“ niektorý poskytovateľ ešte nemá históriu), *no history* **bodkovaná čiara** (deň pred začiatkom najstaršieho záznamu zdroja alebo poskytovateľ bez lokálneho
  zdroja – **nevieme, nie je to nula**), *no price* **prázdny obrys** (len pri odhade ceny: všetky volania dňa bez ceny). Legenda zobrazuje len stavy prítomné v mesiaci. Mesiac bez jediného známeho dňa ukáže súčet „—“ (nie 0).
  Súčet mesiaca je zo známych dní; neocenené volania sú vedľa sumy („+ N unpriced“).
- **Poznámky pod grafom** (stručné, bez ciest): „{poskytovateľ}: local history begins {dátum}“ (história začína uprostred mesiaca), „No local history this early.“, „{poskytovateľ}: daily history isn't available (its local source is off)“
  (napr. Claude len cez statusLine – ten dáva limity a cenu relácie, nie denné tokeny; **statusLine sa do grafu nepočíta**, je to iná veličina), „some files couldn't be read – totals may be low“.
- **Zdroje a obmedzenia úplnosti:** denné tokeny z `~/.claude/projects/**/*.jsonl` (Claude Code **maže prepisy po `cleanupPeriodDays`, predvolene 30 dní**) a `~/.codex/sessions/**/*.jsonl`; čítajú sa len súbory, ktoré na disku ostali.
  Staršie mesiace preto môžu byť prázdne (stav *no history*); transom bez zapnutého archívu **nearchivuje** denné súčty (voliteľný archív: §8.13).
  Odhad ceny je z datovaného cenníka v čase čítania, nie z dobového.

#### 2. Left / Used v paneli
- Segmentovaný prepínač **Left | Used** (trvalý, nad kartami) prepisuje `usage.ring_mode`: panel **znova načíta konfiguráciu z disku, zmení len tento kľúč** a uloží ju (ak je súbor len na čítanie, zmena sa len použije v pamäti a zaloguje),
  hneď sa prekreslia **karty aj kruh v lište** a ostatné procesy sa dozvedia broadcastom `transom-config-changed-v1`. **Otvorené okno nastavení** si z broadcastu prevezme len `ring_mode`, aby ho jeho ďalšie uloženie ticho nevrátilo.
  **Mení sa rozhodnutie z §8.10**, kde karty vždy ukazovali zostávajúce % bez ohľadu na nastavenie kruhu – teraz sledujú voľbu.
- **Karty limitov:** veľké číslo = zvolená strana + slovo („left“ / „used“), pruh = zvolená strana, malé opačné číslo pod pruhom. Odpočet resetu, odznaky STALE / RESET PASSED, „~“ a názvy poskytovateľov ostávajú v oboch režimoch.
  Upozornenie „!“ (málo kapacity) vychádza **vždy zo zostávajúcej časti (≤ 10 %)**, aj v režime Used. Expirované okno má „—“ a prázdnu dráhu; chýbajúce alebo nepodporované okno sa nekreslí a percento sa **nikdy nedomýšľa**.
- Graf histórie sa pri prepnutí **nemení** – jednotky grafu sa volia samostatne (Tokens / Est. cost).

#### 3. Rozloženie, posuvník, klávesnica, prístupnosť
- Popover je 440 × **760** lp (bol 660); **hlavička a prepínač Left/Used, stavový riadok a päta sú pevné**, všetko medzi nimi (karty limitov, potom história / mesiac / Podrobnosti) je v **jednom posuvnom okne so zreteľným posuvníkom**
  (dráha + palec). Karty limitov sú na začiatku, takže pri bežnej veľkosti sú vidieť hneď; test zabezpečuje, že aj najhorší prípad (dvaja poskytovatelia × 3 okná, všetko zastarané) sa zmestí do okna bez posunu. Výška sa pri nízkej pracovnej ploche
  (vysoké DPI) **obmedzí na pracovnú plochu monitora** a obsah sa posúva. Posun: koliesko myši (nová udalosť `PopoverWheel`), ↑/↓, Page Up/Down, Home/End. V mesačnom pohľade: ←/→ vyberú deň, Home/End prvý/posledný deň,
  Page Up/Down mení mesiac, ak sa obsah nedá posúvať.
- **Prístupnosť:** panel publikuje tlačidlá (záložky, Left/Used, navigácia mesiacov, jednotka, filter, **každý deň grafu** s dátumom a hodnotou / stavom slovom, Obnoviť, Nastavenia) ako MSAA `PUSHBUTTON` v `GROUPING`
  (`Kind::Panel` v `menu_access`), aktívna položka = pod myšou, `EVENT_OBJECT_FOCUS`; názvy sú lokalizované (11 jazykov).

#### 4. Overenie
- **Testy:** `history.rs` – 12 testov (jeden stĺpec na deň vrátane prestupného februára a hraníc mesiacov, dni pred históriou = neznáme, nie nula; budúce dni; filter poskytovateľa a súčty; „All“ označí dni, ktoré pokrýva len časť poskytovateľov; poskytovateľ bez
  lokálneho zdroja; chyby čítania; mesiac pred históriou; odhad ceny s neocenenými dňami = neznáma cena, nie $0; **udalosti okolo lokálnej polnoci a konca mesiaca (UTC+1)**; rozpis dňa; mierka; súčet dní = súhrn za obdobie);
  `ai_month.rs` – navigácia (hranice, prelom rokov, „Tento mesiac“, výber dňa, šípky, trvanie jednotky/filtra); `ai_cards.rs` – texty Left/Used, „!“ podľa zostávajúcej časti, „~“, expirované okno bez čísla, slová vo všetkých jazykoch;
  `ai.rs` – kliky, otvorenie panela, klávesnica, rozsah posunu, pätička vždy klikateľná, názvy prístupnosti, označenie odhadu v každom jazyku; **audit vzhľadu** (11 jazykov × 96/144/192 DPI × obe témy × všetky fixtures × Today / 7 dní / Month
  (tokeny, odhad ceny s vybraným dňom, Codex v minulom mesiaci) / Podrobnosti × Left aj Used) – **0 nálezov**.
- **Skutočné lokálne dáta (len čítanie):** Claude Code – 20 súborov, 1 621 záznamov od 2. 11.; Codex – 1 súbor, 53 záznamov od 15. 11.; denné súčty tokenov november 2026 (30 dní) **porovnané so samostatným výpočtom** (`tools/history-check.ps1`,
  PowerShell, vlastná deduplikácia a lokálny deň podľa pásma Windows, nepoužíva kód transom): **30 dní, 0 rozdielov** (Claude aj Codex osobitne); príklad 2026-11-15: 13 233 187 + 829 582 = 14 062 769 tokenov. Nástroj `cargo run -p transom-app --example history_dump`
  vypíše tie isté dni z transom (stav dňa, tokeny, odhad ceny). **Odhad ceny som nezávisle nepočítal** (cenník a oceňovanie sú pokryté testami `pricing`); porovnal som len tokeny.
- **Živo (skript `live-ai.ps1`, `--test-ai`, správy priamo oknu popoveru – nie vstup na ploche; izolovaná kópia konfigurácie a údajov, skutočný `config.json` sa nemení):** otvorenie panela, klik na **Left** → konfigurácia `used → remaining`, kruh v lište
  **`4 % used` → `96 % left`** (v tom istom kroku), uloženie do súboru, klik späť na Used a znova Left, záložka **Month** na skutočnej histórii (stĺpce, legenda, súčet 630,29 M tokenov, rozpis piatku 20. 11. – 572,44 M),
  odhad ceny. Snímky obrazovky skontrolované.
- **Neoverené:** skutočné kliknutie myšou (správy sú posielané oknu), koliesko a klávesnica v živom popoveri (kryjú ich unit testy), Narrator, vysoké DPI naživo (kryje audit), čítanie Claude/Codex z iného profilu než váš, stabilita pri
  zmene časového pásma (nesmiem ho meniť; lokálny deň rieši existujúce `by_day` – test s UTC+1 okolo polnoci).
- Drobnosti: `fmt_tokens` má novú jednotku **G** (nad miliardu), desatinná čiarka v tokenoch ostáva pevná (jestvujúce správanie); `--test-ai` je test-only príznak.

### 8.13 Míľnik: voliteľný lokálny archív denných súčtov (2026-11-21)

**Cieľ.** Mesačná história (§8.12) je len taká dlhá, ako lokálne logy poskytovateľov (Claude Code maže prepisy po `cleanupPeriodDays`, predvolene 30 dní). Voliteľný archív drží **len denné súčty**, aby
história prežila zmazanie logov. **Predvolene vypnutý, nikdy sa nezapne sám.**

#### 1. Čo sa ukladá, kam a čo nie
- Súbor `usage-archive.json` v lokálnych dátach transom (`%LOCALAPPDATA%\transom`, `TRANSOM_DATA` ho presmeruje); atomický zápis (dočasný súbor + premenovanie). Nikam sa neposiela.
- **Jeden riadok na deň × poskytovateľ × model:** tokeny (vstup, výstup, čítanie / zápis cache), počet volaní, odhad ceny v USD (API-ekvivalent, **nie faktúra**) + počet volaní bez ceny, **pôvod**: poskytovateľ, deň, model,
  typ zdroja (lokálne prepisy Claude Code / relácie Codex), **dátum cenníka**, z ktorého je odhad, a príznak **„deň môže byť neúplný“** (`partial`). K tomu **pokrytie**: súvislé rozsahy dní `[najstarší záznam, dnes]` uspešných skenov.
- **Nikdy:** prompty, odpovede, názvy konverzácií, názvy projektov, cesty, ID relácií ani surové logy (test kontroluje, že súbor takéto polia neobsahuje; overené aj na skutočnom archíve).

#### 2. Nastavenia (AI a súkromie → „Archív dennej histórie (voliteľný)“)
- **Zapnutie je výslovný súhlas** (políčko); text vysvetľuje presne, čo sa ukladá, čo nie a kam (zobrazí sa cesta). Zapnutie archívu **nezapína žiadny zdroj** – bez „Claude Code – lokálne prepisy“ / „Codex“ sa nezapisuje nič
  (varovanie v UI) a samotné zapnutie súbor nevytvorí (vytvorí ho až zber).
- **Pozastaviť archivovanie** (nič nové sa nezapíše; uložené dni ostávajú a **zobrazujú sa**), **retencia** (bez obmedzenia / 3 / 6 / 12 / 24 mesiacov; predvolene 12; **kratšia retencia sa uplatní hneď**, dlhšia nič nemaže),
  stav súboru („Archived: N days, M entries · od – do · veľkosť“, varovanie pri poškodenom súbore), **Zmazať archivované údaje…** s **dvojkrokovým potvrdením** (ktorákoľvek iná akcia potvrdenie zruší; maže sa len archív – nie
  súbory Claude Code / Codex ani záznamy mosta statusLine). Vypnutie alebo pozastavenie nič nemaže (dni sa ďalej zobrazujú, kým ich používateľ nezmaže).
- **Konfigurácia v7** (`usage.archive {enabled, paused, retention_months}`): migrácia z v1–v6 **nikdy nezapne archív**, ostatné nastavenia sa zachovajú; neznáma / nezmyselná retencia = predvolená (súbor nie je kvôli tomu len na čítanie);
  novšia verzia ostáva len na čítanie. Zmena archívu v nastaveniach spustí hneď nový zber; zmazanie archívu zahodí aj dáta v pamäti panela a načíta zdroje znova.

#### 3. Zlučovanie: idempotentné, s opravami minulosti
Pri každom zbere (len ak je archív zapnutý a nepozastavený) sa čerstvé denné súčty zlúčia do archívu podľa kľúča (poskytovateľ, model, deň):
- **Deň, ktorý zdroj teraz číta celý** (po najstaršom zázname a bez chýb čítania): čerstvé čítanie je **autorita** a nahradí archivované riadky dňa → opakované skeny nič nepripočítavajú; ak sa záznamy neskôr opravili / deduplikovali,
  archív sa opraví (aj nadol, aj zaniknutý model sa odstráni). Cena sa tak prepočíta aktuálnym cenníkom len pre dni, ktoré zdroj ešte číta.
- **Hraničný deň** (najstarší záznam – zdroj ho mohol časovo orezať) a dni pri **chybách čítania**: pri každom modeli ostáva **väčší súčet** (nikdy sa nezmenší) a deň je označený **neúplný**; plný archivovaný záznam sa nikdy neprepíše menším.
- **Dni, ktoré zdroj už nemá, ostávajú nedotknuté** (o to ide). Chýbajúci / vypnutý / prázdny zdroj archív nemení a **nerozširuje pokrytie**.
- **Chýbajúce údaje sa nemenia na nulu:** deň mimo pokrytia je „bez histórie“ (diera medzi dvoma skenmi zostane dierou); deň v pokrytí bez riadkov je skutočná nula. Poškodený alebo novší súbor archívu sa **nepoužije a neprepíše**.
  Zápis len pri zmene obsahu (opakovaný sken nezmení ani čas súboru). **Retencia** maže staršie riadky aj pokrytie pri zápise.
- **Obmedzenie:** deň po najstaršom zázname, ktorý zdroj číta bez chýb, je považovaný za úplný; ručne zmazaný súbor uprostred histórie by archív pri ďalšom zbere „opravil“ nadol (nie je to bežný prípad – logy sa mažú podľa veku).

#### 4. História (graf) s archívom
Zjednotený pohľad = čerstvé čitateľné dni + dni z archívu. Každý deň má **pôvod**: z logov / **archivovaný** (zdroj ho už nečíta) / **zmiešaný** (filter „All“: jeden poskytovateľ z logov, druhý z archívu). V grafe je archivovaný deň
označený **pruhom pod osou** (plný = archív, bodkovaný = zmiešané), v legende „archived“, v rozpise dňa vetou („Archived total – the provider logs no longer contain this day. Price list: {dátum}“), v poznámkach mesiaca a v názve pre čítačky
(„…(archived)“). Neúplný archivovaný deň (hraničný pri archivácii) ostáva **čiastočný** (pruhovaný). Provider bez zapnutého zdroja, ale s archívom, už nie je „unsupported“ – jeho dni v pokrytí sa zobrazujú, ostatné sú „bez histórie“.
Navigácia po mesiacoch siaha po najstarší deň pokrytia (aj archivovaný).

#### 5. Overenie
- **Testy:** `archive.rs` 12 (agregácia a pôvod, idempotencia ×5, oprava po zmene / deduplikácii zdroja, dni, ktoré zdroj už nemá, hraničný deň sa nezmenší, chyby čítania, chýbajúci / prázdny zdroj, spájanie pokrytí, retencia vrátane
  mesiacov s rôznou dĺžkou, uloženie / načítanie / zmazanie / súkromie obsahu súboru, `prune_file`); `archive_flow.rs` 8 end-to-end (`collect` so syntetickými logmi: logy zmazané → súčty prežili a sú „archived“, opakované zbery
  zapíšu ten istý súbor, oprava zdroja, pozastavenie – číta sa, nezapisuje, vypnutý archív súbor nevytvorí a po zmazaní dni zmiznú, retencia, poškodený archív, diera medzi skenmi); `history_archive_tests.rs` 7 (pôvod, skutočné nuly z archívu,
  diery nie sú nuly, neúplný vs úplný hraničný deň, zmiešané dni, najskorší deň, odhad ceny s dátumom cenníka); `transom-config` (migrácia v6 → v7, nikdy zapnutý, tolerantné čítanie retencie, round-trip, archív nezapína zdroje);
  `transom-settings/archive_tests.rs` 6 (opt-in, pozastavenie, retencia sa uplatní hneď a dlhšia nemaže, dvojkrokové mazanie a zrušenie, poškodený súbor, texty vo všetkých jazykoch); `ai.rs` (archivované dni v názve a popise). **Audit vzhľadu** nastavení
  (nové stavy archívu: zapnuté / pozastavené + potvrdenie mazania / poškodený) a popoveru (fixture s archívom) vo všetkých jazykoch × 96/144/192 DPI × obe témy – **0 nálezov**.
- **Mesačné súčty proti nezávislému výpočtu** (`tools/archive-check.ps1`, použitý nezávislý `tools/history-check.ps1`): **skutočné logy** (len čítanie, archív v dočasnom súbore) – november 2026, 30 dní, **0 rozdielov**, mesiac spolu
  671 307 673 = 671 307 673 tokenov (Claude aj Codex osobitne), opakovaný beh bez rozdielu a archív neobsahuje cesty ani texty. **Syntetické logy** (dočasný profil): pred zmazaním logu transom = nezávislý výpočet; po zmazaní najstaršieho
  logu s archívom všetky dni rovnaké ako pred zmazaním (dni 2–5: 8 010 / 12 010 / 16 010 / 20 010), bez archívu sa stratilo 56 040 tokenov (dni 2–5 = 0); opakovaný beh nezmení ani bajt súboru.
- **Skutočná myš** (`tools/live-mouse-ai.ps1`; `SetCursorPos` + `mouse_event`, izolovaná inštancia s kópiou konfigurácie, **pred každým klikom sa overí, že pod kurzorom je popover práve tejto inštancie** – inak sa nekliká; kurzor sa po teste vráti):
  klik na **Left** → konfigurácia `used → remaining` a kruh v lište **`8 % used` → `92 % left`** (v tom istom okamihu), klik na **Used** späť, záložka **Month**, klik na **stĺpec dňa** (výber 21.), prepínač **Est. cost**, **koliesko** (3 zuby, posun 95/95).
  Skutočný `config.json` používateľa sa nezmenil. Test nemení rozlíšenie, DPI, čas, jazyk ani iné nastavenia systému. (Kliknutie „mimo menu“ a Narrator sa neskúšali.)
- **Neoverené:** archív s **vaším skutočným** nastavením som nezapínal (nemením vaše nastavenia; zapnete ho v Nastaveniach); oceňovanie archivovaných dní som nezávisle nepočítal (len tokeny); dlhodobé správanie po skutočnom zmazaní
  logov Claude Code po 30 dňoch (kryjú ho syntetické logy a testy).
- Pridané test-only nástroje: `history_dump --archive SUBOR`, `transom-app --test-ai-real`, skripty `tools/archive-check.ps1`, `tools/live-mouse-ai.ps1`.

### 8.14 Míľnik: audit spikov, bezpečnosti a okrajových prípadov, lokálny Git (2026-11-21)

**Cieľ.** Pred ďalším vývojom zistiť skutočný stav (nie to, čo tvrdia staré texty), dať projekt pod lokálnu Git kontrolu, opraviť len
konkrétne a vysoko isté chyby a zapísať, čo je overené akým spôsobom. **Nie je to vydanie:** žiadny inštalátor, remote, push ani tag.

**Metóda.** Päť paralelných agentov len na čítanie (audit spikov a dokumentácie, bezpečnosť a súkromie, okrajové prípady, Git/workflow,
produktové nápady). Agenti nič nespúšťali ani nemenili; hlavný agent každé tvrdenie, ktoré viedlo k oprave, **znovu overil priamo v kóde**
a niektoré návrhy zámerne nepoužil (nižšie). Úrovne dôkazov v tomto dokumente: **LIVE** = skutočná plocha, **TEST** = automatizovaný test,
**SIM** = simulácia (správa oknu, fixture, offscreen), **ODHAD** = úsudok z kódu bez behu.

#### 1. Git (lokálny, bez remote)
- Pred auditom to **nebol** repozitár. Teraz `C:\Projects\transom`, vetva `main`, 299 sledovaných súborov pred opravami, 15 commitov (6 základných, potom jedna oprava = jeden commit s testom,
  formátovanie a tento dokument).
- `.gitignore` vylučuje `target/` (root aj `spikes/*/target`, spolu ~15 GB), `*.bmp`/`alpha_check.csv` v koreni (výstupy `examples`), logy, `*.jsonl`,
  `config.json`, `usage-archive.json`, `claude-statusline.jsonl`, poverenia/cookies/tokeny a `spikes/s5-media/data/`. `Cargo.lock` (root aj spiky) je **sledovaný**.
  `.gitattributes`: `text=auto eol=lf` (pracovný strom je zmes CRLF/LF; v repozitári je LF).
- **Nič z toho sa nezmazalo**: ignorované súbory ostali na disku (BMP v koreni, dáta S5, `target/`).
- Lokálna Git identita je **zástupná** (`transom-dev`, `transom-dev@localhost.invalid`, len v `.git/config`); pred zdieľaním ju treba zmeniť a históriu prípadne prepísať.
- Pred základným commitom sa z `PLAN.md`, `spikes/s5-media/RESULTS.md` a `tools/ticker-measure.ps1` **odstránili skutočné názvy skladieb a videa** z lokálneho
  prehrávania (nahradené zástupnými textami). Surové logy S5 (`spikes/s5-media/data/`) obsahujú reálne názvy z prehrávania, preto sa **nesledujú**.
- `.github/workflows/ci.yml` sa nespustí, kým neexistuje GitHub remote. Neoverené: `cargo-deny-action` na `windows-latest` a testy závislé od GDI/písiem.

#### 2. Spiky: stav a typ dôkazu
| Spike | Stav | Dôkaz |
|---|---|---|
| S1 AppBar | čiastočne overený | LIVE podľa textu autora (bez surových logov); zmena DPI a reštart Explorera neoverené |
| S2 monitory | čiastočne overený | LIVE (2 monitory, Win+P), narativ; ID prežije len reláciu, po reštarte neoverené; monitory nad sebou a 175/200 % neoverené |
| S3 fullscreen | čiastočne overený | LIVE len na borderless okne; exkluzívny D3D, F11, video, HDR a pád fullscreen aplikácie neoverené |
| S4 hotkeys | overený pre bežné popredie | LIVE (CLI aj z lišty, §5.1); UIPI s adminom len raz a s výhradou, v `input.rs` **žiadny test** |
| S5 médiá | overený | LIVE, dva 30 min behy; surové CSV sedia s PLAN §6.1 (n = 1 na beh) |
| S6 R0/R1/R4 | overený | automatizované merania (skripty), výsledky v S7 RESULTS a §7.8; R2/R3 vedome neskúmané |
| S7 meranie | čiastočne overený | A, B = LIVE 11 min n = 3 so surovými CSV; C, D = **SIM** (SetCursorPos, 3 reťazce, nie GSMTC); scenár E (30 min) sa **nerobil** |
| S8 okno nastavení | prekonaný | nerobil sa; rozhodlo sa vlastné GDI okno (§7.7) |
| S9 vzhľad | overený pre plnú farbu | LIVE; Mica/Acrylic nepreukazné, vedome mimo MVP |

**Stále nevyriešené technické riziká pod odoslaným kódom** (len toto odôvodňuje nový spike):
1. **S3b – exkluzívny fullscreen a uviaznutá skrytá lišta.** Kód používa len `ABN_FULLSCREENAPP` (hide); `keep_visible`, vlastný detektor a nastavenie v `config.json`
   v kóde **nie sú**, hoci §4 a §1a ich opisujú. Ak „zatvorená“ udalosť nepríde, lišta ostane skrytá (ODHAD).
2. **S10 – skutočný Claude Code `statusLine` end-to-end** (CLI aj Desktop/MSIX): limity Claude boli overené len syntetickým vstupom a `transom-statusline --check`.
3. **S11 – tlačidlo skrytých ikon bez lokalizovaného názvu.** `OVERFLOW_NAMES = ["Show Hidden Icons"]` funguje len v angličtine; v ostatných 10 jazykoch sa Enter nikdy neodošle.
4. **Merný beh E (30 min)** produkčnej lišty (GSMTC + obaly + ticker + zber AI; sleduje handles a pamäť). Ticker (1,04 % / 2,71 % jadra počas prehrávania, n = 1, len 100 % DPI)
   je **vedomá výnimka z brány G1** (§7.4: bez súvislej animácie), ktorú treba v §7.4 zapísať.

Poznámka k §7.8: produkčný renderer je **GDI cez `WM_PAINT`** (ClearType pri 100 %; vrstvené okno len pod 100 % DPI), nie „GDI + UpdateLayeredWindow“ zo spiku R0.
Čísla S7 (11 MB) preto nie sú priamo prenosné na produkciu (25 MB WS v §8.x); podmienky merania sa líšia, nejde o preukázanú regresiu.

#### 3. Register rizík (bezpečnosť, súkromie, spoľahlivosť)
Nasadenie modelu hrozieb: lokálny proces s rovnakými právami **nie je** hranica dôvery; sieť v kóde **nie je** (grep + `cargo tree`, žiadne HTTP/WinHTTP/telemetria/auto-update);
jediný `Command::new` je súrodenecký `transom-statusline.exe --check` so syntetickým vstupom; `SendInput` len po kontrole popredia/UIPI/modifikátorov.

| ID | Riziko | Stav | Dôkaz |
|---|---|---|---|
| B3 | Poškodený/rozpísaný `config.json` vrátil bežiacu lištu na predvolené (monitory, rozloženie, zdroje AI) | **opravené** `1309639` | TEST `a_corrupt_or_unreadable_reload_keeps_the_running_configuration` |
| F2 | `connect`/`rollback` čítali `settings.json` stratovo (U+FFFD) a prepisovali reálny súbor používateľa | **opravené** `3cdd92f` | TEST `non_utf8_settings_are_never_rewritten_or_backed_up`; návrat zálohy je teraz atomický |
| F1 | `command_for` uvodzovkoval len pri medzere (`&`, `(`, `;`, `'` rozbijú príkaz) | **opravené** `3cdd92f` | TEST `command_quotes_every_path_the_shell_would_interpret`; bežné cesty bajt po bajte rovnaké; príkaz s `$`, spätným apostrofom, riadkom alebo vnútornou uvodzovkou sa **odmietne** |
| F5 | Hodnota `u64::MAX` v logu pretiekla súčty (panika v debug, zlé súčty v release) | **opravené** `ab289d1` | TEST `absurd_token_values_are_clamped…` (strop 2^40 na pole a záznam, saturácia v `by_day`) |
| F3a | `writeln!` do mostu = viac zápisov, paralelné relácie prekladali riadky | **opravené** `063958f` | TEST `concurrent_appends_stay_whole_lines…` (8 vlákien, poškodený koniec) |
| F6a | Dekompresná bomba obalu (8 MB limit nestačí) a `w = 0` | **opravené** `c311b5d` | TEST `art_dimensions_are_limited_against_decompression_bombs` (≤ 16 384 px na stranu, ≤ 64 Mpx) |
| T1 | Občasné zlyhanie `disabled_sources_read_nothing` | **príčina nájdená a opravená** `edca783` | test v `main.rs` prepínal globálny jazyk bez `test_lang::guard()` |
| F3b | `claude-statusline.jsonl` rastie bez limitu a `read_file` ho číta celý | **otvorené, vedome nezmenené** | `read_file` z neho počíta **históriu nákladov po dňoch**; orezanie by stratilo staršie dni. Treba kompaktovanie na denné súhrny (návrh nižšie), nie orezanie |
| F5b | Reťazec modelu z logu sa neočisťuje (nafúkne cache/archív); `archive.rs` nerobí saturáciu pri pozmenenom súbore | otvorené | model môže niesť napr. `[1m]`; biela listina by zmenila kľúče archívu a ceny. Zatiaľ len odporúčanie: strop dĺžky a počtu modelov |
| F4 | Pri každom zbere sa celé prepisy čítajú bez limitu a bez cache podľa mtime | otvorené | ODHAD; návrh: limit riadka/súboru + cache (veľkosť, mtime) |
| B1/B2 | Tray: po reštarte Explorera sa pri skrytej lište ikona nepridá; pri 2+ lištách `readd` beží N-krát (`NIM_ADD` na existujúcu ikonu zlyhá, duch po ukončení); zlyhanie `NIM_ADD` sa nezapamätá | **opravené v §8.15** (SIM/TEST; LIVE čaká na súhlas) | kód `appbar.rs`/`tray.rs` (ODHAD z čítania); overenie vyžaduje **reštart Explorera**, na to treba súhlas používateľa |
| B4 | Nastavenia zapisujú celý `config.json` (z pamäte) a prepíšu zmenu iným procesom; spoločný `config.json.tmp` | **opravené v §8.15** (TEST) | ODHAD z kódu |
| B5 | Zber, ktorý už beží, po „Delete data“ znovu zapíše cache/archív; panika vo workeri nechá `busy` navždy | **opravené v §8.15** (TEST) | ODHAD z kódu |
| B6 | `input.rs::precheck` blokuje len `elevated == Some(true)`; pri `None` sa Enter odošle a UIPI ho potichu zahodí (zachytí až `verify`) | **zlepšené v §8.15** (TEST; LIVE čaká) | ODHAD; `input.rs` nemá testy |
| F8 | AUMID z GSMTC ide do `shell:AppsFolder\…` bez validácie | otvorené, **nepoužité** | biela listina `x!y` by zamietla reálne relácie (`Brave` bez `!`) a rozbila overenú aktiváciu; vyžaduje kliknutie používateľa |
| F6b | Texty médií bez limitu; `.join()` WinRT bez timeoutu (`Drop` čaká) | otvorené | skrátenie názvu sa dotýka identity média a väzby obalu; dopad ODHAD |
| F10 | „Delete local data“ nemaže `*.tmp`, `claude-setup.json` ani zálohy `settings.json.transom-backup-*` (môžu niesť tokeny z `settings.json`) | otvorené | dokumentovať; ponúknuť mazanie záloh po návrate |
| F7/F9/F12/F13 | Spoločný `.tmp`, relatívne cesty pri chýbajúcich premenných prostredia, mutex bez DACL, CI bez `permissions:` a SHA pinov | otvorené, nízke | ODHAD |

Dobré (overené čítaním): žiadna sieť a telemetria, 33 balíkov v `Cargo.lock`, `cargo deny` bans/licenses/sources/advisories v poriadku, `unsafe_op_in_unsafe_fn = deny`, tolerantný parser konfigurácie
(poškodený alebo novší súbor sa neprepíše), archív ukladá len denné súčty (bez promptov a ciest), prepisy sa čítajú len v poliach `message.usage`, `model`, `id`, `timestamp`, `auth.json` a poverenia sa
nikdy neotvárajú, `statusLine` sa upravuje textovo s overením parserom, zálohou a návratom.

#### 4. Matica okrajových prípadov (zhrnutie; plná tabuľka bola výstupom agenta 3)
Legenda: **U** unit test, **S** offscreen simulácia/audit, **M** správa poslaná priamo oknu/procesu (nie skutočný vstup), **L** skutočná plocha (dokumentovaná v PLAN), **N** nepokryté.

| Oblasť | Pokrytie | Hlavná medzera |
|---|---|---|
| Monitory: žiadny/jeden/primárny/neplatné ID | U (`select`, `reconcile`), nová sekvenčná simulácia (odpojenie, návrat s iným DPI, zmena primárneho, fallback, žiadny monitor) | LIVE hot-plug a zmena DPI **neoverené** (nesmie sa meniť rozlíšenie/DPI); stabilita ID po reštarte neoverená |
| Shell: pád lišty, hide/restore | L (`gate-appbar.ps1`, skripty v scratchpade) | reštart Explorera (B1/B2) **N**; skripty z §8.10–8.12 (`live-hide`, `live-menu`, `live-chrome`, `live-ai`, `cc-e2e`) nie sú v `tools\`, **nereprodukovateľné** |
| Nastavenia: druhá inštancia, klávesnica, prístupnosť | U (hlavička okna) | Tab medzi prvkami stránky a názvy ovládačov stránok **N**; Narrator neotestovaný; Tab v EDIT poliach sa zhltne |
| Batéria | U (bez batérie, neznámy stav), L (stolný PC) | viditeľná batéria len simulovaná |
| Médiá | U (zmiznutá relácia, viac relácií, chýbajúce metadáta, neplatná timeline, obal) + nový limit rozmerov | oneskorený obal po zmene skladby; Edge/Chrome; časový limit WinRT |
| Search/Tray skratka | S4 + L (§5.1) | `input.rs` bez testov (UIPI, modifikátory, povýšené okno), lokalizované UI iné než EN |
| Čas, polnoc, mesiac/rok, DST | U s vstrekovaným časom | skutočný DST a „intl“ zmena locale **N**; testy `locale_formatting_smoke` a `high_resolution_waitable_timer…` závisia od hostiteľa (možné flaky v CI) |
| AI: vypnuté/odpojené/zastarané/reset | U, S (audit) | súbežné obnovy (B5) |
| AI: neúplné súbory, duplicity, archív | U + nové: skrátený posledný riadok, neplatné UTF-8, duplicitný záznam, prázdny súbor, `u64::MAX` | symlinky/junction (F14) neotestované; cache a most nemajú verziu schémy |
| Vzhľad: úzke monitory, dlhé preklady, priehľadnosť, kontrast | U + S (audit 11 jazykov × 3 DPI) | plné matice sú `#[ignore]` a v CI nebežia; Windows High Contrast v kóde **nie je** |
| Reduced-motion | U | zmena v systéme naživo neoverená |

#### 5. Priorizované ďalšie kroky
1. **Tray odolnosť (B1/B2)** – rozdeliť na čisté stavy a falošné rozhranie shellu, test „`readd` 2×, `NIM_ADD` zlyhá“; potom live overenie **s tvojím súhlasom** (reštart Explorera).
2. **Nastavenia: bezpečnosť zápisu (B4, B5, F7)** – znovunačítať a zlúčiť len zmenené polia, jedinečný `.tmp`, generačné číslo pri „Delete data“.
3. **Most `statusLine`: kompaktovanie (F3b)** – staré záznamy zlúčiť na denné súhrny nákladov (zachová históriu), posledný snímok limitov ponechať.
4. **Klávesnica a Narrator v nastaveniach** (Tab medzi ovládačmi, názvy, Tab v EDIT poliach) – pred akýmkoľvek verejným vydaním.
5. **Autostart** (`HKCU\…\Run`, predvolene vypnutý) a **diagnostická stránka súkromia** (stav zdrojov, veľkosti cache/archívu/mosta, verzie; bez ciest) – väčšinu údajov už máme.
6. **S3b, S10, S11 a merný beh E** (riziká v §8.14 bod 2), potom **upozornenie na limit AI** (využíva existujúci `Notifier`).
Nápady vedome **nerobiť**: súhrn notifikácií Windows (vyžaduje čítanie obsahu správ, proti zásade „nikdy obsah“), CPU/RAM/sieť v lište (proti bráne G1), auto-hide lišty (proti AppBar rezervácii).

#### 6. Overenie tohto míľnika
- Formát, `clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, `cargo deny check` (vrátane advisories) a release build (do oddeleného `CARGO_TARGET_DIR`, lebo
  `target\release` môže používať bežiaca lišta): všetko **prešlo** (**TEST**, 2026-11-21): `cargo fmt --check` čistý; `clippy -D warnings` bez nálezov; `cargo test --workspace` **378 testov prešlo, 0 zlyhalo, 3 ignorované**
  (app 79, config 23, usage 92, settings 25, render 50, platform 36, ďalšie crate-y 70); `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok; release build do oddeleného adresára OK.
  Nové testy v tomto míľniku: 8 (config reload, `settings.json` UTF-8, quoting, orezanie tokenov, súbežný zápis mostu, rozmery obalu, hot-plug sekvencia, poškodený prepis).
  `target\release` v projekte je **starší** než tieto opravy (neprebuildoval som ho, lebo ho môže používať bežiaca lišta).
- Nič z toho **nemenilo** čas, časové pásmo, rozlíšenie, DPI, jazyk systému, poverenia ani nastavenia účtov a nespúšťalo reálne prehrávanie. Skutočný `config.json`,
  `%LOCALAPPDATA%\transom` a `~/.claude/settings.json` sa **nezmenili** (testy bežia v dočasných adresároch).
- **Neoverené v tomto míľniku:** akýkoľvek LIVE test (reštart Explorera, hot-plug, skutočný Claude Code), beh CI, reálne správanie WIC na dekompresných bombách (testovaná len čistá funkcia
  rozmerov), junction/symlink pri čítaní prepisov.

### 8.15 Míľnik: obnova tray ikony, integrita dát a klávesnica v nastaveniach (2026-11-21)

**Vetva** `hardening/tray-integrity-a11y` (z čistej `main`, bez remote). Metóda: štyria agenti na čítanie (tray, integrita dát, UIPI a klávesnica, návrh testov), potom
jeden agent (hlavný) zlúčil a opravil po commitoch; **každú opravu s testom v samostatnom commite**. Označenia: **TEST** = automatizovaný test, **SIM** = simulácia
(falošný shell, offscreen kreslenie, vstrekované hodnoty), **LIVE** = skutočná plocha. **V tomto míľniku sa nerobil žiadny LIVE test** (Explorer sa nereštartoval,
nič sa nespúšťalo povýšené, Narrator sa nespustil, `config.json`, `%LOCALAPPDATA%\transom` a `settings.json` používateľa sa nezmenili).

#### 1. Zmeny a dôkazy
| Riziko (§8.14) | Oprava | Commit | Dôkaz |
|---|---|---|---|
| **B1/B2** tray po `TaskbarCreated` | `tray_model.rs`: čistý stavový automat (`Set`, `TaskbarCreated`, `Retry`, výsledky `Added`/`Modified`) + `Shell` trait; `tray.rs` je tenký Win32 shell s evidenciou `HICON`; `TaskbarCreated` sa spracuje **raz v riadiacom okne** (`ctl_proc`), nie v okne každej lišty (AppBar sa stále registruje pri každej lište); neúspešné pridanie ostáva žiadané a opakuje sa (časovač 2 s, najviac 5×); `NIM_ADD`, ktorý zlyhal, sa overí jedným `NIM_MODIFY` (duplikát / `ERROR_TIMEOUT`); `NIM_MODIFY` na zaniknutú ikonu → `NIM_ADD`; ikona sa odstráni **pred** zničením vlastníka | `b41d3c1` | **SIM/TEST** 11 testov s falošným shellom: 5 reštartov = presne 6 `NIM_ADD`, nikdy 2 ikony; opakovaná správa nepridá duplikát; skrytá lišta ikonu udrží (automat o lištách nevie); zlyhané pridanie sa zopakuje; opakovania sú ohraničené; probe po `ERROR_TIMEOUT`; odstránenie = presne 1 `Delete`. **LIVE: nie** (pozri §3A) |
| **B4/F7** stratená zmena `config.json` | `transom_config::save` (unikátny `.tmp`, `sync_all`), `update` (pod zámkom znova načíta, zmení jedno pole, odmietne poškodený/novší súbor) a `save_merged` (trojcestné zlúčenie: nastavenia zapíšu len to, čo používateľ zmenil oproti stavu, z ktorého vychádzali; cudzie zmeny a neznáme kľúče ostanú); lišta zapisuje `ring_mode` cez `update` | `6de4fef` | **TEST** dvaja zapisovatelia × 40 iterácií (rôzne polia): obe zmeny prežijú, žiadny zvyšný `.tmp`; poškodený/novší súbor sa nikdy neprepíše (bajty rovnaké); zlúčenie zachová cudziu zmenu a neznáme kľúče |
| **B5** mazanie archívu/cache počas zberu | archív: odtlačok `(existuje, veľkosť, mtime)` pred čítaním a zápis len ak sa nezmenil (`save_if_unchanged`); cache/archív zapisujú cez `write_atomic` (unikátny `.tmp`, fsync); `AiState.epoch` sa zvýši pri zmazaní dát: zastaraný zber nezapíše cache ani nevráti výsledok, neskorý výsledok sa zahodí aj s cache, po skončení zastaraného zberu sa spustí nový; panika vo workeri a zlyhaný `spawn` už neblokujú ďalšie zbery; otrávený mutex neblokuje príjem | `2270433`, `3df281e` | **TEST** (dočasné adresáre, žiadne skutočné dáta): zmazanie/retencia počas zberu archív nevzkriesi; zmazanie počas zberu, neskorý výsledok, normálny výsledok, panika, otrávený zámok |
| **B6** UIPI predkontrola | `input.rs`: `Integrity` (z `TokenIntegrityLevel`), čistá `decide()`: najprv porovná integritu popredia a vlastnú (to, čo UIPI zahadzuje), potom príznak zvýšenia; neznámy stav sa pošle a overí `verify` (proces inej identity sa nesmie falošne blokovať) | `edbd02a` | **TEST** 7 testov (hranice RID, tabuľka rozhodnutí, čítanie vlastného tokenu); `SendInput` sa nevolá. **LIVE s povýšeným oknom: nie** (§3B) |
| Nastavenia: klávesnica | `focus.rs`: kruh fokusu = ovládače stránky + viditeľné polia + tlačidlá hlavičky, `route_key`; Tab/Shift+Tab, Enter/Space (rovnako ako klik), Esc, viditeľný rámik, automatický posun obsahu. **Koreň chyby:** hlavná slučka smerovala kláves aj z poľa EDIT do logiky hlavičky, takže Enter z poľa HEX mohol stlačiť Minimalizovať/Zavrieť | `e40e7fb` | **TEST/SIM** čisté testy kruhu a smerovania (vrátane koreňovej príčiny); na skutočne nakreslených 7 stránkach (En/96 a De/192 dpi) je každý dosiahnuteľný ovládač v kruhu raz, Tab prejde všetky, vzorka sa posunie celá do zobrazenia. **LIVE s reálnymi klávesmi a Narratorom: nie** (§3C) |
| Nastavenia: prístupnosť | názvy polí HEX/R/G/B/dátum cez `IAccPropServices` (preložené, obnovia sa pri zmene jazyka) | `14bbe6f`, `d7871e5` | **TEST** názvy sú rôzne, neprázdne a preložené vo všetkých 11 jazykoch; skutočné skryté pole EDIT vráti názov cez MSAA `accName` (v procese testu). Narrator **nie** |

Poznámka k F10/F3b (zálohy `settings.json`, kompaktovanie mostu) a ďalším položkám z §8.14: **nemenené**, ostávajú otvorené.

#### 2. Čo ostáva otvorené (po tomto míľniku)
- **Vyriešené v §8.16 (TEST/SIM; LIVE čaká):** ~~Prístupnosť ovládačov stránok (OBJID_CLIENT):~~ tlačidlá, voľby a zaškrtávacie polia kreslené v GDI nemajú MSAA názov, rolu ani stav (iba hlavička okna a pole). Treba `PageAccessible` (návrh agenta 3: `HitBox{rect, hit, name}` pri kreslení + role/stav) a Narrator test. Klávesnica už k nim vedie.
- **Vyriešené v §8.16:** ~~Zmena poradia položiek lišty (chip) a posuvník nepriehľadnosti ostávajú len myšou;~~ klávesnicová alternatíva (Alt+šípky) nie je.
- **Vyriešené v §8.16:** ~~Esc v poli pole len opustí (hodnota sa uloží); návrat pôvodnej hodnoty nie je.~~
- **LIVE overenia** v §3. **Kľúčové nové zistenie:** pred týmto míľnikom bol `main` v stave, keď `TaskbarCreated` pridával ikonu N-krát a pri skrytej lište vôbec; po oprave to dokazuje len simulácia, nie skutočný Explorer.
- Ďalšie otvorené riziká z §8.14 (F3b, F4, F5b, F6b, F8, F10, F13) bez zmeny.

#### 3. LIVE postupy (vykonáva používateľ; nič z toho sa nespúšťalo)
**A. Tray ikona po reštarte Explorera** (vyžaduje **tvoj výslovný súhlas**; dočasne zmizne panel úloh, plocha a oznamovacia oblasť; zatvoria sa okná Prieskumníka; ikony iných programov, ktoré nepodporujú `TaskbarCreated`, sa nemusia vrátiť).
1. Ulož prácu vo všetkých aplikáciách. Spusti transom so zapnutým logom: `transom-app.exe --log C:\temp\transom-live.log` (mimo projektu; log neobsahuje dáta AI). V Nastaveniach → Vzhľad nech je zapnutá „ikona v oblasti oznámení“.
2. Zapíš si: `tasklist /fi "imagename eq transom-app.exe"` (PID), `tasklist /fi "imagename eq explorer.exe"` (PID), počet ikon transom v oblasti oznámení (aj v skrytých), poslednú ~20 riadkov logu.
3. Reštart: Správca úloh (Ctrl+Shift+Esc) → Procesy → *Windows Explorer* → Reštartovať. (Nepoužívaj `taskkill` na transom.) Počkaj 10 s.
4. **Očakávané (úspech):** PID transom nezmenený, PID explorer nový; po reštarte je ikona transom **presne jedna** (nie dve) a klik na ňu otvorí Nastavenia (alebo vráti skrytú lištu); lišta je znova rezervovaná (maximalizované okná začínajú pod ňou); v logu je riadok `TaskbarCreated` s `pridana=true`.
5. Opakuj 5×. Potom (a) s **dvoma monitormi** (kontrola duplicity pri 2 lištách) a (b) so **skrytou lištou** (pravý klik → Skryť lištu; ikona je jediná cesta späť; po reštarte musí ikona vzniknúť).
6. **Neúspech:** ikona chýba / sú dve alebo „duch“ (zmizne až po prejdení myšou) / `pridana=false` bez ďalšieho pokusu / lišta neprepočítala polohu. Návrat: v Nastaveniach vypni a zapni ikonu, poslednou možnosťou je Ukončiť transom a spustiť znova. Výsledok (úspech/neúspech, log) zapíš do PLAN §8.15.
**B. UIPI s povýšeným oknom:** otvor sám okno „Spustiť ako správca“ (napr. PowerShell) a daj mu popredie; transom beží nepovýšený. Klikni na Search a na Tray. **Očakávané:** skratka sa neodošle, lišta ukáže oznam o povýšenom okne, do konzoly nič nepríde. Potom klik z bežnej aplikácie: skratka funguje. Nič sa nespúšťa povýšené z transom; log `popredie pred: …` ukáže integritu a príznak zvýšenia (údaj z `TokenIntegrityLevel` na tvojom systéme je LIVE neoverený).
**C. Narrator / klávesnica v Nastaveniach:** Win+Ctrl+Enter zapne Narrator. V Nastaveniach: Tab prechádza navigáciu → ovládače stránky → polia → Minimalizovať/Maximalizovať/Zavrieť → von, Shift+Tab opačne; Enter/Space aktivujú; v poli Enter potvrdí a opustí pole, Tab pole opustí; Esc ruší fokus. Narrator má čítať názvy polí a tlačidiel hlavičky; **názvy ovládačov kreslených stránok sa čítať nebudú** (známa medzera, §2).

#### 4. Overenie tohto míľnika
- `cargo fmt --check` čistý; `cargo clippy --workspace --all-targets -D warnings` bez nálezov (po oprave jedného lintu v teste); `cargo test --workspace`: **413 testov prešlo, 0 zlyhalo, 3 ignorované** (app 82, config 27, core 67, i18n 3, media 3, platform 53, render 50, settings 35, usage 93); `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok; release build do oddeleného `CARGO_TARGET_DIR` OK (`target\release` v projekte je **starší**, lebo ho môže používať bežiaca lišta).
- **Opakovanie testov s predošlým občasným správaním:** `cargo test -p transom-app` (obsahuje `disabled_sources_read_nothing`) 5× za sebou (workspace beh, 3 samostatné opakovania a beh po poslednej zmene): **0 zlyhaní** (príčina bola opravená v §8.14, `edca783`). Testy nastavení, ktoré menia globálny jazyk alebo premenné prostredia, teraz zdieľajú jeden zámok (`archive_tests::ENV`).
- **Neovplyvnené:** čas, časové pásmo, usporiadanie displejov, DPI, jazyk systému, poverenia, nastavenia poskytovateľov ani reálne dáta o využití AI. Testy používajú dočasné adresáre a `TRANSOM_CONFIG`/`TRANSOM_DATA`; jediný zásah do systému je vytvorenie skrytého okna s poľom EDIT v procese testu a čítanie vlastného tokenu.
- **Git:** vetva `hardening/tray-integrity-a11y`, commit na každú opravu s testom (pozri tabuľku).


### 8.16 Míľnik: prístupnosť GDI ovládačov a klávesnica pre presun, nepriehľadnosť a polia (2026-11-21)

**Vetva** `hardening/tray-integrity-a11y` (lokálna, bez remote). Metóda: traja agenti na čítanie (prístupnosť MSAA/UIA, klávesnicové ovládanie, zvyšné riziká z §8.14), potom
implementácia po commitoch s testami. Označenia: **TEST** = automatizovaný test (aj vnútroprocesové volanie COM rozhraní na skutočnom skrytom okne), **SIM** = offscreen
kreslenie skutočných stránok, **LIVE** = skutočný čítač obrazovky / skutočné klávesy. **V tomto míľniku sa nespustil Narrator, Inspect, povýšená aplikácia ani Explorer a nerobil
sa žiadny LIVE test.**

#### 1. Zmeny a dôkazy
| Oblasť | Zmena | Commit | Dôkaz |
|---|---|---|---|
| Klávesnica: presun položiek rozloženia | Tab na položku → Enter ju chytí → šípky/Home/End ju presúvajú medzi skupinami a v poradí (živý náhľad, bez uloženia) → Enter pustí a uloží (len ak sa zmenilo), Esc vráti presne pôvodné rozloženie, Tab/iný ovládač potvrdí, zatvorenie okna zruší. Auto-opakovanie nemôže chytiť/pustiť. Položka ostane vybraná a celá viditeľná. **Oprava chyby aj pre myš:** `apply_drop` bral index medzi zobrazenými položkami ako index medzi všetkými položkami skupiny, takže pri neznámej položke z budúcej verzie mohlo byť poradie zlé | `c68617b` | **TEST** čisté (cieľ presunu, okraje, skryté a neznáme položky, `MoveSession`), **SIM** na kreslenej stránke (zobrazenie po presune, fokus ostáva), uloženie/zrušenie s dočasným `TRANSOM_CONFIG` |
| Klávesnica: nepriehľadnosť | Posuvník je v kruhu fokusu: ←/→ 5 % (Shift 1 %), PgUp/PgDn 10 %, Home/End = minimum/100 %; rovnaký rozsah ako tlačidlá +/-; uloží sa po pauze (debounce), pri zatvorení sa vypláchne | `c68617b` | **TEST** `step_opacity`, smerovanie kláves, uloženie po `flush_pending` |
| Esc v poli farby | Esc vráti hodnotu spred úpravy (všetky štyri polia sa obnovia) a **nič neuloží**. **Koreň:** Esc pole opustil a strata fokusu uložila rozpísanú hodnotu, navyše autosave po 450 ms ukladal aj polovičné hodnoty (`#123`, `2`). Teraz sa počas písania neukladá (náhľad je živý), ukladá sa pri Enter/Tab/strate fokusu; Enter neplatnú hodnotu neprijme | `c68617b` | **TEST** `EditSession`, `edit_cancel`/`edit_blur`: po Esc súbor nevznikne; po potvrdení sa uloží |
| MSAA: klientska plocha okna nastavení | Nový druh objektu `Page` (`OBJID_CLIENT`): koreň = PANE s názvom stránky; ovládače majú **rolu** (PAGETAB, CHECKBUTTON, RADIOBUTTON, SLIDER, PUSHBUTTON, LISTITEM, TEXT), **názov** zachytený pri kreslení (voľby nesú názov skupiny, tlačidlá +/− a farby majú zrozumiteľné názvy), **stav** (CHECKED, SELECTED, OFFSCREEN, UNAVAILABLE), **hodnotu** (posuvník „86 %“, text polí), **polohu**, **predvolenú akciu** (Press/Check/Uncheck/Select), `accFocus` podľa virtuálneho fokusu a udalosti `EVENT_OBJECT_FOCUS`/`REORDER` (aj pre tlačidlá hlavičky) | `6753ced`, `d771bf5` | **TEST** vnútroprocesové volania `IAccessible` na skutočnom skrytom okne (názov, rola, stav, hodnota, akcia, poloha, neplatné ID, `accDoDefaultAction` → správa oknu); **SIM** 7 stránok × En/De/Ja × 96/192 dpi: každý ovládač má neprázdny, nesurový, v rámci stránky jedinečný názov, rolu zo `role_of` (vyčerpávajúci `match`), miesto v klientskej ploche, práve jednu vybranú kartu a najviac jednu zvolenú voľbu v skupine |
| Lišta (GDI) | Batéria už nie je neviditeľná: región „Batéria: …“ + tooltip; `accValue` pre hodiny (čas), kruh AI (percento) a text skladby/batérie | `1613793` | **TEST** regióny z reálnych položiek |
| i18n | `layout.kbd_help`, `layout.move_help`, `acc.*` (8 kľúčov) vo všetkých 11 jazykoch | `c68617b`, `d771bf5` | **TEST** úplnosť jazykov, audit vzhľadu stránky Rozloženie (bez orezania) |

UIA: nepísal sa vlastný UIA provider. Očakáva sa, že MSAA→UIA proxy (oleacc) premietne roly na ControlType, `accName` na Name, `accValue` na ValuePattern a
`accDoDefaultAction` na LegacyIAccessiblePattern. **Toto je odhad, nie overenie** (§3).

#### 2. Čo ostáva otvorené po tomto míľniku
- **LIVE (vyžaduje čítač obrazovky / skutočné klávesy):** Narrator/Inspect nad oknom nastavení (strom, poradie čítania, `accDoDefaultAction`, udalosti fokusu pod `OBJID_CLIENT`/`OBJID_TITLEBAR`, ktoré natívne
  polia EDIT ostanú v strome pri vlastnom koreni — preto sa polia vystavujú aj ako uzly `Page`), skutočné stlačenia kláves (Tab/Enter/Esc/šípky/Shift, auto-opakovanie), IME pri Esc v poli, správanie pri 150/200 % DPI.
- **Nezavedené:** `accSelect(SELFLAG_TAKEFOCUS)` (UIA `SetFocus` na uzol stránky), `NotifyWinEvent` pri zmene názvu/stavu položiek **lišty** (zmena skladby, stavu prehrávania), úloha `ALERT` pre prechodný oznam lišty,
  vystavenie statických textov stránok (nadpisy, odseky), karty hodnôt a tlačidlá v popoveri AI majú len rolu tlačidla (bez stavu SELECTED/CHECKED), `ArchivePause` pri vypnutom archíve nie je v strome.
- **Posuvník myšou** nemá alternatívu na dotykovej obrazovke mimo +/- (nerieši sa).

#### 3. Zvyšné riziká z §8.14 (audit agenta 3; **neimplementované**, zatriedené)
**A – malé, vysoká istota, bez regresie (ďalší krok):** `clean_model` (odstrániť riadiace znaky, skrátiť na 64 **znakov**, nikdy netrimovať ani nemeniť veľkosť písmen ani `[1m]` — model je kľúč archívu a cien); orezanie hodnôt po načítaní
archívu/cache (`MAX_ROW_TOKENS = 1<<48`, `saturating_add`); `launch_target(app)` pre `shell:AppsFolder\` (odmietnuť prázdne, > 256 znakov, riadiace znaky, `: / < > " | ? * %`, `..`, začiatok `\`; povoliť `!`, `.`, `_`, `{}`, medzeru; testované na `Brave`,
`Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic`); ohraničené čakanie v `Drop` média (`recv_timeout` 2 s); „Delete local data“ maže aj osirelé `usage-cache.json.*.tmp` a `usage-archive.json.*.tmp` (nie `claude-setup.json` ani zálohy, tie
slúžia na návrat); `claude_setup` zapisuje cez `write_atomic` (dnes pevný `settings.json.transom-tmp` bez fsync; **hotové v §8.34**); `connect`/`rollback` odmietnu relatívnu cestu; `ci.yml`: `permissions: contents: read`, `timeout-minutes`, `--locked`, samostatná úloha `cargo-deny` na Linuxe
(Docker akcia na `windows-latest` zlyhá).
**B – dokumentuj / neskôr:** kompaktovanie mostu `claude-statusline.jsonl` (rotácia + denné súčty cez `.old`, test ekvivalencie `read_file(surový) == read_file(kompaktovaný + chvost)`); cache transkriptov podľa (veľkosť, mtime) so zachovaním globálnej deduplikácie;
strop počtu modelov; timeout WinRT operácií médií (potrebuje LIVE); mazanie záloh `settings.json` po návrate (zmena dokumentovaného správania); DACL mutexu; SHA piny akcií.

#### 4. Postup na neskorší LIVE test reštartu Explorera (vyžaduje tvoj výslovný súhlas; teraz sa nerobí)
Podrobne v §8.15 bod 3A; stručne a presne:
1. **Príprava (nič sa nerestartuje):** ulož prácu vo všetkých aplikáciách; zatvor okná Prieskumníka; vytvor priečinok `C:\temp`. Spusti transom z terminálu: `transom-app.exe --log C:\temp\transom-live.log` (aktuálny build: po `cargo build --release`). V Nastaveniach → Vzhľad nech je „ikona v oblasti oznámení“ zapnutá.
2. **Zaznamenaj stav pred:** `tasklist /fi "imagename eq transom-app.exe"` (PID transom), `tasklist /fi "imagename eq explorer.exe"` (PID Explorera), počet ikon transom v oblasti oznámení vrátane skrytých (očakávaný 1), posledných ~20 riadkov logu.
3. **Reštart (vykoná používateľ):** Správca úloh (Ctrl+Shift+Esc) → karta Procesy → *Windows Explorer* → pravé tlačidlo → *Reštartovať*. Nepoužívaj `taskkill` na transom. Počkaj ~10 s, kým sa panel úloh a oblasť oznámení nanovo zobrazia.
4. **Očakávané (úspech):** PID transom je nezmenený, PID Explorera je nový; v oblasti oznámení je **presne jedna** ikona transom (nie dve, nie „duch“, ktorý zmizne až po prejdení myšou); kliknutie na ňu otvorí Nastavenia (pri skrytej lište vráti lištu);
   lišta je znova rezervovaná (maximalizované okno začína pod ňou); v logu je riadok `TaskbarCreated: ikona v oblasti oznameni pridana=true`.
5. **Varianty (každú ≥ 3×):** (a) bežná viditeľná lišta; (b) **skrytá lišta** (pravý klik → Skryť lištu; ikona je jediná cesta späť; po reštarte musí vzniknúť); (c) **dva monitory** (kontrola, že sa ikona nepridá dvakrát); (d) reštart hneď po štarte transom.
6. **Neúspech:** ikona chýba / sú dve / „duch“ / `pridana=false` bez ďalšieho pokusu / lišta neprepočítala polohu. Návrat: v Nastaveniach vypni a zapni ikonu, poslednou možnosťou je Ukončiť transom z menu a spustiť znova. Zaznamenaj log (`C:\temp\transom-live.log`) a výsledok do §8.15.
7. **Riziko pre používateľa:** dočasne zmizne panel úloh, plocha a oznamovacia oblasť; zatvoria sa okná Prieskumníka; ikony iných programov, ktoré nepodporujú `TaskbarCreated`, sa nemusia vrátiť (treba ich reštartovať). Nerestartuj počas ukladania, inštalácie alebo prehrávania, ktoré chceš zachovať.

#### 5. Overenie tohto míľnika
- `cargo fmt --check` čistý; `cargo clippy --workspace --all-targets -D warnings` bez nálezov; `cargo test --workspace` **3× za sebou rovnako: 430 testov prešlo, 0 zlyhalo, 3 ignorované** (app 83, config 27, core 67, i18n 3, media 3, platform 54, render 50, settings 50, usage 93; testy s predošlým občasným správaním, napr. `disabled_sources_read_nothing`, prešli vo všetkých troch behoch); `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok; release build do oddeleného `CARGO_TARGET_DIR` OK (`target\release` v projekte je **starší**, lebo ho môže používať bežiaca lišta).
- **Neovplyvnené:** čas, časové pásmo, usporiadanie displejov, DPI, jazyk systému, poverenia, nastavenia poskytovateľov ani reálne dáta o využití AI. Nespustil sa Narrator, Inspect, povýšená aplikácia ani reštart Explorera; testy bežali v dočasných adresároch a v procese testu sa vytvárali len skryté okná.
- **Git:** vetva `hardening/tray-integrity-a11y`, commity `c68617b`, `6753ced`, `d771bf5`, `1613793` (+ tento dokument); pracovný strom čistý.


### 8.17 Rozšírenie: profily monitorov, prenosné nastavenia, zvuk a rýchle akcie, Polica, schránka, snímky – plán a 1. inkrement (2026-11-21)

**Vetva** `feature/profiles-audio-shelf` (z `hardening/tray-integrity-a11y`; lokálna, bez remote). Postup: štyria agenti na výskum a návrh (zvuk, Polica a súkromie,
profily a konfigurácia, UX), potom integrácia po malých inkrementoch. Označenia: **[DOC]** = overené v dokumentácii Microsoftu (agent; vetu o rolách zariadení overil
aj hlavný agent), **[CRATE]** = overené v zdrojovom kóde `windows` 0.62.2, **[ODHAD]** = treba spike/LIVE test.

#### 1. Schopnosti API (zhrnutie)
| Schopnosť | API | Stav | Poznámka |
|---|---|---|---|
| Zoznam výstupov, názov, predvolený | `IMMDeviceEnumerator::EnumAudioEndpoints`, `PKEY_Device_FriendlyName`, `GetDefaultAudioEndpoint` | [DOC] | ID zariadenia je nepriehľadné |
| Udalosti zariadení bez pollingu | `IMMNotificationClient` | [DOC] | callback nesmie blokovať ani odregistrovať; registrácia nerobí AddRef |
| Hlasitosť a stlmenie + udalosti | `IAudioEndpointVolume` + `IAudioEndpointVolumeCallback` (`guidEventContext`) | [DOC] | vlastná zmena sa rozlíši GUID; `OnNotify` nenesie ID zariadenia |
| **Zmena predvoleného výstupu** | **žiadne verejné API** (pozn. §8.20: citát z *Device Roles* sa nepodarilo nájsť, tvrdenie je [ODHAD]) | [DOC] | Microsoft Learn, *Device Roles*: „Applications cannot change the roles that are assigned to audio endpoint devices. The operating system allows only the user to assign device roles.“ `IPolicyConfig` (nedokumentované) sa **nepoužije**. |
| Systémový výber výstupu | `ms-settings:sound` / `sound-devices` (URI [DOC]); Win+Ctrl+V (používateľská skratka) | čiastočne | spustenie URI cez `ShellExecute` z nebaleného procesu [ODHAD]; zmena sa potvrdí len udalosťou `OnDefaultDeviceChanged` |
| Rýchle akcie | Win+S, Win+B (existujú), Win+N (oznámenia), Win+A (rýchle nastavenia) | používateľské skratky | úspech sa vždy overiť nedá; okenné triedy flyoutov [ODHAD] – nehlásiť „otvorené“ |
| Drop do lišty | `OleInitialize` + `RegisterDragDrop` / `IDropTarget` | [DOC] | drop na okno `WS_EX_NOACTIVATE`/vrstvené [ODHAD → LIVE]; z povýšeného zdroja nefunguje (UIPI) |
| Ťahanie von | `SHCreateDataObject` + `SHDoDragDrop` | [DOC] | len COPY/LINK, nikdy MOVE; modálna slučka → cez frontu požiadaviek |
| História schránky | `AddClipboardFormatListener` / `WM_CLIPBOARDUPDATE`; formáty `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory`, `CanUploadToCloudClipboard` | [DOC] | „Clipboard Viewer Ignore“ = konvencia správcov hesiel (rešpektovať); Win+V história sa **neimportuje** |
| Snímka | `GraphicsCapturePicker` + `IInitializeWithWindow`, jeden snímok (`Direct3D11CaptureFramePool::CreateFreeThreaded`) | [DOC] | žltý rámik ostáva; `ms-screenclip:` je zastarané a nebalená app výsledok nedostane |
| Profily, export/import | vlastný formát `transom-settings-export` v1, `save_merged` | – | téma ani výška lišty dnes nie sú v konfigurácii (výška = `--height`), do profilu zatiaľ nepatria |

#### 2. Predvolené nastavenia súkromia (záväzné pre ďalšie inkrementy)
- **História schránky:** predvolene **vypnutá**, migrácia ju nikdy nezapne; zachytávajú sa len nové položky po zapnutí; pozastavenie, vylúčené aplikácie (best-effort), text/obrázky podľa voľby,
  limity, retencia, pripnutie, vymazanie; viditeľný indikátor v lište počas zachytávania; obsah sa nikdy nelogujе, nesynchronizuje ani neposiela; oddelené od nastavení a archívu AI.
- **Polica:** len lokálne (`%LOCALAPPDATA%\transom\tools\shelf\`; od §8.22 spolu s ostatnými úložiskami nástrojov); predvolene **odkaz**, kópia obsahu len na výslovnú voľbu s limitom; nič sa automaticky neotvára, nespúšťa ani nenáhľaduje (náhľad len bitmapy cez WIC s limitom rozmerov);
  nedostupný odkaz sa zobrazí, nemaže sa; limity počtu/veľkosti, retencia, pripnutie, „Vymazať policu“ dvojkrokovo.
- **Snímka:** len po výslovnej akcii cez systémový výber; do Police až po potvrdení; žiadne zachytávanie na pozadí.
- **Export nastavení:** len biela listina (§3); nikdy poverenia, dáta AI, archív, schránka, Polica, snímky, cache ani cesty.

#### 3. 1. inkrement – jadro exportu/importu nastavení (`transom-config::transfer`, commit `3e0f3b9`)
- **Formát** `transom-settings-export`, `format_version` 1, hlavička (`app_version`, `exported_at`), sekcie `layout` (len známe položky), `clock`, `media` (len posuvný text), `appearance`, `language`, `usage`
  (len interval obnovy, poskytovateľ a režim kruhu).
- **Nikdy:** zapnutie zdrojov AI (na inom stroji by spustilo čítanie lokálnych prepisov), archív, ručné dátumy fakturácie, výber monitorov (ID sú viazané na stroj), preferovaná aplikácia médií, neznáme kľúče, cesty.
- **Import:** limit 256 KiB, BOM, JSON objekt, presný formát, novšia verzia formátu sa odmietne; každá sekcia ide cez rovnaké tolerantné čítanie ako `config.json` (limity platia), neplatná sekcia sa preskočí,
  neznáme sekcie/polia/položky sa vypíšu ako ignorované. **Náhľad** po poliach; zhodný súbor = nič na import. **Aplikovanie** len zvolených platných sekcií a povolených polí; nepriehľadnosť a farba sa uložia už obmedzené;
  neznáme položky aktuálneho rozloženia (z novšej verzie) sa zachovajú. Zápis cez `save_merged` (zámok, trojcestné zlúčenie, atomický zápis; poškodený/novší súbor sa neprepíše).
- **Testy (TEST, 8):** biela listina (zakázané reťazce vrátane spätnej lomky v exporte chýbajú), round-trip všetkých sekcií, odmietnutia (poškodený, cudzí, novší, bez hlavičky, príliš veľký), podvrhnutý súbor nezapne zdroje AI ani archív
  a neobíde limity, izolácia neplatnej sekcie, presnosť náhľadu a výber sekcií, zachovanie neznámych položiek, súbežná zmena prežije `save_merged` a read-only súbor ostane nezmenený.
- **Obmedzenie:** z UI to zatiaľ **nie je dostupné** (žiadne tlačidlo ani dialóg súboru); profily v exporte zatiaľ nie sú (sekcia `profiles` by sa pri importe vypísala ako ignorovaná).

#### 4. Ďalšie inkrementy (poradie)
1. ~~**UI Export/Import** (nová stránka nastavení na konci `PAGE_ORDER`)~~ **hotové v §8.18** (pôvodný návrh:: `GetSaveFileNameW`/`GetOpenFileNameW` mimo výpožičky stavu, veľkosť pred čítaním, náhľad s výberom sekcií, dvojkrokové potvrdenie,)
   záloha `config.json` do `%APPDATA%\transom\backups\` (posledných 5; zahrnúť do „Delete local data“ podľa F10), prístupnosť (`tag`, `role_of`).
2. **Konfigurácia v8 + profily naraz** (verzia sa zvýši raz): `profiles`, `assignments` (ID monitora → profil), `default_profile`; predvolený profil = dnešné top-level hodnoty (staršia binárka v8 číta len na čítanie, ale správne);
   rozlíšenie len podľa ID monitora, na ktorom lišta naozaj je (fallback lišta nikdy nezdedí profil chýbajúceho monitora), žiadne tiché prepnutie, priradenie nie na `\\.\DISPLAYn`; per-monitor `items`/vzhľad v aplikácii;
   oprava `normalize` (nová položka sa vloží na predvolené miesto, nie za hodiny); export doplní `profiles` a `assignments_hint` (len popisy monitorov).
3. **Prístupnosť popoverov** (role/stavy/hodnoty pre panel, kruh fokusu pre Tab vrátane panela AI) – predpoklad pre zvuk a Policu.
4. **Zvuk** (nový crate, vlastné MTA vlákno ako médiá): zoznam výstupov **len na čítanie**, hlasitosť/stlmenie s udalosťami, tlačidlo „Zmeniť výstup v systéme“ (`ms-settings:sound` / Win+Ctrl+V) s potvrdením až udalosťou;
   čistá logika (GUID vlastnej zmeny, debounce, zlúčenie 3 rolí) s falošným backendom. **Rýchle akcie** (Win+N, Win+A) s poctivým textom.
5. **Polica bez ťahania** (model, index, limity, validácia ciest, deduplikácia, retencia v čistom crate), potom **drop** (OLE; LIVE test `WS_EX_NOACTIVATE`), **ťahanie von**, **kópie súborov** (výslovná voľba).
6. **Snímka** cez `GraphicsCapturePicker` → náhľad → uložiť do Police po potvrdení.
7. **História schránky** (opt-in) s indikátorom a filtrami formátov.

#### 5. Overenie tohto kroku
- `cargo fmt --check` čistý; `cargo clippy --workspace --all-targets -D warnings` bez nálezov; `cargo test --workspace` **3× za sebou: 438 testov prešlo, 0 zlyhalo, 3 ignorované** (app 83, config 35, core 67, i18n 3, media 3, platform 54, render 50, settings 50, usage 93); `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok; release build do oddeleného `CARGO_TARGET_DIR` OK (`target\release` v projekte je starší).
- **LIVE:** nič (tento krok nemá UI ani zásah do systému). Neoverené ostáva všetko označené [ODHAD] v tabuľke §1.
- **Nemenené:** čas, časové pásmo, displeje, DPI, jazyk systému, poverenia, nastavenia poskytovateľov, reálne dáta AI; testy bežali v dočasných adresároch.


### 8.18 Míľnik: Export a import nastavení v okne Nastavení (2026-11-21)

**Vetva** `feature/profiles-audio-shelf` (lokálna, bez remote). Postup: traja agenti na čítanie (rozloženie a lokalizácia, súborové dialógy a bezpečnosť konfigurácie, klávesnica a prístupnosť),
potom implementácia po malých commitoch. Označenia: **TEST** = automatizovaný test, **SIM** = offscreen kreslenie skutočných stránok, **LIVE** = skutočné okno, dialógy a čítač obrazovky.

#### 1. Čo používateľ vidí
Nová stránka **Export a import** (posledná v navigácii, nová ikona):
- **Exportovať nastavenia…** → natívny dialóg Uložiť (`GetSaveFileNameW`, navrhnutý názov `transom-settings-RRRR-MM-DD.json`, potvrdenie prepisu). Zapíše presne `transfer::export` (biela listina z §8.17) atomicky; nevytvára priečinky; odmietne
  súbory samotného transom (konfigurácia, zámok, zálohy, lokálne dáta). Export nemení konfiguráciu.
- **Importovať nastavenia…** → natívny dialóg Otvoriť (`GetOpenFileNameW`) → overenie súboru → **náhľad**: hlavička (verzia aplikácie, dátum exportu), sekcie so zaškrtávacími políčkami a zoznamom zmien v čitateľnej podobe
  (`Opacity: 100 → 60`, rozloženie po skupinách), sekcie bez zmien (sivé), „Nie je v súbore“, blok **Ignorované** (zakázané a neznáme polia, neplatné sekcie) a hlásenie, ak nie je čo importovať.
- **Importovať vybrané** je **dvojkrokové** (druhý klik potvrdí); **Zrušiť**, Esc alebo odchod zo stránky náhľad zahodí a **nezmení nič**. Po úspechu/chybe je v stavovom riadku hlásenie (dlhé sa zalomia na dva riadky).
- Pri konfigurácii len na čítanie (poškodená / novšia verzia) sú export aj import vypnuté s vysvetlením (inak by export z predvolených hodnôt klamal).

#### 2. Pravidlá bezpečnosti (TEST)
- **Biela listina a súkromie** (z jadra): nikdy zapnutie zdrojov AI, archív, ručné dátumy, výber monitorov / ID, preferovaná aplikácia médií, neznáme kľúče, cesty, poverenia, dáta o využití, schránka, polica. Import podvrhnutého súboru
  **nezapne** zdroje AI ani archív (test). Hodnoty idú cez rovnaké tolerantné čítanie ako `config.json` (interval obnovy ≥ 5 min, nepriehľadnosť ≥ 10 %), farba a nepriehľadnosť sa ukladajú normalizované.
- **Nálezy revízie opravené v jadre:** `language` musí byť známy kód (predtým ľubovoľný reťazec do 256 KiB), `marquee_direction`/`speed` sa normalizujú na známe kódy, text z náhľadu nemá riadiace, obojsmerné ani nulovej šírky znaky.
- **Čítanie súboru:** `File::open` + metadáta na otvorenom súbore, čítanie cez `take(256 KiB + 1)` (klamlivá veľkosť ani rastúci súbor nepreskočí limit), len bežný súbor (adresár = „nie je bežný súbor“, zariadenia a `\\?\` cesty sa odmietnu),
  UTF-16 sa odmietne zrozumiteľnou chybou, BOM UTF-8 je v poriadku. Cesta sa v hláseniach zobrazuje len ako názov súboru.
- **Zápis:** `write_atomic` (unikátny dočasný súbor + fsync + premenovanie) na export aj konfiguráciu; pred importom **záloha** `config.json` do `%APPDATA%\transom\backups\config-RRRRMMDD-HHMMSS.json` (surové bajty, posledných 5,
  cudzie súbory sa nemažú); zlyhanie zálohy import zruší. Potom `save_merged` (zámok, trojcestné zlúčenie so súbežnými zmenami, poškodený/novší súbor sa neprepíše).
- **Aktuálnosť náhľadu:** druhý krok znova načíta `config.json`, porovná zmeny s tými, ktoré používateľ videl; ak sa líšia, nezapíše nič a vyžiada si nové potvrdenie. Ak je súbor medzitým poškodený/novší, import sa zruší bez zápisu.
- **Zálohy a súkromie:** zálohy `config.json` obsahujú aj to, čo export vynecháva (prepínače zdrojov AI, nastavenia archívu, ručné dátumy, ID monitorov). **„Delete local data“ ich nemaže** (je to iný priečinok, `%APPDATA%`; rovnaká
  logika ako pri samotnom `config.json`). Stránka to vysvetľuje; zahrnutie do mazania ostáva otvorené rozhodnutie (§8.14 F10).

#### 3. Prístupnosť a klávesnica
- Nové `Hit` varianty majú roly: sekcia = CHECKBUTTON (názov „sekcia – Zmeny: N“, stav CHECKED), Exportovať/Importovať/Importovať vybrané/Zrušiť = PUSHBUTTON. Náhľad nie je modálny, koreň ostáva PANE.
- **Statické texty** (nadpis náhľadu, riadky zmien, ignorované, stavový riadok) sú nové uzly STATICTEXT: len na čítanie (`STATE_READONLY`, bez fokusu a bez akcie), najviac 60, text očistený a skrátený (120 znakov).
  Zatiaľ sa pridávajú **na koniec** zoznamu uzlov (poradie čítania v prehliadacom režime čítača nemusí zodpovedať vizuálnemu; plán B: vložiť podľa polohy).
- Kruh fokusu: Exportovať → Importovať → sekcie → Importovať vybrané → Zrušiť → hlavička. **Esc** pri otvorenom náhľade ho zatvorí a fokus vráti na „Importovať“; po otvorení náhľadu ide fokus na prvú zmenenú sekciu; po dialógu sa
  okno znova aktivuje a fokus ostáva na tlačidle, ktoré dialóg spustilo. **Držanie Enter/Medzerníka** už nepreskočí druhý krok potvrdenia (platí aj pre „Delete archive“).
- Lokalizácia: 45 + 3 reťazcov vo všetkých 11 jazykoch (test úplnosti a zhody `{n}`), popisky polí a hodnôt používajú existujúce kľúče (test, že žiaden nie je surový).

#### 4. Overenie (TEST / SIM; LIVE nič)
- UI toky na skutočnom stave `Ui` (klikanie cez `click`, návrat z dialógu cez `finish_dialog`, izolované dočasné `TRANSOM_CONFIG`/`TRANSOM_DATA`), 14 testov: export len z bielej listiny a bez ciest; odmietnutie vlastných súborov a chýbajúcich priečinkov;
  dvojkrokový import so zálohou (bajty zálohy = pôvodné), uloženie, jazyk sa uplatní, zdroje AI ostanú vypnuté; zrušenie a odchod zo stránky; tabuľka zlých súborov (poškodený, JSON pole, prázdny, cudzí, `config.json`, novšia verzia, binárny, UTF-16,
  príliš veľký, rovnaký obsah, adresár); čiastočne neplatný súbor; súbežná zmena prežije; zmenený náhľad vyžaduje nové potvrdenie; read-only a poškodenie medzi náhľadom a potvrdením; zlyhaná záloha; Esc/auto-opakovanie; popisky a hodnoty;
  pomocníci ciest, veľkostí a kódovania; stavba dialógu (flagy, buffer 32 768, filter s dvojitou nulou) **bez zobrazenia dialógu**.
- Prístupnosť a klávesnica nad náhľadom, platformový test statického textu; **audit vzhľadu** novej stránky vo všetkých 11 jazykoch × 96/144/192 DPI × obe témy × 4 stavy (prázdna, náhľad, po prvom kliku, read-only; vrátane dlhého názvu súboru, dlhých
  ignorovaných položiek a dlhého chybového hlásenia): 0 nálezov. Audit našiel orezanie dlhých hlásení v stavovom riadku (platilo pre všetky stránky) → opravené zalomením na dva riadky.
- `cargo fmt --check` čistý; `cargo clippy --workspace --all-targets -D warnings` bez nálezov; `cargo test --workspace` **3× za sebou rovnako: 459 testov prešlo, 0 zlyhalo, 3 ignorované** (app 83, config 39, core 67, i18n 3, media 3, platform 55, render 50, settings 66, usage 93); `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok; release build do oddeleného `CARGO_TARGET_DIR` OK (`target\release` v projekte je starší).
- **Neovplyvnené:** čas, časové pásmo, displeje, DPI, jazyk systému, poverenia, nastavenia poskytovateľov, reálna konfigurácia a dáta AI používateľa (testy používajú dočasné adresáre; natívne dialógy sa neotvárali).
- **Git:** commity `6c7cdf7` (stránka Export a import), `d9ac8ff` (zalomenie dlhých hlásení), jadro `01b5efe`; pracovný strom čistý.


#### 5. Čo ostáva overiť naživo (LIVE) a obmedzenia
- **Natívne dialógy** Uložiť/Otvoriť sa neautomatizovali: skutočné správanie `lpstrDefExt = "json"` (docs hovoria o 3 znakoch), `OFN_NOREADONLYRETURN`, zrušenie, dlhé cesty, UNC, zatvorenie okna počas dialógu a návrat fokusu.
  Poistka: názov bez prípony export odmietne.
- **Čítač obrazovky** (Narrator): poradie statických uzlov, čítanie názvov sekcií a stavov, oznámenie výsledku (stavový riadok je uzol, ale **bez udalosti ALERT**).
- Vysoký kontrast Windows a minimálna šírka okna (audit beží pri 824 lp klienta) sa overujú ručne.
- **Nie je v tomto kroku:** profily v exporte (sekcia `profiles` sa pri importe vypíše ako ignorovaná), zvuk, polica, schránka, snímky (pozri §8.17 plán).

### 8.19 Míľnik: Zvuky rozhrania (2026-11-21)

**Vetva** `feature/profiles-audio-shelf` (lokálna, bez remote). Nepovinný, tichý a krátky zvuk po úmyselnom a úspešnom kliknutí na ovládač lišty, položku menu alebo ovládač popoveru.
Označenia: **TEST** = automatizovaný test (falošné zariadenie), **SIM** = offscreen kreslenie skutočných stránok, **LIVE-API** = skutočné zvukové zariadenie cez WASAPI (kontrola relácií), **LIVE-UI** = skutočné okno a myš (nerobilo sa).

#### 1. Čo používateľ dostane
- Nová stránka Nastavení **Zvuky rozhrania** (ikona reproduktora, medzi AI a Jazykom): zaškrtávacie políčko „Prehrávať zvuky rozhrania“ (**vypnuté = stlmenie**), posuvník hlasitosti 0–100 % (myš, tlačidlá −/+ o 5 %, klávesy šípky 5 %, Shift 1 %, Page 10 %, Home/End) a tlačidlo **Vyskúšať**.
- **Predvolene vypnuté**, predvolená hlasitosť **25 %** (po aktualizácii lišta nezačne zvučať sama). Nastavenie je v `config.json` v sekcii `ui_sounds` (`enabled`, `volume`); bez zvýšenia verzie konfigurácie (v8 ostáva rezervovaná pre profily), tolerantné čítanie (zlý typ = predvolená hodnota, hlasitosť sa orezáva na 0–100), prežije `save_merged` pri súbežných zmenách.
- Zvuk **nie je v exporte nastavení** (biela listina z §8.17 sa nezmenila; import súbor so sekciou `ui_sounds` ju vypíše ako ignorovanú).

#### 2. Kedy sa hrá (pravidlá, TEST)
Jediné miesto rozhodovania je `transom_sound::policy::should_play`: hrá len **zapnutý** zvuk s hlasitosťou > 0 pri **úmyselnom** podnete (`Pointer`, `Menu`, `Popover`, `Assistive`) a **úspešnej** akcii (`Outcome::Done`). Nikdy: hover, stlačenie bez uvoľnenia, vypnutý ovládač (akcia ani nevznikne), neúspešná alebo zablokovaná akcia (Win+S/Win+B zablokované, žiadna media session), aktualizácie na pozadí (hodiny, médiá, AI, batéria), snímky animácií (posuvný text) a **automatizácia**.
- Miesta v `transom-app`: uvoľnenie myši nad ovládačom lišty (`act()` teraz vracia výsledok), výber položky menu (myš aj Enter; **nie** „Ukončiť“, proces končí), klik na ovládač popoveru AI a kalendára, aktivácia cez MSAA (`accDoDefaultAction`).
- **Automatizácia je vždy ticho:** `--test-*`, `--sim-*`, `--secs`, `--no-tray`, `--ticker-stats`, `--no-sound` a premenná `TRANSOM_NO_SOUND=1` dajú tichý prehrávač (žiadne vlákno, zariadenie ani zvuk); `cargo test` nikdy neotvorí zvukové zariadenie (`App::new` má tichý prehrávač, skutočný sa vytvára až v `main`; testy používajú falošný backend). Všetky skripty v `tools/` bežia s `--secs`.

#### 3. Architektúra a hlasitosť (nezávislosť od systému)
- Nový crate `transom-sound`: čistá logika (`policy`), vygenerovaný zvuk (`synth`), engine (`engine`) a WASAPI (`wasapi`). Žiadna nová závislosť (len funkcie crate `windows`; `cargo deny` bez zmeny).
- **UI vlákno nikdy nečaká:** `Player::cue` kontroluje pravidlá a robí `try_send` do fronty s dĺžkou 1 (jeden zvuk hrá, jeden čaká, ďalšie sa zahodia); odstup medzi zvukmi 60 ms; pracovné vlákno `transom-sound` (MTA) otvára zariadenie lenivo až pri prvom zvuku. Zvuk čakajúci vo fronte sa pri **stlmení zahodí** (nastavenie sa čítajú atomicky tesne pred prehraním).
- **Samostatná relácia transom:** zdieľaný režim WASAPI, pevné GUID relácie, zobrazovaný názov „transom UI sounds“, vlastný formát 44,1 kHz/16 bit/stereo s `AUTOCONVERTPCM`. Hlasitosť = `ISimpleAudioVolume::SetMasterVolume(percent/100)` **len tejto relácie** (lineárne, takže posuvník v Nastaveniach zodpovedá posuvníku „transom UI sounds“ v miešadle Windows). Nastaví sa po vytvorení relácie **pred** prvým spustením prúdu a potom len pri zmene hodnoty z Nastavení; zmena v miešadle počas behu sa teda pri každom kliknutí neprepisuje (prepíše ju až ďalšia zmena posuvníka alebo nová relácia).
- **Čo sa nikdy nerobí:** `IAudioEndpointVolume` (hlavná hlasitosť a stlmenie zariadenia), nastavenie predvoleného výstupu, ovládanie hlasitosti iných aplikácií a médií. Crate to nemá ani povolené v `Cargo.toml` (funkcia `Win32_Media_Audio_Endpoints` je len v dev-závislostiach pre živý test) a test skenuje zdrojové kódy na zakázané identifikátory. Stlmenie a hlasitosť 0 = **žiadny prúd sa nespustí** (nezávisí od stavu miešadla ani nastavení zvuku Windows).
- **Nedostupné zariadenie:** chyby (`Unavailable`, zlý formát, `DEVICE_INVALIDATED`) sa vracajú ako hodnoty (vydanie má `panic = "abort"`); pri neplatnom uloženom prúde (zmena predvoleného výstupu) jeden pokus znova; po zlyhaní sa ďalšie zvuky 5 s preskakujú (bez blokovania a bez záplavy logu; loguje sa len zmena stavu), tlačidlo Vyskúšať skúša vždy. Po 4 s nečinnosti sa prúd uvoľní.
- **Zvuk (pôvod, licencia):** vygenerovaný vzorcom (dve sínusovky 1318,5 a 1976 Hz, nábeh 2,5 ms, exponenciálne dozvukovanie, dobeh 10 ms), 70 ms, vrchol −6 dBFS pri 100 % relácie. Pôvodné dielo projektu (MIT), žiadne externé súbory; spôsob generovania je v `crates/transom-sound/src/synth.rs`.
- **Ukážka v Nastaveniach:** hrá v procese okna Nastavení (vlastná relácia s rovnakým názvom a hlasitosťou ako v lište; v miešadle sa počas ukážky zobrazí ako proces `transom-settings`). Pri ťahaní posuvníka sa ukladá a ukážka prehrá **raz** po uvoľnení myši, pri klávesoch raz po oneskorenom uložení; stlmené alebo 0 % nehrá (tlačidlo vysvetlí prečo). Lišta zmenu prevezme správou o zmene konfigurácie (`reload_config` → `set_settings`).

#### 4. Overenie
- **TEST** (falošný backend, izolovaná konfigurácia): politika (všetky kombinácie podnet × výsledok), stlmenie a hlasitosť 0 bez prúdu, hlasitosť vždy v 0–1 (aj `u32::MAX`), okamžité stlmenie aj pre čakajúci zvuk, chýbajúce zariadenie (neblokuje, odstup, obnova, ukážka ho skúša), pomalé zariadenie (50 zvukov za < 100 ms, zvyšok zahodený), obmedzenie frekvencie, uvoľnenie po nečinnosti, tichý prehrávač a automatizácia, vygenerovaný zvuk (dĺžka, vrchol, nulový začiatok/koniec bez klikania, bez jednosmernej zložky), zákaz hlavnej hlasitosti v zdrojoch; konfigurácia (predvolené vypnuté, tolerantné čítanie, orezanie, `save_merged`); UI (prepínač a hlasitosť sa ukladajú a nič iné nemenia, kroky a klávesy, jedna ukážka po uložení, stlmené nehrá, súbežná zmena prežije, read-only sa nezapíše, prístupnosť: CHECKBUTTON/SLIDER „25 %“/PUSHBUTTON v 11 jazykoch, kruh fokusu, reťazce vo všetkých jazykoch); aplikácia (`reload_config` uplatní stlmenie hneď, automatizačné príznaky, nový `App` má tichý prehrávač).
- **SIM:** audit vzhľadu stránky Zvuky: 11 jazykov × 96/144/192 DPI × obe témy × 3 stavy (vypnuté, zapnuté 25 %, zapnuté 100 %): 0 nálezov.
- **LIVE-API** (`cargo run -p transom-sound --example session_check`, Windows 11 26300, predvolený výstup): hlavná hlasitosť zariadenia **0,1527 pred aj po** (len čítanie), stlmenie nezmenené; pri ukážkach 10/30/60 % mala relácia „transom UI sounds“ hlasitosť presne **0,10 / 0,30 / 0,60** (to isté vidí miešadlo, číta sa `ISimpleAudioVolume`); **relácie iných aplikácií (Spotify, Discord, Steam, Brave, Flow Launcher, systém) mali hlasitosť a stlmenie rovnaké pred aj po**; po stlmení sa nič neprehralo a nevznikla nová relácia; po 5,5 s nečinnosti bola relácia neaktívna (stav 0). Výsledok skriptu `RESULT OK`.
- `cargo fmt --check` čistý; `cargo clippy --workspace --all-targets -D warnings` bez nálezov; `cargo test --workspace` **3× za sebou rovnako: 494 testov prešlo, 0 zlyhalo, 3 ignorované** (app 86, config 41, core 67, i18n 3, media 3, platform 55, render 50, settings 77, sound 19, usage 93); `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok; release build do oddeleného `CARGO_TARGET_DIR` OK.
- **Nemenené:** čas, časové pásmo, displeje, DPI, jazyk systému, poverenia, nastavenia poskytovateľov, reálna konfigurácia a dáta AI používateľa (testy používajú dočasné adresáre a falošné zvukové zariadenie), hlavná hlasitosť a relácie iných aplikácií (len čítanie).
- **Git:** commity `ca4fb75` (jadro, crate `transom-sound`, integrácia do lišty), `c0f0f2f` (stránka Nastavení), `3bc0df5` (živý test); dokumentácia v samostatnom commite.


#### 5. Čo je overené a čo nie (poctivo)
- **Nezávislosť od hlavnej hlasitosti:** overené **len čítaním** (hlavná hlasitosť sa pri zmene hlasitosti relácie nezmenila a relácia ju neovláda: kód nemá prístup k `IAudioEndpointVolume`). **Nerobilo sa** meranie pri zmenenej hlavnej hlasitosti (hlavnú hlasitosť používateľa sme zámerne neprestavovali). Spička relácie z `IAudioMeterInformation` sa meria **pred** hlasitosťou relácie a hlavnou hlasitosťou, takže nie je dôkazom (vyšla 0,5004 pri všetkých hlasitostiach); dôkazom je hodnota `ISimpleAudioVolume` a súčet odmietnutí v politike.
- **LIVE-UI nerobilo sa:** skutočné kliknutie myšou na lištu/menu so zapnutými zvukmi, vizuálna kontrola posuvníka „transom UI sounds“ v miešadle Windows a sluchová kontrola hlasitosti/záznamu. Cesta klik → zvuk je pokrytá testami politiky a integrácie (falošný backend) a revíziou kódu; lišta počas tohto kroku nebežala (nezastavovali sme používateľovu).
- **Známe obmedzenia:** (a) „úspech“ pri Win+S/Win+B a príkaze médiu znamená úspešné **odoslanie** (neskoršie neúspešné overenie lišta zobrazí oznamom, zvuk už zaznel); spustenie Nastavení z menu sa tiež rátá po odoslaní žiadosti. (b) Ikona v oblasti oznámení a „Ukončiť“ zvuk nehrajú; okno Nastavení samotné (okrem ukážky) tiež nie. (c) Zmena v miešadle počas behu sa prepíše až ďalšou zmenou hlasitosti v Nastaveniach alebo novou reláciou (po 4 s nečinnosti). (d) Pri vypnutom zvukovom zariadení sa zvuky ticho preskakujú (log: jedna správa). (e) Zálohy `config.json` z §8.18 obsahujú aj `ui_sounds`.

### 8.20 Míľnik: Panel Zvuk a rýchle akcie (2026-11-21)

**Vetva** `feature/profiles-audio-shelf` (lokálna, bez remote). Postup: traja agenti iba na čítanie (výskum Core Audio API, revízia UI/prístupnosti popoverov, analýza okrajových prípadov), potom implementácia a overenie.
Označenia: **[DOC]** = overené v dokumentácii Microsoftu (agent), **[CRATE]** = overené v zdrojovom kóde `windows` 0.62.2, **[ODHAD]** = neoverené, **TEST** = automatizovaný test (falošný backend), **SIM** = offscreen kreslenie, **LIVE-API** = skutočné zvukové zariadenie, **LIVE-UI** = skutočná lišta, myš, klávesnica a čítač obrazovky.

#### 1. Poznámka k §8.17 (oprava overenia)
Agent pre API nenašiel citát „Applications cannot change the roles…“ na stránke *Device Roles* (zostáva nepotvrdený). Tvrdenie „Windows 11 nemá podporované verejné API na zmenu predvoleného výstupu“ preto stojí len na tom, že ho Learn nepopisuje (**[ODHAD]**). Návrh na tom nezávisí: transom **nevolá žiadne** API na zmenu predvoleného výstupu (ani `IPolicyConfig`, ani shell interné rozhrania) a zmenu nikdy netvrdí, kým ju neohlási Core Audio.

#### 2. Čo používateľ dostane
- Nová položka lišty **Zvuk** (`audio`, v Rozložení **predvolene skrytá**; reproduktor s ikonou podľa hlasitosti 0 / nízka / stredná / vysoká a **stlmené s vlastným tvarom** aj názvom, nielen farbou). Klik / MSAA aktivácia otvorí kompaktný popover 340 × 640 lp (obsah sa pri nedostatku miesta posúva kolieskom).
- Obsah panela: predvolené zariadenie na prehrávanie (role `eConsole`), **systémová hlasitosť výstupu Windows** (posuvník 0–100 %, myš / šípky 5 % / Shift 1 % / Page 10 % / Home / End / koliesko) a tlačidlo **Stlmiť**, text „Windows system output volume“ a veta, že je **oddelená od zvukov rozhrania transom**; zoznam výstupov **len na čítanie** (max 4 + „+ N ďalších“; stavy predvolené / vypnuté / odpojené; duplicitné názvy sa rozlíšia „(2)“); tlačidlá **Vybrať výstup vo Windows** (`Win+Ctrl+V`) a **Nastavenia zvuku** (`ms-settings:sound`); skupina **rýchlych akcií**.
- **Rýchle akcie** (konfigurovateľné na stránke Nastavení **Panel zvuku**, kanonické poradie, predvolene všetky): Rýchle nastavenia (`Win+A`), Oznámenia (`Win+N`), Hľadať (`Win+S`, existujúci tok s overením), Oblasť oznámení (`Win+B`, existujúci tok s overením), Nastavenia transom (spustenie okna). Pod nimi stojí „Tieto akcie otvoria povrch Windows… transom nevie potvrdiť, čo Windows urobia potom.“
- Stavy bez klamstva: zariadenie bez regulácie hlasitosti / stlmenia (ovládač nie je v kruhu fokusu a text vysvetlí prečo), žiadny aktívny výstup, zvuková služba nedostupná (po opätovnom otvorení panela sa skúsi znova), „Číta sa stav“.

#### 3. Poctivé hlásenia (čo sa tvrdí a čo nie)
| Akcia | Mechanizmus | Dokumentované? | Čo panel povie |
|---|---|---|---|
| Vybrať výstup | `SendInput` `Win+Ctrl+V` | skratka z podpory Microsoftu [DOC], nie API | „Poziadané… transom nevie potvrdiť“ |
| Nastavenia zvuku | `ShellExecuteW("ms-settings:sound")` | URI [DOC] (Launch Windows Settings) | „Poziadané…“ (návratová hodnota sa neskúma) |
| Rýchle nastavenia, Oznámenia | `SendInput` `Win+A`, `Win+N` | len klávesové skratky [DOC podpora], nie API | „Poziadané…“ |
| Hľadať, Oblasť oznámení | existujúce toky `Win+S`, `Win+B` + jednorazové overenie popredia | konvencia | pri neúspechu oznam v lište |
| transom Settings | spustenie `transom-settings.exe` | vlastné | „Poziadané…“ |
| Zmena predvoleného výstupu | **nerobí sa** | – | „Windows ohlásili nový predvolený výstup: X.“ **až po** `OnDefaultDeviceChanged` |
Zablokované skratky (zvýšené popredie – UIPI, držané modifikátory) sa nahlásia ako chyba (`blocked.*`), nikdy ako „otvorené“. Hlasitosť a stlmenie sa v modeli zmenia až **notifikáciou** Core Audio (vlastná zmena aj zmena zvonku).

#### 4. Architektúra
- Nový crate `transom-audio`: čistý model (`model`: očistené názvy bez riadiacich / obojsmerných / nulovej šírky znakov, hlasitosť vždy celé 0–100, NaN → 0, duplicity), služba (`service`: vlastné vlákno MTA, jedna fronta udalostí a príkazov, zlučovanie sérií udalostí o 80 ms ticha = **jedno** čítanie zoznamu; nie polling) a Core Audio (`coreaudio`: `IMMDeviceEnumerator`, `IMMNotificationClient` [CRATE `#[implement]`], `IAudioEndpointVolume` + `IAudioEndpointVolumeCallback` s vlastným GUID kontextu na rozpoznanie ozveny). Callbacky iba vložia malú udalosť do fronty; registrácie sa rušia na vlákne služby pred uvoľnením rozhraní, nikdy v callbacku. Chyby sú hodnoty (`panic = "abort"`).
- Zápis hlasitosti ide **len z príkazu UI**; udalosť zo systému (aj ozvena vlastného zápisu) len aktualizuje model → žiadna spätná väzba. `default_changes` rastie len pri oznámenej zmene predvoleného výstupu na **iný** (prvé načítanie nie je zmena; tri roly = jedna zmena).
- Služba beží **len keď je položka Zvuk viditeľná** a nikdy pri automatizácii (`--test-*`, `--sim-*`, `--secs`, `--no-sound`…) ani v testoch; inak nebeží ani vlákno, ani registrácia.
- Súkromie: ID zariadení sa nezobrazujú ani nelogujú (snímka ich nemá); názvy sa nelogujú; do konfigurácie ide len id položky a zoznam id rýchlych akcií; **export/import nastavení ich neobsahuje** (test: sekcie `audio_panel` a `ui_sounds` sa pri importe vypíšu ako ignorované; položka rozloženia `audio` je súčasťou exportu rozloženia, ako ostatné položky).
- Konfigurácia bez zvýšenia verzie: `audio` v `KNOWN_ITEMS` (starší súbor ho dostane skrytý), `audio_panel.quick_actions` (tolerantné čítanie: zlý typ = všetky, neznáme / opakované id sa zahodia, prázdne pole = riadok skrytý).
- Platforma: `Shortcut::{QuickSettings, Notifications, OutputPicker}`, `PopoverKey` nesie `shift` a `repeat` (držaný Enter / medzerník nepreklápa stlmenie), MSAA panel popoveru vie rolu / stav / hodnotu / akciu a názov koreňa (`set_panel_extras`; AI panel ostáva beze zmeny).

#### 5. Zvuky rozhrania vs. systémová hlasitosť
`transom-sound` a `transom-audio` sú oddelené crate-y bez závislosti (test na `Cargo.toml`); `transom-audio` nepoužíva `ISimpleAudioVolume`, `IAudioClient`, relácie ani `transom-sound` (test skenuje zdroje) a `transom-sound` naďalej nemá `IAudioEndpointVolume`. Posuvník a stlmenie v paneli **nikdy nehrajú zvuk rozhrania**; tlačidlá, ktoré otvárajú povrchy Windows, ho môžu prehrať ako ostatné kliknutia (`Origin::Popover`, výsledok `Done` = „odoslané“). **Poctivo:** hlavná hlasitosť fyzicky škáluje aj reláciu „transom UI sounds“ (tak funguje Windows); záruka je len taká, že transom nemení hlasitosť tejto relácie pri zmene systémovej a že zmena systémovej hlasitosti nikdy nespustí zvuk.

#### 6. Zistenia agentov, ktoré ovplyvnili návrh
Popover **preberá popredie** (nie je `WS_EX_NOACTIVATE`, rovnako ako AI panel a kalendár) – preto sa otvára výhradne po úmyselnom kliku alebo MSAA aktivácii a nikdy sám zo zmeny zariadenia; rýchle akcie sa posielajú, kým je popover v popredí (obnova predchádzajúceho popredia pred `Win+A` by flyout hneď zatvorila). `normalize` (§8.17 bod 2) **ostáva** nezmenený: nová položka sa pridáva na koniec zoznamu (skrytá). Hot-plug búrky sú zlúčené; strata služby sa nerieši pollingom, ale ďalším podnetom (udalosť alebo opätovné otvorenie panela).

#### 7. Overenie
- **TEST:** `transom-audio` 14 testov (zlúčenie 1000 udalostí = 1–5 čítaní, ozvena bez ďalšieho zápisu, zmena predvoleného len z ohlásenej zmeny, odpojenie počas zápisu, strata služby a obnova bez pollingu, nepodporované ovládače sa nikdy nepredstierajú, hranice 0–100 vrátane `u32::MAX`, vypnutie služby, sken zakázaných rozhraní), panel v aplikácii (stavy zariadení, popisy, hodnota a stav v MSAA, ovládače bez regulácie nie sú v kruhu, príkazy a hranice, Enter bez opakovania, myš: ťahanie, orezanie, stlačenie a uvoľnenie na tom istom tlačidle, kruh fokusu a konfigurácia rýchlych akcií, tvrdenia v 11 jazykoch, potvrdenie zmeny predvoleného výstupu len po notifikácii, ikona lišty), MSAA objekt panela na skrytom okne, konfigurácia, export/import, stránka Nastavení a jej prístupnosť v 11 jazykoch.
- **SIM:** audit vzhľadu panela (orezanie, prekrytia, kontrast) 11 jazykov × 96/144/192 DPI × 7 stavov (bežný, stlmený, 100 %, 20 dlhých zariadení, nepodporované, bez zariadenia, služba nedostupná): 0 nálezov (kontrast neaktívnych textov som musel zvýšiť: neaktívne zariadenia používajú farbu `muted`, nie `disabled`); audit stránok Panel zvuku a Rozloženie (so skrytou položkou) 11 × 3 DPI × 2 témy: 0 nálezov. Vzhľad som skontroloval aj na obrázkoch (svetlá / tmavá téma, nemčina, stlmený stav, ikona v lište).
- **LIVE-API** (skutočné zariadenie, `cargo run -p transom-audio --example volume_check -- --live`): v čase testu **nemal systém žiadny aktívny výstup** (4 koncové body: 1 odpojený, 3 vypnuté; žiadny predvolený – potvrdené aj nezávisle starším príkladom). Overené naživo: registrácia `IMMNotificationClient` prebehla, enumerácia a stavy (odpojené / vypnuté) a stav „Žiadny aktívny výstup“ vznikli správne. **Test hlasitosti a stlmenia sa preto nespustil** (príklad sa sám preskočil: SKIP); jeho obnovovacia cesta (Drop strážca) a odmietnutie spustenia pri stlmení / 0 % sú v kóde, ale **neboli vykonané**. Predvolený výstup sa nemenil, zariadenia som nezapínal.
- `cargo fmt --check` čistý; `cargo clippy --workspace --all-targets -D warnings` bez nálezov; `cargo test --workspace` **3× za sebou rovnako: 527 testov prešlo, 0 zlyhalo, 5 ignorovaných** (app 97, audio 14, config 44, core 67, i18n 3, media 3, platform 56, render 50, settings 80, sound 20, usage 93; ignorované sú pomocné testy na PNG a 3 staršie); `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok; release build len ako kontrola do oddeleného `CARGO_TARGET_DIR` (**nič sa nebalilo ani nepublikovalo**) OK.
- **Nemenené:** čas, časové pásmo, rozloženie displejov, DPI, jazyk systému, účty, nastavenia poskytovateľov, predvolený výstup zvuku, hlasitosť a stlmenie systému (naživo sa len čítalo / test sa preskočil), reálna konfigurácia a dáta používateľa (testy používajú dočasné adresáre).
- **Git:** commity `605bf06` (crate `transom-audio`), `07437a8` (panel, položka lišty, rýchle akcie, stránka Nastavení); dokumentácia v samostatnom commite.


#### 8. Čo ostáva overiť naživo a známe obmedzenia
- **LIVE-UI nerobilo sa:** skutočný klik na ikonu lišty a ovládače popoveru, klávesnica (Tab / šípky / Enter), správanie popredia pri otvorení a po `Win+A` / `Win+N` / `Win+Ctrl+V`, flyouty Windows 11 (či `Win+Ctrl+V` skutočne ukáže výber výstupu na tejto zostave), čítač obrazovky (Narrator, NVDA – ako UIA proxy čítajú MSAA hodnotu „40 %“ a stavy [ODHAD]). **Fokus:** popover preberá popredie ako AI panel (zámerne, kvôli klávesnici); posuvník, stlmenie a zoznam žiadne okno nespúšťajú ani nepresúvajú fokus; tlačidlá a rýchle akcie ho prenechajú povrchu Windows, na ktorý klikol používateľ. Nič z toho som naživo nemeral.
- **Notifikácie hlasitosti** (`IAudioEndpointVolumeCallback`) a udalosti zariadení pri zmene predvoleného výstupu sa naživo **neoverili** (nebol aktívny výstup a predvolený výstup sa nemení); sú pokryté falošným backendom a kontrolou kódu. Zastavenie / reštart `Audiosrv` sa **neskúšalo** (službu nezastavujeme).
- Klávesnicou sa panel nedá otvoriť bez čítača obrazovky (lišta nie je fokusovateľná; ponuka ikony v oblasti oznámení položku Zvuk zatiaľ nemá).
- Zobrazuje sa len predvolený výstup pre rolu `eConsole`; rozchod rolí Multimedia / Communications sa neukazuje. Zoznam výstupov je len na čítanie (najviac 4 riadky).
- Výška popoveru sa pri 150–200 % DPI orezáva na pracovnú plochu (obsah sa posúva kolieskom; posun fokusom klávesnice je len základný).
- Súčasné obmedzenie z §8.17 ostáva: `normalize` pridáva nové položky na koniec (nová položka `audio` je skrytá, takže to nevadí, kým ju používateľ nezobrazí a nepresunie v Rozložení).

### 8.21 Míľnik: Príprava na verejný open-source repozitár a úspora disku (2026-11-21)

**Vetva** `feature/profiles-audio-shelf` (obsahuje `main` aj `hardening/tray-integrity-a11y`; lokálna, bez remote). **Nič sa nepublikovalo**: žiadny remote, push, tag, release ani binárka; história sa neprepisovala.
Postup: štyria agenti na čítanie (disk, Git/súkromie, open-source pripravenosť, bezpečnosť), potom malé lokálne commity.

#### 1. Zistenia
- **Súkromie a história (44 commitov, 3 vetvy):** žiadne tajomstvá, tokeny, poverenia, e-maily ani používateľské meno; autor je zástupná identita. Nízke položky: trailer `Claude-Session: <url>` a `Co-Authored-By` v každej commit správe, absolútna cesta projektu v dokumentoch a skriptoch, testovací fixture s menom podobným reťazcom (opravený v pracovnom strome, v histórii ostáva), odkazy na neverejné dáta v `spikes/s5-media/RESULTS.md`. **Prepis histórie nie je nutný**; voľba (ponechať / vyčistený klon cez `git filter-repo`) je na vlastníkovi (`docs/PUBLISHING.md`). Neskenovalo sa: obsah binárnych súborov a ignorovaných súborov na disku, voľný text `docs/PLAN.md` len cez vzory.
- **Bezpečnosť:** žiadny nález vysokej závažnosti; žiadny sieťový kód. Stredné: čítanie `*.jsonl` logov celých bez limitu a neobmedzený rast `claude-statusline.jsonl` (výkon / pamäť), zápis `~/.claude/settings.json` s pevným názvom dočasného súboru bez kontroly súbežnej zmeny a symlinkov. Nízke: predvídateľné názvy dočasných súborov, `--log` bez limitu, spúšťanie AUMID cez Shell bez validácie, správy okien od procesov rovnakého používateľa, mutexy, `WM_SETTINGCHANGE` ukazovateľ, `unsafe` bloky bez `SAFETY` komentárov.
- **Pripravenosť:** licencia MIT s neutrálnym držiteľom v poriadku; chýbalo anglické README, PRIVACY, SECURITY, CONTRIBUTING, SUPPORT, šablóny, THIRD_PARTY_NOTICES; CI bez `permissions`/`--locked`; pevné cesty v skriptoch; popisy crate-ov po slovensky.

#### 2. Čo sa zmenilo (malé commity)
- `95ba322` **fix(security):** `read_capped` (limit 256 MiB pre logy, 64 MiB pre súbor mosta, `take(cap + 1)`, nad limit = preskočené a započítané ako chyba) v `transom-usage`; `shell:AppsFolder` sa spúšťa len pre platné AUMID (`is_valid_aumid`, test); neutrálne testovacie cesty. Testy: usage 94, platform 57.
- `f77a06e` **chore(repo):** CI `permissions: contents: read`, `push` na `main` + `pull_request`, `concurrency`, `timeout`, `--locked`; `deny.toml` `yanked = "deny"`; skripty `tools/gate-appbar.ps1` a `tools/ticker-measure.ps1` odvodzujú cestu od umiestnenia skriptu; `.gitignore` (zálohy transom, `claude-setup.json`, `.claude/`, `*.tmp`, `/target-*/`).
- `103a10a` **docs:** anglické `README.md` označené ako alpha / early preview s tabuľkou overenia a obmedzení (pôvodný slovenský text uložený ako `docs/NOTES.sk.md`), `PRIVACY.md`, `SECURITY.md` (vrátane známych, sledovaných rizík), `CONTRIBUTING.md`, `SUPPORT.md`, `THIRD_PARTY_NOTICES.md` (22 externých crate-ov z `cargo metadata`, plné licenčné texty sú potrebné až pred distribúciou binárok), šablóny issue a PR, `spikes/README.md` (archivované, nepodporované), `docs/README.md`, `docs/PUBLISHING.md` (kontrolný zoznam a otvorené rozhodnutia), anglické popisy crate-ov.
- **Nezmenené (zámerne):** `Cargo.lock` ostáva sledovaný; LICENSE; história a vetvy; opravy stredných nálezov (`settings.json` dočasný súbor/zálohy, rotácia mosta) ostávajú zdokumentované v `SECURITY.md` ako ďalší krok.

#### 3. Disk
Voľné miesto na C: **147,63 GB → 178,58 GB (uvoľnených 30,95 GB)**. Zmazané boli iba pomenované, ignorované a regenerovateľné veci po overení, že nebeží žiadny `cargo`/`rustc`/`transom`/`rust-analyzer` proces: `target\` repozitára (~17,9 GB, `cargo build`), `spikes\*\target` pre s1, s2, s3, s4, s5, s6-host, s6-r0, s6-r1, s9 (~1,2 GB, offline overené `cargo metadata --offline --locked`) a z mojej scratchpad: snímky obrazoviek `shot-*`, `shots`, `pop-png*`, `menu-png` (len PNG/TXT z offscreen auditov) a dva moje build adresáre `target-release`, `target-rel`. Žiadny proces sa neukončoval, nepoužil sa `git clean` ani masové mazanie.
**Ponechané:** `spikes/s6-r4/target` (969 MB): jeho závislosti (`eframe`) nie sú v lokálnom registri a **offline sa znova zostaviť nedá**; scratchpad inej relácie (1,2 GB, `cb-r*`); `spikes/s5-media/data` (osobné názvy skladieb, ignorované); `~/.claude` (174 MB), `~/.cargo/registry` (385 MB), `%APPDATA%\transom`, `%LOCALAPPDATA%\transom`, koreňové `*.bmp` (5 MB, ignorované). Repozitár má teraz ~984 MB (z toho 969 MB `s6-r4/target`), `.git` 6 MB.

#### 4. Overenie
`cargo fmt --check` čistý, `cargo clippy --workspace --all-targets -D warnings` bez nálezov, `cargo test --workspace` **3× rovnako: 529 testov prešlo, 0 zlyhalo, 5 ignorovaných**, `cargo deny check` (advisories, bans, licenses, sources) ok, release build do oddeleného adresára OK. **CI na GitHube nikdy nebehlo**; jeho výsledok sa nehlási (pozri `docs/PUBLISHING.md`).

### 8.22 Míľnik: Nástroje (Polica, schránka, poznámky, snímky) a vstavané vyhľadávanie (2026-11-21)

**Vetva** `feature/profiles-audio-shelf` (lokálna, bez remote; žiadny push, merge ani vydanie). Postup: siedmi agenti na nezávislý výskum, kontrolu implementácie a bezpečnosť/okrajové prípady
(OLE a neaktivujúce okno, schránka, súkromie a úložisko, snímanie obrazovky, Windows Search, UX a prístupnosť, riziká); ich zistenia sa zosúladili **pred** zmenou spoločných súborov
(v tomto poradí: `tray`→`tools` premenovanie v modeli/konfigurácii/lokalizácii, potom ukladacie vrstvy, potom platforma, potom UI). Označenia: **TEST** (automatizovaný test),
**SIM** (simulované / falošné rozhranie), **MERANIE** (skutočné meranie na tomto počítači), **LIVE** (na skutočnej ploche používateľa), **NEOVERENÉ**.

#### 1. Čo sa zmenilo
| Oblasť | Zmena | Overenie |
|---|---|---|
| Tlačidlo **Tray** (Win+B) | Z lišty zmizlo; nahradila ho položka **Nástroje** (lokalizovaná v 11 jazykoch). Staré rozloženia sa prenesú (`tray`→`tools` na tom istom mieste, import to robí tiež). Vlastná ikona transom v oblasti oznámení (obnova, ponuka) **ostáva nedotknutá**. `Win+B` ostáva len ako voliteľná rýchla akcia v paneli Zvuk | **TEST** (rozloženie, migrácia, import), kód ikony nezmenený. **LIVE nebolo** |
| **Vybezok pre súbory** | Pri ťahaní súborov nad lištu sa pod stredom animovane rozbalí „vybezok“ (ease-out 180 ms / 140 ms, bez animácie pri zníženom pohybe, bez blikania pri prechode lišta↔vybezok). Pustenie pridá **odkazy** na súbory (viacero naraz, duplicity sa zlúčia). Originály sa **nikdy** nemenia, nemažú ani neotvárajú | **TEST/SIM**: `IDropTarget` volaný priamo so syntetickým `IDataObject` (efekt vždy COPY, nikdy MOVE; ne-súborový objekt sa odmietne); registrácia `RegisterDragDrop` na okne `NOACTIVATE|LAYERED|TOPMOST` **skutočne funguje** (test; samotný vybezok je `NOACTIVATE|TOPMOST` bez `LAYERED`), vrátane odregistrovania a opätovnej registrácie. **LIVE ťahanie myšou NEBOLO** (vyžaduje skutočný vstup na ploche) |
| **Polica** | Odkaz alebo uložená kópia (len na výslovnú voľbu „Kópia“; kvóty, bez adresárov, reparse bodov a cloudových zástupcov), pripnutie, odstránenie, vymazanie, retencia, limity; chýbajúci zdroj sa len ukáže. Pretrvá reštart. Ťahanie von: štandardné OLE (`SHDoDragDrop`, len COPY|LINK) | **TEST** (viacsúborové pustenie, duplicity, chýbajúce zdroje, limity, kópia a pád pri kopírovaní, nikdy nemaže originály). Ťahanie von **NEOVERENÉ LIVE** |
| **História schránky** | **Predvolene vypnutá**; zapína sa v *Nastaveniach > Nástroje*. Odber zmien výlučne cez `AddClipboardFormatListener` (žiadne dotazovanie), len kým je zapnutá a nie je pozastavená (pri pozastavení sa obsah vôbec nečíta). Text / obrázky / zoznamy súborov podľa výberu, limity a retencia, pripnutie, mazanie, obnovenie späť (vyhľadávanie v logike úložiska je hotové a otestované, **v UI zatiaľ nie je zapojené**) (vlastný zápis je označený a nezachytí sa znova). **Citlivý obsah sa preskočí** (`ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, `CanIncludeInClipboardHistory = 0`, vylúčené aplikácie); tvrdenie „zachytíme každé tajomstvo“ sa **nerobí** a UI to hovorí. Nelogue sa, nesynchronizuje, nenahráva, neexportuje. Neimportuje Win+V | **TEST** (rozhodovacia logika: vypnuté/pozastavené, citlivé formáty, vylúčené aplikácie, vlastné zápisy, limity, duplicity, retencia), skutočný WIC a DIB parser. Skutočný odber zo schránky: test len spustí/zastaví listener, **obsah skutočnej schránky sa v testoch nemení ani nečíta** |
| **Rýchle poznámky** | Vytvorenie, úprava, pripnutie, kopírovanie, mazanie; vyhľadávanie poznámok len cez názvy vo vyhľadávači (v paneli zatiaľ nie); atomický zápis so zálohou `.bak` (prerušený zápis sa obnoví), poškodený súbor sa **neprepíše**, nikdy sa automaticky nemažú, žiadna synchronizácia, neexportujú sa. Editor v paneli (pisanie, Backspace, Enter, Ctrl+V, Ctrl+Enter); zatvorenie okna uloží; bidi/nulová šírka/riadiace znaky sa odmietnu | **TEST** (obnova po prerušenom zápise, novšia verzia, limity bez mazania, sfalšovaný index, editor, audit vzhľadu v 11 jazykoch × 3 DPI × 2 témy, MSAA) |
| **Snímky obrazovky** | **Len na výslovnú akciu**, bez snímania na pozadí a bez obchádzania UAC: „Zachytiť…“ otvorí Snipping Tool (`ms-screenclip:`), „Uložiť obrázok zo schránky“ načíta DIB (limity 16 384 px a 64 Mpx), ukáže **náhľad** a až **Uložiť** zakóduje PNG (WIC mimo UI vlákna) a uloží; **Zahodiť** neuloží nič. Premenovanie (F2, sanitizovaný názov, nikdy neprepíše), kopírovanie súboru, tahanie von, mazanie, kvóta a retencia | **TEST** (zrušenie, zlyhanie, strata zariadenia, obrovské rozmery, náhľad vs. uloženie, kolízie názvov, kvóta, skutočné WIC kódovanie PNG). **`GraphicsCapturePicker` sa NEPOUŽIL** (pozri rozhodnutie 6) |
| **Panel Nástroje** | Sekcie Polica / Schránka / Poznámky / Snímky, tlačidlo Hľadať (otvára vyhľadávač), klávesnica, myš, MSAA, téma, jazyk, DPI | **TEST** (audit 11 jazykov × 3 DPI × 2 témy; MSAA role/stavy; klávesy; dvojkrokové mazanie). **Narrator / NVDA neboli spustené** |
| **Nastavenia > Nástroje** | Prepínače, limity (cykly), vysvetlenie čo/kde sa ukladá („nešifrované na tomto PC“), dvojkrokové vymazanie každého úložiska (znacka `clear-*.req`, vykoná bežiaca lišta – jediný pisár), sekcia vyhľadávača vrátane výberu vylúčených priečinkov systémovým dialógom | **TEST** (uloženie, predvolene vypnuté, dva kliky, audit stránky vo všetkých jazykoch, klávesový kruh) |
| **Súkromie dát** | Nový oddiel `tools` v `config.json` nesie len prepínače a limity; **nikdy sa neexportuje ani neimportuje** (test: export neobsahuje `tools`, `clipboard`, `excluded_apps`, cesty; import nezapne schránku). Obsah úložísk je v `%LOCALAPPDATA%\transom\tools\{shelf,clipboard,notes,screenshots,launcher}` (verzované JSON indexy s `format` + `version`, atomický zápis, `.bak`, strop 16 MiB pri čítaní, novšia verzia a poškodený súbor sa **neprepíšu**, odstraňujú sa len vlastné priečinky s náhodným id) | **TEST** |

#### 2. Záznam návrhových rozhodnutí (vyhľadávač a nástroje)
1. **Neaktivujúce okno a OLE pustenie.** Riziko: `WS_EX_NOACTIVATE` AppBar by nemusel byť cieľom pustenia. **Overené testom**: `RegisterDragDrop` uspeje na okne s `NOACTIVATE|LAYERED|TOPMOST`. Vybezok je samostatné neaktivujúce okno (tiež cieľ pustenia), takže funguje aj nad lištou, aj nad vybezkom. OLE beží na vlákne lišty (STA), pustenie sa zaradí a spracuje po návrate z OLE.
2. **Efekt pustenia je COPY alebo NONE.** MOVE sa nikdy neodpovedá, aby zdroj nič nemazal. Dáta sa nečítajú počas `DragOver`; až pri `Drop`, s limitom 4 MiB a vlastným parserom `DROPFILES` (zlé offsety, ANSI, neukončené zoznamy, osamelé surogáty, obrovské cesty – **TEST**).
3. **Samostatná služba nástrojov (jedno vlákno, jediný pisár).** UI vlákno nikdy nečaká na disk. Nastavenia nezapisujú do úložísk: vymazanie ide cez značku, ktorú vykoná lišta; Nastavenia sú tak oddelený proces bez zdieľaného zápisu.
4. **Vyhľadávač je popover lišty, nie samostatný proces.** Alternatíva (rezidentný proces) by pridala IPC, druhý životný cyklus a stálu pamäť. *(Od §8.30 má vyhľadávač aj globálnu skratku, predvolene Alt+Space – registruje ju riadiace okno lišty, stále bez samostatného procesu.)* Popover nepotrebuje globálnu klávesovú skratku (otvára ho položka Search) a nič nebeží, kým ho nepoužijete: pracovné vlákno sa vytvorí pri prvom otvorení, **blokuje na `recv` bez časovača** (nulové prebudenia v pokoji) a po zmene nastavení sa zahodí.
5. **Zdroje a API.** *Aplikácie:* zložka „Aplikácie“ shellu (`FOLDERID_AppsFolder` + `IEnumShellItems`; klasické aj balené aplikácie jedným zdokumentovaným API; spustenie `shell:AppsFolder\<AUMID>`; **MERANIE: 238 položiek na tomto PC**). Alternatíva (rekurzívne skenovanie Štart ponuky `.lnk`) by nezachytila balené aplikácie. *Súbory a priečinky:* index **Windows Search cez OLE DB** (`Search.CollatorDSO`, SQL `SystemIndex`, len čítanie, `TOP n`), pretože `ISearchQueryHelper` samo dotaz nevykoná a ADO by pridalo automatizačnú vrstvu; **MERANIE: studený prvý dotaz ≈ 8,9 s (jednorazovo po štarte služby), teplý 25–60 ms** → nahrievanie na pozadí pri prvom otvorení a **postupné** zobrazovanie (rýchle zdroje hneď, súbory neskôr). Dotaz sa **nikdy nevkladá do príkazového riadka**; do SQL ide len `sql_like_term` (povolené len písmená/číslice/základné znaky, apostrof zdvojený, `%_[]` a riadiace znaky vypustené, ≤ 80 znakov – **TEST**). *Nastavenia Windows:* pevný zoznam 12 zdokumentovaných `ms-settings:` URI. *Akcie transom a Poznámky/Polica:* zoznamy v pamäti (len názvy, **obsah poznámok sa nehľadá**).
6. **Snímanie obrazovky.** `GraphicsCapturePicker` (WinRT, Direct3D11, snímkový fond) by vyžadoval interaktívny výber okna, ktorý sa nedá bezpečne a nenápadne otestovať na ploche používateľa; kód by zostal **neoverený**. Preto sa zvolil **overiteľný** tok: systémový Snipping Tool (`ms-screenclip:`) → schránka → výslovné „Uložiť obrázok zo schránky“ → náhľad → uloženie. Splnené: výslovná akcia, systémom podporovaný výber, náhľad pred uložením, zrušenie/zlyhanie/limit, žiadne snímanie na pozadí, žiadny obchod s UAC. Nesplnené (**dokumentované obmedzenie**): transom nevie, kedy Snipping Tool skončil – používateľ musí kliknúť na druhé tlačidlo. Stavový automat `Capture` (zrušenie, strata zariadenia, oneskorenie, nadmerný rozmer) a rozhranie `Capturer` zostávajú pripravené, ak sa neskôr doplní `GraphicsCapturePicker`.
7. **Radenie.** Čistá funkcia: presná zhoda > predpona názvu > predpona slova > podreťazec > postupnosť znakov > zhoda v podrobnosti; všetky tokeny musia sedieť; bez diakritiky a bez ohľadu na veľkosť písmen; stabilné poradie; duplicity (druh + cieľ) sa zlúčia; voliteľný bonus pre nedávne voľby (predvolene vypnuté, ukladajú sa **názvy zvolených výsledkov**, nie dotazy, lokálne v `tools\launcher\recent.json`).
8. **Zrušenie a odolnosť.** Nový dotaz hneď zruší bežiaci pomalý zdroj; zastarané výsledky sa nikdy nepublikujú (generácia); panika jedného zdroja nezhodí hľadanie; chýbajúci index → plytké prehľadanie nastavených priečinkov (**len názvy, bez rekurzie, najviac 2000 položiek na priečinok**, nikdy obsah) a UI to vysvetlí; vylúčené priečinky sa porovnávajú na hranici cesty.
9. **Nič sa nespustí, kým výsledok nevyberiete.** Cieľom je vždy hotová položka zo zdroja (`Target`), nikdy text dotazu; `ShellExecute` len s overeným cieľom (bez riadiacich znakov); „Ukázať v priečinku“ cez `SHOpenFolderAndSelectItems` (nespúšťa); „Kopírovať cestu“ len pre skutočné cesty (nie pre `shell:AppsFolder`). Zastaralý zástupca: otvorenie zlyhá a ukáže sa „Túto položku sa nepodarilo otvoriť“.
10. **Hlavné hľadanie.** Položka Search na lište otvára vstavaný vyhľadávač (`Action::Launcher`); natívne Windows Search (`Win+S`) ostáva **len ako voliteľná rýchla akcia** v paneli Zvuk (žiadne dve rovnako výrazné tlačidlá).
11. **Vstup textu.** Popover prijíma `WM_CHAR` (zložené surogáty, bez riadiacich znakov) a `Ctrl` stav klávesu; **Ctrl+V** je výslovná akcia (jediné čítanie schránky vyhľadávačom/editorom). Podpora IME: **NEOVERENÁ** (jednoduché IME doručujú znaky cez `WM_CHAR`; zložené okno IME nie je kreslené).

#### 3. Merania a ciele (vyhľadávač)
| Ukazovateľ | Cieľ | Namerané |
|---|---|---|
| UI vlákno počas hľadania | nikdy neblokuje (všetko mimo UI vlákna) | **TEST**: zdroje a radenie bežia na pracovnom vlákne; UI len preberá hotové výsledky |
| Radenie 5000 položiek | < 16 ms (jeden snímok) v release | **MERANIE: 5,7 ms** (release), 33 ms (debug) |
| Debounce | ≤ 60 ms od posledného znaku | 60 ms (**TEST**: rýchle písanie = jediné hľadanie posledného dotazu) |
| Zoznam aplikácií | jednorazovo na pozadí | 238 položiek, načítané mimo UI vlákna |
| Index súborov teplý / studený | < 100 ms / nahriať na pozadí | **MERANIE: 25–60 ms** teplý, **≈ 8,9 s** studený (jednorazovo) |
| Nečinnosť (CPU / pamäť) | 0 prebudení, bez časovača | **Konštrukčne**: vlákno blokuje na `recv`, vytvára sa až pri prvom otvorení. **Meranie CPU/pamäte bežiacej lišty NEBOLO urobené** (vyžaduje spustenie lišty na ploche používateľa) |

#### 4. Čo sa NEOVERILO LIVE (a prečo)
- Ťahanie súborov myšou do lišty a z Police von (potrebuje skutočný vstup myši; **nesimulovalo sa**). Overené je len rozhranie `IDropTarget` a registrácia okna.
- Odber skutočnej schránky a obnova položky (nemení sa schránka používateľa); skutočné obchádzanie citlivých formátov správcami hesiel.
- Narrator/NVDA; IME; viac monitorov a zmena DPI počas behu pre vybezok; Explorer reštart s novými oknami.
- Tlačidlo Win+B je z lišty preč a ikona v oblasti oznámení nezmenená (**TEST** rozloženia a nezmenený kód), ale **LIVE potvrdenie na ploche nebolo urobené**.
- CI na GitHube nikdy nebežalo.

#### 5. Obmedzenia a známe medzery
- Snímanie je cez Snipping Tool a schránku (pozri rozhodnutie 6); `GraphicsCapturePicker` chýba.
- Obrázky zo schránky sa zachytávajú len ako `CF_DIB` (bez priehľadnosti; alfa sa vynúti nepriehľadná); formát `PNG` iných aplikácií sa nečíta.
- Premenovanie snímky je cez F2 (bez tlačidla); poznámky sa upravujú len na konci textu (bez kurzora uprostred a výberu).
- Zložené okno IME sa nekreslí. Podrobnosti o stave súborov z cloudu (OneDrive): nikdy sa nesťahujú, Polica ich označí ako nedostupné.

#### 6. Nezávislá kontrola (dvaja agenti len na čítanie) a jej zosúladenie
Dvaja agenti (správnosť zapojenia; bezpečnosť a okrajové prípady) prečítali kód bez spustenia lišty. Nálezy som overil v kóde a opravil s regresnými testami, kde to bolo možné.

**Opravené:**
| Nález | Oprava |
|---|---|
| **Služba nástrojov sa po štarte nespustila** (`notifier()` ešte nebol k dispozícii v `App::new`, `sync_tools` bežalo len z `reload_config`) – zhodená položka, panel, schránka a zoznamy vyhľadávača po reštarte prázdne | `on_event` spúšťa službu, kým `tools` nie je `Some` (rovnako ako `sync_audio`) |
| Po „Vymazať“/zmazaní ostal obsah v `index.json.bak` (poznámky, náhľady schránky, cesty Police) | `purge_backup` po zmazaní/vymazaní (poznámky, schránka, Polica); **TEST** |
| Zamietnuté pustenie (priečinok pri zapnutej kópii…) vytlačilo existujúce položky Police | vytláčanie až pri prijatí položky; **TEST** |
| Premenovaná snímka s `.tmp-` v názve sa pri štarte zmazala | upratuje sa len presný vzor `<názov>.png.tmp-<id>`; **TEST** |
| „Ponechať navždy“ (história schránky) sa potichu menilo na 1 deň | 0 = ponechať do zmazania (konfigurácia aj `enforce`); **TEST** |
| Neznámy vlastník schránky obchádzal zoznam vylúčených aplikácií | pri neprázdnom zozname sa neznámy vlastník nezachytáva (fail-closed); **TEST** |
| Rozpísaná poznámka sa stratila pri kliknutí na položku Nástroje na lište, pri limite poznámok a pri zlyhaní | pred vynulovaním panela sa uloží; pri dosiahnutí limitu editor ostane otvorený so správou; zlyhanie kódovania PNG ukáže chybu |
| Počas náhľadu snímky klávesy pôsobili na skrytý zoznam (Enter/Delete/F2) | `key()` pri náhľade prijíma len Tab a Enter/medzerník na zaostrenom tlačidle |
| Vypnutý vyhľadávač otváral nefunkčné okno | Search vtedy použije Windows Search (`Win+S`) |
| Pomalý zdroj (studený index) zdržal novší dotaz; zrušenie nezrušilo `Execute` | pomalé zdroje bežia mimo slučky, dotazy do indexu sú jednovláknové a po získaní zámku sa zastarané preskočia; **TEST** (zdroj, ktorý sa nedá zrušiť, nezdrží rýchle výsledky nového dotazu) |
| Nekonečné opätovné dotazovanie po „aplikácie načítané“ a zastaralé výsledky starého behu | samostatná notifikácia pre načítanie aplikácií; schránka výsledkov je pre každý beh vlastná; `Engine::drop` a zatvorenie okna hľadanie zrušia |
| Enter/klik otvoril výsledok starého dotazu | výsledky sú „zastarané“ po zmene textu, kým neprídu nové |
| Otvorená poznámka sa mohla stratiť medzi snímkami; správa „Pridané N“ zmizla pri prvom prebratí konfigurácie | `NoteOpenDone` potvrdenie; správa prežije nečinnostné zmeny |
| Vybezok neprešiel na iný monitor; po ťahaní von ostalo okno visieť | prepočet polohy; po ťahaní sa okno zatvorí |
| `\\localhost\pipe\…` (pomenovaná rúra cez UNC) sa dala pustiť na Policu | odmieta sa ako zariadenie; **TEST** |
| Recent: názvy poznámok a Police sa ukladali do `recent.json` | do nedávnych sa nezapisujú |
| Retencia sa uplatnila len pri štarte; `_` v dotaze sa zahodilo; COM chýbal vo vlákne schránky; chyby textu v PRIVACY (rozbité escape sekvencie); tvrdenie o „vyhľadávaní“ v schránke/poznámkach v PLAN | retencia aj pri zmene nastavení; `_` povolené; COM inicializované; texty opravené/zúžené |

**Neopravené, vedome (dokumentované obmedzenia):**
- Verejné ID riadkov: Delete/Pin cielia na index riadku zo snímky; ak sa snímka zmení medzi kreslením a klikom, môže zasiahnuť inú položku (riziko je nízke, mazanie je dvojkrokové).
- Osirotené kópie Police po páde medzi kopírovaním a uložením indexu sa neupratujú; Polica sondovaním položiek (UNC) môže spomaliť jediné pracovné vlákno služby.
- Obrázok zo schránky do 64 MiB sa kóduje do PNG ešte pred uplatnením limitu `max_image_mb` (vplyv len pri zapnutej histórii a nepriateľskom zdroji).
- Obnovený obrázok sa do schránky zapisuje len ako formát `PNG` (nie `CF_DIB`): vloží sa len do aplikácií, ktoré ho čítajú.
- Neúspešné uloženie poznámky pri **zatvorení** okna (limit) text nezachráni (pri tlačidle Uložiť áno).

#### 7. Záverečné kontroly (2026-11-21)
`cargo fmt --check` čistý; `cargo clippy --workspace --all-targets -D warnings` bez nálezov; `cargo test --workspace --locked`: **643 testov prešlo, 0 zlyhalo, 5 ignorovaných** (po oprave jedného testu vyhľadávača po kontrole; predchádzajúce tri behy mali jediné zlyhanie práve v ňom); `cargo deny check` v poriadku; `cargo build --release --locked` (čistý dočasný `target`, 43 s po predchádzajúcom zostavení závislostí) vytvoril `transom-app`, `transom-settings`, `transom-statusline`. **Spustená lišta, ťahanie myšou, skutočná schránka, Narrator a meranie CPU/pamäte nastaveného behu neboli vykonané.**

### 8.23 Míľnik: Stabilizácia Nástrojov – nezávislé kontroly, živé overenie, audit životného cyklu dát, výkon vyhľadávania, CI (2026-11-21)

**Vetva** `feature/profiles-audio-shelf` (lokálna, bez remote; **bez commitov v tomto míľniku** – zmeny sú v pracovnom strome; žiadny push, merge ani vydanie). Žiadna nová funkcia produktu; opravy, merania a testy.
Postup: traja agenti len na čítanie (bezpečnosť a súkromie; UX a okrajové prípady; build, závislosti a CI) → každé tvrdenie som overil sám (čítaním kódu, testom alebo živým behom) → až potom oprava. Označenia: **TEST** (automatizovaný), **ŽIVÉ** (skutočná lišta na ploche so scratch konfiguráciou a dátami, ovládaná správami priamo oknám popoverov v procese), **SIM** (simulované), **MERANIE**, **NEVYKONANÉ**.

#### 1. Živé akceptačné testy (`transom-app --test-tools`, len test; 47 kontrol, 0 zlyhaní v debug aj release)
Beh používa **scratch** `TRANSOM_CONFIG` a `TRANSOM_DATA` (bez nich sa scenár odmietne), dve skutočné lišty (dva monitory: portrétový vľavo a primárny 2560×1440; usporiadanie, DPI ani jazyk systému sa nemenili), syntetické súbory/poznámky/obrázky. **Do skutočnej schránky sa nič nezapisuje a nič sa z nej nečíta.** Vstup sa neposiela myšou ani klávesnicou plochy, ale `PostMessage` priamo oknu popoveru tohto procesu.
| Oblasť | Výsledok | Poznámka |
|---|---|---|
| Otvorenie Nástrojov, názvy pre čítačku (koreň, karty, Search), Tab do kruhu fokusu, Esc zatvorí | **ŽIVÉ – PASS** | MSAA uzly overené cez `a11y()`; Narrator/NVDA **NEVYKONANÉ** |
| Obnova popredia po zatvorení | **ŽIVÉ – PASS** | pred/po: `Chrome_WidgetWin_1` |
| Dva monitory: popover zostane v hraniciach oboch monitorov | **ŽIVÉ – PASS** | monitor 0 (−1080,−405)–(0,1515), monitor 1 (0,0)–(2560,1440) |
| Polica: pustenie 4 existujúcich + 1 chýbajúcej cesty, duplicita, dvojkrokové zmazanie | **ŽIVÉ+SIM – PASS** | pustenie sa vkladá do rovnakej fronty ako OLE (`simulate_drop`); **samotné OLE ťahanie myšou NEVYKONANÉ**; originály bajt po bajte a podľa mtime nezmenené; chýbajúca cesta zamietnutá |
| Poznámky: nová, písanie, uloženie, znovuotvorenie, zahodenie, úprava, **zlyhanie uloženia (limit)** | **ŽIVÉ – PASS** | pri zlyhaní editor ostane s textom; po zatvorení okna sa neuložiteľná poznámka obnoví (`note_draft`) |
| Snímky: tlačidlá existujú, náhľad pred uložením, Zahodiť neuloží nič, Uložiť zapíše platné PNG, zatvorenie okna zahodí náhľad | **ŽIVÉ+SIM – PASS** | náhľad je syntetický obrázok; tlačidlá „Zachytiť…“ a „Uložiť obrázok zo schránky“ sa **nestláčali** (spúšťajú Snipping Tool / čítajú skutočnú schránku) |
| Vyhľadávanie: akcie, Nastavenia Windows, názov poznámky, názov na Polici, aplikácie, súbory cez index | **ŽIVÉ – PASS** | pozri merania |
| Simulovaný studený index (6 s, necancelovateľný) | **ŽIVÉ – PASS** | rýchle výsledky nového dotazu prídu za 157 ms, UI medzera 42 ms, zobrazí sa „Hľadám súbory…“ |
| Žiadna slučka dotazov po ustálení | **ŽIVÉ – PASS** | 6 dotazov do indexu, o 2 s neskôr stále 6 |
Živý beh **našiel skutočnú chybu**, ktorú testy jednotiek nezachytili: po zapojení „apps loaded“ zostal v `on_launcher_results` vetva, ktorá pri prázdnej schránke znova spustila dotaz → **nekonečná slučka ~6 dotazov/s do indexu**, súborové výsledky nikdy nedobehli (`done=false`). Opravené, regresná kontrola je priamo v živom scenári.

**Nebezpečné / nevykonané bez tvojho kroku – presné ručné kroky:**
1. *Ťahanie dnu/von (OLE):* spusti lištu, v Prieskumníkovi označ 3 testovacie súbory a potiahni ich na lištu (vybezok sa rozbalí, po pustení sa objavia v Nástroje > Polica). Potom potiahni riadok Police do iného priečinka a over, že originál ostal. Zruš ťahanie Esc (vybezok sa musí zbaliť). Potiahni virtuálny súbor (príloha z Outlooku) – lišta ho odmietne bez vysvetlenia (známe).
2. *História schránky:* Nastavenia > Nástroje > zapnúť históriu; skopíruj syntetický text `transom-test-123` a obrázok (Win+Shift+S); over záznamy; Pozastaviť → skopíruj iný text → nesmie pribudnúť; skopíruj z aplikácie označujúcej súkromný obsah (správca hesiel) → nesmie pribudnúť; pridaj `.exe` do `excluded_apps` v `config.json` a over; Vymazať a skontroluj `%LOCALAPPDATA%\transom\tools\clipboard` (aj `index.json.bak`).
3. *Snímky:* Nástroje > Snímky > „Zachytiť…“, vyber výrez; **vráť sa** do Nástrojov > Snímky > „Uložiť obrázok zo schránky“ > náhľad > Uložiť. Zruš výrez Esc a over, že sa nič nezmenilo. (Windows Snipping Tool si ukladá vlastnú kópiu do `Pictures\Screenshots` a do histórie Windows – transom ju nevie zmazať.)
4. *Narrator/NVDA, IME, zmena DPI za behu:* ručne.
5. *Studený Windows Search:* po dlhej nečinnosti PC prvý dotaz na súbory (už nameraný raz ≈ 8,9 s); UI sa nesmie sekať a má ukázať „Hľadám súbory…“.

**Verdikt snímok:** je to **prototyp**, nie hotová funkcia: päťkrokový ručný tok (Zachytiť… → okno sa zatvorí pri aktivácii výstrižku → znova otvoriť Nástroje → karta Snímky → Uložiť obrázok zo schránky → náhľad → Uložiť), žiadne automatické prevzatie výsledku, žiadna správa pri zrušení, len `CF_DIB` (24/32 bpp); či Windows pre výstrižok vyrobí `CF_DIB` zo svojho PNG/`CF_BITMAP`, som **nemohol overiť** bez zásahu do schránky.

#### 2. Overené nálezy a opravy (kontroly + živý beh; každý s regresným testom, ak je možný)
| Oblasť | Nález | Oprava |
|---|---|---|
| Vyhľadávač | **Slučka opätovných dotazov** (viď vyššie), súbory nikdy nedobehli | odstránená vetva, regresia v živom scenári |
| Vyhľadávač | Žiadny ukazovateľ priebehu pri pomalom indexe | „Hľadám súbory…“ v pätičke (11 jazykov); rýchle výsledky sa ukážu hneď |
| Vyhľadávač | Zvýraznený výsledok sa po neskorších súboroch posunul (Enter mohol otvoriť iný) | výber sa drží podľa `Target::key()`; **TEST** |
| Vyhľadávač | Riadky 8+ neboli vykreslené, ale boli vybrateľné; okno prirýchle nízke | navigácia len po nakreslené riadky; okno 530 lp; **TEST** |
| Vyhľadávač | Zoznam aplikácií sa načítal len raz za proces | obnova pri každom otvorení (mimo UI vlákna) |
| Súkromie | `--log` zapisoval každý napísaný znak (`PopoverChar`) a plné cesty pustených súborov (**overené v živom logu**) | loguje sa len druh udalosti/počet; kontrola logu: 0 výskytov textu a ciest |
| Súkromie | `.bak` uchovával staré znenie po úprave poznámky, retencii, vyhodení, zmazaní snímky a premenovaní | `.bak` sa maže po každom uložení, ktoré niečo odstránilo/zmenilo (Polica, schránka, poznámky, snímky); **TEST** |
| Súkromie | Retencia Police sa neukladala (zmazané kópie ostali v indexe) | po `enforce` sa ukladá (+ čistí `.bak`); **TEST** |
| Súkromie | Súkromný príznak prítomný, ale nečitateľný = povolené | nečitateľný príznak = nezachytiť (fail-closed) |
| Súkromie | Naše zápisy do schránky šli do Win+V a cloudu | `CanIncludeInClipboardHistory = 0` a `CanUploadToCloudClipboard = 0` pri našich zápisoch; únik `HGLOBAL` pri zlyhaní opravený |
| Súkromie/dáta | Sirotské súbory (kópie Police bez záznamu, `*.tmp`, nezavesené bloby) sa nikdy nečistili | upratanie pri otvorení (len naše tvary názvov); **TEST** |
| Súkromie/dáta | Nezmazateľný snímok (otvorený v prehliadači) zmizol zo zoznamu, súbor ostal | záznam ostane, kým sa súbor nezmaže; **TEST** (zdieľanie bez práva mazať) |
| Súkromie/dáta | Vymazanie vypnutých poznámok potichu nič nerobilo | mazanie nie je viazané na „zapnuté“; **TEST** |
| Bezpečnosť | `\\?\UNC\localhost\pipe\x`, `//localhost/pipe/x` obišli kontrolu pomenovaných rúr (agent overil pripojenie na rúru) | normalizácia lomítok a `\\?\UNC\`, rúry aj `mailslot`; `copy_in` berie len obyčajný súbor; **TEST** |
| Dáta | Zoznamy súborov v histórii schránky sa nededuplikovali | uložený odtlačok; **TEST** |
| Polica | Viacsúborové pustenie sa zobrazilo v opačnom poradí; pripnuté nebolo hore; odmietnutá položka vytlačila existujúce | poradie pustenia zachované, pripnuté hore, vytláčanie len po prijatí; **TEST** |
| UX | Pri dlhom zozname sa riadky kreslili cez hlavičku a klikalo sa na kartu pod nimi; vybraný riadok mimo obrazovky; správa/potvrdenie pod zlomom | zoznam v orezanom okne, hit-test len viditeľná časť, vyber sa posúva do zobrazenia, pätička vždy viditeľná; **TEST** |
| UX | Potiahnutie riadku a uvoľnenie nad iným riadkom ho aktivovalo (v schránke prepísalo schránku) | príznak „potiahnuté“; prah ťahania podľa DPI; **TEST** |
| UX | Editor poznámok: Esc/zatvorenie ukladá, žiadne klávesové zahodenie | `Ctrl+Delete` zahodí (11 jazykov v nápovedi); názov uzla pri premenovaní; **TEST** |
| UX | Náhľad snímky bez fokusu (Enter nič) | fokus na „Uložiť“; klávesy neovplyvnia skrytý zoznam; **TEST** |
| UX | Koncept poznámky sa pri výbere z vyhľadávača stratil; vyhľadávač otváral Nástroje na náhodnom monitore | poradie `act` → sekcia; monitor z `popover_monitor` |
| UX | Pustenie súborov bez spätnej väzby | oznam na lište po spracovaní (Pridané/Zamietnuté) |
| Build | `cargo-deny-action` je Docker akcia (len Linux) – posledný krok CI by na `windows-latest` zlyhal | samostatná úloha `deny` na `ubuntu-latest` |
| Build | Exe vyžadovali `vcruntime140.dll` (čistý Windows 11 bez redistribuovateľného balíka by ich nespustil) | `.cargo/config.toml`: `+crt-static`; **MERANIE**: po zmene 0 importov `vcruntime140` (2,36/2,26/0,81 MB) |

#### 3. Audit životného cyklu dát (zhrnutie)
| Nástroj | Zber | Úložisko | Záloha | Mazanie | Logy | Export | Pád |
|---|---|---|---|---|---|---|---|
| Polica | len pustenie (odkaz; kópia na výslovnú voľbu) | `tools\shelf\index.json` + `files\<id>\` | `.bak` (čistí sa pri odstránení) | odstrániť/Vymazať/retencia vymaže kópie, **originály nikdy**; sirotské kópie sa upracú | len počet | nie | atomický zápis, `.part` + premenovanie |
| Schránka | len po zapnutí; listener len kým je zapnutá a nepozastavená | `tools\clipboard\index.json` + `blobs\` | `.bak` (čistí sa) | mazanie/Vymazať/retencia; sirotské bloby/`*.tmp` sa upracú | nikdy obsah | nie | atomický zápis |
| Poznámky | vaše písanie | `tools\notes\index.json` | `.bak` (čistí sa po úprave/zmazaní) | Zmazať/Vymazať aj pri vypnutých poznámkach | nikdy obsah | nie | atomický zápis; koncept pri zlyhanom uložení len v pamäti |
| Snímky | len výslovné uloženie z náhľadu | `tools\screenshots\*.png` + index | `.bak` (čistí sa) | zmaže súbor skôr než záznam | len počet | nie | `create_new` + premenovanie; osirotené `.png.tmp-<id>` sa upracú |
| Vyhľadávanie | dotaz v pamäti; voliteľne názvy vybraných výsledkov | `tools\launcher\recent.json` (predvolene nič) | – | vypnutie nastavenia zmaže | nikdy dotaz | nie | – |
Otvorené riziká: obrázok do 64 MiB sa kóduje pred uplatnením `max_image_mb`; UNC/WebDAV odkaz na Police sa sonduje na jedinom pracovnom vlákne služby (pri mŕtvom serveri môže zdržať ostatné nástroje); `excluded_apps` nemá používateľské rozhranie (len `config.json`); Snipping Tool a Windows si držia vlastné kópie obrazu mimo kontroly transom; obnovený obrázok sa zapisuje len ako formát `PNG`.

#### 4. Výkon vyhľadávania (MERANIE)
| Ukazovateľ | Hodnota | Podmienky |
|---|---|---|
| Prvý zodpovedajúci výsledok (akcie, nastavenia, poznámky, Polica, aplikácie) | **≈ 92 ms** (z toho 60 ms debounce) | release, živý beh, dve lišty |
| Prvý výsledok z indexu súborov (teplý) | **125 ms**; samotný dotaz 34–46 ms | release / debug, v procese aj v samostatných testoch (3 behy: prvý 42 ms, druhý 35 ms) |
| Najväčšia medzera medzi tepmi 20 ms (blokovanie UI vlákna počas písania a hľadania) | **32 ms** (release), 230–270 ms (debug, kreslenie ≈ 55 ms/snímku) | živý beh |
| Simulovaný studený dotaz 6 s, novší dotaz | výsledok nového dotazu **157 ms**, UI medzera **42 ms** | test-only háčik `set_test_index_delay_ms`: simulované necancelovateľným čakaním |
| Skutočný studený start Windows Search | **≈ 8,9 s** (nameraný raz po nečinnosti; **znova ho nemožno vyvolať bez zásahu do služby**, preto simulácia) | dotaz beží mimo UI vlákna a nikdy nezdrží novší |
| Nečinnosť (release, predvolená konfigurácia vrátane služby médií, služba nástrojov spustená, schránka vypnutá) | **0,000 % jadra za 30 s**; pracovná sada 24,3 MB, súkromná 3,7 MB, 14 vlákien | živé meranie |
| Špička počas celého živého scenára | pracovná sada **40,6 MB**, CPU spolu 1,06 s za 39 s (štart + kreslenie + hľadanie) | release |
| Radenie 5000 položiek | 5,7 ms (release) | MERANIE z predchádzajúceho míľnika |
Nič sa neprehľadáva celý disk a nepridal sa externý proces: index Windows Search + plytká záloha nastavených priečinkov.

#### 5. CI a pripravenosť repozitára
- **Stav:** `.github/workflows/ci.yml` prepracovaný (úlohy `lint`, `test`, `release-build` na `windows-latest`, `deny` na `ubuntu-latest`; `Swatinem/rust-cache`, `persist-credentials: false`, `workflow_dispatch`, `CARGO_INCREMENTAL=0`, kontrola `rustc -V`, `--locked`, `--no-fail-fast`, žiadne nahrávanie artefaktov ani vydanie). **CI na GitHube nikdy nebežalo a nebolo ho možné spustiť: repozitár nemá nakonfigurovaný remote.** Na spustenie treba **tvoj súhlas**: vytvoriť súkromný repozitár, `git push` vetvy a spustiť workflow (`workflow_dispatch`).
- **Lokálne s `--locked`:** `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -D warnings`, `cargo build --release --locked`, `cargo test --workspace --locked`, `cargo deny check` – výsledky v §8.23.7.
- **Riziká prvého behu (z kontroly, NEOVERENÉ na hosťovanom runneri):** testy rozloženia merajú skutočné GDI texty v 11 jazykoch (potrebujú písma vrátane CJK/Segoe Fluent Icons); app testy trvajú ≈ 500–615 s (serializované globálnym zámkom jazyka) – limit úlohy `test` je 60 min; testy s časovaním (`frametimer`, `launcher/tests`, `sound/engine`) môžu na zdieľanom VM kolísať; `shellops` testy závisia od Windows Search (tolerujú chybu); `measure_index_*` je teraz `#[ignore]`.
- **Neopravené odporúčania z kontroly buildu:** `dtolnay/rust-toolchain@1.100.0` je vetva (nie tag) – pripnúť na SHA po overení; `panic = "abort"` v release robí `catch_unwind` v `ai.rs` neúčinným (pád pri parsovaní logov zhodí lištu) – rozhodnúť; chýba vložený manifest (PerMonitorV2, longPathAware) a `VS_VERSION_INFO`; kokpit `docs/PUBLISHING.md` obsahuje zastarané čísla (commity, „tag exists“) – pred publikovaním znova zmerať; korene obsahujú vygenerované `*.bmp` (ignorované) z `examples/*` – predvolený výstup do `target/`.

#### 6. Zostávajúce prekážky vydania a ďalší míľnik
*(Aktualizované v §8.24: vyriešené sú limity pred kódovaním obrázkov a izolácia sieťových ciest, tok snímok, zmena poradia a „Pridať súbory“ v Polici, UI vylúčených aplikácií, `panic`, referencie CI; zvyšné prekážky sú v §8.24.8.)*
**Prekážky:** (1) CI nikdy nebežalo; (2) chýba živé ľudské overenie ťahania, schránky a čítačky obrazovky (kroky vyššie); (3) snímky sú prototyp; (4) Polica nemá **zmenu poradia** (žiadne `ShelfMove`, ani klávesová; ťahanie riadku je vždy ťahanie von) – vyžiadaná funkcia, v tomto míľniku zámerne nepridaná; (5) zostávajúce riziká zo sekcie 3 a 5 (UNC, limity pred kódovaním, panic, manifest); (6) prístupnosť: päť vecí je len klávesových (P, Delete, F2), žiadne tlačidlo „Pridať“ do Police, pätička vyhľadávača nie je uzol MSAA.
**Odporúčaný ďalší míľnik:** „Ľudské prijatie a prvé CI“ – (a) so súhlasom vytvoriť súkromný remote, spustiť CI a opraviť rozdiely prostredia; (b) prejsť ručné kroky z §8.23.1 s používateľom; (c) dokončiť tok snímok (automatické prevzatie po výstrižku alebo `GraphicsCapturePicker`, správa pri zrušení, `CF_DIBV5`/`PNG` vstup); (d) zmena poradia Police (`Alt+↑/↓`, ťahanie v paneli) a tlačidlo „Pridať“; (e) vložený manifest + verzia + rozhodnutie o `panic`.

#### 7. Záverečné kontroly (2026-11-21, `--locked`)
`cargo fmt --all -- --check` čistý; `cargo clippy --workspace --all-targets --locked -- -D warnings` bez nálezov; `cargo test --workspace --locked --no-fail-fast`: **652 testov prešlo, 0 zlyhalo, 6 ignorovaných** (jeden beh ≈ 10 min; ignorované: 5 pôvodných + `measure_index_*`); `cargo deny check`: advisories, bans, licenses, sources v poriadku; `cargo build --release --locked` (čistý scratch `target`, statické CRT) vytvoril `transom-app`, `transom-settings`, `transom-statusline`; živý scenár `--test-tools` v debug aj release: **47 kontrol prešlo, 0 zlyhalo**. Zmeny **nie sú commitnuté** (podľa zadania tohto míľniku).

### 8.24 Míľnik: Uzavretie zostávajúcich prekážok Nástrojov (2026-11-22)

Zadanie: zatvoriť zostávajúce prekážky z §8.23.6 bez ďalších funkcií mimo zoznamu, bez pushu, bez remote, bez commitu. Tri nezávislé kontroly len na čítanie (pamäť a sieťové cesty; tok snímok a Polica; CI a `panic`) boli **overené sám v kóde** skôr, než sa čokoľvek zmenilo. Každá položka nižšie má stav: **červený test** (zlyhal pred opravou), **len čítanie kódu** (test nemožno napísať bez zásahu do systému) alebo **vyvrátené**.

#### 1. Priebežný záznam (kto čo overil)
| Nález | Overenie | Oprava a test |
|---|---|---|
| `stored_file_name` skrátením nechá medzeru/bodku na konci → po reštarte sa položka s kópiou odmietne a kópia sa zmaže; skracovalo sa po znakoch, nie po UTF-16 jednotkách | **červený test** `stored_names_are_valid_and_stable_so_a_copy_survives_a_restart` | skracovanie po jednotkách UTF-16 + orezanie koncových medzier/bodiek |
| `check_path` prijímal `\\`, `\\server`, `\\server\` a `COM¹/²/³`, `LPT¹/²/³` | **červený test** `bare_unc_roots_and_unicode_device_variants_are_refused` | odmietnuté (`Relative`/`Device`) |
| Kópie z jedného pustenia sa nezapočítavali do spoločnej kvóty (500 × `max_copy_mb`) | **červený test** `copies_of_one_drop_count_together_toward_the_total_quota` (1,8 MiB pri limite 1 MiB) | súčet kópií dávky v kvóte; súčty `copy_bytes`/`total_bytes` saturujú |
| História schránky: obrázok sa kopíroval, dekódoval a kódoval do PNG **pred** kontrolou `images`/`max_side`/`max_image_mb` | **potvrdené čítaním kódu** (`read_snapshot`); vlastné testy hlavičky: `dib_dims`, `within`, `dib_wanted` | `ReadPolicy` z konfigurácie (vypnutý druh sa nečíta), hlavička DIB (12 B) rozhoduje pred kópiou, DIB a BGRA sa uvoľnia hneď po použití, `Listener::drop` už neblokuje UI |
| Obrázok pre **Snímky** sa kóduje pred limitom 64 MiB (zadanie) | **vyvrátené pre čítanie**: DIB ≤ 64 MiB sa kontroluje pred kópiou (`GlobalSize`), `dims_ok` a `d.get(..)` pred alokáciou BGRA (výstup ≤ 4/3 DIB ≈ 85 MiB); test `the_dib_header_is_peeked_without_the_pixels_and_oversized_or_malformed_images_fail_the_gate` to teraz skutočne overuje (hlavička 16384×16384 bez pixelov, `i32::MIN` výška, 1/4/8/16 bpp, RLE) | pridané len skoré odmietnutie podľa hlavičky aj v `read_image` |
| OLE pustenie: `tymed` sa nekontroloval (`u.hGlobal` pri streame = nezmyselný ukazovateľ) | **len čítanie kódu** (pred opravou by test spôsobil UB); test po oprave s nepriateľským `IDataObject` (`TYMED_ISTREAM`) | `is_hglobal_medium`, medium sa uvoľní |
| Sieťová cesta na Polici blokuje spoločné pracovné vlákno (`rows`, `add_paths`, `copy_in`) a `drag_out` volá `SHParseDisplayName` na UI vlákne | **potvrdené čítaním kódu**; test `an_unreachable_shelf_path_never_delays_other_tools_and_is_listed_as_unchecked` (sonda visí 5 s na ceste s písmenom disku aj na UNC) | modul `lane`: pevný fond sond s limitom 250 ms, otvorenie súboru s limitom 3 s, uviaznuté vlákna sa nezadávajú, sieťové cesty sa vôbec nesondujú, kópia zo siete sa odmieta (`RejectReason::Network`), stav `Unchecked`, sieťová položka sa neťahá von |

#### 2. Snímky obrazovky: výskum, tok, overenie
> **ZRUŠENÉ v §8.25:** zadanie sa upravilo – „Snímky“ je knižnica už uložených snímok, nie tok snímania. Celý tento tok (výber okna, náhľad, ukladanie, `shot_flow.rs`, `capture.rs`) je odstránený; text nižšie ostáva len ako záznam výskumu (Windows.Graphics.Capture, zastaraný `ms-screenclip:`).
**Výskum (Microsoft Learn; stránky som čítal sám, skúšky na systéme neboli možné bez človeka):**
- Pôvodné tlačidlo „Zachytiť…“ spúšťalo `ms-screenclip:` ako „dokumentovaný protokol“. Stránka *launch-screen-snipping* hovorí: „This legacy protocol is **deprecated** and is no longer supported“ (nahradený 1. 5. 2025). **Pôvodný tok teda na aktuálnych Windows nemusel fungovať vôbec** a nikdy nebol overený.
- Nový protokol `ms-screenclip://capture/image?rectangle…` vracia výsledok (`code` 200 / 408 / 499 = zrušil používateľ / 500) len cez `LaunchUriForResultsAsync`; súbor sa dáva cez `SharedStorageAccessManager` token, ktorý **nebalená aplikácia nevie vymeniť** (vyžaduje identitu balíka); že obrázok skončí v schránke, stránka **neuvádza**. Ako primárny tok nepoužiteľný, a bez spustenia prekryvu na ploche neoveriteľný.
- **Windows.Graphics.Capture** (`GraphicsCapturePicker` + `Direct3D11CaptureFramePool`) je jediná cesta zdokumentovaná od výberu po pixely pre nebalenú desktopovú aplikáciu (Windows 10 1903+; vlastník dialógu cez `IInitializeWithWindow`). Obmedzenia, ktoré sú zámerné a zdokumentované pre používateľa: celé okno alebo celá obrazovka (žiadny výber výrezu), systém kreslí žltý okraj počas snímania (bez identity ho nemožno vypnúť), len SDR; D3D11 sa používa **výlučne na prevzatie pixelov do pamäte** (vykresľovanie lišty ostáva GDI). DXGI Desktop Duplication (bez súhlasového okna, `E_ACCESSDENIED` na zabezpečenej ploche) a GDI `PrintWindow` (len `PW_CLIENTONLY` je zdokumentovaný) som zamietol.
- Výrez: Win+Shift+S a potom „Uložiť obrázok zo schránky“ (explicitné tlačidlo ostáva, číta schránku len po stlačení a už nie na UI vlákne).

**Tok (modul `shot_flow.rs`, `transom-platform::capture`):** „Zachytiť okno alebo obrazovku…“ → systémový výber (okno Nástroje sa zavrie) → snímka v pamäti → oznam na lište „Snímka je pripravená“ a Nástroje sa otvoria na náhľade → **Uložiť** alebo **Zahodiť** → späť v Nástrojoch (stav „Snímka uložená“). Zrušenie výberu = „Zachytenie zrušené. Nič sa neuložilo.“ (predtým žiadna spätná väzba). Nič sa neuloží bez potvrdenia a snímanie **nečíta ani nemení schránku**. Stavy: nečinný → beží (`Busy` pri druhom kliknutí) → {náhľad, zrušené, chyba: nepodporované / časový limit 120 s a 3 s na prvý snímok / príliš veľké / zlyhalo}; chyba uloženia **náhľad zachová** (predtým sa stratil: `preview.take()` pred výsledkom).
- **Limity pred drahou prácou:** rozmery položky (`dims_ok`: strana ≤ 16384, ≤ 64 Mpx) sa overia **pred** vytvorením zariadenia, frame poolu a staging textúry; kontroly „vypnuté / poškodené úložisko / plný počet / vyčerpaná kvóta“ idú **pred** kódovaním PNG (`precheck_save`); kódovanie beží na vlastnom vlákne. Čítanie obrázka zo schránky (`read_image`) beží mimo UI vlákna a rozlišuje „nie je obrázok / príliš veľký / nečitateľný formát“ (predtým jedna zavádzajúca hláška).
- **Náhľad po zatvorení okna (zmena oproti §8.23):** vyberový dialóg systému Nástroje zavrie, preto sa nahliadnutá snímka drží **len v pamäti**, najviac **5 minút** (časovač uvoľní pamäť), alebo kým ju nezahodíš / neuložíš / nevypneš snímky. Nikdy sa nezapíše na disk pred „Uložiť“. PRIVACY.md to uvádza.
- **Overené:** živý test platformy `capture_of_an_own_window_returns_its_pixels_and_honours_cancellation` (`#[ignore]`, otvorí na ~1 s vlastné okno 200×120): D3D11 → staging → pamäť vrátilo **presnú farbu stredu okna (`E0 80 20 FF`)**, snímanie 0,8 s, zrušenie príznakom = `Cancelled`; `shot_flow` (5 testov: jedna práca naraz, odovzdanie raz, zrušenie, TTL náhľadu, kontroly pred kódovaním, preklady chýb v 11 jazykoch); živý scenár `--test-tools` (9 nových kontrol, vložený výsledok snímania idú rovnakou cestou vlákno → `N_SHOT`): náhľad v otvorenom paneli, zatvorenie okna ho odloží, opätovné otvorenie ho obnoví, Zahodiť uvoľní aj odloženú kópiu, zrušenie oznámi a nič neuloží, výsledok pri zatvorenom okne otvorí panel na náhľade, **zlyhané uloženie náhľad zachová**, potvrdené uloženie zapíše platný PNG a náhľad uvoľní.
- **NEOVERENÉ (potrebuje človeka):** skutočné okno `GraphicsCapturePicker` (otvorí sa systémové UI), že `PickSingleItemAsync` funguje z vlákna MTA bez slučky správ (návrh to predpokladá; pri chybe vráti „zlyhalo“/„nepodporované“, nezablokuje lištu), správanie na Windows 10 < 2004, viac monitorov s rôznym DPI, minimalizované okno (časový limit 3 s), chránený obsah (DRM), HDR.

#### 3. Polica: poradie, „Pridať súbory“, výber podľa id, sieťové cesty
- **Zmena poradia** (nový `Shelf::move_item`, `Cmd::ShelfMove`): `Ctrl+↑/↓` aj tlačidlá „Posunúť hore / dole“ v paneli Police; pripnuté a nepripnuté sú samostatné skupiny (pripnuté sú vždy hore); poradie sa ukladá v indexe, **formát ostáva verzia 1** (staršie vydania ho načítajú); držaná kombinácia sa ignoruje (jeden stisk = jeden presun); na okraji skupiny hlásenie „Vyššie sa už nedá posunúť…“. **Vyhadzovanie podľa veku**, nie podľa pozície (**červený test** `eviction_after_a_reorder_drops_the_oldest_added_item_not_the_last_position`: pozičné vyhadzovanie zmazalo novšiu položku).
- **Výber a potvrdenie mazania podľa id** (nálezy z kontroly, overené: P / pridanie navrch / presun posúvali zvýraznenie na inú položku a druhý Delete mohol zmazať inú položku): `Row.id`, `selected_id`, `confirm_delete: Option<String>`.
- **Čítačka obrazovky:** názov riadku Police obsahuje „…, 2 z 7“ (kľúč `tools.pos`), po presune stavový riadok „Presunuté na pozíciu 3 z 7.“; MSAA nemá živé oblasti, takže oznámenie závisí od presunu fokusu (zmena indexu) – **Narrator/NVDA NEOVERENÉ**.
- **Pridať súbory** (tlačidlo „Pridať súbory…“, `Ctrl+O`, `Insert`): zdokumentovaný `IFileOpenDialog` (viac súborov, len súborový systém, `.lnk` sa nenahrádza cieľom, bez „nedávnych“) na vlastnom vlákne; panel sa pri otvorení dialógu zavrie ako pri každom okne; vybrané cesty idú **tou istou cestou ako pustenie** (kontrola ciest, duplicity, limity, odkaz alebo kópia podľa nastavení; originály sa nikdy nemenia), výsledok ako oznam na lište. Testy: príznaky, očistenie ciest, vytvorenie dialógu bez zobrazenia; živý scenár: simulovaný výsledok dialógu → „Added: 1“, originály nezmenené. **Skutočný dialóg NEOVERENÝ (človek).**
- **Esc pri premenovaní** snímky už nepotvrdzuje premenovanie (zatvorenie okna ho ruší; potvrdí Enter alebo Uložiť); vloženie (Ctrl+V) pri premenovaní je jednoriadkové a najviac 100 bajtov.
- **Sieťové cesty (UNC, WebDAV, namapované disky):** pracovné vlákno nástrojov už **nikdy** nevolá `symlink_metadata` ani `File::open` na ceste od používateľa priamo (nový modul `transom-tools::lane`): sondy idú na 2 pevné vlákna s limitom **250 ms** (neskorý výsledok sa uloží na ďalšie použitie), otvorenie súboru na kopírovanie s limitom **3 s**, uviaznuté vlákna sa už nezadávajú a front je ohraničený (64). Cesta, čo vyzerá sieťová (`\\server`, `\\?\UNC\`, `GetDriveTypeW == DRIVE_REMOTE`), sa **vôbec nesonduje**, zobrazí sa „sieťové umiestnenie, nekontrolované“ (nie „chýba“), kópia z nej sa odmieta (nová hláška), ako odkaz sa prijme a **zo siete sa nepretiahne von** (`SHParseDisplayName` by blokoval UI vlákno; namiesto toho vysvetlenie). Kopírovanie cez iné ako sieťové cesty (napr. `subst`) chráni len limit sond/otvorenia: **zostáva riziko**, že čítanie pomalého média uprostred kopírovania zdrží pracovné vlákno. **Červený stav** (pred opravou: synchrónne volanie na vlákne) a test `an_unreachable_shelf_path_never_delays_other_tools_and_is_listed_as_unchecked` (sonda visí 5 s na ceste s písmenom disku aj na UNC; poznámka vznikne < 1,5 s) plus 4 testy `lane`.
- **Nezmenené / neopravené:** ťahanie riadku na zmenu poradia myšou (koliduje s ťahaním von; zámerne len tlačidlá a klávesy), prah ťahania je 6 bodov (nie `SM_CXDRAG`), `DragOut` zatvorí panel aj pri zrušenom ťahaní.

#### 4. Vylúčené aplikácie (`excluded_apps`)
Nastavenia → Nástroje → „Aplikácie, z ktorých sa história schránky nikdy nezachytáva“: zoznam s tlačidlami „Odstrániť X“ a „Pridať aplikáciu…“ (výber `.exe` cez `GetOpenFileNameW`; **ukladá sa len názov súboru**, nie cesta). Validácia `transom_config::normalize_exe_name`: len `.exe`, bez zakázaných znakov, najviac 64 znakov, duplicity bez ohľadu na veľkosť písmen, najviac 20 položiek v UI; neplatný vstup sa nezapíše a zobrazí sa dôvod. Načítanie konfigurácie **nezahadzuje** záznamy bez `.exe` (tichý zahod by oslabil súkromie). Text uvádza, že ide o zhodu podľa názvu (best effort) a že ak transom nezistí vlastníka schránky a zoznam nie je prázdny, nezachytí sa nič. 9 nových textov × 11 jazykov, ovládače sú pomenované pre čítačku (skupina + názov). Testy: 2 v `transom-config`, 2 v `transom-settings`, audit prístupnosti všetkých stránok ostáva zelený.

#### 5. `panic = "abort"` a `catch_unwind` (rozhodnutie)
- **Zistenie (overené):** `panic = "abort"` je len v `[profile.release]`; `cargo test` a dev vždy odvíjajú, takže test „panika zberu neblokuje ďalší“ prechádzal, kým release exe by pri takejto panike zomrelo. Panika v UI vlákne alebo v `extern "system"` callbacku ukončí proces v **každom** profile. Ostatné pracovné vlákna (médiá, nástroje, schránka, zvuk) by po panike v režime unwind **ticho zomreli** pod živou lištou.
- **Meranie (release, `--locked`, scratch ciele):** `transom-app` 2 457 600 B (abort) → 3 239 424 B (unwind) = **+782 KB (+32 %)**; `transom-settings` 2 291 200 → 2 691 584 (+17 %); `transom-statusline` 840 704 → 910 336 (+8 %).
- **Rozhodnutie: ponechať `abort`**, zavádzajúce zotavenie **odstrániť**: `catch_unwind` v `ai.rs` (v release sa nikdy nevykonal), test premenovaný (zostal len test otráveného zámku), komentáre v `engine.rs`/`sources.rs`/`volume_check` opravené (izolácia zdroja vyhľadávača platí len v zostaveniach s odvíjaním, test je `#[cfg(panic = "unwind")]`). Parsery lokálnych logov sú chránené inak: poškodené riadky sa preskakujú a hodnoty saturujú (testy v `transom-usage`).
- **Hák paniky** (`install_panic_hook`): do `--log` zapíše jeden riadok „vlákno + miesto + verzia“ **bez textu paniky** (ten môže niesť cesty alebo obsah); bez `--log` nezapisuje nič. Test overuje, že tajný text nepadne do logu. Test `the_release_profile_keeps_panic_abort_as_documented` zlyhá, ak by sa profil zmenil bez opätovného posúdenia.
- **Pri páde:** AppBar sa po zabití procesu vracia (meranie v §8.2: po zabití procesu je pracovná plocha späť za 0,12 s, `tools/gate-appbar.ps1`); zostane len prízračná ikona v oblasti oznámení. Pád pri ďalšom štarte sa **neopakuje automaticky** (žiadny dohľadový proces).

#### 6. CI: referencie a postup
- `dtolnay/rust-toolchain@1.100.0` je **vetva** (nie tag), jej `action.yml` má verziu zadrôtovanú a nemá vstup `toolchain`; pripnutie na SHA vyžaduje commit z `master`. Namiesto tretej strany jobs spúšťajú **`rustup toolchain install --profile minimal --no-self-update`** bez argumentov (rustup ≥ 1.28 nainštaluje aktívnu verziu z `rust-toolchain.toml` aj s komponentmi) a overia `rustc -V` voči `channel` zo súboru: **verzia je len na jednom mieste.** Tento príkaz som spustil lokálne (rustup 1.29.3: „active toolchain 1.100.0 … overridden by rust-toolchain.toml“, `VERSION_OK`).
- Pripnuté SHA (overené `git ls-remote`, 2026-11-22): `actions/checkout` v7.0.1 = `3d3c42e5…` (Node 20 už nie je dostupný, preto nie v4), `Swatinem/rust-cache` v2.9.3 = `d1527c8f…` (peeled), `EmbarkStudios/cargo-deny-action` v2.1.2 = `70f81531…` (peeled; v2.1.3 je 3 dni stará). Runner `windows-2025` explicitne (`windows-latest` sa presúva postupne), `deny` na `ubuntu-24.04` (Docker akcia).
- Pasce ošetrené: `RUSTFLAGS` v `env` by potichu zrušilo `+crt-static` z `.cargo/config.toml` (cargo berie len jeden zdroj príznakov) – úloha `release-build` to kontroluje a hľadá `vcruntime140` / `api-ms-win-crt` v exe (lokálne overené na zostaveniach: čisté); `defaults.run.shell: bash` na Windows; `cache-on-failure`, `save-if` len na `main`; `--locked` v clippy/test/build; `cargo deny … --locked` (pozri záverečné kontroly).
- **NEOVERENÉ (nikdy nebežalo):** syntax YAML na GitHube (nebol k dispozícii YAML linter, súbor som kontroloval čítaním), `rustup` s `rust-toolchain.toml` na čistom runneri, `cargo-deny-action` s tokom `rustup` v kontajneri, čas ≈ 10 min app testov.

#### 7. Živé testy (`transom-app --test-tools`) a čo nebolo bezpečné spustiť
Beh používa **scratch** `TRANSOM_CONFIG` (s `tools.notes.max_notes = 1` na simuláciu zlyhaného uloženia) a `TRANSOM_DATA`, dve skutočné lišty (dva monitory; usporiadanie, DPI ani jazyk systému sa nemenili), syntetické súbory a obrázky. Do skutočnej schránky sa **nezapisuje a nič sa z nej nečíta**, nič sa nespúšťa ani nezatvára v iných aplikáciách. Vstup ide `PostMessage` priamo oknu popoveru tohto procesu.
**Výsledok: 62 kontrol prešlo, 0 zlyhalo** (release zostavenie z konca milníka; debug zostavenie 62/0 pred posledným drobným dolaďovaním). Oproti §8.23 (47) pribudlo 15 kontrol:
| Nová kontrola | Stav |
|---|---|
| zatvorenie okna odloží nepotvrdený náhľad len v pamäti a nič neuloží; opätovné otvorenie ho obnoví na karte Snímky; Zahodiť uvoľní aj odloženú kópiu | **ŽIVÉ – PASS** |
| výsledok snímania s otvoreným panelom ukáže náhľad bez ukladania; pri zatvorenom okne oznámi a otvorí panel na náhľade | **SIMULOVANÉ vstupom (výsledok vložený rovnakou cestou vlákno → `N_SHOT`) – PASS** |
| zrušenie výberu oznámi „Zachytenie zrušené. Nič sa neuložilo.“ a nič nepridá | SIMULOVANÉ – PASS |
| zlyhané uloženie (snímky vypnuté v konfigurácii) náhľad zachová; potvrdené uloženie po ňom zapíše platný PNG a náhľad uvoľní | ŽIVÉ – PASS |
| panel Police: tlačidlá „Pridať súbory… / Posunúť hore / Posunúť dole“ a pozície „n z N“ v názvoch pre čítačku | ŽIVÉ – PASS (MSAA uzly cez `a11y()`; **Narrator/NVDA NEVYKONANÉ**) |
| posun hore: nové poradie, zápis do indexu na disku, výber nasleduje položku, originály nezmenené; na okraji hlásenie a nič sa nemení | ŽIVÉ – PASS |
| výsledok „Pridať súbory“ ide cez rovnakú cestu ako pustenie (oznam „Added: 1“, položka v Polici, originál nezmenený) | SIMULOVANÉ dialóg – PASS |

**Nebolo bezpečné / možné spustiť (a presný ručný krok):**
- *Skutočný výber okna/obrazovky (`GraphicsCapturePicker`)*: otvára systémové UI a čaká na človeka. Krok: Nástroje → Snímky → „Zachytiť okno alebo obrazovku…“ → vyber okno → v náhľade „Uložiť snímku“; potom to isté a „Zahodiť“; potom v dialógu „Zrušiť“ (očakáva sa „Zachytenie zrušené“); nechaj výber otvorený > 2 min (očakáva sa „Zachytenie neprišlo včas“). Overiť aj minimalizované okno a monitor s iným DPI.
- *Skutočný dialóg „Pridať súbory…“*: vyber 2 súbory a 1 sieťový (ak je dostupný); očakáva sa oznam „Added: 2“ (sieťový ako „nekontrolované“), originály nezmenené, `Ctrl+↑/↓` a tlačidlá menia poradie, po reštarte lišty poradie ostane.
- *Skutočné ťahanie myšou dnu a von* (OLE `DoDragDrop`), *história schránky* (zachytávanie, pauza, súkromné formáty, vylúčená aplikácia z nového zoznamu – napr. pridaj `notepad.exe`, skopíruj text z Poznámkového bloku a over, že sa nezachytí), *tlačidlo „Uložiť obrázok zo schránky“* – všetko zapisuje/číta **skutočnú schránku**; robiť len na vlastnom stroji.
- *Narrator/NVDA*, *IME*, *skutočný studený štart Windows Search*.
- Ako to spustiť bezpečne: ukonči bežiacu lištu (jedna inštancia naraz), nastav `TRANSOM_CONFIG` a `TRANSOM_DATA` na dočasné priečinky (`%TEMP%\transom-accept\…`), spusti `transom-app.exe`, po teste ho ukonči a spusti normálnu lištu. Skutočné dáta v `%APPDATA%\transom` a `%LOCALAPPDATA%\transom` sa tak nezmenia.

#### 8. Zostávajúce prekážky vydania, schválenia a ďalší míľnik
**Čo treba od teba na spustenie CI:** súhlas na (1) vytvorenie **súkromného** repozitára na GitHube (`gh repo create <meno> --private --source . --remote origin`), (2) `git push` aktuálnej vetvy (vrátane commitu; zmeny sú teraz len v pracovnom strome – podľa zadania som necommitoval), (3) ručné spustenie workflow (`workflow_dispatch`) a odovzdanie logu. Očakávané prvé zlyhania: layout audity (písma v 11 jazykoch na hosťovanom runneri), časovanie testov, `cargo-deny-action` v kontajneri, čas ≈ 10 min app testov.
**Prekážky:** (1) CI nikdy nebežalo; (2) chýba ľudské overenie: výber okna/obrazovky, dialóg súborov, OS ťahanie, história schránky a vylúčenie aplikácie, čítačky obrazovky (kroky vyššie); (3) snímanie je len celé okno/obrazovka so žltým okrajom (bez výberu výrezu, SDR) – produktové rozhodnutie; (4) história schránky kóduje PNG **pred** kontrolou `max_image_mb` (vstup je ohraničený 64 MiB DIB a stranou 8192 a pri vypnutých obrázkoch sa nenačíta nič; presnú veľkosť PNG pred kódovaním nemožno zistiť); (5) kopírovanie z lokálne vyzerajúcej, no pomalej cesty (`subst`, junction na sieť, pomalé médium) môže stále zdržať pracovné vlákno nástrojov uprostred čítania (limitované je len otvorenie a sondy); (6) pád v release (`panic = "abort"`) ukončí lištu bez automatického reštartu; (7) chýba vložený manifest (PerMonitorV2, longPathAware) a `VS_VERSION_INFO`; (8) Polica sa myšou neradí ťahaním (zámerne, kolízia s ťahaním von), prah ťahania nie je `SM_CXDRAG`; (9) zakázané snímky vyvolajú všeobecnú hlášku o ukladaní (nie osobitnú „snímky sú vypnuté“); (10) `docs/PUBLISHING.md` obsahuje zastarané čísla (commity), pred publikovaním znova zmerať.
**Odporúčaný ďalší míľnik:** „Ľudské prijatie a prvé CI“ – (a) tvoj súhlas → súkromný remote → CI → opravy rozdielov prostredia; (b) prejsť ručné kroky z bodu 7 s tebou (výber okna, dialóg súborov, ťahanie, schránka, Narrator); (c) rozhodnúť o výrezovom snímaní (Win+Shift+S zostáva ručne) a o vloženom manifeste; (d) potom prvé súkromné zostavenie na rozdávanie testerom (bez tagu a vydania).

#### 9. Záverečné kontroly (2026-11-22, `--locked`)
`cargo fmt --all -- --check` čistý; `cargo clippy --workspace --all-targets --locked -- -D warnings` bez nálezov (prvý beh našiel 3 nálezy v novom kóde – `chunks_exact_mut`, `result_unit_err`, `unnecessary_lazy_evaluations` – opravené); `cargo test --workspace --locked --no-fail-fast`: **690 testov prešlo, 0 zlyhalo, 7 ignorovaných** (oproti 652 v §8.23: +38; ignorované: 5 pôvodných, `measure_index_*` a nový živý `capture_of_an_own_window_…`, ktorý som spustil samostatne s `--ignored`: PASS). Beh šiel do čistého scratch `CARGO_TARGET_DIR`, lebo zatúlaný testovací proces z predchádzajúceho (zastaveného) behu držal výstup linkera (`LNK1104`); jeho zastavenie mi systém nepovolil, preto som ho nechal bežať. `cargo deny check` aj `cargo deny --locked check`: advisories, bans, licenses, sources v poriadku; `cargo build --release --locked`: v poriadku, statické CRT (kontrola z CI lokálne čistá, `transom-app.exe` 2 459 136 B); živý scenár `--test-tools` v release: **62/62**. **`Cargo.lock` sa nezmenil** (nové funkcie crate `windows` nepridali žiadnu závislosť). Zmeny **nie sú commitnuté** (podľa zadania).

### 8.25 Míľnik: Snímky ako knižnica existujúcich snímok a ikony vo výsledkoch vyhľadávania (2026-11-22)

Oprava zadania: **„Snímky“ nie je nástroj na snímanie**, ale knižnica snímok, ktoré už sú uložené na počítači (najnovšie prvé, miniatúra, názov, dátum). Vyhľadávač má mať pri každom výsledku ikonu. Bez pushu, bez remote, bez commitu; zmeny sú len v pracovnom strome.

#### 1. Čo sa odstránilo a čo sa zachovalo
- **Odstránené** (celý tok z §8.24.2): výber okna/obrazovky (`transom-platform::capture`, `shot_flow.rs`, funkcie `Graphics_Capture`/Direct3D11/WinRT v `windows`), náhľad, ukladanie, premenovanie (F2), mazanie, tlačidlo „Uložiť obrázok zo schránky“ (`clipboard::read_image`), úložisko `transom-tools::shots` (kvóty, retencia) a príkazy `Shot*` v službe nástrojov, kľúče prekladov `tools.shot_*` (11 jazykov) a tlačidlo „Vymazať snímky“ v Nastaveniach (`clear-screenshots.req` sa už nespracúva). **Nie je to „budúca samostatná funkcia“**: kód neostal v strome. Pretože nebol nikdy commitnutý (git ho nemá), **kópia je mimo repozitára** v `C:\Projects\transom-attic\capture-flow-2026-11-22\` (`capture.rs`, `shot_flow.rs`, `shots.rs` a diff sledovaných súborov); môžeš ju zmazať.
- **Staré dáta:** ak skoršie zostavenia uložili snímky do `%LOCALAPPDATA%\transom\tools\screenshots\`, ostávajú na disku **nedotknuté a nepoužité** (nič ich nemaže ani nemigruje; PRIVACY.md to uvádza).
- **Žiadne mazanie, premenovanie ani pripnutie** v sekcii Snímky (Delete, P a F2 nerobia nič; test to kontroluje). Ponuka: **Otvoriť** (predvolená aplikácia), **Ukázať v priečinku**, **Obnoviť** (F5); ťahanie von ostáva len ako kópia (`DROPEFFECT_COPY | LINK`, nikdy presun).

#### 2. Kde sú snímky: výskum a rozhodnutie
- **Čo je zdokumentované** (Microsoft Learn, stránky som čítal sám cez nástroj na čítanie webu): `FOLDERID_Screenshots` je známy priečinok (PERUSER, predvolene `%USERPROFILE%\Pictures\Screenshots`, od Windows 8). Pozor: pri čítaní stránky KNOWNFOLDERID bol jej koniec skrátený, takže **riadok pre Screenshots som videl len v zhrnutí, nie doslovne**; GUID preto neprepisujem, beriem konštantu z crate `windows` (zhodnú s `{b7bede81-df94-4682-a7d8-57a52620b86f}`). `Pictures` a `Camera Roll` doslovne potvrdené.
- **Postup:** `SHGetKnownFolderPath(FOLDERID_Screenshots, KF_FLAG_DONT_VERIFY)` pri **každom otvorení** karty (sleduje presmerovanie Obrázkov na OneDrive aj vlastné presmerovanie priečinka; `KF_FLAG_DEFAULT_PATH` by vrátil nepresmerovanú predvolenú, preto sa nepoužije; priečinok sa nikdy nevytvára). Ak ho systém nevie určiť: `FOLDERID_Pictures` + `Screenshots`. Cesta sa **nezapisuje do konfigurácie** (ukladá sa len prepínač „zahrnúť priečinok Windows“ a vlastné priečinky).
- **Čo sa nedá spoľahlivo zistiť:** vlastné miesto ukladania Snipping Toolu, Xbox Game Bar (`Videos\Captures`), ShareX atď. nemajú zdokumentované API; nečítam nedokumentované nastavenia ani registre. Na to je **zoznam vlastných priečinkov** (Nastavenia → Nástroje, najviac 8, výber systémovým dialógom, validácia: absolútna cesta, nie zariadenie, nie koreň disku). Plytké čítanie, **nikdy rekurzia ani celý disk**.

#### 3. Architektúra a limity
- **Zoznam (`transom-tools::library`, čistá logika):** pripony png/jpg/jpeg/bmp/gif/webp/tif/tiff; čas, veľkosť a atribúty sa berú z vymenovania priečinka (súbor sa neotvára); symbolické odkazy a priečinky sa preskočia; najviac `max_items` najnovších (predvolene 300, ohraničený zásobník `2 × max_items`), najviac 100 000 prejdených položiek na priečinok (`Capped`), pri orezaní hlásenie „zobrazených {0} z {1}“; poradie: čas zmeny zostupne, pri zhode názov (stabilné); duplicity priečinkov (veľkosť písmen, koncový `\`, predvolený = pridaný) sa zlúčia.
- **Vlákna a časové limity:** každý priečinok sa skenuje na vlastnom vlákne s limitom 4 s; nedostupný sieťový priečinok označí `Slow` bez zdržania ostatných priečinkov ani iných nástrojov; opustené vlákno sa pre ten istý priečinok nespúšťa druhýkrát. Knižnica má vlastné vlákno (nezdieľa pracovné vlákno nástrojov).
- **Obnova (`transom-platform::dirwatch`, `ReadDirectoryChangesW`, asynchrónne):** beží **len kým je karta Snímky otvorená**; zatvorenie okna uvoľní uchytky. Uchytka má zdieľanie READ|WRITE|DELETE (test: priečinok sa dá premenovať počas sledovania a zmazať po zatvorení). Buffer 32 KB (sieť ≤ 64 KB). Nulový počet bajtov (pretečenie) a `ERROR_NOTIFY_ENUM_DIR` = úplné nové vymenovanie; iná chyba = „nedostupný“ + opakovanie s narastajúcim odstupom (2–30 s, nikdy tesná slučka); chýbajúci priečinok sa čaká v **bezprostrednom** rodičovi (nikdy vyššie). Série udalostí sa zlučujú (350 ms ticha, najviac 1,5 s). **Známe:** premenovanie samotného sledovaného priečinka systém nehlási (dokumentácia: „does not report changes to the specified directory itself“) → treba Obnoviť (F5) alebo znova otvoriť kartu.
- **Miniatúry (`transom-platform::thumb`, WIC; `transom-core::loader`):** vlastné vlákno, lenivo len pre viditeľné riadky (+ jeden pod nimi), fronta ≤ 24, posledná mnozina vyhráva; rozmery z hlavičky (`dims_ok`: ≤ 16384 px, ≤ 64 Mpx) a veľkosť súboru (≤ 64 MiB) sa overia **pred** alokáciou, dekódovanie ide cez škálovač rovno do bufra miniatúry (nikdy celý obrázok); súbor sa otvára vlastnou uchytkou so zdieľaním READ|WRITE|DELETE a zatvára hneď (používateľ ho môže kedykoľvek zmazať/premenovať). Cache **len v pamäti, 24 MiB**, kľúč (cesta, čas zmeny, veľkosť, rozmer) → zmenený súbor aj iný DPI sa nahradia; zmazané súbory opúšťajú cache po novom vymenovaní; zlyhania sa zapamätajú (nepokúša sa znova pri každom kreslení) a pri ďalšom otvorení karty sa skúsia znova; po zatvorení karty sa miniatúry zahodia. **Bez miniatúry** (zástupná ikona): súbory „len online“ (`OFFLINE`/`RECALL_ON_OPEN`/`RECALL_ON_DATA_ACCESS`; nesťahujú sa), súbory v sieti (UNC/namapovaný sieťový disk), poškodené/nepodporované súbory, príliš veľké.
- **Riadok:** 64 lp (miniatúra 76×48 lp, DPI-škálovaná, posun po klávesnici berie výšku zo spoločného miesta), názov, dátum, veľkosť; čítačka obrazovky číta „názov, dátum“ a veľkosť; tlačidlá Otvoriť / Ukázať v priečinku / Obnoviť; stavové riadky: načítavam, „{0} priečinkov sa nepodarilo načítať“, „zobrazených {0} z {1}“, „iba na čítanie“.
- **Ikony v hľadaní (`transom-app::icon_rt`, `transom-platform::shellicon`, `transom-render::resulticons`):** aplikácie `shell:AppsFolder\...` cez `IShellItemImageFactory(SIIGBF_ICONONLY)`; `.lnk` cez `IShellLinkW::GetIconLocation`/`GetPath` (ikona **cieľa bez šípky odkazu**, bez `Resolve`); `.exe`/`.ico`/`.url`… zo súboru **len ak je lokálny, plne dostupný a nie je cloudový**; ostatné súbory podľa **typu** (`SHGetFileInfoW(SHGFI_USEFILEATTRIBUTES)` + `SHDefExtractIconW`, jedna položka cache na príponu, súbor sa neotvára); priečinok `SHGetStockIconInfo(SIID_FOLDER)`; nastavenia, akcie, poznámky, Polica = vstavané vektorové ikony (rovnaký jazyk ako ikony nastavení, ostré v každej veľkosti). Sieťové cesty, cloudové a chýbajúce súbory idú rovno na ikonu podľa prípony. Ikona sa načíta v **presnej veľkosti v pixeloch** (24 lp × DPI), takže je ostrá; kým sa nenačíta alebo ak zlyhá, kreslí sa vstavaná neutrálna ikona druhu výsledku. Načítanie je asynchrónne (jedno vlákno, volania shellu sa radia, lebo súbežné volania pri prvom použití zlyhávali – namerané), kreslenie nikdy nečaká, `end()` nahrádza čakajúci zoznam poslednou viditeľnou množinou. **Náhľady obrázkov sa v hľadaní zámerne nepoužívajú** (načítanie obsahu súborov nie je lacné). Poradie, hodnotenie, klávesnica a rušenie sa nezmenili (`Item`/`Hit`/`transom-launcher` sa nedotkli, ikona len prekresľuje okno).
- **Súkromie:** cesty a názvy sa nikde nelogujú (nové moduly nič nelogujú); zoznam aj miniatúry sú len v pamäti.

#### 4. Overenie
Všetko bežalo so scratch priečinkami (`%TEMP%`, scratchpad); **skutočný priečinok snímok používateľa sa v žiadnom teste ani živom behu nečítal** (v scenári je predvolený priečinok Windows vypnutý) a testy nevypisujú cesty ani názvy.

| Oblasť | Test | Stav |
|---|---|---|
| Zoznam | `transom-tools::library` (10): pripony a poradie (pri zhode čas → názov), orezanie na K najnovších s pravdivým počtom, strop 100 000 položiek, cloud/sieť príznaky, validácia a zlúčenie priečinkov (nie koreň disku, nie zariadenie), zlúčenie duplicít, **skutočný `FsLister` na disku** (súbory sa nemenia, chýbajúci priečinok sa nevytvorí), otvorenie → sledovanie → zatvorenie uvoľní sledovanie, séria 20 udalostí = 1 nové vymenovanie, pretečenie a nedostupný priečinok = úplné vymenovanie, trvalý prúd udalostí neblokuje obnovu, nedostupný priečinok vyprší bez zdržania ostatných a nespúšťa sa naraz dvakrát, chýbajúci predvolený priečinok nie je „problém“ | **ŽIVÉ na disku** (scratch) + falošné vymenovanie/sledovanie pre časovanie |
| Sledovanie | `transom-platform::dirwatch` (5): mapovanie výsledku (nulové bajty, `ERROR_NOTIFY_ENUM_DIR`, iná chyba), nová položka vyvolá zmenu, **priečinok sa dá premenovať počas sledovania**, zmazaný priečinok = jedno hlásenie bez slučky a po návrate nové vymenovanie, priečinok, ktorý nikdy neexistoval, sa nevytvorí, po zahodení sa uvoľní uchytka (priečinok sa dá zmazať) | **ŽIVÉ** (`ReadDirectoryChangesW`) |
| Miniatúry | `transom-platform::thumb` (4): veľkosť/pomer/bez zväčšovania, PNG so správnou orientáciou a farbami, súbor sa nezmení a po dekódovaní sa dá premenovať/zmazať, priehľadnosť, BMP, chybné/prázdne/zamknuté/priečinok/> 64 MiB/hlavička 30 000×30 000 = druh chyby bez paniky | **ŽIVÉ** (WIC) |
| Cache a načítavač | `transom-core::loader` (4): cache ohraničená v bajtoch a LRU, zlyhania sa pamätajú aj zabúdajú, dekódovanie mimo volajúceho vlákna, zaseknutý dekóder neblokuje `get`/`want`, posledná množina vyhráva, `clear` zahodí výsledky rozbehnutých úloh, fronta je ohraničená | automatické |
| Celé zapojenie | `library_rt` (2): skutočné PNG v scratch priečinku → zoznam od najnovšieho → lenivé miniatúry (nič sa nedekóduje, kým panel nepožiada) → nový súbor sa objaví → zmazaný zmizne aj z cache → po zatvorení sa uvoľní priečinok a miniatúry | **ŽIVÉ** (všetky skutočné súčasti) |
| Panel | `tools_tests` (+3): Otvoriť / Ukázať v priečinku / Obnoviť (myš aj F5, Ctrl+Enter), **F2, Delete a P nerobia nič**, žiadne tlačidlá snímania ani ukladania, názov riadku pre čítačku nesie dátum, hlásia sa len viditeľné riadky bez miniatúry, plocha miniatúry sleduje DPI, posun po klávesnici pri vyšších riadkoch, audit vzhľadu vo všetkých 11 jazykoch × 3 DPI × 2 témy × (riadky, prázdne, vypnuté) | automatické |
| Nastavenia | `tools_page` (+2): prepínač predvoleného priečinka, validácia/zlúčenie/limit/odstránenie priečinkov, zdvojené ovládače majú názov skupiny v 11 jazykoch, **medzi vymazateľnými úložiskami nie sú snímky**; `config`: nové kľúče, staré `max_mb`/`retention_days` sa ticho ignorujú | automatické |
| Ikony | `transom-platform::shellicon` (7): **skutočné ikony** typu, priečinka, `.exe`, aplikácií zo zložky Aplikácie (≥ 6 z 12), chýbajúce/poškodené/zamknuté/zariadenie → ikona podľa typu, **odkaz ukazuje ikonu cieľa bez šípky**, orientácia a alfa zhodné s cestou cez `HICON` (test rozlišuje prevrátené riadky), 80 × 4 načítaní bez úniku GDI objektov, v každej veľkosti 16–48 px; `icon_rt` (3): zdroj podľa druhu, nepriateľské a sieťové cesty sa nikdy nečítajú, skutočné načítanie asynchrónne (odkaz, exe, bežné typy, priečinok, chýbajúce súbory, poškodený exe, zamknutý súbor, neexistujúci UNC server) v 24 aj 36 px, fronta zahodí riadky, ktoré sa posunuli preč; `launcher_pop` (+2): presná veľkosť ikony podľa DPI (96/120/144/192), pýta sa len pre kreslené riadky, názvy pre čítačku sa nezmenili, audit vzhľadu s ikonou; `resulticons` (2) | **ŽIVÉ** (shell) + automatické |

**Zistenia počas práce (každé zachytené testom alebo meraním):** (1) súbežné prvé volania `SHGetFileInfoW`/`SHDefExtractIconW` z viacerých vlákien **zlyhávali** (5 z 6 behov červených) → volania shellu sa radia za sebou, potom 4 zo 4 zelených; (2) chýbajúci UNC server stojí `IShellItemImageFactory` ≈ 2,1 s → sieťové cesty sa k nemu nikdy nedostanú (ikona podľa prípony); (3) predpoklad „premenovanie sledovaného priečinka sa ohlási“ **bol nesprávny** (uchytka nasleduje priečinok, nie názov) → zdokumentované, Obnoviť/znovu otvoriť; (4) útržok PNG sa dekóduje čiastočne (nie chybou) → kľúč cache s časom a veľkosťou nahradí miniatúru po dokončení zápisu; (5) pôvodný audit s 12 riadkami hlásil „Offscreen“ pre riadky pod oknom → audit ide na vysokej ploche ako ostatné zoznamy.

#### 5. Čo je ŽIVÉ, čo SIMULOVANÉ a čo treba spraviť človeku
**Živý scenár `--test-tools`** (release z čistého `CARGO_TARGET_DIR`, scratch `TRANSOM_CONFIG`/`TRANSOM_DATA`, dve skutočné lišty, nič iné nebežalo): **57 kontrol prešlo, 0 zlyhalo** (62 v §8.24 mínus 15 kontrol snímania, plus 9 nových: 8 knižnica snímok + 1 ikony). Nové kontroly:
- ŽIVÉ: karta Snímky vypíše 4 obrázky od najnovšieho a preskočí `.txt`; panel vystavuje tlačidlá a riadky s dátumom pre čítačku (MSAA uzly, **Narrator/NVDA NEVYKONANÉ**); 3 zo 4 miniatúr sa načítali na pozadí a poškodený súbor má zástupnú ikonu; **nový súbor sa objaví bez stlačenia Obnoviť**, zmazaný zmizne; Obnoviť zachová zoznam a **súbory sa nezmenili (bajty ani čas)**; **po zatvorení okna sa sledovanie zastaví a priečinok sa dá premenovať**; pri hľadaní sa načítalo 18 ikon na pozadí a **UI vlákno sa nezablokovalo** (najväčšia medzera medzi 20 ms tepmi 47 ms).
- SIMULOVANÉ (ako v §8.24): pustenie súborov na lištu (OLE), výsledok dialógu „Pridať súbory“, studený štart indexu (6 s).
- **NEKLIKALO sa** na „Otvoriť“ a „Ukázať v priečinku“ (spustili by inú aplikáciu na ploche); logika je pokrytá testami príkazov.

**Čo som NEVYKONAL a treba to spraviť na tvojom stroji** (zatvor bežiacu lištu a spúšťaj so scratch `TRANSOM_CONFIG`/`TRANSOM_DATA`, ak nechceš meniť nastavenia):
1. Nástroje → Snímky na **skutočnom** priečinku: vytvor snímku (Win+PrtScn / Snipping Tool) a over, že sa objaví bez Obnoviť, má správnu miniatúru a dátum; zmaž ju v Prieskumníkovi (zmizne) a premenuj ju (zmení sa názov). Pozn.: zmenu Obrázkov na OneDrive over na stroji, kde to je zapnuté.
2. „Otvoriť“ (Enter) a „Ukázať v priečinku“ (Ctrl+Enter): otvorí sa obrázok / Prieskumník s vybraným súborom; Prieskumník nesmie byť blokovaný pri zmazaní súboru pri otvorenej karte.
3. Priečinok s tisíckami obrázkov a sieťový priečinok (v zozname má byť „bez náhľadu“, lišta sa nesmie spomaliť), súbor „len online“ z OneDrive (nesmie sa stiahnuť).
4. Presunúť lištu medzi monitormi s rôznym DPI počas otvorenej karty (miniatúry a ikony sa majú načítať znova ostro).
5. Vyhľadávač: napísať „note”, „calc”, názov odkazu zo Štart ponuky, priečinok a súbor `.pdf`/`.docx`/`.png`; ikony majú pribúdať bez zdržania písania; kláves hore/dole a Enter fungujú ako predtým.
6. Narrator/NVDA: riadky Snímok („názov, dátum“, veľkosť) a výsledky vyhľadávania.

#### 6. Zostávajúce prekážky
1. **GitHub CI nikdy nebežalo** (repozitár nemá remote) a **chýba ľudské prijatie** (kroky vyššie, drag-and-drop, dialóg súborov, schránka, čítačky).
2. **Iné miesta ukladania snímok sa nedajú spoľahlivo zistiť:** vlastný priečinok Snipping Toolu, Xbox Game Bar, ShareX atď. nemajú zdokumentované API; používateľ ich pridá v Nastaveniach (najviac 8).
3. Miniatúry: orientácia EXIF sa nerešpektuje; formáty ako WebP/HEIC fungujú len s nainštalovaným kodekom WIC; súbory v sieti/cloude/príliš veľké majú len zástupnú ikonu.
4. `max_items` (predvolene 300, 20–2000) nemá ovládač v Nastaveniach (len konfigurácia); premenovanie sledovaného priečinka sa neohlási (Obnoviť / F5).
5. Ikony vyhľadávača: jedno vlákno a volania shellu sa radia, takže **zaseknuté volanie shellu na lokálne vyzerajúcej, no pomalej ceste (napr. `subst`) zastaví len načítavanie ikon** (UI a písanie nie; ostatné ikony ostanú vstavané); náhľady obrázkov sa v hľadaní nepoužívajú (zámerne).
6. Zdedené z §8.24.8: pád v release (`panic = "abort"`) ukončí lištu bez automatického reštartu, chýba vložený manifest a verzia, `docs/PUBLISHING.md` obsahuje zastarané čísla.

**Odporúčaný ďalší míľnik:** „Ľudské prijatie a prvé CI“: súhlas na súkromný remote → CI; prejsť kroky 1–6 vyššie; rozhodnúť o prípadnom ovládači `max_items` a o vloženom manifeste.

#### 7. Záverečné kontroly (2026-11-22, `--locked`)
`cargo fmt --all -- --check` čistý; `cargo clippy --workspace --all-targets --locked -- -D warnings` bez nálezov (nálezy počas práce: `manual_is_multiple_of`, `type_complexity`, `doc_lazy_continuation`, `chunks_exact_to_as_chunks`, nepoužitá metóda, všetky opravené); `cargo test --workspace --locked --no-fail-fast`: **719 prešlo, 0 zlyhalo, 6 ignorovaných** (v §8.24: 690 / 0 / 7; ignorované sú 5 pôvodných v `transom-app` a `measure_index_*`; ignorovaný živý test snímania zanikol s funkciou); `cargo deny --locked check`: advisories, bans, licenses, sources v poriadku; `cargo build --release --locked` do čistého cieľa: v poriadku (`transom-app.exe` 2 522 624 B, predtým 2 459 136 B); živý scenár `--test-tools`: **57/57**; **`Cargo.lock` sa nezmenil** (odstránením snímania sa zmenšili len zapnuté funkcie crate `windows`). Zmeny **nie sú commitnuté** (podľa zadania).

### 8.26 Oprava: lišta ostala viditeľná nad hrou, ktorá už bežala (2026-11-22)

**Nález (od používateľa):** pri spustenej hre na celú obrazovku bola lišta viditeľná. **Príčina (čítanie kódu + časová os, nie reprodukcia starej zostavy):** `create_bar` lištu vždy zobrazil a skrývanie sa spoliehalo len na `ABN_FULLSCREENAPP`, ktoré systém posiela **pri zmene** stavu. Lišta spustená (alebo znovu vytvorená po „Skryť lištu“ či pri pripojení monitora), kým fullscreen aplikácia už bežala, zostala viditeľná, kým sa stav znova nezmenil. V tomto prípade hra (TFT, UnrealWindow) bežala od 18:26 a lišta bola spustená o 19:45. Je to stará medzera z §8.14 (S3b), nie dôsledok zmien v tomto míľniku (`appbar.rs` sa v pracovnom strome predtým nemenil).

**Oprava (`transom-platform::fullscreen`, `appbar.rs`):** pri vzniku lišty sa stav zistí aj priamo: systém musí hlásiť fullscreen (`SHQueryUserNotificationState` = `QUNS_BUSY` alebo `QUNS_RUNNING_D3D_FULL_SCREEN`, rovnaký zdroj ako Prieskumník) **a** popredné okno nesmie byť shell ani transom, nesmie byť minimalizované a musí pokrývať **celý monitor tejto lišty** (druhý monitor sa nedotkne). Takáto lišta sa neukáže (z-poradie dole, `Fullscreen{opened:true}` pre aplikáciu) a každé 2 s sa skontroluje, či fullscreen skončil (časovač beží len kým taká lišta existuje); `ABN_FULLSCREENAPP` ju prevezme a má prednosť. Rozhodnutie je konzervatívne: pri chybe alebo nejasnosti lišta ostáva viditeľná, nikdy nie skrytá navždy.

**Overenie:** 3 testy (`covers`, `decide` v tabuľke: hra na hlavnom monitore skryje len lištu tohto monitora, plocha/panel úloh/okná transom/minimalizované/neviditeľné/okno v okne/systém bez fullscreenu nikdy; živé dotazy sa nezrútia). **ŽIVÉ na tomto stroji:** lišta spustená, kým hra TFT bežala na celú obrazovku (`QUNS_BUSY`, popredné okno 2560×1440 na monitore lišty), zostala **neviditeľná** (`IsWindowVisible` = false, nad hornými pixelmi je hra). **NEOVERENÉ:** návrat lišty po skončení fullscreenu (cesta časovača aj `ABN_FULLSCREENAPP` false) na živej hre, klasický exkluzívny fullscreen, hra na druhom monitore, vplyv na výkon hry.

### 8.27 Míľnik: Prémiový vizuálny prechod – písmo, tokeny, karty, jednotné tlačidlá (2026-11-22)

**Zadanie:** jednotný, uhladený vzhľad Windows 11 pre lištu, nastavenia, menu, popovery (AI, kalendár, Zvuk), Nástroje (Polica, Schránka, Poznámky, Snímky) a Hľadanie. Funkcie, dáta ani predvolené nastavenia súkromia sa nemenia. Dizajnový systém je opísaný v `docs/DESIGN.md`; tu je, čo sa zmenilo a čo je overené.

**Postup:**
1. Snímky „pred“: nastavenia v en pri 100/150/200 %, de, ja; nová galéria aplikácie `gallery_*` pri 100/150/200 % v oboch témach, iba z vymyslených dát.
2. Meranie písma cez GDI.
3. Dvaja nezávislí recenzenti len na čítanie (vizuál, prístupnosť) nad snímkami „pred“ a návrhom tokenov. Ich zistenia som zosúladil skôr, než som upravil zdieľané súbory.
4. Uloženie zlatého výpisu prístupnosti.
5. Implementácia po vrstvách (písmo → tokeny → nastavenia → popovery), po každej vrstve audity.
6. Snímky „po“ a druhé kolo recenzií.

Pred prvou úpravou bola záloha zdrojov v `C:\Projects\transom-attic\pre-design-2026-11-22\` (mimo repozitára).

**Písmo (zmerané):**
- Segoe UI Variable je na tomto stroji nainštalované. GDI vystavuje pomenované inštancie Small / Text / Display (Regular, Semibold) pod skrátenými 31-znakovými názvami.
- Pre všetky inštancie existujú záznamy `FontLink\SystemLink` (japončina a čínština dostanú náhradné písmo).
- **Doterajšie „polotučné“ písmo bolo v skutočnosti Bold:** Segoe UI s váhou 600 pri 14 px meria 183 px, „Segoe UI Semibold“ 163 px.
- Chýbajúce písmo GDI ticho nahradí Arialom. Dostupnosť sa preto overuje cez `GetTextFaceW`.
- Číslice Segoe UI Variable aj Segoe UI Semibold sú proporcionálne („1“ má 6 px namiesto 8 px), Segoe UI Bold ich má rovnako široké.
- **Celá lišta preto ostáva na klasickom Segoe UI** (`Font::Tabular`):
  - prekreslenie hodín po sekundách na mieste ráta s rovnakou šírkou číslic; percentá a odpočty majú stabilnú šírku;
  - pri zmiešaní by v lište stáli dva tvary číslic vedľa seba (nález druhého kola);
  - v sivom vyhladzovaní priehľadnej lišty je Variable pri 13 px tenšie a bledšie.
- Text so znakmi CJK sa kreslí klasickým Segoe UI s pôvodnou váhou. Prepojenie písiem pre inštancie Variable (hlavne Semibold) dávalo čínskym a japonským znakom inú veľkosť a rozostupy – regresia zachytená v druhom kole a opravená.
- Pravidlo: Small ≤ 12 lp, Text 13–19, Display ≥ 20, skutočný Semibold, záloha Segoe UI + Segoe UI Semibold. Rovnaké písmo dostali aj natívne polia nastavení.

**Tokeny a komponenty (`transom-render`):**
- `design.rs`:
  - typografická stupnica (24/32 nadpis, 15/22 sekcia, 13/20 text, 12/18 ovládač a vysvetlivka, 11/16 metaúdaje), rozostupy, polomery, výška ovládača 32 lp;
  - `look()` pre päť druhov tlačidiel × stavy, `button`, `card`, `checkbox`, dvojfarebný `focus_ring` a `focus_ring_inset`;
  - 4 testy (kontrast textu v každom druhu a stave na okne aj karte, odlišné stavy, výber a nebezpečenstvo nie len farbou).
- `theme.rs`: nové tokeny `surface`, `control*`, `accent_hover/pressed`, `selected`, `danger*`.
  - Svetlé `muted`, `accent_text` a `low` sú o odtieň tmavšie, aby text prešiel 4,5 : 1 aj na `pressed`.
  - Test kontrastu pokrýva všetky plochy, na ktorých sa kreslí text.
- `fonts.rs`: výber písma, overenie dostupnosti, `--font classic` / `TRANSOM_GALLERY_FONT=classic` na kontrolu zálohy; 3 testy.
- `record.rs`: záznam kreslenia, aby sa karta sekcie nakreslila pod už rozložený obsah (bez druhého rozkladania a bez blikania); 1 test.
- `painter::wrap_text`: úsek japončiny/čínštiny pokračuje na rozpísanom riadku. Predtým ostávali osamotené „この PC“, „transom“, „Windows Search“ (pozri test).

**Nastavenia:**
- **Rozloženie a typografia:**
  - plošná hlavička a navigácia (vybraná položka = plocha + akcentová značka + Semibold);
  - sekcie stránok v kartách s nadpisom; nadpis stránky 24 Semibold so zalamovaním;
  - vysvetlivky 12 lp s medzerou medzi odsekmi; popisky políčok sa so stavom nestanú tučnými;
  - závislé voľby odsadené o 32 lp.
- **Tlačidlá:**
  - druh podľa významu ovládača (`button_kind`); akcent je len hlavná akcia;
  - mazanie dát má červený okraj a ikonu koša, potvrdenie červenú výplň a znak varovania;
  - tlačidlá majú šírku podľa textu a zalamujú sa (`flow_buttons`). Tým sa opravilo **orezané „Zrušiť“** pri potvrdení zmazania dát AI aj archívu (nález oboch recenzentov).
- **Riadky volieb:** merajú sa vybraným štýlom, sú vyvážené (5 = 3 + 2) a ich popisok sa zalamuje.
- **Ďalšie prvky:** informačný pruh pre varovania (AI položka skrytá v lište, záloha monitora, chýbajúci zdroj archívu, len na čítanie); vzorky farieb (vybraná = akcentový rámik + značka); zoznamy priečinkov a aplikácií ako riadky.
- **Hlavička okna:** v neaktívnom okne sú znaky aj názov `muted` namiesto `disabled` (1,8–2,0 : 1); fokus na červenom „Zavrieť“ má bielu vnútornú čiaru.
- **AI stránka:** karty s nadpismi „Zdroje údajov“ a „Uložené údaje“ (nové kľúče `aip.sources_h`, `aip.data_h` v 11 jazykoch); obnova údajov je v karte „Ukazovateľ v lište“. Riadky volieb majú na všetkých stránkach jeden stĺpec popisku (180 lp).
- **Vzorky farieb:** obrys `muted` (≥ 3 : 1). Prvý návrh s `control_edge` (1,55 : 1) bol regresia, ktorú zachytila druhá recenzia.
- **Okraje stavov:** 1 lp (pri 200 % 2 px).

**Popovery:**
- **Nástroje:**
  - hľadanie vyzerá ako pole s lupou;
  - karty majú podčiarknutie len pod vybranou;
  - riadky 13 Regular;
  - výber = zdvihnutá plocha + akcentová značka (predtým odtieň 1,13 : 1);
  - riadok čakajúci na druhý Delete je červený priamo pri položke a má znak varovania; rámik fokusu je vnútri červeného okraja a výzva v päte je polotučná v `danger`;
  - editor poznámky: hlavné tlačidlo „Uložiť“, nebezpečné „Zahodiť“.
- **Hľadanie:** pole s lupou a akcentovou linkou, výber so značkou, riadok pod myšou len so slabým odtieňom, názov výsledku sa pri výbere nezhrubne, oddelená päta.
- **AI:** karty na `surface` (svetlá téma už nie je sivá), prepínače a obdobia vo vzhľade voľby, tlačidlá päty jednotné.
- **Zvuk:** tlačidlá z dizajnového systému; rámik fokusu stlmenia aj posuvníka je dvojfarebný a mimo akcentovej výplne (predtým 1,1 : 1).
- **Kalendár:** dnešok = pilulka, vybraný deň = obrys.
- **Menu:** polomer zosúladený.
- **Rámy popoverov:** vlas `edge`.

**Lišta:**
- ostáva kompaktná (28 lp) a v klasickom Segoe UI. Tmavá lišta je pred a po redizajne pixelovo zhodná (porovnané bajt po bajte:
  predvolená, hover, stlačenie, 150 %, vlastná modrá, priehľadná). Svetlá lišta sa líši len o odtieň tmavšieho `muted`
  (#566170 → #505B69, kvôli kontrastu na `pressed`);
- **vlastná farba pozadia** (`Theme::for_bar`, `on_background`):
  - text ostáva podľa témy, kým je čitateľný; inak lišta použije farby opačnej témy;
  - tlmený a chybový text sa posunie k hlavnému, kým nemá 4,5 : 1; kruh AI má aspoň 3 : 1;
  - pri nepriehľadnosti pod 50 % rozhoduje téma (pod lištou je hlavne neznáma plocha);
  - predtým dávala svetlá téma s tmavou predvoľbou text s kontrastom ~1 : 1 (nález recenzie prístupnosti);
  - nastavenia ukazujú v náhľade aj v pomere kontrastu skutočne použitý text;
  - text `app.help` je upravený v 11 jazykoch.

**Overenie (offscreen, skutočný GDI renderer; 150 % a 200 % sú vyrenderované do pamäte, nie prepnuté v systéme):**
- `cargo fmt --check` je čistý a `cargo clippy --workspace --all-targets` je bez varovaní.
- **Testy:** všetky prešli okrem jedného nesúvisiaceho, časovo závislého testu.

  | Balík | Prešlo | Ignorované |
  |---|---|---|
  | render | 62 | – |
  | settings | 88 | 1 |
  | app | 140 | 13 |
  | platform | 100 | 1 |
  | usage | 94 | – |
  | tools | 76 | – |
  | core | 71 | – |
  | config | 47 | – |
  | sound | 20 | – |
  | audio | 14 | – |
  | launcher | 14 | – |
  | i18n | 3 | – |
  | media | 3 | – |

  - Nové testy v balíku render: design 4, fonts 4, record 1, zalamovanie CJK a vlastná farba lišty na celej sivej škále.
  - V plnom behu aplikácie raz zlyhal `a_panic_note_names_the_thread_and_place_but_never_the_panic_text`. Je časovo závislý:
    hák paniky zámerne zapisuje cez `try_lock` a súbežný test môže zámok logu práve držať. Samostatne prešiel 3 z 3,
    v predchádzajúcom plnom behu prešiel; dizajnu sa netýka. Nechaný bez zmeny.
- **Audit nastavení:** 0 nálezov pri Segoe UI Variable aj pri vynútenom záložnom Segoe UI (`--font classic`).
  - Rozsah: 11 jazykov × 25 stavov stránok × 100/150/200 % × obe témy = 1 650 kombinácií.
  - Kontroluje orezanie, výšku, prekryv textov a kontrast voči skutočnej ploche pod textom.
  - Prvý beh po zjednotení stĺpca popiskov našiel 108-krát orezaný „Prvý dátum (RRRR-MM-DD)“ (es, pl, pt). Popisok teraz pri
    nedostatku miesta ide nad pole.
- **Audity popoverov** (menu, Nástroje, Hľadanie, AI, Zvuk, kalendár) v každom jazyku, DPI aj téme sú súčasťou testov aplikácie.
- **Prístupnosť:** výpis názov | rola | stav | hodnota | akcia | povolené je pred aj po redizajne **zhodný po riadkoch aj
  poradí** (po normalizácii dočasnej cesty v stavovom riadku).
  - nastavenia: 11 stránok × en/de/ja × obe témy, 1 830 riadkov;
  - popovery Nástroje a Hľadanie: 210 riadkov.

  Ikony do názvov nevstupujú a poradie tabulátora sa nezmenilo.
- **Snímky** sú v scratchpade (nie v repozitári):
  - galéria aplikácie: 157 snímok + 157 so záložným písmom;
  - nastavenia: en pri 100/150/200 % a de, ja, zh-Hans pri 100 %, celé stránky.

  Vizuálne som prezrel výber z nich vrátane zväčšených výrezov textu pri 100 % (ClearType aj sivé vyhladzovanie).
  Pri ClearType je Segoe UI Variable rovnako ostré ako Segoe UI.
- **Po poslednom kole opráv som vyrenderoval a prezrel:**
  - stránky AI, Hodiny, Monitory, Médiá, Pozadie a Nástroje (en pri 100/200 %, de/ja/zh pri 100 %);
  - hárok komponentov `gallery_components`: každý druh × pokoj, hover, stlačenie, fokus, výber, potvrdenie, zakázané, na okne
    aj karte, v oboch témach a troch DPI;
  - snímky fokusu v nastaveniach (`focus_shots`): vybraná voľba, hlavné tlačidlo, potvrdenie mazania, políčko v karte,
    navigácia;
  - popovery Nástroje, Hľadanie, menu a AI v japončine a čínštine (`TRANSOM_GALLERY_LANG`).

**Recenzie:**
- **Prvé kolo** (pred zmenami): vizuálna a prístupnostná recenzia.
  - Zapracované: akcent so štyrmi významami, Bold všade, chýbajúce zoskupenie, orezané „Zrušiť“, nevýrazný výber v Nástrojoch, sivé karty vo svetlej téme, rámik fokusu splývajúci s výberom, nečitateľná vlastná farba lišty, slabé písmo v sivom vyhladzovaní, chybné zalamovanie CJK.
  - Zámerne nezapracované (zmena správania, mimo zadania): veľkosť popoverov podľa obsahu, presun nápovedy z panela Zvuk do nastavení, jeden odznak STALE na kartu, zoznam jazykov ako prepínače.
- **Druhé kolo** (po zmenách): vizuálna aj prístupnostná recenzia snímok „po“.
  - **Opravené:**
    - regresia čínskeho a japonského písma;
    - dva tvary číslic v lište;
    - hover v Hľadaní podobný výberu;
    - karty AI bez nadpisov a rozdelená obnova;
    - rôzny stĺpec popiskov;
    - tlmený text a kruh na vlastných farbách;
    - farba textu pri priehľadnosti pod 50 %;
    - regresia obrysu vzoriek;
    - riadok čakajúci na zmazanie len farbou;
    - okraje stavov pri 200 %;
    - fokus na červenom „Zavrieť“;
    - neaktívny názov okna;
    - jednofarebný rámik posuvníka.
  - **Nezapracované** (zmena správania alebo mimo zadania; ostávajú otvorené):
    - veľkosť popoverov AI a Zvuk podľa obsahu, šesť odznakov STALE, rozloženie a nápoveda panela Zvuk;
    - stlmenie závislých volieb pri vypnutom rodičovi (dnes sú aktívne aj vtedy, mení by to správanie);
    - cyklické tlačidlá („Uchovávať záznamy: 7 dní“) ako rozbaľovacie zoznamy;
    - zoznam jazykov ako prepínače;
    - dvojitý názov „transom“ v hlavičke;
    - nepomenovaný riadok druhu dátumu (pomenovanie by zmenilo prístupné názvy);
    - stav tlačidla „Upraviť“ pre čítačky;
    - audit zatiaľ nemá kontrolu 3 : 1 pre ťahy a rámiky (preto regresiu vzoriek zachytil až recenzent).

**NEOVERENÉ / známe obmedzenia:**
- **Živo** (skutočné okno, testovacia inštancia so scratch konfiguráciou, snímka vlastného okna cez `PrintWindow`, zatvorené cez
  `WM_CLOSE`): len stránky Pozadie a AI pri 100 %. Natívne polia HEX/R/G/B sedia s popiskami a kartou.
  - Dátumové pole na konci stránky AI som nevidel (posun by vyžadoval simuláciu vstupu).
  - Stránka AI pri otvorení len číta stav pripojenia Claude Code (`~/.claude/settings.json`), ako vždy; nič sa nezapisovalo.
- Žiadna živá kontrola lišty, popoverov ani `--test-tools`, nič pri 150/200 % na obrazovke, žiadna kontrola človekom, žiadny beh
  s Narratorom.
- Tokeny pokrývajú hlavné prvky (nadpisy, tlačidlá, karty, výber, polia, fokus). V popoveroch ostáva veľa doslovných veľkostí
  písma (11–15 lp), ktoré stupnicu len približne dodržiavajú.
- Režimy vysokého kontrastu Windows a nastavenie „Veľkosť textu“ nie sú podporované (ako predtým).
- Statické texty stránok nie sú vystavené čítačkám (ako predtým).
- Poradie tabulátora natívnych polí sa nemení.
- Záloha písma je overená len vynútením (`--font classic`), nie na Windows 10.
- Priehľadná lišta nad skutočnou plochou nie je overená naživo.
- Pravidlá zalamovania japončiny (kinsoku) sa neuplatňujú.
- Čínsky text môže mať miestami zmiešaný tvar znakov: systémové prepojenie písiem má na tomto PC prvé japonské Meiryo UI (rovnaké ako pred redizajnom).
- Popovery v de, ja a zh a nastavenia v de/ja/zh pri 150/200 % sú pokryté auditmi, ale snímky sa vizuálne neprezerali.
- Spustená lišta používateľa beží zo starej zostavy, kým sa znova nezostaví a nespustí.

### 8.28 Míľnik: batérie periférií (2026-11-23)

Nová položka lišty **Batérie periférií** (`peripherals`, predvolene viditeľná, zobrazí sa len so zariadením) a crate
`transom-peripherals`. Ukazuje bezdrôtové zariadenia, ktoré hlásia batériu: na lište ikona druhu zariadenia, symbol batérie
a percento (najviac 1/2/3/všetky; pri menšom počte najnižšia úroveň prvá a „+N“), klik otvorí panel so zoznamom.
Všetko **len na čítanie**; nič sa neodhaduje (zariadenie bez úrovne má „–“ a stav slovami).

| Zdroj | Ako | Stav overenia |
|---|---|---|
| Razer | 90-bajtové HID feature reporty, trieda 0x07 príkazy 0x80 (úroveň 0–255) a 0x84 (nabíja sa), transaction id 0x1F/0x3F/0xFF | **Overené naživo** (`probe`, DeathAdder V4 Pro): prijímač PID 00BF 66 %; po zapojení kábla prijímač hlási „timeout“ (spí) a káblové rozhranie PID 00BE 66 % nabíja sa – `Tracker` ukáže jediné zariadenie „66 %, nabíja sa“; po odpojení kábla znova prijímač 66 %, nenabíja sa. Overené samostatnými behmi `probe`, nie počas jedného behu lišty |
| NuPhy WH80 | prijímač 19F5:8F01, MI_01: GetBase 0x04 a GetFunc 0x05 (bajt na `mode*64+32`); report `FA FB 07 00 00 <úroveň>` pri kábli | **Overené naživo** (86 %). Zistené 2026-11-23: s káblom firmvér hlási všade 100 % a bit nabíjania nikdy; bajt v reporte o kábli je úroveň (0x5A = 90 % pri odpojení), nie stav kábla. Počas nabíjania sa ukáže „nabíja sa“ bez percent |
| Bluetooth | vlastnosť uzla `{104EA319-…} 2` (úroveň, ktorú ukazujú Nastavenia), pripojenie `{83DA6326-…} 15`, inak vek úrovne `{104EA319-…} 7` < 30 min | Overené len **negatívne**: vypnutý Xbox ovládač (60 % zo septembra, pripojenie false) sa správne neukáže. Kľúče nie sú v SDK. Nabíjanie Windows nehlási |
| Logitech | HID++ 2.0 cez prijímač (indexy 1–6) alebo priamo (0xFF): ROOT, 0x0005, 0x1004 / 0x1000 / 0x1001 (napätie → percento podľa krivky Li-Pol) | **Neoverené** (na stroji nie je Logitech hardvér); len testy formátu správ a výpočtov |

Architektúra: služba na vlastnom vlákne (kolo každých 30 s – 5 min, `refresh` pri otvorení panela, zlúčenie požiadaviek),
`Tracker` drží poslednú skutočnú hodnotu (spiace zariadenie stlmené s vekom, zabudnuté po 3 min bez hlásenia). Služba beží len
pri viditeľnej položke a zapnutom zdroji, **nikdy pri automatizácii** (`--secs`, `--test-*`, `--sim-*`). Nastavenia: stránka
**Periférie** (zdroje, počet v lište, interval). Kredity protokolov: `THIRD_PARTY_NOTICES.md` (nič nie je prevzaté).

#### Overenie

- `cargo test -p transom-peripherals` (protokoly s bajtmi zachytenými na tomto PC, `Tracker`, služba), konfigurácia (tolerantné
  načítanie), panel a položka (`periph_tests`: stavy slovami, a11y, klávesnica, myš, výber na lištu, audit 11 jazykov × 2 témy ×
  3 DPI), stránka nastavení (a11y, klávesnica vo všetkých jazykoch).
- `cargo run -p transom-peripherals --example probe -- --rounds 2`: Razer 66 % (171 ms), NuPhy 86 % (77 ms), Bluetooth 0, Logitech 0.
- Lišta spustená naživo s dočasnou konfiguráciou: služba sa spustila a prvá snímka prišla za 0,26 s; lišta bola skrytá (aplikácia
  na celej obrazovke), preto vzhľad overený offscreen zo skutočných dát (`live_periph_png`, ručne).

#### Obmedzenia

- Logitech neoverený; slúchadlá Logitech/SteelSeries/Corsair/HyperX (iné protokoly) nie sú podporené.
- Bluetooth: kľúč pripojenia mimo SDK; pozitívny prípad (zapnuté BT zariadenie) neoverený.
- NuPhy: nabitá do plna a odpojená klávesnica vyzerá ako zapojená, kým úroveň neklesne pod 100 %.
- NuPhy: ak sa lišta spustí, keď je kábel už zapojený, ukáže 100 % bez nabíjania (úroveň pred zapojením sa
  neukladá; report o kábli príde len pri zmene).
- Zasunutie prijímača sa prejaví až v ďalšom kole alebo po otvorení panela (bez `WM_DEVICECHANGE`).

### 8.29 Názov a logo (2026-11-26)

Názov **transom** (priečnik – trám nad dverami, ako lišta nad obrazovkou), písaný malými písmenami. Projekt sa pôvodne volal
inak; 2026-11-26 dostal nové meno, nové logo a nový repozitár (`Antive1/transom`) s novou históriou. Crate-y `crates/transom-*`,
binárky `transom-app.exe`, `transom-settings.exe`, `transom-statusline.exe`; dáta v `%APPDATA%\transom` a `%LOCALAPPDATA%\transom`
(prepisateľné `TRANSOM_CONFIG` / `TRANSOM_DATA`).

- **Logo:** veľké „T“ – priečka je lišta, pod ňou obrys obrazovky; wordmark „transom“ (`t` v akcente); ikony a SVG
  v `assets/brand` vygenerované z `brand.rs` (`brand_export`), návrhy v `examples/logo_variants.rs`, náhľady 16–256 px
  skontrolované.

### 8.30 Súbory Police v strede lišty a skratka vyhľadávača (2026-11-23)

**Vetva** `feature/shelf-bar-hotkey` (z `feature/rename-transom`; lokálna, bez remote).

**Polica v lište.** Nová položka rozloženia `shelf` („Súbory z Police“), predvolene **v strede** a viditeľná; ukáže sa, len keď
je na Polici aspoň jeden súbor a Polica je zapnutá.
- Zobrazí prvé 3 súbory v poradí panela (pripnuté hore) ako symbol súboru/priečinka + skrátený názov (max. 18 znakov, krátka
  prípona ostane); ďalšie ako „+N“ (otvorí panel Nástroje). Nedostupný súbor (chýba, sieť, len v cloude) je stlmený
  a stav nesie aj prístupný názov.
- **Rozloženie:** Polica je „pripojená“ položka stredovej skupiny. Ak je v strede sama, leží na polovici lišty. Ak je v strede
  aj iná položka (kotva, napr. hodiny alebo médiá), na polovici ostane **kotva** a Polica je vedľa nej – vľavo alebo vpravo
  podľa poradia v *Nastaveniach > Rozloženie* (predvolene vpravo, lebo existujúce rozloženia ju dostanú na koniec stredu).
  Pružný názov skladby v strede rastie symetricky okolo polovice, Polica jeho rast obmedzuje zo svojej strany. Na úzkej
  lište sa Polica skryje (poradie zahadzovania 5) skôr než hodiny.
- **Klik** = skopíruje súbor do schránky (ako Enter v paneli) a oznam to potvrdí; **potiahnutie** (posun o 6 px alebo
  opustenie lišty so stlačeným tlačidlom) = OLE ťahanie von (len COPY/LINK, cez frontu požiadaviek). Vlastné ťahanie sa na
  lištu znova nepridá (`DragEnter` ho odmietne, kým beží `drag_out`). Kliknutá / ťahaná položka sa určí podľa id súborov
  z posledného kreslenia, nie podľa aktuálnej snímky.
- MSAA: každý súbor je tlačidlo s názvom a stavom; „+N“ má vlastný názov.

**Globálna skratka vyhľadávača.** `tools.launcher.hotkey`, predvolene **Alt+Space**; prázdna = vypnutá.
- Platná skratka: aspoň Ctrl, Alt alebo Win (samotný Shift nie) + písmeno, číslica, F1–F24 alebo medzerník; nie bežné
  skratky, ktoré by globálna registrácia „ukradla“ všade (Alt+F4, Ctrl+C/V/X/Z/Y/A/S). Zlý zápis
  v súbore = predvolená skratka; zápis sa ukladá kanonicky („Ctrl+Shift+K“).
- `RegisterHotKey` s `MOD_NOREPEAT` na riadiacom okne lišty; žiadny hák klávesnice. Registruje sa len pri zapnutom
  vyhľadávači (inak by `Action::Launcher` poslal Win+S s ešte držaným Alt). Zlyhanie (skratku drží iná aplikácia, napr.
  PowerToys Run) sa oznámi na lište raz pre danú skratku a zapíše do logu.
- **Na celej obrazovke predvolene nefunguje** (`hotkey_in_fullscreen` = false): kým lišta hlási aplikáciu na celú obrazovku
  (`ABN_FULLSCREENAPP` alebo vlastná detekcia pri vzniku lišty), skratka sa **odregistruje**, takže ju dostane hra / video;
  po skončení sa zaregistruje znova. Volba v Nastaveniach ju zapne aj na celej obrazovke. Obmedzenie: hlásia to len monitory
  s lištou (aplikácia na celú obrazovku na monitore bez lišty skratku nepozastaví).
- Stlačenie otvorí vyhľadávač na monitore s kurzorom (pod položkou Search, inak v strede lišty), druhé ho zavrie.
  Popover ignoruje `SC_KEYMENU`, aby ešte držaný Alt neotvoril systémovú ponuku.
- *Nastavenia > Nástroje > Vyhľadávanie:* tlačidlo so skratkou (klik → „Stlačte novú skratku…“, Esc zruší, neplatná
  kombinácia ostane v zachytávaní s vysvetlením), „Vypnúť skratku“, „Obnoviť Alt+Space“. Počas zachytávania sa
  `WM_SYSKEYDOWN/UP` a `WM_SYSCHAR` neposielajú oknu (Alt+medzerník neotvorí systémovú ponuku nastavení).
- Skratka sa neexportuje (celý oddiel `tools` je mimo exportu).

**Overenie.**

| Čo | Ako |
|---|---|
| Model skratky (parsovanie, kanonický zápis, neplatné kombinácie, vypnutie) | **TEST** |
| Zachytávanie v nastaveniach (modifikátory, neplatná, Esc, iná voľba, vypnúť, obnoviť, voľba celej obrazovky, uloženie) | **TEST** (volanie logiky, nie skutočné klávesy) |
| Pozastavenie skratky pri aplikácii na celú obrazovku | **NEOVERENÉ LIVE** (logika `sync_hotkey` bez testu) |
| Rozloženie (Polica sama v strede, vedľa hodín vľavo/vpravo, úzka lišta, médiá v strede) | **TEST** (invarianty bez prekryvu, stred ±1 px) |
| Zobrazenie Police (3 súbory + „+N“, skracovanie, nedostupné, predvolené rozloženie) | **TEST**, offscreen PNG v oboch témach pri 100 / 150 % |
| Registrácia Alt+Space na tomto PC, otvorenie vyhľadávača a písanie po skratke | **LIVE** (2026-11-23, debug build na ploche používateľa): registrácia uspela; vložené Alt+Space (`keybd_event`) → `Hotkey(1)` → vyhľadávač otvorený pod Search za ~10 ms; následne napísané „not“ prišlo do vyhľadávača (`PopoverChar`, výsledky Notepad…), systémová ponuka sa neotvorila; druhé Alt+Space ho zavrelo. Vložené klávesy, nie fyzická klávesnica |
| Polica v lište na skutočnej ploche | **LIVE**: jeden súbor používateľa v strede lišty (`Shelf.0@1209+142` pri šírke 2560 = stred 1280) |
| Klik (kopírovanie do schránky) a ťahanie súboru z lišty myšou | **NEOVERENÉ LIVE** – klik by prepísal schránku používateľa a ťahanie vyžaduje skutočnú myš; overí používateľ |
| Zachytávanie skratky skutočnými klávesmi v okne nastavení | **NEOVERENÉ LIVE** (len logika cez test) |

### 8.31 Farba textu a ochrana OLED (2026-11-23)

**Vetva** `feature/oled-text-color` (z `feature/shelf-bar-hotkey`; lokálna, bez remote).

**Farba textu** (*Nastavenia > Pozadie lišty*, karta „Farba textu a ikon“): Automaticky / Svetlý / Tmavý / Vlastná farba
(systémový výber farby). `appearance.text_mode` + `text_color`, exportujú sa s pozadím.
- *Automaticky* = doterajšie správanie: text podľa témy; pri vlastnom pozadí s nepriehľadnosťou ≥ 50 % sa prepne na
  čitateľnejšiu tému (`Theme::on_background`). Pod 50 % je pod lištou neznáma plocha, text ostáva podľa témy.
- Pevná / vlastná farba: `Theme::with_text` – tlmený text, nízka batéria (≥ 4,5 : 1) a akcent (≥ 3 : 1) sa odvodia od
  zvoleného textu a pozadia; samotný text sa **neopravuje**, nastavenie len ukáže kontrast a upozornenie pod 4,5 : 1.
- Lišta aj náhľad v nastaveniach používajú tú istú cestu (pozadie z `Appearance::effective`, potom text, potom stlmenie).

**Ochrana OLED** (`oled`, karta „Ochrana OLED“, **predvolene vypnutá**, neexportuje sa). Formulácia v UI: „znižuje riziko“,
nie „zabraňuje“. Podvoľby (po zapnutí predvolene všetky okrem skrývania):
- **Posun obsahu:** raz za minútu ďalšia z 8 polôh kruhu (±2 px vodorovne, 0–1 px zvisle, lp × DPI). Vodorovný posun ide
  cez rozloženie (vrátené `placed` sú posunuté → hit-testing, MSAA oblasti, tooltipy, ukotvenie popoverov, prekreslenie
  hodín a posuvný text sedia), zvislý len pri kreslení (`ShiftY`). Fáza z minútového tiku, bez nového časovača.
- **Stlmený text:** `Theme::dimmed` – približne 75 % jasu, kontrast textu a tlmeného textu nikdy pod 4,5 : 1.
- **Čisto čierne pozadie:** prebije vlastnú farbu aj priehľadnosť (nepriehľadná čierna, ClearType cesta).
- **Bez čiar:** bez spodného vlasu a oddeľovačov.
- **Skrývanie lišty** (`auto_hide`): lišta sa zaregistruje len `ABM_NEW` (oznámenia o celej obrazovke), **bez**
  `ABM_SETPOS` → pracovná plocha je celá; okno je skryté a vysunie sa (navrch, bez aktivácie), keď je kurzor na hornom
  riadku monitora. Poloha kurzora sa číta časovačom 120 ms **len kým je skrývanie zapnuté** (žiadne neviditeľné okno na
  okraji, ktoré by kradlo kliky na hornom riadku maximalizovaných okien; žiadny hák myši). Skryje sa 600 ms po odchode
  kurzora, nie pri otvorenom popoveri / ťahaní; otvorenie popoveru (aj Alt+Space) lištu vysunie. Pri aplikácii na celú
  obrazovku sa nevysunie (oznámenie systému + kontrola `fullscreen::app_on` v okamihu vysunutia). Zmena voľby lišty
  vytvorí nanovo.
- Skrývanie sekúnd sa **nepridalo**: meniace sa pixely sa opotrebúvajú rovnomerne, riziko nesú statické prvky.
- Stlmenie pri nečinnosti používateľ nechcel.

| Čo | Overenie |
|---|---|
| Konfigurácia (predvolene vypnuté, tolerantné čítanie, čierne pozadie prebije priehľadnosť, kruh posunu) | **TEST** |
| `with_text` / `dimmed` (kontrasty odvodených farieb, stlmenie nikdy pod 4,5 : 1) | **TEST** |
| Posun obsahu a „bez čiar“ v scéne (všetky časti spolu, v hraniciach, zvisle len kresba) | **TEST** |
| Nastavenia (uloženie volieb, náhľad z tej istej cesty, audit stránky v sk/en/de/ja × 100/150 % × 2 témy) | **TEST** |
| Rozhodovanie skrývania (`hide_step`) | **TEST** |
| Skrývanie naživo (debug build, dočasná konfigurácia, kurzor cez `SetCursorPos`, klávesy cez `keybd_event`) | **LIVE**: pracovná plocha 0..1440 (bez rezervy); kurzor na y=0 → lišta viditeľná; nad lištou ostala; 0,3 s po odchode ešte viditeľná, po 1,1 s skrytá; Alt+Space pri skrytej lište ju vysunul spolu s vyhľadávačom, po Esc sa skryla; snímka: čierne pozadie, stlmený text, bez čiar |
| Skrývanie pri hre na celú obrazovku, Explorer reštart, viac monitorov, zmena DPI | **NEOVERENÉ LIVE** |
| Posun obsahu naživo (zmena fázy po minúte) | **NEOVERENÉ LIVE** (len test scény) |

### 8.32 Mixér aplikácií v paneli Zvuk (2026-11-24)

**Vetva** `feature/app-mixer` (z `feature/oled-text-color`; lokálna, bez remote). Nový crate `transom-mixer`.

**Rozhodnutia používateľa:**
1. Mixér je v existujúcom paneli **Zvuk** (sekcia „Aplikácie“ pod systémovou hlasitosťou).
2. Výstup pre aplikáciu „ako EarTrumpet“ → **zmena oproti §8.17/§8.20**: na výber výstupu *aplikácie* sa používa
   nedokumentované rozhranie `Windows.Media.Internal.AudioPolicyConfig` (`SetPersistedDefaultAudioEndpoint`,
   `GetPersistedDefaultAudioEndpoint`). Predvolený výstup **systému** transom stále nemení a `transom-audio` taketo
   rozhrania nepoužíva (jeho test to stráži); všetko nedokumentované je izolované v `transom-mixer/src/core.rs`.
3. Zosilnenie nad 100 %: používateľ chcel **skutočné** zosilnenie bez ďalšej aplikácie, inak nič. Relácia Windows má
   strop 1,0; jediná cesta bez ovládača je zachytiť zvuk procesu (process loopback, dokumentované od Win10 2004),
   zosilniť ho a prehrať znova, s pôvodnou reláciou stlmenou. Spike to **nemohol overiť** (počítač nemal žiadny aktívny
   výstup) a aj pri úspechu to znamená pridané oneskorenie (rozsynchronizovaný obraz a zvuk), ticho pri DRM obsahu,
   limiter, stále bežiace zvukové vlákno a stlmené aplikácie, ak transom spadne. **Zatiaľ nie je implementované**;
   rozsah je 0–100 % a predvolená hodnota (100 %) sa nikdy nezapisuje.

**Model:** relácie zo **všetkých** aktívnych výstupov (aplikácia presmerovaná na iný výstup má reláciu tam), bez
vypršaných; zoskupenie podľa cesty .exe (Chrome s viacerými procesmi = jedna aplikácia), systémové zvuky zvlášť (PID 0
pre výstup) a vždy na konci; hrajúce aplikácie prvé. Meno: popis z verzie .exe → vlastný názov relácie (nie `@zdroj`)
→ meno .exe → PID. Ikona: ikona .exe cez existujúci `IconRt` (asynchrónne), inak vstavaná.

**Služba:** vlastné vlákno (COM MTA), `IMMNotificationClient` + `IAudioSessionNotification` (na každom výstupe) +
`IAudioSessionEvents` (každá relácia) len pošlú udalosť; séria sa zlúči (60 ms) a relácie sa prečítajú raz. Zápis
hlasitosti s vlastným kontextom (ozvena nespustí čítanie). **Beží len kým je panel Zvuk otvorený** (zatvorenie zruší
registrácie). Rozhranie politiky: IID podľa buildu (`ab3d4648…` od 21390, inak `2a59116d…`), vtable podľa EarTrumpet;
ID zariadenia `\\?\SWD#MMDEVAPI#<id>#{e6327cad-…}`, nastavuje sa pre `eMultimedia` aj `eConsole`, „Predvolený“ =
prázdny HSTRING. Výstup v UI sa ukazuje len podľa spätného čítania; odmietnutie = hláška pri aplikácii. Ak sa rozhranie
nedá získať, výber výstupu sa skryje a panel ponúkne `ms-settings:apps-volume`.

| Čo | Overenie |
|---|---|
| Zoskupenie, mená, poradie, formát ID výstupu | **TEST** |
| Služba: zápis všetkým reláciám aplikácie, strop 100, žiadny zápis predvolenej hodnoty, výstup podľa spätného čítania, odmietnutie | **TEST** (falošný backend) |
| Panel: audit kontrastu a pretekania (100/150 %, 2 témy), MSAA názvy/roly/hodnoty, myš, klávesy, koliesko, Tab | **TEST** + **SIM** |
| Získanie rozhrania politiky na tomto PC (build 26300) | **LIVE-API**: tovareň sa aktivovala s IID `ab3d4648…` |
| Hlasitosť, stlmenie a prepnutie výstupu skutočnej aplikácie, systémové zvuky, zmena ikon | **NEOVERENÉ LIVE** (PC bez aktívneho výstupu) |
| Zosilnenie nad 100 % | **NEIMPLEMENTOVANÉ** (pozri rozhodnutie 3) |

### 8.33 Nové položky lišty, zvukové profily, Bluetooth a kalendáre ICS (2026-11-24)

**Vetva** `feature/bar-extras` (z `feature/app-mixer`; lokálna, bez remote). Používateľ chcel „všetko“ z návrhov okrem
zosilnenia nad 100 % (zatiaľ nie). Kalendár: súkromný odkaz ICS (nie OAuth). Reštart transom na živé testy povolený.

Nové položky (všetky **predvolene skryté**, skupina vpravo; nastavenia na novej stránke *Doplnky*):
- **Klávesnica** (`keyboard`): jazyk rozloženia vlákna s fokusom v okne na popredí (`GetGUIThreadInfo` +
  `GetKeyboardLayout`) a Caps Lock (`GetKeyState`). Čítanie každých 250 ms len pri viditeľnej položke; **žiadny hák
  klávesnice**. Klik = Win+Space.
- **Časovač** (`timer`): odpočet s predvoľbami a ±1 min, pomodoro (prestávky sa spúšťajú samy, po N kolách dlhá),
  stopky s kolami. Počíta sa z časových značiek; tik je naplánovaný len kým niečo beží. Na konci oznam + systémový zvuk
  upozornenia (vypnuteľný).
- **Jas monitora** (`brightness`): DDC/CI cez dokumentované Monitor Configuration API (VCP 0x10), vlastné vlákno, zápisy
  zlúčené a najmenej 120 ms od seba, zobrazuje sa **spätné čítanie**, čítanie s 3 pokusmi.
- **Vyťaženie systému** (`sysmon`): CPU (`GetSystemTimes`), RAM, GPU (PDH, anglické počítadlá, súčet procesov na jadro,
  najvyťaženejšie jadro), sieť (`GetIfTable2`, hardvérové rozhrania bez filtrov). Pevná šírka čísel (číslicové medzery).
  Klik = Správca úloh. **Teploty nie** (žiadny zdokumentovaný zdroj bez ovládača/admin).
- **Mikrofón a kamera** (`privacy`): mikrofón v použití = aktívne relácie na vstupoch (dokumentované); kamera (a doplnkovo
  mikrofón) = história súkromia Windows `CapabilityAccessManager\ConsentStore` (**nedokumentované, len čítanie**).
  Stlmenie predvoleného mikrofónu pre obe roly so spätným čítaním; globálna skratka (predvolene Ctrl+Alt+M) len pri
  viditeľnej položke, funguje aj v hre na celú obrazovku.

V paneli **Zvuk**:
- **Zvukové profily**: 6 pevných mien (Hry, Hovor, Hudba, Film, Práca, Noc – Nastavenia nemajú textové pole). Uloží sa
  hlasitosť, stlmenie a výstup spustených aplikácií (kľúč = cesta .exe, systémové zvuky zvlášť); použitie mení len
  rozdiely, výstup len na pripojené zariadenie, aplikácie mimo profilu nemení. Neexportuje sa (cesty a ID výstupov).
- **Bluetooth zvuk**: spárované zvukové zariadenia s tlačidlom Pripojiť/Odpojiť. Výstup → topológia → KS filter
  Bluetooth ovládača → zdokumentovaná sada `KSPROPSETID_BtAudio` (ONESHOT_RECONNECT/DISCONNECT), postup ako nastavenia
  Windows a ToothTray (BSD-2, prevzatý len postup). Výsledok potvrdí nové čítanie (do ~10 s).

**Kalendáre ICS** (Nastavenia › Doplnky): odkaz sa pridá **zo schránky** (https/webcal), v nastaveniach sa ukazuje len
počítač (napr. calendar.google.com); odkaz je tajný údaj – nelogue sa, neexportuje sa. Sťahovanie WinHTTP (len https,
8 MB, 20 s), pri štarte, každých 30 min a pri otvorení kalendára, ak sú dáta staršie ako 10 min. Parser `transom_core::ics`:
VEVENT, celodenné, RRULE (DAILY/WEEKLY/MONTHLY/YEARLY, INTERVAL, COUNT, UNTIL, BYDAY s poradím, BYMONTHDAY, BYMONTH),
EXDATE, RECURRENCE-ID, STATUS:CANCELLED, časové pásma z VTIMEZONE (plávajúci čas a neznáme pásmo = miestny čas PC).
V kalendári štvorcová značka a riadky udalostí zvoleného dňa (pred udalosťami AI). **Prvé a jediné sieťové spojenie
transom.**

| Čo | Overenie |
|---|---|
| Modely (časovač, ICS vrátane letného času/výnimiek/presunutých výskytov, profily, sysmon, prevody jasu, mená BT, URL) | **TEST** |
| Položky v lište a panely (audit kontrastu a pretekania 100/150 %, 2 témy, MSAA, myš, klávesy) | **TEST** + **SIM** |
| Stránka Doplnky (uloženie, odkaz kalendára sa nikde neukáže celý, a11y vo všetkých jazykoch) | **TEST** |
| Klávesnica: EN/SK prepnutie (Alt+Shift) a Caps Lock zachytené v dlho bežiacom procese | **LIVE** (Windows Terminal; aplikácia Store neoverená) |
| Lišta s položkami po štarte, časovač 0:58 → Hotovo, sysmon zhodný so Správcom úloh (GPU 11–12 % vs 11,6 %), panely Mikrofón a Jas | **LIVE** (debug build, dočasná konfigurácia) |
| Jas: prvé čítanie LG ULTRAGEAR+ = 80 %, potom monitor odpovedá chybou DDC (0xC0262589) | **ČIASTOČNE LIVE**, zápis neoverený |
| Mikrofón stlmenie / skratka | **NEOVERENÉ LIVE** (bez mikrofónu) |
| Bluetooth: JBL Charge 5 nájdený; pripojenie zlyhalo (reproduktor vypnutý) | **ČIASTOČNE LIVE** |
| ICS: verejný kalendár sviatkov (176 udalostí, 295 ms, správne najbližšie sviatky) | **LIVE** |
| Zvukové profily na skutočných aplikáciách | **NEOVERENÉ LIVE** (bez zvukového výstupu) |

### 8.34 Audit bezpečnosti a robustnosti (2026-11-26, vetva `agent/audit-security`)

Rozsah: sieť a aktualizácie (WinHTTP, SHA-256/WinVerifyTrust, výber vydania), ICS, počasie, import nastavení, čítanie logov
Claude Code/Codex, most statusline, HID, nástroje (schránka, police, ťahanie), spúšťač, `unsafe` Win32, inštalátor a workflow.
Vydanie má `panic = "abort"`: každý pád vlákna ukončí lištu.

#### 1. Opravené (každé s regresným testom)

| Závažnosť | Miesto | Nález → oprava |
|---|---|---|
| **Vysoká** | `transom-core/src/ics.rs` (`parse_date`, `parse_local`, `parse_offset`) | Rezanie `&s[a..b]` na bajtoch: viacbajtový znak v DTSTART/UNTIL/EXDATE/TZOFFSETTO z kalendára na sieti zhodil lištu pri **každej** synchronizácii. Čísla sa čítajú len z ASCII číslic cez `str::get`. Test `ics::hostile`. |
| Stredná | `ics.rs` (`parse_duration`, BYDAY, `occurrences`) | Pretečenie (DURATION `P9999999999W`, poradie BYDAY `2147483647MO`) – v ladiacom builde pád, vo vydaní nezmysel. `checked_*`, strop 10 rokov, poradie −53..=53. |
| Stredná | `ics.rs` (`expand`) | Zlomyseľný kalendár (staré DTSTART + obrovské COUNT + BYDAY na každý deň, tisíce položiek zoznamov) zasekol vlákno okna na minúty. Stropy zoznamov (64 / 2000 EXDATE / 200 pásiem) a rozpočet práce na udalosť aj na volanie; 8 MiB kalendár sa v ladiacom builde spočíta za ~1,5 s. |
| **Vysoká** | `transom-app/src/update.rs` (`pick`) | Názov inštalátora z JSON GitHubu sa len porovnal so začiatkom/koncom a pripojil k `%TEMP%` → `transom-setup-\..\..\x-x64.exe` mimo priečinka; adresa len so začiatkom `DOWNLOAD_PREFIX` → `download/../../iny/repo/…`. Teraz názov aj značka len `[A-Za-z0-9._-]` a adresa presne `DOWNLOAD_PREFIX<značka>/<názov>` toho istého vydania. |
| Stredná | `update.rs` (`install`), `transom-platform/src/selfupdate.rs` | TOCTOU: súčet sa overil v pamäti, potom zápis na pevnú cestu, kontrola podpisu a spustenie podľa cesty. Teraz nový priečinok (`fresh_dir`), súbor `create_new`, znova otvorený len na čítanie so zdieľaním len na čítanie, súčet znova z disku, handle drží cez WinVerifyTrust aj ShellExecute (`write_pinned`). Test overuje, že súbor sa nedá prepísať/premenovať/zmazať a že sa z neho dá spustiť proces. |
| Stredná | `transom-platform/src/http.rs` | Časové limity WinHTTP platia na operáciu: server posielajúci po bajte držal vlákno (a ďalšie kalendáre v poradí) donekonečna. Celkový limit 6 × `timeout_ms`; TLS 1.2+; presmerovania najviac 5, nikdy https→http, overenie konečnej adresy; počítač sa oddeľuje aj na `?`/`#` ako v `normalize_url`, fragment sa neposiela. |
| Stredná | `transom-usage/src/claude_setup.rs` | Zápis `~/.claude/settings.json` cez pevný `settings.json.transom-tmp` (existujúci symlink sa nasledoval), zálohy cez `exists()` + `fs::copy`. Teraz `cache::write_atomic` (jedinečný názov, `create_new`, `sync_all`), zálohy `create_new`, `rollback` obnoví len súbor `settings.json.transom-backup-<číslice>[-<číslice>]` v tom istom priečinku (podvrhnutý záznam nemôže skopírovať cudzí súbor na miesto nastavení). |
| Nízka | `transom-config` a `transom-usage` `write_atomic` | `File::create` dočasného súboru → `create_new` (zvyšok po páde sa zmaže, nie nasleduje). |
| Stredná | `transom-platform/src/appbar.rs` (`setting_name`), `transom-settings` `WM_SETTINGCHANGE` | Text z `lParam` sa čítal bez overenia (v nastaveniach bez obmedzenia dĺžky): proces toho istého používateľa (alebo vysielanie, kde `lParam` nie je text) mohol zhodiť lištu aj nastavenia. Teraz len pamäť overená `VirtualQuery`, najviac 128 znakov, `read_unaligned`. |
| Nízka | `update.rs` (`pick`) | Stránka vydania (otvára sa cez ShellExecute) sa brala z `html_url` overeného len začiatkom (`…/transom/../../x`); teraz sa skladá z overenej značky. |
| Nízka | `transom-statusline/src/main.rs` | Stdin bez obmedzenia → najviac 1 MiB. |
| Nízka | `.github/workflows/release.yml` | Verzia z `Cargo.toml` sa vkladala do textu skriptu pwsh; teraz cez `env` a s kontrolou tvaru `N.N.N[-pre]`. |
| Nízka | `transom-settings` (kopírovanie do schránky) | Únik `GlobalAlloc` pri zlyhaní `GlobalLock`/`SetClipboardData`. |
| – | 26 súborov | Komentár `// SAFETY:` pri každom `unsafe` bloku knižníc a programov (`clippy::undocumented_unsafe_blocks`: 210 → 17, zvyšok len v `examples/`). Pri kontrole tvrdení o uvoľnení som opravil tri komentáre a našiel únik v schránke (nižšie). |

#### 2. Preverené bez nálezu (a ako)
- **Prečítané:** odpovede HID Razer/Logitech (dĺžky overené pred indexovaním), virtuálne súbory a názvy v Polici (čistenie názvov,
  `create_new`, existujúce testy s `..\`), čítanie `HGLOBAL` v schránke a pri ťahaní (`GlobalSize`), `RefCell` vo wndproc
  (`try_borrow_mut` v `with_ui`/`st`, krátke pôžičky `NOTCH`), počasie (JSON bez indexovania), vlastný skener JSON v `claude_setup`
  (iteratívny, pozície na ASCII znakoch), čísla v správach od iných procesov (`WM_TRANSOM_INVOKE`, stránka nastavení, tlačidlá
  hlavičky, `BarEvent::Invoke`: všade `.get()`).
- **Len vyhľadávaním vzorov** (celý workspace mimo testov, `unwrap`/`expect`, rezanie `[a..b]`, `truncate`/`split_at`/`insert_str`/
  `replace_range`, delenie premennou, `from_raw_parts`, `lp.0 as *`): import nastavení, čítanie logov Claude Code/Codex, spúšťač,
  poznámky, konfigurácia – nájdené miesta sú strážené; vstupné body týchto modulov som celé nečítal.

#### 3. Neopravené (mení správanie alebo je nejednoznačné) – na rozhodnutie
| Závažnosť | Miesto | Poznámka |
|---|---|---|
| Stredná | `update.rs` | Vydavateľ sa porovnáva len podľa zobrazovaného mena certifikátu; nepodpísaná lišta (vývojársky build) kontrolu podpisu preskočí úplne. Pripnutie odtlačku by pri obnove certifikátu SignPath zlomilo aktualizácie. |
| Nízka | `update.rs` | Názov inštalátora sa nemusí zhodovať s verziou značky (vydanie `v9` s inštalátorom `0.1.0` by nainštalovalo staršiu verziu); dá sa to spraviť len vydavateľ. Navrhujem vyžadovať `transom-setup-<verzia>-x64.exe`. |
| Nízka | `claude_setup.rs` | Zálohy `settings.json` sa nerotujú (mazanie súborov v `~/.claude` by bolo zmenou správania). |
| Nízka | `release.yml` | `choco install innosetup` bez pevnej verzie; `workflow_dispatch` z ľubovoľnej vetvy môže poslať žiadosť o podpis (každú schvaľuje človek v SignPath); `gh release upload --clobber` prepíše zverejnené súbory; `contents: write` pre celú úlohu (aj pre `cargo build`). SHA piny akcií som neoveroval voči GitHubu. |
| Nízka | `installer/transom.iss` | Pri inštalácii pre všetkých (`PrivilegesRequiredOverridesAllowed=dialog`) sa kľúč `Run` zapisuje do HKCU správcu; `taskkill /im` pri odinštalovaní. |
| Nízka | `transom-settings` `WM_SETICON` | Predošlé ikony sa po zmene DPI neuvoľnia (malý únik GDI). |
| Nízka | `ics.rs` | Kontrola zmenených inštancií (RECURRENCE-ID) je O(udalosti × výskyty × zmeny); obmedzená stropmi, ale nie indexovaná. |
| Nízka | `selfupdate::write_pinned` | Zámok inštalátora sa uvoľní po návrate ShellExecute; Inno Setup potom svoj súbor otvára znova podľa cesty (kópia do `%TEMP%`). Okno je krátke a v priečinku, ktorý vytvorila lišta; úplné riešenie by bolo spúšťať cez `CreateProcess` a držať handle do skončenia setupu. |
| Nízka | `claude_setup.rs` | Atomický zápis (premenovanie) nahradí aj `settings.json`, ktorý je symlinkom (napr. správa dotfiles): odkaz sa zmení na obyčajný súbor. Zachovanie by vyžadovalo zapisovať do cieľa odkazu. |
| Nízka | `transom-usage/src/cache.rs` (`load`) a iné načítania vlastných dát | Čítajú celý súbor bez stropu veľkosti (vlastné súbory v `%LOCALAPPDATA%\transom`; importy a logy strop majú). |
| – | `Cargo.toml` | Navrhujem zapnúť `clippy::undocumented_unsafe_blocks` až po zlúčení vetiev (inak by zlyhal clippy iných vetiev). |

### 8.35 Vitajte a spustenie s Windows (2026-11-26, vydanie 0.1.1)

**Prvé spustenie.** Keď lišta pri štarte nenájde súbor nastavení (a nejde o test), uloží predvolené nastavenia a otvorí
nastavenia na novej stránke **Vitajte** (Systém › Vitajte). Sprievodca sa preto ukáže len raz; stránka zostáva v navigácii.
Obsah: spúšťať s Windows, ostrov v strede, sklo pod panelmi a pod lištou, AI na lište, Claude Code a Codex (lokálne zdroje),
tlačidlo na prepojenie Claude Code (stránka AI), počasie na lište a výber miesta (stránka Doplnky), automatická kontrola
aktualizácií, *Hotovo* zatvorí okno. Voľby menia tie isté polia konfigurácie ako ich vlastné stránky.

**Spustenie s Windows** (`transom-platform/src/autostart.rs`): hodnota `transom` v `HKCU\…\Run` s cestou k `transom-app.exe`
v úvodzovkách – rovnaká hodnota, akú zapisuje inštalátor, takže ju odinštalátor zmaže; funguje aj pre prenosnú kópiu. Stav sa číta
z registra pri otvorení okna (nie je v konfigurácii).

**Overenie:**
- **TEST:** zápis a zmazanie hodnoty v Run pod testovacím názvom (skutočné spustenie s Windows sa nemení), úvodzovky v ceste;
  stránka Vitajte prešla auditmi nastavení vo všetkých 11 jazykoch a pri 96/144/192 DPI (orezanie textu, prístupnosť – audit
  odhalil dve rovnomenné skupiny volieb, premenované na „AI na lište“ a „Počasie na lište“ –, klávesnica).
- **NÁHĽAD:** `transom-settings --shot … --png --full` (snímky stránky v tmavej aj svetlej téme).
- **Neoverené naživo:** skutočné prvé spustenie na čistom PC (otvorenie stránky z lišty), prepnutie autoštartu z okna.

### 8.36 Farby, počasie, batéria a čitateľnosť (2026-11-26, nevydané)

- **Panel Farby** (`colors_pop.rs`, sada `pop_kit`): veľká vzorka, HEX / RGB / HSL, kopírovanie, história (najviac 24,
  `tools\colors\history.json`, verzované ako ostatné indexy nástrojov). Kapátko už nepoužíva ostrov: panel sa zavrie,
  po výbere sa otvorí znova s novou farbou.
- **Kapátko** (`picker.rs`): okno cez celú virtuálnu obrazovku s krytím 1/255 vlastní kurzor (vlastný kurzor pipety z
  `CreateIconIndirect`, nie `SetSystemCursor` - pád by nechal kurzor zmenený), neaktivuje sa a je vylúčené zo snímania
  (`WDA_EXCLUDEFROMCAPTURE`), rovnako lupa 11 x 11 bodov. Meranie je presné aj s nimi (test `live_sampling_...`).
  Esc len počas výberu cez `RegisterHotKey` (žiadny klávesnicový hák).
- **Sklo vo vlastnej farbe** (`glass::tint_for`): vzhľad podľa farby (svetlá farba = svetlé sklo s tmavým textom),
  najprv sa zvýši krytie (najviac o 40, po 242), až potom sa farba stmaví / zosvetlí; kontrast 4,5 : 1 pre tlmený,
  červený a akcentový text na paneli aj karte a hlavný text na ovládači pod myšou, nad bielou aj čiernou stránkou.
- **Priehľadná lišta** (krytie pod 50 %, automatický text, bez skla): medián riadku tesne pod horným okrajom lišty na
  obrazovke = to, čo je pod ňou; text podľa `Theme::on_background`. Meria sa po štarte, zmene tapety
  (`SPI_SETDESKWALLPAPER`), nastavení, konci celoobrazovkovej aplikácie a najviac raz za 20 s pri tiku.
- **Počasie**: ikona podľa kódu WMO; pri daždi / snehu kvapky / vločky (`precip_particles`, čistá funkcia) ~12 snímok/s,
  prekresľuje sa len položka počasia, nie pri vypnutých animáciách Windows ani skrytej lište.
- **Batéria**: nabíjanie zelené (`Theme::good`), pod 20 % červené (aj „nízka“ pri perifériách od 19 %).
- **Klik inde na lište** zavrie otvorený panel aj menu ostrova (lišta okno neaktivuje, panel by inak ostal otvorený).

## 9. Cargo workspace

```
transom/
├─ Cargo.toml  rust-toolchain.toml  deny.toml  LICENSE (MIT)  README.md
├─ docs/PLAN.md
├─ crates/
│  ├─ transom-core/       # bez Win32: model StatusItem, udalosti, rozloženie
│  ├─ transom-config/     # typy konfigurácie, načítanie, atomický zápis (zdieľané lištou aj nastaveniami)
│  ├─ transom-platform/   # Win32: AppBar, monitory, DPI, fullscreen, SendInput, power, autostart, tray ikona
│  ├─ transom-render/     # rozhranie Painter, témy, hit-testing; backendy za feature podľa výsledku G1
│  ├─ transom-media/      # GSMTC služba
│  ├─ transom-items/      # vstavané položky: clock, battery, search, tray, media
│  ├─ transom-app/        # binár lišty
│  └─ transom-settings/   # binár grafických nastavení (spúšťa sa na požiadanie)
└─ xtask/
```

*Stav 2026-11-20 (§8.6):* existujú `transom-core`, `transom-config`, `transom-i18n` (preklady), `transom-platform`, `transom-render`, `transom-media`,
`transom-usage`, `transom-app`, `transom-settings` (GDI, nie egui) a `transom-statusline` (most pre Claude Code). Vstavané položky sú zatiaľ v `transom-app/src/view.rs` (stav → `StatusItem`), samostatný `transom-items` nevznikol; `xtask` ešte nie.

`StatusItem` (len v `transom-core`, **nestabilné, bez sľubu ABI**): identifikátor, dátový popis obsahu
(text, ikona, tooltip, stav tlačidiel – bez closures), spracovanie hover/click, plánovanie ďalšieho prebudenia,
signál "zneplatnené". Vykreslí sa len pri zneplatnení. Žiadny WASM, plugin proces ani DLL v MVP.

## 10. Riziká

| Riziko | Dopad | Overenie |
|---|---|---|
| AppBar po páde, pri zmene DPI, odpojení monitora | rezervované miesto zostane / okná sa posunú | S1, S2 |
| Stabilita ID monitora | zlý monitor po reštarte | S2 |
| `keep_visible` pri exkluzívnom fullscreene | lišta nie je vidieť, možný pokles výkonu | S3 |
| UIPI a fokus pri `Win+S` / `Win+B` | tlačidlo potichu nereaguje | S4 |
| Spotify a SMTC | Spotify publikuje session len pri určitom nastavení (korelácia, S5); bez neho médiá pre Spotify nefungujú | S5 (overené, riziko ostáva: nápoveda v lište, §6) |
| GSMTC bez package identity | nutnosť balenia (MSIX) | S5 (overené: netreba) |
| Výsledky merania sa líšia od očakávaní | zmena renderera | S6, S7, G1 |
| Mica/Acrylic na okne bez titulku | nutná plná farba | S9 |
| Okno nastavení: footprint a vzhľad toolkitu | rozšírenie bázy závislostí | ~~S8~~ prekonané: vlastné GDI okno (§7.7, §8.4) |
| Licencie závislostí | konflikt s MIT | `cargo-deny` v CI |
| Rust toolchain nie je nainštalovaný | blokuje spike | schváliť inštaláciu |

## 11. Zostávajúce predpoklady (každý má priradený spike)

1. Rezervácia AppBaru sa po páde procesu uvoľní (S1).
2. Viac AppBar okien, po jednom na monitor, je stabilných pri zmene DPI a odpojení (S1, S2).
3. Stabilné ID monitora cez DisplayConfig prežije reštart (S2).
4. Mica/Acrylic a nezaoblené rohy fungujú na okne bez titulku (S9).
5. Klasický exkluzívny fullscreen: správanie lišty v `keep_visible` a vplyv na výkon (S3).
6. ~~`Win+B` a `Win+S` cez `SendInput` fungujú v definovaných scenároch a ich zlyhania sú zrozumiteľné (S4).~~
   **Overené z lišty (2026-11-20, §5.1)** pre bežné popredie; zvýšené popredie len zo S4 (s výhradou).
7. ~~Spotify publikuje SMTC session a sprístupňuje príkazy (S5).~~ **Splnené s výhradou (S5, 2026-11-18):** po zapnutí „Show desktop
   overlay when using media keys“; príčina je pozorovaná korelácia, nie potvrdená (§6.1).
8. ~~GSMTC funguje bez package identity (S5).~~ **Overené (S5):** Spotify, Brave, Windows Media Player.
9. Ktorý renderer spĺňa prahy pre CPU, pamäť a prezentované snímky (S6, S7).
10. Kvalita textu a ikon zo systémového písma pri 100–200 % (S6).
11. ~~Okno nastavení cez egui (alebo natívne) vyzerá a štartuje primerane na Windows 11 (S8).~~ **Prekonané:** S8 sa nerobil, okno je vlastné GDI (§7.7, §8.11).
12. ~~Živé načítanie konfigurácie cez registrovanú správu je spoľahlivé (S8).~~ **Overené v §8.4/§8.11** (registrovaná správa + znovunačítanie; od §8.14 pri poškodenom súbore ostáva platná konfigurácia).
13. Rozsah toolchainu: dostupnosť MSVC a Build Tools na tomto stroji (Etapa 0).
14. ~~Presné GUID power notifikácií a údaje `GetSystemPowerStatus` (Etapa 1; z mojich znalostí).~~ **Overené** v zdrojoch
    `windows` 0.62.2 (`Win32::System::SystemServices`), registrácia funguje; zmena stavu batérie netestovaná (desktop).

## Zdroje (načítané 2026-11-15)

- Application desktop toolbars: https://learn.microsoft.com/en-us/windows/win32/shell/application-desktop-toolbars
- SHAppBarMessage: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shappbarmessage
- APPBARDATA: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-appbardata
- SendInput (UIPI): https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
- QUERY_USER_NOTIFICATION_STATE: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ne-shellapi-query_user_notification_state
- GSMTC session manager: https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionmanager
- GSMTC playback controls: https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionplaybackcontrols
- Klávesové skratky Windows: https://support.microsoft.com/en-us/help/12445/windows-keyboard-shortcuts.
- Navigácia panelom úloh: https://support.microsoft.com/en-US/accessibility/windows/navigate-and-explore-the-windows-taskbar
- Slint licencie: https://slint.dev/terms-and-conditions
- crates.io (verzie a licencie): windows, windows-future, slint, egui, eframe, iced, tiny-skia, cosmic-text, softbuffer, xilem, floem, serde, toml, tracing

Z mojich znalostí (nepotvrdené v tejto relácii): DWM atribúty (rohy, backdrop), DisplayConfig ID monitora,
manifest PerMonitorV2, GUID power notifikácií.
