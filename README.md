# QuickCreateOmniware

Snabbinmatning + journal/tidslinje som körs i aktivitetsfältet. Rust + egui, SQLite (WAL, `synchronous=FULL`), JetBrains Mono inbyggt.

## Bygga (Windows)

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
# ny terminal
cd K:\VFSH\QuickCreateOmniware
cargo build --release
.\target\release\omniware.exe
```

Debug-bygget (`cargo run`) visar en konsol med loggutskrifter, men release-bygget gör det inte.

## Kortkommandon

**Globala**

| Tangent | Funktion |
|---|---|
| `Win+Alt+V` / klick på ikonen | Snabbrutan, förifylld med urklipp (text eller bild) |
| `Win+Alt+Space` / högerklick → Öppna Omni | Huvudfönstret (öppna/stäng) |

**Snabbrutan**

| Tangent | Funktion |
|---|---|
| `Esc` | Spara och stäng (tomt inlägg kastas) |
| `Shift+Esc` | Kasta till papperskorgen |
| `Ctrl+S` | Namnge → `Enter`. Upptaget namn → `Enter` igen flyttar namnet hit; det gamla inlägget behålls utan namn |
| `Ctrl+V` | Text som vanligt; bild sparas och infogas som `![](blob:<hash>.png)` |

**Huvudfönstret**

| Tangent | Funktion |
|---|---|
| `←` `→` / `T` | Föregående/nästa dag / idag (tidslinjen) |
| `Ctrl+K` | Sök i allt. `Enter` öppnar, `Shift+Enter` klistrar in i fönstret du kom från |
| `Ctrl+N` | Nytt inlägg |
| `Ctrl+E` | Växla redigera/förhandsvisa (markdown) |
| `Esc` | Tillbaka → stäng |

## Vart allt hamnar

| Handling | Syns i |
|---|---|
| Spara (Esc) | Tidslinjen för dagen det skapades; prick i månadskalendern |
| Namngett | Samma + Namngivna |
| Kastat | Papperskorgen (återställ / radera permanent) |

## Data

`%APPDATA%\Omniware\` (kan ändras med miljövariabeln `OMNIWARE_DATA`):

- `omniware.db` – alla inlägg, versioner och sökindex (FTS5)
- `blobs\ab\<blake3>.png` – bilder, adresserade efter innehåll
- `config.toml` – skapas med standardvärden vid första start
- `omniware.log` – fel

```toml
renderer = "wgpu"            # "glow" = OpenGL-reserv om GPU:n krånglar

[hotkeys]
capture = "super+alt+KeyV"   # alt/ctrl/shift/super + KeyX/DigitN/F1/Space…
main = "super+alt+Space"

[window]
width = 640.0
height = 420.0
font_size = 14.0
main_width = 1040.0
main_height = 700.0
```

## Hållbarhet

- Urklippet skrivs till databasen **innan** rutan visas.
- Ändringar skrivs 300 ms efter senaste tangenttryck (fsync per commit).
- En version sparas när rutan stängs (identiska versioner hoppas över).
- Bilder skrivs först till en temporär fil, fsyncas och döps sedan om.

## Status

Steg 1–2 klara. Nästa: `Ctrl+D` händelser med datum på svenska, kalendervy, påminnelser; sedan post-its och att-göra.
